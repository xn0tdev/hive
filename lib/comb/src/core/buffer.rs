//! The cell grid: a `width × height` array of [`Cell`]s that widgets draw into.
//! Two buffers (front = on screen, back = next frame) are diffed so the terminal
//! only receives the cells that actually changed.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::core::geom::{Align, Rect, Size};
use crate::core::style::Style;
use crate::core::text::Line;

/// Display columns for a label with `pad` blank cells on each side.
pub fn label_cols(text: &str, pad: u16) -> u16 {
    let w = UnicodeWidthStr::width(text) as u16;
    w.saturating_add(pad.saturating_mul(2))
}

/// Shared chip width that exactly centers the greatest number of labels.
///
/// A label can have equal left and right pads only when its display width has
/// the same parity as the chip. When the labels have mixed parity, this picks
/// the majority parity; ties keep the narrowest possible chip. This makes the
/// unavoidable one-cell imbalance affect the fewest labels.
pub fn chip_cols(labels: &[&str], pad: u16) -> u16 {
    let w = labels
        .iter()
        .map(|t| label_cols(t, pad))
        .max()
        .unwrap_or(pad.saturating_mul(2));

    let (even, odd) = labels.iter().fold((0usize, 0usize), |(even, odd), text| {
        if UnicodeWidthStr::width(*text) % 2 == 0 {
            (even + 1, odd)
        } else {
            (even, odd + 1)
        }
    });
    let preferred_parity = if odd > even { 1 } else { 0 };

    if w % 2 == preferred_parity {
        w
    } else {
        w.saturating_add(1)
    }
}

/// Pad `text` to exactly `width` display columns (centered).
///
/// When the gap is odd, the extra column goes on the **right**.
pub fn center_pad(text: &str, width: u16) -> String {
    pad_align(text, width, Align::Center)
}

fn clip_text(text: &str, width: u16) -> (String, u16) {
    let mut out = String::new();
    let mut tw = 0u16;
    for ch in text.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        if w == 0 {
            continue;
        }
        if tw + w > width {
            break;
        }
        out.push(ch);
        tw += w;
    }
    (out, tw)
}

fn pad_align(text: &str, width: u16, align: Align) -> String {
    let (core, tw) = clip_text(text, width);
    if tw >= width {
        return core;
    }
    let gap = width - tw;
    let (left, right) = match align {
        Align::Start => (0, gap),
        Align::End => (gap, 0),
        Align::Center => {
            let left = gap / 2;
            (left, gap - left)
        }
    };
    let mut s = String::with_capacity(width as usize);
    for _ in 0..left {
        s.push(' ');
    }
    s.push_str(&core);
    for _ in 0..right {
        s.push(' ');
    }
    s
}

/// Marker in the cell to the right of a double-width glyph. Skipped when
/// flushing ANSI so the terminal does not advance an extra column.
pub const WIDE_CONT: char = '\u{FFFE}';

/// One character cell. `'\0'` marks a *transparent* cell — it is skipped when a
/// surface is composited, letting lower layers show through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

impl Cell {
    pub const fn blank() -> Self {
        Cell {
            ch: ' ',
            style: Style::new(),
        }
    }

    pub const fn transparent() -> Self {
        Cell {
            ch: '\0',
            style: Style::new(),
        }
    }

    pub fn is_transparent(&self) -> bool {
        self.ch == '\0'
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell::blank()
    }
}

#[derive(Clone)]
pub struct Buffer {
    pub width: u16,
    pub height: u16,
    cells: Vec<Cell>,
}

impl Buffer {
    /// A buffer of blank (opaque space) cells — used for the screen root.
    pub fn blank(size: Size) -> Self {
        Buffer::filled(size, Cell::blank())
    }

    /// A buffer of transparent cells — used for overlay surfaces.
    pub fn transparent(size: Size) -> Self {
        Buffer::filled(size, Cell::transparent())
    }

    pub fn filled(size: Size, cell: Cell) -> Self {
        Buffer {
            width: size.width,
            height: size.height,
            cells: vec![cell; size.area() as usize],
        }
    }

    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        if x < self.width && y < self.height {
            Some(y as usize * self.width as usize + x as usize)
        } else {
            None
        }
    }

    pub fn get(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).map(|i| &self.cells[i])
    }

    pub fn cell_mut(&mut self, x: u16, y: u16) -> Option<&mut Cell> {
        self.index(x, y).map(|i| &mut self.cells[i])
    }

    pub fn set(&mut self, x: u16, y: u16, ch: char, style: Style) {
        if let Some(c) = self.cell_mut(x, y) {
            c.ch = ch;
            // Patch, don't replace: `None` colours inherit the cell underneath.
            // Critical for panels — `set_line` clears short-line tails with
            // `Style::new()`, which must not punch black holes through a painted bg.
            c.style = c.style.patch(style);
        }
    }

    /// Write one grapheme cell, honouring East-Asian / emoji display width.
    /// Returns columns advanced (0, 1, or 2).
    fn put_glyph(&mut self, x: u16, y: u16, ch: char, style: Style, limit: u16) -> u16 {
        if x >= limit {
            return 0;
        }
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w == 0 {
            // Combining / zero-width: keep layout stable; skip.
            return 0;
        }
        if w >= 2 {
            // Need two cells; if only one remains, fall back to a space.
            if x + 1 >= limit {
                self.set(x, y, ' ', style);
                return 1;
            }
            self.set(x, y, ch, style);
            self.set(x + 1, y, WIDE_CONT, style);
            return 2;
        }
        self.set(x, y, ch, style);
        1
    }

    /// Write `s` starting at `(x, y)`, clipping at the buffer edge. Returns the
    /// column just past the last glyph written.
    pub fn set_str(&mut self, x: u16, y: u16, s: &str, style: Style) -> u16 {
        let mut cx = x;
        for ch in s.chars() {
            if cx >= self.width {
                break;
            }
            let adv = self.put_glyph(cx, y, ch, style, self.width);
            if adv == 0 && UnicodeWidthChar::width(ch).unwrap_or(0) == 0 {
                continue;
            }
            if adv == 0 {
                break;
            }
            cx = cx.saturating_add(adv);
        }
        cx
    }

    /// Draw a styled line at `(x, y)`, each span keeping its own style, clipped
    /// to `max_width` cells (and the buffer edge).
    pub fn set_line(&mut self, x: u16, y: u16, line: &Line, max_width: u16) {
        self.set_line_on(x, y, line, max_width, Style::new());
    }

    /// Like [`set_line`], but `base` (typically a panel background) fills what
    /// each span leaves unset, so fg-only highlight styles don't wipe the row
    /// bg — while a span that picked its own colours keeps them.
    /// Always clears the remainder of the row so scroll / short lines cannot
    /// leave stale glyphs behind.
    pub fn set_line_on(&mut self, x: u16, y: u16, line: &Line, max_width: u16, base: Style) {
        let mut cx = x;
        let limit = x.saturating_add(max_width).min(self.width);
        for span in &line.spans {
            // `base` fills what the span leaves unset — it must not repaint a
            // span that chose its own colours (diff bands, selected chips).
            let st = base.patch(span.style);
            for ch in span.content.chars() {
                if cx >= limit {
                    // Still clear the rest of the row below.
                    break;
                }
                let adv = self.put_glyph(cx, y, ch, st, limit);
                if adv == 0 {
                    if UnicodeWidthChar::width(ch).unwrap_or(0) == 0 {
                        continue;
                    }
                    break;
                }
                cx = cx.saturating_add(adv);
            }
            if cx >= limit {
                break;
            }
        }
        // Clear vacated cells — required for scroll artifacts and width changes.
        while cx < limit {
            self.set(cx, y, ' ', base);
            cx += 1;
        }
    }

    /// Draw `lines` top-down within `area`, starting at logical index `scroll`,
    /// clipped to the area's height and width. The Paragraph-with-scroll of comb.
    pub fn set_lines(&mut self, area: Rect, lines: &[Line], scroll: usize) {
        self.set_lines_on(area, lines, scroll, Style::new());
    }

    /// Like [`set_lines`], patching every cell with `base` (e.g. panel bg).
    /// Empty rows in the viewport are always cleared.
    pub fn set_lines_on(&mut self, area: Rect, lines: &[Line], scroll: usize, base: Style) {
        for row in 0..area.height {
            let idx = scroll + row as usize;
            let y = area.y + row;
            match lines.get(idx) {
                Some(line) => self.set_line_on(area.x, y, line, area.width, base),
                None => {
                    self.paint(Rect::new(area.x, y, area.width, 1), base);
                }
            }
        }
    }

    /// Fill a rectangle with `ch`/`style` (intersected with the buffer).
    pub fn fill(&mut self, rect: Rect, ch: char, style: Style) {
        let r = rect.intersection(self.area());
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                self.set(x, y, ch, style);
            }
        }
    }

    /// Paint a background over a rectangle, leaving spaces. Handy for panels.
    pub fn paint(&mut self, rect: Rect, style: Style) {
        self.fill(rect, ' ', style);
    }

    /// Fill a horizontal band with `style`, then draw `text` inside it.
    ///
    /// The whole band is painted first, then one padded line is written so chip
    /// backgrounds stay solid. [`Align::Center`] uses [`center_pad`].
    pub fn put_str_aligned(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        text: &str,
        style: Style,
        align: Align,
    ) {
        if width == 0 || y >= self.height {
            return;
        }
        let x = x.min(self.width);
        let width = width.min(self.width.saturating_sub(x));
        if width == 0 {
            return;
        }
        self.paint(Rect::new(x, y, width, 1), style);
        let padded = match align {
            Align::Center => center_pad(text, width),
            Align::Start => pad_align(text, width, Align::Start),
            Align::End => pad_align(text, width, Align::End),
        };
        self.set_line_on(
            x,
            y,
            &Line::from(crate::core::text::Span::styled(padded, style)),
            width,
            style,
        );
    }

    /// Fill `area` with `style` and center `text` horizontally + vertically.
    ///
    /// Use [`chip_cols`] to choose a shared width that centers as many labels
    /// as possible when their display widths have mixed parity.
    pub fn put_label(&mut self, area: Rect, text: &str, style: Style) {
        let area = area.intersection(self.area());
        if area.is_empty() {
            return;
        }
        self.paint(area, style);
        let y = area.y + area.height / 2;
        self.put_str_aligned(area.x, y, area.width, text, style, Align::Center);
    }

    /// Copy `src` onto this buffer at `(ox, oy)`. When `skip_transparent`, cells
    /// marked transparent in `src` don't overwrite what's underneath.
    pub fn blit(&mut self, ox: u16, oy: u16, src: &Buffer, skip_transparent: bool) {
        for sy in 0..src.height {
            for sx in 0..src.width {
                let cell = src.cells[sy as usize * src.width as usize + sx as usize];
                if skip_transparent && cell.is_transparent() {
                    continue;
                }
                self.set(ox + sx, oy + sy, cell.ch, cell.style);
            }
        }
    }

    /// The cells that differ from `prev` (same dimensions assumed), as
    /// `(x, y, cell)`, in row-major order — ready to emit to the terminal.
    pub fn diff<'a>(&'a self, prev: &Buffer) -> Vec<(u16, u16, &'a Cell)> {
        let mut out = Vec::new();
        if self.width != prev.width || self.height != prev.height {
            // Different geometry: treat everything as changed.
            for y in 0..self.height {
                for x in 0..self.width {
                    out.push((x, y, &self.cells[self.index(x, y).unwrap()]));
                }
            }
            return out;
        }
        for (i, (a, b)) in self.cells.iter().zip(prev.cells.iter()).enumerate() {
            if a != b {
                let x = (i % self.width as usize) as u16;
                let y = (i / self.width as usize) as u16;
                out.push((x, y, a));
            }
        }
        out
    }

    /// The whole buffer as text, rows joined by `\n` (transparent / wide-cont
    /// cells become spaces). Handy for snapshot-style assertions in tests.
    pub fn text(&self) -> String {
        let mut s = String::with_capacity((self.width as usize + 1) * self.height as usize);
        for y in 0..self.height {
            for x in 0..self.width {
                let ch = self.cells[y as usize * self.width as usize + x as usize].ch;
                s.push(if ch == '\0' || ch == WIDE_CONT {
                    ' '
                } else {
                    ch
                });
            }
            if y + 1 < self.height {
                s.push('\n');
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::style::{Color, Style};
    use crate::core::text::Span;

    #[test]
    fn base_style_fills_gaps_without_repainting_spans() {
        let panel = Style::new().bg(Color::rgb(0x26, 0x26, 0x26));
        let own = Style::new().bg(Color::rgb(0xd8, 0xd8, 0xd8));
        let line = Line::from(vec![Span::raw("a"), Span::styled("b", own)]);

        let mut buf = Buffer::blank(Size::new(4, 1));
        buf.set_line_on(0, 0, &line, 4, panel);

        assert_eq!(
            buf.get(0, 0).unwrap().style.bg,
            panel.bg,
            "unstyled takes base"
        );
        assert_eq!(
            buf.get(1, 0).unwrap().style.bg,
            own.bg,
            "a span that picked its own background keeps it"
        );
        assert_eq!(buf.get(3, 0).unwrap().style.bg, panel.bg, "tail keeps base");
    }

    #[test]
    fn set_str_clips_at_edge() {
        let mut b = Buffer::blank(Size::new(4, 1));
        let next = b.set_str(2, 0, "hello", Style::new());
        assert_eq!(next, 4); // stopped at the right edge
        let row: String = (0..4).map(|x| b.get(x, 0).unwrap().ch).collect();
        assert_eq!(row, "  he");
    }

    #[test]
    fn diff_reports_only_changes() {
        let a = Buffer::blank(Size::new(3, 1));
        let mut b = a.clone();
        b.set(1, 0, 'x', Style::new().fg(Color::rgb(9, 9, 9)));
        let d = b.diff(&a);
        assert_eq!(d.len(), 1);
        assert_eq!((d[0].0, d[0].1, d[0].2.ch), (1, 0, 'x'));
    }

    #[test]
    fn set_line_clears_remainder_of_row() {
        let mut b = Buffer::blank(Size::new(8, 1));
        b.set_str(0, 0, "abcdefgh", Style::new());
        b.set_line(0, 0, &Line::from("hi"), 8);
        let row: String = (0..8).map(|x| b.get(x, 0).unwrap().ch).collect();
        assert_eq!(row, "hi      ");
    }

    #[test]
    fn set_line_preserves_painted_panel_bg_on_tail() {
        let panel = Color::rgb(0x22, 0x22, 0x22);
        let mut b = Buffer::blank(Size::new(10, 1));
        b.paint(Rect::new(0, 0, 10, 1), Style::new().bg(panel));
        // Short line + clear remainder (Style::new base) must keep panel bg.
        b.set_line(
            0,
            0,
            &Line::from(Span::styled(
                "Settings",
                Style::new().fg(Color::rgb(0xe0, 0xe0, 0xe0)),
            )),
            10,
        );
        for x in 0..10 {
            assert_eq!(
                b.get(x, 0).unwrap().style.bg,
                Some(panel),
                "col {x} lost panel bg"
            );
        }
        assert_eq!(b.get(0, 0).unwrap().ch, 'S');
        assert_eq!(b.get(8, 0).unwrap().ch, ' ');
    }

    #[test]
    fn set_lines_clears_empty_viewport_rows() {
        let mut b = Buffer::blank(Size::new(4, 3));
        b.set_str(0, 0, "aaaa", Style::new());
        b.set_str(0, 1, "bbbb", Style::new());
        b.set_str(0, 2, "cccc", Style::new());
        let lines = vec![Line::from("x")];
        b.set_lines(Rect::new(0, 0, 4, 3), &lines, 0);
        assert_eq!(b.get(0, 0).unwrap().ch, 'x');
        assert_eq!(b.get(1, 0).unwrap().ch, ' ');
        assert_eq!(b.get(0, 1).unwrap().ch, ' ');
        assert_eq!(b.get(0, 2).unwrap().ch, ' ');
    }

    #[test]
    fn wide_glyph_reserves_two_cells() {
        let mut b = Buffer::blank(Size::new(4, 1));
        b.set_str(0, 0, "あ", Style::new());
        assert_eq!(b.get(0, 0).unwrap().ch, 'あ');
        assert_eq!(b.get(1, 0).unwrap().ch, WIDE_CONT);
        assert_eq!(b.get(2, 0).unwrap().ch, ' ');
    }

    #[test]
    fn chip_cols_prefers_the_majority_label_parity() {
        assert_eq!(chip_cols(&["MAKE", "MARK", "SEND"], 1), 6);
        assert_eq!(center_pad("MAKE", 6), " MAKE ");
        assert_eq!(center_pad("MARK", 6), " MARK ");
        assert_eq!(center_pad("SEND", 6), " SEND ");
    }

    #[test]
    fn put_label_preserves_shared_action_width() {
        let bg = Color::rgb(0x5a, 0x8f, 0xb0);
        let style = Style::new().bg(bg).fg(Color::rgb(0x11, 0x11, 0x11));
        let w = chip_cols(&["MAKE", "MARK", "SEND"], 1);
        assert_eq!(w, 6);
        for (word, expect) in [("MAKE", " MAKE "), ("MARK", " MARK "), ("SEND", " SEND ")] {
            let mut b = Buffer::blank(Size::new(w, 3));
            b.put_label(Rect::new(0, 0, w, 3), word, style);
            let mid: String = (0..w).map(|x| b.get(x, 1).unwrap().ch).collect();
            assert_eq!(mid, expect, "{word}");
            let text_width = UnicodeWidthStr::width(word);
            let pad = (w as usize - text_width) / 2;
            assert_eq!(&mid[..pad], " ".repeat(pad));
            assert_eq!(&mid[pad + text_width..], " ".repeat(pad));
        }
    }
}
