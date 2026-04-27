use crate::http::{azure_responses_url, send_json, send_stream};
use crate::types::{CreateResponseRequest, ResponseObject, ResponseStreamEvent};
use crate::{ProviderCapabilities, ResponsesError, ResponsesProvider};
use async_trait::async_trait;
use configuration::{Credential, LlmProvider};
use eventsource_stream::Eventsource;
use futures::StreamExt;
use futures::stream::BoxStream;

const DEFAULT_API_VERSION: &str = "2025-04-01-preview";

pub(crate) struct AzureOpenAiResponsesProvider {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    api_version: String,
    deployment: String,
}

impl AzureOpenAiResponsesProvider {
    pub(crate) fn new(provider: LlmProvider, credential: Credential) -> Self {
        let (api_key, deployment, api_version) = match credential {
            Credential::Azure {
                api_key,
                deployment,
                api_version,
            } => (
                api_key,
                deployment,
                api_version.unwrap_or_else(|| DEFAULT_API_VERSION.to_string()),
            ),
            credential => panic!("unexpected credential for Azure provider: {credential:?}"),
        };

        let deployment = if deployment.is_empty() {
            provider.model
        } else {
            deployment
        };

        Self {
            client: reqwest::Client::new(),
            endpoint: provider.endpoint,
            api_key,
            api_version,
            deployment,
        }
    }
}

#[async_trait]
impl ResponsesProvider for AzureOpenAiResponsesProvider {
    async fn create_response(
        &self,
        mut request: CreateResponseRequest,
    ) -> Result<ResponseObject, ResponsesError> {
        request.model = self.deployment.clone();

        let url = azure_responses_url(&self.endpoint, &self.api_version);
        let request_builder = self.client.post(url).header("api-key", &self.api_key);
        send_json(request_builder, &request).await
    }

    async fn create_response_stream(
        &self,
        mut request: CreateResponseRequest,
    ) -> Result<BoxStream<'static, Result<ResponseStreamEvent, ResponsesError>>, ResponsesError>
    {
        request.model = self.deployment.clone();
        request.stream = Some(true);

        let url = azure_responses_url(&self.endpoint, &self.api_version);
        let request_builder = self.client.post(url).header("api-key", &self.api_key);
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
    use super::AzureOpenAiResponsesProvider;
    use crate::ResponsesError;
    use crate::ResponsesProvider;
    use crate::types::{CreateResponseRequest, ResponseInput};
    use configuration::{CapacityLimits, Credential, LlmProvider, LlmProviderType};
    use serde_json::json;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(endpoint: String) -> AzureOpenAiResponsesProvider {
        AzureOpenAiResponsesProvider::new(
            LlmProvider {
                id: "azure".to_string(),
                default: Some(true),
                provider_type: LlmProviderType::Azure,
                responses_native: Some(true),
                priority: 0,
                model: "selector-model".to_string(),
                tags: vec![],
                credential_file: "unused".to_string(),
                endpoint,
                capacity: CapacityLimits {
                    input_tokens: None,
                    requests: None,
                },
            },
            Credential::Azure {
                api_key: "azure-secret".to_string(),
                deployment: "deployment-model".to_string(),
                api_version: Some("2025-04-01-preview".to_string()),
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

    #[tokio::test]
    async fn create_response_uses_azure_url_and_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/responses"))
            .and(query_param("api-version", "2025-04-01-preview"))
            .and(header("api-key", "azure-secret"))
            .and(body_json(json!({
                "model": "deployment-model",
                "input": "hello"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "resp_azure",
                "object": "response",
                "created_at": 456,
                "status": "completed",
                "model": "deployment-model",
                "output": []
            })))
            .mount(&server)
            .await;

        let response = provider(format!("{}/openai/v1", server.uri()))
            .create_response(request())
            .await
            .unwrap();
        assert_eq!(response.id, "resp_azure");
        assert_eq!(response.model, "deployment-model");
    }

    #[tokio::test]
    async fn create_response_maps_authentication_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/responses"))
            .respond_with(ResponseTemplate::new(401).set_body_string("nope"))
            .mount(&server)
            .await;

        let err = provider(server.uri())
            .create_response(request())
            .await
            .unwrap_err();
        assert!(matches!(err, ResponsesError::Authentication));
    }
}
