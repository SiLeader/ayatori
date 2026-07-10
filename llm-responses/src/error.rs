#[derive(Debug, thiserror::Error)]
pub enum ResponsesError {
    #[error("provider HTTP error: status={status} body={body}")]
    Http { status: u16, body: String },

    #[error("provider transport error: {0}")]
    Transport(#[from] reqwest::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("feature not supported by this provider: {0}")]
    Unsupported(&'static str),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("provider returned malformed response: {0}")]
    MalformedResponse(String),

    #[error("authentication missing or invalid")]
    Authentication,

    #[error("internal error: {0}")]
    Internal(String),
}
