use std::{clone::Clone, collections::HashMap};

use axum::{
    async_trait,
    body::{self, Bytes},
    extract::{FromRequest, Multipart, Request},
    response::IntoResponse,
    Form, Json,
};

use http::{header::CONTENT_TYPE, Method};

use super::{
    ModelError, ModelFormFile, ModelFormItem, ModelRequest, ModelRequestData, ModelResponse,
    ModelResponseData, RequestType,
};

#[async_trait]
impl<S> FromRequest<S> for ModelRequest
where
    Bytes: FromRequest<S>,
    S: Send + Sync,
{
    type Rejection = ModelError;

    #[tracing::instrument(name = "deserialize_model_request", level = "debug", skip(state), ret)]
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let r#type = match RequestType::try_from(req.uri()) {
            Ok(r#type) => r#type,
            Err(_) => return Err(ModelError::UnknownEndpoint),
        };

        if req.method() != Method::GET
            && req.method() != Method::HEAD
            && req.method() != Method::POST
        {
            return Err(ModelError::BadEndpointMethod);
        }

        match req
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|header_value| {
                header_value
                    .to_str()
                    .map(|header_string| header_string.to_ascii_lowercase())
                    .ok()
            })
            .as_deref()
        {
            Some("application/x-www-form-urlencoded") => Form::from_request(req, state)
                .await
                .map(|value| value.0)
                .ok()
                .map(|request| ModelRequest::from_json(r#type, request)),
            Some("multipart/form-data") => match Multipart::from_request(req, state).await {
                Ok(mut multipart) => {
                    let mut form = HashMap::new();

                    while let Ok(Some(field)) = multipart.next_field().await {
                        if let Some(name) = field.name() {
                            let name = name.to_string();
                            let content_type =
                                field.content_type().map(|string| string.to_string());
                            let file_name = field.file_name().map(|string| string.to_string());

                            if let Ok(data) = field.bytes().await {
                                let data = data.to_vec();

                                if file_name.is_some() {
                                    form.insert(
                                        name,
                                        ModelFormItem::File(ModelFormFile {
                                            file_name,
                                            content_type,
                                            data,
                                        }),
                                    );
                                } else if let Ok(string) = String::from_utf8(data.clone()) {
                                    form.insert(name, ModelFormItem::Text(string));
                                } else {
                                    form.insert(
                                        name,
                                        ModelFormItem::File(ModelFormFile {
                                            file_name,
                                            content_type,
                                            data,
                                        }),
                                    );
                                }
                            }
                        }
                    }

                    if !form.is_empty() {
                        Some(ModelRequest {
                            tags: Vec::new(),
                            r#type,
                            request: ModelRequestData::Form(form),
                        })
                    } else {
                        None
                    }
                }
                Err(_) => None,
            },
            Some("application/json") => Json::from_request(req, state)
                .await
                .map(|value| value.0)
                .ok()
                .map(|request| ModelRequest::from_json(r#type, request)),
            Some(_) => body::to_bytes(req.into_body(), usize::MAX)
                .await
                .ok()
                .and_then(|body| Json::from_bytes(body.as_ref()).map(|value| value.0).ok())
                .map(|request| ModelRequest::from_json(r#type, request)),
            None => if req.method() == Method::HEAD || req.method() == Method::GET {
                Form::from_request(req, state)
                    .await
                    .map(|value| value.0)
                    .ok()
            } else {
                body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .ok()
                    .and_then(|body| Json::from_bytes(body.as_ref()).map(|value| value.0).ok())
            }
            .map(|request| ModelRequest::from_json(r#type, request)),
        }
        .ok_or(ModelError::BadRequest)
    }
}

impl IntoResponse for ModelResponse {
    #[tracing::instrument(name = "serialize_model_response", level = "debug", ret)]
    fn into_response(self) -> axum::response::Response {
        match self.response {
            ModelResponseData::Json(json) => (self.status, Json(json)).into_response(),
            ModelResponseData::Binary(binary) => (self.status, binary).into_response(),
        }
    }
}

impl IntoResponse for ModelError {
    fn into_response(self) -> axum::response::Response {
        ModelResponse::from(self).into_response()
    }
}
