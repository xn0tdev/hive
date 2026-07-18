//! The slash command catalogue: one source of truth for the autocomplete menu
//! and the key handler.

pub struct SlashCmd {
    pub name: &'static str,
    /// Argument hint shown next to the name, e.g. "[id-or-role]".
    pub hint: &'static str,
    pub desc: &'static str,
    /// Commands that take an argument are completed (not executed) on Enter.
    pub takes_arg: bool,
}

pub const COMMANDS: &[SlashCmd] = &[
    SlashCmd {
        name: "model",
        hint: "[id·role]",
        desc: "Switch model",
        takes_arg: true,
    },
    SlashCmd {
        name: "image",
        hint: "<path>",
        desc: "Attach an image",
        takes_arg: true,
    },
    SlashCmd {
        name: "copy",
        hint: "",
        desc: "Copy last answer",
        takes_arg: false,
    },
    SlashCmd {
        name: "clear",
        hint: "",
        desc: "New conversation",
        takes_arg: false,
    },
    SlashCmd {
        name: "cost",
        hint: "",
        desc: "Token usage",
        takes_arg: false,
    },
    SlashCmd {
        name: "help",
        hint: "",
        desc: "Show all commands",
        takes_arg: false,
    },
    SlashCmd {
        name: "quit",
        hint: "",
        desc: "Exit hive",
        takes_arg: false,
    },
];

/// Commands whose name starts with the given prefix (what the user typed
/// after `/`, before any space).
pub fn filtered(prefix: &str) -> Vec<&'static SlashCmd> {
    COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(prefix))
        .collect()
}
