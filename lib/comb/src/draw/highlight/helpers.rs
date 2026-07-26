//! Shared helpers for language lexers — string reading, ident detection,
//! and span pushing. Reduces duplication across `rust`, `json`, `shell`, `toml`.

use crate::core::style::Style;
use crate::core::text::{Line, Span};

/// Push a slice of chars as a single styled span.
pub fn push_slice(out: &mut Line, chars: &[char], style: Style) {
    if chars.is_empty() {
        return;
    }
    out.push(Span::styled(chars.iter().collect::<String>(), style));
}

/// Read a quoted string starting at `start` (chars[start] is the quote char).
/// Returns (index past closing quote, the string including quotes).
pub fn read_string(chars: &[char], start: usize, quote: char) -> (usize, String) {
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

/// Read a single-quoted char literal (`'a'`, `'\n'`, etc.).
pub fn read_char(chars: &[char], start: usize) -> (usize, String) {
    read_string(chars, start, '\'')
}

pub fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

pub fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Read an identifier starting at `start`; returns the end index.
pub fn read_ident(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i < chars.len() && is_ident_continue(chars[i]) {
        i += 1;
    }
    i
}

/// Read a number literal (digits, hex prefix, underscores, dots).
pub fn read_number(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i < chars.len()
        && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
    {
        i += 1;
    }
    i
}

/// Ensure a line is never empty (renderers expect at least one span).
pub fn ensure_nonempty(line: &mut Line, style: Style) {
    if line.spans.is_empty() {
        line.push(Span::styled(String::new(), style));
    }
}
