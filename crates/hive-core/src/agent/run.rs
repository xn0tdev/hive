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
}

#[path = "tool_dispatch.rs"]
mod tool_dispatch;
use tool_dispatch::is_parallel_tool;
pub use tool_dispatch::tool_args_preview;

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
