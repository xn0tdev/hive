//! Delegation tools: hand work to subagents through the injected
//! `SubagentSpawner`. The main agent authors each subagent's prompt;
//! `verify_project` is a ready checker role, `spawn_subagent` is general.
//! Fan-out `spawn_swarm` stays registered but filtered out of `all_tools()`.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::config::ModelRole;
use crate::spawner::SubagentTask;
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

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

        let st = SubagentTask {
            id: format!("verify_{}", uuid::Uuid::new_v4().simple()),
            label: VERIFY_LABEL.to_string(),
            prompt: build_verify_prompt(task),
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
You write the full task prompt yourself with all needed context. \
Pick model_role: fast (commits/merges/simple), smart (backend/deep), default (frontend), vision (images)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "A self-contained task description with all needed context — you author this prompt."},
                "prompt": {"type": "string", "description": "Alias for task."},
                "model_role": {"type": "string", "description": "default | smart | fast | vision"}
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
                    .or_else(|| item.get("prompt"))
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
