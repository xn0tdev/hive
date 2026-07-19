//! Delegation tools: hand work to subagents through the injected
//! `SubagentSpawner`. The main agent authors each subagent's prompt;
//! `verify_project` is a ready checker role, `spawn_subagent` is general.
//! `spawn_swarm` + worktrees are for MULTITASK. `integrate_worktree` merges a worker.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::config::ModelRole;
use crate::spawner::SubagentTask;
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};
use crate::worktree;

use super::str_arg;

const VERIFY_LABEL: &str = "Checking project";

/// Short role preamble for the ready checker; the main agent authors the task.
const VERIFY_ROLE: &str = "\
You are a project verification subagent. Prefer reporting over editing. \
Only apply a tiny fix if it unblocks a check and is obviously correct. \
Your final message is the entire return value.";

/// Combine the ready-checker role with the orchestrator-authored brief.
pub(crate) fn build_verify_prompt(task: &str) -> String {
    format!("{VERIFY_ROLE}\n\n## Your task\n{}", task.trim())
}

fn make_task(
    id: String,
    label: String,
    prompt: String,
    depth: usize,
    isolate: bool,
) -> SubagentTask {
    SubagentTask {
        id,
        label,
        prompt,
        model_role: ModelRole::Default,
        depth,
        isolate_worktree: isolate,
        cwd: None,
    }
}

/// Spawn the dedicated project-verification subagent (cargo check / review).
pub struct VerifyProject;

#[async_trait]
impl Tool for VerifyProject {
    fn name(&self) -> &str {
        "verify_project"
    }

    fn description(&self) -> &str {
        "Spawn the ready project-verification subagent (fast model). You write the full \
prompt/brief yourself — what to check, which crates, and what to report. The subagent \
runs independently and returns a concise report. Use when you want an independent \
build/lint/review pass."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Self-contained brief you author for the checker (what to run, scope, what to report)."
                },
                "task": {
                    "type": "string",
                    "description": "Alias for prompt."
                }
            },
            "required": ["prompt"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        // Only the main agent may launch the checker — no nested verify loops.
        if ctx.depth > 0 {
            return ToolResult::error("verify_project is only available to the main agent");
        }
        let max_depth = ctx.config.swarm.max_depth;
        if ctx.depth >= max_depth {
            return ToolResult::error(format!(
                "maximum subagent depth ({max_depth}) reached; cannot spawn deeper"
            ));
        }

        let Some(task) = str_arg(&args, "prompt")
            .or_else(|| str_arg(&args, "task"))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolResult::error(
                "missing 'prompt' — write a self-contained brief for the checker subagent",
            );
        };

        let st = make_task(
            format!("verify_{}", uuid::Uuid::new_v4().simple()),
            VERIFY_LABEL.to_string(),
            build_verify_prompt(task),
            ctx.depth + 1,
            false,
        );

        let outcome = ctx.spawner.spawn(st).await;
        match outcome.result {
            Ok(s) => ToolResult::ok(s),
            Err(e) => ToolResult::error(e),
        }
    }
}

pub struct SpawnSubagent;

#[async_trait]
impl Tool for SpawnSubagent {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Spawn one subagent to complete a focused task end-to-end and return its result. \
You write the full task prompt yourself with all needed context. In MULTITASK the worker \
runs in an isolated git worktree."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "A self-contained task description with all needed context — you author this prompt."},
                "prompt": {"type": "string", "description": "Alias for task."}
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(task) = str_arg(&args, "task")
            .or_else(|| str_arg(&args, "prompt"))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolResult::error(
                "missing 'task' — write a self-contained prompt for the subagent",
            );
        };
        let max_depth = ctx.config.swarm.max_depth;
        if ctx.depth >= max_depth {
            return ToolResult::error(format!(
                "maximum subagent depth ({max_depth}) reached; cannot spawn deeper"
            ));
        }

        let id = format!("sub_{}", uuid::Uuid::new_v4().simple());
        let st = make_task(
            id,
            truncate_label(task, 44),
            task.to_string(),
            ctx.depth + 1,
            ctx.isolate_worktrees,
        );

        let outcome = ctx.spawner.spawn(st).await;
        match outcome.result {
            Ok(s) => ToolResult::ok(s),
            Err(e) => ToolResult::error(e),
        }
    }
}

pub struct SpawnSwarm;

#[async_trait]
impl Tool for SpawnSwarm {
    fn name(&self) -> &str {
        "spawn_swarm"
    }

    fn description(&self) -> &str {
        "Spawn many subagents at once to work in parallel (MULTITASK). Each task runs in an \
isolated git worktree and returns its result plus a branch id for `integrate_worktree`. \
Great for fan-out work across independent features."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "List of tasks. Each item is a task string, or an object {task}.",
                    "items": {}
                }
            },
            "required": ["tasks"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        if !ctx.isolate_worktrees {
            return ToolResult::error(
                "spawn_swarm is only available in MULTITASK mode — switch with Tab",
            );
        }
        let Some(arr) = args.get("tasks").and_then(|v| v.as_array()) else {
            return ToolResult::error("missing 'tasks' array");
        };
        let max_depth = ctx.config.swarm.max_depth;
        if ctx.depth >= max_depth {
            return ToolResult::error(format!(
                "maximum subagent depth ({max_depth}) reached; cannot spawn deeper"
            ));
        }

        let mut tasks = Vec::new();
        for item in arr {
            let prompt = if let Some(s) = item.as_str() {
                s.to_string()
            } else {
                item.get("task")
                    .or_else(|| item.get("prompt"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            if prompt.trim().is_empty() {
                continue;
            }
            let id = format!("sub_{}", uuid::Uuid::new_v4().simple());
            tasks.push(make_task(
                id,
                truncate_label(&prompt, 44),
                prompt,
                ctx.depth + 1,
                true,
            ));
        }

        if tasks.is_empty() {
            return ToolResult::error("no valid tasks provided");
        }

        let outcomes = ctx.spawner.spawn_many(tasks).await;
        let mut out = String::new();
        for (i, o) in outcomes.iter().enumerate() {
            out.push_str(&format!("### Subagent {}\n", i + 1));
            out.push_str(&format!("id: `{}`\n\n", o.id));
            match &o.result {
                Ok(s) => out.push_str(s),
                Err(e) => out.push_str(&format!("(failed: {e})")),
            }
            out.push_str("\n\n");
        }
        ToolResult::ok(out)
    }
}

/// Merge a Multitask worker branch into the main checkout.
pub struct IntegrateWorktree;

#[async_trait]
impl Tool for IntegrateWorktree {
    fn name(&self) -> &str {
        "integrate_worktree"
    }

    fn description(&self) -> &str {
        "Merge a MULTITASK worker's git branch into the main checkout and remove its worktree. \
Pass the worker `id` from spawn_swarm / spawn_subagent results (e.g. `sub_…`)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Worker id from the spawn result (e.g. sub_abc123)."
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        if ctx.depth > 0 {
            return ToolResult::error("integrate_worktree is only for the main MULTITASK agent");
        }
        let Some(id) = str_arg(&args, "id").map(str::trim).filter(|s| !s.is_empty()) else {
            return ToolResult::error("missing 'id'");
        };
        // Allow ids with or without hive/ prefix confusion — strip accidental branch prefix.
        let id = id.strip_prefix("hive/").unwrap_or(id);
        match worktree::integrate(&ctx.cwd, id) {
            Ok(msg) => ToolResult::ok(msg),
            Err(e) => ToolResult::error(e),
        }
    }
}

fn truncate_label(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(VerifyProject) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(SpawnSubagent) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(SpawnSwarm) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(IntegrateWorktree) as Arc<dyn Tool> } }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_prompt_includes_orchestrator_brief() {
        let p = build_verify_prompt("Run cargo check -p hive-core and report failures.");
        assert!(p.contains("project verification subagent"));
        assert!(p.contains("## Your task"));
        assert!(p.contains("Run cargo check -p hive-core"));
    }

    #[test]
    fn verify_prompt_trims_task() {
        let p = build_verify_prompt("  check hive-tui  ");
        assert!(p.ends_with("check hive-tui"));
    }
}
