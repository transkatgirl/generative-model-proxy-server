use std::{
    clone::Clone,
    collections::HashMap,
    fmt::Debug,
    iter,
    time::{Duration, Instant, SystemTime},
};

use axum::{
    extract::{Extension, OriginalUri, Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};

use http::{Method, Uri};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

mod state;

use state::{RelatedToItem, RelatedToItemSet};

use super::{
    limiter::Limit,
    model::{ModelBackend, ModelError, ModelResponse, RequestType, TaggedModelRequest},
    AppState,
};

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

    types: Vec<RequestType>,

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
    timestamp: Instant,
    user: User,
    roles: Vec<Role>,
}

#[tracing::instrument(level = "debug", skip(state))]
pub async fn api_router(state: AppState) -> Router {
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
        .fallback(StatusCode::NOT_FOUND)
        .route_layer(middleware::from_fn(authenticate_admin));

    Router::new()
        .fallback(post(model_request))
        .nest("/admin", admin_routes)
        .with_state(state.clone())
        .route_layer(middleware::from_fn_with_state(state, authenticate))
        .layer(TraceLayer::new_for_http())
}

#[tracing::instrument(level = "trace", skip(state), ret)]
async fn authenticate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ModelError> {
    let timestamp = Instant::now();

    match request
        .headers()
        .get("authorization")
        .and_then(|header_value| {
            header_value.to_str().ok().and_then(|header_string| {
                header_string
                    .to_ascii_lowercase()
                    .strip_prefix("basic ")
                    .or(header_string.strip_prefix("bearer "))
                    .map(|string| string.to_string())
            })
        }) {
        Some(api_key) => {
            match state.get_related_item::<_, Uuid, User>(("api_keys", "users"), &api_key) {
                Ok(Json(user)) => {
                    let roles: Vec<Role> = user
                        .roles
                        .iter()
                        .filter_map(|uuid| state.get_item("roles", uuid).map(|item| item.0).ok())
                        .collect();

                    request.extensions_mut().insert(Authenticated {
                        timestamp,
                        user,
                        roles,
                    });
                    Ok(next.run(request).await)
                }
                Err(status) => Err(if status.is_client_error() {
                    ModelError::AuthInvalid
                } else {
                    ModelError::InternalError
                }),
            }
        }
        None => Err(ModelError::AuthMissing),
    }
}

#[tracing::instrument(level = "trace", skip(auth), ret)]
async fn authenticate_admin(
    Extension(auth): Extension<Authenticated>,
    request: Request,
    next: Next,
) -> Result<Response, ModelError> {
    if auth.user.admin {
        return Ok(next.run(request).await);
    }

    for role in auth.roles {
        if role.admin {
            return Ok(next.run(request).await);
        }
    }

    Err(ModelError::UnknownEndpoint)
}

#[tracing::instrument(level = "trace", skip(state), ret)]
async fn model_request(
    Extension(auth): Extension<Authenticated>,
    State(state): State<AppState>,
    request: Request,
) -> Result<ModelResponse, ModelError> {
    let request_type = match RequestType::try_from(request.uri()) {
        Ok(r#type) => r#type,
        Err(_) => return Err(ModelError::UnknownEndpoint),
    };

    if request.method() != Method::POST {
        return Err(ModelError::BadEndpointMethod);
    }

    let models = auth
        .user
        .models
        .iter()
        .chain(auth.roles.iter().flat_map(|role| role.models.iter()))
        .filter_map(|uuid| {
            state
                .get_item::<_, Model>("models", uuid)
                .map(|item| item.0)
                .ok()
        })
        .filter(|model| model.types.contains(&request_type));

    let quotas = auth
        .user
        .quotas
        .iter()
        .chain(auth.roles.iter().flat_map(|role| role.quotas.iter()));

    let tags = iter::once(auth.user.uuid)
        .chain(auth.user.roles.iter().copied())
        .chain(auth.user.quotas.iter().copied())
        .chain(
            auth.roles
                .iter()
                .flat_map(|role| role.quotas.iter().copied()),
        );

    //let request = TaggedModelRequest::from(value)

    /*for model in models {
        if model.label =
    }*/

    // TODO: Add rate limiting

    /*let response = state.model_request(payload).await;

    (response.0, Json(response.1))*/

    todo!()
}

impl IntoResponse for ModelResponse {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(self.response)).into_response()
    }
}

impl IntoResponse for ModelError {
    fn into_response(self) -> axum::response::Response {
        ModelResponse::from(self).into_response()
    }
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
