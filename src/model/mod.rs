use std::{collections::HashMap, fmt::Debug};

use fast32::base32::{CROCKFORD, RFC4648};
use http::{status::StatusCode, Uri};
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    Client, Method, Url,
};
use ring::digest;
use serde::{Deserialize, Serialize};
use serde_json::{value::Value, Map};
use uuid::Uuid;

mod client;
mod interface;

#[tracing::instrument(level = "trace", ret)]
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

#[derive(Debug)]
pub(super) struct ModelRequest {
    pub(super) user: Option<Uuid>,
    pub(super) r#type: RequestType,

    request: ModelRequestData,
}

#[derive(Debug)]
enum ModelRequestData {
    Json(Map<String, Value>),
    Form(HashMap<String, ModelFormItem>),
}

impl ModelRequestData {
    #[tracing::instrument(level = "trace", ret)]
    fn into_openai(self, model: String, user: Option<Uuid>) -> Self {
        let user = user.map(|user| {
            CROCKFORD.encode(digest::digest(&digest::SHA256, user.as_bytes()).as_ref())
        });

        match self {
            Self::Json(mut json) => {
                json.remove("stream");
                json.insert("model".to_string(), Value::String(model));
                match user {
                    Some(user) => {
                        json.insert("user".to_string(), Value::String(user));
                    }
                    None => {
                        json.remove("user");
                    }
                }

                Self::Json(json)
            }
            Self::Form(mut form) => {
                form.insert("model".to_string(), ModelFormItem::Text(model));
                match user {
                    Some(user) => {
                        form.insert("user".to_string(), ModelFormItem::Text(user));
                    }
                    None => {
                        form.remove("user");
                    }
                }

                Self::Form(form)
            }
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    fn into_loopback(self) -> ModelResponse {
        let json = match self {
            Self::Json(json) => json,
            Self::Form(form) => {
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
        };

        ModelResponse {
            status: StatusCode::OK,
            usage: None,
            response: ModelResponseData::Json(json),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    fn get_model(&self) -> Option<&str> {
        match self {
            Self::Json(json) => json.get("model").and_then(|value| value.as_str()),
            Self::Form(form) => {
                if let Some(ModelFormItem::Text(model)) = form.get("model") {
                    Some(model)
                } else {
                    None
                }
            }
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    fn get_count(&self) -> usize {
        match &self {
            Self::Json(json) => {
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
            Self::Form(form) => form
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

    #[tracing::instrument(level = "trace", ret)]
    fn get_max_tokens(&self) -> Option<u64> {
        match self {
            Self::Json(json) => json
                .get("max_tokens")
                .and_then(|value| value.as_u64().map(|int| int.max(1))),
            Self::Form(_) => None,
        }
    }
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

impl ModelRequest {
    pub(super) fn get_model(&self) -> Option<&str> {
        self.request.get_model()
    }

    pub(super) fn get_count(&self) -> usize {
        self.request.get_count()
    }

    pub(super) fn get_max_tokens(&self) -> Option<u64> {
        self.request.get_max_tokens()
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

impl ModelResponseData {
    #[tracing::instrument(level = "trace", ret)]
    fn into_openai(self, model: Option<String>, tag: Uuid) -> Self {
        match self {
            Self::Json(mut json) => {
                if let Some(value) = json.get_mut("model") {
                    *value = model.map(Value::String).unwrap_or(Value::Null);
                }

                if let Some(value) = json.get_mut("id") {
                    *value = Value::String(format!("{}", tag));
                }

                Self::Json(json)
            }
            Self::Binary(binary) => Self::Binary(binary),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    fn get_usage(&self, is_error: bool) -> Option<TokenUsage> {
        match self {
            Self::Json(json) => json.get("usage").and_then(|usage| {
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
            }),
            Self::Binary(_binary) => None,
        }
        .or_else(|| match is_error {
            true => Some(TokenUsage::default()),
            false => None,
        })
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
            usage: Some(TokenUsage::default()),
            status,
            response: ModelResponseData::Json(json),
        }
    }
}

#[derive(Debug, Default)]
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

impl OpenAIModelBackend {
    #[tracing::instrument(level = "trace")]
    fn get_request_parameters(
        &self,
        r#type: RequestType,
    ) -> Option<(Method, Url, HeaderMap, bool)> {
        match Url::parse(&self.openai_api_base).and_then(|base_url| {
            base_url.join(match r#type {
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
        }) {
            Ok(url) => match HeaderValue::from_str(&format!("Bearer {}", self.openai_api_key)) {
                Ok(auth_header) => {
                    let mut headers = HeaderMap::new();
                    headers.insert(AUTHORIZATION, auth_header);

                    if let Some(organization) = self
                        .openai_organization
                        .as_ref()
                        .and_then(|value| value.parse::<HeaderValue>().ok())
                    {
                        headers.insert("OpenAI-Organization", organization);
                    }

                    Some((Method::POST, url, headers, r#type == RequestType::AudioTTS))
                }
                Err(error) => {
                    tracing::warn!("Unable to parse API key: {:?}", error);
                    None
                }
            },
            Err(error) => {
                tracing::warn!("Unable to parse model URL: {:?}", error);
                None
            }
        }
    }
}

impl ModelBackend {
    pub(super) fn get_max_tokens(&self) -> Option<u64> {
        match &self {
            Self::OpenAI(backend) => backend.model_context_len,
            Self::Loopback => None,
        }
    }

    #[tracing::instrument(skip(self, http_client), level = "debug", ret)]
    pub(super) async fn generate(
        &self,
        http_client: &Client,
        mut request: ModelRequest,
    ) -> ModelResponse {
        let tag = Uuid::new_v4();
        tracing::debug!(tag = ?tag);

        match &self {
            Self::OpenAI(config) => match config.get_request_parameters(request.r#type) {
                Some((method, url, headers, binary)) => {
                    let label = request.get_model().map(|value| value.to_string());

                    request.request = request
                        .request
                        .into_openai(config.model_string.clone(), request.user);

                    let mut response = client::send_http_request(
                        http_client,
                        method,
                        url,
                        headers,
                        request,
                        binary,
                    )
                    .await;

                    response.response = response.response.into_openai(label, tag);

                    response
                }
                None => ModelResponse::from(ModelError::InternalError),
            },
            Self::Loopback => request.request.into_loopback(),
        }
    }
}
