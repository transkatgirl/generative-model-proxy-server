use std::{
    clone::Clone,
    fmt::Debug,
    time::{Duration, Instant, SystemTime},
};

use axum::{
    extract::{Extension, Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

mod state;

use state::RelatedToItemSet;

use self::state::RelatedToItem;

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
    quotas: Vec<Uuid>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
struct Role {
    label: String,
    uuid: Uuid,

    admin: bool,

    models: Vec<Uuid>,
    quotas: Vec<Uuid>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Model {
    #[serde(default)]
    label: String,

    #[serde(default)]
    uuid: Uuid,

    endpoints: Vec<String>,

    api: ModelBackend,

    quotas: Vec<Uuid>,
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

#[derive(Debug, Clone)]
struct Authenticated {
    tags: Vec<Uuid>,
    timestamp: Instant,

    user: User,
    roles: Vec<Role>,
}

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
            get(get_quota).patch(update_quota).delete(delete_quota),
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
    let arrived_at = Instant::now();

    if let Some(header_value) = request.headers().get("authorization") {
        match header_value.to_str() {
            Ok(header_string) => {
                let header_string = header_string.to_ascii_lowercase();

                match header_string
                    .strip_prefix("basic ")
                    .or(header_string.strip_prefix("bearer "))
                {
                    Some(api_key) => {
                        /*match state.authenticate(api_key, arrived_at).await {
                            Some(flattened_state) => {
                                request.extensions_mut().insert(flattened_state);
                                Ok(next.run(request).await)
                            }
                            None => Err(StatusCode::UNAUTHORIZED),
                        },*/

                        todo!()
                    }
                    None => Err(StatusCode::UNAUTHORIZED),
                }
            }
            Err(_) => Err(StatusCode::UNAUTHORIZED),
        }
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
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

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum StringOrUuid {
    Uuid(Uuid),
    String(String),
}

impl RelatedToItemSet for User {
    type Key = StringOrUuid;

    fn get_keys(&self, table: &str) -> Vec<Self::Key> {
        match table {
            "roles" => self
                .roles
                .iter()
                .map(|item| StringOrUuid::Uuid(*item))
                .collect(),
            "models" => self
                .models
                .iter()
                .map(|item| StringOrUuid::Uuid(*item))
                .collect(),
            "quotas" => self
                .quotas
                .iter()
                .map(|item| StringOrUuid::Uuid(*item))
                .collect(),
            _ => self
                .api_keys
                .iter()
                .map(|item| StringOrUuid::String(item.clone()))
                .collect(),
        }
    }
}

impl RelatedToItemSet for Role {
    type Key = Uuid;

    fn get_keys(&self, table: &str) -> Vec<Self::Key> {
        match table {
            "quotas" => self.quotas.clone(),
            _ => self.models.clone(),
        }
    }
}

impl RelatedToItemSet for Model {
    type Key = Uuid;

    fn get_keys(&self, _table: &str) -> Vec<Self::Key> {
        self.quotas.clone()
    }
}

impl RelatedToItem for Uuid {
    type Key = Uuid;

    fn get_key(&self, _id: &str) -> Self::Key {
        *self
    }
}

async fn get_users(State(state): State<AppState>) -> Result<Json<Vec<User>>, StatusCode> {
    state.get_items("users")
}

async fn get_user(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<User>, StatusCode> {
    state.get_item("users", &uuid)
}

async fn add_user_post(
    State(state): State<AppState>,
    Json(mut payload): Json<User>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    let related_items: Vec<_> = payload
        .api_keys
        .iter()
        .map(|item| (item, payload.uuid))
        .collect();

    let status = state.insert_related_items(
        ("users", "api_keys"),
        (&payload.uuid, &payload),
        &related_items,
    );

    if status.is_success() {
        Ok(Json(payload.uuid))
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

    state.insert_related_items(
        ("users", "api_keys"),
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

    let related_items: Vec<_> = payload
        .api_keys
        .iter()
        .map(|item| (item, payload.uuid))
        .collect();

    state.insert_related_items(
        ("users", "api_keys"),
        (&payload.uuid, &payload),
        &related_items,
    )
}

#[tracing::instrument(skip(state), level = "debug")]
async fn delete_user(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    state.remove_related_items::<_, User>(("users", "api_keys"), &uuid)
}

async fn get_roles(State(state): State<AppState>) -> Result<Json<Vec<Role>>, StatusCode> {
    state.get_items("roles")
}

async fn get_role(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Role>, StatusCode> {
    state.get_item("roles", &uuid)
}

async fn add_role_post(
    State(state): State<AppState>,
    Json(mut payload): Json<Role>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    let status = state.insert_item("roles", &payload.uuid, &payload);

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

    state.insert_item("roles", &payload.uuid, &payload)
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

    state.insert_item("roles", &payload.uuid, &payload)
}

async fn delete_role(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    state.remove_item("roles", &uuid)
}

async fn get_models(State(state): State<AppState>) -> Result<Json<Vec<Model>>, StatusCode> {
    state.get_items("models")
}

async fn get_model(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Model>, StatusCode> {
    state.get_item("models", &uuid)
}

async fn add_model_post(
    State(state): State<AppState>,
    Json(mut payload): Json<Model>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    let status = state.insert_item("models", &payload.uuid, &payload);

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

    state.insert_item("models", &payload.uuid, &payload)
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

    state.insert_item("models", &payload.uuid, &payload)
}

async fn delete_model(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    state.remove_item("models", &uuid)
}

async fn get_quotas(State(state): State<AppState>) -> Result<Json<Vec<Quota>>, StatusCode> {
    state.get_items("quotas")
}

async fn get_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Quota>, StatusCode> {
    state.get_item("quotas", &uuid)
}

async fn add_quota_post(
    State(state): State<AppState>,
    Json(mut payload): Json<Quota>,
) -> Result<Json<Uuid>, StatusCode> {
    if payload.uuid != Uuid::default() {
        return Err(StatusCode::BAD_REQUEST);
    }
    payload.uuid = Uuid::new_v4();

    let status = state.insert_item("quotas", &payload.uuid, &payload);

    if status.is_success() {
        Ok(Json(payload.uuid))
    } else {
        Err(status)
    }
}

async fn add_quota_put(State(state): State<AppState>, Json(payload): Json<Quota>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        return StatusCode::BAD_REQUEST;
    }

    state.insert_item("quotas", &payload.uuid, &payload)
}

async fn update_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(mut payload): Json<Quota>,
) -> StatusCode {
    if payload.uuid != Uuid::default() && payload.uuid != uuid {
        return StatusCode::BAD_REQUEST;
    }
    payload.uuid = uuid;

    state.insert_item("quotas", &payload.uuid, &payload)
}

async fn delete_quota(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    state.remove_item("quotas", &uuid)
}
