//! Right-hand project panel on wide terminals: name, branch, Context,
//! Sub agents, Terminals, then changed file paths.
//! Body sections are click-to-collapse; agent/terminal rows open their views.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use comb::{Buffer, Line, Modifier, Rect, Span, Style};
use hive_core::event::SubagentStatus;
use hive_core::{SidebarMode, TerminalProcessState};

use crate::app::state::{SubagentCard, TerminalCard};
use crate::app::App;

/// Collapsible sidebar body sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarSection {
    Context,
    Subagents,
    Terminals,
    Changes,
}

/// Clickable row in the sidebar (opens a dedicated view).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarItem {
    Subagent(String),
    Terminal(String),
}

/// In-memory expand/collapse flags for sidebar sections.
#[derive(Clone, Debug)]
pub struct SidebarSections {
    pub context: bool,
    pub subagents: bool,
    pub terminals: bool,
    pub changes: bool,
}

impl Default for SidebarSections {
    fn default() -> Self {
        Self {
            context: true,
            subagents: true,
            terminals: true,
            changes: true,
        }
    }
}

impl SidebarSections {
    pub fn toggle(&mut self, section: SidebarSection) {
        match section {
            SidebarSection::Context => self.context = !self.context,
            SidebarSection::Subagents => self.subagents = !self.subagents,
            SidebarSection::Terminals => self.terminals = !self.terminals,
            SidebarSection::Changes => self.changes = !self.changes,
        }
    }

    pub fn expanded(&self, section: SidebarSection) -> bool {
        match section {
            SidebarSection::Context => self.context,
            SidebarSection::Subagents => self.subagents,
            SidebarSection::Terminals => self.terminals,
            SidebarSection::Changes => self.changes,
        }
    }
}

/// Show the sidebar when the terminal is at least this wide.
pub const MIN_TERM_WIDTH: u16 = 110;
/// Narrowest usable panel.
pub const MIN_WIDTH: u16 = 24;
/// Widest panel — leave room for the chat column.
pub const MAX_WIDTH: u16 = 56;
const REFRESH: Duration = Duration::from_secs(2);

/// Clamp a preferred width into the allowed range.
pub fn clamp_width(w: u16) -> u16 {
    w.clamp(MIN_WIDTH, MAX_WIDTH)
}

#[derive(Clone, Debug, Default)]
pub struct ChangedFile {
    pub path: String,
    pub added: u32,
    pub deleted: u32,
    pub untracked: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectSnapshot {
    pub name: String,
    pub branch: String,
    pub files: Vec<ChangedFile>,
    fetched_at: Option<Instant>,
}

impl ProjectSnapshot {
    pub fn stale(&self) -> bool {
        match self.fetched_at {
            None => true,
            Some(t) => t.elapsed() >= REFRESH,
        }
    }

    pub fn invalidate(&mut self) {
        self.fetched_at = None;
    }

    pub fn refresh_if_stale(&mut self, cwd: &str) {
        if self.stale() {
            *self = fetch(cwd);
        }
    }
}

pub fn fetch(cwd: &str) -> ProjectSnapshot {
    let root = Path::new(cwd);
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(cwd)
        .to_string();

    let branch = git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "HEAD")
        .or_else(|| {
            git(cwd, &["rev-parse", "--short", "HEAD"]).map(|s| format!("detached {}", s.trim()))
        })
        .unwrap_or_else(|| "no git".into());

    let mut map: BTreeMap<String, ChangedFile> = BTreeMap::new();

    for (added, deleted, path) in numstat(cwd, &["diff", "--numstat"]) {
        let e = map.entry(path.clone()).or_insert(ChangedFile {
            path,
            ..Default::default()
        });
        e.added = e.added.saturating_add(added);
        e.deleted = e.deleted.saturating_add(deleted);
    }
    for (added, deleted, path) in numstat(cwd, &["diff", "--cached", "--numstat"]) {
        let e = map.entry(path.clone()).or_insert(ChangedFile {
            path,
            ..Default::default()
        });
        e.added = e.added.saturating_add(added);
        e.deleted = e.deleted.saturating_add(deleted);
    }

    if let Some(porcelain) = git(cwd, &["status", "--porcelain=v1", "-uall"]) {
        for line in porcelain.lines() {
            if line.len() < 4 {
                continue;
            }
            let code = line.get(..2).unwrap_or(line);
            let path = line.get(3..).unwrap_or("").trim();
            if path.is_empty() {
                continue;
            }
            // Rename: `R  old -> new` — show the new side.
            let path = path
                .rsplit_once(" -> ")
                .map(|(_, n)| n)
                .unwrap_or(path)
                .to_string();
            if code.contains('?') {
                map.entry(path.clone())
                    .and_modify(|e| e.untracked = true)
                    .or_insert(ChangedFile {
                        path,
                        untracked: true,
                        ..Default::default()
                    });
            } else {
                map.entry(path.clone()).or_insert(ChangedFile {
                    path,
                    ..Default::default()
                });
            }
        }
    }

    let files: Vec<ChangedFile> = map.into_values().collect();
    ProjectSnapshot {
        name,
        branch,
        files,
        fetched_at: Some(Instant::now()),
    }
}

fn git(cwd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn numstat(cwd: &str, args: &[&str]) -> Vec<(u32, u32, String)> {
    let Some(text) = git(cwd, args) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let a = parts.next().unwrap_or("-");
        let d = parts.next().unwrap_or("-");
        let path = parts.next().unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        // Binary diffs show `-` for counts.
        let added = a.parse::<u32>().unwrap_or(0);
        let deleted = d.parse::<u32>().unwrap_or(0);
        out.push((added, deleted, path.to_string()));
    }
    out
}

/// Layout helper: sidebar width from terminal size + UI prefs.
pub fn width_for(term_width: u16, open: bool, mode: SidebarMode, preferred: u16) -> u16 {
    if term_width < MIN_TERM_WIDTH {
        return 0;
    }
    // Keep enough room for the chat band (~48 cols + gaps).
    let term_cap = term_width.saturating_sub(52).clamp(MIN_WIDTH, MAX_WIDTH);
    let w = clamp_width(preferred).min(term_cap);
    match mode {
        SidebarMode::Hidden => 0,
        SidebarMode::Pinned => w,
        SidebarMode::Auto => {
            if open {
                w
            } else {
                0
            }
        }
    }
}

/// True when the terminal is wide enough for a sidebar (open or collapsed).
pub fn available(term_width: u16) -> bool {
    term_width >= MIN_TERM_WIDTH
}

/// Draw the project panel. Records `app.sidebar_toggle_hit` for the hide control (`›`),
/// `app.sidebar_section_hits` for collapsible headers, and `app.sidebar_item_hits` for
/// clickable subagent / terminal rows.
pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App) {
    if area.width < 12 || area.height < 4 {
        app.sidebar_toggle_hit = None;
        app.sidebar_resize_hit = None;
        app.sidebar_section_hits.clear();
        app.sidebar_item_hits.clear();
        return;
    }
    // Opaque fill so collapsing width / shorter file lists cannot bleed.
    buf.paint(area, Style::default());
    let theme = &app.theme;
    let snap = &app.project;
    let w = area.width as usize;

    // Invisible drag handle on the left edge — stretch the panel left/right.
    app.sidebar_resize_hit = Some(Rect {
        x: area.x.saturating_sub(1),
        y: area.y,
        width: 2,
        height: area.height,
    });
    app.sidebar_right_edge = area.right();

    // Header: "Project" left; hide `›` only in auto mode.
    let can_hide = app.ui.sidebar_mode == SidebarMode::Auto;
    let hide = if can_hide { "›" } else { "" };
    let title = "Project";
    let pad = w.saturating_sub(title.chars().count() + hide.chars().count());
    buf.set_line(
        area.x,
        area.y,
        &Line::from(vec![
            Span::styled(
                title.to_string(),
                Style::default().fg(theme.faint).add(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(pad)),
            Span::styled(
                hide.to_string(),
                Style::default().fg(theme.dim).add(Modifier::BOLD),
            ),
        ]),
        area.width,
    );
    app.sidebar_toggle_hit = if can_hide {
        Some(Rect {
            x: area.x + area.width.saturating_sub(2),
            y: area.y,
            width: 2,
            height: 1,
        })
    } else {
        None
    };

    let body = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    // Logical lines + optional section / item tags for hit-testing after clip.
    let mut lines: Vec<(Line, Option<SidebarSection>, Option<SidebarItem>)> = Vec::new();
    lines.push((
        Line::from(Span::styled(
            truncate(&snap.name, w),
            Style::default().fg(theme.fg).add(Modifier::BOLD),
        )),
        None,
        None,
    ));
    lines.push((
        Line::from(vec![
            Span::styled("· ", Style::default().fg(theme.faint)),
            Span::styled(
                truncate(&snap.branch, w.saturating_sub(2)),
                Style::default().fg(theme.dim),
            ),
        ]),
        None,
        None,
    ));
    // Session spend (footer shows context fill instead).
    let session = format_session_tokens(app.usage.total_tokens);
    let cost = session_cost(app);
    let session_line = if cost.is_empty() {
        Line::from(vec![
            Span::styled("· ", Style::default().fg(theme.faint)),
            Span::styled(session, Style::default().fg(theme.dim)),
        ])
    } else {
        Line::from(vec![
            Span::styled("· ", Style::default().fg(theme.faint)),
            Span::styled(session, Style::default().fg(theme.dim)),
            Span::styled(" ", Style::default().fg(theme.faint)),
            Span::styled(cost, Style::default().fg(theme.dim)),
        ])
    };
    lines.push((session_line, None, None));

    let collapsible = app.ui.sidebar_collapse_sections;

    // Context — project instruction files injected into the system prompt.
    lines.push((Line::from(""), None, None));
    let ctx_open = !collapsible || app.sidebar_sections.expanded(SidebarSection::Context);
    lines.push((
        section_header("Context", ctx_open, collapsible, theme),
        if collapsible {
            Some(SidebarSection::Context)
        } else {
            None
        },
        None,
    ));
    if ctx_open {
        if app.context_files.is_empty() {
            lines.push((
                Line::from(Span::styled("none", Style::default().fg(theme.faint))),
                None,
                None,
            ));
        } else {
            for f in &app.context_files {
                let label = if f.rel_path == f.name {
                    f.name.clone()
                } else {
                    f.rel_path.clone()
                };
                lines.push((
                    Line::from(Span::styled(
                        truncate(&label, w),
                        Style::default().fg(theme.dim),
                    )),
                    None,
                    None,
                ));
            }
        }
    }

    // Sub agents / Terminals sit above Changes so a long file list cannot push them out.
    let agents = app.sidebar_subagents();
    if !agents.is_empty() {
        lines.push((Line::from(""), None, None));
        let open = !collapsible || app.sidebar_sections.expanded(SidebarSection::Subagents);
        lines.push((
            section_header("Sub agents", open, collapsible, theme),
            if collapsible {
                Some(SidebarSection::Subagents)
            } else {
                None
            },
            None,
        ));
        if open {
            for card in agents {
                lines.push((
                    agent_line(card, app, w),
                    None,
                    Some(SidebarItem::Subagent(card.id.clone())),
                ));
            }
        }
    }

    let terminals = app.sidebar_terminals();
    if !terminals.is_empty() {
        lines.push((Line::from(""), None, None));
        let open = !collapsible || app.sidebar_sections.expanded(SidebarSection::Terminals);
        lines.push((
            section_header("Terminals", open, collapsible, theme),
            if collapsible {
                Some(SidebarSection::Terminals)
            } else {
                None
            },
            None,
        ));
        if open {
            for card in terminals {
                lines.push((
                    terminal_line(card, app, w),
                    None,
                    Some(SidebarItem::Terminal(card.id.clone())),
                ));
            }
        }
    }

    lines.push((Line::from(""), None, None));
    let ch_open = !collapsible || app.sidebar_sections.expanded(SidebarSection::Changes);
    lines.push((
        section_header("Changes", ch_open, collapsible, theme),
        if collapsible {
            Some(SidebarSection::Changes)
        } else {
            None
        },
        None,
    ));
    if ch_open {
        if snap.files.is_empty() {
            lines.push((
                Line::from(Span::styled(
                    "clean working tree",
                    Style::default().fg(theme.faint),
                )),
                None,
                None,
            ));
        } else {
            // Changes is the last section, so without a cap the tail of a long
            // list just falls off the bottom edge with nothing to say it did.
            let room = (body.height as usize).saturating_sub(lines.len());
            let (shown, hidden) = if snap.files.len() > room && room > 0 {
                (room - 1, snap.files.len() - (room - 1))
            } else {
                (snap.files.len(), 0)
            };
            let stat_w = snap
                .files
                .iter()
                .take(shown)
                .map(|f| file_stat(f, theme).1)
                .max()
                .unwrap_or(0);
            for f in snap.files.iter().take(shown) {
                lines.push((file_line(f, app, w, stat_w), None, None));
            }
            if hidden > 0 {
                lines.push((
                    Line::from(Span::styled(
                        format!("+{hidden} more"),
                        Style::default().fg(theme.faint),
                    )),
                    None,
                    None,
                ));
            }
        }
    }

    let draw_lines: Vec<Line> = lines.iter().map(|(l, _, _)| l.clone()).collect();
    buf.set_lines(body, &draw_lines, 0);

    app.sidebar_section_hits.clear();
    app.sidebar_item_hits.clear();
    let visible = body.height as usize;
    for (i, (_, section, item)) in lines.iter().enumerate().take(visible) {
        let rect = Rect {
            x: body.x,
            y: body.y + i as u16,
            width: body.width,
            height: 1,
        };
        if let Some(sec) = section {
            app.sidebar_section_hits.push((rect, *sec));
        }
        if let Some(item) = item {
            app.sidebar_item_hits.push((rect, item.clone()));
        }
    }
}

fn section_header(
    label: &str,
    expanded: bool,
    collapsible: bool,
    theme: &crate::theme::Theme,
) -> Line {
    let mark = if !collapsible {
        ""
    } else if expanded {
        "▾ "
    } else {
        "▸ "
    };
    Line::from(vec![
        Span::styled(mark.to_string(), Style::default().fg(theme.dim)),
        Span::styled(
            label.to_string(),
            Style::default().fg(theme.faint).add(Modifier::BOLD),
        ),
    ])
}

/// Collapsed affordance on the far right — click `‹` to reopen the panel.
pub fn draw_collapsed_toggle(buf: &mut Buffer, area: Rect, app: &mut App) {
    app.sidebar_section_hits.clear();
    app.sidebar_item_hits.clear();
    app.sidebar_resize_hit = None;
    if app.ui.sidebar_mode != SidebarMode::Auto {
        app.sidebar_toggle_hit = None;
        return;
    }
    if area.width == 0 || area.height == 0 {
        app.sidebar_toggle_hit = None;
        return;
    }
    let theme = &app.theme;
    let icon = "‹";
    let x = area.right().saturating_sub(1);
    let y = area.y + 1;
    buf.set_line(
        x,
        y,
        &Line::from(Span::styled(
            icon.to_string(),
            Style::default().fg(theme.dim).add(Modifier::BOLD),
        )),
        1,
    );
    app.sidebar_toggle_hit = Some(Rect {
        x,
        y,
        width: 1,
        height: 1,
    });
}

/// `+3 −1` for a file, plus how wide it is.
fn file_stat(f: &ChangedFile, theme: &crate::theme::Theme) -> (Vec<Span>, usize) {
    let mut spans: Vec<Span> = Vec::new();
    if f.added > 0 {
        spans.push(Span::styled(
            format!("+{}", f.added),
            Style::default().fg(theme.add_fg),
        ));
    }
    if f.deleted > 0 {
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("−{}", f.deleted),
            Style::default().fg(theme.del_fg),
        ));
    }
    if spans.is_empty() {
        spans.push(Span::styled("new", Style::default().fg(theme.faint)));
    }
    let w = spans.iter().map(|s| s.content.chars().count()).sum();
    (spans, w)
}

/// `path                +3 −1` — the tail of the path (that's the part that
/// identifies a file) and how much of it changed. `stat_w` is shared across the
/// section so the paths line up instead of each ending wherever it happens to.
fn file_line(f: &ChangedFile, app: &App, width: usize, stat_w: usize) -> Line {
    let theme = &app.theme;
    let fg = if f.untracked { theme.add_fg } else { theme.dim };
    let (stat, own_w) = file_stat(f, theme);

    // Every path here starts with the same directories, so cutting the front
    // leaves a column of identical prefixes. Keep the end.
    let name = truncate_start(&f.path, width.saturating_sub(stat_w + 1));
    let gap = width.saturating_sub(name.chars().count() + own_w);

    let mut spans = vec![Span::styled(name, Style::default().fg(fg))];
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(stat);
    Line::from(spans)
}

/// Truncate from the left, keeping the tail: `…/render/palette.rs`.
fn truncate_start(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".into();
    }
    let tail: String = s.chars().skip(n - (max - 1)).collect();
    format!("…{tail}")
}

fn agent_line(card: &SubagentCard, app: &App, width: usize) -> Line {
    let theme = &app.theme;
    let tokens = format_token_count(card.usage.total_tokens);
    let tokens_w = tokens.chars().count();
    let name_budget = width.saturating_sub(tokens_w.saturating_add(1));
    let label = if card.label.trim().is_empty() {
        "subagent"
    } else {
        card.label.as_str()
    };
    let name = truncate(label, name_budget);
    let pad = name_budget.saturating_sub(name.chars().count());
    let hovered = matches!(
        &app.hover_sidebar_item,
        Some(SidebarItem::Subagent(id)) if id == &card.id
    );

    // Idle: gray (clickable hint). Hover: bright. Failed stays red until hover.
    let name_fg = if hovered {
        theme.fg
    } else if card.status == SubagentStatus::Failed {
        theme.err
    } else {
        theme.dim
    };

    Line::from(vec![
        Span::styled(name, Style::default().fg(name_fg)),
        Span::raw(" ".repeat(pad.saturating_add(1))),
        Span::styled(tokens, Style::default().fg(theme.faint)),
    ])
}

fn terminal_line(card: &TerminalCard, app: &App, width: usize) -> Line {
    let theme = &app.theme;
    let meta = match &card.process {
        TerminalProcessState::Running => format!("{:.0}s", card.secs()),
        TerminalProcessState::Exited { code: 0 } => format!("{:.1}s", card.secs()),
        TerminalProcessState::Exited { code } => format!("e{code}"),
        TerminalProcessState::Failed { .. } => "fail".into(),
    };
    let meta_w = meta.chars().count();
    let name_budget = width.saturating_sub(meta_w.saturating_add(1));
    let label = if card.command.trim().is_empty() {
        "terminal"
    } else {
        card.command.trim()
    };
    let name = truncate(label, name_budget);
    let pad = name_budget.saturating_sub(name.chars().count());
    let hovered = matches!(
        &app.hover_sidebar_item,
        Some(SidebarItem::Terminal(id)) if id == &card.id
    );

    // Idle: gray (clickable hint). Hover: bright. Failed/nonzero stays red until hover.
    let name_fg = if hovered {
        theme.fg
    } else {
        match &card.process {
            TerminalProcessState::Exited { code } if *code != 0 => theme.err,
            TerminalProcessState::Failed { .. } => theme.err,
            _ => theme.dim,
        }
    };

    Line::from(vec![
        Span::styled(name, Style::default().fg(name_fg)),
        Span::raw(" ".repeat(pad.saturating_add(1))),
        Span::styled(meta, Style::default().fg(theme.faint)),
    ])
}

fn format_token_count(n: u64) -> String {
    if n == 0 {
        return "—".into();
    }
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        format!("{n}")
    }
}

fn format_session_tokens(n: u64) -> String {
    if n == 0 {
        return "0 session".into();
    }
    if n >= 1000 {
        format!("{:.1}k session", n as f64 / 1000.0)
    } else {
        format!("{n} session")
    }
}

fn session_cost(app: &crate::app::App) -> String {
    if app.cost_input == 0.0 && app.cost_output == 0.0 {
        return String::new();
    }
    let input_cost = app.usage.prompt_tokens as f64 * app.cost_input / 1_000_000.0;
    let output_cost = app.usage.completion_tokens as f64 * app.cost_output / 1_000_000.0;
    let total = input_cost + output_cost;
    if total < 0.01 {
        format!("${:.4}", total)
    } else {
        format!("${:.2}", total)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".into();
    }
    let keep = max - 1;
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use comb::{Buffer, Rect, Size};
    use hive_core::event::{AgentEvent, SubagentStatus};
    use hive_core::provider::Usage;

    use crate::app::App;
    use crate::TuiInit;

    fn test_app() -> App {
        App::new(TuiInit {
            model: "m".into(),
            model_display: "Model".into(),
            model_choices: Vec::new(),
            skills: Vec::new(),
            connections: Vec::new(),
            active_connection: String::new(),
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0".into(),
            ui: Default::default(),
            context_window: 128_000,
            cost_input: 0.0,
            cost_output: 0.0,
        })
    }

    #[test]
    fn truncate_shortens() {
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate("ab", 4), "ab");
    }

    #[test]
    fn format_token_count_compact() {
        assert_eq!(format_token_count(0), "—");
        assert_eq!(format_token_count(42), "42");
        assert_eq!(format_token_count(1500), "1.5k");
    }

    #[test]
    fn agent_line_shows_label_and_tokens() {
        let mut app = test_app();
        app.apply(AgentEvent::SubagentSpawned {
            id: "s1".into(),
            label: "Checking project".into(),
            prompt: "run cargo check".into(),
        });
        app.apply(AgentEvent::SubagentUsage {
            id: "s1".into(),
            usage: Usage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 1500,
            },
        });
        let card = app.sidebar_subagents()[0];
        let line = agent_line(card, &app, 34);
        let text: String = line.spans.iter().map(|s| s.content.as_str()).collect();
        assert!(text.contains("Checking project"), "{text}");
        assert!(text.contains("1.5k"), "{text}");
        assert_eq!(card.status, SubagentStatus::Running);
    }

    #[test]
    fn terminal_line_brightens_on_hover() {
        let mut app = test_app();
        app.apply(AgentEvent::TerminalStarted {
            id: "term-1".into(),
            command: "theme-installer".into(),
            description: String::new(),
            rows: 12,
            cols: 40,
        });
        let idle_fg = {
            let card = app.sidebar_terminals()[0];
            terminal_line(card, &app, 34).spans[0].style.fg
        };
        assert_eq!(idle_fg, Some(app.theme.dim));

        app.hover_sidebar_item = Some(SidebarItem::Terminal("term-1".into()));
        let hover_fg = {
            let card = app.sidebar_terminals()[0];
            terminal_line(card, &app, 34).spans[0].style.fg
        };
        assert_eq!(hover_fg, Some(app.theme.fg));
    }

    fn changed(path: &str, added: u32, deleted: u32, untracked: bool) -> ChangedFile {
        ChangedFile {
            path: path.into(),
            added,
            deleted,
            untracked,
        }
    }

    fn with_changes(files: Vec<ChangedFile>) -> App {
        let mut app = test_app();
        app.project = ProjectSnapshot {
            name: "hive".into(),
            branch: "development".into(),
            files,
            fetched_at: Some(Instant::now()),
        };
        app
    }

    #[test]
    fn changed_files_keep_their_tail_and_show_their_stat() {
        let mut app = with_changes(vec![
            changed("crates/hive-tui/src/render/palette.rs", 12, 3, false),
            changed("crates/hive-tui/src/render/sidebar.rs", 4, 0, false),
            changed("crates/hive-tui/src/render/brand_new.rs", 0, 0, true),
        ]);
        let mut buf = Buffer::blank(Size::new(40, 24));
        draw(&mut buf, Rect::new(0, 0, 34, 24), &mut app);
        let text = buf.text();

        // The end of a path is what tells two of them apart.
        assert!(text.contains("palette.rs"), "{text}");
        assert!(text.contains("sidebar.rs"), "{text}");
        assert!(
            !text.contains("crates/hive-tui/src/render/palette"),
            "{text}"
        );

        assert!(text.contains("+12"), "{text}");
        assert!(text.contains("−3"), "{text}");
        assert!(text.contains("new"), "untracked says so: {text}");

        // Stats share one column, so the paths end on the same cell.
        let rows: Vec<&str> = text.lines().filter(|l| l.contains(".rs")).collect();
        assert_eq!(rows.len(), 3);
        let ends: Vec<usize> = rows.iter().map(|l| l.find('…').unwrap_or(0)).collect();
        assert!(
            ends.windows(2).all(|w| w[0] == w[1]),
            "paths must start on one column: {rows:?}"
        );
    }

    #[test]
    fn a_long_change_list_says_what_it_cut() {
        let files = (0..60)
            .map(|i| changed(&format!("src/file_{i}.rs"), 1, 1, false))
            .collect();
        let mut app = with_changes(files);

        let mut buf = Buffer::blank(Size::new(40, 20));
        draw(&mut buf, Rect::new(0, 0, 34, 20), &mut app);
        let text = buf.text();

        assert!(text.contains("more"), "cut list must own up to it: {text}");
        let last = text.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("");
        assert!(last.trim().starts_with('+'), "the tally goes last: {last}");
    }

    #[test]
    fn sidebar_shows_sub_agents_above_changes() {
        let mut app = test_app();
        app.project = ProjectSnapshot {
            name: "hive".into(),
            branch: "main".into(),
            files: (0..40)
                .map(|i| ChangedFile {
                    path: format!("crates/file_{i}.rs"),
                    added: 1,
                    deleted: 0,
                    untracked: false,
                })
                .collect(),
            fetched_at: Some(Instant::now()),
        };
        app.apply(AgentEvent::SubagentSpawned {
            id: "s1".into(),
            label: "Reviewing".into(),
            prompt: "review".into(),
        });
        app.apply(AgentEvent::SubagentStatus {
            id: "s1".into(),
            status: SubagentStatus::Done,
            detail: "done".into(),
        });
        app.apply(AgentEvent::SubagentSpawned {
            id: "s2".into(),
            label: "Checking".into(),
            prompt: "check".into(),
        });
        app.apply(AgentEvent::SubagentUsage {
            id: "s2".into(),
            usage: Usage {
                prompt_tokens: 200,
                completion_tokens: 100,
                total_tokens: 2600,
            },
        });

        let mut buf = Buffer::blank(Size::new(40, 16));
        draw(&mut buf, Rect::new(0, 0, 34, 16), &mut app);
        let text = buf.text();

        assert!(text.contains("Context"), "missing Context section: {text}");
        assert!(text.contains("Sub agents"), "missing section label: {text}");
        assert!(text.contains("Checking"), "running agent missing: {text}");
        assert!(
            text.contains("Reviewing"),
            "completed agent missing: {text}"
        );
        assert!(text.contains("2.6k"), "token count missing: {text}");

        let ctx_at = text.find("Context").expect("Context");
        let sub_at = text.find("Sub agents").expect("Sub agents");
        let changes_at = text.find("Changes").expect("Changes");
        assert!(
            ctx_at < sub_at && sub_at < changes_at,
            "section order Context → Sub agents → Changes"
        );
        // Within a short sidebar, the section header itself must be on-screen
        // (not only present in the logical line list past the clip).
        let visible = text.lines().take(16).collect::<Vec<_>>().join("\n");
        assert!(
            visible.contains("Sub agents"),
            "Sub agents clipped out of viewport: {visible}"
        );
        assert!(
            visible.contains("Checking"),
            "running agent clipped out of viewport: {visible}"
        );
    }

    #[test]
    fn width_clamps_and_respects_term() {
        assert_eq!(clamp_width(10), MIN_WIDTH);
        assert_eq!(clamp_width(99), MAX_WIDTH);
        assert_eq!(clamp_width(34), 34);
        assert_eq!(width_for(200, true, SidebarMode::Pinned, 34), 34);
        assert_eq!(width_for(200, true, SidebarMode::Pinned, 99), MAX_WIDTH);
        assert_eq!(width_for(100, true, SidebarMode::Pinned, 40), 0);
    }

    #[test]
    fn collapsing_changes_hides_file_rows() {
        let mut app = test_app();
        app.project = ProjectSnapshot {
            name: "hive".into(),
            branch: "main".into(),
            files: vec![ChangedFile {
                path: "src/main.rs".into(),
                added: 3,
                deleted: 1,
                untracked: false,
            }],
            fetched_at: Some(Instant::now()),
        };
        app.context_files = vec![hive_core::ContextFile {
            name: "AGENTS.md".into(),
            rel_path: "AGENTS.md".into(),
        }];

        let mut buf = Buffer::blank(Size::new(40, 20));
        draw(&mut buf, Rect::new(0, 0, 34, 20), &mut app);
        let open = buf.text();
        assert!(open.contains("src/main.rs"), "{open}");
        assert!(open.contains("AGENTS.md"), "{open}");
        assert!(open.contains('▾'), "expanded marker: {open}");

        app.sidebar_sections.changes = false;
        app.sidebar_sections.context = false;
        let mut buf2 = Buffer::blank(Size::new(40, 20));
        draw(&mut buf2, Rect::new(0, 0, 34, 20), &mut app);
        let closed = buf2.text();
        assert!(!closed.contains("src/main.rs"), "{closed}");
        assert!(!closed.contains("AGENTS.md"), "{closed}");
        assert!(closed.contains('▸'), "collapsed marker: {closed}");
        assert!(
            !app.sidebar_section_hits.is_empty(),
            "section headers should be clickable"
        );
    }

    #[test]
    fn sidebar_shows_terminals_and_records_item_hits() {
        let mut app = test_app();
        app.apply(AgentEvent::TerminalStarted {
            id: "term-1".into(),
            command: "theme-installer".into(),
            description: String::new(),
            rows: 12,
            cols: 40,
        });
        app.apply(AgentEvent::SubagentSpawned {
            id: "s1".into(),
            label: "Reviewing".into(),
            prompt: "review".into(),
        });

        let mut buf = Buffer::blank(Size::new(40, 20));
        draw(&mut buf, Rect::new(0, 0, 34, 20), &mut app);
        let text = buf.text();

        assert!(text.contains("Sub agents"), "{text}");
        assert!(text.contains("Terminals"), "{text}");
        assert!(text.contains("theme-installer"), "{text}");
        assert!(text.contains("Reviewing"), "{text}");

        let sub_at = text.find("Sub agents").expect("Sub agents");
        let term_at = text.find("Terminals").expect("Terminals");
        let changes_at = text.find("Changes").expect("Changes");
        assert!(
            sub_at < term_at && term_at < changes_at,
            "section order Sub agents → Terminals → Changes"
        );

        assert!(
            app.sidebar_item_hits
                .iter()
                .any(|(_, item)| matches!(item, SidebarItem::Terminal(id) if id == "term-1")),
            "terminal row should be clickable: {:?}",
            app.sidebar_item_hits
        );
        assert!(
            app.sidebar_item_hits
                .iter()
                .any(|(_, item)| matches!(item, SidebarItem::Subagent(id) if id == "s1")),
            "subagent row should be clickable: {:?}",
            app.sidebar_item_hits
        );
    }
}
