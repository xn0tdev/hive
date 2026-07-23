//! Plan.md card in the main transcript — same soft strip as subagent cards.

use comb::{Color, Line, Modifier, Span, Style};

use crate::app::state::{PlanCard, PlanStatus};
use crate::app::App;
use crate::render::tools::strip::{soft_bg_line, soft_bg_pad};
use crate::theme::Theme;

pub(crate) fn plan_card_lines(
    card: &PlanCard,
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

    let mut title_spans = vec![
        Span::raw("  "),
        Span::styled(
            "Plan.md".to_string(),
            Style::default().fg(theme.fg).add(Modifier::BOLD),
        ),
    ];
    if matches!(card.status, PlanStatus::Writing) {
        title_spans.push(Span::styled(
            format!("  {}", app.spinner_char()),
            Style::default().fg(theme.dim),
        ));
    }
    if show_hint {
        title_spans.push(Span::styled(
            "  click to open plan",
            Style::default().fg(if hovered { theme.dim } else { theme.faint }),
        ));
    }

    // Revised plans carry an `UPDATED` chip in the true top-right corner of the
    // strip (the top pad row), above the title.
    let top_row = if card.revised {
        badge_row("UPDATED", bg, theme, width)
    } else {
        soft_bg_pad(bg, width)
    };

    // Same left indent as the title ("  ") — not deeper — and trim so a
    // model-provided leading space doesn't show as a mysterious gap.
    let detail = if card.summary.trim().is_empty() {
        match card.status {
            PlanStatus::Writing => "writing plan…".to_string(),
            PlanStatus::Ready => "ready".to_string(),
        }
    } else {
        card.summary.trim().to_string()
    };

    vec![
        top_row,
        soft_bg_line(Line::from(title_spans), bg, width),
        soft_bg_line(
            Line::from(vec![
                Span::raw("  "),
                Span::styled(detail, Style::default().fg(theme.dim)),
            ]),
            bg,
            width,
        ),
        soft_bg_pad(bg, width),
    ]
}

/// Top pad row of the strip with a right-aligned chip badge (e.g. `UPDATED`).
///
/// Unlike [`soft_bg_line`], the badge keeps its own chip background instead of
/// being washed with the strip colour, so it reads as a distinct corner marker.
fn badge_row(label: &str, bg: Color, theme: &Theme, width: usize) -> Line {
    let badge_text = format!(" {label} ");
    let badge_style = Style::default()
        .fg(theme.sel_fg)
        .bg(theme.plan)
        .add(Modifier::BOLD);
    let badge_w = badge_text.chars().count();

    // Right-align the badge; keep it visible even when the row is too narrow.
    let gap = width.saturating_sub(badge_w);
    Line::from(vec![
        Span::styled(" ".repeat(gap), Style::default().bg(bg)),
        Span::styled(badge_text, badge_style),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_is_right_aligned_with_its_own_chip_bg() {
        let theme = Theme::gray();
        let line = badge_row("UPDATED", theme.strip, &theme, 40);

        let text: String = line.spans.iter().map(|s| s.content.as_str()).collect();
        assert!(text.trim_end().ends_with("UPDATED"), "badge sits in the corner: {text:?}");
        assert_eq!(text.chars().count(), 40, "row spans the full strip width");

        // The badge keeps the plan chip colour rather than the strip wash.
        let badge = line
            .spans
            .iter()
            .find(|s| s.content.contains("UPDATED"))
            .expect("badge span");
        assert_eq!(badge.style.bg, Some(theme.plan));
        assert_ne!(theme.plan, theme.strip);
    }

    #[test]
    fn badge_still_shows_when_row_is_too_narrow_to_right_align() {
        let theme = Theme::gray();
        let line = badge_row("UPDATED", theme.strip, &theme, 4);
        let text: String = line.spans.iter().map(|s| s.content.as_str()).collect();
        assert!(text.contains("UPDATED"), "badge never dropped: {text:?}");
    }
}
