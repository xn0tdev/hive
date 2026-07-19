//! In-TUI settings: Chat and Sidebar pages.

use super::App;

/// Keep in sync with `render::sidebar::{MIN_WIDTH, MAX_WIDTH}`.
const SIDEBAR_MIN_WIDTH: u16 = 24;
const SIDEBAR_MAX_WIDTH: u16 = 56;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Root,
    Chat,
    Sidebar,
}

#[derive(Debug, Clone)]
pub struct SettingsState {
    pub page: SettingsPage,
    pub selected: usize,
}

impl SettingsState {
    pub fn root() -> Self {
        Self {
            page: SettingsPage::Root,
            selected: 0,
        }
    }

    pub fn len(&self) -> usize {
        match self.page {
            SettingsPage::Root => 2,
            SettingsPage::Chat => 1,
            SettingsPage::Sidebar => 3,
        }
    }

    pub fn move_up(&mut self) {
        let n = self.len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected + n - 1) % n;
    }

    pub fn move_down(&mut self) {
        let n = self.len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected + 1) % n;
    }

    pub fn enter_chat(&mut self) {
        self.page = SettingsPage::Chat;
        self.selected = 0;
    }

    pub fn enter_sidebar(&mut self) {
        self.page = SettingsPage::Sidebar;
        self.selected = 0;
    }

    pub fn back(&mut self) -> bool {
        if self.page == SettingsPage::Root {
            return true; // close overlay
        }
        self.page = SettingsPage::Root;
        self.selected = 0;
        false
    }
}

/// Activate the selected settings row (drill-in or toggle).
/// Returns true when a value changed and should be persisted.
pub fn activate(app: &mut App) -> bool {
    let Some(st) = app.settings.as_mut() else {
        return false;
    };
    let page = st.page;
    let sel = st.selected;
    match page {
        SettingsPage::Root => {
            if sel == 0 {
                st.enter_chat();
            } else {
                st.enter_sidebar();
            }
            false
        }
        SettingsPage::Chat => {
            if sel == 0 {
                app.ui.thoughts_always_open = !app.ui.thoughts_always_open;
            }
            true
        }
        SettingsPage::Sidebar => match sel {
            0 => {
                app.ui.sidebar_mode = app.ui.sidebar_mode.cycle();
                true
            }
            1 => {
                app.ui.sidebar_collapse_sections = !app.ui.sidebar_collapse_sections;
                true
            }
            2 => nudge_width(app, 2),
            _ => false,
        },
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
