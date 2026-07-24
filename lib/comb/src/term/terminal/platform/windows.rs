//! Win32 Console VT backend (Windows 10+).

use std::io;
use std::sync::Mutex;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    FALSE, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::ReadFile;
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle, SetConsoleMode,
    CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT,
    ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WRAP_AT_EOL_OUTPUT,
    STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

use crate::core::geom::Size;

use super::Wait;

struct SavedModes {
    stdin: u32,
    stdout: u32,
}

static SAVED: Mutex<Option<SavedModes>> = Mutex::new(None);

pub struct PlatformState {
    stdin: HANDLE,
    stdout: HANDLE,
    orig_in: u32,
    orig_out: u32,
}

fn stdin_handle() -> io::Result<HANDLE> {
    let h = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(h)
}

fn stdout_handle() -> io::Result<HANDLE> {
    let h = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(h)
}

fn get_mode(h: HANDLE) -> io::Result<u32> {
    let mut mode = 0u32;
    if unsafe { GetConsoleMode(h, &mut mode) } == FALSE {
        return Err(io::Error::other(
            "comb requires an interactive Windows console (Windows Terminal / conhost with VT)",
        ));
    }
    Ok(mode)
}

pub fn enter_raw() -> io::Result<PlatformState> {
    let stdin = stdin_handle()?;
    let stdout = stdout_handle()?;
    let orig_in = get_mode(stdin)?;
    let orig_out = get_mode(stdout)?;

    // Raw-ish input: VT sequences, no line/echo processing.
    let mut in_mode = orig_in;
    in_mode &= !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT);
    in_mode |= ENABLE_VIRTUAL_TERMINAL_INPUT;

    // VT processing on output (ANSI paint path).
    let mut out_mode = orig_out;
    out_mode |= ENABLE_VIRTUAL_TERMINAL_PROCESSING | ENABLE_WRAP_AT_EOL_OUTPUT;

    if unsafe { SetConsoleMode(stdin, in_mode) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    if unsafe { SetConsoleMode(stdout, out_mode) } == FALSE {
        let _ = unsafe { SetConsoleMode(stdin, orig_in) };
        return Err(io::Error::last_os_error());
    }

    *SAVED.lock().unwrap() = Some(SavedModes {
        stdin: orig_in,
        stdout: orig_out,
    });

    Ok(PlatformState {
        stdin,
        stdout,
        orig_in,
        orig_out,
    })
}

pub fn leave_raw(state: &PlatformState) {
    unsafe {
        let _ = SetConsoleMode(state.stdin, state.orig_in);
        let _ = SetConsoleMode(state.stdout, state.orig_out);
    }
}

pub fn restore_console() {
    if let Some(saved) = SAVED.lock().unwrap().take() {
        if let (Ok(stdin), Ok(stdout)) = (stdin_handle(), stdout_handle()) {
            unsafe {
                let _ = SetConsoleMode(stdin, saved.stdin);
                let _ = SetConsoleMode(stdout, saved.stdout);
            }
        }
    }
}

pub fn query_size() -> Size {
    let Ok(stdout) = stdout_handle() else {
        return Size::new(80, 24);
    };
    unsafe {
        let mut info = std::mem::zeroed::<CONSOLE_SCREEN_BUFFER_INFO>();
        if GetConsoleScreenBufferInfo(stdout, &mut info) == FALSE {
            return Size::new(80, 24);
        }
        let w = (info.srWindow.Right - info.srWindow.Left + 1) as u16;
        let h = (info.srWindow.Bottom - info.srWindow.Top + 1) as u16;
        if w == 0 || h == 0 {
            Size::new(80, 24)
        } else {
            Size::new(w, h)
        }
    }
}

/// No SIGWINCH on Windows — compare console window size each poll.
pub fn take_resize(last: Size) -> Option<(u16, u16)> {
    let now = query_size();
    if now.width > 0 && now != last {
        Some((now.width, now.height))
    } else {
        None
    }
}

pub fn clear_resize_flag() {}

pub fn wait_stdin(timeout: Duration) -> io::Result<Wait> {
    let stdin = stdin_handle()?;
    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    let r = unsafe { WaitForSingleObject(stdin, ms) };
    match r {
        WAIT_OBJECT_0 => Ok(Wait::Ready),
        WAIT_TIMEOUT => Ok(Wait::Timeout),
        WAIT_FAILED => Err(io::Error::last_os_error()),
        _ => Ok(Wait::Timeout),
    }
}

pub fn read_stdin(buf: &mut [u8]) -> io::Result<usize> {
    let stdin = stdin_handle()?;
    let mut read = 0u32;
    let ok = unsafe {
        ReadFile(
            stdin,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            &mut read,
            std::ptr::null_mut(),
        )
    };
    if ok == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(read as usize)
}
