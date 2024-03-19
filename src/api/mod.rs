use std::{clone::Clone, collections::HashSet, fmt::Debug, iter, time::Instant};

use axum::{
    extract::{DefaultBodyLimit, Extension, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    Router,
};

use fast32::base64::RFC4648;
use http::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use serde::{Deserialize, Serialize};
use tokio::time;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

mod admin;
mod state;

use state::{RelatedToItem, RelatedToItemSet};

use crate::limiter::{self, LimiterResult};

use self::state::{DatabaseFunctionResult, DatabaseValueResult};

use super::{
    limiter::Limit,
    model::{ModelBackend, ModelError, ModelRequest, ModelResponse, RequestType},
    AppState,
};

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
struct User {
    label: String,
    uuid: Uuid,

    admin: bool,

    api_keys: HashSet<String>,
    roles: HashSet<Uuid>,

    models: HashSet<Uuid>,
    quotas: HashSet<Uuid>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
struct Role {
    label: String,
    uuid: Uuid,

    admin: bool,

    models: HashSet<Uuid>,
    quotas: HashSet<Uuid>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Model {
    #[serde(default)]
    label: String,

    #[serde(default)]
    uuid: Uuid,

    #[serde(default)]
    name: String,

    #[serde(default)]
    types: HashSet<RequestType>,

    api: ModelBackend,

    #[serde(default)]
    quotas: HashSet<Uuid>,
}

#[derive(Default, Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
struct Quota {
    label: String,
    uuid: Uuid,

    limits: Vec<Limit>,
}

#[derive(Debug, Clone)]
struct Authenticated {
    timestamp: Instant,
    admin: bool,
    user: User,
    roles: Vec<Role>,
}

#[tracing::instrument(level = "debug", skip(state))]
pub fn api_router(state: AppState) -> Router {
    Router::new()
        .fallback(model_request)
        .nest("/admin", admin::admin_router())
        .with_state(state.clone())
        .layer(
            ServiceBuilder::new()
                .layer(DefaultBodyLimit::max(16_777_216))
                .layer(TraceLayer::new_for_http())
                .layer(middleware::map_response(modify_response))
                .layer(middleware::from_fn_with_state(state, authenticate)),
        )
}

#[tracing::instrument(level = "debug", skip(state, next), ret)]
async fn authenticate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ModelError> {
    let timestamp = Instant::now();

    match request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|header_value| {
            header_value.to_str().ok().and_then(|header_string| {
                header_string
                    .strip_prefix("Bearer ")
                    .map(|value| value.to_string())
                    .or_else(|| {
                        header_string
                            .strip_prefix("Basic ")
                            .and_then(|auth_encoded| RFC4648.decode_str(auth_encoded).ok())
                            .and_then(|auth_decoded| {
                                String::from_utf8(auth_decoded).ok().and_then(|value| {
                                    value.strip_prefix(':').map(|value| value.to_string())
                                })
                            })
                    })
            })
        }) {
        Some(api_key) => {
            if state.is_table_empty("users") && api_key == "setup-key" {
                request.extensions_mut().insert(Authenticated {
                    timestamp,
                    admin: true,
                    user: User::default(),
                    roles: Vec::new(),
                });

                return Ok(next.run(request).await);
            }

            match state.get_related_item::<_, Uuid, User>(("api_keys", "users"), &api_key) {
                DatabaseValueResult::Success(user) => {
                    let roles: Vec<Uuid> = user.roles.iter().copied().collect();
                    match state.get_items_skip_missing::<_, Role>("roles", &roles) {
                        DatabaseValueResult::Success(roles) => {
                            let mut admin = user.admin;

                            for role in &roles {
                                if role.admin {
                                    admin = true;
                                }
                            }

                            request.extensions_mut().insert(Authenticated {
                                timestamp,
                                admin,
                                user,
                                roles,
                            })
                        }
                        DatabaseValueResult::NotFound => return Err(ModelError::AuthInvalid),
                        DatabaseValueResult::BackendError => return Err(ModelError::InternalError),
                    };

                    Ok(next.run(request).await)
                }
                DatabaseValueResult::NotFound => Err(ModelError::AuthInvalid),
                DatabaseValueResult::BackendError => Err(ModelError::InternalError),
            }
        }
        None => Err(ModelError::AuthMissing),
    }
}

#[tracing::instrument(level = "debug", skip(next), ret)]
async fn authenticate_admin(
    Extension(auth): Extension<Authenticated>,
    request: Request,
    next: Next,
) -> Result<Response, ModelError> {
    if auth.admin {
        return Ok(next.run(request).await);
    }

    Err(ModelError::UnknownEndpoint)
}

async fn modify_response<B>(mut response: Response<B>) -> Response<B> {
    if response.status() == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            WWW_AUTHENTICATE,
            "Basic realm=\"Please enter your API key into the password field.\", charset=\"UTF-8\""
                .parse()
                .unwrap(),
        );
    }

    response
}

#[tracing::instrument(level = "trace", skip(state), ret)]
async fn model_request(
    Extension(auth): Extension<Authenticated>,
    State(state): State<AppState>,
    request: Result<ModelRequest, ModelError>,
) -> Result<ModelResponse, ModelError> {
    let mut request = match request {
        Ok(request) => request,
        Err(error) => return Err(error),
    };

    let models_result = state.get_items_skip_missing::<_, Model>(
        "models",
        &auth
            .user
            .models
            .iter()
            .chain(auth.roles.iter().flat_map(|role| role.models.iter()))
            .cloned()
            .collect::<Vec<_>>(),
    );

    let model_name = request.get_model().unwrap_or_default();
    let model = match models_result {
        DatabaseValueResult::Success(models) => match models
            .iter()
            .find(|model| model.types.contains(&request.r#type) && model.name == model_name)
        {
            Some(model) => model.clone(),
            None => return Err(ModelError::UnknownModel),
        },
        DatabaseValueResult::NotFound => return Err(ModelError::UnknownModel),
        DatabaseValueResult::BackendError => return Err(ModelError::InternalError),
    };

    if request.get_max_tokens().unwrap_or(1) > model.api.get_max_tokens().unwrap_or(1) {
        return Err(ModelError::UserRateLimit);
    }

    request.set_user(auth.user.uuid);

    let quotas: HashSet<Uuid> = auth
        .user
        .quotas
        .iter()
        .chain(auth.roles.iter().flat_map(|role| role.quotas.iter()))
        .chain(model.quotas.iter())
        .copied()
        .collect();
    let quotas: Vec<Uuid> = quotas.iter().copied().collect();

    request.tags = iter::once(auth.user.uuid)
        .chain(auth.roles.iter().map(|role| role.uuid))
        .chain(quotas.clone())
        .chain(iter::once(model.uuid))
        .chain(iter::once(Uuid::new_v4()))
        .collect();

    let limiter_request = limiter::Request {
        arrived_at: auth.timestamp,
        estimated_tokens: model
            .api
            .get_max_tokens()
            .map(|max_tokens| {
                request
                    .get_max_tokens()
                    .map(|request_max_tokens| request_max_tokens.min(max_tokens))
                    .unwrap_or(max_tokens)
            })
            .unwrap_or(1),
    };

    let limit_request = |quota: &mut Quota| {
        let mut wait_until = Instant::now();

        for limit in &mut quota.limits {
            match limit.request(&state.clock, &limiter_request) {
                LimiterResult::Ready => {}
                LimiterResult::WaitUntil(timestamp) => wait_until = wait_until.max(timestamp),
                LimiterResult::Oversized => return Err(ModelError::UserRateLimit),
            }
        }

        Ok(wait_until)
    };

    match state.modify_items_skip_missing("quotas", &quotas, limit_request) {
        DatabaseFunctionResult::Success(timestamps) => {
            if let Some(wait_until) = timestamps.iter().max().cloned() {
                time::sleep_until(time::Instant::from_std(wait_until)).await
            }
        }
        DatabaseFunctionResult::FunctionError(error) => return Err(error),
        DatabaseFunctionResult::BackendError => return Err(ModelError::InternalError),
    }

    let response = model.api.generate(&state.http, request).await;

    if let Some(usage) = &response.usage {
        let limiter_response = limiter::Response {
            request: limiter_request,
            actual_tokens: usage.total,
        };

        let limit_response = |quota: &mut Quota| {
            let mut wait_until = Instant::now();

            for limit in &mut quota.limits {
                match limit.response(&state.clock, &limiter_response) {
                    LimiterResult::Ready => {}
                    LimiterResult::WaitUntil(timestamp) => wait_until = wait_until.max(timestamp),
                    LimiterResult::Oversized => return Err(ModelError::UserRateLimit),
                }
            }

            Ok(wait_until)
        };

        match state.modify_items_skip_missing("quotas", &quotas, limit_response) {
            DatabaseFunctionResult::Success(timestamps) => {
                if let Some(wait_until) = timestamps.iter().max().cloned() {
                    time::sleep_until(time::Instant::from_std(wait_until)).await
                }
            }
            DatabaseFunctionResult::FunctionError(error) => return Err(error),
            DatabaseFunctionResult::BackendError => return Err(ModelError::InternalError),
        }
    }

    Ok(response)
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum StringOrUuid {
    Uuid(Uuid),
    String(String),
}

impl RelatedToItemSet for User {
    type Key = StringOrUuid;

    fn get_keys(&self, table: &str) -> Vec<Self::Key> {
        match table {
            "roles" => self
                .roles
                .iter()
                .map(|item| StringOrUuid::Uuid(*item))
                .collect(),
            "models" => self
                .models
                .iter()
                .map(|item| StringOrUuid::Uuid(*item))
                .collect(),
            "quotas" => self
                .quotas
                .iter()
                .map(|item| StringOrUuid::Uuid(*item))
                .collect(),
            _ => self
                .api_keys
                .iter()
                .map(|item| StringOrUuid::String(item.clone()))
                .collect(),
        }
    }
}

impl RelatedToItem for Uuid {
    type Key = Uuid;

    fn get_key(&self, _id: &str) -> Self::Key {
        *self
    }
}
