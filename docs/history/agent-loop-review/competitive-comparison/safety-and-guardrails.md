# Hooks, permissions, guardrails and loop detection

> **What this is.** Chapter 4 of the four-part competitive comparison in the
> 2026-07 agentic-loop review: how BioRouter's safety surface — lifecycle hooks,
> the permission/approval flow, LLM judges, sandboxing, dangerous-command
> detection, and repetition/stuck detection — compared against nine other
> open-source coding agents.
> **Status:** Superseded — the "computed-but-discarded / dead-code" thesis this
> chapter is built on has been resolved, so most "behind" rows are now wrong. BR-18
> revived read-only auto-approve and per-action risk grading (SmartApprove no longer
> equals Approve), BR-19 added PreToolUse input rewrite and PostToolUse blocking,
> BR-20 added the always-on catastrophic-command denylist, and BR-29/BR-30 replaced
> the single-deny exact-duplicate guard with staged, semantic loop detection. For
> what shipped, read [the agent-loop campaign outcome report](../../agent-loop-campaign/outcome-report.md),
> [the wave-2 hooks and permissions report](../../agent-loop-campaign/wave-reports/wave-2-hooks-and-permissions.md)
> and [the wave-2 loop-detection report](../../agent-loop-campaign/wave-reports/wave-2-loop-detection.md).
> **Audience:** developers and maintainers working on permissions, hooks and
> agent safety.

This chapter was written on 2026-07-12 as part of the agentic-loop review. Its
subject is everything that stands between the model deciding to act and the act
happening: hook events, permission modes, sandboxes, command denylists, and the
detectors that notice an agent has stopped making progress. Read it as a snapshot
of the state of the art at that date and of BioRouter's position in it — not as a
description of current BioRouter behaviour.

The recurring finding across the three grounding reviews was a
**"computed-but-discarded / dead-code" gap**: BioRouter had ported the *shape* of
a state-of-the-art safety layer from Claude Code and Goose, but several
load-bearing pieces were inert at the reviewed commit. That commit was not
recorded in the original document.

Four conventions used throughout:

- **`BR-NN` identifiers** are proposal numbers from the same review.
  [The improvement proposals register](../improvement-proposals.md) defines
  BR-1…BR-67 with Problem / Proposal / Affected code / Impact / Effort / Risk.
- **Gap citations** take the form *(hooks review, gap #7)* or *(guardrails review,
  Q2)*. They name the subsystem review that established the finding and its
  numbered gap or question. The three grounding reviews for this chapter are
  [the hooks-system review](../subsystem-reviews/hooks-system.md)
  ("hooks review"),
  [the guardrails and permissions review](../subsystem-reviews/guardrails-and-permissions.md)
  ("guardrails review"), and
  [the loop and stuck-detection review](../subsystem-reviews/loop-and-stuck-detection.md)
  ("loop-detection review").
- **The numbered items** in "Where BioRouter was behind" are themselves citation
  anchors: other documents in this review refer to them as
  `safety-and-guardrails.md behind #1` … `behind #9`. The numbering is stable —
  do not renumber it.
- **External claims** are grounded in that tool's report under
  [the coding-agent landscape research](../../../research/coding-agent-landscape/).

## Comparison across ten agents

The table has one column per agent, in this order: BioRouter, Goose,
Cline, OpenCode, Pi, Aider, OpenHands, Codex CLI, Gemini CLI, Claude Code. It is
wide and scrolls horizontally.

> **Note.** The BioRouter column is superseded — see the status header. The nine
> competitor columns are a 2026-07 snapshot and have not been re-verified since.
> Bold in the table marks a cell the review singled out — sometimes a
> best-in-class implementation, sometimes a BioRouter gap. Which is which is
> settled by the prose sections below, not by the formatting.

| Aspect | BioRouter | Goose | Cline | OpenCode | Pi | Aider | OpenHands | Codex CLI | Gemini CLI | Claude Code |
|---|---|---|---|---|---|---|---|---|---|---|
| Lifecycle hook events | 13 (all wired) | 11 | ~11 | broad plugin bus | huge typed bus | none | 6 | 7 | 11 | ~30 |
| Hook can block | yes (4 events) | yes (PreTool/Stop) | yes | yes (throw) | yes | no | yes | yes | yes (exit 2) | yes |
| Hook injects context | partial (2 events; PreTool drops it) | yes | yes | yes | yes | no | limited | yes | yes | yes |
| Hook rewrites tool input | **no** | no | no | mutate args | **yes (mutate)** | no | no | **yes (updated_input)** | **yes (tool_input)** | **yes (updatedInput)** |
| Hook rewrites tool output | no | no | yes (patch) | mutate | yes (chain) | no | no | yes | yes | yes (updatedToolOutput) |
| Hook intercepts model req/resp | no | no | no | no | **yes** | no | no | no | **yes (Before/AfterModel)** | no |
| PostToolUse can block | **no (observe-only)** | no | yes | yes | yes | n/a | yes | yes | yes | yes |
| Hook config surface | file/env only | file (Open Plugins) | file | plugin/npm | TS module | none | file | declarative rules | TOML+file | settings.json |
| Managed/enterprise policy tier | **no (2 tiers)** | no | no | no | no | no | no | no | **yes (admin wins)** | **yes (managed)** |
| Permission modes | 4 (Auto/Approve/Smart/Chat) | 4 | Plan/Act + auto-approve | per-tool allow/ask/deny | YOLO only | confirm/yes/dry-run | policy+confirm | approval×sandbox | 4 + policy | 6 |
| Read-only auto-approve | **broken (empty sets)** | yes (annotations) | model-classified | defaults | tool allowlist | file split | read-only exempt | untrusted mode | policy | default mode |
| LLM permission judge | **dead code** | yes (PermissionJudge) | yes (risk classify) | no | no | no | **yes (default on)** | no | no | **yes (auto mode)** |
| OS-level sandbox | **none** | none | subprocess sandbox | coarse (dir gate) | none (by design) | none | **Docker/VM runtime** | **Seatbelt/Landlock/token** | **Seatbelt/container** | **yes (Bash sandbox)** |
| Dangerous-command detection | regex table (off by default, ask-only) | none | model classify | wildcard deny | opt-in ext | none | LLM risk grade | **execpolicy Starlark** | **TOML policy engine** | **classifier model** |
| Command allow/deny rules | coarse (hashed exact args) | permission.yaml | CommandPermission | **wildcard last-wins** | allowlist | none | policy | execpolicy | **tiered TOML** | allow/ask/deny rules |
| Supply-chain / malware check | **yes (OSV MAL-*)** | no | no | no | no | no | no | no | no | no |
| PII/PHI guardrail | yes (local, apps-only) | no | no | no | no | no | no | no | no | protected paths |
| Exact-duplicate tool guard | yes (>3, reason hidden) | yes (RepetitionInspector) | yes (hash) | yes (doom_loop) | none | none | yes (semantic eq) | 3-turn blocked rule | yes (hash) | internal (undoc) |
| Staged/soft-then-hard loop stop | **no (single deny)** | no | **yes (3 warn / 5 stop)** | **yes (3 warn / 5 gate)** | no | no | yes (STUCK breaks) | no | **yes (3-layer)** | 3-in-a-row fallback |
| Failing-approach / no-progress | **only in /goal** | no | mistake tracker | targeted scenarios | no | reflection cap | **action-error+alt+monologue** | goal audit | **LLM loop check** | classifier fallback |
| Oscillation (A/B/A/B) detect | **no** | no | no | partial | no | no | **yes** | no | no | no |
| Mistake-streak handling | none (generic decline) | empty-turn cap 3 | **recoverable+inject guidance** | no | no | reflections 3 | critic refine | goal blocked audit | AfterAgent retry | classifier counters |
| Max-turns / iteration cap | 100 (soft) | 1000 | maxIterations | via overflow | **none** | 3 reflections | 500 (hard) | token budget | maxSessionTurns | subagent caps |
| Budget cap (tokens/$/wall) | **no** | no | resource-limiter | no | no | no | **yes ($/run)** | **yes (token budget)** | no | no |

## Where BioRouter was ahead

1. **Hook event coverage is a genuine superset of Claude Code's documented set, and every
   enum arm is actually wired.** The hooks review counts **13 event variants** — all of
   Claude Code's public events plus `PostToolUseFailure`, `PermissionRequest`,
   `SubagentStart`, and `PostCompact` — with no dead enum arms (each has a real fire site).
   Only Claude Code (~30 events) and Pi (a very large typed bus) expose more. Goose (11),
   OpenHands (6), Codex (7), and Gemini (11) all cover less. The stdin payload and stdout
   decision JSON deliberately match Claude Code's field names, so existing hook scripts port
   unchanged — a real portability win, not a lookalike.

2. **Supply-chain malware gating at extension install is nearly unique.** BioRouter's
   `extension_malware_check.rs` does an OSV.dev lookup when launching a stdio MCP extension
   and denies any package flagged with a `MAL-*` advisory (guardrails review, Q5). None of
   the nine comparators ships an equivalent published-package malware check; this is a
   defensible edge for an agent that one-click-installs marketplace extensions.

3. **On-device PII/PHI detection with checksum validation.** `guardrails/pii.rs` is a fully
   local regex+checksum detector (Luhn for cards, structural SSN validity, keyword-anchored
   MRN/DOB) — no comparator ships an equivalent, and it is the right call for a biomedical
   tool that must not ship clinical text to a model. (Caveat under "behind": it is
   BRSDK-app-only — the BioRouter App SDK that Agent Drafter apps are built
   against — not on the main loop.)

4. **Most-restrictive-wins, escalation-only inspector merge.** The inspector chain
   (`security → permission → repetition → hooks`) with a monotonic "raise the bar, never
   lower it" override and a "no verdict → needs_approval" default is fail-closed by
   construction (guardrails review, Q1). This is cleaner than Aider's or Pi's ad-hoc gating
   and comparable to OpenHands' `ConfirmationPolicy`.

5. **Stop-hook block cap.** `STOP_HOOK_BLOCK_CAP = 5` bounds runaway "keep-working" loops
   while `stop_hook_active` lets well-behaved judges exit early (hooks review). Goose caps
   at 8; Claude Code publishes no numeric cap. This is a small correctness win over the
   reference design.

## Where BioRouter was behind

> **Warning.** Most findings below were fixed after this review — see the status
> header. They are preserved as the record of what the review found. The item
> numbers are citation anchors used elsewhere in this review; do not renumber them.

Ranked by how load-bearing the gap is for autonomous safety.

1. **No hook can rewrite tool input — and the best-in-class mechanism is small and
   concrete.** BioRouter hooks can only allow/deny/ask/inject; there is no rewrite path
   anywhere (hooks review, Q2 and gap #7). **Claude Code, Codex CLI, and Gemini CLI all
   do this**, and Codex is the cleanest to reimplement: a `PreToolUse` handler returns a
   `PreToolUseOutcome` with three optional fields — `should_block` + `block_reason`,
   `additional_contexts` (model-visible), and **`updated_input`** (the rewritten tool args,
   applied with no re-validation). Gemini's is identical in spirit
   (`hookSpecificOutput.tool_input`), Pi's `tool_call` event mutates `event.input` in place.
   This one field turns hooks from a veto into a policy engine: sandbox a path, redact a
   payload, normalize a shell command — without touching the Rust loop.

2. **The LLM permission judge and read-only auto-approve are dead code, so SmartApprove ≡
   Approve.** The guardrails review (Q2, gaps #1–#2) is blunt: `check_tool_permissions`
   has zero callers, `detect_read_only_tools` is only reachable through it, and the live
   `PermissionInspector`'s `readonly_tools`/`regular_tools` sets are constructed empty with
   no setter. So the "smart" tier never consults the model and never auto-approves reads —
   it over-prompts on everything. **OpenHands is best-in-class** and easiest to copy: the
   LLM emits a `security_risk` (LOW/MEDIUM/HIGH/UNKNOWN) per action, a pluggable
   `ConfirmRisky(threshold=HIGH, confirm_unknown=True)` policy decides when to pause,
   read-only tools auto-pass, and analysis errors fail-safe to HIGH. Goose's live
   `PermissionJudge` + `read_only` tool annotations and Claude Code's tool-results-stripped
   `auto`-mode classifier are the other two working references.

3. **No OS-level sandbox at all.** BioRouter's guardrail is permission gating, not process
   isolation (per the guardrails review — no comparator note needed; the reviews describe
   only inspectors and `.biorouterignore`). **Codex CLI is best-in-class**: it cleanly
   separates *what is technically possible* (OS sandbox) from *when to ask* (approval
   policy), and enforces the sandbox natively and deny-by-default — macOS Seatbelt via
   `sandbox-exec -p` with writable-roots injected and network-outbound denied; Linux via a
   `codex-linux-sandbox` helper combining **Landlock** (filesystem) + **seccomp** (blocks
   network syscalls) + **bubblewrap** namespaces; Windows via a restricted process token. On
   a sandbox denial it *escalates to an approval prompt* instead of hard-failing. OpenHands
   (Docker/VM `Workspace` backends), Gemini CLI (Seatbelt + Docker/Podman), and Claude Code
   (Bash sandbox) all have real isolation; BioRouter, Goose, Pi, and Aider have none.

4. **Dangerous-command detection is a signature scanner that is off by default and only
   ever asks.** `security/patterns.rs` is a 40+ entry regex table; even when enabled it
   never hard-blocks (`should_ask_user: true`), and it is trivially evadable (`r''m -rf`,
   `$(printf …)`, env-var indirection, a different tool wrapper), with no argv parsing or
   path canonicalization (guardrails review, Q3 and gaps #3–#4). **Codex's `execpolicy` is
   best-in-class**: a Starlark policy engine of
   `prefix_rule(pattern=…, decision=allow|prompt|forbidden, justification=…)` with
   `match`/`not_match` self-tests and `host_executable` path pinning — auditable and
   testable, not a fragile signature list. **Gemini CLI's declarative TOML policy engine** is
   the governance-layer analogue a UCSF/lab deployment needs: rules of
   `{tool-glob, args-regex, approvalMode, interactive} → allow|deny|ask_user` resolved by
   tier (Default < Extension < User < **Admin**, admin always wins, ownership-verified),
   living outside the binary as config. OpenCode's simpler wildcard `"rm *": "deny"`
   last-match-wins grammar (with `.env` deny-by-default) is a lighter reimplementation
   target.

5. **Repetition detection is exact-duplicate-only, trivially defeated, and hides its reason
   from the model.** `RepetitionInspector.inspect` matches on byte-exact JSON and counts only
   *consecutive* calls; a one-char arg change, an `A/B/A/B` oscillation, or a repeated failing
   *result* all bypass it, and on trigger the model is told the generic `DECLINED_RESPONSE`
   ("the user declined") rather than the true "exceeded max repetitions" — actively misleading
   (loop-detection review, gaps #1–#2). Three references beat it:
   - **Gemini CLI's `LoopDetectionService` (best-in-class)** is a three-layer real-time
     detector fed every streaming event: (1) SHA-256 of `"${name}:${args}"`, threshold **5**
     consecutive identical calls; (2) content-"chanting" via 50-char sliding-window chunk
     hashing, threshold **10** occurrences within a tight window, with list/code-block
     false-positive guards; (3) a periodic **LLM loop check** after turn 30 inspecting the
     last 20 turns at 0.9 confidence.
   - **OpenHands' `StuckDetector`** adds the patterns BioRouter lacks entirely: repeating
     action-observation (N=4), repeating action-*error* (N=3), monologue (N=3), and the
     **alternating `[A,B,A,B]` loop** (N=4) — over the last 20 events *after the last user
     message*, with semantic equality that ignores IDs/metrics.
   - **Cline / OpenCode's staged escalation** (soft warning at 3 identical calls → hard
     stop / `doom_loop` permission gate at 5) is the cheapest single upgrade: surface the
     REP-001 reason as a soft nudge first, block only on persistence.

6. **No mistake-streak / recoverable-failure handling.** BioRouter has no counter for
   consecutive `api_error`/`invalid_tool_call`/`tool_execution_failed`. **Cline's
   `MistakeTracker` is best-in-class**: below the cap it emits a recoverable error and
   continues; *at* the cap it runs `onLimitReached`, which can either **continue with an
   injected recovery notice** (resetting the counter) or **stop with preserved state** — a
   "one more chance with a hint" pattern strictly better than a hard kill. Aider's single
   `reflected_message` channel (all failure types funnel into one bounded self-correction
   slot, cap 3) is the minimalist version.

7. **No global token / wall-clock / dollar budget per reply.** Only the 100-turn iteration
   count bounds a turn, and 429 backoff (~2 min/call) compounds inside it, so a throttled
   session can run far longer than a user expects (loop-detection review, gap #6). **Codex's
   goals token-budget** (`tokens_used`/`token_budget`/`remaining_tokens` re-injected each
   continuation, with `budget_limit.md` telling the model to wrap up) and **OpenHands'
   `max_budget_per_run` ($ cap → `MaxBudgetReached`)** are the two references.

8. **Guardrails are scoped to BRSDK apps, not the main loop; PostToolUse cannot block.** PII
   masking, `Block`, and `run_state` human-in-the-loop pauses only run on the Agent Drafter app socket; the
   CLI/GUI loop has no PII stage and never scans tool *output* — the classic injection vector
   (guardrails review, gap #6). And PostToolUse is observe-only although the decision is
   already computed (hooks review, gap #2), so "reject a write that fails lint" is
   impossible. Claude Code, Cline, OpenHands, Codex, and Gemini all let PostToolUse block.

9. **No native checkpoint/undo.** Every modern comparator (Cline shadow-git, OpenCode private
   git-object DB, Gemini/Claude Code shadow-repo + rewind, Aider commit-per-edit) has an undo
   safety net; BioRouter has none documented. This is adjacent to safety (it is the recovery
   mechanism that makes aggressive autonomy tolerable) and is called out as the single biggest
   gap in both the Goose and Claude Code external reviews.

## Best-in-class and worst-in-class per aspect

- **Hook event breadth:** *Best* — Claude Code (~30 events incl. model/compaction/task
  lifecycle vetoes). *Runner-up* — Pi (large typed, mutable bus). *Worst* — Aider (no hook
  system at all). BioRouter is strong here (13 wired events, 2nd tier).
- **Hook power (block + inject + rewrite input + rewrite output + model-req intercept):**
  *Best* — Gemini CLI (adds `BeforeModel`/`AfterModel`/`AfterAgent`-force-retry/`PreCompress`)
  and Codex (`updated_input` + Pre/Post-compact abort), tied. *Worst* — Aider (none),
  BioRouter (allow/deny/ask/inject only, and PreTool `additionalContext` is silently dropped).
- **Permission-judge / read-only auto-approve:** *Best* — OpenHands (per-action `security_risk`
  + `ConfirmRisky` + read-only exempt + fail-safe-HIGH). *Runner-up* — Claude Code auto-mode
  classifier (tool results stripped = injection-resistant); Goose live `PermissionJudge`.
  *Worst* — BioRouter (judge + read-only sets are dead code, so SmartApprove is inert) and Pi
  (YOLO, no gating by design).
- **OS sandboxing:** *Best* — Codex CLI (Seatbelt / Landlock+seccomp+bwrap / restricted token,
  deny-by-default, escalate-on-denial). *Runner-up* — OpenHands (Docker/VM), Gemini CLI, Claude
  Code. *Worst* — BioRouter, Goose, Aider, Pi (no isolation; Pi at least honestly declines to
  fake one).
- **Dangerous-command detection:** *Best* — Codex `execpolicy` (testable Starlark rules with
  justifications + path pinning). *Runner-up* — Gemini TOML policy engine (admin-tier override).
  *Worst* — Goose / Aider (nothing), with BioRouter close behind (regex table, off by default,
  ask-only, evadable).
- **Repetition/loop detection:** *Best* — Gemini CLI (three-layer: hash-count + content-chant
  + periodic LLM check). *Runner-up* — OpenHands `StuckDetector` (5 heuristics incl. oscillation
  and action-error). *Worst* — Pi / Aider (no repetition detector; Pi by design). BioRouter sits
  just above worst: it catches only exact consecutive duplicates and misreports the reason.
- **Mistake-streak handling:** *Best* — Cline (recoverable + injected guidance or preserved-state
  stop). *Worst* — BioRouter (no streak counter; generic decline).
- **Budget/termination guarantee:** *Best* — OpenHands ($ + iteration cap) and Codex (token
  budget). *Worst* — Pi (no cap at all, by design); BioRouter has a soft 100-turn cap but no
  time/token/$ budget.
- **Supply-chain / PII:** *Best* — BioRouter (only agent with OSV malware gating and a local
  PII detector). *Worst* — essentially everyone else (no equivalent).

## Implications and where they landed

The recommendations below were the chapter's own. All of them were consolidated
into [the improvement proposals register](../improvement-proposals.md), which is
the authoritative list — read this section as the argument behind the proposals,
not as an open work queue. The register's BR-NN number is noted where the mapping
is one-to-one.

BioRouter's safety layer is architecturally ahead of most of the field on *surface area*
(13 wired hook events, a fail-closed escalation-only inspector chain, unique OSV + PII
guards) but **is undermined by inert implementations of exactly the pieces that do the
work**. The internal reviews name three that should be treated as bugs, not roadmap:

1. **Revive the read-only auto-approve path and either wire or delete the LLM judge**
   (became **BR-18**). Populate `readonly_tools`/`regular_tools` from the extension
   manager's `read_only_hint` annotations (the comment says this was intended), so
   SmartApprove stops behaving identically to Approve. Adopt OpenHands' per-action
   `security_risk` + `ConfirmRisky` shape rather than resurrecting the dead
   `check_tool_permissions`.

2. **Add a rewrite path to PreToolUse hooks and let PostToolUse block** (became
   **BR-19**). Copy Codex's `PreToolUseOutcome` (`should_block` / `additional_contexts` /
   `updated_input`) and stop silently dropping PreToolUse
   `additionalContext`/`systemMessage` (hooks review, gap #1). Honoring the
   already-computed PostToolUse decision unlocks "reject a write that fails lint."

3. **Replace exact-duplicate repetition with a staged, semantic loop detector, and surface
   the reason** (became **BR-29** for the staged stop and honest reason, and **BR-30** for
   semantic/near-duplicate/oscillation detection). Minimum viable: Cline/OpenCode's
   soft-warn-at-3 / hard-stop-at-5 with the REP-001 reason forwarded to the model (fixing
   the misleading `DECLINED_RESPONSE`). Higher value: add OpenHands' alternating-pattern and
   action-*error* heuristics and Gemini's periodic LLM loop check, and bring the good `/goal`
   stall logic (`reason_similarity`, `GOAL_STALL_LIMIT`) to ordinary chat where most stuck
   loops actually occur.

Two further deployment-shaped upgrades follow from the lab context: a **Gemini-style
tiered/admin policy engine** (declarative, ownership-verified, outside the binary) would give
UCSF the non-overridable "no writes outside the data dir / always ask on `rm`" governance the
current 2-tier file-only config cannot express; and **real OS sandboxing** (Codex's
two-axis sandbox×approval model) would make autonomy bounded by the kernel rather than by
prompt compliance and the currently-off regex scanner. Finally, add a **global token/wall-clock
budget per reply** so a throttled or pathological session terminates on cost, not just on the
loose 100-iteration count.

## Related documentation

- [The hooks-system review](../subsystem-reviews/hooks-system.md) — the internal review behind the 13-event count and the PreToolUse/PostToolUse gaps cited above.
- [Guardrails and permissions](../subsystem-reviews/guardrails-and-permissions.md) — the internal review behind the dead-code permission-judge thesis, the regex scanner and the OSV/PII findings.
- [Loop and stuck detection](../subsystem-reviews/loop-and-stuck-detection.md) — the internal review behind the repetition, mistake-streak and budget-cap findings.
- [The command policy engine design](../../../agent-loop/designs/command-policy-engine.md) and [the managed policy tier design](../../../agent-loop/designs/managed-policy-tier.md) — the designs that answered this chapter's two deployment-shaped recommendations.
- [Tool loop, long-running tasks, checkpoints and verification](execution-and-verification.md) — the sibling chapter that covers checkpoints and undo in depth, which item #9 above only points at.
