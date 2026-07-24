//! Dedicated plan writer: `.hive/Plan.md` + one-line card summary.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::{plan_path, PLAN_REL_PATH};
use crate::event::AgentEvent;
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

use super::str_arg;

pub struct WritePlan;

#[async_trait]
impl Tool for WritePlan {
    fn name(&self) -> &str {
        "write_plan"
    }

    fn description(&self) -> &str {
        "Write or replace the workspace plan at `.hive/Plan.md`. Pass a short \
one-line `summary` for the TUI card and the full markdown `content`. Prefer this \
over write_file when creating or revising the plan."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "One-line description shown on the Plan.md card (not the full body)."
                },
                "content": {
                    "type": "string",
                    "description": "Full markdown body for `.hive/Plan.md`."
                }
            },
            "required": ["summary", "content"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(summary) = str_arg(&args, "summary").map(str::trim).filter(|s| !s.is_empty())
        else {
            return ToolResult::error("missing 'summary'");
        };
        let Some(content) = str_arg(&args, "content") else {
            return ToolResult::error("missing 'content'");
        };
        let path = plan_path(&ctx.cwd);
        if let Some(parent) = path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::error(format!("cannot create .hive/: {e}"));
            }
        }
        let body = if content.ends_with('\n') {
            content.to_string()
        } else {
            format!("{content}\n")
        };
        if let Err(e) = tokio::fs::write(&path, &body).await {
            return ToolResult::error(format!("cannot write {}: {e}", path.display()));
        }
        let _ = ctx.events.send(AgentEvent::PlanUpdated {
            summary: summary.trim().to_string(),
            body,
        });
        ToolResult::ok(format!("wrote {PLAN_REL_PATH}"))
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(WritePlan) as Arc<dyn Tool> } }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{no_skills, noop_spawner, AppConfig};
    use std::sync::atomic::AtomicBool;

    #[tokio::test]
    async fn writes_plan_and_emits_update() {
        let dir = std::env::temp_dir().join(format!(
            "hive-write-plan-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ToolContext {
            cwd: dir.clone(),
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
        };
        let r = WritePlan
            .execute(
                json!({"summary": "Fix the bug", "content": "# Fix\n\n## Steps\n"}),
                &ctx,
            )
            .await;
        assert!(!r.is_error, "{}", r.content);
        let body = tokio::fs::read_to_string(dir.join(".hive/Plan.md"))
            .await
            .unwrap();
        assert!(body.contains("# Fix"));
        let ev = rx.try_recv().expect("PlanUpdated");
        match ev {
            AgentEvent::PlanUpdated { summary, .. } => assert_eq!(summary, "Fix the bug"),
            other => panic!("unexpected {other:?}"),
        }
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
