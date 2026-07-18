//! A scrollable, selectable list of text rows — the building block for menus.

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::widgets::scroll::scrollbar;
use crate::widgets::Palette;

#[derive(Default)]
pub struct List {
    pub items: Vec<String>,
    pub selected: usize,
    pub offset: usize,
}

impl List {
    pub fn new(items: Vec<String>) -> Self {
        List {
            items,
            selected: 0,
            offset: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn selected_item(&self) -> Option<&str> {
        self.items.get(self.selected).map(String::as_str)
    }

    pub fn select_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.items.len() {
            self.selected = idx;
        }
    }

    fn ensure_visible(&mut self, h: usize) {
        if h == 0 {
            return;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + h {
            self.offset = self.selected + 1 - h;
        }
    }

    /// Map a screen row inside `area` to the item index it shows, if any.
    pub fn index_at(&self, area: Rect, row: u16) -> Option<usize> {
        if row < area.y || row >= area.bottom() {
            return None;
        }
        let idx = self.offset + (row - area.y) as usize;
        (idx < self.items.len()).then_some(idx)
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, pal: &Palette) {
        if area.is_empty() {
            return;
        }
        let h = area.height as usize;
        let overflow = self.items.len() > h;
        let list_w = if overflow {
            area.width.saturating_sub(1)
        } else {
            area.width
        };
        self.ensure_visible(h);
        self.offset = self.offset.min(self.items.len().saturating_sub(h));

        for row in 0..area.height {
            let idx = self.offset + row as usize;
            let Some(text) = self.items.get(idx) else { break };
            let y = area.y + row;
            let st = if idx == self.selected {
                pal.selected
            } else {
                pal.normal
            };
            buf.paint(Rect::new(area.x, y, list_w, 1), st);
            let mut line = String::with_capacity(text.len() + 1);
            line.push(' ');
            line.push_str(text);
            buf.set_str(area.x, y, &line, st);
        }

        if overflow {
            scrollbar(
                buf,
                Rect::new(area.right() - 1, area.y, 1, area.height),
                self.items.len(),
                self.offset,
                h,
                pal.thumb,
                pal.track,
            );
        }
    }
}
