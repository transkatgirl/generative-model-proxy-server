use std::{
    clone::Clone,
    collections::HashSet,
    fmt::Debug,
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Extension, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    Router,
};

use fast32::base64::RFC4648;
use http::{
    header::{AUTHORIZATION, USER_AGENT, WWW_AUTHENTICATE},
    Version,
};
use http::{
    header::{CONTENT_LENGTH, CONTENT_TYPE},
    uri::Scheme,
};
use serde::{Deserialize, Serialize};
use tokio::time;
use tower::ServiceBuilder;
use tower_http::{classify::ServerErrorsFailureClass, trace::TraceLayer};
use tracing::{field::Empty, Instrument, Span};
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

pub fn api_router(state: AppState) -> Router {
    Router::new()
        .fallback(handle_model_request)
        .nest("/admin", admin::admin_router())
        .with_state(state.clone())
        .layer(
            ServiceBuilder::new()
                .layer(DefaultBodyLimit::max(16_777_216))
                .layer(
                    TraceLayer::new_for_http()
                        .make_span_with(|request: &Request<Body>| {
                            tracing::debug_span!(
                                "request",
                                otel.name =
                                    format!("{} {}", request.method(), request.uri().path()),
                                otel.kind = "Server",
                                url.scheme =
                                    request.uri().scheme().unwrap_or(&Scheme::HTTP).as_str(),
                                http.request.method = request.method().as_str(),
                                "http.request.header.content-type" = request
                                    .headers()
                                    .get(CONTENT_TYPE)
                                    .and_then(|value| value.to_str().ok()),
                                server.address = request.uri().host(),
                                server.port = request.uri().port().map(|port| port.to_string()),
                                url.path = request.uri().path(),
                                url.query = request.uri().query(),
                                http.response.status_code = Empty,
                                "http.response.header.content-type" = Empty,
                                network.protocol.name = "http",
                                network.protocol.version = match request.version() {
                                    Version::HTTP_09 => Some("0.9"),
                                    Version::HTTP_10 => Some("1.0"),
                                    Version::HTTP_11 => Some("1.1"),
                                    Version::HTTP_2 => Some("2"),
                                    Version::HTTP_3 => Some("3"),
                                    _ => None,
                                },
                                user_agent.original = request
                                    .headers()
                                    .get(USER_AGENT)
                                    .and_then(|value| value.to_str().ok()),
                                "error.type" = Empty,
                            )
                        })
                        .on_request(|request: &Request<Body>, _span: &Span| {
                            if let Some(length) = request
                                .headers()
                                .get(CONTENT_LENGTH)
                                .and_then(|value| value.to_str().ok())
                                .map(|value| value.parse::<u64>().ok())
                            {
                                tracing::debug!(
                                    http.server.request.body.size = length,
                                    unit = "By"
                                );
                            }

                            if cfg!(debug_assertions) {
                                tracing::trace!(target: "on_request", request = ?request);
                            }
                        })
                        .on_response(
                            |response: &Response<Body>, latency: Duration, span: &Span| {
                                span.record(
                                    "http.response.status_code",
                                    response.status().as_u16(),
                                );

                                if let Some(length) = response
                                    .headers()
                                    .get(CONTENT_LENGTH)
                                    .and_then(|value| value.to_str().ok())
                                    .map(|value| value.parse::<u64>().ok())
                                {
                                    tracing::debug!(
                                        http.server.response.body.size = length,
                                        unit = "By"
                                    );
                                }

                                if let Some(content_type) = response
                                    .headers()
                                    .get(CONTENT_TYPE)
                                    .and_then(|value| value.to_str().ok())
                                {
                                    span.record("http.response.header.content-type", content_type);
                                }

                                tracing::debug!(
                                    histogram.http.server.request.duration = latency.as_secs_f64(),
                                    unit = "s"
                                );

                                if cfg!(debug_assertions) {
                                    tracing::trace!(target: "on_response", response = ?response);
                                }
                            },
                        )
                        .on_failure(
                            |error: ServerErrorsFailureClass, _latency: Duration, span: &Span| {
                                span.record("error.type", format!("{}", error));

                                tracing::error!(target: "on_error", ?error);
                            },
                        ),
                )
                .layer(middleware::map_response(modify_response))
                .layer(middleware::from_fn_with_state(state, authenticate)),
        )
}

async fn authenticate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, ModelError> {
    let span = tracing::debug_span!("authenticate").entered();

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
            if cfg!(debug_assertions) {
                tracing::trace!(api_key = api_key);
            }

            if state.is_table_empty("users") && api_key == "setup-key" {
                request.extensions_mut().insert(Authenticated {
                    timestamp,
                    admin: true,
                    user: User::default(),
                    roles: Vec::new(),
                });

                tracing::warn!(user = "first-time-setup");

                span.exit();

                return Ok(next.run(request).await);
            }

            match state.get_related_item::<_, Uuid, User>(("api_keys", "users"), &api_key) {
                DatabaseValueResult::Success(user) => {
                    if cfg!(debug_assertions) {
                        tracing::debug!(user = ?user);
                    } else {
                        tracing::debug!(user = ?user.uuid);
                    }

                    let roles: Vec<Uuid> = user.roles.iter().copied().collect();

                    match state.get_items_skip_missing::<_, Role>("roles", &roles) {
                        DatabaseValueResult::Success(roles) => {
                            let mut admin = user.admin;

                            for role in &roles {
                                if role.admin {
                                    admin = true;
                                }
                            }

                            if cfg!(debug_assertions) {
                                tracing::debug!(roles = ?roles)
                            } else {
                                tracing::debug!(roles = ?roles.iter().map(|role| role.uuid).collect::<Vec<Uuid>>());
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

                    span.exit();

                    Ok(next.run(request).await)
                }
                DatabaseValueResult::NotFound => Err(ModelError::AuthInvalid),
                DatabaseValueResult::BackendError => Err(ModelError::InternalError),
            }
        }
        None => Err(ModelError::AuthMissing),
    }
}

#[tracing::instrument(name = "handle_admin_request", level = "debug", skip_all)]
async fn authenticate_admin(
    Extension(auth): Extension<Authenticated>,
    request: Request,
    next: Next,
) -> Result<Response, ModelError> {
    tracing::debug!(admin = auth.admin);

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

#[tracing::instrument(level = "debug", skip_all)]
async fn handle_model_request(
    Extension(auth): Extension<Authenticated>,
    State(state): State<AppState>,
    mut request: ModelRequest,
) -> Result<ModelResponse, ModelError> {
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
        DatabaseValueResult::Success(models) => {
            if cfg!(debug_assertions) {
                tracing::trace!(models = ?models);
            }

            match models
                .iter()
                .find(|model| model.types.contains(&request.r#type) && model.name == model_name)
            {
                Some(model) => model.clone(),
                None => return Err(ModelError::UnknownModel),
            }
        }
        DatabaseValueResult::NotFound => return Err(ModelError::UnknownModel),
        DatabaseValueResult::BackendError => return Err(ModelError::InternalError),
    };

    if cfg!(debug_assertions) {
        tracing::debug!(model = ?model);
    } else {
        tracing::debug!(model = ?model.uuid);
    }

    let model_max_tokens = model.api.get_max_tokens().unwrap_or(1);
    let estimated_tokens = request.get_max_tokens().unwrap_or(model_max_tokens);
    if estimated_tokens > model_max_tokens {
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

    tracing::debug!(quotas = ?quotas);

    request.user = Some(auth.user.uuid);

    let limiter_request = limiter::Request {
        arrived_at: auth.timestamp,
        estimated_tokens,
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
                time::sleep_until(time::Instant::from_std(wait_until))
                    .instrument(tracing::debug_span!("rate_limit_request"))
                    .await
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
                    time::sleep_until(time::Instant::from_std(wait_until))
                        .instrument(tracing::debug_span!("rate_limit_response"))
                        .await
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
