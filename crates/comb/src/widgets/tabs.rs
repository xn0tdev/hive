//! A horizontal tab bar with click hit-testing.

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::widgets::Palette;

#[derive(Default)]
pub struct Tabs {
    pub labels: Vec<String>,
    pub selected: usize,
}

impl Tabs {
    pub fn new(labels: Vec<String>) -> Self {
        Tabs {
            labels,
            selected: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    pub fn select(&mut self, idx: usize) {
        if idx < self.labels.len() {
            self.selected = idx;
        }
    }

    pub fn select_next(&mut self) {
        if !self.labels.is_empty() {
            self.selected = (self.selected + 1) % self.labels.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.labels.is_empty() {
            self.selected = (self.selected + self.labels.len() - 1) % self.labels.len();
        }
    }

    /// Which tab label was clicked, if any.
    pub fn index_at(&self, area: Rect, col: u16, row: u16) -> Option<usize> {
        if row != area.y || !area.contains(col, row) {
            return None;
        }
        let mut x = area.x;
        for (i, label) in self.labels.iter().enumerate() {
            let w = label.chars().count() as u16 + 3;
            if col >= x && col < x + w {
                return Some(i);
            }
            x += w + 1;
        }
        None
    }

    pub fn render(&self, buf: &mut Buffer, area: Rect, pal: &Palette) {
        if area.is_empty() || self.labels.is_empty() {
            return;
        }
        let mut x = area.x;
        let mut sel_x = area.x;
        let mut sel_w = 0u16;
        for (i, label) in self.labels.iter().enumerate() {
            let active = i == self.selected;
            let st = if active { pal.selected } else { pal.normal };
            let text = format!(" {label} ");
            let w = text.chars().count() as u16;
            if x + w > area.right() {
                break;
            }
            if active {
                sel_x = x;
                sel_w = w;
            }
            buf.paint(Rect::new(x, area.y, w, 1), st);
            buf.set_str(x, area.y, &text, st);
            x += w + 1;
        }
        if sel_w > 0 && area.y + 1 < area.bottom() {
            let line = "─".repeat(sel_w as usize);
            buf.set_str(sel_x, area.y + 1, &line, pal.border);
        }
    }
}
