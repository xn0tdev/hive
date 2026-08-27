//! Subagent jobs: start, observe, message, wait, close.
//!
//! Tools only see this trait. The runtime lives in `crate::swarm`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::ModelRole;
use crate::event::SubagentStatus;

/// A unit of work handed to a subagent.
#[derive(Debug, Clone)]
pub struct SubagentTask {
    pub id: String,
    /// Short UI label shown in the transcript card.
    pub label: String,
    pub prompt: String,
    pub model_role: ModelRole,
    /// Recursion depth of the agent that will run this task.
    pub depth: usize,
    /// When true, run inside an isolated git worktree (MULTITASK).
    pub isolate_worktree: bool,
    /// Optional fixed cwd (usually the parent's workspace).
    pub cwd: Option<PathBuf>,
    /// Claimed file paths used to reject overlapping writers.
    pub paths: Vec<String>,
}

/// One tool call recorded for `observe`.
#[derive(Debug, Clone)]
pub struct ToolRecord {
    pub name: String,
    pub args: String,
    pub summary: String,
    pub ok: Option<bool>,
}

/// Snapshot of a live or finished job.
#[derive(Debug, Clone)]
pub struct JobSnapshot {
    pub id: String,
    pub label: String,
    pub status: SubagentStatus,
    pub last_text: String,
    pub tools: Vec<ToolRecord>,
    pub paths: Vec<String>,
}

/// How `wait` decides it is done.
#[derive(Debug, Clone)]
pub enum WaitSpec {
    /// First job that is no longer running (already-done jobs count).
    Any,
    /// Every occupied slot is idle.
    All,
    /// One id is idle.
    Id(String),
}

/// One finished (or failed) job returned from `wait`.
#[derive(Debug, Clone)]
pub struct WaitOutcome {
    pub id: String,
    pub status: SubagentStatus,
    pub last_text: String,
}

/// Wakes the parent driver after a child turn ends while the parent is idle.
#[derive(Debug, Clone)]
pub struct JobWake {
    pub id: String,
    pub ok: bool,
    pub summary: String,
}

pub type JobWakeSender = UnboundedSender<JobWake>;

/// Registry of background subagent jobs.
#[async_trait]
pub trait SubagentSpawner: Send + Sync {
    /// Start a job and return its id without waiting for the child to finish.
    async fn start(&self, task: SubagentTask) -> std::result::Result<String, String>;
    async fn observe(&self, id: &str) -> std::result::Result<JobSnapshot, String>;
    async fn message(&self, id: &str, text: String) -> std::result::Result<(), String>;
    async fn wait(
        &self,
        spec: WaitSpec,
        timeout: Option<Duration>,
    ) -> std::result::Result<Vec<WaitOutcome>, String>;
    /// Interrupt if running, merge (unless `discard`), drop the slot.
    async fn close(&self, id: &str, discard: bool) -> std::result::Result<String, String>;
    /// Interrupt every job. Used on hive exit.
    fn shutdown(&self);
}

/// Spawner used in tests: every call fails loudly.
pub struct NoopSpawner;

#[async_trait]
impl SubagentSpawner for NoopSpawner {
    async fn start(&self, _task: SubagentTask) -> std::result::Result<String, String> {
        Err("subagents are not enabled".into())
    }
    async fn observe(&self, _id: &str) -> std::result::Result<JobSnapshot, String> {
        Err("subagents are not enabled".into())
    }
    async fn message(&self, _id: &str, _text: String) -> std::result::Result<(), String> {
        Err("subagents are not enabled".into())
    }
    async fn wait(
        &self,
        _spec: WaitSpec,
        _timeout: Option<Duration>,
    ) -> std::result::Result<Vec<WaitOutcome>, String> {
        Err("subagents are not enabled".into())
    }
    async fn close(&self, _id: &str, _discard: bool) -> std::result::Result<String, String> {
        Err("subagents are not enabled".into())
    }
    fn shutdown(&self) {}
}

pub fn noop_spawner() -> Arc<dyn SubagentSpawner> {
    Arc::new(NoopSpawner)
}
