//! The multiline input buffer with a char-indexed cursor.

#[derive(Default)]
pub struct InputState {
    pub value: String,
    /// Cursor position as a char index into `value`.
    pub cursor: usize,
}

impl InputState {
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

    pub fn newline(&mut self) {
        self.insert('\n');
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

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.char_count();
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.value)
    }

    /// (line, column) of the cursor for placing the terminal caret.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for (i, ch) in self.value.chars().enumerate() {
            if i == self.cursor {
                return (line, col);
            }
            if ch == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    pub fn line_count(&self) -> usize {
        self.value.chars().filter(|c| *c == '\n').count() + 1
    }
}
