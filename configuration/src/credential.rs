use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Credential {
    Azure {
        api_key: String,
        deployment: String,
        #[serde(default)]
        api_version: Option<String>,
    },
    Bedrock {
        api_key: String,
    },
    Anthropic {
        api_key: String,
    },
    Ollama,
    OpenAI {
        api_key: String,
    },
    VertexAI {
        api_key: String,
    },
}
