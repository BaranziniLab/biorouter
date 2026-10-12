# Hooks system — architecture review

> **What this is.** One of ten subsystem reviews from the 2026-07 BioRouter agentic-loop review. It documents BioRouter's Claude-Code-compatible hook system — the 13 wired event variants, command versus prompt hooks, the outcome and decision model, configuration and matchers — and records thirteen gaps.
> **Status:** Historical record — a snapshot of the code *before* the agent-loop fix campaign, whose findings were then implemented. Gap #1 (`PreToolUse` `additionalContext` silently dropped) and gap #2 (`PostToolUse` observe-only) were fixed by BR-19, #5 (unbounded injected hook stdout) by BR-26, #8 (name-only matchers) by BR-27, #3 (`fire()` aggregates discarded) by BR-28, and #7 (no rewrite path) by BR-19.
> **Audience:** developers working on hooks, guardrails, or the tool-approval path.

Identifier key: `BR-NN` are proposal ids from the [master improvement-proposal list](../improvement-proposals.md). When other documents cite `hooks.md #N` they mean the numbered items under "Gaps and weaknesses" below (this file's former name), **not** the answer sections — those are deliberately unnumbered here to remove that ambiguity.

## Scope and files reviewed

`crates/biorouter/src/hooks/` (`mod`, `config`, `event`, `matcher`, `inspector`, `outcome`,
`command_runner`, `prompt_runner`) plus every invocation site in `agents/`, `scheduler.rs`, and
`biorouter-cli`. Paths are relative to the repository root.

> **Note.** The review recorded no commit or branch, and line numbers in the citations below have drifted since; treat them as pointers to the right function, not exact locations.

## Overview

BioRouter's hooks are a near-verbatim port of Claude Code's lifecycle-hook model: user-
configured **shell commands** or **LLM-judge prompts** that fire at fixed points in the
agent loop and can *block*, *inject context*, *warn*, or *observe*. The stdin payload and
stdout decision JSON use Claude Code's exact snake_case/camelCase field names so existing
hook scripts port unchanged (`crates/biorouter/src/hooks/event.rs:3`,
`crates/biorouter/src/hooks/outcome.rs:1`).

The design invariant is **failure-open**: a crashing, timing-out, or misconfigured hook
never blocks the agent; only an *explicit* decision (exit code 2, a `block`/`deny` in stdout
JSON, or a prompt-hook `ok:false`) blocks (`crates/biorouter/src/hooks/mod.rs:9`).

Data flow (per event):

```text
agent loop reaches a lifecycle point
  → HooksManager::{pre_tool_use|user_prompt_submit|stop|session_start_once|dispatch|fire}
    → resolved_groups(event, cwd)   = global config groups ++ project groups (if opted-in)
    → filter groups by matcher_matches(matcher, matcher_key)   [tool name / source / trigger]
    → ++ session_hooks (runtime, e.g. /goal Stop judge)
    → run every matching HookDefinition concurrently (join_all)
        command  → sh -c / cmd /C, payload JSON on stdin, cwd=working_dir, hard timeout
                   → interpret_command_result(exit_code, stdout, stderr)
        prompt   → provider.complete_fast(system+rule, payload) → parse {"ok",reason}
    → merge_outcomes()  = most-restrictive decision wins (Deny>Ask>Allow),
                          contexts + system_messages concatenated
  → caller acts on HookAggregate.{decision, additional_context, system_messages}
```

Two dispatch modes exist: `dispatch()` (awaited, decision honored) and `fire()` (fire-and-
forget `tokio::spawn`, aggregate **discarded**) at `crates/biorouter/src/hooks/mod.rs:212`
and `:258`.

Blocking decisions reach the agent through three distinct channels:
`HookInspector` (a `ToolInspector`) for PreToolUse (`crates/biorouter/src/hooks/inspector.rs`),
inline checks in `tool_execution.rs` for PermissionRequest, and inline checks in `agent.rs`
for UserPromptSubmit and Stop.

## Review questions answered

### How many hook events there are, and where each fires

There are **13 event variants** in the `HookEvent` enum
(`crates/biorouter/src/hooks/event.rs:13-27`), and — notably — **every one is actually
wired to a fire site** (no dead enum arms):

| Event | Fires at | Dispatch mode |
|---|---|---|
| `PreToolUse` | `crates/biorouter/src/hooks/inspector.rs:57-60` (via inspector registered at `agents/agent.rs:359-361`), wrapper `hooks/mod.rs:395` | awaited, blockable |
| `PostToolUse` | `agents/agent.rs:1858` (dispatched at `:1882`) | awaited, **observe-only** |
| `PostToolUseFailure` | `agents/agent.rs:1856` | awaited, **observe-only** |
| `PermissionRequest` | `agents/tool_execution.rs:73-76` (wrapper `hooks/mod.rs:418`) | awaited, blockable (allow/deny) |
| `UserPromptSubmit` | `agents/agent.rs:1294-1296` (wrapper `hooks/mod.rs:441`) | awaited, blockable |
| `Stop` | `agents/agent.rs:2140` (wrapper `hooks/mod.rs:460`) | awaited, blockable (capped) |
| `SubagentStart` | `agents/subagent_handler.rs:148` | `fire()`, discarded |
| `SubagentStop` | `agents/agent.rs:2130` | `fire()`, discarded |
| `SessionStart` | `agents/agent.rs:1289-1290` (wrapper `hooks/mod.rs:509`) | awaited, context-only |
| `SessionEnd` | `scheduler.rs:909` and `biorouter-cli/src/session/mod.rs:456` | awaited, observe-only |
| `Notification` | `agents/tool_execution.rs:142` | `fire()`, discarded |
| `PreCompact` | `agents/agent.rs:1471,1991`; `agents/execute_commands.rs:99` (via `fire_compaction_hook`, `agent.rs:317-335`) | `fire()`, discarded |
| `PostCompact` | `agents/agent.rs:1482,2004`; `agents/execute_commands.rs:118` | `fire()`, discarded |

`supports_blocking()` (`event.rs:68-77`) declares only `PreToolUse | PermissionRequest |
UserPromptSubmit | Stop` as blockable — but this method is **dead code**: `grep` finds no
caller. Blocking is enforced ad hoc at each call site, not gated by this helper.

The enum is `#[non_exhaustive]` and config parsing skips unknown event names with a warning
rather than failing (`config.rs:78-95`), so the schema is forward-compatible.

### Command hooks versus prompt hooks, and what a hook can do

**Command hooks** (`HookDefinition::Command`, `config.rs:22-28`) run via
`command_runner::run_command_hook` (`command_runner.rs:25`): `sh -c <command>` (or
`cmd /C` on Windows, `:37-45`), cwd = session working dir, event payload JSON piped to
stdin (written on a separate task so a hook that ignores stdin cannot wedge the writer,
`:58-66`), `kill_on_drop(true)`, and a hard `tokio::time::timeout` (`:75-82`, default 60s
`mod.rs:39`). Three env vars are injected: `BIOROUTER_HOOK_EVENT`, `BIOROUTER_SESSION_ID`,
`BIOROUTER_PROJECT_DIR` (`mod.rs:285-298`). Exit-code semantics
(`outcome.rs:137-166`): `0` → parse stdout JSON (or treat raw stdout as context for
UserPromptSubmit/SessionStart only); `2` → **Deny** with stderr as reason; anything else →
non-blocking error (failure-open).

**Prompt hooks** (`HookDefinition::Prompt`, `config.rs:31-42`) run an LLM judge
(`prompt_runner.rs:45`). A fixed system prompt (`:15-21`) plus the author's rule is sent
with the event payload to `provider.complete_fast(...)` (`:57`); the judge must answer
`{"ok":bool,"reason":string}`, tolerantly parsed from fenced/prose output (`:32-41`).
`ok:false` → **Deny**; parse failure / provider error / timeout (default 30s `mod.rs:41`)
→ non-blocking error. Provider selection: an explicit `provider:`+`model:` pair builds and
caches a dedicated provider, otherwise the agent's own provider (fast model) is reused
(`mod.rs:352-391`).

What a hook **can do**:
- **Block** — `deny` (PreToolUse/PermissionRequest), `block`/`continue:false`
  (Stop/UserPromptSubmit), exit 2, or prompt `ok:false`.
- **Route to approval** — `ask` on PreToolUse becomes `RequireApproval`
  (`inspector.rs:73-83`).
- **Auto-approve** — `allow` on PermissionRequest skips the user prompt and dispatches the
  tool immediately (`tool_execution.rs:81-100`).
- **Inject context** — `additionalContext` (or raw stdout for UserPromptSubmit/SessionStart)
  becomes a hidden `<hook-context>` user message (see the outcome model below).
- **Warn** — `systemMessage` surfaces as a yellow inline notice
  (`outcome.rs:23`, rendered at `agent.rs:1889-1898`).

What a hook **cannot do**: mutate `tool_input`/tool output, add tools, or change the model's
response. There is no rewrite path anywhere.

### Where hooks inject context and enforce guardrails, and the outcome model

**Guardrail enforcement points:**
- PreToolUse `Deny` → `InspectionAction::Deny` (tool call becomes an error result fed back
  to the model); `Ask` → `RequireApproval` (`inspector.rs:62-88`).
- PermissionRequest `Allow`/`Deny` short-circuits the approval loop; `Deny` writes a
  `DECLINED_RESPONSE + Hook feedback` tool result (`tool_execution.rs:80-131`).
- UserPromptSubmit `Deny` short-circuits the whole reply — the user prompt is stored and an
  "Prompt blocked by hook" notice is returned, no model call (`agent.rs:1298-1319`).
- Stop `Deny` → `StopHookVerdict::Blocked`; the reason is pushed as a hidden user message
  `Stop hook feedback: <reason>` and the loop continues (`agent.rs:2180-2231`), capped at
  `STOP_HOOK_BLOCK_CAP = 5` consecutive blocks per session (`mod.rs:44,475-500`) after which
  it stops anyway (`CapReached`).

**Context injection points (only two):**
1. SessionStart + UserPromptSubmit `additionalContext`/raw-stdout → joined and stored once
   as a `<hook-context>` user message with visibility `(user=false, model=true)`
   (`agent.rs:1411-1422`).
2. PostToolUse/PostToolUseFailure `additionalContext` → a `<hook-context>` user message
   appended to `messages_to_add` before the next provider call (`agent.rs:1903-1911`).

**Critical absence:** the `HookInspector` (PreToolUse) reads **only** `aggregate.decision`
(`inspector.rs:62`) — it ignores `additional_context` and `system_messages`. Likewise
PermissionRequest reads only `aggregate.decision` (`tool_execution.rs:77`). So
`additionalContext`/`systemMessage` returned by PreToolUse or PermissionRequest hooks are
**silently dropped**. `grep` confirms `joined_context()`/`system_messages` are consumed only
at `agent.rs:1292,1325,1889,1899` (SessionStart/UserPromptSubmit/PostToolUse). All `fire()`
events (Notification, SubagentStart/Stop, Pre/PostCompact) discard their aggregate entirely,
so even a `systemMessage` from those is lost.

**Outcome model** (`outcome.rs`):
- `HookOutput`/`HookSpecificOutput` (`:14-37`) = the raw stdout JSON schema (camelCase):
  top-level `decision`/`reason`/`systemMessage`/`continue`/`stopReason`, and
  `hookSpecificOutput.{permissionDecision, permissionDecisionReason, additionalContext}`.
- `HookDecision` (`:40-55`) = `Allow | Ask | Deny`, with a `rank()` (0/1/2) for merge order.
- `HookOutcome` (`:58-66`) = one hook's result: `decision`, `additional_context`,
  `system_message`, `error` (non-blocking failure).
- `HookAggregate` (`:69-97`) = merged: most-restrictive decision wins, contexts and messages
  concatenate in hook order, errors collected.
- `interpret_stdout` (`:174-244`) precedence: `hookSpecificOutput.permissionDecision` first,
  then top-level `decision` (`block`→Deny, `approve`/`allow`→Allow), then `continue:false`→
  Deny. Unknown decision strings become a recorded error, not a block.

### Configuration — format, scope and matchers

Format is YAML under a `hooks:` key: `event → [ {matcher, hooks:[...]} ]`
(`config.rs:46-62`; user-facing docs are the [hooks reference](../../../agent-loop/hooks/hooks-reference.md)). Two scopes:
- **Global** — `~/.config/biorouter/config.yaml` `hooks:` section (env override
  `BIOROUTER_HOOKS`), loaded at `config.rs:111-120`.
- **Project** — `.biorouter/hooks.yaml` in the session working dir (`config.rs:16`), same
  schema under a top-level `hooks:` key or a bare event map (`parse_project_hooks`,
  `:131-143`). **Disabled by default**; opt in with `allow_project_hooks: true` or
  `BIOROUTER_ALLOW_PROJECT_HOOKS=1` (`mod.rs:74-79`). Project groups are *appended after*
  global groups, never replacing them (`resolved_groups`, `mod.rs:160-169`). Project config
  is cached and invalidated on file mtime change (`mod.rs:171-188`, `config.rs:145-168`).

**Matchers** (`matcher.rs:10-28`): `None`/`""`/`"*"` match everything; else exact string
equality, falling back to an **anchored** regex `^(?:pattern)$` (giving `a|b` alternation and
full regex in one rule). Invalid regex → logged + non-matching (fail-closed on the pattern).
The matcher key is the **tool name** for PreToolUse/PostToolUse/PermissionRequest, the
**source** (`startup`/`resume`) for SessionStart, and the **trigger** (`manual`/`auto`) for
Pre/PostCompact; UserPromptSubmit and Stop pass `None` (match-all).

A third, non-file source exists: **runtime session hooks**
(`set_session_hooks`/`clear_session_hooks`, `mod.rs:103-126`) scoped to one session id, used
by the `/goal` feature to install an LLM Stop-judge (`agents/goal.rs:246-265`). These always
match (no matcher) and merge after config hooks (`mod.rs:229-230`).

There is **no HTTP route and no GUI panel** for hooks — configuration is file/env only
(`grep` for "hook" in `biorouter-server/routes` finds only the unrelated goal Stop-hook).

### Comparison to Claude Code, and missing events

BioRouter's hook system is a deliberate, high-fidelity clone of Claude Code (comments say so,
`event.rs:3`, `outcome.rs:1`), and on **event coverage it is a superset**. Claude Code's
documented events (PreToolUse, PostToolUse, UserPromptSubmit, Notification, Stop,
SubagentStop, SessionStart, SessionEnd, PreCompact) all exist here, and BioRouter adds
**PostToolUseFailure, PermissionRequest, SubagentStart, PostCompact**. The stdin payload
field names, the exit-code contract (0/2/other), and the stdout `hookSpecificOutput` schema
all match Claude Code, so scripts port directly.

Where BioRouter is **weaker/divergent**:
- **PostToolUse cannot block.** In Claude Code a PostToolUse hook may return `decision:block`
  to feed a correction back to the model. Here PostToolUse/Failure are explicitly observe-only
  (`agent.rs:1845-1847`) — the decision is computed but ignored. Context injection is the only
  effect.
- **PreToolUse `additionalContext` is dropped** (see the "Critical absence" note above), whereas Claude Code injects it.
- **No `transcript_path`.** Claude Code hands hooks a path to the full transcript JSONL;
  BioRouter passes only a truncated inline `transcript_tail`, and only for Stop
  (`event.rs:114-117`, `mod.rs:460-485`). No `permission_mode` field either.
- **Only 2 config tiers.** Claude Code layers enterprise-managed / user / project / local
  settings; BioRouter has just global + (opt-in) project.
- **SessionEnd never fires in the GUI/daemon** — only CLI and scheduled runs
  (noted in the [hooks reference](../../../agent-loop/hooks/hooks-reference.md)). Claude Code fires it on every session close.
- **Notification is single-purpose** — only "permission prompt shown" with matcher key
  `permission_prompt` (`tool_execution.rs:136-147`); no idle/waiting/other notification kinds.
- **No output/stdin mutation, no dynamic tool injection** (neither product mutates tool_input,
  but Claude Code is adding richer control surfaces).

**Events genuinely missing vs a state-of-the-art coding agent:** a pre/post LLM-request hook
(cost/token/provider guardrails at the model-call level), a file-edit-specific event, and a
PreToolUse *input-rewrite* capability. None exist.

## Notable design choices (worth keeping)

- **Failure-open everywhere** (`mod.rs:9-11`): errors are recorded in `aggregate.errors` and
  `warn!`-logged (`mod.rs:250-252`) but never block. This is the right default for a
  user-scripted extension point.
- **Most-restrictive-wins merge** with an explicit `rank()` (`outcome.rs:48-54,101-129`) makes
  concurrent multi-hook semantics deterministic and easy to reason about.
- **Claude Code payload/decision compatibility** — real portability, not a lookalike.
- **Project hooks default-off** (`mod.rs:74-79`, and the [hooks reference](../../../agent-loop/hooks/hooks-reference.md)): opening a repo
  can't silently run its `.biorouter/hooks.yaml`. Correct threat model.
- **Concurrent stdin write on a spawned task** (`command_runner.rs:58-66`) prevents a hook
  that ignores stdin from deadlocking the writer — a subtle correctness win.
- **Stop-block cap** (`STOP_HOOK_BLOCK_CAP=5`, `mod.rs:42-44,475-500`) bounds runaway
  "keep-working" loops; the `stop_hook_active` flag lets well-behaved judges exit early.
- **Runtime session hooks** (`set_session_hooks`) elegantly reuse the same machinery for the
  `/goal` LLM guardrail without persisting anything.
- **mtime-keyed project-config cache** avoids re-reading `.biorouter/hooks.yaml` every tool
  call while still hot-reloading on edit.
- **Forward-compatible config parsing** — unknown events/malformed groups are skipped with a
  warning (`config.rs:78-104`), so a newer config never bricks an older binary.

## Gaps and weaknesses

These thirteen items fed the improvement phase. They are what other documents in this
review cite as `hooks.md #N`; the numbering below is that scheme and is stable.

1. **PreToolUse & PermissionRequest `additionalContext`/`systemMessage` are silently dropped.**
   `HookInspector` and `tool_execution` read only `aggregate.decision`
   (`inspector.rs:62`, `tool_execution.rs:77`). A hook author who returns
   `additionalContext` on a PreToolUse hook gets no error and no effect — a confusing,
   undocumented dead end. High-value, low-risk fix.
2. **PostToolUse cannot block.** Being observe-only diverges from Claude Code and removes the
   most useful post-hoc guardrail (e.g. "reject a write that fails lint"). The decision is
   already computed at `outcome.rs` — it's thrown away by design at `agent.rs:1845-1847`.
3. **`fire()` events discard everything.** Notification, SubagentStart/Stop, Pre/PostCompact
   spawn a detached task and drop the `HookAggregate` (`mod.rs:258-271`), so even a
   `systemMessage` is lost and there is no way to know a compaction/subagent hook even ran.
   Also fire-and-forget means these can outlive the turn and race shutdown.
4. **`supports_blocking()` is dead code** (`event.rs:68-77`, zero callers). The abstraction it
   implies (a single source of truth for which events block) does not exist; blocking is
   re-encoded ad hoc in three different files. This invites drift.
5. **No output-size limits.** Raw stdout (UserPromptSubmit/SessionStart) and `additionalContext`
   are injected verbatim with no truncation (`outcome.rs:180-186`, `agent.rs:1413-1422`). A
   hook that emits megabytes silently bloats context or blows the window — and it is a
   prompt-injection surface (a project hook's stdout lands in the model's context as a hidden
   user message).
6. **SessionEnd never fires in the GUI/daemon** (see the [hooks reference](../../../agent-loop/hooks/hooks-reference.md)) — the most common
   deployment. Cleanup/audit hooks are effectively unavailable to desktop users.
7. **No mutation / rewrite capability.** Hooks can only allow/deny/ask/inject; they cannot
   rewrite `tool_input` (e.g. sandbox a path), redact a payload, or transform tool output.
   This is the single biggest capability gap vs. what power users expect.
8. **Matcher only sees the tool name.** No matching on `tool_input` content (e.g. only guard
   `rm -rf`, or only writes under `/etc`). Every shell command must run the full guard script.
   The regex is also recompiled on every call (`matcher.rs:21`) — minor, but avoidable.
9. **No config UI / API / validation surface.** File-and-env only; a typo in a matcher regex
   or a wrong `permissionDecision` string only shows up as a `warn!` in logs
   (`outcome.rs:205-208`), never to the user. No `biorouter hooks list/test/lint` command.
10. **Only 3 env vars and no `transcript_path`.** Hooks that want conversation context must
    parse the (Stop-only, truncated) inline tail; there's no file handle to the transcript,
    no `permission_mode`, no tool-response path for PostToolUse beyond the JSON on stdin.
11. **Prompt-hook cost/latency is unbounded per event.** Every matching PreToolUse prompt hook
    is a synchronous `complete_fast` LLM call in the hot path (`prompt_runner.rs:57`), added to
    the tool-dispatch latency; there's no caching of identical (rule,payload) verdicts and no
    global budget. A chatty PreToolUse prompt hook could double every tool round-trip's latency.
12. **Two config tiers only, no managed/enterprise layer** — no way for an org to enforce a
    non-overridable security hook, and project hooks are all-or-nothing (one global opt-in, no
    per-project trust or per-command allowlist).
13. **Time-of-check-to-time-of-use (TOCTOU) in project-config caching.**
    `project_hooks_mtime` then `read_project_hooks` are
    separate stat+read (`mod.rs:171-188`); a coarse filesystem clock can serve a stale config
    (the tests themselves have to force `+2s`, `mod.rs:866-868`).

## Related documentation

- [Hooks reference](../../../agent-loop/hooks/hooks-reference.md) — the current, living user-facing guide to configuring hooks.
- [Guardrails, security and the permission system](guardrails-and-permissions.md) — the inspector chain the `HookInspector` is the fourth member of; the two reviews overlap on `PreToolUse`.
- [Safety and guardrails compared with other agents](../competitive-comparison/safety-and-guardrails.md) — how this hook model measures against Claude Code and seven others.
- [Verify-and-checkpoint stop hook](../../../agent-loop/hooks/verify-and-checkpoint-stop-hook.md) — a worked example of the `Stop` event described here.
- [Wave 2 hooks and permissions report](../../agent-loop-campaign/wave-reports/wave-2-hooks-and-permissions.md) — what was actually built in response to these gaps.
