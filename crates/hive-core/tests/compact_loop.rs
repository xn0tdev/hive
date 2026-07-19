//! Compaction shrinks history and mid-turn auto-compact lets the loop continue.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use hive_core::config::AppConfig;
use hive_core::error::Result;
use hive_core::message::{Message, Role, ToolCall};
use hive_core::provider::{ChatOutcome, ChatRequest, Delta, LlmProvider, Usage};
use hive_core::skill::no_skills;
use hive_core::spawner::noop_spawner;
use hive_core::tool::{Tool, ToolContext, ToolResult};
use hive_core::vision::no_vision;
use hive_core::{AgentBuilder, UserInput};

struct CompactAwareProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl LlmProvider for CompactAwareProvider {
    async fn chat_stream(
        &self,
        req: ChatRequest,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome> {
        // Summarizer calls have no tools.
        if req.tools.is_empty() {
            return Ok(ChatOutcome {
                message: Message::assistant(
                    "## Goal\nFinish the echo task\n\
## Done\nCalled echo once\n\
## Files\n(none)\n\
## State\nWaiting for final answer\n\
## Next\nReply done\n",
                ),
                usage: Usage {
                    prompt_tokens: 500,
                    completion_tokens: 80,
                    total_tokens: 580,
                },
                finish_reason: "stop".into(),
            });
        }

        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        // Inflate prompt tokens so the next loop iteration auto-compacts.
        let prompt_tokens = if n == 0 { 90_000 } else { 1_200 };
        if n == 0 {
            Ok(ChatOutcome {
                message: Message {
                    role: Role::Assistant,
                    content: Vec::new(),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "echo".into(),
                        arguments: r#"{"text":"hi"}"#.into(),
                    }],
                    tool_call_id: None,
                    name: None,
                },
                usage: Usage {
                    prompt_tokens,
                    completion_tokens: 10,
                    total_tokens: prompt_tokens + 10,
                },
                finish_reason: "tool_calls".into(),
            })
        } else {
            on_delta(Delta::Text("done".into()));
            Ok(ChatOutcome {
                message: Message::assistant("done"),
                usage: Usage {
                    prompt_tokens,
                    completion_tokens: 4,
                    total_tokens: prompt_tokens + 4,
                },
                finish_reason: "stop".into(),
            })
        }
    }
}

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "echo"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]})
    }
    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::ok(format!(
            "echo: {}",
            args.get("text").and_then(|v| v.as_str()).unwrap_or("")
        ))
    }
}

#[tokio::test]
async fn auto_compacts_mid_turn_and_finishes() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut cfg = AppConfig::default();
    cfg.agent.context_window = 100_000; // 75% = 75_000; first call reports 90_000

    let builder = AgentBuilder {
        provider: Arc::new(CompactAwareProvider {
            calls: AtomicUsize::new(0),
        }),
        tools: vec![Arc::new(EchoTool)],
        skills: no_skills(),
        vision: no_vision(),
        config: Arc::new(cfg),
    };
    let mut agent = builder.build(tx, "m".into(), 0, noop_spawner());
    let text = agent
        .run_turn(
            UserInput::from("please echo hi"),
            Arc::new(AtomicBool::new(false)),
        )
        .await;
    assert_eq!(text, "done");
}

#[tokio::test]
async fn manual_compact_replaces_history() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut cfg = AppConfig::default();
    // Huge window so the seeded turn does not auto-compact.
    cfg.agent.context_window = 1_000_000;

    let builder = AgentBuilder {
        provider: Arc::new(CompactAwareProvider {
            calls: AtomicUsize::new(0),
        }),
        tools: vec![Arc::new(EchoTool)],
        skills: no_skills(),
        vision: no_vision(),
        config: Arc::new(cfg),
    };
    let mut agent = builder.build(tx, "m".into(), 0, noop_spawner());
    let _ = agent
        .run_turn(
            UserInput::from("seed"),
            Arc::new(AtomicBool::new(false)),
        )
        .await;

    agent.compact().await.expect("compact");
}
