use crate::knowledge::types::{ChangeKind, HistoryEntry};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::path::Path;

/// One spelling of the lock's **historical** in-tree path, shared with
/// `brkb::walk` (which keeps it out of an archive for the same reason this
/// keeps it out of a commit). The live lock now sits beside the bases
/// (`paths::kb_write_lock_path`), so this only ever matches the file a base
/// created before that move still carries — which still must not be committed.
use crate::knowledge::paths::KB_WRITE_LOCK_REL as WRITE_LOCK_PATH;

/// The one directory that holds curated knowledge. `raw/`, `log.md`, `index.md`
/// and `schema.md` are all bookkeeping around it.
const KNOWLEDGE_DIR: &str = "knowledge";

/// The oid of a commit's `knowledge/` subtree, or `None` when the commit has no
/// such directory (git does not track empty ones, so a brand-new KB has none).
fn knowledge_tree_id(commit: &git2::Commit) -> Result<Option<git2::Oid>> {
    let tree = commit.tree()?;
    Ok(tree.get_name(KNOWLEDGE_DIR).map(|entry| entry.id()))
}

fn stage_all(index: &mut git2::Index) -> Result<()> {
    let write_lock = Path::new(WRITE_LOCK_PATH);
    if index.get_path(write_lock, 0).is_some() {
        index.remove_path(write_lock)?;
    }

    let mut skip_write_lock = |path: &Path, _matched_pathspec: &[u8]| {
        if path == write_lock {
            1
        } else {
            0
        }
    };
    index.add_all(
        ["*"].iter(),
        git2::IndexAddOption::DEFAULT,
        Some(&mut skip_write_lock),
    )?;
    Ok(())
}

pub struct GitRepo {
    inner: git2::Repository,
}

impl GitRepo {
    pub fn init(path: &Path) -> Result<Self> {
        let mut opts = git2::RepositoryInitOptions::new();
        opts.initial_head("main");
        let inner = git2::Repository::init_opts(path, &opts)
            .with_context(|| format!("git init {}", path.display()))?;
        let mut cfg = inner.config()?;
        cfg.set_str("user.name", "Biorouter Knowledge")?;
        cfg.set_str("user.email", "knowledge@biorouter.local")?;
        cfg.set_str("commit.gpgsign", "false")?;
        Ok(Self { inner })
    }

    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            inner: git2::Repository::open(path)?,
        })
    }

    pub fn head_file_matches(&self, path: &Path, content: &[u8]) -> Result<bool> {
        let tree = self.inner.head()?.peel_to_commit()?.tree()?;
        let Ok(entry) = tree.get_path(path) else {
            return Ok(false);
        };
        let Ok(blob) = self.inner.find_blob(entry.id()) else {
            return Ok(false);
        };
        Ok(blob.content() == content)
    }

    pub fn commit_all(
        &self,
        kind: ChangeKind,
        summary: &str,
        delta: Option<&str>,
    ) -> Result<String> {
        let mut index = self.inner.index()?;
        stage_all(&mut index)?;
        index.write()?;
        let tree_oid = index.write_tree()?;
        let tree = self.inner.find_tree(tree_oid)?;
        let sig = self.inner.signature()?;
        let parent = self.inner.head().ok().and_then(|h| h.peel_to_commit().ok());
        let parents: Vec<&git2::Commit> = parent.as_ref().map(|c| vec![c]).unwrap_or_default();
        let msg = render_message(kind, summary, delta);
        let oid = self
            .inner
            .commit(Some("HEAD"), &sig, &sig, &msg, &tree, &parents)?;
        Ok(oid.to_string())
    }

    pub fn log(&self, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut walk = self.inner.revwalk()?;
        walk.push_head()?;
        // ⚠ TOPOLOGICAL, not `TIME` alone. Commit times have one-second
        // resolution and a digest makes several commits inside one second —
        // `add_raw_source`, the squash commit, a lint autofix — and libgit2's
        // time sort leaves equal timestamps in no useful order: measured, it
        // listed HEAD and then the rest of the tie OLDEST first, so the Change
        // log put a base's `create` above the ingests made after it (QA
        // 2026-09-10 F13 asked for the log to match `git log`). Topological
        // order never lists a parent before its child; `TIME` only breaks ties
        // between branches, which a squash-committed history never has.
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
        let mut out = Vec::new();
        for oid in walk.flatten().take(limit) {
            let commit = self.inner.find_commit(oid)?;
            let parsed = parse_message(commit.message().unwrap_or(""));
            out.push(HistoryEntry {
                commit_sha: oid.to_string(),
                kind: parsed.kind,
                summary: parsed.summary,
                delta: parsed.delta,
                timestamp: DateTime::<Utc>::from_timestamp(commit.time().seconds(), 0)
                    .unwrap_or_else(Utc::now),
            });
        }
        Ok(out)
    }
}

fn render_message(kind: ChangeKind, summary: &str, delta: Option<&str>) -> String {
    let kind_str = match kind {
        ChangeKind::Ingest => "ingest",
        ChangeKind::Link => "link",
        ChangeKind::Flag => "flag",
        ChangeKind::Query => "query",
        ChangeKind::Lint => "lint",
        ChangeKind::Restore => "restore",
        ChangeKind::Manual => "manual",
    };
    match delta {
        Some(d) => format!("[{kind_str}] {summary}\n\ndelta: {d}\n"),
        None => format!("[{kind_str}] {summary}\n"),
    }
}

struct Parsed {
    kind: ChangeKind,
    summary: String,
    delta: Option<String>,
}

fn parse_message(msg: &str) -> Parsed {
    let mut lines = msg.lines();
    let header = lines.next().unwrap_or("");
    let (kind, summary) = parse_header(header);
    let delta = msg
        .lines()
        .find_map(|l| l.strip_prefix("delta: ").map(str::to_string));
    Parsed {
        kind,
        summary,
        delta,
    }
}

fn parse_header(header: &str) -> (ChangeKind, String) {
    let kind = if let Some(rest) = header.strip_prefix('[') {
        if let Some((k, _)) = rest.split_once(']') {
            match k {
                "ingest" => ChangeKind::Ingest,
                "link" => ChangeKind::Link,
                "flag" => ChangeKind::Flag,
                "query" => ChangeKind::Query,
                "lint" => ChangeKind::Lint,
                "restore" => ChangeKind::Restore,
                _ => ChangeKind::Manual,
            }
        } else {
            ChangeKind::Manual
        }
    } else {
        ChangeKind::Manual
    };
    let summary = header
        .split_once(']')
        .map(|(_, s)| s.trim_start().to_string())
        .unwrap_or_else(|| header.to_string());
    (kind, summary)
}

pub struct Txn {
    pub(crate) branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnowledgeWriteFailurePhase {
    RolledBack,
    OutcomeUncertain,
    Committed,
}

impl KnowledgeWriteFailurePhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RolledBack => "rolled_back",
            Self::OutcomeUncertain => "outcome_uncertain",
            Self::Committed => "committed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitTxnCheckpoint {
    MainRefAdvanced,
    WorkingTreeCheckedOut,
    TransactionBranchDeleted,
}

/// A mutation failed after a transaction had started, with enough durable
/// state for callers to decide whether retrying is safe.
#[derive(Debug)]
pub struct KnowledgeWriteFailure {
    pub phase: KnowledgeWriteFailurePhase,
    pub commit_sha: Option<String>,
    operation: String,
    cause: String,
}

impl KnowledgeWriteFailure {
    pub fn rolled_back(operation: impl Into<String>, cause: anyhow::Error) -> Self {
        Self {
            phase: KnowledgeWriteFailurePhase::RolledBack,
            commit_sha: None,
            operation: operation.into(),
            cause: format!("{cause:#}"),
        }
    }

    pub fn outcome_uncertain(operation: impl Into<String>, cause: anyhow::Error) -> Self {
        Self {
            phase: KnowledgeWriteFailurePhase::OutcomeUncertain,
            commit_sha: None,
            operation: operation.into(),
            cause: format!("{cause:#}"),
        }
    }

    pub fn committed(
        operation: impl Into<String>,
        commit_sha: impl Into<String>,
        cause: anyhow::Error,
    ) -> Self {
        Self {
            phase: KnowledgeWriteFailurePhase::Committed,
            commit_sha: Some(commit_sha.into()),
            operation: operation.into(),
            cause: format!("{cause:#}"),
        }
    }
}

impl std::fmt::Display for KnowledgeWriteFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.phase {
            KnowledgeWriteFailurePhase::RolledBack => write!(
                f,
                "{} failed and rolled back: {}. It is safe to retry",
                self.operation, self.cause
            ),
            KnowledgeWriteFailurePhase::OutcomeUncertain => write!(
                f,
                "{} failed, and its commit or rollback could not be verified: {}. Inspect knowledge history before retrying",
                self.operation, self.cause
            ),
            KnowledgeWriteFailurePhase::Committed => write!(
                f,
                "{} committed in commit {}, but post-commit cleanup failed: {}. The durable write must not be retried",
                self.operation,
                self.commit_sha.as_deref().unwrap_or("unknown"),
                self.cause
            ),
        }
    }
}

impl std::error::Error for KnowledgeWriteFailure {}

impl GitRepo {
    pub fn begin_txn(&self, label: &str) -> Result<Txn> {
        if !matches!(self.inner.head()?.shorthand(), Some("main" | "master")) {
            anyhow::bail!("knowledge repository is not ready to begin a transaction");
        }
        let id = uuid::Uuid::new_v4();
        let branch = format!("txn/{label}-{id}", label = slugify(label));
        let head = self.inner.head()?.peel_to_commit()?;
        self.inner.branch(&branch, &head, false)?;
        self.inner.set_head(&format!("refs/heads/{branch}"))?;
        Ok(Txn { branch })
    }

    pub fn commit_on_txn(&self, txn: &Txn, message: &str) -> Result<String> {
        self.require_txn_head(&txn.branch)?;
        // Same as commit_all but caller already on the txn branch.
        let mut index = self.inner.index()?;
        stage_all(&mut index)?;
        index.write()?;
        let tree_oid = index.write_tree()?;
        let tree = self.inner.find_tree(tree_oid)?;
        let sig = self.inner.signature()?;
        let parent = self.inner.head()?.peel_to_commit()?;
        let oid = self
            .inner
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])?;
        Ok(oid.to_string())
    }

    /// Did this transaction write any *knowledge*?
    ///
    /// `commit_txn` squash-commits the txn branch's *tree* onto main, and git is
    /// happy to record a commit whose tree is byte-identical to its parent's. So
    /// a sub-agent that wrote nothing still produced a commit sha, and every
    /// caller downstream read that sha as proof the work happened (issue #71).
    ///
    /// Comparing the *whole* tree does not answer the question, though: three of
    /// the sub-agent's tools write outside `knowledge/` — `kb_append_log`
    /// appends to `log.md`, `kb_add_raw_source` materialises under `raw/`, and
    /// `kb_write_page` accepts the top-level `index.md` — and any one of them
    /// moves the tree on its own. Since `INGEST_PROCEDURE` asks for all of them,
    /// a provider that died after the log line would leave a run that announced
    /// a digest it never performed, and a whole-tree check would wave it
    /// through. So the comparison is scoped to the `knowledge/` subtree, whose
    /// oid summarises every page beneath it: differing oids mean at least one
    /// knowledge page was added, changed or removed, and nothing else does.
    pub fn txn_wrote_knowledge_pages(&self, txn: &Txn) -> Result<bool> {
        let main = self
            .inner
            .find_branch("main", git2::BranchType::Local)
            .or_else(|_| self.inner.find_branch("master", git2::BranchType::Local))?;
        let main_knowledge =
            knowledge_tree_id(&main.get().peel_to_commit()?)?.map(|oid| oid.to_string());
        Ok(main_knowledge != self.txn_knowledge_tree_id(txn)?)
    }

    /// The `knowledge/` subtree oid at the tip of a transaction branch, as an
    /// opaque marker a caller can hold and compare later.
    ///
    /// [`Self::txn_wrote_knowledge_pages`] answers "did anything change since
    /// **main**", which is the right question only while main is the last thing
    /// that wrote knowledge. It stops being the right question the moment a
    /// caller seeds the transaction itself — BioOKF ingest materialises the
    /// source node before the sub-agent starts (DR-24) — because the seed alone
    /// then satisfies the check and issue #71's guarantee, that a commit sha
    /// means work happened, quietly stops holding. A caller that seeds takes
    /// this marker afterwards and compares against it instead.
    ///
    /// `Option<String>` and not `String`: a bundle with no `knowledge/`
    /// directory yet is a real state (an empty base), and "absent" has to
    /// compare unequal to "present and empty" rather than being papered over
    /// with a sentinel.
    pub fn txn_knowledge_tree_id(&self, txn: &Txn) -> Result<Option<String>> {
        Ok(knowledge_tree_id(
            &self
                .inner
                .find_branch(&txn.branch, git2::BranchType::Local)?
                .get()
                .peel_to_commit()?,
        )?
        .map(|oid| oid.to_string()))
    }

    pub fn commit_txn(
        &self,
        txn: &Txn,
        kind: ChangeKind,
        summary: &str,
        delta: Option<&str>,
    ) -> Result<String> {
        self.commit_txn_with_checkpoint(txn, kind, summary, delta, |_| Ok(()))
    }

    pub(crate) fn commit_txn_with_checkpoint<F>(
        &self,
        txn: &Txn,
        kind: ChangeKind,
        summary: &str,
        delta: Option<&str>,
        mut checkpoint: F,
    ) -> Result<String>
    where
        F: FnMut(CommitTxnCheckpoint) -> Result<()>,
    {
        self.require_txn_head(&txn.branch)?;
        // Squash-merge txn branch onto main as one commit.
        let main = self
            .inner
            .find_branch("main", git2::BranchType::Local)
            .or_else(|_| self.inner.find_branch("master", git2::BranchType::Local))?;
        let main_name = main.name()?.unwrap_or("main").to_string();
        let txn_commit = self
            .inner
            .find_branch(&txn.branch, git2::BranchType::Local)?
            .get()
            .peel_to_commit()?;
        let txn_tree = txn_commit.tree()?;
        let main_commit = main.get().peel_to_commit()?;

        let sig = self.inner.signature()?;
        let msg = render_message(kind, summary, delta);
        let new_oid = self.inner.commit(
            Some(&format!("refs/heads/{main_name}")),
            &sig,
            &sig,
            &msg,
            &txn_tree,
            &[&main_commit],
        )?;
        let commit_sha = new_oid.to_string();
        let committed_failure = |step: &str, error: anyhow::Error| -> anyhow::Error {
            KnowledgeWriteFailure::committed(
                "knowledge transaction",
                &commit_sha,
                error.context(step.to_string()),
            )
            .into()
        };

        checkpoint(CommitTxnCheckpoint::MainRefAdvanced)
            .map_err(|error| committed_failure("after advancing the main ref", error))?;

        // Move HEAD back to main and check out the new tree.
        self.inner
            .set_head(&format!("refs/heads/{main_name}"))
            .map_err(|error| committed_failure("attaching HEAD to main", error.into()))?;
        self.inner
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .map_err(|error| committed_failure("checking out the committed tree", error.into()))?;
        checkpoint(CommitTxnCheckpoint::WorkingTreeCheckedOut)
            .map_err(|error| committed_failure("after checking out the committed tree", error))?;
        // ⚠ Immediately, and on this path too — see
        // `repair_graph_cache_after_checkout`. This is the SECOND force checkout
        // in this impl block and it clobbers the working copy of the tracked
        // `graph-cache.json` exactly as the abort's does, with the txn branch's
        // committed copy, which lags the txn's own pages by the same
        // write→commit→rebuild ordering. Every macro caller happens to rebuild
        // the cache immediately after committing, which is why nothing was red —
        // but `kb_commit_txn` (`knowledge/server.rs`) is a model-callable tool
        // that commits and returns, and on that path the user's newest pages
        // vanished from the graph one function above the repair.
        self.repair_graph_cache_after_checkout();
        // Delete txn branch.
        let mut transaction_branch = self
            .inner
            .find_branch(&txn.branch, git2::BranchType::Local)
            .map_err(|error| {
                committed_failure("finding the completed transaction branch", error.into())
            })?;
        transaction_branch.delete().map_err(|error| {
            committed_failure("deleting the completed transaction branch", error.into())
        })?;
        checkpoint(CommitTxnCheckpoint::TransactionBranchDeleted).map_err(|error| {
            committed_failure("after deleting the completed transaction branch", error)
        })?;
        Ok(commit_sha)
    }

    pub fn abort_txn(&self, txn: &Txn) -> Result<()> {
        self.require_txn_head(&txn.branch)?;
        let main = self
            .inner
            .find_branch("main", git2::BranchType::Local)
            .or_else(|_| self.inner.find_branch("master", git2::BranchType::Local))?;
        let main_name = main.name()?.unwrap_or("main").to_string();
        self.inner.set_head(&format!("refs/heads/{main_name}"))?;
        self.inner
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
        // ⚠ Immediately after the checkout, and BEFORE the branch delete, which
        // is fallible. `delete()?` returns on a branch that is already gone — a
        // second `abort_txn` for the same txn, which the error paths this is
        // called from can easily produce — or on a locked ref, and a repair
        // placed after it would then be skipped, leaving in place the exact
        // stale cache the checkout just installed. That is the state this
        // function exists to prevent, reached on the error path of an error
        // path, which is the least likely place anyone looks.
        self.repair_graph_cache_after_checkout();
        self.inner
            .find_branch(&txn.branch, git2::BranchType::Local)?
            .delete()?;
        Ok(())
    }

    /// Recover a transaction left checked out by a process crash. A live
    /// transaction retains the KB file lock, so reaching this method after
    /// acquiring that lock proves there is no live owner; no age heuristic or
    /// timeout is needed.
    pub fn recover_orphaned_txn(&self) -> Result<bool> {
        let head = self.inner.head()?;
        let checked_out = head
            .name()
            .and_then(|name| name.strip_prefix("refs/heads/txn/"))
            .map(|suffix| format!("txn/{suffix}"));
        drop(head);
        let mut recovered = false;
        if let Some(branch) = checked_out {
            self.abort_txn(&Txn { branch })?;
            recovered = true;
        }

        let orphaned = self
            .inner
            .branches(Some(git2::BranchType::Local))?
            .filter_map(|entry| entry.ok())
            .filter_map(|(branch, _)| branch.name().ok().flatten().map(str::to_string))
            .filter(|branch| branch.starts_with("txn/"))
            .collect::<Vec<_>>();
        for branch in orphaned {
            self.inner
                .find_branch(&branch, git2::BranchType::Local)?
                .delete()?;
            recovered = true;
        }
        Ok(recovered)
    }

    fn require_txn_head(&self, branch: &str) -> Result<()> {
        if branch
            .strip_prefix("txn/")
            .is_none_or(|suffix| suffix.is_empty())
        {
            anyhow::bail!("knowledge transaction is not active");
        }
        let expected = format!("refs/heads/{branch}");
        let head = self.inner.head()?;
        if head.name() != Some(expected.as_str()) {
            anyhow::bail!("knowledge transaction is not active");
        }
        self.inner
            .find_branch(branch, git2::BranchType::Local)
            .map_err(|_| anyhow::anyhow!("knowledge transaction is not active"))?;
        Ok(())
    }

    pub fn abort_after_failure(
        &self,
        txn: &Txn,
        operation: &str,
        error: anyhow::Error,
    ) -> anyhow::Error {
        match self.abort_txn(txn) {
            Ok(()) => KnowledgeWriteFailure::rolled_back(operation, error).into(),
            Err(abort_error) => KnowledgeWriteFailure::outcome_uncertain(
                operation,
                anyhow::anyhow!("{error:#}; rollback also failed: {abort_error:#}"),
            )
            .into(),
        }
    }

    /// Re-derive `graph-cache.json` from whatever pages the checkout just left
    /// on disk.
    ///
    /// ## The defect
    ///
    /// The cache is a **tracked** file — the scaffolded `.gitignore` covers
    /// `raw/*/original.*`, `.crossref-cache/` and `write.lock`, and nothing
    /// else — and every rebuild in the subsystem runs *after* the commit that
    /// motivated it (`store::write_page` commits, then the service rebuilds).
    /// So the copy in `HEAD` is permanently one write behind the pages in
    /// `HEAD`. The force checkout above restores tracked files, faithfully, and
    /// that is precisely the problem: it installs a cache describing a base
    /// that is one page smaller than the one now on disk.
    ///
    /// Nothing downstream can recover from that, because a stale cache is a
    /// *valid* one. DR-13 has `read_cache` answer `Ok(None)` for a cache that is
    /// absent, unreadable, malformed or of an unknown version, and `get_graph`
    /// re-derives on that answer — but a well-formed cache from the previous
    /// commit passes every one of those checks and is served. The user's newest
    /// pages disappear from the Knowledge view and from `GET /graph` while
    /// sitting on disk, and the trigger is a *failed* operation, which is
    /// exactly when nobody goes looking for silent data loss.
    ///
    /// ## Why here and not at the twelve call sites
    ///
    /// `abort_txn` is called from ingest, query, lint, merge and `kb_abort_txn`,
    /// always as `let _ = repo.abort_txn(&txn)` on an error path. A repair
    /// bolted onto each of those is a repair the thirteenth caller will not
    /// have, and error paths are the ones least likely to be exercised. The
    /// checkout is what invalidates the cache, so the repair belongs beside the
    /// checkout.
    ///
    /// ⚠ **Which means BOTH checkouts, and beside means beside.** This impl
    /// block force-checks-out twice — `abort_txn` and `commit_txn` — and the
    /// first version of this fix repaired only the abort. The commit path is not
    /// benign: it installs the txn branch's committed `graph-cache.json`, which
    /// lags the txn's own pages by exactly the write→commit→rebuild ordering
    /// described above. Every *macro* caller happens to rebuild right after
    /// committing (`macros/ingest.rs`, `macros/query.rs`, `service.rs`), which is
    /// why no test was red; the model-callable `kb_commit_txn` tool does not, and
    /// on that path the newest pages vanished from the graph one function above
    /// the repair. So macro commits now derive the graph twice — once here and
    /// once in the caller — and that is the deliberate trade: a redundant derive
    /// costs a tree walk, a missing one costs the user their newest pages with no
    /// error anywhere.
    ///
    /// "Beside the checkout" is also literal: the call goes immediately after
    /// `checkout_head`, before the fallible branch delete on either path. A
    /// `delete()?` that returns early — an already-deleted branch, a locked
    /// ref — would otherwise skip the repair and leave the stale cache the
    /// checkout just installed.
    ///
    /// ## Why re-derive rather than preserve, or untrack
    ///
    /// Preserving the working copy across the checkout would be wrong whenever
    /// the transaction rebuilt the cache itself (ingest does): the preserved
    /// copy would then describe pages the abort just rolled back. Deriving from
    /// the pages that survived is the only answer that is right in both cases.
    ///
    /// Untracking the file — the other candidate fix — would stop git clobbering
    /// it, and `.brkb` export walks the working tree rather than the git history
    /// so a bundle would still carry it. But it only helps bases created
    /// *after* the change: `.gitignore` does not untrack what is already
    /// tracked, so every base on disk would keep the bug until something
    /// rewrote its index, and that something would be an automatic write to
    /// every knowledge base on the machine to fix a derived file. Not worth it.
    ///
    /// ## Failures are absorbed, and downgraded to "no cache"
    ///
    /// This returns nothing, and neither the abort nor the commit fails on it.
    /// By the time it runs, the thing the caller actually asked for has already
    /// happened and is durable — the rollback on one path, the squash-merge
    /// commit on the other — so failing the call now would report a *derived
    /// file* as a failed transaction and invite a retry of work that is already
    /// done. But a failed rebuild must not leave the stale file usable, or the
    /// silent-loss bug survives its own fix. The repair therefore removes it;
    /// if even removal fails, that second failure is retained in the warning
    /// and the revision-bound reader still rejects the old envelope.
    fn repair_graph_cache_after_checkout(&self) {
        let Some(kb_root) = self.inner.workdir().map(Path::to_path_buf) else {
            // A bare repo has no pages to derive from. Not reachable for a
            // knowledge base, and not worth an error if it ever is.
            return;
        };
        if let Err(e) = crate::knowledge::graph::rebuild_cache(&kb_root) {
            let stale = crate::knowledge::graph::cache_path(&kb_root);
            tracing::warn!(
                "knowledge: could not rebuild the graph cache at {} after a transaction \
                 checkout; the next read will re-derive it: {e:#}",
                stale.display()
            );
        }
    }
}

fn slugify(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

impl GitRepo {
    pub fn read_file_at(&self, sha: &str, path: &str) -> Result<Option<String>> {
        let oid = git2::Oid::from_str(sha)?;
        let commit = self.inner.find_commit(oid)?;
        let tree = commit.tree()?;
        let entry = match tree.get_path(Path::new(path)) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };
        let obj = entry.to_object(&self.inner)?;
        let blob = obj.as_blob().ok_or_else(|| anyhow::anyhow!("not a blob"))?;
        Ok(Some(String::from_utf8_lossy(blob.content()).to_string()))
    }

    pub fn restore_to(&self, sha: &str, summary: &str) -> Result<String> {
        let oid = git2::Oid::from_str(sha)?;
        let target = self.inner.find_commit(oid)?;
        let target_tree = target.tree()?;
        let head = self.inner.head()?.peel_to_commit()?;
        let sig = self.inner.signature()?;
        let msg = render_message(
            ChangeKind::Restore,
            summary,
            Some(&format!("→ {}", sha.get(..7).unwrap_or(sha))),
        );
        let new_oid = self
            .inner
            .commit(Some("HEAD"), &sig, &sig, &msg, &target_tree, &[&head])?;
        // Check out the new commit so working tree matches.
        self.inner
            .checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
        Ok(new_oid.to_string())
    }
}

impl GitRepo {
    /// Commit on the currently-checked-out branch. Used by store::write_page
    /// when a txn is active and the caller has already switched HEAD.
    pub fn commit_on_txn_in_progress(&self, branch: &str, message: &str) -> Result<String> {
        self.require_txn_head(branch)?;
        let mut index = self.inner.index()?;
        stage_all(&mut index)?;
        index.write()?;
        let tree_oid = index.write_tree()?;
        let tree = self.inner.find_tree(tree_oid)?;
        let sig = self.inner.signature()?;
        let parent = self.inner.head()?.peel_to_commit()?;
        let oid = self
            .inner
            .commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])?;
        Ok(oid.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_repo() {
        let dir = tempfile::tempdir().unwrap();
        GitRepo::init(dir.path()).unwrap();
        assert!(dir.path().join(".git").exists());
    }

    #[test]
    fn commit_and_log_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.md"), "hello").unwrap();
        let sha = repo
            .commit_all(ChangeKind::Ingest, "first source", Some("+1 page"))
            .unwrap();
        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].commit_sha, sha);
        assert_eq!(log[0].kind, ChangeKind::Ingest);
        assert_eq!(log[0].summary, "first source");
        assert_eq!(log[0].delta.as_deref(), Some("+1 page"));
    }

    #[test]
    fn commit_all_never_tracks_the_write_lock() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();
        let lock = dir.path().join(WRITE_LOCK_PATH);
        std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
        std::fs::write(&lock, "transient").unwrap();
        let lock_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock)
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock_file).unwrap();
        std::fs::write(dir.path().join("a.md"), "tracked").unwrap();

        let sha = repo
            .commit_all(ChangeKind::Manual, "skip lock", None)
            .unwrap();

        assert_eq!(repo.read_file_at(&sha, WRITE_LOCK_PATH).unwrap(), None);
        assert_eq!(
            repo.read_file_at(&sha, "a.md").unwrap().as_deref(),
            Some("tracked")
        );
    }

    #[test]
    fn multiple_commits_ordered_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.md"), "1").unwrap();
        repo.commit_all(ChangeKind::Ingest, "one", None).unwrap();
        std::fs::write(dir.path().join("b.md"), "2").unwrap();
        repo.commit_all(ChangeKind::Lint, "two", None).unwrap();
        let log = repo.log(10).unwrap();
        assert_eq!(log[0].summary, "two");
        assert_eq!(log[1].summary, "one");
    }

    /// QA 2026-09-10 F13 asked the history route to match `git log`, and a
    /// digest is exactly where it did not: `add_raw_source`, the squash commit
    /// and a lint autofix land within one second, and `Sort::TIME` alone leaves
    /// commits with EQUAL timestamps in no particular order — measured, it
    /// listed a base's `create` commit above the two ingests made after it.
    /// The timestamps here are pinned equal, so the tie is certain rather than
    /// a matter of how fast this machine commits.
    #[test]
    fn log_keeps_commit_order_when_commits_share_a_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();
        let sig = git2::Signature::new(
            "Biorouter Knowledge",
            "knowledge@biorouter.local",
            &git2::Time::new(1_789_000_000, 0),
        )
        .unwrap();
        let mut expected = Vec::new();
        for (step, kind) in [
            ChangeKind::Manual,
            ChangeKind::Ingest,
            ChangeKind::Ingest,
            ChangeKind::Lint,
            ChangeKind::Restore,
        ]
        .into_iter()
        .enumerate()
        {
            std::fs::write(dir.path().join(format!("{step}.md")), step.to_string()).unwrap();
            let mut index = repo.inner.index().unwrap();
            stage_all(&mut index).unwrap();
            index.write().unwrap();
            let tree = repo.inner.find_tree(index.write_tree().unwrap()).unwrap();
            let parent = repo.inner.head().ok().and_then(|h| h.peel_to_commit().ok());
            let parents: Vec<&git2::Commit> = parent.iter().collect();
            let summary = format!("step {step}");
            let oid = repo
                .inner
                .commit(
                    Some("HEAD"),
                    &sig,
                    &sig,
                    &render_message(kind, &summary, None),
                    &tree,
                    &parents,
                )
                .unwrap();
            expected.push((oid.to_string(), summary));
        }
        expected.reverse();

        let listed: Vec<(String, String)> = repo
            .log(10)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.commit_sha, entry.summary))
            .collect();
        assert_eq!(
            listed, expected,
            "newest first, and never a parent before its child"
        );
    }

    #[test]
    fn txn_lifecycle_squash_merges_into_main() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("seed.md"), "seed").unwrap();
        repo.commit_all(ChangeKind::Manual, "seed", None).unwrap();

        let txn = repo.begin_txn("ingest paper X").unwrap();
        std::fs::write(dir.path().join("p1.md"), "1").unwrap();
        repo.commit_on_txn(&txn, "step 1").unwrap();
        std::fs::write(dir.path().join("p2.md"), "2").unwrap();
        repo.commit_on_txn(&txn, "step 2").unwrap();
        let final_sha = repo
            .commit_txn(&txn, ChangeKind::Ingest, "Paper X", Some("+2 pages"))
            .unwrap();

        let log = repo.log(10).unwrap();
        assert_eq!(log[0].commit_sha, final_sha);
        assert_eq!(log[0].summary, "Paper X");
        assert_eq!(log[0].kind, ChangeKind::Ingest);
        assert_eq!(
            log.len(),
            2,
            "seed + squashed-ingest only: no intermediate commits"
        );
    }

    fn commit_failure_after(checkpoint: CommitTxnCheckpoint) {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("seed.md"), "seed").unwrap();
        repo.commit_all(ChangeKind::Manual, "seed", None).unwrap();

        let txn = repo.begin_txn("phase-aware").unwrap();
        std::fs::write(dir.path().join("durable.md"), "durable").unwrap();
        repo.commit_on_txn(&txn, "transaction work").unwrap();
        let error = repo
            .commit_txn_with_checkpoint(
                &txn,
                ChangeKind::Manual,
                "phase-aware commit",
                None,
                |reached| {
                    if reached == checkpoint {
                        anyhow::bail!("injected failure at {reached:?}");
                    }
                    Ok(())
                },
            )
            .expect_err("the checkpoint must fail after advancing main");
        let failure = error
            .downcast_ref::<KnowledgeWriteFailure>()
            .expect("post-ref failures remain phase-aware");
        assert_eq!(failure.phase, KnowledgeWriteFailurePhase::Committed);
        let committed_sha = failure
            .commit_sha
            .as_deref()
            .expect("a committed failure identifies the durable commit");
        let raw = git2::Repository::open(dir.path()).unwrap();
        assert_eq!(
            raw.find_branch("main", git2::BranchType::Local)
                .unwrap()
                .get()
                .target()
                .unwrap()
                .to_string(),
            committed_sha
        );
        assert_eq!(
            repo.read_file_at(committed_sha, "durable.md")
                .unwrap()
                .as_deref(),
            Some("durable")
        );

        repo.recover_orphaned_txn().unwrap();
        assert_eq!(raw.head().unwrap().shorthand(), Some("main"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("durable.md")).unwrap(),
            "durable"
        );
        assert!(raw
            .find_branch(&txn.branch, git2::BranchType::Local)
            .is_err());
    }

    #[test]
    fn failure_after_main_ref_update_reports_committed_phase() {
        commit_failure_after(CommitTxnCheckpoint::MainRefAdvanced);
    }

    #[test]
    fn failure_after_checkout_reports_committed_phase() {
        commit_failure_after(CommitTxnCheckpoint::WorkingTreeCheckedOut);
    }

    #[test]
    fn failure_after_transaction_branch_deletion_reports_committed_phase() {
        commit_failure_after(CommitTxnCheckpoint::TransactionBranchDeleted);
    }

    #[test]
    fn txn_abort_leaves_main_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("seed.md"), "seed").unwrap();
        repo.commit_all(ChangeKind::Manual, "seed", None).unwrap();
        let pre = repo.log(10).unwrap();

        let txn = repo.begin_txn("doomed").unwrap();
        std::fs::write(dir.path().join("doom.md"), "x").unwrap();
        repo.commit_on_txn(&txn, "bad").unwrap();
        repo.abort_txn(&txn).unwrap();

        let post = repo.log(10).unwrap();
        assert_eq!(pre, post);
        assert!(
            !dir.path().join("doom.md").exists(),
            "working tree restored"
        );
    }

    #[test]
    fn transaction_mutations_require_a_txn_branch_and_exact_head() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("seed.md"), "seed").unwrap();
        repo.commit_all(ChangeKind::Manual, "seed", None).unwrap();
        let txn = repo.begin_txn("guarded").unwrap();

        repo.inner.set_head("refs/heads/main").unwrap();
        assert!(repo.commit_on_txn(&txn, "wrong head").is_err());
        assert!(repo.abort_txn(&txn).is_err());
        assert!(repo
            .commit_on_txn_in_progress("main", "caller branch")
            .is_err());

        repo.inner
            .set_head(&format!("refs/heads/{}", txn.branch))
            .unwrap();
        repo.abort_txn(&txn).unwrap();
    }

    /// Aborting must not silently delete the newest pages **from the graph**.
    ///
    /// The bug this pins is entirely invisible from git's own point of view, and
    /// [`txn_abort_leaves_main_untouched`] above is green throughout it: the
    /// abort restores the tracked files exactly as it promises, and
    /// `graph-cache.json` is one of them. The trouble is what the committed copy
    /// of that file *says*. Every rebuild runs after the commit that motivated
    /// it, so the copy in `HEAD` is permanently one write behind the pages in
    /// `HEAD`, and a force checkout therefore installs a cache that is missing
    /// the last committed page. `read_cache` cannot save the reader — a stale
    /// cache is a perfectly valid one, so it is served rather than re-derived,
    /// and the page vanishes from the Knowledge view while sitting on disk.
    ///
    /// So the assertion is deliberately about the **cache's contents** and not
    /// about the file's existence or its mtime: every wrong implementation
    /// leaves a file there.
    ///
    /// The fixture goes through `KnowledgeService` rather than writing a repo by
    /// hand because the staleness is a property of the *order* production writes
    /// happen in — page, commit, then rebuild — and a hand-built cache would be
    /// whatever the test author decided to put in it.
    #[test]
    fn aborting_a_transaction_leaves_the_graph_cache_describing_the_pages_on_disk() {
        use crate::knowledge::{graph, service::KnowledgeService, store};

        let cached_paths = |kb: &Path| -> Vec<String> {
            let mut paths: Vec<String> = graph::read_cache(kb)
                .expect("reading the cache is not an error")
                .expect("a base that has been rebuilt has a cache")
                .nodes
                .iter()
                .map(|n| n.path.clone())
                .collect();
            paths.sort();
            paths
        };
        let page = |identifier: &str| {
            format!("---\ntype: Concept\nidentifier: {identifier}\n---\n\nbody\n")
        };

        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base("kb", "kb", None).unwrap();
        let kb = svc.root().join("kb");

        // A committed page, with the rebuild after the commit — the order every
        // production write takes, and the whole reason the committed cache lags.
        store::write_page(&kb, "knowledge/concept/a.md", &page("A"), "add A", None).unwrap();
        svc.rebuild_graph_cache("kb").unwrap();
        assert!(
            cached_paths(&kb).contains(&"knowledge/concept/a.md".to_string()),
            "the fixture never got A into the cache"
        );

        // A transaction that writes a page and is then abandoned — a sub-agent
        // that ran out of steps, a provider that died, a merge that failed its
        // canonical check.
        let repo = GitRepo::open(&kb).unwrap();
        let txn = repo.begin_txn("doomed").unwrap();
        store::write_page(
            &kb,
            "knowledge/concept/b.md",
            &page("B"),
            "add B",
            Some(&txn.branch),
        )
        .unwrap();
        svc.rebuild_graph_cache("kb").unwrap();
        repo.abort_txn(&txn).unwrap();

        assert!(
            kb.join("knowledge/concept/a.md").exists(),
            "the abort deleted a committed page from disk"
        );
        assert!(
            !kb.join("knowledge/concept/b.md").exists(),
            "the abort left the transaction's page behind"
        );
        assert_eq!(
            cached_paths(&kb),
            vec!["knowledge/concept/a.md".to_string()],
            "the graph after the abort does not describe the pages on disk"
        );
    }

    /// ⚠ The **other** force checkout in the same impl block, which the first
    /// version of this repair left alone.
    ///
    /// `commit_txn` squash-merges the txn tree onto main and then force-checks-out
    /// HEAD, which restores the txn branch's *committed* `graph-cache.json` — and
    /// that copy lags by exactly the write→commit→rebuild ordering the abort test
    /// above describes, because `store::write_page` commits the page and the
    /// rebuild only follows. So a commit installs a cache describing a base one
    /// page smaller than the one it just created.
    ///
    /// Every macro caller happens to rebuild immediately after committing
    /// (`macros/ingest.rs`, `macros/query.rs`, `service.rs`), which is why the
    /// suite stayed green with the repair on one path only. The
    /// **model-callable** `kb_commit_txn` tool (`knowledge/server.rs`) commits and
    /// returns, and this is that path: no rebuild afterwards, so the assertion
    /// measures what `commit_txn` itself left behind.
    #[test]
    fn committing_a_transaction_leaves_the_graph_cache_describing_the_pages_on_disk() {
        use crate::knowledge::{graph, service::KnowledgeService, store, types::ChangeKind};

        let cached_paths = |kb: &Path| -> Vec<String> {
            let mut paths: Vec<String> = graph::read_cache(kb)
                .expect("reading the cache is not an error")
                .expect("a base that has been rebuilt has a cache")
                .nodes
                .iter()
                .map(|n| n.path.clone())
                .collect();
            paths.sort();
            paths
        };
        let page = |identifier: &str| {
            format!("---\ntype: Concept\nidentifier: {identifier}\n---\n\nbody\n")
        };

        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base("kb", "kb", None).unwrap();
        let kb = svc.root().join("kb");

        store::write_page(&kb, "knowledge/concept/a.md", &page("A"), "add A", None).unwrap();
        svc.rebuild_graph_cache("kb").unwrap();

        // A transaction that writes a page and SUCCEEDS. The rebuild runs before
        // the commit-txn, exactly as it does in production: the page is committed
        // onto the txn branch first and the cache is rebuilt afterwards, so the
        // branch's committed cache knows only about A.
        let repo = GitRepo::open(&kb).unwrap();
        let txn = repo.begin_txn("real-work").unwrap();
        store::write_page(
            &kb,
            "knowledge/concept/b.md",
            &page("B"),
            "add B",
            Some(&txn.branch),
        )
        .unwrap();
        svc.rebuild_graph_cache("kb").unwrap();
        repo.commit_txn(&txn, ChangeKind::Manual, "add B", None)
            .unwrap();

        assert!(
            kb.join("knowledge/concept/b.md").exists(),
            "the commit did not keep the transaction's page"
        );
        assert_eq!(
            cached_paths(&kb),
            vec![
                "knowledge/concept/a.md".to_string(),
                "knowledge/concept/b.md".to_string()
            ],
            "the graph after a COMMIT does not describe the pages on disk — the \
             page the transaction was opened to add is missing from it"
        );
    }

    /// ⚠ The repair has to sit **between** the checkout and the branch delete,
    /// not after both, and this is the case that separates the two placements.
    ///
    /// `abort_txn` is called from a dozen error paths as
    /// `let _ = repo.abort_txn(&txn)`, so aborting the same transaction twice is
    /// ordinary rather than exotic: an inner handler aborts, returns an error,
    /// and an outer one aborts again on the way out. The second call still
    /// force-checks-out HEAD — reinstalling the lagging committed cache over the
    /// one the first call repaired — and *then* dies on `find_branch(...)?`,
    /// because the branch is already gone. A repair placed after that `?` never
    /// runs, and the base is left in exactly the state this function exists to
    /// prevent, on the error path of an error path.
    ///
    /// The second abort is expected to return `Err`; that is not the defect. The
    /// assertion is about what it left on disk.
    #[test]
    fn a_second_abort_that_fails_still_leaves_the_graph_cache_describing_disk() {
        use crate::knowledge::{graph, service::KnowledgeService, store};

        let cached_paths = |kb: &Path| -> Vec<String> {
            let mut paths: Vec<String> = graph::read_cache(kb)
                .expect("reading the cache is not an error")
                .expect("a base that has been rebuilt has a cache")
                .nodes
                .iter()
                .map(|n| n.path.clone())
                .collect();
            paths.sort();
            paths
        };
        let page = |identifier: &str| {
            format!("---\ntype: Concept\nidentifier: {identifier}\n---\n\nbody\n")
        };

        let dir = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(dir.path().to_path_buf());
        svc.create_base("kb", "kb", None).unwrap();
        let kb = svc.root().join("kb");

        store::write_page(&kb, "knowledge/concept/a.md", &page("A"), "add A", None).unwrap();
        svc.rebuild_graph_cache("kb").unwrap();

        let repo = GitRepo::open(&kb).unwrap();
        let txn = repo.begin_txn("doomed").unwrap();
        store::write_page(
            &kb,
            "knowledge/concept/b.md",
            &page("B"),
            "add B",
            Some(&txn.branch),
        )
        .unwrap();
        svc.rebuild_graph_cache("kb").unwrap();

        repo.abort_txn(&txn).unwrap();
        let second = repo.abort_txn(&txn);
        assert!(
            second.is_err(),
            "the fixture is not exercising the early return: the branch was \
             supposed to be gone by now"
        );

        assert_eq!(
            cached_paths(&kb),
            vec!["knowledge/concept/a.md".to_string()],
            "the failed second abort checked out the stale cache and returned \
             before repairing it"
        );
    }

    #[test]
    fn preview_state_returns_file_at_commit() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.md"), "v1").unwrap();
        let sha1 = repo.commit_all(ChangeKind::Manual, "v1", None).unwrap();
        std::fs::write(dir.path().join("a.md"), "v2").unwrap();
        repo.commit_all(ChangeKind::Manual, "v2", None).unwrap();
        let v1 = repo.read_file_at(&sha1, "a.md").unwrap();
        assert_eq!(v1.as_deref(), Some("v1"));
    }

    #[test]
    fn restore_state_creates_new_commit_with_old_tree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = GitRepo::init(dir.path()).unwrap();
        std::fs::write(dir.path().join("a.md"), "v1").unwrap();
        let sha1 = repo.commit_all(ChangeKind::Manual, "v1", None).unwrap();
        std::fs::write(dir.path().join("a.md"), "v2").unwrap();
        repo.commit_all(ChangeKind::Manual, "v2", None).unwrap();
        let new_sha = repo.restore_to(&sha1, "restore to v1").unwrap();
        // Working tree should now contain v1.
        let body = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
        assert_eq!(body, "v1");
        // History still grows forward.
        let log = repo.log(10).unwrap();
        assert_eq!(log[0].commit_sha, new_sha);
        assert_eq!(log[0].kind, ChangeKind::Restore);
        assert_eq!(log.len(), 3);
    }
}
