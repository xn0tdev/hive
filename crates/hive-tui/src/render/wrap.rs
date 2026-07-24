//! Width-aware word wrapping that preserves per-span styling. Wrapping happens
//! here (not in the renderer) so we always know the exact rendered line count
//! and can scroll precisely.

use comb::{Line, Span, Style};
use unicode_width::UnicodeWidthChar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapJoin {
    Hard,
    SoftSpace,
    SoftNone,
}

#[derive(Clone, Debug)]
pub struct WrappedLine {
    pub line: Line,
    pub join_before: WrapJoin,
}

pub fn wrap_lines(lines: Vec<Line>, width: usize) -> Vec<Line> {
    wrap_lines_with_joins(lines, width)
        .into_iter()
        .map(|row| row.line)
        .collect()
}

pub fn wrap_lines_with_joins(lines: Vec<Line>, width: usize) -> Vec<WrappedLine> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in lines {
        // Keep markdown tables / fenced code intact — wrapping tables leaves
        // orphan `│` / `└` glyphs, and wrapping code breaks the solid bg band.
        let plain: String = line.spans.iter().map(|s| s.content.as_str()).collect();
        let is_tableish = plain.contains('│')
            || plain.starts_with('┌')
            || plain.starts_with('└')
            || plain.starts_with('├')
            || plain.starts_with('┬')
            || plain.starts_with('┴')
            || (!plain.is_empty()
                && plain.chars().all(|c| {
                    matches!(c, '─' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼')
                }));
        let is_code_fence = is_code_fence_line(&line, &plain);
        if is_tableish || is_code_fence {
            out.push(WrappedLine {
                line: truncate_line(line, width),
                join_before: WrapJoin::Hard,
            });
        } else {
            out.extend(wrap_one(line, width));
        }
    }
    out
}

/// Fenced code rows are left-padded with two spaces and share one `code_bg`
/// across every token span (see `markdown::flush_code_block`).
fn is_code_fence_line(line: &Line, plain: &str) -> bool {
    if !plain.starts_with("  ") {
        return false;
    }
    let mut bg = None;
    for s in &line.spans {
        if s.content.is_empty() {
            continue;
        }
        match (bg, s.style.bg) {
            (None, Some(c)) => bg = Some(c),
            (Some(expected), Some(c)) if c == expected => {}
            _ => return false,
        }
    }
    bg.is_some()
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

fn wrap_one(line: Line, width: usize) -> Vec<WrappedLine> {
    let mut cells: Vec<(char, Style)> = Vec::new();
    for span in line.spans {
        for ch in span.content.chars() {
            cells.push((ch, span.style));
        }
    }
    if cells.is_empty() {
        return vec![WrappedLine {
            line: Line::from(String::new()),
            join_before: WrapJoin::Hard,
        }];
    }

    // Leading whitespace is structural indent (tool tails, nested quotes). Carry
    // it onto every continuation so a short orphan like `1.91s` doesn't jump to
    // column 0.
    let indent: Vec<(char, Style)> = cells
        .iter()
        .take_while(|(ch, _)| *ch == ' ' || *ch == '\t')
        .copied()
        .collect();
    let indent_w: usize = indent.iter().map(|(c, _)| char_width(*c)).sum();
    let preserve_indent = !indent.is_empty() && indent_w + 1 < width;

    let mut result: Vec<WrappedLine> = Vec::new();
    let mut cur: Vec<(char, Style)> = Vec::new();
    let mut cur_w = 0usize;
    let mut last_space: Option<usize> = None;
    let mut cur_join = WrapJoin::Hard;
    // After a soft wrap on non-indented prose, swallow spaces so a new row
    // never starts with a chip's leading pad (` {content} `).
    let mut skip_leading_spaces = false;

    for (ch, st) in cells {
        let w = char_width(ch);
        if cur_w + w > width && !cur.is_empty() {
            // Drop the break-point space and any spaces still trailing on the
            // finished row. Keeping a styled pad (inline `code` chips use
            // ` {content} `) used to paint code_bg out to the wrap edge.
            if let Some(sp) = last_space {
                let carry = if sp + 1 < cur.len() {
                    cur.split_off(sp + 1)
                } else {
                    Vec::new()
                };
                cur.pop(); // drop the space at the break
                trim_trailing_spaces(&mut cur);
                if !cur.is_empty() {
                    result.push(WrappedLine {
                        line: to_line(&cur),
                        join_before: cur_join,
                    });
                }
                cur = reindent(carry, &indent, preserve_indent);
                if !preserve_indent {
                    trim_leading_spaces(&mut cur);
                }
                cur_w = cur.iter().map(|(c, _)| char_width(*c)).sum();
                cur_join = WrapJoin::SoftSpace;
            } else {
                result.push(WrappedLine {
                    line: to_line(&cur),
                    join_before: cur_join,
                });
                cur = reindent(vec![(ch, st)], &indent, preserve_indent);
                cur_w = cur.iter().map(|(c, _)| char_width(*c)).sum();
                cur_join = if ch.is_whitespace() {
                    WrapJoin::SoftSpace
                } else {
                    WrapJoin::SoftNone
                };
                last_space = None;
                skip_leading_spaces = !preserve_indent;
                continue;
            }
            last_space = None;
            skip_leading_spaces = !preserve_indent;
        }

        if skip_leading_spaces {
            if ch == ' ' {
                continue;
            }
            skip_leading_spaces = false;
        }

        if ch == ' ' {
            last_space = Some(cur.len());
        }
        cur.push((ch, st));
        cur_w += w;
    }

    if !cur.is_empty() {
        result.push(WrappedLine {
            line: to_line(&cur),
            join_before: cur_join,
        });
    }
    result
}

fn reindent(
    carry: Vec<(char, Style)>,
    indent: &[(char, Style)],
    preserve: bool,
) -> Vec<(char, Style)> {
    if !preserve || carry.is_empty() {
        return carry;
    }
    let already = carry
        .iter()
        .take(indent.len())
        .map(|(c, _)| *c)
        .eq(indent.iter().map(|(c, _)| *c));
    if already {
        return carry;
    }
    let mut next = indent.to_vec();
    next.extend(carry);
    next
}

fn trim_trailing_spaces(cells: &mut Vec<(char, Style)>) {
    while cells.last().is_some_and(|(c, _)| *c == ' ') {
        cells.pop();
    }
}

fn trim_leading_spaces(cells: &mut Vec<(char, Style)>) {
    let n = cells.iter().take_while(|(c, _)| *c == ' ').count();
    if n > 0 {
        cells.drain(..n);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use comb::Color;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_str()).collect())
            .collect()
    }

    fn code_chip_line() -> Line {
        let code_bg = Color::Rgb(0x1c, 0x1c, 0x1c);
        let code = Style::default()
            .fg(Color::Rgb(0xb0, 0xb0, 0xb0))
            .bg(code_bg);
        let base = Style::default().fg(Color::Rgb(0xd4, 0xd4, 0xd4));
        Line::from(vec![
            Span::styled("Want me to auto-fix the trivial ones (".to_string(), base),
            Span::styled(" useless_format ".to_string(), code),
            Span::styled(", ".to_string(), base),
            Span::styled(" manual_contains ".to_string(), code),
            Span::styled(", ".to_string(), base),
            Span::styled(" unnecessary_unwrap ".to_string(), code),
            Span::styled(")?".to_string(), base),
        ])
    }

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_str()).collect()
    }

    #[test]
    fn reports_space_removed_by_soft_wrap() {
        let rows = wrap_lines_with_joins(vec![Line::from("alpha beta")], 6);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].join_before, WrapJoin::Hard);
        assert_eq!(rows[1].join_before, WrapJoin::SoftSpace);
    }

    #[test]
    fn reports_hard_token_wrap_without_separator() {
        let rows = wrap_lines_with_joins(vec![Line::from("abcdefgh")], 4);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].join_before, WrapJoin::SoftNone);
    }

    #[test]
    fn reports_real_input_line_boundary() {
        let rows = wrap_lines_with_joins(vec![Line::from("first"), Line::from("second")], 20);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].join_before, WrapJoin::Hard);
        assert_eq!(rows[1].join_before, WrapJoin::Hard);
    }

    #[test]
    fn continuation_keeps_leading_indent() {
        let line = Line::from(Span::raw(
            "    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.91s".to_string(),
        ));
        let wrapped = wrap_lines(vec![line], 64);
        let texts = plain(&wrapped);
        assert!(texts.len() >= 2, "expected a wrap, got {texts:?}");
        assert!(
            texts[0].starts_with("    "),
            "first line keeps indent: {:?}",
            texts[0]
        );
        assert!(
            texts.last().unwrap().starts_with("    "),
            "duration continuation stays indented: {:?}",
            texts.last()
        );
        assert!(
            texts.last().unwrap().contains("1.91s"),
            "duration token present: {:?}",
            texts.last()
        );
    }

    #[test]
    fn short_indented_line_stays_one_row() {
        let line = Line::from(Span::raw("    hi".to_string()));
        let wrapped = wrap_lines(vec![line], 40);
        assert_eq!(plain(&wrapped), vec!["    hi".to_string()]);
    }

    #[test]
    fn wrap_drops_break_space_so_code_bg_does_not_fill_row() {
        let wrapped = wrap_lines(vec![code_chip_line()], 60);
        assert!(wrapped.len() >= 2, "expected wrap: {wrapped:?}");

        let first = line_text(&wrapped[0]);
        assert!(
            !first.ends_with(' '),
            "break space must be dropped, got {first:?}"
        );
        assert!(
            first.ends_with(',') || first.ends_with('t'),
            "unexpected wrap point: {first:?}"
        );

        if let Some(last) = wrapped[0].spans.last() {
            let only_space = last.content.chars().all(|c| c == ' ');
            assert!(
                !(only_space && last.style.bg.is_some()),
                "trailing code_bg space would paint to EOL: {:?}",
                last.content
            );
        }
    }

    #[test]
    fn wrap_at_chip_trailing_pad_drops_styled_space() {
        let code_bg = Color::Rgb(0x1c, 0x1c, 0x1c);
        let code = Style::default().bg(code_bg);
        let base = Style::default();
        let line = Line::from(vec![
            Span::styled("xxxxxxxxxx".to_string(), base),
            Span::styled(" chip ".to_string(), code),
            Span::styled("next".to_string(), base),
        ]);
        let wrapped = wrap_lines(vec![line], 16);
        assert_eq!(wrapped.len(), 2, "{}", line_text(&wrapped[0]));
        assert_eq!(line_text(&wrapped[0]), "xxxxxxxxxx chip");
        assert!(
            wrapped[0]
                .spans
                .iter()
                .all(|s| !(s.content.ends_with(' ') && s.style.bg.is_some())),
            "chip trailing pad must not remain on the first row: {:?}",
            wrapped[0].spans
        );
        assert_eq!(line_text(&wrapped[1]), "next");
    }
}
