//! A lightweight Rust syntax highlighter (keywords, strings, comments, numbers).

use crate::core::text::{Line, Span};
use crate::draw::highlight::HighlightTheme;

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
    "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true",
    "type", "unsafe", "use", "where", "while",
];

const TYPES: &[&str] = &[
    "String", "str", "bool", "char", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16",
    "u32", "u64", "u128", "usize", "f32", "f64", "Vec", "Option", "Result",
];

pub fn highlight(source: &str, theme: &HighlightTheme) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut in_block = false;
    for raw in source.lines() {
        lines.push(highlight_line(raw, theme, &mut in_block));
    }
    lines
}

fn highlight_line(line: &str, theme: &HighlightTheme, in_block: &mut bool) -> Line {
    let mut out = Line::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        if *in_block {
            let start = i;
            while i < chars.len() {
                if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                    i += 2;
                    *in_block = false;
                    break;
                }
                i += 1;
            }
            push_slice(&mut out, &chars[start..i], theme.comment);
            continue;
        }

        if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            push_slice(&mut out, &chars[i..], theme.comment);
            break;
        }
        if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            *in_block = true;
            i += 2;
            continue;
        }
        if chars[i] == '"' {
            let (end, s) = read_string(&chars, i, '"');
            out.push(Span::styled(s, theme.string));
            i = end;
            continue;
        }
        if chars[i] == '\'' && i + 1 < chars.len() {
            let (end, s) = read_char(&chars, i);
            out.push(Span::styled(s, theme.string));
            i = end;
            continue;
        }

        if is_ident_start(chars[i]) {
            let start = i;
            i += 1;
            while i < chars.len() && is_ident_continue(chars[i]) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let st = if KEYWORDS.contains(&word.as_str()) {
                theme.keyword
            } else if TYPES.contains(&word.as_str()) {
                theme.type_name
            } else if i < chars.len() && chars[i] == '(' {
                theme.function
            } else {
                theme.text
            };
            out.push(Span::styled(word, st));
            continue;
        }

        if chars[i].is_ascii_digit()
            || (chars[i] == '0'
                && i + 1 < chars.len()
                && (chars[i + 1] == 'x' || chars[i + 1] == 'b'))
        {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            push_slice(&mut out, &chars[start..i], theme.number);
            continue;
        }

        if "(){}[]:;,=+-*/<>!&|^~?#".contains(chars[i]) {
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

fn read_string(chars: &[char], start: usize, quote: char) -> (usize, String) {
    let mut i = start + 1;
    let mut s = String::from(quote);
    while i < chars.len() {
        s.push(chars[i]);
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
            s.push(chars[i]);
        } else if chars[i] == quote {
            i += 1;
            break;
        }
        i += 1;
    }
    (i, s)
}

fn read_char(chars: &[char], start: usize) -> (usize, String) {
    let mut i = start + 1;
    let mut s = String::from('\'');
    while i < chars.len() {
        s.push(chars[i]);
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
            s.push(chars[i]);
        } else if chars[i] == '\'' {
            i += 1;
            break;
        }
        i += 1;
    }
    (i, s)
}

fn push_slice(out: &mut Line, chars: &[char], style: crate::core::style::Style) {
    if chars.is_empty() {
        return;
    }
    out.push(Span::styled(chars.iter().collect::<String>(), style));
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}
