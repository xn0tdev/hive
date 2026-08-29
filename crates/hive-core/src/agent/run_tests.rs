use super::*;
use crate::{all_tools, no_skills, noop_spawner, ChatOutcome, CoreError, TerminalManager};
use async_trait::async_trait;
use std::sync::atomic::AtomicUsize;

struct NoopProvider;

#[async_trait]
impl LlmProvider for NoopProvider {
    async fn chat_stream(
        &self,
        _req: ChatRequest<'_>,
        _on_delta: &mut (dyn FnMut(Delta) + Send),
    ) -> Result<ChatOutcome, CoreError> {
        unreachable!("tool-spec tests do not call the provider")
    }
}

fn builder() -> AgentBuilder {
    AgentBuilder {
        provider: Arc::new(NoopProvider),
        tools: all_tools(),
        skills: no_skills(),
        config: Arc::new(AppConfig::default()),
    }
}

#[test]
fn tool_args_preview_uses_the_meaningful_value() {
    assert_eq!(
        tool_args_preview("read_file", r#"{"path":"src/main.rs","offset":12}"#),
        "src/main.rs"
    );
    assert_eq!(
        tool_args_preview("web_search", r#"{"query":"rust tui"}"#),
        "rust tui"
    );
    assert_eq!(tool_args_preview("custom", r#"{"z":1,"a":2}"#), "a, z");
    assert!(tool_args_preview("custom", "not json").is_empty());
}

struct DelayedParallelTool {
    name: &'static str,
    delay_ms: u64,
    output: &'static str,
}

#[async_trait]
impl Tool for DelayedParallelTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "test tool"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        ToolResult::ok(self.output)
    }
}

struct CountingTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "test tool"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.calls.fetch_add(1, Ordering::Relaxed);
        ToolResult::ok("unexpected")
    }
}

struct SchemaCountingTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for SchemaCountingTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "test tool"
    }

    fn parameters(&self) -> serde_json::Value {
        self.calls.fetch_add(1, Ordering::Relaxed);
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::ok("ok")
    }
}

#[test]
fn tool_schemas_are_built_once_per_agent() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = AgentBuilder {
        provider: Arc::new(NoopProvider),
        tools: vec![Arc::new(SchemaCountingTool {
            calls: calls.clone(),
        })],
        skills: no_skills(),
        config: Arc::new(AppConfig::default()),
    }
    .build(events, "test".into(), 0, noop_spawner());

    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let _ = agent.active_tool_specs();
    let _ = agent.active_tool_specs();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn parallel_results_keep_their_original_call_ids() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(DelayedParallelTool {
            name: "read_file",
            delay_ms: 30,
            output: "slow result",
        }),
        Arc::new(DelayedParallelTool {
            name: "grep",
            delay_ms: 0,
            output: "fast result",
        }),
    ];
    let mut agent = AgentBuilder {
        provider: Arc::new(NoopProvider),
        tools,
        skills: no_skills(),
        config: Arc::new(AppConfig::default()),
    }
    .build(events, "test".into(), 0, noop_spawner());
    let calls = vec![
        ToolCall {
            id: "slow".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        },
        ToolCall {
            id: "fast".into(),
            name: "grep".into(),
            arguments: "{}".into(),
        },
    ];

    agent
        .run_tools_parallel(&calls, &Arc::new(AtomicBool::new(false)))
        .await;

    let results = &agent.session.messages[1..];
    assert_eq!(results[0].tool_call_id.as_deref(), Some("slow"));
    assert_eq!(results[0].text(), "slow result");
    assert_eq!(results[1].tool_call_id.as_deref(), Some("fast"));
    assert_eq!(results[1].text(), "fast result");
}

#[tokio::test]
async fn malformed_tool_arguments_do_not_execute_the_tool() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut agent = AgentBuilder {
        provider: Arc::new(NoopProvider),
        tools: vec![Arc::new(CountingTool {
            calls: calls.clone(),
        })],
        skills: no_skills(),
        config: Arc::new(AppConfig::default()),
    }
    .build(events, "test".into(), 0, noop_spawner());

    agent
        .run_tool("bad-call", "echo", "{", &Arc::new(AtomicBool::new(false)))
        .await;

    assert_eq!(calls.load(Ordering::Relaxed), 0);
    let result = agent.session.messages.last().unwrap();
    assert_eq!(result.tool_call_id.as_deref(), Some("bad-call"));
    assert!(result.text().contains("invalid tool arguments"));
}

#[test]
fn interrupted_calls_are_closed_in_history() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let mut agent = builder().build(events, "test".into(), 0, noop_spawner());
    let calls = vec![
        ToolCall {
            id: "one".into(),
            name: "read_file".into(),
            arguments: "{}".into(),
        },
        ToolCall {
            id: "two".into(),
            name: "grep".into(),
            arguments: "{}".into(),
        },
    ];

    agent.finish_unrun_tool_calls(&calls, "turn interrupted");

    let results = &agent.session.messages[1..];
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].tool_call_id.as_deref(), Some("one"));
    assert_eq!(results[1].tool_call_id.as_deref(), Some("two"));
    assert!(results
        .iter()
        .all(|message| message.text().contains("turn interrupted")));
}

#[test]
fn make_hides_orchestrator_tools_multitask_shows_them() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let mut agent = builder().build(events, "test".into(), 0, noop_spawner());
    let make = tool_names(&agent);
    for name in [
        "spawn_subagent",
        "agent_observe",
        "agent_message",
        "agent_wait",
        "agent_close",
    ] {
        assert!(
            !make.iter().any(|t| t == name),
            "{name} must not appear in MAKE"
        );
    }
    agent.set_mode(AgentMode::Multitask);
    let multi = tool_names(&agent);
    for name in ["spawn_subagent", "agent_wait", "agent_close"] {
        assert!(
            multi.iter().any(|t| t == name),
            "{name} missing from MULTITASK"
        );
    }
    assert!(!multi.iter().any(|t| t == "run_shell"));
    assert!(!multi.iter().any(|t| t == "write_file"));
}

fn tool_names(agent: &Agent) -> Vec<String> {
    agent
        .active_tool_specs()
        .into_iter()
        .map(|tool| tool.name.clone())
        .collect()
}

#[test]
fn terminal_tools_only_root_make_with_manager() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events.clone());
    let root =
        builder().build_with_terminal(events.clone(), "test".into(), 0, noop_spawner(), manager);
    let root_without_manager = builder().build(events.clone(), "test".into(), 0, noop_spawner());
    let subagent = builder().build(events, "test".into(), 1, noop_spawner());

    let terminal_names = [
        "terminal_start",
        "terminal_read",
        "terminal_write",
        "terminal_stop",
    ];
    let root_names = tool_names(&root);
    let plain_root_names = tool_names(&root_without_manager);
    let subagent_names = tool_names(&subagent);
    for name in terminal_names {
        assert!(root_names.iter().any(|tool| tool == name), "{name}");
        assert!(
            !plain_root_names.iter().any(|tool| tool == name),
            "{name} leaked without a manager"
        );
        assert!(
            !subagent_names.iter().any(|tool| tool == name),
            "{name} leaked to a subagent"
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn interrupted_turn_stops_only_agent_controlled_terminal() {
    let (events, _) = tokio::sync::mpsc::unbounded_channel();
    let manager = TerminalManager::new(events.clone());
    let started = manager
        .start("cat", "test", std::path::Path::new("."))
        .await
        .unwrap();
    let mut agent = builder().build_with_terminal(
        events.clone(),
        "test".into(),
        0,
        noop_spawner(),
        manager.clone(),
    );
    let interrupt = Arc::new(AtomicBool::new(true));
    agent
        .run_turn(
            UserInput::from("stop"),
            interrupt,
            Arc::new(Mutex::new(None)),
        )
        .await;
    let stopped = manager.read(&started.id, None, None).await.unwrap();
    assert!(!matches!(
        stopped.session.process,
        crate::TerminalProcessState::Running
    ));

    let restarted = manager
        .start("cat", "test", std::path::Path::new("."))
        .await
        .unwrap();
    manager.attach(&restarted.id).unwrap();
    let interrupt = Arc::new(AtomicBool::new(true));
    agent
        .run_turn(
            UserInput::from("keep"),
            interrupt,
            Arc::new(Mutex::new(None)),
        )
        .await;
    let preserved = manager.read(&restarted.id, None, None).await.unwrap();
    assert!(matches!(
        preserved.session.process,
        crate::TerminalProcessState::Running
    ));
    manager.shutdown();
}
