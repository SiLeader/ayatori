mod credential;

pub use crate::credential::Credential;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::read_to_string;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configuration {
    pub providers: Vec<LlmProvider>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProvider {
    pub id: String,
    pub default: Option<bool>,
    #[serde(rename = "type")]
    pub provider_type: LlmProviderType,
    #[serde(default)]
    pub responses_native: Option<bool>,
    pub priority: usize,
    pub model: String,
    pub tags: Vec<String>,
    pub credential_file: String,
    pub endpoint: String,
    pub capacity: CapacityLimits,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityLimits {
    pub input_tokens: Option<u64>,
    pub requests: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmProviderType {
    Azure,
    Bedrock,
    Anthropic,
    Ollama,
    OpenAI,
    VertexAI,
}

impl Configuration {
    pub fn load_credential_files(&self) -> HashMap<String, Credential> {
        let mut credentials = HashMap::new();
        for provider in &self.providers {
            let content = read_to_string(&provider.credential_file).expect("Unable to read file");
            let creds: HashMap<String, Credential> =
                toml::from_str(&content).expect("Unable to parse file");
            credentials.extend(creds);
        }

        credentials
    }
}
