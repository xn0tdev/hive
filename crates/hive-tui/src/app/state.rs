//! Plain data types describing what's currently on screen.

use hive_core::event::{SubagentLine, SubagentStatus};
use hive_core::provider::Usage;
use hive_core::{AgentMode, TerminalController, TerminalInputRequest, TerminalProcessState};

/// Which conversation the transcript is showing.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum ChatView {
    #[default]
    Main,
    /// Read-only view of a subagent's thread (by card id).
    Subagent(String),
    /// Plan.md preview with text-highlight corrections / Make.
    Plan,
    /// Interactive or read-only persistent terminal session.
    Terminal(String),
}

/// Status of the Plan.md transcript card.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    Writing,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Ok,
    Err,
}

#[derive(Clone)]
pub struct ToolCard {
    pub id: String,
    pub name: String,
    pub args: String,
    pub output: String,
    pub status: ToolStatus,
    pub started: std::time::Instant,
    /// Set when the tool finishes; `None` while still running.
    pub elapsed_ms: Option<u128>,
    /// Whether completed output is expanded. Running and failed tools always
    /// keep their useful progress/error preview visible.
    pub details_open: bool,
    /// Original file content before a write_file/edit_file/delete_path
    /// modified it — used for the "Revert" context-menu action.
    pub snapshot: Option<FileSnapshot>,
}

/// Saved file content for revert.
#[derive(Clone)]
pub struct FileSnapshot {
    pub path: String,
    pub content: String,
}

impl ToolCard {
    pub fn secs(&self) -> f64 {
        match self.elapsed_ms {
            Some(ms) => ms as f64 / 1000.0,
            None => self.started.elapsed().as_millis() as f64 / 1000.0,
        }
    }
}

#[derive(Clone)]
pub struct ExploreCard {
    pub id: String,
    pub tools: Vec<ToolCard>,
    pub started: std::time::Instant,
    pub elapsed_ms: Option<u128>,
    pub failed: usize,
    pub details_open: bool,
}

impl ExploreCard {
    pub fn running(&self) -> bool {
        self.elapsed_ms.is_none()
            || self
                .tools
                .iter()
                .any(|tool| tool.status == ToolStatus::Running)
    }

    pub fn secs(&self) -> f64 {
        match self.elapsed_ms {
            Some(ms) => ms as f64 / 1000.0,
            None => self.started.elapsed().as_millis() as f64 / 1000.0,
        }
    }
}

pub struct TerminalCard {
    pub id: String,
    pub command: String,
    /// Why the agent started this terminal (from the tool call).
    pub description: String,
    pub controller: TerminalController,
    pub process: TerminalProcessState,
    pub revision: u64,
    pub screen: vt100::Screen,
    pub started: std::time::Instant,
    pub elapsed_ms: Option<u128>,
}

impl TerminalCard {
    /// Structured prompt state from the current terminal screen.
    pub fn input_request(&self) -> Option<TerminalInputRequest> {
        if !matches!(self.process, TerminalProcessState::Running) {
            return None;
        }
        let (rows, cols) = self.screen.size();
        let mut text = String::new();
        for row in 0..rows {
            for col in 0..cols {
                match self.screen.cell(row, col) {
                    Some(cell) if cell.has_contents() => text.push_str(cell.contents()),
                    _ => text.push(' '),
                }
            }
            text.push('\n');
        }
        hive_core::terminal::terminal_input_request(&text)
    }

    /// Text of the current prompt, kept for compact status/render call sites.
    pub fn awaiting_user(&self) -> Option<String> {
        self.input_request().map(|request| request.prompt)
    }

    pub fn secs(&self) -> f64 {
        match self.elapsed_ms {
            Some(ms) => ms as f64 / 1000.0,
            None => self.started.elapsed().as_millis() as f64 / 1000.0,
        }
    }

    /// Short process state label for the status line.
    pub fn status_text(&self) -> String {
        match &self.process {
            TerminalProcessState::Running => "running".to_string(),
            TerminalProcessState::Exited { code } => format!("exited {code}"),
            TerminalProcessState::Failed { message } => format!("failed: {message}"),
        }
    }
}

/// Up/down arrow history of submitted prompts (shell-style recall).
#[derive(Default)]
pub struct PromptHistory {
    pub entries: Vec<String>,
    /// `None` = not browsing; `Some(i)` = showing `entries[i]`.
    pub index: Option<usize>,
    /// Text that was in the composer before history browsing started.
    pub draft: String,
}

impl PromptHistory {
    pub fn push(&mut self, text: &str) {
        if text.trim().is_empty() {
            return;
        }
        if self.entries.last().is_some_and(|e| e == text) {
            return;
        }
        self.entries.push(text.to_string());
        self.index = None;
        self.draft.clear();
    }

    pub fn is_browsing(&self) -> bool {
        self.index.is_some()
    }

    pub fn reset(&mut self) {
        self.index = None;
        self.draft.clear();
    }
}

/// A small centered context menu shown when clicking a transcript block
/// (user prompt or tool card). Offers actions like Copy, Recall, Revert.
pub struct ContextMenu {
    pub items: Vec<ContextMenuItem>,
    pub selected: usize,
}

pub struct ContextMenuItem {
    pub label: String,
    pub action: ContextAction,
}

#[derive(Clone)]
pub enum ContextAction {
    Copy(String),
    ToggleToolDetails {
        id: String,
    },
    /// Revert a file-writing tool by restoring the original content.
    RevertFile {
        path: String,
        content: String,
    },
}

/// A reasoning ("thinking") segment. Collapsed by default in the UI; carries
/// its own timing so the header can show how long the model thought.
pub struct Thought {
    pub text: String,
    pub started: std::time::Instant,
    /// Set once the model moves on (text/tool/turn end); `None` = still going.
    pub elapsed_ms: Option<u128>,
    /// Whether the full text is revealed (click the header or ctrl+t).
    pub open: bool,
}

impl Default for Thought {
    fn default() -> Self {
        Self::new()
    }
}

impl Thought {
    pub fn new() -> Self {
        Thought {
            text: String::new(),
            started: std::time::Instant::now(),
            elapsed_ms: None,
            open: false,
        }
    }

    pub fn secs(&self) -> f64 {
        match self.elapsed_ms {
            Some(ms) => ms as f64 / 1000.0,
            None => self.started.elapsed().as_millis() as f64 / 1000.0,
        }
    }

    /// Rough token estimate (~4 chars per token) for the collapsed header.
    pub fn approx_tokens(&self) -> usize {
        self.text.chars().count().div_ceil(4)
    }
}

/// Inline transcript card for a subagent job.
/// Flat like a Thought row — no bordered panel. Click opens a dedicated chat view.
#[derive(Clone)]
pub struct SubagentCard {
    pub id: String,
    pub label: String,
    pub status: SubagentStatus,
    /// Short status/description under the title (not the full report).
    pub detail: String,
    /// Task prompt handed off by the main agent.
    pub prompt: String,
    /// Subagent conversation for the dedicated chat view.
    pub lines: Vec<SubagentLine>,
    /// Cumulative token usage for this subagent (updated as it runs).
    pub usage: Usage,
    pub started: std::time::Instant,
    pub elapsed_ms: Option<u128>,
}

impl SubagentCard {
    pub fn secs(&self) -> f64 {
        match self.elapsed_ms {
            Some(ms) => ms as f64 / 1000.0,
            None => self.started.elapsed().as_millis() as f64 / 1000.0,
        }
    }

    pub fn status_text(&self) -> &str {
        if !self.detail.is_empty() {
            return self.detail.as_str();
        }
        match self.status {
            SubagentStatus::Running => "working",
            SubagentStatus::Done => "done",
            SubagentStatus::Failed => "failed",
        }
    }
}

/// Inline transcript card for the workspace plan file.
#[derive(Clone)]
pub struct PlanCard {
    pub summary: String,
    pub body: String,
    pub status: PlanStatus,
    /// True once the plan has been rewritten over an existing body — drives the
    /// `UPDATED` corner badge on the card so a revision is visible at a glance.
    pub revised: bool,
}

/// How a rendered assistant row connects to the preceding row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistantRowJoin {
    /// A real source/Markdown line boundary.
    Hard,
    /// A soft wrap that removed whitespace.
    SoftSpace,
    /// A soft wrap inside one unbroken token.
    SoftNone,
}

/// One rendered row of a completed assistant response, including offscreen rows.
#[derive(Clone, Debug)]
pub struct AssistantResponseRow {
    pub line_idx: usize,
    pub block: usize,
    pub response_row: usize,
    pub text: String,
    /// Screen columns from the transcript origin to the selectable text.
    pub x_off: u16,
    pub join_before: AssistantRowJoin,
}

/// One visible, selectable row of a completed assistant response.
#[derive(Clone, Debug)]
pub struct AssistantRowHit {
    pub block: usize,
    pub response_row: usize,
    pub screen_row: u16,
    /// First screen column of response content (the outer gutter is excluded).
    pub x: u16,
    pub text: String,
    pub join_before: AssistantRowJoin,
}

/// A display-cell position inside one rendered assistant response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AssistantPoint {
    pub row: usize,
    pub col: usize,
}

/// In-progress mouse selection inside one assistant response.
#[derive(Clone, Debug)]
pub struct AssistantSelection {
    pub block: usize,
    pub anchor: AssistantPoint,
    pub current: AssistantPoint,
    pub dragged: bool,
    /// True only between left-button down and the first matching mouse-up.
    pub active: bool,
}

/// One text-highlight correction on the plan body.
#[derive(Clone, Debug)]
pub struct PlanCorrection {
    /// Inclusive UTF-8 byte start in `Plan.md` body.
    pub start: usize,
    /// Exclusive UTF-8 byte end.
    pub end: usize,
    pub excerpt: String,
    pub note: String,
}

/// In-progress mouse drag selection over the plan body.
#[derive(Clone, Debug)]
pub struct PlanDrag {
    pub anchor: usize,
    pub current: usize,
}

/// Selection / navigation state while `ChatView::Plan` is open.
#[derive(Clone, Default)]
pub struct PlanViewState {
    pub corrections: Vec<PlanCorrection>,
    /// Correction whose note is loaded in the composer.
    pub active: Option<usize>,
    pub drag: Option<PlanDrag>,
    /// First transcript line index of the plan body (after header). Rebuilt each draw.
    pub body_line0: usize,
    /// Per body display-row source spans. Rebuilt each draw.
    pub row_spans: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalViewPhase {
    Attaching,
    UserControl,
    ReadOnly,
    Failed,
}

pub struct TerminalViewState {
    pub phase: TerminalViewPhase,
    pub body_rect: Option<comb::Rect>,
    pub scrollback: usize,
    pub last_size: Option<(u16, u16)>,
}

impl Default for TerminalViewState {
    fn default() -> Self {
        Self {
            phase: TerminalViewPhase::Attaching,
            body_rect: None,
            scrollback: 0,
            last_size: None,
        }
    }
}

/// Right-side plan bar action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlanAction {
    /// No marks — start implementation.
    Make,
    /// Save the note for the current mark (then select more).
    Add,
    /// Send all marks to the agent to revise the plan.
    Send,
}

impl PlanViewState {
    pub fn has_corrections(&self) -> bool {
        !self.corrections.is_empty()
    }

    pub fn drag_range(&self) -> Option<(usize, usize)> {
        let d = self.drag.as_ref()?;
        Some((d.anchor.min(d.current), d.anchor.max(d.current)))
    }
}

/// Inline card announcing the agent switched its own mode (e.g. into PLAN).
#[derive(Clone)]
pub struct ModeSwitchCard {
    pub mode: AgentMode,
    pub reason: String,
}

/// Inline card showing context compaction in progress or completed.
#[derive(Clone)]
pub struct CompactedCard {
    /// None while compacting (spinner), Some(before) when done.
    pub before: Option<u64>,
    pub after: u64,
}

/// Recap of one turn, stored on that turn's "Worked for" card.
#[derive(Clone, Debug, Default)]
pub enum RecapBody {
    #[default]
    Idle,
    Generating {
        text: String,
    },
    Ready {
        text: String,
    },
    Failed {
        error: String,
    },
}

impl RecapBody {
    pub fn is_generating(&self) -> bool {
        matches!(self, Self::Generating { .. })
    }

    pub fn text(&self) -> &str {
        match self {
            Self::Idle => "",
            Self::Generating { text } | Self::Ready { text } => text,
            Self::Failed { error } => error,
        }
    }
}

/// Inline card: "Worked for Nm" summary at the end of a turn.
#[derive(Clone, Default)]
pub struct WorkSummaryCard {
    pub secs: u64,
    pub recap_id: u64,
    pub recap: RecapBody,
}

/// One renderable chunk of the transcript.
pub enum Block {
    /// The greeting shown on a fresh chat.
    Welcome,
    User(String),
    Assistant {
        text: String,
        streaming: bool,
    },
    Reasoning(Thought),
    Tool(ToolCard),
    /// Several independent read-only tools executed in parallel.
    Explore(ExploreCard),
    /// Persistent interactive terminal session.
    Terminal(Box<TerminalCard>),
    /// Subagent status + expandable conversation.
    Subagent(SubagentCard),
    /// Workspace plan card (`.hive/Plan.md`).
    Plan(PlanCard),
    /// The agent switched its working mode with a reason.
    ModeSwitch(ModeSwitchCard),
    /// Context compaction in progress or completed.
    Compacted(CompactedCard),
    /// "Worked for Nm" summary at the end of a turn.
    WorkSummary(WorkSummaryCard),
    /// Agent's task list progress (set_todos tool).
    Todos(Vec<hive_core::TodoItem>),
    Notice(String),
    Error(String),
}
