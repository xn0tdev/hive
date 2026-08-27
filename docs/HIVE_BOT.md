# Hive Bot — second hub for working with persistent agent personas

Design spec for the 0.2.5 feature cycle. Main hive (plan/make/multitask TUI)
stays untouched; hive bot is an additional entry point with its own view,
its own chat model, and bot-only internal tools.

## Concept

The user creates their own agents ("personas") as markdown files dropped into
the scope's `agents/` dir: name, description of what the agent does and how it
works, plus an optional role prompt. One persona usually acts as the manager
the human talks to; other personas are specialists the manager delegates to.

Example flow (illustrative names chosen by the user):

> Human -> Vasya: check current issues on GitHub
>
> Vasya -> Developer (private channel): hey, Artem asked me to look into
> current problems — check the GitHub issues
>
> Developer: hi! give me a minute
>
> Developer: done, here's what I found ...

Vasya waits while Developer works, but may talk to another specialist at the
same time. The human sees the main chat with Vasya normally; sub-chats are
shown as cards that can be opened to watch the conversation.

## Two hubs / two scopes

| Scope | Agents live in | Meaning |
|---|---|---|
| Project | `<project>/.hive/agents/*.md` | Personas belong to this repo/project |
| Global | `~/.hive/agents/*.md` | Available everywhere, launchable from any directory |

Same personas are passed along: a global persona can be talked to from any
project; project personas layer on top (name collision -> project wins).

## No worktrees

Bot agents do NOT run in git worktrees. They work in the real project
directory (which also must work outside git repositories). Coordination has
to stay coherent: sequential or clearly separated responsibilities instead of
parallel worktree-style merging. Refinement of exact coordination rules is
tracked in the task list.

## One long session, memory instead of history resets

A chat with a persona is a single continuous session — not one throwaway
session per task. Context handling:

- Compaction still applies when the window fills, but valuable facts survive
  in **memory files**: plain local notes under the scope's data dir
  (`<project>/.hive/memory/`, respectively `~/.hive/memory/`).
- Bot-only internal tools let the model write memories and search them back
  (`memory_write`, `memory_search`). The model decides when to persist and
  recall; recall goes through convenient full-text search, not context dumps.
- These tools exist ONLY in hive bot mode.

## Sub-chats between agents

- Each manager <-> specialist conversation is its own channel with its own
  message list; agents never see each other's raw tool traffic unless a chat
  explicitly passes it along.
- Channels are asynchronous: while a specialist works, the manager can open
  another channel. The scheduler keeps turns coherent.
- UI: the main chat renders inline; every channel appears as a plaque/card;
  opening it shows that conversation transcript.

## MCP support (main hive)

Simple, clean MCP client support for the regular hive agent (not bot):

- Config-declared stdio servers (`command` + args), JSON-RPC over stdio.
- On startup: `initialize`, `tools/list`; server tools are exposed through
  the existing core `Tool` trait so the rest of the pipeline never knows.
- Tool calls map to `tools/call`.

## Delivery phases

1. **Groundwork (done)** — persona files format + loader/saver in core.
2. **Bot hub shell (done)** — chat rail, persona picker, long-session chat.
   The in-TUI create form was cut: personas come from markdown files on disk.
3. **Memory** — memory dirs, bot-only tools, compaction hand-off.
4. **Channels** — manager->specialist sub-chats, async scheduler, cards UI.
5. **MCP** — stdio MCP client wired into main hive toolset.
