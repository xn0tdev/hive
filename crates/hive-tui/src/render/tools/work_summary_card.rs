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
    hovered: bool,
) -> Vec<Line> {
    let theme = &app.theme;
    let line_color = Color::Rgb(0x40, 0x40, 0x40);
    let text = format!(" Worked for {} ", format_duration(card.secs));
    let text_w = text.chars().count();
    let fill = width.saturating_sub(text_w);
    let left = fill / 2;
    let right = fill.saturating_sub(left);
    let text_fg = if hovered { theme.fg } else { theme.dim };
    let mut text_style = Style::default().fg(text_fg);
    if hovered {
        text_style = text_style.add(comb::Modifier::BOLD);
    }

    vec![Line::from(vec![
        Span::styled("─".repeat(left), Style::default().fg(line_color)),
        Span::styled(text, text_style),
        Span::styled("─".repeat(right), Style::default().fg(line_color)),
    ])]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TuiInit;

    fn app() -> App {
        App::new(TuiInit {
            model: "m".into(),
            model_display: "m".into(),
            model_choices: Vec::new(),
            skills: Vec::new(),
            connections: Vec::new(),
            active_connection: String::new(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
            ui: Default::default(),
            context_window: 128_000,
            cost_input: 0.0,
            cost_output: 0.0,
        })
    }

    #[test]
    fn worked_label_is_dim_even_at_zero_seconds() {
        let app = app();
        let lines = work_summary_card_lines(
            &WorkSummaryCard {
                secs: 0,
                ..Default::default()
            },
            &app,
            40,
            false,
        );

        assert_eq!(lines[0].spans[1].content, " Worked for 0s ");
        assert_eq!(lines[0].spans[1].style.fg, Some(app.theme.dim));
    }
}
