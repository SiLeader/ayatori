use crate::anthropic::AnthropicResponsesProvider;
use crate::azure::AzureOpenAiResponsesProvider;
use crate::genai::GenaiBackedProvider;
use crate::openai::OpenAiResponsesProvider;
use crate::vertexai::VertexAiResponsesProvider;
use crate::{ResponsesError, ResponsesProvider};
use configuration::{Configuration, Credential, LlmProvider, LlmProviderType};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct LlmResponsesComposer {
    providers: HashMap<String, Arc<dyn ResponsesProvider>>,
    default_id: String,
    model_to_id: HashMap<String, String>,
}

impl LlmResponsesComposer {
    pub fn new(configuration: Configuration) -> Self {
        let credentials = configuration.load_credential_files();
        let mut providers = HashMap::new();
        let mut model_to_id = HashMap::new();
        let mut default_id = String::new();

        for provider in configuration.providers {
            let id = provider.id.clone();
            if provider.default.unwrap_or(false) {
                default_id = id.clone();
            }
            model_to_id.insert(provider.model.clone(), id.clone());
            let credential = credentials.get(&id).cloned().expect("Credential not found");
            providers.insert(id, build_provider(provider, credential));
        }

        Self {
            providers,
            default_id,
            model_to_id,
        }
    }

    pub fn get_by_id(&self, id: &str) -> Option<(String, Arc<dyn ResponsesProvider>)> {
        self.providers.get(id).map(|p| (id.to_string(), p.clone()))
    }

    pub fn get_by_model(&self, model: &str) -> Option<(String, Arc<dyn ResponsesProvider>)> {
        let id = self.model_to_id.get(model)?;
        self.get_by_id(id)
    }

    pub fn get_default(&self) -> (String, Arc<dyn ResponsesProvider>) {
        self.get_by_id(&self.default_id)
            .expect("default provider missing")
    }
}

fn build_provider(provider: LlmProvider, credential: Credential) -> Arc<dyn ResponsesProvider> {
    let provider_type = provider.provider_type.clone();
    match provider_type {
        LlmProviderType::OpenAI => Arc::new(OpenAiResponsesProvider::new(provider, credential)),
        LlmProviderType::Azure => Arc::new(AzureOpenAiResponsesProvider::new(provider, credential)),
        LlmProviderType::Anthropic => {
            Arc::new(AnthropicResponsesProvider::new(provider, credential))
        }
        LlmProviderType::VertexAI => Arc::new(VertexAiResponsesProvider::new(provider, credential)),
        LlmProviderType::Ollama => Arc::new(
            GenaiBackedProvider::new(provider.clone(), credential)
                .with_model(provider.model.clone()),
        ),
        LlmProviderType::Bedrock => Arc::new(UnsupportedProvider::new(provider_type)),
    }
}

struct UnsupportedProvider {
    provider_type: LlmProviderType,
}

impl UnsupportedProvider {
    fn new(provider_type: LlmProviderType) -> Self {
        Self { provider_type }
    }
}

#[async_trait::async_trait]
impl ResponsesProvider for UnsupportedProvider {
    async fn create_response(
        &self,
        _request: crate::types::CreateResponseRequest,
    ) -> Result<crate::types::ResponseObject, ResponsesError> {
        let provider = match self.provider_type {
            LlmProviderType::Azure => "azure",
            LlmProviderType::Bedrock => "bedrock",
            LlmProviderType::Anthropic => "anthropic",
            LlmProviderType::Ollama => "ollama",
            LlmProviderType::OpenAI => "openai",
            LlmProviderType::VertexAI => "vertex_ai",
        };
        Err(ResponsesError::Unsupported(match provider {
            "azure" => "azure provider is not configured for responses",
            "bedrock" => "bedrock provider is not implemented yet",
            "anthropic" => "anthropic provider is not configured for responses",
            "ollama" => "ollama provider is not configured for responses",
            "openai" => "openai provider is not configured for responses",
            "vertex_ai" => "vertex_ai provider is not configured for responses",
            _ => "provider is not implemented yet",
        }))
    }
}
