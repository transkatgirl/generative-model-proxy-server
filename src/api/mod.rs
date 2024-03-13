use std::{
    clone::Clone,
    collections::HashMap,
    fmt::Debug,
    iter,
    time::{Duration, Instant, SystemTime},
};

use axum::{
    async_trait,
    body::{self, Bytes},
    extract::{Extension, FromRequest, Multipart, OriginalUri, Path, RawForm, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Form, Json, Router,
};

use base64::{engine::general_purpose::STANDARD, Engine};
use http::{
    header::{AUTHORIZATION, CONTENT_TYPE},
    Method, Uri,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
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
- **Add documentation**

# App todos:
- Clean up logging
- Add state save/restore
- Improve error handling
*/

// TODO: Separate admin routes into separate file

// TODO: Look into async version of base64 library

// TODO: Increase max body size

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
        .get(AUTHORIZATION)
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
                Ok(user) => {
                    match state.get_items_skip_missing("roles", &user.roles) {
                        Ok(roles) => request.extensions_mut().insert(Authenticated {
                            timestamp,
                            user,
                            roles,
                        }),
                        Err(_) => return Err(ModelError::InternalError),
                    };

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
    request: Result<TaggedModelRequest, ModelError>,
) -> Result<ModelResponse, ModelError> {
    let mut request = match request {
        Ok(request) => request,
        Err(error) => return Err(error),
    };

    let label = match request.get_model() {
        Some(label) => label,
        None => return Err(ModelError::UnspecifiedModel),
    };

    let models = match state.get_items_skip_missing::<_, Model>(
        "models",
        &auth
            .user
            .models
            .iter()
            .chain(auth.roles.iter().flat_map(|role| role.models.iter()))
            .cloned()
            .collect::<Vec<_>>(),
    ) {
        Ok(models) => models,
        Err(_) => return Err(ModelError::InternalError),
    };

    let model = match models
        .iter()
        .find(|model| model.types.contains(&request.r#type) && model.label == label)
    {
        Some(model) => model,
        None => return Err(ModelError::UnknownModel),
    };

    let quotas: Vec<Uuid> = auth
        .user
        .quotas
        .iter()
        .chain(auth.roles.iter().flat_map(|role| role.quotas.iter()))
        .chain(model.quotas.iter())
        .copied()
        .collect();

    request.tags = iter::once(auth.user.uuid)
        .chain(auth.roles.iter().map(|role| role.uuid))
        .chain(quotas.iter().cloned())
        .chain(iter::once(model.uuid))
        .collect();

    // TODO: Add rate limiting

    let response = model.api.generate(&state.http, request).await;

    Ok(response)
}

#[async_trait]
impl<S> FromRequest<S> for TaggedModelRequest
where
    Bytes: FromRequest<S>,
    S: Send + Sync,
{
    type Rejection = ModelError;

    #[tracing::instrument(level = "trace", skip(state), ret)]
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let r#type = match RequestType::try_from(req.uri()) {
            Ok(r#type) => r#type,
            Err(_) => return Err(ModelError::UnknownEndpoint),
        };

        if req.method() != Method::GET
            || req.method() != Method::HEAD
            || req.method() != Method::POST
        {
            return Err(ModelError::BadEndpointMethod);
        }

        let request = match req
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|header_value| {
                header_value
                    .to_str()
                    .map(|header_string| header_string.to_ascii_lowercase())
                    .ok()
            })
            .as_deref()
        {
            Some("application/x-www-form-urlencoded") => Form::from_request(req, state)
                .await
                .map(|value| value.0)
                .unwrap_or(Value::Null),
            Some("multipart/form-data") => match Multipart::from_request(req, state).await {
                Ok(mut multipart) => {
                    let mut json_fields = Vec::new();

                    while let Some(field) = multipart.next_field().await.unwrap() {
                        json_fields.push(json!({
                            "name": field.name(),
                            "file_name": field.file_name(),
                            "content_type": field.content_type(),
                            "headers": field.headers().iter().map(|(key, value)| {
                                (key.as_str(), value.to_str().map(|value| Value::String(value.to_string())).unwrap_or(Value::String(STANDARD.encode(value.as_bytes()))))
                            }).collect::<Vec<(_, Value)>>(),
                            "content": field.bytes().await.ok().map(|bytes| STANDARD.encode(bytes)),
                        }))
                    }

                    Value::Array(json_fields)
                }
                Err(_) => Value::Null,
            },
            Some("application/json") => Json::from_request(req, state)
                .await
                .map(|value| value.0)
                .unwrap_or(Value::Null),
            Some(_) => body::to_bytes(req.into_body(), usize::MAX)
                .await
                .ok()
                .and_then(|body| Json::from_bytes(body.as_ref()).map(|value| value.0).ok())
                .unwrap_or(Value::Null),
            None => {
                if req.method() == Method::HEAD || req.method() == Method::GET {
                    Form::from_request(req, state)
                        .await
                        .map(|value| value.0)
                        .unwrap_or(Value::Null)
                } else {
                    body::to_bytes(req.into_body(), usize::MAX)
                        .await
                        .ok()
                        .and_then(|body| Json::from_bytes(body.as_ref()).map(|value| value.0).ok())
                        .unwrap_or(Value::Null)
                }
            }
        };

        if request == Value::Null {
            return Err(ModelError::BadRequest);
        }

        Ok(TaggedModelRequest::new(Vec::new(), r#type, request))
    }
}

impl IntoResponse for ModelResponse {
    fn into_response(self) -> axum::response::Response {
        if self.status == StatusCode::OK {
            if let Value::String(string) = &self.response {
                if let Ok(data) = STANDARD.decode(string) {
                    return (self.status, data).into_response();
                }
            }
        }

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
    state.get_table("users").map(|output| Json(output))
}

async fn get_user(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<User>, StatusCode> {
    state.get_item("users", &uuid).map(|output| Json(output))
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
    state.get_table("roles").map(|output| Json(output))
}

async fn get_role(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Role>, StatusCode> {
    state.get_item("roles", &uuid).map(|output| Json(output))
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
    state.get_table("models").map(|output| Json(output))
}

async fn get_model(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Model>, StatusCode> {
    state.get_item("models", &uuid).map(|output| Json(output))
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
    state.get_table("quotas").map(|output| Json(output))
}

async fn get_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Quota>, StatusCode> {
    state.get_item("quotas", &uuid).map(|output| Json(output))
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
