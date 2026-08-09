//! `/goal` overlay: centered panel with editable objective + time limit.

use comb::{Buffer, Line, ModalLayout, Modifier, Rect, Span, Style};

use crate::app::goal::GoalField;
use crate::app::App;
use crate::render::panel::Panel;

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
    let g = overlay().render(buf, area, theme);

    if g.content.width < 20 || g.content.height < 6 {
        return;
    }

    let mut y = g.content.y + 2;

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

fn overlay() -> Panel<'static> {
    Panel::new("Goal", "tab field · enter start · esc", 7)
        .width_bounds(MIN_W, MAX_W)
        .padding(PAD_X, PAD_Y)
}

fn geom(area: Rect) -> ModalLayout {
    overlay().layout(area)
}
