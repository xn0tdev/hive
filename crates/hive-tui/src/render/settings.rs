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
        ],
        SettingsPage::Chat => vec![(
            "Always show thoughts".into(),
            on_off(app.ui.thoughts_always_open),
        )],
        SettingsPage::Sidebar => vec![
            ("Panel".into(), app.ui.sidebar_mode.label().into()),
            (
                "Collapse sections".into(),
                on_off(app.ui.sidebar_collapse_sections),
            ),
            (
                "Width".into(),
                format!("{} cols", app.ui.sidebar_width),
            ),
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

struct Geom {
    win: Rect,
    content: Rect,
}

fn geom(area: Rect, st: &SettingsState) -> Geom {
    let rows = match st.page {
        SettingsPage::Root => 2,
        SettingsPage::Chat => 1,
        SettingsPage::Sidebar => 3,
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
