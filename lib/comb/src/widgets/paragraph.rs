//! A text paragraph with word-wrap, alignment, and scroll.
//!
//! Handles the most common TUI text need: take a string (or styled [`Line`]s),
//! wrap it to fit a width, align each row, and scroll through the result.
//!
//! ```ignore
//! use comb::{Paragraph, Style, Rect, Buffer};
//! let mut p = Paragraph::new("Some long text that wraps…")
//!     .style(Style::new());
//! p.render(buf, area);
//! ```

use unicode_width::UnicodeWidthChar;

use crate::core::buffer::Buffer;
use crate::core::geom::{Align, Rect};
use crate::core::style::Style;
use crate::core::text::{Line, Span};

#[derive(Clone, Debug, Default)]
pub struct Paragraph {
    lines: Vec<Line>,
    pub scroll: usize,
    pub style: Style,
    pub align: Align,
    /// If true, wrap on any character boundary (no word splitting).
    pub char_wrap: bool,
}

impl Paragraph {
    pub fn new(text: impl Into<String>) -> Self {
        Paragraph {
            lines: text
                .into()
                .split('\n')
                .map(|l| Line::from(Span::styled(l.to_string(), Style::new())))
                .collect(),
            scroll: 0,
            style: Style::new(),
            align: Align::Start,
            char_wrap: false,
        }
    }

    pub fn from_lines(lines: Vec<Line>) -> Self {
        Paragraph {
            lines,
            scroll: 0,
            style: Style::new(),
            align: Align::Start,
            char_wrap: false,
        }
    }

    pub fn style(mut self, s: Style) -> Self {
        self.style = s;
        self
    }

    pub fn align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }

    pub fn char_wrap(mut self) -> Self {
        self.char_wrap = true;
        self
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.lines = text
            .into()
            .split('\n')
            .map(|l| Line::from(Span::styled(l.to_string(), Style::new())))
            .collect();
        self.scroll = 0;
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Wrap `lines` to `width` display columns and return the resulting rows.
    pub fn wrapped(&self, width: usize) -> Vec<Line> {
        if width == 0 {
            return self.lines.clone();
        }
        let mut out = Vec::new();
        for line in &self.lines {
            wrap_line(line, width, self.char_wrap, &mut out);
        }
        out
    }

    /// Total rows after wrapping at `width`.
    pub fn wrapped_height(&self, width: usize) -> usize {
        self.wrapped(width).len()
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn scroll_down(&mut self, n: usize, viewport: usize) {
        let max = self.lines.len().saturating_sub(viewport);
        self.scroll = (self.scroll + n).min(max);
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect) {
        if area.is_empty() {
            return;
        }
        let h = area.height as usize;
        let w = area.width as usize;
        let wrapped = self.wrapped(w);
        let max_scroll = wrapped.len().saturating_sub(h);
        self.scroll = self.scroll.min(max_scroll);

        buf.set_lines_on(area, &wrapped, self.scroll, self.style);
    }
}

/// Wrap a styled line to `width` columns, appending rows to `out`.
fn wrap_line(line: &Line, width: usize, char_wrap: bool, out: &mut Vec<Line>) {
    let total: usize = line.spans.iter().map(|s| s.width()).sum();
    if total <= width {
        out.push(line.clone());
        return;
    }

    if char_wrap {
        wrap_char(line, width, out);
    } else {
        wrap_word(line, width, out);
    }
}

/// Character-level wrap: break at any glyph boundary.
fn wrap_char(line: &Line, width: usize, out: &mut Vec<Line>) {
    let mut current = Line::new();
    let mut col = 0usize;

    for span in &line.spans {
        for ch in span.content.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if col + cw > width && !current.spans.is_empty() {
                out.push(std::mem::take(&mut current));
                col = 0;
            }
            if cw > 0 {
                current.spans.push(Span::styled(ch.to_string(), span.style));
                col += cw;
            }
        }
    }
    if !current.spans.is_empty() {
        out.push(current);
    }
}

/// Word-level wrap: break at spaces when possible, char-level as fallback
/// for words longer than the width.
fn wrap_word(line: &Line, width: usize, out: &mut Vec<Line>) {
    let mut current = Line::new();
    let mut col = 0usize;

    for span in &line.spans {
        let mut word = String::new();
        let mut word_w = 0usize;

        for ch in span.content.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);

            if ch == ' ' {
                // Flush accumulated word.
                if !word.is_empty() {
                    if col + word_w > width && !current.spans.is_empty() {
                        out.push(std::mem::take(&mut current));
                        col = 0;
                    }
                    // Word longer than width → char-wrap it.
                    if word_w > width {
                        for wc in word.chars() {
                            let wcw = UnicodeWidthChar::width(wc).unwrap_or(0);
                            if col + wcw > width && !current.spans.is_empty() {
                                out.push(std::mem::take(&mut current));
                                col = 0;
                            }
                            if wcw > 0 {
                                current.spans.push(Span::styled(wc.to_string(), span.style));
                                col += wcw;
                            }
                        }
                    } else {
                        current.spans.push(Span::styled(word.clone(), span.style));
                        col += word_w;
                    }
                    word.clear();
                    word_w = 0;
                }
                // Add the space or wrap if no room (don't carry space to new line).
                if col + 1 > width && !current.spans.is_empty() {
                    out.push(std::mem::take(&mut current));
                    col = 0;
                    // Skip the space — it's a separator, not content.
                } else if col < width {
                    current
                        .spans
                        .push(Span::styled(" ".to_string(), span.style));
                    col += 1;
                }
            } else {
                word.push(ch);
                word_w += cw;
            }
        }

        // Flush trailing word from this span.
        if !word.is_empty() {
            if col + word_w > width && !current.spans.is_empty() {
                out.push(std::mem::take(&mut current));
                col = 0;
            }
            if word_w > width {
                for wc in word.chars() {
                    let wcw = UnicodeWidthChar::width(wc).unwrap_or(0);
                    if col + wcw > width && !current.spans.is_empty() {
                        out.push(std::mem::take(&mut current));
                        col = 0;
                    }
                    if wcw > 0 {
                        current.spans.push(Span::styled(wc.to_string(), span.style));
                        col += wcw;
                    }
                }
            } else {
                current.spans.push(Span::styled(word, span.style));
                col += word_w;
            }
        }
    }

    if !current.spans.is_empty() {
        out.push(current);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geom::Size;
    use crate::core::style::Color;
    use crate::term::terminal::render;

    #[test]
    fn word_wrap_splits_at_spaces() {
        let p = Paragraph::new("hello world this is a long line");
        let wrapped = p.wrapped(10);
        assert!(wrapped.len() > 1);
        for line in &wrapped {
            assert!(line.width() <= 10);
        }
    }

    #[test]
    fn char_wrap_breaks_anywhere() {
        let p = Paragraph::new("abcdefghijklmnopqrstuvwxyz").char_wrap();
        let wrapped = p.wrapped(5);
        assert_eq!(wrapped.len(), 6);
        assert!(wrapped.iter().all(|l| l.width() <= 5));
    }

    #[test]
    fn short_line_stays_one_row() {
        let p = Paragraph::new("hi");
        assert_eq!(p.wrapped(80).len(), 1);
    }

    #[test]
    fn multiline_text_preserves_breaks() {
        let p = Paragraph::new("line one\nline two");
        let wrapped = p.wrapped(80);
        assert_eq!(wrapped.len(), 2);
    }

    #[test]
    fn render_shows_wrapped_text() {
        let mut p = Paragraph::new("hello world foo bar baz")
            .style(Style::new().fg(Color::rgb(0xff, 0xff, 0xff)));
        let buf = render(Size::new(10, 5), |f| {
            p.render(f.buffer(), Rect::new(0, 0, 10, 5));
        });
        let row0: String = (0..10).map(|x| buf.get(x, 0).unwrap().ch).collect();
        assert!(row0.starts_with("hello"));
    }

    #[test]
    fn wrapped_height_counts_rows() {
        let p = Paragraph::new("aaa bbb ccc ddd eee fff");
        assert_eq!(p.wrapped_height(7), 3);
    }
}
