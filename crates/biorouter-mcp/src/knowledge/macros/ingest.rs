//! `ingest` macro — wraps the sub-agent loop to integrate a new source into
//! a KB with a single, txn-atomic git commit.

use crate::knowledge::{
    biookf,
    convert::SourceInput,
    git::{GitRepo, Txn},
    paths,
    service::{KnowledgeService, RawSourceRefreshFailure},
    subagent::{
        events::{DoneReason, SubAgentEvent},
        kb_tools::{tool_specs, KbToolAccess, KbToolDispatch},
        loop_::{Completer, SubAgent, SubAgentBounds, SubAgentResult},
        procedures::{ingest_curation_procedure, system_prompt, IngestCurationProfile},
    },
    types::{ChangeKind, KbFormat, SourceMeta},
    validate::{Diagnostics, Severity},
};
use anyhow::{Context, Result};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub struct IngestArgs {
    pub kb_id: String,
    /// The capability of the model this macro will run (issue #56). Required,
    /// so all four production callers are a compile error rather than an
    /// omission. A `bool` and not `ProviderTier` because `biorouter-mcp` cannot
    /// depend on `biorouter`, where that enum lives.
    pub caller_is_private: bool,
    /// Whose agreements cover that model — DR-26's third axis (issue #56, Task
    /// 50). Required for the same reason `caller_is_private` is: an omission
    /// must be a compile error. `Unstated` is its `Default`, so a caller that
    /// cannot determine one fails closed.
    pub caller_affiliation: crate::knowledge::affiliation::CallerAffiliation,
    pub source: SourceInput,
    pub completer: Box<dyn Completer>,
    pub focus: Option<String>,
    pub bounds: SubAgentBounds,
    /// Optional channel for live event streaming. When provided, every
    /// `SubAgentEvent` is sent here as soon as it is produced (not just at
    /// the end of the run). Set to `None` if streaming is not needed.
    pub event_sink: Option<tokio::sync::mpsc::UnboundedSender<SubAgentEvent>>,
    /// Optional level-triggered cancellation shared with the request or turn
    /// that owns this macro run.
    pub cancel: Option<tokio_util::sync::CancellationToken>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IngestResult {
    pub source_id: String,
    /// The commit that retained the raw source before curation began. `None`
    /// means the source was already present and deduplication made no commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_commit_sha: Option<String>,
    /// The transaction commit containing the curated knowledge pages.
    pub commit_sha: String,
    pub steps: usize,
    pub events: Vec<SubAgentEvent>,
    /// What the committed pages actually look like — see [`Verification`].
    ///
    /// `serde(default)` so an `IngestResult` written before this field existed
    /// still deserializes; its default is "the check did not run", not "clean".
    #[serde(default)]
    pub verification: Verification,
}

/// A curation failure that happened after the raw source became durable.
///
/// Keeping this typed lets callers report the retained source and its commit
/// instead of turning a failed curation into the false claim that the base did
/// not change.
#[derive(Debug)]
pub struct IngestCurationFailure {
    pub source_id: String,
    pub raw_commit_sha: Option<String>,
    pub curation_commit_sha: Option<String>,
    pub phase: IngestFailurePhase,
    cause: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestFailurePhase {
    NotStarted,
    RolledBack,
    OutcomeUncertain,
    Committed,
}

impl std::fmt::Display for IngestCurationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cause = self.cause.trim_end_matches('.');
        write!(f, "{cause}. ")?;
        match self.phase {
            IngestFailurePhase::NotStarted => f.write_str("Curation did not start")?,
            IngestFailurePhase::RolledBack => f.write_str("Curation rolled back")?,
            IngestFailurePhase::OutcomeUncertain => f.write_str(
                "Curation did not complete and its commit or rollback could not be verified",
            )?,
            IngestFailurePhase::Committed => {
                f.write_str("Curation committed")?;
                if let Some(sha) = &self.curation_commit_sha {
                    write!(f, " in commit {sha}")?;
                }
                f.write_str(", but post-commit refresh failed")?;
            }
        }
        write!(f, "; raw source {} was retained", self.source_id)?;
        if let Some(sha) = &self.raw_commit_sha {
            write!(f, " in commit {sha}")?;
        }
        if matches!(
            self.phase,
            IngestFailurePhase::OutcomeUncertain | IngestFailurePhase::Committed
        ) {
            f.write_str(". Refresh the knowledge base and history before retrying")?;
        }
        f.write_str(".")
    }
}

impl std::error::Error for IngestCurationFailure {}

fn retained_raw_failure(
    source_id: &str,
    raw_commit_sha: Option<&str>,
    phase: IngestFailurePhase,
    curation_commit_sha: Option<&str>,
    error: anyhow::Error,
) -> anyhow::Error {
    IngestCurationFailure {
        source_id: source_id.to_string(),
        raw_commit_sha: raw_commit_sha.map(str::to_string),
        curation_commit_sha: curation_commit_sha.map(str::to_string),
        phase,
        cause: format!("{error:#}"),
    }
    .into()
}

fn aborted_curation_failure(
    repo: &GitRepo,
    txn: &Txn,
    source_id: &str,
    raw_commit_sha: Option<&str>,
    error: anyhow::Error,
) -> anyhow::Error {
    match repo.abort_txn(txn) {
        Ok(()) => retained_raw_failure(
            source_id,
            raw_commit_sha,
            IngestFailurePhase::RolledBack,
            None,
            error,
        ),
        Err(abort_error) => retained_raw_failure(
            source_id,
            raw_commit_sha,
            IngestFailurePhase::OutcomeUncertain,
            None,
            anyhow::anyhow!("{error:#}; rollback also failed: {abort_error:#}"),
        ),
    }
}

/// The tail check: what the run left behind, measured after it committed.
///
/// **The gap this closes.** The ingest macro is the highest-volume writer in the
/// system and the one loop Biorouter fully controls, and until now a run could
/// write fifteen non-conformant pages, commit them and report success. The
/// deterministic scan that would have said so already existed
/// (`macros::lint::scan`) and nobody called it at the one moment it is most
/// useful.
///
/// **It reports; it does not block** (DR-7). A base with warnings — an orphan
/// page, an edge whose target does not exist yet — is a normal base mid-curation,
/// and an ingest that failed on one would be an ingest nobody runs twice. The
/// result travels in the macro's return value and therefore in the SSE stream's
/// terminal `done` frame, where the user sees it.
///
/// ⚠ **In-process and typed, deliberately — not a hook and not a subprocess.**
/// BioOKF's equivalent is a shell stop-hook (`app/hooks/stop-verify.sh`) that
/// runs `bokf verify … --json` and, on the line
/// `res=$(… ) || { rm -f "$counter"; exit 0; }`, treats a **non-zero exit** as
/// "allow the stop". A non-zero exit is exactly what a dirty bundle produces, so
/// the gate fails open on the one outcome it exists to catch, and it does that
/// because a subprocess can only speak in exit codes and "the checker broke" and
/// "the bundle is dirty" share one. A typed value returned from the same process
/// has no code to conflate: `ok: false` means dirty, and a check that could not
/// run says so in [`Self::scan_error`] rather than resolving to clean.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Verification {
    /// True only when the scan RAN and found no errors. A scan that could not
    /// run is `false` with a [`Self::scan_error`], never a quiet `true` —
    /// "unverified" and "verified clean" are different answers and only one of
    /// them lets the user stop looking.
    pub ok: bool,
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
    /// The findings themselves: `items`, capped and most severe first, plus
    /// `total`, the count before the cap.
    pub diagnostics: Diagnostics,
    /// Why there is nothing to report, when the scan itself failed. Absent on
    /// every normal run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scan_error: Option<String>,
}

/// Re-scan the base the run just committed.
///
/// After the commit rather than before it: the question is what the ingest
/// *left behind*, and a scan of the transaction branch would describe a tree the
/// user never sees. It is the same `scan` the `lint` macro and the `kb_lint`
/// tool call, so there is no second implementation to drift.
fn verify(
    kb_root: &std::path::Path,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Verification {
    match super::lint::scan_with_cancellation(kb_root, cancel) {
        Ok(report) => {
            let d = report.diagnostics;
            Verification {
                ok: d.errors() == 0,
                errors: d.errors(),
                warnings: d.count(Severity::Warning),
                info: d.count(Severity::Info),
                diagnostics: d,
                scan_error: None,
            }
        }
        // The ingest itself succeeded and is committed; a scan that could not
        // read the tree back is worth saying out loud and is not worth undoing
        // real work over.
        Err(e) => Verification {
            scan_error: Some(format!("{e:#}")),
            ..Verification::default()
        },
    }
}

pub async fn ingest(svc: &KnowledgeService, args: IngestArgs) -> Result<IngestResult> {
    ingest_with_curation_profile(svc, args, None).await
}

pub async fn ingest_with_curation_profile(
    svc: &KnowledgeService,
    args: IngestArgs,
    curation_profile: Option<IngestCurationProfile>,
) -> Result<IngestResult> {
    let cancel = args.cancel.clone();
    let _lock = svc
        .lock_kb_cancellable(&args.kb_id, cancel.as_ref())
        .await?;
    let (kb_root, format) = prepare_ingest_base(svc, &args, cancel.as_ref()).await?;
    let raw = stage_raw_source(svc, &args.kb_id, args.source, cancel.as_ref()).await?;
    if super::ensure_not_cancelled(cancel.as_ref(), "the ingest transaction").is_err() {
        return Err(retained_raw_failure(
            &raw.source_id,
            raw.commit_sha.as_deref(),
            IngestFailurePhase::NotStarted,
            None,
            anyhow::anyhow!("ingest cancelled before curation started"),
        ));
    }
    let (repo, txn) = begin_ingest_transaction(&kb_root, &raw)?;

    // ⚠ Everything from here to the sub-agent runs INSIDE the transaction, so a
    // failure must abort it. `begin_txn` moves HEAD onto the txn branch, and an
    // early `?` would leave it parked there — which is how the next write to
    // this KB lands somewhere nobody is looking, the same hazard the failure
    // arms below already guard. The `schema.md` read predates this stage and had
    // exactly that bug; the two DR-24 steps join it rather than each growing
    // their own `match`.
    let setup = ingest_setup(svc, &kb_root, &repo, &txn, Some(format), &raw.source_id);
    let (source_node, baseline_knowledge, schema) = match setup {
        Ok(setup) => setup,
        Err(e) => {
            return Err(aborted_curation_failure(
                &repo,
                &txn,
                &raw.source_id,
                raw.commit_sha.as_deref(),
                e,
            ));
        }
    };
    let procedure = ingest_curation_procedure(Some(format), curation_profile.as_ref());
    let system = system_prompt(&schema, procedure.as_ref());

    let dispatch = KbToolDispatch {
        svc: svc.clone(),
        kb_id: args.kb_id.clone(),
        txn_branch: txn.branch.clone(),
        access: KbToolAccess::ReadWrite,
    };
    let agent = SubAgent {
        completer: args.completer,
        tools: tool_specs(Some(format)),
        system_prompt: system,
        bounds: args.bounds,
    };
    let user = opening_message(&raw.source_id, args.focus.as_deref(), source_node.as_ref());

    let cancel_ref = cancel.as_ref();
    let agent_result = agent
        .run(
            &user,
            std::sync::Arc::new(dispatch),
            cancel_ref,
            args.event_sink.as_ref(),
        )
        .await;

    let run = IngestRun {
        kb_id: &args.kb_id,
        kb_root: &kb_root,
        source_id: &raw.source_id,
        raw_commit_sha: raw.commit_sha.as_deref(),
        baseline_knowledge,
    };
    settle_ingest(svc, &repo, &txn, run, agent_result, cancel.as_ref())
}

async fn prepare_ingest_base(
    svc: &KnowledgeService,
    args: &IngestArgs,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<(std::path::PathBuf, KbFormat)> {
    super::ensure_not_cancelled(cancel, "ingest preflight")?;
    // Issue #56. This barrier must precede the ratchet because the sub-agent's
    // KB tools reach the store directly and cannot enforce it at a lower seam.
    crate::knowledge::tier::assert_reachable(
        svc.root(),
        &args.kb_id,
        args.caller_is_private,
        &args.caller_affiliation,
    )?;
    let format = svc.require_current_profile(&args.kb_id)?;
    super::ensure_not_cancelled(cancel, "the ingest privacy ratchet")?;
    // Both privacy axes move together under the ingest lock.
    svc.raise_tier_and_affiliation_cancelled_by(
        &args.kb_id,
        args.caller_is_private,
        &args.caller_affiliation,
        cancel,
    )
    .await?;
    let kb_root = paths::kb_root(svc.root(), &args.kb_id);

    // Refresh before staging so a failed refresh cannot leave a newly durable
    // raw source behind without the retained-source error contract.
    super::ensure_not_cancelled(cancel, "the ingest refresh")?;
    super::refresh_base(svc, &args.kb_id).await?;
    super::ensure_not_cancelled(cancel, "raw source staging")?;
    Ok((kb_root, format))
}

async fn stage_raw_source(
    svc: &KnowledgeService,
    kb_id: &str,
    source: SourceInput,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<crate::knowledge::raw::RawWrite> {
    // Raw material is intentionally durable before the curation transaction.
    match svc
        .add_raw_source_cancelled_by(kb_id, source, None, cancel)
        .await
    {
        Ok(raw) => Ok(raw),
        Err(error) => {
            let retained = error
                .downcast_ref::<RawSourceRefreshFailure>()
                .map(|failure| failure.written.clone());
            // These errors reach the person as the digest's error frame, so the
            // words are theirs. The internal label this used to add,
            // "add_raw_source", was all an ingest route printed and hid the
            // cause, a corrupt PDF included (rq2-029).
            if let Some(written) = retained {
                // Read and saved; only the refresh after it failed, and the
                // failure's own sentence says so.
                return Err(retained_raw_failure(
                    &written.source_id,
                    written.commit_sha.as_deref(),
                    IngestFailurePhase::NotStarted,
                    None,
                    error,
                ));
            }
            // An unreadable source already says so, with its reason
            // (`convert::UnreadableSource`).
            if crate::knowledge::convert::UnreadableSource::is(&error) {
                return Err(error);
            }
            Err(error.context("Could not add this source to the knowledge base"))
        }
    }
}

fn begin_ingest_transaction(
    kb_root: &std::path::Path,
    raw: &crate::knowledge::raw::RawWrite,
) -> Result<(GitRepo, Txn)> {
    let repo = GitRepo::open(kb_root).map_err(|error| {
        retained_raw_failure(
            &raw.source_id,
            raw.commit_sha.as_deref(),
            IngestFailurePhase::NotStarted,
            None,
            error,
        )
    })?;
    let txn = repo
        .begin_txn(&format!("ingest-{}", raw.source_id))
        .map_err(|error| {
            retained_raw_failure(
                &raw.source_id,
                raw.commit_sha.as_deref(),
                IngestFailurePhase::NotStarted,
                None,
                error,
            )
        })?;
    Ok((repo, txn))
}

/// What the settle arm needs to know about the run it is closing, so
/// [`settle_ingest`] takes one value instead of five loose arguments.
struct IngestRun<'a> {
    kb_id: &'a str,
    kb_root: &'a std::path::Path,
    source_id: &'a str,
    raw_commit_sha: Option<&'a str>,
    /// The `knowledge/` tree as the sub-agent FOUND it — see [`ingest_setup`].
    baseline_knowledge: Option<String>,
}

/// The transaction's three endings — commit, abort-and-fail on a run that wrote
/// nothing, abort-and-fail on an aborted run — split out of [`ingest`] so that
/// function stays under `clippy::too_many_lines`, and mirroring
/// [`super::lint::settle_autofix`], which does the same job for the lint macro.
/// The only new behaviour is the tail [`verify`].
fn settle_ingest(
    svc: &KnowledgeService,
    repo: &GitRepo,
    txn: &Txn,
    run: IngestRun<'_>,
    agent_result: Result<SubAgentResult>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<IngestResult> {
    match agent_result {
        Ok(r)
            if matches!(
                r.reason,
                DoneReason::CompleteSentinel | DoneReason::NoMoreToolCalls
            ) =>
        {
            if let Err(error) = super::ensure_not_cancelled(cancel, "the ingest commit") {
                return Err(aborted_curation_failure(
                    repo,
                    txn,
                    run.source_id,
                    run.raw_commit_sha,
                    error,
                ));
            }
            // A clean exit is not the same as a digest. `INGEST_PROCEDURE`
            // requires the run to write `knowledge/sources/<source-id>.md`, so a
            // transaction that left `knowledge/` exactly as it found it produced
            // no knowledge — and committing it would hand the caller a commit sha
            // for work that never happened (issue #71). The most common cause is
            // a provider that failed mid-request: the turn comes back as a bare
            // apology, or (Google, candidate with no `parts`) as a wholly empty
            // message, and both look exactly like "the agent has no more tool
            // calls". It is `knowledge/` and not the whole tree because the
            // procedure's other steps — the `raw/` source, the `index.md` entry,
            // the `log.md` line — each move the tree without adding knowledge, so
            // a run cut short after one of them would otherwise pass.
            //
            // A failure to *answer* the question aborts too: leaving HEAD parked
            // on the txn branch is how the next write to this KB lands somewhere
            // nobody is looking.
            let wrote_knowledge = match repo.txn_knowledge_tree_id(txn) {
                Ok(after) => after != run.baseline_knowledge,
                Err(e) => {
                    return Err(aborted_curation_failure(
                        repo,
                        txn,
                        run.source_id,
                        run.raw_commit_sha,
                        e.context("checking whether the ingest wrote anything"),
                    ));
                }
            };
            if !wrote_knowledge {
                return Err(aborted_curation_failure(
                    repo,
                    txn,
                    run.source_id,
                    run.raw_commit_sha,
                    anyhow::anyhow!(no_pages_written_error(run.source_id, &r)),
                ));
            }
            let sha = match repo.commit_txn(
                txn,
                ChangeKind::Ingest,
                &format!("ingest {}", run.source_id),
                Some(&format!("+1 source · {} steps", r.steps_used)),
            ) {
                Ok(sha) => sha,
                Err(e) => {
                    return Err(retained_raw_failure(
                        run.source_id,
                        run.raw_commit_sha,
                        IngestFailurePhase::OutcomeUncertain,
                        None,
                        e.context("committing curated knowledge pages"),
                    ));
                }
            };
            if let Err(e) = svc.rebuild_graph_cache(run.kb_id) {
                return Err(retained_raw_failure(
                    run.source_id,
                    run.raw_commit_sha,
                    IngestFailurePhase::Committed,
                    Some(&sha),
                    e.context("rebuilding the graph cache after curation committed"),
                ));
            }
            Ok(IngestResult {
                source_id: run.source_id.to_string(),
                raw_commit_sha: run.raw_commit_sha.map(str::to_string),
                commit_sha: sha,
                steps: r.steps_used,
                events: r.events,
                // The tail check. Last, and not a `?`: the pages are committed
                // and the graph is rebuilt, so this describes the run rather
                // than gating it (DR-7). A base with warnings is a normal base
                // mid-curation, and an ingest that failed on one is an ingest
                // nobody runs twice.
                verification: verify(run.kb_root, cancel),
            })
        }
        Ok(r) => Err(aborted_curation_failure(
            repo,
            txn,
            run.source_id,
            run.raw_commit_sha,
            anyhow::anyhow!(
                "ingest sub-agent aborted: reason={:?}, final={}",
                r.reason,
                r.final_text
            ),
        )),
        Err(e) => Err(aborted_curation_failure(
            repo,
            txn,
            run.source_id,
            run.raw_commit_sha,
            e,
        )),
    }
}

/// Everything the sub-agent needs that can fail, in one fallible call so the
/// transaction is aborted exactly once. See the call site for why that matters.
///
/// Returns the run's source node (BioOKF only), the `knowledge/` tree as the
/// sub-agent will find it, and the base's own `schema.md`.
fn ingest_setup(
    _svc: &KnowledgeService,
    kb_root: &std::path::Path,
    repo: &GitRepo,
    txn: &Txn,
    format: Option<KbFormat>,
    source_id: &str,
) -> Result<(Option<SourceNode>, Option<String>, String)> {
    // DR-24: in BioOKF mode the source node is a CONFORMANCE requirement, not a
    // step in a procedure the model may skip. Materialised inside the txn, so it
    // exists before the first edge that has to cite it — and so it goes with the
    // transaction if the run is aborted, rather than being left on main as an
    // orphan Publication nobody cites.
    let source_node = if format.is_some_and(KbFormat::is_biookf) {
        Some(materialize_source_node(kb_root, source_id, txn)?)
    } else {
        None
    };
    // …which means "did the sub-agent write anything" can no longer be asked
    // against main: the seed above already moved the `knowledge/` subtree, and a
    // check against main would answer yes for a run that did nothing (issue
    // #71). The baseline is the tree as the sub-agent finds it.
    let baseline_knowledge = repo.txn_knowledge_tree_id(txn)?;
    let schema_path = crate::knowledge::store::resolve_readable_path(kb_root, "schema.md")?;
    let schema = std::fs::read_to_string(schema_path).context("read schema.md")?;
    Ok((source_node, baseline_knowledge, schema))
}

// ---------------------------------------------------------------------------
// DR-24 — the source node
// ---------------------------------------------------------------------------

/// The sub-agent's first message: which source to integrate, any focus hints,
/// and — in BioOKF mode — the source node's identifier **spelled exactly**.
///
/// Exactly, because every edge the sub-agent writes has to reproduce that string
/// character for character in `primary_source` and in its `reported_in` object.
/// Telling it "cite the source node" and leaving it to reconstruct the name from
/// the source-id is how an ingest ends up with a base full of `primary_source`
/// values that resolve to nothing — one `biookf.edge.primary_source_unresolved`
/// per edge, from a run that looked like it went perfectly.
fn opening_message(
    source_id: &str,
    focus: Option<&str>,
    source_node: Option<&SourceNode>,
) -> String {
    let focus_line = focus.unwrap_or("");
    let mut user =
        format!("New source to integrate: source-id={source_id}. Focus hints: {focus_line}");
    if let Some(node) = source_node {
        user.push_str(&format!(
            "\n\nThe source node for it already exists at {} with identifier: {}\n\
             Use that identifier VERBATIM as `primary_source` on every edge, and give every \
             page you write a `reported_in` edge whose `object` is that same string. Extend \
             that page (type, xref, description) if you learn more about the source itself; do \
             not create a second node for it.",
            node.path, node.identifier
        ));
    }
    user
}

/// Where a run's source node landed, and what every edge must cite.
pub struct SourceNode {
    /// The `identifier` other pages join to. Not the source-id, and not the
    /// path: BioOKF §7.2 joins on `identifier` and nothing else.
    pub identifier: String,
    /// Bundle-relative path, so the sub-agent can read and extend it.
    pub path: String,
}

/// Materialise the ingested source as a real, typed **concept page** (DR-24).
///
/// ## Why this is Rust and not a step in the procedure
///
/// It is a conformance requirement with a hard consequence. BioOKF §8 requires
/// `primary_source` on every edge, and §8.1 requires it to name a
/// Publication/Study/Dataset/Agent **node that exists in the bundle**. If the
/// node is missing, every edge in the ingest fails conformance rule 4 and every
/// one of them raises `biookf.edge.primary_source_unresolved` — a whole ingest's
/// worth of findings from one omission, and an omission a model makes silently
/// because the edges it wrote look perfectly well-formed on their own.
///
/// Asking the procedure to create it first makes the base's conformance
/// contingent on the model having read step 2, on a loop that is bounded at 30
/// steps and may lose the instruction to a long source. Writing it here makes it
/// a floor: the model may improve the page — refine `Publication` to `Study`,
/// add an `xref` it found in the text — and cannot fail to have one.
///
/// ## Provenance, both mechanisms
///
/// The page carries `raw_source`, which anchors the chain to the immutable bytes
/// under `raw/`, and its own `reported_in` edge citing **itself** as
/// `primary_source`. That self-reference is SPEC §8.1's intended terminating
/// base case, not a cycle: a source attests its own contents, and the lint knows
/// it (`check_primary_source` resolves it to a source-typed node and says
/// nothing). Without the self-edge the source node would be the one page in the
/// bundle with no `reported_in`, which reads as an omission rather than as a
/// terminus.
fn materialize_source_node(
    kb_root: &std::path::Path,
    source_id: &str,
    txn: &Txn,
) -> Result<SourceNode> {
    let meta = crate::knowledge::raw::read_meta(kb_root, source_id)
        .with_context(|| format!("read raw/{source_id}/meta.yaml for the source node"))?;
    let identifier = source_identifier(&meta);
    let node_type = source_node_type(&meta);
    let path = format!(
        "knowledge/{}/{}.md",
        node_type.as_str().to_lowercase(),
        source_id
    );
    let content = source_page(&identifier, node_type, &meta, source_id);
    crate::knowledge::store::write_page(
        kb_root,
        &path,
        &content,
        &format!("source node for {source_id}"),
        Some(&txn.branch),
    )?;
    Ok(SourceNode { identifier, path })
}

/// The node's `identifier`: the source's own title when it has one.
///
/// §7.1 wants it human-readable, which the source-id is not — `chen-2020-il6-a1b2c3`
/// is exactly the opaque code that rule exists to keep out of `identifier` and in
/// `xref`. The id is the fallback only because a titleless source has to be
/// called something, and being unresolvable is worse than being ugly.
fn source_identifier(meta: &SourceMeta) -> String {
    let title = meta.title.trim();
    if title.is_empty() {
        meta.id.clone()
    } else {
        title.to_string()
    }
}

/// Which of §8.1's four source types this is.
///
/// Deliberately coarse. Only four types may bear a `primary_source`, and of them
/// only `Dataset` is decidable from a mime type — a CSV or a spreadsheet is
/// data, everything else is something that was *written*. `Publication` is the
/// honest default for a document of unknown kind, and the procedure tells the
/// sub-agent it may refine it (a trial write-up is a `Study`, an authority like
/// HGNC is an `Agent`) once it has read the text. Guessing `Study` from a title
/// containing "trial" would be a heuristic that is wrong quietly.
fn source_node_type(meta: &SourceMeta) -> biookf::NodeType {
    let mime = meta.mime.to_ascii_lowercase();
    let data = mime.contains("csv")
        || mime.contains("tab-separated")
        || mime.contains("spreadsheet")
        || mime.contains("json")
        || mime.contains("parquet");
    if data {
        biookf::NodeType::Dataset
    } else {
        biookf::NodeType::Publication
    }
}

/// The page itself, composed through `serde_yaml` for the same reason
/// `kb_write_concept` does: a title carrying `: ` written into frontmatter by
/// hand produces an unparseable block, and an unparseable block does not fail —
/// the page simply stops being in the graph.
fn source_page(
    identifier: &str,
    node_type: biookf::NodeType,
    meta: &SourceMeta,
    source_id: &str,
) -> String {
    let mut fm = serde_yaml::Mapping::new();
    fm.insert("type".into(), node_type.as_str().into());
    fm.insert("identifier".into(), identifier.into());
    fm.insert(
        "description".into(),
        format!("Source ingested into this knowledge base as `{source_id}`.").into(),
    );
    let xrefs = source_xrefs(meta);
    if !xrefs.is_empty() {
        fm.insert(
            "xref".into(),
            serde_yaml::Value::Sequence(xrefs.iter().map(|x| x.as_str().into()).collect()),
        );
    }
    fm.insert(
        "raw_source".into(),
        serde_yaml::Value::Sequence(vec![format!("raw/{source_id}/source.md").into()]),
    );

    // §8.1's terminating self-citation; see this module's `materialize_source_node`.
    let mut edge = serde_yaml::Mapping::new();
    edge.insert("predicate".into(), "reported_in".into());
    edge.insert("object".into(), identifier.into());
    edge.insert("knowledge_level".into(), "knowledge_assertion".into());
    edge.insert("agent_type".into(), "automated_agent".into());
    edge.insert("primary_source".into(), identifier.into());
    fm.insert(
        "edges".into(),
        serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(edge)]),
    );

    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(fm))
        .unwrap_or_else(|_| format!("type: {}\n", node_type.as_str()));
    let mut body = format!("---\n{yaml}---\n\n# {identifier}\n\n");
    body.push_str(&format!(
        "Ingested source `{source_id}`. The original is at `raw/{source_id}/`.\n"
    ));
    if let Some(url) = meta.url.as_deref().filter(|u| !u.is_empty()) {
        body.push_str(&format!("\nRetrieved from <{url}>.\n"));
    }
    body
}

/// PMID / DOI / arXiv / ISBN, read out of the **title and URL** with the same
/// extractor the credibility classifier already uses.
///
/// One extractor and not a second regex here: two would answer differently
/// about the same source the first time either was tweaked, and an `xref` that
/// disagrees with the credibility record about which paper this is, is worse
/// than an absent one. `xref` is where §7.1 wants the codes — and the reason
/// `identifier` may stay a human-readable title.
///
/// ⚠ **The document body is deliberately not scanned**, and the omission costs
/// real hits: a PDF states its own DOI in the text far more often than in its
/// filename. It is still the right call, because a paper's *reference list* is
/// full of other papers' identifiers, and a regex cannot tell a work's own DOI
/// from one it cites. A missing `xref` is an enrichment opportunity — the node
/// is already anchored by `raw_source`, so `biookf.source.unanchored` does not
/// fire — while a **wrong** one is a false claim about which paper this is,
/// propagated to every edge that cites the node. Reading the document and
/// judging which identifier is its own is a job for the sub-agent, which is why
/// the procedure tells it to extend this page when it learns more.
fn source_xrefs(meta: &SourceMeta) -> Vec<String> {
    let haystack = format!("{} {}", meta.title, meta.url.as_deref().unwrap_or_default());
    let ids = crate::knowledge::credibility::identifiers::extract(&haystack);
    let mut out = Vec::new();
    if let Some(pmid) = ids.pmid {
        out.push(format!("PMID:{pmid}"));
    }
    if let Some(doi) = ids.doi {
        out.push(format!("doi:{doi}"));
    }
    if let Some(arxiv) = ids.arxiv {
        out.push(format!("arXiv:{arxiv}"));
    }
    if let Some(isbn) = ids.isbn {
        out.push(format!("ISBN:{isbn}"));
    }
    out
}

/// Did the model WRITE its tool calls instead of MAKING them?
///
/// A model handed no tool surface still tries to do the job: it emits
/// `<tool_call>{"name": "kb_write", …}</tool_call>` as prose, often inventing
/// its own `<tool_response>OK</tool_response>` to continue against. The loop
/// sees a text-only reply, records zero steps, and the failure reads as "the
/// model refused" — which sends the reader to the prompt, the schema and the
/// source, none of which are the problem.
///
/// Measured on a `claude_code` ingest: a complete, correct plan naming six
/// pages, every call spelled out in text, and nothing written to disk. The
/// cause is upstream and structural — `complete_with_model` on the
/// coding-agent providers accepts `tools` and does not forward them, because
/// their tool surface arrives over the MCP bridge that only the agent's own
/// turn loop establishes. A macro calling the provider directly therefore gets
/// a tool-less model, every time, silently.
///
/// Deliberately keyed on the REPLY and not on the provider name. It is the
/// shape that identifies the failure, so this keeps working for any future
/// provider that cannot carry tools on this path, and stays quiet for a model
/// that merely chose not to call anything.
fn narrated_its_tool_calls(final_text: &str) -> bool {
    const CALL_MARKERS: [&str; 4] = [
        "<tool_call>",
        "<function_call",
        "<invoke",
        "\"name\": \"kb_",
    ];
    const TOOL_NAMES: [&str; 4] = ["kb_write", "kb_append_log", "kb_read", "kb_add_raw_source"];
    CALL_MARKERS.iter().any(|m| final_text.contains(m))
        && TOOL_NAMES.iter().any(|t| final_text.contains(t))
}

/// The message a digest that wrote nothing fails with.
///
/// It has to answer the only question the user has — *what failed* — so it
/// carries the model's own last words and every tool call that errored. Without
/// them a silent model produces a silent failure, which is barely better than
/// the false success it replaces.
fn no_pages_written_error(source_id: &str, result: &SubAgentResult) -> String {
    let mut msg = format!(
        "ingest wrote no knowledge pages for source {source_id} \
         ({} step(s), ended: {:?})",
        result.steps_used, result.reason
    );

    let failures: Vec<String> = result
        .events
        .iter()
        .filter_map(|e| match e {
            SubAgentEvent::ToolResult {
                name,
                ok: false,
                summary,
            } => Some(format!("{name}: {}", clip(summary, 160))),
            _ => None,
        })
        .rev()
        .take(3)
        .collect();
    if !failures.is_empty() {
        msg.push_str(&format!("; failed tool calls: {}", failures.join(" | ")));
    }

    let final_text = result.final_text.trim();
    if final_text.is_empty() {
        msg.push_str(
            "; the model returned no final message, which usually means the \
             provider request failed or was cut short",
        );
    } else {
        if result.steps_used == 0 && narrated_its_tool_calls(final_text) {
            msg.push_str(
                ". The model wrote its tool calls out as text instead of making them, \
                 which is what a model does when it was handed no tools. That is a \
                 property of the run rather than of the source or the prompt. Since \
                 issue #109 a coding-agent provider (Claude Code, Codex) receives its \
                 tools over Biorouter's MCP bridge on this path too, so the usual cause \
                 is now that the bridge was not reachable — check that Biorouter's own \
                 server is running, and try the run again",
            );
        }
        msg.push_str(&format!(
            "; the model's last message was: {}",
            clip(final_text, 400)
        ));
    }
    msg
}

/// Keep the failure message readable: it travels through an SSE frame and lands
/// in a single line of the digest panel, where a few thousand characters of
/// model output would bury the part that explains what happened.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{
        git::GitRepo,
        page_fixtures::valid_page,
        service::KnowledgeService,
        subagent::loop_::{LlmReply, LlmToolCall},
        types::ChangeKind,
    };
    use async_trait::async_trait;
    use rmcp::model::Tool;
    use tokio::sync::Mutex;

    // -------------------------------------------------------------------------
    // Minimal MockCompleter — pops canned replies from a queue.
    // -------------------------------------------------------------------------

    struct MockCompleter {
        replies: Mutex<Vec<LlmReply>>,
    }

    impl MockCompleter {
        fn new(replies: Vec<LlmReply>) -> Self {
            Self {
                replies: Mutex::new(replies),
            }
        }
    }

    #[async_trait]
    impl Completer for MockCompleter {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[crate::knowledge::subagent::loop_::LlmMessage],
            _tools: &[Tool],
        ) -> anyhow::Result<LlmReply> {
            let mut q = self.replies.lock().await;
            if q.is_empty() {
                panic!("MockCompleter ran out of canned replies");
            }
            Ok(q.remove(0))
        }
    }

    fn tool_call_reply(tool_name: &str, args: serde_json::Value) -> LlmReply {
        LlmReply {
            text: String::new(),
            tool_calls: vec![LlmToolCall {
                id: "req-1".into(),
                name: tool_name.to_string(),
                args,
            }],
        }
    }

    fn text_reply(text: &str) -> LlmReply {
        LlmReply {
            text: text.to_string(),
            tool_calls: vec![],
        }
    }

    fn fresh_svc() -> (tempfile::TempDir, KnowledgeService) {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base("k", "K", None).unwrap();
        (dir, svc)
    }

    // -------------------------------------------------------------------------
    // Test 1: happy path — agent writes a page and returns "done"
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ingest_writes_pages_and_commits_one_change() {
        let (_dir, svc) = fresh_svc();

        // The mock completer:
        //   step 0: calls kb_write_page with a stub source page
        //   step 1: returns a text-only reply ("done") → NoMoreToolCalls
        let completer = MockCompleter::new(vec![
            tool_call_reply(
                "kb_write_page",
                serde_json::json!({
                    "path": "knowledge/sources/stub.md",
                    "content": valid_page("source", "stub", "Stub."),
                    "commit_message": "add source"
                }),
            ),
            text_reply("done"),
        ]);

        let result = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .unwrap();

        // 1 iteration of tool-call + 1 step with text reply = steps_used 1
        assert_eq!(result.steps, 1);
        assert!(
            result
                .raw_commit_sha
                .as_deref()
                .is_some_and(|sha| !sha.is_empty()),
            "a new raw source reports the commit that retained it"
        );
        assert!(!result.commit_sha.is_empty());
        assert_ne!(
            result.raw_commit_sha.as_deref(),
            Some(result.commit_sha.as_str())
        );

        // The ingest commit must appear as the latest history entry.
        let log = svc.list_history("k", 10).unwrap();
        // Expected: create + add_raw + squashed-ingest = 3
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].kind, ChangeKind::Ingest);

        // The source page written by the sub-agent must be on main.
        let kb = svc.root().join("k");
        assert!(
            kb.join("knowledge/sources/stub.md").exists(),
            "source page must exist after commit"
        );
    }

    /// A graph-cache write happens after the curation commit. If that derived
    /// refresh fails, retrying the digest would duplicate work that is already
    /// durable, so the failure must carry the curation commit and must never
    /// claim rollback.
    #[tokio::test]
    async fn a_post_commit_refresh_failure_reports_the_curation_commit() {
        let (_dir, svc) = fresh_svc();
        let raw = svc
            .add_raw_source(
                "k",
                SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                None,
            )
            .await
            .unwrap();
        let kb = paths::kb_root(svc.root(), "k");
        let repo = GitRepo::open(&kb).unwrap();
        let txn = repo.begin_txn("post-commit-refresh").unwrap();
        let baseline_knowledge = repo.txn_knowledge_tree_id(&txn).unwrap();
        let page = kb.join("knowledge/sources/stub.md");
        std::fs::create_dir_all(page.parent().unwrap()).unwrap();
        std::fs::write(&page, valid_page("source", "stub", "Stub.")).unwrap();
        repo.commit_on_txn(&txn, "write curated page").unwrap();

        // `commit_txn` repairs after its checkout and `settle_ingest` refreshes
        // once more, so fail both writes to reach the post-commit result arm.
        crate::knowledge::graph::fail_cache_writes(&kb, 2);

        let err = settle_ingest(
            &svc,
            &repo,
            &txn,
            IngestRun {
                kb_id: "k",
                kb_root: &kb,
                source_id: &raw.source_id,
                raw_commit_sha: raw.commit_sha.as_deref(),
                baseline_knowledge,
            },
            Ok(SubAgentResult {
                steps_used: 1,
                reason: DoneReason::NoMoreToolCalls,
                final_text: "done".into(),
                events: vec![],
            }),
            None,
        )
        .expect_err("the failed post-commit refresh must be reported");

        let failure = err
            .downcast_ref::<IngestCurationFailure>()
            .expect("the phase and durable commits remain machine-readable");
        assert_eq!(failure.phase, IngestFailurePhase::Committed);
        let curation_commit = failure
            .curation_commit_sha
            .as_deref()
            .expect("a post-commit failure reports the curation commit");
        assert!(
            svc.list_history("k", 10)
                .unwrap()
                .iter()
                .any(|entry| entry.commit_sha == curation_commit),
            "the reported curation commit must be reachable from history"
        );
        let message = err.to_string();
        assert!(message.contains("Curation committed"), "{message}");
        assert!(
            message.contains("Refresh the knowledge base and history"),
            "{message}"
        );
        assert!(!message.contains("Curation rolled back"), "{message}");
        assert!(
            !crate::knowledge::graph::cache_path(&kb).exists(),
            "an older cache survived the failed post-commit rebuild"
        );

        let history_len = svc.list_history("k", 10).unwrap().len();
        let retry = svc
            .add_raw_source(
                "k",
                SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(retry.source_id, raw.source_id);
        assert!(retry.commit_sha.is_none());
        assert_eq!(svc.list_history("k", 10).unwrap().len(), history_len);
    }

    /// Rollback is a fallible operation. If it cannot be verified, the error
    /// must preserve that uncertainty instead of promising a rollback that may
    /// not have happened.
    #[tokio::test]
    async fn an_abort_failure_does_not_claim_curation_rolled_back() {
        let (_dir, svc) = fresh_svc();
        let raw = svc
            .add_raw_source(
                "k",
                SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                None,
            )
            .await
            .unwrap();
        let repo = GitRepo::open(&paths::kb_root(svc.root(), "k")).unwrap();
        let txn = repo.begin_txn("abort-failure").unwrap();
        repo.abort_txn(&txn).unwrap();

        let err = aborted_curation_failure(
            &repo,
            &txn,
            &raw.source_id,
            raw.commit_sha.as_deref(),
            anyhow::anyhow!("curation failed"),
        );
        let failure = err.downcast_ref::<IngestCurationFailure>().unwrap();
        assert_eq!(failure.phase, IngestFailurePhase::OutcomeUncertain);
        let message = err.to_string();
        assert!(
            message.contains("rollback could not be verified"),
            "{message}"
        );
        assert!(!message.contains("Curation rolled back"), "{message}");
    }

    /// An error before raw staging completes has no retained-source provenance
    /// and must not be wrapped as though raw bytes were durable.
    #[tokio::test]
    async fn a_raw_source_failure_is_not_reported_as_retained() {
        let (_dir, svc) = fresh_svc();
        let err = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Path(std::path::PathBuf::from(
                    "/nonexistent/biorouter-ingest-source.md",
                )),
                completer: Box::new(MockCompleter::new(vec![])),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .expect_err("a missing input cannot be staged");

        assert!(err.downcast_ref::<IngestCurationFailure>().is_none());
        assert!(!err.to_string().contains("raw source"));
    }

    /// The raw commit happens before its derived-cache refresh. A refresh error
    /// must therefore carry the retained source and commit as a typed
    /// not-started curation outcome, not masquerade as a pre-write failure.
    #[tokio::test]
    async fn a_raw_post_commit_cache_failure_reports_the_durable_source() {
        let (_dir, svc) = fresh_svc();
        let kb = paths::kb_root(svc.root(), "k");
        // The macro's best-effort preflight refresh consumes the first injected
        // failure. The second reaches the post-commit refresh in add_raw_source.
        crate::knowledge::graph::fail_cache_writes(&kb, 2);

        let err = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "A raw source whose cache refresh will fail.".into(),
                    title: Some("Retained raw fixture".into()),
                },
                completer: Box::new(MockCompleter::new(vec![])),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .expect_err("the blocked graph-cache refresh must be reported");

        let failure = err
            .downcast_ref::<IngestCurationFailure>()
            .expect("a durable raw commit keeps typed provenance");
        assert_eq!(failure.phase, IngestFailurePhase::NotStarted);
        let raw_commit = failure
            .raw_commit_sha
            .as_deref()
            .expect("the durable raw commit is reported");
        assert!(
            kb.join("raw")
                .join(&failure.source_id)
                .join("source.md")
                .is_file(),
            "the reported raw source must exist"
        );
        assert!(
            svc.list_history("k", 10)
                .unwrap()
                .iter()
                .any(|entry| entry.commit_sha == raw_commit),
            "the reported raw commit must remain reachable"
        );
        let message = err.to_string();
        assert!(message.contains(&failure.source_id), "{message}");
        assert!(message.contains(raw_commit), "{message}");
        assert!(message.contains("Curation did not start"), "{message}");
        assert!(
            !crate::knowledge::graph::cache_path(&kb).exists(),
            "an older cache survived the failed raw post-commit rebuild"
        );
    }

    // -------------------------------------------------------------------------
    // DR-24: the source node is materialised, not asked for
    // -------------------------------------------------------------------------

    fn biookf_svc() -> (tempfile::TempDir, KnowledgeService) {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base_as(
            "bio",
            "Bio",
            None,
            KbFormat::Biookf,
            false,
            &Default::default(),
        )
        .unwrap();
        (dir, svc)
    }

    fn biookf_args(completer: MockCompleter) -> IngestArgs {
        IngestArgs {
            kb_id: "bio".into(),
            caller_is_private: false,
            caller_affiliation: Default::default(),
            source: SourceInput::Text {
                text: "Tocilizumab reduced mortality in severe COVID-19. PMID: 33933206".into(),
                title: Some("RECOVERY: tocilizumab in COVID-19".into()),
            },
            completer: Box::new(completer),
            focus: None,
            bounds: SubAgentBounds::default(),
            event_sink: None,
            cancel: None,
        }
    }

    /// rq2-029. A corrupt PDF fails before the sub-agent starts, and the error
    /// a route shows says why in the person's words. It used to read
    /// `add_raw_source` at its outermost layer, which is all a route printed.
    #[tokio::test]
    async fn an_unreadable_source_fails_the_ingest_with_its_cause() {
        let (_dir, svc) = biookf_svc();
        // No replies: the model must never be asked about a file it cannot see.
        let mut args = biookf_args(MockCompleter::new(Vec::new()));
        args.source = SourceInput::File {
            bytes: b"%PDF-1.7\nthis is not a pdf body at all\n%%EOF\n".to_vec(),
            filename: "broken.pdf".into(),
            mime: Some("application/pdf".into()),
        };
        let error = ingest(&svc, args)
            .await
            .expect_err("a corrupt PDF cannot be ingested");
        let outermost = error.to_string();
        assert!(
            outermost.starts_with("Could not read this source: ")
                && outermost.contains("PDF text extraction failed"),
            "{outermost}"
        );
        assert!(
            !format!("{error:#}").contains("add_raw_source"),
            "{error:#}"
        );
    }

    // -------------------------------------------------------------------------
    // The tail verification pass
    // -------------------------------------------------------------------------

    /// A run that wrote non-conformant pages says so, and **still succeeds**.
    ///
    /// Both halves matter. Before this the macro reported success and nothing
    /// else, so fifteen bad pages could be committed and announced as a clean
    /// digest — and a check that FAILED the run on findings would be worse,
    /// because DR-7 makes conformance findings a description and a base
    /// mid-curation legitimately carries them.
    #[tokio::test]
    async fn a_run_that_wrote_non_conformant_pages_reports_them_and_still_succeeds() {
        let (_dir, svc) = biookf_svc();
        let completer = MockCompleter::new(vec![
            tool_call_reply(
                "kb_write_page",
                serde_json::json!({
                    // `Molecules` is not one of the 28, and the page cites
                    // nothing: exactly the shape a hurried run leaves behind.
                    "path": "knowledge/molecule/tocilizumab.md",
                    "content": "---\ntype: Molecules\nidentifier: Tocilizumab\n---\n\n# Tocilizumab\n",
                    "commit_message": "add molecule",
                }),
            ),
            text_reply("done"),
        ]);
        let result = ingest(&svc, biookf_args(completer))
            .await
            .expect("findings describe the run; they do not fail it");

        assert!(!result.commit_sha.is_empty(), "the pages are committed");
        assert!(!result.verification.ok, "{:#?}", result.verification);
        assert!(
            result.verification.errors >= 1,
            "{:#?}",
            result.verification
        );
        assert_eq!(result.verification.scan_error, None);
        assert!(
            result
                .verification
                .diagnostics
                .has(crate::knowledge::biookf::lint::RULE_TYPE_INVALID),
            "{:#?}",
            result.verification.diagnostics.items
        );
        // The pre-cap total travels with the capped list, so a caller can tell
        // "that is everything" from "that is the first two hundred".
        assert!(result.verification.diagnostics.total >= 1);
    }

    /// The other half: a conformant run verifies clean, so the assertion above
    /// is not satisfied by a check that reports errors unconditionally.
    #[tokio::test]
    async fn a_conformant_run_verifies_clean() {
        let (_dir, svc) = fresh_svc();
        let completer = MockCompleter::new(vec![
            tool_call_reply(
                "kb_write_page",
                serde_json::json!({
                    "path": "knowledge/sources/stub.md",
                    "content": "---\ntype: Note\nidentifier: HRV note\n---\n\nStub.\n",
                    "commit_message": "add source"
                }),
            ),
            text_reply("done"),
        ]);
        let result = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .unwrap();
        assert!(
            result.verification.ok,
            "{:#?}",
            result.verification.diagnostics.items
        );
        assert_eq!(result.verification.errors, 0);
        // …while still carrying the orphan warning, which is the "report, do not
        // block" rule stated from the other side: a warning is information, and
        // `ok` is about errors alone.
        assert!(
            result
                .verification
                .diagnostics
                .has(super::super::lint::RULE_ORPHAN),
            "{:#?}",
            result.verification.diagnostics.items
        );
        assert!(result.verification.warnings >= 1);
    }

    /// "Unverified" is not "verified clean". `Verification::default()` is what a
    /// pre-existing `IngestResult` deserializes into and what a failed scan
    /// leaves behind, and neither may read as a pass.
    #[test]
    fn an_unverified_result_never_reads_as_clean() {
        assert!(!Verification::default().ok);
        let stored: IngestResult = serde_json::from_value(serde_json::json!({
            "source_id": "s", "commit_sha": "abc", "steps": 1, "events": []
        }))
        .expect("a result written before the field existed still loads");
        assert_eq!(stored.raw_commit_sha, None);
        assert!(!stored.verification.ok);
    }

    /// The conformance floor. Every edge a BioOKF ingest writes must cite a
    /// `primary_source` that resolves to a Publication/Study/Dataset/Agent page
    /// **in the bundle** (§8.1); without one, every edge in the run raises
    /// `biookf.edge.primary_source_unresolved` — a whole ingest's worth of
    /// findings from one omission the model makes silently, because the edges it
    /// wrote look well-formed on their own.
    #[tokio::test]
    async fn a_biookf_ingest_materializes_the_source_node_before_the_sub_agent_runs() {
        let (_dir, svc) = biookf_svc();
        let completer = MockCompleter::new(vec![
            tool_call_reply(
                "kb_write_concept",
                serde_json::json!({
                    "type": "Molecule",
                    "identifier": "Tocilizumab",
                    "edges": [{
                        "predicate": "reported_in",
                        "object": "RECOVERY: tocilizumab in COVID-19",
                        "knowledge_level": "knowledge_assertion",
                        "agent_type": "text_mining_agent",
                        "primary_source": "RECOVERY: tocilizumab in COVID-19",
                    }],
                }),
            ),
            text_reply("done"),
        ]);
        ingest(&svc, biookf_args(completer)).await.unwrap();

        let kb = svc.root().join("bio");
        let pages = crate::knowledge::validate::load_bundle(&kb).unwrap();
        let source = pages
            .iter()
            .find(|p| p.path.starts_with("knowledge/publication/"))
            .expect("the source node was materialised");

        // Its identifier is the source's own title, not the opaque source-id
        // §7.1 keeps out of `identifier` and in `xref`.
        assert_eq!(
            source.doc.identifier.as_deref(),
            Some("RECOVERY: tocilizumab in COVID-19")
        );
        // `raw_source` anchors the chain to the immutable bytes — which is
        // what makes an absent `xref` an enrichment opportunity rather than an
        // unanchored source. See `source_xrefs` for why the body is not
        // scanned for identifiers.
        assert!(source.doc.xref.is_empty(), "{:?}", source.doc.xref);
        let text = std::fs::read_to_string(kb.join(&source.path)).unwrap();
        assert!(text.contains("raw_source:"), "{text}");
        assert!(text.contains("/source.md"), "{text}");

        // DR-24 names TWO provenance mechanisms and DR-4 carried only one, so
        // the source node's own `reported_in` edge is asserted rather than
        // assumed: without it the source is the one page in the bundle with no
        // `reported_in`, which reads as an omission rather than as the chain's
        // terminus. It cites ITSELF as `primary_source` — §8.1's intended
        // terminating base case.
        let self_edge = source
            .doc
            .edges
            .iter()
            .find(|e| e.predicate == "reported_in")
            .expect("the source node carries its own reported_in edge");
        assert_eq!(self_edge.object, "RECOVERY: tocilizumab in COVID-19");
        assert_eq!(
            self_edge.primary_source.as_deref(),
            Some("RECOVERY: tocilizumab in COVID-19"),
            "a source attests its own contents"
        );

        // And the whole bundle is conformant: no edge cites a source that does
        // not resolve, and that self-citation is not reported as a dangling one.
        let diagnostics = crate::knowledge::validate::validate_page(
            Some(KbFormat::Biookf),
            Some(&source.path),
            &text,
            &pages,
        );
        assert_eq!(
            diagnostics.errors(),
            0,
            "the source node must be conformant: {:#?}",
            diagnostics.items
        );
        assert!(
            !diagnostics.has("biookf.edge.primary_source_unresolved"),
            "the self-citation is the intended terminus, not a dangling source: {:#?}",
            diagnostics.items
        );
    }

    /// The identifiers that *are* safe to read: the ones in the source's own
    /// title and URL, which name this document and not one it cites.
    #[test]
    fn a_source_url_or_title_carrying_an_identifier_becomes_an_xref() {
        let meta = |title: &str, url: Option<&str>| SourceMeta {
            id: "s".into(),
            title: title.into(),
            url: url.map(str::to_string),
            ingested_at: chrono::Utc::now(),
            sha256: String::new(),
            mime: "text/markdown".into(),
            original_filename: None,
            credibility: crate::knowledge::types::Credibility {
                tier: crate::knowledge::types::CredibilityTier::Web,
                confidence: 0.0,
                publisher: None,
                venue: None,
                doi: None,
                retracted: false,
                reasoning: String::new(),
                classifier_version: 1,
            },
        };
        assert_eq!(
            source_xrefs(&meta(
                "Tocilizumab in COVID-19",
                Some("https://pubmed.ncbi.nlm.nih.gov/33933206"),
            )),
            vec!["PMID:33933206".to_string()]
        );
        assert_eq!(
            source_xrefs(&meta("Something (10.1016/S0140-6736(21)00676-0)", None)),
            vec!["doi:10.1016/s0140-6736(21)00676-0".to_string()]
        );
        assert!(source_xrefs(&meta("An untitled note", None)).is_empty());
    }

    /// A model that was handed no tools must not be reported as a model that
    /// refused.
    ///
    /// This text is taken from a real `claude_code` ingest on merged main. The
    /// plan is correct and complete - six pages, the right types, the right
    /// links - and every call is spelled out as prose, including invented
    /// `<tool_response>OK</tool_response>` replies to continue against. Nothing
    /// reached disk, and the failure quoted that prose back under "the model's
    /// last message was", which reads as a model that would not cooperate.
    ///
    /// The real cause was structural and upstream: `complete_with_model` on the
    /// coding-agent providers accepted `tools` and did not forward them, since
    /// their tool surface arrived over a bridge only a chat turn set up. A macro
    /// calling the provider directly got a tool-less model every time. Nobody
    /// reading the old message would find that.
    ///
    /// ⚠ **That cause is fixed** (#109): a macro turn goes through
    /// `ProviderToolTurnContext`, which issues the grant. This detector stays,
    /// because it is keyed on the SHAPE of the reply rather than on a provider
    /// name — so it still catches any future path that hands a model no tools,
    /// including this one when the bridge is unreachable. Only the remedy it
    /// offers changed.
    #[test]
    fn a_model_that_narrated_its_tool_calls_is_named_as_such() {
        let narrated = "**Step 1 - Source page**\n<tool_call>\n{\"name\": \"kb_write\",                         \"arguments\": {\"path\": \"knowledge/source/tocilizumab.md\"}}\n                        </tool_call>\n<tool_response>\nOK\n</tool_response>";
        assert!(
            narrated_its_tool_calls(narrated),
            "the shape of a narrated call must be recognised"
        );

        let result = SubAgentResult {
            steps_used: 0,
            reason: crate::knowledge::subagent::events::DoneReason::NoMoreToolCalls,
            final_text: narrated.to_string(),
            events: Vec::new(),
        };
        let msg = no_pages_written_error("src-1", &result);
        assert!(
            msg.contains("wrote its tool calls out as text"),
            "the failure must name what actually happened: {msg}"
        );
        assert!(
            msg.contains("try the run again"),
            "and what to do about it: {msg}"
        );
        // #109: the old text told the user to choose a different model, because
        // the coding-agent providers genuinely could not drive a macro. They can
        // now — they receive their tools over the bridge on this path too — so
        // that advice would send someone to change a working configuration
        // instead of looking at the reachability that actually failed.
        assert!(
            !msg.contains("Choose a different model"),
            "the fixed limitation must not still be offered as the remedy: {msg}"
        );
    }

    /// ⚠ And it must stay QUIET for a model that simply chose not to call
    /// anything, which is a different failure with a different fix. Blaming the
    /// provider there would send the reader to the model picker for a problem
    /// in the source or the prompt.
    #[test]
    fn a_model_that_merely_declined_is_not_blamed_on_the_provider() {
        assert!(!narrated_its_tool_calls(
            "I could not find anything to record."
        ));
        let result = SubAgentResult {
            steps_used: 0,
            reason: crate::knowledge::subagent::events::DoneReason::NoMoreToolCalls,
            final_text: "I could not find anything worth recording here.".to_string(),
            events: Vec::new(),
        };
        let msg = no_pages_written_error("src-1", &result);
        assert!(!msg.contains("wrote its tool calls out as text"), "{msg}");
        assert!(
            msg.contains("could not find anything worth recording"),
            "{msg}"
        );
    }

    /// ⚠ **The regression the seed creates.** `txn_wrote_knowledge_pages`
    /// compares the txn branch's `knowledge/` subtree against **main**, which is
    /// the right question only while main is the last thing that wrote
    /// knowledge. Seeding the source node moves that subtree before the
    /// sub-agent starts — so a run in which the model wrote nothing at all would
    /// pass the check, commit, and hand the caller a sha for work that never
    /// happened. That is exactly the false success issue #71 closed.
    #[tokio::test]
    async fn the_seeded_source_node_does_not_by_itself_count_as_a_digest() {
        let (_dir, svc) = biookf_svc();
        // The model reads and then gives up: no page written.
        let completer = MockCompleter::new(vec![
            tool_call_reply("kb_list_pages", serde_json::json!({})),
            text_reply("I could not find anything to record."),
        ]);
        let err = ingest(&svc, biookf_args(completer))
            .await
            .expect_err("a run that wrote no knowledge must fail, seed or no seed");
        assert!(
            err.to_string().contains("wrote no knowledge pages"),
            "{err}"
        );

        // …and the seed went with the aborted transaction rather than being
        // left on main as an orphan Publication nobody cites.
        let kb = svc.root().join("bio");
        let orphans =
            crate::knowledge::store::list_pages(&kb, Some("knowledge/publication")).unwrap();
        assert!(
            orphans.is_empty(),
            "the aborted txn left the source node behind: {orphans:?}"
        );
    }

    /// ⚠ **A failure between `begin_txn` and the sub-agent leaves HEAD parked on
    /// the transaction branch**, and every later write to this KB then lands
    /// somewhere nobody is looking. The `schema.md` read has been on that stretch
    /// since the macro was written and never aborted; Stage 5 put two more
    /// fallible steps beside it, so all three now go through `ingest_setup` and
    /// the transaction is aborted exactly once.
    ///
    /// Provoked by making `schema.md` a directory, which is the cheapest way to
    /// make exactly that read fail while everything before it succeeds.
    #[tokio::test]
    async fn a_failure_setting_up_the_transaction_still_leaves_head_on_main() {
        let (dir, svc) = biookf_svc();
        let schema = dir.path().join("bio/schema.md");
        std::fs::remove_file(&schema).unwrap();
        std::fs::create_dir(&schema).unwrap();

        let completer = MockCompleter::new(vec![text_reply("never reached")]);
        let err = ingest(&svc, biookf_args(completer))
            .await
            .expect_err("an unreadable schema.md fails the digest");
        assert!(err.to_string().contains("schema.md"), "{err}");

        let repo = git2::Repository::open(dir.path().join("bio")).unwrap();
        assert_eq!(
            repo.head().unwrap().shorthand(),
            Some("main"),
            "a failed setup must leave HEAD on main"
        );
        let leftover: Vec<String> = repo
            .branches(Some(git2::BranchType::Local))
            .unwrap()
            .filter_map(|b| b.ok()?.0.name().ok()?.map(str::to_string))
            .filter(|n| n.starts_with("txn/"))
            .collect();
        assert!(
            leftover.is_empty(),
            "the transaction branch must be deleted; found: {leftover:?}"
        );
    }

    /// An OKF base has no closed vocabulary and no §8.1 source-node rule, so it
    /// gets neither the seed nor the typed writer. The negative half: a source
    /// node minted into an OKF base would be a `Publication`-typed page in a
    /// base whose types are the user's own.
    #[tokio::test]
    async fn an_okf_ingest_seeds_nothing_and_keeps_the_permissive_tools() {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base_as(
            "okf",
            "Okf",
            None,
            KbFormat::Okf,
            false,
            &Default::default(),
        )
        .unwrap();
        let completer = MockCompleter::new(vec![
            tool_call_reply(
                "kb_write_page",
                serde_json::json!({
                    "path": "knowledge/source/note.md",
                    "content": valid_page("Source", "Note", "Body."),
                }),
            ),
            text_reply("done"),
        ]);
        ingest(
            &svc,
            IngestArgs {
                kb_id: "okf".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "A note.".into(),
                    title: Some("A note".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .unwrap();
        let kb = svc.root().join("okf");
        assert!(
            crate::knowledge::store::list_pages(&kb, Some("knowledge/publication"))
                .unwrap()
                .is_empty(),
            "an OKF base must not be given a BioOKF source node"
        );
    }

    /// A CSV is data. The only one of §8.1's four source types that is decidable
    /// from a mime type — everything else was *written*, and guessing `Study`
    /// from a title containing "trial" would be a heuristic that is wrong
    /// quietly.
    #[tokio::test]
    async fn a_tabular_source_is_typed_dataset_and_a_document_is_typed_publication() {
        let (_dir, svc) = biookf_svc();
        let completer = MockCompleter::new(vec![
            tool_call_reply(
                "kb_write_concept",
                serde_json::json!({ "type": "Gene", "identifier": "IL6" }),
            ),
            text_reply("done"),
        ]);
        let mut args = biookf_args(completer);
        args.source = SourceInput::File {
            bytes: b"gene,effect\nIL6,0.3\n".to_vec(),
            filename: "effects.csv".into(),
            mime: Some("text/csv".into()),
        };
        ingest(&svc, args).await.unwrap();
        let kb = svc.root().join("bio");
        assert!(
            !crate::knowledge::store::list_pages(&kb, Some("knowledge/dataset"))
                .unwrap()
                .is_empty(),
            "a csv source is a Dataset node"
        );
    }

    // -------------------------------------------------------------------------
    // Test 2: step budget exceeded → txn aborted, git history unchanged
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ingest_aborts_txn_on_step_budget() {
        let (_dir, svc) = fresh_svc();

        // Always return a tool call so the loop never ends.
        let replies: Vec<LlmReply> = (0..20)
            .map(|_| tool_call_reply("kb_list_pages", serde_json::json!({})))
            .collect();
        let completer = MockCompleter::new(replies);

        let err = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "Some note.".into(),
                    title: Some("x".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds {
                    max_steps: 3,
                    ..Default::default()
                },
                event_sink: None,
                cancel: None,
            },
        )
        .await;

        assert!(err.is_err(), "should fail when step budget exceeded");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.to_lowercase().contains("step")
                || msg.to_lowercase().contains("budget")
                || msg.to_lowercase().contains("aborted"),
            "error should mention budget or abort, got: {msg}"
        );

        // The KB history must not have a macro-level Ingest commit (the squash-merge).
        // add_raw_source itself commits with ChangeKind::Ingest and summary "ingested <id>",
        // which is expected. The macro's squash-merge would produce summary "ingest <id>"
        // (no "d" suffix). We check that no such commit exists.
        let log = svc.list_history("k", 10).unwrap();
        let has_macro_ingest_commit = log.iter().any(|e| {
            e.kind == ChangeKind::Ingest
                && e.delta
                    .as_deref()
                    .map(|d| d.contains("steps"))
                    .unwrap_or(false)
        });
        assert!(
            !has_macro_ingest_commit,
            "no macro ingest commit (with 'steps' delta) should appear after budget exceeded; log: {log:?}"
        );
    }

    // -------------------------------------------------------------------------
    // Test 2b: the sub-agent ended cleanly but wrote nothing → must NOT succeed
    //
    // Issue #71: a provider that fails mid-request commonly ends the turn with a
    // plain sentence and no tool calls, and Google's decoder returns a *content
    // free* assistant message when a candidate carries no parts (finishReason
    // MAX_TOKENS / SAFETY). Both land on `DoneReason::NoMoreToolCalls`, which the
    // macro read as "the agent is finished" — squash-committing an unchanged tree
    // and handing the caller a commit sha. The digest reported "completed" while
    // the KB gained no knowledge page at all.
    // -------------------------------------------------------------------------

    /// The reply carries an error sentence and no tool calls.
    #[tokio::test]
    async fn ingest_fails_when_the_subagent_writes_nothing() {
        let (_dir, svc) = fresh_svc();

        let completer = MockCompleter::new(vec![text_reply(
            "I'm sorry - the model provider returned an error and I cannot continue.",
        )]);

        let result = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await;

        let err =
            result.expect_err("a digest that wrote no knowledge page must not report success");
        let retained = err
            .downcast_ref::<IngestCurationFailure>()
            .expect("failures after staging retain typed raw-source provenance");
        assert_eq!(retained.phase, IngestFailurePhase::RolledBack);
        assert_eq!(retained.curation_commit_sha, None);
        let raw_commit = retained
            .raw_commit_sha
            .as_deref()
            .expect("the newly retained raw source has a commit");
        assert!(!retained.source_id.is_empty());

        let kb = paths::kb_root(svc.root(), "k");
        assert!(
            kb.join("raw")
                .join(&retained.source_id)
                .join("source.md")
                .is_file(),
            "curation rollback must preserve the durable raw source"
        );

        let err = err.to_string();
        assert!(
            err.contains("no knowledge"),
            "the error must say the digest wrote nothing, got: {err}"
        );
        assert!(
            err.contains("model provider returned an error"),
            "the error must carry the model's own last words so the user learns \
             what failed, got: {err}"
        );
        assert!(
            err.contains("Curation rolled back; raw source") && err.contains(raw_commit),
            "the error must distinguish rolled-back curation from retained raw data: {err}"
        );
        assert!(
            !err.contains("Nothing was added") && !err.contains("base is unchanged"),
            "the error must not deny the durable raw-source write: {err}"
        );

        // The raw-source commit remains in history; no curation commit may be
        // added on top of it.
        let log = svc.list_history("k", 10).unwrap();
        assert!(
            log.iter().any(|entry| entry.commit_sha == raw_commit),
            "the reported raw commit must remain reachable in history: {log:?}"
        );
        assert!(
            !log.iter().any(|e| e
                .delta
                .as_deref()
                .map(|d| d.contains("steps"))
                .unwrap_or(false)),
            "no macro ingest commit may exist after an empty digest; log: {log:?}"
        );
    }

    /// The provider hands back a completely empty assistant message — no text and
    /// no tool calls. Google produces exactly this when the candidate has no
    /// `parts` (thinking budget exhausted, safety stop).
    #[tokio::test]
    async fn ingest_fails_when_the_provider_returns_an_empty_reply() {
        let (_dir, svc) = fresh_svc();

        let completer = MockCompleter::new(vec![text_reply("")]);

        let err = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .expect_err("an empty provider reply must not report a completed digest")
        .to_string();

        assert!(
            err.contains("no knowledge"),
            "the error must say the digest wrote nothing, got: {err}"
        );
    }

    /// Every tool call failed, then the model gave up with a text reply. The
    /// error must name the tool failures — "what failed" is the whole point.
    #[tokio::test]
    async fn ingest_failure_names_the_failed_tool_calls() {
        let (_dir, svc) = fresh_svc();

        let completer = MockCompleter::new(vec![
            tool_call_reply(
                "kb_write_page",
                serde_json::json!({
                    // A conformant body on purpose. The one thing wrong with
                    // this call is its path, and a body the writer would also
                    // refuse (Stage 3's validator, DR-19) would satisfy the
                    // assertion below with the traversal guard removed.
                    "path": "knowledge/../escape.md",
                    "content": valid_page("source", "Escape", "Body."),
                    "commit_message": "escape"
                }),
            ),
            text_reply("I could not write the page."),
        ]);

        let err = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .expect_err("a digest whose every tool call failed must not report success")
        .to_string();

        assert!(
            err.contains("kb_write_page"),
            "the error must name the tool that failed, got: {err}"
        );
    }

    // -------------------------------------------------------------------------
    // Test 2c: the run touched the transaction, but never wrote knowledge
    //
    // "The tree changed" is not the same as "a digest happened". Three of the
    // sub-agent's tools write outside `knowledge/`: `kb_append_log` appends to
    // `log.md`, `kb_add_raw_source` materialises under `raw/`, and `kb_write_page`
    // accepts the top-level `index.md`. Each commits on the transaction branch,
    // so each on its own moves the txn tree away from main's while the KB gains
    // no knowledge at all.
    //
    // This is not a hypothetical: `INGEST_PROCEDURE` asks for the source page
    // (step 4), the index update (step 9) and the log line (step 10), so a
    // provider that dies part-way through — the reported failure — routinely
    // leaves exactly this state behind.
    // -------------------------------------------------------------------------

    /// Only `log.md` was touched: the run announced a digest it never performed.
    #[tokio::test]
    async fn ingest_fails_when_the_run_only_appended_a_log_line() {
        let (_dir, svc) = fresh_svc();

        let completer = MockCompleter::new(vec![
            tool_call_reply(
                "kb_append_log",
                serde_json::json!({ "summary": "digested the HRV note", "kind": "ingest" }),
            ),
            text_reply("The provider failed before I could write the page."),
        ]);

        let err = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .expect_err("a log line is not a digest and must not report success")
        .to_string();

        assert!(
            err.contains("no knowledge"),
            "the error must say the digest wrote no knowledge, got: {err}"
        );

        // The claim has to hold on disk: nothing under knowledge/.
        let pages = crate::knowledge::store::list_pages(
            &paths::kb_root(svc.root(), "k"),
            Some("knowledge/"),
        )
        .unwrap();
        assert!(
            pages.is_empty(),
            "no knowledge page may exist after a log-only run; pages: {pages:?}"
        );
    }

    /// Only `raw/` was touched: a second source was materialised and nothing was
    /// ever integrated into the wiki.
    #[tokio::test]
    async fn ingest_fails_when_the_run_only_added_another_raw_source() {
        let (_dir, svc) = fresh_svc();

        let completer = MockCompleter::new(vec![
            tool_call_reply(
                "kb_add_raw_source",
                serde_json::json!({ "type": "text", "text": "A second note.", "title": "Second" }),
            ),
            text_reply("I stashed the source but could not integrate it."),
        ]);

        let err = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .expect_err("stashing a raw source is not a digest and must not report success")
        .to_string();

        assert!(
            err.contains("no knowledge"),
            "the error must say the digest wrote no knowledge, got: {err}"
        );
    }

    // -------------------------------------------------------------------------
    // Test 2d: the provider itself fails mid-run
    //
    // `ProviderCompleter` now *fails* a completion when the provider hands back a
    // tool call Biorouter cannot decode, rather than dropping it and letting the
    // run look finished. That turns a provider error into an `Err` travelling out
    // of the sub-agent loop — a route through this macro that nothing exercised
    // before, and the one the reported failure takes.
    //
    // What it must leave behind is a usable knowledge base. `abort_txn` moves HEAD
    // back to main and deletes the transaction branch; if it ever stopped doing
    // so, every later write to this KB would commit onto a branch nobody reads
    // while the Knowledge view kept reading main. The user would digest the next
    // source "successfully" and still see nothing — issue #70's symptom, arrived
    // at from the other end.
    // -------------------------------------------------------------------------

    /// Fails once the canned replies run out, the way `ProviderCompleter` does.
    struct FailsWhenRepliesRunOut {
        replies: Mutex<Vec<LlmReply>>,
    }

    #[async_trait]
    impl Completer for FailsWhenRepliesRunOut {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[crate::knowledge::subagent::loop_::LlmMessage],
            _tools: &[Tool],
        ) -> anyhow::Result<LlmReply> {
            let mut q = self.replies.lock().await;
            match q.is_empty() {
                true => anyhow::bail!(
                    "the model requested a tool call Biorouter could not decode: \
                     The provided function name 'kb.write page' had invalid characters"
                ),
                false => Ok(q.remove(0)),
            }
        }
    }

    #[tokio::test]
    async fn a_provider_error_aborts_the_txn_and_leaves_head_on_main() {
        let (dir, svc) = fresh_svc();

        // The run gets one real page written before the provider dies, so the
        // abort has actual work to undo.
        let completer = FailsWhenRepliesRunOut {
            replies: Mutex::new(vec![tool_call_reply(
                "kb_write_page",
                serde_json::json!({
                    "path": "knowledge/sources/half-done.md",
                    "content": valid_page("source", "half", "Half."),
                    "commit_message": "half a digest"
                }),
            )]),
        };

        let err = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "Note about HRV.".into(),
                    title: Some("HRV note".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .expect_err("a provider error must fail the digest")
        .to_string();

        assert!(
            err.contains("could not decode"),
            "the provider's own explanation must reach the caller, got: {err}"
        );

        // HEAD is back on main and the transaction branch is gone — otherwise
        // every later write to this KB lands somewhere nobody is looking.
        let repo = git2::Repository::open(dir.path().join("k")).unwrap();
        assert_eq!(
            repo.head().unwrap().shorthand(),
            Some("main"),
            "a failed digest must leave HEAD on main"
        );
        let leftover: Vec<String> = repo
            .branches(Some(git2::BranchType::Local))
            .unwrap()
            .filter_map(|b| b.ok()?.0.name().ok()?.map(str::to_string))
            .filter(|n| n.starts_with("txn/"))
            .collect();
        assert!(
            leftover.is_empty(),
            "the transaction branch must be deleted; found: {leftover:?}"
        );

        // And the half-written page must not be visible in the knowledge base.
        let pages = crate::knowledge::store::list_pages(
            &paths::kb_root(svc.root(), "k"),
            Some("knowledge/"),
        )
        .unwrap();
        assert!(
            pages.is_empty(),
            "an aborted digest must leave no page behind; pages: {pages:?}"
        );
    }

    // -------------------------------------------------------------------------
    // Test 3: dispatch fails with invalid path → KB not corrupted
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn ingest_aborts_txn_when_dispatch_fails() {
        let (_dir, svc) = fresh_svc();

        // Step 0: agent tries to escape the KB via path traversal.
        // The dispatch will return an error; the loop records a ToolResult(ok:false)
        // and pushes an error string back.  Then step 1 finishes with a text reply.
        let completer = MockCompleter::new(vec![
            tool_call_reply(
                "kb_write_page",
                serde_json::json!({
                    "path": "knowledge/../escape.md",
                    "content": valid_page("source", "Escape", "Body."),
                    "commit_message": "escape"
                }),
            ),
            text_reply("recovered"),
        ]);

        // The macro might succeed (agent recovered after the error) or fail — both
        // are valid.  We only assert the KB was not corrupted.
        let kb = svc.root().join("k");
        let _ = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "Some note.".into(),
                    title: Some("y".into()),
                },
                completer: Box::new(completer),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await;

        // The escape path must not exist.
        assert!(
            !kb.join("escape.md").exists(),
            "path traversal must not write outside knowledge/"
        );
        // The git repo must still be openable (not corrupted).
        GitRepo::open(&kb).unwrap();
    }

    // ── Issue #56, Task 10B: CP2 ────────────────────────────────────────────

    /// A completer that never answers. The sub-agent run fails immediately, so
    /// this test says nothing about the macro's happy path and everything about
    /// WHEN the raise runs.
    struct RefusesImmediately;

    #[async_trait]
    impl Completer for RefusesImmediately {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[crate::knowledge::subagent::loop_::LlmMessage],
            _tools: &[Tool],
        ) -> anyhow::Result<LlmReply> {
            anyhow::bail!("no model here")
        }
    }

    #[tokio::test]
    async fn the_ingest_macro_ratchets_before_its_sub_agent_runs() {
        // CP2, and the reason it exists. The sub-agent writes through
        // KbToolDispatch → store::write_page / svc.add_raw_source, which no MCP
        // tool gate can see. This is also the test that makes Task 11's headline
        // test reachable: `conversation_ingest::ingest_conversation` funnels into
        // this function, as do the four HTTP macro routes, the CLI and the probe.
        let (dir, svc) = fresh_svc();
        let root = dir.path().to_path_buf();
        assert!(!crate::knowledge::tier::is_private(&root, "k"));

        let _ = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: true,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "n=412".into(),
                    title: Some("t".into()),
                },
                completer: Box::new(RefusesImmediately),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await; // the sub-agent fails; the raise stands

        assert!(
            crate::knowledge::tier::is_private(&root, "k"),
            "the raise ran after the sub-agent, or not at all"
        );
    }

    #[tokio::test]
    async fn a_public_ingest_never_lowers_a_ratcheted_base() {
        let (dir, svc) = fresh_svc();
        let root = dir.path().to_path_buf();
        crate::knowledge::tier::raise_unlocked(&root, "k", true).unwrap();

        let _ = ingest(
            &svc,
            IngestArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                source: SourceInput::Text {
                    text: "public note".into(),
                    title: Some("t".into()),
                },
                completer: Box::new(RefusesImmediately),
                focus: None,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await;

        assert!(
            crate::knowledge::tier::is_private(&root, "k"),
            "a public ingest lowered the tier"
        );
    }
}
