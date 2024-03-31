use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) enum Tokenizer {
    Cl100kBase,
    P50kBase,
    P50kEdit,
    R50kBase,
    Gpt2,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(super) struct TokenizerSettings {
    tokenizer: Tokenizer,
    starting_tokens: Option<i64>,
    tokens_per_message: Option<i64>,
    tokens_per_name: Option<i64>,
}

pub(super) struct TokenizerMessage<'a> {
    pub(super) role: &'a str,
    pub(super) content: Option<&'a str>,
    pub(super) name: Option<&'a str>,
}

impl TokenizerSettings {
    pub(super) fn tokenize_text(&self, text: &str) -> Vec<usize> {
        let bpe_arc = match self.tokenizer {
            Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
            Tokenizer::P50kBase => tiktoken_rs::p50k_base_singleton(),
            Tokenizer::P50kEdit => tiktoken_rs::p50k_edit_singleton(),
            Tokenizer::R50kBase | Tokenizer::Gpt2 => tiktoken_rs::r50k_base_singleton(),
        };

        let bpe = bpe_arc.lock();
        bpe.encode_with_special_tokens(text)
    }

    pub(super) fn get_message_token_count(&self, messages: &[TokenizerMessage]) -> usize {
        let bpe_arc = match self.tokenizer {
            Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
            Tokenizer::P50kBase => tiktoken_rs::p50k_base_singleton(),
            Tokenizer::P50kEdit => tiktoken_rs::p50k_edit_singleton(),
            Tokenizer::R50kBase | Tokenizer::Gpt2 => tiktoken_rs::r50k_base_singleton(),
        };

        let bpe = bpe_arc.lock();

        let mut num_tokens = self.starting_tokens.unwrap_or(3);
        for message in messages {
            num_tokens += self.tokens_per_message.unwrap_or(4);
            num_tokens += bpe.encode_with_special_tokens(message.role).len() as i64;
            num_tokens += bpe
                .encode_with_special_tokens(message.content.unwrap_or_default())
                .len() as i64;
            if let Some(name) = message.name {
                num_tokens += bpe.encode_with_special_tokens(name).len() as i64;
                num_tokens += self.tokens_per_name.unwrap_or(1);
            }
        }

        num_tokens.clamp(usize::MIN as i64, usize::MAX.try_into().unwrap_or(i64::MAX)) as usize
    }
}
