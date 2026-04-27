use actix_web::App;
use actix_web::http::StatusCode;
use actix_web::test;
use actix_web::web::Data;
use configuration::{CapacityLimits, Configuration, LlmProvider, LlmProviderType};
use llm_selector::{LlmSelector, UsageStoreConfig};
use serde_json::{Value, json};
use server::{ApiKey, AppConfig, configure_openai_compatible_endpoints};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use token_measure::{ByteLengthTokenMeasure, TokenMeasure};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

static NEXT_ID: AtomicU64 = AtomicU64::new(10_000);

#[derive(Clone)]
struct TestProvider {
    id: &'static str,
    provider_type: LlmProviderType,
    model: &'static str,
    endpoint: String,
}

fn unique_path(name: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ayatori-{name}-{id}.toml"))
}

fn write_credential_file(providers: &[TestProvider]) -> String {
    let path = unique_path("credential-non-openai");
    let mut content = String::new();
    for provider in providers {
        content.push_str(&format!("[{}]\n", provider.id));
        match provider.provider_type {
            LlmProviderType::Anthropic => {
                content.push_str("type = \"Anthropic\"\napi_key = \"test-key\"\n\n");
            }
            LlmProviderType::VertexAI => {
                content.push_str("type = \"VertexAI\"\napi_key = \"test-key\"\n\n");
            }
            ref other => panic!("unexpected provider type for non-openai test: {other:?}"),
        }
    }
    fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

fn build_configuration(providers: Vec<TestProvider>) -> Configuration {
    let credential_file = write_credential_file(&providers);
    Configuration {
        providers: providers
            .into_iter()
            .map(|provider| LlmProvider {
                id: provider.id.to_string(),
                default: Some(true),
                provider_type: provider.provider_type,
                responses_native: Some(false),
                priority: 0,
                model: provider.model.to_string(),
                tags: vec![],
                credential_file: credential_file.clone(),
                endpoint: provider.endpoint,
                capacity: CapacityLimits {
                    input_tokens: None,
                    requests: None,
                },
            })
            .collect(),
    }
}

async fn build_app(
    providers: Vec<TestProvider>,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse,
    Error = actix_web::Error,
> {
    let configuration = build_configuration(providers);
    let store = UsageStoreConfig::Local.create().await;
    let selector = LlmSelector::new(configuration, store);

    test::init_service(
        App::new()
            .app_data(Data::new(selector))
            .app_data(Data::new(ApiKey::new(None)))
            .app_data(Data::new(AppConfig::new(true)))
            .app_data(Data::new(TokenMeasure::new(ByteLengthTokenMeasure::new(
                0.3,
            ))))
            .configure(configure_openai_compatible_endpoints),
    )
    .await
}

#[actix_web::test]
async fn post_responses_accepts_anthropic_provider() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "test-key"))
        .and(body_json(json!({
            "model": "claude-3-7-sonnet",
            "max_tokens": 4096,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "text",
                    "text": "hello"
                }]
            }],
            "stream": false
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "text", "text": "hello from claude" }],
            "model": "claude-3-7-sonnet",
            "stop_reason": "end_turn",
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5
            }
        })))
        .mount(&mock_server)
        .await;

    let app = build_app(vec![TestProvider {
        id: "anthropic-provider",
        provider_type: LlmProviderType::Anthropic,
        model: "claude-3-7-sonnet",
        endpoint: mock_server.uri(),
    }])
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "claude-3-7-sonnet",
            "input": "hello"
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["output_text"], "hello from claude");
    assert_eq!(body["ayatori_client_id"], "anthropic-provider");
}

#[actix_web::test]
async fn post_responses_accepts_vertex_provider() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/v1/publishers/google/models/gemini-2.5-flash:generateContent",
        ))
        .and(query_param("key", "test-key"))
        .and(body_json(json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": "what is the weather?" }]
            }],
            "tools": [{
                "function_declarations": [{
                    "name": "lookup_weather",
                    "description": "Lookup the weather",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "city": { "type": "string" }
                        }
                    }
                }]
            }],
            "toolConfig": {
                "functionCallingConfig": {
                    "mode": "ANY"
                }
            }
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "responseId": "resp_vertex",
            "modelVersion": "gemini-2.5-flash-001",
            "candidates": [{
                "finishReason": "STOP",
                "content": {
                    "role": "model",
                    "parts": [{
                        "functionCall": {
                            "name": "lookup_weather",
                            "args": { "city": "Tokyo" }
                        }
                    }]
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 12,
                "candidatesTokenCount": 4,
                "totalTokenCount": 16
            }
        })))
        .mount(&mock_server)
        .await;

    let app = build_app(vec![TestProvider {
        id: "vertex-provider",
        provider_type: LlmProviderType::VertexAI,
        model: "publishers/google/models/gemini-2.5-flash",
        endpoint: mock_server.uri(),
    }])
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "publishers/google/models/gemini-2.5-flash",
            "input": "what is the weather?",
            "tools": [{
                "type": "function",
                "name": "lookup_weather",
                "description": "Lookup the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    }
                }
            }],
            "tool_choice": "required"
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["output"][0]["type"], "function_call");
    assert_eq!(body["output"][0]["call_id"], "lookup_weather::0");
    assert_eq!(body["ayatori_client_id"], "vertex-provider");
}
