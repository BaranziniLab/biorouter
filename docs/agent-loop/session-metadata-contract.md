# Session metadata contract

> **What this is.** The canonical contract for a conversation's identity: its ID, its kind,
> its parent, and what makes something a subagent run. One vocabulary, read the same way by
> the agent tools, the History UI, the sidebar, Chat Recall and the CLI.
> **Status:** Current. Established for [#111](https://github.com/BaranziniLab/biorouter/issues/111)
> and relied on by [#114](https://github.com/BaranziniLab/biorouter/issues/114); the two known
> gaps are named in [What is deliberately not in the contract](#what-is-deliberately-not-in-the-contract).
> **Audience:** developers

Four facts identify a conversation, and every surface that shows, copies, nests, filters or
looks one up reads the same four. That is the whole point of writing them down: before this,
"is that a subagent?" had one answer in the data model, another in a tool description, and a
third in what the UI could render — which is how a request for three subagents produced three
ordinary chats that History could never nest ([#111](https://github.com/BaranziniLab/biorouter/issues/111)).

## The four fields

| Field | Wire name | Type | Meaning |
|---|---|---|---|
| Conversation ID | `id` | string | The stable handle. `YYYYMMDD_N`, e.g. `20260823_2`. |
| Session kind | `session_type` | closed enum | `user` \| `scheduled` \| `sub_agent` \| `hidden` \| `terminal`. |
| Parent | `parent_session_id` | string \| null | The conversation that **delegated** this one. Non-null only for a subagent run. |
| Branch lineage | `diverged_from` | string \| null | The conversation this one was **forked from** by the user. A different axis; see below. |

They live on `Session` in `crates/biorouter/src/session/session_manager.rs` and reach the
renderer through the generated OpenAPI client — there is no second copy to keep in step.

## Conversation ID

`YYYYMMDD_N`, where `N` is a per-day counter allocated inside the `INSERT`
(`SessionStorage::create_session`). It is the primary key.

- **Stable across rename.** The name is a separate column (`name`, plus `user_set_name`);
  renaming a chat, in the UI or by the auto-namer, never touches the id.
- **Stable across reopen, close, tab moves and window moves.** Tabs and panes are a view;
  the id belongs to the session record.
- **A copy, a diverge and an import each mint a NEW id.** They are new sessions, so the
  original's id keeps pointing at the original. `copy_session` and `diverge_session` record
  the lineage in `diverged_from`; **import does not** — see the next bullet.
- **Import does not restore the exported id.** `SessionStorage::import_session` calls
  `create_session`, so an imported transcript arrives under a fresh id on the importing
  machine. It carries `session_type` across but sets neither `parent_session_id` nor
  `diverged_from`, which is deliberate: those ids named sessions in the *exporting* store and
  would dangle here. So an exported subagent run imports as a `sub_agent` with no parent, and
  an id copied before an export/import round trip does not resolve afterwards.
- **A deleted or unknown id fails loudly, not silently.** Chat Recall's load mode answers
  `Failed to load session: …`; `workspace_open { session_id }` answers `no such session: …`
  (with the caveat in [Privacy](#privacy)). Nothing falls back to a title search.
- **A deleted id is never minted again.** `create_session` takes `N` from a per-day high-water
  mark (`session_id_high_water`) that neither a delete nor a History reset lowers, so a deleted
  id keeps failing loudly instead of quietly resolving to a newer chat. It used to be
  `MAX(N) + 1` over the rows that still existed, which handed the newest deleted id of the day
  to the next chat — along with everything still keyed by it. A build without the mark that
  shares the database, or a restored backup, can still reissue an id, so `delete_session` also
  removes every row keyed by the chat's id rather than relying on this.
- `external_key` is a separate, optional lookup handle used by durable app sessions
  (`get_or_create_by_external_key`). It is not the conversation's identity and is not what
  any surface shows.

The id is the only identifier a user or an agent should ever be handed. Every ID-taking
operation accepts exactly this string with no decoration: Chat Recall's exact-ID load,
`workspace_list`, `workspace_open`, `workspace_read_conversation`, `workspace_send_prompt`,
`workspace_watch`, `workspace_set_tools`, `workspace_close`, and the `/sessions/{id}` routes.

## Getting an ID out of the app, and what to do with it

Right-click a conversation and pick **Copy conversation ID**. The menu is on all
three places a chat row is drawn — sidebar Recents, full History, and the tab strip —
and carries the same three actions in the same order everywhere: *Open in new tab*,
*Open in new window*, *Copy conversation ID*. History's `⋯` overflow shows the identical
list, which is also the keyboard path there; on any row the Menu key or Shift+F10 opens
the right-click menu, because both dispatch a `contextmenu` event on the focused element.

What lands on the clipboard is `Session.id` and nothing else — no prefix, no URL, no
name. That is the point: the string can be pasted straight into a chat, where it is
already what every ID-taking operation accepts.

**When the user pastes an exact ID and asks about that conversation, resolve it as an
ID.** Do not fuzzy-search by title — an exact handle was supplied precisely so nothing
has to be guessed at, and a title search can match the wrong chat while looking like it
worked. In order of narrowness:

| The user wants | Reach for |
|---|---|
| A compact reminder of what that chat was | Chat Recall's **exact-ID load** (`session_id`, not `query`) — head and tail of the transcript |
| Its full or structured content | `workspace_read_conversation`, narrowest view first (`summary`, then `tool_calls`, then `transcript`) |
| To open it, inject into it, watch it, or re-tool it | `workspace_open` / `send_prompt` / `watch` / `set_tools`, and only when the user asked for that operation and policy permits it |

Holding the ID changes none of the permissions — see [Privacy](#privacy).

## Session kind

`session_type` is a closed vocabulary and the **only** signal for what a conversation is. Do
not classify by title, by prompt, or by which tool happened to create it.

| Value | What it is | Created by |
|---|---|---|
| `user` | A conversation the user owns. The default. | Starting a chat in the app or CLI; `workspace_open { new: { kind: "user" } }`; `/agent/start` |
| `scheduled` | A run of a scheduled job. | The scheduler |
| `sub_agent` | A delegation — an agent's child, with a parent. | The `subagent` tool, and only it |
| `hidden` | Internal machinery, never listed to the user. | Internal callers |
| `terminal` | A terminal-backed session. | The CLI |

`user` is `SessionType::default()`, which is what makes it the safe fallback: a path that
forgets to say produces an ordinary conversation, never a spurious delegation.

## Parent, and subagent-run identity

**A subagent run is `session_type == "sub_agent"` AND `parent_session_id == <parent id>`, both
stamped before the child's first turn.** Neither half is sufficient and neither is inferred.

`create_subagent_session` (`crates/biorouter/src/agents/subagent_tool.rs`) writes both at
birth — not at the first turn, and not in `persist_spawn_context` — so a child that dies
before running anything is still a parented `sub_agent` row that History can nest and
`workspace_list { parent_session_id }` can find. `create_subagent_session_stamps_the_parent_at_birth`
is the test.

Two consequences worth stating plainly:

- **`workspace_open` cannot produce one.** Its `new.kind` is required and closed, and
  `kind: "sub_agent"` is refused with a result naming the `subagent` tool. This is the #111
  fix, and it is a declaration rather than a heuristic: a conversation the *user* owns may
  legitimately open with a first prompt, so the prompt carries no information about which of
  the two was meant.
- **Nothing reclassifies retroactively.** An existing unparented `user` session stays a
  `user` session whatever its title or first message looks like. There is no migration, and
  adding one by title or prompt heuristic would silently relabel the user's own chats.

`parent_session_id` and `diverged_from` are siblings, not synonyms: the first records a
**delegation** (an agent spawned this), the second a **user fork** (a person branched this),
paired with `branch_point_msg_uid` for the exact message the branch was taken at. A row can
carry either, and they are rendered differently — a `sub` badge versus "branched from …".

## How each surface reads the contract

| Surface | Reads |
|---|---|
| History nesting | `session_type === 'sub_agent'` **and** a `parent_session_id` resolving to a top-level row (`ui/desktop/src/components/sessions/sessionGrouping.ts`) |
| History "Show subagent runs" toggle | Whether `sub_agent` rows are fetched at all, then `withoutSubagents` at every read of the shared cache |
| The `sub` badge | The row's own `session_type`, so an unnested child is still labelled |
| Sidebar Recents, tab strip, chat glyph | `chatKind.ts`, which prefers `diverged_from` / `parent_session_id` / `session_type` over the title |
| `workspace_list` | `parent_session_id` and `only_subagents` as independent filters |
| CLI | `crates/biorouter-cli/src/commands/session_grouping.rs`, the same two fields |

Only one level of nesting is rendered, and `groupSessionsByParent` resolves the parent chain
rather than doing a single lookup, so a grandchild surfaces as top-level instead of vanishing.

## Privacy

The contract is about identity, not permission. Holding a valid id grants nothing:
`workspace_open`, `workspace_read_conversation`, `workspace_send_prompt` and Chat Recall's
load mode each ask the §7 visibility predicate first, and a public caller is refused a private
conversation. That refusal is deliberately indistinguishable from "no such session", so an
agent cannot enumerate private conversations one id at a time — which means an ID that fails
for a restricted caller is not evidence the conversation does not exist. See
[Privacy tiers](../security/privacy-tiers.md).

## What is deliberately not in the contract

- **A second identity mechanism in the UI.** Copy conversation ID copies `Session.id` and
  nothing else — no prefix, no URL, no display name. Any surface that needs to name a
  conversation to an agent uses that same string.
- **A `kind` that can mint every `SessionType`.** `workspace_open`'s `new.kind` accepts
  `user` and refuses everything else, `sub_agent` with its own message. `scheduled`, `hidden`
  and `terminal` are not that door's to create.
- **Inference from content.** No title regex, no prompt classifier, no "it looks like a task"
  rule anywhere in the identity path.

## Related documentation

- [Workspace control](workspace-control.md) — the task-oriented guide to running several
  conversations at once.
- [Workspace Control tool reference](workspace-control-tools.md) — the per-tool contract,
  including `workspace_open`'s `new.kind` and its two refusals.
- [Subagents](subagents.md) — what delegation does and what a subagent may not do.
- [Privacy tiers](../security/privacy-tiers.md) — the capability/classification lattices the
  ID-taking operations are gated by.
