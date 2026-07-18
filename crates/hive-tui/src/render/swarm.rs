//! Compact, borderless swarm activity block shown above the input while
//! subagents exist: a summary line plus the most recent entries.

use comb::{Buffer, Line, Modifier, Rect, Span, Style};

use hive_core::event::SubagentStatus;

use crate::app::App;

const MAX_ENTRIES: usize = 4;

pub fn height(app: &App) -> u16 {
    if app.swarm.is_empty() {
        return 0;
    }
    // summary + up to MAX_ENTRIES rows + trailing blank
    (1 + app.swarm.len().min(MAX_ENTRIES) + 1) as u16
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &App) {
    let theme = &app.theme;
    let running = count(app, SubagentStatus::Running);
    let done = count(app, SubagentStatus::Done);
    let failed = count(app, SubagentStatus::Failed);

    let mut summary = vec![
        Span::styled("⬡ ", Style::default().fg(theme.accent)),
        Span::styled(
            "swarm",
            Style::default()
                .fg(theme.accent)
                .add(Modifier::BOLD),
        ),
        Span::styled(
            format!(" · {running} running · {done} done"),
            Style::default().fg(theme.dim),
        ),
    ];
    if failed > 0 {
        summary.push(Span::styled(
            format!(" · {failed} failed"),
            Style::default().fg(theme.err),
        ));
    }
    let mut lines: Vec<Line> = vec![Line::from(summary)];

    // Most recent entries, running ones first in natural order.
    let start = app.swarm.len().saturating_sub(MAX_ENTRIES);
    for entry in &app.swarm[start..] {
        let (icon, color) = match entry.status {
            SubagentStatus::Running => (app.spinner_char().to_string(), theme.accent),
            SubagentStatus::Done => ("✔".to_string(), theme.ok),
            SubagentStatus::Failed => ("✘".to_string(), theme.err),
        };
        let mut spans = vec![
            Span::styled(format!("  {icon} "), Style::default().fg(color)),
            Span::styled(entry.label.clone(), Style::default().fg(theme.dim)),
        ];
        if !entry.detail.is_empty() {
            spans.push(Span::styled(
                format!("  {}", entry.detail),
                Style::default().fg(theme.faint),
            ));
        }
        lines.push(Line::from(spans));
    }

    buf.set_lines(area, &lines, 0);
}

fn count(app: &App, status: SubagentStatus) -> usize {
    app.swarm.iter().filter(|e| e.status == status).count()
}
