//! Bot chat engine: streams one persona turn from the provider on a detached
//! task, forwarding text deltas to the hub over a channel.

use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use hive_core::message::Message;
use hive_core::provider::{ChatRequest, Delta, LlmProvider};

/// Messages flowing from engine tasks back to the hub loop.
#[derive(Debug)]
pub enum EngineMsg {
    Delta { persona: String, text: String },
    Done { persona: String },
    Failed { persona: String, error: String },
}

/// Spawn one provider turn for `persona`. The task owns a snapshot of the
/// history; the hub keeps its own copy and reconciles via `EngineMsg`.
pub fn send(
    provider: Arc<dyn LlmProvider>,
    model: String,
    system: String,
    persona: String,
    history: Vec<Message>,
    text: String,
    tx: UnboundedSender<EngineMsg>,
) {
    tokio::spawn(async move {
        let mut messages = Vec::with_capacity(history.len() + 2);
        messages.push(Message::system(system));
        messages.extend(history);
        messages.push(Message::user(text));

        let delta_tx = tx.clone();
        let delta_persona = persona.clone();
        let outcome = provider
            .chat_stream(
                ChatRequest {
                    model: &model,
                    messages: &messages,
                    tools: &[],
                    temperature: None,
                    max_tokens: None,
                },
                &mut |delta| {
                    if let Delta::Text(text) = delta {
                        let _ = delta_tx.send(EngineMsg::Delta {
                            persona: delta_persona.clone(),
                            text,
                        });
                    }
                },
            )
            .await;

        match outcome {
            Ok(_) => {
                let _ = tx.send(EngineMsg::Done { persona });
            }
            Err(e) => {
                let _ = tx.send(EngineMsg::Failed {
                    persona,
                    error: e.to_string(),
                });
            }
        }
    });
}
