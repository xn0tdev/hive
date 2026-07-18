//! Geometry: positions, sizes, and rectangles in terminal cells.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Pos {
    pub x: u16,
    pub y: u16,
}

impl Pos {
    pub const fn new(x: u16, y: u16) -> Self {
        Pos { x, y }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

impl Size {
    pub const fn new(width: u16, height: u16) -> Self {
        Size { width, height }
    }

    pub const fn area(&self) -> u32 {
        self.width as u32 * self.height as u32
    }
}

/// An axis-aligned rectangle. `x`/`y` are the top-left corner, in cells.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn at_origin(size: Size) -> Self {
        Rect::new(0, 0, size.width, size.height)
    }

    pub const fn right(&self) -> u16 {
        self.x + self.width
    }

    pub const fn bottom(&self) -> u16 {
        self.y + self.height
    }

    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub const fn area(&self) -> u32 {
        self.width as u32 * self.height as u32
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// Shrink by `dx` columns on each side and `dy` rows on each side, clamped
    /// so the result never goes negative.
    pub fn inner(&self, dx: u16, dy: u16) -> Rect {
        let dx = dx.min(self.width / 2);
        let dy = dy.min(self.height / 2);
        Rect::new(
            self.x + dx,
            self.y + dy,
            self.width - dx * 2,
            self.height - dy * 2,
        )
    }

    /// The overlapping region of two rects (empty if they don't touch).
    pub fn intersection(&self, other: Rect) -> Rect {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = self.right().min(other.right());
        let y2 = self.bottom().min(other.bottom());
        if x2 <= x1 || y2 <= y1 {
            Rect::new(x1, y1, 0, 0)
        } else {
            Rect::new(x1, y1, x2 - x1, y2 - y1)
        }
    }
}
