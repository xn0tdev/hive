//! JSON syntax highlighting.

use crate::core::text::{Line, Span};
use crate::draw::highlight::HighlightTheme;

pub fn highlight(source: &str, theme: &HighlightTheme) -> Vec<Line> {
    source.lines().map(|l| highlight_line(l, theme)).collect()
}

fn highlight_line(line: &str, theme: &HighlightTheme) -> Line {
    let trimmed = line.trim_start();
    let pad = line.len() - trimmed.len();
    let mut out = Line::new();
    if pad > 0 {
        out.push(Span::styled(" ".repeat(pad), theme.text));
    }

    let chars: Vec<char> = trimmed.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            out.push(Span::styled(chars[start..i].iter().collect::<String>(), theme.text));
            continue;
        }
        if chars[i] == '"' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            let st = if start > 0 && chars[..start].iter().any(|c| *c == ':') {
                theme.string
            } else {
                theme.type_name
            };
            out.push(Span::styled(s, st));
            continue;
        }
        if chars[i..].starts_with(&['t', 'r', 'u', 'e']) || chars[i..].starts_with(&['f', 'a', 'l', 's', 'e']) || chars[i..].starts_with(&['n', 'u', 'l', 'l']) {
            let word = if chars[i..].starts_with(&['f', 'a', 'l', 's', 'e']) {
                "false"
            } else if chars[i..].starts_with(&['n', 'u', 'l', 'l']) {
                "null"
            } else {
                "true"
            };
            out.push(Span::styled(word.to_string(), theme.keyword));
            i += word.len();
            continue;
        }
        if chars[i].is_ascii_digit() || chars[i] == '-' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '-' || chars[i] == '+') {
                i += 1;
            }
            out.push(Span::styled(chars[start..i].iter().collect::<String>(), theme.number));
            continue;
        }
        if "{}[]:,".contains(chars[i]) {
            out.push(Span::styled(chars[i].to_string(), theme.punctuation));
            i += 1;
            continue;
        }
        out.push(Span::styled(chars[i].to_string(), theme.text));
        i += 1;
    }
    if out.spans.is_empty() {
        out.push(Span::raw(""));
    }
    out
}
