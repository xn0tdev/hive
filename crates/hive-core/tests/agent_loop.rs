//! End-to-end test of the agent loop with a mock provider and a mock tool.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use hive_core::config::AppConfig;
use hive_core::error::Result;
use hive_core::event::AgentEvent;
use hive_core::message::{Message, Role, ToolCall};
use hive_core::provider::{ChatOutcome, ChatRequest, Delta, LlmProvider, Usage};
use hive_core::skill::no_skills;
use hive_core::spawner::noop_spawner;
use hive_core::tool::{Tool, ToolContext, ToolResult};
use hive_core::{AgentBuilder, UserInput};

struct MockProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn chat_stream(
        &self,
        _req: ChatRequest<'_>,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            let msg = Message {
                role: Role::Assistant,
                content: Vec::new(),
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "echo".to_string(),
                    arguments: r#"{"text":"hello world"}"#.to_string(),
                }],
                tool_call_id: None,
                name: None,
                provider_items: Vec::new(),
            };
            Ok(ChatOutcome {
                message: msg,
                usage: Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                },
                finish_reason: "tool_calls".to_string(),
            })
        } else {
            on_delta(Delta::Text("all ".to_string()));
            on_delta(Delta::Text("done".to_string()));
            Ok(ChatOutcome {
                message: Message::assistant("all done"),
                usage: Usage {
                    prompt_tokens: 12,
                    completion_tokens: 3,
                    total_tokens: 15,
                },
                finish_reason: "stop".to_string(),
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
        "echo the input text"
    }
    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]})
    }
    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
        ToolResult::ok(format!("echo: {text}"))
    }
}

#[tokio::test]
async fn runs_tool_then_answers() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let builder = AgentBuilder {
        provider: Arc::new(MockProvider {
            calls: AtomicUsize::new(0),
        }),
        tools: vec![Arc::new(EchoTool)],
        skills: no_skills(),
        config: Arc::new(AppConfig::default()),
    };

    let mut agent = builder.build(
        tx,
        "accounts/fireworks/routers/kimi-k2p6-fast".to_string(),
        0,
        noop_spawner(),
    );

    let interrupt = Arc::new(AtomicBool::new(false));
    let follow_up = Arc::new(std::sync::Mutex::new(None));
    let final_text = agent
        .run_turn(UserInput::from("hi"), interrupt, follow_up)
        .await;
    assert_eq!(final_text, "all done");

    let mut saw_tool_started = false;
    let mut saw_tool_ok = false;
    let mut saw_final = false;
    let mut saw_finish = false;
    let mut total_tokens = 0;

    while let Ok(ev) = rx.try_recv() {
        match ev {
            AgentEvent::ToolStarted { name, .. } if name == "echo" => saw_tool_started = true,
            AgentEvent::ToolFinished { name, ok, .. } if name == "echo" && ok => saw_tool_ok = true,
            AgentEvent::AssistantMessage(t) if t == "all done" => saw_final = true,
            AgentEvent::Usage(u) => total_tokens = u.total_tokens,
            AgentEvent::TurnFinished => saw_finish = true,
            _ => {}
        }
    }

    assert!(saw_tool_started, "expected echo tool to start");
    assert!(saw_tool_ok, "expected echo tool to finish ok");
    assert!(saw_final, "expected final assistant message");
    assert!(saw_finish, "expected turn to finish");
    assert_eq!(total_tokens, 30, "usage should accumulate across steps");
}

struct BatchProvider {
    calls: AtomicUsize,
    result_order: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl LlmProvider for BatchProvider {
    async fn chat_stream(
        &self,
        req: ChatRequest<'_>,
        _on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return Ok(ChatOutcome {
                message: Message {
                    role: Role::Assistant,
                    content: Vec::new(),
                    tool_calls: vec![
                        call("read-1", "read_file", "slow"),
                        call("read-2", "read_file", "fast"),
                        call("shell-1", "run_shell", "write"),
                        call("search-1", "grep", "needle"),
                    ],
                    tool_call_id: None,
                    name: None,
                    provider_items: Vec::new(),
                },
                usage: Usage::default(),
                finish_reason: "tool_calls".into(),
            });
        }

        let order = req
            .messages
            .iter()
            .filter_map(|message| message.tool_call_id.clone())
            .collect();
        *self.result_order.lock().expect("result order lock") = order;
        Ok(ChatOutcome {
            message: Message::assistant("done"),
            usage: Usage::default(),
            finish_reason: "stop".into(),
        })
    }
}

fn call(id: &str, name: &str, label: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: serde_json::json!({ "label": label }).to_string(),
    }
}

struct NamedTool(&'static str);

#[async_trait]
impl Tool for NamedTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "test tool"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "label": { "type": "string" } },
            "required": ["label"]
        })
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let label = args["label"].as_str().unwrap_or_default();
        if label == "slow" {
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }
        ToolResult::ok(format!("{}:{label}", self.0))
    }
}

#[tokio::test]
async fn batches_only_adjacent_parallel_reads_and_preserves_result_order() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let result_order = Arc::new(Mutex::new(Vec::new()));
    let builder = AgentBuilder {
        provider: Arc::new(BatchProvider {
            calls: AtomicUsize::new(0),
            result_order: result_order.clone(),
        }),
        tools: vec![
            Arc::new(NamedTool("read_file")),
            Arc::new(NamedTool("run_shell")),
            Arc::new(NamedTool("grep")),
        ],
        skills: no_skills(),
        config: Arc::new(AppConfig::default()),
    };
    let mut agent = builder.build(tx, "test-model".into(), 0, noop_spawner());
    let result = agent
        .run_turn(
            UserInput::from("inspect"),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
        )
        .await;
    assert_eq!(result, "done");

    let mut batches = Vec::new();
    let mut finished = Vec::new();
    while let Ok(event) = rx.try_recv() {
        match event {
            AgentEvent::ToolBatchStarted { calls, .. } => {
                batches.push(calls.into_iter().map(|call| call.id).collect::<Vec<_>>());
            }
            AgentEvent::ToolBatchFinished { failed, .. } => finished.push(failed),
            _ => {}
        }
    }
    assert_eq!(batches, vec![vec!["read-1", "read-2"]]);
    assert_eq!(finished, vec![0]);
    assert_eq!(
        *result_order.lock().expect("result order lock"),
        vec!["read-1", "read-2", "shell-1", "search-1"]
    );
}

struct NeverStopsProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl LlmProvider for NeverStopsProvider {
    async fn chat_stream(
        &self,
        _req: ChatRequest<'_>,
        _on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome> {
        let round = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ChatOutcome {
            message: Message {
                role: Role::Assistant,
                content: Vec::new(),
                tool_calls: vec![call(
                    &format!("read-{round}"),
                    "read_file",
                    &format!("file-{round}"),
                )],
                tool_call_id: None,
                name: None,
                provider_items: Vec::new(),
            },
            usage: Usage::default(),
            finish_reason: "tool_calls".to_string(),
        })
    }
}

#[tokio::test]
async fn turn_stops_at_the_round_cap() {
    let provider = Arc::new(NeverStopsProvider {
        calls: AtomicUsize::new(0),
    });
    let builder = AgentBuilder {
        provider: provider.clone(),
        tools: vec![Arc::new(NamedTool("read_file"))],
        skills: no_skills(),
        config: Arc::new(AppConfig::default()),
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut agent = builder.build(tx, "test-model".into(), 0, noop_spawner());
    let result = agent
        .run_turn(
            UserInput::from("keep reading"),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
        )
        .await;

    assert_eq!(result, "");
    assert_eq!(
        provider.calls.load(Ordering::SeqCst) as u64,
        hive_core::MAX_ROUNDS,
        "the turn must stop exactly at the round cap"
    );

    let mut stopped = false;
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::Notice(msg) = ev {
            if msg.contains("not converging") {
                stopped = true;
            }
        }
    }
    assert!(stopped, "expected a round-cap Notice event");
}

/// Always answers with `finish_reason: "length"` — the model ran out of
/// context mid-answer.
struct OverflowProvider;

#[async_trait]
impl LlmProvider for OverflowProvider {
    async fn chat_stream(
        &self,
        _req: ChatRequest<'_>,
        on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome> {
        on_delta(Delta::Text("truncated answer".to_string()));
        Ok(ChatOutcome {
            message: Message::assistant("truncated answer"),
            usage: Usage::default(),
            finish_reason: "length".to_string(),
        })
    }
}

#[tokio::test]
async fn context_overflow_ends_the_turn_instead_of_looping() {
    let builder = AgentBuilder {
        provider: Arc::new(OverflowProvider),
        tools: Vec::new(),
        skills: no_skills(),
        config: Arc::new(AppConfig::default()),
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut agent = builder.build(tx, "test-model".into(), 0, noop_spawner());
    let result = agent
        .run_turn(
            UserInput::from("hi"),
            Arc::new(AtomicBool::new(false)),
            Arc::new(Mutex::new(None)),
        )
        .await;

    // Compaction needs at least a few messages; a short session cannot
    // compact, so the turn must surface a clear error and end — no loop.
    assert_eq!(result, "truncated answer");

    let mut saw_overflow = false;
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::Error(msg) = ev {
            if msg.contains("overflowed") && msg.contains("compaction") {
                saw_overflow = true;
            }
        }
    }
    assert!(saw_overflow, "expected a context-overflow error event");
}
