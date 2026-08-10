//! Tool dispatch and tool-call safety for the agent loop.

use super::*;

impl Agent {
    pub(super) async fn run_tool(
        &mut self,
        id: &str,
        name: &str,
        arguments: &str,
        interrupt: &Arc<AtomicBool>,
    ) {
        self.emit(AgentEvent::ToolStarted {
            id: id.to_string(),
            name: name.to_string(),
            args_preview: tool_args_preview(name, arguments),
        });

        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(args) => args,
            Err(error) => {
                let result = ToolResult::error(format!("invalid tool arguments: {error}"));
                self.emit(AgentEvent::ToolFinished {
                    id: id.to_string(),
                    name: name.to_string(),
                    ok: false,
                    summary: first_line(&result.content, 120),
                });
                self.session
                    .push(Message::tool_result(id, name, result.content));
                return;
            }
        };
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // `switch_mode` changes the session mode — only the top-level agent may.
        // Capture the target here (before `args` is consumed) and apply it once
        // the tool reports success below.
        let switch_target = if name == "switch_mode" && self.depth == 0 {
            args.get("mode")
                .and_then(|v| v.as_str())
                .and_then(AgentMode::parse)
                .map(|mode| {
                    let reason = args
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    (mode, reason)
                })
        } else {
            None
        };

        let result = match self.validate_tool_call(name, path.as_deref()) {
            Ok(()) => self.execute_tool(name, args, id, interrupt).await,
            Err(message) => ToolResult::error(message),
        };

        self.emit(AgentEvent::ToolFinished {
            id: id.to_string(),
            name: name.to_string(),
            ok: !result.is_error,
            summary: first_line(&result.content, 120),
        });

        // Apply a validated `switch_mode` now: refresh the system prompt for the
        // next round and tell the frontend so it can update the chip + card.
        // Skip the event when it's a no-op (already in that mode) so the UI
        // doesn't show a redundant switch card + toast.
        if !result.is_error {
            if let Some((mode, reason)) = switch_target {
                if self.mode != mode {
                    self.set_mode(mode);
                    self.emit(AgentEvent::ModeSwitched { mode, reason });
                }
            }
        }

        if !result.is_error && matches!(name, "write_file" | "edit_file") {
            if let Some(path) = path.as_deref() {
                if path == PLAN_REL_PATH || super::super::mode::is_plan_path(&self.cwd, path) {
                    self.emit_plan_updated().await;
                }
            }
        }

        self.session
            .push(Message::tool_result(id, name, result.content));

        if !result.images.is_empty() {
            let mut parts = vec![ContentPart::Text(format!(
                "[Image(s) produced by tool `{name}`]"
            ))];
            for img in result.images {
                parts.push(ContentPart::Image(img));
            }
            self.session.push(Message::user_parts(parts));
        }
    }

    pub(super) async fn run_tools_parallel(
        &mut self,
        batch: &[ToolCall],
        interrupt: &Arc<AtomicBool>,
        loop_redirects: &mut Vec<String>,
    ) {
        enum PendingResult {
            Ready {
                result: ToolResult,
                emit_finished: bool,
            },
            Running(tokio::task::JoinHandle<ToolResult>),
        }

        let mut pending = Vec::with_capacity(batch.len());
        for tc in batch {
            if interrupt.load(Ordering::Relaxed) {
                pending.push(PendingResult::Ready {
                    result: ToolResult::error("turn interrupted"),
                    emit_finished: false,
                });
                continue;
            }
            if self.detect_repeated_tool_call(tc, loop_redirects) {
                pending.push(PendingResult::Ready {
                    result: ToolResult::error(
                        "tool call not run: identical call repeated too many times",
                    ),
                    emit_finished: false,
                });
                continue;
            }

            self.emit(AgentEvent::ToolStarted {
                id: tc.id.clone(),
                name: tc.name.clone(),
                args_preview: tool_args_preview(&tc.name, &tc.arguments),
            });

            let args: serde_json::Value = match serde_json::from_str(&tc.arguments) {
                Ok(args) => args,
                Err(error) => {
                    pending.push(PendingResult::Ready {
                        result: ToolResult::error(format!("invalid tool arguments: {error}")),
                        emit_finished: true,
                    });
                    continue;
                }
            };
            let path = args.get("path").and_then(serde_json::Value::as_str);
            if let Err(message) = self.validate_tool_call(&tc.name, path) {
                pending.push(PendingResult::Ready {
                    result: ToolResult::error(message),
                    emit_finished: true,
                });
                continue;
            }
            let tool = self.tools.iter().find(|t| t.name() == tc.name).cloned();
            let name = tc.name.clone();
            let ctx = self.tool_context(&tc.id, interrupt);
            let interrupt = interrupt.clone();
            pending.push(PendingResult::Running(tokio::spawn(async move {
                match tool {
                    Some(t) => {
                        tokio::select! {
                            biased;
                            _ = wait_interrupt(&interrupt) => ToolResult::error("interrupted"),
                            result = t.execute(args, &ctx) => result,
                        }
                    }
                    None => ToolResult::error(format!("unknown tool: {name}")),
                }
            })));
        }

        // Tasks are already running concurrently. Awaiting their handles in
        // call order preserves the required call-id/result pairing.
        for (tc, pending) in batch.iter().zip(pending) {
            let (result, emit_finished) = match pending {
                PendingResult::Ready {
                    result,
                    emit_finished,
                } => (result, emit_finished),
                PendingResult::Running(handle) => (
                    handle
                        .await
                        .unwrap_or_else(|_| ToolResult::error("tool task panicked")),
                    true,
                ),
            };
            if emit_finished {
                self.emit(AgentEvent::ToolFinished {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    ok: !result.is_error,
                    summary: first_line(&result.content, 120),
                });
            }
            self.session
                .push(Message::tool_result(&tc.id, &tc.name, result.content));
        }
    }

    /// Record a result for calls that were requested by the model but could
    /// not be started. Leaving them unanswered corrupts the next API request.
    pub(super) fn finish_unrun_tool_calls(&mut self, calls: &[ToolCall], reason: &str) {
        for call in calls {
            self.session.push(Message::tool_result(
                &call.id,
                &call.name,
                format!("tool call not run: {reason}"),
            ));
        }
    }

    pub(super) fn detect_repeated_tool_call(
        &mut self,
        call: &ToolCall,
        loop_redirects: &mut Vec<String>,
    ) -> bool {
        if !self.loop_detector.record(&call.name, &call.arguments) {
            return false;
        }

        self.emit(AgentEvent::LoopDetected {
            tool: call.name.clone(),
        });
        loop_redirects.push(redirect_message(&call.name, &call.arguments));
        self.loop_detector.reset_streak();
        true
    }

    fn validate_tool_call(&self, name: &str, path: Option<&str>) -> Result<(), String> {
        if self.depth > 0 {
            return Ok(());
        }
        match self.mode {
            AgentMode::Plan => plan_mode_check(name, path, &self.cwd),
            AgentMode::Multitask => multitask_mode_check(name),
            AgentMode::Make => Ok(()),
        }
    }

    fn tool_context(&self, id: &str, interrupt: &Arc<AtomicBool>) -> ToolContext {
        ToolContext {
            cwd: self.cwd.clone(),
            events: self.events.clone(),
            spawner: self.spawner.clone(),
            skills: self.skills.clone(),
            config: self.config.clone(),
            terminal: self.terminal.clone(),
            vision: self.vision_capable,
            depth: self.depth,
            call_id: id.to_string(),
            isolate_worktrees: self.depth == 0 && self.mode == AgentMode::Multitask,
            interrupt: interrupt.clone(),
        }
    }

    async fn execute_tool(
        &self,
        name: &str,
        args: serde_json::Value,
        id: &str,
        interrupt: &Arc<AtomicBool>,
    ) -> ToolResult {
        let tool = self.tools.iter().find(|t| t.name() == name).cloned();
        match tool {
            Some(t) => {
                let ctx = self.tool_context(id, interrupt);
                // Drop the tool future on Esc so HTTP/spawn unblock; shell
                // also kills its process group in Drop / on interrupt.
                tokio::select! {
                    biased;
                    _ = wait_interrupt(interrupt) => ToolResult::error("interrupted"),
                    result = t.execute(args, &ctx) => result,
                }
            }
            None => ToolResult::error(format!("unknown tool: {name}")),
        }
    }

    async fn emit_plan_updated(&self) {
        let path = plan_path(&self.cwd);
        let body = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let summary = plan_summary(&body);
        self.emit(AgentEvent::PlanUpdated { summary, body });
    }
}

/// Tools that are safe to run concurrently — they don't mutate the session
/// or agent state, just spawn work and return a report.
pub(super) fn is_parallel_tool(name: &str) -> bool {
    matches!(name, "spawn_subagent" | "verify_project")
}

/// A short, human-friendly summary of a tool call — the one argument that
/// matters, not the raw JSON. Falls back to a compact key list.
pub fn tool_args_preview(name: &str, arguments: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(arguments).unwrap_or(serde_json::Value::Null);
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);

    let main = match name {
        "read_file" | "write_file" | "edit_file" | "delete_path" | "list_dir" => s("path"),
        "write_plan" => s("summary"),
        "switch_mode" => s("mode"),
        "glob" | "grep" => s("pattern"),
        "run_shell" => s("command"),
        "terminal_start" => s("command"),
        "terminal_read" | "terminal_write" | "terminal_stop" => s("session_id"),
        "web_search" => s("query"),
        "read_skill" => s("name"),
        "spawn_subagent" | "spawn_swarm" => s("task").or_else(|| s("prompt")),
        "verify_project" => s("prompt")
            .or_else(|| s("task"))
            .or_else(|| Some("project check".into())),
        "web_get_contents" => v
            .get("urls")
            .and_then(|u| u.as_array())
            .and_then(|a| a.first())
            .and_then(|x| x.as_str())
            .map(str::to_string),
        _ => None,
    };

    match main {
        Some(m) => truncate(&m, 80),
        None => match v.as_object() {
            Some(o) => o.keys().take(3).cloned().collect::<Vec<_>>().join(", "),
            None => String::new(),
        },
    }
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

fn first_line(s: &str, max: usize) -> String {
    let line = s.lines().next().unwrap_or("");
    truncate(line, max)
}
