use actix_web::App;
use actix_web::http::StatusCode;
use actix_web::test;
use actix_web::web::Data;
use configuration::{CapacityLimits, Configuration, LlmProvider, LlmProviderType};
use llm_responses::LlmResponsesComposer;
use llm_selector::{LlmSelector, UsageStoreConfig};
use serde_json::{Value, json};
use server::{ApiKey, AppConfig, configure_openai_compatible_endpoints};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use token_measure::{ByteLengthTokenMeasure, TokenMeasure};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn unique_path(name: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ayatori-{name}-{id}.toml"))
}

fn write_credential_file() -> String {
    let path = unique_path("credential");
    fs::write(
        &path,
        r#"[stub-model]
type = "OpenAI"
api_key = "test-key"
"#,
    )
    .unwrap();
    path.to_string_lossy().into_owned()
}

fn build_configuration(model: &str, endpoint: String) -> Configuration {
    Configuration {
        providers: vec![LlmProvider {
            id: "stub-model".to_string(),
            default: Some(true),
            provider_type: LlmProviderType::OpenAI,
            responses_native: Some(true),
            priority: 0,
            model: model.to_string(),
            tags: vec!["test".to_string()],
            credential_file: write_credential_file(),
            endpoint,
            capacity: CapacityLimits {
                input_tokens: None,
                requests: None,
            },
        }],
    }
}

async fn build_app(
    model: &str,
    endpoint: String,
    api_key: Option<&str>,
    client_fallback_enabled: bool,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse,
    Error = actix_web::Error,
> {
    let configuration = build_configuration(model, endpoint);
    let responses_composer = LlmResponsesComposer::new(configuration.clone());
    let store = UsageStoreConfig::Local.create().await;
    let selector = LlmSelector::new(configuration, store);

    test::init_service(
        App::new()
            .app_data(Data::new(selector))
            .app_data(Data::new(responses_composer))
            .app_data(Data::new(ApiKey::new(api_key.map(str::to_string))))
            .app_data(Data::new(AppConfig::new(client_fallback_enabled)))
            .app_data(Data::new(TokenMeasure::new(ByteLengthTokenMeasure::new(
                0.3,
            ))))
            .configure(configure_openai_compatible_endpoints),
    )
    .await
}

fn minimal_request(model: &str) -> Value {
    json!({
        "model": model,
        "input": "hello"
    })
}

fn upstream_response() -> Value {
    json!({
        "id": "resp_upstream",
        "object": "response",
        "created_at": 1,
        "status": "completed",
        "model": "gpt-4.1-mini",
        "output": [{
            "type": "message",
            "id": "msg_1",
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": "hello from upstream",
                "annotations": []
            }]
        }]
    })
}

async fn mount_openai_mock(server: &MockServer, provider_model: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_json(json!({
            "model": provider_model,
            "input": "hello"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(upstream_response()))
        .mount(server)
        .await;
}

#[actix_web::test]
async fn post_responses_returns_provider_response() {
    let mock_server = MockServer::start().await;
    mount_openai_mock(&mock_server, "gpt-4.1-mini").await;

    let app = build_app("gpt-4.1-mini", mock_server.uri(), None, true).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(minimal_request("gpt-4.1-mini"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["id"], "resp_upstream");
    assert_eq!(body["object"], "response");
    assert_eq!(body["output_text"], "hello from upstream");
}

#[actix_web::test]
async fn response_includes_ayatori_client_id() {
    let mock_server = MockServer::start().await;
    mount_openai_mock(&mock_server, "gpt-4.1-mini").await;

    let app = build_app("gpt-4.1-mini", mock_server.uri(), None, true).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(minimal_request("gpt-4.1-mini"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["ayatori_client_id"], "stub-model");
}

#[actix_web::test]
async fn missing_bearer_token_returns_401_when_api_key_is_required() {
    let mock_server = MockServer::start().await;
    let app = build_app("gpt-4.1-mini", mock_server.uri(), Some("secret"), true).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(minimal_request("gpt-4.1-mini"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "invalid_authentication");
}

#[actix_web::test]
async fn unknown_model_returns_404_when_fallback_is_disabled() {
    let mock_server = MockServer::start().await;
    let app = build_app("gpt-4.1-mini", mock_server.uri(), None, false).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(minimal_request("missing-model"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "model_not_found");
}

#[actix_web::test]
async fn unknown_model_uses_default_client_when_fallback_is_enabled() {
    let mock_server = MockServer::start().await;
    mount_openai_mock(&mock_server, "gpt-4.1-mini").await;

    let app = build_app("gpt-4.1-mini", mock_server.uri(), None, true).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(minimal_request("missing-model"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["ayatori_client_id"], "stub-model");
}
