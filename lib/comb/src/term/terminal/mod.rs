//! The native terminal backend: raw mode, alternate screen, mouse reporting,
//! and a diffed draw loop that emits only the ANSI needed to turn last frame
//! into this one.
//!
//! Unix uses termios/poll/SIGWINCH; Windows uses Console VT + WaitForSingleObject.
//! Paint and input parsing are shared ANSI on both.

mod platform;

use std::fmt::Write as _;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use crate::core::buffer::{Buffer, WIDE_CONT};
use crate::core::geom::{Rect, Size};
use crate::core::style::{Color, Modifier, Style};
use crate::draw::surface::Compositor;
use crate::term::event::{self, Event, Key, KeyCode, KeyMods};

use platform::{PlatformState, Wait};

/// How long to wait after a bare ESC before treating it as the Esc key.
/// Shorter than a human Esc press interval; long enough for mouse CSI tails.
const ESC_TIMEOUT: Duration = Duration::from_millis(35);

/// Sequences enabled on enter (alt screen, mouse, kitty disambiguate, paste…).
const ENTER_SEQ: &str =
    "\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[>4;2m\x1b[>1u\x1b[?2004h\x1b[2J\x1b[H\x1b[?25l";

/// Best-effort teardown (also used by panic restore).
const EXIT_SEQ: &[u8] =
    b"\x1b[<u\x1b[>4;0m\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\x1b[?25h\x1b[?1049l";

/// How much mouse activity the terminal reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseMode {
    /// No mouse reporting.
    Off,
    /// Button presses, releases, and the wheel (default).
    Buttons,
    /// The above plus motion while a button is held (drag).
    Drag,
    /// The above plus all motion (hover). Noisier, but enables hover effects.
    Motion,
}

/// Best-effort terminal reset for panic hooks: leave the alternate screen, show
/// the cursor, stop mouse reporting, and undo raw mode if we saved the original
/// settings. Safe to call with no active `Terminal`.
pub fn restore() {
    let mut out = io::stdout();
    let _ = out.write_all(EXIT_SEQ);
    let _ = out.flush();
    platform::restore_console();
}

/// Render a single frame into an in-memory [`Buffer`] without a real terminal —
/// the closure gets the same [`Frame`] it would on screen. Used by tests.
pub fn render<F: FnOnce(&mut Frame)>(size: Size, f: F) -> Buffer {
    render_with_cursor(size, f).0
}

/// Like [`render`], but also returns the hardware cursor position requested for
/// the frame (`None` means the cursor should stay hidden).
pub fn render_with_cursor<F: FnOnce(&mut Frame)>(size: Size, f: F) -> (Buffer, Option<(u16, u16)>) {
    let mut back = Buffer::blank(size);
    let cursor = {
        let mut frame = Frame {
            root: &mut back,
            comp: Compositor::new(),
            cursor: None,
            size,
        };
        f(&mut frame);
        frame.finish()
    };
    (back, cursor)
}

/// What you draw into for one frame: a root buffer plus any overlay layers,
/// composited in z-order when the frame is finished.
pub struct Frame<'a> {
    root: &'a mut Buffer,
    comp: Compositor,
    cursor: Option<(u16, u16)>,
    size: Size,
}

impl Frame<'_> {
    pub fn size(&self) -> Size {
        self.size
    }

    pub fn area(&self) -> Rect {
        Rect::at_origin(self.size)
    }

    /// Draw directly on the base layer.
    pub fn buffer(&mut self) -> &mut Buffer {
        self.root
    }

    /// Add an overlay surface at `area`/`z` and hand back its buffer to draw on.
    /// It's composited over the base (and lower layers) when the frame finishes.
    pub fn layer(&mut self, area: Rect, z: i32) -> &mut Buffer {
        self.comp.layer(area, z).buffer()
    }

    /// Show the hardware cursor at `(x, y)` this frame (e.g. a text caret).
    pub fn set_cursor(&mut self, x: u16, y: u16) {
        self.cursor = Some((x, y));
    }

    fn finish(self) -> Option<(u16, u16)> {
        let Frame {
            root, comp, cursor, ..
        } = self;
        comp.composite(root);
        cursor
    }
}

pub struct Terminal {
    platform: PlatformState,
    front: Buffer,
    back: Buffer,
    ansi: String,
    size: Size,
    inbuf: Vec<u8>,
    /// When `inbuf` is exactly `[ESC]` awaiting more bytes (or Esc timeout).
    esc_seen_at: Option<Instant>,
    cursor_visible: bool,
    out: io::Stdout,
}

impl Terminal {
    /// Enter raw mode + the alternate screen and start mouse reporting.
    pub fn new() -> io::Result<Self> {
        let platform = platform::enter_raw()?;
        let size = platform::query_size();
        let mut term = Terminal {
            platform,
            front: Buffer::blank(size),
            back: Buffer::blank(size),
            ansi: String::with_capacity(size.area() as usize),
            size,
            inbuf: Vec::new(),
            esc_seen_at: None,
            cursor_visible: true,
            out: io::stdout(),
        };

        // alt screen · SGR mouse · modifyOtherKeys level 2 · kitty keyboard
        // disambiguate only (flag 1). Flag 8 (report all keys) + event types
        // made every letter a CSI-u press/release pair → doubled input, and
        // broke UTF-8 Cyrillic. Bracketed paste · clear · hide cursor.
        term.write_raw(ENTER_SEQ)?;
        term.cursor_visible = false;
        Ok(term)
    }

    pub fn size(&self) -> Size {
        self.size
    }

    /// Choose how much mouse activity to receive. Disables the other modes first
    /// so switching is clean. SGR extended coordinates stay on for wide screens.
    pub fn mouse_mode(&mut self, mode: MouseMode) -> io::Result<()> {
        let seq = match mode {
            MouseMode::Off => "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l",
            MouseMode::Buttons => "\x1b[?1002l\x1b[?1003l\x1b[?1000h\x1b[?1006h",
            MouseMode::Drag => "\x1b[?1000l\x1b[?1003l\x1b[?1002h\x1b[?1006h",
            MouseMode::Motion => "\x1b[?1000l\x1b[?1002l\x1b[?1003h\x1b[?1006h",
        };
        self.write_raw(seq)
    }

    /// Forget what's on screen so the next [`draw`](Self::draw) repaints every
    /// cell. Frames are flushed as a diff against the last one, so anything
    /// that writes to the terminal from outside the app — a subprocess prompt,
    /// a kernel message — leaves stale cells no diff will ever correct.
    pub fn invalidate(&mut self) -> io::Result<()> {
        self.front = Buffer::blank(self.size);
        self.write_raw("\x1b[H\x1b[2J\x1b[3J")
    }

    /// Build a frame, then flush the minimal diff to the terminal.
    pub fn draw<F: FnOnce(&mut Frame)>(&mut self, f: F) -> io::Result<()> {
        // Adapt to a resized window: reset both retained buffers and clear.
        let size = platform::query_size();
        if size != self.size {
            self.size = size;
            self.front = Buffer::blank(size);
            self.back = Buffer::blank(size);
            self.write_raw("\x1b[H\x1b[2J\x1b[3J")?;
            platform::clear_resize_flag();
        } else {
            self.back.clear(crate::core::buffer::Cell::blank());
        }

        let cursor = {
            let mut frame = Frame {
                root: &mut self.back,
                comp: Compositor::new(),
                cursor: None,
                size: self.size,
            };
            f(&mut frame);
            frame.finish()
        };

        self.render_diff(cursor);
        self.out.write_all(self.ansi.as_bytes())?;
        self.out.flush()?;
        std::mem::swap(&mut self.front, &mut self.back);
        Ok(())
    }

    fn render_diff(&mut self, cursor: Option<(u16, u16)>) {
        self.ansi.clear();
        let mut last_style: Option<Style> = None;
        let mut pen: Option<(u16, u16)> = None;
        let size = self.size;
        let ansi = &mut self.ansi;

        self.back.for_each_diff(&self.front, |x, y, cell| {
            if cell.ch == WIDE_CONT {
                pen = if x + 1 < size.width {
                    Some((x + 1, y))
                } else {
                    None
                };
                return;
            }
            if pen != Some((x, y)) {
                let _ = write!(ansi, "\x1b[{};{}H", y + 1, x + 1);
            }
            if last_style != Some(cell.style) {
                ansi.push_str(&sgr(cell.style));
                last_style = Some(cell.style);
            }
            let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
            ansi.push(ch);
            let adv = unicode_width::UnicodeWidthChar::width(ch)
                .unwrap_or(1)
                .max(1) as u16;
            let next = x.saturating_add(adv);
            pen = if next < size.width {
                Some((next, y))
            } else {
                None
            };
        });
        if last_style.is_some() {
            ansi.push_str("\x1b[0m");
        }

        match cursor {
            Some((x, y)) => {
                let _ = write!(ansi, "\x1b[{};{}H", y + 1, x + 1);
                if !self.cursor_visible {
                    ansi.push_str("\x1b[?25h");
                    self.cursor_visible = true;
                }
            }
            None => {
                if self.cursor_visible {
                    ansi.push_str("\x1b[?25l");
                    self.cursor_visible = false;
                }
            }
        }
    }

    /// Wait up to `timeout` for the next input event. `Ok(None)` on timeout.
    pub fn read_event(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        if let Some((w, h)) = platform::take_resize(self.size) {
            return Ok(Some(Event::Resize(w, h)));
        }
        if let Some(ev) = self.poll_parsed() {
            self.esc_seen_at = None;
            return Ok(Some(ev));
        }
        if let Some(ev) = self.take_timed_out_esc(Instant::now()) {
            return Ok(Some(ev));
        }

        let wait = self.stdin_wait(timeout, Instant::now());
        match platform::wait_stdin(wait)? {
            Wait::Timeout => {
                if let Some((w, h)) = platform::take_resize(self.size) {
                    return Ok(Some(Event::Resize(w, h)));
                }
                if let Some(ev) = self.take_timed_out_esc(Instant::now()) {
                    return Ok(Some(ev));
                }
                return Ok(None);
            }
            Wait::Ready => {}
        }

        let mut tmp = [0u8; 512];
        let n = platform::read_stdin(&mut tmp)?;
        if n > 0 {
            self.inbuf.extend_from_slice(&tmp[..n]);
            clamp_inbuf(&mut self.inbuf);
        }
        if let Some((w, h)) = platform::take_resize(self.size) {
            return Ok(Some(Event::Resize(w, h)));
        }
        if let Some(ev) = self.poll_parsed() {
            self.esc_seen_at = None;
            return Ok(Some(ev));
        }
        self.note_bare_esc(Instant::now());
        if let Some(ev) = self.take_timed_out_esc(Instant::now()) {
            return Ok(Some(ev));
        }
        Ok(None)
    }

    fn stdin_wait(&mut self, timeout: Duration, now: Instant) -> Duration {
        if self.inbuf.as_slice() != [0x1b] {
            return timeout;
        }
        let seen = *self.esc_seen_at.get_or_insert(now);
        let elapsed = now.saturating_duration_since(seen);
        if elapsed >= ESC_TIMEOUT {
            Duration::ZERO
        } else {
            timeout.min(ESC_TIMEOUT - elapsed)
        }
    }

    fn note_bare_esc(&mut self, now: Instant) {
        if self.inbuf.as_slice() == [0x1b] {
            if self.esc_seen_at.is_none() {
                self.esc_seen_at = Some(now);
            }
        } else {
            self.esc_seen_at = None;
        }
    }

    fn take_timed_out_esc(&mut self, now: Instant) -> Option<Event> {
        if self.inbuf.as_slice() != [0x1b] {
            if !self.inbuf.is_empty() {
                // Incomplete multi-byte escape (e.g. ESC [) — keep waiting.
                self.esc_seen_at = None;
            }
            return None;
        }
        let seen = *self.esc_seen_at.get_or_insert(now);
        if now.saturating_duration_since(seen) < ESC_TIMEOUT {
            return None;
        }
        self.inbuf.clear();
        self.esc_seen_at = None;
        Some(Event::Key(Key {
            code: KeyCode::Esc,
            mods: KeyMods::NONE,
        }))
    }

    fn poll_parsed(&mut self) -> Option<Event> {
        loop {
            let before = self.inbuf.len();
            if let Some(ev) = event::parse(&mut self.inbuf) {
                return Some(ev);
            }
            if self.inbuf.len() < before {
                continue;
            }
            return None;
        }
    }

    fn write_raw(&mut self, s: &str) -> io::Result<()> {
        self.out.write_all(s.as_bytes())?;
        self.out.flush()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.out.write_all(EXIT_SEQ);
        let _ = self.out.flush();
        platform::leave_raw(&self.platform);
    }
}

/// Ceiling on buffered input.
const MAX_INBUF: usize = 8 << 20;
/// Bytes always preserved at the end — a split `\x1b[201~` must survive.
const INBUF_TAIL: usize = 16;

/// Keep buffered input bounded.
///
/// A bracketed paste accumulates until its end marker arrives, and is then held
/// a second time as a decoded `String`. Without a ceiling a huge paste is two
/// copies of itself in memory before anything can reject it.
///
/// Past the cap the *middle* is dropped: the head keeps the start marker and the
/// text worth having, the tail keeps whatever closes the paste. So an oversized
/// paste arrives truncated rather than wedging the process.
fn clamp_inbuf(buf: &mut Vec<u8>) {
    if buf.len() <= MAX_INBUF {
        return;
    }
    let tail_start = buf.len() - INBUF_TAIL;
    buf.drain(MAX_INBUF - INBUF_TAIL..tail_start);
}

/// Build the SGR (Select Graphic Rendition) escape for a style, reset-prefixed
/// so no stale attribute leaks in from the previous cell.
fn sgr(style: Style) -> String {
    let mut s = String::from("\x1b[0m");
    if let Some(Color::Rgb(r, g, b)) = style.fg {
        s.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
    }
    if let Some(Color::Rgb(r, g, b)) = style.bg {
        s.push_str(&format!("\x1b[48;2;{r};{g};{b}m"));
    }
    if style.mods.contains(Modifier::BOLD) {
        s.push_str("\x1b[1m");
    }
    if style.mods.contains(Modifier::DIM) {
        s.push_str("\x1b[2m");
    }
    if style.mods.contains(Modifier::ITALIC) {
        s.push_str("\x1b[3m");
    }
    if style.mods.contains(Modifier::UNDERLINE) {
        s.push_str("\x1b[4m");
    }
    if style.mods.contains(Modifier::REVERSE) {
        s.push_str("\x1b[7m");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::event;

    /// A paste smaller than the cap is left completely alone.
    #[test]
    fn a_normal_paste_is_not_clamped() {
        let mut buf = b"\x1b[200~hello there\x1b[201~".to_vec();
        let before = buf.clone();
        clamp_inbuf(&mut buf);
        assert_eq!(buf, before);
    }

    /// An oversized paste stays bounded instead of growing without limit.
    #[test]
    fn an_oversized_paste_is_capped() {
        let mut buf = Vec::with_capacity(MAX_INBUF * 2);
        buf.extend_from_slice(b"\x1b[200~");
        buf.resize(MAX_INBUF * 2, b'x');
        clamp_inbuf(&mut buf);
        assert_eq!(buf.len(), MAX_INBUF);
    }

    /// Repeated reads must not let the buffer creep past the cap.
    #[test]
    fn repeated_reads_stay_bounded() {
        let mut buf = b"\x1b[200~".to_vec();
        for _ in 0..40 {
            buf.extend(std::iter::repeat_n(b'y', 512 * 1024));
            clamp_inbuf(&mut buf);
            assert!(buf.len() <= MAX_INBUF + 512 * 1024, "len={}", buf.len());
        }
        assert!(buf.len() <= MAX_INBUF + 512 * 1024);
    }

    /// The point of dropping the middle: the paste still closes, so the event
    /// arrives truncated rather than never arriving at all.
    #[test]
    fn a_capped_paste_still_parses_when_it_closes() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"\x1b[200~");
        buf.resize(MAX_INBUF * 2, b'z');
        clamp_inbuf(&mut buf);
        buf.extend_from_slice(b"\x1b[201~");
        clamp_inbuf(&mut buf);

        match event::parse(&mut buf) {
            Some(event::Event::Paste(text)) => {
                assert!(!text.is_empty(), "truncated, not empty");
                assert!(text.len() <= MAX_INBUF, "len={}", text.len());
            }
            other => panic!("expected a paste, got {other:?}"),
        }
    }
}
