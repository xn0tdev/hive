use crate::agent::AgentMode;
use crate::provider::Usage;
use crate::terminal::{TerminalController, TerminalOutputFrame, TerminalProcessState};

/// Status of a subagent in the swarm / transcript card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentStatus {
    Running,
    Done,
    Failed,
}

/// One line of a subagent's conversation, forwarded for the expanded UI.
#[derive(Debug, Clone)]
pub enum SubagentLine {
    /// Assistant reasoning / thinking chunk (may be streamed).
    Thinking(String),
    /// Finalized assistant message.
    Assistant(String),
    /// A tool the subagent invoked.
    Tool {
        name: String,
        detail: String,
        /// `None` while running; `Some` when finished.
        ok: Option<bool>,
    },
    Notice(String),
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
    /// Cumulative session usage (prompt/completion totals for this chat).
    Usage(Usage),
    /// Tokens in the latest prompt — approx. how full the context window is.
    ContextTokens(u64),
    SubagentSpawned {
        id: String,
        label: String,
        /// Task prompt handed off by the parent agent (shown in the expanded view).
        prompt: String,
    },
    SubagentStatus {
        id: String,
        status: SubagentStatus,
        /// Short status line for the collapsed card (not the full report).
        detail: String,
    },
    /// Cumulative token usage for a subagent (forwarded from its agent loop).
    SubagentUsage {
        id: String,
        usage: Usage,
    },
    /// Incremental transcript from a running subagent (for click-to-expand).
    SubagentTranscript {
        id: String,
        line: SubagentLine,
    },
    /// `.hive/Plan.md` was created or updated (PLAN mode / plan amend).
    PlanUpdated {
        summary: String,
        body: String,
    },
    /// The agent switched its own working mode mid-session (e.g. into PLAN
    /// before tackling a large feature). `reason` explains why, for the card.
    ModeSwitched {
        mode: AgentMode,
        reason: String,
    },
    TerminalStarted {
        id: String,
        command: String,
        rows: u16,
        cols: u16,
    },
    TerminalStartFailed {
        id: String,
        command: String,
        message: String,
    },
    TerminalOutput {
        id: String,
        frame: TerminalOutputFrame,
    },
    TerminalState {
        id: String,
        controller: TerminalController,
        process: TerminalProcessState,
        revision: u64,
    },
    TerminalResized {
        id: String,
        rows: u16,
        cols: u16,
    },
    TerminalError {
        id: String,
        message: String,
    },
    /// The active model changed (e.g. via `/model`).
    ModelChanged {
        /// Provider model id (sent to the API).
        id: String,
        /// Pretty label for the TUI.
        display: String,
    },
    /// Live `/models` catalog for the Switch-model picker.
    ModelsListed {
        models: Vec<CatalogModel>,
    },
    /// Catalog fetch failed.
    ModelsListFailed(String),
    /// `/connect` profiles changed or active provider switched.
    ConnectionsUpdated {
        active: String,
        profiles: Vec<ConnectionInfo>,
    },
    /// Informational notice (e.g. "Exa disabled: no key").
    Notice(String),
    Error(String),
    TurnFinished,
}

/// One row in the live model catalog (grouped under `group`).
#[derive(Debug, Clone)]
pub struct CatalogModel {
    /// Provider model id (API).
    pub id: String,
    /// Pretty label.
    pub name: String,
    /// Secondary text (badges / short id).
    pub detail: String,
    /// Section header (provider label).
    pub group: String,
    /// `/connect` profile id this model was listed from.
    pub connection_id: String,
    /// True when the catalog says this model accepts image inputs.
    pub vision: bool,
}

/// One saved provider in the `/connect` picker.
#[derive(Debug, Clone)]
pub struct ConnectionInfo {
    pub id: String,
    pub label: String,
    /// Short secondary line (host).
    pub detail: String,
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
