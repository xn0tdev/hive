//! A scrollable block of lines and the low-level scrollbar it uses.

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::text::Line;
use crate::term::event::MouseKind;
use crate::widgets::scrollbar::Scrollbar;
use crate::widgets::Palette;

/// A scrollable view over a block of styled lines. Uses a customizable
/// [`Scrollbar`] (draggable thumb, custom glyphs) when content overflows.
#[derive(Default)]
pub struct ScrollView {
    pub lines: Vec<Line>,
    pub offset: usize,
    pub scrollbar: Scrollbar,
}

impl ScrollView {
    pub fn new(lines: Vec<Line>) -> Self {
        ScrollView {
            lines,
            offset: 0,
            scrollbar: Scrollbar::default(),
        }
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let h = self.lines.len();
        let last = h.saturating_sub(1) as isize;
        self.offset = (self.offset as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, pal: &Palette) {
        if area.is_empty() {
            return;
        }
        let h = area.height as usize;
        let overflow = self.lines.len() > h;
        let content = self.scrollbar.content_area(area, overflow);
        self.offset = self.offset.min(self.lines.len().saturating_sub(h));
        buf.set_lines(content, &self.lines, self.offset);
        if overflow {
            let mut bar = self.scrollbar.clone();
            if bar.style.track == crate::core::style::Style::new() {
                bar.style.track = pal.track;
                bar.style.thumb = pal.thumb;
                bar.style.thumb_active = pal.thumb;
            }
            bar.render(buf, bar.area(area), self.lines.len(), h, self.offset);
        }
    }

    /// Handle wheel / drag over `area` (content + scrollbar strip).
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
            MouseKind::Up => {
                self.scrollbar.end_drag();
                false
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::text::Span;
    use crate::term::event::MouseButton;

    #[test]
    fn drag_on_custom_bar_starts_drag() {
        let mut view = ScrollView::new(
            (0..30)
                .map(|i| Line::from(Span::raw(format!("line {i}"))))
                .collect(),
        );
        view.scrollbar.style.track_glyph = '░';
        view.scrollbar.style.thumb_glyph = '▐';
        let area = Rect::new(0, 0, 10, 8);
        view.handle_mouse(area, MouseKind::Down(MouseButton::Left), 9, 0);
        assert!(view.scrollbar.is_dragging());
    }
}
