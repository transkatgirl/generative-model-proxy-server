use async_openai::{
    config::OpenAIConfig,
    types::{ImageModel, TextModerationModel},
    Client,
};
use serde::{Deserialize, Serialize};
use tracing::{event, Level};

use super::{ModelAPICallable, ModelRequest, ModelResponse};

// TODO: Improve error handling, forward a *subset* of errors to the user

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

impl ModelAPICallable for OpenAIChatModel {
    type Client = Client<OpenAIConfig>;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        user: &str,
        request: ModelRequest,
    ) -> Option<ModelResponse> {
        if let ModelRequest::Chat(mut req) = request {
            req.model = self.model_id.clone();
            req.stream = None;
            req.user = if self.endpoint.proxy_user_ids {
                Some(user.to_string())
            } else {
                None
            };

            match client.chat().create(req).await {
                Ok(g) => Some(ModelResponse::Chat(g)),
                Err(e) => {
                    event!(Level::WARN, "OpenAIError {:?}", e);
                    Some(ModelResponse::error_internal())
                }
            }
        } else {
            None
        }
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

impl ModelAPICallable for OpenAIEditModel {
    type Client = Client<OpenAIConfig>;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        _user: &str,
        request: ModelRequest,
    ) -> Option<ModelResponse> {
        if let ModelRequest::Edit(mut req) = request {
            req.model = self.model_id.clone();

            #[allow(deprecated)]
            match client.edits().create(req).await {
                Ok(g) => Some(ModelResponse::Edit(g)),
                Err(e) => {
                    event!(Level::WARN, "OpenAIError {:?}", e);
                    Some(ModelResponse::error_internal())
                }
            }
        } else {
            None
        }
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

impl ModelAPICallable for OpenAICompletionModel {
    type Client = Client<OpenAIConfig>;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        user: &str,
        request: ModelRequest,
    ) -> Option<ModelResponse> {
        if let ModelRequest::Completion(mut req) = request {
            req.model = self.model_id.clone();
            req.stream = None;
            req.user = if self.endpoint.proxy_user_ids {
                Some(user.to_string())
            } else {
                None
            };

            match client.completions().create(req).await {
                Ok(g) => Some(ModelResponse::Completion(g)),
                Err(e) => {
                    event!(Level::WARN, "OpenAIError {:?}", e);
                    Some(ModelResponse::error_internal())
                }
            }
        } else {
            None
        }
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

impl ModelAPICallable for OpenAIModerationModel {
    type Client = Client<OpenAIConfig>;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        _user: &str,
        request: ModelRequest,
    ) -> Option<ModelResponse> {
        if let ModelRequest::Moderation(mut req) = request {
            req.model = match &*self.model_id {
                "text-moderation-stable" => Some(TextModerationModel::Stable),
                "text-moderation-latest" => Some(TextModerationModel::Latest),
                _ => None,
            };

            match client.moderations().create(req).await {
                Ok(g) => Some(ModelResponse::Moderation(g)),
                Err(e) => {
                    event!(Level::WARN, "OpenAIError {:?}", e);
                    Some(ModelResponse::error_internal())
                }
            }
        } else {
            None
        }
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

impl ModelAPICallable for OpenAIEmbeddingModel {
    type Client = Client<OpenAIConfig>;

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        _user: &str,
        request: ModelRequest,
    ) -> Option<ModelResponse> {
        if let ModelRequest::Embedding(mut req) = request {
            req.model = self.model_id.clone();

            match client.embeddings().create(req).await {
                Ok(g) => Some(ModelResponse::Embedding(g)),
                Err(e) => {
                    event!(Level::WARN, "OpenAIError {:?}", e);
                    Some(ModelResponse::error_internal())
                }
            }
        } else {
            None
        }
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}

impl ModelAPICallable for OpenAIImageModel {
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
                    Err(e) => {
                        event!(Level::WARN, "OpenAIError {:?}", e);
                        Some(ModelResponse::error_internal())
                    }
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
                    Err(e) => {
                        event!(Level::WARN, "OpenAIError {:?}", e);
                        Some(ModelResponse::error_internal())
                    }
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
                    Err(e) => {
                        event!(Level::WARN, "OpenAIError {:?}", e);
                        Some(ModelResponse::error_internal())
                    }
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
                    Err(e) => {
                        event!(Level::WARN, "OpenAIError {:?}", e);
                        Some(ModelResponse::error_internal())
                    }
                }
            }
            ModelRequest::Translation(mut req) => {
                req.model = self.model_id.clone();

                match client.audio().translate(req).await {
                    Ok(g) => Some(ModelResponse::Translation(g)),
                    Err(e) => {
                        event!(Level::WARN, "OpenAIError {:?}", e);
                        Some(ModelResponse::error_internal())
                    }
                }
            }
            _ => None,
        }
    }

    fn init(&self) -> Self::Client {
        init_openai_client(self.endpoint.clone())
    }
}
