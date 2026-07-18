//! Right-hand project panel on wide terminals: name, branch, Sub agents
//! (running + completed, with tokens), then changed files with +green / -red.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use comb::{Buffer, Line, Modifier, Rect, Span, Style};
use hive_core::event::SubagentStatus;

use crate::app::state::SubagentCard;
use crate::app::App;

/// Show the sidebar when the terminal is at least this wide.
pub const MIN_TERM_WIDTH: u16 = 110;
/// Preferred sidebar content width (not including left gap).
pub const WIDTH: u16 = 34;
const REFRESH: Duration = Duration::from_secs(2);

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
            let code = &line[..2];
            let path = line[3..].trim();
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

/// Layout helper: sidebar width when the terminal is wide enough and open.
pub fn width_for(term_width: u16, open: bool) -> u16 {
    if open && term_width >= MIN_TERM_WIDTH {
        WIDTH
    } else {
        0
    }
}

/// True when the terminal is wide enough for a sidebar (open or collapsed).
pub fn available(term_width: u16) -> bool {
    term_width >= MIN_TERM_WIDTH
}

/// Draw the project panel. Records `app.sidebar_toggle_hit` for the hide control (`›`).
pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App) {
    if area.width < 12 || area.height < 4 {
        app.sidebar_toggle_hit = None;
        return;
    }
    // Opaque fill so collapsing width / shorter file lists cannot bleed.
    buf.paint(area, Style::default());
    let theme = &app.theme;
    let snap = &app.project;
    let w = area.width as usize;

    // Header: "Project" left, hide control `›` right.
    let hide = "›";
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
    app.sidebar_toggle_hit = Some(Rect {
        x: area.x + area.width.saturating_sub(2),
        y: area.y,
        width: 2,
        height: 1,
    });

    let body = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(1),
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        truncate(&snap.name, w),
        Style::default().fg(theme.fg).add(Modifier::BOLD),
    )));
    lines.push(Line::from(vec![
        Span::styled("· ", Style::default().fg(theme.faint)),
        Span::styled(
            truncate(&snap.branch, w.saturating_sub(2)),
            Style::default().fg(theme.dim),
        ),
    ]));

    // Sub agents sit directly under Project so a long Changes list cannot
    // push them below the visible sidebar height.
    let agents = app.sidebar_subagents();
    if !agents.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Sub agents",
            Style::default().fg(theme.faint).add(Modifier::BOLD),
        )));
        for card in agents {
            lines.push(agent_line(card, app, w));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Changes",
        Style::default().fg(theme.faint).add(Modifier::BOLD),
    )));

    if snap.files.is_empty() {
        lines.push(Line::from(Span::styled(
            "clean working tree",
            Style::default().fg(theme.faint),
        )));
    } else {
        for f in &snap.files {
            lines.push(file_line(f, app, w));
        }
    }

    buf.set_lines(body, &lines, 0);
}

/// Collapsed affordance on the far right — click `‹` to reopen the panel.
pub fn draw_collapsed_toggle(buf: &mut Buffer, area: Rect, app: &mut App) {
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

fn file_line(f: &ChangedFile, app: &App, width: usize) -> Line {
    let theme = &app.theme;
    let stats = format_stats(f);
    let stats_w = stats.chars().count();
    let name_budget = width.saturating_sub(stats_w.saturating_add(1));
    let name = truncate(&f.path, name_budget);
    let pad = name_budget.saturating_sub(name.chars().count());

    let mut spans = vec![
        Span::styled(name, Style::default().fg(theme.dim)),
        Span::raw(" ".repeat(pad.saturating_add(1))),
    ];
    if f.untracked && f.added == 0 && f.deleted == 0 {
        spans.push(Span::styled(
            "+new".to_string(),
            Style::default().fg(theme.add_fg),
        ));
    } else {
        if f.added > 0 {
            spans.push(Span::styled(
                format!("+{}", f.added),
                Style::default().fg(theme.add_fg),
            ));
        }
        if f.deleted > 0 {
            if f.added > 0 {
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(
                format!("-{}", f.deleted),
                Style::default().fg(theme.del_fg),
            ));
        }
        if f.added == 0 && f.deleted == 0 {
            spans.push(Span::styled(
                "·".to_string(),
                Style::default().fg(theme.faint),
            ));
        }
    }
    Line::from(spans)
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

    let name_fg = match card.status {
        SubagentStatus::Running => theme.fg,
        SubagentStatus::Done => theme.dim,
        SubagentStatus::Failed => theme.err,
    };

    Line::from(vec![
        Span::styled(name, Style::default().fg(name_fg)),
        Span::raw(" ".repeat(pad.saturating_add(1))),
        Span::styled(tokens, Style::default().fg(theme.faint)),
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

fn format_stats(f: &ChangedFile) -> String {
    if f.untracked && f.added == 0 && f.deleted == 0 {
        return "+new".into();
    }
    let mut s = String::new();
    if f.added > 0 {
        s.push_str(&format!("+{}", f.added));
    }
    if f.deleted > 0 {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(&format!("-{}", f.deleted));
    }
    if s.is_empty() {
        s.push('·');
    }
    s
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
            cwd: "/tmp".into(),
            theme: "gray".into(),
            version: "0".into(),
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

        assert!(text.contains("Sub agents"), "missing section label: {text}");
        assert!(text.contains("Checking"), "running agent missing: {text}");
        assert!(
            text.contains("Reviewing"),
            "completed agent missing: {text}"
        );
        assert!(text.contains("2.6k"), "token count missing: {text}");

        let sub_at = text.find("Sub agents").expect("Sub agents");
        let changes_at = text.find("Changes").expect("Changes");
        assert!(
            sub_at < changes_at,
            "Sub agents must appear above Changes so a long file list cannot clip it"
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
}
