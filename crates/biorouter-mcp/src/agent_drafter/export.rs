//! Exporting a Biorouter app as a folder that runs on another machine.
//!
//! **One implementation behind every door.** The `export_app` tool writes the
//! folder where it was told to ([`export_to_dir`]); the desktop's Export button
//! and a `biorouter serve` tab receive it as a zip that `POST /apps/{id}/export`
//! builds with [`export_to_staging`]; and the legacy `GET /apps/{id}/export`
//! answers older desktops with [`legacy_export_map`]. There used to be two
//! writers, and the desktop's silently ignored every option its dialog offered:
//! a "Full" export came out as a launcher export, and nothing said so.
//!
//! What a folder holds, and the failure each rule exists for:
//! - The **raw authored entry**, never an assembled page. Every way an export
//!   runs is a daemon serving the page, and the daemon assembles it with this
//!   run's socket token. A page assembled at export time carried a token-less
//!   config island that the SDK read first, so no exported agentic app could
//!   reach its agent, and running an export on the author's machine wrote that
//!   page over the live app.
//! - Every other authored file **byte for byte** (`assets/logo.png`, fonts,
//!   wasm). Reading files as text dropped every binary one without a word.
//! - Never `workspace/` (the app's runtime data), never a `.vault`, `.git` or
//!   `node_modules` component anywhere, never a file named like a credential,
//!   and never a symlink. ⚠ The vault exclusion used to match `<app>/.vault/`
//!   only, while the vault lives at `workspace/.vault/`: sealed secrets stayed
//!   out solely because ciphertext is not valid UTF-8, so the binary-safe copy
//!   above could not land without this.
//! - `manifest.json` without `session_id`. A chat id means nothing on another
//!   machine, and there it can name an unrelated chat.
//! - The generated launchers, executable. An authored file with one of their
//!   names moves under `app-files/`, so it can never replace a launcher.
//! - In full mode, `payload/` (knowledge bases and skills) and `export.json`.
//!   An absent selection key means "everything the app references", an empty
//!   list means "none", and each knowledge base is asked of the caller's reach
//!   through [`PayloadAccess`], so a private base never leaves for a caller that
//!   could not open it.

use std::collections::BTreeSet;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use super::store::{validate_artifact_id, ArtifactKind, ArtifactStore, Manifest};
use super::{bundle, render};
use crate::knowledge::caller::KbCaller;
use crate::knowledge::service::KnowledgeService;

/// Path components that never leave the machine, wherever they appear: the
/// app's sealed secrets, a repository's history (remote URLs carry tokens), and
/// installed packages. The same rule the app jail applies (`developer::jail`),
/// by component rather than by prefix.
const EXCLUDED_COMPONENTS: &[&str] = &[".vault", ".git", "node_modules"];

/// How deep an app or skill tree is followed. A real one is a few levels deep;
/// the bound is what makes a pathological tree end.
const MAX_DEPTH: usize = 24;
/// More files than any authored app has; an app past it is refused rather than
/// half-exported.
const MAX_APP_FILES: usize = 20_000;
/// A skill is a few documents and scripts. These bounds keep a skill folder that
/// happens to hold a dataset from turning an export into a copy of it.
const MAX_SKILL_FILES: usize = 5_000;
const MAX_SKILL_BYTES: u64 = 256 * 1024 * 1024;

// ---------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------

/// What the exported folder carries besides the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExportMode {
    /// The app and its launchers. It runs against whatever knowledge bases,
    /// skills and extensions the recipient already has.
    #[default]
    Launcher,
    /// Also the app's server-side payload under `payload/`, and `export.json`.
    Full,
}

impl ExportMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "launcher" => Some(Self::Launcher),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Launcher => "launcher",
            Self::Full => "full",
        }
    }
}

/// A full export's selection. `None` for a key means "everything the app
/// references"; `Some(vec![])` means "none of these". ⚠ The two must stay
/// distinct all the way to the wire: the desktop used to drop an empty list, so
/// unchecking a private knowledge base sent no key at all and the base shipped
/// anyway.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportSelection {
    pub knowledge_bases: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    pub extensions: Option<Vec<String>>,
}

impl ExportSelection {
    /// Read `{"knowledge_bases": [...], "skills": [...], "extensions": [...]}`.
    /// A missing or `null` key is "everything referenced"; anything that is not
    /// a list of strings is refused rather than read as "everything".
    pub fn from_json(include: Option<&serde_json::Value>) -> Result<Self, String> {
        let Some(include) = include.filter(|value| !value.is_null()) else {
            return Ok(Self::default());
        };
        let Some(object) = include.as_object() else {
            return Err(
                "`include` must be an object with `knowledge_bases`, `skills` and `extensions` \
                 lists"
                    .to_string(),
            );
        };
        let list = |key: &str| -> Result<Option<Vec<String>>, String> {
            match object.get(key) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(serde_json::Value::Array(items)) => items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| format!("`include.{key}` must list names as strings"))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Some),
                Some(_) => Err(format!("`include.{key}` must be a list")),
            }
        };
        Ok(Self {
            knowledge_bases: list("knowledge_bases")?,
            skills: list("skills")?,
            extensions: list("extensions")?,
        })
    }
}

/// One export, as every door asks for it.
#[derive(Debug, Clone, Default)]
pub struct ExportRequest {
    pub mode: ExportMode,
    /// Read in full mode only.
    pub include: ExportSelection,
    /// Copy this platform's `biorouterd` into `payload/bin/` ("fat" export).
    pub bundle_daemon: bool,
    /// Export even when the harness lint finds ERRORs. Only a person decides
    /// this: the desktop asks them, and the model-facing tool never sets it.
    pub allow_lint_errors: bool,
}

// ---------------------------------------------------------------------------
// What the caller may carry out
// ---------------------------------------------------------------------------

/// Where a skill lives: its folder, and the root that folder must stay inside.
#[derive(Debug, Clone)]
pub struct SkillLocation {
    pub dir: PathBuf,
    pub root: PathBuf,
}

/// The two things a full export reaches outside the app, answered by the door
/// that asked. `biorouter-mcp` cannot depend on `biorouter`, so the HTTP route
/// plugs in its own reach decision (`HttpCaller`) and the daemon's skill
/// catalog, while the `export_app` tool uses [`DrafterPayload`].
pub trait PayloadAccess {
    /// `Ok` when this caller may carry knowledge base `kb_id` out of the
    /// machine; `Err` is the sentence the export notes in its place.
    fn reach_kb(&self, kb_id: &str) -> Result<(), String>;
    /// The base as a `.brkb` archive. Asked only after [`Self::reach_kb`].
    fn export_kb(&self, kb_id: &str) -> Result<Vec<u8>, String>;
    /// The knowledge store's root, or `None` when it cannot be opened. An
    /// export target inside it is refused.
    fn knowledge_root(&self) -> Option<PathBuf>;
    /// Where skill `name` (already validated by [`is_valid_item_name`]) lives.
    fn skill_dir(&self, name: &str) -> Option<SkillLocation>;
    /// Whether `name` is a skill Biorouter ships, which no export carries
    /// ([`render::SHIPPED_SKILL_NAMES`]). The HTTP door asks the seeder's own
    /// list instead, which a test holds equal to this one.
    fn is_shipped_skill(&self, name: &str) -> bool {
        render::SHIPPED_SKILL_NAMES
            .iter()
            .any(|shipped| shipped.eq_ignore_ascii_case(name))
    }
}

/// The `export_app` tool's [`PayloadAccess`]: the calling chat's capability
/// (issue #56, CP4) against the knowledge store, and the installed-skills
/// folder.
pub struct DrafterPayload<'a> {
    caller: &'a KbCaller,
    knowledge: Option<KnowledgeService>,
}

impl<'a> DrafterPayload<'a> {
    pub fn new(caller: &'a KbCaller) -> Self {
        Self {
            caller,
            knowledge: knowledge_service_for_export(),
        }
    }
}

impl PayloadAccess for DrafterPayload<'_> {
    fn reach_kb(&self, kb_id: &str) -> Result<(), String> {
        let Some(svc) = &self.knowledge else {
            return Err("the knowledge store could not be opened".to_string());
        };
        // Issue #56 (CP4), Task 10C. `export_brkb` writes the WHOLE base into
        // the payload, and the id can come from the model; skip-and-note rather
        // than fail the export, matching `search_visible_bases`.
        self.caller
            .assert_reachable(svc.root(), kb_id)
            .map_err(|e| e.to_string())
    }

    fn export_kb(&self, kb_id: &str) -> Result<Vec<u8>, String> {
        let Some(svc) = &self.knowledge else {
            return Err("the knowledge store could not be opened".to_string());
        };
        svc.export_brkb(kb_id).map_err(|e| e.to_string())
    }

    fn knowledge_root(&self) -> Option<PathBuf> {
        self.knowledge.as_ref().map(|svc| svc.root().to_path_buf())
    }

    fn skill_dir(&self, name: &str) -> Option<SkillLocation> {
        skill_in_root(&skills_root_for_export(), name)
    }
}

/// Resolve a `KnowledgeService` for the drafter's exports. Honours
/// `BIOROUTER_KNOWLEDGE_DIR` (an override used by tests and power users);
/// otherwise the canonical store the `/knowledge` routes use. `None` when the
/// store cannot be opened, and the export then notes the bases it left out.
pub(crate) fn knowledge_service_for_export() -> Option<KnowledgeService> {
    if let Ok(dir) = std::env::var("BIOROUTER_KNOWLEDGE_DIR") {
        if !dir.trim().is_empty() {
            return Some(KnowledgeService::new(PathBuf::from(dir)));
        }
    }
    KnowledgeService::new_default().ok()
}

/// The installed-skills directory (`<config>/skills`). Honours
/// `BIOROUTER_SKILLS_DIR` as a test/override hook.
pub fn skills_root_for_export() -> PathBuf {
    if let Ok(dir) = std::env::var("BIOROUTER_SKILLS_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    crate::paths::in_config_dir("skills")
}

/// A skill or extension name as one plain path component:
/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`.
///
/// ⚠ This is what stops `include.skills: ["../../.ssh"]` — the join used to take
/// any string, so a name climbed out of the skills root on both the source and
/// the destination side, and an absolute name replaced the root outright (with
/// source and destination then the same file, which `fs::copy` truncates).
pub fn is_valid_item_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Find skill `name` under `root`: `<root>/<name>`, else one bundle level down
/// (`<root>/<bundle>/<name>`), which is how skill discovery scans a root. The
/// direct path is returned whatever it is, so a link there is reported as a
/// link rather than as "not installed".
pub fn skill_in_root(root: &Path, name: &str) -> Option<SkillLocation> {
    if !is_valid_item_name(name) {
        return None;
    }
    let direct = root.join(name);
    if std::fs::symlink_metadata(&direct).is_ok() {
        return Some(SkillLocation {
            dir: direct,
            root: root.to_path_buf(),
        });
    }
    let mut bundles: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    bundles.sort();
    bundles
        .into_iter()
        .map(|bundle| bundle.join(name))
        .find(|candidate| {
            std::fs::symlink_metadata(candidate).is_ok_and(|meta| meta.is_dir())
                && std::fs::symlink_metadata(candidate.join("SKILL.md"))
                    .is_ok_and(|meta| meta.is_file())
        })
        .map(|dir| SkillLocation {
            dir,
            root: root.to_path_buf(),
        })
}

// ---------------------------------------------------------------------------
// The result
// ---------------------------------------------------------------------------

/// What an export did, for the tool's summary, the HTTP report header and the
/// desktop's toast.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ExportReport {
    pub id: String,
    pub mode: &'static str,
    /// Where the folder is, for a door that wrote one the caller chose.
    pub target: String,
    /// Files in the folder, and their bytes.
    pub files: usize,
    pub bytes: u64,
    /// Knowledge bases, skills and external extension references the payload
    /// carries.
    pub kbs: Vec<String>,
    pub skills: Vec<String>,
    pub extensions: Vec<String>,
    /// The bundled daemon's `export.json` record, for a fat export.
    pub daemon: Option<serde_json::Value>,
    /// Things the person should know: what was left out and why, a rename.
    pub notes: Vec<String>,
    /// Harness findings that did not block the export.
    pub lint_warnings: Vec<String>,
    /// App or skill files deliberately not exported, each with its reason.
    pub skipped: Vec<String>,
    pub wrote_export_json: bool,
    /// Every relative path written, sorted. For tests and the zip writer.
    #[serde(skip)]
    pub written: Vec<String>,
}

impl ExportReport {
    fn new(id: &str, mode: ExportMode) -> Self {
        Self {
            id: id.to_string(),
            mode: mode.as_str(),
            ..Default::default()
        }
    }

    /// The report as the HTTP route sends it in a response header, without the
    /// daemon's temporary path and capped at about 6 KB.
    ///
    /// ⚠ The cap is not cosmetic: Node's HTTP client refuses a response whose
    /// headers exceed 16 KB, so a long list of skipped files would turn a good
    /// export into a failed download. The lists are trimmed from the end, with
    /// a line saying how many were left out.
    pub fn header_json(&self) -> serde_json::Value {
        const LIMIT: usize = 6_000;
        let mut notes = self.notes.clone();
        let mut lint = self.lint_warnings.clone();
        let mut skipped = self.skipped.clone();
        let render = |notes: &[String], lint: &[String], skipped: &[String]| {
            serde_json::json!({
                "id": self.id,
                "mode": self.mode,
                "files": self.files,
                "bytes": self.bytes,
                "kbs": self.kbs,
                "skills": self.skills,
                "extensions": self.extensions,
                "daemon": self.daemon,
                "notes": notes,
                "lint_warnings": lint,
                "skipped": skipped,
                "wrote_export_json": self.wrote_export_json,
            })
        };
        let mut dropped = 0usize;
        loop {
            let mut value = render(&notes, &lint, &skipped);
            if value.to_string().len() <= LIMIT {
                if dropped > 0 {
                    if let Some(list) = value["notes"].as_array_mut() {
                        list.push(serde_json::Value::String(format!(
                            "{dropped} more line(s) were left out of this summary"
                        )));
                    }
                }
                return value;
            }
            // Halve the longest list (a handful of passes, however long it
            // is); once every list is down to one line, halve the longest line.
            let longest = [&mut skipped, &mut lint, &mut notes]
                .into_iter()
                .filter(|list| !list.is_empty())
                .max_by_key(|list| list.len());
            match longest {
                Some(list) if list.len() > 1 => {
                    let keep = list.len() / 2;
                    dropped += list.len() - keep;
                    list.truncate(keep);
                }
                Some(list) => {
                    let line = &mut list[0];
                    let mut cut = line.len() / 2;
                    while !line.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    if cut == 0 {
                        list.clear();
                        dropped += 1;
                    } else {
                        line.truncate(cut);
                        line.push('…');
                    }
                }
                None => return render(&[], &[], &[]),
            }
        }
    }

    /// The `export_app` tool's answer to the model.
    pub fn summary(&self) -> String {
        let mut msg = format!(
            "Exported '{}' ({} mode) to {} ({} files). To run it: double-click run.command \
             (macOS), `bash run.sh` (Linux), or run.bat (Windows). That installs the app, \
             starts a biorouterd if one isn't already up, and opens it in the browser, with no \
             npm install and no build step.",
            self.id, self.mode, self.target, self.files
        );
        if self.mode == ExportMode::Full.as_str() {
            msg.push_str(&format!(
                "\nPayload: {} knowledge base(s), {} skill(s), {} external extension \
                 reference(s).",
                self.kbs.len(),
                self.skills.len(),
                self.extensions.len()
            ));
        }
        if self.wrote_export_json {
            msg.push_str(" Wrote export.json (audit manifest).");
        }
        for note in &self.notes {
            msg.push_str(&format!("\n- {note}"));
        }
        if !self.skipped.is_empty() {
            msg.push_str("\nNot exported:");
            for line in self.skipped.iter().take(20) {
                msg.push_str(&format!("\n- {line}"));
            }
            if self.skipped.len() > 20 {
                msg.push_str(&format!("\n- and {} more", self.skipped.len() - 20));
            }
        }
        msg
    }
}

/// Why an export did not happen. Each carries the sentence the person reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportError {
    /// There is no such app (HTTP 404).
    NotFound(String),
    /// The request or the target is unusable (HTTP 400).
    BadRequest(String),
    /// The harness lint found ERRORs and nobody chose to export anyway
    /// (HTTP 422, with the findings).
    Lint {
        message: String,
        findings: Vec<String>,
    },
    /// The app itself cannot be exported as it stands: its entry is missing or
    /// unsafe, or its manifest does not load (HTTP 409).
    Unexportable(String),
    /// Reading the app or writing the folder failed (HTTP 500).
    Io(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(m) | Self::BadRequest(m) | Self::Unexportable(m) | Self::Io(m) => {
                f.write_str(m)
            }
            Self::Lint { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for ExportError {}

fn io_error(what: &str, error: impl std::fmt::Display) -> ExportError {
    ExportError::Io(format!("{what}: {error}"))
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Build the export of app `id` into `out`, a folder that must not exist yet
/// and whose parent must. For a door that owns `out` — the HTTP route builds into
/// a temporary directory of its own and zips it.
pub fn export_to_staging(
    store_root: &Path,
    id: &str,
    request: &ExportRequest,
    access: &dyn PayloadAccess,
    out: &Path,
) -> Result<ExportReport, ExportError> {
    build_export(store_root, id, request, access, out)
}

/// Export app `id` into `target`, a folder a model named.
///
/// The target is refused unless it is absolute, outside the Biorouter config
/// folder, the app store and the knowledge store, and a folder of the person's
/// own ([`placement_refusal`]: below Desktop, Documents or Downloads, never
/// hidden or a folder code is imported from); and unless it does not exist,
/// or is empty. With `replace`, a
/// non-empty target is accepted only when it is an earlier export of this same
/// app (its `manifest.json` names `id` and it has `biorouter-launch.sh`) that
/// holds nothing the export does not write ([`foreign_entries`]).
///
/// ⚠ The folder is built beside the target and renamed into place, so a failed
/// export leaves nothing half-written, a replaced export is swapped whole (no
/// stale `payload/` or deleted `src/` files survive), and nothing is ever
/// written through a link that was already in the target. The replaced export
/// is kept beside it, renamed `<name>.replaced-<time>`, and the report names it.
pub fn export_to_dir(
    store_root: &Path,
    id: &str,
    request: &ExportRequest,
    access: &dyn PayloadAccess,
    target: &Path,
    replace: bool,
) -> Result<ExportReport, ExportError> {
    validate_artifact_id(id)
        .map_err(|_| ExportError::BadRequest(format!("'{id}' is not an app id.")))?;
    let state = validate_target(target, id, store_root, access, replace)?;
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(ExportError::BadRequest(
            "target_dir must name a folder, not a filesystem root.".to_string(),
        ));
    };
    std::fs::create_dir_all(parent)
        .map_err(|e| io_error("could not create the target's parent", e))?;
    let name = name.to_string_lossy();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let staging = parent.join(format!(".{name}.partial-{nonce}"));
    let mut report = match build_export(store_root, id, request, access, &staging) {
        Ok(report) => report,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    let placed = match state {
        TargetState::Absent => std::fs::rename(&staging, target),
        TargetState::Empty => {
            std::fs::remove_dir(target).and_then(|()| std::fs::rename(&staging, target))
        }
        // ⚠ The earlier export is KEPT, beside the new one under a name that
        // says what it is, and never deleted: the README invites editing an
        // export where it is, so it may hold changes made nowhere else. It was
        // `remove_dir_all`ed, with no question asked and nothing to restore.
        TargetState::EarlierExport => {
            let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
            let mut aside = parent.join(format!("{name}.replaced-{stamp}"));
            if std::fs::symlink_metadata(&aside).is_ok() {
                let short: String = nonce.chars().take(8).collect();
                aside = parent.join(format!("{name}.replaced-{stamp}-{short}"));
            }
            match std::fs::rename(target, &aside) {
                Err(error) => Err(error),
                Ok(()) => match std::fs::rename(&staging, target) {
                    Ok(()) => {
                        report.notes.push(format!(
                            "the earlier export is kept, unchanged, at {}; delete it once you no \
                             longer need it",
                            aside.display()
                        ));
                        Ok(())
                    }
                    Err(error) => {
                        let _ = std::fs::rename(&aside, target);
                        Err(error)
                    }
                },
            }
        }
    };
    if let Err(error) = placed {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io_error(
            &format!("could not move the export into {}", target.display()),
            error,
        ));
    }
    report.target = target.display().to_string();
    Ok(report)
}

/// The launcher-only text file map the legacy `GET /apps/{id}/export` serves to
/// a desktop that predates `POST`. It carries the same raw entry and stripped
/// manifest as every other export; the files it cannot carry (binary ones, which
/// a JSON string map cannot hold) are listed in `skipped_binary` instead of
/// being dropped without a word.
#[derive(Debug, Clone, Default)]
pub struct LegacyExport {
    pub files: Vec<(String, String)>,
    pub skipped_binary: Vec<String>,
    pub skipped: Vec<String>,
}

pub fn legacy_export_map(store_root: &Path, id: &str) -> Result<LegacyExport, ExportError> {
    let (store, manifest, app_dir, mut report) = prepare(store_root, id, ExportMode::Launcher)?;
    let collected = collect_app_files(&app_dir, &manifest, &mut report)?;
    let mut out = LegacyExport {
        files: vec![(
            "manifest.json".to_string(),
            render::exported_manifest_json(&manifest),
        )],
        ..Default::default()
    };
    let digest = render::runtime_digest(&app_dir, &manifest.entry);
    out.files
        .extend(render::launcher_files(&manifest, digest.as_deref()));
    let mut push = |rel: String, src: &Path| match std::fs::read(src) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => out.files.push((rel, text)),
            Err(_) => out.skipped_binary.push(rel),
        },
        Err(_) => out.skipped.push(format!("{rel}: could not be read")),
    };
    for file in &collected {
        push(export_path_for(&file.rel), &file.src);
    }
    for (rel, src) in built_bundle(&store, id) {
        push(rel, &src);
    }
    out.skipped.extend(report.skipped);
    Ok(out)
}

/// [`legacy_export_map`]'s files, for callers that only want the map.
pub fn export_scaffold(
    root: &Path,
    id: &str,
    _endpoint: Option<&str>,
) -> io::Result<Vec<(String, String)>> {
    legacy_export_map(root, id)
        .map(|export| export.files)
        .map_err(|error| match error {
            ExportError::NotFound(m) => io::Error::new(io::ErrorKind::NotFound, m),
            other => io::Error::other(other.to_string()),
        })
}

// ---------------------------------------------------------------------------
// Building the folder
// ---------------------------------------------------------------------------

/// Validate the id, load the app, lint-free or not, and freshen its bundle.
fn prepare(
    store_root: &Path,
    id: &str,
    mode: ExportMode,
) -> Result<(ArtifactStore, Manifest, PathBuf, ExportReport), ExportError> {
    validate_artifact_id(id)
        .map_err(|_| ExportError::BadRequest(format!("'{id}' is not an app id.")))?;
    let store = ArtifactStore::new(store_root.to_path_buf());
    if !store.exists(id) {
        return Err(ExportError::NotFound(format!("There is no app '{id}'.")));
    }
    let mut manifest = store.load_manifest(id).map_err(|e| {
        ExportError::Unexportable(format!("The manifest of '{id}' could not be read: {e}"))
    })?;
    let app_dir = open_app_dir(&store, id)?;
    let mut report = ExportReport::new(id, mode);
    // Ship a bundle built from the CURRENT sources and SDK, not a stale one the
    // app happens to have on disk: an export is the copy that leaves the
    // machine. The same locked check serving a page makes, so an export never
    // races a page load's build, and a build that failed on these very inputs
    // is not run again.
    match super::ensure_bundle_fresh(&store, id) {
        Ok(check) if check.build.as_ref().is_some_and(|build| !build.ok) => {
            report.notes.push(if check.has_bundle {
                "the app's bundle could not be rebuilt; the export carries the last one that \
                 built, and the daemon that opens it tries again"
                    .to_string()
            } else {
                "the app's bundle could not be built; the daemon that opens it tries again"
                    .to_string()
            })
        }
        Ok(_) => {}
        Err(error) => report
            .notes
            .push(format!("the app's bundle could not be checked ({error})")),
    }
    if let Ok(fresh) = store.load_manifest(id) {
        manifest = fresh;
    }
    validate_entry(&manifest.entry).map_err(ExportError::Unexportable)?;
    Ok((store, manifest, app_dir, report))
}

fn build_export(
    store_root: &Path,
    id: &str,
    request: &ExportRequest,
    access: &dyn PayloadAccess,
    out: &Path,
) -> Result<ExportReport, ExportError> {
    validate_artifact_id(id)
        .map_err(|_| ExportError::BadRequest(format!("'{id}' is not an app id.")))?;
    let store = ArtifactStore::new(store_root.to_path_buf());
    if !store.exists(id) {
        return Err(ExportError::NotFound(format!("There is no app '{id}'.")));
    }
    // The lint runs before anything is built or written: an app the harness
    // refuses is not exported unless a person chose to export it anyway.
    let app_dir = open_app_dir(&store, id)?;
    let lint = bundle::lint_app(&app_dir);
    let errors: Vec<String> = lint
        .iter()
        .filter(|f| f.level == bundle::LintLevel::Error)
        .map(|f| f.msg.clone())
        .collect();
    if !errors.is_empty() && !request.allow_lint_errors {
        return Err(ExportError::Lint {
            message: format!(
                "harness ERRORs block export for '{id}':\n{}",
                bundle::format_lint(&lint)
            ),
            findings: errors,
        });
    }

    let (store, manifest, app_dir, mut report) = prepare(store_root, id, request.mode)?;
    for finding in &lint {
        report.lint_warnings.push(match finding.level {
            bundle::LintLevel::Error => format!("ERROR: {}", finding.msg),
            bundle::LintLevel::Warn => finding.msg.clone(),
        });
    }
    if !errors.is_empty() {
        report.notes.push(format!(
            "exported despite {} harness error(s); the app may not work until they are fixed",
            errors.len()
        ));
    }

    let collected = collect_app_files(&app_dir, &manifest, &mut report)?;
    std::fs::create_dir(out).map_err(|e| io_error("could not create the export folder", e))?;

    for file in &collected {
        let rel = export_path_for(&file.rel);
        if rel != file.rel {
            report.notes.push(format!(
                "the app's own '{}' is in {rel}: the export writes its own file by that name",
                file.rel
            ));
        }
        let dst = join_rel(out, &rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| io_error("could not write the export", e))?;
        }
        std::fs::copy(&file.src, &dst)
            .map_err(|e| io_error(&format!("could not copy {}", file.rel), e))?;
    }
    let bundle = built_bundle(&store, id);
    for (rel, src) in &bundle {
        std::fs::create_dir_all(out.join("dist"))
            .and_then(|()| std::fs::copy(src, join_rel(out, rel)).map(|_| ()))
            .map_err(|e| io_error(&format!("could not copy {rel}"), e))?;
    }
    if bundle.is_empty() && manifest.kind == ArtifactKind::Agentic {
        report.notes.push(
            "the app has no built bundle yet; the daemon that opens it builds one".to_string(),
        );
    }

    write_file(
        out,
        "manifest.json",
        &render::exported_manifest_json(&manifest),
    )?;
    // What the store held as it was exported, so the folder's launchers can
    // tell an untouched install (re-synced) from one changed where it is
    // installed (left alone).
    let digest = render::runtime_digest(&app_dir, &manifest.entry);
    for (rel, content) in render::launcher_files(&manifest, digest.as_deref()) {
        write_file(out, &rel, &content)?;
        if render::EXECUTABLE_EXPORT_FILES.contains(&rel.as_str()) {
            make_executable(&join_rel(out, &rel));
        }
    }

    let staged = if request.mode == ExportMode::Full {
        stage_payload(&manifest, &request.include, access, out, &mut report)
    } else {
        Staged::default()
    };
    if request.bundle_daemon {
        let (record, note) = stage_current_daemon(out);
        report.notes.push(note);
        report.daemon = record;
    }
    // export.json travels whenever the folder carries a payload: full mode, or
    // a bundled daemon in launcher mode (a "fat launcher").
    if request.mode == ExportMode::Full || report.daemon.is_some() {
        let json = build_export_json(id, request.mode, &staged, report.daemon.as_ref());
        write_file(
            out,
            "export.json",
            &serde_json::to_string_pretty(&json).unwrap_or_default(),
        )?;
        report.wrote_export_json = true;
    }

    tally(out, out, &mut report).map_err(|e| io_error("could not read back the export", e))?;
    report.written.sort();
    Ok(report)
}

/// The app's folder, as the store hands it out. The store refuses an app whose
/// tree holds a link or a hard-linked file (it serves no such app either), and
/// that refusal is the app's state, not an I/O failure, so it is answered with
/// a sentence the person can act on rather than as a server error.
fn open_app_dir(store: &ArtifactStore, id: &str) -> Result<PathBuf, ExportError> {
    store.artifact_dir(id).map_err(|error| {
        if error.kind() == io::ErrorKind::InvalidInput {
            ExportError::Unexportable(format!(
                "'{id}' cannot be exported: {error}. Remove the link from the app's folder \
                 and export again."
            ))
        } else {
            io_error("could not open the app", error)
        }
    })
}

/// The authored entry must be a plain relative path that no generated file or
/// reserved folder shadows.
fn validate_entry(entry: &str) -> Result<(), String> {
    let path = Path::new(entry);
    let plain = !entry.is_empty()
        && !entry.contains('\\')
        && !path.is_absolute()
        && path.components().all(|c| matches!(c, Component::Normal(_)));
    let top = entry.split('/').next().unwrap_or_default();
    if !plain
        || render::GENERATED_EXPORT_FILES.contains(&entry)
        || render::RESERVED_EXPORT_DIRS.contains(&top)
        || matches!(top, "workspace" | "dist" | "src")
    {
        return Err(format!(
            "The app's entry file '{entry}' cannot be exported: it must be an HTML file in the \
             app's own folder, such as index.html."
        ));
    }
    Ok(())
}

struct Collected {
    rel: String,
    src: PathBuf,
}

/// Every authored file of the app, except what never leaves the machine. Each
/// deliberate omission is recorded in `report.skipped` with its reason.
fn collect_app_files(
    app_dir: &Path,
    manifest: &Manifest,
    report: &mut ExportReport,
) -> Result<Vec<Collected>, ExportError> {
    let mut out = Vec::new();
    walk_app(app_dir, app_dir, 0, &mut out, report)
        .map_err(|e| io_error("could not read the app's files", e))?;
    if out.len() > MAX_APP_FILES {
        return Err(ExportError::Unexportable(format!(
            "The app has more than {MAX_APP_FILES} files; export the ones it needs from its \
             folder by hand."
        )));
    }
    if !out.iter().any(|file| file.rel == manifest.entry) {
        return Err(ExportError::Unexportable(format!(
            "The app's entry file '{}' is missing or is a link.",
            manifest.entry
        )));
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

fn walk_app(
    base: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<Collected>,
    report: &mut ExportReport,
) -> io::Result<()> {
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)?.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(base) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let name = entry.file_name().to_string_lossy().to_string();
        // `DirEntry::file_type` does not follow links, so a link is seen as one.
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            report
                .skipped
                .push(format!("{rel}: a link, never followed"));
            continue;
        }
        if kind.is_dir() {
            if depth == 0 && name == "workspace" {
                report
                    .skipped
                    .push("workspace/: the app's runtime data stays on this machine".to_string());
                continue;
            }
            // The bundle travels on its own (`built_bundle`): only what the
            // build writes, never a stale chunk an older build left beside it.
            if depth == 0 && name == "dist" {
                continue;
            }
            if EXCLUDED_COMPONENTS.contains(&name.as_str()) {
                report.skipped.push(format!("{rel}/: never exported"));
                continue;
            }
            if depth + 1 > MAX_DEPTH {
                report.skipped.push(format!("{rel}/: nested too deeply"));
                continue;
            }
            walk_app(base, &path, depth + 1, out, report)?;
        } else if kind.is_file() {
            // Re-emitted from the parsed manifest, without the author's chat id.
            if depth == 0 && name == "manifest.json" {
                continue;
            }
            if looks_like_credential(&name) {
                report
                    .skipped
                    .push(format!("{rel}: looks like a credential file"));
                continue;
            }
            out.push(Collected { rel, src: path });
            if out.len() > MAX_APP_FILES {
                return Ok(());
            }
        } else {
            report.skipped.push(format!("{rel}: not a regular file"));
        }
    }
    Ok(())
}

/// File names that hold credentials often enough that an export never carries
/// them: dotenv files, private keys and certificates, and package-manager and
/// cloud credential files.
fn looks_like_credential(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || [".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|ext| lower.ends_with(ext))
        || ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"]
            .iter()
            .any(|key| lower.starts_with(key))
        || matches!(
            lower.as_str(),
            ".netrc" | ".npmrc" | ".pypirc" | "credentials"
        )
}

/// Where an authored file goes in the export: its own path, unless that path is
/// one the export writes itself, in which case under `app-files/`.
fn export_path_for(rel: &str) -> String {
    let top = rel.split('/').next().unwrap_or_default();
    let reserved = (rel == top && render::GENERATED_EXPORT_FILES.contains(&rel))
        || render::RESERVED_EXPORT_DIRS.contains(&top);
    if reserved {
        format!("app-files/{rel}")
    } else {
        rel.to_string()
    }
}

/// What the build writes: `dist/app.js`, and `dist/app.css` when the app's
/// source imports a stylesheet (esbuild writes it beside the script). Each is
/// shipped, for any kind of app, when it is a regular file.
const BUNDLE_OUTPUTS: &[&str] = &["app.js", "app.css"];

fn built_bundle(store: &ArtifactStore, id: &str) -> Vec<(String, PathBuf)> {
    let Ok(dist) = store.artifact_dir(id).map(|dir| dir.join("dist")) else {
        return Vec::new();
    };
    BUNDLE_OUTPUTS
        .iter()
        .map(|name| (format!("dist/{name}"), dist.join(name)))
        .filter(|(_, path)| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file()))
        .collect()
}

fn join_rel(base: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

fn write_file(out: &Path, rel: &str, content: &str) -> Result<(), ExportError> {
    let path = join_rel(out, rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_error("could not write the export", e))?;
    }
    std::fs::write(&path, content).map_err(|e| io_error(&format!("could not write {rel}"), e))
}

/// Give a written export file the exec bit (owner+group+other rx, owner w).
/// No-op on Windows, where executability isn't a file mode.
pub(crate) fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Whether the export path `rel` must be executable: the launchers, and a
/// bundled daemon. A writer that cannot read file modes (a zip on Windows, a
/// legacy file map) asks this.
pub fn is_executable_export_path(rel: &str) -> bool {
    render::EXECUTABLE_EXPORT_FILES.contains(&rel)
        || rel == "payload/bin/biorouterd"
        || rel == "payload/bin/biorouterd.exe"
}

/// Count what was written: files, bytes and relative paths.
fn tally(base: &Path, dir: &Path, report: &mut ExportReport) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            tally(base, &entry.path(), report)?;
        } else if kind.is_file() {
            report.files += 1;
            report.bytes += entry.metadata()?.len();
            if let Ok(rel) = entry.path().strip_prefix(base) {
                report
                    .written
                    .push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The target a model named
// ---------------------------------------------------------------------------

enum TargetState {
    Absent,
    Empty,
    EarlierExport,
}

fn validate_target(
    target: &Path,
    id: &str,
    store_root: &Path,
    access: &dyn PayloadAccess,
    replace: bool,
) -> Result<TargetState, ExportError> {
    let refuse = |why: String| Err(ExportError::BadRequest(why));
    if !target.is_absolute() {
        return refuse(format!(
            "target_dir must be an absolute path, such as /Users/you/Desktop/{id}."
        ));
    }
    if target
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        || target.file_name().is_none()
    {
        return refuse("target_dir must name a folder plainly, without '..'.".to_string());
    }
    // Decide on where the target REALLY is: a link anywhere above it is
    // followed here, once, the way the writes would follow it.
    let resolved = resolve_existing_prefix(target);
    let mut protected = vec![
        (
            crate::paths::config_dir(),
            "Biorouter's configuration folder",
        ),
        (store_root.to_path_buf(), "the app store"),
    ];
    if let Some(knowledge) = access.knowledge_root() {
        protected.push((knowledge, "the knowledge store"));
    }
    for (root, what) in protected {
        let root = resolve_existing_prefix(&root);
        if resolved.starts_with(&root) || root.starts_with(&resolved) {
            return refuse(format!(
                "target_dir {} is inside or contains {what}; export somewhere else, such as \
                 your Desktop.",
                target.display()
            ));
        }
    }
    let home = etcetera::home_dir()
        .ok()
        .map(|home| resolve_existing_prefix(&home));
    // A unit test's folders live under the system temp folder; nothing else's
    // may (a temp folder is shared, and nobody looks there for an export).
    let test_roots: Vec<PathBuf> = if cfg!(test) {
        vec![resolve_existing_prefix(&std::env::temp_dir())]
    } else {
        Vec::new()
    };
    if let Some(why) = placement_refusal(&resolved, home.as_deref(), &test_roots) {
        return refuse(format!("target_dir {} {why}", target.display()));
    }
    match std::fs::symlink_metadata(target) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(TargetState::Absent),
        Err(error) => Err(io_error("could not inspect target_dir", error)),
        Ok(meta) if meta.file_type().is_symlink() => refuse(format!(
            "target_dir {} is a link; name the real folder.",
            target.display()
        )),
        Ok(meta) if !meta.is_dir() => refuse(format!(
            "target_dir {} is a file, not a folder.",
            target.display()
        )),
        Ok(_) => {
            let empty = std::fs::read_dir(target)
                .map_err(|e| io_error("could not read target_dir", e))?
                .next()
                .is_none();
            if empty {
                Ok(TargetState::Empty)
            } else if !replace {
                refuse(format!(
                    "target_dir {} is not empty. Pick a new folder, or pass replace: true to \
                     replace an earlier export of this same app.",
                    target.display()
                ))
            } else if is_export_of(target, id) {
                // The README invites editing an export where it is, so one
                // that holds anything the export does not write (a `.git`,
                // `node_modules`, a file of the person's own) has become their
                // project, and is never replaced.
                let foreign = foreign_entries(target, &store_root.join(id));
                if foreign.is_empty() {
                    Ok(TargetState::EarlierExport)
                } else {
                    let shown: Vec<&str> = foreign.iter().take(5).map(String::as_str).collect();
                    let more = foreign.len().saturating_sub(shown.len());
                    refuse(format!(
                        "target_dir {} is an earlier export of '{id}' that now also holds {}{}, \
                         which this export does not write: it has been used or changed since, \
                         so it is not replaced. Pick a new folder.",
                        target.display(),
                        shown.join(", "),
                        if more > 0 {
                            format!(" and {more} more")
                        } else {
                            String::new()
                        }
                    ))
                }
            } else {
                refuse(format!(
                    "target_dir {} is not an earlier export of '{id}', so it is never replaced. \
                     Pick a new folder.",
                    target.display()
                ))
            }
        }
    }
}

/// Content roots a model may choose for an export. The desktop folder picker
/// is the person's choice and is not subject to this tool-only allow list.
const EXPORT_CONTENT_DIRS: &[&str] = &["Desktop", "Documents", "Downloads"];

/// Folders code is loaded from by name, wherever they sit: a new package in
/// one is imported by whatever asks for that name.
const CODE_LOAD_DIRS: &[&str] = &["node_modules", "site-packages", "dist-packages"];

/// Why a model may not export to `resolved` (already resolved through any
/// link), or `None` when it may: below Desktop, Documents or Downloads, never
/// a hidden folder or a folder code is imported from. The rest
/// of the sentence after "target_dir <path> ".
///
/// ⚠ This is the `export_app` tool's rule only. A chat with Agent Drafter and
/// no developer shell could otherwise write launcher scripts and the app's
/// files into a new folder anywhere the account can write, such as a Python
/// `site-packages` under Homebrew. The desktop's Export writes where the
/// PERSON picked, and needs no such rule.
fn placement_refusal(
    resolved: &Path,
    home: Option<&Path>,
    test_roots: &[PathBuf],
) -> Option<String> {
    let named = |component: &Component, list: &[&str]| match component {
        Component::Normal(name) => {
            let name = name.to_string_lossy();
            list.iter().any(|entry| name.eq_ignore_ascii_case(entry))
        }
        _ => false,
    };
    let hidden = |component: &Component| matches!(component, Component::Normal(name) if name.to_string_lossy().starts_with('.'));
    let content_roots: Vec<PathBuf> = home
        .into_iter()
        .flat_map(|home| EXPORT_CONTENT_DIRS.iter().map(move |name| home.join(name)))
        .map(|root| resolve_existing_prefix(&root))
        .collect();
    let rest: Vec<Component> = if let Some(rest) = content_roots
        .iter()
        .find_map(|root| resolved.strip_prefix(root).ok())
    {
        if rest.as_os_str().is_empty() {
            return Some("is a content folder itself; name a new folder inside it.".to_string());
        }
        rest.components().collect()
    } else if let Some(rest) = test_roots
        .iter()
        .find_map(|root| resolved.strip_prefix(root).ok())
    {
        // Temp folders are hidden names; only the code rule applies here.
        return rest
            .components()
            .any(|c| named(&c, CODE_LOAD_DIRS))
            .then(|| "is inside a folder code is imported from.".to_string());
    } else {
        return Some(
            "is outside the export locations allowed for the tool. Export into a new folder \
             in Desktop, Documents or Downloads; to put an app anywhere else, use Export in Biorouter's \
             Applications view, where you pick the folder."
                .to_string(),
        );
    };
    if rest.iter().any(hidden) {
        return Some(
            "is inside a hidden folder, where settings and credentials live; export somewhere \
             else, such as your Desktop."
                .to_string(),
        );
    }
    if rest.iter().any(|c| named(c, CODE_LOAD_DIRS)) {
        return Some(
            "is inside a folder code is imported from (node_modules, site-packages); export \
             somewhere else, such as your Desktop."
                .to_string(),
        );
    }
    None
}

/// Names an earlier export may hold at its top level without having become
/// someone's project: what every export writes, the app's own top-level files
/// and folders, and the files a file manager leaves behind.
fn foreign_entries(earlier: &Path, app_dir: &Path) -> Vec<String> {
    let mut known: BTreeSet<String> = render::GENERATED_EXPORT_FILES
        .iter()
        .chain(render::RESERVED_EXPORT_DIRS)
        .chain(&["src", "dist", "assets"])
        .map(|name| name.to_string())
        .collect();
    if let Ok(entries) = std::fs::read_dir(app_dir) {
        known.extend(
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| !matches!(name.as_str(), "workspace" | ".git" | "node_modules")),
        );
    }
    let mut foreign: Vec<String> = std::fs::read_dir(earlier)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .filter(|name| {
                    !known.contains(name)
                        && !matches!(name.as_str(), ".DS_Store" | "Thumbs.db" | "desktop.ini")
                        && !name.starts_with("._")
                })
                .collect()
        })
        .unwrap_or_default();
    foreign.sort();
    foreign
}

/// `path` with its longest existing prefix canonicalized, so a link above the
/// target is resolved once and the rest of the path is kept as written.
fn resolve_existing_prefix(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = existing.canonicalize() {
            return rest.iter().rev().fold(real, |path, part| path.join(part));
        }
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Is `dir` an export of app `id`: a `manifest.json` naming it, beside the
/// launcher library every export writes?
fn is_export_of(dir: &Path, id: &str) -> bool {
    let is_file = |p: PathBuf| std::fs::symlink_metadata(p).is_ok_and(|meta| meta.is_file());
    if !is_file(dir.join("biorouter-launch.sh")) || !is_file(dir.join("manifest.json")) {
        return false;
    }
    std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .is_some_and(|manifest| manifest["id"].as_str() == Some(id))
}

// ---------------------------------------------------------------------------
// Full-export payload (design §3.9)
// ---------------------------------------------------------------------------

/// Every knowledge base the app reaches: its agent's `knowledge_base`, each
/// `knowledge` data source's ids, and its memory and shared-memory bases.
/// Sorted and de-duplicated. A full export with no `knowledge_bases` key
/// carries exactly these, and the desktop's dialog lists exactly these.
pub fn referenced_knowledge_bases(manifest: &Manifest) -> Vec<String> {
    let mut ids = BTreeSet::new();
    if let Some(agent) = &manifest.agent {
        ids.extend(agent.knowledge_base.iter().cloned());
        if let Some(data) = &agent.capabilities.data {
            for source in data.sources.iter().filter(|s| s.kind == "knowledge") {
                ids.extend(source.ids.iter().cloned());
            }
        }
        let memory = &agent.capabilities.memory;
        ids.extend(memory.kb.iter().cloned());
        ids.extend(memory.shared_kb.iter().cloned());
    }
    ids.into_iter().filter(|id| !id.trim().is_empty()).collect()
}

/// What `export.json` records about the payload.
#[derive(Default)]
struct Staged {
    /// `{id, file, bytes}` per staged `.brkb`.
    knowledge_bases: Vec<serde_json::Value>,
    /// `{name, path}` per staged skill directory.
    skills: Vec<serde_json::Value>,
    /// `{name, source:"registry", note}` per external extension.
    extensions: Vec<serde_json::Value>,
}

fn sorted_unique(mut items: Vec<String>) -> Vec<String> {
    items.sort();
    items.dedup();
    items
}

fn stage_payload(
    manifest: &Manifest,
    include: &ExportSelection,
    access: &dyn PayloadAccess,
    out: &Path,
    report: &mut ExportReport,
) -> Staged {
    let agent = manifest.agent.clone().unwrap_or_default();
    let kb_ids = sorted_unique(
        include
            .knowledge_bases
            .clone()
            .unwrap_or_else(|| referenced_knowledge_bases(manifest)),
    );
    let skill_names = sorted_unique(
        include
            .skills
            .clone()
            .unwrap_or_else(|| agent.skills.clone()),
    );
    let ext_names = sorted_unique(
        include
            .extensions
            .clone()
            .unwrap_or_else(|| agent.extensions.clone()),
    );
    let payload = out.join("payload");
    let mut staged = Staged::default();

    // ── Knowledge bases → payload/knowledge/<id>.brkb ──────────────────────
    if !kb_ids.is_empty() && access.knowledge_root().is_none() {
        report.notes.push(format!(
            "the knowledge store could not be opened; left out {} knowledge base(s)",
            kb_ids.len()
        ));
    } else {
        for kb in &kb_ids {
            if crate::knowledge::paths::validate_kb_id(kb).is_err() {
                report
                    .notes
                    .push(format!("'{kb}' is not a knowledge base id; skipped"));
                continue;
            }
            // Issue #56: the caller's reach, asked per base, before any byte of
            // it is read. One base out of reach costs the export only that base.
            if let Err(why) = access.reach_kb(kb) {
                report
                    .notes
                    .push(format!("skipped knowledge base '{kb}': {why}"));
                continue;
            }
            match access.export_kb(kb) {
                Ok(bytes) => {
                    let file = format!("{kb}.brkb");
                    let kdir = payload.join("knowledge");
                    let written = std::fs::create_dir_all(&kdir)
                        .and_then(|()| std::fs::write(kdir.join(&file), &bytes));
                    match written {
                        Ok(()) => {
                            staged.knowledge_bases.push(serde_json::json!({
                                "id": kb,
                                "file": format!("payload/knowledge/{file}"),
                                "bytes": bytes.len(),
                            }));
                            report.kbs.push(kb.clone());
                        }
                        Err(error) => report.notes.push(format!(
                            "could not write knowledge base '{kb}' into the payload ({error}); \
                             skipped"
                        )),
                    }
                }
                Err(why) => report
                    .notes
                    .push(format!("skipped knowledge base '{kb}': {why}")),
            }
        }
    }

    // ── Skills → payload/skills/<name>/ (a plain directory copy) ───────────
    // Never one Biorouter ships: the recipient's install seeds its own (maybe
    // newer) copy, and the launcher installing the author's over it under the
    // seeded name is the shadowing `skill_package::refuse_shipped` refuses.
    for name in &skill_names {
        if access.is_shipped_skill(name) {
            report.notes.push(format!(
                "skill '{name}' ships with Biorouter, so the recipient already has it; not \
                 carried"
            ));
            continue;
        }
        if let Some(record) = stage_skill(access, name, &payload.join("skills"), report) {
            staged.skills.push(record);
            report.skills.push(name.clone());
        }
    }

    // ── External extensions → recorded as registry references only ─────────
    // Built-ins travel with the daemon, whether the list came from the app or
    // was named explicitly; recording one as a registry install is a false
    // audit record.
    for name in &ext_names {
        if super::is_builtin_extension(name) {
            continue;
        }
        if !is_valid_item_name(name) {
            report
                .notes
                .push(format!("'{name}' is not an extension name; skipped"));
            continue;
        }
        staged.extensions.push(serde_json::json!({
            "name": name,
            "source": "registry",
            "note": "install from the BAAM registry on the target; .brxt bundle staging is out of scope",
        }));
        report.extensions.push(name.clone());
    }
    staged
}

/// Copy one skill into `payload/skills/<name>/`, or say why not.
fn stage_skill(
    access: &dyn PayloadAccess,
    name: &str,
    skills_out: &Path,
    report: &mut ExportReport,
) -> Option<serde_json::Value> {
    if !is_valid_item_name(name) {
        report
            .notes
            .push(format!("'{name}' is not a skill name; skipped"));
        return None;
    }
    let not_installed = |report: &mut ExportReport| -> Option<serde_json::Value> {
        report
            .notes
            .push(format!("skill '{name}' not installed; skipped"));
        None
    };
    let Some(location) = access.skill_dir(name) else {
        return not_installed(report);
    };
    match std::fs::symlink_metadata(&location.dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            report.notes.push(format!(
                "skill '{name}' is a link to another folder, and links are never followed; \
                 skipped"
            ));
            return None;
        }
        Ok(meta) if meta.is_dir() => {}
        _ => return not_installed(report),
    }
    // The skill folder must really sit inside its root, whatever links sit
    // above either of them.
    let (Ok(dir), Ok(root)) = (location.dir.canonicalize(), location.root.canonicalize()) else {
        return not_installed(report);
    };
    if dir == root || !dir.starts_with(&root) {
        report.notes.push(format!(
            "skill '{name}' is not inside the skills folder it was found in; skipped"
        ));
        return None;
    }
    let dst = skills_out.join(name);
    let mut budget = CopyBudget {
        files: MAX_SKILL_FILES,
        bytes: MAX_SKILL_BYTES,
    };
    let mut skipped = Vec::new();
    let copied = std::fs::create_dir_all(skills_out).and_then(|()| {
        copy_tree(
            &dir,
            &dst,
            &format!("payload/skills/{name}"),
            0,
            &mut budget,
            &mut skipped,
        )
    });
    report.skipped.extend(skipped);
    match copied {
        Ok(()) => Some(serde_json::json!({
            "name": name,
            "path": format!("payload/skills/{name}"),
        })),
        Err(error) => {
            let _ = std::fs::remove_dir_all(&dst);
            report.notes.push(format!(
                "could not copy skill '{name}' into the payload ({error}); skipped"
            ));
            None
        }
    }
}

struct CopyBudget {
    files: usize,
    bytes: u64,
}

/// Copy a tree without ever following a link, into a folder that must not
/// exist yet. Links, special files, excluded components and credential-looking
/// files are left out and listed in `skipped`.
fn copy_tree(
    src: &Path,
    dst: &Path,
    rel: &str,
    depth: usize,
    budget: &mut CopyBudget,
    skipped: &mut Vec<String>,
) -> io::Result<()> {
    std::fs::create_dir(dst)?;
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(src)?.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let shown = name.to_string_lossy();
        let child = format!("{rel}/{shown}");
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            skipped.push(format!("{child}: a link, never followed"));
        } else if kind.is_dir() {
            if EXCLUDED_COMPONENTS.contains(&shown.as_ref()) {
                skipped.push(format!("{child}/: never exported"));
            } else if depth + 1 > MAX_DEPTH {
                skipped.push(format!("{child}/: nested too deeply"));
            } else {
                copy_tree(
                    &entry.path(),
                    &dst.join(&name),
                    &child,
                    depth + 1,
                    budget,
                    skipped,
                )?;
            }
        } else if kind.is_file() {
            if looks_like_credential(&shown) {
                skipped.push(format!("{child}: looks like a credential file"));
                continue;
            }
            let len = entry.metadata()?.len();
            if budget.files == 0 || len > budget.bytes {
                return Err(io::Error::other(format!(
                    "it is larger than {MAX_SKILL_FILES} files or {} MB",
                    MAX_SKILL_BYTES / (1024 * 1024)
                )));
            }
            budget.files -= 1;
            budget.bytes -= len;
            std::fs::copy(entry.path(), dst.join(&name))?;
        } else {
            skipped.push(format!("{child}: not a regular file"));
        }
    }
    Ok(())
}

/// Assemble the deterministic `export.json` payload manifest (design §3.9 —
/// Task 3). `required_credentials` and `runtime_requirements` are empty: the app
/// manifest does not carry the extensions' declared credential env keys or
/// runtime prerequisites (those live in each extension's own bundle metadata /
/// the BAAM registry), so they cannot be enumerated here without the registry.
fn build_export_json(
    id: &str,
    mode: ExportMode,
    staged: &Staged,
    bundled_daemon: Option<&serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "version": 1,
        "app": id,
        "mode": mode.as_str(),
        "knowledge_bases": staged.knowledge_bases,
        "skills": staged.skills,
        "extensions": staged.extensions,
        "bundled_daemon": bundled_daemon,
        "required_credentials": [],
        "runtime_requirements": [],
    })
}

/// Locate a `biorouterd` binary to bundle for a fat export. This code runs
/// inside `biorouterd` (GUI) **or** `biorouter` (CLI), so `current_exe` is not
/// necessarily the daemon — we look next to it (the GUI ships them side by
/// side), then on `PATH`. `BIOROUTERD_BIN` overrides (and is the test hook).
fn find_biorouterd_binary() -> Option<PathBuf> {
    let exe_name = if cfg!(windows) {
        "biorouterd.exe"
    } else {
        "biorouterd"
    };
    if let Ok(p) = std::env::var("BIOROUTERD_BIN") {
        let pb = PathBuf::from(p);
        return pb.is_file().then_some(pb);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join(exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let cand = dir.join(exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Stage the current-platform `biorouterd` under `payload/bin/` for a fat
/// export. Returns the `export.json` record on success (or `None` → thin), plus
/// a note either way. The launchers use it only when it runs on the machine
/// they find themselves on, so a fat export opened on another platform falls
/// back to an installed daemon.
fn stage_current_daemon(target: &Path) -> (Option<serde_json::Value>, String) {
    let bin_name = if cfg!(windows) {
        "biorouterd.exe"
    } else {
        "biorouterd"
    };
    match find_biorouterd_binary() {
        Some(bin) => {
            let dst = target.join("payload").join("bin").join(bin_name);
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::copy(&bin, &dst) {
                Ok(bytes) => {
                    make_executable(&dst);
                    let rec = serde_json::json!({
                        "platform": std::env::consts::OS,
                        "arch": std::env::consts::ARCH,
                        "file": format!("payload/bin/{bin_name}"),
                        "bytes": bytes,
                    });
                    (
                        Some(rec),
                        format!(
                            "bundled the {}-{} daemon at payload/bin/{bin_name} (fat export)",
                            std::env::consts::OS,
                            std::env::consts::ARCH
                        ),
                    )
                }
                Err(e) => (
                    None,
                    format!(
                        "could not copy biorouterd into the payload ({e}); thin export instead"
                    ),
                ),
            }
        }
        None => (
            None,
            "biorouterd not found for a fat export; thin export instead (the launcher locates or \
             installs a daemon at run time)"
                .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_drafter::store::AgentConfig;
    use tempfile::TempDir;

    /// A full-mode access double: every base reachable except the ones named,
    /// bases served from a map, skills under one root.
    struct Fake {
        refused: Vec<&'static str>,
        skills_root: PathBuf,
        knowledge_root: PathBuf,
    }

    impl PayloadAccess for Fake {
        fn reach_kb(&self, kb_id: &str) -> Result<(), String> {
            if self.refused.contains(&kb_id) {
                Err("it is private".to_string())
            } else {
                Ok(())
            }
        }
        fn export_kb(&self, kb_id: &str) -> Result<Vec<u8>, String> {
            Ok(format!("brkb:{kb_id}").into_bytes())
        }
        fn knowledge_root(&self) -> Option<PathBuf> {
            Some(self.knowledge_root.clone())
        }
        fn skill_dir(&self, name: &str) -> Option<SkillLocation> {
            skill_in_root(&self.skills_root, name)
        }
    }

    struct Fixture {
        _dir: TempDir,
        store_root: PathBuf,
        work: PathBuf,
        access: Fake,
    }

    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0xFF, 0xFE, 0x80,
    ];

    fn fixture() -> Fixture {
        let dir = TempDir::new().unwrap();
        let store_root = dir.path().join("store");
        let work = dir.path().join("work");
        let skills_root = dir.path().join("skills");
        let knowledge_root = dir.path().join("knowledge");
        for d in [&store_root, &work, &skills_root, &knowledge_root] {
            std::fs::create_dir_all(d).unwrap();
        }
        Fixture {
            _dir: dir,
            store_root,
            work,
            access: Fake {
                refused: vec![],
                skills_root,
                knowledge_root,
            },
        }
    }

    impl Fixture {
        fn store(&self) -> ArtifactStore {
            ArtifactStore::new(self.store_root.clone())
        }

        /// A static app with a binary asset, workspace data, a nested vault and
        /// a few files that must never leave.
        fn app(&self, id: &str, kind: ArtifactKind) -> PathBuf {
            let store = self.store();
            let mut manifest = store
                .create_with_id(
                    id,
                    "Demo",
                    "",
                    kind,
                    "index.html",
                    &[(
                        "index.html".to_string(),
                        "<html><head></head><body><main>hi</main></body></html>".to_string(),
                    )],
                )
                .unwrap();
            manifest.session_id = Some("20261009_3".to_string());
            if kind == ArtifactKind::Agentic {
                manifest.agent = Some(AgentConfig {
                    system_prompt: "help".into(),
                    ..Default::default()
                });
            }
            // A current bundle (stamped, and written after src/ below), so no
            // build runs in a unit test and `dist/app.js` stays the fixture's.
            manifest.built_at = Some(1);
            manifest.sdk_hash = Some(bundle::sdk_fingerprint());
            store.save_manifest(&manifest).unwrap();
            let app = store.artifact_dir(id).unwrap();
            let write = |rel: &str, bytes: &[u8]| {
                let path = join_rel(&app, rel);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            };
            write("assets/logo.png", PNG);
            write("assets/theme.css", b"body{}");
            write("src/main.ts", b"export {};");
            write("src/node_modules/x.js", b"x");
            write("dist/app.js", b"console.log(1);");
            write("dist/app.css", b"main{}");
            write("dist/stale-chunk.js", b"old");
            // UTF-8 on purpose: binary bytes stayed out only by accident before.
            write("workspace/.vault/API_KEY.enc", b"sealed-as-text");
            write("workspace/notes.csv", b"a,b\n1,2\n");
            write("assets/.vault/X.enc", b"sealed");
            write("assets/.git/config", b"[remote] url=https://token@host");
            write(".env", b"SECRET=1");
            write("assets/server.pem", b"-----BEGIN");
            app
        }

        fn export(&self, id: &str, request: &ExportRequest) -> Result<ExportReport, ExportError> {
            let out = self
                .work
                .join(format!("{id}-{}", uuid::Uuid::new_v4().simple()));
            export_to_staging(&self.store_root, id, request, &self.access, &out).map(|mut r| {
                r.target = out.display().to_string();
                r
            })
        }
    }

    fn out(report: &ExportReport) -> PathBuf {
        PathBuf::from(&report.target)
    }

    #[test]
    fn binary_assets_round_trip_byte_identical_and_runtime_data_stays_home() {
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        let report = f.export("demo", &ExportRequest::default()).unwrap();
        let dir = out(&report);

        assert_eq!(std::fs::read(dir.join("assets/logo.png")).unwrap(), PNG);
        assert!(dir.join("assets/theme.css").is_file());
        assert!(dir.join("src/main.ts").is_file());
        assert_eq!(
            std::fs::read(dir.join("dist/app.js")).unwrap(),
            b"console.log(1);"
        );
        assert_eq!(std::fs::read(dir.join("dist/app.css")).unwrap(), b"main{}");
        assert!(!dir.join("dist/stale-chunk.js").exists());
        for gone in [
            "workspace",
            "assets/.vault",
            "assets/.git",
            "src/node_modules",
            ".env",
            "assets/server.pem",
        ] {
            assert!(!dir.join(gone).exists(), "{gone} must not be exported");
        }
        assert!(
            report
                .written
                .iter()
                .all(|rel| !rel.split('/').any(|c| c == ".vault")),
            "{:?}",
            report.written
        );
        assert!(report.skipped.iter().any(|s| s.starts_with("workspace/")));
        assert!(report.skipped.iter().any(|s| s.starts_with(".env")));
    }

    #[test]
    fn the_raw_entry_is_exported_and_a_served_install_has_exactly_one_config_island() {
        let f = fixture();
        f.app("agent-app", ArtifactKind::Agentic);
        let report = f
            .export(
                "agent-app",
                &ExportRequest {
                    allow_lint_errors: true,
                    ..Default::default()
                },
            )
            .unwrap();
        let dir = out(&report);
        let raw = std::fs::read_to_string(
            f.store()
                .artifact_dir("agent-app")
                .unwrap()
                .join("index.html"),
        )
        .unwrap();
        let exported = std::fs::read_to_string(dir.join("index.html")).unwrap();
        assert_eq!(
            exported, raw,
            "the export carries the authored page as written"
        );
        assert!(!exported.contains("biorouter-app-config"));

        // Install the way the launcher does, into another machine's store, and
        // serve it the way `serve_index` does.
        let other = TempDir::new().unwrap();
        let installed = other.path().join("agent-app");
        std::fs::create_dir_all(&installed).unwrap();
        for rel in ["manifest.json", "index.html"] {
            std::fs::copy(dir.join(rel), installed.join(rel)).unwrap();
        }
        let store = ArtifactStore::new(other.path().to_path_buf());
        let manifest = store.load_manifest("agent-app").unwrap();
        let entry = store.read_file("agent-app", &manifest.entry).unwrap();
        let page = render::assemble_app(
            &manifest,
            &entry,
            Some("/apps/agent-app/"),
            None,
            Some("tok123"),
        );
        assert_eq!(
            page.matches("id=\"biorouter-app-config\"").count(),
            1,
            "{page}"
        );
        assert!(page.contains("\"wsToken\":\"tok123\""));
        assert_eq!(page.matches("src=\"dist/app.js\"").count(), 1);
    }

    #[test]
    fn the_exported_manifest_drops_the_authors_chat_and_keeps_the_rest() {
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        let report = f.export("demo", &ExportRequest::default()).unwrap();
        let text = std::fs::read_to_string(out(&report).join("manifest.json")).unwrap();
        let manifest: Manifest = serde_json::from_str(&text).unwrap();
        assert_eq!(manifest.id, "demo");
        assert_eq!(manifest.session_id, None);
        assert!(!text.contains("20261009_3"));
    }

    #[cfg(unix)]
    #[test]
    fn launchers_are_executable() {
        use std::os::unix::fs::PermissionsExt;
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        let report = f.export("demo", &ExportRequest::default()).unwrap();
        for name in render::EXECUTABLE_EXPORT_FILES {
            let mode = std::fs::metadata(out(&report).join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755, "{name}");
        }
    }

    #[test]
    fn an_authored_file_named_like_a_launcher_moves_under_app_files() {
        let f = fixture();
        let app = f.app("demo", ArtifactKind::Static);
        std::fs::write(app.join("README.md"), "the app's own readme").unwrap();
        std::fs::write(app.join("run.sh"), "echo authored").unwrap();
        let report = f.export("demo", &ExportRequest::default()).unwrap();
        let dir = out(&report);
        assert!(std::fs::read_to_string(dir.join("run.sh"))
            .unwrap()
            .contains("biorouter-launch.sh"));
        assert_eq!(
            std::fs::read_to_string(dir.join("app-files/run.sh")).unwrap(),
            "echo authored"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app-files/README.md")).unwrap(),
            "the app's own readme"
        );
        assert!(report.notes.iter().any(|n| n.contains("app-files/run.sh")));
    }

    /// The store refuses an app whose folder holds a link, so the export does
    /// too, with a sentence and before anything is written. The walk below
    /// skips links on its own as well (a link made after that check), which
    /// `walk_app` is driven through directly to show.
    #[cfg(unix)]
    #[test]
    fn a_symlink_in_the_app_is_never_followed() {
        let f = fixture();
        let app = f.app("demo", ArtifactKind::Static);
        let secret = f.work.join("secret.txt");
        std::fs::write(&secret, "do not ship").unwrap();
        std::os::unix::fs::symlink(&secret, app.join("assets/linked.txt")).unwrap();
        let before: Vec<_> = std::fs::read_dir(&f.work).unwrap().flatten().collect();
        match f.export("demo", &ExportRequest::default()) {
            Err(ExportError::Unexportable(message)) => {
                assert!(message.contains("link"), "{message}")
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
        let after: Vec<_> = std::fs::read_dir(&f.work).unwrap().flatten().collect();
        assert_eq!(before.len(), after.len(), "nothing was written");

        let mut report = ExportReport::default();
        let mut files = Vec::new();
        walk_app(&app, &app, 0, &mut files, &mut report).unwrap();
        assert!(files.iter().all(|file| file.rel != "assets/linked.txt"));
        assert!(report
            .skipped
            .iter()
            .any(|s| s.contains("assets/linked.txt")));
    }

    #[test]
    fn lint_errors_block_unless_a_person_allows_them() {
        let f = fixture();
        let app = f.app("demo", ArtifactKind::Static);
        std::fs::write(
            app.join("index.html"),
            "<html><body><script src=\"https://cdn.example/x.js\"></script></body></html>",
        )
        .unwrap();
        match f.export("demo", &ExportRequest::default()) {
            Err(ExportError::Lint { findings, .. }) => assert!(!findings.is_empty()),
            other => panic!("expected a lint refusal, got {other:?}"),
        }
        let report = f
            .export(
                "demo",
                &ExportRequest {
                    allow_lint_errors: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(report.lint_warnings.iter().any(|w| w.starts_with("ERROR:")));
        assert!(report.notes.iter().any(|n| n.contains("despite")));
    }

    #[test]
    fn missing_apps_and_bad_ids_are_named_errors() {
        let f = fixture();
        assert!(matches!(
            f.export("ghost", &ExportRequest::default()),
            Err(ExportError::NotFound(_))
        ));
        assert!(matches!(
            f.export("../x", &ExportRequest::default()),
            Err(ExportError::BadRequest(_))
        ));
    }

    fn agent_with_payload(f: &Fixture, id: &str) {
        let store = f.store();
        let mut manifest = store.load_manifest(id).unwrap();
        let mut agent = AgentConfig {
            system_prompt: "help".into(),
            knowledge_base: Some("lab-notes".into()),
            skills: vec!["ggplot".into()],
            extensions: vec!["developer".into(), "spokeagent".into()],
            ..Default::default()
        };
        agent.capabilities.data = Some(crate::agent_drafter::manifest::DataCapability {
            sources: vec![serde_json::from_value(serde_json::json!({
                "name": "kb", "kind": "knowledge", "ids": ["cohort"]
            }))
            .unwrap()],
        });
        agent.capabilities.memory.kb = Some("app-memory".into());
        manifest.agent = Some(agent);
        store.save_manifest(&manifest).unwrap();
        let skill = f.access.skills_root.join("ggplot");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\nname: ggplot\n---\n").unwrap();
    }

    #[test]
    fn full_mode_carries_every_referenced_base_and_filters_built_ins() {
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        agent_with_payload(&f, "demo");
        let report = f
            .export(
                "demo",
                &ExportRequest {
                    mode: ExportMode::Full,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(report.kbs, vec!["app-memory", "cohort", "lab-notes"]);
        assert_eq!(report.skills, vec!["ggplot"]);
        assert_eq!(report.extensions, vec!["spokeagent"]);
        let dir = out(&report);
        assert!(dir.join("payload/skills/ggplot/SKILL.md").is_file());
        assert!(dir.join("payload/knowledge/cohort.brkb").is_file());
        let ej: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("export.json")).unwrap())
                .unwrap();
        assert_eq!(ej["mode"], "full");
        assert_eq!(ej["extensions"].as_array().unwrap().len(), 1);

        // Named explicitly, a built-in is still not a registry install.
        let explicit = f
            .export(
                "demo",
                &ExportRequest {
                    mode: ExportMode::Full,
                    include: ExportSelection {
                        knowledge_bases: Some(vec![]),
                        skills: Some(vec![]),
                        extensions: Some(vec!["developer".into(), "autovisualiser".into()]),
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(explicit.extensions.is_empty());
    }

    /// A full export carried `office-word` (a skill every install seeds), and
    /// the recipient's launcher then installed the author's copy over theirs
    /// under the seeded name, which `skill_package::refuse_shipped` exists to
    /// stop. Named by the app or by the request, a shipped skill stays home.
    #[test]
    fn a_skill_biorouter_ships_never_travels() {
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        agent_with_payload(&f, "demo");
        let store = f.store();
        let mut manifest = store.load_manifest("demo").unwrap();
        manifest.agent.as_mut().unwrap().skills = vec![
            "ggplot".into(),
            "office-word".into(),
            "knowledge-bases".into(),
        ];
        store.save_manifest(&manifest).unwrap();
        let seeded = f.access.skills_root.join("office-word");
        std::fs::create_dir_all(&seeded).unwrap();
        std::fs::write(seeded.join("SKILL.md"), "---\nname: office-word\n---\n").unwrap();

        for skills in [
            None,
            Some(vec!["office-word".to_string()]),
            Some(vec!["Office-Word".to_string()]),
            Some(vec!["knowledge-bases".to_string()]),
        ] {
            let report = f
                .export(
                    "demo",
                    &ExportRequest {
                        mode: ExportMode::Full,
                        include: ExportSelection {
                            skills: skills.clone(),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(
                !report.skills.contains(&"office-word".to_string()),
                "{skills:?}"
            );
            for name in ["office-word", "Office-Word", "knowledge-bases"] {
                assert!(!out(&report).join(format!("payload/skills/{name}")).exists());
            }
            assert!(
                report
                    .notes
                    .iter()
                    .any(|note| note.contains("ships with Biorouter")),
                "{:?}",
                report.notes
            );
            let ej: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(out(&report).join("export.json")).unwrap(),
            )
            .unwrap();
            assert!(!ej.to_string().contains("payload/skills/office-word"));
        }
    }

    /// [`render::SHIPPED_SKILL_NAMES`] is a copy: the seeder's list lives in the
    /// `biorouter` crate (`skills_extension::is_builtin_skill_name`), which
    /// this crate cannot import. Read that source and hold the two equal, so a
    /// skill shipped later is never exported over the recipient's.
    #[test]
    fn the_shipped_skill_list_is_the_seeders() {
        let biorouter = Path::new(env!("CARGO_MANIFEST_DIR")).join("../biorouter/src");
        let (Ok(seeder), Ok(soul)) = (
            std::fs::read_to_string(biorouter.join("agents/skills_extension.rs")),
            std::fs::read_to_string(biorouter.join("knowledge/soul.rs")),
        ) else {
            eprintln!("the biorouter crate is not beside this one; nothing to compare");
            return;
        };
        let names_in = |start: &str| -> Vec<String> {
            let (_, rest) = seeder.split_once(start).expect("the list is still there");
            let (list, _) = rest.split_once("];").expect("...and still ends");
            list.split('"')
                .skip(1)
                .step_by(2)
                .filter(|quoted| !quoted.contains('/'))
                .map(str::to_string)
                .collect()
        };
        let mut seeded = names_in("pub static BUILTIN_SKILLS: &[(&str, &str)] = &[");
        seeded.extend(names_in(
            "pub static KNOWLEDGE_SKILLS: &[(&str, &str)] = &[",
        ));
        let (_, rest) = soul
            .split_once("pub const SOUL_SKILL_DIR: &str = \"")
            .expect("SOUL_SKILL_DIR is still there");
        seeded.push(rest.split('"').next().unwrap().to_string());
        let (_, rest) = seeder
            .split_once("pub const KNOWLEDGE_BUNDLE: &str = \"")
            .expect("KNOWLEDGE_BUNDLE is still there");
        seeded.push(rest.split('"').next().unwrap().to_string());
        assert!(seeded.len() > 10, "{seeded:?}");
        assert_eq!(seeded, render::SHIPPED_SKILL_NAMES);
    }

    #[test]
    fn an_explicit_empty_selection_stages_nothing() {
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        agent_with_payload(&f, "demo");
        let report = f
            .export(
                "demo",
                &ExportRequest {
                    mode: ExportMode::Full,
                    include: ExportSelection {
                        knowledge_bases: Some(vec![]),
                        skills: Some(vec![]),
                        extensions: Some(vec![]),
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(report.kbs.is_empty() && report.skills.is_empty() && report.extensions.is_empty());
        assert!(!out(&report).join("payload").exists());
        assert!(
            report.wrote_export_json,
            "a full export still records what it carried"
        );
    }

    #[test]
    fn a_base_out_of_the_callers_reach_is_left_out_with_a_note() {
        let mut f = fixture();
        f.access.refused = vec!["cohort"];
        f.app("demo", ArtifactKind::Static);
        agent_with_payload(&f, "demo");
        let report = f
            .export(
                "demo",
                &ExportRequest {
                    mode: ExportMode::Full,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!report.kbs.contains(&"cohort".to_string()));
        assert!(!out(&report).join("payload/knowledge/cohort.brkb").exists());
        assert!(report
            .notes
            .iter()
            .any(|n| n.contains("cohort") && n.contains("private")));
    }

    #[test]
    fn skill_names_that_are_paths_are_refused_before_any_join() {
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        // Something a traversal would find, beside the skills root.
        let outside = f.access.skills_root.parent().unwrap().join("secrets");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("SKILL.md"), "secret").unwrap();
        for name in ["../secrets", "/etc", "a/b", "..", ".hidden", ""] {
            let report = f
                .export(
                    "demo",
                    &ExportRequest {
                        mode: ExportMode::Full,
                        include: ExportSelection {
                            skills: Some(vec![name.to_string()]),
                            knowledge_bases: Some(vec![]),
                            extensions: Some(vec![]),
                        },
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(report.skills.is_empty(), "{name}");
            assert!(!out(&report).join("payload").exists(), "{name}");
            assert!(
                !f.work.join("secrets").exists(),
                "{name} wrote outside the export"
            );
        }
        assert_eq!(
            std::fs::read_to_string(outside.join("SKILL.md")).unwrap(),
            "secret"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_skill_and_a_link_loop_inside_a_skill_are_skipped_and_the_copy_ends() {
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        let real = f.work.join("elsewhere");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("SKILL.md"), "x").unwrap();
        std::os::unix::fs::symlink(&real, f.access.skills_root.join("linked")).unwrap();
        let looped = f.access.skills_root.join("looped");
        std::fs::create_dir_all(&looped).unwrap();
        std::fs::write(looped.join("SKILL.md"), "---\nname: looped\n---\n").unwrap();
        std::os::unix::fs::symlink(&looped, looped.join("again")).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", looped.join("hosts")).unwrap();

        let report = f
            .export(
                "demo",
                &ExportRequest {
                    mode: ExportMode::Full,
                    include: ExportSelection {
                        skills: Some(vec!["linked".into(), "looped".into()]),
                        knowledge_bases: Some(vec![]),
                        extensions: Some(vec![]),
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(report.skills, vec!["looped"]);
        let dir = out(&report);
        assert!(dir.join("payload/skills/looped/SKILL.md").is_file());
        assert!(!dir.join("payload/skills/looped/again").exists());
        assert!(!dir.join("payload/skills/looped/hosts").exists());
        assert!(!dir.join("payload/skills/linked").exists());
        assert!(report
            .notes
            .iter()
            .any(|n| n.contains("'linked'") && n.contains("link")));
    }

    #[test]
    fn a_bundled_skill_is_found_one_level_down() {
        let f = fixture();
        let skill = f.access.skills_root.join("knowledge-bases").join("okf");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "x").unwrap();
        let found = skill_in_root(&f.access.skills_root, "okf").unwrap();
        assert_eq!(found.dir, skill);
        assert!(skill_in_root(&f.access.skills_root, "../okf").is_none());
    }

    #[test]
    fn a_model_named_target_must_be_new_or_empty_unless_it_replaces_its_own_export() {
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        let access = &f.access;
        let target = f.work.join("Desktop").join("demo");

        // Relative and parent-escaping targets are refused.
        let escaping = f.work.join("x/../demo");
        for bad in [Path::new("demo"), escaping.as_path()] {
            assert!(matches!(
                export_to_dir(
                    &f.store_root,
                    "demo",
                    &ExportRequest::default(),
                    access,
                    bad,
                    false
                ),
                Err(ExportError::BadRequest(_))
            ));
        }
        // Inside the store or the knowledge store.
        for inside in [
            f.store_root.join("demo-out"),
            access.knowledge_root.join("x"),
        ] {
            assert!(matches!(
                export_to_dir(
                    &f.store_root,
                    "demo",
                    &ExportRequest::default(),
                    access,
                    &inside,
                    false
                ),
                Err(ExportError::BadRequest(_))
            ));
        }

        let first = export_to_dir(
            &f.store_root,
            "demo",
            &ExportRequest::default(),
            access,
            &target,
            false,
        )
        .unwrap();
        assert_eq!(first.target, target.display().to_string());
        assert!(target.join("run.sh").is_file());
        // Nothing half-written is left beside it.
        let siblings: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(siblings, vec!["demo"]);

        // Non-empty: refused without replace.
        assert!(matches!(
            export_to_dir(
                &f.store_root,
                "demo",
                &ExportRequest::default(),
                access,
                &target,
                false
            ),
            Err(ExportError::BadRequest(_))
        ));
        // A stale file from the earlier export does not survive a replace,
        // and the earlier export itself is kept beside the new one, as it was.
        std::fs::write(target.join("src/stale.ts"), "old").unwrap();
        std::fs::write(target.join("src/main.ts"), "// edited in the export").unwrap();
        let replaced = export_to_dir(
            &f.store_root,
            "demo",
            &ExportRequest::default(),
            access,
            &target,
            true,
        )
        .unwrap();
        assert!(!target.join("src/stale.ts").exists());
        assert!(target.join("manifest.json").is_file());
        let kept: Vec<PathBuf> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("demo.replaced-")
            })
            .collect();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(
            std::fs::read_to_string(kept[0].join("src/main.ts")).unwrap(),
            "// edited in the export"
        );
        assert!(kept[0].join("src/stale.ts").is_file());
        assert!(
            replaced
                .notes
                .iter()
                .any(|note| note.contains(&kept[0].display().to_string())),
            "{:?}",
            replaced.notes
        );
        std::fs::remove_dir_all(&kept[0]).unwrap();

        // An export that has become someone's project is never replaced:
        // a repository, installed packages, a file of their own.
        std::fs::write(target.join(".DS_Store"), "finder").unwrap();
        for (made, rel) in [
            ("dir", ".git"),
            ("dir", "node_modules"),
            ("file", "package-lock.json"),
            ("file", "notes.md"),
        ] {
            let path = target.join(rel);
            if made == "dir" {
                std::fs::create_dir_all(path.join("x")).unwrap();
            } else {
                std::fs::write(&path, "mine").unwrap();
            }
            let refused = export_to_dir(
                &f.store_root,
                "demo",
                &ExportRequest::default(),
                access,
                &target,
                true,
            );
            match refused {
                Err(ExportError::BadRequest(message)) => {
                    assert!(message.contains(rel), "{message}");
                    assert!(!message.contains(".DS_Store"), "{message}");
                }
                other => panic!("{rel}: {other:?}"),
            }
            assert!(path.exists(), "{rel} is left where it is");
            if made == "dir" {
                std::fs::remove_dir_all(&path).unwrap();
            } else {
                std::fs::remove_file(&path).unwrap();
            }
        }
        // Finder's own file does not make it a project.
        export_to_dir(
            &f.store_root,
            "demo",
            &ExportRequest::default(),
            access,
            &target,
            true,
        )
        .unwrap();

        // Replace never touches a folder that is not this app's export.
        let other = f.work.join("Desktop").join("projects");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("thesis.tex"), "precious").unwrap();
        assert!(matches!(
            export_to_dir(
                &f.store_root,
                "demo",
                &ExportRequest::default(),
                access,
                &other,
                true
            ),
            Err(ExportError::BadRequest(_))
        ));
        assert_eq!(
            std::fs::read_to_string(other.join("thesis.tex")).unwrap(),
            "precious"
        );

        // An empty folder is fine.
        let empty = f.work.join("Desktop").join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        export_to_dir(
            &f.store_root,
            "demo",
            &ExportRequest::default(),
            access,
            &empty,
            false,
        )
        .unwrap();
        assert!(empty.join("index.html").is_file());
    }

    /// `export_app` took any absolute folder the account could write that was
    /// not one of three Biorouter folders or a hidden one at the top of home,
    /// so a chat with no shell could plant a package in a Python
    /// `site-packages` or files in `~/Library`. It writes only into the
    /// person's own folders now.
    #[cfg(unix)]
    #[test]
    fn a_model_exports_only_into_the_persons_own_folders() {
        let home = Path::new("/home/u");
        let tests = [PathBuf::from("/tmp/T")];
        let verdict = |path: &str| placement_refusal(Path::new(path), Some(home), &tests);
        for allowed in [
            "/home/u/Desktop/my-app",
            "/home/u/Documents/apps/my-app",
            "/home/u/Downloads/my-app",
            "/tmp/T/.tmpAbC/work/my-app",
        ] {
            assert_eq!(verdict(allowed), None, "{allowed}");
        }
        for (refused, because) in [
            ("/home/u", "outside the export locations"),
            ("/home/u/Desktop", "content folder itself"),
            (
                "/home/u/code/project/exports/my-app",
                "outside the export locations",
            ),
            ("/Volumes/USB Stick/my-app", "outside the export locations"),
            ("/media/u/usb/my-app", "outside the export locations"),
            (
                "/home/u/.config/biorouter-ish",
                "outside the export locations",
            ),
            ("/home/u/Desktop/.secret/my-app", "hidden"),
            (
                "/home/u/Library/LaunchAgents/my-app",
                "outside the export locations",
            ),
            ("/home/u/library/my-app", "outside the export locations"),
            (
                "/home/u/AppData/Roaming/x/my-app",
                "outside the export locations",
            ),
            (
                "/home/u/snap/firefox/my-app",
                "outside the export locations",
            ),
            (
                "/home/u/Documents/project/node_modules/my-app",
                "imported from",
            ),
            (
                "/home/u/Documents/py/lib/python3.12/site-packages/my-app",
                "imported from",
            ),
            (
                "/opt/homebrew/lib/python3.12/site-packages/my-app",
                "outside the export locations",
            ),
            ("/usr/local/my-app", "outside the export locations"),
            ("/etc/my-app", "outside the export locations"),
            ("/Volumes/USB", "outside the export locations"),
            ("/tmp/other/my-app", "outside the export locations"),
            ("/tmp/T/.tmpAbC/node_modules/my-app", "imported from"),
        ] {
            let why = verdict(refused).unwrap_or_else(|| panic!("{refused} was allowed"));
            assert!(why.contains(because), "{refused}: {why}");
        }
        // No home at all: only the drives and the test folders remain.
        assert!(placement_refusal(Path::new("/home/u/Desktop/x"), None, &tests).is_some());
    }

    #[test]
    fn source_project_metadata_does_not_permit_replacing_an_edited_export() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app");
        let earlier = root.path().join("earlier");
        for dir in [&app, &earlier] {
            std::fs::create_dir_all(dir.join(".git")).unwrap();
            std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        }
        assert_eq!(
            foreign_entries(&earlier, &app),
            vec![".git", "node_modules"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_target_that_is_a_link_is_refused() {
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        let real = f.work.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = f.work.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(matches!(
            export_to_dir(
                &f.store_root,
                "demo",
                &ExportRequest::default(),
                &f.access,
                &link,
                false
            ),
            Err(ExportError::BadRequest(_))
        ));
        // ...including a link above the target into a protected place.
        let into_store = f.work.join("into-store");
        std::os::unix::fs::symlink(&f.store_root, &into_store).unwrap();
        assert!(matches!(
            export_to_dir(
                &f.store_root,
                "demo",
                &ExportRequest::default(),
                &f.access,
                &into_store.join("new"),
                false
            ),
            Err(ExportError::BadRequest(_))
        ));
    }

    #[test]
    fn the_legacy_map_carries_the_raw_entry_and_lists_what_it_cannot_carry() {
        let f = fixture();
        f.app("demo", ArtifactKind::Static);
        let legacy = legacy_export_map(&f.store_root, "demo").unwrap();
        let file = |rel: &str| {
            legacy
                .files
                .iter()
                .find(|(p, _)| p == rel)
                .map(|(_, c)| c.clone())
        };
        assert!(!file("index.html").unwrap().contains("biorouter-app-config"));
        assert!(file("run.sh").is_some());
        assert!(!file("manifest.json").unwrap().contains("20261009_3"));
        assert!(file("workspace/notes.csv").is_none());
        assert_eq!(legacy.skipped_binary, vec!["assets/logo.png"]);
    }

    #[test]
    fn selection_json_keeps_absent_and_empty_apart() {
        let none = ExportSelection::from_json(None).unwrap();
        assert_eq!(none, ExportSelection::default());
        let explicit = ExportSelection::from_json(Some(&serde_json::json!({
            "knowledge_bases": [], "skills": ["a"]
        })))
        .unwrap();
        assert_eq!(explicit.knowledge_bases, Some(vec![]));
        assert_eq!(explicit.skills, Some(vec!["a".to_string()]));
        assert_eq!(explicit.extensions, None);
        assert!(ExportSelection::from_json(Some(&serde_json::json!({"skills": [1]}))).is_err());
        assert!(ExportSelection::from_json(Some(&serde_json::json!(["x"]))).is_err());
    }

    /// The desktop keeps two lists of its own that must agree with this crate:
    /// the built-in extensions its export dialog leaves out, and the paths its
    /// unpacker makes executable whatever the zip recorded. Read from the real
    /// files; skipped when the crate is built outside the repository.
    #[test]
    fn the_desktop_mirrors_of_the_export_lists_agree() {
        let desktop = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/desktop/src");
        let (Ok(management), Ok(unpacker)) = (
            std::fs::read_to_string(desktop.join("components/applications/appManagement.ts")),
            std::fs::read_to_string(desktop.join("utils/appExport.ts")),
        ) else {
            eprintln!("ui/desktop is not beside this crate; nothing to compare");
            return;
        };
        let quoted = |text: &str, start: &str| -> Vec<String> {
            let (_, rest) = text.split_once(start).expect("the list is still there");
            let (list, _) = rest.split_once(']').expect("...and still ends");
            list.split('\'')
                .skip(1)
                .step_by(2)
                .map(str::to_string)
                .collect()
        };
        let builtins = quoted(
            &management,
            "export const BUILTIN_EXTENSIONS: readonly string[] = [",
        );
        assert_eq!(builtins, super::super::BUILTIN_EXTENSION_NAMES);
        let shipped = quoted(
            &management,
            "export const SHIPPED_SKILLS: readonly string[] = [",
        );
        assert_eq!(shipped, render::SHIPPED_SKILL_NAMES);
        let executable = quoted(&unpacker, "export const EXECUTABLE_EXPORT_PATHS");
        let mut expected: Vec<String> = render::EXECUTABLE_EXPORT_FILES
            .iter()
            .map(|name| name.to_string())
            .collect();
        expected.push("payload/bin/biorouterd".to_string());
        assert_eq!(executable, expected);
        assert!(expected.iter().all(|rel| is_executable_export_path(rel)));
    }

    #[test]
    fn the_report_header_stays_small_however_much_was_skipped() {
        let mut report = ExportReport::new("demo", ExportMode::Full);
        report.skipped = (0..5_000)
            .map(|i| format!("assets/file-{i}.bin: a link"))
            .collect();
        report.notes = vec!["x".repeat(20_000)];
        let header = report.header_json().to_string();
        assert!(header.len() <= 6_100, "{}", header.len());
        assert!(header.contains("more line(s)"));
    }
}
