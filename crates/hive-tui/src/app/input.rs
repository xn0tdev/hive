//! The multiline input buffer with a char-indexed cursor and soft-wrap.

/// How many visual text rows the input strip will grow to before it scrolls.
pub const MAX_VISIBLE_LINES: usize = 5;

/// Columns reserved for the `→ ` prompt / continuation indent.
pub const PROMPT_COLS: usize = 2;

#[derive(Default)]
pub struct InputState {
    pub value: String,
    /// Cursor position as a char index into `value`.
    pub cursor: usize,
    /// Text columns after the prompt — set each paint from the strip width.
    /// When unset (0), soft-wrap is disabled (one visual row per hard line).
    pub text_cols: usize,
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

    /// Effective wrap width; 0 / missing → no soft-wrap.
    fn wrap_width(&self) -> usize {
        self.text_cols
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

    /// Move to the start of the current hard line.
    pub fn home(&mut self) {
        let (line, _) = self.cursor_line_col();
        self.cursor = self.line_start(line);
    }

    /// Move to the end of the current hard line.
    pub fn end(&mut self) {
        let (line, _) = self.cursor_line_col();
        self.cursor = self.line_end(line);
    }

    /// Move up one visual row, preserving column when possible.
    /// Returns `false` if already on the first visual row.
    pub fn up(&mut self) -> bool {
        let w = self.wrap_width();
        let (row, col) = self.cursor_visual(w);
        if row == 0 {
            return false;
        }
        self.cursor = self.cursor_at_visual(row - 1, col, w);
        true
    }

    /// Move down one visual row, preserving column when possible.
    /// Returns `false` if already on the last visual row.
    pub fn down(&mut self) -> bool {
        let w = self.wrap_width();
        let (row, col) = self.cursor_visual(w);
        let last = self.visual_row_count(w).saturating_sub(1);
        if row >= last {
            return false;
        }
        self.cursor = self.cursor_at_visual(row + 1, col, w);
        true
    }

    fn line_start(&self, line: usize) -> usize {
        let mut cur = 0usize;
        let mut idx = 0usize;
        for ch in self.value.chars() {
            if cur == line {
                return idx;
            }
            idx += 1;
            if ch == '\n' {
                cur += 1;
            }
        }
        idx
    }

    fn line_end(&self, line: usize) -> usize {
        let mut cur = 0usize;
        let mut idx = 0usize;
        for ch in self.value.chars() {
            if cur == line && ch == '\n' {
                return idx;
            }
            idx += 1;
            if ch == '\n' {
                cur += 1;
            }
        }
        idx
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.value)
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// (hard line, column) of the cursor — column ignores soft-wrap.
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

    #[cfg(test)]
    fn line_count(&self) -> usize {
        self.value.chars().filter(|c| *c == '\n').count() + 1
    }

    /// Visual rows for one hard line of `len` characters at wrap width `w`.
    fn rows_for_len(len: usize, w: usize) -> usize {
        if w == 0 {
            return 1;
        }
        if len == 0 {
            1
        } else {
            len.div_ceil(w)
        }
    }

    /// Total visual rows (hard newlines + soft-wrap), uncapped.
    pub fn visual_row_count(&self, width: usize) -> usize {
        if self.value.is_empty() {
            return 1;
        }
        let w = width;
        self.value
            .split('\n')
            .map(|line| Self::rows_for_len(line.chars().count(), w))
            .sum()
    }

    /// How many text rows the strip should allocate (capped).
    pub fn visible_line_count(&self, width: usize) -> usize {
        self.visual_row_count(width).clamp(1, MAX_VISIBLE_LINES)
    }

    /// (visual row, column within that row) for the cursor.
    pub fn cursor_visual(&self, width: usize) -> (usize, usize) {
        let (hard, col) = self.cursor_line_col();
        let w = width;
        let mut row = 0usize;
        for (i, line) in self.value.split('\n').enumerate() {
            let len = line.chars().count();
            if i == hard {
                if w == 0 {
                    return (row, col);
                }
                return (row + col / w, col % w);
            }
            row += Self::rows_for_len(len, w);
        }
        if w == 0 {
            (row, col)
        } else {
            (row + col / w, col % w)
        }
    }

    /// Char index for a visual (row, col), clamping to the target row's end.
    fn cursor_at_visual(&self, target_row: usize, prefer_col: usize, width: usize) -> usize {
        let w = width;
        let mut row = 0usize;
        let mut char_base = 0usize; // char index at start of current hard line

        for line in self.value.split('\n') {
            let len = line.chars().count();
            let rows = Self::rows_for_len(len, w);
            if target_row < row + rows {
                let within = target_row - row;
                if w == 0 {
                    return (char_base + prefer_col).min(char_base + len);
                }
                let start_col = within * w;
                let end_col = (start_col + w).min(len);
                let col = start_col + prefer_col.min(end_col.saturating_sub(start_col));
                return char_base + col.min(len);
            }
            row += rows;
            char_base += len + 1; // +1 for the '\n' (except we overshoot after last)
        }

        // target past end — clamp to document end
        self.char_count()
    }

    /// Scroll offset so the cursor visual row stays inside the visible window.
    pub fn view_scroll(&self, visible: usize, width: usize) -> usize {
        let visible = visible.max(1);
        let (row, _) = self.cursor_visual(width);
        row.saturating_sub(visible.saturating_sub(1))
    }

    /// Soft-wrapped display rows: each entry is `(is_first_hard_line, text)`.
    pub fn wrapped_rows(&self, width: usize) -> Vec<(bool, String)> {
        let w = width;
        let mut out = Vec::new();
        if self.value.is_empty() {
            return out;
        }
        for (hi, line) in self.value.split('\n').enumerate() {
            let chars: Vec<char> = line.chars().collect();
            if chars.is_empty() {
                out.push((hi == 0, String::new()));
                continue;
            }
            if w == 0 {
                out.push((hi == 0, chars.iter().collect()));
                continue;
            }
            for (ci, chunk) in chars.chunks(w).enumerate() {
                out.push((hi == 0 && ci == 0, chunk.iter().collect()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_down_preserve_column() {
        let mut i = InputState {
            value: "hello\nworld\n!".into(),
            cursor: 0,
            text_cols: 0, // hard-line navigation when no wrap width
        };
        i.end(); // end of "hello"
        assert_eq!(i.cursor, 5);
        assert!(i.down());
        assert_eq!(i.cursor_line_col(), (1, 5)); // end of "world"
        assert!(i.down());
        // "!" is shorter — land at end of that line.
        assert_eq!(i.cursor_line_col(), (2, 1));
        assert!(!i.down());
        assert!(i.up());
        assert_eq!(i.cursor_line_col(), (1, 1));
    }

    #[test]
    fn view_scroll_keeps_cursor_visible() {
        let mut i = InputState::default();
        for _ in 0..8 {
            i.newline();
        }
        assert_eq!(i.line_count(), 9);
        assert_eq!(i.visible_line_count(80), MAX_VISIBLE_LINES);
        assert_eq!(i.view_scroll(MAX_VISIBLE_LINES, 80), 4); // lines 4..8
    }

    #[test]
    fn soft_wrap_grows_visual_rows() {
        let mut i = InputState::default();
        i.text_cols = 10;
        for _ in 0..25 {
            i.insert('x');
        }
        // 25 chars @ width 10 → 3 visual rows
        assert_eq!(i.visual_row_count(10), 3);
        assert_eq!(i.visible_line_count(10), 3);
        assert_eq!(i.cursor_visual(10), (2, 5)); // row 2, col 5
    }

    #[test]
    fn soft_wrap_caps_visible_and_scrolls() {
        let mut i = InputState::default();
        i.text_cols = 4;
        for _ in 0..30 {
            i.insert('a');
        }
        // 30 / 4 = 8 visual rows, capped to 5
        assert_eq!(i.visual_row_count(4), 8);
        assert_eq!(i.visible_line_count(4), MAX_VISIBLE_LINES);
        assert_eq!(i.view_scroll(MAX_VISIBLE_LINES, 4), 3);
    }

    #[test]
    fn soft_wrap_up_down() {
        let mut i = InputState::default();
        i.text_cols = 5;
        for _ in 0..12 {
            i.insert('x');
        }
        // cursor at end: visual (2, 2)
        assert_eq!(i.cursor_visual(5), (2, 2));
        assert!(i.up());
        assert_eq!(i.cursor_visual(5), (1, 2));
        assert!(i.up());
        assert_eq!(i.cursor_visual(5), (0, 2));
        assert!(!i.up());
    }

    #[test]
    fn wrapped_rows_splits_long_line() {
        let mut i = InputState::default();
        i.value = "abcdefghij".into();
        let rows = i.wrapped_rows(4);
        assert_eq!(
            rows,
            vec![
                (true, "abcd".into()),
                (false, "efgh".into()),
                (false, "ij".into()),
            ]
        );
    }
}
