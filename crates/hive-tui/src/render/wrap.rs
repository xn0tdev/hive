//! Width-aware word wrapping that preserves per-span styling. Wrapping happens
//! here (not in ratatui's Paragraph) so we always know the exact rendered line
//! count and can scroll precisely.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

pub fn wrap_lines(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        out.extend(wrap_one(line, width));
    }
    out
}

fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

fn wrap_one(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    let mut cells: Vec<(char, Style)> = Vec::new();
    for span in line.spans {
        for ch in span.content.chars() {
            cells.push((ch, span.style));
        }
    }
    if cells.is_empty() {
        return vec![Line::from(String::new())];
    }

    let mut result: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<(char, Style)> = Vec::new();
    let mut cur_w = 0usize;
    let mut last_space: Option<usize> = None;

    for (ch, st) in cells {
        let w = char_width(ch);
        if cur_w + w > width && !cur.is_empty() {
            match last_space {
                Some(sp) if sp + 1 < cur.len() => {
                    let carry: Vec<(char, Style)> = cur.split_off(sp + 1);
                    cur.pop(); // drop the space at the break
                    result.push(to_line(&cur));
                    cur = carry;
                    cur_w = cur.iter().map(|(c, _)| char_width(*c)).sum();
                }
                _ => {
                    result.push(to_line(&cur));
                    cur.clear();
                    cur_w = 0;
                }
            }
            last_space = None;
        }

        if ch == ' ' {
            last_space = Some(cur.len());
        }
        cur.push((ch, st));
        cur_w += w;
    }

    if !cur.is_empty() {
        result.push(to_line(&cur));
    }
    result
}

fn to_line(cells: &[(char, Style)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut cur_style: Option<Style> = None;

    for (ch, st) in cells {
        match cur_style {
            Some(s) if s == *st => buf.push(*ch),
            _ => {
                if let Some(s) = cur_style {
                    spans.push(Span::styled(std::mem::take(&mut buf), s));
                }
                buf.push(*ch);
                cur_style = Some(*st);
            }
        }
    }
    if let Some(s) = cur_style {
        spans.push(Span::styled(buf, s));
    }
    if spans.is_empty() {
        spans.push(Span::raw(""));
    }
    Line::from(spans)
}
