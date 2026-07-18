//! Width-aware word wrapping that preserves per-span styling. Wrapping happens
//! here (not in the renderer) so we always know the exact rendered line count
//! and can scroll precisely.

use comb::{Line, Span, Style};
use unicode_width::UnicodeWidthChar;

pub fn wrap_lines(lines: Vec<Line>, width: usize) -> Vec<Line> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        // Keep markdown tables / code frames intact — wrapping them leaves
        // orphan `│` / `└` / rule glyphs that look like broken markup.
        let plain: String = line.spans.iter().map(|s| s.content.as_str()).collect();
        let is_tableish = plain.contains('│')
            || plain.starts_with('┌')
            || plain.starts_with('└')
            || (!plain.is_empty() && plain.chars().all(|c| c == '─'));
        if is_tableish {
            out.push(truncate_line(line, width));
        } else {
            out.extend(wrap_one(line, width));
        }
    }
    out
}

fn truncate_line(line: Line, width: usize) -> Line {
    let mut cells: Vec<(char, Style)> = Vec::new();
    let mut w = 0usize;
    for span in line.spans {
        for ch in span.content.chars() {
            let cw = char_width(ch);
            if w + cw > width {
                return to_line(&cells);
            }
            cells.push((ch, span.style));
            w += cw;
        }
    }
    to_line(&cells)
}

fn char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

fn wrap_one(line: Line, width: usize) -> Vec<Line> {
    let mut cells: Vec<(char, Style)> = Vec::new();
    for span in line.spans {
        for ch in span.content.chars() {
            cells.push((ch, span.style));
        }
    }
    if cells.is_empty() {
        return vec![Line::from(String::new())];
    }

    let mut result: Vec<Line> = Vec::new();
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

fn to_line(cells: &[(char, Style)]) -> Line {
    let mut spans: Vec<Span> = Vec::new();
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
