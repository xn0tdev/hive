//! hive — a native Rust + TUI YOLO coding agent.
//!
//! `main` is intentionally tiny: initialize logging, load config, hand off to
//! the composition root in `wire.rs`.

mod config;
mod driver;
mod wire;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = wire::init_tracing();

    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("hive: {e:#}");
            std::process::exit(1);
        }
    };

    wire::run(cfg).await
}
