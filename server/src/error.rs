use actix_web::HttpResponse;
use actix_web::http::StatusCode;
use llm_responses::ResponsesError;
use llm_selector::genai;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub(crate) struct ErrorResponse {
    #[serde(skip)]
    status: StatusCode,
    error: ApiError,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::enum_variant_names)]
enum ErrorType {
    InvalidRequestError,
    AuthenticationError,
    // PermissionError,
    // RateLimitError,
    ApiError,
}

#[derive(Debug, Serialize)]
struct ApiError {
    message: String,
    #[serde(rename = "type")]
    error_type: ErrorType,
    param: Option<String>,
    code: String,
}

impl From<ErrorResponse> for HttpResponse {
    fn from(value: ErrorResponse) -> Self {
        HttpResponse::build(value.status).json(value)
    }
}

impl ErrorResponse {
    pub(crate) fn model_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            error: ApiError::model_not_found(),
        }
    }

    pub(crate) fn invalid_authentication() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            error: ApiError::invalid_authentication(),
        }
    }

    pub(crate) fn incorrect_api_key_provided() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            error: ApiError::incorrect_api_key_provided(),
        }
    }

    fn internal(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: ApiError::internal(message),
        }
    }

    pub(crate) fn feature_not_supported(feature: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: ApiError::feature_not_supported(feature),
        }
    }
}

impl ApiError {
    fn model_not_found() -> Self {
        Self {
            message: "Model not found".to_string(),
            error_type: ErrorType::InvalidRequestError,
            param: Some("model".to_string()),
            code: "model_not_found".to_string(),
        }
    }

    fn invalid_authentication() -> Self {
        Self {
            message: "Invalid authentication".to_string(),
            error_type: ErrorType::AuthenticationError,
            param: None,
            code: "invalid_authentication".to_string(),
        }
    }

    fn incorrect_api_key_provided() -> Self {
        Self {
            message: "Incorrect API key provided".to_string(),
            error_type: ErrorType::AuthenticationError,
            param: None,
            code: "incorrect_api_key_provided".to_string(),
        }
    }

    fn invalid_request(message: String) -> Self {
        Self {
            message,
            error_type: ErrorType::InvalidRequestError,
            param: None,
            code: "invalid_request".to_string(),
        }
    }

    fn feature_not_supported(feature: &str) -> Self {
        Self {
            message: format!("Feature not supported: {feature}"),
            error_type: ErrorType::InvalidRequestError,
            param: None,
            code: "feature_not_supported".to_string(),
        }
    }

    fn upstream(message: String) -> Self {
        Self {
            message,
            error_type: ErrorType::ApiError,
            param: None,
            code: "upstream_error".to_string(),
        }
    }

    fn internal(message: String) -> Self {
        Self {
            message,
            error_type: ErrorType::ApiError,
            param: None,
            code: "internal_error".to_string(),
        }
    }

    fn chat_request_has_no_messages() -> Self {
        Self {
            message: "Chat request has no messages".to_string(),
            error_type: ErrorType::InvalidRequestError,
            param: Some("messages".to_string()),
            code: "chat_request_has_no_messages".to_string(),
        }
    }

    fn last_chat_message_is_not_user() -> Self {
        Self {
            message: "Last chat message is not user".to_string(),
            error_type: ErrorType::InvalidRequestError,
            param: Some("messages".to_string()),
            code: "last_chat_message_is_not_user".to_string(),
        }
    }

    fn message_role_not_supported() -> Self {
        Self {
            message: "Message role not supported".to_string(),
            error_type: ErrorType::InvalidRequestError,
            param: Some("role".to_string()),
            code: "message_role_not_supported".to_string(),
        }
    }

    fn message_content_type_not_supported() -> Self {
        Self {
            message: "Message content type not supported".to_string(),
            error_type: ErrorType::InvalidRequestError,
            param: Some("content_type".to_string()),
            code: "message_content_type_not_supported".to_string(),
        }
    }

    fn no_chat_response() -> Self {
        Self {
            message: "No chat response".to_string(),
            error_type: ErrorType::ApiError,
            param: None,
            code: "no_chat_response".to_string(),
        }
    }

    fn chat_response(reason: String) -> Self {
        Self {
            message: reason,
            error_type: ErrorType::ApiError,
            param: None,
            code: "generate_chat_response_failed".to_string(),
        }
    }
}

impl From<genai::Error> for ErrorResponse {
    fn from(value: genai::Error) -> Self {
        match value {
            genai::Error::ChatReqHasNoMessages { .. } => ErrorResponse {
                status: StatusCode::BAD_REQUEST,
                error: ApiError::chat_request_has_no_messages(),
            },
            genai::Error::LastChatMessageIsNotUser { .. } => ErrorResponse {
                status: StatusCode::BAD_REQUEST,
                error: ApiError::last_chat_message_is_not_user(),
            },
            genai::Error::MessageRoleNotSupported { .. } => ErrorResponse {
                status: StatusCode::BAD_REQUEST,
                error: ApiError::message_role_not_supported(),
            },
            genai::Error::MessageContentTypeNotSupported { .. } => ErrorResponse {
                status: StatusCode::BAD_REQUEST,
                error: ApiError::message_content_type_not_supported(),
            },
            genai::Error::NoChatResponse { .. } => ErrorResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                error: ApiError::no_chat_response(),
            },
            genai::Error::RequiresApiKey { .. } => ErrorResponse::invalid_authentication(),
            genai::Error::NoAuthResolver { .. } | genai::Error::NoAuthData { .. } => {
                ErrorResponse::incorrect_api_key_provided()
            }
            genai::Error::JsonModeWithoutInstruction
            | genai::Error::VerbosityParsing { .. }
            | genai::Error::ReasoningParsingError { .. }
            | genai::Error::InvalidJsonResponseElement { .. }
            | genai::Error::ModelMapperFailed { .. }
            | genai::Error::WebAdapterCall { .. }
            | genai::Error::WebModelCall { .. }
            | genai::Error::ChatResponse { .. }
            | genai::Error::StreamParse { .. }
            | genai::Error::WebStream { .. }
            | genai::Error::Resolver { .. }
            | genai::Error::AdapterNotSupported { .. }
            | genai::Error::Internal(_)
            | genai::Error::JsonValueExt(_)
            | genai::Error::ServiceTierParsing { .. }
            | genai::Error::SerdeJson(_) => ErrorResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                error: ApiError::chat_response("Internal error".to_string()),
            },
            genai::Error::ChatResponseGeneration { cause, .. } => ErrorResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                error: ApiError::chat_response(cause),
            },
            genai::Error::HttpError {
                status,
                canonical_reason,
                ..
            } => ErrorResponse {
                status: StatusCode::from_u16(status.as_u16())
                    .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                error: ApiError::chat_response(canonical_reason),
            },
        }
    }
}

impl From<ResponsesError> for ErrorResponse {
    fn from(value: ResponsesError) -> Self {
        match value {
            ResponsesError::Authentication => Self::invalid_authentication(),
            ResponsesError::InvalidRequest(message) => Self {
                status: StatusCode::BAD_REQUEST,
                error: ApiError::invalid_request(message),
            },
            ResponsesError::Unsupported(feature) => Self::feature_not_supported(feature),
            ResponsesError::Http { status, body } => {
                let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
                if let Some(error) = parse_openai_error(&body) {
                    Self { status, error }
                } else {
                    Self {
                        status,
                        error: ApiError::upstream(body),
                    }
                }
            }
            ResponsesError::Transport(error) => Self::internal(format!("transport: {error}")),
            ResponsesError::Serde(error) => Self::internal(format!("serde: {error}")),
            ResponsesError::MalformedResponse(message) => {
                Self::internal(format!("malformed response: {message}"))
            }
            ResponsesError::Internal(message) => Self::internal(message),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiErrorEnvelope {
    error: OpenAiError,
}

#[derive(Debug, Deserialize)]
struct OpenAiError {
    message: String,
    #[serde(rename = "type")]
    error_type: String,
    param: Option<String>,
    code: Option<String>,
}

fn parse_openai_error(body: &str) -> Option<ApiError> {
    let envelope: OpenAiErrorEnvelope = serde_json::from_str(body).ok()?;
    let error_type = match envelope.error.error_type.as_str() {
        "invalid_request_error" => ErrorType::InvalidRequestError,
        "authentication_error" => ErrorType::AuthenticationError,
        "api_error" => ErrorType::ApiError,
        _ => return None,
    };

    Some(ApiError {
        message: envelope.error.message,
        error_type,
        param: envelope.error.param,
        code: envelope
            .error
            .code
            .unwrap_or_else(|| "upstream_error".to_string()),
    })
}
