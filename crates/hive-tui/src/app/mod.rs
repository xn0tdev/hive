//! The TUI application state and how it reacts to `AgentEvent`s.

pub mod input;
pub mod state;

use comb::Rect;
use hive_core::event::{AgentEvent, SubagentLine, SubagentStatus};
use hive_core::message::ImageSource;
use hive_core::provider::Usage;

use crate::commands;
use crate::render::mascot::LogoBonk;
use crate::render::spinner;
use crate::theme::Theme;
use crate::TuiInit;

use input::InputState;
use state::{Block, ChatView, SubagentCard, Thought, ToolCard, ToolStatus};

pub struct App {
    pub(crate) blocks: Vec<Block>,
    pub(crate) input: InputState,
    pub(crate) theme: Theme,
    pub(crate) model: String,
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
    /// Main transcript vs a read-only subagent chat.
    pub(crate) view: ChatView,
    /// Screen row of the `← back` control in subagent view (last draw).
    pub(crate) back_hit_row: Option<u16>,
}

/// How long a flash message / the ctrl+c confirmation window lives.
pub const FLASH_MS: u128 = 1500;

impl App {
    pub fn new(init: TuiInit) -> Self {
        let theme = Theme::from_name(&init.theme);
        let mut app = App {
            blocks: Vec::new(),
            input: InputState::default(),
            theme,
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
            back_hit_row: None,
        };
        app.blocks.push(Block::Welcome);
        app
    }

    /// True while viewing a subagent thread (read-only; no input).
    pub fn in_subagent_view(&self) -> bool {
        matches!(self.view, ChatView::Subagent(_))
    }

    /// The subagent card currently being viewed, if any.
    pub fn viewed_subagent(&self) -> Option<&SubagentCard> {
        match &self.view {
            ChatView::Main => None,
            ChatView::Subagent(id) => self.blocks.iter().find_map(|b| match b {
                Block::Subagent(card) if card.id == *id => Some(card),
                _ => None,
            }),
        }
    }

    /// Open the read-only chat view for a subagent card.
    pub fn open_subagent_view(&mut self, id: String) {
        self.view = ChatView::Subagent(id);
        self.scroll_from_bottom = 0;
        self.back_hit_row = None;
    }

    /// Return to the main agent transcript.
    pub fn leave_subagent_view(&mut self) {
        self.view = ChatView::Main;
        self.scroll_from_bottom = 0;
        self.back_hit_row = None;
    }

    pub fn spinner_char(&self) -> &'static str {
        spinner::frame(self.spinner)
    }

    /// Advance the animation frame from wall-clock time. Called every loop
    /// iteration; mouse/key event bursts don't speed the animation up because
    /// the frame is a pure function of elapsed time.
    pub fn tick(&mut self) {
        self.spinner = (self.anim_start.elapsed().as_millis() / 90) as usize;
        if self.logo_bonk.as_ref().is_some_and(|b| !b.alive()) {
            self.logo_bonk = None;
        }
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
        !self.running
            && !self
                .blocks
                .iter()
                .any(|b| !matches!(b, Block::Welcome))
    }

    // --- ephemeral footer messages & double-ctrl+c ---

    pub fn flash(&mut self, msg: impl Into<String>) {
        self.flash_msg = Some((msg.into(), std::time::Instant::now()));
    }

    /// The flash text, if it hasn't expired yet.
    pub fn flash_text(&self) -> Option<&str> {
        match &self.flash_msg {
            Some((msg, at)) if at.elapsed().as_millis() < FLASH_MS => Some(msg.as_str()),
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
        self.flash("press ctrl+c again to exit");
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
        self.back_hit_row = None;
    }

    pub fn add_pending_image(&mut self, src: ImageSource) {
        self.pending_images.push(src);
    }

    pub fn take_pending_images(&mut self) -> Vec<ImageSource> {
        std::mem::take(&mut self.pending_images)
    }

    pub fn apply(&mut self, ev: AgentEvent) {
        match ev {
            AgentEvent::TurnStarted => {
                self.running = true;
            }
            AgentEvent::AssistantStarted => {
                self.close_thought();
                self.finalize_streaming();
            }
            AgentEvent::AssistantTextDelta(t) => self.append_assistant(&t),
            AgentEvent::ReasoningDelta(t) => self.append_reasoning(&t),
            AgentEvent::AssistantMessage(text) => self.finalize_assistant(text),
            AgentEvent::ToolStarted {
                id,
                name,
                args_preview,
            } => {
                self.close_thought();
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
            }
            AgentEvent::ToolOutput { id, chunk } => self.append_tool_output(&id, &chunk),
            AgentEvent::ToolFinished { id, ok, .. } => self.finish_tool(&id, ok),
            AgentEvent::Usage(u) => self.usage = u,
            AgentEvent::SubagentSpawned { id, label, prompt } => {
                self.blocks.push(Block::Subagent(SubagentCard {
                    id,
                    label,
                    status: SubagentStatus::Running,
                    detail: String::new(),
                    prompt,
                    lines: Vec::new(),
                    started: std::time::Instant::now(),
                    elapsed_ms: None,
                }));
            }
            AgentEvent::SubagentStatus { id, status, detail } => {
                self.update_subagent(&id, status, detail);
            }
            AgentEvent::SubagentTranscript { id, line } => {
                self.append_subagent_line(&id, line);
            }
            AgentEvent::ModelChanged(m) => self.model = m,
            AgentEvent::Notice(s) => self.blocks.push(Block::Notice(s)),
            AgentEvent::Error(s) => self.blocks.push(Block::Error(s)),
            AgentEvent::TurnFinished => {
                self.running = false;
                self.close_thought();
                self.finalize_streaming();
            }
        }
        // Any new content while following keeps us pinned to the bottom.
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
            "thoughts collapsed"
        } else {
            "thoughts expanded"
        });
    }

    /// Activate an expandable header by index — thoughts toggle; subagents
    /// open their dedicated read-only chat view.
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
            _ => {}
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
    matches!(
        name,
        "verify_project" | "spawn_subagent" | "spawn_swarm"
    )
}
