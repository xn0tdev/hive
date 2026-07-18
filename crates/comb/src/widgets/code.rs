//! A scrollable syntax-highlighted code block with a draggable custom scrollbar.

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::style::Style;
use crate::core::text::Line;
use crate::draw::highlight::{self, HighlightTheme, Lang};
use crate::term::event::MouseKind;
use crate::widgets::scrollbar::Scrollbar;

/// Renders highlighted source in a panel. Rebuilds its cache when source or
/// language changes.
pub struct CodeBlock {
    pub lang: Lang,
    pub theme: HighlightTheme,
    pub scrollbar: Scrollbar,
    pub offset: usize,
    pub show_line_numbers: bool,
    source: String,
    lines: Vec<Line>,
}

impl CodeBlock {
    pub fn new(source: impl Into<String>, lang: Lang) -> Self {
        let mut block = CodeBlock {
            lang,
            theme: HighlightTheme::dark(),
            scrollbar: Scrollbar::default(),
            offset: 0,
            show_line_numbers: true,
            source: String::new(),
            lines: Vec::new(),
        };
        block.set_source(source);
        block
    }

    pub fn set_source(&mut self, source: impl Into<String>) {
        self.source = source.into();
        self.rebuild();
        self.offset = 0;
    }

    pub fn set_lang(&mut self, lang: Lang) {
        self.lang = lang;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let mut lines = highlight::highlight(&self.source, self.lang, &self.theme);
        if self.show_line_numbers {
            lines = highlight::with_line_numbers(&lines, &self.theme);
        }
        self.lines = lines;
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.lines.len().saturating_sub(1) as isize;
        self.offset = (self.offset as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, bg: Style) {
        if area.is_empty() {
            return;
        }
        buf.paint(area, bg);
        let h = area.height as usize;
        let overflow = self.lines.len() > h;
        let content = self.scrollbar.content_area(area, overflow);
        self.offset = self.offset.min(self.lines.len().saturating_sub(h));
        buf.set_lines(content, &self.lines, self.offset);
        if overflow {
            let bar = self.scrollbar.area(area);
            self.scrollbar
                .render(buf, bar, self.lines.len(), h, self.offset);
        }
    }

    /// Mouse over the code panel (content + scrollbar). Returns `true` if handled.
    pub fn handle_mouse(&mut self, area: Rect, kind: MouseKind, col: u16, row: u16) -> bool {
        let h = area.height as usize;
        let overflow = self.lines.len() > h;
        if !overflow {
            return false;
        }
        let bar_area = self.scrollbar.area(area);
        if self.scrollbar.is_dragging() || bar_area.contains(col, row) {
            return self.scrollbar.on_mouse(
                bar_area,
                self.lines.len(),
                h,
                &mut self.offset,
                kind,
                col,
                row,
            );
        }
        let content = self.scrollbar.content_area(area, true);
        if !content.contains(col, row) {
            return false;
        }
        match kind {
            MouseKind::ScrollUp => {
                self.scroll_by(-3);
                true
            }
            MouseKind::ScrollDown => {
                self.scroll_by(3);
                true
            }
            _ => false,
        }
    }

    /// End an in-progress thumb drag (call on mouse up anywhere if needed).
    pub fn end_drag(&mut self) {
        self.scrollbar.end_drag();
    }
}
