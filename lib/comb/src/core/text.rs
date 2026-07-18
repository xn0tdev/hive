//! Styled text: a [`Span`] is a run of one style, a [`Line`] is a row of spans.

use unicode_width::UnicodeWidthStr;

use crate::core::style::Style;

#[derive(Clone, Debug, Default)]
pub struct Span {
    pub content: String,
    pub style: Style,
}

impl Span {
    pub fn raw(content: impl Into<String>) -> Self {
        Span {
            content: content.into(),
            style: Style::new(),
        }
    }

    pub fn styled(content: impl Into<String>, style: Style) -> Self {
        Span {
            content: content.into(),
            style,
        }
    }

    pub fn width(&self) -> usize {
        UnicodeWidthStr::width(self.content.as_str())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Line {
    pub spans: Vec<Span>,
}

impl Line {
    pub fn new() -> Self {
        Line { spans: Vec::new() }
    }

    pub fn from_spans(spans: Vec<Span>) -> Self {
        Line { spans }
    }

    pub fn push(&mut self, span: Span) {
        self.spans.push(span);
    }

    pub fn width(&self) -> usize {
        self.spans.iter().map(Span::width).sum()
    }
}

impl From<&str> for Line {
    fn from(s: &str) -> Self {
        Line::from_spans(vec![Span::raw(s)])
    }
}

impl From<String> for Line {
    fn from(s: String) -> Self {
        Line::from_spans(vec![Span::raw(s)])
    }
}

impl From<Span> for Line {
    fn from(s: Span) -> Self {
        Line::from_spans(vec![s])
    }
}

impl From<Vec<Span>> for Line {
    fn from(spans: Vec<Span>) -> Self {
        Line::from_spans(spans)
    }
}
