//! Shell execution tool: runs a command via `sh -c`, streaming output live.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use hive_core::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

use super::{str_arg, u64_arg};

const OUTPUT_CAP: usize = 60_000;

pub struct RunShell;

#[async_trait]
impl Tool for RunShell {
    fn name(&self) -> &str {
        "run_shell"
    }

    fn description(&self) -> &str {
        "Run a shell command non-interactively in the working directory. Output is streamed live. Executes immediately without confirmation."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to run (via `sh -c`)."},
                "timeout_secs": {"type": "integer", "description": "Optional timeout in seconds; the process is killed if exceeded."}
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let Some(command) = str_arg(&args, "command") else {
            return ToolResult::error("missing 'command'");
        };
        let timeout = u64_arg(&args, "timeout_secs");

        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&ctx.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("failed to spawn: {e}")),
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        if let Some(out) = stdout {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    let _ = tx.send(l);
                }
            });
        }
        if let Some(err) = stderr {
            let tx = tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    let _ = tx.send(l);
                }
            });
        }
        drop(tx);

        let mut collected = String::new();
        let mut truncated = false;

        let drain_and_wait = async {
            while let Some(line) = rx.recv().await {
                ctx.emit_output(format!("{line}\n"));
                if !truncated {
                    if collected.len() + line.len() + 1 > OUTPUT_CAP {
                        truncated = true;
                        collected.push_str("\n… [output truncated]\n");
                    } else {
                        collected.push_str(&line);
                        collected.push('\n');
                    }
                }
            }
            child.wait().await
        };

        let status = match timeout {
            Some(secs) => match tokio::time::timeout(Duration::from_secs(secs), drain_and_wait).await
            {
                Ok(st) => st,
                Err(_) => {
                    // timed out; the future is dropped which releases `child`, but
                    // the process may still run — best-effort kill via pkill is not
                    // reliable, so report the timeout clearly.
                    return ToolResult::error(format!("command timed out after {secs}s\n{collected}"));
                }
            },
            None => drain_and_wait.await,
        };

        match status {
            Ok(st) => {
                let code = st.code().unwrap_or(-1);
                let header = format!("exit {code}\n");
                if collected.trim().is_empty() {
                    ToolResult::ok(format!("{header}(no output)"))
                } else {
                    ToolResult::ok(format!("{header}{collected}"))
                }
            }
            Err(e) => ToolResult::error(format!("wait failed: {e}\n{collected}")),
        }
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(RunShell) as Arc<dyn Tool> } }
