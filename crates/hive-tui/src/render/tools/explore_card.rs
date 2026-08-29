use comb::{Line, Modifier, Span, Style};

use crate::app::state::ExploreCard;
use crate::app::App;

use super::format_tool_secs;
use super::tool_card::{hover_band, tool_lines};

pub(crate) fn explore_card_lines(
    card: &ExploreCard,
    app: &App,
    width: usize,
    hovered: bool,
) -> Vec<Line> {
    let running = card.running();
    let mut title = if running { "Exploring" } else { "Explored" }.to_string();
    for label in count_labels(card) {
        title.push_str(" · ");
        title.push_str(&label);
    }
    if card.failed > 0 {
        title.push_str(&format!(" · {} failed", card.failed));
    }
    title.push_str(&format!(" · {}", format_tool_secs(card.secs())));

    let marker = if card.details_open { "▾" } else { "▸" };
    let status_color = if card.failed > 0 {
        app.theme.err
    } else if running {
        app.theme.fg
    } else {
        app.theme.dim
    };
    let header = Line::from(vec![
        Span::styled(format!("  {marker} "), Style::default().fg(app.theme.faint)),
        Span::styled(title, Style::default().fg(status_color).add(Modifier::BOLD)),
    ]);
    let mut out = vec![header];

    if card.details_open {
        for (index, tool) in card.tools.iter().enumerate() {
            let last = index + 1 == card.tools.len();
            let child_width = width.saturating_sub(5).max(1);
            let child_lines = tool_lines(tool, app, child_width, false);
            for (line_index, line) in child_lines.into_iter().enumerate() {
                let branch = if line_index == 0 {
                    if last {
                        "  └─"
                    } else {
                        "  ├─"
                    }
                } else if last {
                    "    "
                } else {
                    "  │ "
                };
                let mut spans = vec![Span::styled(branch, Style::default().fg(app.theme.faint))];
                spans.extend(line.spans);
                out.push(Line::from(spans));
            }
        }
    }

    if hovered {
        hover_band(out, app.theme.strip_hover, width)
    } else {
        out
    }
}

fn count_labels(card: &ExploreCard) -> Vec<String> {
    let mut files = 0;
    let mut searches = 0;
    let mut directories = 0;
    let mut pages = 0;
    let mut skills = 0;
    let mut other = 0;

    for tool in &card.tools {
        match tool.name.as_str() {
            "read_file" => files += 1,
            "grep" | "glob" | "web_search" => searches += 1,
            "list_dir" => directories += 1,
            "web_get_contents" => pages += 1,
            "read_skill" => skills += 1,
            _ => other += 1,
        }
    }

    let mut labels = Vec::new();
    push_count(&mut labels, files, "file", "files");
    push_count(&mut labels, searches, "search", "searches");
    push_count(&mut labels, directories, "directory", "directories");
    push_count(&mut labels, pages, "page", "pages");
    push_count(&mut labels, skills, "skill", "skills");
    push_count(&mut labels, other, "tool", "tools");
    labels
}

fn push_count(labels: &mut Vec<String>, count: usize, one: &str, many: &str) {
    if count > 0 {
        labels.push(format!("{count} {}", if count == 1 { one } else { many }));
    }
}
