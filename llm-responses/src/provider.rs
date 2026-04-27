use crate::ResponsesError;
use crate::types::{CreateResponseRequest, ResponseObject, ResponseStreamEvent};
use async_trait::async_trait;
use futures::stream::BoxStream;

#[async_trait]
pub trait ResponsesProvider: Send + Sync {
    async fn create_response(
        &self,
        request: CreateResponseRequest,
    ) -> Result<ResponseObject, ResponsesError>;

    async fn create_response_stream(
        &self,
        _request: CreateResponseRequest,
    ) -> Result<BoxStream<'static, Result<ResponseStreamEvent, ResponsesError>>, ResponsesError>
    {
        Err(ResponsesError::Unsupported("streaming"))
    }

    async fn get_response(&self, _id: &str) -> Result<ResponseObject, ResponsesError> {
        Err(ResponsesError::Unsupported("get"))
    }

    async fn delete_response(&self, _id: &str) -> Result<(), ResponsesError> {
        Err(ResponsesError::Unsupported("delete"))
    }

    async fn cancel_response(&self, _id: &str) -> Result<ResponseObject, ResponsesError> {
        Err(ResponsesError::Unsupported("cancel"))
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProviderCapabilities {
    pub responses_native: bool,
    pub builtin_tools: bool,
    pub reasoning: bool,
    pub image_input: bool,
    pub structured_output: bool,
    pub streaming: bool,
    pub get_response: bool,
    pub cancel_response: bool,
}
