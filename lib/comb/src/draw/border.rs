//! Box-drawing borders and framed blocks, added as methods on [`Buffer`].

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::style::Style;
use crate::core::text::{Line, Span};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Border {
    Plain,
    Rounded,
    Double,
    Thick,
}

struct Glyphs {
    tl: char,
    tr: char,
    bl: char,
    br: char,
    h: char,
    v: char,
}

fn glyphs(b: Border) -> Glyphs {
    match b {
        Border::Plain => Glyphs {
            tl: '┌',
            tr: '┐',
            bl: '└',
            br: '┘',
            h: '─',
            v: '│',
        },
        Border::Rounded => Glyphs {
            tl: '╭',
            tr: '╮',
            bl: '╰',
            br: '╯',
            h: '─',
            v: '│',
        },
        Border::Double => Glyphs {
            tl: '╔',
            tr: '╗',
            bl: '╚',
            br: '╝',
            h: '═',
            v: '║',
        },
        Border::Thick => Glyphs {
            tl: '┏',
            tr: '┓',
            bl: '┗',
            br: '┛',
            h: '━',
            v: '┃',
        },
    }
}

impl Buffer {
    /// Draw a border around `area` (needs at least 2×2).
    pub fn border(&mut self, area: Rect, b: Border, style: Style) {
        if area.width < 2 || area.height < 2 {
            return;
        }
        let g = glyphs(b);
        let (x0, y0) = (area.x, area.y);
        let (x1, y1) = (area.right() - 1, area.bottom() - 1);
        self.set(x0, y0, g.tl, style);
        self.set(x1, y0, g.tr, style);
        self.set(x0, y1, g.bl, style);
        self.set(x1, y1, g.br, style);
        for x in (x0 + 1)..x1 {
            self.set(x, y0, g.h, style);
            self.set(x, y1, g.h, style);
        }
        for y in (y0 + 1)..y1 {
            self.set(x0, y, g.v, style);
            self.set(x1, y, g.v, style);
        }
    }

    /// Optionally fill `area`, then draw a border — a framed panel in one call.
    pub fn block(&mut self, area: Rect, b: Border, border_style: Style, fill: Option<Style>) {
        if let Some(f) = fill {
            self.paint(area, f);
        }
        self.border(area, b, border_style);
    }

    /// Draw a border with a title inlaid on the top edge (` title `), padded so
    /// it sits off the corner.
    pub fn border_title(&mut self, area: Rect, b: Border, style: Style, title: &Line) {
        self.border(area, b, style);
        if area.width < 6 {
            return;
        }
        let mut t = Line::new();
        t.push(Span::styled(" ", style));
        for s in &title.spans {
            t.push(s.clone());
        }
        t.push(Span::styled(" ", style));
        self.set_line(area.x + 2, area.y, &t, area.width.saturating_sub(4));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geom::Size;
    use crate::core::style::Style;

    #[test]
    fn rounded_border_corners() {
        let mut b = Buffer::blank(Size::new(5, 3));
        b.border(Rect::new(0, 0, 5, 3), Border::Rounded, Style::new());
        assert_eq!(b.get(0, 0).unwrap().ch, '╭');
        assert_eq!(b.get(4, 0).unwrap().ch, '╮');
        assert_eq!(b.get(0, 2).unwrap().ch, '╰');
        assert_eq!(b.get(4, 2).unwrap().ch, '╯');
        assert_eq!(b.get(2, 0).unwrap().ch, '─');
        assert_eq!(b.get(0, 1).unwrap().ch, '│');
    }
}
