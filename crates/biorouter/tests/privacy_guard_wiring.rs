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
                file: "crates/biorouter/src/agents/schedule_tool.rs",
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`platform__manage_schedule`'s `sessions` action, which lists a job's \
                       execution sessions by name and working directory",
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
                file: "crates/biorouter-server/src/routes/agent.rs",
                counts: c(4, 4, 0),
                kind: SiteKind::Guard,
                what: "`POST /agent/resume`, `POST /agent/update_from_session`, and `POST \
                       /agent/update_working_dir`, plus the shared `authorize_agent_control` \
                       gate used by provider, extension, stop, and restart mutations",
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
                counts: c(2, 2, 0),
                kind: SiteKind::Guard,
                what: "`POST /reply`, which runs an agent turn with tools inside the named \
                       session, plus the explicit continuation takeover and group-abandon \
                       recovery mutation",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session.rs",
                counts: c(2, 2, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/{id}` (the transcript) and `GET /sessions/{id}/export` \
                       (the same transcript, `to_string_pretty`); the export sibling was \
                       ungated until this sweep",
            },
            Site {
                file: "crates/biorouter-server/src/routes/session_events.rs",
                counts: c(1, 1, 0),
                kind: SiteKind::Guard,
                what: "`GET /sessions/{id}/events`, which opens with a full-conversation \
                       snapshot frame and then tails it live, ungated until this sweep",
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
                file: SESSION_REACH,
                counts: c(2, 0, 0),
                kind: SiteKind::Guard,
                what: "`gate_knowledge_active`, whose GET query and POST body branches each \
                       invoke the same reach gate",
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
            counts: c(1, 0, 0),
            kind: SiteKind::Guard,
            what: "`session_reach` itself, which is this predicate plus the two lookups that \
                   feed it",
        }],
    },
    Guard {
        ident: "target_tier",
        defined_in: SESSION_REACH,
        decides: "resolves the named session's tier metadata-only, failing closed to Unreadable",
        status: Status::WiredThrough("session_reach"),
        sites: &[Site {
            file: SESSION_REACH,
            counts: c(1, 0, 0),
            kind: SiteKind::Guard,
            what: "`session_reach`'s tier lookup, deliberately `with_messages: false` so \
                   resolving a tier is never the way to load the transcript being refused",
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
                file: VISIBILITY,
                counts: c(3, 0, 1),
                kind: SiteKind::Guard,
                what: "`may_read`, `may_write` and `appears_in_list` are each defined as VIS \
                       (plus lineage, for write), and the `use super::` that brings it in",
            },
        ],
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
                counts: c(1, 0, 0),
                kind: SiteKind::Guard,
                what: "`SessionClassification::bind_allowed`'s inherent-method form",
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
                       resolved a classification (i.e. under enforcement)",
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
                counts: c(3, 0, 0),
                kind: SiteKind::Guard,
                what: "Gates E and F: the tool list and tool dispatch. It was FOUR until the \
                       2026-09-10 test drive's finding M18: the fourth was \
                       `assert_extension_reachable`, the one gate that reads an unknown name \
                       as Private, where this function's flat sentence asserted a privateness \
                       the gate had not established. That call now goes to \
                       `private_or_absent_refusal`, tracked in its own row. Moving it back \
                       would put the false claim back",
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
                what: "`extension_enable_refusal`'s tier arm, the enable doors' one ask. \
                       `workspace_extension.rs` used to ask it directly and \
                       `extension_manager_extension.rs` hand-wrote the same rule in its own \
                       words; both now go through the shared gate, which is what fixed the \
                       clause order the two copies disagreed about",
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
];

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
