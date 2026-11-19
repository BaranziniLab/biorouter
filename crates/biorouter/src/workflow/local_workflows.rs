use anyhow::{anyhow, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::paths::Paths;
use crate::workflow::read_workflow_file_content::{read_workflow_file, WorkflowFile};
use crate::workflow::Workflow;
use crate::workflow::WORKFLOW_FILE_EXTENSIONS;

const BIOROUTER_WORKFLOW_PATH_ENV_VAR: &str = "BIOROUTER_WORKFLOW_PATH";

/// Opts the working directory *root* back into the workflow listing.
///
/// Listing used to enumerate it unconditionally, which meant opening and
/// parsing every top-level `.json`/`.yaml` in whatever directory a session
/// happened to point at — another tool's config, a lockfile, a large data
/// export. Setting this to `1`/`true`/`yes`/`on` restores that behaviour for
/// anyone who keeps workflows loose in a project root.
///
/// This is deliberately *not* the same as putting `.` on
/// [`BIOROUTER_WORKFLOW_PATH_ENV_VAR`]: an entry on the workflow path is a
/// declared workflow library, so anything in it that fails to load is reported
/// as a broken workflow. The working directory root stays
/// [`ScanOrigin::WorkingDirectory`] however it is reached, so unrelated files
/// in it are still skipped quietly.
const BIOROUTER_WORKFLOW_SCAN_WORKING_DIR_ENV_VAR: &str = "BIOROUTER_WORKFLOW_SCAN_WORKING_DIR";

/// Where a workflow library lives.
///
/// `is_global` picks the user-wide library, which is always resolvable, or this
/// project's, which needs a working directory that still exists. When there is
/// none, the project answer is the empty path — a value that names nothing,
/// never `exists()`, and cannot be written to, so a caller fails instead of
/// operating on some other directory.
///
/// **Search roots never come from here.** They come from
/// [`project_workflow_library_dir`], which reports the same condition as `None`
/// and is then left out of the search entirely; see the note there for why a
/// stand-in path is not an acceptable substitute.
pub fn get_workflow_library_dir(is_global: bool) -> PathBuf {
    if is_global {
        global_workflow_library_dir()
    } else {
        project_workflow_library_dir(working_directory_snapshot().as_deref()).unwrap_or_default()
    }
}

/// The user-wide workflow library, `~/.config/biorouter/workflows`. It does not
/// depend on where the process is standing, so it is always available.
fn global_workflow_library_dir() -> PathBuf {
    Paths::config_dir().join("workflows")
}

/// The single absolute snapshot of the process working directory that every
/// project-relative search root is derived from.
///
/// Taken **once** per resolution. The process working directory is mutable and
/// this codebase moves it itself — `biorouter-cli`'s session builder calls
/// `std::env::set_current_dir` when a resumed session goes back to its original
/// directory — so re-reading it per root, or deferring the read to whenever a
/// root is finally used, would let one search list describe two directories.
/// One snapshot means the whole list describes one project or none.
///
/// Split from [`snapshot_working_directory`] only so the failure branch is
/// testable without deleting a directory out from under the test binary.
fn working_directory_snapshot() -> Option<PathBuf> {
    snapshot_working_directory(env::current_dir())
}

/// Turn the working-directory lookup into "this project" or "no project",
/// saying so in the log.
///
/// Resolving it used to `unwrap`. A session's working directory is user-chosen
/// and can vanish while the process is still running: a scratch directory
/// cleaned up, a volume unmounted, a `git worktree remove`. In the CLI that
/// panic ended one command; in `biorouterd` it took down a long-lived daemon
/// serving every other session, so the user lost the backend for reasons
/// unrelated to whatever they were doing. Listing workflows is not worth that.
fn snapshot_working_directory(working_dir: std::io::Result<PathBuf>) -> Option<PathBuf> {
    match working_dir {
        Ok(dir) => Some(dir),
        Err(err) => {
            // Listing is user-triggered (opening the workflows view, a CLI
            // lookup), not polled, so warning per call reports an ongoing
            // condition rather than flooding the log.
            tracing::warn!(
                "Cannot resolve the working directory ({}); skipping the project \
                 workflow library. Global workflows and BIOROUTER_WORKFLOW_PATH \
                 entries are unaffected.",
                err
            );
            None
        }
    }
}

/// The project workflow library — `<working dir>/.biorouter/workflows` — or
/// `None` when there is no working directory to hang it on.
///
/// `None` is the whole point. The obvious degradation is to keep the path
/// *relative* to the directory that just failed, on the reasoning that a
/// directory which is gone has no library and a relative path pointing into it
/// resolves to nothing. It does not: a relative root is not tied to the
/// directory whose lookup failed, it is tied to whatever the working directory
/// is when the root is eventually consumed — and something has to consume it,
/// because roots are collected first and scanned afterwards. Move the process
/// in between and `.biorouter/workflows` binds to a *different* project, whose
/// workflows are then loaded and shown as this session's.
///
/// That is worse than the panic this replaced. A panic is loud, immediate and
/// local; quietly reading another project's directory is none of those. So an
/// unavailable project library is omitted from the search entirely, and the
/// global library and every `BIOROUTER_WORKFLOW_PATH` entry keep working.
fn project_workflow_library_dir(working_dir: Option<&Path>) -> Option<PathBuf> {
    working_dir.map(|dir| dir.join(".biorouter").join("workflows"))
}

/// Bind a search root to the snapshotted working directory, or drop it.
///
/// Anything still relative when it reaches the search list is a deferred
/// lookup, not a decided one — see [`project_workflow_library_dir`]. A relative
/// `BIOROUTER_WORKFLOW_PATH` entry is a real declaration and is anchored to the
/// snapshot; with no working directory it cannot mean anything at all, so it is
/// dropped rather than left to attach itself to a later one.
fn anchor_to_working_dir(root: PathBuf, working_dir: Option<&Path>) -> Option<PathBuf> {
    if root.is_absolute() {
        return Some(root);
    }
    match working_dir {
        Some(cwd) => Some(cwd.join(root)),
        None => {
            tracing::debug!(
                "Dropping the relative workflow directory {}: there is no working \
                 directory to resolve it against.",
                root.display()
            );
            None
        }
    }
}

/// Why a directory is being scanned — which is what decides how loudly a file
/// in it that fails to load should be reported.
///
/// Variant order is load-bearing: `WorkflowLibrary` sorts first so that a
/// directory reached both ways keeps the stricter origin through `dedup_by`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ScanOrigin {
    /// A directory that exists to hold workflows: `~/.config/biorouter/workflows`,
    /// `<project>/.biorouter/workflows`, or an entry of `BIOROUTER_WORKFLOW_PATH`.
    /// Intent is established by *location*, so any `.yaml`/`.json` in it that
    /// does not load is a broken workflow and worth an error.
    WorkflowLibrary,
    /// The process working directory root. It belongs to the user, not to
    /// Biorouter: nearly every `.yaml`/`.json` in it — package manifests,
    /// lockfiles, CI config — has nothing to do with workflows, so intent has
    /// to be established from the *content* instead.
    ///
    /// Listing no longer enumerates it by default (that is what made an
    /// unrelated tool's config get parsed at all); it is reached by a named
    /// lookup, which opens only the file the caller named, or by opting in with
    /// [`BIOROUTER_WORKFLOW_SCAN_WORKING_DIR_ENV_VAR`]. Opting in says "look
    /// here too", not "everything here is mine", so the content test still
    /// applies.
    WorkingDirectory,
}

/// Whether an on/off environment variable is set to something affirmative.
fn env_flag_is_on(value: Option<&str>) -> bool {
    matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

/// The directories *enumerated* when listing every available workflow.
///
/// Only directories that exist to hold workflows, unless the user has opted the
/// working directory root back in. Everything here gets `read_dir`'d and every
/// `.yaml`/`.json` in it opened, so membership is a licence to read the user's
/// files and is granted by intent — a dedicated location, an entry on
/// `BIOROUTER_WORKFLOW_PATH`, or an explicit opt-in — never by proximity.
fn local_workflow_scan_dirs() -> Vec<(PathBuf, ScanOrigin)> {
    let working_dir = working_directory_snapshot();
    collect_workflow_dirs(
        env::var(BIOROUTER_WORKFLOW_PATH_ENV_VAR).ok().as_deref(),
        global_workflow_library_dir(),
        working_dir.as_deref(),
        env_flag_is_on(
            env::var(BIOROUTER_WORKFLOW_SCAN_WORKING_DIR_ENV_VAR)
                .ok()
                .as_deref(),
        ),
    )
}

/// The directories a listing scan enumerates, as plain paths.
///
/// The staleness fingerprint in [`crate::workflow::service`] needs the real root
/// set, and "real" is load-bearing: a hand-rolled guess at the roots (the global
/// library plus `<cwd>/.biorouter/workflows`) omits every
/// `BIOROUTER_WORKFLOW_PATH` entry and the opted-in working directory, so a
/// workflow added in one of those would never invalidate the cache and the id
/// map would answer from a snapshot taken before it existed. A cache keyed on
/// the wrong roots is worse than no cache, because it looks like it is working.
pub fn workflow_scan_roots() -> Vec<PathBuf> {
    local_workflow_scan_dirs()
        .into_iter()
        .map(|(dir, _origin)| dir)
        .collect()
}

/// The directories searched when the caller *names* a workflow.
///
/// This always includes the working directory, and that is not the behaviour
/// the listing scan gives up. A named lookup opens exactly one path per
/// directory — `<dir>/<name>.yaml` and `<dir>/<name>.json` — so it never reads
/// a file the caller did not ask for by name, and `biorouter run --recipe
/// my-workflow` keeps finding `./my-workflow.yaml`. The CLI's `/workflow`
/// command still saves there by default, so this is the compatibility that
/// matters.
fn local_workflow_lookup_dirs() -> Vec<PathBuf> {
    let working_dir = working_directory_snapshot();
    collect_workflow_dirs(
        env::var(BIOROUTER_WORKFLOW_PATH_ENV_VAR).ok().as_deref(),
        global_workflow_library_dir(),
        working_dir.as_deref(),
        true,
    )
    .into_iter()
    .map(|(dir, _origin)| dir)
    .collect()
}

/// The env-free core of [`local_workflow_scan_dirs`] and
/// [`local_workflow_lookup_dirs`], so the origin tagging and the
/// working-directory decision can be tested without mutating process state.
///
/// `working_dir` is the one snapshot from [`working_directory_snapshot`], and
/// it is the *only* way the working directory enters: the project library and
/// the working-directory root are both derived from it here, and a relative
/// `BIOROUTER_WORKFLOW_PATH` entry is anchored to it. `None` means there is no
/// working directory, and every root that would have needed one is simply
/// absent — so a returned list never contains a root that will re-resolve
/// against a directory the process moves to afterwards.
fn collect_workflow_dirs(
    workflow_path_env: Option<&str>,
    global_library: PathBuf,
    working_dir: Option<&Path>,
    include_working_directory: bool,
) -> Vec<(PathBuf, ScanOrigin)> {
    let mut local_dirs = Vec::new();
    if include_working_directory {
        if let Some(working_dir) = working_dir {
            local_dirs.push((working_dir.to_path_buf(), ScanOrigin::WorkingDirectory));
        }
    }

    if let Some(workflow_path_env) = workflow_path_env {
        let path_separator = if cfg!(windows) { ';' } else { ':' };
        local_dirs.extend(
            workflow_path_env
                .split(path_separator)
                // An empty entry (a stray `::` or a trailing separator) is not
                // a shorthand for the working directory here: honouring it as
                // one would promote the user's own directory to a workflow
                // library, which is exactly the licence-to-read this scan
                // withholds.
                .filter(|dir| !dir.is_empty())
                .filter_map(|dir| anchor_to_working_dir(PathBuf::from(dir), working_dir))
                .map(|dir| (dir, ScanOrigin::WorkflowLibrary)),
        );
    }
    local_dirs.push((global_library, ScanOrigin::WorkflowLibrary));
    if let Some(project_library) = project_workflow_library_dir(working_dir) {
        local_dirs.push((project_library, ScanOrigin::WorkflowLibrary));
    }

    let mut dirs: Vec<(PathBuf, ScanOrigin)> = local_dirs
        .into_iter()
        .map(|(dir, origin)| (dir.canonicalize().unwrap_or(dir), origin))
        .collect();
    dirs.sort();
    // Keep one entry per path. Sorting put `WorkflowLibrary` first within a
    // path, so a workflow library that also happens to be the working directory
    // is still treated as a workflow library.
    dirs.dedup_by(|a, b| a.0 == b.0);
    dirs
}

pub fn load_local_workflow_file(workflow_name: &str) -> Result<WorkflowFile> {
    if WORKFLOW_FILE_EXTENSIONS
        .iter()
        .any(|ext| workflow_name.ends_with(&format!(".{}", ext)))
    {
        let path = PathBuf::from(workflow_name);
        return read_workflow_file(path);
    }

    if is_file_path(workflow_name) || is_file_name(workflow_name) {
        return Err(anyhow!(
            "Workflow file {} is not a json or yaml file",
            workflow_name
        ));
    }

    let search_dirs = local_workflow_lookup_dirs();
    for dir in &search_dirs {
        if let Ok(result) = load_workflow_file_from_dir(dir, workflow_name) {
            return Ok(result);
        }
    }

    let search_dirs_str = search_dirs
        .iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(":");
    Err(anyhow!(
        "ℹ️  Failed to retrieve {}.yaml or {}.json in {}",
        workflow_name,
        workflow_name,
        search_dirs_str
    ))
}

pub fn list_local_workflows() -> Result<Vec<(PathBuf, Workflow)>> {
    let mut workflows = Vec::new();
    for (dir, origin) in local_workflow_scan_dirs() {
        if let Ok(dir_workflows) = scan_directory_for_workflows(&dir, origin) {
            workflows.extend(dir_workflows);
        }
    }

    Ok(workflows)
}

fn is_file_path(workflow_name: &str) -> bool {
    workflow_name.contains('/')
        || workflow_name.contains('\\')
        || workflow_name.starts_with('~')
        || workflow_name.starts_with('.')
}

fn is_file_name(workflow_name: &str) -> bool {
    Path::new(workflow_name).extension().is_some()
}

fn load_workflow_file_from_dir(dir: &Path, workflow_name: &str) -> Result<WorkflowFile> {
    for ext in WORKFLOW_FILE_EXTENSIONS {
        let workflow_path = dir.join(format!("{}.{}", workflow_name, ext));
        if let Ok(result) = read_workflow_file(workflow_path) {
            return Ok(result);
        }
    }
    Err(anyhow!(format!(
        "No {}.yaml or {}.json workflow file found in directory: {}",
        workflow_name,
        workflow_name,
        dir.display()
    )))
}

fn scan_directory_for_workflows(
    dir: &Path,
    origin: ScanOrigin,
) -> Result<Vec<(PathBuf, Workflow)>> {
    let mut workflows = Vec::new();

    if !dir.exists() || !dir.is_dir() {
        return Ok(workflows);
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(extension) = path.extension() {
                if WORKFLOW_FILE_EXTENSIONS.contains(&extension.to_string_lossy().as_ref()) {
                    match Workflow::from_file_path(&path) {
                        Ok(workflow) => workflows.push((path.clone(), workflow)),
                        Err(e) => report_workflow_load_failure(&path, &e, origin),
                    }
                }
            }
        }
    }

    Ok(workflows)
}

/// Keys that only a workflow document plausibly carries, used to establish
/// intent when location cannot — i.e. in the working directory, which is full
/// of other people's `.yaml`/`.json`.
///
/// Deliberately excluded are `version`, `title` and `description`: every
/// package manifest, lockfile and CI config in existence carries some
/// combination of them, and `ui/desktop/package.json` in this very repo carries
/// `version` *and* `description`. Requiring two such generic keys does not make
/// them evidence — it only makes the false positive need one more line of JSON.
const WORKFLOW_INTENT_KEYS: &[&str] = &["instructions", "prompt", "activities", "sub_workflows"];

/// Whether a document in an arbitrary directory declares itself a workflow.
///
/// The `workflow:` envelope is a declaration on its own, *whatever it holds* —
/// `Workflow::from_content` branches on the mere presence of that key, so
/// `workflow: []` is a workflow someone wrote and broke, not an unrelated file.
/// Demanding a mapping under it would hide exactly the documents this triage
/// exists to surface. Otherwise at least one distinctive workflow key must be
/// present.
fn declares_workflow_intent(value: &serde_yaml::Value) -> bool {
    let serde_yaml::Value::Mapping(map) = value else {
        return false;
    };
    if map.contains_key(serde_yaml::Value::from("workflow")) {
        return true;
    }
    WORKFLOW_INTENT_KEYS
        .iter()
        .any(|key| map.contains_key(serde_yaml::Value::from(*key)))
}

/// Log a file that did not load as a workflow at the level it deserves.
///
/// A file that was never a workflow is not a failed workflow: it is skipped at
/// `debug`. `error` is reserved for a document someone meant to be a workflow
/// and that still does not load — the case someone actually needs to see.
/// Intent comes from the directory when the directory is ours, and from the
/// document's own keys when it is not.
fn report_workflow_load_failure(path: &Path, error: &anyhow::Error, origin: ScanOrigin) {
    let is_broken_workflow = match origin {
        ScanOrigin::WorkflowLibrary => true,
        // In a directory that is not ours, a file we cannot read or cannot
        // parse offers no evidence it was ever meant to be a workflow — and a
        // malformed document nobody claimed is somebody else's malformed
        // document. Promoting it to error is how the loader used to shout
        // about unrelated files it had no business opening.
        ScanOrigin::WorkingDirectory => fs::read_to_string(path).is_ok_and(|content| {
            serde_yaml::from_str::<serde_yaml::Value>(&content)
                .is_ok_and(|value| declares_workflow_intent(&value))
        }),
    };

    if is_broken_workflow {
        tracing::error!(
            "Failed to load workflow from file {}: {}",
            path.display(),
            error
        );
    } else {
        tracing::debug!("Skipping non-workflow file {}: {}", path.display(), error);
    }
}

fn generate_workflow_filename(title: &str, workflow_library_dir: &Path) -> PathBuf {
    let base_name = title
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-')
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join("-");

    let filename = if base_name.is_empty() {
        "untitled-workflow".to_string()
    } else {
        base_name
    };

    let mut candidate = workflow_library_dir.join(format!("{}.yaml", filename));
    if !candidate.exists() {
        return candidate;
    }

    let mut counter = 1;
    loop {
        candidate = workflow_library_dir.join(format!("{}-{}.yaml", filename, counter));
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

pub fn save_workflow_to_file(
    workflow: Workflow,
    file_path: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    let workflow_library_dir = get_workflow_library_dir(true);

    let file_path_value = match file_path {
        Some(path) => path,
        None => {
            fs::create_dir_all(&workflow_library_dir)?;
            generate_workflow_filename(&workflow.title, &workflow_library_dir.canonicalize()?)
        }
    };

    if let Some(parent) = file_path_value.parent() {
        fs::create_dir_all(parent)?;
    }

    let yaml_content = workflow.to_yaml()?;
    fs::write(&file_path_value, yaml_content)?;
    Ok(file_path_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::fmt::MakeWriter;

    /// Collects formatted tracing output so a test can assert the *level* a
    /// message was emitted at, not merely that it was emitted.
    #[derive(Clone, Default)]
    struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    impl CapturedLogs {
        fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).to_string()
        }
    }

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CapturedLogs {
        type Writer = CapturedLogs;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn scan_with_logs(dir: &Path, origin: ScanOrigin) -> (Vec<(PathBuf, Workflow)>, String) {
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .finish();
        let workflows = tracing::subscriber::with_default(subscriber, || {
            scan_directory_for_workflows(dir, origin).unwrap()
        });
        (workflows, logs.text())
    }

    fn write_file(dir: &Path, name: &str, contents: &str) {
        fs::write(dir.join(name), contents).unwrap();
    }

    /// Content that not even `serde_yaml::Value` will accept, so the tests
    /// using it provably exercise the parse-failure branch of the triage
    /// rather than the "parsed fine, no workflow keys" one.
    const UNPARSEABLE: &str = "{{ mustache }}: [unclosed\n";

    #[test]
    fn the_unparseable_fixture_really_is_unparseable() {
        assert!(serde_yaml::from_str::<serde_yaml::Value>(UNPARSEABLE).is_err());
    }

    #[test]
    fn unrelated_json_in_a_scanned_directory_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // Shaped like ~/.claude.json: another tool's config that happens to
        // live in a directory the workflow loader scans.
        write_file(
            dir.path(),
            "unrelated.json",
            r#"{"numStartups": 12, "installMethod": "brew", "projects": {}}"#,
        );

        let (workflows, logs) = scan_with_logs(dir.path(), ScanOrigin::WorkingDirectory);

        assert!(workflows.is_empty());
        assert!(
            !logs.contains("ERROR"),
            "a file that was never a workflow is not a failed workflow; logs were:\n{logs}"
        );
        assert!(
            logs.contains("DEBUG") && logs.contains("unrelated.json"),
            "the skip should still be recorded at debug; logs were:\n{logs}"
        );
    }

    #[test]
    fn a_package_manifest_in_the_working_directory_is_not_a_broken_workflow() {
        let dir = tempfile::tempdir().unwrap();
        // An ordinary npm manifest — `ui/desktop/package.json` is the one in
        // this repo. It carries `version` and `description` like almost every
        // package manifest on earth, and nothing that is workflow-specific.
        write_file(
            dir.path(),
            "package.json",
            r#"{"name": "biorouter", "version": "1.88.5", "description": "an app", "scripts": {}}"#,
        );

        let (workflows, logs) = scan_with_logs(dir.path(), ScanOrigin::WorkingDirectory);

        assert!(workflows.is_empty());
        assert!(
            !logs.contains("ERROR"),
            "generic keys shared by every config file are not workflow intent; logs were:\n{logs}"
        );
        assert!(
            logs.contains("DEBUG") && logs.contains("package.json"),
            "the skip should still be recorded at debug; logs were:\n{logs}"
        );
    }

    #[test]
    fn an_unrelated_file_in_a_workflow_directory_is_a_broken_workflow() {
        let dir = tempfile::tempdir().unwrap();
        // The same manifest, but sitting in ~/.config/biorouter/workflows or a
        // BIOROUTER_WORKFLOW_PATH entry. Nothing but workflows belongs there,
        // so location settles intent and content never gets a vote.
        write_file(
            dir.path(),
            "package.json",
            r#"{"name": "biorouter", "version": "1.88.5", "description": "an app", "scripts": {}}"#,
        );

        let (workflows, logs) = scan_with_logs(dir.path(), ScanOrigin::WorkflowLibrary);

        assert!(workflows.is_empty());
        assert!(
            logs.contains("ERROR") && logs.contains("package.json"),
            "a file that fails to load in a workflow directory is a broken workflow; logs were:\n{logs}"
        );
    }

    /// A working directory that is *also* a declared workflow library, so the
    /// same path arrives twice with two different origins.
    #[test]
    fn a_workflow_library_that_is_also_the_working_directory_keeps_the_stricter_origin() {
        let global = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();
        let path_env = working_dir.path().display().to_string();

        let dirs = collect_workflow_dirs(
            Some(&path_env),
            global.path().to_path_buf(),
            Some(working_dir.path()),
            true,
        );

        let canonical = working_dir.path().canonicalize().unwrap();
        let origins: Vec<ScanOrigin> = dirs
            .iter()
            .filter(|(dir, _)| *dir == canonical)
            .map(|(_, origin)| *origin)
            .collect();
        assert_eq!(
            origins,
            vec![ScanOrigin::WorkflowLibrary],
            "dedup must keep one entry, and it must be the stricter one; dirs were {dirs:?}"
        );
    }

    #[test]
    fn no_workflow_library_is_ever_treated_as_an_arbitrary_directory() {
        let global = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();
        let project = working_dir.path().join(".biorouter").join("workflows");
        fs::create_dir_all(&project).unwrap();
        let on_path = tempfile::tempdir().unwrap();
        let path_env = on_path.path().display().to_string();

        let dirs = collect_workflow_dirs(
            Some(&path_env),
            global.path().to_path_buf(),
            Some(working_dir.path()),
            true,
        );

        let ambient: Vec<&PathBuf> = dirs
            .iter()
            .filter(|(_, origin)| *origin == ScanOrigin::WorkingDirectory)
            .map(|(dir, _)| dir)
            .collect();
        assert_eq!(ambient, vec![&working_dir.path().canonicalize().unwrap()]);
        for dir in [global.path(), project.as_path(), on_path.path()] {
            let canonical = dir.canonicalize().unwrap();
            assert!(
                dirs.contains(&(canonical.clone(), ScanOrigin::WorkflowLibrary)),
                "{} should be a workflow library; dirs were {dirs:?}",
                canonical.display()
            );
        }
    }

    #[test]
    fn listing_does_not_enumerate_the_working_directory_by_default() {
        let global = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();

        let dirs = collect_workflow_dirs(
            None,
            global.path().to_path_buf(),
            Some(working_dir.path()),
            false,
        );

        let canonical = working_dir.path().canonicalize().unwrap();
        assert!(
            !dirs.iter().any(|(dir, _)| *dir == canonical),
            "the working directory root is the user's, not a workflow library; \
             listing must not open every .json/.yaml in it. dirs were {dirs:?}"
        );
    }

    /// The guard on the real listing entry point: the previous test proves the
    /// core can leave the working directory out, this one proves the caller
    /// asks it to. No environment is set here, which is exactly the point —
    /// unset must mean "off".
    #[test]
    fn the_default_listing_scan_touches_only_workflow_libraries() {
        let dirs = local_workflow_scan_dirs();

        assert!(
            dirs.iter()
                .all(|(_, origin)| *origin == ScanOrigin::WorkflowLibrary),
            "with no opt-in, every directory listing reads must be one that \
             exists to hold workflows; dirs were {dirs:?}"
        );
    }

    /// The compatibility this fix keeps. `/workflow` in the CLI still saves to
    /// `./workflow.yaml` by default, and a named lookup opens exactly the file
    /// it was asked for, so narrowing the *scan* must not stop `--recipe
    /// my-workflow` from finding a workflow sitting in the working directory.
    #[test]
    fn a_named_lookup_still_searches_the_working_directory() {
        let cwd = env::current_dir().unwrap().canonicalize().unwrap();

        assert!(
            local_workflow_lookup_dirs().contains(&cwd),
            "a named lookup reads only <dir>/<name>.yaml, so the working \
             directory costs nothing there and is where the CLI saves; \
             dirs were {:?}",
            local_workflow_lookup_dirs()
        );
    }

    #[test]
    fn the_working_directory_can_be_scanned_by_explicit_opt_in() {
        let global = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();

        let dirs = collect_workflow_dirs(
            None,
            global.path().to_path_buf(),
            Some(working_dir.path()),
            true,
        );

        // Opting in must not also promote the directory to a workflow library:
        // that would resurrect the error log about everyone else's files.
        let canonical = working_dir.path().canonicalize().unwrap();
        assert!(
            dirs.contains(&(canonical, ScanOrigin::WorkingDirectory)),
            "the opt-in restores the scan while keeping the lenient origin; \
             dirs were {dirs:?}"
        );
    }

    #[test]
    fn only_affirmative_values_turn_the_working_directory_scan_on() {
        for on in ["1", "true", "TRUE", "yes", "on", " true "] {
            assert!(env_flag_is_on(Some(on)), "{on:?} should opt in");
        }
        // `BIOROUTER_WORKFLOW_SCAN_WORKING_DIR=0` in a shell profile must mean
        // off, not "set, therefore on".
        for off in ["0", "false", "no", "off", "", "  "] {
            assert!(!env_flag_is_on(Some(off)), "{off:?} should not opt in");
        }
        assert!(!env_flag_is_on(None));
    }

    #[test]
    fn a_malformed_workflow_is_still_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // Carries workflow keys but is missing the required `title`.
        write_file(
            dir.path(),
            "broken.yaml",
            "description: does something\nprompt: do the thing\n",
        );

        let (workflows, logs) = scan_with_logs(dir.path(), ScanOrigin::WorkingDirectory);

        assert!(workflows.is_empty());
        assert!(
            logs.contains("ERROR") && logs.contains("broken.yaml"),
            "a file that looks like a workflow but does not load must be an error; logs were:\n{logs}"
        );
    }

    #[test]
    fn a_malformed_workflow_envelope_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // `Workflow::from_content` branches on the presence of the `workflow`
        // key alone and then fails to deserialise the sequence under it. That
        // is a workflow someone wrote and broke, whatever its value's type.
        write_file(dir.path(), "enveloped.yaml", "workflow: []\n");

        let (workflows, logs) = scan_with_logs(dir.path(), ScanOrigin::WorkingDirectory);

        assert!(workflows.is_empty());
        assert!(
            logs.contains("ERROR") && logs.contains("enveloped.yaml"),
            "a `workflow:` envelope is a declaration of intent whatever it holds; logs were:\n{logs}"
        );
    }

    #[test]
    fn unparseable_yaml_in_the_working_directory_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // Someone else's broken file, or a template that is not YAML at all.
        // Nothing here says "workflow", so nothing here is our failure.
        write_file(dir.path(), "not-yaml.yaml", UNPARSEABLE);

        let (workflows, logs) = scan_with_logs(dir.path(), ScanOrigin::WorkingDirectory);

        assert!(workflows.is_empty());
        assert!(
            !logs.contains("ERROR"),
            "a malformed file in someone else's directory is someone else's problem; logs were:\n{logs}"
        );
        assert!(
            logs.contains("DEBUG") && logs.contains("not-yaml.yaml"),
            "the skip should still be recorded at debug; logs were:\n{logs}"
        );
    }

    #[test]
    fn unparseable_yaml_in_a_workflow_directory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        write_file(dir.path(), "not-yaml.yaml", UNPARSEABLE);

        let (workflows, logs) = scan_with_logs(dir.path(), ScanOrigin::WorkflowLibrary);

        assert!(workflows.is_empty());
        assert!(
            logs.contains("ERROR") && logs.contains("not-yaml.yaml"),
            "an unparseable file in a workflow directory is a broken workflow; logs were:\n{logs}"
        );
    }

    #[test]
    fn a_readable_working_directory_still_gives_the_project_library() {
        let cwd = PathBuf::from("/some/project");

        assert_eq!(
            project_workflow_library_dir(Some(&cwd)),
            Some(cwd.join(".biorouter").join("workflows"))
        );
    }

    /// A working directory that cannot be resolved yields *no* project library
    /// — not a stand-in path — and says so once, at a level someone reading the
    /// daemon log will see.
    ///
    /// This test used to require the opposite: that the fallback be the
    /// **relative** path `.biorouter/workflows`, on the reasoning that a
    /// directory which is gone has no library and a relative path pointing into
    /// it resolves to nothing. The reasoning does not hold. A relative root is
    /// not tied to the directory whose lookup failed; it is tied to whatever
    /// the working directory is when the root is finally used — and roots are
    /// collected first and scanned afterwards, so something always uses it
    /// later. `biorouter-cli`'s session builder calls `set_current_dir` itself,
    /// and `biorouterd` serves other sessions from the same process, so "later"
    /// can easily be a different project — whose workflows would then be loaded
    /// and shown as this session's. The old expectation asked for exactly the
    /// silent cross-project read it was written to forbid.
    #[test]
    fn an_unreadable_working_directory_yields_no_project_library_and_a_warning() {
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .finish();

        let snapshot = tracing::subscriber::with_default(subscriber, || {
            snapshot_working_directory(Err(std::io::Error::from(std::io::ErrorKind::NotFound)))
        });

        assert_eq!(
            snapshot, None,
            "no working directory means no project, not a placeholder project"
        );
        assert_eq!(
            project_workflow_library_dir(snapshot.as_deref()),
            None,
            "an absent project library must be absent, so callers leave the \
             root out of the search instead of resolving it against whatever \
             directory the process is standing in when they get to it"
        );
        let logs = logs.text();
        assert!(
            logs.contains("WARN") && logs.contains("working directory"),
            "a working directory that cannot be resolved is worth saying out \
             loud; logs were:\n{logs}"
        );
    }

    /// The public accessor is the other way this used to hand out a path bound
    /// to nothing in particular. Whatever it returns for an absent project, it
    /// must not be a relative path that a `join` or a `read_dir` would later
    /// resolve against an unrelated directory.
    #[test]
    fn the_library_accessor_never_returns_a_relative_project_path() {
        // The one case that matters is covered end-to-end, with a real deleted
        // working directory, in
        // `a_lost_working_directory_never_binds_a_root_to_another_project`;
        // here we can only assert the readable case, which must be absolute
        // because the snapshot is.
        let dir = get_workflow_library_dir(false);
        assert!(
            dir.is_absolute(),
            "a project library derived from the working-directory snapshot is \
             absolute by construction; got {}",
            dir.display()
        );
        assert!(get_workflow_library_dir(true).is_absolute());
    }

    #[test]
    fn a_relative_workflow_path_entry_is_anchored_to_the_snapshot_or_dropped() {
        let cwd = PathBuf::from("/some/project");

        assert_eq!(
            anchor_to_working_dir(PathBuf::from("wf"), Some(&cwd)),
            Some(cwd.join("wf")),
            "a declared library is anchored to the directory that was current \
             when the search list was built"
        );
        // ⚠ Spelled per-platform on purpose. `anchor_to_working_dir` asks
        // `Path::is_absolute`, and on Windows that wants a prefix (`C:\…`) or a
        // UNC root — `/abs/wf` merely *has a root* there, so Windows reads it
        // as relative and the entry is dropped. The claim under test is about
        // absoluteness, not about slashes, so the fixture has to be absolute in
        // the host's own spelling for the assertion to mean anything.
        let absolute = if cfg!(windows) {
            PathBuf::from(r"C:\abs\wf")
        } else {
            PathBuf::from("/abs/wf")
        };
        assert!(absolute.is_absolute(), "the fixture must be absolute here");
        assert_eq!(
            anchor_to_working_dir(absolute.clone(), None),
            Some(absolute),
            "an absolute entry needs no working directory"
        );
        assert_eq!(
            anchor_to_working_dir(PathBuf::from("wf"), None),
            None,
            "with no working directory a relative entry cannot mean anything; \
             keeping it would let it mean a later directory"
        );
    }

    /// A stray `::` or a trailing separator must not turn the user's own
    /// directory into a workflow library — the strict origin that makes every
    /// unrelated `.yaml` in it an error.
    #[test]
    fn an_empty_workflow_path_entry_is_not_the_working_directory() {
        let global = tempfile::tempdir().unwrap();
        let working_dir = tempfile::tempdir().unwrap();
        let on_path = tempfile::tempdir().unwrap();
        let path_env = format!("{}::", on_path.path().display());

        let dirs = collect_workflow_dirs(
            Some(&path_env),
            global.path().to_path_buf(),
            Some(working_dir.path()),
            false,
        );

        let canonical = working_dir.path().canonicalize().unwrap();
        assert!(
            !dirs.iter().any(|(dir, _)| *dir == canonical),
            "dirs were {dirs:?}"
        );
    }

    /// Env var that tells a re-executed copy of this test binary that it is the
    /// child half of [`listing_workflows_survives_a_deleted_working_directory`].
    const DELETED_CWD_CHILD: &str = "BIOROUTER_TEST_DELETED_CWD_CHILD";

    /// A session's working directory is user-chosen and can vanish while the
    /// process is still running — a scratch directory cleaned up, a volume
    /// unmounted, a `git worktree remove`. In the CLI resolving it badly costs
    /// one command; in `biorouterd` it takes down a daemon serving every other
    /// session, and the user sees the backend disappear for reasons unrelated
    /// to anything they did. Listing workflows must not be able to do that.
    ///
    /// Deleting the working directory is process-global state, so this cannot
    /// run beside the other tests in this binary: it re-executes the test
    /// binary and does the destructive part in a child process. The child is
    /// pointed at its own config root so it never reads or logs against the
    /// real one.
    #[test]
    #[cfg(unix)]
    fn listing_workflows_survives_a_deleted_working_directory() {
        if env::var(DELETED_CWD_CHILD).is_ok() {
            let scratch = tempfile::tempdir().unwrap().keep();
            env::set_current_dir(&scratch).unwrap();
            fs::remove_dir_all(&scratch).unwrap();
            assert!(
                env::current_dir().is_err(),
                "the child must actually be standing in a deleted directory, \
                 otherwise it proves nothing"
            );

            // None of these may panic: they are all reachable from a daemon
            // request while a session's working directory is gone.
            let project_library = get_workflow_library_dir(false);
            assert!(
                !project_library.is_relative() || project_library.as_os_str().is_empty(),
                "with no working directory there is no project library; a \
                 relative stand-in would bind to whatever directory the process \
                 moves to next, got {}",
                project_library.display()
            );
            let _ = local_workflow_lookup_dirs();
            let workflows = list_local_workflows()
                .expect("listing must degrade to the directories that still exist");
            assert!(workflows.is_empty());
            assert!(
                load_local_workflow_file("no-such-workflow").is_err(),
                "a named lookup must fail as a normal error, not a panic"
            );
            return;
        }

        let config_root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "--nocapture",
                "workflow::local_workflows::tests::listing_workflows_survives_a_deleted_working_directory",
            ])
            .env(DELETED_CWD_CHILD, "1")
            // Hermetic: its own config root, no inherited workflow path, and
            // the working-directory scan opted in so the deleted `.` is
            // enumerated too.
            .env("BIOROUTER_PATH_ROOT", config_root.path())
            .env(BIOROUTER_WORKFLOW_SCAN_WORKING_DIR_ENV_VAR, "1")
            .env_remove(BIOROUTER_WORKFLOW_PATH_ENV_VAR)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "a deleted working directory must not take the process down.\n\
             --- child stdout ---\n{}\n--- child stderr ---\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    /// Env var carrying the decoy project's path to the child half of
    /// [`a_lost_working_directory_never_binds_a_root_to_another_project`].
    const DECOY_CWD_CHILD: &str = "BIOROUTER_TEST_DECOY_CWD_CHILD";

    /// A search root decided while the working directory is unresolvable must
    /// not later attach itself to a *different* project.
    ///
    /// The process working directory is mutable and this codebase moves it
    /// itself — `biorouter-cli`'s session builder calls
    /// `std::env::set_current_dir` when a resumed session asks to go back to
    /// its original directory — while `biorouterd` serves other sessions from
    /// the same process. So a root that is still *relative* when it goes into
    /// the search list is not a root at all: it is a promise to look in
    /// whatever directory the process happens to be standing in when the list
    /// is finally consumed. If that is another project, that project's
    /// workflows are loaded and presented as this one's.
    ///
    /// That is strictly worse than the panic this fallback replaced: a panic is
    /// loud and local, silently reading someone else's directory is neither.
    ///
    /// Moving the working directory is process-global, so the destructive half
    /// runs in a re-executed copy of this test binary, pointed at its own
    /// config root.
    #[test]
    #[cfg(unix)]
    fn a_lost_working_directory_never_binds_a_root_to_another_project() {
        if let Ok(decoy) = env::var(DECOY_CWD_CHILD) {
            let decoy = PathBuf::from(decoy);

            // Stand in a directory and lose it, as a cleaned-up scratch dir or
            // a `git worktree remove` does under a live session.
            let scratch = tempfile::tempdir().unwrap().keep();
            env::set_current_dir(&scratch).unwrap();
            fs::remove_dir_all(&scratch).unwrap();
            assert!(
                env::current_dir().is_err(),
                "the child must actually be standing in a deleted directory, \
                 otherwise it proves nothing"
            );

            // Decide the roots now — this is what a listing request or a named
            // lookup does the moment it arrives.
            let scan_roots = local_workflow_scan_dirs();
            let lookup_roots = local_workflow_lookup_dirs();

            // ...and now the process moves, exactly as the session builder
            // moves it, into an unrelated project that has a workflow library.
            env::set_current_dir(&decoy).unwrap();

            let mut found = Vec::new();
            for (dir, origin) in &scan_roots {
                found.extend(scan_directory_for_workflows(dir, *origin).unwrap());
            }
            let leaked: Vec<String> = found
                .iter()
                .filter(|(_, workflow)| workflow.title.starts_with("Decoy"))
                .map(|(path, _)| path.display().to_string())
                .collect();
            assert!(
                leaked.is_empty(),
                "the working directory these roots were derived from is gone, so \
                 they name nothing; instead they bound themselves to whatever \
                 directory the process moved to and listed that project's \
                 workflows as this session's. Leaked: {leaked:?}; roots were \
                 {scan_roots:?}"
            );

            for dir in &lookup_roots {
                assert!(
                    load_workflow_file_from_dir(dir, "decoy-workflow").is_err(),
                    "a named lookup resolved into another project's workflow \
                     library ({}); lookup roots were {lookup_roots:?}",
                    dir.display()
                );
            }

            // The structural invariant behind both assertions: a root that is
            // still relative has not been decided, only deferred.
            for (dir, _) in &scan_roots {
                assert!(
                    dir.is_absolute(),
                    "search root {} is relative, so it re-resolves against \
                     whatever the working directory is when it is used",
                    dir.display()
                );
            }
            for dir in &lookup_roots {
                assert!(
                    dir.is_absolute(),
                    "lookup root {} is relative, so it re-resolves against \
                     whatever the working directory is when it is used",
                    dir.display()
                );
            }
            return;
        }

        let decoy = tempfile::tempdir().unwrap();
        let decoy_library = decoy.path().join(".biorouter").join("workflows");
        fs::create_dir_all(&decoy_library).unwrap();
        write_file(
            &decoy_library,
            "decoy-workflow.yaml",
            "title: Decoy Project Library\ndescription: another project's workflow\nprompt: go\n",
        );
        // The working-directory root is the second relative root in the list,
        // so give the decoy something there too.
        write_file(
            decoy.path(),
            "decoy-root.yaml",
            "title: Decoy Working Directory\ndescription: another project's workflow\nprompt: go\n",
        );

        let config_root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "--nocapture",
                "workflow::local_workflows::tests::a_lost_working_directory_never_binds_a_root_to_another_project",
            ])
            .env(DECOY_CWD_CHILD, decoy.path())
            .env("BIOROUTER_PATH_ROOT", config_root.path())
            .env(BIOROUTER_WORKFLOW_SCAN_WORKING_DIR_ENV_VAR, "1")
            .env_remove(BIOROUTER_WORKFLOW_PATH_ENV_VAR)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "a search root decided without a working directory must not attach \
             itself to a later one.\n\
             --- child stdout ---\n{}\n--- child stderr ---\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    #[test]
    fn valid_workflows_are_still_loaded_alongside_unrelated_files() {
        let dir = tempfile::tempdir().unwrap();
        write_file(
            dir.path(),
            "good.yaml",
            "title: Good\ndescription: A good workflow\nprompt: go\n",
        );
        write_file(dir.path(), "unrelated.json", r#"{"machineID": "abc"}"#);

        let (workflows, logs) = scan_with_logs(dir.path(), ScanOrigin::WorkingDirectory);

        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0].1.title, "Good");
        assert!(!logs.contains("ERROR"), "logs were:\n{logs}");
    }
}
