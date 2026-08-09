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
        req: ChatRequest<'_>,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome> {
        let body = build_request(&req);
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
                on_delta(delta);
            }
        }

        Ok(acc.finish())
    }

    async fn responses_stream(
        &self,
        req: ChatRequest<'_>,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome> {
        let body = build_responses_request(&req);
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
                on_delta(delta);
            }
        }

        Ok(acc.finish())
    }

    async fn post<T: serde::Serialize + ?Sized>(&self, body: &T) -> Result<reqwest::Response> {
        let resp = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
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
        match self.api_flavor() {
            ApiFlavor::ChatCompletions => self.chat_completions_stream(req, on_delta).await,
            ApiFlavor::Responses => self.responses_stream(req, on_delta).await,
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
            "http://localhost:11434/v1",
            "https://api.openai.com.example.test/v1",
        ] {
            let provider = FireworksProvider::new(base_url, "key");
            assert_eq!(provider.api_flavor(), ApiFlavor::ChatCompletions);
            assert_eq!(provider.endpoint(), format!("{base_url}/chat/completions"));
        }
    }
}
