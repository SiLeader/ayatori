use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateResponseRequest {
    pub model: String,
    pub input: ResponseInput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseInput {
    Text(String),
    Items(Vec<InputItem>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputItem {
    Message(InputMessage),
    FunctionCall(FunctionCallItem),
    FunctionCallOutput(FunctionCallOutput),
    Reasoning(ReasoningItem),
    ItemReference { id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputMessage {
    pub role: String,
    pub content: MessageContentInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContentInput {
    Text(String),
    Parts(Vec<ContentPartInput>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPartInput {
    InputText {
        text: String,
    },
    InputImage {
        #[serde(skip_serializing_if = "Option::is_none")]
        image_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    InputFile {
        #[serde(skip_serializing_if = "Option::is_none")]
        file_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_data: Option<String>,
    },
    OutputText {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        annotations: Option<serde_json::Value>,
    },
    Refusal {
        refusal: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallItem {
    pub id: String,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallOutput {
    pub call_id: String,
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningItem {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<SummaryPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SummaryPart {
    #[serde(rename = "summary_text")]
    Text { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseObject {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    pub status: ResponseStatus,
    pub model: String,
    pub output: Vec<OutputItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ResponseUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ResponseError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<IncompleteDetails>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default)]
    pub ayatori_client_id: String,
}

impl ResponseObject {
    pub fn ensure_output_text(&mut self) {
        if self.output_text.is_some() {
            return;
        }

        let mut text = String::new();
        for item in &self.output {
            if let OutputItem::Message(message) = item {
                for part in &message.content {
                    if let ContentPartOutput::OutputText { text: part, .. } = part {
                        text.push_str(part);
                    }
                }
            }
        }

        if !text.is_empty() {
            self.output_text = Some(text);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Completed,
    InProgress,
    Failed,
    Cancelled,
    Queued,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputItem {
    Message(OutputMessage),
    FunctionCall(FunctionCallItem),
    Reasoning(ReasoningItem),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputMessage {
    pub id: String,
    pub status: String,
    pub role: String,
    pub content: Vec<ContentPartOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPartOutput {
    OutputText {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        annotations: Vec<serde_json::Value>,
    },
    Refusal {
        refusal: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseUsage {
    pub input_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<InputTokensDetails>,
    pub output_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OutputTokensDetails>,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputTokensDetails {
    pub cached_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    pub reasoning_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseError {
    pub code: String,
    pub message: String,
}

impl ResponseError {
    pub fn from_responses_error(error: &crate::ResponsesError) -> Self {
        match error {
            crate::ResponsesError::Http { status, body } => Self {
                code: format!("http_{status}"),
                message: body.clone(),
            },
            crate::ResponsesError::Transport(error) => Self {
                code: "transport_error".to_string(),
                message: error.to_string(),
            },
            crate::ResponsesError::Serde(error) => Self {
                code: "serde_error".to_string(),
                message: error.to_string(),
            },
            crate::ResponsesError::Unsupported(feature) => Self {
                code: "unsupported".to_string(),
                message: format!("Feature not supported: {feature}"),
            },
            crate::ResponsesError::InvalidRequest(message) => Self {
                code: "invalid_request".to_string(),
                message: message.clone(),
            },
            crate::ResponsesError::MalformedResponse(message) => Self {
                code: "malformed_response".to_string(),
                message: message.clone(),
            },
            crate::ResponsesError::Authentication => Self {
                code: "authentication_error".to_string(),
                message: "Invalid authentication".to_string(),
            },
            crate::ResponsesError::Internal(message) => Self {
                code: "internal_error".to_string(),
                message: message.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncompleteDetails {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolDefinition {
    Function {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parameters: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(String),
    Specific {
        #[serde(rename = "type")]
        tool_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<TextFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TextFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponseStreamEvent {
    #[serde(rename = "response.created")]
    Created { response: ResponseObject },

    #[serde(rename = "response.in_progress")]
    InProgress { response: ResponseObject },

    #[serde(rename = "response.output_item.added")]
    OutputItemAdded { output_index: u32, item: OutputItem },

    #[serde(rename = "response.output_item.done")]
    OutputItemDone { output_index: u32, item: OutputItem },

    #[serde(rename = "response.content_part.added")]
    ContentPartAdded {
        item_id: String,
        output_index: u32,
        content_index: u32,
        part: ContentPartOutput,
    },

    #[serde(rename = "response.content_part.done")]
    ContentPartDone {
        item_id: String,
        output_index: u32,
        content_index: u32,
        part: ContentPartOutput,
    },

    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        item_id: String,
        output_index: u32,
        content_index: u32,
        delta: String,
    },

    #[serde(rename = "response.output_text.done")]
    OutputTextDone {
        item_id: String,
        output_index: u32,
        content_index: u32,
        text: String,
    },

    #[serde(rename = "response.refusal.delta")]
    RefusalDelta {
        item_id: String,
        output_index: u32,
        content_index: u32,
        delta: String,
    },

    #[serde(rename = "response.refusal.done")]
    RefusalDone {
        item_id: String,
        output_index: u32,
        content_index: u32,
        refusal: String,
    },

    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        item_id: String,
        output_index: u32,
        delta: String,
    },

    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone {
        item_id: String,
        output_index: u32,
        arguments: String,
    },

    #[serde(rename = "response.reasoning_summary_part.added")]
    ReasoningSummaryPartAdded {
        item_id: String,
        output_index: u32,
        summary_index: u32,
        part: SummaryPart,
    },

    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta {
        item_id: String,
        output_index: u32,
        summary_index: u32,
        delta: String,
    },

    #[serde(rename = "response.reasoning_summary_text.done")]
    ReasoningSummaryTextDone {
        item_id: String,
        output_index: u32,
        summary_index: u32,
        text: String,
    },

    #[serde(rename = "response.reasoning_summary_part.done")]
    ReasoningSummaryPartDone {
        item_id: String,
        output_index: u32,
        summary_index: u32,
        part: SummaryPart,
    },

    #[serde(rename = "response.completed")]
    Completed { response: ResponseObject },

    #[serde(rename = "response.failed")]
    Failed { response: ResponseObject },

    #[serde(rename = "response.incomplete")]
    Incomplete { response: ResponseObject },

    #[serde(rename = "error")]
    Error { error: ResponseError },
}

impl ResponseStreamEvent {
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Created { .. } => "response.created",
            Self::InProgress { .. } => "response.in_progress",
            Self::OutputItemAdded { .. } => "response.output_item.added",
            Self::OutputItemDone { .. } => "response.output_item.done",
            Self::ContentPartAdded { .. } => "response.content_part.added",
            Self::ContentPartDone { .. } => "response.content_part.done",
            Self::OutputTextDelta { .. } => "response.output_text.delta",
            Self::OutputTextDone { .. } => "response.output_text.done",
            Self::RefusalDelta { .. } => "response.refusal.delta",
            Self::RefusalDone { .. } => "response.refusal.done",
            Self::FunctionCallArgumentsDelta { .. } => "response.function_call_arguments.delta",
            Self::FunctionCallArgumentsDone { .. } => "response.function_call_arguments.done",
            Self::ReasoningSummaryPartAdded { .. } => "response.reasoning_summary_part.added",
            Self::ReasoningSummaryTextDelta { .. } => "response.reasoning_summary_text.delta",
            Self::ReasoningSummaryTextDone { .. } => "response.reasoning_summary_text.done",
            Self::ReasoningSummaryPartDone { .. } => "response.reasoning_summary_part.done",
            Self::Completed { .. } => "response.completed",
            Self::Failed { .. } => "response.failed",
            Self::Incomplete { .. } => "response.incomplete",
            Self::Error { .. } => "error",
        }
    }

    pub fn response_mut(&mut self) -> Option<&mut ResponseObject> {
        match self {
            Self::Created { response }
            | Self::InProgress { response }
            | Self::Completed { response }
            | Self::Failed { response }
            | Self::Incomplete { response } => Some(response),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ContentPartOutput, OutputItem, OutputMessage, ResponseObject, ResponseStatus,
        ResponseStreamEvent,
    };

    fn response() -> ResponseObject {
        ResponseObject {
            id: "resp_123".to_string(),
            object: "response".to_string(),
            created_at: 1,
            status: ResponseStatus::InProgress,
            model: "gpt-test".to_string(),
            output: vec![OutputItem::Message(OutputMessage {
                id: "msg_123".to_string(),
                status: "in_progress".to_string(),
                role: "assistant".to_string(),
                content: vec![ContentPartOutput::OutputText {
                    text: String::new(),
                    annotations: vec![],
                }],
            })],
            output_text: None,
            usage: None,
            error: None,
            incomplete_details: None,
            previous_response_id: None,
            metadata: None,
            parallel_tool_calls: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            reasoning: None,
            text: None,
            tool_choice: None,
            tools: None,
            truncation: None,
            user: None,
            ayatori_client_id: String::new(),
        }
    }

    #[test]
    fn response_stream_event_round_trips() {
        let event = ResponseStreamEvent::Created {
            response: response(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let restored: ResponseStreamEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.event_name(), "response.created");
        match restored {
            ResponseStreamEvent::Created { response } => {
                assert_eq!(response.id, "resp_123");
                assert_eq!(response.model, "gpt-test");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
