//! Plan body layout with a stable screen-row → source-byte map for text selection.

use comb::{Line, Modifier, Span, Style};
use unicode_width::UnicodeWidthChar;

use crate::theme::Theme;

/// One display row of the plan body and the UTF-8 byte range it covers in `Plan.md`.
#[derive(Clone, Debug)]
pub struct PlanRowSpan {
    pub start: usize,
    pub end: usize,
}

/// Wrap plan markdown as simple styled lines, tracking source byte ranges per row.
pub fn layout_plan_body(body: &str, width: usize, theme: &Theme) -> (Vec<Line>, Vec<PlanRowSpan>) {
    let content_w = width.saturating_sub(2).max(1);
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    let mut byte = 0usize;

    for raw in body.split_inclusive('\n') {
        let line_start = byte;
        let text = raw.trim_end_matches('\n');
        let has_nl = raw.len() > text.len();
        let line_end = line_start + text.len();

        if text.is_empty() {
            lines.push(Line::from(Span::raw("  ")));
            spans.push(PlanRowSpan {
                start: line_start,
                end: line_start,
            });
        } else {
            let style = line_style(text, theme);
            for (rel, seg) in wrap_segments(text, content_w) {
                let abs_start = line_start + rel;
                let abs_end = abs_start + seg.len();
                lines.push(Line::from(vec![Span::raw("  "), Span::styled(seg, style)]));
                spans.push(PlanRowSpan {
                    start: abs_start,
                    end: abs_end,
                });
            }
        }

        byte = line_end + if has_nl { 1 } else { 0 };
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  _No plan yet._",
            Style::default().fg(theme.faint).add(Modifier::ITALIC),
        )));
        spans.push(PlanRowSpan { start: 0, end: 0 });
    }

    (lines, spans)
}

fn line_style(text: &str, theme: &Theme) -> Style {
    let t = text.trim_start();
    if t.starts_with('#') {
        Style::default().fg(theme.heading).add(Modifier::BOLD)
    } else if t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
        Style::default().fg(theme.fg)
    } else {
        Style::default().fg(theme.dim)
    }
}

/// Wrap `text` into display-width chunks; each entry is `(byte_offset_in_text, chunk)`.
fn wrap_segments(text: &str, width: usize) -> Vec<(usize, String)> {
    let width = width.max(1);
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    if chars.is_empty() {
        return vec![(0, String::new())];
    }

    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let line_start = chars[i].0;
        let mut col = 0usize;
        let mut j = i;
        let mut last_space = None::<usize>; // index into chars

        while j < chars.len() {
            let (_, ch) = chars[j];
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if col + cw > width && j > i {
                break;
            }
            if ch == ' ' {
                last_space = Some(j);
            }
            col += cw;
            j += 1;
        }

        let break_at = if j < chars.len() {
            last_space.filter(|&sp| sp > i).unwrap_or(j)
        } else {
            j
        };
        let end_byte = if break_at < chars.len() {
            chars[break_at].0
        } else {
            text.len()
        };
        out.push((
            line_start,
            text[line_start..end_byte.max(line_start)].to_string(),
        ));

        // Skip the breaking space when we wrapped on a word boundary.
        i = break_at;
        if i < chars.len() && chars[i].1 == ' ' {
            i += 1;
        }
        if i == break_at && j > i {
            // Hard break mid-word: advance at least one char.
            i = j.max(i + 1);
        }
    }
    out
}

/// Map a column within a body display row (after the 2-space indent) to a byte offset.
pub fn col_to_offset(body: &str, span: &PlanRowSpan, col: usize) -> usize {
    if span.start >= span.end || col == 0 {
        return span.start;
    }
    let slice = &body[span.start..span.end];
    let mut w = 0usize;
    for (i, ch) in slice.char_indices() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > col {
            return span.start + i;
        }
        w += cw;
        if w >= col {
            return span.start + i + ch.len_utf8();
        }
    }
    span.end
}

/// A plan body highlight: gray while selecting / before a note, blue after MARK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkTone {
    /// Drag or mark without a saved note.
    Pending,
    /// Mark with a note (committed via MARK / Enter).
    Noted,
}

/// Paint highlight backgrounds onto plan body lines.
/// Only the overlapping source bytes are tinted — no extra leading spaces.
/// Noted (blue) wins over pending (gray) where ranges overlap.
pub fn paint_highlights(
    lines: &mut [Line],
    spans: &[PlanRowSpan],
    body: &str,
    ranges: &[(usize, usize, MarkTone)],
    theme: &Theme,
) {
    let dim = theme.dim;
    let pending = Style::default()
        .fg(theme.mark_pending_fg)
        .bg(theme.mark_pending_bg);
    let noted = Style::default().fg(theme.mark_fg).bg(theme.mark_bg);
    let base = Style::default().fg(dim);

    for (row, span) in spans.iter().enumerate() {
        if row >= lines.len() || span.start >= span.end || span.end > body.len() {
            continue;
        }
        let cuts: Vec<(usize, usize, MarkTone)> = ranges
            .iter()
            .filter_map(|&(a, b, tone)| {
                if a < b && a < span.end && b > span.start {
                    Some((a.max(span.start), b.min(span.end), tone))
                } else {
                    None
                }
            })
            .collect();
        if cuts.is_empty() {
            continue;
        }
        let merged = flatten_mark_cuts(&cuts);
        if merged.is_empty() {
            continue;
        }

        let mut spans_out = vec![Span::raw("  ")];
        let mut cursor = span.start;
        for (a, b, tone) in merged {
            if cursor < a {
                spans_out.push(Span::styled(
                    body[cursor..a.min(body.len())].to_string(),
                    base,
                ));
            }
            let hi = match tone {
                MarkTone::Pending => pending,
                MarkTone::Noted => noted,
            };
            spans_out.push(Span::styled(body[a..b.min(body.len())].to_string(), hi));
            cursor = b;
        }
        if cursor < span.end {
            spans_out.push(Span::styled(
                body[cursor..span.end.min(body.len())].to_string(),
                base,
            ));
        }
        lines[row] = Line::from(spans_out);
    }
}

/// Split overlapping cuts into non-overlapping segments; Noted beats Pending.
fn flatten_mark_cuts(cuts: &[(usize, usize, MarkTone)]) -> Vec<(usize, usize, MarkTone)> {
    let mut points: Vec<usize> = cuts.iter().flat_map(|&(a, b, _)| [a, b]).collect();
    points.sort_unstable();
    points.dedup();
    let mut out: Vec<(usize, usize, MarkTone)> = Vec::new();
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if a >= b {
            continue;
        }
        let mut tone: Option<MarkTone> = None;
        for &(cs, ce, t) in cuts {
            if cs < b && ce > a {
                tone = Some(match (tone, t) {
                    (Some(MarkTone::Noted), _) | (_, MarkTone::Noted) => MarkTone::Noted,
                    _ => MarkTone::Pending,
                });
            }
        }
        if let Some(t) = tone {
            if let Some(last) = out.last_mut() {
                if last.1 == a && last.2 == t {
                    last.1 = b;
                    continue;
                }
            }
            out.push((a, b, t));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    #[test]
    fn wraps_and_maps_offsets() {
        let theme = Theme::gray();
        let body = "hello world from plan\n";
        let (lines, spans) = layout_plan_body(body, 12, &theme);
        assert!(!lines.is_empty());
        assert_eq!(spans.len(), lines.len());
        assert!(spans[0].start < spans[0].end || body.trim().is_empty());
        let off = col_to_offset(body, &spans[0], 0);
        assert_eq!(off, spans[0].start);
    }

    #[test]
    fn empty_body_has_placeholder() {
        let theme = Theme::gray();
        let (lines, spans) = layout_plan_body("", 40, &theme);
        assert_eq!(lines.len(), 1);
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn pending_is_gray_noted_is_blue() {
        let theme = Theme::gray();
        let body = "hello world\n";
        let (mut lines, spans) = layout_plan_body(body, 40, &theme);
        paint_highlights(
            &mut lines,
            &spans,
            body,
            &[(0, 5, MarkTone::Pending), (6, 11, MarkTone::Noted)],
            &theme,
        );
        let line = &lines[0];
        let gray = line
            .spans
            .iter()
            .any(|s| s.content.contains("hello") && s.style.bg == Some(theme.mark_pending_bg));
        let blue = line
            .spans
            .iter()
            .any(|s| s.content.contains("world") && s.style.bg == Some(theme.mark_bg));
        assert!(gray, "pending selection must be gray, got {line:?}");
        assert!(blue, "noted mark must be blue, got {line:?}");
        assert_ne!(theme.mark_bg, theme.mark_pending_bg);
    }
}
