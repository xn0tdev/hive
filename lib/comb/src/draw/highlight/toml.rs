//! Lightweight TOML highlighting (keys, strings, comments, numbers).

use crate::core::text::{Line, Span};
use crate::draw::highlight::helpers::ensure_nonempty;
use crate::draw::highlight::HighlightTheme;

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
    let mut expect_key = true;

    while i < chars.len() {
        let c = chars[i];
        if c == '#' {
            out.push(Span::styled(
                chars[i..].iter().collect::<String>(),
                theme.comment,
            ));
            break;
        }
        if c == '"' || c == '\'' {
            let quote = c;
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
            expect_key = false;
            continue;
        }
        if c.is_ascii_digit() || (c == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
        {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '.' | '_' | '-'))
            {
                i += 1;
            }
            out.push(Span::styled(
                chars[start..i].iter().collect::<String>(),
                theme.number,
            ));
            expect_key = false;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '-' | '.'))
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let st = if matches!(word.as_str(), "true" | "false") {
                theme.keyword
            } else if expect_key {
                theme.type_name
            } else {
                theme.text
            };
            out.push(Span::styled(word, st));
            continue;
        }
        if c == '=' {
            expect_key = false;
            out.push(Span::styled(c.to_string(), theme.punctuation));
            i += 1;
            continue;
        }
        if matches!(c, '[' | ']' | '{' | '}' | ',' | '.') {
            out.push(Span::styled(c.to_string(), theme.punctuation));
            i += 1;
            continue;
        }
        if c == '\n' {
            expect_key = true;
        }
        out.push(Span::styled(c.to_string(), theme.text));
        i += 1;
    }
    ensure_nonempty(&mut out, theme.text);
    out
}
