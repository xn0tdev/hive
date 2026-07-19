//! `hive-llm`: OpenAI-compatible chat provider (default target: Fireworks).
//! Implements `hive_core::LlmProvider` with streaming and tool calling.
//!
//! Organized by responsibility:
//! - `wire/` — request/response JSON shapes and conversions.
//! - `stream/` — SSE delta accumulation into a finished message.
//! - `provider/` — the concrete client(s).
//! - [`catalog`] — `GET /models` listing + models.dev enrichment (setup / picker).

pub mod catalog;
mod provider;
mod stream;
mod wire;

pub use provider::FireworksProvider;
