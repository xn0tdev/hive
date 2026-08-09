use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::AppConfig;
use crate::event::{AgentEvent, EventSender};
use crate::message::{ContentPart, ImageSource, Message, ToolCall};
use crate::provider::{ChatRequest, Delta, LlmProvider, ToolSpec, Usage};
use crate::skill::SkillSource;
use crate::spawner::SubagentSpawner;
use crate::terminal::{is_terminal_tool, TerminalHandle};
use crate::tool::{Tool, ToolContext, ToolResult};

use super::session_store::{self, SessionSnapshot};

use super::compact::{
    compacted_messages, estimate_tokens, format_transcript, should_compact, summarize_request,
    MIN_MESSAGES_TO_COMPACT,
};
use super::loop_detect::{redirect_message, LoopDetector};
use super::mode::{
    multitask_mode_check, multitask_mode_tool_allowed, plan_mode_check, plan_mode_tool_allowed,
    plan_path, plan_summary, AgentMode, PLAN_REL_PATH,
};
use super::prompt::build_system_prompt;
use super::session::Session;

/// Input for a single user turn: text plus any attached images.
pub struct UserInput {
    pub text: String,
    pub images: Vec<ImageSource>,
    pub mode: AgentMode,
}

/// Mid-turn follow-up staged by the TUI (double-Enter → next tool round).
pub type FollowUpSlot = Arc<Mutex<Option<UserInput>>>;

fn take_follow_up(slot: &FollowUpSlot) -> Option<UserInput> {
    slot.lock().ok().and_then(|mut g| g.take())
}

/// Poll until Esc sets the interrupt flag. Used with `tokio::select!` so a
/// long `chat_stream` or tool call can abort without waiting for completion.
async fn wait_interrupt(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn push_user_input(session: &mut Session, input: UserInput) {
    let mut parts: Vec<ContentPart> = Vec::new();
    if !input.text.is_empty() {
        parts.push(ContentPart::Text(input.text));
    }
    for img in input.images {
        parts.push(ContentPart::Image(img));
    }
    if !parts.is_empty() {
        session.push(Message::user_parts(parts));
    }
}

impl From<String> for UserInput {
    fn from(text: String) -> Self {
        UserInput {
            text,
            images: Vec::new(),
            mode: AgentMode::Make,
        }
    }
}

impl From<&str> for UserInput {
    fn from(text: &str) -> Self {
        UserInput::from(text.to_string())
    }
}

/// Holds the shared, cloneable ingredients needed to construct an agent. The
/// composition root builds one of these; the swarm reuses it to spin up
/// subagents. Note it deliberately does NOT hold a spawner, so there is no
/// reference cycle between agents and the swarm.
#[derive(Clone)]
pub struct AgentBuilder {
    pub provider: Arc<dyn LlmProvider>,
    pub tools: Vec<Arc<dyn Tool>>,
    pub skills: Arc<dyn SkillSource>,
    pub config: Arc<AppConfig>,
}

impl AgentBuilder {
    pub fn build(
        &self,
        events: EventSender,
        model: String,
        depth: usize,
        spawner: Arc<dyn SubagentSpawner>,
    ) -> Agent {
        self.build_in(
            events,
            model,
            depth,
            spawner,
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        )
    }

    pub fn build_in(
        &self,
        events: EventSender,
        model: String,
        depth: usize,
        spawner: Arc<dyn SubagentSpawner>,
        cwd: PathBuf,
    ) -> Agent {
        self.build_inner(events, model, depth, spawner, cwd, None)
    }

    pub fn build_with_terminal(
        &self,
        events: EventSender,
        model: String,
        depth: usize,
        spawner: Arc<dyn SubagentSpawner>,
        terminal: TerminalHandle,
    ) -> Agent {
        self.build_inner(
            events,
            model,
            depth,
            spawner,
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            Some(terminal),
        )
    }

    fn build_inner(
        &self,
        events: EventSender,
        model: String,
        depth: usize,
        spawner: Arc<dyn SubagentSpawner>,
        cwd: PathBuf,
        terminal: Option<TerminalHandle>,
    ) -> Agent {
        let mode = AgentMode::Make;
        let system = build_system_prompt(&cwd, self.skills.as_ref(), depth > 0, mode);
        let tool_specs = self
            .tools
            .iter()
            .map(|tool| {
                Arc::new(ToolSpec {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    parameters: tool.parameters(),
                })
            })
            .collect();
        Agent {
            provider: self.provider.clone(),
            tools: self.tools.clone(),
            tool_specs,
            skills: self.skills.clone(),
            config: self.config.clone(),
            spawner,
            terminal,
            events,
            session: Session::new(system),
            model,
            vision_capable: false,
            depth,
            cwd,
            mode,
            last_prompt_tokens: 0,
            loop_detector: LoopDetector::default(),
            goal: Arc::new(Mutex::new(None)),
        }
    }
}

/// One conversational agent: a provider, a tool set, and a running session.
/// Drives the YOLO loop and emits events for a frontend to render.
pub struct Agent {
    provider: Arc<dyn LlmProvider>,
    tools: Vec<Arc<dyn Tool>>,
    tool_specs: Vec<Arc<ToolSpec>>,
    skills: Arc<dyn SkillSource>,
    config: Arc<AppConfig>,
    spawner: Arc<dyn SubagentSpawner>,
    terminal: Option<TerminalHandle>,
    events: EventSender,
    session: Session,
    model: String,
    /// When false, tools must not inject image parts (non-vision models reject them).
    vision_capable: bool,
    depth: usize,
    cwd: PathBuf,
    mode: AgentMode,
    /// Prompt tokens from the most recent chat request (for auto-compact).
    last_prompt_tokens: u64,
    /// Detects repeated tool calls so the agent can break out of loops.
    loop_detector: LoopDetector,
    /// Active goal for the autonomous loop (None = no goal).
    goal: Arc<Mutex<Option<GoalState>>>,
}

/// Persistent goal state for the autonomous agent loop.
#[derive(Debug, Clone)]
pub struct GoalState {
    pub objective: String,
    pub deadline: Option<std::time::Instant>,
    pub paused: bool,
}

impl GoalState {
    /// Remaining seconds until the deadline (0 if no deadline or expired).
    pub fn remaining_secs(&self) -> u64 {
        self.deadline
            .map(|d| {
                let now = std::time::Instant::now();
                if d > now {
                    d.duration_since(now).as_secs()
                } else {
                    0
                }
            })
            .unwrap_or(0)
    }

    /// True when the deadline has passed.
    pub fn expired(&self) -> bool {
        self.deadline
            .is_some_and(|d| std::time::Instant::now() >= d)
    }
}

impl Agent {
    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.model = model.into();
    }

    pub fn set_context_window(&mut self, window: u64) {
        let cfg = Arc::make_mut(&mut self.config);
        cfg.agent.context_window = window;
    }

    // ── Goal / autonomous loop ──────────────────────────────────────────

    /// Set a goal for the autonomous loop. `deadline = None` means no timer —
    /// the agent keeps working until the user stops it.
    pub fn set_goal(&self, objective: String, deadline: Option<std::time::Instant>) {
        if let Ok(mut g) = self.goal.lock() {
            *g = Some(GoalState {
                objective,
                deadline,
                paused: false,
            });
        }
    }

    pub fn stop_goal(&self) {
        if let Ok(mut g) = self.goal.lock() {
            *g = None;
        }
    }

    pub fn pause_goal(&self) {
        if let Ok(mut g) = self.goal.lock() {
            if let Some(gs) = g.as_mut() {
                gs.paused = true;
            }
        }
    }

    pub fn resume_goal(&self) {
        if let Ok(mut g) = self.goal.lock() {
            if let Some(gs) = g.as_mut() {
                gs.paused = false;
            }
        }
    }

    /// True when a goal is active, not paused, and not expired.
    pub fn goal_active(&self) -> bool {
        self.goal
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|gs| !gs.paused && !gs.expired()))
            .unwrap_or(false)
    }

    /// Snapshot of the current goal (if any).
    pub fn goal_snapshot(&self) -> Option<GoalState> {
        self.goal.lock().ok().and_then(|g| g.clone())
    }

    pub fn set_vision_capable(&mut self, capable: bool) {
        self.vision_capable = capable;
    }

    pub fn vision_capable(&self) -> bool {
        self.vision_capable
    }

    /// Swap the LLM HTTP client (e.g. after `/connect` switches provider).
    pub fn set_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.provider = provider;
    }

    pub fn provider_clone(&self) -> Arc<dyn LlmProvider> {
        self.provider.clone()
    }

    pub fn tools_clone(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }

    pub fn skills_clone(&self) -> Arc<dyn SkillSource> {
        self.skills.clone()
    }

    pub fn config_clone(&self) -> Arc<AppConfig> {
        self.config.clone()
    }

    pub fn spawner_clone(&self) -> Arc<dyn SubagentSpawner> {
        self.spawner.clone()
    }

    pub fn events_clone(&self) -> EventSender {
        self.events.clone()
    }

    /// Apply MAKE/PLAN for the next turn and refresh the system prompt.
    pub fn set_mode(&mut self, mode: AgentMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        if self.depth == 0 {
            let system = build_system_prompt(&self.cwd, self.skills.as_ref(), false, mode);
            self.session.set_system(system);
        }
    }

    fn active_tool_specs(&self) -> Vec<Arc<ToolSpec>> {
        // Subagents always get the full MAKE tool set (no swarm fan-out, and no
        // self mode-switching — only the top-level agent owns the session mode).
        if self.depth > 0 {
            return self
                .tool_specs
                .iter()
                .filter(|t| {
                    t.name.as_str() != "spawn_swarm"
                        && t.name.as_str() != "integrate_worktree"
                        && t.name.as_str() != "switch_mode"
                        && !is_terminal_tool(&t.name)
                })
                .cloned()
                .collect();
        }
        match self.mode {
            AgentMode::Make => self
                .tool_specs
                .iter()
                .filter(|t| {
                    t.name.as_str() != "spawn_swarm"
                        && t.name.as_str() != "integrate_worktree"
                        && (self.terminal.is_some() || !is_terminal_tool(&t.name))
                })
                .cloned()
                .collect(),
            AgentMode::Plan => self
                .tool_specs
                .iter()
                .filter(|t| plan_mode_tool_allowed(&t.name))
                .cloned()
                .collect(),
            AgentMode::Multitask => self
                .tool_specs
                .iter()
                .filter(|t| multitask_mode_tool_allowed(&t.name))
                .cloned()
                .collect(),
        }
    }

    pub fn usage(&self) -> Usage {
        self.session.usage
    }

    /// Prompt tokens from the most recent request — what the context gauge shows.
    pub fn context_tokens(&self) -> u64 {
        self.last_prompt_tokens
    }

    /// Snapshot the session for persistence. Pass `created_at` from an existing
    /// snapshot when re-saving so the creation time stays put.
    pub fn session_snapshot(&self, id: &str, created_at: Option<u64>) -> SessionSnapshot {
        session_store::snapshot(
            id,
            &self.model,
            self.session.messages.clone(),
            self.session.usage,
            session_store::SessionEnv {
                connection_id: self.config.connections.active.clone(),
                context_window: self.context_window(),
                vision: self.vision_capable,
                created_at,
            },
        )
    }

    /// Restore a saved transcript. Model and provider are *not* touched here —
    /// a saved model id is only valid on the connection it came from, so the
    /// caller switches connection first and then sets the model.
    pub fn restore_session(&mut self, messages: Vec<Message>, usage: Usage) {
        self.session.replace_messages(messages);
        self.session.usage = usage;
        // Approximate the restored context so the gauge and auto-compact aren't
        // blind until the next response reports real prompt tokens.
        self.last_prompt_tokens = estimate_tokens(&self.session.messages);
        self.loop_detector = LoopDetector::default();
    }

    pub fn reset(&mut self) {
        if let Some(terminal) = &self.terminal {
            terminal.shutdown();
        }
        self.session.reset();
        self.last_prompt_tokens = 0;
        self.loop_detector = LoopDetector::default();
    }

    fn context_window(&self) -> u64 {
        let w = self.config.agent.context_window;
        if w == 0 {
            crate::config::DEFAULT_CONTEXT_WINDOW
        } else {
            w
        }
    }

    fn emit(&self, e: AgentEvent) {
        let _ = self.events.send(e);
    }

    /// Manually compact conversation history (`/compact`).
    pub async fn compact(&mut self) -> Result<(), String> {
        self.compact_inner(true).await
    }

    /// Auto-compact at 75% context when needed (silent no-op otherwise).
    async fn maybe_auto_compact(&mut self) {
        if let Err(e) = self.compact_inner(false).await {
            // Don't stall the turn — just surface the failure.
            self.emit(AgentEvent::Notice(e));
        }
    }

    async fn compact_inner(&mut self, force: bool) -> Result<(), String> {
        if self.session.messages.len() < MIN_MESSAGES_TO_COMPACT {
            if force {
                return Err("nothing to compact yet".into());
            }
            return Ok(());
        }

        let window = self.context_window();
        let estimated = estimate_tokens(&self.session.messages);
        let over =
            should_compact(self.last_prompt_tokens, window) || should_compact(estimated, window);
        if !force && !over {
            return Ok(());
        }

        let before = estimated;

        // Spinner: compaction in progress.
        self.emit(AgentEvent::Compacted {
            before: None,
            after: 0,
        });

        let transcript = format_transcript(&self.session.messages);
        if transcript.trim().is_empty() {
            if force {
                return Err("nothing to compact yet".into());
            }
            return Ok(());
        }

        let messages = summarize_request(&transcript);
        let req = ChatRequest {
            model: &self.model,
            messages: &messages,
            tools: &[],
            temperature: Some(0.2),
            max_tokens: Some(2_048),
        };
        let mut on_delta = |_d: Delta| {};
        let outcome = self
            .provider
            .chat_stream(req, &mut on_delta)
            .await
            .map_err(|e| format!("compact failed: {e}"))?;

        self.last_prompt_tokens = outcome.usage.prompt_tokens;
        self.session.add_usage(outcome.usage);
        self.emit(AgentEvent::Usage(self.session.usage));
        self.emit(AgentEvent::ContextTokens(self.last_prompt_tokens));

        let summary = outcome.message.text();
        if summary.trim().is_empty() {
            return Err("compact failed: empty summary".into());
        }

        let system = self.session.system().to_string();
        self.session
            .replace_messages(compacted_messages(&system, &summary));
        let after = estimate_tokens(&self.session.messages);
        self.last_prompt_tokens = after;

        self.emit(AgentEvent::Compacted {
            before: Some(before),
            after,
        });
        Ok(())
    }

    /// Run one user turn to completion: stream the model, execute any tool calls
    /// (immediately, no confirmation), and repeat until the model stops calling
    /// tools. Returns the final assistant text.
    ///
    /// `follow_up`: optional mid-turn inject from the TUI (second Enter). Applied
    /// before the next model call — after the current tool batch finishes.
    pub async fn run_turn(
        &mut self,
        input: UserInput,
        interrupt: Arc<AtomicBool>,
        follow_up: FollowUpSlot,
    ) -> String {
        if self.depth == 0 {
            self.set_mode(input.mode);
        }
        self.emit(AgentEvent::TurnStarted);

        push_user_input(&mut self.session, input);

        let mut final_text = String::new();

        loop {
            if interrupt.load(Ordering::Relaxed) {
                self.emit(AgentEvent::Notice("Interrupted.".to_string()));
                break;
            }

            // Mid-turn safe point: tool results (if any) are already in history.
            if self.depth == 0 {
                if let Some(fu) = take_follow_up(&follow_up) {
                    push_user_input(&mut self.session, fu);
                }
                self.maybe_auto_compact().await;
            }

            let tools = self.active_tool_specs();
            let req = ChatRequest {
                model: &self.model,
                messages: &self.session.messages,
                tools: &tools,
                temperature: Some(0.3),
                max_tokens: None,
            };

            self.emit(AgentEvent::AssistantStarted);

            let events = self.events.clone();
            let saw_reasoning = Arc::new(AtomicBool::new(false));
            let saw_reasoning_cb = saw_reasoning.clone();
            let mut on_delta = move |d: Delta| match d {
                Delta::Text(t) => {
                    let _ = events.send(AgentEvent::AssistantTextDelta(t));
                }
                Delta::Reasoning(r) => {
                    saw_reasoning_cb.store(true, Ordering::Relaxed);
                    let _ = events.send(AgentEvent::ReasoningDelta(r));
                }
            };

            // Esc must abort mid-stream — not only between rounds.
            let outcome = tokio::select! {
                biased;
                _ = wait_interrupt(&interrupt) => {
                    self.emit(AgentEvent::Notice("Interrupted.".to_string()));
                    break;
                }
                outcome = self.provider.chat_stream(req, &mut on_delta) => {
                    match outcome {
                        Ok(o) => o,
                        Err(e) => {
                            self.emit(AgentEvent::Error(format!("model error: {e}")));
                            break;
                        }
                    }
                }
            };

            self.last_prompt_tokens = outcome.usage.prompt_tokens;
            self.session.add_usage(outcome.usage);
            self.emit(AgentEvent::Usage(self.session.usage));
            self.emit(AgentEvent::ContextTokens(self.last_prompt_tokens));

            let assistant_text = outcome.message.text();
            let tool_calls = outcome.message.tool_calls.clone();
            self.session.push(outcome.message);

            if !assistant_text.trim().is_empty() {
                self.emit(AgentEvent::AssistantMessage(assistant_text.clone()));
                final_text = assistant_text.clone();
            }

            if tool_calls.is_empty() {
                if assistant_text.trim().is_empty() {
                    break;
                }
                // Final reply — but a double-Enter follow-up means continue.
                if self.depth == 0 {
                    if let Some(fu) = take_follow_up(&follow_up) {
                        push_user_input(&mut self.session, fu);
                        continue;
                    }
                }
                break;
            }

            // Delegate tools (spawn_subagent, verify_project) run concurrently
            // when the model returns several in one batch; everything else stays
            // sequential so side-effects (mode switch, file writes) stay ordered.
            let mut i = 0;
            let mut loop_redirects = Vec::new();
            while i < tool_calls.len() {
                if interrupt.load(Ordering::Relaxed) {
                    self.emit(AgentEvent::Notice("Interrupted.".to_string()));
                    self.finish_unrun_tool_calls(&tool_calls[i..], "turn interrupted");
                    break;
                }

                let tc = &tool_calls[i];

                if is_parallel_tool(&tc.name) {
                    let start = i;
                    i += 1;
                    while i < tool_calls.len() && is_parallel_tool(&tool_calls[i].name) {
                        i += 1;
                    }
                    let batch = &tool_calls[start..i];
                    if batch.len() == 1 {
                        let call = &batch[0];
                        if self.detect_repeated_tool_call(call, &mut loop_redirects) {
                            self.finish_unrun_tool_calls(
                                batch,
                                "identical call repeated too many times",
                            );
                        } else {
                            self.run_tool(&call.id, &call.name, &call.arguments, &interrupt)
                                .await;
                        }
                    } else {
                        self.run_tools_parallel(batch, &interrupt, &mut loop_redirects)
                            .await;
                    }
                } else {
                    if self.detect_repeated_tool_call(tc, &mut loop_redirects) {
                        self.finish_unrun_tool_calls(
                            std::slice::from_ref(tc),
                            "identical call repeated too many times",
                        );
                        i += 1;
                        continue;
                    }
                    self.run_tool(&tc.id, &tc.name, &tc.arguments, &interrupt)
                        .await;
                    i += 1;
                }
            }

            // Tool results must remain adjacent to the assistant tool-call
            // message. Inject loop guidance only after every call has a result.
            if !loop_redirects.is_empty() {
                self.session
                    .push(Message::user(loop_redirects.join("\n\n")));
            }

            if interrupt.load(Ordering::Relaxed) {
                break;
            }
        }

        // Don't drop a staged follow-up if we exited on interrupt/error.
        if self.depth == 0 {
            if let Some(fu) = take_follow_up(&follow_up) {
                push_user_input(&mut self.session, fu);
            }
        }

        if interrupt.load(Ordering::Relaxed) {
            if let Some(terminal) = &self.terminal {
                let terminal = terminal.clone();
                let _ =
                    tokio::task::spawn_blocking(move || terminal.stop_if_agent_controlled()).await;
            }
        }

        // Goal loop: instead of TurnFinished, emit a goal event so the driver
        // can start the next turn automatically.
        if self.depth == 0 {
            if let Some(gs) = self.goal_snapshot() {
                if gs.expired() {
                    self.emit(AgentEvent::GoalExpired {
                        objective: gs.objective.clone(),
                    });
                    self.stop_goal();
                } else if !gs.paused && !interrupt.load(Ordering::Relaxed) {
                    self.emit(AgentEvent::GoalContinue {
                        objective: gs.objective.clone(),
                        remaining_secs: gs.remaining_secs(),
                    });
                } else {
                    self.emit(AgentEvent::TurnFinished);
                }
            } else {
                self.emit(AgentEvent::TurnFinished);
            }
        } else {
            self.emit(AgentEvent::TurnFinished);
        }
        final_text
    }

    /// Run a one-shot prompt with no external interrupt, returning the final
    /// text. Used by subagents.
    pub async fn run_headless(&mut self, prompt: impl Into<String>) -> String {
        let interrupt = Arc::new(AtomicBool::new(false));
        let follow_up: FollowUpSlot = Arc::new(Mutex::new(None));
        self.run_turn(UserInput::from(prompt.into()), interrupt, follow_up)
            .await
    }

    async fn run_tool(
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
                if path == PLAN_REL_PATH || super::mode::is_plan_path(&self.cwd, path) {
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

    async fn run_tools_parallel(
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
    fn finish_unrun_tool_calls(&mut self, calls: &[ToolCall], reason: &str) {
        for call in calls {
            self.session.push(Message::tool_result(
                &call.id,
                &call.name,
                format!("tool call not run: {reason}"),
            ));
        }
    }

    fn detect_repeated_tool_call(
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
fn is_parallel_tool(name: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{all_tools, no_skills, noop_spawner, ChatOutcome, CoreError, TerminalManager};
    use async_trait::async_trait;
    use std::sync::atomic::AtomicUsize;

    struct NoopProvider;

    #[async_trait]
    impl LlmProvider for NoopProvider {
        async fn chat_stream(
            &self,
            _req: ChatRequest<'_>,
            _on_delta: &mut (dyn FnMut(Delta) + Send),
        ) -> Result<ChatOutcome, CoreError> {
            unreachable!("tool-spec tests do not call the provider")
        }
    }

    fn builder() -> AgentBuilder {
        AgentBuilder {
            provider: Arc::new(NoopProvider),
            tools: all_tools(),
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
        }
    }

    #[test]
    fn tool_args_preview_uses_the_meaningful_value() {
        assert_eq!(
            tool_args_preview("read_file", r#"{"path":"src/main.rs","offset":12}"#),
            "src/main.rs"
        );
        assert_eq!(
            tool_args_preview("web_search", r#"{"query":"rust tui"}"#),
            "rust tui"
        );
        assert_eq!(tool_args_preview("custom", r#"{"z":1,"a":2}"#), "a, z");
        assert!(tool_args_preview("custom", "not json").is_empty());
    }

    struct DelayedParallelTool {
        name: &'static str,
        delay_ms: u64,
        output: &'static str,
    }

    #[async_trait]
    impl Tool for DelayedParallelTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            ToolResult::ok(self.output)
        }
    }

    struct CountingTool {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Tool for CountingTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            self.calls.fetch_add(1, Ordering::Relaxed);
            ToolResult::ok("unexpected")
        }
    }

    struct SchemaCountingTool {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Tool for SchemaCountingTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "test tool"
        }

        fn parameters(&self) -> serde_json::Value {
            self.calls.fetch_add(1, Ordering::Relaxed);
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::ok("ok")
        }
    }

    #[test]
    fn tool_schemas_are_built_once_per_agent() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let agent = AgentBuilder {
            provider: Arc::new(NoopProvider),
            tools: vec![Arc::new(SchemaCountingTool {
                calls: calls.clone(),
            })],
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
        }
        .build(events, "test".into(), 0, noop_spawner());

        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let _ = agent.active_tool_specs();
        let _ = agent.active_tool_specs();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn parallel_results_keep_their_original_call_ids() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let tools: Vec<Arc<dyn Tool>> = vec![
            Arc::new(DelayedParallelTool {
                name: "verify_project",
                delay_ms: 30,
                output: "slow result",
            }),
            Arc::new(DelayedParallelTool {
                name: "spawn_subagent",
                delay_ms: 0,
                output: "fast result",
            }),
        ];
        let mut agent = AgentBuilder {
            provider: Arc::new(NoopProvider),
            tools,
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
        }
        .build(events, "test".into(), 0, noop_spawner());
        let calls = vec![
            ToolCall {
                id: "slow".into(),
                name: "verify_project".into(),
                arguments: "{}".into(),
            },
            ToolCall {
                id: "fast".into(),
                name: "spawn_subagent".into(),
                arguments: "{}".into(),
            },
        ];

        agent
            .run_tools_parallel(&calls, &Arc::new(AtomicBool::new(false)), &mut Vec::new())
            .await;

        let results = &agent.session.messages[1..];
        assert_eq!(results[0].tool_call_id.as_deref(), Some("slow"));
        assert_eq!(results[0].text(), "slow result");
        assert_eq!(results[1].tool_call_id.as_deref(), Some("fast"));
        assert_eq!(results[1].text(), "fast result");
    }

    #[tokio::test]
    async fn malformed_tool_arguments_do_not_execute_the_tool() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = AgentBuilder {
            provider: Arc::new(NoopProvider),
            tools: vec![Arc::new(CountingTool {
                calls: calls.clone(),
            })],
            skills: no_skills(),
            config: Arc::new(AppConfig::default()),
        }
        .build(events, "test".into(), 0, noop_spawner());

        agent
            .run_tool("bad-call", "echo", "{", &Arc::new(AtomicBool::new(false)))
            .await;

        assert_eq!(calls.load(Ordering::Relaxed), 0);
        let result = agent.session.messages.last().unwrap();
        assert_eq!(result.tool_call_id.as_deref(), Some("bad-call"));
        assert!(result.text().contains("invalid tool arguments"));
    }

    #[test]
    fn interrupted_calls_are_closed_in_history() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let mut agent = builder().build(events, "test".into(), 0, noop_spawner());
        let calls = vec![
            ToolCall {
                id: "one".into(),
                name: "read_file".into(),
                arguments: "{}".into(),
            },
            ToolCall {
                id: "two".into(),
                name: "grep".into(),
                arguments: "{}".into(),
            },
        ];

        agent.finish_unrun_tool_calls(&calls, "turn interrupted");

        let results = &agent.session.messages[1..];
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].tool_call_id.as_deref(), Some("one"));
        assert_eq!(results[1].tool_call_id.as_deref(), Some("two"));
        assert!(results
            .iter()
            .all(|message| message.text().contains("turn interrupted")));
    }

    fn tool_names(agent: &Agent) -> Vec<String> {
        agent
            .active_tool_specs()
            .into_iter()
            .map(|tool| tool.name.clone())
            .collect()
    }

    #[test]
    fn terminal_tools_only_root_make_with_manager() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let manager = TerminalManager::new(events.clone());
        let root = builder().build_with_terminal(
            events.clone(),
            "test".into(),
            0,
            noop_spawner(),
            manager,
        );
        let root_without_manager =
            builder().build(events.clone(), "test".into(), 0, noop_spawner());
        let subagent = builder().build(events, "test".into(), 1, noop_spawner());

        let terminal_names = [
            "terminal_start",
            "terminal_read",
            "terminal_write",
            "terminal_stop",
        ];
        let root_names = tool_names(&root);
        let plain_root_names = tool_names(&root_without_manager);
        let subagent_names = tool_names(&subagent);
        for name in terminal_names {
            assert!(root_names.iter().any(|tool| tool == name), "{name}");
            assert!(
                !plain_root_names.iter().any(|tool| tool == name),
                "{name} leaked without a manager"
            );
            assert!(
                !subagent_names.iter().any(|tool| tool == name),
                "{name} leaked to a subagent"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn interrupted_turn_stops_only_agent_controlled_terminal() {
        let (events, _) = tokio::sync::mpsc::unbounded_channel();
        let manager = TerminalManager::new(events.clone());
        let started = manager
            .start("cat", "test", std::path::Path::new("."))
            .await
            .unwrap();
        let mut agent = builder().build_with_terminal(
            events.clone(),
            "test".into(),
            0,
            noop_spawner(),
            manager.clone(),
        );
        let interrupt = Arc::new(AtomicBool::new(true));
        agent
            .run_turn(
                UserInput::from("stop"),
                interrupt,
                Arc::new(Mutex::new(None)),
            )
            .await;
        let stopped = manager.read(&started.id, None, None).await.unwrap();
        assert!(!matches!(
            stopped.session.process,
            crate::TerminalProcessState::Running
        ));

        let restarted = manager
            .start("cat", "test", std::path::Path::new("."))
            .await
            .unwrap();
        manager.attach(&restarted.id).unwrap();
        let interrupt = Arc::new(AtomicBool::new(true));
        agent
            .run_turn(
                UserInput::from("keep"),
                interrupt,
                Arc::new(Mutex::new(None)),
            )
            .await;
        let preserved = manager.read(&restarted.id, None, None).await.unwrap();
        assert!(matches!(
            preserved.session.process,
            crate::TerminalProcessState::Running
        ));
        manager.shutdown();
    }
}
