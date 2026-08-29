use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::AppConfig;
use crate::event::{AgentEvent, EventSender, ToolBatchCall};
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

use super::mode::{
    multitask_mode_check, multitask_mode_tool_allowed, plan_mode_check, plan_mode_tool_allowed,
    plan_path, plan_summary, AgentMode, PLAN_REL_PATH,
};
use super::prompt::build_system_prompt;
use super::session::Session;

/// Hard backstop for a single turn: if the model keeps issuing tool calls
/// past this many rounds, the turn is stopped. Esc remains the manual escape.
pub const MAX_ROUNDS: u64 = 64;

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
        }
    }
}

/// One conversational agent: a provider, a tool set, and a running session.
/// Drives the agent loop and emits events for a frontend to render.
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

    pub fn set_vision_capable(&mut self, capable: bool) {
        self.vision_capable = capable;
    }

    pub fn vision_capable(&self) -> bool {
        self.vision_capable
    }

    pub fn mode(&self) -> AgentMode {
        self.mode
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
        use super::mode::is_orchestrator_tool;
        if self.depth > 0 {
            return self
                .tool_specs
                .iter()
                .filter(|t| {
                    !is_orchestrator_tool(&t.name)
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
                    !is_orchestrator_tool(&t.name)
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
    }

    pub fn reset(&mut self) {
        if let Some(terminal) = &self.terminal {
            terminal.shutdown();
        }
        self.spawner.shutdown();
        self.session.reset();
        self.last_prompt_tokens = 0;
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

        let mut split = super::compact::tail_start(&self.session.messages);
        if force && split <= 1 && self.session.messages.len() > 2 {
            split = self.session.messages.len();
        }
        let old = &self.session.messages[1..split];
        if old.is_empty() {
            if force {
                return Err("nothing to compact yet".into());
            }
            return Ok(());
        }
        let transcript = format_transcript(old);
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

        // The summarizer call is maintenance, not conversation: its tokens
        // must not pollute the session usage gauge or compact thresholds.
        let summary = outcome.message.text();
        if summary.trim().is_empty() {
            return Err("compact failed: empty summary".into());
        }

        let system = self.session.system().to_string();
        let recent = self.session.messages[split..].to_vec();
        self.session
            .replace_messages(compacted_messages(&system, &summary, recent));
        let after = estimate_tokens(&self.session.messages);
        self.last_prompt_tokens = after;

        self.emit(AgentEvent::Compacted {
            before: Some(before),
            after,
        });
        Ok(())
    }

    /// Run one user turn to completion: stream the model, execute any tool calls,
    /// and repeat until the model stops calling tools or the round cap trips.
    /// Returns the final assistant text.
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
        let mut rounds: u64 = 0;

        loop {
            rounds += 1;

            if interrupt.load(Ordering::Relaxed) {
                self.emit(AgentEvent::Notice("Interrupted.".to_string()));
                break;
            }

            // Hard backstop: stop a turn that will not converge on its own.
            if rounds > MAX_ROUNDS {
                self.emit(AgentEvent::Notice(format!(
                    "stopped after {MAX_ROUNDS} rounds — the turn was not converging; press Esc or rephrase the request"
                )));
                break;
            }

            // Mid-turn safe point: tool results (if any) are already in history.
            if let Some(fu) = take_follow_up(&follow_up) {
                push_user_input(&mut self.session, fu);
            }
            self.maybe_auto_compact().await;

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
            let first_delta_at = Arc::new(Mutex::new(None::<Instant>));
            let first_delta_cb = first_delta_at.clone();
            let model_started = Instant::now();
            let mut on_delta = move |d: Delta| {
                if let Ok(mut first) = first_delta_cb.lock() {
                    first.get_or_insert_with(Instant::now);
                }
                match d {
                    Delta::Text(t) => {
                        let _ = events.send(AgentEvent::AssistantTextDelta(t));
                    }
                    Delta::Reasoning(r) => {
                        let _ = events.send(AgentEvent::ReasoningDelta(r));
                    }
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
            let parallel_batch_size = largest_parallel_batch(&tool_calls);
            let model_elapsed = model_started.elapsed();
            let first_delta_ms = first_delta_at
                .lock()
                .ok()
                .and_then(|first| first.map(|at| at.duration_since(model_started).as_millis()));
            tracing::info!(
                target: "hive::agent_metrics",
                round = rounds,
                depth = self.depth,
                model = %self.model,
                model_ms = model_elapsed.as_millis(),
                first_delta_ms = first_delta_ms.unwrap_or(model_elapsed.as_millis()),
                prompt_tokens = outcome.usage.prompt_tokens,
                completion_tokens = outcome.usage.completion_tokens,
                tool_calls = tool_calls.len(),
                parallel_batch_size,
                "model round completed"
            );
            self.session.push(outcome.message);

            if !assistant_text.trim().is_empty() {
                self.emit(AgentEvent::AssistantMessage(assistant_text.clone()));
                final_text = assistant_text.clone();
            }

            if tool_calls.is_empty() {
                // The model ran out of context mid-answer. Compact once and
                // keep going; if compaction cannot help, stop with a clear
                // error instead of looping on truncated output.
                if outcome.finish_reason == "length" {
                    match self.compact_inner(true).await {
                        Ok(()) => {
                            self.emit(AgentEvent::Notice(
                                "context window overflowed — compacted history and continuing"
                                    .to_string(),
                            ));
                            continue;
                        }
                        Err(error) => {
                            self.emit(AgentEvent::Error(format!(
                                "context window overflowed and compaction could not recover: {error}"
                            )));
                            break;
                        }
                    }
                }
                if assistant_text.trim().is_empty() {
                    break;
                }
                // Final reply — but a follow-up means continue.
                if let Some(fu) = take_follow_up(&follow_up) {
                    push_user_input(&mut self.session, fu);
                    continue;
                }
                break;
            }

            // Delegate tools that only spawn work stay concurrent; read-only
            // file/search/web calls run together. Writes and shell stay ordered.
            let mut i = 0;
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
                        self.run_tool(&call.id, &call.name, &call.arguments, &interrupt)
                            .await;
                    } else {
                        let batch_id = format!("explore-{rounds}-{start}");
                        let calls = batch
                            .iter()
                            .map(|call| ToolBatchCall {
                                id: call.id.clone(),
                                name: call.name.clone(),
                                args_preview: tool_args_preview(&call.name, &call.arguments),
                            })
                            .collect();
                        self.emit(AgentEvent::ToolBatchStarted {
                            id: batch_id.clone(),
                            calls,
                        });
                        let batch_started = Instant::now();
                        let failed = self.run_tools_parallel(batch, &interrupt).await;
                        let elapsed_ms = batch_started.elapsed().as_millis();
                        tracing::info!(
                            target: "hive::agent_metrics",
                            batch = %batch_id,
                            tools = batch.len(),
                            failed,
                            elapsed_ms,
                            "parallel tool batch completed"
                        );
                        self.emit(AgentEvent::ToolBatchFinished {
                            id: batch_id,
                            elapsed_ms,
                            failed,
                        });
                    }
                } else {
                    self.run_tool(&tc.id, &tc.name, &tc.arguments, &interrupt)
                        .await;
                    i += 1;
                }
            }

            if interrupt.load(Ordering::Relaxed) {
                break;
            }
        }

        // Don't drop a staged follow-up if we exited on interrupt/error.
        if let Some(fu) = take_follow_up(&follow_up) {
            push_user_input(&mut self.session, fu);
        }

        if interrupt.load(Ordering::Relaxed) {
            if let Some(terminal) = &self.terminal {
                let terminal = terminal.clone();
                let _ =
                    tokio::task::spawn_blocking(move || terminal.stop_if_agent_controlled()).await;
            }
        }

        self.emit(AgentEvent::TurnFinished);
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

fn largest_parallel_batch(calls: &[ToolCall]) -> usize {
    let mut largest = 0;
    let mut current = 0;
    for call in calls {
        if is_parallel_tool(&call.name) {
            current += 1;
            largest = largest.max(current);
        } else {
            current = 0;
        }
    }
    largest
}

#[path = "tool_dispatch.rs"]
mod tool_dispatch;
use tool_dispatch::is_parallel_tool;
pub use tool_dispatch::tool_args_preview;

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
