use crate::common::{
    append_system_text, collect_text, function_call_output, function_tools,
    incomplete_for_max_tokens, input_items, make_in_progress_response, make_response, new_id,
    parse_data_url_base64, parse_json_objectish, parse_json_string, reasoning_budget,
    reasoning_output, response_format, tool_call_id, tool_choice_mode, tool_name_from_call_id,
    usage,
};
use crate::http::{send_stream, send_value};
use crate::types::{
    ContentPartInput, CreateResponseRequest, InputItem, MessageContentInput, OutputItem,
    ResponseObject, ResponseStatus, ResponseStreamEvent, ToolChoice,
};
use crate::{ProviderCapabilities, ResponsesError, ResponsesProvider};
use async_trait::async_trait;
use configuration::{Credential, LlmProvider};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use futures::stream::BoxStream;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

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

    async fn create_response_stream(
        &self,
        request: CreateResponseRequest,
    ) -> Result<BoxStream<'static, Result<ResponseStreamEvent, ResponsesError>>, ResponsesError>
    {
        let payload = to_vertex_request(&request)?;
        let model_path =
            if self.model.starts_with("publishers/") || self.model.starts_with("projects/") {
                self.model.clone()
            } else {
                format!("models/{}", self.model)
            };
        let url = format!(
            "{}/v1/{}:streamGenerateContent?alt=sse&key={}",
            self.endpoint.trim_end_matches('/'),
            model_path,
            self.api_key
        );
        let response = send_stream(self.client.post(url), &payload).await?;
        let mut mapper = VertexStreamMapper::new(self.model.clone(), &request);
        let stream = response
            .bytes_stream()
            .eventsource()
            .map(move |event| {
                let events = match event {
                    Ok(event) if event.data == "[DONE]" || event.data.is_empty() => Vec::new(),
                    Ok(event) => match serde_json::from_str::<VertexStreamChunk>(&event.data) {
                        Ok(chunk) => mapper.handle(chunk).into_iter().map(Ok).collect::<Vec<_>>(),
                        Err(error) => vec![Err(ResponsesError::Serde(error))],
                    },
                    Err(error) => vec![Err(ResponsesError::Internal(format!(
                        "failed to parse SSE stream: {error}"
                    )))],
                };
                futures::stream::iter(events)
            })
            .flatten();

        Ok(Box::pin(stream))
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

struct VertexStreamMapper {
    response: ResponseObject,
    started: bool,
    message_output_index: Option<u32>,
    tool_calls: HashMap<String, u32>,
}

impl VertexStreamMapper {
    fn new(model: String, request: &CreateResponseRequest) -> Self {
        Self {
            response: make_in_progress_response(request, model),
            started: false,
            message_output_index: None,
            tool_calls: HashMap::new(),
        }
    }

    fn handle(&mut self, chunk: VertexStreamChunk) -> Vec<ResponseStreamEvent> {
        if let Some(response_id) = chunk.response_id {
            self.response.id = response_id;
        }
        if let Some(model) = chunk.model_version.or(chunk.model) {
            self.response.model = model;
        }
        if let Some(usage_metadata) = chunk.usage_metadata {
            self.response.usage = Some(usage(
                usage_metadata.prompt_token_count.unwrap_or_default(),
                None,
                usage_metadata.candidates_token_count.unwrap_or_default()
                    + usage_metadata.thoughts_token_count.unwrap_or_default(),
                usage_metadata.thoughts_token_count,
            ));
        }

        let mut events = Vec::new();
        if !self.started {
            self.started = true;
            events.push(ResponseStreamEvent::Created {
                response: self.response.clone(),
            });
            events.push(ResponseStreamEvent::InProgress {
                response: self.response.clone(),
            });
        }

        for candidate in chunk.candidates {
            if let Some(content) = candidate.content {
                for (part_index, part) in content.parts.into_iter().enumerate() {
                    if let Some(text) = part.text {
                        if part.thought.unwrap_or(false) {
                            continue;
                        }
                        events.extend(self.push_text_delta(text));
                    }
                    if let Some(function_call) = part.function_call {
                        events.extend(self.push_function_call(part_index, function_call));
                    }
                }
            }

            if let Some(finish_reason) = candidate.finish_reason {
                events.extend(self.finish(&finish_reason));
            }
        }

        events
    }

    fn push_text_delta(&mut self, delta: String) -> Vec<ResponseStreamEvent> {
        if delta.is_empty() {
            return Vec::new();
        }

        let mut events = Vec::new();
        let (output_index, item_id, content_index) =
            if let Some(output_index) = self.message_output_index {
                let item_id = match self.response.output.get(output_index as usize) {
                    Some(OutputItem::Message(message)) => message.id.clone(),
                    _ => new_id("msg"),
                };
                (output_index, item_id, 0)
            } else {
                let message = crate::types::OutputMessage {
                    id: new_id("msg"),
                    status: "in_progress".to_string(),
                    role: "assistant".to_string(),
                    content: vec![crate::types::ContentPartOutput::OutputText {
                        text: String::new(),
                        annotations: vec![],
                    }],
                };
                let output_index = self.response.output.len() as u32;
                self.response
                    .output
                    .push(OutputItem::Message(message.clone()));
                self.message_output_index = Some(output_index);
                events.push(ResponseStreamEvent::OutputItemAdded {
                    output_index,
                    item: OutputItem::Message(message.clone()),
                });
                events.push(ResponseStreamEvent::ContentPartAdded {
                    item_id: message.id.clone(),
                    output_index,
                    content_index: 0,
                    part: crate::types::ContentPartOutput::OutputText {
                        text: String::new(),
                        annotations: vec![],
                    },
                });
                (output_index, message.id, 0)
            };

        if let Some(OutputItem::Message(message)) =
            self.response.output.get_mut(output_index as usize)
            && let Some(crate::types::ContentPartOutput::OutputText { text, .. }) =
                message.content.get_mut(content_index as usize)
        {
            text.push_str(&delta);
        }

        events.push(ResponseStreamEvent::OutputTextDelta {
            item_id,
            output_index,
            content_index,
            delta,
        });
        events
    }

    fn push_function_call(
        &mut self,
        part_index: usize,
        function_call: VertexFunctionCall,
    ) -> Vec<ResponseStreamEvent> {
        let call_id = tool_call_id(&function_call.name, part_index);
        let arguments = function_call
            .args
            .unwrap_or(Value::Object(Map::new()))
            .to_string();

        if let Some(output_index) = self.tool_calls.get(&call_id).copied() {
            let mut delta = arguments.clone();
            if let Some(OutputItem::FunctionCall(call)) =
                self.response.output.get_mut(output_index as usize)
            {
                if arguments.starts_with(&call.arguments) {
                    delta = arguments[call.arguments.len()..].to_string();
                }
                call.arguments = arguments.clone();
            }
            if delta.is_empty() {
                return Vec::new();
            }
            return vec![ResponseStreamEvent::FunctionCallArgumentsDelta {
                item_id: self
                    .response
                    .output
                    .get(output_index as usize)
                    .and_then(|item| match item {
                        OutputItem::FunctionCall(call) => Some(call.id.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| new_id("fc")),
                output_index,
                delta,
            }];
        }

        let item = crate::types::FunctionCallItem {
            id: new_id("fc"),
            call_id: call_id.clone(),
            name: function_call.name,
            arguments: arguments.clone(),
            status: "in_progress".to_string(),
        };
        let output_index = self.response.output.len() as u32;
        self.response
            .output
            .push(OutputItem::FunctionCall(item.clone()));
        self.tool_calls.insert(call_id, output_index);

        let mut events = vec![ResponseStreamEvent::OutputItemAdded {
            output_index,
            item: OutputItem::FunctionCall(item.clone()),
        }];
        if !arguments.is_empty() {
            events.push(ResponseStreamEvent::FunctionCallArgumentsDelta {
                item_id: item.id,
                output_index,
                delta: arguments,
            });
        }
        events
    }

    fn finish(&mut self, finish_reason: &str) -> Vec<ResponseStreamEvent> {
        let mut events = Vec::new();

        if let Some(output_index) = self.message_output_index
            && let Some(OutputItem::Message(message)) =
                self.response.output.get_mut(output_index as usize)
            && message.status != "completed"
        {
            let part = message.content.first().cloned().unwrap_or(
                crate::types::ContentPartOutput::OutputText {
                    text: String::new(),
                    annotations: vec![],
                },
            );
            let text = match &part {
                crate::types::ContentPartOutput::OutputText { text, .. } => text.clone(),
                crate::types::ContentPartOutput::Refusal { refusal } => refusal.clone(),
            };
            message.status = "completed".to_string();
            events.push(ResponseStreamEvent::OutputTextDone {
                item_id: message.id.clone(),
                output_index,
                content_index: 0,
                text,
            });
            events.push(ResponseStreamEvent::ContentPartDone {
                item_id: message.id.clone(),
                output_index,
                content_index: 0,
                part,
            });
            events.push(ResponseStreamEvent::OutputItemDone {
                output_index,
                item: OutputItem::Message(message.clone()),
            });
        }

        for output_index in self.tool_calls.values().copied().collect::<Vec<_>>() {
            if let Some(OutputItem::FunctionCall(call)) =
                self.response.output.get_mut(output_index as usize)
                && call.status != "completed"
            {
                call.status = "completed".to_string();
                events.push(ResponseStreamEvent::FunctionCallArgumentsDone {
                    item_id: call.id.clone(),
                    output_index,
                    arguments: call.arguments.clone(),
                });
                events.push(ResponseStreamEvent::OutputItemDone {
                    output_index,
                    item: OutputItem::FunctionCall(call.clone()),
                });
            }
        }

        self.response.incomplete_details = incomplete_for_max_tokens(Some(finish_reason));
        self.response.status = if self.response.incomplete_details.is_some() {
            ResponseStatus::Incomplete
        } else {
            ResponseStatus::Completed
        };
        self.response.ensure_output_text();

        events.push(if self.response.incomplete_details.is_some() {
            ResponseStreamEvent::Incomplete {
                response: self.response.clone(),
            }
        } else {
            ResponseStreamEvent::Completed {
                response: self.response.clone(),
            }
        });
        events
    }
}

#[derive(Debug, Deserialize)]
struct VertexStreamChunk {
    #[serde(rename = "responseId")]
    #[serde(default)]
    response_id: Option<String>,
    #[serde(rename = "modelVersion")]
    #[serde(default)]
    model_version: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(rename = "usageMetadata")]
    #[serde(default)]
    usage_metadata: Option<VertexUsageMetadata>,
    #[serde(default)]
    candidates: Vec<VertexCandidate>,
}

#[derive(Debug, Deserialize)]
struct VertexUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    #[serde(default)]
    prompt_token_count: Option<u32>,
    #[serde(rename = "candidatesTokenCount")]
    #[serde(default)]
    candidates_token_count: Option<u32>,
    #[serde(rename = "thoughtsTokenCount")]
    #[serde(default)]
    thoughts_token_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct VertexCandidate {
    #[serde(rename = "finishReason")]
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    content: Option<VertexContent>,
}

#[derive(Debug, Deserialize)]
struct VertexContent {
    #[serde(default)]
    parts: Vec<VertexPart>,
}

#[derive(Debug, Deserialize)]
struct VertexPart {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thought: Option<bool>,
    #[serde(rename = "functionCall")]
    #[serde(default)]
    function_call: Option<VertexFunctionCall>,
}

#[derive(Debug, Deserialize)]
struct VertexFunctionCall {
    name: String,
    #[serde(default)]
    args: Option<Value>,
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
