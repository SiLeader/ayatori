use super::token::measure_input_tokens;
use super::{
    ContentPartInput, CreateResponseRequest, InputItem, ResponseError, ResponseInput,
    ResponseObject, ResponseStatus, ResponseStreamEvent, TextFormat,
};
use crate::error::ErrorResponse;
use crate::model::RequestModel;
use crate::{ApiKey, AppConfig};
use actix_web::web::{Bytes, Data, Json, Path};
use actix_web::{HttpResponse, delete, get, post};
use actix_web_httpauth::extractors::bearer::BearerAuth;
use futures::StreamExt;
use llm_responses::{ProviderCapabilities, ResponseStore, ResponsesProvider, StoreError};
use llm_selector::{LlmSelector, Usage};
use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use token_measure::TokenMeasure;
use tracing::error;
use uuid::Uuid;

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

    let mut request = request.into_inner();
    if request.background.unwrap_or(false) && matches!(request.store, Some(false)) {
        return ErrorResponse::from(llm_responses::ResponsesError::InvalidRequest(
            "background requests require store=true".to_string(),
        ))
        .into();
    }

    if let Err(response) =
        merge_previous_response_input(&app_config.response_store, &mut request).await
    {
        return response.into();
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

    if request.background.unwrap_or(false) {
        return handle_background(
            provider,
            id,
            request,
            selector,
            usage,
            app_config.response_store.clone(),
            app_config.response_store_ttl,
        )
        .await;
    }

    if request.stream.unwrap_or(false) {
        return handle_streaming(provider, id, request, selector, usage, app_config).await;
    }

    let result = provider.create_response(provider_request(&request)).await;

    if let Err(e) = selector.remove_usage(&id, &usage).await {
        error!("remove_usage failed: {e:?}");
    }

    let mut response = match result {
        Ok(response) => response,
        Err(error) => return ErrorResponse::from(error).into(),
    };
    apply_response_metadata(&mut response, &id, &request);

    if request.store.unwrap_or(true)
        && let Err(store_error) = store_response(
            &app_config.response_store,
            app_config.response_store_ttl,
            &response,
            &request,
        )
        .await
    {
        return store_error.into();
    }

    HttpResponse::Ok().json(response)
}

#[get("/v1/responses/{response_id}")]
pub(crate) async fn handle_get_response(
    api_key: Data<ApiKey>,
    bearer_auth: Option<BearerAuth>,
    app_config: Data<AppConfig>,
    response_id: Path<String>,
) -> HttpResponse {
    if let Err(e) = api_key.check_api_key(bearer_auth) {
        return e.into();
    }

    let response_id = response_id.into_inner();
    match app_config.response_store.get(&response_id).await {
        Ok(Some(response)) => HttpResponse::Ok().json(response),
        Ok(None) => ErrorResponse::response_not_found().into(),
        Err(error) => map_store_error(error).into(),
    }
}

#[delete("/v1/responses/{response_id}")]
pub(crate) async fn handle_delete_response(
    api_key: Data<ApiKey>,
    bearer_auth: Option<BearerAuth>,
    app_config: Data<AppConfig>,
    response_id: Path<String>,
) -> HttpResponse {
    if let Err(e) = api_key.check_api_key(bearer_auth) {
        return e.into();
    }

    let response_id = response_id.into_inner();
    match app_config.response_store.delete(&response_id).await {
        Ok(true) => HttpResponse::Ok().json(DeleteResponseResult {
            id: response_id,
            object: "response.deleted",
            deleted: true,
        }),
        Ok(false) => ErrorResponse::response_not_found().into(),
        Err(error) => map_store_error(error).into(),
    }
}

#[post("/v1/responses/{response_id}/cancel")]
pub(crate) async fn handle_cancel_response(
    api_key: Data<ApiKey>,
    bearer_auth: Option<BearerAuth>,
    selector: Data<LlmSelector>,
    app_config: Data<AppConfig>,
    response_id: Path<String>,
) -> HttpResponse {
    if let Err(e) = api_key.check_api_key(bearer_auth) {
        return e.into();
    }

    let response_id = response_id.into_inner();
    let Some(mut response) = (match app_config.response_store.get(&response_id).await {
        Ok(response) => response,
        Err(error) => return map_store_error(error).into(),
    }) else {
        return ErrorResponse::response_not_found().into();
    };

    if !matches!(
        response.status,
        ResponseStatus::Queued | ResponseStatus::InProgress
    ) {
        return ErrorResponse::conflict("response is not cancellable in its current state").into();
    }

    if !response.id.starts_with("resp_bg_")
        && !response.ayatori_client_id.is_empty()
        && let Some((_, provider)) = selector
            .select_responses_provider_by_id(&response.ayatori_client_id)
            .await
        && provider.capabilities().cancel_response
    {
        match provider.cancel_response(&response.id).await {
            Ok(mut cancelled) => {
                apply_response_metadata(
                    &mut cancelled,
                    &response.ayatori_client_id,
                    &CreateResponseRequest {
                        model: response.model.clone(),
                        input: ResponseInput::Items(vec![]),
                        instructions: None,
                        previous_response_id: response.previous_response_id.clone(),
                        store: Some(true),
                        background: Some(true),
                        stream: None,
                        tools: response.tools.clone(),
                        tool_choice: response.tool_choice.clone(),
                        temperature: response.temperature,
                        top_p: response.top_p,
                        max_output_tokens: response.max_output_tokens,
                        reasoning: response.reasoning.clone(),
                        text: response.text.clone(),
                        metadata: response.metadata.clone(),
                        user: response.user.clone(),
                        parallel_tool_calls: response.parallel_tool_calls,
                        truncation: response.truncation.clone(),
                    },
                );
                if let Err(error) = app_config
                    .response_store
                    .put(&cancelled, app_config.response_store_ttl)
                    .await
                {
                    return map_store_error(error).into();
                }
                return HttpResponse::Ok().json(cancelled);
            }
            Err(error) => return ErrorResponse::from(error).into(),
        }
    }

    response.status = ResponseStatus::Cancelled;
    response.error = None;
    if let Err(error) = app_config
        .response_store
        .put(&response, app_config.response_store_ttl)
        .await
    {
        return map_store_error(error).into();
    }

    HttpResponse::Ok().json(response)
}

#[get("/v1/responses/{response_id}/input_items")]
pub(crate) async fn handle_list_input_items(
    api_key: Data<ApiKey>,
    bearer_auth: Option<BearerAuth>,
    app_config: Data<AppConfig>,
    response_id: Path<String>,
) -> HttpResponse {
    if let Err(e) = api_key.check_api_key(bearer_auth) {
        return e.into();
    }

    let response_id = response_id.into_inner();
    match app_config.response_store.rebuild_input(&response_id).await {
        Ok(items) => HttpResponse::Ok().json(InputItemsList {
            object: "list",
            data: items,
        }),
        Err(StoreError::NotFound) => ErrorResponse::response_not_found().into(),
        Err(error) => map_store_error(error).into(),
    }
}

async fn handle_background(
    provider: Arc<dyn ResponsesProvider>,
    client_id: String,
    request: CreateResponseRequest,
    selector: Data<LlmSelector>,
    usage: Usage,
    response_store: Arc<dyn ResponseStore>,
    response_store_ttl: Option<std::time::Duration>,
) -> HttpResponse {
    let response_id = format!("resp_bg_{}", Uuid::new_v4().simple());
    let mut queued = queued_response(&response_id, &request, &client_id);

    if let Err(error) = store_response(&response_store, response_store_ttl, &queued, &request).await
    {
        if let Err(remove_error) = selector.remove_usage(&client_id, &usage).await {
            error!("remove_usage failed: {remove_error:?}");
        }
        return error.into();
    }

    let selector = selector.into_inner();
    let response_store_clone = response_store.clone();
    let request_clone = request.clone();
    let client_id_clone = client_id.clone();
    let response_id_clone = response_id.clone();
    tokio::spawn(async move {
        let result = provider
            .create_response(provider_request(&request_clone))
            .await;

        let should_skip = match response_store_clone.get(&response_id_clone).await {
            Ok(Some(existing)) => matches!(existing.status, ResponseStatus::Cancelled),
            Ok(None) => true,
            Err(error) => {
                tracing::error!("background store get failed: {error}");
                false
            }
        };

        if !should_skip {
            let mut response = match result {
                Ok(mut response) => {
                    response.id = response_id_clone.clone();
                    response.status = ResponseStatus::Completed;
                    apply_response_metadata(&mut response, &client_id_clone, &request_clone);
                    response
                }
                Err(error) => {
                    failed_response(&response_id_clone, &request_clone, &client_id_clone, &error)
                }
            };
            response.ensure_output_text();

            if let Err(error) = response_store_clone
                .put(&response, response_store_ttl)
                .await
            {
                tracing::error!("background store put failed: {error}");
            }
        }

        if let Err(error) = selector.remove_usage(&client_id_clone, &usage).await {
            tracing::error!("remove_usage failed: {error:?}");
        }
    });

    queued.ensure_output_text();
    HttpResponse::Accepted().json(queued)
}

async fn handle_streaming(
    provider: Arc<dyn ResponsesProvider>,
    client_id: String,
    request: CreateResponseRequest,
    selector: Data<LlmSelector>,
    usage: Usage,
    app_config: Data<AppConfig>,
) -> HttpResponse {
    let stream = match provider
        .create_response_stream(provider_request(&request))
        .await
    {
        Ok(stream) => stream,
        Err(error) => {
            if let Err(remove_error) = selector.remove_usage(&client_id, &usage).await {
                error!("remove_usage failed: {remove_error:?}");
            }
            return ErrorResponse::from(error).into();
        }
    };

    let selector_clone = selector.into_inner();
    let id_clone = client_id.clone();
    let usage_released = Arc::new(AtomicBool::new(false));
    let store = app_config.response_store.clone();
    let ttl = app_config.response_store_ttl;
    let store_request = request.clone();
    let store_enabled = request.store.unwrap_or(true);

    let sse_stream = stream.then(move |item| {
        let selector_clone = selector_clone.clone();
        let id_clone = id_clone.clone();
        let usage = usage.clone();
        let usage_released = usage_released.clone();
        let store = store.clone();
        let store_request = store_request.clone();

        async move {
            let release_usage = |selector: Arc<LlmSelector>, id: String, usage: Usage| {
                tokio::spawn(async move {
                    if let Err(error) = selector.remove_usage(&id, &usage).await {
                        tracing::error!("remove_usage failed: {error:?}");
                    }
                });
            };

            let mut event = match item {
                Ok(event) => event,
                Err(error) => {
                    if !usage_released.swap(true, Ordering::AcqRel) {
                        release_usage(selector_clone.clone(), id_clone.clone(), usage.clone());
                    }
                    ResponseStreamEvent::Error {
                        error: ResponseError::from_responses_error(&error),
                    }
                }
            };

            if let Some(response) = event.response_mut() {
                apply_response_metadata(response, &id_clone, &store_request);
            }

            if !usage_released.load(Ordering::Acquire)
                && matches!(
                    event,
                    ResponseStreamEvent::Completed { .. }
                        | ResponseStreamEvent::Failed { .. }
                        | ResponseStreamEvent::Incomplete { .. }
                        | ResponseStreamEvent::Error { .. }
                )
                && !usage_released.swap(true, Ordering::AcqRel)
            {
                release_usage(selector_clone.clone(), id_clone.clone(), usage.clone());
            }

            if store_enabled
                && let Some(response) = event_terminal_response(&event)
                && let Err(error) = store_response(&store, ttl, response, &store_request).await
            {
                tracing::error!("stream response store failed: {error:?}");
            }

            let event_name = event.event_name();
            let data = serde_json::to_string(&event).unwrap_or_else(|serialize_error| {
                serde_json::json!({
                    "type": "error",
                    "error": {
                        "code": "serialization_error",
                        "message": serialize_error.to_string(),
                    }
                })
                .to_string()
            });

            Ok::<Bytes, actix_web::Error>(Bytes::from(format!(
                "event: {event_name}\ndata: {data}\n\n"
            )))
        }
    });

    HttpResponse::Ok()
        .content_type("text/event-stream")
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("X-Accel-Buffering", "no"))
        .streaming(sse_stream)
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

    if request_uses_builtin_tools(request) && !capabilities.builtin_tools {
        return Err(ErrorResponse::feature_not_supported("builtin_tools"));
    }

    Ok(())
}

async fn merge_previous_response_input(
    response_store: &Arc<dyn ResponseStore>,
    request: &mut CreateResponseRequest,
) -> Result<(), ErrorResponse> {
    let Some(previous_response_id) = request.previous_response_id.clone() else {
        return Ok(());
    };

    let prior_input = match response_store.rebuild_input(&previous_response_id).await {
        Ok(input) => input,
        Err(StoreError::NotFound) => return Err(ErrorResponse::response_not_found()),
        Err(error) => return Err(map_store_error(error)),
    };

    let mut merged = prior_input;
    merged.extend(request_input_items(&request.input));
    request.input = ResponseInput::Items(merged);
    Ok(())
}

async fn store_response(
    response_store: &Arc<dyn ResponseStore>,
    ttl: Option<std::time::Duration>,
    response: &ResponseObject,
    request: &CreateResponseRequest,
) -> Result<(), ErrorResponse> {
    response_store
        .put(response, ttl)
        .await
        .map_err(map_store_error)?;
    response_store
        .append_input_chain(&response.id, request)
        .await
        .map_err(map_store_error)?;
    Ok(())
}

fn apply_response_metadata(
    response: &mut ResponseObject,
    client_id: &str,
    request: &CreateResponseRequest,
) {
    response.ayatori_client_id = client_id.to_string();
    response.previous_response_id = request.previous_response_id.clone();
    response.ensure_output_text();
}

fn queued_response(
    response_id: &str,
    request: &CreateResponseRequest,
    client_id: &str,
) -> ResponseObject {
    let mut response = base_response(response_id, request);
    response.status = ResponseStatus::Queued;
    response.ayatori_client_id = client_id.to_string();
    response
}

fn failed_response(
    response_id: &str,
    request: &CreateResponseRequest,
    client_id: &str,
    error: &llm_responses::ResponsesError,
) -> ResponseObject {
    let mut response = base_response(response_id, request);
    response.status = ResponseStatus::Failed;
    response.error = Some(ResponseError::from_responses_error(error));
    response.ayatori_client_id = client_id.to_string();
    response
}

fn base_response(response_id: &str, request: &CreateResponseRequest) -> ResponseObject {
    ResponseObject {
        id: response_id.to_string(),
        object: "response".to_string(),
        created_at: chrono::Utc::now().timestamp(),
        status: ResponseStatus::InProgress,
        model: request.model.clone(),
        output: vec![],
        output_text: None,
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
        ayatori_client_id: String::new(),
    }
}

fn event_terminal_response(event: &ResponseStreamEvent) -> Option<&ResponseObject> {
    match event {
        ResponseStreamEvent::Completed { response }
        | ResponseStreamEvent::Failed { response }
        | ResponseStreamEvent::Incomplete { response } => Some(response),
        _ => None,
    }
}

fn request_input_items(input: &ResponseInput) -> Vec<InputItem> {
    match input {
        ResponseInput::Text(text) => vec![InputItem::Message(super::InputMessage {
            role: "user".to_string(),
            content: super::MessageContentInput::Text(text.clone()),
        })],
        ResponseInput::Items(items) => items.clone(),
    }
}

fn provider_request(request: &CreateResponseRequest) -> CreateResponseRequest {
    let mut request = request.clone();
    request.previous_response_id = None;
    request.store = None;
    request.background = None;
    request
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

fn request_uses_builtin_tools(request: &CreateResponseRequest) -> bool {
    request.tools.as_ref().is_some_and(|tools| {
        tools
            .iter()
            .any(|tool| !matches!(tool, super::ToolDefinition::Function { .. }))
    })
}

fn map_store_error(error: StoreError) -> ErrorResponse {
    match error {
        StoreError::NotFound => ErrorResponse::response_not_found(),
        StoreError::Internal(message) => ErrorResponse::from(
            llm_responses::ResponsesError::Internal(format!("response store: {message}")),
        ),
    }
}

#[derive(Debug, Serialize)]
struct DeleteResponseResult<'a> {
    id: String,
    object: &'a str,
    deleted: bool,
}

#[derive(Debug, Serialize)]
struct InputItemsList<'a> {
    object: &'a str,
    data: Vec<InputItem>,
}
