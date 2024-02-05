use std::{
    clone::Clone,
    collections::{HashMap, HashSet},
    fmt::Debug,
    hash::Hash,
    ops::Deref,
    sync::Arc,
};

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

use super::limiter::{Limit, Limiter};
use super::model::{CallableModelAPI, ModelAPI, ModelAPIClient, ModelRequest, ModelResponse};

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

type AppUser = Arc<RwLock<User>>;
type AppRole = Arc<RwLock<Role>>;
type AppQuota = Arc<(RwLock<Quota>, Limiter)>;
type AppModel = Arc<(RwLock<Model>, ModelAPIClient)>;

#[derive(Debug, Clone)]
struct AppState {
    users: Arc<RwLock<HashMap<Uuid, AppUser>>>,
    roles: Arc<RwLock<HashMap<Uuid, AppRole>>>,
    quotas: Arc<RwLock<HashMap<Uuid, AppQuota>>>,
    models: Arc<RwLock<HashMap<Uuid, AppModel>>>,

    api_keys: Arc<RwLock<HashMap<String, Uuid>>>,
}

impl AppState {
    fn new() -> AppState {
        AppState {
            users: Arc::new(RwLock::new(HashMap::new())),
            roles: Arc::new(RwLock::new(HashMap::new())),
            quotas: Arc::new(RwLock::new(HashMap::new())),
            models: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn authenticate(&self, api_key: &str) -> Option<FlattenedAppState> {
        if let Some(uuid) = self.api_keys.read().await.get(api_key) {
            if let Some(user) = self.get_user(uuid).await {
                let mut tags = Vec::new();
                let mut models = HashMap::new();
                let mut quotas = Vec::new();

                tags.push(user.uuid);
                for uuid in &user.models {
                    if let Some(model) = self.models.read().await.get(uuid) {
                        models.insert(model.0.read().await.label.clone(), model.clone());
                    }
                }
                for quota_member in &user.quotas {
                    if let Some(quota) = self.quotas.read().await.get(&quota_member.quota) {
                        quotas.push(quota.clone());
                        tags.push(quota.0.read().await.uuid);
                    }
                }

                for uuid in &user.roles {
                    if let Some(role_ref) = self.roles.read().await.get(uuid) {
                        let role = role_ref.read().await;

                        tags.push(role.uuid);
                        for uuid in &role.models {
                            if let Some(model) = self.models.read().await.get(uuid) {
                                models.insert(model.0.read().await.label.clone(), model.clone());
                            }
                        }
                        for quota_member in &role.quotas {
                            if let Some(quota) = self.quotas.read().await.get(&quota_member.quota) {
                                quotas.push(quota.clone());
                                tags.push(quota.0.read().await.uuid);
                            }
                        }
                    }
                }

                return Some(FlattenedAppState {
                    tags: Arc::new(tags),
                    models: Arc::new(models),
                    quotas: Arc::new(quotas),
                });
            }
        }

        None
    }

    async fn add_user(&self, user: User) {
        let uuid = user.uuid;
        let user = Arc::new(RwLock::new(user));

        self.users.write().await.insert(uuid, user.clone());

        let user = user.read().await;

        for api_key in &user.api_keys {
            self.api_keys
                .write()
                .await
                .insert(api_key.clone(), user.uuid);
        }
    }

    async fn get_user(&self, uuid: &Uuid) -> Option<OwnedRwLockReadGuard<User>> {
        if let Some(user) = self.users.read().await.get(uuid) {
            user.clone().read_owned().await;
        }

        None
    }

    async fn update_user(&self, user: User) {
        if let Some(app_user) = self.users.read().await.get(&user.uuid) {
            let mut app_user = app_user.write().await;
            let mut api_keys = self.api_keys.write().await;

            for api_key in &app_user.api_keys {
                api_keys.remove(api_key);
            }

            for api_key in &user.api_keys {
                api_keys.insert(api_key.clone(), user.uuid);
            }

            *app_user = user;
        } else {
            self.add_user(user).await
        }
    }

    async fn remove_user(&self, uuid: &Uuid) -> Option<AppUser> {
        if let Some(user) = self.users.write().await.remove(uuid) {
            {
                let user = user.read().await;

                for api_key in &user.api_keys {
                    self.api_keys.write().await.remove(api_key);
                }
            }

            return Some(user);
        }

        None
    }

    async fn add_role(&self, role: Role) {
        let uuid = role.uuid;
        let role = Arc::new(RwLock::new(role));

        self.roles.write().await.insert(uuid, role);
    }

    async fn get_role(&self, uuid: &Uuid) -> Option<OwnedRwLockReadGuard<Role>> {
        if let Some(role) = self.roles.read().await.get(uuid) {
            role.clone().read_owned().await;
        }

        None
    }

    async fn update_role(&self, role: Role) {
        if let Some(app_role) = self.roles.read().await.get(&role.uuid) {
            let mut app_role = app_role.write().await;
            *app_role = role;
        } else {
            self.add_role(role).await
        }
    }

    async fn remove_role(&self, uuid: &Uuid) -> Option<AppRole> {
        if let Some(role) = self.roles.write().await.remove(uuid) {
            return Some(role);
        }

        None
    }

    async fn add_quota(&self, quota: Quota) {
        let uuid = quota.uuid;
        let limiter = Limiter::new(&quota.limits);
        let quota = Arc::new((RwLock::new(quota), limiter));

        self.quotas.write().await.insert(uuid, quota);
    }

    async fn get_quota(&self, uuid: &Uuid) -> Option<AppQuota> {
        self.quotas.read().await.get(uuid).cloned()
    }

    async fn update_quota_label(&self, uuid: &Uuid, label: String) -> Option<()> {
        if let Some(quota) = self.quotas.read().await.get(uuid) {
            let mut quota = quota.0.write().await;
            quota.label = label;
            return Some(());
        }

        None
    }

    async fn remove_quota(&self, uuid: &Uuid) -> Option<AppQuota> {
        if let Some(quota) = self.quotas.write().await.remove(uuid) {
            return Some(quota);
        }

        None
    }

    async fn add_model(&self, model: Model) {
        let uuid = model.uuid;
        let client = model.api.init();
        let model = Arc::new((RwLock::new(model), client));

        self.models.write().await.insert(uuid, model.clone());
    }

    async fn get_model(&self, uuid: &Uuid) -> Option<AppModel> {
        self.models.read().await.get(uuid).cloned()
    }

    async fn update_model_label(&self, uuid: &Uuid, label: String) -> Option<()> {
        if let Some(model) = self.models.read().await.get(uuid) {
            let mut model = model.0.write().await;
            model.label = label;
            return Some(());
        }

        None
    }

    async fn remove_model(&self, uuid: &Uuid) -> Option<AppModel> {
        if let Some(model) = self.models.write().await.remove(uuid) {
            return Some(model);
        }

        None
    }
}

#[derive(Debug, Clone)]
struct FlattenedAppState {
    tags: Arc<Vec<Uuid>>,
    models: Arc<HashMap<String, AppModel>>,
    quotas: Arc<Vec<AppQuota>>,
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
    State(state): State<AppState>,
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
