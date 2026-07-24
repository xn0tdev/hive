//! Plain data types describing what's currently on screen.

use hive_core::event::{SubagentLine, SubagentStatus};
use hive_core::provider::Usage;
use hive_core::{AgentMode, TerminalController, TerminalProcessState};

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Ok,
    Err,
}

pub struct ToolCard {
    pub id: String,
    pub name: String,
    pub args: String,
    pub output: String,
    pub status: ToolStatus,
    pub started: std::time::Instant,
    /// Set when the tool finishes; `None` while still running.
    pub elapsed_ms: Option<u128>,
}

impl ToolCard {
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
    pub controller: TerminalController,
    pub process: TerminalProcessState,
    pub revision: u64,
    pub screen: vt100::Screen,
    pub started: std::time::Instant,
    pub elapsed_ms: Option<u128>,
}

impl TerminalCard {
    pub fn preview(&self) -> String {
        let contents = self.screen.contents();
        let mut lines = contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .rev()
            .take(2)
            .collect::<Vec<_>>();
        lines.reverse();
        lines.join(" · ")
    }

    pub fn secs(&self) -> f64 {
        match self.elapsed_ms {
            Some(ms) => ms as f64 / 1000.0,
            None => self.started.elapsed().as_millis() as f64 / 1000.0,
        }
    }

    pub fn status_text(&self) -> String {
        let controller = match self.controller {
            TerminalController::Agent => "AGENT",
            TerminalController::User => "USER",
        };
        let process = match &self.process {
            TerminalProcessState::Running => "running".to_string(),
            TerminalProcessState::Exited { code } => format!("exited {code}"),
            TerminalProcessState::Failed { message } => format!("failed: {message}"),
        };
        format!("{controller} · {process}")
    }
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

/// Inline transcript card for a subagent (e.g. `verify_project`).
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
            SubagentStatus::Running => "cargo check · review",
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
    pub started: std::time::Instant,
    pub elapsed_ms: Option<u128>,
    /// True once the plan has been rewritten over an existing body — drives the
    /// `UPDATED` corner badge on the card so a revision is visible at a glance.
    pub revised: bool,
}

impl PlanCard {
    #[allow(dead_code)] // tracked for writing state; card UI no longer shows duration
    pub fn secs(&self) -> f64 {
        match self.elapsed_ms {
            Some(ms) => ms as f64 / 1000.0,
            None => self.started.elapsed().as_millis() as f64 / 1000.0,
        }
    }
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
    /// Persistent interactive terminal session.
    Terminal(Box<TerminalCard>),
    /// Subagent status + expandable conversation (verify_project, etc.).
    Subagent(SubagentCard),
    /// Workspace plan card (`.hive/Plan.md`).
    Plan(PlanCard),
    /// The agent switched its working mode with a reason.
    ModeSwitch(ModeSwitchCard),
    Notice(String),
    Error(String),
}
