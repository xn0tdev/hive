//! Todo progress card in the transcript — compact checklist with strikethrough.

use comb::{Line, Modifier, Span, Style};

use crate::app::App;

pub(crate) fn todo_card_lines(items: &[hive_core::TodoItem], app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let done = items.iter().filter(|t| t.done).count();
    let total = items.len();

    let mut out = Vec::with_capacity(items.len() + 1);

    let header = format!("  Tasks {done}/{total}");
    out.push(Line::from(Span::styled(
        header,
        Style::default().fg(theme.fg).add(Modifier::BOLD),
    )));

    for item in items {
        let text = truncate(&item.text, width.saturating_sub(6));
        if item.done {
            out.push(Line::from(vec![
                Span::styled("  ✓ ", Style::default().fg(theme.ok)),
                Span::styled(text, Style::default().fg(theme.faint).add(Modifier::DIM)),
            ]));
        } else {
            out.push(Line::from(vec![
                Span::styled("    ", Style::default().fg(theme.faint)),
                Span::styled(text, Style::default().fg(theme.dim)),
            ]));
        }
    }

    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max < 4 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}
