//! The TUI application state and how it reacts to `AgentEvent`s.

pub mod files;
pub mod input;
pub mod palette;
pub mod settings;
pub mod state;

use std::collections::HashMap;

use comb::{Line, Rect};
use hive_core::event::{AgentEvent, ConnectionInfo, SubagentLine, SubagentStatus};
use hive_core::message::ImageSource;
use hive_core::provider::Usage;
use hive_core::{AgentMode, SidebarMode, TerminalController, TerminalProcessState, UiConfig};

use crate::commands;
use crate::render::spinner;
use crate::render::wordmark::LogoBonk;
use crate::render::{ProjectSnapshot, SidebarItem, SidebarSection, SidebarSections};
use crate::theme::Theme;
use crate::{ModelChoice, SkillChoice, TuiInit};

/// One row in the composer `/` menu: built-in command or skill.
#[derive(Debug, Clone)]
pub enum SlashItem {
    Command(&'static commands::CommandDef),
    Skill(SkillChoice),
}

impl SlashItem {
    pub fn name(&self) -> &str {
        match self {
            SlashItem::Command(c) => c.name,
            SlashItem::Skill(s) => s.name.as_str(),
        }
    }

    pub fn desc(&self) -> &str {
        match self {
            SlashItem::Command(c) => c.desc,
            SlashItem::Skill(s) => s.description.as_str(),
        }
    }

    pub fn hint(&self) -> &str {
        match self {
            SlashItem::Command(c) => c.hint,
            SlashItem::Skill(_) => "skill",
        }
    }

    pub fn takes_arg(&self) -> bool {
        match self {
            SlashItem::Command(c) => c.takes_arg,
            // Optional note after the skill name is typed manually; menu Enter runs now.
            SlashItem::Skill(_) => false,
        }
    }
}

use files::AtQuery;
use palette::PaletteState;
use settings::SettingsState;

use input::InputState;
use state::{
    AssistantPoint, AssistantResponseRow, AssistantRowHit, AssistantRowJoin, AssistantSelection,
    Block, ChatView, ContextAction, ContextMenu, ContextMenuItem, FileSnapshot, ModeSwitchCard,
    PlanAction, PlanCard, PlanCorrection, PlanStatus, PlanViewState, PromptHistory, SubagentCard,
    TerminalCard, TerminalViewPhase, TerminalViewState, Thought, ToolCard, ToolStatus,
};

/// Cached markdown wraps for finished assistant bodies — avoids re-parsing on
/// every spinner tick / subagent status pulse.
#[derive(Clone)]
pub(crate) struct MdRows {
    pub lines: Vec<Line>,
    pub joins: Vec<AssistantRowJoin>,
}

#[derive(Default)]
pub(crate) struct MdCache {
    width: usize,
    entries: HashMap<u64, MdRows>,
}

impl MdCache {
    pub(crate) fn rows(
        &mut self,
        text: &str,
        width: usize,
        build: impl FnOnce() -> MdRows,
    ) -> MdRows {
        self.cached(text, width, 0x51ec_7100_0000_0001, build)
    }

    pub(crate) fn lines(
        &mut self,
        text: &str,
        width: usize,
        build: impl FnOnce() -> Vec<Line>,
    ) -> Vec<Line> {
        self.cached(text, width, 0x51ec_7100_0000_0002, || {
            let lines = build();
            MdRows {
                joins: vec![AssistantRowJoin::Hard; lines.len()],
                lines,
            }
        })
        .lines
    }

    fn cached(
        &mut self,
        text: &str,
        width: usize,
        salt: u64,
        build: impl FnOnce() -> MdRows,
    ) -> MdRows {
        if width != self.width {
            self.entries.clear();
            self.width = width;
        }
        let key = fnv1a64(text.as_bytes()) ^ salt;
        if let Some(cached) = self.entries.get(&key) {
            return cached.clone();
        }
        let rows = build();
        // Evict a random-ish entry instead of wiping the whole cache — avoids
        // periodic re-render storms in long conversations.
        if self.entries.len() >= 64 {
            if let Some(first_key) = self.entries.keys().next().copied() {
                self.entries.remove(&first_key);
            }
        }
        self.entries.insert(key, rows.clone());
        rows
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

/// A file or image queued for the next turn, shown as a composer `@chip`.
#[derive(Clone)]
pub struct PendingAttach {
    /// Path shown after `@` (project-relative when possible).
    pub label: String,
    /// Path sent to the agent (`[Attached file: …]`).
    pub path: String,
    /// Set for image attachments (vision path unchanged).
    pub image: Option<ImageSource>,
}

/// Follow-up typed while a turn is running — sent after `TurnFinished`.
#[derive(Clone)]
pub struct QueuedFollowUp {
    /// Text shown in the transcript when flushed.
    pub display: String,
    /// Payload for the agent (may include `[Attached file: …]` notes).
    pub text: String,
    /// Composer text restored on ↑ (without `@chip` prefix).
    pub composer: String,
    /// Attachments restored on ↑ / re-queued on Enter.
    pub attaches: Vec<PendingAttach>,
    pub mode: AgentMode,
}

/// Deferred user message: shown in chat but not yet sent to the driver.
/// Gives an ESC-recall window before the agent starts processing.
pub struct PendingDispatch {
    /// Payload for the agent (may include `[Attached file: …]` notes).
    pub agent_text: String,
    pub images: Vec<ImageSource>,
    pub mode: AgentMode,
    /// Original composer text (for restoring to the input on recall).
    pub composer: String,
    /// Attachments (for restoring on recall).
    pub attaches: Vec<PendingAttach>,
    pub submitted_at: std::time::Instant,
}

/// Grace period before a pending dispatch is flushed to the driver (ms).
/// Long enough for a reflexive ESC, short enough to not feel slow.
pub const DISPATCH_GRACE_MS: u128 = 500;

impl PendingAttach {
    /// Composer / transcript chip, e.g. `@src/main.rs`.
    pub fn tag(&self) -> String {
        format!("@{}", self.label)
    }
}

/// Live `/model` catalog fetch state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelsCatalogState {
    Idle,
    Loading,
    Ready,
    Failed(String),
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
    /// Tokens in the latest prompt (context fill).
    pub(crate) context_tokens: u64,
    /// Configured context window size.
    pub(crate) context_window: u64,
    /// USD per 1M input tokens for the active model.
    pub(crate) cost_input: f64,
    /// USD per 1M output tokens for the active model.
    pub(crate) cost_output: f64,
    pub(crate) running: bool,
    pub(crate) spinner: usize,
    pub(crate) scroll_from_bottom: usize,
    /// Max scroll offset from the last transcript paint (`total - viewport`).
    /// Used to clamp scroll and to decide whether blurred arrows can move the view.
    pub(crate) transcript_max_scroll: usize,
    /// Files / images queued for the next user message (shown as tags).
    pub(crate) pending_attaches: Vec<PendingAttach>,
    /// Follow-up waiting for the current turn to finish.
    pub(crate) follow_up: Option<QueuedFollowUp>,
    /// Up/down arrow history of submitted prompts.
    pub(crate) prompt_history: PromptHistory,
    /// Deferred dispatch: message shown in chat but not yet sent to the driver.
    pub(crate) pending_dispatch: Option<PendingDispatch>,
    /// Models offered in the Switch-model picker (live catalog when Ready).
    pub(crate) model_choices: Vec<ModelChoice>,
    /// Skills for the `/` menu (`/skill-name`).
    pub(crate) skills: Vec<SkillChoice>,
    /// Fetch status for the live model catalog.
    pub(crate) models_catalog: ModelsCatalogState,
    /// Saved `/connect` provider profiles.
    pub(crate) connections: Vec<ConnectionInfo>,
    /// Active connection profile id.
    pub(crate) active_connection: String,
    /// Ctrl+P command palette / model picker.
    pub(crate) palette: Option<PaletteState>,
    /// Centered About overlay (HIVE wordmark + version + tagline).
    pub(crate) about_open: bool,
    /// Configure chat / sidebar overlay.
    pub(crate) settings: Option<SettingsState>,
    /// Persisted UI prefs (`[ui]` in config.toml).
    pub(crate) ui: UiConfig,
    /// Selected row in the slash / `@file` menu.
    pub(crate) menu_index: usize,
    /// Cached relative file paths for `@` mentions (lazy).
    pub(crate) file_index: Option<Vec<String>>,
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
    /// Visible rows of completed assistant responses, rebuilt every frame.
    pub(crate) assistant_row_hits: Vec<AssistantRowHit>,
    /// All rendered rows of completed assistant responses, including offscreen rows.
    pub(crate) assistant_rows: Vec<AssistantResponseRow>,
    /// Active drag selection inside one assistant response.
    pub(crate) assistant_selection: Option<AssistantSelection>,
    /// Transcript content width used when the current selection was created.
    pub(crate) assistant_selection_width: usize,
    /// Block index under the mouse (plan / subagent cards light up on hover).
    pub(crate) hover_block: Option<usize>,
    /// Sidebar subagent / terminal row under the mouse.
    pub(crate) hover_sidebar_item: Option<SidebarItem>,
    /// `← back` strip hovered in subagent / plan view.
    pub(crate) hover_back: bool,
    /// Plan-view MAKE button hovered.
    pub(crate) hover_make: bool,
    /// Terminal-view STOP button hovered.
    pub(crate) hover_terminal_stop: bool,
    /// Landing-screen logo rect from the last draw (for click hit-testing).
    pub(crate) logo_hit: Option<Rect>,
    /// Active "bonk" ripple on the HIVE wordmark.
    pub(crate) logo_bonk: Option<LogoBonk>,
    /// Main transcript vs a read-only subagent / plan view.
    pub(crate) view: ChatView,
    /// Hit target for the `← back` control in subagent/plan view (last draw).
    pub(crate) back_hit: Option<Rect>,
    /// Hit target for the Make / Send button in plan preview (last draw).
    pub(crate) make_hit: Option<Rect>,
    /// Hit target for STOP in the terminal bottom bar.
    pub(crate) terminal_stop_hit: Option<Rect>,
    /// Transcript viewport from the last draw (plan body hit-testing).
    pub(crate) transcript_hit: Option<Rect>,
    /// Hit target for the main input strip (last draw). Cleared in special views.
    pub(crate) input_hit: Option<Rect>,
    /// Whether the main input has keyboard focus (caret + typing).
    pub(crate) input_focused: bool,
    /// Wall clock of the last key that affected the composer (typing / caret).
    /// Cleared when blurred. Used for idle auto-blur.
    pub(crate) input_last_activity: Option<std::time::Instant>,
    /// MAKE / PLAN mode for the next user turn.
    pub(crate) agent_mode: AgentMode,
    /// Plan preview selection / amend state.
    pub(crate) plan_view: PlanViewState,
    /// Interactive terminal special-view state.
    pub(crate) terminal_view: TerminalViewState,
    /// Body resize waiting to be sent to the PTY manager.
    pub(crate) pending_terminal_resize: Option<(String, u16, u16)>,
    /// Finished-assistant markdown cache (invalidated on width change).
    pub(crate) md_cache: MdCache,
    /// Cached git project / diff summary for the right sidebar.
    pub(crate) project: ProjectSnapshot,
    /// Whether the wide-screen project sidebar is visible.
    pub(crate) sidebar_open: bool,
    /// Hit target for the sidebar show/hide control (last draw).
    pub(crate) sidebar_toggle_hit: Option<Rect>,
    /// Hit target for dragging the sidebar's left edge to resize.
    pub(crate) sidebar_resize_hit: Option<Rect>,
    /// Exclusive right edge of the sidebar panel (last draw) — used while resizing.
    pub(crate) sidebar_right_edge: u16,
    /// True while the user is dragging the sidebar resize handle.
    pub(crate) sidebar_resizing: bool,
    /// Expand/collapse state for sidebar body sections (in-memory for the session).
    pub(crate) sidebar_sections: SidebarSections,
    /// Hit targets for collapsible section headers (last draw).
    pub(crate) sidebar_section_hits: Vec<(Rect, SidebarSection)>,
    /// Hit targets for clickable subagent / terminal rows (last draw).
    pub(crate) sidebar_item_hits: Vec<(Rect, SidebarItem)>,
    /// Project instruction files present under cwd (refreshed with the project snapshot).
    pub(crate) context_files: Vec<hive_core::ContextFile>,
    /// Visible list rows in the palette (last draw) — keeps keyboard selection in view.
    pub(crate) palette_list_visible: u16,
    /// Centered context menu for the clicked transcript block.
    pub(crate) context_menu: Option<ContextMenu>,
}

/// How long the ctrl+c confirmation window lives.
pub const FLASH_MS: u128 = 1500;
/// Bottom toast lifetime (model/provider status, Ctrl+C, etc.).
pub const TOAST_MS: u128 = 2_000;
/// Composer auto-blur after this many ms with no typing / caret keys.
/// Transcript scroll and global chords do not refresh the timer, so a stuck
/// caret does not linger while the user reads. Mid-range of the 8–15s band.
pub const INPUT_IDLE_BLUR_MS: u128 = 12_000;

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
            context_tokens: 0,
            context_window: init.context_window.max(1),
            cost_input: init.cost_input,
            cost_output: init.cost_output,
            running: false,
            spinner: 0,
            scroll_from_bottom: 0,
            transcript_max_scroll: 0,
            pending_attaches: Vec::new(),
            follow_up: None,
            prompt_history: PromptHistory::default(),
            pending_dispatch: None,
            model_choices: init.model_choices,
            skills: init.skills,
            models_catalog: ModelsCatalogState::Idle,
            connections: init.connections,
            active_connection: init.active_connection,
            palette: None,
            about_open: false,
            settings: None,
            ui: init.ui.clone(),
            menu_index: 0,
            file_index: None,
            flash_msg: None,
            ctrl_c_armed: None,
            anim_start: std::time::Instant::now(),
            click_hits: Vec::new(),
            assistant_row_hits: Vec::new(),
            assistant_rows: Vec::new(),
            assistant_selection: None,
            assistant_selection_width: 0,
            hover_block: None,
            hover_sidebar_item: None,
            hover_back: false,
            hover_make: false,
            hover_terminal_stop: false,
            logo_hit: None,
            logo_bonk: None,
            view: ChatView::Main,
            back_hit: None,
            make_hit: None,
            terminal_stop_hit: None,
            transcript_hit: None,
            input_hit: None,
            input_focused: true,
            input_last_activity: Some(std::time::Instant::now()),
            agent_mode: AgentMode::Make,
            plan_view: PlanViewState::default(),
            terminal_view: TerminalViewState::default(),
            pending_terminal_resize: None,
            md_cache: MdCache::default(),
            project: ProjectSnapshot::default(),
            sidebar_open: !matches!(init.ui.sidebar_mode, SidebarMode::Hidden),
            sidebar_toggle_hit: None,
            sidebar_resize_hit: None,
            sidebar_right_edge: 0,
            sidebar_resizing: false,
            sidebar_sections: SidebarSections::default(),
            sidebar_section_hits: Vec::new(),
            sidebar_item_hits: Vec::new(),
            context_files: Vec::new(),
            palette_list_visible: 0,
            context_menu: None,
        };
        app.blocks.push(Block::Welcome);
        app.project.refresh_if_stale(&app.cwd);
        app.refresh_context_files();
        app
    }

    /// Refresh git project snapshot when the cache is stale.
    pub fn refresh_project(&mut self) {
        let was_stale = self.project.stale();
        self.project.refresh_if_stale(&self.cwd);
        if was_stale {
            self.refresh_context_files();
        }
    }

    /// Re-scan project instruction files (AGENTS.md, CLAUDE.md, …).
    pub fn refresh_context_files(&mut self) {
        self.context_files = hive_core::discover_context_files(std::path::Path::new(&self.cwd));
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

    /// Apply in-memory UI prefs and ask the driver to persist them.
    pub fn persist_ui(
        &mut self,
        input_tx: &tokio::sync::mpsc::UnboundedSender<crate::InputCommand>,
    ) {
        self.apply_ui_prefs();
        let _ = input_tx.send(crate::InputCommand::SaveUi(self.ui.clone()));
        self.flash("Settings saved");
    }

    fn apply_ui_prefs(&mut self) {
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

    /// Persist the composer into the active correction comment.
    pub fn plan_sync_active_note(&mut self) {
        let Some(i) = self.plan_view.active else {
            return;
        };
        if let Some(c) = self.plan_view.corrections.get_mut(i) {
            c.note = self.input.value.clone();
        }
    }

    /// True while writing/editing a comment (composer + MARK).
    /// Idle with saved comments shows ← back + SEND (like ← back + MAKE).
    pub fn plan_composing(&self) -> bool {
        self.plan_view
            .drag_range()
            .is_some_and(|(start, end)| start < end)
            || self.plan_view.active.is_some()
            || (!self.input.is_empty() && self.plan_view.has_corrections())
    }

    /// Enter / MARK: attach composer text to the selection, then idle → SEND.
    /// Returns whether anything changed.
    pub fn plan_commit_note(&mut self) -> bool {
        if !self.plan_composing() && !self.plan_view.has_corrections() {
            return false;
        }
        if self.plan_view.active.is_none() && self.plan_view.drag.is_none() {
            return false;
        }
        self.plan_sync_active_note();
        let _ = self.input.take();
        self.plan_view.active = None;
        self.blur_input();
        let n = self.plan_view.corrections.len();
        if n == 0 {
            self.flash("Nothing to add");
        } else if n == 1 {
            self.flash("Comment added · select more or SEND");
        } else {
            self.flash(format!("{n} comments · select more or SEND"));
        }
        true
    }

    /// Which right-chip: MAKE (no comments) / MARK (composing) / SEND (idle with comments).
    pub fn plan_action(&self) -> PlanAction {
        if self.plan_composing() {
            PlanAction::Add
        } else if !self.plan_view.has_corrections() {
            PlanAction::Make
        } else {
            PlanAction::Send
        }
    }

    /// Four-cell chip labels keep every action exactly centered in the same button.
    pub fn plan_action_word(&self) -> &'static str {
        match self.plan_action() {
            PlanAction::Make => "MAKE",
            PlanAction::Add => "MARK",
            PlanAction::Send => "SEND",
        }
    }

    /// Focus a correction and load its comment into the composer (read / edit).
    pub fn plan_focus_correction(&mut self, idx: usize) -> bool {
        if idx >= self.plan_view.corrections.len() {
            return false;
        }
        self.plan_sync_active_note();
        self.plan_view.active = Some(idx);
        let note = self.plan_view.corrections[idx].note.clone();
        self.input.clear();
        if !note.is_empty() {
            self.input.insert_str(&note);
        }
        self.focus_input();
        self.flash("Edit comment · MARK saves");
        true
    }

    /// Leave the composer; keep saved comments (back + SEND).
    pub fn plan_stop_composing(&mut self) -> bool {
        if !self.plan_composing() {
            return false;
        }
        // Drop an unfinished mark with no comment yet.
        if let Some(i) = self.plan_view.active {
            if self
                .plan_view
                .corrections
                .get(i)
                .is_some_and(|c| c.note.trim().is_empty() && self.input.is_empty())
            {
                self.plan_view.corrections.remove(i);
            } else {
                self.plan_sync_active_note();
            }
        }
        self.plan_view.active = None;
        self.plan_view.drag = None;
        let _ = self.input.take();
        self.blur_input();
        true
    }

    /// Finalize a drag range into a new correction (or focus an overlapping one).
    pub fn plan_finish_selection(&mut self, mut start: usize, mut end: usize) -> bool {
        if start > end {
            std::mem::swap(&mut start, &mut end);
        }
        self.plan_view.drag = None;
        if start > end {
            return false;
        }
        let Some(body) = self.plan_card().map(|c| c.body.clone()) else {
            return false;
        };
        if end > body.len() || !body.is_char_boundary(start) || !body.is_char_boundary(end) {
            return false;
        }
        // Click or selection fully inside an existing mark → open note to read/edit.
        if let Some(i) = self
            .plan_view
            .corrections
            .iter()
            .position(|c| start >= c.start && start < c.end && end <= c.end)
        {
            return self.plan_focus_correction(i);
        }
        // Empty click outside any mark.
        if start == end {
            return false;
        }
        let slice = &body[start..end];
        let Some(rel_start) = slice.find(|c: char| !c.is_whitespace()) else {
            return false;
        };
        let rel_end = slice
            .rfind(|c: char| !c.is_whitespace())
            .map(|i| i + slice[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1))
            .unwrap_or(rel_start);
        let new_start = start + rel_start;
        let new_end = start + rel_end;
        start = new_start;
        end = new_end;
        if start >= end || !body.is_char_boundary(start) || !body.is_char_boundary(end) {
            return false;
        }
        let excerpt = body[start..end].to_string();
        self.plan_sync_active_note();
        self.plan_view.corrections.push(PlanCorrection {
            start,
            end,
            excerpt,
            note: String::new(),
        });
        let idx = self.plan_view.corrections.len() - 1;
        self.plan_view.active = Some(idx);
        let _ = self.input.take();
        self.focus_input();
        true
    }

    /// Build the revise-plan user message from text corrections.
    pub fn plan_corrections_message(&self) -> String {
        let mut out = String::from(
            "Revise the plan in `.hive/Plan.md` based on these corrections. \
Keep everything else unless a note says otherwise.\n",
        );
        for (i, c) in self.plan_view.corrections.iter().enumerate() {
            out.push_str(&format!("\n### Correction {}\n\n", i + 1));
            for line in c.excerpt.lines() {
                out.push_str("> ");
                out.push_str(line);
                out.push('\n');
            }
            if !c.note.trim().is_empty() {
                out.push_str("\nNote: ");
                out.push_str(c.note.trim());
                out.push('\n');
            }
        }
        out
    }

    /// Clear corrections and leave the composer (stay in plan view).
    pub fn plan_clear_corrections(&mut self) {
        self.plan_view.corrections.clear();
        self.plan_view.active = None;
        self.plan_view.drag = None;
        let _ = self.input.take();
        self.blur_input();
    }

    /// Map a screen cell in the plan transcript to a source byte offset.
    pub fn plan_byte_at(&self, col: u16, row: u16) -> Option<usize> {
        use crate::render::tools::{col_to_offset, PlanRowSpan};

        let hit = self.transcript_hit?;
        if !hit.contains(col, row) {
            return None;
        }
        let scroll = self
            .transcript_max_scroll
            .saturating_sub(self.scroll_from_bottom);
        let line_idx = scroll + (row - hit.y) as usize;
        let body_i = line_idx.checked_sub(self.plan_view.body_line0)?;
        let &(start, end) = self.plan_view.row_spans.get(body_i)?;
        let body = self.plan_card()?.body.as_str();
        if start > body.len() || end > body.len() {
            return None;
        }
        let x = col.saturating_sub(hit.x) as usize;
        let content_col = x.saturating_sub(2);
        Some(col_to_offset(
            body,
            &PlanRowSpan { start, end },
            content_col,
        ))
    }

    /// Start / update / finish a drag selection over the plan body.
    pub fn plan_drag_to(&mut self, col: u16, row: u16, finish: bool) -> bool {
        let Some(off) = self.plan_byte_at(col, row) else {
            if finish {
                self.plan_view.drag = None;
            }
            return false;
        };
        match &mut self.plan_view.drag {
            Some(d) => d.current = off,
            None => {
                self.plan_view.drag = Some(crate::app::state::PlanDrag {
                    anchor: off,
                    current: off,
                });
            }
        }
        if finish {
            let (a, b) = self.plan_view.drag_range().unwrap_or((0, 0));
            self.plan_finish_selection(a, b)
        } else {
            true
        }
    }

    pub fn focus_input(&mut self) {
        self.input_focused = true;
        self.input_last_activity = Some(std::time::Instant::now());
    }

    pub fn blur_input(&mut self) {
        self.input_focused = false;
        self.input_last_activity = None;
    }

    /// Record that the user touched the composer (typing, caret, slash menu).
    pub fn note_input_activity(&mut self) {
        if self.input_focused {
            self.input_last_activity = Some(std::time::Instant::now());
        }
    }

    /// Blur the composer when focused but idle past [`INPUT_IDLE_BLUR_MS`].
    /// Returns true when focus was dropped (caller should redraw).
    pub fn maybe_idle_blur_input(&mut self) -> bool {
        if !self.input_focused {
            return false;
        }
        let Some(at) = self.input_last_activity else {
            return false;
        };
        if at.elapsed().as_millis() < INPUT_IDLE_BLUR_MS {
            return false;
        }
        self.blur_input();
        true
    }

    /// True when `(col, row)` lands inside the input strip from the last draw.
    pub fn input_contains(&self, col: u16, row: u16) -> bool {
        self.input_hit
            .map(|r| r.contains(col, row))
            .unwrap_or(false)
    }

    /// Begin a selection only when the pointer is on visible assistant text.
    pub fn start_assistant_selection(&mut self, col: u16, row: u16) -> bool {
        let Some((block, point)) = self.assistant_point_at(col, row, None, false) else {
            return false;
        };
        self.assistant_selection = Some(AssistantSelection {
            block,
            anchor: point,
            current: point,
            dragged: false,
            active: true,
        });
        true
    }

    /// Update the active selection, clamping it to the response where it began.
    pub fn update_assistant_selection(&mut self, col: u16, row: u16) -> bool {
        let Some((block, active)) = self
            .assistant_selection
            .as_ref()
            .map(|selection| (selection.block, selection.active))
        else {
            return false;
        };
        if !active {
            return false;
        }
        let Some((_, point)) = self.assistant_point_at(col, row, Some(block), true) else {
            return false;
        };
        let selection = self.assistant_selection.as_mut().expect("checked above");
        let changed = selection.current != point;
        selection.current = point;
        selection.dragged |= selection.current != selection.anchor;
        changed
    }

    /// Finish a drag, consume its highlight, and return clean rendered text.
    /// A plain click is discarded.
    pub fn finish_assistant_selection(&mut self, col: u16, row: u16) -> Option<String> {
        if !self.assistant_selection_active() {
            return None;
        }
        let _ = self.update_assistant_selection(col, row);
        let selection = self.assistant_selection.clone()?;
        if !selection.dragged {
            self.assistant_selection = None;
            return None;
        }
        let text = self.assistant_selection_text(&selection);
        self.assistant_selection = None;
        text
    }

    pub fn assistant_selection_active(&self) -> bool {
        self.assistant_selection
            .as_ref()
            .is_some_and(|selection| selection.active)
    }

    /// Clear the current selection; returns whether a redraw is needed.
    pub fn clear_assistant_selection(&mut self) -> bool {
        self.assistant_selection.take().is_some()
    }

    fn assistant_row_data(&self, block: usize, row: usize) -> Option<(&str, AssistantRowJoin)> {
        self.assistant_rows
            .iter()
            .find(|candidate| candidate.block == block && candidate.response_row == row)
            .map(|candidate| (candidate.text.as_str(), candidate.join_before))
            .or_else(|| {
                self.assistant_row_hits
                    .iter()
                    .find(|candidate| candidate.block == block && candidate.response_row == row)
                    .map(|candidate| (candidate.text.as_str(), candidate.join_before))
            })
    }

    /// Selected half-open display-column range for one rendered response row.
    pub fn assistant_selected_cols(&self, block: usize, row: usize) -> Option<(usize, usize)> {
        let selection = self.assistant_selection.as_ref()?;
        if selection.block != block {
            return None;
        }
        let (first, last) = normalized_points(selection.anchor, selection.current);
        if row < first.row || row > last.row {
            return None;
        }
        let (text, _) = self.assistant_row_data(block, row)?;
        assistant_row_selected_cols(first, last, row, text)
    }

    fn assistant_point_from_rendered_rows(
        &self,
        col: u16,
        row: u16,
        block: usize,
    ) -> Option<AssistantPoint> {
        let viewport = self.transcript_hit?;
        let viewport_row = usize::from(row.saturating_sub(viewport.y))
            .min(usize::from(viewport.height.saturating_sub(1)));
        let line_idx = self
            .transcript_max_scroll
            .saturating_sub(self.scroll_from_bottom)
            .saturating_add(viewport_row);
        let first = self
            .assistant_rows
            .iter()
            .filter(|candidate| candidate.block == block)
            .min_by_key(|candidate| candidate.line_idx)?;
        let last = self
            .assistant_rows
            .iter()
            .filter(|candidate| candidate.block == block)
            .max_by_key(|candidate| candidate.line_idx)?;
        let candidate = if line_idx <= first.line_idx {
            first
        } else if line_idx >= last.line_idx {
            last
        } else {
            self.assistant_rows
                .iter()
                .filter(|candidate| candidate.block == block)
                .min_by_key(|candidate| candidate.line_idx.abs_diff(line_idx))?
        };
        let width = display_width(&candidate.text);
        let relative = if width == 0 || line_idx < candidate.line_idx {
            0
        } else if line_idx > candidate.line_idx {
            width - 1
        } else {
            usize::from(col.saturating_sub(viewport.x.saturating_add(2))).min(width - 1)
        };
        Some(AssistantPoint {
            row: candidate.response_row,
            col: display_cell_start(&candidate.text, relative),
        })
    }

    fn assistant_point_at(
        &self,
        col: u16,
        row: u16,
        block: Option<usize>,
        clamp: bool,
    ) -> Option<(usize, AssistantPoint)> {
        let exact = self.assistant_row_hits.iter().find(|hit| {
            if block.is_some_and(|wanted| wanted != hit.block) || hit.screen_row != row {
                return false;
            }
            let width = display_width(&hit.text) as u16;
            width > 0 && col >= hit.x && col < hit.x.saturating_add(width)
        });
        let (hit, nearest) = match exact {
            Some(hit) => (hit, false),
            None if clamp => {
                if let Some(block) = block {
                    if let Some(point) = self.assistant_point_from_rendered_rows(col, row, block) {
                        return Some((block, point));
                    }
                }
                (
                    self.assistant_row_hits
                        .iter()
                        .filter(|hit| block.is_none_or(|wanted| wanted == hit.block))
                        .min_by_key(|hit| hit.screen_row.abs_diff(row))?,
                    true,
                )
            }
            None => return None,
        };
        let width = display_width(&hit.text);
        if width == 0 {
            return None;
        }
        let relative = if nearest && row < hit.screen_row {
            0
        } else if nearest && row > hit.screen_row {
            width - 1
        } else {
            usize::from(col.saturating_sub(hit.x)).min(width - 1)
        };
        Some((
            hit.block,
            AssistantPoint {
                row: hit.response_row,
                col: display_cell_start(&hit.text, relative),
            },
        ))
    }

    fn assistant_selection_text(&self, selection: &AssistantSelection) -> Option<String> {
        let (first, last) = normalized_points(selection.anchor, selection.current);
        let has_full_rows = self
            .assistant_rows
            .iter()
            .any(|row| row.block == selection.block);
        let mut rows: Vec<(usize, &str, AssistantRowJoin)> = if has_full_rows {
            self.assistant_rows
                .iter()
                .filter(|row| {
                    row.block == selection.block
                        && row.response_row >= first.row
                        && row.response_row <= last.row
                })
                .map(|row| (row.response_row, row.text.as_str(), row.join_before))
                .collect()
        } else {
            self.assistant_row_hits
                .iter()
                .filter(|hit| {
                    hit.block == selection.block
                        && hit.response_row >= first.row
                        && hit.response_row <= last.row
                })
                .map(|hit| (hit.response_row, hit.text.as_str(), hit.join_before))
                .collect()
        };
        rows.sort_by_key(|(response_row, _, _)| *response_row);
        let expected_len = last.row.checked_sub(first.row)?.checked_add(1)?;
        if rows.len() != expected_len
            || rows
                .iter()
                .enumerate()
                .any(|(i, (response_row, _, _))| *response_row != first.row + i)
        {
            return None;
        }

        let mut out = String::new();
        for (i, (response_row, text, join_before)) in rows.into_iter().enumerate() {
            let (start, end) = assistant_row_selected_cols(first, last, response_row, text)?;
            let mut fragment = slice_display_range(text, start, end);
            if matches!(
                join_before,
                AssistantRowJoin::SoftSpace | AssistantRowJoin::SoftNone
            ) {
                fragment = fragment.trim_start_matches([' ', '\t']).to_string();
            }
            if i > 0 {
                match join_before {
                    AssistantRowJoin::Hard => {
                        trim_horizontal_end(&mut out);
                        out.push('\n');
                    }
                    AssistantRowJoin::SoftSpace => {
                        let separated = out.chars().last().is_some_and(char::is_whitespace)
                            || fragment.chars().next().is_some_and(char::is_whitespace);
                        if !separated && !out.is_empty() && !fragment.is_empty() {
                            out.push(' ');
                        }
                    }
                    AssistantRowJoin::SoftNone => {}
                }
            }
            out.push_str(&fragment);
        }
        let clean = clean_selected_text(&out);
        (!clean.is_empty()).then_some(clean)
    }

    pub fn spinner_char(&self) -> &'static str {
        spinner::frame(self.spinner)
    }

    /// Advance the animation frame from wall-clock time. Called every loop
    /// iteration; mouse/key event bursts don't speed the animation up because
    /// the frame is a pure function of elapsed time. Also idle-blurs the
    /// composer when typing has paused — returns true if a redraw is needed.
    pub fn tick(&mut self) -> bool {
        self.spinner = (self.anim_start.elapsed().as_millis() / 100) as usize;
        if self.logo_bonk.as_ref().is_some_and(|b| !b.alive()) {
            self.logo_bonk = None;
        }
        self.maybe_idle_blur_input()
    }

    /// True when the frame should keep painting (spinner / shimmer / bonk / flash).
    pub fn needs_animation(&self) -> bool {
        if self.logo_bonk.is_some() || self.flash_text().is_some() {
            return true;
        }
        if self.pending_dispatch.is_some() {
            return true;
        }
        if self.running {
            return true;
        }
        let viewed_terminal = self.terminal_view_id();
        self.blocks.iter().any(|b| match b {
            Block::Tool(c) => c.status == ToolStatus::Running,
            Block::Subagent(c) => c.status == SubagentStatus::Running,
            Block::Plan(c) => c.status == PlanStatus::Writing,
            // Backgrounded long-lived PTYs (dev servers, watchers) must not
            // pin the UI at ANIM_TICK forever — only animate when on-screen.
            Block::Terminal(c) => {
                matches!(c.process, hive_core::TerminalProcessState::Running)
                    && viewed_terminal == Some(c.id.as_str())
            }
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
        let Some((lx, ly)) = crate::render::wordmark::hit_cell(logo, col, row) else {
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

    // --- slash / @file menus ---

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

    pub fn slash_items(&self) -> Vec<SlashItem> {
        let Some(prefix) = self.slash_prefix() else {
            return Vec::new();
        };
        let prefix_l = prefix.to_ascii_lowercase();
        let mut items: Vec<SlashItem> = commands::filtered(prefix)
            .into_iter()
            .map(SlashItem::Command)
            .collect();
        for s in &self.skills {
            if commands::is_builtin_name(&s.name) {
                continue;
            }
            let name_l = s.name.to_ascii_lowercase();
            let desc_l = s.description.to_ascii_lowercase();
            if prefix_l.is_empty()
                || name_l.starts_with(&prefix_l)
                || name_l.contains(&prefix_l)
                || desc_l.contains(&prefix_l)
            {
                items.push(SlashItem::Skill(s.clone()));
            }
        }
        items
    }

    /// Active `@path` query at the cursor (disabled while slash menu owns input).
    pub fn at_mention(&self) -> Option<AtQuery> {
        if self.slash_prefix().is_some() {
            return None;
        }
        files::at_query(&self.input.value, self.input.cursor)
    }

    /// Ensure the project file index is loaded (lazy, once per session).
    pub fn ensure_file_index(&mut self) {
        if self.file_index.is_some() {
            return;
        }
        self.file_index = Some(files::index_files(std::path::Path::new(&self.cwd)));
    }

    /// Filtered file paths for the `@` menu (empty when menu closed).
    pub fn file_menu_items(&mut self) -> Vec<String> {
        let Some(q) = self.at_mention() else {
            return Vec::new();
        };
        self.ensure_file_index();
        let index = self.file_index.as_deref().unwrap_or(&[]);
        files::filter_files(index, &q.query)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    pub fn menu_up(&mut self) {
        let n = if !self.slash_items().is_empty() {
            self.slash_items().len()
        } else {
            self.file_menu_items().len()
        };
        if n > 0 {
            self.menu_index = (self.menu_index + n - 1) % n;
        }
    }

    pub fn menu_down(&mut self) {
        let n = if !self.slash_items().is_empty() {
            self.slash_items().len()
        } else {
            self.file_menu_items().len()
        };
        if n > 0 {
            self.menu_index = (self.menu_index + 1) % n;
        }
    }

    pub fn menu_selected(&self) -> Option<SlashItem> {
        let items = self.slash_items();
        if items.is_empty() {
            return None;
        }
        Some(items[self.menu_index.min(items.len() - 1)].clone())
    }

    pub fn find_skill(&self, name: &str) -> Option<&SkillChoice> {
        let name = name.to_ascii_lowercase();
        self.skills
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(&name))
    }

    pub fn file_menu_selected(&mut self) -> Option<String> {
        let items = self.file_menu_items();
        if items.is_empty() {
            return None;
        }
        Some(items[self.menu_index.min(items.len() - 1)].clone())
    }

    /// Turn `@query` into a pending `@file` chip (removes the typed mention).
    pub fn complete_at_file(&mut self, rel: &str) {
        let Some(q) = self.at_mention() else {
            return;
        };
        let end = self.input.cursor;
        self.input.replace_chars(q.at, end, "");
        // Collapse a double space left when `@…` sat mid-sentence.
        let v = self.input.value.clone();
        if let Some(i) = v.find("  ") {
            let chars: Vec<char> = v.chars().collect();
            if i + 1 < chars.len() {
                self.input.replace_chars(i, i + 2, " ");
            }
        }
        match self.attach_rel(rel) {
            Ok(_) => self.reset_menu(),
            Err(e) => {
                self.flash(e);
                self.reset_menu();
            }
        }
    }

    pub fn reset_menu(&mut self) {
        self.menu_index = 0;
    }

    pub fn scroll_up(&mut self, n: usize) {
        let next = self.scroll_from_bottom.saturating_add(n);
        self.scroll_from_bottom = next.min(self.transcript_max_scroll);
    }

    pub fn scroll_down(&mut self, n: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(n);
    }

    /// True when Up can move the transcript toward older content.
    pub fn can_scroll_up(&self) -> bool {
        self.scroll_from_bottom < self.transcript_max_scroll
    }

    /// True when Down can move the transcript back toward the bottom.
    pub fn can_scroll_down(&self) -> bool {
        self.scroll_from_bottom > 0
    }

    /// Remember how far the transcript can scroll (from the last paint) and
    /// pin `scroll_from_bottom` so phantom Ups on a short view don't stick.
    pub fn set_transcript_max_scroll(&mut self, max: usize) {
        self.transcript_max_scroll = max;
        if self.scroll_from_bottom > max {
            self.scroll_from_bottom = max;
        }
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
        self.context_tokens = 0;
        self.scroll_from_bottom = 0;
        self.pending_attaches.clear();
        self.follow_up = None;
        self.prompt_history.reset();
        self.pending_dispatch = None;
        self.input.clear();
        self.reset_menu();
        self.close_palette();
        self.close_about();
        self.close_context_menu();
        self.running = false;
        self.click_hits.clear();
        self.assistant_row_hits.clear();
        self.assistant_rows.clear();
        self.assistant_selection = None;
        self.assistant_selection_width = 0;
        self.hover_block = None;
        self.hover_sidebar_item = None;
        self.hover_back = false;
        self.hover_make = false;
        self.hover_terminal_stop = false;
        self.view = ChatView::Main;
        self.back_hit = None;
        self.make_hit = None;
        self.terminal_stop_hit = None;
        self.plan_view = PlanViewState::default();
        self.terminal_view = TerminalViewState::default();
        self.pending_terminal_resize = None;
        self.md_cache = MdCache::default();
    }

    pub fn has_pending_attaches(&self) -> bool {
        !self.pending_attaches.is_empty()
    }

    pub fn has_follow_up(&self) -> bool {
        self.follow_up.is_some()
    }

    /// Queue (or replace) a follow-up while the agent is busy.
    pub fn queue_follow_up(&mut self, fu: QueuedFollowUp) {
        self.follow_up = Some(fu);
        self.flash("Follow-up queued · Enter again → next step");
    }

    /// Pull the queued follow-up into the composer for editing (↑).
    pub fn recall_follow_up(&mut self) -> bool {
        let Some(fu) = self.follow_up.take() else {
            return false;
        };
        self.pending_attaches = fu.attaches;
        self.agent_mode = fu.mode;
        self.input.value = fu.composer;
        self.input.end();
        self.reset_menu();
        self.flash("Edit follow-up · Enter to re-queue");
        true
    }

    /// Clear a queued follow-up without sending.
    pub fn clear_follow_up(&mut self) -> bool {
        if self.follow_up.take().is_some() {
            self.flash("Follow-up cleared");
            true
        } else {
            false
        }
    }

    // -- Prompt history (shell-style up/down recall) --

    /// Step back through prompt history. Saves the current composer text as a
    /// draft on first entry. Returns `false` if there is no history.
    pub fn history_up(&mut self) -> bool {
        if self.prompt_history.entries.is_empty() {
            return false;
        }
        match self.prompt_history.index {
            None => {
                self.prompt_history.draft = self.input.value.clone();
                let i = self.prompt_history.entries.len() - 1;
                self.prompt_history.index = Some(i);
                self.input.value = self.prompt_history.entries[i].clone();
                self.input.end();
                true
            }
            Some(0) => true,
            Some(i) => {
                let i = i - 1;
                self.prompt_history.index = Some(i);
                self.input.value = self.prompt_history.entries[i].clone();
                self.input.end();
                true
            }
        }
    }

    /// Step forward through prompt history. Past the last entry, restores the
    /// draft that was saved on entry. Returns `false` when not browsing.
    pub fn history_down(&mut self) -> bool {
        let Some(i) = self.prompt_history.index else {
            return false;
        };
        if i + 1 >= self.prompt_history.entries.len() {
            self.prompt_history.index = None;
            self.input.value = self.prompt_history.draft.clone();
            self.input.end();
            self.prompt_history.draft.clear();
        } else {
            let next = i + 1;
            self.prompt_history.index = Some(next);
            self.input.value = self.prompt_history.entries[next].clone();
            self.input.end();
        }
        true
    }

    // -- Pending dispatch (ESC recall before the agent starts) --

    /// True when the grace period has elapsed and the deferred message should
    /// be flushed to the driver.
    pub fn pending_dispatch_ready(&self) -> bool {
        match &self.pending_dispatch {
            Some(pd) => pd.submitted_at.elapsed().as_millis() >= DISPATCH_GRACE_MS,
            None => false,
        }
    }

    /// Take the pending dispatch out for flushing to the driver.
    pub fn take_pending_dispatch(&mut self) -> Option<PendingDispatch> {
        self.pending_dispatch.take()
    }

    /// Recall a deferred message back into the composer (ESC before the agent
    /// starts). Removes the last `Block::User` from the transcript, restores
    /// the composer text and attachments, and flashes a hint.
    pub fn recall_pending_dispatch(&mut self) -> bool {
        let Some(pd) = self.pending_dispatch.take() else {
            return false;
        };
        if matches!(self.blocks.last(), Some(Block::User(_))) {
            self.blocks.pop();
        }
        self.input.value = pd.composer;
        self.input.end();
        self.pending_attaches = pd.attaches;
        self.agent_mode = pd.mode;
        self.reset_menu();
        self.scroll_from_bottom = 0;
        self.flash("Prompt recalled — edit and resend");
        true
    }

    /// One-line preview for the banner above the input.
    pub fn follow_up_preview(&self, max_chars: usize) -> Option<String> {
        let fu = self.follow_up.as_ref()?;
        let t = fu.display.replace('\n', " ");
        let t = t.trim();
        if t.chars().count() <= max_chars {
            return Some(t.to_string());
        }
        let mut s: String = t.chars().take(max_chars.saturating_sub(1)).collect();
        s.push('…');
        Some(s)
    }

    pub fn attachment_tags_line(&self) -> String {
        self.pending_attaches
            .iter()
            .map(PendingAttach::tag)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Attach a project-relative path from the `@` picker.
    pub fn attach_rel(&mut self, rel: &str) -> Result<String, String> {
        let rel = rel.trim().trim_start_matches("./");
        if rel.is_empty() {
            return Err("empty path".into());
        }
        let abs = std::path::Path::new(&self.cwd).join(rel);
        self.attach_path_inner(&abs, rel)
    }

    /// Attach a path: images become vision sources; other files are path notes.
    pub fn attach_path(&mut self, path: &str) -> Result<String, String> {
        let path = path.trim();
        if path.is_empty() {
            return Err("paste a file path to attach".into());
        }
        let abs = std::path::Path::new(path);
        let abs = if abs.is_absolute() {
            abs.to_path_buf()
        } else {
            std::path::Path::new(&self.cwd).join(abs)
        };
        let label = abs
            .strip_prefix(&self.cwd)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| {
                abs.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| abs.to_string_lossy().into_owned())
            });
        self.attach_path_inner(&abs, &label)
    }

    fn attach_path_inner(&mut self, abs: &std::path::Path, label: &str) -> Result<String, String> {
        let bytes =
            std::fs::read(abs).map_err(|e| format!("cannot read {}: {e}", abs.display()))?;
        let is_image = is_image_path(abs.to_string_lossy().as_ref());
        let image = if is_image {
            Some(ImageSource::Base64 {
                media_type: media_type_for(abs.to_string_lossy().as_ref()),
                data: base64_encode(&bytes),
            })
        } else {
            None
        };
        let label = label.trim().trim_start_matches("./").to_string();
        let path = label.clone();
        let tag = format!("@{label}");
        // Skip duplicates (same path already queued).
        if self.pending_attaches.iter().any(|a| a.path == path) {
            return Ok(tag);
        }
        self.pending_attaches
            .push(PendingAttach { label, path, image });
        Ok(tag)
    }

    /// Attach existing file path(s) parsed from pasted / drag-and-dropped `text`
    /// as `@chips`, exactly like the `@` picker. Handles the quirks terminals
    /// apply on drop: surrounding quotes, backslash-escaped spaces, and
    /// (percent-encoded) `file://` URIs — including a multi-file drop. Returns
    /// `Some("")` when at least one file was attached, or `None` when the text
    /// isn't purely path(s) to existing file(s) (so the caller keeps it as text).
    pub fn try_attach_pasted_path(&mut self, text: &str) -> Option<String> {
        let candidates = drop_path_candidates(text)?;
        let mut attached = false;
        for cand in candidates {
            if std::path::Path::new(&cand).is_file() && self.attach_path(&cand).is_ok() {
                attached = true;
            }
        }
        attached.then_some(String::new())
    }

    pub fn take_pending_attaches(&mut self) -> Vec<PendingAttach> {
        std::mem::take(&mut self.pending_attaches)
    }

    pub fn open_palette(&mut self) {
        self.about_open = false;
        self.palette = Some(PaletteState::commands());
    }

    pub fn open_model_picker(
        &mut self,
        input_tx: &tokio::sync::mpsc::UnboundedSender<crate::InputCommand>,
    ) {
        self.about_open = false;
        self.palette = Some(PaletteState::models());
        self.models_catalog = ModelsCatalogState::Loading;
        let _ = input_tx.send(crate::InputCommand::FetchModels);
    }

    pub fn open_connect_picker(&mut self) {
        self.about_open = false;
        self.close_settings();
        let mut pal = PaletteState::connect();
        pal.clamp_selection(&self.model_choices, &self.connections);
        // Prefer selecting the active profile.
        if let Some(i) = pal.connect_rows(&self.connections).iter().position(
            |r| matches!(r, palette::ConnectRow::Profile(c) if c.id == self.active_connection),
        ) {
            pal.selected = i;
        }
        self.palette = Some(pal);
    }

    pub fn close_palette(&mut self) {
        self.palette = None;
    }

    pub fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    // -- Context menu (click on user prompt / tool card) --

    pub fn context_menu_open(&self) -> bool {
        self.context_menu.is_some()
    }

    pub fn close_context_menu(&mut self) {
        self.context_menu = None;
    }

    pub fn context_menu_up(&mut self) {
        if let Some(m) = &mut self.context_menu {
            if m.selected > 0 {
                m.selected -= 1;
            } else {
                m.selected = m.items.len().saturating_sub(1);
            }
        }
    }

    pub fn context_menu_down(&mut self) {
        if let Some(m) = &mut self.context_menu {
            let last = m.items.len().saturating_sub(1);
            if m.selected < last {
                m.selected += 1;
            } else {
                m.selected = 0;
            }
        }
    }

    /// Open a context menu for the given block with the provided items.
    fn open_context_menu(&mut self, block_idx: usize, items: Vec<ContextMenuItem>) {
        self.context_menu = Some(ContextMenu {
            block_idx,
            items,
            selected: 0,
        });
    }

    /// Build and open a context menu for a user prompt block.
    pub fn open_prompt_menu(&mut self, block_idx: usize) {
        match self.blocks.get(block_idx) {
            Some(Block::User(_)) => {}
            _ => return,
        };
        let items = vec![ContextMenuItem {
            label: "Copy".into(),
            action: ContextAction::CopyPrompt,
        }];
        self.open_context_menu(block_idx, items);
    }

    /// Build and open a context menu for a tool card block.
    pub fn open_tool_menu(&mut self, block_idx: usize) {
        let (output, snapshot) = match self.blocks.get(block_idx) {
            Some(Block::Tool(card)) => (card.output.clone(), card.snapshot.clone()),
            _ => return,
        };
        let mut items = Vec::new();
        if let Some(snap) = &snapshot {
            items.push(ContextMenuItem {
                label: format!("Revert {}", snap.path),
                action: ContextAction::RevertFile {
                    path: snap.path.clone(),
                    content: snap.content.clone(),
                },
            });
        }
        if !output.trim().is_empty() {
            items.push(ContextMenuItem {
                label: "Copy output".into(),
                action: ContextAction::CopyOutput,
            });
        }
        if items.is_empty() {
            return;
        }
        self.open_context_menu(block_idx, items);
    }

    /// Take the selected action from the context menu, closing it.
    pub fn take_context_action(&mut self) -> Option<ContextAction> {
        let m = self.context_menu.as_ref()?;
        let item = m.items.get(m.selected)?;
        Some(item.action.clone())
    }

    pub fn open_about(&mut self) {
        self.close_palette();
        self.close_settings();
        self.about_open = true;
    }

    pub fn close_about(&mut self) {
        self.about_open = false;
    }

    pub fn about_open(&self) -> bool {
        self.about_open
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
                let plan_write = name == "write_plan"
                    || ((name == "write_file" || name == "edit_file")
                        && (args_preview.contains("Plan.md")
                            || args_preview.contains(".hive/Plan")));
                if plan_write {
                    self.mark_plan_writing();
                }
                // Plan writes are card-only (no green/orange tool chrome).
                // Subagent tools render as Subagent cards, not tool rows.
                // `switch_mode` renders as its own "Switched to … Mode" card.
                if !plan_write
                    && !is_subagent_tool(&name)
                    && name != "switch_mode"
                    && !hive_core::terminal::is_terminal_tool(&name)
                {
                    self.blocks.push(Block::Tool(ToolCard {
                        id,
                        name,
                        args: args_preview,
                        output: String::new(),
                        status: ToolStatus::Running,
                        started: std::time::Instant::now(),
                        elapsed_ms: None,
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
            AgentEvent::TerminalStarted {
                id,
                command,
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
                let Some(card) = self.blocks.iter_mut().find_map(|block| match block {
                    Block::Terminal(card) if card.id == id => Some(card),
                    _ => None,
                }) else {
                    return false;
                };
                card.screen = screen;
                card.revision = card.revision.max(revision);
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
                cost_input,
                cost_output,
            } => {
                self.model = id;
                self.model_display = display;
                self.cost_input = cost_input;
                self.cost_output = cost_output;
                true
            }
            AgentEvent::ModelsListed { models } => {
                self.model_choices = models
                    .into_iter()
                    .map(|m| ModelChoice {
                        key: m.id,
                        display: m.name,
                        detail: m.detail,
                        group: m.group,
                        connection_id: m.connection_id,
                        vision: m.vision,
                        cost_input: m.cost_input,
                        cost_output: m.cost_output,
                    })
                    .collect();
                self.models_catalog = ModelsCatalogState::Ready;
                if let Some(pal) = self.palette.as_mut() {
                    if pal.mode == palette::PaletteMode::Models {
                        pal.clamp_selection(&self.model_choices, &self.connections);
                    }
                }
                true
            }
            AgentEvent::ModelsListFailed(err) => {
                self.models_catalog = ModelsCatalogState::Failed(err);
                true
            }
            AgentEvent::ConnectionsUpdated { active, profiles } => {
                self.active_connection = active;
                self.connections = profiles;
                if let Some(pal) = self.palette.as_mut() {
                    if matches!(
                        pal.mode,
                        palette::PaletteMode::Connect
                            | palette::PaletteMode::ConnectPresets
                            | palette::PaletteMode::ConnectKey { .. }
                    ) {
                        pal.clamp_selection(&self.model_choices, &self.connections);
                    }
                }
                true
            }
            AgentEvent::Notice(s) => {
                // Status feedback (model/provider switch, interrupt, …) lives in
                // the bottom toast — same place as Ctrl+C — not the transcript.
                self.flash(s);
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
        th.open = self.ui.thoughts_always_open;
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
            Some(Block::Terminal(card)) => {
                let id = card.id.clone();
                self.open_terminal_view(id);
            }
            Some(Block::User(_)) => {
                self.open_prompt_menu(block_idx);
            }
            Some(Block::Tool(_)) => {
                self.open_tool_menu(block_idx);
            }
            _ => {}
        }
    }

    /// Index of the most recent plan card in the transcript.
    fn last_plan_idx(&self) -> Option<usize> {
        self.blocks
            .iter()
            .rposition(|b| matches!(b, Block::Plan(_)))
    }

    fn mark_plan_writing(&mut self) {
        // Rewriting a finished plan moves the single card to the bottom of the
        // transcript (with the UPDATED badge) so the change is visible next to
        // the latest messages — one Plan.md card total, never a stack of copies.
        // A still-empty / in-progress card is reused in place.
        if let Some(i) = self.last_plan_idx() {
            {
                let Block::Plan(card) = &mut self.blocks[i] else {
                    return;
                };
                let rewrite = card.status == PlanStatus::Ready && !card.body.is_empty();
                card.status = PlanStatus::Writing;
                card.elapsed_ms = None;
                if !rewrite {
                    return;
                }
                card.revised = true;
            }
            self.clear_assistant_selection();
            let card = self.blocks.remove(i);
            self.blocks.push(card);
            self.scroll_from_bottom = 0;
            return;
        }
        self.blocks.push(Block::Plan(PlanCard {
            summary: String::new(),
            body: String::new(),
            status: PlanStatus::Writing,
            started: std::time::Instant::now(),
            elapsed_ms: None,
            revised: false,
        }));
    }

    fn upsert_plan(&mut self, summary: String, body: String) {
        let summary = summary.trim().to_string();
        let revised;
        match self.last_plan_idx() {
            None => {
                self.blocks.push(Block::Plan(PlanCard {
                    summary,
                    body,
                    status: PlanStatus::Ready,
                    started: std::time::Instant::now(),
                    elapsed_ms: Some(0),
                    revised: false,
                }));
                revised = false;
            }
            Some(i) => {
                // Rewrite arriving without a "writing…" placeholder (e.g. a
                // direct event) — same rule as mark_plan_writing: move the one
                // card to the bottom and badge it.
                let rewrite = {
                    let Block::Plan(card) = &self.blocks[i] else {
                        return;
                    };
                    card.status == PlanStatus::Ready && !card.body.is_empty()
                };
                {
                    let Block::Plan(card) = &mut self.blocks[i] else {
                        return;
                    };
                    card.summary = summary;
                    card.body = body;
                    card.status = PlanStatus::Ready;
                    card.revised = card.revised || rewrite;
                    if card.elapsed_ms.is_none() {
                        card.elapsed_ms = Some(card.started.elapsed().as_millis());
                    }
                    revised = card.revised;
                }
                if rewrite {
                    self.clear_assistant_selection();
                    let card = self.blocks.remove(i);
                    self.blocks.push(card);
                    self.scroll_from_bottom = 0;
                }
            }
        }
        // Body rewrite invalidates highlight ranges — drop them.
        if matches!(self.view, ChatView::Plan) {
            self.plan_clear_corrections();
            // Show the revised plan from the top, like a fresh open, so
            // the reader isn't stranded mid-scroll over new content.
            self.scroll_from_bottom = usize::MAX;
            if revised {
                self.flash("plan revised");
            }
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
                        let mut start = card.output.len() - CAP;
                        while !card.output.is_char_boundary(start) {
                            start += 1;
                        }
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

    fn attach_snapshot(&mut self, id: &str, snapshot: FileSnapshot) {
        for block in self.blocks.iter_mut().rev() {
            if let Block::Tool(card) = block {
                if card.id == id {
                    card.snapshot = Some(snapshot);
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

    /// Terminal cards for the sidebar (running first, then failed, then exited).
    pub fn sidebar_terminals(&self) -> Vec<&TerminalCard> {
        let mut cards: Vec<&TerminalCard> = self
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Terminal(c) => Some(c.as_ref()),
                _ => None,
            })
            .collect();
        cards.sort_by_key(|c| match &c.process {
            TerminalProcessState::Running => 0u8,
            TerminalProcessState::Failed { .. } => 1,
            TerminalProcessState::Exited { code } if *code != 0 => 2,
            TerminalProcessState::Exited { .. } => 3,
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

fn normalized_points(a: AssistantPoint, b: AssistantPoint) -> (AssistantPoint, AssistantPoint) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn assistant_row_selected_cols(
    first: AssistantPoint,
    last: AssistantPoint,
    row: usize,
    text: &str,
) -> Option<(usize, usize)> {
    let width = display_width(text);
    if width == 0 {
        return Some((0, 0));
    }
    let start = if row == first.row {
        first.col.min(width)
    } else {
        0
    };
    let end = if row == last.row {
        display_char_end(text, last.col).min(width)
    } else {
        width
    };
    (start < end).then_some((start, end))
}

fn display_width(text: &str) -> usize {
    use unicode_width::UnicodeWidthChar;

    text.chars()
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum()
}

/// Start display column of the glyph occupying `col`.
fn display_cell_start(text: &str, col: usize) -> usize {
    use unicode_width::UnicodeWidthChar;

    let mut x = 0;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width == 0 {
            continue;
        }
        if col < x + width {
            return x;
        }
        x += width;
    }
    x
}

/// End display column of the glyph occupying `col`.
fn display_char_end(text: &str, col: usize) -> usize {
    use unicode_width::UnicodeWidthChar;

    let mut x = 0;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width == 0 {
            continue;
        }
        if col < x + width {
            return x + width;
        }
        x += width;
    }
    x
}

fn slice_display_range(text: &str, start: usize, end: usize) -> String {
    use unicode_width::UnicodeWidthChar;

    if start >= end {
        return String::new();
    }
    let mut x = 0;
    let mut byte_start = None;
    let mut byte_end = 0;
    let mut selected = false;
    for (byte, ch) in text.char_indices() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width == 0 {
            if selected {
                byte_end = byte + ch.len_utf8();
            }
            continue;
        }
        if x >= end && selected {
            break;
        }
        if x < end && x + width > start {
            byte_start.get_or_insert(byte);
            byte_end = byte + ch.len_utf8();
            selected = true;
        }
        x += width;
    }
    byte_start
        .map(|from| text[from..byte_end].to_string())
        .unwrap_or_default()
}

fn trim_horizontal_end(text: &mut String) {
    while text.ends_with(' ') || text.ends_with('\t') {
        text.pop();
    }
}

fn clean_selected_text(text: &str) -> String {
    text.split('\n')
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}

/// Tools whose activity belongs in a Subagent transcript card, not a tool row.
fn is_subagent_tool(name: &str) -> bool {
    matches!(name, "verify_project" | "spawn_subagent" | "spawn_swarm")
}

fn is_image_path(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    )
}

fn media_type_for(path: &str) -> String {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "image/png",
    }
    .to_string()
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Parse pasted / drag-and-dropped text into candidate local file paths.
///
/// Terminals emit dropped files in inconsistent ways: a bare path, a quoted
/// path, a path with backslash-escaped spaces (kitty / iTerm2), or one or more
/// `file://` URIs with percent-encoding (GNOME / VTE). This normalizes all of
/// those. Returns `None` when the text clearly isn't just path(s) — e.g.
/// multi-line prose — so the caller can treat it as an ordinary text paste.
fn drop_path_candidates(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // A `file://` URI list — possibly several, one per line (text/uri-list).
    if trimmed.starts_with("file://") {
        let mut out = Vec::new();
        for tok in trimmed.split_whitespace() {
            let rest = tok.strip_prefix("file://")?;
            // Skip an optional host component: everything up to the first '/'.
            let slash = rest.find('/')?;
            out.push(decode_file_uri_path(&rest[slash..]));
        }
        return (!out.is_empty()).then_some(out);
    }

    // Otherwise a single path. Multi-line text is prose, not a dropped path.
    if trimmed.contains('\n') {
        return None;
    }
    let unquoted = strip_matching_quotes(trimmed);
    Some(vec![unescape_backslashes(unquoted)])
}

/// Strip one layer of matching surrounding quotes (`"…"` or `'…'`).
fn strip_matching_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Drop shell escape backslashes (`My\ Photos` → `My Photos`).
fn unescape_backslashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Percent-decode a `file://` path, normalizing a Windows `/C:/…` drive prefix.
fn decode_file_uri_path(path: &str) -> String {
    let decoded = percent_decode(path);
    let bytes = decoded.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        return decoded[1..].to_string();
    }
    decoded
}

/// Minimal percent-decoding for `file://` URIs (`%20` → space, etc.).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TuiInit;
    use hive_core::event::{AgentEvent, SubagentLine, SubagentStatus};
    use hive_core::{TerminalController, TerminalProcessState};

    fn app() -> App {
        App::new(TuiInit {
            model: "m".into(),
            model_display: "m".into(),
            model_choices: Vec::new(),
            skills: Vec::new(),
            connections: Vec::new(),
            active_connection: String::new(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
            ui: Default::default(),
            context_window: 128_000,
            cost_input: 0.0,
            cost_output: 0.0,
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
    fn background_running_terminal_does_not_force_animation() {
        let mut a = app();
        a.apply(AgentEvent::TerminalStarted {
            id: "term-1".into(),
            command: "sleep 999".into(),
            rows: 20,
            cols: 80,
        });
        a.apply(AgentEvent::TerminalState {
            id: "term-1".into(),
            controller: TerminalController::Agent,
            process: TerminalProcessState::Running,
            revision: 1,
        });
        assert!(!a.needs_animation());
        a.open_terminal_view("term-1".into());
        assert!(a.needs_animation());
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
        assert_eq!(a.agent_mode, AgentMode::Make);
        a.toggle_agent_mode();
        assert_eq!(a.agent_mode, AgentMode::Plan);
        a.toggle_agent_mode();
        assert_eq!(a.agent_mode, AgentMode::Multitask);
        a.toggle_agent_mode();
        assert_eq!(a.agent_mode, AgentMode::Make);
    }

    #[test]
    fn mode_switched_updates_chip_and_adds_card() {
        use hive_core::AgentMode;

        let mut a = app();
        assert_eq!(a.agent_mode, AgentMode::Make);
        assert!(a.apply(AgentEvent::ModeSwitched {
            mode: AgentMode::Plan,
            reason: "This is a large multi-part feature.".into(),
        }));
        // Footer chip / next-turn default now follow the agent's choice.
        assert_eq!(a.agent_mode, AgentMode::Plan);
        let Some(Block::ModeSwitch(card)) = a.blocks.last() else {
            panic!("expected a mode-switch card");
        };
        assert_eq!(card.mode, AgentMode::Plan);
        assert!(card.reason.contains("large multi-part"));

        // The card renders the title and the reason in the transcript.
        let buf = comb::render(comb::Size::new(120, 40), |f| crate::render::draw(f, &mut a));
        let text = buf.text();
        assert!(text.contains("Switched to Plan Mode"), "{text}");
        assert!(text.contains("large multi-part"), "{text}");
    }

    #[test]
    fn terminal_events_update_one_persistent_card() {
        let mut app = app();
        app.apply(AgentEvent::TerminalStarted {
            id: "term-1".into(),
            command: "theme-installer".into(),
            rows: 30,
            cols: 120,
        });
        app.apply(AgentEvent::TerminalOutput {
            id: "term-1".into(),
            frame: hive_core::TerminalOutputFrame::from_bytes(
                30,
                120,
                10_000,
                b"Choose preset:\r\n",
                1,
            ),
        });
        app.apply(AgentEvent::TerminalState {
            id: "term-1".into(),
            controller: TerminalController::Agent,
            process: TerminalProcessState::Running,
            revision: 1,
        });

        let cards: Vec<_> = app
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::Terminal(card) => Some(card),
                _ => None,
            })
            .collect();
        assert_eq!(cards.len(), 1);
        assert!(cards[0].preview().contains("Choose preset"));
    }

    #[test]
    fn failed_terminal_start_creates_a_visible_failed_card() {
        let mut app = app();
        app.apply(AgentEvent::TerminalStartFailed {
            id: "term-failed".into(),
            command: "installer".into(),
            message: "cannot spawn".into(),
        });

        let card = app.terminal_card("term-failed").unwrap();
        assert!(matches!(card.process, TerminalProcessState::Failed { .. }));
        assert!(card.status_text().contains("cannot spawn"));
    }

    #[test]
    fn terminal_tool_calls_do_not_create_generic_tool_cards() {
        let mut app = app();
        for name in [
            "terminal_start",
            "terminal_read",
            "terminal_write",
            "terminal_stop",
        ] {
            app.apply(AgentEvent::ToolStarted {
                id: format!("{name}-call"),
                name: name.into(),
                args_preview: "term-1".into(),
            });
        }
        assert!(!app
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Tool(_))));
    }

    #[test]
    fn tool_output_truncation_preserves_utf8_boundaries() {
        let mut a = app();
        a.apply(AgentEvent::ToolStarted {
            id: "shell".into(),
            name: "run_shell".into(),
            args_preview: String::new(),
        });
        a.apply(AgentEvent::ToolOutput {
            id: "shell".into(),
            chunk: format!("{}x", "я".repeat(4000)),
        });

        let Block::Tool(card) = a.blocks.last().expect("tool card") else {
            panic!("expected tool card");
        };
        assert!(card.output.len() <= 8000);
        assert!(card.output.is_char_boundary(0));
    }

    #[test]
    fn plan_rewrite_moves_single_card_to_bottom_with_updated_badge() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        let plans = |a: &App| {
            a.blocks
                .iter()
                .filter(|b| matches!(b, Block::Plan(_)))
                .count()
        };

        // First plan: one card, no badge.
        a.apply(AgentEvent::ToolStarted {
            id: "t1".into(),
            name: "write_plan".into(),
            args_preview: "Initial plan".into(),
        });
        a.apply(AgentEvent::PlanUpdated {
            summary: "Initial".into(),
            body: "## Alpha\n\nx\n".into(),
        });
        assert_eq!(plans(&a), 1);

        // Conversation continues, then the agent rewrites the plan — twice.
        a.push_user("note: change it".into());
        for (id, summary, body) in [
            ("t2", "Revised", "## Alpha\n\nrewritten\n"),
            ("t3", "Revised again", "## Alpha\n\nrewritten again\n"),
        ] {
            a.apply(AgentEvent::ToolStarted {
                id: id.into(),
                name: "write_plan".into(),
                args_preview: summary.into(),
            });
            a.apply(AgentEvent::PlanUpdated {
                summary: summary.into(),
                body: body.into(),
            });
        }

        // Still exactly one card; it moved below the user note and is badged.
        assert_eq!(plans(&a), 1, "rewrites must not stack extra plan cards");
        let Some(Block::Plan(card)) = a.blocks.last() else {
            panic!("expected the plan card at the bottom");
        };
        assert!(card.revised, "card carries the UPDATED badge");
        assert_eq!(card.summary, "Revised again");
        assert_eq!(a.plan_card().unwrap().summary, "Revised again");
    }

    #[test]
    fn plan_rewrite_in_view_pins_top_and_flashes() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::PlanUpdated {
            summary: "A".into(),
            body: "## Alpha\n\nx\n".into(),
        });
        a.open_plan_view();
        a.scroll_from_bottom = 0; // pretend the reader scrolled to the bottom
        a.apply(AgentEvent::PlanUpdated {
            summary: "A2".into(),
            body: "## Alpha\n\nrewritten\n".into(),
        });
        assert_eq!(a.scroll_from_bottom, usize::MAX, "rewrite pins to top");
        assert_eq!(a.flash_text(), Some("plan revised"));
    }

    #[test]
    fn plan_corrections_message_lists_excerpts() {
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::PlanUpdated {
            summary: "A".into(),
            body: "## Alpha\n\nx\n\n## Beta\n\ny\n".into(),
        });
        a.open_plan_view();
        a.plan_view.corrections.push(PlanCorrection {
            start: 0,
            end: 8,
            excerpt: "## Alpha".into(),
            note: "make it shorter".into(),
        });
        let msg = a.plan_corrections_message();
        assert!(msg.contains("## Alpha"), "{msg}");
        assert!(msg.contains("make it shorter"), "{msg}");
        assert!(msg.contains(".hive/Plan.md"), "{msg}");
    }

    #[test]
    fn idle_timeout_blurs_focused_input() {
        let mut a = app();
        assert!(a.input_focused);
        a.input_last_activity = std::time::Instant::now().checked_sub(
            std::time::Duration::from_millis((INPUT_IDLE_BLUR_MS + 50) as u64),
        );
        assert!(a.tick(), "idle blur should request a redraw");
        assert!(!a.input_focused);
        assert!(a.input_last_activity.is_none());
    }

    #[test]
    fn transcript_scroll_clamps_to_max() {
        let mut a = app();
        a.set_transcript_max_scroll(3);
        a.scroll_up(100);
        assert_eq!(a.scroll_from_bottom, 3);
        a.set_transcript_max_scroll(1);
        assert_eq!(
            a.scroll_from_bottom, 1,
            "shrinking max must unpin phantom offset"
        );
        assert!(!a.can_scroll_up());
        assert!(a.can_scroll_down());
        a.scroll_down(1);
        assert_eq!(a.scroll_from_bottom, 0);
        assert!(!a.can_scroll_down());
    }

    #[test]
    fn recent_composer_activity_keeps_focus() {
        let mut a = app();
        a.focus_input();
        assert!(!a.tick());
        assert!(a.input_focused);

        a.input_last_activity = std::time::Instant::now().checked_sub(
            std::time::Duration::from_millis((INPUT_IDLE_BLUR_MS + 50) as u64),
        );
        a.note_input_activity();
        assert!(!a.tick());
        assert!(a.input_focused);
    }

    #[test]
    fn percent_decode_handles_spaces_and_partials() {
        assert_eq!(super::percent_decode("a%20b"), "a b");
        assert_eq!(super::percent_decode("%2Fx%2Fy"), "/x/y");
        // Incomplete / invalid escapes are left untouched.
        assert_eq!(super::percent_decode("a%2"), "a%2");
        assert_eq!(super::percent_decode("a%zz"), "a%zz");
    }

    #[test]
    fn strips_quotes_and_escapes() {
        assert_eq!(super::strip_matching_quotes("\"/a b/c.png\""), "/a b/c.png");
        assert_eq!(super::strip_matching_quotes("'/a/c.png'"), "/a/c.png");
        assert_eq!(super::strip_matching_quotes("/a/c.png"), "/a/c.png");
        assert_eq!(
            super::unescape_backslashes("/a/My\\ Photos/c.png"),
            "/a/My Photos/c.png"
        );
    }

    #[test]
    fn drop_candidates_cover_terminal_quirks() {
        // Plain path.
        assert_eq!(
            super::drop_path_candidates("/a/b.png"),
            Some(vec!["/a/b.png".to_string()])
        );
        // Quoted path with spaces.
        assert_eq!(
            super::drop_path_candidates("\"/a/My Photos/b.png\""),
            Some(vec!["/a/My Photos/b.png".to_string()])
        );
        // Backslash-escaped spaces (kitty / iTerm2).
        assert_eq!(
            super::drop_path_candidates("/a/My\\ Photos/b.png"),
            Some(vec!["/a/My Photos/b.png".to_string()])
        );
        // Percent-encoded file:// URI (GNOME / VTE).
        assert_eq!(
            super::drop_path_candidates("file:///a/My%20Photos/b.png"),
            Some(vec!["/a/My Photos/b.png".to_string()])
        );
        // Multiple file:// URIs from a multi-file drop.
        assert_eq!(
            super::drop_path_candidates("file:///a/x.png\nfile:///a/y.png"),
            Some(vec!["/a/x.png".to_string(), "/a/y.png".to_string()])
        );
        // Multi-line prose is not treated as a path.
        assert_eq!(super::drop_path_candidates("hello\nworld"), None);
        assert_eq!(super::drop_path_candidates("   "), None);
    }

    #[test]
    fn dropped_path_with_spaces_attaches_as_chip() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir()
            .join(format!("hive-drop-{n}"))
            .join("My Photos");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shot.png");
        std::fs::write(&file, b"fakepng").unwrap();

        let mut a = app();
        // Quoted absolute path, as many terminals emit on drop.
        let quoted = format!("\"{}\"", file.display());
        assert_eq!(a.try_attach_pasted_path(&quoted), Some(String::new()));
        assert!(a.has_pending_attaches());
        // Chip shows the filename (label ends with the dropped file name).
        assert!(a.attachment_tags_line().ends_with("shot.png"));

        // Non-file text is left for the composer.
        assert_eq!(a.try_attach_pasted_path("just some text"), None);

        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("hive-drop-{n}")));
    }

    fn assistant_hit(
        block: usize,
        response_row: usize,
        screen_row: u16,
        text: &str,
        join_before: crate::app::state::AssistantRowJoin,
    ) -> crate::app::state::AssistantRowHit {
        crate::app::state::AssistantRowHit {
            block,
            response_row,
            screen_row,
            x: 2,
            text: text.into(),
            join_before,
        }
    }

    #[test]
    fn assistant_selection_extracts_partial_visible_text() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![assistant_hit(
            3,
            0,
            8,
            "hello world",
            AssistantRowJoin::Hard,
        )];

        assert!(a.start_assistant_selection(2, 8));
        assert!(a.update_assistant_selection(6, 8));
        assert_eq!(a.finish_assistant_selection(6, 8).as_deref(), Some("hello"));
    }

    #[test]
    fn assistant_selection_reconstructs_soft_and_hard_breaks() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![
            assistant_hit(3, 0, 8, "hello", AssistantRowJoin::Hard),
            assistant_hit(3, 1, 9, "world", AssistantRowJoin::SoftSpace),
            assistant_hit(3, 2, 10, "code", AssistantRowJoin::Hard),
        ];

        assert!(a.start_assistant_selection(2, 8));
        assert!(a.update_assistant_selection(5, 10));
        assert_eq!(
            a.finish_assistant_selection(5, 10).as_deref(),
            Some("hello world\ncode")
        );
    }

    #[test]
    fn assistant_selection_preserves_blank_paragraph_rows() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![
            assistant_hit(3, 0, 8, "hello", AssistantRowJoin::Hard),
            assistant_hit(3, 1, 9, "", AssistantRowJoin::Hard),
            assistant_hit(3, 2, 10, "world", AssistantRowJoin::Hard),
        ];

        assert!(a.start_assistant_selection(2, 8));
        assert!(a.update_assistant_selection(6, 10));
        assert_eq!(
            a.finish_assistant_selection(6, 10).as_deref(),
            Some("hello\n\nworld")
        );
    }

    #[test]
    fn assistant_selection_drops_visual_indent_on_soft_wraps() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![
            assistant_hit(3, 0, 8, "  bullet text", AssistantRowJoin::Hard),
            assistant_hit(3, 1, 9, "  continues", AssistantRowJoin::SoftSpace),
        ];

        assert!(a.start_assistant_selection(2, 8));
        assert!(a.update_assistant_selection(12, 9));
        assert_eq!(
            a.finish_assistant_selection(12, 9).as_deref(),
            Some("  bullet text continues")
        );
    }

    #[test]
    fn assistant_selection_supports_reverse_drag() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![
            assistant_hit(3, 0, 8, "hello", AssistantRowJoin::Hard),
            assistant_hit(3, 1, 9, "world", AssistantRowJoin::SoftSpace),
        ];

        assert!(a.start_assistant_selection(6, 9));
        assert!(a.update_assistant_selection(2, 8));
        assert_eq!(
            a.finish_assistant_selection(2, 8).as_deref(),
            Some("hello world")
        );
    }

    #[test]
    fn assistant_selection_clamps_to_starting_response() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![
            assistant_hit(3, 0, 8, "first", AssistantRowJoin::Hard),
            assistant_hit(3, 1, 9, "answer", AssistantRowJoin::Hard),
            assistant_hit(4, 0, 10, "other", AssistantRowJoin::Hard),
        ];

        assert!(a.start_assistant_selection(2, 8));
        assert!(a.update_assistant_selection(6, 10));
        assert_eq!(
            a.finish_assistant_selection(6, 10).as_deref(),
            Some("first\nanswer")
        );
    }

    #[test]
    fn assistant_selection_rejects_incomplete_row_coverage() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![
            assistant_hit(3, 0, 8, "first", AssistantRowJoin::Hard),
            assistant_hit(3, 2, 9, "third", AssistantRowJoin::Hard),
        ];

        assert!(a.start_assistant_selection(2, 8));
        assert!(a.update_assistant_selection(6, 9));
        assert_eq!(a.finish_assistant_selection(6, 9), None);
        assert!(a.assistant_selection.is_none());
    }

    #[test]
    fn assistant_click_without_drag_does_not_copy() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![assistant_hit(3, 0, 8, "hello", AssistantRowJoin::Hard)];

        assert!(a.start_assistant_selection(2, 8));
        assert_eq!(a.finish_assistant_selection(2, 8), None);
        assert!(a.assistant_selection.is_none());
    }

    #[test]
    fn completed_assistant_selection_is_consumed_and_cannot_be_reused() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![assistant_hit(
            3,
            0,
            8,
            "hello world",
            AssistantRowJoin::Hard,
        )];

        assert!(a.start_assistant_selection(2, 8));
        assert!(a.update_assistant_selection(6, 8));
        assert_eq!(a.finish_assistant_selection(6, 8).as_deref(), Some("hello"));
        assert!(!a.update_assistant_selection(7, 8));
        assert_eq!(a.finish_assistant_selection(7, 8), None);
        assert!(
            a.assistant_selection.is_none(),
            "highlight disappears after copying"
        );
    }

    #[test]
    fn moving_a_plan_card_clears_block_indexed_assistant_selection() {
        use crate::app::state::{AssistantRowJoin, Block};
        use hive_core::event::AgentEvent;

        let mut a = app();
        a.apply(AgentEvent::PlanUpdated {
            summary: "first".into(),
            body: "# Plan\n\nOld".into(),
        });
        a.blocks.push(Block::Assistant {
            text: "answer".into(),
            streaming: false,
        });
        let block = a.blocks.len() - 1;
        a.assistant_row_hits = vec![assistant_hit(block, 0, 8, "answer", AssistantRowJoin::Hard)];
        assert!(a.start_assistant_selection(2, 8));
        assert!(a.update_assistant_selection(4, 8));

        a.apply(AgentEvent::PlanUpdated {
            summary: "second".into(),
            body: "# Plan\n\nRewritten".into(),
        });
        assert!(a.assistant_selection.is_none());
    }

    #[test]
    fn assistant_selection_uses_display_columns_for_wide_glyphs() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![assistant_hit(3, 0, 8, "a界b", AssistantRowJoin::Hard)];

        assert!(a.start_assistant_selection(3, 8));
        assert!(a.update_assistant_selection(5, 8));
        assert_eq!(a.finish_assistant_selection(5, 8).as_deref(), Some("界b"));
    }

    #[test]
    fn assistant_selection_width_matches_renderer_for_zwj_sequences() {
        use crate::app::state::AssistantRowJoin;

        let mut a = app();
        a.assistant_row_hits = vec![assistant_hit(3, 0, 8, "👩‍💻x", AssistantRowJoin::Hard)];

        // comb renders the two emoji scalars as two wide glyphs and skips ZWJ,
        // so `x` starts four display columns after the row origin.
        assert!(a.start_assistant_selection(6, 8));
        assert_eq!(
            a.assistant_selection.as_ref().map(|s| s.anchor.col),
            Some(4)
        );
    }

    #[test]
    fn prompt_history_up_down_cycles() {
        let mut a = app();
        a.prompt_history.push("first");
        a.prompt_history.push("second");
        a.prompt_history.push("third");

        // Up on empty input → most recent entry.
        assert!(a.history_up());
        assert_eq!(a.input.value, "third");

        // Up again → previous entry.
        assert!(a.history_up());
        assert_eq!(a.input.value, "second");

        // Up again → oldest entry.
        assert!(a.history_up());
        assert_eq!(a.input.value, "first");

        // Up at oldest → stays (no-op, returns true to prevent scroll).
        assert!(a.history_up());
        assert_eq!(a.input.value, "first");

        // Down → next entry.
        assert!(a.history_down());
        assert_eq!(a.input.value, "second");

        // Down → most recent.
        assert!(a.history_down());
        assert_eq!(a.input.value, "third");

        // Down past end → exit browsing, restore draft (was empty).
        assert!(a.history_down());
        assert!(a.input.value.is_empty());
        assert!(!a.prompt_history.is_browsing());
    }

    #[test]
    fn prompt_history_saves_and_restores_draft() {
        let mut a = app();
        a.prompt_history.push("old prompt");
        a.input.value = "my draft".into();

        // Up enters history (cursor at top of non-empty input).
        assert!(a.history_up());
        assert_eq!(a.input.value, "old prompt");

        // Down past end restores the draft.
        assert!(a.history_down());
        assert_eq!(a.input.value, "my draft");
    }

    #[test]
    fn prompt_history_push_dedups_last() {
        let mut a = app();
        a.prompt_history.push("hello");
        a.prompt_history.push("hello");
        assert_eq!(a.prompt_history.entries.len(), 1);
        a.prompt_history.push("world");
        assert_eq!(a.prompt_history.entries.len(), 2);
    }

    #[test]
    fn pending_dispatch_recall_restores_input_and_removes_block() {
        let mut a = app();
        a.blocks.clear();
        a.input.value = "test prompt".into();
        a.pending_dispatch = Some(PendingDispatch {
            agent_text: "test prompt".into(),
            images: Vec::new(),
            mode: AgentMode::Make,
            composer: "test prompt".into(),
            attaches: Vec::new(),
            submitted_at: std::time::Instant::now(),
        });
        a.push_user("test prompt".into());
        assert_eq!(a.blocks.len(), 1);

        assert!(a.recall_pending_dispatch());
        assert!(a.pending_dispatch.is_none());
        assert_eq!(a.input.value, "test prompt");
        assert!(a.blocks.is_empty(), "user block should be removed");
    }
}
