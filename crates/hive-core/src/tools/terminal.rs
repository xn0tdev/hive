use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::terminal::{
    TerminalError, TerminalInputRequest, TerminalKey, TerminalReadResult, TerminalSnapshot,
    TerminalWriteRequest,
};
use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

use super::{bool_arg, str_arg, u64_arg};

pub struct TerminalStart;
pub struct TerminalRead;
pub struct TerminalWrite;
pub struct TerminalStop;

/// Capture the first screen update in the start call. This removes the usual
/// start → immediate read round trip without making a quiet process feel slow.
const START_CAPTURE_MS: u64 = 250;

const PRIVATE_INPUT_HINT: &str = "Tell the user now, in one short sentence, that the background terminal above needs private input and to open it and enter it there. Then stop. Never ask for or send the secret in chat or terminal_write, and do not poll.";
const CONFIRMATION_HINT: &str = "Read the confirmation prompt. If it is clearly safe and within the user's request, answer it with terminal_write; otherwise ask the user. Never approve an unclear destructive action.";

fn agent_hint(request: Option<&TerminalInputRequest>) -> Option<&'static str> {
    request.map(|request| {
        if request.is_private() {
            PRIVATE_INPUT_HINT
        } else {
            CONFIRMATION_HINT
        }
    })
}

#[derive(serde::Serialize)]
struct TerminalReadOutput<'a> {
    #[serde(flatten)]
    result: &'a TerminalReadResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_hint: Option<&'static str>,
}

#[derive(serde::Serialize)]
struct TerminalStartOutput {
    #[serde(flatten)]
    session: TerminalSnapshot,
    screen: Option<String>,
    output: Option<String>,
    output_truncated: bool,
    input_request: Option<TerminalInputRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_hint: Option<&'static str>,
}

fn serialized_read(result: &TerminalReadResult) -> ToolResult {
    serialized(&TerminalReadOutput {
        result,
        agent_hint: agent_hint(result.input_request.as_ref()),
    })
}

fn serialized_start(result: TerminalReadResult) -> ToolResult {
    let agent_hint = agent_hint(result.input_request.as_ref());
    serialized(&TerminalStartOutput {
        session: result.session,
        screen: result.screen,
        output: result.output,
        output_truncated: result.output_truncated,
        input_request: result.input_request,
        agent_hint,
    })
}

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

fn optional_string(args: &Value, key: &str) -> Option<String> {
    str_arg(args, key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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
        "Start one background interactive command in a pseudo-terminal and capture \
its initial screen. Use this first for sudo/password prompts, confirmations, full-screen \
CLIs, installers, dev servers, watchers, and other long-lived commands that `run_shell` \
cannot handle. Supply the ready-to-run command so the user never has to create a terminal."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to run in the interactive terminal."
                },
                "description": {
                    "type": "string",
                    "description": "Short note on why this terminal is being started (shown in the UI)."
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
        let description = optional_string(&args, "description").unwrap_or_default();
        let manager = match manager(ctx) {
            Ok(manager) => manager,
            Err(error) => return ToolResult::error(error.to_string()),
        };
        match manager.start(command, &description, &ctx.cwd).await {
            Ok(snapshot) => match manager
                .read(
                    &snapshot.id,
                    Some(snapshot.revision),
                    Some(START_CAPTURE_MS),
                )
                .await
            {
                Ok(result) => serialized_start(result),
                Err(error) => ToolResult::error(error.to_string()),
            },
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
an optional revision. Can briefly wait for a newer revision and reports structured \
private-input or confirmation prompts."
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
            Ok(result) => serialized_read(&result),
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
Set `submit` to append Enter after the text or key. The backend refuses passwords, \
passphrases, PINs, and verification codes; the user must enter those privately."
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
    #[cfg(unix)]
    use crate::TerminalManager;
    use crate::{no_skills, noop_spawner, AppConfig, TerminalHandle, ToolContext};
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
        assert!(TerminalStart.description().contains("sudo/password"));
        assert!(TerminalStart
            .description()
            .contains("never has to create a terminal"));
        assert!(TerminalWrite.description().contains("backend refuses"));
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

    #[cfg(unix)]
    #[tokio::test]
    async fn start_captures_private_prompt_and_agent_cannot_fill_it() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let manager = TerminalManager::new(events);
        let ctx = context(Some(manager.clone()));

        let started = TerminalStart
            .execute(json!({"command": "printf 'Password:'; read secret"}), &ctx)
            .await;
        assert!(!started.is_error, "{}", started.content);
        let value: serde_json::Value = serde_json::from_str(&started.content).unwrap();
        let id = value["id"].as_str().unwrap();
        assert_eq!(value["input_request"]["kind"], "private");
        assert!(value["agent_hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("background terminal above")));

        let write = TerminalWrite
            .execute(
                json!({"session_id": id, "text": "guess", "submit": true}),
                &ctx,
            )
            .await;
        assert!(write.is_error);
        assert!(write.content.contains("private input"));

        let stopped = TerminalStop.execute(json!({"session_id": id}), &ctx).await;
        assert!(!stopped.is_error, "{}", stopped.content);
    }
}
