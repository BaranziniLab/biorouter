# Self-verification, output validation and done-ness — architecture review

> **What this is.** One of ten subsystem reviews from the 2026-07 BioRouter agentic-loop review. It documents the three code-level verification checkpoints — `final_output` JSON-Schema validation, workflow success-checks with retry, and the `/goal` judge — and argues that nothing enforces done-ness in interactive chat. It records eleven gaps.
> **Status:** Historical record — a snapshot of the code as read on **2026-07-12**, before the agent-loop fix campaign, whose findings were then implemented. Gap #3 (no automatic post-edit diagnostics; the LSP/analyze capability listed but unwired) was fixed by BR-47 (`agents/post_edit_diagnostics.rs`), the done-ness gap by BR-49 (`agents/done_gate.rs`), the missing self-critique pass by BR-50 (`agents/self_critique.rs`), the dormant `structured_output` loop by BR-48, and gap #7 (no no-progress detector outside `/goal`) by BR-31 and BR-32.
> **Audience:** developers working on verification, structured output, or workflow retry.

This is the only one of the ten subsystem reviews carrying a date: its line numbers match the files as read on 2026-07-12. It pins no commit, so the citations are anchored to that date rather than to a revision. Identifier key: `BR-NN` are proposal ids from the [master improvement-proposal list](../improvement-proposals.md); the numbered items under "Gaps and weaknesses" are what sibling reviews cite as `verification.md gap #N` (the file's former name).

## Overview

BioRouter's "self-verification" story is thin and mostly **prompt-level**. There
are exactly three code-level checkpoints, and only one of them is a hard,
enforced gate:

1. **Structured final-output schema validation** (`final_output_tool.rs`) — the
   one place where the agent's answer is *machine-validated* against a JSON
   Schema and rejected with a corrective error until it conforms. Only active
   when a workflow / recipe / subagent declares a `Response { json_schema }`.
2. **Workflow success-checks + retry** (`retry.rs`) — the only "tests-must-pass"
   mechanism. A workflow can declare `checks: [Shell { command }]`; if any
   command exits non-zero the whole conversation is reset to the initial
   messages and re-run up to `max_retries`. Workflow-only; absent in interactive
   chat.
3. **Artifact render-error auto-repair** (`BaseChat.tsx` +
   `ArtifactViewer.tsx`) — a UI-side loop that feeds a rendering runtime error
   from a generated figure/app back into the live conversation as a hidden user
   message so the agent fixes it.

Everything else — "run the build/tests after you edit", "validate your output",
"don't fabricate", "hedge uncertainty" — is **English in a system prompt**, not
an enforced checkpoint.

Text data-flow (interactive chat, the common case):

```text
model turn ──► tool calls ──► tool results (is_error flag + stderr text)
   ▲                                   │
   │  (natural feedback: model reads   ▼
   │   compiler/MCP errors as text)   RepetitionInspector (tool_monitor.rs)
   │                                   │  detects identical repeated calls
   └────────────────────────────────  ▼
turn ends with no tool call ──► finish_reason?
        length  ──► auto-continue (bounded)               [agent.rs:2053]
        stop/None ─► final_output tool present?
                        yes: not called ─► re-prompt "call final_output NOW"
                             called ─► validate ✓ ─► exit  [agent.rs:2072]
                        no: Stop hook / goal LLM-judge verdict [agent.rs:2140]
                             Proceed ─► done │ Block ─► inject feedback, loop
```

Workflow path adds the retry layer *around* the whole loop:

```text
loop finishes ──► execute_success_checks(shell commands)  [retry.rs:191]
   all pass ─► SuccessChecksPassed (done)
   any fail ─► on_failure cmd ─► reset messages to initial ─► retry
              (until max_retries) ─► MaxAttemptsReached message  [retry.rs:131]
```

## Review questions answered

### How the agent realizes an answer needs hardening

**Concrete, code-enforced mechanisms are limited to two, plus one UI loop:**

- **`final_output` tool schema validation.** When a `Response` carries a
  `json_schema`, `FinalOutputTool::validate_json_output` compiles the schema with
  the `jsonschema` crate and returns every violation as an `INVALID_PARAMS`
  error the model must correct
  (`crates/biorouter/src/agents/final_output_tool.rs:94-117`). The error text is
  explicitly corrective: *"Please correct your output to match the expected JSON
  schema and try again."* (`final_output_tool.rs:112`). This is the single
  strongest hardening mechanism, but it only exists when a schema is registered
  (workflows / recipes / structured subagents).

- **Tool-result validation is per-tool, not centralized.** The `final_output`
  tool validates; MCP tools return errors as `is_error: Some(true)` content
  (`crates/biorouter/src/agents/tool_execution.rs:111-116`), which the model
  reads as ordinary text. There is **no** global "did this answer pass?" gate for
  a normal chat turn.

- **Prompt-level nudges** (suggestion, not enforcement): the system prompt tells
  the model to "verify with tools", "run the project's build, tests, or lints
  when available and fix what you broke"
  (`crates/biorouter/src/prompts/system.md:61-64`), and for science to "hedge
  uncertainty; and flag when a claim should be backed by a primary source"
  (`system.md:82-83`). The workflow prompt says "You can also validate your
  output after you have generated it"
  (`crates/biorouter/src/prompts/desktop_workflow_instruction.md:11`). None of
  these are checked.

So the agent "realizes" it needs hardening only because (a) a schema rejected its
output, (b) a tool returned an error it can see, or (c) the prompt told it to —
there is no reflection/critique pass, no self-consistency check, no
LLM-as-judge on ordinary answers.

### How the system decides a task is done

Termination is decided when a model turn ends **with no tool call**
(`crates/biorouter/src/agents/agent.rs:2044`). What happens then, in priority
order:

1. `finish_reason == "length"` → auto-continue up to
   `MAX_TRUNCATION_CONTINUATIONS` (a truncated turn is not "done")
   (`agent.rs:2053-2071`).
2. If a `final_output` tool exists and hasn't been called → inject
   `FINAL_OUTPUT_CONTINUATION_MESSAGE` = *"You MUST call the `final_output` tool
   NOW…"* and loop (`agent.rs:2072-2077`,
   `final_output_tool.rs:9-10`). Done only once it's called and validated.
3. Otherwise → `handle_retry_logic` (workflow success-checks, below)
   (`agent.rs:2087`).
4. On `exit_chat`, a **Stop hook** runs. Its verdict can *block* completion:
   `StopHookVerdict::Proceed` ends the turn; a block injects feedback and keeps
   the agent working (`agent.rs:2120-2150`). This is the interactive
   "keep-going-until-condition-met" hook, used by `/goal`
   (`crates/biorouter/src/agents/goal.rs:1-31`): an LLM judge evaluates a
   user-supplied condition every time the agent tries to stop, bounded by
   `GOAL_MAX_ITERATIONS` and stall detection.

**Enforced "tests-must-pass" exists only for workflows**, via
`execute_success_checks` (`crates/biorouter/src/agents/retry.rs:191-218`). A
`SuccessCheck::Shell { command }` is run; if it exits non-zero the loop resets to
the initial messages (`retry.rs:98-110`, `retry.rs:149`) and retries until
`max_retries` (`retry.rs:131-142`). This is the closest thing to
`tests-must-pass`, but:

- it's **opt-in and workflow-only** — an interactive coding session has no
  equivalent;
- the only check type is `Shell` (`retry.rs:199`) — the enum has a single
  variant, no regex/file-exists/JSON checks;
- on failure it **discards all progress** (full message reset) rather than
  telling the agent what failed and letting it iterate.

For everyday interactive chat, "done" is **whatever the model decides**, gated
only by optional Stop hooks / `/goal`. No enforced verification.

### Feedback loops from tools back to the agent

- **Compiler / shell / MCP errors as text.** Tool failures return
  `CallToolResult { is_error: Some(true), content: [text …] }`
  (`tool_execution.rs:111-116`). Compiler errors, test failures, stack traces etc.
  arrive as the tool's stdout/stderr and the model reads them like any other
  message. There is **no structured error channel** — a `cargo build` failure and
  a successful build both come back as a text blob; the agent must parse it.
  Shell output is size-capped (`rmcp_developer.rs:1422` `validate_shell_output_size`).

- **Artifact render-error auto-repair (the one true self-repair loop).** A
  generated figure/app iframe posts `biorouter-viz-render-error`; only the
  trusted srcDoc frame is honored (anti-injection guard)
  (`ui/desktop/src/components/artifacts/ArtifactViewer.tsx:206-256`). It becomes a
  hidden, agent-visible user message
  (`createArtifactRenderRepairMessage`,
  `ui/desktop/src/types/message.ts:64-94`) whose body is a "[BioRouter artifact
  render guardrail]" repair policy: *"inspect the generated artifact code, fix
  the runtime error, and render a corrected artifact"*
  (`message.ts:76-82`). It is fed back **only while the conversation is live** —
  `shouldAutoRepairArtifact` requires a running turn or a finish within a 15 s
  grace window (`BaseChat.tsx:98`, `:125-134`), otherwise a stale figure the user
  reopened would silently resume a finished chat
  (`BaseChat.tsx:838-858`). If a turn is mid-flight the fix is queued to land at
  turn end (`BaseChat.tsx:850-854`, `:883-888`). Errors are de-duplicated by
  key (`BaseChat.tsx:863-870`, `ArtifactViewer.tsx:242-244`). This is the single
  most sophisticated verification loop in the system — and it lives entirely in
  the **UI**, not the agent core, and only covers rendered artifacts.

- **Repetition inspection.** `RepetitionInspector` in
  `crates/biorouter/src/tool_monitor.rs` flags identical repeated tool calls
  (`InternalToolCall::matches`, `tool_monitor.rs:16-38`); it's wired into the
  retry manager for reset (`retry.rs:44,63-70,78-82`). This detects a stuck loop,
  not correctness.

There is **no** loop that runs the compiler/linter automatically after an edit —
`text_editor` writes do not trigger a build or diagnostics; the `analyze` tool
(tree-sitter semantic analysis, `developer/analyze/`) is a *manual* tool the
agent must choose to call, not an on-save check.

### Explore more or answer quickly — the effort and planning machinery

Very little explicit machinery; it's mostly prompt guidance:

- **System prompt** biases toward answering: "If the user asks *how* to do
  something, answer first rather than immediately acting"; "Be concise. Prefer
  the shortest answer that fully addresses the request — often 1-3 sentences"
  (`system.md:57-58`, `:87-89`). There's no effort budget, no "think harder"
  escalation.

- **Plan mode is a separate one-shot planner, not an in-loop mode.**
  `crates/biorouter/src/prompts/plan.md` defines a "specialized planner AI" that
  either returns clarifying questions *or* a step-by-step plan that becomes the
  first user message of a fresh executor conversation
  (`plan.md:1-4`, `:26-29`). It is invoked deliberately (the user picks plan
  mode); it is **not** a `plan.md` file the agent maintains, and there's no
  automatic "this task is complex → enter plan mode" trigger. It's essentially a
  prompt-rewrite step, not a persistent planning artifact like Claude Code's
  TODO/plan tracking. (A `todo_extension.rs` exists separately for task lists but
  isn't a verification mechanism.)

- **Subagents** are told to minimize exploration: "Use the minimum number of
  tools needed", "Avoid exploratory tool usage unless explicitly required", "Stop
  using tools once you have sufficient information"
  (`crates/biorouter/src/prompts/subagent_system.md:25-30`), bounded by
  `max_turns` (`subagent_system.md:10`). This is the only place with an explicit
  "explore less" budget, and it's a turn cap, not a reasoning-effort control.

No reasoning-effort / thinking-budget knob is surfaced at the agent-loop level;
the planning/exploration tradeoff is left to the model and prompt tone.

### What `final_output_tool.rs` is for, and how output is validated

`FinalOutputTool` is the **structured-output contract for workflows, recipes,
and schema-bearing subagents**. Construction *panics* unless a non-empty,
meta-valid JSON Schema is supplied (`final_output_tool.rs:20-31`) — a schema is
mandatory. It:

- Registers a tool named `workflow__final_output` whose input schema *is* the
  declared output schema (`final_output_tool.rs:61-71`), and injects a system
  prompt: "You MUST use the `final_output` tool… it must match the following
  schema" (`final_output_tool.rs:81-92`, added via `add_final_output_tool`,
  `agent.rs:801-806`).
- On call, compiles and validates the arguments against the schema, collecting
  every error with its instance path (`final_output_tool.rs:94-117`); on success
  stores the answer as a **single-line JSON string** for easy script extraction
  (`final_output_tool.rs:150-153`), on failure returns `INVALID_PARAMS` with the
  full error list + expected schema (`final_output_tool.rs:135-139`).
- Is dispatched specially in the loop (`agent.rs:887-901`), added to the tool
  list only when present (`agent.rs:1197-1198`), and its collected value is
  emitted as the terminal assistant message
  (`agent.rs:1561-1564`, `:2078-2082`). Retry clears it between attempts
  (`retry.rs:106-109`). Subagents surface it when the task had a response schema
  (`agents/subagent_handler.rs:236-247`).

**Separately, `structured_output.rs` exists but is NOT wired in.** Its own module
doc says "The agent-loop wiring … is a separate, carefully-sequenced change"
(`crates/biorouter/src/agents/structured_output.rs:8-11`). It provides
fence-stripping, parse+validate, and a `reprompt_message` for the BRSDK
`output_type` contract — but a repo-wide grep finds **zero call sites** outside
its own file (only `pub mod structured_output;` in `agents/mod.rs:23`). So the
"validate the terminal message, re-prompt up to N times" loop it was written for
**does not exist yet** — it is tested, dead-ish primitive code awaiting wiring.

### What is missing, versus what a careful engineer would want

See [Gaps and weaknesses](#gaps-and-weaknesses) below.

## Notable design choices (worth keeping)

- **Schema validation returns actionable, path-scoped errors** and re-prompts
  rather than silently accepting bad output (`final_output_tool.rs:103-116`,
  `structured_output.rs:44-60`). Good corrective-loop design.
- **`final_output` mandates a schema at construction** (panics otherwise,
  `final_output_tool.rs:20-31`) — fail-fast, no "optional validation that never
  runs."
- **Artifact auto-repair is liveness-gated and injection-hardened.** It refuses
  to resurrect a finished chat (`shouldAutoRepairArtifact`, `BaseChat.tsx:125-134`)
  and only trusts the frame it generated (`ArtifactViewer.tsx:210-215`). Both are
  subtle correctness/safety wins.
- **Truncation vs. natural stop is distinguished** via `finish_reason` and
  auto-continued with a bounded cap (`agent.rs:2049-2071`) — avoids ending on a
  half-written answer without risking an infinite loop.
- **`/goal` Stop-hook loop has real iteration + stall caps and graceful give-up**
  (`goal.rs:1-31`) — a mature "keep working until verified" primitive that
  avoids the runaway-loop failure mode.
- **Observability data model is redaction-safe by default** (spans carry timings
  + token counts, never args/text — `observability/mod.rs:9-11,196-218`).

## Gaps and weaknesses

These eleven items fed the improvement phase. They are what other documents in this
review cite as `verification.md gap #N`; the numbering below is that scheme and is stable.

1. **No enforced verification in interactive chat.** The only hard gate
   (`execute_success_checks`) is workflow-only (`retry.rs:191`). A normal coding
   session can declare "done" with a broken build and nothing stops it. Modern
   coding agents run tests/lints/typecheck as an enforced completion gate; here
   it's a prompt suggestion (`system.md:64`). **Biggest gap.**

2. **`structured_output.rs` is written but unwired — a validation loop that
   doesn't run.** The BRSDK `output_type` contract has parse/validate/re-prompt
   primitives with tests but zero call sites (`agents/mod.rs:23` only). Any app
   relying on `output_type` currently gets no enforcement.

3. **No post-edit compile/lint feedback loop.** `text_editor` writes never
   trigger diagnostics; `analyze` is a manual tree-sitter tool, not a language
   server. The agent only learns of a compile error if it *chooses* to run the
   build. A careful engineer would want LSP/typecheck diagnostics fed back
   automatically after edits (an `LSP` tool is even listed as available but is not
   part of the developer extension's edit path).

4. **Tool errors are unstructured text.** `is_error` is a bool
   (`tool_execution.rs:111-116`); there's no typed error taxonomy, no
   distinction between "transient/retryable" and "your code is wrong". The model
   must parse stderr. No structured propagation of, e.g., compiler diagnostics
   with file:line.

5. **Success-check retry throws away all progress.** On failure it resets to
   initial messages (`retry.rs:98-110`) instead of surfacing *what failed* and
   letting the agent iterate on the diff. This is a blunt instrument versus a
   test-driven "here's the failing test, fix it" loop.

6. **Only one success-check type.** `SuccessCheck` is a single-variant enum
   (`Shell`, `retry.rs:199`). No regex/output-contains, file-exists, JSON-schema,
   or "no diff in golden file" checks.

7. **No self-critique / reflection / self-consistency pass.** Nothing re-reads
   the agent's own answer for correctness, contradiction, or hallucination before
   returning it — despite the science-accuracy mandate in `system.md:82-83`.
   No verifier model, no LLM-as-judge on ordinary answers (only `/goal` and
   permission judging use judges).

8. **Observability/tracing is inert.** `observability::{ObsEvent, TraceBuilder,
   TraceProcessor}` has no emit sites anywhere (grep-confirmed; its own doc admits
   wiring is pending — `observability/mod.rs:6-8`), and `tracing/mod.rs:1-2` is a
   2-line "layers have been removed" stub. So there is **no runtime span/trace of
   what the agent did** to audit verification quality — a reviewer/operator can't
   see tool-failure rates, retry counts, or repair-loop firings. The
   redaction-safe data model exists but records nothing.

9. **Planning is one-shot, not maintained.** `plan.md` rewrites the prompt once
   (`plan.md:26-29`); there's no living plan/TODO the agent checks off and
   verifies against, so no plan-completion checkpoint. Complexity → plan-mode is a
   manual user choice, not an automatic effort trigger.

10. **Artifact repair is the only auto-feedback loop and it's UI-only.** It lives
    in React (`BaseChat.tsx`), covers only rendered figures/apps, and doesn't
    exist for the CLI or headless daemon. Runtime failures of non-artifact
    outputs (a script that errors when the *user* later runs it, a bad SQL
    result) have no equivalent auto-repair.

11. **`final_output` validation is structural only.** JSON Schema checks shape,
    not semantic correctness — a syntactically valid but factually wrong answer
    passes. No cross-check against tool evidence.

## Related documentation

- [Loop detection, repetition and stuck states](loop-and-stuck-detection.md) — the sibling review covering the `/goal` stall logic and repetition inspection from the loop-safety side; the two overlap on no-progress detection.
- [Execution and verification compared with other agents](../competitive-comparison/execution-and-verification.md) — how BioRouter's thin verification story measures against eight other coding agents.
- [Core agent loop and tool dispatch](core-loop-and-tool-dispatch.md) — where the no-tool-call termination decision described here actually lives.
- [Verify-and-checkpoint stop hook](../../../agent-loop/hooks/verify-and-checkpoint-stop-hook.md) — the living, hook-based answer to "nothing enforces done-ness".
- [Master improvement proposals](../improvement-proposals.md) — the BR-NN proposals (BR-47 to BR-50) these gaps became.
