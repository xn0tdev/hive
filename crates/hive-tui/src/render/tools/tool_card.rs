//! Regular tool rows — no strip background.

use comb::{Line, Modifier, Span, Style};

use crate::app::state::{ToolCard, ToolStatus};
use crate::app::App;
use crate::render::wrap;

/// Minimal Claude-style tool lines; duration stays on the header so it never
/// orphans at column 0 when a long output line wraps.
///
/// ```text
///   Reading src/main.rs  · 0.2s
///   $ cargo check  · 1.9s
///   … 23 output lines hidden
///     preview…
/// ```
pub(crate) fn tool_lines(card: &ToolCard, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let running = card.status == ToolStatus::Running;
    let err = card.status == ToolStatus::Err;
    let verb_st = Style::default()
        .fg(if err { theme.err } else { theme.fg })
        .add(Modifier::BOLD);
    let args_st = Style::default().fg(theme.dim);
    let meta_st = Style::default().fg(theme.faint);

    // Only show timing for tools that can genuinely take time.
    let show_timing = !matches!(
        card.name.as_str(),
        "read_file" | "list_dir" | "glob" | "grep" | "read_skill"
    );
    let dur_suffix = if show_timing {
        format!("  · {}", format_tool_secs(card.secs()))
    } else {
        String::new()
    };
    let mut out = vec![tool_header_line(
        card,
        running,
        verb_st,
        args_st,
        &dur_suffix,
        meta_st,
        width,
    )];

    // Code edits render a compact green/red diff block, whatever the status.
    let is_edit = matches!(card.name.as_str(), "edit_file" | "write_file");
    if is_edit && !card.output.trim().is_empty() {
        out.extend(diff_lines(&card.output, app, width));
        return out;
    }

    let lines: Vec<&str> = card
        .output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if lines.is_empty() {
        if running {
            out.push(Line::from(Span::styled("  …", meta_st)));
        }
        return out;
    }

    // Done → just the header; live/error keep a short preview.
    let preview_n = match card.status {
        ToolStatus::Running => 3,
        ToolStatus::Err => 4,
        ToolStatus::Ok => 0,
    };
    if preview_n == 0 {
        return out;
    }

    let hidden = lines.len().saturating_sub(preview_n);
    if hidden > 0 {
        out.push(Line::from(Span::styled(
            format!("  … {hidden} output lines hidden"),
            meta_st,
        )));
    }
    let start = lines.len().saturating_sub(preview_n);
    let raw: Vec<Line> = lines[start..]
        .iter()
        .map(|l| Line::from(Span::styled(format!("    {l}"), meta_st)))
        .collect();
    out.extend(wrap::wrap_lines(raw, width));

    out
}

fn tool_header_line(
    card: &ToolCard,
    running: bool,
    verb_st: Style,
    args_st: Style,
    dur_suffix: &str,
    meta_st: Style,
    width: usize,
) -> Line {
    let args = card.args.as_str();
    let (verb, shell) = tool_verb(&card.name, running);

    let mut spans = vec![Span::raw("  ")];
    if shell {
        spans.push(Span::styled("$ ", verb_st));
        let cmd = if args.is_empty() {
            card.name.as_str()
        } else {
            args
        };
        let budget = width
            .saturating_sub(2 + 2 + display_width(dur_suffix))
            .max(8);
        spans.push(Span::styled(truncate_width(cmd, budget), args_st));
    } else {
        spans.push(Span::styled(format!("{verb} "), verb_st));
        let detail = if !args.is_empty() {
            args.to_string()
        } else if running {
            "…".to_string()
        } else if verb == "Ran" || verb == "Running" {
            card.name.clone()
        } else {
            String::new()
        };
        if !detail.is_empty() {
            let budget = width
                .saturating_sub(2 + display_width(verb) + 1 + display_width(dur_suffix))
                .max(8);
            spans.push(Span::styled(truncate_width(&detail, budget), args_st));
        }
    }
    spans.push(Span::styled(dur_suffix.to_string(), meta_st));
    Line::from(spans)
}

/// Human verb + whether to render as `$ cmd`.
fn tool_verb(name: &str, running: bool) -> (&'static str, bool) {
    match name {
        "run_shell" => ("$", true),
        "read_file" => ("Reading", false),
        "write_file" => (if running { "Writing" } else { "Wrote" }, false),
        "write_plan" => (if running { "Planning" } else { "Planned" }, false),
        "edit_file" => (if running { "Editing" } else { "Edited" }, false),
        "delete_path" => (if running { "Deleting" } else { "Deleted" }, false),
        "list_dir" => (if running { "Listing" } else { "Listed" }, false),
        "glob" => (if running { "Globbing" } else { "Globbed" }, false),
        "grep" => (if running { "Grepping" } else { "Grepped" }, false),
        "web_search" => (if running { "Searching" } else { "Searched" }, false),
        "web_get_contents" => (if running { "Fetching" } else { "Fetched" }, false),
        "read_skill" => (
            if running {
                "Loading skill"
            } else {
                "Loaded skill"
            },
            false,
        ),
        "spawn_subagent" | "spawn_swarm" => (if running { "Spawning" } else { "Spawned" }, false),
        "verify_project" => (if running { "Checking" } else { "Checked" }, false),
        _ => (if running { "Running" } else { "Ran" }, false),
    }
}

pub(crate) fn format_tool_secs(secs: f64) -> String {
    if secs < 0.05 {
        "0s".into()
    } else if secs < 10.0 {
        format!("{secs:.1}s")
    } else {
        format!("{secs:.0}s")
    }
}

fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
        .sum()
}

fn truncate_width(s: &str, max: usize) -> String {
    if display_width(s) <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Render a diff (lines prefixed `+`/`-`) as an indented block where each row's
/// background hugs the content width — a green add / red delete band, never the
/// full terminal width.
pub(crate) fn diff_lines(diff: &str, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let indent = 4usize;
    let avail = width.saturating_sub(indent + 1).max(8);
    let rows: Vec<&str> = diff.lines().filter(|l| !l.trim().is_empty()).collect();

    // Block width tracks the longest row, but is capped to the space we have.
    let block_w = rows
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(1, avail);

    rows.into_iter()
        .map(|row| {
            let (fg, bg) = if row.starts_with("+ ") {
                (theme.add_fg, theme.add_bg)
            } else if row.starts_with("- ") {
                (theme.del_fg, theme.del_bg)
            } else {
                (theme.faint, theme.code_bg)
            };
            let mut text: String = row.chars().take(block_w).collect();
            let pad = block_w.saturating_sub(text.chars().count());
            if pad > 0 {
                text.push_str(&" ".repeat(pad));
            }
            Line::from(vec![
                Span::raw(" ".repeat(indent)),
                Span::styled(text, Style::default().fg(fg).bg(bg)),
            ])
        })
        .collect()
}
