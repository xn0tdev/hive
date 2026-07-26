//! The line the shell keeps after the TUI is gone.
//!
//! Printed once the alternate screen is torn down, so it lands in the user's
//! scrollback: the wordmark, and how to get back into the session they just
//! left.

use std::io::{IsTerminal, Write};

use crate::render::wordmark;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

/// Print the wordmark, plus a resume hint when there's a saved session.
pub fn print(resume_id: Option<&str>) {
    let mut out = std::io::stderr();
    let _ = out.write_all(render(resume_id, out.is_terminal()).as_bytes());
    let _ = out.flush();
}

/// The farewell as text. `color` is off when stderr isn't a terminal, so a
/// redirected run gets clean output instead of escape codes.
fn render(resume_id: Option<&str>, color: bool) -> String {
    let (bold, dim, reset) = if color {
        (BOLD, DIM, RESET)
    } else {
        ("", "", "")
    };

    let mut out = String::from("\n");
    for row in wordmark::ART {
        out.push_str(&format!("  {bold}{row}{reset}\n"));
    }
    if let Some(id) = resume_id {
        out.push_str(&format!("\n  {dim}resume:{reset}  hive --resume {id}\n"));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_output_has_no_escape_codes() {
        let text = render(Some("s_1_abcd"), false);
        assert!(!text.contains('\x1b'), "{text:?}");
        assert!(text.contains("hive --resume s_1_abcd"));
        assert!(text.contains('█'));
    }

    #[test]
    fn colored_output_resets_every_sequence() {
        let text = render(Some("s_1_abcd"), true);
        let opened = text.matches(BOLD).count() + text.matches(DIM).count();
        assert!(opened > 0);
        assert_eq!(
            opened,
            text.matches(RESET).count(),
            "every opened sequence must be closed: {text:?}"
        );
    }

    #[test]
    fn without_a_session_there_is_no_hint() {
        let text = render(None, false);
        assert!(!text.contains("resume"), "{text:?}");
        assert!(text.contains('█'));
    }
}
