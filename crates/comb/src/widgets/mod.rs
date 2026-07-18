//! Ready-made, composable widgets built on top of [`core`](crate::core) and
//! [`draw`](crate::draw): a selectable [`List`], a [`ScrollView`] with a
//! scrollbar, and a dropdown/context [`Menu`] overlay.

pub mod list;
pub mod menu;
pub mod scroll;

pub use list::List;
pub use menu::Menu;
pub use scroll::{scrollbar, ScrollView};

use crate::core::style::Style;

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
}
