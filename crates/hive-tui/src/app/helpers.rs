//! Text, selection, and path helpers shared by App submodules.

use super::*;

pub(super) fn normalized_points(
    a: AssistantPoint,
    b: AssistantPoint,
) -> (AssistantPoint, AssistantPoint) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

pub(super) fn assistant_row_selected_cols(
    first: AssistantPoint,
    last: AssistantPoint,
    row: usize,
    text: &str,
) -> Option<(usize, usize)> {
    let width = display_width(text);
    if width == 0 {
        return Some((0, 0));
    }
    let start = if row == first.row {
        first.col.min(width)
    } else {
        0
    };
    let end = if row == last.row {
        display_char_end(text, last.col).min(width)
    } else {
        width
    };
    (start < end).then_some((start, end))
}

pub(super) fn display_width(text: &str) -> usize {
    use unicode_width::UnicodeWidthChar;

    text.chars()
        .map(|ch| UnicodeWidthChar::width(ch).unwrap_or(0))
        .sum()
}

/// Start display column of the glyph occupying `col`.
pub(super) fn display_cell_start(text: &str, col: usize) -> usize {
    use unicode_width::UnicodeWidthChar;

    let mut x = 0;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width == 0 {
            continue;
        }
        if col < x + width {
            return x;
        }
        x += width;
    }
    x
}

/// End display column of the glyph occupying `col`.
pub(super) fn display_char_end(text: &str, col: usize) -> usize {
    use unicode_width::UnicodeWidthChar;

    let mut x = 0;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width == 0 {
            continue;
        }
        if col < x + width {
            return x + width;
        }
        x += width;
    }
    x
}

pub(super) fn slice_display_range(text: &str, start: usize, end: usize) -> String {
    use unicode_width::UnicodeWidthChar;

    if start >= end {
        return String::new();
    }
    let mut x = 0;
    let mut byte_start = None;
    let mut byte_end = 0;
    let mut selected = false;
    for (byte, ch) in text.char_indices() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width == 0 {
            if selected {
                byte_end = byte + ch.len_utf8();
            }
            continue;
        }
        if x >= end && selected {
            break;
        }
        if x < end && x + width > start {
            byte_start.get_or_insert(byte);
            byte_end = byte + ch.len_utf8();
            selected = true;
        }
        x += width;
    }
    byte_start
        .map(|from| text.get(from..byte_end).unwrap_or("").to_string())
        .unwrap_or_default()
}

pub(super) fn trim_horizontal_end(text: &mut String) {
    while text.ends_with(' ') || text.ends_with('\t') {
        text.pop();
    }
}

pub(super) fn clean_selected_text(text: &str) -> String {
    text.split('\n')
        .map(|line| line.trim_end_matches([' ', '\t']))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}

/// Tools whose activity belongs in a Subagent transcript card, not a tool row.
pub(super) fn is_subagent_tool(name: &str) -> bool {
    matches!(name, "verify_project" | "spawn_subagent" | "spawn_swarm")
}

pub(super) fn is_plan_write(name: &str, args_preview: &str) -> bool {
    name == "write_plan"
        || (matches!(name, "write_file" | "edit_file")
            && (args_preview.contains("Plan.md") || args_preview.contains(".hive/Plan")))
}

/// Special tools already have purpose-built UI. Keeping a second generic row
/// for them makes live and resumed transcripts noisy and inconsistent.
pub(super) fn shows_generic_tool_card(name: &str, args_preview: &str) -> bool {
    !is_plan_write(name, args_preview)
        && !is_subagent_tool(name)
        && name != "switch_mode"
        && name != "set_todos"
        && !hive_core::terminal::is_terminal_tool(name)
}

pub(super) fn short_tokens(n: u64) -> String {
    if n >= 1000 {
        format!("{:.0}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

pub(super) fn is_image_path(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
    )
}

pub(super) fn media_type_for(path: &str) -> String {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        _ => "image/png",
    }
    .to_string()
}

pub(super) fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Parse pasted / drag-and-dropped text into candidate local file paths.
///
/// Terminals emit dropped files in inconsistent ways: a bare path, a quoted
/// path, a path with backslash-escaped spaces (kitty / iTerm2), or one or more
/// `file://` URIs with percent-encoding (GNOME / VTE). This normalizes all of
/// those. Returns `None` when the text clearly isn't just path(s) — e.g.
/// multi-line prose — so the caller can treat it as an ordinary text paste.
pub(super) fn drop_path_candidates(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // A `file://` URI list — possibly several, one per line (text/uri-list).
    if trimmed.starts_with("file://") {
        let mut out = Vec::new();
        for tok in trimmed.split_whitespace() {
            let rest = tok.strip_prefix("file://")?;
            // Skip an optional host component: everything up to the first '/'.
            let slash = rest.find('/')?;
            out.push(decode_file_uri_path(&rest[slash..]));
        }
        return (!out.is_empty()).then_some(out);
    }

    // Otherwise a single path. Multi-line text is prose, not a dropped path.
    if trimmed.contains('\n') {
        return None;
    }
    let unquoted = strip_matching_quotes(trimmed);
    Some(vec![unescape_backslashes(unquoted)])
}

/// Strip one layer of matching surrounding quotes (`"…"` or `'…'`).
pub(super) fn strip_matching_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return s.get(1..s.len().saturating_sub(1)).unwrap_or(s);
        }
    }
    s
}

/// Drop shell escape backslashes (`My\ Photos` → `My Photos`).
pub(super) fn unescape_backslashes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Percent-decode a `file://` path, normalizing a Windows `/C:/…` drive prefix.
pub(super) fn decode_file_uri_path(path: &str) -> String {
    let decoded = percent_decode(path);
    let bytes = decoded.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        return decoded[1..].to_string();
    }
    decoded
}

/// Minimal percent-decoding for `file://` URIs (`%20` → space, etc.).
pub(super) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
