use crate::common::{
    append_system_text, collect_text, function_call_output, function_tools, input_items,
    make_in_progress_response, make_response, message_output, new_id, parse_data_url_base64,
    parse_json_objectish, reasoning_output, usage,
};
use crate::types::{
    ContentPartInput, CreateResponseRequest, InputItem, MessageContentInput, OutputItem,
    ResponseObject, ResponseStatus, ResponseStreamEvent, TextFormat,
};
use crate::{ProviderCapabilities, ResponsesError, ResponsesProvider};
use async_trait::async_trait;
use configuration::{Credential, LlmProvider, LlmProviderType};
use genai::adapter::AdapterKind;
use genai::chat::{
    Binary, BinarySource, ChatMessage, ChatOptions, ChatRequest, ChatResponse, ChatResponseFormat,
    ChatRole, ChatStreamEvent, ContentPart, JsonSpec, MessageContent, Tool, ToolCall,
    ToolResponse,
};
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget};
use futures::StreamExt;
use futures::stream::BoxStream;
use std::collections::HashMap;

pub(crate) struct GenaiBackedProvider {
    client: Client,
    model: String,
    capabilities: ProviderCapabilities,
}

impl GenaiBackedProvider {
    pub(crate) fn new(provider: LlmProvider, credential: Credential) -> Self {
        let provider_type = provider.provider_type.clone();
        let client = Client::builder()
            .with_service_target_resolver(create_service_target_resolver(provider, credential))
            .build();

        let capabilities = match provider_type {
            LlmProviderType::Ollama => ProviderCapabilities {
                responses_native: false,
                builtin_tools: false,
                reasoning: false,
                image_input: true,
                structured_output: true,
                streaming: true,
                get_response: false,
                cancel_response: false,
            },
            _ => ProviderCapabilities::default(),
        };

        Self {
            client,
            model: String::new(),
            capabilities,
        }
    }

    pub(crate) fn with_model(mut self, model: String) -> Self {
        self.model = model;
        self
    }
}

#[async_trait]
impl ResponsesProvider for GenaiBackedProvider {
    async fn create_response(
        &self,
        request: CreateResponseRequest,
    ) -> Result<ResponseObject, ResponsesError> {
        let chat_req = to_genai_chat_request(&request)?;
        let chat_options = to_genai_chat_options(&request)?;
        let response = self
            .client
            .exec_chat(&self.model, chat_req, Some(&chat_options))
            .await
            .map_err(|error| ResponsesError::Internal(format!("genai: {error}")))?;
        Ok(from_genai_chat_response(&request, response))
    }

    async fn create_response_stream(
        &self,
        request: CreateResponseRequest,
    ) -> Result<BoxStream<'static, Result<ResponseStreamEvent, ResponsesError>>, ResponsesError>
    {
        let chat_req = to_genai_chat_request(&request)?;
        let chat_options = to_genai_chat_options(&request)?
            .with_capture_usage(true)
            .with_capture_content(true)
            .with_capture_reasoning_content(true)
            .with_capture_tool_calls(true);
        let stream_res = self
            .client
            .exec_chat_stream(&self.model, chat_req, Some(&chat_options))
            .await
            .map_err(|error| ResponsesError::Internal(format!("genai: {error}")))?;

        let mut mapper =
            GenaiStreamMapper::new(stream_res.model_iden.model_name.to_string(), &request);
        let stream = stream_res
            .stream
            .map(move |event| {
                let events = match event {
                    Ok(event) => mapper
                        .handle(event)
                        .into_iter()
                        .map(Ok)
                        .collect::<Vec<_>>(),
                    Err(error) => vec![Err(ResponsesError::Internal(format!("genai: {error}")))],
                };
                futures::stream::iter(events)
            })
            .flatten();

        Ok(Box::pin(stream))
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities.clone()
    }
}

fn create_service_target_resolver(
    provider: LlmProvider,
    credential: Credential,
) -> ServiceTargetResolver {
    let auth = match credential {
        Credential::Azure { api_key, .. } => AuthData::from_single(api_key),
        Credential::Bedrock { api_key } => AuthData::from_single(api_key),
        Credential::Anthropic { api_key } => AuthData::from_single(api_key),
        Credential::Ollama => AuthData::from_single(""),
        Credential::OpenAI { api_key } => AuthData::from_single(api_key),
        Credential::VertexAI { api_key } => AuthData::from_single(api_key),
    };

    ServiceTargetResolver::from_resolver_fn(
        move |_service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
            let endpoint = Endpoint::from_owned(provider.endpoint.clone());
            let model = ModelIden::new(
                match provider.provider_type {
                    LlmProviderType::Azure => AdapterKind::OpenAI,
                    LlmProviderType::Bedrock => AdapterKind::OpenAI,
                    LlmProviderType::Anthropic => AdapterKind::Anthropic,
                    LlmProviderType::Ollama => AdapterKind::Ollama,
                    LlmProviderType::OpenAI => AdapterKind::OpenAI,
                    LlmProviderType::VertexAI => AdapterKind::Gemini,
                },
                provider.model.clone(),
            );
            Ok(ServiceTarget {
                endpoint,
                auth: auth.clone(),
                model,
            })
        },
    )
}

fn to_genai_chat_request(request: &CreateResponseRequest) -> Result<ChatRequest, ResponsesError> {
    let mut system = request.instructions.clone();
    let mut messages: Vec<ChatMessage> = Vec::new();

    for item in input_items(&request.input) {
        match item {
            InputItem::Message(message)
                if matches!(message.role.as_str(), "system" | "developer") =>
            {
                append_system_text(&mut system, collect_text(&message.content));
            }
            InputItem::Message(message) => {
                let role = if message.role == "assistant" {
                    ChatRole::Assistant
                } else {
                    ChatRole::User
                };
                let content = message_content_to_genai(message.content)?;
                push_genai_message(&mut messages, role, content);
            }
            InputItem::FunctionCall(call) => {
                let tool_call = ToolCall {
                    call_id: call.call_id,
                    fn_name: call.name,
                    fn_arguments: parse_json_objectish(&call.arguments),
                    thought_signatures: None,
                };
                push_genai_message(
                    &mut messages,
                    ChatRole::Assistant,
                    MessageContent::from_tool_calls(vec![tool_call]),
                );
            }
            InputItem::FunctionCallOutput(output) => {
                messages.push(ChatMessage::from(ToolResponse::new(
                    output.call_id,
                    output.output,
                )));
            }
            InputItem::Reasoning(_) => {}
            InputItem::ItemReference { .. } => {
                return Err(ResponsesError::Unsupported("item_reference (genai)"));
            }
        }
    }

    let mut chat_request = ChatRequest {
        system,
        messages,
        tools: None,
    };

    let tools: Vec<Tool> = function_tools(request.tools.as_deref())
        .into_iter()
        .map(|(name, description, parameters)| {
            let mut tool = Tool::new(name);
            if let Some(description) = description {
                tool = tool.with_description(description);
            }
            if let Some(parameters) = parameters {
                tool = tool.with_schema(parameters.clone());
            }
            tool
        })
        .collect();
    if !tools.is_empty() {
        chat_request.tools = Some(tools);
    }

    Ok(chat_request)
}

fn push_genai_message(messages: &mut Vec<ChatMessage>, role: ChatRole, content: MessageContent) {
    if content.is_empty() {
        return;
    }

    if let Some(last) = messages.last_mut()
        && last.role == role
    {
        last.content.extend(content.into_parts());
        return;
    }

    messages.push(ChatMessage {
        role,
        content,
        options: None,
    });
}

fn message_content_to_genai(
    content: MessageContentInput,
) -> Result<MessageContent, ResponsesError> {
    match content {
        MessageContentInput::Text(text) => Ok(MessageContent::from_text(text)),
        MessageContentInput::Parts(parts) => {
            let mut out = MessageContent::default();
            for part in parts {
                match part {
                    ContentPartInput::InputText { text } => out.push(ContentPart::from_text(text)),
                    ContentPartInput::InputImage {
                        image_url, file_id, ..
                    } => {
                        if file_id.is_some() {
                            return Err(ResponsesError::Unsupported("input_image.file_id (genai)"));
                        }
                        let image_url = image_url.ok_or_else(|| {
                            ResponsesError::InvalidRequest(
                                "input_image.image_url is required".to_string(),
                            )
                        })?;
                        let binary = if let Some((mime, data)) = parse_data_url_base64(&image_url) {
                            Binary::new(mime, BinarySource::Base64(data.into()), None)
                        } else {
                            Binary::new("image/*", BinarySource::Url(image_url), None)
                        };
                        out.push(binary);
                    }
                    ContentPartInput::OutputText { text, .. } => {
                        out.push(ContentPart::from_text(text))
                    }
                    ContentPartInput::Refusal { refusal } => {
                        out.push(ContentPart::from_text(refusal))
                    }
                    ContentPartInput::InputFile { .. } => {
                        return Err(ResponsesError::Unsupported("input_file (genai)"));
                    }
                }
            }
            Ok(out)
        }
    }
}

fn to_genai_chat_options(request: &CreateResponseRequest) -> Result<ChatOptions, ResponsesError> {
    let mut options = ChatOptions {
        temperature: request.temperature,
        max_tokens: request.max_output_tokens,
        top_p: request.top_p,
        stop_sequences: vec![],
        ..Default::default()
    };

    if let Some(format) = request.text.as_ref().and_then(|text| text.format.as_ref()) {
        options.response_format = Some(match format {
            TextFormat::Text => return Ok(options),
            TextFormat::JsonObject => ChatResponseFormat::JsonMode,
            TextFormat::JsonSchema { name, schema, .. } => {
                ChatResponseFormat::JsonSpec(JsonSpec::new(name.clone(), schema.clone()))
            }
        });
    }

    Ok(options)
}

fn from_genai_chat_response(
    request: &CreateResponseRequest,
    response: ChatResponse,
) -> ResponseObject {
    let mut output = Vec::new();
    let text = response.content.joined_texts().unwrap_or_default();
    if !text.is_empty() {
        output.push(message_output(text));
    }

    for tool_call in response.content.tool_calls() {
        output.push(function_call_output(
            tool_call.call_id.clone(),
            tool_call.fn_name.clone(),
            tool_call.fn_arguments.clone(),
        ));
    }

    if let Some(reasoning) = response.reasoning_content.clone()
        && !reasoning.is_empty()
    {
        output.push(reasoning_output(reasoning, None));
    }

    let input_tokens = response.usage.prompt_tokens.unwrap_or_default().max(0) as u32;
    let output_tokens = response.usage.completion_tokens.unwrap_or_default().max(0) as u32;
    let cached_tokens = response
        .usage
        .prompt_tokens_details
        .as_ref()
        .and_then(|details| details.cached_tokens)
        .and_then(|value| u32::try_from(value).ok());
    let reasoning_tokens = response
        .usage
        .completion_tokens_details
        .as_ref()
        .and_then(|details| details.reasoning_tokens)
        .and_then(|value| u32::try_from(value).ok());

    make_response(
        request,
        response.provider_model_iden.model_name.to_string(),
        output,
        Some(usage(
            input_tokens,
            cached_tokens,
            output_tokens,
            reasoning_tokens,
        )),
        None,
    )
}

struct GenaiStreamMapper {
    response: ResponseObject,
    started: bool,
    message_output_index: Option<u32>,
    reasoning_output_index: Option<u32>,
    tool_calls: HashMap<String, u32>,
}

impl GenaiStreamMapper {
    fn new(model: String, request: &CreateResponseRequest) -> Self {
        Self {
            response: make_in_progress_response(request, model),
            started: false,
            message_output_index: None,
            reasoning_output_index: None,
            tool_calls: HashMap::new(),
        }
    }

    fn handle(&mut self, event: ChatStreamEvent) -> Vec<ResponseStreamEvent> {
        match event {
            ChatStreamEvent::Start => {
                self.started = true;
                vec![
                    ResponseStreamEvent::Created {
                        response: self.response.clone(),
                    },
                    ResponseStreamEvent::InProgress {
                        response: self.response.clone(),
                    },
                ]
            }
            ChatStreamEvent::Chunk(chunk) => self.push_text_delta(chunk.content),
            ChatStreamEvent::ReasoningChunk(chunk) => self.push_reasoning_delta(chunk.content),
            ChatStreamEvent::ThoughtSignatureChunk(signature) => {
                if let Some(output_index) = self.reasoning_output_index
                    && let Some(OutputItem::Reasoning(reasoning)) =
                        self.response.output.get_mut(output_index as usize)
                {
                    reasoning.encrypted_content = Some(signature.content);
                }
                Vec::new()
            }
            ChatStreamEvent::ToolCallChunk(chunk) => self.push_tool_call(chunk.tool_call),
            ChatStreamEvent::End(end) => self.finish(end),
        }
    }

    fn push_text_delta(&mut self, delta: String) -> Vec<ResponseStreamEvent> {
        if delta.is_empty() {
            return Vec::new();
        }

        let mut events = Vec::new();
        let (output_index, item_id) = if let Some(output_index) = self.message_output_index {
            let item_id = match self.response.output.get(output_index as usize) {
                Some(OutputItem::Message(message)) => message.id.clone(),
                _ => new_id("msg"),
            };
            (output_index, item_id)
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
            self.response.output.push(OutputItem::Message(message.clone()));
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
            (output_index, message.id)
        };

        if let Some(OutputItem::Message(message)) = self.response.output.get_mut(output_index as usize)
            && let Some(crate::types::ContentPartOutput::OutputText { text, .. }) =
                message.content.get_mut(0)
        {
            text.push_str(&delta);
        }

        events.push(ResponseStreamEvent::OutputTextDelta {
            item_id,
            output_index,
            content_index: 0,
            delta,
        });
        events
    }

    fn push_reasoning_delta(&mut self, delta: String) -> Vec<ResponseStreamEvent> {
        if delta.is_empty() {
            return Vec::new();
        }

        let mut events = Vec::new();
        let (output_index, item_id) = if let Some(output_index) = self.reasoning_output_index {
            let item_id = match self.response.output.get(output_index as usize) {
                Some(OutputItem::Reasoning(reasoning)) => reasoning.id.clone(),
                _ => new_id("rs"),
            };
            (output_index, item_id)
        } else {
            let item = crate::types::ReasoningItem {
                id: new_id("rs"),
                summary: vec![crate::types::SummaryPart::Text {
                    text: String::new(),
                }],
                encrypted_content: None,
            };
            let output_index = self.response.output.len() as u32;
            self.response.output.push(OutputItem::Reasoning(item.clone()));
            self.reasoning_output_index = Some(output_index);
            events.push(ResponseStreamEvent::OutputItemAdded {
                output_index,
                item: OutputItem::Reasoning(item.clone()),
            });
            events.push(ResponseStreamEvent::ReasoningSummaryPartAdded {
                item_id: item.id.clone(),
                output_index,
                summary_index: 0,
                part: crate::types::SummaryPart::Text {
                    text: String::new(),
                },
            });
            (output_index, item.id)
        };

        if let Some(OutputItem::Reasoning(reasoning)) = self.response.output.get_mut(output_index as usize)
            && let Some(crate::types::SummaryPart::Text { text }) = reasoning.summary.first_mut()
        {
            text.push_str(&delta);
        }

        events.push(ResponseStreamEvent::ReasoningSummaryTextDelta {
            item_id,
            output_index,
            summary_index: 0,
            delta,
        });
        events
    }

    fn push_tool_call(&mut self, tool_call: ToolCall) -> Vec<ResponseStreamEvent> {
        if self.tool_calls.contains_key(&tool_call.call_id) {
            return Vec::new();
        }

        let item = crate::types::FunctionCallItem {
            id: new_id("fc"),
            call_id: tool_call.call_id.clone(),
            name: tool_call.fn_name.clone(),
            arguments: tool_call.fn_arguments.to_string(),
            status: "completed".to_string(),
        };
        let output_index = self.response.output.len() as u32;
        self.response.output.push(OutputItem::FunctionCall(item.clone()));
        self.tool_calls.insert(tool_call.call_id, output_index);

        vec![
            ResponseStreamEvent::OutputItemAdded {
                output_index,
                item: OutputItem::FunctionCall(item.clone()),
            },
            ResponseStreamEvent::FunctionCallArgumentsDone {
                item_id: item.id.clone(),
                output_index,
                arguments: item.arguments.clone(),
            },
            ResponseStreamEvent::OutputItemDone {
                output_index,
                item: OutputItem::FunctionCall(item),
            },
        ]
    }

    fn finish(&mut self, end: genai::chat::StreamEnd) -> Vec<ResponseStreamEvent> {
        let mut events = Vec::new();

        if let Some(output_index) = self.message_output_index
            && let Some(OutputItem::Message(message)) = self.response.output.get_mut(output_index as usize)
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

        if let Some(output_index) = self.reasoning_output_index {
            let mut done_text = None;
            if let Some(OutputItem::Reasoning(reasoning)) =
                self.response.output.get(output_index as usize)
            {
                done_text = reasoning.summary.first().map(|part| match part {
                    crate::types::SummaryPart::Text { text } => text.clone(),
                });
            }
            let done_text = done_text.unwrap_or_default();
            let item_id = self
                .response
                .output
                .get(output_index as usize)
                .and_then(|item| match item {
                    OutputItem::Reasoning(reasoning) => Some(reasoning.id.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| new_id("rs"));
            events.push(ResponseStreamEvent::ReasoningSummaryTextDone {
                item_id: item_id.clone(),
                output_index,
                summary_index: 0,
                text: done_text.clone(),
            });
            events.push(ResponseStreamEvent::ReasoningSummaryPartDone {
                item_id: item_id.clone(),
                output_index,
                summary_index: 0,
                part: crate::types::SummaryPart::Text { text: done_text },
            });
            if let Some(item) = self.response.output.get(output_index as usize).cloned() {
                events.push(ResponseStreamEvent::OutputItemDone { output_index, item });
            }
        }

        if let Some(stream_usage) = end.captured_usage {
            let input_tokens = stream_usage.prompt_tokens.unwrap_or_default().max(0) as u32;
            let output_tokens = stream_usage.completion_tokens.unwrap_or_default().max(0) as u32;
            let cached_tokens = stream_usage
                .prompt_tokens_details
                .and_then(|details| details.cached_tokens)
                .and_then(|value| u32::try_from(value).ok());
            let reasoning_tokens = stream_usage
                .completion_tokens_details
                .and_then(|details| details.reasoning_tokens)
                .and_then(|value| u32::try_from(value).ok());
            self.response.usage = Some(usage(
                input_tokens,
                cached_tokens,
                output_tokens,
                reasoning_tokens,
            ));
        }

        self.response.status = ResponseStatus::Completed;
        self.response.ensure_output_text();
        events.push(ResponseStreamEvent::Completed {
            response: self.response.clone(),
        });
        events
    }
}

#[cfg(test)]
mod tests {
    use super::{from_genai_chat_response, to_genai_chat_options};
    use crate::types::{CreateResponseRequest, ResponseInput, TextConfig, TextFormat};
    use genai::ModelIden;
    use genai::adapter::AdapterKind;
    use genai::chat::{ChatResponse, MessageContent, Usage};
    use serde_json::json;

    fn request() -> CreateResponseRequest {
        CreateResponseRequest {
            model: "ollama".to_string(),
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
        }
    }

    #[test]
    fn genai_chat_options_support_json_schema() {
        let mut request = request();
        request.text = Some(TextConfig {
            format: Some(TextFormat::JsonSchema {
                name: "answer".to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "answer": { "type": "string" }
                    }
                }),
                strict: None,
            }),
        });

        let options = to_genai_chat_options(&request).unwrap();
        assert!(options.response_format.is_some());
    }

    #[test]
    fn genai_chat_response_maps_to_response_object() {
        let response = ChatResponse {
            content: MessageContent::from_text("hello back"),
            reasoning_content: Some("thinking".to_string()),
            model_iden: ModelIden::new(AdapterKind::Ollama, "gemma3:4b"),
            provider_model_iden: ModelIden::new(AdapterKind::Ollama, "gemma3:4b"),
            usage: Usage {
                prompt_tokens: Some(10),
                prompt_tokens_details: None,
                completion_tokens: Some(5),
                completion_tokens_details: None,
                total_tokens: Some(15),
            },
            captured_raw_body: None,
        };

        let object = from_genai_chat_response(&request(), response);
        assert_eq!(object.model, "gemma3:4b");
        assert_eq!(object.output.len(), 2);
    }
}
