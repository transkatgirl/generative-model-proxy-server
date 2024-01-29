use std::{collections::HashMap, fmt::Debug, hash::Hash, ops::Deref, sync::Arc};

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
use tokio::sync::{mpsc, RwLock};
use tower_http::auth;
use uuid::Uuid;

mod limiter;
mod model;
mod queue;

use limiter::Limit;
use model::{ModelAPI, ModelRequest, ModelResponse, PackagedRequest};

#[derive(Default, Serialize, Deserialize, Debug)]
#[serde(default)]
struct User {
    label: String,
    uuid: Uuid,

    api_key: String,
    roles: Vec<Uuid>,

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
    label: String,
    uuid: Uuid,

    api: ModelAPI,

    quota: Vec<QuotaMember>,
}

#[derive(Default, Serialize, Deserialize, Debug)]
#[serde(default)]
struct QuotaMember {
    quota: Uuid,
    blocking: bool,
    priority: u32,
}

#[derive(Default, Serialize, Deserialize, Debug)]
#[serde(default)]
struct Quota {
    uuid: Uuid,
    limits: Vec<Limit>,
}

/*

Authorization: Bearer OPENAI_API_KEY
[optional] OpenAI-Organization: org-Kew3wyVexePADOHIgJSK7Hsl

SEE https://platform.openai.com/docs/api-reference/streaming



*/

type APIKeyMap = Arc<RwLock<HashMap<Vec<u8>, Arc<RwLock<User>>>>>;

#[derive(Clone)]
struct AppState {
    api_keys: APIKeyMap,
    users: Arc<RwLock<HashMap<Uuid, Arc<RwLock<User>>>>>,
    roles: Arc<RwLock<HashMap<Uuid, Arc<RwLock<Role>>>>>,
    quotas: Arc<RwLock<HashMap<Uuid, Arc<Quota>>>>,
    models: Arc<RwLock<HashMap<Uuid, (Arc<Model>, mpsc::UnboundedSender<PackagedRequest>)>>>,
}

struct Authenticated {
    user: Arc<User>,
    roles: Vec<Arc<Role>>,
    quotas: Vec<Arc<Quota>>,
}

pub fn api_router() -> Router {
    let state = AppState {
        api_keys: Arc::new(RwLock::new(HashMap::new())),
        users: Arc::new(RwLock::new(HashMap::new())),
        roles: Arc::new(RwLock::new(HashMap::new())),
        quotas: Arc::new(RwLock::new(HashMap::new())),
        models: Arc::new(RwLock::new(HashMap::new())),
    };

    let openai_routes = Router::new()
        .route("/chat/completions", post(model_request))
        .route("/edits", post(model_request))
        .route("/completions", post(model_request))
        .route("/moderations", post(model_request))
        .route("/embeddings", post(model_request));

    Router::new()
        .nest("/v1/", openai_routes)
        .route_layer(middleware::from_fn_with_state(
            state.api_keys.clone(),
            authenticate,
        ))
        .with_state(state)
}

async fn authenticate(
    State(state): State<APIKeyMap>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if let Some(authorization) = request.headers().get("authorization") {
        let authorization = authorization.as_bytes().to_ascii_lowercase();

        match authorization
            .strip_prefix("basic".as_bytes())
            .or(authorization.strip_prefix("bearer".as_bytes()))
        {
            Some(api_key) => match state.read().await.get(api_key).map(Arc::clone) {
                Some(user) => {
                    request.extensions_mut().insert(user);
                    Ok(next.run(request).await)
                }
                None => Err(StatusCode::UNAUTHORIZED),
            },
            None => Err(StatusCode::UNAUTHORIZED),
        }
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

/*async fn parse_model_request(
    State(state): State<AppState>,
    Extension(current_user): Extension<Arc<RwLock<User>>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    /*match body::to_bytes(request.body(), usize::MAX).await {
        Ok(body) => {
            //if let Ok(json) = serde_json::from_slice::<router::ModelRequest>(body) {

            //}


            todo!()
        },
        Err(_) => Err(StatusCode::BAD_REQUEST)
    }*/

    todo!()
}*/

async fn model_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ModelRequest>,
) -> Result<Response, StatusCode> {
    todo!()
}

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

async fn update_model_put() {}

async fn update_model_patch() {}

async fn delete_model() {}
