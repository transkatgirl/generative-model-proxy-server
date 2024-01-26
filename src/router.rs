use core::num;
use std::{
    collections::HashMap, future::Future, hash::Hash, num::NonZeroU32, sync::Arc, thread::spawn, fmt::Debug,
};

use async_openai::{
    error::ApiError,
    types::{
        CreateChatCompletionRequest, CreateChatCompletionResponse, CreateCompletionRequest,
        CreateCompletionResponse, CreateEditRequest, CreateEditResponse, CreateEmbeddingRequest,
        CreateEmbeddingResponse, CreateImageEditRequest, CreateImageRequest,
        CreateImageVariationRequest, CreateModerationRequest, CreateModerationResponse,
        CreateTranscriptionRequest, CreateTranscriptionResponse, CreateTranslationRequest,
        CreateTranslationResponse, ImageModel, ImagesResponse, TextModerationModel,
    },
};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{
        mpsc::{self, error::SendTimeoutError, UnboundedReceiver},
        oneshot, RwLock,
    },
    time::{self, Duration, Instant},
};
use tracing::{event, Level};
use uuid::Uuid;

use self::limiter::{RequestQuotaStatus, TokenQuotaStatus};
use crate::api;

mod error;
mod limiter;
mod openai_client;
mod tokenizer;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)]
pub enum ModelAPI {
    OpenAIChat(openai_client::OpenAIChatModel),
    OpenAIEdit(openai_client::OpenAIEditModel),
    OpenAICompletion(openai_client::OpenAICompletionModel),
    OpenAIModeration(openai_client::OpenAIModerationModel),
    OpenAIEmbedding(openai_client::OpenAIEmbeddingModel),
    OpenAIImage(openai_client::OpenAIImageModel),
    OpenAIAudio(openai_client::OpenAIAudioModel),
}

pub trait ModelAPICallable {
    type Client: Send;

    fn init(&self) -> Self::Client;

    fn generate(
        &self,
        client: &Self::Client,
        user: &str,
        request: ModelRequest,
    ) -> impl Future<Output = Option<ModelResponse>> + Send;
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum ModelRequest {
    Chat(CreateChatCompletionRequest),
    Edit(CreateEditRequest),
    Completion(CreateCompletionRequest),
    Moderation(CreateModerationRequest),
    Embedding(CreateEmbeddingRequest),
    Image(CreateImageRequest),
    ImageEdit(CreateImageEditRequest),
    ImageVariation(CreateImageVariationRequest),
    Transcription(CreateTranscriptionRequest),
    Translation(CreateTranslationRequest),
}

impl ModelRequest {
    #[tracing::instrument(level = "debug")]
    pub fn get_model(&self) -> String {
        match self {
            Self::Chat(r) => r.model.clone(),
            Self::Edit(r) => r.model.clone(),
            Self::Completion(r) => r.model.clone(),
            Self::Moderation(r) => match r.model {
                Some(TextModerationModel::Stable) => "text-moderation-stable",
                Some(TextModerationModel::Latest) => "text-moderation-latest",
                None => "text-moderation-latest",
            }
            .to_string(),
            Self::Embedding(r) => r.model.clone(),
            Self::Image(r) => match r.model.clone() {
                Some(ImageModel::DallE3) => "dall-e-3".to_string(),
                Some(ImageModel::DallE2) => "dall-e-2".to_string(),
                Some(ImageModel::Other(m)) => m,
                None => "dall-e-2".to_string(),
            },
            Self::ImageEdit(r) => match r.model.clone() {
                Some(ImageModel::DallE3) => "dall-e-3".to_string(),
                Some(ImageModel::DallE2) => "dall-e-2".to_string(),
                Some(ImageModel::Other(m)) => m,
                None => "dall-e-2".to_string(),
            },
            Self::ImageVariation(r) => match r.model.clone() {
                Some(ImageModel::DallE3) => "dall-e-3".to_string(),
                Some(ImageModel::DallE2) => "dall-e-2".to_string(),
                Some(ImageModel::Other(m)) => m,
                None => "dall-e-2".to_string(),
            },
            Self::Transcription(r) => r.model.clone(),
            Self::Translation(r) => r.model.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum ModelResponse {
    Error(ApiError),
    Chat(CreateChatCompletionResponse),
    Edit(CreateEditResponse),
    Completion(CreateCompletionResponse),
    Moderation(CreateModerationResponse),
    Embedding(CreateEmbeddingResponse),
    Image(ImagesResponse),
    Transcription(CreateTranscriptionResponse),
    Translation(CreateTranslationResponse),
}

impl ModelResponse {
    #[tracing::instrument(level = "debug")]
    pub fn replace_model_id(&mut self, model_id: String) {
        match self {
            Self::Error(_) => {}
            Self::Chat(r) => r.model = model_id,
            Self::Edit(_) => {}
            Self::Completion(r) => r.model = model_id,
            Self::Moderation(r) => r.model = model_id,
            Self::Embedding(r) => r.model = model_id,
            Self::Image(_) => {}
            Self::Transcription(_) => {}
            Self::Translation(_) => {}
        }
    }
}

struct RoutableRequest {
    body: ModelRequest,
    user_id: Uuid,
    response_channel: oneshot::Sender<ModelResponse>,
}

// TODO: Add proxy for image URLs (GPT-4 input, image model output)

#[tracing::instrument(level = "trace")]
fn spawn_model_handler(
    model_metadata: api::Model,
    model_callable: impl ModelAPICallable + Send + Debug + 'static,
) -> mpsc::Sender<RoutableRequest> {
    let (tx, mut rx) =
        mpsc::channel::<RoutableRequest>(if model_metadata.quota.max_queue_size == 0 {
            64
        } else {
            model_metadata.quota.max_queue_size
        });

    tokio::spawn(async move {
        let client = model_callable.init();
        let limiter = limiter::Limiter::new(model_metadata.quota);
        let mut encode_buffer: [u8; 45] = Uuid::encode_buffer();

        while let Some(request) = rx.recv().await {
            let user: &mut str = request.user_id.simple().encode_lower(&mut encode_buffer);

            if let RequestQuotaStatus::LimitedUntil(point) = limiter.request() {
                time::sleep_until(Instant::from_std(point)).await;
            }

            let tokens = request.body.get_token_count(&model_metadata);
            if let Some(tokens) = tokens {
                match request.body.get_max_tokens(&model_metadata) {
                    Some(max) => match limiter.tokens_bounded(tokens as u32, max as u32) {
                        TokenQuotaStatus::Ready(_) => {}
                        TokenQuotaStatus::LimitedUntil(point) => {
                            time::sleep_until(Instant::from_std(point)).await;
                        }
                        TokenQuotaStatus::Oversized => {
                            ModelResponse::error_prompt_too_long();
                        }
                    },
                    None => match limiter.tokens(tokens as u32) {
                        TokenQuotaStatus::Ready(_) => {}
                        TokenQuotaStatus::LimitedUntil(point) => {
                            time::sleep_until(Instant::from_std(point)).await;
                        }
                        TokenQuotaStatus::Oversized => {
                            ModelResponse::error_user_rate_limit();
                        }
                    },
                };
            }

            // TODO: Figure out how to spawn request handling into it's own Task

            let response = match model_callable.generate(&client, user, request.body).await {
                Some(mut g) => {
                    g.replace_model_id(model_metadata.label.clone());
                    g
                }
                None => ModelResponse::error_model_not_found(&model_metadata.label),
            };

            let result_tokens = response.get_token_count();

            if request.response_channel.send(response).is_err() {
                event!(
                    Level::WARN,
                    "Unable to send response to {}",
                    request.user_id
                );
            };

            if let Some(result_tokens) = result_tokens {
                if result_tokens > tokens.unwrap_or(0) as u32 {
                    match limiter.tokens(result_tokens - tokens.unwrap_or(0) as u32) {
                        TokenQuotaStatus::Ready(_) => {}
                        TokenQuotaStatus::LimitedUntil(point) => {
                            time::sleep_until(Instant::from_std(point)).await;
                        }
                        TokenQuotaStatus::Oversized => {
                            event!(
                                Level::WARN,
                                "Request by {} to {:?} exceeded maximum request tokens",
                                request.user_id,
                                model_metadata.uuid
                            );
                            if let TokenQuotaStatus::LimitedUntil(point) =
                                limiter.tokens(model_metadata.quota.tokens_per_minute)
                            {
                                time::sleep_until(Instant::from_std(point)).await;
                            }
                        }
                    }
                }
            }

            // TODO: Add usage statistics!
        }
    });

    tx
}

#[derive(Debug)]
pub struct ModelRequestRouter {
    endpoints: Arc<RwLock<HashMap<Uuid, mpsc::Sender<RoutableRequest>>>>,
}

impl ModelRequestRouter {
    #[tracing::instrument(level = "trace")]
    pub fn new() -> Self {
        Self {
            endpoints: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    #[tracing::instrument(level = "debug")]
    pub async fn add_model(&self, model: api::Model) {
        let handler = match model.clone().api {
            ModelAPI::OpenAIChat(inner) => spawn_model_handler(model.clone(), inner),
            ModelAPI::OpenAIEdit(inner) => spawn_model_handler(model.clone(), inner),
            ModelAPI::OpenAICompletion(inner) => spawn_model_handler(model.clone(), inner),
            ModelAPI::OpenAIModeration(inner) => spawn_model_handler(model.clone(), inner),
            ModelAPI::OpenAIEmbedding(inner) => spawn_model_handler(model.clone(), inner),
            ModelAPI::OpenAIImage(inner) => spawn_model_handler(model.clone(), inner),
            ModelAPI::OpenAIAudio(inner) => spawn_model_handler(model.clone(), inner),
        };

        self.endpoints.write().await.insert(model.uuid, handler);
    }

    #[tracing::instrument(level = "debug")]
    pub async fn remove_model(&self, model_id: &Uuid) -> Option<()> {
        self.endpoints.write().await.remove(model_id).map(|_| ())
    }

    #[tracing::instrument(level = "trace")]
    async fn get_endpoint(&self, model_id: &Uuid) -> Option<mpsc::Sender<RoutableRequest>> {
        self.endpoints.read().await.get(model_id).cloned()
    }

    #[tracing::instrument(level = "debug")]
    pub async fn route_request(
        &self,
        model_id: Uuid,
        user_id: Uuid,
        priority: usize,
        request: ModelRequest,
    ) -> ModelResponse {
        let (tx, rx) = oneshot::channel();

        let model_name = request.get_model();
        let routeable = RoutableRequest {
            body: request,
            user_id,
            response_channel: tx,
        };

        match self.get_endpoint(&model_id).await {
            Some(m) => match m.send_timeout(routeable, Duration::new(5, 0)).await {
                Ok(_) => rx.await.unwrap_or(ModelResponse::error_internal()),
                Err(err) => match err {
                    SendTimeoutError::Closed(_) => ModelResponse::error_internal(),
                    SendTimeoutError::Timeout(_) => ModelResponse::error_internal_rate_limit(),
                },
            },
            None => ModelResponse::error_model_not_found(&model_name),
        }
    }
}
