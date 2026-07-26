//! A composable block: optional border, title, padding, and background fill.
//!
//! `Block` is a *container* — it renders its chrome and hands back the inner
//! area for content. Unlike calling `buf.border()` + `buf.paint()` separately,
//! `Block` bundles everything and stays composable:
//!
//! ```ignore
//! use comb::{Block, Border, Style, Rect, Buffer};
//! let block = Block::default()
//!     .border(Border::Rounded)
//!     .title("Settings")
//!     .padding(1, 1)
//!     .fill(Style::new().bg(Color::rgb(0x1c, 0x1c, 0x1c)));
//! let inner = block.render(buf, area);
//! buf.set_str(inner.x, inner.y, "content", Style::new());
//! ```

use crate::core::buffer::Buffer;
use crate::core::geom::{Rect, Size};
use crate::core::style::Style;
use crate::core::text::Line;
use crate::draw::border::Border;

/// Container chrome: border + title + padding + optional fill.
#[derive(Clone, Debug, Default)]
pub struct Block<'a> {
    pub border: Option<Border>,
    pub border_style: Style,
    pub title: Option<Line>,
    pub title_style: Style,
    pub fill: Option<Style>,
    pub padding: Padding,
    pub _phantom: std::marker::PhantomData<&'a ()>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Padding {
    pub horizontal: u16,
    pub vertical: u16,
}

impl Padding {
    pub const fn new(h: u16, v: u16) -> Self {
        Padding {
            horizontal: h,
            vertical: v,
        }
    }
}

impl<'a> Block<'a> {
    pub fn new() -> Self {
        Block::default()
    }

    pub fn border(mut self, b: Border) -> Self {
        self.border = Some(b);
        self
    }

    pub fn border_style(mut self, s: Style) -> Self {
        self.border_style = s;
        self
    }

    pub fn title(mut self, title: impl Into<Line>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn title_style(mut self, s: Style) -> Self {
        self.title_style = s;
        self
    }

    pub fn fill(mut self, s: Style) -> Self {
        self.fill = Some(s);
        self
    }

    pub fn padding(mut self, h: u16, v: u16) -> Self {
        self.padding = Padding::new(h, v);
        self
    }

    /// How much chrome (border + padding) eats from each edge.
    fn chrome(&self) -> (u16, u16) {
        let border_w = if self.border.is_some() { 1 } else { 0 };
        let dx = border_w + self.padding.horizontal;
        let dy = border_w + self.padding.vertical;
        (dx, dy)
    }

    /// The content area inside the chrome, without rendering.
    pub fn inner(&self, area: Rect) -> Rect {
        let (dx, dy) = self.chrome();
        area.inner(dx, dy)
    }

    /// Minimum size needed to show border + padding (content area could be 0×0).
    pub fn min_size(&self) -> Size {
        let (dx, dy) = self.chrome();
        Size::new(dx * 2, dy * 2)
    }

    /// Render chrome into `buf`, return the inner content area.
    pub fn render(&self, buf: &mut Buffer, area: Rect) -> Rect {
        if area.is_empty() {
            return Rect::default();
        }
        if let Some(f) = self.fill {
            buf.paint(area, f);
        }
        if let Some(b) = self.border {
            if let Some(ref title) = self.title {
                let styled_title = Line {
                    spans: title
                        .spans
                        .iter()
                        .map(|s| {
                            let mut span = s.clone();
                            span.style = span.style.patch(self.title_style);
                            span
                        })
                        .collect(),
                };
                let bs = self.border_style.patch(self.fill.unwrap_or_default());
                buf.border_title(area, b, bs, &styled_title);
            } else {
                let bs = self.border_style.patch(self.fill.unwrap_or_default());
                buf.border(area, b, bs);
            }
        } else if let Some(ref title) = self.title {
            let y = area.y;
            let title_line = Line {
                spans: title
                    .spans
                    .iter()
                    .map(|s| {
                        let mut span = s.clone();
                        span.style = span.style.patch(self.title_style);
                        span
                    })
                    .collect(),
            };
            let ts = self.title_style.patch(self.fill.unwrap_or_default());
            buf.set_line(area.x, y, &title_line, area.width);
            let _ = ts;
        }
        self.inner(area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geom::Size;
    use crate::core::style::Color;
    use crate::term::terminal::render;

    #[test]
    fn inner_without_border() {
        let b = Block::default().padding(2, 1);
        let area = Rect::new(0, 0, 20, 10);
        assert_eq!(b.inner(area), Rect::new(2, 1, 16, 8));
    }

    #[test]
    fn inner_with_border_and_padding() {
        let b = Block::default().border(Border::Rounded).padding(1, 0);
        let area = Rect::new(0, 0, 20, 10);
        assert_eq!(b.inner(area), Rect::new(2, 1, 16, 8));
    }

    #[test]
    fn render_fills_and_borders() {
        let bg = Style::new().bg(Color::rgb(0x22, 0x22, 0x22));
        let b = Block::default()
            .border(Border::Rounded)
            .fill(bg)
            .title("Hi");
        let buf = render(Size::new(12, 5), |f| {
            b.render(f.buffer(), Rect::new(0, 0, 12, 5));
        });
        assert_eq!(buf.get(0, 0).unwrap().ch, '╭');
        assert_eq!(buf.get(11, 4).unwrap().ch, '╯');
        assert_eq!(
            buf.get(0, 1).unwrap().style.bg,
            Some(Color::rgb(0x22, 0x22, 0x22))
        );
    }
}
