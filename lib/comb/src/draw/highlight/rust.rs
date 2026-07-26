//! A lightweight Rust syntax highlighter (keywords, strings, comments, numbers).

use crate::core::text::{Line, Span};
use crate::draw::highlight::helpers::{
    ensure_nonempty, is_ident_start, push_slice, read_char, read_ident, read_number, read_string,
};
use crate::draw::highlight::HighlightTheme;

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
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
            let end = read_ident(&chars, i);
            let word: String = chars[i..end].iter().collect();
            let st = if KEYWORDS.contains(&word.as_str()) {
                theme.keyword
            } else if TYPES.contains(&word.as_str()) {
                theme.type_name
            } else if end < chars.len() && chars[end] == '(' {
                theme.function
            } else {
                theme.text
            };
            out.push(Span::styled(word, st));
            i = end;
            continue;
        }

        if chars[i].is_ascii_digit()
            || (chars[i] == '0'
                && i + 1 < chars.len()
                && (chars[i + 1] == 'x' || chars[i + 1] == 'b'))
        {
            let end = read_number(&chars, i);
            push_slice(&mut out, &chars[i..end], theme.number);
            i = end;
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

    ensure_nonempty(&mut out, theme.text);
    out
}
