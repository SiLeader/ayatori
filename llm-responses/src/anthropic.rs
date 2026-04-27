use crate::common::{
    append_system_text, collect_text, function_call_output, function_tools,
    incomplete_for_max_tokens, input_items, make_response, parse_data_url_base64,
    parse_json_objectish, reasoning_budget, reasoning_output, tool_choice_mode, usage,
};
use crate::http::send_value;
use crate::types::{
    ContentPartInput, CreateResponseRequest, InputItem, MessageContentInput, ResponseObject,
    ToolChoice,
};
use crate::{ProviderCapabilities, ResponsesError, ResponsesProvider};
use async_trait::async_trait;
use configuration::{Credential, LlmProvider};
use serde_json::{Value, json};

const ANTHROPIC_VERSION: &str = "2023-06-01";

pub(crate) struct AnthropicResponsesProvider {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl AnthropicResponsesProvider {
    pub(crate) fn new(provider: LlmProvider, credential: Credential) -> Self {
        let api_key = match credential {
            Credential::Anthropic { api_key } => api_key,
            credential => panic!("unexpected credential for Anthropic provider: {credential:?}"),
        };

        Self {
            client: reqwest::Client::new(),
            endpoint: provider.endpoint,
            api_key,
            model: provider.model,
        }
    }
}

#[async_trait]
impl ResponsesProvider for AnthropicResponsesProvider {
    async fn create_response(
        &self,
        request: CreateResponseRequest,
    ) -> Result<ResponseObject, ResponsesError> {
        let payload = to_anthropic_request(&request, &self.model)?;
        let url = format!("{}/v1/messages", self.endpoint.trim_end_matches('/'));
        let body = send_value(
            self.client
                .post(url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION),
            &payload,
        )
        .await?;

        from_anthropic_response(&request, body)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            responses_native: false,
            builtin_tools: false,
            reasoning: true,
            image_input: true,
            structured_output: false,
            streaming: true,
            get_response: false,
            cancel_response: false,
        }
    }
}

fn to_anthropic_request(
    request: &CreateResponseRequest,
    model: &str,
) -> Result<Value, ResponsesError> {
    let mut system = request.instructions.clone();
    let mut messages: Vec<Value> = Vec::new();

    for item in input_items(&request.input) {
        match item {
            InputItem::Message(message)
                if matches!(message.role.as_str(), "system" | "developer") =>
            {
                append_system_text(&mut system, collect_text(&message.content));
            }
            InputItem::Message(message) => {
                let role = if message.role == "assistant" {
                    "assistant"
                } else {
                    "user"
                };
                let blocks = message_content_to_anthropic_blocks(&message.content)?;
                push_anthropic_message(&mut messages, role, blocks);
            }
            InputItem::FunctionCall(call) => {
                push_anthropic_message(
                    &mut messages,
                    "assistant",
                    vec![json!({
                        "type": "tool_use",
                        "id": call.call_id,
                        "name": call.name,
                        "input": parse_json_objectish(&call.arguments),
                    })],
                );
            }
            InputItem::FunctionCallOutput(output) => {
                push_anthropic_message(
                    &mut messages,
                    "user",
                    vec![json!({
                        "type": "tool_result",
                        "tool_use_id": output.call_id,
                        "content": output.output,
                    })],
                );
            }
            InputItem::Reasoning(reasoning) => {
                let text = reasoning
                    .summary
                    .into_iter()
                    .map(|part| match part {
                        crate::types::SummaryPart::Text { text } => text,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                if !text.is_empty() || reasoning.encrypted_content.is_some() {
                    push_anthropic_message(
                        &mut messages,
                        "assistant",
                        vec![json!({
                            "type": "thinking",
                            "thinking": text,
                            "signature": reasoning.encrypted_content,
                        })],
                    );
                }
            }
            InputItem::ItemReference { .. } => {
                return Err(ResponsesError::Unsupported("item_reference (anthropic)"));
            }
        }
    }

    let tools = tools_to_anthropic(request.tools.as_deref());
    let mut payload = json!({
        "model": model,
        "max_tokens": request.max_output_tokens.unwrap_or(4096),
        "messages": messages,
        "stream": false,
    });

    if let Some(system) = system {
        payload["system"] = json!(system);
    }
    if !tools.is_empty() {
        payload["tools"] = Value::Array(tools);
    }
    if let Some(tool_choice) = request
        .tool_choice
        .as_ref()
        .and_then(tool_choice_to_anthropic)
    {
        payload["tool_choice"] = tool_choice;
    }
    if let Some(temperature) = request.temperature {
        payload["temperature"] = json!(temperature);
    }
    if let Some(top_p) = request.top_p {
        payload["top_p"] = json!(top_p);
    }
    if let Some(budget_tokens) =
        reasoning_budget(request.max_output_tokens, request.reasoning.as_ref())
    {
        payload["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": budget_tokens,
        });
    }

    Ok(payload)
}

fn push_anthropic_message(messages: &mut Vec<Value>, role: &str, blocks: Vec<Value>) {
    if blocks.is_empty() {
        return;
    }

    if let Some(last) = messages.last_mut()
        && last.get("role").and_then(Value::as_str) == Some(role)
        && let Some(content) = last.get_mut("content").and_then(Value::as_array_mut)
    {
        content.extend(blocks);
        return;
    }

    messages.push(json!({
        "role": role,
        "content": blocks,
    }));
}

fn message_content_to_anthropic_blocks(
    content: &MessageContentInput,
) -> Result<Vec<Value>, ResponsesError> {
    match content {
        MessageContentInput::Text(text) => Ok(vec![json!({
            "type": "text",
            "text": text,
        })]),
        MessageContentInput::Parts(parts) => {
            let mut blocks = Vec::new();
            for part in parts {
                match part {
                    ContentPartInput::InputText { text } => blocks.push(json!({
                        "type": "text",
                        "text": text,
                    })),
                    ContentPartInput::InputImage {
                        image_url, file_id, ..
                    } => {
                        if file_id.is_some() {
                            return Err(ResponsesError::Unsupported(
                                "input_image.file_id (anthropic)",
                            ));
                        }
                        let image_url = image_url.as_deref().ok_or_else(|| {
                            ResponsesError::InvalidRequest(
                                "input_image.image_url is required".to_string(),
                            )
                        })?;
                        if let Some((media_type, data)) = parse_data_url_base64(image_url) {
                            blocks.push(json!({
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": media_type,
                                    "data": data,
                                }
                            }));
                        } else {
                            blocks.push(json!({
                                "type": "image",
                                "source": {
                                    "type": "url",
                                    "url": image_url,
                                }
                            }));
                        }
                    }
                    ContentPartInput::OutputText { text, .. } => blocks.push(json!({
                        "type": "text",
                        "text": text,
                    })),
                    ContentPartInput::Refusal { refusal } => blocks.push(json!({
                        "type": "text",
                        "text": refusal,
                    })),
                    ContentPartInput::InputFile { .. } => {
                        return Err(ResponsesError::Unsupported("input_file (anthropic)"));
                    }
                }
            }
            Ok(blocks)
        }
    }
}

fn tools_to_anthropic(tools: Option<&[crate::types::ToolDefinition]>) -> Vec<Value> {
    function_tools(tools)
        .into_iter()
        .map(|(name, description, parameters)| {
            json!({
                "name": name,
                "description": description,
                "input_schema": parameters.cloned().unwrap_or_else(|| json!({"type": "object"})),
            })
        })
        .collect()
}

fn tool_choice_to_anthropic(choice: &ToolChoice) -> Option<Value> {
    if let Some(mode) = tool_choice_mode(choice) {
        return Some(match mode {
            "required" => json!({ "type": "any" }),
            "auto" => json!({ "type": "auto" }),
            "none" => json!({ "type": "none" }),
            _ => json!({ "type": mode }),
        });
    }

    match choice {
        ToolChoice::Specific {
            tool_type,
            name: Some(name),
        } if tool_type == "function" => Some(json!({
            "type": "tool",
            "name": name,
        })),
        _ => None,
    }
}

fn from_anthropic_response(
    request: &CreateResponseRequest,
    mut body: Value,
) -> Result<ResponseObject, ResponsesError> {
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let stop_reason = body
        .get("stop_reason")
        .and_then(Value::as_str)
        .map(str::to_string);

    let usage_value = body.get_mut("usage").cloned().unwrap_or(Value::Null);
    let input_tokens = usage_value
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let output_tokens = usage_value
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let cached_tokens = usage_value
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .map(|value| value as u32);

    let mut output = Vec::new();
    let mut message_parts = Vec::new();
    let content = body
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for part in content {
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    message_parts.push(crate::types::ContentPartOutput::OutputText {
                        text: text.to_string(),
                        annotations: vec![],
                    });
                }
            }
            Some("tool_use") => {
                if !message_parts.is_empty() {
                    output.push(crate::types::OutputItem::Message(
                        crate::types::OutputMessage {
                            id: crate::common::new_id("msg"),
                            status: "completed".to_string(),
                            role: "assistant".to_string(),
                            content: std::mem::take(&mut message_parts),
                        },
                    ));
                }
                output.push(function_call_output(
                    part.get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    part.get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    part.get("input").cloned().unwrap_or(Value::Null),
                ));
            }
            Some("thinking") => {
                output.push(reasoning_output(
                    part.get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    part.get("signature")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                ));
            }
            _ => {}
        }
    }

    if !message_parts.is_empty() {
        output.push(crate::types::OutputItem::Message(
            crate::types::OutputMessage {
                id: crate::common::new_id("msg"),
                status: "completed".to_string(),
                role: "assistant".to_string(),
                content: message_parts,
            },
        ));
    }

    let mut response = make_response(
        request,
        model,
        output,
        Some(usage(input_tokens, cached_tokens, output_tokens, None)),
        incomplete_for_max_tokens(stop_reason.as_deref()),
    );
    if !id.is_empty() {
        response.id = id;
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::AnthropicResponsesProvider;
    use crate::types::{
        CreateResponseRequest, FunctionCallOutput, InputItem, ResponseInput, ToolChoice,
        ToolDefinition,
    };
    use crate::{ResponsesProvider, types};
    use configuration::{CapacityLimits, Credential, LlmProvider, LlmProviderType};
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(endpoint: String) -> AnthropicResponsesProvider {
        AnthropicResponsesProvider::new(
            LlmProvider {
                id: "anthropic".to_string(),
                default: Some(true),
                provider_type: LlmProviderType::Anthropic,
                responses_native: Some(false),
                priority: 0,
                model: "claude-3-7-sonnet".to_string(),
                tags: vec![],
                credential_file: "unused".to_string(),
                endpoint,
                capacity: CapacityLimits {
                    input_tokens: None,
                    requests: None,
                },
            },
            Credential::Anthropic {
                api_key: "secret".to_string(),
            },
        )
    }

    #[tokio::test]
    async fn create_response_maps_text_and_tool_use() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "secret"))
            .and(header("anthropic-version", "2023-06-01"))
            .and(body_json(json!({
                "model": "claude-3-7-sonnet",
                "max_tokens": 4096,
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "text",
                        "text": "hello"
                    }]
                }],
                "stream": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "msg_123",
                "type": "message",
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "Let me check." },
                    { "type": "tool_use", "id": "toolu_1", "name": "lookup_weather", "input": { "city": "Tokyo" } }
                ],
                "model": "claude-3-7-sonnet",
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 7
                }
            })))
            .mount(&server)
            .await;

        let response = provider(server.uri())
            .create_response(CreateResponseRequest {
                model: "caller".to_string(),
                input: ResponseInput::Text("hello".to_string()),
                instructions: None,
                previous_response_id: None,
                store: None,
                background: None,
                stream: None,
                tools: None,
                tool_choice: None,
                temperature: None,
                top_p: None,
                max_output_tokens: None,
                reasoning: None,
                text: None,
                metadata: None,
                user: None,
                parallel_tool_calls: None,
                truncation: None,
            })
            .await
            .unwrap();

        assert_eq!(response.id, "msg_123");
        assert_eq!(response.output.len(), 2);
        assert!(matches!(response.output[0], types::OutputItem::Message(_)));
        assert!(matches!(
            response.output[1],
            types::OutputItem::FunctionCall(_)
        ));
    }

    #[tokio::test]
    async fn create_response_sends_tool_results_as_tool_result_blocks() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(body_json(json!({
                "model": "claude-3-7-sonnet",
                "max_tokens": 4096,
                "messages": [{
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": "call_1",
                        "content": "{\"temperature\":22}"
                    }]
                }],
                "stream": false,
                "tools": [{
                    "name": "lookup_weather",
                    "description": "Lookup",
                    "input_schema": { "type": "object" }
                }],
                "tool_choice": { "type": "any" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "msg_124",
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "text", "text": "It is 22C." }],
                "model": "claude-3-7-sonnet",
                "stop_reason": "end_turn",
                "usage": {
                    "input_tokens": 16,
                    "output_tokens": 5
                }
            })))
            .mount(&server)
            .await;

        let response = provider(server.uri())
            .create_response(CreateResponseRequest {
                model: "caller".to_string(),
                input: ResponseInput::Items(vec![InputItem::FunctionCallOutput(
                    FunctionCallOutput {
                        call_id: "call_1".to_string(),
                        output: "{\"temperature\":22}".to_string(),
                        id: None,
                        status: None,
                    },
                )]),
                instructions: None,
                previous_response_id: None,
                store: None,
                background: None,
                stream: None,
                tools: Some(vec![ToolDefinition::Function {
                    name: "lookup_weather".to_string(),
                    description: Some("Lookup".to_string()),
                    parameters: Some(json!({ "type": "object" })),
                    strict: None,
                }]),
                tool_choice: Some(ToolChoice::Mode("required".to_string())),
                temperature: None,
                top_p: None,
                max_output_tokens: None,
                reasoning: None,
                text: None,
                metadata: None,
                user: None,
                parallel_tool_calls: None,
                truncation: None,
            })
            .await
            .unwrap();

        assert_eq!(response.output_text.as_deref(), None);
        assert_eq!(response.output.len(), 1);
        assert!(matches!(response.output[0], types::OutputItem::Message(_)));
    }
}
