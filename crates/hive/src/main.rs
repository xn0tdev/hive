//! hive — a native Rust + TUI YOLO coding agent.
//!
//! `main` is intentionally tiny: initialize logging, run first-run setup if
//! needed, then hand off to the composition root in `wire.rs`.

mod bot;
mod config;
mod driver;
mod setup;
mod wire;

use anyhow::Result;

fn print_help() {
    eprintln!(
        "\
hive — YOLO coding agent

Usage:
  hive                Start the TUI (runs setup on first launch)
  hive bot            Open the hive bot hub (persistent agent personas)
  hive --continue     Reopen the most recent session (-c)
  hive --resume <id>  Reopen a session by id (no id = most recent)
  hive --intro        Force the setup wizard
  hive --help         Show this help

Hive prints the id of the session you were in when you quit.
"
    );
}

/// Parse `--continue` / `-c` / `--resume [id]` into a resume target.
fn parse_resume(args: &[String]) -> Option<wire::Resume> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--continue" | "-c" => return Some(wire::Resume::Last),
            "--resume" | "-r" => {
                // A bare `--resume` means the same as `--continue`.
                return Some(match it.next() {
                    Some(id) if !id.starts_with('-') => wire::Resume::Id(id.clone()),
                    _ => wire::Resume::Last,
                });
            }
            _ => {}
        }
    }
    None
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    let force_intro = args.iter().any(|a| a == "--intro");
    let bot_mode =
        args.first().map(|a| a.as_str()) == Some("bot") || args.iter().any(|a| a == "--bot");

    let _log_guard = wire::init_tracing();

    let preliminary = match config::try_load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("hive: {e:#}");
            std::process::exit(1);
        }
    };

    match setup::ensure_ready(preliminary, force_intro).await {
        Ok(cfg) if bot_mode => crate::bot::run(cfg).await,
        Ok(cfg) => wire::run(cfg, parse_resume(&args)).await,
        Err(e) => {
            eprintln!("hive: {e:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resume_flags_parse() {
        assert!(parse_resume(&args(&[])).is_none());
        assert!(matches!(
            parse_resume(&args(&["--continue"])),
            Some(wire::Resume::Last)
        ));
        assert!(matches!(
            parse_resume(&args(&["-c"])),
            Some(wire::Resume::Last)
        ));
        assert!(matches!(
            parse_resume(&args(&["--resume", "s_1_abcd"])),
            Some(wire::Resume::Id(id)) if id == "s_1_abcd"
        ));
        // A bare --resume, or one followed by another flag, means "most recent".
        assert!(matches!(
            parse_resume(&args(&["--resume"])),
            Some(wire::Resume::Last)
        ));
        assert!(matches!(
            parse_resume(&args(&["--resume", "--intro"])),
            Some(wire::Resume::Last)
        ));
    }
}
