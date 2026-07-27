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
    /// Every provider in one list — configured ones first, then the rest.
    Connect,
    /// Paste API key for the chosen preset.
    ConnectKey {
        preset_idx: usize,
    },
    /// Replace the API key of an already configured provider.
    EditConnectionKey,
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

/// One row in the `/connect` list. Configured providers and ones you could
/// configure live in the same list — picking either does the obvious thing.
#[derive(Debug, Clone, Copy)]
pub enum ConnectRow<'a> {
    /// A saved profile: selecting it switches to that provider.
    Profile(&'a ConnectionInfo),
    /// A known provider with no key yet: selecting it asks for one.
    Preset(usize),
}

/// Host part of a base URL, matching how the shell builds `ConnectionInfo`.
fn host_of(base_url: &str) -> &str {
    base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(base_url)
}

/// Whether a saved profile is this preset. Labels match for anything added
/// through Hive; the host catches profiles the user has since renamed.
fn is_preset_configured(preset_idx: usize, connections: &[ConnectionInfo]) -> bool {
    let Some(preset) = PRESETS.get(preset_idx) else {
        return false;
    };
    let host = host_of(preset.base_url);
    connections.iter().any(|c| {
        c.label.eq_ignore_ascii_case(preset.label) || (!host.is_empty() && c.detail == host)
    })
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
    /// Provider whose key is being replaced, for `EditConnectionKey`.
    pub edit_connection: Option<String>,
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
            edit_connection: None,
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
            edit_connection: None,
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
            edit_connection: None,
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
            edit_connection: None,
        }
    }

    pub fn edit_connection_key(connection_id: impl Into<String>) -> Self {
        PaletteState {
            mode: PaletteMode::EditConnectionKey,
            query: String::new(),
            cursor: 0,
            selected: 0,
            list_offset: 0,
            search_focused: true,
            edit_connection: Some(connection_id.into()),
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
            edit_connection: None,
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
        let anchor = self.scroll_anchor(choices, sel, vis);
        if anchor < off {
            off = anchor;
        } else if sel >= off + vis {
            off = sel + 1 - vis;
        }
        self.list_offset = off.min(max_off);
    }

    /// The topmost row that scrolling up to `sel` must reveal.
    ///
    /// Headers and spacers can never be selected, so stopping at the selection
    /// leaves the group header above it permanently off-screen — the first
    /// provider's name was unreachable no matter how far up you scrolled. They
    /// belong to the row beneath them, so they come along.
    fn scroll_anchor(&self, choices: &[ModelChoice], sel: usize, vis: usize) -> usize {
        let mut anchor = sel;
        match self.mode {
            PaletteMode::Commands => {
                let rows = self.command_rows();
                while anchor > 0 && !rows[anchor - 1].is_selectable() {
                    anchor -= 1;
                }
            }
            PaletteMode::Models => {
                let rows = self.model_rows(choices);
                while anchor > 0 && !rows[anchor - 1].is_selectable() {
                    anchor -= 1;
                }
            }
            // Every row picks something — nothing to drag into view.
            _ => {}
        }
        // A tall header block must never push the selection itself out the
        // bottom of the viewport.
        anchor.max(sel.saturating_sub(vis.saturating_sub(1)))
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
            PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey => 0,
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

    /// Every provider Hive knows about: the ones you've set up first, then the
    /// rest, so adding one is just picking it off the same list.
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

        rows.extend(
            PRESETS
                .iter()
                .enumerate()
                // "Custom" needs a base URL too, which only the intro asks for.
                .filter(|(i, p)| !p.is_custom && !is_preset_configured(*i, connections))
                .filter(|(_, p)| {
                    q.is_empty()
                        || p.label.to_ascii_lowercase().contains(&q)
                        || p.base_url.to_ascii_lowercase().contains(&q)
                })
                .map(|(i, _)| ConnectRow::Preset(i)),
        );

        rows
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
            PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey => {
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
            PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey => {}
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
            PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey => {}
            PaletteMode::Sessions => {
                let n = self.session_rows(sessions).len();
                if n > 0 {
                    self.selected = (self.selected + 1) % n;
                }
            }
        }
    }

    /// Whether `row` picks something (headers and spacers do not).
    pub fn row_is_selectable(
        &self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
        row: usize,
    ) -> bool {
        match self.mode {
            PaletteMode::Commands => self
                .command_rows()
                .get(row)
                .is_some_and(|r| r.is_selectable()),
            PaletteMode::Models => self
                .model_rows(choices)
                .get(row)
                .is_some_and(|r| r.is_selectable()),
            PaletteMode::Connect => row < self.connect_rows(connections).len(),
            PaletteMode::Sessions => row < self.session_rows(sessions).len(),
            PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey => false,
        }
    }

    /// Point the palette at `row` (mouse hover / click). Returns whether the
    /// highlight moved; a header or an out-of-range row is left alone.
    pub fn select_row(
        &mut self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
        row: usize,
    ) -> bool {
        if self.selected == row || !self.row_is_selectable(choices, connections, sessions, row) {
            return false;
        }
        self.selected = row;
        true
    }

    /// Scroll the viewport by `delta` rows without moving the highlight — the
    /// wheel looks around, the keyboard picks.
    pub fn scroll_list(
        &mut self,
        choices: &[ModelChoice],
        connections: &[ConnectionInfo],
        sessions: &[SessionMeta],
        visible: usize,
        delta: isize,
    ) -> bool {
        let (len, vis) = self.scroll_metrics(choices, connections, sessions, visible);
        if len == 0 || vis == 0 {
            return false;
        }
        let max_off = len.saturating_sub(vis);
        let next = (self.list_offset as isize + delta).clamp(0, max_off as isize) as usize;
        if next == self.list_offset {
            return false;
        }
        self.list_offset = next;
        true
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
            PaletteMode::Connect => {
                self.selected = 0;
                self.clamp_selection(choices, connections, sessions);
            }
            PaletteMode::Sessions => {
                self.selected = 0;
                self.clamp_selection(choices, connections, sessions);
            }
            PaletteMode::ConnectKey { .. } | PaletteMode::EditConnectionKey => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(id: &str, label: &str, host: &str) -> ConnectionInfo {
        ConnectionInfo {
            id: id.into(),
            label: label.into(),
            detail: host.into(),
        }
    }

    fn preset_labels(rows: &[ConnectRow<'_>]) -> Vec<&'static str> {
        rows.iter()
            .filter_map(|r| match r {
                ConnectRow::Preset(i) => Some(PRESETS[*i].label),
                _ => None,
            })
            .collect()
    }

    fn offered_count() -> usize {
        PRESETS.iter().filter(|p| !p.is_custom).count()
    }

    #[test]
    fn nothing_configured_still_lists_every_provider() {
        let rows = PaletteState::connect().connect_rows(&[]);
        assert_eq!(rows.len(), offered_count());
        assert!(rows.iter().all(|r| matches!(r, ConnectRow::Preset(_))));
        assert!(preset_labels(&rows).contains(&"Fireworks"));
    }

    #[test]
    fn a_configured_provider_is_not_offered_again() {
        let conns = vec![conn("fireworks", "Fireworks", "api.fireworks.ai")];
        let rows = PaletteState::connect().connect_rows(&conns);

        assert!(
            matches!(rows.first(), Some(ConnectRow::Profile(c)) if c.label == "Fireworks"),
            "configured providers come first"
        );
        assert!(!preset_labels(&rows).contains(&"Fireworks"));
        assert_eq!(rows.len(), offered_count(), "one row swapped, none added");
    }

    #[test]
    fn a_renamed_profile_is_matched_by_host() {
        let conns = vec![conn("work", "Work box", "api.groq.com")];
        let rows = PaletteState::connect().connect_rows(&conns);
        assert!(
            !preset_labels(&rows).contains(&"Groq"),
            "same host means it's already set up: {:?}",
            preset_labels(&rows)
        );
    }

    #[test]
    fn an_unknown_provider_keeps_every_preset_on_offer() {
        let conns = vec![conn("mine", "My gateway", "llm.internal")];
        let rows = PaletteState::connect().connect_rows(&conns);
        assert_eq!(preset_labels(&rows).len(), offered_count());
        assert!(matches!(rows.first(), Some(ConnectRow::Profile(_))));
    }

    #[test]
    fn search_spans_configured_and_offered() {
        let conns = vec![conn("fireworks", "Fireworks", "api.fireworks.ai")];
        let mut pal = PaletteState::connect();
        pal.query = "gro".into();
        let rows = pal.connect_rows(&conns);
        assert_eq!(preset_labels(&rows), vec!["Groq"]);
        assert!(!rows.iter().any(|r| matches!(r, ConnectRow::Profile(_))));

        pal.query = "fire".into();
        let rows = pal.connect_rows(&conns);
        assert!(matches!(rows.first(), Some(ConnectRow::Profile(c)) if c.label == "Fireworks"));
    }

    #[test]
    fn the_list_is_providers_and_nothing_else() {
        let conns = vec![
            conn("fireworks", "Fireworks", "api.fireworks.ai"),
            conn("groq", "Groq", "api.groq.com"),
        ];
        let rows = PaletteState::connect().connect_rows(&conns);
        // No command rows mixed in: every row is a provider you can act on.
        assert!(rows
            .iter()
            .all(|r| matches!(r, ConnectRow::Profile(_) | ConnectRow::Preset(_))));
        assert_eq!(rows.len(), offered_count());
    }

    #[test]
    fn model_rows_group_every_provider() {
        let model = |key: &str, group: &str, conn: &str| ModelChoice {
            key: key.into(),
            display: key.into(),
            detail: String::new(),
            group: group.into(),
            connection_id: conn.into(),
            vision: false,
            context: 0,
            cost_input: 0.0,
            cost_output: 0.0,
        };
        let choices = vec![
            model("a/one", "Fireworks", "fireworks"),
            model("a/two", "Fireworks", "fireworks"),
            model("b/one", "Groq", "groq"),
        ];

        let rows = PaletteState::models().model_rows(&choices);
        let headers: Vec<&str> = rows
            .iter()
            .filter_map(|r| match r {
                ModelRow::Header(h) => Some(*h),
                _ => None,
            })
            .collect();
        assert_eq!(headers, vec!["Fireworks", "Groq"]);
        assert_eq!(rows.iter().filter(|r| r.is_selectable()).count(), 3);
    }

    /// Scrolling back up used to stop at the first *model*, leaving the first
    /// provider's header (`Fireworks`) permanently above the viewport.
    #[test]
    fn scrolling_up_reaches_the_first_group_header() {
        let model = |key: &str, group: &str| ModelChoice {
            key: key.into(),
            display: key.into(),
            detail: String::new(),
            group: group.into(),
            connection_id: group.to_ascii_lowercase(),
            vision: false,
            context: 0,
            cost_input: 0.0,
            cost_output: 0.0,
        };
        let mut choices = Vec::new();
        for i in 0..6 {
            choices.push(model(&format!("fw/{i}"), "Fireworks"));
        }
        for i in 0..6 {
            choices.push(model(&format!("gq/{i}"), "Groq"));
        }

        let mut pal = PaletteState::models();
        let vis = 5;
        let rows = pal.model_rows(&choices);
        assert!(matches!(rows[0], ModelRow::Header("Fireworks")));
        let first = rows.iter().position(|r| r.is_selectable()).unwrap();

        // Walk down past the viewport…
        for _ in 0..8 {
            pal.move_down_visible(&choices, &[], &[], vis);
        }
        assert!(
            pal.visible_offset(&choices, &[], &[], vis) > 0,
            "the list should have scrolled"
        );

        // …then back up to the very first model.
        for _ in 0..40 {
            if pal.selected == first {
                break;
            }
            pal.move_up_visible(&choices, &[], &[], vis);
        }
        assert_eq!(pal.selected, first);
        assert_eq!(
            pal.visible_offset(&choices, &[], &[], vis),
            0,
            "the first header never scrolled into view"
        );
    }

    /// The same trap in the command palette: the `Suggested` header sits above
    /// the first command, so it has to come along too.
    #[test]
    fn scrolling_up_reaches_the_first_command_header() {
        let mut pal = PaletteState::commands();
        let vis = 4;
        let first = commands::first_selectable(&pal.command_rows());

        for _ in 0..8 {
            pal.move_down_visible(&[], &[], &[], vis);
        }
        assert!(pal.visible_offset(&[], &[], &[], vis) > 0);

        for _ in 0..40 {
            if pal.selected == first {
                break;
            }
            pal.move_up_visible(&[], &[], &[], vis);
        }
        assert_eq!(pal.selected, first);
        assert_eq!(pal.visible_offset(&[], &[], &[], vis), 0);
    }
}
