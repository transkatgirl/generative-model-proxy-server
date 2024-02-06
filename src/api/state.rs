use std::{
    clone::Clone,
    collections::{hash_map::Entry, HashMap},
    fmt::Debug,
    sync::Arc,
    time::Instant,
};

use axum::http::StatusCode;
use fast32::base32::CROCKFORD;
use ring::digest;
use tokio::sync::{OwnedRwLockReadGuard, RwLock};
use uuid::Uuid;

use super::{
    super::limiter::{Limiter, PendingRequestHandle},
    super::model::{
        CallableModelAPI, ModelAPIClient, ModelRequest, ModelResponse, ResponseStatus,
        RoutableModelRequest, RoutableModelResponse,
    },
    Model, Permissions, Quota, Role, User,
};

type AppUser = Arc<RwLock<User>>;
type AppRole = Arc<RwLock<Role>>;
type AppQuota = Arc<(RwLock<Quota>, Limiter)>;
type AppModel = Arc<(RwLock<Model>, ModelAPIClient)>;

#[derive(Debug, Clone)]
pub(super) struct AppState {
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
            users: Arc::new(RwLock::new(HashMap::new())),
            roles: Arc::new(RwLock::new(HashMap::new())),
            quotas: Arc::new(RwLock::new(HashMap::new())),
            models: Arc::new(RwLock::new(HashMap::new())),
            api_keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    #[tracing::instrument(level = "debug")]
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
                let mut perms = user.perms;

                tags.push(user.uuid);
                for uuid in &user.models {
                    if let Some(model) = self.get_model_with_quotas(uuid).await {
                        models.insert(model.0 .0.read().await.label.clone(), model.clone());
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
                        if role.perms.server_admin {
                            perms.server_admin = true
                        }
                        if role.perms.view_metrics {
                            perms.view_metrics = true
                        }
                        if role.perms.sensitive {
                            perms.sensitive = true
                        }
                        for uuid in &role.models {
                            if let Some(model) = self.get_model_with_quotas(uuid).await {
                                models.insert(model.0 .0.read().await.label.clone(), model.clone());
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

                return Some(FlattenedAppState {
                    perms,
                    tags: Arc::new(tags),
                    models: Arc::new(models),
                    quotas: Arc::new(quotas),
                    arrived_at,
                });
            }
        }

        None
    }

    #[tracing::instrument(level = "debug")]
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

    #[tracing::instrument(level = "debug")]
    pub(super) async fn get_users_snapshot(&self) -> Vec<User> {
        let mut users = Vec::new();

        for (_, user) in self.users.read().await.iter() {
            users.push(user.read().await.to_owned());
        }

        users
    }

    #[tracing::instrument(level = "trace")]
    pub(super) async fn get_user(&self, uuid: &Uuid) -> Option<OwnedRwLockReadGuard<User>> {
        if let Some(user) = self.users.read().await.get(uuid) {
            return Some(user.clone().read_owned().await);
        }

        None
    }

    #[tracing::instrument(level = "debug")]
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

    #[tracing::instrument(level = "debug")]
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

    #[tracing::instrument(level = "debug")]
    pub(super) async fn get_roles_snapshot(&self) -> Vec<Role> {
        let mut roles = Vec::new();

        for (_, role) in self.roles.read().await.iter() {
            roles.push(role.read().await.to_owned());
        }

        roles
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn add_or_update_role(&self, role: Role) -> bool {
        let uuid = role.uuid;
        let role = Arc::new(RwLock::new(role));

        self.roles.write().await.insert(uuid, role).is_none()
    }

    #[tracing::instrument(level = "trace")]
    pub(super) async fn get_role(&self, uuid: &Uuid) -> Option<OwnedRwLockReadGuard<Role>> {
        if let Some(role) = self.roles.read().await.get(uuid) {
            return Some(role.clone().read_owned().await);
        }

        None
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn update_role(&self, role: Role) -> bool {
        if let Some(app_role) = self.roles.read().await.get(&role.uuid) {
            let mut app_role = app_role.write().await;
            *app_role = role;

            false
        } else {
            self.add_or_update_role(role).await
        }
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn remove_role(&self, uuid: &Uuid) -> Option<AppRole> {
        if let Some(role) = self.roles.write().await.remove(uuid) {
            return Some(role);
        }

        None
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn add_or_replace_quota(&self, quota: Quota) -> bool {
        let uuid = quota.uuid;
        let limiter = Limiter::new(&quota.limits);
        let quota = Arc::new((RwLock::new(quota), limiter));

        self.quotas.write().await.insert(uuid, quota).is_none()
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn get_quotas_snapshot(&self) -> Vec<Quota> {
        let mut quotas = Vec::new();

        for (_, quota) in self.quotas.read().await.iter() {
            quotas.push(quota.0.read().await.to_owned());
        }

        quotas
    }

    #[tracing::instrument(level = "trace")]
    pub(super) async fn get_quota(&self, uuid: &Uuid) -> Option<AppQuota> {
        self.quotas.read().await.get(uuid).cloned()
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn update_quota_label(&self, uuid: &Uuid, label: String) -> Option<()> {
        if let Some(quota) = self.quotas.read().await.get(uuid) {
            let mut quota = quota.0.write().await;
            quota.label = label;
            return Some(());
        }

        None
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn remove_quota(&self, uuid: &Uuid) -> Option<AppQuota> {
        if let Some(quota) = self.quotas.write().await.remove(uuid) {
            return Some(quota);
        }

        None
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn add_or_replace_model(&self, model: Model) -> bool {
        let uuid = model.uuid;
        let client = model.api.init();
        let model = Arc::new((RwLock::new(model), client));

        self.models
            .write()
            .await
            .insert(uuid, model.clone())
            .is_none()
    }

    #[tracing::instrument(level = "debug")]
    pub(super) async fn get_models_snapshot(&self) -> Vec<Model> {
        let mut models = Vec::new();

        for (_, model) in self.models.read().await.iter() {
            models.push(model.0.read().await.to_owned());
        }

        models
    }

    #[tracing::instrument(level = "trace")]
    pub(super) async fn get_model(&self, uuid: &Uuid) -> Option<AppModel> {
        self.models.read().await.get(uuid).cloned()
    }

    #[tracing::instrument(level = "trace")]
    async fn get_model_with_quotas(&self, uuid: &Uuid) -> Option<(AppModel, Vec<AppQuota>)> {
        if let Some(model) = self.get_model(uuid).await {
            let mut quotas = Vec::new();

            for quota_member in &model.0.read().await.quotas {
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
            let mut model = model.0.write().await;
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
    pub(super) perms: Permissions,
    pub(super) tags: Arc<Vec<Uuid>>,
    models: Arc<HashMap<String, (AppModel, Vec<AppQuota>)>>,
    quotas: Arc<Vec<AppQuota>>,
    arrived_at: Instant,
}

impl FlattenedAppState {
    #[tracing::instrument(level = "debug")]
    pub(super) async fn model_request(
        &self,
        request: ModelRequest,
    ) -> Result<(StatusCode, ModelResponse), StatusCode> {
        let model_label = request.get_model();

        if let Some(model) = self.models.get(&model_label) {
            let (model, model_client, model_quotas) =
                (&model.0 .0.read().await, &model.0 .1, &model.1);

            let mut request_handles = Vec::new();

            match model.api.get_context_len() {
                Some(context_len) => {
                    for quota in self.quotas.iter() {
                        match quota.1.token_request(context_len, self.arrived_at).await {
                            Some(handle) => {
                                request_handles.push((quota.clone(), Some(handle)));
                            }
                            None => return Err(StatusCode::TOO_MANY_REQUESTS),
                        }
                    }
                    for quota in model_quotas {
                        match quota.1.token_request(context_len, self.arrived_at).await {
                            Some(handle) => {
                                request_handles.push((quota.clone(), Some(handle)));
                            }
                            None => return Err(StatusCode::TOO_MANY_REQUESTS),
                        }
                    }
                }
                None => {
                    for quota in self.quotas.iter() {
                        quota.1.plain_request(self.arrived_at).await;
                        request_handles.push((quota.clone(), None))
                    }
                    for quota in model_quotas {
                        quota.1.plain_request(self.arrived_at).await;
                        request_handles.push((quota.clone(), None))
                    }
                }
            }

            let request_label = match self.perms.sensitive {
                true => "".to_string(),
                false => CROCKFORD
                    .encode(digest::digest(&digest::SHA256, self.tags[0].as_bytes()).as_ref()),
            };

            return match model
                .api
                .generate(model_client, &request_label, &model_label, request)
                .await
            {
                Ok(response) => {
                    if let Some(tokens) = response.get_token_count() {
                        for (quota, handle) in request_handles {
                            let handle =
                                handle.unwrap_or(PendingRequestHandle::new(self.arrived_at, 0));

                            quota.1.token_request_finalize(tokens, handle).await;
                        }
                    }

                    let status = match response.get_status() {
                        ResponseStatus::Success => StatusCode::OK,
                        ResponseStatus::InvalidRequest => StatusCode::BAD_REQUEST,
                        ResponseStatus::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
                        ResponseStatus::BadUpstream => StatusCode::BAD_GATEWAY,
                        ResponseStatus::ModelUnavailable => StatusCode::SERVICE_UNAVAILABLE,
                    };

                    return Ok((status, response));
                }
                Err(ResponseStatus::Success) => Err(StatusCode::OK),
                Err(ResponseStatus::InvalidRequest) => Err(StatusCode::BAD_REQUEST),
                Err(ResponseStatus::InternalError) => Err(StatusCode::INTERNAL_SERVER_ERROR),
                Err(ResponseStatus::BadUpstream) => Err(StatusCode::BAD_GATEWAY),
                Err(ResponseStatus::ModelUnavailable) => Err(StatusCode::SERVICE_UNAVAILABLE),
            };
        }

        Err(StatusCode::NOT_FOUND)
    }
}
