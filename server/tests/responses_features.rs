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
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static NEXT_ID: AtomicU64 = AtomicU64::new(40_000);

fn unique_path(name: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ayatori-{name}-{id}.toml"))
}

fn write_credential_file(provider_id: &str, provider_type: &LlmProviderType) -> String {
    let path = unique_path("credential-features");
    let content = match provider_type {
        LlmProviderType::OpenAI => format!(
            r#"[{provider_id}]
type = "OpenAI"
api_key = "test-key"
"#
        ),
        LlmProviderType::Anthropic => format!(
            r#"[{provider_id}]
type = "Anthropic"
api_key = "test-key"
"#
        ),
        LlmProviderType::Ollama => format!(
            r#"[{provider_id}]
type = "Ollama"
"#
        ),
        other => panic!("unsupported provider type for test: {other:?}"),
    };
    fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

async fn build_app(
    endpoint: String,
    provider_type: LlmProviderType,
    responses_native: bool,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse,
    Error = actix_web::Error,
> {
    let provider_id = "provider";
    let configuration = Configuration {
        providers: vec![LlmProvider {
            id: provider_id.to_string(),
            default: Some(true),
            provider_type: provider_type.clone(),
            responses_native: Some(responses_native),
            priority: 0,
            model: "model-1".to_string(),
            tags: vec![],
            credential_file: write_credential_file(provider_id, &provider_type),
            endpoint,
            capacity: CapacityLimits {
                input_tokens: None,
                requests: None,
            },
        }],
    };
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

fn message_response(id: &str, text: &str) -> Value {
    json!({
        "id": id,
        "object": "response",
        "created_at": 1,
        "status": "completed",
        "model": "model-1",
        "output": [{
            "type": "message",
            "id": format!("msg_{id}"),
            "status": "completed",
            "role": "assistant",
            "content": [{
                "type": "output_text",
                "text": text,
                "annotations": []
            }]
        }]
    })
}

#[actix_web::test]
async fn openai_reasoning_and_json_schema_are_passed_through() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_json(json!({
            "model": "model-1",
            "input": "extract structured data",
            "reasoning": {
                "effort": "medium",
                "summary": "auto"
            },
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "weather_response",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "city": { "type": "string" }
                        },
                        "required": ["city"]
                    },
                    "strict": true
                }
            }
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(message_response("resp_reasoning", "{\"city\":\"Tokyo\"}")),
        )
        .mount(&mock_server)
        .await;

    let app = build_app(mock_server.uri(), LlmProviderType::OpenAI, true).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "model-1",
            "input": "extract structured data",
            "reasoning": {
                "effort": "medium",
                "summary": "auto"
            },
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "weather_response",
                    "schema": {
                        "type": "object",
                        "properties": {
                            "city": { "type": "string" }
                        },
                        "required": ["city"]
                    },
                    "strict": true
                }
            }
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["output_text"], "{\"city\":\"Tokyo\"}");
    assert_eq!(body["ayatori_client_id"], "provider");
}

#[actix_web::test]
async fn openai_image_input_is_passed_through() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_json(json!({
            "model": "model-1",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "describe this image"
                    },
                    {
                        "type": "input_image",
                        "image_url": "data:image/png;base64,AAAA",
                        "detail": "low"
                    }
                ]
            }]
        })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(message_response("resp_image", "a tiny image")),
        )
        .mount(&mock_server)
        .await;

    let app = build_app(mock_server.uri(), LlmProviderType::OpenAI, true).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "model-1",
            "input": [{
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "describe this image"
                    },
                    {
                        "type": "input_image",
                        "image_url": "data:image/png;base64,AAAA",
                        "detail": "low"
                    }
                ]
            }]
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["output_text"], "a tiny image");
}

#[actix_web::test]
async fn anthropic_structured_output_is_rejected() {
    let app = build_app(
        "http://localhost:9".to_string(),
        LlmProviderType::Anthropic,
        false,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "model-1",
            "input": "return json",
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "result",
                    "schema": {
                        "type": "object"
                    }
                }
            }
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "feature_not_supported");
    assert_eq!(
        body["error"]["message"],
        "Feature not supported: structured_output"
    );
}

#[actix_web::test]
async fn ollama_reasoning_is_rejected() {
    let app = build_app(
        "http://localhost:11434".to_string(),
        LlmProviderType::Ollama,
        false,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "model-1",
            "input": "think step by step",
            "reasoning": {
                "effort": "high"
            }
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["error"]["code"], "feature_not_supported");
    assert_eq!(body["error"]["message"], "Feature not supported: reasoning");
}
