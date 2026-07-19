//! Single-line text field matching the hive input strip: borderless tinted
//! band, `→` prompt, placeholder, hardware cursor via returned position.

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::style::{Modifier, Style};
use crate::widgets::Palette;

const PROMPT: &str = "→ ";
const PROMPT_W: usize = 2;

#[derive(Default)]
pub struct TextInput {
    pub value: String,
    pub cursor: usize,
    pub placeholder: String,
}

impl TextInput {
    pub fn with_placeholder(placeholder: impl Into<String>) -> Self {
        TextInput {
            placeholder: placeholder.into(),
            ..Default::default()
        }
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.value.len())
    }

    fn char_count(&self) -> usize {
        self.value.chars().count()
    }

    pub fn insert(&mut self, c: char) {
        if c == '\0' || (c.is_control() && c != '\t') {
            return;
        }
        let b = self.byte_at(self.cursor);
        self.value.insert(b, c);
        self.cursor += 1;
    }

    /// Insert clipboard / bracketed-paste text at the cursor.
    /// Newlines become spaces (single-line field). Other C0 controls are dropped.
    pub fn insert_str(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '\n' | '\r' => self.insert(' '),
                c if c.is_control() && c != '\t' => {}
                c => self.insert(c),
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = self.byte_at(self.cursor - 1);
        let end = self.byte_at(self.cursor);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cursor < self.char_count() {
            self.cursor += 1;
        }
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.value)
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Draw the strip. Returns `(x, y)` for the hardware cursor.
    pub fn render(&self, buf: &mut Buffer, area: Rect, pal: &Palette) -> Option<(u16, u16)> {
        if area.height < 3 || area.width < 4 {
            return None;
        }

        let strip = pal.panel;
        buf.paint(area, strip);

        let text_y = area.y + 1;
        let row = Rect::new(area.x + 1, text_y, area.width.saturating_sub(2), 1);
        if row.is_empty() {
            return None;
        }

        buf.paint(row, strip);
        let on = |st: Style| st.patch(strip);
        let prompt = on(pal.accent);
        let body = on(pal.normal);
        let ghost = on(pal.normal).add(Modifier::DIM | Modifier::ITALIC);

        buf.set_str(row.x, text_y, PROMPT, prompt);

        let text_x = row.x + PROMPT_W as u16;
        let avail = row.width.saturating_sub(PROMPT_W as u16) as usize;

        if self.value.is_empty() {
            buf.set_str(text_x, text_y, &self.placeholder, ghost);
            pad_row(
                buf,
                row,
                text_x + self.placeholder.chars().count() as u16,
                strip,
            );
            return Some((text_x, text_y));
        }

        let chars: Vec<char> = self.value.chars().collect();
        let cursor = self.cursor.min(chars.len());
        let mut start = 0usize;
        if cursor > avail.saturating_sub(1) {
            start = cursor.saturating_sub(avail.saturating_sub(1));
        }
        let visible: String = chars.iter().skip(start).take(avail).collect();
        buf.set_str(text_x, text_y, &visible, body);
        pad_row(buf, row, text_x + visible.chars().count() as u16, strip);

        let cx = text_x + (cursor.saturating_sub(start)) as u16;
        Some((cx.min(row.right().saturating_sub(1)), text_y))
    }
}

fn pad_row(buf: &mut Buffer, row: Rect, from_x: u16, bg: Style) {
    for x in from_x..row.right() {
        buf.set(x, row.y, ' ', bg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geom::Size;
    use crate::core::style::Color;
    use crate::term::terminal::render;

    fn pal() -> Palette {
        let bg = Style::new().bg(Color::rgb(0x26, 0x26, 0x26));
        Palette {
            panel: bg,
            border: Style::new().fg(Color::rgb(0x6a, 0x6a, 0x6a)),
            normal: Style::new().fg(Color::rgb(0xd4, 0xd4, 0xd4)),
            accent: Style::new().fg(Color::rgb(0xe6, 0xe6, 0xe6)),
            selected: Style::new()
                .fg(Color::rgb(0x12, 0x12, 0x12))
                .bg(Color::rgb(0xd8, 0xd8, 0xd8)),
            track: Style::new(),
            thumb: Style::new(),
        }
    }

    #[test]
    fn text_row_keeps_strip_background() {
        let input = TextInput::with_placeholder("type here");
        let buf = render(Size::new(40, 5), |f| {
            let _ = input.render(f.buffer(), Rect::new(1, 1, 36, 3), &pal());
        });
        for x in 2..36 {
            let cell = buf.get(x, 2).unwrap();
            assert_eq!(cell.style.bg, Some(Color::rgb(0x26, 0x26, 0x26)), "x={x}");
        }
    }

    #[test]
    fn returns_cursor_on_text_row() {
        let mut input = TextInput::with_placeholder("x");
        input.insert('a');
        let mut cursor = None;
        let _ = render(Size::new(30, 5), |f| {
            cursor = input.render(f.buffer(), Rect::new(1, 1, 26, 3), &pal());
        });
        // area(1,1) → row.x=2 → text after "→ " at x=4 → caret after 'a' at x=5
        assert_eq!(cursor, Some((5, 2)));
    }
}
