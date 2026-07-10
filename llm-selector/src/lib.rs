use crate::tag_selector::{ModelTag, TagSelector};
use crate::usage::UsageSelector;
pub use crate::usage::*;
use configuration::Configuration;
use llm_composer::LlmComposer;
pub use llm_composer::genai;
use llm_responses::{LlmResponsesComposer, ResponsesProvider};
use std::sync::Arc;
mod tag_selector;
mod usage;

#[derive(Clone)]
pub struct LlmSelector {
    tag_selector: TagSelector,
    usage_selector: UsageSelector,
    composer: LlmComposer,
    responses_composer: LlmResponsesComposer,
}

impl LlmSelector {
    pub fn new(configuration: Configuration, store: Arc<dyn UsageStore>) -> Self {
        Self {
            usage_selector: UsageSelector::new(store, &configuration),
            tag_selector: TagSelector::new(configuration.clone()),
            composer: LlmComposer::new(configuration.clone()),
            responses_composer: LlmResponsesComposer::new(configuration),
        }
    }

    pub async fn select_client_by_model(&self, model: &str) -> Option<(String, genai::Client)> {
        self.composer.get_client_by_model(model)
    }

    pub async fn select_client_by_tags(
        &self,
        tags: Vec<String>,
        exclude_tags: Vec<String>,
    ) -> Option<(String, genai::Client)> {
        let tags = tags.into_iter().map(ModelTag::from).collect::<Vec<_>>();
        let exclude_tags = exclude_tags
            .into_iter()
            .map(ModelTag::from)
            .collect::<Vec<_>>();

        let client_ids = self
            .tag_selector
            .get_client_ids_by_tags(&tags, &exclude_tags)
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let client_id = match self.usage_selector.select_client(&client_ids).await {
            Ok(client_id) => client_id,
            Err(e) => {
                tracing::error!("Failed to select client: {e:?}");
                return None;
            }
        };

        client_id.and_then(|id| self.composer.get_client_by_id(&id))
    }

    pub async fn select_client_by_id(&self, id: &str) -> Option<(String, genai::Client)> {
        self.composer.get_client_by_id(id)
    }

    pub fn get_default_client(&self) -> (String, genai::Client) {
        self.composer.get_default_client()
    }

    pub async fn select_responses_provider_by_model(
        &self,
        model: &str,
    ) -> Option<(String, Arc<dyn ResponsesProvider>)> {
        self.responses_composer.get_by_model(model)
    }

    pub async fn select_responses_provider_by_tags(
        &self,
        tags: Vec<String>,
        exclude_tags: Vec<String>,
    ) -> Option<(String, Arc<dyn ResponsesProvider>)> {
        let tags = tags.into_iter().map(ModelTag::from).collect::<Vec<_>>();
        let exclude_tags = exclude_tags
            .into_iter()
            .map(ModelTag::from)
            .collect::<Vec<_>>();

        let client_ids = self
            .tag_selector
            .get_client_ids_by_tags(&tags, &exclude_tags)
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let client_id = match self.usage_selector.select_client(&client_ids).await {
            Ok(client_id) => client_id,
            Err(e) => {
                tracing::error!("Failed to select responses provider: {e:?}");
                return None;
            }
        };

        client_id.and_then(|id| self.responses_composer.get_by_id(&id))
    }

    pub async fn select_responses_provider_by_id(
        &self,
        id: &str,
    ) -> Option<(String, Arc<dyn ResponsesProvider>)> {
        self.responses_composer.get_by_id(id)
    }

    pub fn get_default_responses_provider(&self) -> (String, Arc<dyn ResponsesProvider>) {
        self.responses_composer.get_default()
    }

    pub async fn append_usage(&self, id: &str, usage: &Usage) -> Result<(), UsageError> {
        self.usage_selector.append_usage(id, usage).await
    }
    pub async fn remove_usage(&self, id: &str, usage: &Usage) -> Result<(), UsageError> {
        self.usage_selector.remove_usage(id, usage).await
    }
}
