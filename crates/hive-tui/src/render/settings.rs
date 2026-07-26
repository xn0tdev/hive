//! Simple Settings overlay: Chat and Sidebar pages.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use crate::app::settings::{SettingsPage, SettingsState};
use crate::app::App;

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
    let g = geom(area, st);
    dim_outside(buf, area, g.win);
    buf.paint(g.win, Style::default().bg(panel));

    if g.content.width < 16 || g.content.height < 6 {
        return;
    }

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

    let mut y = g.content.y;
    // One full-width header line so the gap between title and hint keeps panel bg.
    let hint_w = hint.chars().count();
    let title_w = title.chars().count();
    let gap = g
        .content
        .width
        .saturating_sub(title_w as u16)
        .saturating_sub(hint_w as u16);
    crate::render::strip_paint::set_line_on_strip(
        buf,
        g.content.x,
        y,
        &Line::from(vec![
            Span::styled(
                title.to_string(),
                Style::default().fg(theme.fg).add(Modifier::BOLD),
            ),
            Span::styled(" ".repeat(gap as usize), Style::default()),
            Span::styled(hint.to_string(), Style::default().fg(theme.faint)),
        ]),
        g.content.width,
        panel,
    );
    y += 2;

    let rows = rows_for(st, app);
    for (i, (label, value)) in rows.iter().enumerate() {
        if y >= g.content.bottom() {
            break;
        }
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
        y += 1;
    }
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
        SettingsPage::Tools => vec![("Revert file".into(), on_off(app.ui.tool_revert))],
    }
}

fn on_off(v: bool) -> String {
    if v {
        "on".into()
    } else {
        "off".into()
    }
}

struct Geom {
    win: Rect,
    content: Rect,
}

fn geom(area: Rect, st: &SettingsState) -> Geom {
    let rows = match st.page {
        SettingsPage::Root => 3,
        SettingsPage::Chat => 2,
        SettingsPage::Sidebar => 3,
        SettingsPage::Tools => 1,
    };
    let w = (area.width * 2 / 3).clamp(MIN_W, MAX_W).min(area.width);
    let h = (PAD_Y * 2 + 1 + 1 + rows as u16 + 1)
        .min(area.height.saturating_sub(2))
        .max(8);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let win = Rect::new(x, y, w, h);
    let content = Rect {
        x: win.x + PAD_X,
        y: win.y + PAD_Y,
        width: win.width.saturating_sub(PAD_X * 2),
        height: win.height.saturating_sub(PAD_Y * 2),
    };
    Geom { win, content }
}

fn dim_outside(buf: &mut Buffer, area: Rect, exclude: Rect) {
    let area = area.intersection(buf.area());
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if exclude.contains(x, y) {
                continue;
            }
            if let Some(cell) = buf.cell_mut(x, y) {
                if let Some(fg) = cell.style.fg {
                    cell.style.fg = Some(darken(fg));
                }
                cell.style.bg = Some(match cell.style.bg {
                    Some(bg) => darken(bg),
                    None => Color::Rgb(0x0a, 0x0a, 0x0a),
                });
                cell.style = cell.style.add(Modifier::DIM);
            }
        }
    }
}

fn darken(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Rgb(
            ((r as u16 * 160) / 255) as u8,
            ((g as u16 * 160) / 255) as u8,
            ((b as u16 * 160) / 255) as u8,
        ),
        Color::Reset => Color::Rgb(0x0a, 0x0a, 0x0a),
    }
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
