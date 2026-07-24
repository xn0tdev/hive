//! hive — a native Rust + TUI YOLO coding agent.
//!
//! `main` is intentionally tiny: initialize logging, run first-run setup if
//! needed, then hand off to the composition root in `wire.rs`.

mod acp;
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
  hive           Start the TUI (runs setup on first launch)
  hive --acp     Run as an ACP server (JSON-RPC over stdio)
  hive --intro   Force the setup wizard
  hive --help    Show this help
"
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    let force_intro = args.iter().any(|a| a == "--intro");
    let acp_mode = args.iter().any(|a| a == "--acp");

    let _log_guard = wire::init_tracing();

    let preliminary = match config::try_load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("hive: {e:#}");
            std::process::exit(1);
        }
    };

    let cfg = if acp_mode {
        config::load()?
    } else {
        match setup::ensure_ready(preliminary, force_intro).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("hive: {e:#}");
                std::process::exit(1);
            }
        }
    };

    if acp_mode {
        acp::run(cfg).await
    } else {
        wire::run(cfg).await
    }
}
