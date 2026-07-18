//! Borderless tinted input strip: → prompt, placeholder, hardware cursor.
//! Long lines soft-wrap within the strip width so the band can grow.

use comb::{Frame, Line, Modifier, Rect, Span, Style};

use crate::app::input::PROMPT_COLS;
use crate::app::App;

const PROMPT: &str = "→ ";

/// Text columns after the prompt/indent for a strip of the given outer width.
pub fn text_cols(band_width: u16) -> usize {
    band_width
        .saturating_sub(2) // horizontal pad inside the strip
        .saturating_sub(PROMPT_COLS as u16) as usize
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let bg = theme.strip;

    // Padding rows above/below the text are part of the design — don't trim them.
    f.buffer().paint(area, Style::default().bg(bg));

    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let width = app.input.text_cols.max(1);
    let prompt_style = Style::default().fg(theme.accent).bg(bg);
    let text_style = Style::default().fg(theme.fg).bg(bg);

    let mut lines: Vec<Line> = Vec::new();
    if app.input.is_empty() {
        let placeholder = if app.running {
            "Add a follow-up"
        } else if !app.pending_images.is_empty() {
            "Describe what to do with the attached image(s)"
        } else {
            "Ask, build, unleash the swarm"
        };
        lines.push(Line::from(vec![
            Span::styled(PROMPT, prompt_style),
            Span::styled(
                placeholder,
                Style::default()
                    .fg(theme.faint)
                    .bg(bg)
                    .add(Modifier::ITALIC),
            ),
        ]));
    } else {
        for (first, text) in app.input.wrapped_rows(width) {
            let head = if first {
                Span::styled(PROMPT, prompt_style)
            } else {
                Span::styled("  ", text_style)
            };
            lines.push(Line::from(vec![head, Span::styled(text, text_style)]));
        }
    }

    let visible = inner.height as usize;
    let scroll = app.input.view_scroll(visible, width);
    f.buffer()
        .set_lines_on(inner, &lines, scroll, Style::default().bg(bg));

    if app.running && scroll == 0 {
        let hint = "esc to stop";
        let free = inner.width as usize;
        let first_len = PROMPT_COLS
            + app
                .input
                .wrapped_rows(width)
                .first()
                .map(|(_, t)| t.chars().count())
                .unwrap_or(0);
        if free > first_len + hint.len() + 4 {
            let hint_w = hint.chars().count() as u16;
            let hx = inner.x + inner.width.saturating_sub(1 + hint_w);
            let hint_line = Line::from(Span::styled(hint, Style::default().fg(theme.faint).bg(bg)));
            f.buffer().set_line(hx, inner.y, &hint_line, hint_w);
        }
    }

    let (vrow, vcol) = app.input.cursor_visual(width);
    let x_off = PROMPT_COLS as u16;
    let max_x = inner.x + inner.width.saturating_sub(1);
    let max_y = inner.y + inner.height.saturating_sub(1);
    let x = (inner.x + x_off + vcol as u16).min(max_x);
    let y = (inner.y + vrow.saturating_sub(scroll) as u16).min(max_y);
    f.set_cursor(x, y);
}
