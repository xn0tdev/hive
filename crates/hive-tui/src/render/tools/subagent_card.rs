//! Subagent card in the main transcript — soft `strip` band with pad rows.

use comb::{Color, Line, Modifier, Span, Style};

use hive_core::event::SubagentStatus;

use crate::app::state::SubagentCard;
use crate::app::App;
use crate::render::tools::strip::{soft_bg_line, soft_bg_pad};
use crate::render::tools::tool_card::format_tool_secs;

/// Flat subagent card in the main transcript: title + duration, status under.
/// Click navigates into the dedicated subagent chat view (not inline expand).
/// Soft `strip` band with blank pad rows top/bottom so text isn't flush.
pub(crate) fn subagent_card_lines(
    card: &SubagentCard,
    app: &App,
    width: usize,
    show_hint: bool,
    hovered: bool,
) -> Vec<Line> {
    let theme = &app.theme;
    let bg: Color = if hovered {
        theme.strip_hover
    } else {
        theme.strip
    };
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
            Style::default().fg(if hovered { theme.dim } else { theme.faint }),
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
