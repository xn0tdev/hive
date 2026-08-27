//! In-TUI settings: one open list, grouped by Chat / Sidebar / Tools.
//! No tabs, no nested pages — headers are labels, not buttons.

use super::App;

/// Keep in sync with `render::sidebar::{MIN_WIDTH, MAX_WIDTH}`.
const SIDEBAR_MIN_WIDTH: u16 = 24;
const SIDEBAR_MAX_WIDTH: u16 = 56;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsItem {
    Thoughts,
    WorkSummary,
    SidebarMode,
    CollapseSections,
    SidebarWidth,
    ShowToolCards,
    ToolRevert,
    LogoAnimation,
    Sound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRow {
    Header(&'static str),
    Spacer,
    Item(SettingsItem),
}

impl SettingsRow {
    pub fn is_selectable(self) -> bool {
        matches!(self, Self::Item(_))
    }

    pub fn item(self) -> Option<SettingsItem> {
        match self {
            Self::Item(item) => Some(item),
            _ => None,
        }
    }
}

pub const ROWS: &[SettingsRow] = &[
    SettingsRow::Header("Chat"),
    SettingsRow::Item(SettingsItem::Thoughts),
    SettingsRow::Item(SettingsItem::WorkSummary),
    SettingsRow::Spacer,
    SettingsRow::Header("Sidebar"),
    SettingsRow::Item(SettingsItem::SidebarMode),
    SettingsRow::Item(SettingsItem::CollapseSections),
    SettingsRow::Item(SettingsItem::SidebarWidth),
    SettingsRow::Spacer,
    SettingsRow::Header("Tools"),
    SettingsRow::Item(SettingsItem::ShowToolCards),
    SettingsRow::Item(SettingsItem::ToolRevert),
    SettingsRow::Spacer,
    SettingsRow::Header("Landing"),
    SettingsRow::Item(SettingsItem::LogoAnimation),
    SettingsRow::Item(SettingsItem::Sound),
];

#[derive(Debug, Clone)]
pub struct SettingsState {
    pub selected: usize,
}

impl SettingsState {
    pub fn root() -> Self {
        Self {
            selected: first_selectable(),
        }
    }

    pub fn item(&self) -> Option<SettingsItem> {
        ROWS.get(self.selected).and_then(|r| r.item())
    }

    pub fn move_up(&mut self) {
        self.selected = move_selection(self.selected, -1);
    }

    pub fn move_down(&mut self) {
        self.selected = move_selection(self.selected, 1);
    }
}

fn first_selectable() -> usize {
    ROWS.iter().position(|r| r.is_selectable()).unwrap_or(0)
}

fn move_selection(selected: usize, delta: isize) -> usize {
    let selectable: Vec<usize> = ROWS
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.is_selectable().then_some(i))
        .collect();
    if selectable.is_empty() {
        return 0;
    }
    let pos = selectable.iter().position(|&i| i == selected).unwrap_or(0);
    let n = selectable.len() as isize;
    let next = (pos as isize + delta).rem_euclid(n) as usize;
    selectable[next]
}

/// Activate the selected settings row. Returns true when a value changed
/// and should be persisted.
pub fn activate(app: &mut App) -> bool {
    let Some(item) = app.settings.as_ref().and_then(|st| st.item()) else {
        return false;
    };
    match item {
        SettingsItem::Thoughts => {
            app.ui.thoughts_always_open = !app.ui.thoughts_always_open;
            true
        }
        SettingsItem::WorkSummary => {
            app.ui.show_work_summary = !app.ui.show_work_summary;
            true
        }
        SettingsItem::SidebarMode => {
            app.ui.sidebar_mode = app.ui.sidebar_mode.cycle();
            true
        }
        SettingsItem::CollapseSections => {
            app.ui.sidebar_collapse_sections = !app.ui.sidebar_collapse_sections;
            true
        }
        SettingsItem::SidebarWidth => nudge_width(app, 2),
        SettingsItem::ShowToolCards => {
            app.ui.show_tool_cards = !app.ui.show_tool_cards;
            true
        }
        SettingsItem::ToolRevert => {
            app.ui.tool_revert = !app.ui.tool_revert;
            true
        }
        SettingsItem::LogoAnimation => {
            app.ui.logo_animation = !app.ui.logo_animation;
            true
        }
        SettingsItem::Sound => {
            app.ui.sound = !app.ui.sound;
            true
        }
    }
}

/// Adjust sidebar width from Settings (Left/Right). Returns true if changed.
pub fn nudge_width(app: &mut App, delta: i16) -> bool {
    let cur = i16::try_from(app.ui.sidebar_width).unwrap_or(34);
    let next = (cur + delta).clamp(SIDEBAR_MIN_WIDTH as i16, SIDEBAR_MAX_WIDTH as i16) as u16;
    if next == app.ui.sidebar_width {
        return false;
    }
    app.ui.sidebar_width = next;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_skip_headers_and_spacers() {
        let mut st = SettingsState::root();
        assert_eq!(st.item(), Some(SettingsItem::Thoughts));

        st.move_down();
        assert_eq!(st.item(), Some(SettingsItem::WorkSummary));
        st.move_down();
        assert_eq!(st.item(), Some(SettingsItem::SidebarMode));
        st.move_up();
        assert_eq!(st.item(), Some(SettingsItem::WorkSummary));
    }

    #[test]
    fn arrows_wrap_past_the_ends() {
        let mut st = SettingsState::root();
        st.move_up();
        assert_eq!(st.item(), Some(SettingsItem::Sound));
        st.move_down();
        assert_eq!(st.item(), Some(SettingsItem::Thoughts));
    }
}
