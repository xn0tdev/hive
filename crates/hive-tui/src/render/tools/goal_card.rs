//! Goal card in the transcript — objective + timer, no icon.

use comb::{Line, Modifier, Span, Style};

use crate::app::state::GoalCard;
use crate::app::App;

pub(crate) fn goal_card_lines(card: &GoalCard, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let timer = match card.deadline {
        None => "no time limit".to_string(),
        Some(d) => {
            let now = std::time::Instant::now();
            if d > now {
                let secs = d.duration_since(now).as_secs();
                if secs >= 3600 {
                    format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
                } else if secs >= 60 {
                    format!("{}m", secs / 60)
                } else {
                    format!("{}s", secs)
                }
            } else {
                "expired".to_string()
            }
        }
    };

    // The card announces that a goal started — it is not the place to re-read
    // the prompt. A long objective is squeezed to whatever is left after the
    // label and timer, so the card is always exactly one row.
    const LABEL: &str = "  Goal";
    let fixed = LABEL.chars().count() + 3 + 3 + timer.chars().count();
    let objective = fit(&card.objective, width.saturating_sub(fixed));

    let mut spans = vec![Span::styled(
        LABEL,
        Style::default().fg(theme.fg).add(Modifier::BOLD),
    )];
    if !objective.is_empty() {
        spans.push(Span::styled(" · ", Style::default().fg(theme.faint)));
        spans.push(Span::styled(objective, Style::default().fg(theme.dim)));
    }
    spans.push(Span::styled(" · ", Style::default().fg(theme.faint)));
    spans.push(Span::styled(timer, Style::default().fg(theme.faint)));
    vec![Line::from(spans)]
}

/// Clip to `max` columns with an ellipsis. Anything too narrow to say something
/// useful drops the objective entirely rather than showing `…`.
fn fit(s: &str, max: usize) -> String {
    // Multi-line prompts collapse to their first line — the rest never fit.
    let s = s.trim().lines().next().unwrap_or("").trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max < 8 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
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

    fn row(objective: &str, width: usize) -> String {
        let card = GoalCard {
            objective: objective.into(),
            deadline: None,
        };
        let lines = goal_card_lines(&card, &app(), width);
        assert_eq!(lines.len(), 1, "the card is always one row");
        lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_str())
            .collect::<String>()
    }

    /// A pasted prompt used to run straight off the side of the card.
    #[test]
    fn a_long_objective_is_clipped_to_the_card_width() {
        let long = "refactor the entire rendering layer, then rewrite every test \
                    and also update the docs while you are at it";
        for width in [40usize, 60, 80, 120] {
            let text = row(long, width);
            assert!(
                text.chars().count() <= width,
                "width {width}: {} cols — {text:?}",
                text.chars().count()
            );
            assert!(text.contains('…'), "width {width}: not clipped — {text:?}");
        }
    }

    #[test]
    fn a_short_objective_is_left_alone() {
        let text = row("ship it", 80);
        assert_eq!(text, "  Goal · ship it · no time limit");
    }

    /// A multi-line prompt must not turn into a multi-line card.
    #[test]
    fn a_multiline_objective_keeps_the_card_one_row() {
        let text = row("first line\nsecond line\nthird line", 80);
        assert!(text.contains("first line"), "{text:?}");
        assert!(!text.contains("second"), "{text:?}");
        assert!(text.chars().count() <= 80);
    }

    /// Too narrow to say anything useful: keep the label and the timer.
    #[test]
    fn a_narrow_card_drops_the_objective_rather_than_the_timer() {
        let text = row("some fairly long objective here", 24);
        assert!(text.starts_with("  Goal"), "{text:?}");
        assert!(text.ends_with("no time limit"), "{text:?}");
        assert!(!text.contains('…'), "no lone ellipsis: {text:?}");
    }
}
