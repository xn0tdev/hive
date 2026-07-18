//! Plan.md card in the main transcript — same soft strip as subagent cards.

use comb::{Line, Modifier, Span, Style};

use crate::app::state::{PlanCard, PlanSection, PlanStatus};
use crate::app::App;
use crate::render::tools::strip::{soft_bg_line, soft_bg_pad};
use crate::render::tools::tool_card::format_tool_secs;

pub(crate) fn plan_card_lines(
    card: &PlanCard,
    app: &App,
    width: usize,
    show_hint: bool,
) -> Vec<Line> {
    let theme = &app.theme;
    let bg = theme.strip;
    let (icon, icon_fg) = match card.status {
        PlanStatus::Writing => (app.spinner_char().to_string(), theme.plan),
        PlanStatus::Ready => ("▸".to_string(), theme.plan),
    };

    let mut title_spans = vec![
        Span::raw("  "),
        Span::styled(format!("{icon} "), Style::default().fg(icon_fg)),
        Span::styled(
            "Plan.md".to_string(),
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

    let detail = if card.summary.is_empty() {
        match card.status {
            PlanStatus::Writing => "writing plan…".to_string(),
            PlanStatus::Ready => "ready".to_string(),
        }
    } else {
        card.summary.clone()
    };

    vec![
        soft_bg_pad(bg, width),
        soft_bg_line(Line::from(title_spans), bg, width),
        soft_bg_line(
            Line::from(vec![
                Span::raw("    "),
                Span::styled(detail, Style::default().fg(theme.dim)),
            ]),
            bg,
            width,
        ),
        soft_bg_pad(bg, width),
    ]
}

/// Parse selectable sections: `##` headings and top-level list items.
pub(crate) fn parse_sections(body: &str) -> Vec<PlanSection> {
    let lines: Vec<&str> = body.lines().collect();
    let mut sections = Vec::new();
    let mut i = 0usize;
    while i < lines.len() {
        let raw = lines[i];
        let t = raw.trim();
        let indent = raw.len() - raw.trim_start().len();
        let is_h2 = t.starts_with("## ") && !t.starts_with("###");
        let is_list = indent == 0 && is_list_item(t);
        if is_h2 || is_list {
            let title = if is_h2 {
                t.trim_start_matches('#').trim().to_string()
            } else {
                strip_list_marker(t).to_string()
            };
            let start = i;
            i += 1;
            while i < lines.len() {
                let nraw = lines[i];
                let n = nraw.trim();
                let nindent = nraw.len() - nraw.trim_start().len();
                if n.starts_with("## ") && !n.starts_with("###") {
                    break;
                }
                if !is_h2 && nindent == 0 && is_list_item(n) {
                    break;
                }
                i += 1;
            }
            sections.push(PlanSection {
                title,
                start_line: start,
                end_line: i,
            });
        } else {
            i += 1;
        }
    }
    sections
}

fn is_list_item(t: &str) -> bool {
    let t = t.trim();
    if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        return true;
    }
    let bytes = t.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' '
}

fn strip_list_marker(t: &str) -> &str {
    let t = t.trim();
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return rest.trim();
        }
    }
    let bytes = t.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i + 1 < bytes.len() && bytes[i] == b'.' && bytes[i + 1] == b' ' {
        return t[i + 2..].trim();
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_h2_sections() {
        let body = "# Title\n\n## Alpha\n\ndetail\n\n## Beta\n\n- item\n";
        let s = parse_sections(body);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].title, "Alpha");
        assert_eq!(s[1].title, "Beta");
    }
}
