//! Short-lived status toasts — soft text on the last row of a given band.

use comb::{Frame, Line, Rect, Span, Style};

use crate::app::App;

/// Draw the current flash centered on the last row of `area`.
///
/// Callers choose the band: landing uses the screen bottom; active chat uses
/// the transcript band so the toast sits above the input and never fights BUILD.
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

    let style = Style::default().fg(theme.dim);
    f.buffer()
        .set_line(x, y, &Line::from(Span::styled(label, style)), w);
    true
}
