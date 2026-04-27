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
use std::time::Duration;
use token_measure::{ByteLengthTokenMeasure, TokenMeasure};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static NEXT_ID: AtomicU64 = AtomicU64::new(20_000);

fn unique_path(name: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ayatori-{name}-{id}.toml"))
}

fn write_credential_file(provider_ids: &[&str]) -> String {
    let path = unique_path("credential-state");
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

fn build_configuration(endpoint: String) -> Configuration {
    let credential_file = write_credential_file(&["stub-model"]);
    Configuration {
        providers: vec![LlmProvider {
            id: "stub-model".to_string(),
            default: Some(true),
            provider_type: LlmProviderType::OpenAI,
            responses_native: Some(true),
            priority: 0,
            model: "gpt-4.1-mini".to_string(),
            tags: vec![],
            credential_file,
            endpoint,
            capacity: CapacityLimits {
                input_tokens: None,
                requests: None,
            },
        }],
    }
}

async fn build_app(
    endpoint: String,
) -> impl actix_web::dev::Service<
    actix_http::Request,
    Response = actix_web::dev::ServiceResponse,
    Error = actix_web::Error,
> {
    let configuration = build_configuration(endpoint);
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
        "model": "provider-model",
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
async fn previous_response_id_rebuilds_input_and_omits_upstream_id() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_json(json!({
            "model": "gpt-4.1-mini",
            "input": "hello"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(message_response("resp_1", "one")))
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(header("authorization", "Bearer test-key"))
        .and(body_json(json!({
            "model": "gpt-4.1-mini",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": "hello"
                },
                {
                    "type": "message",
                    "role": "user",
                    "content": "follow up"
                }
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(message_response("resp_2", "two")))
        .mount(&mock_server)
        .await;

    let app = build_app(mock_server.uri()).await;

    let first = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "gpt-4.1-mini",
            "input": "hello"
        }))
        .to_request();
    let first_body: Value = test::read_body_json(test::call_service(&app, first).await).await;

    let second = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "gpt-4.1-mini",
            "input": "follow up",
            "previous_response_id": first_body["id"]
        }))
        .to_request();
    let resp = test::call_service(&app, second).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["previous_response_id"], first_body["id"]);
    assert_eq!(body["output_text"], "two");
}

#[actix_web::test]
async fn store_false_skips_response_persistence() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(message_response("resp_3", "ephemeral")),
        )
        .mount(&mock_server)
        .await;

    let app = build_app(mock_server.uri()).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "gpt-4.1-mini",
            "input": "hello",
            "store": false
        }))
        .to_request();
    let body: Value = test::read_body_json(test::call_service(&app, req).await).await;

    let get_req = test::TestRequest::get()
        .uri(&format!("/v1/responses/{}", body["id"].as_str().unwrap()))
        .to_request();
    let get_resp = test::call_service(&app, get_req).await;
    assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);
}

#[actix_web::test]
async fn management_endpoints_expose_stored_response_and_input_chain() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(message_response("resp_4", "stored")),
        )
        .mount(&mock_server)
        .await;

    let app = build_app(mock_server.uri()).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "gpt-4.1-mini",
            "input": "hello"
        }))
        .to_request();
    let body: Value = test::read_body_json(test::call_service(&app, req).await).await;
    let response_id = body["id"].as_str().unwrap();

    let get_req = test::TestRequest::get()
        .uri(&format!("/v1/responses/{response_id}"))
        .to_request();
    let get_body: Value = test::read_body_json(test::call_service(&app, get_req).await).await;
    assert_eq!(get_body["output_text"], "stored");

    let items_req = test::TestRequest::get()
        .uri(&format!("/v1/responses/{response_id}/input_items"))
        .to_request();
    let items_body: Value = test::read_body_json(test::call_service(&app, items_req).await).await;
    assert_eq!(items_body["object"], "list");
    assert_eq!(items_body["data"][0]["content"], "hello");

    let delete_req = test::TestRequest::delete()
        .uri(&format!("/v1/responses/{response_id}"))
        .to_request();
    let delete_body: Value = test::read_body_json(test::call_service(&app, delete_req).await).await;
    assert_eq!(delete_body["object"], "response.deleted");
    assert_eq!(delete_body["deleted"], true);
}

#[actix_web::test]
async fn background_requests_can_be_polled_and_cancelled() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(50))
                .set_body_json(message_response("resp_upstream", "done")),
        )
        .mount(&mock_server)
        .await;

    let app = build_app(mock_server.uri()).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "gpt-4.1-mini",
            "input": "hello",
            "background": true
        }))
        .to_request();
    let queued_resp = test::call_service(&app, req).await;
    assert_eq!(queued_resp.status(), StatusCode::ACCEPTED);
    let queued_body: Value = test::read_body_json(queued_resp).await;
    assert_eq!(queued_body["status"], "queued");
    let response_id = queued_body["id"].as_str().unwrap().to_string();

    let cancel_req = test::TestRequest::post()
        .uri(&format!("/v1/responses/{response_id}/cancel"))
        .to_request();
    let cancel_body: Value = test::read_body_json(test::call_service(&app, cancel_req).await).await;
    assert_eq!(cancel_body["status"], "cancelled");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let get_req = test::TestRequest::get()
        .uri(&format!("/v1/responses/{response_id}"))
        .to_request();
    let get_body: Value = test::read_body_json(test::call_service(&app, get_req).await).await;
    assert_eq!(get_body["status"], "cancelled");
}

#[actix_web::test]
async fn background_requests_eventually_complete() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(message_response("resp_upstream", "done")),
        )
        .mount(&mock_server)
        .await;

    let app = build_app(mock_server.uri()).await;
    let req = test::TestRequest::post()
        .uri("/v1/responses")
        .set_json(json!({
            "model": "gpt-4.1-mini",
            "input": "hello",
            "background": true
        }))
        .to_request();
    let queued_body: Value = test::read_body_json(test::call_service(&app, req).await).await;
    let response_id = queued_body["id"].as_str().unwrap().to_string();

    tokio::time::sleep(Duration::from_millis(30)).await;

    let get_req = test::TestRequest::get()
        .uri(&format!("/v1/responses/{response_id}"))
        .to_request();
    let get_body: Value = test::read_body_json(test::call_service(&app, get_req).await).await;
    assert_eq!(get_body["status"], "completed");
    assert_eq!(get_body["output_text"], "done");
}
