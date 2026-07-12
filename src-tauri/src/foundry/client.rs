use anyhow::{Context, Result};
use log::{info, warn};
use reqwest::Client;
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Serialize)]
struct AgentReference<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    reference_type: &'static str,
}

#[derive(Debug, Serialize)]
struct ResponsesRequest<'a> {
    model: &'a str,
    input: &'a str,
    agent_reference: AgentReference<'a>,
    stream: bool,
}

#[derive(Debug)]
pub struct FoundryClient {
    http: Client,
}

impl FoundryClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .expect("Failed to build Foundry HTTP client");

        Self { http }
    }

    pub async fn process(
        &self,
        project_endpoint: &str,
        model: &str,
        agent_name: &str,
        input: &str,
        bearer_token: Option<&str>,
        on_chunk: Option<&(dyn Fn(&str) + Send + Sync)>,
        cancel_token: Option<&CancellationToken>,
    ) -> Result<String> {
        if project_endpoint.trim().is_empty() {
            anyhow::bail!(
                "Microsoft Foundry project endpoint is not configured. Open Settings and add the project endpoint."
            );
        }

        let endpoint = responses_endpoint(project_endpoint);
        let request = ResponsesRequest {
            model,
            input,
            agent_reference: AgentReference {
                name: agent_name,
                reference_type: "agent_reference",
            },
            stream: false,
        };

        info!(
            "Calling Microsoft Foundry Agent (agent={}, model={}, text_len={})",
            agent_name,
            model,
            input.len()
        );

        let call = async {
            let mut http_request = self.http.post(&endpoint).json(&request);
            if let Some(token) = bearer_token.filter(|token| !token.is_empty()) {
                http_request = http_request.bearer_auth(token);
            }

            let response = http_request
                .send()
                .await
                .context("Failed to connect to Microsoft Foundry Agent")?;

            let status = response.status();
            let body = response
                .text()
                .await
                .context("Failed to read Microsoft Foundry Agent response")?;

            if !status.is_success() {
                anyhow::bail!(
                    "Microsoft Foundry Agent returned HTTP {}: {}",
                    status.as_u16(),
                    body.trim()
                );
            }

            parse_response_text(&body)
        };

        let result = if let Some(token) = cancel_token {
            tokio::select! {
                biased;
                _ = token.cancelled() => anyhow::bail!("Request cancelled"),
                result = call => result?,
            }
        } else {
            call.await?
        };

        if let Some(callback) = on_chunk {
            callback(&result);
        }

        Ok(result)
    }
}

impl Default for FoundryClient {
    fn default() -> Self {
        Self::new()
    }
}

fn responses_endpoint(project_endpoint: &str) -> String {
    let endpoint = project_endpoint.trim().trim_end_matches('/');
    if endpoint.ends_with("/openai/v1/responses") {
        endpoint.to_string()
    } else if endpoint.ends_with("/openai/v1") {
        format!("{endpoint}/responses")
    } else {
        format!("{endpoint}/openai/v1/responses")
    }
}

fn parse_response_text(body: &str) -> Result<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Microsoft Foundry Agent returned an empty response");
    }

    let value: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(error) => {
            warn!(
                "Foundry response was not JSON ({}); treating it as plain text",
                error
            );
            return Ok(trimmed.to_string());
        }
    };

    extract_response_text(&value)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow::anyhow!("Microsoft Foundry Agent response did not contain output text")
        })
}

fn extract_response_text(value: &Value) -> Option<&str> {
    for key in ["output_text", "result", "response"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return Some(text);
        }
    }

    if let Some(text) = value
        .get("output")
        .and_then(Value::as_array)
        .and_then(|output| output.iter().find_map(extract_output_item_text))
    {
        return Some(text);
    }

    value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(extract_content_text)
}

fn extract_output_item_text(item: &Value) -> Option<&str> {
    item.get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.iter().find_map(extract_content_text))
        .or_else(|| item.get("text").and_then(Value::as_str))
}

fn extract_content_text(content: &Value) -> Option<&str> {
    if let Some(text) = content.as_str() {
        return Some(text);
    }

    content.get("text").and_then(Value::as_str).or_else(|| {
        content.get("content").and_then(|nested| {
            nested.as_str().or_else(|| {
                nested
                    .as_array()
                    .and_then(|items| items.iter().find_map(extract_content_text))
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_response_text, responses_endpoint};

    #[test]
    fn builds_responses_endpoint_from_project_endpoint() {
        assert_eq!(
            responses_endpoint("https://example.test/api/projects/demo"),
            "https://example.test/api/projects/demo/openai/v1/responses"
        );
        assert_eq!(
            responses_endpoint("https://example.test/api/projects/demo/openai/v1/"),
            "https://example.test/api/projects/demo/openai/v1/responses"
        );
    }

    #[test]
    fn parses_responses_api_output() {
        let body = r#"{
            "output": [{
                "type": "message",
                "content": [{"type": "output_text", "text": "rewritten text"}]
            }]
        }"#;

        assert_eq!(parse_response_text(body).unwrap(), "rewritten text");
    }

    #[test]
    fn parses_simple_result_contract() {
        assert_eq!(
            parse_response_text(r#"{"result":"translated text"}"#).unwrap(),
            "translated text"
        );
    }

    #[test]
    fn parses_chat_completion_compatibility_response() {
        let body = r#"{"choices":[{"message":{"content":"compatible text"}}]}"#;

        assert_eq!(parse_response_text(body).unwrap(), "compatible text");
    }

    #[test]
    fn accepts_plain_text_facade_response() {
        assert_eq!(
            parse_response_text("plain response").unwrap(),
            "plain response"
        );
    }

    #[test]
    fn rejects_json_without_output() {
        assert!(parse_response_text(r#"{"id":"response-1"}"#).is_err());
    }
}
