use std::{clone::Clone, collections::{HashMap, HashSet}, fmt::Debug, hash::Hash, ops::Deref, sync::Arc};

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
use super::model::{ModelAPI, ModelAPIClient, CallableModelAPI, ModelRequest, ModelResponse};

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

type AppUser = Arc<RwLock<User>>;
type AppRole = Arc<RwLock<Role>>;
type AppQuota = Arc<(Quota, Limiter)>;
type AppModel = Arc<(Model, ModelAPIClient)>;
type AppRefList = Arc<RwLock<HashSet<AppReference>>>;

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
enum AppReference {
    User(Uuid),
    Role(Uuid),
    Quota(Uuid),
    Model(Uuid),
}

#[derive(Debug, Clone)]
struct AppState {
    users: Arc<RwLock<HashMap<Uuid, AppUser>>>,
    roles: Arc<RwLock<HashMap<Uuid, AppRole>>>,
    quotas: Arc<RwLock<HashMap<Uuid, AppQuota>>>,
    models: Arc<RwLock<HashMap<Uuid, AppModel>>>,

    api_keys: Arc<RwLock<HashMap<Vec<u8>, Uuid>>>,
    references: Arc<RwLock<HashMap<AppReference, AppRefList>>>,
    model_labels: Arc<RwLock<HashMap<String, Uuid>>>,
}

impl AppState {
    fn new() -> AppState {
        AppState {
            users: Arc::new(RwLock::new(HashMap::new())),
            roles: Arc::new(RwLock::new(HashMap::new())),
            quotas: Arc::new(RwLock::new(HashMap::new())),
            models: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(RwLock::new(HashMap::new())),
            references: Arc::new(RwLock::new(HashMap::new())),
            model_labels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn authenticate(&self, api_key: &[u8]) -> Option<FlattenedAppState> {
        let api_key_db = self.api_keys.read().await;
        let role_db = self.roles.read().await;
        let quota_db = self.quotas.read().await;
        let model_db = self.models.read().await;

        if let Some(uuid) = api_key_db.get(api_key) {
            if let Some(user) = self.get_user(uuid).await {
                let mut tags = Vec::new();
                let mut models = HashMap::new();
                let mut quotas = Vec::new();

                tags.push(user.uuid);
                for uuid in &user.models {
                    if let Some(model) = model_db.get(uuid) {
                        models.insert(model.0.label.clone(), model.clone());
                    }
                }
                for quota_member in &user.quotas {
                    if let Some(quota) = quota_db.get(&quota_member.quota) {
                        quotas.push(quota.clone());
                        tags.push(quota.0.uuid);
                    }
                }

                for uuid in &user.roles {
                    if let Some(role_ref) = role_db.get(uuid) {
                        let role = role_ref.read().await;

                        tags.push(role.uuid);
                        for uuid in &role.models {
                            if let Some(model) = model_db.get(uuid) {
                                models.insert(model.0.label.clone(), model.clone());
                            }
                        }
                        for quota_member in &role.quotas {
                            if let Some(quota) = quota_db.get(&quota_member.quota) {
                                quotas.push(quota.clone());
                                tags.push(quota.0.uuid);
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
                .insert(api_key.clone().as_bytes().to_vec(), user.uuid);
        }

        let references = self.references.read().await;

        for role in &user.roles {
            if let Some(refs) = references.get(&AppReference::Role(*role)) {
                refs.write().await.insert(AppReference::User(user.uuid));
            }
        }

        for model in &user.models {
            if let Some(refs) = references.get(&AppReference::Model(*model)) {
                refs.write().await.insert(AppReference::User(user.uuid));
            }
        }

        for quota_member in &user.quotas {
            if let Some(refs) = references.get(&AppReference::Quota(quota_member.quota)) {
                refs.write().await.insert(AppReference::User(user.uuid));
            }
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
            // TODO
        } else {
            self.add_user(user).await
        }
    }

    async fn remove_user(&self, uuid: &Uuid) -> Option<AppUser> {
        if let Some(user) = self.users.write().await.remove(uuid) {
            {
                let user = user.read().await;

                for api_key in &user.api_keys {
                    self.api_keys
                        .write()
                        .await
                        .remove(api_key.clone().as_bytes());
                }

                let references = self.references.read().await;

                for role in &user.roles {
                    if let Some(refs) = references.get(&AppReference::Role(*role)) {
                        refs.write().await.remove(&AppReference::User(user.uuid));
                    }
                }

                for model in &user.models {
                    if let Some(refs) = references.get(&AppReference::Model(*model)) {
                        refs.write().await.remove(&AppReference::User(user.uuid));
                    }
                }

                for quota_member in &user.quotas {
                    if let Some(refs) = references.get(&AppReference::Quota(quota_member.quota)) {
                        refs.write().await.remove(&AppReference::User(user.uuid));
                    }
                }
            }

            return Some(user);
        }

        None
    }

    async fn add_role(&self, role: Role) {
        let uuid = role.uuid;
        let role = Arc::new(RwLock::new(role));

        self.roles.write().await.insert(uuid, role.clone());
        self.references.write().await.insert(AppReference::Role(uuid), Arc::new(RwLock::new(HashSet::new())));

        let role = role.read().await;
        let references = self.references.read().await;

        for model in &role.models {
            if let Some(refs) = references.get(&AppReference::Model(*model)) {
                refs.write().await.insert(AppReference::Role(role.uuid));
            }
        }

        for quota_member in &role.quotas {
            if let Some(refs) = references.get(&AppReference::Quota(quota_member.quota)) {
                refs.write().await.insert(AppReference::Role(role.uuid));
            }
        }
    }

    async fn get_role(&self, uuid: &Uuid) -> Option<OwnedRwLockReadGuard<Role>> {
        if let Some(role) = self.roles.read().await.get(uuid) {
            role.clone().read_owned().await;
        }

        None
    }

    async fn update_role(&self, role: Role) {
        if let Some(app_role) = self.roles.read().await.get(&role.uuid) {
            // TODO
        } else {
            self.add_role(role).await
        }
    }

    async fn remove_role(&self, uuid: &Uuid) -> Option<AppRole> {
        if let Some(role) = self.roles.write().await.remove(uuid) {
            if let Some(refs) = self.references.write().await.remove(& AppReference::Role(*uuid)) {
                for reference in refs.write().await.drain() {
                    if let AppReference::User(uuid) = reference {
                        if let Some(user) = self.users.read().await.get(&uuid) {
                            let mut user = user.write().await;

                            let index = user.roles.iter().position(|r| *r == uuid);
                            if let Some(index) = index {
                                user.roles.remove(index);
                            }
                        }
                    }
                }
            }

            {
                let role = role.read().await;
                let references = self.references.read().await;

                for model in &role.models {
                    if let Some(refs) = references.get(&AppReference::Model(*model)) {
                        refs.write().await.remove(&AppReference::Role(role.uuid));
                    }
                }

                for quota_member in &role.quotas {
                    if let Some(refs) = references.get(&AppReference::Quota(quota_member.quota)) {
                        refs.write().await.remove(&AppReference::Role(role.uuid));
                    }
                }
            }

            return Some(role);
        }

        None
    }

    async fn add_quota(&self, quota: Quota) {
        let uuid = quota.uuid;
        let limiter = Limiter::new(&quota.limits);
        let quota = Arc::new((quota, limiter));

        self.quotas.write().await.insert(uuid, quota);
        self.references.write().await.insert(AppReference::Quota(uuid), Arc::new(RwLock::new(HashSet::new())));
    }

    async fn get_quota(&self, uuid: &Uuid) -> Option<AppQuota> {
        self.quotas.read().await.get(uuid).cloned()
    }

    async fn remove_quota(&self, uuid: &Uuid) -> Option<AppQuota> {
        if let Some(quota) = self.quotas.write().await.remove(uuid) {
            todo!()
        }

        None
    }

    async fn add_model(&self, model: Model) {
        let uuid = model.uuid;
        let client = model.api.init();
        let model = Arc::new((model, client));

        self.models.write().await.insert(uuid, model.clone());
        self.references.write().await.insert(AppReference::Model(uuid), Arc::new(RwLock::new(HashSet::new())));

        for quota_member in &model.0.quotas {
            if let Some(refs) = self.references.read().await.get(&AppReference::Quota(quota_member.quota)) {
                refs.write().await.insert(AppReference::Role(uuid));
            }
        }
    }

    async fn get_model(&self, uuid: &Uuid) -> Option<AppModel> {
        self.models.read().await.get(uuid).cloned()
    }

    async fn get_model_by_label(&self, label: &str) -> Option<AppModel> {
        if let Some(uuid) = self.model_labels.read().await.get(label) {
            return self.models.read().await.get(uuid).cloned();
        }

        None
    }

    async fn remove_model(&self, uuid: &Uuid) -> Option<AppModel> {
        if let Some(model) = self.models.write().await.remove(uuid) {
            todo!()
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
    if let Some(authorization) = request.headers().get("authorization") {
        let authorization = authorization.as_bytes().to_ascii_lowercase();

        match authorization
            .strip_prefix("basic".as_bytes())
            .or(authorization.strip_prefix("bearer".as_bytes()))
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
/*
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
 */
