mod chat_completion;

use crate::endpoints::chat_completion::handle_chat_completion;
use actix_web::HttpResponse;
use actix_web::web::ServiceConfig;
use llm_selector::genai;
use serde::Serialize;

pub(crate) fn register_endpoints(config: &mut ServiceConfig) {
    config.service(handle_chat_completion);
}

#[derive(Debug, Serialize)]
#[serde(tag = "code")]
enum ErrorResponse {
    InvalidApiKey,
    ModelNotFound,
    Internal,
    NoMessages,
    LastMessageNotUser,
    UnsupportedRole,
    UnsupportedContentType,
    NoChatResponse,
    ApiKeyNotProvided,
    AuthenticationFailed,
}

impl From<ErrorResponse> for HttpResponse {
    fn from(value: ErrorResponse) -> Self {
        match value {
            ErrorResponse::InvalidApiKey => HttpResponse::Unauthorized().json(value),
            ErrorResponse::ModelNotFound => HttpResponse::NotFound().json(value),
            ErrorResponse::Internal => HttpResponse::InternalServerError().json(value),
            ErrorResponse::NoMessages => HttpResponse::BadRequest().json(value),
            ErrorResponse::LastMessageNotUser => HttpResponse::BadRequest().json(value),
            ErrorResponse::UnsupportedRole => HttpResponse::BadRequest().json(value),
            ErrorResponse::UnsupportedContentType => HttpResponse::BadRequest().json(value),
            ErrorResponse::NoChatResponse => HttpResponse::InternalServerError().json(value),
            ErrorResponse::ApiKeyNotProvided => HttpResponse::ServiceUnavailable().json(value),
            ErrorResponse::AuthenticationFailed => HttpResponse::ServiceUnavailable().json(value),
        }
    }
}

impl From<genai::Error> for ErrorResponse {
    fn from(value: genai::Error) -> Self {
        match value {
            genai::Error::ChatReqHasNoMessages { .. } => ErrorResponse::NoMessages,
            genai::Error::LastChatMessageIsNotUser { .. } => ErrorResponse::LastMessageNotUser,
            genai::Error::MessageRoleNotSupported { .. } => ErrorResponse::UnsupportedRole,
            genai::Error::MessageContentTypeNotSupported { .. } => {
                ErrorResponse::UnsupportedContentType
            }
            genai::Error::NoChatResponse { .. } => ErrorResponse::NoChatResponse,
            genai::Error::RequiresApiKey { .. } => ErrorResponse::ApiKeyNotProvided,
            genai::Error::NoAuthResolver { .. } | genai::Error::NoAuthData { .. } => {
                ErrorResponse::AuthenticationFailed
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
            | genai::Error::EventSourceClone(_)
            | genai::Error::JsonValueExt(_)
            | genai::Error::ReqwestEventSource(_)
            | genai::Error::ServiceTierParsing { .. }
            | genai::Error::SerdeJson(_) => ErrorResponse::Internal,
        }
    }
}
