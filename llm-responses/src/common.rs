use crate::types::{
    ContentPartInput, ContentPartOutput, CreateResponseRequest, FunctionCallItem,
    IncompleteDetails, InputItem, InputTokensDetails, MessageContentInput, OutputItem,
    OutputMessage, OutputTokensDetails, ReasoningConfig, ReasoningItem, ResponseInput,
    ResponseObject, ResponseStatus, ResponseUsage, SummaryPart, TextConfig, ToolChoice,
    ToolDefinition,
};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", NEXT_ID.fetch_add(1, Ordering::Relaxed))
}

pub(crate) fn input_items(input: &ResponseInput) -> Vec<InputItem> {
    match input {
        ResponseInput::Text(text) => vec![InputItem::Message(crate::types::InputMessage {
            role: "user".to_string(),
            content: MessageContentInput::Text(text.clone()),
        })],
        ResponseInput::Items(items) => items.clone(),
    }
}

pub(crate) fn append_system_text(system: &mut Option<String>, text: String) {
    if text.is_empty() {
        return;
    }

    match system {
        Some(existing) if !existing.is_empty() => {
            existing.push_str("\n\n");
            existing.push_str(&text);
        }
        Some(existing) => existing.push_str(&text),
        None => *system = Some(text),
    }
}

pub(crate) fn collect_text(content: &MessageContentInput) -> String {
    match content {
        MessageContentInput::Text(text) => text.clone(),
        MessageContentInput::Parts(parts) => parts
            .iter()
            .filter_map(|part| match part {
                ContentPartInput::InputText { text } => Some(text.as_str()),
                ContentPartInput::OutputText { text, .. } => Some(text.as_str()),
                ContentPartInput::Refusal { refusal } => Some(refusal.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}

pub(crate) fn parse_json_string(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()))
}

pub(crate) fn parse_json_objectish(value: &str) -> Value {
    match parse_json_string(value) {
        Value::Object(map) => Value::Object(map),
        other => serde_json::json!({ "value": other }),
    }
}

pub(crate) fn parse_data_url_base64(url: &str) -> Option<(String, String)> {
    let payload = url.strip_prefix("data:")?;
    let (meta, data) = payload.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?.trim().to_string();
    if mime.is_empty() {
        return None;
    }
    Some((mime, percent_decode(data)))
}

fn percent_decode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte as char);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index] as char);
        index += 1;
    }
    out
}

pub(crate) fn message_output(text: String) -> OutputItem {
    OutputItem::Message(OutputMessage {
        id: new_id("msg"),
        status: "completed".to_string(),
        role: "assistant".to_string(),
        content: vec![ContentPartOutput::OutputText {
            text,
            annotations: vec![],
        }],
    })
}

pub(crate) fn reasoning_output(text: String, encrypted_content: Option<String>) -> OutputItem {
    OutputItem::Reasoning(ReasoningItem {
        id: new_id("rs"),
        summary: if text.is_empty() {
            vec![]
        } else {
            vec![SummaryPart::Text { text }]
        },
        encrypted_content,
    })
}

pub(crate) fn function_call_output(call_id: String, name: String, arguments: Value) -> OutputItem {
    OutputItem::FunctionCall(FunctionCallItem {
        id: new_id("fc"),
        call_id,
        name,
        arguments: arguments.to_string(),
        status: "completed".to_string(),
    })
}

pub(crate) fn make_response(
    request: &CreateResponseRequest,
    model: String,
    output: Vec<OutputItem>,
    usage: Option<ResponseUsage>,
    incomplete_details: Option<IncompleteDetails>,
) -> ResponseObject {
    ResponseObject {
        id: new_id("resp"),
        object: "response".to_string(),
        created_at: chrono::Utc::now().timestamp(),
        status: if incomplete_details.is_some() {
            ResponseStatus::Incomplete
        } else {
            ResponseStatus::Completed
        },
        model,
        output,
        output_text: None,
        usage,
        error: None,
        incomplete_details,
        previous_response_id: None,
        metadata: request.metadata.clone(),
        parallel_tool_calls: request.parallel_tool_calls,
        temperature: request.temperature,
        top_p: request.top_p,
        max_output_tokens: request.max_output_tokens,
        reasoning: request.reasoning.clone(),
        text: request.text.clone(),
        tool_choice: request.tool_choice.clone(),
        tools: request.tools.clone(),
        truncation: request.truncation.clone(),
        user: request.user.clone(),
        ayatori_client_id: String::new(),
    }
}

pub(crate) fn make_in_progress_response(
    request: &CreateResponseRequest,
    model: String,
) -> ResponseObject {
    let mut response = make_response(request, model, vec![], None, None);
    response.status = ResponseStatus::InProgress;
    response
}

pub(crate) fn usage(
    input_tokens: u32,
    cached_tokens: Option<u32>,
    output_tokens: u32,
    reasoning_tokens: Option<u32>,
) -> ResponseUsage {
    ResponseUsage {
        input_tokens,
        input_tokens_details: cached_tokens
            .map(|cached_tokens| InputTokensDetails { cached_tokens }),
        output_tokens,
        output_tokens_details: reasoning_tokens
            .map(|reasoning_tokens| OutputTokensDetails { reasoning_tokens }),
        total_tokens: input_tokens + output_tokens,
    }
}

pub(crate) fn incomplete_for_max_tokens(reason: Option<&str>) -> Option<IncompleteDetails> {
    matches!(reason, Some("max_tokens") | Some("MAX_TOKENS")).then(|| IncompleteDetails {
        reason: "max_output_tokens".to_string(),
    })
}

pub(crate) fn reasoning_budget(
    max_output_tokens: Option<u32>,
    reasoning: Option<&ReasoningConfig>,
) -> Option<u32> {
    let reasoning = reasoning?;
    let budget_hint = match reasoning.effort.as_deref() {
        Some("minimal") | Some("low") => 1_024,
        Some("medium") | None => 2_048,
        Some("high") => 4_096,
        Some("none") => return None,
        Some(_) => 2_048,
    };
    let max_tokens = max_output_tokens.unwrap_or(4_096);
    Some(budget_hint.min(max_tokens.saturating_sub(1)).max(1))
}

pub(crate) fn response_format(text: Option<&TextConfig>) -> (Option<&'static str>, Option<Value>) {
    match text.and_then(|config| config.format.as_ref()) {
        Some(crate::types::TextFormat::JsonObject) => (Some("application/json"), None),
        Some(crate::types::TextFormat::JsonSchema { schema, .. }) => {
            (Some("application/json"), Some(schema.clone()))
        }
        _ => (None, None),
    }
}

pub(crate) fn tool_name_from_call_id(call_id: &str) -> String {
    call_id
        .split_once("::")
        .map(|(name, _)| name.to_string())
        .unwrap_or_else(|| call_id.to_string())
}

pub(crate) fn tool_call_id(name: &str, index: usize) -> String {
    format!("{name}::{index}")
}

pub(crate) fn tool_choice_mode(choice: &ToolChoice) -> Option<&str> {
    match choice {
        ToolChoice::Mode(mode) => Some(mode.as_str()),
        ToolChoice::Specific { .. } => None,
    }
}

pub(crate) fn function_tools(
    tools: Option<&[ToolDefinition]>,
) -> Vec<(&str, Option<&str>, Option<&Value>)> {
    let mut out = Vec::new();
    for tool in tools.unwrap_or(&[]) {
        match tool {
            ToolDefinition::Function {
                name,
                description,
                parameters,
                ..
            } => out.push((name.as_str(), description.as_deref(), parameters.as_ref())),
        }
    }
    out
}
