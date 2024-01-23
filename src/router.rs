use std::{collections::HashMap, hash::Hash, num::NonZeroU32, sync::Arc};

use serde::{Deserialize, Serialize};

use uuid::Uuid;

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
use serde_json::value::Value;
use tokio::sync::{mpsc, oneshot, RwLock};

use crate::{
    api::{self},
    openai_client,
};

use governor::{
    clock::{Clock, QuantaClock, QuantaUpkeepClock, SystemClock},
    DefaultDirectRateLimiter, Quota, RateLimiter,
};
//use governor::{Quota, RateLimiter};

struct Limiter {
    requests_per_minute: DefaultDirectRateLimiter,
    requests_per_hour: DefaultDirectRateLimiter,
    tokens_per_minute: DefaultDirectRateLimiter,
    tokens_per_hour: DefaultDirectRateLimiter,
}

impl Limiter {
    pub fn new(quota: api::Quota) -> Self {
        Limiter {
            requests_per_minute: RateLimiter::direct(Quota::per_minute(
                NonZeroU32::new(quota.requests_per_minute).unwrap_or(NonZeroU32::MAX),
            )),
            requests_per_hour: RateLimiter::direct(Quota::per_hour(
                NonZeroU32::new(quota.requests_per_day / 24).unwrap_or(NonZeroU32::MAX),
            )),
            tokens_per_minute: RateLimiter::direct(Quota::per_minute(
                NonZeroU32::new(quota.tokens_per_minute).unwrap_or(NonZeroU32::MAX),
            )),
            tokens_per_hour: RateLimiter::direct(Quota::per_hour(
                NonZeroU32::new(quota.tokens_per_day / 24).unwrap_or(NonZeroU32::MAX),
            )),
        }
    }
}

struct RoutableRequest {
    body: ModelRequest,
    user_id: Uuid,
    response_channel: oneshot::Sender<ModelResponse>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ModelAPI {
    OpenAIChat(openai_client::OpenAIChatModel),
    OpenAIEdit(openai_client::OpenAIEditModel),
    OpenAICompletion(openai_client::OpenAICompletionModel),
    OpenAIModeration(openai_client::OpenAIModerationModel),
    OpenAIEmbedding(openai_client::OpenAIEmbeddingModel),
    OpenAIImage(openai_client::OpenAIImageModel),
    OpenAIAudio(openai_client::OpenAIAudioModel),
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
// Note: Image/Audio inputs must be added manually, as they will not be serialized/deserialized!
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

#[derive(Serialize, Deserialize)]
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
    pub fn get_token_count(&self) -> Option<u32> {
        match self {
            Self::Error(_) => None,
            Self::Chat(r) => r.usage.clone().map(|u| u.total_tokens),
            Self::Edit(r) => Some(r.usage.total_tokens),
            Self::Completion(r) => r.usage.clone().map(|u| u.total_tokens),
            Self::Moderation(_) => None,
            Self::Embedding(r) => Some(r.usage.total_tokens),
            Self::Image(_) => None,
            Self::Transcription(_) => None,
            Self::Translation(_) => None,
        }
    }

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

    pub fn server_error() -> Self {
        Self::Error(ApiError {
            message: "The proxy server had an error processing your request. You can retry your request, or contact the proxy's administrator if you keep seeing this error.".to_string(),
            r#type: Some("server_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        })
    }

    pub fn model_not_found(model: &str) -> Self {
        Self::Error(ApiError {
            message: ["The model `", model, "` does not exist."].concat(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("model_not_found".to_string())),
        })
    }
}

pub struct ModelRequestRouter {
    endpoints: HashMap<Uuid, mpsc::Sender<RoutableRequest>>,
}

impl ModelRequestRouter {
    pub fn new(models: Vec<api::Model>) -> Self {
        let mut endpoints: HashMap<Uuid, mpsc::Sender<RoutableRequest>> = HashMap::new();

        for model in models {
            let (tx, mut rx) = mpsc::unbounded_channel::<RoutableRequest>();

            tokio::spawn(async move {
                match model.api {
                    ModelAPI::OpenAIChat(m) => {
                        let client = m.init_client();
                        let mut encode_buffer = Uuid::encode_buffer();

                        while let Some(request) = rx.recv().await {
                            let user: &mut str =
                                request.user_id.simple().encode_lower(&mut encode_buffer);

                            request.response_channel.send(
                                if let ModelRequest::Chat(r) = request.body {
                                    match m.generate(&client, user, r).await {
                                        Ok(g) => ModelResponse::Chat(g),
                                        Err(g) => ModelResponse::Error(g),
                                    }
                                } else {
                                    ModelResponse::model_not_found(&model.label)
                                },
                            );
                        }
                    }
                    ModelAPI::OpenAIEdit(m) => {
                        let client = m.init_client();

                        while let Some(request) = rx.recv().await {
                            request.response_channel.send(
                                if let ModelRequest::Edit(r) = request.body {
                                    match m.generate(&client, r).await {
                                        Ok(g) => ModelResponse::Edit(g),
                                        Err(g) => ModelResponse::Error(g),
                                    }
                                } else {
                                    ModelResponse::model_not_found(&model.label)
                                },
                            );
                        }
                    }
                    ModelAPI::OpenAICompletion(m) => {
                        let client = m.init_client();
                        let mut encode_buffer = Uuid::encode_buffer();

                        while let Some(request) = rx.recv().await {
                            let user: &mut str =
                                request.user_id.simple().encode_lower(&mut encode_buffer);

                            request.response_channel.send(
                                if let ModelRequest::Completion(r) = request.body {
                                    match m.generate(&client, user, r).await {
                                        Ok(g) => ModelResponse::Completion(g),
                                        Err(g) => ModelResponse::Error(g),
                                    }
                                } else {
                                    ModelResponse::model_not_found(&model.label)
                                },
                            );
                        }
                    }
                    ModelAPI::OpenAIModeration(m) => {
                        let client = m.init_client();

                        while let Some(request) = rx.recv().await {
                            request.response_channel.send(
                                if let ModelRequest::Moderation(r) = request.body {
                                    match m.generate(&client, r).await {
                                        Ok(g) => ModelResponse::Moderation(g),
                                        Err(g) => ModelResponse::Error(g),
                                    }
                                } else {
                                    ModelResponse::model_not_found(&model.label)
                                },
                            );
                        }
                    }
                    ModelAPI::OpenAIEmbedding(m) => {
                        let client = m.init_client();

                        while let Some(request) = rx.recv().await {
                            request.response_channel.send(
                                if let ModelRequest::Embedding(r) = request.body {
                                    match m.generate(&client, r).await {
                                        Ok(g) => ModelResponse::Embedding(g),
                                        Err(g) => ModelResponse::Error(g),
                                    }
                                } else {
                                    ModelResponse::model_not_found(&model.label)
                                },
                            );
                        }
                    }
                    ModelAPI::OpenAIImage(m) => {
                        let client = m.init_client();
                        let mut encode_buffer = Uuid::encode_buffer();

                        while let Some(request) = rx.recv().await {
                            let user: &mut str =
                                request.user_id.simple().encode_lower(&mut encode_buffer);

                            request.response_channel.send(match request.body {
                                ModelRequest::Image(r) => {
                                    match m.generate(&client, user, r).await {
                                        Ok(g) => ModelResponse::Image(g),
                                        Err(g) => ModelResponse::Error(g),
                                    }
                                }
                                ModelRequest::ImageEdit(r) => {
                                    match m.generate_edit(&client, user, r).await {
                                        Ok(g) => ModelResponse::Image(g),
                                        Err(g) => ModelResponse::Error(g),
                                    }
                                }
                                ModelRequest::ImageVariation(r) => {
                                    match m.generate_variation(&client, user, r).await {
                                        Ok(g) => ModelResponse::Image(g),
                                        Err(g) => ModelResponse::Error(g),
                                    }
                                }
                                _ => ModelResponse::model_not_found(&model.label),
                            });
                        }
                    }
                    ModelAPI::OpenAIAudio(m) => {
                        let client = m.init_client();

                        while let Some(request) = rx.recv().await {
                            request.response_channel.send(match request.body {
                                ModelRequest::Transcription(r) => {
                                    match m.generate_transcription(&client, r).await {
                                        Ok(g) => ModelResponse::Transcription(g),
                                        Err(g) => ModelResponse::Error(g),
                                    }
                                }
                                ModelRequest::Translation(r) => {
                                    match m.generate_translation(&client, r).await {
                                        Ok(g) => ModelResponse::Translation(g),
                                        Err(g) => ModelResponse::Error(g),
                                    }
                                }
                                _ => ModelResponse::model_not_found(&model.label),
                            });
                        }
                    }
                };
            });

            let (tx_2, mut rx_2) =
                mpsc::channel::<RoutableRequest>(if model.quota.max_queue_size == 0 {
                    usize::MAX
                } else {
                    model.quota.max_queue_size
                });

            tokio::spawn(async move {
                let mut limiter = Limiter::new(model.quota);

                while let Some(mut request) = rx_2.recv().await {
                    let tx_4 = request.response_channel;
                    let (tx_3, rx_3) = oneshot::channel();
                    request.response_channel = tx_3;

                    // TODO: Add token-based rate limits!

                    limiter.requests_per_minute.until_ready().await;
                    limiter.requests_per_hour.until_ready().await;

                    let response = if tx.send(request).is_err() {
                        ModelResponse::server_error()
                    } else {
                        rx_3.await.unwrap_or(ModelResponse::server_error())
                    };

                    // TODO: Add usage statistics!

                    tx_4.send(response);
                }
            });

            endpoints.insert(model.uuid, tx_2);
        }

        Self { endpoints }
    }

    pub async fn route_request(
        &self,
        model_id: Uuid,
        user_id: Uuid,
        priority: usize,
        request: ModelRequest,
    ) -> ModelResponse {
        let (tx, rx) = oneshot::channel();

        let routeable = RoutableRequest {
            body: request.clone(),
            user_id,
            response_channel: tx,
        };

        let model_name = request.get_model();

        let mut response = match self.endpoints.get(&model_id) {
            Some(m) => {
                m.try_send(routeable).unwrap();
                rx.await.unwrap_or(ModelResponse::server_error())
            }
            None => ModelResponse::model_not_found(&model_name),
        };

        response.replace_model_id(model_name);

        response
    }
}
