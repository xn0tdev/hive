//! Shell / bash syntax highlighting.

use crate::core::text::{Line, Span};
use crate::draw::highlight::HighlightTheme;

const KEYWORDS: &[&str] = &[
    "if", "then", "else", "fi", "for", "do", "done", "in", "case", "esac", "function", "return",
    "export", "local",
];

pub fn highlight(source: &str, theme: &HighlightTheme) -> Vec<Line> {
    source.lines().map(|l| highlight_line(l, theme)).collect()
}

fn highlight_line(line: &str, theme: &HighlightTheme) -> Line {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return Line::from(Span::styled(line.to_string(), theme.comment));
    }

    let mut out = Line::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        if chars[i] == '"' || chars[i] == '\'' {
            let quote = chars[i];
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 2;
                    continue;
                }
                if chars[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            out.push(Span::styled(
                chars[start..i].iter().collect::<String>(),
                theme.string,
            ));
            continue;
        }
        if chars[i] == '$' {
            let start = i;
            i += 1;
            if i < chars.len() && chars[i] == '{' {
                while i < chars.len() && chars[i] != '}' {
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
            } else if i < chars.len() && (chars[i].is_ascii_alphabetic() || chars[i] == '_') {
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
            }
            out.push(Span::styled(
                chars[start..i].iter().collect::<String>(),
                theme.type_name,
            ));
            continue;
        }
        if chars[i].is_ascii_alphabetic() || chars[i] == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let st = if KEYWORDS.contains(&word.as_str()) {
                theme.keyword
            } else {
                theme.text
            };
            out.push(Span::styled(word, st));
            continue;
        }
        if "#|&;<>".contains(chars[i]) {
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
