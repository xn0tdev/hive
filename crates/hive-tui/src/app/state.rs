//! Plain data types describing what's currently on screen.

use hive_core::event::{SubagentLine, SubagentStatus};

/// Which conversation the transcript is showing.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum ChatView {
    #[default]
    Main,
    /// Read-only view of a subagent's thread (by card id).
    Subagent(String),
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

/// One renderable chunk of the transcript.
pub enum Block {
    /// The greeting shown on a fresh chat.
    Welcome,
    User(String),
    Assistant { text: String, streaming: bool },
    Reasoning(Thought),
    Tool(ToolCard),
    /// Subagent status + expandable conversation (verify_project, etc.).
    Subagent(SubagentCard),
    Notice(String),
    Error(String),
}
