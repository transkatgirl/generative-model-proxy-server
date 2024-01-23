#![allow(deprecated)]
use async_openai::{
    config::{self, OpenAIConfig},
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

// TODO: Improve error handling, add logging
// TODO: Add proxy for image output URLs
/// async_openai::types::ImagesResponse.save()

fn convert_openai_error(error: OpenAIError) -> ApiError {
    match error {
        OpenAIError::ApiError(err) => err,
        OpenAIError::Reqwest(err) => {
            println!("{:?}", err);
            ApiError {
				message: "The proxy server had an error processing your request. You can retry your request, or contact the proxy's administrator if you keep seeing this error.".to_string(),
				r#type: Some("server_error".to_string()),
				param: Some(Value::Null),
				code: Some(Value::Null),
			}
        }
        OpenAIError::JSONDeserialize(err) => {
            println!("{:?}", err);
            ApiError {
				message: "The proxy server had an error processing your request. You can retry your request, or contact the proxy's administrator if you keep seeing this error.".to_string(),
				r#type: Some("server_error".to_string()),
				param: Some(Value::Null),
				code: Some(Value::Null),
			}
        }
        OpenAIError::FileSaveError(err) => {
            println!("{:?}", err);
            ApiError {
				message: "The proxy server had an error processing your request. You can retry your request, or contact the proxy's administrator if you keep seeing this error.".to_string(),
				r#type: Some("server_error".to_string()),
				param: Some(Value::Null),
				code: Some(Value::Null),
			}
        }
        OpenAIError::FileReadError(err) => {
            println!("{:?}", err);
            ApiError {
				message: "The proxy server had an error processing your request. You can retry your request, or contact the proxy's administrator if you keep seeing this error.".to_string(),
				r#type: Some("server_error".to_string()),
				param: Some(Value::Null),
				code: Some(Value::Null),
			}
        }
        OpenAIError::StreamError(err) => {
            println!("{:?}", err);
            ApiError {
				message: "The proxy server had an error processing your request. You can retry your request, or contact the proxy's administrator if you keep seeing this error.".to_string(),
				r#type: Some("server_error".to_string()),
				param: Some(Value::Null),
				code: Some(Value::Null),
			}
        }
        OpenAIError::InvalidArgument(err) => {
            println!("{:?}", err);
            ApiError {
				message: "The proxy server had an error processing your request. You can retry your request, or contact the proxy's administrator if you keep seeing this error.".to_string(),
				r#type: Some("server_error".to_string()),
				param: Some(Value::Null),
				code: Some(Value::Null),
			}
        }
    }
}

fn init_openai_client(endpoint: OpenAIEndpoint) -> Client<OpenAIConfig> {
    let mut config = OpenAIConfig::new()
        .with_api_base(endpoint.openai_api_base)
        .with_api_key(endpoint.openai_api_key);

    if let Some(org) = endpoint.openai_organization {
        config = config.with_org_id(org);
    }

    async_openai::Client::with_config(config)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OpenAIEndpoint {
    openai_api_base: String,
    openai_api_key: String,
    openai_organization: Option<String>,
    proxy_user_ids: bool,
}

#[derive(Serialize, Deserialize)]
pub struct OpenAIChatModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct OpenAIEditModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct OpenAICompletionModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct OpenAIModerationModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct OpenAIEmbeddingModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct OpenAIImageModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct OpenAIAudioModel {
    endpoint: OpenAIEndpoint,
    model_id: String,
}

impl OpenAIChatModel {
    pub async fn generate<C: config::Config>(
        &self,
        client: &Client<C>,
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
    pub async fn generate<C: config::Config>(
        &self,
        client: &Client<C>,
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
    pub async fn generate<C: config::Config>(
        &self,
        client: &Client<C>,
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
    pub async fn generate<C: config::Config>(
        &self,
        client: &Client<C>,
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
    pub async fn generate<C: config::Config>(
        &self,
        client: &Client<C>,
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
    pub async fn generate<C: config::Config>(
        &self,
        client: &Client<C>,
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

    pub async fn generate_edit<C: config::Config>(
        &self,
        client: &Client<C>,
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

    pub async fn generate_variation<C: config::Config>(
        &self,
        client: &Client<C>,
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
    pub async fn generate_transcription<C: config::Config>(
        &self,
        client: &Client<C>,
        mut request: CreateTranscriptionRequest,
    ) -> Result<CreateTranscriptionResponse, ApiError> {
        request.model = self.model_id.clone();

        match client.audio().transcribe(request).await {
            Ok(g) => Ok(g),
            Err(e) => Err(convert_openai_error(e)),
        }
    }

    pub async fn generate_translation<C: config::Config>(
        &self,
        client: &Client<C>,
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