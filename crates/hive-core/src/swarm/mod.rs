//! The swarm: runs up to N subagents concurrently, each in its own agent loop,
//! with a depth cap to prevent runaway recursion. Implements `SubagentSpawner`,
//! so tools reach it only through the core trait.
//!
//! It builds subagents from an `AgentBuilder` and hands each one a spawner for
//! nested delegation (until the depth cap), using a `Weak` self-reference so
//! there is no reference cycle.

use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::event::{AgentEvent, EventSender, SubagentLine, SubagentStatus};
use crate::spawner::{noop_spawner, SubagentOutcome, SubagentSpawner, SubagentTask};
use crate::worktree;

use crate::agent::AgentBuilder;

/// How long to wait for more reasoning tokens before flushing a coalesced
/// Thinking line. Keeps the subagent UI live without flooding the TUI.
const THINK_COALESCE_MS: u64 = 24;

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
        // Reasoning deltas are coalesced so token-by-token streams don't flood
        // the UI event loop.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let fwd = self.events.clone();
        let fwd_id = id.clone();
        let forward = tokio::spawn(async move {
            forward_child_events(rx, fwd, fwd_id).await;
        });

        let model = self.builder.config.model(task.model_role).to_string();
        let main_cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

        let worktree_meta = if task.isolate_worktree {
            match worktree::create(&main_cwd, &id) {
                Ok((path, branch)) => Some((path, branch)),
                Err(e) => {
                    let _ = self.events.send(AgentEvent::SubagentStatus {
                        id: id.clone(),
                        status: SubagentStatus::Failed,
                        detail: "worktree failed".into(),
                    });
                    let _ = forward.await;
                    return SubagentOutcome { id, result: Err(e) };
                }
            }
        } else {
            None
        };

        let child_cwd = worktree_meta
            .as_ref()
            .map(|(p, _)| p.clone())
            .or(task.cwd.clone())
            .unwrap_or_else(|| main_cwd.clone());

        let result = {
            let mut agent =
                self.builder
                    .build_in(tx, model, task.depth, child_spawner, child_cwd.clone());
            agent.run_headless(task.prompt).await
        }; // agent (and its event sender) dropped → forwarder exits
        let _ = forward.await;

        let mut report = result;
        if let Some((ref wt, ref branch)) = worktree_meta {
            let summary = worktree::summarize(wt);
            report = format!(
                "{report}\n\n---\nworktree: {}\nbranch: `{branch}`\nid: `{id}`\n\n{summary}\n\n\
Use `integrate_worktree` with id `{id}` to merge this branch into the main checkout.",
                wt.display()
            );
        }

        let ok = !report.trim().is_empty();
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
            result: Ok(report),
        }
    }
}

async fn forward_child_events(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
    events: EventSender,
    id: String,
) {
    let mut pending_think = String::new();

    while let Some(ev) = rx.recv().await {
        let mut next = Some(ev);
        while let Some(ev) = next.take() {
            match ev {
                AgentEvent::ReasoningDelta(t) if !t.is_empty() => {
                    pending_think.push_str(&t);
                    // Pull already-queued tokens, then wait briefly for more.
                    if let Some(other) = drain_thinking(&mut rx, &mut pending_think) {
                        flush_thinking(&events, &id, &mut pending_think);
                        next = Some(other);
                        continue;
                    }
                    tokio::time::sleep(Duration::from_millis(THINK_COALESCE_MS)).await;
                    if let Some(other) = drain_thinking(&mut rx, &mut pending_think) {
                        flush_thinking(&events, &id, &mut pending_think);
                        next = Some(other);
                        continue;
                    }
                    flush_thinking(&events, &id, &mut pending_think);
                }
                other => {
                    flush_thinking(&events, &id, &mut pending_think);
                    forward_child(&events, &id, other);
                }
            }
        }
    }
    flush_thinking(&events, &id, &mut pending_think);
}

/// Append all immediately available `ReasoningDelta`s. Returns `Some` when a
/// non-thinking event was dequeued (caller must forward it after flushing).
fn drain_thinking(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
    pending: &mut String,
) -> Option<AgentEvent> {
    loop {
        match rx.try_recv() {
            Ok(AgentEvent::ReasoningDelta(t)) => {
                if !t.is_empty() {
                    pending.push_str(&t);
                }
            }
            Ok(other) => return Some(other),
            Err(_) => return None,
        }
    }
}

fn flush_thinking(events: &EventSender, id: &str, pending: &mut String) {
    if pending.is_empty() {
        return;
    }
    let _ = events.send(AgentEvent::SubagentTranscript {
        id: id.to_string(),
        line: SubagentLine::Thinking(std::mem::take(pending)),
    });
}

fn forward_child(events: &EventSender, id: &str, ev: AgentEvent) {
    match ev {
        AgentEvent::ToolStarted {
            name, args_preview, ..
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
            name, ok, summary, ..
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
        AgentEvent::Usage(usage) => {
            let _ = events.send(AgentEvent::SubagentUsage {
                id: id.to_string(),
                usage,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn drain_thinking_coalesces_queued_deltas() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _ = tx.send(AgentEvent::ReasoningDelta("hel".into()));
        let _ = tx.send(AgentEvent::ReasoningDelta("lo".into()));
        let _ = tx.send(AgentEvent::ReasoningDelta("!".into()));
        let _ = tx.send(AgentEvent::Notice("done".into()));

        let mut pending = String::new();
        let other = drain_thinking(&mut rx, &mut pending);
        assert_eq!(pending, "hello!");
        assert!(matches!(other, Some(AgentEvent::Notice(t)) if t == "done"));
    }

    #[tokio::test]
    async fn forward_loop_coalesces_reasoning_burst() {
        let (child_tx, child_rx) = mpsc::unbounded_channel();
        let (ui_tx, mut ui_rx) = mpsc::unbounded_channel();

        let fwd = tokio::spawn(async move {
            forward_child_events(child_rx, ui_tx, "s1".into()).await;
        });

        for part in ["a", "b", "c", "d"] {
            let _ = child_tx.send(AgentEvent::ReasoningDelta(part.into()));
        }
        drop(child_tx);
        let _ = fwd.await;

        let mut thinking = Vec::new();
        while let Ok(ev) = ui_rx.try_recv() {
            if let AgentEvent::SubagentTranscript {
                line: SubagentLine::Thinking(t),
                ..
            } = ev
            {
                thinking.push(t);
            }
        }
        // One coalesced line (or at most a couple if timing splits), never 4.
        assert!(
            thinking.len() <= 2,
            "expected coalesced thinking, got {thinking:?}"
        );
        let joined: String = thinking.concat();
        assert_eq!(joined, "abcd");
    }

    #[tokio::test]
    async fn forward_loop_rewrites_usage_to_subagent() {
        use crate::provider::Usage;

        let (child_tx, child_rx) = mpsc::unbounded_channel();
        let (ui_tx, mut ui_rx) = mpsc::unbounded_channel();

        let fwd = tokio::spawn(async move {
            forward_child_events(child_rx, ui_tx, "s1".into()).await;
        });

        let _ = child_tx.send(AgentEvent::Usage(Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        }));
        drop(child_tx);
        let _ = fwd.await;

        let mut saw = false;
        while let Ok(ev) = ui_rx.try_recv() {
            if let AgentEvent::SubagentUsage { id, usage } = ev {
                assert_eq!(id, "s1");
                assert_eq!(usage.total_tokens, 15);
                saw = true;
            }
        }
        assert!(saw, "expected SubagentUsage event");
    }
}
