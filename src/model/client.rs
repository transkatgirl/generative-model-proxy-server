use std::{collections::HashMap, fmt::Debug};

use fast32::base32::{CROCKFORD, RFC4648};
use http::{status::StatusCode, Uri};
use reqwest::{
    header::HeaderMap,
    multipart::{Form, Part},
    Client, Method, Request, RequestBuilder, Response, Url,
};
use ring::digest;
use serde::{Deserialize, Serialize};
use serde_json::{value::Value, Map};
use tracing::{debug_span, instrument, Instrument};
use uuid::Uuid;

use super::{
    ModelError, ModelFormItem, ModelRequest, ModelRequestData, ModelResponse, ModelResponseData,
};

/*

! Need to redo telemetry

- Use debug level for things which don't contain sensitive info and should show up in release builds
- Use trace level for things that are useful for debug but not release builds; May contain sensitive info

- Make use of logging when useful

See:
- https://opentelemetry.io/docs/specs/semconv/http/http-spans/#http-client
  - https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/index.html
- https://opentelemetry.io/docs/specs/semconv/http/http-metrics/#http-client
  - https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/struct.MetricsLayer.html

ModelRequest / ModelResponse specific attributes worth logging:
- usage
- request_count
- max_tokens

*/

impl ModelRequest {
    #[tracing::instrument(skip(base), level = "trace")]
    fn to_http_body(self, base: RequestBuilder) -> RequestBuilder {
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
    }
}

impl ModelResponse {
    #[instrument(level = "trace", ret)]
    fn from_http_body(status: StatusCode, body: &Vec<u8>, binary: bool) -> ModelResponse {
        if status.is_server_error() {
            tracing::warn!("Backend returned {} error: {:?}", status, body);
            return ModelResponse::from(ModelError::BackendError);
        }

        if status.is_client_error() {
            if status == StatusCode::UNAUTHORIZED
                || status == StatusCode::FORBIDDEN
                || status == StatusCode::PROXY_AUTHENTICATION_REQUIRED
            {
                tracing::warn!("Failed to authenticate with backend: {:?}", body);
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
                tracing::warn!("Backend returned {} error: {:?}", status, body);
                return ModelResponse::from(ModelError::BackendError);
            }

            if status == StatusCode::PAYMENT_REQUIRED || status == StatusCode::TOO_MANY_REQUESTS {
                tracing::warn!("Request was rate-limited by backend: {:?}", body);
                return ModelResponse::from(ModelError::ModelRateLimit);
            }
        }

        match serde_json::from_slice::<Map<String, Value>>(&body) {
            Ok(json) => {
                let response = ModelResponseData::Json(json);

                ModelResponse {
                    status,
                    usage: response.get_usage(status.is_client_error()),
                    response,
                }
            }
            Err(error) => {
                if binary {
                    let response = ModelResponseData::Binary(body.to_vec());

                    ModelResponse {
                        status,
                        usage: response.get_usage(status.is_client_error()),
                        response,
                    }
                } else {
                    tracing::warn!("Error parsing response: {:?}", error);
                    ModelResponse::from(ModelError::BackendError)
                }
            }
        }
    }
}

#[tracing::instrument(level = "trace")]
pub(super) async fn send_http_request(
    client: &Client,
    method: Method,
    url: Url,
    headers: HeaderMap,
    request: ModelRequest,
    binary: bool,
) -> ModelResponse {
    let http_request = request.to_http_body(client.request(method, url).headers(headers));

    match http_request.send().await {
        Ok(http_response) => {
            let status = StatusCode::from_u16(http_response.status().as_u16()).unwrap();
            let body = http_response.bytes().await;

            match body {
                Ok(body) => ModelResponse::from_http_body(status, &body.to_vec(), binary),
                Err(error) => {
                    tracing::warn!("Error receiving response: {:?}", error);

                    ModelResponse::from(ModelError::BackendError)
                }
            }
        }
        Err(error) => {
            tracing::warn!("Error sending request: {:?}", error);

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
