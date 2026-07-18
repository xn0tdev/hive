//! A customizable vertical scrollbar with hit-testing and draggable thumb.
//!
//! Unlike the legacy one-liner [`scrollbar()`], [`Scrollbar`] carries its own
//! style (glyphs, colours, width) and drag state so callers can grab the thumb
//! and pull, or click the track to jump.

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::style::Style;
use crate::term::event::{MouseButton, MouseKind};

/// Visual appearance of a [`Scrollbar`].
#[derive(Clone, Copy, Debug)]
pub struct ScrollbarStyle {
    /// Track width in columns (usually 1, sometimes 2 for a wider thumb).
    pub width: u16,
    pub track: Style,
    pub thumb: Style,
    /// While the thumb is being dragged.
    pub thumb_active: Style,
    pub track_glyph: char,
    pub thumb_glyph: char,
}

impl Default for ScrollbarStyle {
    fn default() -> Self {
        ScrollbarStyle {
            width: 1,
            track: Style::new(),
            thumb: Style::new(),
            thumb_active: Style::new(),
            track_glyph: '│',
            thumb_glyph: '█',
        }
    }
}

/// Geometry of the thumb within a track of `track_h` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollMetrics {
    pub thumb_len: usize,
    pub thumb_top: usize,
    pub max_offset: usize,
}

impl ScrollMetrics {
    pub fn compute(total: usize, viewport: usize, offset: usize, track_h: usize) -> Self {
        if track_h == 0 || total <= viewport || viewport == 0 {
            return ScrollMetrics {
                thumb_len: track_h,
                thumb_top: 0,
                max_offset: 0,
            };
        }
        let thumb_len = ((viewport * track_h) / total).clamp(1, track_h);
        let max_offset = total - viewport;
        let span = track_h - thumb_len;
        let thumb_top = if max_offset == 0 {
            0
        } else {
            (offset.min(max_offset) * span)
                .checked_div(max_offset)
                .unwrap_or(0)
        };
        ScrollMetrics {
            thumb_len,
            thumb_top,
            max_offset,
        }
    }

    pub fn offset_for_thumb_top(&self, thumb_top: usize, track_h: usize) -> usize {
        let span = track_h.saturating_sub(self.thumb_len);
        if self.max_offset == 0 || span == 0 {
            0
        } else {
            (thumb_top.min(span) * self.max_offset)
                .checked_div(span)
                .unwrap_or(0)
        }
    }
}

/// Stateful scrollbar: style + optional drag anchor (row offset inside the thumb).
#[derive(Clone, Debug, Default)]
pub struct Scrollbar {
    pub style: ScrollbarStyle,
    drag: Option<isize>,
}

impl Scrollbar {
    pub fn new(style: ScrollbarStyle) -> Self {
        Scrollbar { style, drag: None }
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    /// The column slice on the right edge of `outer` reserved for this bar.
    pub fn area(&self, outer: Rect) -> Rect {
        let w = self.style.width.min(outer.width);
        Rect::new(outer.right().saturating_sub(w), outer.y, w, outer.height)
    }

    /// `outer` minus the scrollbar strip (when `needs_bar`).
    pub fn content_area(&self, outer: Rect, needs_bar: bool) -> Rect {
        if !needs_bar {
            return outer;
        }
        let w = self.style.width.min(outer.width);
        Rect::new(
            outer.x,
            outer.y,
            outer.width.saturating_sub(w),
            outer.height,
        )
    }

    pub fn metrics(
        &self,
        total: usize,
        viewport: usize,
        offset: usize,
        area: Rect,
    ) -> ScrollMetrics {
        ScrollMetrics::compute(total, viewport, offset, area.height as usize)
    }

    /// Inclusive screen-row range of the thumb, if any.
    pub fn thumb_rows(
        &self,
        area: Rect,
        total: usize,
        viewport: usize,
        offset: usize,
    ) -> Option<(u16, u16)> {
        let m = self.metrics(total, viewport, offset, area);
        if m.max_offset == 0 {
            return None;
        }
        let y0 = area.y + m.thumb_top as u16;
        let y1 = y0 + m.thumb_len as u16 - 1;
        Some((y0, y1))
    }

    pub fn hit(
        &self,
        area: Rect,
        total: usize,
        viewport: usize,
        offset: usize,
        col: u16,
        row: u16,
    ) -> ScrollHit {
        if !area.contains(col, row) || total <= viewport {
            return ScrollHit::None;
        }
        if let Some((y0, y1)) = self.thumb_rows(area, total, viewport, offset) {
            if row >= y0 && row <= y1 {
                return ScrollHit::Thumb;
            }
        }
        ScrollHit::Track
    }

    pub fn render(
        &self,
        buf: &mut Buffer,
        area: Rect,
        total: usize,
        viewport: usize,
        offset: usize,
    ) {
        if area.is_empty() {
            return;
        }
        let _h = area.height as usize;
        let m = self.metrics(total, viewport, offset, area);
        let st = self.style;
        let active = self.drag.is_some();
        let thumb_st = if active { st.thumb_active } else { st.thumb };

        for y in 0..area.height {
            for dx in 0..area.width {
                buf.set(area.x + dx, area.y + y, st.track_glyph, st.track);
            }
        }

        if m.max_offset == 0 {
            return;
        }

        for i in 0..m.thumb_len {
            let y = area.y + (m.thumb_top + i) as u16;
            for dx in 0..area.width {
                buf.set(area.x + dx, y, st.thumb_glyph, thumb_st);
            }
        }
    }

    /// Handle mouse input over the scrollbar `area`. Updates `offset` in place.
    /// Returns `true` if the event was consumed.
    #[allow(clippy::too_many_arguments)]
    pub fn on_mouse(
        &mut self,
        area: Rect,
        total: usize,
        viewport: usize,
        offset: &mut usize,
        kind: MouseKind,
        col: u16,
        row: u16,
    ) -> bool {
        if total <= viewport || !area.contains(col, row) {
            if kind == MouseKind::Up {
                self.end_drag();
            }
            return false;
        }

        let track_h = area.height as usize;
        let m = self.metrics(total, viewport, *offset, area);

        match kind {
            MouseKind::Down(MouseButton::Left) => {
                match self.hit(area, total, viewport, *offset, col, row) {
                    ScrollHit::Thumb => {
                        let rel = row as isize - area.y as isize - m.thumb_top as isize;
                        self.drag = Some(rel);
                        true
                    }
                    ScrollHit::Track => {
                        let rel = row as isize - area.y as isize;
                        let thumb_top = (rel - m.thumb_len as isize / 2)
                            .clamp(0, (track_h - m.thumb_len) as isize)
                            as usize;
                        *offset = m.offset_for_thumb_top(thumb_top, track_h);
                        true
                    }
                    ScrollHit::None => false,
                }
            }
            MouseKind::Drag => {
                if let Some(drag) = self.drag {
                    let rel = row as isize - area.y as isize - drag;
                    let thumb_top = rel.clamp(0, (track_h - m.thumb_len) as isize) as usize;
                    *offset = m.offset_for_thumb_top(thumb_top, track_h);
                    true
                } else {
                    false
                }
            }
            MouseKind::Up => {
                let was = self.drag.is_some();
                self.end_drag();
                was
            }
            MouseKind::ScrollUp => {
                *offset = offset.saturating_sub(3);
                true
            }
            MouseKind::ScrollDown => {
                *offset = (*offset + 3).min(m.max_offset);
                true
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrollHit {
    None,
    Track,
    Thumb,
}

/// Legacy helper — draws a default-style bar in one column.
pub fn scrollbar(
    buf: &mut Buffer,
    area: Rect,
    total: usize,
    offset: usize,
    viewport: usize,
    thumb: Style,
    track: Style,
) {
    let bar = Scrollbar::new(ScrollbarStyle {
        thumb,
        track,
        ..ScrollbarStyle::default()
    });
    bar.render(buf, area, total, viewport, offset);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_top_roundtrip() {
        let m = ScrollMetrics::compute(100, 10, 45, 20);
        let top = m.thumb_top;
        let off = m.offset_for_thumb_top(top, 20);
        assert_eq!(off, 45);
    }

    #[test]
    fn drag_updates_offset() {
        let area = Rect::new(10, 0, 1, 10);
        let mut bar = Scrollbar::default();
        let mut off = 0usize;
        bar.on_mouse(
            area,
            50,
            10,
            &mut off,
            MouseKind::Down(MouseButton::Left),
            10,
            5,
        );
        assert!(off > 0);
    }
}
