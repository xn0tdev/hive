//! A flat, centered modal surface with an optional scrim.
//!
//! [`Window`](crate::widgets::Window) owns interactive window chrome (border,
//! drag and resize). `Modal` is deliberately smaller: it only lays out a
//! centered panel, shades the scene behind it, fills the panel, and returns the
//! content rectangle. Applications keep ownership of titles and controls.

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::style::{Color, Modifier, Style};
use crate::widgets::block::Padding;

/// The two rectangles produced by a [`Modal`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModalLayout {
    /// Full painted modal surface.
    pub panel: Rect,
    /// Panel area after padding.
    pub content: Rect,
}

/// Shade applied outside a modal panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scrim {
    /// Retained RGB brightness (`255` leaves colours unchanged, `0` is black).
    pub brightness: u8,
    /// Background used when the underlying cell has no concrete colour.
    pub fallback_bg: Color,
    /// Attribute added to shaded cells.
    pub modifier: Modifier,
}

impl Scrim {
    pub const fn new(brightness: u8, fallback_bg: Color) -> Self {
        Self {
            brightness,
            fallback_bg,
            modifier: Modifier::DIM,
        }
    }

    /// Shade every cell in `area` except those inside `exclude`.
    pub fn render_outside(self, buf: &mut Buffer, area: Rect, exclude: Rect) {
        let area = area.intersection(buf.area());
        let exclude = exclude.intersection(area);
        if exclude.is_empty() {
            self.render_rect(buf, area);
            return;
        }

        // Four non-overlapping bands avoid a contains check for every cell.
        self.render_rect(
            buf,
            Rect::new(area.x, area.y, area.width, exclude.y.saturating_sub(area.y)),
        );
        self.render_rect(
            buf,
            Rect::new(
                area.x,
                exclude.bottom(),
                area.width,
                area.bottom().saturating_sub(exclude.bottom()),
            ),
        );
        self.render_rect(
            buf,
            Rect::new(
                area.x,
                exclude.y,
                exclude.x.saturating_sub(area.x),
                exclude.height,
            ),
        );
        self.render_rect(
            buf,
            Rect::new(
                exclude.right(),
                exclude.y,
                area.right().saturating_sub(exclude.right()),
                exclude.height,
            ),
        );
    }

    fn render_rect(self, buf: &mut Buffer, area: Rect) {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let Some(cell) = buf.cell_mut(x, y) else {
                    continue;
                };
                if let Some(fg) = cell.style.fg {
                    cell.style.fg = Some(self.shade(fg));
                }
                cell.style.bg = Some(match cell.style.bg {
                    Some(bg) => self.shade(bg),
                    None => self.fallback_bg,
                });
                cell.style = cell.style.add(self.modifier);
            }
        }
    }

    fn shade(self, color: Color) -> Color {
        match color {
            Color::Rgb(r, g, b) => {
                let scale =
                    |channel: u8| ((u16::from(channel) * u16::from(self.brightness)) / 255) as u8;
                Color::Rgb(scale(r), scale(g), scale(b))
            }
            Color::Reset => self.fallback_bg,
        }
    }
}

impl Default for Scrim {
    fn default() -> Self {
        // A visible depth cue without turning the rest of the app black.
        Self::new(160, Color::Rgb(0x0a, 0x0a, 0x0a))
    }
}

/// Flat centered modal layout and chrome.
#[derive(Clone, Copy, Debug)]
pub struct Modal {
    width_numerator: u16,
    width_denominator: u16,
    min_width: u16,
    max_width: u16,
    height: u16,
    vertical_margin: u16,
    padding: Padding,
    fill: Style,
    scrim: Option<Scrim>,
}

impl Modal {
    /// Create a modal with a preferred height and half-screen width.
    pub fn new(height: u16) -> Self {
        Self {
            height,
            ..Self::default()
        }
    }

    /// Set the preferred width as a fraction of the available width.
    pub fn width_ratio(mut self, numerator: u16, denominator: u16) -> Self {
        self.width_numerator = numerator;
        self.width_denominator = denominator.max(1);
        self
    }

    /// Clamp the preferred width to these bounds before fitting it on screen.
    pub fn width_bounds(mut self, min: u16, max: u16) -> Self {
        self.min_width = min.min(max);
        self.max_width = max.max(min);
        self
    }

    /// Keep this many rows free above and below when the screen permits it.
    pub fn vertical_margin(mut self, rows: u16) -> Self {
        self.vertical_margin = rows;
        self
    }

    pub fn padding(mut self, horizontal: u16, vertical: u16) -> Self {
        self.padding = Padding::new(horizontal, vertical);
        self
    }

    pub fn fill(mut self, style: Style) -> Self {
        self.fill = style;
        self
    }

    /// Replace the default scrim, or pass `None` to render without one.
    pub fn scrim(mut self, scrim: Option<Scrim>) -> Self {
        self.scrim = scrim;
        self
    }

    /// Calculate panel and content rectangles without drawing.
    pub fn layout(self, area: Rect) -> ModalLayout {
        let preferred = (u32::from(area.width) * u32::from(self.width_numerator)
            / u32::from(self.width_denominator))
        .min(u32::from(u16::MAX)) as u16;
        let width = preferred
            .clamp(self.min_width, self.max_width)
            .min(area.width);
        let available_height = area
            .height
            .saturating_sub(self.vertical_margin.saturating_mul(2));
        let height = self.height.min(available_height);
        let panel = Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        );
        let content = panel.inner(self.padding.horizontal, self.padding.vertical);
        ModalLayout { panel, content }
    }

    /// Render the scrim and panel fill, returning the content geometry.
    pub fn render(self, buf: &mut Buffer, area: Rect) -> ModalLayout {
        let layout = self.layout(area);
        if layout.panel.is_empty() {
            return layout;
        }
        if let Some(scrim) = self.scrim {
            scrim.render_outside(buf, area, layout.panel);
        }
        buf.paint(layout.panel, self.fill);
        layout
    }
}

impl Default for Modal {
    fn default() -> Self {
        Self {
            width_numerator: 1,
            width_denominator: 2,
            min_width: 0,
            max_width: u16::MAX,
            height: 1,
            vertical_margin: 1,
            padding: Padding::default(),
            fill: Style::default(),
            scrim: Some(Scrim::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geom::Size;

    #[test]
    fn layout_is_centered_clamped_and_padded() {
        let modal = Modal::new(12)
            .width_ratio(2, 3)
            .width_bounds(40, 52)
            .padding(3, 1);
        let layout = modal.layout(Rect::new(10, 5, 90, 24));

        assert_eq!(layout.panel, Rect::new(29, 11, 52, 12));
        assert_eq!(layout.content, Rect::new(32, 12, 46, 10));
    }

    #[test]
    fn tiny_areas_never_produce_rectangles_outside_the_screen() {
        let area = Rect::new(4, 7, 6, 2);
        let layout = Modal::new(20)
            .width_bounds(40, 80)
            .padding(8, 4)
            .layout(area);

        assert_eq!(layout.panel, Rect::new(4, 8, 6, 0));
        assert!(layout.content.is_empty());
        assert!(layout.panel.right() <= area.right());
        assert!(layout.panel.bottom() <= area.bottom());
    }

    #[test]
    fn scrim_preserves_glyphs_and_shades_colours() {
        let mut buf = Buffer::blank(Size::new(5, 3));
        let bright = Style::new()
            .fg(Color::Rgb(0xff, 0xff, 0xff))
            .bg(Color::Rgb(0x80, 0x80, 0x80));
        buf.set(0, 0, 'X', bright);
        Scrim::default().render_outside(&mut buf, Rect::new(0, 0, 5, 3), Rect::new(1, 1, 3, 1));

        let cell = buf.get(0, 0).unwrap();
        assert_eq!(cell.ch, 'X');
        assert_eq!(cell.style.fg, Some(Color::Rgb(0xa0, 0xa0, 0xa0)));
        assert_eq!(cell.style.bg, Some(Color::Rgb(0x50, 0x50, 0x50)));
        assert!(cell.style.mods.contains(Modifier::DIM));
        assert_eq!(buf.get(2, 1).unwrap().style, Style::default());
    }

    #[test]
    fn render_fills_only_the_panel() {
        let panel_bg = Color::Rgb(0x24, 0x24, 0x24);
        let mut buf = Buffer::blank(Size::new(20, 8));
        let layout = Modal::new(4)
            .width_bounds(10, 10)
            .padding(2, 1)
            .fill(Style::new().bg(panel_bg))
            .scrim(None)
            .render(&mut buf, Rect::new(0, 0, 20, 8));

        assert_eq!(layout.panel, Rect::new(5, 2, 10, 4));
        assert_eq!(layout.content, Rect::new(7, 3, 6, 2));
        assert_eq!(buf.get(5, 2).unwrap().style.bg, Some(panel_bg));
        assert_eq!(buf.get(4, 2).unwrap().style.bg, None);
    }
}
