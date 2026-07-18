//! A scrollable block of lines and the low-level scrollbar it uses.

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::style::Style;
use crate::core::text::Line;
use crate::widgets::Palette;

/// Draw a vertical scrollbar in `area` (typically one column wide): a full-height
/// track with a thumb whose size and position reflect `offset`/`viewport`/`total`.
pub fn scrollbar(
    buf: &mut Buffer,
    area: Rect,
    total: usize,
    offset: usize,
    viewport: usize,
    thumb: Style,
    track: Style,
) {
    if area.height == 0 {
        return;
    }
    let h = area.height as usize;
    for y in 0..area.height {
        buf.set(area.x, area.y + y, '│', track);
    }
    if total <= viewport || viewport == 0 {
        return;
    }
    let thumb_len = ((viewport * h) / total).clamp(1, h);
    let max_off = total - viewport;
    let span = h - thumb_len;
    let pos = (offset.min(max_off) * span).checked_div(max_off).unwrap_or(0);
    for y in 0..thumb_len {
        buf.set(area.x, area.y + (pos + y) as u16, '█', thumb);
    }
}

/// A scrollable view over a block of styled lines. Reserves the last column for
/// a scrollbar when the content overflows the viewport.
#[derive(Default)]
pub struct ScrollView {
    pub lines: Vec<Line>,
    pub offset: usize,
}

impl ScrollView {
    pub fn new(lines: Vec<Line>) -> Self {
        ScrollView { lines, offset: 0 }
    }

    /// Scroll by `delta` rows (negative scrolls up), clamped to the content.
    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.lines.len().saturating_sub(1) as isize;
        self.offset = (self.offset as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, pal: &Palette) {
        if area.is_empty() {
            return;
        }
        let h = area.height as usize;
        let overflow = self.lines.len() > h;
        let content_w = if overflow {
            area.width.saturating_sub(1)
        } else {
            area.width
        };
        self.offset = self.offset.min(self.lines.len().saturating_sub(h));
        buf.set_lines(
            Rect::new(area.x, area.y, content_w, area.height),
            &self.lines,
            self.offset,
        );
        if overflow {
            scrollbar(
                buf,
                Rect::new(area.right() - 1, area.y, 1, area.height),
                self.lines.len(),
                self.offset,
                h,
                pal.thumb,
                pal.track,
            );
        }
    }
}
