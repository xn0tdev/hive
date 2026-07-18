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

use crate::event::{AgentEvent, EventSender, SubagentLine, SubagentStatus};
use crate::spawner::{noop_spawner, SubagentOutcome, SubagentSpawner, SubagentTask};

use crate::agent::AgentBuilder;

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
        let label = if task.label.trim().is_empty() {
            truncate(&task.prompt, 44)
        } else {
            task.label.clone()
        };
        let _ = self.events.send(AgentEvent::SubagentSpawned {
            id: id.clone(),
            label,
            prompt: task.prompt.clone(),
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

        // Forward selected child events so the TUI can show the subagent
        // conversation and a live status line under the card header.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let fwd = self.events.clone();
        let fwd_id = id.clone();
        let forward = tokio::spawn(async move {
            while let Some(ev) = rx.recv().await {
                forward_child(&fwd, &fwd_id, ev);
            }
        });

        let model = self.builder.config.model(task.model_role).to_string();
        let result = {
            let mut agent = self.builder.build(tx, model, task.depth, child_spawner);
            agent.run_headless(task.prompt).await
        }; // agent (and its event sender) dropped → forwarder exits
        let _ = forward.await;

        let ok = !result.trim().is_empty();
        let _ = self.events.send(AgentEvent::SubagentStatus {
            id: id.clone(),
            status: if ok {
                SubagentStatus::Done
            } else {
                SubagentStatus::Failed
            },
            // Short status only — the full report lives in the transcript.
            detail: if ok { "done".into() } else { "failed".into() },
        });

        SubagentOutcome {
            id,
            result: Ok(result),
        }
    }
}

fn forward_child(events: &EventSender, id: &str, ev: AgentEvent) {
    match ev {
        AgentEvent::ToolStarted {
            name,
            args_preview,
            ..
        } => {
            let detail = tool_detail(&name, &args_preview);
            let _ = events.send(AgentEvent::SubagentStatus {
                id: id.to_string(),
                status: SubagentStatus::Running,
                detail: detail.clone(),
            });
            let _ = events.send(AgentEvent::SubagentTranscript {
                id: id.to_string(),
                line: SubagentLine::Tool {
                    name,
                    detail: args_preview,
                    ok: None,
                },
            });
        }
        AgentEvent::ToolFinished {
            name,
            ok,
            summary,
            ..
        } => {
            let _ = events.send(AgentEvent::SubagentTranscript {
                id: id.to_string(),
                line: SubagentLine::Tool {
                    name,
                    detail: summary,
                    ok: Some(ok),
                },
            });
        }
        AgentEvent::ReasoningDelta(t) => {
            if t.is_empty() {
                return;
            }
            let _ = events.send(AgentEvent::SubagentTranscript {
                id: id.to_string(),
                line: SubagentLine::Thinking(t),
            });
        }
        AgentEvent::AssistantMessage(t) => {
            if t.trim().is_empty() {
                return;
            }
            let _ = events.send(AgentEvent::SubagentTranscript {
                id: id.to_string(),
                line: SubagentLine::Assistant(t),
            });
        }
        AgentEvent::Notice(t) => {
            let _ = events.send(AgentEvent::SubagentTranscript {
                id: id.to_string(),
                line: SubagentLine::Notice(t),
            });
        }
        _ => {}
    }
}

fn tool_detail(name: &str, args: &str) -> String {
    let s = if args.trim().is_empty() {
        name.to_string()
    } else if name == "run_shell" {
        format!("$ {args}")
    } else {
        format!("{name} · {args}")
    };
    truncate(&s, 56)
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
