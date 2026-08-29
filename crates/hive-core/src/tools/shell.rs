//! Shell execution tool: runs a command via the platform shell
//! (`sh -c` on Unix, `cmd.exe /C` on Windows), streaming output live.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::io;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::tool::{Tool, ToolContext, ToolRegistration, ToolResult};

use super::{str_arg, u64_arg};

const OUTPUT_CAP: usize = 60_000;

fn collect_line(ctx: &ToolContext, collected: &mut String, truncated: &mut bool, line: String) {
    ctx.emit_output(format!("{line}\n"));
    if !*truncated {
        if collected.len() + line.len() + 1 > OUTPUT_CAP {
            *truncated = true;
            collected.push_str("\n… [output truncated]\n");
        } else {
            collected.push_str(&line);
            collected.push('\n');
        }
    }
}

async fn terminate(child: &mut Child) -> io::Result<()> {
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            if pid > 0 {
                // The child is its process-group leader, so a negative PID kills
                // the shell and every process it started.
                let rc = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
                if rc != 0 {
                    let error = io::Error::last_os_error();
                    if error.raw_os_error() != Some(libc::ESRCH) {
                        return Err(error);
                    }
                }
            }
        }
        child.wait().await.map(|_| ())
    }

    #[cfg(not(unix))]
    {
        child.kill().await
    }
}

/// Ensures the process group dies if the tool future is cancelled (Esc).
struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.0.id() {
            if pid > 0 {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
        }
        let _ = self.0.start_kill();
    }
}

impl KillOnDrop {
    fn inner(&mut self) -> &mut Child {
        &mut self.0
    }
}

pub struct RunShell;

#[async_trait]
impl Tool for RunShell {
    fn name(&self) -> &str {
        "run_shell"
    }

    fn description(&self) -> &str {
        "Run a shell command non-interactively in the working directory \
(Unix: `sh -c`, Windows: `cmd.exe /C`). Output is streamed live. \
It has no TTY and cannot answer prompts; use `terminal_start` first for sudo, \
passwords, confirmations, interactive CLIs, or long-lived processes. \
Confirm with the user in chat before destructive or irreversible commands \
(force-push, hard reset, deleting data, wiping directories)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The shell command to run (platform shell: sh -c / cmd /C)."},
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

        #[cfg(unix)]
        let mut cmd = {
            let mut c = Command::new("sh");
            c.arg("-c").arg(command);
            // A new *session*, not just a new process group. Without it the
            // child keeps hive's controlling terminal, and anything that
            // prompts through /dev/tty — sudo, ssh, gpg — writes its prompt
            // straight over the TUI (stdio redirection doesn't stop it) and
            // then blocks forever on input nobody can see. Detached, those
            // tools fail immediately with "no tty present", which the agent
            // can act on. setsid also makes the child a group leader, so the
            // timeout still kills the whole tree with one killpg.
            unsafe {
                c.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            c
        };
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd.exe");
            c.args(["/C", command]);
            c
        };
        cmd.current_dir(&ctx.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return ToolResult::error(format!("failed to spawn: {e}")),
        };
        let mut child = KillOnDrop(child);

        let stdout = child.inner().stdout.take();
        let stderr = child.inner().stderr.take();
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
        let interrupt = ctx.interrupt.clone();

        let drain_and_wait = async {
            loop {
                tokio::select! {
                    biased;
                    _ = async {
                        while !interrupt.load(Ordering::Relaxed) {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    } => {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "interrupted"));
                    }
                    line = rx.recv() => {
                        match line {
                            Some(line) => collect_line(ctx, &mut collected, &mut truncated, line),
                            None => break,
                        }
                    }
                }
            }
            child.inner().wait().await
        };

        let status = match timeout {
            Some(secs) => {
                match tokio::time::timeout(Duration::from_secs(secs), drain_and_wait).await {
                    Ok(st) => st,
                    Err(_) => {
                        let kill_error = terminate(child.inner()).await.err();
                        while let Some(line) = rx.recv().await {
                            collect_line(ctx, &mut collected, &mut truncated, line);
                        }
                        let mut message = format!("command timed out after {secs}s\n{collected}");
                        if let Some(error) = kill_error {
                            message.push_str(&format!("\nfailed to terminate command: {error}"));
                        }
                        return ToolResult::error(message);
                    }
                }
            }
            None => drain_and_wait.await,
        };

        match status {
            Ok(st) => {
                let code = st.code().unwrap_or(-1);
                let header = format!("exit {code}\n");
                if collected.trim().is_empty() {
                    let content = format!("{header}(no output)");
                    if st.success() {
                        ToolResult::ok(content)
                    } else {
                        ToolResult::error(content)
                    }
                } else {
                    let content = format!("{header}{collected}");
                    if st.success() {
                        ToolResult::ok(content)
                    } else {
                        ToolResult::error(content)
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                let _ = terminate(child.inner()).await;
                while let Some(line) = rx.recv().await {
                    collect_line(ctx, &mut collected, &mut truncated, line);
                }
                ToolResult::error(format!("interrupted\n{collected}"))
            }
            Err(e) => ToolResult::error(format!("wait failed: {e}\n{collected}")),
        }
    }
}

inventory::submit! { ToolRegistration { make: || Arc::new(RunShell) as Arc<dyn Tool> } }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{no_skills, noop_spawner, AppConfig};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    fn context(cwd: PathBuf) -> ToolContext {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        ToolContext {
            cwd,
            events,
            spawner: noop_spawner(),
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
            terminal: None,
            vision: false,
            depth: 0,
            call_id: "test".into(),
            isolate_worktrees: false,
            interrupt: Arc::new(AtomicBool::new(false)),
        }
    }

    #[tokio::test]
    async fn nonzero_exit_is_an_error() {
        let command = if cfg!(windows) {
            "exit 7"
        } else {
            "printf nope; exit 7"
        };
        let result = RunShell
            .execute(json!({"command": command}), &context(PathBuf::from(".")))
            .await;

        assert!(result.is_error);
        assert!(result.content.starts_with("exit 7"), "{}", result.content);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_descendant_processes() {
        let marker = std::env::temp_dir().join(format!(
            "hive-shell-timeout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let command = format!("(sleep 2; printf leaked > '{}') & wait", marker.display());
        let result = RunShell
            .execute(
                json!({"command": command, "timeout_secs": 1}),
                &context(PathBuf::from(".")),
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.starts_with("command timed out after 1s"));
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!marker.exists(), "descendant survived the timeout");
    }

    #[cfg(unix)]
    /// A command must not be able to reach hive's controlling terminal: that's
    /// how a `sudo` prompt ended up painted over the TUI while the tool sat
    /// blocked on a password until its timeout.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn commands_run_in_their_own_session() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        let ctx = ToolContext {
            cwd: PathBuf::from("."),
            events,
            spawner: noop_spawner(),
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
            terminal: None,
            vision: false,
            depth: 0,
            call_id: "test".into(),
            isolate_worktrees: false,
            interrupt,
        };

        // Field 6 of /proc/self/stat is the session id.
        let result = RunShell
            .execute(json!({"command": "awk '{print $6}' /proc/self/stat"}), &ctx)
            .await;
        assert!(!result.is_error, "{}", result.content);

        let child_sid: i32 = result
            .content
            .trim()
            .lines()
            .next_back()
            .and_then(|l| l.trim().parse().ok())
            .unwrap_or_else(|| panic!("no session id in {:?}", result.content));
        let own_sid = unsafe { libc::getsid(0) };
        assert_ne!(
            child_sid, own_sid,
            "the child kept hive's session, so /dev/tty still reaches the TUI"
        );
    }

    #[tokio::test]
    async fn interrupt_kills_running_command() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        let ctx = ToolContext {
            cwd: PathBuf::from("."),
            events,
            spawner: noop_spawner(),
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
            terminal: None,
            vision: false,
            depth: 0,
            call_id: "test".into(),
            isolate_worktrees: false,
            interrupt: interrupt.clone(),
        };
        let flag = interrupt.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            flag.store(true, Ordering::Relaxed);
        });
        let result = RunShell.execute(json!({"command": "sleep 30"}), &ctx).await;
        assert!(result.is_error);
        assert!(result.content.contains("interrupted"), "{}", result.content);
    }
}
