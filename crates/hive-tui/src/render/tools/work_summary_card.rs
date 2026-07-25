//! "Worked for Nm" — centered text with horizontal lines on both sides.
//! Shown at the end of each turn so the user sees how long the agent worked.

use comb::{Color, Line, Span, Style};

use crate::app::state::WorkSummaryCard;
use crate::app::App;

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, secs % 3600 / 60)
    }
}

pub(crate) fn work_summary_card_lines(
    card: &WorkSummaryCard,
    app: &App,
    width: usize,
) -> Vec<Line> {
    let theme = &app.theme;
    let line_color = Color::Rgb(0x40, 0x40, 0x40);
    let text = format!(" Worked for {} ", format_duration(card.secs));
    let text_w = text.chars().count();
    let fill = width.saturating_sub(text_w);
    let left = fill / 2;
    let right = fill.saturating_sub(left);

    vec![Line::from(vec![
        Span::styled("─".repeat(left), Style::default().fg(line_color)),
        Span::styled(text, Style::default().fg(theme.fg)),
        Span::styled("─".repeat(right), Style::default().fg(line_color)),
    ])]
}
