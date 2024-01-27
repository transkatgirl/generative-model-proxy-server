use std::{any::Any, ops::Deref};

pub use async_openai::{
    config::OpenAIConfig,
    error::{ApiError, OpenAIError},
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestMessageContentPart,
        ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
        CreateChatCompletionResponse, CreateCompletionRequest, CreateCompletionResponse,
        CreateEditRequest, CreateEditResponse, CreateEmbeddingRequest, CreateEmbeddingResponse,
        CreateImageEditRequest, CreateImageRequest, CreateImageVariationRequest,
        CreateModerationRequest, CreateModerationResponse, CreateTranscriptionRequest,
        CreateTranscriptionResponse, CreateTranslationRequest, CreateTranslationResponse,
        EmbeddingInput, ImageModel, ImagesResponse, ModerationInput, Prompt, TextModerationModel,
    },
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::value::Value;
use tracing::{event, Level};

use super::{
    tokenizer::{self, Tokenizer},
    CallableModelAPI, ModelErrorCode, ModelRequest, ModelResponse, RoutableModelError,
    RoutableModelRequest, RoutableModelResponse,
};
use crate::api;

#[tracing::instrument(level = "trace")]
fn get_token_count_messages(model: &api::Model, messages: &[ChatCompletionRequestMessage]) -> u32 {
    let tokens_per_message = match model.metadata.tokens_per_message {
        Some(t) => t,
        None => {
            if model.label.starts_with("gpt-3.5") {
                4
            } else {
                3
            }
        }
    };
    let tokens_per_name = match model.metadata.tokens_per_name {
        Some(t) => t,
        None => {
            if model.label.starts_with("gpt-3.5") {
                -1
            } else {
                1
            }
        }
    };

    let mut token_offset: i32 = 3;
    let mut text = Vec::new();

    for message in messages {
        token_offset += tokens_per_message;
        match message {
            ChatCompletionRequestMessage::System(m) => {
                text.push("system");
                text.push(&m.content);

                if let Some(name) = &m.name {
                    text.push(name);
                    token_offset += tokens_per_name;
                }
            }
            ChatCompletionRequestMessage::User(m) => {
                text.push("user");

                match &m.content {
                    ChatCompletionRequestUserMessageContent::Text(content) => {
                        text.push(content);
                    }
                    ChatCompletionRequestUserMessageContent::Array(content_array) => {
                        for content in content_array {
                            if let ChatCompletionRequestMessageContentPart::Text(c) = content {
                                text.push(&c.text)
                            }
                        }
                    }
                }
                if let Some(name) = &m.name {
                    text.push(name);
                    token_offset += tokens_per_name;
                }
            }
            ChatCompletionRequestMessage::Assistant(m) => {
                text.push("assistant");

                if let Some(content) = &m.content {
                    text.push(content);
                }
                if let Some(name) = &m.name {
                    text.push(name);
                    token_offset += tokens_per_name;
                }
                if let Some(tool_calls) = &m.tool_calls {
                    for tool_call in tool_calls {
                        text.push(&tool_call.function.name);
                        text.push(&tool_call.function.arguments);
                    }
                }

                #[allow(deprecated)]
                if let Some(function) = &m.function_call {
                    text.push(&function.name);
                    text.push(&function.arguments);
                }
            }
            ChatCompletionRequestMessage::Tool(m) => {
                text.push("tool");
                text.push(&m.content);
            }
            ChatCompletionRequestMessage::Function(m) => {
                text.push("function");
                text.push(&m.name);
                if let Some(content) = &m.content {
                    text.push(content);
                }
            }
        }
    }

    let tokenizer = model.get_tokenizer().unwrap_or(Tokenizer::Cl100kBase);
    tokenizer::get_token_count(tokenizer, &text) + token_offset as u32
}

impl RoutableModelRequest for CreateChatCompletionRequest {
    #[tracing::instrument(level = "debug")]
    fn get_model(&self) -> String {
        self.model.clone()
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self, model: &api::Model) -> Option<u32> {
        Some(get_token_count_messages(model, &self.messages))
    }

    #[tracing::instrument(level = "debug")]
    fn get_max_tokens(&self, model: &api::Model) -> Option<u32> {
        Some(
            self.max_tokens
                .unwrap_or_else(|| model.get_context_len() as u16) as u32
                * self.n.unwrap_or(1) as u32,
        )
    }
}

impl RoutableModelResponse for CreateChatCompletionResponse {
    #[tracing::instrument(level = "debug")]
    fn replace_model_id(&mut self, model_id: String) {
        self.model = model_id
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self) -> Option<u32> {
        self.usage.clone().map(|u| u.total_tokens)
    }
}

impl RoutableModelRequest for CreateEditRequest {
    #[tracing::instrument(level = "debug")]
    fn get_model(&self) -> String {
        self.model.clone()
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self, model: &api::Model) -> Option<u32> {
        let mut input = Vec::new();
        if let Some(i) = self.input.clone() {
            input.push(i);
        }
        input.push(self.instruction.clone());

        let tokenizer = model.get_tokenizer().unwrap_or(Tokenizer::P50kEdit);
        Some(tokenizer::get_token_count(tokenizer, &input))
    }

    #[tracing::instrument(level = "debug")]
    fn get_max_tokens(&self, model: &api::Model) -> Option<u32> {
        Some(model.get_context_len() as u32 * self.n.unwrap_or(1) as u32)
    }
}

impl RoutableModelResponse for CreateEditResponse {
    #[tracing::instrument(level = "debug")]
    fn replace_model_id(&mut self, _model_id: String) {}

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self) -> Option<u32> {
        Some(self.usage.total_tokens)
    }
}

impl RoutableModelRequest for CreateCompletionRequest {
    #[tracing::instrument(level = "debug")]
    fn get_model(&self) -> String {
        self.model.clone()
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self, model: &api::Model) -> Option<u32> {
        let tokenizer = model.get_tokenizer().unwrap_or(Tokenizer::Cl100kBase);
        Some(match self.prompt.clone() {
            Prompt::String(text) => tokenizer::get_token_count(tokenizer, &[&text]),
            Prompt::StringArray(text_array) => tokenizer::get_token_count(tokenizer, &text_array),
            Prompt::IntegerArray(tokens) => tokens.len() as u32,
            Prompt::ArrayOfIntegerArray(token_array) => token_array.concat().len() as u32,
        })
    }

    #[tracing::instrument(level = "debug")]
    fn get_max_tokens(&self, model: &api::Model) -> Option<u32> {
        let per_iteration = self
            .max_tokens
            .unwrap_or_else(|| model.get_context_len() as u16);

        let multiplier = self.best_of.unwrap_or(1).max(self.n.unwrap_or(1)) as u32;
        let iterations = match self.prompt.clone() {
            Prompt::String(_) => multiplier,
            Prompt::StringArray(p) => p.len() as u32 * multiplier,
            Prompt::IntegerArray(_) => multiplier,
            Prompt::ArrayOfIntegerArray(p) => p.len() as u32 * multiplier,
        };

        Some(per_iteration as u32 * iterations)
    }
}

impl RoutableModelResponse for CreateCompletionResponse {
    #[tracing::instrument(level = "debug")]
    fn replace_model_id(&mut self, model_id: String) {
        self.model = model_id
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self) -> Option<u32> {
        self.usage.clone().map(|u| u.total_tokens)
    }
}

impl RoutableModelRequest for CreateModerationRequest {
    #[tracing::instrument(level = "debug")]
    fn get_model(&self) -> String {
        match self.model {
            Some(TextModerationModel::Stable) => "text-moderation-stable",
            Some(TextModerationModel::Latest) => "text-moderation-latest",
            None => "text-moderation-latest",
        }
        .to_string()
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self, model: &api::Model) -> Option<u32> {
        let tokenizer = model.get_tokenizer().unwrap_or(Tokenizer::Cl100kBase);
        Some(match self.input.clone() {
            ModerationInput::String(text) => tokenizer::get_token_count(tokenizer, &[&text]),
            ModerationInput::StringArray(text_array) => {
                tokenizer::get_token_count(tokenizer, &text_array)
            }
        })
    }

    #[tracing::instrument(level = "debug")]
    fn get_max_tokens(&self, _model: &api::Model) -> Option<u32> {
        None
    }
}

impl RoutableModelResponse for CreateModerationResponse {
    #[tracing::instrument(level = "debug")]
    fn replace_model_id(&mut self, model_id: String) {
        self.model = model_id
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self) -> Option<u32> {
        None
    }
}

impl RoutableModelRequest for CreateEmbeddingRequest {
    #[tracing::instrument(level = "debug")]
    fn get_model(&self) -> String {
        self.model.clone()
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self, model: &api::Model) -> Option<u32> {
        let tokenizer = model.get_tokenizer().unwrap_or(Tokenizer::Cl100kBase);
        Some(match self.input.clone() {
            EmbeddingInput::String(text) => tokenizer::get_token_count(tokenizer, &[&text]),
            EmbeddingInput::StringArray(text_array) => {
                tokenizer::get_token_count(tokenizer, &text_array)
            }
            EmbeddingInput::IntegerArray(tokens) => tokens.len() as u32,
            EmbeddingInput::ArrayOfIntegerArray(token_array) => token_array.concat().len() as u32,
        })
    }

    #[tracing::instrument(level = "debug")]
    fn get_max_tokens(&self, _model: &api::Model) -> Option<u32> {
        None
    }
}

impl RoutableModelResponse for CreateEmbeddingResponse {
    #[tracing::instrument(level = "debug")]
    fn replace_model_id(&mut self, model_id: String) {
        self.model = model_id
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self) -> Option<u32> {
        Some(self.usage.total_tokens)
    }
}

/*impl RoutableModelRequest for CreateImageRequest {
    #[tracing::instrument(level = "debug")]
    fn get_model(&self) -> String {
        match self.model.clone() {
            Some(ImageModel::DallE3) => "dall-e-3".to_string(),
            Some(ImageModel::DallE2) => "dall-e-2".to_string(),
            Some(ImageModel::Other(m)) => m,
            None => "dall-e-2".to_string(),
        }
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self, _model: &api::Model) -> Option<u32> {
        Some(self.n.unwrap_or(1) as u32)
    }

    #[tracing::instrument(level = "debug")]
    fn get_max_tokens(&self, _model: &api::Model) -> Option<u32> {
        None
    }
}

impl RoutableModelRequest for CreateImageEditRequest {
    #[tracing::instrument(level = "debug")]
    fn get_model(&self) -> String {
        match self.model.clone() {
            Some(ImageModel::DallE3) => "dall-e-3".to_string(),
            Some(ImageModel::DallE2) => "dall-e-2".to_string(),
            Some(ImageModel::Other(m)) => m,
            None => "dall-e-2".to_string(),
        }
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self, _model: &api::Model) -> Option<u32> {
        Some(self.n.unwrap_or(1) as u32)
    }

    #[tracing::instrument(level = "debug")]
    fn get_max_tokens(&self, _model: &api::Model) -> Option<u32> {
        None
    }
}

impl RoutableModelRequest for CreateImageVariationRequest {
    #[tracing::instrument(level = "debug")]
    fn get_model(&self) -> String {
        match self.model.clone() {
            Some(ImageModel::DallE3) => "dall-e-3".to_string(),
            Some(ImageModel::DallE2) => "dall-e-2".to_string(),
            Some(ImageModel::Other(m)) => m,
            None => "dall-e-2".to_string(),
        }
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self, _model: &api::Model) -> Option<u32> {
        Some(self.n.unwrap_or(1) as u32)
    }

    #[tracing::instrument(level = "debug")]
    fn get_max_tokens(&self, _model: &api::Model) -> Option<u32> {
        None
    }
}

impl RoutableModelResponse for ImagesResponse {
    #[tracing::instrument(level = "debug")]
    fn replace_model_id(&mut self, _model_id: String) {}

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self) -> Option<u32> {
        Some(self.data.len() as u32)
    }
}

impl RoutableModelRequest for CreateTranscriptionRequest {
    #[tracing::instrument(level = "debug")]
    fn get_model(&self) -> String {
        self.model.clone()
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self, _model: &api::Model) -> Option<u32> {
        None
    }

    #[tracing::instrument(level = "debug")]
    fn get_max_tokens(&self, _model: &api::Model) -> Option<u32> {
        None
    }
}

impl RoutableModelResponse for CreateTranscriptionResponse {
    #[tracing::instrument(level = "debug")]
    fn replace_model_id(&mut self, _model_id: String) {}

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self) -> Option<u32> {
        None
    }
}

impl RoutableModelRequest for CreateTranslationRequest {
    #[tracing::instrument(level = "debug")]
    fn get_model(&self) -> String {
        self.model.clone()
    }

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self, _model: &api::Model) -> Option<u32> {
        None
    }

    #[tracing::instrument(level = "debug")]
    fn get_max_tokens(&self, _model: &api::Model) -> Option<u32> {
        None
    }
}

impl RoutableModelResponse for CreateTranslationResponse {
    #[tracing::instrument(level = "debug")]
    fn replace_model_id(&mut self, _model_id: String) {}

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self) -> Option<u32> {
        None
    }
}*/

impl RoutableModelResponse for ApiError {
    #[tracing::instrument(level = "debug")]
    fn replace_model_id(&mut self, _model_id: String) {}

    #[tracing::instrument(level = "debug")]
    fn get_token_count(&self) -> Option<u32> {
        None
    }
}

impl RoutableModelError for ApiError {
    #[tracing::instrument(level = "debug")]
    fn get_error_code(&self) -> ModelErrorCode {
        match self.r#type.as_deref() {
            Some("invalid_request_error") => match self.code.clone().unwrap_or(Value::Null) {
                Value::String(code) => {
                    match code.deref() {
                        "invalid_api_key" => ModelErrorCode::AuthIncorrect,
                        "model_not_found" => ModelErrorCode::ModelNotFound,
                        "unknown_url" => ModelErrorCode::EndpointNotFound,
                        _ => ModelErrorCode::OtherModelError,
                    }
                }
                _ => match self.message.deref() {
                    "We could not parse the JSON body of your request. (HINT: This likely means you aren't using your HTTP library correctly. The OpenAI API expects a JSON payload, but what was sent was not valid JSON. If you have trouble figuring out how to fix this, contact the proxy's administrator.)" => ModelErrorCode::FailedParse,
                    "Your messages exceeded the model's maximum context length. Please reduce the length of the message. If you belive you are seeing this message in error, contact the proxy's administrator." => ModelErrorCode::PromptTooLong,
                    "You didn't provide an API key. You need to provide your API key in an Authorization header using Bearer auth (i.e. Authorization: Bearer YOUR_KEY), or as the password field (with blank username) if you're accessing the API from your browser and are prompted for a username and password. You can obtain an API key from the proxy's administrator." => ModelErrorCode::AuthMissing,
                    _ => ModelErrorCode::OtherModelError,
                },
            },
            Some("insufficient_quota") => ModelErrorCode::RateLimitUser,
            Some("server_error") => match self.message.deref() {
                "That model is currently overloaded with other requests. You can retry your request, or contact the proxy's administrator if the error persists." => ModelErrorCode::RateLimitModel,
                _ => ModelErrorCode::InternalError,
            },
            _ => ModelErrorCode::OtherModelError,
        }
    }
}

#[tracing::instrument(level = "trace")]
fn init_openai_client(endpoint: OpenAIEndpoint) -> Client<OpenAIConfig> {
    let mut config = OpenAIConfig::new()
        .with_api_base(endpoint.openai_api_base)
        .with_api_key(endpoint.openai_api_key);

    if let Some(org) = endpoint.openai_organization {
        config = config.with_org_id(org);
    }

    async_openai::Client::with_config(config)
}

#[tracing::instrument(level = "trace")]
fn convert_openai_error(error: OpenAIError) -> ApiError {
    match error {
        OpenAIError::ApiError(err) => err,
        _ => {
            event!(Level::WARN, "OpenAIError {:?}", error);
            ApiError {
                message: "The proxy server had an error processing your request. Sorry about that! You can retry your request, or contact the proxy's administrator if the error persists.".to_string(),
                r#type: Some("server_error".to_string()),
                param: Some(Value::Null),
                code: Some(Value::Null),
            }
        }
    }
}

#[tracing::instrument(level = "trace")]
fn convert_error_code(code: ModelErrorCode) -> ApiError {
    match code {
        ModelErrorCode::FailedParse => ApiError {
            message: "We could not parse the JSON body of your request. (HINT: This likely means you aren't using your HTTP library correctly. The OpenAI API expects a JSON payload, but what was sent was not valid JSON. If you have trouble figuring out how to fix this, contact the proxy's administrator.)".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        },
        ModelErrorCode::PromptTooLong => ApiError {
            message: "Your messages exceeded the model's maximum context length. Please reduce the length of the message. If you belive you are seeing this message in error, contact the proxy's administrator.".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        },
        ModelErrorCode::AuthMissing => ApiError {
            message: "You didn't provide an API key. You need to provide your API key in an Authorization header using Bearer auth (i.e. Authorization: Bearer YOUR_KEY), or as the password field (with blank username) if you're accessing the API from your browser and are prompted for a username and password. You can obtain an API key from the proxy's administrator.".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        },
        ModelErrorCode::AuthIncorrect => ApiError {
            message: "Incorrect API key provided. You can obtain an API key from the proxy's administrator.".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("invalid_api_key".to_string())),
        },
        ModelErrorCode::ModelNotFound => ApiError {
            message: "The requested model does not exist.  Contact the proxy's administrator for more information.".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("model_not_found".to_string())),
        },
        ModelErrorCode::EndpointNotFound => ApiError {
            message: "Unknown request URL. Please check the URL for typos, or contact the proxy's administrator for information regarding available endpoints.".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("unknown_url".to_string())),
        },
        ModelErrorCode::RateLimitUser => ApiError {
            message: "You exceeded your current quota, please check your API key's rate limits. For more information on this error, contact the proxy's administrator.".to_string(),
            r#type: Some("insufficient_quota".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("insufficient_quota".to_string())),
        },
        ModelErrorCode::RateLimitModel => ApiError {
            message: "That model is currently overloaded with other requests. You can retry your request, or contact the proxy's administrator if the error persists.".to_string(),
            r#type: Some("server_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        },
        ModelErrorCode::InternalError => ApiError {
            message: "The proxy server had an error processing your request. Sorry about that! You can retry your request, or contact the proxy's administrator if the error persists.".to_string(),
            r#type: Some("server_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        },
        ModelErrorCode::OtherModelError => ApiError {
            message: "The proxy backend had an error processing your request. Sorry about that! Contact the proxy's administrator for more information.".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        },
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIEndpoint {
    openai_api_base: String,
    openai_api_key: String,
    openai_organization: Option<String>,
    proxy_user_ids: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIChatModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIEditModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAICompletionModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIModerationModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIEmbeddingModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIImageModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIAudioModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

impl CallableModelAPI for OpenAIChatModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateChatCompletionRequest;
    type ModelResponse = CreateChatCompletionResponse;
    type ModelError = ApiError;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        user: &str,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = self.model_id.clone();
        request.stream = None;
        request.user = if self.endpoint.proxy_user_ids {
            Some(user.to_string())
        } else {
            None
        };

        client
            .chat()
            .create(request)
            .await
            .map_err(convert_openai_error)
    }

    fn to_request(&self, request: impl RoutableModelRequest+ 'static) -> Option<Self::ModelRequest> {
        let item_any: Box<dyn Any> = Box::new(request);

        match item_any.downcast::<Self::ModelRequest>() {
            Ok(item) => Some(*item),
            Err(_) => None,
        }
    }

    fn to_response(&self, error_code: ModelErrorCode) -> Self::ModelError {
        convert_error_code(error_code)
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

impl CallableModelAPI for OpenAIEditModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateEditRequest;
    type ModelResponse = CreateEditResponse;
    type ModelError = ApiError;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        _user: &str,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = self.model_id.clone();

        #[allow(deprecated)]
        client
            .edits()
            .create(request)
            .await
            .map_err(convert_openai_error)
    }

    fn to_request(&self, request: impl RoutableModelRequest + 'static) -> Option<Self::ModelRequest> {
        let item_any: Box<dyn Any> = Box::new(request);

        match item_any.downcast::<Self::ModelRequest>() {
            Ok(item) => Some(*item),
            Err(_) => None,
        }
    }

    fn to_response(&self, error_code: ModelErrorCode) -> Self::ModelError {
        convert_error_code(error_code)
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

impl CallableModelAPI for OpenAICompletionModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateCompletionRequest;
    type ModelResponse = CreateCompletionResponse;
    type ModelError = ApiError;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        user: &str,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = self.model_id.clone();
        request.stream = None;
        request.user = if self.endpoint.proxy_user_ids {
            Some(user.to_string())
        } else {
            None
        };

        client
            .completions()
            .create(request)
            .await
            .map_err(convert_openai_error)
    }

    fn to_request(&self, request: impl RoutableModelRequest+ 'static) -> Option<Self::ModelRequest> {
        let item_any: Box<dyn Any> = Box::new(request);

        match item_any.downcast::<Self::ModelRequest>() {
            Ok(item) => Some(*item),
            Err(_) => None,
        }
    }

    fn to_response(&self, error_code: ModelErrorCode) -> Self::ModelError {
        convert_error_code(error_code)
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

impl CallableModelAPI for OpenAIModerationModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateModerationRequest;
    type ModelResponse = CreateModerationResponse;
    type ModelError = ApiError;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        _user: &str,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = match &*self.model_id {
            "text-moderation-stable" => Some(TextModerationModel::Stable),
            "text-moderation-latest" => Some(TextModerationModel::Latest),
            _ => None,
        };

        client
            .moderations()
            .create(request)
            .await
            .map_err(convert_openai_error)
    }

    fn to_request(&self, request: impl RoutableModelRequest+ 'static) -> Option<Self::ModelRequest> {
        let item_any: Box<dyn Any> = Box::new(request);

        match item_any.downcast::<Self::ModelRequest>() {
            Ok(item) => Some(*item),
            Err(_) => None,
        }
    }

    fn to_response(&self, error_code: ModelErrorCode) -> Self::ModelError {
        convert_error_code(error_code)
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

impl CallableModelAPI for OpenAIEmbeddingModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateEmbeddingRequest;
    type ModelResponse = CreateEmbeddingResponse;
    type ModelError = ApiError;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        _user: &str,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = self.model_id.clone();

        client
            .embeddings()
            .create(request)
            .await
            .map_err(convert_openai_error)
    }

    fn to_request(&self, request: impl RoutableModelRequest+ 'static) -> Option<Self::ModelRequest> {
        let item_any: Box<dyn Any> = Box::new(request);

        match item_any.downcast::<Self::ModelRequest>() {
            Ok(item) => Some(*item),
            Err(_) => None,
        }
    }

    fn to_response(&self, error_code: ModelErrorCode) -> Self::ModelError {
        convert_error_code(error_code)
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

/*impl ModelAPICallable for OpenAIImageModel {
    type Client = Client<OpenAIConfig>;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        user: &str,
        request: ModelRequest,
    ) -> Option<ModelResponse> {
        match request {
            ModelRequest::Image(mut req) => {
                req.model = match &*self.model_id {
                    "dall-e-3" => Some(ImageModel::DallE3),
                    "dall-e-2" => Some(ImageModel::DallE2),
                    _ => Some(ImageModel::Other(self.model_id.clone())),
                };
                req.user = if self.endpoint.proxy_user_ids {
                    Some(user.to_string())
                } else {
                    None
                };

                match client.images().create(req).await {
                    Ok(g) => Some(ModelResponse::Image(g)),
                    Err(e) => Some(convert_openai_error(e)),
                }
            }
            ModelRequest::ImageEdit(mut req) => {
                req.model = match &*self.model_id {
                    "dall-e-3" => Some(ImageModel::DallE3),
                    "dall-e-2" => Some(ImageModel::DallE2),
                    _ => Some(ImageModel::Other(self.model_id.clone())),
                };
                req.user = if self.endpoint.proxy_user_ids {
                    Some(user.to_string())
                } else {
                    None
                };

                match client.images().create_edit(req).await {
                    Ok(g) => Some(ModelResponse::Image(g)),
                    Err(e) => Some(convert_openai_error(e)),
                }
            }
            ModelRequest::ImageVariation(mut req) => {
                req.model = match &*self.model_id {
                    "dall-e-3" => Some(ImageModel::DallE3),
                    "dall-e-2" => Some(ImageModel::DallE2),
                    _ => Some(ImageModel::Other(self.model_id.clone())),
                };
                req.user = if self.endpoint.proxy_user_ids {
                    Some(user.to_string())
                } else {
                    None
                };

                match client.images().create_variation(req).await {
                    Ok(g) => Some(ModelResponse::Image(g)),
                    Err(e) => Some(convert_openai_error(e)),
                }
            }
            _ => None,
        }
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

impl ModelAPICallable for OpenAIAudioModel {
    type Client = Client<OpenAIConfig>;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        user: &str,
        request: ModelRequest,
    ) -> Option<ModelResponse> {
        match request {
            ModelRequest::Transcription(mut req) => {
                req.model = self.model_id.clone();

                match client.audio().transcribe(req).await {
                    Ok(g) => Some(ModelResponse::Transcription(g)),
                    Err(e) => Some(convert_openai_error(e)),
                }
            }
            ModelRequest::Translation(mut req) => {
                req.model = self.model_id.clone();

                match client.audio().translate(req).await {
                    Ok(g) => Some(ModelResponse::Translation(g)),
                    Err(e) => Some(convert_openai_error(e)),
                }
            }
            _ => None,
        }
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}
*/
