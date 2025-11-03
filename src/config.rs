use configuration::Configuration;
use llm_selector::{UsageStore, UsageStoreConfig};
use serde::Deserialize;
use std::fs::read_to_string;
use std::path::Path;
use std::sync::Arc;
use token_measure::TokenMeasureConfig;

#[derive(Debug, Deserialize)]
pub(crate) struct Config {
    llm_configuration: String,
    pub server: ServerConfig,
    usage_store: UsageStoreConfig,
    pub token_measure: TokenMeasureConfig,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ServerConfig {
    pub listen: String,
    pub tls: Option<TlsConfig>,
    pub api_key: Option<String>,
    pub api_key_file: Option<String>,
    pub client_fallback_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TlsConfig {
    pub private_key_file: String,
    pub certificate_chain_file: String,
}

impl Config {
    pub(crate) fn load(file: impl AsRef<Path>) -> Config {
        let content = read_to_string(file).expect("Failed to read config file");
        toml::from_str(&content).expect("Failed to parse config file")
    }

    pub(crate) fn load_configuration(&self) -> Configuration {
        let content =
            read_to_string(&self.llm_configuration).expect("Failed to read LLM configuration file");
        toml::from_str(&content).expect("Failed to parse LLM configuration file")
    }

    pub(crate) async fn load_usage_store(&self) -> Arc<dyn UsageStore> {
        self.usage_store.create().await
    }

    pub(crate) fn load_api_key(&self) -> Option<String> {
        if let Some(ak) = &self.server.api_key {
            return Some(ak.clone());
        }
        if let Some(akf) = &self.server.api_key_file {
            return Some(read_to_string(akf).expect("Failed to read API key file"));
        }
        None
    }
}
