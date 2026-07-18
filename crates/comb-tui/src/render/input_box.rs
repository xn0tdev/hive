//! The input strip: a borderless dark band with a → prompt, placeholder text,
//! and a right-aligned "ctrl+c to stop" hint while a turn is running.

use comb::{Frame, Line, Modifier, Rect, Span, Style};

use crate::app::App;

const PROMPT: &str = "→ ";
/// Display width of the prompt (chars, not bytes — the arrow is multi-byte).
const PROMPT_W: usize = 2;

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let bg = theme.strip;

    // Paint the whole strip. THIS TINTED BAND (a padding row above, the text
    // row, a padding row below) IS THE INPUT — its decorative rows are part of
    // the design. NEVER remove them to "close a gap": the only thing that ever
    // gets trimmed to remove empty space is the layout OUTSIDE this band.
    f.buffer().paint(area, Style::default().bg(bg));

    // One padding row above/below, one padding column left/right — always.
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }

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
        for (i, l) in app.input.value.split('\n').enumerate() {
            let head = if i == 0 {
                Span::styled(PROMPT, prompt_style)
            } else {
                Span::styled("  ", text_style)
            };
            lines.push(Line::from(vec![
                head,
                Span::styled(l.to_string(), text_style),
            ]));
        }
    }
    f.buffer().set_lines(inner, &lines, 0);

    // Right-aligned hint while running, if there's room.
    if app.running {
        let hint = "esc to stop";
        let free = inner.width as usize;
        let first_len = PROMPT_W
            + app
                .input
                .value
                .split('\n')
                .next()
                .unwrap_or("")
                .chars()
                .count();
        if free > first_len + hint.len() + 4 {
            // Leave a column of breathing room on the right so the hint isn't
            // glued to the strip edge.
            let hint_w = hint.chars().count() as u16;
            let hx = inner.x + inner.width.saturating_sub(1 + hint_w);
            let hint_line = Line::from(Span::styled(hint, Style::default().fg(theme.faint).bg(bg)));
            f.buffer().set_line(hx, inner.y, &hint_line, hint_w);
        }
    }

    // Place the caret after the prompt.
    let (line, col) = app.input.cursor_line_col();
    let x_off = PROMPT_W as u16;
    let max_x = inner.x + inner.width.saturating_sub(1);
    let max_y = inner.y + inner.height.saturating_sub(1);
    let x = (inner.x + x_off + col as u16).min(max_x);
    let y = (inner.y + line as u16).min(max_y);
    f.set_cursor(x, y);
}
