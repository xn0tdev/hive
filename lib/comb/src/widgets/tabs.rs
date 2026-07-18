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

    /// Width of a tab label cell including side padding (`" {label} "`).
    fn label_width(label: &str) -> u16 {
        label.chars().count() as u16 + 2
    }

    /// Which tab label was clicked, if any. Hits the label row and the
    /// underline row beneath it when `area.height >= 2`.
    pub fn index_at(&self, area: Rect, col: u16, row: u16) -> Option<usize> {
        if !area.contains(col, row) {
            return None;
        }
        // Label is on `area.y`; underline (if drawn) on `area.y + 1`.
        if row != area.y && !(area.height >= 2 && row == area.y + 1) {
            return None;
        }
        let mut x = area.x;
        for (i, label) in self.labels.iter().enumerate() {
            let w = Self::label_width(label);
            if col >= x && col < x + w {
                return Some(i);
            }
            x = x.saturating_add(w).saturating_add(1);
        }
        None
    }

    pub fn render(&self, buf: &mut Buffer, area: Rect, pal: &Palette) {
        if area.is_empty() || self.labels.is_empty() {
            return;
        }
        // Keep the tab strip opaque (needed when the parent window is stacked).
        buf.paint(area, pal.panel);
        let mut x = area.x;
        let mut sel_x = area.x;
        let mut sel_w = 0u16;
        for (i, label) in self.labels.iter().enumerate() {
            let active = i == self.selected;
            let st = if active {
                pal.selected
            } else {
                pal.normal.patch(pal.panel)
            };
            let text = format!(" {label} ");
            let w = Self::label_width(label);
            if x + w > area.right() {
                break;
            }
            if active {
                sel_x = x;
                sel_w = w;
            }
            buf.paint(Rect::new(x, area.y, w, 1), st);
            buf.set_str(x, area.y, &text, st);
            x = x.saturating_add(w).saturating_add(1);
        }
        if sel_w > 0 && area.y + 1 < area.bottom() {
            let line = "─".repeat(sel_w as usize);
            buf.set_str(sel_x, area.y + 1, &line, pal.border.patch(pal.panel));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_at_matches_render_positions() {
        let tabs = Tabs::new(
            ["widgets", "code", "metrics", "about"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        let area = Rect::new(2, 10, 70, 2);
        // First cell of each rendered label (" {label} ").
        assert_eq!(tabs.index_at(area, 2, 10), Some(0)); // widgets
        assert_eq!(tabs.index_at(area, 12, 10), Some(1)); // code
        assert_eq!(tabs.index_at(area, 19, 10), Some(2)); // metrics
        assert_eq!(tabs.index_at(area, 29, 10), Some(3)); // about
                                                          // Underline row also hits.
        assert_eq!(tabs.index_at(area, 12, 11), Some(1));
    }
}
