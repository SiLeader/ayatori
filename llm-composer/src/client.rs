use configuration::{Credential, LlmProvider, LlmProviderType};
use genai::adapter::AdapterKind;
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget};
use std::collections::HashMap;

pub(crate) fn create_client(
    provider: LlmProvider,
    credentials: &HashMap<String, Credential>,
) -> Client {
    let credential = credentials
        .get(&provider.id)
        .expect("Credential not found")
        .clone();
    Client::builder()
        .with_service_target_resolver(create_service_target_resolver(provider, credential))
        .build()
}

fn create_service_target_resolver(
    provider: LlmProvider,
    credential: Credential,
) -> ServiceTargetResolver {
    let auth = match credential {
        Credential::Azure { api_key, .. } => AuthData::from_single(api_key),
        Credential::Bedrock { api_key, .. } => AuthData::from_single(api_key),
        Credential::Anthropic { api_key, .. } => AuthData::from_single(api_key),
        Credential::Ollama => AuthData::from_single(""),
        Credential::OpenAI { api_key, .. } => AuthData::from_single(api_key),
        Credential::VertexAI { api_key, .. } => AuthData::from_single(api_key),
    };
    ServiceTargetResolver::from_resolver_fn(
        move |_service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
            let endpoint = Endpoint::from_owned(provider.endpoint);

            let model = ModelIden::new(
                match provider.provider_type {
                    LlmProviderType::Azure => AdapterKind::OpenAI,
                    LlmProviderType::Bedrock => AdapterKind::OpenAI,
                    LlmProviderType::Anthropic => AdapterKind::Anthropic,
                    LlmProviderType::Ollama => AdapterKind::Ollama,
                    LlmProviderType::OpenAI => AdapterKind::OpenAI,
                    LlmProviderType::VertexAI => AdapterKind::Gemini,
                },
                provider.model,
            );
            Ok(ServiceTarget {
                endpoint,
                auth: auth.clone(),
                model,
            })
        },
    )
}
