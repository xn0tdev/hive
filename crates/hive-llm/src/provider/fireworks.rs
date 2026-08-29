//! The Fireworks (OpenAI-compatible) implementation of `LlmProvider`.

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;

use hive_core::error::{CoreError, Result};
use hive_core::provider::{ChatOutcome, ChatRequest, Delta, LlmProvider};

use crate::stream::{Accumulator, ResponsesAccumulator};
use crate::wire::{build_request, build_responses_request, ChatChunk, ResponseEvent};

/// Talks to Fireworks and other OpenAI-compatible providers. Official OpenAI
/// uses `/responses`; compatible third-party APIs retain `/chat/completions`.
pub struct FireworksProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

/// How long to wait for the response head. The body is a stream that can
/// legitimately run for many minutes, so only the handshake is bounded here —
/// a total timeout would kill long answers mid-sentence.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Gap between two stream chunks. A provider that stops sending without closing
/// the connection used to hang the turn forever; this ends it instead.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Transient failures are retried only before the first token lands — once
/// output starts streaming, retrying would duplicate it in the UI.
const MAX_ATTEMPTS: usize = 3;

/// Base delay for the first retry; doubles per attempt, capped.
const RETRY_BASE: std::time::Duration = std::time::Duration::from_millis(750);
const RETRY_CAP: std::time::Duration = std::time::Duration::from_secs(8);

/// True for errors worth retrying: network-level failures and well-known
/// transient HTTP statuses (408, 429, 5xx).
fn is_retryable(error: &CoreError) -> bool {
    match error {
        // Connection reset, timeout, closed socket, TLS hiccup.
        CoreError::Http(_) => true,
        // API failures carry the status code up front ("429 Too Many ..." or
        // "503: <body>"). Parse the leading digit run; anything that does not
        // start with a plausible status is not retried.
        CoreError::Api(msg) => msg
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|s| s.parse::<u16>().ok())
            .is_some_and(|code| code == 408 || code == 429 || (500..600).contains(&code)),
        _ => false,
    }
}

/// Exponential backoff with time-based jitter (no rand dependency).
fn retry_delay(attempt: usize) -> std::time::Duration {
    let base = RETRY_BASE
        .saturating_mul(2u32.pow(attempt.saturating_sub(1) as u32))
        .min(RETRY_CAP);
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_millis() as u64)
        .unwrap_or(0)
        % (base.as_millis() as u64 / 2 + 1);
    base.saturating_sub(std::time::Duration::from_millis(jitter))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApiFlavor {
    ChatCompletions,
    Responses,
}

impl FireworksProvider {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .unwrap_or_else(|_| {
                // Fallback: still enforce timeouts so a hanging connection
                // can't stall the turn forever.
                reqwest::Client::builder()
                    .connect_timeout(CONNECT_TIMEOUT)
                    .read_timeout(READ_TIMEOUT)
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new())
            });
        FireworksProvider {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }

    fn api_flavor(&self) -> ApiFlavor {
        let is_openai = reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .is_some_and(|host| host.eq_ignore_ascii_case("api.openai.com"));
        if is_openai {
            ApiFlavor::Responses
        } else {
            ApiFlavor::ChatCompletions
        }
    }

    fn endpoint(&self) -> String {
        let path = match self.api_flavor() {
            ApiFlavor::ChatCompletions => "chat/completions",
            ApiFlavor::Responses => "responses",
        };
        format!("{}/{path}", self.base_url)
    }

    async fn chat_completions_stream(
        &self,
        req: &ChatRequest<'_>,
        on_delta: &mut (dyn FnMut(Delta) + Send),
        saw_delta: &mut bool,
    ) -> Result<ChatOutcome> {
        let mut body = build_request(req);
        body.max_tokens = crate::auth::default_max_tokens(&self.base_url, body.max_tokens);
        let resp = self.post(&body).await?;
        let mut stream = resp.bytes_stream().eventsource();
        let mut acc = Accumulator::new();

        while let Some(event) = stream.next().await {
            let event = event.map_err(|e| CoreError::Http(e.to_string()))?;
            if event.data == "[DONE]" {
                break;
            }
            let chunk: ChatChunk = match serde_json::from_str(&event.data) {
                Ok(chunk) => chunk,
                Err(error) => {
                    tracing::debug!("skipping unparsable chunk: {error}: {}", event.data);
                    continue;
                }
            };
            for delta in acc.push_chunk(chunk) {
                *saw_delta = true;
                on_delta(delta);
            }
        }

        Ok(acc.finish())
    }

    async fn responses_stream(
        &self,
        req: &ChatRequest<'_>,
        on_delta: &mut (dyn FnMut(Delta) + Send),
        saw_delta: &mut bool,
    ) -> Result<ChatOutcome> {
        let body = build_responses_request(req);
        let resp = self.post(&body).await?;
        let mut stream = resp.bytes_stream().eventsource();
        let mut acc = ResponsesAccumulator::new();

        while let Some(event) = stream.next().await {
            let event = event.map_err(|e| CoreError::Http(e.to_string()))?;
            if event.data == "[DONE]" {
                break;
            }
            let event: ResponseEvent = match serde_json::from_str(&event.data) {
                Ok(event) => event,
                Err(error) => {
                    tracing::debug!(
                        "skipping unparsable Responses event: {error}: {}",
                        event.data
                    );
                    continue;
                }
            };
            for delta in acc.push_event(event).map_err(CoreError::Api)? {
                *saw_delta = true;
                on_delta(delta);
            }
        }

        Ok(acc.finish())
    }

    async fn post<T: serde::Serialize + ?Sized>(&self, body: &T) -> Result<reqwest::Response> {
        let resp = crate::auth::with_auth(
            self.client.post(self.endpoint()),
            &self.api_key,
            &self.base_url,
        )
        .json(body)
        .send()
        .await
        .map_err(|error| CoreError::Http(error.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Api(format!("{status}: {text}")));
        }
        Ok(resp)
    }
}

#[async_trait]
impl LlmProvider for FireworksProvider {
    async fn chat_stream(
        &self,
        req: ChatRequest<'_>,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome> {
        let flavor = self.api_flavor();
        let mut attempt: usize = 0;
        loop {
            attempt += 1;
            let mut saw_delta = false;
            let outcome = match flavor {
                ApiFlavor::ChatCompletions => {
                    self.chat_completions_stream(&req, on_delta, &mut saw_delta)
                        .await
                }
                ApiFlavor::Responses => {
                    self.responses_stream(&req, on_delta, &mut saw_delta).await
                }
            };
            match outcome {
                Ok(outcome) => return Ok(outcome),
                Err(error) if attempt < MAX_ATTEMPTS && !saw_delta && is_retryable(&error) => {
                    let delay = retry_delay(attempt);
                    tracing::warn!(
                        target: "hive::provider",
                        attempt,
                        ?error,
                        delay_ms = delay.as_millis(),
                        "transient provider error before first token; retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_openai_uses_responses_endpoint() {
        let provider = FireworksProvider::new("https://api.openai.com/v1/", "key");
        assert_eq!(provider.api_flavor(), ApiFlavor::Responses);
        assert_eq!(provider.endpoint(), "https://api.openai.com/v1/responses");
    }

    #[test]
    fn compatible_providers_keep_chat_completions() {
        for base_url in [
            "https://api.fireworks.ai/inference/v1",
            "https://openrouter.ai/api/v1",
            "https://api.anthropic.com/v1",
            "http://localhost:11434/v1",
            "https://api.openai.com.example.test/v1",
        ] {
            let provider = FireworksProvider::new(base_url, "key");
            assert_eq!(provider.api_flavor(), ApiFlavor::ChatCompletions);
            assert_eq!(provider.endpoint(), format!("{base_url}/chat/completions"));
        }
    }

    #[test]
    fn retry_classifies_transient_errors() {
        assert!(is_retryable(&CoreError::Http("connection reset".into())));
        assert!(is_retryable(&CoreError::Api("429 Too Many Requests".into())));
        assert!(is_retryable(&CoreError::Api("503 Service Unavailable".into())));
        assert!(is_retryable(&CoreError::Api("408: upstream timeout".into())));
        assert!(!is_retryable(&CoreError::Api("404 Not Found".into())));
        assert!(!is_retryable(&CoreError::Api("401 Unauthorized".into())));
        assert!(!is_retryable(&CoreError::Api("no status here".into())));
        assert!(!is_retryable(&CoreError::Parse("bad json".into())));
    }

    #[test]
    fn retry_delay_grows_and_stays_capped() {
        let first = retry_delay(1);
        let second = retry_delay(2);
        let third = retry_delay(3);
        // Base 750ms doubling with ±50% jitter; attempt 3 is 1500–3000ms.
        assert!((375..=750).contains(&first.as_millis()));
        assert!((750..=1500).contains(&second.as_millis()));
        assert!((1500..=3000).contains(&third.as_millis()));
        // Attempt 5 would be 12s — the cap at 8s must hold.
        assert!(retry_delay(5) <= RETRY_CAP);
    }
}
