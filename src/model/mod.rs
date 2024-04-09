use std::{
    collections::HashMap,
    fmt::Debug,
    time::{SystemTime, UNIX_EPOCH},
};

use fast32::base32::{CROCKFORD, RFC4648};
use http::{status::StatusCode, Uri};
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    Client, Method, Url,
};
use ring::digest;
use serde::{Deserialize, Serialize};
use serde_json::{json, value::Value, Map};
use uuid::Uuid;

use self::tokenizer::{TokenizerMessage, TokenizerSettings};

mod client;
mod endpoints;
mod interface;
mod tokenizer;
mod utility;

#[tracing::instrument(level = "trace", ret)]
fn get_prompt_count(prompt: &Value) -> usize {
    match prompt {
        Value::Array(array) => {
            if !array.is_empty() {
                match array[0] {
                    Value::Array(_) | Value::Object(_) | Value::String(_) => array.len(),
                    _ => 1,
                }
            } else {
                0
            }
        }
        Value::Object(_) => 1,
        Value::String(_) | Value::Number(_) | Value::Bool(_) => 1,
        Value::Null => 0,
    }
}

#[tracing::instrument(level = "trace", ret)]
fn get_token_count(map: &Map<String, Value>, tokenizer: &TokenizerSettings) -> Option<usize> {
    None // ! FIXME
}

#[derive(Debug)]
pub(super) struct ModelRequest {
    pub(super) user: Option<Uuid>,
    pub(super) r#type: RequestType,

    request: ModelRequestData,
}

#[derive(Debug)]
enum ModelRequestData {
    Json(Map<String, Value>),
    Form(HashMap<String, ModelFormItem>),
}

impl ModelRequestData {
    #[tracing::instrument(level = "trace", ret)]
    fn convert_type(
        &mut self,
        input_type: RequestType,
        desired_type: RequestType,
        model: String,
        user: Option<Uuid>,
    ) {
        let user = user.map(|user| {
            CROCKFORD.encode(digest::digest(&digest::SHA256, user.as_bytes()).as_ref())
        });

        match self {
            Self::Json(json) => {
                // TODO: Convert OpenAI requests with "N" values into multiple requests

                // TODO
                match desired_type {
                    RequestType::OpenAI_TextChat => match input_type {
                        RequestType::Anthropic_TextChat => {}
                        RequestType::OpenAI_TextCompletion => {
                            utility::update_item(json, "prompt", "messages", |value| match value {
                                Value::String(prompt) => Some(json!([{
                                    "role": "user",
                                    "content": prompt,
                                }])),
                                Value::Array(prompts) => {
                                    let prompts = prompts
                                        .iter()
                                        .filter_map(|prompt| {
                                            prompt.as_str().map(|text| {
                                                json!({
                                                    "role": "user",
                                                    "content": text,
                                                })
                                            })
                                        })
                                        .collect();

                                    Some(Value::Array(prompts))
                                }
                                _ => None,
                            });

                            utility::update_item(json, "logprobs", "top_logprobs", |value| {
                                value
                                    .as_u64()
                                    .filter(|value| *value > 0)
                                    .map(|value| json!(value))
                            });

                            if json.contains_key("top_logprobs") {
                                json.insert("logprobs".to_string(), Value::Bool(true));
                            }

                            json.remove("best_of");
                            json.remove("echo"); // TODO: Handle this properly!
                            json.remove("suffix"); // TODO: Handle this properly!
                        }
                        RequestType::Anthropic_TextCompletion => {}
                        _ => {}
                    },
                    RequestType::Anthropic_TextChat => match input_type {
                        RequestType::OpenAI_TextChat => {}
                        RequestType::OpenAI_TextCompletion => {}
                        RequestType::Anthropic_TextCompletion => {}
                        _ => {}
                    },
                    RequestType::OpenAI_TextCompletion => match input_type {
                        RequestType::OpenAI_TextChat => {
                            // TODO

                            //utility::update_array(json, "messages", "prompt", |value| {});
                        }
                        RequestType::Anthropic_TextChat => {}
                        RequestType::Anthropic_TextCompletion => {}
                        _ => {}
                    },
                    RequestType::Anthropic_TextCompletion => match input_type {
                        RequestType::OpenAI_TextChat => {}
                        RequestType::Anthropic_TextChat => {}
                        RequestType::OpenAI_TextCompletion => {}
                        _ => {}
                    },
                    _ => {}
                }

                if desired_type == RequestType::Anthropic_TextChat
                    || desired_type == RequestType::Anthropic_TextCompletion
                {
                    match &user {
                        Some(user) => {
                            json.insert(
                                "metadata".to_string(),
                                json!({
                                    "user_id": user,
                                }),
                            );
                        }
                        None => {
                            json.remove("metadata");
                        }
                    }
                }

                if desired_type == RequestType::OpenAI_TextChat
                    || desired_type == RequestType::OpenAI_TextCompletion
                    || desired_type == RequestType::OpenAI_TextEmbedding
                    || desired_type == RequestType::OpenAI_ImageGeneration
                    || desired_type == RequestType::OpenAI_ImageEdit
                    || desired_type == RequestType::OpenAI_ImageVariation
                {
                    match &user {
                        Some(user) => {
                            json.insert("user".to_string(), json!(user));
                        }
                        None => {
                            json.remove("user");
                        }
                    }
                }

                json.remove("stream");
                json.insert("model".to_string(), Value::String(model));
            }
            Self::Form(form) => {
                if desired_type == RequestType::OpenAI_ImageEdit
                    || desired_type == RequestType::OpenAI_ImageVariation
                {
                    match user {
                        Some(user) => {
                            form.insert("user".to_string(), ModelFormItem::Text(user));
                        }
                        None => {
                            form.remove("user");
                        }
                    }
                }

                form.insert("model".to_string(), ModelFormItem::Text(model));
            }
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    fn into_loopback(self) -> ModelResponse {
        let json = match self {
            Self::Json(json) => json,
            Self::Form(form) => {
                let mut json = Map::new();

                for (key, value) in form {
                    match value {
                        ModelFormItem::Text(text) => {
                            json.insert(key, Value::String(text));
                        }
                        ModelFormItem::File(file) => {
                            let mut file_json = Map::new();

                            file_json.insert(
                                "filename".to_string(),
                                file.file_name.map(Value::String).unwrap_or(Value::Null),
                            );
                            file_json.insert(
                                "content-type".to_string(),
                                file.content_type.map(Value::String).unwrap_or(Value::Null),
                            );

                            file_json.insert(
                                "data".to_string(),
                                Value::String(RFC4648.encode(&file.data)),
                            );

                            json.insert(key, Value::Object(file_json));
                        }
                    }
                }

                json
            }
        };

        ModelResponse {
            status: StatusCode::OK,
            usage: TokenUsage {
                total: 1,
                input: None,
                output: None,
            },
            response: ModelResponseData::Json(json),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    fn get_model(&self) -> Option<&str> {
        match self {
            Self::Json(json) => json.get("model").and_then(|value| value.as_str()),
            Self::Form(form) => {
                if let Some(ModelFormItem::Text(model)) = form.get("model") {
                    Some(model)
                } else {
                    None
                }
            }
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    fn get_count(&self) -> usize {
        match &self {
            Self::Json(json) => (json
                .get("best_of")
                .and_then(|value| {
                    value
                        .as_u64()
                        .map(|int| int.clamp(1, usize::MAX as u64) as usize)
                })
                .unwrap_or(1)
                * json
                    .get("n")
                    .and_then(|value| {
                        value
                            .as_u64()
                            .map(|int| int.clamp(1, usize::MAX as u64) as usize)
                    })
                    .unwrap_or(1)
                * json.get("prompt").map(get_prompt_count).unwrap_or(1)
                * json.get("input").map(get_prompt_count).unwrap_or(1))
            .max(1),
            Self::Form(form) => form
                .get("n")
                .and_then(|value| {
                    if let ModelFormItem::Text(string) = value {
                        Some(string)
                    } else {
                        None
                    }
                })
                .and_then(|string| string.parse().ok())
                .map(|value: usize| value.max(1))
                .unwrap_or(1),
        }
    }

    #[tracing::instrument(level = "trace", ret)]
    fn get_max_tokens(&self) -> Option<u64> {
        match self {
            Self::Json(json) => json
                .get("max_tokens")
                .and_then(|value| value.as_u64().map(|int| int.max(1))),
            Self::Form(_) => None,
        }
    }
}

#[derive(Debug)]
enum ModelFormItem {
    Text(String),
    File(ModelFormFile),
}

#[derive(Debug)]
struct ModelFormFile {
    file_name: Option<String>,
    content_type: Option<String>,
    data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub(super) enum RequestType {
    OpenAI_TextChat,
    Anthropic_TextChat,
    OpenAI_TextCompletion,
    Anthropic_TextCompletion,
    OpenAI_TextEdit,
    OpenAI_TextEmbedding,
    OpenAI_TextModeration,
    OpenAI_ImageGeneration,
    OpenAI_ImageEdit,
    OpenAI_ImageVariation,
    OpenAI_AudioTTS,
    OpenAI_AudioTranscription,
    OpenAI_AudioTranslation,
}

impl RequestType {
    fn is_text_chat(&self) -> bool {
        matches!(self, Self::OpenAI_TextChat | Self::Anthropic_TextChat)
    }

    fn is_text_completion(&self) -> bool {
        matches!(
            self,
            Self::OpenAI_TextCompletion | Self::Anthropic_TextCompletion
        )
    }

    fn is_text_based(&self) -> bool {
        matches!(
            self,
            Self::OpenAI_TextChat
                | Self::Anthropic_TextChat
                | Self::OpenAI_TextCompletion
                | Self::Anthropic_TextCompletion
                | Self::OpenAI_TextEdit
                | Self::OpenAI_TextEmbedding
                | Self::OpenAI_TextModeration
        )
    }

    fn is_image_based(&self) -> bool {
        matches!(
            self,
            Self::OpenAI_ImageGeneration | Self::OpenAI_ImageEdit | Self::OpenAI_ImageVariation
        )
    }

    fn is_audio_based(&self) -> bool {
        matches!(
            self,
            Self::OpenAI_AudioTTS | Self::OpenAI_AudioTranscription | Self::OpenAI_AudioTranslation
        )
    }

    fn is_openai(&self) -> bool {
        matches!(
            self,
            Self::OpenAI_TextChat
                | Self::OpenAI_TextCompletion
                | Self::OpenAI_TextEdit
                | Self::OpenAI_TextEmbedding
                | Self::OpenAI_TextModeration
                | Self::OpenAI_ImageGeneration
                | Self::OpenAI_ImageEdit
                | Self::OpenAI_ImageVariation
                | Self::OpenAI_AudioTTS
                | Self::OpenAI_AudioTranscription
                | Self::OpenAI_AudioTranslation
        )
    }

    fn as_openai(self) -> Option<Self> {
        match self {
            Self::OpenAI_TextChat | Self::Anthropic_TextChat => Some(Self::OpenAI_TextChat),
            Self::OpenAI_TextCompletion | Self::Anthropic_TextCompletion => {
                Some(Self::OpenAI_TextCompletion)
            }
            Self::OpenAI_TextEdit => Some(Self::OpenAI_TextEdit),
            Self::OpenAI_TextEmbedding => Some(Self::OpenAI_TextEmbedding),
            Self::OpenAI_TextModeration => Some(Self::OpenAI_TextModeration),
            Self::OpenAI_ImageGeneration => Some(Self::OpenAI_ImageGeneration),
            Self::OpenAI_ImageEdit => Some(Self::OpenAI_ImageEdit),
            Self::OpenAI_ImageVariation => Some(Self::OpenAI_ImageVariation),
            Self::OpenAI_AudioTTS => Some(Self::OpenAI_AudioTTS),
            Self::OpenAI_AudioTranscription => Some(Self::OpenAI_AudioTranscription),
            Self::OpenAI_AudioTranslation => Some(Self::OpenAI_AudioTranslation),
        }
    }

    fn is_anthropic(&self) -> bool {
        matches!(
            self,
            Self::Anthropic_TextChat | Self::Anthropic_TextCompletion
        )
    }

    fn as_anthropic(self) -> Option<Self> {
        match self {
            Self::Anthropic_TextChat | Self::OpenAI_TextChat => Some(Self::Anthropic_TextChat),
            Self::Anthropic_TextCompletion | Self::OpenAI_TextCompletion => {
                Some(Self::Anthropic_TextCompletion)
            }
            _ => None,
        }
    }
}

impl ModelRequest {
    pub(super) fn get_model(&self) -> Option<&str> {
        self.request.get_model()
    }

    pub(super) fn get_token_estimate(&self, model: &ModelConfig) -> (bool, u64) {
        let count = self.request.get_count();

        tracing::debug!(histogram.request.count = count);

        let max_tokens = self.request.get_max_tokens();

        if let Some(max_tokens) = max_tokens {
            tracing::debug!(histogram.request.max_tokens = max_tokens, unit = "tokens");
        }

        let max_tokens = max_tokens.unwrap_or(model.max_tokens);
        let is_oversized = max_tokens > model.max_tokens;

        (is_oversized, max_tokens * count as u64)
    }
}

#[derive(Debug)]
pub(super) struct ModelResponse {
    pub(super) status: StatusCode,
    pub(super) usage: TokenUsage,
    response: ModelResponseData,
}

#[derive(Debug)]
enum ModelResponseData {
    Json(Map<String, Value>),
    Binary(Vec<u8>),
}

impl ModelResponseData {
    #[tracing::instrument(level = "trace", ret)]
    fn convert_type(
        &mut self,
        request: &ModelRequest,
        tokenizer: &Option<TokenizerSettings>,
        tag: Uuid,
        fingerprint: Option<Uuid>,
        is_error: bool,
    ) -> TokenUsage {
        let desired_type = request.r#type;
        let model = request.get_model().map(|value| value.to_string());

        match self {
            Self::Json(json) => {
                if is_error {
                    if desired_type.is_anthropic() {
                        json.insert("type".to_string(), Value::String("error".to_string()));
                    }
                } else {
                    let response_type = match json
                        .get("object")
                        .or(json.get("type"))
                        .and_then(|value| value.as_str())
                    {
                        Some("chat.completion") => Some(RequestType::OpenAI_TextChat),
                        Some("message") => Some(RequestType::Anthropic_TextChat),
                        Some("text_completion") => Some(RequestType::OpenAI_TextCompletion),
                        Some("completion") => Some(RequestType::Anthropic_TextCompletion),
                        Some("edit") => Some(RequestType::OpenAI_TextEdit),
                        Some("list") => Some(RequestType::OpenAI_TextEmbedding),
                        Some(_) => None,
                        _ => None,
                    };

                    // TODO: Split these off as separate functions
                    match desired_type {
                        RequestType::OpenAI_TextChat => {
                            let is_supported_conversion = match response_type {
                                Some(RequestType::OpenAI_TextChat) => true,
                                Some(RequestType::OpenAI_TextCompletion) => {
                                    utility::update_object_array_in_place(
                                        json.get_mut("choices"),
                                        |object| {
                                            utility::update_item(
                                                object,
                                                "text",
                                                "message",
                                                |value| {
                                                    value.as_str().map(|text| {
                                                        json!({
                                                            "role": "assistant",
                                                            "content": text,
                                                        })
                                                    })
                                                },
                                            );
                                        },
                                    );

                                    true
                                }
                                Some(RequestType::Anthropic_TextChat) => {
                                    let finish_reason = json
                                        .get("stop_reason")
                                        .and_then(|value| value.as_str())
                                        .map(|stop_reason| {
                                            match stop_reason {
                                                "end_turn" | "stop_sequence" => "stop",
                                                "max_tokens" => "length",
                                                _ => stop_reason,
                                            }
                                            .to_string()
                                        });
                                    if finish_reason.is_some() {
                                        json.remove("stop_reason");
                                        json.remove("stop_sequence");
                                    }

                                    let role = json
                                        .get("role")
                                        .and_then(|value| value.as_str())
                                        .map(|text| text.to_string());
                                    if role.is_some() {
                                        json.remove("role");
                                    }

                                    utility::update_object_array_skipless(
                                        json,
                                        "content",
                                        "choices",
                                        |object| {
                                            if object.get("type").and_then(|value| value.as_str())
                                                == Some("text")
                                            {
                                                object.remove("type");

                                                utility::update_item(
                                                    object,
                                                    "text",
                                                    "message",
                                                    |value| {
                                                        value.as_str().map(|text| {
                                                            json!({
                                                                "role": role,
                                                                "content": text,
                                                            })
                                                        })
                                                    },
                                                );

                                                if let Some(finish_reason) = &finish_reason {
                                                    utility::insert_item(
                                                        object,
                                                        "finish_reason",
                                                        json!(finish_reason),
                                                    )
                                                }
                                            }
                                        },
                                    );

                                    json.remove("type");

                                    true
                                }
                                Some(RequestType::Anthropic_TextCompletion) => {
                                    let finish_reason = json
                                        .get("stop_reason")
                                        .and_then(|value| value.as_str())
                                        .map(|stop_reason| {
                                            match stop_reason {
                                                "end_turn" | "stop_sequence" => "stop",
                                                "max_tokens" => "length",
                                                _ => stop_reason,
                                            }
                                            .to_string()
                                        });
                                    if finish_reason.is_some() {
                                        json.remove("stop_reason");
                                    }

                                    utility::update_item(json, "completion", "choices", |value| {
                                        value.as_str().map(|text| {
                                            if let Some(finish_reason) = &finish_reason {
                                                json!([{
                                                    "message": {
                                                        "role": "assistant",
                                                        "content": text,
                                                    },
                                                    "finish_reason": finish_reason,
                                                }])
                                            } else {
                                                json!([{
                                                    "message": {
                                                        "role": "assistant",
                                                        "content": text,
                                                    },
                                                }])
                                            }
                                        })
                                    });

                                    json.remove("type");

                                    true
                                }
                                _ => false,
                            };

                            if is_supported_conversion {
                                utility::update_object_array_in_place(
                                    json.get_mut("choices"),
                                    |object| {
                                        utility::insert_item(object, "logprobs", Value::Null);
                                        utility::insert_item(
                                            object,
                                            "finish_reason",
                                            json!("stop"),
                                        );
                                    },
                                );

                                json.insert("created".to_string(), Value::Null);
                                json.insert("id".to_string(), Value::Null);
                                json.insert("model".to_string(), Value::Null);
                                if let Some(fingerprint) = fingerprint {
                                    utility::insert_item(
                                        json,
                                        "system_fingerprint",
                                        Value::String(format!("{}", fingerprint)),
                                    );
                                }
                                json.insert(
                                    "object".to_string(),
                                    Value::String("chat.completion".to_string()),
                                );
                            }
                        }
                        RequestType::Anthropic_TextChat => {
                            let is_supported_conversion = match response_type {
                                Some(RequestType::OpenAI_TextChat) => {
                                    json.remove("object");
                                    json.remove("created");
                                    json.remove("system_fingerprint");

                                    let mut stop_reason = None;

                                    utility::update_object_array_skipless(
                                        json,
                                        "choices",
                                        "content",
                                        |object| {
                                            if let Some(Value::String(finish_reason)) =
                                                object.get("finish_reason")
                                            {
                                                stop_reason = match finish_reason.as_ref() {
                                                    "length" => Some("max_tokens".to_string()),
                                                    "stop" | "tool_calls" | "function_call" => None,
                                                    _ => Some(finish_reason.to_string()),
                                                };
                                                object.remove("finish_reason");
                                            }

                                            utility::update_item(
                                                object,
                                                "message",
                                                "text",
                                                |value| {
                                                    value
                                                        .as_object()
                                                        .filter(|object| {
                                                            object
                                                                .get("role")
                                                                .and_then(|value| value.as_str())
                                                                == Some("assistant")
                                                        })
                                                        .and_then(|object| {
                                                            object
                                                                .get("content")
                                                                .and_then(|value| value.as_str())
                                                                .map(|text| {
                                                                    json!({
                                                                        "type": "text",
                                                                        "text": text,
                                                                    })
                                                                })
                                                        })
                                                },
                                            );

                                            if object.contains_key("text") {
                                                object.insert("type".to_string(), json!("text"));
                                            }

                                            object.remove("logprobs");
                                        },
                                    );

                                    if let Some(stop_reason) = stop_reason {
                                        utility::insert_item(
                                            json,
                                            "stop_reason",
                                            json!(stop_reason),
                                        )
                                    }

                                    true
                                }
                                Some(RequestType::OpenAI_TextCompletion) => {
                                    json.remove("object");
                                    json.remove("created");
                                    json.remove("system_fingerprint");

                                    let mut stop_reason = None;

                                    utility::update_object_array_skipless(
                                        json,
                                        "choices",
                                        "content",
                                        |object| {
                                            if let Some(Value::String(finish_reason)) =
                                                object.get("finish_reason")
                                            {
                                                stop_reason = match finish_reason.as_ref() {
                                                    "length" => Some("max_tokens".to_string()),
                                                    "stop" | "tool_calls" | "function_call" => None,
                                                    _ => Some(finish_reason.to_string()),
                                                };
                                                object.remove("finish_reason");
                                            }

                                            if object.contains_key("text") {
                                                object.insert("type".to_string(), json!("text"));
                                            }

                                            object.remove("logprobs");
                                        },
                                    );

                                    if let Some(stop_reason) = stop_reason {
                                        utility::insert_item(
                                            json,
                                            "stop_reason",
                                            json!(stop_reason),
                                        )
                                    }

                                    true
                                }
                                Some(RequestType::Anthropic_TextChat) => true,
                                Some(RequestType::Anthropic_TextCompletion) => {
                                    utility::update_item(json, "completion", "content", |value| {
                                        value.as_str().map(|text| {
                                            json!([{
                                                "type": "text",
                                                "text": text,
                                            }])
                                        })
                                    });

                                    utility::update_item_in_place(
                                        json.get_mut("stop_reason"),
                                        |value| {
                                            if let Value::String(text) = value {
                                                *text = match text.as_ref() {
                                                    "stop_sequence" => "end_turn",
                                                    "max_tokens" => "max_tokens",
                                                    _ => text,
                                                }
                                                .to_string()
                                            }
                                        },
                                    );

                                    true
                                }
                                _ => false,
                            };

                            if is_supported_conversion {
                                utility::insert_item(json, "stop_reason", json!("end_turn"));
                                utility::insert_item(json, "role", json!("assistant"));
                                json.insert("id".to_string(), Value::Null);
                                json.insert("model".to_string(), Value::Null);
                                json.insert(
                                    "type".to_string(),
                                    Value::String("message".to_string()),
                                );
                            }
                        }
                        RequestType::OpenAI_TextCompletion => {
                            let is_supported_conversion = match response_type {
                                Some(RequestType::OpenAI_TextChat) => {
                                    utility::update_object_array_in_place(
                                        json.get_mut("choices"),
                                        |object| {
                                            utility::update_item(
                                                object,
                                                "message",
                                                "text",
                                                |value| {
                                                    value
                                                        .as_object()
                                                        .filter(|object| {
                                                            object
                                                                .get("role")
                                                                .and_then(|value| value.as_str())
                                                                == Some("assistant")
                                                        })
                                                        .and_then(|object| object.get("content"))
                                                        .and_then(|value| value.as_str())
                                                        .map(|text| json!(text))
                                                },
                                            )
                                        },
                                    );

                                    true
                                }
                                Some(RequestType::OpenAI_TextCompletion) => true,
                                Some(RequestType::Anthropic_TextChat) => {
                                    let finish_reason = json
                                        .get("stop_reason")
                                        .and_then(|value| value.as_str())
                                        .map(|stop_reason| {
                                            match stop_reason {
                                                "end_turn" | "stop_sequence" => "stop",
                                                "max_tokens" => "length",
                                                _ => stop_reason,
                                            }
                                            .to_string()
                                        });
                                    if finish_reason.is_some() {
                                        json.remove("stop_reason");
                                        json.remove("stop_sequence");
                                    }

                                    utility::update_object_array_skipless(
                                        json,
                                        "content",
                                        "choices",
                                        |object| {
                                            if object.get("type").and_then(|value| value.as_str())
                                                == Some("text")
                                            {
                                                if let Some(finish_reason) = &finish_reason {
                                                    utility::insert_item(
                                                        object,
                                                        "finish_reason",
                                                        json!(finish_reason),
                                                    );
                                                }
                                            }
                                        },
                                    );

                                    json.remove("role");
                                    json.remove("type");

                                    true
                                }
                                Some(RequestType::Anthropic_TextCompletion) => {
                                    let finish_reason = json
                                        .get("stop_reason")
                                        .and_then(|value| value.as_str())
                                        .map(|stop_reason| {
                                            match stop_reason {
                                                "end_turn" | "stop_sequence" => "stop",
                                                "max_tokens" => "length",
                                                _ => stop_reason,
                                            }
                                            .to_string()
                                        });
                                    if finish_reason.is_some() {
                                        json.remove("stop_reason");
                                    }

                                    utility::update_item(json, "completion", "choices", |value| {
                                        value.as_str().map(|text| {
                                            if let Some(finish_reason) = &finish_reason {
                                                json!([{
                                                    "text": text,
                                                    "finish_reason": finish_reason,
                                                }])
                                            } else {
                                                json!([{
                                                    "text": text,
                                                }])
                                            }
                                        })
                                    });

                                    json.remove("type");

                                    true
                                }
                                _ => false,
                            };

                            if is_supported_conversion {
                                utility::update_object_array_in_place(
                                    json.get_mut("choices"),
                                    |object| {
                                        utility::insert_item(object, "logprobs", Value::Null);
                                        utility::insert_item(
                                            object,
                                            "finish_reason",
                                            json!("stop"),
                                        );
                                    },
                                );

                                json.insert("created".to_string(), Value::Null);
                                json.insert("id".to_string(), Value::Null);
                                json.insert("model".to_string(), Value::Null);
                                if let Some(fingerprint) = fingerprint {
                                    utility::insert_item(
                                        json,
                                        "system_fingerprint",
                                        Value::String(format!("{}", fingerprint)),
                                    );
                                }
                                json.insert("object".to_string(), json!("text_completion"));
                            }
                        }
                        RequestType::Anthropic_TextCompletion => {
                            let is_supported_conversion = match response_type {
                                Some(RequestType::OpenAI_TextChat) => {
                                    json.remove("object");
                                    json.remove("created");
                                    json.remove("system_fingerprint");
                                    json.remove("usage");

                                    utility::update_item_in_place(
                                        json.get_mut("choices"),
                                        |value| {
                                            if let Value::Array(objects) = value {
                                                objects.sort_by_key(|value| {
                                                    value
                                                        .get("index")
                                                        .and_then(|v| v.as_i64())
                                                        .unwrap_or(i64::MAX)
                                                });
                                            }
                                        },
                                    );

                                    let mut stop_reason = None;

                                    utility::update_array_single(
                                        json,
                                        "choices",
                                        "completion",
                                        |value| {
                                            value.as_object().and_then(|object| {
                                                if let Some(Value::String(finish_reason)) =
                                                    object.get("finish_reason")
                                                {
                                                    stop_reason = match finish_reason.as_ref() {
                                                        "length" => Some("max_tokens".to_string()),
                                                        "stop" | "tool_calls" | "function_call" => {
                                                            None
                                                        }
                                                        _ => Some(finish_reason.to_string()),
                                                    };
                                                }

                                                object
                                                    .get("message")
                                                    .and_then(|value| value.as_object())
                                                    .filter(|object| {
                                                        object
                                                            .get("role")
                                                            .and_then(|value| value.as_str())
                                                            == Some("assistant")
                                                    })
                                                    .and_then(|object| {
                                                        object
                                                            .get("content")
                                                            .and_then(|value| value.as_str())
                                                            .map(|text| text.to_string())
                                                            .map(Value::String)
                                                    })
                                            })
                                        },
                                    );

                                    if let Some(stop_reason) = stop_reason {
                                        utility::insert_item(
                                            json,
                                            "stop_reason",
                                            json!(stop_reason),
                                        )
                                    }

                                    true
                                }
                                Some(RequestType::OpenAI_TextCompletion) => {
                                    json.remove("object");
                                    json.remove("created");
                                    json.remove("system_fingerprint");
                                    json.remove("usage");

                                    utility::update_item_in_place(
                                        json.get_mut("choices"),
                                        |value| {
                                            if let Value::Array(objects) = value {
                                                objects.sort_by_key(|value| {
                                                    value
                                                        .get("index")
                                                        .and_then(|v| v.as_i64())
                                                        .unwrap_or(i64::MAX)
                                                });
                                            }
                                        },
                                    );

                                    let mut stop_reason = None;

                                    utility::update_array_single(
                                        json,
                                        "choices",
                                        "completion",
                                        |value| {
                                            value.as_object().and_then(|object| {
                                                if let Some(Value::String(finish_reason)) =
                                                    object.get("finish_reason")
                                                {
                                                    stop_reason = match finish_reason.as_ref() {
                                                        "length" => Some("max_tokens".to_string()),
                                                        "stop" | "tool_calls" | "function_call" => {
                                                            None
                                                        }
                                                        _ => Some(finish_reason.to_string()),
                                                    };
                                                }

                                                object
                                                    .get("text")
                                                    .and_then(|value| value.as_str())
                                                    .map(|text| text.to_string())
                                                    .map(Value::String)
                                            })
                                        },
                                    );

                                    if let Some(stop_reason) = stop_reason {
                                        utility::insert_item(
                                            json,
                                            "stop_reason",
                                            json!(stop_reason),
                                        )
                                    }

                                    true
                                }
                                Some(RequestType::Anthropic_TextChat) => {
                                    utility::update_array_single(
                                        json,
                                        "content",
                                        "completion",
                                        |value| {
                                            value
                                                .as_object()
                                                .filter(|object| {
                                                    object
                                                        .get("type")
                                                        .and_then(|value| value.as_str())
                                                        == Some("text")
                                                })
                                                .and_then(|object| {
                                                    object
                                                        .get("text")
                                                        .and_then(|value| value.as_str())
                                                        .map(|text| text.to_string())
                                                        .map(Value::String)
                                                })
                                        },
                                    );

                                    utility::update_item_in_place(
                                        json.get_mut("stop_reason"),
                                        |value| {
                                            if let Value::String(text) = value {
                                                *text = match text.as_ref() {
                                                    "end_turn" | "stop_sequence" => "stop_sequence",
                                                    "max_tokens" => "max_tokens",
                                                    _ => text,
                                                }
                                                .to_string()
                                            }
                                        },
                                    );

                                    json.remove("role");
                                    json.remove("stop_sequence");
                                    json.remove("usage");

                                    true
                                }
                                Some(RequestType::Anthropic_TextCompletion) => true,
                                _ => false,
                            };

                            if is_supported_conversion {
                                utility::insert_item(json, "stop_reason", json!("stop_sequence"));
                                json.insert("id".to_string(), Value::Null);
                                json.insert("model".to_string(), Value::Null);
                                json.insert("type".to_string(), json!("completion"));
                            }
                        }
                        RequestType::OpenAI_TextEdit => {
                            if response_type == Some(RequestType::OpenAI_TextEdit) {
                                json.insert("created".to_string(), Value::Null);
                            }
                        }
                        RequestType::OpenAI_TextEmbedding => {
                            if response_type == Some(RequestType::OpenAI_TextEmbedding) {
                                json.insert("model".to_string(), Value::Null);
                            }
                        }
                        _ => {}
                    }
                }

                for (_, value) in json.iter_mut() {
                    if let Value::Array(objects) = value {
                        objects.sort_by_key(|value| {
                            value
                                .get("index")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(i64::MAX)
                        });

                        if desired_type.is_openai() {
                            for (index, value) in objects.iter_mut().enumerate() {
                                if let Value::Object(object) = value {
                                    if !object.contains_key("index") {
                                        object.insert(
                                            "index".to_string(),
                                            Value::Number(index.into()),
                                        );
                                    }
                                }
                            }

                            objects.sort_by_key(|value| {
                                value
                                    .get("index")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(i64::MAX)
                            });
                        }
                    }
                }

                if let Some(value) = json.get_mut("model") {
                    *value = model.map(Value::String).unwrap_or(Value::Null);
                }

                if let Some(value) = json.get_mut("id") {
                    *value = Value::String(format!("{}", tag));
                }

                if let Some(value) = json.get_mut("created") {
                    *value = Value::Number(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs()
                            .into(),
                    );
                }

                if let Some(Value::Object(object)) = json.get("usage") {
                    let input = object
                        .get("input_tokens")
                        .or(object.get("prompt_tokens"))
                        .and_then(|num| num.as_u64())
                        .or(tokenizer.as_ref().and_then(|tokenizer| {
                            if let ModelRequestData::Json(json) = &request.request {
                                get_token_count(json, tokenizer)
                                    .and_then(|tokens| tokens.try_into().ok())
                            } else {
                                None
                            }
                        }));
                    let output = object
                        .get("output_tokens")
                        .or(object.get("completion_tokens"))
                        .and_then(|num| num.as_u64())
                        .or(tokenizer.as_ref().and_then(|tokenizer| {
                            get_token_count(json, tokenizer)
                                .and_then(|tokens| tokens.try_into().ok())
                        }));
                    let total = object
                        .get("total_tokens")
                        .and_then(|num| num.as_u64())
                        .unwrap_or((input.unwrap_or_default() + output.unwrap_or_default()).max(1));

                    TokenUsage {
                        total,
                        input,
                        output,
                    }
                } else {
                    TokenUsage {
                        total: match is_error {
                            true => 0,
                            false => request.request.get_count().try_into().unwrap_or(u64::MAX),
                        },
                        input: None,
                        output: None,
                    }
                }
            }
            Self::Binary(binary) => match is_error {
                true => {
                    if let Ok(message) = String::from_utf8(binary.clone()) {
                        if desired_type.is_openai() {
                            *self = Self::Json(
                                json!({
                                    "error": {
                                        "message": message,
                                        "type": Value::Null,
                                        "param": Value::Null,
                                        "code": Value::Null,
                                    }
                                })
                                .as_object()
                                .unwrap()
                                .clone(),
                            );
                        }

                        if desired_type.is_anthropic() {
                            *self = Self::Json(
                                json!({
                                    "type": "error",
                                    "error": {
                                        "message": message,
                                        "type": "invalid_request_error",
                                    }
                                })
                                .as_object()
                                .unwrap()
                                .clone(),
                            );
                        }
                    }

                    TokenUsage::default()
                }
                false => TokenUsage {
                    total: request.request.get_count().try_into().unwrap_or(u64::MAX),
                    input: None,
                    output: None,
                },
            },
        }
    }
}

impl From<ModelError> for ModelResponse {
    fn from(value: ModelError) -> Self {
        let message = match value {
            ModelError::BadRequest => "We could not parse the JSON body of your request. (HINT: This likely means you aren't using your HTTP library correctly. The API expects a JSON payload, but what was sent was not valid JSON. If you have trouble figuring out how to fix this, contact the proxy's administrator.)",
            ModelError::AuthMissing => "You didn't provide an API key. You need to provide your API key in an Authorization header using Bearer auth (i.e. Authorization: Bearer YOUR_KEY), or as the password field (with blank username) if you're accessing the API from your browser and are prompted for a username and password. You can obtain an API key from the proxy's administrator.",
            ModelError::AuthInvalid => "Incorrect API key provided. You can obtain an API key from the proxy's administrator.",
            ModelError::UserRateLimit => "You exceeded your current quota, please check your API key's rate limits. For more information on this error, contact the proxy's administrator.",
            ModelError::ModelRateLimit => "That model is currently overloaded with other requests. You can retry your request, or contact the proxy's administrator if the error persists.",
            ModelError::UnknownEndpoint => "Unknown request URL. Please check the URL for typos, or contact the proxy's administrator for information regarding available endpoints.",
            ModelError::BadEndpointMethod => "Invalid request method. Please check the URL for typos, or contact the proxy's administrator for information regarding available endpoints.",
            ModelError::UnknownModel => "The requested model does not exist. Contact the proxy's administrator for more information.",
            ModelError::InternalError => "The proxy server had an error processing your request. Sorry about that! You can retry your request, or contact the proxy's administrator if the error persists.",
            ModelError::BackendError => "The model had an error processing your request. Sorry about that! Contact the proxy's administrator for more information.",
        };
        let error_type = match value {
            ModelError::BadRequest => "invalid_request_error",
            ModelError::AuthMissing => "invalid_request_error",
            ModelError::AuthInvalid => "invalid_request_error",
            ModelError::UserRateLimit => "insufficient_quota",
            ModelError::ModelRateLimit => "server_error",
            ModelError::UnknownEndpoint => "invalid_request_error",
            ModelError::BadEndpointMethod => "invalid_request_error",
            ModelError::UnknownModel => "invalid_request_error",
            ModelError::InternalError => "server_error",
            ModelError::BackendError => "server_error",
        };
        let error_code = match value {
            ModelError::BadRequest => Value::Null,
            ModelError::AuthMissing => Value::Null,
            ModelError::AuthInvalid => Value::String("invalid_api_key".to_string()),
            ModelError::UserRateLimit => Value::String("insufficient_quota".to_string()),
            ModelError::ModelRateLimit => Value::Null,
            ModelError::UnknownEndpoint => Value::String("unknown_url".to_string()),
            ModelError::BadEndpointMethod => Value::Null,
            ModelError::UnknownModel => Value::String("model_not_found".to_string()),
            ModelError::InternalError => Value::Null,
            ModelError::BackendError => Value::Null,
        };
        let error_param = match value {
            ModelError::UnknownModel => Value::String("model".to_string()),
            _ => Value::Null,
        };

        let status = match value {
            ModelError::BadRequest => StatusCode::BAD_REQUEST,
            ModelError::AuthMissing => StatusCode::UNAUTHORIZED,
            ModelError::AuthInvalid => StatusCode::UNAUTHORIZED,
            ModelError::UserRateLimit => StatusCode::TOO_MANY_REQUESTS,
            ModelError::ModelRateLimit => StatusCode::SERVICE_UNAVAILABLE,
            ModelError::UnknownEndpoint => StatusCode::NOT_FOUND,
            ModelError::BadEndpointMethod => StatusCode::METHOD_NOT_ALLOWED,
            ModelError::UnknownModel => StatusCode::NOT_FOUND,
            ModelError::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
            ModelError::BackendError => StatusCode::BAD_GATEWAY,
        };

        ModelResponse {
            usage: TokenUsage::default(),
            status,
            response: ModelResponseData::Json(
                json!({
                    "type": "error",
                    "error": {
                        "message": message,
                        "type": error_type,
                        "param": error_param,
                        "code": error_code,
                    }
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        }
    }
}

#[derive(Debug, Default)]
#[allow(dead_code)]
pub(super) struct TokenUsage {
    pub(super) total: u64,
    pub(super) input: Option<u64>,
    pub(super) output: Option<u64>,
}

#[derive(Debug)]
pub(super) enum ModelError {
    BadRequest,
    AuthMissing,
    AuthInvalid,
    UserRateLimit,
    ModelRateLimit,
    UnknownEndpoint,
    BadEndpointMethod,
    UnknownModel,
    InternalError,
    BackendError,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct ModelConfig {
    tokenizer: Option<TokenizerSettings>,
    pub max_tokens: u64,
    system_fingerprint: Option<Uuid>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[allow(private_interfaces)]
pub(super) enum ModelBackend {
    OpenAI(OpenAIModelBackend),
    Loopback,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OpenAIModelBackend {
    model_string: String,
    openai_api_base: String,
    openai_api_key: String,
    openai_organization: Option<String>,
}

impl ModelBackend {
    #[tracing::instrument(skip(self, http_client, base_config), level = "debug", ret)]
    pub(super) async fn generate(
        &self,
        http_client: &Client,
        base_config: ModelConfig,
        mut request: ModelRequest,
    ) -> ModelResponse {
        let tag = Uuid::new_v4();
        tracing::debug!(tag = ?tag);

        match &self {
            Self::OpenAI(config) => match config.get_endpoint(request.r#type) {
                Some((endpoint)) => {
                    if let Some(r#type) = request.r#type.as_openai() {
                        request.request.convert_type(
                            request.r#type,
                            r#type,
                            config.model_string.clone(),
                            request.user,
                        );
                    }

                    let requests = vec![&request.request];

                    match client::send_http_request(http_client, &endpoint, &requests).await {
                        Ok((status, mut data)) => {
                            let usage = data.convert_type(
                                &request,
                                &base_config.tokenizer,
                                tag,
                                base_config.system_fingerprint,
                                !status.is_success(),
                            );

                            ModelResponse {
                                status,
                                usage,
                                response: data,
                            }
                        }
                        Err(error) => ModelResponse::from(error),
                    }
                }
                None => ModelResponse::from(ModelError::InternalError),
            },
            Self::Loopback => request.request.into_loopback(),
        }
    }
}
