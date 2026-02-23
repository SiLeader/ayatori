use crate::error::ErrorResponse;
use crate::{ApiKey, AppConfig};
use actix_web::web::{Data, Json};
use actix_web::{HttpResponse, post};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::Utc;
use llm_selector::genai::Client;
use llm_selector::genai::chat::{ChatMessage, ChatOptions, ChatRequest, ChatRole, MessageContent};
use llm_selector::{LlmSelector, Usage};
use serde::{Deserialize, Serialize};
use token_measure::{MeasureToken, TokenMeasure};
use tracing::error;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub(super) struct ChatCompletionRequest {
    messages: Vec<Message>,
    model: String,
    temperature: Option<f64>,
    max_tokens: Option<u32>,
    max_completion_tokens: Option<u32>,
    top_p: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
enum Message {
    System { content: String },
    User { content: String },
    Assistant { content: String },
}

#[derive(Debug, Serialize)]
struct ChatCompletionResponse {
    id: String,
    model: String,
    created: i64,
    object: String,
    service_tier: String,
    choices: Vec<MessageChoice>,
    usage: MessageUsage,
    ayatori_client_id: String,
}

#[derive(Debug, Serialize)]
struct MessageChoice {
    index: u32,
    finish_reason: String,
    message: Message,
}

#[derive(Debug, Serialize)]
struct MessageUsage {
    completion_tokens: i32,
    prompt_tokens: i32,
    total_tokens: i32,
}

#[post("/v1/chat/completions")]
pub(super) async fn handle_chat_completion(
    selector: Data<LlmSelector>,
    api_key: Data<ApiKey>,
    bearer_auth: Option<BearerAuth>,
    app_config: Data<AppConfig>,
    token_measure: Data<TokenMeasure>,
    request: Json<ChatCompletionRequest>,
) -> HttpResponse {
    if let Err(e) = api_key.check_api_key(bearer_auth) {
        return e.into();
    }
    let model = RequestModel::from(request.model.clone());
    let client = model.select_model(&selector).await;
    let (id, client) = match client {
        None => {
            if app_config.client_fallback_enabled {
                selector.get_default_client()
            } else {
                return ErrorResponse::model_not_found().into();
            }
        }
        Some(client) => client,
    };

    let mut system = None;
    let mut messages = vec![];
    let mut token_count = 0;

    for message in request.messages.clone() {
        match message {
            Message::System { content } => {
                token_count += token_measure
                    .measure_token(&id, &content)
                    .await
                    .unwrap_or(0);
                system = Some(content);
            }
            Message::User { content } => {
                token_count += token_measure
                    .measure_token(&id, &content)
                    .await
                    .unwrap_or(0);
                messages.push(ChatMessage {
                    role: ChatRole::User,
                    content: MessageContent::from_text(content),
                    options: None,
                })
            }
            Message::Assistant { content } => {
                token_count += token_measure
                    .measure_token(&id, &content)
                    .await
                    .unwrap_or(0);
                messages.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: MessageContent::from_text(content),
                    options: None,
                })
            }
        }
    }
    let usage = Usage {
        input_tokens: token_count,
        requests: 1,
    };

    if let Err(e) = selector.append_usage(&id, &usage).await {
        error!("Append usage failed: {e:?}");
    }

    let res = client
        .exec_chat(
            "",
            ChatRequest {
                system,
                messages,
                tools: None,
            },
            Some(&ChatOptions {
                temperature: request.temperature,
                max_tokens: request.max_tokens.or(request.max_completion_tokens),
                top_p: request.top_p,
                stop_sequences: vec![],
                ..Default::default()
            }),
        )
        .await;

    if let Err(e) = selector.remove_usage(&id, &usage).await {
        error!("Remove usage failed: {e:?}");
    }

    match res {
        Ok(res) => {
            let response = ChatCompletionResponse {
                id: format!("ayatori-{}", BASE64_URL_SAFE_NO_PAD.encode(Uuid::new_v4())),
                created: Utc::now().timestamp(),
                model: res.provider_model_iden.model_name.to_string(),
                object: "chat.completion".to_string(),
                service_tier: "default".to_string(),
                choices: vec![MessageChoice {
                    index: 0,
                    finish_reason: "stop".to_string(),
                    message: Message::Assistant {
                        content: res.content.joined_texts().unwrap_or_default(),
                    },
                }],
                usage: MessageUsage {
                    completion_tokens: res.usage.completion_tokens.unwrap_or_default(),
                    prompt_tokens: res.usage.prompt_tokens.unwrap_or_default(),
                    total_tokens: res.usage.total_tokens.unwrap_or_default(),
                },
                ayatori_client_id: id,
            };
            HttpResponse::Ok().json(response)
        }
        Err(e) => ErrorResponse::from(e).into(),
    }
}

enum RequestModel {
    Model(String),
    Tags(Vec<String>),
    Id(String),
}

impl RequestModel {
    async fn select_model(self, llm_selector: &LlmSelector) -> Option<(String, Client)> {
        match self {
            RequestModel::Model(model) => llm_selector.select_client_by_model(&model).await,
            RequestModel::Tags(tags) => llm_selector.select_client_by_tags(&tags).await,
            RequestModel::Id(id) => llm_selector.select_client_by_id(&id).await,
        }
    }
}

impl From<String> for RequestModel {
    fn from(value: String) -> Self {
        let Some((scheme, content)) = value.split_once(':') else {
            return Self::Model(value);
        };

        match scheme {
            "tags" | "tag" => {
                Self::Tags(content.split('&').map(|s| s.trim().to_string()).collect())
            }
            "id" => Self::Id(content.to_string()),
            _ => Self::Model(value),
        }
    }
}
