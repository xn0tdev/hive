//! Short-lived status toasts — gray text in the corner, no box.

use comb::{Frame, Line, Rect, Span, Style};

use crate::app::App;

/// Draw active Info / Error toasts in the bottom-right of `area`.
///
/// Error sits above Info when both are up. No fill, no border — same type as
/// the rest of the UI, just a kind label and a line of dim text.
pub fn draw(f: &mut Frame, area: Rect, app: &App) -> bool {
    if area.width < 8 || area.height == 0 {
        return false;
    }
    let theme = &app.theme;
    let max = area.width as usize;
    let mut rows: Vec<(&str, &str)> = Vec::new();
    if let Some(msg) = app.error_toast() {
        rows.push(("Error", msg));
    }
    if let Some(msg) = app.flash_text() {
        rows.push(("Info", msg));
    }
    if rows.is_empty() {
        return false;
    }
    let shown = rows.len().min(area.height as usize);
    let start = rows.len() - shown;
    for (i, (kind, msg)) in rows[start..].iter().enumerate() {
        let y = area.y + area.height - shown as u16 + i as u16;
        paint_row(f, area.x, y, max, kind, msg, theme);
    }
    true
}

fn paint_row(
    f: &mut Frame,
    x: u16,
    y: u16,
    max: usize,
    kind: &str,
    msg: &str,
    theme: &crate::theme::Theme,
) {
    let kind_w = 5; // "Error" / "Info "
    let room = max.saturating_sub(kind_w + 2);
    let body = if msg.chars().count() > room && room > 1 {
        let mut s: String = msg.chars().take(room - 1).collect();
        s.push('…');
        s
    } else {
        msg.to_string()
    };
    let line = Line::from(vec![
        Span::styled(format!("{kind:<5}"), Style::default().fg(theme.faint)),
        Span::styled("  ", Style::default()),
        Span::styled(body, Style::default().fg(theme.dim)),
    ]);
    let w = (line.width() as u16).min(max as u16);
    let x = x + (max as u16).saturating_sub(w);
    f.buffer().set_line(x, y, &line, w);
}
