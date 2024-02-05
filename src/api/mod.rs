use std::{clone::Clone, fmt::Debug, time::Instant};

use axum::{
    extract::{Extension, Path, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod state;

use super::{
    limiter::Limit,
    model::{ModelAPI, ModelRequest, ModelResponse},
};
use state::{AppState, FlattenedAppState};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
struct User {
    label: String,
    uuid: Uuid,

    api_keys: Vec<String>,
    roles: Vec<Uuid>,

    perms: Permissions,
    models: Vec<Uuid>,
    quotas: Vec<QuotaMember>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
struct Role {
    label: String,
    uuid: Uuid,

    perms: Permissions,
    models: Vec<Uuid>,
    quotas: Vec<QuotaMember>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(default)]
struct Permissions {
    server_admin: bool,
    view_metrics: bool,
    sensitive: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Model {
    #[serde(default)]
    label: String,

    #[serde(default)]
    uuid: Uuid,

    api: ModelAPI,

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

pub fn api_router() -> Router {
    let state = AppState::new();

    let openai_routes = Router::new()
        .route("/chat/completions", post(model_request))
        .route("/edits", post(model_request))
        .route("/completions", post(model_request))
        .route("/moderations", post(model_request))
        .route("/embeddings", post(model_request));

    let admin_routes = Router::new()
        .route("/users", get(get_users).post(add_user))
        .route(
            "/users/:uuid",
            get(get_user).put(update_user).delete(delete_user),
        )
        .route("/roles", get(get_roles).post(add_role))
        .route(
            "/roles/:uuid",
            get(get_role).put(update_role).delete(delete_role),
        )
        .route("/models", get(get_models).post(add_model))
        .route(
            "/models/:uuid",
            get(get_model).patch(rename_model).delete(delete_model),
        )
        .route("/quotas", get(get_quotas).post(add_quota))
        .route(
            "/quotas/:uuid",
            get(get_quota).patch(rename_quota).delete(delete_quota),
        )
        .with_state(state.clone())
        .route_layer(middleware::from_fn(authenticate_admin));

    Router::new()
        .nest("/v1/", openai_routes)
        .nest("/admin", admin_routes)
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate))
}

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
                    .strip_prefix("basic")
                    .or(header_string.strip_prefix("bearer"))
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
    }
}

async fn authenticate_admin(
    Extension(state): Extension<FlattenedAppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    match state.perms.server_admin {
        true => Ok(next.run(request).await),
        false => Err(StatusCode::UNAUTHORIZED),
    }
}

async fn authenticate_metrics(
    Extension(state): Extension<FlattenedAppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    match state.perms.view_metrics {
        true => Ok(next.run(request).await),
        false => Err(StatusCode::UNAUTHORIZED),
    }
}

async fn model_request(
    Extension(state): Extension<FlattenedAppState>,
    Json(payload): Json<ModelRequest>,
) -> Result<(StatusCode, Json<ModelResponse>), StatusCode> {
    // TODO: Add metrics
    state
        .model_request(payload)
        .await
        .map(|(status, response)| (status, Json(response)))
}

async fn get_users(State(state): State<AppState>) -> Json<Vec<User>> {
    Json(state.get_users_snapshot().await)
}

async fn get_user(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<User>, StatusCode> {
    state
        .get_user(&uuid)
        .await
        .map(|u| Json(u.clone()))
        .ok_or(StatusCode::NOT_FOUND)
}

async fn add_user(State(state): State<AppState>, Json(mut payload): Json<User>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        payload.uuid = Uuid::new_v4()
    }

    match state.add_or_update_user(payload).await {
        true => StatusCode::CREATED,
        false => StatusCode::OK,
    }
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

    match state.update_user(payload).await {
        true => StatusCode::CREATED,
        false => StatusCode::OK,
    }
}

async fn delete_user(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    match state.remove_user(&uuid).await {
        Some(_) => StatusCode::OK,
        None => StatusCode::NOT_FOUND,
    }
}

async fn get_roles(State(state): State<AppState>) -> Json<Vec<Role>> {
    Json(state.get_roles_snapshot().await)
}

async fn get_role(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Role>, StatusCode> {
    state
        .get_role(&uuid)
        .await
        .map(|r| Json(r.clone()))
        .ok_or(StatusCode::NOT_FOUND)
}

async fn add_role(State(state): State<AppState>, Json(mut payload): Json<Role>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        payload.uuid = Uuid::new_v4()
    }

    match state.add_or_update_role(payload).await {
        true => StatusCode::CREATED,
        false => StatusCode::OK,
    }
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

    match state.update_role(payload).await {
        true => StatusCode::CREATED,
        false => StatusCode::OK,
    }
}

async fn delete_role(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    match state.remove_role(&uuid).await {
        Some(_) => StatusCode::OK,
        None => StatusCode::NOT_FOUND,
    }
}

async fn get_models(State(state): State<AppState>) -> Json<Vec<Model>> {
    Json(state.get_models_snapshot().await)
}

async fn get_model(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Model>, StatusCode> {
    match state.get_model(&uuid).await {
        Some(model) => Ok(Json(model.0.read().await.clone())),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn add_model(State(state): State<AppState>, Json(mut payload): Json<Model>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        payload.uuid = Uuid::new_v4()
    }

    match state.add_or_replace_model(payload).await {
        true => StatusCode::CREATED,
        false => StatusCode::OK,
    }
}

async fn rename_model(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(payload): Json<LabelUpdateRequest>,
) -> StatusCode {
    if payload.uuid != Uuid::default() && payload.uuid != uuid {
        return StatusCode::BAD_REQUEST;
    }

    match state.update_model_label(&uuid, payload.label).await {
        Some(_) => StatusCode::OK,
        None => StatusCode::NOT_FOUND,
    }
}

async fn delete_model(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    match state.remove_model(&uuid).await {
        Some(_) => StatusCode::OK,
        None => StatusCode::NOT_FOUND,
    }
}

async fn get_quotas(State(state): State<AppState>) -> Json<Vec<Quota>> {
    Json(state.get_quotas_snapshot().await)
}

async fn get_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Quota>, StatusCode> {
    match state.get_quota(&uuid).await {
        Some(quota) => Ok(Json(quota.0.read().await.clone())),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn add_quota(State(state): State<AppState>, Json(mut payload): Json<Quota>) -> StatusCode {
    if payload.uuid == Uuid::default() {
        payload.uuid = Uuid::new_v4()
    }

    match state.add_or_replace_quota(payload).await {
        true => StatusCode::CREATED,
        false => StatusCode::OK,
    }
}

async fn rename_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(payload): Json<LabelUpdateRequest>,
) -> StatusCode {
    if payload.uuid != Uuid::default() && payload.uuid != uuid {
        return StatusCode::BAD_REQUEST;
    }

    match state.update_quota_label(&uuid, payload.label).await {
        Some(_) => StatusCode::OK,
        None => StatusCode::NOT_FOUND,
    }
}

async fn delete_quota(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    match state.remove_quota(&uuid).await {
        Some(_) => StatusCode::OK,
        None => StatusCode::NOT_FOUND,
    }
}
