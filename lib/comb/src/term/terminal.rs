//! The native terminal backend: raw mode via termios, the alternate screen,
//! mouse reporting, and a diffed draw loop that emits only the ANSI needed to
//! turn last frame into this one.

use std::io::{self, Write};
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::core::buffer::{Buffer, WIDE_CONT};
use crate::core::geom::{Rect, Size};
use crate::core::style::{Color, Modifier, Style};
use crate::draw::surface::Compositor;
use crate::term::event::{self, Event};

const STDIN: i32 = libc::STDIN_FILENO;
const STDOUT: i32 = libc::STDOUT_FILENO;

/// Set by the SIGWINCH handler; drained into [`Event::Resize`].
static WINCHED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_sigwinch(_: libc::c_int) {
    WINCHED.store(true, Ordering::Relaxed);
}

fn install_sigwinch() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_sigwinch as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut sa.sa_mask);
        // No SA_RESTART: poll must wake on resize so idle UIs redraw promptly.
        sa.sa_flags = 0;
        libc::sigaction(libc::SIGWINCH, &sa, std::ptr::null_mut());
    }
}

/// When the tty size no longer matches `last`, return the new `(cols, rows)`.
/// Consumes a pending SIGWINCH flag. Does not update stored size — [`draw`]
/// owns that (and the full clear).
fn take_resize(last: Size) -> Option<(u16, u16)> {
    let winched = WINCHED.swap(false, Ordering::Relaxed);
    let now = query_size();
    if now.width > 0 && now != last {
        Some((now.width, now.height))
    } else if winched {
        // Signal fired but ioctl still agrees with `last` — ignore.
        None
    } else {
        None
    }
}

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

/// The original termios, saved when a [`Terminal`] enters raw mode, so a panic
/// hook can undo it via [`restore`] even without the `Terminal` in hand.
static SAVED_TERMIOS: Mutex<Option<libc::termios>> = Mutex::new(None);

/// Best-effort terminal reset for panic hooks: leave the alternate screen, show
/// the cursor, stop mouse reporting, and undo raw mode if we saved the original
/// settings. Safe to call with no active `Terminal`.
pub fn restore() {
    let mut out = io::stdout();
    let _ = out.write_all(
        b"\x1b[<u\x1b[>4;0m\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?25h\x1b[?1049l",
    );
    let _ = out.flush();
    if let Some(t) = *SAVED_TERMIOS.lock().unwrap() {
        unsafe {
            libc::tcsetattr(STDIN, libc::TCSANOW, &t);
        }
    }
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
    orig: libc::termios,
    front: Buffer,
    size: Size,
    inbuf: Vec<u8>,
    cursor_visible: bool,
    out: io::Stdout,
}

impl Terminal {
    /// Enter raw mode + the alternate screen and start mouse reporting.
    pub fn new() -> io::Result<Self> {
        unsafe {
            if libc::isatty(STDIN) != 1 || libc::isatty(STDOUT) != 1 {
                return Err(io::Error::other("comb requires an interactive terminal"));
            }
        }
        let orig = get_termios()?;
        let mut raw = orig;
        unsafe { libc::cfmakeraw(&mut raw) };
        set_termios(&raw)?;
        *SAVED_TERMIOS.lock().unwrap() = Some(orig);

        let size = query_size();
        let mut term = Terminal {
            orig,
            front: Buffer::blank(size),
            size,
            inbuf: Vec::new(),
            cursor_visible: true,
            out: io::stdout(),
        };

        install_sigwinch();
        WINCHED.store(false, Ordering::Relaxed);

        // alt screen · SGR mouse · modifyOtherKeys level 2 · kitty keyboard
        // disambiguate only (flag 1). Flag 8 (report all keys) + event types
        // made every letter a CSI-u press/release pair → doubled input, and
        // broke UTF-8 Cyrillic. Bracketed paste · clear · hide cursor.
        term.write_raw(
            "\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[>4;2m\x1b[>1u\x1b[?2004h\x1b[2J\x1b[H\x1b[?25l",
        )?;
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

    /// Build a frame, then flush the minimal diff to the terminal.
    pub fn draw<F: FnOnce(&mut Frame)>(&mut self, f: F) -> io::Result<()> {
        // Adapt to a resized window: reset our record of the screen and clear.
        // Always re-query — SIGWINCH may have been coalesced while drawing.
        let size = query_size();
        if size != self.size {
            self.size = size;
            // Force a full repaint: blank front makes every cell "changed".
            self.front = Buffer::blank(size);
            // Erase alt-screen contents + home the cursor (2J) and scrollback
            // ghosts some terminals keep after a grow (3J).
            self.write_raw("\x1b[H\x1b[2J\x1b[3J")?;
            WINCHED.store(false, Ordering::Relaxed);
        }

        let mut back = Buffer::blank(self.size);
        let cursor = {
            let mut frame = Frame {
                root: &mut back,
                comp: Compositor::new(),
                cursor: None,
                size: self.size,
            };
            f(&mut frame);
            frame.finish()
        };

        let out = self.render_diff(&back, cursor);
        self.write_raw(&out)?;
        self.front = back;
        Ok(())
    }

    fn render_diff(&mut self, back: &Buffer, cursor: Option<(u16, u16)>) -> String {
        let mut s = String::new();
        let mut last_style: Option<Style> = None;
        let mut pen: Option<(u16, u16)> = None; // where the terminal cursor sits

        for (x, y, cell) in back.diff(&self.front) {
            // Second cell of a double-width glyph — terminal already advanced.
            if cell.ch == WIDE_CONT {
                pen = if x + 1 < self.size.width {
                    Some((x + 1, y))
                } else {
                    None
                };
                continue;
            }
            if pen != Some((x, y)) {
                s.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
            }
            if last_style != Some(cell.style) {
                s.push_str(&sgr(cell.style));
                last_style = Some(cell.style);
            }
            let ch = if cell.ch == '\0' { ' ' } else { cell.ch };
            s.push(ch);
            // Wide glyphs advance the hardware cursor by two columns.
            let adv = unicode_width::UnicodeWidthChar::width(ch)
                .unwrap_or(1)
                .max(1) as u16;
            let next = x.saturating_add(adv);
            pen = if next < self.size.width {
                Some((next, y))
            } else {
                None // wrapped / EOL: force a fresh move next time
            };
        }
        if last_style.is_some() {
            s.push_str("\x1b[0m");
        }

        match cursor {
            Some((x, y)) => {
                s.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
                if !self.cursor_visible {
                    s.push_str("\x1b[?25h");
                    self.cursor_visible = true;
                }
            }
            None => {
                if self.cursor_visible {
                    s.push_str("\x1b[?25l");
                    self.cursor_visible = false;
                }
            }
        }
        s
    }

    /// Wait up to `timeout` for the next input event. `Ok(None)` on timeout.
    pub fn read_event(&mut self, timeout: Duration) -> io::Result<Option<Event>> {
        // Never starve a pending resize behind buffered keys.
        if let Some((w, h)) = take_resize(self.size) {
            return Ok(Some(Event::Resize(w, h)));
        }
        if let Some(ev) = self.poll_parsed() {
            return Ok(Some(ev));
        }

        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let mut pfd = libc::pollfd {
            fd: STDIN,
            events: libc::POLLIN,
            revents: 0,
        };
        let r = unsafe { libc::poll(&mut pfd, 1, ms) };
        if r < 0 {
            let err = io::Error::last_os_error();
            // SIGWINCH interrupts poll — surface it as Resize, not a hard error.
            if err.kind() == io::ErrorKind::Interrupted {
                if let Some((w, h)) = take_resize(self.size) {
                    return Ok(Some(Event::Resize(w, h)));
                }
                return Ok(None);
            }
            return Err(err);
        }
        if r == 0 {
            // Idle tick: still notice resizes if SIGWINCH was missed.
            if let Some((w, h)) = take_resize(self.size) {
                return Ok(Some(Event::Resize(w, h)));
            }
            return Ok(None);
        }

        let mut tmp = [0u8; 512];
        let n = unsafe { libc::read(STDIN, tmp.as_mut_ptr() as *mut libc::c_void, tmp.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                if let Some((w, h)) = take_resize(self.size) {
                    return Ok(Some(Event::Resize(w, h)));
                }
                return Ok(None);
            }
            return Err(err);
        }
        if n > 0 {
            self.inbuf.extend_from_slice(&tmp[..n as usize]);
        }
        // Resize wins over a key that arrived in the same wake — layout first.
        if let Some((w, h)) = take_resize(self.size) {
            return Ok(Some(Event::Resize(w, h)));
        }
        Ok(self.poll_parsed())
    }

    /// Parse buffered input, skipping no-op events (kitty key-release, etc.).
    fn poll_parsed(&mut self) -> Option<Event> {
        loop {
            let before = self.inbuf.len();
            if let Some(ev) = event::parse(&mut self.inbuf) {
                return Some(ev);
            }
            // `None` + consumed bytes → ignored event; keep going.
            if self.inbuf.len() < before {
                continue;
            }
            // Incomplete sequence or empty buffer.
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
        // pop kitty keyboard · disable modifyOtherKeys · mouse · bracketed paste
        // · show cursor · leave alt
        let _ = self.out.write_all(
            b"\x1b[<u\x1b[>4;0m\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004l\x1b[?25h\x1b[?1049l",
        );
        let _ = self.out.flush();
        unsafe {
            libc::tcsetattr(STDIN, libc::TCSANOW, &self.orig);
        }
    }
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

fn get_termios() -> io::Result<libc::termios> {
    unsafe {
        let mut t = MaybeUninit::<libc::termios>::uninit();
        if libc::tcgetattr(STDIN, t.as_mut_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(t.assume_init())
    }
}

fn set_termios(t: &libc::termios) -> io::Result<()> {
    if unsafe { libc::tcsetattr(STDIN, libc::TCSANOW, t) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn query_size() -> Size {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(STDOUT, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            Size::new(ws.ws_col, ws.ws_row)
        } else {
            Size::new(80, 24)
        }
    }
}
