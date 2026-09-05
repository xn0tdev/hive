//! Minimal stdio MCP client: JSON-RPC 2.0 over newline-delimited stdio.
//!
//! One [`McpServer`] wraps one spawned server process. The handshake
//! (`initialize` → `notifications/initialized`) happens in [`connect`]; tool
//! discovery and calls go through [`McpServer::list_tools`] and
//! [`McpServer::call_tool`]. Requests are serialized (one in flight at a
//! time), which is plenty for the agent loop and needs no pending-id table.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{mpsc, Mutex};
use tokio::time::timeout;

use crate::config::McpServerConfig;
use crate::error::{CoreError, Result};

/// Handshake deadline — servers should come up fast.
const INIT_TIMEOUT: Duration = Duration::from_secs(15);
/// Fallback per-request timeout when the config doesn't set one.
const DEFAULT_TIMEOUT: u64 = 60;

/// One advertised tool from `tools/list`.
#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments object.
    pub input_schema: Value,
}

/// Text result of a `tools/call`.
#[derive(Debug, Clone)]
pub struct McpToolOutput {
    pub text: String,
    pub is_error: bool,
}

/// A running MCP server process.
pub struct McpServer {
    name: String,
    stdin: Mutex<ChildStdin>,
    /// Raw inbound lines (responses and notifications interleaved).
    incoming: mpsc::UnboundedReceiver<String>,
    /// Serializes whole request → matching-response cycles.
    request_lock: Mutex<()>,
    next_id: Mutex<u64>,
    timeout: Duration,
    child: Mutex<Child>,
}

impl McpServer {
    /// Server id from `[mcp_servers.<id>]`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Send one request and wait for the response with the matching id.
    async fn request(&self, method: &str, params: Value, deadline: Duration) -> Result<Value> {
        let _guard = self.request_lock.lock().await;
        let id = {
            let mut next = self.next_id.lock().await;
            *next += 1;
            *next
        };
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        {
            let mut w = self.stdin.lock().await;
            w.write_all(msg.to_string().as_bytes())
                .await
                .and_then(|_| w.write_all(b"\n").await)
                .and_then(|_| w.flush().await)
                .map_err(|e| CoreError::Other(format!("mcp `{}`: {e}", self.name)))?;
        }

        loop {
            let line = timeout(deadline, self.incoming.recv())
                .await
                .map_err(|_| {
                    CoreError::Other(format!(
                        "mcp `{}`: `{method}` timed out after {}s",
                        self.name,
                        deadline.as_secs()
                    ))
                })?
                .ok_or_else(|| {
                    CoreError::Other(format!("mcp `{}`: server closed the pipe", self.name))
                })?;

            let msg: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue, // not JSON — skip the line
            };
            // Notifications carry no id; responses to stale requests don't match.
            if msg.get("id").and_then(|i| i.as_u64()) == Some(id) {
                if let Some(err) = msg.get("error") {
                    return Err(CoreError::Other(format!(
                        "mcp `{}`: `{method}` failed: {err}",
                        self.name
                    )));
                }
                return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }

    fn notify(&self, method: &str) {
        let msg = json!({"jsonrpc": "2.0", "method": method});
        let stdin = self.stdin.clone();
        tokio::spawn(async move {
            let mut w = stdin.lock().await;
            let _ = w.write_all(msg.to_string().as_bytes()).await;
            let _ = w.write_all(b"\n").await;
            let _ = w.flush().await;
        });
    }

    /// Advertised tools. An empty result is valid (server with no tools).
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>> {
        let result = self
            .request("tools/list", json!({}), self.timeout)
            .await?;
        Ok(parse_tools(&result))
    }

    /// Call a tool; text content parts are joined with newlines.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<McpToolOutput> {
        let result = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
                self.timeout,
            )
            .await?;
        Ok(parse_tool_output(&result))
    }

    /// Ask the process to shut down politely; kills it if it stalls.
    pub async fn shutdown(&self) {
        let _ = self.request("shutdown", json!({}), Duration::from_secs(3)).await;
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
    }
}

/// Spawn a server, run the `initialize` handshake, return the client.
pub async fn connect(name: &str, cfg: &McpServerConfig) -> Result<Arc<McpServer>> {
    if cfg.command.trim().is_empty() {
        return Err(CoreError::Other(format!(
            "mcp `{name}`: no command configured"
        )));
    }
    let mut cmd = tokio::process::Command::new(&cfg.command);
    cmd.args(&cfg.args)
        .envs(clear_env(&cfg.env))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| CoreError::Other(format!("mcp `{name}`: spawn `{}`: {e}", cfg.command)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CoreError::Other(format!("mcp `{name}`: no stdout")))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| CoreError::Other(format!("mcp `{name}`: no stdin")))?;

    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let timeout_secs = if cfg.timeout_secs == 0 {
        DEFAULT_TIMEOUT
    } else {
        cfg.timeout_secs
    };
    let server = Arc::new(McpServer {
        name: name.to_string(),
        stdin: Mutex::new(stdin),
        incoming: rx,
        request_lock: Mutex::new(()),
        next_id: Mutex::new(0),
        timeout: Duration::from_secs(timeout_secs),
        child: Mutex::new(child),
    });

    server
        .request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "hive", "version": env!("CARGO_PKG_VERSION") }
            }),
            INIT_TIMEOUT,
        )
        .await?;
    server.notify("notifications/initialized");
    Ok(server)
}

/// Only apply explicitly configured env vars on top of the inherited ones.
fn clear_env(env: &HashMap<String, String>) -> Vec<(String, String)> {
    env.iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn parse_tools(result: &Value) -> Vec<McpToolInfo> {
    let empty = Vec::new();
    result
        .get("tools")
        .and_then(|t| t.as_array())
        .unwrap_or(&empty)
        .iter()
        .filter_map(|t| {
            let name = t.get("name")?.as_str()?.to_string();
            let description = t
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            let input_schema = t
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object"}));
            Some(McpToolInfo {
                name,
                description,
                input_schema,
            })
        })
        .collect()
}

fn parse_tool_output(result: &Value) -> McpToolOutput {
    let is_error = result
        .get("isError")
        .and_then(|e| e.as_bool())
        .unwrap_or(false);
    let mut parts: Vec<String> = Vec::new();
    if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
        for item in content {
            if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    parts.push(text.to_string());
                }
            }
        }
    }
    McpToolOutput {
        text: if parts.is_empty() {
            String::new()
        } else {
            parts.join("\n")
        },
        is_error,
    }
}

/// `[mcp_servers.<id>]` entries with no command are ignored by the wiring.
pub fn is_configured(cfg: &McpServerConfig) -> bool {
    !cfg.command.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tools_skips_nameless_entries() {
        let result = json!({
            "tools": [
                {"name": "echo", "description": "Echo text", "inputSchema": {"type": "object"}},
                {"description": "no name"},
                {"name": "bare"}
            ]
        });
        let tools = parse_tools(&result);
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(tools[0].description, "Echo text");
        assert_eq!(tools[1].input_schema["type"], "object");
    }

    #[test]
    fn parse_tool_output_joins_text_parts() {
        let result = json!({
            "content": [
                {"type": "text", "text": "line one"},
                {"type": "image", "data": "..."},
                {"type": "text", "text": "line two"}
            ],
            "isError": true
        });
        let out = parse_tool_output(&result);
        assert_eq!(out.text, "line one\nline two");
        assert!(out.is_error);
    }

    #[test]
    fn empty_result_yields_empty_output() {
        let out = parse_tool_output(&Value::Null);
        assert!(out.text.is_empty());
        assert!(!out.is_error);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn end_to_end_with_fake_server() {
        let script = r#"
read init_req
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"fake","version":"0"}}}'
read initialized
read list_req
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}}]}}'
read call_req
printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"pong"}]}}'
read shutdown_req
printf '%s\n' '{"jsonrpc":"2.0","id":4,"result":{}}'
"#;
        let cfg = McpServerConfig {
            command: "sh".into(),
            args: vec!["-c".into(), script.into()],
            env: HashMap::new(),
            timeout_secs: 10,
        };
        let server = connect("fake", &cfg).await.unwrap();
        let tools = server.list_tools().await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");

        let out = server
            .call_tool("echo", json!({"text": "ping"}))
            .await
            .unwrap();
        assert_eq!(out.text, "pong");
        assert!(!out.is_error);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn missing_command_is_rejected() {
        let cfg = McpServerConfig::default();
        assert!(connect("broken", &cfg).await.is_err());
    }
}
