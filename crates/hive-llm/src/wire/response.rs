//! Inbound streaming chunk shapes parsed from Server-Sent Events.

use serde::Deserialize;

#[derive(Deserialize)]
pub struct ChatChunk {
    #[serde(default)]
    pub choices: Vec<ChunkChoice>,
    #[serde(default)]
    pub usage: Option<WireUsage>,
}

#[derive(Deserialize)]
pub struct ChunkChoice {
    #[serde(default)]
    pub delta: ChunkDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
pub struct ChunkDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Deserialize)]
pub struct DeltaToolCall {
    #[serde(default)]
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<DeltaFn>,
}

#[derive(Deserialize, Default)]
pub struct DeltaFn {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Deserialize)]
pub struct WireUsage {
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
}
