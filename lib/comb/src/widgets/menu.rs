//! A dropdown / context menu: a bordered overlay wrapping a selectable [`List`],
//! rendered as a compositor layer so it floats over the scene.

use crate::core::geom::Rect;
use crate::draw::border::Border;
use crate::term::terminal::Frame;
use crate::widgets::list::List;
use crate::widgets::Palette;

pub struct Menu {
    pub list: List,
    pub border: Border,
}

impl Menu {
    pub fn new(items: Vec<String>) -> Self {
        Menu {
            list: List::new(items),
            border: Border::Rounded,
        }
    }

    /// The rectangle the menu occupies if opened with its top-left at `(x, y)`.
    pub fn area_at(&self, x: u16, y: u16) -> Rect {
        let w = self
            .list
            .items
            .iter()
            .map(|s| s.chars().count())
            .max()
            .unwrap_or(4) as u16
            + 4;
        let h = self.list.items.len() as u16 + 2;
        Rect::new(x, y, w, h)
    }

    /// Map a click at `(col, row)` — for a menu opened at `(x, y)` — to an item.
    pub fn index_at(&self, x: u16, y: u16, col: u16, row: u16) -> Option<usize> {
        let area = self.area_at(x, y);
        if !area.contains(col, row) {
            return None;
        }
        let inner = Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2);
        self.list.index_at(inner, row)
    }

    /// Render as a layer at `z`, opened with its top-left at `(x, y)`.
    pub fn render(&mut self, f: &mut Frame, x: u16, y: u16, z: i32, pal: &Palette) {
        let area = self.area_at(x, y).intersection(f.area());
        if area.is_empty() {
            return;
        }
        let (w, h) = (area.width, area.height);
        let lay = f.layer(area, z);
        lay.block(Rect::new(0, 0, w, h), self.border, pal.border, Some(pal.panel));
        let inner = Rect::new(1, 1, w.saturating_sub(2), h.saturating_sub(2));
        self.list.render(lay, inner, pal);
    }
}
