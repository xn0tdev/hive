use async_trait::async_trait;
use std::sync::Arc;

use crate::config::ModelRole;

/// A unit of work handed to a subagent.
#[derive(Debug, Clone)]
pub struct SubagentTask {
    pub id: String,
    /// Short UI label (e.g. "Checking project") shown in the transcript card.
    pub label: String,
    pub prompt: String,
    pub model_role: ModelRole,
    /// Recursion depth of the agent that will run this task.
    pub depth: usize,
}

/// Result of running a subagent to completion.
#[derive(Debug, Clone)]
pub struct SubagentOutcome {
    pub id: String,
    pub result: std::result::Result<String, String>,
}

/// Spawns subagents. The real implementation (`hive-swarm`) caps concurrency and
/// depth; tools only ever see this trait.
#[async_trait]
pub trait SubagentSpawner: Send + Sync {
    async fn spawn(&self, task: SubagentTask) -> SubagentOutcome;
    async fn spawn_many(&self, tasks: Vec<SubagentTask>) -> Vec<SubagentOutcome>;
}

/// Spawner used before the swarm is wired: every spawn fails loudly.
pub struct NoopSpawner;

#[async_trait]
impl SubagentSpawner for NoopSpawner {
    async fn spawn(&self, task: SubagentTask) -> SubagentOutcome {
        SubagentOutcome {
            id: task.id,
            result: Err("subagents are not enabled".to_string()),
        }
    }
    async fn spawn_many(&self, tasks: Vec<SubagentTask>) -> Vec<SubagentOutcome> {
        tasks
            .into_iter()
            .map(|t| SubagentOutcome {
                id: t.id,
                result: Err("subagents are not enabled".to_string()),
            })
            .collect()
    }
}

pub fn noop_spawner() -> Arc<dyn SubagentSpawner> {
    Arc::new(NoopSpawner)
}
