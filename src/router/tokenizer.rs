use async_openai::{
    error::ApiError,
    types::{
        ChatCompletionRequestMessage, ChatCompletionRequestMessageContentPart,
        ChatCompletionRequestUserMessageContent, CreateChatCompletionRequest,
        CreateChatCompletionResponse, CreateCompletionRequest, CreateCompletionResponse,
        CreateEditRequest, CreateEditResponse, CreateEmbeddingRequest, CreateEmbeddingResponse,
        CreateImageEditRequest, CreateImageRequest, CreateImageVariationRequest,
        CreateModerationRequest, CreateModerationResponse, CreateTranscriptionRequest,
        CreateTranscriptionResponse, CreateTranslationRequest, CreateTranslationResponse,
        EmbeddingInput, ImageModel, ImagesResponse, ModerationInput, Prompt, TextModerationModel,
    },
};
use tiktoken_rs::{
    model,
    tokenizer::{self, Tokenizer},
};

use super::{ModelRequest, ModelResponse};
use crate::api;

impl api::Model {
    fn get_tokenizer(&self) -> Option<Tokenizer> {
        match self.metadata.tokenizer.as_deref() {
            Some("cl100k_base") => Some(Tokenizer::Cl100kBase),
            Some("p50k_base") => Some(Tokenizer::P50kBase),
            Some("p50k_edit") => Some(Tokenizer::P50kEdit),
            Some("r50k_base") => Some(Tokenizer::R50kBase),
            Some("gpt2") => Some(Tokenizer::Gpt2),
            _ => tokenizer::get_tokenizer(&self.label),
        }
    }

    fn get_context_len(&self) -> usize {
        match self.metadata.context_len {
            Some(len) => len,
            None => model::get_context_size(&self.label),
        }
    }
}

impl ModelRequest {
    pub fn get_input_tokens(&self, model: &api::Model) -> Option<usize> {
        match self {
            Self::Chat(r) => Some(get_token_count_messages(
                model,
                Tokenizer::Cl100kBase,
                r.messages.clone(),
            )),
            Self::Edit(r) => {
                let mut input = Vec::new();
                if let Some(i) = r.input.clone() {
                    input.push(i);
                }
                input.push(r.instruction.clone());

                Some(get_token_count_array(model, Tokenizer::P50kEdit, input))
            }
            Self::Completion(r) => Some(match r.prompt.clone() {
                Prompt::String(text) => get_token_count(model, Tokenizer::Cl100kBase, &text),
                Prompt::StringArray(text_array) => {
                    get_token_count_array(model, Tokenizer::Cl100kBase, text_array)
                }
                Prompt::IntegerArray(tokens) => tokens.len(),
                Prompt::ArrayOfIntegerArray(token_array) => token_array.concat().len(),
            }),
            Self::Moderation(r) => Some(match r.input.clone() {
                ModerationInput::String(text) => {
                    get_token_count(model, Tokenizer::Cl100kBase, &text)
                }
                ModerationInput::StringArray(text_array) => {
                    get_token_count_array(model, Tokenizer::Cl100kBase, text_array)
                }
            }),
            Self::Embedding(r) => Some(match r.input.clone() {
                EmbeddingInput::String(text) => {
                    get_token_count(model, Tokenizer::Cl100kBase, &text)
                }
                EmbeddingInput::StringArray(text_array) => {
                    get_token_count_array(model, Tokenizer::Cl100kBase, text_array)
                }
                EmbeddingInput::IntegerArray(tokens) => tokens.len(),
                EmbeddingInput::ArrayOfIntegerArray(token_array) => token_array.concat().len(),
            }),
            Self::Image(r) => Some(r.n.unwrap_or(1) as usize),
            Self::ImageEdit(r) => Some(r.n.unwrap_or(1) as usize),
            Self::ImageVariation(r) => Some(r.n.unwrap_or(1) as usize),
            Self::Transcription(r) => None,
            Self::Translation(r) => None,
        }
    }

    pub fn get_max_tokens(&self, model: &api::Model) -> Option<usize> {
        match self {
            Self::Chat(r) => Some(model.get_context_len()),
            Self::Edit(r) => Some(model.get_context_len()),
            Self::Completion(r) => Some(model.get_context_len()),
            Self::Moderation(r) => None,
            Self::Embedding(r) => None,
            Self::Image(r) => None,
            Self::ImageEdit(r) => None,
            Self::ImageVariation(r) => None,
            Self::Transcription(r) => None,
            Self::Translation(r) => None,
        }
    }
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
            Self::Image(r) => Some(r.data.len() as u32),
            Self::Transcription(_) => None,
            Self::Translation(_) => None,
        }
    }
}

fn get_token_count(model: &api::Model, default_tokenizer: Tokenizer, text: &str) -> usize {
    let bpe_arc = match model.get_tokenizer() {
        Some(Tokenizer::Cl100kBase) => tiktoken_rs::cl100k_base_singleton(),
        Some(Tokenizer::P50kBase) => tiktoken_rs::p50k_base_singleton(),
        Some(Tokenizer::P50kEdit) => tiktoken_rs::p50k_edit_singleton(),
        Some(Tokenizer::R50kBase) | Some(Tokenizer::Gpt2) => tiktoken_rs::r50k_base_singleton(),
        None => match default_tokenizer {
            Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
            Tokenizer::P50kBase => tiktoken_rs::p50k_base_singleton(),
            Tokenizer::P50kEdit => tiktoken_rs::p50k_edit_singleton(),
            Tokenizer::R50kBase | Tokenizer::Gpt2 => tiktoken_rs::r50k_base_singleton(),
        },
    };

    let bpe = bpe_arc.lock();
    bpe.encode_with_special_tokens(text).len()
}

fn get_token_count_array(
    model: &api::Model,
    default_tokenizer: Tokenizer,
    text: Vec<String>,
) -> usize {
    let bpe_arc = match model.get_tokenizer() {
        Some(Tokenizer::Cl100kBase) => tiktoken_rs::cl100k_base_singleton(),
        Some(Tokenizer::P50kBase) => tiktoken_rs::p50k_base_singleton(),
        Some(Tokenizer::P50kEdit) => tiktoken_rs::p50k_edit_singleton(),
        Some(Tokenizer::R50kBase) | Some(Tokenizer::Gpt2) => tiktoken_rs::r50k_base_singleton(),
        None => match default_tokenizer {
            Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
            Tokenizer::P50kBase => tiktoken_rs::p50k_base_singleton(),
            Tokenizer::P50kEdit => tiktoken_rs::p50k_edit_singleton(),
            Tokenizer::R50kBase | Tokenizer::Gpt2 => tiktoken_rs::r50k_base_singleton(),
        },
    };

    let mut num_tokens = 0;

    let bpe = bpe_arc.lock();
    for input in text {
        num_tokens += bpe.encode_with_special_tokens(&input).len();
    }

    num_tokens
}

fn get_token_count_messages(
    model: &api::Model,
    default_tokenizer: Tokenizer,
    messages: Vec<ChatCompletionRequestMessage>,
) -> usize {
    let bpe_arc = match model.get_tokenizer() {
        Some(Tokenizer::Cl100kBase) => tiktoken_rs::cl100k_base_singleton(),
        Some(Tokenizer::P50kBase) => tiktoken_rs::p50k_base_singleton(),
        Some(Tokenizer::P50kEdit) => tiktoken_rs::p50k_edit_singleton(),
        Some(Tokenizer::R50kBase) | Some(Tokenizer::Gpt2) => tiktoken_rs::r50k_base_singleton(),
        None => match default_tokenizer {
            Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
            Tokenizer::P50kBase => tiktoken_rs::p50k_base_singleton(),
            Tokenizer::P50kEdit => tiktoken_rs::p50k_edit_singleton(),
            Tokenizer::R50kBase | Tokenizer::Gpt2 => tiktoken_rs::r50k_base_singleton(),
        },
    };

    let mut num_tokens = 3;

    let tokens_per_message = match model.metadata.tokens_per_message {
        Some(t) => t,
        None => {
            if model.label.starts_with("gpt-3.5") {
                4
            } else {
                3
            }
        }
    };
    let tokens_per_name = match model.metadata.tokens_per_name {
        Some(t) => t,
        None => {
            if model.label.starts_with("gpt-3.5") {
                -1
            } else {
                1
            }
        }
    };

    let bpe = bpe_arc.lock();
    let system_role_tokens: i32 = bpe.encode_with_special_tokens("system").len() as i32;
    let user_role_tokens: i32 = bpe.encode_with_special_tokens("user").len() as i32;
    let assistant_role_tokens: i32 = bpe.encode_with_special_tokens("assistant").len() as i32;
    let tool_role_tokens: i32 = bpe.encode_with_special_tokens("tool").len() as i32;
    let function_role_tokens: i32 = bpe.encode_with_special_tokens("function").len() as i32;

    for message in messages {
        num_tokens += tokens_per_message;
        match message {
            ChatCompletionRequestMessage::System(m) => {
                num_tokens += system_role_tokens;

                num_tokens += bpe.encode_with_special_tokens(&m.content).len() as i32;
                if let Some(name) = m.name {
                    num_tokens += bpe.encode_with_special_tokens(&name).len() as i32;
                    num_tokens += tokens_per_name;
                }
            }
            ChatCompletionRequestMessage::User(m) => {
                num_tokens += user_role_tokens;

                match m.content {
                    ChatCompletionRequestUserMessageContent::Text(content) => {
                        num_tokens += bpe.encode_with_special_tokens(&content).len() as i32;
                    }
                    ChatCompletionRequestUserMessageContent::Array(content_array) => {
                        for content in content_array {
                            if let ChatCompletionRequestMessageContentPart::Text(c) = content {
                                num_tokens += bpe.encode_with_special_tokens(&c.text).len() as i32;
                            }
                        }
                    }
                }
                if let Some(name) = m.name {
                    num_tokens += bpe.encode_with_special_tokens(&name).len() as i32;
                    num_tokens += tokens_per_name;
                }
            }
            ChatCompletionRequestMessage::Assistant(m) => {
                num_tokens += assistant_role_tokens;

                if let Some(content) = m.content {
                    num_tokens += bpe.encode_with_special_tokens(&content).len() as i32;
                }
                if let Some(name) = m.name {
                    num_tokens += bpe.encode_with_special_tokens(&name).len() as i32;
                    num_tokens += tokens_per_name;
                }

                for tool_call in m.tool_calls.unwrap_or_default() {
                    num_tokens += bpe
                        .encode_with_special_tokens(&tool_call.function.name)
                        .len() as i32;
                    num_tokens += bpe
                        .encode_with_special_tokens(&tool_call.function.arguments)
                        .len() as i32;
                }

                #[allow(deprecated)]
                if let Some(function) = m.function_call {
                    num_tokens += bpe.encode_with_special_tokens(&function.name).len() as i32;
                    num_tokens += bpe.encode_with_special_tokens(&function.arguments).len() as i32;
                }
            }
            ChatCompletionRequestMessage::Tool(m) => {
                num_tokens += tool_role_tokens;
                num_tokens += bpe.encode_with_special_tokens(&m.content).len() as i32;
            }
            ChatCompletionRequestMessage::Function(m) => {
                num_tokens += function_role_tokens;
                num_tokens += bpe.encode_with_special_tokens(&m.name).len() as i32;
                if let Some(content) = m.content {
                    num_tokens += bpe.encode_with_special_tokens(&content).len() as i32;
                }
            }
        }
    }

    num_tokens as usize
}
