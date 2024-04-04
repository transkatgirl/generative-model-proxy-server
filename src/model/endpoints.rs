use http::Uri;
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    Method, Url,
};

use super::{client::HttpEndpoint, OpenAIModelBackend, RequestType};

impl TryFrom<&Uri> for RequestType {
    type Error = &'static str;

    fn try_from(value: &Uri) -> Result<Self, Self::Error> {
        match value.path() {
            "/v1/chat/completions" => Ok(RequestType::OpenAI_TextChat),
            "/v1/completions" => Ok(RequestType::OpenAI_TextCompletion),
            "/v1/edits" => Ok(RequestType::OpenAI_TextEdit),
            "/v1/embeddings" => Ok(RequestType::OpenAI_TextEmbedding),
            "/v1/moderations" => Ok(RequestType::OpenAI_TextModeration),
            "/v1/images/generations" => Ok(RequestType::OpenAI_ImageGeneration),
            "/v1/images/edits" => Ok(RequestType::OpenAI_ImageEdit),
            "/v1/images/variations" => Ok(RequestType::OpenAI_ImageVariation),
            "/v1/audio/speech" => Ok(RequestType::OpenAI_AudioTTS),
            "/v1/audio/transcriptions" => Ok(RequestType::OpenAI_AudioTranscription),
            "/v1/audio/translations" => Ok(RequestType::OpenAI_AudioTranslation),
            "/v1/messages" => Ok(RequestType::Anthropic_TextChat),
            "/v1/complete" => Ok(RequestType::Anthropic_TextCompletion),
            _ => Err("Invalid URI"),
        }
    }
}

impl OpenAIModelBackend {
    #[tracing::instrument(level = "trace")]
    pub(super) fn get_endpoint(&self, r#type: RequestType) -> Option<HttpEndpoint> {
        let url_suffix = match r#type {
            RequestType::OpenAI_TextChat => "/v1/chat/completions",
            RequestType::OpenAI_TextCompletion => "/v1/completions",
            RequestType::OpenAI_TextEdit => "/v1/edits",
            RequestType::OpenAI_TextEmbedding => "/v1/embeddings",
            RequestType::OpenAI_TextModeration => "/v1/moderations",
            RequestType::OpenAI_ImageGeneration => "/v1/images/generations",
            RequestType::OpenAI_ImageEdit => "/v1/images/edits",
            RequestType::OpenAI_ImageVariation => "/v1/images/variations",
            RequestType::OpenAI_AudioTTS => "/v1/audio/speech",
            RequestType::OpenAI_AudioTranscription => "/v1/audio/transcriptions",
            RequestType::OpenAI_AudioTranslation => "/v1/audio/translations",
            _ => return None,
        };

        match Url::parse(&self.openai_api_base).and_then(|base_url| base_url.join(url_suffix)) {
            Ok(url) => match HeaderValue::from_str(&format!("Bearer {}", self.openai_api_key)) {
                Ok(auth_header) => {
                    let mut headers = HeaderMap::new();
                    headers.insert(AUTHORIZATION, auth_header);

                    if let Some(organization) = self
                        .openai_organization
                        .as_ref()
                        .and_then(|value| value.parse::<HeaderValue>().ok())
                    {
                        headers.insert("OpenAI-Organization", organization);
                    }

                    Some(HttpEndpoint {
                        method: Method::POST,
                        url,
                        headers,
                        is_binary: r#type == RequestType::OpenAI_AudioTTS,
                    })
                }
                Err(error) => {
                    tracing::warn!("Unable to parse API key: {:?}", error);
                    None
                }
            },
            Err(error) => {
                tracing::warn!("Unable to parse model URL: {:?}", error);
                None
            }
        }
    }
}
