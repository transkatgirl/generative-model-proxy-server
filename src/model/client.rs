use std::time::Instant;

use http::status::StatusCode;
use reqwest::{
    header::HeaderMap,
    multipart::{Form, Part},
    Client, Method, Request, RequestBuilder, Url, Version,
};
use serde_json::{value::Value, Map};

use super::{
    ModelError, ModelFormItem, ModelRequest, ModelRequestData, ModelResponse, ModelResponseData,
    TokenUsage,
};

impl ModelRequest {
    #[tracing::instrument(name = "serialize_model_request", level = "debug", skip_all)]
    fn to_http_body(self, base: RequestBuilder) -> reqwest::Result<Request> {
        match self.request {
            ModelRequestData::Json(json) => base.json(&json),
            ModelRequestData::Form(formdata) => {
                let mut form = Form::new();
                for (key, value) in formdata {
                    let key = key.clone();

                    form = match value {
                        ModelFormItem::Text(text) => form.text(key, text.clone()),
                        ModelFormItem::File(file) => {
                            let mut part = Part::bytes(file.data.clone());

                            if let Some(content_type) = &file.content_type {
                                part = match part.mime_str(content_type) {
                                    Ok(updated_part) => updated_part,
                                    Err(_) => Part::bytes(file.data.clone()),
                                };
                            }

                            if let Some(filename) = &file.file_name {
                                part = part.file_name(filename.clone());
                            }

                            form.part(key, part)
                        }
                    };
                }

                base.multipart(form)
            }
        }
        .build()
    }
}

impl ModelResponse {
    #[tracing::instrument(name = "deserialize_model_response", level = "debug", skip_all)]
    fn from_http_body(status: StatusCode, body: &Vec<u8>, binary: bool) -> ModelResponse {
        if status.is_server_error() {
            tracing::error!("Backend returned {} error: {:?}", status, body);
            return ModelResponse::from(ModelError::BackendError);
        }

        if status.is_client_error() {
            if status == StatusCode::UNAUTHORIZED
                || status == StatusCode::FORBIDDEN
                || status == StatusCode::PROXY_AUTHENTICATION_REQUIRED
            {
                tracing::error!("Failed to authenticate with backend: {:?}", body);
                return ModelResponse::from(ModelError::BackendError);
            }

            if status == StatusCode::NOT_FOUND
                || status == StatusCode::METHOD_NOT_ALLOWED
                || status == StatusCode::NOT_ACCEPTABLE
                || status == StatusCode::REQUEST_TIMEOUT
                || status == StatusCode::GONE
                || status == StatusCode::LENGTH_REQUIRED
                || status == StatusCode::URI_TOO_LONG
                || status == StatusCode::EXPECTATION_FAILED
                || status == StatusCode::MISDIRECTED_REQUEST
                || status == StatusCode::UPGRADE_REQUIRED
                || status == StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE
            {
                tracing::error!("Backend returned {} error: {:?}", status, body);
                return ModelResponse::from(ModelError::BackendError);
            }

            if status == StatusCode::PAYMENT_REQUIRED || status == StatusCode::TOO_MANY_REQUESTS {
                tracing::error!("Request was rate-limited by backend: {:?}", body);
                return ModelResponse::from(ModelError::ModelRateLimit);
            }
        }

        match serde_json::from_slice::<Map<String, Value>>(body) {
            Ok(json) => {
                let response = ModelResponseData::Json(json);

                ModelResponse {
                    status,
                    usage: TokenUsage::default(),
                    response,
                }
            }
            Err(error) => {
                if binary || status.is_client_error() {
                    let response = ModelResponseData::Binary(body.to_vec());

                    ModelResponse {
                        status,
                        usage: TokenUsage::default(),
                        response,
                    }
                } else {
                    tracing::error!("Error parsing response: {:?}", error);
                    ModelResponse::from(ModelError::BackendError)
                }
            }
        }
    }
}

#[tracing::instrument(level = "debug", fields(otel.name = format!("{} {}", method, url.as_str()), otel.kind = "Client", network.protocol.name = "http", network.protocol.version, server.address = url.authority(), server.port = url.port_or_known_default(), url.full = url.as_str(), url.scheme = url.scheme(), user_agent.original = "generative-model-proxy-server", http.request.method = method.as_str(), http.request.header.content_type, http.response.status_code, http.response.header.content_type), skip_all)]
pub(super) async fn send_http_request(
    client: &Client,
    method: Method,
    url: Url,
    headers: HeaderMap,
    request: ModelRequest,
    binary: bool,
) -> ModelResponse {
    let span = tracing::Span::current();

    match request.to_http_body(client.request(method, url).headers(headers)) {
        Ok(http_request) => {
            if let Some(content_type) = http_request
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
            {
                span.record("http.request.header.content_type", content_type);
            }
            tracing::debug!(
                histogram.http.client.request.body.size = http_request
                    .body()
                    .and_then(|body| body.as_bytes())
                    .map(|body| body.len())
                    .unwrap_or_default(),
                unit = "By"
            );

            let timestamp = Instant::now();
            match client.execute(http_request).await {
                Ok(http_response) => {
                    span.record(
                        "network.protocol.version",
                        match http_response.version() {
                            Version::HTTP_09 => Some("0.9"),
                            Version::HTTP_10 => Some("1.0"),
                            Version::HTTP_11 => Some("1.1"),
                            Version::HTTP_2 => Some("2"),
                            Version::HTTP_3 => Some("3"),
                            _ => None,
                        },
                    );
                    span.record("http.response.status_code", http_response.status().as_u16());
                    if let Some(content_type) = http_response
                        .headers()
                        .get("content-type")
                        .and_then(|value| value.to_str().ok())
                    {
                        span.record("http.response.header.content_type", content_type);
                    }

                    let status = StatusCode::from_u16(http_response.status().as_u16()).unwrap();
                    let body = http_response.bytes().await;

                    tracing::debug!(
                        histogram.http.client.request.duration = timestamp.elapsed().as_secs_f64(),
                        unit = "s"
                    );

                    match body {
                        Ok(body) => {
                            tracing::debug!(
                                histogram.http.client.response.body.size = body.len(),
                                unit = "By"
                            );

                            ModelResponse::from_http_body(status, &body.to_vec(), binary)
                        }
                        Err(error) => {
                            tracing::error!("Error receiving response: {:?}", error);

                            ModelResponse::from(ModelError::BackendError)
                        }
                    }
                }
                Err(error) => {
                    tracing::error!("Error sending request: {:?}", error);

                    if error.is_connect() | error.is_redirect() | error.is_decode() {
                        return ModelResponse::from(ModelError::BackendError);
                    }

                    if error.is_timeout() {
                        return ModelResponse::from(ModelError::ModelRateLimit);
                    }

                    ModelResponse::from(ModelError::InternalError)
                }
            }
        }
        Err(error) => {
            tracing::error!("Error building request: {:?}", error);
            ModelResponse::from(ModelError::InternalError)
        }
    }
}
