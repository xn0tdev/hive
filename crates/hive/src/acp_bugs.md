# ACP Bugs & Issues — Research Notes

Collected from other ACP implementations and cross-referenced against our code.
Not fixed yet — just catalogued.

---

## Bugs found in other ACP implementations

### 1. session/new fails if called immediately after initialize
**Source:** charmbracelet/crush#2587
**Severity:** High — breaks standard protocol flow

`initialize` response is sent before internal setup (config, providers, model) is
complete. `session/new` arrives during this window → Internal error (-32603).

**Our status:** We build the agent *before* entering the read loop, so
`initialize` returns only after the agent is ready. **Safe**, but
`session/new` calls `rebuild_agent_in` which reconstructs the system prompt
synchronously — if this is slow, the client could time out.

---

### 2. session/update notifications arrive AFTER session/prompt response
**Source:** anomalyco/opencode#18672, anomalyco/opencode#17505
**Severity:** High — violates ACP spec

> The Agent MUST ensure that it does [send session/update] before responding to
> the session/prompt request.

OpenCode's `prompt()` returns `{ stopReason: "end_turn" }` before the event
pipeline finishes draining. Tail-end events (final message parts, usage) arrive
after the response.

**Our status: BUG** — `drain_events` runs in a separate tokio task. When
`run_turn` returns, we immediately write the `stopReason` response. But
`drain_events` may still have queued events in the unbounded channel that haven't
been written to stdout yet. The `session/prompt` response can arrive before the
last `session/update` notification. This is a **race condition**.

**Fix needed:** After `run_turn` returns, drain the event channel to empty
before writing the `stopReason` response. Or use a sync barrier.

---

### 3. session/prompt hangs — never returns stopReason
**Sources:** Kilo-Org/kilocode#10768, NousResearch/hermes-agent#39245, NousResearch/hermes-agent#50765
**Severity:** Critical — client hangs forever

Multiple causes across implementations:
- Provider API error not caught → prompt() never returns
- `usage_update` / `session_info_update` hangs → blocks final response
- Windows-specific: turn prologue hangs on memory prefetch / provider init

**Our status: BUG** — If `provider.chat_stream` errors, `Agent::run_turn`
emits `AgentEvent::Error` and breaks the inner loop, then returns an empty
string. We always return `end_turn` regardless. But if the provider hangs
indefinitely (network timeout with no tokio timeout), `run_turn` never returns
and `session/prompt` hangs forever. **No timeout on the turn.**

Also: `AgentEvent::Error` is mapped to `agent_message_chunk` (text), not to an
ACP error. The client sees it as a message, not a failure. The turn still
returns `end_turn` even when the agent errored. Should return `"refusal"` or
emit a proper error.

---

### 4. I/O backpressure — stdout pipe deadlock (64KB)
**Source:** agent-of-empires/agent-of-empires#2455
**Severity:** Critical — agent freezes mid-stream

The agent's stdout fills the OS pipe buffer (64KB on Linux). If the client
isn't reading fast enough, `write()` blocks. The agent freezes mid-turn until
something pokes stdin. All processes stay alive and idle.

**Our status: POTENTIAL BUG** — `write_msg` uses `tokio::io::Stdout` with
`AsyncWriteExt::write_all`. If the client's read buffer fills, our write will
block (await). `drain_events` holds the stdout Mutex while writing. If it
blocks, no other notifications can be written, and the main loop can't write
responses either. **No timeout on stdout writes.**

---

### 5. MCP servers silently ignored in session/new
**Source:** Cursor forum bug report
**Severity:** Medium

Agent advertises `mcpCapabilities` in `initialize` but never connects to MCP
servers passed in `session/new`. Silently accepted, tools never available.

**Our status:** We don't advertise MCP capabilities and don't parse `mcpServers`
from `session/new` params. **Not a bug for us** (we don't claim to support it),
but it means clients that send MCP servers will get no tools from them.

---

### 6. session/prompt returns "Session not found" for valid session ID
**Source:** XiaomiMiMo/MiMo-Code#499
**Severity:** High

Session is created successfully but immediately lost — `session/prompt` can't
find it. Caused by session store race or async session creation not completing.

**Our status:** We don't have a session store — we keep a single `Agent` in a
Mutex and just set `session_id` as a string. `session/prompt` doesn't validate
the `sessionId` against `session/new`'s returned ID. **Not a bug**, but we
should validate to reject stale/wrong session IDs.

---

### 7. Cancellation doesn't cascade to pending tool calls
**Source:** ACP spec — cancellation section

Per spec: when `session/cancel` is received, the agent MUST:
1. Stop all LLM requests and tool call invocations
2. Send `cancelled` status for all non-finished tool calls
3. Respond to the original `session/prompt` with `stopReason: "cancelled"`

**Our status: PARTIAL BUG** — We set `interrupt` to `true`, which stops the
LLM stream and tool execution. But:
- We don't emit `tool_call_update` with `status: "cancelled"` for in-flight
  tools
- We always return `stopReason: "end_turn"`, never `"cancelled"`
- `session/cancel` is a notification (no `id`), but we don't have a way to
  know which `session/prompt` request to respond to with `cancelled`

---

### 8. No `$/cancel_request` support
**Source:** ACP spec — cancellation section

ACP defines `$/cancel_request` notification for general JSON-RPC request
cancellation. We don't handle it.

**Our status: MISSING** — Only `session/cancel` is handled. `$/cancel_request`
is silently ignored.

---

## Bugs/gaps in our implementation (vs spec)

### 9. session/cancel can't interrupt — blocks on agent Mutex
**Severity:** High

`session/cancel` is handled in the main read loop. But during `session/prompt`,
the main loop is blocked on `agent.run_turn().await` — it's not reading stdin.
So `session/cancel` can't be processed until the turn finishes.

The `interrupt` flag is set by the main loop, which isn't running during a turn.

**Fix needed:** Spawn a separate stdin reader for cancel notifications, or use
`tokio::select!` between `run_turn` and stdin reads during a prompt.

---

### 10. No messageId in agent_message_chunk
**Source:** ACP spec — prompt turn, message IDs section

> The Agent MAY include an opaque, unique messageId on message chunks. Chunks
> with the same messageId belong to the same message.

**Our status: MISSING** — We never set `messageId`. Clients can't distinguish
message boundaries when the agent produces multiple messages in one turn.

---

### 11. No plan notifications
**Source:** ACP spec — agent plan section

Agents SHOULD report execution plans via `session/update` with
`sessionUpdate: "plan"`.

**Our status: MISSING** — `AgentEvent::PlanUpdated` is mapped to nothing
(falls into the `_ => return None` catch-all). We have plan data but don't
emit it as an ACP plan notification.

---

### 12. No session modes advertised
**Source:** ACP spec — session modes section

`session/new` response MAY include `modes` with `currentModeId` and
`availableModes`. We have MAKE/PLAN/MULTITASK modes but don't expose them.

**Our status: MISSING** — `session/new` returns only `{ "sessionId": ... }`.
Should include modes. Also `session/set_mode` is not implemented.

---

### 13. No thought_chunk distinction from agent_message_chunk
**Source:** ACP spec — content section

`thought_chunk` is for reasoning/thinking. `agent_message_chunk` is for final
assistant text. We map `ReasoningDelta` → `thought_chunk` and
`AssistantTextDelta` → `agent_message_chunk`, which is correct. But
`AssistantMessage` (the finalized markdown re-render) is also sent as
`agent_message_chunk` — this duplicates the content the client already
received via `AssistantTextDelta` deltas. Some clients may render both.

**Our status: BUG** — `AssistantMessage` should probably not be sent as a
separate `agent_message_chunk` after the deltas, or it should use the same
`messageId` so the client knows to replace, not append.

---

### 14. No tool_call_update with in_progress status
**Source:** ACP spec — tool calls section

After `tool_call` (pending), agent SHOULD send `tool_call_update` with
`status: "in_progress"` before execution starts. We only send `pending` →
`completed`/`failed`, skipping `in_progress`.

**Our status: MISSING** — `AgentEvent::ToolStarted` → `pending`,
`AgentEvent::ToolFinished` → `completed`/`failed`. No `in_progress` event.
The agent core doesn't emit an `in_progress` event — it goes straight from
`ToolStarted` to executing the tool.

---

### 15. No tool_call raw params/output
**Source:** ACP spec — tool calls section

`tool_call` notification MAY include `rawInput` (raw params) and `rawOutput`
(raw result). We only include `title` and `kind`.

**Our status: MISSING** — `AgentEvent::ToolStarted` has `args_preview` (a
truncated string) but we don't include it. `AgentEvent::ToolFinished` has
`summary` but not the raw output. Not required by spec (MAY), but useful for
clients.

---

### 16. No file locations in tool calls
**Source:** ACP spec — following the agent section

Tool calls can report `locations` — file paths being accessed/modified, with
optional line numbers. Enables "follow-along" in the editor.

**Our status: MISSING** — We don't extract or report file paths from tool
calls.

---

### 17. No diff content in tool_call_update
**Source:** ACP spec — tool calls, diffs section

Tool calls that modify files SHOULD include diff content:
```json
{ "type": "diff", "path": "...", "oldText": "...", "newText": "..." }
```

**Our status: MISSING** — `write_file` / `edit_file` tools produce diffs
internally but we don't capture or emit them as ACP diff content.

---

### 18. No session/config_options or session/set_config_option
**Source:** ACP spec — session config options section

Agents can expose configurable options (model selection, etc.) via
`configOptions` in `session/new` response. Clients can set them via
`session/set_config_option`.

**Our status: MISSING** — Not implemented. Model switching is not available
over ACP.

---

### 19. No session/list, session/load, session/delete, session/resume
**Source:** ACP spec — session lifecycle

We advertise `loadSession: false` in `initialize`, which is correct. But
`session/list`, `session/delete` etc. are not implemented. Not a bug since we
don't advertise the capability, but limits functionality.

---

### 20. No authenticate/logout methods
**Source:** ACP spec — authentication section

We return `authMethods: []` in `initialize`, which is correct. We handle auth
via config file / env vars. Not a bug, but some clients may require
`authenticate` before `session/new`.

---

### 21. No available_commands_update notification
**Source:** ACP spec — slash commands section

Agents can advertise slash commands to the client. We have slash commands
(`/model`, `/compact`, `/clear`, skills) but don't emit them as ACP
`available_commands_update`.

**Our status: MISSING**

---

### 22. No usage_update with cost field
**Source:** ACP spec — prompt turn, usage section

`usage_update` MAY include `cost: { amount, currency }`. We only send
`used` and `size`, no cost.

**Our status: MISSING** — We don't track dollar costs.

---

### 23. No terminal/create support (client-side terminals)
**Source:** ACP spec — terminals section

ACP defines `terminal/create`, `terminal/wait`, `terminal/kill`,
`terminal/release` for agent-initiated terminals that render in the client UI.
We have our own embedded terminal but don't expose it via ACP terminal methods.

**Our status: MISSING** — Not advertised in capabilities. Our terminal tools
run server-side (in the agent process), not via the client.

---

## Priority ranking

| # | Bug | Severity | Fix effort |
|---|-----|----------|------------|
| 2 | session/update after session/prompt response (race) | Critical | Medium |
| 9 | session/cancel can't interrupt mid-turn | Critical | Medium |
| 3 | session/prompt hangs on provider error / no timeout | Critical | Medium |
| 4 | stdout pipe deadlock under backpressure | Critical | Hard |
| 7 | Cancellation doesn't return "cancelled" stopReason | High | Medium |
| 13 | AssistantMessage duplicates delta content | Medium | Low |
| 14 | No tool_call in_progress status | Medium | Low |
| 10 | No messageId in chunks | Medium | Low |
| 11 | No plan notifications | Medium | Low |
| 12 | No session modes advertised | Medium | Medium |
| 17 | No diff content in tool calls | Medium | Medium |
| 16 | No file locations in tool calls | Medium | Medium |
| 8 | No $/cancel_request support | Low | Low |
| 6 | No sessionId validation | Low | Low |
| 21 | No available_commands_update | Low | Low |
| 22 | No cost in usage_update | Low | Low |
| 18 | No session/config_options | Low | Medium |
| 15 | No raw params/output in tool calls | Low | Low |
| 20 | No authenticate/logout | Low | N/A |
| 19 | No session/list/load/delete | Low | N/A |
| 23 | No terminal/create | Low | N/A |
| 5 | No MCP server support | Low | N/A |
| 1 | session/new race after initialize | Low | N/A (already safe) |
