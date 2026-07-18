//! A small, dependency-free markdown-to-ratatui renderer. Handles fenced code
//! blocks, headings, bullets, and inline `code`/**bold**/*italic*. Enough to
//! make assistant output look great without pulling in a heavy parser.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Render markdown into styled logical lines (not yet wrapped to width).
pub fn render(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code = false;

    for raw in text.split('\n') {
        let trimmed = raw.trim_start();

        if trimmed.starts_with("```") {
            in_code = !in_code;
            let label = trimmed.trim_start_matches('`');
            let text = if in_code {
                format!("┌─ {} ", if label.is_empty() { "code" } else { label })
            } else {
                "└─".to_string()
            };
            lines.push(Line::from(Span::styled(
                text,
                Style::default().fg(theme.faint),
            )));
            continue;
        }

        if in_code {
            lines.push(Line::from(Span::styled(
                format!("  {raw}"),
                Style::default().fg(theme.code_fg).bg(theme.code_bg),
            )));
            continue;
        }

        if let Some(level) = heading_level(trimmed) {
            let content = trimmed[level..].trim_start();
            lines.push(Line::from(Span::styled(
                content.to_string(),
                Style::default()
                    .fg(theme.heading)
                    .add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        let (bullet, rest) = if let Some(r) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            (Some("• "), r)
        } else {
            (None, raw)
        };

        let mut spans = Vec::new();
        if let Some(b) = bullet {
            spans.push(Span::styled(
                b.to_string(),
                Style::default().fg(theme.accent),
            ));
        }
        spans.extend(inline(rest, theme));
        lines.push(Line::from(spans));
    }

    lines
}

/// Render text as plain lines (used for live streaming before markdown is final).
pub fn plain(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    text.split('\n')
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(theme.fg))))
        .collect()
}

fn heading_level(s: &str) -> Option<usize> {
    let mut count = 0;
    for ch in s.chars() {
        if ch == '#' {
            count += 1;
        } else {
            break;
        }
    }
    if count > 0 && count <= 6 && s.chars().nth(count) == Some(' ') {
        Some(count)
    } else {
        None
    }
}

fn find(chars: &[char], start: usize, pat: char) -> Option<usize> {
    (start..chars.len()).find(|&j| chars[j] == pat)
}

fn find_double(chars: &[char], start: usize) -> Option<usize> {
    let mut j = start;
    while j + 1 < chars.len() {
        if chars[j] == '*' && chars[j + 1] == '*' {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn inline(text: &str, theme: &Theme) -> Vec<Span<'static>> {
    let base = Style::default().fg(theme.fg);
    let code = Style::default().fg(theme.tool).bg(theme.code_bg);
    let bold = base.add_modifier(Modifier::BOLD);
    let italic = base.add_modifier(Modifier::ITALIC);

    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;

    while i < n {
        let c = chars[i];
        if c == '`' {
            if let Some(j) = find(&chars, i + 1, '`') {
                flush(&mut buf, &mut spans, base);
                let content: String = chars[i + 1..j].iter().collect();
                spans.push(Span::styled(format!(" {content} "), code));
                i = j + 1;
                continue;
            }
        } else if c == '*' && i + 1 < n && chars[i + 1] == '*' {
            if let Some(j) = find_double(&chars, i + 2) {
                flush(&mut buf, &mut spans, base);
                let content: String = chars[i + 2..j].iter().collect();
                spans.push(Span::styled(content, bold));
                i = j + 2;
                continue;
            }
        } else if c == '*' {
            if let Some(j) = find(&chars, i + 1, '*') {
                flush(&mut buf, &mut spans, base);
                let content: String = chars[i + 1..j].iter().collect();
                spans.push(Span::styled(content, italic));
                i = j + 1;
                continue;
            }
        }
        buf.push(c);
        i += 1;
    }

    flush(&mut buf, &mut spans, base);
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    spans
}

fn flush(buf: &mut String, spans: &mut Vec<Span<'static>>, style: Style) {
    if !buf.is_empty() {
        spans.push(Span::styled(std::mem::take(buf), style));
    }
}
