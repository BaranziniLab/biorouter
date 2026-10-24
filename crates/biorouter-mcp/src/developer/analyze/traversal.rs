use crate::file_access::{file_error, EditorAccess, EditorPath};
use crate::secret_guard::SecretGuard;
use biorouter_sandbox::bounded_fs::{AccessMode, BoundedFs};
use ignore::gitignore::Gitignore;
use rayon::prelude::*;
use rmcp::model::{ErrorCode, ErrorData};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::developer::analyze::types::{AnalysisResult, EntryType};
use crate::developer::lang;

pub const MAX_ANALYSIS_ENTRIES: usize = 50_000;
pub const MAX_ANALYSIS_DIRECTORY_DEPTH: u32 = 256;

/// Handles file system traversal with ignore patterns
pub struct FileTraverser<'a> {
    ignore_patterns: &'a Gitignore,
    access: Mutex<Option<EditorAccess>>,
}

impl<'a> FileTraverser<'a> {
    /// Create a new file traverser with the given ignore patterns
    pub fn new(ignore_patterns: &'a Gitignore) -> Self {
        Self {
            ignore_patterns,
            access: Mutex::new(None),
        }
    }

    pub(crate) fn with_access(ignore_patterns: &'a Gitignore, access: EditorAccess) -> Self {
        Self {
            ignore_patterns,
            access: Mutex::new(Some(access)),
        }
    }

    pub(crate) fn bound(&self, path: &Path) -> Result<EditorPath, ErrorData> {
        let mut access = self.access.lock().map_err(file_error)?;
        if access.is_none() {
            // Direct Rust API callers select an explicit analysis root. MCP
            // calls supply the dispatch's already-retained root instead.
            let root = if path.is_dir() {
                path
            } else {
                path.parent().unwrap_or(Path::new("."))
            };
            let workspace = Arc::new(BoundedFs::open(root).map_err(file_error)?);
            *access = Some(EditorAccess {
                workspace,
                skills: None,
                guard: SecretGuard::for_dir(root),
                meta: Default::default(),
            });
        }
        access.as_ref().unwrap().bind(path, AccessMode::Read)
    }

    /// Check if a path should be ignored
    pub fn is_ignored(&self, path: &Path) -> bool {
        let ignored = self.ignore_patterns.matched(path, false).is_ignore();
        if ignored {
            tracing::trace!("Path {:?} is ignored", path);
        }
        ignored
    }

    /// Validate that a path exists and is not ignored
    pub fn validate_path(&self, path: &Path) -> Result<(), ErrorData> {
        // Check if path is ignored
        if self.is_ignored(path) {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "Access to '{}' is restricted by .biorouterignore",
                    path.display()
                ),
                None,
            ));
        }

        // Check if path exists
        if !self.bound(path)?.exists() {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!("Path '{}' does not exist", path.display()),
                None,
            ));
        }

        Ok(())
    }

    /// Collect all files for focused analysis
    pub fn collect_files_for_focused(
        &self,
        path: &Path,
        max_depth: u32,
    ) -> Result<Vec<PathBuf>, ErrorData> {
        tracing::debug!(
            "Collecting files from {:?} with max_depth {}",
            path,
            max_depth
        );

        if max_depth == 0 {
            tracing::warn!("Unlimited depth traversal requested for {:?}", path);
        }

        let mut remaining = MAX_ANALYSIS_ENTRIES;
        let files =
            self.collect_files_recursive(path, 0, max_depth, &mut HashSet::new(), &mut remaining)?;

        tracing::info!("Collected {} files from {:?}", files.len(), path);
        Ok(files)
    }

    /// Recursively collect files
    fn collect_files_recursive(
        &self,
        path: &Path,
        current_depth: u32,
        max_depth: u32,
        visited: &mut HashSet<String>,
        remaining: &mut usize,
    ) -> Result<Vec<PathBuf>, ErrorData> {
        let mut files = Vec::new();

        // Check if we're at a file (base case)
        let bound = self.bound(path)?;
        if bound.is_file() {
            let lang = lang::get_language_identifier(path);
            if !lang.is_empty() {
                tracing::trace!("Including file {:?} (language: {})", path, lang);
                files.push(path.to_path_buf());
            }
            return Ok(files);
        }

        // max_depth of 0 means unlimited depth
        // current_depth starts at 0, max_depth is the number of directory levels to traverse
        if max_depth > 0 && current_depth >= max_depth {
            tracing::trace!("Reached max depth {} at {:?}", max_depth, path);
            return Ok(files);
        }
        if current_depth >= MAX_ANALYSIS_DIRECTORY_DEPTH {
            return Err(file_error(
                "Analysis is too deeply nested; choose a narrower folder or depth",
            ));
        }

        let directory = bound.bound.open_dir().map_err(file_error)?;
        if !visited.insert(format!("{:?}", directory.identity())) {
            return Ok(files);
        }
        let mut entries = directory
            .bind(".", AccessMode::Read)
            .and_then(|path| path.list_limited(*remaining))
            .map_err(file_error)?;
        *remaining -= entries.len();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        let directory_guard = SecretGuard::for_bounded_path(&bound.bound).map_err(file_error)?;
        for entry in entries {
            let entry_path = path.join(&entry.name);
            let actual_path = directory.root().join(entry.name);
            if self.is_ignored(&entry_path) || directory_guard.is_denied(&actual_path) {
                continue;
            }
            let entry = self.bound(&entry_path)?;
            if entry.is_file() {
                if !lang::get_language_identifier(&entry_path).is_empty() {
                    files.push(entry_path);
                }
            } else if entry.is_dir() {
                let mut children = self.collect_files_recursive(
                    &entry_path,
                    current_depth + 1,
                    max_depth,
                    visited,
                    remaining,
                )?;
                files.append(&mut children);
            }
        }

        Ok(files)
    }

    /// Collect directory results for analysis with parallel processing
    pub fn collect_directory_results<F>(
        &self,
        path: &Path,
        max_depth: u32,
        analyze_file: F,
    ) -> Result<Vec<(PathBuf, EntryType)>, ErrorData>
    where
        F: Fn(&Path) -> Result<AnalysisResult, ErrorData> + Sync,
    {
        tracing::debug!("Collecting directory results from {:?}", path);

        // First collect all files to analyze
        let mut remaining = MAX_ANALYSIS_ENTRIES;
        let files_to_analyze =
            self.collect_files_recursive(path, 0, max_depth, &mut HashSet::new(), &mut remaining)?;

        // Then analyze them in parallel using Rayon
        let results: Result<Vec<_>, ErrorData> = files_to_analyze
            .par_iter()
            .map(|file_path| {
                analyze_file(file_path).map(|result| (file_path.clone(), EntryType::File(result)))
            })
            .collect();

        results
    }
}
