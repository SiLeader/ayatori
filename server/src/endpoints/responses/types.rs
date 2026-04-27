#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub(super) struct CreateResponseRequest {
    pub model: String,
    pub input: ResponseInput,
    pub instructions: Option<String>,
    pub previous_response_id: Option<String>,
    #[serde(default)]
    pub store: Option<bool>,
    #[serde(default)]
    pub background: Option<bool>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(default)]
    pub tool_choice: Option<ToolChoice>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u32>,
    pub reasoning: Option<ReasoningConfig>,
    pub text: Option<TextConfig>,
    pub metadata: Option<HashMap<String, String>>,
    pub user: Option<String>,
    #[serde(default)]
    pub parallel_tool_calls: Option<bool>,
    pub truncation: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum ResponseInput {
    Text(String),
    Items(Vec<InputItem>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum InputItem {
    Message(InputMessage),
    FunctionCall(FunctionCallItem),
    FunctionCallOutput(FunctionCallOutput),
    Reasoning(ReasoningItem),
    ItemReference { id: String },
}

#[derive(Debug, Deserialize)]
pub(super) struct InputMessage {
    pub role: String,
    pub content: MessageContentInput,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum MessageContentInput {
    Text(String),
    Parts(Vec<ContentPartInput>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ContentPartInput {
    InputText {
        text: String,
    },
    InputImage {
        image_url: Option<String>,
        file_id: Option<String>,
        detail: Option<String>,
    },
    InputFile {
        file_id: Option<String>,
        file_data: Option<String>,
    },
    OutputText {
        text: String,
        annotations: Option<serde_json::Value>,
    },
    Refusal {
        refusal: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FunctionCallItem {
    pub id: String,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FunctionCallOutput {
    pub call_id: String,
    pub output: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ReasoningItem {
    pub id: String,
    #[serde(default)]
    pub summary: Vec<SummaryPart>,
    #[serde(default)]
    pub encrypted_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum SummaryPart {
    #[serde(rename = "summary_text")]
    Text { text: String },
}

#[derive(Debug, Serialize)]
pub(super) struct ResponseObject {
    pub id: String,
    pub object: &'static str,
    pub created_at: i64,
    pub status: ResponseStatus,
    pub model: String,
    pub output: Vec<OutputItem>,
    pub output_text: Option<String>,
    pub usage: Option<ResponseUsage>,
    pub error: Option<ResponseError>,
    pub incomplete_details: Option<IncompleteDetails>,
    pub previous_response_id: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
    pub parallel_tool_calls: Option<bool>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_output_tokens: Option<u32>,
    pub reasoning: Option<ReasoningConfig>,
    pub text: Option<TextConfig>,
    pub tool_choice: Option<ToolChoice>,
    pub tools: Option<Vec<ToolDefinition>>,
    pub truncation: Option<String>,
    pub user: Option<String>,
    pub ayatori_client_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ResponseStatus {
    Completed,
    InProgress,
    Failed,
    Cancelled,
    Queued,
    Incomplete,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum OutputItem {
    Message(OutputMessage),
    FunctionCall(FunctionCallItem),
    Reasoning(ReasoningItem),
}

#[derive(Debug, Serialize)]
pub(super) struct OutputMessage {
    pub id: String,
    pub status: String,
    pub role: String,
    pub content: Vec<ContentPartOutput>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ContentPartOutput {
    OutputText {
        text: String,
        annotations: Vec<serde_json::Value>,
    },
    Refusal {
        refusal: String,
    },
}

#[derive(Debug, Serialize)]
pub(super) struct ResponseUsage {
    pub input_tokens: u32,
    pub input_tokens_details: Option<InputTokensDetails>,
    pub output_tokens: u32,
    pub output_tokens_details: Option<OutputTokensDetails>,
    pub total_tokens: u32,
}

#[derive(Debug, Serialize)]
pub(super) struct InputTokensDetails {
    pub cached_tokens: u32,
}

#[derive(Debug, Serialize)]
pub(super) struct OutputTokensDetails {
    pub reasoning_tokens: u32,
}

#[derive(Debug, Serialize)]
pub(super) struct ResponseError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub(super) struct IncompleteDetails {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ToolDefinition {
    Function {
        name: String,
        description: Option<String>,
        parameters: Option<serde_json::Value>,
        strict: Option<bool>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum ToolChoice {
    Mode(String),
    Specific {
        #[serde(rename = "type")]
        tool_type: String,
        name: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ReasoningConfig {
    pub effort: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TextConfig {
    pub format: Option<TextFormat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum TextFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        strict: Option<bool>,
        description: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_string_input() {
        let request: CreateResponseRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-4.1",
            "input": "hello"
        }))
        .unwrap();

        match request.input {
            ResponseInput::Text(text) => assert_eq!(text, "hello"),
            ResponseInput::Items(_) => panic!("expected text input"),
        }
    }

    #[test]
    fn deserializes_item_array_input() {
        let request: CreateResponseRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-4.1",
            "input": [{
                "type": "message",
                "role": "user",
                "content": "hello"
            }]
        }))
        .unwrap();

        match request.input {
            ResponseInput::Items(items) => assert_eq!(items.len(), 1),
            ResponseInput::Text(_) => panic!("expected items input"),
        }
    }

    #[test]
    fn deserializes_message_item() {
        let item: InputItem = serde_json::from_value(serde_json::json!({
            "type": "message",
            "role": "developer",
            "content": [{"type": "input_text", "text": "be concise"}]
        }))
        .unwrap();

        match item {
            InputItem::Message(message) => {
                assert_eq!(message.role, "developer");
                match message.content {
                    MessageContentInput::Parts(parts) => assert_eq!(parts.len(), 1),
                    MessageContentInput::Text(_) => panic!("expected parts"),
                }
            }
            _ => panic!("expected message item"),
        }
    }

    #[test]
    fn deserializes_function_call_item() {
        let item: InputItem = serde_json::from_value(serde_json::json!({
            "type": "function_call",
            "id": "fc_123",
            "call_id": "call_123",
            "name": "lookup_weather",
            "arguments": "{\"city\":\"Tokyo\"}",
            "status": "completed"
        }))
        .unwrap();

        match item {
            InputItem::FunctionCall(call) => {
                assert_eq!(call.call_id, "call_123");
                assert_eq!(call.name, "lookup_weather");
            }
            _ => panic!("expected function_call item"),
        }
    }

    #[test]
    fn deserializes_function_call_output_item() {
        let item: InputItem = serde_json::from_value(serde_json::json!({
            "type": "function_call_output",
            "call_id": "call_123",
            "output": "{\"temperature\":24}"
        }))
        .unwrap();

        match item {
            InputItem::FunctionCallOutput(output) => {
                assert_eq!(output.call_id, "call_123");
                assert_eq!(output.output, "{\"temperature\":24}");
            }
            _ => panic!("expected function_call_output item"),
        }
    }

    #[test]
    fn deserializes_reasoning_item() {
        let item: InputItem = serde_json::from_value(serde_json::json!({
            "type": "reasoning",
            "id": "rs_123",
            "summary": [{ "type": "summary_text", "text": "brief summary" }],
            "encrypted_content": "opaque"
        }))
        .unwrap();

        match item {
            InputItem::Reasoning(reasoning) => {
                assert_eq!(reasoning.id, "rs_123");
                assert_eq!(reasoning.summary.len(), 1);
                assert_eq!(reasoning.encrypted_content.as_deref(), Some("opaque"));
            }
            _ => panic!("expected reasoning item"),
        }
    }

    #[test]
    fn deserializes_item_reference() {
        let item: InputItem =
            serde_json::from_value(serde_json::json!({ "type": "item_reference", "id": "msg_1" }))
                .unwrap();

        match item {
            InputItem::ItemReference { id } => assert_eq!(id, "msg_1"),
            _ => panic!("expected item_reference item"),
        }
    }

    #[test]
    fn deserializes_tool_choice_mode() {
        let choice: ToolChoice = serde_json::from_value(serde_json::json!("auto")).unwrap();

        match choice {
            ToolChoice::Mode(mode) => assert_eq!(mode, "auto"),
            ToolChoice::Specific { .. } => panic!("expected mode"),
        }
    }

    #[test]
    fn deserializes_tool_choice_specific_tool() {
        let choice: ToolChoice = serde_json::from_value(serde_json::json!({
            "type": "function",
            "name": "lookup_weather"
        }))
        .unwrap();

        match choice {
            ToolChoice::Specific { tool_type, name } => {
                assert_eq!(tool_type, "function");
                assert_eq!(name.as_deref(), Some("lookup_weather"));
            }
            ToolChoice::Mode(_) => panic!("expected specific tool"),
        }
    }

    #[test]
    fn deserializes_text_format_variants() {
        let text: TextConfig = serde_json::from_value(serde_json::json!({
            "format": { "type": "text" }
        }))
        .unwrap();
        assert!(matches!(text.format, Some(TextFormat::Text)));

        let json_object: TextConfig = serde_json::from_value(serde_json::json!({
            "format": { "type": "json_object" }
        }))
        .unwrap();
        assert!(matches!(json_object.format, Some(TextFormat::JsonObject)));

        let json_schema: TextConfig = serde_json::from_value(serde_json::json!({
            "format": {
                "type": "json_schema",
                "name": "answer",
                "schema": { "type": "object" },
                "strict": true
            }
        }))
        .unwrap();
        match json_schema.format {
            Some(TextFormat::JsonSchema {
                name,
                schema,
                strict,
                description,
            }) => {
                assert_eq!(name, "answer");
                assert_eq!(schema["type"], "object");
                assert_eq!(strict, Some(true));
                assert_eq!(description, None);
            }
            _ => panic!("expected json_schema"),
        }
    }
}
