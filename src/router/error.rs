use async_openai::error::ApiError;
use serde_json::value::Value;

use super::ModelResponse;

impl ModelResponse {
    // Status code 400
    pub fn error_failed_parse() -> Self {
        Self::Error(ApiError {
            message: "We could not parse the JSON body of your request. (HINT: This likely means you aren't using your HTTP library correctly. The OpenAI API expects a JSON payload, but what was sent was not valid JSON. If you have trouble figuring out how to fix this, please contact the proxy's administrator.)".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        })
    }

    // Status code 401
    pub fn error_auth_missing() -> Self {
        Self::Error(ApiError {
            message: "You didn't provide an API key. You need to provide your API key in an Authorization header using Bearer auth (i.e. Authorization: Bearer YOUR_KEY), or as the password field (with blank username) if you're accessing the API from your browser and are prompted for a username and password. You can obtain an API key from the proxy's administrator.".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        })
    }

    // Status code 401
    pub fn error_auth_incorrect() -> Self {
        Self::Error(ApiError {
            message: "Incorrect API key provided. You can obtain an API key from the proxy's administrator.".to_string(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("invalid_api_key".to_string())),
        })
    }

    // Status code 404
    pub fn error_not_found(model: &str) -> Self {
        Self::Error(ApiError {
            message: ["The model `", model, "` does not exist."].concat(),
            r#type: Some("invalid_request_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("model_not_found".to_string())),
        })
    }

    // Status code 429
    pub fn error_user_rate_limit() -> Self {
        Self::Error(ApiError {
            message: "You exceeded your current quota, please check your API key's rate limits. For more information on this error, contact the proxy's administrator.".to_string(),
            r#type: Some("insufficient_quota".to_string()),
            param: Some(Value::Null),
            code: Some(Value::String("insufficient_quota".to_string())),
        })
    }

    // Status code 500
    pub fn error_internal() -> Self {
        Self::Error(ApiError {
            message: "The proxy server had an error processing your request. Sorry about that! You can retry your request, or contact the proxy's administrator if the error persists.".to_string(),
            r#type: Some("server_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        })
    }

    // Status code 503
    pub fn error_internal_rate_limit() -> Self {
        Self::Error(ApiError {
            message: "That model is currently overloaded with other requests. You can retry your request, or contact the proxy's administrator if the error persists.".to_string(),
            r#type: Some("server_error".to_string()),
            param: Some(Value::Null),
            code: Some(Value::Null),
        })
    }

    // add get_status_code()
}