# Agent Drafter remediation plan for the 100-app test drive findings

> **What this is.** The six-wave engineering plan that maps every finding from the 100-app Agent
> Drafter test drive to a specific code fix, with `file:line` citations, effort estimates, risk notes,
> per-wave gates, and target metrics.
> **Status:** Historical record — **this plan was executed.** Its own front matter said "proposed" and
> "no product code has been changed yet"; that was true on 2026-07-12 and is not true now. Waves 0–6
> were built on branch `feat/apps-sdk-v2` in commits `ae8987a6`, `7527f848`, and `d8cf95cc`, and the
> artifacts exist in the tree (`crates/biorouter-mcp/src/agent_drafter/catalog.rs`,
> `state_initial`/`worker_ui` in `manifest.rs`, `scripts/agent-drafter/app-smoke.mjs`). Read
> [remediation-results.md](remediation-results.md) for what actually shipped.
> **Audience:** developers working on Agent Drafter and the Apps SDK.

**Branch:** `feat/apps-sdk-v2` · **Date:** 2026-07-12
**Inputs:** [audit-findings-register.md](audit-findings-register.md) (22 findings),
[authored-app-verdict-index.md](authored-app-verdict-index.md) (18 apps),
[platform-integration-audit.md](platform-integration-audit.md),
[layout-diversity-audit.md](layout-diversity-audit.md),
[`data/ledger.json`](data/ledger.json), and the per-app rubrics in `app-results/`.

## Identifiers and codes used below

- **Finding numbers (1–24)** index the [audit findings register](audit-findings-register.md), which
  numbers its entries 1–22 in document order. Findings **23 and 24 exist only here** — they were
  discovered while verifying this plan's citations and were never added to the register, so the two
  documents disagree by design. See the note under item 0.4.
- **Wave numbers (0–5)** are dependency-ordered work stages; each wave's gate is the precondition for
  the next wave's fixes to be checkable. The implementation later added a Wave 6, described only in
  [remediation-results.md](remediation-results.md).
- **Item numbers (`0.1`, `3.4`, `5.2`)** identify a single fix within a wave and are the stable
  cross-reference target used by the other documents in this folder.
- **Effort codes** are relative size estimates on the scale `XS` < `S` < `M` < `L`, used alongside the
  explicit day counts in each wave heading; the day counts are the concrete figure.

> **Warning.** The `crates/.../file.rs:LINE` citations throughout were read against the branch
> worktree on 2026-07-12 and **no commit SHA is pinned** for them. The remediation commits listed
> above have since moved this code, so treat every line number as an archival pointer, not a current
> address.

## Contents

- [Identifiers and codes used below](#identifiers-and-codes-used-below)
- [The one-sentence thesis](#the-one-sentence-thesis)
- [Findings to plan map](#findings-to-plan-map)
- [How the waves are ordered](#how-the-waves-are-ordered)
- [Wave 0 — ground truth (S, ~1 day)](#wave-0--ground-truth-s-1-day)
- [Wave 1 — the environment becomes knowable (M/L, ~3 days)](#wave-1--the-environment-becomes-knowable-ml-3-days)
- [Wave 2 — the contract is declarable in one shot (M, ~2 days)](#wave-2--the-contract-is-declarable-in-one-shot-m-2-days)
- [Wave 3 — the contract is enforced at runtime (L, ~6 days)](#wave-3--the-contract-is-enforced-at-runtime-l-6-days)
- [Wave 4 — failure becomes visible, never silent (M, ~3 days)](#wave-4--failure-becomes-visible-never-silent-m-3-days)
- [Wave 5 — lint that executes (L, ~4 days)](#wave-5--lint-that-executes-l-4-days)
- [Back-compatibility](#back-compatibility)
- [How the citations were verified](#how-the-citations-were-verified)
- [Acceptance criteria and target metrics](#acceptance-criteria-and-target-metrics)
- [Sequencing summary](#sequencing-summary)

## The one-sentence thesis

> **Agent Drafter reliably builds a correct static shell, and then does not honour the agentic
> contract it just declared — because the contract is enforced only as prose.**

The corpus proves both halves. Across 18 authored apps plus 5 layout probes:

| Check | Result |
|---|---|
| Not-a-chatbot (the original complaint) | **18 ✅ / 0 ❌** |
| Prescribed layout regions present | **11 ✅ / 0 ❌** |
| Declared surface matches registered surface | **25 ✅ / 0 ❌** |
| Client reactivity | 10 ✅ / **4 ❌** |
| Multi-agent orchestration reaches a worker | 9 ✅ / **3 ❌** |
| **Signal round-trip (app → agent)** | **1 ✅ / 11 ❌** |
| **Agent-driven loop (agent → app → agent)** | **0 ✅ / 3 ❌** |
| **Full functional PASS** | **0 / 18** |

Verdicts: 15 PARTIAL, 7 "PASS (static; browser pending)", 2 FAIL. Zero apps passed functionally.

The shape of that table is the finding. Anything the platform *checks* — surface declaration, region
markup, lint rules — the model gets right, first try, every time. Anything the platform merely *asks
for in a system prompt* — "call `ui_subscribe` before the user clicks", "use `consult`, not
`subagent`", "read state from `br.state`", "call the action before you narrate it" — the model gets
wrong, repeatedly, in exactly the ways the prompt warned against.

So the remediation is not "write better instructions". Every fix below moves one clause of the
contract out of the system prompt and into one of four enforcement points:

1. **A JSON Schema the model is handed** (it cannot emit a shape the schema forbids).
2. **A tool that is absent from the tool list** (it cannot call what it cannot see).
3. **A server-side check that fails closed** (an invalid manifest cannot be saved or built).
4. **A check that executes the app** (a control that delivers no turn fails the build).

Where prose survives at all, it is *generated from the manifest* (for example the profile-key list),
never authored by the model.

## Findings to plan map

| # | Finding | Sev | Wave | Plan item |
|---|---|---|---|---|
| 1 | Corpus cannot test layout diversity | high | — | mitigated by controlled probes; no product change |
| 2 | Store ignores `BIOROUTER_PATH_ROOT` | high | **0** | [0.1](#01-one-env-aware-path-resolver-for-biorouter-mcp) |
| 3 | Orchestration config requires schema guessing | med | **2** | [2.1](#21-typed-declaration-tools), [2.2](#22-merge-dont-replace-on-manifestjson) |
| 4 | Invented `br.kb` identifier | high | **1** | [1.2](#12-reject-unknown-ids-at-the-write-boundary) |
| 5 | Multi-agent uses display names; workers seize UI | high | **3** | [3.4](#34-consult-binds-to-manifest-keys-workers-lose-the-ui) |
| 6 | Signal emitted before the agent subscribes | high | **3** | [3.1](#31-declaration-is-subscription-eager-signals) |
| 7 | `br.run` controls deliver no turn | high | **4** | [4.3](#43-run-fails-loudly-and-cannot-wedge) |
| 8 | Model endlessly retries declined tools | high | **4** | [4.1](#41-turn-guard-mask-the-tool-terminate-the-loop) |
| 9 | Progress duplicated into the result region | med | **4** | [4.4](#44-one-progress-sink-result-regions-are-sacred) |
| 10 | Configures nonexistent skills | high | **1** | [1.2](#12-reject-unknown-ids-at-the-write-boundary), [1.3](#13-never-arm-a-tool-for-a-grant-that-cannot-be-satisfied) |
| 11 | Platform strings mistaken for capabilities | high | **1** | [1.1](#11-a-catalog-and-a-discovery-tool), [1.4](#14-a-typed-slot-for-i-need-x-and-it-isnt-here) |
| 12 | Agent state and client state diverge | high | **3** | [3.2](#32-one-canonical-state-document) |
| 13 | First-load bindings blank; range ignores keyboard | high | **3** | [3.2](#32-one-canonical-state-document) |
| 14 | Main bypasses profiles with generic `subagent` | high | **3** | [3.5](#35-one-delegation-mechanism-per-app) |
| 15 | `ui_theme` makes regions illegible | med | **4** | [4.5](#45-ui_theme-becomes-a-round-trip-with-a-contrast-audit) |
| 16 | Agent narrates a plan without calling the action | high | **3** | [3.3](#33-an-action-has-an-effect-and-the-turn-knows-whether-it-ran) |
| 17 | Main fabricates quantitative output | high | **3** | [3.6](#36-evidence-ledger-and-provenance) |
| 18 | Drag-only interaction, no fallback | high | **5** | [5.2](#52-brdnd--a-drag-primitive-that-is-reliable-by-construction) |
| 19 | Reviewer misread omitted default theme | high | **0** | [0.2](#02-read_app-returns-a-resolved-view) — harness bug, but the product caused it |
| 20 | Consults burn 120 s then main silently completes | high | **4** | [4.2](#42-consult-deadlines-that-cancel-and-are-visible) |
| 21 | UCSF Azure egress IP | blocker | — | resolved; environment only |
| 22 | CLI exits zero on a provider 403 | high | **0** | [0.3](#03-a-failed-turn-is-a-failed-turn) |
| 23 | **NEW** — NUL byte in `sdk.ts` defeats grep/git-diff | high | **0** | [0.4](#04-the-nul-byte-that-hides-the-sdk-from-every-tool) |
| 24 | **NEW** — a signal emitted before the socket opens is dropped client-side | high | **3** | [3.1](#31-declaration-is-subscription-eager-signals) |

Findings 23 and 24 were discovered while verifying the citations for this plan; they are not in the
[audit findings register](audit-findings-register.md). Both are load-bearing — see the note under
item 0.4.

## How the waves are ordered

Waves are ordered by dependency, not by severity: each wave's gate is the precondition for the next
wave's fixes to be *checkable*. Wave 0 is a day; Waves 1–5 are roughly 3, 2, 6, 3 and 4 days of
focused work (≈ 3.5 weeks, less with parallel agents — the waves are internally parallel).

## Wave 0 — ground truth (S, ~1 day)

Nothing else can be trusted until the sandbox is real, the manifest readback is honest, and a failed
turn reports as failed.

### 0.1 One env-aware path resolver for `biorouter-mcp`

Three hand-rolled resolvers bypass `BIOROUTER_PATH_ROOT`, so the "isolated" test drive wrote apps
into the user's **global** store and read the user's **global** knowledge bases:

- `crates/biorouter-mcp/src/agent_drafter/mod.rs:659` — `default_root()` → `choose_app_strategy(...).in_config_dir("agent_drafter")`
- `crates/biorouter-mcp/src/agent_drafter/mod.rs:876` — `skills_root_for_export()`
- `crates/biorouter-mcp/src/knowledge/paths.rs:34` — `knowledge_root()`, *and* with a different
  strategy tuple (`io/biorouter/biorouter` vs the tuple defined at `lib.rs:6`)

They exist because `biorouter-mcp` cannot depend on `biorouter` (circular), where the authoritative
`Paths::get_dir` lives (`crates/biorouter/src/config/paths.rs:7`).

**Fix:** new `crates/biorouter-mcp/src/paths.rs` with a single `config_dir()` that honours
`BIOROUTER_PATH_ROOT`, byte-compatible with `biorouter::config::Paths`; repoint all three callers.
Add a **cross-crate agreement test** in `biorouter` (which sees both crates) asserting
`agent_drafter::default_root() == Paths::in_config_dir("agent_drafter")`, so the next hand-rolled
`choose_app_strategy` call fails CI.

*Effort S · risk none when `BIOROUTER_PATH_ROOT` is unset (all ~110 v1 apps unaffected).*
*Blocks: the entire catalog work in Wave 1 — a catalog that enumerates the global store cannot verify a sandbox.*

### 0.2 `read_app` returns a *resolved* view

`ThemeConfig::is_default()` (`manifest.rs:762`) + `skip_serializing_if` (`store.rs:175`) mean an
explicitly-chosen default theme **disappears on save**. `resolved_pack()` (`manifest.rs:766`) exists
and is correct, but nothing on the tool surface calls it: `read_app` just pretty-prints the manifest
(`mod.rs:2079`). The same lossiness hits `surface`, `capabilities`, and every optional `AgentConfig`
field — so the model reads *absence* where the truth is *default*, and is never shown the skeleton of
the fields it must fill in. That is not just the harness's misread; it is half of the schema-guessing
loop (finding 3).

**Fix:** `ReadAppParams.view: "resolved" | "raw"`, defaulting to **resolved** — a canonical,
fully-populated, editable skeleton with every optional block present and a
`_server_managed: ["id","created_at","built_at","sdk_hash","session_id"]` list. `view:"raw"` keeps
today's bytes. `build_app` reports the resolved theme pack in its note.

*Effort S · no on-disk format change.*

### 0.3 A failed turn is a failed turn

`biorouter run` exits **0** when the provider 403s. The bug is not in the CLI: `agent.rs:2020`
downgrades a provider error into an *assistant chat message* and ends the stream normally, so the
CLI's error arm (`session/mod.rs:1094`) is never reached, `--output-format json` reports
`"status":"completed"` (`mod.rs:1122`), and `log_session_completion` records the failed run as a
**success** (`cli.rs:1819`).

**Fix:** introduce `AgentEvent::TurnAborted { code: TurnAbortCode, message }` with
`TurnAbortCode::{ProviderFailure{kind}, ToolLoop{..}, WorkerTimeout{..}}` and yield it alongside the
human-readable message. Add `ProviderError::kind()` so auth/403 is distinguishable from a transient
5xx. CLI maps it to real exit codes (`70` provider failed, `75` auth, `76` tool loop) and to
`status:"failed"`; the server forwards it as a typed SSE/WS frame.

*Effort S–M · this type is shared by [4.1](#41-turn-guard-mask-the-tool-terminate-the-loop) and
[4.2](#42-consult-deadlines-that-cancel-and-are-visible) — **land it first**.*

### 0.4 The NUL byte that hides the SDK from every tool

`templates/sdk.ts:4446` returns `"\0undef"` as `stateStringify`'s undefined sentinel — **a literal NUL
byte at offset 157386**. Consequences, all silent:

- `grep`/`ripgrep` classify the file as **binary** and print `Binary file … matches` instead of the
  matching lines. Any pipeline (`grep … | head`) therefore prints **nothing**, which reads as "the
  symbol does not exist."
- `git diff` shows `Binary files differ` — **every change to the SDK's 6,245 lines is invisible in
  review**.
- This plan's own verification pass initially concluded, wrongly, that half the SDK findings cited
  nonexistent code. A human reviewer would have reached the same conclusion.

The single most-reviewed file in the whole feature is currently unreviewable and unsearchable, on the
branch we are about to merge.

**Fix:** replace the sentinel with a non-NUL marker (a unique object identity, or a Private Use Area
codepoint), and add a CI check that fails on any NUL byte under `crates/**/templates/**`.

*Effort XS. Do it first — it is a precondition for reviewing every other SDK fix in this plan.*

**Wave 0 gate:** a sandboxed run provably writes nothing under `~/.config/biorouter`; `read_app`
shows `theme.pack`; `biorouter run` against a 403 provider exits 75 with `status:"failed"`;
`git diff` on `sdk.ts` renders as text.

## Wave 1 — the environment becomes knowable (M/L, ~3 days)

85 of 100 specs request a knowledge base and 57 request a skill, against a runtime with **zero**
installed KBs and **zero** installed skills. Agent Drafter configured 13 nonexistent KB ids and 7
nonexistent skill lists — including the literal string `br.kb`, which is the *client API namespace*,
not an id. This is not model sloppiness; it is an **epistemic hole**: Agent Drafter's 12 tools
(`mod.rs:1551…2245`) include no way to ask what exists, and `AgentConfig` has no field in which to say
"this app wants ClinVar and it isn't here". The only way to express the need is to invent an id.

Worse, the server then *manufactures* the failure: `routes/apps.rs:804` arms the `skills` extension
whenever the skill list is non-empty — real or not — and `apps.rs:1032` writes a system prompt saying
"You are scoped to ONLY these skills: …". `skills__loadSkill` fails on turn 1 by construction.

### 1.1 A catalog and a discovery tool

New `crates/biorouter-mcp/src/agent_drafter/catalog.rs`:

```rust
pub struct Catalog { knowledge_bases: Vec<KbEntry>, skills: Vec<SkillEntry>,
                     extensions: Vec<ExtEntry>, providers: Vec<ProviderEntry> }
impl Catalog { pub fn discover() -> Self }   // KnowledgeService::list_bases() + skill-dir scan + BUILTIN registry
```

plus a `list_platform_catalog` MCP tool. Expose the real skill scan as `pub fn installed_skills()` in
`crates/biorouter/src/agents/skills_extension.rs:168` (today private) so `biorouter-server` uses the
same source. The MCP-side scan is a deliberate duplicate (the crate cannot see `biorouter`), pinned by
a **cross-crate agreement test**.

### 1.2 Reject unknown ids at the write boundary

`configure_app` (`mod.rs:1751`) accepts any string as `knowledge_base` and any strings as `skills`.
Make it fail closed, with the *available ids in the error message* so the model's next attempt is
grounded rather than another guess:

```text
knowledge_base 'br.kb' is not a KB id: kb-id may only contain a-z, 0-9, and '-'.
`br.kb` is the CLIENT API namespace, never an id.
Installed KBs: (none). Use `requires` to record an unmet KB need.
```

`validate_kb_id` already exists in the same crate (`knowledge/paths.rs:3`). Mirror the rule as a
`lint_app` **Error**, which requires widening the signature to `lint_app_with_catalog(dir, &Catalog)`
— coordinate that change once, with Wave 5.

### 1.3 Never arm a tool for a grant that cannot be satisfied

In `configure_agent` (`routes/apps.rs:804`), intersect `cfg.skills` with `installed_skills()`:

- intersection empty → **do not push the `skills` extension at all** (the tool does not exist, so it
  cannot be called and cannot fail on turn 1);
- replace the allow-list prompt (`apps.rs:1032`) with the *available* skills, and re-frame the
  unavailable ones as domain guidance: "No skill is installed for pathway analysis. Reason from first
  principles; do not attempt to load it."
- same treatment for an invalid/missing KB (`apps.rs:1008` currently swallows the failure into a
  `warn!` and arms `knowledge` anyway).

### 1.4 A typed slot for "I need X and it isn't here"

New additive manifest block (serde defaults ⇒ v1 manifests unchanged):

```rust
pub struct Requirement { kind: KnowledgeBase|Skill|Extension|DataSource,
                         id: String, reason: String, status: Satisfied|Missing }
// AgentConfig { … pub requires: Vec<Requirement> }
```

An unmet requirement is **legal**: it produces a lint *warning* and a runtime banner, not an invented
config that produces a runtime *failure*. This is the load-bearing modelling change — today the
manifest has no vocabulary for "wanted but absent", so the model spends it on a lie.

Close the loop with a `capability_report` frame on socket open (requested / configured / available /
exercised), a `GET /apps/:id/capability-report` route so a harness can assert it without a browser,
and a dismissible degraded-capability strip in `sdk.ts`. `build_app` prints the same triplet so the
*authoring* agent sees the gap in the turn that created it.

**Wave 1 gate:** re-lint the 18 test-drive apps — all 13 invented KB ids and all 7 nonexistent skill
lists are Errors; an app that declares `requires` and no invented id builds clean with a Warn.

## Wave 2 — the contract is declarable in one shot (M, ~2 days)

Agent Drafter's INSTRUCTIONS tell the model to seed `surface` in `create_app` (`mod.rs:1229`) — a
parameter **that does not exist**. `CreateAppParams`/`ConfigureAppParams` expose `capabilities`,
`guardrails`, `orchestration` and `output_type` as opaque `Option<serde_json::Value>`
(`mod.rs:152-165`) and expose *nothing* for `surface` or `theme`. The only path is
`update_app(path="manifest.json", <whole manifest>)`, which:

- hard-fails on `missing field created_at` (`store.rs:139` has no `#[serde(default)]`) — metadata the
  model has no business inventing;
- surfaces raw serde errors ("invalid type: sequence, expected a map") for internally-tagged and
  map-shaped schemas with no shape hint (`mod.rs:695`);
- **writes the model's bytes verbatim** (`mod.rs:1855`), silently destroying `built_at`, `sdk_hash`
  and `session_id`.

Hence the observed 6 rejected manifest mutations per app.

And the surface only exists at all when a *starter* is seeded: `manifest.surface = archetype.surface()`
sits inside `if use_starter` (`mod.rs:1696`), which is false whenever the caller supplies its own
`index.html` — i.e. **every one of the 18 spec apps**. They all began life with `surface: {}` and were
forced into the rewrite loop by a fail-closed lint (`bundle.rs:346`, `:419`).

### 2.1 Typed declaration tools

`declare_surface`, `set_theme`, `declare_profiles`, `set_routes` — each a narrow `JsonSchema` struct,
so the tool schema *is* the documentation and an invalid shape is a schema rejection rather than a
serde error. `set_theme.pack` is an **enum** over `THEME_PACKS` (`manifest.rs:685`).
`declare_profiles` validates each key against `^[a-z0-9_]+$` and **rejects display names** — the same
validator Wave 3 needs for `consult`. Add a typed `surface` param to `create_app` and drop the
`use_starter` gate on the surface assignment, so an app can never reach `build_app` with a live agent
and zero declared actions.

### 2.2 Merge, don't replace, on `manifest.json`

`update_app` on the manifest path loads the on-disk manifest, overwrites only author-owned fields,
force-restores server-owned ones, and re-saves canonical serde output. `created_at`/`updated_at`
become `#[serde(default)]`. Error messages for map-shaped fields carry a one-line canonical example.

**Wave 2 gate:** authoring a spec app declares its full surface in **one** tool call, with **zero**
rejected manifest rewrites (today: 6).

## Wave 3 — the contract is enforced at runtime (L, ~6 days)

This is the wave that turns 0/18 into passes. Every item here replaces a sentence in a system prompt
with a mechanism.

### 3.1 Declaration *is* subscription (eager signals)

Signals round-trip 1✅/11❌ because **no turn ever subscribes automatically**: `control.rs:579` starts
the subscription set empty; the only writer is the `ui_subscribe` tool (`control.rs:3559`);
`validate_signal` fails closed (`control.rs:986`); and `handle_signal` (`apps.rs:2316`) turns the
failure into a `warn` frame and **drops the payload**. The user's first click necessarily precedes the
model's first tool call — this is an ordering problem that no prompt can win, which is exactly why one
probe called `ui_subscribe` five times in a row.

There is a **second, independent drop** on the client that the audit never saw, and it would survive
the server-side fix on its own. `emitSignal` (`sdk.ts:1765`) coalesces on a `setTimeout` and then
fire-and-forgets through `send()` (`sdk.ts:1268`), which **returns `false` if the socket is not
`OPEN`** — with no queue, no retry, and no `await this.connect()`. A signal emitted during page load,
or across a reconnect, never leaves the browser at all. Both ends of the path drop the user's first
gesture, for different reasons.

**Fix — both ends:**

- *Server:* `SignalDecl.eager: bool` (default true). Seed the bridge's eager set from
  `surface.signals` in `AppControlServer::new_with_consult` (`control.rs:2312`, right where the surface
  is already mirrored) and re-seed on `attach()` — so main, workers and reconnects all get it with zero
  `apps.rs` wiring. `validate_signal` checks `subscriptions ∪ eager`. A *declared but unsubscribed*
  signal is enqueued rather than dropped; only *undeclared* signals warn. Delete the "call
  `ui_subscribe`" instruction and replace it with a generated line naming the signals the agent is
  already listening to.
- *Client:* give `send()` a bounded outbound queue that flushes on `open` (cap it, and drop oldest with
  a `ui_error` rather than growing without limit). A `false` return from `send()` must never be a
  silent loss of a user gesture.

*Effort S. This is the single highest-leverage fix in the plan: it converts the worst-performing check
(1/12) into a structural guarantee.*

### 3.2 One canonical state document

Two findings, one root cause: **nothing forces the app's own code onto the shared doc.**
`sdk.ts:1589 call()` ships whatever the author's closure passes, verbatim — so a generated
`const state = { sample_size: 248 }` is the real input to the turn while `ui_patch_state` writes n=784
into a document nobody reads. And `build_call_text` (`apps.rs:1584`) composes the model's message from
`name` + `args` **only**, so the model never sees the contradiction. Meanwhile the doc starts empty
(`sdk.ts:4583`) with no manifest field to initialize it, so every `data-br-bind` KPI is blank until a
paid turn completes — which is *why* authors invent the private object. And there is no two-way
binding anywhere in the SDK, so a bound range snaps back and arrow keys never reach state.

**Fix — three parts:**

- `SurfaceDecl.state_initial: Option<Value>` (validated against `state_schema`), seeded into the
  bridge doc server-side (`apps.rs:2569`) *and* into `this.doc` at SDK construction, so bindings paint
  correctly **before the socket connects** (this is also the export/offline path).
- `data-br-model="/pointer"` — a real two-way binding. The SDK attaches `input`+`change` listeners on
  `input`/`select`/`textarea` that write through `br.state.set`, and pushes state→control (skipping
  the focused element). Keyboard, pointer and programmatic paths converge on one write path; the
  author writes no listener and *cannot* desync. Add `br.state.define(initial, {schema})` as the app's
  one store and rewrite all five starters to use it exclusively.
- `build_call_text` attaches the canonical doc to every typed turn as an `<app-data>` envelope, and
  when a declared action's `params` collide with the doc, appends: *"arg `sample_size`=248 disagrees
  with canonical `/power/n`=784 — use the canonical value."*

### 3.3 An action has an effect, and the turn knows whether it ran

`ActionDecl` (`manifest.rs:625`) carries only `name`/`description`/`params`. The platform cannot tell
"apply an intervention" from "read a value", so it cannot require one — and `ui_patch_state`
(`control.rs:3007`) will happily *simulate* the effect by writing `/params/lion_vision` directly. The
app creates the appearance of agent control; specs 011/013/014 did exactly that.

**Fix:** `ActionDecl.effect: Read|Mutate` (default Read ⇒ back-compat) and
`ActionDecl.writes: Vec<String>` (JSON Pointers).

- **Pointer ownership:** `ui_state`/`ui_patch_state` **refuse** any op at or under a pointer owned by
  a `Mutate` action — *"`/params/lion_vision` is owned by action `apply_intervention`; call `app_call`
  to change it."* The narration-only path becomes impossible: the number on screen can only move
  through the app's real handler.
- **Readback:** `app_call` on a mutate action snapshots before/after and returns the diff —
  *"applied: `/params/lion_vision` 0.68 → 0.52"* — or *"the action returned but did not change any
  pointer it declares it writes."* The model gets ground truth instead of its own claim.
- **Turn gate:** `br.call(..., { expect: "apply_intervention" })` records an expectation; a per-turn
  ledger on `UiBridge` records every `app_call`. If the turn ends unapplied: one bounded corrective
  message, then `done{applied:false, expected:…}` and a standard **"plan not applied"** banner.

### 3.4 `consult` binds to manifest keys; workers lose the UI

Two defects, one of them a **deny-by-default inversion**:

- `run_consult` (`apps.rs:1479`) does an exact map lookup, so `"Prosecutor"` ≠ `"prosecutor"` is a hard
  error — and `control.rs:3621 consult()` does no validation at all (it does not even know the profile
  names), so the model gets no early signal.
- `UiCapability::enabled` defaults **true** (`manifest.rs:109`), and a worker profile is a full
  `AgentConfig`. A profile authored without a `ui` block therefore deserializes with `ui.enabled = true`,
  `validate_profiles` ANDs `true && true` (`apps.rs:1202`), and the worker is handed `appcontrol` on the
  **main bridge** plus the full `ui_system_prompt` whose rule #1 is "drive the page". The worker is
  *instructed* to seize the UI. Telling it not to in prose, while giving it the tools, is the failure.

**Fix:** validate `agent` against the declared profile keys inside `consult`, *before* parking, with
the key list in the error; resolve case/separator-insensitively with a unique-match requirement.
Generate the orchestration prompt from the validated keys (the author never writes the names ⇒ they
cannot drift). Add `UiCapability.worker_ui: bool` defaulting **false**, so no worker gets `ui_*` tools
unless explicitly opted in.

### 3.5 One delegation mechanism per app

Both delegation paths are armed simultaneously, and the generic one is easier to reach:
`orchestration.sub_agents` are registered as subworkflows (`apps.rs:958`) and the engine pushes
`create_subagent_tool` whenever `subagents_enabled()` (`agent.rs:1201`) — a gate with **no per-app
switch**. `spec-006-ward-board.log` shows Agent Drafter declaring the same four workers *twice*
(`sub_agents` **and** `agents`); the model picked the tool with the free-form `instructions` field, and
the declared profiles were dead configuration.

**Fix:** if `orchestration.agents` is non-empty, do not register `sub_agents` as subworkflows
(auto-migrate them into profiles instead). Add `Agent::set_subagent_tool_enabled(false)` so the tool is
**absent from the tool list** — the model cannot call what it cannot see — plus a structured refusal in
`dispatch_tool_call` for a stale tool name. Lint errors on a manifest declaring both.

### 3.6 Evidence ledger and provenance

The worst finding in the corpus: after Fine Mapper reported the data was insufficient, main **invented
five PIPs** and rendered them as a credible set. The platform has no representation of "the evidence is
missing" — `consult` returns free prose (`control.rs:3675`), and `app_call` validates args
**shape-only**, so five floats that sum to 1.0 satisfy any schema an author would write. The model read
the refusal and proceeded anyway: the strongest possible evidence that prose cannot fix this.

**Fix:**

- a required `report` tool for workers: `{status: ok|insufficient_data|error, missing: [...], findings, values?}`,
  recorded in the same per-turn ledger as [3.3](#33-an-action-has-an-effect-and-the-turn-knows-whether-it-ran);
- `ActionDecl.requires_evidence: Vec<String>` + `provenance_required: bool`;
- `app_call` **refuses** a non-synthetic call whose required inputs intersect an `insufficient_data`
  verdict, and steers: *"…either call with `source:\"synthetic\"` (it will be labelled DEMO on the page)
  or render the insufficient-data state."*
- provenance is *carried*: synthetic values land with a `_provenance` sibling; the SDK stamps
  `data-br-provenance="synthetic"` and `theme.css` gives them a hatched **DEMO** badge. The fabricated
  PIPs would still appear — clearly labelled — instead of masquerading as science.
- lint warns on an action publishing statistic-shaped fields (`pip`, `p_value`, `hr`, `ci_*`, `beta`)
  with no declared evidence source.

**Wave 3 gate:** signal round-trip 12/12; the agent-driven loop (agent → app → agent) passes on
specs 011/013/014; a scripted worker reporting `insufficient_data` blocks the downstream action.

## Wave 4 — failure becomes visible, never silent (M, ~3 days)

### 4.1 Turn guard: mask the tool, terminate the loop

The runaway `ui_describe` loop was **caused by BioRouter's own guard**. `RepetitionInspector`
(`tool_monitor.rs:139`) denies a call after `max_repetitions`; `handle_denied_tools` (`agent.rs:747`)
answers with *"The user has declined to run this tool"* (`tool_execution.rs:38`) — **a lie; the user
declined nothing** — and **the loop simply continues**. Nothing removes the tool from the next provider
call. Two further defects fall out: the inspector only tracks the *immediately preceding* call
(`tool_monitor.rs:126`), so `A,B,A,B` never trips at all; and the denial text is indistinguishable from
a human decline, so the model cannot learn "this is a loop guard, do something else."

**Fix:** a turn-scoped `TurnToolGuard`. A denied tool is **removed from the tool list** for the rest of
the turn (`filter_tools` before `stream_response_from_provider`, `agent.rs:1603`) — enforcement, not
advice. Repetition becomes signature-keyed (`name` + hash of canonical args) over the whole turn, not
consecutive-only. A second call of an already-disabled signature is **terminal**: `exit_chat`, skip the
Stop-hook restart branch, and yield `TurnAborted{ ToolLoop }` (the type from
[0.3](#03-a-failed-turn-is-a-failed-turn)). A distinct `LOOP_BLOCKED_RESPONSE` replaces the
misattributed decline text.

*Today a runaway loop costs up to 100 billed provider calls; after this, ≤5.*

### 4.2 `consult` deadlines that cancel, and are visible

"Both workers timed out at 120 s and main silently completed" is **guaranteed** by the current code, not
bad luck. There are two racing timers and a serialized handler:

- `control.rs:3654` starts a 120 s timer *before* the request reaches the socket loop; `apps.rs:1516`
  starts a second one strictly later, so the inner one is dead code.
- `run_consult` is awaited **inline inside the `select!` loop** (`apps.rs:3222`). While worker A runs,
  the loop drains nothing — no agent events, no UI frames, dead air on the page. When the outer timer
  fires at T+120, the loop is still awaiting A until T+240, so main's *second* consult only starts then
  and "times out" too. One slow worker mechanically produces two timeouts.
- Nothing cancels the abandoned worker; when it finally answers, `resolve_consult` finds no pending
  entry and **discards it**. Paid work, thrown away.
- `CONSULT_TIMEOUT_S` (`control.rs:60`) is a compile-time constant with **no configuration path at all**.

**Fix:** one timer, owned by the loop side, which **cancels** the worker's `CancellationToken` on
expiry; `run_consult` moved out of the `select!` body into a `FuturesUnordered` branch so consults are
non-blocking and bounded-concurrent; `consult_timeout_s` per profile (clamped 5..=600, default lowered
to 60) plus an env override; a **structured** timeout result (`is_error: true`,
`{status, elapsed_s, partial, phase}`) so the model cannot treat it as an answer; worker-startup
diagnostics surfaced instead of `warn!`-logged; and a `done{degraded:true, missing_profiles:[…]}` frame
that renders a persistent banner whether or not the model mentions it.

### 4.3 `run()` fails loudly and cannot wedge

A generated control "executes locally but delivers no turn" through two silent paths in `sdk.ts`:

- **an unbounded global run queue** (`sdk.ts:1447`) — a turn that never emits `done` (a blocked
  `ui_ask`, a 120 s consult, a dropped socket, the [4.1](#41-turn-guard-mask-the-tool-terminate-the-loop)
  loop) leaves `runChain` pending **forever**, so every later `br.run` sits in the queue and never even
  reaches its target lookup or its "Starting agent run…" paint. The control looks clickable, the
  handler completes, nothing is sent.
- **a missing target throws before any feedback** (`sdk.ts:1459`), rejecting a promise no generated
  click handler awaits.

Neither emits a `ui_error` frame, so the session contains no record that a control fired and delivered
nothing — which is why static lint could not see it.

**Fix:** resolve the target and paint synchronously on click (miss → visible error card + `ui_error`
frame); paint "Queued — waiting for the current agent run…" so a queued run is *visible*; a watchdog
(default 180 s, plus force-settle on socket close) rejects a stalled head with `run-stalled` and drains
the chain; `prompt()` returns a **delivery receipt** so a closed socket says "not connected" instead of
hanging. Plus a static lint rule: a `br.run` target that is a string literal must exist in `index.html`.

### 4.4 One progress sink; result regions are sacred

`doRun` **always** mounts a timeline inside the run target (`sdk.ts:1462-1465`), so
`br.run(prompt, "#synthesis")` renders tool frames into the semantic region *by construction* — and if
the app also mounted a timeline at `#progress`, the same events render twice. `CallOpts` has no
progress channel at all. There is currently **no way to say "progress here, result there."**

**Fix:** a client-side progress-sink registry; `doRun` streams tool frames to an existing sink and
renders the target with the answer only; `progress?: HTMLElement | string | false` on
`PromptOptions`/`CallOpts`; region roles (`result|progress|inspector`) so the server refuses to route
tool frames into a result region; lint errors on two timeline consumers or on a `run` target that is a
declared result region.

### 4.5 `ui_theme` becomes a round-trip with a contrast audit

`ui_theme` is fire-and-forget (`control.rs:2743`): it emits the frame and reports success **no matter
what it did to the page**. Client-side it flips tokens on `documentElement` only, while the app's own
hardcoded `background:#fff` and hardcoded-hex SVG axes do not move — producing the reported black
blocks on a white page. There are also **no plot tokens at all** in `theme.css`, so every authored SVG
label is invisible after a dark pack lands.

**Fix:** make `ui_theme` park for a `ui_theme_result` frame carrying the client's **WCAG contrast
audit**; on failure the client has already reverted and the **tool returns an error** naming the
offending regions — so the agent sees it inside its own turn. Add a `--br-plot-*` token layer with sane
`svg text`/axis defaults for every pack. Escalate the existing hardcoded-color lint to an Error for
`background`/`fill`/`stroke` inside a region **when `allow_theme` is on**.

**Wave 4 gate:** a scripted looping agent is bounded at ≤5 provider calls and ends in `TurnAborted`;
a wedged turn paints a visible failure instead of a dead control; no turn ends `done` while a required
profile silently produced nothing.

## Wave 5 — lint that executes (L, ~4 days)

Every remaining finding shares one property: **no string analysis can catch it.** A dead control, a
blank first-load binding, a keyboard-unresponsive slider, a drag-only surface, an illegible theme —
all of them are *correct-looking code with a runtime failure*. The acceptance gate has to observe a
frame on the wire.

### 5.1 `app-smoke.mjs` — the executing preview

New `scripts/agent-drafter/app-smoke.mjs` + a `smoke_app` MCP tool that `build_app` invokes when a JS
runtime is present. It reuses the mock daemon already in
`scripts/agent-drafter/ui-control-harness.mjs` (real wire protocol, deterministic) and opens the built
app in headless Chromium (Playwright already resolves from `ui/desktop/node_modules`, see
`preview-runtime-test.mjs:31`), degrading to jsdom when no browser is available. Assertions:

- **every wired control** (`button`, `[role=button]`, `input[type=range]`, `select`, `[data-br-action]`)
  is clicked; the mock daemon must receive a `prompt` or `call` frame within 2 s → *"control 'X' fired
  but delivered no turn"* (finding 7);
- **zero-turn load**: every `[data-br-bind]` has non-empty text and no binding resolves `undefined`
  (finding 13);
- **keyboard**: focus each range/select/checkbox and send real CDP key events; the control value **and**
  `br.state.get(pointer)` must both change (finding 13 — jsdom cannot do this; it needs Chromium);
- **drag**: both a coordinate drag and the keyboard path must register and emit a signal (finding 18);
- **progress isolation**: `.br-run-step` nodes appear in exactly one DOM subtree (finding 9);
- **theme**: drive all six packs and re-run the contrast audit from the test side (finding 15);
- **state conformance**: push a sentinel into each declared state pointer, drive each declared action,
  and assert the resulting `call`/`app_result` frames carry the **sentinel** — the only check that can
  catch a `const state = {…}` captured in a closure (finding 12).

Findings fold into `BuildReport`; `BIOROUTER_APP_SMOKE=off` is the escape hatch, and smoke is
**advisory for v1 apps, gating for newly authored ones**, so no existing app is bricked.

### 5.2 `br.dnd` — a drag primitive that is reliable by construction

`sdk.ts` contains **zero** drag support, while `theme.css:383-390` ships `.br-dropzone` and "Draggable
list items" styling — starter gravity pointing straight at hand-rolled HTML5 DnD, which synthetic
pointer moves cannot drive. Lint has no drag rule at all.

**Fix:** `br.dnd.catalog({source, target, onDrop})` built on **pointer events** (so real and synthetic
drags both work), with **click parity** (click to pick up, click to drop) and **keyboard parity**
(`Enter`/`Space`/arrows/`Escape`, ARIA roles, live-region announcements). The primitive **emits the
declared signal itself**, so the app→agent path cannot be forgotten. Add a `catalog` widget node so the
agent can build one via `ui_render`; restyle `theme.css` to the new markup; lint **Errors** on a
drag-only surface, naming the primitive as the fix. A lint error with no working alternative would just
make the model hand-roll something worse — the primitive is what makes the rule fair.

**Wave 5 gate:** `build_app` fails a deliberately-broken fixture on each of: a dead control, a blank
binding, a drag-only surface, a duplicated progress stream, an illegible theme, and a closure-captured
state object. The five starters pass all of them.

## Back-compatibility

~110 v1 apps in the wild plus the 18 test-drive apps. The rules:

- **Every new manifest field is `Option`/`Vec` with a serde default** (`state_initial`, `requires`,
  `effect`, `writes`, `eager`, `worker_ui`, `consult_timeout_s`, `requires_evidence`), so v1 manifests
  deserialize and re-serialize byte-identically.
- **Every new lint Error is Warn for v1**, gated on an SDK-v2 marker (a non-empty `surface`, a v2
  `sdk_hash`, or `actions.register` present in `main.ts`). A pure `br.run` chat app from v1 keeps
  building.
- **Escape hatches** for one release: `BIOROUTER_APPS_CATALOG_STRICT=0` (a KB that exists on the user's
  machine but not on a CI box must not hard-fail an export) and `BIOROUTER_APP_SMOKE=off`.
- **Three intentional behaviour changes**, to call out in release notes:
  1. an app naming an uninstalled skill loses the (already-failing) `skills` extension;
  2. a worker that relied on driving the UI loses it unless it opts into `worker_ui`;
  3. an app with `orchestration.agents` loses the generic `subagent` tool.
- **One migration:** `knowledge_root()`'s strategy tuple changes (`io/biorouter` → the tuple defined at `lib.rs:6`),
  which is identical on XDG and differs only on Windows. Ship a one-time path check.

## How the citations were verified

Every `file:line` above was re-read against the source in this worktree, not taken from the findings
log. Two claims changed under verification:

- The reporter's guess for finding 7 ("app-specific interaction between `br.run` serialization and the
  timeline mount") named the wrong mechanism; the real cause is the unbounded `runChain` promise
  (`sdk.ts:1447`) plus a pre-paint `throw` (`sdk.ts:1459`).
- The reporter's guess for finding 8 ("the engine neither masks nor terminates repeated calls") is
  right, but the decline the model saw came from **BioRouter's own loop guard**, mislabelled as a user
  decline — which is worse than reported.

And a caution for anyone re-checking this: until
[0.4](#04-the-nul-byte-that-hides-the-sdk-from-every-tool) lands, **`grep` on `sdk.ts` silently returns
nothing** (NUL byte → treated as binary). Use `grep -a`. Assuming grep was telling the truth is what
nearly got finding 7's citations wrongly dismissed as fabricated during this very review.

## Acceptance criteria and target metrics

The plan is only real if the same harness that produced the findings re-runs and the numbers move.

1. **Unit + route tests** as specified per item (all listed inline above; ~40 new tests).
2. **Executing checks**: `scripts/agent-drafter/ui-control-harness.mjs` (jsdom) gains scenarios for
   eager signals, `data-br-model`, the run watchdog, progress isolation, `br.dnd`, and the degraded
   banner; `app-smoke.mjs` (Chromium) owns the keyboard, drag, contrast and state-conformance tiers.
3. **Regression corpus**: run the new `lint_app` + `smoke_app` over the 18 test-drive apps as a
   golden-file test. Every invented KB/skill, every dead control, every drag-only surface must be caught.
4. **Re-run the test drive** (`scripts/agent-drafter-testdrive/run.py`, specs 001–025) against the fixed
   build and re-audit. The target is not "18 apps built" — it already does that. The target is:

   | Check | Today | Target |
   |---|---|---|
   | Signal round-trip | 1 / 12 | **12 / 12** |
   | Agent-driven loop | 0 / 3 | **3 / 3** |
   | Client reactivity | 10 / 14 | **14 / 14** |
   | Multi-agent reaches a worker | 9 / 12 | **12 / 12** |
   | Rejected manifest mutations per app | ~6 | **0** |
   | Full functional PASS | **0 / 18** | **≥ 15 / 18** |

## Sequencing summary

```text
Wave 0  Ground truth ......... NUL byte · paths · resolved read_app · TurnAborted+rc     (1d)
   │
Wave 1  Knowable environment . Catalog · reject unknown ids · never arm a dead grant     (3d)
   │      · requires{} · capability_report
   │
Wave 2  One-shot declaration . declare_surface/set_theme/declare_profiles/set_routes     (2d)
   │      · merge-on-write · surface from round 0
   │
Wave 3  Enforced contract .... eager signals · canonical state · action effect+readback  (6d)
   │      · consult by key · worker UI off · one delegation path · evidence ledger
   │
Wave 4  Visible failure ...... turn guard · consult deadlines · run() watchdog           (3d)
   │      · progress sink · theme audit
   │
Wave 5  Executing lint ....... app-smoke.mjs + smoke_app · br.dnd · plot tokens          (4d)
```

Waves 1 and 2 are internally parallel and can run as two agents; Wave 3's six items are parallel once
the ledger type exists; Wave 5's smoke runner is the long pole and can start during Wave 4 (it is
built once and consumed by five findings).

The single cheapest, highest-leverage item in the whole plan is
**[3.1 (eager signals)](#31-declaration-is-subscription-eager-signals)** — one `bool`, one seed call,
and the worst-performing check in the corpus (1/12) becomes structural.

## Related documentation

- [Remediation results](remediation-results.md) — what was actually built from this plan, including a
  Wave 6 that the plan does not contain.
- [Audit findings register](audit-findings-register.md) — the 22 numbered findings the map table above
  cites.
- [Test drive README](README.md) — the index for the campaign as a whole.
- [Apps SDK v2 design](../../apps-sdk/v2-design.md) — the contract these waves move from prose into
  the platform.
- [Agent Drafter apps platform design](../../agent-drafter/apps-platform-design.md) — the subsystem
  every `file:line` citation above points into.
