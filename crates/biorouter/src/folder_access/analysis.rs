use std::collections::HashSet;
use std::path::{Path, PathBuf};

use biorouter_mcp::developer::analyze::traversal::{
    MAX_ANALYSIS_DIRECTORY_DEPTH, MAX_ANALYSIS_ENTRIES,
};
use biorouter_mcp::secret_guard::SecretGuard;
use biorouter_sandbox::bounded_fs::{authorized_path, BoundedPath};
use biorouter_sandbox::{AccessMode, DispatchAuthority};
use tokio_util::sync::CancellationToken;

use super::discover_target;

struct PendingEntry {
    path: PathBuf,
    depth: u32,
    link_target: Option<PathBuf>,
}

#[derive(Debug)]
pub(super) struct ExternalAnalysisTarget {
    pub requested: PathBuf,
    pub actual: PathBuf,
    pub recursive: bool,
}

pub(super) struct AnalysisDiscovery {
    pending: Vec<PendingEntry>,
    visited: HashSet<String>,
    remaining: usize,
    max_depth: u32,
    ignore: SecretGuard,
}

impl AnalysisDiscovery {
    pub fn new(root: &BoundedPath, max_depth: u32) -> std::io::Result<Self> {
        Ok(Self {
            pending: vec![PendingEntry {
                path: root.absolute_path(),
                depth: 0,
                link_target: None,
            }],
            visited: HashSet::new(),
            remaining: MAX_ANALYSIS_ENTRIES,
            max_depth,
            ignore: SecretGuard::for_bounded_path(root)?,
        })
    }

    pub fn next_external(
        &mut self,
        authority: &DispatchAuthority,
        session: &str,
        action: &str,
        cancel: &CancellationToken,
    ) -> anyhow::Result<Option<ExternalAnalysisTarget>> {
        while let Some(entry) = self.pending.pop() {
            anyhow::ensure!(!cancel.is_cancelled(), "Folder operation was cancelled");
            if self
                .ignore
                .gitignore()
                .matched(&entry.path, false)
                .is_ignore()
            {
                continue;
            }
            let bound = match authorized_path(
                authority.token(),
                session,
                action,
                &entry.path,
                AccessMode::Read,
            ) {
                Ok(bound) => bound,
                Err(error) => {
                    let Some(target) = &entry.link_target else {
                        return Err(error.into());
                    };
                    let actual = discover_target(target)?;
                    let target = ExternalAnalysisTarget {
                        requested: entry.path.clone(),
                        recursive: actual.is_dir(),
                        actual,
                    };
                    self.pending.push(entry);
                    return Ok(Some(target));
                }
            };
            if !bound.metadata()?.is_dir() || (self.max_depth > 0 && entry.depth >= self.max_depth)
            {
                continue;
            }
            anyhow::ensure!(
                entry.depth < MAX_ANALYSIS_DIRECTORY_DEPTH,
                "Analysis is too deeply nested; choose a narrower folder or depth"
            );
            self.schedule_children(&bound, &entry.path, entry.depth)?;
        }
        Ok(None)
    }

    fn schedule_children(
        &mut self,
        bound: &BoundedPath,
        requested: &Path,
        depth: u32,
    ) -> anyhow::Result<()> {
        let directory = bound.open_dir()?;
        if !self.visited.insert(format!("{:?}", directory.identity())) {
            return Ok(());
        }
        let mut entries = directory
            .bind(".", AccessMode::Read)?
            .list_limited(self.remaining)?;
        self.remaining -= entries.len();
        let directory_guard = SecretGuard::for_bounded_path(bound)?;
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        for entry in entries.into_iter().rev() {
            let path = requested.join(&entry.name);
            let actual = directory.root().join(&entry.name);
            if self.ignore.gitignore().matched(&path, false).is_ignore()
                || directory_guard.is_denied(&actual)
            {
                continue;
            }
            let link_target = if entry.is_symlink {
                let target = directory.read_link_contents(&entry.name)?;
                Some(if target.is_absolute() {
                    target
                } else {
                    directory.root().join(target)
                })
            } else {
                None
            };
            self.pending.push(PendingEntry {
                path,
                depth: depth + 1,
                link_target,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use biorouter_sandbox::{BoundedFs, RootGrant};

    use super::*;

    #[test]
    fn discovery_stops_before_listing_beyond_its_entry_budget() {
        let temporary = tempfile::tempdir().unwrap();
        let filesystem = Arc::new(BoundedFs::open(temporary.path()).unwrap());
        for name in ["first.rs", "second.rs"] {
            filesystem
                .bind(name, AccessMode::Write)
                .unwrap()
                .atomic_write(b"synthetic")
                .unwrap();
        }
        let authority = DispatchAuthority::register(
            "session",
            "analysis",
            vec![RootGrant::new(filesystem, AccessMode::Read)],
        )
        .unwrap();
        let root = authorized_path(
            authority.token(),
            "session",
            "analysis",
            temporary.path(),
            AccessMode::Read,
        )
        .unwrap();
        let mut discovery = AnalysisDiscovery::new(&root, 0).unwrap();
        discovery.remaining = 1;
        assert!(discovery
            .next_external(&authority, "session", "analysis", &CancellationToken::new())
            .unwrap_err()
            .to_string()
            .contains("too many directory entries"));
    }

    #[cfg(unix)]
    #[test]
    fn unlimited_depth_still_stops_at_a_retained_directory_cycle() {
        let temporary = tempfile::tempdir().unwrap();
        let root_path = temporary.path().canonicalize().unwrap();
        std::os::unix::fs::symlink(&root_path, root_path.join("cycle")).unwrap();
        let filesystem = Arc::new(BoundedFs::open(&root_path).unwrap());
        let authority = DispatchAuthority::register(
            "session",
            "analysis",
            vec![RootGrant::new(filesystem, AccessMode::Read)],
        )
        .unwrap();
        let root = authorized_path(
            authority.token(),
            "session",
            "analysis",
            &root_path,
            AccessMode::Read,
        )
        .unwrap();
        let mut discovery = AnalysisDiscovery::new(&root, 0).unwrap();
        assert!(discovery
            .next_external(&authority, "session", "analysis", &CancellationToken::new())
            .unwrap()
            .is_none());
        assert_eq!(discovery.visited.len(), 1);
    }
}
