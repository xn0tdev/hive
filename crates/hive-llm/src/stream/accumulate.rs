//! Accumulates streamed deltas into a finished assistant message. Text is
//! forwarded live via a callback; tool-call fragments are merged by `index`.

use hive_core::message::{ContentPart, Message, Role, ToolCall};
use hive_core::provider::{ChatOutcome, Delta, Usage};

use crate::wire::response::{ChatChunk, DeltaToolCall};

#[derive(Default)]
struct PartialToolCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub struct Accumulator {
    text: String,
    reasoning: String,
    tool_calls: Vec<PartialToolCall>,
    usage: Usage,
    finish_reason: String,
}

/// A provider (or a corrupted stream) must not be able to force OOM by
/// sending a huge tool-call index that inflates the vec.
const MAX_TOOL_CALL_INDEX: usize = 64;

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one SSE chunk. Returns any visible/reasoning increments so the
    /// caller can forward them to the frontend as they arrive.
    pub fn push_chunk(&mut self, chunk: ChatChunk) -> Vec<Delta> {
        let mut deltas = Vec::new();

        if let Some(u) = chunk.usage {
            self.usage = Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
            };
        }

        for choice in chunk.choices {
            if let Some(fr) = choice.finish_reason {
                self.finish_reason = fr;
            }
            if let Some(c) = choice.delta.content {
                if !c.is_empty() {
                    self.text.push_str(&c);
                    deltas.push(Delta::Text(c));
                }
            }
            if let Some(r) = choice.delta.reasoning_content {
                if !r.is_empty() {
                    self.reasoning.push_str(&r);
                    deltas.push(Delta::Reasoning(r));
                }
            }
            if let Some(tcs) = choice.delta.tool_calls {
                for tc in tcs {
                    self.merge_tool_call(tc);
                }
            }
        }

        deltas
    }

    fn merge_tool_call(&mut self, tc: DeltaToolCall) {
        if tc.index > MAX_TOOL_CALL_INDEX {
            tracing::warn!(
                index = tc.index,
                "dropping tool-call delta with implausible index"
            );
            return;
        }
        while self.tool_calls.len() <= tc.index {
            self.tool_calls.push(PartialToolCall::default());
        }
        let slot = &mut self.tool_calls[tc.index];
        if let Some(id) = tc.id {
            if !id.is_empty() {
                slot.id = Some(id);
            }
        }
        if let Some(f) = tc.function {
            if let Some(name) = f.name {
                if !name.is_empty() {
                    slot.name = name;
                }
            }
            if let Some(args) = f.arguments {
                slot.arguments.push_str(&args);
            }
        }
    }

    pub fn finish(self) -> ChatOutcome {
        let mut content = Vec::new();
        if !self.text.is_empty() {
            content.push(ContentPart::Text(self.text));
        }

        let tool_calls = self
            .tool_calls
            .into_iter()
            .filter(|p| !p.name.is_empty())
            .map(|p| ToolCall {
                id: p
                    .id
                    .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple())),
                name: p.name,
                arguments: if p.arguments.is_empty() {
                    "{}".to_string()
                } else {
                    p.arguments
                },
            })
            .collect();

        let message = Message {
            role: Role::Assistant,
            content,
            tool_calls,
            tool_call_id: None,
            name: None,
            provider_items: Vec::new(),
        };

        ChatOutcome {
            message,
            usage: self.usage,
            finish_reason: self.finish_reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::response::DeltaFn;

    #[test]
    fn implausible_tool_call_index_is_dropped() {
        let mut acc = Accumulator::new();
        acc.merge_tool_call(DeltaToolCall {
            index: 1_000_000_000,
            id: Some("call_1".into()),
            function: Some(DeltaFn {
                name: Some("read_file".into()),
                arguments: Some("{}".into()),
            }),
        });
        assert_eq!(acc.tool_calls.len(), 0);
    }

    #[test]
    fn normal_indices_still_merge() {
        let mut acc = Accumulator::new();
        acc.merge_tool_call(DeltaToolCall {
            index: 2,
            id: Some("call_3".into()),
            function: Some(DeltaFn {
                name: Some("read_file".into()),
                arguments: Some("{}".into()),
            }),
        });
        assert_eq!(acc.tool_calls.len(), 3);
        assert_eq!(acc.tool_calls[2].name, "read_file");
    }
}
