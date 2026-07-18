//! The cell grid: a `width × height` array of [`Cell`]s that widgets draw into.
//! Two buffers (front = on screen, back = next frame) are diffed so the terminal
//! only receives the cells that actually changed.

use crate::geom::{Rect, Size};
use crate::style::Style;
use crate::text::Line;

/// One character cell. `'\0'` marks a *transparent* cell — it is skipped when a
/// surface is composited, letting lower layers show through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

impl Cell {
    pub const fn blank() -> Self {
        Cell {
            ch: ' ',
            style: Style::new(),
        }
    }

    pub const fn transparent() -> Self {
        Cell {
            ch: '\0',
            style: Style::new(),
        }
    }

    pub fn is_transparent(&self) -> bool {
        self.ch == '\0'
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::blank()
    }
}

#[derive(Clone)]
pub struct Buffer {
    pub width: u16,
    pub height: u16,
    cells: Vec<Cell>,
}

impl Buffer {
    /// A buffer of blank (opaque space) cells — used for the screen root.
    pub fn blank(size: Size) -> Self {
        Buffer::filled(size, Cell::blank())
    }

    /// A buffer of transparent cells — used for overlay surfaces.
    pub fn transparent(size: Size) -> Self {
        Buffer::filled(size, Cell::transparent())
    }

    pub fn filled(size: Size, cell: Cell) -> Self {
        Buffer {
            width: size.width,
            height: size.height,
            cells: vec![cell; size.area() as usize],
        }
    }

    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        if x < self.width && y < self.height {
            Some(y as usize * self.width as usize + x as usize)
        } else {
            None
        }
    }

    pub fn get(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).map(|i| &self.cells[i])
    }

    pub fn cell_mut(&mut self, x: u16, y: u16) -> Option<&mut Cell> {
        self.index(x, y).map(|i| &mut self.cells[i])
    }

    pub fn set(&mut self, x: u16, y: u16, ch: char, style: Style) {
        if let Some(c) = self.cell_mut(x, y) {
            c.ch = ch;
            c.style = style;
        }
    }

    /// Write `s` starting at `(x, y)`, clipping at the buffer edge. Returns the
    /// column just past the last glyph written.
    pub fn set_str(&mut self, x: u16, y: u16, s: &str, style: Style) -> u16 {
        let mut cx = x;
        for ch in s.chars() {
            if cx >= self.width {
                break;
            }
            self.set(cx, y, ch, style);
            cx += 1;
        }
        cx
    }

    /// Draw a styled line at `(x, y)`, each span keeping its own style, clipped
    /// to `max_width` cells (and the buffer edge).
    pub fn set_line(&mut self, x: u16, y: u16, line: &Line, max_width: u16) {
        let mut cx = x;
        let limit = x.saturating_add(max_width).min(self.width);
        for span in &line.spans {
            for ch in span.content.chars() {
                if cx >= limit {
                    return;
                }
                self.set(cx, y, ch, span.style);
                cx += 1;
            }
        }
    }

    /// Draw `lines` top-down within `area`, starting at logical index `scroll`,
    /// clipped to the area's height and width. The Paragraph-with-scroll of comb.
    pub fn set_lines(&mut self, area: Rect, lines: &[Line], scroll: usize) {
        for row in 0..area.height {
            let idx = scroll + row as usize;
            let Some(line) = lines.get(idx) else { break };
            self.set_line(area.x, area.y + row, line, area.width);
        }
    }

    /// Fill a rectangle with `ch`/`style` (intersected with the buffer).
    pub fn fill(&mut self, rect: Rect, ch: char, style: Style) {
        let r = rect.intersection(self.area());
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                self.set(x, y, ch, style);
            }
        }
    }

    /// Paint a background over a rectangle, leaving spaces. Handy for panels.
    pub fn paint(&mut self, rect: Rect, style: Style) {
        self.fill(rect, ' ', style);
    }

    /// Copy `src` onto this buffer at `(ox, oy)`. When `skip_transparent`, cells
    /// marked transparent in `src` don't overwrite what's underneath.
    pub fn blit(&mut self, ox: u16, oy: u16, src: &Buffer, skip_transparent: bool) {
        for sy in 0..src.height {
            for sx in 0..src.width {
                let cell = src.cells[sy as usize * src.width as usize + sx as usize];
                if skip_transparent && cell.is_transparent() {
                    continue;
                }
                self.set(ox + sx, oy + sy, cell.ch, cell.style);
            }
        }
    }

    /// The cells that differ from `prev` (same dimensions assumed), as
    /// `(x, y, cell)`, in row-major order — ready to emit to the terminal.
    pub fn diff<'a>(&'a self, prev: &Buffer) -> Vec<(u16, u16, &'a Cell)> {
        let mut out = Vec::new();
        if self.width != prev.width || self.height != prev.height {
            // Different geometry: treat everything as changed.
            for y in 0..self.height {
                for x in 0..self.width {
                    out.push((x, y, &self.cells[self.index(x, y).unwrap()]));
                }
            }
            return out;
        }
        for (i, (a, b)) in self.cells.iter().zip(prev.cells.iter()).enumerate() {
            if a != b {
                let x = (i % self.width as usize) as u16;
                let y = (i / self.width as usize) as u16;
                out.push((x, y, a));
            }
        }
        out
    }

    /// The whole buffer as text, rows joined by `\n` (transparent cells become
    /// spaces). Handy for snapshot-style assertions in tests.
    pub fn text(&self) -> String {
        let mut s = String::with_capacity((self.width as usize + 1) * self.height as usize);
        for y in 0..self.height {
            for x in 0..self.width {
                let ch = self.cells[y as usize * self.width as usize + x as usize].ch;
                s.push(if ch == '\0' { ' ' } else { ch });
            }
            if y + 1 < self.height {
                s.push('\n');
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Color, Style};

    #[test]
    fn set_str_clips_at_edge() {
        let mut b = Buffer::blank(Size::new(4, 1));
        let next = b.set_str(2, 0, "hello", Style::new());
        assert_eq!(next, 4); // stopped at the right edge
        let row: String = (0..4).map(|x| b.get(x, 0).unwrap().ch).collect();
        assert_eq!(row, "  he");
    }

    #[test]
    fn diff_reports_only_changes() {
        let a = Buffer::blank(Size::new(3, 1));
        let mut b = a.clone();
        b.set(1, 0, 'x', Style::new().fg(Color::rgb(9, 9, 9)));
        let d = b.diff(&a);
        assert_eq!(d.len(), 1);
        assert_eq!((d[0].0, d[0].1, d[0].2.ch), (1, 0, 'x'));
    }
}
