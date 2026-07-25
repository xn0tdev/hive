//! Lightweight markdown → comb lines. Supports fenced code, headings, bullets,
//! GFM tables, horizontal rules, and inline `code` / **bold** / *italic*.

use comb::{highlight, Line, Modifier, Span, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Theme;

/// Render markdown into styled logical lines.
///
/// `width` is the content column budget (already indented). Tables are laid out
/// to fit it so columns stay aligned and long cells wrap instead of clipping.
pub fn render(text: &str, theme: &Theme, width: usize) -> Vec<Line> {
    let raw_lines: Vec<&str> = text.lines().collect();
    let mut lines = Vec::new();
    let mut i = 0usize;
    let mut in_code = false;
    let mut code_buf: Vec<String> = Vec::new();
    let mut code_label = String::from("code");

    while i < raw_lines.len() {
        let raw = raw_lines[i];
        let trimmed = raw.trim_start();

        if trimmed.starts_with("```") {
            if in_code {
                flush_code_block(&mut lines, &code_buf, &code_label, theme);
                code_buf.clear();
                in_code = false;
            } else {
                in_code = true;
                let label = trimmed.trim_start_matches('`').trim();
                code_label = if label.is_empty() {
                    "code".into()
                } else {
                    label.to_string()
                };
            }
            i += 1;
            continue;
        }

        if in_code {
            code_buf.push(raw.to_string());
            i += 1;
            continue;
        }

        // Classic ASCII `+---+` tables → same clean box-drawing layout.
        if let Some(owned_rows) = take_ascii_table(&raw_lines, &mut i) {
            let refs: Vec<&str> = owned_rows.iter().map(String::as_str).collect();
            flush_table(&mut lines, &refs, theme, width);
            continue;
        }

        // GFM table: header + separator, or a run of pipe-rows (models often
        // drop the separator / insert blank lines mid-table).
        if let Some(table_rows) = take_table(&raw_lines, &mut i) {
            flush_table(&mut lines, &table_rows, theme, width);
            continue;
        }

        // Orphan `|` left by models / bad wraps — drop, don't paint as content.
        if !trimmed.is_empty() && trimmed.chars().all(|c| c == '|' || c.is_whitespace()) {
            i += 1;
            continue;
        }

        if is_hr(trimmed) {
            lines.push(Line::from(Span::styled(
                "─".repeat(24),
                Style::default().fg(theme.faint),
            )));
            i += 1;
            continue;
        }

        if let Some(level) = heading_level(trimmed) {
            let content = trimmed[level..].trim_start();
            lines.push(Line::from(Span::styled(
                content.to_string(),
                Style::default().fg(theme.heading).add(Modifier::BOLD),
            )));
            i += 1;
            continue;
        }

        let (bullet, rest): (Option<String>, &str) = if let Some(r) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            (Some("• ".into()), r)
        } else if let Some((n, r)) = numbered_item(trimmed) {
            (Some(n), r)
        } else {
            (None, raw)
        };

        let mut spans = Vec::new();
        if let Some(b) = bullet {
            spans.push(Span::styled(b, Style::default().fg(theme.accent)));
        }
        spans.extend(inline(rest, theme));
        lines.push(Line::from(spans));
        i += 1;
    }

    if in_code {
        flush_code_block(&mut lines, &code_buf, &code_label, theme);
    }

    lines
}

/// Consume a classic ASCII bordered table (`+---+` / `| cell |`) into pipe-rows
/// suitable for [`flush_table`]. Skips border lines; advances `i` past the block.
fn take_ascii_table(raw_lines: &[&str], i: &mut usize) -> Option<Vec<String>> {
    let start = *i;
    let first = raw_lines.get(start)?.trim();
    if !is_ascii_table_border(first) {
        return None;
    }

    let mut rows: Vec<String> = Vec::new();
    let mut j = start;
    while j < raw_lines.len() {
        let t = raw_lines[j].trim();
        if t.is_empty() {
            break;
        }
        if is_ascii_table_border(t) {
            j += 1;
            continue;
        }
        if looks_like_table_row(t) {
            // Normalize to a plain pipe-row (no leading/trailing spaces noise).
            rows.push(t.to_string());
            j += 1;
            continue;
        }
        break;
    }

    if rows.len() < 2 {
        return None;
    }
    *i = j;
    Some(rows)
}

fn is_ascii_table_border(s: &str) -> bool {
    let t = s.trim();
    if t.len() < 3 || !t.contains('+') || !t.contains('-') {
        return false;
    }
    t.chars()
        .all(|c| c == '+' || c == '-' || c == '=' || c == '|' || c.is_whitespace())
}

/// Consume a markdown table starting at `i`, advancing `i` past it.
fn take_table<'a>(raw_lines: &[&'a str], i: &mut usize) -> Option<Vec<&'a str>> {
    let start = *i;
    let first = raw_lines.get(start)?.trim();
    if !looks_like_table_row(first) {
        return None;
    }

    let next = raw_lines.get(start + 1).map(|s| s.trim()).unwrap_or("");
    let has_sep = is_table_separator(next);

    // Need either a GFM separator, or at least two pipe-rows in a row.
    if !has_sep {
        let second_is_row = looks_like_table_row(next);
        if !second_is_row {
            return None;
        }
    }

    let mut rows = vec![first];
    *i = start + 1;
    if has_sep {
        *i += 1; // skip separator
    }

    while *i < raw_lines.len() {
        let t = raw_lines[*i].trim();
        if t.is_empty() {
            // Allow a single blank inside a table; stop on a second blank or
            // when the following line is not a row.
            let following = raw_lines.get(*i + 1).map(|s| s.trim()).unwrap_or("");
            if looks_like_table_row(following) {
                *i += 1;
                continue;
            }
            break;
        }
        if is_table_separator(t) {
            *i += 1;
            continue;
        }
        if looks_like_table_row(t) {
            rows.push(t);
            *i += 1;
            continue;
        }
        break;
    }

    if rows.len() < 2 && !has_sep {
        *i = start;
        return None;
    }
    Some(rows)
}

fn looks_like_table_row(s: &str) -> bool {
    let t = s.trim();
    !is_table_separator(t) && t.chars().filter(|c| *c == '|').count() >= 2
}

fn is_table_separator(s: &str) -> bool {
    let t = s.trim().trim_matches('|');
    !t.is_empty()
        && t.chars()
            .all(|c| c == '-' || c == ':' || c == '|' || c.is_whitespace())
        && t.contains('-')
}

fn is_hr(s: &str) -> bool {
    let t = s.trim();
    t.len() >= 3 && t.chars().all(|c| c == '-' || c == '*' || c == '_')
}

fn numbered_item(s: &str) -> Option<(String, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i >= bytes.len() || bytes[i] != b'.' {
        return None;
    }
    let rest = s[i + 1..].strip_prefix(' ')?;
    Some((format!("{}. ", &s[..i]), rest))
}

fn split_cells(row: &str) -> Vec<String> {
    // Split on `|` but ignore pipes inside `code` spans.
    let t = row.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);

    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut in_code = false;
    for ch in t.chars() {
        if ch == '`' {
            in_code = !in_code;
            cur.push(ch);
        } else if ch == '|' && !in_code {
            cells.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| display_width(&s.content)).sum()
}

/// Visible width of cell text after markdown markers are applied the same way
/// `inline` does (code spans gain padding spaces; `*` markers disappear).
fn cell_display_width(cell: &str) -> usize {
    spans_width(&inline_plain_measure(cell))
}

/// Measure-only inline parse — same geometry as `inline`, default styles.
fn inline_plain_measure(text: &str) -> Vec<Span> {
    // Reuse inline with a throwaway theme of blacks; only content widths matter.
    // Cheaper: duplicate the geometry without Theme.
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        if c == '`' {
            if let Some(j) = find(&chars, i + 1, '`') {
                let content: String = chars[i + 1..j].iter().collect();
                out.push(' ');
                out.push_str(&content);
                out.push(' ');
                i = j + 1;
                continue;
            }
        } else if c == '*' && i + 1 < n && chars[i + 1] == '*' {
            if let Some(j) = find_double(&chars, i + 2) {
                out.extend(chars[i + 2..j].iter());
                i = j + 2;
                continue;
            }
        } else if c == '*' {
            if let Some(j) = find(&chars, i + 1, '*') {
                out.extend(chars[i + 1..j].iter());
                i = j + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    vec![Span::raw(out)]
}

fn flush_table(lines: &mut Vec<Line>, rows: &[&str], theme: &Theme, width: usize) {
    if rows.is_empty() {
        return;
    }
    let parsed: Vec<Vec<String>> = rows.iter().map(|r| split_cells(r)).collect();
    let cols = parsed.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        return;
    }

    let header_empty = parsed
        .first()
        .map(|r| r.iter().all(|c| c.trim().is_empty()))
        .unwrap_or(true);

    // Natural column widths from rendered cell geometry.
    let mut natural = vec![1usize; cols];
    for (ri, row) in parsed.iter().enumerate() {
        if ri == 0 && header_empty {
            continue;
        }
        for (i, cell) in row.iter().enumerate() {
            natural[i] = natural[i].max(cell_display_width(cell)).max(1);
        }
    }

    // Full box grid: `│ pad content pad │` per cell; chrome eats verticals + pad.
    // Every cell is clamped to its column width so `│` always sits under `┬`/`┼`.
    const PAD: usize = 1;
    let chrome = (cols + 1) + cols * (2 * PAD); // │…│…│ + padding spaces
    let width = width.max(chrome + cols);
    let content_budget = width.saturating_sub(chrome).max(cols);
    let widths = fit_columns(&natural, cols, 0, content_budget);

    // One style for the whole frame so junctions read as continuous lines.
    let chrome_st = Style::default().fg(theme.dim);
    let head = Style::default().fg(theme.heading).add(Modifier::BOLD);
    let body = Style::default().fg(theme.fg);

    let border = |left: char, mid: char, right: char| -> Line {
        let mut s = String::new();
        s.push(left);
        for (i, w) in widths.iter().enumerate() {
            if i > 0 {
                s.push(mid);
            }
            s.push_str(&"─".repeat(w + 2 * PAD));
        }
        s.push(right);
        Line::from(Span::styled(s, chrome_st))
    };

    let visible: Vec<&Vec<String>> = parsed
        .iter()
        .enumerate()
        .filter(|(ri, _)| !(*ri == 0 && header_empty))
        .map(|(_, r)| r)
        .collect();
    if visible.is_empty() {
        return;
    }

    lines.push(border('┌', '┬', '┐'));

    for (vi, row) in visible.iter().enumerate() {
        let is_header = vi == 0 && !header_empty;
        let st = if is_header { head } else { body };

        let mut cell_lines: Vec<Vec<String>> = Vec::with_capacity(cols);
        let mut row_h = 1usize;
        for (ci, w) in widths.iter().enumerate() {
            let cell = row.get(ci).map(String::as_str).unwrap_or("");
            // Always wrap to column width (no overflow that shifts later `│`).
            let wrapped = wrap_cell(cell, *w);
            row_h = row_h.max(wrapped.len().max(1));
            cell_lines.push(wrapped);
        }

        for li in 0..row_h {
            let mut spans = vec![Span::styled("│", chrome_st)];
            for (ci, w) in widths.iter().enumerate() {
                spans.push(Span::styled(" ".repeat(PAD), chrome_st));
                let piece = cell_lines[ci].get(li).map(String::as_str).unwrap_or("");
                // Plain padded text keeps column geometry exact (styled inline
                // can disagree with wrap width when markdown markers differ).
                let text = pad_to_width(piece, *w);
                spans.push(Span::styled(text, st));
                spans.push(Span::styled(" ".repeat(PAD), chrome_st));
                spans.push(Span::styled("│", chrome_st));
            }
            lines.push(Line::from(spans));
        }

        if vi + 1 < visible.len() {
            lines.push(border('├', '┼', '┤'));
        }
    }

    lines.push(border('└', '┴', '┘'));
}

/// Truncate/pad by display width so a cell is exactly `width` columns.
fn pad_to_width(s: &str, width: usize) -> String {
    let width = width.max(1);
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > width {
            break;
        }
        out.push(ch);
        w += cw;
    }
    if w < width {
        out.push_str(&" ".repeat(width - w));
    }
    out
}

/// Shrink natural column widths so `sum + seps` fits in `budget`.
fn fit_columns(natural: &[usize], cols: usize, sep_w: usize, budget: usize) -> Vec<usize> {
    let seps = sep_w * cols.saturating_sub(1);
    let mut widths = natural.to_vec();
    let total = |widths: &[usize]| widths.iter().sum::<usize>() + seps;
    if total(&widths) <= budget {
        return widths;
    }

    // Prefer shrinking trailing columns first; keep col0 readable (≥8 when possible).
    let min_first = 8.min(natural[0]);
    let min_other = 6usize;

    // Cap first column.
    if cols >= 1 {
        let max_first = (budget.saturating_sub(seps + min_other * (cols - 1))).max(min_first);
        widths[0] = widths[0].min(max_first).max(min_first.min(natural[0]));
    }

    // Give remaining budget to the last column; squeeze middle cols to min.
    if cols >= 2 {
        for w in widths.iter_mut().take(cols - 1).skip(1) {
            *w = (*w).min(min_other.max(8));
        }
        let used: usize = widths[..cols - 1].iter().sum::<usize>() + seps;
        widths[cols - 1] = budget.saturating_sub(used).max(min_other);
    }

    // Final pass: if still over, shrink from the end.
    while total(&widths) > budget {
        let mut shrunk = false;
        for w in widths.iter_mut().rev() {
            if *w > min_other {
                *w -= 1;
                shrunk = true;
                break;
            }
        }
        if !shrunk {
            break;
        }
    }
    widths
}

fn wrap_cell(cell: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    // Strip markdown for wrapping geometry, then wrap by display width.
    let plain = inline_plain_measure(cell)
        .into_iter()
        .map(|s| s.content)
        .collect::<String>();
    if plain.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    let mut last_space: Option<(usize, usize)> = None; // (byte-ish char idx, width at space)

    for ch in plain.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if cur_w + cw > width && !cur.is_empty() {
            if let Some((idx, _)) = last_space {
                let rest: String = cur.chars().skip(idx + 1).collect();
                let kept: String = cur.chars().take(idx).collect();
                lines.push(kept);
                cur = rest;
                cur_w = display_width(&cur);
                last_space = None;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
        }
        if ch == ' ' {
            last_space = Some((cur.chars().count(), cur_w));
        }
        cur.push(ch);
        cur_w += cw;
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

fn flush_code_block(lines: &mut Vec<Line>, body: &[String], label: &str, theme: &Theme) {
    if body.is_empty() {
        return;
    }

    let block_w = body
        .iter()
        .map(|l| display_width(l))
        .max()
        .unwrap_or(0)
        .max(8);
    let pad_st = Style::default().fg(theme.code_fg).bg(theme.code_bg);
    let ht = code_highlight_theme(theme);
    let lang = highlight::lang_from_info(label);
    let source = body.join("\n");
    let mut highlighted = highlight::highlight(&source, lang, &ht);
    // `str::lines` drops a trailing empty row that the fence body still has.
    while highlighted.len() < body.len() {
        highlighted.push(Line::from(Span::styled(String::new(), ht.text)));
    }

    for line in highlighted {
        let mut spans = vec![Span::styled("  ", pad_st)];
        let mut w = 2usize;
        for s in line.spans {
            if s.content.is_empty() {
                continue;
            }
            w += display_width(&s.content);
            spans.push(s);
        }
        let pad = (block_w + 2).saturating_sub(w);
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), pad_st));
        }
        lines.push(Line::from(spans));
    }
}

/// Token colours from the UI palette, with `code_bg` under every span so the
/// block reads as one soft rectangle (no box-drawing chrome).
fn code_highlight_theme(theme: &Theme) -> highlight::HighlightTheme {
    let bg = theme.code_bg;
    let mk = |fg| Style::default().fg(fg).bg(bg);
    highlight::HighlightTheme {
        text: mk(theme.code_fg),
        keyword: mk(theme.accent),
        string: mk(theme.warn),
        comment: mk(theme.faint),
        number: mk(theme.ok),
        type_name: mk(theme.tool),
        function: mk(theme.heading),
        punctuation: mk(theme.dim),
        line_number: mk(theme.faint),
    }
}

/// Plain lines for live streaming (no markdown yet).
pub fn plain(text: &str, theme: &Theme) -> Vec<Line> {
    text.split('\n')
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(theme.fg))))
        .collect()
}

fn heading_level(s: &str) -> Option<usize> {
    let mut count = 0;
    for ch in s.chars() {
        if ch == '#' {
            count += 1;
        } else {
            break;
        }
    }
    if count > 0 && count <= 6 && s.chars().nth(count) == Some(' ') {
        Some(count)
    } else {
        None
    }
}

fn find(chars: &[char], start: usize, pat: char) -> Option<usize> {
    (start..chars.len()).find(|&j| chars[j] == pat)
}

fn find_double(chars: &[char], start: usize) -> Option<usize> {
    let mut j = start;
    while j + 1 < chars.len() {
        if chars[j] == '*' && chars[j + 1] == '*' {
            return Some(j);
        }
        j += 1;
    }
    None
}

fn inline(text: &str, theme: &Theme) -> Vec<Span> {
    inline_with(text, theme, true)
}

fn inline_with(text: &str, theme: &Theme, code_bg: bool) -> Vec<Span> {
    let base = Style::default().fg(theme.fg);
    let code = if code_bg {
        Style::default().fg(theme.tool).bg(theme.code_bg)
    } else {
        Style::default().fg(theme.tool)
    };
    let bold = base.add(Modifier::BOLD);
    let italic = base.add(Modifier::ITALIC);

    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut spans: Vec<Span> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;

    while i < n {
        let c = chars[i];
        if c == '`' {
            if let Some(j) = find(&chars, i + 1, '`') {
                flush(&mut buf, &mut spans, base);
                let content: String = chars[i + 1..j].iter().collect();
                let lead = content.len() - content.trim_start().len();
                let trail = content.len() - content.trim_end().len();
                if lead > 0 {
                    spans.push(Span::styled(content[..lead].to_string(), base));
                }
                spans.push(Span::styled(
                    format!(" {} ", &content[lead..content.len() - trail]),
                    code,
                ));
                if trail > 0 {
                    spans.push(Span::styled(
                        content[content.len() - trail..].to_string(),
                        base,
                    ));
                }
                i = j + 1;
                continue;
            }
        } else if c == '*' && i + 1 < n && chars[i + 1] == '*' {
            if let Some(j) = find_double(&chars, i + 2) {
                flush(&mut buf, &mut spans, base);
                let content: String = chars[i + 2..j].iter().collect();
                spans.push(Span::styled(content, bold));
                i = j + 2;
                continue;
            }
        } else if c == '*' {
            if let Some(j) = find(&chars, i + 1, '*') {
                flush(&mut buf, &mut spans, base);
                let content: String = chars[i + 1..j].iter().collect();
                spans.push(Span::styled(content, italic));
                i = j + 1;
                continue;
            }
        }
        buf.push(c);
        i += 1;
    }

    flush(&mut buf, &mut spans, base);
    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    spans
}

fn flush(buf: &mut String, spans: &mut Vec<Span>, style: Style) {
    if !buf.is_empty() {
        spans.push(Span::styled(std::mem::take(buf), style));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn col_positions(line: &str) -> Vec<usize> {
        let mut cols = Vec::new();
        let mut w = 0usize;
        for ch in line.chars() {
            if ch == '│' {
                cols.push(w);
            }
            w += UnicodeWidthChar::width(ch).unwrap_or(0);
        }
        cols
    }

    #[test]
    fn table_renders_without_pipe_junk() {
        let md = "\
| Check | Status |
|-------|--------|
| Build | ok |
| Tests | 4 passed |
";
        let out = render(md, &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("Check"));
        assert!(t.contains("Build"));
        assert!(t.contains("│"), "columns separated by box glyph");
        assert!(!t.contains("|---"), "raw separator must not appear");
        assert!(!t.contains("| Build"), "raw markdown pipes must not appear");
    }

    #[test]
    fn table_columns_align_across_rows() {
        let md = "\
| Аспект | Оценка |
|--------|--------|
| Архитектура | Чистое разделение |
| Саморегистрация тулов | Через inventory |
";
        let out = render(md, &Theme::gray(), 80);
        let rows: Vec<String> = text(&out)
            .lines()
            .filter(|l| l.contains('│'))
            .map(|s| s.to_string())
            .collect();
        assert!(rows.len() >= 3, "header + body rows: {rows:?}");
        let pos0 = col_positions(&rows[0]);
        for r in &rows[1..] {
            assert_eq!(
                col_positions(r),
                pos0,
                "misaligned:\n  {}\n  {}",
                rows[0],
                r
            );
        }
        // Top border spans full table width (not just first col).
        let plain = text(&out);
        let rule = plain.lines().find(|l| l.starts_with('┌')).unwrap();
        assert!(display_width(rule) >= display_width(&rows[0]));
    }

    #[test]
    fn table_survives_blank_line_mid_body() {
        let md = "\
| A | B |
|---|---|
| one | x |

| two | y |
";
        let out = render(md, &Theme::gray(), 40);
        let t = text(&out);
        assert!(t.contains("one"));
        assert!(t.contains("two"));
        assert!(!t.contains("| two"));
    }

    #[test]
    fn empty_header_cells_still_align() {
        let md = "\
| | |
|---|---|
| Компиляция | Чисто |
";
        let out = render(md, &Theme::gray(), 40);
        let t = text(&out);
        assert!(t.contains("Компиляция"));
        assert!(t.contains("Чисто"));
        assert!(!t.contains("|---"));
    }

    #[test]
    fn code_fence_still_works() {
        let md = "```rust\nfn main() {}\n```";
        let out = render(md, &Theme::gray(), 40);
        let t = text(&out);
        assert!(t.contains("fn main"));
        assert!(!t.contains('┌'), "no box chrome: {t}");
        assert!(!t.contains('└'), "no box chrome: {t}");
        let body = out
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("fn")))
            .expect("rust body");
        assert!(
            body.spans.len() > 2,
            "expected syntax-coloured spans, got {:?}",
            body.spans.len()
        );
        assert!(body
            .spans
            .iter()
            .all(|s| s.style.bg == Some(Theme::gray().code_bg)));
    }

    #[test]
    fn long_cells_wrap_inside_column() {
        let md = "\
| K | V |
|---|---|
| short | this is a very long value that should wrap within the column budget |
";
        let out = render(md, &Theme::gray(), 36);
        let plain = text(&out);
        let rows: Vec<_> = plain.lines().filter(|l| l.contains('│')).collect();
        assert!(
            rows.len() >= 3,
            "wrapped body should span multiple lines: {rows:?}"
        );
        let pos = col_positions(rows[0]);
        for r in &rows {
            assert_eq!(col_positions(r), pos);
            assert!(
                display_width(r) <= 36,
                "row too wide: {} ({})",
                r,
                display_width(r)
            );
        }
    }

    #[test]
    fn ascii_plus_tables_become_box_grid() {
        let md = "\
+--------+--------+
| Check  | Status |
+--------+--------+
| Build  | ok     |
| Tests  | pass   |
+--------+--------+
";
        let out = render(md, &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains("Check"));
        assert!(t.contains("Build"));
        assert!(t.contains('│'));
        assert!(t.contains('┌'), "full box grid top: {t}");
        assert!(t.contains('┼'), "row/col junctions: {t}");
        assert!(t.contains('└'), "full box grid bottom: {t}");
        assert!(!t.contains('+'), "ASCII + borders must not appear: {t}");
        assert!(!t.contains("|---"), "{t}");
    }

    #[test]
    fn gfm_table_uses_full_box_grid() {
        let md = "\
| Имя | Возраст |
|-----|---------|
| Анна | 28 |
| Борис | 34 |
";
        let out = render(md, &Theme::gray(), 80);
        let t = text(&out);
        assert!(t.contains('┌') && t.contains('┬') && t.contains('┐'), "{t}");
        assert!(t.contains('├') && t.contains('┼') && t.contains('┤'), "{t}");
        assert!(t.contains('└') && t.contains('┴') && t.contains('┘'), "{t}");
        // No floating rules without junctions.
        assert!(!t.lines().any(|l| {
            let t = l.trim();
            !t.is_empty() && t.chars().all(|c| c == '─')
        }));
    }

    #[test]
    fn inline_code_trim_whitespace_outside_bg() {
        let theme = Theme::gray();
        let spans = inline_with("`  firectl signin  `", &theme, true);
        // Leading whitespace must be a separate base span (no bg).
        let lead = spans
            .iter()
            .find(|s| s.content.starts_with(' ') && !s.content.starts_with("  firectl"));
        assert!(
            lead.is_some_and(|s| s.style.bg.is_none()),
            "leading ws must not have code_bg: {spans:?}"
        );
        // The code chip itself keeps code_bg but only on the trimmed text + pad.
        let chip = spans.iter().find(|s| s.content.contains("firectl"));
        assert!(
            chip.is_some_and(|s| s.style.bg == Some(theme.code_bg)),
            "code text must have code_bg: {spans:?}"
        );
        // Trailing whitespace must also be outside the bg.
        let trail = spans
            .iter()
            .find(|s| s.content.chars().all(|c| c == ' ') && s.style.bg.is_none());
        assert!(
            trail.is_some(),
            "trailing ws must be a separate no-bg span: {spans:?}"
        );
    }
}
