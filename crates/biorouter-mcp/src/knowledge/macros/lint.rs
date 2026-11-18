//! `lint` macro — deterministic KB hygiene scan + optional sub-agent autofix.
//!
//! Part A: `scan(kb_root)` — pure, synchronous, no LLM.
//! Part B: `lint(svc, args)` — calls `scan`, optionally runs a sub-agent to fix issues.

use crate::knowledge::{
    biookf,
    git::{GitRepo, KnowledgeWriteFailure, Txn},
    graph, manifest, okf, paths, raw,
    service::KnowledgeService,
    source_anchor,
    store::split_frontmatter,
    subagent::{
        events::{DoneReason, SubAgentEvent},
        kb_tools::{self, tool_specs, KbToolAccess, KbToolDispatch},
        loop_::{Completer, SubAgent, SubAgentBounds, SubAgentResult},
        procedures::{lint_procedure, system_prompt},
    },
    types::ChangeKind,
    types::KbFormat,
    validate::{BundlePage, Diagnostic, Diagnostics, Severity},
};
use anyhow::{Context, Result};
use chrono::Utc;
use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
};

// ---------------------------------------------------------------------------
// LintReport
// ---------------------------------------------------------------------------

/// The four hygiene rules that predate the format layers, as stable ids.
///
/// Prefixed `kb.` rather than `okf.` / `biookf.` because they are BioRouter's
/// own housekeeping and not a clause of either spec: no OKF rule says a page
/// must be linked to, and §11 rule 4 explicitly forbids rejecting a bundle for
/// a broken cross-link. Keeping the prefixes apart is what lets a reader — and a
/// Stage 6 UI — tell "your base is untidy" from "this file is not conformant".
pub const RULE_ORPHAN: &str = "kb.orphan";
pub const RULE_CONTRADICTION: &str = "kb.contradiction";
pub const RULE_STALE_SOURCE: &str = "kb.stale_source";
pub const RULE_MISSING_CONCEPT_PAGE: &str = "kb.missing_concept_page";

/// The base has no `knowledge/` directory, so it holds no pages at all.
///
/// A structural finding rather than a hygiene one, and it exists because the
/// alternative was silence: this used to return `LintReport::default()`, which
/// is four empty lists and no diagnostics — byte-identical to the report a
/// *perfect* base produces. The MCP `kb_lint` tool calls [`scan`] directly
/// rather than going through [`lint`], so a model asking about a structurally
/// broken base was told it was healthy.
pub const RULE_MISSING_KNOWLEDGE_DIR: &str = "kb.structure.missing-knowledge-dir";

/// The base's `manifest.yaml` could not be read, so no format layer could run.
///
/// Distinct from a base that is *legitimately* legacy, which correctly gets no
/// format diagnostics; see [`format_diagnostics`].
pub const RULE_UNREADABLE_MANIFEST: &str = "kb.structure.unreadable-manifest";

/// The payload of the lint stream's terminal `event: done` frame, and — since
/// Stage 6 — a published schema.
///
/// It is `ToSchema` rather than merely `Serialize` because
/// `POST /knowledge/bases/{id}/lint` answers over SSE, so the shape a client
/// has to parse cannot be inferred from the response body's own type. Declaring
/// it puts the four lists **and** the typed [`Diagnostics`] into
/// `components.schemas`, and therefore into the generated TypeScript, where the
/// renderer reads the frame. A hand-written interface in `ui/` would be the
/// same contract with nothing keeping it in step.
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LintReport {
    /// Pages nothing else in the bundle links to, in any of the four grammars
    /// (`graph::bundle_links`) — not the legacy bracket form alone, which is
    /// what made this list every page of a typed base.
    pub orphans: Vec<String>,
    /// Pages with `contradiction: true` in frontmatter.
    pub contradictions: Vec<String>,
    /// Sources whose `ingested_at` is >90 days ago AND have no inbound links.
    pub stale_sources: Vec<String>,
    /// Link targets written in source pages that name no page under
    /// `knowledge/` — again over all four grammars, so a typed base's `edges:`
    /// citations are read. Existing, confined raw evidence files are not missing
    /// concept pages, even though the graph represents them as external nodes.
    pub missing_concept_pages: Vec<String>,
    /// Everything above, plus the format layers, as one typed list: a stable
    /// rule id, a severity, the page or edge it is about, and a message.
    ///
    /// ⚠ **It does not replace the four lists, and that is deliberate.** They
    /// are the shape `LINT_PROCEDURE` teaches the autofix sub-agent and the
    /// shape a stored report deserializes into; dropping them would rewrite a
    /// working prompt under a change whose subject is structure. The four
    /// re-appear here as `kb.*` diagnostics, so a consumer reads either and a
    /// new consumer reads only this.
    ///
    /// `serde(default)` so a report written before Stage 4 still loads.
    #[serde(default)]
    pub diagnostics: Diagnostics,
}

// ---------------------------------------------------------------------------
// Part A: deterministic scan
// ---------------------------------------------------------------------------

pub fn scan(kb_root: &Path) -> Result<LintReport> {
    scan_with_cancellation(kb_root, None)
}

pub(crate) fn scan_with_cancellation(
    kb_root: &Path,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<LintReport> {
    super::ensure_not_cancelled(cancel, "the lint scan")?;
    if !kb_root.join("knowledge").exists() {
        // Reported, not returned empty: see `RULE_MISSING_KNOWLEDGE_DIR`.
        return Ok(LintReport {
            diagnostics: Diagnostics::new(vec![Diagnostic::scan(
                RULE_MISSING_KNOWLEDGE_DIR,
                Severity::Error,
                "knowledge/",
                "the base has no `knowledge/` directory, so it holds no pages; \
                 restore it from git history or re-create the base",
            )]),
            ..LintReport::default()
        });
    }

    // Collect all pages and their bodies — for the checks that read a page's own
    // frontmatter and text. The *links* come from the deriver; see below.
    let mut pages: HashMap<String, String> = HashMap::new(); // logical_path -> body
    for page in crate::knowledge::store::list_pages(kb_root, None)? {
        super::ensure_not_cancelled(cancel, "reading lint pages")?;
        let path = crate::knowledge::store::resolve_readable_path(kb_root, &page.path)?;
        pages.insert(page.path, std::fs::read_to_string(path)?);
    }

    // Both link-shaped rules — "does anything link to this page?" and "does this
    // target name a page?" — are answered off the graph deriver's own edge set
    // (`graph::bundle_links`), which reads all four grammars and resolves through
    // DR-3's identity ladder.
    //
    // Lint used to walk the tree itself for `okf::links`' legacy bracket form
    // alone. That is exactly right for a legacy base and blind on a typed one,
    // where relationships live in BioOKF §6's `edges:` frontmatter array: a
    // BioOKF base of 11 pages and 17 typed edges reported **all 11 as orphans**
    // while the subject band beside it said "17 links" — lint sending the user to
    // fix every page of a base that was fine, and contradicting the graph drawn
    // next to it. Reading the deriver's answer rather than re-deriving one is
    // what makes a fifth grammar impossible to add to one and not the other.
    super::ensure_not_cancelled(cancel, "deriving lint links")?;
    let links = graph::bundle_links(kb_root)?;
    super::ensure_not_cancelled(cancel, "processing lint links")?;
    let inbound = &links.inbound;

    // ---- Orphans: pages with no inbound links, excluding hub pages ----
    // Hub pages = any page directly under knowledge/ (not in a subdirectory).
    let orphans: Vec<String> = inbound
        .iter()
        .filter(|(path, sources)| sources.is_empty() && !is_hub_page(path))
        .map(|(path, _)| path.clone())
        .collect();

    // ---- Contradictions: pages with frontmatter contradiction: true ----
    let contradictions: Vec<String> = pages
        .iter()
        .filter(|(_, body)| {
            let (fm, _) = split_frontmatter(body);
            fm.get("contradiction")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .map(|(path, _)| path.clone())
        .collect();

    // Which pages are source pages, and which raw source each one is for. Both
    // rules below used to answer that from the `knowledge/sources/` path alone,
    // which is a layout no base created by this build uses — see
    // [`SourcePages`].
    let sources = SourcePages::of(&pages);

    // ---- Stale sources: raw/ sources >90 days without inbound links ----
    let ninety_days = chrono::Duration::days(90);
    let now = Utc::now();
    let mut stale_sources: Vec<String> = Vec::new();
    if let Ok(metas) = raw::list_sources(kb_root) {
        for meta in metas {
            super::ensure_not_cancelled(cancel, "scanning lint sources")?;
            let age = now.signed_duration_since(meta.ingested_at);
            if age > ninety_days {
                // Does anything link to the page (or pages) that stand for this
                // raw source? A source cited by half the base is not stale
                // however long ago it was ingested, and this rule reported
                // exactly that as stale on every OKF and BioOKF base.
                let has_inbound = sources
                    .pages_for(&meta.id)
                    .iter()
                    .any(|path| inbound.get(path).is_some_and(|s| !s.is_empty()));
                if !has_inbound {
                    stale_sources.push(meta.id.clone());
                }
            }
        }
    }

    // ---- Missing concept pages: link targets written in source pages that name
    //      no page under knowledge/ ----
    //
    // The same widening, for the same reason: on a typed base the citations a
    // source page makes are `edges:` entries, so a bracket-only reader found
    // none of them and this list was empty on exactly the bases the migration
    // produces. `unresolved` is the deriver's own set of non-resolutions, which
    // is why a target the graph draws an edge to can no longer appear here.
    let mut missing_concept_pages: Vec<String> = Vec::new();
    for (src_path, target) in &links.unresolved {
        super::ensure_not_cancelled(cancel, "scanning unresolved lint links")?;
        if sources.is_source_page(src_path)
            && !missing_concept_pages.contains(target)
            && !is_existing_raw_evidence(kb_root, src_path, target)
        {
            missing_concept_pages.push(target.clone());
        }
    }

    let mut report = LintReport {
        orphans,
        contradictions,
        stale_sources,
        missing_concept_pages,
        diagnostics: Diagnostics::default(),
    };
    // Three of the four are collected out of a `HashMap`/`HashSet`, so before
    // this line the same base lint into a different order on every run. It was
    // invisible while the report was four bags of strings a human skimmed; it
    // stops being invisible the moment the entries are diagnostics, because a
    // diff of two reports becomes noise and the `MAX_DIAGNOSTICS` cut picks a
    // different subset each time. Sorting here rather than at each collection
    // site keeps it one statement about the whole report.
    report.orphans.sort();
    report.contradictions.sort();
    report.stale_sources.sort();
    report.missing_concept_pages.sort();
    super::ensure_not_cancelled(cancel, "lint format diagnostics")?;
    report.diagnostics = Diagnostics::new(
        scan_diagnostics(&report)
            .chain(format_diagnostics(kb_root, &pages))
            .collect(),
    );
    super::ensure_not_cancelled(cancel, "finishing the lint scan")?;
    Ok(report)
}

fn is_existing_raw_evidence(kb_root: &Path, source: &str, target: &str) -> bool {
    let target = target.split('#').next().unwrap_or(target);
    let rooted = target.strip_prefix('/').unwrap_or(target);
    let candidate = if rooted.starts_with("raw/") {
        PathBuf::from(rooted)
    } else {
        Path::new(source)
            .parent()
            .unwrap_or(Path::new(""))
            .join(target)
    };
    let mut logical = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => logical.push(part),
            Component::CurDir => {}
            Component::ParentDir if logical.pop() => {}
            _ => return false,
        }
    }
    if !logical.starts_with("raw") {
        return false;
    }
    // ⚠ Join with '/', do NOT hand over `logical.to_str()`.
    //
    // `resolve_readable_path` takes a KB *logical* path — the slash-separated
    // vocabulary every other KB surface speaks (link targets, `kb_write_page`
    // arguments, the manifest) — and gates on a literal `"raw/"` prefix. The
    // value here is an OS `PathBuf`, so on Windows `to_str()` yields
    // `raw\evidence\source.md`, which fails that prefix test and made the
    // resolver report every existing raw file as absent. The visible symptom was
    // nothing to do with paths: a source page's links to evidence that is really
    // there were listed as `missing_concept_pages`, on Windows only.
    //
    // Every component is `Normal` by the time the loop above finishes, so the
    // join is exact rather than a normalisation guess.
    let logical: Option<String> = logical
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join("/"));
    logical.is_some_and(|logical| {
        crate::knowledge::store::resolve_readable_path(kb_root, &logical)
            .is_ok_and(|path| path.is_file())
    })
}

/// The four deterministic lists, restated as typed diagnostics.
///
/// Built from the report rather than from the loops that produced it, so the two
/// halves cannot disagree about what was found: there is one place a page
/// becomes an orphan, and this reads its answer.
fn scan_diagnostics(report: &LintReport) -> impl Iterator<Item = Diagnostic> + '_ {
    let rule = |rule: &'static str, severity, message: &'static str| {
        move |subject: &String| Diagnostic::scan(rule, severity, subject.clone(), message)
    };
    report
        .orphans
        .iter()
        .map(rule(
            RULE_ORPHAN,
            Severity::Warning,
            "no other page links to this one; link it from a hub page or remove it",
        ))
        .chain(report.contradictions.iter().map(rule(
            RULE_CONTRADICTION,
            Severity::Warning,
            "the page declares `contradiction: true`; resolve it or record which source wins",
        )))
        .chain(report.stale_sources.iter().map(rule(
            RULE_STALE_SOURCE,
            Severity::Info,
            "ingested over 90 days ago and still unreferenced by any page",
        )))
        .chain(report.missing_concept_pages.iter().map(rule(
            RULE_MISSING_CONCEPT_PAGE,
            Severity::Warning,
            "a source page links to this target, but no page under knowledge/ carries it",
        )))
}

/// The OKF layer over every page, plus the BioOKF profile when the base is in
/// it.
///
/// **A legacy base gets nothing** (DR-26). Its pages are `title`/`kind`
/// frontmatter and `[[wiki]]` links, which this build has promised never to
/// rewrite — so `okf.type.missing` on every one of them would report a decision
/// as several hundred defects and bury the four hygiene findings that are real.
/// `Manifest::profile` is the accessor that answers this and `Manifest::format`
/// is the trap: it reads `Okf` on every base written before Stage 3.
///
/// An unreadable manifest gets no format layer either, for the same reason:
/// guessing `Okf` for a base whose generation we could not establish produces
/// exactly the flood the paragraph above describes. It does **not** get silence,
/// though — that is the difference between the two cases. A legacy base is
/// working as designed; a base whose manifest will not parse is damaged in a way
/// that costs the user their `.active-kb` pointers (DR-12), and reporting it as
/// clean is how nobody finds out.
fn format_diagnostics(kb_root: &Path, pages: &HashMap<String, String>) -> Vec<Diagnostic> {
    let manifest = match manifest::load(kb_root) {
        Ok(manifest) => manifest,
        Err(error) => {
            return vec![Diagnostic::scan(
                RULE_UNREADABLE_MANIFEST,
                Severity::Error,
                "manifest.yaml",
                &format!("the base's manifest could not be read, so no conformance check could run: {error:#}"),
            )]
        }
    };
    let Some(format) = manifest.profile() else {
        return Vec::new();
    };
    let mut checked: Vec<(&String, &String, Option<okf::Page>)> = pages
        .iter()
        .filter(|(path, _)| !graph::is_scaffold_page(path))
        .map(|(path, text)| (path, text, okf::Page::parse(text).ok()))
        .collect();
    // `HashMap` iteration order is random, so without this the same base lints
    // into a different order on every run — which turns a diff of two reports
    // into noise and makes the `MAX_DIAGNOSTICS` cut pick a different subset
    // each time.
    checked.sort_by(|a, b| a.0.cmp(b.0));

    let subject_of = |path: &str, page: &Option<okf::Page>| {
        page.as_ref()
            .and_then(|p| p.doc.primary_key())
            .unwrap_or(path)
            .to_string()
    };
    let mut out: Vec<Diagnostic> = Vec::new();
    for (path, text, page) in &checked {
        let subject = subject_of(path, page);
        out.extend(
            okf::check_source(text)
                .into_iter()
                .map(|d| Diagnostic::from_okf_at(d, &subject, Some(path))),
        );
    }
    if format == KbFormat::Biookf {
        // One index for the whole run, which is what makes the cross-document
        // rules (duplicate `identifier`, unresolved `object` / `primary_source`)
        // answerable at all — and O(bundle) once rather than per page.
        let bundle: Vec<BundlePage> = checked
            .iter()
            .filter_map(|(path, _, page)| {
                page.as_ref().map(|p| BundlePage {
                    path: (*path).clone(),
                    doc: p.doc.clone(),
                })
            })
            .collect();
        let index = biookf::BundleIndex::build(bundle.iter().map(|p| (p.path.as_str(), &p.doc)));
        for p in &bundle {
            out.extend(
                biookf::check_doc(Some(&p.path), &p.doc, &index)
                    .findings
                    .into_iter()
                    .map(Diagnostic::from),
            );
        }
        // §10's provenance-quality pass, and the only rule family that reads
        // anything outside the pages above: a source node's credibility verdict
        // lives in `raw/<id>/meta.yaml`. Until this line the classifier's
        // `retracted` flag was written at ingest and read by nothing, so a base
        // could cite a retracted paper as the `primary_source` of a clinical
        // claim and lint clean.
        out.extend(
            biookf::check_credibility(kb_root, bundle.iter().map(|p| (p.path.as_str(), &p.doc)))
                .into_iter()
                .map(Diagnostic::from),
        );
    }
    out
}

// ---- helpers ----------------------------------------------------------------

/// Which pages in a bundle are **source pages**, and which raw source each one
/// stands for — one index, built once per scan, over
/// [`source_anchor`](crate::knowledge::source_anchor).
///
/// ⚠ **The recognition itself is NOT here.** It lives in `source_anchor`
/// because `graph::apply_source_credibility` asks the same question and has to
/// get the same answer: the two used to answer it from two copies of the literal
/// string `knowledge/sources/`, and when the first repair fixed one copy the
/// other kept silently attaching no credibility at all. That module's header
/// carries the full signal table and the reasoning; what is left here is the
/// bundle-shaped index the two lint rules read.
///
/// The rules this feeds, and how each one failed while the recognition was a
/// path prefix:
///
/// * `stale_sources` looked up inbound links at a path that does not exist, got
///   "no such key" and read it as "nothing links here", so **every** raw source
///   older than 90 days was reported stale no matter how heavily cited — a
///   hygiene report telling the user to prune the papers their base is built on.
/// * `missing_concept_pages` filtered its candidates on the same prefix, matched
///   nothing, and was therefore unconditionally empty — which reads exactly like
///   a clean base, because a rule that fires on nothing looks identical to a rule
///   with nothing to report.
struct SourcePages {
    /// raw source id → the logical paths of the pages that stand for it.
    ///
    /// A `Vec` rather than one path, because two pages may legitimately stand
    /// for one ingested document (a publication node and a study node over the
    /// same PDF). Picking one of them arbitrarily would make "is this source
    /// cited?" depend on which page a `HashMap` happened to hand back first.
    by_raw_id: HashMap<String, Vec<String>>,
    /// Every page that is a source page at all, by any of `source_anchor`'s
    /// signals.
    paths: HashSet<String>,
}

impl SourcePages {
    fn of(pages: &HashMap<String, String>) -> Self {
        let mut by_raw_id: HashMap<String, Vec<String>> = HashMap::new();
        let mut paths: HashSet<String> = HashSet::new();
        // Sorted, because `HashMap` iteration order is random and the `Vec`s
        // below become part of a report the user diffs against the last one.
        let mut sorted: Vec<(&String, &String)> = pages.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(b.0));
        for (path, body) in sorted {
            let Ok(page) = okf::Page::parse(body) else {
                // DR-7: an unparseable page is not a reason to fail the scan, and
                // the OKF layer reports the parse failure itself. It can still be
                // a source page by its path, which is the one signal that needs
                // no frontmatter — and on a pre-OKF base that is the only signal
                // there was ever going to be.
                if source_anchor::is_source_dir(path) {
                    paths.insert(path.clone());
                }
                continue;
            };
            if source_anchor::is_source_page(path, &page.doc) {
                paths.insert(path.clone());
            }
            for id in source_anchor::raw_ids_stood_for(path, &page.doc) {
                by_raw_id.entry(id).or_default().push(path.clone());
            }
        }
        Self { by_raw_id, paths }
    }

    /// The pages standing for one raw source, or — for a base whose source pages
    /// carry no anchor at all — the pre-OKF path it would have lived at.
    ///
    /// The fallback is `or_else` rather than a union, matching
    /// `apply_source_credibility`: a base has one layout or the other, and
    /// checking both would let a legacy path that happens to exist speak for a
    /// source some *other* page already stands for.
    fn pages_for(&self, raw_id: &str) -> Vec<String> {
        self.by_raw_id.get(raw_id).cloned().unwrap_or_else(|| {
            // ⚠ BOTH spellings. OKF's source directory is the SINGULAR
            // `knowledge/source/`; only the pre-OKF layout used the plural. This
            // was plural-only, so on the format this build actually creates a
            // source page with no `raw_source` anchor matched nothing — and
            // `schema_okf.md` only asks the model to "create the source's own
            // page", so a page without that anchor is ordinary, not malformed.
            crate::knowledge::graph::source_page_candidates(raw_id).to_vec()
        })
    }

    fn is_source_page(&self, path: &str) -> bool {
        self.paths.contains(path)
    }
}

/// Returns true for pages directly under `knowledge/` (no subdirectory).
fn is_hub_page(path: &str) -> bool {
    // "knowledge/index.md" has exactly one '/' after "knowledge/".
    let rest = path.strip_prefix("knowledge/").unwrap_or(path);
    !rest.contains('/')
}

// `resolve_wiki_link` lived here: a case-insensitive stem comparison with
// spaces replaced by hyphens, which could not read a path-style target at all.
// It became `knowledge::links::LinkIndex::resolve` — the graph's resolver, the
// most complete of the three and the only one that had tests — and is now
// `graph::bundle_links`, the deriver's whole answer rather than one rung of it.
// The intermediate step fixed *how* a target resolved and left *which* links
// were read alone, which is why a typed base still linted as all orphans.

// ---------------------------------------------------------------------------
// Part B: async lint (with optional sub-agent autofix)
// ---------------------------------------------------------------------------

pub struct LintArgs {
    pub kb_id: String,
    /// The capability of the model this macro will run (issue #56). Required,
    /// so every production caller is a compile error rather than an omission.
    pub caller_is_private: bool,
    /// Whose agreements cover that model — DR-26's third axis (issue #56, Task
    /// 50). Required for the same reason `caller_is_private` is: an omission
    /// must be a compile error. `Unstated` is its `Default`, so a caller that
    /// cannot determine one fails closed.
    pub caller_affiliation: crate::knowledge::affiliation::CallerAffiliation,
    pub completer: Option<Box<dyn Completer>>,
    pub autofix: bool,
    pub bounds: SubAgentBounds,
    /// Optional channel for live event streaming. When provided, every
    /// `SubAgentEvent` is sent here as soon as it is produced (not just at
    /// the end of the run). Set to `None` if streaming is not needed.
    pub event_sink: Option<tokio::sync::mpsc::UnboundedSender<SubAgentEvent>>,
    /// Optional level-triggered cancellation shared with the request or turn
    /// that owns this macro run.
    pub cancel: Option<tokio_util::sync::CancellationToken>,
}

/// What the lint stream's terminal `event: done` frame actually carries.
///
/// The [`LintReport`] is nested rather than flattened because a lint answers two
/// questions — what is wrong, and what was changed about it — and an autofix
/// that rewrote pages has a `commit_sha` the report itself cannot express.
/// Published as a schema for the reason its `report` field is; see
/// [`LintReport`].
#[cfg_attr(feature = "utoipa", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LintResult {
    /// What is wrong with the base as it stands when the lint ends. After an
    /// autofix that committed, this is a fresh scan of the committed base, so
    /// it lists what the fix left and never a finding it fixed. Otherwise it is
    /// the one scan the run made.
    pub report: LintReport,
    /// The autofix commit, or `None` for a read-only lint — which is every lint
    /// that ran without `autofix`, and therefore without a provider at all.
    pub commit_sha: Option<String>,
    /// How many page writes the autofix committed: every successful
    /// `kb_write_page` and `kb_write_concept` call. Zero for a read-only lint
    /// and for an autofix that changed no page.
    pub fixes_applied: usize,
}

pub async fn lint(svc: &KnowledgeService, args: LintArgs) -> Result<LintResult> {
    let cancel = args.cancel.clone();
    // A lint without `autofix` writes nothing, so it takes the READ deadline.
    // The GUI never sends `autofix` (the Knowledge view's "Check for problems"
    // posts `{ model }` only), so this IS the common path — and on the write
    // deadline it parked for thirty minutes behind an open transaction while
    // `kb_lint` on the same base refused in forty-five seconds. Two surfaces
    // disagreeing about one base is what makes it a bug rather than a delay.
    let lock = if args.autofix {
        svc.lock_kb_cancellable(&args.kb_id, cancel.as_ref())
            .await?
    } else {
        svc.lock_kb_cancellable_for_read(&args.kb_id, cancel.as_ref())
            .await?
    };
    super::ensure_not_cancelled(cancel.as_ref(), "lint preflight")?;
    // Issue #56. Before the sub-agent, not after: an autofix that fails halfway
    // has already written pages. Task 10C (CP2) puts the barrier on the line
    // above — a lint's `scan` reads every page, and an autofix rewrites them.
    crate::knowledge::tier::assert_reachable(
        svc.root(),
        &args.kb_id,
        args.caller_is_private,
        &args.caller_affiliation,
    )?;
    let format = svc.require_current_profile(&args.kb_id)?;
    let kb_root = paths::kb_root(svc.root(), &args.kb_id);

    // Migrate a stale `schema.md` (the sub-agent's system prompt) and refresh a
    // stale graph cache, neither fatally. See `macros::refresh_base`.
    super::ensure_not_cancelled(cancel.as_ref(), "the lint refresh")?;
    super::refresh_base(svc, &args.kb_id).await?;

    super::ensure_not_cancelled(cancel.as_ref(), "the lint scan")?;
    let scan_root = kb_root.clone();
    let scan_cancel = cancel.clone();
    let (_lock, report) = tokio::task::spawn_blocking(move || {
        let report = scan_with_cancellation(&scan_root, scan_cancel.as_ref())?;
        Ok::<_, anyhow::Error>((lock, report))
    })
    .await
    .map_err(|error| anyhow::anyhow!("knowledge lint scan task failed: {error}"))??;
    super::ensure_not_cancelled(cancel.as_ref(), "returning the lint report")?;

    if !args.autofix {
        // ⚠ RETURN BEFORE THE RATCHET, and that ordering is the whole point.
        //
        // The raise used to sit at this macro's entry, above this return, so a
        // scan that writes nothing still stamped the base. That is the exact
        // thing `server.rs` refuses to do — "a preview writes nothing and must
        // not raise a base's tier permanently because a private chat *looked*"
        // — which is why the MCP `kb_lint` calls `scan` directly rather than
        // coming through here. It went unnoticed because the HTTP and CLI
        // surfaces used to hand this macro a hardcoded `ProviderTier::Public`,
        // making the raise a no-op; the moment they started passing the
        // caller's real tier, a read-only lint from any private chat began
        // permanently privatising whatever public base it was pointed at, and
        // stamping that base with the caller's institution — an owner nothing
        // but deleting the base can remove.
        return Ok(LintResult {
            report,
            commit_sha: None,
            fixes_applied: 0,
        });
    }

    run_autofix(svc, args, &kb_root, report, format, cancel.as_ref()).await
}

async fn run_autofix(
    svc: &KnowledgeService,
    args: LintArgs,
    kb_root: &Path,
    report: LintReport,
    format: KbFormat,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<LintResult> {
    // Issue #56, both axes in one call under one lock — see
    // `KnowledgeService::raise_tier_and_affiliation` for why they cannot be
    // two. It belongs HERE, on the writing half: `assert_reachable` above has
    // already decided whether this caller may READ the base, and only an
    // autofix changes it.
    super::ensure_not_cancelled(cancel, "the lint privacy ratchet")?;
    svc.raise_tier_and_affiliation_cancelled_by(
        &args.kb_id,
        args.caller_is_private,
        &args.caller_affiliation,
        cancel,
    )
    .await?;

    let completer = args
        .completer
        .ok_or_else(|| anyhow::anyhow!("completer required for autofix"))?;

    super::ensure_not_cancelled(cancel, "the lint transaction")?;
    let repo = GitRepo::open(kb_root)?;
    let txn = repo.begin_txn("lint")?;

    let prompt = match prepare_autofix_prompt(kb_root, &report, format) {
        Ok(prompt) => prompt,
        Err(error) => return Err(repo.abort_after_failure(&txn, "lint autofix", error)),
    };

    let dispatch = KbToolDispatch {
        svc: svc.clone(),
        kb_id: args.kb_id.clone(),
        txn_branch: txn.branch.clone(),
        access: KbToolAccess::ReadWrite,
    };
    let agent = SubAgent {
        completer,
        tools: tool_specs(Some(prompt.format)),
        system_prompt: prompt.system,
        bounds: args.bounds,
    };

    let agent_result = agent
        .run(
            &prompt.user,
            std::sync::Arc::new(dispatch),
            cancel,
            args.event_sink.as_ref(),
        )
        .await;

    settle_autofix(svc, &args.kb_id, &repo, &txn, report, agent_result, cancel)
}

struct AutofixPrompt {
    format: KbFormat,
    system: String,
    user: String,
}

fn prepare_autofix_prompt(
    kb_root: &Path,
    report: &LintReport,
    format: KbFormat,
) -> Result<AutofixPrompt> {
    let schema_path = crate::knowledge::store::resolve_readable_path(kb_root, "schema.md")
        .context("resolving schema.md for lint")?;
    let schema = std::fs::read_to_string(schema_path).context("read schema.md")?;
    // The four lists are what `LINT_PROCEDURE` teaches, so they stay exactly as
    // they were. `diagnostics` is added beside them rather than instead of them,
    // and is already capped at `MAX_DIAGNOSTICS` — an uncapped list over a large
    // base is a context-window failure that presents as the model losing track.
    let report_json = serde_json::to_string_pretty(&serde_json::json!({
        "orphans": report.orphans,
        "contradictions": report.contradictions,
        "stale_sources": report.stale_sources,
        "missing_concept_pages": report.missing_concept_pages,
        "diagnostics": report.diagnostics,
    }))?;
    let system = system_prompt(&schema, lint_procedure(Some(format)));
    let user = format!(
        "autofix=true. Here is the current lint report:\n```json\n{report_json}\n```\nPlease fix the issues."
    );

    Ok(AutofixPrompt {
        format,
        system,
        user,
    })
}

/// The autofix transaction's three endings — commit, abort-and-report,
/// abort-and-fail — split out of [`lint`] so that function stays under
/// `clippy::too_many_lines` (issue #56, review round 5). No behaviour change.
fn settle_autofix(
    svc: &KnowledgeService,
    kb_id: &str,
    repo: &GitRepo,
    txn: &Txn,
    report: LintReport,
    agent_result: Result<SubAgentResult>,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<LintResult> {
    match agent_result {
        Ok(r)
            if matches!(
                r.reason,
                DoneReason::CompleteSentinel | DoneReason::NoMoreToolCalls
            ) =>
        {
            if let Err(error) = super::ensure_not_cancelled(cancel, "the lint commit") {
                return Err(repo.abort_after_failure(txn, "lint autofix", error));
            }
            // An autofix run that fixed nothing must not commit. `commit_txn`
            // squash-commits the transaction *tree* and git records a commit
            // whose tree equals its parent's without complaint, so every clean
            // KB used to gain an empty `[lint] lint autofix` entry — visible in
            // the Knowledge change-log drawer as work that never happened, and
            // a `commit_sha` callers read as proof of it. Same false success as
            // issue #71, one macro over.
            let fixed = match repo.txn_wrote_knowledge_pages(txn) {
                Ok(fixed) => fixed,
                Err(e) => {
                    return Err(repo.abort_after_failure(
                        txn,
                        "lint autofix",
                        e.context("checking whether the lint fixed anything"),
                    ));
                }
            };
            if !fixed {
                repo.abort_txn(txn).map_err(|abort_error| {
                    KnowledgeWriteFailure::outcome_uncertain(
                        "lint autofix",
                        anyhow::anyhow!(
                            "the lint changed no page, but discarding its transaction failed: {abort_error:#}"
                        ),
                    )
                })?;
                // Nothing landed, so nothing was applied, and the scan the run
                // started from still describes the base.
                return Ok(LintResult {
                    report,
                    commit_sha: None,
                    fixes_applied: 0,
                });
            }
            let fixes_applied = page_writes(&r.events);
            let sha = repo
                .commit_txn(txn, ChangeKind::Lint, "lint autofix", None)
                .map_err(|error| {
                    KnowledgeWriteFailure::outcome_uncertain(
                        "lint autofix",
                        error.context("committing lint fixes"),
                    )
                })?;
            if let Err(error) = svc.rebuild_graph_cache(kb_id) {
                return Err(KnowledgeWriteFailure::committed(
                    "lint autofix",
                    sha.clone(),
                    error.context("rebuilding the graph cache after lint committed"),
                )
                .into());
            }
            let report = rescan_after_commit(svc, kb_id, &sha, cancel)?;
            Ok(LintResult {
                report,
                commit_sha: Some(sha),
                fixes_applied,
            })
        }
        Ok(r) => Err(repo.abort_after_failure(
            txn,
            "lint autofix",
            anyhow::anyhow!(
                "lint sub-agent aborted: reason={:?}, final={}",
                r.reason,
                r.final_text
            ),
        )),
        Err(e) => Err(repo.abort_after_failure(txn, "lint autofix", e)),
    }
}

/// The successful page writes in a sub-agent run, which is what an autofix
/// reports as `fixes_applied`. Which tools write a page is
/// [`kb_tools::writes_a_page`]'s answer, not a name spelled here.
fn page_writes(events: &[SubAgentEvent]) -> usize {
    events
        .iter()
        .filter(|e| {
            matches!(e, SubAgentEvent::ToolResult { name, ok: true, .. }
                if kb_tools::writes_a_page(name))
        })
        .count()
}

/// Scan the base again once the autofix is committed, with the scan the lint
/// itself ran, so the result reports what remains (rq2-027).
///
/// The report handed to the sub-agent describes the base before the fix, and
/// returning it as the result told the person every finding was still there
/// while the commit said otherwise. A scan that fails here fails after the
/// commit, so it is reported as committed work, sha included, rather than as a
/// lint that changed nothing.
fn rescan_after_commit(
    svc: &KnowledgeService,
    kb_id: &str,
    sha: &str,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<LintReport> {
    scan_with_cancellation(&paths::kb_root(svc.root(), kb_id), cancel).map_err(|error| {
        KnowledgeWriteFailure::committed(
            "lint autofix",
            sha,
            error.context("re-scanning the base after lint committed"),
        )
        .into()
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{
        affiliation::CallerAffiliation, page_fixtures::valid_page, service::KnowledgeService,
        store::write_page,
    };

    fn fresh_svc() -> (tempfile::TempDir, KnowledgeService) {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base("k", "K", None).unwrap();
        (dir, svc)
    }

    /// A base in a named profile, created through the production path so its
    /// `schema.md`, its scaffold directories and its `schema_version` are the
    /// real ones and not a hand-built approximation.
    fn svc_in(format: KbFormat) -> (tempfile::TempDir, KnowledgeService) {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base_as("k", "K", None, format, false, &CallerAffiliation::Unstated)
            .unwrap();
        (dir, svc)
    }

    /// A base written before the OKF generation: the shape every base on disk
    /// has today. `schema_version` is what separates it, not `format` — see
    /// `Manifest::profile`.
    fn make_legacy(svc: &KnowledgeService) {
        let kb = svc.root().join("k");
        let mut m = manifest::load(&kb).unwrap();
        m.schema_version = 1;
        manifest::save(&kb, &m).unwrap();
    }

    fn rules(report: &LintReport) -> Vec<&str> {
        report
            .diagnostics
            .items
            .iter()
            .map(|d| d.rule.as_str())
            .collect()
    }

    // ---- Stage 4: typed diagnostics ----------------------------------------

    /// The structure is additive. The four lists keep their contents and their
    /// meaning, and each entry re-appears as a `kb.*` diagnostic carrying the
    /// same subject — so a consumer reads either and neither can drift.
    #[test]
    fn the_four_lists_survive_and_re_appear_as_typed_diagnostics() {
        let (_dir, svc) = fresh_svc();
        let kb = svc.root().join("k");
        write_page(
            &kb,
            "knowledge/entities/c.md",
            "---\ntitle: C\nkind: entity\ncontradiction: true\n---\n\nNothing links here.",
            "add c",
            None,
        )
        .unwrap();

        let report = scan(&kb).unwrap();
        assert_eq!(report.orphans, vec!["knowledge/entities/c.md"]);
        assert_eq!(report.contradictions, vec!["knowledge/entities/c.md"]);

        for rule in [RULE_ORPHAN, RULE_CONTRADICTION] {
            let found = report
                .diagnostics
                .items
                .iter()
                .find(|d| d.rule == rule)
                .unwrap_or_else(|| panic!("{rule} missing from {:?}", rules(&report)));
            assert_eq!(found.subject, "knowledge/entities/c.md");
            assert_eq!(found.path.as_deref(), Some("knowledge/entities/c.md"));
            assert!(!found.message.is_empty());
        }
    }

    /// DR-26. A legacy base is read through its own generation's path and never
    /// rewritten, so it gets the four hygiene rules and **no format layer** —
    /// otherwise every one of its `title`/`kind` pages reports `okf.type.missing`
    /// and the findings that are real are buried under a decision.
    #[test]
    fn a_legacy_base_is_linted_for_hygiene_and_not_for_conformance() {
        let (_dir, svc) = fresh_svc();
        make_legacy(&svc);
        let kb = svc.root().join("k");
        write_page(
            &kb,
            "knowledge/entities/c.md",
            "---\ntitle: C\nkind: entity\n---\n\nNothing links here.",
            "add c",
            None,
        )
        .unwrap();

        let report = scan(&kb).unwrap();
        assert_eq!(rules(&report), vec![RULE_ORPHAN]);
        assert!(
            !rules(&report).iter().any(|r| r.starts_with("okf.")),
            "a legacy base was checked against OKF: {:?}",
            rules(&report)
        );
    }

    /// A base with no `knowledge/` directory holds no pages at all, and used to
    /// return `LintReport::default()` — four empty lists and no diagnostics,
    /// byte-identical to the report a *perfect* base produces. The MCP
    /// `kb_lint` tool calls `scan` directly, so the model asking about a
    /// structurally broken base was told it was healthy.
    #[test]
    fn a_base_with_no_knowledge_directory_lints_as_an_error_not_as_perfect() {
        let (_dir, svc) = fresh_svc();
        let kb = svc.root().join("k");
        std::fs::remove_dir_all(kb.join("knowledge")).unwrap();

        let report = scan(&kb).unwrap();

        assert!(
            report.diagnostics.has(RULE_MISSING_KNOWLEDGE_DIR),
            "{:?}",
            rules(&report)
        );
        assert_eq!(report.diagnostics.errors(), 1);
        assert_ne!(
            report.diagnostics,
            LintReport::default().diagnostics,
            "a broken base must not be indistinguishable from a clean one"
        );
    }

    /// D8's half of the same class. A base whose `manifest.yaml` will not parse
    /// is damaged in a way that costs the user their `.active-kb` pointers
    /// (DR-12); it must not be filed under the same silence as a base that is
    /// legitimately legacy.
    #[test]
    fn an_unreadable_manifest_is_reported_rather_than_read_as_legacy() {
        let (_dir, svc) = svc_in(KbFormat::Okf);
        let kb = svc.root().join("k");
        std::fs::write(manifest::manifest_path(&kb), "id: [unclosed\n").unwrap();
        assert!(manifest::load(&kb).is_err(), "the fixture must be broken");

        let report = scan(&kb).unwrap();

        let found = report
            .diagnostics
            .items
            .iter()
            .find(|d| d.rule == RULE_UNREADABLE_MANIFEST)
            .unwrap_or_else(|| panic!("missing from {:?}", rules(&report)));
        assert_eq!(found.severity, Severity::Error);
        assert_eq!(found.subject, "manifest.yaml");
    }

    /// The same page in an OKF base **is** checked, which is what makes the
    /// silence above the profile answering rather than the checker doing
    /// nothing.
    #[test]
    fn an_okf_base_reports_the_format_layer_too() {
        let (_dir, svc) = svc_in(KbFormat::Okf);
        let kb = svc.root().join("k");
        write_page(
            &kb,
            "knowledge/concept/c.md",
            "---\ntitle: C\nkind: entity\n---\n\nNothing links here.",
            "add c",
            None,
        )
        .unwrap();

        let report = scan(&kb).unwrap();
        assert!(
            report
                .diagnostics
                .has(crate::knowledge::okf::conformance::RULE_TYPE_MISSING),
            "{:?}",
            rules(&report)
        );
    }

    /// DR-24, end to end and at the bundle level: a base an ingest produced —
    /// materialised source node, concept pages citing it, `reported_in` edges
    /// back to it — lints with **no** unresolved provenance, and the source
    /// node's self-citing `reported_in` is not reported as a dangling source.
    ///
    /// The self-reference is the part that needs an assertion. It is the
    /// intended terminating base case (SPEC §8.1: a source attests its own
    /// contents), and a lint that treated it as an ordinary citation would flag
    /// **every source in every base** — a finding per source, on the one page
    /// that can never be fixed, drowning the findings that are real.
    #[test]
    fn a_bundle_with_materialized_sources_has_no_unresolved_provenance() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write_page(
            &kb,
            "knowledge/publication/chen-2020.md",
            "---\ntype: Publication\nidentifier: Chen 2020\nxref: [PMID:32504360]\n\
             raw_source: [raw/chen-2020/source.md]\nedges:\n  - predicate: reported_in\n    \
             object: Chen 2020\n    knowledge_level: knowledge_assertion\n    \
             agent_type: automated_agent\n    primary_source: Chen 2020\n---\n\n# Chen 2020\n",
            "add source node",
            None,
        )
        .unwrap();
        write_page(
            &kb,
            "knowledge/disease/covid-19.md",
            "---\ntype: Disease\nidentifier: COVID-19\nedges:\n  - predicate: reported_in\n    \
             object: Chen 2020\n    knowledge_level: knowledge_assertion\n    \
             agent_type: text_mining_agent\n    primary_source: Chen 2020\n---\n\n# COVID-19\n",
            "add disease",
            None,
        )
        .unwrap();
        write_page(
            &kb,
            "knowledge/molecule/interleukin-6.md",
            "---\ntype: Molecule\nidentifier: Interleukin-6\nedges:\n  \
             - predicate: associated_with\n    object: COVID-19\n    \
             knowledge_level: statistical_association\n    agent_type: text_mining_agent\n    \
             primary_source: Chen 2020\n    p_value: 3.0e-6\n  - predicate: reported_in\n    \
             object: Chen 2020\n    knowledge_level: knowledge_assertion\n    \
             agent_type: text_mining_agent\n    primary_source: Chen 2020\n---\n\n# Interleukin-6\n",
            "add molecule",
            None,
        )
        .unwrap();

        let report = scan(&kb).unwrap();
        for rule in [
            crate::knowledge::biookf::lint::RULE_EDGE_PRIMARY_SOURCE_UNRESOLVED,
            crate::knowledge::biookf::lint::RULE_EDGE_PRIMARY_SOURCE_NOT_SOURCE,
            crate::knowledge::biookf::lint::RULE_EDGE_OBJECT_UNRESOLVED,
            crate::knowledge::biookf::lint::RULE_SOURCE_UNANCHORED,
        ] {
            assert!(
                !report.diagnostics.has(rule),
                "{rule} fired on a bundle whose sources are all materialised: {:#?}",
                report.diagnostics.items
            );
        }
        assert_eq!(
            report
                .diagnostics
                .count(crate::knowledge::validate::Severity::Error),
            0,
            "{:#?}",
            report.diagnostics.items
        );
    }

    /// The credibility verdict `raw/<id>/meta.yaml` has always carried, finally
    /// read by the base's own lint — not only by a unit test of the rule.
    ///
    /// Before this, a BioOKF base could cite a **retracted** paper as the
    /// `primary_source` of a clinical claim and `scan` reported it clean: the
    /// classifier wrote `retracted: true` at ingest and nothing ever looked.
    #[test]
    fn a_retracted_source_is_a_finding_of_the_bases_own_lint() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write_page(
            &kb,
            "knowledge/publication/chen-2020.md",
            "---\ntype: Publication\nidentifier: Chen 2020\nxref: [PMID:32504360]\n\
             raw_source: [raw/chen-2020/source.md]\n---\n\n# Chen 2020\n",
            "add source node",
            None,
        )
        .unwrap();
        write_page(
            &kb,
            "knowledge/disease/covid-19.md",
            "---\ntype: Disease\nidentifier: COVID-19\nedges:\n  - predicate: reported_in\n    \
             object: Chen 2020\n    knowledge_level: knowledge_assertion\n    \
             agent_type: text_mining_agent\n    primary_source: Chen 2020\n---\n\n# COVID-19\n",
            "add disease",
            None,
        )
        .unwrap();

        // Un-classified first: the base is clean, which is what the guard
        // promises for every base built before the classifier existed.
        assert!(!scan(&kb)
            .unwrap()
            .diagnostics
            .has(crate::knowledge::biookf::lint::RULE_SOURCE_RETRACTED));

        write_retraction(&kb, "chen-2020");
        let report = scan(&kb).unwrap();
        assert!(
            report
                .diagnostics
                .has(crate::knowledge::biookf::lint::RULE_SOURCE_RETRACTED),
            "{:#?}",
            report.diagnostics.items
        );
    }

    /// `raw/<id>/meta.yaml` marking an ingested source retracted, in the shape
    /// the classifier writes it.
    fn write_retraction(kb_root: &Path, source_id: &str) {
        let meta = crate::knowledge::types::SourceMeta {
            id: source_id.to_string(),
            title: source_id.to_string(),
            url: None,
            ingested_at: Utc::now(),
            sha256: "0".into(),
            mime: "text/markdown".into(),
            original_filename: None,
            credibility: crate::knowledge::types::Credibility {
                tier: crate::knowledge::types::CredibilityTier::PeerReviewed,
                confidence: 0.9,
                publisher: None,
                venue: None,
                doi: None,
                retracted: true,
                reasoning: "fixture".into(),
                classifier_version: 1,
            },
        };
        let dir = kb_root.join("raw").join(source_id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta.yaml"), serde_yaml::to_string(&meta).unwrap()).unwrap();
    }

    /// The counterexample that makes the test above mean something: drop the
    /// source node and every edge citing it is reported. This is precisely what
    /// a BioOKF ingest looked like before DR-24 was implemented — one omission,
    /// one finding per edge, and nothing in the report naming the omission.
    #[test]
    fn without_the_source_node_every_citing_edge_is_reported() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write_page(
            &kb,
            "knowledge/disease/covid-19.md",
            "---\ntype: Disease\nidentifier: COVID-19\nedges:\n  - predicate: reported_in\n    \
             object: Chen 2020\n    knowledge_level: knowledge_assertion\n    \
             agent_type: text_mining_agent\n    primary_source: Chen 2020\n---\n\n# COVID-19\n",
            "add disease",
            None,
        )
        .unwrap();
        let report = scan(&kb).unwrap();
        assert!(report
            .diagnostics
            .has(crate::knowledge::biookf::lint::RULE_EDGE_PRIMARY_SOURCE_UNRESOLVED));
    }

    /// The vocabulary layer only runs in BioOKF mode, and it runs against a
    /// bundle index — so an edge into a page that exists resolves, and only the
    /// invented predicate is reported.
    #[test]
    fn a_biookf_base_reports_the_vocabulary_and_resolves_real_edges() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write_page(
            &kb,
            "knowledge/dataset/drugbank.md",
            "---\ntype: Dataset\nidentifier: DrugBank\nxref: [infores:drugbank]\n---\n\n# DrugBank\n",
            "add source node",
            None,
        )
        .unwrap();
        write_page(
            &kb,
            "knowledge/disease/headache.md",
            "---\ntype: Disease\nidentifier: Headache\n---\n\nSee [[Aspirin]].\n",
            "add disease",
            None,
        )
        .unwrap();
        write_page(
            &kb,
            "knowledge/molecule/aspirin.md",
            "---\ntype: Molecule\nidentifier: Aspirin\nedges:\n  - predicate: treats\n    \
             object: Headache\n    knowledge_level: knowledge_assertion\n    \
             agent_type: manual_agent\n    primary_source: DrugBank\n  - predicate: heals\n    \
             object: Headache\n    knowledge_level: knowledge_assertion\n    \
             agent_type: manual_agent\n    primary_source: DrugBank\n---\n\nSee [[Headache]].\n",
            "add molecule",
            None,
        )
        .unwrap();

        let report = scan(&kb).unwrap();
        assert!(
            report
                .diagnostics
                .has(crate::knowledge::biookf::lint::RULE_PREDICATE_INVALID),
            "{:?}",
            rules(&report)
        );
        assert!(
            !report
                .diagnostics
                .has(crate::knowledge::biookf::lint::RULE_EDGE_OBJECT_UNRESOLVED),
            "`treats -> Headache` should resolve against the bundle: {:?}",
            report.diagnostics.items
        );
        // The invalid predicate names a page, so the finding is actionable.
        let finding = report
            .diagnostics
            .items
            .iter()
            .find(|d| d.rule == crate::knowledge::biookf::lint::RULE_PREDICATE_INVALID)
            .unwrap();
        assert_eq!(finding.subject, "Aspirin");
        assert_eq!(
            finding.path.as_deref(),
            Some("knowledge/molecule/aspirin.md")
        );
    }

    /// `HashMap` iteration order is random. Without the sort in
    /// `format_diagnostics` the same base lints into a different order every
    /// run, which turns a diff of two reports into noise and makes the
    /// `MAX_DIAGNOSTICS` cut pick a different subset each time.
    #[test]
    fn two_scans_of_one_base_produce_the_same_diagnostics_in_the_same_order() {
        let (_dir, svc) = svc_in(KbFormat::Okf);
        let kb = svc.root().join("k");
        for i in 0..12 {
            write_page(
                &kb,
                &format!("knowledge/concept/p{i}.md"),
                "---\ntitle: P\n---\n\nbody\n",
                "add",
                None,
            )
            .unwrap();
        }
        let first = scan(&kb).unwrap().diagnostics;
        let second = scan(&kb).unwrap().diagnostics;
        assert_eq!(first, second);
        assert!(first.total >= 12);
    }

    /// A report written before Stage 4 has no `diagnostics` key at all.
    #[test]
    fn a_report_without_the_new_field_still_deserializes() {
        let legacy = r#"{"orphans":["knowledge/a.md"],"contradictions":[],
                         "stale_sources":[],"missing_concept_pages":[]}"#;
        let report: LintReport = serde_json::from_str(legacy).unwrap();
        assert_eq!(report.orphans, vec!["knowledge/a.md"]);
        assert!(report.diagnostics.is_empty());
    }

    // ---- The link-grammar widening ------------------------------------------

    /// One typed page: `type` + `identifier`, and an `edges:` array with the
    /// three required attributes. Deliberately carries **no bracket link at
    /// all**, because that is the shape of a base this migration produces and
    /// the shape the bracket-only reader was blind to.
    fn typed_page(kind: &str, id: &str, edges: &[(&str, &str)]) -> String {
        let mut fm = format!("---\ntype: {kind}\nidentifier: {id}\n");
        if !edges.is_empty() {
            fm.push_str("edges:\n");
            for (predicate, object) in edges {
                fm.push_str(&format!(
                    "  - predicate: {predicate}\n    object: {object}\n    \
                     knowledge_level: knowledge_assertion\n    \
                     agent_type: manual_agent\n    primary_source: DrugBank\n"
                ));
            }
        }
        fm.push_str(&format!("---\n\n# {id}\n\nProse with no links in it.\n"));
        fm
    }

    fn write(kb: &Path, path: &str, body: &str) {
        write_page(kb, path, body, "add", None).unwrap();
    }

    /// The defect, measured in the running app: a BioOKF base whose pages link
    /// to each other **only** through `edges:` frontmatter had every page
    /// reported as an orphan, while the subject band beside it counted the typed
    /// edges. Lint was reading one grammar and the graph four.
    ///
    /// The cycle is what makes this fail loudly against the old reader: every
    /// page has an inbound typed edge, so a correct lint reports **no** orphan
    /// and a bracket-only one reports all three. `lonely` is the control in the
    /// next test — without it "no orphans" is also what a check that stopped
    /// firing would print.
    #[test]
    fn a_base_linked_only_by_typed_edges_has_no_orphans() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write(
            &kb,
            "knowledge/molecule/aspirin.md",
            &typed_page("Molecule", "Aspirin", &[("treats", "Headache")]),
        );
        write(
            &kb,
            "knowledge/disease/headache.md",
            &typed_page("Disease", "Headache", &[("has_phenotype", "Pain")]),
        );
        write(
            &kb,
            "knowledge/phenotype/pain.md",
            &typed_page("PhenotypicFeature", "Pain", &[("treated_by", "Aspirin")]),
        );

        let report = scan(&kb).unwrap();
        assert!(
            report.orphans.is_empty(),
            "every page has an inbound `edges:` entry, so none is an orphan — \
             this is the reported defect: {:?}",
            report.orphans
        );
    }

    /// …and the check still fires. A page with no inbound edge in **any**
    /// grammar is still an orphan, which is what separates "lint reads the typed
    /// edges now" from "lint stopped reporting orphans".
    #[test]
    fn a_page_with_no_inbound_edge_in_any_grammar_is_still_an_orphan() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write(
            &kb,
            "knowledge/molecule/aspirin.md",
            &typed_page("Molecule", "Aspirin", &[("treats", "Headache")]),
        );
        write(
            &kb,
            "knowledge/disease/headache.md",
            &typed_page("Disease", "Headache", &[("treated_by", "Aspirin")]),
        );
        // Points outward at Aspirin and is named by nobody.
        write(
            &kb,
            "knowledge/molecule/ibuprofen.md",
            &typed_page("Molecule", "Ibuprofen", &[("treats", "Headache")]),
        );

        let report = scan(&kb).unwrap();
        assert_eq!(
            report.orphans,
            vec!["knowledge/molecule/ibuprofen.md"],
            "an outbound-only page is still unreferenced"
        );
    }

    /// A base carrying both generations at once — one page reachable only by a
    /// typed `edges:` entry, one only by a legacy bracket link, one only by an
    /// OKF §6.1 markdown link. Each grammar is the *sole* inbound route to its
    /// page, so dropping any one of the three orphans exactly that page.
    #[test]
    fn a_mixed_base_reads_all_three_inbound_grammars() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write(
            &kb,
            "knowledge/molecule/aspirin.md",
            &typed_page("Molecule", "Aspirin", &[("treats", "Headache")]),
        );
        write(
            &kb,
            "knowledge/disease/headache.md",
            "---\ntype: Disease\nidentifier: Headache\n---\n\nSee also [[Migraine]].\n",
        );
        write(
            &kb,
            "knowledge/disease/migraine.md",
            "---\ntype: Disease\nidentifier: Migraine\n---\n\n\
             Related: [Cluster headache](/knowledge/disease/cluster-headache.md).\n",
        );
        write(
            &kb,
            "knowledge/disease/cluster-headache.md",
            &typed_page("Disease", "Cluster headache", &[("treated_by", "Aspirin")]),
        );

        let report = scan(&kb).unwrap();
        assert!(
            report.orphans.is_empty(),
            "one page per grammar, each with exactly one inbound link: {:?}",
            report.orphans
        );
    }

    /// `missing_concept_pages` had the same blindness, and it matters on the
    /// same bases: a typed source page cites its concepts through `edges:`, so a
    /// bracket-only reader found no citations at all and the list was silently
    /// empty on every base the migration produces.
    #[test]
    fn a_typed_source_pages_unresolved_citation_is_a_missing_concept_page() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write(
            &kb,
            "knowledge/disease/covid-19.md",
            &typed_page("Disease", "COVID-19", &[]),
        );
        write(
            &kb,
            "knowledge/sources/chen-2020.md",
            &typed_page(
                "Publication",
                "Chen 2020",
                &[("mentions", "COVID-19"), ("mentions", "Long COVID")],
            ),
        );

        let report = scan(&kb).unwrap();
        assert_eq!(
            report.missing_concept_pages,
            vec!["Long COVID".to_string()],
            "the cited concept with no page, and only it — `COVID-19` resolves, \
             so a lint that listed it would be resolving differently from the \
             edge the graph just drew"
        );
    }

    /// The graph and the lint must now agree by construction, because they read
    /// the same edge set. Asserted on a typed base, which is where they used to
    /// disagree most completely: 17 edges drawn, every page called an orphan.
    #[test]
    fn no_page_the_graph_draws_an_inbound_edge_to_is_reported_as_an_orphan() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write(
            &kb,
            "knowledge/molecule/aspirin.md",
            &typed_page("Molecule", "Aspirin", &[("treats", "Headache")]),
        );
        write(
            &kb,
            "knowledge/disease/headache.md",
            &typed_page("Disease", "Headache", &[]),
        );

        let g = crate::knowledge::graph::derive(&kb).unwrap();
        let report = scan(&kb).unwrap();
        for edge in &g.edges {
            let Some(target) = g.nodes.iter().find(|n| n.id == edge.to) else {
                continue;
            };
            assert!(
                !report.orphans.contains(&target.path),
                "{} has an inbound {edge:?} in the graph and is an orphan in the \
                 lint; orphans={:?}",
                target.path,
                report.orphans
            );
        }
        assert!(!g.edges.is_empty(), "the premise of this test is gone");
    }

    // ---- Part A: scan tests -------------------------------------------------

    /// A page with no inbound links from other pages should be detected as an orphan.
    #[test]
    fn scan_finds_orphan_page() {
        let (_dir, svc) = fresh_svc();
        let kb = svc.root().join("k");

        // Page A links to page B. Page C has no inbound links → orphan.
        write_page(
            &kb,
            "knowledge/entities/a.md",
            "---\ntitle: A\nkind: entity\n---\n\nSee also [[B]].",
            "add a",
            None,
        )
        .unwrap();
        write_page(
            &kb,
            "knowledge/entities/b.md",
            "---\ntitle: B\nkind: entity\n---\n\nB body.",
            "add b",
            None,
        )
        .unwrap();
        write_page(
            &kb,
            "knowledge/entities/c.md",
            "---\ntitle: C\nkind: entity\n---\n\nC body with no inbound links.",
            "add c",
            None,
        )
        .unwrap();

        let report = scan(&kb).unwrap();
        let orphan_paths: Vec<_> = report.orphans.iter().map(|s| s.as_str()).collect();
        assert!(
            orphan_paths.contains(&"knowledge/entities/c.md"),
            "page c should be orphaned; orphans={orphan_paths:?}"
        );
        // Page B is referenced by A so it must NOT be an orphan.
        assert!(
            !orphan_paths.contains(&"knowledge/entities/b.md"),
            "page b is referenced by a, so not an orphan"
        );
    }

    /// A page with `contradiction: true` in frontmatter should appear in `contradictions`.
    #[test]
    fn scan_finds_contradiction_flag() {
        let (_dir, svc) = fresh_svc();
        let kb = svc.root().join("k");

        write_page(
            &kb,
            "knowledge/concepts/disputed.md",
            "---\ntitle: Disputed\nkind: concept\ncontradiction: true\n---\n\nConflicting claims.",
            "add disputed",
            None,
        )
        .unwrap();
        write_page(
            &kb,
            "knowledge/concepts/normal.md",
            "---\ntitle: Normal\nkind: concept\n---\n\nNormal body.",
            "add normal",
            None,
        )
        .unwrap();

        let report = scan(&kb).unwrap();
        let contra_paths: Vec<_> = report.contradictions.iter().map(|s| s.as_str()).collect();
        assert!(
            contra_paths.contains(&"knowledge/concepts/disputed.md"),
            "disputed.md should appear in contradictions; got {contra_paths:?}"
        );
        assert!(
            !contra_paths.contains(&"knowledge/concepts/normal.md"),
            "normal.md must not appear in contradictions"
        );
    }

    /// A source page that references `[[Z]]` but no `knowledge/concepts/z.md` exists
    /// should produce "Z" in `missing_concept_pages`.
    #[test]
    fn scan_finds_missing_concept() {
        let (_dir, svc) = fresh_svc();
        let kb = svc.root().join("k");

        // A source page that references [[Z]] (no knowledge/concepts/z.md exists).
        write_page(
            &kb,
            "knowledge/sources/paper-x.md",
            "---\ntitle: Paper X\nkind: source\n---\n\nThis paper mentions [[Z]] frequently.",
            "add paper",
            None,
        )
        .unwrap();

        let report = scan(&kb).unwrap();
        assert!(
            report
                .missing_concept_pages
                .iter()
                .any(|m| m.eq_ignore_ascii_case("Z")),
            "missing_concept_pages should contain 'Z'; got {:?}",
            report.missing_concept_pages
        );
    }

    #[test]
    fn deterministic_scan_observes_level_triggered_cancellation() {
        let (_dir, svc) = fresh_svc();
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();

        let error = scan_with_cancellation(&svc.root().join("k"), Some(&cancel))
            .expect_err("the deterministic read pass must observe cancellation itself");
        assert!(error.to_string().contains("cancelled"), "{error:#}");
    }

    // ---- Part B: async lint tests ------------------------------------------

    /// scan-only (autofix=false) should return the report with no commit.
    #[tokio::test]
    async fn lint_scan_only_returns_report_no_commit() {
        let (_dir, svc) = fresh_svc();
        let kb = svc.root().join("k");

        // Create one orphan page.
        write_page(
            &kb,
            "knowledge/entities/lonely.md",
            "---\ntitle: Lonely\nkind: entity\n---\n\nNo one links to me.",
            "add lonely",
            None,
        )
        .unwrap();

        let result = lint(
            &svc,
            LintArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                completer: None,
                autofix: false,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .unwrap();

        assert!(
            !result.report.orphans.is_empty(),
            "report should contain orphans"
        );
        assert!(
            result.commit_sha.is_none(),
            "scan-only must not produce a commit"
        );
        assert_eq!(result.fixes_applied, 0);
    }

    #[tokio::test]
    async fn a_read_only_lint_honors_level_triggered_cancellation() {
        let (_dir, svc) = fresh_svc();
        let cancel = tokio_util::sync::CancellationToken::new();
        cancel.cancel();

        let error = lint(
            &svc,
            LintArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                completer: None,
                autofix: false,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: Some(cancel),
            },
        )
        .await
        .expect_err("a read-only lint must honor a level-triggered cancellation");

        assert!(error.to_string().contains("cancelled"), "{error:#}");
    }

    /// An autofix run that fixed nothing must not commit either.
    ///
    /// Every clean knowledge base used to gain an empty `[lint] lint autofix`
    /// entry from a `--fix` pass with nothing to do, because `commit_txn` will
    /// happily record a commit whose tree equals its parent's. The user sees it
    /// in the Knowledge change-log drawer as work that never happened, and the
    /// `commit_sha` invites callers to say so — issue #71's false success, one
    /// macro over.
    #[tokio::test]
    async fn an_autofix_that_fixed_nothing_does_not_commit() {
        use crate::knowledge::subagent::loop_::{Completer, LlmMessage, LlmReply};

        /// Reports there is nothing to fix, without calling a tool.
        struct NothingToFix;

        #[async_trait::async_trait]
        impl Completer for NothingToFix {
            async fn complete(
                &self,
                _system: &str,
                _messages: &[LlmMessage],
                _tools: &[rmcp::model::Tool],
            ) -> anyhow::Result<LlmReply> {
                Ok(LlmReply {
                    text: "Nothing needed fixing.".into(),
                    tool_calls: Vec::new(),
                })
            }
        }

        let (_dir, svc) = fresh_svc();
        let kb = svc.root().join("k");
        write_page(
            &kb,
            "knowledge/entities/a.md",
            "---\ntitle: A\nkind: entity\n---\n\nA body.",
            "add a",
            None,
        )
        .unwrap();

        let result = lint(
            &svc,
            LintArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                completer: Some(Box::new(NothingToFix)),
                autofix: true,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .unwrap();

        assert!(
            result.commit_sha.is_none(),
            "an autofix that changed nothing must not hand back a commit sha, got: {:?}",
            result.commit_sha
        );
        assert_eq!(result.fixes_applied, 0);

        let log = svc.list_history("k", 10).unwrap();
        assert!(
            !log.iter().any(|e| e.kind == ChangeKind::Lint),
            "no lint commit may appear in the change log; log: {log:?}"
        );
    }

    /// rq2-027. An autofix that rewrote a page with the typed writer reported
    /// `fixes_applied: 0` (only `kb_write_page` was counted), and its terminal
    /// frame carried the report the run STARTED from, so it listed the finding
    /// it had just fixed. Both now describe the base as it stands.
    #[tokio::test]
    async fn an_autofix_counts_concept_writes_and_reports_what_remains() {
        use crate::knowledge::biookf::lint::{RULE_PREDICATE_INVALID, RULE_TYPE_INVALID};
        use crate::knowledge::subagent::kb_tools::WRITE_CONCEPT;
        use crate::knowledge::subagent::loop_::{Completer, LlmMessage, LlmReply, LlmToolCall};

        /// Fixes the predicate with the typed writer, then stops.
        struct FixesThePredicate {
            replies: tokio::sync::Mutex<Vec<LlmReply>>,
        }

        #[async_trait::async_trait]
        impl Completer for FixesThePredicate {
            async fn complete(
                &self,
                _system: &str,
                _messages: &[LlmMessage],
                _tools: &[rmcp::model::Tool],
            ) -> anyhow::Result<LlmReply> {
                let mut replies = self.replies.lock().await;
                anyhow::ensure!(!replies.is_empty(), "the mock ran out of replies");
                Ok(replies.remove(0))
            }
        }

        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write(
            &kb,
            "knowledge/dataset/drugbank.md",
            "---\ntype: Dataset\nidentifier: DrugBank\nxref: [infores:drugbank]\n---\n\n# DrugBank\n",
        );
        write(
            &kb,
            "knowledge/disease/headache.md",
            "---\ntype: Disease\nidentifier: Headache\n---\n\nSee [[Aspirin]].\n",
        );
        write(
            &kb,
            "knowledge/molecule/aspirin.md",
            &typed_page("Molecule", "Aspirin", &[("heals", "Headache")]),
        );
        write(
            &kb,
            "knowledge/gene/badgene.md",
            "---\ntype: Spaceship\nidentifier: BADGENE\n---\n\n# BADGENE\n",
        );
        let before = scan(&kb).unwrap();
        assert!(
            before.diagnostics.has(RULE_PREDICATE_INVALID),
            "{:?}",
            rules(&before)
        );
        assert!(
            before.diagnostics.has(RULE_TYPE_INVALID),
            "{:?}",
            rules(&before)
        );

        let fix = LlmReply {
            text: String::new(),
            tool_calls: vec![LlmToolCall {
                id: "fix-1".into(),
                name: WRITE_CONCEPT.into(),
                args: serde_json::json!({
                    "type": "Molecule",
                    "identifier": "Aspirin",
                    "path": "knowledge/molecule/aspirin.md",
                    "edges": [{
                        "predicate": "treats",
                        "object": "Headache",
                        "knowledge_level": "knowledge_assertion",
                        "agent_type": "text_mining_agent",
                        "primary_source": "DrugBank",
                    }],
                }),
            }],
        };
        let done = LlmReply {
            text: "Fixed the predicate.".into(),
            tool_calls: Vec::new(),
        };
        let result = lint(
            &svc,
            LintArgs {
                kb_id: "k".into(),
                caller_is_private: false,
                caller_affiliation: Default::default(),
                completer: Some(Box::new(FixesThePredicate {
                    replies: tokio::sync::Mutex::new(vec![fix, done]),
                })),
                autofix: true,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await
        .unwrap();

        assert!(result.commit_sha.is_some(), "the fix must commit");
        assert_eq!(result.fixes_applied, 1, "the typed write is a fix");
        assert!(
            !result.report.diagnostics.has(RULE_PREDICATE_INVALID),
            "the report must not list the finding the autofix fixed: {:?}",
            rules(&result.report)
        );
        assert!(
            result.report.diagnostics.has(RULE_TYPE_INVALID),
            "the report must still list what the autofix left: {:?}",
            rules(&result.report)
        );
    }

    #[test]
    fn lint_post_commit_refresh_failure_is_committed_and_retry_does_not_duplicate() {
        let (_dir, svc) = fresh_svc();
        let kb = svc.root().join("k");
        write_page(
            &kb,
            "knowledge/concept/a.md",
            &valid_page("concept", "a", "A"),
            "add A",
            None,
        )
        .unwrap();
        svc.rebuild_graph_cache("k").unwrap();
        let repo = GitRepo::open(&kb).unwrap();
        let txn = repo.begin_txn("lint-refresh-failure").unwrap();
        write_page(
            &kb,
            "knowledge/concept/b.md",
            &valid_page("concept", "b", "B"),
            "fix B",
            Some(&txn.branch),
        )
        .unwrap();
        crate::knowledge::graph::fail_cache_writes(&kb, 2);

        let error = settle_autofix(
            &svc,
            "k",
            &repo,
            &txn,
            scan(&kb).unwrap(),
            Ok(SubAgentResult {
                steps_used: 1,
                reason: DoneReason::NoMoreToolCalls,
                final_text: "fixed".into(),
                events: vec![],
            }),
            None,
        )
        .expect_err("the injected post-commit refresh must be reported");
        let failure = error
            .downcast_ref::<KnowledgeWriteFailure>()
            .expect("the durable phase remains machine-readable");
        assert_eq!(
            failure.phase,
            crate::knowledge::git::KnowledgeWriteFailurePhase::Committed
        );
        assert!(failure.commit_sha.is_some());
        assert!(!crate::knowledge::graph::cache_path(&kb).exists());
        let history_len = GitRepo::open(&kb).unwrap().log(10).unwrap().len();

        let retry = GitRepo::open(&kb).unwrap();
        let retry_txn = retry.begin_txn("lint-retry").unwrap();
        assert!(write_page(
            &kb,
            "knowledge/concept/b.md",
            &valid_page("concept", "b", "B"),
            "retry B",
            Some(&retry_txn.branch),
        )
        .unwrap()
        .is_none());
        let result = settle_autofix(
            &svc,
            "k",
            &retry,
            &retry_txn,
            scan(&kb).unwrap(),
            Ok(SubAgentResult {
                steps_used: 1,
                reason: DoneReason::NoMoreToolCalls,
                final_text: "already fixed".into(),
                events: vec![],
            }),
            None,
        )
        .unwrap();
        assert!(result.commit_sha.is_none());
        assert_eq!(
            GitRepo::open(&kb).unwrap().log(10).unwrap().len(),
            history_len
        );
    }

    // ── Issue #56, Task 10B: CP2 ────────────────────────────────────────────

    /// The other half of the pair: an AUTOFIX does write, so it must still
    /// ratchet. Without this, moving the raise off the entry would have traded
    /// one defect for its mirror image — a private chat rewriting a public
    /// base's pages and leaving it public.
    #[tokio::test]
    async fn an_autofix_still_ratchets_because_it_writes() {
        let (dir, svc) = fresh_svc();
        let root = dir.path().to_path_buf();
        assert!(!crate::knowledge::tier::is_private(&root, "k"));

        // No completer, so the autofix path fails immediately AFTER the raise —
        // which is what this asserts: the stamp lands on the writing half even
        // when the write itself does not complete.
        let _ = lint(
            &svc,
            LintArgs {
                kb_id: "k".into(),
                caller_is_private: true,
                caller_affiliation: Default::default(),
                completer: None,
                autofix: true,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await;

        assert!(
            crate::knowledge::tier::is_private(&root, "k"),
            "an autofix from a private caller left the base public"
        );
    }

    /// ⚠ A READ-ONLY LINT MUST NOT RATCHET. This asserted the opposite until
    /// 2026-08-20, and the behaviour it pinned was a live defect: because the
    /// HTTP and CLI lint routes hand this macro the caller's real tier, a
    /// scan run from any private-model chat permanently privatised whatever
    /// PUBLIC base it was pointed at and stamped that base with the caller's
    /// institution — an owner only deleting the base can remove. `server.rs`
    /// states the rule this restores: a preview writes nothing and must not
    /// raise a base's tier because a private chat *looked*.
    #[tokio::test]
    async fn a_read_only_lint_does_not_ratchet_a_public_base() {
        let (dir, svc) = fresh_svc();
        let root = dir.path().to_path_buf();
        assert!(!crate::knowledge::tier::is_private(&root, "k"));

        let _ = lint(
            &svc,
            LintArgs {
                kb_id: "k".into(),
                caller_is_private: true,
                caller_affiliation: Default::default(),
                completer: None,
                autofix: false,
                bounds: SubAgentBounds::default(),
                event_sink: None,
                cancel: None,
            },
        )
        .await;

        assert!(
            !crate::knowledge::tier::is_private(&root, "k"),
            "a read-only lint permanently privatised a public base"
        );
    }

    // ---- the source-page layout --------------------------------------------

    /// A raw source with an `ingested_at` far enough back to trip the 90-day
    /// rule, and a credibility block the classifier never wrote.
    fn write_old_source(kb_root: &Path, source_id: &str, days_ago: i64) {
        let meta = crate::knowledge::types::SourceMeta {
            id: source_id.to_string(),
            title: source_id.to_string(),
            url: None,
            ingested_at: Utc::now() - chrono::Duration::days(days_ago),
            sha256: "0".into(),
            mime: "text/markdown".into(),
            original_filename: None,
            credibility: crate::knowledge::types::Credibility {
                tier: crate::knowledge::types::CredibilityTier::PeerReviewed,
                confidence: 0.9,
                publisher: None,
                venue: None,
                doi: None,
                retracted: false,
                reasoning: "fixture".into(),
                classifier_version: 1,
            },
        };
        let dir = kb_root.join("raw").join(source_id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta.yaml"), serde_yaml::to_string(&meta).unwrap()).unwrap();
    }

    /// A source page written where BioOKF actually puts one, anchored to its
    /// raw source.
    fn anchored_source_page(id: &str, raw_id: &str) -> String {
        format!(
            "---\ntype: Publication\nidentifier: {id}\n\
             raw_source: [raw/{raw_id}/source.md]\n---\n\n# {id}\n"
        )
    }

    /// The defect: a heavily cited paper reported as stale because the rule
    /// looked for its page at `knowledge/sources/<id>.md`, which is the pre-OKF
    /// layout and the one layout no base created by this build uses.
    ///
    /// `chen-2020` is cited by a disease page through a typed edge, so nothing
    /// about it is stale but its ingest date. `orphan-2019` is the control, and
    /// it is what stops "no stale sources" being the answer a rule that simply
    /// stopped firing would also give.
    #[test]
    fn a_cited_source_is_not_stale_when_its_page_lives_where_okf_puts_it() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write_old_source(&kb, "chen-2020", 200);
        write_old_source(&kb, "orphan-2019", 400);
        write(
            &kb,
            "knowledge/publication/chen-2020.md",
            &anchored_source_page("Chen 2020", "chen-2020"),
        );
        write(
            &kb,
            "knowledge/publication/nobody-cites-me.md",
            &anchored_source_page("Nobody 2019", "orphan-2019"),
        );
        write(
            &kb,
            "knowledge/disease/covid-19.md",
            &typed_page("Disease", "COVID-19", &[("reported_in", "Chen 2020")]),
        );

        let report = scan(&kb).unwrap();
        assert_eq!(
            report.stale_sources,
            vec!["orphan-2019".to_string()],
            "a source cited by a typed edge was reported stale, and only the \
             genuinely unreferenced one should be"
        );
    }

    /// The other half of the same layout bug, failing in the opposite direction:
    /// the rule filtered its candidates on the `knowledge/sources/` prefix, so on
    /// a base whose source pages live anywhere else it matched nothing and was
    /// unconditionally empty — which reads exactly like a clean base.
    ///
    /// `COVID-19` has a page and `Long COVID` does not, so a rule that resolved
    /// differently from the graph would show up as either name appearing in the
    /// wrong list rather than as a missing assertion.
    #[test]
    fn a_source_page_outside_the_legacy_directory_still_reports_its_missing_citations() {
        let (_dir, svc) = svc_in(KbFormat::Biookf);
        let kb = svc.root().join("k");
        write(
            &kb,
            "knowledge/disease/covid-19.md",
            &typed_page("Disease", "COVID-19", &[]),
        );
        write(
            &kb,
            "knowledge/publication/chen-2020.md",
            &format!(
                "---\ntype: Publication\nidentifier: Chen 2020\n\
                 raw_source: [raw/chen-2020/source.md]\nedges:\n\
                 {}{}---\n\n# Chen 2020\n",
                "  - predicate: mentions\n    object: COVID-19\n    \
                 knowledge_level: knowledge_assertion\n    agent_type: manual_agent\n    \
                 primary_source: Chen 2020\n",
                "  - predicate: mentions\n    object: Long COVID\n    \
                 knowledge_level: knowledge_assertion\n    agent_type: manual_agent\n    \
                 primary_source: Chen 2020\n",
            ),
        );

        let report = scan(&kb).unwrap();
        assert_eq!(
            report.missing_concept_pages,
            vec!["Long COVID".to_string()],
            "the cited concept with no page, and only it"
        );
    }

    // ---- …and the same two rules on a PLAIN OKF base ------------------------
    //
    // ⚠ The two tests above are `svc_in(KbFormat::Biookf)`, and the first repair
    // of these rules passed both while leaving plain OKF — which is what
    // `create_base` produces by default — exactly as broken as it found it. Both
    // signals that repair read are absent there: `raw_source` is a BioOKF-only
    // key whose only two writers are gated on `KbFormat::is_biookf`, and OKF's
    // source directory is the SINGULAR `knowledge/source/` while the fallback
    // matched the pre-OKF plural. A format the fix does not exercise is a format
    // the fix does not cover.

    #[test]
    fn existing_raw_evidence_links_are_not_missing_concept_pages() {
        let (_dir, svc) = svc_in(KbFormat::Okf);
        let kb = svc.root().join("k");
        std::fs::create_dir_all(kb.join("raw/evidence")).unwrap();
        std::fs::write(kb.join("raw/evidence/source.md"), "Synthetic raw evidence").unwrap();
        write(
            &kb,
            "knowledge/source/evidence.md",
            "---\ntype: Source\nidentifier: Evidence\n---\n\n\
             [Relative evidence](../../raw/evidence/source.md)\n\
             [Root evidence](/raw/evidence/source.md)\n\
             [Bundle evidence](raw/evidence/source.md)\n\
             [Evidence section](../../raw/evidence/source.md#section)\n\
             [Missing evidence](../../raw/evidence/missing.md)\n\
             [Missing concept](/knowledge/method/missing.md)\n",
        );
        assert_eq!(
            scan(&kb).unwrap().missing_concept_pages,
            vec![
                "../../raw/evidence/missing.md".to_string(),
                "/knowledge/method/missing.md".to_string(),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn raw_evidence_warnings_remain_for_symlinks_and_bundle_escape() {
        let (_dir, svc) = svc_in(KbFormat::Okf);
        let kb = svc.root().join("k");
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("source.md"), "Outside evidence").unwrap();
        std::fs::create_dir_all(kb.join("raw/evidence")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("source.md"),
            kb.join("raw/evidence/external.md"),
        )
        .unwrap();
        write(
            &kb,
            "knowledge/source/evidence.md",
            "---\ntype: Source\nidentifier: Evidence\n---\n\n\
             [Symlink](../../raw/evidence/external.md)\n\
             [Escape](../../raw/../../../source.md)\n",
        );
        assert_eq!(
            scan(&kb).unwrap().missing_concept_pages,
            vec![
                "../../raw/../../../source.md".to_string(),
                "../../raw/evidence/external.md".to_string(),
            ]
        );
    }

    /// An OKF source page, stating its provenance the way `schema_okf.md`'s page
    /// contract does: a `sources:` list whose `resource` points into `raw/`.
    /// There is no `raw_source` key here and there cannot be one.
    fn okf_source_page(identifier: &str, raw_id: &str, links: &[&str]) -> String {
        let body: String = links
            .iter()
            .map(|target| format!("- [[{target}]]\n"))
            .collect();
        format!(
            "---\ntype: Source\nidentifier: {identifier}\ntitle: {identifier}\n\
             sources:\n  - id: {raw_id}\n    resource: raw/{raw_id}/source.md\n---\n\n\
             # {identifier}\n\n{body}"
        )
    }

    /// The half of the defect that reports a false positive, on the format the
    /// first repair missed. `chen-2020` is linked from a concept page, so the
    /// only thing stale about it is its ingest date; `orphan-2019` is the
    /// control, and it is what stops "no stale sources" — the answer a rule that
    /// simply stopped firing would also give — from passing.
    #[test]
    fn a_cited_source_on_a_plain_okf_base_is_not_stale() {
        let (_dir, svc) = svc_in(KbFormat::Okf);
        let kb = svc.root().join("k");
        write_old_source(&kb, "chen-2020", 200);
        write_old_source(&kb, "orphan-2019", 400);
        write(
            &kb,
            "knowledge/source/chen-2020.md",
            &okf_source_page("Chen 2020", "chen-2020", &[]),
        );
        write(
            &kb,
            "knowledge/source/nobody-cites-me.md",
            &okf_source_page("Nobody 2019", "orphan-2019", &[]),
        );
        write(
            &kb,
            "knowledge/concept/covid-19.md",
            "---\ntype: Concept\nidentifier: COVID-19\ntitle: COVID-19\n---\n\n\
             # COVID-19\n\nReported in [[Chen 2020]].\n",
        );

        let report = scan(&kb).unwrap();
        assert_eq!(
            report.stale_sources,
            vec!["orphan-2019".to_string()],
            "on an OKF base the source page carries no `raw_source` and lives in \
             the SINGULAR directory, so both of the first repair's signals miss \
             it and a cited paper is still reported stale"
        );
    }

    /// The half that reports nothing at all, on the same format. `COVID-19` has
    /// a page and `Long COVID` does not, so a rule resolving differently from the
    /// graph shows up as either name landing in the wrong list rather than as a
    /// missing assertion.
    #[test]
    fn an_okf_source_page_still_reports_its_missing_citations() {
        let (_dir, svc) = svc_in(KbFormat::Okf);
        let kb = svc.root().join("k");
        write(
            &kb,
            "knowledge/concept/covid-19.md",
            "---\ntype: Concept\nidentifier: COVID-19\ntitle: COVID-19\n---\n\n# COVID-19\n",
        );
        write(
            &kb,
            "knowledge/source/chen-2020.md",
            &okf_source_page("Chen 2020", "chen-2020", &["COVID-19", "Long COVID"]),
        );

        let report = scan(&kb).unwrap();
        assert_eq!(
            report.missing_concept_pages,
            vec!["Long COVID".to_string()],
            "the cited concept with no page, and only it"
        );
    }

    /// ⚠ The restriction that makes the rule above safe, asserted where it
    /// bites. `schema_okf.md`'s ingest workflow step 4 tells the model to record
    /// the source in `sources` on **every concept page it touches**, so if
    /// `sources[]` were read as "this is a source page", every concept page in
    /// the base would become one — and `missing_concept_pages` would report every
    /// unresolved link anywhere in the bundle instead of the citations a source
    /// makes.
    #[test]
    fn a_concept_page_citing_a_source_does_not_turn_the_whole_base_into_source_pages() {
        let (_dir, svc) = svc_in(KbFormat::Okf);
        let kb = svc.root().join("k");
        write(
            &kb,
            "knowledge/concept/covid-19.md",
            "---\ntype: Concept\nidentifier: COVID-19\ntitle: COVID-19\n\
             sources:\n  - id: chen-2020\n    resource: raw/chen-2020/source.md\n---\n\n\
             # COVID-19\n\nSee [[Some Concept With No Page]].\n",
        );

        let report = scan(&kb).unwrap();
        assert!(
            report.missing_concept_pages.is_empty(),
            "a concept page merely citing a source is not a source page, so its \
             unresolved links are not missing CONCEPT pages: {:?}",
            report.missing_concept_pages
        );
    }
}
