//! `comb` — a tiny, native terminal-UI engine.
//!
//! No ratatui, no crossterm. Raw terminal control lives in [`terminal`] (termios
//! and ANSI), the screen is a diffed grid of [`Cell`]s in [`buffer`], and the
//! headline feature is compositing: [`Surface`]s stacked as [`Compositor`]
//! layers with z-order and transparency, so overlays (menus, popups, toasts)
//! paint on top of the scene without disturbing it.
//!
//! The mental model is deliberately close to what a ratatui user expects
//! (a [`Rect`], a [`Style`], [`Line`]/[`Span`] text, a `Frame` you draw into) so
//! porting an existing UI is mechanical, while the internals are ours to shape.
//!
//! ```no_run
//! use comb::{Terminal, Style, Color, Rect};
//! let mut term = Terminal::new().unwrap();
//! term.draw(|f| {
//!     let area = f.area();
//!     f.buffer().set_str(2, 1, "hello, comb", Style::new().fg(Color::rgb(0xe6,0xe6,0xe6)));
//!     // an overlay layer, composited on top:
//!     let pop = f.layer(Rect::new(4, 3, 20, 3), 10);
//!     pop.fill(Rect::new(0, 0, 20, 3), ' ', Style::new().bg(Color::rgb(0x26,0x26,0x26)));
//! }).unwrap();
//! ```

// Logic is grouped into folders; `lib.rs` just wires them and re-exports a flat,
// stable public API so callers use `comb::Buffer`, `comb::Style`, etc.
pub mod core;
pub mod draw;
pub mod term;
pub mod widgets;

// The `effects` module is re-exported by name so `comb::effects::…` keeps working.
pub use crate::draw::effects;
pub use crate::draw::highlight::{self, HighlightTheme, Lang};
pub use crate::widgets::{
    unified_diff, CodeBlock, DiffOp, DiffTheme, DiffView, List, Menu, Palette, ResizeEdge,
    ScrollView, Scrollbar, ScrollbarStyle, Tabs, TextInput, Toasts, Window,
};

pub use crate::core::buffer::{Buffer, Cell};
pub use crate::core::geom::{Pos, Rect, Size};
pub use crate::core::style::{Color, Modifier, Style};
pub use crate::core::text::{Line, Span};
pub use crate::draw::border::Border;
pub use crate::draw::surface::{Compositor, Surface};
pub use crate::term::event::{Event, Key, KeyCode, KeyMods, Mouse, MouseButton, MouseKind};
pub use crate::term::terminal::{render, render_with_cursor, restore, Frame, MouseMode, Terminal};
