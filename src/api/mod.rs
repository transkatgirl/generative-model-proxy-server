use std::{clone::Clone, collections::HashSet, fmt::Debug, iter, time::Instant};

use axum::{
    async_trait,
    body::{self, Bytes},
    extract::{DefaultBodyLimit, Extension, FromRequest, Multipart, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Form, Json, Router,
};

use fast32::base64::RFC4648;
use http::{
    header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE},
    Method,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
    model::{ModelBackend, ModelError, ModelResponse, RequestType, TaggedModelRequest},
    AppState,
};

// TODO: Add API documentation

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

#[tracing::instrument(level = "trace", skip(state), ret)]
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

#[tracing::instrument(level = "trace", skip(auth), ret)]
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
    request: Result<TaggedModelRequest, ModelError>,
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

#[async_trait]
impl<S> FromRequest<S> for TaggedModelRequest
where
    Bytes: FromRequest<S>,
    S: Send + Sync,
{
    type Rejection = ModelError;

    #[tracing::instrument(level = "trace", skip(state), ret)]
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let r#type = match RequestType::try_from(req.uri()) {
            Ok(r#type) => r#type,
            Err(_) => return Err(ModelError::UnknownEndpoint),
        };

        if req.method() != Method::GET
            || req.method() != Method::HEAD
            || req.method() != Method::POST
        {
            return Err(ModelError::BadEndpointMethod);
        }

        let request = match req
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|header_value| {
                header_value
                    .to_str()
                    .map(|header_string| header_string.to_ascii_lowercase())
                    .ok()
            })
            .as_deref()
        {
            Some("application/x-www-form-urlencoded") => Form::from_request(req, state)
                .await
                .map(|value| value.0)
                .unwrap_or(Value::Null),
            Some("multipart/form-data") => match Multipart::from_request(req, state).await {
                Ok(mut multipart) => {
                    let mut json_fields = Vec::new();

                    while let Some(field) = multipart.next_field().await.unwrap() {
                        json_fields.push(json!({
                            "name": field.name(),
                            "file_name": field.file_name(),
                            "content_type": field.content_type(),
                            "headers": field.headers().iter().map(|(key, value)| {
                                (key.as_str(), value.to_str().map(|value| Value::String(value.to_string())).unwrap_or(Value::String(RFC4648.encode(value.as_bytes()))))
                            }).collect::<Vec<(_, Value)>>(),
                            "content": field.bytes().await.ok().map(|bytes| RFC4648.encode(bytes.as_ref())),
                        }))
                    }

                    Value::Array(json_fields)
                }
                Err(_) => Value::Null,
            },
            Some("application/json") => Json::from_request(req, state)
                .await
                .map(|value| value.0)
                .unwrap_or(Value::Null),
            Some(_) => body::to_bytes(req.into_body(), usize::MAX)
                .await
                .ok()
                .and_then(|body| Json::from_bytes(body.as_ref()).map(|value| value.0).ok())
                .unwrap_or(Value::Null),
            None => {
                if req.method() == Method::HEAD || req.method() == Method::GET {
                    Form::from_request(req, state)
                        .await
                        .map(|value| value.0)
                        .unwrap_or(Value::Null)
                } else {
                    body::to_bytes(req.into_body(), usize::MAX)
                        .await
                        .ok()
                        .and_then(|body| Json::from_bytes(body.as_ref()).map(|value| value.0).ok())
                        .unwrap_or(Value::Null)
                }
            }
        };

        if request == Value::Null {
            return Err(ModelError::BadRequest);
        }

        Ok(TaggedModelRequest::new(Vec::new(), r#type, request))
    }
}

impl IntoResponse for ModelResponse {
    fn into_response(self) -> axum::response::Response {
        if self.status == StatusCode::OK {
            if let Value::String(string) = &self.response {
                if let Ok(data) = RFC4648.decode(string.as_bytes()) {
                    return (self.status, data).into_response();
                }
            }
        }

        (self.status, Json(self.response)).into_response()
    }
}

impl IntoResponse for ModelError {
    fn into_response(self) -> axum::response::Response {
        ModelResponse::from(self).into_response()
    }
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
