use crate::types::{CreateResponseRequest, InputItem, ResponseInput, ResponseObject};
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

mod local;

pub use local::LocalResponseStore;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("response not found")]
    NotFound,

    #[error("store internal error: {0}")]
    Internal(String),
}

#[async_trait]
pub trait ResponseStore: Send + Sync {
    async fn put(&self, response: &ResponseObject, ttl: Option<Duration>) -> Result<(), StoreError>;
    async fn get(&self, id: &str) -> Result<Option<ResponseObject>, StoreError>;
    async fn delete(&self, id: &str) -> Result<bool, StoreError>;
    async fn append_input_chain(
        &self,
        id: &str,
        request: &CreateResponseRequest,
    ) -> Result<(), StoreError>;
    async fn rebuild_input(&self, id: &str) -> Result<Vec<InputItem>, StoreError>;
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseStoreConfig {
    Local {
        ttl_seconds: Option<u64>,
        max_entries: Option<usize>,
    },
}

impl Default for ResponseStoreConfig {
    fn default() -> Self {
        Self::Local {
            ttl_seconds: Some(24 * 60 * 60),
            max_entries: Some(10_000),
        }
    }
}

impl ResponseStoreConfig {
    pub async fn create(&self) -> Arc<dyn ResponseStore> {
        match self {
            Self::Local { max_entries, .. } => {
                Arc::new(LocalResponseStore::new(max_entries.unwrap_or(10_000)))
            }
        }
    }

    pub fn ttl(&self) -> Option<Duration> {
        match self {
            Self::Local { ttl_seconds, .. } => ttl_seconds.map(|seconds| Duration::from_secs(seconds)),
        }
    }
}

pub(crate) fn request_input_items(request: &CreateResponseRequest) -> Vec<InputItem> {
    match &request.input {
        ResponseInput::Text(text) => vec![InputItem::Message(crate::types::InputMessage {
            role: "user".to_string(),
            content: crate::types::MessageContentInput::Text(text.clone()),
        })],
        ResponseInput::Items(items) => items.clone(),
    }
}
