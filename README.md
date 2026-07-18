# hive

Native Rust + TUI YOLO coding agent. You say it, hive does it — tools run without confirmations.

## Run

```bash
cargo run -p hive
```

Config: `~/.config/hive/config.toml`

```toml
[provider]
api_key = "fw_..."
```

Or set `FIREWORKS_API_KEY`.

## Layout

```
lib/comb          TUI engine
crates/hive-core  agent loop, tools, swarm
crates/hive-llm   LLM provider
crates/hive-tui   terminal UI
crates/hive       binary
```

## License

MIT
