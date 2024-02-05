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

    models: Vec<Uuid>,
    quotas: Vec<QuotaMember>,
}

#[derive(Default, Serialize, Deserialize, Debug)]
#[serde(default)]
struct Role {
    label: String,
    uuid: Uuid,

    models: Vec<Uuid>,
    quotas: Vec<QuotaMember>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Model {
    label: String,
    uuid: Uuid,

    api: ModelAPI,

    quotas: Vec<QuotaMember>,
}

#[derive(Default, Serialize, Deserialize, Debug)]
#[serde(default)]
struct QuotaMember {
    quota: Uuid,
    //priority: Option<u32>,
}

#[derive(Default, Serialize, Deserialize, Debug)]
struct Quota {
    label: String,
    uuid: Uuid,
    limits: Vec<Limit>,
}

pub fn api_router() -> Router {
    let state = AppState::new();

    let openai_routes = Router::new()
        .route("/chat/completions", post(model_request))
        .route("/edits", post(model_request))
        .route("/completions", post(model_request))
        .route("/moderations", post(model_request))
        .route("/embeddings", post(model_request));

    //let admin_routes = Router::new().with_state(state);

    Router::new()
        .nest("/v1/", openai_routes)
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state)
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

async fn model_request(
    //State(state): State<AppState>,
    Extension(authenticated): Extension<FlattenedAppState>,
    headers: HeaderMap,
    Json(payload): Json<ModelRequest>,
) -> Result<Response, StatusCode> {
    todo!()
}

/* TODO: Add /admin/ API for configuration changes

async fn get_users() {}

async fn get_user() {}

async fn create_user() {}

async fn update_user_put() {}

async fn update_user_patch() {}

async fn delete_user() {}

async fn get_roles() {}

async fn get_role() {}

async fn create_role() {}

async fn update_role_put() {}

async fn update_role_patch() {}

async fn delete_role() {}

async fn get_models() {}

async fn get_model() {}

async fn create_model() {}

async fn delete_model() {}

async fn get_quotas() {}

async fn get_quota() {}

async fn create_quota() {}

async fn delete_quota() {}
*/
