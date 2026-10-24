use super::patch_content::apply_content;
use crate::file_access::{file_error, EditorPath};
use anyhow::Result;
use biorouter_sandbox::bounded_fs::AccessMode;
use indoc::formatdoc;
use mpatch::{parse_diffs, PatchError};
use std::path::{Path, PathBuf};
use url::Url;

use rmcp::model::{Content, ErrorCode, ErrorData, Role};

use super::editor_models::EditorModel;
use super::lang;
use super::shell::normalize_line_endings_like;
use super::undo_history::FileHistory;

fn ensure_trailing_newline(content: &mut String, existing: Option<&str>) {
    if content.ends_with('\n') {
        return;
    }

    content.push_str(&normalize_line_endings_like(existing, "\n"));
}

// Constants
pub const LINE_READ_LIMIT: usize = 2000;
pub const MAX_DIFF_SIZE: usize = 1024 * 1024; // 1MB max diff size
pub const MAX_FILES_IN_DIFF: usize = 100; // Maximum files in a multi-file diff

/// Results from applying a diff
#[derive(Debug, Default)]
pub struct DiffResults {
    files_created: usize,
    files_modified: usize,
    files_deleted: usize,
    lines_added: usize,
    lines_removed: usize,
}

/// Validates the size of the diff content
fn validate_diff_size(diff_content: &str) -> Result<(), ErrorData> {
    if diff_content.len() > MAX_DIFF_SIZE {
        return Err(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            format!(
                "Diff is too large ({} bytes). Maximum size is {} bytes (1MB).",
                diff_content.len(),
                MAX_DIFF_SIZE
            ),
            None,
        ));
    }
    Ok(())
}

/// Counts line changes from the diff content
fn count_line_changes(diff_content: &str) -> (usize, usize) {
    let lines_added = diff_content
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .count();
    let lines_removed = diff_content
        .lines()
        .filter(|l| l.starts_with('-') && !l.starts_with("---"))
        .count();
    (lines_added, lines_removed)
}

/// Generates the summary for the diff application
fn generate_summary(results: &DiffResults, is_single_file: bool, base_path: &Path) -> Vec<Content> {
    let summary = if is_single_file {
        format!(
            "Successfully applied diff to {}:\n• Lines added: {}\n• Lines removed: {}",
            base_path.display(),
            results.lines_added,
            results.lines_removed
        )
    } else if results.files_created + results.files_modified + results.files_deleted > 1 {
        format!(
            "Successfully applied multi-file diff:\n\
            • Files created: {}\n\
            • Files modified: {}\n\
            • Files deleted: {}\n\
            • Lines added: {}\n\
            • Lines removed: {}",
            results.files_created,
            results.files_modified,
            results.files_deleted,
            results.lines_added,
            results.lines_removed
        )
    } else {
        format!(
            "Successfully applied diff:\n\
            • Files created: {}\n\
            • Files modified: {}\n\
            • Files deleted: {}\n\
            • Lines added: {}\n\
            • Lines removed: {}",
            results.files_created,
            results.files_modified,
            results.files_deleted,
            results.lines_added,
            results.lines_removed
        )
    };

    let user_message = if is_single_file {
        format!("{}\n\nUse 'undo_edit' to revert if needed.\n\n", summary)
    } else {
        format!(
            "{}\n\nUse 'undo_edit' on individual files to revert if needed.\n\n",
            summary
        )
    };

    vec![
        Content::text(summary.clone()).with_audience(vec![Role::Assistant]),
        Content::text(user_message)
            .with_audience(vec![Role::User])
            .with_priority(0.2),
    ]
}

fn adjust_base_dir_for_overlap(base_dir: &Path, file_path: &Path) -> PathBuf {
    let base_components: Vec<_> = base_dir.components().collect();
    let file_components: Vec<_> = file_path.components().collect();

    let min_len = base_components.len().min(file_components.len());
    let max_k = (1..=min_len)
        .rfind(|&k| file_components[0..k] == base_components[base_components.len() - k..])
        .unwrap_or(0);

    if max_k > 0 {
        let adjusted_components = base_components[..base_components.len() - max_k].to_vec();
        PathBuf::from_iter(adjusted_components)
    } else {
        base_dir.to_path_buf()
    }
}

/// Parses diff content into patches with proper error handling
fn parse_diff_content(diff_content: &str) -> Result<Vec<mpatch::Patch>, ErrorData> {
    let wrapped_diff = if diff_content.contains("```diff") || diff_content.contains("```patch") {
        diff_content.to_string()
    } else {
        format!("```diff\n{}\n```", diff_content)
    };

    parse_diffs(&wrapped_diff).map_err(|e| match e {
        PatchError::MissingFileHeader => ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            "Invalid diff format: Missing file header (e.g., '--- a/path/to/file')".to_string(),
            None,
        ),
        PatchError::Io { path, source } => ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("I/O error processing {}: {}", path.display(), source),
            None,
        ),
        PatchError::PathTraversal(path) => ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            format!(
                "Security: Path '{}' would escape the base directory",
                path.display()
            ),
            None,
        ),
        PatchError::TargetNotFound(path) => ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("Target file not found: {}", path.display()),
            None,
        ),
    })
}

/// Reports partial failures from patch application
fn report_partial_failures(failed_hunks: &[String]) {
    if !failed_hunks.is_empty() {
        let error_msg = format!(
            "Some patches were only partially applied (fuzzy matching at 70% similarity):\n\n{}\n\n\
            The files have been modified but some hunks couldn't find their context.\n\
            This usually happens when:\n\
            • The file has changed significantly from when the diff was created\n\
            • Line numbers in the diff are incorrect\n\
            • The context lines don't match exactly\n\n\
            Review the changes and use 'undo_edit' if needed.",
            failed_hunks.join("\n")
        );

        tracing::warn!("{}", error_msg);
    }
}

pub(crate) fn diff_target_paths(base_path: &Path, diff: &str) -> Result<Vec<PathBuf>, ErrorData> {
    diff_target_paths_for_kind(base_path, diff, base_path.is_file())
}

pub(crate) fn diff_target_paths_bounded(
    base_path: &EditorPath,
    diff: &str,
) -> Result<Vec<PathBuf>, ErrorData> {
    diff_target_paths_for_kind(base_path, diff, base_path.is_file())
}

fn diff_target_paths_for_kind(
    base_path: &Path,
    diff: &str,
    is_file: bool,
) -> Result<Vec<PathBuf>, ErrorData> {
    validate_diff_size(diff)?;
    let patches = parse_diff_content(diff)?;
    if patches.len() > MAX_FILES_IN_DIFF {
        return Err(file_error("Too many files in diff"));
    }
    let base_dir = if is_file {
        base_path.parent().unwrap_or(Path::new("."))
    } else {
        base_path
    };
    patches
        .iter()
        .map(|patch| {
            if patch.file_path.components().any(|part| {
                matches!(
                    part,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            }) {
                return Err(file_error(
                    "Patch paths must be relative without '..' components",
                ));
            }
            let target =
                adjust_base_dir_for_overlap(base_dir, &patch.file_path).join(&patch.file_path);
            if !target.starts_with(base_dir) {
                return Err(file_error("Patch target is outside the selected directory"));
            }
            Ok(target)
        })
        .collect()
}

pub(crate) async fn apply_diff_bounded(
    base_path: &EditorPath,
    diff_content: &str,
    file_history: &FileHistory,
) -> Result<Vec<Content>, ErrorData> {
    let targets = diff_target_paths_bounded(base_path, diff_content)?;
    let patches = parse_diff_content(diff_content)?;
    // Bind and policy-check every child, and prepare all content, before any
    // filesystem mutation. The third-party patch engine never receives a path.
    let bound: Vec<_> = targets
        .iter()
        .map(|target| base_path.access.bind(target, AccessMode::Write))
        .collect::<Result<_, _>>()?;
    let mut prepared = Vec::new();
    let mut staged = std::collections::HashMap::new();
    let mut results = DiffResults::default();
    let mut failed_hunks = Vec::new();
    for (patch, target) in patches.iter().zip(&bound) {
        let key = target.bound.absolute_path();
        let original = if let Some(content) = staged.get(&key) {
            Some(String::clone(content))
        } else {
            match target.read_to_string() {
                Ok(content) => Some(content),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(file_error(error)),
            }
        };
        let (updated, complete) = apply_content(patch, original.as_deref()).ok_or_else(|| {
            file_error(format!(
                "File '{}' not found and patch doesn't create it",
                target.display()
            ))
        })?;
        let mut updated = normalize_line_endings_like(original.as_deref(), &updated);
        ensure_trailing_newline(&mut updated, original.as_deref());
        if !complete {
            failed_hunks.push(format!(
                "Failed to apply some hunks to '{}' ({} hunks total)",
                target.display(),
                patch.hunks.len()
            ));
        }
        if original.is_some() {
            results.files_modified += 1;
        } else {
            results.files_created += 1;
        }
        staged.insert(key, updated.clone());
        prepared.push((target, original, updated));
    }
    for (target, original, updated) in prepared {
        target.write(&updated).map_err(file_error)?;
        if let Some(original) = original {
            file_history.record(
                &target.bound.try_history_key().map_err(file_error)?,
                original,
            );
        }
    }
    report_partial_failures(&failed_hunks);
    let (added, removed) = count_line_changes(diff_content);
    results.lines_added = added;
    results.lines_removed = removed;
    Ok(generate_summary(&results, patches.len() == 1, base_path))
}

// Helper method to validate and calculate view range indices
pub fn calculate_view_range(
    view_range: Option<(usize, i64)>,
    total_lines: usize,
) -> Result<(usize, usize), ErrorData> {
    if let Some((start_line, end_line)) = view_range {
        // Convert 1-indexed line numbers to 0-indexed
        let start_idx = if start_line > 0 { start_line - 1 } else { 0 };
        let end_idx = if end_line == -1 {
            total_lines
        } else {
            std::cmp::min(end_line as usize, total_lines)
        };

        if start_idx >= total_lines {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "Start line {} is beyond the end of the file (total lines: {})",
                    start_line, total_lines
                ),
                None,
            ));
        }

        if start_idx >= end_idx {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "Start line {} must be less than end line {}",
                    start_line, end_line
                ),
                None,
            ));
        }

        Ok((start_idx, end_idx))
    } else {
        Ok((0, total_lines))
    }
}

// Helper method to format file content with line numbers
pub fn format_file_content(
    path: &Path,
    lines: &[&str],
    start_idx: usize,
    end_idx: usize,
    view_range: Option<(usize, i64)>,
) -> String {
    let display_content = if lines.is_empty() {
        String::new()
    } else {
        let selected_lines: Vec<String> = lines[start_idx..end_idx]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}: {}", start_idx + i + 1, line))
            .collect();

        selected_lines.join("\n")
    };

    let language = lang::get_language_identifier(path);
    if let Some((start, end)) = view_range {
        formatdoc! {"
            ### {path} (lines {start}-{end})
            ```{language}
            {content}
            ```
            ",
            path=path.display(),
            start=start,
            end=if end == -1 { "end".to_string() } else { end.to_string() },
            language=language,
            content=display_content,
        }
    } else {
        formatdoc! {"
            ### {path}
            ```{language}
            {content}
            ```
            ",
            path=path.display(),
            language=language,
            content=display_content,
        }
    }
}

/// The file text the **model** receives from a `view`: exactly the bytes of
/// lines `start_idx..end_idx`, and nothing outside them.
///
/// The user's copy is [`format_file_content`], which has honoured `view_range`
/// all along. This one had not: it shipped the whole file however narrow the
/// request, so `view_range` cost the model nothing and saved it nothing. That
/// also defeated [`recommend_read_range`], whose entire job is to stop a file
/// over [`LINE_READ_LIMIT`] lines from landing in the request: it forces the
/// model to name a range and the model then received the file anyway.
///
/// **Byte-exact, not re-joined.** `str::lines()` drops the `\r` of a `\r\n` pair
/// and the file's final newline, so rebuilding the slice from it would hand the
/// model LF text for a CRLF file and invite a `str_replace` whose `old_str`
/// cannot match. `split_inclusive('\n')` keeps each line's own terminator, and
/// it partitions the string, so concatenating a run of its pieces returns the
/// original bytes of those lines.
///
/// The whole-file case returns `content` itself. That is an allocation saved,
/// not a second rule: [`calculate_view_range`] answers `(0, total_lines)` when
/// no range was asked for, and `split_inclusive('\n')` yields exactly
/// `total_lines` pieces, so the slice would rebuild an identical string.
/// `the_two_paths_agree_on_a_whole_file` pins the two together.
///
/// The model is not told which lines these are, and does not need to be: the
/// `view_range` it asked for sits in the same request, one message above the
/// result.
pub fn model_file_view(
    content: String,
    start_idx: usize,
    end_idx: usize,
    total_lines: usize,
) -> String {
    if start_idx == 0 && end_idx == total_lines {
        return content;
    }

    content
        .split_inclusive('\n')
        .skip(start_idx)
        .take(end_idx.saturating_sub(start_idx))
        .collect()
}

pub fn recommend_read_range(path: &Path, total_lines: usize) -> Result<Vec<Content>, ErrorData> {
    Err(ErrorData::new(ErrorCode::INTERNAL_ERROR, format!(
        "File '{}' is {} lines long, recommended to read in with view_range (or searching) to get bite size content. If you do wish to read all the file, please pass in view_range with [1, {}] to read it all at once",
        path.display(),
        total_lines,
        total_lines
    ), None))
}

/// Lists the contents of a directory with a maximum number of items
fn list_directory_contents(path: &EditorPath) -> Result<Vec<Content>, ErrorData> {
    const MAX_ITEMS: usize = 50; // Maximum number of items to display

    // List files in the directory (similar to ls output)
    let entries = path.bound.list().map_err(file_error)?;

    let mut files = Vec::new();
    let mut dirs = Vec::new();
    let mut total_count = 0;

    for entry in entries {
        total_count += 1;

        // Only process up to MAX_ITEMS entries
        if dirs.len() + files.len() < MAX_ITEMS {
            let name = entry.name.to_string_lossy().to_string();
            if entry.is_dir {
                dirs.push(format!("{}/", name));
            } else {
                files.push(name);
            }
        }
    }

    // Sort for consistent output
    dirs.sort();
    files.sort();

    let mut output = format!("'{}' is a directory. Contents:\n\n", path.display());

    if !dirs.is_empty() {
        output.push_str("Directories:\n");
        for dir in &dirs {
            output.push_str(&format!("  {}\n", dir));
        }
        output.push('\n');
    }

    if !files.is_empty() {
        output.push_str("Files:\n");
        for file in &files {
            output.push_str(&format!("  {}\n", file));
        }
    }

    if dirs.is_empty() && files.is_empty() {
        output.push_str("  (empty directory)\n");
    }

    // If we hit the limit, indicate there are more items
    if total_count > MAX_ITEMS {
        output.push_str(&format!(
            "\n... and {} more items (showing first {} items)\n",
            total_count - MAX_ITEMS,
            MAX_ITEMS
        ));
    }

    Ok(vec![Content::text(output)])
}

pub(crate) async fn text_editor_view_bounded(
    path: &EditorPath,
    view_range: Option<(usize, i64)>,
) -> Result<Vec<Content>, ErrorData> {
    // Check if path is a directory
    if path.is_dir() {
        return list_directory_contents(path);
    }

    if !path.is_file() {
        return Err(ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!(
                "The path '{}' does not exist or is not accessible.",
                path.display()
            ),
            None,
        ));
    }

    const MAX_FILE_SIZE: u64 = 400 * 1024; // 400KB

    let file_size = path.bound.metadata().map_err(file_error)?.len();

    if file_size > MAX_FILE_SIZE {
        return Err(ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!(
                "File '{}' is too large ({:.2}KB). Maximum size is 400KB to prevent memory issues.",
                path.display(),
                file_size as f64 / 1024.0
            ),
            None,
        ));
    }

    // Ensure we never read over that limit even if the file is being concurrently mutated

    let uri = Url::from_file_path(&**path)
        .map_err(|_| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Invalid file path".to_string(),
                None,
            )
        })?
        .to_string();

    let content = path
        .bound
        .read_to_string_limited(MAX_FILE_SIZE)
        .map_err(file_error)?;

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    // We will gently encourage the LLM to specify a range for large line count files
    // it can of course specify exact range to read any size file
    if view_range.is_none() && total_lines > LINE_READ_LIMIT {
        return recommend_read_range(path, total_lines);
    }

    let (start_idx, end_idx) = calculate_view_range(view_range, total_lines)?;
    let formatted = format_file_content(path, &lines, start_idx, end_idx, view_range);
    drop(lines);
    let for_model = model_file_view(content, start_idx, end_idx, total_lines);

    // Two renderings of the same range: the raw bytes for the model, the
    // numbered and fenced version for the user. `uri` names the file either
    // way, with no marker for a partial read — the CLI's markdown export reads
    // the syntax highlighting off the text after the URI's last `.`
    // (`session/export.rs`), so a `#L3-L6` suffix would turn `rs` into
    // `rs#L3-L6` and silently unhighlight every exported file view.
    Ok(vec![
        Content::embedded_text(uri, for_model).with_audience(vec![Role::Assistant]),
        Content::text(formatted)
            .with_audience(vec![Role::User])
            .with_priority(0.0),
    ])
}

pub(crate) async fn text_editor_write_bounded(
    path: &EditorPath,
    file_text: &str,
) -> Result<Vec<Content>, ErrorData> {
    // Overwriting an existing file keeps that file's line endings; only a file
    // being created takes the host convention. Without this, every write from
    // Windows rewrote an LF file to CRLF end to end -- a whole-file diff for a
    // one-line change, and a `view` whose output no longer matched the next
    // `old_str` the model sent back.
    let existing = path.read_to_string().ok();
    let mut normalized_text = normalize_line_endings_like(existing.as_deref(), file_text);
    ensure_trailing_newline(&mut normalized_text, existing.as_deref());

    // Write to the file
    path.write(&normalized_text) // Write the potentially modified text
        .map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to write file: {}", e),
                None,
            )
        })?;

    // Try to detect the language from the file extension
    let language = lang::get_language_identifier(path);

    // The assistant output does not show the file again because the content is already in the tool request
    // but we do show it to the user here, using the final written content
    Ok(vec![
        Content::text(format!("Successfully wrote to {}", path.display()))
            .with_audience(vec![Role::Assistant]),
        Content::text(formatdoc! {
            r#"
            ### {path}
            ```{language}
            {content}
            ```
            "#,
            path=path.display(),
            language=language,
            content=&normalized_text // Use the final normalized_text for user feedback
        })
        .with_audience(vec![Role::User])
        .with_priority(0.2),
    ])
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn text_editor_replace_bounded(
    path: &EditorPath,
    old_str: &str,
    new_str: &str,
    diff: Option<&str>,
    editor_model: &Option<EditorModel>,
    file_history: &FileHistory,
) -> Result<Vec<Content>, ErrorData> {
    // Check if diff is provided
    if let Some(diff_content) = diff {
        // Validate it's a proper diff
        if !diff_content.contains("---") || !diff_content.contains("+++") {
            return Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                "The 'diff' parameter must be in unified diff format".to_string(),
                None,
            ));
        }

        return apply_diff_bounded(path, diff_content, file_history).await;
    }
    // Check if file exists and is active
    if !path.exists() {
        return Err(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            format!(
                "File '{}' does not exist, you can write a new file with the `write` command",
                path.display()
            ),
            None,
        ));
    }

    // Read content
    let content = path.read_to_string().map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("Failed to read file: {}", e),
            None,
        )
    })?;

    // Check if Editor API is configured and use it as the primary path
    if let Some(ref editor) = editor_model {
        match editor.edit_code(&content, old_str, new_str).await {
            Ok(updated_content) => {
                // Write the updated content directly
                let mut normalized_content =
                    normalize_line_endings_like(Some(&content), &updated_content);

                ensure_trailing_newline(&mut normalized_content, Some(&content));

                path.write(&normalized_content).map_err(|e| {
                    ErrorData::new(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to write file: {}", e),
                        None,
                    )
                })?;

                file_history.record(
                    &path.bound.try_history_key().map_err(file_error)?,
                    content.clone(),
                );
                // Simple success message for Editor API
                return Ok(vec![
                    Content::text(format!("Successfully edited {}", path.display()))
                        .with_audience(vec![Role::Assistant]),
                    Content::text(format!("File {} has been edited", path.display()))
                        .with_audience(vec![Role::User])
                        .with_priority(0.2),
                ]);
            }
            Err(e) => {
                tracing::debug!(
                    "Editor API call failed: {}, falling back to string replacement",
                    e
                );
                // Fall through to traditional path below
            }
        }
    }

    // Traditional string replacement path (original logic)
    // Ensure 'old_str' appears exactly once
    if content.matches(old_str).count() > 1 {
        return Err(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            "'old_str' must appear exactly once in the file, but it appears multiple times"
                .to_string(),
            None,
        ));
    }
    if content.matches(old_str).count() == 0 {
        return Err(ErrorData::new(ErrorCode::INVALID_PARAMS, "'old_str' must appear exactly once in the file, but it does not appear in the file. Make sure the string exactly matches existing file content, including whitespace!".to_string(), None));
    }

    let new_content = content.replace(old_str, new_str);
    // The file's own line endings win over the host's -- see
    // `normalize_line_endings_like`.
    let mut normalized_content = normalize_line_endings_like(Some(&content), &new_content);

    ensure_trailing_newline(&mut normalized_content, Some(&content));

    path.write(&normalized_content).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("Failed to write file: {}", e),
            None,
        )
    })?;

    file_history.record(
        &path.bound.try_history_key().map_err(file_error)?,
        content.clone(),
    );
    // Try to detect the language from the file extension
    let language = lang::get_language_identifier(path);

    // Show a snippet of the changed content with context
    const SNIPPET_LINES: usize = 4;

    // Count newlines before the replacement to find the line number
    let replacement_line = content
        .split(old_str)
        .next()
        .expect("should split on already matched content")
        .matches('\n')
        .count();

    // Calculate start and end lines for the snippet
    let start_line = replacement_line.saturating_sub(SNIPPET_LINES);
    let end_line = replacement_line + SNIPPET_LINES + new_content.matches('\n').count();

    // Get the relevant lines for our snippet
    let lines: Vec<&str> = new_content.lines().collect();
    let snippet = lines
        .iter()
        .skip(start_line)
        .take(end_line - start_line + 1)
        .cloned()
        .collect::<Vec<&str>>()
        .join("\n");

    let output = formatdoc! {r#"
        ```{language}
        {snippet}
        ```
        "#,
        language=language,
        snippet=snippet
    };

    let success_message = formatdoc! {r#"
        The file {} has been edited, and the section now reads:
        {}
        Review the changes above for errors. Undo and edit the file again if necessary!
        "#,
        path.display(),
        output
    };

    Ok(vec![
        Content::text(success_message).with_audience(vec![Role::Assistant]),
        Content::text(output)
            .with_audience(vec![Role::User])
            .with_priority(0.2),
    ])
}

pub(crate) async fn text_editor_insert_bounded(
    path: &EditorPath,
    insert_line_spec: i64,
    new_str: &str,
    file_history: &FileHistory,
) -> Result<Vec<Content>, ErrorData> {
    // Check if file exists
    if !path.exists() {
        return Err(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            format!(
                "File '{}' does not exist, you can write a new file with the `write` command",
                path.display()
            ),
            None,
        ));
    }

    // Read content
    let content = path.read_to_string().map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("Failed to read file: {}", e),
            None,
        )
    })?;

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();

    // Allow insert_line to be negative
    let insert_line = if insert_line_spec < 0 {
        // -1 == end of file, -2 == before the last line, etc.
        (total_lines as i64 + 1 + insert_line_spec) as usize
    } else {
        insert_line_spec as usize
    };

    // Validate insert_line parameter
    if insert_line > total_lines {
        return Err(ErrorData::new(ErrorCode::INVALID_PARAMS, format!(
            "Insert line {} is beyond the end of the file (total lines: {}). Use 0 to insert at the beginning or {} to insert at the end.",
            insert_line, total_lines, total_lines
        ), None));
    }

    // Create new content with inserted text
    let mut new_lines = Vec::new();

    // Add lines before the insertion point
    for (i, line) in lines.iter().enumerate() {
        if i == insert_line {
            // Insert the new text at this position
            new_lines.push(new_str.to_string());
        }
        new_lines.push(line.to_string());
    }

    // If inserting at the end (after all existing lines)
    if insert_line == total_lines {
        new_lines.push(new_str.to_string());
    }

    let new_content = new_lines.join("\n");
    let mut final_content = normalize_line_endings_like(Some(&content), &new_content);
    ensure_trailing_newline(&mut final_content, Some(&content));

    path.write(&final_content).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("Failed to write file: {}", e),
            None,
        )
    })?;

    file_history.record(
        &path.bound.try_history_key().map_err(file_error)?,
        content.clone(),
    );
    // Try to detect the language from the file extension
    let language = lang::get_language_identifier(path);

    // Show a snippet of the inserted content with context
    const SNIPPET_LINES: usize = 4;
    let insertion_line = insert_line + 1; // Convert to 1-indexed for display

    // Calculate start and end lines for the snippet
    let start_line = insertion_line.saturating_sub(SNIPPET_LINES);
    let end_line = std::cmp::min(insertion_line + SNIPPET_LINES, new_lines.len());

    // Get the relevant lines for our snippet with line numbers
    let snippet_lines: Vec<String> = new_lines[start_line.saturating_sub(1)..end_line]
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}: {}", start_line + i, line))
        .collect();

    let snippet = snippet_lines.join("\n");

    let output = formatdoc! {r#"
        ```{language}
        {snippet}
        ```
        "#,
        language=language,
        snippet=snippet
    };

    let success_message = formatdoc! {r#"
        Text has been inserted at line {} in {}. The section now reads:
        {}
        Review the changes above for errors. Undo and edit the file again if necessary!
        "#,
        insertion_line,
        path.display(),
        output
    };

    Ok(vec![
        Content::text(success_message).with_audience(vec![Role::Assistant]),
        Content::text(output)
            .with_audience(vec![Role::User])
            .with_priority(0.2),
    ])
}

pub(crate) async fn text_editor_undo_bounded(
    path: &EditorPath,
    file_history: &FileHistory,
) -> Result<Vec<Content>, ErrorData> {
    if file_history
        .restore_with(
            &path.bound.try_history_key().map_err(file_error)?,
            || path.read_to_string(),
            |previous_content| path.write(previous_content),
        )
        .map_err(file_error)?
    {
        Ok(vec![Content::text("Undid the last edit")])
    } else {
        Err(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            "No edit history available to undo".to_string(),
            None,
        ))
    }
}

#[cfg(test)]
pub(crate) fn save_file_history(
    path: &EditorPath,
    file_history: &FileHistory,
) -> Result<(), ErrorData> {
    file_history.snapshot_bounded(&path.bound).map_err(|e| {
        ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            format!("Failed to read file: {}", e),
            None,
        )
    })
}

#[cfg(test)]
pub async fn apply_diff(
    path: &Path,
    diff: &str,
    history: &FileHistory,
) -> Result<Vec<Content>, ErrorData> {
    apply_diff_bounded(&crate::file_access::test_editor_path(path), diff, history).await
}
#[cfg(test)]
pub async fn text_editor_write(path: &Path, text: &str) -> Result<Vec<Content>, ErrorData> {
    text_editor_write_bounded(&crate::file_access::test_editor_path(path), text).await
}
#[cfg(test)]
pub async fn text_editor_replace(
    path: &Path,
    old: &str,
    new: &str,
    diff: Option<&str>,
    editor: &Option<EditorModel>,
    history: &FileHistory,
) -> Result<Vec<Content>, ErrorData> {
    text_editor_replace_bounded(
        &crate::file_access::test_editor_path(path),
        old,
        new,
        diff,
        editor,
        history,
    )
    .await
}
#[cfg(test)]
pub async fn text_editor_undo(
    path: &Path,
    history: &FileHistory,
) -> Result<Vec<Content>, ErrorData> {
    text_editor_undo_bounded(&crate::file_access::test_editor_path(path), history).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contents whose line structure is easy to get wrong, each with the
    /// number of lines `str::lines()` reports.
    fn tricky_contents() -> Vec<(&'static str, &'static str, usize)> {
        vec![
            ("empty", "", 0),
            ("one line, no terminator", "alpha", 1),
            ("one line, terminated", "alpha\n", 1),
            ("trailing blank line", "alpha\n\n", 2),
            ("leading blank line", "\nalpha\n", 2),
            ("crlf", "alpha\r\nbravo\r\n", 2),
            ("mixed endings", "alpha\r\nbravo\ncharlie", 3),
            ("no final newline", "alpha\nbravo", 2),
            ("only newlines", "\n\n\n", 3),
        ]
    }

    /// The whole-file fast path returns `content` untouched instead of
    /// rebuilding it. That is only sound if the slice path would have produced
    /// the identical string, which rests on two properties of
    /// `split_inclusive('\n')`: it yields one piece per `lines()` line, and its
    /// pieces concatenate back to the original.
    ///
    /// Both are asserted here rather than assumed, because a disagreement would
    /// not show up as a panic. It would silently hand the model a file with a
    /// line missing off one end, on the one code path that carries whole files.
    #[test]
    fn the_two_paths_agree_on_a_whole_file() {
        for (name, content, total_lines) in tricky_contents() {
            assert_eq!(
                content.split_inclusive('\n').count(),
                total_lines,
                "{name}: split_inclusive disagrees with lines() on how many lines there are"
            );
            assert_eq!(
                content.split_inclusive('\n').collect::<String>(),
                content,
                "{name}: the pieces do not concatenate back to the original"
            );

            // The fast path, taken when `calculate_view_range` answered
            // `(0, total_lines)`.
            assert_eq!(
                model_file_view(content.to_string(), 0, total_lines, total_lines),
                content,
                "{name}: the whole-file path changed the file"
            );

            // The slice path over that same whole range. `total_lines + 1` is
            // what forces it: there is no other way to reach the slice for a
            // range that covers everything, and the point of the test is that
            // reaching it changes nothing.
            assert_eq!(
                model_file_view(content.to_string(), 0, total_lines, total_lines + 1),
                content,
                "{name}: slicing the whole range did not reproduce the file"
            );
        }
    }

    /// A range gets the bytes of those lines, with their own terminators, and
    /// nothing else.
    #[test]
    fn a_range_is_exactly_the_bytes_of_those_lines() {
        let content = "Line 1\nLine 2\nLine 3\nLine 4\nLine 5\n";

        assert_eq!(
            model_file_view(content.to_string(), 2, 4, 5),
            "Line 3\nLine 4\n",
            "lines 3-4 should arrive with their terminators and no neighbours"
        );
        assert_eq!(
            model_file_view(content.to_string(), 0, 1, 5),
            "Line 1\n",
            "the first line alone"
        );
        assert_eq!(
            model_file_view(content.to_string(), 4, 5, 5),
            "Line 5\n",
            "the last line alone"
        );
    }

    /// `str::lines()` strips the `\r` of a `\r\n` pair, so a slice rebuilt with
    /// `lines().join("\n")` would hand the model LF text for a CRLF file. The
    /// model's next move after a view is usually `str_replace`, whose `old_str`
    /// has to match the file byte for byte, so that rewrite would produce an
    /// edit that cannot apply and a failure with no visible cause.
    #[test]
    fn a_range_of_a_crlf_file_keeps_its_carriage_returns() {
        let content = "alpha\r\nbravo\r\ncharlie\r\n";

        let sliced = model_file_view(content.to_string(), 1, 2, 3);
        assert_eq!(sliced, "bravo\r\n");
        assert!(
            sliced.contains('\r'),
            "the carriage return was dropped, so this no longer matches the file on disk"
        );

        // What the discarded implementation would have produced.
        let rejoined = content.lines().collect::<Vec<_>>()[1..2].join("\n");
        assert_ne!(
            sliced, rejoined,
            "if these agree the test has stopped distinguishing the two implementations"
        );
    }

    /// A file shorter than the range it is asked for, and a range that is
    /// empty. `calculate_view_range` clamps `end_idx` to `total_lines` before
    /// this runs, but the arithmetic must not panic if it ever stops.
    #[test]
    fn an_over_long_or_empty_range_does_not_panic() {
        let content = "alpha\nbravo\n";

        assert_eq!(model_file_view(content.to_string(), 1, 99, 2), "bravo\n");
        assert_eq!(model_file_view(content.to_string(), 5, 9, 2), "");
        assert_eq!(model_file_view(content.to_string(), 3, 1, 2), "");
        assert_eq!(model_file_view(String::new(), 0, 4, 0), "");
    }
}
