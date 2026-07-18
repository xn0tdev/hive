//! Short-lived status toasts — soft strip, centered near the bottom.

use comb::{Frame, Line, Rect, Span, Style};

use crate::app::App;

/// Draw the current flash as a centered soft pill on the last row of `area`.
pub fn draw(f: &mut Frame, area: Rect, app: &App) -> bool {
    let Some(msg) = app.flash_text() else {
        return false;
    };
    if area.width < 4 || area.height == 0 {
        return false;
    }

    let theme = &app.theme;
    let mut label = msg.to_string();
    let max = area.width as usize;
    if label.chars().count() > max {
        label = label.chars().take(max).collect();
    }
    let w = label.chars().count() as u16;
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(1);

    // No background — just faint centered text so it doesn't look like a bar.
    let style = Style::default().fg(theme.dim);
    f.buffer()
        .set_line(x, y, &Line::from(Span::styled(label, style)), w);
    true
}
