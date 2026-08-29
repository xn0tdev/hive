//! The TUI application state and how it reacts to `AgentEvent`s.

pub mod files;
pub mod input;
pub mod palette;
pub mod pasted;
pub mod recap;
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
use crate::render::{
    ProjectRefresh, ProjectSnapshot, SidebarItem, SidebarSection, SidebarSections,
};
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
use recap::RecapOverlay;
use settings::SettingsState;

use input::InputState;
use state::{
    AssistantPoint, AssistantResponseRow, AssistantRowHit, AssistantRowJoin, AssistantSelection,
    Block, ChatView, CompactedCard, ContextAction, ContextMenu, ContextMenuItem, ExploreCard,
    FileSnapshot, ModeSwitchCard, PlanAction, PlanCard, PlanCorrection,
    PlanStatus, PlanViewState, PromptHistory, RecapBody, SubagentCard, TerminalCard,
    TerminalViewPhase, TerminalViewState, Thought, ToolCard, ToolStatus, WorkSummaryCard,
};

/// Cached markdown wraps for finished assistant bodies — avoids re-parsing on
/// every spinner tick / subagent status pulse.
#[derive(Clone)]
pub(crate) struct MdRows {
    pub lines: Vec<Line>,
    pub joins: Vec<AssistantRowJoin>,
}

#[derive(Default)]
pub(crate) struct TranscriptCache {
    pub width: usize,
    pub revision: u64,
    pub animation_epoch: usize,
    pub hover_block: Option<usize>,
    pub show_tool_cards: bool,
    pub lines: Vec<Line>,
    pub heads: Vec<(usize, usize)>,
    pub assistant_rows: Vec<AssistantResponseRow>,
    pub builds: u64,
    pub valid: bool,
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
    /// UTF-8 content for text files (read at attach time).
    pub content: Option<String>,
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
    /// Large pasted blocks restored on ↑ / re-queued on Enter.
    pub pasted: Vec<pasted::PastedBlock>,
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
    /// Large pasted blocks (for restoring on recall).
    pub pasted: Vec<pasted::PastedBlock>,
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
    /// Attachment chip the keyboard is on, once ↓ steps out of the composer.
    pub(crate) attach_selected: Option<usize>,
    /// Large pasted text blocks parked behind inline `[ pasted text N ]` tokens.
    pub(crate) pasted_blocks: Vec<pasted::PastedBlock>,
    pub(crate) pasted_selected: Option<usize>,
    pub(crate) pasted_view: Option<pasted::PastedViewState>,
    /// Next number for a `[ pasted text N ]` token.
    pub(crate) next_pasted_id: usize,
    /// Set by ctrl+L: repaint every cell, not just the diff.
    pub(crate) repaint_requested: bool,
    /// When an image last landed from the clipboard, so the composer can say
    /// so for a moment instead of the chip appearing out of nowhere.
    pub(crate) image_pasted_at: Option<std::time::Instant>,
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
    /// Settings overlay (chat / sidebar / tools / landing).
    pub(crate) settings: Option<SettingsState>,
    /// Recap panel opened from a "Worked for" line.
    pub(crate) recap_overlay: Option<RecapOverlay>,
    /// Click target for the centered Generating label (last draw).
    pub(crate) recap_generating_hit: Option<Rect>,
    /// Next id stamped on a WorkSummaryCard so recap events find it.
    pub(crate) next_recap_id: u64,
    /// Agent's self-managed task list (set_todos tool).
    pub(crate) todos: Vec<hive_core::TodoItem>,
    /// Completed tasks stay visible briefly, then retire from the live UI.
    pub(crate) todos_completed_at: Option<std::time::Instant>,
    /// Saved sessions for the `/resume` picker.
    pub(crate) saved_sessions: Vec<hive_core::SessionMeta>,
    /// Persisted UI prefs (`[ui]` in config.toml).
    pub(crate) ui: UiConfig,
    /// Selected row in the slash / `@file` menu.
    pub(crate) menu_index: usize,
    /// Where the composer menu was last drawn, plus the item its first visible
    /// row shows. `None` when no menu is on screen.
    pub(crate) menu_hit: Option<(Rect, usize)>,
    /// A mouse click asked to quit (clicking `/quit` in the slash menu). The
    /// mouse handler reports redraws, not exits, so it parks the request here.
    pub(crate) quit_requested: bool,
    /// Cached relative file paths for `@` mentions (lazy).
    pub(crate) file_index: Option<Vec<String>>,
    /// Short-lived Info toast (bottom-right chip, not the chat).
    pub(crate) flash_msg: Option<(String, std::time::Instant)>,
    /// Short-lived Error toast, stacked above Info when both are up.
    pub(crate) error_flash: Option<(String, std::time::Instant)>,
    /// Set on the first ctrl+c; a second press within the window quits.
    pub(crate) ctrl_c_armed: Option<std::time::Instant>,
    /// Animation clock: frames derive from elapsed time, so the spinner and
    /// shimmer run at a constant speed no matter how often events arrive.
    pub(crate) anim_start: std::time::Instant,
    /// Context-menu panel and item rects from the last draw, so the mouse can
    /// pick an action or dismiss the menu.
    pub(crate) context_menu_win: Option<Rect>,
    pub(crate) context_menu_hits: Vec<(Rect, usize)>,
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
    /// Jump-to-bottom chip hovered.
    pub(crate) hover_scroll_bottom: bool,
    /// Hit target for the jump-to-bottom chip (last draw). `None` when the
    /// transcript is already at the bottom, so the chip isn't there.
    pub(crate) scroll_bottom_hit: Option<Rect>,
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
    /// Context window update waiting to be sent to the agent driver.
    pub(crate) pending_context_update: Option<u64>,
    /// Finished-assistant markdown cache (invalidated on width change).
    pub(crate) md_cache: MdCache,
    /// Retained main transcript layout. Stable history is not rebuilt for every footer frame.
    pub(crate) transcript_cache: TranscriptCache,
    pub(crate) transcript_revision: u64,
    /// Cached git project / diff summary for the right sidebar.
    pub(crate) project: ProjectSnapshot,
    /// Coalescing background loader; Git and filesystem scans never run while drawing.
    pub(crate) project_refresh: ProjectRefresh,
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
    /// Frame rect the centered overlays were last laid out against. Their
    /// geometry is a pure function of it, so mouse hit-testing re-derives
    /// rather than caching a rect per row.
    pub(crate) overlay_area: Rect,
    /// Centered context menu for the clicked transcript block.
    pub(crate) context_menu: Option<ContextMenu>,
    /// When the current turn started (for the "Worked for Nm" summary).
    pub(crate) turn_started_at: Option<std::time::Instant>,
}

/// How long the ctrl+c confirmation window lives.
pub const FLASH_MS: u128 = 1500;
/// Toast lifetime (Copied, New chat, Ctrl+C, etc.).
pub const TOAST_MS: u128 = 2_000;
/// Keep the completed task state long enough to acknowledge it without
/// leaving permanent progress chrome in the transcript/sidebar.
pub const TASKS_DONE_VISIBLE_MS: u128 = 2_000;
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
            attach_selected: None,
            pasted_blocks: Vec::new(),
            pasted_selected: None,
            pasted_view: None,
            next_pasted_id: 1,
            repaint_requested: false,
            image_pasted_at: None,
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
            recap_overlay: None,
            recap_generating_hit: None,
            next_recap_id: 1,
            todos: Vec::new(),
            todos_completed_at: None,
            saved_sessions: Vec::new(),
            ui: init.ui.clone(),
            menu_index: 0,
            menu_hit: None,
            quit_requested: false,
            file_index: None,
            flash_msg: None,
            error_flash: None,
            ctrl_c_armed: None,
            anim_start: std::time::Instant::now(),
            context_menu_win: None,
            context_menu_hits: Vec::new(),
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
            hover_scroll_bottom: false,
            scroll_bottom_hit: None,
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
            pending_context_update: None,
            md_cache: MdCache::default(),
            transcript_cache: TranscriptCache::default(),
            transcript_revision: 1,
            project: ProjectSnapshot::default(),
            project_refresh: ProjectRefresh::new(),
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
            overlay_area: Rect::new(0, 0, 0, 0),
            context_menu: None,
            turn_started_at: None,
        };
        app.blocks.push(Block::Welcome);
        app.project_refresh.request(&app.cwd);
        app
    }
}

mod animation;
mod composer;
mod event_blocks;
mod events;
mod helpers;
mod navigation;
mod overlays;
mod plan;
mod selection;

use helpers::*;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
