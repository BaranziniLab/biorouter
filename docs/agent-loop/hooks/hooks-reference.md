# Hooks reference

> **What this is.** The full reference for Biorouter's hook system: the lifecycle
> events you can hook, how matchers select them, the two hook types (`command` and
> `prompt`), the stdin/exit-code/JSON contract, and the blocking and rewriting
> semantics.
> **Status:** Current.
> **Audience:** end users configuring hooks, and developers working on the agent loop.

Hooks let you run your own shell commands — or an LLM judge — at specific points in
Biorouter's agent lifecycle: before a tool runs, after a prompt is submitted, around
context compaction, when a session starts or ends, and more. Use them to enforce
guardrails, inject context, log activity, or trigger notifications.

Hooks work everywhere the agent runs: the desktop app, the CLI, scheduled runs, and
subagents.

## Configuration

Hooks live under a `hooks:` section in your global config
(`~/.config/biorouter/config.yaml`):

```yaml
hooks:
  PreToolUse:
    - matcher: "developer__shell"
      hooks:
        - type: command
          command: "$HOME/hooks/shell-guard.sh"
          timeout: 30
  PreCompact:
    - matcher: "auto"
      hooks:
        - type: command
          command: "cp ~/.config/biorouter/sessions/* ~/backups/ 2>/dev/null || true"
```

The `shell-guard.sh` referenced above is written out in full under
[Worked examples](#worked-examples).

Projects can ship their own hooks in `.biorouter/hooks.yaml` at the session working
directory, using the same schema under a top-level `hooks:` key. **Project hooks are
disabled by default** — a repository should not be able to run commands on your
machine just because you opened it. Opt in globally with:

```yaml
hooks:
  allow_project_hooks: true
```

(or `BIOROUTER_ALLOW_PROJECT_HOOKS=1`). Project hooks run *in addition to* global
hooks, never instead of them.

> **Note.** An administrator can deploy a managed policy that adds mandatory hooks
> which cannot be disabled, and that forces or forbids project hooks org-wide.
> Managed config wins over everything on this page — the precedence is
> `Default < User (global config) < Project (opt-in) < Managed (admin)`. If a hook
> you did not configure is running, or your `allow_project_hooks` setting appears to
> be ignored, check [Managed / enterprise policy](../../security/managed-policy.md).

## Events

| Event | Fires | Can block? | Matcher matches on |
|---|---|---|---|
| `PreToolUse` | before a tool call executes | yes — `deny` / `ask` (and may rewrite the input) | tool name |
| `PostToolUse` | after a tool call succeeds | yes — `block` (capped at 3) | tool name |
| `PostToolUseFailure` | after a tool call fails | yes — `block` (capped at 3) | tool name |
| `PermissionRequest` | when a tool would show an approval prompt | yes — `allow` / `deny` | tool name |
| `UserPromptSubmit` | when a user prompt is submitted | yes | — |
| `Stop` | when the agent is about to finish its turn | yes (capped at 5) | — |
| `SessionStart` | first prompt of a session in this process | no (context only) | `startup` \| `resume` |
| `SessionEnd` | CLI exit, headless completion, scheduled-run completion | no | exit reason |
| `Notification` | when a permission prompt is shown | no | `permission_prompt` |
| `PreCompact` / `PostCompact` | around context compaction | no | `manual` \| `auto` |
| `SubagentStart` / `SubagentStop` | around a subagent task | no | — |

Matchers: omit (or use `""` / `"*"`) to match everything; otherwise exact match,
`a|b` alternation, or a full regex (anchored). Tool names follow the
`extension__tool` convention, e.g. `developer__shell`, `developer__.*`.

### Matching on the tool input

A `matcher` only sees the tool *name*, so a shell guard would run on every shell
call. Add an optional `input_matcher` to narrow a group to specific tool
**arguments** — "only guard `rm -rf`", "only writes under `/etc`":

```yaml
hooks:
  PreToolUse:
    # A single regex, searched against the whole tool_input JSON.
    - matcher: "developer__shell"
      input_matcher: "rm\\s+-rf"
      hooks:
        - type: command
          command: "$HOME/hooks/confirm-destructive.sh"

    # Or a map of field -> regex; every entry must match.
    - matcher: "developer__text_editor"
      input_matcher:
        command: "^(write|str_replace)$"
        path: "^/etc/"
      hooks:
        - type: command
          command: "echo 'no edits under /etc' >&2; exit 2"
```

- Field paths are dotted and may index arrays: `path`, `params.path`, `argv.0`.
  Non-string values (numbers, booleans, nested objects) are matched against their
  JSON text.
- Unlike the tool-name matcher, `input_matcher` patterns are **searched, not
  anchored** — `rm\s+-rf` hits anywhere in the value. Anchor explicitly with `^…$`
  when you mean the whole value.
- A missing field never matches, and a group with an `input_matcher` never runs on
  an event that carries no tool input (`Stop`, `UserPromptSubmit`, …). An
  `input_matcher` can only *narrow* a group, never widen it.
- An invalid regex is logged once and treated as non-matching.

## Hook types

**`command`** — a shell command. It receives the event as JSON on stdin and runs in
the session working directory with `BIOROUTER_HOOK_EVENT`, `BIOROUTER_SESSION_ID`,
and `BIOROUTER_PROJECT_DIR` set. Default timeout 60s (`timeout` overrides, in
seconds).

**`prompt`** — an LLM judge. The rule you write is evaluated against the event
payload by your configured provider (its fast model when available, or an explicit
`provider:` + `model:` pair). The judge answers `{"ok": true|false, "reason": "..."}`;
`ok: false` blocks. Default timeout 30s.

```yaml
hooks:
  PreToolUse:
    - matcher: "developer__shell"
      hooks:
        - type: prompt
          prompt: "Block any command that deletes files outside the project directory."
```

## Command hook contract

Input on stdin, with snake_case keys:

```json
{
  "session_id": "…",
  "cwd": "/path/to/project",
  "hook_event_name": "PreToolUse",
  "tool_name": "developer__shell",
  "tool_input": {"command": "rm -rf build"}
}
```

Exit codes:

- **0** — success. stdout may contain a JSON decision (below). For
  `UserPromptSubmit` and `SessionStart`, plain (non-JSON) stdout is injected as
  context for the model.
- **2** — block. stderr is used as the reason: for `PreToolUse` it is fed back to
  the model, for `UserPromptSubmit` it is shown to the user, for `Stop` it becomes
  feedback the agent must address before finishing.
- anything else — non-blocking error; the event proceeds (failure-open).

Optional JSON on stdout, with camelCase keys:

```json
{
  "decision": "block",
  "reason": "tests have not been run",
  "systemMessage": "shown to the user as a yellow notice",
  "hookSpecificOutput": {
    "permissionDecision": "allow | deny | ask",
    "permissionDecisionReason": "…",
    "additionalContext": "injected for the model",
    "updatedInput": {"command": "rm -rf ./build"}
  }
}
```

When several hooks match one event they run in parallel and the most restrictive
decision wins (`deny` > `ask` > `allow`).

### Rewriting a tool call (`updatedInput`)

A `PreToolUse` hook does not have to choose between allowing and denying: it can
return `hookSpecificOutput.updatedInput` — the tool's **complete** replacement
argument object — to sandbox a path, redact a payload, or normalize a command. The
rewritten call is what executes and what the transcript records, and the model is
told (as injected context) that its arguments were changed, so it never silently
works from a call it did not make.

```bash
#!/bin/bash
# PreToolUse, matcher developer__shell — pin any rm to the build dir
input=$(cat)
command=$(echo "$input" | jq -r '.tool_input.command // ""')
case "$command" in
  "rm -rf /"*)
    jq -nc '{hookSpecificOutput: {hookEventName: "PreToolUse",
                                  updatedInput: {command: "rm -rf ./build"}}}'
    ;;
esac
```

Rules worth knowing:

- `updatedInput` is honored **only on `PreToolUse`** (elsewhere the tool has already
  run, or the call is already recorded); on any other event it is ignored and
  logged.
- It must be a JSON **object** (the full argument map, not a patch) and is capped at
  256 KB. A malformed rewrite is a non-blocking error — failure-open, like every
  other hook mistake.
- The rewritten input is **re-validated**: every other inspector (the
  catastrophic-command denylist, the permission gate) re-runs on the new arguments,
  so a rewrite cannot smuggle a call past the safety gates. The hook inspector
  itself is not re-run — a rewrite cannot trigger another rewrite.
- A hook that both denies *and* rewrites is treated as a deny (the call never runs,
  so there is nothing to rewrite). Hooks run concurrently, so rewrites do not chain:
  if two hooks return `updatedInput` for the same call, the last one (in config
  order) wins and the conflict is logged.

### Blocking a tool *result* (`PostToolUse`)

`PostToolUse` / `PostToolUseFailure` hooks can `block` (exit 2, or
`{"decision":"block"}`) — e.g. reject a write that fails lint. The tool has already
run, so its side effects stand and its output is preserved; the hook's reason is
appended to the tool result, the result is marked as an error, and the agent keeps
working on the correction. Consecutive blocks are capped at **3** per session (the
model typically retries the tool, so an unconditional blocker would otherwise wedge
the turn); past the cap the result is delivered anyway with a notice.

## Compatibility with other agents' hook formats

The wire format is deliberately Claude Code-compatible, so most hook scripts written
for it run unchanged. Where other agents spell a field differently, the alternate
spelling is accepted as an alias.

| Surface | Biorouter spelling | Also accepted |
|---|---|---|
| stdin event payload | snake_case (`hook_event_name`, `tool_name`, `tool_input`) | Claude Code uses the same keys |
| stdout decision payload | camelCase (`decision`, `systemMessage`, `hookSpecificOutput`) | Claude Code uses the same keys |
| rewritten tool arguments | `hookSpecificOutput.updatedInput` | Codex `updated_input`, Gemini CLI `tool_input` |

## Runtime semantics

### Safety guarantees

- **Failure-open everywhere.** A crashing, timing-out, or misconfigured hook never
  blocks the agent; only explicit decisions do.
- **Rewrites are re-validated.** See
  [Rewriting a tool call](#rewriting-a-tool-call-updatedinput) — a rewritten call
  still passes through the denylist and the permission gate.

### Decision semantics

- **PreToolUse `deny`** turns the tool call into an error result containing your
  reason, so the model can adapt. **`ask`** routes the call through the normal
  approval dialog with your reason attached. **`updatedInput`** rewrites the call
  instead of refusing it (see above).
- **PermissionRequest `allow`** auto-approves a call that would otherwise prompt the
  user — useful for trusted commands in trusted projects. It covers approvals the
  *permission mode* raised, which is nearly all of them. It does **not** cover the
  small fixed set of approvals BioRouter raises whatever your mode — a command
  policy `ask`, an Auto-mode write to a credential store, a global
  (machine-wide) memory read or write, a managed-policy `ask`. Those exist
  precisely because no automated grant should answer them, and a hook is an
  automated grant: an `allow` on one is logged and dropped, and the card is shown
  as if no hook had run. See
  [what still asks, whatever your mode](../../security/permission-modes.md#what-still-asks-whatever-your-mode).
  `deny` is unrestricted — a hook can always refuse anything.
- **`additionalContext` and `systemMessage` work on every tool-path event**,
  `PreToolUse` and `PermissionRequest` included. The context reaches the model
  wrapped in the `<hook-context untrusted="true">` frame used for all injected hook
  output, and the `systemMessage` surfaces as a yellow inline notice.

### Limits on repeated blocking

- **Stop blocks are capped** at 5 consecutive blocks per session; the payload field
  `stop_hook_active` is `true` on re-checks so well-behaved hooks can exit early.
- **PostToolUse blocks are capped** at 3 consecutive blocks per session (see
  [Blocking a tool result](#blocking-a-tool-result-posttooluse)).

### Built-in checks that run before your Stop hooks

When the agent tries to finish a turn, Biorouter runs its own checks first, in
this order, and only then consults your `Stop` hooks:

1. the done gate, when you configured one (`BIOROUTER_DONE_GATE`);
2. the self-critique pass, when you enabled it;
3. the Todo checklist check: a turn that created or changed the checklist may
   not end while items are unfinished, unless the final message names each open
   item. See [What Biorouter enforces](../../extensions/built-in/todo.md#what-biorouter-enforces).

A built-in check that blocks ends that stop attempt, so your hooks run on the
next one. The checklist check runs ahead of your hooks because it is
deterministic and cheap, while a command hook may run a whole test suite; there
is no point paying for a hook on a stop that is already refused. It has its own
budget, 5 blocks per turn, which does not reset when the agent runs tools and
does not count against the Stop-hook cap above. Its feedback reaches the model
the same way a Stop-hook block does: as hidden feedback, with a notice for you.
It stands aside while a `/goal` is active, because the goal's own judge decides
then.

### Scheduling of observe-only events

**Observe-only events run detached, but are not discarded.** `Notification`,
`SubagentStart` / `SubagentStop` and `PreCompact` / `PostCompact` cannot block, so
they are dispatched in the background rather than on the agent's critical path.
Their `systemMessage` still reaches you — it is collected at the next turn boundary,
where any outstanding hook is also joined. A hook slower than the boundary's short
wait is never waited on; it simply surfaces one boundary later, and is joined at
session end.

### What you see in the interface

Hook activity (blocks, judge verdicts, `systemMessage`s) appears as yellow inline
notices in both the CLI and the desktop app chat.

### Known limitations

- **GUI sessions do not fire `SessionEnd`.** The desktop app has no reliable
  session-close signal, so `SessionEnd` fires only on CLI exit, headless completion,
  and scheduled-run completion.
- **`PreCompact` can fire without a matching `PostCompact`.** They are not a
  bracket. `PreCompact` is fired *speculatively* — before a summarization whose
  outcome is not yet known, which is what makes it "pre" and what gives a hook its
  chance to capture the transcript before it is replaced. `PostCompact` fires only
  when a compaction actually landed, so it is skipped when the summarizer errors,
  and when the write-back is declined because another writer changed the history
  while the summary was being computed (see
  [Conversation writeback freshness](../conversation-writeback-freshness.md)).
  Firing it anyway would be worse than the asymmetry: it would tell every consumer
  the transcript had been replaced when it had not, so a hook that re-indexes the
  history, invalidates a cache, or reports "compacted to N tokens" would act on a
  history that never changed. Read `PreCompact` as "a compaction is about to be
  **attempted**", and do not pair acquire/release work across the two events
  without a timeout of your own.

## Worked examples

Block destructive shell commands — this is the `shell-guard.sh` referenced from
[Configuration](#configuration), and it guards on the command text inside the
script. To do the same filtering in config instead, use an
[`input_matcher`](#matching-on-the-tool-input).

```bash
#!/bin/bash
# ~/hooks/shell-guard.sh — PreToolUse, matcher developer__shell
input=$(cat)
command=$(echo "$input" | jq -r '.tool_input.command // ""')
if echo "$command" | grep -qE 'rm -rf|mkfs|dd if='; then
  echo "Destructive command blocked by shell-guard" >&2
  exit 2
fi
```

Require a clean test run before the agent finishes:

```yaml
hooks:
  Stop:
    - hooks:
        - type: command
          command: "cargo test --quiet 2>/dev/null || { echo 'tests are failing; fix them before finishing' >&2; exit 2; }"
          timeout: 300
```

Inject lab context at session start:

```yaml
hooks:
  SessionStart:
    - hooks:
        - type: command
          command: "cat ~/.config/biorouter/lab-context.md 2>/dev/null"
```

For a ready-made, maintained version of the "don't finish until it builds and is
committed" pattern, see the
[verify-and-checkpoint Stop hook](verify-and-checkpoint-stop-hook.md).

## Related documentation

- [Verify-and-checkpoint Stop hook](verify-and-checkpoint-stop-hook.md) — a shipped
  Stop hook that applies this contract to build/test verification and git commits.
- [Managed / enterprise policy](../../security/managed-policy.md) — how an admin tier
  adds mandatory hooks that override everything on this page.
- [Permission modes](../../security/permission-modes.md) — the approval gate that
  `PermissionRequest` hooks and `updatedInput` rewrites are re-validated against.
- [Config file reference](../../configuration/config-file-reference.md) — the
  structure and location of `config.yaml`, the file the `hooks:` block lives in.
- [Subagents](../subagents.md) — the tasks that `SubagentStart` / `SubagentStop`
  wrap.
