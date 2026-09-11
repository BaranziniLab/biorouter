//! `/headless/*` — the browser surface's bridge to the machine the daemon runs on.
//!
//! # Why the path is still called `headless`
//!
//! These routes were served by a separate binary, `biorouter-headless`,
//! which sat in front of the daemon: it served the single-page application,
//! reverse-proxied `/api/*` to `biorouterd`, and answered `/headless/*` itself
//! with a re-implementation of the Electron preload's IPC surface — browse the
//! filesystem, read and write a file, keep the interface's own settings,
//! install a `.brxt` extension bundle, fetch a marketplace asset. The daemon now serves the interface itself (see
//! [`crate::routes::web_ui`]) and that binary is retired, so the handlers moved
//! here.
//!
//! **The `/headless` prefix is retained deliberately.** The renderer builds its
//! endpoint base as `origin + '/headless'` (`ui/desktop/src/renderer.tsx`;
//! `web_ui::inject_runtime_config` injects exactly `"headlessBaseUrl":
//! "/headless"`), so keeping the paths byte-identical makes this move invisible
//! to the browser. Renaming the prefix would be a renderer change for no gain —
//! the name is a URL, not a claim about which process is answering.
//!
//! # The security model, and how it differs from the binary this replaces
//!
//! The original was **unauthenticated** — it bound `0.0.0.0` by default and
//! nothing in front of `/headless/*` checked anything. On top of that, its
//! filesystem handlers had no path validation worth the name: `fs_read` took a
//! query parameter, expanded `~`, and returned the file; `validate_file_path`
//! refused exactly two strings, `""` and `/`. So `GET
//! /headless/fs/read?path=/etc/passwd` worked, and so did
//! `path=~/.ssh/id_rsa`.
//!
//! Two things close that here.
//!
//! **The secret-key middleware.** These routes are merged inside
//! `routes::configure`, so `check_token` (applied in `commands::agent`) wraps
//! them and `auth::is_unauthenticated_path` does not name them. A caller must
//! present `X-Secret-Key`, exactly as for `/config` or `/sessions`.
//!
//! **[`PathGuard`] — an allowlist of roots, checked after canonicalization.**
//! Every client-supplied path goes through [`PathGuard::resolve`], which
//! returns the *resolved* path or a refusal, and every handler operates on the
//! resolved path rather than on the string it was handed. The roots are the
//! Biorouter config directory, the user's home directory, the process working
//! directory and the system temporary directory (that last one is not
//! decoration: `registry/download` writes an asset there and hands the path
//! back for `brxt/install` to read, so excluding it would break the marketplace
//! flow). Resolution follows the canonicalize-then-`starts_with` shape this
//! surface already used for zip entries in `safe_entry_target`, extended in
//! three ways it needed:
//!
//! * `..` is folded out **before** the root test, so `~/../../etc/passwd` is
//!   compared as `/etc/passwd` rather than as something that lexically begins
//!   with the home directory.
//! * A path that does not exist yet — the write and ensure-directory cases —
//!   has its deepest **existing** ancestor canonicalized, and the remaining
//!   components appended to that. So a new file inherits its parent's real
//!   identity instead of skipping the check.
//! * Because canonicalization resolves symbolic links, a link inside a root
//!   that points outside one is refused: the comparison is against the link's
//!   target, never against the requested string.
//! * A link that does **not** resolve — its target is missing, or it is part
//!   of a loop — is refused wherever it sits in the path. It has no target to
//!   compare, and treating it as a name not created yet compared the link's own
//!   location instead, which made every handler an oracle for whether its
//!   target existed and let `fs_write` create that target (QA-D F6).
//!
//! The two reading routes then open the resolved path without following a
//! link and check that the descriptor is the file the guard validated
//! ([`open_validated_file`]), so a name flipped to a link after the check is
//! refused rather than read.
//!
//! An allowlist alone is not enough, because the most sensitive things on the
//! machine live *inside* the home directory. [`is_denied`] additionally refuses
//! the credential stores — `.ssh`, `.gnupg`, `.aws`, `.kube`, `.docker` — and
//! `secrets.yaml`, which is Biorouter's own plaintext secret store and is the
//! *normal* case on headless Linux, where the keyring is unavailable and
//! `BIOROUTER_DISABLE_KEYRING` is the default. Nothing legitimate on this
//! surface reads any of them; secrets reach the interface through the `/config`
//! routes.
//!
//! # What did not come across
//!
//! The reverse proxy, the static-file serving, the index rewriting and the
//! `biorouterd` child-process spawn are gone rather than ported: the daemon is
//! the process now, so there is nothing to proxy to and nothing to spawn.
//! [`crate::routes::web_ui`] serves the shell.
//!
//! # Why there are no `utoipa` annotations
//!
//! The renderer reaches these with hand-written `fetch` calls through its
//! headless bridge, not through the generated OpenAPI client, so a spec entry
//! would describe a door no generated code opens. `tool_bridge` is absent from
//! the spec for the same reason.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use axum::{
    extract::{DefaultBodyLimit, Query},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine as _;
use biorouter::config::paths::Paths;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::process::Command;

use crate::state::AppState;

/// Ceiling on a request body to these routes.
///
/// The binary this replaces put `usize::MAX` on the one body it read by hand
/// (the proxy's), and left everything else on axum's implicit default — so the
/// limit on this surface was never stated anywhere. It is stated here. Eight
/// mebibytes is far above anything the interface actually sends (a settings
/// document, a skill file, a workflow) and far below a body worth buffering.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Ceiling on a marketplace asset download.
const MAX_REGISTRY_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;

/// Ceiling on the total **uncompressed** size of an archive this surface
/// unpacks. The original read every entry into memory with no bound at all, so
/// a 200 MB download that expanded a thousandfold was a way to exhaust the
/// daemon. Enforced while reading, not from the archive's own declared sizes,
/// which an attacker writes.
const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;

/// How long `uv sync` may run while installing an extension bundle.
const UV_SYNC_TIMEOUT: Duration = Duration::from_secs(600);

/// Hosts a marketplace asset may be fetched from.
const REGISTRY_DOWNLOAD_HOSTS: &[&str] = &[
    "biorouter.ucsf.edu",
    "github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
    "codeload.github.com",
];

/// Directory names that are refused wherever they appear in a resolved path,
/// including inside an allowed root. These are credential stores; the interface
/// has no reason to read or write one, and an agent that can drive this surface
/// has every reason to want to.
const DENIED_DIR_NAMES: &[&str] = &[".ssh", ".gnupg", ".gpg", ".aws", ".kube", ".docker"];

/// File names that are refused wherever they appear. `secrets.yaml` is
/// Biorouter's plaintext secret store, which is what a headless Linux
/// deployment uses when no OS keyring is available — i.e. exactly the
/// deployment this surface exists for.
const DENIED_FILE_NAMES: &[&str] = &["secrets.yaml", "id_rsa", "id_ed25519", "id_ecdsa", "id_dsa"];

/// One client for the marketplace fetches, built once. The daemon's `reqwest`
/// is rustls-only, so this carries no system TLS stack with it.
static REGISTRY_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

// ---------------------------------------------------------------------------
// Wire types. Field names and casing are the originals: the renderer's bridge
// reads them directly, so a rename here is a silent break there.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PathQuery {
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListFilesQuery {
    pub path: Option<String>,
    pub extension: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WriteFileRequest {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct FsPathRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct RegistryDownloadRequest {
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct RegistryDownloadResponse {
    pub path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePathRequest {
    pub file_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrxtInstallRequest {
    pub file_path: String,
    pub extension_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrxtUninstallRequest {
    pub extension_name: String,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub home_dir: String,
    pub api_base_url: String,
}

#[derive(Debug, Serialize)]
pub struct RootsResponse {
    pub roots: Vec<FsRoot>,
}

#[derive(Debug, Serialize)]
pub struct FsRoot {
    pub label: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct DirsResponse {
    pub dirs: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct FilesResponse {
    pub files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct FsOkResponse {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    pub settings: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct ReadFileResponse {
    #[serde(rename = "filePath")]
    pub file_path: String,
    pub file: String,
    pub found: bool,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FsListResponse {
    pub path: String,
    pub entries: Vec<FsEntry>,
}

#[derive(Debug, Serialize)]
pub struct FsEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "isDir")]
    pub is_dir: bool,
    #[serde(rename = "isFile")]
    pub is_file: bool,
}

struct ArchiveEntry {
    name: String,
    is_dir: bool,
    data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// The path guard.
// ---------------------------------------------------------------------------

/// Why a requested path was not resolved.
///
/// Deliberately carries no path: a refusal that echoed the resolved location
/// would report whether `/etc/shadow` exists to a caller who is not allowed to
/// read it. Four variants rather than one because two of them are the caller's
/// mistake (`400`) and two are a boundary (`403`). The two boundaries share one
/// message as well as one status; [`Refusal::message`] says why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The path was empty, or `~` with no resolvable home directory.
    Unusable,
    /// `..` walked above the filesystem root.
    Malformed,
    /// The resolved path is not inside any allowed root.
    Outside,
    /// The resolved path is inside an allowed root but names a credential
    /// store.
    Denied,
}

impl Refusal {
    fn status(self) -> StatusCode {
        match self {
            Refusal::Unusable | Refusal::Malformed => StatusCode::BAD_REQUEST,
            Refusal::Outside | Refusal::Denied => StatusCode::FORBIDDEN,
        }
    }

    /// One sentence for both boundaries, on purpose. Which of the two applies is
    /// decided by where a path resolves, and a caller who can plant a link
    /// inside a root chooses that: a link to an existing file outside every
    /// root is `Outside`, and the same link to a missing one is `Denied` (a link
    /// that does not resolve). Two wordings would answer "does this exist" about
    /// every path the guard refuses (QA-D F6). The variants stay distinct for
    /// the tests, which assert which boundary held; a caller learns only that
    /// one did. The interface never shows this text anyway: its headless bridge
    /// reads any non-2xx as "no data".
    fn message(self) -> &'static str {
        match self {
            Refusal::Unusable => "path is empty or could not be resolved",
            Refusal::Malformed => "path escapes the filesystem root",
            Refusal::Outside | Refusal::Denied => {
                "path is outside the directories this server may touch, names a credential \
                 store, or goes through a link this server will not follow"
            }
        }
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        (
            self.status(),
            Json(serde_json::json!({ "error": self.message() })),
        )
            .into_response()
    }
}

/// The set of directories this surface may touch, plus what it needs to turn a
/// client string into a path inside one of them.
///
/// Constructed per request by [`PathGuard::current`] rather than cached, so a
/// process that changes its working directory cannot be left checking against a
/// stale root. Tests construct one directly over a temporary directory, which
/// is what makes the boundary testable without touching process-wide state.
pub struct PathGuard {
    /// Canonical roots. A resolved path must be one of these or live under one.
    roots: Vec<PathBuf>,
    /// What `~` means. `None` when no home directory could be determined, in
    /// which case `~` is refused rather than silently treated as a literal.
    home: Option<PathBuf>,
    /// What a relative path is relative to.
    cwd: PathBuf,
}

impl PathGuard {
    /// The guard the handlers run under.
    pub fn current() -> Self {
        let home = home_dir();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut roots = vec![Paths::config_dir(), std::env::temp_dir(), cwd.clone()];
        if let Some(home) = home.clone() {
            roots.push(home);
        }
        Self {
            // A root that is itself a link to nothing names no directory, and
            // nothing under it could be resolved anyway, so it is not a root.
            roots: roots
                .iter()
                .filter_map(|root| canonical_prefix(root).ok())
                .collect(),
            home,
            cwd,
        }
    }

    /// Resolve a client-supplied path, or say why not.
    ///
    /// The returned path is the one a handler must act on. Acting on the
    /// requested string instead would re-open every hole this closes, because
    /// the string and the resolved path differ exactly when it matters.
    pub fn resolve(&self, requested: &str) -> Result<PathBuf, Refusal> {
        let expanded = self.expand(requested)?;
        let absolute = if expanded.is_absolute() {
            expanded
        } else {
            self.cwd.join(expanded)
        };
        // Fold `..` out first: `~/../../etc/passwd` must be compared as
        // `/etc/passwd`, not as a string that begins with the home directory.
        let normalized = lexical_normalize(&absolute).ok_or(Refusal::Malformed)?;
        // Then resolve symbolic links, so a link inside a root that points out
        // of one is compared as its target — and a link with no target to
        // compare is refused here, before any handler can follow it.
        let resolved = canonical_prefix(&normalized)?;
        if !self
            .roots
            .iter()
            .any(|root| resolved == *root || resolved.starts_with(root))
        {
            return Err(Refusal::Outside);
        }
        if is_denied(&resolved) {
            return Err(Refusal::Denied);
        }
        Ok(resolved)
    }

    /// Is this resolved path one of the roots itself? Deleting a root is
    /// refused however the caller spelled it.
    fn is_root(&self, resolved: &Path) -> bool {
        self.roots.iter().any(|root| root == resolved)
    }

    fn expand(&self, requested: &str) -> Result<PathBuf, Refusal> {
        if requested.is_empty() {
            return Err(Refusal::Unusable);
        }
        if requested == "~" {
            return self.home.clone().ok_or(Refusal::Unusable);
        }
        if let Some(rest) = requested.strip_prefix("~/") {
            let home = self.home.as_ref().ok_or(Refusal::Unusable)?;
            return Ok(home.join(rest));
        }
        Ok(PathBuf::from(requested))
    }
}

/// Fold `.` and `..` out of an absolute path textually.
///
/// `None` when `..` would walk above the root — a caller who wrote that is not
/// naming anything this server can serve. Purely lexical on purpose: it runs
/// *before* [`canonical_prefix`], whose job is the symlink half. Doing it in
/// this order is what stops a lexical prefix match from being the whole check.
fn lexical_normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                out.pop();
            }
            Component::Normal(part) => {
                depth += 1;
                out.push(part);
            }
        }
    }
    Some(out)
}

/// Canonicalize as much of `path` as exists, and re-append the rest.
///
/// A write or a directory creation names something that is not there yet, so a
/// plain `canonicalize` would fail and leave the caller with a choice between
/// trusting the string and refusing every write. Canonicalizing the deepest
/// existing ancestor gives the new path its parent's real identity — including
/// through any symbolic link on the way — which is the property the root test
/// needs.
///
/// **A link that does not resolve is refused, never stepped past.** Its target
/// may be missing, or it may be one link of a loop; either way `canonicalize`
/// fails on it exactly as it fails on a name that is not there yet, and this
/// walk used to treat the two alike, re-appending the link's own name to its
/// parent. What came back was then the link's location — inside a root, naming
/// nothing denied — while every handler that acted on it followed the link to
/// wherever it pointed: `fs_read` answered `200 {"found":false}` for a link
/// into `~/.ssh` whose target was missing and `403` for one whose target was
/// there (QA-D F6), and `fs_write` created the missing target. The root test and
/// the deny list have to see a link's target, a link that does not resolve
/// gives them none to see, and so it is [`Refusal::Denied`] — "a link this
/// server will not follow" — wherever in the path it sits.
fn canonical_prefix(path: &Path) -> Result<PathBuf, Refusal> {
    let mut tail: Vec<OsString> = Vec::new();
    let mut probe = path.to_path_buf();
    loop {
        if let Ok(canonical) = probe.canonicalize() {
            let mut out = canonical;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return Ok(out);
        }
        // `symlink_metadata` follows every component but the last, so a link
        // higher up the path fails this probe as "not found" and is met here
        // as the walk climbs to it.
        if std::fs::symlink_metadata(&probe).is_ok_and(|named| named.file_type().is_symlink()) {
            return Err(Refusal::Denied);
        }
        let name = probe.file_name().map(OsStr::to_os_string);
        let parent = probe.parent().map(Path::to_path_buf);
        match (name, parent) {
            (Some(name), Some(parent)) if !parent.as_os_str().is_empty() => {
                tail.push(name);
                probe = parent;
            }
            // Nothing along the path exists (or we reached the root). The
            // lexical form is all there is, and the root test still applies to
            // it.
            _ => return Ok(path.to_path_buf()),
        }
    }
}

/// Does this resolved path name a credential store?
fn is_denied(path: &Path) -> bool {
    for component in path.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        let Some(part) = part.to_str() else {
            continue;
        };
        if DENIED_DIR_NAMES
            .iter()
            .any(|denied| part.eq_ignore_ascii_case(denied))
        {
            return true;
        }
    }
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| {
            DENIED_FILE_NAMES
                .iter()
                .any(|denied| name.eq_ignore_ascii_case(denied))
        })
}

/// The user's home directory.
///
/// Read from the environment rather than pulled in as a dependency: the two
/// variables below are what every home-directory crate consults first, and
/// keeping it here means a test can build a [`PathGuard`] over a temporary
/// directory without one.
fn home_dir() -> Option<PathBuf> {
    // One answer, not two. This rule now lives on `Paths` because
    // `skill_catalog` needed it as well — and needed it to fall back to
    // `dirs::home_dir()`, which this copy did not do.
    biorouter::config::paths::Paths::home_dir()
}

/// Where this surface keeps the interface's own settings.
///
/// The original hardcoded `~/.config/biorouter/headless/settings.json`, which
/// is not where the configuration lives on Windows or under a non-default
/// `XDG_CONFIG_HOME`, and is not where `BIOROUTER_PATH_ROOT` puts it in a test.
fn settings_path() -> PathBuf {
    Paths::config_dir().join("headless").join("settings.json")
}

/// Where extension bundles are installed.
fn extensions_dir() -> PathBuf {
    Paths::config_dir().join("extensions")
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn json_error(status: StatusCode, message: String) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

// ---------------------------------------------------------------------------
// Handlers.
// ---------------------------------------------------------------------------

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        home_dir: home_dir()
            .map(|home| path_string(&home))
            .unwrap_or_default(),
        api_base_url: api_base_url(),
    })
}

/// The daemon's own base URL.
///
/// `commands::agent` publishes the bound address as `BIOROUTER_APP_BASE_URL`
/// once the listener is up, which is the only value that is right when the port
/// was ephemeral. The configured host/port is the fallback for a process that
/// has not reached that point.
fn api_base_url() -> String {
    if let Some(base) = std::env::var("BIOROUTER_APP_BASE_URL")
        .ok()
        .filter(|base| !base.is_empty())
    {
        return base;
    }
    let host = std::env::var("BIOROUTER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = std::env::var("BIOROUTER_PORT").unwrap_or_else(|_| "3000".to_string());
    format!("http://{host}:{port}")
}

async fn fs_roots() -> Json<RootsResponse> {
    let mut roots = Vec::new();
    if let Some(home) = home_dir() {
        roots.push(FsRoot {
            label: "Home".to_string(),
            path: path_string(&home),
        });
    }
    roots.push(FsRoot {
        label: "Biorouter config".to_string(),
        path: path_string(&Paths::config_dir()),
    });
    roots.push(FsRoot {
        label: "Temporary files".to_string(),
        path: path_string(&std::env::temp_dir()),
    });
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(FsRoot {
            label: "Current directory".to_string(),
            path: path_string(&cwd),
        });
    }
    Json(RootsResponse { roots })
}

async fn fs_list(Query(query): Query<PathQuery>) -> Response {
    let guard = PathGuard::current();
    let requested = query.path.as_deref().unwrap_or("~");
    let path = match guard.resolve(requested) {
        Ok(path) => path,
        Err(refusal) => return refusal.into_response(),
    };
    let mut read_dir = match tokio::fs::read_dir(&path).await {
        Ok(read_dir) => read_dir,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, format!("failed to list path: {e}")),
    };
    let mut entries = Vec::new();
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        if let Ok(metadata) = entry.metadata().await {
            entries.push(FsEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                path: path_string(&entry.path()),
                is_dir: metadata.is_dir(),
                is_file: metadata.is_file(),
            });
        }
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Json(FsListResponse {
        path: path_string(&path),
        entries,
    })
    .into_response()
}

async fn fs_list_dirs(Query(query): Query<PathQuery>) -> Response {
    let guard = PathGuard::current();
    let path = match guard.resolve(query.path.as_deref().unwrap_or("~")) {
        Ok(path) => path,
        Err(refusal) => return refusal.into_response(),
    };
    let Ok(mut read_dir) = tokio::fs::read_dir(&path).await else {
        return Json(DirsResponse { dirs: Vec::new() }).into_response();
    };
    let mut dirs = Vec::new();
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        if entry
            .metadata()
            .await
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false)
        {
            dirs.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    dirs.sort();
    Json(DirsResponse { dirs }).into_response()
}

async fn fs_list_files(Query(query): Query<ListFilesQuery>) -> Response {
    let guard = PathGuard::current();
    let path = match guard.resolve(query.path.as_deref().unwrap_or("~")) {
        Ok(path) => path,
        Err(refusal) => return refusal.into_response(),
    };
    let Ok(mut read_dir) = tokio::fs::read_dir(&path).await else {
        return Json(FilesResponse { files: Vec::new() }).into_response();
    };
    let mut files = Vec::new();
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if query
            .extension
            .as_ref()
            .is_none_or(|extension| name.ends_with(extension))
        {
            files.push(name);
        }
    }
    files.sort();
    Json(FilesResponse { files }).into_response()
}

async fn fs_read(Query(query): Query<PathQuery>) -> Response {
    let requested = query.path.unwrap_or_default();
    into_answer(read_file_within(&PathGuard::current(), &requested).await)
}

/// A file route's result as the response its caller receives. Both reading
/// routes answer through this one mapping, so a test that compares two answers
/// compares what a caller actually sees rather than a copy of the mapping.
fn into_answer<T: Serialize>(result: Result<T, Refusal>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(refusal) => refusal.into_response(),
    }
}

const MAX_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024;

/// Preview ceilings, held equal to the Electron previewer's
/// (`ui/desktop/src/utils/artifactPreviewLimits.ts`).
///
/// The two surfaces read the same files for the same panel, so a document the
/// desktop app refuses to open must not become previewable by pointing a
/// browser at the same daemon. The archive pair below sat at twice the
/// Electron numbers, which is exactly that: a limit the user can pick by
/// choosing an interface.
const MAX_OFFICE_ENTRIES: usize = 4_096;
const MAX_OFFICE_ENTRY_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OFFICE_EXPANDED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PRESENTATION_SLIDES: usize = 500;
const MAX_WORKBOOK_WORKSHEETS: usize = 50;
const MAX_WORKSHEET_COLUMNS: u64 = 2_000;
const MAX_WORKSHEET_ROWS: u64 = 200_000;
const MAX_WORKBOOK_USED_CELLS: u64 = 500_000;
const MAX_WORKBOOK_POPULATED_CELLS: usize = 200_000;
const MAX_OFFICE_TEXT_CHARS: usize = 100_000;
const MAX_WORKBOOK_TEXT_ROWS: usize = 10_000;

/// How much text to gather before the extractor stops opening parts.
///
/// Clipping only at the end would let a document with a thousand `headerN.xml`
/// parts accumulate all of them first. Four bytes is the longest UTF-8
/// encoding of one character, so stopping here still guarantees
/// [`MAX_OFFICE_TEXT_CHARS`] characters are on hand and the truncation flag
/// stays accurate.
const MAX_OFFICE_TEXT_BYTES: usize = MAX_OFFICE_TEXT_CHARS * 4;

/// Decoded-pixel ceilings for an image preview.
///
/// The byte ceiling above is not one of these. Every raster format states its
/// dimensions in a header a few dozen bytes long, so a well-formed file far
/// under 16 MiB can ask the renderer to allocate a gigapixel surface — the
/// decompression-bomb shape. The pixel cap is the binding one: 8192 × 8192 is
/// 67 108 864, more than twice [`MAX_IMAGE_PIXELS`], so a dimension cap alone
/// would let the worst case through.
const MAX_IMAGE_DIMENSION: u64 = 8_192;
const MAX_IMAGE_PIXELS: u64 = 32_000_000;

fn artifact_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html",
        "md" | "txt" | "rs" | "ts" | "tsx" | "js" | "json" | "yaml" | "yml" | "csv" | "sql" => {
            "text/plain"
        }
        "apng" => "image/apng",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "cur" => "image/vnd.microsoft.icon",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "jfif" | "jpeg" | "jpg" | "pjpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

fn artifact_format(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(OsStr::to_str)?
        .to_ascii_lowercase()
        .as_str()
    {
        "pdf" => Some("pdf"),
        "docx" => Some("docx"),
        "xlsx" => Some("xlsx"),
        "pptx" => Some("pptx"),
        _ => None,
    }
}

fn is_slide_part(name: &str) -> bool {
    name.starts_with("ppt/slides/slide") && name.ends_with(".xml")
}

fn is_worksheet_part(name: &str) -> bool {
    name.starts_with("xl/worksheets/sheet") && name.ends_with(".xml")
}

/// The columns × rows a worksheet declares as its used range.
///
/// `None` when it declares none, which SpreadsheetML permits: `<dimension>` is
/// an optional element, so a workbook that simply omits it skips this check
/// entirely. That is why the aggregate populated-cell total in
/// [`validate_office_archive`] is not redundant with this — it is the only
/// bound left on a workbook that declares nothing.
fn spreadsheet_used_range(xml: &str) -> Option<(u64, u64)> {
    let start = xml.find("<dimension")?;
    let after_start = xml.get(start..)?;
    let tag = after_start
        .split_once('>')
        .map_or(after_start, |(tag, _)| tag);
    let (_, after_reference_start) = tag.split_once("ref=\"")?;
    let (reference, _) = after_reference_start.split_once('"')?;
    let last = reference.rsplit(':').next().unwrap_or_default();
    let row_start = last.find(|character: char| character.is_ascii_digit())?;
    let column_reference = last.get(..row_start)?;
    let row_reference = last.get(row_start..)?;
    let columns = column_reference
        .bytes()
        .filter(u8::is_ascii_alphabetic)
        .fold(0_u64, |value, character| {
            value
                .saturating_mul(26)
                .saturating_add(u64::from(character.to_ascii_uppercase() - b'A' + 1))
        });
    let rows = row_reference.parse::<u64>().unwrap_or(u64::MAX);
    Some((columns, rows))
}

fn populated_cell_count(xml: &str) -> usize {
    xml.match_indices("<c")
        .filter(|(index, _)| {
            xml.as_bytes()
                .get(index + 2)
                .is_some_and(|next| next.is_ascii_whitespace() || *next == b'>')
        })
        .count()
}

fn validate_office_archive(bytes: &[u8], format: &str) -> Result<(), Refusal> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| Refusal::Unusable)?;
    if archive.len() > MAX_OFFICE_ENTRIES {
        return Err(Refusal::Unusable);
    }

    let required_part = match format {
        "docx" => "word/document.xml",
        "xlsx" => "xl/workbook.xml",
        "pptx" => "ppt/presentation.xml",
        _ => return Ok(()),
    };
    let mut has_required_part = false;
    let mut slide_count = 0_usize;
    let mut worksheet_count = 0_usize;
    let mut expanded_bytes = 0_u64;
    // A per-worksheet ceiling bounds one sheet, and a workbook is N of them:
    // fifty sheets that each sit just under the line pass every per-sheet test
    // and still hand the renderer ten million cells. The running totals are
    // what make the limits a property of the workbook.
    let mut used_cells = 0_u64;
    let mut populated_cells = 0_usize;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| Refusal::Unusable)?;
        let size = entry.size();
        let compressed = entry.compressed_size().max(1);
        if size > MAX_OFFICE_ENTRY_BYTES || size / compressed > 200 {
            return Err(Refusal::Unusable);
        }
        expanded_bytes = expanded_bytes.checked_add(size).ok_or(Refusal::Unusable)?;
        if expanded_bytes > MAX_OFFICE_EXPANDED_BYTES {
            return Err(Refusal::Unusable);
        }

        let name = entry.name().to_string();
        has_required_part |= name == required_part;
        if format == "pptx" && is_slide_part(&name) {
            slide_count += 1;
            if slide_count > MAX_PRESENTATION_SLIDES {
                return Err(Refusal::Unusable);
            }
        }
        if format == "xlsx" && is_worksheet_part(&name) {
            worksheet_count += 1;
            if worksheet_count > MAX_WORKBOOK_WORKSHEETS {
                return Err(Refusal::Unusable);
            }
            let xml = read_office_part(&mut entry)?;
            if let Some((columns, rows)) = spreadsheet_used_range(&xml) {
                let sheet_cells = columns.saturating_mul(rows);
                used_cells = used_cells.saturating_add(sheet_cells);
                if columns > MAX_WORKSHEET_COLUMNS
                    || rows > MAX_WORKSHEET_ROWS
                    || sheet_cells > MAX_WORKBOOK_USED_CELLS
                    || used_cells > MAX_WORKBOOK_USED_CELLS
                {
                    return Err(Refusal::Unusable);
                }
            }
            let sheet_populated = populated_cell_count(&xml);
            populated_cells = populated_cells.saturating_add(sheet_populated);
            if sheet_populated > MAX_WORKBOOK_POPULATED_CELLS
                || populated_cells > MAX_WORKBOOK_POPULATED_CELLS
            {
                return Err(Refusal::Unusable);
            }
        }
    }
    has_required_part.then_some(()).ok_or(Refusal::Unusable)
}

/// Read one archive entry as text, bounded by what actually arrives.
///
/// `entry.size()` is a number the person who built the archive chose, so it
/// bounds nothing and cannot size the buffer either — the same invariant
/// [`read_archive`] already states for skill and extension bundles. Reading one
/// byte past the ceiling is what distinguishes "exactly fills it" from
/// "overruns it".
fn read_office_part(entry: &mut impl Read) -> Result<String, Refusal> {
    let mut xml = String::new();
    entry
        .take(MAX_OFFICE_ENTRY_BYTES + 1)
        .read_to_string(&mut xml)
        .map_err(|_| Refusal::Unusable)?;
    if xml.len() as u64 > MAX_OFFICE_ENTRY_BYTES {
        return Err(Refusal::Unusable);
    }
    Ok(xml)
}

// ---------------------------------------------------------------------------
// Office text extraction.
// ---------------------------------------------------------------------------

/// The readable text of an Office document, and whether it was clipped.
struct OfficeText {
    text: String,
    truncated: bool,
}

type OfficeArchive<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

/// Flatten a validated Office document into the text the panel is showing.
///
/// The desktop previewer has always done this (`extractOfficeText` in
/// `ui/desktop/src/main.ts`) and the renderer's response type has always
/// declared the two fields, but this surface never produced them — so
/// `workspace_read_panel` returned nothing at all for a DOCX opened through
/// browser access. The caps below are that function's, to the number, because
/// the two surfaces feed the same tool.
///
/// `None` when the archive cannot be read; the document still previews, it just
/// carries no text, which is what the field being optional means.
fn extract_office_text(bytes: &[u8], format: &str) -> Option<OfficeText> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let text = match format {
        "docx" => document_text(&mut archive),
        "pptx" => presentation_text(&mut archive),
        "xlsx" => workbook_text(&mut archive),
        _ => return None,
    };
    let clip_at = text
        .char_indices()
        .nth(MAX_OFFICE_TEXT_CHARS)
        .map(|(index, _)| index);
    Some(match clip_at {
        Some(index) => OfficeText {
            text: text.get(..index).unwrap_or_default().to_string(),
            truncated: true,
        },
        None => OfficeText {
            text,
            truncated: false,
        },
    })
}

/// Every part of the archive this extractor wants, in archive order.
fn office_part_names(archive: &OfficeArchive, wanted: impl Fn(&str) -> bool) -> Vec<String> {
    archive
        .file_names()
        .filter(|name| wanted(name))
        .map(str::to_string)
        .collect()
}

/// The trailing number in a part name, so `slide10.xml` sorts after
/// `slide9.xml` rather than before it.
fn office_part_order(name: &str) -> u64 {
    name.trim_end_matches(".xml")
        .rsplit(|character: char| !character.is_ascii_digit())
        .next()
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}

fn office_part_text(archive: &mut OfficeArchive, name: &str) -> Option<String> {
    let mut entry = archive.by_name(name).ok()?;
    read_office_part(&mut entry).ok()
}

fn document_text(archive: &mut OfficeArchive) -> String {
    let names = office_part_names(archive, |name| {
        name.strip_prefix("word/").is_some_and(|part| {
            matches!(part, "document.xml" | "footnotes.xml" | "endnotes.xml")
                || ((part.starts_with("header") || part.starts_with("footer"))
                    && part.ends_with(".xml"))
        })
    });
    let mut parts = Vec::new();
    let mut gathered = 0_usize;
    for name in &names {
        if gathered > MAX_OFFICE_TEXT_BYTES {
            break;
        }
        let Some(xml) = office_part_text(archive, name) else {
            continue;
        };
        let text = decode_office_xml_text(&xml);
        if !text.is_empty() {
            gathered += text.len();
            parts.push(text);
        }
    }
    parts.join("\n\n")
}

fn presentation_text(archive: &mut OfficeArchive) -> String {
    let mut names = office_part_names(archive, is_slide_part);
    names.sort_by_key(|name| office_part_order(name));
    let mut slides = Vec::new();
    let mut gathered = 0_usize;
    for (index, name) in names.iter().enumerate() {
        if gathered > MAX_OFFICE_TEXT_BYTES {
            break;
        }
        let xml = office_part_text(archive, name).unwrap_or_default();
        let slide = format!("[Slide {}]\n{}", index + 1, decode_office_xml_text(&xml));
        gathered += slide.len();
        slides.push(slide);
    }
    slides.join("\n\n")
}

fn workbook_text(archive: &mut OfficeArchive) -> String {
    // A cell of type `s` holds an index into this table rather than its own
    // text, so without it a workbook flattens to a column of integers.
    let shared: Vec<String> = office_part_text(archive, "xl/sharedStrings.xml")
        .map(|xml| {
            xml_elements(&xml, "si")
                .into_iter()
                .map(|(_, inner)| decode_office_xml_text(inner))
                .collect()
        })
        .unwrap_or_default();
    let mut names = office_part_names(archive, is_worksheet_part);
    names.sort_by_key(|name| office_part_order(name));
    let mut rows = Vec::new();
    for (index, name) in names.iter().enumerate() {
        rows.push(format!("[Sheet {}]", index + 1));
        let Some(xml) = office_part_text(archive, name) else {
            continue;
        };
        for (attributes, inner) in xml_elements(&xml, "c") {
            if rows.len() >= MAX_WORKBOOK_TEXT_ROWS {
                break;
            }
            let reference = xml_attribute(attributes, "r").unwrap_or("?");
            let raw = xml_elements(inner, "v").first().map(|(_, value)| *value);
            let inline = xml_elements(inner, "is").first().map(|(_, value)| *value);
            let value = match (xml_attribute(attributes, "t"), raw) {
                (Some("s"), Some(index)) => index
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| shared.get(index))
                    .cloned()
                    .unwrap_or_default(),
                _ => decode_office_xml_text(inline.or(raw).unwrap_or_default()),
            };
            if !value.is_empty() {
                rows.push(format!("{reference}: {value}"));
            }
        }
    }
    rows.join("\n")
}

/// The `(attributes, inner)` of every `<name …>…</name>` element, in document
/// order.
///
/// Deliberately not an XML parser. The parts this reads are machine-generated
/// and flat, the elements it asks for do not nest inside themselves, and the
/// alternative is a full XML stack in the daemon for the sake of a preview.
fn xml_elements<'a>(xml: &'a str, name: &str) -> Vec<(&'a str, &'a str)> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut elements = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let Some(after_name) = rest.get(start + open.len()..) else {
            break;
        };
        // `<c` must not match `<cols`.
        if !after_name.starts_with([' ', '\t', '\r', '\n', '>', '/']) {
            rest = after_name;
            continue;
        }
        let Some((attributes, body)) = after_name.split_once('>') else {
            break;
        };
        if let Some(attributes) = attributes.strip_suffix('/') {
            elements.push((attributes, ""));
            rest = body;
            continue;
        }
        let Some((inner, tail)) = body.split_once(close.as_str()) else {
            break;
        };
        elements.push((attributes, inner));
        rest = tail;
    }
    elements
}

/// One attribute's value, matched only at an attribute boundary so `t="s"` is
/// not found inside `xt="s"`.
fn xml_attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let mut rest = attributes;
    loop {
        let start = rest.find(&needle)?;
        let at_boundary = rest
            .get(..start)
            .and_then(|before| before.chars().next_back())
            .is_none_or(char::is_whitespace);
        let after = rest.get(start + needle.len()..)?;
        if at_boundary {
            return after.split_once('"').map(|(value, _)| value);
        }
        rest = after;
    }
}

/// Flatten one Office XML part into the text a reader would see.
fn decode_office_xml_text(xml: &str) -> String {
    let mut flattened = String::with_capacity(xml.len() / 4);
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        flattened.push_str(rest.get(..open).unwrap_or_default());
        let Some(after) = rest.get(open + 1..) else {
            break;
        };
        let Some((tag, tail)) = after.split_once('>') else {
            break;
        };
        flattened.push_str(office_tag_separator(tag));
        rest = tail;
    }
    flattened.push_str(rest);
    collapse_office_whitespace(&unescape_xml(&flattened))
}

/// What a structural tag contributes once the markup is gone.
///
/// Word writes a tab and a line break as elements rather than as characters,
/// and the end of a paragraph or a row is the only thing separating one line
/// of the flattened text from the next.
fn office_tag_separator(tag: &str) -> &'static str {
    let closing = tag.starts_with('/');
    let name = tag
        .trim_start_matches('/')
        .split([' ', '\t', '\r', '\n', '/'])
        .next()
        .unwrap_or_default();
    match (closing, name) {
        (false, "w:tab") => "\t",
        (false, "w:br") => "\n",
        (true, "w:p" | "a:p" | "row") => "\n",
        _ => "",
    }
}

fn unescape_xml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        // Last, so an escaped `&amp;lt;` survives as the literal `&lt;`.
        .replace("&amp;", "&")
}

/// Drop trailing blanks from each line and never leave more than one empty line
/// between blocks, so a document whose markup carried the layout does not
/// arrive as a column of whitespace.
fn collapse_office_whitespace(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut blank_run = 0_usize;
    for line in text.split('\n') {
        let line = line.trim_end_matches([' ', '\t']);
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

// ---------------------------------------------------------------------------
// Image bounds.
// ---------------------------------------------------------------------------

/// Refuse an image whose header asks the renderer for more pixels than it will
/// decode, before those bytes are encoded into a response.
fn assert_previewable_image(bytes: &[u8], mime_type: &str) -> Result<(), Refusal> {
    let Some((width, height)) = image_dimensions(bytes, mime_type) else {
        return Ok(());
    };
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || width.saturating_mul(height) > MAX_IMAGE_PIXELS
    {
        return Err(Refusal::Unusable);
    }
    Ok(())
}

/// The pixel dimensions an image's header declares.
///
/// `None` is a deliberate pass rather than a refusal, and it covers two cases.
/// TIFF, ICO and the Windows cursor formats keep their dimensions behind a
/// directory the renderer walks and bounds for itself (`safeTiffDimensions` in
/// `ArtifactViewer.tsx`), so a second half-parse here would add a way to refuse
/// a legitimate file without adding a bound. And a file whose header is
/// truncated or malformed decodes to nothing in the renderer anyway — refusing
/// every unparseable header would break previews for files that are not a
/// threat.
fn image_dimensions(bytes: &[u8], mime_type: &str) -> Option<(u64, u64)> {
    match mime_type {
        "image/png" | "image/apng" => {
            (bytes.get(12..16)? == b"IHDR").then_some(())?;
            Some((read_be_u32(bytes, 16)?, read_be_u32(bytes, 20)?))
        }
        "image/gif" => Some((read_le_u16(bytes, 6)?, read_le_u16(bytes, 8)?)),
        // The dimensions are signed: a bottom-up bitmap states a negative
        // height, which is a direction rather than a smaller allocation.
        "image/bmp" => Some((
            read_le_i32_magnitude(bytes, 18)?,
            read_le_i32_magnitude(bytes, 22)?,
        )),
        "image/jpeg" => jpeg_dimensions(bytes),
        "image/webp" => webp_dimensions(bytes),
        "image/avif" => avif_dimensions(bytes),
        "image/svg+xml" => svg_dimensions(bytes),
        _ => None,
    }
}

fn read_be_u32(bytes: &[u8], offset: usize) -> Option<u64> {
    let field = <[u8; 4]>::try_from(bytes.get(offset..offset + 4)?).ok()?;
    Some(u64::from(u32::from_be_bytes(field)))
}

fn read_le_u16(bytes: &[u8], offset: usize) -> Option<u64> {
    let field = <[u8; 2]>::try_from(bytes.get(offset..offset + 2)?).ok()?;
    Some(u64::from(u16::from_le_bytes(field)))
}

fn read_le_u24(bytes: &[u8], offset: usize) -> Option<u64> {
    let field = bytes.get(offset..offset + 3)?;
    Some(
        field
            .iter()
            .rev()
            .fold(0_u64, |value, byte| (value << 8) | u64::from(*byte)),
    )
}

fn read_le_i32_magnitude(bytes: &[u8], offset: usize) -> Option<u64> {
    let field = <[u8; 4]>::try_from(bytes.get(offset..offset + 4)?).ok()?;
    Some(u64::from(i32::from_le_bytes(field).unsigned_abs()))
}

/// Walk the JPEG marker chain to the start-of-frame that carries the size.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u64, u64)> {
    if bytes.get(..2)? != [0xff, 0xd8] {
        return None;
    }
    let mut offset = 2_usize;
    while offset + 8 < bytes.len() {
        if bytes.get(offset).copied()? != 0xff {
            offset += 1;
            continue;
        }
        let marker = bytes.get(offset + 1).copied()?;
        // Standalone markers carry no length field to skip over.
        if marker == 0xd8 || marker == 0xd9 {
            offset += 2;
            continue;
        }
        let length = usize::try_from(read_be_u16(bytes, offset + 2)?).ok()?;
        if length < 2 || offset + 2 + length > bytes.len() {
            return None;
        }
        let is_start_of_frame =
            matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf);
        if is_start_of_frame {
            return Some((
                read_be_u16(bytes, offset + 7)?,
                read_be_u16(bytes, offset + 5)?,
            ));
        }
        offset += 2 + length;
    }
    None
}

fn read_be_u16(bytes: &[u8], offset: usize) -> Option<u64> {
    let field = <[u8; 2]>::try_from(bytes.get(offset..offset + 2)?).ok()?;
    Some(u64::from(u16::from_be_bytes(field)))
}

fn webp_dimensions(bytes: &[u8]) -> Option<(u64, u64)> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WEBP" {
        return None;
    }
    match bytes.get(12..16)? {
        // The extended container states the canvas size minus one.
        b"VP8X" => Some((read_le_u24(bytes, 24)? + 1, read_le_u24(bytes, 27)? + 1)),
        b"VP8 " => Some((
            read_le_u16(bytes, 26)? & 0x3fff,
            read_le_u16(bytes, 28)? & 0x3fff,
        )),
        b"VP8L" if bytes.get(20).copied()? == 0x2f => {
            let field = <[u8; 4]>::try_from(bytes.get(21..25)?).ok()?;
            let bits = u64::from(u32::from_le_bytes(field));
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        _ => None,
    }
}

/// AVIF states its size in an `ispe` box somewhere in the metadata tree. Scan
/// for it rather than walking the box hierarchy: the tree's shape varies with
/// the encoder, and the box's own length field is what validates a hit.
fn avif_dimensions(bytes: &[u8]) -> Option<(u64, u64)> {
    for offset in 4..bytes.len().saturating_sub(15) {
        if bytes.get(offset..offset + 4)? != b"ispe" {
            continue;
        }
        let box_start = offset - 4;
        let box_size = read_be_u32(bytes, box_start)?;
        if box_size < 20 || u64::try_from(box_start).ok()? + box_size > bytes.len() as u64 {
            continue;
        }
        return Some((
            read_be_u32(bytes, offset + 8)?,
            read_be_u32(bytes, offset + 12)?,
        ));
    }
    None
}

/// The size an SVG asks to be painted at.
///
/// Vector markup is a decompression bomb too — `width="100000"` is nine bytes
/// and a ten-gigapixel surface — so it is bounded on the same numbers as the
/// raster formats, exactly as the desktop previewer bounds it.
fn svg_dimensions(bytes: &[u8]) -> Option<(u64, u64)> {
    let head = bytes.get(..bytes.len().min(64 * 1024))?;
    let source = String::from_utf8_lossy(head);
    let start = source.find("<svg")?;
    let after = source.get(start..)?;
    // The opening tag only: a `width` after the first `>` belongs to a child.
    let tag = after.split_once('>').map_or(after, |(tag, _)| tag);
    if let (Some(width), Some(height)) = (svg_length(tag, "width"), svg_length(tag, "height")) {
        return Some((width, height));
    }
    // Without both, `viewBox="minX minY width height"` is what the image
    // scales to.
    let view_box = xml_attribute(tag, "viewBox")?;
    let mut extent = view_box
        .split([' ', ',', '\t', '\n', '\r'])
        .filter(|part| !part.is_empty())
        .skip(2);
    Some((
        leading_number(extent.next()?)?,
        leading_number(extent.next()?)?,
    ))
}

fn svg_length(tag: &str, name: &str) -> Option<u64> {
    leading_number(xml_attribute(tag, name)?)
}

/// The number a CSS length starts with — `100%` and `210mm` are both lengths,
/// and the unit does not change the order of magnitude being asked for.
fn leading_number(value: &str) -> Option<u64> {
    let value = value.trim_start();
    let end = value
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(value.len());
    value
        .get(..end)?
        .parse::<f64>()
        .ok()
        .and_then(|number| (number.is_finite() && number >= 0.0).then(|| number.ceil() as u64))
}

// ---------------------------------------------------------------------------
// Artifact identity.
// ---------------------------------------------------------------------------

/// The key [`artifact_revision`] is computed under.
///
/// A revision travels outward: `workspace_read_panel` hands it to the model as
/// proof that the text it is about to read is the text on screen. An *unkeyed*
/// digest of a local file would be a reusable oracle — anyone holding one can
/// confirm a guess at the file's contents by hashing the guess. Generating the
/// key per process, never writing it down and never returning it makes a
/// revision comparable only against another revision from the same daemon,
/// which is all a staleness check ever needed.
static REVISION_KEY: LazyLock<[u8; 32]> = LazyLock::new(rand::random);

/// Identity of the exact bytes this surface previewed.
///
/// `size:mtime` alone is not one, and the consumer's staleness check
/// (`useArtifactPanelAccess.ts`) reduces entirely to this string being
/// collision-free: a same-size edit inside one modification-time tick — and a
/// whole second is one tick on plenty of filesystems — reproduces it exactly,
/// so the agent could be handed bytes that were never displayed, stamped with
/// the displayed revision.
fn artifact_revision(size: u64, mtime_millis: u128, content: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(REVISION_KEY.as_slice())
        .expect("HMAC-SHA256 accepts a key of any length");
    mac.update(content);
    format!(
        "{size}:{mtime_millis}:{}",
        hex::encode(mac.finalize().into_bytes())
    )
}

/// Identity of a file this surface declined to read.
///
/// The oversize branch never opens the bytes, so there is no content to
/// identify — but omitting the field makes an absence the consumer has to
/// special-case, so the identity of the *file* stands in for the identity of
/// its contents. Keyed like every other revision, so nothing unkeyed about the
/// user's filesystem leaves the process.
fn unread_artifact_revision(metadata: &std::fs::Metadata, mtime_millis: u128) -> String {
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    #[cfg(not(unix))]
    let identity = metadata.len().to_string();
    artifact_revision(metadata.len(), mtime_millis, identity.as_bytes())
}

/// Open a validated path without following a link, and prove the descriptor is
/// the file that was validated.
///
/// [`PathGuard::resolve`] canonicalizes, but what it returns is a *string*, and
/// [`File::open`] resolves that string again from scratch. Everything in
/// between — the root test, [`is_denied`], the project's `.biorouterignore` —
/// therefore describes whatever the name pointed at *then*. `std::env::temp_dir`
/// is one of the roots and is world-writable on Unix, so "then" and "now" are a
/// window anyone with a shell on this machine can drive: leave
/// `/tmp/a/report.pdf` a real file until the checks pass, flip it to a link at
/// `~/.config/biorouter/secrets.yaml`, and the bytes that come back are not the
/// bytes that were checked. Retryable until it wins.
///
/// Two things close it. `O_NOFOLLOW` makes an open through a link fail rather
/// than succeed quietly, and comparing the opened descriptor's identity against
/// the name's turns any other substitution — a rename, a fresh regular file —
/// into a mismatch instead of a read.
///
/// Not closed, and not closeable by name: a **hard** link to a credential store
/// is a regular file at a permitted path, so it passes both checks and the deny
/// lists alike. It also needs local write access to one of the roots and, on
/// Linux, `fs.protected_hardlinks=0`.
fn open_validated_file(path: &Path) -> Result<(File, std::fs::Metadata), Refusal> {
    open_validated(path).map_err(|failure| match failure {
        OpenFailure::Io(_) => Refusal::Unusable,
        OpenFailure::Refused(refusal) => refusal,
    })
}

/// Why [`open_validated`] handed back no descriptor.
///
/// Two kinds, because the two reading routes answer them differently. An I/O
/// failure is an ordinary answer about a path the guard already allowed — not
/// there, not readable, not a regular file — which `/headless/fs/read` has
/// always reported as `200 {"found":false}` and the artifact route as
/// [`Refusal::Unusable`]. A refusal is a boundary, and both routes refuse it.
enum OpenFailure {
    Io(std::io::Error),
    Refused(Refusal),
}

/// [`open_validated_file`], keeping an I/O failure's own error for the route
/// that reports it.
fn open_validated(path: &Path) -> Result<(File, std::fs::Metadata), OpenFailure> {
    let named = std::fs::symlink_metadata(path).map_err(OpenFailure::Io)?;
    if named.file_type().is_symlink() {
        return Err(OpenFailure::Refused(Refusal::Denied));
    }
    if !named.file_type().is_file() {
        return Err(OpenFailure::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        )));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(OpenFailure::Io)?;
    let opened = file.metadata().map_err(OpenFailure::Io)?;
    if !opened.is_file() || !is_same_file(&named, &opened) {
        return Err(OpenFailure::Refused(Refusal::Denied));
    }
    Ok((file, opened))
}

/// Do two stat results name one file? On Unix that is `(device, inode)`. There
/// is no such pair off Unix, where there is also no `O_NOFOLLOW` to pair it
/// with; the file-type check is what stands there.
#[cfg(unix)]
fn is_same_file(named: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    named.dev() == opened.dev() && named.ino() == opened.ino()
}

#[cfg(not(unix))]
fn is_same_file(_named: &std::fs::Metadata, _opened: &std::fs::Metadata) -> bool {
    true
}

async fn fs_artifact(Query(query): Query<PathQuery>) -> Response {
    let requested = query.path.unwrap_or_default();
    into_answer(read_artifact_within(&PathGuard::current(), &requested))
}

fn read_artifact_within(guard: &PathGuard, requested: &str) -> Result<serde_json::Value, Refusal> {
    let path = guard.resolve(requested)?;
    let (mut file, metadata) = open_validated_file(&path)?;
    // Both deny lists run again now that the descriptor is proven to be this
    // path. `resolve` ran them against a *name*, and a name is the one thing
    // the substitution above changes — a check that never reaches the file it
    // is protecting is advice, not a boundary.
    if is_denied(&path) {
        return Err(Refusal::Denied);
    }
    let project_root = path
        .parent()
        .and_then(|parent| {
            parent
                .ancestors()
                .find(|ancestor| ancestor.join(".biorouterignore").is_file())
        })
        .unwrap_or(&guard.cwd);
    let secret_guard = biorouter_mcp::secret_guard::SecretGuard::cached_for_dir(project_root);
    if secret_guard.is_denied(&path) {
        return Err(Refusal::Denied);
    }
    let title = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(requested)
        .to_string();
    let mime_type = artifact_mime(&path);
    let mtime_millis = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_millis());
    if metadata.len() > MAX_ARTIFACT_BYTES {
        return Ok(serde_json::json!({
            "kind": "binary", "title": title, "path": requested,
            "mimeType": mime_type, "size": metadata.len(),
            "revision": unread_artifact_revision(&metadata, mtime_millis), "found": true
        }));
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(MAX_ARTIFACT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Refusal::Unusable)?;
    if bytes.len() as u64 > MAX_ARTIFACT_BYTES {
        return Err(Refusal::Unusable);
    }
    let revision = artifact_revision(metadata.len(), mtime_millis, &bytes);
    if let Some(format) = artifact_format(&path) {
        // The renderer extracts PDF text itself (`readPdfText`); for the Office
        // formats it has always expected the producer to do it.
        let extracted = if format == "pdf" {
            None
        } else {
            validate_office_archive(&bytes, format)?;
            extract_office_text(&bytes, format)
        };
        let mut document = serde_json::json!({
            "kind": "document", "format": format, "title": title, "path": requested,
            "mimeType": mime_type, "dataBase64": base64::engine::general_purpose::STANDARD.encode(bytes),
            "size": metadata.len(), "revision": revision, "found": true
        });
        if let Some(extracted) = extracted {
            document["extractedText"] = extracted.text.into();
            document["textTruncated"] = extracted.truncated.into();
        }
        return Ok(document);
    }
    if mime_type.starts_with("image/") {
        assert_previewable_image(&bytes, mime_type)?;
        if mime_type == "image/tiff" {
            return Ok(serde_json::json!({
                "kind": "image", "title": title, "path": requested, "mimeType": mime_type,
                "dataBase64": base64::engine::general_purpose::STANDARD.encode(bytes),
                "size": metadata.len(), "revision": revision, "found": true
            }));
        }
        return Ok(serde_json::json!({
            "kind": "image", "title": title, "path": requested, "mimeType": mime_type,
            "dataUrl": format!("data:{mime_type};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)),
            "size": metadata.len(), "revision": revision, "found": true
        }));
    }
    if mime_type.starts_with("text/") {
        let text = String::from_utf8(bytes).map_err(|_| Refusal::Unusable)?;
        return Ok(serde_json::json!({
            "kind": if mime_type == "text/html" { "html" } else { "text" },
            "title": title, "path": requested, "mimeType": mime_type, "text": text,
            "size": metadata.len(), "revision": revision, "found": true
        }));
    }
    Ok(serde_json::json!({
        "kind": "binary", "title": title, "path": requested,
        "mimeType": mime_type, "size": metadata.len(),
        "revision": revision, "found": true
    }))
}

/// The whole of `fs_read` except for the HTTP shell, so the boundary can be
/// exercised against a real directory instead of against the process's own
/// home.
///
/// The file is opened the way the artifact route opens it, through
/// [`open_validated`], and for the reason recorded on [`open_validated_file`]:
/// `resolve` checked a *name*, and a plain open resolves that name again from
/// scratch. This route returns contents, so a regular file flipped to a link
/// between the two was a read of whatever the link pointed at, however firmly
/// the guard had just refused it by name.
async fn read_file_within(guard: &PathGuard, requested: &str) -> Result<ReadFileResponse, Refusal> {
    let path = guard.resolve(requested)?;
    let read = tokio::task::spawn_blocking(move || read_validated_text(&path))
        .await
        .map_err(|_| Refusal::Unusable)??;
    // Echo what the caller asked for, not where it landed: the renderer keys
    // its cache on the string it sent.
    let file_path = requested.to_string();
    Ok(match read {
        Ok(file) => ReadFileResponse {
            file_path,
            file,
            found: true,
            error: None,
        },
        Err(e) => ReadFileResponse {
            file_path,
            file: String::new(),
            found: false,
            error: Some(e.to_string()),
        },
    })
}

/// A resolved path's contents as text, opened without following a link and
/// only if the descriptor is the file that was validated.
///
/// `Err` is a refusal. `Ok(Err(_))` is an ordinary I/O answer (no such file,
/// not UTF-8, not a regular file), which the route reports as
/// `200 {"found":false}` exactly as it always has.
fn read_validated_text(path: &Path) -> Result<std::io::Result<String>, Refusal> {
    let mut file = match open_validated(path) {
        Ok((file, _)) => file,
        Err(OpenFailure::Io(e)) => return Ok(Err(e)),
        Err(OpenFailure::Refused(refusal)) => return Err(refusal),
    };
    let mut text = String::new();
    Ok(file.read_to_string(&mut text).map(|_| text))
}

async fn fs_write(Json(request): Json<WriteFileRequest>) -> Response {
    let guard = PathGuard::current();
    let path = match guard.resolve(&request.path) {
        Ok(path) => path,
        Err(refusal) => return refusal.into_response(),
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return json_error(
                StatusCode::BAD_REQUEST,
                format!("failed to create parent directory: {e}"),
            );
        }
    }
    match tokio::fs::write(&path, request.content).await {
        Ok(()) => Json(FsOkResponse { ok: true }).into_response(),
        Err(e) => json_error(
            StatusCode::BAD_REQUEST,
            format!("failed to write file: {e}"),
        ),
    }
}

async fn fs_ensure_dir(Json(request): Json<FsPathRequest>) -> Response {
    let guard = PathGuard::current();
    let path = match guard.resolve(&request.path) {
        Ok(path) => path,
        Err(refusal) => return refusal.into_response(),
    };
    match tokio::fs::create_dir_all(&path).await {
        Ok(()) => Json(FsOkResponse { ok: true }).into_response(),
        Err(e) => json_error(
            StatusCode::BAD_REQUEST,
            format!("failed to create directory: {e}"),
        ),
    }
}

async fn fs_delete_file(Json(request): Json<FsPathRequest>) -> Response {
    let guard = PathGuard::current();
    let path = match guard.resolve(&request.path) {
        Ok(path) => path,
        Err(refusal) => return refusal.into_response(),
    };
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Json(FsOkResponse { ok: true }).into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Json(FsOkResponse { ok: true }).into_response()
        }
        Err(e) => json_error(
            StatusCode::BAD_REQUEST,
            format!("failed to delete file: {e}"),
        ),
    }
}

async fn fs_delete_dir(Json(request): Json<FsPathRequest>) -> Response {
    let guard = PathGuard::current();
    let path = match guard.resolve(&request.path) {
        Ok(path) => path,
        Err(refusal) => return refusal.into_response(),
    };
    // A recursive delete of a root itself would take the home directory, the
    // configuration, or the working tree with it. The original checked two
    // literals (`$HOME` and `/tmp`); every root is checked here, so the config
    // directory and the working directory are covered too.
    if guard.is_root(&path) {
        return json_error(
            StatusCode::FORBIDDEN,
            "refusing to delete a root directory".to_string(),
        );
    }
    match tokio::fs::remove_dir_all(&path).await {
        Ok(()) => Json(FsOkResponse { ok: true }).into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Json(FsOkResponse { ok: true }).into_response()
        }
        Err(e) => json_error(
            StatusCode::BAD_REQUEST,
            format!("failed to delete directory: {e}"),
        ),
    }
}

async fn settings_read() -> Json<SettingsResponse> {
    let settings = match tokio::fs::read_to_string(settings_path()).await {
        Ok(file) => serde_json::from_str(&file).unwrap_or_else(|_| default_settings()),
        Err(_) => default_settings(),
    };
    Json(SettingsResponse { settings })
}

async fn settings_write(Json(settings): Json<serde_json::Value>) -> Response {
    if !settings.is_object() {
        return json_error(
            StatusCode::BAD_REQUEST,
            "settings payload must be a JSON object".to_string(),
        );
    }
    let path = settings_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return json_error(
                StatusCode::BAD_REQUEST,
                format!("failed to create settings directory: {e}"),
            );
        }
    }
    let Ok(serialized) = serde_json::to_string_pretty(&settings) else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "failed to serialize settings".to_string(),
        );
    };
    match tokio::fs::write(&path, serialized).await {
        Ok(()) => Json(FsOkResponse { ok: true }).into_response(),
        Err(e) => json_error(
            StatusCode::BAD_REQUEST,
            format!("failed to write settings: {e}"),
        ),
    }
}

fn default_settings() -> serde_json::Value {
    serde_json::json!({
        "envToggles": {
            "BIOROUTER_SERVER__MEMORY": false,
            "BIOROUTER_SERVER__COMPUTER_CONTROLLER": false
        },
        "showMenuBarIcon": false,
        "showDockIcon": false,
        "enableWakelock": false,
        "spellcheckEnabled": true
    })
}

async fn registry_download(
    Json(request): Json<RegistryDownloadRequest>,
) -> Json<RegistryDownloadResponse> {
    Json(match fetch_registry_asset(&request.url).await {
        Ok(path) => RegistryDownloadResponse {
            path: Some(path_string(&path)),
            error: None,
        },
        Err(error) => RegistryDownloadResponse {
            path: None,
            error: Some(error),
        },
    })
}

async fn fetch_registry_asset(raw_url: &str) -> Result<PathBuf, String> {
    let url = allowed_registry_url(raw_url).ok_or("Refusing to download from an untrusted URL.")?;
    let path_lower = url.path().to_ascii_lowercase();
    if !path_lower.ends_with(".zip") && !path_lower.ends_with(".brxt") {
        return Err("Unsupported asset type.".to_string());
    }

    let response = REGISTRY_CLIENT
        .get(url.clone())
        .header(header::USER_AGENT, "Biorouter")
        .send()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Download failed: HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_REGISTRY_DOWNLOAD_BYTES)
    {
        return Err("Download too large.".to_string());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    if bytes.len() as u64 > MAX_REGISTRY_DOWNLOAD_BYTES {
        return Err("Download too large.".to_string());
    }

    let dir = std::env::temp_dir().join("biorouter-registry");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    let safe_name = url
        .path_segments()
        .and_then(Iterator::last)
        .filter(|name| !name.is_empty())
        .map(sanitize_asset_name)
        .unwrap_or_else(|| "asset.zip".to_string());
    let dest = dir.join(format!("{}-{safe_name}", random_suffix()));
    tokio::fs::write(&dest, bytes)
        .await
        .map_err(|e| format!("Download failed: {e}"))?;
    Ok(dest)
}

fn allowed_registry_url(raw_url: &str) -> Option<reqwest::Url> {
    let url = reqwest::Url::parse(raw_url).ok()?;
    if url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?;
    REGISTRY_DOWNLOAD_HOSTS.contains(&host).then_some(url)
}

fn sanitize_asset_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "asset.zip".to_string()
    } else {
        sanitized
    }
}

fn random_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}")
}

async fn brxt_validate(Json(request): Json<FilePathRequest>) -> Response {
    let path = match PathGuard::current().resolve(&request.file_path) {
        Ok(path) => path,
        Err(refusal) => return refusal.into_response(),
    };
    Json(match validate_brxt_bundle(&path) {
        Ok(value) => value,
        Err(e) => serde_json::json!({ "error": e }),
    })
    .into_response()
}

async fn brxt_install(Json(request): Json<BrxtInstallRequest>) -> Response {
    let path = match PathGuard::current().resolve(&request.file_path) {
        Ok(path) => path,
        Err(refusal) => return refusal.into_response(),
    };
    Json(
        match install_brxt_bundle(&path, &request.extension_name).await {
            Ok(install_dir) => {
                serde_json::json!({ "success": true, "installDir": path_string(&install_dir) })
            }
            Err(e) => serde_json::json!({ "error": format!("Installation failed: {e}") }),
        },
    )
    .into_response()
}

async fn brxt_uninstall(Json(request): Json<BrxtUninstallRequest>) -> Json<serde_json::Value> {
    Json(match uninstall_brxt_extension(&request.extension_name) {
        Ok(()) => serde_json::json!({ "success": true }),
        Err(e) => serde_json::json!({ "error": format!("Uninstall failed: {e}") }),
    })
}

// ---------------------------------------------------------------------------
// Archive handling.
// ---------------------------------------------------------------------------

fn read_archive(path: &Path) -> Result<Vec<ArchiveEntry>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to read ZIP: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Failed to read ZIP: {e}"))?;
    let mut entries = Vec::new();
    let mut budget = MAX_ARCHIVE_BYTES;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|e| format!("Failed to read ZIP: {e}"))?;
        let name = file.name().replace('\\', "/");
        safe_entry_name(&name).map_err(|e| format!("Failed to read ZIP: {e}"))?;
        let is_dir = file.is_dir();
        let mut data = Vec::new();
        if !is_dir {
            // Read one byte past the remaining budget so "exactly fills it" and
            // "overruns it" are distinguishable. Measured while reading rather
            // than taken from the archive's declared sizes, which the person
            // who built the archive chose.
            file.by_ref()
                .take(budget.saturating_add(1))
                .read_to_end(&mut data)
                .map_err(|e| format!("Failed to read ZIP: {e}"))?;
            if data.len() as u64 > budget {
                return Err(format!(
                    "ZIP expands past the {MAX_ARCHIVE_BYTES} byte limit"
                ));
            }
            budget -= data.len() as u64;
        }
        entries.push(ArchiveEntry { name, is_dir, data });
    }
    Ok(entries)
}

fn safe_entry_name(entry_name: &str) -> Result<PathBuf, String> {
    if entry_name.is_empty() || entry_name.contains('\0') {
        return Err(format!("Unsafe zip entry path: {entry_name}"));
    }
    let path = Path::new(entry_name);
    if path.is_absolute() || entry_name.starts_with('/') || entry_name.starts_with('\\') {
        return Err(format!("Unsafe zip entry path: {entry_name}"));
    }
    let mut safe = PathBuf::new();
    for part in entry_name
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
    {
        if part == ".." {
            return Err(format!("Unsafe zip entry path: {entry_name}"));
        }
        safe.push(part);
    }
    if safe.as_os_str().is_empty() {
        return Err(format!("Unsafe zip entry path: {entry_name}"));
    }
    Ok(safe)
}

/// Where an archive entry may be written, relative to an install directory.
///
/// This is the canonicalize-then-`starts_with` shape [`PathGuard::resolve`]
/// generalises; it stays here because it answers a narrower question — a zip
/// entry is always relative, so it needs no `~` expansion and no root set.
fn safe_entry_target(base: &Path, entry_name: &str) -> Result<PathBuf, String> {
    let relative = safe_entry_name(entry_name)?;
    let base = base
        .canonicalize()
        .unwrap_or_else(|_| base.to_path_buf())
        .components()
        .collect::<PathBuf>();
    let target = base.join(relative);
    if target != base && target.starts_with(&base) {
        Ok(target)
    } else {
        Err(format!("Unsafe zip entry path: {entry_name}"))
    }
}

fn parse_skill_frontmatter(content: &str) -> Option<(String, String)> {
    let mut lines = content.lines();
    if lines.next()? != "---" {
        return None;
    }
    let mut name = None;
    let mut description = None;
    for line in lines {
        if line == "---" {
            break;
        }
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("description:") {
            description = Some(value.trim().to_string());
        }
    }
    match (name, description) {
        (Some(name), Some(description)) if !name.is_empty() && !description.is_empty() => {
            Some((name, description))
        }
        _ => None,
    }
}

fn entry_text(entry: &ArchiveEntry) -> String {
    String::from_utf8_lossy(&entry.data).to_string()
}

fn validate_brxt_bundle(path: &Path) -> Result<serde_json::Value, String> {
    let entries = read_archive(path)?;
    require_bundle_shape(&entries)?;

    let manifest_entry = entries
        .iter()
        .find(|entry| entry.name == "manifest.json")
        .ok_or_else(|| "Could not read manifest.json".to_string())?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_entry.data)
        .map_err(|e| format!("Failed to read bundle: {e}"))?;
    require_manifest_fields(&manifest)?;

    let mut skills_preview = Vec::new();
    for entry in &entries {
        let parts: Vec<_> = entry.name.split('/').collect();
        if parts.len() == 3 && parts[0] == "skills" && parts[2] == "SKILL.md" {
            if let Some((name, description)) = parse_skill_frontmatter(&entry_text(entry)) {
                skills_preview.push(serde_json::json!({
                    "slug": parts[1],
                    "name": name,
                    "description": description,
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "manifest": manifest,
        "skillsPreview": skills_preview,
    }))
}

fn require_bundle_shape(entries: &[ArchiveEntry]) -> Result<(), String> {
    if !entries.iter().any(|entry| entry.name == "manifest.json") {
        return Err("Missing manifest.json: not a valid .brxt bundle".to_string());
    }
    if !entries
        .iter()
        .any(|entry| entry.name.eq_ignore_ascii_case("README.md"))
    {
        return Err("Missing README.md: not a valid .brxt bundle".to_string());
    }
    if !entries.iter().any(|entry| entry.name == "pyproject.toml") {
        return Err("Missing pyproject.toml: not a valid .brxt bundle".to_string());
    }
    if !entries.iter().any(|entry| entry.name.starts_with("src/")) {
        return Err("Missing src/ directory: not a valid .brxt bundle".to_string());
    }
    Ok(())
}

fn require_manifest_fields(manifest: &serde_json::Value) -> Result<(), String> {
    for field in [
        "name",
        "display_name",
        "description",
        "version",
        "entry_point",
        "repository",
    ] {
        if manifest
            .get(field)
            .and_then(|value| value.as_str())
            .is_none_or(|value| value.trim().is_empty())
        {
            return Err(format!("manifest.json missing required field: \"{field}\""));
        }
    }
    if !manifest
        .get("env_vars")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err("manifest.json \"env_vars\" must be an array".to_string());
    }
    Ok(())
}

async fn install_brxt_bundle(file_path: &Path, extension_name: &str) -> anyhow::Result<PathBuf> {
    validate_extension_name(extension_name)?;
    let base = extensions_dir();
    let install_dir = base.join(extension_name);
    if !install_dir.starts_with(&base) {
        return Err(anyhow::anyhow!("Invalid extension name."));
    }
    tokio::fs::create_dir_all(&install_dir).await?;
    let entries = read_archive(file_path).map_err(|e| anyhow::anyhow!(e))?;
    for entry in entries {
        let target =
            safe_entry_target(&install_dir, &entry.name).map_err(|e| anyhow::anyhow!(e))?;
        if entry.is_dir {
            tokio::fs::create_dir_all(&target).await?;
        } else {
            if let Some(parent) = target.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(&target, entry.data).await?;
        }
    }
    run_uv_sync(&install_dir).await?;
    Ok(install_dir)
}

async fn run_uv_sync(install_dir: &Path) -> anyhow::Result<()> {
    let search_path = command_path(home_dir().as_deref());
    let mut command = Command::new(resolve_program("uv", &search_path));
    command
        .arg("sync")
        .current_dir(install_dir)
        .env("PATH", &search_path);
    // `HOME` is meaningful to `uv` on Unix and is not the variable Windows uses;
    // setting it there would point the tool at nothing.
    #[cfg(unix)]
    if let Some(home) = home_dir() {
        command.env("HOME", home);
    }

    let output = match tokio::time::timeout(UV_SYNC_TIMEOUT, command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return Err(anyhow::anyhow!("uv sync failed: {e}")),
        Err(_) => {
            return Err(anyhow::anyhow!(
                "uv sync timed out after {} minutes",
                UV_SYNC_TIMEOUT.as_secs() / 60
            ));
        }
    };
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = if detail.trim().is_empty() {
        String::from_utf8_lossy(&output.stdout).to_string()
    } else {
        detail.to_string()
    };
    Err(anyhow::anyhow!("uv sync failed: {}", detail.trim()))
}

fn uninstall_brxt_extension(extension_name: &str) -> anyhow::Result<()> {
    validate_extension_name(extension_name)?;
    let extensions_base = extensions_dir();
    let install_dir = extensions_base.join(extension_name);
    if !install_dir.starts_with(&extensions_base) {
        return Err(anyhow::anyhow!("Invalid extension name."));
    }
    if install_dir.exists() {
        std::fs::remove_dir_all(install_dir)?;
    }
    Ok(())
}

fn validate_extension_name(extension_name: &str) -> anyhow::Result<()> {
    if extension_name.is_empty()
        || extension_name == "."
        || extension_name == ".."
        || extension_name.contains('/')
        || extension_name.contains('\\')
    {
        return Err(anyhow::anyhow!("Invalid extension name."));
    }
    Ok(())
}

/// The `PATH` an installed extension's `uv sync` runs under.
///
/// The original built this by joining with a literal `:` and hardcoding
/// `~/.local/bin`, neither of which is right on Windows. `join_paths` uses the
/// platform separator and refuses a directory that contains one, which is the
/// case the manual join silently corrupted.
fn command_path(home: Option<&Path>) -> OsString {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(home) = home {
        dirs.push(home.join(".local").join("bin"));
    }
    // Unix-only: these three are meaningless on Windows, and prepending them
    // would just be noise in front of the inherited PATH.
    #[cfg(unix)]
    {
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/usr/bin"));
        dirs.push(PathBuf::from("/bin"));
    }
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    dirs.extend(std::env::split_paths(&inherited));
    std::env::join_paths(dirs).unwrap_or(inherited)
}

/// Find `name` on `search_path`, honouring the platform's executable suffix.
///
/// Needed because setting `PATH` through `Command::env` does not reliably
/// change where the program itself is looked up — so the search happens here,
/// against the `PATH` this surface actually built, and the child is given a
/// resolved path. `EXE_SUFFIX` is empty on Unix and `.exe` on Windows; the
/// original never appended it anywhere.
fn resolve_program(name: &str, search_path: &OsStr) -> PathBuf {
    let file_name = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(search_path)
        .map(|dir| dir.join(&file_name))
        .find(|candidate| candidate.is_file())
        // Nothing on the constructed PATH: hand the bare name to the platform
        // and let its own resolution have the last word.
        .unwrap_or_else(|| PathBuf::from(name))
}

// ---------------------------------------------------------------------------
// Router.
// ---------------------------------------------------------------------------

/// The seventeen `/headless/*` routes.
///
/// No handler reads [`AppState`]: this surface is about the machine the daemon
/// runs on, not about its sessions. The parameter is kept so the module is
/// merged the same way every other one is.
pub fn routes(_state: Arc<AppState>) -> Router {
    Router::new()
        .route("/headless/health", get(health))
        .route(
            "/headless/settings",
            get(settings_read).post(settings_write),
        )
        .route("/headless/registry/download", post(registry_download))
        .route("/headless/brxt/validate", post(brxt_validate))
        .route("/headless/brxt/install", post(brxt_install))
        .route("/headless/brxt/uninstall", post(brxt_uninstall))
        .route("/headless/fs/roots", get(fs_roots))
        .route("/headless/fs/list", get(fs_list))
        .route("/headless/fs/list-files", get(fs_list_files))
        .route("/headless/fs/list-dirs", get(fs_list_dirs))
        .route("/headless/fs/read", get(fs_read))
        .route("/headless/fs/artifact", get(fs_artifact))
        .route("/headless/fs/write", post(fs_write))
        .route("/headless/fs/ensure-dir", post(fs_ensure_dir))
        .route("/headless/fs/delete-file", post(fs_delete_file))
        .route("/headless/fs/delete-dir", post(fs_delete_dir))
        // Stated, finite, and applied to the whole surface at once. See
        // [`MAX_BODY_BYTES`].
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    /// A guard whose only root is `root`, with `root/home` standing in for the
    /// home directory. Nothing here reads process-wide state, so these tests
    /// neither race each other nor depend on the developer's machine.
    fn guard_over(root: &Path) -> PathGuard {
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        PathGuard {
            roots: vec![canonical_prefix(root).unwrap()],
            home: Some(canonical_prefix(&home).unwrap()),
            cwd: canonical_prefix(&home).unwrap(),
        }
    }

    fn office_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    /// A legitimate read inside the allowlist works.
    ///
    /// This one passes against the original implementation too, and that is the
    /// point of having it: without a positive control, every refusal test below
    /// would also pass against a guard that refused everything, which would be
    /// a broken surface rather than a secure one.
    #[tokio::test]
    async fn a_read_inside_a_root_succeeds() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let target = tmp.path().join("home").join("notes.md");
        fs::write(&target, "hello").unwrap();

        let response = read_file_within(&guard, target.to_str().unwrap())
            .await
            .expect("a file inside a root must resolve");
        assert!(response.found);
        assert_eq!(response.file, "hello");
    }

    #[test]
    fn artifact_route_returns_preview_bytes_and_honors_biorouterignore() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let home = tmp.path().join("home");
        let visible = home.join("visible.png");
        fs::write(&visible, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]).unwrap();

        let value = read_artifact_within(&guard, visible.to_str().unwrap()).unwrap();
        assert_eq!(value["kind"], "image");
        assert_eq!(value["mimeType"], "image/png");
        assert!(value["dataUrl"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));

        let tiff = home.join("visible.tiff");
        fs::write(&tiff, b"II*\0fixture").unwrap();
        let value = read_artifact_within(&guard, tiff.to_str().unwrap()).unwrap();
        assert_eq!(value["mimeType"], "image/tiff");
        assert!(value["dataBase64"].is_string());
        assert!(value.get("dataUrl").is_none());

        fs::write(home.join(".biorouterignore"), "private.png\n").unwrap();
        let private = home.join("private.png");
        fs::write(&private, b"not public").unwrap();
        assert_eq!(
            read_artifact_within(&guard, private.to_str().unwrap()),
            Err(Refusal::Denied)
        );
    }

    #[test]
    fn artifact_route_honors_the_requested_projects_ignore_file() {
        let tmp = TempDir::new().unwrap();
        let mut guard = guard_over(tmp.path());
        guard.cwd = canonical_prefix(&tmp.path().join("daemon")).unwrap();
        fs::create_dir_all(&guard.cwd).unwrap();

        let project = tmp.path().join("project");
        fs::create_dir_all(project.join("reports")).unwrap();
        fs::write(project.join(".biorouterignore"), "reports/private.pdf\n").unwrap();
        let private = project.join("reports/private.pdf");
        fs::write(&private, b"private report").unwrap();

        assert_eq!(
            read_artifact_within(&guard, private.to_str().unwrap()),
            Err(Refusal::Denied)
        );
    }

    #[test]
    fn office_preview_rejects_implausible_workbook_ranges() {
        let bytes = office_zip(&[
            ("xl/workbook.xml", b"<workbook/>"),
            (
                "xl/worksheets/sheet1.xml",
                b"<worksheet><dimension ref=\"A1:XFD1048576\"/></worksheet>",
            ),
        ]);
        assert_eq!(
            validate_office_archive(&bytes, "xlsx"),
            Err(Refusal::Unusable)
        );
    }

    /// The range reader indexes by byte, so a multi-byte character anywhere in
    /// the part must not land it mid-codepoint.
    #[test]
    fn office_preview_range_guard_accepts_non_ascii_worksheet_xml() {
        assert_eq!(
            spreadsheet_used_range(
                "<worksheet><note>étude</note><dimension ref=\"λA1:C5\"/></worksheet>"
            ),
            Some((3, 5))
        );
    }

    #[test]
    fn office_preview_accepts_a_bounded_presentation_shape() {
        let bytes = office_zip(&[
            ("ppt/presentation.xml", b"<p:presentation/>"),
            ("ppt/slides/slide1.xml", b"<p:sld/>"),
        ]);
        assert_eq!(validate_office_archive(&bytes, "pptx"), Ok(()));
    }

    /// Stored rather than deflated, so a fixture built from repetitive markup
    /// is not refused by the compression-ratio guard for a reason that has
    /// nothing to do with the ceiling under test.
    fn bulky_office_zip(entries: &[(String, String)]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            writer.start_file(name.as_str(), options).unwrap();
            writer.write_all(data.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn workbook_of(sheets: Vec<String>) -> Vec<u8> {
        let mut entries = vec![("xl/workbook.xml".to_string(), "<workbook/>".to_string())];
        for (index, sheet) in sheets.into_iter().enumerate() {
            entries.push((format!("xl/worksheets/sheet{}.xml", index + 1), sheet));
        }
        bulky_office_zip(&entries)
    }

    /// A 128-byte PNG header stating whatever dimensions the caller wants.
    /// Nothing decodes it — the point is that nothing has to.
    fn png_declaring(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    /// A link where the guard validated a regular file is the state a won race
    /// leaves behind, and the reason it is worth winning: `/tmp` is one of the
    /// roots, so any local user can keep flipping the name until an open lands
    /// between the check and the read.
    #[cfg(unix)]
    #[test]
    fn a_link_swapped_in_after_validation_is_refused_rather_than_followed() {
        let tmp = TempDir::new().unwrap();
        let secret = tmp.path().join("secrets.yaml");
        fs::write(&secret, "OPENAI_API_KEY: real").unwrap();
        let requested = tmp.path().join("report.pdf");
        fs::write(&requested, b"%PDF-1.7 benign").unwrap();

        let (_, metadata) =
            open_validated_file(&requested).expect("the validated regular file must open");
        assert_eq!(metadata.len(), 15);

        fs::remove_file(&requested).unwrap();
        std::os::unix::fs::symlink(&secret, &requested).unwrap();

        // The instrument first: a plain open of the same path — which is what
        // this route did — hands back the credential store.
        assert_eq!(
            fs::read_to_string(&requested).unwrap(),
            "OPENAI_API_KEY: real"
        );
        assert_eq!(open_validated_file(&requested).err(), Some(Refusal::Denied));
    }

    /// The identity check behind the refusal above: a second *name* for one
    /// inode is the same file, and a different inode is not, however alike the
    /// two names look.
    #[cfg(unix)]
    #[test]
    fn a_descriptor_is_only_accepted_when_it_is_the_file_that_was_validated() {
        let tmp = TempDir::new().unwrap();
        let one = tmp.path().join("one.txt");
        let two = tmp.path().join("two.txt");
        let also_one = tmp.path().join("also-one.txt");
        fs::write(&one, "one").unwrap();
        fs::write(&two, "two").unwrap();
        fs::hard_link(&one, &also_one).unwrap();

        let one = fs::symlink_metadata(&one).unwrap();
        assert!(is_same_file(
            &one,
            &fs::symlink_metadata(&also_one).unwrap()
        ));
        assert!(!is_same_file(&one, &fs::symlink_metadata(&two).unwrap()));
    }

    /// A link the guard cannot canonicalize — its target does not exist — gives
    /// the root test and the deny list no target to see, so the whole route has
    /// to refuse it rather than reach through it. `canonical_prefix` used to
    /// hand such a link to the opener verbatim, and the opener's own link check
    /// was all that refused it here; since QA-D F6 the guard refuses it for
    /// every route, and the opener's check stands behind it for a link swapped
    /// in after validation.
    #[cfg(unix)]
    #[test]
    fn artifact_read_refuses_a_link_at_the_final_component() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let home = tmp.path().join("home");
        let link = home.join("report.txt");
        std::os::unix::fs::symlink(home.join("not-yet.txt"), &link).unwrap();

        assert_eq!(
            read_artifact_within(&guard, link.to_str().unwrap()),
            Err(Refusal::Denied)
        );
    }

    #[test]
    fn artifact_read_refuses_a_credential_store_inside_a_root() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let secrets = tmp.path().join("home").join("secrets.yaml");
        fs::write(&secrets, "OPENAI_API_KEY: real").unwrap();

        assert_eq!(
            read_artifact_within(&guard, secrets.to_str().unwrap()),
            Err(Refusal::Denied)
        );
    }

    /// The case a dimension-only cap misses: both sides are inside
    /// [`MAX_IMAGE_DIMENSION`] and the file is a few dozen bytes, yet the
    /// renderer is being asked for twice [`MAX_IMAGE_PIXELS`].
    #[test]
    fn an_image_under_every_other_ceiling_is_refused_on_total_pixels() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let home = tmp.path().join("home");

        let bomb = home.join("bomb.png");
        fs::write(&bomb, png_declaring(8_000, 8_000)).unwrap();
        assert_eq!(
            read_artifact_within(&guard, bomb.to_str().unwrap()),
            Err(Refusal::Unusable)
        );

        let wide = home.join("wide.png");
        fs::write(&wide, png_declaring(9_000, 10)).unwrap();
        assert_eq!(
            read_artifact_within(&guard, wide.to_str().unwrap()),
            Err(Refusal::Unusable)
        );

        // Exactly at the pixel ceiling, so the refusals above are a ceiling
        // rather than a blanket.
        let allowed = home.join("large.png");
        fs::write(&allowed, png_declaring(8_000, 4_000)).unwrap();
        let value = read_artifact_within(&guard, allowed.to_str().unwrap()).unwrap();
        assert_eq!(value["kind"], "image");
    }

    /// `size:mtime` is reproducible by anyone who can write the file: a
    /// same-size edit inside one modification-time tick collides with the
    /// revision the panel is displaying, and the consumer's staleness check is
    /// nothing but a comparison of these strings.
    #[test]
    fn two_files_sharing_a_size_and_an_mtime_still_get_different_revisions() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let home = tmp.path().join("home");
        let one = home.join("one.txt");
        let two = home.join("two.txt");
        fs::write(&one, "aaaaa").unwrap();
        fs::write(&two, "bbbbb").unwrap();
        let stamp = std::fs::FileTimes::new()
            .set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        for path in [&one, &two] {
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(stamp)
                .unwrap();
        }

        let first = read_artifact_within(&guard, one.to_str().unwrap()).unwrap();
        let second = read_artifact_within(&guard, two.to_str().unwrap()).unwrap();
        let first = first["revision"].as_str().unwrap().to_string();
        let second = second["revision"].as_str().unwrap().to_string();
        let (first_stamp, first_digest) = first.rsplit_once(':').unwrap();
        let (second_stamp, second_digest) = second.rsplit_once(':').unwrap();

        // Assert the collision really was set up. Without this the test would
        // pass against a revision that is still nothing but size and mtime.
        assert_eq!(first_stamp, "5:1700000000000");
        assert_eq!(first_stamp, second_stamp);
        assert_ne!(first_digest, second_digest);
    }

    /// A revision leaves the daemon, so it must not double as a hash anyone
    /// can test a guess at the file's contents against.
    #[test]
    fn a_revision_never_carries_an_unkeyed_digest_of_the_content() {
        use sha2::Digest as _;
        let content = b"the user's local file";
        let revision = artifact_revision(content.len() as u64, 0, content);
        assert!(!revision.contains(&hex::encode(Sha256::digest(content))));
    }

    /// The binary branches used to omit `revision` entirely, and the consumer
    /// fails closed on its absence.
    #[test]
    fn every_artifact_kind_carries_a_revision() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let opaque = tmp.path().join("home").join("archive.bin");
        fs::write(&opaque, b"\0\x01\x02").unwrap();

        let value = read_artifact_within(&guard, opaque.to_str().unwrap()).unwrap();
        assert_eq!(value["kind"], "binary");
        assert!(value["revision"].as_str().unwrap().starts_with("3:"));
    }

    /// The renderer's response type has always declared these two fields and
    /// this surface never produced them, so `workspace_read_panel` returned
    /// nothing at all for a document opened through browser access.
    #[test]
    fn office_preview_extracts_document_text() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let path = tmp.path().join("home").join("report.docx");
        fs::write(
            &path,
            office_zip(&[(
                "word/document.xml",
                b"<w:document><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p>\
                  <w:p><w:r><w:t>World &amp; welcome</w:t></w:r></w:p></w:body></w:document>",
            )]),
        )
        .unwrap();

        let value = read_artifact_within(&guard, path.to_str().unwrap()).unwrap();
        assert_eq!(value["format"], "docx");
        assert_eq!(value["extractedText"], "Hello\nWorld & welcome");
        assert_eq!(value["textTruncated"], false);
    }

    #[test]
    fn office_preview_clips_extracted_text_at_the_cap() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let body = "abcdefghij".repeat(MAX_OFFICE_TEXT_CHARS / 10 + 500);
        let path = tmp.path().join("home").join("long.docx");
        fs::write(
            &path,
            office_zip(&[(
                "word/document.xml",
                format!("<w:document><w:body><w:p><w:t>{body}</w:t></w:p></w:body></w:document>")
                    .as_bytes(),
            )]),
        )
        .unwrap();

        let value = read_artifact_within(&guard, path.to_str().unwrap()).unwrap();
        assert_eq!(value["textTruncated"], true);
        assert_eq!(
            value["extractedText"].as_str().unwrap().chars().count(),
            MAX_OFFICE_TEXT_CHARS
        );
    }

    /// A cell of type `s` holds an index, not text; without the shared-string
    /// table a workbook flattens to a column of integers.
    #[test]
    fn workbook_text_resolves_shared_strings() {
        let bytes = office_zip(&[
            ("xl/workbook.xml", b"<workbook/>"),
            (
                "xl/sharedStrings.xml",
                b"<sst><si><t>Alpha</t></si><si><t>Beta</t></si></sst>",
            ),
            (
                "xl/worksheets/sheet1.xml",
                b"<worksheet><sheetData><row><c r=\"A1\" t=\"s\"><v>1</v></c>\
                  <c r=\"B1\"><v>42</v></c></row></sheetData></worksheet>",
            ),
        ]);

        let extracted = extract_office_text(&bytes, "xlsx").unwrap();
        assert_eq!(extracted.text, "[Sheet 1]\nA1: Beta\nB1: 42");
        assert!(!extracted.truncated);
    }

    /// Per-sheet ceilings bound one sheet. A workbook is N of them, and N
    /// sheets that each sit under every per-sheet line is the shape that used
    /// to pass.
    #[test]
    fn office_preview_refuses_a_used_range_that_only_exceeds_the_limit_in_aggregate() {
        // 26 × 8000 = 208 000 cells per sheet: inside every per-sheet ceiling.
        let sheet = || "<worksheet><dimension ref=\"A1:Z8000\"/></worksheet>".to_string();

        let two = workbook_of(vec![sheet(), sheet()]);
        assert_eq!(validate_office_archive(&two, "xlsx"), Ok(()));

        let three = workbook_of(vec![sheet(), sheet(), sheet()]);
        assert_eq!(
            validate_office_archive(&three, "xlsx"),
            Err(Refusal::Unusable)
        );
    }

    #[test]
    fn office_preview_refuses_populated_cells_that_only_exceed_the_limit_in_aggregate() {
        let per_sheet = MAX_WORKBOOK_POPULATED_CELLS / 2 + 1_000;
        let sheet = || {
            format!(
                "<worksheet><sheetData>{}</sheetData></worksheet>",
                "<c r=\"A1\"/>".repeat(per_sheet)
            )
        };
        assert_eq!(
            populated_cell_count(&sheet()),
            per_sheet,
            "the fixture must actually carry the cells it claims"
        );

        let bytes = workbook_of(vec![sheet(), sheet()]);
        assert_eq!(
            validate_office_archive(&bytes, "xlsx"),
            Err(Refusal::Unusable)
        );
    }

    #[test]
    fn office_preview_caps_the_number_of_worksheets() {
        let sheet = || "<worksheet/>".to_string();

        let allowed = workbook_of(vec![sheet(); MAX_WORKBOOK_WORKSHEETS]);
        assert_eq!(validate_office_archive(&allowed, "xlsx"), Ok(()));

        let refused = workbook_of(vec![sheet(); MAX_WORKBOOK_WORKSHEETS + 1]);
        assert_eq!(
            validate_office_archive(&refused, "xlsx"),
            Err(Refusal::Unusable)
        );
    }

    /// An absolute path outside every root is refused with `403`.
    ///
    /// On the original this returned `200 {found: true}` with the file's
    /// contents: `fs_read` performed no validation of any kind — it expanded
    /// `~` and called `read_to_string`. The file below stands in for
    /// `/etc/passwd`; using a real temporary file rather than the machine's own
    /// `/etc/passwd` keeps the test honest on a machine where that file is
    /// unreadable for unrelated reasons, which would otherwise let a broken
    /// guard pass.
    #[tokio::test]
    async fn a_read_outside_every_root_is_refused() {
        let allowed = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let secret = elsewhere.path().join("passwd");
        fs::write(&secret, "root:x:0:0").unwrap();

        let guard = guard_over(allowed.path());
        let refusal = read_file_within(&guard, secret.to_str().unwrap())
            .await
            .expect_err("a file outside every root must be refused");
        assert_eq!(refusal, Refusal::Outside);
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
    }

    /// A symbolic link inside a root that points outside one is refused.
    ///
    /// This is the case an allowlist checked against the *requested string*
    /// cannot catch, and it is why resolution canonicalizes before comparing.
    /// The original had no comparison at all, so it followed the link and
    /// returned the target's contents; a naive `starts_with` on the requested
    /// path would have done the same, because the requested path really is
    /// inside the root.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_symlink_that_escapes_a_root_is_refused() {
        let allowed = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let secret = elsewhere.path().join("id_rsa_target");
        fs::write(&secret, "PRIVATE KEY").unwrap();

        let guard = guard_over(allowed.path());
        let link = allowed.path().join("home").join("innocent.txt");
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        // The link itself is inside the root, so a string comparison passes.
        assert!(link.starts_with(allowed.path()));

        let refusal = read_file_within(&guard, link.to_str().unwrap())
            .await
            .expect_err("a symlink out of the allowlist must be refused");
        assert_eq!(refusal, Refusal::Outside);
    }

    /// A symlinked *directory* is refused too, including for a file under it
    /// that does not exist — the deepest-existing-ancestor rule has to resolve
    /// the link, not skip it.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_directory_does_not_launder_a_path() {
        let allowed = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        let guard = guard_over(allowed.path());

        let link = allowed.path().join("home").join("out");
        std::os::unix::fs::symlink(elsewhere.path(), &link).unwrap();

        assert_eq!(
            guard.resolve(link.join("existing_or_not.txt").to_str().unwrap()),
            Err(Refusal::Outside)
        );
    }

    // --- Links the guard cannot resolve (QA-D F6) ---------------------------

    /// What a caller of `/headless/fs/read` receives for `requested`, through
    /// the route's own mapping: the status and the body.
    async fn read_answer(guard: &PathGuard, requested: &Path) -> (StatusCode, String) {
        let result = read_file_within(guard, requested.to_str().unwrap()).await;
        answer_parts(into_answer(result)).await
    }

    /// The same, for `/headless/fs/artifact`.
    async fn artifact_answer(guard: &PathGuard, requested: &Path) -> (StatusCode, String) {
        answer_parts(into_answer(read_artifact_within(
            guard,
            requested.to_str().unwrap(),
        )))
        .await
    }

    async fn answer_parts(response: Response) -> (StatusCode, String) {
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    /// QA-D F6, as reported. A link placed inside a root and aimed into a
    /// credential store answered `403` when its target existed and
    /// `200 {"found":false}` when it did not, so `/headless/fs/read` said
    /// whether a file exists about exactly the paths the deny list is there to
    /// say nothing about. `canonical_prefix` stepped past the dangling link as
    /// if it were a name not created yet, and the guard compared the link's own
    /// location (inside a root, naming nothing denied) instead of its target.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dangling_link_into_a_credential_store_answers_what_a_live_one_does() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let home = tmp.path().join("home");
        let ssh = home.join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        fs::write(ssh.join("known_hosts"), "github.com ssh-ed25519 AAAA").unwrap();

        let live = home.join("probe-live.txt");
        std::os::unix::fs::symlink(ssh.join("known_hosts"), &live).unwrap();
        let dangling = home.join("probe-dangling.txt");
        std::os::unix::fs::symlink(ssh.join("nonexistent"), &dangling).unwrap();

        let refused = read_answer(&guard, &live).await;
        assert_eq!(
            refused.0,
            StatusCode::FORBIDDEN,
            "a live link into .ssh is refused, as it always was"
        );
        assert_eq!(
            read_answer(&guard, &dangling).await,
            refused,
            "whether a file in a credential store exists must not change the answer"
        );
        // The artifact route already gave these one answer; it must keep to it.
        let refused = artifact_answer(&guard, &live).await;
        assert_eq!(refused.0, StatusCode::FORBIDDEN);
        assert_eq!(artifact_answer(&guard, &dangling).await, refused);
    }

    /// The same link aimed into a directory the guard allows is refused as a
    /// link. That is the classification `/headless/fs/artifact` already gave it
    /// (`artifact_read_refuses_a_link_at_the_final_component`), now given by
    /// the guard itself, so it holds for every route. What is refused is the
    /// link rather than its target, which is why the answer cannot depend on
    /// whether the target exists.
    ///
    /// Named directly, the missing target is an answer and not a refusal —
    /// `200 {"found":false}` — which is the contract the renderer's optional
    /// reads (skill overrides, `SKILL.md`) are written against.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dangling_link_into_an_allowed_directory_is_refused_as_a_link() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let home = tmp.path().join("home");
        let missing = home.join("not-yet.txt");
        let link = home.join("report.txt");
        std::os::unix::fs::symlink(&missing, &link).unwrap();

        assert_eq!(
            read_file_within(&guard, link.to_str().unwrap()).await.err(),
            Some(Refusal::Denied)
        );
        let direct = read_file_within(&guard, missing.to_str().unwrap())
            .await
            .expect("a missing file inside a root is an answer, not a refusal");
        assert!(!direct.found);
        assert!(direct.error.is_some());
    }

    /// One level up, the same hole had a second shape, and there the artifact
    /// route leaked too. A directory link whose target is missing made both
    /// routes act on the link's own location: `fs_read` answered
    /// `200 {"found":false}` and `fs_artifact` `400`, where a link to a
    /// credential store that exists got `403`. So whether `~/.aws` exists (does
    /// this machine hold cloud credentials at all) could be read off either
    /// route.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_dangling_directory_link_does_not_say_whether_its_target_exists() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join(".ssh")).unwrap();
        let to_existing = home.join("keys");
        std::os::unix::fs::symlink(home.join(".ssh"), &to_existing).unwrap();
        let to_missing = home.join("cloud");
        std::os::unix::fs::symlink(home.join(".aws"), &to_missing).unwrap();

        let refused = read_answer(&guard, &to_existing.join("config")).await;
        assert_eq!(refused.0, StatusCode::FORBIDDEN);
        assert_eq!(
            read_answer(&guard, &to_missing.join("credentials")).await,
            refused
        );

        let refused = artifact_answer(&guard, &to_existing.join("config")).await;
        assert_eq!(refused.0, StatusCode::FORBIDDEN);
        assert_eq!(
            artifact_answer(&guard, &to_missing.join("credentials")).await,
            refused
        );
    }

    /// Outside every root the trick answered "does this path exist" about the
    /// rest of the machine: a live link to an existing file was refused
    /// `Outside`, and a dangling one read as `200 {"found":false}`. With the
    /// link refused the status agrees. The two boundary refusals also share one
    /// body, because which of them applies is decided by where a link the
    /// caller planted resolves, and two wordings would say it in words.
    #[cfg(unix)]
    #[tokio::test]
    async fn outside_every_root_a_dangling_link_answers_what_a_live_one_does() {
        let allowed = TempDir::new().unwrap();
        let elsewhere = TempDir::new().unwrap();
        fs::write(elsewhere.path().join("passwd"), "root:x:0:0").unwrap();
        let guard = guard_over(allowed.path());
        let home = allowed.path().join("home");

        let live = home.join("probe-live");
        std::os::unix::fs::symlink(elsewhere.path().join("passwd"), &live).unwrap();
        let dangling = home.join("probe-dangling");
        std::os::unix::fs::symlink(elsewhere.path().join("definitely-absent"), &dangling).unwrap();

        // Two different refusals underneath, which the tests can still tell
        // apart...
        assert_eq!(guard.resolve(live.to_str().unwrap()), Err(Refusal::Outside));
        assert_eq!(
            guard.resolve(dangling.to_str().unwrap()),
            Err(Refusal::Denied)
        );
        // ...and one answer on the wire, which a caller cannot.
        let refused = read_answer(&guard, &live).await;
        assert_eq!(refused.0, StatusCode::FORBIDDEN);
        assert_eq!(read_answer(&guard, &dangling).await, refused);
    }

    /// The write half of the same defect, and the reason the fix is in the
    /// guard every handler resolves through rather than in `fs_read` alone.
    /// `fs_write` acts on the path `resolve` returns, which for a dangling link
    /// was the link, and writing to a link creates its missing target. So a
    /// credential store could be written into through a link planted anywhere
    /// in a root, past the deny list whose whole job is to keep this surface
    /// out of it.
    #[cfg(unix)]
    #[test]
    fn a_dangling_link_cannot_carry_a_write_into_a_credential_store() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let home = tmp.path().join("home");
        let ssh = home.join(".ssh");
        fs::create_dir_all(&ssh).unwrap();
        let planted = home.join("notes.txt");
        std::os::unix::fs::symlink(ssh.join("authorized_keys"), &planted).unwrap();

        // The instrument first: writing to the link's own path, which is what
        // `fs_write` did with what the guard handed it, lands inside `.ssh`.
        fs::write(&planted, "ssh-ed25519 AAAA attacker").unwrap();
        assert!(ssh.join("authorized_keys").is_file());
        fs::remove_file(ssh.join("authorized_keys")).unwrap();

        assert_eq!(
            guard.resolve(planted.to_str().unwrap()),
            Err(Refusal::Denied)
        );
    }

    /// `fs_read` now opens the way the artifact route does. It returns
    /// contents, so a validated file flipped to a link before the open was a
    /// read of the link's target: `resolve` checked the name, and the plain
    /// `read_to_string` this route did resolved it again from scratch.
    #[cfg(unix)]
    #[test]
    fn fs_read_refuses_a_link_swapped_in_after_validation() {
        let tmp = TempDir::new().unwrap();
        let secret = tmp.path().join("secrets.yaml");
        fs::write(&secret, "OPENAI_API_KEY: real").unwrap();
        let requested = tmp.path().join("notes.md");
        fs::write(&requested, "benign").unwrap();
        assert_eq!(read_validated_text(&requested).unwrap().unwrap(), "benign");

        fs::remove_file(&requested).unwrap();
        std::os::unix::fs::symlink(&secret, &requested).unwrap();

        // The instrument first: the read this route used to do hands back the
        // credential store.
        assert_eq!(
            fs::read_to_string(&requested).unwrap(),
            "OPENAI_API_KEY: real"
        );
        assert_eq!(read_validated_text(&requested).err(), Some(Refusal::Denied));
    }

    /// A loop has no target either. It is refused as a link, not stepped past
    /// as a name that is not there yet.
    #[cfg(unix)]
    #[test]
    fn a_link_loop_is_refused_as_a_link() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let home = tmp.path().join("home");
        std::os::unix::fs::symlink(home.join("b"), home.join("a")).unwrap();
        std::os::unix::fs::symlink(home.join("a"), home.join("b")).unwrap();

        assert_eq!(
            guard.resolve(home.join("a").to_str().unwrap()),
            Err(Refusal::Denied)
        );
        assert_eq!(
            guard.resolve(home.join("a").join("child").to_str().unwrap()),
            Err(Refusal::Denied)
        );
    }

    /// Traversal is folded out before the root test, so `../../..` cannot walk
    /// out of a root by pretending to stay in it.
    ///
    /// The original's `validate_file_path` accepted every string except `""`
    /// and `/`, so `~/../../etc/passwd` was accepted verbatim by `fs_write`,
    /// `fs_ensure_dir` and `fs_delete_file` — and `fs_read` did not even call
    /// it. A guard that compared the *unnormalized* string would also pass this
    /// path, since it does begin with the home directory.
    #[test]
    fn traversal_out_of_a_root_is_refused() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());

        assert_eq!(
            guard.resolve("~/../../../etc/passwd"),
            Err(Refusal::Outside)
        );
        // Climb out of the root by an amount that does NOT depend on how deep
        // the temporary directory happens to be. `{tmp}/home/../..` folds to
        // the PARENT of `tmp`, which always exists and is always outside the
        // root -- on every platform.
        //
        // A fixed `../../..` from `tmp` is not portable and was a real CI
        // failure: macOS hands out a six-component temporary path
        // (`/var/folders/xx/yyy/T/.tmpNNN`), so three levels up is still a real
        // directory and the answer is `Outside`; Linux hands out
        // `/tmp/.tmpNNN`, so the same three levels walk above `/` and the
        // answer is correctly `Malformed`. The guard was right both times --
        // the assertion was reading a property of the temporary directory
        // rather than of the guard.
        assert_eq!(
            guard.resolve(&format!("{}/home/../..", tmp.path().display())),
            Err(Refusal::Outside)
        );
        // Relative paths resolve against the working directory and are folded
        // the same way.
        assert_eq!(guard.resolve("../../../etc/passwd"), Err(Refusal::Outside));
        // Climbing above the filesystem root is refused everywhere, but it is
        // not the SAME refusal everywhere, and the difference is real rather
        // than incidental.
        //
        // On Unix `/` is the top of the single tree, so `/../../..` has nowhere
        // left to go and the path is malformed. On Windows a leading `\` means
        // "root of the current drive" and `..` there is absorbed rather than
        // running out of tree, so the path is well-formed and simply lands
        // outside every root. Asserting `Malformed` on both is asserting a
        // Unix path model on Windows -- which is what CI caught.
        //
        // Both arms are asserted exactly, not collapsed to `is_err()`: a guard
        // that refused every path would satisfy a weaker check and satisfy
        // nothing this test exists for.
        #[cfg(unix)]
        assert_eq!(guard.resolve("/../../.."), Err(Refusal::Malformed));
        #[cfg(windows)]
        assert_eq!(guard.resolve("/../../.."), Err(Refusal::Outside));
    }

    /// Traversal that stays inside a root is fine — `..` is not banned, it is
    /// resolved.
    #[test]
    fn traversal_that_stays_inside_a_root_resolves() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        fs::create_dir_all(tmp.path().join("home").join("a")).unwrap();
        let resolved = guard
            .resolve("~/a/../a")
            .expect("a path that stays inside the root must resolve");
        assert_eq!(
            resolved,
            canonical_prefix(&tmp.path().join("home/a")).unwrap()
        );
    }

    /// A sibling directory whose name merely starts with a root's name is not
    /// inside it. `Path::starts_with` is component-wise, which is what makes
    /// this hold; a `String::starts_with` would let `/tmp/root-evil` through.
    #[test]
    fn a_sibling_with_a_shared_prefix_is_outside() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("root");
        let sibling = tmp.path().join("root-evil");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        let guard = PathGuard {
            roots: vec![canonical_prefix(&root).unwrap()],
            home: Some(canonical_prefix(&root).unwrap()),
            cwd: canonical_prefix(&root).unwrap(),
        };
        assert_eq!(
            guard.resolve(sibling.join("loot").to_str().unwrap()),
            Err(Refusal::Outside)
        );
    }

    /// Credential stores are refused even though they sit inside an allowed
    /// root. The allowlist alone would admit `~/.ssh/id_rsa`, which is the
    /// second half of what the original leaked.
    #[test]
    fn credential_stores_inside_a_root_are_refused() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        for path in [
            "~/.ssh/id_rsa",
            "~/.ssh/config",
            "~/.aws/credentials",
            "~/.gnupg/secring.gpg",
            "~/secrets.yaml",
            "~/nested/dir/id_ed25519",
        ] {
            assert_eq!(
                guard.resolve(path),
                Err(Refusal::Denied),
                "{path} must be refused"
            );
        }
        // A file that merely mentions one of the names is not one of them.
        assert!(guard.resolve("~/ssh-notes.md").is_ok());
        assert!(guard.resolve("~/id_rsa_backup_notes.md").is_ok());
    }

    /// A path that does not exist yet still gets checked — otherwise every
    /// write would be unguarded, which is the whole point of resolving the
    /// deepest existing ancestor.
    #[test]
    fn a_path_that_does_not_exist_yet_is_still_placed() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let inside = guard
            .resolve("~/does/not/exist/yet.txt")
            .expect("a new file under the home root must resolve");
        assert!(inside.starts_with(canonical_prefix(tmp.path()).unwrap()));
        assert!(!inside.exists());

        let elsewhere = TempDir::new().unwrap();
        assert_eq!(
            guard.resolve(elsewhere.path().join("new/file.txt").to_str().unwrap()),
            Err(Refusal::Outside)
        );
    }

    /// An empty path is a caller error, not a filesystem root. The original
    /// refused `""` for writes only, and `fs_read` treated a missing `path`
    /// query parameter as `""`.
    #[test]
    fn an_empty_path_is_refused() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        assert_eq!(guard.resolve(""), Err(Refusal::Unusable));
    }

    /// The roots themselves resolve, and are recognised as roots so a recursive
    /// delete cannot take one.
    #[test]
    fn a_root_is_reachable_but_known_to_be_a_root() {
        let tmp = TempDir::new().unwrap();
        let guard = guard_over(tmp.path());
        let root = guard.resolve(tmp.path().to_str().unwrap()).unwrap();
        assert!(guard.is_root(&root));
        assert!(!guard.is_root(&root.join("child")));
    }

    /// A zip entry cannot climb out of its install directory. Unchanged from
    /// the original, and asserted here because `install_brxt_bundle` is the one
    /// handler that writes a path it did not resolve through the guard.
    #[test]
    fn zip_entries_cannot_escape_the_install_directory() {
        let tmp = TempDir::new().unwrap();
        assert!(safe_entry_target(tmp.path(), "../../evil").is_err());
        assert!(safe_entry_target(tmp.path(), "/etc/passwd").is_err());
        assert!(safe_entry_target(tmp.path(), "..\\..\\evil").is_err());
        assert!(safe_entry_target(tmp.path(), "src/server.py").is_ok());
    }

    /// The extension name is a single path segment, so it cannot redirect the
    /// install or the uninstall out of the extensions directory.
    #[test]
    fn an_extension_name_is_one_segment() {
        assert!(validate_extension_name("spoke-agent").is_ok());
        for bad in ["", ".", "..", "../../etc", "a/b", "a\\b"] {
            assert!(
                validate_extension_name(bad).is_err(),
                "{bad} must be rejected"
            );
        }
    }

    /// The `PATH` handed to `uv` is built with the platform separator, so it
    /// round-trips through `split_paths` on every platform. The original joined
    /// with a literal `:`, which produces one nonsense entry on Windows.
    #[test]
    fn the_command_path_uses_the_platform_separator() {
        let home = PathBuf::from(if cfg!(windows) { "C:\\Users\\x" } else { "/h" });
        let joined = command_path(Some(&home));
        let parsed: Vec<PathBuf> = std::env::split_paths(&joined).collect();
        assert_eq!(parsed.first(), Some(&home.join(".local").join("bin")));
        assert!(parsed.len() > 1);
    }

    /// The program lookup appends the platform's executable suffix, and falls
    /// back to the bare name when nothing on the path matches.
    #[test]
    fn a_program_is_looked_up_with_the_platform_suffix() {
        let tmp = TempDir::new().unwrap();
        let name = format!("uv{}", std::env::consts::EXE_SUFFIX);
        fs::write(tmp.path().join(&name), "#!/bin/sh\n").unwrap();
        let search = std::env::join_paths([tmp.path().to_path_buf()]).unwrap();
        assert_eq!(resolve_program("uv", &search), tmp.path().join(&name));

        let empty = TempDir::new().unwrap();
        let search = std::env::join_paths([empty.path().to_path_buf()]).unwrap();
        assert_eq!(resolve_program("uv", &search), PathBuf::from("uv"));
    }

    /// Only the marketplace hosts, and only over TLS.
    #[test]
    fn registry_downloads_are_host_and_scheme_bound() {
        assert!(allowed_registry_url("https://github.com/a/b.zip").is_some());
        assert!(allowed_registry_url("http://github.com/a/b.zip").is_none());
        assert!(allowed_registry_url("https://github.com.evil.test/a/b.zip").is_none());
        assert!(allowed_registry_url("file:///etc/passwd").is_none());
    }

    /// The settings document lives under the resolved configuration directory,
    /// not under a hardcoded `~/.config/biorouter`.
    #[test]
    fn the_settings_document_follows_the_configured_root() {
        let path = settings_path();
        assert!(path.starts_with(Paths::config_dir()));
        assert!(path.ends_with(Path::new("headless").join("settings.json")));
        assert!(extensions_dir().starts_with(Paths::config_dir()));
    }

    /// Every route the retired binary served is still served, at the same path.
    #[test]
    fn all_seventeen_routes_are_registered() {
        let source = include_str!("shell.rs");
        for path in [
            "/headless/health",
            "/headless/settings",
            "/headless/registry/download",
            "/headless/brxt/validate",
            "/headless/brxt/install",
            "/headless/brxt/uninstall",
            "/headless/fs/roots",
            "/headless/fs/list",
            "/headless/fs/list-files",
            "/headless/fs/list-dirs",
            "/headless/fs/read",
            "/headless/fs/artifact",
            "/headless/fs/write",
            "/headless/fs/ensure-dir",
            "/headless/fs/delete-file",
            "/headless/fs/delete-dir",
        ] {
            assert!(
                source.contains(&format!("\"{path}\"")),
                "{path} is no longer routed"
            );
        }
    }
}
