use super::token::measure_input_tokens;
use super::{ContentPartInput, CreateResponseRequest, InputItem, ResponseInput, TextFormat};
use crate::error::ErrorResponse;
use crate::model::RequestModel;
use crate::{ApiKey, AppConfig};
use actix_web::web::{Data, Json};
use actix_web::{HttpResponse, post};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use llm_responses::ProviderCapabilities;
use llm_selector::{LlmSelector, Usage};
use token_measure::TokenMeasure;
use tracing::error;

#[post("/v1/responses")]
pub(crate) async fn handle_create_response(
    selector: Data<LlmSelector>,
    api_key: Data<ApiKey>,
    bearer_auth: Option<BearerAuth>,
    app_config: Data<AppConfig>,
    token_measure: Data<TokenMeasure>,
    request: Json<CreateResponseRequest>,
) -> HttpResponse {
    if let Err(e) = api_key.check_api_key(bearer_auth) {
        return e.into();
    }

    let request = request.into_inner();

    if request.stream.unwrap_or(false) {
        return ErrorResponse::feature_not_supported("streaming").into();
    }

    if request.background.unwrap_or(false) || request.store.unwrap_or(false) {
        return ErrorResponse::feature_not_supported("background/store").into();
    }
    if request.previous_response_id.is_some() {
        return ErrorResponse::feature_not_supported("previous_response_id").into();
    }

    let model = RequestModel::from(request.model.clone());
    let provider = match model.select_responses_provider(&selector).await {
        Some(provider) => provider,
        None if app_config.client_fallback_enabled => selector.get_default_responses_provider(),
        None => return ErrorResponse::model_not_found().into(),
    };
    let (id, provider) = provider;

    if let Err(e) = check_capabilities(&request, &provider.capabilities()) {
        return e.into();
    }

    let usage = Usage {
        input_tokens: measure_input_tokens(&request, &id, &token_measure).await,
        requests: 1,
    };

    if let Err(e) = selector.append_usage(&id, &usage).await {
        error!("append_usage failed: {e:?}");
    }

    let result = provider.create_response(request).await;

    if let Err(e) = selector.remove_usage(&id, &usage).await {
        error!("remove_usage failed: {e:?}");
    }

    let mut response = match result {
        Ok(response) => response,
        Err(error) => return ErrorResponse::from(error).into(),
    };
    response.ayatori_client_id = id;
    response.ensure_output_text();

    HttpResponse::Ok().json(response)
}

fn check_capabilities(
    request: &CreateResponseRequest,
    capabilities: &ProviderCapabilities,
) -> Result<(), ErrorResponse> {
    if request.reasoning.is_some() && !capabilities.reasoning {
        return Err(ErrorResponse::feature_not_supported("reasoning"));
    }

    if request
        .text
        .as_ref()
        .and_then(|text| text.format.as_ref())
        .is_some_and(|format| {
            matches!(
                format,
                TextFormat::JsonObject | TextFormat::JsonSchema { .. }
            )
        })
        && !capabilities.structured_output
    {
        return Err(ErrorResponse::feature_not_supported("structured_output"));
    }

    if request_uses_image_input(&request.input) && !capabilities.image_input {
        return Err(ErrorResponse::feature_not_supported("image_input"));
    }

    Ok(())
}

fn request_uses_image_input(input: &ResponseInput) -> bool {
    let ResponseInput::Items(items) = input else {
        return false;
    };

    items.iter().any(|item| match item {
        InputItem::Message(message) => match &message.content {
            super::MessageContentInput::Text(_) => false,
            super::MessageContentInput::Parts(parts) => parts.iter().any(|part| {
                matches!(
                    part,
                    ContentPartInput::InputImage { .. } | ContentPartInput::InputFile { .. }
                )
            }),
        },
        _ => false,
    })
}
