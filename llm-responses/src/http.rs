use crate::ResponsesError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub(crate) async fn send_json<T, R>(
    request_builder: reqwest::RequestBuilder,
    payload: &T,
) -> Result<R, ResponsesError>
where
    T: Serialize + ?Sized,
    R: DeserializeOwned,
{
    let response = request_builder.json(payload).send().await?;
    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => ResponsesError::Authentication,
            code => ResponsesError::Http { status: code, body },
        });
    }

    serde_json::from_str(&body).map_err(ResponsesError::from)
}

pub(crate) async fn send_value<T>(
    request_builder: reqwest::RequestBuilder,
    payload: &T,
) -> Result<Value, ResponsesError>
where
    T: Serialize + ?Sized,
{
    let response = request_builder.json(payload).send().await?;
    let status = response.status();
    let body = response.text().await?;

    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => ResponsesError::Authentication,
            code => ResponsesError::Http { status: code, body },
        });
    }

    serde_json::from_str(&body).map_err(ResponsesError::from)
}

pub(crate) fn openai_responses_url(endpoint: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    if endpoint.ends_with("/responses") {
        endpoint.to_string()
    } else if endpoint.ends_with("/v1") {
        format!("{endpoint}/responses")
    } else {
        format!("{endpoint}/v1/responses")
    }
}

pub(crate) fn azure_responses_url(endpoint: &str, api_version: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    let endpoint = endpoint
        .strip_suffix("/openai/v1")
        .or_else(|| endpoint.strip_suffix("/openai"))
        .unwrap_or(endpoint);
    format!("{endpoint}/openai/responses?api-version={api_version}")
}
