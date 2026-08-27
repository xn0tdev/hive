//! Settings overlay: one open list, grouped into Chat / Sidebar / Tools.
//! Headers are labels only — no tabs, no nested pages.

#[cfg(test)]
use comb::ModalLayout;
use comb::{Buffer, Line, Rect, Span, Style};

use crate::app::settings::{SettingsItem, SettingsRow, ROWS};
use crate::app::App;
use crate::render::panel::Panel;

const MIN_W: u16 = 36;
const MAX_W: u16 = 48;
const PAD_X: u16 = 2;
const PAD_Y: u16 = 1;
/// Title + gap before the list.
const CHROME_ROWS: u16 = 2;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App) {
    let Some(st) = app.settings.as_ref() else {
        return;
    };
    let theme = &app.theme;
    let panel = theme.strip;
    let g = overlay().render(buf, area, theme);

    if g.content.width < 16 || g.content.height < 4 {
        return;
    }

    let first = g.content.y + CHROME_ROWS;
    for (i, row) in ROWS.iter().enumerate() {
        let y = first + i as u16;
        if y >= g.content.bottom() {
            break;
        }
        match row {
            SettingsRow::Spacer => {
                buf.paint(
                    Rect::new(g.content.x, y, g.content.width, 1),
                    Style::default().bg(panel),
                );
            }
            SettingsRow::Header(label) => {
                crate::render::strip_paint::section_header(
                    buf,
                    g.content.x,
                    y,
                    g.content.width,
                    label,
                    theme,
                    panel,
                );
            }
            SettingsRow::Item(item) => {
                let sel = i == st.selected;
                let bg = if sel { theme.sel_bg } else { panel };
                let name_fg = if sel { theme.sel_fg } else { theme.fg };
                let value_fg = if sel { theme.sel_fg } else { theme.dim };
                let (label, value) = item_cells(*item, app);
                let avail = g.content.width as usize;
                let gap = avail
                    .saturating_sub(label.chars().count())
                    .saturating_sub(value.chars().count());
                crate::render::strip_paint::set_line_on_strip(
                    buf,
                    g.content.x,
                    y,
                    &Line::from(vec![
                        Span::styled(label, Style::default().fg(name_fg).bg(bg)),
                        Span::styled(" ".repeat(gap), Style::default().bg(bg)),
                        Span::styled(value, Style::default().fg(value_fg).bg(bg)),
                    ]),
                    g.content.width,
                    bg,
                );
            }
        }
    }
}

/// The panel rect, for tests that need to aim a click at or away from it.
#[cfg(test)]
pub fn window_rect(area: Rect, app: &App) -> Option<Rect> {
    app.settings.as_ref()?;
    Some(geom(area).panel)
}

fn item_cells(item: SettingsItem, app: &App) -> (String, String) {
    match item {
        SettingsItem::Thoughts => (
            "Always show thoughts".into(),
            on_off(app.ui.thoughts_always_open),
        ),
        SettingsItem::WorkSummary => ("Work summary".into(), on_off(app.ui.show_work_summary)),
        SettingsItem::SidebarMode => ("Panel".into(), app.ui.sidebar_mode.label().into()),
        SettingsItem::CollapseSections => (
            "Collapse sections".into(),
            on_off(app.ui.sidebar_collapse_sections),
        ),
        SettingsItem::SidebarWidth => ("Width".into(), format!("{} cols", app.ui.sidebar_width)),
        SettingsItem::ShowToolCards => ("Completed tools".into(), on_off(app.ui.show_tool_cards)),
        SettingsItem::ToolRevert => ("Revert file".into(), on_off(app.ui.tool_revert)),
        SettingsItem::LogoAnimation => ("Logo animation".into(), on_off(app.ui.logo_animation)),
        SettingsItem::Sound => ("Sound".into(), on_off(app.ui.sound)),
    }
}

fn on_off(v: bool) -> String {
    if v {
        "on".into()
    } else {
        "off".into()
    }
}

fn overlay() -> Panel<'static> {
    Panel::new(
        "Settings",
        "esc",
        PAD_Y * 2 + CHROME_ROWS + ROWS.len() as u16,
    )
    .width_ratio(1, 2)
    .width_bounds(MIN_W, MAX_W)
    .padding(PAD_X, PAD_Y)
}

#[cfg(test)]
fn geom(area: Rect) -> ModalLayout {
    overlay().layout(area)
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
    fn open_list_groups_chat_sidebar_and_tools() {
        let mut a = app();
        a.open_settings();
        let shown = text(&mut a);
        assert!(shown.contains("Chat"), "{shown}");
        assert!(shown.contains("Sidebar"), "{shown}");
        assert!(shown.contains("Tools"), "{shown}");
        assert!(shown.contains("Landing"), "{shown}");
        assert!(shown.contains("Always show thoughts"), "{shown}");
        assert!(shown.contains("Collapse sections"), "{shown}");
        assert!(shown.contains("Revert file"), "{shown}");
        assert!(shown.contains("Logo animation"), "{shown}");
        assert!(shown.contains("Sound"), "{shown}");
        assert!(!shown.contains("enter toggle"), "no button chrome: {shown}");
        assert!(
            !shown.contains('›'),
            "selection is the highlight bar: {shown}"
        );

        let chat = shown.find("Chat").expect("Chat");
        let sidebar = shown.find("Sidebar").expect("Sidebar");
        let tools = shown.find("Tools").expect("Tools");
        let landing = shown.find("Landing").expect("Landing");
        assert!(
            chat < sidebar && sidebar < tools && tools < landing,
            "groups top to bottom: {shown}"
        );
    }

    #[test]
    fn toggling_revert_asks_to_persist() {
        let mut a = app();
        a.open_settings();
        assert!(text(&mut a).contains("on"), "starts enabled");

        if let Some(st) = a.settings.as_mut() {
            st.selected = ROWS
                .iter()
                .position(|r| r.item() == Some(SettingsItem::ToolRevert))
                .expect("Revert file row");
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
