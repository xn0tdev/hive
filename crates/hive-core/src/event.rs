use crate::provider::Usage;

/// Status of a subagent in the swarm panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentStatus {
    Running,
    Done,
    Failed,
}

/// Everything the agent core wants to tell a frontend. The TUI is just one
/// consumer of this stream; a headless frontend could consume the same events.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    TurnStarted,
    /// Model started producing an assistant message; frontends open a fresh bubble.
    AssistantStarted,
    /// A visible text increment.
    AssistantTextDelta(String),
    /// A reasoning/thinking increment.
    ReasoningDelta(String),
    /// The finalized assistant text for the current message (used to re-render
    /// with full markdown once streaming completes).
    AssistantMessage(String),
    ToolStarted {
        id: String,
        name: String,
        args_preview: String,
    },
    /// Streamed output from a running tool (e.g. shell stdout).
    ToolOutput {
        id: String,
        chunk: String,
    },
    ToolFinished {
        id: String,
        name: String,
        ok: bool,
        summary: String,
    },
    Usage(Usage),
    SubagentSpawned {
        id: String,
        label: String,
    },
    SubagentStatus {
        id: String,
        status: SubagentStatus,
        detail: String,
    },
    /// The active model changed (e.g. via `/model`).
    ModelChanged(String),
    /// Informational notice (e.g. "Exa disabled: no key").
    Notice(String),
    Error(String),
    TurnFinished,
}

/// The channel end used to push events toward the frontend. Unbounded so the
/// agent never blocks on a slow renderer.
pub type EventSender = tokio::sync::mpsc::UnboundedSender<AgentEvent>;
pub type EventReceiver = tokio::sync::mpsc::UnboundedReceiver<AgentEvent>;

/// A frontend that reacts to agent events. The TUI implements its own loop, but
/// this trait is the seam for alternative frontends (headless, logging, tests).
pub trait Renderer: Send {
    fn handle(&mut self, event: AgentEvent);
}
