//! BR-43 — shadow-git checkpoints + three-axis restore (files / conversation /
//! both).
//!
//! A shadow git repository per session captures the work-tree before/after
//! mutating model steps into a private object DB in the app data dir — never the
//! user's `.git`. Each snapshot is recorded in the SQLite `checkpoints` side
//! table (migration 11), keyed to the turn's anchor `created_timestamp`.
//! `CheckpointManager::restore` then rewinds along one of three axes.
//!
//! First mergeable slice (design `docs/agent-loop/designs/shadow-git-checkpoints.md`,
//! Slice 1): the module + turn-boundary capture + programmatic restore API,
//! gated off by default (`BIOROUTER_CHECKPOINTS`). No server routes / GUI yet.

mod manager;
mod store;

pub use manager::CheckpointManager;
pub use store::{Caps, ShadowRepo};

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

/// The directory holding one chat's shadow repository and nothing else:
/// `<data_root>/checkpoints/<session_id>` (the repository itself is its `git/`).
///
/// `None` unless the id is exactly one plain path component, because the id
/// comes from a free-text column and the result is handed to
/// `remove_dir_all`: `.` would name every chat's repository at once, `..` the
/// data directory, and `a/.` would name chat `a`'s. No id `create_session`
/// mints is any of those, but a restored or hand-edited database can hold one.
pub(crate) fn repository_dir(data_root: &Path, session_id: &str) -> Option<PathBuf> {
    let mut components = Path::new(session_id).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) if name == std::ffi::OsStr::new(session_id) => {
            Some(data_root.join("checkpoints").join(session_id))
        }
        _ => None,
    }
}

/// Which snapshot boundary produced a checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointKind {
    /// Taken as a turn opens, before the agent has mutated anything.
    PreStep,
    /// Taken after a mutating model step's tool responses are persisted.
    PostStep,
    /// User-requested "mark a checkpoint here".
    Manual,
    /// Baseline taken immediately before a restore, so the restore is reversible.
    PreRestore,
}

impl CheckpointKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckpointKind::PreStep => "pre_step",
            CheckpointKind::PostStep => "post_step",
            CheckpointKind::Manual => "manual",
            CheckpointKind::PreRestore => "pre_restore",
        }
    }
}

impl FromStr for CheckpointKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pre_step" => Ok(CheckpointKind::PreStep),
            "post_step" => Ok(CheckpointKind::PostStep),
            "manual" => Ok(CheckpointKind::Manual),
            "pre_restore" => Ok(CheckpointKind::PreRestore),
            other => Err(anyhow::anyhow!("unknown checkpoint kind: {other}")),
        }
    }
}

/// One row in the `checkpoints` table (a shadow-repo commit + its metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointRecord {
    pub id: String,
    pub session_id: String,
    /// Monotonic per-session ordinal, for display/order.
    pub turn_index: i64,
    /// `created_timestamp` (ms) of the user message that opened the turn — the
    /// stable anchor `truncate_conversation` also keys on.
    pub anchor_ts: i64,
    pub kind: CheckpointKind,
    /// Commit oid in the shadow repo.
    pub commit_sha: String,
    /// Tree oid — O(1) dedup key between consecutive snapshots.
    pub tree_sha: String,
    /// Paths whose blobs differ from the previous checkpoint (best-effort).
    pub changed_paths: Vec<String>,
    pub created_at: String,
}

/// Which axis (or axes) a restore rewinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestoreAxis {
    /// Check out the checkpoint's tree into the work-tree.
    Files,
    /// Truncate the conversation at the checkpoint's anchor timestamp.
    Conversation,
    /// Both, atomically.
    Both,
}

impl RestoreAxis {
    fn touches_files(self) -> bool {
        matches!(self, RestoreAxis::Files | RestoreAxis::Both)
    }
    fn touches_conversation(self) -> bool {
        matches!(self, RestoreAxis::Conversation | RestoreAxis::Both)
    }
}

/// Result of one restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreOutcome {
    pub checkpoint_id: String,
    pub axis: RestoreAxis,
    /// Paths written back on a files/both restore.
    pub restored_paths: Vec<String>,
    /// The timestamp the conversation was truncated from (conversation/both).
    pub truncated_from_ts: Option<i64>,
    /// The reversible baseline snapshot taken just before the restore.
    pub pre_restore_checkpoint_id: Option<String>,
}

/// Runtime configuration (caps + on/off), read from the environment so the
/// first slice ships behind a flag and old sessions are untouched.
#[derive(Debug, Clone, Copy)]
pub struct CheckpointConfig {
    pub enabled: bool,
    /// Per-file blob cap (bytes). Files over this are skipped.
    pub max_file_bytes: u64,
    /// Skip snapshotting entirely when the eligible work-tree exceeds this.
    pub max_tree_bytes: u64,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_file_bytes: 2 * 1024 * 1024,
            max_tree_bytes: 512 * 1024 * 1024,
        }
    }
}

impl CheckpointConfig {
    /// Build from env. Disabled by default (a defaults-changing feature must be
    /// flag-gated); enabled by `BIOROUTER_CHECKPOINTS` truthy, or by `ALPHA`
    /// truthy unless `BIOROUTER_CHECKPOINTS` is explicitly falsey.
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        let explicit = std::env::var("BIOROUTER_CHECKPOINTS")
            .ok()
            .map(|v| truthy(&v));
        cfg.enabled = match explicit {
            Some(v) => v,
            None => std::env::var("ALPHA")
                .ok()
                .map(|v| truthy(&v))
                .unwrap_or(false),
        };
        if let Ok(mb) = std::env::var("BIOROUTER_CHECKPOINT_MAX_FILE_MB") {
            if let Ok(mb) = mb.trim().parse::<u64>() {
                cfg.max_file_bytes = mb.saturating_mul(1024 * 1024);
            }
        }
        if let Ok(mb) = std::env::var("BIOROUTER_CHECKPOINT_MAX_TREE_MB") {
            if let Ok(mb) = mb.trim().parse::<u64>() {
                cfg.max_tree_bytes = mb.saturating_mul(1024 * 1024);
            }
        }
        cfg
    }

    pub fn caps(&self) -> Caps {
        Caps {
            max_file_bytes: self.max_file_bytes,
        }
    }
}

fn truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_roundtrip() {
        for k in [
            CheckpointKind::PreStep,
            CheckpointKind::PostStep,
            CheckpointKind::Manual,
            CheckpointKind::PreRestore,
        ] {
            assert_eq!(CheckpointKind::from_str(k.as_str()).unwrap(), k);
        }
    }

    #[test]
    fn axis_predicates() {
        assert!(RestoreAxis::Files.touches_files());
        assert!(!RestoreAxis::Files.touches_conversation());
        assert!(RestoreAxis::Conversation.touches_conversation());
        assert!(!RestoreAxis::Conversation.touches_files());
        assert!(RestoreAxis::Both.touches_files());
        assert!(RestoreAxis::Both.touches_conversation());
    }
}
