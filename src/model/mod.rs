use std::{fmt::Debug, time::Duration};

use fast32::base32::CROCKFORD;
use http::{status::StatusCode, Uri};
use reqwest::{Client, ClientBuilder, Url};
use ring::digest;
use serde::{Deserialize, Serialize};
use serde_json::{json, value::Value};
use tracing::instrument;
use uuid::Uuid;

// TODO: Perform rate-limiting based on headers, support Audio models

#[instrument(level = "trace", ret)]
fn get_prompt_count(prompt: &Value) -> usize {
    match prompt {
        Value::Array(array) => {
            if !array.is_empty() {
                match array[0] {
                    Value::Array(_) | Value::Object(_) | Value::String(_) => array.len(),
                    _ => 1,
                }
            } else {
                0
            }
        }
        Value::Object(_) => 1,
        Value::String(_) | Value::Number(_) | Value::Bool(_) => 1,
        Value::Null => 0,
    }
}

#[instrument(level = "trace", ret)]
fn get_usage(response: &Value) -> Option<TokenUsage> {
    if let Value::Object(data) = response {
        if let Some(Value::Object(usage)) = data.get("usage") {
            let input_tokens = usage.get("prompt_tokens").and_then(|num| num.as_u64());
            let output_tokens = usage.get("completion_tokens").and_then(|num| num.as_u64());

            usage
                .get("total_tokens")
                .and_then(|num| num.as_u64())
                .or(input_tokens
                    .and_then(|input_tokens| {
                        output_tokens.map(|output_tokens| input_tokens + output_tokens)
                    })
                    .or(output_tokens))
                .map(|total| TokenUsage {
                    total,
                    input: input_tokens,
                    output: output_tokens,
                })
        } else {
            None
        }
    } else {
        None
    }
}

#[instrument(level = "trace")]
pub(super) fn get_configured_client() -> reqwest::Result<Client> {
    ClientBuilder::new()
        .user_agent("language-model-proxy-server")
        .connect_timeout(Duration::from_secs(5))
        .http2_keep_alive_interval(Some(Duration::from_secs(5)))
        .http2_keep_alive_timeout(Duration::from_secs(15))
        .http2_keep_alive_while_idle(true)
        .build()
}

#[derive(Debug)]
pub(super) struct TaggedModelRequest {
    pub(super) tags: Vec<Uuid>,
    pub(super) r#type: RequestType,

    request: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RequestType {
    TextChat,
    TextCompletion,
    TextEdit,
    TextEmbedding,
    TextModeration,
    ImageGeneration,
    ImageEdit,
    ImageVariation,
    AudioTTS,
    AudioTranscription,
    AudioTranslation,
}

impl TryFrom<&Uri> for RequestType {
    type Error = &'static str;

    fn try_from(value: &Uri) -> Result<Self, Self::Error> {
        match value.path() {
            "/v1/chat/completions" => Ok(RequestType::TextChat),
            "/v1/completions" => Ok(RequestType::TextCompletion),
            "/v1/edits" => Ok(RequestType::TextEdit),
            "/v1/embeddings" => Ok(RequestType::TextEmbedding),
            "/v1/moderations" => Ok(RequestType::TextModeration),
            "/v1/images/generations" => Ok(RequestType::ImageGeneration),
            "/v1/images/edits" => Ok(RequestType::ImageEdit),
            "/v1/images/variations" => Ok(RequestType::ImageVariation),
            "/v1/audio/speech" => Ok(RequestType::AudioTTS),
            "/v1/audio/transcriptions" => Ok(RequestType::AudioTranscription),
            "/v1/audio/translations" => Ok(RequestType::AudioTranslation),
            _ => Err("Invalid URI"),
        }
    }
}

impl TaggedModelRequest {
    #[instrument(level = "trace", ret)]
    pub(super) fn new(
        tags: Vec<Uuid>,
        r#type: RequestType,
        mut request: Value,
    ) -> TaggedModelRequest {
        if let Some(request) = request.as_object_mut() {
            match tags.first() {
                Some(user) => request.insert(
                    "user".to_string(),
                    Value::String(
                        CROCKFORD.encode(digest::digest(&digest::SHA256, user.as_bytes()).as_ref()),
                    ),
                ),
                None => request.remove("user"),
            };

            request.remove("stream");
        }

        TaggedModelRequest {
            tags,
            r#type,
            request,
        }
    }

    #[instrument(level = "trace", ret)]
    pub(super) fn get_model(&self) -> Option<&str> {
        self.request
            .as_object()
            .and_then(|request| request.get("model").and_then(|value| value.as_str()))
    }

    #[instrument(level = "trace", ret)]
    pub(super) fn get_count(&self) -> usize {
        match &self.request {
            Value::Object(request) => {
                request
                    .get("best_of")
                    .and_then(|value| {
                        value
                            .as_u64()
                            .map(|int| int.clamp(1, usize::MAX as u64) as usize)
                    })
                    .unwrap_or(1)
                    * request
                        .get("n")
                        .and_then(|value| {
                            value
                                .as_u64()
                                .map(|int| int.clamp(1, usize::MAX as u64) as usize)
                        })
                        .unwrap_or(1)
                    * request.get("prompt").map(get_prompt_count).unwrap_or(1)
                    * request.get("input").map(get_prompt_count).unwrap_or(1)
            }
            Value::Array(array) => array.len(),
            Value::Null => 0,
            _ => 1,
        }
    }

    #[instrument(level = "trace", ret)]
    pub(super) fn get_max_tokens(&self) -> Option<u64> {
        if let Value::Object(request) = &self.request {
            request
                .get("max_tokens")
                .and_then(|value| value.as_u64().map(|int| int.max(1)))
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub(super) struct ModelResponse {
    pub(super) status: StatusCode,
    pub(super) usage: Option<TokenUsage>,
    pub(super) response: Value,
}

impl From<ModelError> for ModelResponse {
    fn from(value: ModelError) -> Self {
        let response = match value {
            ModelError::BadRequest => json!({
                "message": "We could not parse the JSON body of your request. (HINT: This likely means you aren't using your HTTP library correctly. The API expects a JSON payload, but what was sent was not valid JSON. If you have trouble figuring out how to fix this, contact the proxy's administrator.)",
                "type": "invalid_request_error",
                "param": Value::Null,
                "code": Value::Null,
            }),
            ModelError::AuthMissing => json!({
                "message": "You didn't provide an API key. You need to provide your API key in an Authorization header using Bearer auth (i.e. Authorization: Bearer YOUR_KEY), or as the password field (with blank username) if you're accessing the API from your browser and are prompted for a username and password. You can obtain an API key from the proxy's administrator.",
                "type": "invalid_request_error",
                "param": Value::Null,
                "code": Value::Null,
            }),
            ModelError::AuthInvalid => json!({
                "message": "Incorrect API key provided. You can obtain an API key from the proxy's administrator.",
                "type": "invalid_request_error",
                "param": Value::Null,
                "code": "invalid_api_key",
            }),
            ModelError::UserRateLimit => json!({
                "message": "You exceeded your current quota, please check your API key's rate limits. For more information on this error, contact the proxy's administrator.",
                "type": "insufficient_quota",
                "param": Value::Null,
                "code": "insufficient_quota",
            }),
            ModelError::ModelRateLimit => json!({
                "message": "That model is currently overloaded with other requests. You can retry your request, or contact the proxy's administrator if the error persists.",
                "type": "server_error",
                "param": Value::Null,
                "code": Value::Null,
            }),
            ModelError::UnknownEndpoint => json!({
                "message": "Unknown request URL. Please check the URL for typos, or contact the proxy's administrator for information regarding available endpoints.",
                "type": "invalid_request_error",
                "param": Value::Null,
                "code": "unknown_url",
            }),
            ModelError::BadEndpointMethod => json!({
                "message": "Invalid request method. Please check the URL for typos, or contact the proxy's administrator for information regarding available endpoints.",
                "type": "invalid_request_error",
                "param": Value::Null,
                "code": Value::Null,
            }),
            ModelError::UnknownModel => json!({
                "message": "The requested model does not exist.  Contact the proxy's administrator for more information.",
                "type": "invalid_request_error",
                "param": Value::Null,
                "code": "model_not_found",
            }),
            ModelError::UnspecifiedModel => json!({
                "message": "You must provide a model parameter. Contact the proxy's administrator for more information.",
                "type": "invalid_request_error",
                "param": Value::Null,
                "code": Value::Null,
            }),
            ModelError::InternalError => json!({
                "message": "The proxy server had an error processing your request. Sorry about that! You can retry your request, or contact the proxy's administrator if the error persists.",
                "type": "server_error",
                "param": Value::Null,
                "code": Value::Null,
            }),
            ModelError::BackendError => json!({
                "message": "The model had an error processing your request. Sorry about that! Contact the proxy's administrator for more information.",
                "type": "server_error",
                "param": Value::Null,
                "code": Value::Null,
            }),
        };

        let status = match value {
            ModelError::BadRequest => StatusCode::BAD_REQUEST,
            ModelError::AuthMissing => StatusCode::UNAUTHORIZED,
            ModelError::AuthInvalid => StatusCode::UNAUTHORIZED,
            ModelError::UserRateLimit => StatusCode::TOO_MANY_REQUESTS,
            ModelError::ModelRateLimit => StatusCode::SERVICE_UNAVAILABLE,
            ModelError::UnknownEndpoint => StatusCode::NOT_FOUND,
            ModelError::BadEndpointMethod => StatusCode::METHOD_NOT_ALLOWED,
            ModelError::UnknownModel => StatusCode::NOT_FOUND,
            ModelError::UnspecifiedModel => StatusCode::BAD_REQUEST,
            ModelError::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
            ModelError::BackendError => StatusCode::BAD_GATEWAY,
        };

        ModelResponse {
            usage: None,
            status,
            response,
        }
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub(super) struct TokenUsage {
    pub(super) total: u64,
    pub(super) input: Option<u64>,
    pub(super) output: Option<u64>,
}

#[derive(Debug)]
pub(super) enum ModelError {
    BadRequest,
    AuthMissing,
    AuthInvalid,
    UserRateLimit,
    ModelRateLimit,
    UnknownEndpoint,
    BadEndpointMethod,
    UnknownModel,
    UnspecifiedModel,
    InternalError,
    BackendError,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(private_interfaces)]
pub(super) enum ModelBackend {
    OpenAI(OpenAIModelBackend),
    Loopback,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OpenAIModelBackend {
    model_string: String,
    model_context_len: Option<u64>,
    openai_api_base: String,
    openai_api_key: String,
    openai_organization: Option<String>,
}

impl ModelBackend {
    #[instrument(level = "trace", ret)]
    pub(super) fn get_max_tokens(&self) -> Option<u64> {
        match &self {
            Self::OpenAI(backend) => backend.model_context_len,
            Self::Loopback => None,
        }
    }

    #[instrument(level = "debug", ret)]
    pub(super) async fn generate(
        &self,
        http_client: &Client,
        mut tagged_request: TaggedModelRequest,
    ) -> ModelResponse {
        let label = tagged_request
            .get_model()
            .map(|label| Value::String(label.to_string()))
            .unwrap_or(Value::Null);
        let request_id = Value::String(format!(
            "{}",
            tagged_request.tags.last().unwrap_or(&Uuid::new_v4())
        ));

        match &self {
            Self::OpenAI(config) => {
                if let Some(request) = tagged_request.request.as_object_mut() {
                    request.insert(
                        "model".to_string(),
                        Value::String(config.model_string.clone()),
                    );

                    let url = Url::parse(&config.openai_api_base).and_then(|base_url| {
                        base_url.join(match tagged_request.r#type {
                            RequestType::TextChat => "/v1/chat/completions",
                            RequestType::TextCompletion => "/v1/completions",
                            RequestType::TextEdit => "/v1/edits",
                            RequestType::TextEmbedding => "/v1/embeddings",
                            RequestType::TextModeration => "/v1/moderations",
                            RequestType::ImageGeneration => "/v1/images/generations",
                            RequestType::ImageEdit => "/v1/images/edits",
                            RequestType::ImageVariation => "/v1/images/variations",
                            RequestType::AudioTTS => "/v1/audio/speech",
                            RequestType::AudioTranscription => "/v1/audio/transcriptions",
                            RequestType::AudioTranslation => "/v1/audio/translations",
                        })
                    });

                    match url {
                        Ok(url) => {
                            let mut builder =
                                http_client.post(url).bearer_auth(&config.openai_api_key);

                            if let Some(organization) = &config.openai_organization {
                                builder = builder.header("OpenAI-Organization", organization);
                            }

                            builder = builder.json(request);

                            match builder.send().await {
                                Ok(response) => {
                                    let status =
                                        StatusCode::from_u16(response.status().as_u16()).unwrap();
                                    let body = response.text().await;

                                    if status.is_server_error() {
                                        tracing::warn!(
                                            "Backend returned {} error: {:?}",
                                            status,
                                            body
                                        );
                                        return ModelResponse::from(ModelError::BackendError);
                                    }

                                    if status.is_client_error() {
                                        if status == StatusCode::UNAUTHORIZED
                                            || status == StatusCode::FORBIDDEN
                                            || status == StatusCode::PROXY_AUTHENTICATION_REQUIRED
                                        {
                                            tracing::warn!(
                                                "Failed to authenticate with backend: {:?}",
                                                body
                                            );
                                            return ModelResponse::from(ModelError::BackendError);
                                        }

                                        if status == StatusCode::NOT_FOUND
                                            || status == StatusCode::METHOD_NOT_ALLOWED
                                            || status == StatusCode::NOT_ACCEPTABLE
                                            || status == StatusCode::REQUEST_TIMEOUT
                                            || status == StatusCode::GONE
                                            || status == StatusCode::LENGTH_REQUIRED
                                            || status == StatusCode::URI_TOO_LONG
                                            || status == StatusCode::EXPECTATION_FAILED
                                            || status == StatusCode::MISDIRECTED_REQUEST
                                            || status == StatusCode::UPGRADE_REQUIRED
                                            || status == StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
                                        {
                                            tracing::warn!(
                                                "Backend returned {} error: {:?}",
                                                status,
                                                body
                                            );
                                            return ModelResponse::from(ModelError::BackendError);
                                        }

                                        if status == StatusCode::PAYMENT_REQUIRED
                                            || status == StatusCode::TOO_MANY_REQUESTS
                                        {
                                            tracing::warn!(
                                                "Request was rate-limited by backend: {:?}",
                                                body
                                            );
                                            return ModelResponse::from(ModelError::ModelRateLimit);
                                        }
                                    }

                                    match body {
                                        Ok(body) => match serde_json::from_str::<Value>(&body) {
                                            Ok(mut json) => {
                                                if let Some(json) = json.as_object_mut() {
                                                    if let Some(value) = json.get_mut("model") {
                                                        *value = label;
                                                    }

                                                    if let Some(value) = json.get_mut("id") {
                                                        *value = request_id;
                                                    }
                                                }

                                                ModelResponse {
                                                    status,
                                                    usage: get_usage(&json),
                                                    response: json,
                                                }
                                            }
                                            Err(error) => {
                                                tracing::warn!(
                                                    "Error parsing response: {:?}",
                                                    error
                                                );
                                                ModelResponse::from(ModelError::BackendError)
                                            }
                                        },
                                        Err(error) => {
                                            tracing::warn!("Error receiving response: {:?}", error);

                                            ModelResponse::from(ModelError::BackendError)
                                        }
                                    }
                                }
                                Err(error) => {
                                    tracing::warn!("Error sending request: {:?}", error);

                                    if error.is_connect() | error.is_redirect() | error.is_decode()
                                    {
                                        return ModelResponse::from(ModelError::BackendError);
                                    }

                                    if error.is_timeout() {
                                        return ModelResponse::from(ModelError::ModelRateLimit);
                                    }

                                    ModelResponse::from(ModelError::InternalError)
                                }
                            }
                        }
                        Err(error) => {
                            tracing::warn!("Unable to parse model URL: {:?}", error);
                            ModelResponse::from(ModelError::InternalError)
                        }
                    }
                } else {
                    ModelResponse::from(ModelError::BadRequest)
                }
            }
            Self::Loopback => ModelResponse {
                status: StatusCode::OK,
                usage: None,
                response: tagged_request.request,
            },
        }
    }
}
