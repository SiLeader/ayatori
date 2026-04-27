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

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct TestProvider {
    id: &'static str,
    default: bool,
    model: &'static str,
    tags: Vec<&'static str>,
    priority: usize,
    endpoint: String,
}

fn unique_path(name: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ayatori-{name}-{id}.toml"))
}

fn write_credential_file(provider_ids: &[&str]) -> String {
    let path = unique_path("credential");
    let mut content = String::new();
    for provider_id in provider_ids {
        content.push_str(&format!(
            r#"[{provider_id}]
type = "OpenAI"
api_key = "test-key"

"#
        ));
    }
    fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

fn build_configuration(providers: Vec<TestProvider>) -> Configuration {
    let credential_file = write_credential_file(
        &providers
            .iter()
            .map(|provider| provider.id)
            .collect::<Vec<_>>(),
    );

    Configuration {
        providers: providers
            .into_iter()
            .map(|provider| LlmProvider {
                id: provider.id.to_string(),
                default: Some(provider.default),
                provider_type: LlmProviderType::OpenAI,
                responses_native: Some(true),
                priority: provider.priority,
                model: provider.model.to_string(),
                tags: provider.tags.into_iter().map(str::to_string).collect(),
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
    api_key: Option<&str>,
    client_fallback_enabled: bool,
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
            .app_data(Data::new(ApiKey::new(api_key.map(str::to_string))))
            .app_data(Data::new(AppConfig::new(client_fallback_enabled)))
            .app_data(Data::new(TokenMeasure::new(ByteLengthTokenMeasure::new(
                0.3,
            ))))
            .configure(configure_openai_compatible_endpoints),
    )
    .await
}

fn provider(
    id: &'static str,
    default: bool,
    model: &'static str,
    tags: Vec<&'static str>,
    priority: usize,
    endpoint: String,
) -> TestProvider {
    TestProvider {
        id,
        default,
        model,
        tags,
        priority,
        endpoint,
    }
}

fn minimal_request(model: &str) -> Value {
    json!({
        "model": model,
        "input": "hello"
    })
}

fn message_response(text: &str) -> Value {
    json!({
        "id": "resp_upstream",
        "object": "response",
        "created_at": 1,
        "status": "completed",
        "model": "provider-model",
        "output": [{
            "type": "message",
            "id": "msg_1",
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

fn parse_sse_events(body: &[u8]) -> Vec<(String, Value)> {
    String::from_utf8_lossy(body)
        .split("\n\n")
        .filter(|chunk| !chunk.trim().is_empty())
        .map(|chunk| {
            let mut event = String::new();
            let mut data = String::new();
            for line in chunk.lines() {
                if let Some(value) = line.strip_prefix("event: ") {
                    event = value.to_string();
                } else if let Some(value) = line.strip_prefix("data: ") {
                    data.push_str(value);
                }
            }
            (event, serde_json::from_str(&data).unwrap())
        })
        .collect()
}

async fn mount_openai_mock(server: &MockServer, expected_body: Value, response_body: Value) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_json(expected_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(response_body))
        .mount(server)
        .await;
}

async fn mount_openai_stream_mock(server: &MockServer, expected_body: Value, response_body: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_json(expected_body))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(response_body),
        )
        .mount(server)
        .await;
}

#[actix_web::test]
async fn post_responses_returns_provider_response() {
    let mock_server = MockServer::start().await;
    mount_openai_mock(
        &mock_server,
        json!({
            "model": "gpt-4.1-mini",
            "input": "hello"
        }),
        message_response("hello from upstream"),
    )
    .await;

    let app = build_app(
        vec![provider(
            "stub-model",
            true,
            "gpt-4.1-mini",
            vec!["test"],
            0,
            mock_server.uri(),
        )],
        None,
        true,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(minimal_request("gpt-4.1-mini"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["id"], "resp_upstream");
    assert_eq!(body["output_text"], "hello from upstream");
    assert_eq!(body["ayatori_client_id"], "stub-model");
}

#[actix_web::test]
async fn post_responses_streams_openai_events() {
    let mock_server = MockServer::start().await;
    let upstream_stream = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_stream\",\"object\":\"response\",\"created_at\":1,\"status\":\"in_progress\",\"model\":\"gpt-4.1-mini\",\"output\":[]}}\n\n",
        "event: response.in_progress\n",
        "data: {\"type\":\"response.in_progress\",\"response\":{\"id\":\"resp_stream\",\"object\":\"response\",\"created_at\":1,\"status\":\"in_progress\",\"model\":\"gpt-4.1-mini\",\"output\":[]}}\n\n",
        "event: response.output_item.added\n",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_stream\",\"status\":\"in_progress\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"\",\"annotations\":[]}]}}\n\n",
        "event: response.content_part.added\n",
        "data: {\"type\":\"response.content_part.added\",\"item_id\":\"msg_stream\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"\",\"annotations\":[]}}\n\n",
        "event: response.output_text.delta\n",
        "data: {\"type\":\"response.output_text.delta\",\"item_id\":\"msg_stream\",\"output_index\":0,\"content_index\":0,\"delta\":\"hello stream\"}\n\n",
        "event: response.output_text.done\n",
        "data: {\"type\":\"response.output_text.done\",\"item_id\":\"msg_stream\",\"output_index\":0,\"content_index\":0,\"text\":\"hello stream\"}\n\n",
        "event: response.content_part.done\n",
        "data: {\"type\":\"response.content_part.done\",\"item_id\":\"msg_stream\",\"output_index\":0,\"content_index\":0,\"part\":{\"type\":\"output_text\",\"text\":\"hello stream\",\"annotations\":[]}}\n\n",
        "event: response.output_item.done\n",
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"message\",\"id\":\"msg_stream\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello stream\",\"annotations\":[]}]}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_stream\",\"object\":\"response\",\"created_at\":1,\"status\":\"completed\",\"model\":\"gpt-4.1-mini\",\"output\":[{\"type\":\"message\",\"id\":\"msg_stream\",\"status\":\"completed\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"hello stream\",\"annotations\":[]}]}],\"output_text\":\"hello stream\"}}\n\n"
    );
    mount_openai_stream_mock(
        &mock_server,
        json!({
            "model": "gpt-4.1-mini",
            "input": "hello",
            "stream": true
        }),
        upstream_stream,
    )
    .await;

    let app = build_app(
        vec![provider(
            "stub-model",
            true,
            "gpt-4.1-mini",
            vec!["test"],
            0,
            mock_server.uri(),
        )],
        None,
        true,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "gpt-4.1-mini",
            "input": "hello",
            "stream": true
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );

    let events = parse_sse_events(&test::read_body(resp).await);
    assert_eq!(events[0].0, "response.created");
    assert_eq!(events[0].1["response"]["ayatori_client_id"], "stub-model");
    assert_eq!(events[4].0, "response.output_text.delta");
    assert_eq!(events[4].1["delta"], "hello stream");
    let completed = events.last().unwrap();
    assert_eq!(completed.0, "response.completed");
    assert_eq!(completed.1["response"]["output_text"], "hello stream");
    assert_eq!(completed.1["response"]["ayatori_client_id"], "stub-model");
}

#[actix_web::test]
async fn post_responses_passes_instructions_through() {
    let mock_server = MockServer::start().await;
    mount_openai_mock(
        &mock_server,
        json!({
            "model": "gpt-4.1-mini",
            "instructions": "Be concise",
            "input": "hello"
        }),
        message_response("short answer"),
    )
    .await;

    let app = build_app(
        vec![provider(
            "stub-model",
            true,
            "gpt-4.1-mini",
            vec!["test"],
            0,
            mock_server.uri(),
        )],
        None,
        true,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "gpt-4.1-mini",
            "instructions": "Be concise",
            "input": "hello"
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[actix_web::test]
async fn post_responses_supports_function_calling_requests() {
    let mock_server = MockServer::start().await;
    mount_openai_mock(
        &mock_server,
        json!({
            "model": "gpt-4.1-mini",
            "input": "what is the weather?",
            "tools": [{
                "type": "function",
                "name": "lookup_weather",
                "description": "Lookup the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    },
                    "required": ["city"]
                },
                "strict": true
            }],
            "tool_choice": "required"
        }),
        json!({
            "id": "resp_tool_call",
            "object": "response",
            "created_at": 1,
            "status": "completed",
            "model": "provider-model",
            "output": [{
                "type": "function_call",
                "id": "fc_1",
                "call_id": "call_1",
                "name": "lookup_weather",
                "arguments": "{\"city\":\"Tokyo\"}",
                "status": "completed"
            }]
        }),
    )
    .await;

    let app = build_app(
        vec![provider(
            "stub-model",
            true,
            "gpt-4.1-mini",
            vec!["test"],
            0,
            mock_server.uri(),
        )],
        None,
        true,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "gpt-4.1-mini",
            "input": "what is the weather?",
            "tools": [{
                "type": "function",
                "name": "lookup_weather",
                "description": "Lookup the weather",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "city": { "type": "string" }
                    },
                    "required": ["city"]
                },
                "strict": true
            }],
            "tool_choice": "required"
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["output"][0]["type"], "function_call");
    assert_eq!(body["output"][0]["call_id"], "call_1");
    assert_eq!(body["output"][0]["name"], "lookup_weather");
}

#[actix_web::test]
async fn post_responses_accepts_function_call_output_follow_up() {
    let mock_server = MockServer::start().await;
    mount_openai_mock(
        &mock_server,
        json!({
            "model": "gpt-4.1-mini",
            "input": [{
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "{\"temperature_c\":22}"
            }]
        }),
        message_response("It is 22C in Tokyo."),
    )
    .await;

    let app = build_app(
        vec![provider(
            "stub-model",
            true,
            "gpt-4.1-mini",
            vec!["test"],
            0,
            mock_server.uri(),
        )],
        None,
        true,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "gpt-4.1-mini",
            "input": [{
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "{\"temperature_c\":22}"
            }]
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["output_text"], "It is 22C in Tokyo.");
}

#[actix_web::test]
async fn upstream_openai_error_is_forwarded() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_json(json!({
            "model": "gpt-4.1-mini",
            "input": "hello"
        })))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": {
                "message": "provider exploded",
                "type": "api_error",
                "param": null,
                "code": "server_error"
            }
        })))
        .mount(&mock_server)
        .await;

    let app = build_app(
        vec![provider(
            "stub-model",
            true,
            "gpt-4.1-mini",
            vec!["test"],
            0,
            mock_server.uri(),
        )],
        None,
        true,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(minimal_request("gpt-4.1-mini"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["error"]["message"], "provider exploded");
    assert_eq!(body["error"]["code"], "server_error");
    assert_eq!(body["error"]["type"], "api_error");
}

#[actix_web::test]
async fn model_tag_selects_matching_provider() {
    let mock_server = MockServer::start().await;
    mount_openai_mock(
        &mock_server,
        json!({
            "model": "gpt-fast",
            "input": "hello"
        }),
        message_response("from fast"),
    )
    .await;

    let app = build_app(
        vec![
            provider(
                "slow-model",
                false,
                "gpt-slow",
                vec!["slow"],
                1,
                mock_server.uri(),
            ),
            provider(
                "fast-model",
                true,
                "gpt-fast",
                vec!["fast"],
                0,
                mock_server.uri(),
            ),
        ],
        None,
        true,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(minimal_request("tags:fast"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["ayatori_client_id"], "fast-model");
}

#[actix_web::test]
async fn model_id_selects_exact_provider() {
    let mock_server = MockServer::start().await;
    mount_openai_mock(
        &mock_server,
        json!({
            "model": "gpt-exact",
            "input": "hello"
        }),
        message_response("from exact"),
    )
    .await;

    let app = build_app(
        vec![
            provider(
                "default-model",
                true,
                "gpt-default",
                vec!["default"],
                0,
                mock_server.uri(),
            ),
            provider(
                "exact-model",
                false,
                "gpt-exact",
                vec!["exact"],
                1,
                mock_server.uri(),
            ),
        ],
        None,
        true,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(minimal_request("id:exact-model"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["ayatori_client_id"], "exact-model");
}

#[actix_web::test]
async fn missing_bearer_token_returns_401_when_api_key_is_required() {
    let mock_server = MockServer::start().await;
    let app = build_app(
        vec![provider(
            "stub-model",
            true,
            "gpt-4.1-mini",
            vec!["test"],
            0,
            mock_server.uri(),
        )],
        Some("secret"),
        true,
    )
    .await;
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
async fn matching_bearer_token_returns_200_when_api_key_is_required() {
    let mock_server = MockServer::start().await;
    mount_openai_mock(
        &mock_server,
        json!({
            "model": "gpt-4.1-mini",
            "input": "hello"
        }),
        message_response("authorized"),
    )
    .await;

    let app = build_app(
        vec![provider(
            "stub-model",
            true,
            "gpt-4.1-mini",
            vec!["test"],
            0,
            mock_server.uri(),
        )],
        Some("secret"),
        true,
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .insert_header(("Authorization", "Bearer secret"))
        .set_json(minimal_request("gpt-4.1-mini"))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}
