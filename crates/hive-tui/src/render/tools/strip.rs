//! Soft strip background helpers shared by subagent / plan cards.

use comb::{Color, Line, Span, Style};

/// Paint a soft strip under a card line, padding to `width`.
pub(crate) fn soft_bg_line(line: Line, bg: Color, width: usize) -> Line {
    let mut used = 0usize;
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    for s in line.spans {
        used += s.width();
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
pub(crate) fn soft_bg_pad(bg: Color, width: usize) -> Line {
    soft_bg_line(Line::from(""), bg, width)
}
