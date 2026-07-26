use async_trait::async_trait;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::message::Message;

/// Tool advertisement sent to the model (OpenAI function-calling schema).
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the parameters object.
    pub parameters: serde_json::Value,
}

/// One request to the chat completion endpoint.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// Token accounting returned by the provider.
#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, rhs: Self) {
        self.prompt_tokens += rhs.prompt_tokens;
        self.completion_tokens += rhs.completion_tokens;
        self.total_tokens += rhs.total_tokens;
    }
}

/// A streamed increment from the model.
#[derive(Debug, Clone)]
pub enum Delta {
    /// Visible assistant text.
    Text(String),
    /// Reasoning / thinking tokens (rendered dimmed if the model emits them).
    Reasoning(String),
}

/// The final result of a streamed chat turn.
#[derive(Debug, Clone)]
pub struct ChatOutcome {
    pub message: Message,
    pub usage: Usage,
    pub finish_reason: String,
}

/// The contract every LLM backend implements. The provider drives the network
/// stream internally and pushes text increments through `on_delta`, then
/// returns the fully-assembled assistant message.
///
/// Implementing a new backend = one type implementing this trait. Nothing else
/// in the workspace needs to change.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat_stream(
        &self,
        req: ChatRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome>;
}
