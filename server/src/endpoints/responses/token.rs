use super::{
    ContentPartInput, CreateResponseRequest, InputItem, MessageContentInput, ResponseInput,
};
use token_measure::{MeasureToken, TokenMeasure};

pub(super) async fn measure_input_tokens(
    request: &CreateResponseRequest,
    id: &str,
    token_measure: &TokenMeasure,
) -> u64 {
    let mut total = 0;

    if let Some(instructions) = &request.instructions {
        total += token_measure
            .measure_token(id, instructions)
            .await
            .unwrap_or(0);
    }

    match &request.input {
        ResponseInput::Text(text) => {
            total += token_measure.measure_token(id, text).await.unwrap_or(0);
        }
        ResponseInput::Items(items) => {
            for item in items {
                total += measure_item_tokens(item, id, token_measure).await;
            }
        }
    }

    total
}

async fn measure_item_tokens(item: &InputItem, id: &str, token_measure: &TokenMeasure) -> u64 {
    match item {
        InputItem::Message(message) => match &message.content {
            MessageContentInput::Text(text) => {
                token_measure.measure_token(id, text).await.unwrap_or(0)
            }
            MessageContentInput::Parts(parts) => {
                let mut total = 0;
                for part in parts {
                    if let ContentPartInput::InputText { text } = part {
                        total += token_measure.measure_token(id, text).await.unwrap_or(0);
                    }
                }
                total
            }
        },
        InputItem::FunctionCallOutput(output) => token_measure
            .measure_token(id, &output.output)
            .await
            .unwrap_or(0),
        _ => 0,
    }
}
