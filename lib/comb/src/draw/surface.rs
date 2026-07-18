//! Surfaces and layer compositing.
//!
//! A [`Surface`] is an off-screen [`Buffer`] anchored at a screen [`Rect`] with
//! a z-index. Its cells start transparent, so a surface only paints where you
//! actually draw — unless you fill a background, giving an opaque panel. The
//! [`Compositor`] stacks surfaces by z and flattens them onto a root buffer,
//! which is how overlays (menus, popups, toasts) sit *above* the scene without
//! reflowing it.

use crate::core::buffer::Buffer;
use crate::core::geom::{Rect, Size};

pub struct Surface {
    pub area: Rect,
    pub z: i32,
    pub buf: Buffer,
}

impl Surface {
    /// A transparent surface covering `area`, at depth `z`.
    pub fn new(area: Rect, z: i32) -> Self {
        Surface {
            area,
            z,
            buf: Buffer::transparent(Size::new(area.width, area.height)),
        }
    }

    pub fn buffer(&mut self) -> &mut Buffer {
        &mut self.buf
    }
}

/// Collects surfaces and flattens them, lowest z first, onto a root buffer.
#[derive(Default)]
pub struct Compositor {
    surfaces: Vec<Surface>,
}

impl Compositor {
    pub fn new() -> Self {
        Compositor::default()
    }

    pub fn push(&mut self, surface: Surface) {
        self.surfaces.push(surface);
    }

    /// Add a fresh transparent layer and hand back a mutable reference for
    /// drawing. Call sites draw into it before adding the next layer.
    pub fn layer(&mut self, area: Rect, z: i32) -> &mut Surface {
        self.surfaces.push(Surface::new(area, z));
        self.surfaces.last_mut().unwrap()
    }

    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }

    /// Composite every surface onto `root`, stable-sorted by ascending z so
    /// later same-z layers land on top.
    pub fn composite(mut self, root: &mut Buffer) {
        self.surfaces.sort_by_key(|s| s.z);
        for s in &self.surfaces {
            root.blit(s.area.x, s.area.y, &s.buf, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::style::{Color, Style};

    #[test]
    fn higher_z_wins_and_transparency_shows_through() {
        let mut root = Buffer::blank(Size::new(10, 3));
        root.set_str(0, 0, "..........", Style::new());

        let mut comp = Compositor::new();
        // Lower layer: opaque 'A' at x=1.
        let low = comp.layer(Rect::new(1, 0, 3, 1), 1).buffer();
        low.set(0, 0, 'A', Style::new().fg(Color::rgb(1, 1, 1)));
        low.set(1, 0, 'A', Style::new());
        low.set(2, 0, 'A', Style::new());
        // Higher layer overlapping: 'B' only in its first cell, rest transparent.
        let high = comp.layer(Rect::new(2, 0, 3, 1), 5).buffer();
        high.set(0, 0, 'B', Style::new());
        // cells 1,2 stay transparent → 'A'/'.' beneath show through

        comp.composite(&mut root);
        let row: String = (0..10).map(|x| root.get(x, 0).unwrap().ch).collect();
        // x1..3 = 'A' from low; x2 overwritten by 'B' from high; x3 stays 'A'
        // because high's overlapping cells are transparent.
        assert_eq!(row, ".ABA......");
    }
}
