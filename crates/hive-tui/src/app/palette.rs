//! Command palette and nested model-picker state.

use crate::commands::{self, CmdId, PaletteRow};
use crate::ModelChoice;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteMode {
    Commands,
    Models,
}

#[derive(Debug, Clone)]
pub struct PaletteState {
    pub mode: PaletteMode,
    pub query: String,
    /// Byte index into `query` for the search caret.
    pub cursor: usize,
    /// Index into the current row list (headers + commands).
    pub selected: usize,
    /// First visible row in the scrollable list (keyboard overflow viewport).
    pub list_offset: usize,
    /// Search field has the caret. Starts false so the list can be navigated
    /// first; set when the user types into search.
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

    /// Keep `selected` inside the viewport after keyboard navigation.
    pub fn ensure_selection_visible(&mut self, choices: &[ModelChoice], visible: usize) {
        let (len, vis) = self.scroll_metrics(choices, visible);
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

    fn scroll_metrics(&self, choices: &[ModelChoice], visible: usize) -> (usize, usize) {
        match self.mode {
            PaletteMode::Commands => (self.command_rows().len(), visible),
            PaletteMode::Models => {
                // Sticky "Models" header consumes one viewport row.
                (
                    self.model_rows(choices).len(),
                    visible.saturating_sub(1),
                )
            }
        }
    }

    pub fn focus_search(&mut self) {
        self.search_focused = true;
    }

    pub fn command_rows(&self) -> Vec<PaletteRow> {
        commands::palette_rows(&self.query)
    }

    pub fn model_rows<'a>(&self, choices: &'a [ModelChoice]) -> Vec<&'a ModelChoice> {
        let q = self.query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return choices.iter().collect();
        }
        choices
            .iter()
            .filter(|c| {
                c.key.to_ascii_lowercase().contains(&q)
                    || c.display.to_ascii_lowercase().contains(&q)
                    || c.detail.to_ascii_lowercase().contains(&q)
            })
            .collect()
    }

    pub fn clamp_selection(&mut self, choices: &[ModelChoice]) {
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
                let n = self.model_rows(choices).len();
                if n == 0 {
                    self.selected = 0;
                } else if self.selected >= n {
                    self.selected = n - 1;
                }
            }
        }
    }

    pub fn move_up(&mut self, choices: &[ModelChoice]) {
        match self.mode {
            PaletteMode::Commands => {
                let rows = self.command_rows();
                self.selected = commands::move_selection(&rows, self.selected, -1);
            }
            PaletteMode::Models => {
                let n = self.model_rows(choices).len();
                if n > 0 {
                    self.selected = (self.selected + n - 1) % n;
                }
            }
        }
    }

    pub fn move_down(&mut self, choices: &[ModelChoice]) {
        match self.mode {
            PaletteMode::Commands => {
                let rows = self.command_rows();
                self.selected = commands::move_selection(&rows, self.selected, 1);
            }
            PaletteMode::Models => {
                let n = self.model_rows(choices).len();
                if n > 0 {
                    self.selected = (self.selected + 1) % n;
                }
            }
        }
    }

    /// Keyboard up/down: move selection and keep it painted in the viewport.
    pub fn move_up_visible(&mut self, choices: &[ModelChoice], visible: usize) {
        self.move_up(choices);
        self.ensure_selection_visible(choices, visible);
    }

    pub fn move_down_visible(&mut self, choices: &[ModelChoice], visible: usize) {
        self.move_down(choices);
        self.ensure_selection_visible(choices, visible);
    }

    pub fn insert(&mut self, ch: char, choices: &[ModelChoice]) {
        self.search_focused = true;
        let idx = self.byte_at(self.cursor);
        self.query.insert(idx, ch);
        self.cursor += 1;
        self.after_query_change(choices);
    }

    pub fn backspace(&mut self, choices: &[ModelChoice]) {
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
        self.after_query_change(choices);
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

    fn after_query_change(&mut self, choices: &[ModelChoice]) {
        self.list_offset = 0;
        match self.mode {
            PaletteMode::Commands => {
                let rows = self.command_rows();
                self.selected = commands::first_selectable(&rows);
            }
            PaletteMode::Models => {
                self.selected = 0;
                self.clamp_selection(choices);
            }
        }
    }

    /// First visible index for the scrollable list.
    pub fn visible_offset(&self, choices: &[ModelChoice], visible: usize) -> usize {
        let (len, vis) = self.scroll_metrics(choices, visible);
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
        rows.get(self.selected).copied()
    }

    pub fn selected_cmd_id(&self) -> Option<CmdId> {
        self.selected_command().map(|c| c.id)
    }
}
