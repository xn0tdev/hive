//! A short-lived notification that floats in a corner as a compositor layer.

use std::time::{Duration, Instant};

use crate::core::geom::Rect;
use crate::draw::border::Border;
use crate::term::terminal::Frame;
use crate::widgets::Palette;

/// One toast message with an expiry time.
#[derive(Clone, Debug)]
pub struct Toast {
    pub message: String,
    pub until: Instant,
}

impl Toast {
    pub fn new(message: impl Into<String>, duration: Duration) -> Self {
        Toast {
            message: message.into(),
            until: Instant::now() + duration,
        }
    }

    pub fn active(&self) -> bool {
        Instant::now() < self.until
    }

    /// Top-right corner of the screen, clipped to `root`.
    pub fn render(&self, f: &mut Frame, z: i32, pal: &Palette) {
        if !self.active() {
            return;
        }
        let root = f.area();
        let pad = 2usize;
        let w = (self.message.chars().count() + pad * 2).clamp(8, root.width as usize) as u16;
        let h = 3u16;
        let x = root.right().saturating_sub(w + 1);
        let y = root.y + 1;
        let area = Rect::new(x, y, w, h).intersection(root);
        if area.is_empty() {
            return;
        }
        let lay = f.layer(area, z);
        lay.block(
            Rect::new(0, 0, area.width, area.height),
            Border::Rounded,
            pal.border,
            Some(pal.panel),
        );
        lay.set_str(1, 1, &self.message, pal.normal);
    }
}

/// A tiny queue: only the newest toast is shown.
#[derive(Default)]
pub struct Toasts {
    current: Option<Toast>,
}

impl Toasts {
    pub fn show(&mut self, message: impl Into<String>, duration: Duration) {
        self.current = Some(Toast::new(message, duration));
    }

    pub fn tick(&mut self) {
        if self.current.as_ref().is_some_and(|t| !t.active()) {
            self.current = None;
        }
    }

    pub fn render(&self, f: &mut Frame, z: i32, pal: &Palette) {
        if let Some(t) = &self.current {
            t.render(f, z, pal);
        }
    }
}
