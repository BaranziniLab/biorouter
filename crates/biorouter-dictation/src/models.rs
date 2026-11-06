//! Installing, verifying and removing speech model files (DD-17, DD-21, DD-23, DD-24;
//! `docs/dictation/engine-and-models.md`, "The installer, step by step").
//!
//! One installer for the daemon and the CLI. It takes a catalog entry, never a URL, and
//! a folder, never a configuration: the core crate decides which folder (the lookup
//! order) and whether the caller may install (policy, terminal, proof).
//!
//! A file is verified by SHA-256 before anything parses it. In a writable folder
//! `installed.json` caches the verification by size and modification time, and any
//! change forces a full hash; a file that then fails is deleted. Files in the read only
//! folders (managed, env, system) are hashed once per process, cached by path, size and
//! modification time, and never deleted.
//!
//! A download is resumable: bytes go to `<models>/.partial/<id>/<file>.partial` with
//! `<file>.partial.json` beside it, on the same volume as the final file so the last
//! rename is atomic. Every attempt requests the github.com URL again, because its signed
//! redirect expires after about an hour [M]; at most 5 https redirects are followed and
//! only GitHub's two asset hosts may answer with the body. No `Authorization` header is
//! ever sent.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use crate::catalog::{has_ggml_magic, Catalog, SttCatalogEntry};

/// The verification record beside a file in a writable folder.
pub const INSTALLED_JSON: &str = "installed.json";
/// Where partial downloads wait, beside the `<id>` folders.
pub const PARTIAL_DIR: &str = ".partial";
/// One lock per id, inside its folder.
pub const LOCK_FILE: &str = ".lock";
/// Free space kept on top of what a download still needs (DD-22).
pub const DISK_MARGIN_BYTES: u64 = 256 << 20;
/// The only hosts that may answer a release download with its body [M].
pub const GITHUB_ASSET_HOSTS: [&str; 2] = [
    "release-assets.githubusercontent.com",
    "objects.githubusercontent.com",
];
pub const MAX_REDIRECTS: usize = 5;
/// Attempts in a row that made no progress before an install stops.
pub const RETRY_BUDGET: u32 = 5;
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
/// Environment only (DD-25): an https base tried before GitHub, with the same hashes.
pub const MIRROR_ENV: &str = "BIOROUTER_DICTATION_MODEL_MIRROR";

/// MSG-HASH.
pub const MSG_HASH: &str =
    "The downloaded file did not match Biorouter's pin, so nothing was installed.";

/// Where a verified copy came from (`installed.json`'s `source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Download,
    Mirror,
    /// `install --from`.
    Import,
    /// Found in place: a file put in the folder by hand, then hashed.
    Rescan,
}

/// `installed.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledRecord {
    pub id: String,
    pub sha256: String,
    pub size: u64,
    /// The file's modification time, RFC 3339 with nanoseconds, compared as text.
    pub mtime: String,
    pub verified_at: String,
    pub source: Source,
    pub release_tag: String,
}

/// Folder and file modes. A `--system` install writes 0755 folders and 0644 files, so
/// every account on the computer can read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    User,
    System,
}

/// `<root>/<id>/<file_name>`: where an installed copy lives in every folder.
pub fn model_path(root: &Path, entry: &SttCatalogEntry) -> PathBuf {
    root.join(entry.id).join(entry.file_name)
}

/// `<root>/<file_name>`: a copy an administrator dropped in flat (read only folders only).
pub fn flat_path(root: &Path, entry: &SttCatalogEntry) -> PathBuf {
    root.join(entry.file_name)
}

fn record_path(root: &Path, entry: &SttCatalogEntry) -> PathBuf {
    root.join(entry.id).join(INSTALLED_JSON)
}

fn partial_paths(root: &Path, entry: &SttCatalogEntry) -> (PathBuf, PathBuf, PathBuf) {
    let dir = root.join(PARTIAL_DIR).join(entry.id);
    let file = dir.join(format!("{}.partial", entry.file_name));
    let meta = dir.join(format!("{}.partial.json", entry.file_name));
    (dir, file, meta)
}

/// Size and modification time as `installed.json` records them.
pub fn file_stamp(path: &Path) -> io::Result<(u64, String)> {
    let meta = fs::metadata(path)?;
    if !meta.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
    }
    Ok((meta.len(), rfc3339(meta.modified()?)))
}

fn rfc3339(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
}

fn now() -> String {
    rfc3339(SystemTime::now())
}

/// Lower case hex SHA-256 of `path`.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    crate::build_info::sha256_file(path)
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What checking a copy found.
#[derive(Debug)]
pub enum Verdict {
    /// The bytes match the pin, or an accepted earlier revision (`superseded`).
    Verified {
        path: PathBuf,
        sha256: String,
        superseded: bool,
    },
    /// No file there.
    Missing,
    /// The file's hash matches no pin; in a writable folder it has been deleted.
    Mismatch { path: PathBuf, deleted: bool },
    /// The file could not be read.
    Unreadable { path: PathBuf, error: io::Error },
}

impl Verdict {
    /// The copy can be loaded: the current pin or an accepted earlier revision.
    pub fn is_verified(&self) -> bool {
        matches!(self, Verdict::Verified { .. })
    }

    /// The copy is the current revision. An earlier revision stays loadable, but an
    /// install fetches the current one over it (DD-23).
    pub fn is_current(&self) -> bool {
        matches!(
            self,
            Verdict::Verified {
                superseded: false,
                ..
            }
        )
    }
}

/// Checks the copy in a writable folder: `installed.json` decides while the size and the
/// modification time match it; any difference forces a full hash. A copy that fails is
/// deleted with its record (P-11). A copy with no record that verifies gets one.
pub fn verify_writable(root: &Path, entry: &SttCatalogEntry) -> Verdict {
    verify_writable_with(root, entry, true)
}

/// `biorouter dictation verify`: hashes the copy whatever `installed.json` says, and
/// deletes it when the hash matches no pin.
pub fn verify_writable_fully(root: &Path, entry: &SttCatalogEntry) -> Verdict {
    verify_writable_with(root, entry, false)
}

fn verify_writable_with(root: &Path, entry: &SttCatalogEntry, trust_record: bool) -> Verdict {
    let path = model_path(root, entry);
    let (size, mtime) = match file_stamp(&path) {
        Ok(stamp) => stamp,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Verdict::Missing,
        Err(error) => return Verdict::Unreadable { path, error },
    };
    let record = read_record(root, entry);
    if let Some(record) = record.as_ref().filter(|_| trust_record) {
        if record.size == size && record.mtime == mtime && entry.accepts(&record.sha256) {
            return Verdict::Verified {
                superseded: record.sha256 != entry.sha256,
                sha256: record.sha256.clone(),
                path,
            };
        }
    }
    // The pinned size is the cheap first test. An earlier revision may have another
    // size, so the size decides alone only for an entry without one.
    if size != entry.size_bytes && entry.accepted_sha256.len() == 1 {
        let deleted = fs::remove_file(&path).is_ok();
        let _ = fs::remove_file(record_path(root, entry));
        return Verdict::Mismatch { path, deleted };
    }
    match sha256_file(&path) {
        Ok(digest) if entry.accepts(&digest) => {
            let source = record.map_or(Source::Rescan, |r| r.source);
            let _ = write_record(root, entry, &digest, source, None);
            Verdict::Verified {
                superseded: digest != entry.sha256,
                sha256: digest,
                path,
            }
        }
        Ok(_) => {
            let deleted = fs::remove_file(&path).is_ok();
            let _ = fs::remove_file(record_path(root, entry));
            Verdict::Mismatch { path, deleted }
        }
        Err(error) => Verdict::Unreadable { path, error },
    }
}

/// A read only copy's path, size and mtime, and the hash they had.
type ReadonlyHashes = Mutex<HashMap<(PathBuf, u64, String), String>>;

/// The process wide cache of hashes of read only copies, by path, size and mtime.
fn readonly_cache() -> &'static ReadonlyHashes {
    static CACHE: OnceLock<ReadonlyHashes> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Checks a copy in a read only folder at `path`: hashed once per process for each size
/// and mtime, never deleted.
pub fn verify_readonly(path: &Path, entry: &SttCatalogEntry) -> Verdict {
    verify_readonly_with(path, entry, true)
}

/// `biorouter dictation verify` on a read only copy: hashed again, never deleted.
pub fn verify_readonly_fully(path: &Path, entry: &SttCatalogEntry) -> Verdict {
    verify_readonly_with(path, entry, false)
}

fn verify_readonly_with(path: &Path, entry: &SttCatalogEntry, use_cache: bool) -> Verdict {
    let path = path.to_path_buf();
    let (size, mtime) = match file_stamp(&path) {
        Ok(stamp) => stamp,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Verdict::Missing,
        Err(error) => return Verdict::Unreadable { path, error },
    };
    if size != entry.size_bytes && entry.accepted_sha256.len() == 1 {
        return Verdict::Mismatch {
            path,
            deleted: false,
        };
    }
    let key = (path.clone(), size, mtime);
    let cached = readonly_cache()
        .lock()
        .ok()
        .filter(|_| use_cache)
        .and_then(|cache| cache.get(&key).cloned());
    let digest = match cached {
        Some(digest) => digest,
        None => match sha256_file(&path) {
            Ok(digest) => {
                if let Ok(mut cache) = readonly_cache().lock() {
                    cache.insert(key, digest.clone());
                }
                digest
            }
            Err(error) => return Verdict::Unreadable { path, error },
        },
    };
    if entry.accepts(&digest) {
        Verdict::Verified {
            superseded: digest != entry.sha256,
            sha256: digest,
            path,
        }
    } else {
        Verdict::Mismatch {
            path,
            deleted: false,
        }
    }
}

pub fn read_record(root: &Path, entry: &SttCatalogEntry) -> Option<InstalledRecord> {
    let bytes = fs::read(record_path(root, entry)).ok()?;
    serde_json::from_slice::<InstalledRecord>(&bytes)
        .ok()
        .filter(|record| record.id == entry.id)
}

fn write_record(
    root: &Path,
    entry: &SttCatalogEntry,
    digest: &str,
    source: Source,
    mode: Option<WriteMode>,
) -> io::Result<()> {
    let path = model_path(root, entry);
    let (size, mtime) = file_stamp(&path)?;
    let record = InstalledRecord {
        id: entry.id.to_string(),
        sha256: digest.to_string(),
        size,
        mtime,
        verified_at: now(),
        source,
        release_tag: entry.release_tag.to_string(),
    };
    let json = serde_json::to_vec_pretty(&record).map_err(io::Error::other)?;
    write_atomically(
        &record_path(root, entry),
        &json,
        mode.unwrap_or(WriteMode::User),
    )
}

/// Writes `bytes` to `path` through a temporary file and a rename. The temporary file is
/// created new, after any stale one is unlinked, so a link planted at its name is
/// replaced, never followed.
fn write_atomically(path: &Path, bytes: &[u8], mode: WriteMode) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut file = create_fresh(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    set_file_mode(&tmp, mode)?;
    fs::rename(&tmp, path)
}

/// A new file at `path`: whatever was there is unlinked first, and the open refuses an
/// existing entry, so it never writes through a link.
fn create_fresh(path: &Path) -> io::Result<File> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Opens `path` for writing, creating it but never truncating it, and on Unix refusing a
/// symbolic link in its place (`O_NOFOLLOW`): the lock and the resumable partial.
fn open_no_follow(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn set_file_mode(path: &Path, mode: WriteMode) -> io::Result<()> {
    #[cfg(unix)]
    if mode == WriteMode::System {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o644))?;
    }
    let _ = (path, mode);
    Ok(())
}

fn create_dirs(dir: &Path, mode: WriteMode) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    if mode == WriteMode::System {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Why the computer's shared folder may not be written by root or an Administrator at
/// `dir`, if it may not; every `--system` command asks before it writes or deletes
/// anything there. `--system` must follow no link an ordinary account could have
/// planted: on Unix every existing folder on the way to `dir`, as written and as
/// resolved, must be owned by root and writable by no one else (a sticky folder such as
/// `/tmp` passes, since no one can rename what root put there), so no other account can
/// swap a part of the path for a link while the install runs. On Windows no part may be
/// a link or a junction; owners are not read there, as in the managed policy's own check.
pub fn system_folder_problem(dir: &Path) -> Option<String> {
    let mut paths: Vec<PathBuf> = dir.ancestors().map(Path::to_path_buf).collect();
    if let Ok(resolved) = fs::canonicalize(nearest_existing(dir)) {
        paths.extend(resolved.ancestors().map(Path::to_path_buf));
    }
    paths
        .into_iter()
        .find_map(|path| component_problem(&path).map(|why| format!("{} {why}", path.display())))
}

/// What is wrong with one existing part of a shared folder's path; nothing for a part
/// that does not exist yet, which the installer creates itself.
fn component_problem(path: &Path) -> Option<&'static str> {
    let link = fs::symlink_metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // A link is judged by what it names; the folder that holds it is checked too.
        let meta = if link.file_type().is_symlink() {
            fs::metadata(path).ok()?
        } else {
            link
        };
        if !meta.is_dir() {
            return Some("is not a folder");
        }
        unix_folder_rule(meta.uid(), meta.mode())
    }
    #[cfg(not(unix))]
    {
        if link.file_type().is_symlink() {
            return Some("is a link or a junction");
        }
        if !link.is_dir() {
            return Some("is not a folder");
        }
        None
    }
}

/// The rule for one folder on the way to the shared models folder, on Unix.
#[cfg(unix)]
fn unix_folder_rule(uid: u32, mode: u32) -> Option<&'static str> {
    const STICKY: u32 = 0o1000;
    if uid != 0 {
        return Some("is not owned by root");
    }
    if mode & 0o022 != 0 && mode & STICKY == 0 {
        return Some("can be changed by accounts other than root");
    }
    None
}

fn sync_dir(dir: &Path) {
    // Windows cannot open a folder this way; NTFS journals the rename itself.
    #[cfg(unix)]
    if let Ok(handle) = File::open(dir) {
        let _ = handle.sync_all();
    }
    let _ = dir;
}

/// Writes the entry's license texts beside its file (DD-18).
fn write_licenses(dir: &Path, entry: &SttCatalogEntry, mode: WriteMode) -> io::Result<()> {
    for license in entry.licenses {
        write_atomically(&dir.join(license.file_name), license.text.as_bytes(), mode)?;
    }
    Ok(())
}

/// Moves a verified file into place as `<root>/<id>/<file_name>`, then writes its
/// licenses and `installed.json` and syncs the folder. `staged` must be on the same
/// volume, inside `root`.
fn publish(
    root: &Path,
    entry: &SttCatalogEntry,
    staged: &Path,
    digest: &str,
    source: Source,
    mode: WriteMode,
) -> io::Result<PathBuf> {
    let dir = root.join(entry.id);
    create_dirs(root, mode)?;
    create_dirs(&dir, mode)?;
    File::open(staged)?.sync_all()?;
    set_file_mode(staged, mode)?;
    let target = model_path(root, entry);
    fs::rename(staged, &target)?;
    write_licenses(&dir, entry, mode)?;
    write_record(root, entry, digest, source, Some(mode))?;
    sync_dir(&dir);
    Ok(target)
}

/// Why a removal stopped.
#[derive(Debug)]
pub enum RemoveError {
    /// Another process holds the model's lock: it is installing it now.
    Busy {
        id: String,
    },
    Io {
        id: String,
        error: io::Error,
    },
}

impl std::fmt::Display for RemoveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RemoveError::Busy { id } => write!(
                f,
                "{id} is being installed by another Biorouter process; try again when it finishes."
            ),
            RemoveError::Io { id, error } => write!(f, "{id} could not be removed: {error}."),
        }
    }
}

impl std::error::Error for RemoveError {}

/// Whether another process is installing `entry` into `root` now: it holds the model's
/// lock. Opens the lock file only when it is there and never creates one. A removal asks
/// this before it takes the voice detector with the last speech model, because an install
/// that is downloading another speech model planned its files when the detector was
/// there and does not fetch it again.
pub fn install_running(root: &Path, entry: &SttCatalogEntry) -> bool {
    use fs2::FileExt;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let Ok(lock) = options.open(root.join(entry.id).join(LOCK_FILE)) else {
        return false;
    };
    // Called through the trait: `std::fs::File` has inherent methods of the same names.
    match FileExt::try_lock_shared(&lock) {
        Ok(()) => {
            let _ = FileExt::unlock(&lock);
            false
        }
        Err(e) => e.kind() == fs2::lock_contended_error().kind(),
    }
}

/// Deletes the installed copy in a writable folder: the file, its record, its license
/// texts, a staged `--from` copy, temporary files, its partial download and the folder
/// when nothing else is left in it. True when a model file was removed.
///
/// It takes the model's lock first, the one an install holds for the whole download, and
/// refuses while another process holds it: deleting a partial under a running download
/// would fail that install at its end, and unlinking its lock would let a third install
/// run beside it. The lock file goes last, while it is still held.
pub fn remove(root: &Path, entry: &SttCatalogEntry) -> Result<bool, RemoveError> {
    use fs2::FileExt;
    let io_error = |error| RemoveError::Io {
        id: entry.id.to_string(),
        error,
    };
    let dir = root.join(entry.id);
    let (partial_dir, _, _) = partial_paths(root, entry);
    let lock = if dir.is_dir() {
        let lock = open_no_follow(&dir.join(LOCK_FILE)).map_err(io_error)?;
        match lock.try_lock_exclusive() {
            Ok(()) => Some(lock),
            Err(e) if e.kind() == fs2::lock_contended_error().kind() => {
                return Err(RemoveError::Busy {
                    id: entry.id.to_string(),
                })
            }
            Err(e) => return Err(io_error(e)),
        }
    } else {
        None
    };
    let removed = match fs::remove_file(model_path(root, entry)) {
        Ok(()) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(io_error(e)),
    };
    let staged = format!(".{}.import", entry.file_name);
    for name in entry
        .licenses
        .iter()
        .map(|l| l.file_name)
        .chain([INSTALLED_JSON, staged.as_str()])
    {
        let _ = fs::remove_file(dir.join(name));
    }
    // What a write that stopped halfway leaves: `installed.tmp`, `LICENSE-….tmp`.
    if let Ok(items) = fs::read_dir(&dir) {
        for item in items.flatten() {
            if item.path().extension().is_some_and(|e| e == "tmp") {
                let _ = fs::remove_file(item.path());
            }
        }
    }
    let _ = fs::remove_dir_all(&partial_dir);
    let _ = fs::remove_dir(root.join(PARTIAL_DIR));
    if lock.is_some() {
        let _ = fs::remove_file(dir.join(LOCK_FILE));
    }
    // Windows keeps a deleted file that is still open in its folder until it is closed.
    drop(lock);
    let _ = fs::remove_dir(&dir);
    Ok(removed)
}

/// What `install --from` did.
#[derive(Debug, Default)]
pub struct ImportReport {
    /// Newly placed: id and final path.
    pub installed: Vec<(&'static str, PathBuf)>,
    /// Already verified in the target folder.
    pub already: Vec<&'static str>,
    /// Files whose hash matches no pin, or whose size matches no catalog file.
    pub unrecognized: Vec<PathBuf>,
    /// Catalog files the caller's filter refused (managed `allowed_models`): not copied.
    pub refused: Vec<(&'static str, PathBuf)>,
}

/// Files that travel with a release or an installed copy and are not models.
fn is_companion(name: &str) -> bool {
    name.starts_with('.')
        || name == INSTALLED_JSON
        || name == "provenance.json"
        || name == "SHA256SUMS"
        || name == "NOTICE.md"
        || (name.starts_with("LICENSE-") && name.ends_with(".txt"))
        || name.ends_with(".partial")
        || name.ends_with(".partial.json")
}

/// The candidate files under `input`: the file itself, or a folder's files and the files
/// of its direct subfolders (a copied models folder holds `<id>/<file>`).
fn import_candidates(input: &Path) -> io::Result<Vec<PathBuf>> {
    let meta = fs::metadata(input)?;
    if meta.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    let mut files = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(input)?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for item in entries {
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        if is_companion(&name) {
            continue;
        }
        if path.is_file() {
            files.push(path);
        } else if path.is_dir() {
            let mut inner: Vec<_> = fs::read_dir(&path)?.filter_map(Result::ok).collect();
            inner.sort_by_key(|e| e.file_name());
            for file in inner {
                let name = file.file_name().to_string_lossy().into_owned();
                if !is_companion(&name) && file.path().is_file() {
                    files.push(file.path());
                }
            }
        }
    }
    Ok(files)
}

/// Why `install --from` stopped.
#[derive(Debug)]
pub enum ImportError {
    /// A file under the input could not be read.
    Read { path: PathBuf, error: io::Error },
    /// Writing into the models folder failed: MSG-DISK when the volume is full, MSG-HASH
    /// when the copy came out other than the file that was identified.
    Install(InstallError),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Read { path, error } => {
                write!(f, "{} could not be read: {error}.", path.display())
            }
            ImportError::Install(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for ImportError {}

/// Deletes a staged copy when it is dropped: after a failure anywhere between its
/// creation and its rename into place (a full disk, a read error, a hash mismatch). After
/// the rename nothing is left at the path, and the delete finds nothing.
struct StagedFile(PathBuf);

impl Drop for StagedFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// `install --from <file|dir>` (DD-24): identifies every file by its SHA-256, whatever
/// its name, and copies each recognized one into `root` with its licenses and an
/// `installed.json` whose `source` is `import`. Only files whose size equals a catalog
/// size are hashed. A file `allow` refuses is listed in `refused` and not copied. Before
/// anything is copied, DD-22's disk rule must hold for every file that will be; the copy
/// is hashed again while it is written, and a copy that fails for any reason is deleted.
pub fn import_from(
    catalog: &Catalog,
    root: &Path,
    input: &Path,
    mode: WriteMode,
    allow: &dyn Fn(&SttCatalogEntry) -> bool,
    space: &SpaceProbe,
) -> Result<ImportReport, ImportError> {
    let read = |path: &Path| {
        let path = path.to_path_buf();
        move |error| ImportError::Read { path, error }
    };
    let mut report = ImportReport::default();
    let mut planned: Vec<(&'static SttCatalogEntry, PathBuf, String)> = Vec::new();
    for file in import_candidates(input).map_err(read(input))? {
        let size = fs::metadata(&file).map_err(read(&file))?.len();
        if catalog.sized(size).next().is_none() {
            report.unrecognized.push(file);
            continue;
        }
        let digest = sha256_file(&file).map_err(read(&file))?;
        let Some(entry) = catalog.by_sha256(&digest) else {
            report.unrecognized.push(file);
            continue;
        };
        if !allow(entry) {
            report.refused.push((entry.id, file));
            continue;
        }
        let there = verify_writable(root, entry);
        // The current revision, or this very file, is there already. An earlier
        // revision is replaced by a newer one (DD-23).
        let same = matches!(&there, Verdict::Verified { sha256, .. } if *sha256 == digest);
        if there.is_current() || same || planned.iter().any(|(e, _, _)| e.id == entry.id) {
            report.already.push(entry.id);
            continue;
        }
        planned.push((entry, file, digest));
    }
    if planned.is_empty() {
        return Ok(report);
    }
    let needed: u64 = planned.iter().map(|(entry, _, _)| entry.size_bytes).sum();
    let entries: Vec<&SttCatalogEntry> = planned.iter().map(|(entry, _, _)| *entry).collect();
    require_space(root, needed, &entries, space).map_err(ImportError::Install)?;
    for (entry, file, digest) in planned {
        let dir = root.join(entry.id);
        let write = |error| ImportError::Install(io_failure(error, root, entry.id, needed));
        create_dirs(root, mode).map_err(write)?;
        create_dirs(&dir, mode).map_err(write)?;
        let staged = StagedFile(dir.join(format!(".{}.import", entry.file_name)));
        let copied = copy_hashing(&file, &staged.0, root, entry.id, entry.size_bytes)?;
        if copied != digest {
            return Err(ImportError::Install(InstallError::Hash));
        }
        let target =
            publish(root, entry, &staged.0, &digest, Source::Import, mode).map_err(write)?;
        report.installed.push((entry.id, target));
    }
    Ok(report)
}

/// Copies `from` to a new file `to`, hashing what it reads. A read error names `from`; a
/// write error is MSG-DISK when the volume is full.
fn copy_hashing(
    from: &Path,
    to: &Path,
    root: &Path,
    id: &str,
    size: u64,
) -> Result<String, ImportError> {
    let read = |error| ImportError::Read {
        path: from.to_path_buf(),
        error,
    };
    let write = |error| ImportError::Install(io_failure(error, root, id, size));
    let mut input = File::open(from).map_err(read)?;
    let mut output = create_fresh(to).map_err(write)?;
    let mut hasher = Sha256::new();
    let mut block = vec![0u8; 1 << 20];
    loop {
        let n = input.read(&mut block).map_err(read)?;
        if n == 0 {
            break;
        }
        let chunk = &block[..n];
        hasher.update(chunk);
        output.write_all(chunk).map_err(write)?;
    }
    output.sync_all().map_err(write)?;
    Ok(hex(&hasher.finalize()))
}

/// Free bytes on the volume that holds `path`, or its nearest existing parent.
pub fn available_space(path: &Path) -> io::Result<u64> {
    let mut probe = path;
    loop {
        if probe.exists() {
            return fs2::available_space(probe);
        }
        match probe.parent() {
            Some(parent) => probe = parent,
            None => return fs2::available_space(path),
        }
    }
}

pub type SpaceProbe = Arc<dyn Fn(&Path) -> io::Result<u64> + Send + Sync>;

/// How a download reaches the network. [`Network::production`] is the only constructor
/// outside tests: https only, GitHub's asset hosts, the environment's proxies, and the
/// mirror from [`MIRROR_ENV`].
#[derive(Clone, Debug)]
pub struct Network {
    pub https_only: bool,
    pub final_hosts: Vec<String>,
    pub mirror: Option<String>,
    /// Honor `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` (reqwest's own reading).
    pub env_proxy: bool,
    pub max_redirects: usize,
    pub retry_budget: u32,
    /// The first backoff; it doubles per attempt without progress.
    pub backoff: Duration,
    pub max_retry_after: Duration,
    pub connect_timeout: Duration,
    /// The longest silence inside a body.
    pub read_timeout: Duration,
}

/// [`MIRROR_ENV`]'s value as the installer uses it: an https base without its trailing
/// slash, or nothing. Any other scheme is ignored, never followed (DD-25).
pub fn mirror_base(value: Option<&str>) -> Option<String> {
    value
        .map(|m| m.trim().trim_end_matches('/').to_string())
        .filter(|m| m.starts_with("https://") && m.len() > "https://".len())
}

impl Network {
    pub fn production() -> Network {
        let mirror = mirror_base(std::env::var(MIRROR_ENV).ok().as_deref());
        Network {
            https_only: true,
            final_hosts: GITHUB_ASSET_HOSTS.iter().map(|h| h.to_string()).collect(),
            mirror,
            env_proxy: true,
            max_redirects: MAX_REDIRECTS,
            retry_budget: RETRY_BUDGET,
            backoff: Duration::from_secs(1),
            max_retry_after: MAX_RETRY_AFTER,
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(60),
        }
    }

    fn client(&self) -> Result<reqwest::Client, InstallError> {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(self.connect_timeout)
            .read_timeout(self.read_timeout)
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .user_agent(concat!(
                "Biorouter/",
                env!("CARGO_PKG_VERSION"),
                " dictation"
            ));
        if !self.env_proxy {
            builder = builder.no_proxy();
        }
        builder
            .build()
            .map_err(|e| InstallError::Internal(format!("no HTTP client ({e})")))
    }
}

/// The phases a person sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Another process holds this model's lock.
    Waiting,
    Connecting,
    Downloading,
    Verifying,
    Installed,
}

/// The status `install_job`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub id: String,
    pub phase: Phase,
    pub downloaded: u64,
    pub total: u64,
}

/// Why an install stopped. `Display` is the message catalog's sentence where one exists.
#[derive(Debug)]
pub enum InstallError {
    /// MSG-OFFLINE: no host answered. The CLI and the routes fill in the folder.
    Offline,
    /// MSG-HASH.
    Hash,
    /// MSG-DISK.
    Disk {
        id: String,
        needed: u64,
        available: u64,
        volume: PathBuf,
    },
    /// An answer other than the ones the installer follows.
    Http {
        status: u16,
    },
    /// The https connection to `host` could not be verified: a proxy that inspects
    /// traffic, or a clock far off. Never retried.
    Tls {
        host: String,
    },
    /// A redirect that was not https, a seventh hop, or a body from another host.
    Redirect(String),
    /// A host that serves the bytes answered, but [`RETRY_BUDGET`] attempts in a row added
    /// nothing; the partial stays for the next install to resume.
    KeptFailing,
    /// `--system`: a part of the shared folder's path could be changed by an account other
    /// than root or an Administrator, so nothing was written there.
    UnsafeFolder(String),
    /// The person or the caller cancelled; the partial stays for a resume.
    Cancelled,
    Io(io::Error),
    Internal(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Offline => {
                f.write_str("This computer cannot reach Biorouter's model repository.")
            }
            InstallError::Hash => f.write_str(MSG_HASH),
            InstallError::Disk {
                id,
                needed,
                available,
                volume,
            } => write!(
                f,
                "Installing {id} needs {} GB free on {}; {} GB is available.",
                gb(*needed),
                volume.display(),
                gb_down(*available)
            ),
            InstallError::Http { status } => {
                write!(f, "The model repository answered HTTP {status}.")
            }
            InstallError::Tls { host } => write!(
                f,
                "The secure connection to {host} could not be verified, so nothing was downloaded."
            ),
            InstallError::Redirect(why) => write!(f, "The download was refused: {why}."),
            InstallError::KeptFailing => f.write_str(
                "The download kept failing, so it stopped. Run the install again to continue where it stopped.",
            ),
            InstallError::UnsafeFolder(why) => write!(
                f,
                "Nothing was installed in the shared model folder, because {why}. Make each folder on that path a real folder that only root or an Administrator can change, then try again."
            ),
            InstallError::Cancelled => f.write_str("The install was cancelled."),
            InstallError::Io(e) => write!(f, "The model could not be written: {e}."),
            InstallError::Internal(why) => write!(f, "The install failed: {why}."),
        }
    }
}

impl std::error::Error for InstallError {}

/// Bytes as GB with one decimal, rounded up so a need is never understated.
pub fn gb(bytes: u64) -> String {
    let tenths = bytes.div_ceil(100_000_000);
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// Bytes as GB with one decimal, rounded down so free space is never overstated.
pub fn gb_down(bytes: u64) -> String {
    let tenths = bytes / 100_000_000;
    format!("{}.{}", tenths / 10, tenths % 10)
}

fn io_failure(error: io::Error, volume: &Path, id: &str, needed: u64) -> InstallError {
    if error.kind() == io::ErrorKind::StorageFull {
        return InstallError::Disk {
            id: id.to_string(),
            needed,
            available: available_space(volume).unwrap_or(0),
            volume: volume.to_path_buf(),
        };
    }
    InstallError::Io(error)
}

/// Options for one install.
#[derive(Clone)]
pub struct InstallOptions {
    pub network: Network,
    pub mode: WriteMode,
    pub space: SpaceProbe,
    pub progress: Option<watch::Sender<Progress>>,
    pub cancel: Arc<AtomicBool>,
}

impl InstallOptions {
    pub fn new(network: Network, mode: WriteMode) -> InstallOptions {
        InstallOptions {
            network,
            mode,
            space: Arc::new(|path: &Path| available_space(path)),
            progress: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn report(&self, id: &str, phase: Phase, downloaded: u64, total: u64) {
        if let Some(tx) = &self.progress {
            let _ = tx.send(Progress {
                id: id.to_string(),
                phase,
                downloaded,
                total,
            });
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// What one file's install did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileOutcome {
    pub id: &'static str,
    pub path: PathBuf,
    /// False when a verified copy was already there, for example after waiting on
    /// another process's lock.
    pub downloaded: bool,
    pub source: Source,
}

/// The bytes a download into `root` still needs for `files`: each size minus its
/// partial, nothing for a copy of the current revision. An earlier revision counts as
/// missing, because an install fetches the current one (DD-23).
pub fn remaining_bytes(root: &Path, files: &[&SttCatalogEntry]) -> u64 {
    files
        .iter()
        .filter(|entry| !verify_writable(root, entry).is_current())
        .map(|entry| {
            let (_, partial, meta) = partial_paths(root, entry);
            let have = match read_partial_meta(&meta) {
                Some(m) if m.sha256 == entry.sha256 => {
                    fs::metadata(&partial).map_or(0, |m| m.len())
                }
                _ => 0,
            };
            entry.size_bytes.saturating_sub(have.min(entry.size_bytes))
        })
        .sum()
}

/// DD-22's disk rule: available ≥ remaining + 256 MiB.
pub fn check_space(
    root: &Path,
    files: &[&SttCatalogEntry],
    probe: &SpaceProbe,
) -> Result<(), InstallError> {
    require_space(root, remaining_bytes(root, files), files, probe)
}

/// MSG-DISK unless the volume of `root` holds `remaining` bytes plus the margin.
fn require_space(
    root: &Path,
    remaining: u64,
    files: &[&SttCatalogEntry],
    probe: &SpaceProbe,
) -> Result<(), InstallError> {
    if remaining == 0 {
        return Ok(());
    }
    let volume = nearest_existing(root);
    let available = probe(&volume).map_err(InstallError::Io)?;
    let needed = remaining + DISK_MARGIN_BYTES;
    if available < needed {
        let id = files
            .iter()
            .rev()
            .find(|e| e.is_speech())
            .or(files.last())
            .map_or(String::new(), |e| e.id.to_string());
        return Err(InstallError::Disk {
            id,
            needed,
            available,
            volume,
        });
    }
    Ok(())
}

fn nearest_existing(path: &Path) -> PathBuf {
    let mut probe = path;
    loop {
        if probe.exists() {
            return probe.to_path_buf();
        }
        match probe.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => probe = parent,
            _ => return path.to_path_buf(),
        }
    }
}

/// Runs filesystem work (hashing a copy, the disk check, a publish with its fsyncs) on a
/// blocking thread: on the daemon's runtime the same threads serve dictation streams,
/// and a 574 MB hash would hold one for over a second.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, InstallError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| InstallError::Internal(e.to_string()))
}

/// Installs every file in `files` into `root`, in order, after the disk check. A file
/// whose current revision is already there is skipped.
pub async fn install_files(
    root: &Path,
    files: &[&'static SttCatalogEntry],
    opts: &InstallOptions,
) -> Result<Vec<FileOutcome>, InstallError> {
    let (dir, list, probe) = (root.to_path_buf(), files.to_vec(), Arc::clone(&opts.space));
    blocking(move || check_space(&dir, &list, &probe)).await??;
    let mut outcomes = Vec::new();
    for entry in files {
        outcomes.push(install_one(root, entry, opts).await?);
    }
    Ok(outcomes)
}

/// One file: its lock, then the download unless the current revision is there by then.
pub async fn install_one(
    root: &Path,
    entry: &'static SttCatalogEntry,
    opts: &InstallOptions,
) -> Result<FileOutcome, InstallError> {
    let dir = root.join(entry.id);
    create_dirs(root, opts.mode).map_err(InstallError::Io)?;
    create_dirs(&dir, opts.mode).map_err(InstallError::Io)?;
    let lock = lock_for(&dir, entry, opts).await?;
    let result = install_locked(root, entry, opts).await;
    drop(lock);
    result
}

async fn lock_for(
    dir: &Path,
    entry: &SttCatalogEntry,
    opts: &InstallOptions,
) -> Result<File, InstallError> {
    use fs2::FileExt;
    let file = open_no_follow(&dir.join(LOCK_FILE)).map_err(InstallError::Io)?;
    match file.try_lock_exclusive() {
        Ok(()) => return Ok(file),
        Err(e) if e.kind() == fs2::lock_contended_error().kind() => {}
        Err(e) => return Err(InstallError::Io(e)),
    }
    opts.report(entry.id, Phase::Waiting, 0, entry.size_bytes);
    tokio::task::spawn_blocking(move || file.lock_exclusive().map(|()| file))
        .await
        .map_err(|e| InstallError::Internal(e.to_string()))?
        .map_err(InstallError::Io)
}

async fn install_locked(
    root: &Path,
    entry: &'static SttCatalogEntry,
    opts: &InstallOptions,
) -> Result<FileOutcome, InstallError> {
    // An earlier revision stays where it is, loadable, until the current one is renamed
    // over it (DD-23).
    let folder = root.to_path_buf();
    let there = blocking(move || verify_writable(&folder, entry)).await?;
    if let Verdict::Verified {
        path,
        superseded: false,
        ..
    } = there
    {
        let source = read_record(root, entry).map_or(Source::Rescan, |r| r.source);
        opts.report(
            entry.id,
            Phase::Installed,
            entry.size_bytes,
            entry.size_bytes,
        );
        return Ok(FileOutcome {
            id: entry.id,
            path,
            downloaded: false,
            source,
        });
    }
    let (partial_dir, partial, meta_path) = partial_paths(root, entry);
    create_dirs(&partial_dir, opts.mode).map_err(InstallError::Io)?;
    let source = download(entry, &partial, &meta_path, root, opts).await?;
    opts.report(
        entry.id,
        Phase::Verifying,
        entry.size_bytes,
        entry.size_bytes,
    );
    let (partial_for_hash, entry_size) = (partial.clone(), entry.size_bytes);
    let digest = blocking(move || -> io::Result<Option<String>> {
        let len = fs::metadata(&partial_for_hash)?.len();
        if len != entry_size {
            return Ok(None);
        }
        sha256_file(&partial_for_hash).map(Some)
    })
    .await?
    .map_err(InstallError::Io)?;
    let digest = match digest {
        // A download is always of the current revision.
        Some(digest) if digest == entry.sha256 => digest,
        _ => {
            discard_partial(&partial, &meta_path);
            return Err(InstallError::Hash);
        }
    };
    let (folder, staged, mode) = (root.to_path_buf(), partial.clone(), opts.mode);
    let path = blocking(move || publish(&folder, entry, &staged, &digest, source, mode))
        .await?
        .map_err(|e| io_failure(e, root, entry.id, entry.size_bytes))?;
    let _ = fs::remove_file(&meta_path);
    let _ = fs::remove_dir(&partial_dir);
    let _ = fs::remove_dir(root.join(PARTIAL_DIR));
    opts.report(
        entry.id,
        Phase::Installed,
        entry.size_bytes,
        entry.size_bytes,
    );
    Ok(FileOutcome {
        id: entry.id,
        path,
        downloaded: true,
        source,
    })
}

/// `<file>.partial.json`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct PartialMeta {
    url: String,
    etag: Option<String>,
    size: u64,
    sha256: String,
}

fn read_partial_meta(path: &Path) -> Option<PartialMeta> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn write_partial_meta(path: &Path, meta: &PartialMeta) -> io::Result<()> {
    let json = serde_json::to_vec(meta).map_err(io::Error::other)?;
    write_atomically(path, &json, WriteMode::User)
}

fn discard_partial(partial: &Path, meta: &Path) {
    let _ = fs::remove_file(partial);
    let _ = fs::remove_file(meta);
}

/// How one attempt ended.
enum Attempt {
    /// The partial now holds every byte.
    Complete(Source),
    /// Try again. `progress`: the partial holds more bytes than it did before the
    /// attempt, which resets the count of misses. `after`: a server's wait. `reached`: a host
    /// that serves the bytes (an asset host or the mirror) answered; github.com's
    /// redirect alone does not count, so a network that blocks the asset hosts ends in
    /// MSG-OFFLINE with its air gapped route.
    Retry {
        progress: bool,
        after: Option<Duration>,
        reached: bool,
    },
    Fatal(InstallError),
}

/// The download loop: attempts until the partial is complete, the retry budget is spent
/// on attempts without progress, or an answer stops it.
async fn download(
    entry: &SttCatalogEntry,
    partial: &Path,
    meta_path: &Path,
    root: &Path,
    opts: &InstallOptions,
) -> Result<Source, InstallError> {
    let client = opts.network.client()?;
    if read_partial_meta(meta_path).is_none_or(|m| m.sha256 != entry.sha256) {
        discard_partial(partial, meta_path);
    }
    let mut misses = 0u32;
    let mut reached_any = false;
    loop {
        if opts.cancelled() {
            return Err(InstallError::Cancelled);
        }
        let mut attempt = Attempt::Retry {
            progress: false,
            after: None,
            reached: false,
        };
        let mut candidates: Vec<(String, Source)> = Vec::new();
        if let Some(mirror) = &opts.network.mirror {
            candidates.push((
                format!("{mirror}/{}/{}", entry.release_tag, entry.file_name),
                Source::Mirror,
            ));
        }
        candidates.push((entry.primary_url().to_string(), Source::Download));
        for (url, source) in candidates {
            attempt =
                one_attempt(&client, entry, &url, source, partial, meta_path, root, opts).await;
            match &attempt {
                Attempt::Complete(_) => break,
                Attempt::Retry { progress: true, .. } => break,
                // A mirror that failed falls back to GitHub in the same attempt.
                _ if source == Source::Mirror => continue,
                _ => break,
            }
        }
        match attempt {
            Attempt::Complete(source) => return Ok(source),
            Attempt::Fatal(e) => return Err(e),
            Attempt::Retry {
                progress,
                after,
                reached,
            } => {
                reached_any |= reached;
                if progress {
                    misses = 0;
                    continue;
                }
                misses += 1;
                if misses >= opts.network.retry_budget {
                    return Err(if reached_any {
                        InstallError::KeptFailing
                    } else {
                        InstallError::Offline
                    });
                }
                let backoff = opts
                    .network
                    .backoff
                    .saturating_mul(1u32 << (misses - 1).min(6));
                let wait = after
                    .map(|a| a.min(opts.network.max_retry_after))
                    .unwrap_or(backoff);
                tokio::time::sleep(wait).await;
            }
        }
    }
}

/// What a request that got no answer ran into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SendFailure {
    /// TLS could not be set up or verified: a certificate, an alert from the server, or
    /// bytes that are not TLS at all (a plain HTTP server, a proxy that rewrites the
    /// connection). DD-21: it stops the install at once, since retrying cannot fix it.
    Tls,
    /// Anything else, retried: no route, a refused, reset or aborted connection, a
    /// handshake the network cut short (tokio-rustls's `UnexpectedEof`), a timeout.
    Network,
}

/// Classifies by type, never by the message's words: rustls's own `Error` anywhere in
/// the chain is a TLS failure. tokio-rustls hands it over inside an `io::Error`, whose
/// `source()` skips the error it wraps, so each `io::Error` is opened with `get_ref`.
fn send_failure(error: &(dyn std::error::Error + 'static)) -> SendFailure {
    let mut next = Some(error);
    while let Some(current) = next {
        if current.downcast_ref::<rustls::Error>().is_some() {
            return SendFailure::Tls;
        }
        if let Some(inner) = current
            .downcast_ref::<io::Error>()
            .and_then(io::Error::get_ref)
        {
            if send_failure(inner) == SendFailure::Tls {
                return SendFailure::Tls;
            }
        }
        next = current.source();
    }
    SendFailure::Network
}

fn host_of(url: &reqwest::Url) -> String {
    url.host_str().unwrap_or_default().to_ascii_lowercase()
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?;
    value
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// `bytes <start>-<end>/<total>`: the start.
fn content_range_start(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    let rest = value.trim().strip_prefix("bytes ")?;
    rest.split('-').next()?.trim().parse().ok()
}

fn partial_len(partial: &Path) -> u64 {
    fs::metadata(partial).map_or(0, |m| m.len())
}

#[allow(clippy::too_many_arguments)]
async fn one_attempt(
    client: &reqwest::Client,
    entry: &SttCatalogEntry,
    start: &str,
    source: Source,
    partial: &Path,
    meta_path: &Path,
    root: &Path,
    opts: &InstallOptions,
) -> Attempt {
    let size = entry.size_bytes;
    let mut have = partial_len(partial);
    if have > size {
        discard_partial(partial, meta_path);
        have = 0;
    }
    // Progress is measured against what the partial held before this attempt: a server
    // that answers a range with 200 restarts the file, and a restart that ends short of
    // the old partial is a miss, so one that always cuts the body cannot loop forever.
    let had = have;
    let meta = read_partial_meta(meta_path).filter(|m| m.sha256 == entry.sha256);
    let etag = meta.as_ref().and_then(|m| m.etag.clone());
    let Ok(mut url) = reqwest::Url::parse(start) else {
        return Attempt::Fatal(InstallError::Internal(
            "a catalog URL does not parse".into(),
        ));
    };
    let mut final_hosts = opts.network.final_hosts.clone();
    if source == Source::Mirror {
        final_hosts.push(host_of(&url));
    }
    opts.report(entry.id, Phase::Connecting, have, size);
    let retry = |reached| Attempt::Retry {
        progress: false,
        after: None,
        reached,
    };
    let mut hop = 0usize;
    let response = loop {
        if opts.network.https_only && url.scheme() != "https" {
            return Attempt::Fatal(InstallError::Redirect(format!(
                "{} is not an https address",
                host_of(&url)
            )));
        }
        let mut request = client.get(url.clone());
        if have > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
            if let Some(etag) = &etag {
                request = request.header(reqwest::header::IF_RANGE, etag);
            }
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) => {
                return match send_failure(&error) {
                    SendFailure::Tls => Attempt::Fatal(InstallError::Tls {
                        host: host_of(&url),
                    }),
                    // No host that serves the bytes answered, whether or not github.com
                    // redirected first.
                    SendFailure::Network => retry(false),
                };
            }
        };
        let status = response.status();
        if status.is_redirection() {
            if hop >= opts.network.max_redirects {
                return Attempt::Fatal(InstallError::Redirect(format!(
                    "more than {} redirects",
                    opts.network.max_redirects
                )));
            }
            let Some(next) = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .and_then(|l| url.join(l).ok())
            else {
                return Attempt::Fatal(InstallError::Redirect(
                    "a redirect without a location".into(),
                ));
            };
            url = next;
            hop += 1;
            continue;
        }
        break response;
    };
    let status = response.status().as_u16();
    let came_through_redirect = hop > 0;
    let host = host_of(response.url());
    let reached = final_hosts.contains(&host);
    match status {
        200 | 206 | 416 => {}
        // A signed URL that expired: request the github.com URL again.
        403 | 404 | 410 if came_through_redirect => return retry(reached),
        408 | 425 | 429 | 500..=599 | 403 => {
            return Attempt::Retry {
                progress: false,
                after: retry_after(response.headers()),
                reached,
            }
        }
        other => return Attempt::Fatal(InstallError::Http { status: other }),
    }
    if !reached {
        return Attempt::Fatal(InstallError::Redirect(format!(
            "{host} is not one of GitHub's asset hosts"
        )));
    }
    if status == 416 {
        // The server has nothing past our partial: it is whole, or it is wrong.
        let whole = have == size && {
            let path = partial.to_path_buf();
            let digest = blocking(move || sha256_file(&path)).await;
            matches!(digest, Ok(Ok(digest)) if digest == entry.sha256)
        };
        if whole {
            return Attempt::Complete(source);
        }
        discard_partial(partial, meta_path);
        return retry(true);
    }
    let resume_from = if status == 206 {
        match content_range_start(response.headers()) {
            Some(start) if start == have => have,
            _ => {
                // A range we did not ask for: start the file again.
                discard_partial(partial, meta_path);
                return retry(true);
            }
        }
    } else {
        0
    };
    let new_etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|e| e.to_str().ok())
        .map(str::to_string);
    let record = PartialMeta {
        url: entry.primary_url().to_string(),
        etag: new_etag.or(etag),
        size,
        sha256: entry.sha256.to_string(),
    };
    if let Err(e) = write_partial_meta(meta_path, &record) {
        return Attempt::Fatal(io_failure(e, root, entry.id, size - resume_from));
    }
    let mut file = match open_no_follow(partial) {
        Ok(file) => file,
        Err(e) => return Attempt::Fatal(io_failure(e, root, entry.id, size - resume_from)),
    };
    if let Err(e) = file
        .set_len(resume_from)
        .and_then(|()| file.seek(SeekFrom::Start(resume_from)).map(|_| ()))
    {
        return Attempt::Fatal(io_failure(e, root, entry.id, size - resume_from));
    }
    let mut written = resume_from;
    let mut response = response;
    opts.report(entry.id, Phase::Downloading, written, size);
    loop {
        if opts.cancelled() {
            let _ = file.sync_all();
            return Attempt::Fatal(InstallError::Cancelled);
        }
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if written + chunk.len() as u64 > size {
                    discard_partial(partial, meta_path);
                    return Attempt::Fatal(InstallError::Hash);
                }
                if let Err(e) = file.write_all(&chunk) {
                    return Attempt::Fatal(io_failure(e, root, entry.id, size - written));
                }
                written += chunk.len() as u64;
                opts.report(entry.id, Phase::Downloading, written, size);
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    let _ = file.sync_all();
    if written == size {
        return Attempt::Complete(source);
    }
    Attempt::Retry {
        progress: written > had,
        after: None,
        reached: true,
    }
}

/// Whether this process may write the computer's shared folder: root on Unix, an
/// Administrator whose administrative rights are in force (`TokenElevation`) on Windows
/// (`--system`).
pub fn running_as_administrator() -> bool {
    #[cfg(unix)]
    {
        // SAFETY: geteuid has no preconditions and cannot fail.
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo handle of the current process is always valid.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
        // SAFETY: `elevation` is the TOKEN_ELEVATION the class asks for, `size` its size.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                size,
                &mut size,
            )
        };
        // SAFETY: `token` was opened above.
        unsafe { CloseHandle(token) };
        ok != 0 && elevation.TokenIsElevated != 0
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// Every file under `dir`, recursively, that starts with the ggml magic: the R6 guard
/// for any folder that ships (an app's `src/bin`, a package's tree).
pub fn ggml_files_under(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for item in fs::read_dir(&next)? {
            let item = item?;
            let kind = item.file_type()?;
            if kind.is_dir() {
                stack.push(item.path());
            } else if kind.is_file() {
                let mut head = [0u8; 4];
                let n = File::open(item.path()).and_then(|mut f| f.read(&mut head))?;
                if has_ggml_magic(&head[..n]) {
                    found.push(item.path());
                }
            }
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    //! The installer against a loopback HTTP server (`testing-and-release.md`, "Models
    //! and installer"). The server speaks just enough HTTP/1.1: one request per
    //! connection, `Connection: close`, and a handler that sees every request in order.

    use super::*;
    use crate::catalog::{
        Languages, RetiredSttModel, SttEngine, SttFormat, SttKind, Tokenizer,
        LICENSE_OPENAI_WHISPER, LICENSE_SILERO_VAD,
    };
    use crate::cpu::HardwareClass;
    use std::io::BufRead;
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::time::Instant;

    #[derive(Clone, Debug)]
    struct Seen {
        path: String,
        headers: Vec<(String, String)>,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        }
    }

    #[derive(Default)]
    struct Reply {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        /// Write this many body bytes, then hold the connection open.
        stall_after: Option<usize>,
        /// Write this many body bytes, then close the connection.
        cut_after: Option<usize>,
        /// Wait before answering.
        delay: Option<Duration>,
    }

    impl Reply {
        fn status(status: u16) -> Reply {
            Reply {
                status,
                ..Reply::default()
            }
        }

        fn redirect(location: String) -> Reply {
            Reply {
                status: 302,
                headers: vec![("Location".into(), location)],
                ..Reply::default()
            }
        }

        fn header(mut self, name: &str, value: &str) -> Reply {
            self.headers.push((name.into(), value.into()));
            self
        }
    }

    type Handler = dyn Fn(&Seen, usize) -> Reply + Send + Sync;

    struct Server {
        addr: SocketAddr,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl Server {
        fn start(handler: impl Fn(&Seen, usize) -> Reply + Send + Sync + 'static) -> Server {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
            let handler: Arc<Handler> = Arc::new(handler);
            let log = Arc::clone(&seen);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (log, handler) = (Arc::clone(&log), Arc::clone(&handler));
                    std::thread::spawn(move || serve(stream, &log, &*handler));
                }
            });
            Server { addr, seen }
        }

        fn url(&self, path: &str) -> String {
            format!("http://127.0.0.1:{}{path}", self.addr.port())
        }

        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }

        fn paths(&self) -> Vec<String> {
            self.seen().into_iter().map(|s| s.path).collect()
        }
    }

    fn serve(stream: TcpStream, log: &Mutex<Vec<Seen>>, handler: &Handler) {
        let mut reader = io::BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let path = line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string();
        let mut headers = Vec::new();
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = header.split_once(':') {
                headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
            }
        }
        let seen = Seen { path, headers };
        let index = {
            let mut log = log.lock().unwrap();
            log.push(seen.clone());
            log.len() - 1
        };
        let reply = handler(&seen, index);
        if let Some(delay) = reply.delay {
            std::thread::sleep(delay);
        }
        let mut out = stream;
        let mut head = format!(
            "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
            reply.status,
            reply.body.len()
        );
        for (k, v) in &reply.headers {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        if out.write_all(head.as_bytes()).is_err() {
            return;
        }
        let limit = reply
            .stall_after
            .or(reply.cut_after)
            .unwrap_or(reply.body.len())
            .min(reply.body.len());
        let _ = out.write_all(&reply.body[..limit]);
        let _ = out.flush();
        if reply.stall_after.is_some() {
            std::thread::sleep(Duration::from_secs(120));
        }
    }

    /// Answers `seen` from `body` the way GitHub's asset host does: 200, or 206 for a
    /// `Range` whose `If-Range` matches `etag`, or 416 past the end.
    fn asset(seen: &Seen, body: &[u8], etag: &str) -> Reply {
        let range = seen
            .header("range")
            .and_then(|r| r.strip_prefix("bytes="))
            .and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
        let if_range_ok = seen.header("if-range").is_none_or(|v| v == etag);
        match range {
            Some(start) if if_range_ok && start >= body.len() => {
                Reply::status(416).header("Content-Range", &format!("bytes */{}", body.len()))
            }
            Some(start) if if_range_ok => Reply {
                status: 206,
                body: body[start..].to_vec(),
                ..Reply::default()
            }
            .header(
                "Content-Range",
                &format!("bytes {start}-{}/{}", body.len() - 1, body.len()),
            )
            .header("ETag", etag),
            _ => Reply {
                status: 200,
                body: body.to_vec(),
                ..Reply::default()
            }
            .header("ETag", etag),
        }
    }

    /// Pseudo random bytes that start like a ggml model.
    fn model_bytes(len: usize, seed: u32) -> Vec<u8> {
        let mut state = seed.wrapping_mul(2_654_435_761).max(1);
        let mut bytes: Vec<u8> = (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state & 0xff) as u8
            })
            .collect();
        bytes[..4].copy_from_slice(&crate::catalog::GGML_MAGIC);
        bytes
    }

    fn digest_of(bytes: &[u8]) -> String {
        hex(&Sha256::digest(bytes))
    }

    fn leak(s: String) -> &'static str {
        Box::leak(s.into_boxed_str())
    }

    fn entry_for(id: &str, bytes: &[u8], url: String) -> &'static SttCatalogEntry {
        let sha: &'static str = leak(digest_of(bytes));
        Box::leak(Box::new(SttCatalogEntry {
            id: leak(id.to_string()),
            display_name: "test model",
            kind: SttKind::Speech,
            engine: SttEngine::WhisperCpp,
            format: SttFormat::GgmlBin,
            languages: Languages::EnglishOnly,
            tokenizer: Tokenizer::Gpt2,
            file_name: leak(format!("ggml-{id}.bin")),
            size_bytes: bytes.len() as u64,
            sha256: sha,
            urls: Box::leak(vec![leak(url)].into_boxed_slice()),
            release_tag: leak(format!("stt-{id}-r1")),
            licenses: &[LICENSE_OPENAI_WHISPER],
            requires: &[],
            classes: &[HardwareClass::As, HardwareClass::Cpu],
            default_for: &[],
            min_total_ram_gib: 0,
            accepted_sha256: Box::leak(vec![sha].into_boxed_slice()),
        }))
    }

    fn network() -> Network {
        Network {
            https_only: false,
            final_hosts: vec!["127.0.0.1".into()],
            mirror: None,
            env_proxy: false,
            max_redirects: MAX_REDIRECTS,
            retry_budget: RETRY_BUDGET,
            backoff: Duration::from_millis(1),
            max_retry_after: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(2),
            read_timeout: Duration::from_secs(2),
        }
    }

    fn options() -> InstallOptions {
        InstallOptions::new(network(), WriteMode::User)
    }

    const SIZE: usize = 300_000;

    /// A GitHub shaped pair of routes: `/gh/<file>` redirects to `/asset/<file>`.
    fn github(body: Vec<u8>) -> Server {
        Server::start(move |seen, _| {
            if seen.path.starts_with("/gh/") {
                return Reply::redirect(seen.path.replacen("/gh/", "/asset/", 1));
            }
            asset(seen, &body, "\"E1\"")
        })
    }

    fn assert_installed(root: &Path, entry: &SttCatalogEntry, bytes: &[u8]) {
        let path = model_path(root, entry);
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(verify_writable(root, entry).is_verified());
        for license in entry.licenses {
            assert!(root.join(entry.id).join(license.file_name).is_file());
        }
        let (partial_dir, _, _) = partial_paths(root, entry);
        assert!(!partial_dir.exists(), "the partial folder is left behind");
    }

    #[tokio::test]
    async fn a_redirected_200_installs_with_licenses_and_a_record() {
        let bytes = model_bytes(SIZE, 1);
        let server = github(bytes.clone());
        let entry = entry_for("m200", &bytes, server.url("/gh/ggml-m200.bin"));
        let root = tempfile::tempdir().unwrap();
        let (tx, rx) = watch::channel(Progress {
            id: String::new(),
            phase: Phase::Connecting,
            downloaded: 0,
            total: 0,
        });
        let mut opts = options();
        opts.progress = Some(tx);
        let outcome = install_one(root.path(), entry, &opts).await.unwrap();
        assert!(outcome.downloaded);
        assert_eq!(outcome.source, Source::Download);
        assert_installed(root.path(), entry, &bytes);
        let record = read_record(root.path(), entry).unwrap();
        assert_eq!(record.source, Source::Download);
        assert_eq!(record.size, SIZE as u64);
        assert_eq!(record.sha256, entry.sha256);
        assert_eq!(rx.borrow().phase, Phase::Installed);
        assert_eq!(
            server.paths(),
            ["/gh/ggml-m200.bin", "/asset/ggml-m200.bin"]
        );
        for seen in server.seen() {
            assert!(seen.header("authorization").is_none(), "{seen:?}");
            assert!(seen.header("range").is_none(), "{seen:?}");
        }
        // Installing again finds the verified copy and downloads nothing.
        let again = install_one(root.path(), entry, &opts).await.unwrap();
        assert!(!again.downloaded);
        assert_eq!(server.seen().len(), 2);
    }

    fn plant_partial(root: &Path, entry: &SttCatalogEntry, bytes: &[u8], etag: Option<&str>) {
        let (dir, partial, meta) = partial_paths(root, entry);
        fs::create_dir_all(dir).unwrap();
        fs::write(&partial, bytes).unwrap();
        write_partial_meta(
            &meta,
            &PartialMeta {
                url: entry.primary_url().into(),
                etag: etag.map(str::to_string),
                size: entry.size_bytes,
                sha256: entry.sha256.into(),
            },
        )
        .unwrap();
    }

    #[tokio::test]
    async fn a_partial_resumes_with_range_and_if_range() {
        let bytes = model_bytes(SIZE, 2);
        let server = github(bytes.clone());
        let entry = entry_for("m206", &bytes, server.url("/gh/ggml-m206.bin"));
        let root = tempfile::tempdir().unwrap();
        plant_partial(root.path(), entry, &bytes[..SIZE / 3], Some("\"E1\""));
        install_one(root.path(), entry, &options()).await.unwrap();
        assert_installed(root.path(), entry, &bytes);
        let seen = server.seen();
        let asset_request = seen.iter().find(|s| s.path.starts_with("/asset/")).unwrap();
        assert_eq!(
            asset_request.header("range"),
            Some(format!("bytes={}-", SIZE / 3).as_str())
        );
        assert_eq!(asset_request.header("if-range"), Some("\"E1\""));
        // Range goes to github.com too: the redirect must carry it to the asset host.
        assert!(seen[0].header("range").is_some());
    }

    #[tokio::test]
    async fn a_changed_etag_gets_the_whole_file_again() {
        let bytes = model_bytes(SIZE, 3);
        let server = github(bytes.clone());
        let entry = entry_for("metag", &bytes, server.url("/gh/ggml-metag.bin"));
        let root = tempfile::tempdir().unwrap();
        // The server ignores the range because the entity changed, and answers 200.
        plant_partial(
            root.path(),
            entry,
            &model_bytes(SIZE / 2, 99),
            Some("\"OLD\""),
        );
        install_one(root.path(), entry, &options()).await.unwrap();
        assert_installed(root.path(), entry, &bytes);
    }

    #[tokio::test]
    async fn a_wrong_content_range_starts_the_file_again() {
        let bytes = model_bytes(SIZE, 4);
        let body = bytes.clone();
        let server = Server::start(move |seen, _| {
            if seen.header("range").is_some() {
                // A range that starts at 0 while the partial holds a third.
                return Reply {
                    status: 206,
                    body: body.clone(),
                    ..Reply::default()
                }
                .header(
                    "Content-Range",
                    &format!("bytes 0-{}/{}", body.len() - 1, body.len()),
                );
            }
            asset(seen, &body, "\"E1\"")
        });
        let entry = entry_for("mcr", &bytes, server.url("/asset/ggml-mcr.bin"));
        let root = tempfile::tempdir().unwrap();
        plant_partial(root.path(), entry, &bytes[..SIZE / 3], Some("\"E1\""));
        install_one(root.path(), entry, &options()).await.unwrap();
        assert_installed(root.path(), entry, &bytes);
        let seen = server.seen();
        assert_eq!(seen.len(), 2);
        assert!(
            seen[1].header("range").is_none(),
            "the second attempt starts from 0"
        );
    }

    #[tokio::test]
    async fn a_416_installs_a_whole_partial_and_restarts_a_bad_one() {
        let bytes = model_bytes(SIZE, 5);
        let server = github(bytes.clone());
        let good = entry_for("m416a", &bytes, server.url("/gh/ggml-m416a.bin"));
        let root = tempfile::tempdir().unwrap();
        plant_partial(root.path(), good, &bytes, Some("\"E1\""));
        let outcome = install_one(root.path(), good, &options()).await.unwrap();
        assert!(outcome.downloaded);
        assert_installed(root.path(), good, &bytes);
        // The whole partial was installed from the 416 alone: one request to each host,
        // both for the bytes past the partial, and no second body.
        let good_requests: Vec<Seen> = server
            .seen()
            .into_iter()
            .filter(|s| s.path.ends_with("/ggml-m416a.bin"))
            .collect();
        let paths: Vec<&str> = good_requests.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["/gh/ggml-m416a.bin", "/asset/ggml-m416a.bin"]);
        let whole_range = format!("bytes={SIZE}-");
        for seen in &good_requests {
            assert_eq!(seen.header("range"), Some(whole_range.as_str()), "{seen:?}");
        }

        let bad = entry_for("m416b", &bytes, server.url("/gh/ggml-m416b.bin"));
        let mut wrong = bytes.clone();
        wrong[SIZE / 2] ^= 0xff;
        plant_partial(root.path(), bad, &wrong, Some("\"E1\""));
        install_one(root.path(), bad, &options()).await.unwrap();
        assert_installed(root.path(), bad, &bytes);
        let asset_requests: Vec<Seen> = server
            .seen()
            .into_iter()
            .filter(|s| s.path == "/asset/ggml-m416b.bin")
            .collect();
        assert_eq!(asset_requests.len(), 2);
        assert!(asset_requests[1].header("range").is_none());
    }

    #[tokio::test]
    async fn an_expired_redirect_requests_the_github_url_again() {
        let bytes = model_bytes(SIZE, 6);
        let body = bytes.clone();
        let server = Server::start(move |seen, index| {
            if seen.path.starts_with("/gh/") {
                return Reply::redirect(format!("/signed/{index}"));
            }
            if seen.path == "/signed/0" {
                return Reply::status(403);
            }
            asset(seen, &body, "\"E1\"")
        });
        let entry = entry_for("mexp", &bytes, server.url("/gh/ggml-mexp.bin"));
        let root = tempfile::tempdir().unwrap();
        install_one(root.path(), entry, &options()).await.unwrap();
        assert_installed(root.path(), entry, &bytes);
        assert_eq!(
            server.paths(),
            [
                "/gh/ggml-mexp.bin",
                "/signed/0",
                "/gh/ggml-mexp.bin",
                "/signed/2"
            ]
        );
    }

    #[tokio::test]
    async fn retry_after_is_honoured_on_429() {
        let bytes = model_bytes(SIZE, 7);
        let body = bytes.clone();
        let server = Server::start(move |seen, index| {
            if index == 0 {
                return Reply::status(429).header("Retry-After", "1");
            }
            asset(seen, &body, "\"E1\"")
        });
        let entry = entry_for("m429", &bytes, server.url("/asset/ggml-m429.bin"));
        let root = tempfile::tempdir().unwrap();
        let started = Instant::now();
        install_one(root.path(), entry, &options()).await.unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(950),
            "{:?}",
            started.elapsed()
        );
        assert_installed(root.path(), entry, &bytes);
    }

    #[tokio::test]
    async fn a_sixth_redirect_is_refused_and_five_are_followed() {
        let bytes = model_bytes(SIZE, 8);
        let body = bytes.clone();
        let server = Server::start(move |seen, _| {
            // /hop/<n> redirects to /hop/<n-1>; /hop/0 is the asset.
            let n: usize = seen.path.trim_start_matches("/hop/").parse().unwrap_or(0);
            if n > 0 {
                return Reply::redirect(format!("/hop/{}", n - 1));
            }
            asset(seen, &body, "\"E1\"")
        });
        let root = tempfile::tempdir().unwrap();
        let five = entry_for("mhop5", &bytes, server.url("/hop/5"));
        install_one(root.path(), five, &options()).await.unwrap();
        let six = entry_for("mhop6", &bytes, server.url("/hop/6"));
        let error = install_one(root.path(), six, &options()).await.unwrap_err();
        assert!(matches!(error, InstallError::Redirect(_)), "{error}");
        assert!(!model_path(root.path(), six).exists());
    }

    #[tokio::test]
    async fn a_body_from_another_host_or_over_http_is_refused() {
        let bytes = model_bytes(SIZE, 9);
        let body = bytes.clone();
        let server = Server::start(move |seen, _| {
            if seen.path.starts_with("/gh/") {
                // `localhost` is not on the list of final hosts; 127.0.0.1 is.
                let port = seen
                    .header("host")
                    .and_then(|h| h.rsplit(':').next())
                    .unwrap_or_default()
                    .to_string();
                return Reply::redirect(format!("http://localhost:{port}/asset/x"));
            }
            asset(seen, &body, "\"E1\"")
        });
        let root = tempfile::tempdir().unwrap();
        let entry = entry_for("mhost", &bytes, server.url("/gh/ggml-mhost.bin"));
        let error = install_one(root.path(), entry, &options())
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("localhost is not one of GitHub's asset hosts"),
            "{error}"
        );
        // https only: an http address is refused before anything is sent.
        let mut opts = options();
        opts.network.https_only = true;
        let before = server.seen().len();
        let entry = entry_for("mhttp", &bytes, server.url("/asset/ggml-mhttp.bin"));
        let error = install_one(root.path(), entry, &opts).await.unwrap_err();
        assert!(
            error.to_string().contains("is not an https address"),
            "{error}"
        );
        assert_eq!(server.seen().len(), before);
        assert_eq!(
            Network::production().final_hosts,
            [
                "release-assets.githubusercontent.com",
                "objects.githubusercontent.com"
            ]
        );
        assert!(Network::production().https_only);
    }

    #[tokio::test]
    async fn a_hash_mismatch_deletes_the_file_and_reports_msg_hash() {
        let bytes = model_bytes(SIZE, 10);
        let served = model_bytes(SIZE, 11);
        let server = github(served);
        let entry = entry_for("mhash", &bytes, server.url("/gh/ggml-mhash.bin"));
        let root = tempfile::tempdir().unwrap();
        let error = install_one(root.path(), entry, &options())
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Hash));
        assert_eq!(error.to_string(), MSG_HASH);
        let (_, partial, meta) = partial_paths(root.path(), entry);
        assert!(!partial.exists() && !meta.exists());
        assert!(!model_path(root.path(), entry).exists());
    }

    #[tokio::test]
    async fn nothing_answering_is_offline() {
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let bytes = model_bytes(1_000, 12);
        let entry = entry_for("moff", &bytes, format!("http://127.0.0.1:{port}/gh/x"));
        let root = tempfile::tempdir().unwrap();
        let error = install_one(root.path(), entry, &options())
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Offline), "{error}");
        assert_eq!(
            error.to_string(),
            "This computer cannot reach Biorouter's model repository."
        );
    }

    fn closed_port() -> u16 {
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        port
    }

    /// A hospital proxy that lets github.com through but blocks the asset hosts: the
    /// redirect answers, the asset host never does, and the install ends in MSG-OFFLINE
    /// with its air gapped route, not in a failure the catalog has no words for.
    #[tokio::test]
    async fn a_blocked_asset_host_is_offline() {
        let port = closed_port();
        let server = Server::start(move |seen, _| {
            Reply::redirect(format!(
                "http://127.0.0.1:{port}{}",
                seen.path.replacen("/gh/", "/asset/", 1)
            ))
        });
        let bytes = model_bytes(1_000, 40);
        let entry = entry_for("mblock", &bytes, server.url("/gh/ggml-mblock.bin"));
        let root = tempfile::tempdir().unwrap();
        let error = install_one(root.path(), entry, &options())
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Offline), "{error}");
        assert_eq!(server.seen().len(), RETRY_BUDGET as usize);
    }

    /// A server that ignores `Range` and cuts every body at the same place makes no
    /// progress after its first answer, so the install ends within the retry budget
    /// instead of downloading the first half forever.
    #[tokio::test]
    async fn a_body_cut_short_on_every_attempt_ends_within_the_budget() {
        let bytes = model_bytes(SIZE, 41);
        let body = bytes.clone();
        let server = Server::start(move |_, _| Reply {
            status: 200,
            body: body.clone(),
            cut_after: Some(SIZE / 2),
            ..Reply::default()
        });
        let entry = entry_for("mhalf", &bytes, server.url("/asset/ggml-mhalf.bin"));
        let root = tempfile::tempdir().unwrap();
        let error = tokio::time::timeout(
            Duration::from_secs(30),
            install_one(root.path(), entry, &options()),
        )
        .await
        .expect("the install looped")
        .unwrap_err();
        assert!(matches!(error, InstallError::KeptFailing), "{error}");
        assert_eq!(
            error.to_string(),
            "The download kept failing, so it stopped. Run the install again to continue where it stopped."
        );
        // The first attempt added half the file; each later one restarted it and added
        // nothing past that, until the budget was spent.
        assert_eq!(server.seen().len(), 1 + RETRY_BUDGET as usize);
        let (_, partial, _) = partial_paths(root.path(), entry);
        assert_eq!(
            partial_len(&partial),
            (SIZE / 2) as u64,
            "the partial is kept"
        );
    }

    /// The handshake cut short by the network (a server that accepts and closes) is a
    /// dropped connection, retried and then MSG-OFFLINE, never MSG-TLS: nothing about it
    /// says a certificate is wrong.
    #[tokio::test]
    async fn a_handshake_cut_short_is_retried_not_a_tls_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                drop(stream);
            }
        });
        let bytes = model_bytes(1_000, 42);
        let entry = entry_for("meof", &bytes, format!("https://127.0.0.1:{port}/x"));
        let root = tempfile::tempdir().unwrap();
        let error = install_one(root.path(), entry, &options())
            .await
            .unwrap_err();
        assert!(matches!(error, InstallError::Offline), "{error}");
    }

    /// Bytes that are not TLS behind an https address (a plain HTTP server, a proxy that
    /// rewrites the connection) stop the install at once (DD-21).
    #[tokio::test]
    async fn a_server_that_does_not_speak_tls_stops_at_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(Mutex::new(0usize));
        let count = Arc::clone(&accepted);
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                *count.lock().unwrap() += 1;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut hello = [0u8; 2048];
                let _ = stream.read(&mut hello);
                let _ = stream.write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                let _ = stream.flush();
                // Wait for the client to give up, so the answer is read before a reset.
                let _ = stream.read(&mut hello);
            }
        });
        let bytes = model_bytes(1_000, 43);
        let entry = entry_for("mplain", &bytes, format!("https://127.0.0.1:{port}/x"));
        let root = tempfile::tempdir().unwrap();
        let error = install_one(root.path(), entry, &options())
            .await
            .unwrap_err();
        assert!(
            matches!(&error, InstallError::Tls { host } if host == "127.0.0.1"),
            "{error}"
        );
        assert_eq!(*accepted.lock().unwrap(), 1, "a TLS failure was retried");
    }

    #[test]
    fn tls_failures_are_told_apart_by_type() {
        let certificate = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
        );
        assert_eq!(send_failure(&certificate), SendFailure::Tls);
        // Wrapped once more, as a connector may box it again.
        assert_eq!(
            send_failure(&io::Error::other(certificate)),
            SendFailure::Tls
        );
        let alert = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::AlertReceived(rustls::AlertDescription::HandshakeFailure),
        );
        assert_eq!(send_failure(&alert), SendFailure::Tls);
        for kind in [
            io::ErrorKind::UnexpectedEof,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::ConnectionAborted,
        ] {
            // The words are a handshake's, the type is the network's.
            let dropped = io::Error::new(kind, "tls handshake eof");
            assert_eq!(send_failure(&dropped), SendFailure::Network, "{kind:?}");
        }
    }

    /// DD-25: the mirror is tried first and answers with the body; a mirror that fails
    /// falls back to GitHub in the same attempt.
    #[tokio::test]
    async fn the_mirror_is_tried_first_and_github_is_the_fallback() {
        let bytes = model_bytes(SIZE, 44);
        let body = bytes.clone();
        let server = Server::start(move |seen, _| {
            if seen.path.starts_with("/broken/") {
                return Reply::status(500);
            }
            if seen.path.starts_with("/gh/") {
                return Reply::redirect(seen.path.replacen("/gh/", "/asset/", 1));
            }
            asset(seen, &body, "\"E1\"")
        });
        let root = tempfile::tempdir().unwrap();
        let mut opts = options();
        opts.network.mirror = Some(server.url("/mirror"));
        let served = entry_for("mmir", &bytes, server.url("/gh/ggml-mmir.bin"));
        let outcome = install_one(root.path(), served, &opts).await.unwrap();
        assert_eq!(outcome.source, Source::Mirror);
        assert_eq!(
            read_record(root.path(), served).unwrap().source,
            Source::Mirror
        );
        assert_eq!(server.paths(), ["/mirror/stt-mmir-r1/ggml-mmir.bin"]);
        assert_installed(root.path(), served, &bytes);

        opts.network.mirror = Some(server.url("/broken"));
        let fallback = entry_for("mfall", &bytes, server.url("/gh/ggml-mfall.bin"));
        let outcome = install_one(root.path(), fallback, &opts).await.unwrap();
        assert_eq!(outcome.source, Source::Download);
        assert_eq!(
            server.paths()[1..],
            [
                "/broken/stt-mfall-r1/ggml-mfall.bin",
                "/gh/ggml-mfall.bin",
                "/asset/ggml-mfall.bin"
            ]
        );
        assert_installed(root.path(), fallback, &bytes);
    }

    #[test]
    fn only_an_https_mirror_is_used() {
        assert_eq!(
            mirror_base(Some(" https://models.example.org/stt/ ")).as_deref(),
            Some("https://models.example.org/stt")
        );
        assert_eq!(mirror_base(Some("http://models.example.org")), None);
        assert_eq!(mirror_base(Some("models.example.org")), None);
        assert_eq!(mirror_base(Some("https://")), None);
        assert_eq!(mirror_base(Some("")), None);
        assert_eq!(mirror_base(None), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_shared_folder_rule() {
        assert_eq!(unix_folder_rule(0, 0o40755), None);
        assert_eq!(
            unix_folder_rule(0, 0o41777),
            None,
            "a sticky folder such as /tmp"
        );
        assert_eq!(unix_folder_rule(501, 0o40755), Some("is not owned by root"));
        assert_eq!(
            unix_folder_rule(0, 0o40775),
            Some("can be changed by accounts other than root")
        );
        assert_eq!(
            unix_folder_rule(0, 0o40757),
            Some("can be changed by accounts other than root")
        );
        // A folder this account owns, as an ordinary account sees it: refused, whether
        // the models folder exists yet or not.
        // SAFETY: geteuid has no preconditions and cannot fail.
        if unsafe { libc::geteuid() } != 0 {
            let dir = tempfile::tempdir().unwrap();
            let models = dir.path().join("dictation").join("models");
            let why = system_folder_problem(&models).expect("a user's folder is refused");
            assert!(why.ends_with("is not owned by root"), "{why}");
            // A link planted in that folder is judged by what it names, and its folder
            // is checked too.
            std::os::unix::fs::symlink("/", dir.path().join("link")).unwrap();
            assert!(system_folder_problem(&dir.path().join("link").join("models")).is_some());
        }
        assert_eq!(system_folder_problem(Path::new("/")), None);
    }

    #[tokio::test]
    async fn a_dropped_connection_resumes_in_the_same_install() {
        let bytes = model_bytes(SIZE, 13);
        let body = bytes.clone();
        let server = Server::start(move |seen, index| {
            let mut reply = asset(seen, &body, "\"E1\"");
            if index == 0 {
                reply.cut_after = Some(SIZE / 4);
            }
            reply
        });
        let entry = entry_for("mcut", &bytes, server.url("/asset/ggml-mcut.bin"));
        let root = tempfile::tempdir().unwrap();
        install_one(root.path(), entry, &options()).await.unwrap();
        assert_installed(root.path(), entry, &bytes);
        let seen = server.seen();
        assert_eq!(
            seen[1].header("range"),
            Some(format!("bytes={}-", SIZE / 4).as_str())
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_second_install_attaches_through_the_lock() {
        let bytes = model_bytes(SIZE, 14);
        let body = bytes.clone();
        let server = Server::start(move |seen, _| {
            let mut reply = asset(seen, &body, "\"E1\"");
            reply.delay = Some(Duration::from_millis(600));
            reply
        });
        let entry = entry_for("mlock", &bytes, server.url("/asset/ggml-mlock.bin"));
        let root = tempfile::tempdir().unwrap();
        let first_root = root.path().to_path_buf();
        let first = tokio::spawn(async move { install_one(&first_root, entry, &options()).await });
        while server.seen().is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let (tx, mut rx) = watch::channel(Progress {
            id: String::new(),
            phase: Phase::Connecting,
            downloaded: 0,
            total: 0,
        });
        let mut opts = options();
        opts.progress = Some(tx);
        let second_root = root.path().to_path_buf();
        let second = tokio::spawn(async move { install_one(&second_root, entry, &opts).await });
        rx.changed().await.unwrap();
        assert_eq!(rx.borrow().phase, Phase::Waiting);
        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap().unwrap();
        assert!(first.downloaded);
        assert!(
            !second.downloaded,
            "the second install must reuse the first's file"
        );
        assert_eq!(server.seen().len(), 1, "the file was requested twice");
        assert_installed(root.path(), entry, &bytes);
    }

    #[tokio::test]
    async fn too_little_space_gives_msg_disk_before_any_request() {
        let bytes = model_bytes(SIZE, 15);
        let server = github(bytes.clone());
        let entry = entry_for("mdisk", &bytes, server.url("/gh/ggml-mdisk.bin"));
        let root = tempfile::tempdir().unwrap();
        let mut opts = options();
        // ENOSPC as the space probe reports it: the remaining bytes plus 256 MiB, less 1.
        opts.space = Arc::new(|_: &Path| Ok(SIZE as u64 + DISK_MARGIN_BYTES - 1));
        let error = install_files(root.path(), &[entry], &opts)
            .await
            .unwrap_err();
        let InstallError::Disk {
            needed, available, ..
        } = &error
        else {
            panic!("{error}")
        };
        assert_eq!(*needed, SIZE as u64 + DISK_MARGIN_BYTES);
        assert_eq!(*available, *needed - 1);
        assert!(
            error
                .to_string()
                .starts_with("Installing mdisk needs 0.3 GB free on "),
            "{error}"
        );
        assert!(
            error.to_string().ends_with("; 0.2 GB is available."),
            "{error}"
        );
        assert!(server.seen().is_empty());
        opts.space = Arc::new(|_: &Path| Ok(SIZE as u64 + DISK_MARGIN_BYTES));
        install_files(root.path(), &[entry], &opts).await.unwrap();
        // A partial counts: only the missing bytes are needed.
        let other = entry_for("mdisk2", &bytes, server.url("/gh/ggml-mdisk2.bin"));
        plant_partial(root.path(), other, &bytes[..100_000], Some("\"E1\""));
        assert_eq!(
            remaining_bytes(root.path(), &[other]),
            SIZE as u64 - 100_000
        );
        assert_eq!(remaining_bytes(root.path(), &[entry]), 0);
    }

    #[test]
    fn gigabytes_round_up() {
        assert_eq!(gb(574_041_195), "0.6");
        assert_eq!(gb(264_477_561 + DISK_MARGIN_BYTES), "0.6");
        assert_eq!(gb(1_000_000_000), "1.0");
        assert_eq!(gb(1_000_000_001), "1.1");
        assert_eq!(gb(0), "0.0");
        assert_eq!(gb_down(1_099_999_999), "1.0");
        assert_eq!(gb_down(99_999_999), "0.0");
    }

    #[test]
    fn a_partial_is_never_loaded() {
        let bytes = model_bytes(SIZE, 16);
        let entry = entry_for("mpart", &bytes, "http://127.0.0.1:9/x".into());
        let root = tempfile::tempdir().unwrap();
        plant_partial(root.path(), entry, &bytes, None);
        assert!(matches!(
            verify_writable(root.path(), entry),
            Verdict::Missing
        ));
        // Lookups read only these two paths, and a partial is neither.
        let (_, partial, _) = partial_paths(root.path(), entry);
        assert_ne!(partial, model_path(root.path(), entry));
        assert_ne!(partial, flat_path(root.path(), entry));
        assert!(partial.to_string_lossy().ends_with(".partial"));
    }

    /// `installed.json` decides while size and mtime match; touching the file forces a
    /// full hash, and a per user copy that then fails is deleted (P-11).
    #[test]
    fn the_verification_cache_follows_size_and_mtime() {
        let bytes = model_bytes(SIZE, 17);
        let entry = entry_for("mcache", &bytes, "http://127.0.0.1:9/x".into());
        let catalog = Catalog {
            entries: Box::leak(vec![*entry].into_boxed_slice()),
            retired: &[],
        };
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("anything.dat");
        fs::write(&input, &bytes).unwrap();
        let models = root.path().join("models");
        import_from(
            &catalog,
            &models,
            &input,
            WriteMode::User,
            &|_| true,
            &roomy(),
        )
        .unwrap();
        let path = model_path(&models, entry);
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        // Same size, same mtime, other bytes: the record is trusted, nothing is hashed.
        let mut changed = bytes.clone();
        changed[1000] ^= 1;
        fs::write(&path, &changed).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        assert!(verify_writable(&models, entry).is_verified());
        // A new mtime forces the hash, which fails, so the copy is deleted.
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(mtime + Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            verify_writable(&models, entry),
            Verdict::Mismatch { deleted: true, .. }
        ));
        assert!(!path.exists());
        assert!(read_record(&models, entry).is_none());
    }

    #[test]
    fn a_readonly_copy_is_hashed_once_and_never_deleted() {
        let bytes = model_bytes(SIZE, 18);
        let entry = entry_for("mro", &bytes, "http://127.0.0.1:9/x".into());
        let root = tempfile::tempdir().unwrap();
        let flat = flat_path(root.path(), entry);
        fs::write(&flat, &bytes).unwrap();
        assert!(verify_readonly(&flat, entry).is_verified());
        let mut wrong = bytes.clone();
        wrong[7] ^= 1;
        fs::write(&flat, &wrong).unwrap();
        File::options()
            .write(true)
            .open(&flat)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(5))
            .unwrap();
        assert!(matches!(
            verify_readonly(&flat, entry),
            Verdict::Mismatch { deleted: false, .. }
        ));
        assert!(flat.exists(), "a read only copy is never deleted");
    }

    #[test]
    fn import_identifies_files_by_hash_and_lists_the_rest() {
        let bytes = model_bytes(SIZE, 19);
        let vad = model_bytes(5_000, 20);
        let speech = entry_for("mimp", &bytes, "http://127.0.0.1:9/x".into());
        let mut detector = *entry_for("vimp", &vad, "http://127.0.0.1:9/y".into());
        detector.kind = SttKind::Vad;
        detector.licenses = &[LICENSE_SILERO_VAD];
        let catalog = Catalog {
            entries: Box::leak(vec![*speech, detector].into_boxed_slice()),
            retired: &[] as &[RetiredSttModel],
        };
        let carried = tempfile::tempdir().unwrap();
        fs::write(carried.path().join("renamed-model.data"), &bytes).unwrap();
        fs::create_dir(carried.path().join("sub")).unwrap();
        fs::write(carried.path().join("sub").join("detector"), &vad).unwrap();
        fs::write(carried.path().join("notes.txt"), b"hello").unwrap();
        let mut wrong = vad.clone();
        wrong[100] ^= 1;
        fs::write(carried.path().join("same-size-other-bytes.bin"), &wrong).unwrap();
        fs::write(carried.path().join("LICENSE-openai-whisper.txt"), b"MIT").unwrap();
        fs::write(carried.path().join("SHA256SUMS"), b"x").unwrap();
        let models = tempfile::tempdir().unwrap();
        let report = import_from(
            &catalog,
            models.path(),
            carried.path(),
            WriteMode::System,
            &|_| true,
            &roomy(),
        )
        .unwrap();
        let ids: Vec<&str> = report.installed.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, ["mimp", "vimp"]);
        let unrecognized: Vec<String> = report
            .unrecognized
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(unrecognized, ["notes.txt", "same-size-other-bytes.bin"]);
        assert_eq!(
            read_record(models.path(), speech).unwrap().source,
            Source::Import
        );
        assert!(models
            .path()
            .join("vimp")
            .join("LICENSE-silero-vad.txt")
            .is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&model_path(models.path(), speech)), 0o644);
            assert_eq!(mode(&models.path().join("mimp")), 0o755);
        }
        // The same file again is already there.
        let again = import_from(
            &catalog,
            models.path(),
            &carried.path().join("renamed-model.data"),
            WriteMode::User,
            &|_| true,
            &roomy(),
        )
        .unwrap();
        assert_eq!(again.already, ["mimp"]);
        assert!(again.installed.is_empty());
        // A model the caller's policy refuses is reported and not copied.
        let other = tempfile::tempdir().unwrap();
        let refused = import_from(
            &catalog,
            other.path(),
            carried.path(),
            WriteMode::User,
            &|entry| entry.id != "mimp",
            &roomy(),
        )
        .unwrap();
        assert_eq!(refused.refused.len(), 1);
        assert_eq!(refused.refused[0].0, "mimp");
        assert!(!other.path().join("mimp").exists());
        assert!(other.path().join("vimp").exists());
    }

    #[test]
    fn remove_takes_away_everything_it_wrote() {
        let bytes = model_bytes(SIZE, 21);
        let entry = entry_for("mrm", &bytes, "http://127.0.0.1:9/x".into());
        let catalog = Catalog {
            entries: Box::leak(vec![*entry].into_boxed_slice()),
            retired: &[],
        };
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("in.bin");
        fs::write(&input, &bytes).unwrap();
        let models = root.path().join("models");
        import_from(
            &catalog,
            &models,
            &input,
            WriteMode::User,
            &|_| true,
            &roomy(),
        )
        .unwrap();
        plant_partial(&models, entry, &bytes[..10], None);
        // What an interrupted import and an interrupted record write leave behind.
        fs::write(models.join("mrm").join(".ggml-mrm.bin.import"), b"half").unwrap();
        fs::write(models.join("mrm").join("installed.tmp"), b"{").unwrap();

        // While another process installs this id it holds the lock: nothing is deleted.
        assert!(!install_running(&models, entry));
        let held = File::create(models.join("mrm").join(LOCK_FILE)).unwrap();
        fs2::FileExt::lock_exclusive(&held).unwrap();
        assert!(install_running(&models, entry));
        let busy = remove(&models, entry).unwrap_err();
        assert!(matches!(busy, RemoveError::Busy { .. }), "{busy}");
        assert_eq!(
            busy.to_string(),
            "mrm is being installed by another Biorouter process; try again when it finishes."
        );
        let (_, partial, _) = partial_paths(&models, entry);
        assert!(partial.exists(), "a running download's partial was deleted");
        assert!(model_path(&models, entry).exists());
        drop(held);
        assert!(!install_running(&models, entry));

        assert!(remove(&models, entry).unwrap());
        assert!(!models.join("mrm").exists());
        // No folder, no lock file: nothing runs, and asking creates nothing.
        assert!(!install_running(&models, entry));
        assert!(!models.join("mrm").exists());
        assert!(!models.join(PARTIAL_DIR).exists());
        assert!(!remove(&models, entry).unwrap());
    }

    fn roomy() -> SpaceProbe {
        Arc::new(|_: &Path| Ok(u64::MAX))
    }

    fn one_entry_catalog(entry: &'static SttCatalogEntry) -> Catalog {
        Catalog {
            entries: Box::leak(vec![*entry].into_boxed_slice()),
            retired: &[],
        }
    }

    /// DD-22 applies to `--from` too: too little space refuses before a byte is copied,
    /// with MSG-DISK.
    #[test]
    fn an_import_checks_the_space_first() {
        let bytes = model_bytes(SIZE, 30);
        let entry = entry_for("mspace", &bytes, "http://127.0.0.1:9/x".into());
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("in.bin");
        fs::write(&input, &bytes).unwrap();
        let models = root.path().join("models");
        let full: SpaceProbe = Arc::new(|_: &Path| Ok(SIZE as u64 + DISK_MARGIN_BYTES - 1));
        let error = import_from(
            &one_entry_catalog(entry),
            &models,
            &input,
            WriteMode::User,
            &|_| true,
            &full,
        )
        .unwrap_err();
        assert!(
            matches!(error, ImportError::Install(InstallError::Disk { .. })),
            "{error}"
        );
        assert!(error.to_string().starts_with("Installing mspace needs "));
        assert!(!models.exists(), "nothing may be written");
    }

    /// A copy that fails after its staged file exists leaves nothing behind: here the
    /// rename into place fails because a folder holds the model's name.
    #[test]
    fn a_failed_import_leaves_no_staged_copy() {
        let bytes = model_bytes(SIZE, 31);
        let entry = entry_for("mstage", &bytes, "http://127.0.0.1:9/x".into());
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("in.bin");
        fs::write(&input, &bytes).unwrap();
        let models = root.path().join("models");
        let in_the_way = model_path(&models, entry);
        fs::create_dir_all(in_the_way.join("occupied")).unwrap();
        let error = import_from(
            &one_entry_catalog(entry),
            &models,
            &input,
            WriteMode::User,
            &|_| true,
            &roomy(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("The model could not be written: "),
            "{error}"
        );
        assert!(!models
            .join("mstage")
            .join(".ggml-mstage.bin.import")
            .exists());
        // An input that cannot be read is named as such.
        let missing = root.path().join("nowhere.bin");
        let error = import_from(
            &one_entry_catalog(entry),
            &models,
            &missing,
            WriteMode::User,
            &|_| true,
            &roomy(),
        )
        .unwrap_err();
        assert!(matches!(error, ImportError::Read { .. }), "{error}");
        assert!(
            error
                .to_string()
                .starts_with(&format!("{} could not be read: ", missing.display())),
            "{error}"
        );
    }

    /// DD-23: an earlier revision in `accepted_sha256` stays loadable and is reported as
    /// superseded, and `install` fetches the current revision over it.
    #[tokio::test]
    async fn an_install_replaces_an_earlier_revision() {
        let old = model_bytes(SIZE, 32);
        let new = model_bytes(SIZE, 33);
        let server = github(new.clone());
        let current = entry_for("mrev", &new, server.url("/gh/ggml-mrev.bin"));
        let old_sha: &'static str = leak(digest_of(&old));
        let mut revised = *current;
        revised.accepted_sha256 = Box::leak(vec![current.sha256, old_sha].into_boxed_slice());
        let entry: &'static SttCatalogEntry = Box::leak(Box::new(revised));
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("old.bin");
        fs::write(&input, &old).unwrap();
        let catalog = one_entry_catalog(entry);
        import_from(
            &catalog,
            root.path(),
            &input,
            WriteMode::User,
            &|_| true,
            &roomy(),
        )
        .unwrap();
        let there = verify_writable(root.path(), entry);
        assert!(there.is_verified() && !there.is_current(), "{there:?}");
        assert_eq!(remaining_bytes(root.path(), &[entry]), SIZE as u64);

        let outcome = install_one(root.path(), entry, &options()).await.unwrap();
        assert!(outcome.downloaded, "the earlier revision was kept");
        assert_eq!(fs::read(model_path(root.path(), entry)).unwrap(), new);
        assert_eq!(
            read_record(root.path(), entry).unwrap().sha256,
            entry.sha256
        );
        assert!(verify_writable(root.path(), entry).is_current());
        assert_eq!(
            server.paths(),
            ["/gh/ggml-mrev.bin", "/asset/ggml-mrev.bin"]
        );

        // `--from` with the current revision replaces an earlier one too.
        let other = tempfile::tempdir().unwrap();
        import_from(
            &catalog,
            other.path(),
            &input,
            WriteMode::User,
            &|_| true,
            &roomy(),
        )
        .unwrap();
        let newer = root.path().join("new.bin");
        fs::write(&newer, &new).unwrap();
        let report = import_from(
            &catalog,
            other.path(),
            &newer,
            WriteMode::User,
            &|_| true,
            &roomy(),
        )
        .unwrap();
        assert_eq!(report.installed.len(), 1);
        assert!(verify_writable(other.path(), entry).is_current());
    }

    #[test]
    fn ggml_files_are_found_in_any_folder() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("llamacpp").join("deep")).unwrap();
        fs::write(dir.path().join("biorouterd"), b"\x7fELF....").unwrap();
        fs::write(
            dir.path().join("llamacpp").join("deep").join("planted"),
            model_bytes(64, 1),
        )
        .unwrap();
        fs::write(dir.path().join("gguf"), b"GGUF\x03\0\0\0").unwrap();
        fs::write(dir.path().join("tiny"), b"lm").unwrap();
        let found = ggml_files_under(dir.path()).unwrap();
        assert_eq!(
            found,
            [dir.path().join("llamacpp").join("deep").join("planted")]
        );
    }

    const PROXY_ROOT: &str = "BR_TEST_PROXY_ROOT";
    const PROXY_EXPECT: &str = "BR_TEST_PROXY_EXPECT";

    /// The child half of `the_environments_proxy_is_honoured`: one install with the
    /// environment the parent gave it, which must end as `BR_TEST_PROXY_EXPECT` says. Does
    /// nothing when run on its own.
    #[tokio::test]
    #[ignore = "the child process of the_environments_proxy_is_honoured"]
    async fn proxy_child_install() {
        let (Ok(root), Ok(expect)) = (std::env::var(PROXY_ROOT), std::env::var(PROXY_EXPECT))
        else {
            return;
        };
        let bytes = model_bytes(SIZE, 22);
        let entry = entry_for("mproxy", &bytes, "http://models.invalid/asset/x".into());
        let mut net = network();
        net.env_proxy = true;
        net.final_hosts = vec!["models.invalid".into()];
        net.retry_budget = 1;
        let result = install_one(
            Path::new(&root),
            entry,
            &InstallOptions::new(net, WriteMode::User),
        )
        .await;
        match expect.as_str() {
            "installed" => assert!(result.is_ok(), "{:?}", result.err()),
            _ => assert!(matches!(result, Err(InstallError::Offline)), "{result:?}"),
        }
    }

    /// `HTTPS_PROXY` and `NO_PROXY` reach the production client: the request goes to the
    /// proxy, unless `NO_PROXY` names the host. `HTTP_PROXY` stands in for
    /// `HTTPS_PROXY`, because the loopback server speaks plain HTTP; reqwest reads both
    /// the same way. Each install runs in a child process given its environment by
    /// `Command::env`: setting variables in this process would race the C code of the
    /// engine tests, which reads the environment from other threads.
    #[test]
    fn the_environments_proxy_is_honoured() {
        let bytes = model_bytes(SIZE, 22);
        let body = bytes.clone();
        let proxy = Server::start(move |seen, _| asset(seen, &body, "\"E1\""));
        let proxy_url = proxy.url("");
        let child = |root: &Path, expect: &str, no_proxy: Option<&str>| {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "models::tests::proxy_child_install",
                    "--exact",
                    "--ignored",
                    "--test-threads=1",
                ])
                .env(PROXY_ROOT, root)
                .env(PROXY_EXPECT, expect)
                .env("HTTP_PROXY", &proxy_url);
            for name in [
                "http_proxy",
                "HTTPS_PROXY",
                "https_proxy",
                "ALL_PROXY",
                "all_proxy",
                "NO_PROXY",
                "no_proxy",
            ] {
                command.env_remove(name);
            }
            if let Some(hosts) = no_proxy {
                command.env("NO_PROXY", hosts);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        let root = tempfile::tempdir().unwrap();
        child(root.path(), "installed", None);
        assert_eq!(proxy.paths(), ["http://models.invalid/asset/x"]);
        let entry = entry_for("mproxy", &bytes, "http://models.invalid/asset/x".into());
        assert_installed(root.path(), entry, &bytes);

        let again = tempfile::tempdir().unwrap();
        child(again.path(), "offline", Some("models.invalid"));
        assert_eq!(
            proxy.seen().len(),
            1,
            "NO_PROXY did not keep the request off the proxy"
        );
    }

    const RESUME_URL: &str = "BR_TEST_RESUME_URL";
    const RESUME_ROOT: &str = "BR_TEST_RESUME_ROOT";
    const RESUME_SIZE: usize = 2_000_000;

    /// The child half of `a_killed_install_resumes_in_another_process`; does nothing when
    /// run on its own.
    #[tokio::test]
    #[ignore = "the child process of a_killed_install_resumes_in_another_process"]
    async fn resume_child_install() {
        let (Ok(url), Ok(root)) = (std::env::var(RESUME_URL), std::env::var(RESUME_ROOT)) else {
            return;
        };
        let bytes = model_bytes(RESUME_SIZE, 23);
        let entry = entry_for("mkill", &bytes, url);
        let _ = install_one(Path::new(&root), entry, &options()).await;
    }

    #[tokio::test]
    async fn a_killed_install_resumes_in_another_process() {
        let bytes = model_bytes(RESUME_SIZE, 23);
        let body = bytes.clone();
        let server = Server::start(move |seen, index| {
            let mut reply = asset(seen, &body, "\"E7\"");
            if index == 0 {
                reply.stall_after = Some(RESUME_SIZE / 2);
            }
            reply
        });
        let url = server.url("/asset/ggml-mkill.bin");
        let entry = entry_for("mkill", &bytes, url.clone());
        let root = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "models::tests::resume_child_install",
                "--exact",
                "--ignored",
                "--test-threads=1",
            ])
            .env(RESUME_URL, &url)
            .env(RESUME_ROOT, root.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let (_, partial, meta) = partial_paths(root.path(), entry);
        let deadline = Instant::now() + Duration::from_secs(60);
        while partial_len(&partial) < (RESUME_SIZE / 2) as u64 {
            assert!(
                Instant::now() < deadline,
                "the child never wrote half the file"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(read_partial_meta(&meta).is_some_and(|m| m.etag.as_deref() == Some("\"E7\"")));
        install_one(root.path(), entry, &options()).await.unwrap();
        assert_installed(root.path(), entry, &bytes);
        let seen = server.seen();
        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen[1].header("range"),
            Some(format!("bytes={}-", RESUME_SIZE / 2).as_str())
        );
        assert_eq!(seen[1].header("if-range"), Some("\"E7\""));
    }

    #[test]
    fn content_range_and_retry_after_parse() {
        use reqwest::header::{HeaderMap, HeaderValue, CONTENT_RANGE, RETRY_AFTER};
        let headers = |name, value: &'static str| {
            let mut map = HeaderMap::new();
            map.insert(name, HeaderValue::from_static(value));
            map
        };
        assert_eq!(
            content_range_start(&headers(CONTENT_RANGE, "bytes 100-199/200")),
            Some(100)
        );
        assert_eq!(
            content_range_start(&headers(CONTENT_RANGE, "bytes */200")),
            None
        );
        assert_eq!(
            retry_after(&headers(RETRY_AFTER, "7")),
            Some(Duration::from_secs(7))
        );
        assert_eq!(
            retry_after(&headers(RETRY_AFTER, "Wed, 21 Oct 2015 07:28:00 GMT")),
            None
        );
    }
}
