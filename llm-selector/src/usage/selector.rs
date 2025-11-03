use crate::Usage;
use crate::usage::{UsageError, UsageStore};
use configuration::{CapacityLimits, Configuration};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct UsageSelector {
    usage_store: Arc<dyn UsageStore>,
    capacities: HashMap<String, CapacityLimits>,
}

impl UsageSelector {
    pub(crate) fn new(usage_store: Arc<dyn UsageStore>, configuration: &Configuration) -> Self {
        let capacities = configuration
            .providers
            .iter()
            .map(|l| (l.id.clone(), l.capacity.clone()))
            .collect();

        Self {
            usage_store,
            capacities,
        }
    }

    pub(crate) async fn select_client(
        &self,
        client_ids: &[String],
    ) -> Result<Option<String>, UsageError> {
        for id in client_ids {
            let Some(capacity) = self.capacities.get(id) else {
                continue;
            };

            let usage = self.usage_store.get_usage(id).await?;
            if !usage.is_reached(capacity) {
                return Ok(Some(id.clone()));
            }
        }

        Ok(None)
    }

    pub(crate) async fn append_usage(&self, id: &str, usage: &Usage) -> Result<(), UsageError> {
        self.usage_store.append_usage(id, usage).await
    }

    pub(crate) async fn remove_usage(&self, id: &str, usage: &Usage) -> Result<(), UsageError> {
        self.usage_store.remove_usage(id, usage).await
    }
}
