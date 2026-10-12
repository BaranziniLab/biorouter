# Guardrails, security and the permission system — architecture review

> **What this is.** One of ten subsystem reviews from the 2026-07 BioRouter agentic-loop review. It documents the tool-call gauntlet — the four-inspector chain, permission modes and the approval flow, the security scanner, PII guardrails and the OSV malware check — and records ten gaps.
> **Status:** Historical record — a snapshot of the code *before* the agent-loop fix campaign, whose central findings were then implemented. Gaps #1 and #2 (empty read-only sets and a dead LLM judge, which together made `SmartApprove` identical to `Approve`) were fixed by BR-18, #3 (no screening in `Auto` mode) by BR-20, #4 (the evadable regex scanner) by BR-21, #6 and #9 (unused `ToolOutput` guardrail stages) by BR-22, #7 (`.biorouterignore` confined to the Developer server) by BR-23, #8 (exact-args permission keys) by BR-24, and #10 (`unwrap` panics) by BR-25.
> **Audience:** developers working on permissions, guardrails, or the security scanner.

Reviewed at commit `24cdc3a2` on branch `main`. This is the only one of the ten subsystem reviews that pins a commit: [self-verification and done-ness](self-verification-and-doneness.md) records only the date 2026-07-12, [state awareness and version control](state-awareness-and-version-control.md) records the unrelated branch `ui-hardening-a11y-tests`, and the remaining seven record no revision at all — so no sibling's line citations can be anchored the same way. Identifier key: `BR-NN` are proposal ids from the [master improvement-proposal list](../improvement-proposals.md); the numbered items under "Gaps and weaknesses" are what sibling reviews cite as `guardrails-permissions.md #N` (the file's former name), and `SEC-<uuid>` / `REP-001` are runtime finding ids emitted by the inspectors themselves.

## Overview

BioRouter's guard rail surface is split across three loosely coupled subsystems that live at different points in the tool lifecycle:

1. **Tool inspectors** (`tool_inspection.rs`) — a pluggable chain (`ToolInspectionManager`) run inside the agent loop *after* the model emits tool requests and *before* any tool executes.
   - Four inspectors are registered in a fixed order: `security` → `permission` → `repetition` → `hooks`.
   - Each returns `InspectionResult { action: Allow | Deny | RequireApproval(msg) }`.
   - The permission inspector's decisions are the baseline. The other inspectors are applied as **monotonic escalation-only** overrides: they can raise the bar to approval or deny, never lower it.

2. **The permission model** (`permission/`) — decides, per tool call, whether it is auto-approved, needs a human, or is denied, based on the session's `BioRouterMode` and per-tool user-stored `PermissionLevel`. Human approvals flow over an mpsc channel and can be persisted as `AlwaysAllow`/`NeverAllow`.

3. **Content guardrails** (`guardrails/`) — an on-device masker for personally identifiable information and protected health information (PII/PHI, `pii.rs`) plus a serializable human-in-the-loop pause state (`run_state.rs`). These are **BRSDK-app-only** (Agent Drafter apps), wired in `biorouter-server/src/routes/apps.rs`, not in the main CLI/GUI agent loop.

Two more pieces sit outside the inspector chain:
- **Extension malware check** (`agents/extension_malware_check.rs`) — an OSV.dev lookup run at *extension launch* time (not tool-call time).
- **`.biorouterignore`** — enforced inside the Developer MCP server (`biorouter-mcp/src/developer/rmcp_developer.rs`), not in the core agent.

### Data-flow (the tool-call gauntlet)

```text
model emits tool requests
   └─ agent.rs reply loop (~line 1704)
      ├─ BioRouterMode::Chat → skip every tool, splice CHAT_MODE_TOOL_SKIPPED_RESPONSE (no execution)
      └─ else → tool_inspection_manager.inspect_tools(requests, msgs, mode, session)   [agent.rs:1723]
            ├─ SecurityInspector.inspect()      → prompt-injection scan (DISABLED by default)
            ├─ PermissionInspector.inspect()    → mode + user PermissionLevel → Allow/Deny/RequireApproval
            ├─ RepetitionInspector.inspect()    → same-tool-args loop breaker (max 3)
            └─ HookInspector.inspect()          → user PreToolUse hooks
         process_inspection_results_with_permission_inspector()   [agent.rs:1732]
            → permission baseline, then non-permission results applied as Deny/RequireApproval overrides
         ├─ approved  → handle_approved_and_denied_tools → dispatch_tool_call
         ├─ needs_approval → handle_approval_tool_requests → ActionRequired frame → await mpsc confirmation
         │       └─ AllowOnce/AlwaysAllow → dispatch;  Deny → DECLINED_RESPONSE;  AlwaysAllow/AlwaysDeny persist
         └─ denied   → declined tool result

Extension launch (separate path): extension_manager.rs:568 → OSV MAL-* check → deny or fail-open
Developer MCP tool (separate path): is_ignored(path) → refuse read/shell/analyze on .biorouterignore matches
BRSDK app socket (separate path): apply_pii_policy on user input → mask/block; RunState pause for HITL approvals
```

## Review questions answered

### Permission modes and how tool-call approval flows (auto-approve, ask, deny)

There are **two distinct enums** and they must not be confused:

- **`BioRouterMode`** — the session-wide policy: `Auto`, `Approve`, `SmartApprove`, `Chat` (`config/biorouter_mode.rs:7-11`, string forms `auto`/`approve`/`smart_approve`/`chat` at `:19-22`).
- **`PermissionLevel`** — the per-tool stored user preference: `AlwaysAllow`, `AskBefore`, `NeverAllow` (referenced throughout `permission_inspector.rs:126-131`).
- **`Permission`** — the *live human decision* returned from the UI: `AlwaysAllow`, `AllowOnce`, `Cancel`, `DenyOnce`, `AlwaysDeny` (`permission/permission_confirmation.rs:5-11`).

The live decision path is `PermissionInspector::inspect` (`permission_inspector.rs:106-188`):
- `Chat` → `continue` (tool skipped entirely; the agent later fills a canned skip response, `agent.rs:1704-1720`).
- `Auto` → `InspectionAction::Allow` (everything auto-approved, `permission_inspector.rs:122`).
- `Approve | SmartApprove` (`:123-150`): (1) user permission first — `AlwaysAllow`→Allow, `NeverAllow`→Deny, `AskBefore`→RequireApproval; (2) else if the tool is in `readonly_tools` **or** `regular_tools`, Allow; (3) else if it is the extension-management tool, RequireApproval with a security note; (4) else RequireApproval (default-deny-to-human for unknown tools).

Baseline-vs-override merging: `process_inspection_results` (`permission_inspector.rs:35-93`) seeds the result from the permission inspector (defaulting to `needs_approval` if a request has no permission verdict — `:72-75`), then `apply_inspection_results_to_permissions` (`tool_inspection.rs:181-261`) lets any other inspector move a request `approved → needs_approval → denied` but **never** the other direction (`Allow` is a no-op override, `:253-256`). Good: fail-closed.

Human approval flow (`agents/tool_execution.rs:150-229`): a `needs_approval` request yields an `ActionRequired` confirmation message carrying any inspector warning string (`:161-169`), then blocks on `confirmation_rx.recv()` (`:171`). On `AllowOnce`/`AlwaysAllow` it dispatches the tool (`:184-197`); on anything else it returns `DECLINED_RESPONSE` as an error tool result (`:205-219`). `AlwaysAllow`/`AlwaysDeny` additionally persist a `PermissionLevel` via `update_permission_manager` (`:200-203`, `:221-225`). The confirmation is delivered by `Agent::handle_confirmation` over an mpsc sender (`agent.rs:1228-1236`).

### The LLM permission judge — when it is consulted and what it decides

The judge (`detect_read_only_tools`, `permission_judge.rs:132-160`) asks the configured `Provider` to classify which pending tools are strictly read-only, using a single-tool schema `platform__tool_by_tool_permission` (`:23-75`) and the system prompt `prompts/permission_judge.md` ("You are a careful security analyst… When in doubt, classify an operation as NOT read-only"). It parses the model's tool call for a `read_only_tools` array (`:107-129`) and, on any error, returns an empty list — i.e., **fails toward requiring approval**. It is invoked only from `check_tool_permissions`'s `smart_approve` branch (`permission_judge.rs:239-259`), which also caches the verdict into a `smart_approve` `PermissionLevel`.

**Critical absence finding:** `check_tool_permissions` has **zero callers** in the entire workspace (`grep` for the name outside its definition returns nothing), and `detect_read_only_tools` is called *only* from inside `check_tool_permissions`. The live agent loop uses `PermissionInspector::inspect`, which contains a **parallel, simpler** re-implementation that **never calls the LLM judge**. So at this revision the LLM permission judge is effectively dead code, and `SmartApprove` and `Approve` behave identically in practice (see the full gauntlet below, and gaps #1 and #2).

### What the security scanner detects, and what it does on detection

The scanner detects two things:
- **Dangerous shell commands / injection** via a static regex table `THREAT_PATTERNS` (`security/patterns.rs:48-353`): 40+ patterns across `FileSystemDestruction`, `RemoteCodeExecution`, `DataExfiltration`, `SystemModification`, `NetworkAccess`, `ProcessManipulation`, `PrivilegeEscalation`, `CommandInjection` (e.g. `rm -rf` on system dirs → Critical, `curl … | bash` → Critical, reverse shells, `dd` disk wipe, `/etc/passwd` reads, netcat listeners, base64/hex-decoded pipes). Each has a `RiskLevel` mapped to a fixed confidence: Critical 0.95, High 0.75, Medium 0.60, Low 0.45 (`patterns.rs:37-44`).
- **Prompt injection** via an optional ML text-classifier over the tool args *and* the last ≤10 user messages (`scanner.rs:158-190`), calling a HuggingFace-style classification endpoint (`classification_client.rs:116-209`) that maps an `INJECTION`/`LABEL_1` score to an injection probability.

Scoring: per-scan confidence is `max(ml_confidence, pattern_confidence)` (`scanner.rs:146-156`); a context-awareness step *suppresses* a finding when the conversation looks safe and the tool only tripped non-Critical patterns (`scanner.rs:192-219`) — an explicit false-positive reducer. Malicious iff confidence ≥ threshold, default **0.8** (`scanner.rs:102-106`, `128`).

On detection, the action is **ask, never hard-block** (unless the user then denies): `SecurityManager::analyze_tool_requests` only emits a `SecurityResult` when confidence is *above* threshold and always sets `should_ask_user: true` (`security/mod.rs:133-142`); below-threshold findings are logged but non-blocking (`:143-151`). `SecurityInspector` converts that into `RequireApproval` with a "🔒 Security Alert" message including confidence %, explanation, and a `SEC-<uuid>` finding id (`security_inspector.rs:27-51`). So the outcome is **annotate + escalate to human approval**; it does not autonomously deny.

**Default-off:** the whole scanner is gated on `SECURITY_PROMPT_ENABLED` (default `false`, `security/mod.rs:35-41`) and ML on `SECURITY_PROMPT_CLASSIFIER_ENABLED` (default `false`, `:43-49`). `SecurityInspector::is_enabled` returns the former (`security_inspector.rs:89-92`), so in a stock install the security inspector is a no-op.

### PII guardrails and the run_state pause

**PII (`guardrails/pii.rs`):** a fully local, no-network regex+checksum detector for `Ssn, Mrn, Dob, Phone, Email, CreditCard, IpAddress, PersonName` (`:19-28`). Precision-tuned: SSNs must be dash/space formatted *and* pass a structural validity check (`ssn_valid`, area ≠ 000/666/<900, `:67-80`); credit cards must pass Luhn (`luhn_ok`, `:116-135`); MRN/DOB/PersonName are keyword-anchored so bare dates/9-digit ids/capitalized prose are not flagged (`:86-113`). `scan` de-overlaps matches (earliest-then-longest, `:198-208`); `mask` rewrites each span to `[REDACTED:KIND]` (`:216-232`). The seam comment (`:10-12`) explicitly leaves room for a Presidio/ONNX-NER upgrade behind `scan`.

Wiring is **BRSDK-app-only**: `apply_pii_policy` (`biorouter-server/src/routes/apps.rs:1353-1379`) applies a per-app `PiiMode::{Off,Mask,Block}` to the **user input** at the app socket boundary, and only if the user opted in globally (`BrsdkSettings.pii_guardrail`, default off, `apps.rs:1121-1130`). `Mask` rewrites the prompt and emits a `guardrail` frame; `Block` refuses the turn with `type:done` (`apps.rs:1131-1154`). There is **no** tool-input/tool-output/final-answer PII stage despite `GuardrailStage` enumerating `ToolInput/ToolOutput/Output` (`guardrails/mod.rs:13-26`) — those stages are declared but unused.

**run_state (`guardrails/run_state.rs`):** a serializable snapshot of a *paused* run awaiting human approval — `RunStatus {AwaitingApproval, Approved, Rejected, Resumed}` (`:26-37`), a `PendingTool { request_id, name, args }` (`:40-47`), plus session handle, reason, and `remaining_turns` budget (`:50-67`). It has one-way state transitions guarded against double-resume (`approve`/`reject`/`mark_resumed`, `:92-116`), a schema-version guard that rejects newer snapshots (`from_json`, `:129-139`), and persists into `session.extension_data` so a paused approval survives reconnects (`store_into`/`load_from`/`clear`, `:144-165`). It is used by the app agent socket to record and surface HITL tool approvals (`apps.rs:1449-1461`). It is a **durability/observability** aid, not itself an enforcement gate.

### The inspector framework and the extension malware check

**`tool_inspection.rs`** is the inspector framework: the `InspectionResult`/`InspectionAction` types (`:12-31`), the `ToolInspector` trait (`:34-55`), and `ToolInspectionManager` which runs inspectors in registration order and swallows individual failures so one crashing inspector doesn't abort the chain (`:96-117` — note: a failing inspector is skipped, i.e. **fails open**). `apply_inspection_results_to_permissions` (`:181-261`) is the escalation-only merge described under permission modes above. It also has a downcast-based hook to update the permission manager and to run the permission inspector's result processing (`:127-170`).

**Extension malware check (`agents/extension_malware_check.rs`)** is an OSV.dev query (`OsvChecker`, `:9-42`) run only when launching a **Stdio** MCP extension (`extension_manager.rs:568`). It infers ecosystem from the launcher (`uvx`→PyPI, `npx`→npm; anything else skipped, `:48-56`), parses the first non-flag package token + optional pinned version (`:82-152`), queries OSV, and **denies** the extension launch if any advisory id starts with `MAL-` (`:235-265`). It is deliberately **fail-open** on every error path — network failure, HTTP error, JSON parse error, unknown ecosystem, no version pin (`:213-233`, `:54`). So it blocks only *known-flagged malicious* published packages and never blocks unknown/local/HTTP-transport extensions.

**`.biorouterignore`** (Developer MCP, `biorouter-mcp/src/developer/rmcp_developer.rs`): a gitignore-style matcher built at server start (`build_ignore_patterns`, `:1670-1699`) from a project-local `.biorouterignore` and a global one, defaulting to `**/.env`, `**/.env.*`, `**/secrets.*` when neither exists (`:1692-1696`). `is_ignored` (`:1702-1704`) gates text-editor reads (`:982`), shell command paths (`:1257-1261`), and code analysis (`:1496`), returning a "restricted by .biorouterignore" error. This is a read-side protection scoped to the Developer extension only.

### Where each control sits in the dispatch path — the full gauntlet

For an ordinary CLI/GUI tool call the order is exactly: **(0)** Chat-mode short-circuit; **(1)** `SecurityInspector` (off by default); **(2)** `PermissionInspector` (the real gate); **(3)** `RepetitionInspector` (loop breaker, max 3 identical calls, `agent.rs:355-357`); **(4)** `HookInspector` (user PreToolUse hooks, `agent.rs:359-361`); then merge → approved dispatch / human approval / denied. See the diagram above and `agent.rs:1704-1763`. The PII masker, run_state pause, OSV malware check, and `.biorouterignore` sit on **separate paths** (BRSDK app socket, extension launch, Developer MCP respectively) and are *not* part of this in-loop chain.

## Notable design choices (worth keeping)

- **Escalation-only override merge** (`tool_inspection.rs:253-256`): a lower-priority inspector can never downgrade a higher inspector's Deny/RequireApproval. Combined with the "no verdict → needs_approval" default (`permission_inspector.rs:72-75`), the permission layer is fail-closed by construction.
- **On-device PII detection with checksum validation** (`pii.rs`): Luhn for cards and structural SSN validation keep false positives low without shipping clinical text to a model — the right call for a biomedical tool. The single `scan` seam is a clean upgrade point.
- **OSV `MAL-*` gating at extension install** (`extension_malware_check.rs`) is a genuinely good supply-chain check that most coding agents lack, and correctly de-scoped to *malicious* (not merely *vulnerable*) advisories.
- **Persisted pause state with schema-version guard and idempotent transitions** (`run_state.rs`) makes HITL approvals survive reconnects cleanly.
- **Context-aware false-positive suppression** in the scanner (`scanner.rs:192-219`) and structured finding ids (`SEC-…`) with metrics counters show operational maturity.

## Gaps and weaknesses

These ten items fed the improvement phase. They are what other documents in this
review cite as `guardrails-permissions.md #N`; the numbering below is that scheme and is stable.

1. **The LLM permission judge is dead code.** `check_tool_permissions` has no callers and `detect_read_only_tools` is only reachable through it (`permission_judge.rs`). The live `PermissionInspector` re-implements permission logic without the judge, so `SmartApprove` never consults the model and — because the read-only path is also broken (see #2) — **`SmartApprove` is behaviorally identical to `Approve`**. The whole "smart" tier and its prompt are inert.

2. **`PermissionInspector`'s `readonly_tools`/`regular_tools` sets are always empty.** They are constructed with `HashSet::new()` at `agent.rs:348-351` (comment: "will be populated from extension manager") and there is **no setter and no second construction site** anywhere in the workspace. So the read-only auto-approve short-circuit (`permission_inspector.rs:135-138`) never fires, and the `read_only_hint` annotations that extensions carefully set are ignored by the permission layer. This silently makes every non-user-configured tool require approval in Approve/SmartApprove, which both over-prompts and hides the intended smart behavior.

3. **Security scanning is off by default and, when on, only asks.** `SECURITY_PROMPT_ENABLED` defaults false (`security/mod.rs:35-41`); even enabled, a `curl | bash` never hard-blocks, it prompts (`security/mod.rs:138`). State-of-the-art agents keep an always-on non-bypassable denylist for a few catastrophic patterns; here a user in Auto mode gets *no* command screening at all, because `Auto` allows everything before the (disabled) scanner would matter and the scanner only ever escalates to a prompt Auto mode wouldn't show.

4. **Regex-only command detection is trivially evadable.** The patterns match literal command shapes (`patterns.rs`); `r''m -rf`, `$(printf ...)`, aliases, base64 wrapped differently, `env`-var indirection, or simply a different tool wrapper all slip through. There is no argv parsing, no path canonicalization, no allow/deny-list of binaries. It is a signature scanner presented as a security control.

5. **Pervasive fail-open.** The OSV check fails open on every error and skips unknown ecosystems, HTTP/SSE extensions, and unpinned/local packages (`extension_malware_check.rs:54,213-233`); the inspector manager swallows inspector errors and continues (`tool_inspection.rs:108-116`); the ML classifier returns `None`/0.0 on failure (`scanner.rs:236-239`). Each is individually defensible for availability but collectively means a network blip or a slightly-off input silently disables the guardrail with no user signal.

6. **Guardrails are scoped to BRSDK apps, not the main agent.** PII masking, `Block`, and `run_state` HITL only run on the Agent Drafter app socket (`apps.rs`); the CLI/GUI agent loop has **no** PII stage, no output moderation, and no groundedness/injection-of-tool-*output* handling. `GuardrailStage::{ToolInput, ToolOutput, Output}` are declared (`guardrails/mod.rs:13-26`) but never implemented — tool *results* (the classic injection vector for agents reading web/file content) are never scanned anywhere.

7. **`.biorouterignore` is Developer-MCP-local and read-only.** It lives in `rmcp_developer.rs`, so any other extension (compute, files, a third-party MCP server, shell via a different tool) that reads `.env`/`secrets.*` is unaffected. There is no central secret-redaction boundary. The default patterns (`**/.env`, `**/secrets.*`) also miss `.pem`, `id_rsa`, `.aws/credentials`, etc.

8. **Permission scoping is coarse and hashed on raw args.** `ToolPermissionStore` keys `AlwaysAllow` on `blake3(tool_name + exact-JSON args)` (`permission_store.rs:79-127`), so "always allow `shell`" is not expressible per-directory or per-command-prefix — it's either exact-args reuse or a blanket `PermissionLevel::AlwaysAllow` on the tool name (which whitelists *all* future invocations, including dangerous ones). No notion of "allow reads under this dir but not writes." Note the store is also not obviously the one consulted by the live inspector (`PermissionManager` config is), suggesting another parallel-implementation seam.

9. **No prompt-injection defense on the input side of the main loop.** The scanner examines the last ≤10 user messages only when ML is enabled (`scanner.rs:158-190`); with ML off (the default) conversation context is not scanned at all, and tool-returned content is never treated as untrusted. This is materially weaker than agents that quarantine tool output or use structured, signed tool results.

10. **`unwrap()` on tool_call in the permission store** (`permission_store.rs:81,99,122`) will panic if a `ToolRequest` carries an `Err` tool_call; the inspectors guard with `if let Ok`, but the store does not, so any caller that reaches it with a malformed request crashes rather than denies.

## Related documentation

- [Hooks system](hooks-system.md) — the fourth inspector in this chain; `PreToolUse` hooks and this permission layer overlap and are best read together.
- [Safety and guardrails compared with other agents](../competitive-comparison/safety-and-guardrails.md) — how this gauntlet measures against eight other coding agents.
- [Core agent loop and tool dispatch](core-loop-and-tool-dispatch.md) — where in the loop the inspector chain runs.
- [Permission modes guide](../../../security/permission-modes.md) — the current, living reference for `Auto` / `Approve` / `SmartApprove` / `Chat`.
- [Wave 1 security report](../../agent-loop-campaign/wave-reports/wave-1-security.md) and [wave 2 hooks and permissions](../../agent-loop-campaign/wave-reports/wave-2-hooks-and-permissions.md) — what was built in response to these gaps.
