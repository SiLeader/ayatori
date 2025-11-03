mod client;

use configuration::Configuration;
use genai::Client;
use std::collections::HashMap;

pub use genai;

#[derive(Debug, Clone)]
pub struct LlmComposer {
    default_client_id: String,
    default_client: Client,
    clients: HashMap<String, Client>,
    client_id_by_model: HashMap<String, String>,
}

impl LlmComposer {
    pub fn new(configuration: Configuration) -> Self {
        let credentials = configuration.load_credential_files();
        let mut clients = HashMap::new();
        let mut client_id_by_model = HashMap::new();

        let mut default_client = None;
        let mut default_client_id = "".to_string();
        for provider in configuration.providers {
            let id = provider.id.clone();
            let is_default = provider.default.unwrap_or(false);
            client_id_by_model.insert(provider.model.clone(), id.clone());
            let client = client::create_client(provider, &credentials);
            if is_default {
                default_client = Some(client.clone());
                default_client_id = id.clone();
            }
            clients.insert(id, client);
        }

        Self {
            clients,
            default_client_id,
            default_client: default_client.expect("Least one provider must be default"),
            client_id_by_model,
        }
    }

    pub fn get_client_by_model(&self, model: &str) -> Option<(String, Client)> {
        self.client_id_by_model
            .get(model)
            .and_then(|id| self.get_client_by_id(id))
    }

    pub fn get_client_by_id(&self, id: &str) -> Option<(String, Client)> {
        Some((id.to_string(), self.clients.get(id)?.clone()))
    }

    pub fn get_default_client(&self) -> (String, Client) {
        (self.default_client_id.clone(), self.default_client.clone())
    }
}
