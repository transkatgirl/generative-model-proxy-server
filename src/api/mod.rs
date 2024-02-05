use std::{clone::Clone, fmt::Debug};

use axum::{
    body::{self, Body, Bytes},
    extract::{Extension, Path, Request, State},
    http::{header::HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, OwnedRwLockReadGuard, RwLock};
use uuid::Uuid;

mod state;

use super::limiter::{Limit, Limiter};
use super::model::{CallableModelAPI, ModelAPI, ModelAPIClient, ModelRequest, ModelResponse};
use state::{AppState, FlattenedAppState};

#[derive(Default, Serialize, Deserialize, Debug)]
#[serde(default)]
struct User {
    label: String,
    uuid: Uuid,

    api_keys: Vec<String>,
    roles: Vec<Uuid>,

    admin: bool,

    models: Vec<Uuid>,
    quotas: Vec<QuotaMember>,
}

#[derive(Default, Serialize, Deserialize, Debug)]
#[serde(default)]
struct Role {
    label: String,
    uuid: Uuid,

    admin: bool,

    models: Vec<Uuid>,
    quotas: Vec<QuotaMember>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Model {
    #[serde(default)]
    label: String,

    #[serde(default)]
    uuid: Uuid,

    api: ModelAPI,

    quotas: Vec<QuotaMember>,
}

#[derive(Serialize, Deserialize, Debug)]
struct QuotaMember {
    quota: Uuid,
    //priority: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug)]
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
    if let Some(header_value) = request.headers().get("authorization") {
        match header_value.to_str() {
            Ok(header_string) => {
                let header_string = header_string.to_ascii_lowercase();

                match header_string
                    .strip_prefix("basic")
                    .or(header_string.strip_prefix("bearer"))
                {
                    Some(api_key) => match state.authenticate(api_key).await {
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
    match state.is_admin() {
        true => Ok(next.run(request).await),
        false => Err(StatusCode::UNAUTHORIZED),
    }
}

async fn model_request(
    Extension(state): Extension<FlattenedAppState>,
    Json(payload): Json<ModelRequest>,
) -> Result<(StatusCode, Json<ModelResponse>), StatusCode> {
    state
        .model_request(payload)
        .await
        .map(|(status, response)| (status, Json(response)))
}

async fn get_users(State(state): State<AppState>) -> Json<Vec<User>> {
    todo!()
}

async fn get_user(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<User>, StatusCode> {
    todo!()
}

async fn add_user(State(state): State<AppState>, Json(payload): Json<User>) -> StatusCode {
    todo!()
}

async fn update_user(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(payload): Json<User>,
) -> StatusCode {
    todo!()
}

async fn delete_user(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    todo!()
}

async fn get_roles(State(state): State<AppState>) -> Json<Vec<Role>> {
    todo!()
}

async fn get_role(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Role>, StatusCode> {
    todo!()
}

async fn add_role(State(state): State<AppState>, Json(payload): Json<Role>) -> StatusCode {
    todo!()
}

async fn update_role(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(payload): Json<Role>,
) -> StatusCode {
    todo!()
}

async fn delete_role(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    todo!()
}

async fn get_models(State(state): State<AppState>) -> Json<Vec<Model>> {
    todo!()
}

async fn get_model(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Model>, StatusCode> {
    todo!()
}

async fn add_model(State(state): State<AppState>, Json(payload): Json<Model>) -> StatusCode {
    todo!()
}

async fn rename_model(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(payload): Json<LabelUpdateRequest>,
) -> StatusCode {
    todo!()
}

async fn delete_model(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    todo!()
}

async fn get_quotas(State(state): State<AppState>) -> Json<Vec<Quota>> {
    todo!()
}

async fn get_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
) -> Result<Json<Quota>, StatusCode> {
    todo!()
}

async fn add_quota(State(state): State<AppState>, Json(payload): Json<Quota>) -> StatusCode {
    todo!()
}

async fn rename_quota(
    State(state): State<AppState>,
    Path(uuid): Path<Uuid>,
    Json(payload): Json<LabelUpdateRequest>,
) -> StatusCode {
    todo!()
}

async fn delete_quota(State(state): State<AppState>, Path(uuid): Path<Uuid>) -> StatusCode {
    todo!()
}
