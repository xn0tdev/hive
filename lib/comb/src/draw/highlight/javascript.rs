//! Lightweight JavaScript/TypeScript syntax highlighting.

use crate::core::text::{Line, Span};
use crate::draw::highlight::helpers::{ensure_nonempty, push_slice, read_number, read_string};
use crate::draw::highlight::HighlightTheme;

const KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "infer",
    "instanceof",
    "interface",
    "keyof",
    "let",
    "namespace",
    "new",
    "null",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "satisfies",
    "set",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "type",
    "typeof",
    "undefined",
    "var",
    "while",
    "with",
    "yield",
];

const TYPES: &[&str] = &[
    "any", "Array", "bigint", "Boolean", "boolean", "Date", "Error", "Map", "never", "Number",
    "number", "Object", "Promise", "Record", "RegExp", "Set", "String", "string", "Symbol",
    "symbol", "unknown", "void", "WeakMap", "WeakSet",
];

#[derive(Default)]
struct LexerState {
    block_comment: bool,
    template_string: bool,
}

pub fn highlight(source: &str, theme: &HighlightTheme) -> Vec<Line> {
    let mut state = LexerState::default();
    source
        .lines()
        .map(|line| highlight_line(line, theme, &mut state))
        .collect()
}

fn highlight_line(line: &str, theme: &HighlightTheme, state: &mut LexerState) -> Line {
    let mut out = Line::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        if state.block_comment {
            let start = i;
            while i < chars.len() {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    i += 2;
                    state.block_comment = false;
                    break;
                }
                i += 1;
            }
            push_slice(&mut out, &chars[start..i], theme.comment);
            continue;
        }

        if state.template_string {
            let (end, closed) = read_template(&chars, i, false);
            push_slice(&mut out, &chars[i..end], theme.string);
            state.template_string = !closed;
            i = end;
            continue;
        }

        if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
            push_slice(&mut out, &chars[i..], theme.comment);
            break;
        }
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            let start = i;
            i += 2;
            state.block_comment = true;
            while i < chars.len() {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    i += 2;
                    state.block_comment = false;
                    break;
                }
                i += 1;
            }
            push_slice(&mut out, &chars[start..i], theme.comment);
            continue;
        }

        if chars[i] == '`' {
            let (end, closed) = read_template(&chars, i, true);
            push_slice(&mut out, &chars[i..end], theme.string);
            state.template_string = !closed;
            i = end;
            continue;
        }
        if matches!(chars[i], '\'' | '"') {
            let (end, string) = read_string(&chars, i, chars[i]);
            out.push(Span::styled(string, theme.string));
            i = end;
            continue;
        }

        if is_ident_start(chars[i]) {
            let end = read_ident(&chars, i);
            let word: String = chars[i..end].iter().collect();
            let next = chars[end..]
                .iter()
                .position(|c| !c.is_whitespace())
                .map(|n| end + n);
            let style = if KEYWORDS.contains(&word.as_str()) {
                theme.keyword
            } else if TYPES.contains(&word.as_str())
                || word.chars().next().is_some_and(char::is_uppercase)
            {
                theme.type_name
            } else if next.is_some_and(|next| chars[next] == '(') {
                theme.function
            } else {
                theme.text
            };
            out.push(Span::styled(word, style));
            i = end;
            continue;
        }

        if chars[i].is_ascii_digit() {
            let end = read_number(&chars, i);
            push_slice(&mut out, &chars[i..end], theme.number);
            i = end;
            continue;
        }

        if "(){}[]:;,.=+-*/<>!&|^~?@#".contains(chars[i]) {
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

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, '_' | '$')
}

fn read_ident(chars: &[char], start: usize) -> usize {
    let mut i = start;
    while i < chars.len() && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '$')) {
        i += 1;
    }
    i
}

/// Reads a template string segment. When `opening` is false the backtick that
/// started the template was on an earlier source line.
fn read_template(chars: &[char], start: usize, opening: bool) -> (usize, bool) {
    let mut i = start + usize::from(opening);
    while i < chars.len() {
        if chars[i] == '\\' {
            i = (i + 2).min(chars.len());
            continue;
        }
        if chars[i] == '`' {
            return (i + 1, true);
        }
        i += 1;
    }
    (i, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::highlight::HighlightTheme;

    fn has_span(line: &Line, text: &str, style: crate::core::style::Style) -> bool {
        line.spans
            .iter()
            .any(|span| span.content == text && span.style == style)
    }

    #[test]
    fn highlights_javascript_semantics() {
        let theme = HighlightTheme::dark();
        let lines = highlight(
            "function nav(user: User, amount = 42) {\n  const label = \"paid\";\n  // done\n}",
            &theme,
        );

        assert!(has_span(&lines[0], "function", theme.keyword));
        assert!(has_span(&lines[0], "nav", theme.function));
        assert!(has_span(&lines[0], "User", theme.type_name));
        assert!(has_span(&lines[0], "42", theme.number));
        assert!(has_span(&lines[1], "const", theme.keyword));
        assert!(has_span(&lines[1], "\"paid\"", theme.string));
        assert!(has_span(&lines[2], "// done", theme.comment));
    }

    #[test]
    fn multiline_comments_and_templates_keep_their_colour() {
        let theme = HighlightTheme::dark();
        let lines = highlight("/* first\nsecond */\nconst text = `hello\nworld`;", &theme);

        assert!(has_span(&lines[0], "/* first", theme.comment));
        assert!(has_span(&lines[1], "second */", theme.comment));
        assert!(has_span(&lines[2], "`hello", theme.string));
        assert!(has_span(&lines[3], "world`", theme.string));
    }
}
