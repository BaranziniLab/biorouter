//! The two capability constructors and the **Phase 6 audit** (issue #56 Task 51,
//! DR-26), tested from OUTSIDE `crates/*/src/` on purpose.
//!
//! # Why this file is where the audit lives
//!
//! The census below greps `crates/*/src/` for `CallCapability::sample(` and
//! `CallCapability::public_enforced(` and asserts the exact set of production
//! entries — the check that catches an entry nobody classified. No filter a grep
//! can express separates a test hit from a production one: `grep -v "mod tests"`
//! drops only lines that literally contain that string, which a call to either
//! constructor does not, and excluding the whole definition file instead would
//! blind the census in exactly the file where a fifth sampler is most plausible.
//!
//! So the two tests that must name the real constructors live **here**, where the
//! census does not walk, and the census needs no exception list for them at all.
//! Every other test in the tree builds its capability with
//! `CallCapability::for_test*` and stays beside the code it exercises.
//!
//! # What the audit does NOT assert
//!
//! ⚠ **Not "every path goes through `CallCapability`" — that claim is false**, and
//! a test asserting it would either fail immediately or be quietly weakened until
//! it passed. `CallCapability` covers Gates C, E and F; three other paths decide
//! reach without touching one, for reasons documented in the tree and which are
//! not defects (an HTTP route with no admitted capability to inherit, a subagent
//! spawn constructing a whole new agent, and the `biorouter-mcp` crate boundary
//! across which only a `KbCaller` and a `_meta` string can travel).
//!
//! ⚠ There used to be a fourth: the agent-drafter catalog read the master toggle
//! itself, because it asked `tier::is_private` rather than the barrier. Audit
//! finding 17 removed that second spelling — the catalog now asks
//! `KbCaller::can_reach`, which inherits the toggle from `tier::assert_reachable`
//! like every other knowledge-base filter — so that row is gone rather than
//! merely renumbered.
//!
//! The audit is therefore a **census**: it pins the complete set of sites, the
//! sanctioned ones and the bypassing ones together, and fails when a new one
//! appears. That is what makes DR-26's *"checked in all scenarios where this need
//! to be checked"* mechanical instead of aspirational — a list of call sites is
//! not the answer, because the next call site will not be on it.

use biorouter::agents::types::SharedProvider;
use biorouter::privacy::{CallCapability, ProviderTier};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[test]
fn an_entry_with_no_caller_identity_is_the_most_restrictive_pair() {
    let cap = CallCapability::public_enforced();
    assert_eq!(cap.tier(), ProviderTier::Public);
    assert!(cap.enforced());
    assert!(cap.restricts_private_data());
    // Issue #56 Task 48 (DR-26). A public model's affiliation is the absence of
    // one, and this constructor is the entry with no caller identity at all.
    assert_eq!(cap.affiliation(), None);
}

#[tokio::test]
async fn an_unbound_provider_samples_public_and_states_no_affiliation() {
    // `None` is the legitimate state before the first bind, not an error —
    // and it must resolve to the tier that grants the least reach.
    let provider: SharedProvider = std::sync::Arc::new(tokio::sync::Mutex::new(None));
    let cap = CallCapability::sample(&provider).await;
    assert_eq!(cap.tier(), ProviderTier::Public);
    // Issue #56 Task 48 (DR-26), the other half of `sample`'s `unwrap_or`. It
    // had no assertion at all, so the affiliation of an unbound provider could
    // have been anything — including the `Some(Local)` that pairs with Private
    // in the test constructors, which would hand a chat with no model bound the
    // most permissive affiliation in the vocabulary.
    //
    // Public + `None` is also the only pair that is self-consistent here:
    // `gate_cross_affiliation_warning`'s guards ask affiliation only of two
    // Private endpoints, so a Public tier makes this field unreachable rather
    // than merely unused.
    //
    // The BOUND path — that `sample` reads `p.affiliation()` off the same
    // `as_ref()` as `p.tier()` — is driven end to end by
    // `agents::agent::gate_c_dispatch_tests::
    // binding_a_foreign_institutions_model_warns_and_still_binds`, which binds
    // a real provider covered by another institution and reads the warning back
    // out through `Agent::cross_affiliation_warnings`.
    assert_eq!(cap.affiliation(), None);
}

// ---------------------------------------------------------------------------
// Task 51 — the Phase 6 audit.
// ---------------------------------------------------------------------------

/// One production program point at which something decides how far a caller may
/// reach.
///
/// The `what` field is not decoration. A census whose rows are bare paths is one
/// a future engineer extends by pasting a line to make the build green; a census
/// whose rows must be *described* asks them what the new site is first, which is
/// the whole point of failing the build.
struct Site {
    /// The spelling the audit greps for, outside `//` comments.
    needle: &'static str,
    /// Repo-relative path, `/`-separated.
    file: &'static str,
    /// How many times `needle` occurs in `file`.
    count: usize,
    /// What this site is and why it is a decider.
    what: &'static str,
}

/// Every site that decides how far a caller reaches, as measured against this
/// tree on 2026-08-04.
///
/// Grouped by needle; each needle is one *way* of deciding, and each row is one
/// file that decides that way.
const EXPECTED: &[Site] = &[
    // ---------------------------------------------------------------- the
    // admitted capability. Gates C, E and F all read one of these; it is the ONE
    // read of the provider mutex and the ONE read of the master toggle on a
    // call's path.
    Site {
        needle: "CallCapability::sample(",
        file: "crates/biorouter/src/agents/agent.rs",
        count: 5,
        what: "`call_prefetch_tool` (which dispatches before the turn), the agent \
               loop's own tool-call sample, `cross_affiliation_grant_subject` \
               (which composes the sentence the user is asked to accept), the \
               schedule branch of `dispatch_tool_call`, which returns before the \
               loop's own sample, so `platform__manage_schedule` had no capability \
               in scope while two of its actions read another chat's content — and \
               `issue_tool_bridge`, which decides for a caller that is not in this \
               process at all. A bridged coding agent (`claude_code`, `codex`) runs \
               its own loop in a child and reaches Biorouter's tools by calling back \
               in over MCP, so the child's calls arrive with no capability of their \
               own to inherit. Sampling once when the grant is issued and carrying it \
               in the lease is what fixes a bridged call's capability BEFORE it runs, \
               which is the whole reason this type exists; re-reading per callback \
               would be the two-reads race with a process boundary through the middle",
    },
    Site {
        needle: "CallCapability::sample(",
        file: "crates/biorouter/src/agents/workspace_inspector.rs",
        count: 2,
        what: "TWO inspectors in this file, each deciding for a batch it sees \
               BEFORE the dispatch that would admit a capability — that is the \
               point of an inspector — so neither has one in scope to inherit on \
               the ordinary agent loop, and the alternative to sampling is not \
               inheriting but deciding on `Config::global()`, which is the bug \
               this type exists to prevent. \
               (1) `WorkspaceCrossingInspector`, the first-crossing disclosure, \
               sampled ONCE per batch rather than per request, because two calls \
               in one batch must not be able to gate on two different models, and \
               only after a cheap name check has established that the batch \
               contains a cross-session write at all: an ordinary turn must not \
               pay a provider-mutex read for a disclosure that cannot apply to \
               it. \
               (2) `WorkspaceMutationInspector`'s `workspace_set_tools` \
               pre-flight (QA finding F4). The pair it samples is handed to \
               `set_tools_preflight_refusal` → `preflight_set_tools`, which asks \
               §7's write row (`refuse_unless_writable`), Gate F reach for each \
               ADDED extension, the manageability refusal for each REMOVED one, \
               and `privacy::bind_allowed` for a provider switch — so the answer \
               is a Deny carrying the handler's own sentence, the always-confirm \
               card when the change is one §5 asks about, or silence and on to \
               dispatch; a card for a change this caller's model may not make \
               asks the user to authorise nothing. It is NOT the \
               gate: `handle_set_tools` re-runs the same pre-flight against the \
               capability the call is finally admitted on, so a model swapped \
               between inspection and dispatch can only make this sample stale in \
               the fail-safe direction. \
               ⚠ Each funnels into its OWN `inspect_with_pinned_capability`, \
               and in both a pinned pair WINS and the provider mutex is never \
               read — but by different mechanisms, so grep for the right one: (2) \
               binds `let mut sampled = capability` up front and memoises into \
               it, while (1) has no `sampled` binding at all and matches the \
               `Option` once at its sample point, after the early return above. \
               The only caller that pins a pair is the coding-agent bridge, which \
               threads the one it fixed at issue time so a bridged child's calls \
               cannot re-read the flag across the process boundary; the agent \
               loop, the approval relay and the non-capability entry point pass \
               `None`. (2) additionally samples LAZILY — inside the \
               `is_set_tools_call` arm only — so a batch carrying no \
               `workspace_set_tools` call samples nothing at all, and one \
               carrying two still gates on one model",
    },
    Site {
        needle: "CallCapability::sample(",
        file: "crates/biorouter/src/security/sensitive_ops.rs",
        count: 1,
        what: "`SensitiveOpsInspector::caller_tier`, criterion 5 of the Auto-mode \
               sensitive-operation gate: a PUBLIC-tier model calling a tool that \
               captures the screen or drives the computer raises an approval the \
               user can grant. It is a decider for the same reason \
               `WorkspaceCrossingInspector` is a SECOND one rather than a \
               duplicate — an inspector runs BEFORE the dispatch that would admit \
               a capability, so on the ordinary agent loop there is none in scope \
               to inherit, and the alternative to sampling is deciding on \
               `Config::global()`, which is the bug this type exists to prevent. \
               It differs from that sibling in what it does with an absent \
               capability: it FAILS CLOSED to Public rather than sampling \
               unconditionally, because a screenshot is ambient — it captures \
               whatever is on screen, not a file anything chose to open — and a \
               gate that quietly no-ops without a capability would no-op on \
               exactly the entries nobody classified. Sampled at most once per \
               batch and only after a name check has found such a call, so an \
               ordinary turn pays no provider-mutex read. ⚠ The threaded \
               capability short-circuits this sample: a bridged coding agent \
               (`claude_code`, `codex` — the Public providers this criterion \
               exists for) arrives with `Some(_)` pinned at grant time and never \
               reaches the provider field",
    },
    Site {
        needle: "CallCapability::sample(",
        file: "crates/biorouter/src/agents/knowledge_tool.rs",
        count: 1,
        what: "`handle_ingest_conversation`, the conversation-ingest branch of \
               `dispatch_tool_call`, which returns before the agent loop's own \
               sample exactly as the schedule branch does. It reads `p.tier()` \
               alone before audit finding 17: the tier fed a candidate list that \
               names knowledge bases to the model, so a cross-institution chat was \
               handed the ids of bases the KB barrier then refused. It now samples \
               both axes at one instant and crosses them to `KbCaller`",
    },
    Site {
        needle: "CallCapability::sample(",
        file: "crates/biorouter/src/agents/knowledge_source_tool.rs",
        count: 1,
        what: "`handle_ingest_source`, the chat-document-ingest branch of \
               `dispatch_tool_call`, which returns before the agent loop's own \
               sample for the same reason the conversation-ingest branch above \
               does. It is a SECOND instance of that pattern rather than a \
               duplicate: a different tool, reached by a different argument \
               shape, whose sample feeds `resolve_target_kb` the audience for \
               the candidate list its no-target error may name. That list is \
               exactly what audit finding 17 was about, so the branch that \
               builds it has to decide the pair itself rather than inherit one \
               that is not in scope yet",
    },
    Site {
        needle: "CallCapability::sample(",
        file: "crates/biorouter/src/agents/extension_manager.rs",
        count: 2,
        what: "`extension_reach` (Gate E's discovery filter and mark, which \
               `cross_affiliation_warnings` also reads through) and \
               `assert_extension_reachable` (Gate F's non-tool-call entry points): \
               each falling back to a sample only when handed no admitted capability",
    },
    Site {
        needle: "CallCapability::sample(",
        file: "crates/biorouter/src/knowledge/provider_completer.rs",
        count: 1,
        what: "#109's `complete_with_dispatch`, the seam that lets a knowledge macro \
               run under a coding-agent provider. It is a decider for the same \
               reason `issue_tool_bridge` is and it is a SECOND one rather than a \
               duplicate: a macro calls `Provider::complete` directly, so no agent \
               loop has sampled anything, and the bridged calls arrive from a child \
               process with no capability of their own to inherit. This is the one \
               place holding the `Arc<dyn Provider>` those calls will run under, so \
               sampling here — once, before the turn — is what fixes the pair \
               instead of re-reading per callback across a process boundary. The \
               three callers that build a completer (the HTTP route, the CLI, the \
               probe) inherit it rather than each sampling their own, which is the \
               difference between one decision and three that can disagree",
    },
    Site {
        needle: "CallCapability::public_enforced(",
        file: "crates/biorouter/src/providers/tool_turn.rs",
        count: 1,
        what: "NOT a production decider: `mod tests`' `context()` helper, which every \
               `ProviderToolTurnContext` those tests build takes its pair from — the \
               most restrictive one, rather than a permissive one invented for a \
               test's convenience. Counted for the reason the bridge's test helper \
               below is: a line-wise grep cannot tell a `#[cfg(test)]` block from \
               production, and a filter that tried would blind the census to \
               production too. The production side of this primitive TAKES its \
               capability as an argument — `for_workflow` has no constructor call of \
               its own — precisely so that a workflow cannot acquire one by \
               accident; the one production caller is the `sample(` row above",
    },
    Site {
        needle: "CallCapability::public_enforced(",
        file: "crates/biorouter-server/src/routes/apps.rs",
        count: 1,
        what: "NOT a production decider: `mod skills_grant_scope`'s `dispatch()` \
               helper, which drives a real tool call to prove #149's app skills \
               allow-list is enforced at BOTH the roster and the dispatch layer. It \
               takes the most restrictive pair rather than a permissive one invented \
               for the test's convenience, and pairs it with `without_human_surface` \
               plus a 20s deadline so a refusal that PARKS fails the test instead of \
               hanging the binary. Counted for the same reason as the `tool_turn.rs` \
               and `bridge.rs` helpers above: a line-wise grep cannot tell a \
               `#[cfg(test)]` block from production, and a filter that tried would \
               blind the census to production too. Production app agents never build \
               a capability here — `configure_agent` arms extensions and the turn \
               loop samples, which are `sample(` rows above",
    },
    Site {
        needle: "CallCapability::public_enforced(",
        file: "crates/biorouter-server/src/routes/agent.rs",
        count: 2,
        what: "the two entries with no caller identity — `POST /agent/call_tool` and \
               `POST /agent/read_resource` — which therefore take the most \
               restrictive pair this type can express. ⚠ `read_resource` reached \
               2026-09-09 passing `None` instead, which made the guard sample the \
               NAMED session's bound model: an HTTP client holding the daemon \
               secret could name a private chat and read its private extensions' \
               resources on that chat's reach. That is the same \"borrow another \
               session's capability\" hole the execution plan closed at \
               `call_tool`; it ruled the resource route the same way for the same \
               reason (§13012) and only `call_tool` was migrated. The route lost \
               its last in-repo caller when MCP apps was removed, which is why \
               nothing noticed",
    },
    Site {
        needle: "CallCapability::public_enforced(",
        file: "crates/biorouter/src/providers/coding_agent/bridge.rs",
        count: 1,
        what: "NOT a production decider: `mod tests`' `test_capability()` helper, \
               which every `BridgeGrant` the bridge's own unit tests build takes its \
               pair from — the most restrictive one, rather than a permissive one \
               invented for a test's convenience. Counted for the same reason \
               `privacy/grant.rs`'s test helper is — a line-wise grep cannot tell a \
               `#[cfg(test)]` block from production, and a filter that tried would \
               blind the census to production too. The production side of this \
               bridge decides in `Agent::issue_tool_bridge`, which is a `sample(` \
               row above. ⚠ This row read `dummy_grant()` while four inline \
               spellings had accumulated in that file, so the census sat RED \
               through a whole branch whose own gate list never ran this binary. \
               The single helper is what stops it drifting that way again: a new \
               test that inlines the constructor still moves the count off 1 and \
               still fires here",
    },
    // ---------------------------------------------------------- DR-26's third
    // axis, asked through the free function rather than off a capability. Its
    // own doc records why it exists: for surfaces that hold an `Arc<dyn Provider>`
    // directly and have no admitted capability to inherit.
    Site {
        needle: "affiliation::gate_cross_affiliation",
        file: "crates/biorouter-server/src/routes/agent.rs",
        count: 1,
        what: "DR-26 bypassing path 1, `POST /agent/add_extension`, an HTTP route \
               and not a tool dispatch, so there is no admitted capability to inherit",
    },
    Site {
        needle: "affiliation::gate_cross_affiliation",
        file: "crates/biorouter/src/agents/subagent_tool.rs",
        count: 1,
        what: "DR-26 bypassing path 2, the subagent spawn's extension filter, which \
               is constructing a whole new agent and reads the child's own provider",
    },
    Site {
        needle: "affiliation::gate_cross_affiliation",
        file: "crates/biorouter/src/privacy/capability.rs",
        count: 1,
        what: "the sanctioned narrowing: `CallCapability::cross_affiliation`, which is \
               this same gate with the three model axes taken off one sample",
    },
    Site {
        needle: "affiliation::gate_cross_affiliation",
        file: "crates/biorouter/src/privacy/grant.rs",
        count: 1,
        what: "NOT a production decider: `mod tests`' `statement()` helper, which \
               composes the user-facing prompt through the real gate rather than \
               re-deriving it. Counted because a line-wise grep cannot tell a \
               `#[cfg(test)]` block from production, and a filter that tried would \
               blind the census to production too",
    },
    // ------------------------------------------------------- DR-27's narrowing
    // (Task 52). The mixing policy is read in exactly one function, and this is
    // the census of who asks it. It decides whether a resolved mismatch is
    // SPOKEN — so a new caller is a new place `open` could be silent or not, and
    // the two disagreeing is precisely what the single reader exists to prevent.
    // Both spellings of the reader (`refusing_mismatch`, and `refusing_mismatches`
    // for a whole surface's worth) share this prefix on purpose.
    Site {
        needle: "affiliation::refusing_mismatch",
        file: "crates/biorouter/src/privacy/capability.rs",
        count: 1,
        what: "`CallCapability::cross_affiliation_warning`, the REFUSAL spelling of \
               the gate, which Gate C's dispatch denial, `assert_extension_reachable`'s \
               eight entry points and `extensionmanager__manage_extensions` all read \
               through, so none of them reads a mode of its own",
    },
    Site {
        needle: "affiliation::refusing_mismatch",
        file: "crates/biorouter/src/agents/agent.rs",
        count: 1,
        what: "`Agent::cross_affiliation_warnings`, the bind-time statement, which is \
               the same sentence `/agent/add_extension` logs from the other end and so \
               must go quiet in `open` with it. The RESOLUTION it is built on stays \
               mode-blind",
    },
    // ------------------------------------------------ the `biorouter-mcp` crate
    // boundary. That crate cannot depend on `biorouter`, so no `CallCapability`
    // can cross: what crosses is a `KbCaller` (both axes, one value — audit
    // finding 17) and a `_meta` string, and the master opt-out is read from the
    // process-global directly.
    Site {
        needle: "crate::privacy_toggle::privacy_tiers_enabled()",
        file: "crates/biorouter-mcp/src/knowledge/server.rs",
        count: 1,
        what: "`kb_export` forcing a private base's `.brkb` into the model-export \
               directory: its own read, not one inherited from `assert_reachable`, \
               because choosing the destination is a decision rather than a barrier",
    },
    Site {
        needle: "crate::privacy_toggle::privacy_tiers_enabled()",
        file: "crates/biorouter-mcp/src/knowledge/tier.rs",
        count: 2,
        what: "DR-26 bypassing path 3, the knowledge-base gates: the barrier \
               (`assert_reachable`) and `ratchets_are_live`, the ONE spelling \
               both classification ratchets read. It was 3, one read per \
               ratchet, until the KB-to-KB merge added a third KIND of reader — \
               a dry run, which must report what the two ratchets WOULD do \
               without doing it. Three reads of a process-global another thread \
               can flip is the two-reads race with an extra leg, and a preview \
               that disagrees with the write it previews is the specific \
               failure; the barrier's read stays separate because it answers a \
               different question. Every knowledge-base LISTING still inherits \
               the toggle from the barrier rather than reading it; see audit \
               finding 17",
    },
];

/// `<workspace>` — `CARGO_MANIFEST_DIR` is `<workspace>/crates/biorouter`.
fn workspace_root() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Every production `.rs` file under `crates/*/src/`, as `(repo-relative path,
/// contents)`.
///
/// ⚠ **The window is `/src/` and not `crates/`**, which is what keeps this file's
/// own `EXPECTED` table — which necessarily spells every needle — from being
/// counted as a decider. Every crate in this workspace keeps its sources in
/// `crates/<name>/src/` and nothing else in the tree contains that segment, so
/// the filter is exact rather than heuristic.
///
/// Every way this walk can silently do no work is made loud instead: a wrong
/// root, an unreadable directory, an unreadable file and an implausibly small
/// scan all fail rather than reporting the same clean result as a clean tree.
fn production_sources() -> Vec<(String, String)> {
    let root = workspace_root();
    let crates = root.join("crates");
    assert!(
        crates.is_dir(),
        "the audit walks {}; if that path is wrong, every assertion below passes \
         for the wrong reason",
        crates.display()
    );
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(&crates) {
        let entry = entry.expect("the audit must not silently skip an unreadable directory");
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let rel = p
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if !rel.contains("/src/") {
            continue;
        }
        let src = std::fs::read_to_string(p)
            .unwrap_or_else(|e| panic!("the audit could not read {rel}: {e}"));
        out.push((rel, src));
    }
    assert!(
        out.len() >= 400,
        "only {} production .rs files were scanned (556 at Task 51). The walk is no \
         longer covering the tree, and a broken walk reports the same empty set as a \
         clean one.",
        out.len()
    );
    out
}

/// The audit that answers DR-26's *"checked in all scenarios where this need to
/// be checked"* — structurally, by census rather than by enumeration.
///
/// ⚠ **Every row here is load-bearing, including the ones that are not defects.**
/// The four bypassing paths are legitimately separate and the census does not ask
/// them to change; what it asks is that a *fifth* cannot join them unnoticed. A
/// new site fails this test, and the repair is to add a row that says what the
/// site is — which is the conversation that would otherwise never happen.
///
/// The tree already does exactly this twice, and both caught real omissions:
/// `declassify::tests::the_proof_of_user_is_constructed_in_exactly_two_places` and
/// `privacy::tests::floor_is_crossed_only_where_a_capability_establishes_a_classification`.
///
/// ⚠ **Comment lines are skipped**, for the reason `grant.rs`'s twin audit
/// records: prose cannot decide anything, and an audit that counted a `///` line
/// goes red across a task boundary with nobody's code at fault — which teaches the
/// next person to relax an assertion in order to make a comment compile.
///
/// ⚠ **What this cannot see, stated so it is not read as airtight.** The census
/// counts five *spellings*. A decider that reaches for none of them — one that
/// reads `Provider::tier()` off an `Arc` and compares it to a resolved
/// extension's tier inline, never touching a capability, the free gate or the
/// crate-boundary toggle — is invisible here. The affiliation half of such a
/// decider is caught by the second audit below, which asks a different question
/// and would see the type it has to name; the tier half is not covered by this
/// file at all. What narrows it in practice is that
/// `privacy::refusal::privacy_refusal` is the one predicate every tier gate asks,
/// so a hand-rolled tier comparison is already a visible anomaly in review — but
/// that is a habit, not a mechanism, and it should not be cited as one.
#[test]
fn the_sites_that_decide_how_far_a_caller_reaches_are_exactly_these() {
    let needles: BTreeSet<&str> = EXPECTED.iter().map(|s| s.needle).collect();

    let mut found: BTreeMap<(&str, String), usize> = BTreeMap::new();
    for (rel, src) in production_sources() {
        for line in src.lines() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for needle in &needles {
                // Occurrences, not lines: EXPECTED asserts exact COUNTS, so two
                // decisions written on one line have to read as two.
                let hits = code.matches(needle).count();
                if hits > 0 {
                    *found.entry((needle, rel.clone())).or_default() += hits;
                }
            }
        }
    }

    let found: Vec<(&str, String, usize)> = found
        .into_iter()
        .map(|((needle, file), count)| (needle, file, count))
        .collect();
    let mut want: Vec<(&str, String, usize)> = EXPECTED
        .iter()
        .map(|s| (s.needle, s.file.to_string(), s.count))
        .collect();
    // `found` comes out of a BTreeMap and is therefore sorted; sort `want` too,
    // so the next row added fails for the reason it is really wrong about rather
    // than for its position in the list.
    want.sort();

    // The descriptions are rendered into the failure, so whoever trips this reads
    // what the census already knows about before deciding what their new site is.
    let described: String = EXPECTED
        .iter()
        .map(|s| {
            format!(
                "  {} x{}\n    {}\n    {}\n",
                s.needle, s.count, s.file, s.what
            )
        })
        .collect();

    assert_eq!(
        found, want,
        "the set of sites that decide how far a caller reaches has changed.\n\n\
         Adding one is a design change, not a refactor: DR-26 requires that the third \
         axis be checked everywhere reach is decided, and the only thing making that \
         mechanical is this census being complete. If the new site is legitimate, add \
         a `Site` row saying what it is and why it decides; if it is a gate that \
         should have inherited an admitted `CallCapability`, give it one instead.\n\n\
         The sites this census already knows about:\n{described}"
    );
}

/// The half a census cannot see on its own: a path that compares affiliations
/// **by hand** instead of asking the one comparison.
///
/// A gate that hand-compares is a second implementation of DR-26's table, and the
/// two will disagree on the row nobody thought about — silently, because a
/// disagreement between a mark and a refusal shows up as a tool listed without a
/// warning and then refused, or marked and then dispatched.
///
/// # What "compare" means here, stated exactly
///
/// A line that names an affiliation type (or reads one off a capability or a
/// provider) **and** applies a comparison operator to it. Those are the operators
/// a gate would reach for; a test's `assert_eq!` on an affiliation is not a reach
/// decision and is deliberately not matched, because pinning it would have to
/// exempt `privacy/capability.rs` — a real production file, and precisely the one
/// where a hand-rolled comparison would be most at home.
///
/// # The two sanctioned comparisons, and why there are two
///
/// `privacy::affiliation::compatible` is **the** comparison. Its mirror
/// `biorouter_mcp::knowledge::affiliation::reachable` exists only because
/// `biorouter-mcp` cannot depend on `biorouter`, so the vocabulary has to be
/// restated on the far side of the crate boundary; the two are driven against each
/// other by the cross-crate agreement tests rather than trusted to agree.
///
/// # The residual, written down rather than left to be discovered
///
/// A comparison whose operands are bound to locals first would not match, because
/// no affiliation token remains on the comparing line. What catches that class is
/// the second half below: to compare a model against an extension you must obtain
/// the extension side, and `ExtensionAffiliation` is the only door to it.
///
/// `providers::composite_affiliation` compares two `InstitutionId`s with `==` and
/// is not matched (its line names no type). That is correct rather than a miss: it
/// folds two *model* halves into the single value representing a lead/worker pair,
/// which is a different question from reach and is pinned by
/// `providers::affiliation_tests` in its own right.
#[test]
fn compatible_is_the_only_function_that_compares_two_affiliations() {
    /// Reading one of these off anything is holding an affiliation.
    const AFFILIATION: &[&str] = &[
        "ModelAffiliation",
        "ExtensionAffiliation",
        "CallerAffiliation",
        "KbAffiliation",
        "InstitutionId",
        // Task 56 made the model side set-valued, so the set is a second way to
        // be holding an affiliation and belongs in this list beside the id.
        "InstitutionSet",
        ".affiliation()",
        // ⚠ Renamed from `.institution()` with the accessor it names. A model may
        // now be covered by several institutions, so the accessor answers `None`
        // rather than picking one — and a needle that no longer matches anything
        // is a census that has quietly stopped looking.
        ".sole_institution()",
    ];
    /// The operators a gate deciding reach would use.
    const COMPARISON: &[&str] = &["==", "!=", "matches!(", ".contains("];

    /// `privacy::affiliation::compatible` and its cross-crate mirror. These two
    /// files ARE the comparison; everything else must call one of them.
    const SANCTIONED: &[&str] = &[
        "crates/biorouter/src/privacy/affiliation.rs",
        "crates/biorouter-mcp/src/knowledge/affiliation.rs",
    ];
    /// A whole-file `#[cfg(test)]` module, exempt because it exercises the fold
    /// above rather than deciding anything. The exemption's own precondition is
    /// asserted below, so it cannot quietly become a production exemption.
    const CFG_TEST_MODULE: &str = "crates/biorouter/src/providers/affiliation_tests.rs";

    let sources = production_sources();

    let providers_mod = sources
        .iter()
        .find(|(rel, _)| rel == "crates/biorouter/src/providers/mod.rs")
        .map(|(_, src)| src.clone())
        .expect("providers/mod.rs is where the exempt module is declared");
    let lines: Vec<&str> = providers_mod.lines().map(|l| l.trim()).collect();
    assert!(
        lines
            .windows(2)
            .any(|w| w[0] == "#[cfg(test)]" && w[1] == "mod affiliation_tests;"),
        "{CFG_TEST_MODULE} is exempted below on the grounds that it is compiled only \
         under `cfg(test)`. It is not any more, so the exemption now hides a \
         hand-rolled affiliation comparison from a shipped binary."
    );

    let mut hand_rolled: Vec<String> = Vec::new();
    for (rel, src) in &sources {
        if SANCTIONED.contains(&rel.as_str()) || rel == CFG_TEST_MODULE {
            continue;
        }
        for (i, line) in src.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if AFFILIATION.iter().any(|t| code.contains(t))
                && COMPARISON.iter().any(|c| code.contains(c))
            {
                hand_rolled.push(format!("{rel}:{}: {}", i + 1, line.trim()));
            }
        }
    }
    assert!(
        hand_rolled.is_empty(),
        "an affiliation is compared outside `privacy::affiliation::compatible` (and its \
         cross-crate mirror `knowledge::affiliation::reachable`). DR-26's table has one \
         implementation on purpose; a second will disagree with the first on the row \
         nobody thought about, and the disagreement is silent. Call `compatible`, \
         `owners_compatible`, `CallCapability::cross_affiliation` or \
         `affiliation::gate_cross_affiliation` instead:\n{hand_rolled:#?}"
    );

    // The other half: the extension side of the pair is reachable only through
    // its type, so a comparison that bound its operands to locals first still has
    // to name `ExtensionAffiliation` somewhere in the file that does it.
    const NAMES_THE_EXTENSION_SIDE: &[&str] = &[
        // The vocabulary itself, and the resolver that produces it.
        "crates/biorouter/src/privacy/affiliation.rs",
        "crates/biorouter/src/privacy/extensions.rs",
        // The re-export, so `crate::privacy::ExtensionAffiliation` resolves.
        "crates/biorouter/src/privacy/mod.rs",
        // The narrowing every gate asks, plus its unit tests' fixtures.
        "crates/biorouter/src/privacy/capability.rs",
        // Task 49's grant: `mod tests` builds an extension affiliation to drive
        // the real gate with.
        "crates/biorouter/src/privacy/grant.rs",
        // Gate C's classification ratchet destructures `Institutions(..)` to
        // record the owners a chat has touched — a read, never a comparison.
        "crates/biorouter/src/agents/extension_manager.rs",
        // Serialises a marketplace descriptor for the confirmation card. It
        // never compares the extension affiliation with a model affiliation.
        "crates/biorouter/src/agents/extension_manager_extension.rs",
        // Validates and carries registry affiliation metadata. Its conservative
        // extension-authority merge delegates to `privacy::affiliation`; model
        // reach still delegates to `compatible`.
        "crates/biorouter/src/marketplace.rs",
        // Converts the durable private-authority representation into the shared
        // affiliation vocabulary. No model affiliation enters this module.
        "crates/biorouter/src/privacy/registry_live.rs",
        // The `cfg(test)` module exempted above.
        CFG_TEST_MODULE,
    ];
    let mut naming: Vec<&str> = sources
        .iter()
        .filter(|(_, src)| {
            src.lines().any(|l| {
                let code = l.trim_start();
                !code.starts_with("//") && code.contains("ExtensionAffiliation")
            })
        })
        .map(|(rel, _)| rel.as_str())
        .collect();
    naming.sort();
    let mut want: Vec<&str> = NAMES_THE_EXTENSION_SIDE.to_vec();
    want.sort();
    assert_eq!(
        naming, want,
        "a new file names `ExtensionAffiliation`. That type is the extension side of \
         DR-26's comparison, and obtaining it is the first move of any hand-rolled \
         reach decision, including one that binds its operands to locals and so slips \
         past the line-wise scan above. If the new file really only reads an \
         affiliation rather than comparing one, add it with a comment saying which."
    );
}
