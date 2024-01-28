use std::{any::Any, collections::HashMap, fmt::Debug, future::Future, sync::Arc};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::{
    sync::{
        mpsc::{self, error::SendTimeoutError},
        oneshot, RwLock,
    },
    time::Duration,
};
use tracing::{event, Level};
use uuid::Uuid;

use self::limiter::Limiter;
use crate::api;

mod limiter;
mod openai;
mod tokenizer;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)]
pub enum ModelAPI {
    OpenAIChat(openai::OpenAIChatModel),
    OpenAIEdit(openai::OpenAIEditModel),
    OpenAICompletion(openai::OpenAICompletionModel),
    OpenAIModeration(openai::OpenAIModerationModel),
    OpenAIEmbedding(openai::OpenAIEmbeddingModel),
    OpenAIImage(openai::OpenAIImageModel),
    OpenAIAudio(openai::OpenAIAudioModel),
}

trait CallableModelAPI: Send + Sync + Debug + Serialize + DeserializeOwned {
    type Client: Send + Sync;
    type ModelRequest: RoutableModelRequest;
    type ModelResponse: RoutableModelResponse;
    type ModelError: RoutableModelError;

    fn init(&self) -> Self::Client;

    fn to_request(
        &self,
        request: impl RoutableModelRequest + 'static,
    ) -> Option<Self::ModelRequest>;

    fn to_response(&self, error_code: ModelErrorCode) -> Self::ModelError;

    fn generate(
        &self,
        client: &Self::Client,
        user: &str,
        request: Self::ModelRequest,
    ) -> impl Future<Output = Result<Self::ModelResponse, Self::ModelError>> + Send;
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant, clippy::enum_variant_names)]
pub enum ModelRequest {
    OpenAIChat(openai::CreateChatCompletionRequest),
    OpenAIEdit(openai::CreateEditRequest),
    OpenAICompletion(openai::CreateCompletionRequest),
    OpenAIModeration(openai::CreateModerationRequest),
    OpenAIEmbedding(openai::CreateEmbeddingRequest),
    OpenAIImage(openai::ImagesRequest),
    OpenAIAudio(openai::AudioRequest),
}

trait RoutableModelRequest: Send + Debug + DeserializeOwned {
    fn get_model(&self) -> String;

    fn get_token_count(&self, model: &api::Model) -> Option<u32>;

    fn get_max_tokens(&self, model: &api::Model) -> Option<u32>;
}

impl RoutableModelRequest for ModelRequest {
    fn get_model(&self) -> String {
        match self {
            Self::OpenAIChat(r) => r.get_model(),
            Self::OpenAIEdit(r) => r.get_model(),
            Self::OpenAICompletion(r) => r.get_model(),
            Self::OpenAIModeration(r) => r.get_model(),
            Self::OpenAIEmbedding(r) => r.get_model(),
            Self::OpenAIImage(r) => r.get_model(),
            Self::OpenAIAudio(r) => r.get_model(),
        }
    }

    fn get_token_count(&self, model: &api::Model) -> Option<u32> {
        match self {
            Self::OpenAIChat(r) => r.get_token_count(model),
            Self::OpenAIEdit(r) => r.get_token_count(model),
            Self::OpenAICompletion(r) => r.get_token_count(model),
            Self::OpenAIModeration(r) => r.get_token_count(model),
            Self::OpenAIEmbedding(r) => r.get_token_count(model),
            Self::OpenAIImage(r) => r.get_token_count(model),
            Self::OpenAIAudio(r) => r.get_token_count(model),
        }
    }

    fn get_max_tokens(&self, model: &api::Model) -> Option<u32> {
        match self {
            Self::OpenAIChat(r) => r.get_max_tokens(model),
            Self::OpenAIEdit(r) => r.get_max_tokens(model),
            Self::OpenAICompletion(r) => r.get_max_tokens(model),
            Self::OpenAIModeration(r) => r.get_max_tokens(model),
            Self::OpenAIEmbedding(r) => r.get_max_tokens(model),
            Self::OpenAIImage(r) => r.get_max_tokens(model),
            Self::OpenAIAudio(r) => r.get_max_tokens(model),
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant, clippy::enum_variant_names)]
pub enum ModelResponse {
    OpenAIChat(openai::CreateChatCompletionResponse),
    OpenAIEdit(openai::CreateEditResponse),
    OpenAICompletion(openai::CreateCompletionResponse),
    OpenAIModeration(openai::CreateModerationResponse),
    OpenAIEmbedding(openai::CreateEmbeddingResponse),
    OpenAIImage(openai::ImagesResponse),
    OpenAIAudio(openai::AudioResponse),
}

trait RoutableModelResponse: Send + Debug + Serialize {
    fn replace_model_id(&mut self, model_id: String);

    fn get_token_count(&self) -> Option<u32>;
}

impl RoutableModelResponse for ModelResponse {
    fn replace_model_id(&mut self, model_id: String) {
        match self {
            Self::OpenAIChat(r) => r.replace_model_id(model_id),
            Self::OpenAIEdit(r) => r.replace_model_id(model_id),
            Self::OpenAICompletion(r) => r.replace_model_id(model_id),
            Self::OpenAIModeration(r) => r.replace_model_id(model_id),
            Self::OpenAIEmbedding(r) => r.replace_model_id(model_id),
            Self::OpenAIImage(r) => r.replace_model_id(model_id),
            Self::OpenAIAudio(r) => r.replace_model_id(model_id),
        }
    }

    fn get_token_count(&self) -> Option<u32> {
        match self {
            Self::OpenAIChat(r) => r.get_token_count(),
            Self::OpenAIEdit(r) => r.get_token_count(),
            Self::OpenAICompletion(r) => r.get_token_count(),
            Self::OpenAIModeration(r) => r.get_token_count(),
            Self::OpenAIEmbedding(r) => r.get_token_count(),
            Self::OpenAIImage(r) => r.get_token_count(),
            Self::OpenAIAudio(r) => r.get_token_count(),
        }
    }
}

impl ModelResponse {
    fn from(item: impl RoutableModelResponse + 'static) -> Self {
        let item_any: Box<dyn Any> = Box::new(item);

        if item_any.is::<openai::CreateChatCompletionResponse>() {
            return ModelResponse::OpenAIChat(
                *item_any
                    .downcast::<openai::CreateChatCompletionResponse>()
                    .unwrap(),
            );
        }

        if item_any.is::<openai::CreateEditResponse>() {
            return ModelResponse::OpenAIEdit(
                *item_any.downcast::<openai::CreateEditResponse>().unwrap(),
            );
        }

        if item_any.is::<openai::CreateCompletionResponse>() {
            return ModelResponse::OpenAICompletion(
                *item_any
                    .downcast::<openai::CreateCompletionResponse>()
                    .unwrap(),
            );
        }

        if item_any.is::<openai::CreateModerationResponse>() {
            return ModelResponse::OpenAIModeration(
                *item_any
                    .downcast::<openai::CreateModerationResponse>()
                    .unwrap(),
            );
        }

        if item_any.is::<openai::CreateEmbeddingResponse>() {
            return ModelResponse::OpenAIEmbedding(
                *item_any
                    .downcast::<openai::CreateEmbeddingResponse>()
                    .unwrap(),
            );
        }

        if item_any.is::<openai::ImagesResponse>() {
            return ModelResponse::OpenAIImage(
                *item_any.downcast::<openai::ImagesResponse>().unwrap(),
            );
        }

        if item_any.is::<openai::AudioResponse>() {
            return ModelResponse::OpenAIAudio(
                *item_any.downcast::<openai::AudioResponse>().unwrap(),
            );
        }

        panic!()
    }
}

#[derive(Serialize, Clone, Copy, Debug)]
pub enum ModelErrorCode {
    FailedParse,
    PromptTooLong,
    AuthMissing,
    AuthIncorrect,
    ModelNotFound,
    EndpointNotFound,
    RateLimitUser,
    RateLimitModel,
    InternalError,
    OtherModelError,
}

#[derive(Serialize, Debug)]
#[serde(untagged)]
pub enum ModelError {
    OpenAIError(openai::ApiError),
    NoAPI(ModelErrorCode),
}

trait RoutableModelError: RoutableModelResponse {
    fn get_error_code(&self) -> ModelErrorCode;
}

impl RoutableModelResponse for ModelError {
    fn replace_model_id(&mut self, model_id: String) {
        match self {
            Self::OpenAIError(r) => r.replace_model_id(model_id),
            Self::NoAPI(_) => {}
        }
    }

    fn get_token_count(&self) -> Option<u32> {
        match self {
            Self::OpenAIError(r) => r.get_token_count(),
            Self::NoAPI(_) => None,
        }
    }
}

impl RoutableModelError for ModelError {
    fn get_error_code(&self) -> ModelErrorCode {
        match self {
            Self::OpenAIError(e) => e.get_error_code(),
            Self::NoAPI(e) => *e,
        }
    }
}

impl ModelError {
    fn from(item: impl RoutableModelError + 'static) -> Self {
        let item_any: Box<dyn Any> = Box::new(item);

        if item_any.is::<openai::ApiError>() {
            return ModelError::OpenAIError(*item_any.downcast::<openai::ApiError>().unwrap());
        }

        if item_any.is::<ModelErrorCode>() {
            return ModelError::NoAPI(*item_any.downcast::<ModelErrorCode>().unwrap());
        }

        panic!()
    }
}

struct RoutableRequest {
    body: ModelRequest,
    user_id: Uuid,
    response_channel: oneshot::Sender<Result<ModelResponse, ModelError>>,
}

// TODO: Add proxy for image URLs (GPT-4 input, image model output)
#[tracing::instrument(level = "trace")]
fn spawn_model_handler(
    model_metadata: api::Model,
    model_callable: impl CallableModelAPI + 'static,
) -> mpsc::Sender<RoutableRequest> {
    let (tx, mut rx) =
        mpsc::channel::<RoutableRequest>(if model_metadata.quota.max_queue_size == 0 {
            64
        } else {
            model_metadata.quota.max_queue_size
        });

    tokio::spawn(async move {
        let client = Arc::new(model_callable.init());
        let limiter = Arc::new(Limiter::new(model_metadata.quota));
        let mut encode_buffer: [u8; 45] = Uuid::encode_buffer();
        let model_callable = Arc::new(model_callable);
        let model_metadata = Arc::new(model_metadata);

        while let Some(request) = rx.recv().await {
            let user: &mut str = request.user_id.simple().encode_lower(&mut encode_buffer);

            let handle = match limiter
                .wait_model_request(&model_metadata, &request.body)
                .await
            {
                Ok(h) => h,
                Err(_) => {
                    if request
                        .response_channel
                        .send(Err(ModelError::from(
                            model_callable.to_response(ModelErrorCode::RateLimitUser),
                        )))
                        .is_err()
                    {
                        event!(
                            Level::WARN,
                            "Unable to send response to {}",
                            request.user_id
                        );
                    };
                    continue;
                }
            };

            let user = user.to_string();
            let limiter = limiter.clone();
            let client = client.clone();
            let model_callable = model_callable.clone();
            let model_metadata = model_metadata.clone();
            tokio::spawn(async move {
                let response = match model_callable.to_request(request.body) {
                    Some(request) => model_callable.generate(&client, &user, request).await,
                    None => Err(model_callable.to_response(ModelErrorCode::ModelNotFound)),
                }
                .map(|mut response| {
                    response.replace_model_id(model_metadata.label.clone());
                    ModelResponse::from(response)
                })
                .map_err(ModelError::from);

                if match &response {
                    Ok(r) => limiter.model_response(handle, r).await,
                    Err(e) => limiter.model_response(handle, e).await,
                }
                .is_err()
                {
                    event!(
                        Level::WARN,
                        "Request by {} to {:?} exceeded maximum request tokens",
                        request.user_id,
                        model_metadata.uuid
                    );
                }

                if request.response_channel.send(response).is_err() {
                    event!(
                        Level::WARN,
                        "Unable to send response to {}",
                        request.user_id
                    );
                };
            });

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
    ) -> Result<ModelResponse, ModelError> {
        let (tx, rx) = oneshot::channel();

        let routeable = RoutableRequest {
            body: request,
            user_id,
            response_channel: tx,
        };

        match self.get_endpoint(&model_id).await {
            Some(m) => match m.send_timeout(routeable, Duration::new(5, 0)).await {
                Ok(_) => rx
                    .await
                    .unwrap_or(Err(ModelError::NoAPI(ModelErrorCode::InternalError))),
                Err(err) => match err {
                    SendTimeoutError::Closed(_) => {
                        Err(ModelError::NoAPI(ModelErrorCode::InternalError))
                    }
                    SendTimeoutError::Timeout(_) => {
                        Err(ModelError::NoAPI(ModelErrorCode::RateLimitModel))
                    }
                },
            },
            None => Err(ModelError::NoAPI(ModelErrorCode::ModelNotFound)),
        }
    }
}
