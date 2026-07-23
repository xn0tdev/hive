//! termios + poll + SIGWINCH backend.

use std::io;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::core::geom::Size;

use super::Wait;

const STDIN: i32 = libc::STDIN_FILENO;
const STDOUT: i32 = libc::STDOUT_FILENO;

static WINCHED: AtomicBool = AtomicBool::new(false);
static SAVED_TERMIOS: Mutex<Option<libc::termios>> = Mutex::new(None);

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

pub struct PlatformState {
    orig: libc::termios,
}

pub fn enter_raw() -> io::Result<PlatformState> {
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
    install_sigwinch();
    WINCHED.store(false, Ordering::Relaxed);
    Ok(PlatformState { orig })
}

pub fn leave_raw(state: &PlatformState) {
    unsafe {
        libc::tcsetattr(STDIN, libc::TCSANOW, &state.orig);
    }
}

/// Best-effort restore for panic hooks (no Terminal in hand).
pub fn restore_console() {
    if let Some(t) = *SAVED_TERMIOS.lock().unwrap() {
        unsafe {
            libc::tcsetattr(STDIN, libc::TCSANOW, &t);
        }
    }
}

pub fn query_size() -> Size {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(STDOUT, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
            Size::new(ws.ws_col, ws.ws_row)
        } else {
            Size::new(80, 24)
        }
    }
}

pub fn take_resize(last: Size) -> Option<(u16, u16)> {
    // Consume the SIGWINCH flag so it doesn't linger; the decision is based on
    // the actual queried size, not the flag.
    WINCHED.swap(false, Ordering::Relaxed);
    let now = query_size();
    if now.width > 0 && now != last {
        Some((now.width, now.height))
    } else {
        None
    }
}

pub fn clear_resize_flag() {
    WINCHED.store(false, Ordering::Relaxed);
}

pub fn wait_stdin(timeout: Duration) -> io::Result<Wait> {
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    let mut pfd = libc::pollfd {
        fd: STDIN,
        events: libc::POLLIN,
        revents: 0,
    };
    let r = unsafe { libc::poll(&mut pfd, 1, ms) };
    if r < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(Wait::Timeout);
        }
        return Err(err);
    }
    if r == 0 {
        Ok(Wait::Timeout)
    } else {
        Ok(Wait::Ready)
    }
}

pub fn read_stdin(buf: &mut [u8]) -> io::Result<usize> {
    let n = unsafe { libc::read(STDIN, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
    if n < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(0);
        }
        return Err(err);
    }
    Ok(n as usize)
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
