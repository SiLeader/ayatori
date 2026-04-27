use super::CreateResponseRequest;
use crate::error::ErrorResponse;
use crate::model::RequestModel;
use crate::{ApiKey, AppConfig};
use actix_web::web::{Data, Json};
use actix_web::{HttpResponse, post};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use llm_responses::LlmResponsesComposer;
use llm_selector::LlmSelector;

#[post("/v1/responses")]
pub(crate) async fn handle_create_response(
    selector: Data<LlmSelector>,
    responses_composer: Data<LlmResponsesComposer>,
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
    let Some((_, provider)) = responses_composer.get_by_id(&id) else {
        return ErrorResponse::model_not_found().into();
    };

    let mut response = match provider.create_response(request.into_inner()).await {
        Ok(response) => response,
        Err(error) => return ErrorResponse::from(error).into(),
    };
    response.ayatori_client_id = id;
    response.ensure_output_text();

    HttpResponse::Ok().json(response)
}
