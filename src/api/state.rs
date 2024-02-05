use std::{clone::Clone, collections::HashMap, fmt::Debug, sync::Arc};

use tokio::sync::{OwnedRwLockReadGuard, RwLock};
use uuid::Uuid;

use super::super::limiter::Limiter;
use super::super::model::{CallableModelAPI, ModelAPIClient};
use super::{Model, Quota, Role, User};

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
    pub(super) async fn authenticate(&self, api_key: &str) -> Option<FlattenedAppState> {
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

    #[tracing::instrument(level = "debug")]
	pub(super) async fn add_user(&self, user: User) {
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

	#[tracing::instrument(level = "debug")]
    pub(super) async fn get_user(&self, uuid: &Uuid) -> Option<OwnedRwLockReadGuard<User>> {
        if let Some(user) = self.users.read().await.get(uuid) {
            user.clone().read_owned().await;
        }

        None
    }

	#[tracing::instrument(level = "debug")]
    pub(super) async fn update_user(&self, user: User) {
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
    pub(super) async fn add_role(&self, role: Role) {
        let uuid = role.uuid;
        let role = Arc::new(RwLock::new(role));

        self.roles.write().await.insert(uuid, role);
    }

	#[tracing::instrument(level = "debug")]
    pub(super) async fn get_role(&self, uuid: &Uuid) -> Option<OwnedRwLockReadGuard<Role>> {
        if let Some(role) = self.roles.read().await.get(uuid) {
            role.clone().read_owned().await;
        }

        None
    }

	#[tracing::instrument(level = "debug")]
    pub(super) async fn update_role(&self, role: Role) {
        if let Some(app_role) = self.roles.read().await.get(&role.uuid) {
            let mut app_role = app_role.write().await;
            *app_role = role;
        } else {
            self.add_role(role).await
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
    pub(super) async fn add_quota(&self, quota: Quota) {
        let uuid = quota.uuid;
        let limiter = Limiter::new(&quota.limits);
        let quota = Arc::new((RwLock::new(quota), limiter));

        self.quotas.write().await.insert(uuid, quota);
    }

	#[tracing::instrument(level = "debug")]
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
    pub(super) async fn add_model(&self, model: Model) {
        let uuid = model.uuid;
        let client = model.api.init();
        let model = Arc::new((RwLock::new(model), client));

        self.models.write().await.insert(uuid, model.clone());
    }

	#[tracing::instrument(level = "debug")]
    pub(super) async fn get_model(&self, uuid: &Uuid) -> Option<AppModel> {
        self.models.read().await.get(uuid).cloned()
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
    pub(super) tags: Arc<Vec<Uuid>>,
    models: Arc<HashMap<String, AppModel>>,
    pub(super) quotas: Arc<Vec<AppQuota>>,
}

impl FlattenedAppState {
	#[tracing::instrument(level = "debug")]
	pub(super) fn get_model(&self, label: &str) -> Option<AppModel> {
		self.models.get(label).cloned()
	}

	pub(super) fn get_request_label(&self) -> String {
		self.tags[0].as_simple().encode_lower(&mut Uuid::encode_buffer()).to_string()
	}
}