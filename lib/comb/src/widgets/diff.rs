//! A scrollable unified-diff view (additions / deletions / context).

use crate::core::buffer::Buffer;
use crate::core::geom::Rect;
use crate::core::style::{Color, Style};
use crate::core::text::{Line, Span};
use crate::term::event::MouseKind;
use crate::widgets::scrollbar::Scrollbar;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiffOp {
    Header,
    Context,
    Add,
    Delete,
}

/// Colours for a diff view.
#[derive(Clone, Copy, Debug)]
pub struct DiffTheme {
    pub header: Style,
    pub context: Style,
    pub add: Style,
    pub delete: Style,
    pub meta: Style,
}

impl DiffTheme {
    pub fn dark() -> Self {
        DiffTheme {
            header: Style::new().fg(Color::rgb(0x7a, 0xa2, 0xf7)).bold(),
            context: Style::new().fg(Color::rgb(0xa0, 0xa0, 0xa0)),
            add: Style::new().fg(Color::rgb(0x81, 0xc7, 0x84)),
            delete: Style::new().fg(Color::rgb(0xe5, 0x73, 0x73)),
            meta: Style::new().fg(Color::rgb(0x70, 0x70, 0x70)),
        }
    }
}

/// Scrollable highlighted unified diff.
pub struct DiffView {
    pub theme: DiffTheme,
    pub scrollbar: Scrollbar,
    pub offset: usize,
    lines: Vec<Line>,
}

impl DiffView {
    pub fn new(old: &str, new: &str) -> Self {
        let mut v = DiffView {
            theme: DiffTheme::dark(),
            scrollbar: Scrollbar::default(),
            offset: 0,
            lines: Vec::new(),
        };
        v.set_diff(old, new);
        v
    }

    pub fn from_lines(lines: Vec<Line>) -> Self {
        DiffView {
            theme: DiffTheme::dark(),
            scrollbar: Scrollbar::default(),
            offset: 0,
            lines,
        }
    }

    pub fn set_diff(&mut self, old: &str, new: &str) {
        self.lines = unified_diff(old, new, &self.theme);
        self.offset = 0;
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let last = self.lines.len().saturating_sub(1) as isize;
        self.offset = (self.offset as isize + delta).clamp(0, last.max(0)) as usize;
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, bg: Option<Style>) {
        if area.is_empty() {
            return;
        }
        if let Some(bg) = bg {
            buf.paint(area, bg);
        }
        let h = area.height as usize;
        let overflow = self.lines.len() > h;
        let content = self.scrollbar.content_area(area, overflow);
        self.offset = self.offset.min(self.lines.len().saturating_sub(h));
        match bg {
            Some(bg) => buf.set_lines_on(content, &self.lines, self.offset, bg),
            None => buf.set_lines(content, &self.lines, self.offset),
        }
        if overflow {
            let bar = self.scrollbar.area(area);
            self.scrollbar
                .render(buf, bar, self.lines.len(), h, self.offset);
        }
    }

    pub fn handle_mouse(&mut self, area: Rect, kind: MouseKind, col: u16, row: u16) -> bool {
        let h = area.height as usize;
        let overflow = self.lines.len() > h;
        if !overflow {
            return false;
        }
        let bar_area = self.scrollbar.area(area);
        if self.scrollbar.is_dragging() || bar_area.contains(col, row) {
            return self.scrollbar.on_mouse(
                bar_area,
                self.lines.len(),
                h,
                &mut self.offset,
                kind,
                col,
                row,
            );
        }
        let content = self.scrollbar.content_area(area, true);
        if !content.contains(col, row) {
            return false;
        }
        match kind {
            MouseKind::ScrollUp => {
                self.scroll_by(-3);
                true
            }
            MouseKind::ScrollDown => {
                self.scroll_by(3);
                true
            }
            _ => false,
        }
    }

    pub fn end_drag(&mut self) {
        self.scrollbar.end_drag();
    }
}

/// Build a simple unified diff between `old` and `new` (line-based LCS).
pub fn unified_diff(old: &str, new: &str, theme: &DiffTheme) -> Vec<Line> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let mut out = Vec::new();
    out.push(line("--- a/source.rs".to_string(), theme.meta));
    out.push(line("+++ b/source.rs".to_string(), theme.meta));
    out.push(line(
        format!("@@ -1,{} +1,{} @@", a.len(), b.len()),
        theme.header,
    ));

    let pairs = lcs_ops(&a, &b);
    for (op, text) in pairs {
        let (prefix, st) = match op {
            DiffOp::Context => (' ', theme.context),
            DiffOp::Add => ('+', theme.add),
            DiffOp::Delete => ('-', theme.delete),
            DiffOp::Header => ('@', theme.header),
        };
        out.push(line(format!("{prefix}{text}"), st));
    }
    out
}

fn line(s: String, st: Style) -> Line {
    Line::from(Span::styled(s, st))
}

fn lcs_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(DiffOp, &'a str)> {
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![vec![0u16; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            dp[i + 1][j + 1] = if a[i] == b[j] {
                dp[i][j] + 1
            } else {
                dp[i][j + 1].max(dp[i + 1][j])
            };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            ops.push((DiffOp::Context, a[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            ops.push((DiffOp::Add, b[j - 1]));
            j -= 1;
        } else {
            ops.push((DiffOp::Delete, a[i - 1]));
            i -= 1;
        }
    }
    ops.reverse();
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_add_and_delete() {
        let theme = DiffTheme::dark();
        let lines = unified_diff("a\nb\nc\n", "a\nx\nc\n", &theme);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.clone()).collect())
            .collect();
        assert!(text.iter().any(|t| t.starts_with("-b")));
        assert!(text.iter().any(|t| t.starts_with("+x")));
        assert!(text.iter().any(|t| t.starts_with(" a") || t == " a"));
    }
}
