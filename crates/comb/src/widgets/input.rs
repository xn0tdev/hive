//! A single-line text field with a char-indexed cursor.

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::style::Modifier;
use crate::draw::border::Border;
use crate::widgets::Palette;

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
        let b = self.byte_at(self.cursor);
        self.value.insert(b, c);
        self.cursor += 1;
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

    /// Draw the field and return the screen position of the caret, if any.
    pub fn render(
        &self,
        buf: &mut Buffer,
        area: Rect,
        pal: &Palette,
        show_caret: bool,
    ) -> Option<(u16, u16)> {
        if area.height < 3 || area.width < 4 {
            return None;
        }
        buf.block(area, Border::Rounded, pal.border, Some(pal.panel));
        let inner = Rect::new(area.x + 2, area.y + 1, area.width.saturating_sub(4), 1);
        let prompt = pal.selected;
        let body = pal.normal;
        let ghost = body.add(Modifier::DIM | Modifier::ITALIC);

        if self.value.is_empty() {
            buf.set_str(inner.x, inner.y, "→ ", prompt);
            buf.set_str(inner.x + 2, inner.y, &self.placeholder, ghost);
            return show_caret.then_some((inner.x + 2, inner.y));
        }

        buf.set_str(inner.x, inner.y, "→ ", prompt);
        let text_x = inner.x + 2;
        let avail = inner.width.saturating_sub(2) as usize;
        let chars: Vec<char> = self.value.chars().collect();
        let cursor = self.cursor.min(chars.len());
        let mut start = 0usize;
        if cursor > avail.saturating_sub(1) {
            start = cursor.saturating_sub(avail.saturating_sub(1));
        }
        let visible: String = chars.iter().skip(start).take(avail).collect();
        buf.set_str(text_x, inner.y, &visible, body);

        if show_caret {
            let caret_col = text_x + (cursor.saturating_sub(start)) as u16;
            return Some((caret_col.min(inner.right() - 1), inner.y));
        }
        None
    }
}
