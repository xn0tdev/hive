//! Ready-made, composable widgets built on top of [`core`](crate::core) and
//! [`draw`](crate::draw): a selectable [`List`], a [`ScrollView`] with a
//! scrollbar, and a dropdown/context [`Menu`] overlay.

pub mod block;
pub mod code;
pub mod diff;
pub mod input;
pub mod list;
pub mod menu;
pub mod modal;
pub mod paragraph;
pub mod scroll;
pub mod scrollbar;
pub mod table;
pub mod tabs;
pub mod toast;
pub mod window;

pub use block::{Block, Padding};
pub use code::CodeBlock;
pub use diff::{unified_diff, DiffOp, DiffTheme, DiffView};
pub use input::TextInput;
pub use list::List;
pub use menu::Menu;
pub use modal::{Modal, ModalLayout, Scrim};
pub use paragraph::Paragraph;
pub use scroll::ScrollView;
pub use scrollbar::{ScrollHit, ScrollMetrics, Scrollbar, ScrollbarStyle};
pub use table::{Table, TableCol};
pub use tabs::Tabs;
pub use toast::{Toast, Toasts};
pub use window::{ResizeEdge, Window};

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::style::Style;

/// A widget that renders into a buffer region. Stateful widgets borrow `&mut self`
/// so they can track scroll offset, selection, etc.
pub trait Widget {
    fn render(&mut self, buf: &mut Buffer, area: Rect);
}

/// A widget that responds to mouse input within its area.
pub trait Interactive: Widget {
    fn handle_mouse(
        &mut self,
        area: Rect,
        kind: crate::term::event::MouseKind,
        col: u16,
        row: u16,
    ) -> bool;
}

/// A small bundle of styles shared by the widgets so call sites stay tidy.
/// Construct with the fields you care about; the rest default to "inherit".
#[derive(Clone, Copy, Default)]
pub struct Palette {
    /// Panel / overlay background.
    pub panel: Style,
    /// Frame / border.
    pub border: Style,
    /// An ordinary row or body line.
    pub normal: Style,
    /// The highlighted (selected) row.
    pub selected: Style,
    /// Scrollbar track.
    pub track: Style,
    /// Scrollbar thumb.
    pub thumb: Style,
    /// Prompt arrow / accent text on the input strip.
    pub accent: Style,
    /// Window fill background (defaults to `panel` when unset).
    pub window_fill: Style,
    /// Window resize grip color.
    pub window_grip: Style,
}
