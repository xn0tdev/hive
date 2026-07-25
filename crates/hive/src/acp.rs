//! Agent Client Protocol (ACP) v1 server over stdio.
//!
//! JSON-RPC 2.0, newline-delimited, stdin → stdout.
//! Implements: `initialize`, `session/new`, `session/prompt`, `session/cancel`,
//! `session/set_mode`, `$/cancel_request`.
//! Emits `session/update` notifications for agent text, thoughts, tool calls,
//! plan updates, mode changes, and usage.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, Mutex};

use hive_core::event::{AgentEvent, EventReceiver};
use hive_core::{Agent, AgentMode, FollowUpSlot, UserInput, DEFAULT_CONTEXT_WINDOW};

use crate::wire;

/// Maximum time for a single prompt turn before we force-cancel.
const TURN_TIMEOUT: Duration = Duration::from_secs(600);
/// Timeout for a single stdout write (prevents pipe-backpressure deadlock).
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long to wait for the drain task to flush after a turn before giving up.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

pub async fn run(cfg: Arc<hive_core::config::AppConfig>) -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));

    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let interrupt = Arc::new(AtomicBool::new(false));
    let follow_up: FollowUpSlot = Arc::new(std::sync::Mutex::new(None));

    let agent = wire::build_agent(&cfg, event_tx.clone());

    let context_window = effective_context_window(cfg.agent.context_window);

    let state = Arc::new(AcpState {
        agent: Mutex::new(agent),
        session_id: Mutex::new(String::new()),
        interrupt: interrupt.clone(),
        follow_up: follow_up.clone(),
        out: stdout.clone(),
        context_window,
        errored: Arc::new(AtomicBool::new(false)),
        drain_signal: Arc::new(Mutex::new(None)),
    });

    let state_for_drain = state.clone();
    tokio::spawn(drain_events(event_rx, state_for_drain));

    let mut reader = tokio::io::BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = match msg.get("method").and_then(|m| m.as_str()) {
            Some(m) => m.to_string(),
            None => continue,
        };
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        match method.as_str() {
            "initialize" => {
                write_msg(&state.out, &reply(id, initialize_result())).await;
            }
            "session/new" => {
                handle_session_new(id, params, &state).await;
            }
            "session/prompt" => {
                handle_session_prompt(id, params, &state, &mut reader).await;
            }
            "session/set_mode" => {
                handle_set_mode(id, params, &state).await;
            }
            "session/cancel" | "$/cancel_request" => {
                state.interrupt.store(true, Ordering::Relaxed);
            }
            _ => {
                if id.is_some() {
                    write_msg(
                        &state.out,
                        &error_reply(id, -32601, &format!("method not found: {method}")),
                    )
                    .await;
                }
            }
        }
    }

    Ok(())
}

struct AcpState {
    agent: Mutex<Agent>,
    session_id: Mutex<String>,
    interrupt: Arc<AtomicBool>,
    follow_up: FollowUpSlot,
    out: Arc<Mutex<tokio::io::Stdout>>,
    context_window: u64,
    /// Set when an `AgentEvent::Error` is seen during a turn → `refusal` stopReason.
    errored: Arc<AtomicBool>,
    /// One-shot signal fired by the drain task after `TurnFinished` is processed.
    drain_signal: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

fn effective_context_window(configured: u64) -> u64 {
    if configured == 0 {
        DEFAULT_CONTEXT_WINDOW
    } else {
        configured
    }
}

fn extract_prompt_text(params: &Value) -> String {
    params
        .get("prompt")
        .and_then(|p| p.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                        b.get("text").and_then(|t| t.as_str()).map(String::from)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

async fn write_msg(out: &Arc<Mutex<tokio::io::Stdout>>, msg: &str) {
    let write_fut = async {
        let mut w = out.lock().await;
        let _ = w.write_all(msg.as_bytes()).await;
        let _ = w.write_all(b"\n").await;
        let _ = w.flush().await;
    };
    let _ = tokio::time::timeout(WRITE_TIMEOUT, write_fut).await;
}

/// The `modes` object advertised in `initialize` and `session/new` responses.
fn modes_state() -> Value {
    json!({
        "currentModeId": "make",
        "availableModes": [
            {"id": "make", "name": "Make", "description": "Full coding agent"},
            {"id": "plan", "name": "Plan", "description": "Research and plan only — no project edits or shell"},
            {"id": "multitask", "name": "Multitask", "description": "Orchestrate parallel subagents"}
        ]
    })
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": 1,
        "agentCapabilities": {
            "loadSession": false,
            "promptCapabilities": {
                "image": false,
                "audio": false,
                "embeddedContext": false
            }
        },
        "agentInfo": {
            "name": "hive",
            "title": "Hive",
            "version": env!("CARGO_PKG_VERSION")
        },
        "authMethods": [],
        "modes": modes_state()
    })
}

async fn handle_session_new(id: Option<Value>, params: Value, state: &Arc<AcpState>) {
    let cwd = params
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // mcpServers and additionalDirectories are part of the spec but Hive uses
    // its own tool set — parse and ignore.
    if let Some(servers) = params.get("mcpServers").and_then(|v| v.as_array()) {
        if !servers.is_empty() {
            tracing::debug!(
                "session/new: ignoring {} mcpServers (not yet supported)",
                servers.len()
            );
        }
    }

    let session_id = format!("sess_{}", uuid::Uuid::new_v4().simple());

    *state.session_id.lock().await = session_id.clone();

    let mut agent = state.agent.lock().await;
    if let Some(new_agent) = wire::rebuild_agent_in(&agent, &cwd) {
        *agent = new_agent;
    }
    drop(agent);

    write_msg(
        &state.out,
        &reply(
            id,
            json!({ "sessionId": session_id, "modes": modes_state() }),
        ),
    )
    .await;
}

/// Handle `session/set_mode` — switch the agent's working mode.
async fn handle_set_mode(id: Option<Value>, params: Value, state: &Arc<AcpState>) {
    let mode_id = params.get("modeId").and_then(|v| v.as_str()).unwrap_or("");

    let mode = match AgentMode::parse(mode_id) {
        Some(m) => m,
        None => {
            write_msg(
                &state.out,
                &error_reply(id, -32602, &format!("unknown modeId: {mode_id}")),
            )
            .await;
            return;
        }
    };

    let mut agent = state.agent.lock().await;
    agent.set_mode(mode);
    drop(agent);

    write_msg(&state.out, &reply(id, json!({}))).await;
}

/// Handle session/prompt. This is the critical path — we must:
/// 1. Run the agent turn concurrently with stdin reads (so cancel works).
/// 2. After the turn ends, wait for the drain task to flush all events.
/// 3. Return "cancelled" stopReason if interrupted, "refusal" on error.
async fn handle_session_prompt(
    id: Option<Value>,
    params: Value,
    state: &Arc<AcpState>,
    reader: &mut tokio::io::BufReader<tokio::io::Stdin>,
) {
    let prompt_text = extract_prompt_text(&params);
    if prompt_text.is_empty() {
        write_msg(&state.out, &error_reply(id, -32602, "prompt is empty")).await;
        return;
    }

    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    *state.session_id.lock().await = session_id;
    state.interrupt.store(false, Ordering::Relaxed);
    state.errored.store(false, Ordering::Relaxed);
    if let Ok(mut g) = state.follow_up.lock() {
        *g = None;
    }

    // Set up a fresh drain barrier — the drain task fires this after
    // processing TurnFinished.
    let (drain_tx, drain_rx) = oneshot::channel::<()>();
    {
        let mut g = state.drain_signal.lock().await;
        *g = Some(drain_tx);
    }

    let interrupt = state.interrupt.clone();
    let follow_up = state.follow_up.clone();

    // Track whether the turn was cancelled by the client.
    let cancelled = Arc::new(AtomicBool::new(false));

    // Run the turn. We select! between the turn future, stdin reads (for cancel),
    // and a timeout.
    {
        let mut agent = state.agent.lock().await;
        let turn_fut = agent.run_turn(
            UserInput {
                text: prompt_text,
                images: Vec::new(),
                mode: AgentMode::Make,
            },
            interrupt.clone(),
            follow_up,
        );

        tokio::pin!(turn_fut);

        let final_text;
        let mut buf = String::new();

        loop {
            tokio::select! {
                biased;
                text = &mut turn_fut => {
                    final_text = text;
                    break;
                }
                _ = tokio::time::sleep(TURN_TIMEOUT) => {
                    interrupt.store(true, Ordering::Relaxed);
                    // Wait for turn to finish after interrupt.
                    final_text = turn_fut.await;
                    break;
                }
                n = reader.read_line(&mut buf) => {
                    if n.unwrap_or(0) == 0 {
                        // stdin closed — cancel everything.
                        interrupt.store(true, Ordering::Relaxed);
                        final_text = turn_fut.await;
                        break;
                    }
                    let trimmed = buf.trim().to_string();
                    buf.clear();
                    if trimmed.is_empty() { continue; }
                    if let Ok(msg) = serde_json::from_str::<Value>(&trimmed) {
                        let method = msg.get("method").and_then(|m| m.as_str());
                        if method == Some("session/cancel") || method == Some("$/cancel_request") {
                            interrupt.store(true, Ordering::Relaxed);
                            cancelled.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
        }

        // turn_fut is pinned and borrows agent; it's done here so the borrow ends.
        final_text
    };

    // Wait for the drain task to flush all remaining events (TurnFinished).
    // Fall back to a timeout so a missed signal never hangs the response.
    let _ = tokio::time::timeout(DRAIN_TIMEOUT, drain_rx).await;

    // Determine stop reason: refusal > cancelled > end_turn.
    let stop_reason = if state.errored.load(Ordering::Relaxed) {
        "refusal"
    } else if cancelled.load(Ordering::Relaxed) || state.interrupt.load(Ordering::Relaxed) {
        "cancelled"
    } else {
        "end_turn"
    };

    // Reset flags for next turn.
    state.interrupt.store(false, Ordering::Relaxed);
    state.errored.store(false, Ordering::Relaxed);

    write_msg(&state.out, &reply(id, json!({ "stopReason": stop_reason }))).await;
}

async fn drain_events(mut events: EventReceiver, state: Arc<AcpState>) {
    let mut msg_counter: u64 = 0;
    let mut current_msg_id: Option<String> = None;
    // Track file snapshots by tool-call id so we can emit diffs on completion.
    let mut snapshots: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();

    while let Some(event) = events.recv().await {
        // Track errors for refusal stopReason.
        if matches!(event, AgentEvent::Error(_)) {
            state.errored.store(true, Ordering::Relaxed);
        }

        // Fire the drain barrier when the turn is fully done.
        if matches!(event, AgentEvent::TurnFinished) {
            let signal = state.drain_signal.lock().await.take();
            if let Some(tx) = signal {
                let _ = tx.send(());
            }
            // TurnFinished itself doesn't produce a session/update.
            continue;
        }

        // Stash file snapshots for diff emission on ToolFinished.
        if let AgentEvent::FileSnapshot { id, path, content } = &event {
            snapshots.insert(id.clone(), (path.clone(), content.clone()));
            // Don't emit a session/update for the snapshot itself.
            continue;
        }

        // For ToolFinished of file-writing tools, emit a diff if we have a snapshot.
        if let AgentEvent::ToolFinished { id, name, ok, .. } = &event {
            if *ok && is_file_write_tool(name) {
                if let Some((path, old_content)) = snapshots.remove(id) {
                    let new_content = tokio::fs::read_to_string(&path).await.unwrap_or_default();
                    let sid = state.session_id.lock().await.clone();
                    if let Some(note) =
                        tool_diff_update(id, &path, &old_content, &new_content, &sid)
                    {
                        write_msg(&state.out, &note).await;
                    }
                }
            } else {
                snapshots.remove(id);
            }
        }

        let sid = state.session_id.lock().await.clone();
        if let Some(note) = event_to_update(
            &event,
            &sid,
            &mut msg_counter,
            &mut current_msg_id,
            state.context_window,
        ) {
            write_msg(&state.out, &note).await;
        }
    }
}

/// True for tools that modify files and should emit a diff.
fn is_file_write_tool(name: &str) -> bool {
    matches!(name, "write_file" | "edit_file")
}

/// Build a `tool_call_update` notification with a diff content block.
fn tool_diff_update(
    tool_call_id: &str,
    path: &str,
    old_text: &str,
    new_text: &str,
    session_id: &str,
) -> Option<String> {
    let notification = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": tool_call_id,
                "status": "completed",
                "content": [{
                    "type": "diff",
                    "diff": {
                        "path": path,
                        "oldText": old_text,
                        "newText": new_text
                    }
                }]
            }
        }
    });
    serde_json::to_string(&notification).ok()
}

fn event_to_update(
    event: &AgentEvent,
    session_id: &str,
    msg_counter: &mut u64,
    current_msg_id: &mut Option<String>,
    context_window: u64,
) -> Option<String> {
    let update = match event {
        AgentEvent::AssistantTextDelta(text) => {
            let mid = ensure_msg_id(msg_counter, current_msg_id);
            json!({
                "sessionUpdate": "agent_message_chunk",
                "messageId": mid,
                "content": { "type": "text", "text": text }
            })
        }
        // AssistantMessage is the finalized markdown. In the TUI we skip it
        // (deltas already delivered the text), but in ACP we send it as a
        // final chunk so Zed renders the complete formatted message (plan,
        // code blocks, etc.) in the chat.
        AgentEvent::AssistantMessage(text) => {
            let mid = ensure_msg_id(msg_counter, current_msg_id);
            json!({
                "sessionUpdate": "agent_message_chunk",
                "messageId": mid,
                "content": { "type": "text", "text": text }
            })
        }
        AgentEvent::ReasoningDelta(text) => {
            let mid = ensure_msg_id(msg_counter, current_msg_id);
            json!({
                "sessionUpdate": "agent_thought_chunk",
                "messageId": mid,
                "content": { "type": "text", "text": text }
            })
        }
        AgentEvent::ToolStarted {
            id,
            name,
            args_preview,
            ..
        } => {
            // New tool call starts a new message context.
            *current_msg_id = None;
            let title = if args_preview.is_empty() {
                name.to_string()
            } else {
                format!("{name}: {args_preview}")
            };
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": id,
                "title": title,
                "kind": tool_kind(name),
                "status": "pending"
            })
        }
        AgentEvent::ToolOutput { id, chunk } => {
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": "in_progress",
                "content": [{
                    "type": "content",
                    "content": { "type": "text", "text": chunk }
                }]
            })
        }
        AgentEvent::ToolFinished {
            id,
            name,
            ok,
            summary,
            ..
        } => {
            // File-writing tools already got a diff update in drain_events.
            // Skip the text summary for those to avoid duplicate content.
            if is_file_write_tool(name) && *ok {
                return None;
            }
            json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": if *ok { "completed" } else { "failed" },
                "content": [{
                    "type": "content",
                    "content": { "type": "text", "text": summary }
                }]
            })
        }
        AgentEvent::Usage(usage) => json!({
            "sessionUpdate": "usage_update",
            "used": usage.total_tokens,
            "size": context_window
        }),
        AgentEvent::ContextTokens(tokens) => json!({
            "sessionUpdate": "usage_update",
            "used": tokens,
            "size": context_window
        }),
        AgentEvent::Notice(text) => {
            let mid = ensure_msg_id(msg_counter, current_msg_id);
            json!({
                "sessionUpdate": "agent_message_chunk",
                "messageId": mid,
                "content": { "type": "text", "text": text }
            })
        }
        AgentEvent::Error(text) => {
            let mid = ensure_msg_id(msg_counter, current_msg_id);
            json!({
                "sessionUpdate": "agent_message_chunk",
                "messageId": mid,
                "content": { "type": "text", "text": text }
            })
        }
        AgentEvent::ModeSwitched { mode, .. } => json!({
            "sessionUpdate": "current_mode_update",
            "currentModeId": mode_id(mode)
        }),
        AgentEvent::PlanUpdated { body, .. } => {
            let entries = plan_entries(body);
            json!({
                "sessionUpdate": "plan",
                "entries": entries
            })
        }
        // Terminal events, subagent events, model/catalog events — not mapped.
        _ => return None,
    };

    let notification = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": update
        }
    });

    serde_json::to_string(&notification).ok()
}

/// Parse `## ` headings from plan markdown into ACP plan entries.
/// First entry is `in_progress`, rest are `pending`. Priority is `high` for
/// the first, `medium` for the rest.
fn plan_entries(body: &str) -> Vec<Value> {
    let headings: Vec<&str> = body
        .lines()
        .filter_map(|line| {
            let t = line.trim();
            t.strip_prefix("## ")
                .map(str::trim)
                .filter(|s| !s.is_empty())
        })
        .collect();

    headings
        .iter()
        .enumerate()
        .map(|(i, h)| {
            json!({
                "content": h,
                "priority": if i == 0 { "high" } else { "medium" },
                "status": if i == 0 { "in_progress" } else { "pending" }
            })
        })
        .collect()
}

fn ensure_msg_id(counter: &mut u64, current: &mut Option<String>) -> String {
    if current.is_none() {
        *counter += 1;
        *current = Some(format!("msg_{}", *counter));
    }
    current.clone().unwrap_or_default()
}

fn mode_id(mode: &AgentMode) -> &'static str {
    match mode {
        AgentMode::Make => "make",
        AgentMode::Plan => "plan",
        AgentMode::Multitask => "multitask",
    }
}

fn tool_kind(name: &str) -> &'static str {
    match name {
        "read_file" | "list_dir" | "read_skill" => "read",
        "glob" | "grep" => "search",
        "web_search" | "web_get_contents" => "fetch",
        "write_file" | "edit_file" => "edit",
        "delete_path" => "delete",
        "run_shell" | "terminal_start" | "terminal_write" | "terminal_read" | "terminal_stop" => {
            "execute"
        }
        "write_plan" => "think",
        "switch_mode" => "switch_mode",
        _ => "other",
    }
}

fn reply(id: Option<Value>, result: Value) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "result": result
    }))
    .unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"serialize failed"}}"#.into()
    })
}

fn error_reply(id: Option<Value>, code: i32, message: &str) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message }
    }))
    .unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"serialize failed"}}"#.into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_kind_mapping() {
        assert_eq!(tool_kind("read_file"), "read");
        assert_eq!(tool_kind("list_dir"), "read");
        assert_eq!(tool_kind("read_skill"), "read");
        assert_eq!(tool_kind("glob"), "search");
        assert_eq!(tool_kind("grep"), "search");
        assert_eq!(tool_kind("web_search"), "fetch");
        assert_eq!(tool_kind("web_get_contents"), "fetch");
        assert_eq!(tool_kind("write_file"), "edit");
        assert_eq!(tool_kind("edit_file"), "edit");
        assert_eq!(tool_kind("delete_path"), "delete");
        assert_eq!(tool_kind("run_shell"), "execute");
        assert_eq!(tool_kind("terminal_start"), "execute");
        assert_eq!(tool_kind("write_plan"), "think");
        assert_eq!(tool_kind("switch_mode"), "switch_mode");
        assert_eq!(tool_kind("spawn_subagent"), "other");
        assert_eq!(tool_kind("unknown_tool"), "other");
    }

    #[test]
    fn mode_id_mapping() {
        assert_eq!(mode_id(&AgentMode::Make), "make");
        assert_eq!(mode_id(&AgentMode::Plan), "plan");
        assert_eq!(mode_id(&AgentMode::Multitask), "multitask");
    }

    #[test]
    fn plan_entries_parses_headings() {
        let body = "# Plan\n\n## First step\n\nDo things\n\n## Second step\n\nMore things\n";
        let entries = plan_entries(body);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["content"], "First step");
        assert_eq!(entries[0]["status"], "in_progress");
        assert_eq!(entries[0]["priority"], "high");
        assert_eq!(entries[1]["content"], "Second step");
        assert_eq!(entries[1]["status"], "pending");
        assert_eq!(entries[1]["priority"], "medium");
    }

    #[test]
    fn plan_entries_empty_when_no_headings() {
        let body = "Just some text\nwithout headings\n";
        let entries = plan_entries(body);
        assert!(entries.is_empty());
    }

    #[test]
    fn plan_entries_skips_empty_headings() {
        let body = "## \n\n## Real\n";
        let entries = plan_entries(body);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["content"], "Real");
    }

    #[test]
    fn effective_context_window_falls_back() {
        assert_eq!(effective_context_window(0), DEFAULT_CONTEXT_WINDOW);
        assert_eq!(effective_context_window(128_000), 128_000);
    }

    #[test]
    fn modes_state_has_three_modes() {
        let modes = modes_state();
        assert_eq!(modes["currentModeId"], "make");
        let available = modes["availableModes"].as_array().unwrap();
        assert_eq!(available.len(), 3);
        assert_eq!(available[0]["id"], "make");
        assert_eq!(available[1]["id"], "plan");
        assert_eq!(available[2]["id"], "multitask");
    }

    #[test]
    fn initialize_result_includes_modes() {
        let result = initialize_result();
        assert_eq!(result["protocolVersion"], 1);
        assert_eq!(result["agentCapabilities"]["loadSession"], false);
        assert!(result["modes"].is_object());
        assert_eq!(result["modes"]["currentModeId"], "make");
    }

    #[test]
    fn extract_prompt_text_joins_blocks() {
        let params = json!({
            "prompt": [
                {"type": "text", "text": "hello"},
                {"type": "image", "data": "abc"},
                {"type": "text", "text": "world"}
            ]
        });
        assert_eq!(extract_prompt_text(&params), "hello\nworld");
    }

    #[test]
    fn extract_prompt_text_empty_when_no_text() {
        let params = json!({
            "prompt": [
                {"type": "image", "data": "abc"}
            ]
        });
        assert_eq!(extract_prompt_text(&params), "");
    }

    #[test]
    fn extract_prompt_text_empty_when_no_prompt() {
        let params = json!({});
        assert_eq!(extract_prompt_text(&params), "");
    }

    #[test]
    fn is_file_write_tool_recognizes_edit_tools() {
        assert!(is_file_write_tool("write_file"));
        assert!(is_file_write_tool("edit_file"));
        assert!(!is_file_write_tool("read_file"));
        assert!(!is_file_write_tool("run_shell"));
        assert!(!is_file_write_tool("delete_path"));
    }

    #[test]
    fn tool_diff_update_produces_diff_content() {
        let note = tool_diff_update("call_1", "src/main.rs", "old text", "new text", "sess_1");
        let note = note.unwrap();
        let v: Value = serde_json::from_str(&note).unwrap();
        assert_eq!(v["method"], "session/update");
        assert_eq!(v["params"]["update"]["sessionUpdate"], "tool_call_update");
        assert_eq!(v["params"]["update"]["toolCallId"], "call_1");
        assert_eq!(v["params"]["update"]["status"], "completed");
        assert_eq!(v["params"]["update"]["content"][0]["type"], "diff");
        assert_eq!(
            v["params"]["update"]["content"][0]["diff"]["path"],
            "src/main.rs"
        );
        assert_eq!(
            v["params"]["update"]["content"][0]["diff"]["oldText"],
            "old text"
        );
        assert_eq!(
            v["params"]["update"]["content"][0]["diff"]["newText"],
            "new text"
        );
    }
}
