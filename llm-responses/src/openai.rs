use crate::http::{openai_responses_url, send_json, send_request, send_stream};
use crate::types::{CreateResponseRequest, ResponseObject, ResponseStreamEvent};
use crate::{ProviderCapabilities, ResponsesError, ResponsesProvider};
use async_trait::async_trait;
use configuration::{Credential, LlmProvider, LlmProviderType};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use futures::stream::BoxStream;

enum Auth {
    Bearer(String),
    None,
}

pub(crate) struct OpenAiResponsesProvider {
    client: reqwest::Client,
    endpoint: String,
    auth: Auth,
    model: String,
}

impl OpenAiResponsesProvider {
    pub(crate) fn new(provider: LlmProvider, credential: Credential) -> Self {
        let provider_type = provider.provider_type.clone();
        let auth = match (provider_type, credential) {
            (LlmProviderType::OpenAI, Credential::OpenAI { api_key }) => Auth::Bearer(api_key),
            (LlmProviderType::Ollama, Credential::Ollama) => Auth::None,
            (_, credential) => {
                panic!("unexpected credential for OpenAI-compatible provider: {credential:?}")
            }
        };

        Self {
            client: reqwest::Client::new(),
            endpoint: provider.endpoint,
            auth,
            model: provider.model,
        }
    }
}

#[async_trait]
impl ResponsesProvider for OpenAiResponsesProvider {
    async fn create_response(
        &self,
        mut request: CreateResponseRequest,
    ) -> Result<ResponseObject, ResponsesError> {
        request.model = self.model.clone();

        let url = openai_responses_url(&self.endpoint);
        let request_builder = match &self.auth {
            Auth::Bearer(api_key) => self.client.post(url).bearer_auth(api_key),
            Auth::None => self.client.post(url),
        };

        send_json(request_builder, &request).await
    }

    async fn create_response_stream(
        &self,
        mut request: CreateResponseRequest,
    ) -> Result<BoxStream<'static, Result<ResponseStreamEvent, ResponsesError>>, ResponsesError>
    {
        request.model = self.model.clone();
        request.stream = Some(true);

        let url = openai_responses_url(&self.endpoint);
        let request_builder = match &self.auth {
            Auth::Bearer(api_key) => self.client.post(url).bearer_auth(api_key),
            Auth::None => self.client.post(url),
        };

        let response = send_stream(request_builder, &request).await?;
        let stream = response
            .bytes_stream()
            .eventsource()
            .filter_map(|event| async move {
                match event {
                    Ok(event) if event.data == "[DONE]" || event.data.is_empty() => None,
                    Ok(event) => Some(serde_json::from_str(&event.data).map_err(ResponsesError::from)),
                    Err(error) => Some(Err(ResponsesError::Internal(format!(
                        "failed to parse SSE stream: {error}"
                    )))),
                }
            });

        Ok(Box::pin(stream))
    }

    async fn cancel_response(&self, id: &str) -> Result<ResponseObject, ResponsesError> {
        let url = format!("{}/{id}/cancel", openai_responses_url(&self.endpoint));
        let request_builder = match &self.auth {
            Auth::Bearer(api_key) => self.client.post(url).bearer_auth(api_key),
            Auth::None => self.client.post(url),
        };
        send_request(request_builder).await
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            responses_native: true,
            builtin_tools: true,
            reasoning: true,
            image_input: true,
            structured_output: true,
            streaming: true,
            get_response: true,
            cancel_response: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OpenAiResponsesProvider;
    use crate::ResponsesError;
    use crate::types::{CreateResponseRequest, ResponseInput};
    use crate::{ResponsesProvider, types};
    use configuration::{Credential, LlmProvider, LlmProviderType};
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(endpoint: String) -> OpenAiResponsesProvider {
        OpenAiResponsesProvider::new(
            LlmProvider {
                id: "openai".to_string(),
                default: Some(true),
                provider_type: LlmProviderType::OpenAI,
                responses_native: Some(true),
                priority: 0,
                model: "provider-model".to_string(),
                tags: vec![],
                credential_file: "unused".to_string(),
                endpoint,
                capacity: configuration::CapacityLimits {
                    input_tokens: None,
                    requests: None,
                },
            },
            Credential::OpenAI {
                api_key: "secret".to_string(),
            },
        )
    }

    fn request() -> CreateResponseRequest {
        CreateResponseRequest {
            model: "caller-model".to_string(),
            input: ResponseInput::Text("hello".to_string()),
            instructions: None,
            previous_response_id: None,
            store: None,
            background: None,
            stream: None,
            tools: None,
            tool_choice: None,
            temperature: None,
            top_p: None,
            max_output_tokens: None,
            reasoning: None,
            text: None,
            metadata: None,
            user: None,
            parallel_tool_calls: None,
            truncation: None,
        }
    }

    fn upstream_response() -> serde_json::Value {
        json!({
            "id": "resp_123",
            "object": "response",
            "created_at": 123,
            "status": "completed",
            "model": "provider-model",
            "output": [{
                "type": "message",
                "id": "msg_123",
                "status": "completed",
                "role": "assistant",
                "content": [{
                    "type": "output_text",
                    "text": "hello back",
                    "annotations": []
                }]
            }],
            "output_text": "hello back"
        })
    }

    #[tokio::test]
    async fn create_response_passes_through_to_openai() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(header("authorization", "Bearer secret"))
            .and(body_json(json!({
                "model": "provider-model",
                "input": "hello"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(upstream_response()))
            .mount(&server)
            .await;

        let response = provider(server.uri())
            .create_response(request())
            .await
            .unwrap();
        assert_eq!(response.id, "resp_123");
        assert_eq!(response.output_text.as_deref(), Some("hello back"));
        assert!(response.ayatori_client_id.is_empty());
    }

    #[tokio::test]
    async fn create_response_maps_http_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let err = provider(server.uri())
            .create_response(request())
            .await
            .unwrap_err();
        match err {
            ResponsesError::Http { status, body } => {
                assert_eq!(status, 500);
                assert_eq!(body, "boom");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_response_maps_invalid_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("{not-json}")
                    .insert_header("content-type", "application/json"),
            )
            .mount(&server)
            .await;

        let err = provider(server.uri())
            .create_response(request())
            .await
            .unwrap_err();
        assert!(matches!(err, ResponsesError::Serde(_)));
    }

    #[test]
    fn response_types_are_reachable() {
        let _: Option<types::ResponseObject> = None;
    }
}
