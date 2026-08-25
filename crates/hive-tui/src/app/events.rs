//! Agent event application and transcript mutation.

use super::*;

impl App {
    /// Apply an agent event. Returns `true` when the visible UI should redraw
    /// (background subagent transcript updates while on the main chat do not).
    pub fn apply(&mut self, ev: AgentEvent) -> bool {
        let transcript_changed = !matches!(
            &ev,
            AgentEvent::Usage(_)
                | AgentEvent::ContextTokens(_)
                | AgentEvent::ModelsListed { .. }
                | AgentEvent::ModelsListFailed(_)
                | AgentEvent::ConnectionsUpdated { .. }
                | AgentEvent::RecapDelta { .. }
                | AgentEvent::RecapFinished { .. }
                | AgentEvent::RecapFailed { .. }
        );
        let changed = self.apply_inner(ev);
        if changed && transcript_changed {
            self.invalidate_transcript();
        }
        changed
    }

    pub(super) fn apply_inner(&mut self, ev: AgentEvent) -> bool {
        match ev {
            AgentEvent::TurnStarted => {
                self.running = true;
                self.turn_started_at = Some(std::time::Instant::now());
                true
            }
            AgentEvent::AssistantStarted => {
                self.close_thought();
                self.finalize_streaming();
                true
            }
            AgentEvent::AssistantTextDelta(t) => {
                self.append_assistant(&t);
                true
            }
            AgentEvent::ReasoningDelta(t) => {
                self.append_reasoning(&t);
                true
            }
            AgentEvent::AssistantMessage(text) => {
                self.finalize_assistant(text);
                true
            }
            AgentEvent::ToolStarted {
                id,
                name,
                args_preview,
            } => {
                self.close_thought();
                let plan_write = is_plan_write(&name, &args_preview);
                if plan_write {
                    self.mark_plan_writing();
                }
                // Plan writes are card-only (no green/orange tool chrome).
                // Subagent tools render as Subagent cards, not tool rows.
                // `switch_mode` renders as its own "Switched to … Mode" card.
                if shows_generic_tool_card(&name, &args_preview) {
                    let details_open = matches!(name.as_str(), "edit_file" | "write_file");
                    self.blocks.push(Block::Tool(ToolCard {
                        id,
                        name,
                        args: args_preview,
                        output: String::new(),
                        status: ToolStatus::Running,
                        started: std::time::Instant::now(),
                        elapsed_ms: None,
                        details_open,
                        snapshot: None,
                    }));
                }
                true
            }
            AgentEvent::ToolOutput { id, chunk } => {
                self.append_tool_output(&id, &chunk);
                true
            }
            AgentEvent::ToolFinished { id, ok, .. } => {
                self.finish_tool(&id, ok);
                true
            }
            AgentEvent::FileSnapshot { id, path, content } => {
                self.attach_snapshot(&id, FileSnapshot { path, content });
                false
            }
            AgentEvent::Usage(u) => {
                self.usage = u;
                true
            }
            AgentEvent::ContextTokens(n) => {
                self.context_tokens = n;
                true
            }
            AgentEvent::SubagentSpawned { id, label, prompt } => {
                self.blocks.push(Block::Subagent(SubagentCard {
                    id,
                    label,
                    status: SubagentStatus::Running,
                    detail: String::new(),
                    prompt,
                    lines: Vec::new(),
                    usage: Usage::default(),
                    started: std::time::Instant::now(),
                    elapsed_ms: None,
                }));
                true
            }
            AgentEvent::SubagentStatus { id, status, detail } => {
                self.update_subagent(&id, status, detail);
                // Status line is visible on the main-chat card.
                true
            }
            AgentEvent::SubagentUsage { id, usage } => {
                self.update_subagent_usage(&id, usage);
                true
            }
            AgentEvent::SubagentTranscript { id, line } => {
                self.append_subagent_line(&id, line);
                // Transcript body is only painted in the dedicated subagent view.
                matches!(&self.view, ChatView::Subagent(open) if *open == id)
            }
            AgentEvent::PlanUpdated { summary, body } => {
                self.upsert_plan(summary, body);
                true
            }
            AgentEvent::ModeSwitched { mode, reason } => {
                self.agent_mode = mode;
                self.blocks
                    .push(Block::ModeSwitch(ModeSwitchCard { mode, reason }));
                self.scroll_from_bottom = 0;
                self.flash(format!("Switched to {} mode", mode.title()));
                true
            }
            AgentEvent::LoopDetected { tool: _ } => {
                self.blocks.push(Block::LoopDetected(LoopDetectedCard));
                self.scroll_from_bottom = 0;
                self.flash("Loop detected · restarting task");
                true
            }
            AgentEvent::Compacted { before, after } => {
                // Replace the in-progress compaction card if present, else add.
                if let Some(Block::Compacted(c)) = self
                    .blocks
                    .iter_mut()
                    .rev()
                    .find(|b| matches!(b, Block::Compacted(_)))
                {
                    c.before = before;
                    c.after = after;
                } else {
                    self.blocks
                        .push(Block::Compacted(CompactedCard { before, after }));
                }
                self.scroll_from_bottom = 0;
                if let Some(b) = before {
                    self.flash(format!(
                        "Context compacted · {} → {}",
                        short_tokens(b),
                        short_tokens(after)
                    ));
                }
                true
            }
            AgentEvent::TerminalStarted {
                id,
                command,
                description,
                rows,
                cols,
            } => {
                self.close_thought();
                if !self
                    .blocks
                    .iter()
                    .any(|block| matches!(block, Block::Terminal(card) if card.id == id))
                {
                    let parser = vt100::Parser::new(rows.max(1), cols.max(1), 10_000);
                    self.blocks.push(Block::Terminal(Box::new(TerminalCard {
                        id,
                        command,
                        description,
                        controller: TerminalController::Agent,
                        process: TerminalProcessState::Running,
                        revision: 0,
                        screen: parser.screen().clone(),
                        started: std::time::Instant::now(),
                        elapsed_ms: None,
                    })));
                }
                true
            }
            AgentEvent::TerminalStartFailed {
                id,
                command,
                description,
                message,
            } => {
                self.close_thought();
                if !self
                    .blocks
                    .iter()
                    .any(|block| matches!(block, Block::Terminal(card) if card.id == id))
                {
                    let parser = vt100::Parser::new(1, 1, 0);
                    self.blocks.push(Block::Terminal(Box::new(TerminalCard {
                        id,
                        command,
                        description,
                        controller: TerminalController::Agent,
                        process: TerminalProcessState::Failed {
                            message: message.clone(),
                        },
                        revision: 0,
                        screen: parser.screen().clone(),
                        started: std::time::Instant::now(),
                        elapsed_ms: Some(0),
                    })));
                }
                self.flash(message);
                true
            }
            AgentEvent::TerminalOutput { id, frame } => {
                let Some((screen, revision)) = frame.snapshot() else {
                    return false;
                };
                let new_request = {
                    let Some(card) = self.blocks.iter_mut().find_map(|block| match block {
                        Block::Terminal(card) if card.id == id => Some(card),
                        _ => None,
                    }) else {
                        return false;
                    };
                    let previous = card.input_request();
                    card.screen = screen;
                    card.revision = card.revision.max(revision);
                    let current = card.input_request();
                    if current != previous {
                        current
                    } else {
                        None
                    }
                };
                if new_request.is_some_and(|request| request.is_private()) {
                    self.flash("Background terminal needs private input · click its card to open");
                }
                true
            }
            AgentEvent::TerminalState {
                id,
                controller,
                process,
                revision,
            } => {
                let Some(card) = self.blocks.iter_mut().find_map(|block| match block {
                    Block::Terminal(card) if card.id == id => Some(card),
                    _ => None,
                }) else {
                    return false;
                };
                card.controller = controller;
                card.process = process;
                card.revision = card.revision.max(revision);
                if !matches!(card.process, TerminalProcessState::Running)
                    && card.elapsed_ms.is_none()
                {
                    card.elapsed_ms = Some(card.started.elapsed().as_millis());
                }
                let phase = if !matches!(card.process, TerminalProcessState::Running) {
                    TerminalViewPhase::ReadOnly
                } else if card.controller == TerminalController::User {
                    TerminalViewPhase::UserControl
                } else {
                    TerminalViewPhase::Attaching
                };
                if self.terminal_view_id() == Some(id.as_str()) {
                    self.terminal_view.phase = phase;
                }
                true
            }
            AgentEvent::TerminalResized { id, rows, cols } => {
                let Some(card) = self.blocks.iter_mut().find_map(|block| match block {
                    Block::Terminal(card) if card.id == id => Some(card),
                    _ => None,
                }) else {
                    return false;
                };
                card.screen.set_size(rows.max(1), cols.max(1));
                true
            }
            AgentEvent::TerminalError { id, message } => {
                if self.terminal_view_id() == Some(id.as_str())
                    && self.terminal_view.phase == TerminalViewPhase::Attaching
                {
                    self.terminal_view.phase = TerminalViewPhase::Failed;
                }
                self.flash(message);
                true
            }
            AgentEvent::ModelChanged {
                id,
                display,
                context,
                cost_input,
                cost_output,
            } => {
                self.model = id;
                self.model_display = display;
                if context > 0 {
                    self.context_window = context;
                }
                self.cost_input = cost_input;
                self.cost_output = cost_output;
                true
            }
            AgentEvent::ModelsListed { models } => {
                let selected_model = self.palette.as_ref().and_then(|pal| {
                    pal.selected_model(&self.model_choices)
                        .map(|choice| (choice.connection_id.clone(), choice.key.clone()))
                });
                self.model_choices = models
                    .into_iter()
                    .map(|m| ModelChoice {
                        key: m.id,
                        display: m.name,
                        detail: m.detail,
                        group: m.group,
                        connection_id: m.connection_id,
                        vision: m.vision,
                        context: m.context,
                        cost_input: m.cost_input,
                        cost_output: m.cost_output,
                    })
                    .collect();
                self.models_catalog = ModelsCatalogState::Ready;
                // Apply context window / cost for the active model from the
                // freshly loaded catalog (so the footer shows the right value
                // without requiring a manual /model re-pick).
                if let Some(m) = self.model_choices.iter().find(|m| {
                    m.key == self.model
                        && (m.connection_id.is_empty() || m.connection_id == self.active_connection)
                }) {
                    if m.context > 0 {
                        self.context_window = m.context;
                        self.pending_context_update = Some(m.context);
                    }
                    self.cost_input = m.cost_input;
                    self.cost_output = m.cost_output;
                }
                if let Some(pal) = self.palette.as_mut() {
                    if pal.mode == palette::PaletteMode::Models {
                        let restored = selected_model.as_ref().and_then(|(connection, key)| {
                            pal.model_rows(&self.model_choices).iter().position(|row| {
                                matches!(
                                    row,
                                    palette::ModelRow::Model(choice)
                                        if choice.connection_id == connection.as_str()
                                            && choice.key == key.as_str()
                                )
                            })
                        });
                        if let Some(selected) = restored {
                            pal.selected = selected;
                        } else {
                            pal.clamp_selection(
                                &self.model_choices,
                                &self.connections,
                                &self.saved_sessions,
                            );
                        }
                    }
                }
                true
            }
            AgentEvent::ModelsListFailed(err) => {
                // A refresh failure must not hide either the last live catalog
                // or the per-provider models seeded from config at startup.
                if self.models_catalog == ModelsCatalogState::Ready
                    || !self.model_choices.is_empty()
                {
                    self.models_catalog = ModelsCatalogState::Ready;
                    if self
                        .palette
                        .as_ref()
                        .is_some_and(|pal| pal.mode == palette::PaletteMode::Models)
                    {
                        self.flash_error(format!("Couldn't refresh models: {err}"));
                    }
                } else {
                    self.models_catalog = ModelsCatalogState::Failed(err);
                }
                true
            }
            AgentEvent::ConnectionsUpdated { active, profiles } => {
                self.active_connection = active;
                self.connections = profiles;
                if let Some(pal) = self.palette.as_mut() {
                    if matches!(
                        pal.mode,
                        palette::PaletteMode::Connect
                            | palette::PaletteMode::ConnectKey { .. }
                            | palette::PaletteMode::EditConnectionKey
                    ) {
                        pal.clamp_selection(
                            &self.model_choices,
                            &self.connections,
                            &self.saved_sessions,
                        );
                    }
                }
                true
            }
            AgentEvent::GoalSet {
                objective,
                deadline,
            } => {
                self.goal = Some(goal::GoalStatus {
                    objective: objective.clone(),
                    deadline,
                    paused: false,
                    circle: 0,
                });
                self.blocks.push(Block::User(objective.clone()));
                self.blocks.push(Block::Goal(GoalCard {
                    objective,
                    deadline,
                }));
                self.running = true;
                self.turn_started_at = Some(std::time::Instant::now());
                self.scroll_from_bottom = 0;
                true
            }
            AgentEvent::GoalContinue {
                objective,
                remaining_secs,
            } => {
                if let Some(g) = self.goal.as_mut() {
                    g.objective = objective;
                    if let Some(d) = g.deadline {
                        let now = std::time::Instant::now();
                        g.deadline = Some(now + std::time::Duration::from_secs(remaining_secs));
                        let _ = d;
                    }
                    g.circle += 1;
                }
                // Treat like TurnFinished for UI state (stop spinner).
                self.running = false;
                self.close_thought();
                self.finalize_streaming();
                self.collapse_completed_tool_details();
                // "Circle N" summary instead of "Worked for Nm" during a goal loop.
                if let Some(g) = self.goal.as_ref() {
                    self.blocks.push(Block::GoalCircle(g.circle));
                    self.scroll_from_bottom = 0;
                }
                self.project.invalidate();
                self.refresh_project();
                true
            }
            AgentEvent::GoalExpired { objective } => {
                self.goal = None;
                self.running = false;
                self.close_thought();
                self.finalize_streaming();
                self.collapse_completed_tool_details();
                self.blocks
                    .push(Block::Notice(format!("Goal time expired: {objective}")));
                self.scroll_from_bottom = 0;
                self.project.invalidate();
                self.refresh_project();
                true
            }
            AgentEvent::GoalStopped => {
                self.goal = None;
                self.collapse_completed_tool_details();
                self.flash("Goal stopped");
                true
            }
            AgentEvent::GoalPaused => {
                if let Some(g) = self.goal.as_mut() {
                    g.paused = true;
                }
                // Pausing on its own looks like a dead end — say what stops it.
                self.flash("Goal paused — esc again to stop");
                true
            }
            AgentEvent::GoalResumed => {
                if let Some(g) = self.goal.as_mut() {
                    g.paused = false;
                }
                self.flash("Goal resumed");
                true
            }
            AgentEvent::Notice(s) => {
                if looks_like_error(&s) {
                    self.flash_error(s);
                } else {
                    self.flash(s);
                }
                true
            }
            AgentEvent::SessionsListed(metas) => {
                self.set_sessions_list(metas);
                true
            }
            AgentEvent::SessionLoaded {
                title,
                model,
                context_window,
                messages,
                usage,
            } => {
                self.new_chat();
                self.blocks.clear();
                let mut msg_count = 0usize;
                for m in &messages {
                    match m.role {
                        hive_core::Role::System => {}
                        hive_core::Role::User => {
                            let text = m.text();
                            if !text.is_empty() {
                                self.blocks.push(Block::User(text));
                                msg_count += 1;
                            }
                        }
                        hive_core::Role::Assistant => {
                            let text = m.text();
                            if !text.is_empty() {
                                self.blocks.push(Block::Assistant {
                                    text,
                                    streaming: false,
                                });
                                msg_count += 1;
                            }
                            for tc in &m.tool_calls {
                                let args = hive_core::tool_args_preview(&tc.name, &tc.arguments);
                                if !shows_generic_tool_card(&tc.name, &args) {
                                    continue;
                                }
                                self.blocks.push(Block::Tool(ToolCard {
                                    id: tc.id.clone(),
                                    name: tc.name.clone(),
                                    args,
                                    output: String::new(),
                                    status: ToolStatus::Ok,
                                    started: std::time::Instant::now(),
                                    // Durations aren't persisted — don't invent one.
                                    elapsed_ms: None,
                                    details_open: false,
                                    snapshot: None,
                                }));
                            }
                        }
                        hive_core::Role::Tool => {
                            // Match by call id, not position: one assistant
                            // message can carry several tool calls, so the
                            // last card is usually the wrong one.
                            let Some(call_id) = m.tool_call_id.as_deref() else {
                                continue;
                            };
                            if let Some(Block::Tool(card)) = self
                                .blocks
                                .iter_mut()
                                .rev()
                                .find(|b| matches!(b, Block::Tool(c) if c.id == call_id))
                            {
                                card.output = m.text();
                            }
                        }
                    }
                }
                if self.blocks.is_empty() {
                    self.blocks.push(Block::Welcome);
                }
                // Prefer the catalog's label and rates for the restored model;
                // the snapshot only stores the provider id.
                match self.model_choices.iter().find(|c| c.key == model) {
                    Some(choice) => {
                        self.model_display = choice.display.clone();
                        self.cost_input = choice.cost_input;
                        self.cost_output = choice.cost_output;
                    }
                    None => {
                        self.model_display = model.rsplit('/').next().unwrap_or(&model).to_string();
                    }
                }
                self.model = model;
                if context_window > 0 {
                    self.context_window = context_window;
                }
                self.usage = usage;
                // `context_tokens` is the last prompt size, not the session
                // total — the driver sends it right after this event.
                self.scroll_from_bottom = 0;
                self.flash(format!("Resumed: {title} ({msg_count} msgs)"));
                true
            }
            AgentEvent::TodosUpdated { items } => {
                if items.is_empty() {
                    self.clear_todos();
                    self.scroll_from_bottom = 0;
                    return true;
                }
                if let Some(Block::Todos(existing)) = self
                    .blocks
                    .iter_mut()
                    .rev()
                    .find(|b| matches!(b, Block::Todos(_)))
                {
                    *existing = items.clone();
                } else {
                    self.blocks.push(Block::Todos(items.clone()));
                }
                self.todos_completed_at = items
                    .iter()
                    .all(|item| item.done)
                    .then(std::time::Instant::now);
                self.todos = items;
                self.scroll_from_bottom = 0;
                true
            }
            AgentEvent::Error(s) => {
                self.flash_error(s.clone());
                self.blocks.push(Block::Error(s));
                true
            }
            AgentEvent::RecapDelta { id, chunk } => {
                let Some(card) = self.recap_card_mut(id) else {
                    return false;
                };
                if let RecapBody::Generating { text } = &mut card.recap {
                    text.push_str(&chunk);
                }
                self.recap_overlay
                    .as_ref()
                    .is_some_and(|o| o.recap_id == id && o.reveal_stream)
            }
            AgentEvent::RecapFinished { id, text } => {
                let Some(card) = self.recap_card_mut(id) else {
                    return false;
                };
                let body = text.trim();
                let fallback = match &card.recap {
                    RecapBody::Generating { text } => text.trim().to_string(),
                    _ => String::new(),
                };
                let final_text = if body.is_empty() {
                    fallback
                } else {
                    body.to_string()
                };
                card.recap = RecapBody::Ready { text: final_text };
                self.recap_overlay
                    .as_ref()
                    .is_some_and(|o| o.recap_id == id)
            }
            AgentEvent::RecapFailed { id, error } => {
                if let Some(card) = self.recap_card_mut(id) {
                    card.recap = RecapBody::Failed { error };
                }
                self.recap_overlay
                    .as_ref()
                    .is_some_and(|o| o.recap_id == id)
            }
            AgentEvent::TurnFinished => {
                self.running = false;
                self.close_thought();
                self.finalize_streaming();
                self.collapse_completed_tool_details();
                // "Worked for Nm" summary line at the end of the turn.
                if self.ui.show_work_summary {
                    if let Some(started) = self.turn_started_at.take() {
                        let secs = started.elapsed().as_secs();
                        let recap_id = self.next_recap_id;
                        self.next_recap_id = self.next_recap_id.saturating_add(1);
                        self.blocks.push(Block::WorkSummary(WorkSummaryCard {
                            secs,
                            recap_id,
                            recap: RecapBody::Idle,
                        }));
                        self.scroll_from_bottom = 0;
                    }
                }
                // Force a fresh git snapshot after the agent may have edited files.
                self.project.invalidate();
                self.refresh_project();
                true
            }
        }
    }
}

fn looks_like_error(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.contains("fail")
        || l.contains("error")
        || l.contains("couldn't")
        || l.contains("could not")
        || l.contains("invalid")
        || l.contains("can't")
        || l.contains("cannot")
}
