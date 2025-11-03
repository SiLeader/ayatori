use crate::usage::{Usage, UsageError, UsageStore};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Debug, Default)]
pub struct LocalUsageStore {
    usages: Arc<RwLock<HashMap<String, Usage>>>,
}

#[async_trait::async_trait]
impl UsageStore for LocalUsageStore {
    async fn get_usage(&self, id: &str) -> Result<Usage, UsageError> {
        let us = self.usages.read().await;
        Ok(us.get(id).cloned().unwrap_or_default())
    }

    async fn append_usage(&self, id: &str, usage: &Usage) -> Result<(), UsageError> {
        let mut us = self.usages.write().await;
        us.entry(id.to_string())
            .and_modify(|u| {
                u.input_tokens += usage.input_tokens;
                u.requests += usage.requests;
            })
            .or_insert_with(|| usage.clone());
        Ok(())
    }

    async fn remove_usage(&self, id: &str, usage: &Usage) -> Result<(), UsageError> {
        let mut us = self.usages.write().await;
        us.entry(id.to_string()).and_modify(|u| {
            u.input_tokens -= usage.input_tokens;
            u.requests -= usage.requests;
        });
        Ok(())
    }
}
