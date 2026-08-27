# AGENTS.md

Notes for humans and agents working on this repo.

## Naming

| What | Name |
|------|------|
| GitHub repo | `sqweeet/hive` |
| crates.io binary package | **`hive-agent`** (`hive` is taken) |
| Installed command | **`hive`** (`[[bin]] name = "hive"`) |
| TUI engine (crates.io) | **`comb-tui`** (`use comb::`) — separate repo `xn0tdev/comb` |
| Local comb package | `lib/comb` → package `comb-tui`, lib name `comb` |

`cargo install hive-agent` installs the **`hive`** binary into Cargo’s bin dir (`~/.cargo/bin` by default). If that dir is on `PATH`, you can run `hive` from anywhere.

## Workspace

```
lib/comb            → comb-tui (publish from github.com/xn0tdev/comb, publish=false here)
crates/hive-core    → hive-core
crates/hive-llm     → chat client + `catalog` (GET /models + models.dev)
crates/hive-tui     → TUI + first-run intro wizard
crates/hive         → hive-agent (binary: hive)
```

**Intro:** on first run (no provider key) Hive opens a TUI setup wizard (provider → one model via `hive_llm::catalog` → search).

Version and license live in the root `[workspace.package]`. Crates use `version.workspace = true`.

## Release checklist

Do this every release. Do **not** skip the version bump.

1. **Bump version** in root `Cargo.toml` → `[workspace.package].version`  
   (all crates share it; also bump `comb` path dep versions in `[workspace.dependencies]` if they pin `version = "…"`).
2. If **comb** changed API/behavior worth shipping:
   - release from `/path/to/comb` (or sync `lib/comb` → `xn0tdev/comb`)
   - bump comb’s version, tag, `cargo publish` as `comb-tui`
   - bump the `comb = { … version = "…" }` pin in this workspace
3. Update README / changelog one-liners if needed (keep them short).
4. Commit on `main` (clean tree preferred before publish).
5. **Publish in dependency order** (wait until each is available on crates.io):

   ```bash
   cargo publish -p hive-core
   cargo publish -p hive-llm
   cargo publish -p hive-tui
   cargo publish -p hive-agent
   ```

   Do **not** `cargo publish -p comb-tui` from this monorepo (`publish = false`). Use the comb repo.
6. Tag the hive release: `git tag -a vX.Y.Z -m "vX.Y.Z"` and `git push origin main --tags`.
7. Smoke-check:

   ```bash
   cargo install hive-agent --force
   hive --help   # or just `hive`
   ```

### Version gotchas

- Forgetting the workspace version → crates.io rejects republish of the same version.
- Publishing `hive-tui` / `hive-agent` before deps are live → resolve errors; wait or retry.
- Path + `version` in workspace deps must match the version you publish.
- Binary name is **`hive`**, package name is **`hive-agent`** — don’t rename the bin when “fixing” the crates.io name.

## Branch workflow

**Never commit directly to `main`.** All work accumulates in `development` throughout the day.

- `main` is the default branch on GitHub — stable, only updated via merge from `development`.
- `development` is the working branch — it lives on the local machine and on `origin`.
- Locally you work **only** in `development`; `main` exists on `origin` and is updated by merging.
- Commit freely to `development` as you go; small commits are fine there.
- `development` on `origin` doubles as a public beta — anyone can check it out to see work in progress before it lands in `main`.
- At the end of the day (or when the work is verified), merge `development` → `main` as one reviewed batch — no trickle of small commits into `main`.
- If the day's work isn't verified yet, keep going in `development` the next day.
- Merge via `gh` CLI (always available, authenticated) or plain `git` — whichever is simpler. Never force-push or reset `main`.
- Don't accidentally switch `main` to a different commit; merge into it, don't rebase it onto `development`.

## Local dev

```bash
cargo run -p hive-agent
# or
cargo run -p hive   # may fail; package is hive-agent
```

Prefer: `cargo run -p hive-agent`.

Config: `~/.config/hive/config.toml` (or `FIREWORKS_API_KEY`).

## Don’t

- Don’t commit directly to `main` — use `development`.
- Don’t try to publish crates.io name `hive` or `comb`.
- Don’t invent parallel version numbers per crate unless you intentionally break the workspace version.
- Don’t commit secrets / API keys / `~/.config/hive`.
