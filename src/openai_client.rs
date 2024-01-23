#![allow(deprecated)]
use async_openai::{
    config::OpenAIConfig,
    error::{ApiError, OpenAIError},
    types::{
        CreateChatCompletionRequest, CreateChatCompletionResponse, CreateCompletionRequest,
        CreateCompletionResponse, CreateEditRequest, CreateEditResponse, CreateEmbeddingRequest,
        CreateEmbeddingResponse, CreateImageEditRequest, CreateImageRequest,
        CreateImageVariationRequest, CreateModerationRequest, CreateModerationResponse,
        CreateTranscriptionRequest, CreateTranscriptionResponse, CreateTranslationRequest,
        CreateTranslationResponse, ImageModel, ImagesResponse, TextModerationModel,
    },
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::value::Value;
use tracing::{event, Level};

// TODO: Add proxy for image output URLs?
/// See async_openai::types::ImagesResponse.save()
// TODO: Add proxy for GPT-4 image input URLs?

fn convert_openai_error(error: OpenAIError) -> ApiError {
    event!(Level::WARN, "OpenAIError {:?}", error);

    ApiError {
        message: "The proxy server had an error processing your request. You can retry your request, or contact the proxy's administrator if you keep seeing this error.".to_string(),
        r#type: Some("server_error".to_string()),
        param: Some(Value::Null),
        code: Some(Value::Null),
    }
}

#[tracing::instrument(level = "trace")]
fn init_openai_client(endpoint: OpenAIEndpoint) -> Client<OpenAIConfig> {
    event!(Level::TRACE, "Init client {:?}", endpoint);

    let mut config = OpenAIConfig::new()
        .with_api_base(endpoint.openai_api_base)
        .with_api_key(endpoint.openai_api_key);

    if let Some(org) = endpoint.openai_organization {
        config = config.with_org_id(org);
    }

    async_openai::Client::with_config(config)
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct OpenAIEndpoint {
    openai_api_base: String,
    openai_api_key: String,
    openai_organization: Option<String>,
    proxy_user_ids: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIChatModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIEditModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAICompletionModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIModerationModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIEmbeddingModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIImageModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIAudioModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

impl OpenAIChatModel {
    #[tracing::instrument(level = "trace")]
    pub async fn generate(
        &self,
        client: &Client<OpenAIConfig>,
        user: &str,
        mut request: CreateChatCompletionRequest,
    ) -> Result<CreateChatCompletionResponse, ApiError> {
        request.model = self.model_id.clone();
        request.stream = None;
        request.user = if self.endpoint.proxy_user_ids {
            Some(user.to_string())
        } else {
            None
        };

        match client.chat().create(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    pub fn init_client(&self) -> Client<OpenAIConfig> {
        init_openai_client(self.endpoint.clone())
    }
}

impl OpenAIEditModel {
    #[tracing::instrument(level = "trace")]
    pub async fn generate(
        &self,
        client: &Client<OpenAIConfig>,
        mut request: CreateEditRequest,
    ) -> Result<CreateEditResponse, ApiError> {
        request.model = self.model_id.clone();

        match client.edits().create(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    pub fn init_client(&self) -> Client<OpenAIConfig> {
        init_openai_client(self.endpoint.clone())
    }
}

impl OpenAICompletionModel {
    #[tracing::instrument(level = "trace")]
    pub async fn generate(
        &self,
        client: &Client<OpenAIConfig>,
        user: &str,
        mut request: CreateCompletionRequest,
    ) -> Result<CreateCompletionResponse, ApiError> {
        request.model = self.model_id.clone();
        request.stream = None;
        request.user = if self.endpoint.proxy_user_ids {
            Some(user.to_string())
        } else {
            None
        };

        match client.completions().create(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    pub fn init_client(&self) -> Client<OpenAIConfig> {
        init_openai_client(self.endpoint.clone())
    }
}

impl OpenAIModerationModel {
    #[tracing::instrument(level = "trace")]
    pub async fn generate(
        &self,
        client: &Client<OpenAIConfig>,
        mut request: CreateModerationRequest,
    ) -> Result<CreateModerationResponse, ApiError> {
        request.model = match &*self.model_id {
            "text-moderation-stable" => Some(TextModerationModel::Stable),
            "text-moderation-latest" => Some(TextModerationModel::Latest),
            _ => None,
        };

        match client.moderations().create(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    pub fn init_client(&self) -> Client<OpenAIConfig> {
        init_openai_client(self.endpoint.clone())
    }
}

impl OpenAIEmbeddingModel {
    #[tracing::instrument(level = "trace")]
    pub async fn generate(
        &self,
        client: &Client<OpenAIConfig>,
        mut request: CreateEmbeddingRequest,
    ) -> Result<CreateEmbeddingResponse, ApiError> {
        request.model = self.model_id.clone();

        match client.embeddings().create(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    pub fn init_client(&self) -> Client<OpenAIConfig> {
        init_openai_client(self.endpoint.clone())
    }
}

impl OpenAIImageModel {
    #[tracing::instrument(level = "trace")]
    pub async fn generate(
        &self,
        client: &Client<OpenAIConfig>,
        user: &str,
        mut request: CreateImageRequest,
    ) -> Result<ImagesResponse, ApiError> {
        request.model = match &*self.model_id {
            "dall-e-3" => Some(ImageModel::DallE3),
            "dall-e-2" => Some(ImageModel::DallE2),
            _ => Some(ImageModel::Other(self.model_id.clone())),
        };
        request.user = if self.endpoint.proxy_user_ids {
            Some(user.to_string())
        } else {
            None
        };

        match client.images().create(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    #[tracing::instrument(level = "trace")]
    pub async fn generate_edit(
        &self,
        client: &Client<OpenAIConfig>,
        user: &str,
        mut request: CreateImageEditRequest,
    ) -> Result<ImagesResponse, ApiError> {
        request.model = match &*self.model_id {
            "dall-e-3" => Some(ImageModel::DallE3),
            "dall-e-2" => Some(ImageModel::DallE2),
            _ => Some(ImageModel::Other(self.model_id.clone())),
        };
        request.user = if self.endpoint.proxy_user_ids {
            Some(user.to_string())
        } else {
            None
        };

        match client.images().create_edit(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    #[tracing::instrument(level = "trace")]
    pub async fn generate_variation(
        &self,
        client: &Client<OpenAIConfig>,
        user: &str,
        mut request: CreateImageVariationRequest,
    ) -> Result<ImagesResponse, ApiError> {
        request.model = match &*self.model_id {
            "dall-e-3" => Some(ImageModel::DallE3),
            "dall-e-2" => Some(ImageModel::DallE2),
            _ => Some(ImageModel::Other(self.model_id.clone())),
        };
        request.user = if self.endpoint.proxy_user_ids {
            Some(user.to_string())
        } else {
            None
        };

        match client.images().create_variation(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    pub fn init_client(&self) -> Client<OpenAIConfig> {
        init_openai_client(self.endpoint.clone())
    }
}

impl OpenAIAudioModel {
    #[tracing::instrument(level = "trace")]
    pub async fn generate_transcription(
        &self,
        client: &Client<OpenAIConfig>,
        mut request: CreateTranscriptionRequest,
    ) -> Result<CreateTranscriptionResponse, ApiError> {
        request.model = self.model_id.clone();

        match client.audio().transcribe(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    #[tracing::instrument(level = "trace")]
    pub async fn generate_translation(
        &self,
        client: &Client<OpenAIConfig>,
        mut request: CreateTranslationRequest,
    ) -> Result<CreateTranslationResponse, ApiError> {
        request.model = self.model_id.clone();

        match client.audio().translate(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    pub fn init_client(&self) -> Client<OpenAIConfig> {
        init_openai_client(self.endpoint.clone())
    }
}
