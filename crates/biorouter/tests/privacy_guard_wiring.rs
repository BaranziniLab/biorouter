//! **The wiring census** (issue #56): every privacy guard in the registry below
//! has at least one live production caller, or a row saying why it has none.
//!
//! # Why this file exists
//!
//! Five times in this campaign the same defect shipped: *a guard written,
//! correct, unit-tested — and never called.* The worst instance was
//! [`privacy::visibility::may_read`] and [`privacy::visibility::appears_in_list`]
//! having **zero** callers while `workspace_read_conversation` checked
//! `session_type == Hidden` and nothing else, so a public chat read private
//! transcripts, listed private chats, and injected prompts into them. Every unit
//! test in the tree passed the whole time, because the unit under test was
//! correct. Code review passes it too, for the same reason: the function you are
//! reading is right.
//!
//! An audit finds today's instances. This finds every future one.
//!
//! # What it asserts, exactly
//!
//! For each [`Guard`] in [`REGISTRY`], the census walks `crates/**/src/**.rs`,
//! removes comments, string bodies and `#[cfg(test)]` items, and measures the
//! **exact** set of places the guard's identifier occurs, split by kind (a call,
//! a bare reference, an import). That measured map must equal the declared
//! [`Site`] rows. Then, by [`Status`]:
//!
//! * [`Status::Wired`] — at least one real call or reference **outside the
//!   defining file**. This is the assertion that fails when a guard is
//!   disconnected.
//! * [`Status::WiredThrough`] — no outside caller, but reached from a named
//!   guard inside its own file, which must itself be wired. The chain is
//!   checked, so a "helper of a wired guard" cannot quietly become a helper of a
//!   dead one.
//! * [`Status::Unwired`] — no live caller at all, deliberately or as an admitted
//!   gap, with the reason written down. The audit asserts it really has none: if
//!   someone wires it, this test fails and the row has to move. A gap that gets
//!   closed should not stay filed as a gap.
//!
//! # Three things it does NOT do, each learned from a gate that passed by accident
//!
//! ⚠ **It does not grep for a name.** This campaign shipped a grep gate that
//! passed on a mention in a comment. Comments and string literals are removed by
//! a small lexer before anything is counted, and every site is classified as a
//! call, a reference or an import — an `use` line is not wiring.
//!
//! ⚠ **It does not trust a name match to be the guard.** `may_read` is also a
//! method on the memory server's `Audience`, and `privacy_refusal` is also a
//! local variable in the agent loop. Those are declared as
//! [`SiteKind::Unrelated`] rows and excluded from every wiring decision — so a
//! guard cannot look wired because something else shares its name. That is the
//! difference between counting call sites and counting matches.
//!
//! ⚠ **It does not assume the walk worked.** A broken walk reports the same
//! empty set as a clean tree. The controls below assert the file count, assert
//! that spellings only tests use are absent from what the census calls
//! production (so the split is real in one direction), and assert that every
//! guard the census calls unwired is nonetheless mentioned in the tree's *test*
//! text (so the split is real in the other, and "no production caller" is not
//! "no file was read").
//!
//! # The residual, stated rather than left to be discovered
//!
//! The completeness half ([`REGISTRY_COVERS_EVERY_GUARD_MODULE`]) discovers new
//! `pub fn`s only in [`COMPLETE_MODULES`] — the three files whose every public
//! function is a reach decision. A new predicate added to `privacy/refusal.rs`,
//! `privacy/mixing.rs` or `knowledge/tier.rs` is **not** discovered; those files
//! contribute curated rows only. Widening completeness to them means classifying
//! their accessors too, and a census padded with `as_sql` and `is_private` rows
//! is one people stop reading. If a new reach predicate lands outside the three,
//! add its row by hand.
//!
//! [`privacy::visibility::may_read`]: biorouter::privacy::visibility::may_read
//! [`privacy::visibility::appears_in_list`]: biorouter::privacy::visibility::appears_in_list
//! [`REGISTRY_COVERS_EVERY_GUARD_MODULE`]: the_registry_covers_every_guard_module

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// The registry.
// ---------------------------------------------------------------------------

/// How a site mentions the guard, as the scanner classifies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Counts {
    /// `ident(` — a call.
    calls: usize,
    /// The bare identifier in a code position: a function passed as a value
    /// (axum middleware), a module path that happens to share the name, a local
    /// binding that happens to share the name.
    refs: usize,
    /// Inside a `use` statement. An import is not wiring: a guard can be
    /// imported, re-exported and never called.
    imports: usize,
}

const fn c(calls: usize, refs: usize, imports: usize) -> Counts {
    Counts {
        calls,
        refs,
        imports,
    }
}

/// Whether a site is the guard at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SiteKind {
    /// This site really is the guard: a call, or a deliberate reference to it.
    Guard,
    /// Something else with the same name. Recorded so the map stays exact, and
    /// excluded from every wiring decision.
    Unrelated,
}

/// One file that mentions a guard, and what those mentions are.
struct Site {
    file: &'static str,
    counts: Counts,
    kind: SiteKind,
    /// What is at this site. Not decoration: a census whose rows are bare paths
    /// is one a future engineer extends by pasting a line to make the build
    /// green.
    what: &'static str,
}

enum Status {
    /// Has live production callers outside its defining file.
    Wired,
    /// Reached only from inside its own file, by the named guard — which must
    /// itself be wired, and is checked to be.
    WiredThrough(&'static str),
    /// Has no live production caller. The string says why, and who owns the
    /// decision to leave it that way.
    Unwired(&'static str),
}

struct Guard {
    ident: &'static str,
    defined_in: &'static str,
    /// What this predicate decides. A guard nobody can describe is a guard
    /// nobody can tell is missing.
    decides: &'static str,
    status: Status,
    sites: &'static [Site],
}

/// Files whose every production `pub fn` must appear in [`REGISTRY`]. See the
/// module doc's residual for why the list is three files and not the whole of
/// `privacy/`.
const COMPLETE_MODULES: &[&str] = &[
    "crates/biorouter/src/privacy/visibility.rs",
    "crates/biorouter-server/src/routes/session_reach.rs",
    "crates/biorouter/src/privacy/extensions.rs",
];

const VISIBILITY: &str = "crates/biorouter/src/privacy/visibility.rs";
const SESSION_REACH: &str = "crates/biorouter-server/src/routes/session_reach.rs";
const EXTENSIONS: &str = "crates/biorouter/src/privacy/extensions.rs";

/// Every privacy guard under census, as measured against this tree.
const REGISTRY: &[Guard] = &[
    // ------------------------------------------------------------ §7, the
    // capability matrix. One predicate per verb.
    Guard {
        ident: "may_read",
        defined_in: VISIBILITY,
        decides: "READ ⇔ VIS: whether a caller of tier C may read a session classified T",
        // It was `WiredThrough("refuse_unless_readable")` until `biorouter web` became
        // its first caller outside this file; the in-file row below still holds.
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-cli/src/commands/session_decisions.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`check_local_reach`, the gate on `biorouter session decisions` and \
                       `biorouter session monitors` when they read the LOCAL store (no daemon \
                       answered): a chat's decision log holds what each call was sent, its own \
                       content, so the chat must be readable at this terminal's capability, as \
                       `routes::decisions` requires over HTTP. Plus its import",
            },
            Site {
                file: "crates/biorouter-cli/src/commands/session_subagents.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`biorouter session subagents` read from the LOCAL store (no daemon \
                       answered): the parent chat must be readable at this terminal's \
                       capability, as `GET /sessions/{id}/subagents` requires over HTTP, so \
                       unsetting the daemon secret is not a way around the route's gate. Plus \
                       its import",
            },
            Site {
                file: "crates/biorouter-cli/src/commands/web.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`refuse_turn_unless_reachable`, the gate on `biorouter web`'s WebSocket: \
                       a message there runs a turn in whichever chat it names, so the page must \
                       be able to read that chat. Plus its import",
            },
            Site {
                file: "crates/biorouter-mcp/src/memory/mod.rs",
                counts: c(2, 0, 0),
                kind: SiteKind::Unrelated,
                what: "`Audience::may_read` on the memory server, an unrelated method that \
                       happens to share the name. Left in the map on purpose: it is the \
                       standing proof that a name match is not a call site, because these \
                       two hits alone would satisfy a census that only counted matches",
            },
            Site {
                file: VISIBILITY,
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`refuse_unless_readable`'s two asks: the short-circuit for a caller \
                       that may read Private at all, and the decision on the resolved row",
            },
        ],
    },
    Guard {
        ident: "appears_in_list",
        defined_in: VISIBILITY,
        decides: "whether a session may appear in a listing at all: omission, not redaction, \
                  because an LLM-generated title is content",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-cli/src/commands/session_subagents.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`biorouter session subagents` on the local store: each child is listed \
                       at this terminal's capability before the phase plan is rebuilt from the \
                       children that remain, the CLI counterpart of the route's `lists_session` \
                       filter. Plus its import",
            },
            Site {
                file: "crates/biorouter-server/src/routes/crew_tools/mod.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`Ctx::lists_chat`, the one filter every Crew person-tool action that \
                       names another chat asks before it does (account ownership AO-12): \
                       `crew__read grants_list` and `crew__change grants_revoke` name chats \
                       holding a grant, and `crew__read tasks_list` and `tasks_watch` show a \
                       task whose conversation is a chat of its own. A model's call, not an HTTP \
                       caller, so it is `workspace_list`'s rule on the call's one sampled \
                       capability rather than `lists_session`, which needs the person's proof. \
                       Plus its import",
            },
            Site {
                file: "crates/biorouter/src/agents/schedule_tool.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`platform__manage_schedule`'s `sessions` action, which lists a job's \
                       execution sessions by name and working directory",
            },
            Site {
                file: "crates/biorouter/src/agents/recurring.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`/schedule sessions <id>`, the same listing typed as a slash command: a \
                       run of a schedule whose work is private is a private chat, so a chat \
                       that is not on a private model is shown only the runs it could list — \
                       filtered, as `GET /schedule/{id}/sessions` filters, on the command's \
                       one sampled capability",
            },
            Site {
                file: "crates/biorouter/src/agents/workspace_extension.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`workspace_list`, where the filter sits BEFORE the match counter so \
                       the paging metadata cannot become a count oracle",
            },
        ],
    },
    Guard {
        ident: "refuse_unless_readable",
        defined_in: VISIBILITY,
        decides: "READ applied to a session the caller merely NAMED: resolve the target's \
                  classification metadata-only, then ask `may_read`",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter/src/agents/schedule_tool.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`platform__manage_schedule`'s `session_content` action, which returns \
                       a named session's entire transcript",
            },
            Site {
                file: "crates/biorouter/src/agents/workspace_extension.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`WorkspaceClient::refuse_unless_visible`, shared by the read handlers \
                       and the writable-lineage adapter",
            },
        ],
    },
    Guard {
        ident: "may_write",
        defined_in: VISIBILITY,
        decides: "WRITE ⇔ VIS: whether a caller may steer a session, which is now every \
                  session it may read. The lineage clause it used to carry — steer what you \
                  spawned, read everything else — is retired: an agent may inject into any \
                  conversation, and the tier is the only boundary",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-cli/src/commands/web.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`refuse_turn_unless_reachable`, the write half: a `biorouter web` message \
                       is written into the chat it names. Plus its import",
            },
            Site {
                file: "crates/biorouter/src/agents/workspace_extension.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "the shared writable adapter used by send_prompt, set_tools and close",
            },
        ],
    },
    Guard {
        ident: "requires_first_crossing_approval",
        defined_in: VISIBILITY,
        decides: "whether a write is a downgrade crossing (private caller → public target) and \
                  so must disclose its payload the first time",
        // ⚠ This row read `Status::Unwired("OPERATOR DECISION OUTSTANDING")` for a long
        // time, and what settled the decision was widening the write rule. While WRITE
        // carried its lineage clause, the only public targets a private caller could write
        // into were ones it had spawned itself; now it can write into any public
        // conversation on the machine, so the moment private-origin text leaves for a
        // public model is one the user has to be able to see. The state the predicate
        // always needed is `privacy/crossing.rs`.
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter/src/privacy/crossing.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`needs_disclosure`, which crosses the pure predicate with the \
                       (caller, target) ledger the disclosure is keyed on. The inspector \
                       that raises the approval — `WorkspaceCrossingInspector` — asks \
                       through here rather than asking the predicate itself, so there is \
                       one place that knows what 'first' means",
            },
            Site {
                file: "crates/biorouter/src/agents/workspace_extension.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`record_crossing_if_disclosed`, the RECORD half, called by both \
                       `handle_send_prompt` and `handle_set_tools`. ONE call site for two \
                       handlers, deliberately: the pair is marked as crossed only once the \
                       write has landed AND only when there was something to disclose, and \
                       a second copy of that pair of conditions is how one of them goes \
                       missing",
            },
            Site {
                file: "crates/biorouter/src/agents/workspace_inspector.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "the one crossing whose target does not exist yet. \
                       `workspace_open { new: { prompt } }` mints its conversation and \
                       writes into it in a single call, so there is no row for \
                       `needs_disclosure` to consult and no ledger entry that could make \
                       it a SECOND crossing — a call that creates its own target is a \
                       first crossing outright. It therefore asks the pure predicate \
                       directly, against `Public`, which is the classification a new \
                       conversation is born at. \
                       ⚠ This site was ABSENT while the tool was not, which is exactly \
                       the hole the census exists to make visible from the other \
                       direction: a private chat could write caller-chosen text into a \
                       new public conversation with no card and no ledger entry, while \
                       `workspace_send_prompt` with the same text into the same session \
                       raised one",
            },
        ],
    },
    // ------------------------------------------------------- the HTTP reach
    // gate. `session_id` is a request parameter, not a credential.
    Guard {
        ident: "session_reach",
        defined_in: SESSION_REACH,
        decides: "whether an HTTP caller naming a session may reach it: private and unreadable \
                  targets require the user-action proof",
        status: Status::Wired,
        sites: &[
            // ⚠ The `refs` on these four rows are the MODULE qualifier: the gate
            // is called as `session_reach::session_reach(…)`, which is one
            // reference to the module and one call to the function on the same
            // line. Counted honestly rather than folded into the call, so that a
            // handler which imported the module and never called the gate would
            // read as refs-only and stand out.
            Site {
                file: "crates/biorouter-server/src/routes/apps.rs",
                counts: c(0, 3, 0),
                kind: SiteKind::Unrelated,
                what: "the MODULE qualifier on `session_reach::HttpCaller` (twice) and \
                       `http_caller`, for `POST /apps/{id}/export`: a full export asks \
                       `reach_knowledge_base` of each base it would carry, and names no chat, so \
                       it never calls this function",
            },
            Site {
                file: "crates/biorouter-server/src/routes/active_work.rs",
                counts: c(0, 3, 0),
                kind: SiteKind::Unrelated,
                what: "the MODULE qualifier on `session_reach::HttpCaller`, `http_caller` and \
                       `work_reach`. `GET /active_work` filters through `lists_work` and its \
                       cancel asks `work_reach`, which calls this function for the work's chat; \
                       neither route calls it directly, which is what refs-only records here",
            },
            Site {
                file: "crates/biorouter-server/src/routes/action_required.rs",
                counts: c(2, 2, 0),
                kind: SiteKind::Guard,
                what: "the typed folder-decision branch of `POST /action-required/tool-confirmation`, \
                       before resolving the named chat's pending request, and `POST \
                       /action-required/crew-step` (`dismiss_crew_step`), before the registry is \
                       read, so a 404 for a private chat's Crew step card cannot tell a caller \
                       out of reach that the chat exists. The count went 1 -> 2 because the \
                       dismiss route is new, not because a guarded route grew a second gate. \
                       Each call also names the module once, which is the second ref",
            },
            Site {
                file: "crates/biorouter-server/src/routes/folder_access.rs",
                counts: c(2, 2, 0),
                kind: SiteKind::Guard,
                what: "`GET /action-required/folder-access` before listing the named chat's \
                       grants, and `POST /action-required/folder-access/revoke` before changing \
                       one. Each call also names the module once. Native lifetime scoping adds \
                       owner isolation after these existing reach decisions; it does not replace them",
            },
            Site {
                file: "crates/biorouter-server/src/routes/agent.rs",
                counts: c(7, 7, 0),
                kind: SiteKind::Guard,
                what: "`POST /agent/resume`, `POST /agent/update_from_session`, and `POST \
                       /agent/update_working_dir`, plus the shared `authorize_agent_control` \
                       gate used by provider, extension, stop, and restart mutations — and, \
                       since QA's 2026-09-10 M2, `GET /agent/tools` (a private chat's \
                       private-extension tool names, handed to a secret-only caller while \
                       `add_extension` on the same chat refused) and `GET \
                       /agent/callable_tool_count`, both of which mint an agent for the chat \
                       they name. \
                       ⚠ The count went 4 → 7. The first two additions were routes that had no \
                       gate at all rather than second gates on guarded ones: `/agent/tools` \
                       answered any caller that could name a chat, and the SD-8 review of #260 \
                       found `callable_tool_count` doing the same through `get_or_create_agent`, \
                       which CREATES an agent for a session that has none — so each fix is the \
                       same reach gate, placed before the fetch for the reason \
                       `agent_add_extension` states at its own. \
                       `POST /agent/call_tool` also asks this gate before authorizing a \
                       Crew-scoped tool call, so a private session cannot be used to discover \
                       or invoke its bound tool. \
                       On a daemon that holds no user-action key `authorize_agent_control` is \
                       also the turn-control gate of `/agent/cancel` and the two continuation \
                       routes (SD-11), reached from `routes::reply`'s `authorize_turn_control` \
                       by name — so SD-11 added no call here. `/interrupt` keeps the proof on \
                       every daemon and reaches neither gate",
            },
            Site {
                file: "crates/biorouter-server/src/routes/crew.rs",
                counts: c(1, 1, 0),
                kind: SiteKind::Guard,
                what: "`POST /crew/connections/{id}/sessions/{session_id}/grant` (`grant_session`), \
                       which requires human proof and checks session reach before attaching \
                       conversation authority to the Crew connection",
            },
            Site {
                file: "crates/biorouter-server/src/routes/dictation.rs",
                counts: c(1, 3, 1),
                kind: SiteKind::Guard,
                what: "`POST /dictation/streams` into a chat (PR-15, DD-29, DD-30): the gate \
                       before the chat's tier and live model are read, a refusal degrading to \
                       public terms with a byte identical answer. The three refs are the \
                       MODULE qualifier on this call, `target_tier` and `http_caller`, and the \
                       import is the `use` that brings the module in",
            },
            Site {
                file: "crates/biorouter-server/src/routes/dictionary.rs",
                counts: c(1, 17, 1),
                kind: SiteKind::Guard,
                what: "`read_chat` (PR-16, O-35, O-36): the gate before a stamp reads the chat \
                       an add started in (`origin_session_id`) or an observation names, a \
                       refusal giving the unknown institutions for an add and a silent no op \
                       for an observation. The seventeen refs are the MODULE qualifier: on this \
                       call, on `target_tier`, on `gate_dictionary_entry` in the router, and on \
                       `http_caller` in each of the fourteen places that resolve their caller \
                       (PR-23 added the agent row Undo and the activation choice; \
                       `admit_dictionary_attach`, DD-56, asks for `POST /agent/add_extension`); \
                       the import is the `use` that brings the module in",
            },
            Site {
                file: "crates/biorouter-server/src/routes/knowledge.rs",
                counts: c(1, 7, 0),
                kind: SiteKind::Guard,
                what: "`POST /knowledge/bases/{id}/ingest-conversation`, one call inside the \
                       loop over the chats the request names, before any transcript is read: \
                       the route streams what a model makes of those chats back to its caller, \
                       and a caller holding only the daemon secret could name a private model \
                       (QA 2026-09-10 H2). The other seven refs are the MODULE qualifier on \
                       `session_reach::gate_knowledge_base`, `http_caller` (FOUR handlers \
                       since the adversarial review of 2026-09-12 gated `POST \
                       /knowledge/bases`) and `HttpCaller` — names that live beside the gate, \
                       not the gate. `calls` is unmoved: the create gate asks \
                       `mints_knowledge_base`, its own row below, not this function",
            },
            Site {
                file: "crates/biorouter-server/src/routes/mod.rs",
                counts: c(0, 2, 0),
                kind: SiteKind::Unrelated,
                what: "`pub mod session_reach;` and the `session_reach::gate_knowledge_active` \
                       path in the knowledge layer: the MODULE's name, not the function's",
            },
            Site {
                file: "crates/biorouter-server/src/routes/reply.rs",
                counts: c(3, 3, 0),
                kind: SiteKind::Guard,
                what: "`POST /reply`, which runs an agent turn with tools inside the named \
                       session, plus the explicit continuation takeover and group-abandon \
                       recovery mutation. `/agent/cancel` and `/agent/continuation/abandon` reach \
                       the gate for the chat they name only on a keyless daemon, and through \
                       `authorize_agent_control` in `routes/agent.rs` (SD-11); `/interrupt` keeps \
                       the user-action proof on every daemon and reaches no reach gate at all. \
                       The third call is `reachable_subagents`, asked of each subagent a plain \
                       Stop on `/agent/cancel` would also stop, on its OWN chat: the gate admitted \
                       the parent, and a child a mid-run raise made more private than its parent \
                       was stopped and named in `stopped_subagents` for a caller that could not \
                       open it",
            },
            Site {
                file: "crates/biorouter-server/src/routes/schedule.rs",
                counts: c(0, 12, 0),
                kind: SiteKind::Unrelated,
                what: "the MODULE qualifier twelve times, on no occasion this function. Once on \
                       `session_reach::http_caller`, which filters `GET /schedule/{id}/sessions` \
                       — a listing, gated by `lists_session`. Once on \
                       `session_reach::work_reach` for `POST /schedule/{id}/kill`: the stop \
                       resolves the run to its chat and asks THAT function, exactly as `POST \
                       /active_work/{id}/cancel` does for the same kill, so neither route is \
                       the easier way to stop a private chat's run. Once more on the same \
                       function for `GET /schedule/{id}/inspect`, which hands back the chat a \
                       run is in. Two are `GET /schedule/list`'s redaction: the qualifier on \
                       `http_caller`, and on the `HttpCaller` TYPE in \
                       `redact_unreachable_chats`'s signature — a type, not a decision. SIX, \
                       from 2026-09-14, are `session_reach::schedule_reach` on the six writes — \
                       create, run now, pause, resume, re-time and delete — its own row below; \
                       and the twelfth is `http_caller` again, for `PUT /schedule/{id}`, which \
                       answers with the job and redacts it exactly as the listing does",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session.rs",
                counts: c(8, 13, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/{id}` (the transcript) and `GET /sessions/{id}/export` \
                       (the same transcript, `to_string_pretty`), and — QA 2026-09-10 F0 and \
                       the sweep it asked for — every other route that names a chat: `DELETE \
                       /sessions/{id}` (measured deleting a private chat the read refused, four \
                       of four), `PUT …/name`, `PUT …/user_workflow_values`, the in-place arm \
                       of `POST …/edit_message` (it truncates), `GET …/extensions` and `GET \
                       …/usage`. THIRTEEN refs and still EIGHT calls: the module qualifier on \
                       each of the eight calls, and on `http_caller` for the THREE listings — \
                       `GET /sessions`, `GET /sessions/sidebar`, and, since the 2026-09-12 serve \
                       sweep, `GET /sessions/running`. The two new refs are `http_caller` on \
                       `GET /sessions/insights` and `GET /sessions/activity`, which omit Crew-owned \
                       rows as metadata listings rather than refusing a named chat. ⚠ The refs \
                       are LISTINGS and not additional reach decisions, which is why `calls` did \
                       not move: a listing asks `HttpCaller` whether to SHOW a row and drops it \
                       silently, where a call here refuses the request outright",
            },
            Site {
                file: "crates/biorouter-server/src/routes/skills.rs",
                counts: c(1, 1, 0),
                kind: SiteKind::Guard,
                what: "`POST /skills/session`, which writes a skill's instructions into the \
                       named chat's next turn (QA 2026-09-10, F0's sweep)",
            },
            Site {
                file: "crates/biorouter-server/src/routes/decisions.rs",
                counts: c(1, 1, 0),
                kind: SiteKind::Guard,
                what: "`reach_chat`, the ONE helper every decision route that names a chat asks \
                       before it reads or parses anything about that chat: `GET \
                       /sessions/{id}/decisions` (a page of the decision log, which holds what \
                       was sent, i.e. the chat's own content), `GET …/decisions/summary`, `GET \
                       …/decisions/{decision_id}` (one call with its full context), `GET|PUT \
                       …/decision-settings` and `GET|PUT …/decision-chat` (the PUTs then also \
                       take the user-action proof on a keyed daemon), plus `GET \
                       /decisions/models?sessionId=` and `POST /decisions/decide` with a \
                       `sessionId` (whose decision the service then checks against that chat's \
                       tier, Gate H). One call for nine routes, so the count does not grow with \
                       the routes: a decision route that skipped the helper would not show up \
                       here, and `decisions_tests.rs` drives every one of them instead. The ref \
                       is the module qualifier",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session_subagents.rs",
                counts: c(1, 2, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/{id}/subagents`, which names the PARENT chat and answers \
                       with its children's names, models and result lines, so the parent is \
                       gated before its row is read. The second ref is the module qualifier on \
                       `http_caller`, which then filters each child as a listing does: a child \
                       can be more private than its parent",
            },
            Site {
                file: "crates/biorouter-server/src/routes/workflow.rs",
                counts: c(1, 3, 0),
                kind: SiteKind::Guard,
                what: "`POST /workflows/create`, which loads the named chat's whole transcript \
                       and returns a workflow a model wrote from it (QA 2026-09-10, F0's \
                       sweep). The second ref is the module qualifier on `http_caller`, which \
                       filters the knowledge bases the workflow names; the third is the \
                       qualifier on `schedule_reach` for `POST /workflows/schedule` — a \
                       different function, with its own row below",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session_meta.rs",
                counts: c(0, 1, 0),
                kind: SiteKind::Unrelated,
                what: "the MODULE qualifier on `session_reach::http_caller` for `GET \
                       /sessions/changes`, which filters the row changes it reports through \
                       `lists_session` rather than refusing: a feed, like a listing, not a route \
                       that names one chat",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session_events.rs",
                counts: c(2, 2, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/{id}/events`, which gates before opening the \
                       full-conversation snapshot and rechecks reach before each live event \
                       frame, so a revocation closes both sides of the stream",
            },
            Site {
                file: "crates/biorouter-server/src/routes/status.rs",
                counts: c(1, 3, 0),
                kind: SiteKind::Guard,
                what: "`GET /diagnostics/{id}`, whose zip carries `session.json` straight from \
                       `export_session` (the third spelling of the same transcript) plus \
                       this session's log files, which carry its prompts. It was the one \
                       session-addressing route in the tree with ZERO `session_reach` calls \
                       after the first sweep, and this census could not see that: a file with \
                       no mention of a guard is indistinguishable from a file that needs none. \
                       The 3 refs are the module qualifier three times over. One is the \
                       ordinary one every row above carries: the `session_reach::session_reach(…)` \
                       call names the module and the function on the same line. The other TWO \
                       are in `refusal_body`, the handler-local helper that specialises the \
                       refusal on the keyless arm — it takes a `session_reach::SessionOutOfReach` \
                       and compares against `session_reach::SESSION_REACH_NO_KEY` to tell that \
                       arm from the out-of-reach one, so that a headless daemon is told to run \
                       `biorouter session diagnostics` instead of to use a desktop app it does \
                       not have. Both name a TYPE and a CONSTANT that live beside the gate; \
                       NEITHER is a second decision. That is the distinction this row exists to \
                       record: `calls` stayed at 1 because the route still asks the gate exactly \
                       once and still refuses on exactly its answer, and only the *wording* of \
                       the refusal became route-specific. If `calls` ever moves off 1 here, that \
                       is a real change of the reach decision and not a rewording",
            },
            Site {
                file: "crates/biorouter-server/src/routes/usage_limit.rs",
                counts: c(1, 1, 0),
                kind: SiteKind::Guard,
                what: "`reach_chat`, the one helper both `GET /sessions/{id}/usage-limit` and `PUT \
                       /sessions/{id}/usage-limit` call before reading or writing the named \
                       chat's limit. The ref is the module qualifier on the same line as the \
                       call. The PUT then also needs the user-action proof on a keyed daemon, so \
                       a model cannot lift its own limit; that proof is not this gate",
            },
            Site {
                file: SESSION_REACH,
                counts: c(3, 0, 0),
                kind: SiteKind::Guard,
                what: "`gate_knowledge_active`, whose GET query and POST body branches each \
                       invoke the same reach gate; and `work_reach`, which asks it for the chat a \
                       piece of running work belongs to, so stopping a chat's work is gated by \
                       the very call that gates reading it",
            },
        ],
    },
    Guard {
        ident: "excluded_crew_sessions",
        defined_in: SESSION_REACH,
        decides: "which Crew-owned sessions an HTTP caller must have omitted from listings and metadata feeds",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/schedule.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /schedule/{id}/sessions`, filtering Crew-owned sessions before the response is built",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session.rs",
                counts: c(3, 0, 0),
                kind: SiteKind::Guard,
                what: "`keyset_listing_scope`, which `GET /sessions/sidebar` and the paged `GET /sessions?limit=` (History's first paint) both ask, plus `GET /sessions/insights` and `GET /sessions/activity`, each omitting Crew-owned rows before serialization",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session_subagents.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/{id}/subagents`, omitting Crew-owned children from the roster before the plan's counts are derived from it",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session_meta.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/changes`, excluding Crew-owned sessions from the long-poll metadata feed",
            },
        ],
    },
    Guard {
        ident: "gate_knowledge_active",
        defined_in: SESSION_REACH,
        decides: "the same reach gate for `GET|POST /knowledge/active`, as a layer rather than a \
                  line in the handler",
        status: Status::Wired,
        sites: &[Site {
            file: "crates/biorouter-server/src/routes/mod.rs",
            counts: c(0, 1, 0),
            kind: SiteKind::Guard,
            what: "`from_fn_with_state(state, session_reach::gate_knowledge_active)` on the \
                   nested knowledge router. A REFERENCE, not a call, which is why this census \
                   counts references as wiring: a middleware never appears with parentheses, \
                   and a census that demanded a call would have reported this live gate as dead",
        }],
    },
    Guard {
        ident: "refuse_unless_reachable",
        defined_in: SESSION_REACH,
        decides: "the pure decision under the gate: enforced × target tier × proof → reach or \
                  refusal",
        status: Status::WiredThrough("session_reach"),
        sites: &[Site {
            file: SESSION_REACH,
            counts: c(7, 0, 0),
            kind: SiteKind::Guard,
            what: "`session_reach` itself, which is this predicate plus the two lookups that \
                   feed it; and since QA's 2026-09-10 sweep `HttpCaller::admits` (a listing is \
                   the rows this decision admits, one at a time — the one spelling that both \
                   `lists_session` and, for running work, `lists_work` are) and \
                   `HttpCaller::reach_knowledge_base` (the same decision with a knowledge base \
                   as the target); `work_reach`'s arm for running work that names no chat, \
                   whose target is `Unreadable` because there is no chat to resolve; and, from \
                   the adversarial review of 2026-09-12, `HttpCaller::mints_knowledge_base` — \
                   whether a caller may TAKE a base id, asked at `TargetTier::Unreadable` and \
                   WITHOUT the id, because create refuses one that is taken and an answer that \
                   varied with the id would say which private bases exist. And, from 2026-09-14, \
                   `schedule_reach` TWICE: once at `Unreadable` to answer an admitted caller \
                   without reading the store or the provider registry (the `lists_work` \
                   observation), and once on the schedule's resolved work. ONE decision, six \
                   subjects: a second spelling of it is what this census exists to stop. \
                   ⚠ This row read `four` on both sides of the #237 merge — each branch added \
                   a subject and neither could see the other's — which is exactly the arithmetic \
                   this census is here to refuse to take on trust",
        }],
    },
    Guard {
        ident: "http_caller",
        defined_in: SESSION_REACH,
        decides: "who is asking, resolved ONCE per request: the stated capability, the \
                  user-action proof, DR-15's switch, and — on a serve daemon only — the \
                  operator's tier for a request carrying the served document's cookie",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/apps.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /apps/{id}/export`, whose full export carries knowledge bases out \
                       of the machine — resolved once for the whole export, then asked of each \
                       base",
            },
            Site {
                file: "crates/biorouter-server/src/routes/active_work.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /active_work`, every running shell command and subagent prompt on \
                       the machine, each row shown by the chat it belongs to — resolved once for \
                       the whole list",
            },
            Site {
                file: "crates/biorouter-server/src/routes/knowledge.rs",
                counts: c(4, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /knowledge/bases` (a private base omitted from a caller that \
                       cannot open it), both halves of `/knowledge/active` (the selection \
                       filtered, and a write unable to move what its caller cannot see), and \
                       `POST /knowledge/bases` — which refused a taken id and so answered \
                       whether a PRIVATE base held it (adversarial review 2026-09-12)",
            },
            Site {
                file: "crates/biorouter-server/src/routes/dictation.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /dictation/streams` (PR-15, DD-30), resolved once per stream for \
                       `dictation_standing`, which reads only its proof and stated capability",
            },
            Site {
                file: "crates/biorouter-server/src/routes/dictionary.rs",
                counts: c(14, 0, 0),
                kind: SiteKind::Guard,
                what: "every `/dictionary/*` handler (PR-16, DD-37, DD-38), each resolving its \
                       caller once for the listing decision, the write rules, the stamp and \
                       private standing: the listing, add, edit, delete, tier change, the bulk \
                       routes' shared reader, import, export, observations, both halves of the \
                       settings, and since PR-23 the agent row Undo \
                       (`POST /dictionary/entries/{id}/undo-agent-add`) and the activation \
                       choice (`POST /dictionary/activation`, DD-56); plus \
                       `admit_dictionary_attach`, which `POST /agent/add_extension` asks before \
                       it attaches the Dictionary to a chat, by the activation choice's own rule \
                       (DD-56, owner decision 2026-11-13)",
            },
            Site {
                file: "crates/biorouter-server/src/routes/schedule.rs",
                counts: c(3, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /schedule/{id}/sessions`, a schedule's runs by name and directory; \
                       `GET /schedule/list`, which resolves the caller ONCE for the whole \
                       listing and then redacts each row's chat-naming fields; and `PUT \
                       /schedule/{id}`, which answered with the whole job and so handed a \
                       secret-only caller the `creator_session_id` the listing redacts \
                       (measured on 1038a113) — now redacted the same way",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session_meta.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/changes`, resolved ONCE for the life of the long poll: \
                       every change carries a chat's provider, model and privacy tier, and a \
                       secret-only caller naming a private chat was told all three on every \
                       switch (measured on 1038a113)",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session.rs",
                counts: c(5, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions` and `GET /sessions/sidebar` — QA 2026-09-10 M1, every \
                       chat on the machine, titled, to a secret-only caller — and, since the \
                       2026-09-12 serve sweep, `GET /sessions/running`, `GET /sessions/insights` \
                       and `GET /sessions/activity`, which each resolve one caller before \
                       filtering the rows they expose",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session_subagents.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/{id}/subagents`, resolved ONCE for the whole roster after \
                       the parent's own gate, then asked of every child row",
            },
            Site {
                file: SESSION_REACH,
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`gate_knowledge_base`, the layer on every `/knowledge/bases/{id}` route, \
                       and `gate_dictionary_entry`, the layer on every `/dictionary/entries/{id}` \
                       route (PR-16)",
            },
            Site {
                file: "crates/biorouter-server/src/routes/workflow.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /workflows/create`, whose workflow names the chat's knowledge \
                       bases — filtered to the ones its caller may open",
            },
        ],
    },
    Guard {
        ident: "lists_session",
        defined_in: SESSION_REACH,
        decides: "whether a listing may show a caller a chat of a given classification: \
                  exactly the singular gate's answer for that row, so omission and never \
                  redaction",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/schedule.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /schedule/{id}/sessions`, filtered BEFORE its limit",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session_meta.rs",
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/changes`: ONE predicate (the `may_show` closure) asked of \
                       the rows the poll observes, of the tier each change was recorded with, \
                       and — through `visible_changes` — of that chat's row as it is NOW. \
                       Observed only for chats the caller could open, so naming a private chat \
                       cannot make the poll publish — and so answer, or move the revision — when \
                       that chat's row moves; answered only with changes to chats the caller \
                       could still open, since the ring is process-wide, holds rows other \
                       windows watch, and keeps a change stamped with the tier its chat had \
                       then (a chat that went private kept leaking its earlier public binding, \
                       independent QA 2026-09-14). The second call asks it once at `Private`, \
                       so a caller that could open every chat is answered without reading a row \
                       back",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session.rs",
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions`, filtered row by row, and `keyset_listing_scope` ONCE \
                       — asked by `GET /sessions/sidebar` and by the paged `GET /sessions?limit=` \
                       alike (V1-R3-2), so the two cannot admit different chats. It asks whether \
                       this caller is shown private chats and hands the \
                       answer to SQL as a predicate. It used to ask per row of a scan over the \
                       unfiltered ordering and resume from the position it reached, which made \
                       the continuation value a count of the rows it hid (adversarial review \
                       2026-09-12)",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session_subagents.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/{id}/subagents`, asked of each child's OWN classification \
                       (a child can be more private than its parent), BEFORE the plan is \
                       recomputed from the rows that survive, so a phase neither counts nor \
                       names a child the caller was not listed",
            },
        ],
    },
    Guard {
        ident: "lists_work",
        defined_in: SESSION_REACH,
        decides: "whether a listing of RUNNING WORK may show a caller a row belonging to a given \
                  chat — or to none. The chat is resolved metadata-only, and a row naming no \
                  chat, or one that cannot be read, is answered as a private chat's row: its \
                  command came from some chat and nothing says whose",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/active_work.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`visible_items`, which `GET /active_work` passes every row through — \
                   background jobs, foreground commands, subagents, detached turns and scheduled \
                   runs alike — after one `http_caller` for the whole list",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/running`, whose every row IS a chat id — so the id is \
                       shown exactly when `GET /active_work` would show that chat's work. It \
                       holds ids and not rows, which is why it takes this predicate rather than \
                       `lists_session`: the chat must be resolved, resolution can fail, and a \
                       turn held on a chat this daemon cannot read is answered as a private \
                       chat's",
            },
            Site {
                file: "crates/biorouter-server/src/routes/schedule.rs",
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`redact_unreachable_chats`, twice: `GET /schedule/list` asks once for a \
                       row's `current_session_id` and once for its `creator_session_id`, \
                       because the two can name DIFFERENT chats. ⚠ Reusing this predicate \
                       rather than writing a `may_name_chat` beside it is the point — 'may this \
                       caller be told this chat exists' must have ONE spelling, and a second \
                       one is the drift this census exists to catch. What differs is only what \
                       a `false` does: `/active_work` drops the row, this drops the field, \
                       because a schedule is not a chat and an idle one names none",
            },
        ],
    },
    Guard {
        ident: "work_reach",
        defined_in: SESSION_REACH,
        decides: "whether an HTTP caller naming RUNNING WORK by its own handle may stop it: the \
                  work's chat through `session_reach` itself, and work that names no chat — or a \
                  handle that names nothing — as an unreadable target, refused in the same words",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/active_work.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /active_work/{id}/cancel`, after the id is resolved to its chat \
                       (the registry entry's, or the running schedule's) and before the \
                       registry's cancel action or the scheduler's kill. It stopped any chat's \
                       work for a caller holding only the daemon secret",
            },
            Site {
                file: "crates/biorouter-server/src/routes/schedule.rs",
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /schedule/{id}/kill`, which stops the SAME run as the cancel route \
                       above, reached by the schedule id instead of the work handle. ⚠ This \
                       second call site is not redundancy: while it was missing, the gate on \
                       the row above protected nothing for its `sched:` arm, because a caller \
                       refused there re-issued the request one URL over and stopped the run \
                       anyway. Both now resolve the run to its chat first, and both pass that \
                       chat to `kill_running_job_in_session` so a run that changed under the \
                       decision is refused rather than stopped. The second call is `GET \
                       /schedule/{id}/inspect`, which answered any secret-holder with the chat \
                       a schedule is running in — the association the listing beside it \
                       redacts, handed over whole one route away",
            },
        ],
    },
    Guard {
        ident: "schedule_reach",
        defined_in: SESSION_REACH,
        decides: "whether an HTTP caller may create, run, re-time, pause, resume or remove a \
                  SCHEDULE: its work is private when a chat it names (creator, current run) is \
                  private or unreadable, or when the model its runs bind is private; an id that \
                  names no schedule is answered as a private one. The same pure decision as \
                  every other row, so work that is public stays open to any secret holder. An \
                  admission also says whether the caller could reach private work at all, and \
                  each arming door records that on the job, because a run resolves its model \
                  again when it starts (`scheduler::scheduled_run_refusal`)",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/schedule.rs",
                counts: c(6, 0, 0),
                kind: SiteKind::Guard,
                what: "the six writes: `POST /schedule/create` (the job about to be made, which \
                       names no chat and so runs on the configured default), `POST \
                       /schedule/{id}/run_now`, `/pause`, `/unpause`, `PUT /schedule/{id}` and \
                       `DELETE /schedule/delete/{id}`, each on the job resolved before anything \
                       changes. All six answered a caller holding only the daemon secret on \
                       1038a113 — run now with a new chat on a private model",
            },
            Site {
                file: "crates/biorouter-server/src/routes/workflow.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /workflows/schedule`, which adds, re-times or removes the schedule \
                       of a saved workflow through the same scheduler. ⚠ Not redundancy: without \
                       it a caller refused at `/schedule/create` re-issued the request one URL \
                       over",
            },
        ],
    },
    Guard {
        ident: "schedule_work",
        defined_in: "crates/biorouter/src/scheduler.rs",
        decides: "THE definition of a schedule's private work: a chat it names (creator, \
                  current run) is private or unreadable, or the model its runs bind — \
                  `scheduled_run_provider_name`'s declared tier — is private. Every door that \
                  changes a schedule answers with this one function, so the HTTP routes and \
                  the slash verbs cannot drift into two definitions",
        status: Status::Wired,
        sites: &[
            Site {
                file: SESSION_REACH,
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`schedule_reach`, for an unproven public caller naming a schedule that \
                       exists — the six `/schedule/…` writes and `POST /workflows/schedule`",
            },
            Site {
                file: "crates/biorouter/src/agents/recurring.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`Agent::slash_verb_reaches`, which every `/schedule remove|delete|run|\
                       pause|unpause|resume` and `/loop stop <id|all>` asks before it changes \
                       anything, for a chat that is not on a private model. Independent QA, \
                       2026-09-14: `Agent::reply` runs `execute_command` before any model \
                       call, and a public chat accepts `POST /reply` from any holder of the \
                       daemon secret, so the verbs paused, ran and removed a private chat's \
                       schedule for a caller the HTTP routes refused. The import is the \
                       module's `use` of it",
            },
        ],
    },
    Guard {
        ident: "default_bind_refused",
        defined_in: "crates/biorouter/src/privacy/refusal.rs",
        decides: "DR-16 through a RESTORE: whether a chat that records no model may be bound to \
                  a PRIVATE configured default on the caller's standing (`DefaultBind`), which \
                  `routes::agent::RestoreStanding` decides by `new_chat_bind_decision` — the \
                  proof on a keyed daemon, SD-12's launch-pinned exemption on a keyless one",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter/src/agents/agent.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`Agent::restore_provider_from_session`, the ONE place a restore binds \
                       the default, asked of the constructed instance's own tier before \
                       anything is bound. Independent QA, 2026-09-14: with only the daemon \
                       secret, `POST /agent/restart` of a chat whose row named no provider \
                       wrote `versa_azure` onto it",
            },
            Site {
                file: "crates/biorouter-server/src/routes/agent.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`RestoreStanding::refuse_before_restoring`, the same predicate asked of \
                       the DECLARED tier by every door before it changes anything — `POST \
                       /agent/restart` before it evicts the agent, `POST \
                       /agent/update_working_dir` before it writes the directory, and a \
                       workspace turn injected into a cold chat before it hydrates one",
            },
        ],
    },
    Guard {
        ident: "mints_knowledge_base",
        defined_in: SESSION_REACH,
        decides: "whether a caller may take a knowledge-base id at all — asked WITHOUT the id, \
                  because create refuses one that is taken and an answer that varied with the id \
                  would say which private bases exist",
        status: Status::Wired,
        sites: &[Site {
            file: "crates/biorouter-server/src/routes/knowledge.rs",
            counts: c(1, 0, 0),
            kind: SiteKind::Guard,
            what: "`POST /knowledge/bases`, before `create_base_in` reads the id — which used \
                   to answer a guessed private id with `400 … already exists at <absolute \
                   path>` to a caller holding nothing but the daemon secret",
        }],
    },
    Guard {
        ident: "reach_knowledge_base",
        defined_in: SESSION_REACH,
        decides: "whether an HTTP caller naming a knowledge base may reach it: the chat gate's \
                  decision with the base's tier as the target, an absent or malformed id \
                  answered as a private one",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/apps.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /apps/{id}/export`: each knowledge base a full export would carry, \
                       the decision `GET /knowledge/bases/{id}/export` gets — a base out of the \
                       caller's reach is left out of the zip and named in its report",
            },
            Site {
                file: "crates/biorouter-server/src/routes/knowledge.rs",
                counts: c(4, 0, 0),
                kind: SiteKind::Guard,
                what: "the bases listing's filter, the selection response's filter, and \
                       `POST /knowledge/active`'s two uses: the refusal for pinning a base the \
                       caller cannot reach, and the predicate `set_selection_within` merges by",
            },
            Site {
                file: SESSION_REACH,
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`gate_knowledge_base`, which asks it for every `/knowledge/bases/{id}` \
                       route before the handler runs",
            },
            Site {
                file: "crates/biorouter-server/src/routes/workflow.rs",
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /workflows/create`: a generated workflow's visible bases, and its \
                       default one",
            },
        ],
    },
    Guard {
        ident: "gate_knowledge_base",
        defined_in: SESSION_REACH,
        decides: "the knowledge-base reach gate, as ONE route layer on the sub-router holding \
                  exactly the routes that name a base by `{id}`",
        status: Status::Wired,
        sites: &[Site {
            file: "crates/biorouter-server/src/routes/knowledge.rs",
            counts: c(0, 1, 0),
            kind: SiteKind::Guard,
            what: "`route_layer(from_fn_with_state(svc, session_reach::gate_knowledge_base))` \
                   in `knowledge::router`. A REFERENCE, as `gate_knowledge_active`'s is: a \
                   middleware never appears with parentheses",
        }],
    },
    Guard {
        ident: "target_tier",
        defined_in: SESSION_REACH,
        decides: "resolves the named session's tier metadata-only, failing closed to Unreadable",
        // `WiredThrough("session_reach")` until PR-15, when the dictation stream became
        // its first caller outside this file.
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/dictation.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /dictation/streams` into a chat (PR-15, DD-30): the stored tier \
                       `chat_reach` reads beside the live model, asked only after \
                       `session_reach` admitted the caller",
            },
            Site {
                file: "crates/biorouter-server/src/routes/dictionary.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`read_chat` (PR-16): the stored tier an observation's stamp reads \
                       through `chat_reach` beside the live model, asked only after \
                       `session_reach` admitted the caller",
            },
            Site {
                file: SESSION_REACH,
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`session_reach`'s tier lookup, deliberately `with_messages: false` so \
                       resolving a tier is never the way to load the transcript being refused; \
                       and `HttpCaller::lists_work`'s, for a row of running work that names a \
                       chat — the same lookup, so an unreadable chat fails closed there too. \
                       `schedule_reach` asked it once per chat a schedule names until \
                       2026-09-14, when that decision moved whole into \
                       `scheduler::schedule_work` so the `/schedule` and `/loop` slash verbs \
                       could answer with the same definition rather than a second one",
            },
        ],
    },
    Guard {
        ident: "dictation_standing",
        defined_in: SESSION_REACH,
        decides: "whether private dictionary entries may shape a caller's dictation (PR-15, \
                  DD-29, DD-30): the user action proof or a private provider stated in \
                  `X-Caller-Provider`, never a `serve` daemon's operator tier (SD-10)",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/dictation.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /dictation/streams`, once per stream, beside the target the \
                       daemon decides; `bias_request` reads both (PERF-1)",
            },
            Site {
                file: "crates/biorouter-server/src/routes/dictionary.rs",
                counts: c(6, 0, 0),
                kind: SiteKind::Guard,
                what: "the dictionary routes (PR-16, oracle rule 5): whether a listing, an \
                       add's, an edit's and a tier change's answer carry the usage figures and \
                       `suggested_in`; whether an export may include private rows; and the \
                       standing an observation's `bias_set` admission check reads",
            },
            Site {
                file: SESSION_REACH,
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`authorizes_dictionary_write` (a private add or import without private \
                       standing is refused on a daemon with no key) and \
                       `dictionary_write_stamp` (such a daemon stamps a write with the \
                       caller's standing): the same standing a dictation is biased by, never \
                       the served operator's tier",
            },
        ],
    },
    Guard {
        ident: "lists_dictionary_entry",
        defined_in: SESSION_REACH,
        decides: "whether a caller may be shown a dictionary row of a tier (PR-16, DD-37): \
                  the listing decision a chat of that classification gets, omission and \
                  never a count",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/dictionary.rs",
                counts: c(3, 0, 0),
                kind: SiteKind::Guard,
                what: "`listable`, the one per row test the listing, the row an edit or a \
                       delete names, and the ids a bulk route names all ask, again under the \
                       store's lock before each write; the tiers the same letters warning and \
                       the import counts consult (`reach_of`); and whether an observation's \
                       stamp is a tier the caller may list (a silent no op when not)",
            },
            Site {
                file: SESSION_REACH,
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`authorizes_dictionary_write`, which admits a keyless write only for a \
                       row the caller can list, and `gate_dictionary_entry`, which answers an \
                       unlisted id exactly as a missing one",
            },
        ],
    },
    Guard {
        ident: "authorizes_dictionary_write",
        defined_in: SESSION_REACH,
        decides: "whether a caller may make a dictionary write (PR-16, DD-38), from the \
                  request alone: the proof on a daemon that holds a key; on one that holds \
                  none, a listable row, with tier changes, raised settings and a private add \
                  without private standing refused with the host sentences",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/dictionary.rs",
                counts: c(12, 0, 0),
                kind: SiteKind::Guard,
                what: "every write and the export: add, edit, delete, the tier change (again \
                       after the gate, right before the tier proof is built), the bulk routes' \
                       shared reader, import, export, observations, the settings change, the \
                       settings answer's `authority`, which asks once for managed writes and \
                       separately for deletion with the same caller proof and row reach, so \
                       neither control is offered when its route would refuse, and the agent \
                       row Undo (PR-23), which asks with the row's listed tier before the \
                       store write",
            },
            Site {
                file: SESSION_REACH,
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`gate_dictionary_entry`, which decides the tier route before it reads \
                       the id, so its answer is the same for every id",
            },
        ],
    },
    Guard {
        ident: "dictionary_write_stamp",
        defined_in: SESSION_REACH,
        decides: "the tier a dictionary write is stamped with (PR-16, Who stamps the tier): \
                  the person's choice on a daemon that holds a key, the caller's standing on \
                  one that holds none, and never a tier a client sends",
        status: Status::Wired,
        sites: &[Site {
            file: "crates/biorouter-server/src/routes/dictionary.rs",
            counts: c(3, 0, 0),
            kind: SiteKind::Guard,
            what: "an add, an edit (whose higher stamp lands in the private twin) and an \
                   import's default tier",
        }],
    },
    Guard {
        ident: "gate_dictionary_entry",
        defined_in: SESSION_REACH,
        decides: "the route layer on every `/dictionary/entries/{id}` route (PR-16, oracle \
                  rule 2): 404 with MSG-NOT-FOUND for a missing id and an unlisted one alike, \
                  and the tier route's authority decided before the id is read",
        status: Status::Wired,
        sites: &[Site {
            file: "crates/biorouter-server/src/routes/dictionary.rs",
            counts: c(0, 1, 0),
            kind: SiteKind::Guard,
            what: "`route_layer(from_fn(session_reach::gate_dictionary_entry))` on \
                   `entry_routes()` in `routes`. A REFERENCE, as `gate_knowledge_base`'s is: a \
                   middleware never appears with parentheses",
        }],
    },
    // ----------------------------------------------------- extension tiering
    Guard {
        ident: "resolve_extension",
        defined_in: EXTENSIONS,
        decides: "an extension's tier AND affiliation together, from the registry plus \
                  provenance: one resolution, one call (DR-26)",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/agent.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /agent/add_extension`",
            },
            Site {
                file: "crates/biorouter/src/agents/extension_manager.rs",
                counts: c(4, 0, 0),
                kind: SiteKind::Guard,
                what: "add / list / dispatch / tool-listing paths of the extension manager",
            },
            Site {
                file: "crates/biorouter/src/agents/subagent_tool.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "subagent spawn, which builds a whole new agent and so cannot inherit a \
                       capability",
            },
            Site {
                file: "crates/biorouter/src/marketplace.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "the live-registry anti-downgrade path, which raises a registry row to any \
                       stricter built-in or previously learned extension authority",
            },
            Site {
                file: "crates/biorouter/src/privacy/refusal.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`extension_enable_refusal`, the ONE enable gate, the resolution the \
                       tier arm and the affiliation arm both read off, which is why there is \
                       one of it. It absorbed the two separate resolutions that used to sit in \
                       `extension_manager_extension.rs` and `workspace_extension.rs`, one per \
                       copy of that gate",
            },
            Site {
                file: EXTENSIONS,
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`resolve_in`, the testable inner form",
            },
            Site {
                file: "crates/biorouter/src/privacy/mod.rs",
                counts: c(0, 0, 1),
                kind: SiteKind::Guard,
                what: "the `pub use` re-export",
            },
        ],
    },
    Guard {
        ident: "classify_extension",
        defined_in: EXTENSIONS,
        decides: "an extension's tier from its bare name",
        status: Status::Unwired(
            "Deliberate and not a defect: production resolves tier and affiliation together \
             through `resolve_extension`, which consults the registry and provenance rather \
             than joining on a name. Recorded so the zero is on the record as a decision \
             rather than as an oversight; those two look identical from the outside, which \
             is the whole reason this census exists.",
        ),
        sites: &[Site {
            file: "crates/biorouter/src/privacy/mod.rs",
            counts: c(0, 0, 1),
            kind: SiteKind::Guard,
            what: "the `pub use` re-export, an import, and imports are not wiring",
        }],
    },
    Guard {
        ident: "classify_extension_entry",
        defined_in: EXTENSIONS,
        decides: "the same classification for a configured extension entry rather than a bare \
                  name",
        status: Status::Unwired(
            "Superseded by `resolve_extension` (DR-26 / Task 47: affiliation rides the same \
             resolution, in the same call). Its one call site is inside `classify_extension`, \
             which is itself unwired: a two-link dead chain, which is why it is filed here \
             rather than as `WiredThrough`.",
        ),
        sites: &[
            Site {
                file: EXTENSIONS,
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "called by `classify_extension`, itself unwired",
            },
            Site {
                file: "crates/biorouter/src/privacy/mod.rs",
                counts: c(0, 0, 1),
                kind: SiteKind::Guard,
                what: "the `pub use` re-export",
            },
        ],
    },
    Guard {
        ident: "private_extension_ids",
        defined_in: EXTENSIONS,
        decides: "enumerates the private extension ids",
        status: Status::Unwired(
            "Intended, and its own doc says so: the one legitimate reader is the disclosure \
             test in `privacy::refusal`. A production caller appearing here would mean some \
             path is deciding by enumeration instead of by classification.",
        ),
        sites: &[Site {
            file: "crates/biorouter/src/privacy/mod.rs",
            counts: c(0, 0, 1),
            kind: SiteKind::Guard,
            what: "the `pub use` re-export",
        }],
    },
    // --------------------------------------------- curated rows: guards that
    // live outside the three completeness modules.
    Guard {
        ident: "visible_to",
        defined_in: "crates/biorouter/src/privacy/mod.rs",
        decides: "VIS: the one crossing between the capability lattice and the classification \
                  lattice, which every other visibility predicate is defined in terms of",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter/src/agents/chatrecall_extension.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "chat recall, which searches other conversations",
            },
            Site {
                file: "crates/biorouter/src/knowledge/conversation_ingest.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`readable`, the gate on ingesting a conversation into a knowledge base",
            },
            Site {
                file: "crates/biorouter/src/dictation/bias.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`bias_set` (PR-14, DD-36): which dictionary rows a dictation may use, \
                       a row's tier read as a classification against the reach its target \
                       and its caller's standing give",
            },
            Site {
                file: VISIBILITY,
                counts: c(3, 0, 1),
                kind: SiteKind::Guard,
                what: "`may_read`, `may_write` and `appears_in_list` are each defined as VIS \
                       (plus lineage, for write), and the `use super::` that brings it in",
            },
        ],
    },
    Guard {
        ident: "bias_set",
        defined_in: "crates/biorouter/src/dictation/bias.rs",
        decides: "which dictionary rows a dictation may use (PR-14, DD-36): the one value the \
                  token prompt, the decoder bonus, the aliases, the exact keys and the fuzzy \
                  matcher all read, filtered by tier through `visible_to` and by institution \
                  through the extension gate",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-cli/src/dictation/frontend.rs",
                counts: c(2, 0, 1),
                kind: SiteKind::Guard,
                what: "terminal front ends use the shared bias set for a session recording and to admit an observed row",
            },
            Site {
                file: "crates/biorouter-cli/src/commands/dictate.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`biorouter dictate`: the dictionary set for this recording, after \
                       the terminal's DD-77 standing or the person's private terms answer \
                       sets its target and standing, and the `use` that brings it in",
            },
            Site {
                file: "crates/biorouter-cli/src/commands/transcribe.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`biorouter transcribe`, with the target and standing of DD-77's terminal \
                       rule, and the `use` that brings it in",
            },
            Site {
                file: "crates/biorouter-server/src/routes/dictionary.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`POST /dictionary/observations` (PR-16, P-16): whether a private row's \
                       institutions all admit the chat's model, asked of that one row through \
                       the bias set itself so the rule has no second spelling, and the `use` \
                       that brings it in",
            },
            Site {
                file: "crates/biorouter/src/dictation/mod.rs",
                counts: c(0, 0, 1),
                kind: SiteKind::Guard,
                what: "the `pub use` re-export",
            },
        ],
    },
    // PERF-1: the daemon's dictation stream stopped calling `bias_set` and asks the
    // daemon's `DictionaryCache` instead, which runs the same predicate for every request
    // and serves a kept set only when that request's own decision admitted exactly its
    // rows. The stream's row moved here from `bias_set`'s, so the census still sees the
    // stream ask a guard, and fails if it stops.
    Guard {
        ident: "bias_request",
        defined_in: "crates/biorouter/src/dictation/bias.rs",
        decides: "which dictionary rows a daemon's dictation may use (PERF-1, DD-36): \
                  `bias_set`'s predicate, run in full for every request, with the rows it \
                  admitted found in the daemon's `DictionaryCache` instead of copied again",
        status: Status::Wired,
        sites: &[Site {
            file: "crates/biorouter-server/src/routes/dictation.rs",
            counts: c(1, 0, 0),
            kind: SiteKind::Guard,
            what: "`POST /dictation/streams` (PR-15, PERF-1), with the target the daemon \
                   decided and `dictation_standing`, built off the path from stop to text",
        }],
    },
    Guard {
        ident: "chat_reach",
        defined_in: "crates/biorouter/src/dictation/target.rs",
        decides: "whether a chat is private, from its stored tier and its live agent's \
                  bound model (DD-30): the one answer the dictation stream target, the \
                  observation stamp and the extractor stamp share, a target counting \
                  `Unknown` as public and a stamp as private",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-cli/src/dictation/frontend.rs",
                counts: c(2, 0, 2),
                kind: SiteKind::Guard,
                what: "terminal adds and observations use the session's stored tier and live model to stamp a row",
            },
            Site {
                file: "crates/biorouter-server/src/routes/dictation.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /dictation/streams` into a chat (PR-15), after `session_reach` \
                       admitted the caller",
            },
            Site {
                file: "crates/biorouter-server/src/routes/dictionary.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`read_chat` (PR-16): the stamp of an observation and of an add's origin \
                       chat, as a stamp reads it (Private unless Public), after \
                       `session_reach` admitted the caller",
            },
            Site {
                file: "crates/biorouter/src/dictation/extractor.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`stamp_tier` (PR-24, DD-57): the chat extractor's stamp, over the \
                       chat's stored tier and the provider its turn pinned, as a stamp reads \
                       it (Private unless Public)",
            },
            Site {
                file: "crates/biorouter/src/dictation/mod.rs",
                counts: c(0, 0, 1),
                kind: SiteKind::Guard,
                what: "the `pub use` re-export",
            },
        ],
    },
    Guard {
        ident: "assert_alt_provider_matches_session",
        defined_in: "crates/biorouter/src/privacy/alt_provider.rs",
        decides: "Gate H's RATCHETING half: whether a provider named for a knowledge ingest may \
                  differ in tier from the session whose content it will fold into a base",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter/src/agents/knowledge_tool.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`build_model_ref_provider`, the ONE Gate H site both knowledge paths \
                       share — `platform__ingest_source`'s `model` argument (a provider name \
                       the MODEL wrote) and a base's stored `default_model` on a scheduled \
                       digest. Its laxer sibling `assert_alt_provider_allowed` is what used to \
                       be here, and it refuses only the DOWNWARD choice: the tier that comes \
                       back becomes `caller_capability`, crosses to `caller_is_private` and \
                       lands in `knowledge::tier::raise_unlocked`, a permanent monotone \
                       ratchet — so the upward choice Gate A calls harmless let a PUBLIC chat \
                       privatise its own base for good and then be refused at every KB read \
                       choke point. If this row ever reads ZERO calls, that hole is open again \
                       and no behavioural test elsewhere will say so, because the sibling \
                       predicate is still correct for the two paths that keep it",
            },
            Site {
                file: "crates/biorouter/src/privacy/mod.rs",
                counts: c(0, 0, 1),
                kind: SiteKind::Guard,
                what: "the `pub use` beside its sibling's, so `crate::privacy::` is the one \
                       path both halves of Gate H are called through",
            },
        ],
    },
    Guard {
        ident: "assert_alt_tier_allowed",
        defined_in: "crates/biorouter/src/privacy/alt_provider.rs",
        decides: "Gate H's downward half for an alternate model that is not a `Provider`: whether \
                  a decision model of a declared tier may read a session of this classification, \
                  with the master switch as the caller sampled it",
        status: Status::Wired,
        sites: &[Site {
            file: "crates/biorouter/src/decision/service.rs",
            counts: c(1, 0, 1),
            kind: SiteKind::Guard,
            what: "`privacy_allows`, the one place every decision is gated, plus its import. Every \
                   decision path goes through `DecisionService` (a monitor's window, the `decide` \
                   tool's chunks, a phase gate's hand-off, a decision chat, `POST \
                   /decisions/decide` and the CLI), and the service asks `privacy_allows` before \
                   anything is sent: for the model a batch resolved, again per chunk on the \
                   classification as it stands then (a chat raised mid-batch stops reaching a \
                   public model at the next chunk), for each candidate of an automatic choice, \
                   and for the advisory `unavailableReason` of `GET /decisions/models`. If this \
                   row ever reads ZERO calls, a private chat's text can reach a public decision \
                   model and no route test will say so, because each one drives a fake source",
        }],
    },
    Guard {
        ident: "bind_allowed",
        defined_in: "crates/biorouter/src/privacy/mod.rs",
        decides: "Gate A: whether a provider of tier P may be bound to a session classified T",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-cli/src/session/builder.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "the CLI's session builder",
            },
            Site {
                file: "crates/biorouter/src/agents/agent.rs",
                counts: c(4, 0, 0),
                kind: SiteKind::Guard,
                what: "`Agent::update_provider` and the turn barrier, the reason a public \
                       caller cannot use `workspace_set_tools`' provider switch to read a \
                       private chat",
            },
            Site {
                file: "crates/biorouter/src/privacy/alt_provider.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "the alternate-provider path, plus its import",
            },
            Site {
                file: "crates/biorouter/src/privacy/mod.rs",
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`SessionClassification::bind_allowed`'s inherent-method form, plus \
                       `tool_bind_allowed` — the model-facing predicate — which COMPOSES this \
                       one rather than re-spelling its cells. The second call is deliberate and \
                       is the point of that predicate's shape: `tool_bind_allowed` is Gate A's \
                       rule AND DR-16's, and if it restated Gate A's half instead of calling it, \
                       the two would be free to drift and this census would see only one of \
                       them. Its own row is below",
            },
            Site {
                file: "crates/biorouter/src/workflow/privacy.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "workflow execution, plus its import",
            },
            Site {
                file: "crates/biorouter/src/agents/workspace_extension.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`workspace_set_tools`' pre-flight (QA finding F4): asked BEFORE the \
                       always-confirm card, so a provider switch this bind would refuse is \
                       refused before the user is asked to approve it, instead of after. It \
                       is a pre-flight, not the gate — `Agent::update_provider`'s conditional \
                       `WHERE` still decides when the change is applied — and it asks this \
                       predicate by name rather than re-spelling it, only when the write gate \
                       resolved a classification (i.e. under enforcement). \
                       ⚠ It is ALSO the only privacy check on this tool's provider bind at \
                       the dispatch boundaries no inspector reaches — an `execute_code` \
                       script's inner call and `POST /agent/call_tool` both go straight to \
                       `ExtensionManager::dispatch_tool_call`, so the always-confirm card \
                       named above does not run for either. Do not weaken this on the \
                       reasoning that a card backs it up: for two of its callers, nothing does",
            },
        ],
    },
    Guard {
        ident: "tool_bind_allowed",
        defined_in: "crates/biorouter/src/privacy/mod.rs",
        decides: "Gate A's rule PLUS DR-16's, for the surface a MODEL asks the bind on: it may \
                  LOWER a conversation's capability, never RAISE it",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter/src/agents/workspace_extension.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`workspace_set_tools`' pre-flight, beside its `bind_allowed` sibling and \
                       after it, so the more specific refusal owns the downward case. \
                       `bind_allowed` permits every bind onto a public conversation (Gate A \
                       refuses only the downward one), which let a public-tier model hand any \
                       conversation it could write to a private provider — Private capability, \
                       and a permanent ratchet of that conversation's stored `privacy_tier` on \
                       its next turn. If this row ever reads ZERO calls, that hole is open \
                       again and no behavioural test elsewhere will say so, because the \
                       predicate itself is still correct",
            },
            Site {
                file: "crates/biorouter/src/privacy/alt_provider.rs",
                counts: c(1, 0, 1),
                kind: SiteKind::Guard,
                what: "`assert_alt_provider_matches_session`, Gate H's ratcheting half, plus \
                       its import. ⚠ This row is why the sibling row above no longer claims to \
                       be this predicate's only caller: DR-16's rule now has TWO model-facing \
                       surfaces, a bind (`workspace_set_tools`' provider switch) and a \
                       construction that binds nothing (a knowledge ingest's `model` argument, \
                       whose tier ratchets a knowledge base instead of a session). Both CALL \
                       this predicate rather than re-spelling `is_private() == is_private()`, \
                       so the two can never answer the raise cell differently. A new site that \
                       inlines the comparison would satisfy every behavioural test and be \
                       invisible here, which is exactly the drift this census exists to see",
            },
        ],
    },
    Guard {
        ident: "privacy_refusal",
        defined_in: "crates/biorouter/src/privacy/refusal.rs",
        decides: "composes the refusal a tier gate returns: the one sentence every gate uses, \
                  so a hand-rolled comparison is visible in review",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter/src/agents/agent.rs",
                counts: c(0, 5, 0),
                kind: SiteKind::Unrelated,
                what: "a LOCAL VARIABLE named `privacy_refusal` in the turn barrier. The second \
                       standing proof that a name match is not a call site. The count went 4 -> 5 \
                       when Gate B began stating what every turn runs on: the frame is built only \
                       for a turn that is not being refused, so the local is read once more. Still \
                       ZERO calls — the guard function itself gained no caller here",
            },
            Site {
                file: "crates/biorouter/src/agents/extension_manager.rs",
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "Gate E's two filters (the tool list and the extension listing), which \
                       only ask whether to hide and compose no sentence. It was FOUR until the \
                       2026-09-10 test drive's finding M18: the fourth was \
                       `assert_extension_reachable`, the one gate that reads an unknown name \
                       as Private, where this function's flat sentence asserted a privateness \
                       the gate had not established. That call now goes to \
                       `private_or_absent_refusal`, tracked in its own row. Moving it back \
                       would put the false claim back. It was THREE until DR-33: Gate C's \
                       dispatch refusal moved onto `privacy_refusal_explained`, so a chat made \
                       public by a decision model is told why instead of to switch models",
            },
            Site {
                file: "crates/biorouter/src/agents/subagent_tool.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "subagent spawn",
            },
            Site {
                file: "crates/biorouter/src/privacy/refusal.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`privacy_refusal_explained` (DR-33), which is this function plus the \
                       decision-model sentence; `extension_enable_refusal`'s tier arm reaches it \
                       through that, so it is still the enable doors' one ask. \
                       `workspace_extension.rs` used to ask it directly and \
                       `extension_manager_extension.rs` hand-wrote the same rule in its own \
                       words; both now go through the shared gate, which is what fixed the \
                       clause order the two copies disagreed about",
            },
        ],
    },
    Guard {
        ident: "privacy_refusal_explained",
        defined_in: "crates/biorouter/src/privacy/refusal.rs",
        decides: "DR-33 (design §3.9): `privacy_refusal`'s rule, unchanged, with the sentence \
                  naming the decision model when the sampled capability says one made the chat \
                  public, or (issue #56) saying the chat's new private model takes effect from \
                  the next message when the person switched it while the turn ran. It decides \
                  nothing of its own; it exists so no gate re-spells the rule to pick its words",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter/src/agents/extension_manager.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "Gate C, `authorize_dispatch_boundaries`: the one refusal a model reads \
                       when a chat reaches for a private extension at dispatch. The words are \
                       read off the slot only when the sample says a decision model lowered it \
                       and the extension is private, which is exactly the refusal path. Gate \
                       E's two filters stay on `privacy_refusal`: they hide and say nothing",
            },
            Site {
                file: "crates/biorouter/src/privacy/refusal.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`extension_enable_refusal`'s tier arm",
            },
        ],
    },
    Guard {
        ident: "extension_enable_refusal",
        defined_in: "crates/biorouter/src/privacy/refusal.rs",
        decides: "whether an extension may be ENABLED: the tier arm, then the affiliation arm, \
                  then #42's operator pin: one clause order, shared by every agent-facing \
                  enable door, with both privacy arms above the one arm that speaks about this \
                  machine",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter/src/agents/extension_manager_extension.rs",
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "#117's marketplace install tool, which gates the ATTACH rather than \
                       the install. Installing writes bytes to disk and is not an enable, \
                       so a refusal must still leave the package correctly installed. The \
                       manager's ordinary enable door uses the stricter \
                       `extension_manager_enable_refusal`, tracked in the next row. \
                       TWO calls, and the second is not redundant: the first asks with the \
                       REGISTRY's name, which is all that is known before the download, and \
                       the registry's name and the installed one demonstrably differ in \
                       production (SPOKEAgent advertises `spokeagent-0.4.1` and installs as \
                       `spokeagent`). The second is a `guard_attach` closure asking the same \
                       gate again with the manifest's real name, at the only point it is \
                       knowable. Deleting either leaves a name the gate was never asked about",
            },
            Site {
                file: "crates/biorouter/src/agents/workspace_extension.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`refuse_gated_extension_enable`, which both workspace enable doors \
                       reach: `workspace_set_tools {add_extensions}` and `workspace_open \
                       {new:{extensions}}`. It renders the refusal as a `String` and decides \
                       nothing",
            },
        ],
    },
    Guard {
        ident: "extension_manager_enable_refusal",
        defined_in: "crates/biorouter/src/privacy/refusal.rs",
        decides: "whether Extension Manager may attach an extension: the shared enable rule \
                  plus an absolute public-to-private boundary and the narrow proof-backed \
                  override for a public extension's persisted operator pin",
        status: Status::Wired,
        sites: &[Site {
            file: "crates/biorouter/src/agents/extension_manager_extension.rs",
            counts: c(2, 0, 0),
            kind: SiteKind::Guard,
            what: "TWO callers, both deliberate. (1) `check_enable_allowed_impl`, shared by \
                   the ordinary manager enable path and its proof-backed retry after a user \
                   approval. (2) `remove_extension`'s preflight, which reuses this guard so \
                   an uninstall is bound by the SAME tier and affiliation arms as an enable \
                   — a public caller must not be able to uninstall a private extension it \
                   may not even see. It passes `persisted: false, user_granted: false` on \
                   purpose: those two arms exist to refuse re-enabling what an operator \
                   pinned OFF, which is an argument about turning something on and does not \
                   apply in the removal direction.",
        }],
    },
    Guard {
        ident: "private_or_absent_refusal",
        defined_in: "crates/biorouter/src/privacy/refusal.rs",
        decides: "the sentence Gate C' returns — the resource and prompt surface, whose \
                  unknown-name default is Private, so its refusal must cover BOTH a private \
                  extension and a name that is not installed and must not tell the two apart",
        status: Status::Wired,
        sites: &[Site {
            file: "crates/biorouter/src/agents/extension_manager.rs",
            counts: c(1, 0, 0),
            kind: SiteKind::Guard,
            what: "`reachability_refusal`, the ONE caller and deliberately so — the decision \
                   `assert_extension_reachable` (and, through `manageability_refusal`, \
                   `assert_extension_manageable` and `workspace_set_tools`' pre-flight) \
                   asks, split out of the method in F4 so a caller with no manager asks it \
                   rather than re-spelling it. Every other tier gate resolves its extension \
                   from an installed record before it refuses, so `privacy_refusal`'s flat \
                   statement is a fact there and the better thing to hand a model. A second \
                   caller of this one would be a gate hedging about an extension it can see",
        }],
    },
    Guard {
        ident: "tier_refuses",
        defined_in: "crates/biorouter/src/privacy/refusal.rs",
        decides: "the boolean under `privacy_refusal`: private extension, non-private caller. \
                  One rule, three renderings: the model's sentence, the typed HTTP body, and a \
                  bare `if`",
        status: Status::Wired,
        sites: &[
            Site {
                file: "crates/biorouter-server/src/routes/agent.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`POST /agent/add_extension`, the USER's enable door. It cannot call \
                       `extension_enable_refusal` (its refusal is the typed \
                       `PrivateExtensionOverHttp` body, and a user proceeds past the other two \
                       arms), so it asks the predicate instead of re-typing it, which is what \
                       it did until this seam was closed",
            },
            Site {
                file: "crates/biorouter/src/privacy/refusal.rs",
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`privacy_refusal` and `private_or_absent_refusal`, which are this \
                       predicate plus the two sentences the model reads. TWO renderings and \
                       one rule is the whole point of the split: the gate that resolved the \
                       extension states it is private, the gate that read an unknown name as \
                       Private states the disjunction, and neither re-derives WHEN to refuse",
            },
        ],
    },
    Guard {
        ident: "grant_needs_system_authentication",
        defined_in: "crates/biorouter/src/privacy/mixing.rs",
        decides: "whether granting a mixing exception must cost an OS authentication (Strict \
                  raises the price of a yes)",
        status: Status::Unwired(
            "OPERATOR DECISION OUTSTANDING, and the smallest of the three. The production \
             grant route re-derives the same rule inline (`routes/agent.rs`: `if policy != \
             MixingPolicy::Strict { return Ok(()); }`) instead of asking the predicate. The \
             two agree today. That is the 'one table becomes seven slightly-different tables' \
             shape this campaign keeps citing, and the repair is one line at the route, not a \
             new mechanism.",
        ),
        sites: &[],
    },
    Guard {
        ident: "no_wider_than",
        defined_in: "crates/biorouter/src/privacy/capability.rs",
        decides: "how far a call on a running turn reaches: the lesser of the slot sampled now \
                  and the capability the turn started with, so a model switch that lands while \
                  a turn runs widens nothing until the next turn (issue #56). The reply loop \
                  pins the provider a turn's requests run on for the whole turn, so without it \
                  a public chat switched to a private model mid-turn had private extension \
                  results sent to the public model",
        status: Status::Wired,
        sites: &[Site {
            file: "crates/biorouter/src/agents/agent.rs",
            counts: c(2, 0, 0),
            kind: SiteKind::Guard,
            what: "`Agent::capped_to_turn`, which every call on a running turn is admitted \
                   through (`sample_turn_call`: the loop's tool calls, the schedule branch, the \
                   prefetch, Gate E's roster and the spawn check; and, called directly, the two \
                   knowledge-ingest branches in `knowledge_tool.rs` and \
                   `knowledge_source_tool.rs`), and `TurnCeilings::lower`, \
                   which narrows a turn's ceiling to the slot as the loop pinned its provider. \
                   Losing either caller leaves every behavioural gate test green on a turn \
                   that never switches model, which is all of them but one",
        }],
    },
];

// ---------------------------------------------------------------------------
// DR-33: where a chat's decision models are folded into its tier.
// ---------------------------------------------------------------------------

/// Where `privacy::decision_axes` is COMPUTED, by qualified spelling, per
/// production file (design §7.2). `from_row` and `fold` are ordinary words
/// (`sqlx::FromRow::from_row`, `Iterator::fold`), so the guard registry above
/// cannot count them by identifier; this counts the path-qualified call, and
/// [`decision_axes_is_never_imported_by_name`] keeps a caller from dropping the
/// path to hide from it.
///
/// The fold is computed at the bind, at Gate B, after an admitted lowering and
/// at spawn; everything else only explains or predicts. `agent.rs` has one site
/// because all three of its moments go through `install_bound_provider`. Pin
/// the measured count per file; a later addition must argue for itself here.
const DECISION_AXES_SITES: &[(&str, &str, usize, &str)] = &[
    (
        "decision_axes::from_row(",
        "crates/biorouter/src/agents/agent.rs",
        1,
        "`install_bound_provider`, the slot's one writer: the bind, Gate B's refresh and \
         `lower_decision_axes` all reach the fold through it",
    ),
    (
        "decision_axes::fold(",
        "crates/biorouter/src/agents/subagent_tool.rs",
        1,
        "spawn (design §3.4): the child's effective tier, its model folded with the public \
         ceiling it is about to be stamped with, which R4/DR-19/DR-31 and the extension filter \
         read. The same fold the child's slot computes from its row once the ceiling lands",
    ),
    (
        "decision_axes::from_row(",
        "crates/biorouter/src/decision/service.rs",
        1,
        "a prediction that installs nothing: `DecisionSession::with_row_tier_context` reads \
         whether the stored row's decision models lower it now, so a picker row and a save's \
         usability check can say what the save does (design §3.2, §5)",
    ),
    (
        "decision_axes::from_row(",
        "crates/biorouter-cli/src/session/output.rs",
        1,
        "explains, installs nothing: `decision_privacy_reason`, the terminal banner's and the \
         TUI header's \"public because monitor … uses joey-1 · Baseten\" (design §5), from the \
         stored row with the agent's own bound model",
    ),
    (
        "decision_axes::from_row(",
        "crates/biorouter-server/src/routes/session.rs",
        1,
        "explains, installs nothing: `GET /sessions/{id}`'s `privacyBasis`, the decision model \
         that made the chat public, from the stored row with the bound model's tier (design §5), \
         so no renderer recomputes the fold",
    ),
];

fn qualified_counts(tree: &Tree, needle: &str) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for (rel, lines) in &tree.production {
        if rel.ends_with("privacy/decision_axes.rs") {
            continue;
        }
        let hits: usize = lines.iter().map(|(_, t)| t.matches(needle).count()).sum();
        if hits > 0 {
            out.insert(rel.clone(), hits);
        }
    }
    out
}

#[test]
fn decision_axes_are_computed_only_where_this_census_says() {
    let tree = scan_tree();
    for needle in ["decision_axes::from_row(", "decision_axes::fold("] {
        let measured = qualified_counts(&tree, needle);
        let declared: BTreeMap<String, usize> = DECISION_AXES_SITES
            .iter()
            .filter(|(n, ..)| *n == needle)
            .map(|(_, file, count, _)| (file.to_string(), *count))
            .collect();
        assert_eq!(
            measured, declared,
            "the places a chat's decision models are folded into its tier changed for \
             `{needle}`. Add or grow a row in DECISION_AXES_SITES saying what the new site is \
             (a bind, Gate B, a lowering, a spawn, or a prediction that installs nothing)"
        );
    }
}

/// The census above counts the qualified spelling. A `use` that names
/// `from_row` or `fold` out of `decision_axes` would let a caller write the bare
/// word and vanish from it.
#[test]
fn decision_axes_is_never_imported_by_name() {
    let tree = scan_tree();
    let word = regex::Regex::new(r"\b(from_row|fold)\b").unwrap();
    let mut offenders = Vec::new();
    for (rel, lines) in &tree.production {
        // A `use` of `decision_axes` may span lines; follow it to its `;`.
        let mut in_axes_use = false;
        for (ln, text) in lines {
            let t = text.trim_start();
            if (t.starts_with("use ") || t.starts_with("pub use ")) && t.contains("decision_axes") {
                in_axes_use = true;
            }
            if in_axes_use && word.is_match(t) {
                offenders.push(format!("{rel}:{ln}: {}", t.trim()));
            }
            if in_axes_use && t.contains(';') {
                in_axes_use = false;
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "`decision_axes::from_row` / `fold` imported by name: {offenders:?}"
    );
}

/// Design §2.1: the readers that ask "what model is this chat bound to" (the
/// save checks, DR-16's raise comparators, the workflow seed) read the inner
/// language model (`bound_model_tier`, `bound_model_provider`), never the slot.
/// The slot's `tier()` is the folded, decision-lowered capability, and a raise
/// comparator that read it called a Versa-to-Versa move on a lowered chat a
/// raise from Public.
///
/// Each entry is a file (whole production text) or one function in it, and
/// names a token its body must still contain, so the scan cannot pass by
/// failing to find what it scans.
#[test]
fn save_checks_and_raise_comparators_read_the_bound_model_tier() {
    const SITES: &[(&str, Option<&str>, &str)] = &[
        (
            "crates/biorouter-server/src/routes/decisions.rs",
            None,
            "bound_model_provider()",
        ),
        (
            "crates/biorouter/src/workflow/runtime.rs",
            None,
            "bound_model_provider()",
        ),
        (
            "crates/biorouter/src/decision/save.rs",
            None,
            "bound_model_tier",
        ),
        (
            "crates/biorouter-server/src/routes/agent.rs",
            Some("async fn update_agent_provider("),
            "bound_model_tier()",
        ),
        (
            "crates/biorouter-server/src/routes/apps.rs",
            Some("async fn established_capability("),
            "bound_model_provider()",
        ),
        (
            "crates/biorouter/src/agents/workspace_extension.rs",
            Some("async fn preflight_set_tools("),
            "target_bound_model_tier(",
        ),
    ];
    let tree = scan_tree();
    let mut offenders = Vec::new();
    for (rel, function, witness) in SITES {
        let lines = tree
            .production
            .get(*rel)
            .unwrap_or_else(|| panic!("{rel} is not in the production tree"));
        let text: String = lines
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let body = match function {
            None => text,
            Some(signature) => {
                let start = text
                    .find(signature)
                    .unwrap_or_else(|| panic!("{rel} no longer defines `{signature}`"));
                let open = start
                    + text
                        .get(start..)
                        .and_then(|rest| rest.find('{'))
                        .unwrap_or_else(|| panic!("`{signature}` has no body"));
                let mut depth = 0usize;
                let mut end = text.len();
                for (at, ch) in text.get(open..).unwrap_or_default().char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = open + at + 1;
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                text.get(start..end)
                    .unwrap_or_else(|| panic!("`{signature}` is not on character boundaries"))
                    .to_string()
            }
        };
        let squeezed: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            squeezed.contains(witness),
            "{rel} {function:?} no longer reads `{witness}`; the scan would pass vacuously"
        );
        // `.provider()` is the slot; `bound_model_provider()` does not match,
        // because its dot precedes `bound_model_`.
        let hits = squeezed.matches(".provider()").count();
        if hits > 0 {
            offenders.push(format!(
                "{rel} {function:?}: {hits} read(s) of `.provider()`"
            ));
        }
    }
    assert!(
        offenders.is_empty(),
        "a save check or raise comparator reads the folded slot instead of the bound language \
         model (`Agent::bound_model_tier` / `bound_model_provider`): {offenders:?}"
    );
}

// ---------------------------------------------------------------------------
// DR-33: the provider a turn's requests run on never decides a tier.
// ---------------------------------------------------------------------------

/// What a function on a turn's path does with the provider the turn's
/// requests run on, when it hands that provider to something else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HandOff {
    /// Another holder in [`TURN_PROVIDER_HOLDERS`]; the census follows it.
    Follow,
    /// The provider keeps going under a name this holder already follows,
    /// or is the request itself. Nothing reads a tier there.
    Carry,
    /// Something that reads the provider's tier, affiliation or decision
    /// model's cause, allowed only for the reason written beside it.
    Reader,
}

/// One function that holds the turn's request provider.
struct TurnProviderHolder {
    file: &'static str,
    /// The text that opens the function, unique in the file's production code.
    signature: &'static str,
    /// The names the provider has inside it.
    names: &'static [&'static str],
    /// Every callee the provider is handed to, measured and declared equal.
    hand_offs: &'static [(&'static str, HandOff, &'static str)],
}

/// Where the provider the reply loop sends requests on goes, from
/// `reply_internal` down.
///
/// At a non-Normal reasoning effort `Agent::provider_with_effort` rebuilds that
/// provider around the language model ALONE, without the decision-bound
/// wrapper `install_bound_provider` puts in the slot, so its `tier()`,
/// `affiliation()` and `decision_lowered_by()` are the model's and not the
/// chat's. A gate that judged with it would let a private model with a public
/// decision model (DR-33's Versa plus Baseten) reach private data at Quick or
/// Deep effort and not at Normal. Every gate reads the slot instead; this keeps
/// it that way. A new hand-off has to be classified here before it lands.
const TURN_PROVIDER_HOLDERS: &[TurnProviderHolder] = &[
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/agent.rs",
        signature: "async fn reply_internal(",
        names: &["turn_provider", "reply_provider", "iteration_provider"],
        hand_offs: &[
            (
                "match",
                HandOff::Carry,
                "binds `reply_provider`: the effort-rebuilt provider when there is one, else the \
                 slot's own (`self.provider()`)",
            ),
            (
                "let iteration_provider",
                HandOff::Carry,
                "one request's pin of `reply_provider`",
            ),
            ("prepare_reply_context", HandOff::Follow, ""),
            ("prepare_tools_and_prompt_for_provider", HandOff::Follow, ""),
            ("check_crew_before_request", HandOff::Follow, ""),
            ("issue_tool_bridge", HandOff::Follow, ""),
            ("persist_composite_provider_state", HandOff::Follow, ""),
            ("stream_response_from_provider", HandOff::Follow, ""),
        ],
    },
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/agent.rs",
        signature: "async fn prepare_reply_context(",
        names: &["provider"],
        hand_offs: &[("prepare_tools_and_prompt_for_provider", HandOff::Follow, "")],
    },
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/reply_parts.rs",
        signature: "pub(super) async fn prepare_tools_and_prompt_for_provider(",
        names: &["provider"],
        hand_offs: &[
            ("prepare_model_tool_surface", HandOff::Follow, ""),
            (
                "runs_on_this_machine",
                HandOff::Reader,
                "asks where inference runs, to pick the system prompt's variant and clock: a \
                 property of the model the request runs on, which no privacy decision reads",
            ),
        ],
    },
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/reply_parts.rs",
        signature: "async fn prepare_model_tool_surface(",
        names: &["provider"],
        hand_offs: &[("prepare_coding_agent_bridge_plan", HandOff::Follow, "")],
    },
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/agent.rs",
        signature: "pub(super) async fn prepare_coding_agent_bridge_plan(",
        names: &["iteration_provider", "judged"],
        hand_offs: &[
            (
                "match arm",
                HandOff::Carry,
                "`judged` is the slot's provider whenever that one is Public, and the request's \
                 only while the slot is Private, where no decision model lowered the chat",
            ),
            (
                "Mutex::new",
                HandOff::Reader,
                "the bridge's capability, sampled once off `judged`: the lesser of the chat's \
                 (the slot's) and the request's provider's, so never wider than either",
            ),
        ],
    },
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/agent.rs",
        signature: "async fn check_crew_before_request(",
        names: &["request_provider"],
        hand_offs: &[(
            "check_provider_dispatch",
            HandOff::Reader,
            "Crew's binding and fresh workspace admission are about the model the request runs \
             on. The tier half of it reads the model's own tier at a non-Normal effort, so the \
             same function then asks Crew about the slot's provider (`check_provider_use` on \
             `bound_provider_unchecked()`), which a Normal-effort request is already checked on",
        )],
    },
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/agent.rs",
        signature: "async fn issue_tool_bridge(",
        names: &["iteration_provider"],
        hand_offs: &[
            ("coding_agent_bridge_subagent_context", HandOff::Follow, ""),
            ("chat_bridge_dispatch", HandOff::Follow, ""),
        ],
    },
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/agent.rs",
        signature: "async fn coding_agent_bridge_subagent_context(",
        names: &["iteration_provider"],
        hand_offs: &[(
            "TaskConfig::new",
            HandOff::Reader,
            "a bridged child's spawn reads the parent's effective tier off it. Only a provider \
             that needs a bridge gets here, and every one is Public, so a fold cannot change \
             its tier, affiliation or cause \
             (`every_bridge_provider_is_public_so_a_fold_cannot_change_it`)",
        )],
    },
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/agent.rs",
        signature: "fn chat_bridge_dispatch(",
        names: &["iteration_provider"],
        hand_offs: &[(
            "field ingest_provider",
            HandOff::Reader,
            "a bridged knowledge ingest runs on it and ratchets the base by its tier. Only a \
             bridge provider gets here, and every one is Public \
             (`every_bridge_provider_is_public_so_a_fold_cannot_change_it`)",
        )],
    },
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/agent.rs",
        signature: "async fn persist_composite_provider_state(",
        names: &["provider"],
        hand_offs: &[],
    },
    TurnProviderHolder {
        file: "crates/biorouter/src/agents/reply_parts.rs",
        signature: "pub(crate) async fn stream_response_from_provider(",
        names: &["provider"],
        hand_offs: &[(
            "let provider",
            HandOff::Carry,
            "re-binds the same provider under the same name to move it into the stream",
        )],
    },
];

/// Methods that read what DR-33 folds, and which nothing may ask of the
/// turn's request provider.
const FOLDED_READS: &[&str] = &["tier", "affiliation", "decision_lowered_by"];

/// Calls and methods that hand the value straight through; the walk looks at
/// whatever receives their result instead.
const PASS_THROUGH_CALLS: &[&str] = &["Arc::clone", "clone", "Some", ""];
const PASS_THROUGH_METHODS: &[&str] = &["as_ref", "clone"];

/// How one occurrence of a holder's name is used.
#[derive(Debug, PartialEq, Eq)]
enum ProviderUse {
    /// A parameter, a `let` or a field pattern: where the name is bound.
    Bound,
    /// `name.method(..)` for a method that reads no folded value.
    Method,
    /// `name.tier()` and the other two, possibly after `.as_ref()`.
    FoldedRead(String),
    /// The value is handed to this callee.
    HandedTo(String),
}

fn skip_space(c: &[char], mut i: usize) -> usize {
    while i < c.len() && c[i].is_whitespace() {
        i += 1;
    }
    i
}

fn closing_paren(c: &[char], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, ch) in c.iter().enumerate().skip(open) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    c.len()
}

/// `Type::name` when the path names a type's function, else the last segment.
fn callee_name(path: &str) -> String {
    let segments: Vec<&str> = path.split("::").filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        [] => String::new(),
        [only] => (*only).to_string(),
        [.., owner, last] if owner.starts_with(char::is_uppercase) && *owner != "Self" => {
            format!("{owner}::{last}")
        }
        [.., last] => (*last).to_string(),
    }
}

/// Who receives the value whose expression starts at `start`.
fn receiver_of(c: &[char], start: usize) -> String {
    let mut pos = start;
    loop {
        let mut depth = 0usize;
        let mut i = pos;
        let mut open = None;
        while i > 0 {
            i -= 1;
            let ch = c[i];
            if depth == 0 {
                match ch {
                    '(' => {
                        open = Some(i);
                        break;
                    }
                    '{' | '[' | '}' | ';' => break,
                    ')' | ']' => depth += 1,
                    _ => {}
                }
            } else {
                match ch {
                    ')' | ']' | '}' => depth += 1,
                    '(' | '[' | '{' => depth -= 1,
                    _ => {}
                }
            }
        }
        let Some(open) = open else {
            let before: String = c[..pos].iter().collect();
            let before = before.trim_end().trim_end_matches('&').trim_end();
            let last_word = |text: &str| {
                text.trim_end()
                    .rsplit(|ch: char| !is_word(ch))
                    .next()
                    .unwrap_or_default()
                    .to_string()
            };
            return if before.ends_with("=>") {
                "match arm".to_string()
            } else if before.ends_with('=') {
                format!("let {}", last_word(before.trim_end_matches('=')))
            } else if before.ends_with(':') && !before.ends_with("::") {
                format!("field {}", last_word(before.trim_end_matches(':')))
            } else if last_word(before) == "match" {
                "match".to_string()
            } else {
                "statement".to_string()
            };
        };
        let mut j = open;
        while j > 0 && c[j - 1].is_whitespace() {
            j -= 1;
        }
        let end = j;
        while j > 0 && (is_word(c[j - 1]) || c[j - 1] == ':' || c[j - 1] == '!') {
            j -= 1;
        }
        let path: String = c[j..end].iter().collect();
        let callee = callee_name(&path);
        if PASS_THROUGH_CALLS.contains(&callee.as_str()) {
            pos = j;
            continue;
        }
        return callee;
    }
}

/// Classify the occurrence of a name spanning `start..end`.
fn provider_use(c: &[char], start: usize, end: usize) -> ProviderUse {
    let mut i = skip_space(c, end);
    if c.get(i) == Some(&':') && c.get(i + 1) != Some(&':') {
        return ProviderUse::Bound;
    }
    if c.get(i) == Some(&'=') && !matches!(c.get(i + 1), Some('=') | Some('>')) {
        return ProviderUse::Bound;
    }
    while c.get(i) == Some(&'.') {
        let at = skip_space(c, i + 1);
        let mut k = at;
        while k < c.len() && is_word(c[k]) {
            k += 1;
        }
        let method: String = c[at..k].iter().collect();
        let paren = skip_space(c, k);
        if c.get(paren) != Some(&'(') {
            // `.await`, or a field of the provider: no folded value either way.
            return ProviderUse::Method;
        }
        if FOLDED_READS.contains(&method.as_str()) {
            return ProviderUse::FoldedRead(method);
        }
        if !PASS_THROUGH_METHODS.contains(&method.as_str()) {
            return ProviderUse::Method;
        }
        i = skip_space(c, closing_paren(c, paren) + 1);
    }
    ProviderUse::HandedTo(receiver_of(c, start))
}

/// Every use of `names` in `body`, skipping a name that is a field or a path
/// segment of something else (`self.provider`, `usage.provider`).
fn provider_uses(body: &str, names: &[&str]) -> Vec<(String, ProviderUse)> {
    let c: Vec<char> = body.chars().collect();
    let mut out = Vec::new();
    for name in names {
        let pat: Vec<char> = name.chars().collect();
        let mut i = 0usize;
        while i + pat.len() <= c.len() {
            let whole = c[i..i + pat.len()] == pat[..]
                && (i == 0 || !is_word(c[i - 1]))
                && (i + pat.len() == c.len() || !is_word(c[i + pat.len()]));
            if !whole {
                i += 1;
                continue;
            }
            let of_something_else = i > 0 && (c[i - 1] == '.' || c[i - 1] == ':');
            if !of_something_else {
                out.push((name.to_string(), provider_use(&c, i, i + pat.len())));
            }
            i += pat.len();
        }
    }
    out
}

/// The production text of the function `signature` opens, from the signature
/// to its closing brace.
fn function_text(tree: &Tree, rel: &str, signature: &str) -> String {
    let text: String = tree
        .production
        .get(rel)
        .unwrap_or_else(|| panic!("{rel} is not in the production tree"))
        .iter()
        .map(|(_, t)| t.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        text.matches(signature).count(),
        1,
        "`{signature}` must open exactly one function in {rel}"
    );
    let start = text.find(signature).unwrap();
    let open = start
        + text
            .get(start..)
            .and_then(|rest| rest.find('{'))
            .expect("a function body");
    let mut depth = 0usize;
    for (at, ch) in text.get(open..).unwrap_or_default().char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return text
                        .get(start..=open + at)
                        .expect("a function on character boundaries")
                        .to_string();
                }
            }
            _ => {}
        }
    }
    panic!("`{signature}` in {rel} never closes");
}

/// The instrument, on code it must catch. A census that cannot fail passes on
/// any tree.
#[test]
fn the_turn_provider_census_catches_what_it_is_for() {
    let uses = |src: &str| provider_uses(src, &["iteration_provider"]);
    assert_eq!(
        uses("let t = iteration_provider.tier();"),
        vec![(
            "iteration_provider".into(),
            ProviderUse::FoldedRead("tier".into())
        )]
    );
    assert_eq!(
        uses("x(iteration_provider\n    .as_ref()\n    .affiliation())"),
        vec![(
            "iteration_provider".into(),
            ProviderUse::FoldedRead("affiliation".into())
        )]
    );
    assert_eq!(
        uses("let s: SharedProvider = Arc::new(Mutex::new(Some(Arc::clone(iteration_provider))));"),
        vec![(
            "iteration_provider".into(),
            ProviderUse::HandedTo("Mutex::new".into())
        )],
        "the throwaway slot the bridge plan used to sample"
    );
    assert_eq!(
        uses("crew.check_provider_dispatch(id, iteration_provider.as_ref()).await?;"),
        vec![(
            "iteration_provider".into(),
            ProviderUse::HandedTo("check_provider_dispatch".into())
        )]
    );
    assert_eq!(
        uses("f(iteration_provider: &Arc<dyn Provider>) { iteration_provider.get_name(); }"),
        vec![
            ("iteration_provider".into(), ProviderUse::Bound),
            ("iteration_provider".into(), ProviderUse::Method)
        ]
    );
    assert_eq!(
        uses("let p = self.iteration_provider.tier();"),
        vec![],
        "a field of something else is not the name"
    );
}

/// DR-33: nothing on a turn's path reads `tier()`, `affiliation()` or
/// `decision_lowered_by()`, or samples a `CallCapability`, off the provider the
/// turn's requests run on, except the readers [`TURN_PROVIDER_HOLDERS`] names
/// with a reason. See that table for why.
#[test]
fn the_turn_provider_never_decides_a_tier() {
    let tree = scan_tree();
    let holders: Vec<String> = TURN_PROVIDER_HOLDERS
        .iter()
        .map(|holder| {
            let name = holder.signature.split("fn ").nth(1).unwrap_or_default();
            name.trim_end_matches('(').to_string()
        })
        .collect();
    let mut failures = Vec::new();
    for holder in TURN_PROVIDER_HOLDERS {
        let body = function_text(&tree, holder.file, holder.signature);
        let uses = provider_uses(&body, holder.names);
        assert!(
            uses.iter().any(|(_, kind)| *kind != ProviderUse::Bound),
            "`{}` in {} no longer uses {:?}; the census would pass on nothing",
            holder.signature,
            holder.file,
            holder.names
        );
        let mut measured = std::collections::BTreeSet::new();
        for (name, kind) in &uses {
            match kind {
                ProviderUse::FoldedRead(method) => failures.push(format!(
                    "`{}` ({}) reads `{name}.{method}()`: at a non-Normal effort that is the \
                     model's value, not the chat's. Read the slot (`self.provider`)",
                    holder.signature, holder.file
                )),
                ProviderUse::HandedTo(callee) => {
                    measured.insert(callee.clone());
                }
                ProviderUse::Bound | ProviderUse::Method => {}
            }
        }
        // A capability sampled straight off one of the names, with no slot
        // in between, is the same read.
        let squeezed: String = body.chars().filter(|ch| !ch.is_whitespace()).collect();
        for name in holder.names {
            for spelling in [format!("sample(&{name})"), format!("sample({name})")] {
                if squeezed.contains(&spelling) {
                    failures.push(format!(
                        "`{}` ({}) samples a capability off `{name}`",
                        holder.signature, holder.file
                    ));
                }
            }
        }
        let declared: std::collections::BTreeSet<String> = holder
            .hand_offs
            .iter()
            .map(|(callee, ..)| callee.to_string())
            .collect();
        if measured != declared {
            failures.push(format!(
                "`{}` ({}) hands the turn's provider to {measured:?}, but the census declares \
                 {declared:?}. Classify each new receiver in TURN_PROVIDER_HOLDERS: follow it \
                 as a holder, or say why what it reads is not a tier decision",
                holder.signature, holder.file
            ));
        }
        for (callee, kind, reason) in holder.hand_offs {
            let is_holder = holders.iter().any(|h| h.as_str() == *callee);
            match kind {
                HandOff::Follow if !is_holder => failures.push(format!(
                    "`{callee}` is followed but is not in TURN_PROVIDER_HOLDERS"
                )),
                HandOff::Carry | HandOff::Reader if is_holder => failures.push(format!(
                    "`{callee}` is a holder, so it must be followed, not excused"
                )),
                HandOff::Carry | HandOff::Reader if reason.trim().is_empty() => failures.push(
                    format!("`{callee}` in `{}` has no reason", holder.signature),
                ),
                _ => {}
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------------
// The scanner.
// ---------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Blank out comment bodies, string bodies and char literals, **preserving line
/// numbering and every brace that is really a brace**.
///
/// Without this the census counts a guard named in a doc comment as a caller —
/// which is the grep gate this campaign already shipped once — and a `}` inside
/// a string literal ends a `#[cfg(test)] mod tests` early, dragging test code
/// back into the production view. Both failures are silent and both make the
/// gate greener than the tree.
fn strip_lexical(src: &str) -> Vec<String> {
    #[derive(PartialEq)]
    enum S {
        Code,
        Line,
        Block(usize),
        Str,
        Raw(usize),
        Char,
    }
    let b: Vec<char> = src.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut st = S::Code;
    let mut escape = false;
    let mut i = 0usize;
    while i < b.len() {
        let ch = b[i];
        if escape && ch != '\n' {
            escape = false;
            line.push(' ');
            i += 1;
            continue;
        }
        escape = false;
        if ch == '\n' {
            out.push(std::mem::take(&mut line));
            if st == S::Line {
                st = S::Code;
            }
            i += 1;
            continue;
        }
        let two: String = b[i..(i + 2).min(b.len())].iter().collect();
        match st {
            S::Line => {
                line.push(' ');
                i += 1;
            }
            S::Block(depth) => {
                if two == "*/" {
                    st = if depth == 1 {
                        S::Code
                    } else {
                        S::Block(depth - 1)
                    };
                    line.push_str("  ");
                    i += 2;
                } else if two == "/*" {
                    st = S::Block(depth + 1);
                    line.push_str("  ");
                    i += 2;
                } else {
                    line.push(' ');
                    i += 1;
                }
            }
            S::Str => {
                if ch == '\\' {
                    escape = true;
                    line.push(' ');
                    i += 1;
                } else if ch == '"' {
                    st = S::Code;
                    line.push('"');
                    i += 1;
                } else {
                    line.push(' ');
                    i += 1;
                }
            }
            S::Raw(hashes) => {
                let closes = ch == '"'
                    && b[i + 1..(i + 1 + hashes).min(b.len())]
                        .iter()
                        .filter(|c| **c == '#')
                        .count()
                        == hashes;
                if closes {
                    st = S::Code;
                    for _ in 0..=hashes {
                        line.push(' ');
                    }
                    i += 1 + hashes;
                } else {
                    line.push(' ');
                    i += 1;
                }
            }
            S::Char => {
                if ch == '\\' {
                    escape = true;
                    line.push(' ');
                    i += 1;
                } else if ch == '\'' {
                    st = S::Code;
                    line.push('\'');
                    i += 1;
                } else {
                    line.push(' ');
                    i += 1;
                }
            }
            S::Code => {
                if two == "//" {
                    st = S::Line;
                    line.push_str("  ");
                    i += 2;
                } else if two == "/*" {
                    st = S::Block(1);
                    line.push_str("  ");
                    i += 2;
                } else if ch == 'r' && i + 1 < b.len() && (b[i + 1] == '"' || b[i + 1] == '#') {
                    // `r"…"` or `r#"…"#`; anything else starting with `r` is an
                    // identifier and must fall through untouched.
                    let mut j = i + 1;
                    let mut hashes = 0usize;
                    while j < b.len() && b[j] == '#' {
                        hashes += 1;
                        j += 1;
                    }
                    if j < b.len() && b[j] == '"' {
                        st = S::Raw(hashes);
                        for _ in i..=j {
                            line.push(' ');
                        }
                        i = j + 1;
                    } else {
                        line.push(ch);
                        i += 1;
                    }
                } else if ch == '"' {
                    st = S::Str;
                    line.push('"');
                    i += 1;
                } else if ch == '\'' {
                    // A char literal is `'x'` or `'\x'`; anything else is a
                    // lifetime and must stay in the code text.
                    let is_char = (i + 2 < b.len() && b[i + 1] != '\\' && b[i + 2] == '\'')
                        || (i + 3 < b.len() && b[i + 1] == '\\' && b[i + 3] == '\'');
                    if is_char {
                        st = S::Char;
                        line.push(' ');
                        i += 1;
                    } else {
                        line.push(ch);
                        i += 1;
                    }
                } else {
                    line.push(ch);
                    i += 1;
                }
            }
        }
    }
    out.push(line);
    out
}

/// Lines belonging to a `#[cfg(test)]` item, by brace matching over code text.
fn test_region(code: &[String]) -> Vec<bool> {
    let mut skip = vec![false; code.len()];
    let mut i = 0usize;
    while i < code.len() {
        let t = code[i].trim_start();
        if t.starts_with("#[cfg(test)]") || t.starts_with("#[cfg(all(test") {
            let mut j = i;
            let mut depth: i64 = 0;
            let mut opened = false;
            while j < code.len() {
                for ch in code[j].chars() {
                    if ch == '{' {
                        depth += 1;
                        opened = true;
                    } else if ch == '}' {
                        depth -= 1;
                    }
                }
                if opened && depth <= 0 {
                    break;
                }
                // `#[cfg(test)] use …;` — an item with no block.
                if !opened && code[j].trim_end().ends_with(';') {
                    break;
                }
                j += 1;
            }
            for entry in skip.iter_mut().take((j + 1).min(code.len())).skip(i) {
                *entry = true;
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }
    skip
}

/// Is this a production source file? `crates/*/tests/**` are integration tests;
/// `tests.rs`, `*_test.rs`, `*_tests.rs` and `src/**/tests/**` are test modules
/// that happen to live under `src/`.
fn is_production_file(rel: &str) -> bool {
    let Some((_, tail)) = rel.split_once("/src/") else {
        return false;
    };
    if tail.starts_with("tests/") || tail.contains("/tests/") {
        return false;
    }
    let base = tail.rsplit('/').next().unwrap_or(tail);
    !(base == "tests.rs"
        || base.ends_with("_tests.rs")
        || base.ends_with("_test.rs")
        || base.starts_with("tests_"))
}

struct Tree {
    /// repo-relative path → production code lines (1-based line number, text)
    production: BTreeMap<String, Vec<(usize, String)>>,
    /// Everything the production view excludes: test regions, test files. Used
    /// only by the controls.
    excluded_text: String,
}

fn scan_tree() -> Tree {
    let root = repo_root();
    let mut production = BTreeMap::new();
    let mut excluded_text = String::new();
    for entry in walkdir::WalkDir::new(root.join("crates")) {
        let entry = entry.expect("the census must not silently skip an unreadable directory");
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let rel = p
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let src = std::fs::read_to_string(p)
            .unwrap_or_else(|e| panic!("the census could not read {rel}: {e}"));
        if !is_production_file(&rel) {
            excluded_text.push_str(&src);
            continue;
        }
        let code = strip_lexical(&src);
        assert_eq!(
            code.len(),
            // `split`, not `lines`: a trailing newline ends a line here, and the
            // census reports 1-based line numbers that must match the file.
            src.split('\n').count(),
            "the lexer lost or invented lines in {rel}; every line number the census reports \
             would be wrong"
        );
        let skip = test_region(&code);
        let mut lines = Vec::new();
        for (idx, text) in code.iter().enumerate() {
            if skip[idx] {
                excluded_text.push_str(text);
                excluded_text.push('\n');
            } else {
                lines.push((idx + 1, text.clone()));
            }
        }
        production.insert(rel, lines);
    }
    Tree {
        production,
        excluded_text,
    }
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Occurrences of `ident` as a whole identifier in one line of code text, split
/// into calls (`ident(`, allowing spaces) and bare references.
fn occurrences(line: &str, ident: &str) -> (usize, usize) {
    let chars: Vec<char> = line.chars().collect();
    let pat: Vec<char> = ident.chars().collect();
    let (mut calls, mut refs) = (0usize, 0usize);
    let mut i = 0usize;
    while i + pat.len() <= chars.len() {
        if chars[i..i + pat.len()] == pat[..]
            && (i == 0 || !is_word(chars[i - 1]))
            && (i + pat.len() == chars.len() || !is_word(chars[i + pat.len()]))
        {
            let mut j = i + pat.len();
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            if chars.get(j) == Some(&'(') {
                calls += 1;
            } else {
                refs += 1;
            }
            i += pat.len();
        } else {
            i += 1;
        }
    }
    (calls, refs)
}

/// Does this line define `fn <ident>`? The definition is not a call site.
fn defines(line: &str, ident: &str) -> bool {
    let t = line.trim_start();
    let t = t.strip_prefix("pub ").unwrap_or(t);
    let t = match t.split_once(") ") {
        // `pub(crate) fn`, `pub(in …) fn`
        Some((_, rest)) if t.starts_with("pub(") => rest,
        _ => t,
    };
    let t = t.strip_prefix("async ").unwrap_or(t);
    let t = t.strip_prefix("const ").unwrap_or(t);
    let t = t.strip_prefix("unsafe ").unwrap_or(t);
    let t = t.strip_prefix("async ").unwrap_or(t);
    t.strip_prefix("fn ")
        .map(|rest| rest.trim_start().starts_with(ident))
        .unwrap_or(false)
        && t.strip_prefix("fn ").is_some_and(|rest| {
            let rest = rest.trim_start();
            rest.len() == ident.len() || !is_word(rest.chars().nth(ident.len()).unwrap_or(' '))
        })
}

/// Measure one guard across the production tree: file → counts.
fn measure(tree: &Tree, ident: &str, defined_in: &str) -> BTreeMap<String, Counts> {
    let mut out: BTreeMap<String, Counts> = BTreeMap::new();
    for (rel, lines) in &tree.production {
        // A `use` statement may span lines; track it so every line of one counts
        // as an import.
        let mut in_use = false;
        for (_, text) in lines {
            let t = text.trim_start();
            let t_nopub = t.strip_prefix("pub ").unwrap_or(t);
            if !in_use && (t_nopub.starts_with("use ") || t_nopub.starts_with("use\t")) {
                in_use = true;
            }
            let this_is_use = in_use;
            if in_use && text.contains(';') {
                in_use = false;
            }
            let (mut calls, refs) = occurrences(text, ident);
            if rel == defined_in && defines(text, ident) {
                // The definition line itself: `fn may_read(` reads as a call.
                calls = calls.saturating_sub(1);
            }
            if calls + refs == 0 {
                continue;
            }
            let e = out.entry(rel.clone()).or_insert(c(0, 0, 0));
            if this_is_use {
                e.imports += calls + refs;
            } else {
                e.calls += calls;
                e.refs += refs;
            }
        }
    }
    out.retain(|_, v| *v != c(0, 0, 0));
    out
}

/// Production `pub fn` names defined in `file`.
fn public_fns(tree: &Tree, file: &str) -> Vec<String> {
    let lines = tree
        .production
        .get(file)
        .unwrap_or_else(|| panic!("{file} is not in the production scan; did it move?"));
    let mut out = Vec::new();
    for (_, text) in lines {
        let t = text.trim_start();
        if !t.starts_with("pub ") && !t.starts_with("pub(") {
            continue;
        }
        let after = match t.split_once(" fn ") {
            Some((_, rest)) => rest,
            None => continue,
        };
        let name: String = after
            .trim_start()
            .chars()
            .take_while(|ch| is_word(*ch))
            .collect();
        if !name.is_empty() {
            out.push(name);
        }
    }
    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------------------
// The controls. A completeness test never seen to fail is not known to work,
// and a walk that reads nothing reports the same clean tree as a clean tree.
// ---------------------------------------------------------------------------

#[test]
fn the_scan_really_separates_production_from_tests() {
    let tree = scan_tree();
    assert!(
        tree.production.len() >= 400,
        "only {} production .rs files were scanned (520 when this census was written). A \
         broken walk reports the same empty set as a clean tree.",
        tree.production.len()
    );
    // One direction: spellings only tests use must be absent from the production
    // view. `CallCapability::for_test*` is the marker — production capabilities
    // come from `sample` or `public_enforced` — and `#[test]` is the blunt one.
    for marker in [
        "for_test_restricted",
        "CallCapability::for_test",
        "#[test]",
        "#[tokio::test]",
    ] {
        let hits: Vec<String> = tree
            .production
            .iter()
            .flat_map(|(rel, lines)| {
                lines
                    .iter()
                    .filter(|(_, t)| t.contains(marker))
                    .map(move |(ln, _)| format!("{rel}:{ln}"))
            })
            .collect();
        assert!(
            hits.is_empty(),
            "`{marker}` is test-only, and the census counted it as production at {hits:?}. \
             The `#[cfg(test)]` split is leaking, so every 'has a live caller' verdict below \
             may be a test caller wearing a production hat."
        );
    }
    // The other direction: the test text must be non-empty and must contain what
    // was removed. Otherwise "no production caller" could mean "no file read".
    assert!(
        tree.excluded_text.contains("for_test_restricted"),
        "the excluded text does not contain a spelling that certainly exists in the tree's \
         tests, so the split removed the wrong half"
    );
}

#[test]
fn the_registry_covers_every_guard_module() {
    let tree = scan_tree();
    let mut missing: Vec<String> = Vec::new();
    for module in COMPLETE_MODULES {
        for name in public_fns(&tree, module) {
            if !REGISTRY
                .iter()
                .any(|g| g.ident == name && g.defined_in == *module)
            {
                missing.push(format!("{module}::{name}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "a public function was added to a guard module and no census row classifies it: \
         {missing:?}\n\n\
         Every public function in these modules is a reach decision, so a new one is either \
         wired (add a `Status::Wired` row naming its callers) or it is the next `may_read`, \
         a correct, tested predicate that nothing calls. Deciding which is the point of the \
         build being red."
    );
}

#[test]
fn every_privacy_guard_has_a_live_caller_or_a_reason() {
    let tree = scan_tree();

    // ---- 1. The measured map must equal the declared one, exactly.
    let mut drift: Vec<String> = Vec::new();
    for guard in REGISTRY {
        let measured = measure(&tree, guard.ident, guard.defined_in);
        let declared: BTreeMap<String, Counts> = guard
            .sites
            .iter()
            .map(|s| (s.file.to_string(), s.counts))
            .collect();
        assert_eq!(
            guard.sites.len(),
            declared.len(),
            "`{}` declares the same file twice; merge the rows",
            guard.ident
        );
        if measured != declared {
            let described: String = guard
                .sites
                .iter()
                .map(|s| {
                    format!(
                        "      {} {:?} {:?}\n        {}\n",
                        s.file, s.counts, s.kind, s.what
                    )
                })
                .collect();
            drift.push(format!(
                "  {} ({})\n    declared:\n{described}    measured: {measured:?}\n",
                guard.ident, guard.defined_in
            ));
        }
    }
    assert!(
        drift.is_empty(),
        "the places a privacy guard is named have changed:\n\n{}\n\
         If a call was ADDED, add or grow the site row saying what it is. If a call was \
         REMOVED, that is the defect this census exists to catch: a guard loses its last \
         caller and every behavioural test still passes, because the unit under test is still \
         correct.",
        drift.join("\n")
    );

    // ---- 2. The wiring verdicts, over REAL sites only.
    let mut failures: Vec<String> = Vec::new();
    for guard in REGISTRY {
        let real_outside: usize = guard
            .sites
            .iter()
            .filter(|s| s.kind == SiteKind::Guard && s.file != guard.defined_in)
            .map(|s| s.counts.calls + s.counts.refs)
            .sum();
        let real_inside: usize = guard
            .sites
            .iter()
            .filter(|s| s.kind == SiteKind::Guard && s.file == guard.defined_in)
            .map(|s| s.counts.calls + s.counts.refs)
            .sum();
        match guard.status {
            Status::Wired => {
                if real_outside == 0 {
                    failures.push(format!(
                        "  {} ({}) is declared WIRED and has no live caller outside its own \
                         file.\n    It decides: {}\n    This is the release blocker recurring: \
                         the predicate is still correct, its unit tests still pass, and nothing \
                         asks it.",
                        guard.ident, guard.defined_in, guard.decides
                    ));
                }
            }
            Status::WiredThrough(parent) => {
                if real_inside == 0 {
                    failures.push(format!(
                        "  {} ({}) is declared reached through `{parent}` and has no call site \
                         in its own file either.",
                        guard.ident, guard.defined_in
                    ));
                }
                match REGISTRY.iter().find(|g| g.ident == parent) {
                    None => failures.push(format!(
                        "  {} names `{parent}` as the guard that reaches it, and `{parent}` has \
                         no row. A chain that leaves the census is not a chain.",
                        guard.ident
                    )),
                    Some(p) if matches!(p.status, Status::Unwired(_)) => failures.push(format!(
                        "  {} is reached only through `{parent}`, which is itself UNWIRED. The \
                         whole chain is dead; reclassify it.",
                        guard.ident
                    )),
                    Some(_) => {}
                }
            }
            Status::Unwired(reason) => {
                if real_outside > 0 {
                    failures.push(format!(
                        "  {} ({}) is filed as UNWIRED but now has {real_outside} live \
                         caller(s) outside its own file.\n    The filed reason was: \
                         {reason}\n    Good news is still a failure here: move the row to \
                         `Status::Wired` and name the callers, so the next reader is not told a \
                         closed gap is open.",
                        guard.ident, guard.defined_in
                    ));
                }
                // A guard with no production caller AND no test caller is not a
                // gap, it is dead code — or the scan is not reading files.
                if !tree.excluded_text.contains(guard.ident) {
                    failures.push(format!(
                        "  {} has no production caller and is not mentioned anywhere in the \
                         tree's test text either. Either it is dead code, or this census is \
                         reading nothing, and those two look identical from a green build.",
                        guard.ident
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "the privacy guard wiring census failed:\n\n{}\n\n\
         Read the module doc before relaxing anything here. A guard with no caller is the \
         defect this campaign has shipped five times; it is invisible to unit tests and to \
         code review, and this is the only thing in the tree that sees it.",
        failures.join("\n")
    );
}

/// The name of the `fn` item a line of code text opens, if it opens one.
fn opened_fn(line: &str) -> Option<String> {
    let t = line.trim_start();
    let after = match t.strip_prefix("fn ") {
        Some(rest) => rest,
        None => t.split_once(" fn ")?.1,
    };
    let name: String = after
        .trim_start()
        .chars()
        .take_while(|ch| is_word(*ch))
        .collect();
    (!name.is_empty()).then_some(name)
}

/// **Private standing is built in two places** (DD-29, DD-30, DD-77; PR-16).
///
/// `bias_set` lets private terms shape a transcript only for a caller whose `Standing`
/// is private, and `Standing`'s field is private, so `Standing::from_checked` is the one
/// way to build such a value. This pins every production call of it to the two functions
/// that check a caller first: `HttpCaller::dictation_standing` (the user action proof or
/// a stated private capability, never a `serve` daemon's operator tier) and the terminal
/// rule of DD-77, `target::cli_bias`. A third call is a door that hands out private
/// standing without either check, which every behavioural test of the two existing doors
/// still passes.
///
/// PR-25, which added the terminal's consent prompt, kept the terminal call in
/// `target::cli_bias`: `crates/biorouter-cli/src/dictation/consent.rs` mints
/// `CaptureConsent` and calls `cli_bias`, and never `from_checked` itself.
#[test]
fn private_standing_is_built_in_two_files() {
    const BIAS: &str = "crates/biorouter/src/dictation/bias.rs";
    let tree = scan_tree();
    let mut defined = false;
    let mut found: Vec<(String, String)> = Vec::new();
    for (rel, lines) in &tree.production {
        for (index, (line_no, text)) in lines.iter().enumerate() {
            let (calls, refs) = occurrences(text, "from_checked");
            if calls + refs == 0 {
                continue;
            }
            if rel == BIAS && defines(text, "from_checked") {
                defined = true;
                continue;
            }
            let within = lines[..=index]
                .iter()
                .rev()
                .find_map(|(_, line)| opened_fn(line))
                .unwrap_or_else(|| format!("no fn before line {line_no}"));
            for _ in 0..calls + refs {
                found.push((rel.clone(), within.clone()));
            }
        }
    }
    assert!(
        defined,
        "the scan did not see `Standing::from_checked` defined in {BIAS}. Either it moved, \
         and this test must follow it, or the walk read nothing, which reports the same \
         empty set as a clean tree."
    );
    found.sort();
    let mut want = vec![
        (SESSION_REACH.to_string(), "dictation_standing".to_string()),
        (
            "crates/biorouter/src/dictation/target.rs".to_string(),
            "cli_bias".to_string(),
        ),
    ];
    want.sort();
    assert_eq!(
        found, want,
        "`Standing::from_checked` is called outside the two functions that check a caller \
         before handing out private standing (file, enclosing fn). A new door must ask \
         `HttpCaller::dictation_standing` or the terminal rule instead of building a \
         `Standing` of its own."
    );
}
