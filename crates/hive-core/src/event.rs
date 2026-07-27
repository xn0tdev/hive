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
    /// Original file content before a file-writing tool modified it.
    /// The TUI stores this so the user can revert the change.
    FileSnapshot {
        id: String,
        path: String,
        content: String,
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
    /// The agent was stuck in a tool-call loop and is being redirected.
    LoopDetected {
        tool: String,
    },
    /// Context compaction started (spinner) or finished (with token counts).
    Compacted {
        /// None while compacting (spinner), Some when done.
        before: Option<u64>,
        after: u64,
    },
    TerminalStarted {
        id: String,
        command: String,
        description: String,
        rows: u16,
        cols: u16,
    },
    TerminalStartFailed {
        id: String,
        command: String,
        description: String,
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
        /// Context window in tokens (0 if unknown — keep previous value).
        context: u64,
        /// USD per 1M input tokens (0 if unknown).
        cost_input: f64,
        /// USD per 1M output tokens (0 if unknown).
        cost_output: f64,
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
    /// A goal was set — the agent will work autonomously until it expires or is stopped.
    GoalSet {
        objective: String,
        /// Absolute deadline (None = no timer).
        deadline: Option<std::time::Instant>,
    },
    /// A turn finished but the goal is still active — the driver should start the next turn.
    GoalContinue {
        objective: String,
        remaining_secs: u64,
    },
    /// The goal timer expired.
    GoalExpired {
        objective: String,
    },
    /// The goal was stopped by the user.
    GoalStopped,
    /// The goal loop was paused.
    GoalPaused,
    /// The goal loop was resumed.
    GoalResumed,
    /// Informational notice (e.g. "Exa disabled: no key").
    Notice(String),
    /// Saved sessions list (response to ListSessions).
    SessionsListed(Vec<crate::agent::SessionMeta>),
    /// A session was loaded — the TUI rebuilds its transcript from messages.
    SessionLoaded {
        title: String,
        model: String,
        /// Context window of the restored model (0 = unknown, keep current).
        context_window: u64,
        messages: Vec<crate::message::Message>,
        usage: crate::provider::Usage,
    },
    /// The agent updated its task list (todo progress).
    TodosUpdated {
        items: Vec<TodoItem>,
    },
    Error(String),
    TurnFinished,
}

/// One task in the agent's todo list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItem {
    pub text: String,
    pub done: bool,
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
    /// Context window in tokens (0 if unknown).
    pub context: u64,
    /// USD per 1M input tokens (0 if unknown).
    pub cost_input: f64,
    /// USD per 1M output tokens (0 if unknown).
    pub cost_output: f64,
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
