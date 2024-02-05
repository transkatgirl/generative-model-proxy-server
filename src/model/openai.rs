use async_openai::{
    config::OpenAIConfig,
    error::OpenAIError,
    types::{
        CreateImageEditRequest, CreateImageRequest, CreateImageVariationRequest,
        CreateTranscriptionRequest, CreateTranscriptionResponse, CreateTranslationRequest,
        CreateTranslationResponse, EmbeddingInput, ImageModel, ModerationInput, Prompt,
        TextModerationModel,
    },
    Client,
};
pub(super) use async_openai::{
    error::ApiError,
    types::{
        CreateChatCompletionRequest, CreateChatCompletionResponse, CreateCompletionRequest,
        CreateCompletionResponse, CreateEditRequest, CreateEditResponse, CreateEmbeddingRequest,
        CreateEmbeddingResponse, CreateModerationRequest, CreateModerationResponse, ImagesResponse,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::value::Value;
use tracing::{event, Level};

use super::{CallableModelAPI, ResponseStatus, RoutableModelRequest, RoutableModelResponse};

impl RoutableModelRequest for CreateChatCompletionRequest {
    fn get_model(&self) -> String {
        self.model.clone()
    }

    fn get_total_n(&self) -> u32 {
        self.n.unwrap_or(1) as u32
    }
}

impl RoutableModelResponse for CreateChatCompletionResponse {
    fn get_status(&self) -> ResponseStatus {
        ResponseStatus::Success
    }

    fn get_token_count(&self) -> Option<u32> {
        self.usage.as_ref().map(|u| u.total_tokens)
    }
}

impl RoutableModelRequest for CreateEditRequest {
    fn get_model(&self) -> String {
        self.model.clone()
    }

    fn get_total_n(&self) -> u32 {
        self.n.unwrap_or(1) as u32
    }
}

impl RoutableModelResponse for CreateEditResponse {
    fn get_status(&self) -> ResponseStatus {
        ResponseStatus::Success
    }

    fn get_token_count(&self) -> Option<u32> {
        Some(self.usage.total_tokens)
    }
}

impl RoutableModelRequest for CreateCompletionRequest {
    fn get_model(&self) -> String {
        self.model.clone()
    }

    fn get_total_n(&self) -> u32 {
        self.best_of.unwrap_or(1).max(self.n.unwrap_or(1)) as u32
            * match &self.prompt {
                Prompt::String(_) => 1,
                Prompt::StringArray(p) => p.len() as u32,
                Prompt::IntegerArray(_) => 1,
                Prompt::ArrayOfIntegerArray(p) => p.len() as u32,
            }
    }
}

impl RoutableModelResponse for CreateCompletionResponse {
    fn get_status(&self) -> ResponseStatus {
        ResponseStatus::Success
    }

    fn get_token_count(&self) -> Option<u32> {
        self.usage.as_ref().map(|u| u.total_tokens)
    }
}

impl RoutableModelRequest for CreateModerationRequest {
    fn get_model(&self) -> String {
        match self.model {
            Some(TextModerationModel::Stable) => "text-moderation-stable",
            Some(TextModerationModel::Latest) => "text-moderation-latest",
            None => "text-moderation-latest",
        }
        .to_string()
    }

    fn get_total_n(&self) -> u32 {
        match &self.input {
            ModerationInput::String(_) => 1,
            ModerationInput::StringArray(p) => p.len() as u32,
        }
    }
}

impl RoutableModelResponse for CreateModerationResponse {
    fn get_status(&self) -> ResponseStatus {
        ResponseStatus::Success
    }

    fn get_token_count(&self) -> Option<u32> {
        None
    }
}

impl RoutableModelRequest for CreateEmbeddingRequest {
    fn get_model(&self) -> String {
        self.model.clone()
    }

    fn get_total_n(&self) -> u32 {
        match &self.input {
            EmbeddingInput::String(_) => 1,
            EmbeddingInput::StringArray(p) => p.len() as u32,
            EmbeddingInput::IntegerArray(_) => 1,
            EmbeddingInput::ArrayOfIntegerArray(p) => p.len() as u32,
        }
    }
}

impl RoutableModelResponse for CreateEmbeddingResponse {
    fn get_status(&self) -> ResponseStatus {
        ResponseStatus::Success
    }

    fn get_token_count(&self) -> Option<u32> {
        Some(self.usage.total_tokens)
    }
}

impl RoutableModelRequest for CreateImageRequest {
    fn get_model(&self) -> String {
        match &self.model {
            Some(ImageModel::DallE3) => "dall-e-3".to_string(),
            Some(ImageModel::DallE2) => "dall-e-2".to_string(),
            Some(ImageModel::Other(m)) => m.clone(),
            None => "dall-e-2".to_string(),
        }
    }

    fn get_total_n(&self) -> u32 {
        self.n.unwrap_or(1) as u32
    }
}

impl RoutableModelRequest for CreateImageEditRequest {
    fn get_model(&self) -> String {
        match &self.model {
            Some(ImageModel::DallE3) => "dall-e-3".to_string(),
            Some(ImageModel::DallE2) => "dall-e-2".to_string(),
            Some(ImageModel::Other(m)) => m.clone(),
            None => "dall-e-2".to_string(),
        }
    }

    fn get_total_n(&self) -> u32 {
        self.n.unwrap_or(1) as u32
    }
}

impl RoutableModelRequest for CreateImageVariationRequest {
    fn get_model(&self) -> String {
        match &self.model {
            Some(ImageModel::DallE3) => "dall-e-3".to_string(),
            Some(ImageModel::DallE2) => "dall-e-2".to_string(),
            Some(ImageModel::Other(m)) => m.clone(),
            None => "dall-e-2".to_string(),
        }
    }

    fn get_total_n(&self) -> u32 {
        self.n.unwrap_or(1) as u32
    }
}

impl RoutableModelResponse for ImagesResponse {
    fn get_status(&self) -> ResponseStatus {
        ResponseStatus::Success
    }

    fn get_token_count(&self) -> Option<u32> {
        None
    }
}

impl RoutableModelRequest for CreateTranscriptionRequest {
    fn get_model(&self) -> String {
        self.model.clone()
    }

    fn get_total_n(&self) -> u32 {
        1
    }
}

impl RoutableModelResponse for CreateTranscriptionResponse {
    fn get_status(&self) -> ResponseStatus {
        ResponseStatus::Success
    }

    fn get_token_count(&self) -> Option<u32> {
        None
    }
}

impl RoutableModelRequest for CreateTranslationRequest {
    fn get_model(&self) -> String {
        self.model.clone()
    }

    fn get_total_n(&self) -> u32 {
        1
    }
}

impl RoutableModelResponse for CreateTranslationResponse {
    fn get_status(&self) -> ResponseStatus {
        ResponseStatus::Success
    }

    fn get_token_count(&self) -> Option<u32> {
        None
    }
}

impl RoutableModelResponse for ApiError {
    fn get_status(&self) -> ResponseStatus {
        match self.r#type.as_deref() {
            Some("invalid_request_error") => ResponseStatus::InvalidRequest,
            Some("insufficient_quota") => ResponseStatus::ModelUnavailable,
            Some("server_error") => ResponseStatus::BadUpstream,
            _ => ResponseStatus::InternalError,
        }
    }

    fn get_token_count(&self) -> Option<u32> {
        None
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) struct OpenAIEndpoint {
    openai_api_base: String,
    openai_api_key: String,
    openai_organization: Option<String>,
    proxy_user_ids: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub(super) struct OpenAIChatModel {
    endpoint: OpenAIEndpoint,
    context_len: u32,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(super) struct OpenAIEditModel {
    endpoint: OpenAIEndpoint,
    context_len: u32,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(super) struct OpenAICompletionModel {
    endpoint: OpenAIEndpoint,
    context_len: u32,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(super) struct OpenAIModerationModel {
    endpoint: OpenAIEndpoint,
    context_len: u32,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(super) struct OpenAIEmbeddingModel {
    endpoint: OpenAIEndpoint,
    context_len: u32,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(super) struct OpenAIImageModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub(super) struct OpenAIAudioModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

impl CallableModelAPI for OpenAIChatModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateChatCompletionRequest;
    type ModelResponse = CreateChatCompletionResponse;
    type ModelError = ApiError;

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }

    fn get_context_len(&self) -> Option<u32> {
        Some(self.context_len)
    }

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        request_label: &str,
        model_label: &str,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = self.model_id.clone();
        request.stream = None;
        request.user = if self.endpoint.proxy_user_ids {
            Some(request_label.to_string())
        } else {
            None
        };

        client
            .chat()
            .create(request)
            .await
            .map(|mut response| {
                response.model = model_label.to_string();
                response
            })
            .map_err(convert_openai_error)
    }
}

impl CallableModelAPI for OpenAIEditModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateEditRequest;
    type ModelResponse = CreateEditResponse;
    type ModelError = ApiError;

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }

    fn get_context_len(&self) -> Option<u32> {
        Some(self.context_len)
    }

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        _request_label: &str,
        _model_label: &str,
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
}

impl CallableModelAPI for OpenAICompletionModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateCompletionRequest;
    type ModelResponse = CreateCompletionResponse;
    type ModelError = ApiError;

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }

    fn get_context_len(&self) -> Option<u32> {
        Some(self.context_len)
    }

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        request_label: &str,
        model_label: &str,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = self.model_id.clone();
        request.stream = None;
        request.user = if self.endpoint.proxy_user_ids {
            Some(request_label.to_string())
        } else {
            None
        };

        client
            .completions()
            .create(request)
            .await
            .map(|mut response| {
                response.model = model_label.to_string();
                response
            })
            .map_err(convert_openai_error)
    }
}

impl CallableModelAPI for OpenAIModerationModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateModerationRequest;
    type ModelResponse = CreateModerationResponse;
    type ModelError = ApiError;

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }

    fn get_context_len(&self) -> Option<u32> {
        Some(self.context_len)
    }

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        _request_label: &str,
        model_label: &str,
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
            .map(|mut response| {
                response.model = model_label.to_string();
                response
            })
            .map_err(convert_openai_error)
    }
}

impl CallableModelAPI for OpenAIEmbeddingModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateEmbeddingRequest;
    type ModelResponse = CreateEmbeddingResponse;
    type ModelError = ApiError;

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }

    fn get_context_len(&self) -> Option<u32> {
        Some(self.context_len)
    }

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        _request_label: &str,
        model_label: &str,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = self.model_id.clone();

        client
            .embeddings()
            .create(request)
            .await
            .map(|mut response| {
                response.model = model_label.to_string();
                response
            })
            .map_err(convert_openai_error)
    }
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub(super) enum ImagesRequest {
    Image(CreateImageRequest),
    Edit(CreateImageEditRequest),
    Variation(CreateImageVariationRequest),
}

impl RoutableModelRequest for ImagesRequest {
    fn get_model(&self) -> String {
        match self {
            Self::Image(r) => r.get_model(),
            Self::Edit(r) => r.get_model(),
            Self::Variation(r) => r.get_model(),
        }
    }

    fn get_total_n(&self) -> u32 {
        match self {
            Self::Image(r) => r.get_total_n(),
            Self::Edit(r) => r.get_total_n(),
            Self::Variation(r) => r.get_total_n(),
        }
    }
}

impl CallableModelAPI for OpenAIImageModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = ImagesRequest;
    type ModelResponse = ImagesResponse;
    type ModelError = ApiError;

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }

    fn get_context_len(&self) -> Option<u32> {
        None
    }

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        request_label: &str,
        _model_label: &str,
        request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        match request {
            ImagesRequest::Image(mut request) => {
                request.model = match self.model_id.as_ref() {
                    "dall-e-3" => Some(ImageModel::DallE3),
                    "dall-e-2" => Some(ImageModel::DallE2),
                    _ => Some(ImageModel::Other(self.model_id.clone())),
                };
                request.user = if self.endpoint.proxy_user_ids {
                    Some(request_label.to_string())
                } else {
                    None
                };

                client
                    .images()
                    .create(request)
                    .await
                    .map_err(convert_openai_error)
            }
            ImagesRequest::Edit(mut request) => {
                request.model = match self.model_id.as_ref() {
                    "dall-e-3" => Some(ImageModel::DallE3),
                    "dall-e-2" => Some(ImageModel::DallE2),
                    _ => Some(ImageModel::Other(self.model_id.clone())),
                };
                request.user = if self.endpoint.proxy_user_ids {
                    Some(request_label.to_string())
                } else {
                    None
                };

                client
                    .images()
                    .create_edit(request)
                    .await
                    .map_err(convert_openai_error)
            }
            ImagesRequest::Variation(mut request) => {
                request.model = match self.model_id.as_ref() {
                    "dall-e-3" => Some(ImageModel::DallE3),
                    "dall-e-2" => Some(ImageModel::DallE2),
                    _ => Some(ImageModel::Other(self.model_id.clone())),
                };
                request.user = if self.endpoint.proxy_user_ids {
                    Some(request_label.to_string())
                } else {
                    None
                };

                client
                    .images()
                    .create_variation(request)
                    .await
                    .map_err(convert_openai_error)
            }
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub(super) enum AudioRequest {
    Transcription(CreateTranscriptionRequest),
    Translation(CreateTranslationRequest),
}

impl RoutableModelRequest for AudioRequest {
    fn get_model(&self) -> String {
        match self {
            Self::Transcription(r) => r.get_model(),
            Self::Translation(r) => r.get_model(),
        }
    }

    fn get_total_n(&self) -> u32 {
        match self {
            Self::Transcription(r) => r.get_total_n(),
            Self::Translation(r) => r.get_total_n(),
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(untagged)]
pub(super) enum AudioResponse {
    Transcription(CreateTranscriptionResponse),
    Translation(CreateTranslationResponse),
}

impl RoutableModelResponse for AudioResponse {
    fn get_status(&self) -> ResponseStatus {
        match self {
            Self::Transcription(r) => r.get_status(),
            Self::Translation(r) => r.get_status(),
        }
    }

    fn get_token_count(&self) -> Option<u32> {
        match self {
            Self::Transcription(r) => r.get_token_count(),
            Self::Translation(r) => r.get_token_count(),
        }
    }
}

impl CallableModelAPI for OpenAIAudioModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = AudioRequest;
    type ModelResponse = AudioResponse;
    type ModelError = ApiError;

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }

    fn get_context_len(&self) -> Option<u32> {
        None
    }

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        _request_label: &str,
        _model_label: &str,
        request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        match request {
            AudioRequest::Transcription(mut request) => {
                request.model = self.model_id.clone();

                client
                    .audio()
                    .transcribe(request)
                    .await
                    .map(AudioResponse::Transcription)
                    .map_err(convert_openai_error)
            }
            AudioRequest::Translation(mut request) => {
                request.model = self.model_id.clone();

                client
                    .audio()
                    .translate(request)
                    .await
                    .map(AudioResponse::Translation)
                    .map_err(convert_openai_error)
            }
        }
    }
}
