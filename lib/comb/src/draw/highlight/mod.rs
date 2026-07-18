//! Syntax-highlighting palettes and language dispatch.

mod json;
mod rust;
mod shell;
mod toml;

use crate::core::style::{Color, Style};
use crate::core::text::{Line, Span};

/// Supported languages for [`highlight`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    Plain,
    Rust,
    Json,
    Shell,
    Toml,
}

/// Token colours for a code block. Tune per theme.
#[derive(Clone, Copy, Debug)]
pub struct HighlightTheme {
    pub text: Style,
    pub keyword: Style,
    pub string: Style,
    pub comment: Style,
    pub number: Style,
    pub type_name: Style,
    pub function: Style,
    pub punctuation: Style,
    pub line_number: Style,
}

impl HighlightTheme {
    pub fn dark() -> Self {
        let mk = |r, g, b| Style::new().fg(Color::rgb(r, g, b));
        HighlightTheme {
            text: mk(0xd4, 0xd4, 0xd4),
            keyword: mk(0xc7, 0x92, 0xea),
            string: mk(0xce, 0x91, 0x78),
            comment: mk(0x6a, 0x99, 0x59),
            number: mk(0xb5, 0xce, 0xa8),
            type_name: mk(0x4e, 0xc9, 0xb0),
            function: mk(0xd4, 0xdc, 0xa8),
            punctuation: mk(0x80, 0x80, 0x80),
            line_number: mk(0x50, 0x50, 0x50),
        }
    }
}

/// Turn source text into styled lines (one [`Line`] per input row).
pub fn highlight(source: &str, lang: Lang, theme: &HighlightTheme) -> Vec<Line> {
    match lang {
        Lang::Plain => source
            .lines()
            .map(|l| Line::from(Span::styled(l.to_string(), theme.text)))
            .collect(),
        Lang::Rust => rust::highlight(source, theme),
        Lang::Json => json::highlight(source, theme),
        Lang::Shell => shell::highlight(source, theme),
        Lang::Toml => toml::highlight(source, theme),
    }
}

/// Map a markdown fence info string (`rust`, `bash`, …) to a [`Lang`].
pub fn lang_from_info(info: &str) -> Lang {
    // Fence labels are often `rust,no_run` or `toml title="…"`.
    let token = info
        .split(|c: char| c.is_whitespace() || c == ',' || c == '{' || c == '=')
        .find(|t| !t.is_empty())
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '+' && c != '#')
        .to_ascii_lowercase();
    match token.as_str() {
        "rs" | "rust" => Lang::Rust,
        "json" | "jsonc" => Lang::Json,
        "sh" | "bash" | "shell" | "zsh" | "fish" => Lang::Shell,
        "toml" => Lang::Toml,
        _ => Lang::Plain,
    }
}

/// Prefix each line with a gutter column: ` 1 │ code…`
pub fn with_line_numbers(lines: &[Line], theme: &HighlightTheme) -> Vec<Line> {
    let digits = lines.len().to_string().len().max(2);
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let mut out = Line::new();
            out.push(Span::styled(
                format!(" {:>width$} │ ", i + 1, width = digits),
                theme.line_number,
            ));
            for s in &line.spans {
                out.push(s.clone());
            }
            out
        })
        .collect()
}
