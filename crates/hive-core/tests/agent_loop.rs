//! End-to-end test of the YOLO loop with a mock provider and a mock tool.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

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
