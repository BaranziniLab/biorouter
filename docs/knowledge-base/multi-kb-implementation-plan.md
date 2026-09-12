# One knowledge-base set per session, one primary

> **What this is.** The task-by-task implementation plan that collapsed the Knowledge feature's two overlapping selection axes into one set plus one primary pointer, and fixed the six pre-existing bugs the survey found on the way (GitHub issue #45).
> **Status:** Implemented — landed on `feat/multi-kb` (Tasks 1-24), verified against `main` @ `a01be9b7` (v1.88.6).
> **Audience:** developers working on the Knowledge subsystem.

Before this change a session had two selections that could disagree: a plural `hidden_kbs` list that decided what `kb_search` and the chat chip saw, and a singular `active_kb` that decided what the Knowledge view, the KB-less writes and the single-base reads used. Nothing kept them consistent, so a base could be "active" while hidden from the very session it was active in. The design below keeps the plural axis exactly as it is — it already works — and reduces `active_kb` to an explicit **primary** pointer with one enforced invariant: the primary is always a member of the set. Read the Design decisions section (D1-D12) first; each records the alternative that lost and why.

> **For agentic workers:** Recommended: Follow the subagent-driven-development skill (recommended) or executing-plans skill to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the two overlapping knowledge-base axes into one. A session's **visible set** *is* its knowledge-base set — every base in it is searchable, readable and usable — and the single `active_kb` becomes an explicit **primary** pointer: the write target for KB-less mutating calls and the default subject for single-base reads and the Knowledge view. Six pre-existing bugs uncovered by the survey are fixed on the way. No storage format changes, so an older on-PATH `biorouter` keeps working.

**Architecture:** One axis + one pointer. The axis already exists and already works — `hidden_kbs` (plural, ordered, session-scoped with a machine-wide fallback), driving `kb_list_bases`, the cross-base `kb_search` fan-out with per-hit `kb_id` attribution, and the chat KB chip. This plan adds nothing to it except the ability to say "explicitly nothing hidden". The pointer is today's `active_kb`, renamed to `primary` throughout the code and given one enforced invariant: **the primary is always a member of the set.** One is never *invented* for a session that has none — a session that has not chosen a primary has none, and a KB-less write then fails with an error that names the candidates and the exact command to fix it. A primary the session already had is a different matter: when a set change orphans it, the writer repairs it deterministically and persists the repair, so the user can see what happened.

**Tech Stack:** Rust 1.92 workspace (`crates/biorouter-mcp` knowledge module, `crates/biorouter-server` Axum + utoipa, `crates/biorouter-cli` clap), Electron + React 19 + TypeScript (Vite, Vitest, `@testing-library/react`), OpenAPI-generated TS client (`@hey-api`).

**Verified against:** `main` @ `a01be9b7` (v1.88.6). Every file:line anchor below was re-read in that tree.

---

## Design decisions

Each decision is **resolved**. The rejected alternative is named so a future reader knows it was considered and why it lost.

### D1 — Merge the axes: the visible set *is* the session's knowledge-base set

**Decision.** There is no separate "active set". The bases `kb_list_bases` returns for a session are the bases that session uses: `kb_search` with no `kb_id` already searches all of them and tags every hit with its `kb_id` (`crates/biorouter-mcp/src/knowledge/server.rs:291` `search_visible_bases`, called at `:620` and `:653`); the chat chip already multi-selects them (`ui/desktop/src/components/bottom_menu/BottomMenuKnowledgeSelection.tsx`, backed by `get_hidden_for_session_or_persisted`, `service.rs:1043`). Nothing about that changes. What changes is that we stop pretending a second, narrower "active" collection exists.

**Rationale.** Cross-KB search is not missing — it shipped in `0f4a4987` ("Split knowledge focus from chat discovery"). The only genuinely singular thing is the focus pointer. Adding a third collection on top of two working ones would give every row in `KBSelectorPalette` three independent toggles (active / primary / hidden), which the survey flagged as the point where the feature becomes unusable, and would force a "narrowing vs widening" decision on KB-less `kb_search` that can only regress somebody.

**Alternative rejected.** Three axes — `visible ⊇ active ∋ primary`. Rejected for the three-toggle row, for the second empty-set/fallback semantic to keep in sync with the first, and because "active but not visible" and "visible but not active" have no meaning a user could state.

**Consequence for Soul.** The Soul carve-out survives unchanged and becomes *more* coherent: a hidden base is simply not in the session's set, so it is not searched by default and cannot be the primary — but an explicit `kb_id="soul"` still reaches it, because an explicit id always overrides the set. `instructions.md` already says this; the new section keeps saying it.

### D2 — The primary: an explicit pointer, member of the set, never invented

**Decision.** `primary` is `Option<String>`. It is:

1. the **write target** for a KB-less mutating call (`platform__ingest_conversation`, the CLI's `--kb`-less `ingest`/`query`/`lint`, the GUI ingest panel),
2. the **default subject** for the four single-base MCP reads (`kb_list_pages`, `kb_read_page`, `kb_get_graph`, `kb_list_history`),
3. the **Knowledge view's focus** (graph, ingest, change log).

Invariant: **the primary is a member of the session's set.** Enforced on both sides:

- *Read side* (`primary_for_session`): the stored id is returned only while it names a member. Otherwise `None`. It never promotes at read time — a read must not silently invent a write target.
- *Write side* (`repair_primary_unlocked`, run by every writer that changes the set): if the primary the scope is **using** has left the set, **promote** to the lexicographically first remaining member, or **clear** when the set is now empty. The repair is persisted, so the CLI, the GUI and the model all see the same answer.

"The primary the scope is **using**" is load-bearing, and getting it wrong was a real bug. A session's own primary file has three states (D5), and the great majority of chats sit in the `Inherit` one: they have pinned nothing and are displaying the machine-wide pointer. If the repair only fires for a chat with its own stored pin, then two chats showing the user the identical thing — "alpha is this chat's primary" — answer the identical click two different ways: the pinning chat promotes to beta, the inheriting chat comes back with no primary at all, and the inheriting case is the common one. So the repair resolves the pointer through the session → machine fallback first, and **writes the result at the session's own scope**: a chat that has moved off the machine default is precisely what a session-scope pin represents, and repairing one chat must never move another chat's pointer.

**Never invented, but it does move.** Two rules that read as one contradiction until they are stated apart:

1. **An explicit no-primary choice is never overwritten.** A fresh/reset profile resolves the absent machine preference to the shipped Soul base. Creating any other base does not pin it, a sole non-Soul base is not promoted, and an explicit Clear remains durable. A pointer at a base that no longer *exists* is cleared rather than promoted. If Soul is unavailable or the user cleared the primary, a KB-less write fails loudly with the candidate list.
2. **A primary the user already had, whose base they just removed from this chat, moves to another member of the set.** Nothing is invented here: the user ranked that base, and the gesture was "take it out of this chat", not "disarm my write target".

Deletion is the one exception to promotion: `delete_base` **clears** a primary that pointed at the deleted base. Hiding a base is a scoping gesture and the base still exists, so promoting is friendly; deleting one is destructive, and silently re-pointing the write target at an unrelated base immediately after a destructive act is the wrong default. The same reasoning covers a *dangling* pointer — one naming a base that is not installed, which an upgrade from an older `.active-kb` can produce: it is cleared, never promoted, so an unrelated hide cannot turn it into a base nobody chose. A session that merely *inherits* a dangling pointer has nothing of its own to clear and is left untouched, which reads the same way (no primary) without silently severing that chat from the machine default over a pointer it never chose.

**Rationale.** Soul is a named product default, not a read-time guess from whatever bases happen to exist. General read-time promotion would make an explicit "no primary" unreachable whenever the set is non-empty and could send a KB-less *write* to the alphabetically first base the user never ranked. Write-time repair keeps the pointer honest, makes every change visible, and preserves the user-controlled Clear state.

**Alternative rejected.** Derive the primary at read time as "stored, else first member". Rejected because it silently picks a write target. Also rejected: *require* an explicit `kb_id` on every mutating call once the set has more than one member — that punishes exactly the multi-base workflow this change exists to enable, and it is unnecessary because a primary can be set once and then named in every result.

### D3 — Storage and migration: the file does not change

**Decision.** `<knowledge-root>/.active-kb` and `<knowledge-root>/.active-kb-sessions/<sha256(session_id)>` keep holding **one bare kb id** when pinned. A blank file is an explicit durable Clear. An absent session file inherits the machine preference; an absent machine file resolves to the shipped Soul product default. There is no eager migration. Only the *names in Rust* change (`get_active_persisted` → `get_primary_persisted`, `paths::active_kb_path` → `paths::primary_kb_path`); the path helpers keep returning `.active-kb` with a doc comment saying why.

**Rationale.** The merged model needs exactly one id, which is what the file already stores, so the shape that "cannot corrupt a downgrade" is *the shape already on disk*. A lagging PATH-installed `biorouter` (a documented drift mode — see CLAUDE.md "Runtime CLI-vs-app drift") reads a bare id and gets a real kb id whose meaning is unchanged for it: the base its KB-less commands target. Renaming the file would strand that binary's reads on a missing file and silently reset the user's selection.

**Alternative rejected.** Write a JSON array into `.active-kb`. `get_active_persisted` (`service.rs:981-993`) performs zero validation, so an older binary would take the literal string `["kb-a"]` as a kb id and join it into a filesystem path. Also rejected: a new `.primary-kb` sibling. Two files holding the same fact can disagree, and it needs a migration to populate — cost with no benefit, because the semantics are already compatible.

**The one storage-format change is on the other axis** — see D4 — and it is downgrade-safe by inspection: an older binary reading `[]` from a hidden file parses it as an empty list, which is exactly what it means.

### D4 — Bug 3: "explicitly nothing hidden" becomes representable

**Bug.** `get_hidden_for_session_or_persisted` (`service.rs:1043-1053`) uses **file existence** as the "is there a session override?" discriminator, while `set_hidden_path_unlocked` (`service.rs:157-162`) **deletes the file when the list is empty**. So a session that explicitly hides nothing is indistinguishable from a session that has never been touched, and silently re-inherits the machine-wide hidden list. Concretely: hide `soul` machine-wide, then press "Show all" in one chat — the chat re-hides `soul`. The same fires through `apply_workflow_knowledge_selection` (`routes/agent.rs:107-111`), where a workflow that declares *every* base visible produces an empty complement, deletes the override, and gets the machine default instead of what it declared. `service.rs:1740-1745` pins the wart today.

**Fix.** `set_hidden_path_unlocked` always writes the JSON array, `[]` included; existence stays the discriminator, so `[]` now means "this session overrides, and hides nothing". Add `clear_hidden_for_session` / `clear_hidden_persisted` for the genuine "stop overriding, go back to inheriting" gesture, which previously had no way to be expressed either.

**Rationale.** The discriminator was already the right one; the writer was contradicting it. Under the merged model the hidden list *is* the session's set, so "I chose all of them" has to be a state the system can hold — it is now the single most common gesture in the UI, not a rarity.

**Alternative rejected.** Switch the discriminator from existence to a sentinel value inside the file. Rejected: it changes what an older binary reads, and existence already worked.

### D5 — Bug 2: `ActiveKbState` is deleted, not pluralised

**Bug.** `ActiveKbState` (`server.rs:20-47`) is one `Arc<Mutex<Option<String>>>` **for the whole `KnowledgeServer` process**, not per session. `kb_set_active` writes it (`:692`) alongside the session file, and `active_kb_for_context` consults it (`:337`) for any session that has no `.active-kb-sessions/<digest>` file. One chat's choice therefore becomes another chat's default inside the same daemon. It is also never invalidated on rename or delete, so a stale id outranks the corrected on-disk value.

**Fix.** Delete the struct and the field. `primary_kb_for_context` reads the service directly: session file → machine file, guarded by membership.

**Rationale.** The cache exists only to avoid two `stat`+`read` calls of a file that is a few dozen bytes, on a path that already does a `list_bases()` walk. It buys nothing and it is a correctness hazard with a leak, a staleness bug and no invalidation. Removing it makes session scoping true by construction rather than by remembering to write two places.

**Alternative rejected.** Make it `HashMap<SessionId, String>`. Rejected: still a cache with no invalidation on rename/delete, still bootstrapped from a file it then shadows, and now with unbounded growth per session.

### D6 — Bug 1: a KB-less ingest targets the session, not the machine

**Bug.** `Agent::resolve_target_kb` (`crates/biorouter/src/agents/knowledge_tool.rs:121`) resolves the KB-less `platform__ingest_conversation` target from `svc.get_active_persisted()` — the **machine-wide** `.active-kb` — even though `session` is in scope at the call site (`:44-46`). Every other surface (the GUI chip, `kb_set_active`, `apply_workflow_knowledge_selection`, the apps platform) writes *session-scoped* state, so a Meditation/workflow session whose KB was set per session ingests into whatever the machine happens to point at.

**Fix.** Make it a free function taking `session_id`, resolving through `primary_for_session(Some(session_id))`, with an error that lists the session's bases and names the exact fix. The success text already names the base it wrote to (`knowledge_tool.rs:81-89`); a test pins that it does.

**Note on the anchor.** The task brief anchors this at "routes/knowledge.rs ingest path". Re-verified: the HTTP route is `POST /knowledge/bases/{id}/ingest` (`routes/knowledge.rs:51`, handler `:1028`) and takes its target from the **path segment**, so it is explicit and unaffected. The machine-wide leak is entirely in `knowledge_tool.rs:121`. The CLI's `resolve_kb` (`commands/knowledge.rs:30-41`) is also persisted-only, but that is correct there — the CLI has no session concept at all — and D11 makes it say so.

### D7 — Bug 4: app KB grants compose instead of clobbering

**Bug.** `configure_main_agent` (`routes/apps.rs:1238-1247`) and `configure_worker_agent` (`:1522-1529`) both call `set_active_for_session` with **the same session id** and each profile's own KB. Last writer wins, so which base an app's agents actually use depends on profile configuration order.

**Fix.** A grant now (a) makes the granted base a member of the session's set — it un-hides it, never hides anything — and (b) sets the primary **only from the main agent's grant**. A worker's grant joins the set and never steals the primary.

**Rationale.** Per-profile KB isolation does not exist today and cannot: every profile in an app shares one session id, so `set_active_for_session` was always writing to the same slot. This change does not widen the sandbox — the grant never restricted anything, it only *focused* — it replaces an order-dependent race with a stated rule. The main agent owning the primary matches the manifest, where `granted_knowledge_base` is the app-level grant and worker `knowledge_base` is a per-profile hint.

**Alternative rejected.** Let each worker's grant set the primary as it configures (i.e. keep last-writer-wins but on a set). Rejected: same nondeterminism, now harder to see.

### D8 — Bug 5: a workflow applies its whole set, and takes its primary from `default`

**Bug.** `apply_workflow_knowledge_selection` (`routes/agent.rs:112-115`) does `selection.default.or(selection.visible.first())` — a workflow that lists five bases gets one, silently.

**Fix.** The visible set is applied whole (it already was, via the hidden complement — D4 is what makes that reliable when the complement is empty). The primary comes from `selection.default`; when `default` is absent it is taken **only if the visible set has exactly one member**, otherwise left unset. A `default` that is not in `visible` is unioned into the set rather than dropped, so the invariant holds and the author's intent survives.

**Rationale.** `WorkflowKnowledgeBases { default, visible }` (`crates/biorouter/src/workflow/mod.rs:104-110`) is *already* "a set plus one primary" — the merged model needs **no schema change**, no back-compat deserializer, and no edit to the hand-written TS mirror at `ui/desktop/src/workflow/index.ts:10-15`. The silent `.first()` was the only thing standing between the workflow schema and the session model. Keeping the one-member case preserves every single-KB workflow in the wild (including Soul's, `crates/biorouter/src/knowledge/soul.rs:223-224`) without ever picking among many.

**Alternative rejected.** Pluralise `WorkflowKnowledgeBases.default`. Rejected: it is a serialised on-disk/deeplink format with artifacts in the wild, it would need a back-compat deserializer and a hand-edited TS mirror, and it buys nothing the existing shape does not already express.

### D9 — Bug 6: the session rewriters skip crash leftovers

**Bug.** `rewrite_session_active_refs_unlocked` (`service.rs:191-215`) walks `.active-kb-sessions/` with only an `is_file()` check, while `set_active_path_unlocked` stages writes as `<digest>.tmp` **in that same directory** (`:109-111`). A crash between write and rename leaves a `.tmp` file the rewriter then reads and rewrites as if it were a live session. `rewrite_hidden_refs_unlocked` (`:230-236`) has the identical hole, from the identical `with_extension("tmp")` at `:164-166`.

**Fix.** Both loops skip any filename that is not 64 lowercase hex characters — the exact shape `raw::hash_bytes` produces (`raw.rs:62-66`, `format!("{:x}")` of a SHA-256).

**Rationale.** A leftover `.tmp` is not a session, and a torn one makes the whole rename or delete fail with `?` — turning a crash from a week ago into "rename knowledge base" mysteriously erroring today. Filtering on the digest shape is cheaper and stricter than an extension blacklist.

### D10 — Wire shape: `primary_kb`, with `active_kb` kept as a deprecated mirror for one release

**Decision.** `ActiveKbResponse` gains `primary_kb` and `kb_ids` (the session's set) and keeps emitting `active_kb` as an exact duplicate of `primary_kb`. `SetActiveBody` accepts `primary_kb`, keeps `kb_id` as a legacy alias, and gains `clear_primary: bool`. A body that mentions neither leaves the primary **unchanged** (so a hidden-only edit can never nuke it), which is the same composability rule D7 applies to app grants. `GET`/`POST /knowledge/active` keep their paths; no new endpoints.

The body carries **three mutually exclusive** primary gestures, one per state of the stored tri-state (D5): `primary_kb` pins a base, `clear_primary` says this scope has no primary, and `inherit_primary` drops the scope's own preference. In a chat it follows the machine-wide pointer again; at machine scope it restores the shipped Soul default. The third is not optional decoration — `clear_primary` writes a *durable* override, and `delete_base` installs the same override in every chat that had pinned the deleted base, so without a way back over the wire such a chat could never follow the machine default again. Two gestures in one body is a **400 naming both fields**, not a precedence rule: they are three incompatible outcomes, and silently honouring one hands the caller a 200 for something it did not ask for and cannot detect. Two *spellings of the same* gesture — `clear_primary` alongside `primary_kb: null`, which is how a bundle predating `clear_primary` clears — is not a conflict. The CLI mirrors all three as `knowledge active --set` / `--clear` / `--inherit`; machine-scope `--inherit` restores Soul, while `--session <id> --inherit` resumes following the machine choice.

**Rationale.** The desktop app ships renderer and daemon together, but `just debug-ui` and a half-rebuilt dev tree routinely mix them, and `KnowledgeContext`'s `setActive` is fire-and-forget with no error surfaced to the user (`KnowledgeContext.tsx:83-93`) — a shape disagreement would present as an invisible state divergence. One release of a duplicated field costs one line.

**Alternative rejected.** Hard-rename `active_kb` → `primary_kb` in one step. Rejected on the mixed-version dev path above. Also rejected: new `/knowledge/active/add|remove` endpoints — with a single pointer there is nothing to add to, and the set is already edited through `hidden_kbs` in the same body.

### D11 — CLI: keeps `knowledge active`, gains membership validation and a named write target

**Decision.** `biorouter knowledge active [--set <id>] [--clear] [--inherit] [--session <id>]` keeps its name and flags. `--set` now additionally validates that the id is **not hidden machine-wide** (it must be a member of the set it would be primary of), `list` marks the primary (fixing the stale doc comment at `cli.rs:901`, which has promised that since the focus/discovery split), and the `--kb`-less `ingest`/`query`/`lint` path prints which base it resolved to before it writes anything. `--set`, `--clear` and `--inherit` are mutually exclusive.

**Rationale.** The CLI has no session concept — `handle_active`, `handle_hide`, `handle_unhide` and `resolve_kb` are all `*_persisted` — so machine-wide *is* its scope, and that is correct rather than a bug. Under one pointer there is no plural CLI syntax to invent. What the CLI owes the user is (a) not letting them pin a primary they have hidden and (b) never writing into a base it did not name first.

**Amendment (`--session`/`--inherit`).** One gesture has to reach into a chat: lifting the explicit "no primary" override (D10). `delete_base` can install it in a chat that had pinned the deleted base, and it survives every other gesture. `knowledge active` therefore takes an optional `--session <id>` that re-scopes show/`--set`/`--clear`/`--inherit` onto one chat. Without `--session`, `--inherit` removes the machine preference and restores the Soul product default. No other knowledge subcommand grows a session flag — the CLI still has no session of its own.

**Alternative rejected.** Add `--session <id>` so the CLI can drive a chat's selection. Rejected as scope: it is a real gap, but it is a new capability, not part of collapsing the axes, and it needs a session-id discovery story of its own.

### D12 — GUI: the palette row carries two states, never three

**Decision.** `KBSelectorPalette`'s existing per-row switch becomes the **membership** switch ("in this chat"), which is what it already was under a different name. The row body click becomes **"make primary"** and no longer closes the palette; a `PRIMARY` badge replaces `Focused`. Making a base primary while its switch is off turns the switch on in the same request — one user gesture, one `POST`, and the server validates the *resulting* state. `KBSelectorTrigger` shows the primary's dot and name plus the set size. `KnowledgeContext` renames `activeKbId`/`activeKb`/`setActiveKbId` to `primaryKbId`/`primaryKb`/`setPrimaryKbId`, and **takes the primary from the server's response** rather than re-deriving the repair rule in TypeScript.

The chat chip (`BottomMenuKnowledgeSelection.tsx`) is **untouched**: it already edits exactly the session set, with a searchable multi-toggle and a count. Its test is untouched too — none of the context fields it consumes are renamed.

**localStorage keys are unchanged** (`knowledge_active_kb`, `knowledge_hidden_kbs`). Renaming them would strand every existing key and silently break `ResetPanel.clearKnowledgeSelections` (`ResetPanel.tsx:116-123`), which prefix-scans those exact strings and has no test.

**Rationale.** Two independent per-row states is the ceiling for a comprehensible row, and the merged model happens to land exactly on two. Letting the daemon own the promote/clear rule keeps one implementation of it in the product.

**Alternative rejected.** A separate "primary" radio column next to the membership switch. Rejected: it makes "off but primary" clickable, i.e. it makes the invariant violable in the UI and then requires an error state to explain it.

**Amendment (the way back to the default).** The row stays two-state, but the *palette* grows a third gesture that is not on a row: a notice above the list offering **"Follow the default (<name>)"**, which sends `inherit_primary` (D10). It exists for the same reason the wire field does — `clear_primary` and `delete_base` both write a durable "this chat has no primary", so without it a chat whose pinned base was deleted could never follow the machine-wide default again from the GUI, only from `curl` or `biorouter knowledge active --session <id> --inherit`. It is one statement about the whole chat rather than a per-row control, so the D12 row is untouched and "off but primary" stays unclickable.

It is shown **only when following the default would visibly change something**: the default names one of *this chat's* bases and is not what the chat already shows. A chat already on the default has nothing to inherit, and a default this chat has left out of its set would resolve to no primary at all (the pointer is filtered through the set), so the click would appear to do nothing — the membership switch is the gesture for that, and turning it on is what makes the offer appear. Whether a chat overrides at all is **not** derivable from its own selection — a chat that pinned alpha and a chat that inherits an alpha default answer identically — so `KnowledgeContext` reads `GET /active` with no `session_id` for the machine-wide default, lazily, since only this surface needs it.

The gesture carries **no optimistic pointer** and **omits `hidden_kbs`**. The daemon resolves which base the chat lands on, so guessing would guess at the rule the gesture defers to; and a chat may be inheriting the machine-wide hidden list, so echoing the resolved list back would install a set override from a gesture that means "stop overriding here".

### Policies this plan deliberately does **not** change

- **KB-less `kb_search` keeps meaning "every base in this session."** Under the merged model that sentence is both today's behaviour and the literal ask. No `scope` parameter, no narrowing, no regression.
- **Reads are unlocked; writes take one exclusive per-KB lock.** Writes stay single-target, so no multi-KB lock acquisition exists and the `{a,b}` vs `{b,a}` deadlock hazard never arises. **Do not add a `lock_kbs` helper speculatively.**
- **A transaction is scoped to one KB, forever.** There is no cross-repo two-phase commit and this plan does not pretend otherwise.
- **Every MCP mutating tool keeps `kb_id` required** (`kb_write_page`, `kb_add_raw_source`, `kb_append_log`, `kb_restore_state`, the three txn tools, `kb_export`). This is now a stated invariant: do not make one optional "for symmetry with the reads".
- **Macros stay single-base.** `ingest`/`query`/`lint` each bind one `kb_id`, one `schema.md` and one txn. Cross-base `[[wiki-links]]` do not exist, so `lint` cannot produce cross-base false positives.
- **`kb_get_graph` and `kb_list_history` do not fan out.** They resolve to the primary. Namespacing graph node ids across bases is a separate feature.
- **BM25 cross-base ranking is unchanged.** `search_visible_bases` sorts by raw score across corpora, which is not strictly comparable — a real, pre-existing defect, but one this change neither introduces nor worsens, and fixing it is a search-quality project with its own evaluation.

---

## File structure

### Created

| Path | Responsibility |
|---|---|
| `ui/desktop/src/components/knowledge/KnowledgeContext.test.tsx` | First-ever Vitest coverage for the context: primary/hidden hydration from the server, the membership invariant on `setPrimaryKbId`, prune-don't-clear. |
| `ui/desktop/src/components/knowledge/KBSelector/KBSelectorPalette.test.tsx` | Row has exactly two states; "make primary" does not close the palette and turns membership on. |

### Modified

| Path | Change |
|---|---|
| `crates/biorouter-mcp/src/knowledge/paths.rs` | `active_kb_path`/`active_kb_sessions_dir` → `primary_kb_path`/`primary_kb_sessions_dir`, same `.active-kb*` filenames, with the downgrade rationale in the doc comment. |
| `crates/biorouter-mcp/src/knowledge/service.rs` | `session_kb_ids`, `primary_for_session`, `KbSelection`/`PrimaryUpdate`/`set_selection`, `clear_hidden_*`, `repair_primary_unlocked`; explicit-empty hidden writes; digest-shaped filename filter in both session rewriters; active→primary rename throughout. |
| `crates/biorouter-mcp/src/knowledge/server.rs` | Delete `ActiveKbState`; `primary_kb_for_context`; `kb_id_or_primary` with a candidate-listing error; `kb_set_active`/`kb_get_active` semantics + membership validation; five tool descriptions. |
| `crates/biorouter-mcp/src/knowledge/instructions.md` | New "Knowledge bases in this session" section — the model-facing contract for the set and the primary. |
| `crates/biorouter/src/agents/knowledge_tool.rs` | `resolve_target_kb` becomes a session-aware free function with a candidate-listing error. |
| `crates/biorouter/src/agents/platform_tools.rs` | `platform__ingest_conversation` description states the primary rule. |
| `crates/biorouter-server/src/routes/knowledge.rs` | `ActiveKbResponse`/`SetActiveBody` gain `primary_kb`/`kb_ids`/`clear_primary`; one shared resolver; validate-then-write through `set_selection`. |
| `crates/biorouter-server/src/routes/agent.rs` | Workflow selection applies the whole set and takes the primary from `default`. |
| `crates/biorouter-server/src/routes/apps.rs` | KB grants compose; only the main agent's grant sets the primary. |
| `crates/biorouter-server/src/routes/workflow.rs` | Capture side follows the rename. |
| `crates/biorouter-cli/src/cli.rs` | Fix the stale `list` doc comment; reword `Active`'s help for the primary. |
| `crates/biorouter-cli/src/commands/knowledge.rs` | Membership validation on `--set`; primary marked in `list`; `resolve_kb` names the base it resolved to. |
| `crates/biorouter-cli/src/session/output.rs` | Greeting row follows the rename. |
| `ui/desktop/src/components/knowledge/KnowledgeContext.tsx` | `primaryKbId`/`primaryKb`/`setPrimaryKbId`; server response is the source of truth for the repaired primary. |
| `ui/desktop/src/components/knowledge/KBSelector/KBSelectorPalette.tsx` | Membership switch + make-primary row body; `PRIMARY` badge; no auto-close. |
| `ui/desktop/src/components/knowledge/KBSelector/KBSelectorTrigger.tsx` | Primary dot + name + set size. |
| `ui/desktop/src/components/knowledge/IngestPanel/IngestPanel.tsx`, `graph/KnowledgeGraphPanel.tsx`, `changelog/ChangeLogDrawer.tsx`, `hooks/useKnowledgeBases.ts` | Consume `primaryKbId`. |
| `ui/desktop/src/components/MentionPopover.tsx` | Reads `primary_kb`; labels membership vs primary. |
| `ui/desktop/openapi.json`, `ui/desktop/src/api/*.gen.ts`, `ui/desktop/src/api/index.ts` | **Generated** — never hand-edited. |
| `CLAUDE.md`, `docs/knowledge-base/README.md` | Document the merged model and the primary rule. |

### Untouched on purpose

| Path | Why |
|---|---|
| `ui/desktop/src/components/bottom_menu/BottomMenuKnowledgeSelection.tsx` (+ its test) | Already edits exactly the session set. No context field it consumes is renamed. |
| `crates/biorouter/src/workflow/mod.rs` (`WorkflowKnowledgeBases`) | `{ default, visible }` already *is* "a set plus one primary" (D8). |
| `ui/desktop/src/workflow/index.ts` | The hand-written TS mirror only drifts if the Rust struct changes. It does not. |
| `crates/biorouter-mcp/src/knowledge/macros/*`, `subagent/*`, `store.rs` | Macros stay single-base; search fan-out and `SearchHitWithKb` attribution already ship. |
| The `.active-kb` / `.active-kb-sessions/<digest>` file format | D3 — the migration is a read. |

---

## Phase 1 — Service and storage

Everything in this phase is inside `crates/biorouter-mcp`. The phase ends with the crate green and no behaviour visible to any outer surface yet, except the two bug fixes (D4, D9) that are pure corrections.

### Task 1: The session's knowledge-base set, in the service

The id-level "which bases does this session use" computation currently lives in the MCP server (`server.rs:273-282`, `visible_bases_for_session`), so the HTTP routes and the CLI cannot reuse it. Under the merged model it is *the* central concept, so it belongs in the service.

**Files:**
- Modify: `crates/biorouter-mcp/src/knowledge/service.rs` (add near `get_hidden_for_session_or_persisted`, `:1043-1053`)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/biorouter-mcp/src/knowledge/service.rs`, immediately after `hidden_kbs_can_be_scoped_per_session` (`:1748`):

```rust
    /// The session's knowledge-base *set* — the one axis. Sorted, so any
    /// "first member" rule downstream is stable across processes and
    /// independent of registry insertion order.
    #[test]
    fn session_kb_ids_are_the_visible_set_sorted() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("zulu", "Zulu", None)?;
        svc.create_base("alpha", "Alpha", None)?;
        svc.create_base("mike", "Mike", None)?;

        // No session in scope (the CLI, a scheduled job): the machine list applies.
        svc.set_hidden_persisted(&["mike".to_string()])?;
        assert_eq!(
            svc.session_kb_ids(None)?,
            vec!["alpha".to_string(), "zulu".to_string()]
        );

        // A session override replaces the machine list wholesale, never unions.
        svc.set_hidden_for_session("session-a", &["zulu".to_string()])?;
        assert_eq!(
            svc.session_kb_ids(Some("session-a"))?,
            vec!["alpha".to_string(), "mike".to_string()]
        );

        // A session that never overrode inherits.
        assert_eq!(
            svc.session_kb_ids(Some("session-b"))?,
            vec!["alpha".to_string(), "zulu".to_string()]
        );
        Ok(())
    }
```

- [ ] **Step 2: Run the test — see it fail**

```bash
cargo test -p biorouter-mcp --lib knowledge::service::tests::session_kb_ids_are_the_visible_set_sorted
```

Expected: a compile error, not a failed assertion.

```
error[E0599]: no method named `session_kb_ids` found for struct `KnowledgeService` in the current scope
```

- [ ] **Step 3: Implement**

Add to `crates/biorouter-mcp/src/knowledge/service.rs`, in the `impl KnowledgeService` block that ends at `:1059`, right after `set_hidden_for_session`:

```rust
    /// The knowledge bases this scope may use, as ids, sorted.
    ///
    /// This is *the* set under the merged model: every base returned here is
    /// searchable by a `kb_id`-less `kb_search`, readable, and eligible to be
    /// the primary. `session_id = None` means "no session in scope" — the CLI
    /// and scheduled jobs — and falls back to the machine-wide hidden list.
    ///
    /// Sorted deliberately: the "lexicographically first member" promotion rule
    /// in [`Self::repair_primary_unlocked`] must not depend on registry
    /// insertion order, which differs between machines.
    pub fn session_kb_ids(&self, session_id: Option<&str>) -> anyhow::Result<Vec<String>> {
        let hidden = match session_id {
            Some(session_id) => self.get_hidden_for_session_or_persisted(session_id)?,
            None => self.get_hidden_persisted()?,
        };
        let mut ids = self
            .list_bases()?
            .into_iter()
            .map(|base| base.id)
            .filter(|id| !hidden.contains(id))
            .collect::<Vec<_>>();
        ids.sort();
        Ok(ids)
    }
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-mcp --lib knowledge::service::tests::session_kb_ids_are_the_visible_set_sorted
```

Expected: `test result: ok. 1 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-mcp/src/knowledge/service.rs
git commit -m "feat(knowledge): add session_kb_ids — the session's knowledge-base set"
```

---

### Task 2 (Bug 3): "explicitly nothing hidden" becomes representable

**Files:**
- Modify: `crates/biorouter-mcp/src/knowledge/service.rs:139-168` (`set_hidden_path_unlocked`), `:1030-1058` (public hidden API), `:1740-1745` (the test that pins the wart)

- [ ] **Step 1: Write the failing test**

Add to the tests module, after `session_kb_ids_are_the_visible_set_sorted`:

```rust
    /// Under the merged model the hidden list *is* the session's set, so
    /// "everything is in this chat" is the most common gesture there is. It
    /// must be a state the store can hold — writing an empty list used to
    /// delete the override file, and `get_hidden_for_session_or_persisted`
    /// uses file existence as its discriminator, so the session silently
    /// re-inherited the machine-wide list.
    #[test]
    fn session_hidden_override_can_be_explicitly_empty() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.set_hidden_persisted(&["kb-a".to_string()])?;

        // "Show everything in this chat" must NOT re-inherit the machine list.
        svc.set_hidden_for_session("session-a", &[])?;
        assert!(
            svc.get_hidden_for_session_or_persisted("session-a")?
                .is_empty(),
            "an explicitly empty session override must not inherit the machine default"
        );

        // A session that never overrode still inherits.
        assert_eq!(
            svc.get_hidden_for_session_or_persisted("session-b")?,
            vec!["kb-a".to_string()]
        );

        // Dropping the override is a separate, explicit gesture.
        svc.clear_hidden_for_session("session-a")?;
        assert_eq!(
            svc.get_hidden_for_session_or_persisted("session-a")?,
            vec!["kb-a".to_string()]
        );
        Ok(())
    }
```

Then **edit the existing test that pins the old behaviour**, `hidden_kbs_can_be_scoped_per_session` at `crates/biorouter-mcp/src/knowledge/service.rs:1740-1745`. Replace:

```rust
        svc.set_hidden_for_session("session-a", &[])?;
        assert!(svc.get_hidden_for_session("session-a")?.is_empty());
        assert_eq!(
            svc.get_hidden_for_session_or_persisted("session-a")?,
            vec!["kb-a".to_string(), "kb-b".to_string()]
        );
```

with:

```rust
        // Setting an empty list is an explicit override ("hide nothing here"),
        // not a request to fall back to the machine-wide list. See
        // `session_hidden_override_can_be_explicitly_empty`.
        svc.set_hidden_for_session("session-a", &[])?;
        assert!(svc.get_hidden_for_session("session-a")?.is_empty());
        assert!(svc
            .get_hidden_for_session_or_persisted("session-a")?
            .is_empty());
```

- [ ] **Step 2: Run the tests — see them fail**

```bash
cargo test -p biorouter-mcp --lib -- knowledge::service::tests::session_hidden_override_can_be_explicitly_empty knowledge::service::tests::hidden_kbs_can_be_scoped_per_session
```

Expected: a compile error for the missing method.

```
error[E0599]: no method named `clear_hidden_for_session` found for struct `KnowledgeService` in the current scope
```

After adding only the method stub the assertion failure would be:

```
assertion failed: an explicitly empty session override must not inherit the machine default
```

- [ ] **Step 3: Implement**

In `crates/biorouter-mcp/src/knowledge/service.rs`, in `set_hidden_path_unlocked`, delete the empty-list special case at `:157-162`:

```rust
        if sanitized.is_empty() {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
            return Ok(());
        }

```

so the function falls straight through to the tmp+rename write. Add a comment above the write:

```rust
        // An empty list is written, not deleted: `get_hidden_for_session_or_persisted`
        // discriminates on file *existence*, so `[]` is how a session says
        // "I override, and I hide nothing". Deleting the file here made that
        // state unrepresentable and silently re-inherited the machine default.
        let tmp = path.with_extension("tmp");
```

Add the two clear helpers, after `set_hidden_for_session` (`:1055-1058`):

```rust
    /// Drop a session's hidden-KB override so it inherits the machine-wide
    /// list again. Distinct from `set_hidden_for_session(sid, &[])`, which is
    /// an override that hides nothing.
    pub fn clear_hidden_for_session(&self, session_id: &str) -> anyhow::Result<()> {
        let _lock = self.lock_root()?;
        let path = self.hidden_session_path(session_id);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Remove the machine-wide hidden list entirely (equivalent to an empty
    /// list at this scope, but leaves no file behind).
    pub fn clear_hidden_persisted(&self) -> anyhow::Result<()> {
        let _lock = self.lock_root()?;
        let path = crate::knowledge::paths::hidden_kbs_path(self.root());
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-mcp --lib knowledge::
```

Expected: all knowledge tests pass, including `hidden_kbs_track_rename_and_delete` (which asserts emptiness, not file absence, so the new write satisfies it).

```
test result: ok. 122 passed; 0 failed
```

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-mcp/src/knowledge/service.rs
git commit -m "fix(knowledge): let a session explicitly hide nothing instead of re-inheriting the machine default"
```

---

### Task 3: Rename active → primary across the service and its callers

Pure rename, no behaviour change, no file-format change. Doing it now keeps every later task readable; doing it later would mean writing new code in the old vocabulary.

**Files:**
- Modify: `crates/biorouter-mcp/src/knowledge/paths.rs:51-59`, `crates/biorouter-mcp/src/knowledge/service.rs`, `crates/biorouter-mcp/src/knowledge/server.rs`, `crates/biorouter-server/src/routes/{knowledge.rs,agent.rs,apps.rs,workflow.rs}`, `crates/biorouter-cli/src/commands/knowledge.rs`, `crates/biorouter-cli/src/session/output.rs`, `crates/biorouter/src/agents/knowledge_tool.rs`

- [ ] **Step 1: Apply the rename**

```bash
cd /Users/wgu/Desktop/BioRouter
FILES="crates/biorouter-mcp/src/knowledge/paths.rs \
crates/biorouter-mcp/src/knowledge/service.rs \
crates/biorouter-mcp/src/knowledge/server.rs \
crates/biorouter-server/src/routes/knowledge.rs \
crates/biorouter-server/src/routes/agent.rs \
crates/biorouter-server/src/routes/apps.rs \
crates/biorouter-server/src/routes/workflow.rs \
crates/biorouter-cli/src/commands/knowledge.rs \
crates/biorouter-cli/src/session/output.rs \
crates/biorouter/src/agents/knowledge_tool.rs"
sed -i '' \
  -e 's/active_kb_sessions_dir/primary_kb_sessions_dir/g' \
  -e 's/active_kb_path/primary_kb_path/g' \
  -e 's/active_session_path/primary_session_path/g' \
  -e 's/set_active_path_unlocked/set_primary_path_unlocked/g' \
  -e 's/rewrite_session_active_refs_unlocked/rewrite_session_primary_refs_unlocked/g' \
  -e 's/get_active_persisted/get_primary_persisted/g' \
  -e 's/set_active_persisted/set_primary_persisted/g' \
  -e 's/get_active_for_session/get_primary_for_session/g' \
  -e 's/set_active_for_session/set_primary_for_session/g' \
  $FILES
```

(The `_unlocked` variants ride along on the `get_/set_active_persisted` patterns. `ActiveKbState`, `active_kb_for_context` and the HTTP field `active_kb` are deliberately **not** renamed here — they are deleted or reshaped in Tasks 8 and 14.)

Rename the three service tests by hand so their names match what they test:

- `active_kb_persists_to_disk` → `primary_kb_persists_to_disk`
- `active_kb_can_be_scoped_per_session` → `primary_kb_can_be_scoped_per_session`
- `session_scoped_active_kb_tracks_rename_and_delete` → `session_scoped_primary_kb_tracks_rename_and_delete`

- [ ] **Step 2: Fix the path-helper doc comments**

In `crates/biorouter-mcp/src/knowledge/paths.rs`, replace `:51-59` with:

```rust
/// Returns `<knowledge-root>/.active-kb` — the file that persists the
/// **primary** knowledge base id (the write target for KB-less mutating calls).
///
/// The filename keeps its historical `.active-kb` spelling on purpose. The
/// merged model needs exactly one id, which is exactly what this file already
/// holds, so today's value *is* the primary and reading it is the entire
/// migration. It also keeps a lagging PATH-installed `biorouter` (see CLAUDE.md,
/// "Runtime CLI-vs-app drift") working: it reads a bare kb id whose meaning is
/// unchanged for it. Renaming the file, or writing anything structured into it,
/// would break that binary — `get_primary_persisted` performs no validation, so
/// it would happily join a JSON array into a filesystem path.
pub fn primary_kb_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".active-kb")
}

/// Returns `<knowledge-root>/.active-kb-sessions` — one file per session,
/// named `sha256(session_id)`, each holding that session's primary kb id.
/// Same naming rationale as [`primary_kb_path`].
pub fn primary_kb_sessions_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".active-kb-sessions")
}
```

- [ ] **Step 3: Verify — the rename compiles and changes nothing**

```bash
cargo test -p biorouter-mcp --lib knowledge:: && cargo build -p biorouter-server -p biorouter-cli
```

Expected: the same test count as Task 2, all passing, and a clean build of both dependent crates.

```
test result: ok. 122 passed; 0 failed
    Finished `dev` profile [unoptimized + debuginfo] target(s)
```

Confirm no stale spelling survives outside the deliberately-deferred names:

```bash
grep -rn --include='*.rs' -e 'get_active_persisted' -e 'set_active_persisted' \
  -e 'get_active_for_session' -e 'set_active_for_session' crates/ | grep -v '/tests/'
```

Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add -A crates/
git commit -m "refactor(knowledge): rename the active-KB pointer to primary (no behaviour change)"
```

---

### Task 4: The primary is a member of the set — read guard and write repair

**Files:**
- Modify: `crates/biorouter-mcp/src/knowledge/service.rs` (the primary accessors at `:975-1028`, the hidden writers at `:1034-1058`)

- [ ] **Step 1: Write the failing test**

Add to the tests module:

```rust
    /// The one invariant of the merged model: the primary is always a member
    /// of the session's set. Enforced on the read side (never return a
    /// non-member) and on the write side (repair, and persist the repair, when
    /// a set change orphans it). It is never *invented* — a session with bases
    /// but no chosen primary has none, so a KB-less write fails loudly instead
    /// of landing in whichever base happens to sort first.
    #[test]
    fn primary_must_be_a_member_of_the_session_set() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("alpha", "Alpha", None)?;
        svc.create_base("beta", "Beta", None)?;
        svc.create_base("gamma", "Gamma", None)?;

        // Never invented.
        assert_eq!(svc.primary_for_session(Some("session-a"))?, None);

        svc.set_primary_for_session("session-a", Some("beta"))?;
        assert_eq!(
            svc.primary_for_session(Some("session-a"))?.as_deref(),
            Some("beta")
        );

        // Hiding the primary from this chat promotes to the lexicographically
        // first remaining member — and persists it, so the CLI and the GUI see
        // the same answer as the model.
        svc.set_hidden_for_session("session-a", &["beta".to_string()])?;
        assert_eq!(
            svc.primary_for_session(Some("session-a"))?.as_deref(),
            Some("alpha")
        );
        assert_eq!(
            svc.get_primary_for_session("session-a")?.as_deref(),
            Some("alpha"),
            "the promotion must be persisted, not re-derived on every read"
        );

        // Hiding everything clears it.
        svc.set_hidden_for_session(
            "session-a",
            &["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
        )?;
        assert_eq!(svc.primary_for_session(Some("session-a"))?, None);
        assert_eq!(svc.get_primary_for_session("session-a")?, None);

        // A machine-wide primary is inherited by a session that has not chosen
        // one — and hiding it repairs inside that session exactly as hiding a
        // pinned primary does, leaving the machine pointer alone for every
        // other chat. (D2: the repair reasons about the pointer the scope is
        // *using*, not only one it pinned itself.)
        svc.set_primary_persisted(Some("gamma"))?;
        assert_eq!(
            svc.primary_for_session(Some("session-b"))?.as_deref(),
            Some("gamma")
        );
        svc.set_hidden_for_session("session-b", &["gamma".to_string()])?;
        assert_eq!(
            svc.primary_for_session(Some("session-b"))?.as_deref(),
            Some("alpha"),
            "hiding the inherited primary promotes inside the session, exactly as \
             hiding a pinned one does"
        );
        assert_eq!(
            svc.get_primary_persisted()?.as_deref(),
            Some("gamma"),
            "and it leaves the machine pointer alone for every other chat"
        );
        Ok(())
    }
```

- [ ] **Step 2: Run the test — see it fail**

```bash
cargo test -p biorouter-mcp --lib knowledge::service::tests::primary_must_be_a_member_of_the_session_set
```

Expected:

```
error[E0599]: no method named `primary_for_session` found for struct `KnowledgeService` in the current scope
```

- [ ] **Step 3: Implement**

In `crates/biorouter-mcp/src/knowledge/service.rs`, first factor the duplicated single-id reader. Replace the bodies of `get_primary_persisted_unlocked` (`:981-993`) and `get_primary_for_session` (`:1006-1018`) with calls to one helper. Add the helper next to `set_primary_path_unlocked` (`:101`):

```rust
    /// Read a single-id primary pointer file. Absent or blank ⇒ `None`.
    /// The on-disk format is one bare kb id — see [`paths::primary_kb_path`].
    fn get_primary_path_unlocked(&self, path: &Path) -> anyhow::Result<Option<String>> {
        if !path.exists() {
            return Ok(None);
        }
        let s = std::fs::read_to_string(path)?;
        let trimmed = s.trim();
        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed.to_string()))
        }
    }
```

then:

```rust
    fn get_primary_persisted_unlocked(&self) -> anyhow::Result<Option<String>> {
        self.get_primary_path_unlocked(&crate::knowledge::paths::primary_kb_path(self.root()))
    }
```

```rust
    pub fn get_primary_for_session(&self, session_id: &str) -> anyhow::Result<Option<String>> {
        self.get_primary_path_unlocked(&self.primary_session_path(session_id))
    }
```

Add the read guard and the repair, after `session_kb_ids`:

```rust
    /// This scope's **primary** knowledge base: the write target for KB-less
    /// mutating calls and the default subject for single-base reads.
    ///
    /// Resolution is session file → machine file, and the result is returned
    /// only while it names a member of [`Self::session_kb_ids`]. A non-member
    /// yields `None` rather than promoting: promoting at read time would make
    /// "no primary" unreachable and let a KB-less *write* silently land in a
    /// base the user never ranked. Promotion happens once, at the moment the
    /// set changes, in [`Self::repair_primary_unlocked`].
    pub fn primary_for_session(&self, session_id: Option<&str>) -> anyhow::Result<Option<String>> {
        let stored = match session_id {
            Some(session_id) => match self.get_primary_for_session(session_id)? {
                Some(id) => Some(id),
                None => self.get_primary_persisted()?,
            },
            None => self.get_primary_persisted()?,
        };
        let Some(stored) = stored else {
            return Ok(None);
        };
        let ids = self.session_kb_ids(session_id)?;
        Ok(ids.into_iter().find(|id| id == &stored))
    }

    /// Re-establish "the primary is a member of the set" for one scope after
    /// the set changed. Promotes to the lexicographically first remaining
    /// member, or clears when nothing remains. Never invents a pointer where
    /// there was none. Callers must already hold the root lock.
    fn repair_primary_unlocked(&self, session_id: Option<&str>) -> anyhow::Result<Option<String>> {
        let path = match session_id {
            Some(session_id) => self.primary_session_path(session_id),
            None => crate::knowledge::paths::primary_kb_path(self.root()),
        };
        let Some(stored) = self.get_primary_path_unlocked(&path)? else {
            return Ok(None);
        };
        let ids = self.session_kb_ids(session_id)?;
        if ids.iter().any(|id| id == &stored) {
            return Ok(Some(stored));
        }
        let next = ids.into_iter().next();
        self.set_primary_path_unlocked(&path, next.as_deref())?;
        Ok(next)
    }
```

Finally call the repair from the three hidden writers, each of which already holds `lock_root()`. In `set_hidden_persisted` (`:1034-1037`):

```rust
    pub fn set_hidden_persisted(&self, ids: &[String]) -> anyhow::Result<()> {
        let _lock = self.lock_root()?;
        self.set_hidden_path_unlocked(&crate::knowledge::paths::hidden_kbs_path(self.root()), ids)?;
        self.repair_primary_unlocked(None)?;
        Ok(())
    }
```

In `set_hidden_for_session` (`:1055-1058`):

```rust
    pub fn set_hidden_for_session(&self, session_id: &str, ids: &[String]) -> anyhow::Result<()> {
        let _lock = self.lock_root()?;
        self.set_hidden_path_unlocked(&self.hidden_session_path(session_id), ids)?;
        self.repair_primary_unlocked(Some(session_id))?;
        Ok(())
    }
```

and in both `clear_hidden_*` helpers added in Task 2, after the `remove_file`:

```rust
        self.repair_primary_unlocked(Some(session_id))?;
        Ok(())
```

(and `self.repair_primary_unlocked(None)?;` in `clear_hidden_persisted`).

> The repair is per scope. A session that has no primary file of its own inherits the machine pointer, and the read guard in `primary_for_session` is what keeps *that* honest — there is no walk over every session digest on a machine-wide hide.

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-mcp --lib knowledge::
```

Expected: `test result: ok. 124 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-mcp/src/knowledge/service.rs
git commit -m "feat(knowledge): enforce that the primary KB is a member of the session's set"
```

---

### Task 5 (Bug 6): the session rewriters skip crash leftovers

**Files:**
- Modify: `crates/biorouter-mcp/src/knowledge/service.rs:191-215` (`rewrite_session_primary_refs_unlocked`), `:217-239` (`rewrite_hidden_refs_unlocked`)

- [ ] **Step 1: Write the failing test**

Add to the tests module:

```rust
    /// Both session directories are staged through `<digest>.tmp` in the same
    /// directory they are read from, so a crash between write and rename
    /// leaves a file the rewriters used to treat as a live session: the
    /// primary rewriter edited it, and a torn hidden leftover made the whole
    /// rename fail with `?` — a crash last week surfacing as "rename knowledge
    /// base" erroring today.
    #[test]
    fn session_rewriters_skip_crash_leftover_tmp_files() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("kb-a", "KB A", None)?;
        svc.set_primary_for_session("session-live", Some("kb-a"))?;
        svc.set_hidden_for_session("session-live", &[])?;

        let leftover = format!("{}.tmp", crate::knowledge::raw::hash_bytes(b"session-dead"));
        let primary_tmp =
            crate::knowledge::paths::primary_kb_sessions_dir(svc.root()).join(&leftover);
        std::fs::write(&primary_tmp, b"kb-a")?;
        let hidden_tmp =
            crate::knowledge::paths::hidden_kb_sessions_dir(svc.root()).join(&leftover);
        std::fs::write(&hidden_tmp, b"half-written garbage")?;

        // A rename must succeed and must rewrite only the live session files.
        let renamed = svc.update_base("kb-a", Some("Renamed KB"), None)?;
        assert_eq!(renamed.id, "renamed-kb");
        assert_eq!(
            svc.get_primary_for_session("session-live")?.as_deref(),
            Some("renamed-kb"),
            "the live session file must still be rewritten"
        );
        assert_eq!(
            std::fs::read_to_string(&primary_tmp)?,
            "kb-a",
            "a crash leftover is not a session"
        );
        assert_eq!(std::fs::read_to_string(&hidden_tmp)?, "half-written garbage");
        Ok(())
    }
```

- [ ] **Step 2: Run the test — see it fail**

```bash
cargo test -p biorouter-mcp --lib knowledge::service::tests::session_rewriters_skip_crash_leftover_tmp_files
```

Expected: the torn hidden leftover aborts `update_base`.

```
Error: expected value at line 1 column 1
test knowledge::service::tests::session_rewriters_skip_crash_leftover_tmp_files ... FAILED
```

- [ ] **Step 3: Implement**

Add a free function at the bottom of `crates/biorouter-mcp/src/knowledge/service.rs`, outside the `impl` blocks (above the `#[cfg(test)]` module):

```rust
/// True when `name` is a session-digest filename — 64 lowercase hex chars,
/// exactly what `raw::hash_bytes` produces. Everything else in a
/// `.*-sessions/` directory is debris (most often a `<digest>.tmp` staged
/// write that a crash left behind) and must never be read as session state.
fn is_session_digest(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
```

In `rewrite_session_primary_refs_unlocked`, after the `is_file()` guard (`:204-206`):

```rust
            if !is_session_digest(&entry.file_name()) {
                continue;
            }
```

In `rewrite_hidden_refs_unlocked`, in the same place (`:231-234`):

```rust
            if !entry.file_type()?.is_file() || !is_session_digest(&entry.file_name()) {
                continue;
            }
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-mcp --lib knowledge::
```

Expected: `test result: ok. 125 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-mcp/src/knowledge/service.rs
git commit -m "fix(knowledge): stop the session rewriters treating <digest>.tmp leftovers as live sessions"
```

---

### Task 6: `set_selection` — change the set and the primary as one operation

The HTTP handler currently writes the primary and then the hidden list through two separate `lock_root()` acquisitions, and validates the primary against the *old* set. That makes "un-hide this base and make it primary", the single most common GUI gesture under the merged model, impossible to express correctly.

**Files:**
- Modify: `crates/biorouter-mcp/src/knowledge/service.rs`

- [ ] **Step 1: Write the failing test**

```rust
    /// One request, one lock, validated against the *resulting* set — so
    /// "add this base to the chat and make it primary" is expressible, and a
    /// set-only edit can never move the pointer.
    #[test]
    fn set_selection_applies_set_and_primary_as_one_operation() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        for id in ["alpha", "beta", "gamma"] {
            svc.create_base(id, id, None)?;
        }
        svc.set_hidden_for_session("session-a", &["beta".to_string()])?;

        let sel = svc.set_selection(Some("session-a"), Some(&[]), PrimaryUpdate::Set("beta"))?;
        assert_eq!(
            sel.kb_ids,
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
        assert_eq!(sel.primary_kb.as_deref(), Some("beta"));

        let sel = svc.set_selection(
            Some("session-a"),
            Some(&["gamma".to_string()]),
            PrimaryUpdate::Unchanged,
        )?;
        assert_eq!(sel.kb_ids, vec!["alpha".to_string(), "beta".to_string()]);
        assert_eq!(
            sel.primary_kb.as_deref(),
            Some("beta"),
            "a set-only edit must not move the pointer"
        );

        let err = svc
            .set_selection(Some("session-a"), None, PrimaryUpdate::Set("gamma"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("gamma") && err.contains("alpha, beta"),
            "the rejection must name the id and the set it is not in, got: {err}"
        );

        let sel = svc.set_selection(Some("session-a"), None, PrimaryUpdate::Clear)?;
        assert_eq!(sel.primary_kb, None);
        Ok(())
    }
```

- [ ] **Step 2: Run the test — see it fail**

```bash
cargo test -p biorouter-mcp --lib knowledge::service::tests::set_selection_applies_set_and_primary_as_one_operation
```

Expected:

```
error[E0433]: failed to resolve: use of undeclared type `PrimaryUpdate`
error[E0599]: no method named `set_selection` found for struct `KnowledgeService` in the current scope
```

- [ ] **Step 3: Implement**

Add the two public types near `KnowledgeWriteGuard` (`crates/biorouter-mcp/src/knowledge/service.rs:85-88`):

```rust
/// One coherent knowledge-base selection for a scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KbSelection {
    /// The scope's knowledge bases, sorted. Every one is searchable, readable,
    /// and eligible to be the primary.
    pub kb_ids: Vec<String>,
    /// The hidden ids that produced `kb_ids`.
    pub hidden_kbs: Vec<String>,
    /// The write target for KB-less mutating calls. Always a member of
    /// `kb_ids`, or `None` when the scope has not chosen one.
    pub primary_kb: Option<String>,
}

/// What a caller wants to happen to the primary pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryUpdate<'a> {
    /// Leave the stored pointer alone. A set-only edit must never move it —
    /// this is what stops one surface's write clobbering another's choice.
    Unchanged,
    /// Forget the pointer. KB-less writes then fail until one is chosen.
    Clear,
    /// Pin this id. It must be a member of the *resulting* set.
    Set(&'a str),
}
```

Add the methods in the same `impl` block as `session_kb_ids`:

```rust
    /// Read-only snapshot of a scope's selection.
    pub fn selection(&self, session_id: Option<&str>) -> anyhow::Result<KbSelection> {
        Ok(KbSelection {
            kb_ids: self.session_kb_ids(session_id)?,
            hidden_kbs: match session_id {
                Some(session_id) => self.get_hidden_for_session_or_persisted(session_id)?,
                None => self.get_hidden_persisted()?,
            },
            primary_kb: self.primary_for_session(session_id)?,
        })
    }

    /// Apply a set change and a primary change as one operation under one root
    /// lock, validating the primary against the **resulting** set. `hidden =
    /// None` leaves the set alone.
    ///
    /// Every helper called here is an `*_unlocked` variant: taking `lock_root`
    /// twice in one call stack deadlocks (the guard is an `flock` on one path,
    /// and a second acquisition from the same process blocks forever).
    pub fn set_selection(
        &self,
        session_id: Option<&str>,
        hidden: Option<&[String]>,
        primary: PrimaryUpdate<'_>,
    ) -> anyhow::Result<KbSelection> {
        let _lock = self.lock_root()?;

        if let Some(hidden) = hidden {
            let path = match session_id {
                Some(session_id) => self.hidden_session_path(session_id),
                None => crate::knowledge::paths::hidden_kbs_path(self.root()),
            };
            self.set_hidden_path_unlocked(&path, hidden)?;
        }

        let primary_path = match session_id {
            Some(session_id) => self.primary_session_path(session_id),
            None => crate::knowledge::paths::primary_kb_path(self.root()),
        };
        match primary {
            PrimaryUpdate::Unchanged => {
                self.repair_primary_unlocked(session_id)?;
            }
            PrimaryUpdate::Clear => {
                self.set_primary_path_unlocked(&primary_path, None)?;
            }
            PrimaryUpdate::Set(id) => {
                let ids = self.session_kb_ids(session_id)?;
                if !ids.iter().any(|known| known == id) {
                    anyhow::bail!(
                        "knowledge base '{id}' is not one of this session's knowledge bases ({}). \
                         Add it to the session first, or pass kb_id explicitly to read it once.",
                        if ids.is_empty() {
                            "none".to_string()
                        } else {
                            ids.join(", ")
                        }
                    );
                }
                self.set_primary_path_unlocked(&primary_path, Some(id))?;
            }
        }

        self.selection(session_id)
    }
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-mcp --lib knowledge::
```

Expected: `test result: ok. 126 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-mcp/src/knowledge/service.rs
git commit -m "feat(knowledge): add set_selection — one lock, one validation, set plus primary"
```

---

### Task 7: Phase 1 gate

- [ ] **Step 1: Run every suite that touches the knowledge storage layer**

```bash
cargo test -p biorouter-mcp --lib knowledge::
cargo test -p biorouter-mcp --test knowledge_macros_e2e --test knowledge_registered --test knowledge_revert_integration
cargo test -p biorouter-server --test knowledge_routes
cargo test -p biorouter --lib knowledge::
cargo build -p biorouter-cli
```

Expected: every suite green. `knowledge_routes` still passes because Task 3 was a rename and the HTTP behaviour is untouched.

- [ ] **Step 2: Style**

```bash
cargo fmt && ./scripts/clippy-lint.sh
```

Expected: no diff from `cargo fmt`, no clippy warnings.

- [ ] **Step 3: Commit any formatting**

```bash
git add -A crates/
git commit -m "chore(knowledge): phase 1 gate — fmt and clippy clean" || echo "nothing to commit"
```

---
## Phase 2 — MCP tools and the model-facing contract

### Task 8 (Bug 2): delete the process-global `ActiveKbState`

**Files:**
- Modify: `crates/biorouter-mcp/src/knowledge/server.rs:20-55` (the struct and the field), `:244-253` (`new`), `:321-342` (`active_kb_for_context`), `:809-817` (the test helper)

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/biorouter-mcp/src/knowledge/server.rs`:

```rust
    /// Regression (pre-existing, not introduced by the merge): `ActiveKbState`
    /// was one `Option<String>` for the **whole KnowledgeServer process**.
    /// `kb_set_active` wrote it alongside the session file (`:692`) and
    /// `active_kb_for_context` consulted it (`:337`) for any session that had
    /// no file of its own — so one chat's choice silently became every other
    /// chat's write target inside one daemon, and it was never invalidated on
    /// rename or delete.
    #[test]
    fn one_sessions_primary_does_not_leak_into_another() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let server = server_with_root(tmp.path().to_path_buf());
        server.service.create_base("alpha", "Alpha", None)?;
        server.service.create_base("beta", "Beta", None)?;

        server
            .service
            .set_primary_for_session("session-a", Some("beta"))?;

        assert_eq!(
            server.primary_kb_for_session(Some("session-a"))?.as_deref(),
            Some("beta")
        );
        assert_eq!(
            server.primary_kb_for_session(Some("session-b"))?,
            None,
            "session-b never chose a primary; session-a's choice must not become its write target"
        );
        Ok(())
    }

    /// The guard against re-introducing the cache. Primary resolution must be
    /// a pure function of (session id, on-disk state) — any in-process slot
    /// re-opens the cross-session leak and the stale-after-rename bug, and
    /// neither has a cheap behavioural test because both need a live
    /// `RequestContext`.
    #[test]
    fn knowledge_server_keeps_no_in_process_primary_cache() {
        let src = include_str!("server.rs");
        assert!(
            !src.contains("ActiveKbState"),
            "primary resolution must read the service, not a process-local cache"
        );
    }
```

- [ ] **Step 2: Run the tests — see them fail**

```bash
cargo test -p biorouter-mcp --lib knowledge::server::tests
```

Expected:

```
error[E0599]: no method named `primary_kb_for_session` found for struct `KnowledgeServer` in the current scope
```

and, once that compiles, `knowledge_server_keeps_no_in_process_primary_cache` fails while the struct is still there.

- [ ] **Step 3: Implement**

Delete `crates/biorouter-mcp/src/knowledge/server.rs:20-47` (the whole `ActiveKbState` struct and its impl) and the `active: ActiveKbState,` field at `:54`. Drop `use std::sync::Arc` if nothing else in the file needs it (`cargo build` will say).

In `new` (`:244-253`):

```rust
    pub fn new() -> Result<Self> {
        Ok(Self {
            tool_router: Self::tool_router(),
            service: KnowledgeService::new_default()?,
            instructions: include_str!("instructions.md").to_string(),
        })
    }
```

Replace `active_kb_for_context` (`:321-342`) with two sync methods:

```rust
    /// This session's primary knowledge base — the write target for KB-less
    /// mutating calls and the default subject for single-base reads. Resolved
    /// from disk on every call: session file → machine file, returned only
    /// while it names a member of the session's set.
    fn primary_kb_for_session(&self, session_id: Option<&str>) -> Result<Option<String>, ErrorData> {
        self.service
            .primary_for_session(session_id)
            .map_err(into_err)
    }

    fn primary_kb_for_context(
        &self,
        context: Option<&RequestContext<RoleServer>>,
    ) -> Result<Option<String>, ErrorData> {
        self.primary_kb_for_session(Self::session_id(context))
    }
```

In the test helper `server_with_root` (`:809-817`), drop the `active: ActiveKbState::default(),` line.

`kb_set_active` (`:686-703`) and `kb_get_active` (`:709-715`) still reference `self.active` and will not compile yet — delete `self.active.set(&p.0.kb_id).await;` at `:692` now and leave the rest; Task 10 rewrites both bodies.

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-mcp --lib knowledge::
```

Expected: `test result: ok. 128 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-mcp/src/knowledge/server.rs
git commit -m "fix(knowledge): drop the process-global active-KB cache that leaked between sessions"
```

---

### Task 9: `kb_id_or_primary` — read tools resolve to the primary, or say who the candidates are

**Files:**
- Modify: `crates/biorouter-mcp/src/knowledge/server.rs:344-359` (`kb_id_or_active`), `:386-417` and `:489-520` (the four read-tool descriptions and call sites)

- [ ] **Step 1: Write the failing test**

```rust
    /// The hinge of the whole change. With no `kb_id` and no primary, the
    /// error is the only instruction the model gets — it must name the
    /// candidates and the exact recovery, never guess a base.
    #[test]
    fn kb_id_or_primary_errors_with_the_candidate_list() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let server = server_with_root(tmp.path().to_path_buf());
        server.service.create_base("alpha", "Alpha", None)?;
        server.service.create_base("beta", "Beta", None)?;

        let err = server
            .kb_id_or_primary(None, None)
            .expect_err("no primary chosen");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("alpha, beta") && err.message.contains("kb_set_active"),
            "the error must list the candidates and the fix, got: {}",
            err.message
        );

        server.service.set_primary_persisted(Some("beta"))?;
        assert_eq!(server.kb_id_or_primary(None, None)?, "beta");
        assert_eq!(
            server.kb_id_or_primary(Some("alpha".to_string()), None)?,
            "alpha",
            "an explicit kb_id always wins — that is how a base outside the set is reached"
        );
        Ok(())
    }

    /// The four read tools that fall back to the primary must say so, in the
    /// new vocabulary — the model's mental model is built from these strings,
    /// and "the active KB" is what makes it switch instead of passing kb_id.
    #[test]
    fn read_tool_descriptions_teach_the_primary_not_the_active_kb() {
        let tools = KnowledgeServer::tool_router().list_all();
        for name in [
            "kb_list_pages",
            "kb_read_page",
            "kb_get_graph",
            "kb_list_history",
        ] {
            let desc = tools
                .iter()
                .find(|t| t.name == name)
                .and_then(|t| t.description.clone())
                .unwrap_or_else(|| panic!("{name} has a description"));
            assert!(
                desc.contains("primary knowledge base"),
                "{name} must name the primary, got: {desc}"
            );
            assert!(
                !desc.contains("active KB"),
                "{name} must not keep teaching the single-active model, got: {desc}"
            );
        }
    }
```

- [ ] **Step 2: Run the tests — see them fail**

```bash
cargo test -p biorouter-mcp --lib knowledge::server::tests
```

Expected:

```
error[E0599]: no method named `kb_id_or_primary` found for struct `KnowledgeServer` in the current scope
```

- [ ] **Step 3: Implement**

Replace `kb_id_or_active` (`crates/biorouter-mcp/src/knowledge/server.rs:344-359`) with:

```rust
    /// Resolve `supplied` kb_id, else this session's primary.
    ///
    /// An explicit `kb_id` always wins and is never filtered against the
    /// session's set — that is how a hidden base (Soul) stays reachable.
    fn kb_id_or_primary(
        &self,
        supplied: Option<String>,
        context: Option<&RequestContext<RoleServer>>,
    ) -> Result<String, ErrorData> {
        if let Some(id) = supplied {
            return Ok(id);
        }
        if let Some(primary) = self.primary_kb_for_context(context)? {
            return Ok(primary);
        }
        let ids = self
            .service
            .session_kb_ids(Self::session_id(context))
            .map_err(into_err)?;
        Err(ErrorData::invalid_params(
            if ids.is_empty() {
                "this session has no knowledge bases, so there is nothing to read. \
                 Create one with kb_create_base."
                    .to_string()
            } else {
                format!(
                    "kb_id not supplied and this session has no primary knowledge base. \
                     Pass kb_id explicitly (one of: {}), or call kb_set_active to make one \
                     the primary — that is also where KB-less writes go.",
                    ids.join(", ")
                )
            },
            None,
        ))
    }
```

Update the four call sites — `:396`, `:413`, `:499`, `:514` — from

```rust
        let kb_id = self.kb_id_or_active(p.kb_id, Some(&context)).await?;
```

to

```rust
        let kb_id = self.kb_id_or_primary(p.kb_id, Some(&context))?;
```

and the four descriptions, replacing `Omit kb_id to use the active KB.` with:

```
Omit kb_id to use this session's primary knowledge base. To read a different base, pass its kb_id — you never need to change the primary to read.
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-mcp --lib knowledge::
```

Expected: `test result: ok. 130 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-mcp/src/knowledge/server.rs
git commit -m "feat(knowledge): resolve KB-less reads to the session primary with a candidate-listing error"
```

---

### Task 10: `kb_set_active` / `kb_get_active` speak set + primary

The tool **names stay**: models have learned them and the MCP cassettes encode them. What changes is what they mean, what they validate, and what they return.

**Files:**
- Modify: `crates/biorouter-mcp/src/knowledge/server.rs:682-715`

- [ ] **Step 1: Write the failing test**

```rust
    /// `kb_set_active` used to validate the id's *format* only — it would
    /// happily point the session at a base that does not exist, and with a
    /// KB-less write behind it that is a lost write. It now validates
    /// membership, and reports the whole selection back so the model does not
    /// need a second round-trip to see its bases.
    #[test]
    fn set_primary_validates_membership_and_reports_the_set() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let server = server_with_root(tmp.path().to_path_buf());
        for id in ["alpha", "beta", "gamma"] {
            server.service.create_base(id, id, None)?;
        }
        server
            .service
            .set_hidden_for_session("session-a", &["gamma".to_string()])?;

        let v = server.set_primary_json(Some("session-a"), "beta")?;
        assert_eq!(v["primary_kb"], serde_json::json!("beta"));
        assert_eq!(
            v["active_kb"],
            serde_json::json!("beta"),
            "the deprecated mirror must track the primary for one release"
        );
        assert_eq!(
            v["knowledge_bases"],
            serde_json::json!(["alpha", "beta"]),
            "the set comes back with the primary, so discovery is one call"
        );

        let err = server
            .set_primary_json(Some("session-a"), "gamma")
            .expect_err("gamma is not in this session");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("gamma") && err.message.contains("alpha, beta"),
            "got: {}",
            err.message
        );

        let err = server
            .set_primary_json(Some("session-a"), "no-such-kb")
            .expect_err("a base that does not exist can never be primary");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);

        assert_eq!(
            server.selection_json(Some("session-a"))?["primary_kb"],
            serde_json::json!("beta")
        );
        Ok(())
    }

    #[test]
    fn state_tool_descriptions_teach_the_merged_model() {
        let tools = KnowledgeServer::tool_router().list_all();
        let desc = tools
            .iter()
            .find(|t| t.name == "kb_set_active")
            .and_then(|t| t.description.clone())
            .expect("kb_set_active has a description");
        assert!(
            desc.contains("primary") && desc.contains("does not change what you can search"),
            "kb_set_active must stop implying that activating narrows search, got: {desc}"
        );
    }
```

- [ ] **Step 2: Run the tests — see them fail**

```bash
cargo test -p biorouter-mcp --lib knowledge::server::tests
```

Expected:

```
error[E0599]: no method named `set_primary_json` found for struct `KnowledgeServer` in the current scope
```

- [ ] **Step 3: Implement**

Replace `crates/biorouter-mcp/src/knowledge/server.rs:682-715` with the two thin tools plus the two testable helpers they wrap:

```rust
    /// Body of `kb_set_active`, split out so it can be unit-tested without
    /// fabricating a `RequestContext`.
    fn set_primary_json(
        &self,
        session_id: Option<&str>,
        kb_id: &str,
    ) -> Result<serde_json::Value, ErrorData> {
        crate::knowledge::paths::validate_kb_id(kb_id)
            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
        let selection = self
            .service
            .set_selection(
                session_id,
                None,
                crate::knowledge::service::PrimaryUpdate::Set(kb_id),
            )
            .map_err(|e| ErrorData::invalid_params(format!("{e:#}"), None))?;
        Ok(Self::selection_value(&selection, true))
    }

    /// Body of `kb_get_active`.
    fn selection_json(&self, session_id: Option<&str>) -> Result<serde_json::Value, ErrorData> {
        let selection = self.service.selection(session_id).map_err(into_err)?;
        Ok(Self::selection_value(&selection, false))
    }

    fn selection_value(
        selection: &crate::knowledge::service::KbSelection,
        ok: bool,
    ) -> serde_json::Value {
        let mut v = serde_json::json!({
            "primary_kb": selection.primary_kb,
            "knowledge_bases": selection.kb_ids,
            // Deprecated mirror of `primary_kb`, kept for one release so
            // anything that learned the old key keeps working.
            "active_kb": selection.primary_kb,
        });
        if ok {
            v["ok"] = serde_json::Value::Bool(true);
        }
        v
    }

    #[tool(
        name = "kb_set_active",
        description = "Make one knowledge base this session's primary: the base that KB-less writes land in and that single-base reads default to. It does not change what you can search — kb_search with no kb_id already covers every knowledge base in this session, tagging each hit with its kb_id. To read or write another base, pass its kb_id; do not switch the primary to get at it. The base must be one of this session's knowledge bases."
    )]
    pub async fn kb_set_active(
        &self,
        p: Parameters<SetActiveParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let v = self.set_primary_json(Self::session_id(Some(&context)), &p.0.kb_id)?;
        ok_json(&v)
    }

    #[tool(
        name = "kb_get_active",
        description = "Return this session's knowledge bases and which one is the primary (the KB-less write target)."
    )]
    pub async fn kb_get_active(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let v = self.selection_json(Self::session_id(Some(&context)))?;
        ok_json(&v)
    }
```

> `set_selection` with `session_id = None` writes the machine-wide pointer, which preserves today's behaviour for a caller with no session meta (the CLI-driven MCP path).

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-mcp --lib knowledge::
```

Expected: `test result: ok. 132 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-mcp/src/knowledge/server.rs
git commit -m "feat(knowledge): kb_set_active pins a validated primary and reports the session's set"
```

---

### Task 11: Teach the model the merged model

Without this the model keeps the single-active picture and "switches" between bases instead of using the set — recreating exactly the serialisation this change removes. `instructions.md` is the only prose spec it gets.

**Files:**
- Modify: `crates/biorouter-mcp/src/knowledge/instructions.md`, `crates/biorouter-mcp/src/knowledge/server.rs` (tests)

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/biorouter-mcp/src/knowledge/server.rs`:

```rust
    /// Prose is behaviour here. Pin the sentences the model needs: that every
    /// base in the session is already in play, that one of them is the primary
    /// write target, and — the load-bearing one — that reading another base
    /// means passing kb_id, not switching the primary.
    #[test]
    fn instructions_teach_the_session_set_and_the_primary() {
        let instructions = include_str!("instructions.md");
        assert!(
            instructions.contains("primary") && instructions.contains("kb_get_active"),
            "instructions must name the primary and how to read it"
        );
        assert!(
            instructions.contains("Do not switch the primary"),
            "instructions must forbid switching the primary just to read another base"
        );
        assert!(
            instructions.contains("every knowledge base in this session"),
            "instructions must state that a kb_id-less kb_search already covers the whole set"
        );
        assert!(
            instructions.contains("kb_set_active"),
            "instructions must name the recovery when there is no primary"
        );
    }
```

- [ ] **Step 2: Run the test — see it fail**

```bash
cargo test -p biorouter-mcp --lib knowledge::server::tests::instructions_teach_the_session_set_and_the_primary
```

Expected:

```
thread '...' panicked at ...: instructions must name the primary and how to read it
```

- [ ] **Step 3: Implement**

In `crates/biorouter-mcp/src/knowledge/instructions.md`, replace the `kb_search` bullet (`:22`) with:

```markdown
- `kb_search` — search curated knowledge pages. If you omit `kb_id`, the search runs across **every knowledge base in this session** and each hit is tagged with the `kb_id` it came from. Cite that id when you use a hit.
```

and insert a new section between "Retrieval behavior" and "Personal context (Soul)":

```markdown
Knowledge bases in this session:

- Every base `kb_list_bases` returns is in play. There is no narrower "active" list to manage: a `kb_search` with no `kb_id` already covers all of them, and any tool call may name any base with an explicit `kb_id`.
- One of them is the **primary**. It is the base that KB-less writes land in, and the base that single-base reads (`kb_list_pages`, `kb_read_page`, `kb_get_graph`, `kb_list_history`) default to when you omit `kb_id`. Call `kb_get_active` to see the session's bases and which is primary; call `kb_set_active` to move the primary to another of them.
- **Do not switch the primary in order to read another base.** Pass that base's `kb_id` on the call. Changing the primary changes where writes go for the rest of the session, which is rarely what the user asked for.
- Writes name their base. `kb_write_page`, `kb_add_raw_source`, `kb_append_log`, `kb_restore_state` and the transaction tools all require `kb_id` — this is deliberate, so a write is never ambiguous. Tools that write on the user's behalf without one (for example `platform__ingest_conversation`) use the primary and tell you which base they used.
- If the session has no primary, a KB-less write fails and the error lists the bases you can choose from. Call `kb_set_active` with one of them, or pass `kb_id` on the call.
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-mcp --lib knowledge::server::tests
```

Expected: both instruction tests pass — the pre-existing `instructions_cover_soul_and_hidden_kb_access` still passes because the Soul section is untouched.

```
test result: ok. 8 passed; 0 failed
```

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-mcp/src/knowledge/instructions.md crates/biorouter-mcp/src/knowledge/server.rs
git commit -m "docs(knowledge): teach the model the session set and the primary write target"
```

---

### Task 12 (Bug 1): a KB-less conversation ingest targets the session's primary

**Files:**
- Modify: `crates/biorouter/src/agents/knowledge_tool.rs:44-46, 81-89, 92-127, 197-250`
- Modify: `crates/biorouter/src/agents/platform_tools.rs:68-70`

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/biorouter/src/agents/knowledge_tool.rs`. Extend the `use super::{...}` line at `:199` to include `ingest_summary` and `resolve_target_kb`, and add the service import the test needs:

```rust
    use super::{
        ingest_summary, resolve_target_kb, should_use_knowledge_default_model, slugify_kb_name,
    };
    use biorouter_mcp::knowledge::service::KnowledgeService;
```

Then the two tests:

```rust
    /// Pre-existing bug: the KB-less target came from the **machine-wide**
    /// `.active-kb`, while every other surface — the chat chip, kb_set_active,
    /// workflows, the apps platform — writes session-scoped state. A
    /// Meditation/workflow session whose KB was set per session therefore
    /// ingested into whatever the machine happened to point at.
    #[test]
    fn resolve_target_kb_uses_the_session_primary_not_the_machine_default() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("machine-kb", "Machine", None)?;
        svc.create_base("session-kb", "Session", None)?;
        svc.set_primary_persisted(Some("machine-kb"))?;
        svc.set_primary_for_session("chat-1", Some("session-kb"))?;

        let args = serde_json::json!({});
        assert_eq!(resolve_target_kb(&svc, &args, "chat-1")?, "session-kb");
        assert_eq!(
            resolve_target_kb(&svc, &args, "chat-2")?,
            "machine-kb",
            "a chat that never chose one still inherits the machine pointer"
        );

        svc.set_primary_persisted(None)?;
        let err = resolve_target_kb(&svc, &args, "chat-9")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("machine-kb, session-kb") && err.contains("kb_id"),
            "the error must list the candidates and the fix, got: {err}"
        );
        Ok(())
    }

    /// A KB-less write must name the base it wrote to, in the text the model
    /// and the user both read.
    #[test]
    fn ingest_summary_names_the_target_base() {
        let summary = ingest_summary(2, "my-kb", "src-1", "abcdef1234567890", 7);
        assert!(summary.contains("'my-kb'"), "got: {summary}");
        assert!(summary.contains("abcdef12") && !summary.contains("abcdef123"));
    }
```

- [ ] **Step 2: Run the tests — see them fail**

```bash
cargo test -p biorouter --lib agents::knowledge_tool
```

Expected:

```
error[E0425]: cannot find function `resolve_target_kb` in this scope
error[E0425]: cannot find function `ingest_summary` in this scope
```

- [ ] **Step 3: Implement**

In `crates/biorouter/src/agents/knowledge_tool.rs`, move `resolve_target_kb` out of the `impl Agent` block and make it a session-aware free function (replacing `:92-127`):

```rust
/// Resolve which KB a conversation ingest targets: `new_kb_name` creates one,
/// else an explicit `kb_id`, else **this session's primary**.
///
/// It must be the session's primary, not the machine-wide pointer: every other
/// surface writes session-scoped state, so reading the machine default here
/// sent a workflow/Meditation session's transcript into an unrelated base.
pub(crate) fn resolve_target_kb(
    svc: &KnowledgeService,
    arguments: &Value,
    session_id: &str,
) -> anyhow::Result<String> {
    if let Some(name) = arguments.get("new_kb_name").and_then(|v| v.as_str()) {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("new_kb_name cannot be empty");
        }
        let id = slugify_kb_name(name);
        if id.is_empty() {
            anyhow::bail!("new_kb_name must contain letters or numbers");
        }
        if !svc.list_bases()?.iter().any(|b| b.id == id) {
            svc.create_base(&id, name, None)?;
        }
        return Ok(id);
    }
    if let Some(id) = arguments.get("kb_id").and_then(|v| v.as_str()) {
        let id = id.trim();
        if !svc.list_bases()?.iter().any(|b| b.id == id) {
            anyhow::bail!("knowledge base '{id}' does not exist");
        }
        return Ok(id.to_string());
    }
    if let Some(primary) = svc.primary_for_session(Some(session_id))? {
        return Ok(primary);
    }
    let ids = svc.session_kb_ids(Some(session_id))?;
    if ids.is_empty() {
        anyhow::bail!(
            "no target knowledge base: this chat has none. Pass new_kb_name to create one, \
             or kb_id to name an existing base."
        );
    }
    anyhow::bail!(
        "no target knowledge base: pass kb_id (one of: {}) or new_kb_name, or call \
         kb_set_active to make one of them this chat's primary.",
        ids.join(", ")
    )
}

/// The success text for a conversation ingest. A KB-less write resolves its
/// target silently, so the result must name the base it landed in.
fn ingest_summary(
    session_count: usize,
    kb_id: &str,
    source_id: &str,
    commit_sha: &str,
    steps: usize,
) -> String {
    format!(
        "Ingested {session_count} conversation(s) into knowledge base '{kb_id}'. \
         Source id: {source_id}, commit: {}, sub-agent steps: {steps}.",
        commit_sha.chars().take(8).collect::<String>()
    )
}
```

Update the call site at `:43-46`:

```rust
        // Resolve target KB: explicit id → new-by-name → this session's primary.
        let kb_id = resolve_target_kb(&svc, &arguments, &session.id).map_err(invalid_params)?;
```

and the result at `:81-89`:

```rust
        Ok(vec![Content::text(ingest_summary(
            session_ids.len(),
            &kb_id,
            &result.source_id,
            &result.commit_sha,
            result.steps,
        ))])
```

In `crates/biorouter/src/agents/platform_tools.rs`, replace the last line of the description (`:70`):

```
            If neither is given, the currently active knowledge base is used.
```

with:

```
            If neither is given it uses this chat's primary knowledge base, and
            the result names the base it wrote to. If the chat has no primary,
            the error lists the bases you can pass as `kb_id`.
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter --lib agents::knowledge_tool && cargo test -p biorouter --lib knowledge::
```

Expected: `test result: ok. 4 passed; 0 failed` for the first, all green for the second.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter/src/agents/knowledge_tool.rs crates/biorouter/src/agents/platform_tools.rs
git commit -m "fix(knowledge): resolve a KB-less conversation ingest from the session, not the machine default"
```

---

### Task 13: Phase 2 gate

- [ ] **Step 1: Run the MCP and agent suites**

```bash
cargo test -p biorouter-mcp --lib knowledge::
cargo test -p biorouter-mcp --test knowledge_macros_e2e --test knowledge_registered --test knowledge_revert_integration
cargo test -p biorouter --lib knowledge:: --lib agents::knowledge_tool
cargo test -p biorouter --test knowledge_e2e
cargo build -p biorouter-server -p biorouter-cli
```

Expected: every suite green.

- [ ] **Step 2: Check the recorded MCP cassettes**

```bash
grep -rln 'kb_set_active\|kb_get_active' crates/biorouter-mcp/tests/ crates/biorouter/tests/ 2>/dev/null
```

If any cassette contains a recorded `kb_set_active`/`kb_get_active` exchange, its response now carries `primary_kb`/`knowledge_bases` alongside the unchanged `active_kb`. Re-record only if a test asserts on the payload:

```bash
BIOROUTER_RECORD_MCP=1 just record-mcp-tests
```

Expected: no matches, i.e. nothing to re-record.

- [ ] **Step 3: Style and commit**

```bash
cargo fmt && ./scripts/clippy-lint.sh
git add -A crates/
git commit -m "chore(knowledge): phase 2 gate — fmt and clippy clean" || echo "nothing to commit"
```

---
## Phase 3 — HTTP and OpenAPI

### Task 14: `/knowledge/active` returns the set and a validated primary

**Files:**
- Modify: `crates/biorouter-server/src/routes/knowledge.rs:11-19` (imports), `:600-708` (both handlers and both structs)
- Modify: `crates/biorouter-server/tests/knowledge_routes.rs:1606-1875`

- [ ] **Step 1: Write the failing tests**

Two edits to existing tests, then two new ones.

**(a)** In `active_kb_roundtrip`, replace the "Clear it" body at `crates/biorouter-server/tests/knowledge_routes.rs:1700`:

```rust
    let clear_body = serde_json::to_vec(&serde_json::json!({"kb_id": null})).unwrap();
```

with

```rust
    // Clearing is now an explicit flag. A body that simply does not mention
    // the primary leaves it alone, so a hidden-only edit can never nuke it —
    // the same composability rule the app-grant fix relies on.
    let clear_body = serde_json::to_vec(&serde_json::json!({"clear_primary": true})).unwrap();
```

**(b)** Replace the whole of `active_kb_can_be_scoped_per_session` (`:1762-1875`). The old test pinned a primary that was *hidden from the very scope it was primary of* (`{"kb_id": "act", "hidden_kbs": ["act"]}`) — the exact contradiction the merged model removes. The rewrite keeps its real subject, session-over-machine precedence:

```rust
#[tokio::test]
async fn primary_kb_can_be_scoped_per_session() {
    let (_d, app) = build_test_router();

    for (id, name) in [("act", "Act"), ("session-kb", "Session KB")] {
        let create_body = serde_json::to_vec(&serde_json::json!({"id": id, "name": name})).unwrap();
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
    }

    // Machine-wide: both bases in play, "act" is the primary.
    let global = post_active(&app, serde_json::json!({"primary_kb": "act"})).await;
    assert_eq!(global.0, 200);
    assert_eq!(global.1["primary_kb"].as_str(), Some("act"));
    assert_eq!(
        global.1["kb_ids"],
        serde_json::json!(["act", "session-kb"]),
        "the response carries the session's whole set, not just the pointer"
    );

    // session-a narrows to one base and points at it.
    let scoped = post_active(
        &app,
        serde_json::json!({
            "primary_kb": "session-kb",
            "session_id": "session-a",
            "hidden_kbs": ["act"],
        }),
    )
    .await;
    assert_eq!(scoped.0, 200);
    assert_eq!(scoped.1["primary_kb"].as_str(), Some("session-kb"));
    assert_eq!(scoped.1["kb_ids"], serde_json::json!(["session-kb"]));
    assert_eq!(
        scoped.1["active_kb"].as_str(),
        Some("session-kb"),
        "the deprecated mirror must track the primary for one release"
    );

    // The machine scope is untouched, and a session that never overrode inherits it.
    let machine = get_active(&app, None).await;
    assert_eq!(machine["primary_kb"].as_str(), Some("act"));
    let other = get_active(&app, Some("session-b")).await;
    assert_eq!(other["primary_kb"].as_str(), Some("act"));
    assert_eq!(other["kb_ids"], serde_json::json!(["act", "session-kb"]));
}
```

**(c)** Two new tests for the invariant and for composability:

```rust
/// The merged model's one invariant, at the wire. A primary that is not in the
/// resulting set is rejected with both halves named; the un-hide and the
/// re-point travel in ONE body so the GUI's "make primary" on an off row is a
/// single request validated against the state it produces.
#[tokio::test]
async fn primary_must_be_a_member_of_the_resulting_set() {
    let (_d, app) = build_test_router();
    for id in ["alpha", "beta"] {
        let create_body = serde_json::to_vec(&serde_json::json!({"id": id, "name": id})).unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    let bad = post_active(
        &app,
        serde_json::json!({"primary_kb": "beta", "hidden_kbs": ["beta"]}),
    )
    .await;
    assert_eq!(bad.0, 400, "a hidden base cannot be the primary");

    let good = post_active(
        &app,
        serde_json::json!({"primary_kb": "beta", "hidden_kbs": []}),
    )
    .await;
    assert_eq!(good.0, 200);
    assert_eq!(good.1["primary_kb"].as_str(), Some("beta"));
}

/// A set-only edit must never move the pointer, and hiding the primary must
/// promote deterministically rather than leaving a dangling write target.
#[tokio::test]
async fn set_only_edit_keeps_the_primary_until_it_leaves_the_set() {
    let (_d, app) = build_test_router();
    for id in ["alpha", "beta", "gamma"] {
        let create_body = serde_json::to_vec(&serde_json::json!({"id": id, "name": id})).unwrap();
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/bases")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
    }
    post_active(&app, serde_json::json!({"primary_kb": "beta"})).await;

    let narrowed = post_active(&app, serde_json::json!({"hidden_kbs": ["gamma"]})).await;
    assert_eq!(narrowed.1["primary_kb"].as_str(), Some("beta"));

    let orphaned = post_active(&app, serde_json::json!({"hidden_kbs": ["beta"]})).await;
    assert_eq!(
        orphaned.1["primary_kb"].as_str(),
        Some("alpha"),
        "hiding the primary promotes to the first remaining member"
    );
}
```

Add the two request helpers near `build_test_router` (`crates/biorouter-server/tests/knowledge_routes.rs:11-27`):

```rust
async fn post_active(app: &Router, body: serde_json::Value) -> (u16, serde_json::Value) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/active")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status().as_u16();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn get_active(app: &Router, session_id: Option<&str>) -> serde_json::Value {
    let uri = match session_id {
        Some(sid) => format!("/active?session_id={sid}"),
        None => "/active".to_string(),
    };
    let res = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}
```

- [ ] **Step 2: Run the tests — see them fail**

```bash
cargo test -p biorouter-server --test knowledge_routes active
```

Expected: the new field is missing and the clear flag is ignored.

```
assertion `left == right` failed
  left: None
 right: Some("act")
test primary_kb_can_be_scoped_per_session ... FAILED
test active_kb_roundtrip ... FAILED
```

- [ ] **Step 3: Implement**

In `crates/biorouter-server/src/routes/knowledge.rs`, add `PrimaryUpdate` to the `biorouter_mcp::knowledge` import (`:11-19`):

```rust
    service::{KnowledgeService, PrimaryUpdate, ReadPageError},
```

Replace `:606-708` with:

```rust
#[derive(Deserialize, ToSchema)]
pub struct SetActiveBody {
    /// Make this base the session's primary — the KB-less write target. It
    /// must be a member of the **resulting** set, so `hidden_kbs` in the same
    /// body is applied first. Omit to leave the pointer alone.
    #[serde(default)]
    pub primary_kb: Option<String>,
    /// Deprecated alias for `primary_kb`, kept for one release so a stale
    /// renderer bundle talking to a fresh daemon keeps working.
    #[serde(default)]
    pub kb_id: Option<String>,
    /// Forget the primary. Wins over `primary_kb`.
    #[serde(default)]
    pub clear_primary: bool,
    #[serde(default)]
    pub session_id: Option<String>,
    /// Replace this scope's hidden list — i.e. redefine the session's set.
    /// Omit to leave the set alone. `[]` is an explicit "hide nothing here",
    /// not a request to inherit the machine-wide list.
    #[serde(default)]
    pub hidden_kbs: Option<Vec<String>>,
}

#[derive(Serialize, ToSchema)]
pub struct ActiveKbResponse {
    /// The session's knowledge bases, sorted. Every one is searchable and
    /// readable; there is no narrower "active" list.
    pub kb_ids: Vec<String>,
    /// The KB-less write target. Always a member of `kb_ids`, or `null`.
    pub primary_kb: Option<String>,
    /// Deprecated mirror of `primary_kb`.
    pub active_kb: Option<String>,
    pub hidden_kbs: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct GetActiveQuery {
    #[serde(default)]
    pub session_id: Option<String>,
}

fn selection_response(
    svc: &KnowledgeService,
    session_id: Option<&str>,
) -> Result<ActiveKbResponse, (StatusCode, String)> {
    let selection = svc
        .selection(session_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    Ok(ActiveKbResponse {
        kb_ids: selection.kb_ids,
        active_kb: selection.primary_kb.clone(),
        primary_kb: selection.primary_kb,
        hidden_kbs: selection.hidden_kbs,
    })
}

#[utoipa::path(
    get, path = "/knowledge/active",
    params(
        ("session_id" = Option<String>, Query, description = "Optional chat session id for the session-scoped selection"),
    ),
    responses((status = 200, description = "The session's knowledge bases and its primary", body = ActiveKbResponse))
)]
pub async fn get_active(
    State(svc): State<Arc<KnowledgeService>>,
    Query(q): Query<GetActiveQuery>,
) -> Result<Json<ActiveKbResponse>, (StatusCode, String)> {
    Ok(Json(selection_response(&svc, q.session_id.as_deref())?))
}

#[utoipa::path(
    post, path = "/knowledge/active",
    request_body = SetActiveBody,
    responses(
        (status = 200, description = "The resulting selection", body = ActiveKbResponse),
        (status = 400, description = "Unknown kb id, or a primary outside the resulting set"),
    )
)]
pub async fn set_active(
    State(svc): State<Arc<KnowledgeService>>,
    Json(body): Json<SetActiveBody>,
) -> Result<Json<ActiveKbResponse>, (StatusCode, String)> {
    let primary_id = body.primary_kb.clone().or_else(|| body.kb_id.clone());
    let primary = if body.clear_primary {
        PrimaryUpdate::Clear
    } else {
        match primary_id.as_deref() {
            Some(id) => PrimaryUpdate::Set(id),
            None => PrimaryUpdate::Unchanged,
        }
    };
    let selection = svc
        .set_selection(
            body.session_id.as_deref(),
            body.hidden_kbs.as_deref(),
            primary,
        )
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    Ok(Json(ActiveKbResponse {
        kb_ids: selection.kb_ids,
        active_kb: selection.primary_kb.clone(),
        primary_kb: selection.primary_kb,
        hidden_kbs: selection.hidden_kbs,
    }))
}
```

Delete the stale comment block at `:600-604` ("Thin pass-throughs to `KnowledgeService::{get,set}_active_persisted`") and replace it with:

```rust
// ──────────────────────────────────────────────────────────────────────────────
// GET + POST /knowledge/active — the session's knowledge-base set and its
// primary. One axis (the set, expressed as the hidden complement) plus one
// pointer. Both halves travel in one body so the primary is validated against
// the state the request produces, not the state it started from.
// ──────────────────────────────────────────────────────────────────────────────
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-server --test knowledge_routes
```

Expected: `test result: ok. 22 passed; 0 failed` (19 existing + the two new + the renamed one).

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-server/src/routes/knowledge.rs crates/biorouter-server/tests/knowledge_routes.rs
git commit -m "feat(knowledge): /knowledge/active returns the session set and a validated primary"
```

---

### Task 15 (Bug 5): a workflow applies its whole set

**Files:**
- Modify: `crates/biorouter-server/src/routes/agent.rs:87-133`

- [ ] **Step 1: Write the failing test**

Add a `#[cfg(test)] mod tests` block at the bottom of `crates/biorouter-server/src/routes/agent.rs` (or extend the existing one if present):

```rust
#[cfg(test)]
mod knowledge_selection_tests {
    use super::plan_workflow_knowledge_selection;
    use biorouter::workflow::WorkflowKnowledgeBases;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// A workflow that lists five bases used to activate exactly one, silently
    /// (`selection.default.or(selection.visible.first())`). Under the merged
    /// model `{ default, visible }` already *is* "a set plus one primary", so
    /// the whole set applies and only `default` may set the pointer.
    #[test]
    fn workflow_applies_every_declared_base() {
        let all = ids(&["a", "b", "c", "d", "e", "unrelated"]);
        let selection = WorkflowKnowledgeBases {
            default: Some("c".to_string()),
            visible: ids(&["a", "b", "c", "d", "e"]),
        };
        let (hidden, primary) = plan_workflow_knowledge_selection(&selection, &all);
        assert_eq!(hidden, ids(&["unrelated"]));
        assert_eq!(primary.as_deref(), Some("c"));
    }

    /// No `default`: one visible base is unambiguous, several are not — and
    /// picking the first of several is exactly the silent data loss being
    /// removed. A KB-less write then fails with a candidate list instead.
    #[test]
    fn workflow_without_a_default_only_infers_an_unambiguous_primary() {
        let all = ids(&["a", "b"]);
        let one = WorkflowKnowledgeBases {
            default: None,
            visible: ids(&["a"]),
        };
        assert_eq!(
            plan_workflow_knowledge_selection(&one, &all).1.as_deref(),
            Some("a")
        );

        let many = WorkflowKnowledgeBases {
            default: None,
            visible: ids(&["a", "b"]),
        };
        assert_eq!(plan_workflow_knowledge_selection(&many, &all).1, None);
    }

    /// A `default` the author forgot to list is still the author's intent, and
    /// the invariant requires the primary to be a member — so union it in
    /// rather than dropping it.
    #[test]
    fn workflow_default_joins_the_set_when_it_was_not_listed() {
        let all = ids(&["a", "b"]);
        let selection = WorkflowKnowledgeBases {
            default: Some("b".to_string()),
            visible: ids(&["a"]),
        };
        let (hidden, primary) = plan_workflow_knowledge_selection(&selection, &all);
        assert!(hidden.is_empty(), "b must not be hidden — it is the primary");
        assert_eq!(primary.as_deref(), Some("b"));
    }
}
```

- [ ] **Step 2: Run the tests — see them fail**

```bash
cargo test -p biorouter-server --lib routes::agent
```

Expected:

```
error[E0432]: unresolved import `super::plan_workflow_knowledge_selection`
```

- [ ] **Step 3: Implement**

In `crates/biorouter-server/src/routes/agent.rs`, add the pure planner above `apply_workflow_knowledge_selection` (`:87`):

```rust
/// Turn a workflow's `{ default, visible }` into "which bases to hide" and
/// "what the primary should be".
///
/// `WorkflowKnowledgeBases` already expresses a set plus one primary, which is
/// exactly the session model — so this is a translation, not a schema change.
/// Two rules earn their keep: a `default` that was not listed is unioned into
/// the set (the invariant requires the primary to be a member, and the author
/// clearly meant it), and a missing `default` only yields a primary when the
/// set has exactly one member — never the first of several.
pub(crate) fn plan_workflow_knowledge_selection(
    selection: &WorkflowKnowledgeBases,
    all_base_ids: &[String],
) -> (Vec<String>, Option<String>) {
    let mut visible: HashSet<&str> = selection.visible.iter().map(String::as_str).collect();
    if let Some(default) = selection.default.as_deref() {
        visible.insert(default);
    }
    let hidden = all_base_ids
        .iter()
        .filter(|id| !visible.contains(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let primary = match selection.default.clone() {
        Some(default) => Some(default),
        None if visible.len() == 1 => visible.iter().next().map(|id| id.to_string()),
        None => None,
    };
    (hidden, primary)
}
```

and rewrite `apply_workflow_knowledge_selection` (`:87-133`) to use it plus one atomic `set_selection`:

```rust
fn apply_workflow_knowledge_selection(
    state: &AppState,
    session_id: &str,
    workflow: &Workflow,
) -> Result<(), ErrorResponse> {
    let Some(selection) = workflow.knowledge_bases.as_ref() else {
        return Ok(());
    };

    let all_base_ids = state
        .knowledge_service
        .list_bases()
        .map_err(|err| {
            error!("Failed to list knowledge bases for workflow session: {}", err);
            ErrorResponse {
                message: format!("Failed to apply workflow knowledge bases: {}", err),
                status: StatusCode::INTERNAL_SERVER_ERROR,
            }
        })?
        .into_iter()
        .map(|base| base.id)
        .collect::<Vec<_>>();

    let (hidden, primary) = plan_workflow_knowledge_selection(selection, &all_base_ids);
    let primary = match primary.as_deref() {
        Some(id) => PrimaryUpdate::Set(id),
        None => PrimaryUpdate::Clear,
    };

    state
        .knowledge_service
        .set_selection(Some(session_id), Some(&hidden), primary)
        .map_err(|err| ErrorResponse {
            message: format!("Failed to apply workflow knowledge bases: {}", err),
            status: StatusCode::BAD_REQUEST,
        })?;

    Ok(())
}
```

Add `use biorouter_mcp::knowledge::service::PrimaryUpdate;` to the imports.

> Task 2 is what makes this reliable: a workflow that declares *every* base visible produces an empty `hidden`, which previously deleted the session override and silently handed the session the machine-wide hidden list instead of what the workflow declared.

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-server --lib routes::agent && cargo test -p biorouter --lib knowledge::soul
```

Expected: the three new tests pass, and Soul's `workflow_yaml_parses_into_a_valid_workflow` still passes — `WorkflowKnowledgeBases` is unchanged, so nothing on disk or in a deeplink moves.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-server/src/routes/agent.rs
git commit -m "fix(knowledge): apply a workflow's whole knowledge-base set instead of only the first"
```

---

### Task 16 (Bug 4): app knowledge grants compose

**Files:**
- Modify: `crates/biorouter-server/src/routes/apps.rs:1235-1247`, `:1522-1529`, and `mod tests` at `:4589`

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/biorouter-server/src/routes/apps.rs`:

```rust
    /// `configure_main_agent` and `configure_worker_agent` both called
    /// `set_active_for_session` with the SAME session id (every profile in an
    /// app shares one), so which base the app used depended on profile
    /// configuration order. A grant now joins the session's set and only the
    /// main agent's grant takes the primary.
    #[test]
    fn app_knowledge_grants_compose_and_only_main_takes_the_primary() -> anyhow::Result<()> {
        use biorouter_mcp::knowledge::service::{KnowledgeService, PrimaryUpdate};

        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        for id in ["user-kb", "main-kb", "worker-kb"] {
            svc.create_base(id, id, None)?;
        }
        // The user had narrowed this chat to their own base and chosen it.
        svc.set_selection(
            Some("app-session"),
            Some(&["main-kb".to_string(), "worker-kb".to_string()]),
            PrimaryUpdate::Set("user-kb"),
        )?;

        super::grant_knowledge_base(&svc, "app-session", "main-kb", true)?;
        super::grant_knowledge_base(&svc, "app-session", "worker-kb", false)?;

        let selection = svc.selection(Some("app-session"))?;
        assert_eq!(
            selection.kb_ids,
            vec![
                "main-kb".to_string(),
                "user-kb".to_string(),
                "worker-kb".to_string()
            ],
            "a grant adds to the session's set, it never replaces it"
        );
        assert_eq!(
            selection.primary_kb.as_deref(),
            Some("main-kb"),
            "the main agent's grant owns the primary; a worker's grant must not steal it"
        );
        Ok(())
    }
```

- [ ] **Step 2: Run the test — see it fail**

```bash
cargo test -p biorouter-server --lib routes::apps::tests::app_knowledge_grants_compose
```

Expected:

```
error[E0425]: cannot find function `grant_knowledge_base` in module `super`
```

- [ ] **Step 3: Implement**

Add the helper to `crates/biorouter-server/src/routes/apps.rs`, above `configure_main_agent`:

```rust
/// Make `kb` available to an app session without disturbing anything else.
///
/// Every profile of an app shares one session id, so per-profile KB isolation
/// has never existed — the previous `set_active_for_session` per profile was
/// simply last-writer-wins. Composing is not a widening of the sandbox: the
/// grant never restricted what the session could reach, it only chose a focus.
/// What changes is that the outcome is now stated rather than ordering-derived.
pub(crate) fn grant_knowledge_base(
    svc: &biorouter_mcp::knowledge::service::KnowledgeService,
    session_id: &str,
    kb: &str,
    make_primary: bool,
) -> anyhow::Result<()> {
    let hidden = svc
        .get_hidden_for_session_or_persisted(session_id)?
        .into_iter()
        .filter(|id| id != kb)
        .collect::<Vec<_>>();
    svc.set_selection(
        Some(session_id),
        Some(&hidden),
        if make_primary {
            biorouter_mcp::knowledge::service::PrimaryUpdate::Set(kb)
        } else {
            biorouter_mcp::knowledge::service::PrimaryUpdate::Unchanged
        },
    )?;
    Ok(())
}
```

Replace the main-agent block (`:1238-1247`):

```rust
    if let Some(kb) = report.granted_knowledge_base.clone() {
        if let Err(e) = grant_knowledge_base(&state.knowledge_service, session_id, &kb, true) {
            warn!(app = %manifest.id, kb = %kb, "grant knowledge base failed: {e}");
            report.granted_knowledge_base = None;
            report.missing_knowledge_base = Some(kb);
        }
    }
```

and the worker block (`:1522-1529`):

```rust
    if let Some(kb) = cfg.knowledge_base.as_ref() {
        // A worker's grant joins the app session's set but never takes the
        // primary — the main agent's grant owns that.
        if let Err(e) = grant_knowledge_base(&state.knowledge_service, session_id, kb, false) {
            warn!(app = %manifest.id, profile = %profile_name, kb = %kb, "worker grant knowledge base failed: {e}");
        }
    }
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-server --lib routes::apps
```

Expected: `test result: ok. 55 passed; 0 failed` (the existing ~54 plus the new one).

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-server/src/routes/apps.rs
git commit -m "fix(apps): compose knowledge-base grants instead of overwriting the session's selection"
```

---

### Task 17: Regenerate the OpenAPI spec and the TypeScript client

**Files:**
- Generated: `ui/desktop/openapi.json`, `ui/desktop/src/api/types.gen.ts`, `ui/desktop/src/api/sdk.gen.ts`, `ui/desktop/src/api/index.ts`

- [ ] **Step 1: Regenerate**

```bash
source bin/activate-hermit
just generate-openapi
cd ui/desktop && npm run generate-api
```

- [ ] **Step 2: Verify the new wire shape landed**

```bash
grep -n 'ActiveKbResponse' -A 8 ui/desktop/src/api/types.gen.ts
```

Expected:

```ts
export type ActiveKbResponse = {
    /** Deprecated mirror of `primary_kb`. */
    active_kb?: string | null;
    hidden_kbs: Array<string>;
    kb_ids: Array<string>;
    primary_kb?: string | null;
};
```

- [ ] **Step 3: Confirm nothing was hand-edited**

```bash
git diff --stat ui/desktop/openapi.json ui/desktop/src/api/
```

Expected: only the four generated files changed. `ui/desktop/src/workflow/index.ts` must **not** appear — `WorkflowKnowledgeBases` is unchanged by design (D8).

- [ ] **Step 4: Commit**

```bash
git add ui/desktop/openapi.json ui/desktop/src/api/
git commit -m "chore(api): regenerate OpenAPI spec and TypeScript client for the knowledge selection"
```

---
## Phase 4 — CLI

The CLI has **no session concept** — `handle_active`, `handle_hide`, `handle_unhide` and `resolve_kb` are all machine-wide by construction. That is correct, not a bug (D11). What it owes the user is that it cannot pin a base it refuses to list, and that it never writes into a base it did not name first.

### Task 18: `knowledge active` and `knowledge list` speak primary

**Files:**
- Modify: `crates/biorouter-cli/src/cli.rs:899-924`
- Modify: `crates/biorouter-cli/src/commands/knowledge.rs:89-147` (`handle_list`), `:194-232` (`handle_active`), `:238-259` (`handle_create`)

- [ ] **Step 1: Write the failing test**

Add a test module at the bottom of `crates/biorouter-cli/src/commands/knowledge.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{active_command, render_list};
    use biorouter::knowledge::service::{KnowledgeService, PrimaryUpdate};

    fn svc() -> (tempfile::TempDir, KnowledgeService) {
        let tmp = tempfile::TempDir::new().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("alpha", "Alpha", None).unwrap();
        svc.create_base("beta", "Beta", None).unwrap();
        (tmp, svc)
    }

    /// First-ever CLI coverage for the knowledge commands. `--set` used to
    /// validate only that the base existed, so it would happily pin a base the
    /// CLI hides from the agent — a primary outside the set.
    #[test]
    fn active_command_shows_sets_validates_and_clears_the_primary() -> anyhow::Result<()> {
        let (_tmp, svc) = svc();

        assert!(active_command(&svc, None, false)?.contains("no primary knowledge base"));
        assert!(active_command(&svc, Some("beta".to_string()), false)?.contains("beta"));
        assert!(active_command(&svc, None, false)?.contains("beta"));

        svc.set_hidden_persisted(&["alpha".to_string()])?;
        let err = active_command(&svc, Some("alpha".to_string()), false)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("alpha") && err.contains("knowledge list"),
            "a hidden base cannot be the primary, and the error must say how to look, got: {err}"
        );

        assert!(active_command(&svc, None, true)?.contains("cleared"));
        assert_eq!(svc.primary_for_session(None)?, None);
        Ok(())
    }

    /// `cli.rs:901` has promised "the active one is marked" since the
    /// focus/discovery split; `handle_list` only ever marked hidden-vs-visible.
    #[test]
    fn list_marks_the_primary_base() -> anyhow::Result<()> {
        let (_tmp, svc) = svc();
        svc.set_selection(None, None, PrimaryUpdate::Set("beta"))?;

        let text = render_list(&svc, "text")?;
        assert!(text.contains("beta") && text.contains("primary"), "got: {text}");

        let json: serde_json::Value = serde_json::from_str(&render_list(&svc, "json")?)?;
        assert_eq!(json["primary_kb"], serde_json::json!("beta"));
        let beta = json["bases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["id"] == serde_json::json!("beta"))
            .unwrap();
        assert_eq!(beta["primary"], serde_json::json!(true));
        Ok(())
    }
}
```

- [ ] **Step 2: Run the tests — see them fail**

```bash
cargo test -p biorouter-cli --lib commands::knowledge
```

Expected:

```
error[E0432]: unresolved imports `super::active_command`, `super::render_list`
```

- [ ] **Step 3: Implement**

In `crates/biorouter-cli/src/commands/knowledge.rs`, add the import:

```rust
use biorouter::knowledge::service::PrimaryUpdate;
```

Replace `handle_active` (`:194-232`) with a thin printer over a testable core:

```rust
pub async fn handle_active(set: Option<String>, clear: bool) -> Result<()> {
    let svc = service()?;
    println!("{}", active_command(&svc, set, clear)?);
    Ok(())
}

/// Show, set or clear the **primary** knowledge base — the base a `--kb`-less
/// ingest/query/lint writes to. Setting one validates membership: a base the
/// CLI hides from the agent can never be the primary.
fn active_command(svc: &KnowledgeService, set: Option<String>, clear: bool) -> Result<String> {
    if clear {
        svc.set_selection(None, None, PrimaryUpdate::Clear)?;
        return Ok(format!(
            "  {} primary knowledge base cleared",
            style("✓").green()
        ));
    }

    if let Some(id) = set {
        svc.set_selection(None, None, PrimaryUpdate::Set(&id))
            .map_err(|e| anyhow!("{e} Run `biorouter knowledge list` to see them."))?;
        return Ok(format!(
            "  {} primary knowledge base set to {}",
            style("✓").green(),
            style(&id).fg(ACCENT).bold()
        ));
    }

    Ok(match svc.primary_for_session(None)? {
        Some(id) => format!(
            "  {} {}",
            style("primary:").dim(),
            style(id).fg(ACCENT).bold()
        ),
        None => format!(
            "  {}",
            style("no primary knowledge base (use --set <id>)").dim()
        ),
    })
}
```

Replace `handle_list` (`:89-147`) with the same split — keep the body verbatim, but build into a `String` and add the primary marker:

```rust
pub async fn handle_list(format: &str) -> Result<()> {
    let svc = service()?;
    println!("{}", render_list(&svc, format)?);
    Ok(())
}

fn render_list(svc: &KnowledgeService, format: &str) -> Result<String> {
    let bases = svc.list_bases()?;
    let hidden = svc.get_hidden_persisted().unwrap_or_default();
    let primary = svc.primary_for_session(None)?;

    if format == "json" {
        return Ok(serde_json::json!({
            "primary_kb": primary,
            "bases": bases.iter().map(|b| serde_json::json!({
                "id": b.id, "name": b.name, "color": b.color,
                "hidden": hidden.contains(&b.id),
                "primary": primary.as_deref() == Some(b.id.as_str()),
            })).collect::<Vec<_>>(),
        })
        .to_string());
    }

    let mut out = String::new();
    out.push_str(&format!(
        "  {} {}\n",
        style("▌").fg(ACCENT),
        style("Knowledge bases").bold()
    ));
    if bases.is_empty() {
        out.push_str(&format!(
            "    {}",
            style("none yet — create one with `biorouter knowledge create <id> --name <name>`")
                .dim()
        ));
        return Ok(out);
    }
    out.push_str(&format!(
        "    {}\n",
        style("Visible bases are available to the agent; the primary is where a --kb-less ingest writes.")
            .dim()
    ));
    let width = bases.iter().map(|b| b.id.len()).max().unwrap_or(0);
    for base in &bases {
        let is_hidden = hidden.contains(&base.id);
        let marker = if is_hidden {
            style("○").dim().to_string()
        } else {
            style("●").fg(ACCENT).to_string()
        };
        let suffix = if is_hidden {
            style("  (hidden)").dim().to_string()
        } else if primary.as_deref() == Some(base.id.as_str()) {
            style("  (primary)").fg(ACCENT).to_string()
        } else {
            String::new()
        };
        out.push_str(&format!(
            "    {} {:<width$}  {}{}\n",
            marker,
            style(&base.id).bold(),
            style(&base.name).dim(),
            suffix,
            width = width
        ));
    }
    Ok(out)
}
```

(The `section` helper is now only used by other commands; leave it.)

In `handle_create` (`:252-257`), **delete** the auto-promote.

> **Superseded during review.** This step originally read "make it the primary
> when there was no prior choice, so the next `--kb`-less ingest/query just
> works", and prescribed the snippet below:
>
> ```rust
>     if svc.primary_for_session(None)?.is_none() {
>         svc.set_selection(None, None, PrimaryUpdate::Set(&manifest.id))?;
>         println!("  {} set as the primary knowledge base", style("·").dim());
>     }
> ```
>
> That is exactly the invention the merged model forbids. The primary is where
> a `--kb`-less write *commits*, so a pointer the user never chose sends an
> ingest into a base by accident — as a git commit in that base's history that
> is easy to miss. "Exactly one candidate" is still a candidate, not a choice.

Create the base and only create the base. When there is still no primary
afterwards, name the remedy instead of guessing at it:

```rust
    if svc.primary_for_session(None)?.is_none() {
        out.push_str(&format!(
            "  {}\n",
            style("no primary knowledge base yet — set one with \
                   `biorouter knowledge active --set <id>`")
                .dim()
        ));
    }
```

With no primary, a KB-less command fails and lists the candidates
(`resolve_kb`), which is the behaviour the model asks for.

In `crates/biorouter-cli/src/cli.rs`, fix the stale doc comment (`:901`) and reword `Active` (`:913-924`):

```rust
    /// List knowledge bases (hidden ones are dimmed; the primary is marked)
    #[command(about = "List knowledge bases")]
```

```rust
    /// Show, set, or clear the primary knowledge base
    #[command(about = "Show or set the primary knowledge base (the --kb-less write target)")]
    Active {
        #[arg(
            long = "set",
            value_name = "ID",
            help = "Make this base the primary (it must not be hidden)"
        )]
        set: Option<String>,
        #[arg(long = "clear", help = "Clear the primary knowledge base")]
        clear: bool,
    },
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-cli --lib commands::knowledge
```

Expected: `test result: ok. 2 passed; 0 failed`.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-cli/src/cli.rs crates/biorouter-cli/src/commands/knowledge.rs
git commit -m "feat(cli): knowledge active/list speak primary and validate membership"
```

---

### Task 19: a `--kb`-less CLI write names the base before it writes

**Files:**
- Modify: `crates/biorouter-cli/src/commands/knowledge.rs:28-41` (`resolve_kb`) and its four call sites (`:276`, `:375`, `:465`, `:546`)
- Modify: `crates/biorouter-cli/src/session/output.rs:1401-1406`

- [ ] **Step 1: Write the failing test**

Add to the CLI test module:

```rust
    /// A `--kb`-less ingest/query/lint resolves its target silently. It must
    /// hand back a notice so the command can say where it is about to write —
    /// an ingest commits to that base's git history and is hard to notice
    /// afterwards.
    #[test]
    fn resolve_kb_names_the_primary_and_lists_candidates_when_there_is_none() -> anyhow::Result<()> {
        let (_tmp, svc) = svc();

        let err = super::resolve_kb(&svc, None).unwrap_err().to_string();
        assert!(
            err.contains("alpha, beta") && err.contains("--kb"),
            "with no primary the error must list the candidates, got: {err}"
        );

        svc.set_selection(None, None, PrimaryUpdate::Set("beta"))?;
        let (id, notice) = super::resolve_kb(&svc, None)?;
        assert_eq!(id, "beta");
        assert!(
            notice.expect("a resolved primary must be announced").contains("beta"),
            "the notice must name the base"
        );

        let (id, notice) = super::resolve_kb(&svc, Some("alpha".to_string()))?;
        assert_eq!(id, "alpha");
        assert!(notice.is_none(), "an explicit --kb needs no notice");
        Ok(())
    }
```

- [ ] **Step 2: Run the test — see it fail**

```bash
cargo test -p biorouter-cli --lib commands::knowledge
```

Expected:

```
error[E0308]: mismatched types
   expected `Result<(String, Option<String>), _>`, found `Result<String, _>`
```

- [ ] **Step 3: Implement**

Replace `resolve_kb` (`crates/biorouter-cli/src/commands/knowledge.rs:28-41`):

```rust
/// Resolve the base a command operates on: the explicit `--kb` flag, else the
/// primary. Returns the id and, when it was resolved rather than given, a
/// notice the caller must print *before* doing any work — a KB-less write
/// must never be silent about which base it landed in.
fn resolve_kb(
    svc: &KnowledgeService,
    explicit: Option<String>,
) -> Result<(String, Option<String>)> {
    if let Some(id) = explicit {
        return Ok((id, None));
    }
    if let Some(id) = svc.primary_for_session(None)? {
        let notice = format!(
            "  {} using primary knowledge base {}",
            style("·").dim(),
            style(&id).fg(ACCENT).bold()
        );
        return Ok((id, Some(notice)));
    }
    let ids = svc.session_kb_ids(None)?;
    if ids.is_empty() {
        bail!("No knowledge bases yet. Create one with `biorouter knowledge create <id> --name <name>`.");
    }
    bail!(
        "No primary knowledge base. Pass --kb <id> (one of: {}), or set one with \
         `biorouter knowledge active --set <id>`.",
        ids.join(", ")
    )
}
```

At each of the four call sites (`:276`, `:375`, `:465`, `:546`) replace

```rust
    let kb_id = resolve_kb(&svc, kb)?;
```

with

```rust
    let (kb_id, notice) = resolve_kb(&svc, kb)?;
    if let Some(notice) = notice {
        println!("{notice}");
    }
```

(`:375` is inside an `else` arm of a `let` — bind it first and yield `kb_id`:)

```rust
    } else {
        let (kb_id, notice) = resolve_kb(&svc, kb)?;
        if let Some(notice) = notice {
            println!("{notice}");
        }
        kb_id
    };
```

In `crates/biorouter-cli/src/session/output.rs:1403-1405`, follow the rename and label the row for what it is:

```rust
    if let Ok(svc) = biorouter::knowledge::service::KnowledgeService::new_default() {
        if let Ok(Some(kb)) = svc.primary_for_session(None) {
            row("knowledge", kb);
        }
    }
```

- [ ] **Step 4: Verify**

```bash
cargo test -p biorouter-cli --lib commands::knowledge && cargo build -p biorouter-cli
```

Expected: three tests pass, clean build.

- [ ] **Step 5: Commit**

```bash
git add crates/biorouter-cli/src/commands/knowledge.rs crates/biorouter-cli/src/session/output.rs
git commit -m "fix(cli): name the primary knowledge base before a --kb-less write"
```

---
## Phase 5 — Desktop GUI

### Task 20: `KnowledgeContext` holds a primary, and lets the daemon own the repair rule

**Files:**
- Create: `ui/desktop/src/components/knowledge/KnowledgeContext.test.tsx`
- Modify: `ui/desktop/src/components/knowledge/KnowledgeContext.tsx`

- [ ] **Step 1: Write the failing test**

Create `ui/desktop/src/components/knowledge/KnowledgeContext.test.tsx`:

```tsx
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { KnowledgeProvider, useKnowledge } from './KnowledgeContext';

const mocks = vi.hoisted(() => ({
  listBases: vi.fn(),
  getActive: vi.fn(),
  setActive: vi.fn(),
}));

vi.mock('../../api', () => ({
  listBases: mocks.listBases,
  getActive: mocks.getActive,
  setActive: mocks.setActive,
}));

function base(id: string) {
  return { id, name: id, color: '#cf6d47', created_at: '', schema_version: 1 };
}

function Probe() {
  const { primaryKbId, hiddenKbIds, setPrimaryKbId } = useKnowledge();
  return (
    <div>
      <span data-testid="primary">{primaryKbId ?? 'none'}</span>
      <span data-testid="hidden">{hiddenKbIds.join(',') || 'none'}</span>
      <button type="button" onClick={() => setPrimaryKbId('beta')}>
        make beta primary
      </button>
    </div>
  );
}

function renderProvider() {
  return render(
    <KnowledgeProvider sessionId="chat-1">
      <Probe />
    </KnowledgeProvider>
  );
}

beforeEach(() => {
  localStorage.clear();
  vi.clearAllMocks();
  mocks.listBases.mockResolvedValue({ data: [base('alpha'), base('beta')] });
  mocks.getActive.mockResolvedValue({
    data: { kb_ids: ['alpha'], primary_kb: 'alpha', active_kb: 'alpha', hidden_kbs: ['beta'] },
  });
  mocks.setActive.mockResolvedValue({
    data: { kb_ids: ['alpha', 'beta'], primary_kb: 'beta', active_kb: 'beta', hidden_kbs: [] },
  });
});

describe('KnowledgeContext', () => {
  it('hydrates the primary and the set from the daemon', async () => {
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));
    expect(screen.getByTestId('hidden')).toHaveTextContent('beta');
  });

  // The invariant, at the UI edge: the primary must be a member of the set, so
  // "make primary" on a base that is toggled off is ONE request that does both
  // and is validated by the daemon against the state it produces.
  it('makes a base primary and adds it to the chat in the same request', async () => {
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

    await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));

    await waitFor(() => expect(mocks.setActive).toHaveBeenCalled());
    const body = mocks.setActive.mock.calls.at(-1)?.[0]?.body;
    expect(body.primary_kb).toBe('beta');
    expect(body.hidden_kbs).toEqual([]);
    expect(body.session_id).toBe('chat-1');
  });

  // The promote/clear rule lives in the daemon. If the UI re-derived it, the
  // two would drift and the chat chip would disagree with the model.
  it('adopts the primary the daemon reports back', async () => {
    mocks.setActive.mockResolvedValue({
      data: { kb_ids: ['alpha'], primary_kb: 'alpha', active_kb: 'alpha', hidden_kbs: ['beta'] },
    });
    renderProvider();
    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));

    await userEvent.click(screen.getByRole('button', { name: 'make beta primary' }));

    await waitFor(() => expect(screen.getByTestId('primary')).toHaveTextContent('alpha'));
  });
});
```

- [ ] **Step 2: Run the test — see it fail**

```bash
cd ui/desktop && npx vitest run src/components/knowledge/KnowledgeContext.test.tsx
```

Expected: the context has no such field.

```
TypeError: setPrimaryKbId is not a function
```

- [ ] **Step 3: Implement**

In `ui/desktop/src/components/knowledge/KnowledgeContext.tsx`:

Rename in the interface (`:25-42`):

```ts
interface KnowledgeContextType {
  bases: Manifest[];
  /** The session's knowledge bases — the one axis. Searchable, readable, usable. */
  visibleBases: Manifest[];
  loading: boolean;
  /** The KB-less write target and the Knowledge view's subject. Always a member of visibleBases, or null. */
  primaryKb: Manifest | null;
  primaryKbId: string | null;
  hiddenKbIds: string[];
  setPrimaryKbId: (id: string | null) => void;
  setHiddenKbIds: (ids: string[]) => void;
  toggleKbHidden: (id: string) => void;
  hideAllKnowledgeBases: () => void;
  showAllKnowledgeBases: () => void;
  refresh: () => Promise<void>;
  registerGraphRefresh: (fn: (() => Promise<void>) | null) => void;
  triggerGraphRefresh: () => void;
}
```

Rename the state hook (`:57-59`) to `primaryKbId` / `setPrimaryKbIdState`, and replace `syncSelection` (`:76-95`) and `setActiveKbId` (`:97-102`):

```tsx
  const syncSelection = useCallback(
    (nextPrimaryKbId: string | null, nextHiddenKbIds: string[]) => {
      setPrimaryKbIdState(nextPrimaryKbId);
      setHiddenKbIdsState(nextHiddenKbIds);
      if (nextPrimaryKbId) localStorage.setItem(storageKey, nextPrimaryKbId);
      else localStorage.removeItem(storageKey);
      localStorage.setItem(hiddenStorageKey, JSON.stringify(nextHiddenKbIds));
      void setActive({
        body: {
          primary_kb: nextPrimaryKbId ?? undefined,
          clear_primary: nextPrimaryKbId === null,
          hidden_kbs: nextHiddenKbIds,
          session_id: sessionId || undefined,
        },
        throwOnError: false,
      })
        .then((res) => {
          // The daemon owns the "primary must be a member" repair: hiding the
          // primary promotes to the first remaining base, hiding everything
          // clears it. Adopt its answer instead of re-implementing that rule
          // here, where the two would silently drift apart.
          const applied = res?.data?.primary_kb ?? null;
          setPrimaryKbIdState(applied);
          if (applied) localStorage.setItem(storageKey, applied);
          else localStorage.removeItem(storageKey);
        })
        .catch((err) => {
          console.warn('setActive (server sync) failed:', err);
        });
    },
    [hiddenStorageKey, sessionId, storageKey]
  );

  const setPrimaryKbId = useCallback(
    (id: string | null) => {
      // The primary must be a member of the set, so making a base primary adds
      // it to this chat in the same request — one gesture, one POST.
      const nextHidden = id ? hiddenKbIds.filter((hiddenId) => hiddenId !== id) : hiddenKbIds;
      syncSelection(id, nextHidden);
    },
    [hiddenKbIds, syncSelection]
  );
```

`setHiddenKbIds` (`:104-110`) keeps passing `primaryKbId` through — the daemon's response then corrects it if that hide orphaned the pointer.

In the hydrate effect (`:194`) read the new field with the deprecated fallback:

```ts
        const server = res.data?.primary_kb ?? res.data?.active_kb ?? null;
```

Rename the remaining references: the prune effect (`:155-159`), `activeKb` → `primaryKb` (`:215-218`), and the context value object (`:224-239`). The `storageKeyForSession` constants stay exactly as they are — renaming them would strand every existing key and silently break `ResetPanel.clearKnowledgeSelections`, which prefix-scans `'knowledge_active_kb'`.

- [ ] **Step 4: Verify**

```bash
cd ui/desktop && npx vitest run src/components/knowledge/KnowledgeContext.test.tsx
```

Expected: `Test Files  1 passed (1)` / `Tests  3 passed (3)`.

- [ ] **Step 5: Commit**

```bash
git add ui/desktop/src/components/knowledge/KnowledgeContext.tsx ui/desktop/src/components/knowledge/KnowledgeContext.test.tsx
git commit -m "feat(knowledge-ui): hold a primary KB and adopt the daemon's membership repair"
```

---

### Task 21: the palette row carries two states, and picking a primary does not close it

**Files:**
- Create: `ui/desktop/src/components/knowledge/KBSelector/KBSelectorPalette.test.tsx`
- Modify: `ui/desktop/src/components/knowledge/KBSelector/KBSelectorPalette.tsx:37`, `:127`, `:132-133`, `:302-345`
- Modify: `ui/desktop/src/components/knowledge/KBSelector/KBSelectorTrigger.tsx:14`, `:31`, `:34`

- [ ] **Step 1: Write the failing test**

Create `ui/desktop/src/components/knowledge/KBSelector/KBSelectorPalette.test.tsx`:

```tsx
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { KBSelectorPalette } from './KBSelectorPalette';

const mocks = vi.hoisted(() => ({
  setPrimaryKbId: vi.fn(),
  toggleKbHidden: vi.fn(),
  refresh: vi.fn().mockResolvedValue(undefined),
  onClose: vi.fn(),
}));

vi.mock('../KnowledgeContext', () => ({
  useKnowledge: () => ({
    bases: [
      { id: 'alpha', name: 'Alpha', color: '#cf6d47' },
      { id: 'beta', name: 'Beta', color: '#b85a32' },
    ],
    primaryKbId: 'alpha',
    hiddenKbIds: ['beta'],
    refresh: mocks.refresh,
    setPrimaryKbId: mocks.setPrimaryKbId,
    toggleKbHidden: mocks.toggleKbHidden,
  }),
}));

vi.mock('../hooks/useKnowledgeBases', () => ({
  useKnowledgeBases: () => ({
    create: vi.fn(),
    exportArchive: vi.fn(),
    importArchive: vi.fn(),
    remove: vi.fn(),
    rename: vi.fn(),
  }),
}));

beforeAll(() => {
  // The palette pulls in Radix primitives that observe their trigger.
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    }
  );
});

afterAll(() => vi.unstubAllGlobals());

beforeEach(() => vi.clearAllMocks());

describe('KBSelectorPalette', () => {
  // Two states per row, never three. Under the merged model membership and
  // the primary are the only two things a base can be, and the row body is
  // the "make primary" affordance.
  it('offers exactly one membership switch per row', () => {
    render(<KBSelectorPalette onClose={mocks.onClose} />);
    expect(screen.getByLabelText('Include Alpha in this chat')).toBeInTheDocument();
    expect(screen.getByLabelText('Include Beta in this chat')).toBeInTheDocument();
    expect(screen.getAllByRole('switch')).toHaveLength(2);
  });

  // Picking a primary used to close the palette, which made the selector feel
  // like a radio group over a single-active model. It is now a place you stay.
  it('makes a base primary without closing the palette', async () => {
    render(<KBSelectorPalette onClose={mocks.onClose} />);
    await userEvent.click(screen.getByText('Beta'));
    expect(mocks.setPrimaryKbId).toHaveBeenCalledWith('beta');
    expect(mocks.onClose).not.toHaveBeenCalled();
  });

  it('marks the primary', () => {
    render(<KBSelectorPalette onClose={mocks.onClose} />);
    expect(screen.getByText('Primary')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test — see it fail**

```bash
cd ui/desktop && npx vitest run src/components/knowledge/KBSelector/KBSelectorPalette.test.tsx
```

Expected:

```
TestingLibraryElementError: Unable to find a label with the text of: Include Alpha in this chat
```

- [ ] **Step 3: Implement**

In `ui/desktop/src/components/knowledge/KBSelector/KBSelectorPalette.tsx`:

`:37` —

```tsx
  const { bases, primaryKbId, hiddenKbIds, refresh, setPrimaryKbId, toggleKbHidden } =
    useKnowledge();
```

`:127` and `:132-133` — `setActiveKbId(...)` → `setPrimaryKbId(...)`, and `activeKbId === draftMode.base.id` → `primaryKbId === draftMode.base.id`.

`:302` —

```tsx
                  const isPrimary = primaryKbId === base.id;
```

and use `isPrimary` for the row highlight at `:308`.

`:311-317` — the row body becomes "make primary" and stops closing:

```tsx
                      <button
                        type="button"
                        // Making a base primary is not a navigation: the palette
                        // is where the whole selection is managed, so it stays open.
                        onClick={() => setPrimaryKbId(base.id)}
                        className="flex min-w-0 flex-1 items-center gap-3 text-left"
                      >
```

`:331-335` — the off-state badge:

```tsx
                            {hidden && (
                              <Badge uppercase className="text-[10px]">
                                Not in this chat
                              </Badge>
                            )}
```

`:341-345` — the primary badge:

```tsx
                        {isPrimary && (
                          <Badge uppercase tone="accent" className="text-[10px]">
                            Primary
                          </Badge>
                        )}
```

and the switch's label (`:353-358`):

```tsx
                          <Switch
                            checked={!hidden}
                            onCheckedChange={() => toggleKbHidden(base.id)}
                            variant="mono"
                            aria-label={`Include ${base.name} in this chat`}
                          />
```

In `ui/desktop/src/components/knowledge/KBSelector/KBSelectorTrigger.tsx`:

```tsx
export function KBSelectorTrigger({ open: openProp, onOpenChange }: Props) {
  const { primaryKb, visibleBases } = useKnowledge();
```

```tsx
        <span
          className="h-2 w-2 flex-shrink-0 rounded-full"
          style={{ background: primaryKb?.color ?? 'var(--text-muted)' }}
        />
        <span className="min-w-0 flex-1 truncate text-left font-semibold">
          {primaryKb?.name ?? 'No primary knowledge base'}
        </span>
        {visibleBases.length > 1 && (
          <span className="shrink-0 text-[11px] text-text-muted">
            +{visibleBases.length - 1}
          </span>
        )}
```

> The `data-testid="knowledge-kb-selector-trigger"` and the primary's **name** both stay, so `ui/desktop/tests/e2e/knowledge-ingest.spec.ts:65` and `:110` — which assert the trigger contains the KB name — keep passing.

- [ ] **Step 4: Verify**

```bash
cd ui/desktop && npx vitest run src/components/knowledge/
```

Expected: `Tests  7 passed (7)` (3 context + 3 palette + 1 pre-existing KnowledgeView).

- [ ] **Step 5: Commit**

```bash
git add ui/desktop/src/components/knowledge/KBSelector/
git commit -m "feat(knowledge-ui): membership switch plus make-primary row, never a third toggle"
```

---

### Task 22: the remaining consumers follow the primary

**Files:**
- Modify: `ui/desktop/src/components/knowledge/IngestPanel/IngestPanel.tsx`, `graph/KnowledgeGraphPanel.tsx`, `changelog/ChangeLogDrawer.tsx`, `hooks/useKnowledgeBases.ts`, `ui/desktop/src/components/MentionPopover.tsx:539-546`

- [ ] **Step 1: Find every remaining reference**

```bash
cd ui/desktop && grep -rn 'activeKbId\|activeKb\b\|setActiveKbId\|active_kb' src/ --include='*.tsx' --include='*.ts' | grep -v '/api/'
```

Expected before the change: `IngestPanel.tsx` (9), `KnowledgeGraphPanel.tsx` (13), `ChangeLogDrawer.tsx` (2), `useKnowledgeBases.ts` (4), `MentionPopover.tsx` (2).

- [ ] **Step 2: Apply the rename**

```bash
cd ui/desktop
# Order matters and there are no word boundaries in BSD sed: the two longer
# identifiers are rewritten first, so the bare `activeKb` pattern can only
# match what is left.
sed -i '' \
  -e 's/setActiveKbId/setPrimaryKbId/g' \
  -e 's/activeKbId/primaryKbId/g' \
  -e 's/activeKb/primaryKb/g' \
  src/components/knowledge/IngestPanel/IngestPanel.tsx \
  src/components/knowledge/graph/KnowledgeGraphPanel.tsx \
  src/components/knowledge/changelog/ChangeLogDrawer.tsx \
  src/components/knowledge/hooks/useKnowledgeBases.ts
```

Then two edits by hand.

`ui/desktop/src/components/knowledge/graph/KnowledgeGraphPanel.tsx:54`:

```tsx
            {primaryKb?.name ?? 'No primary knowledge base'}
```

`ui/desktop/src/components/MentionPopover.tsx:539-546` — this file calls the API directly and does not use `KnowledgeContext`, so a context-only refactor leaves it stale:

```tsx
        const hiddenKbIds = new Set(activeResponse.data?.hidden_kbs ?? []);
        const primaryKbId =
          activeResponse.data?.primary_kb ?? activeResponse.data?.active_kb ?? null;
        for (const base of basesResponse.data ?? []) {
          if (hiddenKbIds.has(base.id)) continue;
          commandItems.push({
            name: `kb:${base.name}`,
            extra: `${primaryKbId === base.id ? 'Primary knowledge base' : 'Knowledge base in this chat'} · ${base.id}`,
            itemType: 'KnowledgeBase',
            relativePath: base.id,
          });
        }
```

- [ ] **Step 3: Verify**

```bash
cd ui/desktop && npm run typecheck && npx vitest run src/components/knowledge/ src/components/bottom_menu/
```

Expected: a clean typecheck, and the chat-chip suite untouched and green — none of the context fields `BottomMenuKnowledgeSelection` consumes were renamed.

```
Test Files  3 passed (3)
```

Confirm nothing stale survives:

```bash
grep -rn 'activeKbId\|setActiveKbId' src/ --include='*.tsx' --include='*.ts'
```

Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add ui/desktop/src/components/
git commit -m "refactor(knowledge-ui): point the ingest, graph, change-log and mention surfaces at the primary"
```

---

## Phase 6 — Documentation and gates

### Task 23: Document the merged model

**Files:**
- Modify: `CLAUDE.md` (the "Knowledge feature" section), `docs/knowledge-base/README.md`

- [ ] **Step 1: Update `CLAUDE.md`**

In the **Knowledge feature** section, replace the storage-layout bullet:

```markdown
- **Storage layout:** `~/.config/biorouter/knowledge/<kb-id>/` with `raw/`, `knowledge/`, `index.md`, `log.md`, `schema.md`, and a hidden `.git/`. The active-KB id is persisted at `~/.config/biorouter/knowledge/.active-kb`.
```

with:

```markdown
- **Storage layout:** `~/.config/biorouter/knowledge/<kb-id>/` with `raw/`, `knowledge/`, `index.md`, `log.md`, `schema.md`, and a hidden `.git/`.
- **One axis, one pointer.** A session's knowledge bases are the *visible* set — everything not in `.hidden-kbs` (machine-wide) or `.hidden-kb-sessions/<sha256(session_id)>` (per session, and an empty `[]` there means "this chat hides nothing", not "inherit"). Every base in the set is searched by a `kb_id`-less `kb_search`, with per-hit `kb_id` attribution. One member is the **primary**, persisted as a bare id in `.active-kb` / `.active-kb-sessions/<digest>` (historical filenames, kept so a lagging PATH-installed CLI still reads a valid id): it is the write target for KB-less mutating calls, the default for single-base reads, and the Knowledge view's subject. On a fresh install, after reset, or after the machine preference is restored to its product default, the built-in Soul base is primary. An explicit user choice or explicit clear persists. The primary is always a member of the set — hiding its base promotes to the lexicographically first remaining one (identically whether the chat pinned that primary itself or was merely displaying the machine-wide one, since the two are indistinguishable to the user; the promotion is written at the chat's own scope so the machine pointer stays put for every other chat), while deleting its base clears it. There is no third "active" collection; `kb_set_active` moves the primary and does not narrow search.
```

- [ ] **Step 2: Update `docs/knowledge-base/README.md`**

Add a row to the Documents table:

```markdown
| [One knowledge-base set per session, one primary](multi-kb-implementation-plan.md) | The merged-axes design: the session's visible set *is* its knowledge-base set, and the old single `active_kb` becomes an explicit primary pointer (KB-less write target, default single-base read, Knowledge-view subject) that is always a member of the set. Also carries the fixes for six pre-existing bugs the survey found. |
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md docs/knowledge-base/README.md
git commit -m "docs(knowledge): describe the merged set-plus-primary model"
```

---

### Task 24: Final gates

- [ ] **Step 1: Rust**

```bash
cargo test -p biorouter-mcp --lib knowledge::
cargo test -p biorouter-mcp --test knowledge_macros_e2e --test knowledge_registered --test knowledge_revert_integration
cargo test -p biorouter-server --test knowledge_routes
cargo test -p biorouter-server --lib -- routes::apps routes::agent routes::knowledge
cargo test -p biorouter-cli
cargo test -p biorouter --lib knowledge:: --lib agents::knowledge_tool
cargo test -p biorouter --test knowledge_e2e
```

Expected: every suite green.

- [ ] **Step 2: Whole workspace**

```bash
cargo test
```

Expected: green, or only failures that were already failing on `a01be9b7` — capture that baseline first with `git stash && cargo test 2>&1 | tail -40 && git stash pop` if anything looks pre-existing.

- [ ] **Step 3: Style**

```bash
cargo fmt && ./scripts/clippy-lint.sh
cd ui/desktop && npm run lint:check
```

Expected: no diff, no warnings.

- [ ] **Step 4: Frontend**

```bash
cd ui/desktop && npm run test:run
```

Expected: green, with the three new context tests and three new palette tests included.

- [ ] **Step 5: Generated client is in sync**

```bash
source bin/activate-hermit
just generate-openapi && cd ui/desktop && npm run generate-api
git diff --stat ui/desktop/openapi.json ui/desktop/src/api/
```

Expected: **no diff** — Task 17 already regenerated, and nothing since then touched a route signature.

- [ ] **Step 6: Manual smoke (optional but recommended)**

Read [`docs/desktop-ui/launching-the-dev-gui.md`](../desktop-ui/launching-the-dev-gui.md) first.

```bash
env -u ELECTRON_RUN_AS_NODE BIOROUTER_NO_HMR=1 just run-dev
```

Check, in order: the Knowledge palette shows one switch and a `PRIMARY` badge; clicking a row's name moves the badge without closing the palette; toggling the primary's switch off moves the badge to another row instead of leaving none; the chat chip count still tracks the same set; the ingest panel targets the base named in the trigger.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "chore(knowledge): final gate — full suite, lint and generated client clean" || echo "nothing to commit"
```

---

## Self-review

**The brief, item by item.**

| Requirement | Where |
|---|---|
| Merge the axes; the visible set is the session's set | D1; no code needed for search or the chat chip — both already do this |
| `active_kb` replaced by an explicit primary pointer | D2, Tasks 3, 4, 10, 14, 18, 20 |
| Primary must be a member; promote or clear deterministically, and test it | D2, Task 4 (`primary_must_be_a_member_of_the_session_set`), Task 14 (`set_only_edit_keeps_the_primary_until_it_leaves_the_set`) |
| No KB row ever carries three toggles | D12, Task 21 (`offers exactly one membership switch per row`) |
| Migration: today's `active_kb` becomes the primary; a legacy read *is* the migration | D3, Task 3 — the file format does not change at all |
| A downgrade cannot be corrupted, and the plan says why | D3 — an older binary reads the same bare id it always read; nothing structured is ever written to `.active-kb` |
| KB-less writes target the primary and name the base | Task 12 (`ingest_summary_names_the_target_base`), Task 19 (`resolve_kb` returns a notice) |
| No primary ⇒ the error says exactly how to set one | Task 9 (MCP), Task 12 (ingest), Task 19 (CLI) — all list the candidates |
| Instructions + the five tool descriptions rewritten, with a pinning test | Tasks 9, 10, 11 |
| Bug 1 — ingest targets the machine-wide KB | Task 12 (anchor corrected to `knowledge_tool.rs:121`; see D6) |
| Bug 2 — `ActiveKbState` is process-global | Task 8 |
| Bug 3 — explicit-empty unrepresentable | Task 2 |
| Bug 4 — `apps.rs` grants clobber | Task 16 |
| Bug 5 — workflows activate `visible.first()` | Task 15 |
| Bug 6 — `<digest>.tmp` treated as a live session | Task 5 (both rewriters — the hidden one has the identical hole) |
| Phase order leaves the tree green | Gates at Tasks 7, 13, 24; every task compiles the crates it touches |
| Final gates task | Task 24 |

**Two survey claims corrected against the tree.**

1. The brief anchors Bug 1 at "routes/knowledge.rs ingest path". Re-read: `POST /knowledge/bases/{id}/ingest` takes its target from the **path segment** and is not affected. The machine-wide leak is `crates/biorouter/src/agents/knowledge_tool.rs:121`. D6 records this.
2. The survey framed Bug 3 as "deselect-all silently re-inherits the machine default". The stored list is the *hidden* complement, so the unrepresentable gesture is actually **select-all** ("show everything in this chat"), plus the workflow path that declares every base visible. Same mechanism, same fix; D4 states it precisely so an implementer writes the right test.

**What this plan deliberately leaves open.**

- **Cross-base BM25 ranking.** `search_visible_bases` sorts raw scores across corpora with per-KB IDF, which is not strictly comparable. Pre-existing, unchanged, and a search-quality project with its own evaluation — not a refactor.
- **`--session <id>` for the CLI.** The CLI cannot see or set a chat's selection (D11). A real gap, a new capability.
- **Session-state GC.** `.active-kb-sessions/` and `.hidden-kb-sessions/` are never pruned when a session is deleted, so a reused session id resurrects a stale selection. Task 5 stops the rewriters tripping over debris; it does not collect it.
- **A union graph or merged change log.** No cross-KB edge model exists (`[[links]]` resolve within a base), so both panels follow the primary.
- **`session_id` is addressing, not authorization** ([#47](https://github.com/BaranziniLab/biorouter/issues/47)). Any caller holding the daemon-wide `X-Secret-Key` can read or overwrite *any* session's selection by naming its id on `/knowledge/active`, because the knowledge router holds no session state and `auth.rs` has no principal to check the id against. This is **pre-existing and daemon-wide**, not something this plan introduces: the caller-supplied `session_id` on both halves of `/knowledge/active` landed in `e1f59dbb` (2026-06-02, on `main`), and every sibling session route behaves the same way — `POST /reply` will run an agent turn, with tools, in any session id you hand it, which strictly dominates anything reachable here. What this plan does change is the *amount* of state behind the parameter: `/knowledge/active` used to carry one pointer and now carries a session's whole membership set. That is a wider blast radius through an unchanged hole, so it is recorded rather than fixed — closing it means issuing per-session capabilities and breaking the desktop client, the CLI, exported apps and the generated TypeScript client, which is a design task of its own and not a multi-KB task.

---

## Related documentation

- [Knowledge base index](README.md) — the other live working documents for this subsystem.
- [Plan 1 — storage, git and graph derivation](../history/knowledge-base-buildout/plan-1-storage-git-and-graph.md) — the `KnowledgeService` layer this plan renames and extends.
- [Plan 3 — HTTP routes and `.brkb` export/import](../history/knowledge-base-buildout/plan-3-http-routes-and-export.md) — where `/knowledge/active` came from, whose shape D10 evolves.
