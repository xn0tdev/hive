//! Full-screen viewer for one large pasted text block.

use comb::{Buffer, Line, Modifier, Rect, Span, Style};

use crate::app::App;
use crate::render::panel::Panel;
use crate::render::strip_paint::set_line_on_strip;
use crate::render::wrap;

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App) {
    let Some(view) = app.pasted_view.as_ref() else {
        return;
    };
    let idx = view.idx;
    let scroll = view.scroll;
    let Some(block) = app.pasted_blocks.get(idx) else {
        return;
    };
    let label = block.label();
    let theme = &app.theme;
    let bg = theme.strip;

    let layout = Panel::new(&label, "esc close · ↑↓ scroll", 18)
        .width_ratio(3, 4)
        .width_bounds(40, 80)
        .vertical_margin(1)
        .padding(2, 1)
        .render(buf, area, theme);

    let content_area = layout.content;
    if content_area.width < 4 || content_area.height < 2 {
        return;
    }

    // Row 0 is the panel header; the body is everything under it.
    let body_area = Rect::new(
        content_area.x,
        content_area.y + 1,
        content_area.width,
        content_area.height.saturating_sub(1),
    );

    let style = Style::default().fg(theme.fg).bg(bg);
    let lines: Vec<Line> = block
        .content
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), style)))
        .collect();
    let wrapped = wrap::wrap_lines(lines, body_area.width as usize);
    let vis = body_area.height as usize;
    let max_off = wrapped.len().saturating_sub(vis);
    let off = scroll.min(max_off);

    // Write the clamped offset back so ↑/End keep working off the real range.
    if let Some(v) = app.pasted_view.as_mut() {
        v.scroll = off;
    }

    for row in 0..vis {
        let y = body_area.y + row as u16;
        match wrapped.get(off + row) {
            Some(line) => {
                set_line_on_strip(buf, body_area.x, y, line, body_area.width, bg);
            }
            None => {
                buf.paint(
                    Rect::new(body_area.x, y, body_area.width, 1),
                    Style::default().bg(bg),
                );
            }
        }
    }

    // Scroll position, bottom-right of the body.
    if wrapped.len() > vis && body_area.width > 6 {
        let pct = if max_off == 0 {
            0
        } else {
            (off * 100).div_ceil(max_off)
        };
        let hint = format!("{pct}%");
        let hw = hint.chars().count() as u16;
        let hx = body_area.x + body_area.width - hw - 1;
        let hy = body_area.y + body_area.height - 1;
        let line = Line::from(Span::styled(
            hint,
            Style::default()
                .fg(theme.faint)
                .bg(bg)
                .add(Modifier::ITALIC),
        ));
        buf.set_line(hx, hy, &line, hw);
    }
}
