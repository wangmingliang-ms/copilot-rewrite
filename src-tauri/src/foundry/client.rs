use anyhow::{Context, Result};
use futures_util::StreamExt;
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
            input,
            agent_reference: AgentReference {
                name: agent_name,
                reference_type: "agent_reference",
            },
            stream: true,
        };

        info!(
            "Calling latest Microsoft Foundry Agent version (agent={}, text_len={})",
            agent_name,
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
            if !status.is_success() {
                let body = response
                    .text()
                    .await
                    .context("Failed to read Microsoft Foundry Agent error response")?;
                anyhow::bail!(
                    "Microsoft Foundry Agent returned HTTP {}: {}",
                    status.as_u16(),
                    body.trim()
                );
            }

            let is_event_stream = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/event-stream"));

            if is_event_stream {
                consume_event_stream(response, on_chunk).await
            } else {
                let body = response
                    .text()
                    .await
                    .context("Failed to read Microsoft Foundry Agent response")?;
                let result = parse_response_text(&body)?;
                if let Some(callback) = on_chunk {
                    callback(&result);
                }
                Ok(result)
            }
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

async fn consume_event_stream(
    response: reqwest::Response,
    on_chunk: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> Result<String> {
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut accumulated = StreamingResponse::default();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Failed while reading Microsoft Foundry Agent stream")?;
        buffer.extend_from_slice(&chunk);

        while let Some((event_end, delimiter_len)) = find_sse_event_end(&buffer) {
            let remainder = buffer.split_off(event_end + delimiter_len);
            buffer.truncate(event_end);
            apply_stream_event(&buffer, &mut accumulated, on_chunk)?;
            buffer = remainder;
        }
    }

    if buffer.iter().any(|byte| !byte.is_ascii_whitespace()) {
        apply_stream_event(&buffer, &mut accumulated, on_chunk)?;
    }

    if !accumulated.completed {
        anyhow::bail!("Microsoft Foundry Agent stream ended before completion");
    }

    let result = accumulated.output.trim();
    if result.is_empty() {
        anyhow::bail!("Microsoft Foundry Agent returned an empty streamed response");
    }

    Ok(result.to_string())
}

fn find_sse_event_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|window| window == b"\n\n");
    let crlf = buffer.windows(4).position(|window| window == b"\r\n\r\n");

    match (lf, crlf) {
        (Some(lf), Some(crlf)) if lf <= crlf => Some((lf, 2)),
        (Some(_), Some(crlf)) => Some((crlf, 4)),
        (Some(lf), None) => Some((lf, 2)),
        (None, Some(crlf)) => Some((crlf, 4)),
        (None, None) => None,
    }
}

#[derive(Debug, Default)]
struct StreamingResponse {
    output: String,
    completed: bool,
}

fn apply_stream_event(
    event_bytes: &[u8],
    accumulated: &mut StreamingResponse,
    on_chunk: Option<&(dyn Fn(&str) + Send + Sync)>,
) -> Result<()> {
    let event = std::str::from_utf8(event_bytes)
        .context("Microsoft Foundry Agent stream contained invalid UTF-8")?;
    let mut event_name = None;
    let mut data_lines = Vec::new();

    for line in event.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start());
        }
    }

    if data_lines.is_empty() {
        return Ok(());
    }

    let data = data_lines.join("\n");
    if data.trim() == "[DONE]" {
        accumulated.completed = true;
        return Ok(());
    }

    let value: Value = serde_json::from_str(&data)
        .with_context(|| format!("Failed to parse Microsoft Foundry stream event: {data}"))?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .or(event_name)
        .unwrap_or_default();

    match event_type {
        "response.output_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                accumulated.output.push_str(delta);
                if let Some(callback) = on_chunk {
                    callback(&accumulated.output);
                }
            }
        }
        "response.completed" => {
            accumulated.completed = true;
            if accumulated.output.is_empty() {
                accumulated.output = value
                    .get("response")
                    .and_then(extract_response_text)
                    .unwrap_or_default()
                    .to_string();
                if !accumulated.output.is_empty() {
                    if let Some(callback) = on_chunk {
                        callback(&accumulated.output);
                    }
                }
            }
        }
        "response.failed" | "error" => {
            let message = value
                .pointer("/response/error/message")
                .or_else(|| value.pointer("/error/message"))
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Unknown streaming error");
            anyhow::bail!("Microsoft Foundry Agent stream failed: {message}");
        }
        _ => {}
    }

    Ok(())
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
    use super::{
        apply_stream_event, find_sse_event_end, parse_response_text, responses_endpoint,
        AgentReference, ResponsesRequest, StreamingResponse,
    };

    #[test]
    fn request_delegates_version_and_model_selection_to_agent() {
        let request = ResponsesRequest {
            input: "hello",
            agent_reference: AgentReference {
                name: "demo-agent",
                reference_type: "agent_reference",
            },
            stream: true,
        };
        let value = serde_json::to_value(request).unwrap();

        assert!(value.get("model").is_none());
        assert!(value["agent_reference"].get("version").is_none());
        assert_eq!(value["agent_reference"]["name"], "demo-agent");
        assert_eq!(value["stream"], true);
    }

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

    #[test]
    fn finds_lf_and_crlf_sse_event_boundaries() {
        assert_eq!(find_sse_event_end(b"data: one\n\ndata: two"), Some((9, 2)));
        assert_eq!(
            find_sse_event_end(b"data: one\r\n\r\ndata: two"),
            Some((9, 4))
        );
    }

    #[test]
    fn accumulates_streaming_text_and_completion() {
        let mut response = StreamingResponse::default();
        apply_stream_event(
            br#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":"Hello"}"#,
            &mut response,
            None,
        )
        .unwrap();
        apply_stream_event(
            br#"event: response.output_text.delta
data: {"type":"response.output_text.delta","delta":" world"}"#,
            &mut response,
            None,
        )
        .unwrap();
        apply_stream_event(b"data: [DONE]", &mut response, None).unwrap();

        assert_eq!(response.output, "Hello world");
        assert!(response.completed);
    }

    #[test]
    fn uses_completed_response_when_no_delta_events_arrive() {
        let mut response = StreamingResponse::default();
        apply_stream_event(
            br#"event: response.completed
data: {"type":"response.completed","response":{"output":[{"type":"message","content":[{"type":"output_text","text":"complete text"}]}]}}"#,
            &mut response,
            None,
        )
        .unwrap();

        assert_eq!(response.output, "complete text");
        assert!(response.completed);
    }

    #[test]
    fn surfaces_streaming_failures() {
        let mut response = StreamingResponse::default();
        let error = apply_stream_event(
            br#"event: response.failed
data: {"type":"response.failed","response":{"error":{"message":"agent failed"}}}"#,
            &mut response,
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("agent failed"));
    }
}
