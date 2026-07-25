# Hive — Zed Extension

Installs [Hive](https://github.com/xn0tdev/hive) as an ACP agent server in Zed's Agent Panel.

## What this does

This extension registers Hive as an agent server using the Agent Client Protocol (ACP). After installation, Hive appears in Zed's agent panel and can be used for coding tasks directly from the editor.

## Prerequisites

Hive needs a configured provider API key before it can run. Either:

1. Run `hive --intro` to use the setup wizard (provider → model → search), or
2. Create `~/.config/hive/config.toml` manually with your API key:

```toml
[provider]
base_url = "https://api.fireworks.ai/inference/v1"
api_key = "fw_..."

[models]
default = { id = "accounts/fireworks/routers/kimi-k2p6-fast", name = "Kimi Fast" }
```

See the [Hive README](https://github.com/xn0tdev/hive) for full configuration details.

## Installation

This extension is published to the Zed extension registry. Install it from Zed's extension browser, or by opening a PR to [`zed-industries/extensions`](https://github.com/zed-industries/extensions) adding this repo as a submodule.

## How it works

The extension downloads a platform-specific Hive binary from GitHub Releases and launches it with `--acp` (ACP server mode over stdio). The binary communicates with Zed via JSON-RPC 2.0.

## Platforms

- macOS ARM (darwin-aarch64)
- macOS Intel (darwin-x86_64)
- Linux x86_64 (linux-x86_64)
- Windows x86_64 (windows-x86_64)

## License

MIT
