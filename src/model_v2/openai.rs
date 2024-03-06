use std::{collections::HashMap, iter, ops::Deref, sync::Arc};

use async_openai::{
    config::OpenAIConfig,
    error::OpenAIError,
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestUserMessageContent,
        ChatCompletionResponseFormatType, CompletionFinishReason, CreateImageEditRequest,
        CreateImageRequest, CreateImageVariationRequest, CreateTranscriptionRequest,
        CreateTranscriptionResponse, CreateTranslationRequest, CreateTranslationResponse,
        EmbeddingInput, EncodingFormat, FinishReason, ImageModel, ModerationInput, Prompt,
        ResponseFormat, Role, Stop, TextModerationModel,
    },
    Client,
};
pub(super) use async_openai::{
    error::ApiError,
    types::{
        CreateChatCompletionRequest, CreateChatCompletionResponse, CreateCompletionRequest,
        CreateCompletionResponse, CreateEditRequest, CreateEditResponse, CreateEmbeddingRequest,
        CreateEmbeddingResponse, CreateModerationRequest, CreateModerationResponse, ImagesResponse,
    },
};
use opentelemetry::trace::Status;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::value::Value;

use super::{
    ChatChoice, ChatMessage, ChatModel, ChatModelRequest, ChatModelResponse, CompletionChoice,
    CompletionModel, CompletionModelRequest, CompletionModelResponse, EmbeddingModel,
    EmbeddingModelRequest, EmbeddingModelResponse, GenerativeTextModelRequest, LogProb,
    RoutableModel, RoutableModelError, RoutableModelRequest, RoutableModelResponse,
    StringOrTokenSet, TextBasedModel, TokenUsage, TopLogProb,
};

#[derive(Debug)]
struct OpenAIModelAPI {
    internal_model_label: String,
    openai_api_base: String,
    openai_api_key: String,
    openai_organization: Option<String>,
}

impl OpenAIModelAPI {
    fn init_client(&self) -> Client<OpenAIConfig> {
        let mut config = OpenAIConfig::new()
            .with_api_base(&self.openai_api_base)
            .with_api_key(&self.openai_api_key);

        if let Some(org) = &self.openai_organization {
            config = config.with_org_id(org);
        }

        async_openai::Client::with_config(config)
    }
}

#[derive(Debug)]
struct OpenAIChatModel {
    label: String,
    api_endpoint: OpenAIModelAPI,
    context_len: u32,
    tokenizer: String,
}

#[derive(Debug)]
struct OpenAICompletionModel {
	label: String,
	api_endpoint: OpenAIModelAPI,
	context_len: u32,
	tokenizer: String,
}

#[derive(Debug)]
struct OpenAIEmbeddingModel {
	label: String,
	api_endpoint: OpenAIModelAPI,
	max_tokens: u32,
	tokenizer: String,
}

impl RoutableModel for OpenAIChatModel {
    fn get_label(&self) -> &str {
        &self.label
    }

    fn update_label(&mut self, label: String) {
        self.label = label
    }

    fn get_max_tokens(&self) -> u32 {
        self.context_len
    }
}

impl RoutableModel for OpenAICompletionModel {
	fn get_label(&self) -> &str {
		&self.label
	}

	fn update_label(&mut self, label: String) {
		self.label = label
	}

    fn get_max_tokens(&self) -> u32 {
        self.context_len
    }
}

impl RoutableModel for OpenAIEmbeddingModel {
	fn get_label(&self) -> &str {
		&self.label
	}

	fn update_label(&mut self, label: String) {
		self.label = label
	}

    fn get_max_tokens(&self) -> u32 {
        self.max_tokens
    }
}

impl TextBasedModel for OpenAIChatModel {
    fn get_tokenizer(&self) -> &str {
        &self.tokenizer
    }
}

impl TextBasedModel for OpenAICompletionModel {
    fn get_tokenizer(&self) -> &str {
        &self.tokenizer
    }
}

impl TextBasedModel for OpenAIEmbeddingModel {
    fn get_tokenizer(&self) -> &str {
        &self.tokenizer
    }
}

impl ChatModel for OpenAIChatModel {
    type Client = Client<OpenAIConfig>;
    type ModelRequest = CreateChatCompletionRequest;
    type ModelResponse = CreateChatCompletionResponse;
    type ModelError = OpenAIError;

    fn init(&self) -> Self::Client {
        self.api_endpoint.init_client()
    }

    async fn generate(
        &self,
        client: Self::Client,
        tag: Option<uuid::Uuid>,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = self.api_endpoint.internal_model_label.clone();
        request.stream = None;
        request.user = tag.map(|uuid| {
            format!("{}", uuid)
        });

        client
            .chat()
            .create(request)
            .await
            .map(|mut response| {
                response.model = self.label.clone();
                response
            })
    }
}

impl CompletionModel for OpenAICompletionModel {
	type Client = Client<OpenAIConfig>;
	type ModelRequest = CreateCompletionRequest;
	type ModelResponse = CreateCompletionResponse;
	type ModelError = OpenAIError;

    fn init(&self) -> Self::Client {
        self.api_endpoint.init_client()
    }

	async fn generate(
        &self,
        client: Self::Client,
        tag: Option<uuid::Uuid>,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = self.api_endpoint.internal_model_label.clone();
        request.stream = None;
        request.user = tag.map(|uuid| {
            format!("{}", uuid)
        });

		client
            .completions()
            .create(request)
            .await
            .map(|mut response| {
                response.model = self.label.clone();
                response
            })
	}
}

impl EmbeddingModel for OpenAIEmbeddingModel {
	type Client = Client<OpenAIConfig>;
	type ModelRequest = CreateEmbeddingRequest;
	type ModelResponse = CreateEmbeddingResponse;
	type ModelError = OpenAIError;

    fn init(&self) -> Self::Client {
        self.api_endpoint.init_client()
    }

	async fn generate(
        &self,
        client: Self::Client,
        tag: Option<uuid::Uuid>,
        mut request: Self::ModelRequest,
    ) -> Result<Self::ModelResponse, Self::ModelError> {
        request.model = self.api_endpoint.internal_model_label.clone();

		client
            .embeddings()
            .create(request)
            .await
            .map(|mut response| {
                response.model = self.label.clone();
                response
            })
	}
}

impl RoutableModelRequest for CreateChatCompletionRequest {
    fn get_model(&self) -> &str {
        &self.model
    }

    fn get_count(&self) -> u32 {
        self.n.unwrap_or(1) as u32
    }

    fn get_max_tokens(&self) -> Option<u32> {
        self.max_tokens.map(|max_tokens| max_tokens.into())
    }
}

impl RoutableModelRequest for CreateCompletionRequest {
    fn get_model(&self) -> &str {
        &self.model
    }

    fn get_count(&self) -> u32 {
        self.best_of.unwrap_or(1).max(self.n.unwrap_or(1)) as u32
            * match &self.prompt {
                Prompt::String(_) => 1,
                Prompt::StringArray(p) => p.len() as u32,
                Prompt::IntegerArray(_) => 1,
                Prompt::ArrayOfIntegerArray(p) => p.len() as u32,
            }
    }

    fn get_max_tokens(&self) -> Option<u32> {
        self.max_tokens.map(|max_tokens| max_tokens.into())
    }
}

impl RoutableModelRequest for CreateEmbeddingRequest {
    fn get_model(&self) -> &str {
        &self.model
    }

    fn get_count(&self) -> u32 {
        match &self.input {
            EmbeddingInput::String(_) => 1,
            EmbeddingInput::StringArray(p) => p.len() as u32,
            EmbeddingInput::IntegerArray(_) => 1,
            EmbeddingInput::ArrayOfIntegerArray(p) => p.len() as u32,
        }
    }

    fn get_max_tokens(&self) -> Option<u32> {
        None
    }
}

impl GenerativeTextModelRequest for CreateChatCompletionRequest {
    fn get_n(&self) -> Option<u16> {
        self.n.map(|n| n.into())
    }

    fn get_best_of(&self) -> Option<u16> {
        None
    }

    fn get_seed(&self) -> Option<i64> {
        self.seed
    }

    fn get_suffix(&self) -> Option<&str> {
        None
    }

    fn get_stop_sequences<'s>(&'s self) -> Option<Box<dyn Iterator<Item = &'s str> + 's>> {
        self.stop.as_ref().map(|stop| match stop {
            Stop::String(s) => {
                Box::new(iter::once(s.as_str())) as Box<dyn Iterator<Item = &'s str>>
            }
            Stop::StringArray(s) => {
                Box::new(s.iter().map(|s| s.deref())) as Box<dyn Iterator<Item = &'s str>>
            }
        })
    }

    fn get_echo(&self) -> Option<bool> {
        None
    }

    fn get_response_format<'s>(&self) -> Option<&'s str> {
        self.response_format
            .as_ref()
            .map(|format| match format.r#type {
                ChatCompletionResponseFormatType::JsonObject => "json_object",
                ChatCompletionResponseFormatType::Text => "text",
            })
    }

    fn get_temperature(&self) -> Option<f32> {
        self.temperature
    }

    fn get_top_p(&self) -> Option<f32> {
        self.top_p
    }

    fn get_top_k(&self) -> Option<f32> {
        None
    }

    fn get_logprobs(&self) -> Option<u16> {
        self.top_logprobs
            .map(|logprobs| logprobs.into())
            .or(match self.logprobs {
                Some(true) => Some(1),
                _ => None,
            })
    }

    fn get_frequency_penalty(&self) -> Option<f32> {
        self.frequency_penalty
    }

    fn get_presence_penalty(&self) -> Option<f32> {
        self.presence_penalty
    }

    fn get_logit_bias<'s>(&'s self) -> Option<Box<dyn Iterator<Item = (&'s str, i64)> + 's>> {
        self.logit_bias.as_ref().map(|logit_bias| {
            Box::new(logit_bias.iter().filter_map(|(key, val)| {
                if let Value::Number(num) = val {
                    if let Some(number) = num.as_i64() {
                        return Some((key.as_str(), number));
                    }
                }

                None
            })) as Box<dyn Iterator<Item = (&'s str, i64)> + 's>
        })
    }
}

impl GenerativeTextModelRequest for CreateCompletionRequest {
    fn get_n(&self) -> Option<u16> {
        self.n.map(|n| n.into())
    }

    fn get_best_of(&self) -> Option<u16> {
        self.best_of.map(|best_of| best_of.into())
    }

    fn get_seed(&self) -> Option<i64> {
        self.seed
    }

    fn get_suffix(&self) -> Option<&str> {
        self.suffix.as_deref()
    }

    fn get_stop_sequences<'s>(&'s self) -> Option<Box<dyn Iterator<Item = &'s str> + 's>> {
        self.stop.as_ref().map(|stop| match stop {
            Stop::String(s) => {
                Box::new(iter::once(s.as_str())) as Box<dyn Iterator<Item = &'s str>>
            }
            Stop::StringArray(s) => {
                Box::new(s.iter().map(|s| s.deref())) as Box<dyn Iterator<Item = &'s str>>
            }
        })
    }

    fn get_echo(&self) -> Option<bool> {
        self.echo
    }

    fn get_response_format<'s>(&self) -> Option<&'s str> {
        None
    }

    fn get_temperature(&self) -> Option<f32> {
        self.temperature
    }

    fn get_top_p(&self) -> Option<f32> {
        self.top_p
    }

    fn get_top_k(&self) -> Option<f32> {
        None
    }

    fn get_logprobs(&self) -> Option<u16> {
        self.logprobs.map(|logprobs| logprobs.into())
    }

    fn get_frequency_penalty(&self) -> Option<f32> {
        self.frequency_penalty
    }

    fn get_presence_penalty(&self) -> Option<f32> {
        self.presence_penalty
    }

    fn get_logit_bias<'s>(&'s self) -> Option<Box<dyn Iterator<Item = (&'s str, i64)> + 's>> {
        self.logit_bias.as_ref().map(|logit_bias| {
            Box::new(logit_bias.iter().filter_map(|(key, val)| {
                if let Value::Number(num) = val {
                    if let Some(number) = num.as_i64() {
                        return Some((key.as_str(), number));
                    }
                }

                None
            })) as Box<dyn Iterator<Item = (&'s str, i64)> + 's>
        })
    }
}

impl ChatModelRequest for CreateChatCompletionRequest {
    type Response = CreateChatCompletionResponse;

    fn get_messages<'s>(&'s self) -> Box<dyn Iterator<Item = ChatMessage<'s>> + 's> {
        Box::new(self.messages.iter().filter_map(|message| match message {
            ChatCompletionRequestMessage::User(m) => {
                if let ChatCompletionRequestUserMessageContent::Text(content) = &m.content {
                    return Some(ChatMessage {
                        role: "user",
                        content,
                        name: m.name.as_deref(),
                    });
                }

                None
            }
            ChatCompletionRequestMessage::System(m) => Some(ChatMessage {
                role: "system",
                content: &m.content,
                name: m.name.as_deref(),
            }),
            ChatCompletionRequestMessage::Assistant(m) => {
                if let Some(content) = &m.content {
                    return Some(ChatMessage {
                        role: "assistant",
                        content,
                        name: m.name.as_deref(),
                    });
                }

                None
            }
            _ => None,
        }))
    }
}

impl CompletionModelRequest for CreateCompletionRequest {
    type Response = CreateCompletionResponse;

    fn get_prompt<'s>(&'s self) -> Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's> {
        match &self.prompt {
            Prompt::String(s) => Box::new(iter::once(StringOrTokenSet::String(s.as_str())))
                as Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's>,
            Prompt::StringArray(s) => {
                Box::new(s.iter().map(|s| StringOrTokenSet::String(s.deref())))
                    as Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's>
            }
            Prompt::IntegerArray(i) => Box::new(iter::once(StringOrTokenSet::TokenSet(Box::new(
                i.iter().map(|i| (*i).into()),
            ))))
                as Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's>,
            Prompt::ArrayOfIntegerArray(i) => Box::new(
                i.iter()
                    .map(|i| StringOrTokenSet::TokenSet(Box::new(i.iter().map(|i| (*i).into())))),
            )
                as Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's>,
        }
    }
}

impl EmbeddingModelRequest for CreateEmbeddingRequest {
    type Response = CreateEmbeddingResponse;

    fn get_input<'s>(&'s self) -> Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's> {
        match &self.input {
            EmbeddingInput::String(s) => Box::new(iter::once(StringOrTokenSet::String(s.as_str())))
                as Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's>,
            EmbeddingInput::StringArray(s) => {
                Box::new(s.iter().map(|s| StringOrTokenSet::String(s.deref())))
                    as Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's>
            }
            EmbeddingInput::IntegerArray(i) => Box::new(iter::once(StringOrTokenSet::TokenSet(
                Box::new(i.iter().map(|i| (*i).into())),
            )))
                as Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's>,
            EmbeddingInput::ArrayOfIntegerArray(i) => Box::new(
                i.iter()
                    .map(|i| StringOrTokenSet::TokenSet(Box::new(i.iter().map(|i| (*i).into())))),
            )
                as Box<dyn Iterator<Item = StringOrTokenSet<'s>> + 's>,
        }
    }

    fn get_encoding_format(&self) -> Option<&str> {
        match self.encoding_format {
            Some(EncodingFormat::Float) => Some("float"),
            Some(EncodingFormat::Base64) => Some("base64"),
            None => None,
        }
    }

    fn get_dimensions(&self) -> Option<u32> {
        None
    }
}

impl RoutableModelResponse for CreateChatCompletionResponse {
    fn get_token_usage(&self) -> Option<TokenUsage> {
        self.usage.as_ref().map(|usage| TokenUsage {
            total: usage.total_tokens,
            input: Some(usage.prompt_tokens),
            output: Some(usage.completion_tokens),
        })
    }

    fn get_system_fingerprint(&self) -> Option<&str> {
        self.system_fingerprint.as_deref()
    }
}

impl RoutableModelResponse for CreateCompletionResponse {
    fn get_token_usage(&self) -> Option<TokenUsage> {
        self.usage.as_ref().map(|usage| TokenUsage {
            total: usage.total_tokens,
            input: Some(usage.prompt_tokens),
            output: Some(usage.completion_tokens),
        })
    }

    fn get_system_fingerprint(&self) -> Option<&str> {
        self.system_fingerprint.as_deref()
    }
}

impl RoutableModelResponse for CreateEmbeddingResponse {
    fn get_token_usage(&self) -> Option<TokenUsage> {
        Some(TokenUsage {
            total: self.usage.total_tokens,
            input: Some(self.usage.prompt_tokens),
            output: None,
        })
    }

    fn get_system_fingerprint(&self) -> Option<&str> {
        None
    }
}

impl RoutableModelResponse for OpenAIError {
    fn get_token_usage(&self) -> Option<TokenUsage> {
        None
    }

    fn get_system_fingerprint(&self) -> Option<&str> {
        None
    }
}

impl ChatModelResponse for CreateChatCompletionResponse {
    fn get_choices<'s>(&'s self) -> Box<dyn Iterator<Item = ChatChoice<'s>> + 's> {
        Box::new(self.choices.iter().filter_map(|choice| {
            if let Some(content) = &choice.message.content {
                return Some(ChatChoice {
                    message: ChatMessage {
                        role: match choice.message.role {
                            Role::System => "system",
                            Role::User => "user",
                            Role::Assistant => "assistant",
                            Role::Tool => "tool",
                            Role::Function => "function",
                        },
                        content,
                        name: None,
                    },
                    logprobs: choice.logprobs.as_ref().and_then(|logprobs| {
                        logprobs.content.as_ref().map(|logprobs| {
                            Box::new(logprobs.iter().map(|logprob| LogProb {
                                token: &logprob.token,
                                logprob: logprob.logprob,
                                bytes: logprob.bytes.as_deref(),
                                top_logprobs: Some(Box::new(logprob.top_logprobs.iter().map(
                                    |toplogprob| TopLogProb {
                                        token: &toplogprob.token,
                                        logprob: toplogprob.logprob,
                                        bytes: toplogprob.bytes.as_deref(),
                                    },
                                ))
                                    as Box<dyn Iterator<Item = TopLogProb<'s>> + 's>),
                            }))
                                as Box<dyn Iterator<Item = LogProb<'s>> + 's>
                        })
                    }),
                    finish_reason: choice
                        .finish_reason
                        .map(|finish_reason| match finish_reason {
                            FinishReason::Stop => "stop",
                            FinishReason::Length => "length",
                            FinishReason::ToolCalls => "tool_calls",
                            FinishReason::ContentFilter => "content_filter",
                            FinishReason::FunctionCall => "function_call",
                        }),
                });
            }

            None
        }))
    }
}

impl CompletionModelResponse for CreateCompletionResponse {
    fn get_choices<'s>(&'s self) -> Box<dyn Iterator<Item = CompletionChoice<'s>> + 's> {
        Box::new(self.choices.iter().map(|choice| {
            CompletionChoice {
                text: &choice.text,
                logprobs: choice.logprobs.as_ref().map(|logprobs| {
                    Box::new(logprobs.tokens.iter().enumerate().filter_map(|(i, token)| {
                        if let Some(Some(logprob)) = logprobs.token_logprobs.get(i) {
                            return Some(LogProb {
                                token,
                                logprob: *logprob,
                                bytes: None,
                                top_logprobs: logprobs.top_logprobs.get(i).and_then(
                                    |top_logprobs| match top_logprobs {
                                        Value::Object(obj_map) => Some(Box::new(
                                            obj_map.iter().flat_map(|(token, value)| match value {
                                                Value::Number(num) => {
                                                    num.as_f64().map(|logprob| TopLogProb {
                                                        token,
                                                        logprob: logprob as f32,
                                                        bytes: None,
                                                    })
                                                }
                                                _ => None,
                                            }),
                                        )
                                            as Box<dyn Iterator<Item = TopLogProb<'s>> + 's>),
                                        _ => None,
                                    },
                                ),
                            });
                        }

                        None
                    })) as Box<dyn Iterator<Item = LogProb<'s>> + 's>
                }),
                finish_reason: choice
                    .finish_reason
                    .map(|finish_reason| match finish_reason {
                        CompletionFinishReason::Stop => "stop",
                        CompletionFinishReason::Length => "length",
                        CompletionFinishReason::ContentFilter => "content_filter",
                    }),
            }
        }))
    }
}

impl EmbeddingModelResponse for CreateEmbeddingResponse {
    fn get_embeddings<'s>(
        &'s self,
    ) -> Box<dyn Iterator<Item = Box<dyn Iterator<Item = f32> + 's>> + 's> {
        Box::new(self.data.iter().map(|embedding| {
            Box::new(embedding.embedding.iter().copied()) as Box<dyn Iterator<Item = f32>>
        })) as Box<dyn Iterator<Item = Box<dyn Iterator<Item = f32> + 's>> + 's>
    }
}

impl RoutableModelError for OpenAIError {
    fn get_status_code(&self) -> StatusCode {
        match self {
            OpenAIError::Reqwest(err) => {
                if err.is_timeout() || err.is_redirect() {
                    StatusCode::GATEWAY_TIMEOUT
                } else if err.is_connect() || err.is_body() || err.is_decode() {
                    StatusCode::BAD_GATEWAY
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
            OpenAIError::ApiError(err) => match err.r#type.as_deref() {
                Some("invalid_request_error") => StatusCode::BAD_REQUEST,
                Some("insufficient_quota") => StatusCode::SERVICE_UNAVAILABLE,
                Some("server_error") => StatusCode::BAD_GATEWAY,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
            OpenAIError::JSONDeserialize(_) => StatusCode::BAD_GATEWAY,
            OpenAIError::FileSaveError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            OpenAIError::FileReadError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            OpenAIError::StreamError(_) => StatusCode::BAD_GATEWAY,
            OpenAIError::InvalidArgument(_) => StatusCode::BAD_REQUEST,
        }
    }
}

// ! FIXME

impl<T> TryFrom<Box<dyn ChatModelRequest<Response = T>>> for CreateChatCompletionRequest {
    type Error = &'static str;

    fn try_from(value: Box<dyn ChatModelRequest<Response = T>>) -> Result<Self, Self::Error> {
        todo!()
    }
}

impl<T> TryFrom<Box<dyn CompletionModelRequest<Response = T>>> for CreateCompletionRequest {
    type Error = &'static str;

    fn try_from(value: Box<dyn CompletionModelRequest<Response = T>>) -> Result<Self, Self::Error> {
        todo!()
    }
}

impl<T> TryFrom<Box<dyn EmbeddingModelRequest<Response = T>>> for CreateEmbeddingRequest {
    type Error = &'static str;

    fn try_from(value: Box<dyn EmbeddingModelRequest<Response = T>>) -> Result<Self, Self::Error> {
        todo!()
    }
}

impl TryFrom<Box<dyn ChatModelResponse>> for CreateChatCompletionResponse {
    type Error = &'static str;

    fn try_from(value: Box<dyn ChatModelResponse>) -> Result<Self, Self::Error> {
        todo!()
    }
}

impl TryFrom<Box<dyn CompletionModelResponse>> for CreateCompletionResponse {
    type Error = &'static str;

    fn try_from(value: Box<dyn CompletionModelResponse>) -> Result<Self, Self::Error> {
        todo!()
    }
}

impl TryFrom<Box<dyn EmbeddingModelResponse>> for CreateEmbeddingResponse {
    type Error = &'static str;

    fn try_from(value: Box<dyn EmbeddingModelResponse>) -> Result<Self, Self::Error> {
        todo!()
    }
}
