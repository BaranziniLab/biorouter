use std::path::{Path, PathBuf};

pub fn validate_kb_id(id: &str) -> Result<(), KbIdError> {
    if id.is_empty() {
        return Err(KbIdError::Empty);
    }
    if id.len() > 64 {
        return Err(KbIdError::TooLong);
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(KbIdError::InvalidChars);
    }
    if id.starts_with('-') || id.ends_with('-') || id.contains("--") {
        return Err(KbIdError::BadShape);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum KbIdError {
    #[error("kb-id is empty")]
    Empty,
    #[error("kb-id is longer than 64 characters")]
    TooLong,
    #[error("kb-id may only contain a-z, 0-9, and '-'")]
    InvalidChars,
    #[error("kb-id may not start/end with '-' or contain '--'")]
    BadShape,
}

/// `<config>/knowledge`, honouring `BIOROUTER_PATH_ROOT` via [`crate::paths`].
///
/// This used to hand-roll `choose_app_strategy` with an `io/biorouter/biorouter`
/// tuple — a *third* app-strategy in the codebase, and one that ignored the
/// sandbox override, so an isolated run read and wrote the user's global KBs.
/// It now shares the crate resolver (`Block/Block/biorouter`, matching
/// `biorouter::config::Paths`). The two tuples resolve identically on XDG and on
/// macOS; they differ only on Windows, where the old path was never the one the
/// rest of Biorouter used anyway.
pub fn knowledge_root() -> anyhow::Result<PathBuf> {
    Ok(knowledge_root_in(&crate::paths::config_dir()))
}

/// The same store, under a config root the caller resolved itself.
///
/// Exists so a caller that must NOT re-read the ambient `BIOROUTER_PATH_ROOT`
/// at write time — `knowledge::soul`'s seeders, which run in a task spawned by
/// `AgentManager::new` — can still spell the `<config>/knowledge` join exactly
/// once, here, rather than growing a second copy of it that drifts.
pub fn knowledge_root_in(config_dir: &Path) -> PathBuf {
    config_dir.join("knowledge")
}

pub fn kb_root(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

/// Returns `<knowledge-root>/.active-kb` — the file that persists the
/// **primary** knowledge base id (the write target for KB-less mutating calls).
///
/// The filename keeps its historical `.active-kb` spelling on purpose. The
/// merged model needs exactly one id, which is exactly what this file already
/// holds, so today's value *is* the primary and reading it is the entire
/// migration. It also keeps a lagging PATH-installed `biorouter` (see CLAUDE.md,
/// "Runtime CLI-vs-app drift") working: it reads a bare kb id whose meaning is
/// unchanged for it. Renaming the file, or writing anything structured into it,
/// would break that binary — `get_primary_persisted` performs no validation, so
/// it would happily join a JSON array into a filesystem path.
pub fn primary_kb_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".active-kb")
}

/// Returns `<knowledge-root>/.active-kb-sessions` — one file per session,
/// named `sha256(session_id)`, each holding that session's primary kb id.
/// Same naming rationale as [`primary_kb_path`].
pub fn primary_kb_sessions_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".active-kb-sessions")
}

pub fn hidden_kbs_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".hidden-kbs")
}

pub fn hidden_kb_sessions_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".hidden-kb-sessions")
}

/// Returns `<knowledge-root>/.kb-tiers` — the machine-local map of kb id →
/// privacy tier (issue #56).
///
/// Deliberately a sibling of `.active-kb` and `.hidden-kbs` rather than a field
/// in each base's `manifest.yaml`: the manifest is inside the base's git tree
/// and travels inside the `.brkb` archive, so a tier stored there would be
/// supplied by whoever authored the archive. This file never leaves the machine.
pub fn kb_tiers_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join(".kb-tiers")
}

/// The leaf name of [`model_export_dir`]. A **dot** prefix, and that is the
/// whole point: it fails [`validate_kb_id`], so it can never also be the id of a
/// knowledge base.
///
/// Without the dot this directory would be named `exports`, which is a perfectly
/// legal kb id — and `create_base` only refuses an id whose directory already
/// exists, so any session could create the base `exports` first. A private
/// session's `kb_export` would then write its archive *inside that public base's
/// own directory*, where `brkb::walk` packs every file it finds with no filter:
/// exporting the public base would hand out the whole private one. That is
/// exactly the laundering path decision (2) exists to close, walked in through
/// the directory name.
pub const MODEL_EXPORT_DIR: &str = ".exports";

/// Where a **model's** export of a **private** base is forced to land
/// (issue #56, decision 2b). See [`MODEL_EXPORT_DIR`] for why it is a dotfile.
pub fn model_export_dir(root: &Path) -> PathBuf {
    root.join(MODEL_EXPORT_DIR)
}

pub fn kb_knowledge_dir(root: &Path, id: &str) -> PathBuf {
    kb_root(root, id).join("knowledge")
}

pub fn kb_raw_dir(root: &Path, id: &str) -> PathBuf {
    kb_root(root, id).join("raw")
}

pub fn kb_internal_dir(root: &Path, id: &str) -> PathBuf {
    kb_root(root, id).join(".biorouter-knowledge")
}

/// Where every knowledge base's write lock lives: **beside** the bases, never
/// inside the one it locks. See [`kb_write_lock_path`] for why.
///
/// Dot-prefixed so it fails [`validate_kb_id`], which is what every root scanner
/// filters on (`tier::ensure_migrated_unlocked`, `soul::purge_unregistered_legacy`)
/// — a lock directory must not read as a knowledge base.
pub const KB_LOCKS_DIR: &str = ".kb-locks";

/// The cross-process write lock for one base.
///
/// ⚠ **It must not live under [`kb_root`], and the reason is Windows.** An open
/// handle to any file inside a directory makes Windows refuse to rename or
/// remove that directory (`ERROR_ACCESS_DENIED`, os error 5); Unix does not
/// care, because an fd follows the inode. The lock used to sit at
/// `<kb_root>/`[`KB_WRITE_LOCK_REL`], so every operation that holds the lock
/// *and* moves the base — staging a delete, renaming a base — worked on Unix
/// and failed on every Windows machine. Keeping the lock out of the tree it
/// guards makes the two platforms agree without releasing the lock early: a
/// release-then-move would hand a queued macro the base between the two steps,
/// which is a race rather than a fix.
///
/// The id is a bare filename component here, so callers must
/// [`validate_kb_id`] first — `service::lock_kb_path_cancellable` does.
pub fn kb_write_lock_path(root: &Path, id: &str) -> PathBuf {
    root.join(KB_LOCKS_DIR).join(format!("{id}.lock"))
}

/// The write lock's **historical** path, relative to a knowledge base's own
/// root. Bases created before [`kb_write_lock_path`] moved the lock out still
/// carry this file; it is inert, but it must stay out of commits and archives.
///
/// Spelled once here because three unrelated places have to agree on it and
/// none of them is a write: `service::GITIGNORE` keeps git from tracking it,
/// `git::stage_all` keeps it out of a commit even if the ignore file is
/// missing, and `brkb::walk` keeps it out of an archive. Every one of those is
/// "the transient lock is not content", and a fourth spelling of the same
/// string is how one of them silently stops holding.
pub const KB_WRITE_LOCK_REL: &str = ".biorouter-knowledge/write.lock";

/// Is `rel`, a path relative to a knowledge base's root, the write lock?
///
/// Compared as a [`Path`] rather than as a string on purpose: a caller that
/// built `rel` by walking the directory has the platform's separator in it, and
/// on Windows `.biorouter-knowledge\write.lock` must still match. `Path`'s
/// equality is component-wise and treats both separators as separators there,
/// so this holds on every platform; `==` over the raw strings would not.
pub fn is_kb_write_lock(rel: &Path) -> bool {
    rel == Path::new(KB_WRITE_LOCK_REL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_good_ids() {
        for id in ["a", "ms-patient", "kb-01", "personal", "x1-y2-z3"] {
            assert!(validate_kb_id(id).is_ok(), "should accept {id}");
        }
    }

    #[test]
    fn rejects_bad_ids() {
        for (id, want) in [
            ("", KbIdError::Empty),
            ("ABC", KbIdError::InvalidChars),
            ("with space", KbIdError::InvalidChars),
            ("with/slash", KbIdError::InvalidChars),
            ("-leading", KbIdError::BadShape),
            ("trailing-", KbIdError::BadShape),
            ("dou--ble", KbIdError::BadShape),
            ("../escape", KbIdError::InvalidChars),
        ] {
            assert_eq!(validate_kb_id(id).unwrap_err(), want, "for {id}");
        }
    }

    #[test]
    fn the_model_export_directory_can_never_be_a_knowledge_base_id() {
        // Issue #56. If this name validated, a session could `kb_create_base` it
        // and a private export would land inside a public base's own tree, where
        // `brkb::walk` would pack it into that base's next archive.
        assert!(validate_kb_id(MODEL_EXPORT_DIR).is_err());
        assert_eq!(
            model_export_dir(Path::new("/tmp/kb")),
            Path::new("/tmp/kb/.exports")
        );
    }

    #[test]
    fn path_helpers_compose() {
        let root = Path::new("/tmp/kb");
        assert_eq!(kb_root(root, "x"), Path::new("/tmp/kb/x"));
        assert_eq!(
            kb_knowledge_dir(root, "x"),
            Path::new("/tmp/kb/x/knowledge")
        );
        assert_eq!(kb_raw_dir(root, "x"), Path::new("/tmp/kb/x/raw"));
        assert_eq!(
            kb_internal_dir(root, "x"),
            Path::new("/tmp/kb/x/.biorouter-knowledge")
        );
    }

    #[test]
    fn the_write_lock_is_recognised_however_the_caller_spelled_the_separator() {
        assert!(is_kb_write_lock(Path::new(KB_WRITE_LOCK_REL)));
        assert!(is_kb_write_lock(Path::new(
            ".biorouter-knowledge/write.lock"
        )));
        // What a Windows directory walk hands back. `PathBuf::from(a).join(b)`
        // is what `brkb::walk` effectively has, and there the separator is the
        // platform's.
        assert!(is_kb_write_lock(
            &Path::new(".biorouter-knowledge").join("write.lock")
        ));
        // Neighbours in the same directory are content and must be packed.
        assert!(!is_kb_write_lock(Path::new(
            ".biorouter-knowledge/.crossref-cache"
        )));
        assert!(!is_kb_write_lock(Path::new("knowledge/write.lock")));
        assert!(!is_kb_write_lock(Path::new("write.lock")));
    }
}
