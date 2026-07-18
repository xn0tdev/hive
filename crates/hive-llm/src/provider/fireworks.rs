//! The Fireworks (OpenAI-compatible) implementation of `LlmProvider`.

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;

use hive_core::error::{CoreError, Result};
use hive_core::provider::{ChatOutcome, ChatRequest, Delta, LlmProvider};

use crate::stream::Accumulator;
use crate::wire::{build_request, ChatChunk};

/// Talks to any OpenAI-compatible `/chat/completions` endpoint. Fireworks is the
/// default, but the same client works with OpenAI, OpenRouter, local servers,
/// etc. — just change `base_url` and the key.
pub struct FireworksProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl FireworksProvider {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        FireworksProvider {
            client: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

#[async_trait]
impl LlmProvider for FireworksProvider {
    async fn chat_stream(
        &self,
        req: ChatRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome> {
        let body = build_request(&req);

        let resp = self
            .client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Api(format!("{status}: {text}")));
        }

        let mut stream = resp.bytes_stream().eventsource();
        let mut acc = Accumulator::new();

        while let Some(event) = stream.next().await {
            let event = event.map_err(|e| CoreError::Http(e.to_string()))?;
            if event.data == "[DONE]" {
                break;
            }
            let chunk: ChatChunk = match serde_json::from_str(&event.data) {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!("skipping unparsable chunk: {e}: {}", event.data);
                    continue;
                }
            };
            for delta in acc.push_chunk(chunk) {
                on_delta(delta);
            }
        }

        Ok(acc.finish())
    }
}
