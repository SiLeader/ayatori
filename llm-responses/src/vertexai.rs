use crate::common::{
    append_system_text, collect_text, function_call_output, function_tools,
    incomplete_for_max_tokens, input_items, make_response, parse_data_url_base64,
    parse_json_objectish, parse_json_string, reasoning_budget, reasoning_output, response_format,
    tool_call_id, tool_choice_mode, tool_name_from_call_id, usage,
};
use crate::http::send_value;
use crate::types::{
    ContentPartInput, CreateResponseRequest, InputItem, MessageContentInput, ResponseObject,
    ToolChoice,
};
use crate::{ProviderCapabilities, ResponsesError, ResponsesProvider};
use async_trait::async_trait;
use configuration::{Credential, LlmProvider};
use serde_json::{Map, Value, json};

pub(crate) struct VertexAiResponsesProvider {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl VertexAiResponsesProvider {
    pub(crate) fn new(provider: LlmProvider, credential: Credential) -> Self {
        let api_key = match credential {
            Credential::VertexAI { api_key } => api_key,
            credential => panic!("unexpected credential for VertexAI provider: {credential:?}"),
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
impl ResponsesProvider for VertexAiResponsesProvider {
    async fn create_response(
        &self,
        request: CreateResponseRequest,
    ) -> Result<ResponseObject, ResponsesError> {
        let payload = to_vertex_request(&request)?;
        let model_path =
            if self.model.starts_with("publishers/") || self.model.starts_with("projects/") {
                self.model.clone()
            } else {
                format!("models/{}", self.model)
            };
        let url = format!(
            "{}/v1/{}:generateContent?key={}",
            self.endpoint.trim_end_matches('/'),
            model_path,
            self.api_key
        );
        let body = send_value(self.client.post(url), &payload).await?;
        from_vertex_response(&request, body)
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            responses_native: false,
            builtin_tools: false,
            reasoning: true,
            image_input: true,
            structured_output: true,
            streaming: true,
            get_response: false,
            cancel_response: false,
        }
    }
}

fn to_vertex_request(request: &CreateResponseRequest) -> Result<Value, ResponsesError> {
    let mut system = request.instructions.clone();
    let mut contents: Vec<Value> = Vec::new();

    for item in input_items(&request.input) {
        match item {
            InputItem::Message(message)
                if matches!(message.role.as_str(), "system" | "developer") =>
            {
                append_system_text(&mut system, collect_text(&message.content));
            }
            InputItem::Message(message) => {
                let role = if message.role == "assistant" {
                    "model"
                } else {
                    "user"
                };
                let parts = message_content_to_vertex_parts(&message.content)?;
                push_vertex_content(&mut contents, role, parts);
            }
            InputItem::FunctionCall(call) => {
                push_vertex_content(
                    &mut contents,
                    "model",
                    vec![json!({
                        "functionCall": {
                            "name": call.name,
                            "args": parse_json_objectish(&call.arguments),
                        }
                    })],
                );
            }
            InputItem::FunctionCallOutput(output) => {
                push_vertex_content(
                    &mut contents,
                    "user",
                    vec![json!({
                        "functionResponse": {
                            "id": output.call_id,
                            "name": tool_name_from_call_id(&output.call_id),
                            "response": function_response_payload(&output.output),
                        }
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
                if !text.is_empty() {
                    push_vertex_content(
                        &mut contents,
                        "model",
                        vec![json!({
                            "text": text,
                            "thought": true,
                        })],
                    );
                }
            }
            InputItem::ItemReference { .. } => {
                return Err(ResponsesError::Unsupported("item_reference (vertex_ai)"));
            }
        }
    }

    let mut payload = json!({ "contents": contents });
    if let Some(system) = system {
        payload["systemInstruction"] = json!({
            "parts": [{ "text": system }],
        });
    }

    let tools = tools_to_vertex(request.tools.as_deref());
    if !tools.is_empty() {
        payload["tools"] = Value::Array(tools);
    }

    if let Some(tool_config) = request.tool_choice.as_ref().and_then(tool_choice_to_vertex) {
        payload["toolConfig"] = tool_config;
    }

    let mut generation_config = Map::new();
    if let Some(temperature) = request.temperature {
        generation_config.insert("temperature".to_string(), json!(temperature));
    }
    if let Some(top_p) = request.top_p {
        generation_config.insert("topP".to_string(), json!(top_p));
    }
    if let Some(max_output_tokens) = request.max_output_tokens {
        generation_config.insert("maxOutputTokens".to_string(), json!(max_output_tokens));
    }
    if let Some(budget_tokens) =
        reasoning_budget(request.max_output_tokens, request.reasoning.as_ref())
    {
        generation_config.insert(
            "thinkingConfig".to_string(),
            json!({
                "thinkingBudget": budget_tokens,
                "includeThoughts": true,
            }),
        );
    }
    let (response_mime_type, response_json_schema) = response_format(request.text.as_ref());
    if let Some(response_mime_type) = response_mime_type {
        generation_config.insert("responseMimeType".to_string(), json!(response_mime_type));
    }
    if let Some(response_json_schema) = response_json_schema {
        generation_config.insert("responseJsonSchema".to_string(), response_json_schema);
    }

    if !generation_config.is_empty() {
        payload["generationConfig"] = Value::Object(generation_config);
    }

    Ok(payload)
}

fn push_vertex_content(contents: &mut Vec<Value>, role: &str, parts: Vec<Value>) {
    if parts.is_empty() {
        return;
    }

    if let Some(last) = contents.last_mut()
        && last.get("role").and_then(Value::as_str) == Some(role)
        && let Some(existing_parts) = last.get_mut("parts").and_then(Value::as_array_mut)
    {
        existing_parts.extend(parts);
        return;
    }

    contents.push(json!({
        "role": role,
        "parts": parts,
    }));
}

fn message_content_to_vertex_parts(
    content: &MessageContentInput,
) -> Result<Vec<Value>, ResponsesError> {
    match content {
        MessageContentInput::Text(text) => Ok(vec![json!({ "text": text })]),
        MessageContentInput::Parts(parts) => {
            let mut values = Vec::new();
            for part in parts {
                match part {
                    ContentPartInput::InputText { text } => values.push(json!({ "text": text })),
                    ContentPartInput::InputImage {
                        image_url, file_id, ..
                    } => {
                        if file_id.is_some() {
                            return Err(ResponsesError::Unsupported(
                                "input_image.file_id (vertex_ai)",
                            ));
                        }
                        let image_url = image_url.as_deref().ok_or_else(|| {
                            ResponsesError::InvalidRequest(
                                "input_image.image_url is required".to_string(),
                            )
                        })?;
                        if let Some((mime_type, data)) = parse_data_url_base64(image_url) {
                            values.push(json!({
                                "inline_data": {
                                    "mime_type": mime_type,
                                    "data": data,
                                }
                            }));
                        } else {
                            values.push(json!({
                                "file_data": {
                                    "mime_type": "image/*",
                                    "file_uri": image_url,
                                }
                            }));
                        }
                    }
                    ContentPartInput::OutputText { text, .. } => {
                        values.push(json!({ "text": text }))
                    }
                    ContentPartInput::Refusal { refusal } => {
                        values.push(json!({ "text": refusal }))
                    }
                    ContentPartInput::InputFile { .. } => {
                        return Err(ResponsesError::Unsupported("input_file (vertex_ai)"));
                    }
                }
            }
            Ok(values)
        }
    }
}

fn tools_to_vertex(tools: Option<&[crate::types::ToolDefinition]>) -> Vec<Value> {
    let declarations: Vec<Value> = function_tools(tools)
        .into_iter()
        .map(|(name, description, parameters)| {
            json!({
                "name": name,
                "description": description,
                "parameters": parameters.cloned().unwrap_or_else(|| json!({"type": "object"})),
            })
        })
        .collect();
    if declarations.is_empty() {
        vec![]
    } else {
        vec![json!({
            "function_declarations": declarations,
        })]
    }
}

fn tool_choice_to_vertex(choice: &ToolChoice) -> Option<Value> {
    if let Some(mode) = tool_choice_mode(choice) {
        let mode = match mode {
            "required" => "ANY",
            "auto" => "AUTO",
            "none" => "NONE",
            _ => return None,
        };
        return Some(json!({
            "functionCallingConfig": {
                "mode": mode,
            }
        }));
    }

    match choice {
        ToolChoice::Specific {
            tool_type,
            name: Some(name),
        } if tool_type == "function" => Some(json!({
            "functionCallingConfig": {
                "mode": "ANY",
                "allowedFunctionNames": [name],
            }
        })),
        _ => None,
    }
}

fn function_response_payload(output: &str) -> Value {
    match parse_json_string(output) {
        Value::Object(map) if map.contains_key("output") || map.contains_key("error") => {
            Value::Object(map)
        }
        other => json!({ "output": other }),
    }
}

fn from_vertex_response(
    request: &CreateResponseRequest,
    body: Value,
) -> Result<ResponseObject, ResponsesError> {
    let model = body
        .get("modelVersion")
        .and_then(Value::as_str)
        .or_else(|| body.get("model").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let id = body
        .get("responseId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let usage_value = body.get("usageMetadata").cloned().unwrap_or(Value::Null);
    let input_tokens = usage_value
        .get("promptTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let candidate_tokens = usage_value
        .get("candidatesTokenCount")
        .and_then(Value::as_u64)
        .unwrap_or_default() as u32;
    let reasoning_tokens = usage_value
        .get("thoughtsTokenCount")
        .and_then(Value::as_u64)
        .map(|value| value as u32);
    let output_tokens = candidate_tokens + reasoning_tokens.unwrap_or(0);

    let candidate = body
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|candidates| candidates.first())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let finish_reason = candidate
        .get("finishReason")
        .and_then(Value::as_str)
        .map(str::to_string);
    let parts = candidate
        .get("content")
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut output = Vec::new();
    let mut message_text = String::new();

    for (index, part) in parts.into_iter().enumerate() {
        let thought_signature = part
            .get("thoughtSignature")
            .and_then(Value::as_str)
            .map(str::to_string);
        let thought = part
            .get("thought")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if let Some(text) = part.get("text").and_then(Value::as_str) {
            if thought {
                output.push(reasoning_output(text.to_string(), thought_signature));
            } else {
                message_text.push_str(text);
            }
        }

        if let Some(function_call) = part.get("functionCall") {
            if !message_text.is_empty() {
                output.push(crate::common::message_output(std::mem::take(
                    &mut message_text,
                )));
            }
            let name = function_call
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let arguments = function_call.get("args").cloned().unwrap_or(Value::Null);
            output.push(function_call_output(
                tool_call_id(&name, index),
                name,
                arguments,
            ));
        }
    }

    if !message_text.is_empty() {
        output.push(crate::common::message_output(message_text));
    }

    let mut response = make_response(
        request,
        model,
        output,
        Some(usage(input_tokens, None, output_tokens, reasoning_tokens)),
        incomplete_for_max_tokens(finish_reason.as_deref()),
    );
    if !id.is_empty() {
        response.id = id;
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::VertexAiResponsesProvider;
    use crate::types::{CreateResponseRequest, ResponseInput, ToolChoice, ToolDefinition};
    use crate::{ResponsesProvider, types};
    use configuration::{CapacityLimits, Credential, LlmProvider, LlmProviderType};
    use serde_json::json;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(endpoint: String) -> VertexAiResponsesProvider {
        VertexAiResponsesProvider::new(
            LlmProvider {
                id: "vertex".to_string(),
                default: Some(true),
                provider_type: LlmProviderType::VertexAI,
                responses_native: Some(false),
                priority: 0,
                model: "publishers/google/models/gemini-2.5-flash".to_string(),
                tags: vec![],
                credential_file: "unused".to_string(),
                endpoint,
                capacity: CapacityLimits {
                    input_tokens: None,
                    requests: None,
                },
            },
            Credential::VertexAI {
                api_key: "vertex-secret".to_string(),
            },
        )
    }

    #[tokio::test]
    async fn create_response_maps_function_calls() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/v1/publishers/google/models/gemini-2.5-flash:generateContent",
            ))
            .and(query_param("key", "vertex-secret"))
            .and(body_json(json!({
                "contents": [{
                    "role": "user",
                    "parts": [{ "text": "what is the weather?" }]
                }],
                "tools": [{
                    "function_declarations": [{
                        "name": "lookup_weather",
                        "description": "Lookup",
                        "parameters": { "type": "object" }
                    }]
                }],
                "toolConfig": {
                    "functionCallingConfig": {
                        "mode": "ANY"
                    }
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "responseId": "vertex_resp_1",
                "modelVersion": "gemini-2.5-flash-001",
                "candidates": [{
                    "finishReason": "STOP",
                    "content": {
                        "role": "model",
                        "parts": [{
                            "functionCall": {
                                "name": "lookup_weather",
                                "args": { "city": "Tokyo" }
                            }
                        }]
                    }
                }],
                "usageMetadata": {
                    "promptTokenCount": 10,
                    "candidatesTokenCount": 4,
                    "totalTokenCount": 14
                }
            })))
            .mount(&server)
            .await;

        let response = provider(server.uri())
            .create_response(CreateResponseRequest {
                model: "caller".to_string(),
                input: ResponseInput::Text("what is the weather?".to_string()),
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

        assert_eq!(response.id, "vertex_resp_1");
        assert!(matches!(
            response.output[0],
            types::OutputItem::FunctionCall(_)
        ));
        match &response.output[0] {
            types::OutputItem::FunctionCall(call) => {
                assert_eq!(call.call_id, "lookup_weather::0");
                assert_eq!(call.name, "lookup_weather");
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn create_response_uses_generation_config_for_json_schema() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/v1/publishers/google/models/gemini-2.5-flash:generateContent",
            ))
            .and(query_param("key", "vertex-secret"))
            .and(body_json(json!({
                "contents": [{
                    "role": "user",
                    "parts": [{ "text": "return json" }]
                }],
                "generationConfig": {
                    "responseMimeType": "application/json",
                    "responseJsonSchema": {
                        "type": "object",
                        "properties": {
                            "answer": { "type": "string" }
                        }
                    }
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "responseId": "vertex_resp_2",
                "modelVersion": "gemini-2.5-flash-001",
                "candidates": [{
                    "finishReason": "STOP",
                    "content": {
                        "role": "model",
                        "parts": [{ "text": "{\"answer\":\"ok\"}" }]
                    }
                }],
                "usageMetadata": {
                    "promptTokenCount": 10,
                    "candidatesTokenCount": 4,
                    "totalTokenCount": 14
                }
            })))
            .mount(&server)
            .await;

        let response = provider(server.uri())
            .create_response(CreateResponseRequest {
                model: "caller".to_string(),
                input: ResponseInput::Text("return json".to_string()),
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
                text: Some(types::TextConfig {
                    format: Some(types::TextFormat::JsonSchema {
                        name: "answer".to_string(),
                        schema: json!({
                            "type": "object",
                            "properties": {
                                "answer": { "type": "string" }
                            }
                        }),
                        strict: None,
                    }),
                }),
                metadata: None,
                user: None,
                parallel_tool_calls: None,
                truncation: None,
            })
            .await
            .unwrap();

        assert!(matches!(response.output[0], types::OutputItem::Message(_)));
    }
}
