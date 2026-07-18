//! The TUI application state and how it reacts to `AgentEvent`s.

pub mod input;
pub mod state;

use std::collections::HashMap;

use comb::{Line, Rect};
use hive_core::event::{AgentEvent, SubagentLine, SubagentStatus};
use hive_core::message::ImageSource;
use hive_core::provider::Usage;
use hive_core::AgentMode;

use crate::commands;
use crate::render::mascot::LogoBonk;
use crate::render::spinner;
use crate::render::tools::parse_sections;
use crate::render::ProjectSnapshot;
use crate::theme::Theme;
use crate::TuiInit;

use input::InputState;
use state::{
    Block, ChatView, PlanCard, PlanStatus, PlanViewState, SubagentCard, Thought, ToolCard,
    ToolStatus,
};

/// Cached markdown wraps for finished assistant bodies — avoids re-parsing on
/// every spinner tick / subagent status pulse.
#[derive(Default)]
pub(crate) struct MdCache {
    width: usize,
    entries: HashMap<u64, Vec<Line>>,
}

impl MdCache {
    pub(crate) fn lines(
        &mut self,
        text: &str,
        width: usize,
        build: impl FnOnce() -> Vec<Line>,
    ) -> Vec<Line> {
        if width != self.width {
            self.entries.clear();
            self.width = width;
        }
        let key = fnv1a64(text.as_bytes());
        if let Some(cached) = self.entries.get(&key) {
            return cached.clone();
        }
        let lines = build();
        if self.entries.len() >= 64 {
            self.entries.clear();
        }
        self.entries.insert(key, lines.clone());
        lines
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub struct App {
    pub(crate) blocks: Vec<Block>,
    pub(crate) input: InputState,
    pub(crate) theme: Theme,
    /// Provider model id (kept for /model / vision checks).
    pub(crate) model: String,
    /// Pretty name shown in the footer.
    pub(crate) model_display: String,
    pub(crate) cwd: String,
    pub(crate) version: String,
    pub(crate) usage: Usage,
    pub(crate) running: bool,
    pub(crate) spinner: usize,
    pub(crate) scroll_from_bottom: usize,
    pub(crate) pending_images: Vec<ImageSource>,
    /// Selected row in the slash command menu.
    pub(crate) menu_index: usize,
    /// Short-lived status message shown in the footer (not the chat).
    pub(crate) flash_msg: Option<(String, std::time::Instant)>,
    /// Set on the first ctrl+c; a second press within the window quits.
    pub(crate) ctrl_c_armed: Option<std::time::Instant>,
    /// Animation clock: frames derive from elapsed time, so the spinner and
    /// shimmer run at a constant speed no matter how often events arrive.
    pub(crate) anim_start: std::time::Instant,
    /// Screen rows of expandable headers from the last draw: (row, block index).
    /// Rebuilt every frame; used to hit-test mouse clicks on thoughts/subagents.
    pub(crate) click_hits: Vec<(u16, usize)>,
    /// Landing-screen logo rect from the last draw (for click hit-testing).
    pub(crate) logo_hit: Option<Rect>,
    /// Active "bonk" ripple on the HIVE wordmark.
    pub(crate) logo_bonk: Option<LogoBonk>,
    /// Main transcript vs a read-only subagent / plan view.
    pub(crate) view: ChatView,
    /// Hit target for the `← back` control in subagent/plan view (last draw).
    pub(crate) back_hit: Option<Rect>,
    /// Hit target for the Build button in plan preview (last draw).
    pub(crate) build_hit: Option<Rect>,
    /// Hit target for the main input strip (last draw). Cleared in special views.
    pub(crate) input_hit: Option<Rect>,
    /// Whether the main input has keyboard focus (caret + typing).
    pub(crate) input_focused: bool,
    /// BUILD / PLAN mode for the next user turn.
    pub(crate) agent_mode: AgentMode,
    /// Plan preview selection / amend state.
    pub(crate) plan_view: PlanViewState,
    /// Finished-assistant markdown cache (invalidated on width change).
    pub(crate) md_cache: MdCache,
    /// Cached git project / diff summary for the right sidebar.
    pub(crate) project: ProjectSnapshot,
    /// Whether the wide-screen project sidebar is visible.
    pub(crate) sidebar_open: bool,
    /// Hit target for the sidebar show/hide control (last draw).
    pub(crate) sidebar_toggle_hit: Option<Rect>,
}

/// How long the ctrl+c confirmation window lives.
pub const FLASH_MS: u128 = 1500;
/// Bottom toast lifetime — short so it doesn't linger.
pub const TOAST_MS: u128 = 700;

impl App {
    pub fn new(init: TuiInit) -> Self {
        let theme = Theme::from_name(&init.theme);
        let mut app = App {
            blocks: Vec::new(),
            input: InputState::default(),
            theme,
            model_display: if init.model_display.is_empty() {
                init.model.rsplit('/').next().unwrap_or("model").to_string()
            } else {
                init.model_display
            },
            model: init.model,
            cwd: init.cwd,
            version: init.version,
            usage: Usage::default(),
            running: false,
            spinner: 0,
            scroll_from_bottom: 0,
            pending_images: Vec::new(),
            menu_index: 0,
            flash_msg: None,
            ctrl_c_armed: None,
            anim_start: std::time::Instant::now(),
            click_hits: Vec::new(),
            logo_hit: None,
            logo_bonk: None,
            view: ChatView::Main,
            back_hit: None,
            build_hit: None,
            input_hit: None,
            input_focused: true,
            agent_mode: AgentMode::Build,
            plan_view: PlanViewState::default(),
            md_cache: MdCache::default(),
            project: ProjectSnapshot::default(),
            sidebar_open: true,
            sidebar_toggle_hit: None,
        };
        app.blocks.push(Block::Welcome);
        app.project.refresh_if_stale(&app.cwd);
        app
    }

    /// Refresh git project snapshot when the cache is stale.
    pub fn refresh_project(&mut self) {
        self.project.refresh_if_stale(&self.cwd);
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_open = !self.sidebar_open;
    }

    /// True while viewing a subagent thread (read-only; no input).
    pub fn in_subagent_view(&self) -> bool {
        matches!(self.view, ChatView::Subagent(_))
    }

    /// True while previewing Plan.md.
    pub fn in_plan_view(&self) -> bool {
        matches!(self.view, ChatView::Plan)
    }

    /// True in any read-only special view (subagent or plan).
    pub fn in_special_view(&self) -> bool {
        self.in_subagent_view() || self.in_plan_view()
    }

    /// The subagent card currently being viewed, if any.
    pub fn viewed_subagent(&self) -> Option<&SubagentCard> {
        match &self.view {
            ChatView::Main | ChatView::Plan => None,
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

    /// Open the read-only chat view for a subagent card.
    pub fn open_subagent_view(&mut self, id: String) {
        self.view = ChatView::Subagent(id);
        self.scroll_from_bottom = 0;
        self.back_hit = None;
        self.build_hit = None;
        self.input_focused = false;
    }

    /// Open the Plan.md markdown preview.
    pub fn open_plan_view(&mut self) {
        let body = self.plan_card().map(|c| c.body.clone()).unwrap_or_default();
        self.plan_view = PlanViewState {
            sections: parse_sections(&body),
            cursor: 0,
            selected: Default::default(),
            amending: false,
        };
        self.view = ChatView::Plan;
        self.scroll_from_bottom = 0;
        self.back_hit = None;
        self.build_hit = None;
        self.input_focused = false;
        let _ = self.input.take();
    }

    /// Return to the main agent transcript.
    pub fn leave_special_view(&mut self) {
        self.view = ChatView::Main;
        self.scroll_from_bottom = 0;
        self.back_hit = None;
        self.build_hit = None;
        self.plan_view = PlanViewState::default();
        let _ = self.input.take();
        self.input_focused = true;
    }

    /// Return to the main agent transcript.
    pub fn leave_subagent_view(&mut self) {
        self.leave_special_view();
    }

    pub fn toggle_agent_mode(&mut self) {
        self.agent_mode = self.agent_mode.toggle();
        // Mode is visible on the BUILD/PLAN chip — no toast.
    }

    /// Toggle selection of the section under the plan cursor.
    pub fn plan_toggle_select(&mut self) {
        if self.plan_view.sections.is_empty() {
            return;
        }
        let i = self.plan_view.cursor;
        if self.plan_view.selected.contains(&i) {
            self.plan_view.selected.remove(&i);
        } else {
            self.plan_view.selected.insert(i);
        }
        self.plan_view.amending = !self.plan_view.selected.is_empty();
        if self.plan_view.amending {
            self.input_focused = true;
        } else {
            self.input_focused = false;
            let _ = self.input.take();
        }
    }

    pub fn plan_cursor_up(&mut self) {
        if self.plan_view.cursor > 0 {
            self.plan_view.cursor -= 1;
        }
    }

    pub fn plan_cursor_down(&mut self) {
        if self.plan_view.cursor + 1 < self.plan_view.sections.len() {
            self.plan_view.cursor += 1;
        }
    }

    /// Build the amend user message from selected sections + composer text.
    pub fn plan_amend_message(&self, notes: &str) -> String {
        let titles: Vec<&str> = self
            .plan_view
            .selected
            .iter()
            .filter_map(|i| self.plan_view.sections.get(*i).map(|s| s.title.as_str()))
            .collect();
        let list = if titles.is_empty() {
            "(whole plan)".to_string()
        } else {
            titles
                .iter()
                .map(|t| format!("- {t}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "Revise the plan in `.hive/Plan.md`.\n\nSelected sections:\n{list}\n\nUser notes:\n{notes}"
        )
    }

    pub fn focus_input(&mut self) {
        self.input_focused = true;
    }

    pub fn blur_input(&mut self) {
        self.input_focused = false;
    }

    /// True when `(col, row)` lands inside the input strip from the last draw.
    pub fn input_contains(&self, col: u16, row: u16) -> bool {
        self.input_hit
            .map(|r| r.contains(col, row))
            .unwrap_or(false)
    }

    pub fn spinner_char(&self) -> &'static str {
        spinner::frame(self.spinner)
    }

    /// Advance the animation frame from wall-clock time. Called every loop
    /// iteration; mouse/key event bursts don't speed the animation up because
    /// the frame is a pure function of elapsed time.
    pub fn tick(&mut self) {
        self.spinner = (self.anim_start.elapsed().as_millis() / 100) as usize;
        if self.logo_bonk.as_ref().is_some_and(|b| !b.alive()) {
            self.logo_bonk = None;
        }
    }

    /// True when the frame should keep painting (spinner / shimmer / bonk / flash).
    pub fn needs_animation(&self) -> bool {
        if self.logo_bonk.is_some() || self.flash_text().is_some() {
            return true;
        }
        if self.running {
            return true;
        }
        self.blocks.iter().any(|b| match b {
            Block::Tool(c) => c.status == ToolStatus::Running,
            Block::Subagent(c) => c.status == SubagentStatus::Running,
            Block::Plan(c) => c.status == PlanStatus::Writing,
            Block::Reasoning(th) => th.elapsed_ms.is_none(),
            Block::Assistant {
                streaming: true, ..
            } => true,
            _ => false,
        })
    }

    /// Start a logo bonk ripple at a screen cell (landing only).
    pub fn bonk_logo_at(&mut self, col: u16, row: u16) -> bool {
        let Some(logo) = self.logo_hit else {
            return false;
        };
        let Some((lx, ly)) = crate::render::mascot::hit_cell(logo, col, row) else {
            return false;
        };
        self.logo_bonk = Some(LogoBonk::fresh(lx, ly));
        crate::sound::play_bonk();
        true
    }

    /// True when nothing has happened yet (only the welcome block) and no turn
    /// is running — the landing screen with a centered input.
    pub fn is_empty_chat(&self) -> bool {
        !self.running && !self.blocks.iter().any(|b| !matches!(b, Block::Welcome))
    }

    // --- ephemeral footer messages & double-ctrl+c ---

    pub fn flash(&mut self, msg: impl Into<String>) {
        self.flash_msg = Some((msg.into(), std::time::Instant::now()));
    }

    /// The flash text, if it hasn't expired yet.
    pub fn flash_text(&self) -> Option<&str> {
        match &self.flash_msg {
            Some((msg, at)) if at.elapsed().as_millis() < TOAST_MS => Some(msg.as_str()),
            _ => None,
        }
    }

    /// First press arms; a second within the window confirms the quit.
    pub fn arm_or_confirm_quit(&mut self) -> bool {
        if let Some(at) = self.ctrl_c_armed {
            if at.elapsed().as_millis() < FLASH_MS {
                return true;
            }
        }
        self.ctrl_c_armed = Some(std::time::Instant::now());
        self.flash("Ctrl+C again to quit");
        false
    }

    pub fn disarm_quit(&mut self) {
        self.ctrl_c_armed = None;
    }

    /// The last finished assistant answer, for copying.
    pub fn last_answer(&self) -> Option<&str> {
        self.blocks.iter().rev().find_map(|b| match b {
            Block::Assistant { text, streaming } if !streaming && !text.trim().is_empty() => {
                Some(text.as_str())
            }
            _ => None,
        })
    }

    // --- slash command menu ---

    /// The prefix typed after `/`, if the menu should be open: input starts
    /// with `/`, is a single line, and has no space yet.
    pub fn slash_prefix(&self) -> Option<&str> {
        let v = self.input.value.as_str();
        let rest = v.strip_prefix('/')?;
        if rest.contains(' ') || rest.contains('\n') {
            return None;
        }
        Some(rest)
    }

    pub fn menu_items(&self) -> Vec<&'static commands::SlashCmd> {
        match self.slash_prefix() {
            Some(prefix) => commands::filtered(prefix),
            None => Vec::new(),
        }
    }

    pub fn menu_up(&mut self) {
        let n = self.menu_items().len();
        if n > 0 {
            self.menu_index = (self.menu_index + n - 1) % n;
        }
    }

    pub fn menu_down(&mut self) {
        let n = self.menu_items().len();
        if n > 0 {
            self.menu_index = (self.menu_index + 1) % n;
        }
    }

    pub fn menu_selected(&self) -> Option<&'static commands::SlashCmd> {
        let items = self.menu_items();
        if items.is_empty() {
            return None;
        }
        Some(items[self.menu_index.min(items.len() - 1)])
    }

    pub fn reset_menu(&mut self) {
        self.menu_index = 0;
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_add(n);
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(n);
    }

    pub fn push_user(&mut self, text: String) {
        self.blocks.push(Block::User(text));
        self.scroll_from_bottom = 0;
    }

    pub fn notice(&mut self, text: impl Into<String>) {
        self.blocks.push(Block::Notice(text.into()));
    }

    /// Start a fresh chat — back to the centered Home/landing screen.
    pub fn new_chat(&mut self) {
        self.blocks.clear();
        self.blocks.push(Block::Welcome);
        self.usage = Usage::default();
        self.scroll_from_bottom = 0;
        self.pending_images.clear();
        self.input.clear();
        self.reset_menu();
        self.running = false;
        self.click_hits.clear();
        self.view = ChatView::Main;
        self.back_hit = None;
        self.build_hit = None;
        self.plan_view = PlanViewState::default();
        self.md_cache = MdCache::default();
    }

    pub fn add_pending_image(&mut self, src: ImageSource) {
        self.pending_images.push(src);
    }

    pub fn take_pending_images(&mut self) -> Vec<ImageSource> {
        std::mem::take(&mut self.pending_images)
    }

    /// Apply an agent event. Returns `true` when the visible UI should redraw
    /// (background subagent transcript updates while on the main chat do not).
    pub fn apply(&mut self, ev: AgentEvent) -> bool {
        match ev {
            AgentEvent::TurnStarted => {
                self.running = true;
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
                if (name == "write_file" || name == "edit_file")
                    && (args_preview.contains("Plan.md") || args_preview.contains(".hive/Plan"))
                {
                    self.mark_plan_writing();
                }
                // Subagent tools render as Subagent cards, not tool rows.
                if !is_subagent_tool(&name) {
                    self.blocks.push(Block::Tool(ToolCard {
                        id,
                        name,
                        args: args_preview,
                        output: String::new(),
                        status: ToolStatus::Running,
                        started: std::time::Instant::now(),
                        elapsed_ms: None,
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
            AgentEvent::Usage(u) => {
                self.usage = u;
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
            AgentEvent::ModelChanged { id, display } => {
                self.model = id;
                self.model_display = display;
                true
            }
            AgentEvent::Notice(s) => {
                self.blocks.push(Block::Notice(s));
                true
            }
            AgentEvent::Error(s) => {
                self.blocks.push(Block::Error(s));
                true
            }
            AgentEvent::TurnFinished => {
                self.running = false;
                self.close_thought();
                self.finalize_streaming();
                // Force a fresh git snapshot after the agent may have edited files.
                self.project.invalidate();
                self.refresh_project();
                true
            }
        }
    }

    fn append_assistant(&mut self, t: &str) {
        self.close_thought();
        if let Some(Block::Assistant { text, streaming }) = self.blocks.last_mut() {
            if *streaming {
                text.push_str(t);
                return;
            }
        }
        self.blocks.push(Block::Assistant {
            text: t.to_string(),
            streaming: true,
        });
    }

    fn append_reasoning(&mut self, t: &str) {
        if let Some(Block::Reasoning(th)) = self.blocks.last_mut() {
            if th.elapsed_ms.is_none() {
                th.text.push_str(t);
                return;
            }
        }
        let mut th = Thought::new();
        th.text.push_str(t);
        self.blocks.push(Block::Reasoning(th));
    }

    /// Stamp the duration on the most recent still-open thought, if any.
    fn close_thought(&mut self) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Reasoning(th) = block {
                if th.elapsed_ms.is_none() {
                    th.elapsed_ms = Some(th.started.elapsed().as_millis());
                }
                return;
            }
        }
    }

    /// ctrl+t: if any thought is open, close them all; otherwise open them all.
    pub fn toggle_thoughts(&mut self) {
        let any_open = self.blocks.iter().any(|b| match b {
            Block::Reasoning(th) => th.open,
            _ => false,
        });
        for block in self.blocks.iter_mut() {
            if let Block::Reasoning(th) = block {
                th.open = !any_open;
            }
        }
        self.flash(if any_open {
            "Thoughts hidden"
        } else {
            "Thoughts shown"
        });
    }

    /// Activate an expandable header by index — thoughts toggle; subagents /
    /// plan cards open their dedicated views.
    pub fn activate_expandable_at(&mut self, block_idx: usize) {
        match self.blocks.get(block_idx) {
            Some(Block::Reasoning(_)) => {
                if let Some(Block::Reasoning(th)) = self.blocks.get_mut(block_idx) {
                    th.open = !th.open;
                }
            }
            Some(Block::Subagent(card)) => {
                let id = card.id.clone();
                self.open_subagent_view(id);
            }
            Some(Block::Plan(_)) => {
                self.open_plan_view();
            }
            _ => {}
        }
    }

    fn mark_plan_writing(&mut self) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Plan(card) = block {
                card.status = PlanStatus::Writing;
                card.elapsed_ms = None;
                return;
            }
        }
        self.blocks.push(Block::Plan(PlanCard {
            summary: String::new(),
            body: String::new(),
            status: PlanStatus::Writing,
            started: std::time::Instant::now(),
            elapsed_ms: None,
        }));
    }

    fn upsert_plan(&mut self, summary: String, body: String) {
        let sections = parse_sections(&body);
        for block in self.blocks.iter_mut().rev() {
            if let Block::Plan(card) = block {
                card.summary = summary;
                card.body = body;
                card.status = PlanStatus::Ready;
                if card.elapsed_ms.is_none() {
                    card.elapsed_ms = Some(card.started.elapsed().as_millis());
                }
                if matches!(self.view, ChatView::Plan) {
                    let selected = self.plan_view.selected.clone();
                    let cursor = self.plan_view.cursor.min(sections.len().saturating_sub(1));
                    self.plan_view.sections = sections;
                    self.plan_view.cursor = cursor;
                    self.plan_view.selected = selected
                        .into_iter()
                        .filter(|i| *i < self.plan_view.sections.len())
                        .collect();
                }
                return;
            }
        }
        self.blocks.push(Block::Plan(PlanCard {
            summary,
            body,
            status: PlanStatus::Ready,
            started: std::time::Instant::now(),
            elapsed_ms: Some(0),
        }));
        if matches!(self.view, ChatView::Plan) {
            self.plan_view.sections = sections;
        }
    }

    /// The expandable header (if any) drawn on this screen row in the last frame.
    pub fn expandable_at_row(&self, row: u16) -> Option<usize> {
        self.click_hits
            .iter()
            .find(|(r, _)| *r == row)
            .map(|(_, idx)| *idx)
    }

    fn finalize_assistant(&mut self, final_text: String) {
        self.close_thought();
        for block in self.blocks.iter_mut().rev() {
            if let Block::Assistant { text, streaming } = block {
                if *streaming {
                    *text = final_text;
                    *streaming = false;
                    return;
                }
            }
        }
        self.blocks.push(Block::Assistant {
            text: final_text,
            streaming: false,
        });
    }

    fn finalize_streaming(&mut self) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Assistant { streaming, .. } = block {
                if *streaming {
                    *streaming = false;
                    return;
                }
            }
        }
    }

    fn append_tool_output(&mut self, id: &str, chunk: &str) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Tool(card) = block {
                if card.id == id {
                    card.output.push_str(chunk);
                    const CAP: usize = 8000;
                    if card.output.len() > CAP {
                        let start = card.output.len() - CAP;
                        card.output = card.output.split_off(start);
                    }
                    return;
                }
            }
        }
    }

    fn finish_tool(&mut self, id: &str, ok: bool) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Tool(card) = block {
                if card.id == id {
                    card.status = if ok { ToolStatus::Ok } else { ToolStatus::Err };
                    if card.elapsed_ms.is_none() {
                        card.elapsed_ms = Some(card.started.elapsed().as_millis());
                    }
                    return;
                }
            }
        }
    }

    fn update_subagent(&mut self, id: &str, status: SubagentStatus, detail: String) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Subagent(card) = block {
                if card.id == id {
                    card.status = status;
                    if !detail.is_empty() {
                        card.detail = detail;
                    }
                    if status != SubagentStatus::Running && card.elapsed_ms.is_none() {
                        card.elapsed_ms = Some(card.started.elapsed().as_millis());
                    }
                    return;
                }
            }
        }
    }

    fn update_subagent_usage(&mut self, id: &str, usage: Usage) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Subagent(card) = block {
                if card.id == id {
                    card.usage = usage;
                    return;
                }
            }
        }
    }

    /// Subagent cards for the sidebar (running first, then recent finished).
    pub fn sidebar_subagents(&self) -> Vec<&SubagentCard> {
        let mut cards: Vec<&SubagentCard> = self
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Subagent(c) => Some(c),
                _ => None,
            })
            .collect();
        cards.sort_by_key(|c| match c.status {
            SubagentStatus::Running => 0u8,
            SubagentStatus::Failed => 1,
            SubagentStatus::Done => 2,
        });
        cards
    }

    fn append_subagent_line(&mut self, id: &str, line: SubagentLine) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Subagent(card) = block {
                if card.id != id {
                    continue;
                }
                match line {
                    SubagentLine::Thinking(t) => {
                        if let Some(SubagentLine::Thinking(prev)) = card.lines.last_mut() {
                            prev.push_str(&t);
                        } else {
                            card.lines.push(SubagentLine::Thinking(t));
                        }
                    }
                    SubagentLine::Tool {
                        name,
                        detail,
                        ok: Some(ok),
                    } => {
                        // Prefer updating the matching in-flight tool row.
                        let open = card.lines.iter_mut().rev().find(|l| {
                            matches!(
                                l,
                                SubagentLine::Tool { ok: None, name: n, .. } if *n == name
                            )
                        });
                        if let Some(SubagentLine::Tool {
                            detail: d,
                            ok: running,
                            ..
                        }) = open
                        {
                            if !detail.is_empty() {
                                *d = detail;
                            }
                            *running = Some(ok);
                        } else {
                            card.lines.push(SubagentLine::Tool {
                                name,
                                detail,
                                ok: Some(ok),
                            });
                        }
                    }
                    other => card.lines.push(other),
                }
                return;
            }
        }
    }
}

/// Tools whose activity belongs in a Subagent transcript card, not a tool row.
fn is_subagent_tool(name: &str) -> bool {
    matches!(name, "verify_project" | "spawn_subagent" | "spawn_swarm")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TuiInit;
    use hive_core::event::{AgentEvent, SubagentLine, SubagentStatus};

    fn app() -> App {
        App::new(TuiInit {
            model: "m".into(),
            model_display: "m".into(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
        })
    }

    #[test]
    fn subagent_transcript_does_not_dirty_main_view() {
        let mut a = app();
        assert!(a.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking".into(),
            prompt: "go".into(),
        }));
        // Body updates are stored but not painted on the main chat.
        assert!(!a.apply(AgentEvent::SubagentTranscript {
            id: "v1".into(),
            line: SubagentLine::Thinking("hmm".into()),
        }));
        // Status line is visible on the card.
        assert!(a.apply(AgentEvent::SubagentStatus {
            id: "v1".into(),
            status: SubagentStatus::Running,
            detail: "$ cargo check".into(),
        }));
    }

    #[test]
    fn subagent_transcript_dirties_open_subagent_view() {
        let mut a = app();
        a.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking".into(),
            prompt: "go".into(),
        });
        a.open_subagent_view("v1".into());
        assert!(a.apply(AgentEvent::SubagentTranscript {
            id: "v1".into(),
            line: SubagentLine::Thinking("hmm".into()),
        }));
        // A different subagent's transcript shouldn't force a redraw.
        assert!(!a.apply(AgentEvent::SubagentTranscript {
            id: "other".into(),
            line: SubagentLine::Thinking("nope".into()),
        }));
    }

    #[test]
    fn needs_animation_while_subagent_running() {
        let mut a = app();
        a.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking".into(),
            prompt: "go".into(),
        });
        assert!(a.needs_animation());
        a.apply(AgentEvent::SubagentStatus {
            id: "v1".into(),
            status: SubagentStatus::Done,
            detail: "done".into(),
        });
        assert!(!a.needs_animation());
    }

    #[test]
    fn subagent_usage_updates_card_and_sidebar_list() {
        let mut a = app();
        a.apply(AgentEvent::SubagentSpawned {
            id: "v1".into(),
            label: "Checking".into(),
            prompt: "go".into(),
        });
        assert!(a.apply(AgentEvent::SubagentUsage {
            id: "v1".into(),
            usage: Usage {
                prompt_tokens: 20,
                completion_tokens: 10,
                total_tokens: 30,
            },
        }));
        let cards = a.sidebar_subagents();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].usage.total_tokens, 30);
    }

    #[test]
    fn tab_toggles_agent_mode() {
        use hive_core::AgentMode;

        let mut a = app();
        assert_eq!(a.agent_mode, AgentMode::Build);
        a.toggle_agent_mode();
        assert_eq!(a.agent_mode, AgentMode::Plan);
        a.toggle_agent_mode();
        assert_eq!(a.agent_mode, AgentMode::Build);
    }

    #[test]
    fn plan_amend_message_lists_sections() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::PlanUpdated {
            summary: "A".into(),
            body: "## Alpha\n\nx\n\n## Beta\n\ny\n".into(),
        });
        a.open_plan_view();
        a.plan_view.selected.insert(0);
        let msg = a.plan_amend_message("make it shorter");
        assert!(msg.contains("Alpha"), "{msg}");
        assert!(msg.contains("make it shorter"), "{msg}");
        assert!(msg.contains(".hive/Plan.md"), "{msg}");
    }
}
