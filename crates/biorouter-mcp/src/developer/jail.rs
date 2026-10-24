//! Workspace path-jail for BRSDK app file access.
//!
//! Wraps the shared [`biorouter_sandbox::resolve_in_workspace`] primitive
//! (canonicalize + prefix + symlink-escape defence) with the policy an app's
//! `files` capability needs: a single workspace root, read-only vs read-write
//! enforcement, and an always-denied set of sensitive sub-paths (e.g. the
//! encrypted vault dir) so they can never be reached even from inside the jail.
//!
//! This is the file-access security primitive; wiring it into the Developer
//! server's path resolution (per-app construction) is the next sequenced step.

use std::path::{Component, Path, PathBuf};

use biorouter_sandbox::bounded_fs::{AccessMode, BoundedFs, BoundedPath};
use std::sync::Arc;

/// Reasons a path is refused by the jail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JailError {
    /// The path escapes the workspace root (traversal, absolute-outside, symlink).
    Escape(String),
    /// A write was attempted against a read-only jail.
    ReadOnly(String),
    /// The path is under an always-denied sub-path (e.g. the vault).
    Denied(String),
}

impl std::fmt::Display for JailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JailError::Escape(p) => write!(f, "path '{p}' escapes the app workspace"),
            JailError::ReadOnly(p) => write!(f, "path '{p}' is in a read-only workspace"),
            JailError::Denied(p) => write!(f, "path '{p}' is in a restricted area"),
        }
    }
}

impl std::error::Error for JailError {}

/// A workspace jail rooted at a single directory.
#[derive(Clone)]
pub struct Jail {
    root: PathBuf,
    filesystem: Option<Arc<BoundedFs>>,
    writable: bool,
    /// Path-component names that are always denied (matched anywhere in the path).
    denied_components: Vec<String>,
}

impl Jail {
    /// A jail at `root`. `writable` gates all write operations. The vault dir
    /// (`.vault`) and VCS metadata (`.git`) are always denied.
    pub fn new(root: impl Into<PathBuf>, writable: bool) -> Self {
        let root = root.into();
        let filesystem = BoundedFs::open(&root).ok().map(Arc::new);
        Jail {
            root,
            filesystem,
            writable,
            denied_components: vec![".vault".to_string(), ".git".to_string()],
        }
    }

    /// Add an always-denied path component (matched anywhere in the path).
    pub fn deny_component(mut self, name: impl Into<String>) -> Self {
        self.denied_components.push(name.into());
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn is_writable(&self) -> bool {
        self.writable
    }

    pub fn bind(&self, requested: &str, need_write: bool) -> Result<BoundedPath, JailError> {
        if need_write && !self.writable {
            return Err(JailError::ReadOnly(requested.to_string()));
        }
        if self.has_denied_component(Path::new(requested)) {
            return Err(JailError::Denied(requested.to_string()));
        }
        let filesystem = self
            .filesystem
            .as_ref()
            .ok_or_else(|| JailError::Escape(requested.to_string()))?;
        let bound = filesystem
            .bind(
                requested,
                if need_write {
                    AccessMode::Write
                } else {
                    AccessMode::Read
                },
            )
            .map_err(|_| JailError::Escape(requested.to_string()))?;
        if self.has_denied_component(bound.relative_path())
            || crate::memory::is_in_global_memory_store(&bound.absolute_path())
        {
            return Err(JailError::Denied(requested.to_string()));
        }
        Ok(bound)
    }

    /// The returned name is for display only. File operations use `bind` so
    /// validation and I/O retain the same directory capability.
    pub fn resolve(&self, requested: &str, need_write: bool) -> Result<PathBuf, JailError> {
        Ok(self.bind(requested, need_write)?.absolute_path())
    }

    fn has_denied_component(&self, p: &Path) -> bool {
        p.components().any(|c| {
            if let Component::Normal(os) = c {
                let s = os.to_string_lossy();
                self.denied_components.iter().any(|d| d == s.as_ref())
            } else {
                false
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn resolves_in_workspace_paths() {
        let dir = ws();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a.txt"), b"x").unwrap();
        let jail = Jail::new(dir.path(), true);
        assert!(jail.resolve("sub/a.txt", false).is_ok());
        // New-file write path under the root is allowed.
        assert!(jail.resolve("sub/new.txt", true).is_ok());
    }

    #[test]
    fn rejects_traversal_and_absolute_outside() {
        let dir = ws();
        let jail = Jail::new(dir.path(), true);
        assert!(matches!(
            jail.resolve("../../etc/passwd", false),
            Err(JailError::Escape(_))
        ));
        assert!(matches!(
            jail.resolve("/etc/hosts", false),
            Err(JailError::Escape(_))
        ));
    }

    #[test]
    #[cfg(unix)]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let dir = ws();
        symlink("/etc", dir.path().join("escape")).unwrap();
        let jail = Jail::new(dir.path(), true);
        assert!(matches!(
            jail.resolve("escape/hosts", false),
            Err(JailError::Escape(_))
        ));
    }

    #[test]
    fn enforces_read_only() {
        let dir = ws();
        let jail = Jail::new(dir.path(), false);
        assert!(jail.resolve("a.txt", false).is_ok()); // reads ok
        assert!(matches!(
            jail.resolve("a.txt", true),
            Err(JailError::ReadOnly(_))
        )); // writes refused
    }

    #[test]
    fn denies_vault_and_git_even_inside_workspace() {
        let dir = ws();
        std::fs::create_dir_all(dir.path().join(".vault")).unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        let jail = Jail::new(dir.path(), true);
        assert!(matches!(
            jail.resolve(".vault/secret.enc", false),
            Err(JailError::Denied(_))
        ));
        assert!(matches!(
            jail.resolve(".git/config", false),
            Err(JailError::Denied(_))
        ));
        // A nested denied component is also caught.
        assert!(matches!(
            jail.resolve("sub/.git/config", false),
            Err(JailError::Denied(_))
        ));
    }

    #[test]
    #[cfg(unix)]
    fn denies_symlink_into_vault() {
        use std::os::unix::fs::symlink;
        let dir = ws();
        std::fs::create_dir_all(dir.path().join(".vault")).unwrap();
        std::fs::write(dir.path().join(".vault/k.enc"), b"secret").unwrap();
        // A sneaky symlink that points into the vault must still be denied.
        symlink(dir.path().join(".vault"), dir.path().join("peek")).unwrap();
        let jail = Jail::new(dir.path(), true);
        // The resolved real path contains ".vault" → denied.
        assert!(matches!(
            jail.resolve("peek/k.enc", false),
            Err(JailError::Denied(_))
        ));
    }
}
