use std::{any::Any, fmt::Debug, future::Future, sync::Arc};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

mod openai;
pub(super) trait CallableModelAPI:
    Send + Sync + Debug + Serialize + DeserializeOwned + 'static
{
    type Client: Send + Sync;
    type ModelRequest: RoutableModelRequest + DeserializeOwned;
    type ModelResponse: RoutableModelResponse + Serialize;
    type ModelError: RoutableModelResponse + Serialize;

    fn init(&self) -> Self::Client;

    fn get_context_len(&self) -> Option<u32>;

    fn generate(
        &self,
        client: &Self::Client,
        request_label: &str,
        model_label: &str,
        request: Self::ModelRequest,
    ) -> impl Future<Output = Result<Self::ModelResponse, Self::ModelError>> + Send;
}

pub(super) trait RoutableModelRequest: Send + Debug + 'static {
    fn get_model(&self) -> String;

    fn get_total_n(&self) -> u32;
}

#[derive(Serialize, Debug, Clone, Copy)]
pub(super) enum ResponseStatus {
    Success,
    InvalidRequest,
    InternalError,
    BadUpstream,
    ModelUnavailable,
}

pub(super) trait RoutableModelResponse: Send + Debug + 'static {
    fn get_status(&self) -> ResponseStatus;

    fn get_token_count(&self) -> Option<u32>;
}

impl RoutableModelResponse for ResponseStatus {
    fn get_status(&self) -> ResponseStatus {
        *self
    }

    fn get_token_count(&self) -> Option<u32> {
        None
    }
}

// TODO: Add proxy for image URLs (GPT-4 input, image model output)?

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
#[allow(private_interfaces, clippy::enum_variant_names)]
pub(super) enum ModelAPI {
    OpenAIChat(Arc<openai::OpenAIChatModel>),
    OpenAIEdit(Arc<openai::OpenAIEditModel>),
    OpenAICompletion(Arc<openai::OpenAICompletionModel>),
    OpenAIModeration(Arc<openai::OpenAIModerationModel>),
    OpenAIEmbedding(Arc<openai::OpenAIEmbeddingModel>),
    OpenAIImage(Arc<openai::OpenAIImageModel>),
    OpenAIAudio(Arc<openai::OpenAIAudioModel>),
}

#[derive(Debug)]
struct PackagedRequest {
    body: ModelRequest,
    request_label: Arc<str>,
    model_label: Arc<str>,
    response_channel: oneshot::Sender<ModelResponse>,
}

#[derive(Debug)]
pub(super) struct ModelAPIClient {
    sender: mpsc::UnboundedSender<PackagedRequest>,
}

impl CallableModelAPI for ModelAPI {
    type Client = ModelAPIClient;
    type ModelRequest = ModelRequest;
    type ModelResponse = ModelResponse;
    type ModelError = ResponseStatus;

    fn init(&self) -> Self::Client {
        let (tx, rx) = mpsc::unbounded_channel::<PackagedRequest>();

        match self.clone() {
            ModelAPI::OpenAIChat(model) => spawn_model_handler_task(model, rx),
            ModelAPI::OpenAIEdit(model) => spawn_model_handler_task(model, rx),
            ModelAPI::OpenAICompletion(model) => spawn_model_handler_task(model, rx),
            ModelAPI::OpenAIModeration(model) => spawn_model_handler_task(model, rx),
            ModelAPI::OpenAIEmbedding(model) => spawn_model_handler_task(model, rx),
            ModelAPI::OpenAIImage(model) => spawn_model_handler_task(model, rx),
            ModelAPI::OpenAIAudio(model) => spawn_model_handler_task(model, rx),
        };

        ModelAPIClient { sender: tx }
    }

    fn get_context_len(&self) -> Option<u32> {
        match self.clone() {
            ModelAPI::OpenAIChat(m) => m.get_context_len(),
            ModelAPI::OpenAIEdit(m) => m.get_context_len(),
            ModelAPI::OpenAICompletion(m) => m.get_context_len(),
            ModelAPI::OpenAIModeration(m) => m.get_context_len(),
            ModelAPI::OpenAIEmbedding(m) => m.get_context_len(),
            ModelAPI::OpenAIImage(m) => m.get_context_len(),
            ModelAPI::OpenAIAudio(m) => m.get_context_len(),
        }
    }

    #[tracing::instrument(level = "debug")]
    async fn generate(
        &self,
        client: &Self::Client,
        request_label: &str,
        model_label: &str,
        request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        let (tx, rx) = oneshot::channel();

        let packaged = PackagedRequest {
            body: request,
            request_label: request_label.into(),
            model_label: model_label.into(),
            response_channel: tx,
        };

        if client.sender.send(packaged).is_err() {
            tracing::warn!("Unable to send response to {}", model_label);
            return Err(ResponseStatus::InternalError);
        }

        rx.await.map_err(|_| {
            tracing::warn!("Unable to receive response from {}", model_label);
            ResponseStatus::InternalError
        })
    }
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
#[allow(
    private_interfaces,
    clippy::large_enum_variant,
    clippy::enum_variant_names
)]
pub(super) enum ModelRequest {
    OpenAIChat(openai::CreateChatCompletionRequest),
    OpenAIEdit(openai::CreateEditRequest),
    OpenAICompletion(openai::CreateCompletionRequest),
    OpenAIModeration(openai::CreateModerationRequest),
    OpenAIEmbedding(openai::CreateEmbeddingRequest),
    OpenAIImage(openai::ImagesRequest),
    OpenAIAudio(openai::AudioRequest),
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

    fn get_total_n(&self) -> u32 {
        match self {
            Self::OpenAIChat(r) => r.get_total_n(),
            Self::OpenAIEdit(r) => r.get_total_n(),
            Self::OpenAICompletion(r) => r.get_total_n(),
            Self::OpenAIModeration(r) => r.get_total_n(),
            Self::OpenAIEmbedding(r) => r.get_total_n(),
            Self::OpenAIImage(r) => r.get_total_n(),
            Self::OpenAIAudio(r) => r.get_total_n(),
        }
    }
}

impl ModelRequest {
    fn into_any(self) -> Box<dyn Any> {
        match self {
            Self::OpenAIChat(r) => Box::new(r),
            Self::OpenAIEdit(r) => Box::new(r),
            Self::OpenAICompletion(r) => Box::new(r),
            Self::OpenAIModeration(r) => Box::new(r),
            Self::OpenAIEmbedding(r) => Box::new(r),
            Self::OpenAIImage(r) => Box::new(r),
            Self::OpenAIAudio(r) => Box::new(r),
        }
    }
}

#[derive(Serialize, Debug)]
#[serde(untagged)]
#[allow(private_interfaces, clippy::large_enum_variant)]
pub(super) enum ModelResponse {
    OpenAIChat(openai::CreateChatCompletionResponse),
    OpenAIEdit(openai::CreateEditResponse),
    OpenAICompletion(openai::CreateCompletionResponse),
    OpenAIModeration(openai::CreateModerationResponse),
    OpenAIEmbedding(openai::CreateEmbeddingResponse),
    OpenAIImage(openai::ImagesResponse),
    OpenAIAudio(openai::AudioResponse),
    OpenAIError(openai::ApiError),
    NoAPI(ResponseStatus),
}

impl ModelResponse {
    // TODO: Implement this with a macro
    fn from_any(value: Box<dyn Any>) -> Self {
        if value.is::<openai::CreateChatCompletionResponse>() {
            return ModelResponse::OpenAIChat(
                *value
                    .downcast::<openai::CreateChatCompletionResponse>()
                    .unwrap(),
            );
        }

        if value.is::<openai::CreateEditResponse>() {
            return ModelResponse::OpenAIEdit(
                *value.downcast::<openai::CreateEditResponse>().unwrap(),
            );
        }

        if value.is::<openai::CreateCompletionResponse>() {
            return ModelResponse::OpenAICompletion(
                *value
                    .downcast::<openai::CreateCompletionResponse>()
                    .unwrap(),
            );
        }

        if value.is::<openai::CreateModerationResponse>() {
            return ModelResponse::OpenAIModeration(
                *value
                    .downcast::<openai::CreateModerationResponse>()
                    .unwrap(),
            );
        }

        if value.is::<openai::CreateEmbeddingResponse>() {
            return ModelResponse::OpenAIEmbedding(
                *value.downcast::<openai::CreateEmbeddingResponse>().unwrap(),
            );
        }

        if value.is::<openai::ImagesResponse>() {
            return ModelResponse::OpenAIImage(
                *value.downcast::<openai::ImagesResponse>().unwrap(),
            );
        }

        if value.is::<openai::AudioResponse>() {
            return ModelResponse::OpenAIAudio(*value.downcast::<openai::AudioResponse>().unwrap());
        }

        if value.is::<openai::ApiError>() {
            return ModelResponse::OpenAIError(*value.downcast::<openai::ApiError>().unwrap());
        }

        panic!()
    }
}

impl RoutableModelResponse for ModelResponse {
    fn get_status(&self) -> ResponseStatus {
        match self {
            Self::OpenAIChat(r) => r.get_status(),
            Self::OpenAIEdit(r) => r.get_status(),
            Self::OpenAICompletion(r) => r.get_status(),
            Self::OpenAIModeration(r) => r.get_status(),
            Self::OpenAIEmbedding(r) => r.get_status(),
            Self::OpenAIImage(r) => r.get_status(),
            Self::OpenAIAudio(r) => r.get_status(),
            Self::OpenAIError(r) => r.get_status(),
            Self::NoAPI(status) => *status,
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
            Self::OpenAIError(r) => r.get_token_count(),
            Self::NoAPI(_) => None,
        }
    }
}

#[tracing::instrument(level = "debug")]
fn spawn_model_handler_task<M: CallableModelAPI>(
    model: Arc<M>,
    mut channel: mpsc::UnboundedReceiver<PackagedRequest>,
) {
    tokio::spawn(async move {
        let client = Arc::new(model.init());

        while let Some(request) = channel.recv().await {
            let model_request = match request.body.into_any().downcast::<M::ModelRequest>() {
                Ok(model_request) => *model_request,
                Err(_) => {
                    tracing::warn!("Unable to convert ModelRequest!");
                    if request
                        .response_channel
                        .send(ModelResponse::NoAPI(ResponseStatus::InternalError))
                        .is_err()
                    {
                        tracing::warn!("Unable to send response to {}", request.request_label);
                    };
                    continue;
                }
            };

            let model = model.clone();
            let client = client.clone();
            tokio::spawn(async move {
                let result: Box<dyn Any> = match model
                    .generate(
                        &client,
                        &request.request_label,
                        &request.model_label,
                        model_request,
                    )
                    .await
                {
                    Ok(r) => Box::new(r),
                    Err(r) => Box::new(r),
                };

                if request
                    .response_channel
                    .send(ModelResponse::from_any(result))
                    .is_err()
                {
                    tracing::warn!("Unable to send response to {}", request.request_label);
                };
            });
        }
    });
}
