use actix_web::HttpResponse;
use actix_web::http::StatusCode;
use llm_selector::genai;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct ErrorResponse {
    #[serde(skip)]
    status: StatusCode,
    error: ApiError,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
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
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR),
                error: ApiError::chat_response(canonical_reason),
            },
        }
    }
}
