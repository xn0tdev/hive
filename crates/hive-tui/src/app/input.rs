//! The multiline input buffer with a char-indexed cursor and soft-wrap.
//! Wrapping and caret columns use Unicode display width (not raw char count),
//! so CJK / emoji lines stay aligned with the terminal.

use unicode_width::UnicodeWidthChar;

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
        // Ignore NUL / most C0 controls except newline/tab handled elsewhere.
        if c == '\0' || (c.is_control() && c != '\n' && c != '\t') {
            return;
        }
        let b = self.byte_at(self.cursor);
        self.value.insert(b, c);
        self.cursor += 1;
    }

    /// Insert a paste / multi-char chunk at the cursor. Normalizes `\r\n` / `\r`
    /// to `\n` and strips other C0 controls.
    pub fn insert_str(&mut self, text: &str) {
        let normalized = normalize_paste(text);
        if normalized.is_empty() {
            return;
        }
        let b = self.byte_at(self.cursor);
        let n = normalized.chars().count();
        self.value.insert_str(b, &normalized);
        self.cursor += n;
    }

    /// Replace the char range `[start, end)` with `text` and place the cursor after it.
    pub fn replace_chars(&mut self, start: usize, end: usize, text: &str) {
        let start = start.min(self.char_count());
        let end = end.min(self.char_count()).max(start);
        let b0 = self.byte_at(start);
        let b1 = self.byte_at(end);
        self.value.replace_range(b0..b1, text);
        self.cursor = start + text.chars().count();
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

    /// Total visual rows (hard newlines + soft-wrap), uncapped.
    pub fn visual_row_count(&self, width: usize) -> usize {
        if self.value.is_empty() {
            return 1;
        }
        let w = width;
        self.value
            .split('\n')
            .map(|line| wrap_hard_line(line, w).len())
            .sum()
    }

    /// How many text rows the strip should allocate (capped).
    pub fn visible_line_count(&self, width: usize) -> usize {
        self.visual_row_count(width).clamp(1, MAX_VISIBLE_LINES)
    }

    /// (visual row, display column within that row) for the cursor.
    pub fn cursor_visual(&self, width: usize) -> (usize, usize) {
        let w = width;
        if self.value.is_empty() {
            return (0, 0);
        }
        let mut row = 0usize;
        let mut char_offset = 0usize;
        for line in self.value.split('\n') {
            let line_chars = line.chars().count();
            let chunks = wrap_hard_line(line, w);
            if self.cursor <= char_offset + line_chars {
                let cursor_within = self.cursor - char_offset;
                let last_i = chunks.len() - 1;
                for (ci, (chunk_text, chunk_start)) in chunks.iter().enumerate() {
                    let chunk_chars = chunk_text.chars().count();
                    let in_chunk = if ci == last_i {
                        cursor_within <= *chunk_start + chunk_chars
                    } else {
                        cursor_within < *chunk_start + chunk_chars
                    };
                    if in_chunk {
                        let into = cursor_within.saturating_sub(*chunk_start);
                        let col: usize = chunk_text
                            .chars()
                            .take(into)
                            .map(glyph_width)
                            .sum();
                        return (row + ci, col);
                    }
                }
                let col = display_width(&chunks[last_i].0);
                return (row + last_i, col);
            }
            row += chunks.len();
            char_offset += line_chars + 1; // +1 for '\n'
        }
        (row, 0)
    }

    /// Char index for a visual (row, prefer_display_col).
    fn cursor_at_visual(&self, target_row: usize, prefer_col: usize, width: usize) -> usize {
        let w = width;
        let mut row = 0usize;
        let mut char_i = 0usize;

        for line in self.value.split('\n') {
            let chunks = wrap_hard_line(line, w);
            let n = chunks.len().max(1);
            if target_row < row + n {
                let within = target_row - row;
                let chunk_text = chunks
                    .get(within)
                    .map(|(t, _)| t.as_str())
                    .unwrap_or("");
                let col = prefer_col.min(display_width(chunk_text));
                return char_i + char_index_at_display_col(line, within, col, w);
            }
            row += n;
            char_i += line.chars().count() + 1; // +1 for '\n'
        }

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
            let chunks = wrap_hard_line(line, w);
            for (ci, (chunk, _)) in chunks.into_iter().enumerate() {
                out.push((hi == 0 && ci == 0, chunk));
            }
        }
        out
    }
}

fn glyph_width(ch: char) -> usize {
    match UnicodeWidthChar::width(ch) {
        Some(0) | None => 1, // keep caret math stable for combining marks
        Some(w) => w,
    }
}

fn display_width(s: &str) -> usize {
    s.chars().map(glyph_width).sum()
}

/// Normalize clipboard paste: CRLF/CR → LF, drop other C0 controls (keep tab/LF).
fn normalize_paste(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Char-wrap one hard line into chunks of at most `w` display columns.
/// Returns `(text, char_offset_in_line)` for each chunk. A space that would
/// land on a new row stays on the current row (invisible trailing space) —
/// only a visible character triggers the wrap. This prevents the cursor
/// from jumping to a new line that looks empty.
fn wrap_hard_line(line: &str, w: usize) -> Vec<(String, usize)> {
    if line.is_empty() {
        return vec![(String::new(), 0)];
    }
    if w == 0 {
        return vec![(line.to_string(), 0)];
    }

    let chars: Vec<char> = line.chars().collect();
    let mut rows = Vec::new();
    let mut start = 0usize;

    loop {
        if start >= chars.len() {
            break;
        }

        let mut end = start;
        let mut width = 0usize;

        while end < chars.len() {
            let cw = glyph_width(chars[end]);
            if width > 0 && width + cw > w {
                // Spaces don't trigger a wrap — they stay as invisible
                // trailing whitespace on the current row.
                if chars[end] == ' ' {
                    end += 1;
                    continue;
                }
                break;
            }
            width += cw;
            end += 1;
        }

        if end < chars.len() {
            let text: String = chars[start.min(chars.len())..end.min(chars.len())].iter().collect();
            rows.push((text, start));
            start = end;
        } else {
            let text: String = chars[start.min(chars.len())..].iter().collect();
            rows.push((text.clone(), start));
            if display_width(&text) >= w {
                rows.push((String::new(), chars.len()));
            }
            break;
        }
    }

    if rows.is_empty() {
        rows.push((String::new(), 0));
    }
    rows
}

/// Char offset into `line` for visual chunk `within` at display column `col`.
fn char_index_at_display_col(line: &str, within: usize, col: usize, w: usize) -> usize {
    if w == 0 {
        return col.min(line.chars().count());
    }
    let chunks = wrap_hard_line(line, w);
    if let Some((chunk_text, chunk_start)) = chunks.get(within) {
        let mut cur_w = 0usize;
        let mut idx = 0usize;
        for ch in chunk_text.chars() {
            if cur_w >= col {
                return chunk_start + idx;
            }
            cur_w += glyph_width(ch);
            idx += 1;
        }
        return chunk_start + idx;
    }
    line.chars().count()
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
        let mut i = InputState {
            text_cols: 10,
            ..Default::default()
        };
        for _ in 0..25 {
            i.insert('x');
        }
        // 25 chars @ width 10 → 3 visual rows (char-wrap, single long word)
        assert_eq!(i.visual_row_count(10), 3);
        assert_eq!(i.visible_line_count(10), 3);
        assert_eq!(i.cursor_visual(10), (2, 5)); // row 2, col 5
    }

    #[test]
    fn soft_wrap_caps_visible_and_scrolls() {
        let mut i = InputState {
            text_cols: 4,
            ..Default::default()
        };
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
        let mut i = InputState {
            text_cols: 5,
            ..Default::default()
        };
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
        let i = InputState {
            value: "abcdefghij".into(),
            ..Default::default()
        };
        let rows = i.wrapped_rows(4);
        // Single long word → char-wrap
        assert_eq!(
            rows,
            vec![
                (true, "abcd".into()),
                (false, "efgh".into()),
                (false, "ij".into()),
            ]
        );
    }

    #[test]
    fn trailing_space_does_not_create_empty_row() {
        let i = InputState {
            value: "abc ".into(), // 4 chars, width 4
            ..Default::default()
        };
        let rows = i.wrapped_rows(4);
        // "abc " fills the width → one row, trailing empty for caret
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1, "abc ");
        assert_eq!(rows[1].1, "");
    }

    #[test]
    fn insert_str_normalizes_crlf_and_moves_cursor() {
        let mut i = InputState::default();
        i.insert_str("a\r\nb\rc");
        assert_eq!(i.value, "a\nb\nc");
        assert_eq!(i.cursor, 5);
    }

    #[test]
    fn wide_glyphs_wrap_by_display_width() {
        // Fullwidth chars are typically width 2.
        let i = InputState {
            value: "あああ".into(), // 3 chars, 6 columns
            cursor: 3,
            ..Default::default()
        };
        let rows = i.wrapped_rows(4);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(i.visual_row_count(4), 2);
        // caret after all three → row 1, col 2 (third glyph starts a new row)
        assert_eq!(i.cursor_visual(4), (1, 2));
    }

    #[test]
    fn empty_hard_lines_keep_a_row() {
        let i = InputState {
            value: "a\n\nb".into(),
            ..Default::default()
        };
        let rows = i.wrapped_rows(80);
        assert_eq!(
            rows,
            vec![
                (true, "a".into()),
                (false, String::new()),
                (false, "b".into()),
            ]
        );
    }

    #[test]
    fn exact_wrap_width_keeps_end_caret_row() {
        let i = InputState {
            value: "abcd".into(),
            cursor: 4,
            text_cols: 4,
        };
        // "abcd" fills width exactly → trailing empty row for caret
        assert_eq!(i.visual_row_count(4), 2);
        assert_eq!(
            i.wrapped_rows(4),
            vec![(true, "abcd".into()), (false, String::new())]
        );
        assert_eq!(i.cursor_visual(4), (1, 0));
        assert_eq!(i.view_scroll(i.visible_line_count(4), 4), 0);
    }

    #[test]
    fn space_after_near_full_line_does_not_blank_strip() {
        let mut i = InputState {
            text_cols: 4,
            ..Default::default()
        };
        for ch in ['a', 'b', 'c'] {
            i.insert(ch);
        }
        assert_eq!(i.visual_row_count(4), 1);
        assert_eq!(i.view_scroll(1, 4), 0);

        i.insert(' '); // now exact width 4
        assert_eq!(i.value, "abc ");
        // "abc " fills the width → trailing empty row for caret
        assert_eq!(i.visual_row_count(4), 2);
        assert_eq!(i.cursor_visual(4), (1, 0));
        assert_eq!(i.view_scroll(2, 4), 0);

        i.insert(' '); // past the boundary — still stable
        // "abc  " → "abc " (row 0) + "" (row 1, trailing space skipped)
        assert_eq!(i.visual_row_count(4), 2);
        assert_eq!(i.view_scroll(2, 4), 0);
    }
}
