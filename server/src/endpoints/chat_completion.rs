use crate::error::ErrorResponse;
use crate::{ApiKey, AppConfig};
use actix_web::web::{Bytes, Data, Json};
use actix_web::{HttpResponse, post};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::Utc;
use futures::StreamExt;
use llm_selector::genai::Client;
use llm_selector::genai::chat::{
    ChatMessage, ChatOptions, ChatRequest, ChatRole, ChatStreamEvent, ContentPart, MessageContent,
    Tool, ToolCall, ToolResponse,
};
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
    #[serde(default)]
    stream: Option<bool>,
    #[serde(default)]
    tools: Option<Vec<ToolDefinition>>,
    #[serde(default)]
    #[allow(dead_code)]
    tool_choice: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolDefinition {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    tool_type: String,
    function: FunctionDefinition,
}

#[derive(Debug, Clone, Deserialize)]
struct FunctionDefinition {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    parameters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
enum Message {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_calls: Option<Vec<ResponseToolCall>>,
    },
    Tool {
        content: String,
        tool_call_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResponseToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: ResponseFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResponseFunction {
    name: String,
    arguments: String,
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

#[derive(Serialize)]
struct ChatCompletionChunkResponse {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<ChunkChoice>,
    ayatori_client_id: String,
}

#[derive(Serialize)]
struct ChunkChoice {
    index: u32,
    delta: Delta,
    finish_reason: Option<String>,
}

#[derive(Serialize)]
struct Delta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Serialize)]
struct DeltaToolCall {
    index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    call_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function: Option<DeltaFunction>,
}

#[derive(Serialize)]
struct DeltaFunction {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    arguments: Option<String>,
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
            Message::Assistant {
                content,
                tool_calls,
            } => {
                if let Some(ref text) = content {
                    token_count += token_measure.measure_token(&id, text).await.unwrap_or(0);
                }
                if let Some(ref tcs) = tool_calls {
                    let genai_tool_calls: Vec<ToolCall> = tcs
                        .iter()
                        .map(|tc| ToolCall {
                            call_id: tc.id.clone(),
                            fn_name: tc.function.name.clone(),
                            fn_arguments: serde_json::from_str(&tc.function.arguments).unwrap_or(
                                serde_json::Value::String(tc.function.arguments.clone()),
                            ),
                            thought_signatures: None,
                        })
                        .collect();
                    let mut msg_content = MessageContent::from_tool_calls(genai_tool_calls);
                    if let Some(text) = content {
                        msg_content.prepend(ContentPart::from_text(text));
                    }
                    messages.push(ChatMessage {
                        role: ChatRole::Assistant,
                        content: msg_content,
                        options: None,
                    });
                } else {
                    messages.push(ChatMessage {
                        role: ChatRole::Assistant,
                        content: MessageContent::from_text(content.unwrap_or_default()),
                        options: None,
                    });
                }
            }
            Message::Tool {
                content,
                tool_call_id,
            } => {
                token_count += token_measure
                    .measure_token(&id, &content)
                    .await
                    .unwrap_or(0);
                messages.push(ChatMessage::from(ToolResponse::new(tool_call_id, content)));
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

    let tools: Option<Vec<Tool>> = request.tools.as_ref().map(|defs| {
        defs.iter()
            .map(|td| {
                let mut tool = Tool::new(&td.function.name);
                if let Some(desc) = &td.function.description {
                    tool = tool.with_description(desc);
                }
                if let Some(params) = &td.function.parameters {
                    tool = tool.with_schema(params.clone());
                }
                tool
            })
            .collect()
    });

    let chat_req = ChatRequest {
        system,
        messages,
        tools,
    };

    let chat_options = ChatOptions {
        temperature: request.temperature,
        max_tokens: request.max_tokens.or(request.max_completion_tokens),
        top_p: request.top_p,
        stop_sequences: vec![],
        ..Default::default()
    };

    if request.stream.unwrap_or(false) {
        handle_streaming(client, chat_req, chat_options, selector, id, usage).await
    } else {
        handle_non_streaming(client, chat_req, chat_options, selector, id, usage).await
    }
}

async fn handle_non_streaming(
    client: Client,
    chat_req: ChatRequest,
    chat_options: ChatOptions,
    selector: Data<LlmSelector>,
    id: String,
    usage: Usage,
) -> HttpResponse {
    let res = client.exec_chat("", chat_req, Some(&chat_options)).await;

    if let Err(e) = selector.remove_usage(&id, &usage).await {
        error!("Remove usage failed: {e:?}");
    }

    match res {
        Ok(res) => {
            let tool_calls_raw = res.content.tool_calls();
            let (finish_reason, message) = if !tool_calls_raw.is_empty() {
                let response_tool_calls: Vec<ResponseToolCall> = tool_calls_raw
                    .iter()
                    .map(|tc| ResponseToolCall {
                        id: tc.call_id.clone(),
                        call_type: "function".to_string(),
                        function: ResponseFunction {
                            name: tc.fn_name.clone(),
                            arguments: tc.fn_arguments.to_string(),
                        },
                    })
                    .collect();
                let text_content = res.content.first_text().map(|s| s.to_string());
                (
                    "tool_calls",
                    Message::Assistant {
                        content: text_content,
                        tool_calls: Some(response_tool_calls),
                    },
                )
            } else {
                (
                    "stop",
                    Message::Assistant {
                        content: Some(res.content.joined_texts().unwrap_or_default()),
                        tool_calls: None,
                    },
                )
            };

            let response = ChatCompletionResponse {
                id: format!("ayatori-{}", BASE64_URL_SAFE_NO_PAD.encode(Uuid::new_v4())),
                created: Utc::now().timestamp(),
                model: res.provider_model_iden.model_name.to_string(),
                object: "chat.completion".to_string(),
                service_tier: "default".to_string(),
                choices: vec![MessageChoice {
                    index: 0,
                    finish_reason: finish_reason.to_string(),
                    message,
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

async fn handle_streaming(
    client: Client,
    chat_req: ChatRequest,
    chat_options: ChatOptions,
    selector: Data<LlmSelector>,
    id: String,
    usage: Usage,
) -> HttpResponse {
    let stream_res = client
        .exec_chat_stream("", chat_req, Some(&chat_options))
        .await;

    let stream_res = match stream_res {
        Ok(r) => r,
        Err(e) => {
            if let Err(e) = selector.remove_usage(&id, &usage).await {
                error!("Remove usage failed: {e:?}");
            }
            return ErrorResponse::from(e).into();
        }
    };

    let model_name = stream_res.model_iden.model_name.to_string();
    let response_id = format!("ayatori-{}", BASE64_URL_SAFE_NO_PAD.encode(Uuid::new_v4()));
    let created = Utc::now().timestamp();

    let selector_clone = selector.into_inner();
    let id_clone = id.clone();

    let mut has_tool_calls = false;
    let mut tool_call_index: u32 = 0;

    let sse_stream = stream_res.stream.map(move |event| {
        let event = match event {
            Ok(e) => e,
            Err(e) => {
                error!("Stream error: {e:?}");
                return Ok::<Bytes, actix_web::Error>(Bytes::new());
            }
        };

        match event {
            ChatStreamEvent::Start => {
                let chunk = ChatCompletionChunkResponse {
                    id: response_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model_name.clone(),
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: Delta {
                            role: Some("assistant".to_string()),
                            content: None,
                            tool_calls: None,
                        },
                        finish_reason: None,
                    }],
                    ayatori_client_id: id_clone.clone(),
                };
                let json = serde_json::to_string(&chunk).unwrap_or_default();
                Ok(Bytes::from(format!("data: {json}\n\n")))
            }
            ChatStreamEvent::Chunk(c) => {
                let chunk = ChatCompletionChunkResponse {
                    id: response_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model_name.clone(),
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: Delta {
                            role: None,
                            content: Some(c.content),
                            tool_calls: None,
                        },
                        finish_reason: None,
                    }],
                    ayatori_client_id: id_clone.clone(),
                };
                let json = serde_json::to_string(&chunk).unwrap_or_default();
                Ok(Bytes::from(format!("data: {json}\n\n")))
            }
            ChatStreamEvent::ToolCallChunk(tc) => {
                has_tool_calls = true;
                let idx = tool_call_index;
                tool_call_index += 1;

                let chunk = ChatCompletionChunkResponse {
                    id: response_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model_name.clone(),
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: Delta {
                            role: None,
                            content: None,
                            tool_calls: Some(vec![DeltaToolCall {
                                index: idx,
                                id: Some(tc.tool_call.call_id.clone()),
                                call_type: Some("function".to_string()),
                                function: Some(DeltaFunction {
                                    name: Some(tc.tool_call.fn_name.clone()),
                                    arguments: Some(tc.tool_call.fn_arguments.to_string()),
                                }),
                            }]),
                        },
                        finish_reason: None,
                    }],
                    ayatori_client_id: id_clone.clone(),
                };
                let json = serde_json::to_string(&chunk).unwrap_or_default();
                Ok(Bytes::from(format!("data: {json}\n\n")))
            }
            ChatStreamEvent::End(_) => {
                let selector = selector_clone.clone();
                let id = id_clone.clone();
                let usage = usage.clone();
                tokio::spawn(async move {
                    if let Err(e) = selector.remove_usage(&id, &usage).await {
                        error!("Remove usage failed: {e:?}");
                    }
                });

                let finish_reason = if has_tool_calls {
                    "tool_calls".to_string()
                } else {
                    "stop".to_string()
                };

                let chunk = ChatCompletionChunkResponse {
                    id: response_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model_name.clone(),
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: Delta {
                            role: None,
                            content: None,
                            tool_calls: None,
                        },
                        finish_reason: Some(finish_reason),
                    }],
                    ayatori_client_id: id_clone.clone(),
                };
                let json = serde_json::to_string(&chunk).unwrap_or_default();
                Ok(Bytes::from(format!("data: {json}\n\ndata: [DONE]\n\n")))
            }
            _ => Ok(Bytes::new()),
        }
    });

    HttpResponse::Ok()
        .content_type("text/event-stream")
        .streaming(sse_stream)
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
