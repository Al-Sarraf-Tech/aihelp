use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::sleep;

#[derive(Debug, Clone)]
pub struct OpenAiClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    retry_attempts: usize,
    retry_backoff_ms: u64,
}

impl OpenAiClient {
    pub fn new(
        base_url: String,
        api_key: String,
        timeout_secs: u64,
        retry_attempts: usize,
        retry_backoff_ms: u64,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .context("failed to build HTTP client")?;

        Ok(Self {
            http,
            base_url,
            api_key,
            retry_attempts,
            retry_backoff_ms,
        })
    }

    fn max_attempts(&self) -> usize {
        self.retry_attempts.saturating_add(1)
    }

    fn should_retry_status(status: StatusCode) -> bool {
        status == StatusCode::REQUEST_TIMEOUT
            || status == StatusCode::TOO_MANY_REQUESTS
            || status.is_server_error()
    }

    fn should_retry_reqwest_error(err: &reqwest::Error) -> bool {
        err.is_timeout() || err.is_connect() || err.is_request() || err.is_body()
    }

    fn is_timeout_like_error(err: &reqwest::Error) -> bool {
        err.is_timeout() || err.to_string().to_ascii_lowercase().contains("timed out")
    }

    fn retry_hint() -> &'static str {
        "Try increasing --timeout-secs or --retries."
    }

    fn retry_delay(&self, attempt: usize) -> Duration {
        let exp = 1u64 << attempt.min(6);
        let base = self.retry_backoff_ms.max(1);
        let millis = base.saturating_mul(exp).min(10_000);
        Duration::from_millis(millis)
    }

    async fn sleep_before_retry(&self, attempt: usize) {
        sleep(self.retry_delay(attempt)).await;
    }

    pub fn endpoint_url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/'),
        )
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = self.endpoint_url("/v1/models");
        let max_attempts = self.max_attempts();

        for attempt in 0..max_attempts {
            let mut req = self.http.get(&url);
            if !self.api_key.is_empty() {
                req = req.bearer_auth(&self.api_key);
            }

            let resp = match req.send().await {
                Ok(resp) => resp,
                Err(err) => {
                    let retryable = Self::should_retry_reqwest_error(&err);
                    if retryable && attempt + 1 < max_attempts {
                        self.sleep_before_retry(attempt).await;
                        continue;
                    }

                    if retryable {
                        return Err(err).context(format!(
                            "request to /v1/models failed after {max_attempts} attempts. {}",
                            Self::retry_hint()
                        ));
                    }
                    return Err(err).context("request to /v1/models failed");
                }
            };

            if resp.status() != StatusCode::OK {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();

                if Self::should_retry_status(status) && attempt + 1 < max_attempts {
                    self.sleep_before_retry(attempt).await;
                    continue;
                }

                if Self::should_retry_status(status) {
                    bail!(
                        "/v1/models returned {status} after {max_attempts} attempts: {body}. {}",
                        Self::retry_hint()
                    );
                }

                bail!("/v1/models returned {status}: {body}");
            }

            let payload: ModelsResponse = resp
                .json()
                .await
                .context("failed to parse /v1/models response")?;

            return Ok(payload.data.into_iter().map(|m| m.id).collect());
        }

        bail!("request to /v1/models failed after {max_attempts} attempts")
    }

    pub async fn verify_model_presence(&self, model: &str) -> Result<()> {
        let mut models = self.list_models().await?;
        models.sort();

        if models.iter().any(|id| id == model) {
            return Ok(());
        }

        let joined = if models.is_empty() {
            "<none>".to_string()
        } else {
            models.join(", ")
        };

        bail!(
            "default model '{model}' not found in /v1/models. Available model IDs: {joined}. Load '{model}' in LM Studio or override with --model <ID>."
        );
    }

    pub fn dry_run_payload(&self, request: &ChatCompletionRequest) -> Value {
        let auth = if self.api_key.is_empty() {
            Value::Null
        } else {
            Value::String("Bearer ***REDACTED***".to_string())
        };

        json!({
            "method": "POST",
            "url": self.endpoint_url("/v1/chat/completions"),
            "headers": {
                "Authorization": auth,
                "Content-Type": "application/json"
            },
            "body": request,
        })
    }

    pub async fn chat_completion(
        &self,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionEnvelope> {
        let url = self.endpoint_url("/v1/chat/completions");
        let max_attempts = self.max_attempts();

        for attempt in 0..max_attempts {
            let mut req = self.http.post(&url).json(request);
            if !self.api_key.is_empty() {
                req = req.bearer_auth(&self.api_key);
            }

            let resp = match req.send().await {
                Ok(resp) => resp,
                Err(err) => {
                    let retryable = Self::should_retry_reqwest_error(&err);
                    if retryable && attempt + 1 < max_attempts {
                        self.sleep_before_retry(attempt).await;
                        continue;
                    }

                    if retryable {
                        return Err(err).context(format!(
                            "request to /v1/chat/completions failed after {max_attempts} attempts. {}",
                            Self::retry_hint()
                        ));
                    }

                    return Err(err).context("request to /v1/chat/completions failed");
                }
            };

            let status = resp.status();
            let raw_text = match resp.text().await {
                Ok(text) => text,
                Err(err) => {
                    let retryable = Self::should_retry_reqwest_error(&err);
                    if retryable && attempt + 1 < max_attempts {
                        self.sleep_before_retry(attempt).await;
                        continue;
                    }

                    if retryable {
                        return Err(err).context(format!(
                            "failed reading /v1/chat/completions response body after {max_attempts} attempts. {}",
                            Self::retry_hint()
                        ));
                    }

                    return Err(err).context("failed reading /v1/chat/completions response body");
                }
            };

            if !status.is_success() {
                if Self::should_retry_status(status) && attempt + 1 < max_attempts {
                    self.sleep_before_retry(attempt).await;
                    continue;
                }

                if Self::should_retry_status(status) {
                    bail!(
                        "/v1/chat/completions returned {status} after {max_attempts} attempts: {raw_text}. {}",
                        Self::retry_hint()
                    );
                }

                bail!("/v1/chat/completions returned {status}: {raw_text}");
            }

            let raw_json: Value = serde_json::from_str(&raw_text)
                .with_context(|| format!("failed parsing chat completion JSON: {raw_text}"))?;

            let parsed: ChatCompletionResponse = serde_json::from_value(raw_json.clone())
                .context("failed to decode chat completion payload")?;

            return Ok(ChatCompletionEnvelope {
                response: parsed,
                raw_json,
            });
        }

        bail!(
            "request to /v1/chat/completions failed after {max_attempts} attempts. {}",
            Self::retry_hint()
        )
    }

    pub async fn chat_completion_stream<FT, FJ>(
        &self,
        request: &ChatCompletionRequest,
        mut on_text_delta: FT,
        mut on_chunk_json: FJ,
    ) -> Result<ChatCompletionEnvelope>
    where
        FT: FnMut(&str) -> Result<()>,
        FJ: FnMut(&Value) -> Result<()>,
    {
        let mut stream_request = request.clone();
        stream_request.stream = true;

        let url = self.endpoint_url("/v1/chat/completions");
        let max_attempts = self.max_attempts();

        'attempt_loop: for attempt in 0..max_attempts {
            let mut req = self.http.post(&url).json(&stream_request);
            if !self.api_key.is_empty() {
                req = req.bearer_auth(&self.api_key);
            }

            let resp = match req.send().await {
                Ok(resp) => resp,
                Err(err) => {
                    let retryable = Self::should_retry_reqwest_error(&err);
                    if retryable && attempt + 1 < max_attempts {
                        self.sleep_before_retry(attempt).await;
                        continue;
                    }

                    if retryable {
                        return Err(err).context(format!(
                            "stream request to /v1/chat/completions failed after {max_attempts} attempts. {}",
                            Self::retry_hint()
                        ));
                    }

                    return Err(err).context("stream request to /v1/chat/completions failed");
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();

                if Self::should_retry_status(status) && attempt + 1 < max_attempts {
                    self.sleep_before_retry(attempt).await;
                    continue;
                }

                if Self::should_retry_status(status) {
                    bail!(
                        "stream /v1/chat/completions returned {status} after {max_attempts} attempts: {body}. {}",
                        Self::retry_hint()
                    );
                }

                bail!("stream /v1/chat/completions returned {status}: {body}");
            }

            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_ascii_lowercase();

            // Some OpenAI-compatible servers ignore stream=true and return regular JSON.
            // Fall back to a non-stream parse so default-stream UX remains resilient.
            if !content_type.contains("text/event-stream") {
                let raw_text = match resp.text().await {
                    Ok(text) => text,
                    Err(err) => {
                        let retryable = Self::should_retry_reqwest_error(&err);
                        if retryable && attempt + 1 < max_attempts {
                            self.sleep_before_retry(attempt).await;
                            continue;
                        }

                        if retryable {
                            return Err(err).context(format!(
                                "failed reading non-SSE fallback response body after {max_attempts} attempts. {}",
                                Self::retry_hint()
                            ));
                        }

                        return Err(err).context("failed reading non-SSE fallback response body");
                    }
                };

                let raw_json: Value = serde_json::from_str(&raw_text)
                    .with_context(|| format!("failed parsing non-SSE fallback JSON: {raw_text}"))?;
                let parsed: ChatCompletionResponse = serde_json::from_value(raw_json.clone())
                    .context("failed decoding non-SSE fallback payload")?;

                if let Some(text) = parsed
                    .choices
                    .first()
                    .and_then(|c| c.message.content.as_deref())
                {
                    on_text_delta(text)?;
                }
                on_chunk_json(&raw_json)?;

                return Ok(ChatCompletionEnvelope {
                    response: parsed,
                    raw_json,
                });
            }

            let mut bytes_stream = resp.bytes_stream();
            let mut buf = String::new();
            let mut done = false;
            let mut emitted_data = false;

            let mut text_acc = String::new();
            let mut finish_reason: Option<String> = None;
            let mut tool_calls_acc: BTreeMap<usize, ToolCall> = BTreeMap::new();
            let mut raw_chunks = Vec::<Value>::new();

            while let Some(next) = bytes_stream.next().await {
                let chunk = match next {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        if !emitted_data
                            && Self::should_retry_reqwest_error(&err)
                            && attempt + 1 < max_attempts
                        {
                            self.sleep_before_retry(attempt).await;
                            continue 'attempt_loop;
                        }

                        if Self::is_timeout_like_error(&err) {
                            return Err(err).context(format!(
                                "failed to read SSE chunk. Stream timed out{} {}",
                                if attempt + 1 >= max_attempts {
                                    format!(" after {max_attempts} attempts.")
                                } else {
                                    String::new()
                                },
                                Self::retry_hint()
                            ));
                        }

                        return Err(err).context("failed to read SSE chunk");
                    }
                };

                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(idx) = buf.find("\n\n") {
                    let event_block = buf[..idx].to_string();
                    buf = buf[idx + 2..].to_string();

                    if event_block.trim().is_empty() {
                        continue;
                    }

                    let data = extract_sse_data(&event_block);
                    if data.is_empty() {
                        continue;
                    }

                    emitted_data = true;

                    if data == "[DONE]" {
                        done = true;
                        break;
                    }

                    let chunk_json: Value = serde_json::from_str(&data)
                        .with_context(|| format!("failed to parse SSE data JSON: {data}"))?;
                    on_chunk_json(&chunk_json)?;
                    raw_chunks.push(chunk_json.clone());

                    let parsed_chunk: ChatCompletionChunk = serde_json::from_value(chunk_json)
                        .context("failed to decode streamed chunk")?;

                    for choice in parsed_chunk.choices {
                        if let Some(reason) = choice.finish_reason {
                            finish_reason = Some(reason);
                        }

                        if let Some(content) = choice.delta.content {
                            text_acc.push_str(&content);
                            on_text_delta(&content)?;
                        }

                        if let Some(tool_calls) = choice.delta.tool_calls {
                            for tc in tool_calls {
                                let entry =
                                    tool_calls_acc.entry(tc.index).or_insert_with(|| ToolCall {
                                        id: format!("tool_call_{}", tc.index),
                                        kind: "function".to_string(),
                                        function: ToolFunction {
                                            name: String::new(),
                                            arguments: String::new(),
                                        },
                                    });

                                if let Some(id) = tc.id {
                                    entry.id = id;
                                }

                                if let Some(kind) = tc.kind {
                                    entry.kind = kind;
                                }

                                if let Some(func) = tc.function {
                                    if let Some(name) = func.name {
                                        entry.function.name.push_str(&name);
                                    }
                                    if let Some(arguments) = func.arguments {
                                        entry.function.arguments.push_str(&arguments);
                                    }
                                }
                            }
                        }
                    }
                }

                if done {
                    break;
                }
            }

            let tool_calls: Vec<ToolCall> = tool_calls_acc.values().cloned().collect();
            let message = ChatMessage {
                role: "assistant".to_string(),
                content: if text_acc.is_empty() {
                    None
                } else {
                    Some(text_acc.clone())
                },
                name: None,
                tool_call_id: None,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
            };

            let response = ChatCompletionResponse {
                id: None,
                object: None,
                created: None,
                model: Some(request.model.clone()),
                choices: vec![ChatChoice {
                    index: 0,
                    message,
                    finish_reason,
                }],
            };

            return Ok(ChatCompletionEnvelope {
                response,
                raw_json: json!({
                    "object": "chat.completion.chunk.aggregate",
                    "chunks": raw_chunks,
                }),
            });
        }

        bail!(
            "stream request to /v1/chat/completions failed after {max_attempts} attempts. {}",
            Self::retry_hint()
        )
    }
}

fn extract_sse_data(event_block: &str) -> String {
    let mut data_lines = Vec::new();

    for line in event_block.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_string());
        }
    }

    data_lines.join("\n")
}

#[derive(Debug, Clone)]
pub struct ChatCompletionEnvelope {
    pub response: ChatCompletionResponse,
    pub raw_json: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>,
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(content.into()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(content.into()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    pub fn assistant(content: Option<String>, tool_calls: Option<Vec<ToolCall>>) -> Self {
        Self {
            role: "assistant".to_string(),
            content,
            name: None,
            tool_call_id: None,
            tool_calls,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content.into()),
            name: None,
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub model: Option<String>,
    pub choices: Vec<ChatChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatChoice {
    pub index: usize,
    pub message: ChatMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Debug, Deserialize)]
struct ModelInfo {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    choices: Vec<ChatCompletionChunkChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunkChoice {
    delta: ChatCompletionChunkDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ChatCompletionChunkDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChunkToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct ChunkToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(rename = "type")]
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    function: Option<ChunkToolFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct ChunkToolFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}
