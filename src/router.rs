use std::{collections::HashMap, hash::Hash, num::NonZeroU32, sync::Arc, future::Future};

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
use tokio::sync::{mpsc::{self, UnboundedReceiver}, oneshot, RwLock};

use crate::{
    api::{self},
    openai_client,
};

use governor::{
    clock::{Clock, QuantaClock, QuantaUpkeepClock, SystemClock},
    middleware::{StateInformationMiddleware, StateSnapshot},
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
            )), /*.with_middleware::<StateInformationMiddleware>();*/
            tokens_per_hour: RateLimiter::direct(Quota::per_hour(
                NonZeroU32::new(quota.tokens_per_day / 24).unwrap_or(NonZeroU32::MAX),
            )), /*.with_middleware::<StateInformationMiddleware>();*/
        }
    }
}

struct RoutableRequest {
    body: ModelRequest,
    user_id: Uuid,
    response_channel: oneshot::Sender<ModelResponse>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
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

    /*pub fn get_input_token_count(&self, model: &api::Model) -> Option<u32> {

    }

    pub fn get_max_tokens(&self, model: &api::Model) -> Option<u32> {

    }*/
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

    // Status code 400
    pub fn error_failed_parse() -> Self {
        Self::Error(ApiError {
            message: "We could not parse the JSON body of your request. (HINT: This likely means you aren't using your HTTP library correctly. The OpenAI API expects a JSON payload, but what was sent was not valid JSON. If you have trouble figuring out how to fix this, please contact the proxy's administrator.)".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        })
    }

    // Status code 401
    pub fn error_auth_missing() -> Self {
        Self::Error(ApiError {
            message: "You didn't provide an API key. You need to provide your API key in an Authorization header using Bearer auth (i.e. Authorization: Bearer YOUR_KEY), or as the password field (with blank username) if you're accessing the API from your browser and are prompted for a username and password. You can obtain an API key from the proxy's administrator.".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        })
    }

    // Status code 401
    pub fn error_auth_incorrect() -> Self {
        Self::Error(ApiError {
            message: "Incorrect API key provided. You can obtain an API key from the proxy's administrator.".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("invalid_api_key".to_string())),
        })
    }

    // Status code 404
    pub fn error_not_found(model: &str) -> Self {
        Self::Error(ApiError {
            message: ["The model `", model, "` does not exist."].concat(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("model_not_found".to_string())),
        })
    }

    // Status code 429
    pub fn error_user_rate_limit() -> Self {
        Self::Error(ApiError {
            message: "You exceeded your current quota, please check your API key's rate limits. For more information on this error, contact the proxy's administrator.".to_string(),
            r#type: Some("insufficient_quota".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("insufficient_quota".to_string())),
        })
    }

    // Status code 500
    pub fn error_internal() -> Self {
        Self::Error(ApiError {
            message: "The proxy server had an error processing your request. Sorry about that! You can retry your request, or contact the proxy's administrator if the error persists.".to_string(),
            r#type: Some("server_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        })
    }

    // Status code 503
    pub fn error_internal_rate_limit() -> Self {
        Self::Error(ApiError {
            message: "That model is currently overloaded with other requests. You can retry your request, or contact the proxy's administrator if the error persists.".to_string(),
            r#type: Some("server_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        })
    }

    // add get_status_code()
}

fn spawn_model_handler(model_metadata: api::Model, model_callable: impl ModelAPICallable + Send + 'static) -> mpsc::Sender<RoutableRequest>
{
    let (tx, mut rx) = mpsc::channel::<RoutableRequest>(if model_metadata.quota.max_queue_size == 0 {
        usize::MAX
    } else {
        model_metadata.quota.max_queue_size
    });

    tokio::spawn(async move {
        let client = model_callable.init();
        let limiter: Limiter = Limiter::new(model_metadata.quota);
        let mut encode_buffer: [u8; 45] = Uuid::encode_buffer();

        while let Some(request) = rx.recv().await {
            let user: &mut str =
                request.user_id.simple().encode_lower(&mut encode_buffer);

            limiter.requests_per_minute.until_ready().await;
            limiter.requests_per_hour.until_ready().await;

            // TODO: Add token-based rate limits!

            request.response_channel.send(match model_callable.generate(&client, user, request.body).await {
                Some(mut g) => {
                    g.replace_model_id(model_metadata.label.clone());
                    g
                }
                None => ModelResponse::error_not_found(&model_metadata.label),
            });

            // TODO: Add usage statistics!
        }
    });

    tx
}

pub struct ModelRequestRouter {
    endpoints: HashMap<Uuid, mpsc::Sender<RoutableRequest>>,
}

impl ModelRequestRouter {
    pub fn new(models: Vec<api::Model>) -> Self {
        let mut endpoints: HashMap<Uuid, mpsc::Sender<RoutableRequest>> = HashMap::new();

        for model in models {
            endpoints.insert(
                model.uuid,
                match model.clone().api {
                ModelAPI::OpenAIChat(inner) => spawn_model_handler(model.clone(), inner),
                ModelAPI::OpenAIEdit(inner) => spawn_model_handler(model.clone(), inner),
                ModelAPI::OpenAICompletion(inner) => spawn_model_handler(model.clone(), inner),
                ModelAPI::OpenAIModeration(inner) => spawn_model_handler(model.clone(), inner),
                ModelAPI::OpenAIEmbedding(inner) => spawn_model_handler(model.clone(), inner),
                ModelAPI::OpenAIImage(inner) => spawn_model_handler(model.clone(), inner),
                ModelAPI::OpenAIAudio(inner) => spawn_model_handler(model.clone(), inner),
            });
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
                rx.await.unwrap_or(ModelResponse::error_internal())
            }
            None => ModelResponse::error_not_found(&model_name),
        };

        response.replace_model_id(model_name);

        response
    }
}
