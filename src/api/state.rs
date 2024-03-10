use std::{
    clone::Clone,
    collections::{hash_map::Entry, HashMap},
    fmt::Debug,
    sync::Arc,
    time::Instant,
};

use axum::http::StatusCode;
use reqwest::Client;
use serde_json::Value;
use tokio::sync::{OwnedRwLockReadGuard, RwLock};
use uuid::Uuid;

use super::{
    super::limiter::{Limiter, PendingRequestHandle},
    super::model::{self, ModelBackend, ModelError, ModelResponse, TaggedModelRequest, TokenUsage},
    Model, Quota, Role, User,
};

type AppUser = Arc<RwLock<User>>;
type AppRole = Arc<RwLock<Role>>;
type AppQuota = Arc<(RwLock<Quota>, Limiter)>;
type AppModel = Arc<RwLock<Model>>;

#[derive(Debug, Clone)]
pub(super) struct AppState {
    http_client: Client,

    users: Arc<RwLock<HashMap<Uuid, AppUser>>>,
    roles: Arc<RwLock<HashMap<Uuid, AppRole>>>,
    quotas: Arc<RwLock<HashMap<Uuid, AppQuota>>>,
    models: Arc<RwLock<HashMap<Uuid, AppModel>>>,

    api_keys: Arc<RwLock<HashMap<String, Uuid>>>,
}

// TODO: Add functions to save/load state from disk
// TODO: Figure out logging/metrics
impl AppState {
    #[tracing::instrument(level = "debug")]
    pub(super) fn new() -> AppState {
        AppState {
            http_client: model::get_configured_client().unwrap(),
            users: Arc::new(RwLock::new(HashMap::new())),
            roles: Arc::new(RwLock::new(HashMap::new())),
            quotas: Arc::new(RwLock::new(HashMap::new())),
            models: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn authenticate(
        &self,
        api_key: &str,
        arrived_at: Instant,
    ) -> Option<FlattenedAppState> {
        if let Some(uuid) = self.api_keys.read().await.get(api_key) {
            if let Some(user) = self.get_user(uuid).await {
                let mut tags = Vec::new();
                let mut models = HashMap::new();
                let mut quotas = Vec::new();
                let mut admin = user.admin;

                tags.push(user.uuid);
                for uuid in &user.models {
                    if let Some(model) = self.get_model_with_quotas(uuid).await {
                        models.insert(model.0.read().await.label.clone(), model.clone());
                    }
                }
                for quota_member in &user.quotas {
                    if let Some(quota) = self.get_quota(&quota_member.quota).await {
                        quotas.push(quota.clone());
                        tags.push(quota.0.read().await.uuid);
                    }
                }

                for uuid in &user.roles {
                    if let Some(role) = self.get_role(uuid).await {
                        tags.push(role.uuid);
                        if role.admin {
                            admin = true
                        }
                        for uuid in &role.models {
                            if let Some(model) = self.get_model_with_quotas(uuid).await {
                                models.insert(model.0.read().await.label.clone(), model.clone());
                            }
                        }
                        for quota_member in &role.quotas {
                            if let Some(quota) = self.get_quota(&quota_member.quota).await {
                                quotas.push(quota.clone());
                                tags.push(quota.0.read().await.uuid);
                            }
                        }
                    }
                }

                tags.push(Uuid::new_v4());

                return Some(FlattenedAppState {
                    admin,
                    tags: Arc::new(tags),
                    models: Arc::new(models),
                    quotas: Arc::new(quotas),
                    arrived_at,
                    http_client: self.http_client.clone(),
                });
            }
        }

        None
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn add_or_update_user(&self, user: User) -> bool {
        let uuid = user.uuid;

        match self.users.write().await.entry(uuid) {
            Entry::Occupied(entry) => {
                let mut app_user = entry.get().write().await;
                let mut api_keys = self.api_keys.write().await;

                for api_key in &app_user.api_keys {
                    api_keys.remove(api_key);
                }

                for api_key in &user.api_keys {
                    api_keys.insert(api_key.clone(), user.uuid);
                }

                *app_user = user;

                false
            }
            Entry::Vacant(entry) => {
                let user = Arc::new(RwLock::new(user));

                entry.insert(user.clone());

                for api_key in &user.read().await.api_keys {
                    self.api_keys.write().await.insert(api_key.clone(), uuid);
                }

                true
            }
        }
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn get_users_snapshot(&self) -> Vec<User> {
        let mut users = Vec::new();

        for (_, user) in self.users.read().await.iter() {
            users.push(user.read().await.to_owned());
        }

        users
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub(super) async fn get_user(&self, uuid: &Uuid) -> Option<OwnedRwLockReadGuard<User>> {
        if let Some(user) = self.users.read().await.get(uuid) {
            return Some(user.clone().read_owned().await);
        }

        None
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn update_user(&self, user: User) -> bool {
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

            false
        } else {
            self.add_or_update_user(user).await
        }
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn remove_user(&self, uuid: &Uuid) -> Option<AppUser> {
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

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn get_roles_snapshot(&self) -> Vec<Role> {
        let mut roles = Vec::new();

        for (_, role) in self.roles.read().await.iter() {
            roles.push(role.read().await.to_owned());
        }

        roles
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn add_or_update_role(&self, role: Role) -> bool {
        let uuid = role.uuid;
        let role = Arc::new(RwLock::new(role));

        self.roles.write().await.insert(uuid, role).is_none()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub(super) async fn get_role(&self, uuid: &Uuid) -> Option<OwnedRwLockReadGuard<Role>> {
        if let Some(role) = self.roles.read().await.get(uuid) {
            return Some(role.clone().read_owned().await);
        }

        None
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn update_role(&self, role: Role) -> bool {
        if let Some(app_role) = self.roles.read().await.get(&role.uuid) {
            let mut app_role = app_role.write().await;
            *app_role = role;

            false
        } else {
            self.add_or_update_role(role).await
        }
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn remove_role(&self, uuid: &Uuid) -> Option<AppRole> {
        if let Some(role) = self.roles.write().await.remove(uuid) {
            return Some(role);
        }

        None
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn add_or_replace_quota(&self, quota: Quota) -> bool {
        let uuid = quota.uuid;
        let limiter = Limiter::new(&quota.limits);
        let quota = Arc::new((RwLock::new(quota), limiter));

        self.quotas.write().await.insert(uuid, quota).is_none()
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn get_quotas_snapshot(&self) -> Vec<Quota> {
        let mut quotas = Vec::new();

        for (_, quota) in self.quotas.read().await.iter() {
            quotas.push(quota.0.read().await.to_owned());
        }

        quotas
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub(super) async fn get_quota(&self, uuid: &Uuid) -> Option<AppQuota> {
        self.quotas.read().await.get(uuid).cloned()
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn update_quota_label(&self, uuid: &Uuid, label: String) -> Option<()> {
        if let Some(quota) = self.quotas.read().await.get(uuid) {
            let mut quota = quota.0.write().await;
            quota.label = label;
            return Some(());
        }

        None
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn remove_quota(&self, uuid: &Uuid) -> Option<AppQuota> {
        if let Some(quota) = self.quotas.write().await.remove(uuid) {
            return Some(quota);
        }

        None
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn add_or_replace_model(&self, model: Model) -> bool {
        let uuid = model.uuid;
        let model = Arc::new(RwLock::new(model));

        self.models
            .write()
            .await
            .insert(uuid, model.clone())
            .is_none()
    }

    #[tracing::instrument(skip(self), level = "debug")]
    pub(super) async fn get_models_snapshot(&self) -> Vec<Model> {
        let mut models = Vec::new();

        for (_, model) in self.models.read().await.iter() {
            models.push(model.read().await.to_owned());
        }

        models
    }

    #[tracing::instrument(skip(self), level = "trace")]
    pub(super) async fn get_model(&self, uuid: &Uuid) -> Option<AppModel> {
        self.models.read().await.get(uuid).cloned()
    }

    #[tracing::instrument(skip(self), level = "trace")]
    async fn get_model_with_quotas(&self, uuid: &Uuid) -> Option<(AppModel, Vec<AppQuota>)> {
        if let Some(model) = self.get_model(uuid).await {
            let mut quotas = Vec::new();

            for quota_member in &model.read().await.quotas {
                if let Some(quota) = self.get_quota(&quota_member.quota).await {
                    quotas.push(quota)
                }
            }

            return Some((model, quotas));
        }

        None
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn update_model_label(&self, uuid: &Uuid, label: String) -> Option<()> {
        if let Some(model) = self.models.read().await.get(uuid) {
            let mut model = model.write().await;
            model.label = label;
            return Some(());
        }

        None
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn remove_model(&self, uuid: &Uuid) -> Option<AppModel> {
        if let Some(model) = self.models.write().await.remove(uuid) {
            return Some(model);
        }

        None
    }
}

#[derive(Debug, Clone)]
pub(super) struct FlattenedAppState {
    pub(super) admin: bool,
    pub(super) tags: Arc<Vec<Uuid>>,
    models: Arc<HashMap<String, (AppModel, Vec<AppQuota>)>>,
    quotas: Arc<Vec<AppQuota>>,
    http_client: Client,
    arrived_at: Instant,
}

impl FlattenedAppState {
    #[tracing::instrument(level = "debug")]
    pub(super) async fn model_request(&self, request: Value) -> (StatusCode, Value) {
        let request = TaggedModelRequest::new(self.tags.clone(), request);

        let model_label = match request.get_model() {
            Some(label) => label,
            None => return from_model_error(ModelError::UnknownModel),
        };

        if let Some((model, model_quotas)) = self.models.get(model_label) {
            let model = model.read().await;

            let mut request_handles = Vec::new();

            let tokens = model
                .api
                .get_max_tokens()
                .map(|max_tokens| max_tokens as u32 * request.get_count() as u32)
                .unwrap_or(request.get_count() as u32);

            for quota in self.quotas.iter() {
                match quota.1.token_request(tokens, self.arrived_at).await {
                    Some(handle) => {
                        request_handles.push((quota.clone(), handle));
                    }
                    None => return from_model_error(ModelError::UserRateLimit),
                }
            }
            for quota in model_quotas {
                match quota.1.token_request(tokens, self.arrived_at).await {
                    Some(handle) => {
                        request_handles.push((quota.clone(), handle));
                    }
                    None => return from_model_error(ModelError::UserRateLimit),
                }
            }

            let response = model.api.generate(&self.http_client, request).await;

            if let Some(usage) = response.usage {
                for (quota, handle) in request_handles {
                    quota
                        .1
                        .token_request_finalize(usage.total as u32, handle)
                        .await;
                }
            }

            (response.status, response.response)
        } else {
            from_model_error(ModelError::UnknownModel)
        }
    }
}

fn from_model_error(error: ModelError) -> (StatusCode, Value) {
    let response = ModelResponse::from_error(error);
    (response.status, response.response)
}
