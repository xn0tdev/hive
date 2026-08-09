//! Simple Settings overlay: Chat and Sidebar pages.

use comb::{Buffer, Line, ModalLayout, Modifier, Rect, Span, Style};

use crate::app::settings::{SettingsPage, SettingsState};
use crate::app::App;
use crate::render::panel::Panel;

const MIN_W: u16 = 36;
const MAX_W: u16 = 48;
const PAD_X: u16 = 2;
const PAD_Y: u16 = 1;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App) {
    let Some(st) = app.settings.as_ref() else {
        return;
    };
    let theme = &app.theme;
    let panel = theme.strip;
    let g = overlay(st).render(buf, area, theme);

    if g.content.width < 16 || g.content.height < 6 {
        return;
    }

    let rows = rows_for(st, app);
    for (y, (i, (label, value))) in
        (g.content.y + 2..g.content.bottom()).zip(rows.iter().enumerate())
    {
        let sel = i == st.selected;
        let bg = if sel { theme.sel_bg } else { panel };
        let fg = if sel { theme.sel_fg } else { theme.fg };
        let dim = if sel { theme.sel_fg } else { theme.dim };
        let mark = if sel { "› " } else { "  " };
        let left = format!("{mark}{label}");
        let right = value.clone();
        let gap = g
            .content
            .width
            .saturating_sub(left.chars().count() as u16)
            .saturating_sub(right.chars().count() as u16);
        let mut style_left = Style::default().fg(fg);
        if sel {
            style_left = style_left.add(Modifier::BOLD);
        }
        crate::render::strip_paint::set_line_on_strip(
            buf,
            g.content.x,
            y,
            &Line::from(vec![
                Span::styled(left, style_left),
                Span::styled(" ".repeat(gap as usize), Style::default()),
                Span::styled(right, Style::default().fg(dim)),
            ]),
            g.content.width,
            bg,
        );
    }
}

/// The panel rect, for telling a click on the overlay from one that dismisses it.
pub fn window_rect(area: Rect, app: &App) -> Option<Rect> {
    let st = app.settings.as_ref()?;
    Some(geom(area, st).panel)
}

/// Which settings row sits under the pointer.
///
/// Rows start two lines below the content top (title, then a blank), one per
/// line — the same walk `draw` does, so the two can't disagree.
pub fn row_at(area: Rect, app: &App, col: u16, row: u16) -> Option<usize> {
    let st = app.settings.as_ref()?;
    let g = geom(area, st);
    if col < g.content.x || col >= g.content.right() {
        return None;
    }
    let first = g.content.y + 2;
    if row < first || row >= g.content.bottom() {
        return None;
    }
    let idx = usize::from(row - first);
    (idx < st.len()).then_some(idx)
}

fn rows_for(st: &SettingsState, app: &App) -> Vec<(String, String)> {
    match st.page {
        // Root: label only — selection › is drawn on the left.
        SettingsPage::Root => vec![
            ("Chat".into(), String::new()),
            ("Sidebar".into(), String::new()),
            ("Tools".into(), String::new()),
        ],
        SettingsPage::Chat => vec![
            (
                "Always show thoughts".into(),
                on_off(app.ui.thoughts_always_open),
            ),
            ("Work summary".into(), on_off(app.ui.show_work_summary)),
        ],
        SettingsPage::Sidebar => vec![
            ("Panel".into(), app.ui.sidebar_mode.label().into()),
            (
                "Collapse sections".into(),
                on_off(app.ui.sidebar_collapse_sections),
            ),
            ("Width".into(), format!("{} cols", app.ui.sidebar_width)),
        ],
        SettingsPage::Tools => vec![
            ("Completed tools".into(), on_off(app.ui.show_tool_cards)),
            ("Revert file".into(), on_off(app.ui.tool_revert)),
        ],
    }
}

fn on_off(v: bool) -> String {
    if v {
        "on".into()
    } else {
        "off".into()
    }
}

fn title_hint(st: &SettingsState) -> (&'static str, &'static str) {
    let title = match st.page {
        SettingsPage::Root => "Settings",
        SettingsPage::Chat => "Chat",
        SettingsPage::Sidebar => "Sidebar",
        SettingsPage::Tools => "Tools",
    };
    let hint = match st.page {
        SettingsPage::Root => "enter open  ·  esc close",
        _ => "enter toggle  ·  esc back",
    };
    (title, hint)
}

fn overlay(st: &SettingsState) -> Panel<'static> {
    let (title, hint) = title_hint(st);
    Panel::new(title, hint, 8)
        .width_bounds(MIN_W, MAX_W)
        .padding(PAD_X, PAD_Y)
}

fn geom(area: Rect, st: &SettingsState) -> ModalLayout {
    overlay(st).layout(area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::settings::activate;
    use crate::TuiInit;
    use comb::{render, Size};

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

    fn text(app: &mut App) -> String {
        render(Size::new(80, 24), |f| crate::render::draw(f, app))
            .text()
            .to_string()
    }

    #[test]
    fn tools_page_toggles_revert_and_asks_to_persist() {
        let mut a = app();
        a.open_settings();
        assert!(text(&mut a).contains("Tools"), "root lists the page");

        // Root row 2 → Tools.
        if let Some(st) = a.settings.as_mut() {
            st.selected = 2;
        }
        assert!(!activate(&mut a), "drilling in changes nothing to save");
        assert_eq!(
            a.settings.as_ref().map(|s| s.page),
            Some(SettingsPage::Tools)
        );

        let shown = text(&mut a);
        assert!(shown.contains("Revert file"), "{shown}");
        assert!(shown.contains("on"), "starts enabled: {shown}");

        // Row 1 is "Revert file" (row 0 is "Completed tools").
        if let Some(st) = a.settings.as_mut() {
            st.selected = 1;
        }
        assert!(activate(&mut a), "a toggle must be persisted");
        assert!(!a.ui.tool_revert);
        assert!(text(&mut a).contains("off"));
    }

    #[test]
    fn revert_disappears_from_the_tool_menu_when_off() {
        use crate::app::state::{Block, FileSnapshot, ToolCard, ToolStatus};

        let mut a = app();
        a.blocks.clear();
        a.blocks.push(Block::Tool(ToolCard {
            id: "t1".into(),
            name: "write_file".into(),
            args: "src/main.rs".into(),
            output: "+1\tnew line".into(),
            status: ToolStatus::Ok,
            started: std::time::Instant::now(),
            elapsed_ms: Some(1),
            details_open: false,
            snapshot: Some(FileSnapshot {
                path: "src/main.rs".into(),
                content: "old".into(),
            }),
        }));

        a.open_tool_menu(0);
        let labels: Vec<String> = a
            .context_menu
            .as_ref()
            .map(|m| m.items.iter().map(|i| i.label.clone()).collect())
            .unwrap_or_default();
        assert!(
            labels.iter().any(|l| l.starts_with("Revert")),
            "on by default: {labels:?}"
        );

        a.close_context_menu();
        a.ui.tool_revert = false;
        a.open_tool_menu(0);
        let labels: Vec<String> = a
            .context_menu
            .as_ref()
            .map(|m| m.items.iter().map(|i| i.label.clone()).collect())
            .unwrap_or_default();
        assert!(
            !labels.iter().any(|l| l.starts_with("Revert")),
            "a stray click must not be able to roll the file back: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l == "Copy output"),
            "the rest of the menu stays: {labels:?}"
        );
    }
}
