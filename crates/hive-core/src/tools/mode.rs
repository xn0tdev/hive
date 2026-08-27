//! `switch_mode`: let the agent move itself into PLAN before a large feature.
//!
//! The tool only validates the request and reports back to the model; the agent
//! loop (`run_tool`) applies the actual mode change and emits `ModeSwitched` so
//! the frontend can update its chip and drop a "Switched to … Mode" card.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::agent::AgentMode;
use crate::tool::{Tool, ToolContext, ToolResult};

use super::str_arg;

pub struct SwitchMode;

#[async_trait]
impl Tool for SwitchMode {
    fn name(&self) -> &str {
        "switch_mode"
    }

    fn description(&self) -> &str {
        "Switch your own working mode when the task calls for it. Use this when a \
request turns out to be a large, multi-part, or architecturally significant \
feature that deserves an agreed plan before any code changes: call \
`switch_mode` with `mode: \"plan\"` and a short `reason`, then write the plan \
with `write_plan`. Prefer this over diving straight into a sprawling change. \
For small, clear tasks just implement — do not switch. `plan` is planning only \
(no project edits); `make` is the full coding agent; `multitask` orchestrates \
background worker jobs."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "enum": ["plan", "make", "multitask"],
                    "description": "Mode to switch into. Usually \"plan\" for a big feature."
                },
                "reason": {
                    "type": "string",
                    "description": "In your own words, one or two short sentences on why the switch is needed. Shown verbatim to the user on the mode card, so write it naturally — do not use a canned template."
                }
            },
            "required": ["mode", "reason"]
        })
    }

    async fn execute(&self, args: Value, _ctx: &ToolContext) -> ToolResult {
        let Some(mode) = str_arg(&args, "mode").and_then(AgentMode::parse) else {
            return ToolResult::error("invalid 'mode' — expected one of: plan, make, multitask");
        };
        let reason = str_arg(&args, "reason").map(str::trim).unwrap_or("");
        if reason.is_empty() {
            return ToolResult::error("missing 'reason' — say briefly why the switch is needed");
        }
        // The agent loop performs the real switch after this returns ok. Keep the
        // result neutral: no canned next-step prose — the mode's own guidance and
        // the agent's own reason drive what happens next.
        ToolResult::ok(format!("Switched to {} mode.", mode.label()))
    }
}

inventory::submit! { crate::tool::ToolRegistration { make: || Arc::new(SwitchMode) as Arc<dyn Tool> } }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{no_skills, noop_spawner, AppConfig};
    use std::sync::atomic::AtomicBool;

    fn ctx() -> ToolContext {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            cwd: std::env::temp_dir(),
            events: tx,
            spawner: noop_spawner(),
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
            terminal: None,
            vision: false,
            depth: 0,
            call_id: "t".into(),
            isolate_worktrees: false,
            interrupt: Arc::new(AtomicBool::new(false)),
        }
    }

    #[tokio::test]
    async fn accepts_plan_with_reason() {
        let r = SwitchMode
            .execute(json!({"mode": "plan", "reason": "big feature"}), &ctx())
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert!(r.content.contains("PLAN"));
    }

    #[tokio::test]
    async fn exposes_make_as_the_full_coding_mode() {
        assert_eq!(
            SwitchMode.parameters()["properties"]["mode"]["enum"],
            json!(["plan", "make", "multitask"])
        );

        let r = SwitchMode
            .execute(json!({"mode": "make", "reason": "implement now"}), &ctx())
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert!(r.content.contains("MAKE"), "{}", r.content);
    }

    #[tokio::test]
    async fn rejects_bad_mode_and_empty_reason() {
        let bad = SwitchMode
            .execute(json!({"mode": "zoom", "reason": "x"}), &ctx())
            .await;
        assert!(bad.is_error);
        let empty = SwitchMode
            .execute(json!({"mode": "plan", "reason": "  "}), &ctx())
            .await;
        assert!(empty.is_error);
    }
}
