use crate::common::{incomplete_for_max_tokens, make_in_progress_response, new_id, usage};
use crate::types::{
    ContentPartOutput, CreateResponseRequest, FunctionCallItem, OutputItem, OutputMessage,
    ReasoningItem, ResponseStatus, ResponseStreamEvent, ResponseUsage, SummaryPart,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

pub(crate) struct AnthropicStreamMapper {
    response: crate::types::ResponseObject,
    current_message_output_index: Option<u32>,
    blocks: HashMap<u32, BlockState>,
    stop_reason: Option<String>,
}

impl AnthropicStreamMapper {
    pub(crate) fn new(model: String, request: &CreateResponseRequest) -> Self {
        Self {
            response: make_in_progress_response(request, model),
            current_message_output_index: None,
            blocks: HashMap::new(),
            stop_reason: None,
        }
    }

    pub(crate) fn handle(
        &mut self,
        event: AnthropicStreamPayload,
    ) -> Vec<ResponseStreamEvent> {
        match event {
            AnthropicStreamPayload::MessageStart { message } => {
                if let Some(id) = message.id {
                    self.response.id = id;
                }
                if let Some(model) = message.model {
                    self.response.model = model;
                }
                self.update_usage(message.usage.as_ref());
                vec![
                    ResponseStreamEvent::Created {
                        response: self.response.clone(),
                    },
                    ResponseStreamEvent::InProgress {
                        response: self.response.clone(),
                    },
                ]
            }
            AnthropicStreamPayload::ContentBlockStart {
                index,
                content_block,
            } => self.handle_block_start(index, content_block),
            AnthropicStreamPayload::ContentBlockDelta { index, delta } => {
                self.handle_block_delta(index, delta)
            }
            AnthropicStreamPayload::ContentBlockStop { index } => self.handle_block_stop(index),
            AnthropicStreamPayload::MessageDelta { delta, usage } => {
                self.stop_reason = delta.stop_reason;
                self.update_usage(usage.as_ref());
                Vec::new()
            }
            AnthropicStreamPayload::MessageStop => self.handle_message_stop(),
            AnthropicStreamPayload::Ping => Vec::new(),
            AnthropicStreamPayload::Error { error } => vec![ResponseStreamEvent::Error {
                error: crate::types::ResponseError {
                    code: error.error_type,
                    message: error.message,
                },
            }],
        }
    }

    fn handle_block_start(
        &mut self,
        index: u32,
        content_block: AnthropicContentBlock,
    ) -> Vec<ResponseStreamEvent> {
        match content_block {
            AnthropicContentBlock::Text { text } => {
                let mut events = Vec::new();
                let (output_index, item_id) = if let Some(output_index) = self.current_message_output_index
                {
                    (
                        output_index,
                        self.message_item(output_index)
                            .map(|message| message.id.clone())
                            .unwrap_or_else(|| new_id("msg")),
                    )
                } else {
                    let message = OutputMessage {
                        id: new_id("msg"),
                        status: "in_progress".to_string(),
                        role: "assistant".to_string(),
                        content: Vec::new(),
                    };
                    let output_index = self.push_output(OutputItem::Message(message.clone()));
                    self.current_message_output_index = Some(output_index);
                    events.push(ResponseStreamEvent::OutputItemAdded {
                        output_index,
                        item: OutputItem::Message(message.clone()),
                    });
                    (output_index, message.id)
                };

                let part = ContentPartOutput::OutputText {
                    text: text.clone().unwrap_or_default(),
                    annotations: vec![],
                };
                let content_index = {
                    let message = self
                        .message_item_mut(output_index)
                        .expect("message item must exist");
                    let content_index = message.content.len() as u32;
                    message.content.push(part.clone());
                    content_index
                };
                events.push(ResponseStreamEvent::ContentPartAdded {
                    item_id: item_id.clone(),
                    output_index,
                    content_index,
                    part: part.clone(),
                });
                self.blocks.insert(
                    index,
                    BlockState::Text {
                        item_id,
                        output_index,
                        content_index,
                        text: text.unwrap_or_default(),
                    },
                );
                events
            }
            AnthropicContentBlock::ToolUse { id, name, input } => {
                self.current_message_output_index = None;
                let arguments = input.unwrap_or(Value::Object(Default::default())).to_string();
                let item = FunctionCallItem {
                    id: new_id("fc"),
                    call_id: id,
                    name,
                    arguments: arguments.clone(),
                    status: "in_progress".to_string(),
                };
                let output_index = self.push_output(OutputItem::FunctionCall(item.clone()));
                self.blocks.insert(
                    index,
                    BlockState::ToolUse {
                        item_id: item.id.clone(),
                        output_index,
                        arguments,
                    },
                );
                vec![ResponseStreamEvent::OutputItemAdded {
                    output_index,
                    item: OutputItem::FunctionCall(item),
                }]
            }
            AnthropicContentBlock::Thinking { thinking, signature } => {
                let item = ReasoningItem {
                    id: new_id("rs"),
                    summary: vec![SummaryPart::Text {
                        text: thinking.clone().unwrap_or_default(),
                    }],
                    encrypted_content: signature,
                };
                let output_index = self.push_output(OutputItem::Reasoning(item.clone()));
                self.blocks.insert(
                    index,
                    BlockState::Thinking {
                        item_id: item.id.clone(),
                        output_index,
                        summary_index: 0,
                        text: thinking.unwrap_or_default(),
                    },
                );
                vec![
                    ResponseStreamEvent::OutputItemAdded {
                        output_index,
                        item: OutputItem::Reasoning(item.clone()),
                    },
                    ResponseStreamEvent::ReasoningSummaryPartAdded {
                        item_id: item.id,
                        output_index,
                        summary_index: 0,
                        part: SummaryPart::Text {
                            text: item
                                .summary
                                .first()
                                .map(|part| match part {
                                    SummaryPart::Text { text } => text.clone(),
                                })
                                .unwrap_or_default(),
                        },
                    },
                ]
            }
        }
    }

    fn handle_block_delta(
        &mut self,
        index: u32,
        delta: AnthropicDelta,
    ) -> Vec<ResponseStreamEvent> {
        match delta {
            AnthropicDelta::TextDelta { text: delta } => {
                let Some(BlockState::Text {
                    item_id,
                    output_index,
                    content_index,
                    text,
                }) = self.blocks.get_mut(&index)
                else {
                    return Vec::new();
                };
                text.push_str(&delta);
                let item_id = item_id.clone();
                let output_index = *output_index;
                let content_index = *content_index;
                let updated_text = text.clone();
                if let Some(ContentPartOutput::OutputText { text, .. }) = self
                    .message_item_mut(output_index)
                    .and_then(|message| message.content.get_mut(content_index as usize))
                {
                    *text = updated_text;
                }
                vec![ResponseStreamEvent::OutputTextDelta {
                    item_id,
                    output_index,
                    content_index,
                    delta,
                }]
            }
            AnthropicDelta::InputJsonDelta { partial_json } => {
                let Some(BlockState::ToolUse {
                    item_id,
                    output_index,
                    arguments,
                }) = self.blocks.get_mut(&index)
                else {
                    return Vec::new();
                };
                arguments.push_str(&partial_json);
                let item_id = item_id.clone();
                let output_index = *output_index;
                let arguments = arguments.clone();
                if let Some(OutputItem::FunctionCall(call)) =
                    self.response.output.get_mut(output_index as usize)
                {
                    call.arguments = arguments;
                }
                vec![ResponseStreamEvent::FunctionCallArgumentsDelta {
                    item_id,
                    output_index,
                    delta: partial_json,
                }]
            }
            AnthropicDelta::ThinkingDelta { thinking } => {
                let Some(BlockState::Thinking {
                    item_id,
                    output_index,
                    summary_index,
                    text,
                }) = self.blocks.get_mut(&index)
                else {
                    return Vec::new();
                };
                text.push_str(&thinking);
                let item_id = item_id.clone();
                let output_index = *output_index;
                let summary_index = *summary_index;
                let updated_text = text.clone();
                if let Some(OutputItem::Reasoning(reasoning)) =
                    self.response.output.get_mut(output_index as usize)
                    && let Some(SummaryPart::Text { text }) = reasoning.summary.first_mut()
                {
                    *text = updated_text;
                }
                vec![ResponseStreamEvent::ReasoningSummaryTextDelta {
                    item_id,
                    output_index,
                    summary_index,
                    delta: thinking,
                }]
            }
            AnthropicDelta::SignatureDelta { signature } => {
                let Some(BlockState::Thinking { output_index, .. }) = self.blocks.get(&index)
                else {
                    return Vec::new();
                };
                if let Some(OutputItem::Reasoning(reasoning)) =
                    self.response.output.get_mut(*output_index as usize)
                {
                    reasoning.encrypted_content = Some(signature);
                }
                Vec::new()
            }
        }
    }

    fn handle_block_stop(&mut self, index: u32) -> Vec<ResponseStreamEvent> {
        let Some(state) = self.blocks.remove(&index) else {
            return Vec::new();
        };

        match state {
            BlockState::Text {
                item_id,
                output_index,
                content_index,
                text,
            } => {
                let part = self
                    .message_item(output_index)
                    .and_then(|message| message.content.get(content_index as usize))
                    .cloned()
                    .unwrap_or(ContentPartOutput::OutputText {
                        text: text.clone(),
                        annotations: vec![],
                    });
                vec![
                    ResponseStreamEvent::OutputTextDone {
                        item_id: item_id.clone(),
                        output_index,
                        content_index,
                        text,
                    },
                    ResponseStreamEvent::ContentPartDone {
                        item_id,
                        output_index,
                        content_index,
                        part,
                    },
                ]
            }
            BlockState::ToolUse {
                item_id,
                output_index,
                mut arguments,
            } => {
                if arguments.is_empty() {
                    arguments = "{}".to_string();
                }
                let mut item = None;
                if let Some(OutputItem::FunctionCall(call)) =
                    self.response.output.get_mut(output_index as usize)
                {
                    call.arguments = arguments.clone();
                    call.status = "completed".to_string();
                    item = Some(OutputItem::FunctionCall(call.clone()));
                }
                let item = item.unwrap_or(OutputItem::FunctionCall(FunctionCallItem {
                    id: item_id.clone(),
                    call_id: String::new(),
                    name: String::new(),
                    arguments: arguments.clone(),
                    status: "completed".to_string(),
                }));
                vec![
                    ResponseStreamEvent::FunctionCallArgumentsDone {
                        item_id,
                        output_index,
                        arguments,
                    },
                    ResponseStreamEvent::OutputItemDone { output_index, item },
                ]
            }
            BlockState::Thinking {
                item_id,
                output_index,
                summary_index,
                text,
            } => vec![
                ResponseStreamEvent::ReasoningSummaryTextDone {
                    item_id: item_id.clone(),
                    output_index,
                    summary_index,
                    text: text.clone(),
                },
                ResponseStreamEvent::ReasoningSummaryPartDone {
                    item_id: item_id.clone(),
                    output_index,
                    summary_index,
                    part: SummaryPart::Text { text },
                },
                ResponseStreamEvent::OutputItemDone {
                    output_index,
                    item: self
                        .response
                        .output
                        .get(output_index as usize)
                        .cloned()
                        .unwrap_or(OutputItem::Reasoning(ReasoningItem {
                            id: item_id,
                            summary: vec![],
                            encrypted_content: None,
                        })),
                },
            ],
        }
    }

    fn handle_message_stop(&mut self) -> Vec<ResponseStreamEvent> {
        let mut events = Vec::new();

        for (index, item) in self.response.output.iter_mut().enumerate() {
            if let OutputItem::Message(message) = item
                && message.status != "completed"
            {
                message.status = "completed".to_string();
                events.push(ResponseStreamEvent::OutputItemDone {
                    output_index: index as u32,
                    item: OutputItem::Message(message.clone()),
                });
            }
        }

        self.response.incomplete_details = incomplete_for_max_tokens(self.stop_reason.as_deref());
        self.response.status = if self.response.incomplete_details.is_some() {
            ResponseStatus::Incomplete
        } else {
            ResponseStatus::Completed
        };
        self.response.ensure_output_text();

        let final_event = if self.response.incomplete_details.is_some() {
            ResponseStreamEvent::Incomplete {
                response: self.response.clone(),
            }
        } else {
            ResponseStreamEvent::Completed {
                response: self.response.clone(),
            }
        };
        events.push(final_event);
        events
    }

    fn push_output(&mut self, item: OutputItem) -> u32 {
        let output_index = self.response.output.len() as u32;
        self.response.output.push(item);
        output_index
    }

    fn message_item(&self, output_index: u32) -> Option<&OutputMessage> {
        match self.response.output.get(output_index as usize) {
            Some(OutputItem::Message(message)) => Some(message),
            _ => None,
        }
    }

    fn message_item_mut(&mut self, output_index: u32) -> Option<&mut OutputMessage> {
        match self.response.output.get_mut(output_index as usize) {
            Some(OutputItem::Message(message)) => Some(message),
            _ => None,
        }
    }

    fn update_usage(&mut self, usage_delta: Option<&AnthropicUsage>) {
        let Some(usage_delta) = usage_delta else {
            return;
        };

        let current = self
            .response
            .usage
            .clone()
            .unwrap_or_else(|| usage(0, None, 0, None));
        let input_tokens = current.input_tokens + usage_delta.input_tokens.unwrap_or_default();
        let output_tokens = current.output_tokens + usage_delta.output_tokens.unwrap_or_default();
        let cached_tokens = usage_delta
            .cache_read_input_tokens
            .or_else(|| current.input_tokens_details.map(|details| details.cached_tokens));

        self.response.usage = Some(ResponseUsage {
            total_tokens: input_tokens + output_tokens,
            ..usage(input_tokens, cached_tokens, output_tokens, None)
        });
    }
}

enum BlockState {
    Text {
        item_id: String,
        output_index: u32,
        content_index: u32,
        text: String,
    },
    ToolUse {
        item_id: String,
        output_index: u32,
        arguments: String,
    },
    Thinking {
        item_id: String,
        output_index: u32,
        summary_index: u32,
        text: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AnthropicStreamPayload {
    MessageStart {
        message: AnthropicMessageStart,
    },
    ContentBlockStart {
        index: u32,
        content_block: AnthropicContentBlock,
    },
    ContentBlockDelta {
        index: u32,
        delta: AnthropicDelta,
    },
    ContentBlockStop {
        index: u32,
    },
    MessageDelta {
        delta: AnthropicMessageDelta,
        #[serde(default)]
        usage: Option<AnthropicUsage>,
    },
    MessageStop,
    Ping,
    Error {
        error: AnthropicErrorPayload,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicMessageStart {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AnthropicContentBlock {
    Text {
        #[serde(default)]
        text: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Option<Value>,
    },
    Thinking {
        #[serde(default)]
        thinking: Option<String>,
        #[serde(default)]
        signature: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AnthropicDelta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
    ThinkingDelta { thinking: String },
    SignatureDelta { signature: String },
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicMessageDelta {
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicUsage {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicErrorPayload {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}
