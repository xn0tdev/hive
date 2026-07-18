//! A floating, draggable / resizable window with a title-bar grab handle.

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::style::Style;
use crate::core::text::{Line, Span};
use crate::draw::border::Border;
use crate::term::event::{MouseButton, MouseKind};
use crate::widgets::Palette;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResizeEdge {
    Bottom,
    Right,
    BottomRight,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DragKind {
    Move { grab_x: u16, grab_y: u16 },
    Resize(ResizeEdge),
}

/// Floating chrome: title bar to move, edges to resize, clamped to a workspace.
#[derive(Clone, Debug)]
pub struct Window {
    pub title: String,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub min_w: u16,
    pub min_h: u16,
    pub border: Border,
    /// Panel fill behind content. Windows must be opaque so a window on top
    /// fully covers whatever is underneath (no bleed-through).
    pub fill: Style,
    drag: Option<DragKind>,
}

impl Window {
    pub fn new(title: impl Into<String>, x: u16, y: u16, width: u16, height: u16) -> Self {
        Window {
            title: title.into(),
            x,
            y,
            width,
            height,
            min_w: 16,
            min_h: 5,
            border: Border::Rounded,
            fill: Style::new().bg(crate::core::style::Color::rgb(0x1c, 0x1c, 0x1c)),
            drag: None,
        }
    }

    pub fn area(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, self.height)
    }

    pub fn inner(&self) -> Rect {
        self.area().inner(1, 1)
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn end_drag(&mut self) {
        self.drag = None;
    }

    /// Clamp geometry into `workspace` (usually the usable screen area).
    pub fn clamp_to(&mut self, workspace: Rect) {
        if workspace.is_empty() {
            return;
        }
        self.width = self
            .width
            .clamp(self.min_w, workspace.width.max(self.min_w));
        self.height = self
            .height
            .clamp(self.min_h, workspace.height.max(self.min_h));
        let max_x = workspace.right().saturating_sub(self.width);
        let max_y = workspace.bottom().saturating_sub(self.height);
        self.x = self.x.clamp(workspace.x, max_x.max(workspace.x));
        self.y = self.y.clamp(workspace.y, max_y.max(workspace.y));
    }

    /// Draw opaque fill + border + title. Content is left to the caller (`inner()`).
    /// Always paints `fill` first so stacking windows never leak lower layers.
    pub fn render_chrome(&self, buf: &mut Buffer, pal: &Palette, focused: bool) {
        let area = self.area();
        if area.is_empty() {
            return;
        }
        let fill = if self.fill.bg.is_some() {
            self.fill
        } else {
            pal.panel
        };
        buf.paint(area, fill);
        let border = if focused { pal.accent } else { pal.border };
        let title_st = if focused {
            pal.accent.bold().patch(fill)
        } else {
            pal.normal.patch(fill)
        };
        let title = Line::from(Span::styled(self.title.clone(), title_st));
        // Border cells keep the window fill so edges stay opaque when stacked.
        buf.border_title(area, self.border, border.patch(fill), &title);
        // Subtle resize grip on the bottom-right corner.
        if area.width >= 3 && area.height >= 2 {
            let grip = Style::new()
                .fg(crate::core::style::Color::rgb(0x70, 0x70, 0x70))
                .patch(fill);
            buf.set(area.right() - 1, area.bottom() - 1, '╯', grip);
        }
    }

    /// Handle move / resize. Returns `true` if the event was consumed.
    /// `workspace` clamps the window while dragging.
    pub fn handle_mouse(&mut self, kind: MouseKind, col: u16, row: u16, workspace: Rect) -> bool {
        let area = self.area();
        match kind {
            MouseKind::Up => {
                let was = self.drag.is_some();
                self.drag = None;
                was
            }
            MouseKind::Drag | MouseKind::Moved if self.drag.is_some() => {
                match self.drag {
                    Some(DragKind::Move { grab_x, grab_y }) => {
                        let nx = col.saturating_sub(grab_x);
                        let ny = row.saturating_sub(grab_y);
                        self.x = nx;
                        self.y = ny;
                        self.clamp_to(workspace);
                    }
                    Some(DragKind::Resize(edge)) => {
                        match edge {
                            ResizeEdge::Bottom | ResizeEdge::BottomRight => {
                                self.height =
                                    row.saturating_sub(self.y).saturating_add(1).max(self.min_h);
                            }
                            ResizeEdge::Right => {}
                        }
                        match edge {
                            ResizeEdge::Right | ResizeEdge::BottomRight => {
                                self.width =
                                    col.saturating_sub(self.x).saturating_add(1).max(self.min_w);
                            }
                            ResizeEdge::Bottom => {}
                        }
                        self.clamp_to(workspace);
                    }
                    None => {}
                }
                true
            }
            MouseKind::Down(MouseButton::Left) => {
                if !area.contains(col, row) {
                    return false;
                }
                let on_title = row == area.y;
                let on_bottom = row + 1 == area.bottom();
                let on_right = col + 1 == area.right();
                if on_title {
                    self.drag = Some(DragKind::Move {
                        grab_x: col.saturating_sub(self.x),
                        grab_y: row.saturating_sub(self.y),
                    });
                    return true;
                }
                if on_bottom && on_right {
                    self.drag = Some(DragKind::Resize(ResizeEdge::BottomRight));
                    return true;
                }
                if on_bottom {
                    self.drag = Some(DragKind::Resize(ResizeEdge::Bottom));
                    return true;
                }
                if on_right {
                    self.drag = Some(DragKind::Resize(ResizeEdge::Right));
                    return true;
                }
                // Content click — still "handled" for focus/z-order by the caller
                // when they see contains(); return false so content widgets run.
                false
            }
            _ => false,
        }
    }

    pub fn contains(&self, col: u16, row: u16) -> bool {
        self.area().contains(col, row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_drag_moves_window() {
        let mut w = Window::new("t", 4, 4, 20, 8);
        let ws = Rect::new(0, 0, 80, 40);
        assert!(w.handle_mouse(MouseKind::Down(MouseButton::Left), 6, 4, ws));
        assert!(w.is_dragging());
        w.handle_mouse(MouseKind::Drag, 16, 10, ws);
        assert_eq!((w.x, w.y), (14, 10));
    }

    #[test]
    fn resize_bottom_right_grows() {
        let mut w = Window::new("t", 2, 2, 20, 8);
        let ws = Rect::new(0, 0, 80, 40);
        let br_x = w.area().right() - 1;
        let br_y = w.area().bottom() - 1;
        assert!(w.handle_mouse(MouseKind::Down(MouseButton::Left), br_x, br_y, ws));
        w.handle_mouse(MouseKind::Drag, 40, 20, ws);
        assert!(w.width >= 30);
        assert!(w.height >= 15);
    }
}
