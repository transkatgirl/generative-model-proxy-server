use std::{any::Any, fmt::Debug, future::Future, sync::Arc};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tracing::{event, Level};

mod openai;

pub(super) trait CallableModelAPI:
    Send + Sync + Debug + Serialize + DeserializeOwned + 'static
{
    type Client: Send + Sync;
    type ModelRequest: RoutableModelRequest;
    type ModelResponse: RoutableModelResponse;
    type ModelError: RoutableModelResponse;

    fn init(&self) -> Self::Client;

    fn get_context_len(&self) -> Option<u32>;

    fn to_request(&self, request: impl RoutableModelRequest) -> Option<Self::ModelRequest> {
        let item_any: Box<dyn Any> = Box::new(request);

        item_any.downcast::<Self::ModelRequest>().map(|d| *d).ok()
    }

    fn generate(
        &self,
        client: &Self::Client,
        user: &str,
        label: &str,
        request: Self::ModelRequest,
    ) -> impl Future<Output = Result<Self::ModelResponse, Self::ModelError>> + Send;
}

pub(super) trait RoutableModelRequest: Send + Debug + DeserializeOwned + 'static {
    fn get_model(&self) -> String;

    fn get_total_n(&self) -> u32;
}

#[derive(Serialize, Debug)]
pub(super) enum ResponseStatus {
    Success,
    InvalidRequest,
    ModelNotFound,
    InternalError,
    BadUpstream,
    ModelUnavailable,
}

pub(super) trait RoutableModelResponse:
    Send + Debug + Serialize + Into<ResponseStatus> + 'static
{
    fn get_token_count(&self) -> Option<u32>;
}

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

pub(super) struct PackagedRequest {
    body: ModelRequest,
    user: String,
    response_channel: oneshot::Sender<ModelResponse>,
}

impl ModelAPI {
    pub(super) fn spawn_model_handler(self, label: &str) -> mpsc::UnboundedSender<PackagedRequest> {
        let (tx, rx) = mpsc::unbounded_channel::<PackagedRequest>();

        match self {
            ModelAPI::OpenAIChat(model) => spawn_model_handler_task(model, label.into(), rx),
            ModelAPI::OpenAIEdit(model) => spawn_model_handler_task(model, label.into(), rx),
            ModelAPI::OpenAICompletion(model) => spawn_model_handler_task(model, label.into(), rx),
            ModelAPI::OpenAIModeration(model) => spawn_model_handler_task(model, label.into(), rx),
            ModelAPI::OpenAIEmbedding(model) => spawn_model_handler_task(model, label.into(), rx),
            ModelAPI::OpenAIImage(model) => spawn_model_handler_task(model, label.into(), rx),
            ModelAPI::OpenAIAudio(model) => spawn_model_handler_task(model, label.into(), rx),
        };

        tx
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
    fn from(item: impl RoutableModelResponse) -> Self {
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

        if item_any.is::<openai::ApiError>() {
            return ModelResponse::OpenAIError(*item_any.downcast::<openai::ApiError>().unwrap());
        }

        panic!()
    }
}

impl RoutableModelResponse for ModelResponse {
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

#[allow(clippy::from_over_into)]
impl Into<ResponseStatus> for ModelResponse {
    fn into(self) -> ResponseStatus {
        match self {
            Self::OpenAIChat(r) => r.into(),
            Self::OpenAIEdit(r) => r.into(),
            Self::OpenAICompletion(r) => r.into(),
            Self::OpenAIModeration(r) => r.into(),
            Self::OpenAIEmbedding(r) => r.into(),
            Self::OpenAIImage(r) => r.into(),
            Self::OpenAIAudio(r) => r.into(),
            Self::OpenAIError(r) => r.into(),
            Self::NoAPI(status) => status,
        }
    }
}

#[tracing::instrument(level = "debug")]
fn spawn_model_handler_task<M: CallableModelAPI>(
    model: Arc<M>,
    label: Arc<str>,
    mut channel: mpsc::UnboundedReceiver<PackagedRequest>,
) {
    tokio::spawn(async move {
        let client = Arc::new(model.init());

        while let Some(request) = channel.recv().await {
            let model_request = match model.to_request(request.body) {
                Some(model_request) => model_request,
                None => {
                    if request
                        .response_channel
                        .send(ModelResponse::NoAPI(ResponseStatus::InternalError))
                        .is_err()
                    {
                        event!(Level::WARN, "Unable to send response to {}", request.user);
                    };
                    continue;
                }
            };

            let model = model.clone();
            let client = client.clone();
            let label = label.clone();
            tokio::spawn(async move {
                let result = match model
                    .generate(&client, &request.user, &label, model_request)
                    .await
                {
                    Ok(r) => ModelResponse::from(r),
                    Err(r) => ModelResponse::from(r),
                };

                if request.response_channel.send(result).is_err() {
                    event!(Level::WARN, "Unable to send response to {}", request.user);
                };
            });
        }
    });
}
