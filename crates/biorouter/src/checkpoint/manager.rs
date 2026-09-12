//! `CheckpointManager` — the facade the agent turn boundary and (later) the
//! server routes call. Owns the shadow-repo location + config and delegates
//! record persistence to `SessionManager` (the SQLite `checkpoints` table).
//!
//! All git2 work runs inside `spawn_blocking` so nothing non-`Send`
//! (`git2::Repository`) ever crosses an `.await` — the agent reply stream must
//! stay `Send`.

use super::{CheckpointConfig, CheckpointKind, CheckpointRecord, RestoreAxis, RestoreOutcome};
use crate::checkpoint::store::ShadowRepo;
use crate::session::SessionManager;
use anyhow::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

pub struct CheckpointManager {
    /// `<data_dir>` — shadow repos live under `<data_root>/checkpoints/<id>/git`.
    data_root: PathBuf,
    session_manager: Arc<SessionManager>,
    cfg: CheckpointConfig,
}

impl CheckpointManager {
    pub fn new(
        data_root: PathBuf,
        session_manager: Arc<SessionManager>,
        cfg: CheckpointConfig,
    ) -> Self {
        Self {
            data_root,
            session_manager,
            cfg,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.cfg.enabled
    }

    fn git_dir(&self, session_id: &str) -> PathBuf {
        self.data_root
            .join("checkpoints")
            .join(session_id)
            .join("git")
    }

    /// Snapshot the work-tree at a turn boundary. Returns `None` (no row) when
    /// checkpoints are disabled, the tree exceeds the cap, or the tree is
    /// identical to the previous checkpoint (dedup by `tree_sha`).
    pub async fn snapshot(
        &self,
        session_id: &str,
        working_dir: &Path,
        anchor_ts: i64,
        kind: CheckpointKind,
    ) -> Result<Option<CheckpointRecord>> {
        if !self.cfg.enabled {
            return Ok(None);
        }
        let last = self.session_manager.last_checkpoint(session_id).await?;

        let git_dir = self.git_dir(session_id);
        let worktree = working_dir.to_path_buf();
        let caps = self.cfg.caps();
        let max_tree = self.cfg.max_tree_bytes;
        let commit = tokio::task::spawn_blocking(move || -> Result<Option<(String, String)>> {
            let ignore = build_ignore(&worktree);
            let repo = ShadowRepo::open_or_init(&git_dir, &worktree)?;
            repo.snapshot(&ignore, &caps, Some(max_tree))
        })
        .await??;

        let Some((commit_sha, tree_sha)) = commit else {
            return Ok(None);
        };
        // Dedup: an unchanged work-tree produces the same tree oid.
        if last.as_ref().map(|c| c.tree_sha.as_str()) == Some(tree_sha.as_str()) {
            return Ok(None);
        }

        let turn_index = last.as_ref().map(|c| c.turn_index + 1).unwrap_or(0);
        let rec = CheckpointRecord {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            turn_index,
            anchor_ts,
            kind,
            commit_sha,
            tree_sha,
            changed_paths: Vec::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.session_manager.insert_checkpoint(&rec).await?;
        Ok(Some(rec))
    }

    /// List a session's checkpoints, newest-first.
    pub async fn list(&self, session_id: &str) -> Result<Vec<CheckpointRecord>> {
        self.session_manager.list_checkpoints(session_id).await
    }

    /// Restore along one axis. Takes a reversible `pre_restore` baseline first
    /// (so a rewind can itself be undone), then applies the files and/or
    /// conversation axes.
    pub async fn restore(
        &self,
        session_id: &str,
        checkpoint_id: &str,
        axis: RestoreAxis,
        working_dir: &Path,
    ) -> Result<RestoreOutcome> {
        let target = self
            .session_manager
            .get_checkpoint(session_id, checkpoint_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("checkpoint {checkpoint_id} not found"))?;

        // Reversible baseline (redo). Best-effort — a snapshot skip (dedup/cap)
        // must not block the restore itself.
        let pre_restore_checkpoint_id = if axis.touches_files() {
            self.snapshot(
                session_id,
                working_dir,
                target.anchor_ts,
                CheckpointKind::PreRestore,
            )
            .await
            .ok()
            .flatten()
            .map(|r| r.id)
        } else {
            None
        };

        let mut restored_paths = Vec::new();
        if axis.touches_files() {
            let git_dir = self.git_dir(session_id);
            let worktree = working_dir.to_path_buf();
            let caps = self.cfg.caps();
            let commit_sha = target.commit_sha.clone();
            let paths = tokio::task::spawn_blocking(move || -> Result<Vec<PathBuf>> {
                let ignore = build_ignore(&worktree);
                let repo = ShadowRepo::open_or_init(&git_dir, &worktree)?;
                repo.restore_files(&commit_sha, &ignore, &caps)
            })
            .await??;
            restored_paths = paths
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
        }

        let mut truncated_from_ts = None;
        if axis.touches_conversation() {
            // #51: DELIBERATELY the unbounded cut. `truncate_conversation_bounded`
            // exists and this is the site with the widest read→write window (a
            // `pre_restore` snapshot plus, on `Both`, a whole work-tree checkout),
            // so it looks like the obvious caller — it is not. A bounded cut keeps
            // whatever landed after the basis, which leaves a transcript that ends
            // at the anchor and then trails messages about files just rewound out
            // from under them, and reports `Truncated` either way: an INCOMPLETE
            // rewind reported as a complete one. The two honest alternatives
            // (refuse, or rewind and report what was discarded) both change
            // `RestoreOutcome` and the GUI that renders it, so they are a semantics
            // decision, not a swap of one call for another. Do not "fix" this
            // without making that decision — see
            // docs/agent-loop/conversation-writeback-freshness.md,
            // "Checkpoint restore — open, and it is a semantics question".
            self.session_manager
                .truncate_conversation(session_id, target.anchor_ts)
                .await?;
            truncated_from_ts = Some(target.anchor_ts);
        }

        Ok(RestoreOutcome {
            checkpoint_id: target.id,
            axis,
            restored_paths,
            truncated_from_ts,
            pre_restore_checkpoint_id,
        })
    }

    /// Remove a session's shadow repo + checkpoint rows.
    ///
    /// Deleting a chat does not come through here: `SessionStorage::delete_session`
    /// removes the rows in the chat's own transaction and the same
    /// [`super::repository_dir`] after it commits, which is what reaches the CLI
    /// and every other caller that never builds a `CheckpointManager`.
    pub async fn gc(&self, session_id: &str) -> Result<()> {
        if let Some(dir) = super::repository_dir(&self.data_root, session_id) {
            // BR-57: removing a whole shadow git repo is blocking file I/O — keep
            // it off the async runtime like the rest of the checkpoint file work.
            let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&dir)).await;
        }
        self.session_manager.delete_checkpoints(session_id).await
    }
}

/// Build the extra ignore matcher (`.biorouterignore` + `BIOROUTER_CHECKPOINT_IGNORE`
/// globs). `.gitignore` is handled separately by the walk itself.
fn build_ignore(worktree: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(worktree);
    let local = worktree.join(".biorouterignore");
    if local.is_file() {
        let _ = builder.add(&local);
    }
    if let Ok(extra) = std::env::var("BIOROUTER_CHECKPOINT_IGNORE") {
        for glob in extra
            .split([',', ':'])
            .map(str::trim)
            .filter(|g| !g.is_empty())
        {
            let _ = builder.add_line(None, glob);
        }
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint::CheckpointConfig;

    async fn mgr(enabled: bool) -> (CheckpointManager, tempfile::TempDir, tempfile::TempDir) {
        let data = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let sm = Arc::new(SessionManager::new(sessions.path().to_path_buf()));
        let cfg = CheckpointConfig {
            enabled,
            ..Default::default()
        };
        (
            CheckpointManager::new(data.path().to_path_buf(), sm, cfg),
            data,
            sessions,
        )
    }

    #[tokio::test]
    async fn disabled_manager_is_noop() {
        let (m, _d, _s) = mgr(false).await;
        let work = tempfile::tempdir().unwrap();
        std::fs::write(work.path().join("a.txt"), "hi").unwrap();
        let rec = m
            .snapshot("s1", work.path(), 100, CheckpointKind::PreStep)
            .await
            .unwrap();
        assert!(rec.is_none());
        assert!(m.list("s1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn snapshot_dedup_by_tree_sha() {
        let (m, _d, _s) = mgr(true).await;
        let work = tempfile::tempdir().unwrap();
        std::fs::write(work.path().join("a.txt"), "v1").unwrap();

        let first = m
            .snapshot("s1", work.path(), 100, CheckpointKind::PreStep)
            .await
            .unwrap();
        assert!(first.is_some());
        // Same tree → no new row.
        let second = m
            .snapshot("s1", work.path(), 200, CheckpointKind::PostStep)
            .await
            .unwrap();
        assert!(second.is_none(), "identical tree must dedup");

        // Change a file → new row with incremented turn_index.
        std::fs::write(work.path().join("a.txt"), "v2").unwrap();
        let third = m
            .snapshot("s1", work.path(), 300, CheckpointKind::PostStep)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(third.turn_index, 1);
        assert_eq!(m.list("s1").await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn files_only_restore_leaves_conversation() {
        let (m, _d, _s) = mgr(true).await;
        let work = tempfile::tempdir().unwrap();
        std::fs::write(work.path().join("a.txt"), "v1").unwrap();
        let cp = m
            .snapshot("s1", work.path(), 100, CheckpointKind::PreStep)
            .await
            .unwrap()
            .unwrap();

        std::fs::write(work.path().join("a.txt"), "v2").unwrap();
        std::fs::write(work.path().join("new.txt"), "n").unwrap();

        let out = m
            .restore("s1", &cp.id, RestoreAxis::Files, work.path())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(work.path().join("a.txt")).unwrap(),
            "v1"
        );
        assert!(!work.path().join("new.txt").exists());
        assert!(out.truncated_from_ts.is_none());
        // A pre_restore baseline was taken → restore is reversible.
        assert!(out.pre_restore_checkpoint_id.is_some());
    }

    #[tokio::test]
    async fn gc_removes_repo_and_rows() {
        let (m, _d, _s) = mgr(true).await;
        let work = tempfile::tempdir().unwrap();
        std::fs::write(work.path().join("a.txt"), "v1").unwrap();
        m.snapshot("s1", work.path(), 100, CheckpointKind::PreStep)
            .await
            .unwrap();
        assert_eq!(m.list("s1").await.unwrap().len(), 1);
        m.gc("s1").await.unwrap();
        assert!(m.list("s1").await.unwrap().is_empty());
        assert!(!m.git_dir("s1").exists());
    }
}
