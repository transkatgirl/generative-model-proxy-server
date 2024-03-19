use std::{collections::HashMap, fmt::Debug, time::Duration};

use fast32::base32::{CROCKFORD, RFC4648};
use http::{status::StatusCode, Uri};
use reqwest::{
    multipart::{Form, Part},
    Client, ClientBuilder, RequestBuilder, Response, Url,
};
use ring::digest;
use serde::{Deserialize, Serialize};
use serde_json::{value::Value, Map};
use tracing::instrument;
use uuid::Uuid;

mod interface;

// TODO: Perform rate-limiting based on headers

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
fn get_usage(response: &Map<String, Value>) -> Option<TokenUsage> {
    if let Some(Value::Object(usage)) = response.get("usage") {
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

    request: ModelRequestData,
}

#[derive(Debug)]
enum ModelRequestData {
    Json(Map<String, Value>),
    Form(HashMap<String, ModelFormItem>),
}

#[derive(Debug)]
enum ModelFormItem {
    Text(String),
    File(ModelFormFile),
}

#[derive(Debug)]
struct ModelFormFile {
    file_name: Option<String>,
    content_type: Option<String>,
    data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    fn from_json(r#type: RequestType, mut request: Map<String, Value>) -> TaggedModelRequest {
        request.remove("stream");

        TaggedModelRequest {
            tags: Vec::new(),
            r#type,
            request: ModelRequestData::Json(request),
        }
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_json(self) -> Map<String, Value> {
        match self.request {
            ModelRequestData::Json(json) => json,
            ModelRequestData::Form(form) => {
                let mut json = Map::new();

                for (key, value) in form {
                    match value {
                        ModelFormItem::Text(text) => {
                            json.insert(key, Value::String(text));
                        }
                        ModelFormItem::File(file) => {
                            let mut file_json = Map::new();

                            file_json.insert(
                                "filename".to_string(),
                                file.file_name.map(Value::String).unwrap_or(Value::Null),
                            );
                            file_json.insert(
                                "content-type".to_string(),
                                file.content_type.map(Value::String).unwrap_or(Value::Null),
                            );

                            file_json.insert(
                                "data".to_string(),
                                Value::String(RFC4648.encode(&file.data)),
                            );

                            json.insert(key, Value::Object(file_json));
                        }
                    }
                }

                json
            }
        }
    }

    #[instrument(level = "trace")]
    pub(super) fn set_user(&mut self, user: Uuid) {
        let user = CROCKFORD.encode(digest::digest(&digest::SHA256, user.as_bytes()).as_ref());

        match &mut self.request {
            ModelRequestData::Json(json) => {
                json.insert("user".to_string(), Value::String(user));
            }
            ModelRequestData::Form(form) => {
                form.insert("user".to_string(), ModelFormItem::Text(user));
            }
        }
    }

    #[instrument(level = "trace", ret)]
    pub(super) fn get_model(&self) -> Option<&str> {
        match &self.request {
            ModelRequestData::Json(json) => json.get("model").and_then(|value| value.as_str()),
            ModelRequestData::Form(form) => {
                if let Some(ModelFormItem::Text(model)) = form.get("model") {
                    Some(model)
                } else {
                    None
                }
            }
        }
    }

    #[instrument(level = "trace")]
    fn update_model(&mut self, model: String) {
        match &mut self.request {
            ModelRequestData::Json(json) => {
                json.insert("model".to_string(), Value::String(model));
            }
            ModelRequestData::Form(form) => {
                form.insert("model".to_string(), ModelFormItem::Text(model));
            }
        }
    }

    #[instrument(level = "trace", ret)]
    pub(super) fn get_count(&self) -> usize {
        match &self.request {
            ModelRequestData::Json(json) => {
                json.get("best_of")
                    .and_then(|value| {
                        value
                            .as_u64()
                            .map(|int| int.clamp(1, usize::MAX as u64) as usize)
                    })
                    .unwrap_or(1)
                    * json
                        .get("n")
                        .and_then(|value| {
                            value
                                .as_u64()
                                .map(|int| int.clamp(1, usize::MAX as u64) as usize)
                        })
                        .unwrap_or(1)
                    * json.get("prompt").map(get_prompt_count).unwrap_or(1)
                    * json.get("input").map(get_prompt_count).unwrap_or(1)
            }
            ModelRequestData::Form(form) => form
                .get("n")
                .and_then(|value| {
                    if let ModelFormItem::Text(string) = value {
                        Some(string)
                    } else {
                        None
                    }
                })
                .and_then(|string| string.parse().ok())
                .unwrap_or(1),
        }
    }

    #[instrument(level = "trace", ret)]
    pub(super) fn get_max_tokens(&self) -> Option<u64> {
        match &self.request {
            ModelRequestData::Json(json) => json
                .get("max_tokens")
                .and_then(|value| value.as_u64().map(|int| int.max(1))),
            ModelRequestData::Form(_) => None,
        }
    }

    #[instrument(skip(base), level = "trace")]
    fn to_http_body(&self, base: RequestBuilder) -> RequestBuilder {
        match &self.request {
            ModelRequestData::Json(json) => base.json(&json),
            ModelRequestData::Form(formdata) => {
                let mut form = Form::new();

                for (key, value) in formdata {
                    let key = key.clone();

                    form = match value {
                        ModelFormItem::Text(text) => form.text(key, text.clone()),
                        ModelFormItem::File(file) => {
                            let mut part = Part::bytes(file.data.clone());

                            if let Some(content_type) = &file.content_type {
                                part = match part.mime_str(content_type) {
                                    Ok(updated_part) => updated_part,
                                    Err(_) => Part::bytes(file.data.clone()),
                                };
                            }

                            if let Some(filename) = &file.file_name {
                                part = part.file_name(filename.clone());
                            }

                            form.part(key, part)
                        }
                    };
                }

                base.multipart(form)
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct ModelResponse {
    pub(super) status: StatusCode,
    pub(super) usage: Option<TokenUsage>,
    response: ModelResponseData,
}

#[derive(Debug)]
enum ModelResponseData {
    Json(Map<String, Value>),
    Binary(Vec<u8>),
}

impl ModelResponse {
    async fn from_http_response(
        request: &TaggedModelRequest,
        response: Result<Response, reqwest::Error>,
    ) -> ModelResponse {
        match response {
            Ok(response) => {
                let status = StatusCode::from_u16(response.status().as_u16()).unwrap();
                let body = response.bytes().await;

                if status.is_server_error() {
                    tracing::warn!("Backend returned {} error: {:?}", status, body);
                    return ModelResponse::from(ModelError::BackendError);
                }

                if status.is_client_error() {
                    if status == StatusCode::UNAUTHORIZED
                        || status == StatusCode::FORBIDDEN
                        || status == StatusCode::PROXY_AUTHENTICATION_REQUIRED
                    {
                        tracing::warn!("Failed to authenticate with backend: {:?}", body);
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
                        tracing::warn!("Backend returned {} error: {:?}", status, body);
                        return ModelResponse::from(ModelError::BackendError);
                    }

                    if status == StatusCode::PAYMENT_REQUIRED
                        || status == StatusCode::TOO_MANY_REQUESTS
                    {
                        tracing::warn!("Request was rate-limited by backend: {:?}", body);
                        return ModelResponse::from(ModelError::ModelRateLimit);
                    }
                }

                match body {
                    Ok(body) => match serde_json::from_slice::<Map<String, Value>>(&body) {
                        Ok(mut json) => {
                            if let Some(value) = json.get_mut("model") {
                                *value = request
                                    .get_model()
                                    .map(|label| Value::String(label.to_string()))
                                    .unwrap_or(Value::Null);
                            }

                            if let Some(value) = json.get_mut("id") {
                                *value = Value::String(format!(
                                    "{}",
                                    request.tags.last().unwrap_or(&Uuid::new_v4())
                                ));
                            }

                            ModelResponse {
                                status,
                                usage: get_usage(&json).or_else(|| {
                                    if status.is_client_error() {
                                        Some(TokenUsage {
                                            total: 0,
                                            input: None,
                                            output: None,
                                        })
                                    } else {
                                        None
                                    }
                                }),
                                response: ModelResponseData::Json(json),
                            }
                        }
                        Err(error) => {
                            if request.r#type == RequestType::AudioTTS {
                                ModelResponse {
                                    status,
                                    usage: None,
                                    response: ModelResponseData::Binary(body.to_vec()),
                                }
                            } else {
                                tracing::warn!("Error parsing response: {:?}", error);
                                ModelResponse::from(ModelError::BackendError)
                            }
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

                if error.is_connect() | error.is_redirect() | error.is_decode() {
                    return ModelResponse::from(ModelError::BackendError);
                }

                if error.is_timeout() {
                    return ModelResponse::from(ModelError::ModelRateLimit);
                }

                ModelResponse::from(ModelError::InternalError)
            }
        }
    }
}

impl From<ModelError> for ModelResponse {
    fn from(value: ModelError) -> Self {
        let mut json = Map::new();

        let message = match value {
            ModelError::BadRequest => "We could not parse the JSON body of your request. (HINT: This likely means you aren't using your HTTP library correctly. The API expects a JSON payload, but what was sent was not valid JSON. If you have trouble figuring out how to fix this, contact the proxy's administrator.)",
            ModelError::AuthMissing => "You didn't provide an API key. You need to provide your API key in an Authorization header using Bearer auth (i.e. Authorization: Bearer YOUR_KEY), or as the password field (with blank username) if you're accessing the API from your browser and are prompted for a username and password. You can obtain an API key from the proxy's administrator.",
            ModelError::AuthInvalid => "Incorrect API key provided. You can obtain an API key from the proxy's administrator.",
            ModelError::UserRateLimit => "You exceeded your current quota, please check your API key's rate limits. For more information on this error, contact the proxy's administrator.",
            ModelError::ModelRateLimit => "That model is currently overloaded with other requests. You can retry your request, or contact the proxy's administrator if the error persists.",
            ModelError::UnknownEndpoint => "Unknown request URL. Please check the URL for typos, or contact the proxy's administrator for information regarding available endpoints.",
            ModelError::BadEndpointMethod => "Invalid request method. Please check the URL for typos, or contact the proxy's administrator for information regarding available endpoints.",
            ModelError::UnknownModel => "The requested model does not exist. Contact the proxy's administrator for more information.",
            ModelError::InternalError => "The proxy server had an error processing your request. Sorry about that! You can retry your request, or contact the proxy's administrator if the error persists.",
            ModelError::BackendError => "The model had an error processing your request. Sorry about that! Contact the proxy's administrator for more information.",
        };
        let error_type = match value {
            ModelError::BadRequest => "invalid_request_error",
            ModelError::AuthMissing => "invalid_request_error",
            ModelError::AuthInvalid => "invalid_request_error",
            ModelError::UserRateLimit => "insufficient_quota",
            ModelError::ModelRateLimit => "server_error",
            ModelError::UnknownEndpoint => "invalid_request_error",
            ModelError::BadEndpointMethod => "invalid_request_error",
            ModelError::UnknownModel => "invalid_request_error",
            ModelError::InternalError => "server_error",
            ModelError::BackendError => "server_error",
        };
        let error_code = match value {
            ModelError::BadRequest => Value::Null,
            ModelError::AuthMissing => Value::Null,
            ModelError::AuthInvalid => Value::String("invalid_api_key".to_string()),
            ModelError::UserRateLimit => Value::String("insufficient_quota".to_string()),
            ModelError::ModelRateLimit => Value::Null,
            ModelError::UnknownEndpoint => Value::String("unknown_url".to_string()),
            ModelError::BadEndpointMethod => Value::Null,
            ModelError::UnknownModel => Value::String("model_not_found".to_string()),
            ModelError::InternalError => Value::Null,
            ModelError::BackendError => Value::Null,
        };

        json.insert("message".to_string(), Value::String(message.to_string()));
        json.insert("type".to_string(), Value::String(error_type.to_string()));
        json.insert("param".to_string(), Value::Null);
        json.insert("code".to_string(), error_code);

        let status = match value {
            ModelError::BadRequest => StatusCode::BAD_REQUEST,
            ModelError::AuthMissing => StatusCode::UNAUTHORIZED,
            ModelError::AuthInvalid => StatusCode::UNAUTHORIZED,
            ModelError::UserRateLimit => StatusCode::TOO_MANY_REQUESTS,
            ModelError::ModelRateLimit => StatusCode::SERVICE_UNAVAILABLE,
            ModelError::UnknownEndpoint => StatusCode::NOT_FOUND,
            ModelError::BadEndpointMethod => StatusCode::METHOD_NOT_ALLOWED,
            ModelError::UnknownModel => StatusCode::NOT_FOUND,
            ModelError::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
            ModelError::BackendError => StatusCode::BAD_GATEWAY,
        };

        ModelResponse {
            usage: Some(TokenUsage {
                total: 0,
                input: None,
                output: None,
            }),
            status,
            response: ModelResponseData::Json(json),
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
        match &self {
            Self::OpenAI(config) => {
                tagged_request.update_model(config.model_string.clone());

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
                        let mut builder = http_client.post(url).bearer_auth(&config.openai_api_key);

                        if let Some(organization) = &config.openai_organization {
                            builder = builder.header("OpenAI-Organization", organization);
                        }

                        builder = tagged_request.to_http_body(builder);

                        ModelResponse::from_http_response(&tagged_request, builder.send().await)
                            .await
                    }
                    Err(error) => {
                        tracing::warn!("Unable to parse model URL: {:?}", error);
                        ModelResponse::from(ModelError::InternalError)
                    }
                }
            }
            Self::Loopback => ModelResponse {
                status: StatusCode::OK,
                usage: None,
                response: ModelResponseData::Json(tagged_request.to_json()),
            },
        }
    }
}
