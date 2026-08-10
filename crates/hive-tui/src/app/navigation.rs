//! Project, view, sidebar, and navigation state methods.

use super::*;

impl App {
    /// Request stale project data and apply a completed background refresh.
    /// Returns true only when visible data changed.
    pub fn refresh_project(&mut self) -> bool {
        if self.project.stale() {
            self.project_refresh.request(&self.cwd);
        }
        let Some(result) = self.project_refresh.take_latest() else {
            return false;
        };
        if result.cwd != self.cwd {
            return false;
        }
        let context_changed = self.context_files != result.context_files;
        self.context_files = result.context_files;
        if !result.snapshot.available && self.project.available {
            // A transient Git failure must not erase a previously useful
            // snapshot. Keep it quietly and retry on the normal cadence.
            self.project.mark_refreshed();
            return context_changed;
        }
        let changed = self.project.name != result.snapshot.name
            || self.project.branch != result.snapshot.branch
            || self.project.files != result.snapshot.files
            || self.project.available != result.snapshot.available
            || context_changed;
        self.project = result.snapshot;
        changed
    }

    pub fn toggle_sidebar(&mut self) {
        match self.ui.sidebar_mode {
            SidebarMode::Pinned => self.flash("Sidebar pinned — /settings to change"),
            SidebarMode::Hidden => self.flash("Sidebar hidden — /settings to change"),
            SidebarMode::Auto => self.sidebar_open = !self.sidebar_open,
        }
    }

    /// Toggle a collapsible sidebar section (Context / Sub agents / Terminals / Changes).
    pub fn toggle_sidebar_section(&mut self, section: SidebarSection) {
        if !self.ui.sidebar_collapse_sections {
            return;
        }
        self.sidebar_sections.toggle(section);
    }

    pub fn open_settings(&mut self) {
        self.close_palette();
        self.close_about();
        self.settings = Some(settings::SettingsState::root());
    }

    pub fn close_settings(&mut self) {
        self.settings = None;
    }

    pub fn settings_open(&self) -> bool {
        self.settings.is_some()
    }

    // ── Goal overlay ────────────────────────────────────────────────────

    pub fn open_goal_overlay(&mut self) {
        self.close_palette();
        self.close_settings();
        self.close_about();
        self.goal_overlay = Some(goal::GoalOverlayState::new());
    }

    pub fn close_goal_overlay(&mut self) {
        self.goal_overlay = None;
    }

    pub fn goal_overlay_open(&self) -> bool {
        self.goal_overlay.is_some()
    }

    // ── Sessions picker ─────────────────────────────────────────────────

    pub fn open_sessions_picker(&mut self) {
        self.about_open = false;
        self.close_settings();
        self.saved_sessions.clear();
        self.palette = Some(palette::PaletteState::sessions());
    }

    pub fn set_sessions_list(&mut self, metas: Vec<hive_core::SessionMeta>) {
        self.saved_sessions = metas;
        if let Some(pal) = self.palette.as_mut() {
            if pal.mode == palette::PaletteMode::Sessions {
                pal.selected = 0;
                pal.list_offset = 0;
            }
        }
    }

    pub fn goal_active(&self) -> bool {
        self.goal.is_some()
    }

    pub fn goal_paused(&self) -> bool {
        self.goal.as_ref().is_some_and(|g| g.paused)
    }

    /// Apply in-memory UI prefs and ask the driver to persist them.
    pub fn persist_ui(
        &mut self,
        input_tx: &tokio::sync::mpsc::UnboundedSender<crate::InputCommand>,
    ) {
        self.apply_ui_prefs();
        self.invalidate_transcript();
        let _ = input_tx.send(crate::InputCommand::SaveUi(self.ui.clone()));
        self.flash("Settings saved");
    }

    pub(super) fn apply_ui_prefs(&mut self) {
        match self.ui.sidebar_mode {
            SidebarMode::Pinned => self.sidebar_open = true,
            SidebarMode::Hidden => self.sidebar_open = false,
            SidebarMode::Auto => {}
        }
        // Sync open state with the pref so turning it off collapses again.
        let open = self.ui.thoughts_always_open;
        for block in &mut self.blocks {
            if let Block::Reasoning(th) = block {
                th.open = open;
            }
        }
        if !self.ui.sidebar_collapse_sections {
            self.sidebar_sections = SidebarSections::default();
        }
    }

    /// True while viewing a subagent thread (read-only; no input).
    pub fn in_subagent_view(&self) -> bool {
        matches!(self.view, ChatView::Subagent(_))
    }

    /// True while previewing Plan.md.
    pub fn in_plan_view(&self) -> bool {
        matches!(self.view, ChatView::Plan)
    }

    pub fn in_terminal_view(&self) -> bool {
        matches!(self.view, ChatView::Terminal(_))
    }

    /// True in any special view (subagent, plan, or terminal).
    pub fn in_special_view(&self) -> bool {
        self.in_subagent_view() || self.in_plan_view() || self.in_terminal_view()
    }

    /// The subagent card currently being viewed, if any.
    pub fn viewed_subagent(&self) -> Option<&SubagentCard> {
        match &self.view {
            ChatView::Main | ChatView::Plan | ChatView::Terminal(_) => None,
            ChatView::Subagent(id) => self.blocks.iter().find_map(|b| match b {
                Block::Subagent(card) if card.id == *id => Some(card),
                _ => None,
            }),
        }
    }

    /// The plan card, if present in the transcript.
    pub fn plan_card(&self) -> Option<&PlanCard> {
        self.blocks.iter().rev().find_map(|b| match b {
            Block::Plan(card) => Some(card),
            _ => None,
        })
    }

    pub fn terminal_card(&self, id: &str) -> Option<&TerminalCard> {
        self.blocks.iter().find_map(|block| match block {
            Block::Terminal(card) if card.id == id => Some(card.as_ref()),
            _ => None,
        })
    }

    pub fn viewed_terminal(&self) -> Option<&TerminalCard> {
        let ChatView::Terminal(id) = &self.view else {
            return None;
        };
        self.terminal_card(id)
    }

    pub fn terminal_view_id(&self) -> Option<&str> {
        match &self.view {
            ChatView::Terminal(id) => Some(id),
            _ => None,
        }
    }

    /// Open the read-only chat view for a subagent card.
    pub fn open_subagent_view(&mut self, id: String) {
        self.clear_assistant_selection();
        self.view = ChatView::Subagent(id);
        self.scroll_from_bottom = 0;
        self.hover_block = None;
        self.hover_sidebar_item = None;
        self.hover_back = false;
        self.hover_make = false;
        self.back_hit = None;
        self.make_hit = None;
        self.blur_input();
    }

    /// Open the Plan.md markdown preview.
    pub fn open_plan_view(&mut self) {
        self.clear_assistant_selection();
        self.plan_view = PlanViewState::default();
        self.view = ChatView::Plan;
        // Pin to the top of the plan (chat uses bottom-pin; 0 would show the end).
        self.scroll_from_bottom = usize::MAX;
        self.hover_block = None;
        self.hover_sidebar_item = None;
        self.hover_back = false;
        self.hover_make = false;
        self.back_hit = None;
        self.make_hit = None;
        self.blur_input();
        let _ = self.input.take();
    }

    pub fn open_terminal_view(&mut self, id: String) {
        let (phase, last_size) = self
            .terminal_card(&id)
            .map(|card| {
                let phase = if !matches!(card.process, TerminalProcessState::Running) {
                    TerminalViewPhase::ReadOnly
                } else if card.controller == TerminalController::User {
                    TerminalViewPhase::UserControl
                } else {
                    TerminalViewPhase::Attaching
                };
                (phase, Some(card.screen.size()))
            })
            .unwrap_or((TerminalViewPhase::Failed, None));
        self.clear_assistant_selection();
        self.terminal_view = TerminalViewState {
            phase,
            body_rect: None,
            scrollback: 0,
            last_size,
        };
        self.view = ChatView::Terminal(id);
        self.scroll_from_bottom = 0;
        self.hover_block = None;
        self.hover_sidebar_item = None;
        self.hover_back = false;
        self.hover_make = false;
        self.hover_terminal_stop = false;
        self.back_hit = None;
        self.make_hit = None;
        self.terminal_stop_hit = None;
        self.pending_terminal_resize = None;
        self.pending_context_update = None;
        self.blur_input();
        let _ = self.input.take();
    }

    pub fn leave_terminal_view(&mut self) {
        self.leave_special_view();
    }

    pub fn terminal_input_modes(&self) -> Option<(bool, bool)> {
        let screen = &self.viewed_terminal()?.screen;
        Some((screen.application_cursor(), screen.bracketed_paste()))
    }

    pub fn scroll_terminal(&mut self, rows: isize) -> bool {
        let Some(id) = self.terminal_view_id().map(str::to_string) else {
            return false;
        };
        let current = self.terminal_view.scrollback;
        let requested = if rows >= 0 {
            current.saturating_add(rows as usize)
        } else {
            current.saturating_sub(rows.unsigned_abs())
        };
        let Some(card) = self.blocks.iter_mut().find_map(|block| match block {
            Block::Terminal(card) if card.id == id => Some(card.as_mut()),
            _ => None,
        }) else {
            return false;
        };
        card.screen.set_scrollback(requested);
        let actual = card.screen.scrollback();
        if actual == current {
            return false;
        }
        self.terminal_view.scrollback = actual;
        true
    }

    pub fn take_pending_terminal_resize(&mut self) -> Option<(String, u16, u16)> {
        self.pending_terminal_resize.take()
    }

    pub fn take_pending_context_update(&mut self) -> Option<u64> {
        self.pending_context_update.take()
    }

    /// Return to the main agent transcript.
    pub fn leave_special_view(&mut self) {
        self.clear_assistant_selection();
        self.view = ChatView::Main;
        self.scroll_from_bottom = 0;
        self.hover_block = None;
        self.hover_sidebar_item = None;
        self.hover_back = false;
        self.hover_make = false;
        self.hover_terminal_stop = false;
        self.back_hit = None;
        self.make_hit = None;
        self.terminal_stop_hit = None;
        self.plan_view = PlanViewState::default();
        self.terminal_view = TerminalViewState::default();
        self.pending_terminal_resize = None;
        let _ = self.input.take();
        self.focus_input();
    }

    /// Update hover highlight; returns whether the UI should redraw.
    pub fn set_hover_block(&mut self, idx: Option<usize>) -> bool {
        if self.hover_block == idx {
            return false;
        }
        self.hover_block = idx;
        true
    }

    /// Update sidebar row hover; returns whether the UI should redraw.
    pub fn set_hover_sidebar_item(&mut self, item: Option<SidebarItem>) -> bool {
        if self.hover_sidebar_item == item {
            return false;
        }
        self.hover_sidebar_item = item;
        true
    }

    /// Update `← back` hover; returns whether the UI should redraw.
    pub fn set_hover_back(&mut self, on: bool) -> bool {
        if self.hover_back == on {
            return false;
        }
        self.hover_back = on;
        true
    }

    /// Update MAKE hover; returns whether the UI should redraw.
    pub fn set_hover_make(&mut self, on: bool) -> bool {
        if self.hover_make == on {
            return false;
        }
        self.hover_make = on;
        true
    }

    pub fn set_hover_scroll_bottom(&mut self, on: bool) -> bool {
        if self.hover_scroll_bottom == on {
            return false;
        }
        self.hover_scroll_bottom = on;
        true
    }

    /// True when the pointer is over the jump-to-bottom chip.
    pub fn scroll_bottom_contains(&self, col: u16, row: u16) -> bool {
        self.scroll_bottom_hit
            .is_some_and(|hit| hit.contains(col, row))
    }

    pub fn set_hover_terminal_stop(&mut self, on: bool) -> bool {
        if self.hover_terminal_stop == on {
            return false;
        }
        self.hover_terminal_stop = on;
        true
    }

    /// Return to the main agent transcript.
    pub fn leave_subagent_view(&mut self) {
        self.leave_special_view();
    }

    pub fn toggle_agent_mode(&mut self) {
        self.agent_mode = self.agent_mode.toggle();
        // Mode is visible on the MAKE/PLAN chip — no toast.
    }
}
