use super::types::{CreateResponseRequest, ResponseObject, ResponseStatus};
use crate::error::ErrorResponse;
use crate::model::RequestModel;
use crate::{ApiKey, AppConfig};
use actix_web::web::{Data, Json};
use actix_web::{HttpResponse, post};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::Utc;
use llm_selector::LlmSelector;
use uuid::Uuid;

#[post("/v1/responses")]
pub(crate) async fn handle_create_response(
    selector: Data<LlmSelector>,
    api_key: Data<ApiKey>,
    bearer_auth: Option<BearerAuth>,
    app_config: Data<AppConfig>,
    request: Json<CreateResponseRequest>,
) -> HttpResponse {
    if let Err(e) = api_key.check_api_key(bearer_auth) {
        return e.into();
    }

    let model = RequestModel::from(request.model.clone());
    let client = match model.select_model(&selector).await {
        Some(client) => client,
        None if app_config.client_fallback_enabled => selector.get_default_client(),
        None => return ErrorResponse::model_not_found().into(),
    };
    let (id, _client) = client;

    // TODO(phase2): replace this stub with a provider-backed implementation.
    let response = ResponseObject {
        id: format!(
            "resp_{}",
            BASE64_URL_SAFE_NO_PAD.encode(Uuid::new_v4().as_bytes())
        ),
        object: "response",
        created_at: Utc::now().timestamp(),
        status: ResponseStatus::Completed,
        model: request.model.clone(),
        output: vec![],
        output_text: Some("(stub: not yet implemented)".to_string()),
        usage: None,
        error: None,
        incomplete_details: None,
        previous_response_id: request.previous_response_id.clone(),
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
        ayatori_client_id: id,
    };

    HttpResponse::Ok().json(response)
}
