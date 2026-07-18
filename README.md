# hive

Native Rust + TUI YOLO coding agent. You say it, hive does it — tools run without confirmations.

The crates.io package is **`hive-agent`** (the name `hive` was taken). The binary is still `hive`.

## Install

```bash
cargo install hive-agent
```

Or from source:

```bash
cargo run -p hive-agent
```

## Config

`~/.config/hive/config.toml`

```toml
[provider]
api_key = "fw_..."
```

Or set `FIREWORKS_API_KEY`.

## Layout

```
lib/comb          TUI engine ([comb-tui](https://crates.io/crates/comb-tui))
crates/hive-core  agent loop, tools, swarm
crates/hive-llm   LLM provider
crates/hive-tui   terminal UI
crates/hive       binary (package: hive-agent)
```

## License

MIT
