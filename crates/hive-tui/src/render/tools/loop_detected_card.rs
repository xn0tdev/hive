//! "Loop detected" — centered text with horizontal lines on both sides.

use comb::{Line, Span, Style};

use crate::app::App;

pub(crate) fn loop_detected_card_lines(
    app: &App,
    width: usize,
) -> Vec<Line> {
    let theme = &app.theme;
    let line_color = comb::Color::Rgb(0x40, 0x40, 0x40);
    let text = " Loop detected ";
    let text_w = text.chars().count();
    let fill = width.saturating_sub(text_w);
    let left = fill / 2;
    let right = fill.saturating_sub(left);

    vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("─".repeat(left), Style::default().fg(line_color)),
            Span::styled(text, Style::default().fg(theme.fg)),
            Span::styled("─".repeat(right), Style::default().fg(line_color)),
        ]),
    ]
}
