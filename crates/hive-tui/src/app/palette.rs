//! Command palette, model picker, and `/connect` provider menu.

use hive_core::event::ConnectionInfo;
use hive_core::SessionMeta;

use crate::commands::{self, CmdId, PaletteRow};
use crate::intro::PRESETS;
use crate::ModelChoice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteMode {
    Commands,
    Models,
    /// Saved providers + Add.
    Connect,
    /// Pick a preset to add.
    ConnectPresets,
    /// Paste API key for the chosen preset.
    ConnectKey {
        preset_idx: usize,
    },
    /// Saved sessions picker.
    Sessions,
}

/// One row in the model picker (provider header or selectable model).
#[derive(Debug, Clone, Copy)]
pub enum ModelRow<'a> {
    Header(&'a str),
    Model(&'a ModelChoice),
}

impl ModelRow<'_> {
    pub fn is_selectable(self) -> bool {
        matches!(self, ModelRow::Model(_))
    }
}

/// One row in the `/connect` list.
#[derive(Debug, Clone, Copy)]
pub enum ConnectRow<'a> {
    Profile(&'a ConnectionInfo),
    Add,
    RemoveActive,
}

#[derive(Debug, Clone)]
pub struct PaletteState {
    pub mode: PaletteMode,
    pub query: String,
    /// Byte index into `query` for the search caret.
    pub cursor: usize,
    /// Index into the current row list.
    pub selected: usize,
    /// First visible row in the scrollable list.
    pub list_offset: usize,
    /// Search field has the caret.
    pub search_focused: bool,
}

impl PaletteState {
    pub fn commands() -> Self {
        let rows = commands::palette_rows("");
        PaletteState {
            mode: PaletteMode::Commands,
            query: String::new(),
            cursor: 0,
            selected: commands::first_selectable(&rows),
            list_offset: 0,
            search_focused: false,
        }
    }

    pub fn models() -> Self {
        PaletteState {
            mode: PaletteMode::Models,
            query: String::new(),
            cursor: 0,
            selected: 0,
            list_offset: 0,
            search_focused: false,
        }
    }

    pub fn connect() -> Self {
        PaletteState {
            mode: PaletteMode::Connect,
            query: String::new(),
            cursor: 0,
            selected: 0,
            list_offset: 0,
            search_focused: false,
        }
    }

    pub fn connect_presets() -> Self {
        PaletteState {
            mode: PaletteMode::ConnectPresets,
            query: String::new(),
            cursor: 0,
            selected: 0,
            list_offset: 0,
            search_focused: false,
        }
    }

    pub fn connect_key(preset_idx: usize) -> Self {
        PaletteState {
            mode: PaletteMode::ConnectKey { preset_idx },
            query: String::new(),
            cursor: 0,
            selected: 0,
            list_offset: 0,
            search_focused: true,
        }
    }

    pub fn sessions() -> Self {
        PaletteState {
            mode: PaletteMode::Sessions,
            query: String::new(),
            cursor: 0,
            selected: 0,
            list_offset: 0,
            search_focused: false,
        }
    }

    pub fn ensure_selection_visible(
        &mut self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
        visible: usize,
    ) {
        let (len, vis) = self.scroll_metrics(choices, connections, sessions, visible);
        if len == 0 || vis == 0 {
            self.list_offset = 0;
            return;
        }
        let sel = self.selected.min(len - 1);
        let max_off = len.saturating_sub(vis);
        let mut off = self.list_offset.min(max_off);
        if sel < off {
            off = sel;
        } else if sel >= off + vis {
            off = sel + 1 - vis;
        }
        self.list_offset = off.min(max_off);
    }

    fn scroll_metrics(
        &self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
        visible: usize,
    ) -> (usize, usize) {
        let len = match self.mode {
            PaletteMode::Commands => self.command_rows().len(),
            PaletteMode::Models => self.model_rows(choices).len(),
            PaletteMode::Connect => self.connect_rows(connections).len(),
            PaletteMode::ConnectPresets => self.preset_indices().len(),
            PaletteMode::ConnectKey { .. } => 0,
            PaletteMode::Sessions => self.session_rows(sessions).len(),
        };
        (len, visible)
    }

    pub fn focus_search(&mut self) {
        self.search_focused = true;
    }

    pub fn command_rows(&self) -> Vec<PaletteRow> {
        commands::palette_rows(&self.query)
    }

    pub fn model_rows<'a>(&self, choices: &'a [ModelChoice]) -> Vec<ModelRow<'a>> {
        let q = self.query.trim().to_ascii_lowercase();
        let filtered: Vec<&'a ModelChoice> = if q.is_empty() {
            choices.iter().collect()
        } else {
            choices
                .iter()
                .filter(|c| {
                    c.key.to_ascii_lowercase().contains(&q)
                        || c.display.to_ascii_lowercase().contains(&q)
                        || c.detail.to_ascii_lowercase().contains(&q)
                        || c.group.to_ascii_lowercase().contains(&q)
                })
                .collect()
        };

        let mut rows = Vec::new();
        let mut last_group: Option<&str> = None;
        for c in filtered {
            let g = if c.group.is_empty() {
                "Models"
            } else {
                c.group.as_str()
            };
            if last_group != Some(g) {
                rows.push(ModelRow::Header(g));
                last_group = Some(g);
            }
            rows.push(ModelRow::Model(c));
        }
        rows
    }

    pub fn connect_rows<'a>(&self, connections: &'a [ConnectionInfo]) -> Vec<ConnectRow<'a>> {
        let q = self.query.trim().to_ascii_lowercase();
        let mut rows: Vec<ConnectRow<'a>> = connections
            .iter()
            .filter(|c| {
                q.is_empty()
                    || c.label.to_ascii_lowercase().contains(&q)
                    || c.detail.to_ascii_lowercase().contains(&q)
                    || c.id.to_ascii_lowercase().contains(&q)
            })
            .map(ConnectRow::Profile)
            .collect();
        rows.push(ConnectRow::Add);
        if connections.len() > 1 {
            rows.push(ConnectRow::RemoveActive);
        }
        rows
    }

    /// Indices into [`PRESETS`] (Custom last, filterable).
    pub fn preset_indices(&self) -> Vec<usize> {
        let q = self.query.trim().to_ascii_lowercase();
        PRESETS
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                !p.is_custom
                    && (q.is_empty()
                        || p.label.to_ascii_lowercase().contains(&q)
                        || p.base_url.to_ascii_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Filtered session rows for the sessions picker.
    pub fn session_rows<'a>(&self, sessions: &'a [SessionMeta]) -> Vec<&'a SessionMeta> {
        let q = self.query.trim().to_ascii_lowercase();
        if q.is_empty() {
            sessions.iter().collect()
        } else {
            sessions
                .iter()
                .filter(|s| {
                    s.title.to_ascii_lowercase().contains(&q)
                        || s.model.to_ascii_lowercase().contains(&q)
                        || s.id.to_ascii_lowercase().contains(&q)
                })
                .collect()
        }
    }

    pub fn clamp_selection(
        &mut self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
    ) {
        match self.mode {
            PaletteMode::Commands => {
                let rows = self.command_rows();
                if rows.is_empty() {
                    self.selected = 0;
                    return;
                }
                if self.selected >= rows.len() || !rows[self.selected].is_selectable() {
                    self.selected = commands::first_selectable(&rows);
                }
            }
            PaletteMode::Models => {
                let rows = self.model_rows(choices);
                if rows.is_empty() {
                    self.selected = 0;
                    return;
                }
                if self.selected >= rows.len() || !rows[self.selected].is_selectable() {
                    self.selected = first_selectable_model(&rows);
                }
            }
            PaletteMode::Connect => {
                let n = self.connect_rows(connections).len();
                if n == 0 {
                    self.selected = 0;
                } else if self.selected >= n {
                    self.selected = n - 1;
                }
            }
            PaletteMode::ConnectPresets => {
                let n = self.preset_indices().len();
                if n == 0 {
                    self.selected = 0;
                } else if self.selected >= n {
                    self.selected = n - 1;
                }
            }
            PaletteMode::ConnectKey { .. } => {
                self.selected = 0;
            }
            PaletteMode::Sessions => {
                let n = self.session_rows(sessions).len();
                if n == 0 {
                    self.selected = 0;
                } else if self.selected >= n {
                    self.selected = n - 1;
                }
            }
        }
    }

    pub fn move_up(
        &mut self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
    ) {
        match self.mode {
            PaletteMode::Commands => {
                let rows = self.command_rows();
                self.selected = commands::move_selection(&rows, self.selected, -1);
            }
            PaletteMode::Models => {
                let rows = self.model_rows(choices);
                self.selected = move_model_selection(&rows, self.selected, -1);
            }
            PaletteMode::Connect => {
                let n = self.connect_rows(connections).len();
                if n > 0 {
                    self.selected = (self.selected + n - 1) % n;
                }
            }
            PaletteMode::ConnectPresets => {
                let n = self.preset_indices().len();
                if n > 0 {
                    self.selected = (self.selected + n - 1) % n;
                }
            }
            PaletteMode::ConnectKey { .. } => {}
            PaletteMode::Sessions => {
                let n = self.session_rows(sessions).len();
                if n > 0 {
                    self.selected = (self.selected + n - 1) % n;
                }
            }
        }
    }

    pub fn move_down(
        &mut self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
    ) {
        match self.mode {
            PaletteMode::Commands => {
                let rows = self.command_rows();
                self.selected = commands::move_selection(&rows, self.selected, 1);
            }
            PaletteMode::Models => {
                let rows = self.model_rows(choices);
                self.selected = move_model_selection(&rows, self.selected, 1);
            }
            PaletteMode::Connect => {
                let n = self.connect_rows(connections).len();
                if n > 0 {
                    self.selected = (self.selected + 1) % n;
                }
            }
            PaletteMode::ConnectPresets => {
                let n = self.preset_indices().len();
                if n > 0 {
                    self.selected = (self.selected + 1) % n;
                }
            }
            PaletteMode::ConnectKey { .. } => {}
            PaletteMode::Sessions => {
                let n = self.session_rows(sessions).len();
                if n > 0 {
                    self.selected = (self.selected + 1) % n;
                }
            }
        }
    }

    pub fn move_up_visible(
        &mut self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
        visible: usize,
    ) {
        self.move_up(choices, connections, sessions);
        self.ensure_selection_visible(choices, connections, sessions, visible);
    }

    pub fn move_down_visible(
        &mut self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
        visible: usize,
    ) {
        self.move_down(choices, connections, sessions);
        self.ensure_selection_visible(choices, connections, sessions, visible);
    }

    pub fn insert(
        &mut self,
        ch: char,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
    ) {
        self.focus_search();
        let idx = self.byte_at(self.cursor);
        self.query.insert(idx, ch);
        self.cursor += 1;
        self.after_query_change(choices, connections, sessions);
    }

    /// Insert a multi-char paste at the cursor (API key / search filter).
    pub fn insert_str(
        &mut self,
        text: &str,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
    ) {
        if text.is_empty() {
            return;
        }
        self.focus_search();
        let idx = self.byte_at(self.cursor);
        let n = text.chars().count();
        self.query.insert_str(idx, text);
        self.cursor += n;
        self.after_query_change(choices, connections, sessions);
    }

    pub fn backspace(
        &mut self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
    ) {
        if !self.search_focused {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let start = self.byte_at(self.cursor - 1);
        let end = self.byte_at(self.cursor);
        self.query.replace_range(start..end, "");
        self.cursor -= 1;
        self.after_query_change(choices, connections, sessions);
    }

    pub fn left(&mut self) {
        if !self.search_focused {
            return;
        }
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if !self.search_focused {
            return;
        }
        if self.cursor < self.query.chars().count() {
            self.cursor += 1;
        }
    }

    fn after_query_change(
        &mut self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
    ) {
        self.list_offset = 0;
        match self.mode {
            PaletteMode::Commands => {
                let rows = self.command_rows();
                self.selected = commands::first_selectable(&rows);
            }
            PaletteMode::Models => {
                let rows = self.model_rows(choices);
                self.selected = first_selectable_model(&rows);
            }
            PaletteMode::Connect | PaletteMode::ConnectPresets => {
                self.selected = 0;
                self.clamp_selection(choices, connections, sessions);
            }
            PaletteMode::Sessions => {
                self.selected = 0;
                self.clamp_selection(choices, connections, sessions);
            }
            PaletteMode::ConnectKey { .. } => {}
        }
    }

    pub fn visible_offset(
        &self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
        visible: usize,
    ) -> usize {
        let (len, vis) = self.scroll_metrics(choices, connections, sessions, visible);
        if len == 0 || vis == 0 {
            return 0;
        }
        self.list_offset.min(len.saturating_sub(vis))
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.query
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.query.len())
    }

    pub fn selected_command(&self) -> Option<&'static commands::CommandDef> {
        if self.mode != PaletteMode::Commands {
            return None;
        }
        let rows = self.command_rows();
        rows.get(self.selected).and_then(|r| r.command())
    }

    pub fn selected_model<'a>(&self, choices: &'a [ModelChoice]) -> Option<&'a ModelChoice> {
        if self.mode != PaletteMode::Models {
            return None;
        }
        let rows = self.model_rows(choices);
        match rows.get(self.selected) {
            Some(ModelRow::Model(c)) => Some(*c),
            _ => None,
        }
    }

    pub fn selected_connect<'a>(
        &self,
        connections: &'a [ConnectionInfo],
    ) -> Option<ConnectRow<'a>> {
        if self.mode != PaletteMode::Connect {
            return None;
        }
        self.connect_rows(connections).get(self.selected).copied()
    }

    pub fn selected_preset_idx(&self) -> Option<usize> {
        if self.mode != PaletteMode::ConnectPresets {
            return None;
        }
        self.preset_indices().get(self.selected).copied()
    }

    pub fn selected_session<'a>(&self, sessions: &'a [SessionMeta]) -> Option<&'a SessionMeta> {
        if self.mode != PaletteMode::Sessions {
            return None;
        }
        self.session_rows(sessions).get(self.selected).copied()
    }

    pub fn selected_cmd_id(&self) -> Option<CmdId> {
        self.selected_command().map(|c| c.id)
    }
}

fn first_selectable_model(rows: &[ModelRow<'_>]) -> usize {
    rows.iter().position(|r| r.is_selectable()).unwrap_or(0)
}

fn move_model_selection(rows: &[ModelRow<'_>], current: usize, dir: isize) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let n = rows.len() as isize;
    let mut i = current as isize;
    for _ in 0..rows.len() {
        i = (i + dir).rem_euclid(n);
        if rows[i as usize].is_selectable() {
            return i as usize;
        }
    }
    current.min(rows.len() - 1)
}
