//! Delegation tools: hand work to subagents through the injected
//! `SubagentSpawner`. Swarm fan-out stays registered but filtered out of
//! `all_tools()`; the live surface is a single `verify_project` checker.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::config::ModelRole;
use crate::spawner::SubagentTask;
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

use super::str_arg;

const VERIFY_LABEL: &str = "Checking project";

const VERIFY_PROMPT: &str = "\
You are a project verification subagent. Check that this workspace is in good shape.

Do the following end-to-end:
1. Run `cargo check` in the working directory (add `-p <crate>` only if the task clearly scopes one crate).
2. If check fails, diagnose the top errors; do not make broad refactors.
3. Optionally run a focused `cargo test` only when it is quick and clearly relevant.
4. Finish with a concise report: what you ran, pass/fail, and the most important issues (if any).

Prefer reporting over editing. Only apply a tiny fix if it unblocks the check and is obviously correct.
Your final message is the entire return value.";

/// Spawn the dedicated project-verification subagent (cargo check / review).
pub struct VerifyProject;

#[async_trait]
impl Tool for VerifyProject {
    fn name(&self) -> &str {
        "verify_project"
    }

    fn description(&self) -> &str {
        "Spawn the project verification subagent. It runs cargo check (and a light review) \
and returns a concise pass/fail report. Use when you want an independent build/lint check \
of the workspace. Takes no arguments."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "focus": {
                    "type": "string",
                    "description": "Optional hint (crate name, area, or concern) appended to the checker brief."
                }
            },
            "required": []
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

        let mut prompt = VERIFY_PROMPT.to_string();
        if let Some(focus) = str_arg(&args, "focus").filter(|s| !s.trim().is_empty()) {
            prompt.push_str("\n\nFocus / extra context from the main agent:\n");
            prompt.push_str(focus);
        }

        let st = SubagentTask {
            id: format!("verify_{}", uuid::Uuid::new_v4().simple()),
            label: VERIFY_LABEL.to_string(),
            prompt,
            model_role: ModelRole::Fast,
            depth: ctx.depth + 1,
        };

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
Pick model_role: fast (commits/merges/simple), smart (backend/deep), default (frontend), vision (images)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "A self-contained task description with all needed context."},
                "model_role": {"type": "string", "description": "default | smart | fast | vision"}
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(task) = str_arg(&args, "task") else {
            return ToolResult::error("missing 'task'");
        };
        let max_depth = ctx.config.swarm.max_depth;
        if ctx.depth >= max_depth {
            return ToolResult::error(format!(
                "maximum subagent depth ({max_depth}) reached; cannot spawn deeper"
            ));
        }

        let role = str_arg(&args, "model_role")
            .and_then(ModelRole::parse)
            .unwrap_or(ModelRole::Default);

        let st = SubagentTask {
            id: format!("sub_{}", uuid::Uuid::new_v4().simple()),
            label: truncate_label(task, 44),
            prompt: task.to_string(),
            model_role: role,
            depth: ctx.depth + 1,
        };

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
        "Spawn many subagents at once to work in parallel. Each task runs independently and returns its result. \
Great for fan-out work (e.g. investigate N files, implement N independent pieces)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "description": "List of tasks. Each item is a task string, or an object {task, model_role}.",
                    "items": {}
                },
                "model_role": {"type": "string", "description": "Default role for all tasks: default | smart | fast | vision"}
            },
            "required": ["tasks"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(arr) = args.get("tasks").and_then(|v| v.as_array()) else {
            return ToolResult::error("missing 'tasks' array");
        };
        let max_depth = ctx.config.swarm.max_depth;
        if ctx.depth >= max_depth {
            return ToolResult::error(format!(
                "maximum subagent depth ({max_depth}) reached; cannot spawn deeper"
            ));
        }

        let default_role = str_arg(&args, "model_role")
            .and_then(ModelRole::parse)
            .unwrap_or(ModelRole::Default);

        let mut tasks = Vec::new();
        for item in arr {
            let (prompt, role) = if let Some(s) = item.as_str() {
                (s.to_string(), default_role)
            } else {
                let p = item
                    .get("task")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let r = item
                    .get("model_role")
                    .and_then(|v| v.as_str())
                    .and_then(ModelRole::parse)
                    .unwrap_or(default_role);
                (p, r)
            };
            if prompt.trim().is_empty() {
                continue;
            }
            tasks.push(SubagentTask {
                id: format!("sub_{}", uuid::Uuid::new_v4().simple()),
                label: truncate_label(&prompt, 44),
                prompt,
                model_role: role,
                depth: ctx.depth + 1,
            });
        }

        if tasks.is_empty() {
            return ToolResult::error("no valid tasks provided");
        }

        let outcomes = ctx.spawner.spawn_many(tasks).await;
        let mut out = String::new();
        for (i, o) in outcomes.iter().enumerate() {
            out.push_str(&format!("### Subagent {}\n", i + 1));
            match &o.result {
                Ok(s) => out.push_str(s),
                Err(e) => out.push_str(&format!("(failed: {e})")),
            }
            out.push_str("\n\n");
        }
        ToolResult::ok(out)
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
