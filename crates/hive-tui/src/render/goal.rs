//! `/goal` overlay: centered panel with editable objective + time limit.

use comb::{Buffer, Color, Line, Modifier, Rect, Span, Style};

use crate::app::goal::GoalField;
use crate::app::App;

const MIN_W: u16 = 40;
const MAX_W: u16 = 56;
const PAD_X: u16 = 2;
const PAD_Y: u16 = 1;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App) {
    let Some(st) = app.goal_overlay.as_ref() else {
        return;
    };
    let theme = &app.theme;
    let panel = theme.strip;
    let g = geom(area);
    dim_outside(buf, area, g.win);
    buf.paint(g.win, Style::default().bg(panel));

    if g.content.width < 20 || g.content.height < 6 {
        return;
    }

    let title = "Goal";
    let hint = "enter start · esc close";
    let mut y = g.content.y;

    // Header line.
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

    // Objective field.
    let obj_focused = st.focus == GoalField::Objective;
    let obj_mark = if obj_focused { "› " } else { "  " };
    let obj_label = "Objective: ";
    let obj_val = &st.objective;
    let obj_cursor = if obj_focused { "│" } else { "" };
    let obj_line = format!("{obj_mark}{obj_label}{obj_val}{obj_cursor}");
    let obj_fg = if obj_focused { theme.fg } else { theme.dim };
    let obj_style = if obj_focused {
        Style::default().fg(obj_fg).add(Modifier::BOLD)
    } else {
        Style::default().fg(obj_fg)
    };
    crate::render::strip_paint::set_line_on_strip(
        buf,
        g.content.x,
        y,
        &Line::from(vec![Span::styled(obj_line, obj_style)]),
        g.content.width,
        panel,
    );
    y += 1;

    // Time limit field.
    let tl_focused = st.focus == GoalField::TimeLimit;
    let tl_mark = if tl_focused { "› " } else { "  " };
    let tl_label = "Time limit: ";
    let tl_val = if st.time_limit.is_empty() && !tl_focused {
        "optional, e.g. 30m"
    } else {
        &st.time_limit
    };
    let tl_cursor = if tl_focused { "│" } else { "" };
    let tl_line = format!("{tl_mark}{tl_label}{tl_val}{tl_cursor}");
    let tl_fg = if tl_focused {
        theme.fg
    } else if st.time_limit.is_empty() {
        theme.faint
    } else {
        theme.dim
    };
    let tl_style = if tl_focused {
        Style::default().fg(tl_fg).add(Modifier::BOLD)
    } else {
        Style::default().fg(tl_fg)
    };
    crate::render::strip_paint::set_line_on_strip(
        buf,
        g.content.x,
        y,
        &Line::from(vec![Span::styled(tl_line, tl_style)]),
        g.content.width,
        panel,
    );
}

/// Which field sits under the pointer. Same two rows `draw` writes: objective
/// two lines below the title, time limit under it.
pub fn field_at(area: Rect, app: &App, col: u16, row: u16) -> Option<GoalField> {
    app.goal_overlay.as_ref()?;
    let g = geom(area);
    if col < g.content.x || col >= g.content.right() {
        return None;
    }
    match row.checked_sub(g.content.y + 2)? {
        0 => Some(GoalField::Objective),
        1 => Some(GoalField::TimeLimit),
        _ => None,
    }
}

struct Geom {
    win: Rect,
    content: Rect,
}

fn geom(area: Rect) -> Geom {
    let w = (area.width * 2 / 3).clamp(MIN_W, MAX_W).min(area.width);
    let h = (PAD_Y * 2 + 1 + 1 + 2 + 1)
        .min(area.height.saturating_sub(2))
        .max(7);
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
