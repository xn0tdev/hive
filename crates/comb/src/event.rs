//! Input events and the parser that turns raw terminal bytes into them.
//! Handles UTF-8 text, ctrl/alt chords, the common CSI keys, and SGR mouse
//! reporting (`ESC [ < b ; x ; y M|m`).

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct KeyMods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl KeyMods {
    pub const NONE: KeyMods = KeyMods {
        ctrl: false,
        alt: false,
        shift: false,
    };
    pub const CTRL: KeyMods = KeyMods {
        ctrl: true,
        alt: false,
        shift: false,
    };
    pub const ALT: KeyMods = KeyMods {
        ctrl: false,
        alt: true,
        shift: false,
    };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Tab,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyMods,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseKind {
    Down(MouseButton),
    Up,
    Drag,
    Moved,
    ScrollUp,
    ScrollDown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mouse {
    pub kind: MouseKind,
    /// Zero-based column and row on screen.
    pub col: u16,
    pub row: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    Key(Key),
    Mouse(Mouse),
    Resize(u16, u16),
}

fn key(code: KeyCode, mods: KeyMods) -> Event {
    Event::Key(Key { code, mods })
}

/// Parse one event from the front of `buf`, removing the bytes it consumed.
/// Returns `None` only when `buf` is empty or holds an incomplete sequence.
pub fn parse(buf: &mut Vec<u8>) -> Option<Event> {
    if buf.is_empty() {
        return None;
    }

    let b0 = buf[0];
    if b0 == 0x1b {
        return parse_escape(buf);
    }

    // Single-byte keys.
    let ev = match b0 {
        b'\r' | b'\n' => Some((key(KeyCode::Enter, KeyMods::NONE), 1)),
        0x7f | 0x08 => Some((key(KeyCode::Backspace, KeyMods::NONE), 1)),
        b'\t' => Some((key(KeyCode::Tab, KeyMods::NONE), 1)),
        0x01..=0x1a => {
            // Ctrl+letter (0x01 == ctrl-a). Tab/Enter handled above.
            let ch = (b0 - 1 + b'a') as char;
            Some((key(KeyCode::Char(ch), KeyMods::CTRL), 1))
        }
        _ => None,
    };
    if let Some((event, n)) = ev {
        buf.drain(0..n);
        return Some(event);
    }

    // UTF-8 text.
    let len = utf8_len(b0);
    if buf.len() < len {
        return None; // wait for the rest of the codepoint
    }
    let ch = std::str::from_utf8(&buf[0..len])
        .ok()
        .and_then(|s| s.chars().next());
    buf.drain(0..len);
    ch.map(|c| key(KeyCode::Char(c), KeyMods::NONE))
}

fn parse_escape(buf: &mut Vec<u8>) -> Option<Event> {
    // Lone ESC (nothing follows yet): treat as the Esc key.
    if buf.len() == 1 {
        buf.drain(0..1);
        return Some(key(KeyCode::Esc, KeyMods::NONE));
    }

    match buf[1] {
        b'[' => parse_csi(buf),
        b'O' => parse_ss3(buf),
        // ESC + printable == Alt+char.
        c if c >= 0x20 => {
            let len = utf8_len(c);
            if buf.len() < 1 + len {
                return None;
            }
            let ch = std::str::from_utf8(&buf[1..1 + len])
                .ok()
                .and_then(|s| s.chars().next());
            buf.drain(0..1 + len);
            ch.map(|c| key(KeyCode::Char(c), KeyMods::ALT))
        }
        _ => {
            buf.drain(0..1);
            Some(key(KeyCode::Esc, KeyMods::NONE))
        }
    }
}

fn parse_ss3(buf: &mut Vec<u8>) -> Option<Event> {
    if buf.len() < 3 {
        return None;
    }
    let code = match buf[2] {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        _ => {
            buf.drain(0..3);
            return None;
        }
    };
    buf.drain(0..3);
    Some(key(code, KeyMods::NONE))
}

fn parse_csi(buf: &mut Vec<u8>) -> Option<Event> {
    // SGR mouse: ESC [ < b ; x ; y (M|m)
    if buf.len() >= 3 && buf[2] == b'<' {
        return parse_sgr_mouse(buf);
    }

    // Find the final byte (0x40..=0x7e) that ends the sequence.
    let mut end = None;
    for (i, &b) in buf.iter().enumerate().skip(2) {
        if (0x40..=0x7e).contains(&b) {
            end = Some(i);
            break;
        }
    }
    let end = end?; // incomplete; wait for more bytes
    let final_byte = buf[end];
    let params = &buf[2..end];

    let code = match final_byte {
        b'A' => Some(KeyCode::Up),
        b'B' => Some(KeyCode::Down),
        b'C' => Some(KeyCode::Right),
        b'D' => Some(KeyCode::Left),
        b'H' => Some(KeyCode::Home),
        b'F' => Some(KeyCode::End),
        b'~' => match params.first() {
            Some(b'1') | Some(b'7') => Some(KeyCode::Home),
            Some(b'4') | Some(b'8') => Some(KeyCode::End),
            Some(b'3') => Some(KeyCode::Delete),
            Some(b'5') => Some(KeyCode::PageUp),
            Some(b'6') => Some(KeyCode::PageDown),
            _ => None,
        },
        _ => None,
    };

    buf.drain(0..=end);
    code.map(|c| key(c, KeyMods::NONE))
}

fn parse_sgr_mouse(buf: &mut Vec<u8>) -> Option<Event> {
    // Terminated by 'M' (press/motion) or 'm' (release).
    let mut end = None;
    for (i, &b) in buf.iter().enumerate().skip(3) {
        if b == b'M' || b == b'm' {
            end = Some(i);
            break;
        }
    }
    let end = end?;
    let released = buf[end] == b'm';
    let body = std::str::from_utf8(&buf[3..end]).ok()?;
    let mut it = body.split(';');
    let cb: u16 = it.next()?.parse().ok()?;
    let cx: u16 = it.next()?.parse().ok()?;
    let cy: u16 = it.next()?.parse().ok()?;
    buf.drain(0..=end);

    let col = cx.saturating_sub(1);
    let row = cy.saturating_sub(1);

    let kind = if cb & 0x40 != 0 {
        if cb & 0x1 == 0 {
            MouseKind::ScrollUp
        } else {
            MouseKind::ScrollDown
        }
    } else if released {
        MouseKind::Up
    } else if cb & 0x20 != 0 {
        // Motion: a held button drags; no button (bits == 3) is a hover move.
        if cb & 0x3 == 3 {
            MouseKind::Moved
        } else {
            MouseKind::Drag
        }
    } else {
        match cb & 0x3 {
            0 => MouseKind::Down(MouseButton::Left),
            1 => MouseKind::Down(MouseButton::Middle),
            2 => MouseKind::Down(MouseButton::Right),
            _ => MouseKind::Moved,
        }
    };

    Some(Event::Mouse(Mouse { kind, col, row }))
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else if b >> 3 == 0b11110 {
        4
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_char_and_ctrl() {
        let mut b = b"a".to_vec();
        assert_eq!(parse(&mut b), Some(key(KeyCode::Char('a'), KeyMods::NONE)));
        assert!(b.is_empty());

        let mut b = vec![0x03]; // ctrl-c
        assert_eq!(parse(&mut b), Some(key(KeyCode::Char('c'), KeyMods::CTRL)));
    }

    #[test]
    fn arrows_and_enter() {
        let mut b = b"\x1b[A".to_vec();
        assert_eq!(parse(&mut b), Some(key(KeyCode::Up, KeyMods::NONE)));
        let mut b = b"\r".to_vec();
        assert_eq!(parse(&mut b), Some(key(KeyCode::Enter, KeyMods::NONE)));
    }

    #[test]
    fn sgr_mouse_click_and_wheel() {
        let mut b = b"\x1b[<0;10;5M".to_vec();
        assert_eq!(
            parse(&mut b),
            Some(Event::Mouse(Mouse {
                kind: MouseKind::Down(MouseButton::Left),
                col: 9,
                row: 4,
            }))
        );
        let mut b = b"\x1b[<64;3;3M".to_vec();
        assert_eq!(
            parse(&mut b),
            Some(Event::Mouse(Mouse {
                kind: MouseKind::ScrollUp,
                col: 2,
                row: 2,
            }))
        );
    }

    #[test]
    fn incomplete_waits() {
        let mut b = b"\x1b[".to_vec();
        assert_eq!(parse(&mut b), None);
        assert_eq!(b, b"\x1b[");
    }

    #[test]
    fn utf8_multibyte() {
        let mut b = "→".as_bytes().to_vec();
        assert_eq!(parse(&mut b), Some(key(KeyCode::Char('→'), KeyMods::NONE)));
    }
}
