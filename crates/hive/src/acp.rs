//! Agent Client Protocol (ACP) v1 server over stdio.
//!
//! JSON-RPC 2.0, newline-delimited, stdin → stdout.
//! Implements: `initialize`, `session/new`, `session/prompt`, `session/cancel`.
//! Emits `session/update` notifications for agent text, tool calls, and usage.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

use hive_core::event::{AgentEvent, EventReceiver};
use hive_core::{Agent, AgentMode, FollowUpSlot, UserInput};

use crate::wire;

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
                let prompt_text = extract_prompt_text(&params);
                if prompt_text.is_empty() {
                    write_msg(&state.out, &error_reply(id, -32602, "prompt is empty")).await;
                    continue;
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

                let mut agent = state.agent.lock().await;
                agent
                    .run_turn(
                        UserInput {
                            text: prompt_text,
                            images: Vec::new(),
                            mode: AgentMode::Make,
                        },
                        state.interrupt.clone(),
                        state.follow_up.clone(),
                    )
                    .await;
                drop(agent);

                write_msg(&state.out, &reply(id, json!({ "stopReason": "end_turn" }))).await;
            }
            "session/cancel" => {
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
    let mut w = out.lock().await;
    let _ = w.write_all(msg.as_bytes()).await;
    let _ = w.write_all(b"\n").await;
    let _ = w.flush().await;
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

async fn drain_events(mut events: EventReceiver, state: Arc<AcpState>) {
    while let Some(event) = events.recv().await {
        let sid = state.session_id.lock().await.clone();
        if let Some(note) = event_to_update(&event, &sid) {
            write_msg(&state.out, &note).await;
        }
    }
}

fn event_to_update(event: &AgentEvent, session_id: &str) -> Option<String> {
    let update = match event {
        AgentEvent::AssistantTextDelta(text) => json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": text }
        }),
        AgentEvent::AssistantMessage(text) => json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": text }
        }),
        AgentEvent::ReasoningDelta(text) => json!({
            "sessionUpdate": "thought_chunk",
            "content": { "type": "text", "text": text }
        }),
        AgentEvent::ToolStarted { id, name, .. } => json!({
            "sessionUpdate": "tool_call",
            "toolCallId": id,
            "title": name,
            "kind": tool_kind(name),
            "status": "pending"
        }),
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
        AgentEvent::Notice(text) => json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": text }
        }),
        AgentEvent::Error(text) => json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": text }
        }),
        AgentEvent::ModeSwitched { mode, .. } => json!({
            "sessionUpdate": "agent_message_chunk",
            "content": { "type": "text", "text": format!("Switched to {} mode.", mode.label()) }
        }),
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
