//! Orchestrator tools: start / observe / message / wait / close subagent jobs.
//! Only the MULTITASK parent sees these. Worktrees stay an implementation detail.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use crate::spawner::{SubagentTask, WaitSpec};
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

use super::str_arg;

fn make_task(
    id: String,
    label: String,
    prompt: String,
    depth: usize,
    isolate: bool,
    cwd: &std::path::Path,
    paths: Vec<String>,
) -> SubagentTask {
    SubagentTask {
        id,
        label,
        prompt,
        model_role: crate::config::ModelRole::Default,
        depth,
        isolate_worktree: isolate,
        cwd: Some(cwd.to_path_buf()),
        paths,
    }
}

fn paths_arg(args: &Value) -> Vec<String> {
    args.get("paths")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
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

fn require_orchestrator(ctx: &ToolContext, name: &str) -> Option<ToolResult> {
    if ctx.depth > 0 {
        return Some(ToolResult::error(format!(
            "{name} is only for the main MULTITASK agent"
        )));
    }
    if !ctx.isolate_worktrees {
        return Some(ToolResult::error(format!(
            "{name} is only available in MULTITASK — switch with Tab"
        )));
    }
    None
}

pub struct SpawnSubagent;

#[async_trait]
impl Tool for SpawnSubagent {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Start a worker job and return its id immediately. The worker runs in the \
background. Pass `paths` for files it will edit so two workers cannot collide. \
Then `agent_wait` (and `agent_close` when you have the result)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "Self-contained brief: goal, files, constraints, what done means."},
                "prompt": {"type": "string", "description": "Alias for task."},
                "paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Files or directories this worker will touch. Overlap with another occupied worker is rejected."
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        if let Some(err) = require_orchestrator(ctx, "spawn_subagent") {
            return err;
        }
        let Some(task) = str_arg(&args, "task")
            .or_else(|| str_arg(&args, "prompt"))
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolResult::error(
                "missing 'task' — write a self-contained prompt for the worker",
            );
        };
        let max_depth = ctx.config.agents.max_depth.max(1);
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
            true,
            &ctx.cwd,
            paths_arg(&args),
        );

        match ctx.spawner.start(st).await {
            Ok(id) => ToolResult::ok(format!(
                "started `{id}`. It runs in the background. Use agent_wait / agent_observe / agent_close."
            )),
            Err(e) => ToolResult::error(e),
        }
    }
}

pub struct AgentObserve;

#[async_trait]
impl Tool for AgentObserve {
    fn name(&self) -> &str {
        "agent_observe"
    }

    fn description(&self) -> &str {
        "Read a worker's status, tool log (name + arguments + result), and last message. \
Does not wait. Use this to steer; do not poll in a tight loop — prefer agent_wait."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Worker id from spawn_subagent."}
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        if let Some(err) = require_orchestrator(ctx, "agent_observe") {
            return err;
        }
        let Some(id) = str_arg(&args, "id")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolResult::error("missing 'id'");
        };
        match ctx.spawner.observe(id).await {
            Ok(snap) => {
                let mut out = format!(
                    "id: `{}`\nstatus: {:?}\npaths: {}\n\n",
                    snap.id,
                    snap.status,
                    if snap.paths.is_empty() {
                        "(none claimed)".into()
                    } else {
                        snap.paths.join(", ")
                    }
                );
                if snap.tools.is_empty() {
                    out.push_str("tools: (none yet)\n");
                } else {
                    out.push_str("tools:\n");
                    for t in &snap.tools {
                        let state = match t.ok {
                            None => "running",
                            Some(true) => "ok",
                            Some(false) => "err",
                        };
                        out.push_str(&format!("- `{name}` [{state}]", name = t.name));
                        if !t.args.is_empty() {
                            out.push_str(&format!(" {}", t.args));
                        }
                        if !t.summary.is_empty() {
                            out.push_str(&format!(" → {}", truncate_label(&t.summary, 80)));
                        }
                        out.push('\n');
                    }
                }
                if !snap.last_text.trim().is_empty() {
                    out.push_str("\nlast:\n");
                    out.push_str(&snap.last_text);
                }
                ToolResult::ok(out)
            }
            Err(e) => ToolResult::error(e),
        }
    }
}

pub struct AgentMessage;

#[async_trait]
impl Tool for AgentMessage {
    fn name(&self) -> &str {
        "agent_message"
    }

    fn description(&self) -> &str {
        "Send a follow-up instruction to a running or idle worker. It continues the same job."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Worker id."},
                "text": {"type": "string", "description": "Message for the worker."}
            },
            "required": ["id", "text"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        if let Some(err) = require_orchestrator(ctx, "agent_message") {
            return err;
        }
        let Some(id) = str_arg(&args, "id")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolResult::error("missing 'id'");
        };
        let Some(text) = str_arg(&args, "text")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolResult::error("missing 'text'");
        };
        match ctx.spawner.message(id, text.to_string()).await {
            Ok(()) => ToolResult::ok(format!("sent to `{id}`")),
            Err(e) => ToolResult::error(e),
        }
    }
}

pub struct AgentWait;

#[async_trait]
impl Tool for AgentWait {
    fn name(&self) -> &str {
        "agent_wait"
    }

    fn description(&self) -> &str {
        "Block until workers finish a turn. `on` is `any` (default), `all`, or a worker id. \
Prefer this over polling agent_observe."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "on": {
                    "type": "string",
                    "description": "`any`, `all`, or a worker id."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Give up after this many seconds (default 180)."
                }
            }
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        if let Some(err) = require_orchestrator(ctx, "agent_wait") {
            return err;
        }
        let on = str_arg(&args, "on").unwrap_or("any");
        let spec = match on {
            "any" | "" => WaitSpec::Any,
            "all" => WaitSpec::All,
            id => WaitSpec::Id(id.to_string()),
        };
        let timeout = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(180);
        match ctx
            .spawner
            .wait(spec, Some(Duration::from_secs(timeout)))
            .await
        {
            Ok(outcomes) => {
                if outcomes.is_empty() {
                    return ToolResult::ok("no workers");
                }
                let mut out = String::new();
                for o in outcomes {
                    out.push_str(&format!(
                        "### `{}` {:?}\n{}\n\n",
                        o.id, o.status, o.last_text
                    ));
                }
                ToolResult::ok(out)
            }
            Err(e) => ToolResult::error(e),
        }
    }
}

pub struct AgentClose;

#[async_trait]
impl Tool for AgentClose {
    fn name(&self) -> &str {
        "agent_close"
    }

    fn description(&self) -> &str {
        "Stop a worker and free its slot. Merges its changes into the main checkout \
unless `discard` is true. Always close finished workers so they do not occupy slots."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Worker id."},
                "discard": {
                    "type": "boolean",
                    "description": "If true, drop the worker's changes instead of merging."
                }
            },
            "required": ["id"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        if let Some(err) = require_orchestrator(ctx, "agent_close") {
            return err;
        }
        let Some(id) = str_arg(&args, "id")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolResult::error("missing 'id'");
        };
        let id = id.strip_prefix("hive/").unwrap_or(id);
        let discard = args
            .get("discard")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        match ctx.spawner.close(id, discard).await {
            Ok(msg) => ToolResult::ok(msg),
            Err(e) => ToolResult::error(e),
        }
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(SpawnSubagent) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(AgentObserve) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(AgentMessage) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(AgentWait) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(AgentClose) as Arc<dyn Tool> } }
