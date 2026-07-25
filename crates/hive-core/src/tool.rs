use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::AppConfig;
use crate::event::{AgentEvent, EventSender};
use crate::message::ImageSource;
use crate::skill::SkillSource;
use crate::spawner::SubagentSpawner;
use crate::terminal::TerminalHandle;

/// What a tool returns after executing.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// Text handed back to the model as the tool result.
    pub content: String,
    pub is_error: bool,
    /// Images produced by the tool (e.g. a screenshot), fed back into context.
    pub images: Vec<ImageSource>,
}

impl ToolResult {
    pub fn ok(content: impl Into<String>) -> Self {
        ToolResult {
            content: content.into(),
            is_error: false,
            images: Vec::new(),
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        ToolResult {
            content: content.into(),
            is_error: true,
            images: Vec::new(),
        }
    }

    pub fn with_images(mut self, images: Vec<ImageSource>) -> Self {
        self.images = images;
        self
    }
}

/// Everything a tool needs at execution time, injected by the agent. Tools hold
/// no state of their own, so they can be zero-sized and self-registered.
pub struct ToolContext {
    pub cwd: PathBuf,
    pub events: EventSender,
    pub spawner: Arc<dyn SubagentSpawner>,
    pub skills: Arc<dyn SkillSource>,
    pub config: Arc<AppConfig>,
    /// Root-only interactive terminal service. Subagents receive `None`.
    pub terminal: Option<TerminalHandle>,
    /// Whether the active model accepts image inputs (gates `read_file` attaches).
    pub vision: bool,
    /// Recursion depth of the agent running this tool (0 = top-level).
    pub depth: usize,
    /// Correlates streamed `ToolOutput` events with the running tool card.
    pub call_id: String,
    /// When true, spawned workers get isolated git worktrees (MULTITASK).
    pub isolate_worktrees: bool,
    /// Shared Esc / cancel flag from the running turn.
    pub interrupt: Arc<std::sync::atomic::AtomicBool>,
}

impl ToolContext {
    /// Convenience for tools that stream partial output (e.g. shell).
    pub fn emit_output(&self, chunk: impl Into<String>) {
        let _ = self.events.send(AgentEvent::ToolOutput {
            id: self.call_id.clone(),
            chunk: chunk.into(),
        });
    }

    /// Emit the original file content before a modification so the TUI can
    /// offer a revert action.
    pub fn emit_snapshot(&self, path: impl Into<String>, content: impl Into<String>) {
        let _ = self.events.send(AgentEvent::FileSnapshot {
            id: self.call_id.clone(),
            path: path.into(),
            content: content.into(),
        });
    }
}

/// The contract every tool implements. Adding a capability = one new type.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON Schema for the arguments object.
    fn parameters(&self) -> serde_json::Value;
    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}

/// Compile-time registration record. Each tool file submits one of these via
/// `inventory`, and `hive-tools` collects them into the registry with zero
/// central edits.
pub struct ToolRegistration {
    pub make: fn() -> Arc<dyn Tool>,
}

inventory::collect!(ToolRegistration);
