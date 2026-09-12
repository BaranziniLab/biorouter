use crate::knowledge::{
    biookf, convert, credibility,
    git::{GitRepo, KnowledgeWriteFailure},
    manifest, okf, paths, raw, registry,
    types::{
        manifest_generation, Credibility, KbFormat, Manifest, ManifestGeneration, ModelRef,
        RegistryEntry, SourceMeta,
    },
};
use anyhow::{Context, Result};
use chrono::Utc;
use dashmap::DashMap;
use fs2::FileExt as _;
use std::future::Future;
use std::sync::Arc;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

/// The `schema.md` a new base is scaffolded with, one per profile (DR-6).
///
/// Both carry `type: Schema` frontmatter (DR-23). OKF §3.1 reserves exactly
/// `index.md` and `log.md` and then says "All other `.md` files are concept
/// documents", so an untyped `schema.md` is a **conformance failure**, not an
/// extension — BioOKF's own spec reserves a third file and is wrong to.
const SCHEMA_OKF: &str = include_str!("schema_okf.md");
const SCHEMA_BIOOKF: &str = include_str!("schema_biookf.md");

/// Placeholders the BioOKF schema template carries, filled from
/// [`crate::knowledge::biookf::vocabulary`] at write time rather than typed into
/// the markdown.
///
/// The vocabulary is declared once, by a macro over a single table, precisely so
/// the enum and its accessors cannot drift; a hand-written copy of the 28 types
/// in a prompt file would be the one place that could, and it would drift
/// silently because nothing reads a prompt for correctness.
const PLACEHOLDER_NODE_TYPES: &str = "{{NODE_TYPES}}";
const PLACEHOLDER_PREDICATES: &str = "{{PREDICATES}}";
const PLACEHOLDER_KNOWLEDGE_LEVELS: &str = "{{KNOWLEDGE_LEVELS}}";
const PLACEHOLDER_AGENT_TYPES: &str = "{{AGENT_TYPES}}";

const DEFAULT_LOG: &str = "# Log\n\n";
const GITIGNORE: &str =
    "raw/*/original.*\n.biorouter-knowledge/.crossref-cache/\n.biorouter-knowledge/write.lock\n";

/// What a directory under the knowledge root is, as far as a caller that may
/// **destroy** it is allowed to conclude.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseFormat {
    /// The manifest states a pre-OKF generation *and* the tree has the shape a
    /// Biorouter knowledge base has. The only verdict that authorizes deletion.
    Legacy,
    /// The manifest states the OKF generation or a later one.
    Current,
    /// Neither could be established. Carries the reason, because a caller that
    /// leaves a directory alone has to be able to say which one and why.
    Undiagnosable(String),
}

/// Classify a directory strictly enough to destroy it.
///
/// [`Manifest::is_legacy_format`] is the wrong question here and the difference
/// is the whole of this function: it is `schema_version < CURRENT_SCHEMA_VERSION`
/// over a struct whose every field defaults, so a `manifest.yaml` of `{}`, a
/// current BioOKF base that lost its `schema_version` line to a partial write,
/// and an unrelated tool's directory with an id-shaped name all answer "legacy"
/// — and the startup purge would `remove_dir_all` every one of them, `.git`
/// history included, logging a success.
///
/// So two things the accident cannot produce are required: the file must
/// **state** its generation ([`manifest_generation`]), and the tree must carry
/// the two things every base this build has ever written carries. Anything else
/// is [`BaseFormat::Undiagnosable`], which callers must read as "leave it
/// exactly where it is, and say so".
///
/// Deliberately **not** `Result`: an unreadable directory is a verdict here, not
/// an error, because the one caller runs on the daemon's startup path and an
/// `Err` there is a machine that will not boot over a base nobody asked about.
pub fn classify_base_format(kb_root: &Path) -> BaseFormat {
    let manifest_path = manifest::manifest_path(kb_root);
    let yaml = match std::fs::read_to_string(&manifest_path) {
        Ok(yaml) => yaml,
        Err(error) => {
            return BaseFormat::Undiagnosable(format!(
                "cannot read {}: {error}",
                manifest_path.display()
            ))
        }
    };
    match manifest_generation(&yaml) {
        ManifestGeneration::DeclaredCurrent(_) => BaseFormat::Current,
        ManifestGeneration::Undeclared => BaseFormat::Undiagnosable(format!(
            "{} states no schema_version this build can read, so its generation is unknown",
            manifest_path.display()
        )),
        // A stated pre-OKF generation is necessary and not sufficient. Any YAML
        // document may carry a `schema_version` key meaning something else
        // entirely; a base this build wrote also has a `knowledge/` tree and the
        // `schema.md` that is its sub-agent's system prompt.
        ManifestGeneration::DeclaredLegacy(stated) => {
            if kb_root.join("knowledge").is_dir() && kb_root.join("schema.md").is_file() {
                BaseFormat::Legacy
            } else {
                BaseFormat::Undiagnosable(format!(
                    "{} states schema_version {stated}, but the directory has no knowledge/ \
                     tree and schema.md, so it is not a knowledge base this build wrote",
                    kb_root.display()
                ))
            }
        }
    }
}

/// The generation this build writes, and the ceiling an automatic migration may
/// reach. Both live on [`crate::knowledge::types`] beside [`Manifest`] itself,
/// because [`Manifest::profile`] — the accessor every reader should use — has to
/// fold the first one in, and a second declaration of a number is one more than
/// can be kept in step.
pub use crate::knowledge::types::{AUTOMATIC_SCHEMA_CEILING, CURRENT_SCHEMA_VERSION};

#[derive(Debug, thiserror::Error)]
#[error(
    "legacy pre-OKF knowledge-base archives are no longer supported; import an OKF or BioOKF archive"
)]
pub struct LegacyKnowledgeArchiveUnsupported;

#[derive(Debug, thiserror::Error)]
#[error(
    "knowledge base '{kb_id}' uses the retired pre-OKF format; restart Biorouter to finish the legacy purge, then use an OKF or BioOKF knowledge base"
)]
pub struct LegacyKnowledgeBaseUnsupported {
    pub kb_id: String,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "cannot restore knowledge base '{kb_id}' to commit '{commit_sha}': that commit uses the retired pre-OKF format"
)]
pub struct LegacyKnowledgeRestoreUnsupported {
    pub kb_id: String,
    pub commit_sha: String,
}

/// A raw source commit succeeded, but the derived graph cache could not be
/// refreshed afterwards.
///
/// The committed [`raw::RawWrite`] is retained so callers can report durable
/// state instead of collapsing this into an ordinary pre-write error. The graph
/// cache is derived and repairable; the commit is the source of truth.
#[derive(Debug)]
pub struct RawSourceRefreshFailure {
    pub written: raw::RawWrite,
    cause: String,
}

struct PreparedRawSource {
    title: String,
    url: Option<String>,
    original_bytes: Option<Vec<u8>>,
    original_filename: Option<String>,
    sha256: String,
    credibility: Credibility,
    converted: convert::Converted,
}

impl std::fmt::Display for RawSourceRefreshFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let commit = self
            .written
            .commit_sha
            .as_deref()
            .map_or_else(String::new, |sha| format!(" in commit {sha}"));
        write!(
            f,
            "raw source {} was committed{commit}, but the graph cache could not be refreshed: {}. The cache will be re-derived on its next read; retry will reuse the committed source",
            self.written.source_id, self.cause
        )
    }
}

impl std::error::Error for RawSourceRefreshFailure {}

/// Cross-reference rules block appended to `schema.md` files still at
/// generation 1. Kept in sync with the equivalent block in `schema_default.md`.
const SCHEMA_CROSSREF_RULES: &str = r#"
### Cross-reference rules (the graph depends on these)

The knowledge graph is derived **purely** from `[[link]]` patterns in page
bodies. If you do not emit links, the graph will have nodes but no edges.

When you write or update any knowledge page:

1. Every mention of another entity or concept that has (or should have) its
   own page **must** be wrapped in `[[double brackets]]`. Match the target
   page's title exactly (case-insensitive); the deriver slugifies both sides.
   Good: `[[EPAS1]] interacts with [[HIF2A]] under [[hypoxia]].`
   Bad:  `EPAS1 interacts with HIF2A under hypoxia.`
2. Every source page **must** include a `## Related pages` section listing
   every entity/concept it touches, one `- [[Name]]` bullet per line.
3. Every entity/concept page **must** include a `## Sources` section with
   one `- [[source-id]]` bullet per supporting source.
4. Prefer linking over re-stating. If a fact lives on another page, write
   `See [[Page Name]]` instead of restating it.

The lint workflow (`kb_lint`) reports pages with no inbound links as orphans;
fix them by adding inbound `[[links]]` from related pages.
"#;

/// Every `schema.md` step from generation `from` up to
/// [`AUTOMATIC_SCHEMA_CEILING`], applied in order.
///
/// A ladder rather than one branch, because a base can be arbitrarily far
/// behind: a user who has not opened a knowledge base since before a migration
/// landed must get every step, in order, on the next macro that touches it.
/// Each step keeps its own idempotence guard so that a base whose *stamp* is
/// behind but whose *content* is not comes out unchanged — which is the state
/// every base on disk is in for step 1 → 2 (see
/// [`KnowledgeService::migrate_schema_if_needed`]).
///
/// ⚠ **There is deliberately no 2 → 3 step, and there must not be one here.**
/// Generation 3 is the OKF format, and reaching it is not a schema edit: it
/// rewrites the base's *pages* out of `title`/`kind` frontmatter and `[[wiki]]`
/// links into typed OKF frontmatter. DR-17 traces three concrete privacy
/// bypasses that a migration on this path would open — this ladder runs from
/// the three macros with no caller identity of its own, so a rewrite here would
/// touch every page of a private base with nothing having called
/// `tier::assert_reachable` — and DR-22 defers the migration outright. A base
/// below generation 3 keeps working untouched, through its own generation's
/// path, which is exactly what DR-6 promises.
fn migrated_schema(current: &str, from: u32) -> String {
    let mut schema = current.to_string();
    if from < 2 {
        schema = with_crossref_rules(schema);
    }
    schema
}

/// Step 1 → 2: teach the sub-agent that the graph is derived purely from
/// `[[link]]` patterns. Without it a base gains nodes and no edges.
fn with_crossref_rules(mut schema: String) -> String {
    if schema.contains("Cross-reference rules") {
        return schema;
    }
    // A blank line between whatever the user had and the new section, even if
    // their file did not end with a newline.
    if !schema.ends_with('\n') {
        schema.push('\n');
    }
    schema.push_str(SCHEMA_CROSSREF_RULES);
    schema
}

/// The `knowledge/` subdirectories a new base is scaffolded with.
///
/// Under OKF the layout is **producer-defined** — §3.1 reserves two filenames
/// and says nothing about directories — so this is a starting convention, not a
/// contract, and the convention both profiles teach is one directory per
/// lowercased `type`.
///
/// The two lists differ because the two vocabularies do. OKF's is open, so three
/// generic type names are all that can honestly be pre-created; a fourth would
/// be a guess about a base nobody has written yet. BioOKF's is closed, and yet
/// pre-creating all 28 would be a table of contents for a bundle that does not
/// exist — so it gets the four SPEC §8.1 **source** types, which are the only
/// directories the profile genuinely requires: every edge must cite a
/// `primary_source`, and that citation has to resolve to a real page of one of
/// these four types. Entity directories appear as their types are used.
///
/// ⚠ **Everything here is under `knowledge/`, and that is load-bearing** (issue
/// #71). `GitRepo::txn_wrote_knowledge_pages` compares only the `knowledge/`
/// subtree oid, because `log.md`, `raw/` and `index.md` each move the whole tree
/// on their own — an ingest that wrote nothing but a log line would otherwise
/// look like a digest. Scaffolding authored content anywhere else silently
/// breaks that guard.
fn scaffold_dirs(format: KbFormat) -> Vec<String> {
    match format {
        KbFormat::Okf => ["concept", "source", "note"]
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        KbFormat::Biookf => biookf::NodeType::SOURCE_TYPES
            .iter()
            .map(|t| t.as_str().to_lowercase())
            .collect(),
    }
}

/// The `schema.md` for `format`, with the BioOKF template's vocabulary
/// placeholders filled in from the vocabulary module.
fn schema_for(format: KbFormat) -> String {
    match format {
        KbFormat::Okf => SCHEMA_OKF.to_string(),
        KbFormat::Biookf => SCHEMA_BIOOKF
            .replace(PLACEHOLDER_NODE_TYPES, &node_type_cheatsheet())
            .replace(PLACEHOLDER_PREDICATES, &predicate_cheatsheet())
            .replace(
                PLACEHOLDER_KNOWLEDGE_LEVELS,
                &join(biookf::KNOWLEDGE_LEVELS),
            )
            .replace(PLACEHOLDER_AGENT_TYPES, &join(biookf::AGENT_TYPES)),
    }
}

fn join(words: &[&str]) -> String {
    words.join(", ")
}

/// The 28 types, grouped by SPEC §5's own two families so the 20/8 split is
/// visible rather than implied by a comma count.
fn node_type_cheatsheet() -> String {
    biookf::Family::ALL
        .iter()
        .map(|family| {
            let members: Vec<&str> = family.members().iter().map(|t| t.as_str()).collect();
            format!(
                "- **{}** ({}): {}",
                family.as_str(),
                members.len(),
                members.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The 24 positive predicates, plus the negatable subset named rather than
/// listed twice — `not_<X>` is derived from `<X>`, and printing both lists is
/// how the two would come to disagree.
fn predicate_cheatsheet() -> String {
    let positives: Vec<&str> = biookf::PositivePredicate::ALL
        .iter()
        .map(|p| p.as_str())
        .collect();
    let negatables: Vec<String> = biookf::PositivePredicate::negatables()
        .iter()
        .map(|p| format!("{}{}", biookf::NEGATION_PREFIX, p.as_str()))
        .collect();
    format!(
        "- **Positive** ({}): {}\n- **Negated** ({}): {}",
        positives.len(),
        positives.join(", "),
        negatables.len(),
        negatables.join(", ")
    )
}

/// The bundle-root `index.md` a new base is scaffolded with (OKF §8).
///
/// `okf_version` in the frontmatter, and nothing else: §8 permits exactly that
/// one key in exactly this one file, which is why `okf::check_index` takes an
/// `is_bundle_root` flag. `biookf_version` is deliberately absent even for a
/// BioOKF base — it lives in `manifest.yaml` (DR-23's corollary), and BioOKF's
/// own spec permitting it here is one of its two known divergences from OKF
/// v0.2.
///
/// Built through `serde_yaml` rather than `format!` so the revision is emitted
/// **quoted**. Unquoted, YAML resolves `0.2` to a float and a later `0.10` would
/// silently become `0.1` — a revision that sorts *below* `0.2`.
fn index_scaffold() -> String {
    let mut frontmatter = serde_yaml::Mapping::new();
    frontmatter.insert("okf_version".into(), okf::OKF_VERSION.into());
    let yaml = serde_yaml::to_string(&serde_yaml::Value::Mapping(frontmatter))
        .expect("a one-key string mapping always serializes");
    format!("---\n{yaml}---\n\n# Pages\n\n_No pages yet._\n")
}

#[derive(Clone)]
pub struct KnowledgeService {
    root: PathBuf,
    locks: Arc<DashMap<String, Arc<Mutex<()>>>>,
}

struct FileLockGuard {
    file: File,
}

struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Did `try_lock_exclusive` fail because somebody else holds the lock — as
/// opposed to failing for real?
///
/// ⚠ **The error kind alone is not the answer, and the platform that disagrees
/// is Windows.** `fs2` reports contention with whatever the OS returned: Unix's
/// `EWOULDBLOCK` decodes to [`std::io::ErrorKind::WouldBlock`], but Windows
/// returns `ERROR_LOCK_VIOLATION` (os error 33), which `std` does not decode at
/// all and leaves `Uncategorized`. Matching on the kind therefore made every
/// contended acquisition on Windows a hard error: the poll loop propagated it
/// instead of waiting, so a queued lint reported "another process has locked a
/// portion of the file" and a cancellable mutation returned that instead of
/// "cancelled". `fs2::lock_contended_error()` is the crate's own name for the
/// error it raises, so asking it is the platform-correct question.
fn is_lock_contended(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return true;
    }
    let contended = fs2::lock_contended_error();
    error.raw_os_error().is_some() && error.raw_os_error() == contended.raw_os_error()
}

impl FileLockGuard {
    fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.lock_exclusive()?;
        Ok(Self { file })
    }

    /// How long a knowledge write may wait for another holder before it gives
    /// up and says so (#157).
    ///
    /// FINITE, because the alternative measured in a live daemon was an unbounded
    /// wait: writes to one base stopped answering entirely — no result, no error,
    /// no timeout — while reads to the same base and writes to other bases were
    /// fine, and only restarting the daemon cleared it. The desktop app runs one
    /// long-lived daemon (observed uptime: days), so "until restart" is a long
    /// time to be unable to save.
    ///
    /// ⚠ And LONGER THAN THE LONGEST LEGITIMATE HOLD, which is what actually sets
    /// the number. A macro holds this lock across its whole sub-agent loop, so the
    /// ceiling is that loop's own budget — [`SubAgentBounds::max_wall`], 300 s by
    /// default but **900 s** as `biorouter-cli`'s knowledge commands construct it
    /// — plus conversion, staging and commit either side of it. A bound below that
    /// does not report a wedge, it manufactures one: an ordinary long ingest would
    /// fail every concurrent caller with a message blaming a holder that is in fact
    /// working normally, and the louder the message the more convincing the false
    /// report. This constant was first written as 120 s, under even the DEFAULT
    /// budget, which is the bug this paragraph exists to stop recurring;
    /// `the_lock_wait_exceeds_every_macro_wall_clock_budget` fails if a macro
    /// budget is ever raised past it.
    ///
    /// Raising it also lengthens the SYNCHRONOUS wait in [`Self::acquire_bounded`],
    /// which would be a bad trade if it could pin a Tokio worker for half an hour.
    /// It cannot: checked caller by caller, every production path into the sync
    /// `lock_existing_kb` runs inside `spawn_blocking` — the `*_async` wrappers go
    /// through `run_existing_kb_mutation`, and `reset_knowledge` is spawned by its
    /// route. Re-check that before raising this further.
    const KB_WRITE_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(1800);

    /// How long a READ may wait for the same lock (#159).
    ///
    /// Deliberately far shorter than [`Self::KB_WRITE_LOCK_WAIT`], and the
    /// asymmetry is the point rather than an oversight:
    ///
    /// * A **write** that gives up loses work, so it waits out the longest
    ///   legitimate holder.
    /// * A **read** that gives up loses nothing and is safe to retry, so making
    ///   it wait the same 30 minutes buys the caller nothing and costs them a
    ///   view of their own knowledge base.
    ///
    /// Reads take this lock for FAIRNESS, not visibility — `get_graph`'s own
    /// note says so, and `read_page` takes no lock at all — so the property to
    /// preserve is "a stream of readers cannot starve a macro", which a bounded
    /// wait preserves exactly. What it drops is the case that has no business
    /// blocking a read at all: an OPEN TRANSACTION, which holds this lock for as
    /// long as somebody leaves it open. Measured before this bound:
    /// `kb_get_graph` and `kb_lint` never returned while a transaction was open
    /// on the base, while another base answered in 8 ms.
    const KB_READ_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(45);

    /// How long a DELETE may wait for the same lock.
    ///
    /// Same asymmetry as [`Self::KB_READ_LOCK_WAIT`] and a sharper edge. The
    /// long wait exists so a write does not lose work; a delete has NO work to
    /// lose — nothing is staged, the base is untouched, and retrying costs the
    /// caller a second tool call.
    ///
    /// What it would otherwise wait out is precisely the case where waiting is
    /// wrong. `kb_delete_base` refuses outright when it can see an open
    /// transaction on the base, but that check reads ONE `KnowledgeServer`'s
    /// registry and every chat gets its own; a transaction opened in another
    /// conversation is invisible to it, so the delete sails past the refusal and
    /// parks on the lock that transaction is holding instead. For half an hour,
    /// in a chat that looks hung. The messages `lock_kb_path_waiting` raises on
    /// elapse already say "most often that is an OPEN TRANSACTION: commit or
    /// abort it" — this is what makes the user read them today rather than
    /// after lunch.
    const KB_DELETE_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(45);

    /// `acquire`, but bounded. Whatever holds the lock, the caller gets a
    /// sentence naming the base instead of a call that never returns.
    ///
    /// This does NOT explain why a lock is ever held that long — that is #157's
    /// open question, and this deliberately does not pretend to answer it. It
    /// converts an unbounded wait into a reportable failure, which is worth
    /// having whatever the cause turns out to be.
    fn acquire_bounded(path: &Path, wait: std::time::Duration, what: &str) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        let deadline = std::time::Instant::now() + wait;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { file }),
                Err(error) if is_lock_contended(&error) => {
                    if std::time::Instant::now() >= deadline {
                        anyhow::bail!(
                            "timed out after {}s waiting for the write lock on {what}. Another \
                             knowledge operation still holds it; nothing was written. If this \
                             persists with no operation running, restarting Biorouter releases \
                             the lock (see issue #157).",
                            wait.as_secs()
                        );
                    }
                    std::thread::park_timeout(std::time::Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Interruptible `flock` acquisition for async callers. There is no
    /// artificial deadline: a live operation waits as long as its owner does,
    /// while cancellation bounds shutdown latency to one poll interval.
    fn acquire_cancellable(path: &Path, cancel: &CancellationToken) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        loop {
            if cancel.is_cancelled() {
                anyhow::bail!("knowledge operation cancelled while waiting for a file lock");
            }
            match file.try_lock_exclusive() {
                Ok(()) => {
                    if cancel.is_cancelled() {
                        let _ = file.unlock();
                        anyhow::bail!("knowledge operation cancelled after acquiring a file lock");
                    }
                    return Ok(Self { file });
                }
                Err(error) if is_lock_contended(&error) => {
                    std::thread::park_timeout(std::time::Duration::from_millis(10));
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub struct KnowledgeWriteGuard {
    _process_guard: OwnedMutexGuard<()>,
    _file_guard: FileLockGuard,
}

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
    /// Forget the pointer *at this scope*. KB-less writes then fail until one
    /// is chosen — including in a session whose machine-wide default still
    /// names a base. See [`StoredPrimary::NoPrimary`].
    Clear,
    /// Drop this scope's own pointer so it falls back to the machine-wide one.
    /// The mirror of [`KnowledgeService::clear_hidden_for_session`], and
    /// distinct from [`PrimaryUpdate::Clear`]. At machine scope this restores
    /// Biorouter's product default (`soul`); it does not mean explicit no-primary.
    Inherit,
    /// Pin this id. It must be a member of the *resulting* set.
    Set(&'a str),
}

/// The three states a scope's primary-pointer file can be in.
///
/// Two of them collapse into `None` under a naive `Option<String>` read, and
/// that collapse *was* a bug: a session that says "no primary here" is not a
/// session that has said nothing. Clearing a session's primary used to delete
/// its file, which is the encoding of "I have no opinion" — so the very next
/// read handed the session back the machine-wide default it had just rejected,
/// silently re-arming KB-less writes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredPrimary {
    /// No file. This scope has expressed nothing, so a session falls back to
    /// the machine-wide pointer and the machine scope falls back to Soul.
    Inherit,
    /// A file holding one bare kb id.
    Pinned(String),
    /// A file that exists but is blank: an explicit "this scope has no
    /// primary", which does **not** inherit. Blank rather than a sentinel word
    /// so a lagging PATH-installed `biorouter` (see CLAUDE.md, "Runtime
    /// CLI-vs-app drift") reading `.active-kb` trims it to nothing and agrees.
    NoPrimary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeletePathState {
    Missing,
    File(Vec<u8>),
    Directory(Vec<(std::ffi::OsString, DeletePathState)>),
}

impl DeletePathState {
    fn capture(path: &Path) -> Result<Self> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::Missing);
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.is_file() {
            return Ok(Self::File(std::fs::read(path)?));
        }
        anyhow::ensure!(
            metadata.is_dir(),
            "knowledge deletion metadata path is neither a file nor a directory: {}",
            path.display()
        );
        let mut names = std::fs::read_dir(path)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<_>>>()?;
        names.sort();
        let mut entries = Vec::with_capacity(names.len());
        for name in names {
            entries.push((name.clone(), Self::capture(&path.join(name))?));
        }
        Ok(Self::Directory(entries))
    }

    fn restore(&self, path: &Path) -> Result<()> {
        remove_path_if_present(path)?;
        match self {
            Self::Missing => Ok(()),
            Self::File(contents) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, contents)?;
                Ok(())
            }
            Self::Directory(entries) => {
                std::fs::create_dir_all(path)?;
                for (name, state) in entries {
                    state.restore(&path.join(name))?;
                }
                Ok(())
            }
        }
    }
}

fn remove_path_if_present(path: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn staged_delete_id(name: &std::ffi::OsStr) -> Option<String> {
    let name = name.to_str()?.strip_prefix(".deleting-")?;
    let separator = name.len().checked_sub(37)?;
    let id = name.get(..separator)?;
    let uuid = name.get(separator..)?.strip_prefix('-')?;
    uuid::Uuid::parse_str(uuid).ok()?;
    paths::validate_kb_id(id).ok()?;
    Some(id.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeleteMetadataSnapshot {
    paths: Vec<(PathBuf, DeletePathState)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BasePublicationSnapshot {
    paths: Vec<(PathBuf, DeletePathState)>,
}

impl BasePublicationSnapshot {
    fn capture(root: &Path) -> Result<Self> {
        let registry_path = registry::registry_path(root);
        let tiers_path = paths::kb_tiers_path(root);
        let paths = [
            registry_path.clone(),
            registry_path.with_extension("yaml.tmp"),
            tiers_path.clone(),
            tiers_path.with_extension("tmp"),
        ]
        .into_iter()
        .map(|path| Ok((path.clone(), DeletePathState::capture(&path)?)))
        .collect::<Result<Vec<_>>>()?;
        Ok(Self { paths })
    }

    fn restore(&self) -> Result<()> {
        let mut failures = Vec::new();
        for (path, state) in &self.paths {
            if let Err(error) = state.restore(path) {
                failures.push(format!("{}: {error}", path.display()));
            }
        }
        anyhow::ensure!(
            failures.is_empty(),
            "could not restore knowledge publication metadata: {}",
            failures.join("; ")
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreateCheckpoint {
    Files,
    Repository,
    GraphCache,
    Classification,
    Registry,
    Published,
}

struct CreateBaseSpec<'a> {
    id: &'a str,
    name: &'a str,
    color: Option<&'a str>,
    format: KbFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportCheckpoint {
    Staged,
    Classification,
    Registry,
    Published,
}

fn staged_publication_id(name: &std::ffi::OsStr) -> Option<String> {
    let name = name.to_str()?;
    let name = [".importing-", ".creating-"]
        .into_iter()
        .find_map(|prefix| name.strip_prefix(prefix))?;
    let separator = name.len().checked_sub(37)?;
    let id = name.get(..separator)?;
    let uuid = name.get(separator..)?.strip_prefix('-')?;
    uuid::Uuid::parse_str(uuid).ok()?;
    paths::validate_kb_id(id).ok()?;
    Some(id.to_string())
}

fn publication_rollback_failure(
    operation: &str,
    error: anyhow::Error,
    paths: &[PathBuf],
    metadata: &BasePublicationSnapshot,
) -> KnowledgeWriteFailure {
    let mut rollback_failures = Vec::new();
    for path in paths {
        if let Err(rollback_error) = remove_path_if_present(path) {
            rollback_failures.push(format!("remove {}: {rollback_error:#}", path.display()));
        }
    }
    if let Err(rollback_error) = metadata.restore() {
        rollback_failures.push(format!("restore metadata: {rollback_error:#}"));
    }
    if rollback_failures.is_empty() {
        KnowledgeWriteFailure::rolled_back(operation, error)
    } else {
        KnowledgeWriteFailure::outcome_uncertain(
            operation,
            anyhow::anyhow!(
                "{error:#}; rollback also failed: {}",
                rollback_failures.join("; ")
            ),
        )
    }
}

impl DeleteMetadataSnapshot {
    fn capture(root: &Path) -> Result<Self> {
        let registry_path = registry::registry_path(root);
        let primary_path = paths::primary_kb_path(root);
        let hidden_path = paths::hidden_kbs_path(root);
        let tiers_path = paths::kb_tiers_path(root);
        let paths = vec![
            tiers_path.clone(),
            tiers_path.with_extension("tmp"),
            primary_path.clone(),
            primary_path.with_extension("tmp"),
            paths::primary_kb_sessions_dir(root),
            hidden_path.clone(),
            hidden_path.with_extension("tmp"),
            paths::hidden_kb_sessions_dir(root),
            registry_path.clone(),
            registry_path.with_extension("yaml.tmp"),
        ];
        let paths = paths
            .into_iter()
            .map(|path| Ok((path.clone(), DeletePathState::capture(&path)?)))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { paths })
    }

    fn restore(&self) -> Result<()> {
        let mut failures = Vec::new();
        for (path, state) in &self.paths {
            if let Err(error) = state.restore(path) {
                failures.push(format!("{}: {error}", path.display()));
            }
        }
        anyhow::ensure!(
            failures.is_empty(),
            "could not restore knowledge deletion metadata: {}",
            failures.join("; ")
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteCheckpoint {
    Staged,
    Registry,
    MachinePrimary,
    SessionPrimaries,
    HiddenSelections,
    Classification,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "knowledge base '{kb_id}' was removed from the active store, but its staged files could not be fully erased: {cause}"
)]
pub struct KnowledgeDeleteCleanupFailure {
    pub kb_id: String,
    cause: String,
}

impl StoredPrimary {
    /// The pinned id, if any. Both `Inherit` and `NoPrimary` are "no id here";
    /// only [`KnowledgeService::primary_for_session`] cares which.
    fn pinned(&self) -> Option<&str> {
        match self {
            StoredPrimary::Pinned(id) => Some(id.as_str()),
            _ => None,
        }
    }
}

/// The shipped default when a scope has never expressed a primary choice.
/// Kept here rather than importing Biorouter's Soul module because the MCP
/// crate is below the application crate in the dependency graph.
///
/// `server::kb_delete_base` refuses this id, and the refusal must name the SAME
/// base this file resolves an unexpressed primary to. A second `"soul"` literal
/// over there is how the model-facing refusal and the product default come to
/// disagree after someone renames one of them.
///
/// ⚠ `pub`, so `biorouter::knowledge::soul::SOUL_KB_ID` — the id the SEEDER
/// writes, one crate up — can be defined *as* this rather than beside it. It
/// was a second `"soul"` literal in a different crate with nothing pinning the
/// two equal, and the pair a delete guard cannot afford to have drift is
/// exactly "the id that is refused" and "the id that is created".
pub const DEFAULT_PRIMARY_KB_ID: &str = "soul";

/// "No primary at this scope" is always an explicit blank-file choice. At
/// machine scope an absent file now means "use the product default Soul", so
/// removing the file would incorrectly undo a user's explicit Clear.
fn no_primary_for(_session_id: Option<&str>) -> StoredPrimary {
    StoredPrimary::NoPrimary
}

impl KnowledgeService {
    fn primary_session_path(&self, session_id: &str) -> PathBuf {
        let digest = raw::hash_bytes(session_id.as_bytes());
        paths::primary_kb_sessions_dir(self.root()).join(digest)
    }

    fn hidden_session_path(&self, session_id: &str) -> PathBuf {
        let digest = raw::hash_bytes(session_id.as_bytes());
        paths::hidden_kb_sessions_dir(self.root()).join(digest)
    }

    /// Read a primary-pointer file as the tri-state it actually is: absent ⇒
    /// `Inherit`, blank ⇒ `NoPrimary`, otherwise the bare kb id it holds. The
    /// on-disk format is one bare kb id — see [`paths::primary_kb_path`].
    fn read_primary_file_unlocked(&self, path: &Path) -> anyhow::Result<StoredPrimary> {
        let s = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(StoredPrimary::Inherit);
            }
            Err(err) => return Err(err.into()),
        };
        let trimmed = s.trim();
        if trimmed.is_empty() {
            Ok(StoredPrimary::NoPrimary)
        } else {
            Ok(StoredPrimary::Pinned(trimmed.to_string()))
        }
    }

    /// The pinned id, or `None` for either of the two "no id" states. Callers
    /// that must tell `Inherit` from `NoPrimary` read the tri-state directly.
    fn get_primary_path_unlocked(&self, path: &Path) -> anyhow::Result<Option<String>> {
        Ok(self
            .read_primary_file_unlocked(path)?
            .pinned()
            .map(ToOwned::to_owned))
    }

    /// Persist one of the three states. `NoPrimary` writes a *blank file* — it
    /// is a real, durable override, so it must leave something on disk that a
    /// later read can tell apart from "never chose".
    fn write_primary_file_unlocked(
        &self,
        path: &Path,
        value: &StoredPrimary,
    ) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        match value {
            StoredPrimary::Pinned(id) => {
                crate::knowledge::paths::validate_kb_id(id)?;
                let tmp = path.with_extension("tmp");
                std::fs::write(&tmp, id.as_bytes())?;
                std::fs::rename(tmp, path)?;
            }
            StoredPrimary::NoPrimary => {
                let tmp = path.with_extension("tmp");
                std::fs::write(&tmp, b"")?;
                std::fs::rename(tmp, path)?;
            }
            StoredPrimary::Inherit => {
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
            }
        }

        Ok(())
    }

    fn get_hidden_path_unlocked(&self, path: &Path) -> anyhow::Result<Vec<String>> {
        if !path.exists() {
            return Ok(Vec::new());
        }

        let s = std::fs::read_to_string(path)?;
        if s.trim().is_empty() {
            return Ok(Vec::new());
        }

        let mut hidden = serde_json::from_str::<Vec<String>>(&s)?;
        hidden.sort();
        hidden.dedup();
        Ok(hidden)
    }

    /// Normalise and validate a caller-supplied list of kb ids — trim, drop
    /// blanks, sort, dedupe, reject malformed ids. Kept separate from the write
    /// so a request can be rejected in full *before* anything touches disk.
    fn sanitize_kb_id_list(ids: &[String]) -> anyhow::Result<Vec<String>> {
        let mut sanitized = ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        sanitized.sort();
        sanitized.dedup();

        for id in &sanitized {
            crate::knowledge::paths::validate_kb_id(id)?;
        }

        Ok(sanitized)
    }

    /// Persist an already-sanitized hidden list. Infallible except for I/O, so
    /// it is safe to call once every decision in an operation has been made.
    fn write_hidden_file_unlocked(&self, path: &Path, ids: &[String]) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // An empty list is written, not deleted: `get_hidden_for_session_or_persisted`
        // discriminates on file *existence*, so `[]` is how a session says
        // "I override, and I hide nothing". Deleting the file here made that
        // state unrepresentable and silently re-inherited the machine default.
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec(ids)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    fn set_hidden_path_unlocked(&self, path: &Path, ids: &[String]) -> anyhow::Result<()> {
        let sanitized = Self::sanitize_kb_id_list(ids)?;
        self.write_hidden_file_unlocked(path, &sanitized)
    }

    fn rewrite_hidden_path_refs_unlocked(
        &self,
        path: &Path,
        current_kb_id: &str,
        next_kb_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let hidden = self.get_hidden_path_unlocked(path)?;
        if hidden.is_empty() || !hidden.iter().any(|id| id == current_kb_id) {
            return Ok(());
        }

        let mut next_hidden = hidden
            .into_iter()
            .filter(|id| id != current_kb_id)
            .collect::<Vec<_>>();
        if let Some(next_kb_id) = next_kb_id {
            next_hidden.push(next_kb_id.to_string());
        }
        self.set_hidden_path_unlocked(path, &next_hidden)
    }

    /// Follow a rename, or absorb a delete, in every session's primary file.
    ///
    /// `next_kb_id = None` means the base is gone. That must leave the session
    /// with an explicit **no primary** (a blank file), not with no file at all:
    /// deleting the file is the encoding of "this session never chose", so a
    /// session that had deliberately pinned the deleted base would come back
    /// pointed at the machine-wide default it never asked for.
    fn rewrite_session_primary_refs_unlocked(
        &self,
        current_kb_id: &str,
        next_kb_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let dir = paths::primary_kb_sessions_dir(self.root());
        if !dir.exists() {
            return Ok(());
        }

        let next = match next_kb_id {
            Some(id) => StoredPrimary::Pinned(id.to_string()),
            None => StoredPrimary::NoPrimary,
        };

        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }
            if !is_session_digest(&entry.file_name()) {
                continue;
            }

            if self.read_primary_file_unlocked(&path)?.pinned() == Some(current_kb_id) {
                self.write_primary_file_unlocked(&path, &next)?;
            }
        }

        Ok(())
    }

    fn rewrite_hidden_refs_unlocked(
        &self,
        current_kb_id: &str,
        next_kb_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let global_path = paths::hidden_kbs_path(self.root());
        self.rewrite_hidden_path_refs_unlocked(&global_path, current_kb_id, next_kb_id)?;

        let dir = paths::hidden_kb_sessions_dir(self.root());
        if !dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() || !is_session_digest(&entry.file_name()) {
                continue;
            }
            self.rewrite_hidden_path_refs_unlocked(&entry.path(), current_kb_id, next_kb_id)?;
        }

        Ok(())
    }

    fn slugify_kb_name(name: &str) -> String {
        let mut slug = String::with_capacity(name.len());
        let mut last_was_dash = false;

        for ch in name.chars().flat_map(|c| c.to_lowercase()) {
            if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
                slug.push(ch);
                last_was_dash = false;
            } else if !slug.is_empty() && !last_was_dash {
                slug.push('-');
                last_was_dash = true;
            }
        }

        while slug.ends_with('-') {
            slug.pop();
        }

        slug
    }

    pub fn new(root: PathBuf) -> Self {
        let svc = Self {
            root,
            locks: Arc::new(DashMap::new()),
        };
        if let Err(e) = svc.resume_pending_import_cleanup() {
            tracing::warn!("knowledge: could not recover an interrupted base publication: {e:#}");
        }
        // Issue #56. Best-effort: `new` is infallible and a failure here must
        // not stop the app from opening. A root that never migrates reads every
        // base PUBLIC (the file is absent ⇒ "not migrated"), which is AR-2's
        // accepted direction, not a new one.
        if let Err(e) = svc.ensure_tiers_migrated() {
            tracing::warn!("knowledge: could not migrate kb tiers: {e:#}");
        }
        svc
    }

    pub fn new_default() -> Result<Self> {
        Ok(Self::new(paths::knowledge_root()?))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn root_lock_path(&self) -> PathBuf {
        self.root.join(".knowledge-root.lock")
    }

    /// ⚠ **Beside the base, never inside it** — see
    /// [`paths::kb_write_lock_path`]. Holding a handle to a file under
    /// `kb_root` makes Windows refuse to rename or remove `kb_root`, and both
    /// the delete transaction and a base rename do exactly that with this lock
    /// held.
    fn kb_lock_path(&self, kb_id: &str) -> PathBuf {
        paths::kb_write_lock_path(&self.root, kb_id)
    }

    /// Carry a base's write lock across a rename, so the guard its renamer is
    /// holding keeps excluding writers of the base under its new id.
    ///
    /// A missing source is not an error: the lock file only exists once
    /// somebody has taken the lock, and a base that has never been locked has
    /// nothing to move.
    fn rename_kb_lock(&self, from_id: &str, to_id: &str) -> Result<()> {
        let from = self.kb_lock_path(from_id);
        if !from.exists() {
            return Ok(());
        }
        let to = self.kb_lock_path(to_id);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&from, &to)
            .with_context(|| format!("move the knowledge write lock from '{from_id}' to '{to_id}'"))
    }

    /// Transaction lock for synchronous mutations of an existing base. Call
    /// this before `lock_root`: macros already hold the KB lock when they take
    /// the privacy/registry lock, so KB then root is the only deadlock-safe
    /// order shared by both paths.
    fn lock_existing_kb(&self, kb_id: &str) -> Result<FileLockGuard> {
        paths::validate_kb_id(kb_id)?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        if !kb_root.exists() {
            anyhow::bail!("kb '{kb_id}' not found");
        }
        // #157: bounded, not blocking. `acquire` uses `flock` with no deadline, so
        // a lock nobody releases turned every later write to this base into a
        // call that never returned.
        let guard = FileLockGuard::acquire_bounded(
            &self.kb_lock_path(kb_id),
            FileLockGuard::KB_WRITE_LOCK_WAIT,
            &format!("knowledge base '{kb_id}'"),
        )
        .map_err(|error| {
            if !kb_root.exists() {
                anyhow::anyhow!("kb '{kb_id}' not found")
            } else {
                error
            }
        })?;
        if kb_root.join(".git").is_dir() {
            GitRepo::open(&kb_root)?.recover_orphaned_txn()?;
        }
        Ok(guard)
    }

    fn lock_root(&self) -> Result<FileLockGuard> {
        FileLockGuard::acquire(&self.root_lock_path())
    }

    fn lock_root_cancellable(&self, cancel: Option<&CancellationToken>) -> Result<FileLockGuard> {
        match cancel {
            Some(cancel) => FileLockGuard::acquire_cancellable(&self.root_lock_path(), cancel),
            None => self.lock_root(),
        }
    }

    /// Take the root lock and raise `kb_id` on **both** of issue #56's axes: to
    /// the caller's tier, and to the set of institutions whose content it holds
    /// (DR-26 / Task 50 Step 1).
    ///
    /// For callers OUTSIDE this module. Inside it — `create_base`,
    /// `import_brkb`, `delete_base` — the lock is already held, so those call
    /// `tier::*_unlocked` directly (through [`Self::stamp_base_unlocked`],
    /// which is this function's unlocked twin). Calling this from there
    /// deadlocks.
    ///
    /// ⚠ **One method rather than two that callers are asked to pair.** It was
    /// two, and review found the consequences: the five production call sites
    /// paired them by convention, in two different orders (so a failure between
    /// them left a *public* base carrying an owner at one site and a claimed
    /// base at public tier at another), each taking and releasing the root lock
    /// separately. And a caller that raised only the tier would put an
    /// institution's content into a base no institution is recorded as owning —
    /// which reads as unclaimed and is therefore reachable from every other
    /// institution's model. Neither is expressible now: there is one call, one
    /// lock, one order.
    pub fn raise_tier_and_affiliation(
        &self,
        kb_id: &str,
        caller_is_private: bool,
        caller: &crate::knowledge::affiliation::CallerAffiliation,
    ) -> Result<()> {
        self.raise_tier_and_affiliation_under_root_lock(kb_id, caller_is_private, caller, None)
    }

    fn raise_tier_and_affiliation_under_root_lock(
        &self,
        kb_id: &str,
        caller_is_private: bool,
        caller: &crate::knowledge::affiliation::CallerAffiliation,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        let _lock = self.lock_root_cancellable(cancel)?;
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("knowledge privacy ratchet cancelled before mutation");
        }
        let kb_root = paths::kb_root(&self.root, kb_id);
        if !kb_root.exists() {
            anyhow::bail!("kb '{kb_id}' not found");
        }
        manifest::load(&kb_root)?;
        crate::knowledge::tier::stamp_unlocked(
            &self.root,
            kb_id,
            caller_is_private,
            crate::knowledge::affiliation::contributed_owners(caller),
        )
    }

    /// Async entry point for the privacy ratchet. The root's process-wide file
    /// lock may wait behind another process, so async callers must not acquire
    /// it on a Tokio worker.
    pub async fn raise_tier_and_affiliation_async(
        &self,
        kb_id: &str,
        caller_is_private: bool,
        caller: &crate::knowledge::affiliation::CallerAffiliation,
    ) -> Result<()> {
        self.raise_tier_and_affiliation_cancelled_by(kb_id, caller_is_private, caller, None)
            .await
    }

    pub async fn raise_tier_and_affiliation_cancelled_by(
        &self,
        kb_id: &str,
        caller_is_private: bool,
        caller: &crate::knowledge::affiliation::CallerAffiliation,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        let operation_cancel = cancel
            .map(CancellationToken::child_token)
            .unwrap_or_default();
        let cancel_operation_on_drop = CancelOnDrop(operation_cancel.clone());
        let svc = self.clone();
        let kb_id = kb_id.to_string();
        let caller = caller.clone();
        let operation_cancel_for_task = operation_cancel.clone();
        let result = tokio::task::spawn_blocking(move || {
            svc.raise_tier_and_affiliation_under_root_lock(
                &kb_id,
                caller_is_private,
                &caller,
                Some(&operation_cancel_for_task),
            )
        })
        .await
        .map_err(|error| anyhow::anyhow!("knowledge privacy ratchet task failed: {error}"))?;
        drop(cancel_operation_on_drop);
        result
    }

    /// Stamp a base on **both** of issue #56's axes from an explicit owner set,
    /// inside a root lock the caller already holds.
    ///
    /// ⚠ **The pairing is this function, not a convention.** `create_base_as`
    /// and `import_brkb` are the two tools whose subject id is minted by the
    /// call, so neither can go through the `call_tool` seam that pairs the two
    /// raises for the other seventeen — and neither is visible to
    /// `server::tests::every_tool_that_ratchets_the_tier_also_records_the_callers_institution`,
    /// which is parameterised over a base that already exists. Task 50 shipped
    /// with both of them raising the tier alone: the affiliation axis was
    /// laundered by `kb_export` + `kb_import`, both endpoints Private, no gate
    /// crossed. One function that does both is what stops that from being
    /// re-introduced by someone reading only one of the call sites.
    ///
    /// ⚠ **It was named `stamp_new_base_unlocked` and it is not only for new
    /// bases.** [`Self::absorb_classification`] is the third caller: a merge
    /// destination already exists, and what it needs is exactly this — a raise
    /// on the tier axis and a UNION on the owner axis, from an owner set the
    /// call supplies rather than derives from one caller.
    /// [`Self::raise_tier_and_affiliation`] cannot serve it, because the owners
    /// being folded in are the **source base's** and there is no single caller
    /// to derive them from — the same reason `tier::add_owners_unlocked` exists
    /// beside `tier::raise_affiliation_unlocked`.
    fn stamp_base_unlocked(
        &self,
        kb_id: &str,
        caller_is_private: bool,
        owners: std::collections::BTreeSet<String>,
    ) -> Result<()> {
        crate::knowledge::tier::stamp_unlocked(&self.root, kb_id, caller_is_private, owners)
    }

    /// Take the root lock and SET `kb_id`'s tier on the user's behalf (issue #56
    /// DR-18) — the one call in the tree that can lower one.
    ///
    /// It sits beside [`Self::raise_tier`] and shares its lock discipline and its
    /// deadlock rule: never call it from `create_base` / `import_brkb` /
    /// `delete_base`, which are already inside `lock_root()`.
    ///
    /// The `&UserKbTierChange` is a proof-of-user with a private field. This
    /// wrapper can accept one and cannot make one — the only construction site
    /// in the tree is the HTTP handler behind the user-action header, pinned by
    /// `tier_user::tests::the_proof_of_user_is_constructed_in_exactly_one_place`.
    pub fn set_tier_by_user(
        &self,
        kb_id: &str,
        tier: crate::knowledge::types::KbTier,
        ok: &crate::knowledge::tier_user::UserKbTierChange,
    ) -> Result<()> {
        let _kb_lock = self.lock_existing_kb(kb_id)?;
        self.set_tier_by_user_under_kb_lock(kb_id, tier, ok, None)
    }

    pub async fn set_tier_by_user_async(
        &self,
        kb_id: &str,
        tier: crate::knowledge::types::KbTier,
        ok: crate::knowledge::tier_user::UserKbTierChange,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        let lock_id = kb_id.to_string();
        let kb_id = lock_id.clone();
        self.run_existing_kb_mutation(&lock_id, cancel, move |svc, cancel| {
            svc.set_tier_by_user_under_kb_lock(&kb_id, tier, &ok, Some(cancel))
        })
        .await
    }

    fn set_tier_by_user_under_kb_lock(
        &self,
        kb_id: &str,
        tier: crate::knowledge::types::KbTier,
        ok: &crate::knowledge::tier_user::UserKbTierChange,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        let _lock = self.lock_root_cancellable(cancel)?;
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("knowledge tier change cancelled before writing its classification");
        }
        crate::knowledge::tier_user::set_unlocked(&self.root, kb_id, tier, ok)
    }

    /// The mirror of [`Self::raise_tier`], for a base that has gone away.
    pub fn forget_tier(&self, kb_id: &str) -> Result<()> {
        let _lock = self.lock_root()?;
        crate::knowledge::tier::forget_unlocked(&self.root, kb_id)
    }

    /// Idempotent, and cheap on the common path: it stats `.kb-tiers` BEFORE
    /// taking the lock and returns immediately when it exists, so the ~90
    /// `KnowledgeService::new` calls in the test suite do not each `flock`.
    fn ensure_tiers_migrated(&self) -> Result<()> {
        if crate::knowledge::paths::kb_tiers_path(&self.root).exists() {
            return Ok(());
        }
        if !self.root.exists() {
            return Ok(()); // no bases yet; the first create_base registers
        }
        let _lock = self.lock_root()?;
        crate::knowledge::tier::ensure_migrated_unlocked(&self.root)
    }

    /// Acquire an exclusive lock for `kb_id`. Held until the returned guard is dropped.
    /// Used by macros to serialize concurrent writers against the same KB.
    ///
    /// The in-process mutex is awaited first, so at most one blocking-pool task
    /// per service and KB can wait in the cross-process `flock`. File acquisition
    /// runs in `spawn_blocking`, so neither Tokio workers nor the rest of this
    /// process's queue are held hostage by a process outside Biorouter.
    pub async fn lock_kb(&self, kb_id: &str) -> Result<KnowledgeWriteGuard> {
        self.lock_kb_cancellable(kb_id, None).await
    }

    #[cfg(test)]
    pub(crate) fn kb_queue_is_occupied(&self, kb_id: &str) -> bool {
        self.locks
            .get(kb_id)
            .is_some_and(|slot| Arc::clone(slot.value()).try_lock_owned().is_err())
    }

    /// [`Self::lock_kb`] with level-triggered cancellation while queued on
    /// either the in-process mutex or the cross-process file lock.
    pub async fn lock_kb_cancellable(
        &self,
        kb_id: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<KnowledgeWriteGuard> {
        self.lock_kb_path_cancellable(kb_id, cancel).await
    }

    /// [`Self::lock_kb_cancellable`] at the READ deadline.
    ///
    /// Same lock, same queue, same fairness — only the patience differs. It
    /// exists because the read/write split (#159) was applied at the tool
    /// boundary, and the two *macros* decide read-vs-write from an argument
    /// (`lint`'s `autofix`, `query`'s `file_as_page`) that is known one line
    /// before the lock is taken. Without this they took the 1800s write
    /// deadline for the read case, which is the whole defect: the Knowledge
    /// view's "Check for problems" never sends `autofix`, so the GUI's lint is
    /// ALWAYS read-only and parked for thirty minutes behind any open
    /// transaction, while `kb_lint` on the same base refused in 45 seconds.
    pub async fn lock_kb_cancellable_for_read(
        &self,
        kb_id: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<KnowledgeWriteGuard> {
        self.lock_kb_path_waiting(kb_id, cancel, FileLockGuard::KB_READ_LOCK_WAIT)
            .await
    }

    /// [`Self::lock_existing_kb_cancellable`] for a READ path: same queue, same
    /// fairness, [`FileLockGuard::KB_READ_LOCK_WAIT`] instead of the write
    /// deadline. See that constant for why the two differ (#159).
    pub(crate) async fn lock_existing_kb_for_read(
        &self,
        kb_id: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<KnowledgeWriteGuard> {
        self.lock_existing_kb_waiting(kb_id, cancel, FileLockGuard::KB_READ_LOCK_WAIT)
            .await
    }

    pub(crate) async fn lock_existing_kb_cancellable(
        &self,
        kb_id: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<KnowledgeWriteGuard> {
        self.lock_existing_kb_waiting(kb_id, cancel, FileLockGuard::KB_WRITE_LOCK_WAIT)
            .await
    }

    async fn lock_existing_kb_waiting(
        &self,
        kb_id: &str,
        cancel: Option<&CancellationToken>,
        wait: std::time::Duration,
    ) -> Result<KnowledgeWriteGuard> {
        paths::validate_kb_id(kb_id)?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        if !kb_root.exists() {
            anyhow::bail!("kb '{kb_id}' not found");
        }
        self.lock_kb_path_waiting(kb_id, cancel, wait)
            .await
            .map_err(|error| {
                if !kb_root.exists() {
                    anyhow::anyhow!("kb '{kb_id}' not found")
                } else {
                    error
                }
            })
    }

    /// ⚠ `validate_kb_id` FIRST, and not only for the callers that already do.
    /// The lock's filename is now the id ([`paths::kb_write_lock_path`]), so an
    /// id that is not a plain slug is a path, and `lock_kb` is reachable
    /// straight from an HTTP handler with a caller-supplied one.
    async fn lock_kb_path_cancellable(
        &self,
        kb_id: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<KnowledgeWriteGuard> {
        self.lock_kb_path_waiting(kb_id, cancel, FileLockGuard::KB_WRITE_LOCK_WAIT)
            .await
    }

    /// [`Self::lock_kb_path_cancellable`] with the queue deadline supplied.
    ///
    /// The deadline is a parameter for ONE reason: so the tests can prove it
    /// actually fires. The shipped value is 30 minutes (it has to clear the
    /// longest legitimate macro), which is not a duration a test can wait out,
    /// and driving `tokio::time` forward instead needs the `test-util` feature
    /// this crate does not enable. Threading it through is the alternative to a
    /// bound nothing exercises — and both arms take the same deadline, so a test
    /// that drives one is testing the code path the other uses.
    async fn lock_kb_path_waiting(
        &self,
        kb_id: &str,
        cancel: Option<&CancellationToken>,
        wait: std::time::Duration,
    ) -> Result<KnowledgeWriteGuard> {
        paths::validate_kb_id(kb_id)?;
        let m = self
            .locks
            .entry(kb_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        // #157: BOUNDED — and on BOTH arms. This in-process mutex, not the file
        // lock below, is what matched every symptom measured in a live daemon:
        // writes to one base stopped answering while reads and other bases were
        // fine, and only a restart cleared it (a file lock would have survived
        // one; an in-process `Mutex` does not). A holder that never releases
        // makes every later caller here await forever.
        //
        // ⚠ Two ways to put this fix in the wrong place, both of which I did:
        //
        //  1. Bounding `lock_existing_kb`'s FILE lock instead. It compiled, its
        //     unit test passed, and it changed nothing — the MCP tools reach a
        //     base through THIS function. Measured: with only that bound in
        //     place, `kb_get_graph` still returned nothing at 200 s.
        //  2. Bounding only the `None` arm. Nearly every tool handler in
        //     `server.rs` passes `Some(context.ct)`, so the arm that looks like
        //     the exception is the one the whole surface actually takes; a
        //     cancel token is an escape only if somebody cancels.
        //
        // Evidence, both halves: the DIAGNOSIS came from the running app — a live
        // daemon, a real wedge, and a `kb_get_graph` that answered at the deadline
        // instead of never — which is what caught (1) being on the wrong wait
        // while its unit test was passing. The BEHAVIOUR is covered here, by tests
        // that drive `lock_kb_path_waiting` with a short deadline on each arm.
        let queued = async {
            match cancel {
                Some(cancel) => tokio::select! {
                    biased;
                    () = cancel.cancelled() => None,
                    guard = m.lock_owned() => Some(guard),
                },
                None => Some(m.lock_owned().await),
            }
        };
        let process_guard = match tokio::time::timeout(wait, queued).await {
            Ok(Some(guard)) => guard,
            Ok(None) => {
                anyhow::bail!("knowledge operation cancelled while waiting for the KB lock")
            }
            // Both causes named, because this message cannot tell them apart and
            // a confident wrong diagnosis is worse than an honest fork.
            Err(_) => anyhow::bail!(
                "timed out after {}s waiting for knowledge base '{kb_id}'. Another \
                     operation still holds it. Most often that is an OPEN TRANSACTION: \
                     commit or abort it (kb_commit_txn / kb_abort_txn) and this will go \
                     through. Otherwise an ingest, query or lint is running — let it finish \
                     and retry. Nothing was written.",
                wait.as_secs()
            ),
        };
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("knowledge operation cancelled after acquiring the KB queue lock");
        }

        let lock_path = self.kb_lock_path(kb_id);
        let kb_root = paths::kb_root(&self.root, kb_id);
        let waiter_cancel = cancel
            .map(CancellationToken::child_token)
            .unwrap_or_default();
        let cancel_waiter_on_drop = CancelOnDrop(waiter_cancel.clone());
        let acquire = tokio::task::spawn_blocking(move || {
            let file_guard = FileLockGuard::acquire_cancellable(&lock_path, &waiter_cancel)?;
            if kb_root.join(".git").is_dir() {
                GitRepo::open(&kb_root)?.recover_orphaned_txn()?;
            }
            Ok::<_, anyhow::Error>(file_guard)
        });
        // #157: BOUNDED, and this is the half that actually wedges. Proven on a
        // live daemon: with nothing running, an outside `flock(LOCK_EX|LOCK_NB)`
        // on the base's lock file is REFUSED, and `sample` shows every waiter
        // parked in `acquire_cancellable` -> `fs2::unix::flock` -> the syscall,
        // having already passed the in-process mutex above. So a leaked
        // `FileLockGuard` — not a busy mutex — is what turns later operations on
        // that base into calls that never return, until the process exits.
        //
        // ⚠ I twice reported the in-process mutex as the mechanism. It can wedge
        // too, and it is bounded above, but the file lock is a SECOND unbounded
        // wait on the same path, and bounding only one leaves the hang intact.
        // A cancel token is not a deadline: `acquire_cancellable` waits forever
        // while nobody cancels.
        //
        // On elapse, drop `cancel_waiter_on_drop` FIRST. That cancels the token
        // the blocking task is polling, so it stops waiting on `flock` and the
        // thread returns to the pool instead of being pinned for the life of the
        // process — bounding the caller while leaking a blocking thread per
        // attempt would trade one exhaustion for another.
        let file_guard = match tokio::time::timeout(wait, acquire).await {
            Ok(joined) => joined
                .map_err(|error| anyhow::anyhow!("knowledge KB lock task failed: {error}"))??,
            Err(_) => {
                drop(cancel_waiter_on_drop);
                anyhow::bail!(
                    "timed out after {}s waiting for the on-disk lock of knowledge base \
                     '{kb_id}'. Another operation still holds it. Most often that is an OPEN \
                     TRANSACTION: commit or abort it (kb_commit_txn / kb_abort_txn) and this \
                     will go through. Otherwise an ingest, query or lint is running — let it \
                     finish and retry. Nothing was written.",
                    wait.as_secs()
                );
            }
        };
        drop(cancel_waiter_on_drop);
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("knowledge operation cancelled after acquiring the KB lock");
        }
        Ok(KnowledgeWriteGuard {
            _process_guard: process_guard,
            _file_guard: file_guard,
        })
    }

    async fn run_existing_kb_mutation<T, F>(
        &self,
        kb_id: &str,
        cancel: Option<&CancellationToken>,
        operation: F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&KnowledgeService, &CancellationToken) -> Result<T> + Send + 'static,
    {
        self.run_existing_kb_mutation_waiting(
            kb_id,
            cancel,
            FileLockGuard::KB_WRITE_LOCK_WAIT,
            operation,
        )
        .await
    }

    /// [`Self::run_existing_kb_mutation`], with the caller choosing how long to
    /// wait for the base. Only a mutation that loses nothing by giving up may
    /// shorten it — see [`FileLockGuard::KB_DELETE_LOCK_WAIT`].
    async fn run_existing_kb_mutation_waiting<T, F>(
        &self,
        kb_id: &str,
        cancel: Option<&CancellationToken>,
        wait: std::time::Duration,
        operation: F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&KnowledgeService, &CancellationToken) -> Result<T> + Send + 'static,
    {
        let operation_cancel = cancel
            .map(CancellationToken::child_token)
            .unwrap_or_default();
        let cancel_operation_on_drop = CancelOnDrop(operation_cancel.clone());
        let guard = self
            .lock_existing_kb_waiting(kb_id, Some(&operation_cancel), wait)
            .await?;
        let svc = self.clone();
        let operation_cancel_for_task = operation_cancel.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            operation(&svc, &operation_cancel_for_task)
        })
        .await
        .map_err(|error| anyhow::anyhow!("knowledge mutation task failed: {error}"))?;
        drop(cancel_operation_on_drop);
        result
    }

    /// Create a base on behalf of the user (no model involved), so it is born
    /// PUBLIC and unclaimed. Model-facing callers use [`Self::create_base_as`]
    /// instead.
    pub fn create_base(&self, id: &str, name: &str, color: Option<&str>) -> Result<Manifest> {
        self.create_base_in(id, name, color, KbFormat::default())
    }

    /// [`Self::create_base`] with an explicit profile. Same user-facing,
    /// born-PUBLIC path; the profile is the one thing about a base that cannot
    /// be changed afterwards without a format migration, so it has to be
    /// choosable at the only moment it is free.
    pub fn create_base_in(
        &self,
        id: &str,
        name: &str,
        color: Option<&str>,
        format: KbFormat,
    ) -> Result<Manifest> {
        self.create_base_as(
            id,
            name,
            color,
            format,
            false,
            &crate::knowledge::affiliation::CallerAffiliation::Unstated,
        )
    }

    /// Create a base and stamp it with the creating session's tier, both inside
    /// **one** root-lock transaction (issue #56).
    ///
    /// `create_base` + a separate `raise_tier` would be two transactions with a
    /// window between them in which a lock-free `is_private` reader sees a
    /// private session's brand-new base as PUBLIC, and in which a failing raise
    /// returns `Err` to the caller while a PUBLIC base persists on disk. That is
    /// the same reasoning that keeps `import_brkb`'s stamp inside its own single
    /// store write, applied to the other tool whose subject id does not exist
    /// before the call.
    ///
    /// `caller_affiliation` is stamped in the same transaction and for the same
    /// reason (issue #56 DR-26 / Task 50): the tier is decided **at creation**,
    /// not at the first ingest (DR-18(b)), and a base born in a UCSF chat that
    /// recorded no owner would read as unclaimed — reachable from every other
    /// institution's model — until something happened to write into it.
    ///
    /// `format` picks the profile (DR-6) and therefore the scaffolded tree and
    /// the `schema.md` written into it. It rides in the same transaction for a
    /// duller reason than the tier does: the manifest, the directories and the
    /// schema are three statements about one base and a half-written base whose
    /// manifest says `biookf` over an OKF tree would teach the sub-agent the
    /// wrong format for the rest of its life.
    ///
    /// ⚠ **Do not add a second transaction here, and do not call
    /// [`Self::lock_root`] from anything this calls.** The lock is *not*
    /// re-entrant — that is why the `_unlocked` twins exist — so a helper that
    /// takes it deadlocks the creation of every base.
    pub fn create_base_as(
        &self,
        id: &str,
        name: &str,
        color: Option<&str>,
        format: KbFormat,
        caller_is_private: bool,
        caller_affiliation: &crate::knowledge::affiliation::CallerAffiliation,
    ) -> Result<Manifest> {
        self.create_base_as_with_checkpoint(
            CreateBaseSpec {
                id,
                name,
                color,
                format,
            },
            caller_is_private,
            caller_affiliation,
            |_| Ok(()),
        )
    }

    /// Refuse an id the registry already holds, distinguishing a live row from an
    /// **orphan** one.
    ///
    /// #158: a bare "already registered" is a dead end when the directory is gone
    /// — `kb_list_bases` does not show the base, so the id can be neither seen,
    /// read, deleted nor re-created. Naming the stale row gives the refusal
    /// somewhere to point. (`registry::register` carries the same distinction for
    /// its own callers; this exists because create refuses here first and never
    /// reaches it.)
    ///
    /// ⚠ **It names the registry FILE, not its absolute path** (adversarial
    /// security review 2026-09-12, MEDIUM), for the same reason the
    /// already-exists bail alongside it stopped naming one: this string is a
    /// `POST /knowledge/bases` response body. The file sits in the knowledge
    /// root, which whoever can act on the message already has.
    ///
    /// Lifted out of [`Self::create_base_as_with_checkpoint`] rather than left
    /// inline: spelling the message this carefully pushed that function past
    /// `clippy::too_many_lines`, and a self-contained refusal is the part of it
    /// that was never about creating anything.
    fn refuse_if_the_id_is_registered(&self, id: &str) -> Result<()> {
        let Some(stale) = registry::load(&self.root)?
            .into_iter()
            .find(|entry| entry.id == id)
        else {
            return Ok(());
        };
        if stale.path.exists() {
            anyhow::bail!("kb-id '{id}' already registered");
        }
        anyhow::bail!(
            "kb-id '{id}' is registered but its directory is missing. The row is stale, \
             which is why this id is neither listed nor creatable. Remove the '{id}' entry \
             from '{}' in your Biorouter knowledge directory to free the id.",
            registry::registry_path(&self.root)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "the knowledge registry".to_string()),
        );
    }

    fn create_base_as_with_checkpoint(
        &self,
        spec: CreateBaseSpec<'_>,
        caller_is_private: bool,
        caller_affiliation: &crate::knowledge::affiliation::CallerAffiliation,
        mut checkpoint: impl FnMut(CreateCheckpoint) -> Result<()>,
    ) -> Result<Manifest> {
        let CreateBaseSpec {
            id,
            name,
            color,
            format,
        } = spec;
        let _lock = self.lock_root()?;
        paths::validate_kb_id(id)?;
        let kb_root = paths::kb_root(&self.root, id);
        if kb_root.exists() {
            // ⚠ **No path in this message** (adversarial security review
            // 2026-09-12, MEDIUM). `POST /knowledge/bases` returns whatever this
            // says verbatim, and it used to end `… at
            // /Users/<user>/.config/biorouter/knowledge/<id>` — the machine's
            // absolute config path, handed to whoever asked. The caller supplied
            // the id and already knows the root if it is entitled to know
            // anything here, so the path added nothing but the disclosure. The
            // route's own gate is what stops an unentitled caller reaching this
            // line at all; this is the second layer.
            anyhow::bail!("kb '{id}' already exists");
        }
        let metadata = BasePublicationSnapshot::capture(&self.root)?;
        self.refuse_if_the_id_is_registered(id)?;
        let staged_root = self
            .root
            .join(format!(".creating-{id}-{}", uuid::Uuid::new_v4()));
        let mutation = (|| -> Result<Manifest> {
            if crate::knowledge::tier::has_metadata_unlocked(&self.root, id)? {
                crate::knowledge::tier::forget_unlocked(&self.root, id)?;
            }
            // The four hardcoded directories this replaced (`entities`, `concepts`,
            // `sources`, `notes`) were the pre-OKF taxonomy, encoded in a `kind:`
            // frontmatter key. Under OKF the axis is `type` and the layout is the
            // producer's — see `scaffold_dirs`, including why every one of these is
            // still under `knowledge/`.
            let knowledge_dir = staged_root.join("knowledge");
            std::fs::create_dir_all(&knowledge_dir)?;
            for dir in scaffold_dirs(format) {
                std::fs::create_dir_all(knowledge_dir.join(dir))?;
            }
            std::fs::create_dir_all(staged_root.join("raw"))?;
            std::fs::create_dir_all(staged_root.join(".biorouter-knowledge"))?;

            let m = Manifest {
                id: id.to_string(),
                name: name.to_string(),
                color: color.unwrap_or("#5a6394").to_string(),
                created_at: Utc::now(),
                // The generation of the `schema.md` written below, not a constant 1.
                // A manifest that under-reports what its own base carries makes the
                // migration ladder run on a base that is already current, which is
                // only harmless for as long as every step happens to be idempotent.
                schema_version: CURRENT_SCHEMA_VERSION,
                default_model: None,
                format,
                // Written for BOTH profiles, because a BioOKF bundle is an OKF
                // bundle — the profile only adds constraints. It mirrors what
                // `index_scaffold` puts in the bundle-root `index.md`, which is the
                // one place OKF permits it.
                okf_version: Some(okf::OKF_VERSION.to_string()),
                // …and this one is here rather than in `index.md` (DR-23's
                // corollary): OKF §8 permits `okf_version` there and nothing else.
                biookf_version: format
                    .is_biookf()
                    .then(|| biookf::BIOOKF_VERSION.to_string()),
            };
            manifest::save(&staged_root, &m)?;

            std::fs::write(staged_root.join("schema.md"), schema_for(format))?;
            std::fs::write(staged_root.join("index.md"), index_scaffold())?;
            std::fs::write(staged_root.join("log.md"), DEFAULT_LOG)?;
            std::fs::write(staged_root.join(".gitignore"), GITIGNORE)?;
            checkpoint(CreateCheckpoint::Files)?;

            let repo = GitRepo::init(&staged_root)?;
            repo.commit_all(
                crate::knowledge::types::ChangeKind::Manual,
                &format!("create knowledge base {id}"),
                None,
            )
            .context("initial commit")?;
            checkpoint(CreateCheckpoint::Repository)?;

            let graph = crate::knowledge::graph::derive(&staged_root)?;
            crate::knowledge::graph::write_cache(&staged_root, &graph)?;
            checkpoint(CreateCheckpoint::GraphCache)?;
            // Issue #56, decision (5a). A base with no entry reads PRIVATE
            // (unknown provenance), so an unregistered base would lock its own
            // creator out. `raise_unlocked` registers an absent id at the caller's
            // tier and can never lower an existing entry, so it subsumes
            // `register_public_if_absent_unlocked` for the `false` case that the
            // ~90 user-facing `create_base` call sites take. Inside the root lock,
            // so the `_unlocked` twin is the one that must be called — and in the
            // SAME transaction as the directory, so there is no window in which a
            // private session's new base reads PUBLIC.
            self.stamp_base_unlocked(
                id,
                caller_is_private,
                crate::knowledge::affiliation::contributed_owners(caller_affiliation),
            )?;
            checkpoint(CreateCheckpoint::Classification)?;
            registry::register(
                &self.root,
                RegistryEntry {
                    id: id.to_string(),
                    path: kb_root.clone(),
                },
            )?;
            checkpoint(CreateCheckpoint::Registry)?;
            std::fs::rename(&staged_root, &kb_root).context("publish new knowledge base")?;
            checkpoint(CreateCheckpoint::Published)?;
            Ok(m)
        })();
        mutation.map_err(|error| {
            publication_rollback_failure(
                &format!("knowledge base creation for {id}"),
                error,
                &[staged_root, kb_root],
                &metadata,
            )
            .into()
        })
    }

    pub fn export_brkb(&self, kb_id: &str) -> Result<Vec<u8>> {
        paths::validate_kb_id(kb_id)?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        if !kb_root.exists() {
            anyhow::bail!("kb '{kb_id}' not found");
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        // Issue #56, decision (2a): the archive carries a raise-only provenance
        // marker. No barrier here — this is the USER's download path too
        // (`GET /knowledge/bases/{id}/export`), and DR-14 governs what a MODEL
        // can reach. The model-facing location rule lives in `kb_export`.
        let is_private = crate::knowledge::tier::is_private(&self.root, kb_id);
        // Issue #56 DR-26 / Task 50: the archive carries the owners too, or an
        // export is a way to strip them. `Unknown` — an unreadable store — is
        // the one case with nothing honest to write: the archive grammar has no
        // "unknown ownership" value, and inventing one that fails OPEN is
        // exactly the inversion Step 0's migration warning names. Refuse
        // instead. The same machine cannot write any knowledge base either
        // (`load_for_write` bails on an unreadable store), so this is a broken
        // store surfacing at one more place rather than a new failure mode.
        let owners = match crate::knowledge::tier::affiliation(&self.root, kb_id).owners() {
            Some(owners) => owners.clone(),
            None => anyhow::bail!(
                "cannot export '{kb_id}': the knowledge-base classification store is unreadable, \
                 so whose content this base holds cannot be established and the archive would \
                 claim it belongs to nobody. Repair or remove {}",
                crate::knowledge::paths::kb_tiers_path(&self.root).display()
            ),
        };
        crate::knowledge::brkb::export(&kb_root, &mut buf, is_private, &owners)?;
        Ok(buf.into_inner())
    }

    /// `importer_is_private` is the tier of whoever asked for the import. The
    /// new base ends up at `max(archive marker, importer)` — the marker can only
    /// raise, never lower (issue #56, decision 2a).
    ///
    /// On DR-26's third axis the new base's owners are the **union** of what the
    /// archive carried and the importer's own institution, which is the same
    /// disjunction the tier takes one axis over:
    ///
    /// * The archive's owners, because the content is theirs and an export must
    ///   not be a way to strip a claim.
    /// * The importer's, because importing is an act by that institution's
    ///   session — the same reason a private importer privatises what it
    ///   imports.
    ///
    /// So a Stanford chat importing a UCSF archive lands a base owned by both,
    /// which `affiliation::reachable` puts out of reach of *both* their models.
    /// That is over-restrictive only in the case DR-26 exists to stop, and the
    /// innocent case (UCSF importing its own archive) is a no-op.
    pub fn import_brkb(
        &self,
        zip_bytes: &[u8],
        importer_is_private: bool,
        importer_affiliation: &crate::knowledge::affiliation::CallerAffiliation,
    ) -> Result<String> {
        self.import_brkb_with_checkpoint(
            zip_bytes,
            importer_is_private,
            importer_affiliation,
            |_| Ok(()),
        )
    }

    fn import_brkb_with_checkpoint(
        &self,
        zip_bytes: &[u8],
        importer_is_private: bool,
        importer_affiliation: &crate::knowledge::affiliation::CallerAffiliation,
        mut checkpoint: impl FnMut(ImportCheckpoint) -> Result<()>,
    ) -> Result<String> {
        if zip_bytes.len() as u64 > crate::knowledge::brkb::MAX_ARCHIVE_FILE_BYTES {
            return Err(
                crate::knowledge::brkb::InvalidKnowledgeArchive::new(format!(
                    "compressed archive exceeds the {} MiB limit",
                    crate::knowledge::brkb::MAX_ARCHIVE_FILE_BYTES / (1024 * 1024)
                ))
                .into(),
            );
        }
        let _lock = self.lock_root()?;
        std::fs::create_dir_all(&self.root)?;
        let metadata = BasePublicationSnapshot::capture(&self.root)?;
        let cursor = std::io::Cursor::new(zip_bytes);
        let staged = crate::knowledge::brkb::stage_import(cursor, &self.root)?;
        let crate::knowledge::brkb::StagedImport {
            imported,
            staged_path,
            final_path,
        } = staged;
        let new_id = imported.id;
        let provenance_private = imported.provenance_private;
        let mut owners = imported.owners;
        owners.extend(crate::knowledge::affiliation::contributed_owners(
            importer_affiliation,
        ));
        let mutation = (|| -> Result<String> {
            checkpoint(ImportCheckpoint::Staged)?;
            self.stamp_base_unlocked(&new_id, provenance_private || importer_is_private, owners)?;
            checkpoint(ImportCheckpoint::Classification)?;
            crate::knowledge::registry::register(
                &self.root,
                crate::knowledge::types::RegistryEntry {
                    id: new_id.clone(),
                    path: final_path.clone(),
                },
            )?;
            checkpoint(ImportCheckpoint::Registry)?;
            std::fs::rename(&staged_path, &final_path)
                .context("publish imported knowledge base")?;
            checkpoint(ImportCheckpoint::Published)?;
            Ok(new_id.clone())
        })();
        mutation.map_err(|error| {
            publication_rollback_failure(
                &format!("knowledge base import for {new_id}"),
                error,
                &[staged_path, final_path],
                &metadata,
            )
            .into()
        })
    }

    /// Merge `source_kb_id` **into** `destination_kb_id`. The deterministic half
    /// — see [`crate::knowledge::merge`] for what that means and what it
    /// deliberately leaves to a later macro.
    ///
    /// `dry_run` reports what would move, be renamed and be deduped, and writes
    /// nothing. A merge is the least reversible operation in this subsystem and
    /// `kb_restore_state` restores a whole tree, so a preview is not a
    /// convenience.
    ///
    /// # Why this is a privacy write choke point (DR-17)
    ///
    /// A merge is a content-touching write, and it is the one write in the tree
    /// whose content comes from **another base**. It joins the four existing
    /// choke points rather than inheriting one:
    ///
    /// * The **barrier** runs first, over BOTH ids
    ///   ([`crate::knowledge::merge::MergeAuthority::assert_may_merge`]). You
    ///   cannot merge a base you cannot read into a base you cannot write — and
    ///   the report itself quotes page paths and identifiers back to the caller,
    ///   so a model barred from reading the source must be barred from previewing
    ///   it too.
    /// * The **fold** runs second, before a byte is written: the destination
    ///   takes `max` over the tier axis and the UNION over owner institutions,
    ///   which is exactly the rule
    ///   [`Self::import_brkb`] already applies to an incoming archive. Merging
    ///   base A into base B is the same transfer with the archive step removed,
    ///   so it takes the same rule rather than a second one.
    ///
    /// A merge can therefore **raise** either axis and can never lower one: the
    /// fold goes through the monotone ratchet, and the source base is only read,
    /// so there is nothing of its own to lower.
    ///
    /// ⚠ **The fold precedes the write, and the residual is the accepted
    /// direction.** A merge that fails after the fold leaves the destination
    /// raised with no content added — visible to the user, and reversible with
    /// the tier control. The other order would leave a base holding a private
    /// source's content at PUBLIC if the process died mid-commit, which is
    /// silent. It is the same ordering, for the same reason, as the ratchet in
    /// `KnowledgeServer::call_tool`.
    /// [`Self::merge_bases`] with the caller's cancellation token.
    ///
    /// A merge is the only operation that holds TWO KB locks, taken in id
    /// order, and it holds them at the write deadline. Passing `None` for the
    /// token therefore made Stop do nothing for up to 2 x 1800s — measured in a
    /// live drive, where `kb_merge` parked and the sweep stalled on it.
    pub async fn merge_bases(
        &self,
        destination_kb_id: &str,
        source_kb_id: &str,
        authority: &crate::knowledge::merge::MergeAuthority<'_>,
        dry_run: bool,
    ) -> Result<crate::knowledge::merge::MergeReport> {
        self.merge_bases_cancellable(destination_kb_id, source_kb_id, authority, dry_run, None)
            .await
    }

    pub async fn merge_bases_cancellable(
        &self,
        destination_kb_id: &str,
        source_kb_id: &str,
        authority: &crate::knowledge::merge::MergeAuthority<'_>,
        dry_run: bool,
        cancel: Option<&CancellationToken>,
    ) -> Result<crate::knowledge::merge::MergeReport> {
        paths::validate_kb_id(destination_kb_id)?;
        paths::validate_kb_id(source_kb_id)?;
        anyhow::ensure!(
            destination_kb_id != source_kb_id,
            "cannot merge '{destination_kb_id}' into itself"
        );
        let dst_root = paths::kb_root(&self.root, destination_kb_id);
        let src_root = paths::kb_root(&self.root, source_kb_id);
        anyhow::ensure!(dst_root.exists(), "kb '{destination_kb_id}' not found");
        anyhow::ensure!(src_root.exists(), "kb '{source_kb_id}' not found");

        // FIRST, and over both ids.
        authority.assert_may_merge(&self.root, destination_kb_id, source_kb_id)?;

        // Both locks, in id order. A merge is the only operation that holds two
        // KB locks at once, so it is the only one that can deadlock against a
        // concurrent merge in the other direction; a total order on the ids is
        // what stops that.
        let (first, second) = if destination_kb_id < source_kb_id {
            (destination_kb_id, source_kb_id)
        } else {
            (source_kb_id, destination_kb_id)
        };
        // ⚠ The DEADLINE depends on `dry_run`, and this is the asymmetry #159
        // introduced, applied to the one read path that was still missing it.
        //
        // `kb_merge_preview` writes nothing — the server does not even take a
        // transaction slot for it — but it took the WRITE deadline here, which
        // #159 raised from 120s to 1800s so a real merge could outlast a long
        // ingest. The result was a read-only preview parking for THIRTY MINUTES
        // behind any open transaction on either base. Found by the live tool
        // sweep, where an earlier case left a transaction open on the
        // destination and the preview simply never came back; the daemon log
        // showed the same `kb_merge_preview` dispatched twice, five minutes
        // apart, with nothing in between.
        //
        // A read that gives up loses nothing, so it gets the short deadline and
        // returns a refusal the caller can act on. A write that gives up may
        // strand half a merge, so it keeps the long one. Both still queue on the
        // same lock in the same id order — only the patience differs.
        let wait = if dry_run {
            FileLockGuard::KB_READ_LOCK_WAIT
        } else {
            FileLockGuard::KB_WRITE_LOCK_WAIT
        };
        let _first = self.lock_kb_path_waiting(first, cancel, wait).await?;
        let _second = self.lock_kb_path_waiting(second, cancel, wait).await?;

        // A caller can wait here while another writer raises either base and
        // commits private content. Re-authorize over the locked snapshots before
        // the plan reads page paths, identifiers, or raw-source metadata.
        authority.assert_may_merge(&self.root, destination_kb_id, source_kb_id)?;
        self.require_current_profile(destination_kb_id)?;
        self.require_current_profile(source_kb_id)?;

        let plan = crate::knowledge::merge::plan(&dst_root, &src_root, source_kb_id)?;
        if dry_run {
            let (tier, owners) =
                self.projected_classification(destination_kb_id, source_kb_id, authority)?;
            return Ok(crate::knowledge::merge::report(
                &plan,
                destination_kb_id,
                true,
                &tier,
                owners,
                None,
            ));
        }

        let (tier, owners_added) =
            self.absorb_classification(destination_kb_id, source_kb_id, authority)?;
        let sha = crate::knowledge::merge::apply(&dst_root, &src_root, &plan)?;
        self.rebuild_graph_cache(destination_kb_id)?;
        Ok(crate::knowledge::merge::report(
            &plan,
            destination_kb_id,
            false,
            &tier,
            owners_added,
            Some(sha),
        ))
    }

    /// Fold the source base's classification into the destination's: `max` on
    /// the tier axis, UNION on the owner axis. Returns the destination's tier
    /// word afterwards and the owners the fold added.
    ///
    /// ⚠ It routes through [`Self::stamp_base_unlocked`] rather than calling
    /// `tier::raise_unlocked` itself, and DR-20 says why: the two ratchets must
    /// be reached from the same line of the same function, or a future edit
    /// raises one axis and not the other.
    fn absorb_classification(
        &self,
        destination_kb_id: &str,
        source_kb_id: &str,
        authority: &crate::knowledge::merge::MergeAuthority<'_>,
    ) -> Result<(String, Vec<String>)> {
        let _lock = self.lock_root()?;
        let source_owners = self.owners_or_bail(source_kb_id)?;
        let before = self.owners_or_bail(destination_kb_id)?;
        // The caller's own institution rides along with the source's. The MCP
        // seam already records it for `kb_merge` (it is in `KB_RATCHETING_TOOLS`),
        // so this is idempotent there; it is what covers every other caller.
        let mut owners = source_owners;
        owners.extend(crate::knowledge::affiliation::contributed_owners(
            &authority.caller_affiliation(),
        ));
        self.stamp_base_unlocked(
            destination_kb_id,
            crate::knowledge::tier::is_private(&self.root, source_kb_id)
                || authority.caller_is_private(),
            owners,
        )?;
        // Read back rather than predicted. `add_owners_unlocked` is a no-op with
        // the master toggle off (DR-15), so a report built from what was *asked
        // for* would tell the user their base gained owners it did not.
        let after = self.owners_or_bail(destination_kb_id)?;
        Ok((
            self.tier_word(destination_kb_id),
            after.difference(&before).cloned().collect(),
        ))
    }

    /// What [`Self::absorb_classification`] *would* land on, without writing —
    /// the dry run's answer to "what will this do to my base's privacy?", which
    /// is the question a preview of a merge most needs to answer.
    /// ⚠ It re-derives the ratchet's arithmetic rather than sharing it, which is
    /// the one duplication in this feature and is bounded on purpose: the
    /// alternative is applying the fold and rolling it back, and a ratchet with a
    /// rollback path is a ratchet with a lowering path. What keeps the two in
    /// step is a test that runs a dry run and the real merge over the same pair
    /// and asserts they agree.
    ///
    /// DR-15's master toggle is read here for the same reason
    /// `tier::raise_unlocked` reads it: with the feature off nothing ratchets, so
    /// a preview promising a raise that will not happen would be worse than no
    /// preview.
    fn projected_classification(
        &self,
        destination_kb_id: &str,
        source_kb_id: &str,
        authority: &crate::knowledge::merge::MergeAuthority<'_>,
    ) -> Result<(String, Vec<String>)> {
        let mut incoming = self.owners_or_bail(source_kb_id)?;
        incoming.extend(crate::knowledge::affiliation::contributed_owners(
            &authority.caller_affiliation(),
        ));
        let before = self.owners_or_bail(destination_kb_id)?;
        // Through `tier`'s own spelling, never a second read of the atomic — see
        // `tier::ratchets_are_live`, which exists for this caller.
        let enabled = crate::knowledge::tier::ratchets_are_live();
        let raises = enabled
            && (crate::knowledge::tier::is_private(&self.root, source_kb_id)
                || authority.caller_is_private());
        let private = crate::knowledge::tier::is_private(&self.root, destination_kb_id) || raises;
        let word = if private {
            crate::knowledge::tier::PRIVATE
        } else {
            crate::knowledge::tier::PUBLIC
        };
        let added = if enabled {
            incoming.difference(&before).cloned().collect()
        } else {
            Vec::new()
        };
        Ok((word.to_string(), added))
    }

    /// A base's owner set, refusing when the store cannot say.
    ///
    /// `Unknown` — an unreadable classification store — is the one case with
    /// nothing honest to do: a merge would fold an owner set nobody can read
    /// into another base and the result would claim to belong to whoever the
    /// destination already named. `export_brkb` refuses the same case for the
    /// same reason, and the same machine cannot write any knowledge base either.
    fn owners_or_bail(&self, kb_id: &str) -> Result<std::collections::BTreeSet<String>> {
        match crate::knowledge::tier::affiliation(&self.root, kb_id).owners() {
            Some(owners) => Ok(owners.clone()),
            None => anyhow::bail!(
                "cannot merge with '{kb_id}': the knowledge-base classification store is \
                 unreadable, so whose content it holds cannot be established and the merge \
                 would silently drop an institution's claim. Repair or remove {}",
                crate::knowledge::paths::kb_tiers_path(&self.root).display()
            ),
        }
    }

    fn tier_word(&self, kb_id: &str) -> String {
        if crate::knowledge::tier::is_private(&self.root, kb_id) {
            crate::knowledge::tier::PRIVATE.to_string()
        } else {
            crate::knowledge::tier::PUBLIC.to_string()
        }
    }

    pub fn list_bases(&self) -> Result<Vec<Manifest>> {
        let entries = registry::load(&self.root)?;
        let mut out = Vec::new();
        for e in entries {
            match manifest::load(&e.path) {
                Ok(m) => out.push(m),
                // Still not fatal — one broken base must not take the whole
                // list with it — but no longer *silent*. A dropped base does
                // not report as broken, it vanishes, and DR-12 traces what
                // happens next: the id leaves the installed universe that
                // `installed_kb_ids_unlocked` derives from this list,
                // `repair_decision` reads the stored primary as pointing at
                // something uninstalled, and the next selection edit PERSISTS
                // the cleared `.active-kb`. The user loses their pointers to a
                // base that is still sitting on disk, intact.
                //
                // So the log line has to carry the path: it is the only thing
                // that tells a user with N bases which one to go and look at.
                Err(err) => tracing::warn!(
                    "knowledge: skipping a base whose manifest could not be read at {}: {err:#}",
                    e.path.display()
                ),
            }
        }
        Ok(out)
    }

    pub fn get_base(&self, id: &str) -> Result<Manifest> {
        paths::validate_kb_id(id)?;
        let kb_root = paths::kb_root(&self.root, id);
        if !kb_root.exists() {
            anyhow::bail!("kb '{id}' not found");
        }
        manifest::load(&kb_root)
    }

    pub fn require_current_profile(&self, id: &str) -> Result<KbFormat> {
        self.get_base(id)?.profile().ok_or_else(|| {
            LegacyKnowledgeBaseUnsupported {
                kb_id: id.to_string(),
            }
            .into()
        })
    }

    pub fn update_base(
        &self,
        id: &str,
        name: Option<&str>,
        color: Option<&str>,
    ) -> Result<Manifest> {
        let _kb_lock = self.lock_existing_kb(id)?;
        self.update_base_under_kb_lock(id, name, color, None)
    }

    pub async fn update_base_async(
        &self,
        id: &str,
        name: Option<&str>,
        color: Option<&str>,
        cancel: Option<&CancellationToken>,
    ) -> Result<Manifest> {
        let lock_id = id.to_string();
        let id = lock_id.clone();
        let name = name.map(str::to_string);
        let color = color.map(str::to_string);
        self.run_existing_kb_mutation(&lock_id, cancel, move |svc, cancel| {
            svc.update_base_under_kb_lock(&id, name.as_deref(), color.as_deref(), Some(cancel))
        })
        .await
    }

    fn update_base_under_kb_lock(
        &self,
        id: &str,
        name: Option<&str>,
        color: Option<&str>,
        cancel: Option<&CancellationToken>,
    ) -> Result<Manifest> {
        let _lock = self.lock_root_cancellable(cancel)?;
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("knowledge base update cancelled before mutation");
        }
        let current_root = paths::kb_root(&self.root, id);
        if !current_root.exists() {
            anyhow::bail!("kb '{id}' not found");
        }

        let mut current = manifest::load(&current_root)?;
        let mut changed = false;
        let mut target_id = id.to_string();
        let mut target_root = current_root.clone();

        if let Some(name) = name {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                anyhow::bail!("knowledge base name cannot be empty");
            }
            if current.name != trimmed {
                let next_id = Self::slugify_kb_name(trimmed);
                if next_id.is_empty() {
                    anyhow::bail!("knowledge base name must contain letters or numbers");
                }
                paths::validate_kb_id(&next_id)?;
                let next_root = paths::kb_root(&self.root, &next_id);
                if next_id != id && next_root.exists() {
                    anyhow::bail!("kb '{next_id}' already exists");
                }
                current.name = trimmed.to_string();
                current.id = next_id.clone();
                target_id = next_id;
                target_root = next_root;
                changed = true;
            }
        }

        if let Some(color) = color {
            let trimmed = color.trim();
            if trimmed.is_empty() {
                anyhow::bail!("knowledge base color cannot be empty");
            }
            if current.color != trimmed {
                current.color = trimmed.to_string();
                changed = true;
            }
        }

        if changed {
            let commit_message = if target_id != id {
                format!("rename knowledge base {id} to {}", current.id)
            } else {
                format!("update knowledge base {id} metadata")
            };

            if target_id != id {
                self.move_base_to_new_id(id, &target_id, &current_root, &target_root)?;
            }

            manifest::save(&target_root, &current)?;
            let repo = GitRepo::open(&target_root)?;
            repo.commit_all(
                crate::knowledge::types::ChangeKind::Manual,
                &commit_message,
                None,
            )?;
            self.rebuild_graph_cache(&current.id)?;
        }

        Ok(current)
    }

    /// Carry every id-keyed thing a base owns from `id` to `target_id`: its
    /// write lock, its directory, its registry row, the primary pointers that
    /// name it, the hidden-selection references, and its classification.
    ///
    /// Called with the root lock held and the caller's KB lock for `id` still
    /// alive.
    ///
    /// ⚠ **The lock moves FIRST, and that ordering is the rollback point.** The
    /// caller's guard has to keep covering this base under its new id — the
    /// lock's name IS the id ([`paths::kb_write_lock_path`]) — so it has to
    /// move, and a caller that later opens the new id then gets the same locked
    /// file rather than a second transaction domain. It is also the step most
    /// likely to fail (on Windows it renames a file this process holds open,
    /// legal there only because `std` opens with `FILE_SHARE_DELETE`), and
    /// doing it before the directory has moved is what leaves nothing
    /// half-applied when it does.
    fn move_base_to_new_id(
        &self,
        id: &str,
        target_id: &str,
        current_root: &Path,
        target_root: &Path,
    ) -> Result<()> {
        self.rename_kb_lock(id, target_id)?;
        if let Err(error) = std::fs::rename(current_root, target_root) {
            // Put the lock back, so the guard the caller still holds names the
            // base that still exists.
            let _ = self.rename_kb_lock(target_id, id);
            return Err(error).with_context(|| {
                format!("rename knowledge base directory '{id}' to '{target_id}'")
            });
        }
        registry::replace(
            &self.root,
            id,
            RegistryEntry {
                id: target_id.to_string(),
                path: target_root.to_path_buf(),
            },
        )?;

        if self.get_primary_persisted_unlocked()?.as_deref() == Some(id) {
            self.set_primary_persisted_unlocked(Some(target_id))?;
        }
        self.rewrite_session_primary_refs_unlocked(id, Some(target_id))?;
        self.rewrite_hidden_refs_unlocked(id, Some(target_id))?;
        // ⚠ The classification is keyed by kb id too, so it has to move with
        // everything else above. It did not, and the consequence was not the one
        // it looks like: the TIER survived by accident (`tier::is_private` reads
        // an unknown id whose directory exists as private), while the
        // AFFILIATION did not — an id with no row answers `Owners(∅)`, which is
        // *unclaimed* rather than *nobody's*, and every private model may reach
        // it. So renaming a base holding one institution's data made it readable
        // by another institution's private model, with nothing on screen marking
        // the change.
        crate::knowledge::tier::rename_unlocked(&self.root, id, target_id)
    }

    pub fn set_default_model(&self, id: &str, model: Option<ModelRef>) -> Result<Manifest> {
        let _kb_lock = self.lock_existing_kb(id)?;
        self.set_default_model_under_kb_lock(id, model, None)
    }

    pub async fn set_default_model_async(
        &self,
        id: &str,
        model: Option<ModelRef>,
        cancel: Option<&CancellationToken>,
    ) -> Result<Manifest> {
        let lock_id = id.to_string();
        let id = lock_id.clone();
        self.run_existing_kb_mutation(&lock_id, cancel, move |svc, cancel| {
            svc.set_default_model_under_kb_lock(&id, model, Some(cancel))
        })
        .await
    }

    fn set_default_model_under_kb_lock(
        &self,
        id: &str,
        model: Option<ModelRef>,
        cancel: Option<&CancellationToken>,
    ) -> Result<Manifest> {
        let _lock = self.lock_root_cancellable(cancel)?;
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("default knowledge model update cancelled before mutation");
        }
        let kb_root = paths::kb_root(&self.root, id);
        if !kb_root.exists() {
            anyhow::bail!("kb '{id}' not found");
        }

        let mut current = manifest::load(&kb_root)?;
        if current.default_model == model {
            return Ok(current);
        }

        current.default_model = model;
        manifest::save(&kb_root, &current)?;
        let repo = GitRepo::open(&kb_root)?;
        repo.commit_all(
            crate::knowledge::types::ChangeKind::Manual,
            &format!("set default knowledge model for {id}"),
            None,
        )?;
        Ok(current)
    }

    pub fn delete_base(&self, id: &str) -> Result<()> {
        let _kb_lock = self.lock_existing_kb(id)?;
        self.delete_base_under_kb_lock(id, None)
    }

    /// Remove a **registered** legacy base, re-deciding under the locks that
    /// authorize the deletion that it is still the base the caller classified.
    ///
    /// `Ok(false)` — nothing touched — when it is no longer legacy, no longer
    /// registered at its canonical path, or already gone.
    ///
    /// The revalidation is not belt-and-braces. A caller classifies with no lock
    /// held, and then [`Self::lock_existing_kb`] blocks on `flock` with no
    /// deadline, for as long as whoever holds it takes; the delete itself only
    /// ever checked `kb_root.exists()`, and existence is neither identity nor
    /// format. The interleaving that costs a user their data is ordinary rather
    /// than exotic: there is no in-place legacy → OKF upgrade in this build
    /// (`AUTOMATIC_SCHEMA_CEILING` sits below `CURRENT_SCHEMA_VERSION` on
    /// purpose), so delete-then-recreate at the same id is the *only* way to
    /// move an id off the legacy format — which is to say, the racing operation
    /// is precisely the one this build's own error text tells the user to
    /// perform.
    pub fn delete_registered_legacy_base(&self, id: &str) -> Result<bool> {
        paths::validate_kb_id(id)?;
        if !paths::kb_root(&self.root, id).exists() {
            return Ok(false);
        }
        let _kb_lock = self.lock_existing_kb(id)?;
        self.delete_base_under_locks(
            id,
            None,
            |svc| {
                let kb_root = paths::kb_root(&svc.root, id);
                // `lock_existing_kb` opens the lock file **by path**, so holding
                // it proves nothing about which base now lives there. The
                // registry has to still name this id at this exact path, and the
                // manifest has to still say pre-OKF.
                if !registry::load(&svc.root)?
                    .iter()
                    .any(|entry| entry.id == id && entry.path == kb_root)
                {
                    return Ok(false);
                }
                Ok(classify_base_format(&kb_root) == BaseFormat::Legacy)
            },
            |_| Ok(()),
        )
    }

    /// Remove an on-disk legacy base that is no longer present in the registry.
    /// Startup migration uses this for interrupted upgrades where the old
    /// directory survived but its registry row did not.
    pub fn delete_unregistered_legacy_base(&self, id: &str) -> Result<bool> {
        let _kb_lock = self.lock_existing_kb(id)?;
        let _root_lock = self.lock_root()?;
        let kb_root = paths::kb_root(&self.root, id);
        let entries = registry::load(&self.root)?;
        if entries
            .iter()
            .any(|entry| entry.id == id || entry.path == kb_root)
        {
            return Ok(false);
        }
        // `classify_base_format` and not `Manifest::is_legacy_format`: the
        // latter reads "legacy" off any YAML mapping at all, so it would hand
        // this `remove_dir_all` a foreign directory or a current base with one
        // line missing from its manifest.
        if classify_base_format(&kb_root) != BaseFormat::Legacy {
            return Ok(false);
        }

        let metadata = DeleteMetadataSnapshot::capture(&self.root)?;
        let staged_root = self
            .root
            .join(format!(".deleting-{id}-{}", uuid::Uuid::new_v4()));
        std::fs::rename(&kb_root, &staged_root)
            .with_context(|| format!("stage unregistered legacy knowledge base '{id}'"))?;
        let mutation = (|| -> Result<()> {
            if self.get_primary_persisted_unlocked()?.as_deref() == Some(id) {
                self.set_primary_persisted_unlocked(None)?;
            }
            self.rewrite_session_primary_refs_unlocked(id, None)?;
            self.rewrite_hidden_refs_unlocked(id, None)?;
            crate::knowledge::tier::forget_unlocked(&self.root, id)?;
            Ok(())
        })();
        if let Err(error) = mutation {
            let metadata_restore = metadata.restore();
            let directory_restore = std::fs::rename(&staged_root, &kb_root);
            if let Err(rollback_error) = metadata_restore.and(directory_restore.map_err(Into::into))
            {
                anyhow::bail!(
                    "unregistered legacy purge failed ({error:#}); rollback also failed: {rollback_error:#}"
                );
            }
            return Err(error);
        }
        std::fs::remove_dir_all(&staged_root).with_context(|| {
            format!("finish deletion of unregistered legacy knowledge base '{id}'")
        })?;
        Ok(true)
    }

    pub async fn delete_base_async(
        &self,
        id: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        let lock_id = id.to_string();
        let id = lock_id.clone();
        // ⚠ NOT the 1800 s write wait. A delete has no staged work to lose, and
        // the holder it would be waiting out is most often an open transaction
        // in another conversation — which `kb_delete_base`'s own refusal cannot
        // see, because each chat's `KnowledgeServer` keeps its own transaction
        // registry. See `FileLockGuard::KB_DELETE_LOCK_WAIT`.
        self.run_existing_kb_mutation_waiting(
            &lock_id,
            cancel,
            FileLockGuard::KB_DELETE_LOCK_WAIT,
            move |svc, cancel| svc.delete_base_under_kb_lock(&id, Some(cancel)),
        )
        .await
    }

    fn delete_base_under_kb_lock(
        &self,
        id: &str,
        cancel: Option<&CancellationToken>,
    ) -> Result<()> {
        self.delete_base_under_kb_lock_with_checkpoint(id, cancel, |_| Ok(()))
    }

    fn delete_base_under_kb_lock_with_checkpoint(
        &self,
        id: &str,
        cancel: Option<&CancellationToken>,
        checkpoint: impl FnMut(DeleteCheckpoint) -> Result<()>,
    ) -> Result<()> {
        self.delete_base_under_locks(id, cancel, |_| Ok(true), checkpoint)
            .map(|_| ())
    }

    /// The delete transaction itself.
    ///
    /// `authorize` runs with the root lock held and nothing yet moved, and
    /// `Ok(false)` abandons the deletion leaving the base untouched. It is the
    /// one place a caller that made its decision *before* the locks can re-make
    /// it *under* them — see [`Self::delete_registered_legacy_base`], where that
    /// is the difference between purging a legacy base and destroying the
    /// replacement someone created while the purge was parked on `flock`.
    fn delete_base_under_locks(
        &self,
        id: &str,
        cancel: Option<&CancellationToken>,
        authorize: impl FnOnce(&Self) -> Result<bool>,
        mut checkpoint: impl FnMut(DeleteCheckpoint) -> Result<()>,
    ) -> Result<bool> {
        let _lock = self.lock_root_cancellable(cancel)?;
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("knowledge base deletion cancelled before mutation");
        }
        if !authorize(self)? {
            return Ok(false);
        }
        let kb_root = paths::kb_root(&self.root, id);
        if !kb_root.exists() {
            anyhow::bail!("kb '{id}' not found");
        }

        let metadata = DeleteMetadataSnapshot::capture(&self.root)?;
        let staged_root = self
            .root
            .join(format!(".deleting-{id}-{}", uuid::Uuid::new_v4()));
        std::fs::rename(&kb_root, &staged_root)
            .with_context(|| format!("stage knowledge base '{id}' for deletion"))?;

        let mutation = (|| -> Result<()> {
            checkpoint(DeleteCheckpoint::Staged)?;
            registry::unregister(&self.root, id)?;
            checkpoint(DeleteCheckpoint::Registry)?;

            if self.get_primary_persisted_unlocked()?.as_deref() == Some(id) {
                self.set_primary_persisted_unlocked(None)?;
            }
            checkpoint(DeleteCheckpoint::MachinePrimary)?;

            self.rewrite_session_primary_refs_unlocked(id, None)?;
            checkpoint(DeleteCheckpoint::SessionPrimaries)?;

            self.rewrite_hidden_refs_unlocked(id, None)?;
            checkpoint(DeleteCheckpoint::HiddenSelections)?;

            crate::knowledge::tier::forget_unlocked(&self.root, id)?;
            checkpoint(DeleteCheckpoint::Classification)?;
            Ok(())
        })();

        if let Err(error) = mutation {
            if let Err(rollback_error) = metadata.restore() {
                anyhow::bail!(
                    "knowledge base deletion failed ({error:#}); metadata rollback also failed: {rollback_error:#}. The base remains staged and unavailable"
                );
            }
            if let Err(rollback_error) = std::fs::rename(&staged_root, &kb_root) {
                anyhow::bail!(
                    "knowledge base deletion failed ({error:#}); its metadata was restored but its directory could not be moved back: {rollback_error}. The base remains staged and unavailable"
                );
            }
            return Err(error).context("knowledge base deletion was fully rolled back");
        }

        match std::fs::remove_dir_all(&staged_root) {
            Ok(()) => Ok(true),
            Err(_) if !staged_root.exists() => Ok(true),
            Err(error) => Err(KnowledgeDeleteCleanupFailure {
                kb_id: id.to_string(),
                cause: error.to_string(),
            }
            .into()),
        }
    }

    pub fn base_is_current_or_fully_removed(&self, id: &str) -> Result<bool> {
        paths::validate_kb_id(id)?;
        let _lock = self.lock_root()?;
        let kb_root = paths::kb_root(&self.root, id);
        let registry = registry::load(&self.root)?;
        let registered = registry
            .iter()
            .any(|entry| entry.id == id && entry.path == kb_root);

        if kb_root.exists() {
            return Ok(registered
                && manifest::load(&kb_root).is_ok_and(|manifest| manifest.profile().is_some()));
        }
        if registry.iter().any(|entry| entry.id == id) {
            return Ok(false);
        }
        if self
            .staged_delete_paths_unlocked()?
            .iter()
            .any(|(staged_id, _)| staged_id == id)
            || self
                .staged_publication_paths_unlocked()?
                .iter()
                .any(|(staged_id, _)| staged_id == id)
        {
            return Ok(false);
        }
        if self.get_primary_persisted_unlocked()?.as_deref() == Some(id)
            || self.session_primary_references_unlocked(id)?
            || self.hidden_references_unlocked(id)?
            || crate::knowledge::tier::has_metadata_unlocked(&self.root, id)?
        {
            return Ok(false);
        }
        Ok(true)
    }

    /// Finish or roll back a delete that was interrupted after its atomic
    /// directory rename. Completed logical deletes are cleaned idempotently;
    /// a base that is still registered is restored instead of destroyed.
    pub fn resume_pending_delete_cleanup(&self) -> Result<Vec<String>> {
        let _lock = self.lock_root()?;
        let mut grouped = std::collections::BTreeMap::<String, Vec<PathBuf>>::new();
        for (id, path) in self.staged_delete_paths_unlocked()? {
            grouped.entry(id).or_default().push(path);
        }
        let registered = registry::load(&self.root)?
            .into_iter()
            .map(|entry| entry.id)
            .collect::<std::collections::HashSet<_>>();
        let mut cleaned = Vec::new();

        for (id, mut staged_paths) in grouped {
            staged_paths.sort();
            let kb_root = paths::kb_root(&self.root, &id);
            if registered.contains(&id) {
                if kb_root.exists() {
                    for staged in staged_paths {
                        std::fs::remove_dir_all(&staged).with_context(|| {
                            format!(
                                "remove stale completed-delete directory {}",
                                staged.display()
                            )
                        })?;
                    }
                    cleaned.push(id);
                    continue;
                }
                anyhow::ensure!(
                    staged_paths.len() == 1,
                    "registered knowledge base '{id}' has multiple staged delete directories"
                );
                std::fs::rename(&staged_paths[0], &kb_root).with_context(|| {
                    format!("restore interrupted deletion of registered knowledge base '{id}'")
                })?;
                continue;
            }

            anyhow::ensure!(
                !kb_root.exists(),
                "unregistered knowledge base '{id}' has both an active and a staged directory"
            );
            if self.get_primary_persisted_unlocked()?.as_deref() == Some(id.as_str()) {
                self.set_primary_persisted_unlocked(None)?;
            }
            self.rewrite_session_primary_refs_unlocked(&id, None)?;
            self.rewrite_hidden_refs_unlocked(&id, None)?;
            crate::knowledge::tier::forget_unlocked(&self.root, &id)?;
            for staged in staged_paths {
                std::fs::remove_dir_all(&staged).with_context(|| {
                    format!("finish interrupted deletion of knowledge base '{id}'")
                })?;
            }
            cleaned.push(id);
        }

        Ok(cleaned)
    }

    /// Roll back new-base publications interrupted before their final directory
    /// rename. A staged directory is never a readable base, and its id was chosen
    /// while holding the root lock before metadata was written.
    pub fn resume_pending_import_cleanup(&self) -> Result<Vec<String>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        if self.staged_publication_paths_unlocked()?.is_empty() {
            return Ok(Vec::new());
        }
        let _lock = self.lock_root()?;
        let mut grouped = std::collections::BTreeMap::<String, Vec<PathBuf>>::new();
        for (id, path) in self.staged_publication_paths_unlocked()? {
            grouped.entry(id).or_default().push(path);
        }
        if grouped.is_empty() {
            return Ok(Vec::new());
        }
        let mut cleaned = Vec::new();
        for (id, mut staged_paths) in grouped {
            staged_paths.sort();
            if paths::kb_root(&self.root, &id).exists() {
                for staged in staged_paths {
                    remove_path_if_present(&staged)?;
                }
                cleaned.push(id);
                continue;
            }
            if registry::load(&self.root)?
                .iter()
                .any(|entry| entry.id == id)
            {
                registry::unregister(&self.root, &id)?;
            }
            crate::knowledge::tier::forget_unlocked(&self.root, &id)?;
            for staged in staged_paths {
                remove_path_if_present(&staged).with_context(|| {
                    format!(
                        "remove interrupted base publication at {}",
                        staged.display()
                    )
                })?;
            }
            cleaned.push(id);
        }
        Ok(cleaned)
    }

    fn staged_delete_paths_unlocked(&self) -> Result<Vec<(String, PathBuf)>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut staged = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Some(id) = staged_delete_id(&entry.file_name()) {
                staged.push((id, entry.path()));
            }
        }
        Ok(staged)
    }

    fn staged_publication_paths_unlocked(&self) -> Result<Vec<(String, PathBuf)>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut staged = Vec::new();
        for entry in std::fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Some(id) = staged_publication_id(&entry.file_name()) {
                staged.push((id, entry.path()));
            }
        }
        Ok(staged)
    }

    fn session_primary_references_unlocked(&self, id: &str) -> Result<bool> {
        let dir = paths::primary_kb_sessions_dir(self.root());
        if !dir.exists() {
            return Ok(false);
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() || !is_session_digest(&entry.file_name()) {
                continue;
            }
            if self.read_primary_file_unlocked(&entry.path())?.pinned() == Some(id) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn hidden_references_unlocked(&self, id: &str) -> Result<bool> {
        if self
            .get_hidden_path_unlocked(&paths::hidden_kbs_path(self.root()))?
            .iter()
            .any(|hidden| hidden == id)
        {
            return Ok(true);
        }
        let dir = paths::hidden_kb_sessions_dir(self.root());
        if !dir.exists() {
            return Ok(false);
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() || !is_session_digest(&entry.file_name()) {
                continue;
            }
            if self
                .get_hidden_path_unlocked(&entry.path())?
                .iter()
                .any(|hidden| hidden == id)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl KnowledgeService {
    fn find_existing_source_match(
        &self,
        kb_root: &Path,
        url: Option<&str>,
        sha256: &str,
    ) -> Result<(Option<SourceMeta>, Option<SourceMeta>)> {
        let mut by_url = None;
        let mut by_hash = None;

        for meta in raw::list_sources(kb_root)? {
            if by_url.is_none() && meta.url.as_deref() == url && url.is_some() {
                by_url = Some(meta.clone());
            }
            if by_hash.is_none() && meta.sha256 == sha256 {
                by_hash = Some(meta);
            }
            if by_url.is_some() && by_hash.is_some() {
                break;
            }
        }

        Ok((by_url, by_hash))
    }

    pub async fn add_raw_source(
        &self,
        kb_id: &str,
        input: convert::SourceInput,
        txn_branch: Option<&str>,
    ) -> Result<raw::RawWrite> {
        self.add_raw_source_cancelled_by(kb_id, input, txn_branch, None)
            .await
    }

    pub(crate) async fn add_raw_source_cancelled_by(
        &self,
        kb_id: &str,
        input: convert::SourceInput,
        txn_branch: Option<&str>,
        cancel: Option<&CancellationToken>,
    ) -> Result<raw::RawWrite> {
        paths::validate_kb_id(kb_id)?;
        self.require_current_profile(kb_id)?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        if !kb_root.exists() {
            anyhow::bail!("kb '{kb_id}' does not exist");
        }

        let prepared = prepare_raw_source(&input, cancel).await?;
        let (existing_by_url, existing_by_hash) =
            self.find_existing_source_match(&kb_root, prepared.url.as_deref(), &prepared.sha256)?;
        if let Some(existing) = durable_duplicate_raw_source(
            &kb_root,
            existing_by_url.as_ref(),
            existing_by_hash.as_ref(),
            cancel,
        )? {
            return Ok(existing);
        }

        let source_id = existing_by_url
            .as_ref()
            .map(|meta| meta.id.clone())
            .unwrap_or_else(|| raw::new_source_id(&prepared.title));
        self.commit_prepared_raw_source(
            kb_id,
            source_id,
            prepared,
            existing_by_url.is_some(),
            txn_branch,
            cancel,
        )
    }

    fn commit_prepared_raw_source(
        &self,
        kb_id: &str,
        source_id: String,
        prepared: PreparedRawSource,
        refreshing_existing: bool,
        txn_branch: Option<&str>,
        cancel: Option<&CancellationToken>,
    ) -> Result<raw::RawWrite> {
        let kb_root = paths::kb_root(&self.root, kb_id);
        let meta = SourceMeta {
            id: source_id.clone(),
            title: prepared.title,
            url: prepared.url,
            ingested_at: Utc::now(),
            sha256: prepared.sha256,
            mime: prepared.converted.mime.clone(),
            original_filename: prepared.original_filename,
            credibility: prepared.credibility,
        };
        let source_markdown = source_markdown_with_quality_banner(&prepared.converted);

        // This is the cancellation linearization point. Before it, this call
        // has not modified raw source files. Once the synchronous write/commit
        // section starts, its durable result is returned even if cancellation
        // races it so the macro can report retained raw state accurately.
        ensure_raw_source_not_cancelled(cancel, "raw source commit")?;
        let written = raw::write_raw(
            &kb_root,
            prepared.original_bytes.as_deref(),
            meta.original_filename.clone().as_deref(),
            &source_markdown,
            meta,
        )?;

        let repo = GitRepo::open(&kb_root)?;
        let (summary, delta) = if refreshing_existing {
            (format!("refresh source {source_id}"), "~1 source")
        } else {
            (format!("ingested {source_id}"), "+1 source")
        };
        let committed = if let Some(branch) = txn_branch {
            repo.commit_on_txn_in_progress(branch, &summary)
        } else {
            repo.commit_all(
                crate::knowledge::types::ChangeKind::Ingest,
                &summary,
                Some(delta),
            )
        };
        let commit_sha = committed.map_err(|error| {
            KnowledgeWriteFailure::outcome_uncertain(
                format!("raw source write for {source_id}"),
                error.context("committing raw source files"),
            )
        })?;
        let written = raw::RawWrite {
            commit_sha: Some(commit_sha),
            ..written
        };
        if let Err(error) = self.rebuild_graph_cache(kb_id) {
            return Err(RawSourceRefreshFailure {
                written,
                cause: format!("{error:#}"),
            }
            .into());
        }
        Ok(written)
    }
}

async fn prepare_raw_source(
    input: &convert::SourceInput,
    cancel: Option<&CancellationToken>,
) -> Result<PreparedRawSource> {
    ensure_raw_source_not_cancelled(cancel, "source conversion")?;
    let converted =
        await_raw_source_step(cancel, "source conversion", convert::convert(input)).await?;
    // Classify against the converted text, not just the raw input bytes, so a
    // paper's DOI / journal markers in the body are actually seen.
    let credibility = await_raw_source_step(
        cancel,
        "source credibility classification",
        credibility::classify_with_text(input, Some(&converted.markdown), None),
    )
    .await?;
    let title = humanize_source_title(input, &converted);
    let (original_bytes, original_filename, url) = stage_original_source(input, cancel).await?;
    let sha256 = original_bytes.as_ref().map_or_else(
        || raw::hash_bytes(converted.markdown.as_bytes()),
        |bytes| raw::hash_bytes(bytes),
    );
    Ok(PreparedRawSource {
        title,
        url,
        original_bytes,
        original_filename,
        sha256,
        credibility,
        converted,
    })
}

async fn stage_original_source(
    input: &convert::SourceInput,
    cancel: Option<&CancellationToken>,
) -> Result<(Option<Vec<u8>>, Option<String>, Option<String>)> {
    match input {
        convert::SourceInput::File {
            bytes, filename, ..
        } => Ok((Some(bytes.clone()), Some(filename.clone()), None)),
        convert::SourceInput::Path(path) => {
            let bytes = await_raw_source_step(cancel, "source staging", async {
                Ok(tokio::fs::read(path).await?)
            })
            .await?;
            let filename = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("source")
                .to_string();
            Ok((Some(bytes), Some(filename), None))
        }
        convert::SourceInput::Url(url) => Ok((None, None, Some(url.clone()))),
        convert::SourceInput::Text { .. } => Ok((None, None, None)),
    }
}

fn durable_duplicate_raw_source(
    kb_root: &Path,
    existing_by_url: Option<&SourceMeta>,
    existing_by_hash: Option<&SourceMeta>,
    cancel: Option<&CancellationToken>,
) -> Result<Option<raw::RawWrite>> {
    let Some(existing) = existing_by_hash else {
        return Ok(None);
    };
    if existing_by_url
        .map(|meta| meta.id.as_str())
        .unwrap_or(existing.id.as_str())
        != existing.id
    {
        return Ok(None);
    }

    let source_md_path = format!("raw/{}/source.md", existing.id);
    let source_on_disk = std::fs::read(kb_root.join(&source_md_path))?;
    let repo = GitRepo::open(kb_root)?;
    if !repo.head_file_matches(Path::new(&source_md_path), &source_on_disk)? {
        return Ok(None);
    }
    ensure_raw_source_not_cancelled(cancel, "raw source deduplication")?;
    Ok(Some(raw::RawWrite {
        source_id: existing.id.clone(),
        source_md_path,
        meta_path: format!("raw/{}/meta.yaml", existing.id),
        commit_sha: None,
    }))
}

fn ensure_raw_source_not_cancelled(cancel: Option<&CancellationToken>, phase: &str) -> Result<()> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        anyhow::bail!("raw source ingest cancelled during {phase}");
    }
    Ok(())
}

async fn await_raw_source_step<T>(
    cancel: Option<&CancellationToken>,
    phase: &str,
    step: impl Future<Output = Result<T>>,
) -> Result<T> {
    let Some(cancel) = cancel else {
        return step.await;
    };
    tokio::select! {
        biased;
        () = cancel.cancelled() => anyhow::bail!("raw source ingest cancelled during {phase}"),
        result = step => result,
    }
}

/// Derive a human-readable title for a source, never a hash or UUID filename.
///
/// Order of preference:
///   1. The title extracted by the converter (PDF metadata title, HTML
///      `<title>`, explicit note title) — unless it itself looks machine
///      generated (e.g. `a64e171e-….pdf`).
///   2. The first usable heading / line of the converted markdown body.
///   3. The URL or filename as a last resort, cleaned of obvious noise.
fn humanize_source_title(input: &convert::SourceInput, converted: &convert::Converted) -> String {
    if let Some(t) = converted.title.as_ref() {
        let t = t.trim();
        if !t.is_empty() && !looks_machine_generated(t) {
            return t.to_string();
        }
    }

    // Explicit note titles always win when provided by the caller.
    if let convert::SourceInput::Text { title: Some(t), .. } = input {
        let t = t.trim();
        if !t.is_empty() && !looks_machine_generated(t) {
            return t.to_string();
        }
    }

    if let Some(t) = title_from_markdown(&converted.markdown) {
        return t;
    }

    // Fall back to the source locator, cleaned up.
    let fallback = match input {
        convert::SourceInput::Url(u) => u.clone(),
        convert::SourceInput::File { filename, .. } => filename.clone(),
        convert::SourceInput::Path(path) => path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("source")
            .to_string(),
        convert::SourceInput::Text { .. } => String::new(),
    };
    if !fallback.is_empty() && !looks_machine_generated(&fallback) {
        return fallback;
    }
    let noun = match input {
        convert::SourceInput::Text { .. } => "note",
        _ => "source",
    };
    format!("Untitled {noun}")
}

/// Pull a plausible title out of converted markdown: the first markdown heading
/// (`# …`), or failing that the first reasonably-sized line of prose.
fn title_from_markdown(markdown: &str) -> Option<String> {
    // Skip a leading quality-warning blockquote if present.
    for line in markdown.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('>') {
            continue;
        }
        if let Some(h) = line.strip_prefix('#') {
            let h = h.trim_start_matches('#').trim();
            if h.len() >= 3 && !looks_machine_generated(h) {
                return Some(truncate_title(h));
            }
        }
    }
    // No heading — use the first substantial, sentence-like line.
    for line in markdown.lines() {
        let line = line.trim().trim_start_matches(['#', '>', '*', '-', ' ']);
        if line.len() >= 12
            && line.split_whitespace().count() >= 3
            && !looks_machine_generated(line)
        {
            return Some(truncate_title(line));
        }
    }
    None
}

fn truncate_title(s: &str) -> String {
    const MAX: usize = 160;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let truncated: String = s.chars().take(MAX).collect();
    match truncated.rsplit_once(' ') {
        Some((head, _)) => format!("{}…", head.trim_end()),
        None => format!("{truncated}…"),
    }
}

/// True when a string reads like a machine identifier (UUID, long hex hash, or
/// such with a file extension) rather than a human title. Mirrors the
/// frontend `looksMachineGenerated` guard so titles are clean at the source.
fn looks_machine_generated(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return true;
    }
    let stripped = t
        .rsplit_once('.')
        .map(|(stem, ext)| {
            if matches!(
                ext.to_ascii_lowercase().as_str(),
                "pdf" | "doc" | "docx" | "html" | "htm" | "txt" | "md" | "csv" | "pptx" | "ppt"
            ) {
                stem
            } else {
                t
            }
        })
        .unwrap_or(t);
    let compact: String = stripped
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | ' ' | '.'))
        .collect();
    if compact.len() >= 12 && compact.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    // UUID anywhere, with only noise (digits / "pdf" / separators) around it.
    if let Some(m) = find_uuid(t) {
        let remainder: String = t
            .replacen(m, " ", 1)
            .chars()
            .map(|c| if c.is_ascii_alphabetic() { c } else { ' ' })
            .collect();
        let words: Vec<&str> = remainder
            .split_whitespace()
            .filter(|w| {
                w.len() >= 3
                    && !matches!(
                        w.to_ascii_lowercase().as_str(),
                        "pdf" | "doc" | "docx" | "html"
                    )
            })
            .collect();
        if words.is_empty() {
            return true;
        }
    }
    false
}

/// Return the first UUID substring (8-4-4-4-12 hex) in `s`, if any.
fn find_uuid(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let is_hex = |b: u8| b.is_ascii_hexdigit();
    let pattern = [8usize, 4, 4, 4, 12];
    let mut i = 0;
    while i + 36 <= bytes.len() {
        let mut ok = true;
        let mut pos = i;
        for (gi, &group) in pattern.iter().enumerate() {
            for _ in 0..group {
                if pos >= bytes.len() || !is_hex(bytes[pos]) {
                    ok = false;
                    break;
                }
                pos += 1;
            }
            if !ok {
                break;
            }
            if gi < pattern.len() - 1 {
                if pos >= bytes.len() || bytes[pos] != b'-' {
                    ok = false;
                    break;
                }
                pos += 1;
            }
        }
        if ok {
            // `i`/`pos` only ever land on ASCII hex / '-' bytes, so this is a
            // valid char boundary; `get` keeps clippy happy and is panic-free.
            return s.get(i..pos);
        }
        i += 1;
    }
    None
}

/// Surface poor extraction (scanned / image-based sources) instead of silently
/// storing an empty source.md: the banner is visible in the raw-source preview
/// and tells the digestion sub-agent the content is incomplete. Applied after
/// content hashing so dedup stays keyed on the real converted content.
fn source_markdown_with_quality_banner(converted: &convert::Converted) -> String {
    if converted.needs_llm_fallback {
        format!(
            "> **Warning: poor extraction quality.** This source appears to be \
             scanned or image-based; its text could not be extracted faithfully. \
             Treat the content below (if any) as incomplete.\n\n{}",
            converted.markdown
        )
    } else {
        converted.markdown.clone()
    }
}

/// Typed error returned by [`KnowledgeService::read_page`] so HTTP handlers can
/// map each variant onto the right status code (400 / 404 / 500) without
/// substring-matching the `Display` text.
#[derive(thiserror::Error, Debug)]
pub enum ReadPageError {
    #[error("invalid kb id: {0}")]
    InvalidKbId(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("page not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl KnowledgeService {
    /// Re-derive the knowledge graph from the on-disk pages and overwrite the
    /// cached `graph-cache.json`. Public so macros (and bug-fix migrations
    /// like the wiki-link deriver fix) can refresh stale caches without
    /// hand-crafting a commit.
    pub fn rebuild_graph_cache(&self, kb_id: &str) -> anyhow::Result<()> {
        let kb_root = paths::kb_root(&self.root, kb_id);
        crate::knowledge::graph::rebuild_cache(&kb_root)
    }

    /// Bring a base's `schema.md` up to [`AUTOMATIC_SCHEMA_CEILING`], if it is
    /// behind. User customisations elsewhere in the file are preserved.
    ///
    /// ## The ceiling is not [`CURRENT_SCHEMA_VERSION`], and the gap is the point
    ///
    /// This runs from all three macros, on whatever base they were pointed at,
    /// with no caller identity of its own. That is fine for a *schema* edit and
    /// disqualifying for a *format* one: generation 3 is OKF, reaching it means
    /// rewriting the base's pages, and DR-17 names three concrete privacy
    /// bypasses that a migration on this path opens — starting with rewriting
    /// every page of a private base with nothing having called
    /// `tier::assert_reachable`. DR-22 defers the migration outright. So a base
    /// at generation 2 is **already current** as far as this function is
    /// concerned and comes back `Ok(false)` forever.
    ///
    /// Returns `Ok(true)` when `schema.md` was actually rewritten (and
    /// committed), `Ok(false)` when there was nothing to write — the base was
    /// already current, it has no `schema.md`, or its stamp was behind but its
    /// content was not.
    ///
    /// ## The decision is the version, not a substring
    ///
    /// This used to fingerprint on the literal text `"Cross-reference rules"`
    /// and return early if it was present. That worked exactly once. Every base
    /// created since that block joined `schema_default.md` contains the string,
    /// so the function reported "already migrated" for the entire installed
    /// base — and a *new* schema could therefore never be installed, however
    /// different it was, because the fingerprint of the old migration was still
    /// there. A content fingerprint answers "did migration N run?"; what the
    /// caller needs to know is "which generation is this base at?", and only a
    /// recorded version answers that.
    ///
    /// So `Manifest.schema_version` — declared long ago, always 1, and read by
    /// nothing until now — is the gate, and the manifest is stamped forward
    /// afterwards whether or not the content changed. That last part matters:
    /// bases on disk today are stamped 1 while already carrying generation-2
    /// content, because `create_base` wrote the current schema and hardcoded
    /// the stamp. They must end up stamped 2 *without* gaining a second copy of
    /// the rules block, which is why the step below keeps a content guard of its
    /// own — as an idempotence check inside a step, never as the decision.
    pub fn migrate_schema_if_needed(&self, kb_id: &str) -> Result<bool> {
        paths::validate_kb_id(kb_id)?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        let mut manifest = manifest::load(&kb_root).context("read manifest.yaml")?;
        if manifest.schema_version >= AUTOMATIC_SCHEMA_CEILING {
            return Ok(false);
        }
        let schema_path = kb_root.join("schema.md");
        if !schema_path.exists() {
            // Nothing to migrate and nothing to claim: leaving the stamp behind
            // means a base that later gains a `schema.md` still gets the ladder.
            return Ok(false);
        }
        let from = manifest.schema_version;
        let current = std::fs::read_to_string(&schema_path).context("read schema.md")?;
        let next = migrated_schema(&current, from);
        let rewritten = next != current;
        if rewritten {
            std::fs::write(&schema_path, next).context("write schema.md")?;
        }

        // Stamped before the commit, so the manifest change rides in the same
        // commit as the schema change rather than surfacing later as a stray
        // diff inside an unrelated ingest.
        //
        // To the CEILING, never to `CURRENT_SCHEMA_VERSION`: stamping a base 3
        // would declare it OKF on the strength of a `[[wiki]]`-linked schema,
        // and `Manifest::profile` would hand every later reader that answer with
        // nothing left on disk to contradict it.
        manifest.schema_version = AUTOMATIC_SCHEMA_CEILING;
        manifest::save(&kb_root, &manifest).context("stamp schema_version")?;

        if rewritten {
            let repo = GitRepo::open(&kb_root)?;
            repo.commit_all(
                crate::knowledge::types::ChangeKind::Manual,
                &format!("migrate schema: v{from} → v{AUTOMATIC_SCHEMA_CEILING}"),
                None,
            )
            .context("commit schema migration")?;
        }
        Ok(rewritten)
    }

    pub fn get_graph(&self, kb_id: &str) -> anyhow::Result<crate::knowledge::types::Graph> {
        paths::validate_kb_id(kb_id)?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        anyhow::ensure!(kb_root.exists(), "kb '{kb_id}' does not exist");
        let _file_guard = self.lock_existing_kb(kb_id)?;
        self.get_graph_unlocked(kb_id)
    }

    /// Async graph read for HTTP/MCP handlers. Graph reads join the same
    /// per-KB queue as macros, so a stream of readers cannot repeatedly beat a
    /// macro to the file lock while that macro is waiting or using a provider.
    pub async fn get_graph_async(
        &self,
        kb_id: &str,
    ) -> anyhow::Result<crate::knowledge::types::Graph> {
        paths::validate_kb_id(kb_id)?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        anyhow::ensure!(kb_root.exists(), "kb '{kb_id}' does not exist");
        // #159: the READ deadline, for the reason above — the queue is here for
        // fairness against macros, and an open transaction is not a macro.
        let guard = self
            .lock_kb_path_waiting(kb_id, None, FileLockGuard::KB_READ_LOCK_WAIT)
            .await?;
        let svc = self.clone();
        let kb_id = kb_id.to_string();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            svc.get_graph_unlocked(&kb_id)
        })
        .await
        .map_err(|error| anyhow::anyhow!("knowledge graph read task failed: {error}"))?
    }

    fn get_graph_unlocked(&self, kb_id: &str) -> anyhow::Result<crate::knowledge::types::Graph> {
        let kb_root = paths::kb_root(&self.root, kb_id);
        if let Some(g) = crate::knowledge::graph::read_cache(&kb_root)? {
            return Ok(g);
        }
        // No usable cache. `read_cache` folds four cases into that one answer —
        // absent, unreadable, malformed, or written to a shape this build does
        // not read — because the repair for all four is the same, and it is
        // here: derive fresh and rewrite the cache so the next reader is served
        // from disk. Nothing on this path used to rewrite it, which is how a
        // single bad cache became a permanent 404 on the graph route (DR-13).
        //
        // This subsumes and retires the scaffold self-heal that used to sit
        // here — a hardcoded `n.id == "index" || n.id == "log"` predicate that
        // detected exactly one historical shape change (the deriver learning to
        // exclude the index/log hub pages) and could not detect any other. The
        // envelope's `version` detects every shape change, including that one,
        // because a cache written by the older deriver does not carry a version
        // key at all.
        let fresh = crate::knowledge::graph::derive(&kb_root)?;
        if let Err(e) = crate::knowledge::graph::write_cache(&kb_root, &fresh) {
            let invalidation = crate::knowledge::graph::invalidate_cache(&kb_root);
            match invalidation {
                Ok(()) => tracing::warn!(
                    "knowledge: could not rewrite the graph cache for '{kb_id}', removed the old cache: {e:#}"
                ),
                Err(invalidate_error) => tracing::warn!(
                    "knowledge: could not rewrite the graph cache for '{kb_id}': {e:#}; stale cache removal also failed: {invalidate_error:#}"
                ),
            }
        }
        Ok(fresh)
    }

    /// Read the raw markdown body of a page (knowledge/*.md, raw/*/source.md,
    /// or top-level index.md/schema.md/log.md).
    ///
    /// Returns a typed [`ReadPageError`] so HTTP handlers can map each variant
    /// onto the right status code (400 / 404 / 500) without inspecting the
    /// `Display` text. Path traversal and out-of-scope paths are rejected.
    pub fn read_page(&self, kb_id: &str, rel_path: &str) -> Result<String, ReadPageError> {
        paths::validate_kb_id(kb_id).map_err(|e| ReadPageError::InvalidKbId(e.to_string()))?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        let abs = crate::knowledge::store::resolve_readable_path(&kb_root, rel_path)
            .map_err(|e| ReadPageError::InvalidPath(e.to_string()))?;
        if !abs.exists() {
            return Err(ReadPageError::NotFound(rel_path.to_string()));
        }
        Ok(std::fs::read_to_string(&abs)?)
    }

    /// Read the persisted primary-KB id (set via the UI or `kb_set_active`).
    /// Returns `Ok(None)` if no preference file exists or it is explicitly
    /// blank. This is a raw persistence read; use [`Self::primary_for_session`]
    /// for the effective primary, including the shipped Soul default.
    pub fn get_primary_persisted(&self) -> anyhow::Result<Option<String>> {
        self.get_primary_persisted_unlocked()
    }

    fn get_primary_persisted_unlocked(&self) -> anyhow::Result<Option<String>> {
        self.get_primary_path_unlocked(&crate::knowledge::paths::primary_kb_path(self.root()))
    }

    /// Persist the primary-KB id. Pass `None` to record an explicit durable
    /// clear. Use `set_selection(None, None, PrimaryUpdate::Inherit)` to remove
    /// that preference and restore the shipped Soul default.
    pub fn set_primary_persisted(&self, id: Option<&str>) -> anyhow::Result<()> {
        let _lock = self.lock_root()?;
        self.set_primary_persisted_unlocked(id)
    }

    fn set_primary_persisted_unlocked(&self, id: Option<&str>) -> anyhow::Result<()> {
        let path = crate::knowledge::paths::primary_kb_path(self.root());
        let value = match id {
            Some(id) => StoredPrimary::Pinned(id.to_string()),
            // A blank file is a real machine-level choice. An absent file now
            // means "use Soul", so deleting it here would undo an explicit
            // Clear on the next read.
            None => no_primary_for(None),
        };
        self.write_primary_file_unlocked(&path, &value)
    }

    /// This session's *own* pinned id, ignoring the machine-wide default.
    /// `None` covers both "never chose" and "explicitly has none" — callers
    /// resolving the effective pointer must use
    /// [`Self::primary_for_session`], which knows the difference and applies
    /// the set-membership rule.
    pub fn get_primary_for_session(&self, session_id: &str) -> anyhow::Result<Option<String>> {
        self.get_primary_path_unlocked(&self.primary_session_path(session_id))
    }

    /// Pin (or explicitly un-pin) this session's primary. `kb_id = None` is an
    /// *override*: the session then has no primary even when the machine-wide
    /// default names a base. To go back to following that default, use
    /// [`Self::clear_primary_override_for_session`].
    pub fn set_primary_for_session(
        &self,
        session_id: &str,
        kb_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let _lock = self.lock_root()?;
        let path = self.primary_session_path(session_id);
        let value = match kb_id {
            Some(id) => StoredPrimary::Pinned(id.to_string()),
            None => no_primary_for(Some(session_id)),
        };
        self.write_primary_file_unlocked(&path, &value)
    }

    /// Drop this session's primary override so it follows the machine-wide
    /// pointer again. The mirror of [`Self::clear_hidden_for_session`], and
    /// distinct from `set_primary_for_session(session_id, None)`, which is an
    /// override that pins nothing.
    pub fn clear_primary_override_for_session(&self, session_id: &str) -> anyhow::Result<()> {
        let _lock = self.lock_root()?;
        let path = self.primary_session_path(session_id);
        self.write_primary_file_unlocked(&path, &StoredPrimary::Inherit)
    }

    pub fn get_hidden_persisted(&self) -> anyhow::Result<Vec<String>> {
        self.get_hidden_path_unlocked(&crate::knowledge::paths::hidden_kbs_path(self.root()))
    }

    /// Overwrite the machine-wide hidden list wholesale.
    ///
    /// Reach for [`Self::hide_kb`], [`Self::include_kb`] or
    /// [`Self::set_visible_kbs`] instead unless you already hold the complete
    /// list. This setter is atomic in itself, but the shape it invites —
    /// `get_hidden_persisted()`, edit, `set_hidden_persisted()` — is a
    /// read-modify-write across two unlocked calls, and two surfaces hiding two
    /// *different* bases at the same time will lose one of the two edits.
    pub fn set_hidden_persisted(&self, ids: &[String]) -> anyhow::Result<()> {
        let _lock = self.lock_root()?;
        self.set_hidden_path_unlocked(&crate::knowledge::paths::hidden_kbs_path(self.root()), ids)?;
        self.repair_primary_unlocked(None)?;
        Ok(())
    }

    pub fn get_hidden_for_session(&self, session_id: &str) -> anyhow::Result<Vec<String>> {
        self.get_hidden_path_unlocked(&self.hidden_session_path(session_id))
    }

    pub fn get_hidden_for_session_or_persisted(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<String>> {
        let path = self.hidden_session_path(session_id);
        if path.exists() {
            self.get_hidden_path_unlocked(&path)
        } else {
            self.get_hidden_persisted()
        }
    }

    /// Overwrite one session's hidden-list override wholesale. Same caveat as
    /// [`Self::set_hidden_persisted`]: prefer [`Self::hide_kb`],
    /// [`Self::include_kb`] or [`Self::set_visible_kbs`], which take the whole
    /// gesture under one lock and cannot lose a concurrent edit.
    pub fn set_hidden_for_session(&self, session_id: &str, ids: &[String]) -> anyhow::Result<()> {
        let _lock = self.lock_root()?;
        self.set_hidden_path_unlocked(&self.hidden_session_path(session_id), ids)?;
        self.repair_primary_unlocked(Some(session_id))?;
        Ok(())
    }

    /// Drop a session's hidden-KB override so it inherits the machine-wide
    /// list again. Distinct from `set_hidden_for_session(sid, &[])`, which is
    /// an override that hides nothing.
    pub fn clear_hidden_for_session(&self, session_id: &str) -> anyhow::Result<()> {
        let _lock = self.lock_root()?;
        let path = self.hidden_session_path(session_id);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        self.repair_primary_unlocked(Some(session_id))?;
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
        self.repair_primary_unlocked(None)?;
        Ok(())
    }

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
        let _lock = self.lock_root()?;
        self.session_kb_ids_unlocked(session_id)
    }

    fn session_kb_ids_unlocked(&self, session_id: Option<&str>) -> anyhow::Result<Vec<String>> {
        let hidden = self.hidden_for_scope_unlocked(session_id)?;
        Ok(self
            .installed_kb_ids_unlocked()?
            .into_iter()
            .filter(|id| !hidden.contains(id))
            .collect())
    }

    /// This scope's **primary** knowledge base: the write target for KB-less
    /// mutating calls and the default subject for single-base reads.
    ///
    /// Resolution is session file → machine file → shipped Soul default, and
    /// the result is returned only while it names a member of
    /// [`Self::session_kb_ids`]. A non-member yields `None` rather than
    /// promoting: promoting at read time would make explicit "no primary"
    /// unreachable and let a KB-less *write* silently land in an arbitrary
    /// base. Promotion happens once, at the moment the set changes, in
    /// [`Self::repair_primary_unlocked`].
    ///
    /// Only an *absent* session file inherits. A session file that exists but
    /// is blank is an explicit "no primary here" and stops the fallback dead —
    /// otherwise clearing a session's primary would be a no-op whenever the
    /// machine had one, and a KB-less write would silently re-arm.
    pub fn primary_for_session(&self, session_id: Option<&str>) -> anyhow::Result<Option<String>> {
        let _lock = self.lock_root()?;
        self.primary_for_session_unlocked(session_id)
    }

    fn primary_for_session_unlocked(
        &self,
        session_id: Option<&str>,
    ) -> anyhow::Result<Option<String>> {
        let stored = self.stored_primary_unlocked(session_id)?;
        let Some(stored) = stored.pinned() else {
            return Ok(None);
        };
        let ids = self.session_kb_ids_unlocked(session_id)?;
        Ok(ids.into_iter().find(|id| id == stored))
    }

    /// The tri-state that governs this scope, after the session → machine
    /// fallback but before the set-membership filter.
    fn stored_primary_unlocked(&self, session_id: Option<&str>) -> anyhow::Result<StoredPrimary> {
        let own = self.read_primary_file_unlocked(&self.primary_path_for_scope(session_id))?;
        self.effective_primary_unlocked(&own, session_id)
    }

    /// Resolve a scope's *own* tri-state into the one it is actually **using**:
    /// an absent session file inherits from the machine scope, while an absent
    /// machine preference inherits the shipped Soul product default.
    ///
    /// Split out of [`Self::stored_primary_unlocked`] because the repair path
    /// needs both halves — it decides against the pointer the scope is using
    /// and writes the answer to the file the scope owns.
    fn effective_primary_unlocked(
        &self,
        own: &StoredPrimary,
        session_id: Option<&str>,
    ) -> anyhow::Result<StoredPrimary> {
        let resolved = match (own, session_id) {
            (StoredPrimary::Inherit, Some(_)) => self
                .read_primary_file_unlocked(&crate::knowledge::paths::primary_kb_path(self.root())),
            _ => Ok(own.clone()),
        }?;
        Ok(match resolved {
            StoredPrimary::Inherit => StoredPrimary::Pinned(DEFAULT_PRIMARY_KB_ID.to_string()),
            settled => settled,
        })
    }

    /// Every knowledge base installed on this machine, as ids, sorted — the
    /// universe the set is carved out of, and the answer to "does this id
    /// still exist?".
    fn installed_kb_ids_unlocked(&self) -> anyhow::Result<Vec<String>> {
        let mut ids = self
            .list_bases()?
            .into_iter()
            .map(|base| base.id)
            .collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    fn hidden_for_scope_unlocked(&self, session_id: Option<&str>) -> anyhow::Result<Vec<String>> {
        match session_id {
            Some(session_id) => self.get_hidden_for_session_or_persisted(session_id),
            None => self.get_hidden_persisted(),
        }
    }

    fn primary_path_for_scope(&self, session_id: Option<&str>) -> PathBuf {
        match session_id {
            Some(session_id) => self.primary_session_path(session_id),
            None => crate::knowledge::paths::primary_kb_path(self.root()),
        }
    }

    fn hidden_path_for_scope(&self, session_id: Option<&str>) -> PathBuf {
        match session_id {
            Some(session_id) => self.hidden_session_path(session_id),
            None => crate::knowledge::paths::hidden_kbs_path(self.root()),
        }
    }

    /// What a scope's primary file must become for "the primary is a member of
    /// the set" to hold against `next_ids`. `None` = leave the file alone.
    ///
    /// Pure, so the decision can be taken before anything is written.
    ///
    /// It reasons about `effective` — the pointer the scope is actually
    /// *using*, which for a session that has pinned nothing is the machine-wide
    /// one it inherits — and writes the answer to `own`, the file that scope
    /// owns. Reasoning about `own` alone made the common case wrong: most chats
    /// never pin their own primary, so hiding the base the chat displayed as
    /// *its* primary repaired nothing and the chat came back with no primary at
    /// all, while a chat that had pinned that very same base came back promoted.
    /// Same visible starting state, same click, two answers. Writing the
    /// promotion at session scope is what keeps the machine pointer intact for
    /// every other chat — a session that has moved off the default is exactly
    /// what a session-scope pin represents.
    ///
    /// It still never *invents* a pointer, which is a different thing from
    /// moving one the user already had:
    ///
    /// - an explicit `NoPrimary` choice is returned untouched, so a user who
    ///   cleared the primary is never silently handed one again;
    /// - a pointer at a base that no longer **exists** is *cleared*, never
    ///   promoted — deletion clears, hiding promotes.
    ///
    /// That last distinction is the whole reason `installed` is a parameter.
    /// Promotion is only defensible for a base the user genuinely ranked and
    /// that is still there to rank — one that was merely hidden. A pointer at a
    /// base that has been deleted (or that an upgrade inherited from an older
    /// `.active-kb`) is a dangling reference: it correctly reads as no-primary,
    /// and treating it as "hidden" meant the next unrelated hide silently
    /// promoted a base nobody had chosen, which is precisely the invention the
    /// merged model forbids. A session that merely *inherits* a dangling
    /// pointer has nothing of its own to clear, so it is left alone: writing the
    /// blank override anyway would silently sever that chat from the machine
    /// default over a pointer it never chose, and both spellings read the same
    /// way today — no primary.
    fn repair_decision(
        own: &StoredPrimary,
        effective: &StoredPrimary,
        installed: &[String],
        next_ids: &[String],
        session_id: Option<&str>,
    ) -> Option<StoredPrimary> {
        let StoredPrimary::Pinned(stored) = effective else {
            return None;
        };
        if !installed.iter().any(|id| id == stored) {
            return match own {
                StoredPrimary::Inherit => None,
                _ => Some(no_primary_for(session_id)),
            };
        }
        if next_ids.iter().any(|id| id == stored) {
            return None;
        }
        Some(match next_ids.first() {
            Some(id) => StoredPrimary::Pinned(id.clone()),
            None => no_primary_for(session_id),
        })
    }

    /// Re-establish "the primary is a member of the set" for one scope after
    /// the set changed. Never invents a pointer where there was none. Callers
    /// must already hold the root lock.
    fn repair_primary_unlocked(&self, session_id: Option<&str>) -> anyhow::Result<Option<String>> {
        let path = self.primary_path_for_scope(session_id);
        let own = self.read_primary_file_unlocked(&path)?;
        let effective = self.effective_primary_unlocked(&own, session_id)?;
        let installed = self.installed_kb_ids_unlocked()?;
        let hidden = self.hidden_for_scope_unlocked(session_id)?;
        let next_ids = installed
            .iter()
            .filter(|id| !hidden.contains(id))
            .cloned()
            .collect::<Vec<_>>();
        if let Some(value) =
            Self::repair_decision(&own, &effective, &installed, &next_ids, session_id)
        {
            self.write_primary_file_unlocked(&path, &value)?;
            return Ok(value.pinned().map(ToOwned::to_owned));
        }
        Ok(effective.pinned().map(ToOwned::to_owned))
    }

    /// Read-only snapshot of a scope's selection.
    ///
    /// Taken under the root lock, because a `KbSelection` is a *claim*: its
    /// `primary_kb` is a member of its own `kb_ids`. Composing it from three
    /// separately-unlocked reads made that claim false whenever a writer landed
    /// between them — the reader could measure the set while a base was hidden
    /// and then read the pointer after it had been re-added and pinned, and
    /// hand back a primary that was not in the set it just reported.
    pub fn selection(&self, session_id: Option<&str>) -> anyhow::Result<KbSelection> {
        let _lock = self.lock_root()?;
        self.selection_unlocked(session_id)
    }

    /// One coherent snapshot from one pass over the store. Callers must already
    /// hold the root lock — the public [`Self::selection`] is the one that takes
    /// it, because acquiring `lock_root` twice in a call stack deadlocks.
    fn selection_unlocked(&self, session_id: Option<&str>) -> anyhow::Result<KbSelection> {
        let hidden_kbs = self.hidden_for_scope_unlocked(session_id)?;
        let kb_ids = self
            .installed_kb_ids_unlocked()?
            .into_iter()
            .filter(|id| !hidden_kbs.contains(id))
            .collect::<Vec<_>>();
        let primary_kb = self
            .stored_primary_unlocked(session_id)?
            .pinned()
            .filter(|id| kb_ids.iter().any(|known| known == id))
            .map(ToOwned::to_owned);

        Ok(KbSelection {
            kb_ids,
            hidden_kbs,
            primary_kb,
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
        self.apply_selection_unlocked(session_id, hidden, primary, &|_| true)
    }

    /// [`Self::set_selection`] for a caller that cannot reach every base (issue
    /// #56, QA 2026-09-10 H2): **a caller changes only what it can see.**
    ///
    /// `reachable` is the caller's reach, decided by the daemon's HTTP gate.
    /// For a caller that reaches everything it admits every id and this is
    /// exactly [`Self::set_selection`]. For one that does not:
    ///
    /// * `hidden` is taken literally for the bases `reachable` admits, and every
    ///   base it does not admit keeps the state it already had in this scope —
    ///   neither hidden nor revealed by a list its caller was never shown. That
    ///   is the case that matters: a renderer prunes ids missing from the list
    ///   it was given, and a filtered list would otherwise un-hide every private
    ///   base on the machine as a side effect of one click.
    /// * `Clear` is a no-op when the scope's effective primary is a base the
    ///   caller cannot reach. It was shown no primary, so it asked to clear none.
    ///   `Inherit` likewise leaves a pin this scope holds on such a base: it
    ///   would drop a choice the caller was never shown.
    /// * `Set(id)` naming a base the caller cannot reach is refused. The route
    ///   answers that case first, with the gate's own refusal; this is the
    ///   backstop, and it names nothing.
    /// * A refusal's list of available bases names only reachable ones.
    ///
    /// One root lock across the read of the stored state and the write, so the
    /// merge cannot interleave with another writer (see [`Self::hide_kb`] for
    /// why a read-modify-write across two calls loses an edit).
    pub fn set_selection_within(
        &self,
        session_id: Option<&str>,
        hidden: Option<&[String]>,
        primary: PrimaryUpdate<'_>,
        reachable: &dyn Fn(&str) -> bool,
    ) -> anyhow::Result<KbSelection> {
        let _lock = self.lock_root()?;
        let hidden = match hidden {
            None => None,
            Some(submitted) => {
                let mut next = Self::sanitize_kb_id_list(submitted)?
                    .into_iter()
                    .filter(|id| reachable(id))
                    .collect::<Vec<_>>();
                next.extend(
                    self.hidden_for_scope_unlocked(session_id)?
                        .into_iter()
                        .filter(|id| !reachable(id)),
                );
                Some(next)
            }
        };
        let primary = match primary {
            PrimaryUpdate::Set(id) if !reachable(id) => {
                anyhow::bail!("knowledge base '{id}' is not available")
            }
            PrimaryUpdate::Clear | PrimaryUpdate::Inherit => {
                let own =
                    self.read_primary_file_unlocked(&self.primary_path_for_scope(session_id))?;
                // `Clear` is judged against the pointer the scope is USING and
                // `Inherit` against the one it HOLDS: clearing hides what is
                // shown, and inheriting drops only this scope's own pin.
                let judged = match primary {
                    PrimaryUpdate::Clear => self.effective_primary_unlocked(&own, session_id)?,
                    _ => own,
                };
                if judged.pinned().is_some_and(|id| !reachable(id)) {
                    PrimaryUpdate::Unchanged
                } else {
                    primary
                }
            }
            other => other,
        };
        self.apply_selection_unlocked(session_id, hidden.as_deref(), primary, reachable)
    }

    /// Drop one base from this scope's set, in one root-locked step.
    ///
    /// Prefer this over reading the hidden list, editing it, and writing it
    /// back: that pattern is a read-modify-write across two unlocked calls, so
    /// two surfaces hiding two *different* bases at the same time each write a
    /// list computed before the other's edit, and one of them silently
    /// disappears. Every operation on this API takes the whole gesture, so the
    /// racy shape is not expressible.
    ///
    /// Hiding a base that is not installed is accepted and idempotent — a base
    /// that no longer exists is already absent from the set, and a UI racing a
    /// concurrent delete should not have to care.
    pub fn hide_kb(
        &self,
        session_id: Option<&str>,
        kb_id: &str,
        primary: PrimaryUpdate<'_>,
    ) -> anyhow::Result<KbSelection> {
        let _lock = self.lock_root()?;
        crate::knowledge::paths::validate_kb_id(kb_id)?;
        let mut hidden = self.hidden_for_scope_unlocked(session_id)?;
        if !hidden.iter().any(|id| id == kb_id) {
            hidden.push(kb_id.to_string());
        }
        self.apply_selection_unlocked(session_id, Some(&hidden), primary, &|_| true)
    }

    /// Add one base to this scope's set (un-hide it), in one root-locked step.
    /// See [`Self::hide_kb`] for why this exists rather than a bare setter.
    ///
    /// Unlike hiding, this **rejects** a base that is not installed: "make sure
    /// this is not in my set" is satisfiable for a base that does not exist,
    /// "make sure this *is* in my set" is not, and a caller granting a session
    /// access to a named base wants to hear that the name was wrong rather than
    /// succeed against nothing.
    pub fn include_kb(
        &self,
        session_id: Option<&str>,
        kb_id: &str,
        primary: PrimaryUpdate<'_>,
    ) -> anyhow::Result<KbSelection> {
        let _lock = self.lock_root()?;
        crate::knowledge::paths::validate_kb_id(kb_id)?;
        let installed = self.installed_kb_ids_unlocked()?;
        if !installed.iter().any(|id| id == kb_id) {
            let available = if installed.is_empty() {
                "none".to_string()
            } else {
                installed.join(", ")
            };
            anyhow::bail!("knowledge base '{kb_id}' does not exist (installed: {available})");
        }
        let hidden = self
            .hidden_for_scope_unlocked(session_id)?
            .into_iter()
            .filter(|id| id != kb_id)
            .collect::<Vec<_>>();
        self.apply_selection_unlocked(session_id, Some(&hidden), primary, &|_| true)
    }

    /// Set this scope's set from the ids that should be **visible** — the
    /// inverse of the stored `hidden_kbs`, and the shape every UI actually has
    /// (a list of checked bases), so no caller has to invert it by hand against
    /// a base list it read separately.
    ///
    /// The set is taken literally: any installed base absent from `visible`
    /// becomes hidden, including one created since the caller last listed. Ids
    /// that are not installed are simply not part of the set and are dropped —
    /// there is nothing to hide or show.
    pub fn set_visible_kbs(
        &self,
        session_id: Option<&str>,
        visible: &[String],
        primary: PrimaryUpdate<'_>,
    ) -> anyhow::Result<KbSelection> {
        let _lock = self.lock_root()?;
        let visible = Self::sanitize_kb_id_list(visible)?;
        let hidden = self
            .installed_kb_ids_unlocked()?
            .into_iter()
            .filter(|id| !visible.contains(id))
            .collect::<Vec<_>>();
        self.apply_selection_unlocked(session_id, Some(&hidden), primary, &|_| true)
    }

    /// The engine behind every selection write: decide, validate, *then* write.
    ///
    /// The two halves are strictly ordered and that ordering is the whole
    /// point. This used to write the hidden list first and validate the
    /// requested primary afterwards, so "hide the base I am pinned to, and pin
    /// one that does not exist" persisted the hide, returned an error, and left
    /// the stored pointer sitting outside the resulting set — a half-applied
    /// request that broke the model's one invariant. Nothing below the
    /// "commit" line can fail on anything but I/O.
    ///
    /// Callers must already hold the root lock.
    ///
    /// `listed` decides which bases a refusal may NAME when it lists what is
    /// available: every base for the in-process callers, the reachable ones for
    /// an HTTP caller that cannot reach them all (see
    /// [`Self::set_selection_within`]). A refusal that enumerated the rest would
    /// hand over the ids the caller was just refused.
    fn apply_selection_unlocked(
        &self,
        session_id: Option<&str>,
        hidden: Option<&[String]>,
        primary: PrimaryUpdate<'_>,
        listed: &dyn Fn(&str) -> bool,
    ) -> anyhow::Result<KbSelection> {
        // ---- decide: touch nothing on disk until every branch has succeeded ----
        let installed = self.installed_kb_ids_unlocked()?;
        let next_hidden = match hidden {
            Some(ids) => Self::sanitize_kb_id_list(ids)?,
            None => self.hidden_for_scope_unlocked(session_id)?,
        };
        let next_ids = installed
            .iter()
            .filter(|id| !next_hidden.contains(id))
            .cloned()
            .collect::<Vec<_>>();

        let primary_path = self.primary_path_for_scope(session_id);
        let own_primary = self.read_primary_file_unlocked(&primary_path)?;
        let effective_primary = self.effective_primary_unlocked(&own_primary, session_id)?;

        // `None` means "leave this scope's primary file exactly as it is".
        let next_primary: Option<StoredPrimary> = match primary {
            PrimaryUpdate::Unchanged => Self::repair_decision(
                &own_primary,
                &effective_primary,
                &installed,
                &next_ids,
                session_id,
            ),
            PrimaryUpdate::Clear => Some(no_primary_for(session_id)),
            PrimaryUpdate::Inherit => Some(StoredPrimary::Inherit),
            PrimaryUpdate::Set(id) => {
                if !next_ids.iter().any(|known| known == id) {
                    let named = next_ids
                        .iter()
                        .filter(|known| listed(known))
                        .map(String::as_str)
                        .collect::<Vec<_>>();
                    let available = if named.is_empty() {
                        "none".to_string()
                    } else {
                        named.join(", ")
                    };
                    // Scope-appropriate vocabulary: the CLI and scheduled jobs
                    // pass `None` and have no session concept at all (D11), so
                    // telling them to "add it to the session" names nothing
                    // they can act on.
                    match session_id {
                        Some(_) => anyhow::bail!(
                            "knowledge base '{id}' is not one of this session's knowledge bases \
                             ({available}). Add it to the session first, or pass kb_id \
                             explicitly to read it once."
                        ),
                        None => anyhow::bail!(
                            "knowledge base '{id}' is not available ({available}): it does not \
                             exist, or it is hidden."
                        ),
                    }
                }
                Some(StoredPrimary::Pinned(id.to_string()))
            }
        };

        // ---- commit ----
        if hidden.is_some() {
            let path = self.hidden_path_for_scope(session_id);
            self.write_hidden_file_unlocked(&path, &next_hidden)?;
        }
        if let Some(value) = &next_primary {
            self.write_primary_file_unlocked(&primary_path, value)?;
        }

        // Report what we just decided rather than re-reading it: the values are
        // known-coherent here, and a re-read would be three more chances to
        // observe someone else's half-finished edit.
        let effective = match next_primary {
            // Just written: this scope now defers to the machine pointer, and an
            // unset machine pointer resolves to the shipped Soul default.
            Some(StoredPrimary::Inherit) => {
                self.effective_primary_unlocked(&StoredPrimary::Inherit, session_id)?
            }
            Some(settled) => settled,
            None => effective_primary,
        };
        let primary_kb = effective
            .pinned()
            .filter(|id| next_ids.iter().any(|known| known == id))
            .map(ToOwned::to_owned);

        Ok(KbSelection {
            kb_ids: next_ids,
            hidden_kbs: next_hidden,
            primary_kb,
        })
    }
}

impl KnowledgeService {
    pub fn list_history(
        &self,
        kb_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<crate::knowledge::types::HistoryEntry>> {
        paths::validate_kb_id(kb_id)?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        let repo = GitRepo::open(&kb_root)?;
        repo.log(limit)
    }

    pub fn restore_state(&self, kb_id: &str, commit_sha: &str) -> anyhow::Result<String> {
        let _lock = self.lock_existing_kb(kb_id)?;
        self.restore_state_under_kb_lock(kb_id, commit_sha, None)
    }

    pub async fn restore_state_async(
        &self,
        kb_id: &str,
        commit_sha: &str,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<String> {
        let lock_id = kb_id.to_string();
        let kb_id = lock_id.clone();
        let commit_sha = commit_sha.to_string();
        self.run_existing_kb_mutation(&lock_id, cancel, move |svc, cancel| {
            svc.restore_state_under_kb_lock(&kb_id, &commit_sha, Some(cancel))
        })
        .await
    }

    fn restore_state_under_kb_lock(
        &self,
        kb_id: &str,
        commit_sha: &str,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<String> {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("knowledge restore cancelled before mutation");
        }
        self.require_current_profile(kb_id)?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        let repo = GitRepo::open(&kb_root)?;
        let target_manifest = repo
            .read_file_at(commit_sha, "manifest.yaml")?
            .ok_or_else(|| anyhow::anyhow!("commit '{commit_sha}' has no knowledge manifest"))?;
        let target_manifest: Manifest =
            serde_yaml::from_str(&target_manifest).context("read target knowledge manifest")?;
        if target_manifest.profile().is_none() {
            anyhow::bail!(LegacyKnowledgeRestoreUnsupported {
                kb_id: kb_id.to_string(),
                commit_sha: commit_sha.to_string(),
            });
        }
        let summary = format!("restore to {}", commit_sha.get(..7).unwrap_or(commit_sha));
        let sha = repo.restore_to(commit_sha, &summary)?;
        if let Err(error) = self.rebuild_graph_cache(kb_id) {
            return Err(KnowledgeWriteFailure::committed("knowledge restore", &sha, error).into());
        }
        Ok(sha)
    }

    pub fn preview_state(
        &self,
        kb_id: &str,
        commit_sha: &str,
        path: &str,
    ) -> anyhow::Result<Option<String>> {
        paths::validate_kb_id(kb_id)?;
        let kb_root = paths::kb_root(&self.root, kb_id);
        let repo = GitRepo::open(&kb_root)?;
        repo.read_file_at(commit_sha, path)
    }
}

impl KnowledgeService {
    /// Re-run credibility classification for an existing raw source using the stored URL or
    /// the derived markdown text (for File/Text sources) and persist the result to `meta.yaml`.
    pub async fn reclassify_source(
        &self,
        kb_id: &str,
        source_id: &str,
    ) -> anyhow::Result<crate::knowledge::types::Credibility> {
        self.reclassify_source_cancelled_by(kb_id, source_id, None)
            .await
    }

    pub async fn reclassify_source_cancelled_by(
        &self,
        kb_id: &str,
        source_id: &str,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<crate::knowledge::types::Credibility> {
        let operation_cancel = cancel
            .map(CancellationToken::child_token)
            .unwrap_or_default();
        let cancel_operation_on_drop = CancelOnDrop(operation_cancel.clone());
        let guard = self
            .lock_existing_kb_cancellable(kb_id, Some(&operation_cancel))
            .await?;
        let kb_id = kb_id.to_string();
        let kb_root = paths::kb_root(&self.root, &kb_id);
        let source_id = source_id.to_string();
        let source_for_preflight = source_id.clone();
        let preflight_cancel = operation_cancel.clone();
        let (guard, mut meta, stored_body, input) = tokio::task::spawn_blocking(move || {
            ensure_raw_source_not_cancelled(Some(&preflight_cancel), "source reclassification")?;
            let meta = raw::read_meta(&kb_root, &source_for_preflight)?;
            let stored_body = std::fs::read_to_string(
                kb_root
                    .join("raw")
                    .join(&source_for_preflight)
                    .join("source.md"),
            )
            .ok();
            let input = if let Some(url) = meta.url.clone() {
                convert::SourceInput::Url(url)
            } else {
                convert::SourceInput::Text {
                    text: stored_body.clone().unwrap_or_default(),
                    title: Some(meta.title.clone()),
                }
            };
            Ok::<_, anyhow::Error>((guard, meta, stored_body, input))
        })
        .await
        .map_err(|error| anyhow::anyhow!("source reclassification preflight failed: {error}"))??;

        let new_cred = await_raw_source_step(
            Some(&operation_cancel),
            "source reclassification",
            credibility::classify_with_text(&input, stored_body.as_deref(), None),
        )
        .await?;
        ensure_raw_source_not_cancelled(Some(&operation_cancel), "source reclassification")?;

        let svc = self.clone();
        let commit_cancel = operation_cancel.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            ensure_raw_source_not_cancelled(Some(&commit_cancel), "source reclassification")?;
            let kb_root = paths::kb_root(svc.root(), &kb_id);
            meta.credibility = new_cred.clone();
            let yaml = serde_yaml::to_string(&meta)?;
            std::fs::write(kb_root.join("raw").join(&source_id).join("meta.yaml"), yaml)?;

            let repo = GitRepo::open(&kb_root)?;
            repo.commit_all(
                crate::knowledge::types::ChangeKind::Manual,
                &format!("reclassify {source_id}"),
                None,
            )?;
            svc.rebuild_graph_cache(&kb_id)?;
            Ok::<_, anyhow::Error>(new_cred)
        })
        .await
        .map_err(|error| anyhow::anyhow!("source reclassification commit failed: {error}"))?;
        drop(cancel_operation_on_drop);
        result
    }

    /// Write a manually-specified `Credibility` override to `meta.yaml` and commit.
    /// Returns the credibility that was stored (same as input).
    pub fn override_credibility(
        &self,
        kb_id: &str,
        source_id: &str,
        cred: crate::knowledge::types::Credibility,
    ) -> anyhow::Result<crate::knowledge::types::Credibility> {
        let _lock = self.lock_existing_kb(kb_id)?;
        self.override_credibility_under_kb_lock(kb_id, source_id, cred, None)
    }

    pub async fn override_credibility_async(
        &self,
        kb_id: &str,
        source_id: &str,
        cred: crate::knowledge::types::Credibility,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<crate::knowledge::types::Credibility> {
        let lock_id = kb_id.to_string();
        let kb_id = lock_id.clone();
        let source_id = source_id.to_string();
        self.run_existing_kb_mutation(&lock_id, cancel, move |svc, cancel| {
            svc.override_credibility_under_kb_lock(&kb_id, &source_id, cred, Some(cancel))
        })
        .await
    }

    fn override_credibility_under_kb_lock(
        &self,
        kb_id: &str,
        source_id: &str,
        cred: crate::knowledge::types::Credibility,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<crate::knowledge::types::Credibility> {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            anyhow::bail!("knowledge credibility override cancelled before mutation");
        }
        let kb_root = paths::kb_root(&self.root, kb_id);
        let mut meta = raw::read_meta(&kb_root, source_id)?;
        meta.credibility = cred.clone();
        let yaml = serde_yaml::to_string(&meta)?;
        std::fs::write(kb_root.join("raw").join(source_id).join("meta.yaml"), yaml)?;
        let repo = GitRepo::open(&kb_root)?;
        repo.commit_all(
            crate::knowledge::types::ChangeKind::Manual,
            &format!("override credibility for {source_id}"),
            None,
        )?;
        self.rebuild_graph_cache(kb_id)?;
        Ok(cred)
    }
}

impl KnowledgeService {
    /// Perform a trivial LLM completion to verify the provider is reachable.
    ///
    /// Sends a single "Reply with OK" message to the model.  Any network error,
    /// authentication failure, or invalid model name surfaces as `Err`.
    pub async fn check_model(
        &self,
        completer: Box<dyn crate::knowledge::subagent::loop_::Completer>,
    ) -> anyhow::Result<()> {
        let messages = vec![crate::knowledge::subagent::loop_::LlmMessage::User(
            "Reply with exactly OK and nothing else.".to_string(),
        )];
        completer
            .complete(
                "You are a connectivity test. Respond with OK.",
                &messages,
                &[], // no tools
            )
            .await
            .map_err(|e| anyhow::anyhow!("LLM check failed: {e}"))?;
        Ok(())
    }
}

/// True when `name` is a session-digest filename — 64 lowercase hex chars,
/// exactly what `raw::hash_bytes` produces. Everything else in a
/// `.*-sessions/` directory is debris (most often a `<digest>.tmp` staged
/// write that a crash left behind) and must never be read as session state.
fn is_session_digest(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    name.len() == 64 && name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::convert::SourceInput;
    use crate::knowledge::types::{ChangeKind, Credibility, CredibilityTier};
    use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

    fn svc() -> (tempfile::TempDir, KnowledgeService) {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        (dir, svc)
    }

    #[test]
    fn shipped_schema_workflows_match_available_knowledge_tools() {
        for (name, schema) in [
            ("okf", SCHEMA_OKF),
            ("biookf", SCHEMA_BIOOKF),
            ("legacy", include_str!("schema_default.md")),
        ] {
            assert!(schema.contains("`kb_lint` is read-only"), "{name}");
            for unavailable in ["`kb_ingest_source`", "`kb_query`", "`autofix=true`"] {
                assert!(!schema.contains(unavailable), "{name}: {unavailable}");
            }
            assert!(schema.contains("only when the user asks to save"), "{name}");
        }
    }

    #[test]
    fn service_rejects_and_cleans_up_a_legacy_archive() {
        let (dir, svc) = svc();
        svc.create_base("legacy", "Legacy", None).unwrap();
        let root = dir.path().join("legacy");
        let mut manifest = crate::knowledge::manifest::load(&root).unwrap();
        manifest.schema_version = crate::knowledge::types::AUTOMATIC_SCHEMA_CEILING;
        crate::knowledge::manifest::save(&root, &manifest).unwrap();
        let archive = svc.export_brkb("legacy").unwrap();

        let error = svc
            .import_brkb(
                &archive,
                false,
                &crate::knowledge::affiliation::CallerAffiliation::Unstated,
            )
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<LegacyKnowledgeArchiveUnsupported>()
                .is_some(),
            "{error:#}"
        );
        assert!(!dir.path().join("legacy-2").exists());
        assert_eq!(svc.list_bases().unwrap().len(), 1);
    }

    #[test]
    fn creation_rolls_back_files_registry_and_classification_at_every_phase() -> anyhow::Result<()>
    {
        for fault in [
            CreateCheckpoint::Files,
            CreateCheckpoint::Repository,
            CreateCheckpoint::GraphCache,
            CreateCheckpoint::Classification,
            CreateCheckpoint::Registry,
            CreateCheckpoint::Published,
        ] {
            let (_dir, svc) = svc();
            let before = BasePublicationSnapshot::capture(svc.root())?;
            let error = svc
                .create_base_as_with_checkpoint(
                    CreateBaseSpec {
                        id: "retryable",
                        name: "Retryable",
                        color: None,
                        format: KbFormat::Okf,
                    },
                    true,
                    &crate::knowledge::affiliation::CallerAffiliation::Institution(
                        "ucsf".to_string(),
                    ),
                    |checkpoint| {
                        anyhow::ensure!(checkpoint != fault, "injected failure after {fault:?}");
                        Ok(())
                    },
                )
                .expect_err("the selected create checkpoint must fail");
            let failure = error
                .downcast_ref::<KnowledgeWriteFailure>()
                .expect("a failed publication reports whether retry is safe");
            assert_eq!(
                failure.phase,
                crate::knowledge::git::KnowledgeWriteFailurePhase::RolledBack,
                "{fault:?}: {error:#}"
            );
            assert!(!svc.root().join("retryable").exists(), "{fault:?}");
            assert!(!std::fs::read_dir(svc.root())?.any(|entry| {
                entry.is_ok_and(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".creating-retryable-")
                })
            }));
            assert_eq!(
                BasePublicationSnapshot::capture(svc.root())?,
                before,
                "{fault:?}"
            );
            svc.create_base("retryable", "Retryable", None)?;
        }
        Ok(())
    }

    #[test]
    fn import_rolls_back_staging_registry_and_classification_at_every_phase() -> anyhow::Result<()>
    {
        let source_dir = tempfile::tempdir()?;
        let source = KnowledgeService::new(source_dir.path().to_path_buf());
        source.create_base("retryable", "Retryable", None)?;
        let archive = source.export_brkb("retryable")?;

        for fault in [
            ImportCheckpoint::Staged,
            ImportCheckpoint::Classification,
            ImportCheckpoint::Registry,
            ImportCheckpoint::Published,
        ] {
            let (_dir, svc) = svc();
            let before = BasePublicationSnapshot::capture(svc.root())?;
            let error = svc
                .import_brkb_with_checkpoint(
                    &archive,
                    true,
                    &crate::knowledge::affiliation::CallerAffiliation::Institution(
                        "ucsf".to_string(),
                    ),
                    |checkpoint| {
                        anyhow::ensure!(checkpoint != fault, "injected failure after {fault:?}");
                        Ok(())
                    },
                )
                .expect_err("the selected import checkpoint must fail");
            let failure = error
                .downcast_ref::<KnowledgeWriteFailure>()
                .expect("a failed publication reports whether retry is safe");
            assert_eq!(
                failure.phase,
                crate::knowledge::git::KnowledgeWriteFailurePhase::RolledBack,
                "{fault:?}: {error:#}"
            );
            assert!(!svc.root().join("retryable").exists(), "{fault:?}");
            assert!(!std::fs::read_dir(svc.root())?.any(|entry| {
                entry.is_ok_and(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".importing-retryable-")
                })
            }));
            assert_eq!(
                BasePublicationSnapshot::capture(svc.root())?,
                before,
                "{fault:?}"
            );
            svc.create_base("retryable", "Retryable", None)?;
        }
        Ok(())
    }

    #[test]
    fn startup_rolls_back_an_interrupted_import_before_publication() -> anyhow::Result<()> {
        let source_dir = tempfile::tempdir()?;
        let source = KnowledgeService::new(source_dir.path().to_path_buf());
        source.create_base("interrupted", "Interrupted", None)?;
        let archive = source.export_brkb("interrupted")?;

        let destination = tempfile::tempdir()?;
        let svc = KnowledgeService::new(destination.path().to_path_buf());
        let staged = crate::knowledge::brkb::stage_import(
            std::io::Cursor::new(&archive),
            destination.path(),
        )?;
        svc.stamp_base_unlocked(
            &staged.imported.id,
            true,
            std::collections::BTreeSet::from(["ucsf".to_string()]),
        )?;
        registry::register(
            destination.path(),
            RegistryEntry {
                id: staged.imported.id.clone(),
                path: staged.final_path.clone(),
            },
        )?;
        assert!(staged.staged_path.exists());

        let reopened = KnowledgeService::new(destination.path().to_path_buf());
        assert!(!staged.staged_path.exists());
        assert!(!staged.final_path.exists());
        assert!(!crate::knowledge::tier::has_metadata_unlocked(
            destination.path(),
            "interrupted"
        )?);
        assert!(registry::load(destination.path())?.is_empty());
        reopened.create_base("interrupted", "Interrupted", None)?;
        Ok(())
    }

    #[test]
    fn startup_rolls_back_an_interrupted_creation_before_publication() -> anyhow::Result<()> {
        let destination = tempfile::tempdir()?;
        let svc = KnowledgeService::new(destination.path().to_path_buf());
        let staged = destination
            .path()
            .join(format!(".creating-interrupted-{}", uuid::Uuid::new_v4()));
        let final_path = destination.path().join("interrupted");
        std::fs::create_dir_all(staged.join("knowledge"))?;
        svc.stamp_base_unlocked(
            "interrupted",
            true,
            std::collections::BTreeSet::from(["ucsf".to_string()]),
        )?;
        registry::register(
            destination.path(),
            RegistryEntry {
                id: "interrupted".to_string(),
                path: final_path.clone(),
            },
        )?;

        let reopened = KnowledgeService::new(destination.path().to_path_buf());
        assert!(!staged.exists());
        assert!(!final_path.exists());
        assert!(!crate::knowledge::tier::has_metadata_unlocked(
            destination.path(),
            "interrupted"
        )?);
        assert!(registry::load(destination.path())?.is_empty());
        reopened.create_base("interrupted", "Interrupted", None)?;
        Ok(())
    }

    #[test]
    fn current_profile_boundary_refuses_a_pre_okf_base() {
        let (dir, svc) = svc();
        svc.create_base("old", "Old", None).unwrap();
        let root = dir.path().join("old");
        let mut manifest = crate::knowledge::manifest::load(&root).unwrap();
        manifest.schema_version = crate::knowledge::types::AUTOMATIC_SCHEMA_CEILING;
        crate::knowledge::manifest::save(&root, &manifest).unwrap();

        let error = svc.require_current_profile("old").unwrap_err();
        let typed = error
            .downcast_ref::<LegacyKnowledgeBaseUnsupported>()
            .expect("legacy base error stays typed");
        assert_eq!(typed.kb_id, "old");
        assert!(svc
            .require_current_profile("missing")
            .unwrap_err()
            .downcast_ref::<LegacyKnowledgeBaseUnsupported>()
            .is_none());
    }

    #[test]
    fn staged_delete_id_rejects_non_ascii_ids_without_byte_slicing() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let valid = format!(".deleting-victim-{uuid}");
        let non_ascii = format!(".deleting-victím-{uuid}");

        assert_eq!(
            staged_delete_id(std::ffi::OsStr::new(&valid)),
            Some("victim".to_string())
        );
        assert_eq!(staged_delete_id(std::ffi::OsStr::new(&non_ascii)), None);
    }

    /// Issue #56 DR-26 / Task 50, and the regression that made it necessary.
    ///
    /// This module is the whole production surface of the combined classification
    /// ratchet. Both callers use `stamp_unlocked`, whose single store replacement
    /// moves the tier and affiliation axes together.
    ///
    /// ⚠ **The count stayed at 2 when the merge landed, and that is the shape a
    /// new choke point should have.** `merge_bases` is a fifth privacy write
    /// choke point (DR-17) and folds the SOURCE base's classification into the
    /// destination — a raise on the tier axis, a union on the owner axis — yet
    /// it adds no site here, because `absorb_classification` routes through
    /// `stamp_base_unlocked`. That is DR-20's instruction applied rather than
    /// its number bumped.
    ///
    /// ⚠ **A tripwire over one spelling, not a proof** — the same shape as
    /// `tier_user::tests::exactly_one_writer_outside_the_ratchet_saves_the_tier_store`.
    /// What it reliably catches is the realistic case: a third ratchet path
    /// added here that stamps the tier and forgets the third axis. The two
    /// permitted sites are [`KnowledgeService::raise_tier_and_affiliation`] (for
    /// a base that already exists) and [`KnowledgeService::stamp_base_unlocked`]
    /// (for one this call is minting); each does both raises itself, so there is
    /// no third function that could do one.
    #[test]
    fn the_tier_ratchet_has_no_production_call_site_that_skips_the_affiliation() {
        let this =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/knowledge/service.rs");
        let src = std::fs::read_to_string(&this)
            .unwrap_or_else(|e| panic!("the audit could not read {}: {e}", this.display()));
        // Composed, so the audit does not match itself.
        let needle = concat!("tier::", "stamp_unlocked(");
        let sites: Vec<&str> = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("//") && l.contains(needle))
            .collect();
        assert_eq!(
            sites.len(),
            2,
            "the combined classification ratchet is called {} times in service.rs, not 2. Use \
             `stamp_base_unlocked` for a base this call is minting, or \
             `raise_tier_and_affiliation` for one that already exists. Sites \
             found: {sites:#?}",
            sites.len()
        );
    }

    #[test]
    fn create_base_writes_all_files_and_inits_git() {
        let (_dir, svc) = svc();
        let m = svc.create_base("ms", "MS Patient Analysis", None).unwrap();
        let kb = svc.root().join("ms");
        assert!(kb.join("manifest.yaml").exists());
        assert!(kb.join("schema.md").exists());
        assert!(kb.join("index.md").exists());
        assert!(kb.join("log.md").exists());
        assert!(kb.join(".gitignore").exists());
        for dir in scaffold_dirs(KbFormat::Okf) {
            assert!(
                kb.join("knowledge").join(&dir).exists(),
                "scaffold directory knowledge/{dir} was not created"
            );
        }
        assert!(kb.join("raw").exists());
        assert!(kb.join(".biorouter-knowledge").exists());
        assert!(kb.join(".git").exists());
        assert_eq!(m.id, "ms");

        // Initial commit exists.
        let repo = GitRepo::open(&kb).unwrap();
        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 1);
        assert!(log[0].summary.contains("create knowledge base ms"));

        // Registry has one entry.
        let bases = svc.list_bases().unwrap();
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0].name, "MS Patient Analysis");
    }

    // -----------------------------------------------------------------------
    // Stage 3: the scaffold, per profile (DR-6, DR-23, requirement E)
    // -----------------------------------------------------------------------

    /// Both profiles, one assertion set: whatever `create_base_in` scaffolds has
    /// to be an OKF-conformant bundle by the project's own checker — the one
    /// that has been able to answer this since Stage 0 and was never asked.
    #[test]
    fn what_create_base_scaffolds_is_conformant_by_the_projects_own_checker() {
        for format in [KbFormat::Okf, KbFormat::Biookf] {
            let (_dir, svc) = svc();
            svc.create_base_in("k", "K", None, format).unwrap();
            let kb = svc.root().join("k");

            // §11 rules 1 and 2 for `schema.md` — DR-23: it is a concept
            // document, not a third reserved file, so it needs a `type`.
            let schema = std::fs::read_to_string(kb.join("schema.md")).unwrap();
            let page = okf::Page::parse(&schema)
                .unwrap_or_else(|e| panic!("{format:?} schema.md does not parse: {e}"));
            assert_eq!(page.doc.r#type, "Schema", "{format:?}");
            assert!(page.doc.primary_key().is_some(), "{format:?} has no title");
            assert!(page.doc.description.is_some(), "{format:?}");
            let diagnostics = okf::check(&page);
            assert!(diagnostics.is_empty(), "{format:?}: {diagnostics:?}");

            // §8 for the bundle-root index, §9 for the log.
            let index = std::fs::read_to_string(kb.join("index.md")).unwrap();
            assert!(
                okf::check_index(&index, true).is_empty(),
                "{format:?}: {:?}",
                okf::check_index(&index, true)
            );
            let log = std::fs::read_to_string(kb.join("log.md")).unwrap();
            assert!(okf::check_log(&log).is_empty(), "{format:?}");
        }
    }

    /// OKF §8 permits exactly one frontmatter key in exactly one file, and
    /// DR-23's corollary is that `biookf_version` is not it. The BioOKF profile
    /// declares its own revision in `manifest.yaml` instead — which is the one
    /// place BioRouter is deliberately stricter than BioOKF's own spec.
    #[test]
    fn the_root_index_declares_okf_version_and_never_biookf_version() {
        for format in [KbFormat::Okf, KbFormat::Biookf] {
            let (_dir, svc) = svc();
            let m = svc.create_base_in("k", "K", None, format).unwrap();
            let index = std::fs::read_to_string(svc.root().join("k").join("index.md")).unwrap();
            let split = okf::frontmatter::split(&index).unwrap();
            let keys: Vec<String> = split
                .frontmatter
                .keys()
                .map(|k| k.as_str().unwrap_or("?").to_string())
                .collect();
            assert_eq!(keys, vec!["okf_version".to_string()], "{format:?}");
            assert_eq!(
                split
                    .frontmatter
                    .get("okf_version")
                    .and_then(|v| v.as_str()),
                Some(okf::OKF_VERSION),
                "{format:?}: the revision must be a STRING; unquoted, YAML reads \
                 0.2 as a float and a later 0.10 becomes 0.1"
            );

            // …and the manifest carries the pair.
            assert_eq!(
                m.okf_version.as_deref(),
                Some(okf::OKF_VERSION),
                "{format:?}"
            );
            assert_eq!(
                m.biookf_version.as_deref(),
                format.is_biookf().then_some(biookf::BIOOKF_VERSION),
                "{format:?}"
            );
        }
    }

    #[test]
    fn a_new_base_records_its_profile_and_reads_back_as_that_profile() {
        for format in [KbFormat::Okf, KbFormat::Biookf] {
            let (_dir, svc) = svc();
            let m = svc.create_base_in("k", "K", None, format).unwrap();
            assert_eq!(m.format, format);
            let on_disk = manifest::load(&svc.root().join("k")).unwrap();
            assert_eq!(
                on_disk, m,
                "the stamp on disk must match the one handed back"
            );
            assert_eq!(
                on_disk.profile(),
                Some(format),
                "a base created at the OKF generation must report its profile"
            );
        }
    }

    /// The two schemas are different documents, and the BioOKF one is generated
    /// from the vocabulary rather than typed out — so a spec bump moves the
    /// prompt instead of leaving a stale copy of the 28 types in a markdown
    /// file that nothing reads for correctness.
    #[test]
    fn the_biookf_schema_carries_the_vocabulary_the_module_declares() {
        let schema = schema_for(KbFormat::Biookf);
        assert!(
            !schema.contains("{{"),
            "an unfilled placeholder shipped into the prompt: {:?}",
            schema
                .lines()
                .filter(|l| l.contains("{{"))
                .collect::<Vec<_>>()
        );
        for t in biookf::NodeType::ALL {
            assert!(
                schema.contains(t.as_str()),
                "missing node type {}",
                t.as_str()
            );
        }
        for p in biookf::PositivePredicate::ALL {
            assert!(
                schema.contains(p.as_str()),
                "missing predicate {}",
                p.as_str()
            );
        }
        for level in biookf::KNOWLEDGE_LEVELS {
            assert!(schema.contains(level), "missing knowledge level {level}");
        }
        for agent in biookf::AGENT_TYPES {
            assert!(schema.contains(agent), "missing agent type {agent}");
        }
        // The negatives are derived from `negatable()`, not listed twice.
        let negated = format!("{}treats", biookf::NEGATION_PREFIX);
        assert!(schema.contains(&negated), "missing {negated}");

        // The OKF schema must NOT carry it: its vocabulary is open, and handing
        // the model 28 types would quietly turn it into the other profile.
        let okf_schema = schema_for(KbFormat::Okf);
        assert!(!okf_schema.contains("BiologicalPathway"), "{okf_schema}");
        assert!(!okf_schema.contains("knowledge_level"), "{okf_schema}");
    }

    /// Issue #71, stated as a property of the scaffold rather than left to be
    /// noticed later. `txn_wrote_knowledge_pages` compares ONLY the `knowledge/`
    /// subtree oid, because `log.md`, `raw/` and `index.md` each move the whole
    /// tree on their own — an ingest that wrote nothing but a log line would
    /// otherwise report as a digest. A profile that scaffolded authored content
    /// outside `knowledge/` would break that guard silently.
    #[test]
    fn every_scaffolded_directory_lives_under_knowledge() {
        for format in [KbFormat::Okf, KbFormat::Biookf] {
            let (_dir, svc) = svc();
            svc.create_base_in("k", "K", None, format).unwrap();
            let kb = svc.root().join("k");
            for dir in scaffold_dirs(format) {
                assert!(
                    kb.join("knowledge").join(&dir).is_dir(),
                    "{format:?}: knowledge/{dir} missing"
                );
                assert!(
                    !dir.starts_with('/') && !dir.contains(".."),
                    "{format:?}: {dir} escapes knowledge/"
                );
            }
            // Nothing authored at the bundle root beyond the three reserved-ish
            // files, the manifest and the ignore file.
            let mut top: Vec<String> = std::fs::read_dir(&kb)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .collect();
            top.sort();
            assert_eq!(
                top,
                vec![
                    "index.md".to_string(),
                    "knowledge".to_string(),
                    "log.md".to_string(),
                    "manifest.yaml".to_string(),
                    "raw".to_string(),
                    "schema.md".to_string(),
                ],
                "{format:?}"
            );
        }
    }

    /// The BioOKF directories are derived from SPEC §8.1's source types rather
    /// than typed out, because those four are the only directories the profile
    /// genuinely requires: every edge must cite a `primary_source`, and that
    /// citation has to resolve to a real page of one of them.
    #[test]
    fn the_biookf_scaffold_is_the_four_source_types_lowercased() {
        let dirs = scaffold_dirs(KbFormat::Biookf);
        let expected: Vec<String> = biookf::NodeType::SOURCE_TYPES
            .iter()
            .map(|t| t.as_str().to_lowercase())
            .collect();
        assert_eq!(dirs, expected);
        assert_eq!(dirs.len(), 4, "SPEC §8.1 names four, not eight");
        assert!(!dirs.contains(&"population".to_string()));
    }

    /// A user-facing create is still born PUBLIC and unclaimed whichever
    /// profile it is in — the profile argument must not have wandered into the
    /// tier or affiliation position.
    #[test]
    fn choosing_a_profile_does_not_change_how_the_base_is_classified() {
        let (_dir, svc) = svc();
        svc.create_base_in("pub-kb", "P", None, KbFormat::Biookf)
            .unwrap();
        assert!(!crate::knowledge::tier::is_private(svc.root(), "pub-kb"));
        assert_eq!(
            crate::knowledge::tier::affiliation(svc.root(), "pub-kb")
                .owners()
                .map(|o| o.len()),
            Some(0),
            "a user-facing create claims no institution"
        );
    }

    #[test]
    fn create_base_rejects_duplicate() {
        let (_dir, svc) = svc();
        svc.create_base("ms", "x", None).unwrap();
        let err = svc.create_base("ms", "y", None).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn create_base_rejects_invalid_id() {
        let (_dir, svc) = svc();
        let err = svc.create_base("BAD", "x", None).unwrap_err();
        assert!(err.to_string().contains("a-z, 0-9"), "got: {err}");
    }

    #[tokio::test]
    async fn add_raw_source_from_text() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let kb = svc.root().join("k");

        let res = svc
            .add_raw_source(
                "k",
                SourceInput::Text {
                    text: "Lab note: HRV trend up after week of zone-2.".into(),
                    title: Some("HRV note".into()),
                },
                None,
            )
            .await
            .unwrap();

        assert!(kb.join(format!("raw/{}/source.md", res.source_id)).exists());
        assert!(kb.join(format!("raw/{}/meta.yaml", res.source_id)).exists());
        let meta = raw::read_meta(&kb, &res.source_id).unwrap();
        assert_eq!(meta.title, "HRV note");
        assert_eq!(meta.credibility.tier, CredibilityTier::Personal);

        // A commit was made.
        let repo = GitRepo::open(&kb).unwrap();
        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 2, "create + add_raw_source");
        assert_eq!(log[0].kind, ChangeKind::Ingest);
    }

    #[tokio::test]
    async fn cancelling_source_conversion_does_not_stage_or_commit_raw_files() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_secs(30))
                    .set_body_string("conversion completed too late"),
            )
            .mount(&server)
            .await;

        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task_svc = svc.clone();
        let url = server.uri();
        let ingest = tokio::spawn(async move {
            task_svc
                .add_raw_source_cancelled_by("k", SourceInput::Url(url), None, Some(&task_cancel))
                .await
        });

        // ⚠ A LIVENESS wait, not a performance assertion. What this test is about
        // is cancellation semantics; reaching the mock server at all is only the
        // precondition. One second was a budget for "spawn a task, resolve a URL
        // and complete an HTTP round trip", which is a property of the machine —
        // it failed 6/6 here on a loaded box while passing in a quieter full-suite
        // run 30 minutes earlier, with and without the change under test.
        //
        // Same shape as the other wall-clock deadlines this campaign had to fix:
        // a policy row decided by the host's $HOME, a scheduler test starved by
        // its siblings, an esbuild reaper given one second to fork. Generous here
        // costs nothing when the code is right and stops a green suite going red
        // for reasons that have nothing to do with it.
        const REACHED_SERVER: std::time::Duration = std::time::Duration::from_secs(30);
        tokio::time::timeout(REACHED_SERVER, async {
            loop {
                if !server.received_requests().await.unwrap().is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the source conversion never reached the blocking HTTP response");
        cancel.cancel();

        // Cancellation itself IS bounded on purpose: it must interrupt promptly,
        // and a generous bound here would hide a cancel that never lands. 10s is
        // still two orders of magnitude above the work involved.
        let error = tokio::time::timeout(std::time::Duration::from_secs(10), ingest)
            .await
            .expect("cancellation did not interrupt source conversion")
            .expect("the source task panicked")
            .expect_err("a cancelled conversion must not report success");
        assert!(
            error
                .to_string()
                .contains("cancelled during source conversion"),
            "{error:#}"
        );

        let kb = svc.root().join("k");
        assert!(raw::list_sources(&kb).unwrap().is_empty());
        assert_eq!(
            GitRepo::open(&kb).unwrap().log(10).unwrap().len(),
            1,
            "only the base-creation commit may remain"
        );
    }

    #[tokio::test]
    async fn add_raw_source_from_html_file() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let html = b"<html><head><title>Test</title></head><body><h1>H</h1></body></html>";
        let res = svc
            .add_raw_source(
                "k",
                SourceInput::File {
                    bytes: html.to_vec(),
                    filename: "x.html".into(),
                    mime: Some("text/html".into()),
                },
                None,
            )
            .await
            .unwrap();
        let kb = svc.root().join("k");
        let md =
            std::fs::read_to_string(kb.join(format!("raw/{}/source.md", res.source_id))).unwrap();
        assert!(md.contains("# H"));
    }

    #[test]
    fn looks_machine_generated_detects_uuids_and_hashes() {
        assert!(looks_machine_generated(
            "a64e171e-f161-4615-9299-839c8a066049.pdf"
        ));
        assert!(looks_machine_generated(
            "a64e171e-f161-4615-9299-839c8a066049-pdf-48f040"
        ));
        assert!(looks_machine_generated("deadbeefdeadbeefdeadbeef"));
        assert!(!looks_machine_generated(
            "Effects of e-cigarette aerosol inhalation in mice"
        ));
        assert!(!looks_machine_generated("RNA-seq"));
    }

    #[test]
    fn title_from_markdown_prefers_first_heading() {
        let md =
            "> **Warning: poor extraction quality.**\n\n# A Study of Airway Deposition\n\nbody";
        assert_eq!(
            title_from_markdown(md).as_deref(),
            Some("A Study of Airway Deposition")
        );
    }

    #[tokio::test]
    async fn add_raw_source_rescues_uuid_filename_title_from_body() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        // An HTML upload whose filename is a UUID but whose body has a real title.
        let html = b"<html><body><h1>Intersubject Variability in Aerosol Deposition</h1>\
                     <p>Long body text describing the study in detail.</p></body></html>";
        let res = svc
            .add_raw_source(
                "k",
                SourceInput::File {
                    bytes: html.to_vec(),
                    filename: "a64e171e-f161-4615-9299-839c8a066049.html".into(),
                    mime: Some("text/html".into()),
                },
                None,
            )
            .await
            .unwrap();
        let kb = svc.root().join("k");
        let meta = raw::read_meta(&kb, &res.source_id).unwrap();
        assert!(
            !meta.title.contains("a64e171e"),
            "title should not be the UUID filename, got {}",
            meta.title
        );
        assert!(
            meta.title.to_lowercase().contains("aerosol"),
            "got {}",
            meta.title
        );
    }

    #[tokio::test]
    async fn add_raw_source_reuses_existing_source_for_same_text_hash() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let first = svc
            .add_raw_source(
                "k",
                SourceInput::Text {
                    text: "Same content".into(),
                    title: Some("First note".into()),
                },
                None,
            )
            .await
            .unwrap();

        let second = svc
            .add_raw_source(
                "k",
                SourceInput::Text {
                    text: "Same content".into(),
                    title: Some("Second note".into()),
                },
                None,
            )
            .await
            .unwrap();

        assert_eq!(first.source_id, second.source_id);

        let repo = GitRepo::open(&svc.root().join("k")).unwrap();
        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 2, "create + first add_raw_source only");
    }

    #[tokio::test]
    async fn failed_raw_post_commit_refresh_removes_old_cache_and_retry_is_idempotent() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let kb = svc.root().join("k");
        assert!(crate::knowledge::graph::cache_path(&kb).exists());
        crate::knowledge::graph::fail_cache_writes(&kb, 1);

        let err = svc
            .add_raw_source(
                "k",
                SourceInput::Text {
                    text: "Same durable source".into(),
                    title: Some("Durable note".into()),
                },
                None,
            )
            .await
            .expect_err("the injected post-commit refresh must be reported");
        let failure = err
            .downcast_ref::<RawSourceRefreshFailure>()
            .expect("durable raw state remains machine-readable");
        assert!(failure.written.commit_sha.is_some());
        let source_id = failure.written.source_id.clone();
        assert!(
            !crate::knowledge::graph::cache_path(&kb).exists(),
            "an older graph cache survived a failed post-commit rebuild"
        );
        let history_len = GitRepo::open(&kb).unwrap().log(10).unwrap().len();

        let retry = svc
            .add_raw_source(
                "k",
                SourceInput::Text {
                    text: "Same durable source".into(),
                    title: Some("Durable note".into()),
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(retry.source_id, source_id);
        assert!(retry.commit_sha.is_none());
        assert_eq!(
            GitRepo::open(&kb).unwrap().log(10).unwrap().len(),
            history_len
        );
    }

    #[tokio::test]
    async fn get_graph_returns_cached_after_create_and_add() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let g_empty = svc.get_graph("k").unwrap();
        assert!(g_empty.nodes.is_empty());
        svc.add_raw_source(
            "k",
            convert::SourceInput::Text {
                text: "note".into(),
                title: Some("N".into()),
            },
            None,
        )
        .await
        .unwrap();
        let kb = svc.root().join("k");
        // Source pages aren't written by add_raw_source — only raw/. So the graph
        // remains empty until a macro creates knowledge/sources/<id>.md (Plan 2).
        let g = svc.get_graph("k").unwrap();
        assert_eq!(g.nodes.len(), 0, "no knowledge pages yet");
        assert!(kb.join(".biorouter-knowledge/graph-cache.json").exists());
    }

    /// `GITIGNORE` is a file body, so it cannot be built from
    /// [`paths::KB_WRITE_LOCK_REL`] at compile time, which leaves it the one
    /// place the lock's path is still spelled by hand. This is the seam that
    /// closes it: rename the lock and the const stops covering it, silently,
    /// and the transient file starts appearing in every KB's git history.
    #[test]
    fn the_gitignore_still_names_the_write_lock_it_is_meant_to_hide() {
        assert!(
            GITIGNORE
                .lines()
                .any(|line| line == paths::KB_WRITE_LOCK_REL),
            "GITIGNORE does not ignore {}: {GITIGNORE:?}",
            paths::KB_WRITE_LOCK_REL
        );
    }

    /// The structural half of the Windows fix, and the only half Unix can
    /// falsify.
    ///
    /// Windows refuses to rename or remove a directory while any file inside it
    /// is open, so a write lock kept under the base it guards made `delete_base`
    /// and a base rename fail there with `Access is denied. (os error 5)` — on
    /// every machine, not as a race. Unix cannot reproduce that, but it can
    /// assert the property that makes it impossible: nothing the guard holds
    /// open lives under `kb_root`.
    #[test]
    fn the_kb_write_lock_lives_beside_the_base_not_inside_it() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let kb_root = paths::kb_root(svc.root(), "k");
        let lock_path = svc.kb_lock_path("k");

        let guard = svc.lock_existing_kb("k").unwrap();
        assert!(
            lock_path.exists(),
            "the guard did not materialise {}",
            lock_path.display()
        );
        assert!(
            !lock_path.starts_with(&kb_root),
            "the write lock {} is inside the base it locks ({}); Windows then refuses to move that base",
            lock_path.display(),
            kb_root.display()
        );
        assert!(
            !kb_root.join(paths::KB_WRITE_LOCK_REL).exists(),
            "a lock file was left at the base's historical in-tree path"
        );
        drop(guard);
    }

    /// The behavioural half. It passes trivially on Unix and is the exact
    /// operation that failed on Windows: stage the base for deletion while its
    /// own write lock is held.
    #[test]
    fn a_base_can_be_staged_for_deletion_while_its_write_lock_is_held() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let _guard = svc.lock_existing_kb("k").unwrap();
        let kb_root = paths::kb_root(svc.root(), "k");
        let staged = svc.root().join(".deleting-k-probe");

        std::fs::rename(&kb_root, &staged)
            .expect("a base must be movable while its own write lock is held");
        std::fs::rename(&staged, &kb_root).unwrap();
    }

    /// A rename moves the base *and* its lock, so the guard the renamer holds
    /// keeps excluding writers of the base under its new id. Without this the
    /// commit at the end of `update_base` would run against a base a macro
    /// could already have opened under the new id.
    #[test]
    fn renaming_a_base_carries_its_write_lock_to_the_new_id() {
        let (_dir, svc) = svc();
        svc.create_base("kb-a", "KB A", None).unwrap();
        drop(svc.lock_existing_kb("kb-a").unwrap());
        assert!(svc.kb_lock_path("kb-a").exists());

        let renamed = svc.update_base("kb-a", Some("Renamed KB"), None).unwrap();

        assert_eq!(renamed.id, "renamed-kb");
        assert!(
            !svc.kb_lock_path("kb-a").exists(),
            "the old id's lock survived the rename, so two ids now lock separately"
        );
        assert!(
            svc.kb_lock_path("renamed-kb").exists(),
            "the lock did not move to the new id"
        );
    }

    /// The lock directory must never read as a knowledge base to the scanners
    /// that walk the knowledge root by directory name (`tier::ensure_migrated_unlocked`,
    /// `soul::purge_unregistered_legacy`). Both filter on `validate_kb_id`.
    #[test]
    fn the_lock_directory_is_not_mistakable_for_a_knowledge_base() {
        assert!(paths::validate_kb_id(paths::KB_LOCKS_DIR).is_err());
    }

    /// ⚠ Windows does not report a contended lock the way Unix does: `fs2`
    /// surfaces `ERROR_LOCK_VIOLATION` (os error 33), which `std` leaves
    /// `Uncategorized`, so a `kind() == WouldBlock` test there turned every
    /// queued acquisition into a hard error. This runs on every platform and
    /// asks the real question of the real API.
    #[test]
    fn a_contended_try_lock_is_recognised_as_contention_on_every_platform() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("contended.lock");
        let held = FileLockGuard::acquire(&path).unwrap();

        let second = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let error = second
            .try_lock_exclusive()
            .expect_err("a second exclusive lock must not be granted while the first is held");

        assert!(
            is_lock_contended(&error),
            "contention was not recognised: {error:?} (kind {:?}, raw {:?}); the poll loop would \
             propagate it instead of waiting",
            error.kind(),
            error.raw_os_error()
        );
        drop(held);
    }

    /// The same question asked of `fs2`'s own name for the error, so the two
    /// cannot drift if the crate changes which code it raises.
    #[test]
    fn fs2s_own_contention_error_counts_as_contention() {
        assert!(is_lock_contended(&fs2::lock_contended_error()));
    }

    /// ⚠ Every lock wait must route contention through [`is_lock_contended`],
    /// and no Unix run can fail on the difference — which is why this is
    /// asserted at the source.
    ///
    /// `error.kind() == WouldBlock` is the shape that was here, and it is right
    /// on Unix and wrong on Windows: `fs2` raises `ERROR_LOCK_VIOLATION` there,
    /// os error 33, which `std` leaves `Uncategorized`. The arm then falls
    /// through to `Err(error) => return Err(...)`, so a *contended* lock — the
    /// ordinary case the loop exists to wait out — became a hard failure, and
    /// five tests went red on windows-latest with "another process has locked a
    /// portion of the file" surfacing where a queued wait or a cancellation
    /// message belonged.
    #[test]
    fn every_lock_wait_asks_the_platform_aware_contention_question() {
        let production = include_str!("service.rs")
            .split("\n#[cfg(test)]\nmod tests {")
            .next()
            .expect("service.rs has a production half above its tests");

        assert!(
            production.contains("Err(error) if is_lock_contended(&error) =>"),
            "the cancellable acquisition loop stopped routing contention through is_lock_contended"
        );
        let banned = concat!(
            "if error.kind() == std::io::ErrorKind::",
            "WouldBlock",
            " =>"
        );
        assert_eq!(
            production.matches(banned).count(),
            0,
            "a lock wait is matching the error KIND again; that arm is a no-op on Windows, \
             where contention is ERROR_LOCK_VIOLATION rather than WouldBlock"
        );
    }

    /// The other half of the same argument, and the one a reviewer is most
    /// likely to undo while "simplifying": the write lock's path must never be
    /// derived from [`paths::kb_root`]. Windows refuses to move a directory
    /// with an open handle underneath it, so a lock built that way breaks
    /// `delete_base` and every base rename there and nowhere else.
    /// #157. A held write lock must produce a REPORTABLE failure, not a call
    /// that never returns.
    ///
    /// Measured in a live daemon before this: writes to one base stopped
    /// answering entirely — no result, no error, 45s+ — while reads to that same
    /// base and writes to other bases were unaffected, and only a restart
    /// cleared it. This does not explain why a lock is held that long; it makes
    /// the wait finite and the failure legible.
    #[test]
    fn a_held_write_lock_times_out_with_a_sentence_instead_of_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("held.lock");

        // A real contended flock, taken and kept for the duration.
        let held = super::FileLockGuard::acquire(&lock).expect("take the lock");

        let started = std::time::Instant::now();
        let Err(err) = super::FileLockGuard::acquire_bounded(
            &lock,
            std::time::Duration::from_millis(300),
            "knowledge base 'probe'",
        ) else {
            panic!("a lock held by someone else must not be waited on forever");
        };
        let waited = started.elapsed();

        let msg = err.to_string();
        assert!(msg.contains("timed out"), "must say it timed out: {msg}");
        assert!(
            msg.contains("probe"),
            "must name the base, so the caller knows WHICH one is stuck: {msg}"
        );
        assert!(
            msg.contains("nothing was written"),
            "must say the write did not happen: {msg}"
        );
        assert!(
            waited >= std::time::Duration::from_millis(250),
            "it must actually wait for the deadline, not fail instantly: {waited:?}"
        );

        // And once the holder lets go, the same call succeeds — the bound must
        // not have broken ordinary contention.
        drop(held);
        assert!(
            super::FileLockGuard::acquire_bounded(
                &lock,
                std::time::Duration::from_secs(5),
                "knowledge base 'probe'",
            )
            .is_ok(),
            "a released lock must be takeable — the bound must not break ordinary contention"
        );
    }

    #[test]
    fn the_write_lock_path_is_never_derived_from_the_base_root() {
        let production = include_str!("service.rs")
            .split("\n#[cfg(test)]\nmod tests {")
            .next()
            .expect("service.rs has a production half above its tests");
        let body = production
            .split("fn kb_lock_path(&self, kb_id: &str) -> PathBuf {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("kb_lock_path is still a one-expression function");

        assert!(
            body.contains("kb_write_lock_path"),
            "kb_lock_path no longer delegates to paths::kb_write_lock_path: {body:?}"
        );
        assert!(
            !body.contains("kb_root"),
            "the write lock is being built under the base it locks again; Windows then \
             refuses to rename or remove that base while the lock is held: {body:?}"
        );
    }

    #[tokio::test]
    async fn merge_reauthorizes_after_waiting_for_both_kb_locks() {
        let (_dir, svc) = svc();
        svc.create_base("a-destination", "Destination", None)
            .unwrap();
        svc.create_base("z-source", "Source", None).unwrap();

        let source_lock = svc.lock_kb("z-source").await.unwrap();
        let merge_svc = svc.clone();
        let merge = tokio::spawn(async move {
            let caller = crate::knowledge::caller::KbCaller::new(
                false,
                crate::knowledge::affiliation::CallerAffiliation::Unstated,
            );
            merge_svc
                .merge_bases(
                    "a-destination",
                    "z-source",
                    &crate::knowledge::merge::MergeAuthority::Model(&caller),
                    true,
                )
                .await
        });

        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !svc.kb_queue_is_occupied("a-destination") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("merge did not acquire its first KB lock");

        svc.raise_tier_and_affiliation(
            "z-source",
            true,
            &crate::knowledge::affiliation::CallerAffiliation::Unstated,
        )
        .unwrap();
        crate::knowledge::store::write_page(
            &svc.root().join("z-source"),
            "knowledge/observation/private.md",
            "---\ntype: Observation\nidentifier: Private source\n---\n\nprivate\n",
            "private writer",
            None,
        )
        .unwrap();
        drop(source_lock);

        let error = tokio::time::timeout(std::time::Duration::from_secs(3), merge)
            .await
            .expect("merge did not resume after the source lock was released")
            .unwrap()
            .expect_err("the public merge read a source that became private while queued");
        assert!(
            error
                .to_string()
                .contains(crate::knowledge::tier::KB_PRIVATE_REFUSAL),
            "{error:#}"
        );
    }

    #[tokio::test]
    async fn lock_kb_serializes_writers() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let svc1 = svc.clone();
        let svc2 = svc.clone();
        let h1 = tokio::spawn(async move {
            let _g = svc1.lock_kb("k").await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            std::time::Instant::now()
        });
        // Brief delay so h1 acquires the lock first.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let h2 = tokio::spawn(async move {
            let _g = svc2.lock_kb("k").await.unwrap();
            std::time::Instant::now()
        });
        let t1 = h1.await.unwrap();
        let t2 = h2.await.unwrap();
        assert!(
            t2 >= t1,
            "h2 must observe lock acquisition after h1 released"
        );
    }

    #[tokio::test]
    async fn acquiring_the_kb_lock_recovers_a_crash_orphan_without_a_timeout() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let kb = svc.root().join("k");
        let repo = GitRepo::open(&kb).unwrap();
        let txn = repo.begin_txn("orphan").unwrap();
        std::fs::write(kb.join("orphan.md"), "uncommitted after crash").unwrap();
        repo.commit_on_txn(&txn, "orphaned write").unwrap();
        drop(repo);

        let _guard = svc.lock_kb("k").await.unwrap();
        assert!(!kb.join("orphan.md").exists());
        let recovered = git2::Repository::open(&kb).unwrap();
        assert!(recovered
            .find_branch(&txn.branch, git2::BranchType::Local)
            .is_err());
        assert!(matches!(
            recovered.head().unwrap().shorthand(),
            Some("main" | "master")
        ));
    }

    #[test]
    fn a_synchronous_lock_recovers_a_crash_orphan_without_a_timeout() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let kb = svc.root().join("k");
        let repo = GitRepo::open(&kb).unwrap();
        let txn = repo.begin_txn("sync-orphan").unwrap();
        std::fs::write(kb.join("orphan.md"), "uncommitted after crash").unwrap();
        repo.commit_on_txn(&txn, "orphaned write").unwrap();
        drop(repo);

        svc.get_graph("k").unwrap();
        assert!(!kb.join("orphan.md").exists());
        let recovered = git2::Repository::open(&kb).unwrap();
        assert!(recovered
            .find_branch(&txn.branch, git2::BranchType::Local)
            .is_err());
        assert!(matches!(
            recovered.head().unwrap().shorthand(),
            Some("main" | "master")
        ));
    }

    /// #159: a READ gives up long before a write does, so an open transaction
    /// cannot make a base unreadable.
    ///
    /// The asymmetry is the whole fix, so both halves are asserted here: a
    /// read's deadline must be shorter than a write's, and the read path must
    /// actually use it. Held with a bare `FileLockGuard` so only the on-disk
    /// half blocks, matching the live wedge.
    #[tokio::test]
    async fn a_read_gives_up_far_sooner_than_a_write_when_the_base_is_locked() {
        assert!(
            FileLockGuard::KB_READ_LOCK_WAIT < FileLockGuard::KB_WRITE_LOCK_WAIT,
            "a read must not wait as long as a write: losing a read costs nothing \
             and it can be retried, so making it wait out the longest legitimate \
             writer is all cost and no benefit"
        );

        let (_dir, svc) = svc();
        let kb = "read-vs-write";
        svc.create_base(kb, "K", None).unwrap();
        let held = svc.lock_existing_kb(kb).expect("hold the on-disk lock");

        // The read path's own entry point, at a short deadline, so this asserts
        // the wiring rather than the constant.
        let waited = std::time::Instant::now();
        let error = match svc
            .lock_kb_path_waiting(kb, None, std::time::Duration::from_millis(150))
            .await
        {
            Ok(_) => panic!("a read behind a held lock must give up, not hang"),
            Err(error) => error,
        };
        assert!(waited.elapsed() >= std::time::Duration::from_millis(150));
        assert!(format!("{error:#}").contains("timed out"), "{error:#}");
        drop(held);

        // …and with the lock free it succeeds, so the bound is not refusing
        // every read.
        svc.lock_kb_path_waiting(kb, None, std::time::Duration::from_millis(150))
            .await
            .expect("an uncontended read must still acquire the lock");
    }

    /// A merge PREVIEW takes the read deadline; a real merge takes the write one.
    ///
    /// `kb_merge_preview` writes nothing, and the server does not even take a
    /// transaction slot for it — but `merge_bases` locked both bases at the
    /// WRITE deadline whatever `dry_run` said. #159 had just raised that from
    /// 120s to 1800s so a real merge could outlast a long ingest, which turned a
    /// read-only preview into a THIRTY-MINUTE park behind any open transaction
    /// on either base.
    ///
    /// Found by the live tool sweep, not by this suite: an earlier case left a
    /// transaction open on the destination and the preview never came back. The
    /// daemon log showed the same `kb_merge_preview` dispatched twice, five
    /// minutes apart, with nothing in between.
    ///
    /// The assertion is a DURATION, because that is the whole defect. Asserting
    /// only that the preview errors would pass against the 1800s version too —
    /// eventually. What matters is that it gives up while someone is still
    /// waiting for it.
    #[tokio::test]
    async fn a_merge_preview_gives_up_on_a_locked_base_instead_of_waiting_out_a_write() {
        let (_dir, svc) = svc();
        svc.create_base("dst", "Destination", None).unwrap();
        svc.create_base("src", "Source", None).unwrap();

        // Hold the destination exactly as an open transaction does.
        let held = svc.lock_existing_kb("dst").expect("hold the destination");

        // `User`, not `Model`: the point is the DEADLINE, and a Model authority
        // would refuse at the tier barrier before ever reaching the lock.
        let proof = crate::knowledge::merge::UserKbMerge::for_test();
        let authority = crate::knowledge::merge::MergeAuthority::User(&proof);
        let started = std::time::Instant::now();
        let outcome = tokio::time::timeout(
            FileLockGuard::KB_READ_LOCK_WAIT + std::time::Duration::from_secs(30),
            svc.merge_bases("dst", "src", &authority, true),
        )
        .await;
        let elapsed = started.elapsed();
        drop(held);

        let error = outcome
            .expect("a preview must not outlast the READ deadline — it took the write one")
            .expect_err("a preview behind a held lock must refuse, not succeed");
        assert!(format!("{error:#}").contains("timed out"), "{error:#}");
        assert!(
            elapsed < FileLockGuard::KB_WRITE_LOCK_WAIT,
            "a preview waited {elapsed:?}, i.e. the WRITE deadline; it must use the read one"
        );
    }

    /// Every WRITE-deadline lock site is a write, by name.
    ///
    /// This is a repo-grep guard, and it exists because the same defect has now
    /// been found three times in three places: `merge_bases` and `kb_export`
    /// first, then `lint` and `query`. Each time the audit that fixed it looked
    /// at one file and the next instance was in another. A read that takes the
    /// 1800s write deadline does not fail — it *parks*, for thirty minutes,
    /// behind any open transaction, and the user reports a hang with no error.
    ///
    /// So the check is over the tree rather than over a list of known sites: if
    /// you add a caller of a write-deadline helper, either it is a write and
    /// belongs in the table below, or it is a read and belongs on
    /// `lock_kb_cancellable_for_read` / `lock_existing_kb_for_read`.
    ///
    /// ⚠ The right fix on a failure is almost never to add a row. Ask what the
    /// call DOES first — three of the five instances so far were reads.
    #[test]
    fn no_read_path_takes_the_write_deadline() {
        // Source files that may take a write-deadline lock, and why each is a
        // write. A file absent from this map may not take one at all.
        let permitted: std::collections::HashMap<&str, &str> = [
            (
                "knowledge/server.rs",
                "kb_write_page, kb_add_raw_source, kb_begin_txn, kb_append_log",
            ),
            (
                "knowledge/service.rs",
                "the helpers themselves, plus real mutations",
            ),
            ("knowledge/macros/lint.rs", "the autofix arm only"),
            ("knowledge/macros/query.rs", "the file_as_page arm only"),
            ("knowledge/macros/ingest.rs", "ingest writes"),
            ("knowledge/conversation_ingest.rs", "ingest writes"),
        ]
        .into_iter()
        .collect();

        let write_helpers = [
            "lock_kb(",
            "lock_kb_cancellable(",
            "lock_existing_kb_cancellable(",
            "lock_kb_path_cancellable(",
        ];

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders: Vec<String> = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)
                .expect("read the source tree")
                .flatten()
            {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let rel = path
                    .strip_prefix(&root)
                    .expect("under src")
                    .to_string_lossy()
                    .replace('\\', "/");
                if permitted.contains_key(rel.as_str()) {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("read a source file");
                // Production code only. A TEST that holds a write lock on
                // purpose — to prove a waiter blocks — is doing exactly what it
                // should, and the first run of this guard flagged one
                // (`knowledge/tier_user.rs`, a fixture holding `lock_kb`). Test
                // modules sit at the end of a file in this crate, so everything
                // from the first `#[cfg(test)]` onwards is out of scope.
                // `split_at`, not `&source[..at]`: `clippy::string_slice` is denied
                // repo-wide. The index came from `find` so it is a char boundary
                // here, but the slice form invites one that is not.
                let production = match source.find("#[cfg(test)]") {
                    Some(at) => source.split_at(at).0,
                    None => source.as_str(),
                };
                for (n, line) in production.lines().enumerate() {
                    let code = line.trim_start();
                    // Skip prose, so a comment naming the rule cannot trip it —
                    // this repo has shipped that bug three times.
                    if code.starts_with("//") || code.starts_with('*') {
                        continue;
                    }
                    if write_helpers.iter().any(|h| line.contains(h)) {
                        offenders.push(format!("{rel}:{} — {}", n + 1, code.trim()));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these take the 1800s WRITE deadline from a file that is not listed as a \
             writer. If the call is a read, use `lock_kb_cancellable_for_read` or \
             `lock_existing_kb_for_read`; only add a row if it genuinely writes:\n  {}",
            offenders.join("\n  ")
        );

        // ⚠ The sweep above is FILE-granular, which measurement showed is not
        // enough on its own: reverting `lint`'s read arm to the write deadline
        // leaves it green, because `lint.rs` is a permitted file. The permission
        // is for its autofix arm only, so the two files that branch must be
        // held to actually having the read arm.
        for (file, discriminator) in [
            ("knowledge/macros/lint.rs", "autofix"),
            ("knowledge/macros/query.rs", "file_as_page"),
        ] {
            let source = std::fs::read_to_string(root.join(file)).expect("read the macro");
            assert!(
                source.contains("lock_kb_cancellable_for_read"),
                "{file} is permitted to take the write deadline for its `{discriminator}` \
                 arm ONLY. It no longer takes the read deadline anywhere, so its read \
                 path is parking for thirty minutes behind an open transaction."
            );
        }
    }

    /// A merge honours Stop. It holds TWO locks; neither may ignore the token.
    ///
    /// Found by driving the app: the tool sweep stalled on `kb_merge`, and the
    /// reason was that `merge_bases` passed `None` for the cancellation token on
    /// both `lock_kb_path_waiting` calls. A merge is the one operation that
    /// takes two KB locks, in id order, at the WRITE deadline — so a caller
    /// pressing Stop waited out up to 2 x 1800s while the turn looked cancelled.
    ///
    /// The dry-run half was already fixed (it takes the read deadline). This is
    /// the real merge, where the long deadline is correct and the missing
    /// cancellation was not.
    #[tokio::test]
    async fn a_merge_stops_when_the_caller_cancels() {
        let (_dir, svc) = svc();
        svc.create_base("dst", "Destination", None).unwrap();
        svc.create_base("src", "Source", None).unwrap();

        // Hold the first lock in id order ("dst" < "src") so the merge queues.
        let held = svc.lock_existing_kb("dst").expect("hold the destination");
        let token = CancellationToken::new();
        token.cancel();

        let proof = crate::knowledge::merge::UserKbMerge::for_test();
        let authority = crate::knowledge::merge::MergeAuthority::User(&proof);
        let started = std::time::Instant::now();
        let error = svc
            .merge_bases_cancellable("dst", "src", &authority, false, Some(&token))
            .await
            .expect_err("a cancelled merge must not proceed");
        let elapsed = started.elapsed();
        drop(held);

        assert!(
            elapsed < std::time::Duration::from_secs(30),
            "a cancelled merge waited {elapsed:?} — it is ignoring the token and \
             will sit out the 1800s write deadline"
        );
        let text = format!("{error:#}");
        assert!(
            text.contains("cancel") || text.contains("timed out"),
            "the refusal should say it was cancelled: {text}"
        );
    }

    /// #157, the OTHER unbounded wait on the same path — the on-disk `flock`.
    ///
    /// This is the one that actually wedged a live daemon. Held here by a
    /// SEPARATE `FileLockGuard` rather than by a `lock_kb` guard, which is what
    /// makes it a file-lock test: the in-process mutex is free, so a waiter
    /// walks straight past the bound above and blocks in `acquire_cancellable`.
    /// Exactly the stack a `sample` of the wedged daemon showed.
    ///
    /// Fails against the shipped-at-the-time code, which awaited that
    /// `spawn_blocking` with no deadline and simply never returned.
    #[tokio::test]
    async fn a_waiter_that_cannot_get_the_on_disk_lock_gives_up_and_names_the_base() {
        let (_dir, svc) = svc();
        let kb = "flock-wedged-b2";
        svc.create_base(kb, "K", None).unwrap();

        // The leak, reproduced: the file lock is held and nothing holds the
        // in-process mutex, so only the `flock` half can block.
        let held = svc.lock_existing_kb(kb).expect("take the on-disk lock");
        assert!(
            !svc.kb_queue_is_occupied(kb),
            "the in-process queue must be FREE"
        );

        let waited = std::time::Instant::now();
        let error = match svc
            .lock_kb_path_waiting(kb, None, std::time::Duration::from_millis(200))
            .await
        {
            Ok(_) => panic!("a waiter behind a held on-disk lock must time out, not hang"),
            Err(error) => error,
        };
        let elapsed = waited.elapsed();

        let message = format!("{error:#}");
        assert!(message.contains("timed out"), "{message}");
        assert!(
            message.contains("on-disk lock"),
            "must say WHICH lock: {message}"
        );
        assert!(
            message.contains(kb),
            "the base must be named so the user knows which one is stuck: {message}"
        );
        assert!(message.contains("Nothing was written"), "{message}");
        assert!(
            elapsed >= std::time::Duration::from_millis(200),
            "returned in {elapsed:?} — that is not the deadline firing"
        );
        drop(held);
    }

    /// #157, the shipped behaviour: a caller that cannot get the queue lock
    /// eventually gives up and SAYS SO, instead of never returning.
    ///
    /// Driven at 150 ms rather than the shipped 30 minutes — the duration is
    /// arithmetic, the wiring is what this proves.
    #[tokio::test]
    async fn a_waiter_that_cannot_get_the_kb_queue_gives_up_and_names_the_base() {
        let (_dir, svc) = svc();
        // ⚠ A DISTINCTIVE id, not the `"k"` the tests around this one use. With
        // `"k"` the naming assertion below is satisfied by the letter k in
        // "knowledge base" — it passed against a message with the id stripped
        // out of it entirely, which is the whole property it claims to check.
        let kb = "wedged-base-7f3";
        svc.create_base(kb, "K", None).unwrap();
        let held = svc.lock_kb(kb).await.unwrap();

        let waited = std::time::Instant::now();
        let error = match svc
            .lock_kb_path_waiting(kb, None, std::time::Duration::from_millis(150))
            .await
        {
            Ok(_) => panic!("a waiter behind a held lock must time out, not hang"),
            Err(error) => error,
        };
        let elapsed = waited.elapsed();

        let message = format!("{error:#}");
        assert!(message.contains("timed out"), "{message}");
        assert!(
            message.contains(kb),
            "the base must be named so the user knows WHICH one is stuck: {message}"
        );
        assert!(
            message.contains("Nothing \\\nwas written.") || message.contains("Nothing was written"),
            "the caller must be told no write landed: {message}"
        );
        // Both causes, because the message cannot tell them apart.
        assert!(message.contains("let it finish"), "{message}");
        assert!(
            message.contains("kb_commit_txn"),
            "the message must name the fix for the COMMON cause — an open transaction: {message}"
        );
        assert!(
            elapsed >= std::time::Duration::from_millis(150),
            "returned in {elapsed:?} — that is not the deadline firing"
        );
        drop(held);
    }

    /// The same deadline on the CANCELLABLE arm.
    ///
    /// This is the arm that matters most and the one I first left unbounded:
    /// nearly every tool handler in `server.rs` passes `Some(context.ct)`, so the
    /// arm that reads like the special case is the one the whole tool surface
    /// takes. A cancel token bounds the wait only if somebody actually cancels.
    #[tokio::test]
    async fn the_cancellable_arm_is_bounded_too_when_nobody_cancels() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let held = svc.lock_kb("k").await.unwrap();

        let never_fired = CancellationToken::new();
        let error = match svc
            .lock_kb_path_waiting(
                "k",
                Some(&never_fired),
                std::time::Duration::from_millis(150),
            )
            .await
        {
            Ok(_) => panic!("an uncancelled waiter must still hit the deadline"),
            Err(error) => error,
        };

        let message = format!("{error:#}");
        assert!(
            message.contains("timed out"),
            "expected the deadline, got: {message}"
        );
        assert!(!never_fired.is_cancelled());
        drop(held);
    }

    /// ...and cancellation still wins over the deadline when it does fire, with
    /// its own distinct message. Restructuring the two arms into one `timeout`
    /// could have collapsed these into a single "timed out" answer.
    #[tokio::test]
    async fn cancelling_a_queued_waiter_still_reports_cancellation_not_a_timeout() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let held = svc.lock_kb("k").await.unwrap();

        let cancel = CancellationToken::new();
        let fires = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            fires.cancel();
        });
        let error = match svc
            .lock_kb_path_waiting("k", Some(&cancel), std::time::Duration::from_secs(30))
            .await
        {
            Ok(_) => panic!("a cancelled waiter must not acquire the lock"),
            Err(error) => error,
        };

        let message = format!("{error:#}");
        assert!(message.contains("cancelled"), "{message}");
        assert!(
            !message.contains("timed out"),
            "cancellation must not be reported as the deadline: {message}"
        );
        drop(held);
    }

    /// #157. The lock wait must exceed the longest hold it can legitimately be
    /// waiting on, or it stops reporting wedges and starts inventing them.
    ///
    /// A macro holds the KB lock across its whole sub-agent loop, so every
    /// `max_wall` budget anywhere in the workspace is a lower bound on a
    /// legitimate hold. `KB_WRITE_LOCK_WAIT` was first written as 120 s against a
    /// default budget of 300 s and a CLI budget of 900 s — every long ingest would
    /// have failed its concurrent callers with a message blaming a stuck holder.
    /// Grepping the tree rather than naming the constants is deliberate: the CLI's
    /// budget lives in a crate that depends on this one, so it cannot be imported
    /// here, and a hand-copied list is exactly what drifts.
    #[test]
    fn the_lock_wait_exceeds_every_macro_wall_clock_budget() {
        // CARGO_MANIFEST_DIR is <workspace>/crates/biorouter-mcp; go up twice.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let crates = root.join("crates");
        assert!(
            crates.is_dir(),
            "this guard walks {}; if that path is wrong every assertion below \
             passes for the wrong reason",
            crates.display()
        );

        // `max_wall: Duration::from_secs(300)` and `const MAX_WALL_SECS: u64 = 30`.
        let literal = regex::Regex::new(r"max_wall\s*:\s*Duration::from_secs\((\d+)\)").unwrap();
        let named = regex::Regex::new(r"MAX_WALL_SECS\s*:\s*u64\s*=\s*(\d+)").unwrap();

        let mut scanned = 0usize;
        let mut budgets: Vec<(String, u64)> = vec![];
        for entry in walkdir::WalkDir::new(&crates) {
            let entry = entry.expect("this guard must not silently skip an unreadable directory");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("unreadable source file {}: {e}", path.display()));
            scanned += 1;
            let rel = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            for caps in literal
                .captures_iter(&text)
                .chain(named.captures_iter(&text))
            {
                budgets.push((rel.clone(), caps[1].parse().unwrap()));
            }
        }

        // A walk that reads nothing is indistinguishable from a walk that finds
        // nothing wrong, so make both ways of doing no work loud.
        assert!(
            scanned > 500,
            "only {scanned} source files scanned — the walk is not covering the \
             workspace, so this guard proves nothing"
        );
        assert!(
            budgets.len() >= 3,
            "found {} wall-clock budgets; the sub-agent default, the CLI's and the \
             credibility fallback's are all expected, so a shortfall means the \
             patterns have gone stale and stopped matching",
            budgets.len()
        );

        let wait = FileLockGuard::KB_WRITE_LOCK_WAIT.as_secs();
        for (file, budget) in &budgets {
            assert!(
                *budget < wait,
                "{file} allows a macro to run for {budget}s while a caller waiting \
                 for that macro's knowledge base gives up after {wait}s. The waiter \
                 would fail an operation that is working normally, and blame a stuck \
                 lock for it. Raise KB_WRITE_LOCK_WAIT above {budget}s."
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_queued_file_lock_never_blocks_the_tokio_worker() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let external = FileLockGuard::acquire(&svc.kb_lock_path("k")).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(external);
        });

        let mut acquisition = Box::pin(svc.lock_kb("k"));
        tokio::select! {
            biased;
            () = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
            _ = &mut acquisition => panic!("file-lock acquisition ran on and blocked the Tokio worker"),
        }

        let _guard = acquisition.await.unwrap();
        releaser.join().unwrap();
    }

    #[tokio::test]
    async fn cancellation_releases_a_waiter_queued_on_the_process_lock() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let first = svc.lock_kb("k").await.unwrap();
        let cancel = CancellationToken::new();
        let waiting_svc = svc.clone();
        let waiting_cancel = cancel.clone();
        let waiting = tokio::spawn(async move {
            waiting_svc
                .lock_kb_cancellable("k", Some(&waiting_cancel))
                .await
        });
        tokio::task::yield_now().await;

        cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
            .await
            .expect("cancellation must not wait for the current lock holder")
            .unwrap();
        let error = match result {
            Ok(_) => panic!("the queued acquisition was not cancelled"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("cancelled"), "{error:#}");

        drop(first);
        let _next = svc.lock_kb("k").await.unwrap();
    }

    #[tokio::test]
    async fn repeated_cancellation_terminates_file_lock_waiters() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let external = FileLockGuard::acquire(&svc.kb_lock_path("k")).unwrap();
        for _ in 0..16 {
            let cancel = CancellationToken::new();
            let waiting_svc = svc.clone();
            let waiting_cancel = cancel.clone();
            let waiting = tokio::spawn(async move {
                waiting_svc
                    .lock_kb_cancellable("k", Some(&waiting_cancel))
                    .await
            });
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;

            cancel.cancel();
            let result = tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
                .await
                .expect("cancellation must terminate the file-lock poller")
                .unwrap();
            assert!(
                result.is_err(),
                "the file-lock acquisition ignored cancellation"
            );
            let process_queue = Arc::clone(
                svc.locks
                    .get("k")
                    .expect("the acquisition registered a process queue")
                    .value(),
            );
            assert!(
                process_queue.try_lock_owned().is_ok(),
                "a cancelled waiter survived after its caller returned"
            );
        }

        drop(external);
        let _next = tokio::time::timeout(std::time::Duration::from_secs(1), svc.lock_kb("k"))
            .await
            .expect("cancelled file-lock pollers accumulated behind the released lock")
            .unwrap();
    }

    #[tokio::test]
    async fn repeated_cancellation_terminates_root_lock_waiters() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let external = FileLockGuard::acquire(&svc.root_lock_path()).unwrap();
        for _ in 0..16 {
            let cancel = CancellationToken::new();
            let waiting_svc = svc.clone();
            let waiting_cancel = cancel.clone();
            let waiting = tokio::spawn(async move {
                waiting_svc
                    .raise_tier_and_affiliation_cancelled_by(
                        "k",
                        true,
                        &Default::default(),
                        Some(&waiting_cancel),
                    )
                    .await
            });
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;

            cancel.cancel();
            let result = tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
                .await
                .expect("cancellation must terminate the root-lock poller")
                .unwrap();
            assert!(result.is_err(), "the root-lock waiter ignored cancellation");
            assert!(!crate::knowledge::tier::is_private(svc.root(), "k"));
        }

        drop(external);
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            svc.raise_tier_and_affiliation_async("k", true, &Default::default()),
        )
        .await
        .expect("cancelled root-lock pollers accumulated behind the released lock")
        .unwrap();
        assert!(crate::knowledge::tier::is_private(svc.root(), "k"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_async_update_keeps_the_tokio_worker_responsive() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let external = FileLockGuard::acquire(&svc.kb_lock_path("k")).unwrap();
        let cancel = CancellationToken::new();
        let update_svc = svc.clone();
        let update_cancel = cancel.clone();
        let update = tokio::spawn(async move {
            update_svc
                .update_base_async("k", None, Some("#ffffff"), Some(&update_cancel))
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        cancel.cancel();
        let error = tokio::time::timeout(std::time::Duration::from_secs(1), update)
            .await
            .expect("the Axum-style mutation blocked the current-thread runtime")
            .unwrap()
            .expect_err("the cancelled mutation unexpectedly updated the base");
        assert!(error.to_string().contains("cancelled"), "{error:#}");
        assert_ne!(svc.get_base("k").unwrap().color, "#ffffff");
        drop(external);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn queued_reclassification_never_blocks_the_tokio_worker() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let external = FileLockGuard::acquire(&svc.kb_lock_path("k")).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(external);
        });

        let mut reclassify = Box::pin(svc.reclassify_source("k", "missing-source"));
        tokio::select! {
            biased;
            () = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
            _ = &mut reclassify => panic!("source reclassification blocked the Tokio worker while acquiring flock"),
        }
        drop(reclassify);
        releaser.join().unwrap();
        let _next = tokio::time::timeout(std::time::Duration::from_secs(1), svc.lock_kb("k"))
            .await
            .expect("the dropped reclassification left a detached file-lock waiter")
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn queued_restore_never_blocks_the_tokio_worker() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let external = FileLockGuard::acquire(&svc.kb_lock_path("k")).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(external);
        });

        let mut restore = Box::pin(svc.restore_state_async("k", "missing-commit", None));
        tokio::select! {
            biased;
            () = tokio::time::sleep(std::time::Duration::from_millis(25)) => {}
            _ = &mut restore => panic!("state restore blocked the Tokio worker while acquiring flock"),
        }
        drop(restore);
        releaser.join().unwrap();
        let _next = tokio::time::timeout(std::time::Duration::from_secs(1), svc.lock_kb("k"))
            .await
            .expect("the dropped restore left a detached file-lock waiter")
            .unwrap();
    }

    #[tokio::test]
    async fn rename_waits_for_the_macro_lock_without_holding_the_root_lock() {
        let (_dir, svc) = svc();
        svc.create_base("kb-a", "KB A", None).unwrap();
        let macro_guard = svc.lock_kb("kb-a").await.unwrap();

        let (rename_tx, rename_rx) = std::sync::mpsc::channel();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let rename_svc = svc.clone();
        let rename = std::thread::spawn(move || {
            let _ = started_tx.send(());
            let _ = rename_tx.send(rename_svc.update_base("kb-a", Some("Renamed KB"), None));
        });
        started_rx.recv().unwrap();
        assert!(
            rename_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "rename bypassed the macro transaction lock"
        );

        let (root_tx, root_rx) = std::sync::mpsc::channel();
        let root_svc = svc.clone();
        let root = std::thread::spawn(move || {
            let _ = root_tx.send(root_svc.raise_tier_and_affiliation(
                "kb-a",
                false,
                &Default::default(),
            ));
        });
        let root_result = root_rx.recv_timeout(std::time::Duration::from_secs(1));
        drop(macro_guard);
        let renamed = rename_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("rename did not resume after the macro released its lock")
            .unwrap();
        rename.join().unwrap();
        root.join().unwrap();

        root_result
            .expect("rename held the root lock while waiting for the KB lock")
            .unwrap();
        assert_eq!(renamed.id, "renamed-kb");
        assert!(!svc.root().join("kb-a").exists());
        assert!(svc.root().join("renamed-kb").exists());
        let _renamed_guard = svc.lock_kb("renamed-kb").await.unwrap();
    }

    #[tokio::test]
    async fn set_default_model_waits_for_the_macro_lock() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let macro_guard = svc.lock_kb("k").await.unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let update_svc = svc.clone();
        let update = std::thread::spawn(move || {
            let _ = started_tx.send(());
            let _ = tx.send(update_svc.set_default_model(
                "k",
                Some(ModelRef {
                    provider: "test".into(),
                    model: "model".into(),
                }),
            ));
        });
        started_rx.recv().unwrap();

        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "default-model update bypassed the macro transaction lock"
        );
        drop(macro_guard);
        let updated = rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("default-model update did not resume")
            .unwrap();
        update.join().unwrap();
        assert_eq!(updated.default_model.unwrap().provider, "test");
    }

    #[tokio::test]
    async fn delete_waits_for_the_macro_lock() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let macro_guard = svc.lock_kb("k").await.unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let delete_svc = svc.clone();
        let delete = std::thread::spawn(move || {
            let _ = started_tx.send(());
            let _ = tx.send(delete_svc.delete_base("k"));
        });
        started_rx.recv().unwrap();

        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "delete bypassed the macro transaction lock"
        );
        drop(macro_guard);
        rx.recv_timeout(std::time::Duration::from_secs(1))
            .expect("delete did not resume")
            .unwrap();
        delete.join().unwrap();
        assert!(!svc.root().join("k").exists());
    }

    #[tokio::test]
    async fn graph_reads_wait_behind_the_macro_lock() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let macro_guard = svc.lock_kb("k").await.unwrap();
        let read_svc = svc.clone();
        let mut read = tokio::spawn(async move { read_svc.get_graph_async("k").await });

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), &mut read)
                .await
                .is_err(),
            "a graph read bypassed the macro's per-KB queue"
        );
        drop(macro_guard);

        read.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn reclassify_source_updates_meta() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();

        // Add a text source (no URL → falls back to Personal tier).
        let written = svc
            .add_raw_source(
                "k",
                SourceInput::Text {
                    text: "lab note".into(),
                    title: Some("note".into()),
                },
                None,
            )
            .await
            .unwrap();

        // Reclassify — same text, should still come back Personal.
        let cred = svc
            .reclassify_source("k", &written.source_id)
            .await
            .unwrap();
        assert_eq!(cred.tier, CredibilityTier::Personal);

        // Verify meta.yaml was updated.
        let kb = svc.root().join("k");
        let meta = raw::read_meta(&kb, &written.source_id).unwrap();
        assert_eq!(meta.credibility.tier, CredibilityTier::Personal);

        // A new commit was made.
        let repo = crate::knowledge::git::GitRepo::open(&kb).unwrap();
        let log = repo.log(10).unwrap();
        // create + add_raw + reclassify = 3 commits
        assert!(log.len() >= 3);
    }

    #[tokio::test]
    async fn override_credibility_writes_and_commits() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();

        let written = svc
            .add_raw_source(
                "k",
                SourceInput::Text {
                    text: "draft".into(),
                    title: Some("draft".into()),
                },
                None,
            )
            .await
            .unwrap();

        let override_cred = Credibility {
            tier: CredibilityTier::PeerReviewed,
            confidence: 0.99,
            publisher: Some("Nature".into()),
            venue: Some("Nature 2024".into()),
            doi: Some("10.1000/xyz".into()),
            retracted: false,
            reasoning: "Manual override: confirmed peer-reviewed publication.".into(),
            classifier_version: 1,
        };

        let returned = svc
            .override_credibility("k", &written.source_id, override_cred.clone())
            .unwrap();
        assert_eq!(returned.tier, CredibilityTier::PeerReviewed);
        assert_eq!(returned.doi.as_deref(), Some("10.1000/xyz"));

        // Verify meta.yaml persisted the override.
        let kb = svc.root().join("k");
        let meta = raw::read_meta(&kb, &written.source_id).unwrap();
        assert_eq!(meta.credibility.tier, CredibilityTier::PeerReviewed);
        assert_eq!(meta.credibility.doi.as_deref(), Some("10.1000/xyz"));

        // A commit was made with the override.
        let repo = crate::knowledge::git::GitRepo::open(&kb).unwrap();
        let log = repo.log(10).unwrap();
        assert!(log[0].summary.contains("override credibility"));
        assert_eq!(log[0].kind, ChangeKind::Manual);
    }

    // -----------------------------------------------------------------------
    // check_model tests
    // -----------------------------------------------------------------------

    use crate::knowledge::subagent::loop_::{Completer, LlmMessage, LlmReply};
    use async_trait::async_trait;
    use rmcp::model::Tool;

    struct OkCompleter;

    #[async_trait]
    impl Completer for OkCompleter {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[LlmMessage],
            _tools: &[Tool],
        ) -> anyhow::Result<LlmReply> {
            Ok(LlmReply {
                text: "OK".to_string(),
                tool_calls: vec![],
            })
        }
    }

    struct ErrCompleter;

    #[async_trait]
    impl Completer for ErrCompleter {
        async fn complete(
            &self,
            _system: &str,
            _messages: &[LlmMessage],
            _tools: &[Tool],
        ) -> anyhow::Result<LlmReply> {
            anyhow::bail!("provider unreachable")
        }
    }

    #[tokio::test]
    async fn check_model_ok_with_mock_completer() {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.check_model(Box::new(OkCompleter)).await.unwrap();
    }

    #[tokio::test]
    async fn check_model_propagates_completer_error() {
        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        let err = svc.check_model(Box::new(ErrCompleter)).await.unwrap_err();
        assert!(
            err.to_string().contains("provider unreachable"),
            "error should mention underlying cause, got: {err}"
        );
    }

    #[tokio::test]
    async fn list_history_and_restore_roundtrip() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        svc.add_raw_source(
            "k",
            convert::SourceInput::Text {
                text: "first".into(),
                title: Some("a".into()),
            },
            None,
        )
        .await
        .unwrap();
        let history_after_one = svc.list_history("k", 10).unwrap();
        assert_eq!(history_after_one.len(), 2);
        let target = history_after_one.last().unwrap().commit_sha.clone();

        svc.add_raw_source(
            "k",
            convert::SourceInput::Text {
                text: "second".into(),
                title: Some("b".into()),
            },
            None,
        )
        .await
        .unwrap();
        let history_after_two = svc.list_history("k", 10).unwrap();
        assert_eq!(history_after_two.len(), 3);

        svc.restore_state_async("k", &target, None).await.unwrap();
        let history_after_restore = svc.list_history("k", 10).unwrap();
        assert_eq!(history_after_restore.len(), 4);
        assert_eq!(
            history_after_restore[0].kind,
            crate::knowledge::types::ChangeKind::Restore
        );
    }

    #[tokio::test]
    async fn restore_refuses_a_pre_okf_commit_without_mutating_the_current_base() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let kb = svc.root().join("k");
        let repo = GitRepo::open(&kb).unwrap();

        let mut manifest = manifest::load(&kb).unwrap();
        manifest.schema_version = AUTOMATIC_SCHEMA_CEILING;
        manifest::save(&kb, &manifest).unwrap();
        let legacy_sha = repo
            .commit_all(ChangeKind::Manual, "legacy checkpoint", None)
            .unwrap();

        manifest.schema_version = CURRENT_SCHEMA_VERSION;
        manifest.format = KbFormat::Okf;
        manifest.okf_version = Some("0.2".to_string());
        manifest::save(&kb, &manifest).unwrap();
        let current_sha = repo
            .commit_all(ChangeKind::Manual, "current checkpoint", None)
            .unwrap();

        let error = svc
            .restore_state_async("k", &legacy_sha, None)
            .await
            .expect_err("a restore must not reintroduce the retired format");
        assert!(
            error
                .downcast_ref::<LegacyKnowledgeRestoreUnsupported>()
                .is_some(),
            "{error:#}"
        );
        assert_eq!(svc.list_history("k", 1).unwrap()[0].commit_sha, current_sha);
        assert_eq!(svc.require_current_profile("k").unwrap(), KbFormat::Okf);
        assert!(kb.exists());
    }

    #[tokio::test]
    async fn restore_cache_failure_reports_the_committed_phase_and_sha() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let target = svc.list_history("k", 10).unwrap()[0].commit_sha.clone();
        svc.add_raw_source(
            "k",
            convert::SourceInput::Text {
                text: "later state".into(),
                title: Some("later".into()),
            },
            None,
        )
        .await
        .unwrap();
        let kb = svc.root().join("k");
        crate::knowledge::graph::fail_cache_writes(&kb, 1);

        let error = svc
            .restore_state_async("k", &target, None)
            .await
            .expect_err("the injected post-commit refresh must be reported");
        let failure = error
            .downcast_ref::<KnowledgeWriteFailure>()
            .expect("restore retains its durable outcome as a typed failure");
        assert_eq!(
            failure.phase,
            crate::knowledge::git::KnowledgeWriteFailurePhase::Committed
        );
        let commit_sha = failure
            .commit_sha
            .as_deref()
            .expect("a committed restore carries its exact sha");
        assert_eq!(svc.list_history("k", 10).unwrap()[0].commit_sha, commit_sha);
    }

    #[test]
    fn primary_kb_persists_to_disk() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        assert!(svc.get_primary_persisted()?.is_none());

        svc.set_primary_persisted(Some("my-kb"))?;
        assert_eq!(svc.get_primary_persisted()?.as_deref(), Some("my-kb"));

        // Setting again overwrites.
        svc.set_primary_persisted(Some("other-kb"))?;
        assert_eq!(svc.get_primary_persisted()?.as_deref(), Some("other-kb"));

        // Clearing removes the file.
        svc.set_primary_persisted(None)?;
        assert!(svc.get_primary_persisted()?.is_none());

        // Invalid IDs are rejected.
        let err = svc.set_primary_persisted(Some("INVALID--KB"));
        assert!(err.is_err());
        Ok(())
    }

    #[test]
    fn primary_kb_can_be_scoped_per_session() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());

        assert!(svc.get_primary_for_session("session-a")?.is_none());
        assert!(svc.get_primary_for_session("session-b")?.is_none());

        svc.set_primary_for_session("session-a", Some("kb-a"))?;
        svc.set_primary_for_session("session-b", Some("kb-b"))?;

        assert_eq!(
            svc.get_primary_for_session("session-a")?.as_deref(),
            Some("kb-a")
        );
        assert_eq!(
            svc.get_primary_for_session("session-b")?.as_deref(),
            Some("kb-b")
        );

        svc.set_primary_for_session("session-a", None)?;
        assert!(svc.get_primary_for_session("session-a")?.is_none());
        assert_eq!(
            svc.get_primary_for_session("session-b")?.as_deref(),
            Some("kb-b")
        );

        Ok(())
    }

    #[test]
    fn session_scoped_primary_kb_tracks_rename_and_delete() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());

        svc.create_base("kb-a", "KB A", None)?;
        svc.set_primary_persisted(Some("kb-a"))?;
        svc.set_primary_for_session("session-a", Some("kb-a"))?;
        svc.set_primary_for_session("session-b", Some("kb-a"))?;

        let renamed = svc.update_base("kb-a", Some("Renamed KB"), None)?;
        assert_eq!(renamed.id, "renamed-kb");
        assert_eq!(svc.get_primary_persisted()?.as_deref(), Some("renamed-kb"));
        assert_eq!(
            svc.get_primary_for_session("session-a")?.as_deref(),
            Some("renamed-kb")
        );
        assert_eq!(
            svc.get_primary_for_session("session-b")?.as_deref(),
            Some("renamed-kb")
        );

        svc.delete_base("renamed-kb")?;
        assert!(svc.get_primary_persisted()?.is_none());
        assert!(svc.get_primary_for_session("session-a")?.is_none());
        assert!(svc.get_primary_for_session("session-b")?.is_none());

        Ok(())
    }

    #[test]
    fn hidden_kbs_can_be_scoped_per_session() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());

        assert!(svc.get_hidden_persisted()?.is_empty());
        assert!(svc.get_hidden_for_session("session-a")?.is_empty());

        svc.set_hidden_persisted(&["kb-a".to_string(), "kb-b".to_string()])?;
        assert_eq!(
            svc.get_hidden_for_session_or_persisted("session-a")?,
            vec!["kb-a".to_string(), "kb-b".to_string()]
        );

        svc.set_hidden_for_session("session-a", &["kb-c".to_string()])?;
        assert_eq!(
            svc.get_hidden_for_session("session-a")?,
            vec!["kb-c".to_string()]
        );
        assert_eq!(
            svc.get_hidden_for_session_or_persisted("session-a")?,
            vec!["kb-c".to_string()]
        );
        assert_eq!(
            svc.get_hidden_for_session_or_persisted("session-b")?,
            vec!["kb-a".to_string(), "kb-b".to_string()]
        );

        // Setting an empty list is an explicit override ("hide nothing here"),
        // not a request to fall back to the machine-wide list. See
        // `session_hidden_override_can_be_explicitly_empty`.
        svc.set_hidden_for_session("session-a", &[])?;
        assert!(svc.get_hidden_for_session("session-a")?.is_empty());
        assert!(svc
            .get_hidden_for_session_or_persisted("session-a")?
            .is_empty());

        Ok(())
    }

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
        // one — but only while that session actually holds the base.
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

    /// Two chats, one gesture, one answer. A chat that *pinned* alpha and a
    /// chat that *inherited* alpha from the machine pointer show the user the
    /// same thing — "alpha is this chat's primary" — so hiding alpha must land
    /// them in the same place.
    ///
    /// It did not. `repair_decision` returned early unless the scope's own file
    /// was `Pinned`, and an inheriting session's own file is `Inherit`, so the
    /// repair never saw the pointer it was meant to repair: the pinning chat
    /// came back promoted to beta, the inheriting chat came back with no
    /// primary at all. The inheriting case is the common one — most chats never
    /// pin their own primary — so the divergence was the default experience.
    #[test]
    fn hiding_the_primary_promotes_whether_the_chat_pinned_or_inherited_it() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        for id in ["alpha", "beta", "gamma"] {
            svc.create_base(id, id, None)?;
        }
        svc.set_primary_persisted(Some("alpha"))?;
        svc.set_primary_for_session("pinned", Some("alpha"))?;

        // Identical starting state as far as the user can tell.
        assert_eq!(
            svc.primary_for_session(Some("pinned"))?.as_deref(),
            Some("alpha")
        );
        assert_eq!(
            svc.primary_for_session(Some("inherits"))?.as_deref(),
            Some("alpha")
        );

        let pinned = svc.hide_kb(Some("pinned"), "alpha", PrimaryUpdate::Unchanged)?;
        let inherits = svc.hide_kb(Some("inherits"), "alpha", PrimaryUpdate::Unchanged)?;
        assert_eq!(
            pinned.primary_kb, inherits.primary_kb,
            "the same gesture from the same visible state must give the same primary"
        );
        assert_eq!(inherits.primary_kb.as_deref(), Some("beta"));

        // The promotion is persisted as this chat's *own* pin: it has diverged
        // from the machine default, which is what a session pin means.
        assert_eq!(
            svc.get_primary_for_session("inherits")?.as_deref(),
            Some("beta"),
            "the promotion must be persisted, not re-derived on every read"
        );
        // And the machine pointer is untouched, so every other chat still
        // follows it.
        assert_eq!(svc.get_primary_persisted()?.as_deref(), Some("alpha"));
        assert_eq!(
            svc.primary_for_session(Some("bystander"))?.as_deref(),
            Some("alpha")
        );

        // Hiding the rest leaves the chat with no primary — and it stays gone
        // rather than re-inheriting the machine default it has moved away from,
        // and is never re-invented when a base comes back into the set.
        let emptied = svc.set_visible_kbs(Some("inherits"), &[], PrimaryUpdate::Unchanged)?;
        assert_eq!(emptied.primary_kb, None);
        let restored = svc.include_kb(Some("inherits"), "gamma", PrimaryUpdate::Unchanged)?;
        assert_eq!(
            restored.primary_kb, None,
            "a chat that has been left with no primary must not be handed one"
        );
        Ok(())
    }

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
        assert_eq!(
            std::fs::read_to_string(&hidden_tmp)?,
            "half-written garbage"
        );
        Ok(())
    }

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

    /// Issue #56, QA 2026-09-10 H2: a caller that cannot reach every base
    /// changes only the bases it can. Each rule is driven against the one a
    /// plausible wrong implementation would break — taking the submitted set
    /// literally, clearing a primary it was never shown, dropping a pin it could
    /// not see, and naming the rest of the machine's bases in a refusal.
    #[test]
    fn a_limited_caller_changes_only_what_it_can_see() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        for id in ["alpha", "beta", "secret"] {
            svc.create_base(id, id, None)?;
        }
        let sees = |id: &str| id != "secret";

        // The user pins `secret` as this chat's primary, with nothing hidden.
        svc.set_selection(Some("s1"), Some(&[]), PrimaryUpdate::Set("secret"))?;

        // A limited caller rewrites the set naming only what it saw: `secret`
        // stays visible (it was not hidden) and `beta` is hidden as asked.
        let sel = svc.set_selection_within(
            Some("s1"),
            Some(&["beta".to_string()]),
            PrimaryUpdate::Unchanged,
            &sees,
        )?;
        assert_eq!(sel.hidden_kbs, vec!["beta".to_string()]);
        assert_eq!(sel.primary_kb.as_deref(), Some("secret"));

        // It asks to clear the primary it was shown as none: `secret` stays.
        let sel = svc.set_selection_within(Some("s1"), None, PrimaryUpdate::Clear, &sees)?;
        assert_eq!(
            sel.primary_kb.as_deref(),
            Some("secret"),
            "cleared a hidden primary"
        );
        // …and to inherit, which would drop this chat's pin on `secret`: stays.
        let sel = svc.set_selection_within(Some("s1"), None, PrimaryUpdate::Inherit, &sees)?;
        assert_eq!(
            sel.primary_kb.as_deref(),
            Some("secret"),
            "dropped a hidden pin"
        );

        // It may not hide `secret` by naming it, nor pin it.
        let sel = svc.set_selection_within(
            Some("s1"),
            Some(&["secret".to_string()]),
            PrimaryUpdate::Unchanged,
            &sees,
        )?;
        assert!(
            sel.hidden_kbs.is_empty(),
            "hid a base it could not see: {sel:?}"
        );
        let err = svc
            .set_selection_within(Some("s1"), None, PrimaryUpdate::Set("secret"), &sees)
            .unwrap_err()
            .to_string();
        assert!(!err.contains("alpha") && !err.contains("beta"), "{err}");

        // The user hides `secret`; a limited caller that "un-hides everything"
        // leaves it hidden.
        svc.set_selection(
            Some("s1"),
            Some(&["secret".to_string()]),
            PrimaryUpdate::Set("alpha"),
        )?;
        let sel =
            svc.set_selection_within(Some("s1"), Some(&[]), PrimaryUpdate::Unchanged, &sees)?;
        assert_eq!(sel.hidden_kbs, vec!["secret".to_string()]);

        // A refusal names only what the caller can see: hiding `beta` while
        // pinning it fails, and the list of what IS available omits `secret`
        // even though `secret` is not hidden from the set at this point.
        svc.set_selection(Some("s1"), Some(&[]), PrimaryUpdate::Set("alpha"))?;
        let err = svc
            .set_selection_within(
                Some("s1"),
                Some(&["beta".to_string()]),
                PrimaryUpdate::Set("beta"),
                &sees,
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("alpha"), "{err}");
        assert!(
            !err.contains("secret"),
            "a refusal named a base the caller cannot see: {err}"
        );

        // A caller that sees everything is `set_selection`, byte for byte.
        let everything = |_: &str| true;
        let sel =
            svc.set_selection_within(Some("s1"), Some(&[]), PrimaryUpdate::Clear, &everything)?;
        assert_eq!(sel.primary_kb, None);
        Ok(())
    }

    /// The membership primitives every caller actually needs, so none of them
    /// has to read the hidden list, edit it and write it back. Each takes the
    /// whole gesture and applies it under one root lock.
    #[test]
    fn membership_primitives_apply_one_gesture_under_one_lock() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        for id in ["alpha", "beta", "gamma"] {
            svc.create_base(id, id, None)?;
        }

        // Hide one base; the pointer is untouched because it was not the one hidden.
        svc.set_selection(Some("s1"), Some(&[]), PrimaryUpdate::Set("alpha"))?;
        let sel = svc.hide_kb(Some("s1"), "gamma", PrimaryUpdate::Unchanged)?;
        assert_eq!(sel.kb_ids, vec!["alpha".to_string(), "beta".to_string()]);
        assert_eq!(sel.hidden_kbs, vec!["gamma".to_string()]);
        assert_eq!(sel.primary_kb.as_deref(), Some("alpha"));

        // Hiding is idempotent, and hiding an uninstalled base is accepted.
        assert_eq!(
            svc.hide_kb(Some("s1"), "gamma", PrimaryUpdate::Unchanged)?,
            sel
        );
        svc.hide_kb(Some("s1"), "ghost", PrimaryUpdate::Unchanged)?;
        assert_eq!(
            svc.selection(Some("s1"))?.kb_ids,
            vec!["alpha".to_string(), "beta".to_string()]
        );

        // Add a base back and make it primary in the same operation — the
        // combined gesture the apps platform and the chat chip both need.
        let sel = svc.include_kb(Some("s1"), "gamma", PrimaryUpdate::Set("gamma"))?;
        assert_eq!(
            sel.kb_ids,
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
        assert_eq!(sel.primary_kb.as_deref(), Some("gamma"));

        // Including a base that does not exist is an error, not a silent no-op.
        let err = svc
            .include_kb(Some("s1"), "ghost", PrimaryUpdate::Unchanged)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("ghost") && err.contains("does not exist"),
            "got: {err}"
        );
        assert_eq!(
            svc.selection(Some("s1"))?,
            sel,
            "a rejected include changes nothing"
        );

        // The visible-set form is the exact inverse of hidden_kbs.
        let sel = svc.set_visible_kbs(
            Some("s1"),
            &["beta".to_string(), "gamma".to_string()],
            PrimaryUpdate::Unchanged,
        )?;
        assert_eq!(sel.kb_ids, vec!["beta".to_string(), "gamma".to_string()]);
        assert_eq!(sel.hidden_kbs, vec!["alpha".to_string()]);
        assert_eq!(
            sel.primary_kb.as_deref(),
            Some("gamma"),
            "the pointer stays put while it is still a member"
        );

        // Dropping the primary out of the visible set repairs it, once.
        let sel =
            svc.set_visible_kbs(Some("s1"), &["beta".to_string()], PrimaryUpdate::Unchanged)?;
        assert_eq!(sel.kb_ids, vec!["beta".to_string()]);
        assert_eq!(sel.primary_kb.as_deref(), Some("beta"));

        // And they validate the primary against the resulting set, all-or-nothing.
        let before = svc.selection(Some("s1"))?;
        assert!(svc
            .set_visible_kbs(
                Some("s1"),
                &["beta".to_string()],
                PrimaryUpdate::Set("alpha")
            )
            .is_err());
        assert_eq!(svc.selection(Some("s1"))?, before);

        // Machine scope works the same way.
        let sel = svc.hide_kb(None, "beta", PrimaryUpdate::Unchanged)?;
        assert_eq!(sel.kb_ids, vec!["alpha".to_string(), "gamma".to_string()]);
        Ok(())
    }

    /// The reason these primitives exist. Hiding four different bases from four
    /// threads used to be four read-modify-write cycles across two unlocked
    /// calls, so each writer persisted a list computed before the others' edits
    /// and updates were silently lost. Every hide must survive.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_hides_of_different_bases_all_survive() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        let ids = ["alpha", "beta", "gamma", "delta"];
        for id in ids {
            svc.create_base(id, id, None)?;
        }

        let mut tasks = Vec::new();
        for id in ids {
            let svc = svc.clone();
            tasks.push(tokio::task::spawn_blocking(
                move || -> anyhow::Result<()> {
                    svc.hide_kb(Some("s1"), id, PrimaryUpdate::Unchanged)?;
                    Ok(())
                },
            ));
        }
        for task in tasks {
            task.await??;
        }

        let sel = svc.selection(Some("s1"))?;
        assert!(
            sel.kb_ids.is_empty(),
            "every concurrent hide must survive, got {:?}",
            sel.kb_ids
        );
        assert_eq!(sel.hidden_kbs.len(), ids.len());
        Ok(())
    }

    /// Repair must tell "deleted" from "hidden". A pointer at a base that is no
    /// longer installed is a dangling reference and must be *cleared*; only a
    /// base that still exists and was merely hidden may promote. Conflating the
    /// two meant an upgrade whose `.active-kb` named a since-removed base read
    /// as no-primary at first and then, on the next entirely unrelated hide,
    /// invented one — the single thing the merged model forbids.
    #[test]
    fn repair_clears_a_stale_pointer_and_promotes_only_a_hidden_one() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        for id in ["alpha", "beta", "gamma"] {
            svc.create_base(id, id, None)?;
        }

        // An upgrade (or an out-of-band edit): the machine pointer names a base
        // that is not installed. It correctly reads as no primary …
        std::fs::write(
            crate::knowledge::paths::primary_kb_path(svc.root()),
            b"ghost",
        )?;
        assert_eq!(svc.primary_for_session(None)?, None);

        // … and an unrelated set edit must not turn it into one.
        let sel =
            svc.set_selection(None, Some(&["gamma".to_string()]), PrimaryUpdate::Unchanged)?;
        assert_eq!(
            sel.primary_kb, None,
            "a primary must never be invented from a dangling pointer"
        );
        assert_eq!(
            svc.get_primary_persisted()?,
            None,
            "the dangling pointer is cleared, not promoted"
        );

        // A pointer at a base that still exists but was hidden *does* promote:
        // the user ranked it, and there is still a set to fall back into.
        svc.set_selection(None, Some(&[]), PrimaryUpdate::Set("beta"))?;
        let sel = svc.set_selection(None, Some(&["beta".to_string()]), PrimaryUpdate::Unchanged)?;
        assert_eq!(sel.primary_kb.as_deref(), Some("alpha"));
        assert_eq!(svc.get_primary_persisted()?.as_deref(), Some("alpha"));

        // Same rule at session scope, where a wrong promotion is worse: the
        // machine now points at alpha, so an invented session pointer would be
        // indistinguishable from an inherited one.
        let session_file = svc.primary_session_path("s1");
        std::fs::create_dir_all(session_file.parent().unwrap())?;
        std::fs::write(&session_file, b"ghost")?;
        assert_eq!(svc.primary_for_session(Some("s1"))?, None);

        let sel = svc.set_selection(
            Some("s1"),
            Some(&["gamma".to_string()]),
            PrimaryUpdate::Unchanged,
        )?;
        assert_eq!(sel.primary_kb, None, "a session must not invent one either");
        assert_eq!(svc.get_primary_for_session("s1")?, None);
        assert_eq!(svc.primary_for_session(Some("s1"))?, None);
        Ok(())
    }

    /// A `KbSelection` is a claim — its `primary_kb` is a member of its own
    /// `kb_ids` — and `selection()` composed it from three separately-unlocked
    /// reads, so a writer landing between them made the claim false. Genuinely
    /// concurrent on purpose: this interleaving does not exist on one thread,
    /// so no single-threaded test can catch it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn selection_is_a_coherent_snapshot_under_a_concurrent_writer() -> anyhow::Result<()> {
        use std::sync::atomic::{AtomicBool, Ordering};

        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("alpha", "Alpha", None)?;
        svc.create_base("beta", "Beta", None)?;

        let stop = Arc::new(AtomicBool::new(false));
        let writer = {
            let svc = svc.clone();
            let stop = stop.clone();
            tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                while !stop.load(Ordering::Relaxed) {
                    // beta in the set and pinned …
                    svc.set_selection(Some("s1"), Some(&[]), PrimaryUpdate::Set("beta"))?;
                    // … then beta out of the set entirely, which repairs the
                    // pointer onto alpha. The reader must never see a snapshot
                    // that straddles the two.
                    svc.set_selection(
                        Some("s1"),
                        Some(&["beta".to_string()]),
                        PrimaryUpdate::Unchanged,
                    )?;
                }
                Ok(())
            })
        };

        let reader = {
            let svc = svc.clone();
            tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                for _ in 0..3000 {
                    let sel = svc.selection(Some("s1"))?;
                    if let Some(primary) = &sel.primary_kb {
                        anyhow::ensure!(
                            sel.kb_ids.contains(primary),
                            "incoherent snapshot: primary {primary} is not in kb_ids {:?}",
                            sel.kb_ids
                        );
                    }
                }
                Ok(())
            })
        };

        let observed = reader.await?;
        stop.store(true, Ordering::Relaxed);
        writer.await??;
        observed?;
        Ok(())
    }

    /// All-or-nothing, or the invariant does not hold. The set was written
    /// before the requested primary was validated, so "hide the base I am
    /// pinned to, and pin one that does not exist" persisted the hide and
    /// *then* returned an error — leaving the stored pointer sitting outside
    /// the resulting set, which is precisely the state the merged model exists
    /// to forbid.
    #[test]
    fn a_rejected_set_selection_persists_nothing() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        for id in ["alpha", "beta"] {
            svc.create_base(id, id, None)?;
        }
        svc.set_selection(Some("s1"), Some(&[]), PrimaryUpdate::Set("beta"))?;
        let before = svc.selection(Some("s1"))?;
        assert_eq!(before.primary_kb.as_deref(), Some("beta"));

        // Hide the pinned base *and* ask for a primary that does not exist.
        let err = svc
            .set_selection(
                Some("s1"),
                Some(&["beta".to_string()]),
                PrimaryUpdate::Set("ghost"),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("ghost"), "got: {err}");

        assert_eq!(
            svc.selection(Some("s1"))?,
            before,
            "a rejected request must not persist the half of it that ran first"
        );
        assert_eq!(
            svc.get_primary_for_session("s1")?.as_deref(),
            Some("beta"),
            "the stored pointer must still be a member of the stored set"
        );

        // A malformed id in the set half is rejected the same way.
        let err = svc
            .set_selection(
                Some("s1"),
                Some(&["NOT VALID".to_string()]),
                PrimaryUpdate::Unchanged,
            )
            .unwrap_err()
            .to_string();
        assert!(!err.is_empty());
        assert_eq!(svc.selection(Some("s1"))?, before);

        // Same at machine scope, where the vocabulary differs but the rule does not.
        svc.set_selection(None, Some(&[]), PrimaryUpdate::Set("alpha"))?;
        let before = svc.selection(None)?;
        assert!(svc
            .set_selection(
                None,
                Some(&["alpha".to_string()]),
                PrimaryUpdate::Set("ghost")
            )
            .is_err());
        assert_eq!(svc.selection(None)?, before);
        Ok(())
    }

    /// Three states, not two. "This chat has no primary" and "this chat never
    /// said" look the same to an `Option<String>` reader, so clearing a
    /// session's primary used to delete its file — the encoding of "no
    /// opinion" — and the machine-wide default walked straight back in, silently
    /// re-arming the KB-less write the user had just disarmed.
    #[test]
    fn a_session_can_explicitly_have_no_primary_while_the_machine_has_one() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("alpha", "Alpha", None)?;
        svc.create_base("beta", "Beta", None)?;
        svc.set_primary_persisted(Some("alpha"))?;

        // A chat that never chose one inherits the machine default.
        assert_eq!(
            svc.primary_for_session(Some("s1"))?.as_deref(),
            Some("alpha")
        );

        // Clearing is an override, not a reset-to-inherit.
        let sel = svc.set_selection(Some("s1"), None, PrimaryUpdate::Clear)?;
        assert_eq!(sel.primary_kb, None);
        assert_eq!(
            svc.primary_for_session(Some("s1"))?,
            None,
            "a cleared session must stay cleared, not re-inherit the machine primary"
        );

        // The machine pointer is untouched and other chats still follow it.
        assert_eq!(svc.get_primary_persisted()?.as_deref(), Some("alpha"));
        assert_eq!(
            svc.primary_for_session(Some("s2"))?.as_deref(),
            Some("alpha")
        );

        // The override is durable: it survives a fresh service over the same
        // root, i.e. it is genuinely on disk and not an in-memory artefact.
        let reopened = KnowledgeService::new(tmp.path().to_path_buf());
        assert_eq!(reopened.primary_for_session(Some("s1"))?, None);
        assert_eq!(reopened.selection(Some("s1"))?.primary_kb, None);
        assert_eq!(reopened.get_primary_for_session("s1")?, None);

        // Set-only edits must not resurrect it either.
        let sel = svc.set_selection(Some("s1"), Some(&[]), PrimaryUpdate::Unchanged)?;
        assert_eq!(sel.primary_kb, None);

        // Dropping the override is a separate, explicit gesture.
        let sel = svc.set_selection(Some("s1"), None, PrimaryUpdate::Inherit)?;
        assert_eq!(sel.primary_kb.as_deref(), Some("alpha"));
        svc.set_primary_for_session("s1", None)?;
        assert_eq!(svc.primary_for_session(Some("s1"))?, None);
        svc.clear_primary_override_for_session("s1")?;
        assert_eq!(
            svc.primary_for_session(Some("s1"))?.as_deref(),
            Some("alpha")
        );
        Ok(())
    }

    /// Deleting the base a chat had pinned must leave that chat with *no*
    /// primary. The delete rewriter removed the session's file, which is
    /// "never chose" — so the chat silently adopted the machine-wide default
    /// it had never selected, and the next KB-less write landed there.
    #[test]
    fn deleting_a_pinned_base_does_not_hand_the_session_the_machine_primary() -> anyhow::Result<()>
    {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("alpha", "Alpha", None)?;
        svc.create_base("beta", "Beta", None)?;
        svc.set_primary_persisted(Some("alpha"))?;
        svc.set_primary_for_session("s1", Some("beta"))?;

        svc.delete_base("beta")?;

        assert_eq!(
            svc.primary_for_session(Some("s1"))?,
            None,
            "the chat pinned the deleted base; it must not inherit 'alpha' now"
        );
        assert_eq!(
            svc.primary_for_session(Some("s2"))?.as_deref(),
            Some("alpha"),
            "a chat that never chose one still follows the machine default"
        );
        Ok(())
    }

    /// The four ways a directory can fail to be a legacy base, against the one
    /// way it can be one. `Manifest::is_legacy_format` answers `true` for every
    /// row below, which is why nothing destructive may consult it.
    #[test]
    fn only_a_stated_generation_over_a_knowledge_base_tree_classifies_as_legacy(
    ) -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base_in("real", "Real", None, KbFormat::Biookf)?;
        let real = svc.root().join("real");
        assert_eq!(classify_base_format(&real), BaseFormat::Current);

        let mut stamped = manifest::load(&real)?;
        stamped.schema_version = crate::knowledge::types::AUTOMATIC_SCHEMA_CEILING;
        manifest::save(&real, &stamped)?;
        assert_eq!(classify_base_format(&real), BaseFormat::Legacy);

        // The same generation, minus the tree that makes it a base of ours.
        let foreign = svc.root().join("foreign");
        std::fs::create_dir_all(&foreign)?;
        std::fs::write(manifest::manifest_path(&foreign), "schema_version: 1\n")?;
        assert!(matches!(
            classify_base_format(&foreign),
            BaseFormat::Undiagnosable(_)
        ));

        // The tree, minus the statement. A mapping that deserializes cleanly
        // and reads as legacy…
        let undeclared = svc.root().join("undeclared");
        std::fs::create_dir_all(undeclared.join("knowledge"))?;
        std::fs::write(undeclared.join("schema.md"), "# schema\n")?;
        std::fs::write(manifest::manifest_path(&undeclared), "{}\n")?;
        assert!(
            manifest::load(&undeclared)?.is_legacy_format(),
            "fixture must reproduce the trap"
        );
        assert!(matches!(
            classify_base_format(&undeclared),
            BaseFormat::Undiagnosable(_)
        ));

        // …and one that does not deserialize at all.
        std::fs::write(manifest::manifest_path(&undeclared), "id: [unclosed\n")?;
        assert!(manifest::load(&undeclared).is_err());
        assert!(matches!(
            classify_base_format(&undeclared),
            BaseFormat::Undiagnosable(_)
        ));

        // No manifest at all.
        let bare = svc.root().join("bare");
        std::fs::create_dir_all(&bare)?;
        assert!(matches!(
            classify_base_format(&bare),
            BaseFormat::Undiagnosable(_)
        ));
        Ok(())
    }

    fn deletion_fixture(
    ) -> anyhow::Result<(tempfile::TempDir, KnowledgeService, DeleteMetadataSnapshot)> {
        let tmp = tempfile::tempdir()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("victim", "Victim", None)?;
        svc.create_base("keep", "Keep", None)?;
        std::fs::write(
            svc.root().join("victim/knowledge/marker.md"),
            "---\ntype: Observation\nidentifier: marker\n---\n",
        )?;
        svc.set_hidden_persisted(&["victim".to_string()])?;
        svc.set_hidden_for_session("session-a", &["victim".to_string()])?;
        svc.set_primary_persisted(Some("victim"))?;
        svc.set_primary_for_session("session-a", Some("victim"))?;
        svc.raise_tier_and_affiliation(
            "victim",
            true,
            &crate::knowledge::affiliation::CallerAffiliation::Institution("ucsf".to_string()),
        )?;
        let snapshot = DeleteMetadataSnapshot::capture(svc.root())?;
        Ok((tmp, svc, snapshot))
    }

    #[test]
    fn deletion_rolls_back_every_metadata_phase_and_the_full_base_tree() -> anyhow::Result<()> {
        for fault in [
            DeleteCheckpoint::Staged,
            DeleteCheckpoint::Registry,
            DeleteCheckpoint::MachinePrimary,
            DeleteCheckpoint::SessionPrimaries,
            DeleteCheckpoint::HiddenSelections,
            DeleteCheckpoint::Classification,
        ] {
            let (_tmp, svc, before) = deletion_fixture()?;
            let _kb_lock = svc.lock_existing_kb("victim")?;
            let error = svc
                .delete_base_under_kb_lock_with_checkpoint("victim", None, |checkpoint| {
                    anyhow::ensure!(checkpoint != fault, "injected failure after {fault:?}");
                    Ok(())
                })
                .expect_err("the selected delete checkpoint must fail");
            assert!(error.to_string().contains("fully rolled back"), "{error:#}");
            assert_eq!(
                DeleteMetadataSnapshot::capture(svc.root())?,
                before,
                "{fault:?}"
            );
            assert!(svc.root().join("victim/knowledge/marker.md").exists());
            assert!(svc.get_base("victim").is_ok());
            assert!(!std::fs::read_dir(svc.root())?.any(|entry| {
                entry.is_ok_and(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".deleting-victim-")
                })
            }));
        }
        Ok(())
    }

    #[test]
    fn successful_delete_cleans_every_reference_and_classification() -> anyhow::Result<()> {
        let (_tmp, svc, _before) = deletion_fixture()?;
        svc.delete_base("victim")?;

        assert!(!svc.root().join("victim").exists());
        assert!(svc.get_base("victim").is_err());
        assert_eq!(svc.get_primary_persisted()?, None);
        assert_eq!(svc.get_primary_for_session("session-a")?, None);
        assert!(svc.get_hidden_persisted()?.is_empty());
        assert!(svc.get_hidden_for_session("session-a")?.is_empty());
        assert!(!crate::knowledge::tier::has_metadata_unlocked(
            svc.root(),
            "victim"
        )?);
        assert!(svc.base_is_current_or_fully_removed("victim")?);
        assert_eq!(
            registry::load(svc.root())?
                .into_iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec!["keep".to_string()]
        );
        Ok(())
    }

    #[test]
    fn startup_finishes_a_logically_committed_delete_and_erases_the_staged_tree(
    ) -> anyhow::Result<()> {
        let (_tmp, svc, _before) = deletion_fixture()?;
        let staged = svc
            .root()
            .join(format!(".deleting-victim-{}", uuid::Uuid::new_v4()));
        std::fs::rename(svc.root().join("victim"), &staged)?;
        registry::unregister(svc.root(), "victim")?;

        assert!(!svc.base_is_current_or_fully_removed("victim")?);
        assert_eq!(
            svc.resume_pending_delete_cleanup()?,
            vec!["victim".to_string()]
        );
        assert!(!staged.exists());
        assert!(svc.base_is_current_or_fully_removed("victim")?);
        assert_eq!(svc.get_primary_persisted()?, None);
        assert_eq!(svc.get_primary_for_session("session-a")?, None);
        assert!(svc.get_hidden_persisted()?.is_empty());
        assert!(svc.get_hidden_for_session("session-a")?.is_empty());
        assert!(!crate::knowledge::tier::has_metadata_unlocked(
            svc.root(),
            "victim"
        )?);
        Ok(())
    }

    #[test]
    fn startup_restores_a_staged_base_that_is_still_registered() -> anyhow::Result<()> {
        let (_tmp, svc, _before) = deletion_fixture()?;
        let staged = svc
            .root()
            .join(format!(".deleting-victim-{}", uuid::Uuid::new_v4()));
        std::fs::rename(svc.root().join("victim"), &staged)?;

        assert!(svc.resume_pending_delete_cleanup()?.is_empty());
        assert!(!staged.exists());
        assert!(svc.root().join("victim/knowledge/marker.md").exists());
        assert!(svc.get_base("victim").is_ok());
        Ok(())
    }

    /// …and the chat it leaves that way must have a way back. The blank
    /// override is durable by design, which made it a **one-way door**: no set
    /// edit lifts it, `Clear` reinstates it, and `Set` is only available if the
    /// user wants to pin something specific. A chat that lost its pinned base
    /// to a delete could therefore never follow the machine-wide default again.
    /// `PrimaryUpdate::Inherit` is the escape hatch, and it must leave the
    /// chat genuinely *following* the pointer rather than having copied it.
    #[test]
    fn inherit_lifts_the_no_primary_override_a_delete_installed() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        for id in ["alpha", "beta", "gamma", DEFAULT_PRIMARY_KB_ID] {
            svc.create_base(id, id, None)?;
        }
        svc.set_primary_persisted(Some("alpha"))?;
        svc.set_primary_for_session("s1", Some("gamma"))?;
        svc.delete_base("gamma")?;
        assert_eq!(svc.primary_for_session(Some("s1"))?, None);

        // Every other gesture leaves the override standing.
        svc.set_selection(Some("s1"), Some(&[]), PrimaryUpdate::Unchanged)?;
        assert_eq!(svc.primary_for_session(Some("s1"))?, None);
        svc.set_selection(Some("s1"), None, PrimaryUpdate::Clear)?;
        assert_eq!(svc.primary_for_session(Some("s1"))?, None);

        let sel = svc.set_selection(Some("s1"), None, PrimaryUpdate::Inherit)?;
        assert_eq!(sel.primary_kb.as_deref(), Some("alpha"));

        // Following, not a one-time copy: the chat tracks later machine moves.
        svc.set_primary_persisted(Some("beta"))?;
        assert_eq!(
            svc.primary_for_session(Some("s1"))?.as_deref(),
            Some("beta")
        );
        assert_eq!(
            svc.get_primary_for_session("s1")?,
            None,
            "inheriting means holding no pointer of its own"
        );

        // At machine scope Inherit restores the product default Soul. It is
        // distinct from Clear and leaves no preference file behind.
        let sel = svc.set_selection(None, None, PrimaryUpdate::Inherit)?;
        assert_eq!(sel.primary_kb.as_deref(), Some(DEFAULT_PRIMARY_KB_ID));
        assert!(!crate::knowledge::paths::primary_kb_path(svc.root()).exists());
        Ok(())
    }

    /// Hiding Soul is a user decision, and the product default must not undo it.
    ///
    /// The default resolves at READ time — an absent machine pointer becomes
    /// `Pinned("soul")` in `effective_primary_unlocked` — and that resolution
    /// runs BEFORE the visible set is consulted. So the question this answers is
    /// whether the repair path downstream actually filters the resolved default
    /// against what the scope can see, or whether a hidden Soul comes back as
    /// the primary anyway. Reasoning said the former; this measures it.
    ///
    /// Written while completing an uncommitted work-in-progress that solved the
    /// same problem by WRITING the pointer once at startup instead. That
    /// approach was discarded as superseded — resolving at read time needs no
    /// lock, no startup write and cannot go stale — but its test list named this
    /// case, and it was the one case main had no test for.
    #[test]
    fn a_hidden_soul_does_not_come_back_as_the_default_primary() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base(DEFAULT_PRIMARY_KB_ID, "Soul", None)?;
        svc.create_base("project", "Project", None)?;

        // Never configured: the default applies.
        assert_eq!(
            svc.primary_for_session(None)?.as_deref(),
            Some(DEFAULT_PRIMARY_KB_ID)
        );

        // The user hides Soul machine-wide, expressing nothing about the primary.
        svc.set_selection(
            None,
            Some(&[DEFAULT_PRIMARY_KB_ID.to_string()]),
            PrimaryUpdate::Unchanged,
        )?;

        let primary = svc.primary_for_session(None)?;
        assert_ne!(
            primary.as_deref(),
            Some(DEFAULT_PRIMARY_KB_ID),
            "a hidden base must never be the primary — the product default cannot \
             override the user's decision to hide it"
        );
        // It falls to the remaining visible base rather than to nothing, which is
        // the same rule every other primary repair follows.
        assert_eq!(primary.as_deref(), Some("project"));
        Ok(())
    }

    #[test]
    fn soul_is_the_default_until_the_user_explicitly_changes_or_clears_it() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base(DEFAULT_PRIMARY_KB_ID, "Soul", None)?;
        svc.create_base("project", "Project", None)?;

        let primary_path = crate::knowledge::paths::primary_kb_path(svc.root());
        assert!(!primary_path.exists());
        assert_eq!(
            svc.primary_for_session(None)?.as_deref(),
            Some(DEFAULT_PRIMARY_KB_ID)
        );
        assert_eq!(
            svc.primary_for_session(Some("fresh-chat"))?.as_deref(),
            Some(DEFAULT_PRIMARY_KB_ID)
        );

        svc.set_selection(None, None, PrimaryUpdate::Clear)?;
        assert!(
            primary_path.exists(),
            "Clear must persist as a blank override"
        );
        assert_eq!(std::fs::read_to_string(&primary_path)?, "");
        assert_eq!(svc.primary_for_session(None)?, None);
        assert_eq!(svc.primary_for_session(Some("fresh-chat"))?, None);

        svc.set_selection(None, None, PrimaryUpdate::Set("project"))?;
        assert_eq!(svc.primary_for_session(None)?.as_deref(), Some("project"));

        svc.set_selection(None, None, PrimaryUpdate::Inherit)?;
        assert!(!primary_path.exists());
        assert_eq!(
            svc.primary_for_session(None)?.as_deref(),
            Some(DEFAULT_PRIMARY_KB_ID)
        );
        Ok(())
    }

    /// ⚠ **A rename must not declassify the base**, and the axis that broke is
    /// not the one that looks fragile.
    ///
    /// `update_base` moves the directory, the registry, the primary pointer and
    /// the hidden-KB refs, all keyed by kb id — and for a long time it did not
    /// move the classification, which is keyed the same way. The TIER survived
    /// by accident (`tier::is_private` reads an unknown id whose directory
    /// exists as private), so a test that checked privacy alone passed. The
    /// AFFILIATION did not: `tier::affiliation` answers `Owners(∅)` for an
    /// unknown id, an empty owner set is *unclaimed* rather than *nobody's*, and
    /// `affiliation::reachable` admits an unclaimed base from EVERY private
    /// model. Renaming a base holding UCSF data made it readable by another
    /// institution's private model, with nothing on screen marking the change.
    ///
    /// This is the end-to-end half; `tier::tests::renaming_a_base_carries_its_tier_and_its_owners_with_it`
    /// pins the primitive. Both are needed: the primitive can be correct while
    /// nothing calls it.
    #[test]
    fn renaming_a_base_carries_its_classification() -> anyhow::Result<()> {
        use crate::knowledge::affiliation::KbAffiliation;

        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base("omop-cohort", "OMOP cohort", None)?;

        // ⚠ Set the state up through the PAIRED api the production ratchet
        // uses, not through `tier::raise_unlocked` directly. Two reasons, and
        // the second one caught this: a direct raise is not how any real base
        // reaches this state, and `the_tier_ratchet_has_no_production_call_site_that_skips_the_affiliation`
        // greps this file line-wise and cannot tell a test from production, so
        // a bare raise here reads to it as a third production site that forgets
        // the affiliation axis.
        let root = tmp.path();
        svc.raise_tier_and_affiliation(
            "omop-cohort",
            true,
            &crate::knowledge::affiliation::CallerAffiliation::Institution("ucsf".to_string()),
        )?;

        let renamed = svc.update_base("omop-cohort", Some("OMOP cohort 2024"), None)?;
        assert_eq!(renamed.id, "omop-cohort-2024");

        assert!(
            crate::knowledge::tier::is_private(root, &renamed.id),
            "the renamed base must still be private"
        );
        match crate::knowledge::tier::affiliation(root, &renamed.id) {
            KbAffiliation::Owners(owners) => assert!(
                owners.contains("ucsf"),
                "the renamed base must still be claimed by UCSF; an empty owner set is \
                 UNCLAIMED, which every private model may reach. Got {owners:?}"
            ),
            KbAffiliation::Unknown => panic!("the renamed base's owners were unreadable"),
        }
        Ok(())
    }

    #[test]
    fn hidden_kbs_track_rename_and_delete() -> anyhow::Result<()> {
        let tmp = tempfile::TempDir::new()?;
        let svc = KnowledgeService::new(tmp.path().to_path_buf());

        svc.create_base("kb-a", "KB A", None)?;
        svc.set_hidden_persisted(&["kb-a".to_string()])?;
        svc.set_hidden_for_session("session-a", &["kb-a".to_string()])?;

        let renamed = svc.update_base("kb-a", Some("Renamed KB"), None)?;
        assert_eq!(renamed.id, "renamed-kb");
        assert_eq!(svc.get_hidden_persisted()?, vec!["renamed-kb".to_string()]);
        assert_eq!(
            svc.get_hidden_for_session("session-a")?,
            vec!["renamed-kb".to_string()]
        );

        svc.delete_base("renamed-kb")?;
        assert!(svc.get_hidden_persisted()?.is_empty());
        assert!(svc.get_hidden_for_session("session-a")?.is_empty());

        Ok(())
    }

    // -----------------------------------------------------------------------
    // list_bases: a base that cannot be read must not vanish (DR-12)
    // -----------------------------------------------------------------------

    /// Collects formatted tracing output so a test can assert the *level* a
    /// message was emitted at, not merely that something was written. The
    /// subscriber is installed per-thread by `with_default`, so this is safe
    /// under `cargo test`'s parallel threads.
    #[derive(Clone, Default)]
    struct CapturedLogs(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
        type Writer = CapturedLogs;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture<T>(f: impl FnOnce() -> T) -> (T, String) {
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .finish();
        let value = tracing::subscriber::with_default(subscriber, f);
        let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).to_string();
        (value, text)
    }

    /// DR-12's (b). A base whose `manifest.yaml` will not parse used to be
    /// dropped by `if let Ok(m) = …`, and a base that vanishes with no
    /// explanation is worse than an error: the id then leaves the installed
    /// universe, the stored primary reads as pointing at something uninstalled,
    /// and the next selection edit persists the cleared pointer. The user's
    /// `.active-kb` is destroyed for a base still sitting intact on disk.
    ///
    /// It must still not be fatal — one broken base cannot take the listing
    /// down — so what is asserted is the pair: the healthy base is still
    /// listed, and the broken one is named at WARN with its path, because the
    /// path is the only thing that tells a user with a dozen bases which
    /// directory to go and look at.
    #[test]
    fn an_unreadable_manifest_is_named_at_warn_rather_than_silently_dropped() {
        let (dir, svc) = svc();
        svc.create_base("healthy", "Healthy", None).unwrap();
        svc.create_base("broken", "Broken", None).unwrap();
        let broken_manifest = dir.path().join("broken").join("manifest.yaml");
        std::fs::write(&broken_manifest, "id: [this is not a manifest\n").unwrap();

        let (bases, logs) = capture(|| svc.list_bases().expect("one bad base is not fatal"));

        let ids: Vec<&str> = bases.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["healthy"], "the healthy base must still list");
        assert!(
            logs.contains("WARN"),
            "the drop must be reported at WARN, got: {logs}"
        );
        assert!(
            logs.contains("broken"),
            "the report must name the base whose manifest failed, got: {logs}"
        );
    }

    // -----------------------------------------------------------------------
    // migrate_schema_if_needed tests
    // -----------------------------------------------------------------------

    /// A generation-1 `schema.md`: the minimal pre-Plan-5-Task-2 file, with no
    /// "Cross-reference rules" section.
    const LEGACY_SCHEMA: &str =
        "# Knowledge Base: Maintenance Schema\n\n## Layout\n\n- wiki/sources/...\n";

    /// Put a freshly created base back to schema generation 1 — **both** halves.
    ///
    /// The stamp is not decoration here: since the migration stopped
    /// fingerprinting on a substring and started reading
    /// `Manifest.schema_version`, a base with legacy bytes and a current stamp
    /// is not a legacy base, it is a base someone hand-edited. Seeding only the
    /// bytes would test a state the ladder deliberately does not act on.
    fn seed_generation_1(svc: &KnowledgeService, kb_id: &str) {
        let kb = svc.root().join(kb_id);
        std::fs::write(kb.join("schema.md"), LEGACY_SCHEMA).unwrap();
        let mut m = manifest::load(&kb).unwrap();
        m.schema_version = 1;
        manifest::save(&kb, &m).unwrap();
        GitRepo::open(&kb)
            .unwrap()
            .commit_all(ChangeKind::Manual, "seed legacy schema", None)
            .unwrap();
    }

    /// The generation-2 `schema.md`: the file every base on disk carries. Kept
    /// here rather than in production, because no code path writes it any more
    /// — a new base is scaffolded at generation 3 — and a `const` reachable only
    /// from tests is dead code in a real build.
    const GENERATION_2_SCHEMA: &str = include_str!("schema_default.md");

    /// Put a freshly created base into the state every base on disk is in:
    /// generation-2 *content*, stamped 1 because `create_base` used to hardcode
    /// the number. It is no longer enough to seed only the stamp — a base
    /// created today carries the OKF schema, which the 1 -> 2 step would
    /// genuinely rewrite.
    fn seed_generation_2_content_stamped_1(svc: &KnowledgeService, kb_id: &str) {
        let kb = svc.root().join(kb_id);
        std::fs::write(kb.join("schema.md"), GENERATION_2_SCHEMA).unwrap();
        let mut m = manifest::load(&kb).unwrap();
        m.schema_version = 1;
        manifest::save(&kb, &m).unwrap();
        GitRepo::open(&kb)
            .unwrap()
            .commit_all(ChangeKind::Manual, "seed generation-2 schema", None)
            .unwrap();
    }

    /// The claim `SCHEMA_CROSSREF_RULES`'s own comment makes ("kept in sync with
    /// the equivalent block in `schema_default.md`"), turned into a check. The
    /// two texts are not byte-identical and never were, so what is asserted is
    /// the property the ladder actually depends on: the generation-2 file is
    /// already at generation 2, so the 1 -> 2 step is a no-op over it.
    #[test]
    fn the_generation_2_schema_is_already_at_generation_2() {
        assert_eq!(
            with_crossref_rules(GENERATION_2_SCHEMA.to_string()),
            GENERATION_2_SCHEMA,
            "the ladder would append a second copy of the rules block to every \
             base on disk"
        );
    }

    #[test]
    fn migrate_schema_appends_cross_reference_rules_when_missing() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let kb = svc.root().join("k");

        // Put the base back to generation 1 and commit, so we have a clean
        // baseline to migrate from.
        seed_generation_1(&svc, "k");
        let repo = GitRepo::open(&kb).unwrap();
        let before = repo.log(10).unwrap().len();

        let migrated = svc.migrate_schema_if_needed("k").unwrap();
        assert!(migrated, "first call should migrate");

        let new_schema = std::fs::read_to_string(kb.join("schema.md")).unwrap();
        assert!(new_schema.contains("Cross-reference rules"));
        // Original content is preserved.
        assert!(new_schema.contains("wiki/sources/..."));

        let after = repo.log(10).unwrap().len();
        assert_eq!(after, before + 1, "exactly one migration commit added");
        assert!(repo.log(1).unwrap()[0].summary.contains("migrate schema"));
    }

    /// The state EVERY base on disk is in right now, and the one the old
    /// substring fingerprint handled by accident: stamped generation 1 (because
    /// `create_base` hardcoded the number) while already carrying generation-2
    /// content (because it wrote the current `schema_default.md`).
    ///
    /// The version says migrate; the content says there is nothing to do. Both
    /// must be honoured — the stamp moves forward so the base stops re-entering
    /// the ladder, and the rules block must NOT be appended a second time.
    #[test]
    fn a_base_stamped_behind_but_already_current_is_stamped_forward_without_a_rewrite() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let kb = svc.root().join("k");
        seed_generation_2_content_stamped_1(&svc, "k");
        let original = std::fs::read_to_string(kb.join("schema.md")).unwrap();
        let repo = GitRepo::open(&kb).unwrap();
        let before = repo.log(10).unwrap().len();

        assert!(
            !svc.migrate_schema_if_needed("k").unwrap(),
            "nothing was rewritten, so nothing is reported as rewritten"
        );

        let after_schema = std::fs::read_to_string(kb.join("schema.md")).unwrap();
        assert_eq!(after_schema, original, "the rules block was appended twice");
        assert_eq!(
            after_schema.matches("Cross-reference rules").count(),
            original.matches("Cross-reference rules").count()
        );
        assert_eq!(
            manifest::load(&kb).unwrap().schema_version,
            AUTOMATIC_SCHEMA_CEILING,
            "the stamp must move even when the content did not, or the base \
             re-enters the ladder on every macro call forever"
        );
        assert_eq!(
            manifest::load(&kb).unwrap().profile(),
            None,
            "…and it must stop at the ceiling: stamping this base 3 would \
             declare a `[[wiki]]`-linked, `kind:`-typed base OKF"
        );
        assert_eq!(
            repo.log(10).unwrap().len(),
            before,
            "a stamp-only migration must not put an entry in the user's change log"
        );
    }

    /// The bug this whole change is about: with the old substring fingerprint,
    /// a base carrying the *previous* migration's text reported "already
    /// migrated" and no later schema could ever be installed over it. Keyed off
    /// the version, a base behind the current generation migrates however
    /// familiar its contents look.
    #[test]
    fn a_base_behind_the_current_generation_migrates_even_carrying_the_old_marker() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        let kb = svc.root().join("k");
        // Generation-1 stamp, and content that already contains the string the
        // old fingerprint looked for — plus a marker of the user's own, to
        // prove customisations survive.
        std::fs::write(
            kb.join("schema.md"),
            "# Mine\n\n### Cross-reference rules\n\nold text\n",
        )
        .unwrap();
        let mut m = manifest::load(&kb).unwrap();
        m.schema_version = 0; // behind every step in the ladder
        manifest::save(&kb, &m).unwrap();

        // Generation 0 → 2 runs the 1 → 2 step, whose own content guard sees
        // the marker and leaves the file alone; the stamp still moves.
        assert!(!svc.migrate_schema_if_needed("k").unwrap());
        assert_eq!(
            manifest::load(&kb).unwrap().schema_version,
            AUTOMATIC_SCHEMA_CEILING
        );
        assert!(std::fs::read_to_string(kb.join("schema.md"))
            .unwrap()
            .contains("old text"));
    }

    #[test]
    fn a_new_base_is_stamped_with_the_generation_it_was_written_at() {
        // Otherwise the first macro call on a brand-new base runs the ladder
        // over a base that is already current — harmless only for as long as
        // every step happens to be idempotent.
        let (_dir, svc) = svc();
        let m = svc.create_base("k", "K", None).unwrap();
        assert_eq!(m.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            manifest::load(&svc.root().join("k"))
                .unwrap()
                .schema_version,
            CURRENT_SCHEMA_VERSION,
            "the stamp on disk must match the one handed back"
        );
    }

    /// A base created today is at generation 3, which is *ahead* of the ladder,
    /// not merely level with it. The ladder must leave it alone — and in
    /// particular must not append the `[[wiki]]`-link rules block to an OKF
    /// schema, which is what a ceiling of `CURRENT_SCHEMA_VERSION` would do to
    /// the first base someone hand-edited the stamp on.
    #[test]
    fn migrate_schema_leaves_a_base_that_is_ahead_of_the_ladder_alone() {
        let (_dir, svc) = svc();
        let m = svc.create_base("k", "K", None).unwrap();
        let kb = svc.root().join("k");
        assert_eq!(m.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(m.schema_version > AUTOMATIC_SCHEMA_CEILING);

        let original = std::fs::read_to_string(kb.join("schema.md")).unwrap();
        assert!(
            !original.contains("Cross-reference rules"),
            "an OKF schema must not carry the legacy wiki-link rules"
        );
        let repo = GitRepo::open(&kb).unwrap();
        let before = repo.log(10).unwrap().len();

        let migrated = svc.migrate_schema_if_needed("k").unwrap();
        assert!(!migrated, "a base ahead of the ladder is a no-op");

        let after_schema = std::fs::read_to_string(kb.join("schema.md")).unwrap();
        assert_eq!(after_schema, original, "schema bytes unchanged");
        assert_eq!(repo.log(10).unwrap().len(), before, "no new commit");
        assert_eq!(
            manifest::load(&kb).unwrap().schema_version,
            CURRENT_SCHEMA_VERSION,
            "and the stamp was not walked BACK to the ceiling"
        );
    }

    #[test]
    fn migrate_schema_is_idempotent_across_calls() {
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        seed_generation_1(&svc, "k");

        // First call migrates.
        assert!(svc.migrate_schema_if_needed("k").unwrap());
        // Second call is a no-op.
        assert!(!svc.migrate_schema_if_needed("k").unwrap());
        // Third call too.
        assert!(!svc.migrate_schema_if_needed("k").unwrap());
    }

    #[tokio::test]
    async fn registering_a_tier_from_inside_the_root_lock_does_not_deadlock() {
        // The whole of decision (5b), as a test that TIMES OUT rather than fails
        // if the `_unlocked` convention is broken — a deadlock does not assert,
        // it waits. `create_base` holds `lock_root()`; a `tier::raise` that
        // acquires it again blocks forever and the daemon stops answering on the
        // very first knowledge call, while every tier.rs unit test still passes
        // because they call the store on a bare root no service is holding.
        let d = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(d.path().to_path_buf());
        let done = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                svc.create_base("k", "K", None)?; // registers, inside the lock
                                                  // the wrapper: takes the lock itself, once, for both axes
                svc.raise_tier_and_affiliation(
                    "k",
                    true,
                    &crate::knowledge::affiliation::CallerAffiliation::Institution(
                        "ucsf".to_string(),
                    ),
                )?;
                svc.delete_base("k") // forgets, inside the lock
            }),
        )
        .await
        .expect("create_base / raise_tier / delete_base deadlocked on the root lock");
        done.unwrap().unwrap();
    }

    #[test]
    fn a_base_created_by_any_surface_is_registered_public_rather_than_unknown() {
        // Decision (5a). Without the registration, decision (3) reads a freshly
        // created base as PRIVATE and Task 10C locks the user out of a base they
        // just made from the CLI or the Knowledge view.
        let (_dir, svc) = svc();
        svc.create_base("k", "K", None).unwrap();
        assert!(!crate::knowledge::tier::is_private(svc.root(), "k"));
    }
}
