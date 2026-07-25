use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::terminal::{TerminalError, TerminalKey, TerminalWriteRequest};
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

use super::{bool_arg, str_arg, u64_arg};

pub struct TerminalStart;
pub struct TerminalRead;
pub struct TerminalWrite;
pub struct TerminalStop;

fn manager(ctx: &ToolContext) -> Result<&crate::TerminalHandle, TerminalError> {
    if ctx.depth > 0 {
        return Err(TerminalError::Unavailable);
    }
    ctx.terminal.as_ref().ok_or(TerminalError::Unavailable)
}

fn required_string<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolResult> {
    str_arg(args, key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ToolResult::error(format!("missing '{key}'")))
}

fn serialized(value: &impl serde::Serialize) -> ToolResult {
    match serde_json::to_string_pretty(value) {
        Ok(value) => ToolResult::ok(value),
        Err(error) => ToolResult::error(format!("cannot serialize terminal result: {error}")),
    }
}

#[async_trait]
impl Tool for TerminalStart {
    fn name(&self) -> &str {
        "terminal_start"
    }

    fn description(&self) -> &str {
        "Start one persistent interactive command in a pseudo-terminal. Use this \
when a CLI requires prompts or terminal behavior that `run_shell` cannot provide."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to run in the interactive terminal."
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let command = match required_string(&args, "command") {
            Ok(command) => command,
            Err(result) => return result,
        };
        let manager = match manager(ctx) {
            Ok(manager) => manager,
            Err(error) => return ToolResult::error(error.to_string()),
        };
        match manager.start(command, &ctx.cwd).await {
            Ok(snapshot) => serialized(&snapshot),
            Err(error) => ToolResult::error(error.to_string()),
        }
    }
}

#[async_trait]
impl Tool for TerminalRead {
    fn name(&self) -> &str {
        "terminal_read"
    }

    fn description(&self) -> &str {
        "Read the current interactive terminal screen and printable output since \
an optional revision. Can briefly wait for a newer revision."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Terminal session id returned by terminal_start."
                },
                "after_revision": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Return printable output newer than this revision."
                },
                "wait_ms": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": crate::terminal::MAX_READ_WAIT_MS,
                    "description": "Wait up to this many milliseconds for a newer revision."
                }
            },
            "required": ["session_id"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let id = match required_string(&args, "session_id") {
            Ok(id) => id,
            Err(result) => return result,
        };
        let manager = match manager(ctx) {
            Ok(manager) => manager,
            Err(error) => return ToolResult::error(error.to_string()),
        };
        match manager
            .read(
                id,
                u64_arg(&args, "after_revision"),
                u64_arg(&args, "wait_ms"),
            )
            .await
        {
            Ok(result) => serialized(&result),
            Err(error) => ToolResult::error(error.to_string()),
        }
    }
}

#[async_trait]
impl Tool for TerminalWrite {
    fn name(&self) -> &str {
        "terminal_write"
    }

    fn description(&self) -> &str {
        "Write text or a named key to an agent-controlled interactive terminal. \
Set `submit` to append Enter after the text or key."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Terminal session id returned by terminal_start."
                },
                "text": {
                    "type": "string",
                    "description": "Literal text to type."
                },
                "key": {
                    "type": "string",
                    "enum": [
                        "enter", "tab", "escape", "up", "down", "left", "right",
                        "backspace", "ctrl_c", "ctrl_d"
                    ],
                    "description": "Optional named terminal key."
                },
                "submit": {
                    "type": "boolean",
                    "description": "Append Enter after text/key."
                }
            },
            "required": ["session_id"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let id = match required_string(&args, "session_id") {
            Ok(id) => id,
            Err(result) => return result,
        };
        let key = match str_arg(&args, "key") {
            Some(name) => match TerminalKey::parse(name) {
                Some(key) => Some(key),
                None => {
                    return ToolResult::error(format!(
                        "invalid 'key': {name} — expected enter, tab, escape, up, down, \
left, right, backspace, ctrl_c, or ctrl_d"
                    ));
                }
            },
            None => None,
        };
        let manager = match manager(ctx) {
            Ok(manager) => manager,
            Err(error) => return ToolResult::error(error.to_string()),
        };
        let request = TerminalWriteRequest {
            text: str_arg(&args, "text").map(str::to_string),
            key,
            submit: bool_arg(&args, "submit"),
        };
        match manager.write_agent(id, request).await {
            Ok(snapshot) => serialized(&snapshot),
            Err(error) => ToolResult::error(error.to_string()),
        }
    }
}

#[async_trait]
impl Tool for TerminalStop {
    fn name(&self) -> &str {
        "terminal_stop"
    }

    fn description(&self) -> &str {
        "Stop the running interactive terminal session and its descendant processes."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Terminal session id returned by terminal_start."
                }
            },
            "required": ["session_id"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let id = match required_string(&args, "session_id") {
            Ok(id) => id,
            Err(result) => return result,
        };
        let manager = match manager(ctx) {
            Ok(manager) => manager,
            Err(error) => return ToolResult::error(error.to_string()),
        };
        let manager = manager.clone();
        let id = id.to_string();
        let stopped = tokio::task::spawn_blocking(move || manager.stop(&id)).await;
        match stopped {
            Ok(Ok(snapshot)) => serialized(&snapshot),
            Ok(Err(error)) => ToolResult::error(error.to_string()),
            Err(error) => ToolResult::error(format!("terminal stop task failed: {error}")),
        }
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(TerminalStart) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(TerminalRead) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(TerminalWrite) as Arc<dyn Tool> } }
inventory::submit! { ToolRegistration { make: || Arc::new(TerminalStop) as Arc<dyn Tool> } }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{no_skills, noop_spawner, AppConfig, TerminalHandle, ToolContext};
    #[cfg(unix)]
    use crate::TerminalManager;
    use serde_json::json;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn context(terminal: Option<TerminalHandle>) -> ToolContext {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            cwd: std::env::temp_dir(),
            events,
            spawner: noop_spawner(),
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
            terminal,
            vision: false,
            depth: 0,
            call_id: "terminal-test".into(),
            isolate_worktrees: false,
            interrupt: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn terminal_tools_expose_stable_names_and_key_schema() {
        assert_eq!(TerminalStart.name(), "terminal_start");
        assert_eq!(TerminalRead.name(), "terminal_read");
        assert_eq!(TerminalWrite.name(), "terminal_write");
        assert_eq!(TerminalStop.name(), "terminal_stop");
        assert!(TerminalWrite.parameters()["properties"]["key"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "ctrl_c"));
    }

    #[tokio::test]
    async fn terminal_tools_report_unavailable_without_manager() {
        let result = TerminalStart
            .execute(json!({"command": "printf hi"}), &context(None))
            .await;
        assert!(result.is_error);
        assert!(result.content.contains("unavailable"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_tools_start_read_and_stop_a_session() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let manager = TerminalManager::new(events);
        let ctx = context(Some(manager));
        let started = TerminalStart
            .execute(json!({"command": "printf hi; sleep 30"}), &ctx)
            .await;
        assert!(!started.is_error, "{}", started.content);
        let value: serde_json::Value = serde_json::from_str(&started.content).unwrap();
        let id = value["id"].as_str().unwrap();

        let read = TerminalRead
            .execute(
                json!({"session_id": id, "after_revision": 0, "wait_ms": 500}),
                &ctx,
            )
            .await;
        assert!(!read.is_error, "{}", read.content);
        assert!(read.content.contains("hi"));

        let stopped = TerminalStop.execute(json!({"session_id": id}), &ctx).await;
        assert!(!stopped.is_error, "{}", stopped.content);
    }
}
