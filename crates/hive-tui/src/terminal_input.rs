use comb::{Key, KeyCode};

pub(crate) fn terminal_key_bytes(key: Key, application_cursor: bool) -> Vec<u8> {
    let modifier =
        1 + u8::from(key.mods.shift) + 2 * u8::from(key.mods.alt) + 4 * u8::from(key.mods.ctrl);
    let modified = modifier != 1;

    if let Some(final_byte) = match key.code {
        KeyCode::Up => Some(b'A'),
        KeyCode::Down => Some(b'B'),
        KeyCode::Right => Some(b'C'),
        KeyCode::Left => Some(b'D'),
        KeyCode::Home => Some(b'H'),
        KeyCode::End => Some(b'F'),
        _ => None,
    } {
        if modified {
            return format!("\x1b[1;{modifier}{}", char::from(final_byte)).into_bytes();
        }
        if application_cursor
            && matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right
            )
        {
            return vec![0x1b, b'O', final_byte];
        }
        return vec![0x1b, b'[', final_byte];
    }

    if let Some(number) = match key.code {
        KeyCode::Insert => Some(2),
        KeyCode::Delete => Some(3),
        KeyCode::PageUp => Some(5),
        KeyCode::PageDown => Some(6),
        _ => None,
    } {
        return if modified {
            format!("\x1b[{number};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{number}~").into_bytes()
        };
    }

    let mut bytes = Vec::new();
    if key.mods.alt {
        bytes.push(0x1b);
    }
    match key.code {
        KeyCode::Char(ch) => {
            if key.mods.ctrl {
                if let Some(control) = control_byte(ch) {
                    bytes.push(control);
                } else {
                    let mut encoded = [0; 4];
                    bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
                }
            } else {
                let mut encoded = [0; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
            }
        }
        KeyCode::Enter => bytes.push(b'\r'),
        KeyCode::Tab if key.mods.shift => bytes.extend_from_slice(b"\x1b[Z"),
        KeyCode::Tab => bytes.push(b'\t'),
        KeyCode::Esc => bytes.push(0x1b),
        KeyCode::Backspace => bytes.push(0x7f),
        KeyCode::Home
        | KeyCode::End
        | KeyCode::Insert
        | KeyCode::Delete
        | KeyCode::Left
        | KeyCode::Right
        | KeyCode::Up
        | KeyCode::Down
        | KeyCode::PageUp
        | KeyCode::PageDown => unreachable!("navigation keys returned above"),
    }
    bytes
}

pub(crate) fn terminal_paste_bytes(text: &str, bracketed_paste: bool) -> Vec<u8> {
    if bracketed_paste {
        let mut bytes = Vec::with_capacity(text.len() + 12);
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        text.as_bytes().to_vec()
    }
}

fn control_byte(ch: char) -> Option<u8> {
    match ch {
        'a'..='z' => Some(ch as u8 - b'a' + 1),
        'A'..='Z' => Some(ch as u8 - b'A' + 1),
        '@' | ' ' => Some(0),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use comb::{Key, KeyCode, KeyMods};

    fn key(code: KeyCode) -> Key {
        Key {
            code,
            mods: KeyMods::NONE,
        }
    }

    fn ctrl(ch: char) -> Key {
        Key {
            code: KeyCode::Char(ch),
            mods: KeyMods::CTRL,
        }
    }

    #[test]
    fn terminal_key_bytes_encode_text_control_and_cursor_modes() {
        assert_eq!(terminal_key_bytes(key(KeyCode::Enter), false), b"\r");
        assert_eq!(terminal_key_bytes(key(KeyCode::Up), false), b"\x1b[A");
        assert_eq!(terminal_key_bytes(key(KeyCode::Up), true), b"\x1bOA");
        assert_eq!(terminal_key_bytes(ctrl('c'), false), b"\x03");
        assert_eq!(
            terminal_key_bytes(
                Key {
                    code: KeyCode::Right,
                    mods: KeyMods {
                        ctrl: true,
                        alt: false,
                        shift: true,
                    },
                },
                false,
            ),
            b"\x1b[1;6C"
        );
    }

    #[test]
    fn terminal_paste_bytes_respect_bracketed_paste_mode() {
        assert_eq!(
            terminal_paste_bytes("hello", true),
            b"\x1b[200~hello\x1b[201~"
        );
        assert_eq!(terminal_paste_bytes("hello", false), b"hello");
    }
}
