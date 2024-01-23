use serde::{Deserialize, Serialize};
use std::{collections::HashMap, hash::Hash, sync::Arc};
use uuid::Uuid;

use reqwest::Client;
use tokio::sync::{mpsc, RwLock};

use crate::{openai_client, router};

use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
    routing::{get, post},
    Json, Router,
};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct User {
    pub label: String,
    pub uuid: Uuid,
    pub api_key: String,
    pub roles: Vec<Uuid>,
    pub priority: usize,
    pub quota: Quota,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Role {
    pub label: String,
    pub uuid: Uuid,
    pub models: Vec<Uuid>,
    pub default_models: Vec<Uuid>,
    pub priority: usize,
    pub quota: Quota,
}

#[derive(Serialize, Deserialize)]
pub struct Model {
    pub label: String,
    pub uuid: Uuid,
    pub api: router::ModelAPI,
    pub quota: Quota,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Quota {
    pub requests_per_minute: Option<usize>,
    pub requests_per_day: Option<usize>,
    pub tokens_per_minute: Option<usize>,
    pub tokens_per_day: Option<usize>,
    pub max_queue_size: usize,
}

/*

Authorization: Bearer OPENAI_API_KEY
[optional] OpenAI-Organization: org-Kew3wyVexePADOHIgJSK7Hsl

SEE https://platform.openai.com/docs/api-reference/streaming



*/

#[derive(Clone)]
struct AppState {
    users: Arc<RwLock<HashMap<Uuid, User>>>,
    roles: Arc<RwLock<HashMap<Uuid, Role>>>,
    models: Arc<RwLock<HashMap<Uuid, Model>>>,
}

pub fn api_router() -> Router {
    Router::new()
        .route("/v1/chat/completions", post(openai_model_request))
        .route("/v1/completions", post(openai_model_request))
        .with_state(AppState {
            users: Arc::new(RwLock::new(HashMap::new())),
            roles: Arc::new(RwLock::new(HashMap::new())),
            models: Arc::new(RwLock::new(HashMap::new())),
            //limiters: Arc::new(RwLock::new(HashMap::new())),
        })
}

//async fn authenticate(State(state): State<AppState>, mut req: Request, next: Next) -> Response {}

async fn openai_model_request(
    State(state): State<AppState>,
    Json(payload): Json<router::ModelRequest>,
) -> (StatusCode, Json<router::ModelResponse>) {
    /* Steps:
    1. auth
    2.
     */

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
