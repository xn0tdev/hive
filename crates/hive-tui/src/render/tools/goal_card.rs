//! Goal card in the transcript — objective + timer, no icon.

use comb::{Line, Modifier, Span, Style};

use crate::app::state::GoalCard;
use crate::app::App;

pub(crate) fn goal_card_lines(card: &GoalCard, app: &App, _width: usize) -> Vec<Line> {
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

    vec![Line::from(vec![
        Span::styled("  Goal", Style::default().fg(theme.fg).add(Modifier::BOLD)),
        Span::styled(" · ", Style::default().fg(theme.faint)),
        Span::styled(card.objective.clone(), Style::default().fg(theme.dim)),
        Span::styled(" · ", Style::default().fg(theme.faint)),
        Span::styled(timer, Style::default().fg(theme.faint)),
    ])]
}
