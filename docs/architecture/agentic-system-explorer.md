# Biorouter agentic system explorer

> **What this is.** The written companion to the agentic-system explorer: a code-aligned account of how a request becomes model context, inspected tool work, durable state, recovery, and a verified answer.
> **Status:** Current — follows current Rust behavior and the agent-loop documentation.
> **Audience:** developers working on the agent runtime and harness; agents reading the runtime contracts without a browser.

Biorouter's agent runtime is a loop the *runtime* controls, not the provider. The provider supplies responses and tool requests; Biorouter decides what is admitted, what context is assembled, which tools are allowed to run, what is persisted, and whether a tool-free answer actually ends the turn. This document defines the implemented runtime and its harness contracts, in the same sixteen-part order as the explorer page.

> **The diagrams live in the HTML.** Seventeen rendered SVG architecture diagrams — the turn lifecycle, entry paths, request assembly, inspection pipeline, vault substitution, dispatch, hook lanes, recovery paths, safety escalation, and transport lanes — are in [`agentic-system-explorer.html`](agentic-system-explorer.html) and must be opened in a browser to be seen. This companion carries the reasoning and the specifications, not the pixels.

## Turn lifecycle

Biorouter admits the turn, prepares the provider request, runs approved tools, persists results, and applies completion checks. The provider supplies responses and tool requests; the runtime controls the loop.

> **Rendered in the HTML.** A full-width lifecycle diagram tracing `USER TURN → ENTRY → ASSEMBLE → PROVIDER → TOOLS?`, with the yes-branch running `INSPECT → EXECUTE → RESULT` back into context assembly and the no-branch running to `COMPLETION → FINAL EVENT`, plus the soft-interrupt, hard-cancel, and shadow-checkpoint lanes beneath it. Caption: tool results return through persistence; tool-free responses move to completion checks.

**Admission.** One active turn is allowed per session. Duplicate client turn IDs are idempotent; a different concurrent turn is rejected.

**Iteration.** Each provider call uses freshly assembled agent-visible context. Tool calls return through inspection and persistence before another call.

**Termination.** A plain answer is not always enough: structured output, retry checks, done gates, critique, goals, and Stop hooks may continue the loop.

Primary implementation: `crates/biorouter/src/agents/agent.rs`, `crates/biorouter-server/src/routes/reply.rs`

## Entry paths and session types

Interactive requests use the server's turn guard and SSE (Server-Sent Events) transport. Scheduled runs and subagents create typed sessions and call the same agent runtime without the interactive reply route.

> **Rendered in the HTML.** A diagram showing interactive clients passing through the reply route, turn guard, and user session before `AGENT.REPLY` (with a conflicting interactive turn peeling off to `409 CONFLICT`), while the scheduler and parent-agent lanes create scheduled or subagent typed sessions and join the same agent runtime. Caption: only interactive turns use the reply-route ownership guard.

**Special replies**

- Elicitation responses are delivered to the waiting request and persisted.
- Slash commands may answer immediately, rewrite history, or resolve into a normal user message.
- Persisted goals are restored before a new reply begins.

**Session types**

- `User` for interactive work.
- `Scheduled` and `SubAgent` for background or delegated runs.
- `Hidden` and `Terminal` for nonstandard surfaces.

Primary implementation: `crates/biorouter/src/session/`, `crates/biorouter-server/src/routes/reply.rs`, `crates/biorouter/src/scheduler.rs`

## Provider request assembly

System instructions, agent-visible messages, and tool schemas are prepared on separate paths. They join only when Biorouter builds the next provider request.

> **Rendered in the HTML.** A three-lane assembly diagram: system sources (base, model overlay, mode, local date, extensions, frontend, workflow, project hints, `AGENTS.md`) sanitized and ordered into the system prompt; agent context inputs (selected resources, a fresh model-oriented information message (MOIM), harness messages, conversation history) placed on an agent-visible clone and normalized; tool definitions sorted separately — the three converging inside a `TURN REQUEST` boundary bound for the provider. Caption: system, messages, and tools join only at the provider boundary.

**Selected skills.** Explicit skill references are loaded before the call. Full bodies are capped and injected once per session; later turns receive a short pointer. Failed loads are not cached.

**Selected knowledge.** Biorouter searches the exact selected knowledge base with the user's whole message and injects up to five primary results, or an explicit retrieval failure. Selected-resource material is wrapped in an agent-only `<explicit-resource-context>` message.

**MOIM.** The model-oriented information message is rebuilt on a conversation clone for every provider call. A stale standalone MOIM is removed, then the fresh one is placed after the last assistant message and any trailing tool results so tool pairs stay valid. It is never persisted.

**Selected extensions.** Aliases are canonicalized. A known built-in extension selected by the user can be enabled and persisted, then its mandatory guidance and tools join the stable prompt assembly.

**Live state awareness.** Platform extensions contribute current state through MOIM. Durable todos, goals, run state, enabled extensions, and related harness data live in the session's `extension_data`.

**Project instructions.** `.biorouterhints`, `AGENTS.md`, and supported imports join workflow/application instructions. Unicode control-like tags are sanitized before injection.

> **Workspace context is bounded.** The default workspace map respects `.gitignore` and `.biorouterignore`, scans at most 20,000 entries, reports at most 200 entries to depth three, targets about 2,000 tokens, and caches for 30 seconds. MOIM as a whole targets about 8,000 estimated tokens. Under aggregate prompt pressure, extension guidance is retained before project hints.

Primary implementation: `crates/biorouter/src/agents/prompt_manager.rs`, `reply_parts.rs`, `resource_refs.rs`, `moim.rs`, `workspace_summary.rs`, `crates/biorouter/src/context_budget.rs`

## Visibility and role are independent contracts

A message can be visible to the user, the agent, both, or neither surface. The provider receives a normalized projection, while the UI retains the durable human-readable history.

> **Rendered in the HTML.** A diagram splitting the durable UUIDv7 message record (`role · content · created · user_visible · agent_visible`) into a user-visible view for the desktop and session APIs and an agent-visible view for the provider normalizer, with compaction feeding only the latter. The normalizer box lists its operations: merge adjacent assistant text, drop empty or orphaned tool items, merge same effective roles, remove assistant edges, and seed "Hello" only if empty. Caption: normalization operates on the provider projection, not the user-facing record.

**Tool pairing.** Assistant tool requests and user-role tool responses are persisted as a pair. "Effective tool role" normalization preserves provider-specific validity.

**Compacted history.** Older originals stay user-visible but become agent-hidden. A model-visible summary and continuation carry the working state forward.

Primary implementation: `crates/biorouter/src/conversation/`, `crates/biorouter/src/conversation/normalize.rs`

## Provider call and response handling

Reasoning effort adjusts turn and tool budgets. The runtime classifies streamed text, tool requests, usage, finish reasons, and provider failures before deciding what happens next.

> **Rendered in the HTML.** A fan-out diagram from the provider stream into four classified outcomes — completion candidate, bounded continuation, frontend request via the client bridge, and backend request into the tool harness — with tool results integrated and persisted before another provider turn and failures routed to recovery or abort. Caption: tool results re-enter the provider loop; tool-free output proceeds to completion checks.

**Truncation.** A `length` finish injects a hidden continuation instruction. Automatic continuation is capped at twelve attempts.

**Frontend calls.** Browser/UI tools are streamed to the client; results return on a request-scoped channel and rejoin the same tool-result path.

**Tool shims.** Providers without native tool calling receive JSON tool instructions in the system prompt. Tool messages are translated to text and parsed back.

Primary implementation: `crates/biorouter/src/agents/agent.rs`, `prompt_manager.rs`, provider adapters under `crates/biorouter/src/providers/`

## Backend calls use escalation-only policy merging

Inspectors do not vote. Their decisions merge toward greater restriction: deny outranks ask, and ask outranks allow. Rewritten arguments repeat the earlier checks before dispatch.

> **Rendered in the HTML.** The inspection pipeline diagram: a backend tool call passing managed policy, security, permission, repetition, and hook inspection, with hook argument rewrites looping back into the earlier inspectors; approved calls execute, denied calls become tool errors. Caption: no permissive layer can override a stricter decision.

| Mode | Default behavior | Still enforced |
|---|---|---|
| `Chat` | No backend dispatch. | Conversation and model response only. |
| `Auto` | Baseline allow. | Managed policy, security, command policy, and hooks can escalate. |
| `Approve` | Prompt unless a trusted remembered or scoped rule applies. | All higher-order restrictions. |
| `SmartApprove` | Read-only / low-risk tools can auto-approve; higher risk asks. | Risk threshold, unknown-risk handling, security, policy, and hooks. |

**Managed policy trust.** Administrator policy is read from the platform-managed Biorouter location: `/Library/Application Support/Biorouter/managed-policy.yaml` on macOS, `/etc/biorouter/managed-policy.yaml` on Linux, or `%ProgramData%\Biorouter\managed-policy.yaml` on Windows. Unix ownership and write permissions are verified before use.

**Approval identity.** The approval request is registered before the UI event and keyed by request ID. The card carries a risk grade plus a tool-specific preview, command, or diff. One-shot delivery prevents a late answer from approving another call.

> **Secrets have a second boundary.** The extension dispatcher's `SecretGuard` rejects arguments that reach known credential files such as `.env`, private keys, and cloud credential stores — read the way the shell will read them, so `~`, variables, globs and `cd` are resolved first, and refused whether or not the file exists — regardless of extension. Credential values that still appear in a tool's output are withheld before any model sees them. See [secret guard](../security/secret-guard.md).

Primary implementation: `crates/biorouter/src/tool_inspection.rs`, `crates/biorouter/src/permission/`, `crates/biorouter/src/security/`, `crates/biorouter/src/hooks/inspector.rs`, `crates/biorouter/src/tool_monitor.rs`

## Vault substitution occurs at leaf MCP dispatch

The model sees symbolic placeholders. Plaintext is attached only after inspection and only for an allow-listed Agent Drafter app's leaf MCP (Model Context Protocol) arguments.

> **Rendered in the HTML.** A provisioning-and-substitution diagram: an application manifest allow-list, a per-application AES key in the operating-system keyring, and encrypted workspace files combine into in-memory `VaultRefs`; during a turn the model emits a symbolic placeholder that inspectors also see, and only `apply_vault` at leaf MCP dispatch substitutes plaintext. Caption: provisioning creates VaultRefs; only leaf MCP dispatch consumes them.

**Key material.** The key identifier is `brsdk_vault_key_{app_id}`. A missing key is generated and persisted; an unreadable or invalid key is never silently replaced.

**Encrypted files.** Each allow-listed name maps to `.vault/<sanitized-name>.enc`, containing a random 12-byte nonce followed by AES-256-GCM ciphertext and tag. Agent file tools are jailed away from `.vault/`.

**Residual boundary.** The leaf tool receives plaintext by design. A tool that echoes its arguments can expose a secret in its result, so result handling and tool trust remain part of the security model.

> **Vault creation is capability-gated.** It applies to BRSDK Agent Drafter apps only when encryption support is enabled and the app manifest declares at least one name in `agent.capabilities.vault.encrypted`. Other agents do not receive a `VaultRefs` map.

Primary implementation: `crates/biorouter-server/src/routes/apps.rs`, `crates/biorouter-mcp/src/agent_drafter/vault.rs`, `crates/biorouter/src/agents/vault_refs.rs`

## Tool dispatch, concurrency, and result controls

The dispatcher selects platform branches before leaf MCP calls, limits global concurrency, serializes conflicting writes, and converts every result into a bounded model-visible form.

> **Rendered in the HTML.** A dispatch diagram splitting approved requests into special agent branches, frontend calls, and leaf MCP tools, with execution controlled by a semaphore and path locks before results are offloaded, scanned, typed, hooked, persisted, and returned to the loop. Caption: all dispatch branches converge on one result path before another provider turn.

| Dispatch branch | Runtime treatment |
|---|---|
| Schedule / ingest / session blob | Handled by agent or platform logic before extension dispatch. |
| `final_output` | Validated and stored inside the agent; never leaves for MCP execution. |
| Subagent / subagent status | Uses child-session and handle controllers; vault placeholders remain symbolic. |
| Frontend tools | Round-trip through the client result channel; vault placeholders remain symbolic. |
| Leaf MCP tools | Pass availability and secret-file checks, receive session metadata/progress routing, and are the only branch eligible for vault substitution. |

**Large results.** Oversized text is written completely under `.biorouter/tool-output/`. The model receives metadata, a head/tail preview, and the path; non-text parts remain intact.

**Guardrails.** Successful text can be annotated or masked for prompt-injection markers and PII/PHI patterns. Errors and non-text payloads pass through unchanged.

**Typed failures.** Errors become `not_found`, `permission_denied`, `timeout`, `invalid_args`, `transient`, `tool_failure`, or `internal`. Only timeout and transient are retryable.

Primary implementation: `crates/biorouter/src/agents/agent.rs`, `tool_dispatch_limits.rs`, `large_response_handler.rs`, `tool_errors.rs`, `crates/biorouter/src/guardrails/tool_output.rs`, extension manager modules

## Hooks attach to defined lifecycle events

User, project, and managed hooks attach to specific session, tool, compaction, notification, and child-session events. Returned context is marked untrusted before it becomes agent-visible.

> **Rendered in the HTML.** A swimlane diagram of independent hook lanes — session hooks from `SessionStart` through `UserPromptSubmit`, the agent loop, `Stop`, and `SessionEnd`; tool hooks surrounding inspection, optional permission, and execution; compaction hooks surrounding compaction; subagent hooks surrounding a child session; and `Notification` observing runtime notifications independently. Caption: each hook runs at a named boundary; the lanes are independent.

| Event | Contract |
|---|---|
| `SessionStart` / `SessionEnd` | Observe session lifecycle; start may add context for startup or resume. |
| `UserPromptSubmit` | May block the turn or add untrusted agent-visible context. |
| `PreToolUse` | May allow, ask, deny, or replace the full argument object. |
| `PermissionRequest` | May allow or deny before a human dialog is shown. |
| `PostToolUse` / `PostToolUseFailure` | Preserves the completed side effect/output but can mark the result as an error and inject bounded feedback. |
| `PreCompact` / `PostCompact` | Observe automatic or manual history compaction. |
| `Stop` | May block completion up to five times and return corrective context. |
| `Notification` | Observe runtime notifications such as a permission prompt. |
| `SubagentStart` / `SubagentStop` | Observe child lifecycle; subagent Stop is not a completion gate. |

**Configuration tiers.** User hooks live in `~/.config/biorouter/config.yaml`. Project hooks live in `.biorouter/hooks.yaml` and require opt-in. Managed hooks come from trusted administrator policy and can govern project-hook availability.

**Execution semantics.** Command hooks receive JSON on stdin and environment context; prompt hooks use a fast or explicitly configured model. Observe-only hooks may detach, but the loop settles their outcomes at a boundary.

Primary guide and implementation: [`agent-loop/hooks/hooks-reference.md`](../agent-loop/hooks/hooks-reference.md), `crates/biorouter/src/hooks/`, `crates/biorouter/src/hooks/inspector.rs`

## Background work and child-session handles

Shell jobs, subagents, and scheduled workflows continue beyond one synchronous tool result. Each has a distinct handle, durability rule, status path, and cancellation contract.

> **Rendered in the HTML.** A diagram of the main agent launching background shell jobs, child subagent sessions, and durable schedules, with an active-work aggregator over all three and scheduler runs deferring during interactive work or provider rate limits. Caption: every extended activity has a handle, a status path, and a cancellation path.

**Background shell.** `shell(background:true)` returns immediately. `shell_output` reads new output, `shell_wait` waits up to 600 seconds without killing, and `shell_kill` escalates from TERM to KILL.

**Subagents.** Children inherit allowed extensions and may override model settings. They cannot spawn children or manage schedules/extensions. Concurrency defaults to eight with a bounded in-flight queue.

**Schedules.** A claimed cron run never overlaps itself. Paused, exhausted, active, rate-limited, or interactively contended runs are skipped without consuming a capped run.

Primary implementation: developer shell MCP, `crates/biorouter/src/agents/subagent_*.rs`, `crates/biorouter/src/scheduler.rs`, server `active_work` routes

## Session history, context, and large payloads

Biorouter retains a complete user-facing session while bounding the provider's working set. Compaction changes the agent-visible projection; large-result externalization is a separate storage path.

> **Rendered in the HTML.** A two-path diagram: durable session rows producing a user-visible history and an agent-visible projection measured against the context threshold, with compaction creating a summary projection while retaining originals; independently, large tool results move to a session blob or workspace file and leave a compact reference in the conversation. Caption: compaction and large-result storage solve different limits and do not feed each other.

**Overflow ladder**

1. Standard compact.
2. Keep two recent user turns.
3. Summarize all.
4. Drop the oldest agent-visible half, then summarize.
5. Return a context-limit error.

**Persistence.** SQLite stores session metadata, messages, current and lifetime usage, extension state, workflow/schedule links, lineage, and large message blobs. Visible user text is mirrored into FTS5 for recall.

Primary implementation: `crates/biorouter/src/context_mgmt/`, `crates/biorouter/src/session/`, `crates/biorouter/src/agents/large_response_handler.rs`

## Checkpoint restore and session branching

Shadow checkpoints capture filesystem state around tool work. Session branching creates a new conversation lineage. They are separate recovery paths, and neither writes to the project's own Git database.

> **Rendered in the HTML.** A diagram anchoring PreStep, tool iteration, and PostStep snapshots in Biorouter's shadow object database, with restore first recording a PreRestore baseline and then restoring files, conversation, or both; alongside it, session branching copies history through a selected message UID into a new session while leaving the original unchanged. Caption: checkpoint restore rewinds selected state; session branching preserves an alternative.

**Checkpoint semantics.** PreStep snapshots allow the whole turn to be undone. PostStep snapshots occur only after a tool iteration; an unchanged tree hash removes read-only duplicates. Manual and PreRestore checkpoint kinds also exist.

**Branch semantics.** A branch copies the source session through a stable message ID, assigns a sibling-aware name, and records both the parent session and divergence message without altering the original.

Primary implementation: `crates/biorouter/src/checkpoint/`, `crates/biorouter-server/src/routes/session.rs`

## Loop safety controllers and escalation

Exact repetition, semantic cycles, repeated failures, mistake streaks, periodic stall review, hard caps, and reply budgets contribute independent evidence to one escalation path.

> **Rendered in the HTML.** A diagram feeding model and tool actions into a set of independent safety controllers whose evidence is evaluated against thresholds to produce advisory feedback, wrap-up grace, or terminal controls, with every outcome able to emit a structured `LoopSafetyEvent` and hard cancel / soft interrupt shown as separate controls. Caption: controller evidence selects an escalation level; structured events report every level.

| Controller | Default signal | Default response |
|---|---|---|
| Exact tool repetition | Same call 3 times; hard threshold 5. | Warn, then stop. |
| Near duplicate / A-B cycle | Similarity ≥ 0.9 or four-step alternation. | Warn; semantic hard stop is off by default. |
| Repeated same failure | Nudge at 3, escalate at 5, deny after 6. | Replan, then prevent another identical failing call; retryable timeout/transient are exempt by default. |
| Periodic stall judge | First review at 30 actions, then every 10. | Nudge, then short wrap-up grace on repeated stall evidence. |
| Hard caps | 100 turns and 200 tool calls by default. | Terminate with a clear limit result. |
| Reply budget | Configured wall time, tokens, or cost. | Warn, request wrap-up, then stop after grace. |

Primary implementation: `crates/biorouter/src/tool_monitor.rs`, `crates/biorouter/src/agents/mistakes.rs`, `stall.rs`, `budget.rs`, `goal.rs`, `turn_guard.rs`, and `turn_abort.rs`

## Completion checks and retry paths

A tool-free response is only a completion candidate. Biorouter applies the checks enabled by the workflow or session, and failed checks return the turn to work.

> **Rendered in the HTML.** A decision diagram in which a completion candidate checks whether a response schema is configured, then whether workflow checks and the done gate are enabled; active goals use the goal judge and skip self-critique, sessions without a goal may use optional self-critique, and Stop hooks run before final emission — any failed check returning to the work loop, while workflow retry alone may restore the initial conversation. Caption: only configured gates run; failures return to work before final emission.

**Structured response.** When a workflow defines a response schema, the `final_output` tool validates and stores one JSON value. If it is missing, the loop asks the model to continue.

**Workflow retry.** Configured checks run after apparent success. A failed attempt can run `on_failure`, clear final output, and restore the initial conversation before a bounded retry.

**Interactive verification.** Optional done checks iterate on current work. Tree-sitter diagnostics can reflect on edit errors, and fast self-critique can challenge correctness, contradictions, or fabrication.

Primary implementation: `crates/biorouter/src/agents/final_output_tool.rs`, `retry.rs`, `done_gate.rs`, `post_edit_diagnostics.rs`, `self_critique.rs`, goals and hooks modules

## Server turn ownership and SSE transport

The HTTP layer admits one interactive turn per session and carries agent events to the client. Frontend tools, approvals, cancellation, and interrupts return through explicit request or control paths.

> **Rendered in the HTML.** A four-lane transport diagram (client, server, agent, SSE): the client posts reply to the server reply route, which acquires the session turn guard and invokes agent reply; agent events travel out over SSE; frontend tool and approval requests go to the client over SSE and their results return through request-scoped endpoints and channels; cancel and interrupt endpoints target the active loop. Caption: the server owns admission and transport; the agent owns loop behavior.

**Hard cancel.** Cancellation is idempotent and session-addressable. The token is registered before turn admission, unblocks pending approvals, and is observed at loop checkpoints.

**Soft interrupt.** An interrupt is accepted only for an active turn. The message enters a FIFO and is persisted at the next safe boundary; if it arrives during final streaming, it keeps the loop alive for another iteration.

Primary implementation: `crates/biorouter-server/src/routes/reply.rs`, action-required routes, session routes, and desktop event consumers

## Default limits and feature gates

These values are implementation defaults, not performance targets. Environment flags and workflow or session settings can narrow or extend them.

> **Rendered in the HTML.** A "default runtime envelope" diagram ringing the agent harness with its six controls. Caption: defaults bound ordinary turns; optional features are explicit.

| Control | Default |
|---|---|
| Context threshold | 80% · keep 4 user turns |
| Hard loop caps | 100 turns · 200 tools |
| Tool concurrency | 8 global |
| Continuation | 12 attempts |
| Stall review | 30, then every 10 |
| Off by default | checkpoints · done gate · diagnostics · critique |

Workflow and session configuration may narrow or extend these defaults.

**Always-on foundations**

- Conversation normalization.
- Managed policy and catastrophic security denylist.
- Per-session turn ownership.
- Usage and history persistence.
- Tool error typing and result annotation defaults.

**Configuration-sensitive**

- Permission mode and remembered/scoped grants.
- Workflow retries and response schema.
- Wall-time, token, and dollar budgets.
- Subagent background handles.
- Tool-output mask versus annotate.

**Feature-gated**

- Shadow checkpoints.
- Shared process MCP pool.
- Post-edit diagnostics.
- Interactive done gate.
- Fast self-critique.

> **Scope.** This reference follows current Rust behavior and the agent-loop documentation. Delivery history is outside this document.

## Related documentation

- [Agentic system explorer (rendered page)](agentic-system-explorer.html) — the rendered explorer this file accompanies; open it for the seventeen architecture diagrams.
- [Theme system explorer (rendered page)](../design/theming/theme-system-explorer.html) — the companion theme-system explorer, including the Parchment theme this page is rendered in.
- [Design system gallery (rendered page)](../design/design-system-gallery.html) — the rendered design-system component and token gallery.
- [Context engineering](../agent-loop/context-engineering.md) — how context is selected and budgeted, in depth.
- [Hooks reference](../agent-loop/hooks/hooks-reference.md) — the full hook event contracts summarized above.
- [Subagents](../agent-loop/subagents.md) — child-session limits, lineage, and delegation.
- [Permission modes](../security/permission-modes.md) — the `Chat` / `Auto` / `Approve` / `SmartApprove` modes in detail.
- [Managed policy](../security/managed-policy.md) — administrator policy files, trust checks, and precedence.
- [System overview](system-overview.md) — how the agent runtime sits inside the wider system.
- [Documentation index](../README.md) — the entry point for the rest of the tree.
