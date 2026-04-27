use crate::common::{
    append_system_text, collect_text, function_call_output, function_tools, input_items,
    make_response, message_output, parse_data_url_base64, parse_json_objectish, reasoning_output,
    usage,
};
use crate::types::{
    ContentPartInput, CreateResponseRequest, InputItem, MessageContentInput, ResponseObject,
    TextFormat,
};
use crate::{ProviderCapabilities, ResponsesError, ResponsesProvider};
use async_trait::async_trait;
use configuration::{Credential, LlmProvider, LlmProviderType};
use genai::adapter::AdapterKind;
use genai::chat::{
    Binary, BinarySource, ChatMessage, ChatOptions, ChatRequest, ChatResponse, ChatResponseFormat,
    ChatRole, ContentPart, JsonSpec, MessageContent, Tool, ToolCall, ToolResponse,
};
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget};

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
