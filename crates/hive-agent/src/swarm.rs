//! The swarm: runs up to N subagents concurrently, each in its own agent loop,
//! with a depth cap to prevent runaway recursion. Implements `SubagentSpawner`,
//! so tools reach it only through the core trait.
//!
//! It builds subagents from an `AgentBuilder` and hands each one a spawner for
//! nested delegation (until the depth cap), using a `Weak` self-reference so
//! there is no reference cycle.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use hive_core::event::{AgentEvent, EventSender, SubagentStatus};
use hive_core::spawner::{noop_spawner, SubagentOutcome, SubagentSpawner, SubagentTask};

use crate::AgentBuilder;

struct Inner {
    builder: AgentBuilder,
    events: EventSender,
    sem: Arc<Semaphore>,
    max_depth: usize,
    me: Weak<Inner>,
}

/// Build a swarm spawner. Returned as the core trait object so callers stay
/// decoupled from this module's internals.
pub fn new_spawner(
    builder: AgentBuilder,
    events: EventSender,
    max_concurrent: usize,
    max_depth: usize,
) -> Arc<dyn SubagentSpawner> {
    Arc::new_cyclic(|me| Inner {
        builder,
        events,
        sem: Arc::new(Semaphore::new(max_concurrent.max(1))),
        max_depth,
        me: me.clone(),
    })
}

impl Inner {
    async fn run_one(&self, task: SubagentTask) -> SubagentOutcome {
        let _permit = self.sem.acquire().await.ok();

        let id = task.id.clone();
        let label = truncate(&task.prompt, 44);
        let _ = self.events.send(AgentEvent::SubagentSpawned {
            id: id.clone(),
            label,
        });

        // Allow nested delegation until the depth cap; then hand subagents a
        // spawner that refuses to go deeper.
        let child_spawner: Arc<dyn SubagentSpawner> = if task.depth < self.max_depth {
            match self.me.upgrade() {
                Some(inner) => inner,
                None => noop_spawner(),
            }
        } else {
            noop_spawner()
        };

        // Subagent events are discarded; the swarm surfaces status via the
        // SubagentStatus events above/below instead.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        drop(rx);

        let model = self.builder.config.model(task.model_role).to_string();
        let mut agent = self.builder.build(tx, model, task.depth, child_spawner);
        let result = agent.run_headless(task.prompt).await;

        let ok = !result.trim().is_empty();
        let _ = self.events.send(AgentEvent::SubagentStatus {
            id: id.clone(),
            status: if ok {
                SubagentStatus::Done
            } else {
                SubagentStatus::Failed
            },
            detail: truncate(&result, 60),
        });

        SubagentOutcome {
            id,
            result: Ok(result),
        }
    }
}

#[async_trait]
impl SubagentSpawner for Inner {
    async fn spawn(&self, task: SubagentTask) -> SubagentOutcome {
        self.run_one(task).await
    }

    async fn spawn_many(&self, tasks: Vec<SubagentTask>) -> Vec<SubagentOutcome> {
        let mut set: JoinSet<SubagentOutcome> = JoinSet::new();
        for task in tasks {
            let me = self.me.upgrade();
            set.spawn(async move {
                match me {
                    Some(inner) => inner.run_one(task).await,
                    None => SubagentOutcome {
                        id: task.id,
                        result: Err("swarm no longer available".to_string()),
                    },
                }
            });
        }

        let mut out = Vec::new();
        while let Some(joined) = set.join_next().await {
            if let Ok(outcome) = joined {
                out.push(outcome);
            }
        }
        out
    }
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}
