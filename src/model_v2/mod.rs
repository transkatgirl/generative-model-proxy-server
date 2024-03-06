use std::{error::Error, fmt::Debug, future::Future, iter::Iterator};

use reqwest::StatusCode;
use uuid::Uuid;

mod openai;

pub(super) trait RoutableModel: Send + Sync + Debug + 'static {
    fn get_label(&self) -> &str;

    fn update_label(&mut self, label: String);

    fn get_max_tokens(&self) -> u32;
}

pub(super) trait TextBasedModel: RoutableModel {
    fn get_tokenizer(&self) -> &str;
}

pub(super) trait ChatModel: TextBasedModel {
    type Client: Send + Sync;
    type ModelRequest: ChatModelRequest
        + TryFrom<Box<dyn ChatModelRequest<Response = Box<dyn ChatModelResponse>>>>;
    type ModelResponse: ChatModelResponse;
    type ModelError: RoutableModelError;

    fn init(&self) -> Self::Client;

    fn generate(
        &self,
        client: Self::Client,
        tag: Option<Uuid>,
        request: Self::ModelRequest,
    ) -> impl Future<Output = Result<Self::ModelResponse, Self::ModelError>> + Send;
}

pub(super) trait CompletionModel: TextBasedModel {
    type Client: Send + Sync;
    type ModelRequest: CompletionModelRequest
        + TryFrom<Box<dyn CompletionModelRequest<Response = Box<dyn CompletionModelResponse>>>>;
    type ModelResponse: CompletionModelResponse;
    type ModelError: RoutableModelError;

    fn init(&self) -> Self::Client;

    fn generate(
        &self,
        client: Self::Client,
        tag: Option<Uuid>,
        request: Self::ModelRequest,
    ) -> impl Future<Output = Result<Self::ModelResponse, Self::ModelError>> + Send;
}

pub(super) trait EmbeddingModel: TextBasedModel {
    type Client: Send + Sync;
    type ModelRequest: EmbeddingModelRequest
        + TryFrom<Box<dyn EmbeddingModelRequest<Response = Box<dyn EmbeddingModelResponse>>>>;
    type ModelResponse: EmbeddingModelResponse;
    type ModelError: RoutableModelError;

    fn init(&self) -> Self::Client;

    fn generate(
        &self,
        client: Self::Client,
        tag: Option<Uuid>,
        request: Self::ModelRequest,
    ) -> impl Future<Output = Result<Self::ModelResponse, Self::ModelError>> + Send;
}

pub(super) trait RoutableModelRequest: Send + Debug + 'static {
    fn get_model(&self) -> &str;

    fn get_count(&self) -> u32;

    fn get_max_tokens(&self) -> Option<u32>;
}

pub(super) trait GenerativeTextModelRequest: RoutableModelRequest {
    fn get_n(&self) -> Option<u16>;

    fn get_best_of(&self) -> Option<u16>;

    fn get_seed(&self) -> Option<i64>;

    fn get_suffix(&self) -> Option<&str>;

    fn get_stop_sequences<'s>(&'s self) -> Option<Box<dyn Iterator<Item = &'s str> + 's>>;

    fn get_echo(&self) -> Option<bool>;

    fn get_response_format<'s>(&self) -> Option<&'s str>;

    fn get_temperature(&self) -> Option<f32>;

    fn get_top_p(&self) -> Option<f32>;

    fn get_top_k(&self) -> Option<f32>;

    fn get_logprobs(&self) -> Option<u16>;

    fn get_frequency_penalty(&self) -> Option<f32>;

    fn get_presence_penalty(&self) -> Option<f32>;

    fn get_logit_bias<'s>(&'s self) -> Option<Box<dyn Iterator<Item = (&'s str, i64)> + 's>>;
}

pub(super) struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
    name: Option<&'a str>,
}

pub(super) trait ChatModelRequest: GenerativeTextModelRequest {
    type Response: ChatModelResponse + TryFrom<Box<dyn ChatModelResponse>>;

    fn get_messages<'s>(&'s self) -> Box<dyn Iterator<Item = ChatMessage<'s>> + 's>;
}

pub(super) enum StringOrTokenSet<'a> {
    String(&'a str),
    TokenSet(Box<dyn Iterator<Item = u64> + 'a>),
}

pub(super) trait CompletionModelRequest: GenerativeTextModelRequest {
    type Response: CompletionModelResponse + TryFrom<Box<dyn CompletionModelResponse>>;

    fn get_prompt<'s>(&'s self) -> Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's>;
}

pub(super) trait EmbeddingModelRequest: RoutableModelRequest {
    type Response: EmbeddingModelResponse + TryFrom<Box<dyn EmbeddingModelResponse>>;

    fn get_input<'s>(&'s self) -> Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's>;

    fn get_encoding_format(&self) -> Option<&str>;

    fn get_dimensions(&self) -> Option<u32>;
}

pub(super) struct TokenUsage {
    total: u32,
    input: Option<u32>,
    output: Option<u32>,
}

pub(super) trait RoutableModelResponse: Send + Debug + 'static {
    fn get_token_usage(&self) -> Option<TokenUsage>;

    fn get_system_fingerprint(&self) -> Option<&str>;
}

pub(super) trait RoutableModelError: RoutableModelResponse {
    fn get_status_code(&self) -> StatusCode;

    // TODO: Figure out error handling API

    //fn get_error_type(&self) -> Box<dyn Error>;
}

pub(super) struct TopLogProb<'a> {
    token: &'a str,
    logprob: f32,
    bytes: Option<&'a [u8]>,
}

pub(super) struct LogProb<'a> {
    token: &'a str,
    logprob: f32,
    bytes: Option<&'a [u8]>,
    top_logprobs: Option<Box<dyn Iterator<Item = TopLogProb<'a>> + 'a>>,
}

pub(super) struct ChatChoice<'a> {
    message: ChatMessage<'a>,
    logprobs: Option<Box<dyn Iterator<Item = LogProb<'a>> + 'a>>,
    finish_reason: Option<&'a str>,
}

pub(super) trait ChatModelResponse: RoutableModelResponse {
    fn get_choices<'s>(&'s self) -> Box<dyn Iterator<Item = ChatChoice<'s>> + 's>;
}

pub(super) struct CompletionChoice<'a> {
    text: &'a str,
    logprobs: Option<Box<dyn Iterator<Item = LogProb<'a>> + 'a>>,
    finish_reason: Option<&'a str>,
}

pub(super) trait CompletionModelResponse: RoutableModelResponse {
    fn get_choices<'s>(&'s self) -> Box<dyn Iterator<Item = CompletionChoice<'s>> + 's>;
}

pub(super) trait EmbeddingModelResponse: RoutableModelResponse {
    fn get_embeddings<'s>(
        &'s self,
    ) -> Box<dyn Iterator<Item = Box<dyn Iterator<Item = f32> + 's>> + 's>;
}
