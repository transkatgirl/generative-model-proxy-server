use std::{clone::Clone, fmt::Debug, time::Instant};

use axum::{
    extract::{Extension, Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};

use postcard::{from_bytes, to_stdvec};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use sled::{
    transaction::{
        abort, ConflictableTransactionError, ConflictableTransactionResult, TransactionError,
        TransactionResult, Transactional, TransactionalTree,
    },
    Batch,
};
use tower_http::{follow_redirect::policy::PolicyExt, trace::TraceLayer};
use uuid::Uuid;

use super::{limiter::Limit, model::ModelBackend, AppState};

/*
# API todos:
- Allow limiting models to specific endpoints
- Rework model/quota API to be the same as users/roles
- Improve error messages
- **Add documentation**

# App todos:
- Clean up logging
- Add state save/restore
- Improve error handling
*/

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
struct User {
    label: String,
    uuid: Uuid,

    admin: bool,

    api_keys: Vec<String>,
    roles: Vec<Uuid>,

    models: Vec<Uuid>,
    quotas: Vec<QuotaMember>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
struct Role {
    label: String,
    uuid: Uuid,

    admin: bool,

    models: Vec<Uuid>,
    quotas: Vec<QuotaMember>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Model {
    #[serde(default)]
    label: String,

    #[serde(default)]
    uuid: Uuid,

    api: ModelBackend,

    quotas: Vec<QuotaMember>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct QuotaMember {
    quota: Uuid,
    //priority: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Quota {
    #[serde(default)]
    label: String,

    #[serde(default)]
    uuid: Uuid,

    limits: Vec<Limit>,
}

#[derive(Serialize, Deserialize, Debug)]
struct LabelUpdateRequest {
    label: String,
    uuid: Uuid,
}

#[derive(Debug, Clone)]
struct Authenticated {
    tags: Vec<Uuid>,
}

#[derive(Debug, PartialEq)]
struct DatabaseTransactionError;

#[tracing::instrument(level = "debug", skip(state))]
pub async fn api_router(state: AppState) -> Router {
    let openai_routes = Router::new()
        .route("/chat/completions", post(model_request))
        .route("/edits", post(model_request))
        .route("/completions", post(model_request))
        .route("/moderations", post(model_request))
        .route("/embeddings", post(model_request));

    let admin_routes = Router::new()
        .route(
            "/users",
            get(get_users).post(add_user_post).put(add_user_put),
        )
        .route(
            "/users/:uuid",
            get(get_user).put(update_user).delete(delete_user),
        )
        .route(
            "/roles",
            get(get_roles).post(add_role_post).put(add_role_put),
        )
        .route(
            "/roles/:uuid",
            get(get_role).put(update_role).delete(delete_role),
        )
        .route(
            "/models",
            get(get_models).post(add_model_post).put(add_model_put),
        )
        .route(
            "/models/:uuid",
            get(get_model).put(update_model).delete(delete_model),
        )
        .route(
            "/quotas",
            get(get_quotas).post(add_quota_post).put(add_quota_put),
        )
        .route(
            "/quotas/:uuid",
            get(get_quota).patch(rename_quota).delete(delete_quota),
        )
        .with_state(state.clone())
        .route_layer(middleware::from_fn(authenticate_admin));

    Router::new()
        .nest("/v1/", openai_routes)
        .nest("/admin", admin_routes)
        .route_layer(middleware::from_fn_with_state(state, authenticate))
        .layer(TraceLayer::new_for_http())
}

//#[tracing::instrument(level = "debug", skip(state))]
async fn authenticate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    /*let arrived_at = Instant::now();

    if let Some(header_value) = request.headers().get("authorization") {
        match header_value.to_str() {
            Ok(header_string) => {
                let header_string = header_string.to_ascii_lowercase();

                match header_string
                    .strip_prefix("basic ")
                    .or(header_string.strip_prefix("bearer "))
                {
                    Some(api_key) => match state.authenticate(api_key, arrived_at).await {
                        Some(flattened_state) => {
                            request.extensions_mut().insert(flattened_state);
                            Ok(next.run(request).await)
                        }
                        None => Err(StatusCode::UNAUTHORIZED),
                    },
                    None => Err(StatusCode::UNAUTHORIZED),
                }
            }
            Err(_) => Err(StatusCode::UNAUTHORIZED),
        }
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }*/

    todo!()
}

async fn authenticate_admin(
    Extension(state): Extension<Authenticated>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    /*match state.admin {
        true => Ok(next.run(request).await),
        false => Err(StatusCode::UNAUTHORIZED),
    }*/

    todo!()
}
async fn model_request(
    Extension(state): Extension<Authenticated>,
    Json(payload): Json<Value>,
) -> (StatusCode, Json<Value>) {
    /*let response = state.model_request(payload).await;

    (response.0, Json(response.1))*/

    todo!()
}

#[tracing::instrument(skip(state), level = "debug")]
fn get_items<V>(table: &str, state: AppState) -> Result<Json<Vec<V>>, StatusCode>
where
    V: DeserializeOwned,
{
    match state.database.open_tree(table.as_bytes()) {
        Ok(tree) => Ok(Json(
            tree.iter()
                .filter_map(|item| {
                    item.ok()
                        .and_then(|(_, value)| postcard::from_bytes(&value).ok())
                })
                .collect(),
        )),
        Err(error) => {
            tracing::warn!("Unable to open \"{}\" table: {}", table, error);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

#[tracing::instrument(skip(state), level = "debug")]
fn get_item<K, V>(table: &str, state: AppState, key: &K) -> Result<Json<V>, StatusCode>
where
    K: Serialize + Debug,
    V: DeserializeOwned,
{
    match state.database.open_tree(table.as_bytes()) {
        Ok(tree) => tree
            .transaction(|tree| {
                match tree
                    .get(postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?)?
                {
                    Some(value) => Ok(Ok(Json(
                        postcard::from_bytes(&value)
                            .map_err(ConflictableTransactionError::Abort)?,
                    ))),
                    None => Ok(Err(StatusCode::NOT_FOUND)),
                }
            })
            .unwrap_or_else(|error| {
                tracing::warn!("Unable to apply database transaction: {}", error);
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }),
        Err(error) => {
            tracing::warn!("Unable to open \"{}\" table: {}", table, error);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

pub trait RelatedTo {
    type Link: Serialize + DeserializeOwned;

    fn get_keys(&self) -> Vec<Self::Link>;
}

#[tracing::instrument(skip(state), level = "debug")]
fn insert_item<K, V>(table: &str, state: AppState, key: &K, value: &V) -> StatusCode
where
    K: Serialize + Debug,
    V: Serialize + Debug,
{
    match state.database.open_tree(table.as_bytes()) {
        Ok(tree) => tree
            .transaction(|tree| {
                tree.insert(
                    postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?,
                    postcard::to_stdvec(value).map_err(ConflictableTransactionError::Abort)?,
                )?;

                Ok(StatusCode::OK)
            })
            .unwrap_or_else(|error| {
                tracing::warn!("Unable to apply database transaction: {}", error);
                StatusCode::INTERNAL_SERVER_ERROR
            }),
        Err(error) => {
            tracing::warn!("Unable to open \"{}\" table: {}", table, error);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

#[tracing::instrument(skip(state), level = "debug")]
fn insert_related_items<K, L, V, W>(
    tables: (&str, &str),
    state: AppState,
    main_item: (&K, &V),
    related_items: &[(L, W)],
) -> StatusCode
where
    K: Serialize + Debug,
    L: Serialize + Debug,
    V: Serialize + DeserializeOwned + RelatedTo + Debug,
    W: Serialize + Debug,
{
    let table_main = match state.database.open_tree(tables.0.as_bytes()) {
        Ok(tree) => tree,
        Err(error) => {
            tracing::warn!("Unable to open \"{}\" table: {}", tables.0, error);
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    let table_related = match state.database.open_tree(tables.1.as_bytes()) {
        Ok(tree) => tree,
        Err(error) => {
            tracing::warn!("Unable to open \"{}\" table: {}", tables.1, error);
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    (&table_main, &table_related)
        .transaction(|(table_main, table_related)| {
            let mut batch = Batch::default();
            if let Some(payload) = table_main.insert(
                postcard::to_stdvec(main_item.0).map_err(ConflictableTransactionError::Abort)?,
                postcard::to_stdvec(main_item.1).map_err(ConflictableTransactionError::Abort)?,
            )? {
                let deserialized: V =
                    postcard::from_bytes(&payload).map_err(ConflictableTransactionError::Abort)?;

                for linked_key in deserialized.get_keys() {
                    batch.remove(
                        postcard::to_stdvec(&linked_key)
                            .map_err(ConflictableTransactionError::Abort)?,
                    )
                }
            }

            for (key, value) in related_items {
                batch.insert(
                    postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?,
                    postcard::to_stdvec(value).map_err(ConflictableTransactionError::Abort)?,
                )
            }

            table_related.apply_batch(&batch)?;

            Ok(StatusCode::OK)
        })
        .unwrap_or_else(|error| {
            tracing::warn!("Unable to apply database transaction: {}", error);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

#[tracing::instrument(skip(state), level = "debug")]
fn remove_item<K>(table: &str, state: AppState, key: &K) -> StatusCode
where
    K: Serialize + Debug,
{
    match state.database.open_tree(table.as_bytes()) {
        Ok(tree) => tree
            .transaction(|tree| {
                tree.remove(
                    postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?,
                )?;

                Ok(StatusCode::OK)
            })
            .unwrap_or_else(|error| {
                tracing::warn!("Unable to apply database transaction: {}", error);
                StatusCode::INTERNAL_SERVER_ERROR
            }),
        Err(error) => {
            tracing::warn!("Unable to open \"{}\" table: {}", table, error);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn remove_related_items<K, V>(tables: (&str, &str), state: AppState, key: &K) -> StatusCode
where
    K: Serialize + Debug,
    V: Serialize + DeserializeOwned + RelatedTo + Debug,
{
    let table_main = match state.database.open_tree(tables.0.as_bytes()) {
        Ok(tree) => tree,
        Err(error) => {
            tracing::warn!("Unable to open \"{}\" table: {}", tables.0, error);
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    let table_related = match state.database.open_tree(tables.1.as_bytes()) {
        Ok(tree) => tree,
        Err(error) => {
            tracing::warn!("Unable to open \"{}\" table: {}", tables.1, error);
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    (&table_main, &table_related)
        .transaction(|(table_main, table_related)| {
            match table_main
                .remove(postcard::to_stdvec(key).map_err(ConflictableTransactionError::Abort)?)?
            {
                Some(payload) => {
                    let deserialized: V = postcard::from_bytes(&payload)
                        .map_err(ConflictableTransactionError::Abort)?;

                    let mut batch = Batch::default();
                    for linked_key in deserialized.get_keys() {
                        batch.remove(
                            postcard::to_stdvec(&linked_key)
                                .map_err(ConflictableTransactionError::Abort)?,
                        )
                    }
                    table_related.apply_batch(&batch)?;

                    Ok(StatusCode::OK)
                }
                None => Ok(StatusCode::NOT_FOUND),
            }
        })
        .unwrap_or_else(|error| {
            tracing::warn!("Unable to apply database transaction: {}", error);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

impl RelatedTo for User {
    type Link = String;

    fn get_keys(&self) -> Vec<Self::Link> {
        self.api_keys.clone()
    }
}

async fn get_users(State(state): State<AppState>) -> Result<Json<Vec<User>>, StatusCode> {
    get_items("users", state)
}

async fn get_user(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<User>, StatusCode> {
    get_item("users", state, &uuid)
}

async fn add_user_post(
    State(state): State<AppState>,
    Json(mut payload): Json<User>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    let uuid = payload.uuid;
    let status = add_user_put(State(state), Json(payload)).await;

    if status.is_success() {
        Ok(Json(uuid))
    } else {
        Err(status)
    }
}

async fn add_user_put(State(state): State<AppState>, Json(payload): Json<User>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    let related_items: Vec<_> = payload
        .api_keys
        .iter()
        .map(|item| (item, payload.uuid))
        .collect();

    insert_related_items(
        ("users", "api_keys"),
        state,
        (&payload.uuid, &payload),
        &related_items,
    )
}

async fn update_user(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(mut payload): Json<User>,
) -> StatusCode {
    if payload.uuid != Uuid::default() && payload.uuid != uuid {
        return StatusCode::BAD_REQUEST;
    }
    payload.uuid = uuid;

    add_user_put(State(state), Json(payload)).await
}

#[tracing::instrument(skip(state), level = "debug")]
async fn delete_user(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    remove_related_items::<_, User>(("users", "api_keys"), state, &uuid)
}

async fn get_roles(State(state): State<AppState>) -> Result<Json<Vec<Role>>, StatusCode> {
    get_items("roles", state)
}

async fn get_role(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Role>, StatusCode> {
    get_item("roles", state, &uuid)
}

async fn add_role_post(
    State(state): State<AppState>,
    Json(mut payload): Json<Role>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    let status = insert_item("roles", state, &payload.uuid, &payload);

    if status.is_success() {
        Ok(Json(payload.uuid))
    } else {
        Err(status)
    }
}

async fn add_role_put(State(state): State<AppState>, Json(payload): Json<Role>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    insert_item("roles", state, &payload.uuid, &payload)
}

async fn update_role(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(mut payload): Json<Role>,
) -> StatusCode {
    if payload.uuid != Uuid::default() && payload.uuid != uuid {
        return StatusCode::BAD_REQUEST;
    }
    payload.uuid = uuid;

    insert_item("roles", state, &payload.uuid, &payload)
}

async fn delete_role(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    remove_item("roles", state, &uuid)
}

async fn get_models(State(state): State<AppState>) -> Result<Json<Vec<Model>>, StatusCode> {
    get_items("models", state)
}

async fn get_model(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Model>, StatusCode> {
    get_item("models", state, &uuid)
}

async fn add_model_post(
    State(state): State<AppState>,
    Json(mut payload): Json<Model>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    let status = insert_item("models", state, &payload.uuid, &payload);

    if status.is_success() {
        Ok(Json(payload.uuid))
    } else {
        Err(status)
    }
}

async fn add_model_put(State(state): State<AppState>, Json(payload): Json<Model>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    insert_item("models", state, &payload.uuid, &payload)
}

async fn update_model(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(mut payload): Json<Model>,
) -> StatusCode {
    if payload.uuid != Uuid::default() && payload.uuid != uuid {
        return StatusCode::BAD_REQUEST;
    }
    payload.uuid = uuid;

    insert_item("models", state, &payload.uuid, &payload)
}

async fn delete_model(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    remove_item("models", state, &uuid)
}

async fn get_quotas(State(state): State<AppState>) -> Result<Json<Vec<Quota>>, StatusCode> {
    get_items("quotas", state)
}

async fn get_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Quota>, StatusCode> {
    get_item("quotas", state, &uuid)
}

async fn add_quota_post(
    State(state): State<AppState>,
    Json(mut payload): Json<Quota>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    todo!()
}

async fn add_quota_put(State(state): State<AppState>, Json(payload): Json<Quota>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    todo!()
}

async fn rename_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(payload): Json<LabelUpdateRequest>,
) -> StatusCode {
    if payload.uuid != Uuid::default() && payload.uuid != uuid {
        return StatusCode::BAD_REQUEST;
    }

    todo!()
}

async fn delete_quota(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    todo!()
}
