//! Agent task list: `set_todos` lets the agent track its own progress.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::event::{AgentEvent, TodoItem};
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

pub struct SetTodos;

#[async_trait]
impl Tool for SetTodos {
    fn name(&self) -> &str {
        "set_todos"
    }

    fn description(&self) -> &str {
        "Set or update your task list. Pass the full list of tasks each time \
         (replaces the previous list). Each task has a short `text` and a `done` \
         flag. Use this to track multi-step work — set tasks after planning, \
         mark them done as you finish each one. Pass an empty list to clear it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "text": {"type": "string", "description": "Short task description."},
                            "done": {"type": "boolean", "description": "True when the task is complete."}
                        },
                        "required": ["text", "done"]
                    },
                    "description": "The full task list (replaces previous)."
                }
            },
            "required": ["todos"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(todos) = args.get("todos").and_then(|v| v.as_array()) else {
            return ToolResult::error("missing 'todos' array");
        };
        if todos.is_empty() {
            let _ = ctx
                .events
                .send(AgentEvent::TodosUpdated { items: Vec::new() });
            return ToolResult::ok("task list cleared");
        }
        let mut items = Vec::new();
        for entry in todos {
            let Some(text) = entry.get("text").and_then(|v| v.as_str()) else {
                continue;
            };
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            let done = entry.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
            items.push(TodoItem { text, done });
        }
        if items.is_empty() {
            return ToolResult::error("no valid tasks in 'todos'");
        }
        let done_count = items.iter().filter(|t| t.done).count();
        let total = items.len();
        let _ = ctx.events.send(AgentEvent::TodosUpdated {
            items: items.clone(),
        });
        ToolResult::ok(format!("tasks updated: {done_count}/{total} done"))
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(SetTodos) as Arc<dyn Tool> } }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{no_skills, noop_spawner, AppConfig};
    use std::sync::atomic::AtomicBool;

    #[tokio::test]
    async fn sets_todos_and_emits_event() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ToolContext {
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
        };
        let r = SetTodos
            .execute(
                json!({"todos": [
                    {"text": "Read the code", "done": true},
                    {"text": "Fix the bug", "done": false},
                    {"text": "Write tests", "done": false}
                ]}),
                &ctx,
            )
            .await;
        assert!(!r.is_error, "{}", r.content);
        assert!(r.content.contains("1/3"));
        let ev = rx.try_recv().expect("TodosUpdated");
        match ev {
            AgentEvent::TodosUpdated { items } => {
                assert_eq!(items.len(), 3);
                assert!(items[0].done);
                assert!(!items[1].done);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_list_clears_todos() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let ctx = ToolContext {
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
        };

        let result = SetTodos.execute(json!({"todos": []}), &ctx).await;

        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("cleared"));
        assert!(matches!(
            rx.try_recv(),
            Ok(AgentEvent::TodosUpdated { items }) if items.is_empty()
        ));
    }
}
