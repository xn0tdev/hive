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

pub mod border;
pub mod buffer;
pub mod effects;
pub mod event;
pub mod geom;
pub mod style;
pub mod surface;
pub mod terminal;
pub mod text;

pub use border::Border;
pub use buffer::{Buffer, Cell};
pub use event::{Event, Key, KeyCode, KeyMods, Mouse, MouseButton, MouseKind};
pub use geom::{Pos, Rect, Size};
pub use style::{Color, Modifier, Style};
pub use surface::{Compositor, Surface};
pub use terminal::{render, restore, Frame, MouseMode, Terminal};
pub use text::{Line, Span};
