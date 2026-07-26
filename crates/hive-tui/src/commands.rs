//! Command catalogue: one source of truth for the slash menu, command palette,
//! and key handler. Canonical names are listed once; aliases resolve at execute.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdId {
    Clear,
    Compact,
    Model,
    Connect,
    Copy,
    Cost,
    About,
    Settings,
    Goal,
    Resume,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Suggested,
    Session,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Suggested => "Suggested",
            Category::Session => "Session",
        }
    }

    fn order(self) -> u8 {
        match self {
            Category::Suggested => 0,
            Category::Session => 1,
        }
    }
}

#[derive(Debug)]
pub struct CommandDef {
    pub id: CmdId,
    /// Canonical slash name (without `/`).
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    /// Palette row label.
    pub label: &'static str,
    pub desc: &'static str,
    /// Slash-menu argument hint, e.g. "[path]".
    pub hint: &'static str,
    pub takes_arg: bool,
    pub category: Category,
    /// Optional shortcut hint shown right-aligned in the palette.
    pub shortcut: Option<&'static str>,
    /// When true, also listed under Suggested.
    pub suggested: bool,
}

pub const COMMANDS: &[CommandDef] = &[
    CommandDef {
        id: CmdId::Model,
        name: "model",
        aliases: &["models"],
        label: "Switch model",
        desc: "Change the active model",
        hint: "[id]",
        takes_arg: true,
        category: Category::Session,
        shortcut: None,
        suggested: true,
    },
    CommandDef {
        id: CmdId::Connect,
        name: "connect",
        aliases: &["provider", "providers"],
        label: "Providers",
        desc: "Add or switch API providers",
        hint: "",
        takes_arg: false,
        category: Category::Session,
        shortcut: None,
        suggested: true,
    },
    CommandDef {
        id: CmdId::Clear,
        name: "clear",
        aliases: &["new", "reset"],
        label: "Clear",
        desc: "New chat",
        hint: "",
        takes_arg: false,
        category: Category::Session,
        shortcut: None,
        suggested: true,
    },
    CommandDef {
        id: CmdId::Compact,
        name: "compact",
        aliases: &[],
        label: "Compact context",
        desc: "Compress chat history; keeps the task",
        hint: "",
        takes_arg: false,
        category: Category::Session,
        shortcut: None,
        suggested: true,
    },
    CommandDef {
        id: CmdId::Copy,
        name: "copy",
        aliases: &[],
        label: "Copy last answer",
        desc: "Copy to clipboard",
        hint: "",
        takes_arg: false,
        category: Category::Session,
        shortcut: Some("ctrl+y"),
        suggested: false,
    },
    CommandDef {
        id: CmdId::Cost,
        name: "cost",
        aliases: &[],
        label: "Token usage",
        desc: "Show prompt / completion totals",
        hint: "",
        takes_arg: false,
        category: Category::Session,
        shortcut: None,
        suggested: false,
    },
    CommandDef {
        id: CmdId::Settings,
        name: "settings",
        aliases: &["prefs", "config"],
        label: "Configure chat",
        desc: "Chat & sidebar preferences",
        hint: "",
        takes_arg: false,
        category: Category::Session,
        shortcut: None,
        suggested: true,
    },
    CommandDef {
        id: CmdId::Goal,
        name: "goal",
        aliases: &[],
        label: "Set a goal",
        desc: "Autonomous agent loop",
        hint: "[objective]",
        takes_arg: true,
        category: Category::Session,
        shortcut: None,
        suggested: true,
    },
    CommandDef {
        id: CmdId::Resume,
        name: "resume",
        aliases: &["sessions", "history", "load"],
        label: "Resume session",
        desc: "Browse and resume saved chats",
        hint: "",
        takes_arg: false,
        category: Category::Session,
        shortcut: None,
        suggested: true,
    },
    CommandDef {
        id: CmdId::About,
        name: "about",
        aliases: &["help", "version"],
        label: "About",
        desc: "A little info about Hive",
        hint: "",
        takes_arg: false,
        category: Category::Session,
        shortcut: None,
        suggested: true,
    },
    CommandDef {
        id: CmdId::Quit,
        name: "quit",
        aliases: &["q", "exit"],
        label: "Quit",
        desc: "Exit hive",
        hint: "",
        takes_arg: false,
        category: Category::Session,
        shortcut: Some("ctrl+q"),
        suggested: false,
    },
];

/// Commands matching the typed slash prefix (canonical name or any alias).
/// Each command appears at most once.
pub fn filtered(prefix: &str) -> Vec<&'static CommandDef> {
    let prefix = prefix.to_ascii_lowercase();
    COMMANDS
        .iter()
        .filter(|c| matches_prefix(c, &prefix))
        .collect()
}

fn matches_prefix(c: &CommandDef, prefix: &str) -> bool {
    c.name.starts_with(prefix) || c.aliases.iter().any(|a| a.starts_with(prefix))
}

/// True when `name` would collide with a built-in slash command.
pub fn is_builtin_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    COMMANDS
        .iter()
        .any(|c| c.name == name || c.aliases.iter().any(|a| *a == name))
}

/// Resolve a typed slash name (canonical or alias) to its definition.
pub fn resolve(name: &str) -> Option<&'static CommandDef> {
    let name = name.to_ascii_lowercase();
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.iter().any(|a| *a == name))
}

/// Palette rows for a search query, grouped by category (Suggested first).
/// When `query` is empty, Suggested shows `suggested` commands; otherwise
/// all matching commands are listed under their home category (no Suggested).
pub fn palette_rows(query: &str) -> Vec<PaletteRow> {
    let q = query.trim().to_ascii_lowercase();
    let mut rows = Vec::new();

    if q.is_empty() {
        let suggested: Vec<_> = COMMANDS.iter().filter(|c| c.suggested).collect();
        if !suggested.is_empty() {
            rows.push(PaletteRow::Header(Category::Suggested));
            for c in suggested {
                rows.push(PaletteRow::Command(c));
            }
        }
        for cat in [Category::Session] {
            // Suggested items stay under Suggested only when browsing.
            let list: Vec<_> = COMMANDS
                .iter()
                .filter(|c| c.category == cat && !c.suggested)
                .collect();
            if list.is_empty() {
                continue;
            }
            if !rows.is_empty() {
                rows.push(PaletteRow::Spacer);
            }
            rows.push(PaletteRow::Header(cat));
            for c in list {
                rows.push(PaletteRow::Command(c));
            }
        }
        return rows;
    }

    let matched: Vec<_> = COMMANDS
        .iter()
        .filter(|c| {
            c.label.to_ascii_lowercase().contains(&q)
                || c.name.contains(&q)
                || c.desc.to_ascii_lowercase().contains(&q)
                || c.aliases.iter().any(|a| a.contains(&q))
        })
        .collect();

    let mut cats: Vec<Category> = matched.iter().map(|c| c.category).collect();
    cats.sort_by_key(|c| c.order());
    cats.dedup();

    for cat in cats {
        let items: Vec<_> = matched
            .iter()
            .copied()
            .filter(|c| c.category == cat)
            .collect();
        if items.is_empty() {
            continue;
        }
        if !rows.is_empty() {
            rows.push(PaletteRow::Spacer);
        }
        rows.push(PaletteRow::Header(cat));
        for c in items {
            rows.push(PaletteRow::Command(c));
        }
    }
    rows
}

#[derive(Clone, Copy)]
pub enum PaletteRow {
    Header(Category),
    Command(&'static CommandDef),
    /// Blank breathing room between category groups.
    Spacer,
}

impl PaletteRow {
    pub fn is_selectable(self) -> bool {
        matches!(self, PaletteRow::Command(_))
    }

    pub fn command(self) -> Option<&'static CommandDef> {
        match self {
            PaletteRow::Command(c) => Some(c),
            PaletteRow::Header(_) | PaletteRow::Spacer => None,
        }
    }
}

/// First selectable index in `rows`, or 0 if none.
pub fn first_selectable(rows: &[PaletteRow]) -> usize {
    rows.iter().position(|r| r.is_selectable()).unwrap_or(0)
}

/// Move selection to the next/previous selectable row.
pub fn move_selection(rows: &[PaletteRow], selected: usize, delta: isize) -> usize {
    let selectable: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.is_selectable().then_some(i))
        .collect();
    if selectable.is_empty() {
        return 0;
    }
    let pos = selectable.iter().position(|&i| i == selected).unwrap_or(0);
    let n = selectable.len() as isize;
    let next = (pos as isize + delta).rem_euclid(n) as usize;
    selectable[next]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_and_new_resolve_to_same_command() {
        let clear = resolve("clear").unwrap();
        let new = resolve("new").unwrap();
        assert_eq!(clear.id, CmdId::Clear);
        assert_eq!(new.id, CmdId::Clear);
        assert_eq!(clear.name, "clear");
    }

    #[test]
    fn quit_and_exit_resolve_to_same_command() {
        assert_eq!(resolve("quit").unwrap().id, CmdId::Quit);
        assert_eq!(resolve("exit").unwrap().id, CmdId::Quit);
        assert_eq!(resolve("q").unwrap().id, CmdId::Quit);
    }

    #[test]
    fn slash_filter_dedupes_aliases() {
        let items = filtered("n");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, CmdId::Clear);

        let items = filtered("ex");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, CmdId::Quit);
    }

    #[test]
    fn no_image_or_attach_command() {
        assert!(resolve("image").is_none());
        assert!(resolve("attach").is_none());
        assert!(filtered("im").is_empty());
        assert!(filtered("attach").is_empty());
    }

    #[test]
    fn palette_rows_include_suggested() {
        let rows = palette_rows("");
        assert!(rows
            .iter()
            .any(|r| matches!(r, PaletteRow::Header(Category::Suggested))));
        let labels: Vec<_> = rows
            .iter()
            .filter_map(|r| r.command().map(|c| c.label))
            .collect();
        assert!(labels.contains(&"Switch model"));
        assert!(labels.contains(&"Clear"));
        assert!(labels.contains(&"About"));
        // Clear appears in Suggested; may also appear under Session — count unique ids.
        let clear_count = rows
            .iter()
            .filter_map(|r| r.command())
            .filter(|c| c.id == CmdId::Clear)
            .count();
        assert!(clear_count >= 1);
    }

    #[test]
    fn about_aliases_resolve() {
        assert_eq!(resolve("about").unwrap().id, CmdId::About);
        assert_eq!(resolve("help").unwrap().id, CmdId::About);
        assert_eq!(resolve("version").unwrap().id, CmdId::About);
        assert_eq!(resolve("about").unwrap().label, "About");
    }
}
