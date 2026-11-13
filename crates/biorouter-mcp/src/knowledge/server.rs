use crate::knowledge::{
    affiliation::CallerAffiliation,
    convert::SourceInput,
    service::KnowledgeService,
    store::SearchScope,
    types::{ChangeKind, Manifest},
};
use anyhow::Result;
use dashmap::DashMap;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    schemars::JsonSchema,
    service::RequestContext,
    tool, tool_router, ErrorData, RoleServer, ServerHandler,
};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, collections::HashSet, sync::Arc};
use tokio::sync::Mutex;

const SESSION_ID_META_KEY: &str = "biorouter-session-id";
const TRANSACTION_UNAVAILABLE: &str =
    "knowledge transaction is not available for this session and knowledge base";

/// Both axes of the caller's identity — the tier bit and the affiliation — read
/// from ONE request meta at one instant (issue #56 DR-26).
///
/// ⚠ **It is a struct, and that is the whole point (audit finding 17).** The two
/// guards over this one capability used to take different arguments:
/// `assert_kb_reachable` took `(caller_private, caller_affiliation)` and
/// `kb_is_out_of_reach` took `caller_private` alone — so the filters *could not*
/// ask the affiliation axis even if their author wanted to, and in a
/// cross-institution chat `kb_list_bases` listed the names of bases whose
/// content the very next call refused. Half-knowing something is how a user
/// experiences a barrier with two spellings.
///
/// Passing the pair as one value means a future filter cannot silently drop an
/// axis: there is no narrower thing to pass. A third axis, if DR-26 ever grows
/// one, is a field here and every guard gets it at once.
#[derive(Clone, Debug)]
pub(crate) struct CallerIdentity {
    /// Whether the model bound to this chat is private (`ProviderTier`).
    private: bool,
    /// Whose agreements cover that model.
    affiliation: CallerAffiliation,
}

impl CallerIdentity {
    /// Read both axes off one `RequestContext`. Absent context reads the
    /// restrictive value on both — see [`KnowledgeServer::caller_affiliation`].
    fn from_context(context: Option<&RequestContext<RoleServer>>) -> Self {
        Self {
            private: KnowledgeServer::caller_is_private(context),
            affiliation: KnowledgeServer::caller_affiliation(context),
        }
    }

    /// The same pair, as the crate-wide carrier the barrier takes directly.
    ///
    /// A conversion and not a second sampling: `caller::KbCaller` is this type's
    /// public twin — one value, both axes — and anything outside this file that
    /// needs to ask the barrier takes that one. Building it here, from the
    /// `CallerIdentity` already read off this request, is what keeps the merge's
    /// two ids gated by the same instant's identity as CP1 used.
    fn kb_caller(&self) -> crate::knowledge::caller::KbCaller {
        crate::knowledge::caller::KbCaller::new(self.private, self.affiliation.clone())
    }
}

/// Tools whose `kb_id` argument names a base the caller must be allowed to
/// reach. One list, one rule.
///
/// It is an opt-in allowlist, so on its own a twenty-second `kb_*` tool would
/// default to *ungated* and nothing here would say so. What makes the list
/// complete is a test, not this comment:
/// `every_tool_the_router_exposes_is_classified_by_the_probe_table` requires
/// every tool the router exposes to appear in the classification table with an
/// explicit ratchets= decision, and requires every name here to be a real tool.
/// Opting a new tool out is therefore an edit a reviewer sees.
///
/// ⚠ `kb_set_active` is deliberately absent, and Task 10C decided it: it stays
/// off this list and is capability-aware **in its own body** instead. A private
/// base is not "refused" to it — it is simply NOT A MEMBER of the caller's set,
/// answered with the same sentence an id that does not exist gets, because a
/// refusal that says "that one is private" confirms it exists. Being on this
/// list would produce the privacy refusal by name and re-open exactly that
/// oracle. See `EXEMPT` in the tests below, where each of the five non-gated
/// tools carries the test that pins the behaviour its exemption claims.
const KB_ID_GATED_TOOLS: &[&str] = &[
    "kb_list_pages",
    "kb_read_page",
    // Reads nothing back to the caller from the base's *content*, and is gated
    // anyway: in BioOKF mode it indexes every page's `identifier` to answer
    // "does this `object` resolve?", and an unresolved-vs-resolved answer over
    // a private base's names is that base's contents one bit at a time.
    "kb_validate_page",
    // Reads every page of the base to build its report, and the report itself
    // quotes page paths, identifiers and edges back to the caller. There is no
    // reading of a base more thorough than this one.
    "kb_lint",
    "kb_get_graph",
    "kb_list_history",
    "kb_search",
    "kb_search_raw_sources",
    "kb_export",
    // Reads nothing back and writes nothing — it removes the base entirely —
    // and is gated for the reason the writes are, one step further on.
    // Destruction is at least as sensitive as a read: a public caller that
    // could not read a private base must not be able to end it either, and the
    // refusal has to say what it refused rather than reporting a base that is
    // not there.
    //
    // Being on this list is also what stops the delete resolving an omitted
    // `kb_id`: `KB_PRIMARY_RESOLVING_TOOLS` deliberately does NOT name it, so
    // `gated_kb_id` returns `None` for an absent id and the tool's own required
    // argument produces the error. See `DeleteBaseParams`.
    "kb_delete_base",
    "kb_write_page",
    "kb_add_raw_source",
    "kb_append_log",
    "kb_restore_state",
    "kb_begin_txn",
    "kb_commit_txn",
    "kb_abort_txn",
    // Both merge tools name the DESTINATION in `kb_id`, so this entry gates the
    // base being written. The SOURCE takes its own `assert_reachable` inside
    // `KnowledgeService::merge_bases`, because this seam resolves one id and a
    // merge names two — and a preview that reported the source's page paths and
    // identifiers to a caller barred from reading it would be the leak, with the
    // refusal on the write half merely the tell.
    "kb_merge_preview",
    "kb_merge",
];

/// The subset that resolves an omitted `kb_id` to the session's primary (see
/// [`KnowledgeServer::kb_id_or_primary`]). For these an ABSENT id must be
/// resolved and checked too, or "just drop the kb_id" is the bypass.
const KB_PRIMARY_RESOLVING_TOOLS: &[&str] = &[
    "kb_list_pages",
    "kb_read_page",
    "kb_lint",
    "kb_get_graph",
    "kb_list_history",
];

/// Content-bearing writes by a model: the base takes the caller's tier BEFORE
/// the write runs (issue #56).
///
/// ⚠ **`kb_validate_page` is deliberately absent** (Stage 4, DR-8). It is the
/// one new tool that names a base and writes nothing: it parses text the caller
/// already holds and reports diagnostics. Ratcheting on it would raise a public
/// base to PRIVATE because a private chat *checked a draft* it never committed
/// — a tier raise is permanent, so that is a one-way loss of reach bought for
/// nothing. Gated: yes. Ratcheting: no. The pair is asserted by
/// `every_tool_that_writes_content_ratchets_and_the_plumbing_ones_do_not`.
///
/// ⚠ **`kb_lint` is absent for the same reason, and it cost the autofix.** DR-8
/// says every tool that WRITES joins this list, and a lint has two halves: the
/// scan writes nothing, the autofix rewrites pages. This table is a set of tool
/// NAMES — there is nowhere in it to say "ratchets when an argument is set", and
/// a row that ratcheted sometimes would be a row a reader has to open the tool
/// to understand. So the tool exposes the read-only scan **only**: gated, not
/// ratcheting, and honest under one word. The autofix stays on the two surfaces
/// that name a provider and therefore have a tier to ratchet with —
/// `biorouter kb lint --fix` and `POST /knowledge/bases/{id}/lint`, both of
/// which go through `macros::lint::lint`, which ratchets at its own entry.
///
/// ⚠ **`kb_merge` is here and `kb_merge_preview` is not**, and the split is
/// DR-8's corollary rather than a judgement about merges: a tool whose ratchet
/// decision depends on an argument is narrowed until the decision is a constant.
/// One `kb_merge` with a `dry_run` flag would be a row that ratchets half the
/// time, so a preview would permanently privatise a public base because a
/// private chat *looked at what a merge would do*. Two tools, two constants.
const KB_RATCHETING_TOOLS: &[&str] = &[
    "kb_write_page",
    "kb_add_raw_source",
    "kb_append_log",
    "kb_merge",
];

/// Tools that read across the session's visible set rather than one named base
/// (DR-33, design §2.6): with no `kb_id` the searches fan out, the listing
/// names every visible base, and the two pointer tools answer with the set. A
/// result of one of these, given to a private caller, is marked as carrying
/// private content whenever a private base is in that set.
const KB_VISIBLE_SET_TOOLS: &[&str] = &[
    "kb_search",
    "kb_search_raw_sources",
    "kb_list_bases",
    "kb_get_active",
    "kb_set_active",
];

#[derive(Clone)]
pub struct KnowledgeServer {
    tool_router: ToolRouter<Self>,
    service: KnowledgeService,
    instructions: String,
    transactions: KnowledgeTransactionCoordinator,
}

#[derive(Clone, Default)]
struct KnowledgeTransactionCoordinator {
    active: Arc<DashMap<String, ActiveKnowledgeTransactionSlot>>,
}

type ActiveKnowledgeTransactionSlot = Arc<Mutex<Option<ActiveKnowledgeTransaction>>>;

struct ActiveKnowledgeTransaction {
    handle: String,
    session_id: String,
    txn: crate::knowledge::git::Txn,
    _write_guard: crate::knowledge::service::KnowledgeWriteGuard,
    /// When this transaction was opened or last used (#157).
    ///
    /// An open transaction holds the base's WRITE LOCK — the in-process mutex
    /// and the on-disk `flock` both — for its whole lifetime, so an abandoned
    /// one makes every later operation on that base, from any other session,
    /// block forever. Measured: `kb_begin_txn` and nothing else leaves an
    /// outside `flock(LOCK_EX|LOCK_NB)` refused, and a second session's
    /// `kb_get_graph` and `kb_lint` on that base never return while another
    /// base answers in 8 ms.
    ///
    /// The two reapers that existed both fire on TEARDOWN — session end,
    /// connection end — so a transaction abandoned while its session is still
    /// alive was never released. That is the ordinary case: a model opens one,
    /// then the turn errors, is cancelled, or simply moves on.
    touched_at: std::time::Instant,
}

/// How long an open knowledge transaction may sit untouched before it is
/// treated as abandoned and rolled back (#157).
///
/// ⚠ Longer than the longest LEGITIMATE quiet stretch inside one, which is a
/// macro's own wall-clock budget (`SubAgentBounds::max_wall` — 300 s by default,
/// 900 s as `biorouter-cli` builds it): a macro opens a transaction and can go
/// quiet for a whole model call inside it. Reaping a live transaction would
/// roll back work in progress, which is worse than the hang this prevents, so
/// the threshold is deliberately generous. It only has to be FINITE — the bug
/// is that an abandoned transaction was held until the process exited.
const ABANDONED_TRANSACTION_IDLE: std::time::Duration = std::time::Duration::from_secs(1800);

impl KnowledgeTransactionCoordinator {
    /// Slots whose transaction has been untouched for longer than
    /// [`ABANDONED_TRANSACTION_IDLE`].
    async fn abandoned(&self) -> Vec<(String, ActiveKnowledgeTransactionSlot)> {
        self.abandoned_after(ABANDONED_TRANSACTION_IDLE).await
    }

    /// [`Self::abandoned`] with the threshold supplied, so a test can drive the
    /// reaper without waiting out the shipped 30 minutes.
    async fn abandoned_after(
        &self,
        idle: std::time::Duration,
    ) -> Vec<(String, ActiveKnowledgeTransactionSlot)> {
        let mut out = Vec::new();
        for (kb_id, slot) in self.slots() {
            // `try_lock`, never `lock().await`: a slot held by a transaction
            // that is being committed right now is BUSY, not abandoned, and
            // waiting for it here would put the reaper behind the very work it
            // is meant to be checking on.
            let is_abandoned = match slot.try_lock() {
                Ok(active) => active
                    .as_ref()
                    .is_some_and(|txn| txn.touched_at.elapsed() >= idle),
                Err(_) => false,
            };
            if is_abandoned {
                out.push((kb_id, slot));
            }
        }
        out
    }

    fn slot(&self, kb_id: &str) -> ActiveKnowledgeTransactionSlot {
        self.active
            .entry(kb_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone()
    }

    fn slots(&self) -> Vec<(String, ActiveKnowledgeTransactionSlot)> {
        self.active
            .iter()
            .map(|entry| (entry.key().clone(), Arc::clone(entry.value())))
            .collect()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateBaseParams {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    // ⚠ A doc comment here would be shipped TO THE MODEL: schemars renders `///`
    // into the property's `description`, and this file's convention of writing
    // the reasoning at the site would put a paragraph about DR-12 into every
    // tool listing, on every turn. So the model-facing sentence is a `///` and
    // everything below is a plain comment.
    //
    // ⚠ **A `String` parsed by hand, not a `KbFormat`, and the asymmetry is the
    // point.** `KbFormat`'s own `Deserialize` is deliberately lenient — DR-12
    // traces what a failing `manifest.yaml` load costs, so an unknown profile on
    // disk reads as plain OKF rather than losing the user their pointers. That
    // is the correct reading of a *file already written*. It is the wrong
    // reading of a *request*: a model that asks for `bio-okf` and silently gets
    // an OKF base has been given the opposite of what it asked for, discovers it
    // pages later, and cannot convert (DR-26). Producers are held to a higher
    // bar than consumers (DR-7), so this one refuses and names the two legal
    // values.
    //
    // `schemars(with)` keeps the generated schema an `enum` of exactly those
    // two, which is what a provider can constrain sampling with (DR-16) — so the
    // strict parse in `format()` is the backstop, not the first line of defence.
    /// `okf` (default) for general-purpose knowledge, or `biookf` for biomedical
    /// knowledge under the BioOKF controlled vocabulary. See the tool
    /// description; the choice cannot be changed later.
    #[serde(default)]
    #[schemars(with = "Option<crate::knowledge::types::KbFormat>")]
    pub format: Option<String>,
}

impl CreateBaseParams {
    /// The requested profile, or an `INVALID_PARAMS` naming the legal values.
    fn format(&self) -> Result<crate::knowledge::types::KbFormat, ErrorData> {
        let Some(raw) = self.format.as_deref().map(str::trim) else {
            return Ok(crate::knowledge::types::KbFormat::default());
        };
        if raw.is_empty() {
            return Ok(crate::knowledge::types::KbFormat::default());
        }
        crate::knowledge::types::KbFormat::parse(raw).ok_or_else(|| {
            ErrorData::invalid_params(
                format!(
                    "unknown knowledge base format {raw:?}: use \"okf\" for general-purpose \
                     knowledge or \"biookf\" for biomedical knowledge under the BioOKF \
                     controlled vocabulary"
                ),
                None,
            )
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ValidatePageParams {
    pub kb_id: String,
    /// The path this page will be written to, when you know it (for example
    /// `knowledge/molecule/aspirin.md`). Optional, and worth passing: it is what
    /// lets the check tell "this rewrites the page that already owns this
    /// identifier" from "this is a second page claiming a name that is taken".
    #[serde(default)]
    pub path: Option<String>,
    /// The full page text you are about to write, frontmatter block included.
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportArchiveParams {
    /// Knowledge base id to export.
    pub kb_id: String,
    /// Absolute file path to write the `.brkb` archive to. If omitted, a file
    /// named `<kb_id>.brkb` is written to the system temp directory.
    #[serde(default)]
    pub dest_path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportArchiveParams {
    /// Absolute path to the `.brkb` archive file to import.
    pub src_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KbIdParams {
    pub kb_id: String,
}

/// `kb_id` is the **destination** on purpose, and it is not a naming preference.
///
/// `gated_kb_id` reads the argument called `kb_id` and nothing else, so spelling
/// the destination that way is what puts the merge behind CP1's barrier and
/// CP1's ratchet with no change to the seam. The source takes the *second*
/// barrier, inside `merge_bases`, because one seam cannot gate two ids.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MergeParams {
    /// The knowledge base to merge INTO. It is canonical: its identifiers,
    /// paths and raw sources win on every collision and are never modified.
    pub kb_id: String,
    /// The knowledge base to merge FROM. It is only read, and is left unchanged.
    pub source_kb_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListPagesParams {
    pub kb_id: String,
    #[serde(default)]
    pub path_prefix: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadPageParams {
    pub kb_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct WritePageParams {
    pub kb_id: String,
    pub path: String,
    pub content: String,
    pub commit_message: String,
    /// Opaque handle returned by `kb_begin_txn`. Omit for a standalone commit.
    #[serde(default)]
    pub txn: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddRawSourceParams {
    pub kb_id: String,
    pub source: RawSourceInput,
    /// Opaque handle returned by `kb_begin_txn`. Omit for a standalone commit.
    #[serde(default)]
    pub txn: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RawSourceInput {
    Url {
        url: String,
    },
    Text {
        text: String,
        #[serde(default)]
        title: Option<String>,
    },
    // File uploads via MCP are out of scope; the HTTP layer (Plan 3) handles them.
}

impl From<RawSourceInput> for SourceInput {
    fn from(r: RawSourceInput) -> Self {
        match r {
            RawSourceInput::Url { url } => SourceInput::Url(url),
            RawSourceInput::Text { text, title } => SourceInput::Text { text, title },
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryParams {
    pub kb_id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RestoreParams {
    pub kb_id: String,
    pub commit_sha: String,
}

// ── Task 4: Transaction tools ─────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BeginTxnParams {
    pub kb_id: String,
    pub label: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommitTxnParams {
    pub kb_id: String,
    pub txn: String,
    pub summary: String,
    pub kind: ChangeKind,
    #[serde(default)]
    pub delta: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AbortTxnParams {
    pub kb_id: String,
    pub txn: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    pub kb_id: Option<String>,
    pub query: String,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
    /// Which corpus to search. Defaults to the curated knowledge pages.
    #[serde(default)]
    pub scope: SearchScope,
}

fn default_search_limit() -> usize {
    5
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AppendLogParams {
    pub kb_id: String,
    pub kind: ChangeKind,
    pub summary: String,
    #[serde(default)]
    pub delta: Option<String>,
    /// Opaque handle returned by `kb_begin_txn`. Omit for a standalone commit.
    #[serde(default)]
    pub txn: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SearchHitWithKb {
    pub kb_id: String,
    pub path: String,
    pub score: f32,
    pub snippet: String,
}

// ── Task 5: Active-KB tools ───────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetActiveParams {
    pub kb_id: String,
}

/// ⚠ `kb_id` is **required**, and is deliberately not an `Option` that falls
/// back to the session's primary.
///
/// Every other base-addressing tool that takes an optional id reads; the ones
/// that write all require it, "so a write is never ambiguous" (the rule stated
/// in `instructions.md` and in the multi-KB plan). Deletion is the extreme of
/// that rule: an omitted id would mean "destroy whichever base this session
/// happens to point at", resolved from on-disk pointer state that the Knowledge
/// view or another session may have moved since the model last looked. There is
/// no reading of an absent argument here that is safe to guess.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteBaseParams {
    pub kb_id: String,
}

/// What [`KnowledgeServer::kb_delete_base`] reports back — captured BEFORE the
/// delete, because afterwards there is nothing left to read it off.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DeletedBase {
    pub kb_id: String,
    pub name: String,
    /// Curated knowledge pages the base held. The number the approval card
    /// showed the user, so the model's summary and the card cannot disagree.
    ///
    /// ⚠ `Option`, and `None` when the page tree could not be walked — which is
    /// the only shape that keeps the sentence above true. The card
    /// (`security::knowledge_delete::describe_base`) already fails soft in that
    /// direction and says "its knowledge pages"; reporting `0` here made the
    /// model tell the user "0 knowledge pages removed" about a base whose
    /// contents were simply unreadable. "0 pages" reads as "nothing was lost",
    /// which is the one wrong thing either surface could say.
    pub pages_removed: Option<usize>,
    /// Always `true` — the tool errors rather than reporting a partial delete;
    /// `delete_base_under_locks` rolls its metadata back if any step fails.
    pub deleted: bool,
}

// ── Task 5: Optional-kb_id variants of read-only params ─────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListPagesOptParams {
    pub kb_id: Option<String>,
    #[serde(default)]
    pub path_prefix: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadPageOptParams {
    pub kb_id: Option<String>,
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct KbIdOptParams {
    pub kb_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryOptParams {
    pub kb_id: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

/// Registered so the name keeps dispatching, but never advertised.
///
/// A retired tool is one whose job another tool already does through an
/// argument — `kb_search_raw_sources` is `kb_search { scope: "raw_sources" }`.
/// Dropping the ROUTE as well as the declaration would turn a stored
/// `always allow` grant and every persisted transcript's retry into an
/// unknown-tool error the caller cannot act on, so the route stays and only the
/// advertisement goes.
const RETIRED_TOOL_NAMES: &[&str] = &["kb_search_raw_sources"];

/// How long a caller waits for another call's transaction slot before being
/// told the base is busy.
///
/// The slot is held across a KB-lock acquisition, which is bounded at
/// `KB_WRITE_LOCK_WAIT` (1800s) — so an UNBOUNDED wait here inherits that
/// half hour and adds no deadline of its own. Measured in a live drive: a
/// `kb_write_page` behind an open transaction sat for twenty-one minutes
/// with no error, no progress and nothing in the log after the dispatch
/// line. The tool had not failed; it was queued behind a lock the caller
/// could not see and could not cancel.
///
/// The read deadline, not the write one, and for the same reason #159 gave:
/// this is a PRECONDITION CHECK, not the write itself. Giving up costs the
/// caller a retry and tells them why; waiting out a long ingest costs them
/// the whole turn and tells them nothing.
const SLOT_WAIT: std::time::Duration = std::time::Duration::from_secs(45);

/// Acquire a base's transaction slot, bounded and cancellable.
///
/// Cancellation matters as much as the bound: `Stop` in the GUI cancels the
/// request token, and a caller parked here previously ignored it — the turn
/// looked cancelled while the task went on waiting.
async fn lock_transaction_slot(
    slot: &ActiveKnowledgeTransactionSlot,
    kb_id: &str,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<tokio::sync::OwnedMutexGuard<Option<ActiveKnowledgeTransaction>>, ErrorData> {
    lock_transaction_slot_waiting(slot, kb_id, cancel, SLOT_WAIT).await
}

/// [`lock_transaction_slot`] with the deadline supplied, so a test can assert
/// the WIRING without waiting out the real one.
async fn lock_transaction_slot_waiting(
    slot: &ActiveKnowledgeTransactionSlot,
    kb_id: &str,
    cancel: Option<&tokio_util::sync::CancellationToken>,
    wait: std::time::Duration,
) -> Result<tokio::sync::OwnedMutexGuard<Option<ActiveKnowledgeTransaction>>, ErrorData> {
    let acquire = Arc::clone(slot).lock_owned();
    let bounded = tokio::time::timeout(wait, acquire);
    let guard = match cancel {
        Some(token) => tokio::select! {
            biased;
            () = token.cancelled() => {
                return Err(ErrorData::internal_error(
                    format!("cancelled while waiting for knowledge base '{kb_id}'"),
                    None,
                ))
            }
            result = bounded => result,
        },
        None => bounded.await,
    };
    guard.map_err(|_| {
        ErrorData::internal_error(
            format!(
                "knowledge base '{kb_id}' is busy with another transaction and did not \
                 free up within {}s. Commit or abort the open transaction, then retry.",
                SLOT_WAIT.as_secs()
            ),
            None,
        )
    })
}

#[tool_router(router = tool_router)]
impl KnowledgeServer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            tool_router: Self::tool_router(),
            service: KnowledgeService::new_default()?,
            instructions: include_str!("instructions.md").to_string(),
            transactions: KnowledgeTransactionCoordinator::default(),
        })
    }

    fn session_id_from_context(context: &RequestContext<RoleServer>) -> Option<&str> {
        context.meta.0.get(SESSION_ID_META_KEY)?.as_str()
    }

    fn session_id(context: Option<&RequestContext<RoleServer>>) -> Option<&str> {
        context.and_then(Self::session_id_from_context)
    }

    fn transaction_session_id(context: &RequestContext<RoleServer>) -> Result<String, ErrorData> {
        Self::session_id_from_context(context)
            .filter(|session_id| !session_id.is_empty())
            .map(ToOwned::to_owned)
            .ok_or_else(transaction_unavailable)
    }

    async fn abort_transaction_slot(
        &self,
        kb_id: &str,
        slot: ActiveKnowledgeTransactionSlot,
        required_session_id: Option<&str>,
    ) -> (bool, Option<anyhow::Error>) {
        // Unbounded on purpose: this is the REAPER, and it runs with no request
        // context to cancel against and no caller to return an error to. It also
        // cannot deadlock against the bound below — it is what frees the slot the
        // bounded waiters are waiting on.
        let mut stored = slot.lock().await;
        if stored.as_ref().is_none_or(|active| {
            required_session_id.is_some_and(|session_id| active.session_id != session_id)
        }) {
            return (false, None);
        }
        let Some(active) = stored.take() else {
            return (false, None);
        };
        let kb_root = crate::knowledge::paths::kb_root(self.service.root(), kb_id);
        let result = crate::knowledge::git::GitRepo::open(&kb_root)
            .and_then(|repo| repo.abort_txn(&active.txn));
        (true, result.err())
    }

    async fn abort_transaction_slots(
        &self,
        slots: Vec<(String, ActiveKnowledgeTransactionSlot)>,
        required_session_id: Option<&str>,
    ) -> Result<usize> {
        let mut released = 0;
        let mut errors = Vec::new();
        for (kb_id, slot) in slots {
            let (was_released, error) = self
                .abort_transaction_slot(&kb_id, slot, required_session_id)
                .await;
            released += usize::from(was_released);
            if let Some(error) = error {
                errors.push(format!("{kb_id}: {error:#}"));
            }
        }
        if errors.is_empty() {
            Ok(released)
        } else {
            anyhow::bail!(
                "released {released} abandoned knowledge transaction lock(s), but repository cleanup failed: {}",
                errors.join("; ")
            );
        }
    }

    /// End every transaction owned by a session that is being torn down.
    pub async fn abort_transactions_for_session(&self, session_id: &str) -> Result<usize> {
        anyhow::ensure!(!session_id.is_empty(), "knowledge session id is empty");
        self.abort_transaction_slots(self.transactions.slots(), Some(session_id))
            .await
    }

    /// Supervisor-only recovery for an owner that cannot issue `kb_abort_txn`.
    pub async fn force_abort_transaction(&self, kb_id: &str) -> Result<bool> {
        let Some(slot) = self
            .transactions
            .active
            .get(kb_id)
            .map(|entry| Arc::clone(entry.value()))
        else {
            return Ok(false);
        };
        self.abort_transaction_slots(vec![(kb_id.to_string(), slot)], None)
            .await
            .map(|released| released != 0)
    }

    /// Release every transaction owned by this MCP connection during teardown.
    pub async fn abort_all_transactions(&self) -> Result<usize> {
        self.abort_transaction_slots(self.transactions.slots(), None)
            .await
    }

    fn active_transaction_branch<'a>(
        active: &'a Option<ActiveKnowledgeTransaction>,
        handle: &str,
        session_id: &str,
    ) -> Result<&'a str, ErrorData> {
        active
            .as_ref()
            .filter(|txn| txn.handle == handle && txn.session_id == session_id)
            .map(|txn| txn.txn.branch.as_str())
            .ok_or_else(transaction_unavailable)
    }

    /// Mark a transaction as still in use, so the reaper measures IDLE time
    /// rather than total age. Without this a macro that legitimately runs
    /// longer than [`ABANDONED_TRANSACTION_IDLE`] would have its work rolled
    /// back underneath it.
    fn touch_active_transaction(
        active: &mut Option<ActiveKnowledgeTransaction>,
        handle: &str,
        session_id: &str,
    ) {
        if let Some(txn) = active
            .as_mut()
            .filter(|txn| txn.handle == handle && txn.session_id == session_id)
        {
            txn.touched_at = std::time::Instant::now();
        }
    }

    async fn assert_transaction_admission(
        &self,
        tool: &str,
        kb_id: &str,
        args: Option<&rmcp::model::JsonObject>,
        context: &RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let supports_handle = matches!(
            tool,
            "kb_write_page" | "kb_add_raw_source" | "kb_append_log"
        );
        // `kb_delete_base` joins the no-handle arm (#157). An open transaction
        // holds the base's write lock — the in-process mutex and the on-disk
        // `flock` — for its whole lifetime, so a delete underneath one does not
        // race, it PARKS: `delete_base_async` queues on `lock_existing_kb` for
        // the full 1800s deadline while the caller and every other session wait
        // on a base that is already half-decided. Refusing is also the honest
        // answer — there is uncommitted work on that base, and the owner should
        // commit or abort it before its base is destroyed. There is no `txn`
        // argument to offer, because a transaction cannot outlive the thing it
        // is a transaction on.
        if !supports_handle && !matches!(tool, "kb_restore_state" | "kb_merge" | "kb_delete_base") {
            return Ok(());
        }
        let slot = self.transactions.slot(kb_id);
        let active = lock_transaction_slot(&slot, kb_id, Some(&context.ct)).await?;
        let handle = if supports_handle {
            args.and_then(|args| args.get("txn"))
                .and_then(|txn| txn.as_str())
        } else {
            None
        };
        match handle {
            Some(handle) => {
                let session_id = Self::transaction_session_id(context)?;
                Self::active_transaction_branch(&active, handle, &session_id)?;
                // Still in use, so the reaper's clock restarts here.
                let mut active = active;
                Self::touch_active_transaction(&mut active, handle, &session_id);
                Ok(())
            }
            None if active.is_some() => Err(transaction_unavailable()),
            None => Ok(()),
        }
    }

    /// Issue #56. The capability the daemon admitted this call on, PUBLIC unless
    /// the request meta says otherwise. It *delegates* to
    /// [`crate::knowledge::tier::caller_is_private`] rather than re-reading the
    /// key, so CP1 here and CP4 in `agent_drafter` cannot drift.
    ///
    /// Consumed by the hand-written `call_tool` below (CP1) and by
    /// `kb_create_base` / `kb_import`, the two tools whose subject id does not
    /// exist before the call.
    fn caller_is_private(context: Option<&RequestContext<RoleServer>>) -> bool {
        context
            .map(|c| crate::knowledge::tier::caller_is_private(&c.meta))
            .unwrap_or(false)
    }

    /// Issue #56 DR-26 / Task 50. The caller's **affiliation**, the third axis,
    /// off the second `_meta` key.
    ///
    /// Delegates to [`crate::knowledge::affiliation::caller_affiliation`] for
    /// the reason [`Self::caller_is_private`] delegates to its reader: CP1 here
    /// and CP4 in `agent_drafter` must not drift.
    ///
    /// No context — an in-crate unit test, or a caller with no request — reads
    /// [`CallerAffiliation::Unstated`], which is the restrictive answer and
    /// matches `caller_is_private`'s `false`.
    fn caller_affiliation(context: Option<&RequestContext<RoleServer>>) -> CallerAffiliation {
        context
            .map(|c| crate::knowledge::affiliation::caller_affiliation(&c.meta))
            .unwrap_or(CallerAffiliation::Unstated)
    }

    /// The base this call names, or `None` when it names none (issue #56).
    fn gated_kb_id(
        &self,
        tool: &str,
        args: Option<&rmcp::model::JsonObject>,
        context: Option<&RequestContext<RoleServer>>,
    ) -> Result<Option<String>, ErrorData> {
        if !KB_ID_GATED_TOOLS.contains(&tool) {
            return Ok(None);
        }
        if let Some(id) = args
            .and_then(|a| a.get("kb_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            crate::knowledge::paths::validate_kb_id(id)
                .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
            return Ok(Some(id.to_string()));
        }
        if !KB_PRIMARY_RESOLVING_TOOLS.contains(&tool) {
            // `kb_search` / `kb_search_raw_sources` with no kb_id fan out over
            // the visible set and filter per base (`search_visible_bases`) —
            // Task 10C's fan-out check is per-hit, not all-or-nothing.
            // `kb_export` and the writes REQUIRE kb_id, so an absent one is the
            // tool's own 400 and not ours to pre-empt.
            return Ok(None);
        }
        // Resolve exactly as the tool will (`kb_id_or_primary`), so omitting the
        // kb_id is not the bypass. Its error case — no id and no primary — is
        // the tool's own message and must NOT become a privacy refusal, so
        // `None` falls through and the tool answers.
        let primary = self.primary_kb_for_context(context)?;
        if let Some(id) = primary.as_deref() {
            crate::knowledge::paths::validate_kb_id(id)
                .map_err(|error| ErrorData::invalid_params(error.to_string(), None))?;
        }
        Ok(primary)
    }

    /// The KB twin of `ExtensionManager::assert_extension_reachable` (issue
    /// #56). `Err` is the refusal; `Ok(())` permits.
    ///
    /// Reads the base's stored tier, never the session's set — hiding and
    /// privacy are different questions, and [`Self::kb_id_or_primary`] answers
    /// only the first. That is why its doc comment ("an explicit `kb_id` always
    /// wins and is never filtered against the session's set") stays true and
    /// this check lives above it rather than inside it.
    fn assert_kb_reachable(&self, kb_id: &str, caller: &CallerIdentity) -> Result<(), ErrorData> {
        crate::knowledge::tier::assert_reachable(
            self.service.root(),
            kb_id,
            caller.private,
            &caller.affiliation,
        )
        .map_err(|e| ErrorData::invalid_request(e.to_string(), None))
    }

    /// Whether this caller must be kept away from `kb_id`. The one predicate
    /// behind every *filter* in this file — the fan-outs, the candidate lists
    /// and the two pointer tools — so "omit" and "refuse" cannot disagree about
    /// what is reachable.
    ///
    /// ⚠ **Audit finding 17: this is [`Self::assert_kb_reachable`], negated, and
    /// it must stay that way.** It used to be a *second* spelling — a public
    /// caller conjoined with the tier-only predicate in
    /// [`crate::knowledge::tier`] — which asked only the tier
    /// axis. The consequence was visible to the user: in a chat bound to a model
    /// covered by another institution's agreements, `kb_list_bases` listed the
    /// base (tier says "private caller, fine") while `kb_read_page` on the id it
    /// had just handed over refused it (affiliation says "cross-institutional").
    /// A KB name routinely names a cohort or a study, so the listing was the
    /// leak and the refusal was merely the tell.
    ///
    /// The fix is not "add the affiliation argument here too" — that is how the
    /// two came to disagree in the first place, and a fourth axis would repeat
    /// it. There is now no independent predicate to keep in sync: the barrier
    /// answers, and the filters ask the barrier. `is_err()` and not a re-derived
    /// condition, so the master toggle (DR-15), the tier axis and DR-26's
    /// affiliation axis are read once, in `tier::assert_reachable`, for both the
    /// "omit" decision and the "refuse" decision.
    ///
    /// It follows the toggle now, which it did not before: with privacy tiers
    /// off, `assert_reachable` permits every read, so a listing that still hid
    /// bases was the same inconsistency in the other direction — a name withheld
    /// for content the very next call would hand over in full. DR-15's promise
    /// is that nothing is impacted when the feature is off.
    fn kb_is_out_of_reach(&self, kb_id: &str, caller: &CallerIdentity) -> bool {
        self.assert_kb_reachable(kb_id, caller).is_err()
    }

    fn hidden_kbs_for_session(&self, session_id: Option<&str>) -> Result<Vec<String>, ErrorData> {
        match session_id {
            Some(session_id) => self
                .service
                .get_hidden_for_session_or_persisted(session_id)
                .map_err(into_err),
            None => self.service.get_hidden_persisted().map_err(into_err),
        }
    }

    fn visible_bases_for_session(
        &self,
        session_id: Option<&str>,
        caller: &CallerIdentity,
    ) -> Result<Vec<Manifest>, ErrorData> {
        let hidden = self.hidden_kbs_for_session(session_id)?;
        let hidden = hidden.into_iter().collect::<HashSet<_>>();
        let mut bases = self.service.list_bases().map_err(into_err)?;
        // Issue #56, beside the `hidden` retain deliberately: `kb_list_bases`
        // must OMIT a base this caller may not reach, never redact it — a KB
        // name is user-authored and routinely names a cohort or a study, and an
        // omitted base cannot tempt the model into passing the id explicitly,
        // which is the bypass Task 10C closes.
        bases.retain(|base| {
            !hidden.contains(&base.id) && !self.kb_is_out_of_reach(&base.id, caller)
        });
        Ok(bases)
    }

    fn visible_bases_for_context(
        &self,
        context: Option<&RequestContext<RoleServer>>,
    ) -> Result<Vec<Manifest>, ErrorData> {
        self.visible_bases_for_session(
            Self::session_id(context),
            &CallerIdentity::from_context(context),
        )
    }

    fn search_visible_bases(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        caller: &CallerIdentity,
        scope: SearchScope,
    ) -> Result<Vec<SearchHitWithKb>, ErrorData> {
        let mut hits = Vec::new();
        for base in self.visible_bases_for_session(session_id, caller)? {
            // Issue #56. INSIDE the loop, per base. A guard BEFORE it would make
            // a KB-less search all-or-nothing, so one private base in the
            // session's set would cost the user every other base — and skipping
            // here rather than filtering the hits afterwards is what keeps the
            // private base's index off the disk entirely. The listing above has
            // already dropped it; this is the second reading, on the base we are
            // about to open, so one that was ratcheted in between is still
            // skipped.
            if self.kb_is_out_of_reach(&base.id, caller) {
                continue;
            }
            let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &base.id);
            let kb_hits = crate::knowledge::store::search_with_scope(&kb_root, query, limit, scope)
                .map_err(into_err)?;
            hits.extend(kb_hits.into_iter().map(|hit| SearchHitWithKb {
                kb_id: base.id.clone(),
                path: hit.path,
                score: hit.score,
                snippet: hit.snippet,
            }));
        }
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.kb_id.cmp(&b.kb_id))
                .then_with(|| a.path.cmp(&b.path))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    /// This session's primary knowledge base — the write target for KB-less
    /// mutating calls and the default subject for single-base reads. Resolved
    /// from disk on every call: session file → machine file, returned only
    /// while it names a member of the session's set.
    fn primary_kb_for_session(
        &self,
        session_id: Option<&str>,
    ) -> Result<Option<String>, ErrorData> {
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

    /// Resolve `supplied` kb_id, else this session's primary.
    ///
    /// An explicit `kb_id` always wins and is never filtered against the
    /// session's set — that is how a hidden base (Soul) stays reachable. HIDING
    /// is a tidiness control and that sentence is correct for it.
    ///
    /// ⚠ Issue #56. PRIVACY is not a tidiness control, and it is answered
    /// **above** this function, at CP1 in `call_tool`, not inside it: `kb_search`,
    /// `kb_search_raw_sources`, `kb_export` and all nine writes take `kb_id`
    /// directly and never call this, and it is also how a *write* resolves its
    /// target, so a shared refusal here would report a read error on a write.
    /// What this function does own is the ERROR it produces when there is
    /// neither an id nor a primary — CP1 deliberately falls through in that case
    /// so the tool answers, and that message must not enumerate a base the
    /// caller may not reach.
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
        let caller = CallerIdentity::from_context(context);
        let ids: Vec<String> = self
            .service
            .session_kb_ids(Self::session_id(context))
            .map_err(into_err)?
            .into_iter()
            // Issue #56. `session_kb_ids_unlocked` filters on `hidden` and
            // NOTHING else, and this string is read by the model. Same rule as
            // `visible_bases_for_session`: OMIT. A barrier that refuses a read
            // and then hands over the identifier of the thing it refused is not
            // a barrier — and that identifier is the one argument that makes the
            // explicit-`kb_id` branch reachable. When the filter empties the
            // list, the "this session has no knowledge bases" branch below takes
            // over; an empty `(one of: )` is both useless and a tell.
            //
            // ⚠ Read per id, not once: that is what lets the public bases
            // survive the private one, the same all-or-nothing trap the fan-out
            // filters exist to avoid, in a third place.
            .filter(|id| !self.kb_is_out_of_reach(id, &caller))
            .collect();
        Err(ErrorData::invalid_params(
            if ids.is_empty() {
                "this session has no knowledge bases, so there is nothing to read. \
                 Create one with kb_create_base."
                    .to_string()
            } else {
                format!(
                    "kb_id not supplied and this session has no primary knowledge base. \
                     Pass kb_id explicitly (one of: {}), or call kb_set_active to make one \
                     the primary; that is also where KB-less writes go.",
                    ids.join(", ")
                )
            },
            None,
        ))
    }

    #[tool(
        name = "kb_list_bases",
        description = "List knowledge bases visible to this session. Hidden knowledge bases are omitted from discovery."
    )]
    pub async fn kb_list_bases(
        &self,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let bases = self.visible_bases_for_context(Some(&context))?;
        ok_json(&bases)
    }

    #[tool(
        name = "kb_create_base",
        description = "Create a new knowledge base. Choose its `format` now: it decides how pages \
                       are written and checked for the life of the base, and this build has no \
                       conversion between the two.\n\
                       \n\
                       - `okf` (the default) — the Open Knowledge Format v0.2. Open vocabulary: a \
                       page's `type` is any word that fits, and links are ordinary markdown links. \
                       Pick this for general-purpose memory, retrieval, development and design \
                       notes, project and codebase context, meeting records, personal knowledge — \
                       anything that is not biomedical.\n\
                       - `biookf` — OKF v0.2 plus the BioOKF v0.5 profile: a controlled vocabulary \
                       of 28 entity types and 35 relationship predicates, where every asserted \
                       relationship carries provenance (how the claim is known, what produced it, \
                       and which source page it came from). Pick this for biomedical literature, \
                       curated biology, clinical or genomic knowledge, and for anything meant to \
                       be exchanged with another institution or another BioOKF tool. It costs more \
                       per page and buys a graph other people's tools can read.\n\
                       \n\
                       If the subject is not biomedical, choose `okf`: a biomedical vocabulary \
                       does not make a non-biomedical base stricter, it makes it wrong, because \
                       every page ends up typed `Other`. If you are unsure, ask the user; failing \
                       that choose `okf`, which is the profile a BioOKF base is also valid under."
    )]
    pub async fn kb_create_base(
        &self,
        p: Parameters<CreateBaseParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        // Issue #56. One of exactly TWO tools that take a `RequestContext` for
        // the ratchet, because their subject id is not knowable before the
        // call. Not a raise *before* the write and not one *after* it either:
        // `create_base_as` stamps the tier inside the same root-lock
        // transaction that creates the directory, so there is no window in
        // which a private session's brand-new base reads PUBLIC and no way for
        // a failing stamp to leave a PUBLIC base behind an `Err`. Same shape as
        // `import_brkb`, whose stamp rides in its single store write.
        let m = self
            .service
            .create_base_as(
                &p.id,
                &p.name,
                p.color.as_deref(),
                // Stage 4: the model chooses, and an unparseable choice is an
                // INVALID_PARAMS rather than a silent OKF base — see
                // `CreateBaseParams::format`.
                p.format()?,
                Self::caller_is_private(Some(&context)),
                // Issue #56 DR-26 / Task 50: the third axis is stamped in the
                // same transaction as the tier, by the same argument — see
                // `create_base_as`.
                &Self::caller_affiliation(Some(&context)),
            )
            .map_err(into_err)?;
        ok_json(&m)
    }

    #[tool(
        name = "kb_delete_base",
        description = "Permanently delete a knowledge base: its pages, its raw sources and its \
                       whole git history, plus its registry entry and any selection pointing at \
                       it. There is no undo and no trash — `kb_restore_state` restores a commit \
                       WITHIN a base and cannot bring a deleted base back. Export it first with \
                       `kb_export` if the content might be wanted again.\n\
                       \n\
                       Use this to clean up a base you created that is no longer wanted. If the \
                       user only wants it out of the way, `kb_set_active` moves the primary and \
                       hiding a base removes it from this session without destroying it — prefer \
                       either of those unless the user asked for deletion.\n\
                       \n\
                       `kb_id` is required and is never inferred from the session's primary. The \
                       built-in `soul` base cannot be deleted this way.",
        annotations(
            title = "Delete a knowledge base",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    pub async fn kb_delete_base(
        &self,
        p: Parameters<DeleteBaseParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let kb_id = p.0.kb_id;

        // ⚠ Refused HERE, on the model's surface, and deliberately NOT in
        // `KnowledgeService::delete_base`. Two reasons, and they point the same
        // way:
        //
        // 1. A factory reset (`routes::reset::reset_knowledge`) deletes every
        //    base and recreates Soul. A service-level refusal would break it,
        //    and working around that would mean a second delete path — which is
        //    how a guard ends up applying to the caller who least needs it.
        // 2. The governing asymmetry: a user who insists proceeds past a
        //    warning; an agent never does the same thing automatically. The
        //    Knowledge view's own delete already shows a human "cannot be
        //    undone" confirmation for whichever base they picked. What has no
        //    business happening unattended is a model deciding to destroy the
        //    base that holds what Biorouter knows about this user.
        //
        // Soul is re-seeded empty at the next start (`soul::ensure_soul`), so
        // deleting it is not even durable — it is just a silent erasure of the
        // user's accumulated context wearing the clothes of a cleanup.
        if kb_id == crate::knowledge::service::DEFAULT_PRIMARY_KB_ID {
            return Err(ErrorData::invalid_request(
                format!(
                    "'{}' is Biorouter's built-in knowledge base and cannot be deleted. It is \
                     re-created empty on the next start, so deleting it would erase what \
                     Biorouter has learned about this user without removing anything. To stop \
                     using it in this conversation, point the primary elsewhere with \
                     kb_set_active, or ask the user to hide it in the Knowledge view.",
                    crate::knowledge::service::DEFAULT_PRIMARY_KB_ID
                ),
                None,
            ));
        }

        // Read what the base WAS before it stops existing. `pages_removed` is
        // the same count the approval card showed, so the summary the model
        // reports and the sentence the user approved cannot drift apart.
        let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &kb_id);
        let name = crate::knowledge::manifest::load(&kb_root)
            .map(|m| m.name)
            .unwrap_or_else(|_| kb_id.clone());
        // `.ok()`, never `.unwrap_or(0)`: see `DeletedBase::pages_removed`. An
        // unreadable tree is an unknown count, not an empty base.
        let pages_removed = crate::knowledge::store::list_pages(&kb_root, None)
            .ok()
            .map(|pages| pages.len());

        // The service owns the transaction: staged rename, registry unregister,
        // machine primary, per-session primaries, both hidden sets and the tier
        // record, with a rollback if any step fails. CP1 above has already
        // refused a caller that may not reach this base and refused the call
        // outright if a transaction is open on it.
        //
        // `context.ct` and not `None`: deletion takes the base's write lock,
        // whose deadline is 1800s, so without the request's token Stop would do
        // nothing for half an hour behind a busy base.
        self.service
            .delete_base_async(&kb_id, Some(&context.ct))
            .await
            .map_err(into_err)?;

        ok_json(&DeletedBase {
            kb_id,
            name,
            pages_removed,
            deleted: true,
        })
    }

    #[tool(
        name = "kb_list_pages",
        description = "List knowledge pages in a knowledge base. Omit kb_id to use this session's primary knowledge base. To read a different base, pass its kb_id; you never need to change the primary to read."
    )]
    pub async fn kb_list_pages(
        &self,
        p: Parameters<ListPagesOptParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let kb_id = self.kb_id_or_primary(p.kb_id, Some(&context))?;
        let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &kb_id);
        let pages = crate::knowledge::store::list_pages(&kb_root, p.path_prefix.as_deref())
            .map_err(into_err)?;
        ok_json(&pages)
    }

    #[tool(
        name = "kb_read_page",
        description = "Read a single knowledge page by path. Omit kb_id to use this session's primary knowledge base. To read a different base, pass its kb_id; you never need to change the primary to read."
    )]
    pub async fn kb_read_page(
        &self,
        p: Parameters<ReadPageOptParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let kb_id = self.kb_id_or_primary(p.kb_id, Some(&context))?;
        let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &kb_id);
        let page = crate::knowledge::store::read_page(&kb_root, &p.path).map_err(into_err)?;
        ok_json(&page)
    }

    #[tool(
        name = "kb_validate_page",
        description = "Check a page against its knowledge base's format BEFORE writing it with \
                       kb_write_page. Nothing is written and nothing is rejected: you get back a \
                       list of diagnostics, each with a stable rule id, a severity (error / \
                       warning / info), the page or edge it is about, and a message saying what to \
                       fix.\n\
                       \n\
                       In a **BioOKF** base, call this on every draft. It is where an invented \
                       `type` or `predicate`, a missing provenance triplet, an `object` naming a \
                       page that does not exist, a duplicate `identifier` and a domain/range \
                       violation are caught — one page at a time, while you can still fix them, \
                       instead of at the end of a whole ingest. In an **OKF** base it checks OKF \
                       v0.2 conformance only: a parseable frontmatter block, a non-empty `type`, \
                       footnotes that resolve to a `sources[]` entry, and sources that name a \
                       resource. Retired pre-OKF bases are refused; restart Biorouter to finish \
                       the legacy purge before validating or writing."
    )]
    pub async fn kb_validate_page(
        &self,
        p: Parameters<ValidatePageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let profile = self
            .service
            .require_current_profile(&p.kb_id)
            .map_err(into_err)?;
        // Only BioOKF has cross-document rules, so only BioOKF pays to read the
        // bundle. In OKF mode the page is checked entirely against itself.
        let pages = match profile {
            crate::knowledge::types::KbFormat::Biookf => {
                let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &p.kb_id);
                crate::knowledge::validate::load_bundle(&kb_root).map_err(into_err)?
            }
            _ => Vec::new(),
        };
        let diagnostics = crate::knowledge::validate::validate_page(
            Some(profile),
            p.path.as_deref(),
            &p.content,
            &pages,
        );
        ok_json(&serde_json::json!({
            "kb_id": p.kb_id,
            "path": p.path,
            "format": profile.as_str(),
            // DR-7 keeps this a *producer's* verdict and nothing else: a page
            // that is not `ok` is still read, still rendered and still linked.
            // It is a statement about writing it, made by the one actor DR-7
            // holds to a higher bar.
            "ok": diagnostics.errors() == 0,
            "errors": diagnostics.errors(),
            "warnings": diagnostics.count(crate::knowledge::validate::Severity::Warning),
            "diagnostics": diagnostics,
        }))
    }

    #[tool(
        name = "kb_lint",
        description = "Check a WHOLE knowledge base and report what is wrong with it. Nothing is \
                       written and nothing is rejected. Omit kb_id to use this session's primary \
                       knowledge base.\n\
                       \n\
                       This is `kb_validate_page` at the scale of the base, and it is the only \
                       way to see the findings a single page cannot have: housekeeping (`kb.*` — \
                       orphan pages, declared contradictions, stale sources, links to pages that \
                       do not exist), OKF v0.2 conformance (`okf.*`), and in a BioOKF base the \
                       profile's vocabulary and provenance rules (`biookf.*`), including sources \
                       that are RETRACTED or too weak for the claims resting on them.\n\
                       \n\
                       Run it after writing a batch of pages — it is how you check your own work \
                       — and before exporting or sharing a base.\n\
                       \n\
                       You get back `ok`, counts by severity, and `diagnostics`: `items`, each \
                       with a stable rule id / severity / subject / message, and `total`. **Read \
                       `total`, not `items.len()`** — the list is capped, so a base with hundreds \
                       of findings hands back the most severe ones and tells you how many there \
                       were. Fix a batch and run it again.\n\
                       \n\
                       Fixing is yours to do with kb_write_page; this tool never edits anything. \
                       Retired pre-OKF bases are refused; restart Biorouter to finish the legacy \
                       purge before linting."
    )]
    pub async fn kb_lint(
        &self,
        p: Parameters<KbIdOptParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let kb_id = self.kb_id_or_primary(p.0.kb_id, Some(&context))?;
        let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &kb_id);
        // `macros::lint::scan`, NOT `macros::lint::lint`. Three reasons, and each
        // one on its own decides it:
        //
        // 1. `lint()` RATCHETS at its entry, because its autofix arm writes
        //    pages. Reaching it from here would raise a public base to PRIVATE
        //    because a private chat *looked* at it — permanently, for a call that
        //    committed nothing. See `KB_RATCHETING_TOOLS`, where the same
        //    argument keeps `kb_validate_page` off the list.
        // 2. `lint()`'s autofix arm needs a `Completer`, which an MCP tool has
        //    no way to build: the model calling it IS the provider.
        // 3. `scan` is the deterministic half — pure, synchronous, no LLM — and
        //    it is what produces the diagnostics either way. There is no second
        //    scan to keep in step with this one.
        //
        // The autofix path stays where a caller can be held to it: `biorouter kb
        // lint --fix` and `POST /knowledge/bases/{id}/lint`, both of which name a
        // provider and therefore have a tier to ratchet with.
        let caller = CallerIdentity::from_context(Some(&context));
        let cancel = context.ct;
        // #159: a READ deadline. Lint queues for fairness — so a stream of lints
        // cannot starve a macro — but it must not queue forever behind an OPEN
        // TRANSACTION, which holds this lock for as long as somebody leaves it
        // open. A lint that gives up loses nothing and can be retried.
        let lock = self
            .service
            .lock_existing_kb_for_read(&kb_id, Some(&cancel))
            .await
            .map_err(into_err)?;
        // CP1 ran before this potentially long lock wait. A private writer can
        // finish while lint is queued, so authorize the snapshot scan will read.
        self.assert_kb_reachable(&kb_id, &caller)?;
        let profile = self
            .service
            .require_current_profile(&kb_id)
            .map_err(into_err)?;
        let scan_root = kb_root.clone();
        let scan_cancel = cancel.clone();
        let (_lock, report) = tokio::task::spawn_blocking(move || {
            let report = crate::knowledge::macros::lint::scan_with_cancellation(
                &scan_root,
                Some(&scan_cancel),
            )?;
            Ok::<_, anyhow::Error>((lock, report))
        })
        .await
        .map_err(|error| into_err(anyhow::anyhow!("knowledge lint scan task failed: {error}")))?
        .map_err(into_err)?;
        let diagnostics = &report.diagnostics;
        ok_json(&serde_json::json!({
            "kb_id": kb_id,
            "format": profile.as_str(),
            // DR-7: a producer's verdict about writing, not a statement that
            // anything will stop being read. Nothing here rejects a page.
            "ok": diagnostics.errors() == 0,
            "errors": diagnostics.errors(),
            "warnings": diagnostics.count(crate::knowledge::validate::Severity::Warning),
            "info": diagnostics.count(crate::knowledge::validate::Severity::Info),
            // `items` is capped and `total` is the count BEFORE the cap. Both,
            // never just the first: a truncated list reporting its own length is
            // how "3 errors" gets rendered for a base with four hundred.
            "diagnostics": diagnostics,
            "truncated": diagnostics.truncated(),
            // The four hygiene lists `LintReport` also carries are deliberately
            // NOT repeated here: each entry already appears in `items` as a
            // `kb.*` diagnostic, and sending both would double the payload of
            // every call to say the same thing twice.
        }))
    }

    #[tool(
        name = "kb_write_page",
        description = "Create or overwrite a knowledge page and commit. The path must be under \
                       knowledge/ (e.g. knowledge/<topic>.md) or be index.md/schema.md/log.md; \
                       raw/ holds immutable ingested sources and is read-only. To add or update \
                       a source, use kb_add_raw_source or re-ingest it."
    )]
    pub async fn kb_write_page(
        &self,
        p: Parameters<WritePageParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        // Issue #26: reject contract violations as INVALID_PARAMS (the error
        // taxonomy reads that as invalid_args — "fix the call itself") instead
        // of letting them flow through into_err as an opaque internal error.
        if !crate::knowledge::store::is_writable_page_path(&p.path) {
            return Err(ErrorData::invalid_params(
                format!(
                    "invalid write path {:?}: must start with knowledge/ or be \
                     index.md/schema.md/log.md. {}",
                    p.path,
                    crate::knowledge::store::WRITE_PATH_RECOVERY
                ),
                None,
            ));
        }
        let slot = self.transactions.slot(&p.kb_id);
        let active = lock_transaction_slot(&slot, &p.kb_id, None).await?;
        let txn_branch = match p.txn.as_deref() {
            Some(handle) => Some(
                Self::active_transaction_branch(
                    &active,
                    handle,
                    &Self::transaction_session_id(&context)?,
                )?
                .to_string(),
            ),
            None if active.is_some() => return Err(transaction_unavailable()),
            None => None,
        };
        let _write_guard = if txn_branch.is_none() {
            Some(
                self.service
                    .lock_existing_kb_cancellable(&p.kb_id, Some(&context.ct))
                    .await
                    .map_err(into_err)?,
            )
        } else {
            None
        };
        self.service
            .require_current_profile(&p.kb_id)
            .map_err(into_err)?;
        let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &p.kb_id);
        let sha = crate::knowledge::store::write_page(
            &kb_root,
            &p.path,
            &p.content,
            &p.commit_message,
            txn_branch.as_deref(),
        )
        .map_err(into_err)?;
        // Keep the derived graph cache in sync after a page write. Without
        // this, pages authored from chat (via this tool) never appear in the
        // Knowledge graph view: get_graph returns the empty cache written at
        // create time, and the "Refresh graph" button only re-reads that
        // cache. add_raw_source already rebuilds for the GUI ingest path; do
        // the same here so chat-curated KBs visualize their pages/links.
        if let Err(error) = self.service.rebuild_graph_cache(&p.kb_id) {
            let failure: anyhow::Error = match sha.as_deref() {
                Some(commit_sha) => crate::knowledge::git::KnowledgeWriteFailure::committed(
                    format!("page write to {}", p.path),
                    commit_sha,
                    error,
                )
                .into(),
                None => anyhow::anyhow!(
                    "page {} already matched durable content, but its graph cache could not be refreshed: {error:#}. The cache will be re-derived on its next read",
                    p.path
                ),
            };
            return Err(into_err(failure));
        }
        ok_json(&serde_json::json!({ "commit_sha": sha }))
    }

    #[tool(
        name = "kb_add_raw_source",
        description = "Add a raw source (URL or pasted text), convert to markdown, and classify credibility."
    )]
    pub async fn kb_add_raw_source(
        &self,
        p: Parameters<AddRawSourceParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let slot = self.transactions.slot(&p.kb_id);
        let active = lock_transaction_slot(&slot, &p.kb_id, None).await?;
        let txn_branch = match p.txn.as_deref() {
            Some(handle) => Some(
                Self::active_transaction_branch(
                    &active,
                    handle,
                    &Self::transaction_session_id(&context)?,
                )?
                .to_string(),
            ),
            None if active.is_some() => return Err(transaction_unavailable()),
            None => None,
        };
        let _write_guard = if txn_branch.is_none() {
            Some(
                self.service
                    .lock_existing_kb_cancellable(&p.kb_id, Some(&context.ct))
                    .await
                    .map_err(into_err)?,
            )
        } else {
            None
        };
        self.service
            .require_current_profile(&p.kb_id)
            .map_err(into_err)?;
        let res = self
            .service
            .add_raw_source_cancelled_by(
                &p.kb_id,
                p.source.into(),
                txn_branch.as_deref(),
                Some(&context.ct),
            )
            .await
            .map_err(into_err)?;
        ok_json(&serde_json::json!({
            "source_id": res.source_id,
            "source_md_path": res.source_md_path,
            "meta_path": res.meta_path,
        }))
    }

    #[tool(
        name = "kb_get_graph",
        description = "Return the cached node+edge graph for a knowledge base. Omit kb_id to use this session's primary knowledge base. To read a different base, pass its kb_id; you never need to change the primary to read."
    )]
    pub async fn kb_get_graph(
        &self,
        p: Parameters<KbIdOptParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let kb_id = self.kb_id_or_primary(p.kb_id, Some(&context))?;
        let g = self
            .service
            .get_graph_async(&kb_id)
            .await
            .map_err(into_err)?;
        ok_json(&g)
    }

    #[tool(
        name = "kb_list_history",
        description = "List recent change-log entries from the git history. Omit kb_id to use this session's primary knowledge base. To read a different base, pass its kb_id; you never need to change the primary to read."
    )]
    pub async fn kb_list_history(
        &self,
        p: Parameters<HistoryOptParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let kb_id = self.kb_id_or_primary(p.kb_id, Some(&context))?;
        let h = self
            .service
            .list_history(&kb_id, p.limit)
            .map_err(into_err)?;
        ok_json(&h)
    }

    #[tool(
        name = "kb_restore_state",
        description = "Restore the knowledge folder to a previous commit by creating a new commit on top of HEAD."
    )]
    pub async fn kb_restore_state(
        &self,
        p: Parameters<RestoreParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let slot = self.transactions.slot(&p.kb_id);
        let active = lock_transaction_slot(&slot, &p.kb_id, None).await?;
        if active.is_some() {
            return Err(transaction_unavailable());
        }
        let sha = self
            .service
            .restore_state_async(&p.kb_id, &p.commit_sha, Some(&context.ct))
            .await
            .map_err(into_err)?;
        ok_json(&serde_json::json!({ "ok": true, "new_commit_sha": sha }))
    }

    // ── Task 4: Transaction MCP tools ─────────────────────────────────────────

    #[tool(
        name = "kb_begin_txn",
        description = "Open a knowledge transaction for this session. Returns an opaque txn handle for subsequent write, add-source, append-log, commit, or abort calls."
    )]
    pub async fn kb_begin_txn(
        &self,
        p: Parameters<BeginTxnParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let session_id = Self::transaction_session_id(&context)?;
        let slot = self.transactions.slot(&p.kb_id);
        let mut active = lock_transaction_slot(&slot, &p.kb_id, None).await?;
        if active.is_some() {
            return Err(transaction_unavailable());
        }
        let write_guard = self
            .service
            .lock_existing_kb_cancellable(&p.kb_id, Some(&context.ct))
            .await
            .map_err(into_err)?;
        self.service
            .require_current_profile(&p.kb_id)
            .map_err(into_err)?;
        let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &p.kb_id);
        let repo = crate::knowledge::git::GitRepo::open(&kb_root).map_err(into_err)?;
        let txn = repo.begin_txn(&p.label).map_err(into_err)?;
        let handle = uuid::Uuid::new_v4().to_string();
        *active = Some(ActiveKnowledgeTransaction {
            handle: handle.clone(),
            session_id,
            txn,
            _write_guard: write_guard,
            touched_at: std::time::Instant::now(),
        });
        ok_json(&serde_json::json!({ "txn": handle }))
    }

    #[tool(
        name = "kb_commit_txn",
        description = "Commit this session's opaque transaction handle as one history entry. A handle is consumed by the first commit or abort attempt."
    )]
    pub async fn kb_commit_txn(
        &self,
        p: Parameters<CommitTxnParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let session_id = Self::transaction_session_id(&context)?;
        let slot = self.transactions.slot(&p.kb_id);
        let mut active = lock_transaction_slot(&slot, &p.kb_id, None).await?;
        Self::active_transaction_branch(&active, &p.txn, &session_id)?;
        let active = active.take().ok_or_else(transaction_unavailable)?;
        let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &p.kb_id);
        let repo = crate::knowledge::git::GitRepo::open(&kb_root).map_err(into_err)?;
        let sha = repo
            .commit_txn(&active.txn, p.kind, &p.summary, p.delta.as_deref())
            .map_err(into_err)?;
        ok_json(&serde_json::json!({ "commit_sha": sha }))
    }

    #[tool(
        name = "kb_abort_txn",
        description = "Abort this session's opaque transaction handle and restore the working tree. A handle is consumed by the first commit or abort attempt."
    )]
    pub async fn kb_abort_txn(
        &self,
        p: Parameters<AbortTxnParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let session_id = Self::transaction_session_id(&context)?;
        let slot = self.transactions.slot(&p.kb_id);
        let mut active = lock_transaction_slot(&slot, &p.kb_id, None).await?;
        Self::active_transaction_branch(&active, &p.txn, &session_id)?;
        let active = active.take().ok_or_else(transaction_unavailable)?;
        let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &p.kb_id);
        let repo = crate::knowledge::git::GitRepo::open(&kb_root).map_err(into_err)?;
        repo.abort_txn(&active.txn).map_err(into_err)?;
        ok_json(&serde_json::json!({ "ok": true }))
    }

    #[tool(
        name = "kb_search",
        description = "BM25 full-text search over a knowledge base. Omit kb_id to search all visible knowledge bases. `scope` chooses the corpus: \"knowledge\" (the default) searches the curated pages; \"raw_sources\" searches the original source markdown only, and is for the rare case where the user specifically asks for raw/original/source-document evidence instead of the curated graph; \"all\" searches both."
    )]
    pub async fn kb_search(
        &self,
        p: Parameters<SearchParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let scope = p.scope;
        let hits = if let Some(kb_id) = p.kb_id {
            let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &kb_id);
            crate::knowledge::store::search_with_scope(&kb_root, &p.query, p.limit, scope)
                .map_err(into_err)?
                .into_iter()
                .map(|hit| SearchHitWithKb {
                    kb_id: kb_id.clone(),
                    path: hit.path,
                    score: hit.score,
                    snippet: hit.snippet,
                })
                .collect::<Vec<_>>()
        } else {
            self.search_visible_bases(
                &p.query,
                p.limit,
                Self::session_id(Some(&context)),
                &CallerIdentity::from_context(Some(&context)),
                scope,
            )?
        };
        ok_json(&hits)
    }

    /// ⚠ **Registered but not advertised** — see [`RETIRED_TOOL_NAMES`]. It was
    /// `kb_search` with `scope` pinned, and it advertised an
    /// `include_raw_sources` field it then ignored. The route stays so persisted
    /// transcripts and stored grants keep working; new calls go through
    /// `kb_search { scope: "raw_sources" }`.
    #[tool(
        name = "kb_search_raw_sources",
        description = "Retired: call kb_search with scope=\"raw_sources\" instead."
    )]
    pub async fn kb_search_raw_sources(
        &self,
        p: Parameters<SearchParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let hits = if let Some(kb_id) = p.kb_id {
            let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &kb_id);
            crate::knowledge::store::search_with_scope(
                &kb_root,
                &p.query,
                p.limit,
                SearchScope::RawSources,
            )
            .map_err(into_err)?
            .into_iter()
            .map(|hit| SearchHitWithKb {
                kb_id: kb_id.clone(),
                path: hit.path,
                score: hit.score,
                snippet: hit.snippet,
            })
            .collect::<Vec<_>>()
        } else {
            self.search_visible_bases(
                &p.query,
                p.limit,
                Self::session_id(Some(&context)),
                &CallerIdentity::from_context(Some(&context)),
                SearchScope::RawSources,
            )?
        };
        ok_json(&hits)
    }

    #[tool(
        name = "kb_append_log",
        description = "Append a structured entry to the KB change log and commit it."
    )]
    pub async fn kb_append_log(
        &self,
        p: Parameters<AppendLogParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let slot = self.transactions.slot(&p.kb_id);
        let active = lock_transaction_slot(&slot, &p.kb_id, None).await?;
        let txn_branch = match p.txn.as_deref() {
            Some(handle) => Some(
                Self::active_transaction_branch(
                    &active,
                    handle,
                    &Self::transaction_session_id(&context)?,
                )?
                .to_string(),
            ),
            None if active.is_some() => return Err(transaction_unavailable()),
            None => None,
        };
        let _write_guard = if txn_branch.is_none() {
            Some(
                self.service
                    .lock_existing_kb_cancellable(&p.kb_id, Some(&context.ct))
                    .await
                    .map_err(into_err)?,
            )
        } else {
            None
        };
        self.service
            .require_current_profile(&p.kb_id)
            .map_err(into_err)?;
        let kb_root = crate::knowledge::paths::kb_root(self.service.root(), &p.kb_id);
        let sha = crate::knowledge::log::append(
            &kb_root,
            p.kind,
            &p.summary,
            p.delta.as_deref(),
            txn_branch.as_deref(),
        )
        .map_err(into_err)?;
        ok_json(&serde_json::json!({ "ok": true, "commit_sha": sha }))
    }

    // ── The session's knowledge-base set and its primary ──────────────────────

    /// The ids in `selection` this caller may reach — the **view**, never the
    /// store.
    ///
    /// ⚠ The filter is HERE and not in `service::selection` or
    /// `apply_selection_unlocked`. Those two feed `repair_decision`, which
    /// promotes the primary to `next_ids.first()` and then **writes it to
    /// disk**. Filtering the service would therefore make a public model's
    /// `kb_get_active` silently re-point the user's primary at the
    /// lexicographically first public base — a persisted, machine-wide change as
    /// a side effect of a read, and one the Knowledge view would then show. The
    /// store keeps one truth; the two model-facing tools render a filtered
    /// projection of it.
    fn visible_kb_ids(
        &self,
        selection: &crate::knowledge::service::KbSelection,
        caller: &CallerIdentity,
    ) -> Vec<String> {
        selection
            .kb_ids
            .iter()
            .filter(|id| !self.kb_is_out_of_reach(id, caller))
            .cloned()
            .collect()
    }

    /// Body of `kb_set_active`, split out so it can be unit-tested without
    /// fabricating a `RequestContext`.
    fn set_primary_json(
        &self,
        session_id: Option<&str>,
        kb_id: &str,
        caller: &CallerIdentity,
    ) -> Result<serde_json::Value, ErrorData> {
        crate::knowledge::paths::validate_kb_id(kb_id)
            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
        // Issue #56. Membership is decided against the set THIS CALLER can see.
        // A private base is not "refused" — it is NOT A MEMBER, byte-identical
        // to the answer an id that does not exist gets. Refusing it by name
        // would confirm it exists, in a politer sentence.
        let selection = self.service.selection(session_id).map_err(into_err)?;
        let visible = self.visible_kb_ids(&selection, caller);
        if !visible.iter().any(|id| id == kb_id) {
            return Err(ErrorData::invalid_params(
                not_a_member(kb_id, &visible, session_id),
                None,
            ));
        }
        let selection = self
            .service
            .set_selection(
                session_id,
                None,
                crate::knowledge::service::PrimaryUpdate::Set(kb_id),
            )
            // Pre-checked above, so an `Err` here is a concurrent hide (or I/O).
            // Answer with OUR list either way: `apply_selection_unlocked`'s
            // message is built from `next_ids` — the WHOLE set — and would put
            // every private id into a public caller's error on a race.
            .map_err(|e| {
                tracing::warn!("kb_set_active: {e:#}");
                ErrorData::invalid_params(not_a_member(kb_id, &visible, session_id), None)
            })?;
        Ok(self.selection_value(&selection, caller, true))
    }

    /// Body of `kb_get_active`.
    fn selection_json(
        &self,
        session_id: Option<&str>,
        caller: &CallerIdentity,
    ) -> Result<serde_json::Value, ErrorData> {
        let selection = self.service.selection(session_id).map_err(into_err)?;
        Ok(self.selection_value(&selection, caller, false))
    }

    fn selection_value(
        &self,
        selection: &crate::knowledge::service::KbSelection,
        caller: &CallerIdentity,
        ok: bool,
    ) -> serde_json::Value {
        let kb_ids = self.visible_kb_ids(selection, caller);
        // Issue #56. The POINTER is metadata too, and it is the single id that
        // makes the explicit-`kb_id` branch usable without guessing. A primary
        // the caller may not reach reads `null` — truthful for this caller (it
        // has no write target it can use) and the same OMISSION rule
        // `kb_list_bases` takes. `active_kb` is the deprecated mirror and must
        // move with it; filtering two of the three fields is the natural
        // half-fix and this is the field it forgets.
        let primary = selection
            .primary_kb
            .as_ref()
            .filter(|id| kb_ids.iter().any(|visible| visible == *id))
            .cloned();
        let mut v = serde_json::json!({
            "primary_kb": primary.clone(),
            "knowledge_bases": kb_ids,
            // Deprecated mirror of `primary_kb`, kept for one release so
            // anything that learned the old key keeps working.
            "active_kb": primary,
        });
        if ok {
            v["ok"] = serde_json::Value::Bool(true);
        }
        v
    }

    #[tool(
        name = "kb_set_active",
        description = "Make one knowledge base this session's primary: the base that KB-less writes land in and that single-base reads default to. It does not change what you can search: kb_search with no kb_id already covers every knowledge base in this session, tagging each hit with its kb_id. To read or write another base, pass its kb_id; do not switch the primary to get at it. The base must be one of this session's knowledge bases."
    )]
    pub async fn kb_set_active(
        &self,
        p: Parameters<SetActiveParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let v = self.set_primary_json(
            Self::session_id(Some(&context)),
            &p.0.kb_id,
            &CallerIdentity::from_context(Some(&context)),
        )?;
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
        let v = self.selection_json(
            Self::session_id(Some(&context)),
            &CallerIdentity::from_context(Some(&context)),
        )?;
        ok_json(&v)
    }

    #[tool(
        name = "kb_export",
        description = "Export a knowledge base to a .brkb archive file on disk. Returns the absolute path written."
    )]
    pub async fn kb_export(
        &self,
        p: Parameters<ExportArchiveParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        // The READ deadline. An export READS the base — the archive is written
        // outside it — and the lock is here only to keep a writer from tearing
        // the snapshot. Same lock and the same exclusion either way; the only
        // thing that changes is how long this waits before giving the caller an
        // answer. On the write deadline it parked for THIRTY MINUTES behind an
        // open transaction, which is the same defect the merge preview had.
        //
        // ⚠ Still `None` for the cancellation token, because this signature has
        // no `context` to take one from. A caller who cancels therefore waits
        // out the deadline rather than the turn — much less bad at 45s than at
        // 1800s, but not fixed. Threading the token here is the real repair.
        let _lock = self
            .service
            .lock_existing_kb_for_read(&p.kb_id, None)
            .await
            .map_err(into_err)?;
        let bytes = self.service.export_brkb(&p.kb_id).map_err(into_err)?;
        // Issue #56, decision (2b). A MODEL's export of a PRIVATE base is not
        // written where the model asked; it goes to `<knowledge-root>/.exports/`.
        //
        // ⚠ WHAT THIS IS AND IS NOT, because the plan's own wording has been
        // amended and the stale half is the appealing one. It is NOT a barrier.
        // The original argument — "`.exports/` is inside DR-14 deny root #2, so
        // the same kernel deny that hides the base hides the artifact" — depends
        // on a read-deny that DR-17 **descoped for v1**; AR-8 says so in as many
        // words ("Withdrawn: the claim that a model's export of a private base
        // lands somewhere a public session cannot read... Task 10A still forces
        // the export location, and it is now a provenance control rather than a
        // barrier"), and DR-17's accepted risk 4 names exports specifically. In
        // v1 nothing stops a public session's shell from reading this file, and
        // the tool reports the path it wrote.
        //
        // What it DOES buy, and why the rule stays: every model-made archive of
        // a private base lands in one known place, beside the base it came from
        // and inside the tree the user already treats as their knowledge store,
        // instead of scattered wherever a model chose. That is what makes the
        // whole set of them findable — by the user today, and by the read-deny
        // if DR-14 is ever un-descoped, with no change here. Keeping it also
        // keeps `.brkb` archives from being the one artifact whose location a
        // model picks, which is the shape the laundering path used.
        //
        // The directory name is a DOTFILE on purpose: see
        // `paths::MODEL_EXPORT_DIR` — a plain `exports/` is a legal kb id, so a
        // session could create the base `exports` and collect every private
        // archive inside a public base's own tree.
        //
        // Scoped to PRIVATE bases on purpose: relocating every model export
        // would break `kb_export` as a feature. And it lives HERE rather than in
        // `KnowledgeService::export_brkb`, because that function also serves the
        // user's own download from the Knowledge view, which this rule must not
        // touch.
        //
        // The tier is read BEFORE `dest_path` is honoured — a write-then-move
        // would leave a complete copy of a private knowledge base at the path
        // the model chose for however long the copy takes.
        //
        // DR-15's master opt-out. A direct read of the process-global rather
        // than a `CallCapability` (this crate cannot see `biorouter`), and its
        // own read rather than one inherited from `assert_reachable`: forcing
        // the location is a decision over `is_private` directly, not a barrier.
        let dest = if crate::privacy_toggle::privacy_tiers_enabled()
            && crate::knowledge::tier::is_private(self.service.root(), &p.kb_id)
        {
            crate::knowledge::paths::model_export_dir(self.service.root())
                .join(format!("{}.brkb", p.kb_id))
        } else {
            match p.dest_path {
                Some(path) => std::path::PathBuf::from(path),
                None => std::env::temp_dir().join(format!("{}.brkb", p.kb_id)),
            }
        };
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| into_err(anyhow::anyhow!("create export dir: {e}")))?;
        }
        std::fs::write(&dest, &bytes).map_err(|e| into_err(anyhow::anyhow!("write .brkb: {e}")))?;
        ok_json(&serde_json::json!({
            "kb_id": p.kb_id,
            "path": dest.to_string_lossy(),
            "bytes": bytes.len(),
        }))
    }

    #[tool(
        name = "kb_import",
        description = "Import a .brkb archive file from disk as a new knowledge base. Returns the new knowledge base id."
    )]
    pub async fn kb_import(
        &self,
        p: Parameters<ImportArchiveParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = p.0;
        let bytes = crate::knowledge::brkb::read_archive_path(std::path::Path::new(&p.src_path))
            .map_err(into_err)?;
        // Issue #56. The second of exactly TWO tools that take a
        // `RequestContext`: the new base's id is chosen by `brkb::import`'s
        // collision loop, so it is not knowable before the call. The importer's
        // tier and the archive's marker are a disjunction inside `import_brkb`
        // — the marker can only raise, never lower — and because the loop
        // always lands on a FRESH id, classifying there can never re-tier an
        // existing base.
        // Issue #56 DR-26 / Task 50: and the archive's own owners are unioned
        // with this caller's inside `import_brkb`, which is what stops
        // `kb_export` + `kb_import` from being a way to strip an institution's
        // claim while both endpoints stay Private and no gate fires.
        let new_id = self
            .service
            .import_brkb(
                &bytes,
                Self::caller_is_private(Some(&context)),
                &Self::caller_affiliation(Some(&context)),
            )
            .map_err(into_err)?;
        ok_json(&serde_json::json!({ "imported_kb_id": new_id }))
    }

    #[tool(
        name = "kb_merge_preview",
        description = "Preview merging one knowledge base into another WITHOUT writing anything. Reports what would be carried over, what would be renamed because its identifier or path collides, which raw sources are already present (matched by content hash) and would be deduped, and what the destination's privacy tier and owning institutions would become. Call this before kb_merge: a merge is the least reversible operation here, and restoring afterwards restores the whole base, not one page."
    )]
    pub async fn kb_merge_preview(
        &self,
        p: Parameters<MergeParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.merge(p.0, true, &context).await
    }

    #[tool(
        name = "kb_merge",
        description = "Merge one knowledge base into another. The destination is canonical: its identifiers, paths and raw sources always win, and an incoming page whose identifier already exists is RENAMED rather than combined with it — every reference to it is repointed so nothing dangles. Raw sources already present are deduped by content hash. The source base is only read and is left unchanged. The whole merge is one transaction: on any failure the destination is untouched. Run kb_merge_preview first."
    )]
    pub async fn kb_merge(
        &self,
        p: Parameters<MergeParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        self.merge(p.0, false, &context).await
    }

    /// Both merge tools, which differ only in whether they write.
    ///
    /// ⚠ **Two tools and not one `dry_run` argument**, and DR-8's corollary is
    /// the reason: `KB_RATCHETING_TOOLS` is a set of tool NAMES, so "ratchets
    /// when `dry_run` is false" is unsayable in it, and a row that is true half
    /// the time is a row a reader has to open the tool to understand. A preview
    /// writes nothing and must not raise a base's tier permanently because a
    /// private chat *looked*; the merge writes content and must. Narrowing each
    /// tool until its ratchet decision is a constant is what DR-8 asks for, and
    /// it is exactly why `kb_lint` exposes only its read-only half.
    async fn merge(
        &self,
        p: MergeParams,
        dry_run: bool,
        context: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // The SOURCE barrier lives in `merge_bases` alongside the destination's,
        // so the CLI and the HTTP surface take it too. CP1 has already cleared
        // `kb_id` by the time this runs; asking again there is free and asking
        // for `source_kb_id` is the half CP1 structurally cannot do.
        let caller = CallerIdentity::from_context(Some(context));
        let transaction_slot = (!dry_run).then(|| self.transactions.slot(&p.kb_id));
        let transaction_state = match transaction_slot.as_ref() {
            Some(slot) => Some(lock_transaction_slot(slot, &p.kb_id, Some(&context.ct)).await?),
            None => None,
        };
        if transaction_state
            .as_ref()
            .is_some_and(|active| active.is_some())
        {
            return Err(transaction_unavailable());
        }
        let report = self
            .service
            .merge_bases_cancellable(
                &p.kb_id,
                &p.source_kb_id,
                &crate::knowledge::merge::MergeAuthority::Model(&caller.kb_caller()),
                dry_run,
                // A merge holds TWO KB locks at the write deadline. Without the
                // request's token, Stop did nothing for up to 2 x 1800s — which
                // is how a live drive stalled on `kb_merge`.
                Some(&context.ct),
            )
            .await
            .map_err(into_err)?;
        ok_json(&report)
    }
}

impl ServerHandler for KnowledgeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: "biorouter-knowledge".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                title: None,
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(self.instructions.clone()),
            ..Default::default()
        }
    }

    /// Verbatim what `#[tool_handler]` generated
    /// (`rmcp-macros-0.14.0/src/tool_handler.rs`); re-check that file when
    /// bumping rmcp.
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, ErrorData> {
        Ok(rmcp::model::ListToolsResult {
            tools: self
                .tool_router
                .list_all()
                .into_iter()
                .filter(|tool| !RETIRED_TOOL_NAMES.contains(&tool.name.as_ref()))
                .collect(),
            meta: None,
            next_cursor: None,
        })
    }

    /// Issue #56, design §9.3 B4 as ruled. ONE seam for every `kb_*` tool,
    /// including handlers that do not otherwise need caller metadata. The
    /// transaction-aware writes now take a `RequestContext` for session binding,
    /// but the privacy decision remains here so new write paths cannot bypass it.
    ///
    /// This is `#[tool_handler]`'s generated body plus the gate: the last two
    /// statements are exactly what the macro emitted.
    async fn call_tool(
        &self,
        mut request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        // Issue #56 DR-26 / Task 50 Step 1. BOTH axes off the SAME request meta,
        // at the same instant, so the two halves of one caller's identity cannot
        // come from two reads — and, since audit finding 17, they travel as one
        // value so a callee cannot ask half the question.
        let caller = CallerIdentity::from_context(Some(&context));
        let name = request.name.to_string();

        // #157. Roll back transactions nothing has touched for a long time,
        // BEFORE this call goes looking for a lock. An open transaction holds
        // the base's write lock, and the only two reapers that existed fired on
        // teardown, so one abandoned by a live session held its base until the
        // process exited. Doing it here rather than on a timer means no
        // background task to own, and the check runs exactly when it matters:
        // just before somebody would otherwise queue behind it.
        //
        // Cheap in the normal case — `abandoned()` walks the open transactions,
        // which is almost always none, and never blocks on a busy slot.
        let abandoned = self.transactions.abandoned().await;
        if !abandoned.is_empty() {
            let ids: Vec<String> = abandoned.iter().map(|(kb_id, _)| kb_id.clone()).collect();
            match self.abort_transaction_slots(abandoned, None).await {
                Ok(released) if released > 0 => tracing::warn!(
                    "knowledge: rolled back {released} abandoned transaction(s) on {ids:?} \
                     after {}s idle; the base(s) were holding a write lock (see issue #157)",
                    ABANDONED_TRANSACTION_IDLE.as_secs()
                ),
                Ok(_) => {}
                // Never fail the caller's tool because cleanup of somebody
                // else's transaction failed; it will be retried next call.
                Err(error) => {
                    tracing::warn!(
                        "knowledge: could not roll back an abandoned transaction: {error:#}"
                    )
                }
            }
        }

        // DR-33, design §2.6: the private bases this call reads, decided BEFORE
        // it runs (and before a write's raise), so a public base a private
        // write is about to raise is not counted as private content the caller
        // was given. Only a private caller can be given any.
        let mut private_sources: Vec<String> = Vec::new();
        let gated = self.gated_kb_id(&name, request.arguments.as_ref(), Some(&context))?;
        if gated.is_none() && caller.private && KB_VISIBLE_SET_TOOLS.contains(&name.as_str()) {
            private_sources = self.private_bases_in_view(Some(&context), &caller);
        }
        if let Some(kb_id) = gated {
            // Issue #56, Task 10C. The read half of the ruling, and the reason
            // this is a hand-written `call_tool`: `kb_search`'s explicit-`kb_id`
            // branch joined `kb_root(root, &kb_id)` and searched it, and six
            // more read paths did the same. ONE line, above the router and above
            // the raise, covers all sixteen — including the ones that resolve
            // an ABSENT id to the session's primary, because `gated_kb_id`
            // resolves it exactly as the tool will.
            self.assert_kb_reachable(&kb_id, &caller)?;
            self.assert_transaction_admission(&name, &kb_id, request.arguments.as_ref(), &context)
                .await?;
            if caller.private {
                private_sources =
                    self.private_bases_named(&name, &kb_id, request.arguments.as_ref());
            }
            if KB_RATCHETING_TOOLS.contains(&name.as_str()) {
                // BEFORE the write: a raise that only lands on success leaves
                // content in a base whose tier never moved if the write panics
                // or the process dies mid-commit. The failure direction of an
                // over-raise is a badge the user can see; the failure direction
                // of an under-raise is silent.
                //
                // Issue #56 DR-26 / Task 50 Step 1. BOTH axes, in one call under
                // one lock: a write that raised the tier and not the affiliation
                // would put an institution's content into a base no institution
                // is recorded as owning, which reads as unclaimed and is
                // therefore reachable from every other institution's model.
                // `every_tool_that_ratchets_the_tier_also_records_the_callers_institution`
                // drives every ratcheting tool through this seam and pins it.
                self.service
                    .raise_tier_and_affiliation_async(&kb_id, caller.private, &caller.affiliation)
                    .await
                    .map_err(into_err)?;
            }
            // Issue #56, review round 5. PIN what was checked. Without this the
            // tool resolves the base a SECOND time on the far side of the
            // `.await` below — from the same argument, and for the four
            // `KB_PRIMARY_RESOLVING_TOOLS` from on-disk pointer state that any
            // other session or the Knowledge view may have moved in between.
            // Two resolutions with the barrier between them is a TOCTOU, and it
            // also let the two disagree with no race at all: `gated_kb_id`
            // normalises with `str::trim` and `kb_id_or_primary` does not.
            //
            // `kb_id_or_primary`'s "an explicit `kb_id` always wins" contract is
            // unchanged — this makes CP1's answer the explicit one, so the tool
            // takes its early return instead of asking the disk again. Tools
            // whose id CP1 left unresolved (a KB-less `kb_search` fan-out, a
            // write with no id at all) never reach here.
            request
                .arguments
                .get_or_insert_with(Default::default)
                .insert("kb_id".to_string(), serde_json::Value::String(kb_id));
        }

        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        let mut result = self.tool_router.call(tcc).await?;
        crate::knowledge::tier::mark_private_content(&mut result, &private_sources);
        Ok(result)
    }
}

impl KnowledgeServer {
    /// The private bases among those a call naming `kb_id` reads: that base,
    /// and a merge's source (DR-33, design §2.6). Read before the call runs.
    fn private_bases_named(
        &self,
        tool: &str,
        kb_id: &str,
        args: Option<&rmcp::model::JsonObject>,
    ) -> Vec<String> {
        let root = self.service.root();
        let mut named = vec![kb_id.to_string()];
        if matches!(tool, "kb_merge" | "kb_merge_preview") {
            if let Some(source) = args
                .and_then(|a| a.get("source_kb_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| crate::knowledge::paths::validate_kb_id(s).is_ok())
            {
                named.push(source.to_string());
            }
        }
        crate::knowledge::tier::private_among(root, named)
    }

    /// The private bases in this caller's visible set, for the tools that read
    /// across it. A set that cannot be read is doubt, and doubt marks: the
    /// tool may still answer from it.
    fn private_bases_in_view(
        &self,
        context: Option<&RequestContext<RoleServer>>,
        caller: &CallerIdentity,
    ) -> Vec<String> {
        let root = self.service.root();
        match self.visible_bases_for_session(Self::session_id(context), caller) {
            Ok(bases) => {
                crate::knowledge::tier::private_among(root, bases.into_iter().map(|base| base.id))
            }
            Err(_) => vec!["unknown".to_string()],
        }
    }
}

fn ok_json<T: Serialize>(v: &T) -> Result<CallToolResult, ErrorData> {
    let text = serde_json::to_string_pretty(v)
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

fn into_err(e: anyhow::Error) -> ErrorData {
    if let Some(failure) = e.downcast_ref::<crate::knowledge::git::KnowledgeWriteFailure>() {
        return ErrorData::internal_error(
            format!("{e:#}"),
            Some(serde_json::json!({
                "phase": failure.phase.as_str(),
                "commit_sha": failure.commit_sha.as_deref(),
            })),
        );
    }
    if e.downcast_ref::<crate::knowledge::service::LegacyKnowledgeBaseUnsupported>()
        .is_some()
        || e.downcast_ref::<crate::knowledge::service::LegacyKnowledgeArchiveUnsupported>()
            .is_some()
        || e.downcast_ref::<crate::knowledge::service::LegacyKnowledgeRestoreUnsupported>()
            .is_some()
        || e.downcast_ref::<crate::knowledge::brkb::InvalidKnowledgeArchive>()
            .is_some()
    {
        return ErrorData::invalid_request(e.to_string(), None);
    }
    ErrorData::internal_error(format!("{e:#}"), None)
}

fn transaction_unavailable() -> ErrorData {
    ErrorData::invalid_request(TRANSACTION_UNAVAILABLE, None)
}

/// "That is not one of your knowledge bases", built from the ids the caller may
/// see. ONE sentence for a base that does not exist and for one that is private:
/// telling them apart is the leak (issue #56).
///
/// Deliberately a **verbatim** mirror of `apply_selection_unlocked`'s two
/// branches (`service.rs`), including its session/no-session vocabulary split,
/// so that moving the decision up a layer does not invent a second message the
/// model can tell apart from the old one.
fn not_a_member(kb_id: &str, visible: &[String], session_id: Option<&str>) -> String {
    let available = if visible.is_empty() {
        "none".to_string()
    } else {
        visible.join(", ")
    };
    match session_id {
        Some(_) => format!(
            "knowledge base '{kb_id}' is not one of this session's knowledge bases \
             ({available}). Add it to the session first, or pass kb_id explicitly to read it once."
        ),
        None => format!(
            "knowledge base '{kb_id}' is not available ({available}): it does not exist, or it \
             is hidden."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::git::GitRepo;
    // Every page these tests write goes through the one fixture builder, so
    // that when the page format tightens the change lands there and not in
    // twenty tests that are about tiers and paths (DR-19).
    use crate::knowledge::page_fixtures::valid_page;
    use std::path::{Path, PathBuf};

    /// The knowledge instructions must teach the agent to consult the built-in
    /// Soul KB for personal context and that a hidden KB (which Soul may be) is
    /// reachable only by explicit kb_id — otherwise the agent never personalises
    /// from Soul because the default cross-base search skips hidden bases.
    #[test]
    fn instructions_cover_soul_and_hidden_kb_access() {
        let instructions = include_str!("instructions.md");
        assert!(
            instructions.contains("Soul") && instructions.contains("kb_id=\"soul\""),
            "instructions must name the Soul KB and how to search it"
        );
        assert!(
            instructions.contains("hidden") && instructions.to_lowercase().contains("explicit"),
            "instructions must explain searching a hidden KB by explicit kb_id"
        );
    }

    #[test]
    fn committed_write_failures_include_the_durable_phase_and_sha_on_the_wire() {
        let error = into_err(
            crate::knowledge::git::KnowledgeWriteFailure::committed(
                "knowledge transaction",
                "abc123",
                anyhow::anyhow!("injected cleanup failure"),
            )
            .into(),
        );
        let data = error.data.as_ref().expect("phase-aware error data");
        assert_eq!(data["phase"], "committed");
        assert_eq!(data["commit_sha"], "abc123");
        assert!(error.message.contains("must not be retried"), "{error:?}");
    }

    /// A private caller with no stated affiliation — what the unit tests below
    /// mean by "the unfiltered view".
    ///
    /// `Unstated` and not `Local` for the reason `call_tool_as_session` gives
    /// for its own default: `Local` clears every institutional base, so a future
    /// affiliation assertion written against this helper would pass vacuously.
    fn private_caller() -> CallerIdentity {
        CallerIdentity {
            private: true,
            affiliation: CallerAffiliation::Unstated,
        }
    }

    fn server_with_root(root: std::path::PathBuf) -> KnowledgeServer {
        let service = KnowledgeService::new(root);
        KnowledgeServer {
            tool_router: KnowledgeServer::tool_router(),
            service,
            instructions: String::new(),
            transactions: KnowledgeTransactionCoordinator::default(),
        }
    }

    /// Regression (pre-existing, not introduced by the merge): the deleted
    /// process-local active-KB cache was one `Option<String>` for the **whole
    /// KnowledgeServer process**. `kb_set_active` wrote it alongside the
    /// session file and `active_kb_for_context` consulted it for any session
    /// that had no file of its own — so one chat's choice silently became
    /// every other chat's write target inside one daemon, and it was never
    /// invalidated on rename or delete.
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
        // Assembled at runtime: spelling the identifier literally anywhere in
        // this file — including in this test — would make the guard pass
        // vacuously the moment somebody re-introduced the struct.
        let banned = concat!("Active", "KbState");
        assert!(
            !src.contains(banned),
            "primary resolution must read the service, not a process-local cache"
        );
    }

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
            "an explicit kb_id always wins; that is how a base outside the set is reached"
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

        // Task 10C: `true` at all four sites — this test is about HIDING, not
        // privacy, and a private caller sees the unfiltered set, so nothing here
        // moves. Its refusal assertion below is also the check that
        // `not_a_member` really is a verbatim mirror of the service's sentence:
        // if the spelling drifted, `alpha, beta` stops matching.
        let v = server.set_primary_json(Some("session-a"), "beta", &private_caller())?;
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
            .set_primary_json(Some("session-a"), "gamma", &private_caller())
            .expect_err("gamma is not in this session");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("gamma") && err.message.contains("alpha, beta"),
            "got: {}",
            err.message
        );

        let err = server
            .set_primary_json(Some("session-a"), "no-such-kb", &private_caller())
            .expect_err("a base that does not exist can never be primary");
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);

        assert_eq!(
            server.selection_json(Some("session-a"), &private_caller())?["primary_kb"],
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

    #[test]
    fn list_bases_hides_session_hidden_kbs() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let server = server_with_root(tmp.path().to_path_buf());
        server.service.create_base("visible", "Visible", None)?;
        server.service.create_base("hidden", "Hidden", None)?;
        server
            .service
            .set_hidden_for_session("session-a", &["hidden".to_string()])?;

        let visible = server.visible_bases_for_session(Some("session-a"), &private_caller())?;
        let ids = visible.into_iter().map(|base| base.id).collect::<Vec<_>>();
        assert_eq!(ids, vec!["visible".to_string()]);

        let all_visible = server.visible_bases_for_session(Some("session-b"), &private_caller())?;
        assert_eq!(all_visible.len(), 2);
        Ok(())
    }

    #[test]
    fn search_without_kb_id_spans_all_visible_bases() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let server = server_with_root(tmp.path().to_path_buf());
        server.service.create_base("alpha", "Alpha", None)?;
        server.service.create_base("beta", "Beta", None)?;
        server.service.create_base("hidden", "Hidden", None)?;

        crate::knowledge::store::write_page(
            &crate::knowledge::paths::kb_root(server.service.root(), "alpha"),
            "knowledge/notes/a.md",
            "# Shared topic\n\nalpha content",
            "alpha page",
            None,
        )?;
        crate::knowledge::store::write_page(
            &crate::knowledge::paths::kb_root(server.service.root(), "beta"),
            "knowledge/notes/b.md",
            "# Shared topic\n\nbeta content",
            "beta page",
            None,
        )?;
        crate::knowledge::store::write_page(
            &crate::knowledge::paths::kb_root(server.service.root(), "hidden"),
            "knowledge/notes/c.md",
            "# Shared topic\n\nhidden content",
            "hidden page",
            None,
        )?;
        server
            .service
            .set_hidden_for_session("session-a", &["hidden".to_string()])?;

        let hits = server.search_visible_bases(
            "shared topic",
            10,
            Some("session-a"),
            &private_caller(),
            SearchScope::Knowledge,
        )?;
        let kb_ids = hits.into_iter().map(|hit| hit.kb_id).collect::<Vec<_>>();
        assert!(kb_ids.contains(&"alpha".to_string()));
        assert!(kb_ids.contains(&"beta".to_string()));
        assert!(!kb_ids.contains(&"hidden".to_string()));
        Ok(())
    }

    /// Issue #26: a raw/ write is a contract violation the caller can fix, so
    /// it must surface as INVALID_PARAMS (taxonomy: invalid_args — "fix the
    /// call itself") carrying the recovery path, not as an opaque internal
    /// error classified tool_failure.
    #[tokio::test]
    async fn kb_write_page_rejects_raw_paths_as_invalid_params() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let server = server_with_root(tmp.path().to_path_buf());
        server.service.create_base("kb", "KB", None)?;

        let err = call_tool_as_session(
            &server,
            "kb_write_page",
            serde_json::json!({
                "kb_id": "kb",
                "path": "raw/x/source.md",
                "content": "body",
                "commit_message": "try to edit a raw source",
            }),
            Some("session-a"),
            Private,
        )
        .await
        .expect_err("raw/ writes must be rejected");

        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            err.message.contains("knowledge/") && err.message.contains("kb_add_raw_source"),
            "rejection must state the contract and the recovery, got: {}",
            err.message
        );

        // The description itself must teach the path contract up front.
        let desc = KnowledgeServer::tool_router()
            .list_all()
            .into_iter()
            .find(|t| t.name == "kb_write_page")
            .and_then(|t| t.description.clone())
            .expect("kb_write_page has a description");
        assert!(
            desc.contains("knowledge/") && desc.contains("read-only"),
            "kb_write_page description must state the path contract, got: {desc}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn committed_page_refresh_failure_removes_old_cache_and_retry_does_not_recommit(
    ) -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let server = server_with_root(tmp.path().to_path_buf());
        server.service.create_base("kb", "KB", None)?;
        let kb = crate::knowledge::paths::kb_root(server.service.root(), "kb");
        crate::knowledge::store::write_page(
            &kb,
            "knowledge/concept/a.md",
            &valid_page("concept", "a", "A"),
            "add A",
            None,
        )?;
        server.service.rebuild_graph_cache("kb")?;
        crate::knowledge::graph::fail_cache_writes(&kb, 1);
        let params = serde_json::json!({
            "kb_id": "kb",
            "path": "knowledge/concept/b.md",
            "content": valid_page("concept", "b", "B"),
            "commit_message": "add B",
        });

        let error = call_tool_as_session(
            &server,
            "kb_write_page",
            params.clone(),
            Some("session-a"),
            Private,
        )
        .await
        .expect_err("the injected post-commit refresh must be reported");
        assert!(error.message.contains("committed in commit"), "{error:?}");
        assert_eq!(
            error.data.as_ref().and_then(|data| data["phase"].as_str()),
            Some("committed")
        );
        assert!(
            error
                .data
                .as_ref()
                .and_then(|data| data["commit_sha"].as_str())
                .is_some(),
            "{error:?}"
        );
        assert!(kb.join("knowledge/concept/b.md").exists());
        assert!(
            !crate::knowledge::graph::cache_path(&kb).exists(),
            "an older cache survived the failed rebuild"
        );
        let history_len = GitRepo::open(&kb)?.log(10)?.len();

        call_tool_as_session(&server, "kb_write_page", params, Some("session-a"), Private)
            .await
            .expect("an identical retry reuses the durable page");
        assert_eq!(GitRepo::open(&kb)?.log(10)?.len(), history_len);
        assert!(server
            .service
            .get_graph("kb")?
            .nodes
            .iter()
            .any(|node| node.path == "knowledge/concept/b.md"));
        Ok(())
    }

    async fn begin_transaction_handle(
        server: &KnowledgeServer,
        kb_id: &str,
        session_id: &str,
    ) -> String {
        let begin = call_tool_as_session(
            server,
            "kb_begin_txn",
            serde_json::json!({ "kb_id": kb_id, "label": "curate" }),
            Some(session_id),
            Private,
        )
        .await;
        json_of(&begin)["txn"]
            .as_str()
            .expect("begin returns an opaque handle")
            .to_string()
    }

    fn transaction_write_args(kb_id: &str, txn: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "kb_id": kb_id,
            "path": "knowledge/note/a.md",
            "content": valid_page("Note", "A", "body"),
            "commit_message": "write A",
            "txn": txn,
        })
    }

    /// #157, the root cause. An open transaction holds the base's write lock —
    /// the in-process mutex AND the on-disk `flock` — for its whole lifetime, so
    /// one that is never committed made every later operation on that base, from
    /// any session, block until the process exited.
    ///
    /// The two reapers that existed both fired on TEARDOWN, which is exactly the
    /// case this is not: the session is alive and simply never comes back to the
    /// transaction.
    #[tokio::test]
    async fn a_transaction_nobody_touches_is_rolled_back_and_releases_its_base() {
        let (server, _tmp, _root) = migrated_server_with_bases(&["kb", "other"]);
        let _handle = begin_transaction_handle(&server, "kb", "session-a").await;

        // Precondition, and the property the whole bug rests on: an open
        // transaction really is holding the base's lock. Without this assertion
        // the test below could pass against a build where a transaction holds
        // nothing at all.
        assert!(
            server.service.kb_queue_is_occupied("kb"),
            "precondition: an open transaction holds the base's write lock"
        );

        // Nothing is abandoned yet — a transaction opened a moment ago must not
        // be reaped, or a live macro would have its work rolled back under it.
        assert!(
            server
                .transactions
                .abandoned_after(std::time::Duration::from_secs(3600))
                .await
                .is_empty(),
            "a transaction opened just now is in use, not abandoned"
        );

        // …and once it has been idle past the threshold, it is.
        let stale = server
            .transactions
            .abandoned_after(std::time::Duration::ZERO)
            .await;
        assert_eq!(stale.len(), 1, "the idle transaction must be reaped");
        assert_eq!(stale[0].0, "kb");

        let released = server
            .abort_transaction_slots(stale, None)
            .await
            .expect("rolling back an abandoned transaction");
        assert_eq!(released, 1);

        // The lock is free again, which is the whole point: the base is usable
        // without restarting the process.
        assert!(
            !server.service.kb_queue_is_occupied("kb"),
            "rolling the transaction back must release the base's write lock"
        );

        // And the base really does take writes again.
        let after = call_tool_as_session(
            &server,
            "kb_write_page",
            transaction_write_args("kb", None),
            Some("session-b"),
            Private,
        )
        .await;
        let after = after.expect("the write must reach the tool, not be refused upstream");
        assert!(
            !after.is_error.unwrap_or(false),
            "the base must accept writes after the abandoned transaction is rolled back: {after:?}"
        );
    }

    #[tokio::test]
    async fn transaction_handles_are_session_bound_opaque_and_non_oracular() {
        let (server, _tmp, _root) = migrated_server_with_bases(&["kb", "other"]);
        let handle = begin_transaction_handle(&server, "kb", "session-a").await;
        uuid::Uuid::parse_str(&handle).expect("the handle is an opaque UUID");
        assert!(!handle.starts_with("txn/"));

        let wrong_handle = call_tool_as_session(
            &server,
            "kb_write_page",
            transaction_write_args("kb", Some("txn/caller-constructed")),
            Some("session-a"),
            Private,
        )
        .await
        .expect_err("caller-constructed branches are not handles");
        let wrong_session = call_tool_as_session(
            &server,
            "kb_write_page",
            transaction_write_args("kb", Some(&handle)),
            Some("session-b"),
            Private,
        )
        .await
        .expect_err("a handle belongs to exactly one session");
        let wrong_kb = call_tool_as_session(
            &server,
            "kb_write_page",
            transaction_write_args("other", Some(&handle)),
            Some("session-a"),
            Private,
        )
        .await
        .expect_err("a handle belongs to exactly one knowledge base");
        let missing_handle = call_tool_as_session(
            &server,
            "kb_write_page",
            transaction_write_args("kb", None),
            Some("session-a"),
            Private,
        )
        .await
        .expect_err("standalone writes cannot enter an active transaction");
        for error in [&wrong_session, &wrong_kb, &missing_handle] {
            assert_eq!(error.code, wrong_handle.code);
            assert_eq!(error.message, wrong_handle.message);
        }
        call_tool_as_session(
            &server,
            "kb_abort_txn",
            serde_json::json!({ "kb_id": "kb", "txn": handle }),
            Some("session-a"),
            Private,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn transaction_open_refuses_a_legacy_profile_without_retaining_the_lock() {
        let (server, _tmp, root) = migrated_server_with_bases(&["kb"]);
        let kb_root = root.join("kb");
        let mut manifest = crate::knowledge::manifest::load(&kb_root).unwrap();
        manifest.schema_version = crate::knowledge::types::AUTOMATIC_SCHEMA_CEILING;
        crate::knowledge::manifest::save(&kb_root, &manifest).unwrap();

        for session_id in ["session-a", "session-b"] {
            let error = call_tool_as_session(
                &server,
                "kb_begin_txn",
                serde_json::json!({ "kb_id": "kb", "label": "legacy" }),
                Some(session_id),
                Private,
            )
            .await
            .expect_err("retired knowledge must not retain a transaction lock");
            assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_REQUEST);
            assert!(error.message.contains("retired pre-OKF"), "{error:?}");
            assert!(!server.service.kb_queue_is_occupied("kb"));
        }
    }

    #[tokio::test]
    async fn owner_teardown_releases_the_kb_lock_without_a_timeout() -> anyhow::Result<()> {
        let (server, _tmp, _root) = migrated_server_with_bases(&["kb"]);
        begin_transaction_handle(&server, "kb", "session-a").await;
        assert!(server.service.kb_queue_is_occupied("kb"));

        let waiting_service = server.service.clone();
        let (acquired_tx, acquired_rx) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn(async move {
            let _guard = waiting_service.lock_kb("kb").await?;
            let _ = acquired_tx.send(());
            Ok::<_, anyhow::Error>(())
        });
        tokio::task::yield_now().await;
        assert!(
            !waiter.is_finished(),
            "a live owner still holds the KB lock"
        );
        assert_eq!(
            server.abort_transactions_for_session("session-b").await?,
            0,
            "tearing down another session must not affect the owner"
        );
        assert!(!waiter.is_finished());

        assert_eq!(server.abort_transactions_for_session("session-a").await?, 1);
        tokio::time::timeout(std::time::Duration::from_secs(1), acquired_rx)
            .await
            .expect("the abandoned transaction lock must be released")
            .expect("the lock waiter remains alive");
        waiter.await??;

        begin_transaction_handle(&server, "kb", "session-b").await;
        assert!(server.force_abort_transaction("kb").await?);
        assert!(!server.force_abort_transaction("kb").await?);
        assert!(!server.service.kb_queue_is_occupied("kb"));
        Ok(())
    }

    #[tokio::test]
    async fn one_handle_drives_all_transactional_writes_and_is_consumed() -> anyhow::Result<()> {
        let (server, _tmp, root) = migrated_server_with_bases(&["kb"]);
        let handle = begin_transaction_handle(&server, "kb", "session-a").await;
        call_tool_as_session(
            &server,
            "kb_write_page",
            transaction_write_args("kb", Some(&handle)),
            Some("session-a"),
            Private,
        )
        .await?;
        call_tool_as_session(
            &server,
            "kb_add_raw_source",
            serde_json::json!({
                "kb_id": "kb",
                "source": { "kind": "text", "text": "source body", "title": "source" },
                "txn": handle.clone(),
            }),
            Some("session-a"),
            Private,
        )
        .await?;
        call_tool_as_session(
            &server,
            "kb_append_log",
            serde_json::json!({
                "kb_id": "kb",
                "kind": "manual",
                "summary": "curated",
                "txn": handle.clone(),
            }),
            Some("session-a"),
            Private,
        )
        .await?;
        call_tool_as_session(
            &server,
            "kb_commit_txn",
            serde_json::json!({
                "kb_id": "kb",
                "txn": handle.clone(),
                "summary": "curated transaction",
                "kind": "manual",
            }),
            Some("session-a"),
            Private,
        )
        .await?;

        assert!(root.join("kb/knowledge/note/a.md").exists());
        assert_eq!(GitRepo::open(&root.join("kb"))?.log(20)?.len(), 2);
        let replay = call_tool_as_session(
            &server,
            "kb_commit_txn",
            serde_json::json!({
                "kb_id": "kb",
                "txn": handle,
                "summary": "replay",
                "kind": "manual",
            }),
            Some("session-a"),
            Private,
        )
        .await
        .expect_err("a terminal call consumes the handle");
        assert_eq!(replay.message, TRANSACTION_UNAVAILABLE);
        Ok(())
    }

    // ---- Issue #56, decision (2)(b): where a MODEL's export comes to rest ----
    //
    // ⚠ DEVIATION from the task text, recorded rather than hidden. The task
    // drives these through `call_tool_as(&srv, tool, args, tier)` — CP1's
    // harness, which Task 10C creates. There is no such seam in this task: the
    // generated `<KnowledgeServer as ServerHandler>::call_tool` demands an
    // `rmcp::RequestContext`, and building one needs a live `Peer` (see
    // `developer/rmcp_developer.rs`'s `serve_directly` fixtures). The tools are
    // therefore invoked directly, which is the same production function body the
    // router would reach. Nothing is lost: the location rule keys on the
    // **base's** tier, not the caller's — a public caller never gets this far,
    // because Task 10C's barrier refuses it outright — so the `tier` argument
    // would have been inert here anyway.

    fn migrated_server_with_base(id: &str) -> (KnowledgeServer, tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let server = server_with_root(root.clone());
        server.service.create_base(id, id, None).unwrap();
        (server, tmp, root)
    }

    fn seed_page(root: &Path, kb_id: &str, rel: &str, body: &str) {
        let p = root.join(kb_id).join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn reported_export_path(out: &CallToolResult) -> PathBuf {
        let text = out
            .content
            .iter()
            .find_map(|c| c.as_text())
            .expect("kb_export returns a text payload");
        let v: serde_json::Value = serde_json::from_str(&text.text).expect("valid json");
        PathBuf::from(v["path"].as_str().expect("a reported path"))
    }

    fn zip_names(bytes: &[u8]) -> Vec<String> {
        let mut a = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        (0..a.len())
            .map(|i| a.by_index(i).unwrap().name().to_string())
            .collect()
    }

    async fn kb_export_via_tool(
        srv: &KnowledgeServer,
        kb_id: &str,
        dest_path: Option<String>,
    ) -> Result<CallToolResult, ErrorData> {
        srv.kb_export(Parameters(ExportArchiveParams {
            kb_id: kb_id.to_string(),
            dest_path,
        }))
        .await
    }

    #[tokio::test]
    async fn a_models_export_of_a_private_base_lands_inside_the_knowledge_root() {
        // Decision (2)(b) as behaviour. The exporter is PRIVATE — a public one is
        // refused outright by Task 10C's barrier — so this is the caller the
        // location rule exists for: permitted to export, not permitted to choose
        // where the bytes come to rest.
        let (srv, _tmp, root) = migrated_server_with_base("omop");
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        seed_page(&root, "omop", "knowledge/x.md", "SENTINEL-COHORT-N-412");
        let elsewhere = tempfile::tempdir().unwrap();
        let asked = elsewhere.path().join("omop.brkb");
        // ⚠ READ-ONLY, and this is the assertion — not decoration. A
        // `!asked.exists()` at the END passes "write the archive outside, then
        // move it inside before returning", which opens a real public-read window
        // for however long the copy takes. A final-state check cannot see a
        // transient file and no amount of polling makes it deterministic. Making
        // the directory unwritable turns the timing question into an ERROR: the
        // write-then-move implementation gets EACCES and fails the export; the
        // correct one never touches this directory and is unaffected.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(elsewhere.path(), std::fs::Permissions::from_mode(0o555))
                .unwrap();
            // ⚠ SELF-CHECK ON THE FIXTURE, and it is part of the gate. Under root
            // the mode bits are ignored and the `chmod` silently becomes a no-op,
            // which turns this whole test back into the final-state check it
            // replaces. Assert the property directly rather than proxying it
            // through a euid comparison.
            assert!(
                std::fs::write(elsewhere.path().join(".probe"), b"x").is_err(),
                "the read-only fixture did not take (running as root?), so this test \
                 would silently degrade to the assertion it was written to replace"
            );
        }

        let out = kb_export_via_tool(&srv, "omop", Some(asked.display().to_string()))
            .await
            .unwrap();

        // (a) nothing was written where the model aimed it — at any point, not
        //     just at the end.
        assert!(
            !asked.exists(),
            "a private base was exported outside the deny root"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // so TempDir can clean up
            std::fs::set_permissions(elsewhere.path(), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
        // (b) the tool REPORTED the real location, and it is under <root>/.exports/.
        let written = reported_export_path(&out);
        assert!(
            written.starts_with(crate::knowledge::paths::model_export_dir(&root)),
            "reported {}, which is not inside the knowledge root",
            written.display()
        );
        assert!(written.exists());
        // (c) …and it is the archive, not an empty file that satisfies (a) and (b).
        //     Without this, "write nothing anywhere" passes.
        assert!(zip_names(&std::fs::read(&written).unwrap())
            .iter()
            .any(|n| n.ends_with("knowledge/x.md")));
    }

    #[tokio::test]
    #[allow(non_snake_case)]
    async fn a_models_export_of_a_PUBLIC_base_still_honours_dest_path() {
        // The mirror, and the reason the rule is scoped to private bases: forcing
        // the location for EVERY model export breaks `kb_export` as a feature, and
        // whoever hits that next will "fix" it by deleting the rule.
        let (srv, _tmp, root) = migrated_server_with_base("notes"); // registers public
        let elsewhere = tempfile::tempdir().unwrap();
        let asked = elsewhere.path().join("notes.brkb");
        let out = kb_export_via_tool(&srv, "notes", Some(asked.display().to_string()))
            .await
            .unwrap();
        assert!(asked.exists(), "a public base's export was relocated");
        assert_eq!(reported_export_path(&out), asked);
        assert!(!crate::knowledge::paths::model_export_dir(&root)
            .join("notes.brkb")
            .exists());
    }

    #[tokio::test]
    async fn a_private_export_cannot_be_collected_inside_a_public_base() {
        // Issue #56. The export directory is a sibling of the bases, never one
        // of them. If its name validated as a kb id, a session could create that
        // base first and every private archive would land inside a PUBLIC base's
        // own tree — `brkb::walk` packs whatever it finds — so exporting that
        // base would hand out every private one. The name is `.exports`, which
        // `validate_kb_id` rejects, so `create_base` cannot reach it at all.
        let (srv, _tmp, root) = migrated_server_with_base("omop");
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        seed_page(&root, "omop", "knowledge/x.md", "SENTINEL-COHORT-N-412");

        let dir = crate::knowledge::paths::MODEL_EXPORT_DIR;
        assert!(
            crate::knowledge::paths::validate_kb_id(dir).is_err(),
            "the export directory {dir} is a legal kb id, so a base can be created over it"
        );
        assert!(srv.service.create_base(dir, "collector", None).is_err());

        let written = reported_export_path(&kb_export_via_tool(&srv, "omop", None).await.unwrap());
        assert_eq!(
            written.parent().unwrap(),
            crate::knowledge::paths::model_export_dir(&root)
        );
        // …and the archive is not inside any knowledge base's directory.
        for entry in std::fs::read_dir(&root).unwrap() {
            let e = entry.unwrap();
            let name = e.file_name().to_string_lossy().to_string();
            if crate::knowledge::paths::validate_kb_id(&name).is_ok() {
                assert!(
                    !written.starts_with(e.path()),
                    "the export landed inside the knowledge base {name}"
                );
            }
        }
    }

    // ── Issue #56, Task 10B: the ratchet at CP1 ──────────────────────────────

    /// The capability a test drives a tool call with. An enum rather than a
    /// `bool` so the call sites read as `Private` / `Public` and cannot be
    /// transposed silently.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Caller {
        Public,
        Private,
    }
    use Caller::{Private, Public};

    impl Caller {
        fn is_private(self) -> bool {
            matches!(self, Caller::Private)
        }
    }

    fn migrated_server_with_bases(ids: &[&str]) -> (KnowledgeServer, tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let server = server_with_root(root.clone());
        for id in ids {
            server.service.create_base(id, id, None).unwrap();
        }
        (server, tmp, root)
    }

    /// Drive `KnowledgeServer::call_tool` BY NAME with a request whose meta
    /// carries the caller's capability — the only way to express "as a private
    /// caller" for the eight `kb_*` tools that take no `RequestContext` at all,
    /// and therefore the whole reason CP1 is a hand-written `call_tool` rather
    /// than a per-tool argument.
    ///
    /// A `RequestContext` needs a live `Peer`, which only `serve_directly`
    /// mints; the duplex transport is drained and dropped with the call. This
    /// mirrors `developer/rmcp_developer.rs`'s `create_test_transport`.
    async fn call_tool_as(
        srv: &KnowledgeServer,
        name: &str,
        args: serde_json::Value,
        caller: Caller,
    ) -> Result<CallToolResult, ErrorData> {
        call_tool_as_session(srv, name, args, None, caller).await
    }

    /// The same seam, with a chat session id in the meta — the scope every
    /// primary-pointer and visible-set question is answered against.
    async fn call_tool_as_session(
        srv: &KnowledgeServer,
        name: &str,
        args: serde_json::Value,
        session_id: Option<&str>,
        caller: Caller,
    ) -> Result<CallToolResult, ErrorData> {
        call_tool_as_full(
            srv,
            name,
            args,
            session_id,
            caller,
            // Issue #56 DR-26. `Unstated` and not `Local`, so the ~40 existing
            // callers of this seam keep testing the TIER axis against the
            // restrictive value: a `Local` default would clear every
            // institutional base and silently make a future affiliation
            // assertion vacuous.
            &CallerAffiliation::Unstated,
        )
        .await
    }

    /// The same seam again, with DR-26's third axis stated. Named separately
    /// rather than added to the two above so a test that means to exercise
    /// affiliation says so.
    async fn call_tool_as_affiliated(
        srv: &KnowledgeServer,
        name: &str,
        args: serde_json::Value,
        institution: &str,
    ) -> Result<CallToolResult, ErrorData> {
        call_tool_as_full(
            srv,
            name,
            args,
            None,
            Private,
            &CallerAffiliation::Institution(institution.to_string()),
        )
        .await
    }

    async fn call_tool_as_full(
        srv: &KnowledgeServer,
        name: &str,
        args: serde_json::Value,
        session_id: Option<&str>,
        caller: Caller,
        affiliation: &CallerAffiliation,
    ) -> Result<CallToolResult, ErrorData> {
        call_tool_as_full_with_cancellation(
            srv,
            name,
            args,
            session_id,
            caller,
            affiliation,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
    }

    async fn call_tool_as_full_with_cancellation(
        srv: &KnowledgeServer,
        name: &str,
        args: serde_json::Value,
        session_id: Option<&str>,
        caller: Caller,
        affiliation: &CallerAffiliation,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<CallToolResult, ErrorData> {
        use tokio::io::AsyncReadExt as _;

        let (mut client, server_side) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            let mut buffer = [0_u8; 8192];
            while client.read(&mut buffer).await.unwrap_or(0) != 0 {}
        });
        let running = rmcp::service::serve_directly(srv.clone(), server_side, None);
        let mut meta = rmcp::model::Meta::new();
        meta.0.insert(
            crate::knowledge::tier::CAPABILITY_TIER_META_KEY.to_string(),
            serde_json::Value::String(
                crate::knowledge::tier::capability_meta_value(caller.is_private()).to_string(),
            ),
        );
        // Issue #56 DR-26 / Task 50. Written through the production formatter,
        // never a literal: the reader compares against its own spelling, and a
        // hand-typed copy here would drift silently.
        if let Some(wire) = crate::knowledge::affiliation::capability_meta_value(affiliation) {
            meta.0.insert(
                crate::knowledge::affiliation::CAPABILITY_AFFILIATION_META_KEY.to_string(),
                serde_json::Value::String(wire),
            );
        }
        if let Some(sid) = session_id {
            meta.0.insert(
                SESSION_ID_META_KEY.to_string(),
                serde_json::Value::String(sid.to_string()),
            );
        }
        let context = RequestContext {
            ct: cancel,
            id: rmcp::model::NumberOrString::Number(1),
            meta,
            extensions: Default::default(),
            peer: running.peer().clone(),
        };
        let request = rmcp::model::CallToolRequestParams {
            name: name.to_string().into(),
            arguments: args.as_object().cloned(),
            task: None,
            meta: None,
        };
        let out = ServerHandler::call_tool(srv, request, context).await;
        drop(running);
        out
    }

    /// A `.brkb` archive of a base whose tier is `tier`, written to a file and
    /// returned with the `TempDir` that owns it — bind BOTH or the path is
    /// unlinked before the import reads it.
    fn brkb_fixture(tier: Caller) -> (tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("src-root");
        std::fs::create_dir_all(&src_root).unwrap();
        let svc = KnowledgeService::new(src_root.clone());
        svc.create_base("shipped", "Shipped", None).unwrap();
        seed_page(&src_root, "shipped", "knowledge/x.md", "SENTINEL");
        if tier.is_private() {
            crate::knowledge::tier::raise_unlocked(&src_root, "shipped", true).unwrap();
        }
        let bytes = svc.export_brkb("shipped").unwrap();
        let path = tmp.path().join("shipped.brkb");
        std::fs::write(&path, &bytes).unwrap();
        (tmp, path.to_string_lossy().to_string())
    }

    fn legacy_brkb_fixture() -> (tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("src-root");
        std::fs::create_dir_all(&src_root).unwrap();
        let svc = KnowledgeService::new(src_root.clone());
        svc.create_base("shipped", "Shipped", None).unwrap();
        let kb = src_root.join("shipped");
        let mut manifest = crate::knowledge::manifest::load(&kb).unwrap();
        manifest.schema_version = crate::knowledge::types::AUTOMATIC_SCHEMA_CEILING;
        crate::knowledge::manifest::save(&kb, &manifest).unwrap();
        let bytes = svc.export_brkb("shipped").unwrap();
        let path = tmp.path().join("legacy.brkb");
        std::fs::write(&path, &bytes).unwrap();
        (tmp, path.to_string_lossy().to_string())
    }

    /// The same fixture on DR-26's axis: a base whose content belongs to
    /// `institution`, stamped through the production ratchet rather than by
    /// writing the store, and exported to a file.
    fn brkb_fixture_owned_by(institution: &str) -> (tempfile::TempDir, String) {
        let tmp = tempfile::tempdir().unwrap();
        let src_root = tmp.path().join("src-root");
        std::fs::create_dir_all(&src_root).unwrap();
        let svc = KnowledgeService::new(src_root.clone());
        svc.create_base("shipped", "Shipped", None).unwrap();
        seed_page(&src_root, "shipped", "knowledge/x.md", "SENTINEL");
        svc.raise_tier_and_affiliation(
            "shipped",
            true,
            &CallerAffiliation::Institution(institution.to_string()),
        )
        .unwrap();
        let bytes = svc.export_brkb("shipped").unwrap();
        let path = tmp.path().join("shipped.brkb");
        std::fs::write(&path, &bytes).unwrap();
        (tmp, path.to_string_lossy().to_string())
    }

    fn imported_kb_id(out: &CallToolResult) -> String {
        let text = out
            .content
            .iter()
            .find_map(|c| c.as_text())
            .expect("kb_import returns a text payload");
        let v: serde_json::Value = serde_json::from_str(&text.text).expect("valid json");
        v["imported_kb_id"]
            .as_str()
            .expect("an imported id")
            .to_string()
    }

    #[tokio::test]
    async fn a_private_session_writing_one_page_ratchets_the_whole_base() {
        // THE test for the ruling: one page from one private chat privatises the
        // machine-wide base.
        let (srv, _tmp, root) = migrated_server_with_bases(&["default"]);
        call_tool_as(
            &srv,
            "kb_write_page",
            serde_json::json!({
                "kb_id": "default", "path": "knowledge/omop.md",
                "content": valid_page("note", "OMOP", "n=412 T2D patients"),
                "commit_message": "x"
            }),
            Private,
        )
        .await
        .unwrap();
        assert!(crate::knowledge::tier::is_private(&root, "default"));
    }

    #[tokio::test]
    async fn a_public_session_writing_never_lowers_a_ratcheted_base() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["default"]);
        crate::knowledge::tier::raise_unlocked(&root, "default", true).unwrap();
        // Task 10B's comment here read "Task 10C has not landed, so this write
        // still SUCCEEDS", and the call was `.unwrap()`ed. 10C has landed, so
        // the write is now REFUSED — the strictly stronger outcome. The
        // assertion this test exists for is unchanged and still the last line:
        // the ratchet is monotone, and nothing a public caller does may lower it.
        let out = call_tool_as(
            &srv,
            "kb_append_log",
            serde_json::json!({ "kb_id": "default", "kind": "manual", "summary": "hi" }),
            Public,
        )
        .await;
        assert!(
            is_privacy_refusal(&out),
            "a public write reached a ratcheted base: {}",
            rendered(&out)
        );
        assert!(
            crate::knowledge::tier::is_private(&root, "default"),
            "a public write lowered the tier"
        );
    }

    /// name, arguments against a NAMED base, whether the call must leave that
    /// base private when made by a private caller, and — Task 10C — whether a
    /// PUBLIC caller naming a PRIVATE base must be refused.
    ///
    /// `refused_naming_a_private_base` is not a mirror of `KB_ID_GATED_TOOLS`:
    /// `kb_set_active` is deliberately not on that list (it reads no content)
    /// and is still refused, because a private base is not a member of a public
    /// caller's set. Keeping the two fields apart is what stops "gated" from
    /// quietly becoming "the only tools we thought about".
    struct ToolProbe {
        name: &'static str,
        args: fn(&str) -> serde_json::Value,
        ratchets: bool,
        refused_naming_a_private_base: bool,
    }

    impl ToolProbe {
        fn args_for(&self, kb_id: &str) -> serde_json::Value {
            (self.args)(kb_id)
        }
    }

    /// All twenty-two `default`-addressable `kb_*` tools. The exclusion list as
    /// data, reviewable in one
    /// place:
    ///   ratchets "default":      kb_write_page, kb_add_raw_source, kb_append_log
    ///   ratchets its OWN new id: kb_create_base, kb_import
    ///   does not ratchet:        the other seventeen
    const KB_TOOL_PROBES: &[ToolProbe] = &[
        ToolProbe {
            name: "kb_list_bases",
            args: |_kb| serde_json::json!({}),
            ratchets: false,
            // Omits rather than refuses: a single-base refusal would hide every
            // base the moment one of them is private.
            refused_naming_a_private_base: false,
        },
        ToolProbe {
            name: "kb_list_pages",
            args: |kb| serde_json::json!({ "kb_id": kb }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_read_page",
            args: |kb| serde_json::json!({ "kb_id": kb, "path": "index.md" }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            // Stage 4. Names a base and writes nothing, so: gated, not
            // ratcheting. `every_tool_that_writes_content_ratchets_and_the_
            // plumbing_ones_do_not` reads `ratchets: false` here and asserts the
            // base is still PUBLIC after a PRIVATE caller validates a draft
            // against it — which is the whole claim, since a tier raise is
            // permanent and a check that committed nothing must not cost one.
            name: "kb_validate_page",
            args: |kb| {
                serde_json::json!({
                    "kb_id": kb,
                    "path": "knowledge/p.md",
                    "content": valid_page("note", "P", "body"),
                })
            },
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            // Names a base and writes nothing, so: gated, not ratcheting —
            // `kb_validate_page`'s decision one scale up, and for the same
            // reason (a permanent tier raise bought by a caller who only
            // looked).
            //
            // ⚠ The decision that is NOT visible from this row is what the tool
            // leaves out. `macros::lint::lint` also has an AUTOFIX arm that
            // rewrites pages, and that arm must ratchet — but `ratchets` here is
            // one bool per tool NAME, so "ratchets when autofix=true" is
            // unsayable in this table. Rather than write a row that is true half
            // the time, the tool exposes `macros::lint::scan` alone and autofix
            // is not reachable from MCP at all. See `KB_RATCHETING_TOOLS`.
            name: "kb_lint",
            args: |kb| serde_json::json!({ "kb_id": kb }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_write_page",
            args: |kb| {
                serde_json::json!({
                    "kb_id": kb, "path": "knowledge/p.md",
                    "content": valid_page("note", "P", "body"),
                    "commit_message": "m"
                })
            },
            ratchets: true,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_add_raw_source",
            args: |kb| {
                serde_json::json!({
                    "kb_id": kb,
                    "source": { "kind": "text", "text": "n=412", "title": "note" }
                })
            },
            ratchets: true,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_append_log",
            args: |kb| serde_json::json!({ "kb_id": kb, "kind": "manual", "summary": "s" }),
            ratchets: true,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_get_graph",
            args: |kb| serde_json::json!({ "kb_id": kb }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_list_history",
            args: |kb| serde_json::json!({ "kb_id": kb }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_restore_state",
            args: |kb| serde_json::json!({ "kb_id": kb, "commit_sha": "HEAD" }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_begin_txn",
            args: |kb| serde_json::json!({ "kb_id": kb, "label": "t" }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_commit_txn",
            args: |kb| {
                serde_json::json!({
                    "kb_id": kb, "txn": "txn/t", "kind": "manual", "summary": "s"
                })
            },
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_abort_txn",
            args: |kb| serde_json::json!({ "kb_id": kb, "txn": "txn/t" }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_search",
            args: |kb| serde_json::json!({ "kb_id": kb, "query": "n" }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_search_raw_sources",
            args: |kb| serde_json::json!({ "kb_id": kb, "query": "n" }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_set_active",
            args: |kb| serde_json::json!({ "kb_id": kb }),
            ratchets: false,
            // NOT in `KB_ID_GATED_TOOLS`, and still refused — as a NON-MEMBER,
            // byte-identical to an id that does not exist (Task 10D's rule).
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_get_active",
            args: |_kb| serde_json::json!({}),
            ratchets: false,
            // Takes no arguments at all: it reports a FILTERED view rather than
            // refusing. Pinned by
            // `kb_get_active_does_not_enumerate_a_private_base_or_point_at_one`.
            refused_naming_a_private_base: false,
        },
        ToolProbe {
            name: "kb_export",
            args: |kb| serde_json::json!({ "kb_id": kb }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_merge_preview",
            // The source id names a base that does not exist, so the call fails
            // in the tool body — which is exactly what makes this a clean probe
            // of the SEAM: whatever CP1 decided about `kb_id` has already
            // happened by then.
            args: |kb| serde_json::json!({ "kb_id": kb, "source_kb_id": "elsewhere" }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            name: "kb_merge",
            args: |kb| serde_json::json!({ "kb_id": kb, "source_kb_id": "elsewhere" }),
            ratchets: true,
            refused_naming_a_private_base: true,
        },
        ToolProbe {
            // Gated, and pointedly NOT ratcheting — which reads backwards until
            // you ask what a ratchet is for. It records that a base has been
            // exposed to a more sensitive session, so that LATER reads of it can
            // be refused. A successful delete leaves no later reads: the tier
            // record is one of the six things `delete_base_under_locks` removes
            // (`tier::forget_unlocked`). Ratcheting first would write a
            // classification for an id that is about to stop existing, and if
            // the delete then failed and rolled back, a private caller's
            // *attempt* would have permanently privatised a public base — the
            // same one-way loss `kb_validate_page` and `kb_lint` are kept off
            // this list to avoid, bought here for even less.
            name: "kb_delete_base",
            args: |kb| serde_json::json!({ "kb_id": kb }),
            ratchets: false,
            refused_naming_a_private_base: true,
        },
    ];

    #[tokio::test]
    async fn every_tool_that_writes_content_ratchets_and_the_plumbing_ones_do_not() {
        // Parameterised over the twenty `default`-addressing tools, driven
        // through `call_tool` BY NAME — which is the point of CP1: eight of them
        // take no `RequestContext`, so a test that calls the `#[tool]` fn
        // directly cannot express "as a private caller" for them at all. A test
        // on kb_write_page alone passes an implementation that misses
        // kb_add_raw_source — the tool the GUI ingest panel and the `ingest`
        // macro actually call — so the whole ingest path would launder.
        //
        // `kb_create_base` and `kb_import` are the other two of the twenty-two;
        // they ratchet their OWN new id and have their own tests below.
        for probe in KB_TOOL_PROBES {
            let (srv, _tmp, root) = migrated_server_with_bases(&["default"]);
            let _ = call_tool_as(&srv, probe.name, probe.args_for("default"), Private).await;
            assert_eq!(
                crate::knowledge::tier::is_private(&root, "default"),
                probe.ratchets,
                "{} ratchets={} but the store says otherwise",
                probe.name,
                probe.ratchets
            );
        }
    }

    /// Issue #56 DR-26 / Task 50 Step 1, and the reason a grep census over
    /// `raise_tier` call sites was not what got written: this drives the SAME
    /// probe table through the SAME seam and asserts the affiliation ratchet
    /// fires wherever the tier one does.
    ///
    /// ⚠ **A tool that raised the tier and not the affiliation is the hole.**
    /// It would put a UCSF chat's content into a base no institution is
    /// recorded as owning, which reads as unclaimed and is therefore reachable
    /// from every other institution's model — a laundering path with no gate
    /// crossed. `every_tool_the_router_exposes_is_classified_by_the_probe_table`
    /// is what keeps this parameterisation exhaustive as tools are added.
    #[tokio::test]
    async fn every_tool_that_ratchets_the_tier_also_records_the_callers_institution() {
        for probe in KB_TOOL_PROBES {
            let (srv, _tmp, root) = migrated_server_with_bases(&["default"]);
            let _ =
                call_tool_as_affiliated(&srv, probe.name, probe.args_for("default"), "ucsf").await;

            let owners = crate::knowledge::tier::affiliation(&root, "default");
            let recorded = owners.owners().expect("a readable store").contains("ucsf");
            assert_eq!(
                recorded, probe.ratchets,
                "{} ratchets={} on the tier axis, but the affiliation store says \
                 recorded={recorded}",
                probe.name, probe.ratchets
            );
        }
    }

    #[tokio::test]
    async fn missing_base_mutations_leave_no_directory_lock_or_classification_residue() {
        for probe in KB_TOOL_PROBES.iter().filter(|probe| probe.ratchets) {
            let (srv, _tmp, root) = migrated_server_with_bases(&[]);
            let result = call_tool_as(&srv, probe.name, probe.args_for("retryable"), Private).await;
            assert!(
                result.is_err(),
                "{} unexpectedly accepted a missing base",
                probe.name
            );
            assert!(
                !root.join("retryable").exists(),
                "{} created a partial base or write lock",
                probe.name
            );
            assert!(
                !crate::knowledge::tier::has_metadata_unlocked(&root, "retryable").unwrap(),
                "{} classified an id that does not exist",
                probe.name
            );
            assert!(srv.service.list_bases().unwrap().is_empty());
            srv.service
                .create_base("retryable", "Retryable", None)
                .unwrap_or_else(|error| {
                    panic!("{} left the id uncreatable: {error:#}", probe.name)
                });
        }

        let (srv, _tmp, root) = migrated_server_with_bases(&[]);
        let result = call_tool_as(
            &srv,
            "kb_begin_txn",
            serde_json::json!({ "kb_id": "retryable", "label": "missing" }),
            Private,
        )
        .await;
        assert!(result.is_err());
        assert!(!root.join("retryable").exists());
        assert!(!crate::knowledge::tier::has_metadata_unlocked(&root, "retryable").unwrap());
        srv.service
            .create_base("retryable", "Retryable", None)
            .unwrap();
    }

    /// The end-to-end shape of DR-26 for knowledge bases, through CP1: a base
    /// two institutions' content has landed in is reachable from **neither** of
    /// their models. Both callers are PRIVATE, so every tier gate in this
    /// campaign says yes and only the third axis refuses.
    ///
    /// ⚠ **The two-owner state is seeded through the production ratchet, not by
    /// writing twice, and the reason is a finding worth recording.** With the
    /// barrier ABOVE the raise at this choke point (`call_tool`), a second
    /// institution's write is refused *before* it can add itself — so a base
    /// cannot accumulate two owners through `kb_*` at all. The state is still
    /// reachable: a hand-edited store, Step 3's cross-session ingest, and any
    /// future KB-scoped grant (which DR-26 requires and this task does not
    /// ship — see `tier::cross_affiliation_refusal`) each produce it. That is
    /// exactly why `affiliation::reachable` must not stop at the first matching
    /// owner, and why this asserts the refusal rather than assuming the state is
    /// unreachable.
    #[tokio::test]
    async fn a_base_two_institutions_wrote_into_is_out_of_reach_of_both() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["shared"]);

        // A UCSF chat writes: permitted, and it claims the base.
        call_tool_as_affiliated(
            &srv,
            "kb_write_page",
            serde_json::json!({
                "kb_id": "shared",
                "path": "knowledge/ucsf.md",
                "content": valid_page("note", "UCSF", "SENTINEL-UCSF"),
                "commit_message": "m",
            }),
            "ucsf",
        )
        .await
        .expect("a UCSF chat may write into an unclaimed base");

        // Stanford's write is refused by the barrier — which is the control
        // working, and the reason the second owner has to be seeded through the
        // production ratchet instead.
        let refused = call_tool_as_affiliated(
            &srv,
            "kb_write_page",
            serde_json::json!({
                "kb_id": "shared",
                "path": "knowledge/stanford.md",
                "content": valid_page("note", "Stanford", "SENTINEL-STANFORD"),
                "commit_message": "m",
            }),
            "stanford",
        )
        .await
        .expect_err("a Stanford chat may not write into a base holding UCSF content");
        assert!(refused.to_string().contains("Cross-institutional"));

        srv.service
            .raise_tier_and_affiliation(
                "shared",
                true,
                &CallerAffiliation::Institution("stanford".to_string()),
            )
            .unwrap();

        let owners = crate::knowledge::tier::affiliation(&root, "shared");
        let owners = owners.owners().expect("a readable store");
        assert!(
            owners.contains("ucsf") && owners.contains("stanford"),
            "{owners:?}"
        );

        // Now neither institution's model may read it.
        for institution in ["ucsf", "stanford"] {
            let err = call_tool_as_affiliated(
                &srv,
                "kb_search",
                serde_json::json!({ "kb_id": "shared", "query": "SENTINEL" }),
                institution,
            )
            .await
            .expect_err("a model matching only one of two owners is a mismatch");
            let msg = err.to_string();
            assert!(msg.contains("Cross-institutional"), "{msg}");
            assert!(msg.contains("ucsf") && msg.contains("stanford"), "{msg}");
            assert!(
                !msg.contains("SENTINEL"),
                "the refusal carried content: {msg}"
            );
        }

        // …and a local model still reaches it, because no transfer occurs.
        call_tool_as_full(
            &srv,
            "kb_search",
            serde_json::json!({ "kb_id": "shared", "query": "SENTINEL" }),
            None,
            Private,
            &CallerAffiliation::Local,
        )
        .await
        .expect("a local model reaches every private base");
    }

    fn declared_tool(name: &str) -> rmcp::model::Tool {
        KnowledgeServer::tool_router()
            .list_all()
            .into_iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} is no longer declared"))
    }

    /// ⚠ **The tool description is the mechanism, not documentation about it.**
    /// Which format a base gets is decided by a model reading this string, so a
    /// rewrite that drops the guidance is a behaviour change with no other
    /// symptom — every base silently becomes whichever one the model guesses.
    ///
    /// Two halves, and both are load-bearing:
    ///
    /// - the **schema** carries the closed vocabulary, which is the half a
    ///   provider can constrain sampling with (DR-16) and the half that survives
    ///   a model not reading carefully;
    /// - the **prose** carries the *choice*, which no schema can express: that
    ///   the axis is biomedical-or-not, and that OKF is the answer when unsure.
    #[test]
    fn the_format_argument_reaches_the_model_as_a_closed_vocabulary_with_the_choice_explained() {
        let tool = declared_tool("kb_create_base");
        let schema = serde_json::to_string(&*tool.input_schema).unwrap();
        assert!(
            schema.contains("\"format\""),
            "kb_create_base no longer takes a format: {schema}"
        );
        // Derived from the type, never re-typed here: a hand-written pair that
        // agrees with the code proves only that somebody transcribed it twice.
        for f in [
            crate::knowledge::types::KbFormat::Okf,
            crate::knowledge::types::KbFormat::Biookf,
        ] {
            assert!(
                schema.contains(&format!("\"{}\"", f.as_str())),
                "`{}` is not in the declared vocabulary, so the provider cannot \
                 constrain sampling to it: {schema}",
                f.as_str()
            );
        }

        let description = tool.description.as_deref().unwrap_or_default();
        for phrase in ["okf", "biookf", "biomedical", "not biomedical", "unsure"] {
            assert!(
                description.contains(phrase),
                "the description no longer teaches the choice: `{phrase}` is gone"
            );
        }
        // …and it teaches the choice without leaking the reasoning behind the
        // implementation, which the model pays for on every turn and cannot use.
        assert!(
            !schema.contains("DR-12") && !description.contains("DR-12"),
            "implementation rationale is being shipped to the model"
        );
    }

    /// The one thing `kb_validate_page`'s description has to establish, because
    /// a model that reads it as a lint pass will call it after writing, which is
    /// the moment it stops being useful.
    #[test]
    fn the_validator_describes_itself_as_something_to_call_before_writing() {
        let tool = declared_tool("kb_validate_page");
        let description = tool.description.as_deref().unwrap_or_default();
        assert!(description.contains("BEFORE writing"), "{description}");
        assert!(description.contains("Nothing is written"), "{description}");
    }

    /// The claim on `KB_ID_GATED_TOOLS` — that a twenty-second `kb_*` tool is
    /// classified the day it is written — is only true if something ties the
    /// lists to the router. Both `KB_ID_GATED_TOOLS` and `KB_RATCHETING_TOOLS`
    /// are opt-in allowlists, so a new tool defaults to ungated and unratcheted
    /// and every other test in this file would still pass. This is the tie: the
    /// classification table above plus the two id-minting tools must account for
    /// every tool the router exposes, exactly.
    #[test]
    fn every_tool_the_router_exposes_is_classified_by_the_probe_table() {
        let exposed: std::collections::BTreeSet<String> = KnowledgeServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        let classified: std::collections::BTreeSet<String> = KB_TOOL_PROBES
            .iter()
            .map(|p| p.name.to_string())
            // The two whose subject id is minted BY the call, so they cannot be
            // probed against "default"; they have their own tests below.
            .chain(["kb_create_base".to_string(), "kb_import".to_string()])
            .collect();
        assert_eq!(
            exposed, classified,
            "a kb_* tool is missing from the ratchet classification table (or the \
             table names one the router does not expose). Add it to KB_TOOL_PROBES \
             with the ratchets= decision, and to KB_ID_GATED_TOOLS / \
             KB_RATCHETING_TOOLS if it names or writes a base."
        );
        // …and every name the gate lists really is a tool, so a rename cannot
        // silently empty either list.
        for name in KB_ID_GATED_TOOLS.iter().chain(KB_RATCHETING_TOOLS.iter()) {
            assert!(
                exposed.contains(*name),
                "{name} is gated but is not a tool this server exposes"
            );
        }
        for name in KB_RATCHETING_TOOLS {
            assert!(
                KB_ID_GATED_TOOLS.contains(name),
                "{name} ratchets but is not kb_id-gated, so `gated_kb_id` returns \
                 None for it and the raise never runs"
            );
        }
    }

    // ── kb_delete_base ───────────────────────────────────────────────────────

    /// An unreadable page tree is an UNKNOWN count, never zero.
    ///
    /// The approval card already fails soft this way — `describe_base` hands
    /// `deletion_card` an `Option` and it says "its knowledge pages" when the
    /// walk fails — and the summary is documented as the same number the card
    /// showed. `unwrap_or(0)` broke that agreement in the one direction that
    /// matters: the model then tells the user "0 knowledge pages removed" about
    /// a base whose contents were merely unreadable, and "0 pages" reads as
    /// "nothing was lost".
    ///
    /// The `knowledge/` symlink is how the walk is made to fail without making
    /// the delete fail: `list_pages` refuses a path that escapes the base
    /// (`ensure_existing_path_confined`), while the staged rename and the
    /// removal that follow it only ever touch the link, not its target.
    ///
    /// ⚠ `#[cfg(unix)]` because the FIXTURE is POSIX, not because the behaviour
    /// is: `std::os::unix::fs::symlink` does not exist on Windows, where a
    /// symlink needs either developer mode or an elevated process. The rule it
    /// pins — an unreadable tree is `None` — holds on every platform, and the
    /// `.ok()` that implements it is not gated. Same lesson as the 25 POSIX
    /// fixtures in `security::sensitive_ops`: gate the fixture, never the rule.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_unreadable_page_tree_reports_an_unknown_count_not_zero() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["scratch"]);
        let outside = root.join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();

        let knowledge = root.join("scratch").join("knowledge");
        if knowledge.exists() {
            std::fs::remove_dir_all(&knowledge).unwrap();
        }
        std::os::unix::fs::symlink(&outside, &knowledge).unwrap();
        assert!(
            crate::knowledge::store::list_pages(&root.join("scratch"), None).is_err(),
            "the fixture must actually make the walk fail, or this proves nothing"
        );

        let out = call_tool_as(
            &srv,
            "kb_delete_base",
            serde_json::json!({ "kb_id": "scratch" }),
            Private,
        )
        .await;
        let summary = json_of(&out);
        assert_eq!(summary["deleted"], true, "{summary}");
        assert!(
            summary["pages_removed"].is_null(),
            "an unreadable tree is an unknown count, and `0` reads as \"nothing was \
             lost\": {summary}"
        );
    }

    /// The whole transaction, from the model's seam.
    ///
    /// ⚠ The directory is the least of it. A base that is gone from disk but
    /// still named in `registry.yaml` is worse than one that is still there:
    /// every listing shows it, every read of it fails, and the user cannot
    /// remove what does not exist. The selection files are the same shape one
    /// layer up — `docs`' own note is that the primary must remain visible and
    /// the daemon repairs selection when a base is hidden or deleted, which it
    /// cannot do for a pointer nothing ever cleared.
    #[tokio::test]
    async fn deleting_a_base_removes_its_directory_registry_row_and_every_pointer_at_it() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["scratch", "keeper"]);
        seed_page(&root, "scratch", "knowledge/a.md", "SCRATCH-BODY");
        seed_page(&root, "scratch", "knowledge/b.md", "MORE-SCRATCH");

        // Every pointer shape the delete has to clear: the machine-wide
        // primary, a per-session primary, the machine-wide hidden set and a
        // per-session hidden set.
        let svc = KnowledgeService::new(root.clone());
        svc.set_primary_persisted(Some("scratch")).unwrap();
        svc.set_primary_for_session("sess-1", Some("scratch"))
            .unwrap();
        svc.set_hidden_persisted(&["scratch".to_string(), "keeper".to_string()])
            .unwrap();
        svc.set_hidden_for_session("sess-2", &["scratch".to_string()])
            .unwrap();

        let out = call_tool_as(
            &srv,
            "kb_delete_base",
            serde_json::json!({ "kb_id": "scratch" }),
            Private,
        )
        .await;
        let summary = json_of(&out);
        assert_eq!(summary["kb_id"], "scratch");
        assert_eq!(summary["deleted"], true);
        assert_eq!(
            summary["pages_removed"], 2,
            "the summary must report what was actually lost: {summary}"
        );

        // 1. The directory, `.git` and all.
        assert!(
            !root.join("scratch").exists(),
            "the base directory survived the delete"
        );
        // …and no staging residue: a `.deleting-<id>-<uuid>` left behind is a
        // full copy of the base the user believes is gone.
        let leftovers: Vec<String> = std::fs::read_dir(&root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(".deleting-"))
            .collect();
        assert!(leftovers.is_empty(), "staged delete residue: {leftovers:?}");

        // 2. The registry row.
        let registry = crate::knowledge::registry::load(&root).unwrap();
        assert!(
            !registry.iter().any(|entry| entry.id == "scratch"),
            "the registry still names a base that no longer exists"
        );
        assert!(
            registry.iter().any(|entry| entry.id == "keeper"),
            "the delete took an unrelated base's registry row with it"
        );

        // 3. Every selection pointing at it — and nothing pointing at anything
        //    else.
        let svc = KnowledgeService::new(root.clone());
        assert_eq!(
            svc.primary_for_session(None).unwrap().as_deref(),
            None,
            "the machine-wide primary still points at a deleted base"
        );
        assert_eq!(
            stored_primary(&root, "sess-1"),
            None,
            "a session's primary still points at a deleted base"
        );
        assert_eq!(
            svc.get_hidden_persisted().unwrap(),
            vec!["keeper".to_string()],
            "the machine-wide hidden set kept a deleted id (and must keep the others)"
        );
        assert_eq!(
            svc.get_hidden_for_session("sess-2").unwrap(),
            Vec::<String>::new(),
            "a session's hidden set kept a deleted id"
        );
        assert!(
            !crate::knowledge::tier::has_metadata_unlocked(&root, "scratch").unwrap(),
            "the classification record outlived the base it classified"
        );

        // 4. The id is genuinely free again — the strongest single statement
        //    that nothing was left behind, since `create_base` refuses an id
        //    whose directory or registry row still exists.
        srv.service
            .create_base("scratch", "Scratch", None)
            .expect("the deleted id must be reusable");
    }

    /// The other half of "delete a base you created": the model must be able to
    /// see, straight afterwards, that it is gone.
    #[tokio::test]
    async fn a_deleted_base_stops_being_listed() {
        let (srv, _tmp, _root) = migrated_server_with_bases(&["scratch", "keeper"]);
        let before =
            json_of(&call_tool_as(&srv, "kb_list_bases", serde_json::json!({}), Private).await);
        assert_eq!(before.as_array().unwrap().len(), 2);

        call_tool_as(
            &srv,
            "kb_delete_base",
            serde_json::json!({ "kb_id": "scratch" }),
            Private,
        )
        .await
        .expect("the delete must succeed");

        let after =
            json_of(&call_tool_as(&srv, "kb_list_bases", serde_json::json!({}), Private).await);
        let ids: Vec<&str> = after
            .as_array()
            .unwrap()
            .iter()
            .map(|base| base["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            vec!["keeper"],
            "the listing still names the deleted base"
        );
    }

    /// Issue #56. Destruction is at least as sensitive as a read, and the
    /// refusal has to say what it refused.
    ///
    /// ⚠ The second half is the one that matters. A gate that answered "kb
    /// 'omop' not found" would look like a refusal and be a *worse* outcome
    /// than none: the model would conclude the base is already gone and report
    /// the cleanup as done. This tool is not `kb_set_active`, whose non-member
    /// answer is deliberately indistinguishable from a missing id — that
    /// exemption exists because moving a pointer to a base you cannot see is a
    /// no-op, whereas here the caller has just been stopped from destroying
    /// something and needs to know that is what happened.
    #[tokio::test]
    async fn a_public_caller_cannot_delete_a_private_base_and_is_told_so() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["omop"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        seed_page(&root, "omop", "knowledge/x.md", "SENTINEL-BODY");
        let before = kb_fingerprint(&root, "omop");

        let out = call_tool_as(
            &srv,
            "kb_delete_base",
            serde_json::json!({ "kb_id": "omop" }),
            Public,
        )
        .await;
        assert!(
            is_privacy_refusal(&out),
            "a public caller deleted, or was refused for the wrong reason: {}",
            rendered(&out)
        );
        assert!(
            root.join("omop").exists(),
            "the base was deleted despite the refusal"
        );
        assert_eq!(
            kb_fingerprint(&root, "omop"),
            before,
            "the refused delete still touched the base"
        );
        assert!(
            crate::knowledge::registry::load(&root)
                .unwrap()
                .iter()
                .any(|entry| entry.id == "omop"),
            "the refused delete unregistered the base"
        );

        // …and the barrier is the only thing standing in the way: a private
        // caller deletes the same base, or "cannot delete" is satisfied by
        // "delete never works".
        call_tool_as(
            &srv,
            "kb_delete_base",
            serde_json::json!({ "kb_id": "omop" }),
            Private,
        )
        .await
        .expect("a private caller must be able to delete its own private base");
        assert!(!root.join("omop").exists());
    }

    /// #157. An open transaction holds the base's write lock for its whole
    /// lifetime, so a delete underneath one does not race — it parks on `flock`
    /// for the full write deadline, taking the caller and every other session
    /// with it. Refusing is also the honest answer: there is uncommitted work
    /// on that base.
    #[tokio::test]
    async fn a_base_with_an_open_transaction_refuses_to_be_deleted() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["busy"]);
        let txn = call_tool_json_as_session(
            &srv,
            "kb_begin_txn",
            serde_json::json!({ "kb_id": "busy", "label": "in-flight" }),
            "sess-1",
            Private,
        )
        .await;
        let handle = txn["txn"]
            .as_str()
            .unwrap_or_else(|| panic!("kb_begin_txn must report its handle: {txn}"))
            .to_string();

        let out = call_tool_as_session(
            &srv,
            "kb_delete_base",
            serde_json::json!({ "kb_id": "busy" }),
            Some("sess-1"),
            Private,
        )
        .await;
        assert!(
            matches!(&out, Err(e) if e.message.contains(TRANSACTION_UNAVAILABLE)),
            "an open transaction did not stop the delete: {}",
            rendered(&out)
        );
        assert!(root.join("busy").exists(), "the base was deleted anyway");

        // ⚠ And the refusal is not permanent: once the transaction is closed
        // the base deletes normally. A gate that wedged the base would have
        // reproduced #157 rather than avoided it.
        call_tool_as_session(
            &srv,
            "kb_abort_txn",
            serde_json::json!({ "kb_id": "busy", "txn": handle }),
            Some("sess-1"),
            Private,
        )
        .await
        .expect("the transaction must abort");
        call_tool_as_session(
            &srv,
            "kb_delete_base",
            serde_json::json!({ "kb_id": "busy" }),
            Some("sess-1"),
            Private,
        )
        .await
        .expect("the base must delete once nothing holds it");
        assert!(!root.join("busy").exists());
    }

    /// Soul is refused to the MODEL, and the refusal names what to do instead.
    ///
    /// ⚠ It is refused here and NOT in `KnowledgeService::delete_base`, which
    /// `routes::reset::reset_knowledge` calls over every base including this
    /// one. Pushing the guard down a layer would break the factory reset — and
    /// the user's own delete in the Knowledge view is deliberately left alone:
    /// a person who insists proceeds past a warning, an agent never does the
    /// same thing automatically.
    #[tokio::test]
    async fn the_built_in_soul_base_is_refused_to_the_model_but_not_to_the_service() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["soul", "scratch"]);

        let out = call_tool_as(
            &srv,
            "kb_delete_base",
            serde_json::json!({ "kb_id": "soul" }),
            Private,
        )
        .await;
        let message = rendered(&out);
        assert!(out.is_err(), "soul was deleted by the model: {message}");
        assert!(
            message.contains("kb_set_active") || message.contains("hide"),
            "the refusal must name what to do instead: {message}"
        );
        assert!(root.join("soul").exists());

        // The same id, one layer down, still deletes — this is the path the
        // factory reset takes.
        srv.service
            .delete_base("soul")
            .expect("the service must still be able to delete soul (factory reset)");
        assert!(!root.join("soul").exists());

        // …and the refusal is about that one id, not about deletion.
        call_tool_as(
            &srv,
            "kb_delete_base",
            serde_json::json!({ "kb_id": "scratch" }),
            Private,
        )
        .await
        .expect("an ordinary base must still delete");
    }

    /// The primary must remain coherent. A session whose primary is deleted
    /// falls back — it does not keep a dangling pointer, and it does not lose
    /// the ability to write.
    #[tokio::test]
    async fn deleting_a_sessions_primary_leaves_a_coherent_selection() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["scratch", "keeper"]);
        set_primary(&root, "sess-1", "scratch");
        assert_eq!(stored_primary(&root, "sess-1"), Some("scratch".to_string()));

        call_tool_as_session(
            &srv,
            "kb_delete_base",
            serde_json::json!({ "kb_id": "scratch" }),
            Some("sess-1"),
            Private,
        )
        .await
        .expect("the delete must succeed");

        // The pointer is cleared rather than left naming a base that is gone.
        assert_eq!(
            stored_primary(&root, "sess-1"),
            None,
            "the session kept a primary pointing at a deleted base"
        );

        // And the session is still usable: `kb_get_active` answers, and nothing
        // it reports names the deleted base.
        let active = call_tool_json_as_session(
            &srv,
            "kb_get_active",
            serde_json::json!({}),
            "sess-1",
            Private,
        )
        .await;
        assert!(
            !active.to_string().contains("scratch"),
            "the selection view still names the deleted base: {active}"
        );

        // A KB-less write in that session must not land in the base that was
        // deleted, and must not be answered with a dangling id.
        let out = call_tool_as_session(
            &srv,
            "kb_append_log",
            serde_json::json!({ "kind": "manual", "summary": "after the delete" }),
            Some("sess-1"),
            Private,
        )
        .await;
        assert!(
            !rendered(&out).contains("scratch"),
            "a KB-less write after the delete still resolved to the deleted base: {}",
            rendered(&out)
        );
    }

    /// Deleting is not a way to reach a base you may not name. The id is
    /// validated before anything opens a path, exactly as every other gated
    /// tool's is.
    #[tokio::test]
    async fn a_traversing_kb_id_is_rejected_before_anything_is_removed() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["keeper"]);
        for bad in ["../keeper", "keeper/../keeper", "KEEPER"] {
            let out = call_tool_as(
                &srv,
                "kb_delete_base",
                serde_json::json!({ "kb_id": bad }),
                Private,
            )
            .await;
            assert!(out.is_err(), "{bad} was accepted as a kb id");
        }
        assert!(root.join("keeper").exists());
    }

    /// The tool description has to state the thing that makes this different
    /// from every other `kb_*` tool: it cannot be undone, and `kb_restore_state`
    /// is not the escape hatch a model would reasonably assume it is.
    #[test]
    fn the_delete_tool_warns_that_restore_cannot_bring_a_base_back() {
        let tool = declared_tool("kb_delete_base");
        let description = tool.description.as_deref().unwrap_or_default();
        assert!(description.contains("kb_restore_state"), "{description}");
        assert!(description.contains("kb_export"), "{description}");
        let annotations = tool
            .annotations
            .as_ref()
            .expect("kb_delete_base must be annotated, or it grades Unknown");
        assert_eq!(
            annotations.destructive_hint,
            Some(true),
            "the risk grader reads this; without it the tool grades Unknown \
             rather than High"
        );
        assert_eq!(annotations.read_only_hint, Some(false));
    }

    // ── Stage 4: the format argument and the validator ──────────────────────

    /// The default is OKF, and it is the *default*, not a fallback: a call that
    /// says nothing about the format is a legitimate call, not a broken one.
    #[tokio::test]
    async fn a_base_created_without_a_format_is_okf() {
        let (srv, _tmp, root) = migrated_server_with_bases(&[]);
        call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "notes", "name": "Notes" }),
            Public,
        )
        .await
        .unwrap();
        let m = crate::knowledge::manifest::load(&root.join("notes")).unwrap();
        assert_eq!(m.profile(), Some(crate::knowledge::types::KbFormat::Okf));
        assert_eq!(m.biookf_version, None);
    }

    /// The model asks and gets what it asked for, all the way to disk — the
    /// manifest, the declared revision and the `schema.md` the sub-agent is
    /// later taught from.
    #[tokio::test]
    async fn a_model_can_ask_for_a_biookf_base_and_gets_one() {
        let (srv, _tmp, root) = migrated_server_with_bases(&[]);
        call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "lit", "name": "Literature", "format": "biookf" }),
            Public,
        )
        .await
        .unwrap();
        let m = crate::knowledge::manifest::load(&root.join("lit")).unwrap();
        assert_eq!(m.profile(), Some(crate::knowledge::types::KbFormat::Biookf));
        assert_eq!(
            m.biookf_version.as_deref(),
            Some(crate::knowledge::biookf::BIOOKF_VERSION)
        );
        let schema = std::fs::read_to_string(root.join("lit/schema.md")).unwrap();
        assert!(
            schema.contains("Molecule"),
            "the BioOKF vocabulary is taught"
        );
    }

    /// ⚠ The failure this refuses is silent by default. `KbFormat`'s own
    /// `Deserialize` is lenient on purpose (DR-12: a manifest that fails to load
    /// costs the user their pointers), so a typed `Option<KbFormat>` parameter
    /// would have read `bio-okf` as OKF, created a plain base, returned success,
    /// and left the model to discover it pages later — with no conversion
    /// available (DR-26). A request is not a file: it is refused, and the
    /// refusal names both legal values so the retry is the right one.
    #[tokio::test]
    async fn an_unknown_format_is_refused_and_names_the_two_that_exist() {
        let (srv, _tmp, root) = migrated_server_with_bases(&[]);
        let out = call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "lit", "name": "Literature", "format": "bio-okf" }),
            Public,
        )
        .await;
        let message = rendered(&out);
        assert!(out.is_err(), "a misspelt format created a base: {message}");
        assert!(
            message.contains("okf") && message.contains("biookf"),
            "{message}"
        );
        assert!(
            !root.join("lit").exists(),
            "a refused create left a base behind"
        );
    }

    /// The Stage 4 gate, in one line: adding an argument to the tool did not
    /// displace the ratchet that rides in the same call.
    #[tokio::test]
    async fn a_biookf_base_created_from_a_private_chat_is_still_born_private() {
        let (srv, _tmp, root) = migrated_server_with_bases(&[]);
        call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "cohort", "name": "Cohort", "format": "biookf" }),
            Private,
        )
        .await
        .unwrap();
        assert!(crate::knowledge::tier::is_private(&root, "cohort"));
        assert_eq!(
            crate::knowledge::manifest::load(&root.join("cohort"))
                .unwrap()
                .profile(),
            Some(crate::knowledge::types::KbFormat::Biookf)
        );
    }

    /// The tool's reason to exist: catching an invented predicate on one draft
    /// instead of at the end of an ingest that wrote twelve pages.
    #[tokio::test]
    async fn validate_flags_a_biookf_page_and_leaves_an_okf_one_alone() {
        let (srv, _tmp, _root) = migrated_server_with_bases(&[]);
        for (id, format) in [("lit", "biookf"), ("notes", "okf")] {
            call_tool_as(
                &srv,
                "kb_create_base",
                serde_json::json!({ "id": id, "name": id, "format": format }),
                Public,
            )
            .await
            .unwrap();
        }
        let draft = "---\ntype: Molecule\nidentifier: Aspirin\nedges:\n  - predicate: heals\n    \
                     object: Headache\n---\n\n# Aspirin\n";

        let strict = json_of(
            &call_tool_as(
                &srv,
                "kb_validate_page",
                serde_json::json!({
                    "kb_id": "lit", "path": "knowledge/molecule/aspirin.md", "content": draft
                }),
                Public,
            )
            .await,
        );
        assert_eq!(strict["ok"], false, "{strict}");
        assert_eq!(strict["format"], "biookf");
        let rules: Vec<&str> = strict["diagnostics"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["rule"].as_str().unwrap())
            .collect();
        assert!(
            rules.contains(&crate::knowledge::biookf::lint::RULE_PREDICATE_INVALID),
            "{rules:?}"
        );
        // Every diagnostic is actionable without re-reading the page.
        for d in strict["diagnostics"]["items"].as_array().unwrap() {
            assert!(!d["subject"].as_str().unwrap().is_empty(), "{d}");
            assert!(!d["message"].as_str().unwrap().is_empty(), "{d}");
            assert!(matches!(
                d["severity"].as_str().unwrap(),
                "error" | "warning" | "info"
            ));
        }

        // The same bytes in an OKF base: `Molecule` and `heals` are just words.
        let open = json_of(
            &call_tool_as(
                &srv,
                "kb_validate_page",
                serde_json::json!({
                    "kb_id": "notes", "path": "knowledge/aspirin.md", "content": draft
                }),
                Public,
            )
            .await,
        );
        assert_eq!(open["ok"], true, "{open}");
        assert_eq!(open["format"], "okf");
    }

    /// DR-7, at the one place a caller might mistake a diagnostic for a
    /// rejection: validating writes nothing, commits nothing, and creates
    /// nothing. It is a question, not a dry-run of a write.
    #[tokio::test]
    async fn validate_writes_nothing() {
        let (srv, _tmp, root) = migrated_server_with_bases(&[]);
        call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "lit", "name": "Lit", "format": "biookf" }),
            Public,
        )
        .await
        .unwrap();
        let before = crate::knowledge::store::list_pages(&root.join("lit"), None).unwrap();
        call_tool_as(
            &srv,
            "kb_validate_page",
            serde_json::json!({
                "kb_id": "lit",
                "path": "knowledge/molecule/aspirin.md",
                "content": "---\ntype: Molecule\nidentifier: Aspirin\n---\n\n# Aspirin\n",
            }),
            Public,
        )
        .await
        .unwrap();
        let after = crate::knowledge::store::list_pages(&root.join("lit"), None).unwrap();
        assert_eq!(before, after);
    }

    /// A retired pre-OKF base must never be treated as an empty OKF validation.
    /// Startup purges these bases; this boundary catches an out-of-band stale
    /// store before an agent can extend it with more legacy pages.
    #[tokio::test]
    async fn validate_refuses_a_legacy_base() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["old"]);
        let kb = root.join("old");
        let mut m = crate::knowledge::manifest::load(&kb).unwrap();
        m.schema_version = 1;
        crate::knowledge::manifest::save(&kb, &m).unwrap();

        let out = call_tool_as(
            &srv,
            "kb_validate_page",
            serde_json::json!({
                "kb_id": "old",
                "path": "knowledge/a.md",
                "content": "---\ntitle: A\nkind: entity\n---\n\nbody\n",
            }),
            Public,
        )
        .await;
        assert!(out.is_err(), "{}", rendered(&out));
        assert!(
            rendered(&out).contains("retired pre-OKF"),
            "{}",
            rendered(&out)
        );
    }

    /// `kb_lint` exists as a tool at all, and answers about the base rather than
    /// about a draft the caller already holds.
    ///
    /// It did not, and two shipped skills were written around calling it —
    /// `knowledge-lint` names it five times, `knowledge-ingest-biookf` tells the
    /// agent to "run kb_lint over the base and fix every …". Lint was reachable
    /// only from HTTP and the CLI, so an agent that had just written a dozen
    /// pages could not check its own work, and following Biorouter's own skill
    /// produced "unknown tool".
    #[tokio::test]
    async fn kb_lint_reports_the_whole_bases_findings() {
        let (srv, _tmp, root) = migrated_server_with_bases(&[]);
        call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "lit", "name": "Lit", "format": "biookf" }),
            Public,
        )
        .await
        .unwrap();
        call_tool_as(
            &srv,
            "kb_write_page",
            serde_json::json!({
                "kb_id": "lit",
                "path": "knowledge/molecule/aspirin.md",
                // `Molecules` is not one of the 28 — a whole-base finding a
                // caller only sees by asking about the base.
                "content": "---\ntype: Molecules\nidentifier: Aspirin\n---\n\n# Aspirin\n",
                "commit_message": "m",
            }),
            Public,
        )
        .await
        .unwrap();

        let out = json_of(
            &call_tool_as(&srv, "kb_lint", serde_json::json!({"kb_id": "lit"}), Public).await,
        );
        assert_eq!(out["format"], "biookf", "{out}");
        assert_eq!(out["ok"], false, "{out}");
        assert!(out["errors"].as_u64().unwrap() >= 1, "{out}");
        // The pre-cap total travels beside the capped list, so a caller can tell
        // "that is everything" from "that is the first two hundred".
        assert_eq!(out["truncated"], false, "{out}");
        assert!(out["diagnostics"]["total"].as_u64().unwrap() >= 1, "{out}");
        let rules: Vec<&str> = out["diagnostics"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["rule"].as_str().unwrap())
            .collect();
        assert!(
            rules.contains(&crate::knowledge::biookf::lint::RULE_TYPE_INVALID),
            "{rules:?}"
        );

        // …and it wrote nothing doing it: a lint is a question, not a dry run.
        let before = crate::knowledge::store::list_pages(&root.join("lit"), None).unwrap();
        call_tool_as(&srv, "kb_lint", serde_json::json!({"kb_id": "lit"}), Public)
            .await
            .unwrap();
        assert_eq!(
            before,
            crate::knowledge::store::list_pages(&root.join("lit"), None).unwrap()
        );
    }

    #[tokio::test]
    async fn kb_lint_reauthorizes_after_waiting_for_the_kb_lock() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["lit"]);
        let private_writer = KnowledgeService::new(root.clone());
        let writer_lock = private_writer.lock_kb("lit").await.unwrap();

        let lint_server = srv.clone();
        let lint = tokio::spawn(async move {
            call_tool_as(
                &lint_server,
                "kb_lint",
                serde_json::json!({"kb_id": "lit"}),
                Public,
            )
            .await
        });

        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !srv.service.kb_queue_is_occupied("lit") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("lint did not reach the cross-process KB lock wait");

        private_writer
            .raise_tier_and_affiliation("lit", true, &CallerAffiliation::Unstated)
            .unwrap();
        crate::knowledge::store::write_page(
            &root.join("lit"),
            "knowledge/observation/private.md",
            "---\ntype: Observation\nidentifier: Private note\n---\n\nprivate\n",
            "private writer",
            None,
        )
        .unwrap();
        drop(writer_lock);

        let result = tokio::time::timeout(std::time::Duration::from_secs(3), lint)
            .await
            .expect("lint did not resume after the writer released the KB lock")
            .unwrap();
        assert!(
            is_privacy_refusal(&result),
            "public lint read a base that became private while queued: {}",
            rendered(&result)
        );
    }

    #[tokio::test]
    async fn kb_lint_waits_until_a_partial_transaction_is_gone() {
        let (srv, _tmp, root) = migrated_server_with_bases(&[]);
        call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "lit", "name": "Lit", "format": "biookf" }),
            Public,
        )
        .await
        .unwrap();

        let lock = srv.service.lock_kb("lit").await.unwrap();
        let kb_root = root.join("lit");
        let repo = GitRepo::open(&kb_root).unwrap();
        let txn = repo.begin_txn("partial-lint-fixture").unwrap();
        crate::knowledge::store::write_page(
            &kb_root,
            "knowledge/molecule/transient.md",
            "---\ntype: Molecules\nidentifier: Transient\n---\n\n# Transient\n",
            "partial transaction page",
            Some(&txn.branch),
        )
        .unwrap();

        let lint_server = srv.clone();
        let mut lint = tokio::spawn(async move {
            call_tool_as(
                &lint_server,
                "kb_lint",
                serde_json::json!({"kb_id": "lit"}),
                Public,
            )
            .await
        });
        tokio::task::yield_now().await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut lint)
                .await
                .is_err(),
            "kb_lint bypassed the KB transaction queue"
        );

        repo.abort_txn(&txn).unwrap();
        drop(lock);

        let result = tokio::time::timeout(std::time::Duration::from_secs(3), lint)
            .await
            .expect("lint should resume after the transaction lock is released")
            .unwrap();
        let out = json_of(&result);
        let rules = out["diagnostics"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|diagnostic| diagnostic["rule"].as_str())
            .collect::<Vec<_>>();
        assert!(
            !rules.contains(&crate::knowledge::biookf::lint::RULE_TYPE_INVALID),
            "lint observed the aborted transaction's transient page: {out}"
        );
        assert!(
            !kb_root.join("knowledge/molecule/transient.md").exists(),
            "the transaction fixture did not abort cleanly"
        );
    }

    #[tokio::test]
    async fn cancelling_kb_lint_while_it_waits_for_the_transaction_queue_returns() {
        let (srv, _tmp, _root) = migrated_server_with_bases(&["lit"]);
        let lock = srv.service.lock_kb("lit").await.unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let call_cancel = cancel.clone();
        let lint_server = srv.clone();
        let mut lint = tokio::spawn(async move {
            call_tool_as_full_with_cancellation(
                &lint_server,
                "kb_lint",
                serde_json::json!({"kb_id": "lit"}),
                None,
                Public,
                &CallerAffiliation::Unstated,
                call_cancel,
            )
            .await
        });

        tokio::task::yield_now().await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut lint)
                .await
                .is_err(),
            "kb_lint completed without waiting for the held KB lock"
        );
        cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), lint)
            .await
            .expect("cancellation should interrupt the KB lock wait")
            .unwrap();
        let error = result.expect_err("a cancelled lint must not return a report");
        assert!(error.message.contains("cancelled"), "{error:?}");

        drop(lock);
    }

    /// A KB-less `kb_lint` lints the session's primary, like every other
    /// single-base read. `KB_PRIMARY_RESOLVING_TOOLS` is what makes the barrier
    /// resolve the same id the tool will, so "just drop the kb_id" is not the
    /// bypass.
    #[tokio::test]
    async fn kb_lint_with_no_kb_id_lints_the_primary() {
        let (srv, _tmp, _root) = migrated_server_with_bases(&["alpha", "beta"]);
        call_tool_as(
            &srv,
            "kb_set_active",
            serde_json::json!({ "kb_id": "beta" }),
            Public,
        )
        .await
        .unwrap();
        let out = json_of(&call_tool_as(&srv, "kb_lint", serde_json::json!({}), Public).await);
        assert_eq!(out["kb_id"], "beta", "{out}");
    }

    #[tokio::test]
    async fn a_base_created_from_a_private_chat_is_born_private() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["default"]);
        call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "omop", "name": "OMOP" }),
            Private,
        )
        .await
        .unwrap();
        assert!(crate::knowledge::tier::is_private(&root, "omop"));
        assert!(
            !crate::knowledge::tier::is_private(&root, "default"),
            "creating one base moved another"
        );
    }

    /// DR-18(b), pinned here as well as in Task 10B's
    /// `a_base_created_from_a_private_chat_is_born_private`, because this is the
    /// assertion a reader lands on when they ask "when does a base become
    /// private?" — and the answer is *at creation*, not at the first ingest.
    ///
    /// The extra thing this says over its sibling is the timing: NOTHING has been
    /// written into the base at the moment it is classified. No page, no raw
    /// source, no macro run. An implementation that stamped the tier from the
    /// first write would leave a window in which a private session's own base is
    /// readable by every public model, and its sibling — which only checks the
    /// end state — would pass.
    #[tokio::test]
    async fn a_base_created_by_a_private_model_is_private_before_any_ingest() {
        let (srv, _tmp, root) = migrated_server_with_bases(&[]);
        call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "cohort", "name": "Cohort" }),
            Private,
        )
        .await
        .unwrap();
        assert!(crate::knowledge::tier::is_private(&root, "cohort"));
        assert!(
            crate::knowledge::store::list_pages(&root.join("cohort"), None)
                .unwrap()
                .is_empty(),
            "the base was classified only after something was written into it"
        );
        assert!(
            std::fs::read_dir(root.join("cohort").join("raw"))
                .unwrap()
                .next()
                .is_none(),
            "the base already holds a raw source, so this proves nothing about timing"
        );

        // The symmetric half: a public chat's brand-new base is public from the
        // same instant, so the assertion above is not satisfied by "everything is
        // private".
        call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "notes", "name": "Notes" }),
            Public,
        )
        .await
        .unwrap();
        assert!(!crate::knowledge::tier::is_private(&root, "notes"));
    }

    #[tokio::test]
    async fn a_public_chat_can_still_create_and_import_a_knowledge_base() {
        // The regression the sixteen-site enumeration encoded, as a test. A
        // public session must be able to make its own base; `assert_reachable`
        // permits a kb id with no directory on disk (Task 10A, decision 3).
        let (srv, _tmp, root) = migrated_server_with_bases(&["default"]);
        call_tool_as(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "notes", "name": "Notes" }),
            Public,
        )
        .await
        .unwrap();
        assert!(!crate::knowledge::tier::is_private(&root, "notes"));

        let (_fx, path) = brkb_fixture(Public);
        let out = call_tool_as(
            &srv,
            "kb_import",
            serde_json::json!({ "src_path": path }),
            Public,
        )
        .await
        .unwrap();
        assert!(!crate::knowledge::tier::is_private(
            &root,
            &imported_kb_id(&out)
        ));
    }

    #[tokio::test]
    async fn kb_import_reports_a_legacy_archive_as_invalid_input() {
        let (srv, _tmp, _root) = migrated_server_with_bases(&["default"]);
        let (_fixture, path) = legacy_brkb_fixture();
        let out = call_tool_as(
            &srv,
            "kb_import",
            serde_json::json!({ "src_path": path }),
            Public,
        )
        .await;
        let error = out.expect_err("the retired archive must be refused");
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_REQUEST);
        assert!(error.message.contains("legacy pre-OKF"), "{error:?}");
    }

    #[tokio::test]
    async fn kb_import_bounds_the_selected_file_before_reading_it() {
        let (srv, _tmp, _root) = migrated_server_with_bases(&["default"]);
        let missing = call_tool_as(
            &srv,
            "kb_import",
            serde_json::json!({ "src_path": "/path/that/does/not/exist.brkb" }),
            Public,
        )
        .await
        .expect_err("an invalid selected path must be a caller-correctable error");
        assert_eq!(missing.code, rmcp::model::ErrorCode::INVALID_REQUEST);
        assert!(
            missing.message.contains("cannot open archive"),
            "{missing:?}"
        );

        let archive = tempfile::NamedTempFile::new().unwrap();
        archive
            .as_file()
            .set_len(crate::knowledge::brkb::MAX_ARCHIVE_FILE_BYTES + 1)
            .unwrap();
        let error = call_tool_as(
            &srv,
            "kb_import",
            serde_json::json!({ "src_path": archive.path() }),
            Public,
        )
        .await
        .expect_err("a sparse over-limit archive must be refused before read-to-end");
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_REQUEST);
        assert!(error.message.contains("128 MiB"), "{error:?}");
    }

    #[tokio::test]
    async fn an_imported_base_takes_the_importing_sessions_tier_or_the_archives_floor() {
        // `brkb::import` resolves collisions by suffixing, so an import always
        // lands on a FRESH id — which is what makes stamping after the call safe.
        let (srv, _tmp, root) = migrated_server_with_bases(&["default"]);
        let (_fx, public_path) = brkb_fixture(Public);
        let out = call_tool_as(
            &srv,
            "kb_import",
            serde_json::json!({ "src_path": public_path }),
            Private,
        )
        .await
        .unwrap();
        assert!(crate::knowledge::tier::is_private(
            &root,
            &imported_kb_id(&out)
        ));

        // ⚠ The line above is the SAFE direction and, on its own, it is what let
        // export-private / import-public through a whole review round: a private
        // importer privatising what it imports proves nothing about a public one.
        // The unsafe direction is Task 10A's
        // `a_private_export_cannot_be_laundered_by_importing_it_into_a_public_chat`;
        // this is its tool-level twin, so the bypass is closed at the surface a
        // model actually calls and not only in the store.
        let (_fx2, private_path) = brkb_fixture(Private);
        let out = call_tool_as(
            &srv,
            "kb_import",
            serde_json::json!({ "src_path": private_path }),
            Public,
        )
        .await
        .unwrap();
        assert!(
            crate::knowledge::tier::is_private(&root, &imported_kb_id(&out)),
            "a public chat imported a private base's archive and got a public base"
        );
    }

    /// ⚠ **An archive is a transfer, so it carries its owners.** DR-26's own
    /// case arriving through export/import, which the two tools whose subject id
    /// is minted by the call are the only way to reach.
    ///
    /// A UCSF chat may `kb_export` a base UCSF owns — it is the owner, so no
    /// gate fires. If the archive drops the owner set, the base a Stanford-bound
    /// chat imports from it reads UNCLAIMED, and `affiliation::reachable` treats
    /// an unclaimed base as permissive for every private model. Both endpoints
    /// are Private, so every tier gate in this campaign says yes and UCSF
    /// content lands in a Stanford model's context with nothing crossed.
    ///
    /// The positive control is the second import: the same archive into a UCSF
    /// chat still reads, or this gate is just "refuse every import".
    #[tokio::test]
    async fn a_ucsf_base_cannot_be_laundered_into_a_stanford_chat_through_an_export() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["default"]);
        let (_fx, path) = brkb_fixture_owned_by("ucsf");

        let out = call_tool_as_affiliated(
            &srv,
            "kb_import",
            serde_json::json!({ "src_path": path }),
            "stanford",
        )
        .await
        .expect("the import itself is not the refusal; what it lands on is what gates");
        let laundered = imported_kb_id(&out);

        let held = crate::knowledge::tier::affiliation(&root, &laundered);
        assert!(
            held.owners().expect("a readable store").contains("ucsf"),
            "the archive's owners did not survive the import: {held:?}"
        );

        let err = call_tool_as_affiliated(
            &srv,
            "kb_search",
            serde_json::json!({ "kb_id": laundered, "query": "SENTINEL" }),
            "stanford",
        )
        .await
        .expect_err("a Stanford model read UCSF content it had imported from an archive");
        let msg = err.to_string();
        assert!(msg.contains("Cross-institutional"), "{msg}");
        assert!(
            !msg.contains("SENTINEL"),
            "the refusal carried content: {msg}"
        );

        // The positive control. A UCSF chat imports the same archive — the
        // collision loop lands it on a fresh id — and reads it, because the
        // content never left the institution that owns it.
        let out = call_tool_as_affiliated(
            &srv,
            "kb_import",
            serde_json::json!({ "src_path": path }),
            "ucsf",
        )
        .await
        .expect("a UCSF chat may import a UCSF archive");
        let mine = imported_kb_id(&out);
        assert_ne!(mine, laundered, "the second import reused the first id");
        call_tool_as_affiliated(
            &srv,
            "kb_search",
            serde_json::json!({ "kb_id": mine, "query": "SENTINEL" }),
            "ucsf",
        )
        .await
        .expect("a UCSF model may read UCSF content it imported itself");
    }

    /// The other id-minting tool, on the same axis. `kb_create_base` stamps the
    /// TIER at creation (DR-18(b),
    /// `a_base_created_from_a_private_chat_is_born_private`); an implementation
    /// that stamped only the tier would leave every base an institutional chat
    /// makes reading as unclaimed until its first `kb_write_page` — and
    /// `create_base_as` is also how the apps runtime and the CLI make one.
    #[tokio::test]
    async fn a_base_created_from_an_institutions_chat_is_born_carrying_that_institution() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["default"]);
        call_tool_as_affiliated(
            &srv,
            "kb_create_base",
            serde_json::json!({ "id": "omop", "name": "OMOP" }),
            "ucsf",
        )
        .await
        .unwrap();
        let held = crate::knowledge::tier::affiliation(&root, "omop");
        assert!(
            held.owners().expect("a readable store").contains("ucsf"),
            "a base born in a UCSF chat records no owner: {held:?}"
        );
        assert!(
            crate::knowledge::tier::affiliation(&root, "default")
                .owners()
                .expect("a readable store")
                .is_empty(),
            "creating one base claimed another"
        );

        // …so another institution's model cannot write into it, and the chat
        // that made it still can — or the stamp is just "claim everything".
        let err = call_tool_as_affiliated(
            &srv,
            "kb_write_page",
            serde_json::json!({
                "kb_id": "omop", "path": "knowledge/x.md",
                "content": valid_page("note", "X", "c"),
                "commit_message": "m"
            }),
            "stanford",
        )
        .await
        .expect_err("a Stanford chat wrote into a base UCSF owns");
        assert!(err.to_string().contains("Cross-institutional"));
        call_tool_as_affiliated(
            &srv,
            "kb_write_page",
            serde_json::json!({
                "kb_id": "omop", "path": "knowledge/x.md",
                "content": valid_page("note", "X", "c"),
                "commit_message": "m"
            }),
            "ucsf",
        )
        .await
        .expect("the institution that made the base may write into it");
    }

    // ── Issue #56, Task 10C: the barrier at CP1 ──────────────────────────────

    /// Everything a call said, whether it answered or refused — one string, so
    /// a leak assertion cannot be satisfied by the payload moving from the
    /// success branch to the error branch.
    fn rendered(out: &Result<CallToolResult, ErrorData>) -> String {
        match out {
            Ok(r) => r
                .content
                .iter()
                .filter_map(|c| c.as_text())
                .map(|t| t.text.clone())
                .collect::<Vec<_>>()
                .join("\n"),
            Err(e) => e.message.to_string(),
        }
    }

    #[tokio::test]
    async fn kb_id_gate_rejects_paths_before_any_store_operation() {
        let (srv, _tmp, root) = migrated_server_with_bases(&[]);
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("knowledge")).unwrap();
        std::fs::write(
            outside.path().join("knowledge/escaped.md"),
            "OUTSIDE-KNOWLEDGE-SENTINEL",
        )
        .unwrap();
        let outside_id = outside.path().to_string_lossy().to_string();

        let explicit = call_tool_as(
            &srv,
            "kb_list_pages",
            serde_json::json!({ "kb_id": outside_id }),
            Private,
        )
        .await;
        assert!(explicit.is_err(), "absolute kb_id was accepted");
        assert!(!rendered(&explicit).contains("OUTSIDE-KNOWLEDGE-SENTINEL"));

        std::fs::write(
            crate::knowledge::paths::primary_kb_path(&root),
            outside.path().to_string_lossy().as_bytes(),
        )
        .unwrap();
        let persisted = call_tool_as(&srv, "kb_list_pages", serde_json::json!({}), Private).await;
        assert!(persisted.is_err(), "invalid persisted primary was accepted");
        assert!(!rendered(&persisted).contains("OUTSIDE-KNOWLEDGE-SENTINEL"));

        for invalid in ["../escape", "nested/base", "UPPER"] {
            let result = call_tool_as(
                &srv,
                "kb_search",
                serde_json::json!({ "kb_id": invalid, "query": "sentinel" }),
                Private,
            )
            .await;
            assert!(result.is_err(), "invalid kb_id {invalid:?} was accepted");
        }
    }

    fn refusal_text(out: &Result<CallToolResult, ErrorData>) -> String {
        match out {
            Ok(_) => panic!("expected a refusal, got: {}", rendered(out)),
            Err(e) => e.message.to_string(),
        }
    }

    fn err_of(out: Result<CallToolResult, ErrorData>) -> String {
        refusal_text(&out)
    }

    /// The one refusal string, recognised rather than re-spelled.
    fn is_privacy_refusal(out: &Result<CallToolResult, ErrorData>) -> bool {
        matches!(out, Err(e) if e.message.contains(crate::knowledge::tier::KB_PRIVATE_REFUSAL))
    }

    fn json_of(out: &Result<CallToolResult, ErrorData>) -> serde_json::Value {
        serde_json::from_str(&rendered(out)).unwrap_or_else(|e| {
            panic!("expected a JSON payload ({e}), got: {}", rendered(out));
        })
    }

    async fn call_tool_json_as_session(
        srv: &KnowledgeServer,
        name: &str,
        args: serde_json::Value,
        session_id: &str,
        caller: Caller,
    ) -> serde_json::Value {
        json_of(&call_tool_as_session(srv, name, args, Some(session_id), caller).await)
    }

    fn set_primary(root: &Path, session_id: &str, kb_id: &str) {
        KnowledgeService::new(root.to_path_buf())
            .set_primary_for_session(session_id, Some(kb_id))
            .unwrap();
    }

    /// An explicit "no primary at this scope" — not an absent file, which would
    /// fall back to the machine-wide pointer.
    fn clear_primary(root: &Path, session_id: &str) {
        KnowledgeService::new(root.to_path_buf())
            .set_primary_for_session(session_id, None)
            .unwrap();
    }

    /// What `.active-kb-sessions/<digest>` really names, read through a fresh
    /// service so no in-process state can answer for it.
    fn stored_primary(root: &Path, session_id: &str) -> Option<String> {
        KnowledgeService::new(root.to_path_buf())
            .primary_for_session(Some(session_id))
            .unwrap()
    }

    /// Every file under one base, with its size — the cheapest thing that
    /// changes when a tool writes and does not change when it reads.
    fn kb_fingerprint(root: &Path, kb_id: &str) -> Vec<(String, u64)> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, u64)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else if let Ok(meta) = std::fs::metadata(&p) {
                    out.push((
                        p.strip_prefix(base)
                            .unwrap_or(&p)
                            .to_string_lossy()
                            .to_string(),
                        meta.len(),
                    ));
                }
            }
        }
        let kb = root.join(kb_id);
        let mut out = Vec::new();
        walk(&kb, &kb, &mut out);
        out.sort();
        out
    }

    async fn base_ids(srv: &KnowledgeServer, caller: Caller) -> Vec<String> {
        let out = call_tool_as(srv, "kb_list_bases", serde_json::json!({}), caller).await;
        json_of(&out)
            .as_array()
            .expect("kb_list_bases returns an array")
            .iter()
            .map(|b| b["id"].as_str().expect("an id").to_string())
            .collect()
    }

    async fn search_hits(
        srv: &KnowledgeServer,
        args: serde_json::Value,
        caller: Caller,
    ) -> Vec<serde_json::Value> {
        let out = call_tool_as(srv, "kb_search", args, caller).await;
        json_of(&out)
            .as_array()
            .expect("kb_search returns an array")
            .clone()
    }

    #[tokio::test]
    async fn the_explicit_kb_id_branch_is_not_a_way_around_the_barrier() {
        // The finding, exactly. Before this task the `kb_id`-carrying branch of
        // `kb_search` searched any base on the machine, and `search_visible_bases`
        // — the only code that consults the session's set — was in the `else`.
        let (srv, _tmp, root) = migrated_server_with_bases(&["default"]);
        crate::knowledge::tier::raise_unlocked(&root, "default", true).unwrap();
        seed_page(
            &root,
            "default",
            "knowledge/omop.md",
            "SENTINEL-COHORT-N-412",
        );

        let out = call_tool_as(
            &srv,
            "kb_search",
            serde_json::json!({ "kb_id": "default", "query": "SENTINEL-COHORT-N-412" }),
            Public,
        )
        .await;
        let text = refusal_text(&out);
        assert!(text.contains("private"), "must say why: {text}");
        assert!(
            !text.contains("SENTINEL-COHORT-N-412"),
            "leaked a snippet: {text}"
        );
        assert!(
            !text.contains("knowledge/omop.md"),
            "leaked a page path: {text}"
        );

        // ⚠ The discrimination half. Without it "nothing leaked" is satisfied by
        // "the tool returns nothing to anybody".
        let out = call_tool_as(
            &srv,
            "kb_search",
            serde_json::json!({ "kb_id": "default", "query": "SENTINEL-COHORT-N-412" }),
            Private,
        )
        .await;
        assert!(
            rendered(&out).contains("SENTINEL-COHORT-N-412"),
            "a private caller lost its own base: {}",
            rendered(&out)
        );
    }

    #[tokio::test]
    async fn no_tool_that_names_a_base_reaches_a_private_one_under_a_public_model() {
        // Parameterised over the twenty base-addressing tools, BY NAME through
        // `call_tool` — the shape CP1 makes possible and a per-tool design could
        // not express for the eight that take no `RequestContext`. `kb_export` is
        // the one to watch: it writes the entire base to an attacker-named path
        // on disk in one call.
        //
        // ⚠ DEVIATION from the task text, recorded rather than hidden. The task
        // says "all NINETEEN" (there are twenty now, `kb_delete_base` having
        // joined them); `kb_create_base` and `kb_import` cannot be probed
        // against an existing id at all (they MINT one, and naming "omop" makes
        // create fail with "already exists" for a reason that has nothing to do
        // with this barrier). They are covered by
        // `a_public_chat_can_still_create_and_import_a_knowledge_base`, and the
        // partition test below is what proves nothing fell between the two sets.
        for probe in KB_TOOL_PROBES {
            let (srv, _tmp, root) = migrated_server_with_bases(&["omop"]);
            crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
            seed_page(&root, "omop", "knowledge/x.md", "SENTINEL-BODY");
            let before = kb_fingerprint(&root, "omop");

            let out = call_tool_as(&srv, probe.name, probe.args_for("omop"), Public).await;
            assert_eq!(
                out.is_err(),
                probe.refused_naming_a_private_base,
                "{} refused={} but the table says {}: {}",
                probe.name,
                out.is_err(),
                probe.refused_naming_a_private_base,
                rendered(&out)
            );
            assert!(
                !rendered(&out).contains("SENTINEL-BODY"),
                "{} leaked a body: {}",
                probe.name,
                rendered(&out)
            );
            if probe.refused_naming_a_private_base {
                assert_eq!(
                    kb_fingerprint(&root, "omop"),
                    before,
                    "{} wrote anyway",
                    probe.name
                );
            }

            // …and the SAME call from a PRIVATE caller must not be refused for
            // privacy, or "no leak" is satisfied by "the tools return nothing".
            // Not `.is_ok()`: several of these fail on their own terms against a
            // fresh base (`kb_commit_txn` has no branch to squash), which has
            // nothing to do with this barrier.
            let public_text = rendered(&out);
            let out = call_tool_as(&srv, probe.name, probe.args_for("omop"), Private).await;
            assert!(
                !is_privacy_refusal(&out),
                "{} refused a private caller: {}",
                probe.name,
                rendered(&out)
            );
            if probe.refused_naming_a_private_base {
                assert_ne!(
                    rendered(&out),
                    public_text,
                    "{} answered a private caller with the public caller's refusal",
                    probe.name
                );
            }
        }
    }

    #[tokio::test]
    async fn omitting_the_kb_id_is_not_the_bypass() {
        // `kb_id_or_primary` resolves an absent id to the session's primary, so a
        // handler that only checks an EXPLICIT kb_id is bypassed by deleting one
        // argument. Four tools take that path.
        let (srv, _tmp, root) = migrated_server_with_bases(&["omop"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        set_primary(&root, "sess-1", "omop");
        for tool in [
            "kb_read_page",
            "kb_list_pages",
            "kb_get_graph",
            "kb_list_history",
        ] {
            let out = call_tool_as_session(
                &srv,
                tool,
                serde_json::json!({ "path": "knowledge/x.md" }),
                Some("sess-1"),
                Public,
            )
            .await;
            assert!(
                out.is_err(),
                "{tool} answered from the primary without a check: {}",
                rendered(&out)
            );
        }
    }

    #[tokio::test]
    async fn the_tool_operates_on_the_base_cp1_checked_rather_than_resolving_a_second_one() {
        // Issue #56, review round 5. CP1 resolves the target (`gated_kb_id`),
        // checks it against the barrier, ratchets it — and then handed the
        // request on unchanged, so the TOOL resolved it a SECOND time from the
        // same argument (and, for the four primary-resolving tools, from on-disk
        // pointer state) on the far side of an `.await`. Two resolutions with
        // the barrier between them is a TOCTOU: whatever moves in that window,
        // the tool acts on a base CP1 never saw. `call_tool` now PINS its
        // resolved id into the arguments, so there is exactly one resolution.
        //
        // ⚠ What this asserts and what it does not. The cross-session window —
        // another chat or the Knowledge view moving the machine-wide
        // `.active-kb` between the two reads — needs an interleave no test in
        // this suite can schedule, and it is NOT what fails below. What fails
        // below is the pin's one deterministic observable: `gated_kb_id`
        // normalises with `str::trim` and the tools did not, so a padded id was
        // checked as `alpha` and then acted on as `"  alpha  "` — a different
        // base by every path this module builds. One fix closes both, because
        // both are the same defect: the id the barrier vetted is not the id the
        // tool used.
        let (srv, _tmp, root) = migrated_server_with_bases(&["alpha"]);
        seed_page(&root, "alpha", "knowledge/a.md", "PINNED");

        // (1) the read path.
        let out = call_tool_as(
            &srv,
            "kb_list_pages",
            serde_json::json!({ "kb_id": "  alpha  " }),
            Public,
        )
        .await;
        let listed = rendered(&out);
        assert!(
            listed.contains("knowledge/a.md"),
            "kb_list_pages read a base CP1 did not check, got: {listed}"
        );

        // (2) the write path, which also ratchets. CP1 raised the tier of
        // `alpha`; a write that then lands anywhere else has put content into a
        // base whose tier never moved.
        let out = call_tool_as(
            &srv,
            "kb_write_page",
            serde_json::json!({
                "kb_id": "  alpha  ",
                "path": "knowledge/b.md",
                "content": valid_page("note", "B", "PINNED-WRITE"),
                "commit_message": "pin",
            }),
            Private,
        )
        .await;
        assert!(out.is_ok(), "kb_write_page: {}", rendered(&out));
        assert!(
            root.join("alpha").join("knowledge/b.md").is_file(),
            "the write landed outside the base CP1 checked and ratcheted"
        );
        assert!(
            !root.join("  alpha  ").exists(),
            "a look-alike base directory was created beside the one CP1 checked"
        );
    }

    #[tokio::test]
    async fn a_kb_less_search_still_serves_the_public_bases_it_can_see() {
        // The fan-out shape: a single up-front refusal turns
        // `search_visible_bases` into all-or-nothing, so one private base in the
        // session's set costs the user every other base.
        let (srv, _tmp, root) = migrated_server_with_bases(&["default", "omop"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        seed_page(&root, "default", "knowledge/a.md", "publichit zebracohort");
        seed_page(&root, "omop", "knowledge/b.md", "privatehit zebracohort");

        let hits = search_hits(&srv, serde_json::json!({ "query": "zebracohort" }), Public).await;
        let kb_ids: std::collections::BTreeSet<&str> =
            hits.iter().map(|h| h["kb_id"].as_str().unwrap()).collect();
        assert_eq!(
            kb_ids.into_iter().collect::<Vec<_>>(),
            vec!["default"],
            "got {hits:?}"
        );
        let rendered = serde_json::to_string(&hits).unwrap();
        assert!(!rendered.contains("privatehit"), "{rendered}");
        assert!(rendered.contains("publichit"), "{rendered}");

        // A private caller still spans both.
        let hits = search_hits(&srv, serde_json::json!({ "query": "zebracohort" }), Private).await;
        let kb_ids: std::collections::BTreeSet<&str> =
            hits.iter().map(|h| h["kb_id"].as_str().unwrap()).collect();
        assert_eq!(
            kb_ids.into_iter().collect::<Vec<_>>(),
            vec!["default", "omop"]
        );
    }

    #[tokio::test]
    async fn kb_list_bases_omits_a_private_base_rather_than_redacting_it() {
        // A KB name is user-authored and routinely names a cohort or a study.
        // Omission also removes the temptation to then pass the id explicitly,
        // which is the very bypass this task closes.
        let (srv, _tmp, root) = migrated_server_with_bases(&["default", "omop"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        assert_eq!(base_ids(&srv, Public).await, vec!["default".to_string()]);
        assert_eq!(
            base_ids(&srv, Private).await,
            vec!["default".to_string(), "omop".to_string()]
        );
    }

    #[tokio::test]
    async fn the_no_primary_error_names_only_the_bases_the_caller_may_reach() {
        // The fall-through `gated_kb_id` deliberately leaves open: with no
        // explicit kb_id and no primary, the TOOL answers — and its answer used
        // to be the full id list. Same leak class as `kb_list_bases` redacting
        // instead of omitting, one function over, and it hands the public caller
        // the exact argument the explicit-`kb_id` branch needs.
        let (srv, _tmp, root) = migrated_server_with_bases(&["default", "omop"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        clear_primary(&root, "sess-1");

        let public = call_tool_as_session(
            &srv,
            "kb_read_page",
            serde_json::json!({ "path": "knowledge/x.md" }),
            Some("sess-1"),
            Public,
        )
        .await;
        let t = rendered(&public);
        assert!(
            t.contains("default"),
            "the public base must still be offered: {t}"
        );
        assert!(
            !t.contains("omop"),
            "the no-primary error enumerated a private base: {t}"
        );

        let private = call_tool_as_session(
            &srv,
            "kb_read_page",
            serde_json::json!({ "path": "knowledge/x.md" }),
            Some("sess-1"),
            Private,
        )
        .await;
        assert!(
            rendered(&private).contains("omop"),
            "a private caller lost its own base: {}",
            rendered(&private)
        );
    }

    #[tokio::test]
    async fn a_public_session_whose_only_base_is_private_is_told_it_has_none() {
        // The degrade direction. Filtering the list must not leave
        // "Pass kb_id explicitly (one of: )" — an empty parenthesis is both
        // useless and a tell. It falls through to the branch that already exists
        // for a session with no bases at all.
        let (srv, _tmp, root) = migrated_server_with_bases(&["omop"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        clear_primary(&root, "sess-1");
        let t = rendered(
            &call_tool_as_session(
                &srv,
                "kb_list_pages",
                serde_json::json!({}),
                Some("sess-1"),
                Public,
            )
            .await,
        );
        assert!(t.contains("no knowledge bases"), "{t}");
        assert!(!t.contains("one of:"), "left an empty enumeration: {t}");
    }

    #[tokio::test]
    async fn kb_get_active_does_not_enumerate_a_private_base_or_point_at_one() {
        // The tool that takes NO arguments and returned the whole selection.
        // `selection_value` serialises `knowledge_bases`, `primary_kb` and the
        // deprecated `active_kb` mirror — all three are asserted, because
        // filtering two of the three is the natural half-fix and `active_kb` is
        // the one a reader forgets.
        let (srv, _tmp, root) = migrated_server_with_bases(&["default", "omop"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        set_primary(&root, "sess-1", "omop");

        let v = call_tool_json_as_session(
            &srv,
            "kb_get_active",
            serde_json::json!({}),
            "sess-1",
            Public,
        )
        .await;
        assert_eq!(v["knowledge_bases"], serde_json::json!(["default"]));
        // The pointer is metadata too. It reads null rather than naming a base
        // this caller may not reach — the truthful answer for THIS caller, which
        // has no usable write target, and the same omission rule kb_list_bases
        // takes.
        assert_eq!(v["primary_kb"], serde_json::Value::Null);
        assert_eq!(
            v["active_kb"],
            serde_json::Value::Null,
            "the deprecated mirror leaked it"
        );

        let v = call_tool_json_as_session(
            &srv,
            "kb_get_active",
            serde_json::json!({}),
            "sess-1",
            Private,
        )
        .await;
        assert_eq!(v["knowledge_bases"], serde_json::json!(["default", "omop"]));
        assert_eq!(v["primary_kb"], serde_json::json!("omop"));

        // And the STORE was not touched by the public read: the session's
        // primary file still names omop. This is the assertion that fails the
        // "filter it in `service::selection`" implementation, which looks
        // identical from the tool's output and silently re-points the user's
        // primary.
        assert_eq!(stored_primary(&root, "sess-1"), Some("omop".to_string()));
    }

    #[tokio::test]
    async fn a_private_target_and_a_nonexistent_one_are_indistinguishable_to_kb_set_active() {
        // Two halves. (1) A public caller may not move the pointer onto a private
        // base. (2) The refusal must be BYTE-IDENTICAL to the answer a base that
        // does not exist gets — a message saying "that base is private" confirms
        // it exists, in a politer sentence.
        let (srv, _tmp, root) = migrated_server_with_bases(&["default", "omop"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();

        let private_target = err_of(
            call_tool_as_session(
                &srv,
                "kb_set_active",
                serde_json::json!({ "kb_id": "omop" }),
                Some("sess-1"),
                Public,
            )
            .await,
        );
        let absent_target = err_of(
            call_tool_as_session(
                &srv,
                "kb_set_active",
                serde_json::json!({ "kb_id": "no-such-kb" }),
                Some("sess-1"),
                Public,
            )
            .await,
        );
        assert_eq!(
            private_target.replace("omop", "no-such-kb"),
            absent_target,
            "the two answers differ, so the difference is the oracle"
        );
        assert!(
            !private_target.to_lowercase().contains("private"),
            "{private_target}"
        );
        // The candidate list in the refusal is filtered, for the same reason
        // `the_no_primary_error_names_only_the_bases_the_caller_may_reach` exists.
        assert!(
            private_target.contains("default") && !private_target.contains("omop, "),
            "the refusal enumerated the set it refused: {private_target}"
        );
        assert_eq!(
            stored_primary(&root, "sess-1"),
            None,
            "the refused set was written anyway"
        );

        // A private caller still moves it.
        call_tool_as_session(
            &srv,
            "kb_set_active",
            serde_json::json!({ "kb_id": "omop" }),
            Some("sess-1"),
            Private,
        )
        .await
        .unwrap();
        assert_eq!(stored_primary(&root, "sess-1"), Some("omop".to_string()));
    }

    /// A tool that is not in `KB_ID_GATED_TOOLS`, why, and **the test that pins
    /// the behaviour that exemption claims**. The third field is the whole point:
    /// a bare string is a claim with nothing behind it — which is how
    /// `kb_get_active`, a no-argument tool that returned every visible base id,
    /// could sit on such a list being described as "the caller already knows the
    /// pointer".
    struct ExemptTool {
        name: &'static str,
        why: &'static str,
        pinned_by: &'static str,
    }

    const EXEMPT: &[ExemptTool] = &[
        ExemptTool {
            name: "kb_list_bases",
            why: "omits rather than refuses; a single-base refusal would hide every base",
            pinned_by: "kb_list_bases_omits_a_private_base_rather_than_redacting_it",
        },
        ExemptTool {
            name: "kb_get_active",
            why: "reports the selection; filters the VIEW, ids omitted, pointer null",
            pinned_by: "kb_get_active_does_not_enumerate_a_private_base_or_point_at_one",
        },
        ExemptTool {
            name: "kb_set_active",
            why: "moves the pointer; a private target is NOT A MEMBER, not 'private'",
            pinned_by:
                "a_private_target_and_a_nonexistent_one_are_indistinguishable_to_kb_set_active",
        },
        ExemptTool {
            name: "kb_create_base",
            why: "names a base that does not exist yet, so nothing to leak (Task 10A (3))",
            pinned_by: "a_public_chat_can_still_create_and_import_a_knowledge_base",
        },
        ExemptTool {
            name: "kb_import",
            why: "same; the id is chosen by brkb::import's collision loop",
            pinned_by: "a_public_chat_can_still_create_and_import_a_knowledge_base",
        },
    ];

    #[test]
    fn every_kb_tool_is_gated_or_exempt_for_a_pinned_reason() {
        // The partition: the router's own tool list must equal the gated list
        // plus the exemptions, nothing unaccounted for in either direction, so a
        // TWENTY-SECOND tool is a test failure rather than a silent hole.
        let mut known: Vec<&str> = KB_ID_GATED_TOOLS
            .iter()
            .copied()
            .chain(EXEMPT.iter().map(|e| e.name))
            .collect();
        known.sort();
        let mut actual: Vec<String> = KnowledgeServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        actual.sort();
        assert_eq!(
            actual, known,
            "a kb_* tool is neither gated nor listed as exempt"
        );
        // …and no exemption may be a bare assertion. Step 5 greps every
        // `pinned_by` for a real `fn` in this file; here we only pin that the
        // field is filled, because Rust has no way to name a test function from
        // another test.
        for e in EXEMPT {
            assert!(
                !e.why.is_empty() && e.pinned_by.starts_with(|c: char| c.is_alphabetic()),
                "{} is exempt with no reason and no pinning test",
                e.name
            );
        }
    }

    #[tokio::test]
    async fn no_exempt_tool_volunteers_a_private_bases_id_to_a_public_caller() {
        // The rule that REPLACES a blanket exemption, and the one that would have
        // caught `kb_get_active` before it shipped. `KB_ID_GATED_TOOLS` decides
        // who takes the CONTENT barrier and says nothing about METADATA; listing
        // the non-gated tools and stopping is not a completeness test, it is a
        // permission slip. This is universal over the exempt set, so a twenty-second
        // exempt tool is covered the day it is written.
        //
        // ⚠ Every probe's arguments name ONLY the public base. That is the
        // volunteering/being-asked line: echoing back an id the caller supplied
        // is not a leak (`kb_set_active {kb_id: "omop"}` must say so by name),
        // whereas producing that id from arguments that never mentioned it is
        // the content crossing.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let srv = server_with_root(root.clone());
        srv.service.create_base("default", "Default", None).unwrap();
        srv.service
            .create_base("omop-cohort-412", "OMOP Cohort", None)
            .unwrap();
        crate::knowledge::tier::raise_unlocked(&root, "omop-cohort-412", true).unwrap();
        set_primary(&root, "sess-1", "omop-cohort-412"); // the pointer names it

        let (_fx, brkb_path) = brkb_fixture(Public);
        let args_naming_only_default = |name: &str| -> serde_json::Value {
            match name {
                "kb_set_active" => serde_json::json!({ "kb_id": "default" }),
                "kb_create_base" => serde_json::json!({ "id": "fresh-kb", "name": "Fresh" }),
                "kb_import" => serde_json::json!({ "src_path": brkb_path }),
                // `kb_list_bases` and `kb_get_active` take no arguments at all —
                // which is exactly why they are the dangerous ones.
                _ => serde_json::json!({}),
            }
        };

        for e in EXEMPT {
            let out = call_tool_as_session(
                &srv,
                e.name,
                args_naming_only_default(e.name),
                Some("sess-1"),
                Public,
            )
            .await;
            let text = rendered(&out);
            assert!(
                !text.contains("omop-cohort-412"),
                "{} volunteered a private base id to a public caller: {text}",
                e.name
            );
            assert!(
                !text.contains("OMOP Cohort"), // the NAME, which is user-authored
                "{} volunteered a private base name: {text}",
                e.name
            );
        }

        // The same loop as a PRIVATE caller must still see it, or "no leak" is
        // satisfied by "the tools return nothing".
        let out = call_tool_as_session(
            &srv,
            "kb_get_active",
            serde_json::json!({}),
            Some("sess-1"),
            Private,
        )
        .await;
        assert!(rendered(&out).contains("omop-cohort-412"));
    }

    // ── Audit finding 17: one capability, ONE question ───────────────────────

    /// Three bases the finding-17 tests tell apart: unclaimed and public,
    /// UCSF's, Stanford's. Both cohorts are private *and* claimed, and the
    /// public one is claimed by nobody — which is what separates the tier axis
    /// from the affiliation axis. Ids and names are study-shaped on purpose:
    /// "the name is the leak" is the finding.
    ///
    /// Classified through the production ratchets (`raise_unlocked` +
    /// `raise_affiliation_unlocked`), so the on-disk shape is the one a real
    /// chat would have written.
    fn cross_institution_fixture() -> (KnowledgeServer, tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let srv = server_with_root(root.clone());
        for (id, name) in [
            ("open-notes", "Open Notes"),
            ("ucsf-cohort-412", "UCSF Cohort 412"),
            ("stanford-cohort-77", "Stanford Cohort 77"),
        ] {
            srv.service.create_base(id, name, None).unwrap();
            crate::knowledge::store::write_page(
                &crate::knowledge::paths::kb_root(&root, id),
                "knowledge/x.md",
                "# Shared topic\n\nbody",
                "seed",
                None,
            )
            .unwrap();
        }
        for (id, owner) in [
            ("ucsf-cohort-412", "ucsf"),
            ("stanford-cohort-77", "stanford"),
        ] {
            crate::knowledge::tier::raise_unlocked(&root, id, true).unwrap();
            crate::knowledge::tier::raise_affiliation_unlocked(
                &root,
                id,
                &CallerAffiliation::Institution(owner.to_string()),
            )
            .unwrap();
        }
        (srv, tmp, root)
    }

    const FIXTURE_BASES: [&str; 3] = ["open-notes", "ucsf-cohort-412", "stanford-cohort-77"];

    /// One caller identity to drive the seam with, both axes stated.
    struct Identity {
        label: &'static str,
        tier: Caller,
        affiliation: CallerAffiliation,
    }

    fn identities() -> Vec<Identity> {
        vec![
            Identity {
                label: "a public model",
                tier: Public,
                affiliation: CallerAffiliation::Unstated,
            },
            Identity {
                label: "a private model that states no affiliation",
                tier: Private,
                affiliation: CallerAffiliation::Unstated,
            },
            Identity {
                label: "a private model covered by UCSF",
                tier: Private,
                affiliation: CallerAffiliation::Institution("ucsf".to_string()),
            },
            Identity {
                label: "a private model covered by Stanford",
                tier: Private,
                affiliation: CallerAffiliation::Institution("stanford".to_string()),
            },
            Identity {
                label: "a local model",
                tier: Private,
                affiliation: CallerAffiliation::Local,
            },
        ]
    }

    /// Does this caller actually get `kb_id`'s CONTENT? Asked of the real
    /// barrier through the real `call_tool` seam, never recomputed here — a test
    /// that re-derives the expected answer is testing its own copy of the rule.
    async fn serves_content(srv: &KnowledgeServer, id: &Identity, kb_id: &str) -> bool {
        call_tool_as_full(
            srv,
            "kb_search",
            serde_json::json!({ "kb_id": kb_id, "query": "shared topic" }),
            Some("sess-1"),
            id.tier,
            &id.affiliation,
        )
        .await
        .is_ok()
    }

    /// The finding, stated as the user experiences it: in a chat bound to
    /// another institution's model the app listed the base's NAME and then
    /// refused its content — half-knowing something.
    ///
    /// The two guards over one capability disagreed about how many axes to ask.
    /// `assert_kb_reachable` asked tier **and** affiliation; `kb_is_out_of_reach`
    /// — the predicate behind every listing filter — asked only the tier, so for
    /// any private caller it answered "reachable" about every base on the
    /// machine. A UCSF chat was therefore handed `stanford-cohort-77`, which is
    /// both a disclosure in itself (a KB name routinely names a cohort or a
    /// study) and the one argument the explicit-`kb_id` branch needs.
    #[tokio::test]
    async fn a_cross_institution_chat_is_not_shown_the_names_of_bases_it_will_be_refused() {
        let (srv, _tmp, _root) = cross_institution_fixture();
        let ucsf = Identity {
            label: "a private model covered by UCSF",
            tier: Private,
            affiliation: CallerAffiliation::Institution("ucsf".to_string()),
        };

        let listing = rendered(
            &call_tool_as_full(
                &srv,
                "kb_list_bases",
                serde_json::json!({}),
                Some("sess-1"),
                ucsf.tier,
                &ucsf.affiliation,
            )
            .await,
        );
        assert!(
            listing.contains("open-notes") && listing.contains("ucsf-cohort-412"),
            "the UCSF chat lost the bases it may actually read: {listing}"
        );
        assert!(
            !listing.contains("stanford-cohort-77") && !listing.contains("Stanford Cohort 77"),
            "kb_list_bases named a base whose content the barrier refuses: {listing}"
        );

        // …and the refusal it was protecting the caller from is real, so the
        // omission above is not the listing simply being over-tight.
        let refusal = err_of(
            call_tool_as_full(
                &srv,
                "kb_search",
                serde_json::json!({ "kb_id": "stanford-cohort-77", "query": "shared topic" }),
                Some("sess-1"),
                ucsf.tier,
                &ucsf.affiliation,
            )
            .await,
        );
        assert!(
            refusal.contains("Cross-institutional"),
            "expected the affiliation barrier, got: {refusal}"
        );
    }

    /// The completeness half. Every KB entry point that *omits* a base must omit
    /// exactly the bases the barrier *refuses* — for every caller identity and
    /// every base, with both sides computed by production code.
    ///
    /// ⚠ This is the test that makes finding 17 non-recurring, and it is written
    /// as an invariant rather than a table of today's answers: `listed` and
    /// `served` are both read out of real `call_tool` responses, so a future
    /// axis added to the barrier and forgotten in a filter fails here without
    /// anyone editing it. It also proves the predicate is WIRED — all four
    /// listing surfaces below reach `kb_is_out_of_reach` through production
    /// paths, which a unit test of the predicate alone would not show.
    ///
    /// The five surfaces are the complete set of places this file omits a base:
    /// 1. `kb_list_bases`      → `visible_bases_for_session`
    /// 2. `kb_get_active`      → `visible_kb_ids` (all three of its fields)
    /// 3. the no-primary error → `kb_id_or_primary`'s candidate list
    /// 4. a KB-less `kb_search` fan-out → `search_visible_bases`
    /// 5. `kb_set_active`'s "not one of your bases" → `visible_kb_ids` again,
    ///    through `not_a_member`
    #[tokio::test]
    async fn every_listing_surface_omits_exactly_what_the_barrier_refuses() {
        for id in identities() {
            let (srv, _tmp, root) = cross_institution_fixture();
            clear_primary(&root, "sess-1");

            let mut served = Vec::new();
            for kb in FIXTURE_BASES {
                if serves_content(&srv, &id, kb).await {
                    served.push(kb);
                }
            }
            assert!(
                !served.is_empty(),
                "{}: no base is readable at all, so every omission assertion below is vacuous",
                id.label
            );

            // 1 + 2 + 3: three text surfaces, each asserted for every base.
            let listing = rendered(
                &call_tool_as_full(
                    &srv,
                    "kb_list_bases",
                    serde_json::json!({}),
                    Some("sess-1"),
                    id.tier,
                    &id.affiliation,
                )
                .await,
            );
            let selection = rendered(
                &call_tool_as_full(
                    &srv,
                    "kb_get_active",
                    serde_json::json!({}),
                    Some("sess-1"),
                    id.tier,
                    &id.affiliation,
                )
                .await,
            );
            // No kb_id and no primary, so the tool answers with its candidate
            // list — the fall-through `gated_kb_id` deliberately leaves open.
            let candidates = rendered(
                &call_tool_as_full(
                    &srv,
                    "kb_list_pages",
                    serde_json::json!({}),
                    Some("sess-1"),
                    id.tier,
                    &id.affiliation,
                )
                .await,
            );
            // 4: the fan-out, which attributes every hit with its kb_id.
            let fanout = rendered(
                &call_tool_as_full(
                    &srv,
                    "kb_search",
                    serde_json::json!({ "query": "shared topic" }),
                    Some("sess-1"),
                    id.tier,
                    &id.affiliation,
                )
                .await,
            );
            // 5: `not_a_member`. The id asked for is one that cannot exist, so
            // nothing in the answer is an echo of what the caller supplied —
            // every fixture id in it was volunteered by the server.
            let not_a_member = rendered(
                &call_tool_as_full(
                    &srv,
                    "kb_set_active",
                    serde_json::json!({ "kb_id": "no-such-kb" }),
                    Some("sess-1"),
                    id.tier,
                    &id.affiliation,
                )
                .await,
            );

            for kb in FIXTURE_BASES {
                let served = served.contains(&kb);
                for (surface, text) in [
                    ("kb_list_bases", &listing),
                    ("kb_get_active", &selection),
                    ("the no-primary candidate list", &candidates),
                    ("a KB-less kb_search", &fanout),
                    ("kb_set_active's not-a-member answer", &not_a_member),
                ] {
                    assert_eq!(
                        text.contains(kb),
                        served,
                        "{}: {surface} {} '{kb}', but the barrier {} its content.\n{text}",
                        id.label,
                        if served { "omitted" } else { "named" },
                        if served { "serves" } else { "refuses" },
                    );
                }
            }
        }
    }

    /// Finding 17's structural half: there must be no SECOND spelling of the
    /// question inside this file.
    ///
    /// The behavioural test above catches a filter that asks too little today.
    /// This one catches the way that happens — someone writing
    /// `tier::is_private(..)` inline next to a new listing, which reads
    /// perfectly plausible and re-forks the two guards. The barrier
    /// (`tier::assert_reachable`) is the only thing this file may ask, and it may
    /// ask it only through `assert_kb_reachable`.
    #[test]
    fn this_file_asks_the_barrier_and_never_re_spells_it() {
        let src = include_str!("server.rs");
        let production = src
            .split("#[cfg(test)]")
            .next()
            .expect("server.rs has a production half above its tests");

        // Assembled at runtime so this test's own text cannot satisfy it.
        let re_spelling = concat!("tier::", "is_private");
        assert_eq!(
            production.matches(re_spelling).count(),
            1,
            "a listing filter re-spelled the reachability question instead of asking \
             assert_kb_reachable; that is exactly how finding 17 happened"
        );
        // The ONE permitted direct read, and it is not a reachability question:
        // `kb_export` decides *where the archive lands*, over a base CP1 has
        // already cleared. If it ever moves out of that function, the exemption
        // this assertion grants has to be re-argued.
        let export = production
            .split("pub async fn kb_export")
            .nth(1)
            .expect("kb_export still exists");
        assert!(
            export.contains(re_spelling),
            "the one permitted direct tier read is no longer inside kb_export"
        );

        // And the predicate really is the barrier, negated. A body that grew a
        // condition of its own would pass the grep above and still fork.
        let body = production
            .split("fn kb_is_out_of_reach")
            .nth(1)
            .expect("kb_is_out_of_reach still exists")
            .split("\n    }")
            .next()
            .expect("a closing brace");
        assert!(
            body.contains("self.assert_kb_reachable(kb_id, caller).is_err()")
                && !body.contains("&&")
                && !body.contains("||"),
            "kb_is_out_of_reach must be assert_kb_reachable negated, nothing more, got:{body}"
        );

        // The predicate must have live production callers — nine guards in this
        // campaign shipped correct, tested and called by nothing.
        let call_sites = production.matches("self.kb_is_out_of_reach(").count();
        assert!(
            call_sites >= 4,
            "expected the four listing surfaces to reach the predicate, found {call_sites}"
        );
    }

    // -----------------------------------------------------------------------
    // DR-33, design §2.6: a result read out of a private base is MARKED, so the
    // daemon's dispatch choke point ratchets the chat that receives it.
    // -----------------------------------------------------------------------

    fn marked_bases(out: &Result<CallToolResult, ErrorData>) -> Option<Vec<String>> {
        let result = out.as_ref().expect("the call answers");
        crate::knowledge::tier::private_content_bases(result.meta.as_ref())
    }

    fn probe_args(tool: &str, kb: &str) -> serde_json::Value {
        (KB_TOOL_PROBES
            .iter()
            .find(|probe| probe.name == tool)
            .unwrap_or_else(|| panic!("{tool} has a probe"))
            .args)(kb)
    }

    #[tokio::test]
    async fn a_private_callers_read_of_a_private_base_is_marked_with_the_base() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["omop", "notes"]);
        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();

        let out = call_tool_as(
            &srv,
            "kb_read_page",
            probe_args("kb_read_page", "omop"),
            Private,
        )
        .await;
        assert_eq!(marked_bases(&out), Some(vec!["omop".to_string()]));

        // A public base, read by the same caller, carries nothing private.
        let out = call_tool_as(
            &srv,
            "kb_read_page",
            probe_args("kb_read_page", "notes"),
            Private,
        )
        .await;
        assert_eq!(marked_bases(&out), None);
        // A public caller is never given private content, so is never marked.
        let out = call_tool_as(
            &srv,
            "kb_read_page",
            probe_args("kb_read_page", "notes"),
            Public,
        )
        .await;
        assert_eq!(marked_bases(&out), None);
    }

    /// The tier is read BEFORE a write's raise: a private chat writing into a
    /// public base raises the base, but was given nothing private by the call.
    #[tokio::test]
    async fn a_private_write_into_a_public_base_is_not_marked() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["notes"]);
        let out = call_tool_as(
            &srv,
            "kb_append_log",
            probe_args("kb_append_log", "notes"),
            Private,
        )
        .await;
        assert!(
            crate::knowledge::tier::is_private(&root, "notes"),
            "precondition: the write raised the base"
        );
        assert_eq!(marked_bases(&out), None);
    }

    #[tokio::test]
    async fn a_search_across_the_visible_set_is_marked_when_a_private_base_is_in_it() {
        let (srv, _tmp, root) = migrated_server_with_bases(&["omop", "notes"]);
        let search = serde_json::json!({ "query": "anything" });

        let out = call_tool_as(&srv, "kb_search", search.clone(), Private).await;
        assert_eq!(marked_bases(&out), None, "every base is public");

        crate::knowledge::tier::raise_unlocked(&root, "omop", true).unwrap();
        let out = call_tool_as(&srv, "kb_search", search.clone(), Private).await;
        assert_eq!(marked_bases(&out), Some(vec!["omop".to_string()]));
        let out = call_tool_as(&srv, "kb_list_bases", serde_json::json!({}), Private).await;
        assert_eq!(marked_bases(&out), Some(vec!["omop".to_string()]));

        // A public caller's search skips the private base, so is not marked.
        let out = call_tool_as(&srv, "kb_search", search, Public).await;
        assert_eq!(marked_bases(&out), None);
    }

    /// Every tool is placed by the mark: it names one base (and is marked when
    /// that base is private), reads across the visible set, or reads no
    /// existing base. A new tool in none of the three would hand a private
    /// base's content to a chat without the chat ever being told.
    #[test]
    fn every_tool_is_classified_for_the_private_content_mark() {
        // Tools that read no existing base: creating one, and importing an
        // archive the caller supplies. Their results are never marked.
        const KB_NO_EXISTING_BASE_TOOLS: &[&str] = &["kb_create_base", "kb_import"];
        let exposed: std::collections::BTreeSet<String> = KnowledgeServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        let mut placed = std::collections::BTreeSet::new();
        let mut twice = std::collections::BTreeSet::new();
        for name in KB_ID_GATED_TOOLS
            .iter()
            .chain(KB_VISIBLE_SET_TOOLS)
            .chain(KB_NO_EXISTING_BASE_TOOLS)
        {
            if !placed.insert(name.to_string()) {
                twice.insert(name.to_string());
            }
        }
        assert_eq!(exposed, placed);
        // The two searches name one base when given a `kb_id` and fan out over
        // the visible set when not; nothing else may be in two lists.
        assert_eq!(
            twice,
            std::collections::BTreeSet::from([
                "kb_search".to_string(),
                "kb_search_raw_sources".to_string()
            ])
        );
    }
}

#[cfg(test)]
mod transaction_slot_bound_tests {
    use super::*;

    /// A caller waiting on a busy base gives up and says so.
    ///
    /// Found by DRIVING the app, not by reading it: a `kb_write_page` behind an
    /// open transaction sat for twenty-one minutes — no error, no progress, and
    /// nothing in the daemon log after the dispatch line. The tool had not
    /// failed; it was queued on a slot with neither a deadline nor a cancel,
    /// inheriting the 1800s KB-lock wait it is held across.
    ///
    /// The assertion is a DURATION and a MESSAGE. Asserting only that it
    /// eventually errors would pass against the unbounded version too — after
    /// half an hour — which is precisely the bug.
    #[tokio::test]
    async fn a_busy_base_refuses_instead_of_parking_for_the_write_deadline() {
        let slot: ActiveKnowledgeTransactionSlot = Arc::new(Mutex::new(None));
        let held = Arc::clone(&slot).lock_owned().await;

        let wait = std::time::Duration::from_millis(120);
        let started = std::time::Instant::now();
        let outcome = lock_transaction_slot_waiting(&slot, "busy-base", None, wait).await;
        let waited = started.elapsed();
        drop(held);

        let error = outcome.err().expect("a busy slot must refuse, not hang");
        assert!(
            error.message.contains("busy with another transaction"),
            "the refusal must name the cause and the remedy: {}",
            error.message
        );
        assert!(
            waited >= wait,
            "it must actually wait the deadline it was given"
        );
        // The shipped deadline is the read one, and the point of the bound is
        // that it is far shorter than the 1800s KB write wait it is held across.
        assert!(
            SLOT_WAIT < std::time::Duration::from_secs(1800),
            "SLOT_WAIT must stay well under the write deadline it exists to escape"
        );
    }

    /// Cancelling actually cancels. `Stop` in the GUI cancels the request token,
    /// and a caller parked here used to ignore it — the turn looked cancelled
    /// while the task went on waiting.
    #[tokio::test]
    async fn cancelling_a_waiter_returns_immediately() {
        let slot: ActiveKnowledgeTransactionSlot = Arc::new(Mutex::new(None));
        let held = Arc::clone(&slot).lock_owned().await;
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();

        let started = std::time::Instant::now();
        let error = lock_transaction_slot_waiting(
            &slot,
            "busy-base",
            Some(&token),
            std::time::Duration::from_secs(30),
        )
        .await
        .err()
        .expect("a cancelled waiter must not acquire");
        drop(held);

        assert!(error.message.contains("cancelled"), "{}", error.message);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "cancellation must return at once, not wait out the deadline"
        );
    }
}
