# Privacy tiers

> **What this is.** The design for a privacy-tier system that keeps conversations touched by
> private models or private data sources from ever reaching a model hosted outside the user's
> institution. It classifies models, sessions, MCP extensions and knowledge bases, and enforces the
> boundary at choke points in the agent loop — the **five** gates A–E designed in §9.1, plus **three**
> more (F, G, H) that implementation found this design had not named, for **eight** in the system that
> shipped. They are enumerated in
> [What shipped, and what did not](#what-shipped-and-what-did-not).
> **Status:** **Implemented for v1** on the `feat/privacy-tiers` branch — not merged to `main` as
> of 2026-08-05 — and **narrowed by operator ruling on 2026-07-30 ([DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store)); read §1 before
> anything else.** ⚠ **Every section below describes the design as it was written, not the running
> system.** [What shipped, and what did not](#what-shipped-and-what-did-not) — immediately after
> this header — is the only place in this document that says which is which, and it names three
> things a reader would otherwise assume from §7 and §9. Read it first. The general filesystem
> read-deny of §9.5 is **descoped for v1** and this document
> no longer claims it. §17 still needs rulings. **§3's R17 (added 2026-08-02) governs the shape of
> every control here, and [§3.1](#31-the-review-checklist--two-questions-every-control-answers-in-writing)
> is the two-question checklist each one is reviewed against** — warn the user and let them proceed, never let an agent proceed at all —
> and **R18 (added the same day) refines it for declassification**: an operating-system
> authentication per operation, which is what lets an agent *ask* for one. ⚠ **§12 is amended in
> three places by R18**; read the banners in §12.1, §12.2 and §12.6 before implementing any of it.
> **Audience:** developers working on the agent loop, `biorouter-server`, the session store, and
> the desktop GUI.

> ⚠ **Every line anchor in this document is historical.**
> They were verified against `main` at `708390d8` and have since moved by roughly **+150
> to +720 lines** — `extension_manager.rs` by +150, `agent.rs` by +719, `session_manager.rs` by
> +200. Do **not** chase a line number from this document; the named **symbol** is the anchor. The
> current positions are tabulated in
> [the execution plan's drift table](privacy-tiers-execution-plan.md#read-this-before-you-chase-a-line-number),
> which was re-verified at `9558c346` and confirmed unchanged at `89c1f026`. Three claims here were
> also false about the tree — §9.3 A1 named a shell-command builder that has never existed, §9.3 B3
> described a global-memory mechanism issue #58 deleted, and §2.3 asserted a uniqueness that a second
> code path contradicts. **All three were corrected in place on 2026-08-01** (execution plan Task 1),
> along with §9.3 B4's ruling, §11.4's missing row, §15.1's migration numbering and §16's counts. The
> anchors added by that pass were verified against the tree on that date; every *other* anchor in this
> document remains historical.
>
> §9.3 B4's forced choice (ratchet knowledge bases, or declare them a public sink) has since been
> **ruled** by the operator: *ratchet*. See the execution plan's
> [Accepted risks](privacy-tiers-execution-plan.md#accepted-risks) for the costs that ruling
> accepts, and its Tasks 10A–10C for the implementation.

---

## What shipped, and what did not

Written at the close of implementation, 2026-08-05. Everything after this section is the design;
this section is the ledger.

### Shipped

- **The two-lattice model (§4).** `ProviderTier` is capability, `SessionClassification` is
  classification, and [`floor`](../../crates/biorouter/src/privacy/mod.rs) is the single crossing
  between them. `crates/biorouter/src/privacy/`.
- **The enforcement gates (§9), eight of them.** A — the bind, in `Agent::update_provider`. B — the
  turn, at the top of `Agent::reply`, repair-first, and the site of the classification ratchet. C —
  the dispatch, in `ExtensionManager::dispatch_tool_call`, plus its resource- and prompt-reading
  siblings. D — `chatrecall`, as a SQL predicate in SEARCH and an explicit check in LOAD. E —
  discovery, in `filter_tools`. F — the two extension channels that are not tool calls. G —
  cross-session conversation ingest. H — the alternate-provider construction sites.
- **The knowledge-base tier (DR-18).** A base takes the tier of the most sensitive session that has
  written to it, the ratchet fires at four write choke points, the barrier refuses at the read ones,
  and a refusal names what it refused rather than returning a silently short answer. The user can
  publicize or privatize a base themselves, graded and audited.
- **Declassification (§12), graded** — a `turn:*` chat keeps its single click; every other
  provenance owes both the typed phrase and R18 / DR-20's operating-system authentication, and one
  predicate decides both so they cannot drift apart. In the desktop app, and as
  `biorouter session declassify <id>` in the CLI, which is the only surface that reaches a private
  chat no listing shows.

  ⚠ **R18's "no cached grant" is honoured on Linux and NOT guaranteed on macOS (F-13).** R18 asks
  for a prompt "raised once per operation (no session, no cached grant)". The Linux prompter
  implements that literally — it declines `pkexec`'s `org.freedesktop.policykit.exec` *because*
  `auth_admin_keep` caches the authorization for minutes, and declares a dedicated `auth_self`
  action instead. macOS reaches the cached outcome by another road: this repo has observed
  `LAContext.evaluatePolicy` returning success **instantly, with no dialog**, off a recent
  authentication (`crates/biorouter-authprompt/src/main.rs:116-122`, written to explain a lifecycle
  bug and only later read as a security property). The helper now sets
  `touchIDAuthenticationAllowableReuseDuration = 0` explicitly, which is already the documented
  default — so if the instant approval survives it, the cause is a system-level grace period that
  `LAContext` cannot override, and the requirement is unmet on macOS rather than merely
  unconfigured. **Untested as of 2026-08-08**; it needs two authentications a minute apart on real
  hardware. Do not describe R18 as fully satisfied on macOS until that runs. See
  `docs/releases/v1.89.0-verification-log.md` § F-13.
- **The master switch** (R7 / DR-15 / DR-22) in Settings → Privacy: one control that disables every
  gate and the ratchet, behind a typed confirmation, stored in its own record beside `config.yaml`
  rather than in it — because a switch an agent can edit with `text_editor` is not a switch. The
  record itself stays agent-writable (DR-17), so an OFF answer is **announced** rather than merely
  obeyed: one `WARN` at load, a standing note above every composer, and *turned off outside the app*
  when no door the app records wrote it — a signal, not a barrier (§10.6).
- **The badges** (§14) on every session, model and extension surface, and the **registry and
  marketplace tiers** (§13).
- **The migration and the day-one notice** (§15) — see
  [what happens to your existing chats](privacy-tiers-migration.md).
- **The non-private-model disclosure** — see
  [data privacy and patient data](data-privacy-and-phi.md#what-a-non-private-model-can-reach). It is
  the shipped mitigation for the first item under *Did not ship*, and it is shown whether or not
  privacy tiers are enabled.
- **An axis this document does not describe: institutional affiliation.** Ruled after this design was
  written ([DR-26](privacy-tiers-execution-plan.md#dr-26--affiliation-is-a-third-axis-and-hipaa-compliance-does-not-transfer-between-institutions),
  plan Phase 6). Tier asks *how sensitive*; affiliation asks *whose*. A UCSF-hosted model reaching
  another institution's private connector passes every gate above, because both endpoints are
  Private — the affiliation axis is what refuses it, or warns and lets the user accept it. Do not
  reason about §9 as though tier were the only axis.
- **The cross-institution mixing policy (DR-27) and its accept control, in all three modes.** The
  setting is `open` / `standard` / `strict`, stored in its own record beside `config.yaml` for the
  master switch's reason, and *loosening* it costs the operating system's authentication while
  tightening it is free. The user-facing half is one control, on the refusal itself: under
  `standard` a press records the acceptance on the in-app proof alone; under `strict` the same
  press additionally makes `POST /agent/cross_affiliation_grant` raise the system authentication,
  and the card says so before the press rather than springing an unannounced dialog. Under `open`
  no mismatch is raised, so there is nothing to accept and no control appears. ⚠ **`strict` is a
  higher price for a yes, never the absence of one** — a build that withheld the control there
  would restore the hard block DR-26 exists to prevent, for exactly the deployments careful enough
  to choose `strict`.
- **§14.3 P4's decoupling — added 2026-09-11, after this ledger was written.** A model switch made
  in a chat changes that chat only; making it the model new chats start on is an explicit,
  unticked "Also use for new chats" box in the switcher. Provider QA F measured the coupling it
  replaces: one chat switched to Claude Code for one check, and the next chat opened came up
  public. The same change (QA finding F3) makes every window's chip follow the app-wide
  selection live, so no window names a private model while its next new chat would bind a public
  one. See [model selection across windows](../desktop-ui/model-selection-across-windows.md).

### Did not ship

- **§9.5's general filesystem read-deny — both of DR-14's layers.** Descoped by
  [DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store);
  plan Tasks 14A–14F are `DEFERRED`, not deleted. A public-capability chat with a shell can still
  read ordinary files on this machine, including files an earlier private chat wrote outside
  Biorouter's own storage. This is disclosed to the user rather than mechanised.
- **§7's cross-session capability matrix: READ and WRITE are both wired now.** ⚠ This entry has
  been wrong twice, and both times in the direction of understating coverage. It first claimed
  that `crates/biorouter/src/privacy/visibility.rs` had **no production caller** and that
  `workspace_list` and `workspace_read_conversation` *"do not consult `privacy_tier`"* — rewritten
  against the tree on 2026-08-06, because every part of it was already false. It then claimed
  WRITE was "wired on none", which was false too: `refuse_unless_writable` has composed the read
  gate with `may_write` since. A security document that understates its coverage costs as much
  trust as one that overstates it, so what follows is the state of the tree, symbol by symbol.

  ⚠ **Re-verify before you rely on this.** This branch is under concurrent repair; between the
  first and second reading of `workspace_extension.rs` while this entry was being written, three
  tools moved from the *open* list to the *wired* one. Treat every claim here as timestamped
  2026-08-06 and check the symbol, not the sentence.

  **Wired — the READ half** (the handlers are all in
  `crates/biorouter/src/agents/workspace_extension.rs`):
  `workspace_read_conversation`, `workspace_open` *on an existing id*, `workspace_send_prompt`,
  `workspace_set_tools`, `workspace_close` and `workspace_watch` (per named id, before it
  subscribes) each call `refuse_unless_visible` before touching the target — a thin delegation to
  `visibility::refuse_unless_readable`, which asks `visibility::may_read` against a metadata-only
  row read; `workspace_list` drops rows with `visibility::appears_in_list`. The refusal
  (`privacy::refusal::workspace_out_of_reach`) answers *private*, *unreadable* and *no such
  conversation* in one sentence, so it cannot be walked as an oracle. Both extension-**enable**
  doors on this surface — `workspace_set_tools { add_extensions }` and
  `workspace_open { new: { extensions } }` — carry Gate F1 through
  `refuse_gated_extension_enable`, resolved from the compiled baseline *before* the installed-set
  lookup so that an uninstalled private extension and an installed one give the same sentence.

  ⚠ **That sentence was true of one of the two doors when it was written.** `refuse_gated_extension_enable`
  was a second, independent spelling of the gate `manage_extensions` already had
  (`check_enable_allowed`) — same three arms, different words, and the **operator pin first**,
  where the other put the tier arm first because issue #56's finding 13 had shown the pin to be
  an install-state oracle. `workspace_set_tools` survived that ordering by accident (it asks with
  no config entry, before the lookup); `workspace_open { new: { extensions } }` looks the entry up
  first and so answered *"…is disabled in the Biorouter configuration (enabled: false)"* to a
  public caller who may not have the connector at all. Closed 2026-08-06 by collapsing the two
  functions into one — `privacy::refusal::extension_enable_refusal` — with a single clause order:
  **tier arm, affiliation arm, then the operator pin**, both privacy arms above the one arm that
  says anything about this machine. `check_enable_allowed` and `refuse_gated_extension_enable` are
  now renderings of that one gate and decide nothing themselves; a source scan in each file fails
  if either grows an arm of its own again. The user's HTTP enable door
  (`POST /agent/add_extension`) keeps its own typed refusal and its own warn-and-proceed posture on
  the other two arms — DR-26's user/agent asymmetry — but asks the same tier predicate,
  `privacy::refusal::tier_refuses`, rather than a fourth hand-written copy of it.

  `visibility::tests::the_matrix_has_production_callers` is the assertion that keeps the predicates
  wired at all — it exists because "the mechanism is built, the entry point is never called" is a
  failure this campaign shipped repeatedly, and no behavioural test can be written against a gate
  that does not exist. ⚠ **It is a source scan of `workspace_extension.rs`, and it does not follow
  the code it guards.** As of this writing the decision body has moved into `visibility.rs`, so
  `may_read(` no longer appears in the production half of the file the scan reads, while
  `appears_in_list(` still does. An anti-regression gate keyed on a file name goes quiet when the
  thing it watches is refactored out of that file, and it cannot tell that case from the
  regression it was written to catch. Whoever finishes that refactor owes the scan a new anchor.

  **Still open:**

  1. ~~**§7's WRITE row is implemented nowhere, so every wired write enforces VIS only.**~~
     **CLOSED, and in both directions at once.** Half of it was closed by ruling rather than by
     code: R6's lineage floor is retired, so "a public caller may steer a public sibling it did
     not spawn" is the intended behaviour and `Lineage`/`lineage_of` are deleted. The other half
     was a real gap and is now built: column D's `✓!` first-crossing approval fires, through
     `agents/workspace_inspector.rs`'s `WorkspaceCrossingInspector` (which asks) and
     `privacy/crossing.rs` (which is the per-(caller, target) state the predicate always needed
     and never had). Widening the write rule is what settled the operator decision that had been
     outstanding against it: the public targets a private caller can reach are no longer only the
     ones it spawned itself, so the disclosure went from nice-to-have to load-bearing.
  2. **`workspace_open { new: … }` implements none of §8.2's spawn matrix, and `new.prompt`
     is now the sharpest edge of it.** A new session is minted PUBLIC (the schema default), so
     a private-capability caller passing `new: { prompt: … }` puts private-origin text in front
     of a public model with no first-crossing approval — the disclosure covers
     `workspace_send_prompt` and `workspace_set_tools`, both of which name an existing
     `session_id`, and a session that does not exist yet has no (caller, target) pair to key on.
     `workspace_open` is not in the delegation tier's injected tool list, so this needs the
     explicit Workspace Control opt-in.

     The older half of this item stands unchanged. §7's last row defers that form to §8.2. The extension dimension is now gated (above), but the **model** dimension
     is not: `open_new_session` creates the session through `WorkspaceServices::start_session`,
     which takes no capability and binds the machine default provider, and then optionally seeds it
     with a detached turn carrying prompt text the model wrote. §8.2's hard refusal (public parent,
     private child) and its approval (private parent, public child) live in `subagent_tool.rs` and
     are reached only by the `subagent` tool. This is the exact route DR-19's own refusal names —
     *"they can start a new chat on it and give it the task directly"* — with the model, rather than
     the user, taking it.
  3. **DR-16's upward capability raise is HTTP-only.** `raise_needs_user_action` is called from
     `routes/agent.rs` and `routes/apps.rs` and nowhere else, while
     `workspace_set_tools { provider, model }` performs the same bind in-process through
     `Agent::update_provider`. Gate A refuses the *downward* bind there, so a private chat cannot be
     moved onto a public model; nothing on that path asks for the user proof DR-16 requires to move
     a chat **up**. `workspace_set_tools` also has no self-target guard — only
     `workspace_send_prompt` refuses `session_id == caller` — so the target may be the caller's own
     conversation.

  What is unchanged is the reasoning about the **instrument**: none of this is fixable with the
  daemon's user-action proof, because a tool call is by definition the model and can never carry
  proof of a human. That is why the wired half uses `may_read` — the caller's capability against
  the target's classification — and it is recorded in full in
  `crates/biorouter-server/src/routes/session_reach.rs`, whose module header carries an overlapping
  still-open list and must be kept in step with this one.

  Items 1–3 are instances of the pattern [§3.1](#31-the-review-checklist--two-questions-every-control-answers-in-writing)
  exists to catch: each is the door a requester reaches for once a neighbouring one is shut.
- **Every open question in §17.** They are open questions, not resolved ones, and several
  (5 — institutional versus hosted Ollama; 9 — skills carry no classification) describe live
  permissiveness in the shipped system.

### What the verification does and does not prove

The gates each carry unit and integration tests, the master switch has its own integration binaries
— `privacy_toggle` in `crates/biorouter/tests/`, `privacy_toggle_config` in
`crates/biorouter-server/tests/` and `privacy_toggle_export` in `crates/biorouter-mcp/tests/`, one
per surface rather than one glob in one crate — and the enforcement points are held in place by
repo-grep assertions that fail when a second call site appears. What this branch does **not** have
is an independent adversarial review: 19% of its review verdicts came from a second model and 81%
were self-review by the implementing model family. Weigh the design's claims accordingly, and see
the execution plan's
[Review provenance](privacy-tiers-execution-plan.md#review-provenance--what-evidence-this-branch-actually-has)
for the measurement.

---

## 1. Summary

> ⚠ **Scope ruling, 2026-07-30 ([DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store) and [DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base)).** This section was rewritten to match what
> is actually being built. **The general filesystem read-deny of §9.5 is descoped for v1.** What is in
> scope: session logs and histories locked against a public-capability model; a public model that can
> neither raise its own tier nor reach the private-only extensions; knowledge bases as first-class
> tiered objects with a user-controlled tier; and a **disclosure** telling users what a non-private
> model can reach. §9.5, §16(8) and §16(9) still describe the wider control and are marked in place —
> they are retained as the specification a revival would start from, not as work.

Two independent lattices, one column, one predicate, five gates — **and, deliberately, no general
filesystem barrier**.

- **Capability** — what a session may *do* — is the **least** privileged model currently bound to
  it. A mixed lead/worker configuration therefore has public reach, because its transcript already
  goes to the public worker.
- **Classification** — how sensitive a session's contents *are* — is the **most** sensitive thing
  it has ever touched, and is a permanent ratchet in SQL.

A public model must never reach a private session — not once, not read-only, not indirectly. The
converse is unrestricted: a private model may read anything.

The five gates sit on *tool calls*, and **that is where this design's guarantee ends.** A
public-capability session also holds tools that run arbitrary commands and read arbitrary paths, and
the private material is ordinary files on disk. §9.5 specifies a sixth control that would close that
channel — a two-layer read-deny over four directories and two files. **It is descoped for v1 by
operator ruling, and this design does not claim it.**

**So state the boundary plainly, because a reader must not infer a stronger one.** A public model
with shell access **can** still read a private session's derived artifacts, and any file a private
session wrote outside BioRouter's own session store. What the gates stop is narrower and is worth
stating positively:

- **The agent-mediated path.** A public model cannot ask BioRouter for another session's content —
  not through `chatrecall` in either mode, not through cross-session conversation ingest, not through
  the BR-71 workspace tools, not through a private knowledge base.
- **The transcript path.** A private session's history is never sent to a public model on a turn, on a
  summary, on an auto-name, or through a copy, diverge or import.
- **The tier-escalation path.** A public session cannot acquire private capability, cannot attach a
  private extension, and cannot see or call `ucsfomopagent` or `cdwagent` — the *"public models cannot
  spin up private models to help them do their work"* requirement.

**Knowledge bases are first-class, not incidental files.** A base carries a tier, takes it at creation
from the model that created it, ratchets on ingest, and a public-capability session may neither read
nor write a private one. The **user** — never a model — moves a base between private and public
([DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base)).

Because the barrier is narrower than the risk, **the risk is disclosed**: a model that is not
HIPAA-compliant, not hosted on-premise and not local can reach what is on the machine, and the
product says so where a user reads it. That disclosure is a shipped requirement (R15), not a caveat —
it is what makes accepting the rest a considered tradeoff rather than an omission.

One **master toggle** turns the enforcement half — gates and ratchet — off for a user who does not
want it (§10.6). It does not turn off the disclosure.

The system is not expressible with what exists today. `provider_class` in
`crates/biorouter-server/src/routes/apps.rs:2089` is the only thing in the tree that resembles it,
and it is **inverted at both ends** (§2.2). Nothing in the session store records a privacy
property, and five distinct code paths can currently attach a public model to a session whose
history came from a private one.

---

## 2. The problem, and why today's code cannot express it

### 2.1 There is no tier anywhere in the backend

| Concept | Today | Verified |
|---|---|---|
| Model tier | Does not exist in Rust. `ProviderMetadata` has exactly eight fields (`name`, `display_name`, `description`, `default_model`, `known_models`, `model_doc_link`, `config_keys`, `allows_unlisted_models`) and no tier. | `crates/biorouter/src/providers/base.rs:145-164` |
| The only tier-shaped list | Lives in the **renderer**: `const INSTITUTIONAL = new Set(['versa_azure','versa_bedrock']); const LOCAL = new Set(['llamacpp','ollama']);` — presentation only, unreachable from the CLI, the daemon or any gate. | `ui/desktop/src/components/settings/providers/providerOrdering.ts:4-5` |
| Session tier | No column. The `sessions` DDL ends `provider_name, model_config_json, diverged_from, external_key, branch_point_msg_uid`. `CURRENT_SCHEMA_VERSION` is 16. | `session_manager.rs:29`, `:1865-1889` |
| Extension tier | No field on `struct Extension` (`config`, `client`, `server_info`, `_temp_dir`, `inprocess`, `_pooled`) and no classification field in the marketplace registry. | `extension_manager.rs:56-72`; `landing/registry.json` (version 1, 37 extensions, 129 skills, keys `id name organization version description tags github download filename license`) |

### 2.2 The one existing approximation is backwards

`provider_class` (`routes/apps.rs:2089`) classifies a provider name into `Local` /
`Institutional` / `External` and gates three real sites (`provider_allowed_for_app`,
`resolve_route`, worker-profile admission). Membership is **exact equality**:

```rust
if p.contains("local") || LOCAL_PROVIDERS.iter().any(|x| p == *x) { return Local; }
if p.contains("institution") || INSTITUTIONAL_PROVIDERS.iter().any(|x| p == *x) { return Institutional; }
External
```

`versa_azure` and `versa_bedrock` match neither list nor either substring, so they fall through to
**`External`** — while `azure`, `azure_openai`, `aws_bedrock`, `bedrock`, `databricks`, `vertex`
are listed **`Institutional`**. Agent Drafter today refuses a Versa route for a sensitive app and
permits a direct Bedrock one. The function's own test table (`:6447`) never exercises a `versa_*`
name, which is why the inversion is green.

### 2.3 Five present-tense paths put private history in front of a public model

Each verified by reading the code, each fixed as a by-product of this design:

1. **`Agent::update_provider` performs no check at all** (`agent.rs:4936-4956`) and swaps the
   in-memory provider *before* it persists. It is nevertheless the **sole writer** of both
   `Agent::provider` and `sessions.provider_name` — a grep for `.provider_name(` outside
   `session_manager.rs` returns exactly one hit, at `agent.rs:4951`. That is what makes a single
   bind gate sufficient.
2. **`chatrecall` LOAD mode has no filter of any kind** (`chatrecall_extension.rs:91-158`). Given
   any session id it calls `get_session(&sid, true)` and emits the session name, working
   directory, message count and six verbatim messages. It does not even carry SEARCH's
   `exclude_session_id` guard. This is **one of two** fully-open cross-session reads in the product
   today. The other is `platform__ingest_conversation`
   (`crates/biorouter/src/agents/knowledge_tool.rs:24-86`), which takes a caller-supplied
   `session_ids` array (`:32-41`), loads each session's full conversation with
   `get_session(sid, true)` (`:49`) and ingests it into a knowledge base — with no lineage,
   ownership or tier check. It is dispatched at `agent.rs:3205`, *before* the extension-manager
   fall-through at `:3339`, so Gate C never sees it; it is not an MCP tool, so Gate E cannot hide
   it; and it never touches `chat_history_search.rs`, so Gate D never sees it. It is advertised
   unconditionally (`agent.rs:3878-3883`, whose own comment reads "The conversation-ingestion tool
   is always available on the platform extension") and its description tells the model outright to
   "Pass `session_ids` to ingest specific (or multiple) sessions instead"
   (`agents/platform_tools.rs:64-65`). Because a knowledge base is a machine-wide tree any session
   may name (§9.3 B4), this is a one-call private→public laundering primitive, and it belongs at or
   above LOAD in §19's order.
3. **Three independent session-copy paths carry the conversation but not the provider.**
   `copy_session` (`session_manager.rs:4138-4168`), `diverge_session` (`:4204-4265` — the primary
   GUI diverge, and it does *not* call `copy_session`) and `import_session` (`:4096-4135`) each
   hand-roll their own update builder. None sets `provider_name` or `model_config`, so a branch of
   a private chat resolves through `restore_provider_from_session`'s `Config::global()` fallback
   (`agent.rs:4963-4978`) and runs private history on the user's default public model, with no
   prompt.
4. **Agent Drafter's `ClientFrame::ModelSelect`** (`routes/apps.rs:3409-3428`) calls
   `create_provider` then `agent.update_provider` with no check whatsoever, over the
   `GET /apps/{id}/agent` WebSocket, which `auth.rs:52-77` exempts from secret-key auth.
5. **`POST /agent/call_tool`** (`routes/agent.rs:1140-1163`) dispatches straight into
   `ExtensionManager::dispatch_tool_call`, skipping `Agent::dispatch_tool_call` and every
   `ToolInspector`. Any control expressed as an inspector is invisible to it.

---

## 3. Settled requirements

| # | Requirement |
|---|---|
| R1 | **Private models** are institutionally hosted (`versa_azure`, `versa_bedrock`) and user self-hosted (`llamacpp`, `ollama`). **Public** is everything hosted by an AI company or a large cloud. |
| R2 | Capability = **least** privileged model in play. Classification = **most** sensitive thing touched. The two are independent. |
| R3 | Classification is a permanent ratchet. |
| R4 | A private session may spawn public children; a public session may never gain private reach. |
| R5 | Children inherit the parent's model and lead/worker mode unless the user says otherwise. |
| R6 | ~~Lineage decides write access: sessions the caller spawned get full control, everything else is read-only.~~ **RETIRED.** Superseded by the product owner's ruling that an agent may inject a prompt into *any* conversation — child or unrelated — provided it does not cross the private/public boundary. WRITE ⇔ VIS; the tier is the only boundary (§7). |
| R7 | A global opt-out exists, off by default. It is a **master** switch: with it off there is no gate and no ratchet anywhere (§10.6). ⚠ It does **not** switch off R15's disclosure — with enforcement off the exposure is larger, not smaller. |
| R8 | A public model must never reach a private session. |
| R9 | Only the user can deprivatise a session, from history settings, with a warning. Nothing automatic, nothing agent-invocable. |
| R10 | Badges are visible everywhere — models, sessions, MCP servers. |
| R11 | The BAAM registry is the **sole** source of extension classification. (i) nothing local can grant private; (ii) **anything not on BAAM is public**. Built-ins are public. |
| R12 | Skills carry no classification. |
| R13 | `chatrecall` obeys the barrier. Side channels (existence, counts, timing) are out of scope; only content must not cross. |
| R14 | The registry is trusted (only the Baranzini Lab can publish) and the classification ships on the landing site and in `registry.json`. |
| **R15** | **Users are told what a non-private model can reach.** A model that is not HIPAA-compliant, not hosted on-premise and not local can read what is on the machine; the product says so, in the GUI, in the CLI and in the docs, from one shared copy. Added by [DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store), which is also what makes its accepted risks acceptable. |
| **R16** | **A knowledge base is a first-class tiered object, and the *user* owns its tier.** It takes a tier at creation from the model that created it, ratchets on ingest, is unreadable and unwritable to a public-capability session when private — and the user, never a model, may publicize or privatize it. Added by [DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base). |
| **R17** | **A warning for the user, a wall for the agent.** For every privacy- or security-sensitive operation in this design: an operation the **user explicitly initiates** is **warned about and then allowed if they insist** — never hard-blocked; the same operation **initiated automatically by an agent** is **never** permitted — it escalates to a human or it does not happen. Added by [DR-19](privacy-tiers-execution-plan.md#dr-19--a-warning-for-the-user-a-wall-for-the-agent). |
| **R18** | **Declassification is gated by a system authentication, and that is what lets an agent ask.** Every declassification — one chat or a batch — is authorised by an **operating-system** prompt of the same class as the Keychain authorization at app start: raised **once per operation** (no session, no cached grant), naming the **exact** set it covers, with the password verified by the OS and **never** seen by BioRouter. Because the gate is the prompt and not the caller, **either the agent or the UI may initiate** — an agent may *ask*, precisely because it cannot *satisfy*. Added by [DR-20](privacy-tiers-execution-plan.md#dr-20--declassification-is-gated-by-a-system-authentication-and-that-is-what-lets-an-agent-ask). |

⚠ **R18 refines R17; it does not repeal it.** R17's agent half — *never automatically* — is intact:
an agent-initiated declassification **is** an escalation to a human, because the request is the
agent's and the effect is the human's. The relaxation reaches **only** operations where an
unforgeable human act stands between the request and the effect, and it is earned per operation
rather than inherited. It does **not** reach the five gates of §9.1, the spawn-downgrade, the
capability raises of DR-16, or any control a task merely *confirms* — a dialog is not a prompt. R18
also retires the two proofs this document assumed and never defined (§12.1's *"one-shot token minted
by the renderer over Electron IPC"*, and the execution plan's `secret_key_and_capability_token()`),
and supersedes §12.6's *"No general bulk declassification"*.

⚠ **R17 is the shape of R9 and of every user-only control here, stated once.** R9 (only the user may
deprivatise a session) and R16 (only the user may move a base's tier) are **instances** of R17, not
separate rules — as are DR-16's user-only tier raise and DR-18's user-only publicize / privatize.
Read R17 first when designing any refusal, confirmation or override, and answer its question — *who
can initiate this?* — for every control. A design that does not say is defective: an unstated
initiator becomes whatever the implementer assumes, and the assumption that ships is the permissive
one. [§3.1](#31-the-review-checklist--two-questions-every-control-answers-in-writing) turns that
question into the two-item checklist a review must answer **in writing**.

Both halves are load-bearing. A control that **walls the user** is one they route around — by
turning the whole feature off (R7's master switch exists because that pressure is real) or by
leaving the product — so it trades a narrow refusal for a machine-wide one and buys nothing. A
control an **agent** can proceed past after a warning is not a control: there is nobody at the
keyboard to decline it, and the model writes the next tool call. §14.6's *"warned rather than
walled"* is R17's user half; §12.2's *"why no agent can invoke it"* is its agent half. **R15 is what
makes the permissive half legitimate** — a user can only accept a risk that was stated to them —
which is why R7's ⚠ holds: turning enforcement off must never turn the disclosure off with it.

⚠ **R17 weakens no gate.** The five gates of §9.1 refuse a **model**; they are its agent half and
stay hard refusals with no override. *"The user could have done this"* is never a reason to let a
model do it.

⚠ **R8 is unchanged in words and narrowed in reach.** *"A public model must never reach a private
session"* remains the invariant every gate is written against. What [DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store) descoped is the
**filesystem** channel to that material, not the rule: a public model may not be *handed* a private
session by BioRouter, and it may still read files on the machine it is running on. §1 states the
boundary; R15 is why that is disclosed rather than implied.

### 3.1 The review checklist — two questions every control answers in writing

Every new control in this design — a gate, a refusal, a confirmation, an override, a filter — is
reviewed against exactly two questions. Neither is answerable with a judgement; both are answered
with an enumeration, in the control's own text, where the next reader finds it instead of
re-deriving it.

> **Q1 (R17 / [DR-19](privacy-tiers-execution-plan.md#dr-19--a-warning-for-the-user-a-wall-for-the-agent)). Who can initiate this?**
> A user, an agent, or both — and what does the other party get instead? Silence is a defect, not a
> gap: an unstated initiator becomes whatever the implementer assumes, and the assumption that
> ships is the permissive one.
>
> **Q2 (added 2026-08-06). What will the user or the model do instead, and is that guarded?**
> A control does not end the requester's attempt; it redirects it. Name every other entry point to
> the same capability, and say for each whether it is guarded, by which named predicate, at which
> call site.

**Q2 exists because one pattern produced three separate leaks in a single audit:** *a refusal that
closes one door pushes the requester through an adjacent open one.* All three were found by
someone else's fix, not by the review of the control that did the refusing.

| The control that refused | Where the requester goes next | Guarded when the audit found it |
|---|---|---|
| DR-19's spawn downgrade (`PrivacyRefusal::PublicChildOfPrivateParent`), whose text ends *"they can start a new chat on it and give it the task directly"* | `workspace_open { new: { prompt: … } }` — the model starts that chat itself and seeds it with a detached turn | **No**, and **still no**: `open_new_session` binds the machine default provider and asks nothing; §8.2 is enforced only in `subagent_tool.rs` |
| Gate F1 (`check_enable_allowed`), refusing `manage_extensions` on a private extension | `workspace_set_tools { add_extensions: … }` and `workspace_open { new: { extensions: … } }`, which reached `ExtensionManager::add_extension` directly | **No.** F1 sat on the `manage_extensions` path alone. Closed 2026-08-06 by `refuse_gated_extension_enable` on both doors |
| The global-memory consent gate (`security/global_memory.rs`), refusing `retrieve_memories(category="*", is_global=true)` | the **project-local** store — itself a cross-session channel, since local memories are inlined in full into every session opened in that directory (§19 item 2) | **No**, by a ruled scope boundary, and the refusal *points at it*: *"Local bulk retrieval (is_global=false) is unaffected."* |

**The third row is why Q2 is worded the way it is.** The weaker question a reviewer reaches for
first — *does this refusal's text advise a forbidden action?* — catches rows 1 and 2 and misses row
3, because that refusal advises nothing: it states a **scope disclaimer**, which is good practice,
accurate, and hands the model the pointer anyway. Nothing in the sentence is advice-shaped for a
text scan to catch, and the requester needs no advice — it does the next obvious thing. Q2 is a
question about the **space of available next actions**, not about the wording of the refusal, and
it must be answered even for a control whose message names no alternative at all.

How an answer is judged:

- **An enumeration, or it is not an answer.** *"Considered"*, *"no other path"* and *"seems
  covered"* are nods. The artifact Q2 produces is a list of sibling entry points with a verdict
  each. *"Nothing else reaches this capability"* is acceptable **only** with the list that
  establishes it — which, for every finding in this campaign, is where the missed path turned up.
- **Both audiences, separately.** The user's next move matters as much as the model's, and they
  differ. A control that walls a **user** is one they route around — by turning the whole feature
  off (R7's master switch exists because that pressure is real) or by leaving the product. A
  control that merely inconveniences a **model** is one it re-attempts through the next tool in its
  list, without malice and usually within the same turn.
- **A guard with no caller is not a guard.** If an answer cites a predicate, it must also cite the
  production call site. This campaign has shipped **nine** correct, tested, entirely uncalled
  guards; §7's own matrix was one of them, and the *"Did not ship"* entry above records both the
  omission and the assertion (`the_matrix_has_production_callers`) that now holds it.
- **"The model would not think of that" is not a verdict.** The model reads the refusal, the tool
  list and the error text, and the next tool call is the cheapest thing in the loop. Treat every
  advertised tool that touches the same data as a live path until it is shown gated.
- **Q1 does not imply Q2.** A control can name its initiator perfectly and still leak: Q1 is about
  who *starts* the operation, Q2 about what happens *after it is refused*. Every one of the three
  rows above answers Q1 correctly.

⚠ **Scope this to the capability, not to the module.** All three leaks crossed a file boundary, and
two crossed a crate boundary. The sibling path is rarely next to the control — it is the other
surface that reaches the same data, which is why the answer is written as a list of entry points
rather than as a claim about the file the control lives in.

**The form the answer takes**, so that a review can be checked for having done it rather than for
having said it. Q2 is answered with this table and nothing shorter; a control whose section does
not carry one has not answered Q2, whatever prose surrounds it.

| Other entry point to the same capability | Reachable by | Guarded by | Where that guard is called |
|---|---|---|---|
| *(one row per surface — tool, HTTP route, CLI command, config key, file the agent can write)* | user / model / both | named predicate, or **nothing** | `file::symbol`, or **no caller** |

A row reading *"nothing / no caller"* is a legitimate answer. It is a **finding**, recorded where
the next reader meets it, which is the whole difference between this campaign's fixed leaks and its
found ones. What is not legitimate is the absence of the row.

### 3.1 Q2 answered for one capability: unloading an extension

Kept here as the worked example, because it is the capability that produced the pattern twice —
once as the leak (finding 14: `manage_extensions {disable}` ran `ExtensionManager::remove_extension`
with no privacy decision in scope, so a chat on a public model could unload the clinical connector
that Gate E will not show it, that Gate C refuses its every call into, and that — since finding 13
— the catalogue will not even name) and once as the leak's own sibling. The first fix gated
`manage_extensions` and named the workspace file out of scope; the workspace fix gated the two
**enable** doors and did not look at the unload one. **One capability, two entrances, and the
requester needed one line of the tool list to walk from the closed one to the open one.**

| Other entry point to the same capability | Reachable by | Guarded by | Where that guard is called |
|---|---|---|---|
| `extensionmanager__manage_extensions {action:"disable"}` | model | `ExtensionManager::assert_extension_manageable` (= `assert_extension_reachable(&normalize(name), Some(admitted))`) | `agents/extension_manager_extension.rs::manage_extensions_impl` |
| `workspace_set_tools {remove_extensions}` | model | the **same** predicate, called by name | `agents/workspace_extension.rs::handle_set_tools`, per name, before `apply_extension_changes` — **closed 2026-08-06; it was the open door** |
| `POST /agent/remove_extension` | user, and any holder of the daemon secret (AR-11) | **nothing** | **no caller.** Deliberately off [the gated list](../../crates/biorouter-server/src/routes/session_reach.rs) — it is a *negative control* in `session_reach.rs::every_gated_route_resolves_the_tier_before_it_touches_the_session`, so gating it is an edit to that list and not a line in the handler. Its sibling `POST /agent/add_extension` **is** gated (`session_reach`), which is the same asymmetry one crate over. **Open, and stated: the residual of [#47](https://github.com/BaranziniLab/biorouter/issues/47)** |
| `GET /agent/tools?session_id=` (the empty-`session_id` branch) | user, through Settings → Extensions → tool permissions | **nothing**, by ruling | `routes/agent.rs::get_tools` removes and re-adds one globally enabled extension on the permission-settings pseudo-session to list its tools. Task 16 keeps this surface unfiltered on purpose — it is the user's own administration view, not the model's context — and the pseudo-session is not a conversation |
| The config path: `biorouter configure` and `biorouter extension remove` → `config::set_extension_enabled` / `config::remove_extension` | user at the keyboard | **nothing**, by ruling | `biorouter-cli/src/commands/configure.rs`, `commands/extension.rs`. These edit the operator's own file and unload nothing from a live agent. #42's pin is *read* out of that file by every gate above; a gate *on* it would be a gate on the user's own config, which R7's master switch already says is theirs |

The enable half of the same capability is enumerated in §3's third row above and is now one
predicate (`privacy::refusal::extension_enable_refusal`) behind three model-facing doors —
`manage_extensions {enable}`, `workspace_set_tools {add_extensions}`,
`workspace_open {new:{extensions}}` — with the user's HTTP door asking the same tier boolean under
its own typed refusal. Two further **load** paths are deliberately ungated and belong in the
enumeration rather than in a fix: `config::resolve_extensions_for_new_session` (a session starts
holding the operator's enabled set; the tier is enforced downstream at Gates E and C, never by
refusing to load) and the app/ACP paths (`routes/apps.rs`, `biorouter-acp/src/server.rs`), which
attach an app's declared extensions to that app's own agent.

The enumeration above is the complete set of production call sites that reach
`ExtensionManager::remove_extension`, whether directly or through `Agent::remove_extension`. Two
further matches for `.remove_extension(` in the tree are **not** this capability and are recorded
so the next enumeration does not have to re-decide them: `PermissionManager::remove_extension`
(`biorouter-cli/src/commands/configure.rs`, `config/permission.rs`) drops a *permission map* entry
and touches no server — the census's standing lesson that a name match is not a call site.

⚠ **Do not add a third spelling of the unload rule.** Both model-facing doors call
`assert_extension_manageable`, which is `assert_extension_reachable` **verbatim** — so discovery
and management answer with one function, and all three of its consequences hold at both doors: an
unknown name reads Private and is refused (which is what stops the refusal being the install-state
oracle finding 13 closed at the catalogue), the name is normalized to the key the executor removes
under, and a model bound to another institution may *see* a mismatched connector but may not unload
it. A hand-written `class.tier.is_private() && cap.tier() == Public` at a third door is how these
two came apart.

---

## 4. The two lattices

They are not two reductions over one set. They are reductions over **different domains**, which is
what makes them provably consistent rather than contradictory.

```
capability(S)     = least over { components of the provider bound to S RIGHT NOW }
                    domain: providers.  Pure function of live state.  NOT STORED.

classification(S) = max   over { floor(capability) at each turn S has ever run,
                                 Private for each private MCP call S made,
                                 the classification inherited at creation }
                    domain: events over time.  Monotone accumulator.  STORED.
```

Capability has no memory. Classification has no reference to live state. The independence is
enforced three ways:

1. **Type level.** Two Rust types that do not interconvert.

   ```rust
   // crates/biorouter/src/privacy/mod.rs

   /// CAPABILITY. Deliberately NO Ord: `max` over this type is always a bug.
   pub enum ProviderTier { Public, Private }
   impl ProviderTier { pub fn least(a: Self, b: Self) -> Self { … } }

   /// CLASSIFICATION. Monotone in time.  Public < Private.
   #[derive(PartialOrd, Ord, …)]
   pub enum Classification { Public, Private }

   /// The ONE crossing. pub(crate); a repo-grep test asserts exactly two callers.
   pub(crate) fn floor(t: ProviderTier) -> Classification { … }
   ```

   There is no `From`/`Into`. A single `Ord`-derived tier would be simpler and would invite
   `max()` where `least()` belongs.

2. **Storage level.** Capability is `Agent::capability_tier()`, a method reading
   `self.provider.lock().await.tier()`. No field, no column, no cache that can go stale.

3. **Scope level.** Of the five gates, exactly one reads both (Gate A/B, the bind). Gate C reads
   capability and `extension.tier`. Gate D reads capability and the *target's* classification,
   never the caller's.

**Invariant, and it should be a property test:** for any sequence of legal binds,
`capability(S) ≥ classification(S)`. The bind admits `P` only if `tier(P) ≥ classification(S)`; the
ratchet then sets `classification := max(old, floor(tier(P))) ≤ floor(tier(P))`. By induction, the
naive failure — "the very next turn feeds private history to a public model" — has no code path.

### 4.1 The mixed configuration, worked through

**Private lead + public worker.** `tier = least(Private, Public) = Public`.

- Capability is **Public**: cannot call a private MCP server, cannot read a private session.
- Classification does **not** ratchet, because `floor(Public) = Public`. The transcript has already
  gone to the public worker, so there is nothing left on that axis to protect — and marking it
  Private would make the bind gate refuse *that same composite* on the next resume, bricking a
  working configuration.

This is the one place the design reads R3 in spirit rather than letter (R3 says "switched to a
private model even once → private permanently", and a composite *contains* a private model). It
needs an explicit ruling — see §17, question 1.

`LeadWorkerProvider::tier()` returning `least(lead, worker)` is the **only** override needed, and
it is the reason `tier()` is an instance method rather than a lookup on `get_name()`:
`get_name()` on a composite returns the **lead's** name (verified,
`providers/lead_worker.rs:332-334`), which would badge a private-lead/public-worker session Private
— the exact inverse of R2.

### 4.2 The residual state

`classification = Private, capability = Public` is representable but unreachable through any legal
bind. It arises only from legacy rows, a pre-fix copy path, an LRU-evicted agent rehydrated with
the process default, or a scheduled job built from global config. **Gate B owns it**, and in that
state both properties hold at once: the session cannot run a turn until repaired or declassified,
*and* it stays invisible to every public reader, because Gate D keys on the target's classification
against the caller's capability, independent of the target's own capability. A corrupted private
session cannot leak while it is broken.

---

## 5. How each thing acquires a tier

### 5.1 Models

One new field and one new method.

```rust
// ProviderMetadata gains a ninth field — Serialize + ToSchema, so it reaches every UI
// surface through `just generate-openapi` → `npm run generate-api`.
pub tier: ProviderTier,

pub trait Provider {
    /// Least-private component of what this instance actually resolved.
    /// DEFAULT = Public: a provider module that forgets to implement it is
    /// least-trusted and can never attach to a private session.
    fn tier(&self) -> ProviderTier { ProviderTier::Public }
}
```

The private set is the list that already exists, moved from the renderer to Rust
(`PRIVATE_PROVIDERS = ["versa_azure", "versa_bedrock", "llamacpp", "ollama"]`). The two frontend
`Set`s are **deleted**; `classifyProvider` switches on the backend field, keeping `PRIORITY_ORDER`
for ordering. One list, one place, drift structurally impossible.

**Unknown ⇒ Public, and that is fail-safe rather than fail-open.** Public is the *less* privileged
tier, so an unrecognised provider gets less reach. This is the opposite fail direction from R11(ii)
and the asymmetry is deliberate: an unknown model is a place data might *go* (restrict it); an
unknown extension is a place data might *come from* (the operator ruled fail-open).

**Only demotions, never promotions.** One demotion rule, closing a verified hazard: `versa_azure`
reads `AZURE_OPENAI_ENDPOINT` / `AZURE_OPENAI_DEPLOYMENT_NAME` / `AZURE_OPENAI_API_VERSION`
(`versa_azure.rs:106-112`) and the public `azure_openai` provider reads the same three keys
(`azure.rs:115-118`). `versa_bedrock` falls back to `AWS_ENDPOINT_URL_BEDROCK_RUNTIME`
(`versa_bedrock.rs:113`), which `bedrock.rs:92` sets **process-globally** with
`std::env::set_var`. So `versa_*` demotes to Public when its resolved endpoint host is not the
compiled-in UCSF gateway (`unified-api.ucsf.edu`), computed at construction when the endpoint is
already resolved.

> **Update (2026-09-11).** `versa_azure` now reads only its own `VERSA_AZURE_ENDPOINT` /
> `_DEPLOYMENT_NAME` / `_API_VERSION`, so the shared-key half of this hazard is closed at the source:
> whatever the public `azure_openai` card is set up with no longer reaches it. The demotion rule is
> unchanged and still needed, because `VERSA_AZURE_ENDPOINT` is user-writable config.
>
> **Update (2026-09-11), Bedrock.** `versa_bedrock` now reads only its own `VERSA_BEDROCK_ENDPOINT` /
> `VERSA_BEDROCK_REGION`, with no fallback to an `AWS_*` key or to the process environment, and
> `bedrock.rs` no longer promotes `AWS_ENDPOINT_URL_BEDROCK` to `AWS_ENDPOINT_URL_BEDROCK_RUNTIME`.
> That closes the fallback named above in both directions. The shared namespace also hid a crossing
> no endpoint check can see: the AWS SDK reads `AWS_BEARER_TOKEN_BEDROCK` from the environment itself
> and authenticated Versa's requests with that token. So an instance that resolved the gateway, and
> was rightly Private, carried the public card's Bedrock API key to UCSF. `versa_bedrock` now chooses
> SigV4 in code. The demotion rule is unchanged and still needed, because `VERSA_BEDROCK_ENDPOINT`
> is user-writable config.

**Never keyed on a model id.** `us.anthropic.claude-opus-4-8` appears in both
`BEDROCK_KNOWN_MODELS` and `VERSA_BEDROCK_KNOWN_MODELS`. Any model-name badge is wrong by
construction.

**Composites.** `providers::create` intercepts *before* the registry lookup when
`BIOROUTER_LEAD_MODEL` is set (`factory.rs:139-149`), so `create("ollama", …)` can hand back a
composite whose lead is `anthropic`. Tier must always be read off the **constructed instance**.

### 5.2 Sessions

Two columns, added on the line after `diverged_from TEXT,` in the fresh-DB DDL:

```sql
privacy_tier   TEXT NOT NULL DEFAULT 'public',
privacy_reason TEXT
```

`privacy_reason` is audit and UX only, never read by a gate: `turn:versa_azure`,
`mcp:ucsfomopagent`, `inherited:<parent_id>`, `diverged:<parent_id>`, `backfill:<provider>`,
`declassified_by_user`.

**The read fails closed, loudly.** The tree's convention for optional columns is
`row.try_get(…).ok().flatten()`, and BR-71's own plan warns that a missed SELECT "compiles and
silently reads `None`". So:

```rust
privacy_tier: row.try_get::<String,_>("privacy_tier")
    .map(|s| Classification::from_str(&s))
    .unwrap_or_else(|_| { error!("privacy_tier missing from projection"); Classification::Private }),
```

Defaulting to Private paints every session with a Private badge — immediately visible, immediately
fixed, and safe meanwhile. Both `Session`-building projections (`get_session`,
`list_sessions_by_types`) must name the column, each with a unit test asserting a known-public row
does not come back defaulted.

### 5.3 MCP extensions

**Not stored anywhere — re-derived per read.** This was originally
`tier: ProviderTier` on `struct Extension`, stamped once at admission in `add_extension`,
`add_client` and `add_inprocess_server`.
[DR-23](privacy-tiers-execution-plan.md#dr-23--an-extensions-tier-is-re-derived-from-the-registry-never-stored-locally)
deleted that field: three call sites had no record to read and re-classified from a bare name
anyway (Gate F1, `/agent/add_extension`, the sub-agent spawn partition), so a stamped copy was one
source of truth among four. Every gate now calls `classify_extension` at the point of decision.

It was on the **record** rather than on `ExtensionConfig`, and those reasons now argue for having no
stored copy at all: `ExtensionConfig` round-trips through user-writable `config.yaml`, which would
make classification locally forgeable and contradict R11(i); and `pool_key` carries no session id,
so one `ucsfomopagent` child process is shared across sessions — the badge could not live on the
process either.

**What the resolver keys on.** The union of the config entry's own name and the BAAM registry `id`
the install recorded in `<config dir>/extension-provenance.json` (`privacy::provenance`). Union
rather than precedence, so a record can only ever *raise* a tier — forging one, corrupting the store
or deleting it leaves the answer at least as restrictive as the config-name join alone. That is why
the store needs no gated writer, and it is the same "raises and never lowers" rule §10.2 states for
registry freshness.

**Finding the record, when the name is exactly what changed.** The store is keyed by the config
entry's name at install time, which is a direct hit in the normal case — including the case where
the registry `id` and the installed name already disagree (`spokeagent-0.4.1` does so today). It is
*not* a hit after a later rename, because a rename rewrites both the map key and the entry's `name`.
So each record also carries the `install_dir` the bundle was unpacked into, and a lookup that misses
by key falls back to matching that directory against the config's own arguments, whole and exactly
— never by parsing for a `--directory` flag. The install directory is the link a rename cannot
break: repointing it means relocating the server's code, not editing a label.

**The honest limit, stated at its real height.** There are two ways past the install-directory join,
and the cheaper one has to be named first or the bar reads higher than it is:

1. **Rename the entry *and* remove its record.** The provenance store is an ordinary file in the
   config directory, and [DR-17](privacy-tiers-execution-plan.md) descoped the filesystem barrier —
   so `config.yaml` is agent-writable and so is its sibling. Delete or truncate
   `extension-provenance.json` and the renamed entry falls back to the config-name join, which after
   a rename answers Public. The stat-keyed cache picks the deletion up on the next lookup, so this
   takes effect immediately rather than at the next restart. This is **two edits**, not one, and it
   is the actual bar.
2. **Repoint `args` at a copy of the directory.** Strictly harder — it means relocating the server's
   code, not editing a label — and it is what the directory match is designed to cost.

Neither is a regression: before DR-23 the rename *alone* sufficed, and both routes still only return
the answer the config-name join would have given, never anything lower. Nor can either be closed
here, because DR-23 forbids storing a tier — there is no retained value left to fall back on once
the identity record is gone, and inventing one would recreate the locally-forgeable field DR-23
deleted. The residual is asserted, not merely described:
`the_residual_bar_is_a_rename_plus_removing_the_record` in `extension_manager.rs` pins it, so the
next reader finds it in a test rather than discovering it.

Callers that hold a config must therefore use `classify_extension_entry`; the name-only
`classify_extension` cannot see the directory and is for callers that genuinely have only a name.

### 5.4 Knowledge bases

Added by [DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base) (R16). A base is not an incidental file — it is *"a piece of biorouter
component"* — so it carries a tier of its own, in a machine-local `<knowledge-root>/.kb-tiers` store
rather than in the session database (`biorouter-mcp` cannot depend on `biorouter`, so it cannot name
`ProviderTier`; the store carries a boolean and the crossing happens one layer up).

Three rules, and the third is the one this design did not have before:

1. **At creation.** A base a **private-capability model** creates is private from birth, before any
   ingest — the tool handler raises it immediately after `create_base` returns. A base a **user**
   creates from the Knowledge view or the CLI starts public: the user is not a model, and inheriting a
   tier from whatever chat happens to be open is the same mis-click hazard §6.1 refuses for sessions.
2. **On ingest.** The base takes the tier of the most sensitive session that has ingested into it, at
   all five choke points, and a public-capability session may neither read nor write a private base.
3. **The user may move it, in both directions, and only the user.** Publicizing is graded — a typed
   confirmation naming how many pages it releases, and the statement that it cannot be undone for
   content already read. Privatizing is one click, because nothing is disclosed by it. The control
   uses the **same** proof-of-user as §12, not a second mechanism, and there is no MCP tool that sets
   a tier. [Task 29A](privacy-tiers-execution-plan.md#task-29a-knowledge-base-publicize--privatize--user-only-graded-audited).

---

## 6. The ratchet

### 6.1 Two triggers, neither of them the bind

Binding a model is not a disclosure. Ratcheting at bind time is wrong in both directions:

- **Ergonomically**, a user who opens the picker, selects Versa and immediately reselects Claude
  has permanently privatised a chat that never sent a byte to a private model, with only an
  irreversible declassification as the exit.
- **Structurally**, it misses a hole. `POST /agent/call_tool` never touches the reply path, so a
  private-bound session could pull clinical data through `ucsfomopagent` — permitted, capability is
  Private — with `privacy_tier` still `public`, then legitimately rebind to a public model.

So classification moves at the two moments content actually appears:

1. **Reply entry (Gate B).** Before a turn runs,
   `privacy_tier := max(privacy_tier, floor(tier(bound provider)))`, reason `turn:<provider>`.
   `Agent::reply` is the single turn entry for every surface — GUI, CLI, subagent, Agent Drafter,
   scheduler, ACP.
2. **Permitted private-extension dispatch (Gate C).** On a successful dispatch to an extension
   whose `tier == Private`, `privacy_tier := max(privacy_tier, Private)`, reason `mcp:<name>`.

`Agent::update_provider` keeps **only** the refusal.

### 6.2 Persisted so it cannot be reversed

The session update builder emits plain `col = ?` through its `add_update!` macro
(`session_manager.rs:2876-2880`). Eight lines above it the same function already contains the
precedent, verbatim (`:2852-2854`):

> `// Additive, atomic accumulation. Emitted as `col = COALESCE(col,0) + ?` so a concurrent turn on the same session cannot lose an update.`

The ratchet copies that shape:

```sql
privacy_tier = CASE WHEN privacy_tier = 'private' THEN 'private' ELSE ? END
```

**This is the load-bearing line of the whole design.** No caller anywhere — a route handler, a CLI
command, a test, a future BR-71 tool, a hand-written query through the builder — can lower the
tier, whatever it passes. The storage layer refuses, not the caller. Concurrency is safe in both
orderings. Auditing "can the ratchet be reversed" is reading one SQL fragment. There is
deliberately no builder setter that accepts `Public`.

### 6.3 Irreversibility across every carrier

| Carrier | Mechanism |
|---|---|
| Restart / resume | Column plus `restore_provider_from_session` passing Gate A; Gate B repairs or refuses |
| Fork / branch / diverge / copy / import | **All four** builders carry `privacy_tier`, `provider_name`, `model_config` — see §9.3, this is a bug fix |
| Spawn | Stamped in `create_subagent_session`'s INSERT, same statement as BR-71 Task 32's `parent_session_id` |
| Scheduled job | The fresh `Scheduled` session inherits the creating session's tier at creation |
| Export / import | Absent `privacy_tier` on import ⇒ **`private`**, not the column default. Read the field only in the raising direction — `max(private_default, imported)` — never as authority to set `public`. |
| DB copy / manual SQL | The column travels with the file; a repo-grep test asserts exactly one statement in the tree matches `privacy_tier\s*=\s*'public'` outside the migration, and that statement is `declassify_session` |

### 6.4 What deliberately does not raise it

- A mixed lead/worker composite (`least = Public`, so `floor(Public) = Public`). Flagged for ruling.
- A private MCP call from a public-capability session — Gate C refuses, so there is no "it
  happened, now ratchet" path.
- Reading a public session into a private one. Upward flow is safe.
- Agent Drafter's transient per-turn switch-and-restore in `apply_route_for_turn`: it writes
  `provider_name` twice per turn, but the ratchet keys on the tier of the provider a turn *runs
  under*, not on the column changing.

---

## 7. The capability matrix

**Inputs.**

- **C** = `capability(caller)` = `least` over the components of the caller's currently-bound
  provider. `Pub` | `Priv`.
- **T** = `target.privacy_tier`, the stored classification. `Pub` | `Priv`.
**Lineage was a third input and is not one any more.** The matrix used to take
`L ∈ {self, child, other}` and make WRITE depend on it, so an agent could steer a conversation it
had spawned and only *read* one it had not (R6). The product owner has since ruled the opposite:
an agent may inject a prompt into **any** conversation — a subagent child or an unrelated chat —
as long as it does not cross the private/public boundary. **The privacy tier is the only boundary.**
R6 is retired, and `Lineage`/`lineage_of` are deleted rather than left as an unread argument.

Two things make that retirement smaller than it sounds, and both are load-bearing:

- READ and WRITE now coincide, so **no verb reaches a conversation the caller could not already
  read in full**. Widening WRITE removes a read-only cell; it does not add a target.
- The two *other* one-hop rules in this product are untouched, and neither is `may_write`. A
  delegation-scoped grant (`McpMeta::workspace_child_scope_only`) still confines an auto-injected
  supervision surface's read/close/watch to direct children, and a `SessionType::SubAgent` is
  still refused every `workspace_*` tool outright (§8.2).

**The three rules.**

```
VIS(T)     ⇔  T ≤ C                      // a public caller sees public only
READ       ⇔  VIS
WRITE      ⇔  VIS
BIND(P→T)  ⇔  WRITE ∧ tier(P) ≥ T        // Gate A, evaluated on the target
```

A downgrade write (C=Priv, T=Pub) is **permitted**, with a first-use approval. R4 explicitly
permits a private session to spawn public children, and a rule that lets you spawn a public child
but never send it a prompt makes the permission useless — it would forbid exactly the
private-leader/public-worker arrangement R2 names. But the prompt text *is* private-origin content
crossing into a public model, so the first `workspace_send_prompt` / `workspace_set_tools` from a
given caller into a given public target raises an approval showing the exact payload.

**The matrix.** `✓` allowed · `✓!` allowed, first crossing requires approval showing the payload ·
`✗` refused with a teaching message · `∅` omitted from results entirely. Nine columns became four:
with lineage gone, A and B collapse, as do D and E, and F and G — and the collapse is exactly where
the behaviour changed, because the `other` half of each pair used to read `✗ R6` on every write row.

| BR-71 tool | Class | **A**<br>C=Pub T=Pub | **C**<br>C=Pub T=**Priv** | **D**<br>C=Priv T=Pub | **F**<br>C=Priv T=Priv |
|---|---|---|---|---|---|
| `workspace_list` | read | ✓ | **∅ row omitted** | ✓ | ✓ |
| `workspace_read_conversation` | read | ✓ | ✗ | ✓ | ✓ |
| `workspace_watch` | read | ✓ | ✗ | ✓ | ✓ |
| `workspace_open` *(existing session)* | read | ✓ | ✗ | ✓ | ✓ |
| `workspace_send_prompt` *(turn / steer / note)* | write | ✓ | ✗ | **✓!** | ✓ |
| `workspace_set_tools` — extensions / skills / KBs | write | ✓ | ✗ | **✓!** | ✓ |
| `workspace_set_tools` — `add_extensions` naming a **private** extension | write | ✗ target is public-capability | ✗ | ✗ | ✓ |
| `workspace_set_tools` — `{ provider, model }` | bind | ✓ if `tier(P) ≥ Pub` (always) | ✗ | **✓!** if `tier(P) ≥ Pub` | ✓ **only if `tier(P)=Priv`** |
| `workspace_close` | write | ✓ | ✗ | ✓ | ✓ |
| `workspace_spawn_subagent` / `workspace_open { new: … }` | spawn | see §8.2 | | | |

**`workspace_list` omits private rows rather than redacting them.** The operator ruled existence
leaks *acceptable*, not *required*, and omission is strictly simpler: a `workspace_list` row
carries a **title**, and a session title in this product is LLM-generated from the conversation,
i.e. content. Redacting would mean a redaction pass to review; omitting is one `WHERE` clause and
removes the temptation to then call `workspace_read_conversation` on the id.

### 7.1 Non-BR-71 surfaces the same matrix governs

| Surface | Rule |
|---|---|
| `chatrecall` SEARCH | VIS as a SQL predicate (Gate D) |
| `chatrecall` LOAD (`session_id`) | VIS, refused before any text is built |
| Any tool on a private MCP extension | `extension.tier ≤ C` (Gate C) |
| MCP `read_resource`, `read_resource_tool`, `list_resources`, `get_ui_resources` | Same as Gate C |
| MCP `list_prompts`, `list_prompts_from_extension`, `get_prompt` | Same as Gate C |
| Tool *discovery* (`filter_tools`) | Private extensions absent from a public model's tool list |
| `active_work` registry (`ActiveWorkItem.title` is built from a subagent's task prompt) | VIS |
| `GET /sessions`, the History list | **Not** filtered — this is the user's own UI, not a model. Badges instead. **Conditional on §9.1 being fixed;** see the bypass. |
| Agent Drafter route / worker admission | The BIND predicate, replacing `provider_class` |

### 7.2 Delegation is not amplification

A private parent may spawn a public child (R4). The child is Public, sees only public sessions, and
cannot call private extensions — because VIS is evaluated against the **child's own** capability,
never its parent's. A public parent cannot spawn a private child at all, so it can never mint a
private-capability agent to read from; and even if it held one, `workspace_read_conversation` on
that child would be C=Pub, T=Priv → refused.

---

## 8. Inheritance and spawn

### 8.1 R5 is already the implemented behaviour — it needs a gate and a test, not a mechanism

`Agent::dispatch_tool_call` builds the child's config from `self.provider().await` — the parent's
**same `Arc<dyn Provider>`**, not a name and not a config. `apply_settings_overrides`
(`subagent_tool.rs:756-793`) is entered only when
`settings.provider.is_some() || settings.model.is_some() || settings.temperature.is_some()`. With
no settings, the parent's provider instance passes through untouched.

| Thing | Inherited today | Added by this design |
|---|---|---|
| Provider **instance** — hence tier, model, temperature, context limit, and the lead/worker composite | yes, same `Arc` | — |
| `AgentConfig`, including `BioRouterMode`, session manager, permission manager | yes, cloned | — |
| Extension configs — **all of them** | yes | **filtered to `ext.tier ≤ child_tier`** |
| Working directory | yes | — |
| Reasoning effort | no, deliberately (BR-63) | — |
| Knowledge bases | no | — |
| `privacy_tier` | — | **yes, stamped in `create_subagent_session`'s INSERT** |
| `parent_session_id` | — | BR-71 Task 32, moved into the same INSERT |

The extension filter closes a live hole: the parent's full extension set is snapshotted into
`TaskConfig`, `subagent_handler.rs` re-adds every one to the child, and `apply_settings_overrides`
narrows only by **name** (verified: `task_config.extensions.retain(|ext| extension_names.contains(&ext.name()))`),
never by tier — so today a session holding `ucsfomopagent` can spawn a public-model child that
inherits it verbatim.

**"The same worker/leader mode the parent is operating under" has no session-level referent in the
codebase.** `grep -rni '\bleader\b' crates/ --include="*.rs"` returns only process-group leaders.
The clause is satisfied by the `Arc` inheritance itself: because `TaskConfig.provider` is the
parent's *same* `LeadWorkerProvider` instance, a child of a lead/worker parent runs the identical
composite with the identical split, literally rather than by copying settings. Nothing to design,
nothing to persist. One consequence to write down so a future reader does not "fix" it: sharing the
instance also shares its mutable `turn_count` / `failure_count` / `in_fallback_mode`, so a
subagent's turns advance the parent's lead→worker transition. Pre-existing and orthogonal — but
cloning the wrapper to fix it would split the tier computation.

### 8.2 The spawn matrix

| Parent capability | Requested child model | Verdict | Child's `privacy_tier` |
|---|---|---|---|
| Priv | *default — inherit the parent's `Arc`* | ✓ no prompt | `private` (`inherited:<parent>`) |
| Priv | explicit Private | ✓ | `private` |
| Priv | explicit **Public** | **✓! approval** showing the task prompt | **`public`** — not the parent's |
| Pub | *default — inherit* | ✓ | `public` |
| Pub | explicit Public | ✓ | `public` |
| Pub | explicit **Private** | **✗ hard refusal** — R4: a public session may not gain private reach | — |
| any | child would inherit a private extension its own tier cannot call | ✓, extension **dropped** from the child's set, drop reported in the tool result | — |

The downgraded child is created **Public, not inheriting the parent's Private** — otherwise it
would be born in the stuck residual state. It receives only the task prompt, none of the parent's
history, and none of the parent's private extensions. That is why the confirmation shows the
prompt: the prompt is the entire disclosure.

```rust
if settings.provider.is_some() || settings.model.is_some() || settings.temperature.is_some() {
    …existing construction via providers::create…
    let child_tier = task_config.provider.tier();   // the INSTANCE, post-construction
    let parent_cap = parent_provider.tier();        // least() over a composite

    if child_tier == Private && parent_cap == Public {
        return Err(PrivacyRefusal::spawn_upgrade(…));           // R4: hard refusal
    }
    if child_tier == Public && parent_cap == Private {
        task_config.requires_downgrade_confirmation = true;     // R4 permits; disclose
    }
    task_config.privacy_tier = floor(child_tier);
    task_config.extensions.retain(|e| ext_tier(e) <= child_tier);
}
```

**Validated on the constructed instance, not the requested name.** `providers::create` can return
something other than what was asked for (the `BIOROUTER_LEAD_MODEL` intercept), and when only
`settings.model` is given, today's code keeps the parent's `provider_name` and swaps the model
string — and `versa_azure`, `versa_bedrock`, `ollama`, `llamacpp` and every declarative provider
are `allows_unlisted_models`, so an arbitrary model string is accepted. Both are harmless because
the tier is a property of the instance and never of the model id.

### 8.3 Agent Drafter workers violate R5 today

`configure_worker_provider` has no branch that reads the main agent's provider — an unpinned
profile falls through to `Config::global()`, so a worker under a `versa_azure` app runs on the
user's commercial default. Add main-agent inheritance before the global fallback, and extend the
existing §3.7 check (which today inspects only an explicit pin) to cover the fallback path.

---

## 9. Enforcement

### 9.1 Five gates

**Gate A — bind · `Agent::update_provider` (`agent.rs:4936-4956`).** Check `tier(incoming)` against
the target row's `privacy_tier` atomically, in the `WHERE` of one conditional `UPDATE`:

```sql
UPDATE sessions
   SET provider_name = ?, model_config_json = ?, updated_at = datetime('now')
 WHERE id = ?
   AND (privacy_tier = 'public' OR ? /*incoming_is_private*/ = 1)
```

`rows_affected == 0` ⇒ `Err(PrivacyRefusal::PublicModelOnPrivateSession)`. Two properties follow.
**No TOCTOU:** a concurrent ratchet cannot interleave into "private session, public provider
bound". And **the in-memory swap moves after the successful UPDATE**, inverting today's order
(verified: `*current_provider = Some(provider);` at `:4945` precedes the persist at `:4947-4955`),
so a refused swap leaves the chat running on the model it already had.

Refusing at the bind rather than the turn matters because it is the only point that acts before
private history is in front of a public endpoint; refusing at the turn means the decision is
already serialised into `sessions.provider_name` and every later reader sees a public provider on a
private session. The invariant becomes one checkable sentence: **the provider bound to a private
session is always private.**

Every path terminates here, because `update_provider` is the sole writer:

| Path | Site |
|---|---|
| GUI model picker | `ModelAndProviderContext.tsx` `changeModel` → `POST /agent/update_provider` |
| HTTP route | `routes/agent.rs:685-729` |
| Session bootstrap | `ui/desktop/src/utils/providerUtils.ts` `initializeSystem` |
| Resume / restart | `restore_provider_from_session` (`agent.rs:4960-4986`, ends in `self.update_provider`) |
| CLI configure + session builder | `biorouter-cli/src/commands/configure.rs`, `session/builder.rs` |
| Scheduler | `crates/biorouter/src/scheduler.rs:866` |
| Subagent spawn (inherit and override) | `subagent_handler.rs` |
| **Agent Drafter browser `ClientFrame::ModelSelect`** | `routes/apps.rs:3409-3428` — fixed with zero new code |
| Agent Drafter per-turn route + restore, worker profiles | `apps.rs` |
| ACP | `biorouter-acp/src/server.rs` |
| **BR-71 `workspace_set_tools { provider, model }`** | required to call `Agent::update_provider`, not reimplement the persist |

**Gate B — turn · top of `Agent::reply`.** It owns the residual state and is **repair-first**:

1. Load the session row. If `privacy_tier ≤ tier(bound provider)` → ratchet if needed, continue.
2. Else, if `session.provider_name` names a provider whose tier satisfies the classification →
   **rebind from the row and continue silently.** This turns LRU rehydration, the global-config
   fallback and legacy rows into no-ops the user never sees.
3. Else → refuse **this turn** with the repair card.

`Agent::provider()` (`agent.rs:2017-2022`) already returns `Result<Arc<dyn Provider>>` — it returns
`Err(anyhow!("Provider not set"))` today — so it can carry a second, non-DB assertion for the
completion paths that do not pass `reply`, notably `complete_fast` summarisation and session
naming, which read the entire transcript. The assertion compares two enums against a cached
`Agent.session_classification`, written by Gates A and B; `AgentManager` caches one `Arc<Agent>`
per session id, so the cache is sound and re-syncs from the row at every reply entry.

**Gate C — dispatch · `ExtensionManager::dispatch_tool_call` (`extension_manager.rs:1288`).**
Placed immediately beside the BR-23 SecretGuard block, whose in-code comment at `:1351` states
verbatim the reason this location was chosen for exactly this class of rule: *"Enforce it here —
the single choke point every tool call flows through — so no extension can bypass it."*

```rust
let ext_tier = self.extensions.lock().await.get(&client_name).map(|e| e.tier).unwrap_or(Public);
if enforcement_on() && ext_tier == Private && self.capability_tier().await == Public {
    return Err(ErrorData::new(ErrorCode::INVALID_REQUEST, teaching_message(&client_name), None).into());
}
```

By that point the function already has `client_name` from `get_client_for_tool`, `session_id`, and
the manager's `provider: SharedProvider` — **the same `Arc` the Agent swaps** (verified:
`Agent::new` passes `provider.clone()` to `ExtensionManager::new`), so a mid-session model change
is visible on the very next dispatch with zero new plumbing and no TOCTOU window.

**Returning `ErrorData` directly, not routing through an inspector, is deliberate.**
`handle_denied_tools` (`agent.rs:1961-2000`) matches on `inspector_name`, and only the hook
inspector, `"security"` and the repetition inspector get their real reason through — everything
else falls to `DECLINED_RESPONSE` ("The user has declined to run this tool"), which the code itself
calls "actively misleading". Gate C sidesteps that: the message reaches the model verbatim.

**Read off the resolved `Extension` record, never off the tool-name string.** `get_client_for_tool`
(`:1033-1040`) routes by `prefixed_name.starts_with(*key)` over a `HashMap` in nondeterministic
order, and `normalize()` permits `_`, so extensions keyed `a` and `a__b` make `a__b__c` ambiguous.
It mostly fails closed via the `strip_prefix("__")` error, but "mostly fails closed" is not an
argument for a security boundary.

Three call paths converge on this function and only one passes an inspector — which is exactly why
Gate C is not an inspector: the agent loop; `POST /agent/call_tool`; and the `execute_code` JS
bridge (`code_execution_extension.rs:1815`, dispatching inner calls straight to the manager with
the extension name as a first-class value — no static-analysis fudge needed, unlike
`SensitiveOpsInspector`).

**Gate C's siblings.** `dispatch_tool_call` is a complete choke point for *tool calls*, not for
*reaching an MCP server*. Each takes the same one-line check: `read_resource` (`:1116`) and
`read_resource_tool` (`:1043`, the worst — with no `extension_name` it loops **every** installed
extension trying the URI, actively probing private servers on the model's behalf);
`list_resources` (`:1226`); `get_ui_resources` (`:1153`); `list_prompts_from_extension` (`:1428`),
`list_prompts` (`:1458`, fans out over all) and `get_prompt` (`:1505`), since an MCP prompt body is
server-authored text that can carry data.

**Gate D — query · `crates/biorouter/src/session/chat_history_search.rs`.** One SQL literal per
builder. Detailed in §11.

**Gate E — discovery · `filter_tools` (`extension_manager.rs:877-902`).** Not a veto; the reason a
public model never sees a private server's tool names, descriptions or JSON schemas in its system
prompt. It must be here and not in `fetch_all_tools` (`:940`) or `get_all_tools_cached`
(`:904-933`): the cache is keyed on a version counter bumped only by extension add/remove, and
`update_provider` does not bump it, so filtering upstream would freeze one model's allowed set
across a mid-session model swap. Covers `get_prefixed_tools`, `get_prefixed_tools_excluding`, and
therefore `code_execution`'s importable-module catalogue for free.

### 9.2 Why no path escapes

⚠ **Read this table as "every path *through BioRouter*", not "every path".** [DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store) descoped
the filesystem barrier, so three escapes are **not** covered and are named here rather than left to be
discovered:

| Not covered (accepted, and disclosed under R15) | Why |
|---|---|
| `cat ~/.local/share/biorouter/sessions/sessions.db` from `developer__shell` | There is no read-deny in v1. The session store is an ordinary file to a tool that runs an arbitrary command. |
| Reading any file a private session wrote outside BioRouter's own stores | Working files, outputs, exports and artifacts are not tracked and carry no tier. |
| A local caller that holds the daemon secret calling `GET /sessions/{id}/export` | The secret is recoverable from the daemon's own environment; `check_token` has no principal. §17 carries the fix. |

**What the table below does cover** is the agent-mediated, transcript and tier-escalation paths — the
three §1 names — and each row is exact:

| Would-be escape | Covered by |
|---|---|
| Swap the model from GUI / CLI / HTTP / ACP / scheduler / app page / BR-71 tool | Gate A (the 11-row table above) |
| Reach a turn with a mismatched provider — LRU rehydrate, global fallback, diverge, legacy row | Gate B |
| Summarise or auto-name a private transcript on a public model | Gate B's assertion in `Agent::provider()` |
| Call a private tool from the agent loop | Gate C |
| Call a private tool from `POST /agent/call_tool`, skipping every inspector | Gate C |
| Call a private tool from inside `execute_code`'s sandbox | Gate C |
| Read a private server's **resources** or **prompts** instead of calling a tool | Gate C's siblings |
| Learn a private server's schemas without calling it | Gate E |
| Recall private chat text | Gate D + the LOAD-mode check |
| Read another session via BR-71 | The matrix, in each tool handler |
| Spawn a private child from a public parent | `apply_settings_overrides` |
| Inherit a private extension into a public child | The `TaskConfig` extension filter |
| Diverge / copy / import to launder the transcript | All four builders carry the tier (§9.3) |
| Lower the tier through any DB write | The monotone `CASE WHEN` |
| Reach a private server through an Agent Drafter app | Already impossible — a manifest can only produce `Builtin`/`Platform` configs, so `Builtin { name: "ucsfomopagent" }` fails the `BUILTIN_EXTENSIONS` lookup. **Locked in with a test**, not added. |
| Disable enforcement from the agent | The opt-out is not env-readable and is gated in `/config/upsert` — but see §9.3 C1, SecretGuard alone is not sufficient |

**Two hazards the implementation must respect.** `ToolInspectionManager::inspect_tools` logs and
continues when an inspector returns `Err`, so any privacy check expressed as an inspector must
encode every unknown as `Deny` *inside* `Ok(...)` and never return `Err`. And BR-71 Task 10's own
`Deny` branch still needs its inspector name registered in `handle_denied_tools`
(`agent.rs:1977-2000`) or every cross-session refusal is reported to the model as a user decline
and retried. One line, very easy to omit.

### 9.3 Bypasses the five gates do not cover, found in adversarial review

These are **required work**, not follow-ups. Two of them would let a public model read private
content on day one of shipping the gates as designed.

**A1 (critical) — the daemon's auth secret is an ambient bearer credential.** The original finding
was that from any public-model session with `developer`:

```
printenv BIOROUTER_SERVER__SECRET_KEY BIOROUTER_PORT
curl -s -H "X-Secret-Key: $KEY" http://127.0.0.1:$PORT/sessions/<private-id>/export
```

returns the private session's entire transcript as JSON. **The `printenv` step no longer works** —
see the status note below — but everything that made it worth reading still holds, so the finding
stays. Verified: `ui/desktop/src/biorouterd.ts:305-306` puts `BIOROUTER_PORT` and
`BIOROUTER_SERVER__SECRET_KEY` into `additionalEnv` (`:299`), spread into
`processEnv = {...process.env, ...additionalEnv}` at `:320-322`, so the *daemon* still holds the
secret in its own environment and every in-process code path can read it with `env::var`.
`auth.rs:131-142` is a plain header equality; rate limiting is keyed on peer IP, which is
`127.0.0.1`. `routes/session.rs:1076/1078/1079` expose `GET /sessions/{id}`, `/export` and
`POST /sessions/import`. Possession of the string is still total authority over the daemon, and
nothing distinguishes the renderer presenting it from anyone else.

This defeats the design specifically: Gate D filters `chatrecall`'s SQL and this route never
touches `chat_history_search.rs`; Gate B guards `Agent::reply` and this is a read. The
`UserConfirmation` ZST (§12) is true and irrelevant — the agent does not construct one, it calls
the handler that does, and `GET /sessions` yields the session name the typed confirmation wants.
It also falsifies the design's premise that `GET /sessions` need not be filtered because "this is
the user's own UI, not a model."

> **Closed for the tool-process paths (2026-07).** `strip_daemon_private_env`
> (`crates/biorouter-sandbox/src/environment.rs:54`) removes BioRouter's daemon-private variables
> from every child spawned on an agent's behalf, both the inherited copies and any the extension's
> own manifest explicitly declares — `doomed_env_keys` (`:81-87`) chains `env::vars_os()` with the
> command's own `get_envs()`. It is invoked last inside `prepare_child_environment`
> (`extension_manager.rs:445`), which every stdio and inline-python extension spawn reaches through
> `child_process_client` (`:448`, called at `:850` and `:920`), and inside `configure_shell_command`
> (`developer/shell.rs:433`), which is the Developer server's `shell`. Landed in `b249a203` and
> `8e7407fe` (issue #57). Fix (1) below is therefore **done**, and pinned by
> `daemon_secret_never_reaches_an_extension_child` (`extension_manager.rs:3541`), which re-invokes
> the test binary with the secret exported and spawns four real children through the real
> `prepare_child_environment` — clean manifest and hostile, `None` working dir and `Some(..)` —
> covering the inherited copy, a manifest that names a daemon-private key in its own `env_keys`, and
> the `Some(&working_dir)` argument both production spawns actually pass.
>
> ⚠ **"Closed" means the child's own environment, not "the model cannot get the secret".** That is
> the whole of what the strip does and the whole of what the test proves.
> [AR-11](privacy-tiers-execution-plan.md#ar-11--amended-by-dr-17--the-daemons-own-api-secret-is-recoverable)
> measured two channels that survive it, because the *daemon* still holds the secret: on macOS a
> child reads its parent's environment with `ps -Ewww -p $PPID` (under a hardened, notarized binary,
> and under every sandbox profile that can be constructed, because `sysctl-read` is not gated), and
> on Linux `computercontroller__cache view /proc/self/environ` returns it in-process. So a
> tool-capable session can still obtain the secret; what it can no longer do is find it lying in its
> own environment. Fix (2) is what closes those channels, and it is open.
>
> Fixes (2) and (3) remain open. (2) — stop carrying the secret in the environment at all — is
> unaddressed and is the reason this finding is not simply deleted: the strip is a filter, a filter
> is only as good as its key list, and AR-11's two channels do not care about the key list at all.
> (3) — bind declassification to a proof the daemon never hands its own children, rather than to
> `X-Secret-Key` — is designed as the `X-User-Action` digest in
> [Task 18A](privacy-tiers-execution-plan.md#task-18a-the-two-http-channels-that-raise-a-sessions-own-tier-and-the-user-proof-neither-of-them-has),
> with the residual local-caller half carried by
> [Open question 20](privacy-tiers-execution-plan.md#open-questions). ~~It is why Task 29's R9
> property is "only a human *through the GUI*", not "only a human".~~
>
> ⚠ **(3) is CLOSED by [R18](#3-settled-requirements) / [DR-20](privacy-tiers-execution-plan.md#dr-20--declassification-is-gated-by-a-system-authentication-and-that-is-what-lets-an-agent-ask),
> and closed by something stronger than the token this finding asked for.** The gate is an
> **operating-system authentication**, so the property is no longer "only a human *through the GUI*"
> — it is *only a human, on any surface*, including the CLI, which is why
> [Task 31](privacy-tiers-execution-plan.md#task-31-the-cli-is-a-required-r10-surface)'s subcommand
> is legitimate. The `X-User-Action` digest remains the **carrier** on the HTTP path — the Electron
> main process presents it *after* the prompt it raised — and DR-20 states plainly what the daemon
> can and cannot verify: that a process holding the per-launch key asserts an authentication
> succeeded, never that a prompt occurred. Open question 20's local-caller residual is unchanged.

Three fixes were called for. (1) **Done** — strip the daemon's credentials from every child spawned
on an agent's behalf, on the extension spawn path as well as the shell one. (2) **Open** — stop
carrying the secret in the environment at all: pass it on a pipe/fd at startup, or a `0600` file the
daemon reads and unlinks (`BIOROUTER_PORT` may stay). This is what turns the guarantee from "we
remembered to filter this name" into "there is nothing to filter"; the filter's key list
(`is_daemon_private_env_key`, `environment.rs:36-50`) is deny-by-default only inside
`BIOROUTER_SERVER__`/`GOOSE_SERVER__` and falls back to name-shaped markers elsewhere, so a future
daemon-private variable named outside those prefixes and without a credential-shaped word in it
would pass straight through. The same list is also only consulted for names that are valid UTF-8 —
`doomed_env_keys` filters on `key.to_str().is_some_and(..)` (`environment.rs:85`), so a non-UTF-8
key is never even offered to it. That is out of reach for any name BioRouter sets itself, but it is
a second reason the guarantee is "we filter this list" rather than "there is nothing to filter".
(3) ~~**Open**~~ **CLOSED by [R18](#3-settled-requirements)** — the original wording asked to bind
declassify to a one-shot capability token minted by the renderer rather than to `X-Secret-Key`, *or
R9's "only a human" property is documentation rather than mechanism*. That property is now mechanism,
and by a stronger route than a token: an **operating-system authentication** per operation, with the
`X-User-Action` digest as the HTTP carrier. See the ⚠ in the amendment banner above.

**A2 — `sessions.db` is a plain file.** `sqlite3 <db> "select text from messages_fts"` returns
every message of every session, no JSON parsing needed, because `messages_fts` is a **contentful**
FTS5 table (DDL comment at `session_manager.rs:33-47` says exactly why). `DEFAULT_SECRET_PATTERNS`
(`secret_guard.rs:33-45`) covers `**/.env`, `**/secrets.*`, keys and `**/.aws/credentials` — no
session DB. **Correction to the reviewer's version of this finding:** the DB is under
`Paths::data_dir()`, i.e. `~/.local/share/biorouter/sessions/sessions.db` on this machine — *not*
`~/.config/biorouter/`, which holds only `config.yaml` and friends. A pattern written against
`.config` would not match.

**This is bigger than `sessions.db`, and the fix is not a SecretGuard pattern.** The same reasoning
reaches the knowledge tree, the global memory store and the Agent Drafter apps: all are files, all
are readable by any tool that can run a command or open a path, and none of the five gates is on
that path. `developer__shell` executes an arbitrary command (`rmcp_developer.rs:1307`) and is
explicitly *not* jailed by the file tools' containment base (`:1950`); the OS sandbox that could
confine it defaults to **off** (`shell_sandbox/mod.rs:244`);
`computercontroller__automation_script` writes and executes a model-supplied script
(`computercontroller/mod.rs:833`). **So a public-capability model does not need to defeat any gate
in §9.1 — it can read the private material directly.**

Adding `**/sessions.db*` and the data directory to `DEFAULT_SECRET_PATTERNS` was this design's first
answer and is **rejected**, for two reasons that only appear on measurement. That list is an
always-on floor applied to *every* session, so it would hide the user's own chat history and
knowledge base from a **private** session too — a cost nothing in R1–R14 asks for. And it would not
close the read in any case: `candidate_is_denied` (`secret_guard.rs:278-292`) is lexical and
existence-gated, so a computed path or a shell expression walks past it. Stated honestly, it raises
the cost and does not close the read.

> ⚠ **Changed 2026-09-11 (QA-C H1).** The sentence above described the scan as it then was, and
> QA-C measured exactly that hole on the credential floor: `~/.aws/credentials`, `$HOME/…`, a glob
> and `cd … && head` all reached a public model. The scan now resolves a command the way the shell
> will (`~`, variables, globs, `cd`, nested `sh -c`) and refuses a match **whether or not the file
> exists**, and tool output is scanned for credential material on the way back — see
> [secret guard](secret-guard.md). The conclusion of this paragraph still stands: a path a
> *program* assembles at run time is invisible to any text scan, so a floor pattern still raises
> the cost of reading `sessions.db` without closing the read.

**The answer is §9.5** — a read-deny on the tools, conditioned on the session's capability. Note
which half of §9.5 answers which half of the objection: the *capability-conditional* part is what
makes it scoped rather than an always-on floor, and the *in-process barrier at the dispatch choke
point* is what makes it a barrier rather than a cost increase, because it evaluates the argument the
tool was actually given instead of pattern-matching a string. The OS sandbox behind it covers the one
case an in-process check cannot: a shell that constructs the path at runtime, after the daemon has
already handed over the command. Adding the patterns anyway remains worthwhile as defence in depth
for the credential half, but it is not the control.

**B1 — three copy paths, not one.** `copy_session` (`:4138-4168`) is called only by
`diverge_session_for_edit`. The **primary GUI diverge** is `diverge_session` (`:4204-4265`), which
does *not* call it and has its own builder
(`.extension_data().schedule_id().workflow().user_workflow_values().user_provided_name().diverged_from().branch_point_msg_uid()`);
`import_session` (`:4096-4135`) is a third. All three write the conversation via
`replace_conversation` and none carries `provider_name`. Reachable from `routes/session.rs:784`,
the CLI `/diverge`, and `biorouter-cli/src/commands/session.rs`. **Put the carry-over on
`create_session` itself, parameterised**, rather than on each caller's builder — three hand-rolled
builders is three chances to miss one and the fail direction is open. Add a test enumerating every
`create_session` call site.

**B3 (was critical; now a narrower channel) — `memory`'s global store.** Issues #58 and #63 both
landed in this branch's base, and between them they closed the two channels this section was written
about.

*The prompt-injection half (#58).* `MemoryServer::new`
(`crates/biorouter-mcp/src/memory/mod.rs:489`) calls `compose_instructions` (`:524`, defined at
`:632`), and global memories are now **index-only**: the global half reads
`category_names(true)` (`:641`) — which enumerates directory entries and *never opens a body*, so
the bound holds by construction rather than by convention — and contributes only sorted **category
names** under `GLOBAL_INDEX_HEADER` (`:342`), emitted at `:676`. Each name is validated as a label
and rendered as a JSON string literal, i.e. as data, so it cannot forge a prompt line. The listing
is further gated on `GlobalMemoryConsent`: a server with no consent path (`Unavailable`, `:642`)
lists nothing at all.

*The tool-call half (#63).* `retrieve_memories` (`:1200`) now calls
`require_global_consent_path` (called at `:1205`, defined at `:815`) and then refuses
`category == "*" && is_global` outright (`:1221-1234`), backed by a pre-dispatch gate in
`crates/biorouter/src/security/global_memory.rs`. The whole store stays reachable one
user-approved category at a time; it can no longer be drained in one call.

*What remains for #56* is stated in the module's own doc comment (`:627-631`): **the line is drawn
by _store_ — global vs local — not by the sensitivity of the session that wrote the entry.** Local
memories are still inlined in full (`:691-703`), so a sensitive note a private session saved locally
reaches the system prompt of every session later opened in that directory, with no tool call for
Gate C or Gate E to see. The v1 fix is therefore not a `retrieve_all` filter and not a second
consent prompt: it is to refuse `memory__remember_memory { is_global: true }` from a
**private-capability** session, which needs no storage change and is the exact mirror of Gate C.

**B4 — knowledge bases are an unclassified shared sink.** `knowledge` is a built-in ⇒ public.
Storage is a global tree `~/.config/biorouter/knowledge/<kb-id>/`, and any session may name any KB.
Gate C never fires, because both sessions are calling a public extension. **Correction to the
reviewer's version:** the active-KB state is no longer purely global — `paths.rs:66-71` adds
`.active-kb-sessions`, one file per session, with `.active-kb` as the primary fallback. So a public
session does not silently *inherit* the private session's KB by default; it does still reach it by
naming it.

**Ruled (operator, second review round): ratchet.** A knowledge base takes the tier of the most
sensitive session that has ingested into it, and a public-capability session may not read a private
KB. The read side is enforced at the seven entry points that accept an explicit `kb_id` and
therefore bypass the visible-set logic — `kb_search` (`knowledge/server.rs:590-592`),
`kb_search_raw_sources` (`:618-619`), `kb_export` (`:743`), and the four that route through
`kb_id_or_primary`, whose doc comment states the bypass outright ("An explicit `kb_id` always wins
and is never filtered against the session's set", `:308-311`): `kb_list_pages` (`:379`),
`kb_read_page` (`:396`), `kb_get_graph` (`:482`) and `kb_list_history` (`:497`).

⚠ **Those seven are the *model-facing* read surface, not the whole read surface.** The daemon's HTTP
routes read a base by a caller-supplied path id with no visible-set filtering either — `GET
/knowledge/bases/{id}/` `graph` (`routes/knowledge.rs:38` → `get_graph`), `location` (`:39` →
`:496`), `page` (`:40` → `:822`), `pages` (`:41` → `:522`, and `pages/{*page_path}` at `:42-45` →
`:543`), `history` (`:46`), `preview` (`:47`) and `export` (`:55` → `:1522`), plus the two raw-source
handlers at `:1604`/`:1636`. Those are the GUI's own path and are user-driven rather than
model-driven, so scoping the *tool* ratchet to the seven MCP entry points is the right call — but
§9.3 A1 establishes that the daemon secret is an ambient bearer credential — no longer sitting in a
tool child's own environment, but still held in the daemon's, and equal to full authority for
anything that can read it.
⚠ **This paragraph used to end with a second clause, citing
[AR-15](privacy-tiers-execution-plan.md#ar-15--retired-by-dr-16--a-caller-holding-the-daemon-secret-can-raise-its-own-sessions-capability-with-no-credentials)
for the claim that holding the secret also lets a caller raise its own session's capability. That
clause is **withdrawn**, because the risk it cited is closed.** AR-15 was **retired** on 2026-08-02
(DR-16, commit `0757823f`): binding a session to a private provider
over HTTP now needs a proof that a human acted (`X-User-Action`, minted per launch by the desktop
app and held by the daemon only as a digest) on top of the secret, so the secret alone raises
nothing. **What the secret still buys is unchanged and is the point of this paragraph** — it reads
these KB routes directly, and that is what makes the seven-tool ratchet a partial control.
⚠ **Do not read that as "a tool cannot get it".** [AR-11](privacy-tiers-execution-plan.md#ar-11--amended-by-dr-17--the-daemons-own-api-secret-is-recoverable)
measured a child recovering its parent's environment with `ps -Ewww -p $PPID` on macOS and
`/proc/self/environ` in-process on Linux, so a shell-capable session can still reach a KB through
the HTTP side without touching any of the seven — which is why fix (2) of A1, not the strip, is what
would close this. The implementing task must decide deliberately whether
the HTTP routes carry the check too, and record the answer; it must not conclude from this paragraph
that seven checks are the whole job.

The tier lives in a machine-local sidecar beside `.active-kb` and `.hidden-kbs`, not in
`manifest.yaml`, because the manifest travels inside the `.brkb` archive and an imported tier would
be attacker-supplied.
Existing knowledge bases migrate **public** (fail-open, DR-10) even if a private session fed them —
an accepted cost, recorded as
[AR-2](privacy-tiers-execution-plan.md#ar-2--every-knowledge-base-that-exists-today-starts-public-at-migration-even-if-a-private-session-fed-it). The way back out of the ratchet is
**user-only**: DR-18 gives the user a publicize/privatize control (Task 29A), which is what resolved
the original AR-1 — a base ratcheted by one private page is *not* unreadable for ever. No model can
invoke it, in either direction.

**B4.1 — the selection itself is content, so a public session sees a filtered view of it and its
pointer can read `null`.** A knowledge base's **id and name are user-authored** and routinely name a
cohort or a study, which is the same reasoning §7 uses to make `workspace_list` *omit* private rows
rather than redact them. That rule has to reach the pointer, not just the bases: `kb_get_active`
takes **no arguments** and returns the whole selection — every visible id, plus the primary and its
deprecated `active_kb` mirror — so one call with nothing to guess enumerates what `kb_list_bases`
omits. So a public-capability session sees `knowledge_bases` with the private ids removed, and
`primary_kb` reads `null` whenever the stored pointer names a base it may not reach. That is
truthful for that session rather than a redaction: it has no write target it can use, and its
KB-less writes correctly fail with the existing "no primary" message. `kb_set_active` refuses a
private target with the sentence a *nonexistent* id gets, byte for byte — a refusal that said
"private" would confirm the base exists in a politer sentence.

**The filter is on the view and never on the store**, and that is a decision rather than an
implementation detail. The stored pointer is what the "one axis, one pointer" repair logic reads;
that logic promotes a primary to the first remaining member of the set and *writes it*. Filtering
the store would therefore make a public model's read silently re-point the user's own primary,
machine-wide and persisted, and the Knowledge view would then show the moved pointer with nothing
having asked for it. One truth on disk; two model-facing tools render a capability-scoped
projection of it.

**B5 — the composite misclassifies the backfill in the leaky direction.** `get_name()` returns the
lead's name (`lead_worker.rs:332-334`) and `update_provider` persists exactly that string. With
`BIOROUTER_LEAD_PROVIDER=anthropic` over a `versa_azure` base, every worker turn runs on the
private gateway while `sessions.provider_name` records `"anthropic"` — so the backfill marks it
public and a public model may read it forever. The mirror case is an annoyance rather than a leak
(lead=`versa_azure`/worker=`anthropic` backfills private, then Gate A permanently refuses the very
composite it was running). Fix: persist the composite truthfully (`provider_components: Vec<String>`
or a `lead:X|worker:Y` encoding) and backfill from it; at minimum treat any `provider_name` that is
unresolvable-as-a-composite as `backfill:unknown` rather than public.

**B6 — Gate E cannot see the extension record as specified, and its prefix rule disagrees with Gate
C's.** `filter_tools` is a **sync** `fn` over `&[Tool]` deriving the prefix as
`tool.name.split("__").next()`; `get_client_for_tool` resolves by `starts_with` over a
`tokio::sync::Mutex<HashMap>`. So (1) a tier lookup inside `filter_tools` requires the function to
become `async` or to take a **precomputed allowed-set** from the async caller — the caching
argument for why it must live there is correct, so pass a `HashSet<&str>`; and (2) for an extension
keyed with an embedded `__` the two rules diverge (`a` vs `a__b`), and while Gate C fails closed,
Gate E could **show a private server's tool names, descriptions and JSON schemas** to a public
model. Schema text is content. Derive both from one resolver.

**C1 — SecretGuard cannot stop `shell` from writing `config.yaml`.** `find_denied_path` scans
tool-argument strings and `candidate_is_denied` requires a literal path token that exists on disk.
`cd ~/.config/biorouter && python3 -c "open('config.yaml','a').write(...)"` evades it, as does any
variable indirection. The module's own doc-comment concedes it is "conservative by design". Do not
rely on it for the opt-out: hold the authoritative value in daemon memory from startup and require
the GUI IPC path to change it, or read it from a trusted file using the `managed/trust.rs`
`verify_trusted` pattern — noting `verify_trusted` is a **no-op on Windows**.

> ⚠ **Changed 2026-09-11 (QA-C H1)** — the mechanism, not the advice. The scan no longer needs a
> literal existing token: it follows `cd`, variables and nested shells and refuses a match whether
> or not the file exists ([secret guard](secret-guard.md)). The advice stands for a different
> reason: `config.yaml` and `privacy-tiers.json` are not in the floor, and a path a program
> assembles at run time is invisible to any text scan, so SecretGuard is still not a control for
> the opt-out.

**C2 — a scheduled job created from a private session becomes permanently, silently broken.**
`scheduler.rs:844-866` builds its provider from `Config::global()`, creates a fresh `Scheduled`
session, then calls `agent.update_provider`. Under this design that session inherits the creator's
private tier, Gate A refuses the bind, and `run_workflow_job` returns `Err` on every cron tick
forever — with no repair affordance, since a new session is minted per run. Resolve the scheduled
session's provider from the **creating session's** `provider_name` before falling back to global
(which is also what R5 wants), and surface a job-level error in the schedules UI.

**P6 — CLI plan mode ships the entire conversation to a different provider, outside every gate.**
`biorouter-cli/src/session/mod.rs:786-791` and `:881-885` clone the whole message list and hand it
to `get_reasoner()` (`:2260-2290`), which builds a provider from `BIOROUTER_PLANNER_PROVIDER`
falling back to the global default — never touching `Agent::update_provider`, never entering
`Agent::reply`, never recorded on the session row. A documented first-class feature and a complete
private→public transcript leak. **Hard refusal in `get_reasoner`**, not a warning, with a message
naming `BIOROUTER_PLANNER_PROVIDER`.

### 9.4 Attacks that were tried and genuinely fail

Recorded so they are not re-litigated: `execute_code` inner dispatch (hits the real Gate C choke
point with the extension name as a first-class value); `POST /agent/call_tool` (same reason — this
is the correct placement call); a stale provider inside the extension manager (the `SharedProvider`
`Arc` is the same one `update_provider` writes through); a mixed composite calling a private MCP
server (`least = Public` → refused); a public parent spawning a private child then reading its
output; a private parent's public child reading back up (VIS is evaluated on the child's
capability); `workspace_read_conversation` on an ancestor (which was never about lineage, and is even less so now that lineage decides nothing);
`chatrecall` SEARCH after Gate D; a stale registry copy downgrading an extension (the union rule
holds).

---

### 9.5 The sixth control — a private-data read-deny for public capability, on by default

> ⛔ **DESCOPED for v1 by operator ruling ([DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store)), 2026-07-30 — retained, not deleted.**
> *"We don't have to enforce and encrypt every single step along the way. for now."* Everything in
> §9.5 below specifies the **filesystem** channel and is not being built: Layer A (the argument
> barrier), Layer B (the OS sandbox), the four roots and the two file entries. The execution plan's
> Tasks 14A–14F carry the same banner and keep the measured platform analysis, which is the expensive
> part and stays true.
>
> **Two things in this section survive the descoping and must not be read as deferred.** (1) The
> **tool channel** for knowledge bases — §9.5.1's third column, "the roots' doors" — is exactly the
> CP1–CP4 barrier the KB gates implement, and [DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base) makes it a requirement (R16) rather than a
> defence-in-depth extra. (2) The structural argument for *why* enumerating file readers cannot work
> is the most durable paragraph here, and any future filesystem control must start from it.
>
> **What is deliberately NOT true in v1:** that a public-capability session's tools cannot reach
> BioRouter's private data on disk. They can. See §1 and R15.

**Ruling (descoped — see the banner above).** When a session's capability is **public**, its tools may
not reach BioRouter's own private data. Private-capability sessions are unaffected: this is not a general jail and must not
become one. Everything outside the named entries stays readable and writable, so ordinary work is
untouched.

#### 9.5.1 It is two channels and three enforcement points

There are two ways a read of a private root reaches the disk, and the difference is who supplies the
path. That is a structural fact about the product, not an implementation detail, so it belongs in the
design.

**The filesystem channel — the *caller* names the path.** A tool argument (`text_editor` with
`path: ~/.config/biorouter/knowledge/p.md`), or a shell command line. The name is visible before
anything runs, so it can be refused before anything runs.

**The tool channel — the *handler* supplies the path.** `agent_drafter__read_app` receives an app id
and a relative path, and the store joins the root itself. `knowledge__kb_read_page` receives a base
id and a page. `memory__retrieve_memories` receives a category. **Nothing in the arguments names a
private root**, so a barrier that inspects arguments finds nothing to refuse and the handler opens
the file anyway. Three successive adversarial reviews found a reader the previous round's control
could not see, and the third found this whole class.

| | **Layer A — the argument barrier** | **Layer B — the OS sandbox** | **The roots' doors** |
|---|---|---|---|
| Channel | filesystem | filesystem | tool |
| Covers | every tool call's arguments, at the daemon's own dispatch choke point | the processes the daemon spawns | every read that goes through a root's own resolver |
| Mechanism | a refusal at `ExtensionManager::dispatch_tool_call`, before the tool's future exists | Seatbelt / bubblewrap wrapping the child | a capability-taking guard inside `ArtifactStore`, the knowledge service's choke points, and the memory funnel |
| Enforced by | BioRouter | the kernel | BioRouter |
| Needs kernel support | **no** | yes | **no** |
| Granularity | the whole root — a raw path cannot be attributed to an object inside it | the whole root | the **object**, where the root has a per-object tier; the whole root where it has none |
| Defeated by | a read whose path the handler supplies | a pre-planted hardlink (both kernels match paths, not inodes); on Linux, a root absent at job start | a reader that does not use the root's resolver — which is why each resolver is private and each leak around it is closed |

**All three are the ruling, and none of them is optional.** An earlier draft of this section carved
the tool channel out of the ruling entirely, on the grounds that `kb_read_page` and `read_app` are
governed by the extension classification instead. That was a redefinition of the ruling rather than
an implementation of it: the ruling says a public session's *tools* may not reach BioRouter's private
data, not "unless the tool owns the directory".

**A public session reaching a *public* object is not an exemption — it is the root's door
answering.** The ruling protects BioRouter's *private* data. A knowledge base classified public, a
session row classified public, an app built in a public chat: none of those is private data, and
refusing them would be a general jail of exactly the kind §9.5's ruling forbids. What is not
acceptable is a root whose contents are **undifferentiated** being handed over because no gate
exists. That was the Agent Drafter root, which had no per-app classification at all, and §9.5.3 says
what it gets instead.

**Why an in-process check has to exist at all.** `computercontroller__cache` reads a caller-supplied path with
`tokio::fs::read_to_string`; `agent_drafter__read_app` reads app bytes with `std::fs::read_to_string`;
`developer__text_editor` opens files directly. **None of them spawns anything — they are the
daemon.** No sandbox the daemon installs on its children can constrain the daemon, so on every
platform, including the two where Layer B works perfectly, those reads are governed by a check in the
code path or by nothing at all. Two successive adversarial reviews found a public tool reading a
private root without spawning a process; the second found one *inside* an extension the first had
already named.

**Why the check must sit at a choke point rather than on a list of tools.** The readers cannot be
found by grep — `xlsx_tool` reads through `umya_spreadsheet`, `pdf_tool` through `lopdf`,
`data_query` through `sqlx`, none of which contains an `fs::` token — and there are 125 tool
declarations in `biorouter-mcp` with 48 path-shaped parameters, a count that both over- and
under-states the real set. **Any control phrased as "the tools that read files" is unmaintainable by
construction. It must be phrased as "every tool call passes through symbol X", and the test for it
must be a tool the production code has never heard of.**

**What the in-process halves buy, and it changes the cost of this feature.** Because Layer A and the
roots' doors hold everywhere — neither needs kernel support — Layer B's platform gaps stop being
feature-killers. Landlock cannot subtract a read and
Windows has no unprivileged confinement — but that no longer means *"a public session cannot read
files on those hosts"*, it means *"a public session cannot spawn a shell on those hosts"*. The
fail-closed refusal narrows from **every file tool** to the five that spawn a child:
`developer__shell` and its background jobs, `computercontroller__automation_script`,
`computer_control`, and `compute_run`/`compute_python`. `text_editor`, `analyze`, `image_processor`,
`cache`, `xlsx_tool`, `pdf_tool`, `docx_tool` and every knowledge / memory / drafter tool keep
working on Windows.

#### 9.5.2 The entries

**Four roots**, and two files:

| Entry | Resolved by | Why |
|---|---|---|
| `<data>/sessions` | `Paths::data_dir()` + `SESSIONS_FOLDER` | `sessions.db` and its FTS mirror — §9.3 A2 |
| `<config>/knowledge` | `knowledge::paths::knowledge_root()` | the tree the KB barrier gates |
| `<config>/memory` | `memory::global_memory_dir()` | the global store §9.3 B3 is about |
| `<config>/agent_drafter` | `agent_drafter::default_root()` | app source, `.vault/`, **and app ids** |
| `<config>/privacy-tiers.json` | `privacy::master_switch::path_for` | **the master switch itself.** §10.6's toggle is loaded from this record at startup, and it is an ordinary non-`SecretGuard` file: a public model can edit it and the next restart has privacy tiers off — *announced* since H3 (a load `WARN` and a standing note in the app, §10.6), never prevented |
| `<config>/config.yaml` | `Paths::config_dir()` + `CONFIG_YAML_NAME` | **hooks.** A planted `PermissionRequest` hook loads from here (§17 Q2) and is the one control that can bypass the spawn-downgrade approval. Five tools can write it — `text_editor`, `shell`, `cache` **delete**, `kb_export`'s `dest_path` and `export_app`'s `target_dir` — so protecting one of them protects nothing, and the same five reach every other entry in this table |

⚠ **The switch's row moved, and an implementer working from this table must not harden the wrong
file.** Before [DR-22](privacy-tiers-execution-plan.md#dr-22--the-master-switch-does-not-live-in-configyaml) the toggle
lived in `config.yaml` and this table had one file entry, which is where that row's reasoning came
from. Task 42 moved the value to `privacy-tiers.json` and *retired* the `config.yaml` key — it is
read once at migration and ignored for ever after — so `config.yaml` is no longer a route to the
switch, and `privacy-tiers.json` is. `config.yaml` stays in the table on its own merits, which are
hooks.

Note the two different directories: the session store is under `data_dir`, the other five under
`config_dir`. A deny list written against one prefix misses most of them, and every test that
relocates both with `BIOROUTER_PATH_ROOT` still passes.

Two properties of the entry list, both learned the hard way:

- **The two file entries deny reads as well as writes,** because telling a read from a write
  means knowing which argument of which tool is a destination — the per-tool knowledge this design
  has just finished abandoning. The cost is stated in §16: a public chat cannot view `config.yaml`
  through a tool. The user can still open it.
- **The verdict is not existence-gated.** `<config>/memory` is created lazily on first write and does
  not exist on a fresh install, so a containment test that requires the path to exist fails open on
  it — while every test written against a populated fixture passes.

#### 9.5.3 What each enforcement point covers

**Layer A** refuses any tool call whose arguments name a path inside an entry, at the one dispatch
choke point, plus the seven branches the agent short-circuits before that choke point is reached (one
of which, `platform__manage_schedule`, reads an arbitrary `workflow_path` in-process).

**Layer B** wraps the five spawning tools, carrying the same decision as a per-call flag in the MCP
request metadata rather than in session state.

**The roots' doors** refuse a read the handler was about to perform, inside each root's own resolver,
consulting the object's classification where the root has one.

**All three read one capability, sampled once.** The capability is captured at the entry that admits
the tool call — the agent loop, the daemon's `call_tool` route, the code-execution bridge, or the
pre-turn prefetch — and threaded, with the master switch's state captured in the same instant. It is
never re-derived downstream. That matters for a reason worth stating in the design rather than in a
task: the daemon returns a tool call as an **un-awaited future** that then queues behind a global
concurrency limit, so "downstream" can be minutes later. A gate that re-reads the model binding at
that point can let a call admitted under a public model run with private privileges — and a
mid-session model change would then take effect *retroactively*, on work already permitted.

The cost of that choice, stated: **switching a chat's model takes effect on the next tool call, not
on the ones already in flight.** A call admitted as public stays public (a spurious refusal, which
the user resolves by asking again); a call admitted as private stays private for its duration.

**What Layer A does *not* cover, and what does:** the tools that own these roots reach them through
their own resolvers, so nothing in their arguments names a root and Layer A finds nothing to refuse —
`kb_read_page`, `retrieve_memories`, `read_app`, `list_apps`. **That is a gap in Layer A, not an
exemption from the ruling**, and it is closed at each root's own door:

| Root | Its door | What the door consults | If the object is private |
|---|---|---|---|
| `<config>/knowledge` | the knowledge service's four tool-facing choke points (CP1–CP4), plus the two ends that bypass them, `kb_export` and `kb_import` | the base's tier (§9.3 B4's ratchet) | refused, without naming the base |
| `<config>/agent_drafter` | `ArtifactStore`'s private `dir()`, through which every id-keyed read passes — plus its `list()` and its raw-root accessor | **a per-app tier, which this design adds**: an app takes the tier of the most sensitive session that has written to it, exactly as a knowledge base does | refused; and the app's id and title are omitted from `list_apps` rather than reported as hidden |
| `<config>/memory` | `get_memory_file`, the funnel all four memory tools pass through, plus `retrieve_all`'s enumeration | nothing — and it needs nothing, because §9.3 B3 refuses a **private** session's write to the global store, so the global store can only ever hold public content | n/a; the invariant is what makes this door open, and it is load-bearing |
| `<data>/sessions` | the API, not the filesystem: it is a SQLite pool and BioRouter never reads it as files. Gate D (`chatrecall`), Gate G (`ingest_conversation`), and `read_session_blob`, which is scoped to the session's own id | the session row's classification | refused, without naming the session |

**Two consequences worth stating plainly.**

1. **Agent Drafter apps gain a classification they do not have today.** Without one the root is
   undifferentiated, and the only rules available are "all readable from a public chat" (which is
   the hole) or "none readable from a public chat" (which takes a flagship feature out of every chat
   on a commercial model). Every app that exists at migration is classified **public** — there is no
   evidence to classify it otherwise, `Manifest.session_id` is optional and the session it names may
   be gone — so nothing changes on upgrade day and the rule starts applying to what happens next.
2. **The tool channel is finer-grained than the filesystem channel, and a user can see the seam.** A
   public chat may `read_app` its own public app and may not `cat` the same file from the shell,
   because a raw path cannot be attributed to an object inside the root. That asymmetry is a cost of
   having one rule that needs no per-object knowledge; narrowing the filesystem side is a follow-up.

If a tool-channel classification is wrong, the fix is still to change the classification — but "there
is no classification" is not a classification, and that is what this section changes.

#### 9.5.4 What each platform can express, for Layer B

Measured by execution against `crates/biorouter-sandbox` rather than assumed — macOS on a real host,
Linux in a real `bubblewrap 0.8.0` container:

- **macOS (Seatbelt) — yes, directly.** A `(deny file-read* (subpath …))` appended to the base
  profile subtracts the subtree, verified in the production shape where the writable root is `/` and
  is therefore an ancestor of every entry: read, write and `rm` inside the entry all fail while
  writes elsewhere all succeed. A single file uses `literal` rather than `subpath`. Effectively free.
  **Every path must be canonicalized into the profile**: a deny declared with an uncanonicalized
  spelling matches *nothing at all*, in both directions, and the profile still compiles and runs.
- **Linux (bubblewrap) — yes, with `--tmpfs <root> --remount-ro <root>`.** `--tmpfs` after
  `--ro-bind / /` overmounts the directory with an empty tmpfs in the child's own mount namespace —
  but a bare `--tmpfs` is **writable**, so the read-only remount is what makes the policy true as
  stated. A single file is bound read-only from `/dev/null`. Requires `bwrap` installed **and**
  unprivileged user namespaces enabled — and the first does not imply the second: under default
  Docker seccomp `bwrap` is present, executable, and fails on every invocation. **The capability must
  be a live probe, on both platforms, and the probe needs two legs** — one asserting the deny bites,
  one asserting an unrelated read still succeeds — because a host where the sandbox cannot start any
  process passes a one-legged probe.
- **Ordering is load-bearing on Linux and fails open silently.** A `--tmpfs` (or a file's
  `--ro-bind`) emitted *before* the writable `--bind` of its parent is defeated with no error and no
  diagnostic. Three of the four roots live under `$HOME`, which is routinely the session working
  directory and therefore a writable bind root, so this is the common case rather than a corner.
- **Linux (Landlock) — no, and not because of this host.** A Landlock ruleset is a set of *grants*
  with no deny rule and no way to subtract a subpath from a broader grant; the implementation
  deliberately leaves read accesses unhandled so reads stay open. Expressing a deny means granting
  the *complement* — every sibling of every ancestor of every deny root, re-enumerated per command —
  and anything created in one of those ancestors after the ruleset is built becomes unreadable for
  that command's lifetime. Declined; a control whose failure mode is "the file I just wrote cannot
  be read back" is a control that gets switched off.
- **Windows — no.** There is no unprivileged, general-purpose confinement that can wrap an arbitrary
  developer command without breaking it; the sandbox module's own header works through the five
  candidates and why each fails.

**The fail direction is closed, for the five spawning tools.** A public-capability session on a host
where the kernel deny cannot be established does **not** get an unsandboxed arbitrary-execution tool.
Those specific tools are refused, with a deterministic error that names the state, the reason, and
the two ways out — switch this chat to a private model, or turn privacy tiers off for this machine
(§10.6) — and forecloses the retry. On Linux the message names a third fix for the machine itself
(`install bubblewrap`). The refused tools are **not** hidden from the model's tool list: hiding them
makes a model invent workarounds, while a refusal that forecloses the retry makes it stop. Every
other tool keeps working on such a host, because Layer A is not what failed.

**The costs, stated rather than discovered.** On Windows, and on Linux without bubblewrap, a
public-capability chat loses the shell — which for a commercial model on a Windows laptop is the
common configuration, and is a large part of why R7's opt-out is a master switch. A
public-capability chat also cannot read its own history through the `biorouter` CLI (that reads the
session store), cannot `cat` its own drafted app's source (that is the fourth root), and cannot view
`config.yaml` through a tool (that is the sixth entry). All four are the control working as
specified.

#### 9.5.5 The second-order path, stated honestly

The daemon's HTTP API is another read path: `GET /sessions/{id}/export` returns a transcript to
anyone holding `BIOROUTER_SERVER__SECRET_KEY`. An earlier version of this section concluded that the
child-environment strip closed it — *"a sandboxed child knows where the daemon is and cannot
authenticate to it."* **That conclusion is withdrawn.** Measured: on macOS a child reads its parent's
environment with `ps -Ewww -p $PPID`, and the protection people assume (SIP withholding process
environments) applies only to Apple *platform* binaries — a locally compiled binary leaks, and so
does the shipped, notarized, hardened-runtime `biorouterd`. Nor is it `ps`'s setuid bit: a plain
non-setuid `sysctl(KERN_PROCARGS2)` reader recovers it, including under a Seatbelt profile carrying
the deny entries. On Linux the daemon's own `/proc/self/environ` is readable in-process by any tool
that reads a caller-supplied path.

**So: the daemon's API secret is not defensible against a tool running inside the daemon, and no
sandbox this feature installs changes that.** What remains true, and what it costs:

- The strip (`strip_daemon_private_env`, the fix for §9.3 A1, applied last in each command builder)
  is still correct and still required. It keeps the secret out of the child's own environment, where
  a careless `env` dump or log would find it, and it keeps every **remote** caller out.
  `BIOROUTER_PORT` and `BIOROUTER_APP_BASE_URL` are deliberately kept, so locating the daemon is free
  and meant to be.
- **The biggest local route is held by Layer A, not by the secret.** `POST /agent/call_tool` executes
  any tool of any extension with no capability check and no approval prompt — and it dispatches
  through the same choke point Layer A and Gate C sit at. A caller holding the secret therefore gets
  exactly what the chat already had. This is the concrete reason the barrier belongs in the extension
  manager and not one frame up in the agent.
- **What is left exposed** is the set of routes that return private content without running a tool:
  the transcript family, the `/knowledge/*` read routes, `GET /apps/{id}/export`, and
  `GET /diagnostics/{id}` — which returns a zip of `session.json`, recent logs and a verbatim
  `config.yaml`, and is the widest single route in the API. Closing that needs a per-caller
  credential the daemon does not hand to its own children (open question).

One further residual, and it is larger than previously stated: `GET /apps/{id}` and
`GET /apps/{id}/agent` are deliberately unauthenticated, and the served page carries the app's socket
token, so a client that knows an **app id** can drive that app's agent — including any knowledge base
the app's manifest granted it, private ones included. It was previously argued that denying the Agent
Drafter root removed the only local source of app ids. **That is false:** `agent_drafter__list_apps`
is a Public tool that enumerates every id in-process, takes no path argument, and is therefore
untouched by both layers. A public model does not need to already know an app id — it can ask for
one. `GET /apps` still requires the secret, and that is now the only thing standing there.
Authenticating the app socket against a local client is a separate change (open question).

---

## 10. MCP badges

### 10.1 Where the badge is declared

**`landing/registry.json`, and nowhere else** (R11 i). Nothing local can grant private; the `.brxt`
bundle is never read. A bundle *could* self-declare `"privacy": "private"` trivially — `main.ts`
validates only `name`, `display_name`, `description`, `version`, `entry_point`, `repository`,
`env_vars`, and `BrxtInstallModal.tsx:152-161` writes a config recording **no provenance
whatsoever** (no registry id, no source URL, no hash). What stops it is that the resolver never
reads the bundle, and never reads `config.yaml` either.

### 10.2 How the badge reaches the enforcement layer

Enforcement is Rust; the registry fetch is Electron (`main.ts:2832` `REGISTRY_URL`, IPC handler at
`:2855-2866` — a bare `fetch`, no timeout, no cache, no integrity check). The Rust side has no
network path and the CLI and daemon have no Electron, so **a runtime-fetch-only design cannot
enforce anything outside the desktop app.**

`landing/scripts/build-registry.mjs` gains a second output:

```
crates/biorouter/src/privacy/registry_private.rs      (@generated — do not edit)
    pub const PRIVATE_EXTENSIONS: &[&str] = &["cdwagent", "ucsfomopagent"];
```

with a `--check` mode wired into `just check-everything`, mirroring the theme-generator precedent
CLAUDE.md already blesses (`npm run themes -- --check` inside `lint:check`).
`ui/desktop/src/components/baam/registry.fallback.json` — today a hand-maintained copy with no
generator and no CI check, in sync at 37/37 and 129/129 by luck — joins the generator's outputs and
the same gate.

**The matching key is fixed in the generator, not with a heuristic.** Verified in the live registry:
SPOKE's `id` is **`spokeagent-0.4.1`** (derived from the download filename by `slugFromUrl`) while
the installed extension is keyed `spokeagent` from `manifest.name`. `landing/baam.html` already
carries a `featIndex` workaround for this; the app has none. Today harmless — spokeagent is public
— but the identical bug would classify a future version-suffixed *private* extension as unlisted ⇒
public. The generator emits a stable `extension_name` field, the Rust const is keyed on it, and the
build fails if any entry marked `private` lacks one. A suffix-stripping heuristic in a security
path is exactly the thing that is right until it isn't.

**Freshness raises, never lowers.**

```
private_set = PRIVATE_EXTENSIONS ∪ private(last_good_fetch)
```

An upgrade (newly private upstream) takes effect on the next successful fetch and persists. A
downgrade requires a successful live fetch and is never honoured for an entry the compiled-in set
names. An offline laptop can fail to *learn* a new private badge; it can never *lose* one. The
fetch gains a 10 s `AbortController` and writes the last-good copy; `loadRegistry`'s
existing-but-discarded `live: boolean` is surfaced as "catalogue last updated <date>".

### 10.3 The catalogue, classified

| Extension | On BAAM | Tier | Evidence |
|---|---|---|---|
| `ucsfomopagent` | yes, `id: ucsfomopagent` | **PRIVATE** | verified in `landing/registry.json` |
| `cdwagent` | yes, `id: cdwagent` | **PRIVATE** | verified |
| `spokeagent` | yes, `id: spokeagent-0.4.1` | PUBLIC | operator ruling — SPOKE holds no patient data; its passcode gates the service, not private content |
| `medcp` | **no** | PUBLIC | verified absent — all 37 ids scanned |
| `msbaseagent` | **no** | PUBLIC | verified absent |
| the remaining 34 catalogue entries | yes | PUBLIC | no marker |
| built-ins: `developer`, `autovisualiser`, `computercontroller`, `memory`, `agent_drafter`, `knowledge` | n/a | PUBLIC | R11 |
| platform: `todo`, `chatrecall`, `extensionmanager`, `skills`, `code_execution` | n/a | PUBLIC | R11 |
| in-process app servers: `appcontrol`, `datasql`, `files`, `compute`, `evidence` | n/a | PUBLIC | per-app sandbox |
| anything hand-installed | no | PUBLIC | R11(ii) |

**The private set is exactly two strings.**

`Frontend` extensions are public by construction — `Agent::add_extension` intercepts them before
the ExtensionManager sees them and `add_extension` refuses the variant outright
(`extension_manager.rs:691-693`), so no EM gate can ever see one. Assert it as a test so the
invariant is deliberate rather than incidental.

### 10.4 Accepted risks, stated plainly

**Fail-open on hand-installed extensions is a deliberate bypass, and it is live right now.**
`medcp` is verified enabled in the operator's config with `CLINICAL_RECORDS_BACKEND`,
`CLINICAL_RECORDS_SERVER`, `CLINICAL_RECORDS_DATABASE` and `CLINICAL_RECORDS_USERNAME` in the
plaintext `envs` map and `CLINICAL_RECORDS_PASSWORD` in `env_keys`, against a clinical MSSQL
backend. **A public model can query patient data through it today, and will still be able to after
this design ships.** The operator's reasoning: a hand-installed extension is the user's own choice
and responsibility, and medcp is a *connector* rather than a data source — the same binary may be
pointed at a local SQLite file. The reasoning and the consequence belong in one sentence, and they
are: **the badge is a statement about provenance, not about the data behind the connector.**

**Publishing to BAAM is what grants a private badge.** `medcp` and `msbaseagent` both reach
institutional data and are public *solely because they are unpublished*. Where and by whom that is
revisited: **in the pull request that adds the card to `landing/baam.html`, by the Baranzini Lab
reviewer.** Two mechanisms make that a build failure rather than a thing someone remembers, and one
mechanism that used to be claimed here no longer exists.

1. **The `--check` gate** (`node landing/scripts/build-registry.mjs --check`, run by CI and by
   `just check-everything`) fails unless the three generated outputs are exactly what `baam.html`
   generates — so a card cannot be published without `landing/registry.json` and the desktop
   fallback changing in the same commit, and a *private* card cannot be published without
   `crates/biorouter/src/privacy/registry_private.rs` changing too.
2. **The private set is a closed list** (`EXPECTED_PRIVATE_EXTENSIONS` in
   `landing/scripts/build-registry.mjs`, Task 54). The generator hard-fails unless the catalog's
   private set is exactly `{cdwagent, ucsfomopagent}` with exactly the affiliation each is recorded
   under — checked in all three directions, so an extra private card, a missing one, and a
   re-affiliated one are each a named failure. A card therefore **cannot make itself private**:
   doing so takes two edits, the list and the page, and the second is what makes somebody review the
   first.

⚠ **What neither mechanism covers, and what was given up to get here.** This section previously
asserted a third: that the generator hard-fails when a card's description matches a clinical keyword
list (`patient`, `clinical record`, `EHR`, `PHI`, `medical record`, `de-identified clinical`) with no
`data-privacy` attribute. **That rule was deleted in Task 54** (operator ruling, 2026-08-04: *"the
description don't matter"*). It inferred a security property from marketing prose and could only
produce false failures — SPOKE describes diseases, and an imaging or literature tool can honestly say
"patient" while touching nothing sensitive. Its one real use was the case this section is about: a
future clinical extension whose author forgets to tag it. **The closed list does not catch that** —
the set simply stays at two, and an untagged clinical card publishes as public. That case now rests
entirely on the Baranzini Lab reviewer named above. If it needs mechanical cover later, the answer is
an explicit field on the card, not a return to guessing from the description.

**Two naming consequences, known rather than discovered:** a hand-installed extension *named*
`ucsfomopagent` inherits the private badge (fail-closed, fine); and a genuinely private extension
renamed locally used to become public.

The second is closed for marketplace installs by
[DR-23](privacy-tiers-execution-plan.md#dr-23--an-extensions-tier-is-re-derived-from-the-registry-never-stored-locally):
the install records the registry `id` and the install directory beside the config entry, and the
resolver unions them with the name, so renaming the entry changes nothing. It remains open for the
two cases that carry no registry id — an extension installed **before** that change, and a `.brxt`
dropped in by hand from a local file — where the config-name join is still the only join available.

**A mechanical scan of the 37-entry registry surfaces one candidate the ruling does not cover** and
eight that hit a single signal: `ucsfhpcagent` (UCSF CHPC/SLURM job planning, account inspection,
file transfer) scores on both credential and institutional-data signals; `labarchivesagent` (an
electronic lab notebook — unpublished experimental data), `benchlingagent`, `latchbioagent`,
`opennotebookagent`, `lamindbagent`, `clinicalvariantagent`, `protocolsioagent` and
`ginkgocloudlabagent` hit one. Description prose is a usable but insufficient signal —
`omeroagent` and `dnanexusagent` scored clean despite reaching institutional imaging and genomic
platforms. Not a blocker; a list for the reviewer at publish time.

### 10.5 The v2 option, clearly labelled

**Per-installation credential-derived privacy:** an extension declaring credentials for a clinical
or institutional data source is treated as private *on that machine*. The registry's authority is
untouched — this can only *add* private, never remove it. Two corrections to how it is usually
framed, both verified and both in the design's favour:

1. **It needs no registry-schema change.** `env_keys` and `envs` already exist locally on the
   `Stdio` and `StreamableHttp` variants of `ExtensionConfig`.
2. **It closes `medcp` and does not close `msbaseagent`.** `medcp` puts the identifying keys in the
   plaintext `envs` map and only the password in `env_keys`, so a rule reading `env_keys` alone
   misses it — it must read **both** maps. `msbaseagent` declares no credential keys at all
   (`env_keys: []`, `envs: {MSBASE_LOG_LEVEL}`): no credential-shaped local signal exists for it.
   The rule would also fire on `spokeagent` (`SPOKEAGENT_PASSCODE`) unless the pattern list is
   narrow, which would contradict a settled ruling.

Cost: one hand-maintained pattern list — the one hand-maintained list this design otherwise avoids.
Hence v2, opt-in, and never a correction to the ruling.

### 10.6 The opt-out (R7) — one master toggle

Global, explicit, on by default: `BIOROUTER_PRIVACY_TIERS` (default `on`), and it is a **master**
switch over the whole feature. With it off there is no bind gate, no turn gate, no dispatch gate, no
discovery filter, no `chatrecall` filter and no `chatrecall` LOAD guard, no knowledge-base barrier,
no scoping of the Agent Drafter catalog, no forced export location for a model's `.brkb`, **no
refusal when a public session enables a private extension and no stripping of a private server's
instructions from a public system prompt** (both of which are tool-call-shaped only by analogy and
are the two channels an enumeration is most likely to miss), no spawn matrix, no classification on a
copied session, no visibility predicate, no classification ratchet and **no read-deny at all**
(§9.5) — neither the in-process barrier nor the OS sandbox. Nothing is refused, nothing is
sandboxed, and no path is out of reach.

**The enumeration above is the specification, and it is checked mechanically rather than read.** A
master toggle wired to *some* of the enforcement points is the failure mode here, and it is invisible
to any textual check: the execution plan therefore asserts every point in both toggle positions, and
closes the list at both ends — every place that reads the switch must have a row, and **every place
that refuses on privacy grounds must read the switch**. The second half is what catches an
enforcement point added later that nobody thought to wire, because a new enforcement point is a new
refusal. The one exception is the OS sandbox, which refuses nothing — it hands the kernel a policy —
and so is carried by its own row and its own paired assertion.

**An earlier draft of this section scoped the opt-out to Gate C** — turning off the tool gate
decides what a model may *call*, whereas turning off the session barrier retroactively exposes data
gathered under a private badge — and flagged the scope as open question 3. The operator has ruled
the other way, reading their own words ("opt out of the **entire** protection layer") literally. The
narrower key is retired rather than kept alongside the master one: two switches whose scopes nest
are two things a user must reason about at the moment they are least able to.

**What it does *not* do.** It does not drop the columns, the stamps already written, or the audit
rows, so turning it back on resumes enforcement over the history that existed when it was turned
off. And it does not hide the badges: they keep rendering, restyled and suffixed *— enforcement
off*, beside a persistent strip. Hiding them makes an unprotected machine indistinguishable from a
machine with nothing private on it, at exactly the moment the distinction matters; leaving them
unchanged makes a pill reading plain **Private** a false statement the user acts on.

**The one-way cost, which the confirmation must state.** With the toggle off the ratchet does not
run, so a session that handled private material during that window stays stamped `public` for ever:
there is no content scan, no provenance on the messages, and `privacy_tier` is monotone, so
re-enabling cannot go back and mark it. The alternative — keep ratcheting while the guardrails are
off — was rejected because it silently privatises sessions the user believes are unprotected, and
they would first learn of it as a refusal weeks later.

Three hardening measures, because the failure mode is an agent disabling its own protection:
(1) read it bypassing `Config::get_param`'s env branch, so no environment variable can turn
protection off; (2) gate the key in `POST /config/upsert` so the flip must come from Settings →
Privacy with its confirmation; (3) hold the authoritative value in daemon memory from startup —
**not** SecretGuard, which cannot enforce this (§9.3 C1). Check *inside* the gate rather than in an
`is_enabled()`, following the `SensitiveOpsInspector` pattern, so a mid-session change is honoured
and the opt-out is one auditable line rather than an absent gate.

**Where the answer is recorded, which is not `config.yaml` (DR-22).** The three measures above left
one channel open: the value's home was a file, and the general filesystem barrier was descoped, so
writing `BIOROUTER_PRIVACY_TIERS: off` into `config.yaml` by hand was still a **next-launch**
disable — and "only on restart" is not a control, because daemons restart routinely and a model can
wait. The value therefore lives in its own record, `privacy-tiers.json`, beside `config.yaml` in the
configuration directory, written by nothing in the tree except the confirmed `/config/upsert` arm.
`BIOROUTER_PRIVACY_TIERS` remains the name the switch is *addressed* by — the panel reads and writes
it over `/config/read` and `/config/upsert`, and the daemon answers those from the live value — but
as a `config.yaml` **key it is retired**: read once, at the first start-up after the upgrade, to
carry an existing answer across, and removed and ignored from then on. Honouring it "for
compatibility" would not be a kindness, it would be a second channel.

⚠ **What this does not claim.** The record is an ordinary file, and the descoped filesystem barrier
leaves it writable by an agent holding `developer__shell` exactly as it leaves the disclosure
acknowledgement writable. What the move buys is that the documented key is inert and that the value
has one writer; closing the file channel outright needs the barrier DR-17 deferred or an
OS-authenticated store, and neither is in v1.

**What it does instead: an OFF answer is never silent** (H3 of the 2026-09-10 security test drive).
The drive overwrote the record with `{"enabled":false}` from a chat's `developer__shell`; at the
next launch every gate and the ratchet were off and nothing said so — no log line, no banner, only
a badge suffix and the strip inside Settings → Privacy. The file channel is still the accepted
risk above; the silence is what changed.

- **The load announces it.** `master_switch::load` logs one `WARN` per process whenever the record
  resolves to OFF, naming the file and the fact. ON is the default and is not announced.
- **Each door stamps what it writes.** The record carries `changed_by: {via, set_to,
  system_authenticated, user_action}` — `via` is `settings` (the typed-confirmation arm of
  `/config/upsert`) or `migration` (DR-22's one-time carry). A record whose `enabled` no stamp
  vouches for — no stamp at all, or one whose `set_to` disagrees — is **unrecorded**, and the WARN
  and the app both say *turned off outside the app*. The stamp names its value and not only its door
  because the edit an agent is likeliest to make is a one-field flip (`jq '.enabled = false'`) that
  keeps every other byte; a door-only stamp would vouch for a value it never wrote. A stamp that
  does not parse is read as no stamp and never makes the record unreadable, for `changed_at`'s
  reason: a stamp must not be able to turn a user's `off` into the fail-safe `on`.
- **The report travels with the value.** The loader and the confirmed write — the atomic's two
  writers — each remember a report beside their write, served on `/config` and `/config/read` as
  `BIOROUTER_PRIVACY_TIERS_RECORD`: `enabled`, `origin` (`default` / `settings` / `migration` /
  `unrecorded`), the record's `path` and the `last_change`. It is served from memory, never a
  second read of the file, and both read paths overwrite any copy `config.yaml` holds under that
  name, so a pre-filled report written through `/config/upsert` reaches no reader.
- **The app stands a note above every composer** while the tiers are off
  (`PrivacyTiersOffNote`), and Settings → Privacy's strip carries the same account. It has no
  dismiss control — anything that remembered a dismissal would be a file an agent could write — so
  the only way to clear it is to turn the tiers back on. Its visibility is the switch's alone; a
  daemon that sends no report costs the explanation, never the note.
- **The record is created owner-only (`0600`)** by both doors.

⚠ **None of this is a barrier, and the stamp is not a proof.** The switch still obeys the file,
exactly as DR-17 accepted. Anything that can write the record can write a plausible stamp into it;
what the stamp reliably catches is the overwrite the drive measured and a one-field flip, not a
forger who has read this section. `0600` keeps *other* local accounts out and does nothing about
the agent, whose shell runs as the user and therefore owns the file — so no permission, ownership
or integrity check at load can tell the agent from the user, and a check keyed to a secret on this
machine would be exactly as strong as the agent's inability to read that secret, which DR-17 does
not give. A record written by a Biorouter older than the stamp — including a deliberate `off` from
Settings → Privacy — also reads as unrecorded, and the copy says so rather than accusing anyone;
turning the tiers on and off again in Settings → Privacy stamps it.

Because it is now one predicate read by every gate, the test that matters is a **matrix**: each gate
asserted in both toggle positions. A master toggle wired to three gates out of ten passes every
textual check and is the failure this design is most likely to ship.

---

## 11. `chatrecall`

### 11.1 LOAD is the mode that gets forgotten

`handle_chatrecall` (`chatrecall_extension.rs:78-159`) has two modes and only SEARCH touches the
FTS index. **LOAD has no filter of any kind** — not even the `exclude_session_id` guard SEARCH has.
It takes a caller-supplied `session_id`, calls `get_session(&sid, true)`, and builds:

```
Session: {name} (ID: {sid})
Working Dir: {working_dir}
Total Messages: {total}

--- First Few Messages ---   [first 3, message text verbatim]
--- Last Few Messages ---    [last 3, message text verbatim]
```

The guard goes immediately after `get_session` and **before the header string is built**, so not
even the name or working directory escapes. Ship this first, on its own, ahead of everything else
in this design.

### 11.2 SEARCH — filtered in the query, in both builders

`execute` branches on `fts_available()` (`:108-116`, a `sqlite_master` probe), so a design that
filters only the FTS path leaks on any un-migrated profile. **`sessions` is already joined in
both** — `fetch_rows_fts` at `:135` and `build_sql` at `:211` — so it is one clause each, in the
same position as the existing exclusion:

```rust
if caller_is_public { sql.push_str(" AND s.privacy_tier = 'public'"); }
```

**Written as a SQL literal, not a bind.** Both builders bind conditionally and positionally
(`:154`, `:176`), so an inserted `?` in the wrong position mis-binds silently. The literal is a
compile-time constant of the code path, not user input.

To make an unfiltered search unconstructible, `ChatHistorySearch` takes
`caller_capability: ProviderTier` as a **required constructor parameter** — not a builder setter,
not an `Option`. It threads through three signatures, so a missed call site is a compile error.
`caller_is_public` is `agent.capability_tier() == Public` — the **live provider's** tier, not the
caller session's stored classification. That is the correct operand: the question is who is about
to read this, and the reader is the model. It also means a private session in the residual state
reads as a public caller, which is the safe direction.

### 11.3 Why in the query rather than a partitioned index

1. **`LIMIT ?` is applied by SQLite** (`:150`, `:244`). Post-filtering in Rust would silently
   truncate a public model's results whenever private rows outrank them — fewer public hits than
   exist, non-deterministically, with no error.
2. Private rows are then never in the result set, so `process_rows`, `get_session_totals` and
   `convert_to_results` need no knowledge of tiers. The reviewable surface is two lines of SQL.
3. `messages_fts` is a **contentful** FTS5 table maintained by hand from Rust at every message
   write site (`session_manager.rs:33-47`). Partitioning into `messages_fts_public`/`_private`
   would double all five hand-written write/rebuild/delete sites and — decisively — would make the
   ratchet and declassification **reindex an entire session's text**. With the column on
   `sessions`, both are one `UPDATE` and zero write sites change.

### 11.4 Content versus existence, field by field

| Field | Verdict | Why |
|---|---|---|
| `m.content_json` → `messages[].content` | **CONTENT — withheld** | it *is* the message text, including the `[Tool: …]` renderings |
| `messages[].role`, `messages[].timestamp` | **CONTENT — withheld** | meaningful only attached to a withheld body; returning them buys nothing and invites reconstruction |
| `s.description` → `session_description` | **CONTENT — withheld** | the LLM-generated session title, produced *from the conversation*. A summary of private text, not a label. The field most likely to be mislabelled as metadata, and the one that leaks most per byte. |
| `s.working_dir` → `session_working_dir` | **CONTENT — withheld** | a filesystem path, but in this product it routinely names a cohort, a study or a patient population |
| `ChatRecallResult.last_activity` (`session/chat_history_search.rs:14`, rendered at `chatrecall_extension.rs:219`) | **CONTENT-adjacent — withheld** | it is `max` over *matched* message timestamps (`:347-351`), so it dates the private message containing the search term, not the session. Under §11.4's own rule ("anything derived from a message body is content") it is message-derived. Moot once rows are filtered in SQL, but a reviewer checking the table for completeness must not find a hole. |
| `s.id`, `s.created_at` | existence — may be revealed | acceptable per R13 — **and only safe because LOAD is gated independently.** The two decisions are coupled and must stay coupled. |
| `total_matches`, `results.len()`, `total_messages_in_session` | existence — may be revealed | and nothing is needed: `total_matches` is summed *after* filtering, and `get_session_totals` counts only sessions already in the filtered set |

**The rule a reviewer can apply without re-reading this:** anything derived from a message body is
content; anything the row would have had before its first message is existence.
`session_description` fails that test, which is the point of stating it.

**Side channels are explicitly out of scope** (R13): no count padding, no constant-time responses,
no decoys, no cover traffic. A public model may see a bare "no results", a reduced count, or a
timing difference. The invariant is narrower and checkable: **no snippet, no title, no excerpt, no
working directory, no field drawn from a message body ever reaches a public model** — a property of
the SQL rather than of the rendering code.

### 11.5 Tests

FTS path and LIKE path, each: a public caller with a private chat containing the term gets zero
rows, a private caller gets the row. The `LIMIT` interaction: 10 private rows ranking above 3
public rows with `limit = 5` → a public caller gets all 3 public rows, not 0. LOAD: a public caller
passing a private session id gets a refusal whose text contains no part of the session name or
working directory. `get_session_totals` consistent with the filtered set. A source-level test
asserting `s.privacy_tier` appears in **both** builders.

---

## 12. Declassification

> ⚠ **AMENDED in three places by [R18](#3-settled-requirements) / [DR-20](privacy-tiers-execution-plan.md#dr-20--declassification-is-gated-by-a-system-authentication-and-that-is-what-lets-an-agent-ask), 2026-08-02.**
> The **authorization** is now an operating-system authentication prompt, raised once per operation
> and naming the exact chats it covers. §12.1's *"one-shot token minted by the renderer over Electron
> IPC"* is retired unbuilt; §12.2's *"no CLI subcommand can construct one"* is withdrawn; §12.6's
> *"No general bulk declassification"* is superseded — a batch is now the general case. §12.3's
> wording, §12.4's grading and §12.5's audit are **unchanged in substance**: the grading now governs
> the in-app review step that precedes the prompt, because DR-20 admits no grading of the prompt
> itself. The implementation is
> [Task 29](privacy-tiers-execution-plan.md#task-29-declassification--the-system-authentication-the-batch-and-the-audit)
> and [Task 31](privacy-tiers-execution-plan.md#task-31-the-cli-is-a-required-r10-surface).

### 12.1 Where it lives

**History → the session's own row → overflow menu → "Make this chat public…"**, shown only when the
chat is private, and the identical control on the session-detail header, sharing one
`DeclassifySessionDialog` so the two entry points cannot diverge. Not in Settings. Not in the chat
header. Not anywhere an agent can reach.

Route: ~~**`POST /sessions/{session_id}/declassify`**~~ → **`POST /sessions/declassify`** under R18,
taking `{ "session_ids": [...] }`, because a per-id route cannot express one authentication over N.
Not under `/config/*`, not a tool, not reachable from any `workspace_*` handler or MCP server, and
explicitly not added to the public-GET exemption list. **Per §9.3 A1, secret-key auth alone is not
sufficient** — ~~bind it to a one-shot token minted by the renderer over Electron IPC~~. **R18
supplies what that sentence assumed and never defined:** the Electron **main** process raises the OS
prompt and makes the call itself, presenting the per-launch user-action key
[DR-16](privacy-tiers-execution-plan.md#decisions-of-record) already requires — one proof of user,
not two. The renderer never calls this route.

**The same mechanism, for knowledge bases.** [DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base) adds a second user-only tier change —
publicize / privatize a base (§5.4) — and it reuses this section's proof-of-user, this section's
dialog primitive and this section's "exactly one lowering writer in the tree" rule. **Two mechanisms
for one idea is how the two confirmations diverge**, so a `POST /knowledge/bases/{id}/tier` that
accepts the secret key alone, or a `kb_set_tier` MCP tool, is the wrong implementation of §5.4 and
not a shortcut. [Task 29A](privacy-tiers-execution-plan.md#task-29a-knowledge-base-publicize--privatize--user-only-graded-audited).

**This section is R17's agent half, and §12.3–§12.4 are its user half.** Declassification is the
clearest instance of the pair: a model may never invoke it under any circumstances, and the user is
never *refused* it — they are shown what they are about to expose, graded by how it became private,
and then allowed to proceed. Neither half is negotiable independently of the other.

### 12.2 Why no agent can invoke it

```rust
// crates/biorouter/src/privacy/declassify.rs
pub struct UserConfirmation(());   // ZST; constructor is pub(in crate::…)

pub async fn declassify(sm: &SessionManager, session_id: &str, ok: UserConfirmation) -> Result<()>
```

⚠ **Amended by R18.** `UserConfirmation` is no longer a ZST and its constructor is no longer the
HTTP handler: it is a private-field newtype over the **authorised id set**, returned only by
`privacy::system_auth::authenticate` after an approved OS prompt, and consumed **by value** — so a
proof minted for one batch is not spendable on another and none is spendable twice. No MCP server,
no `ToolRouter` and no `workspace_*` handler can construct one; **the CLI can**, and that is now
correct, because what gates the operation is the prompt rather than the caller
([Task 31](privacy-tiers-execution-plan.md#task-31-the-cli-is-a-required-r10-surface)). "An agent
cannot *complete* this" is enforced by Rust's module privacy plus the operating system, rather than
by the route being undocumented.

> ⚠ **What actually landed differs from the paragraph above in shape, not in property** (Task 55,
> which wired DR-20's prompt to the two operations that had been ruled to need it and had no caller).
> Task 29 shipped before DR-20 was ruled, so `UserConfirmation` **stayed** a ZST carrying the
> user-action half. The newtype over the authorised id set exists beside it as
> **`declassify::SystemAuthorization`** — private field, no public constructor, neither `Clone` nor
> `Copy`, returned only by `declassify::authenticate_declassification` after an approved prompt, and
> checked per chat with `covers(id)`. There is no `privacy::system_auth::authenticate`: the prompt
> lives in `declassify.rs` so that `system_auth` never has to name the proof-of-user, which
> `the_proof_of_user_is_constructed_in_exactly_two_places` would otherwise fail.
>
> **Three residuals, recorded rather than left to be discovered.**
>
> **(a) Step 3's batch has no consumer.** The **route is still per-id** (`POST
> /sessions/{id}/declassify`), so §12.1's `POST /sessions/declassify` taking `session_ids[]` is still
> unbuilt; both real doors pass a one-element slice. A batch costs one prompt *in the mechanism* —
> one `SystemAuthorization` covers the set it named, pinned by
> `one_prompt_covers_a_batch_and_covers_only_the_chats_it_named` — but no HTTP surface and no UI
> reaches it, so **today's product still costs one prompt per chat**. Read plainly: Step 3 shipped a
> mechanism with no caller, which is structurally the same defect Task 55 exists to fix, one level
> down. It is the safe version of it — the property is tested and the missing piece is a route, not a
> gate — but it should not be read as "batch declassification now costs one prompt" in the product.
>
> **(b) The prompt is raised by the daemon**, not by the Electron main process as §12.1 describes.
> That is Task 44's mechanism and it is what makes the CLI door work at all; the cost is that on
> macOS the dialog is presented by `biorouterd` rather than by the app. ⚠ **Treat this as a shipping
> risk, not a footnote.** If `LAContext.evaluatePolicy` cannot present a dialog from a background
> helper — or presents one behind the app window — then every `mcp:*` chat becomes **undeclassifiable
> from the desktop app**, and no test in the tree can see it, because tests reach the seam and never a
> platform prompter. What *is* closed: the failure cannot read as success. `DeclassifySessionDialog`
> calls the route with `throwOnError: true` and routes a rejection to `toastError`, covered by
> `DeclassifySessionDialog.test.tsx`, and the 403's own sentence names the operating system as the
> thing that did not confirm. What is **not** verified: whether the dialog appears at all on a
> notarized macOS build, and whether that sentence survives the generated client into the toast body.
> Both need a human at a real GUI.
>
> **(c) The master switch's prompt is agent-reachable.** `/config/upsert`'s privacy-tiers arm returns
> before the `is_capability_key(&key) && !is_user_action(&headers)` gate below it, so disabling the
> tiers takes the typed phrase and `is_secret == false` and **no `X-User-Action`**. (The task text's
> premise that "the master switch demands `X-User-Action` alone" was never true of the code, before
> this change or after it.) `PRIVACY_TIERS_DISABLE_PHRASE` is a public constant in shipped source, so
> a tool call can compose the request. Net of Task 55 Step 2 this is strictly better — what used to
> disable the tiers *silently* now needs an operating-system dialog that fails closed — but it does
> introduce something that did not exist: **a hostile tool call can raise an OS authentication dialog
> on demand**, which is a prompt-fatigue and social-engineering surface. Requiring the header looks
> nearly free, because `ConfigContext.tsx` is the GUI's only path to `/config/upsert` and always
> sends it; what stops it being free is `UserActionProof::NoKeyInstalled`. `just run-server`, a
> hand-run `biorouterd` and every headless deployment hold no key and would be refused, and
> `/config/upsert` is the **only** production writer of the switch record — so the requirement would
> make the feature permanently un-disableable there, short of hand-writing `privacy-tiers.json` and
> restarting. Refusing only `Unproven` while still allowing `NoKeyInstalled` is the shape that closes
> it without that cost. Left for a ruling rather than taken inside a fixup, because it is a change to
> an authorization boundary and not to Task 55's four steps.

It is also **the only writer in the tree permitted to lower `privacy_tier`.** Every other write
goes through the session update builder, whose emission is the monotone `CASE WHEN` and physically
cannot lower it; `declassify_session` bypasses the builder with its own `UPDATE`. A repo-grep test
asserts exactly one statement matching `privacy_tier\s*=\s*'public'` exists outside the migration —
the same shape as the existing `scripts/check-version-consistency.sh` guard. That single assertion
is the whole audit surface for "can the ratchet be reversed".

### 12.3 The wording

> ### Make this chat public?
>
> **"{session name}"** is marked **Private**. It has been running on **Versa · versa_azure**, a
> model hosted by UCSF, since **14 Mar 2026**. Its contents may include data from UCSF systems —
> patient records, clinical notes, or other institutional data.
>
> Making it public means **any model can read it from now on** — including commercial models hosted
> outside UCSF such as Anthropic, OpenAI and Google. That covers its full message history, the
> files and query results in it, and anything a private extension returned. It will also start
> appearing in chat-recall searches run by those models.
>
> By continuing you are asserting that nothing in this conversation needs to stay inside UCSF.
>
> **This is permanent and cannot be undone.**
>
> **[ Cancel ]   [ Make public — permanent ]**

If the chat is private by inheritance, one extra line appears: *This chat inherited its private
badge from **"OMOP cohort characterisation"**. Making this one public does not change that chat.*

Destructive button styling using the app's existing destructive treatment (no new colour). `Cancel`
holds initial focus. No keyboard shortcut binds the confirm button.

### 12.4 The confirmation — graded by what was actually touched

This is where the practitioner review changed the design, and it is the single most important
ergonomic decision in it. `privacy_reason` already distinguishes the two cases; use it:

| Reason | Control | Rationale |
|---|---|---|
| `mcp:*`, or inherited from an `mcp:*` ancestor | Typed confirmation + audit row + permanent record | A private data source was actually reached. |
| `turn:*` only, never any `mcp:` event | **Single-click "Make public"** with a 5-second undo, still audited, still user-only, still not agent-invocable | Nothing from a private data source entered this session. The only thing that happened is that text was *sent to* a UCSF-hosted endpoint, which is not a disclosure of UCSF data. |

This preserves every invariant (no automatic downgrade, only a human can lower it, monotone in SQL,
one writer) and removes the friction from the large majority of affected rows — see §16.

**When a typed confirmation is required, confirm on the last 6 characters of the session id**,
displayed immediately beside the field — **not** the session name. `is_default_session_name`
(`session_manager.rs:1614-1632`) shows `"New chat"`, legacy `"New Session"`, `"CLI Session"`, `"Session <N>"` and
`"New session <N>"` are all live placeholders, and the naming pass is best-effort, so failures
leave sessions stuck on them; `fallback_session_name` (`:1527`) produces up-to-60-character titles
built from the first words of the user's message. A name-typed phrase is therefore either a
duplicate string shared by dozens of rows — destroying the justification, which is forcing the user
to look at *which* conversation — or a sentence to retype. An id suffix is unique, short, and
forces row-identity checking.

### 12.5 What is recorded

**An append-only audit table**, written in the same transaction, *before* the `UPDATE`:

```sql
CREATE TABLE classification_audit (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  session_id              TEXT NOT NULL,
  from_classification     TEXT NOT NULL,
  to_classification       TEXT NOT NULL,
  reason                  TEXT NOT NULL,   -- 'declassified_by_user'
  actor                   TEXT NOT NULL,   -- OS user + machine
  actor_kind              TEXT NOT NULL,   -- always 'user'; no other value is constructible
  occurred_at             TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
  app_version             TEXT NOT NULL,
  provider_name_at_change TEXT,
  privacy_reason_before   TEXT,
  message_count_at_change INTEGER
);
```

**And a transcript record**, following the BR-71 Task 32 pattern of a
`user_visible: true / agent_visible: false` message written into the session's own conversation:

> *Made public by **wgu** on 27 Jul 2026 at 14:12 UTC. Before this change the chat was **private**
> (private since 14 Mar 2026 — first turn on Versa · versa_azure). All models can read this chat
> from now on.*

paired with the ratchet's own record, written when the tier is first raised:

> *This chat became private on 14 Mar 2026 on its first turn using Versa · versa_azure.*

Both are `agent_visible: false`, so the model never sees them and cannot be steered by them. The
transcript record travels with the session on export; the audit table survives session deletion.
They answer different questions — "what happened to this chat" and "what has ever been declassified
on this machine". A `warn!` with the stable event name `session_declassified` is emitted alongside,
matching the convention already used for `catastrophic_command_blocked` and `command_policy_deny`.

The session row keeps `privacy_reason = 'declassified_by_user'`, so History shows the badge as
**"Public — made public by you on 27 Jul 2026"** permanently. A declassified session must never be
indistinguishable from one that was always public.

### 12.6 After declassification, and bulk

The chat is genuinely public: its bound provider is left exactly as it was (a public chat may run a
private model — that direction was never restricted), nothing is re-indexed, nothing is rewritten.
If a private model later runs a turn on it, the ratchet fires again and the user may declassify
again. That is deliberate — declassification is an assertion about the contents *as they stood*,
not a permanent exemption.

⚠ **SUPERSEDED by R18: bulk is now the general case.** The operator ruled that *"each
declassification action can declassify multiple chats (in batch) if the user so wants it"* — one
system authentication may cover any set the user assembles, provided the set is **fixed before the
prompt and named inside it**. What survives from the paragraph below is its *reasoning*, which
became the general design: a batch is presented as a review list naming every chat, and it applies
as **one transaction** or not at all. The `backfill:*` case is now an instance rather than the sole
exception.

~~**No general bulk declassification.**~~ One exception, extended from the original design per §16:
sessions whose reason is `backfill:*` get one grouped dialog with a review-by-provider list,
because a backfill is a **guess made by the system from the last-used provider**, not a user
assertion about content — and `provider_name` records only the *last* provider, so the guess is
wrong in both directions. The per-session flow then applies only to sessions this build actually
observed going private.

---

## 13. Marketplace, registry and landing site

### 13.1 `landing/registry.json` — schema change

Two new fields per extension; registry `version` goes 1 → 2.

```json
{
  "id": "spokeagent-0.4.1",
  "extension_name": "spokeagent",     // NEW — stable join key, == the bundle's manifest.name
  "privacy": "public",                // NEW — "private" | "public", default "public"
  "name": "SPOKEAgent", …
}
```

`RegistryExtension` (`ui/desktop/src/components/baam/registry.ts:8-19`) gains both, `privacy`
optional with a `'public'` default so an old cached document still parses.

### 13.2 `landing/scripts/build-registry.mjs` — generator change

The generator scrapes `landing/baam.html` with regexes and already has the idiom to copy
(`const license = first(/data-license="([^"]+)"/, card) || 'Apache-2.0';` at `:102`). Add:

```js
const privacy        = first(/data-privacy="([^"]+)"/, card) || 'public';
const extension_name = slugFromUrl(download).replace(/-v?\d+(\.\d+)*$/, '');
```

plus both keys in the emitted object, plus **hard build failures** (the generator currently never
fails): if `privacy` is neither value; and if `privacy === 'private'` and `extension_name` is empty.

> **Amended 2026-08-04 (Task 54).** This section originally listed a third failure — a card whose
> description matches a clinical keyword list with **no** `data-privacy` attribute — and called it
> the mechanism that forces the medcp/msbaseagent revisit at publish time. **That rule was never
> going to work and is not in the shipped generator.** It is replaced by a closed list,
> `EXPECTED_PRIVATE_EXTENSIONS`, which fails the build unless the catalog's private set is exactly
> `{cdwagent, ucsfomopagent}` with the affiliation each is recorded under. The closed list does *not*
> cover the untagged-clinical-card case; see the ⚠ in [§10.4](#104-accepted-risks-stated-plainly)
> for what that trades away and who now carries it.

**The default matters more than the extraction.** Defaulting to `'public'` means an un-annotated
card is public by construction, so R11(ii)'s fail-open direction is enforced by the tool rather
than by reviewer discipline.

Two further outputs, both new: `crates/biorouter/src/privacy/registry_private.rs` and
`ui/desktop/src/components/baam/registry.fallback.json`, both joining the `--check` mode.

### 13.3 `landing/baam.html` — five components

The shelf is rendered client-side (`loadRegistryExtensions` at `:3941` fetches `registry.json` with
`cache:'no-store'`, then `renderExtensions` at `:3864` empties the static grid), so both the
template and the fallback change.

1. **`extCardHtml` (`:3804`)** — add `data-privacy="${privacy}"` on the card div beside the
   existing `data-license`, and **prepend** the badge to the `.ext-tags` row. Prepending matters:
   that row is `max-height: 22px; overflow: hidden` (`:191`), so an appended badge would be clipped
   on cards with many tags. **Both states are labelled** — `<span class="tag private">Private</span>`
   and `<span class="tag public">Public</span>` — because a badge shown only on private teaches a
   visitor nothing about its absence.
2. **`buildExtChips` (`:3838`)** — a Private/Public facet next to the existing UCSF/Community org
   chips.
3. **`filterExtensions` (`:3909`)** — one clause. It already special-cases a dataset-attribute
   facet for `org`; extend the ternary to `(f === 'org' || f === 'privacy')` against
   `c.dataset[f]`. **This is what makes "tell at a glance which are private before installing" real
   rather than aspirational** — a visitor can filter the shelf down to the two private extensions.
4. **The static no-JS cards (from `:471`)** — simultaneously the no-JS view *and the generator's
   input*, so this is where classification is authored. `data-privacy="private"` +
   `data-extension-name` on the CDWAgent card (`:471`) and the UCSFOMOPAgent card (`:495`), plus
   the visible badge in each `.ext-tags` row (`:486`, `:510`). The other 35 rely on the generator
   default.
5. **`landing/shared.css:406-415`** — a `.tag.private` variant beside the existing `.tag.ucsf` and
   `.tag.mcp`:

   ```css
   .tag.private { background: rgba(5,32,73,0.07); color: var(--ucsf); }
   .tag.public  { background: var(--bg-3);        color: var(--text-3); }
   ```

   Private uses the **navy** ramp: institutional in tone, visually distinct from the coral MCP tag.
   **No new red.** The palette has exactly one accent (coral `#b85a32` text, `#cf6d47` bars-only)
   and no semantic danger colour; inventing one breaks the Apple-deck reskin. Dark overrides go in
   the same block, since `landing/theme.js` sets the `.dark` class pre-paint.

### 13.4 Other landing surfaces

`landing/docs.html:1468-1478` carries a hand-written "Extension agents in the marketplace" table
listing six agents with an Agent / What it connects / Credentials header. It gains a **Privacy**
column, plus a new `landing/scripts/check-docs-privacy.mjs` in the site check step comparing it to
`registry.json` — nothing generates this table today, so nothing would catch the drift the day BAAM
ships badges. `landing/skills.html` and `landing/index.html` list no extensions and need no change;
say so in the doc so a later reviewer does not "fix" it.

### 13.5 How a user learns a hand-installed extension is public

The fail direction is settled; these make the consequences legible **before** the user trusts it
with sensitive work.

- **Extensions settings** shows a badge plus provenance on every card, in three distinct strings:
  *"Private: published on the Biorouter marketplace"*, *"Public: published on the Biorouter
  marketplace"*, *"Public: installed from a file, not on the marketplace. Any model can call it."*
- **The `.brxt` install modal** shows the resulting badge above the Install button with:
  *"Extensions installed from a file are always Public. Any model, including commercial models
  hosted outside your institution, will be able to call this extension."* The manual "Add stdio extension" form
  carries the same line.
- **A one-time notice on first launch after upgrade** names any **enabled** extension that is Public
  and declares clinical-looking credentials. On this machine it names `medcp`. It informs; it does
  not block.
- **Gate C's refusal message** names the marketplace as the grantor, so the mental model is taught
  by the system rather than by documentation.

---

## 14. User experience

### 14.1 The badge

**Private is the marked state; Public is the quiet state.** A badge on absolutely everything trains
people to stop seeing badges, which defeats R10's actual goal — knowing which tier you are in
*before* hitting a wall.

- **Private** — filled pill: `--background-muted` fill, `--text-standard` label, small
  shield-outline glyph, the word "Private". It has weight.
- **Public** — hairline pill: 1 px `--border-subtle`, `--text-subtle` label, no glyph, the word
  "Public". Present and readable (R10 satisfied literally) but recessive.
- In the two dense surfaces (tab bar, narrow session-list rows) only Private renders, as a 6 px
  `--text-standard` dot with the full label in the tooltip. No dot means public.

Never colour alone: shape plus glyph plus word.

> **As shipped, the three bullets above are superseded** — the reasoning is argued at length in
> `ui/desktop/src/components/ui/PrivacyBadge.tsx`, which is the authority.
>
> 1. **The glyph is a PADLOCK, not a shield.** The app was marking one fact with three unrelated
>    figures: a padlocked speech bubble on a private conversation (`chatKind.ts`), a shield on a
>    private extension, and a bare dot on a private model. They are the same tier, so they are now
>    the same mark, and `Shield` has been removed from the icon barrel. Do not reintroduce a second
>    privacy glyph for a fourth subject — name the *subject* in words beside the padlock instead.
> 2. **Public is a filled pill, not a hairline one.** No neutral resting border token in this system
>    clears 1.6:1 against either badge ground; in parchment:dark the hairline and its surface are
>    literally the same colour, so the pill above would have shipped invisible.
> 3. **The dense mark is the padlock at 12 px, not a 6 px dot**, and its one surface today is the
>    composer's model chip — the chat lists moved to `ChatKindIcon`, which folds the tier into the
>    glyph. Public still renders nothing there, and deliberately not an *open* padlock: a mark on
>    every public row puts both tiers at the same weight and trains people past both.

**Zero theme work.** A theme is one file (`ui/desktop/themes/<id>.theme.mjs`) and
`npm run themes -- --check` runs inside `lint:check`. The badge uses **only existing semantic
tokens** and adds no new token to any theme file, so it renders correctly across Parchment, Alma
Mater and Roche Limit in both modes with no generator run and cannot fail `check-contrast.mjs`.

### 14.2 Every surface

| Surface | What shows | Where |
|---|---|---|
| Provider grid / settings | Tier badge per provider; the Local / Institutional / Commercial headers stay, now driven by the backend field | `settings/providers/ProviderGrid.tsx`, `providerOrdering.ts` |
| Model picker | Badge per model row; **public rows disabled with an inline reason in a private chat** | `ModelAndProviderContext.tsx` consumers |
| Composer model chip | Badge beside the model name — the most-looked-at spot in the app | `components/bottom_menu/` |
| Chat header | Session badge beside the title | BaseChat header |
| Tab bar | Private dot only | BR-71 Tasks 22-27 |
| Session list / History | Session badge, a "Private only" filter chip, "Made public by you on …" after a declassification | History view |
| Extensions settings | Badge + provenance line, **and a third state computed against the focused session** (§14.5) | extensions settings |
| BAAM Browse (in-app) | Badge per entry, Private/Public facet, and the `live: false` staleness line | `components/baam/` |
| `.brxt` install modal | The resulting badge, above the Install button | `BrxtInstallModal.tsx` |
| **Knowledge view + KB palette** | Base tier chip, and the publicize / privatize control (§5.4) | `components/knowledge/` |
| **The non-private-model disclosure (R15)** | Once, blocking, on the first public bind in an install; then permanently as the Commercial section's line, the model chip's tooltip, and the Settings → Privacy statement | [Task 30A](privacy-tiers-execution-plan.md#task-30a-the-non-private-model-disclosure) |
| `workspace_list` rows and the GUI workspace panel | Badge per row | BR-71 Tasks 12 / 22-27 |
| Landing `baam.html`, `docs.html` | `.tag.private` on the navy ramp; Privacy column | `landing/` |

**Pre-flight, not post-refusal.** In the model picker a public model in a private chat is rendered
disabled with the reason inline; a private extension under a public model is *visible but disabled*
with its reason, so the user knows it exists and why.

### 14.3 Three prerequisites the UX section is built on and which do not exist

These are named tasks, not assumptions. Without them the first user interaction with this feature
is a green success toast over a refused operation.

**P2 — the refusal is currently swallowed and replaced with a success toast.**
`ui/desktop/src/components/ModelAndProviderContext.tsx:235-286` is the one and only model-switch
path. Verified: `updateAgentProvider` is called **without `throwOnError`**, so the generated
`@hey-api` client returns `{error}` rather than throwing; a Gate A refusal would be discarded,
execution continues, `setConfigProvider` rewrites the global default to the refused provider, the
chip flips, and a green toast says the switch succeeded — while the session is still bound to
Versa. That is worse than a wall: the user believes they are on a public model and are not, which
inverts the design's central promise at the most-used surface in the app. Fix: add
`throwOnError: true`; move `setConfigProvider` to *after* a successful session bind; add a `catch`
arm keyed on the privacy code that renders the repair card.

**P3 — the structured 409 does not exist; the route 500s everything.**
`routes/agent.rs:721-727` maps every `update_provider` error to
`(StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update provider: {}", e))`, and the
declared `responses(...)` block at `:677-683` lists 400/401/424/500 — no 409. So a typed
`PrivacyRefusal` mapped to 409 with a JSON body
(`{code:"privacy_barrier", session_classification, provider_tier, available_private_providers}`),
the `responses` entry, and `just generate-openapi && npm run generate-api` must ship in the same
commit as Gate A, or Gate A ships as "Internal server error".

**P5 — the model chip is global, not per-session.** `ModelsBottomBar.tsx:38-45` reads
`currentModel`/`currentProvider` from the app-wide `ModelAndProviderContext`. The per-session
alternative is already dead code: `CurrentModelContext` is created at `BaseChat.tsx:100` and
exported as `useCurrentModelInfo` at `:101`, and **`CurrentModelContext.Provider` is never rendered
anywhere in the tree** — a grep across `ui/desktop/src` returns exactly those two declarations plus
the two consumers in `ModelsBottomBar`. So `currentModelInfo` is always `null` (the file even
carries the comment *"Since currentModelInfo.mode is not working, let's determine mode
differently"*), and the lead/worker label logic at `:117-121` is already partly inert. Open two
sessions on different providers — which BR-71's tabs make routine — and both chips show the same
string. A tier badge attached to that chip would be confidently wrong for every session not on the
global default. Drive the chip from the focused session (`GET /sessions/{id}` already returns
`provider_name` and `model_config`) or delete the dead context.

**P4, related — switching a model in one chat rewrites the global default.** The same function
follows the per-session bind with `setConfigProvider`, which writes `BIOROUTER_PROVIDER`/
`BIOROUTER_MODEL` app-wide; every new chat then starts on that provider via
`restore_provider_from_session`'s fallback. So "pick Versa once in a scratch chat" privatises not
one session but **every session created afterwards**, each ratcheting on its first turn. Decouple:
offer "Also make this my default for new chats" as an explicit checkbox. If that is too large a
behaviour change, the new-chat flow must render the tier of the inherited default *before* the
first turn with a one-click "start this chat on a public model instead".

### 14.4 Refusals that teach

Each names what happened, which two tiers collided, why the boundary exists, and the shortest way
forward.

**Gate A — model switch refused** (rendered from the 409 of P3):

> **Can't switch this chat to Claude Opus.** This chat is **private** — it has been running on
> Versa (versa_azure) since 14 Mar. Private chats only run on private models, so their contents
> never reach a model hosted outside UCSF.
> Available private models: **Versa · GPT-5.5**, **Versa · Claude Opus**, **Llama Server ·
> qwen3.5-4b**.
> To use a public model here, make this chat public first (History → this chat → Make public). That
> permanently exposes its contents and can't be undone.

**Gate B — turn refused after repair failed:**

> **This chat can't run right now.** It's marked private, but it's currently set to Claude Opus
> (public). This usually happens after branching a chat or restarting the app.
> **[ Switch to Versa · Claude Opus ]  [ Choose another private model ]  [ Make this chat public… ]**

The first button is the one-click fix: the wall is a repair prompt, not a dead end.

**Gate C — private extension, public model** (model-facing, returned verbatim as `ErrorData`):

> `ucsfomopagent` is a private extension: it reaches UCSF clinical data, so only private models may
> call it. This session is running on `anthropic` (public). Ask the user to switch this chat to a
> private model — Settings → Models, or the model chip in the composer — and then try again. This
> is a data-protection boundary set by the Biorouter marketplace, not something to work around: do
> not retry with a different tool name, through code execution, or through a resource read.

Three load-bearing properties: **deterministic** (identical on retry, so the model stops looping);
**names the human action** rather than implying the model can fix it; and **explicitly forecloses
workarounds**, following the register established by issue #42's operator-disabled extension gate.

**Never leak content in a refusal.** Refusals go into the model's context, so they name the tool and
the tier only — never a session title (LLM-generated content) and never a working directory.

**Gate C' — the resource and prompt surface — says something different, and must.** The sibling
entry points that reach a server without being a tool call go through
`ExtensionManager::assert_extension_reachable`, which reads an **unknown** name as Private. That is
the one place in this feature where the unknown-name default is inverted, deliberately: the
alternative is permitting a reach at a name the manager could not resolve at all. It means the gate
does not know whether the name it is refusing belongs to a private extension or to nothing, so it
cannot say that it does — and until the 2026-09-10 test drive (finding M18) it said it anyway,
sending a model looking for a private model to reach an extension that did not exist.

`privacy::refusal::private_or_absent_refusal` is the sentence that surface returns instead. It
states the disjunction — a private extension, or a name that is not installed — and returns the
**identical string in both cases**, which is the constraint that rules out the obvious repair:

> `nonexistent_ext` cannot be reached from this chat, which is running on a public model. To a
> public model a private extension — one that reaches data held inside the institution — and a name
> that is not installed here are the same answer, and Biorouter does not say which
> `nonexistent_ext` is. If it is a private extension, ask the user to switch this chat to a private
> model (Settings > Models, or the model chip in the composer) and try again; if it is not
> installed, no model will reach it. …

⚠ **Answering "no such extension" for the absent branch would be an existence oracle.** A public
caller could walk names until one answered differently and learn which private connectors a chat has
loaded — precisely the set Gate E withholds, and precisely the leak finding M6 closed next door in
`read_resource_tool`'s not-found roster. Two renderings of one predicate (`tier_refuses`), each true
where it is used: Gate C proper resolves an installed client before it refuses, so its flat
statement is a fact; Gate C' has not, so its statement is a disjunction.

### 14.5 Two low-cost changes that remove whole classes of surprise

**Relabel the provider groups so the two taxonomies are literally the same words in the same
place.** `providerOrdering.ts:64-82` labels three groups Local (green) / Institutional (indigo) /
Commercial (amber), and `PRIORITY_ORDER` puts `azure_openai: 0` and `aws_bedrock: 1` at the *top*
of the Commercial group. A UCSF user whose Azure OpenAI account is provisioned and paid for by UCSF
IT will read "my institution's Azure" as institutional; it is **public** under this design.
Conversely `ollama` pointed at a hosted SaaS badges **private**. Relabel to **"Private · Local"**,
**"Private · Institutional"**, **"Public · Commercial"**, with one line of card copy each:
Institutional → *"Private because Biorouter recognises this specific UCSF gateway endpoint"*;
Azure/Bedrock → *"Public — Biorouter can't verify where this account's endpoint points."*

> **Note on the Azure copy.** The obvious wording — *"Public: a direct cloud account, even if your
> institution pays for it"* — is not accurate as shipped. `azure.rs:200-205` gives
> `AZURE_OPENAI_ENDPOINT` a **default of `https://unified-api.ucsf.edu/general`**, the same UCSF
> gateway `versa_azure` uses (`versa_azure.rs:23`). A name-keyed tier therefore calls
> `azure_openai` Public even when it in fact resolves to the UCSF gateway — a conservative,
> fail-safe error, but the copy must not claim something the configuration contradicts.

**Shipped, as tabs.** The three groups are the three tabs of the provider catalog
(`ui/desktop/src/components/settings/providers/ProviderCatalog.tsx`), each panel opening with
the same label and the same one-line note this section specifies, still served from
`providerOrdering.ts` to both the catalog and `IngestModelPicker` rather than written twice.
See [the provider catalog](../desktop-ui/provider-catalog.md).

**Show the pairing, not just the extension.** `~/.config/biorouter/config.yaml` enables extensions
**globally** with a single `enabled:` flag; there is no per-session enablement. Under Gate E a user
who enables `ucsfomopagent` sees **Enabled** in Settings while the tool is simply *absent* from
every public-model chat. A static badge describes a property of the extension; what the user needs
is a property of the *pairing*. The Extensions card renders a third state computed against the
focused session — **"Enabled · unavailable in this chat (public model)"** with the one-click switch
— and the composer's extension selector shows private extensions greyed with the same reason rather
than omitting them. Omission is what produces "the OMOP tool is broken".

### 14.6 The opt-out surface, and warned-rather-than-walled

**"Warned rather than walled" is R17's user half, and this section is where it is most visible.**
Every control on this page states a consequence and then lets the user proceed; none of them refuses
a person. The agent half is that **none of these surfaces may be reachable by a model** — the master
toggle in particular, whose typed phrase is a UX guard against an accidental or model-composed
config write and *not* a proof of a human, since a fixed string compiled into the source is
replayable by anything holding the daemon secret. The proof of a human is the one
[DR-16](privacy-tiers-execution-plan.md#decisions-of-record) requires, and there is exactly one of
it.

**Settings → Privacy**, one switch, **on** by default:

> **Privacy tiers** — On
> Chats on private models (Versa, or a local model) stay private: a public model can't read them,
> can't call a private extension, and can't reach your knowledge bases through the shell.

Turning it off requires typing `DISABLE PRIVACY TIERS` and shows all four sentences — the third and
fourth are the ones a user cannot reconstruct for themselves, and are why this is a typed
confirmation rather than a switch:

> This turns off **every** privacy guardrail on this machine, for every conversation.
> Commercial models will be able to call UCSF clinical extensions, read private chat history, read
> and write your knowledge bases, and read your saved chats, memories and Biorouter apps straight
> off the disk through the shell.
> **While it is off, Biorouter stops recording which conversations touched private material.**
> Turning it back on will protect what is already marked private — but it cannot go back and mark
> anything that happened while it was off.

While off, a persistent amber strip sits in the settings sidebar and **every** privacy badge in the
app renders muted with the suffix **"— enforcement off"** — on the session list, the model chip and
the extension rows, not only in Settings.

`POST /config/set_provider` and the `/config/upsert` paths used by `ProviderGuard.tsx` and the
onboarding cards change the *default for future sessions*, not any existing one. Those surfaces show
the tier of what is being set and, if any private session exists, add: *"Existing private
conversations will keep their current model."*

**`LeadWorkerSettings` pre-flights before writing `BIOROUTER_LEAD_MODEL`** — *"Setting a public lead
model will make N private conversations unrunnable until you change it back."* — with the count
computed live, and the resulting composite badged in the chip as **Public**, not as the lead's name.
This control is one `DropdownMenuItem` away from the composer's model chip
(`ModelsBottomBar.tsx:202-208`).

### 14.7 The CLI is a required R10 surface

Every repair affordance above is a GUI card. `biorouter-cli/src/session/builder.rs:479-483` resolves
the provider as `--provider` flag → saved session provider → workflow's `biorouter_provider` →
global default; two of those four can produce a public provider on a private session, and one is a
**shared workflow YAML** pinning `anthropic`, which would now refuse to run in any private session
with nothing explaining why. Minimum CLI surface: (a) `biorouter session -r` prints the session's
tier at start; (b) Gate B's terminal refusal prints the available private models and the exact
re-run command; (c) `biorouter session declassify <id>` runs the same confirmation at the terminal
— which also gives Hidden and Terminal sessions a declassification path (§15.4); (d) a workflow
pinning a public provider fails at *load* with "this workflow pins `anthropic`, which cannot run in
a private session", not mid-turn.

### 14.8 Discoverability

One first-run tip the first time a private model is selected — *"Chats on private models stay
private. Public models can't read them, ever."* — dismissible, shown once. No tour, no banner, no
persistent nag. The badges carry the rest.

---

## 15. Migration

### 15.1 Schema

`privacy_tier TEXT NOT NULL DEFAULT 'public'` and `privacy_reason TEXT`, plus
`classification_audit`, all in the same migration.

**The migration number is not load-bearing, and must not become load-bearing** (execution plan
O10). `main` is at `CURRENT_SCHEMA_VERSION = 16`. The BR-71 worktree
(`feat/br71-workspace-control`) already has **17** with a written, working
`17 => ALTER TABLE sessions ADD COLUMN parent_session_id TEXT`. Whoever merges second silently
re-uses a number, and a database that already ran the other branch's 17 skips the second feature's
arm entirely — the exact incident `run_migrations`' own comment records for v11–v14. So this work
ships a **shape-guarded numbered arm plus an unconditional `ensure_privacy_schema`**, following the
`ensure_session_incarnation_schema` precedent (called from `reconcile_loop_schema`, itself invoked
*after* the version loop). With that, merge order is free in both directions and neither branch has
to wait on the other.

### 15.2 Backfill — fails open, by decision

```sql
UPDATE sessions SET privacy_tier = 'private', privacy_reason = 'backfill:' || provider_name
 WHERE provider_name IN ('versa_azure','versa_bedrock','llamacpp','ollama');
```

A fail-*closed* backfill (NULL provider + ≥1 message ⇒ private) was rejected: a user who has only
ever used a commercial provider would find a large slice of their history marked private on first
launch, refused on the model they normally use, with only an irreversible declassification as the
exit, one chat at a time. The column default is `'public'` and the backfill catches what the data
can prove.

Four counts logged at `info!` so the size of the annoyance is measurable on day one:
`backfilled_private`, `backfilled_public_named`, `backfilled_unknown_provider`, `backfilled_empty`.

**The residual, stated plainly rather than buried:** `provider_name` records the *last* provider,
not every provider. A session that ran on Versa and was later switched to a public model backfills
as public even though its transcript contains private-model work. There is no transcript scan and
there will not be one — it would be slow, heuristic, and wrong in both directions. Mitigation is the
badge plus one release-note line:

> *"Chats from before this version are marked by the model they were last using. If an older chat
> contains work you want kept private, switch it to a private model — it will be marked private from
> its next turn on."*

See §15.5 and §16 for what the backfill actually does to a real machine on day one.

### 15.3 The fail directions, and why they differ

| Situation | Direction | Reasoning |
|---|---|---|
| Migration backfill | fail **open** (public) | one-time, deliberate, user-visible; the alternative bricks a history the user cannot un-brick |
| Runtime read — column missing from a projection, unparseable value | fail **closed** (private, with `error!`) | a bug, not a decision. Every session shows a Private badge: immediately visible, immediately fixed, safe meanwhile. The tolerant `try_get(…).ok().flatten()` convention would silently read public, and `branch_point_msg_uid` already being absent from `list_sessions_by_types`'s projection is the live proof that a projection gets missed. |
| Import of a session with no `privacy_tier` | fail **closed** (private) | an imported transcript of unknown provenance is treated as sensitive; unlike migration, there is no local evidence to reason from |
| Unknown provider | Public | fail-**safe**, not fail-open: Public is the *less* privileged tier |
| Unlisted extension | Public | fail-open, **operator ruling R11(ii)**, isolated to the final `ProviderTier::Public` of one function — `classify_extension_entry`, which `classify_extension(name)` now delegates to — with one const and one comment naming the ruling, so reversing it later is a one-line change rather than an audit |
| Any gate's lookup fails | refuse | encoded as a refusal inside `Ok(...)`, never as `Err` |
| ~~NULL `parent_session_id`~~ | ~~`other` ⇒ read-only~~ | **moot.** R6 is retired and lineage is no longer an input to any rule, so a NULL parent decides nothing. The column itself remains, for the interface's parent/child grouping. |

### 15.4 Sessions, configs and extensions

**Sessions in flight when the app updates:** none survive. The daemon restarts, agents are rebuilt,
and every session resolves its provider through `restore_provider_from_session` on resume, which now
passes Gate A. Where the backfilled tier and the stored provider disagree, Gate B's repair path
rebinds from the row silently if it can.

**Existing configs:** one migration, and it touches `config.yaml` only on the installs that set the
opt-out. No key renamed, no extension entry rewritten.
[DR-22](privacy-tiers-execution-plan.md#dr-22--the-master-switch-does-not-live-in-configyaml) moves
the master switch out of `config.yaml` into `privacy-tiers.json`, so the first start after the
upgrade records this install's answer there and removes `BIOROUTER_PRIVACY_TIERS` from `config.yaml`
**if it was set** — carrying an existing `off` across rather than silently resetting it. An install
that never set it, which is nearly all of them, gets the new record and its `config.yaml` is not
written at all. Absent the key the answer is the default (`on`). One sharp edge to pre-empt: a
user with `BIOROUTER_LEAD_MODEL` set to a public lead over a private worker now holds a **Public**
composite, so their private sessions become unrunnable until they change it — hence the pre-flight
warning in §14.6.

**Existing installed extensions:** no migration; the tier is resolved at admission from
`compiled_baseline ∪ last_good_fetch`. On first run after upgrade there is no stored fetch, so the
compiled baseline governs — `ucsfomopagent` and `cdwagent` are Private immediately, offline, with no
network. Nothing is uninstalled or disabled.

**Sessions outside History:** `list_sessions` filters to `session_type IN ('user','scheduled')`
(`session_manager.rs:3537`), so a private Hidden or Terminal session would be enforced forever with
no GUI declassification surface. **Do not add a "System sessions" filter to History** — on this
machine that would surface **720** hidden sessions (668 public, 52 private) into a user-facing list,
a regression traded for an edge case. (Re-measured 2026-08-01 alongside §16; the figure was 511 when
this section was first written, and the bucket grows continuously — the argument only gets stronger.) Use the CLI escape hatch instead (`biorouter session declassify
<id>`, §14.7), which works by id regardless of `session_type`.

**Rollback:** downgrading the app leaves the columns in place and ignored, and
`classification_audit` inert. Nothing is moved, re-indexed or rewritten. The one-way door is the
backfill; rolling forward re-runs nothing, since the migration is versioned.

### 15.5 Day one must be shown, not discovered

1. The first-run notice states the **actual counts, computed from the user's own DB**, over the same
   population History shows — user + scheduled sessions with at least one message (§16) — for example
   *"642 of your 2,587 conversations are now marked private because they last ran on Versa or a
   local model."* Those are this machine's real figures on 2026-08-01; compute, never hardcode.
2. Grouped declassification extends to **all** `backfill:*` reasons, not only `backfill:unknown`,
   with a review-by-provider list (§12.6).
3. Run the backfill and show the counts **before** enforcement begins. One launch of "here is what
   will change" beats a week of unexplained refusals.

### 15.6 Migration test gates

Fresh DB at 17 → column exists, defaults public. DB at 16 with sessions on `versa_azure`,
`anthropic`, NULL → private, public, public. DB at 16 with `messages_fts` absent → the LIKE path is
filtered too. Round-trip: `privacy_tier` survives, and an attempt to lower it through the builder is
a no-op. `copy_session`, `diverge_session` and `import_session` of a private session each yield a
private branch carrying `provider_name`. Import with no tier yields private.

---

## 16. The honest cost

**Private is the default state on a real machine, not the exception.** Measured directly from
`~/.local/share/biorouter/sessions/sessions.db` (aggregate `provider_name` counts only, no message
content):

| session_type | would backfill private | public | NULL provider | total |
|---|---:|---:|---:|---:|
| **user** | **963** | 588 | 2,831 | **4,382** |
| scheduled | 121 | 76 | 0 | 197 |
| hidden | 52 | 668 | 0 | 720 |
| sub_agent | 42 | 33 | 0 | 75 |

**963 of the 1,551 user conversations whose provider is known — 62.1% — go private on first
launch.** 875 of those are `versa_azure` (733) or `versa_bedrock` (142); 88 are `llamacpp` (48) or
`ollama` (40).

> **Re-measured 2026-08-01, four days after the design was written, and the numbers moved a lot.**
> The **NULL-provider** bucket for `user` sessions went from 29 to **2,831** — two orders of
> magnitude — so the fail-open residual is far larger than first reported. Of those 2,831,
> **1,509 have at least one message**: 1,509 real conversations of unknown provenance that backfill
> **public**. Separately, History shows fewer rows than the raw counts imply, because
> `list_sessions_by_types` uses `INNER JOIN messages m ON s.id = m.session_id`
> (`session_manager.rs:4279`), so empty sessions never appear. The number the first-run notice must
> quote is user+scheduled **with at least one message** — **2,587** on this machine today.
> Re-measure at implementation time; this moved by a factor of three in four days.

And it is not an accident of one machine. `ProviderGuard.tsx:177-186` orders the onboarding cards
**Llama Server → Ollama → Institutional → Commercial** — three of the four first-run cards are
private-tier — and CLAUDE.md states the ordering is deliberate. A new UCSF user's default,
zero-setup path produces private sessions. **Every friction below should be re-costed at that
multiplier before it ships.**

Where this annoys someone who has done nothing wrong:

1. **You can never move a private conversation to a public model.** Not with a warning. A researcher
   who used Versa once and now wants Opus on the same thread must stay on Versa, start a new chat,
   or declassify. This is the design's central cost and it is deliberate. The
   ratchet-at-first-turn correction (§6.1) removes the mis-click version, and the graded
   confirmation (§12.4) removes the friction from the `turn:`-only majority.
2. **Turning on lead/worker with a public lead locks you out of your private chats.** `least()`
   makes the composite Public, so one global setting one click from the composer affects unrelated
   conversations, which reads as a bug unless the panel pre-flight-warns. At a 71% private base rate
   this is not a footnote.
3. **Right after upgrade, an orchestration loses control of its existing children.**
   `parent_session_id` only exists going forward, so every pre-upgrade subagent is `other` and
   read-only to its own parent. No clean mitigation; it resolves as new work is spawned.
4. **A remote llama.cpp or Ollama badges Private with no way to say otherwise** — the one place this
   design over-trusts. See §17.
5. **A private extension simply vanishes from a public model's tool list** (Gate E). Without the
   pairing-aware state in §14.5 this reads as "the OMOP tool is broken".
6. **Declassification is one chat at a time** for `mcp:`-reason sessions.
7. **A shared workflow pinning a public provider stops working in private sessions** with nothing
   explaining why, unless §14.7(d) ships.
8. ~~**A public chat cannot view `config.yaml` through a tool.**~~ **Not paid — [DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store) descopes
   §9.5.** A public chat reads `config.yaml` like any other file, so "why isn't my extension loading"
   debugging stays where it is. The corollary is the one §9.5.2 warned about and this design now
   accepts: **a master switch a public model can read and edit is not a switch**, so the toggle's
   integrity rests on nothing but the file's own permissions — owner-only (`0600`) since H3 of the
   2026-09-10 test drive, which keeps other accounts out and is no obstacle to an agent running as
   the owner. What H3 added is that the edit cannot be *silent*: an OFF record is announced at load
   and in the app, and one no door stamped reads *turned off outside the app* (§10.6).
9. ~~**On Windows and on Linux without bubblewrap, a public chat loses the five tools that spawn a
   child process.**~~ **Not paid — [DR-17](privacy-tiers-execution-plan.md#scope-ruling--dr-17-narrows-this-plan-to-the-session-store) descopes §9.5.** `developer__shell` and its four
   siblings keep working for a public-capability chat on every platform. This was the single largest
   usability price in the design.
10. **The barrier is narrower than a reader of an earlier draft would expect**, and that is now the
   honest cost. A public model with a shell can read the session database and anything a private chat
   left on disk. R15's disclosure is the whole of the mitigation, which is why it is a requirement
   with a task and a gate rather than a paragraph in §14.
11. **A knowledge base a private chat touched is unreadable from public chats until the user
   publicizes it** — one click, but a click they must discover. [DR-18](privacy-tiers-execution-plan.md#dr-18--the-knowledge-base-tier-is-user-controllable-and-a-private-session-creates-a-private-base) adds the control; the
   confirmation names how many pages it releases, and releasing cannot be undone for content already
   read.

If these are not budgeted, the honest prediction is that the first user with 900 private chats and
a commercial subscription tries to turn the feature off — and discovers the R7 opt-out covers only
Gate C, i.e. not the part annoying them. They file a bug instead. (R7 is a master switch now; the
prediction stands for whatever the next narrowest reading of it turns out to be.)

---

## 17. Open questions needing an operator ruling

1. **Does a mixed lead/worker composite ratchet the session?** R3 says "switched to a private model
   even once → private permanently", and a private-lead/public-worker composite *contains* a private
   model. This design says it does **not** ratchet, because `tier = least` and the transcript has
   already gone to the public worker, and because ratcheting on `max` would make the bind gate
   refuse that same composite on the next resume. Using one reduction for both the gate and the
   ratchet is what makes `capability ≥ classification` provable by induction. **This is the single
   place the letter of a requirement was not followed.**
2. **Is the spawn-downgrade an approval or a refusal?** R4 permits it, so it is an approval showing
   the task prompt. But the prompt is written by a private-context model and is the only leak
   vector, and it is the one control a planted `PermissionRequest` hook could bypass — hooks load
   from `~/.config/biorouter/config.yaml` and, with `allow_project_hooks`, from
   `.biorouter/hooks.yaml` in the working directory, both writable by an agent with `text_editor`.
   An operator wanting zero risk makes it a `Deny`.
3. ~~**Does the R7 opt-out really stop at Gate C?**~~ **RULED — it stops nowhere.** Turning the
   master switch off in Settings → Privacy disables every gate, the ratchet and both read-deny
   layers (§10.6). The cost is that nothing is recorded while it is off and re-enabling does not
   reclassify the gap; the typed confirmation states it. ⚠ The `BIOROUTER_PRIVACY_TIERS=off`
   spelling this entry used to carry names **no channel that exists**: hardening measure (1)
   bypasses `Config::get_param`'s environment branch, so the variable was never read, and
   [DR-22](privacy-tiers-execution-plan.md#dr-22--the-master-switch-does-not-live-in-configyaml) retired
   the `config.yaml` key it looked like. Settings → Privacy is the one door.
4. **Is the first cross-tier write approval remembered per (caller, target) or per call?**
   Per-pair-per-session-lifetime was chosen because a confirmation on every steer of a public worker
   is miserable and would be clicked through.
5. **Institutional Ollama versus hosted Ollama SaaS.** R1 says self-hosted *or* institution-hosted
   is private, and config cannot tell a lab GPU box at `OLLAMA_HOST=gpu.lab.ucsf.edu` from a hosted
   SaaS. "Non-loopback stays Private, the badge names the resolved host" is a **false-private** — the
   one place this design is permissive rather than restrictive. Certainty needs a
   `BIOROUTER_PRIVATE_HOSTS` allowlist, a new concept deliberately not added (it would need its own
   protection and settings UI).
6. **Should `versa_azure` get its own config keys?** It shares all three `AZURE_OPENAI_*` keys with
   the public `azure_openai` provider, whose shipped default endpoint is the same UCSF gateway. The
   demotion rule catches the dangerous direction, but it means a user can *lose* their private tier
   by configuring an unrelated provider.
7. **Should the compiled-in private baseline be a signed registry snapshot?** Signing would let a
   *downgrade* be trusted offline. Today the union rule means an extension can only ever gain a
   private badge without a fresh fetch — safe, but a genuine reclassification-to-public needs
   connectivity.
8. **Who is "who" in the declassification record?** The app is single-user, so the local OS username
   is recorded. On a shared lab machine that is right; in a multi-account setup it is not, and there
   is no user identity in the product to record instead.
9. **Skills (R12) carry no classification, which leaves three gaps.** (a) A skill authored while a
   private chat was open can embed pasted private text and is then readable by every session and
   publishable to the marketplace. (b) A skill can instruct the model to call `ucsfomopagent` —
   harmless in effect because Gate C refuses at dispatch, but the steering is unblocked and produces
   confusing refusals. (c) BR-71 Task 15 lets one session add skills to another. The v1 mitigation
   is a line in the skill-creation UI; closing (a) needs skills to carry a classification, which
   contradicts R12.
10. **`ActiveWorkItem.title` is cross-session content and predates all of this** — derived from a
    subagent's task prompt and surfaced process-wide with a session id. The visibility rule is
    applied to it, but it is exposed only via `GET /active_work` for the GUI (the model-facing
    `workspace_read_conversation` / `workspace_watch` are session-scoped), so it may deserve its own
    fix rather than riding this one.
11. **`POST /agent/call_tool` remains inspector-free.** This design is correct either way because
    the barrier is in the extension manager, but the route is a standing hazard for every *future*
    inspector-based control, including BR-71's.
12. **Should the daemon's HTTP API authenticate a caller that is on the same machine?** §9.5.5: the
    API secret is recoverable from the daemon's own environment, so the header check stops a remote
    caller and not a local one. §9.5's barrier covers the largest local route because that route
    dispatches through the same choke point; it does not cover the routes that return private
    content without running a tool, of which `GET /diagnostics/{id}` is the widest. A per-caller
    credential the daemon does not hand to its own children is the shape of the fix, and it is
    probably the same fix as the app socket's.
13. **Should the Agent Drafter app socket be authenticated by something a local client cannot
    obtain?** §9.5.5: `agent_drafter__list_apps` hands a public model every app id, and an app id is
    all the unauthenticated `GET /apps/{id}/` page needs to yield that app's socket token. The
    alternative — reclassifying `agent_drafter` as Private in §10.3 — closes it at Gate C and takes
    the whole drafter workflow out of every public chat, which is a much larger behaviour change and
    needs a ruling rather than an implementation decision.

---

## 18. Relationship to BR-71

BR-71 ([issue #30](https://github.com/BaranziniLab/biorouter/issues/30),
[design](docs/agent-loop/designs/agent-workspace-control.md),
[44-task execution plan](docs/agent-loop/designs/br71-execution-plan.md)) is the plan of record for
agent workspace control and glass-box subagents. Its design doc's status line reads *"Current —
proposal only; nothing below is implemented."*

**This work must land before BR-71's cross-session tools ship.** Task 15's `workspace_set_tools
{ provider, model }` changes **another session's** provider and, per the plan's own table, takes
effect next turn — a first-class, agent-callable path to attach a public model to a private session.

### 18.1 Prerequisites, each of which justifies itself without BR-71

| # | Item | Standalone justification |
|---|---|---|
| P1 | `ProviderTier` / `Classification`, `ProviderMetadata.tier`, `Provider::tier()`, `LeadWorkerProvider::tier() = least` | foundation |
| P2 | **Gate A** + the monotone `CASE WHEN` | also fixes `ClientFrame::ModelSelect`, verified today to swap a session's provider from a browser page with no check of any kind |
| P3 | **Gate B** + the `Agent::provider()` assertion | LRU rehydration and the `restore_provider_from_session` global fallback are present-tense downgrades |
| P4 | **Gate C** + its resource/prompt siblings, and **Gate E** | R7 standalone; covers `/agent/call_tool` and `execute_code`, which no inspector reaches |
| P5 | **Gate D** — both chatrecall builders + the LOAD-mode check | today's most direct cross-session read path; LOAD has verified zero filtering |
| P6 | `create_session` carries `privacy_tier` + `provider_name` + `model_config` for all three copy paths | a live laundering path, verified |
| P7 | The generator's second and third outputs + `--check` | the badge cannot exist in Rust without it |
| P8 | A1, B3 from §9.3 | the two findings that would have let a public model read private content on day one; the scrubs named in each have since shipped (#57, #58, #63). **Neither finding is closed by that.** A1's fixes (2) and (3) are open, and AR-11 measures the secret still recoverable from the *parent* process on both platforms; B3's global-memory injection is gone, but local memories are still inlined in full, so a private session's local note reaches every later session in that directory |

### 18.2 Additions to named BR-71 tasks

**Task 1 (`sessions.parent_session_id`, migration 17).** Add `privacy_tier` + `privacy_reason` in
the same DDL, the same `ALTER TABLE` arm and the same backfill pass, plus `classification_audit`.
Task 1's Step 3 already enumerates every touchpoint — `CURRENT_SCHEMA_VERSION`, the `Session`
struct, the fresh-DB DDL after `diverged_from TEXT,`, the migration arm, the row mapper, the INSERT
column list, the builder field/setter/emission, and both explicit SELECT projections. This
duplicates that checklist exactly, and its warning that a missed SELECT "compiles and silently reads
`None`" is precisely why this design's reader is fail-closed.

**Tasks 4 / 12 (`workspace_list`).** Projection includes `privacy_tier`; rows carry the badge; a
public caller's list **omits** private rows. Task 4 already amends this projection for
`parent_session_id` and `session_type`.

**Task 10 (`WorkspaceMutationInspector`).** Three additions, one of which changes a verdict class:

- The plan already specifies a confirmation for a provider switch, reason *"switches this
  conversation to provider '{provider}', which sends its whole history to that provider's
  endpoint"* (`br71-execution-plan.md:4829`) — the exact sharp edge arriving in the exact code path.
  **When the target is private and the provider public, `RequireApproval` must become `Deny`**: a
  confirmation is the wrong control for a boundary the user may cross only through R9. The
  precedence machinery makes it free — `apply_inspection_results_to_permissions` removes a `Deny`'d
  request from both `approved` and `needs_approval`, and `Allow` is explicitly a no-op, so `Deny`
  beats Auto mode and any stored always-allow with no new machinery.
- The spawn-downgrade confirmation and the first cross-tier write confirmation ride **this**
  inspector, not a new one.
- **Register the inspector's name in `handle_denied_tools`.** Without that one arm, every
  cross-session privacy refusal tells the model the user declined, which is false and invites an
  identical retry. Needed only for Task 10's `Deny`s — Gate C bypasses this machinery entirely.

**Task 15 (`workspace_set_tools`).** Three constraints, all resolved off lookups the task already
performs: `{ provider, model }` **must call `Agent::update_provider`** rather than reimplement the
persist; `add_extensions` naming a private extension gains the tier refusal beside the issue-#42
operator-disabled gate the plan already wires in at `get_extension_entry_by_name`; and a
private→public invocation raises the first-crossing approval. ⚠ This paragraph also said *"lineage
gates the whole tool to `self`/`child`"* — that clause is retired with R6; the tool is gated by the
tier alone.

**Tasks 17 / 19 / 13 / 14 / 16 / 24 (the tool surface).** The matrix in §7 covers `workspace_list`
(12), `workspace_read_conversation` (13), `workspace_send_prompt` (14), `workspace_set_tools` (15),
`workspace_close` (16), `workspace_watch` (17), the merged `subagent` (19) and `workspace_open` (24).

**Task 32 (spawn-context persistence).** Stamp `parent_session_id` **inside
`create_subagent_session`'s INSERT** rather than after `override_system_prompt`, and stamp
`privacy_tier` in the same statement. Today the child row is created with only
`(working_dir, name, SessionType::SubAgent)` and its provider lands later — and for a *background*
subagent the whole stretch runs in a detached `tokio::spawn`, so a daemon kill leaves a durable
`SubAgent` row with no provider and no parent. One INSERT closes both windows.

**Task 36 (the subagent guard).** The existing refusal (a `SessionType::SubAgent` session may not
call the subagent tool) is the shape and the location; the tier check belongs beside it. (This read
"the lineage and tier checks" until R6 was retired. The `SubAgent` refusal itself is a *different*
one-hop rule and stays.)

**Tasks 22-28 (GUI).** Tab-bar dots, workspace-row badges, provenance chips, set-tools toasts.

### 18.3 What this asks BR-71 to change: nothing structural

Not its tool surface, schemas, UI, task ordering or worktree strategy. Every intersection above is
one column in a DDL edit already happening, one branch in an inspector already being written, or a
constraint on which existing function a handler calls. That is deliberate: **a policy that forces a
re-plan of an approved, about-to-be-built feature does not get built.**

### 18.4 Independent and parallelisable

- The whole `landing/` change (§13). Ships on its own cadence via `deploy-landing.yml`; enforcement
  runs off the compiled-in const, so the website blocks nothing.
- The last-good-fetch persistence and fetch timeout in `main.ts`; surfacing `live: false`.
- Replacing `provider_class` in `routes/apps.rs` with the shared tier function (a genuine bug fix —
  ship it separately so it is not gated on this design landing).
- `configure_worker_provider`'s missing parent-inheritance and `AgentManager::get_or_create_agent`'s
  default-provider fallback.
- The knowledge-macro `ModelRef` gate and the prompt-hook provider check (v1 emits a load-time
  warning; the hard skip is v1.1). Both run against session content on an unrecorded provider:
  `crates/biorouter/src/hooks/mod.rs` `resolve_prompt_provider`,
  `crates/biorouter/src/agents/knowledge_tool.rs` and `routes/knowledge.rs`.
- The `UserConfirmation` ZST and the repo-grep tests — belt over braces; `privacy_reason` plus a
  `warn!` gives auditability on day one.
- `classification_audit` as a table (the same reasoning).

---

## 19. Suggested implementation order

1. **`chatrecall` LOAD-mode guard, and the `platform__ingest_conversation` guard beside it.** One of
   these is five lines; both are fully-open cross-session reads in the product today (§2.3 item 2),
   and `ingest_conversation` is the worse of the pair because it *writes what it read* into a
   machine-wide knowledge base. Ship the two together, on their own, ahead of everything.
2. **A1's remaining half (the secret off the environment entirely)** and
   **B3 (refuse a global `remember_memory` from a private-capability session)**. Both halves of A1's
   scrub — shell and extension spawn — and both of B3's original channels have already shipped as
   #57, #58 and #63, so **neither of the two original repros still works**. What is left of A1 is
   not merely cosmetic hardening: AR-11 measures the secret still recoverable from the daemon's own
   environment through the *parent* process (`ps -Ewww -p $PPID` on macOS, `/proc/self/environ`
   in-process on Linux), which no key list can filter, so taking the secret off the environment is
   what turns the filter into a guarantee. And B3 keeps one live channel of its own — local
   memories are still inlined in full into every session opened in that directory
   ([open question 14](privacy-tiers-execution-plan.md#open-questions)).
3. **P1 + P2 + P3 (types, Gate A, Gate B)** together with **the typed 409 and the `throwOnError`
   fix**, in one commit. Gate A without them ships as "Internal server error" over a green success
   toast.
4. **B1** — carry-over on `create_session`, with the call-site enumeration test.
5. **Gate C + siblings + Gate E** (with B6's precomputed allowed-set and shared prefix resolver),
   plus the generated `registry_private.rs` and its `--check`.
6. **Gate D** — both builders, with the required-constructor-parameter threading.
7. **Migration + backfill + the day-one notice** (§15.5).
8. **Badges and the per-session model chip (P5)**, then declassification.
9. **C2, P6, the CLI surface**, then the landing site.

---

## 20. Claims verified against the tree, and corrections made

Every anchor below was verified at `708390d8` and has since moved; see
[the execution plan's drift table](privacy-tiers-execution-plan.md#read-this-before-you-chase-a-line-number).
**The bare line numbers this section used to carry have been removed rather than re-verified**, because
a stale number here is worse than none — it reads as a citation and is not one. What survives is the
record of *what* was checked, anchored on the symbol, which is the only part that was ever durable.

Verified by reading the code at `main` (708390d8) before writing: `CURRENT_SCHEMA_VERSION = 16`; the
eight-field `ProviderMetadata`; `providerOrdering.ts`'s two `Set`s; `update_provider`'s
swap-before-persist and its status as the **sole** writer of both `Agent::provider` and
`sessions.provider_name`; `restore_provider_from_session`'s `Config::global()` fallback;
`chatrecall` LOAD's complete absence of filtering; both `chat_history_search` builders' existing
`INNER JOIN sessions s` and SQLite-applied `LIMIT ?`; every `extension_manager.rs` symbol
(`filter_tools`, `get_all_tools_cached`, `get_client_for_tool`, `read_resource_tool`,
`read_resource`, `get_ui_resources`, `list_resources`, `dispatch_tool_call`, the SecretGuard
comment, `list_prompts_from_extension`, `list_prompts`, `get_prompt`, `add_extension`, `add_client`,
`add_inprocess_server`, the `Extension` struct's six fields, the `Frontend` refusal);
`copy_session`/`diverge_session`/`import_session` each hand-rolling a builder that omits
`provider_name`; `provider_class`'s exact-equality inversion; `routes/agent.rs`'s 500-only error
mapping and its `call_tool` inspector bypass; `ClientFrame::ModelSelect`'s unchecked
`update_provider`; `ModelAndProviderContext.tsx`'s missing `throwOnError` and its global
`setConfigProvider` write; `CurrentModelContext` never being rendered; `memory/mod.rs`'s
global-memory system-prompt injection (**since deleted by #58 — see §9.3 B3**);
`DEFAULT_SECRET_PATTERNS`; the secret key in
`biorouterd.ts`'s `additionalEnv` and the absence of `env_clear` in both `shell.rs` and the stdio
spawn (**both since scrubbed by #57 — see §9.3 A1**);
`LeadWorkerProvider::get_name()` returning the lead's name; `factory.rs`'s
`BIOROUTER_LEAD_MODEL` intercept; the `COALESCE` accumulation precedent in `session_manager.rs`;
the `messages_fts` contentful DDL; `registry.json` (version 1, 37 extensions, 129 skills, ten keys,
no classification field, `spokeagent-0.4.1` the only version-suffixed id, `medcp` and `msbaseagent`
absent); `medcp` enabled locally with `CLINICAL_RECORDS_*`; `baam.html`'s render functions and
`.ext-tags` clipping; `shared.css`'s tag variants; `build-registry.mjs`'s `data-license` idiom;
`docs.html`'s hand-written table; `ProviderGuard.tsx`'s onboarding card order; `scheduler.rs`'s
global-config provider; the CLI plan-mode `get_reasoner` path; `apply_settings_overrides`'s
name-only extension narrowing; BR-71's 44-task execution plan including Task 1's migration 17, Task
10's confirmation reason string, and Task 17's `workspace_watch`; and the session-provider aggregate
counts (**re-measured 2026-08-01 — see §16**).

**Corrections made to the input material:**

| Claim as received | Correction |
|---|---|
| The session DB is at `~/.config/biorouter/sessions/sessions.db`, so add `**/.config/biorouter/**` to the secret floor | It resolves through `Paths::data_dir()` — `~/.local/share/biorouter/sessions/sessions.db` on this machine. A `.config`-scoped pattern would not match. Corrected in §9.3 A2. |
| The public `azure` provider shares three config keys with `versa_azure` | The registry name is `azure_openai`, and its shipped `AZURE_OPENAI_ENDPOINT` **default is the UCSF gateway itself** (`azure.rs:204`). The demotion rule is unchanged, but the proposed UX copy ("a direct cloud account, even if your institution pays for it") would be inaccurate. Corrected in §14.5. |
| The knowledge active-KB is a global file, so a public session inherits a private session's KB by default | `paths.rs:66-71` adds `.active-kb-sessions`, one file per session, with `.active-kb` as the primary fallback. The KB-as-shared-sink attack stands (any session may name any KB); the "inherits by default" framing does not. Corrected in §9.3 B4. |
| The design's fix list covers `copy_session` | `diverge_session` (`:4204`) is the primary GUI diverge path and does **not** call `copy_session`; `import_session` (`:4096`) is a third. Corrected in §9.3 B1, and the fix moved onto `create_session`. |
| History would gain a "System sessions" filter so Hidden sessions have a declassification path | That surfaces every Hidden session on this machine into a user-facing list — 511 when the objection was raised, 720 as of 2026-08-01 (see the row below). Replaced with the CLI `declassify <id>` escape hatch. §15.4. |
| Hidden sessions: 435 public / 52 private (487 total) | Re-measured: 459 public / 52 private (511 total). Re-measured again 2026-08-01: 668 public / 52 private (720 total) — the point is that this bucket grows continuously, not that any one figure is right. §16. |
| `provider_class` at `routes/apps.rs:2061-2098` | The function is at `:2089`; `:2061` is the start of its doc comment. |
| `filterExtensions` at `baam.html:3906` | `:3909`. |
| BR-71 is approved and about to be built | Its design doc's own status line reads *"Current — proposal only; nothing below is implemented"*, and issue #30 is open. The dependency argument is unaffected; the wording is. §18. |
| The BR-71 design doc lists the workspace tool surface | The design doc does not mention `workspace_watch`; the **execution plan** does (Task 17), and it is the authoritative task list. Both are cited. |

Nothing in the input material was found to be wrong in a way that changes a gate placement or a
matrix cell.

---

## Related documentation

- [Agent workspace control and glass-box subagents (BR-71)](docs/agent-loop/designs/agent-workspace-control.md) — the feature this must land ahead of.
- [BR-71 execution plan](docs/agent-loop/designs/br71-execution-plan.md) — the 44-task plan whose Tasks 1, 4, 10, 12–17, 19, 22–28, 32 and 36 this design amends.
- [Secret storage](docs/security/secret-storage.md) — the credential model the §9.3 A1 fix touches.
- [Tool routing](docs/agent-loop/tool-routing.md) — the chatrecall/workspace split Gate D sits inside.
- [Subagents](docs/agent-loop/subagents.md) — the inheritance behaviour §8 gates.
