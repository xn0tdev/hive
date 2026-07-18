//! Subagent card in the main transcript — soft `strip` band with pad rows.

use comb::{Color, Line, Modifier, Span, Style};

use hive_core::event::SubagentStatus;

use crate::app::state::SubagentCard;
use crate::app::App;
use crate::render::tools::tool_card::format_tool_secs;

/// Flat subagent card in the main transcript: title + duration, status under.
/// Click navigates into the dedicated subagent chat view (not inline expand).
/// Soft `strip` band with blank pad rows top/bottom so text isn't flush.
pub(crate) fn subagent_card_lines(
    card: &SubagentCard,
    app: &App,
    width: usize,
    show_hint: bool,
) -> Vec<Line> {
    let theme = &app.theme;
    let bg = theme.strip;
    let (icon, icon_fg) = match card.status {
        SubagentStatus::Running => (app.spinner_char().to_string(), theme.accent),
        SubagentStatus::Done => ("✓".to_string(), theme.ok),
        SubagentStatus::Failed => ("✗".to_string(), theme.err),
    };
    let title = if card.label.trim().is_empty() {
        "Checking project"
    } else {
        card.label.as_str()
    };

    let mut title_spans = vec![
        Span::raw("  "),
        Span::styled(format!("{icon} "), Style::default().fg(icon_fg)),
        Span::styled(
            title.to_string(),
            Style::default().fg(theme.fg).add(Modifier::BOLD),
        ),
        Span::styled(
            format!(" · {}", format_tool_secs(card.secs())),
            Style::default().fg(theme.dim),
        ),
    ];
    if show_hint {
        title_spans.push(Span::styled(
            "  click to open",
            Style::default().fg(theme.faint),
        ));
    }

    let title_line = soft_bg_line(Line::from(title_spans), bg, width);
    let status_line = soft_bg_line(
        Line::from(vec![
            Span::raw("    "),
            Span::styled(
                card.status_text().to_string(),
                Style::default().fg(theme.dim),
            ),
        ]),
        bg,
        width,
    );
    vec![
        soft_bg_pad(bg, width),
        title_line,
        status_line,
        soft_bg_pad(bg, width),
    ]
}

/// Paint a soft strip under a card line, padding to `width`.
fn soft_bg_line(line: Line, bg: Color, width: usize) -> Line {
    let mut used = 0usize;
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    for s in line.spans {
        used += s.content.chars().count();
        spans.push(Span::styled(s.content, s.style.bg(bg)));
    }
    if width > used {
        spans.push(Span::styled(
            " ".repeat(width - used),
            Style::default().bg(bg),
        ));
    }
    Line::from(spans)
}

/// Blank full-width row in the same soft strip colour (vertical breathing room).
fn soft_bg_pad(bg: Color, width: usize) -> Line {
    soft_bg_line(Line::from(""), bg, width)
}
