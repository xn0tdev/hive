//! Regular tool rows — no strip background until the pointer is on them.

use comb::{Color, Line, Modifier, Span, Style};

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
pub(crate) fn tool_lines(card: &ToolCard, app: &App, width: usize, hovered: bool) -> Vec<Line> {
    let lines = tool_body_lines(card, app, width);
    if hovered {
        hover_band(lines, app.theme.strip_hover, width)
    } else {
        lines
    }
}

/// Lay the hover band under a card's rows so it reads as one clickable block.
/// Spans that already carry a background (diff bands, code) keep theirs — the
/// band only fills what's behind the plain text and past the end of the row.
pub(crate) fn hover_band(lines: Vec<Line>, bg: Color, width: usize) -> Vec<Line> {
    lines
        .into_iter()
        .map(|line| {
            let mut used = 0usize;
            let mut spans: Vec<Span> = line
                .spans
                .into_iter()
                .map(|s| {
                    used += s.width();
                    if s.style.bg.is_some() {
                        s
                    } else {
                        let style = s.style.bg(bg);
                        Span::styled(s.content, style)
                    }
                })
                .collect();
            if width > used {
                spans.push(Span::styled(
                    " ".repeat(width - used),
                    Style::default().bg(bg),
                ));
            }
            Line::from(spans)
        })
        .collect()
}

fn tool_body_lines(card: &ToolCard, app: &App, width: usize) -> Vec<Line> {
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
    let dur_suffix = if show_timing && (running || card.elapsed_ms.is_some()) {
        format!("  · {}", format_tool_secs(card.secs()))
    } else {
        String::new()
    };

    // Code edits show their diff while active, then collapse to the header's
    // +/− summary at the end of the turn. Details remain available on click.
    let is_edit = matches!(card.name.as_str(), "edit_file" | "write_file");
    // How much changed belongs next to the file name, not buried in the rows.
    let stat_suffix = if is_edit {
        match diff_counts(&card.output) {
            (0, 0) => String::new(),
            (add, 0) => format!("  · +{add}"),
            (0, del) => format!("  · −{del}"),
            (add, del) => format!("  · +{add} −{del}"),
        }
    } else {
        String::new()
    };
    let meta_suffix = format!("{stat_suffix}{dur_suffix}");

    let mut out = vec![tool_header_line(
        card,
        running,
        verb_st,
        args_st,
        &meta_suffix,
        meta_st,
        width,
    )];

    if is_edit
        && !card.output.trim().is_empty()
        && (card.status != ToolStatus::Ok || card.details_open)
    {
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

    // Done → just the header unless the user asked for details. Live/error
    // always keep a short preview so hiding completed tools cannot hide state.
    let preview_n = match card.status {
        ToolStatus::Running => 3,
        ToolStatus::Err => 4,
        ToolStatus::Ok if card.details_open => 4,
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
    let raw: Vec<Line> = lines[start.min(lines.len())..]
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
        "spawn_subagent" => (if running { "Spawning" } else { "Spawned" }, false),
        "agent_observe" => (if running { "Observing" } else { "Observed" }, false),
        "agent_message" => (if running { "Messaging" } else { "Messaged" }, false),
        "agent_wait" => (if running { "Waiting" } else { "Waited" }, false),
        "agent_close" => (if running { "Closing" } else { "Closed" }, false),
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

/// One parsed row of a `compact_diff`: `{sign}{line}\t{text}`.
struct DiffRow<'a> {
    sign: Option<char>,
    line: Option<u32>,
    text: &'a str,
}

fn parse_diff_row(row: &str) -> DiffRow<'_> {
    let sign = match row.chars().next() {
        Some(c @ ('+' | '-')) => c,
        _ => {
            return DiffRow {
                sign: None,
                line: None,
                text: row,
            }
        }
    };
    match row[1..].split_once('\t') {
        Some((num, text)) => DiffRow {
            sign: Some(sign),
            line: num.parse().ok(),
            text,
        },
        // No number (older output, or the "… N more" tail).
        None => DiffRow {
            sign: Some(sign),
            line: None,
            text: row[1..].trim_start(),
        },
    }
}

/// How many lines a diff adds and removes.
pub(crate) fn diff_counts(diff: &str) -> (usize, usize) {
    diff.lines()
        .fold((0, 0), |(add, del), row| match parse_diff_row(row).sign {
            Some('+') => (add + 1, del),
            Some('-') => (add, del + 1),
            _ => (add, del),
        })
}

/// Render a diff as an indented block: a faint line-number gutter, then a
/// green add / red delete band that hugs the content width rather than the
/// whole terminal.
pub(crate) fn diff_lines(diff: &str, app: &App, width: usize) -> Vec<Line> {
    let theme = &app.theme;
    let indent = 4usize;
    let rows: Vec<DiffRow<'_>> = diff
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(parse_diff_row)
        .collect();

    // The gutter is as wide as the largest line number, so it reads as a column.
    let gutter = rows
        .iter()
        .filter_map(|r| r.line)
        .max()
        .map(|n| n.to_string().len())
        .unwrap_or(0);
    let lead = if gutter > 0 { gutter + 1 } else { 0 };
    let avail = width.saturating_sub(indent + lead + 1).max(8);

    // Block width tracks the longest row, but is capped to the space we have.
    let block_w = rows
        .iter()
        .map(|r| r.text.chars().count() + 2)
        .max()
        .unwrap_or(0)
        .clamp(1, avail);

    rows.into_iter()
        .map(|row| {
            let (fg, bg) = match row.sign {
                Some('+') => (theme.add_fg, theme.add_bg),
                Some('-') => (theme.del_fg, theme.del_bg),
                _ => (theme.faint, theme.code_bg),
            };
            let body = match row.sign {
                Some(sign) => format!("{sign} {}", row.text),
                None => row.text.to_string(),
            };
            let mut text: String = body.chars().take(block_w).collect();
            let pad = block_w.saturating_sub(text.chars().count());
            if pad > 0 {
                text.push_str(&" ".repeat(pad));
            }

            let mut spans = vec![Span::raw(" ".repeat(indent))];
            if gutter > 0 {
                let num = match row.line {
                    Some(n) => format!("{n:>gutter$} "),
                    None => " ".repeat(gutter + 1),
                };
                spans.push(Span::styled(num, Style::default().fg(theme.faint)));
            }
            spans.push(Span::styled(text, Style::default().fg(fg).bg(bg)));
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod diff_tests {
    use super::{diff_counts, diff_lines, tool_body_lines};
    use crate::app::state::{ToolCard, ToolStatus};
    use crate::app::App;
    use crate::TuiInit;
    use comb::Line;

    fn app() -> App {
        App::new(TuiInit {
            model: "m".into(),
            model_display: "m".into(),
            model_choices: Vec::new(),
            skills: Vec::new(),
            connections: Vec::new(),
            active_connection: String::new(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0.1.0".into(),
            ui: Default::default(),
            context_window: 128_000,
            cost_input: 0.0,
            cost_output: 0.0,
        })
    }

    fn rendered(diff: &str, width: usize) -> Vec<String> {
        let a = app();
        diff_lines(diff, &a, width)
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_str()).collect())
            .collect()
    }

    fn card(name: &str, output: &str) -> ToolCard {
        ToolCard {
            id: "tool".into(),
            name: name.into(),
            args: "src/main.rs".into(),
            output: output.into(),
            status: ToolStatus::Ok,
            started: std::time::Instant::now(),
            elapsed_ms: None,
            details_open: false,
            snapshot: None,
        }
    }

    fn line_text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_str())
            .collect()
    }

    const DIFF: &str = "-118\tlet mark = old;\n-119\t    inner\n+118\tlet mark = new;\n";

    #[test]
    fn rows_carry_the_line_they_changed() {
        let rows = rendered(DIFF, 70);
        assert!(rows[0].contains("118"), "{rows:?}");
        assert!(rows[0].contains("- let mark = old;"), "{rows:?}");
        assert!(rows[1].contains("119"), "{rows:?}");
        assert!(rows[2].contains("+ let mark = new;"), "{rows:?}");

        // The numbers form a column: same width for every row.
        let gutters: Vec<&str> = rows.iter().map(|r| &r[..8]).collect();
        assert!(
            gutters.iter().all(|g| g.len() == gutters[0].len()),
            "{gutters:?}"
        );
    }

    #[test]
    fn a_line_keeps_its_own_indentation() {
        let rows = rendered("+7\t\t\tdeeply indented\n", 70);
        assert!(rows[0].contains("+ \t\tdeeply indented"), "{rows:?}");
    }

    #[test]
    fn counts_are_what_the_header_shows() {
        assert_eq!(diff_counts(DIFF), (1, 2));
        assert_eq!(diff_counts(""), (0, 0));
        // The truncation tail is not a changed line.
        assert_eq!(diff_counts("+1\ta\n… 4 more\n"), (1, 0));
    }

    #[test]
    fn output_without_numbers_still_renders() {
        // Sessions saved before the format carried line numbers.
        let rows = rendered("- old line\n+ new line\n", 70);
        assert!(rows[0].contains("- old line"), "{rows:?}");
        assert!(rows[1].contains("+ new line"), "{rows:?}");
    }

    #[test]
    fn a_narrow_card_still_renders_every_row() {
        for width in [12usize, 20, 40] {
            let rows = rendered(DIFF, width);
            assert_eq!(rows.len(), 3, "width {width}");
        }
    }

    #[test]
    fn resumed_tool_does_not_invent_a_duration() {
        let a = app();
        let card = card("custom", "done");
        let lines = tool_body_lines(&card, &a, 70);

        assert_eq!(lines.len(), 1);
        assert!(!line_text(&lines[0]).contains(" · "));
    }

    #[test]
    fn completed_edit_collapses_to_its_diff_summary() {
        let a = app();
        let mut card = card("edit_file", "+1\tnew line\n-2\told line");
        let collapsed = tool_body_lines(&card, &a, 70);
        assert_eq!(collapsed.len(), 1);
        let header = line_text(&collapsed[0]);
        assert!(header.contains("+1"), "{header}");
        assert!(header.contains("−1"), "{header}");

        card.details_open = true;
        let expanded = tool_body_lines(&card, &a, 70);
        assert_eq!(expanded.len(), 3);
    }
}
