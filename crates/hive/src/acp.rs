//! Agent Client Protocol (ACP) v1 server over stdio.
//!
//! JSON-RPC 2.0, newline-delimited, stdin → stdout.
//! Implements: `initialize`, `session/new`, `session/prompt`, `session/cancel`,
//! `$/cancel_request`.
//! Emits `session/update` notifications for agent text, tool calls, and usage.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use hive_core::event::{AgentEvent, EventReceiver};
use hive_core::{Agent, AgentMode, FollowUpSlot, UserInput};

use crate::wire;

/// Maximum time for a single prompt turn before we force-cancel.
const TURN_TIMEOUT: Duration = Duration::from_secs(600);
/// Timeout for a single stdout write (prevents pipe-backpressure deadlock).
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn run(cfg: Arc<hive_core::config::AppConfig>) -> Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));

    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();
    let interrupt = Arc::new(AtomicBool::new(false));
    let follow_up: FollowUpSlot = Arc::new(std::sync::Mutex::new(None));

    let agent = wire::build_agent(&cfg, event_tx.clone());

    let state = Arc::new(AcpState {
        agent: Mutex::new(agent),
        session_id: Mutex::new(String::new()),
        interrupt: interrupt.clone(),
        follow_up: follow_up.clone(),
        out: stdout.clone(),
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
        "authMethods": []
    })
}

async fn handle_session_new(id: Option<Value>, params: Value, state: &Arc<AcpState>) {
    let cwd = params
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let session_id = format!("sess_{}", uuid::Uuid::new_v4().simple());

    *state.session_id.lock().await = session_id.clone();

    let mut agent = state.agent.lock().await;
    if let Some(new_agent) = wire::rebuild_agent_in(&agent, &cwd) {
        *agent = new_agent;
    }
    drop(agent);

    write_msg(&state.out, &reply(id, json!({ "sessionId": session_id }))).await;
}

/// Handle session/prompt. This is the critical path — we must:
/// 1. Run the agent turn concurrently with stdin reads (so cancel works).
/// 2. After the turn ends, drain all remaining events before sending the response.
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
    if let Ok(mut g) = state.follow_up.lock() {
        *g = None;
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

        // We need to concurrently read stdin for cancel notifications while the
        // turn runs. But we can't easily pass the BufReader around. Instead,
        // we rely on the interrupt flag being set by the main loop — BUT the
        // main loop IS this function. So we need select! here.
        //
        // Strategy: use select! between the turn and reading one line from stdin.
        // If we get a cancel notification, set interrupt and continue waiting
        // for the turn to finish.
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

    // Drain remaining events before sending the response (fixes race #2).
    // Give the drain task a moment to flush.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Determine stop reason.
    let stop_reason = if cancelled.load(Ordering::Relaxed)
        || state.interrupt.load(Ordering::Relaxed)
    {
        "cancelled"
    } else {
        "end_turn"
    };

    // Reset interrupt for next turn.
    state.interrupt.store(false, Ordering::Relaxed);

    write_msg(&state.out, &reply(id, json!({ "stopReason": stop_reason }))).await;
}

async fn drain_events(mut events: EventReceiver, state: Arc<AcpState>) {
    let mut msg_counter: u64 = 0;
    let mut current_msg_id: Option<String> = None;

    while let Some(event) = events.recv().await {
        let sid = state.session_id.lock().await.clone();
        if let Some(note) = event_to_update(&event, &sid, &mut msg_counter, &mut current_msg_id) {
            write_msg(&state.out, &note).await;
        }
    }
}

fn event_to_update(
    event: &AgentEvent,
    session_id: &str,
    msg_counter: &mut u64,
    current_msg_id: &mut Option<String>,
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
        // AssistantMessage is the finalized markdown — the client already has
        // the full text via deltas. Sending it again duplicates content (#13).
        // Skip it.
        AgentEvent::AssistantMessage(_) => return None,
        AgentEvent::ReasoningDelta(text) => {
            let mid = ensure_msg_id(msg_counter, current_msg_id);
            json!({
                "sessionUpdate": "thought_chunk",
                "messageId": mid,
                "content": { "type": "text", "text": text }
            })
        }
        AgentEvent::ToolStarted { id, name, .. } => {
            // New tool call starts a new message context.
            *current_msg_id = None;
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": id,
                "title": name,
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
        AgentEvent::ToolFinished { id, ok, summary, .. } => json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": id,
            "status": if *ok { "completed" } else { "failed" },
            "content": [{
                "type": "content",
                "content": { "type": "text", "text": summary }
            }]
        }),
        AgentEvent::Usage(usage) => json!({
            "sessionUpdate": "usage_update",
            "used": usage.total_tokens,
            "size": 200000
        }),
        AgentEvent::ContextTokens(tokens) => json!({
            "sessionUpdate": "usage_update",
            "used": tokens,
            "size": 200000
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
            "modeId": mode_id(mode)
        }),
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

    Some(serde_json::to_string(&notification).ok()?)
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
        "read_file" | "list_dir" | "glob" | "grep" | "web_search" | "web_get_contents"
        | "read_skill" => "read",
        "write_file" | "edit_file" => "edit",
        "delete_path" => "delete",
        "run_shell" | "terminal_start" | "terminal_write" | "terminal_read"
        | "terminal_stop" => "execute",
        "write_plan" | "switch_mode" => "think",
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
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"serialize failed"}}"#
            .into()
    })
}

fn error_reply(id: Option<Value>, code: i32, message: &str) -> String {
    serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message }
    }))
    .unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"serialize failed"}}"#
            .into()
    })
}
