pub(super) use tiktoken_rs::tokenizer::Tokenizer;
use tiktoken_rs::{model, tokenizer};
use tracing::{span, Level};

use crate::api;

impl api::Model {
    #[tracing::instrument(level = "trace")]
    pub(super) fn get_tokenizer(&self) -> Option<Tokenizer> {
        match self.metadata.tokenizer.as_deref() {
            Some("cl100k_base") => Some(Tokenizer::Cl100kBase),
            Some("p50k_base") => Some(Tokenizer::P50kBase),
            Some("p50k_edit") => Some(Tokenizer::P50kEdit),
            Some("r50k_base") => Some(Tokenizer::R50kBase),
            Some("gpt2") => Some(Tokenizer::Gpt2),
            _ => tokenizer::get_tokenizer(&self.label),
        }
    }

    #[tracing::instrument(level = "trace")]
    pub(super) fn get_context_len(&self) -> usize {
        match self.metadata.context_len {
            Some(len) => len,
            None => model::get_context_size(&self.label),
        }
    }
}

pub(super) fn get_token_count<T: AsRef<str>>(tokenizer: Tokenizer, text: &[T]) -> u32 {
    let span = span!(Level::TRACE, "get_token_count");
    let _handle = span.enter();

    let bpe_arc = match tokenizer {
        Tokenizer::Cl100kBase => tiktoken_rs::cl100k_base_singleton(),
        Tokenizer::P50kBase => tiktoken_rs::p50k_base_singleton(),
        Tokenizer::P50kEdit => tiktoken_rs::p50k_edit_singleton(),
        Tokenizer::R50kBase | Tokenizer::Gpt2 => tiktoken_rs::r50k_base_singleton(),
    };

    let mut num_tokens = 0;
    let bpe = bpe_arc.lock();
    for item in text {
        num_tokens += bpe.encode_with_special_tokens(item.as_ref()).len();
    }

    num_tokens as u32
}
