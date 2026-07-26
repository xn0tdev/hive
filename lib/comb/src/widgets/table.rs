//! A column-aligned table with headers, box borders, and per-cell styling.
//!
//! ```ignore
//! use comb::{Table, TableCol, Style, Rect, Buffer};
//! let mut t = Table::new()
//!     .header(&["Name", "Status", "Count"])
//!     .col(TableCol::new().min(8))
//!     .col(TableCol::new().align(Align::Center))
//!     .col(TableCol::new().align(Align::End))
//!     .row(&["hive-core", "ok", "42"]);
//! t.render(buf, area);
//! ```

use unicode_width::UnicodeWidthStr;

use crate::core::buffer::Buffer;
use crate::core::geom::{Align, Rect};
use crate::core::style::Style;
use crate::draw::border::Border;

/// Column descriptor: sizing + alignment.
#[derive(Clone, Debug)]
pub struct TableCol {
    pub min_width: u16,
    pub max_width: Option<u16>,
    pub align: Align,
}

impl Default for TableCol {
    fn default() -> Self {
        TableCol {
            min_width: 1,
            max_width: None,
            align: Align::Start,
        }
    }
}

impl TableCol {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn min(mut self, w: u16) -> Self {
        self.min_width = w;
        self
    }

    pub fn max(mut self, w: u16) -> Self {
        self.max_width = Some(w);
        self
    }

    pub fn align(mut self, a: Align) -> Self {
        self.align = a;
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub cols: Vec<TableCol>,
    pub border: Border,
    pub header_style: Style,
    pub cell_style: Style,
    pub border_style: Style,
    pub header_bg: Option<Style>,
    pub cell_bg: Option<Style>,
    pub scroll: usize,
}

impl Table {
    pub fn new() -> Self {
        Table {
            border: Border::Plain,
            ..Default::default()
        }
    }

    pub fn header(mut self, labels: &[&str]) -> Self {
        self.header = labels.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn col(mut self, c: TableCol) -> Self {
        self.cols.push(c);
        self
    }

    pub fn row(mut self, cells: &[&str]) -> Self {
        self.rows.push(cells.iter().map(|s| s.to_string()).collect());
        self
    }

    pub fn border(mut self, b: Border) -> Self {
        self.border = b;
        self
    }

    pub fn header_style(mut self, s: Style) -> Self {
        self.header_style = s;
        self
    }

    pub fn cell_style(mut self, s: Style) -> Self {
        self.cell_style = s;
        self
    }

    pub fn border_style(mut self, s: Style) -> Self {
        self.border_style = s;
        self
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Compute column widths from content, clamped to `available`.
    fn col_widths(&self, available: usize) -> Vec<u16> {
        let n = self.cols.len().max(self.header.len());
        if n == 0 {
            return Vec::new();
        }
        let pad = 2usize;
        let chrome = (n + 1) + n * pad;
        let budget = available.saturating_sub(chrome).max(n);

        let mut natural = vec![1u16; n];
        for (i, h) in self.header.iter().enumerate().take(n) {
            natural[i] = natural[i].max(UnicodeWidthStr::width(h.as_str()) as u16);
        }
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate().take(n) {
                natural[i] = natural[i].max(UnicodeWidthStr::width(cell.as_str()) as u16);
            }
        }
        for (i, c) in self.cols.iter().enumerate().take(n) {
            natural[i] = natural[i].max(c.min_width);
            if let Some(mx) = c.max_width {
                natural[i] = natural[i].min(mx);
            }
        }

        fit_widths(&natural, n, budget)
    }
    pub fn render(&mut self, buf: &mut Buffer, area: Rect) {
        if area.is_empty() || self.cols.is_empty() {
            return;
        }
        let h = area.height as usize;
        let w = area.width as usize;
        let widths = self.col_widths(w);

        let rows_total = self.rows.len() + 1; // +1 for header
        let max_scroll = rows_total.saturating_sub(h);
        self.scroll = self.scroll.min(max_scroll);

        let pad = 1u16;
        let chrome_st = self.border_style;
        let head_st = self.header_style;
        let body_st = self.cell_style;

        let mut y = area.y;
        let mut row_idx = self.scroll;

        // Header
        if row_idx == 0 && y < area.bottom() {
            let bg = self.header_bg.unwrap_or(Style::new());
            render_row(buf, area.x, y, &widths, pad, &self.header, &self.cols, head_st, bg, chrome_st);
            y += 1;
            row_idx = 1;
        }

        // Body
        while y < area.bottom() && row_idx - 1 < self.rows.len() {
            let row = &self.rows[row_idx - 1];
            let bg = self.cell_bg.unwrap_or(Style::new());
            render_row(buf, area.x, y, &widths, pad, row, &self.cols, body_st, bg, chrome_st);
            y += 1;
            row_idx += 1;
        }
    }
}

fn render_row(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    widths: &[u16],
    pad: u16,
    cells: &[String],
    cols: &[TableCol],
    style: Style,
    bg: Style,
    chrome: Style,
) {
    let mut cx = x;
    buf.set(cx, y, '│', chrome);
    cx += 1;
    for (i, w) in widths.iter().enumerate() {
        let text = cells.get(i).map(|s| s.as_str()).unwrap_or("");
        let align = cols.get(i).map(|c| c.align).unwrap_or(Align::Start);
        let bg = bg.patch(chrome);
        let st = style.patch(bg);

        buf.paint(Rect::new(cx, y, pad, 1), bg);
        cx += pad;

        let text_w = UnicodeWidthStr::width(text) as u16;
        let (lp, rp) = match align {
            Align::Start | Align::Center => (0u16, *w - text_w.min(*w)),
            Align::End => (*w - text_w.min(*w), 0u16),
        };
        if lp > 0 {
            buf.paint(Rect::new(cx, y, lp, 1), bg);
        }
        buf.set_str(cx + lp, y, text, st);
        if rp > 0 {
            buf.paint(Rect::new(cx + lp + text_w.min(*w) as u16, y, rp, 1), bg);
        }
        cx += *w;
        buf.paint(Rect::new(cx, y, pad, 1), bg);
        cx += pad;
        buf.set(cx, y, '│', chrome);
        cx += 1;
    }
}

/// Distribute `budget` across `n` columns, respecting minimum widths.
fn fit_widths(natural: &[u16], n: usize, budget: usize) -> Vec<u16> {
    let mut widths = vec![0u16; n];
    let mut remaining = budget;

    // Pass 1: give each column its minimum or natural, whichever is smaller.
    for (i, &nat) in natural.iter().enumerate().take(n) {
        let give = nat as usize;
        let actual = give.min(remaining);
        widths[i] = actual as u16;
        remaining -= actual;
    }

    // Pass 2: distribute leftover proportionally to what each column still needs.
    let mut needed: Vec<usize> = (0..n)
        .map(|i| (natural[i] as usize).saturating_sub(widths[i] as usize))
        .collect();
    let mut total_needed: usize = needed.iter().sum();
    while remaining > 0 && total_needed > 0 {
        for i in 0..n {
            if remaining == 0 {
                break;
            }
            if needed[i] > 0 {
                widths[i] += 1;
                needed[i] -= 1;
                remaining -= 1;
            }
        }
        let new_total: usize = needed.iter().sum();
        if new_total == 0 {
            break;
        }
        total_needed = new_total;
    }

    // Pass 3: any remaining goes to columns that can still grow (round-robin).
    let mut i = 0;
    while remaining > 0 {
        widths[i % n] += 1;
        remaining -= 1;
        i += 1;
        if i > n * 1000 {
            break;
        }
    }

    widths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::geom::Size;
    use crate::core::style::Color;
    use crate::term::terminal::render;

    #[test]
    fn render_header_and_row() {
        let mut t = Table::new()
            .header(&["Name", "Count"])
            .col(TableCol::new().min(4))
            .col(TableCol::new().align(Align::End))
            .row(&["hive", "42"])
            .border_style(Style::new().fg(Color::rgb(0x50, 0x50, 0x50)));

        let buf = render(Size::new(30, 5), |f| {
            t.render(f.buffer(), Rect::new(0, 0, 30, 5));
        });

        let row0: String = (0..30).map(|x| buf.get(x, 0).unwrap().ch).collect();
        assert!(row0.contains("Name"));
        assert!(row0.contains("Count"));
        let row1: String = (0..30).map(|x| buf.get(x, 1).unwrap().ch).collect();
        assert!(row1.contains("hive"));
        assert!(row1.contains("42"));
    }

    #[test]
    fn col_widths_respect_min() {
        let t = Table::new()
            .header(&["A", "B"])
            .col(TableCol::new().min(10))
            .col(TableCol::new().min(5));
        let w = t.col_widths(50);
        assert!(w[0] >= 10);
        assert!(w[1] >= 5);
    }

    #[test]
    fn row_count() {
        let t = Table::new()
            .header(&["A"])
            .row(&["1"])
            .row(&["2"])
            .row(&["3"]);
        assert_eq!(t.row_count(), 3);
    }
}
