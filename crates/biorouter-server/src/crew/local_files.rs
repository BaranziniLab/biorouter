use anyhow::{ensure, Context, Result};
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

#[cfg(windows)]
pub mod windows;

pub const CHUNK: usize = 128 * 1024;
pub const MAX_SIZE: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Upload,
    Download,
}

pub enum Selection {
    Source {
        file: File,
        stamp: Stamp,
        name: String,
    },
    Destination {
        directory: ProtectedDirectory,
        name: String,
        overwrite: bool,
        target: TargetApproval,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum TargetApproval {
    Cleanup,
    Absent,
    Existing(String),
}
impl TargetApproval {
    pub fn capture(directory: &Dir, name: &str) -> Result<Self> {
        let file = match directory.open_with(name, nofollow_options().read(true)) {
            Ok(file) => file.into_std(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Self::Absent),
            Err(error) => return Err(error.into()),
        };
        reject_reparse(&file)?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "Destination must be a regular file");
        #[cfg(unix)]
        let stamp = {
            use std::os::unix::fs::MetadataExt;
            serde_json::json!([
                metadata.dev(),
                metadata.ino(),
                metadata.len(),
                metadata.mtime(),
                metadata.mtime_nsec(),
                metadata.ctime(),
                metadata.ctime_nsec()
            ])
        };
        #[cfg(windows)]
        let stamp = {
            use std::os::windows::fs::MetadataExt;
            let identity = windows::file_identity(&file)?;
            serde_json::json!([
                identity.volume,
                identity.index,
                metadata.len(),
                metadata.last_write_time(),
                metadata.creation_time(),
                windows::change_time(&file)?
            ])
        };
        Ok(Self::Existing(hex::encode(Sha256::digest(
            serde_json::to_vec(&stamp)?,
        ))))
    }
    pub fn verify(&self, directory: &Dir, name: &str) -> Result<()> {
        ensure!(
            *self != Self::Cleanup && *self == Self::capture(directory, name)?,
            "Destination changed since selection; select it again to approve publication"
        );
        Ok(())
    }
}

/// Holds the selected directory identity for the full transfer. Windows also
/// retains no-delete-share ancestor handles used by path-based write-through moves.
pub struct ProtectedDirectory {
    directory: Dir,
    #[cfg(windows)]
    lease: windows::DirectoryLease,
}
impl std::ops::Deref for ProtectedDirectory {
    type Target = Dir;
    fn deref(&self) -> &Dir {
        &self.directory
    }
}
impl ProtectedDirectory {
    pub fn new(directory: Dir) -> Result<Self> {
        protected_directory(&directory)?;
        #[cfg(windows)]
        let lease = windows::DirectoryLease::acquire(&directory)?;
        Ok(Self {
            directory,
            #[cfg(windows)]
            lease,
        })
    }
    pub fn revalidate(&self) -> Result<()> {
        protected_directory(&self.directory)?;
        #[cfg(windows)]
        self.lease.revalidate()?;
        Ok(())
    }
    pub fn publish_file(
        &self,
        file: &File,
        temporary: &str,
        name: &str,
        overwrite: bool,
    ) -> Result<()> {
        self.revalidate()?;
        #[cfg(windows)]
        return require_publication(self.lease.publish(file, temporary, name, overwrite)?);
        #[cfg(not(windows))]
        {
            file.sync_all()?;
            publish_named(
                &self.directory,
                file,
                temporary.as_ref(),
                name,
                overwrite,
                None,
            )
        }
    }
    pub fn publish_selected(
        &self,
        file: &File,
        temporary: &str,
        name: &str,
        overwrite: bool,
        target: &TargetApproval,
    ) -> Result<()> {
        let overwrite = overwrite && matches!(target, TargetApproval::Existing(_));
        self.revalidate()?;
        #[cfg(windows)]
        return require_publication(self.lease.publish_selected(
            file,
            temporary,
            name,
            overwrite,
            Some(target),
        )?);
        #[cfg(not(windows))]
        {
            file.sync_all()?;
            publish_named(
                &self.directory,
                file,
                temporary.as_ref(),
                name,
                overwrite,
                Some(target),
            )
        }
    }
}

#[cfg(windows)]
fn require_publication(publication: windows::Publication) -> Result<()> {
    match publication {
        windows::Publication::Published(_) => Ok(()),
        windows::Publication::Unconfirmed { expected, diagnostic } => anyhow::bail!(
            "Publication is unconfirmed for Windows file {}:{}; inspect before retrying: {diagnostic}", expected.volume, expected.index),
    }
}

#[derive(Clone)]
pub struct Stamp {
    size: u64,
    modified: SystemTime,
}
impl Stamp {
    fn read(file: &File) -> Result<Self> {
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "Select a regular file");
        ensure!(metadata.len() <= MAX_SIZE, "Crew attachment limit is 1 GiB");
        Ok(Self {
            size: metadata.len(),
            modified: metadata.modified()?,
        })
    }
    pub fn unchanged(&self, file: &File) -> Result<()> {
        let current = Self::read(file)?;
        ensure!(
            current.size == self.size && current.modified == self.modified,
            "The selected source changed; select the original file again"
        );
        Ok(())
    }
    pub fn size(&self) -> u64 {
        self.size
    }
}

pub fn nofollow_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .mode(0o600);
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000).share_mode(7);
    }
    options
}

fn reject_link(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "Symlink file selections are not supported"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Reparse-point selections are not supported"
        );
    }
    Ok(())
}

pub fn open_directory(path: &Path, create: bool) -> Result<Dir> {
    ensure!(path.is_absolute(), "An absolute directory is required");
    let mut root = PathBuf::new();
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => root.push(prefix.as_os_str()),
            Component::RootDir => root.push(component.as_os_str()),
            Component::Normal(name) => names.push(name),
            _ => anyhow::bail!("Relative directory components are not supported"),
        }
    }
    let mut directory = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
    for name in names {
        if create && !directory.try_exists(name)? {
            let builder = cap_std::fs::DirBuilder::new();
            #[cfg(unix)]
            let builder = {
                use cap_std::fs::DirBuilderExt;
                let mut builder = builder;
                builder.mode(0o700);
                builder
            };
            match directory.create_dir_with(name, &builder) {
                Ok(()) => namespace_checkpoint(&directory)?.accept_receipt_reload_recovery(),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let mut options = nofollow_options();
        options.read(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY);
        }
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            options
                .custom_flags(0x0020_0000 | 0x0200_0000)
                .share_mode(3);
        }
        let file = directory.open_with(name, &options)?.into_std();
        ensure!(file.metadata()?.is_dir(), "Directory path changed");
        reject_reparse(&file)?;
        directory = Dir::from_std_file(file);
    }
    Ok(directory)
}

fn reject_reparse(file: &File) -> Result<()> {
    ensure!(
        !file.metadata()?.file_type().is_symlink(),
        "Symlink selection refused"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            file.metadata()?.file_attributes() & 0x400 == 0,
            "Reparse point refused"
        );
    }
    Ok(())
}

/// The code `POST /crew/files` answers a [`CredentialRefusal`] with, beside its sentence.
pub const CREDENTIAL_REFUSAL_CODE: &str = "crew_file_is_credential";

/// How much of a source file the content check reads. A credential store is small; a key
/// buried deeper in a large file is beyond this check, as it is beyond the name check.
const CREDENTIAL_SNIFF_BYTES: u64 = 64 * 1024;

/// A local path Crew refuses because the BR-23 credential floor says it holds, or would hold,
/// credentials (Q3-01).
///
/// Every selection of a local file reaches the daemon through `POST /crew/files`, and the
/// renderer holds the daemon secret and the user-action proof, so a native dialog in front of
/// that route is not a boundary for it. This refusal is: it runs here, on every registration,
/// whichever door the path came through (the Attach picker, D-DROP, the CLI, or page script).
///
/// There is deliberately no list of Crew's own. The floor is
/// [`biorouter_mcp::secret_guard::SecretGuard`] (its built-in `DEFAULT_SECRET_PATTERNS` plus the
/// machine-wide `.biorouterignore`), judged after links are resolved and with case folded, and
/// the content check is `biorouter::guardrails::secret_output`, the same detectors that
/// withhold credential material from tool output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialRefusal {
    /// An upload source. `name` is the file name as the request spelled it, made printable.
    Source { name: String },
    /// A download destination: a location the credential floor names, or (Q4-55) a settings,
    /// login or autostart location or an executable file ([`download_destination_denied`]).
    /// One sentence covers both, so it names both.
    Destination,
}

impl std::fmt::Display for CredentialRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source { name } => write!(
                f,
                "\u{201c}{name}\u{201d} looks like a credential file (a password, key or token store), so Crew won't share it."
            ),
            Self::Destination => f.write_str(
                "Crew won't save into a credential or settings location. Choose another folder.",
            ),
        }
    }
}

impl std::error::Error for CredentialRefusal {}

impl CredentialRefusal {
    fn source(path: &Path) -> Self {
        let raw = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = biorouter::utils::sanitize_untrusted_label(&raw, 255);
        Self::Source {
            name: if name.is_empty() {
                "This file".into()
            } else {
                name
            },
        }
    }
}

/// The floor's verdict on a path, before anything is opened. Only an absolute path is judged;
/// `select_local` refuses any other in words of its own.
///
/// The guard is rooted at the filesystem root, not at the file's folder. Crew has no project, so
/// only the guard's machine-wide statements apply: the built-in floor and the global
/// `.biorouterignore`. Rooted at the folder, that folder's own `.biorouterignore` would join
/// them, and could add a rule this refusal would then misname as a credential, or negate one of
/// the floor's for Crew.
fn credential_floor_denies(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let root = path.ancestors().last().unwrap_or(path);
    biorouter_mcp::secret_guard::SecretGuard::cached_for_dir(root).is_denied_resolved(path)
}

/// True when the first [`CREDENTIAL_SNIFF_BYTES`] of `file` hold credential material a
/// credential file's *name* would not reveal: a hard-linked or renamed copy of a private key,
/// an AWS credentials file, a provider-key store. Leaves the file positioned at its start.
fn holds_credential_material(file: &File) -> Result<bool> {
    let mut reader = file;
    reader.seek(SeekFrom::Start(0))?;
    let mut head = Vec::new();
    reader.take(CREDENTIAL_SNIFF_BYTES).read_to_end(&mut head)?;
    reader.seek(SeekFrom::Start(0))?;
    Ok(
        biorouter::guardrails::secret_output::redact_text(&String::from_utf8_lossy(&head))
            .is_some(),
    )
}

// ---- Q4-55: settings, login and autostart destinations -------------------------------------

/// The places a download may not write although no credential lives there (Q4-55).
///
/// **Defense in depth, not the boundary.** The renderer holds the daemon secret and the
/// user-action proof, so page script can register a download to any path it can name, and the
/// credential floor names only credential stores. A write is worse than a read: attacker-chosen
/// bytes in an rc file, `~/.ssh/authorized_keys` or a LaunchAgent are code execution or a login
/// (security round 4, F1, wrote `~/.bashrc` end to end). [`download_destination_denied`] refuses
/// those places. It does not close the door, and says so: `POST /agent/call_tool` reaches
/// `developer__shell` with the same proof, and a registration proof only the main process
/// holds, which would close both, is an open decision
/// (`docs/research/biorouter-crew/implementation-plan.md` §16, D-DROP).
struct SettingsLocations {
    /// The person's home directory. Below it, a path that passes through a component starting
    /// with `.` is refused.
    homes: Vec<PathBuf>,
    /// Folders refused whole: `~/Library` on macOS, `%APPDATA%`, `%LOCALAPPDATA%` and
    /// `~\AppData` on Windows, apart from the drives in `drives`.
    settings: Vec<PathBuf>,
    /// Cloud drives inside a settings folder, open to downloads again: on macOS every
    /// File Provider drive (Box, OneDrive, Dropbox, Google Drive) lives in
    /// `~/Library/CloudStorage`, and iCloud Drive in `~/Library/Mobile Documents`. Saving to
    /// them is ordinary work. Rules (a) and (d) still judge a destination inside one. See
    /// [`CloudDrive`].
    drives: Vec<CloudDrive>,
}

/// A folder of cloud drives inside a settings folder. A destination at least `depth`
/// components below `root`, its own name included, is inside a drive: for
/// `~/Library/CloudStorage`, whose children are the drives, that is 2 (`Box-Box/plan.csv`), so
/// `CloudStorage` itself is not a destination; for iCloud Drive's own folder it is 1.
struct CloudDrive {
    root: PathBuf,
    depth: usize,
}

/// The cloud-drive folders macOS keeps inside `~/Library`, below the home, with their
/// [`CloudDrive::depth`]. Only `com~apple~CloudDocs` is iCloud Drive; the other folders in
/// `Mobile Documents` are apps' own containers and stay refused.
const MACOS_CLOUD_DRIVES: [(&str, usize); 2] = [
    ("Library/CloudStorage", 2),
    ("Library/Mobile Documents/com~apple~CloudDocs", 1),
];

/// [`MACOS_CLOUD_DRIVES`] below `home`.
fn macos_cloud_drives(home: &Path) -> Vec<CloudDrive> {
    MACOS_CLOUD_DRIVES
        .iter()
        .map(|(folder, depth)| CloudDrive {
            root: home.join(folder),
            depth: *depth,
        })
        .collect()
}

impl SettingsLocations {
    /// This process's. On unix that is `$HOME` **and** the account's home from the user
    /// database: they differ when a profile or a launcher moves `HOME`, and the account's real
    /// rc files are no less worth protecting then.
    fn current() -> Self {
        #[cfg(unix)]
        let (homes, settings) = (
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .into_iter()
                .chain(account_home())
                .collect(),
            Vec::new(),
        );
        #[cfg(windows)]
        let (homes, settings) = (
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .into_iter()
                .collect(),
            ["APPDATA", "LOCALAPPDATA"]
                .into_iter()
                .filter_map(std::env::var_os)
                .map(PathBuf::from)
                .collect(),
        );
        #[cfg(not(any(unix, windows)))]
        let (homes, settings) = (Vec::new(), Vec::new());
        Self::new(homes, settings, Vec::new())
    }

    /// `homes`, with the platform's settings folder below each one added to `settings` and, on
    /// macOS, its cloud drives added to `drives`. A relative path and the filesystem root are
    /// dropped: neither names a person's home, and a root "home" would refuse every hidden
    /// folder on the machine.
    fn new(homes: Vec<PathBuf>, settings: Vec<PathBuf>, drives: Vec<CloudDrive>) -> Self {
        let names = |path: &PathBuf| path.is_absolute() && path.parent().is_some();
        let homes: Vec<PathBuf> = homes.into_iter().filter(names).collect();
        let below_home: &[&str] = if cfg!(target_os = "macos") {
            &["Library"]
        } else if cfg!(windows) {
            &["AppData"]
        } else {
            &[]
        };
        let settings = settings
            .into_iter()
            .chain(
                homes
                    .iter()
                    .flat_map(|home| below_home.iter().map(move |name| home.join(name))),
            )
            .filter(names)
            .collect();
        let derived: Vec<CloudDrive> = if cfg!(target_os = "macos") {
            homes
                .iter()
                .flat_map(|home| macos_cloud_drives(home))
                .collect()
        } else {
            Vec::new()
        };
        let drives = drives
            .into_iter()
            .chain(derived)
            .filter(|drive| names(&drive.root))
            .collect();
        Self {
            homes,
            settings,
            drives,
        }
    }
}

/// The account's home directory from the user database, whatever `$HOME` says.
#[cfg(unix)]
fn account_home() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let uid = unsafe { libc::geteuid() };
    let mut buffer = vec![0u8; 4096];
    loop {
        // SAFETY: every pointer is to a live local; `getpwuid_r` writes the entry's strings
        // into `buffer`, which outlives the `CStr` read from it below.
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        if status == libc::ERANGE && buffer.len() < 1 << 20 {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 || found.is_null() || entry.pw_dir.is_null() {
            return None;
        }
        let dir = unsafe { std::ffi::CStr::from_ptr(entry.pw_dir) };
        let home = PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes()));
        return home.is_absolute().then_some(home);
    }
}

/// `path` with `.` dropped and `..` taken lexically, never above the root. Nothing is opened.
fn lexical(path: &Path) -> PathBuf {
    let mut folded = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                folded.pop();
            }
            other => folded.push(other.as_os_str()),
        }
    }
    folded
}

/// `path` with links and `..` resolved the way the filesystem resolves them, as far as it
/// exists; a tail that does not exist yet is appended as [`lexical`] spells it.
fn resolved(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    let spelled = lexical(path);
    for ancestor in spelled.ancestors().skip(1) {
        if let (Ok(canonical), Ok(rest)) = (
            std::fs::canonicalize(ancestor),
            spelled.strip_prefix(ancestor),
        ) {
            return canonical.join(rest);
        }
    }
    spelled
}

/// The components of `path` after `root`, when `root` is a prefix of it, compared with case
/// folded: on APFS and NTFS `~/LIBRARY` opens `~/Library`.
fn below(path: &Path, root: &Path) -> Option<Vec<String>> {
    let fold = |component: Component| component.as_os_str().to_string_lossy().to_lowercase();
    let mut rest = path.components();
    for expected in root.components() {
        if fold(rest.next()?) != fold(expected) {
            return None;
        }
    }
    Some(rest.map(fold).collect())
}

/// Q4-55: is `path` a download destination Crew refuses although the credential floor does
/// not name it? True when, spelled as written (with `..` folded), with its folder's links
/// resolved, or through the file's own link:
///
/// - (a) it is below the home directory and passes through a component that starts with `.`:
///   rc and profile files, all of `~/.ssh` including `authorized_keys` and `config`,
///   `~/.gitconfig`, `~/.config/**` (autostart, systemd user units), `~/.local/**`, `.git/**`;
/// - (b) it is under `~/Library` (macOS), outside the cloud drives macOS keeps there
///   (`~/Library/CloudStorage/<drive>/**` for Box, OneDrive, Dropbox and Google Drive, and
///   iCloud Drive's `~/Library/Mobile Documents/com~apple~CloudDocs/**`), where (a) and (d)
///   still apply;
/// - (c) it is under `%APPDATA%` or `%LOCALAPPDATA%` (Windows), the Startup folder included;
/// - (d) it names an existing file with an execute bit (unix), which a replacement would take
///   over.
///
/// A cloud drive is judged in the same spelling that put the path in a settings folder, so a
/// link inside a drive that leads to `~/Library/LaunchAgents` is refused as the place it
/// reaches.
///
/// On unix the folder is also matched by device and inode, so a route no string comparison
/// sees (a macOS firmlink such as `/System/Volumes/Data/Users/…`, a bind mount) is refused too.
fn download_destination_denied(path: &Path, locations: &SettingsLocations) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let (Some(name), Some(parent)) = (path.file_name(), path.parent()) else {
        return false;
    };
    let folder = resolved(parent);
    let mut spellings = vec![lexical(path), folder.join(name)];
    spellings.extend(std::fs::canonicalize(path));
    let roots = |paths: &[PathBuf]| -> Vec<PathBuf> {
        paths
            .iter()
            .flat_map(|root| [lexical(root), resolved(root)])
            .collect()
    };
    let (homes, settings) = (roots(&locations.homes), roots(&locations.settings));
    let drives: Vec<CloudDrive> = locations
        .drives
        .iter()
        .flat_map(|drive| {
            [lexical(&drive.root), resolved(&drive.root)].map(|root| CloudDrive {
                root,
                depth: drive.depth,
            })
        })
        .collect();
    let hidden = |rest: Vec<String>| rest.iter().any(|name| name.starts_with('.'));
    spellings.iter().any(|spelled| {
        homes
            .iter()
            .any(|home| below(spelled, home).is_some_and(hidden))
            || in_settings_folder(spelled, &settings, &drives)
    }) || denied_by_identity(&folder, name, locations, &drives)
        || names_an_executable(path)
}

/// Rules (b) and (c) for one spelling: below a settings folder and not inside a cloud drive.
fn in_settings_folder(spelled: &Path, settings: &[PathBuf], drives: &[CloudDrive]) -> bool {
    settings.iter().any(|root| below(spelled, root).is_some()) && !in_cloud_drive(spelled, drives)
}

/// Whether `spelled` is inside one of `drives`, as [`CloudDrive::depth`] defines inside.
fn in_cloud_drive(spelled: &Path, drives: &[CloudDrive]) -> bool {
    drives
        .iter()
        .any(|drive| below(spelled, &drive.root).is_some_and(|rest| rest.len() >= drive.depth))
}

/// Rules (a) to (c) by device and inode: walk the resolved folder's ancestors and compare each
/// with the home and settings folders themselves. Below a settings folder found that way, the
/// rest of the path is spelled again from that folder's own name, and a cloud drive is judged
/// on that spelling, as [`in_settings_folder`] judges one.
#[cfg(unix)]
fn denied_by_identity(
    folder: &Path,
    name: &std::ffi::OsStr,
    locations: &SettingsLocations,
    drives: &[CloudDrive],
) -> bool {
    use std::os::unix::fs::MetadataExt;
    let identity = |path: &Path| std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()));
    let homes: Vec<_> = locations.homes.iter().filter_map(|p| identity(p)).collect();
    let settings: Vec<_> = locations
        .settings
        .iter()
        .filter_map(|p| identity(p).map(|id| (id, p)))
        .collect();
    folder.ancestors().any(|ancestor| {
        let Some(this) = identity(ancestor) else {
            return false;
        };
        let rest = || {
            folder
                .strip_prefix(ancestor)
                .unwrap_or(Path::new(""))
                .join(name)
        };
        settings
            .iter()
            .any(|(id, root)| *id == this && !in_cloud_drive(&root.join(rest()), drives))
            || (homes.contains(&this)
                && rest()
                    .components()
                    .any(|part| part.as_os_str().to_string_lossy().starts_with('.')))
    })
}

#[cfg(not(unix))]
fn denied_by_identity(
    _: &Path,
    _: &std::ffi::OsStr,
    _: &SettingsLocations,
    _: &[CloudDrive],
) -> bool {
    false
}

/// Rule (d): an existing file with any execute bit, following a link as the replacement would
/// be judged by whoever runs it.
#[cfg(unix)]
fn names_an_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn names_an_executable(_: &Path) -> bool {
    false
}

/// What every download destination must pass, whichever door registered it: the credential
/// floor (Q3-01) and the settings locations (Q4-55), answered with one refusal.
fn refuse_destination(path: &Path, locations: &SettingsLocations) -> Result<()> {
    if credential_floor_denies(path) || download_destination_denied(path, locations) {
        return Err(CredentialRefusal::Destination.into());
    }
    Ok(())
}

pub fn select(path: &Path, direction: Direction, overwrite: bool) -> Result<Selection> {
    select_with(path, direction, overwrite, &SettingsLocations::current())
}

fn select_with(
    path: &Path,
    direction: Direction,
    overwrite: bool,
    locations: &SettingsLocations,
) -> Result<Selection> {
    if direction == Direction::Download {
        refuse_destination(path, locations)?;
    }
    select_local(path, direction, overwrite, false)
}

/// Removing this transfer's own `.part` file is the one selection neither the floor nor the
/// settings locations judge: a partial left in a folder they name (by a receipt older than
/// the rule) must still be removable.
pub fn select_cleanup(path: &Path) -> Result<Selection> {
    select_local(path, Direction::Download, false, true)
}

pub fn select_download_replay(path: &Path, overwrite: bool) -> Result<Selection> {
    select_download_replay_with(path, overwrite, &SettingsLocations::current())
}

fn select_download_replay_with(
    path: &Path,
    overwrite: bool,
    locations: &SettingsLocations,
) -> Result<Selection> {
    refuse_destination(path, locations)?;
    select_local(path, Direction::Download, overwrite, true)
}

pub fn destination_selection_identity(selection: &Selection) -> Result<Option<String>> {
    let Selection::Destination {
        directory,
        name,
        overwrite,
        ..
    } = selection
    else {
        return Ok(None);
    };
    Ok(Some(hex::encode(Sha256::digest(serde_json::to_vec(
        &serde_json::json!([destination_identity(directory, name)?, overwrite]),
    )?))))
}

pub fn selection_identity(selection: &Selection) -> Result<String> {
    let value = match selection {
        Selection::Source { file, stamp, name } => {
            stamp.unchanged(file)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let metadata = file.metadata()?;
                serde_json::json!([
                    "source",
                    metadata.dev(),
                    metadata.ino(),
                    name,
                    metadata.len(),
                    metadata.mtime(),
                    metadata.mtime_nsec()
                ])
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                let identity = windows::file_identity(file)?;
                let metadata = file.metadata()?;
                serde_json::json!([
                    "source",
                    identity.volume,
                    identity.index,
                    name,
                    metadata.len(),
                    metadata.last_write_time()
                ])
            }
            #[cfg(not(any(unix, windows)))]
            anyhow::bail!("Secure local source identity is unavailable on this platform")
        }
        Selection::Destination {
            directory,
            name,
            overwrite,
            target,
        } => {
            serde_json::json!([
                "destination",
                destination_identity(directory, name)?,
                overwrite,
                target
            ])
        }
    };
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&value)?)))
}

fn select_local(
    path: &Path,
    direction: Direction,
    overwrite: bool,
    cleanup: bool,
) -> Result<Selection> {
    ensure!(path.is_absolute(), "Select an absolute local path");
    let name = path
        .file_name()
        .context("Select a file, not a directory")?
        .to_str()
        .context("Selected filename must be valid Unicode")?
        .to_owned();
    let parent = path
        .parent()
        .context("Selected file has no parent directory")?;
    // Judged before the folder is opened, and before a link is refused as a link, so a
    // credential store is named as one whichever way it was reached.
    if direction == Direction::Upload && credential_floor_denies(path) {
        return Err(CredentialRefusal::source(path).into());
    }
    let directory = open_directory(parent, false)?;
    match direction {
        Direction::Upload => {
            reject_link(path)?;
            let file = directory
                .open_with(&name, nofollow_options().read(true))?
                .into_std();
            reject_reparse(&file)?;
            let stamp = Stamp::read(&file)?;
            if holds_credential_material(&file)? {
                return Err(CredentialRefusal::source(path).into());
            }
            Ok(Selection::Source { file, stamp, name })
        }
        Direction::Download => {
            protected_directory(&directory)?;
            if !cleanup && directory.try_exists(&name)? {
                reject_link(path)?;
                ensure!(
                    directory.metadata(&name)?.is_file() && overwrite,
                    "Destination exists; explicitly approve replacement or select another filename"
                );
            }
            let target = if cleanup {
                TargetApproval::Cleanup
            } else {
                TargetApproval::capture(&directory, &name)?
            };
            Ok(Selection::Destination {
                target,
                directory: ProtectedDirectory::new(directory)?,
                name,
                overwrite,
            })
        }
    }
}

impl Selection {
    pub fn direction(&self) -> Direction {
        match self {
            Self::Source { .. } => Direction::Upload,
            Self::Destination { .. } => Direction::Download,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Source { name, .. } | Self::Destination { name, .. } => name,
        }
    }
    pub fn size(&self) -> Option<u64> {
        match self {
            Self::Source { stamp, .. } => Some(stamp.size()),
            _ => None,
        }
    }
}

pub fn hash(file: &mut File, size: u64) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; CHUNK];
    let mut remaining = size;
    while remaining > 0 {
        let wanted = remaining.min(CHUNK as u64) as usize;
        file.read_exact(&mut buffer[..wanted])?;
        hash.update(&buffer[..wanted]);
        remaining -= wanted as u64;
    }
    Ok(hex::encode(hash.finalize()))
}

pub fn part_name(id: &str) -> PathBuf {
    PathBuf::from(format!(".biorouter-crew-{id}.part"))
}

/// Unix checkpoints synchronize directory entries. Windows checkpoints validate
/// the namespace only: creation/removal can require receipt reload/reselection
/// after a crash. Windows receipt and final-file publication use write-through
/// moves separately; no directory FlushFileBuffers equivalence is claimed.
#[derive(Clone, Copy)]
pub enum NamespaceCheckpoint {
    #[cfg(unix)]
    DirectorySynced,
    #[cfg(windows)]
    ReloadRecoveryRequired,
}
impl NamespaceCheckpoint {
    pub fn accept_receipt_reload_recovery(self) {
        match self {
            #[cfg(unix)]
            Self::DirectorySynced => {}
            #[cfg(windows)]
            Self::ReloadRecoveryRequired => {}
        }
    }
}
pub fn namespace_checkpoint(directory: &Dir) -> Result<NamespaceCheckpoint> {
    #[cfg(unix)]
    {
        directory.try_clone()?.into_std_file().sync_all()?;
        Ok(NamespaceCheckpoint::DirectorySynced)
    }
    #[cfg(windows)]
    {
        let file = directory.try_clone()?.into_std_file();
        ensure!(
            file.metadata()?.is_dir(),
            "Namespace checkpoint requires a directory"
        );
        windows::file_identity(&file)?;
        Ok(NamespaceCheckpoint::ReloadRecoveryRequired)
    }
}

pub fn publish(directory: &Dir, id: &str, name: &str, overwrite: bool) -> Result<()> {
    let part = part_name(id);
    #[cfg(windows)]
    {
        let selected = ProtectedDirectory::new(directory.try_clone()?)?;
        let file = directory
            .open_with(&part, nofollow_options().read(true).write(true))?
            .into_std();
        selected.publish_file(
            &file,
            part.to_str().context("Invalid partial filename")?,
            name,
            overwrite,
        )
    }
    #[cfg(not(windows))]
    {
        let file = directory
            .open_with(&part, nofollow_options().read(true).write(true))?
            .into_std();
        publish_named(directory, &file, &part, name, overwrite, None)
    }
}

#[cfg(not(windows))]
fn publish_named(
    directory: &Dir,
    file: &File,
    part: &Path,
    name: &str,
    overwrite: bool,
    target: Option<&TargetApproval>,
) -> Result<()> {
    protected_directory(directory)?;
    verify_named_file(directory, part, file)?;
    if let Some(target) = target {
        target.verify(directory, name)?;
    }
    if overwrite {
        if let Ok(metadata) = directory.symlink_metadata(name) {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Destination became a non-regular file; choose another destination"
            );
        }
        directory.rename(part, directory, name)?;
    } else {
        directory.hard_link(part, directory, name)?;
        directory.remove_file(part)?;
    }
    verify_named_file(directory, Path::new(name), file)?;
    namespace_checkpoint(directory)?.accept_receipt_reload_recovery();
    Ok(())
}

#[cfg(unix)]
pub(crate) fn verify_named_file(directory: &Dir, name: &Path, file: &File) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let named = directory
        .open_with(name, nofollow_options().read(true))?
        .into_std();
    let expected = file.metadata()?;
    let actual = named.metadata()?;
    ensure!(
        actual.is_file() && actual.dev() == expected.dev() && actual.ino() == expected.ino(),
        "Publication file identity changed; inspect the destination before retrying"
    );
    Ok(())
}

pub fn protected_directory(directory: &Dir) -> Result<()> {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        let metadata = directory.dir_metadata()?;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o022 == 0,
            "Choose an output directory owned by your account that other accounts cannot modify"
        );
    }
    #[cfg(target_os = "macos")]
    reject_acl_allows(&directory.try_clone()?.into_std_file())?;
    #[cfg(windows)]
    return windows::protected_directory(directory);
    #[cfg(not(windows))]
    Ok(())
}

pub fn destination_identity(directory: &Dir, name: &str) -> Result<String> {
    protected_directory(directory)?;
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        let metadata = directory.dir_metadata()?;
        Ok(hex::encode(Sha256::digest(
            format!("{}:{}:{name}", metadata.dev(), metadata.ino()).as_bytes(),
        )))
    }
    #[cfg(windows)]
    {
        let lease = windows::DirectoryLease::acquire(directory)?;
        let identity = lease.identity();
        Ok(hex::encode(Sha256::digest(
            format!("{}:{}:{name}", identity.volume, identity.index).as_bytes(),
        )))
    }
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("Secure local directory identity is unavailable on this platform")
}

pub fn validate_file_acl(file: &File) -> Result<()> {
    #[cfg(target_os = "macos")]
    reject_acl_allows(file)?;
    #[cfg(windows)]
    windows::validate_cleanup(file)?;
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = file;
    Ok(())
}

#[cfg(target_os = "macos")]
fn reject_acl_allows(file: &File) -> Result<()> {
    use std::ffi::c_void;
    use std::os::fd::AsRawFd;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: i32, kind: i32) -> *mut c_void;
        fn acl_get_entry(acl: *mut c_void, entry_id: i32, entry: *mut *mut c_void) -> i32;
        fn acl_get_tag_type(entry: *mut c_void, tag: *mut i32) -> i32;
        fn acl_free(acl: *mut c_void) -> i32;
    }
    let acl = unsafe { acl_get_fd_np(file.as_raw_fd(), 0x100) };
    if acl.is_null() {
        let error = std::io::Error::last_os_error();
        // Darwin reports an absent FILESEC_ACL property as ENOENT on a valid descriptor.
        if error.raw_os_error() == Some(libc::ENOENT) {
            file.metadata()?;
            return Ok(());
        }
        return Err(error).context("Could not verify extended ACL");
    }
    let result = (|| {
        let mut entry = std::ptr::null_mut();
        let mut selector = 0;
        loop {
            if unsafe { acl_get_entry(acl, selector, &mut entry) } != 0 {
                ensure!(
                    std::io::Error::last_os_error().raw_os_error() == Some(libc::EINVAL),
                    "Could not enumerate destination ACL"
                );
                break;
            }
            let mut tag = 0;
            ensure!(
                unsafe { acl_get_tag_type(entry, &mut tag) } == 0,
                "Could not verify destination ACL entry"
            );
            ensure!(
                tag != 1,
                "Choose a private output directory without extended ACL allow grants"
            );
            selector = -1;
        }
        Ok(())
    })();
    unsafe {
        acl_free(acl);
    }
    result
}

/// Q3-01: the BR-23 credential floor on every local file selection.
///
/// Every row runs against a throwaway HOME holding fake credentials — never the operator's real
/// `~/.aws`, `~/.ssh` or `~/.config/biorouter`. Key-shaped literals are assembled at run time so
/// none sits in the source for a secret scanner to trip on.
#[cfg(all(test, unix))]
mod credential_floor_tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};

    fn fake_private_key() -> String {
        let kind = ["OPENSSH", "PRIVATE", "KEY"].join(" ");
        format!(
            "-----BEGIN {kind}-----\nZmFrZSBrZXkgbWF0ZXJpYWwgZm9yIHRlc3RzIG9ubHk=\n-----END {kind}-----\n"
        )
    }

    fn fake_aws_credentials() -> String {
        let id = format!("{}{}", "AKIA", "FAKEFAKEFAKE0000");
        let secret = format!("{}{}", "fakeSecretKeyForTestsOnly", "0".repeat(15));
        format!("[default]\naws_access_key_id = {id}\naws_secret_access_key = {secret}\n")
    }

    /// The shape of a keyring-less profile's `secrets.yaml`.
    fn fake_provider_store() -> String {
        format!(
            "VERSA_AZURE_API_KEY: {}{}\n",
            "fakeVersaKey", "0123456789abcdef"
        )
    }

    struct FakeHome {
        _dir: tempfile::TempDir,
        home: PathBuf,
        data: PathBuf,
        profile_secrets: PathBuf,
    }

    impl FakeHome {
        fn new() -> Self {
            let base = fs::canonicalize(std::env::temp_dir()).unwrap();
            let dir = tempfile::tempdir_in(base).unwrap();
            // Canonical, because `open_directory` refuses a linked folder (macOS `/var`).
            let root = fs::canonicalize(dir.path()).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let home = root.join("home");
            let data = home.join("data");
            let config = home.join("profile/biorouter/config");
            for folder in [
                home.join(".ssh"),
                home.join(".aws"),
                data.clone(),
                config.clone(),
            ] {
                fs::create_dir_all(&folder).unwrap();
            }
            for folder in [&home, &home.join(".ssh"), &home.join(".aws"), &data] {
                fs::set_permissions(folder, fs::Permissions::from_mode(0o700)).unwrap();
            }
            fs::write(home.join(".ssh/id_ed25519"), fake_private_key()).unwrap();
            fs::write(home.join(".ssh/id_rsa"), fake_private_key()).unwrap();
            fs::write(
                home.join(".ssh/id_ed25519.pub"),
                "ssh-ed25519 AAAAC3fake tester@example\n",
            )
            .unwrap();
            fs::write(home.join(".ssh/config"), "Host lab\n  User tester\n").unwrap();
            fs::write(home.join(".aws/credentials"), fake_aws_credentials()).unwrap();
            let profile_secrets = config.join("secrets.yaml");
            fs::write(&profile_secrets, fake_provider_store()).unwrap();
            fs::write(
                data.join(".env"),
                "LAB_DB_PASSWORD=fakePassword0123456789\n",
            )
            .unwrap();
            fs::write(data.join("gina-assay.csv"), "sample,od600\ngina-1,0.42\n").unwrap();
            Self {
                _dir: dir,
                home,
                data,
                profile_secrets,
            }
        }

        fn credentials(&self) -> Vec<PathBuf> {
            vec![
                self.home.join(".ssh/id_ed25519"),
                self.home.join(".ssh/id_rsa"),
                self.home.join(".aws/credentials"),
                self.profile_secrets.clone(),
                self.data.join(".env"),
            ]
        }
    }

    fn refusal(result: Result<Selection>) -> CredentialRefusal {
        let error = match result {
            Ok(selection) => panic!("expected a credential refusal for {}", selection.name()),
            Err(error) => error,
        };
        error
            .downcast_ref::<CredentialRefusal>()
            .cloned()
            .unwrap_or_else(|| panic!("expected a credential refusal, got: {error:#}"))
    }

    fn upload(path: &Path) -> Result<Selection> {
        select(path, Direction::Upload, false)
    }

    #[test]
    fn credential_files_are_refused_by_name_with_a_plain_sentence() {
        let home = FakeHome::new();
        for path in home.credentials() {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            assert_eq!(
                refusal(upload(&path)),
                CredentialRefusal::Source { name: name.clone() },
                "{}",
                path.display()
            );
        }
        // Named by the floor, holding nothing the content check would see: the name alone
        // refuses them.
        for name in [
            "secrets.json",
            ".env.production",
            "server.pem",
            "id_ecdsa",
            "cert.p12",
        ] {
            let path = home.data.join(name);
            fs::write(&path, "placeholder\n").unwrap();
            assert_eq!(
                refusal(upload(&path)),
                CredentialRefusal::Source { name: name.into() },
                "{name}"
            );
        }
        assert_eq!(
            refusal(upload(&home.profile_secrets)).to_string(),
            "\u{201c}secrets.yaml\u{201d} looks like a credential file (a password, key or token store), so Crew won't share it."
        );
    }

    #[test]
    fn a_link_to_a_credential_is_refused_as_the_credential_it_reaches() {
        let home = FakeHome::new();
        for (index, target) in home.credentials().into_iter().enumerate() {
            let link = home.data.join(format!("shared-{index}.csv"));
            symlink(&target, &link).unwrap();
            assert_eq!(
                refusal(upload(&link)),
                CredentialRefusal::Source {
                    name: format!("shared-{index}.csv")
                },
                "link to {}",
                target.display()
            );
        }
        // A linked folder: the path spells `aws-folder/credentials`, which no pattern names,
        // until the link is resolved to `.aws/credentials`.
        symlink(home.home.join(".aws"), home.data.join("aws-folder")).unwrap();
        assert!(matches!(
            refusal(upload(&home.data.join("aws-folder/credentials"))),
            CredentialRefusal::Source { .. }
        ));
    }

    #[test]
    fn a_case_variant_spelling_is_refused_before_anything_is_opened() {
        let home = FakeHome::new();
        // On a case-insensitive volume (APFS, NTFS) these open the real files; on a
        // case-sensitive one they do not exist. The floor refuses both by name, first.
        for spelling in [
            ".SSH/ID_ED25519",
            ".AWS/Credentials",
            "profile/biorouter/config/SECRETS.YAML",
        ] {
            let path = home.home.join(spelling);
            assert!(
                matches!(refusal(upload(&path)), CredentialRefusal::Source { .. }),
                "{spelling}"
            );
        }
    }

    #[test]
    fn a_hard_linked_or_renamed_copy_is_refused_by_its_content() {
        let home = FakeHome::new();
        let copies = [
            (home.home.join(".ssh/id_ed25519"), "notes.txt"),
            (home.home.join(".aws/credentials"), "table.csv"),
            (home.profile_secrets.clone(), "config-backup.txt"),
        ];
        for (original, alias) in copies {
            let hard_link = home.data.join(alias);
            fs::hard_link(&original, &hard_link).unwrap();
            assert_eq!(
                refusal(upload(&hard_link)),
                CredentialRefusal::Source { name: alias.into() },
                "hard link to {}",
                original.display()
            );
        }
        // An OpenSSH key saved under a name nothing would guess.
        let renamed = home.data.join("lab-backup-2026.dat");
        fs::write(&renamed, format!("exported\n{}", fake_private_key())).unwrap();
        assert!(matches!(
            refusal(upload(&renamed)),
            CredentialRefusal::Source { .. }
        ));
    }

    #[test]
    fn ordinary_files_are_still_shared() {
        let home = FakeHome::new();
        let csv = home.data.join("gina-assay.csv");
        let selected = upload(&csv).unwrap();
        assert_eq!(selected.name(), "gina-assay.csv");
        assert_eq!(selected.size(), Some(25));
        // The floor names private keys, not everything in `~/.ssh`.
        for path in [
            home.home.join(".ssh/id_ed25519.pub"),
            home.home.join(".ssh/config"),
        ] {
            assert!(upload(&path).is_ok(), "{}", path.display());
        }
        // A file that talks about keys without holding one.
        let prose = home.data.join("setup.md");
        fs::write(
            &prose,
            "Set OPENAI_API_KEY=<your key> and keep id_ed25519 private.\n",
        )
        .unwrap();
        assert!(upload(&prose).is_ok());
    }

    #[test]
    fn a_credential_location_is_refused_as_a_download_destination() {
        let home = FakeHome::new();
        let destinations = [
            (home.home.join(".ssh/id_ed25519"), true),
            (home.home.join(".ssh/id_ecdsa"), false),
            (home.home.join(".aws/credentials"), true),
            (home.profile_secrets.clone(), true),
            (home.data.join(".env"), true),
            (home.home.join(".AWS/CREDENTIALS"), true),
        ];
        for (path, overwrite) in destinations {
            assert_eq!(
                refusal(select(&path, Direction::Download, overwrite)),
                CredentialRefusal::Destination,
                "{}",
                path.display()
            );
            assert_eq!(
                refusal(select_download_replay(&path, overwrite)),
                CredentialRefusal::Destination,
                "replay {}",
                path.display()
            );
        }
        assert_eq!(
            CredentialRefusal::Destination.to_string(),
            "Crew won't save into a credential or settings location. Choose another folder."
        );
        // Removing a transfer's own partial is not judged: it must stay possible.
        assert!(select_cleanup(&home.home.join(".ssh/id_ecdsa")).is_ok());
        // An ordinary destination still works.
        let saved = select(&home.data.join("results.csv"), Direction::Download, false).unwrap();
        assert_eq!(saved.name(), "results.csv");
    }

    /// Q4-56: security round 4 uploaded each of these with a 200. Their contents hold nothing
    /// the content check recognises (a netrc password, a pgpass row, a `user:token@host` URL,
    /// docker's base64 `auth`, a lowercase kubeconfig `token:`), so the name must refuse them.
    #[test]
    fn the_long_tail_of_credential_stores_is_refused() {
        let home = FakeHome::new();
        for (store, text) in [
            (".netrc", "machine lab login tester password notapassword\n"),
            ("_netrc", "machine lab login tester password notapassword\n"),
            (".pgpass", "db:5432:lab:tester:notapassword\n"),
            (".git-credentials", "https://tester:notatoken@git.example\n"),
            (
                ".docker/config.json",
                "{\"auths\":{\"x\":{\"auth\":\"dGVzdDp0ZXN0\"}}}\n",
            ),
            (".kube/config", "users:\n- user:\n    token: notatoken\n"),
            (".config/gh/hosts.yml", "github.com:\n  user: tester\n"),
        ] {
            let path = home.home.join(store);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
            fs::write(&path, text).unwrap();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            assert_eq!(
                refusal(upload(&path)),
                CredentialRefusal::Source { name },
                "{store}"
            );
            assert_eq!(
                refusal(select(&path, Direction::Download, true)),
                CredentialRefusal::Destination,
                "download over {store}"
            );
        }
    }

    #[test]
    fn a_refused_name_is_made_printable() {
        let home = FakeHome::new();
        let tricky = home.data.join("id_rsa\u{202e}vsc.\n");
        fs::write(&tricky, fake_private_key()).unwrap();
        let CredentialRefusal::Source { name } = refusal(upload(&tricky)) else {
            panic!("expected a source refusal");
        };
        assert!(!name.chars().any(|c| c.is_control() || c == '\u{202e}'));
        assert!(name.starts_with("id_rsa"));
    }
}

/// Q4-55: settings, login and autostart locations and executables are refused as download
/// destinations, as defense in depth beside the credential floor.
///
/// Every row runs against a throwaway HOME handed to the check as its home; the operator's
/// real home, `~/.ssh`, `~/Library` and `~/.config/biorouter` are never read or written.
#[cfg(all(test, unix))]
mod settings_destination_tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};

    /// Creates `folder` and every missing folder above it as owner-only, whatever the umask: a
    /// group-writable folder is refused for a reason of its own (`protected_directory`), which
    /// would hide what these rows test.
    fn private_folder(folder: &Path) {
        let mut missing = Vec::new();
        for ancestor in folder.ancestors() {
            if ancestor.exists() {
                break;
            }
            missing.push(ancestor.to_path_buf());
        }
        fs::create_dir_all(folder).unwrap();
        for created in missing {
            fs::set_permissions(&created, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }

    struct FakeHome {
        _dir: tempfile::TempDir,
        /// The folder the fake HOME lives in, outside the home itself.
        root: PathBuf,
        home: PathBuf,
    }

    impl FakeHome {
        fn new() -> Self {
            let base = fs::canonicalize(std::env::temp_dir()).unwrap();
            let dir = tempfile::tempdir_in(base).unwrap();
            // Canonical, because `open_directory` refuses a linked folder (macOS `/var`).
            let root = fs::canonicalize(dir.path()).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let home = root.join("home");
            for folder in [
                ".ssh",
                ".config/autostart",
                ".config/systemd/user",
                ".local/bin",
                "Library/LaunchAgents",
                "Library/CloudStorage/Box-Box/Lab data",
                "Library/CloudStorage/OneDrive-UCSF",
                "Library/CloudStorage/Dropbox-LCATeam/.dropbox.cache",
                "Library/CloudStorage/GoogleDrive-tester@example.org/My Drive",
                "Library/Mobile Documents/com~apple~CloudDocs/Papers",
                "Library/Mobile Documents/com~apple~Numbers/Documents",
                "Downloads",
                "bin",
                "project/results",
                "project/.git/hooks",
            ] {
                private_folder(&home.join(folder));
            }
            for (file, text) in [
                (".bashrc", "# rc\n"),
                (".zshrc", "# rc\n"),
                (".profile", "# profile\n"),
                (".gitconfig", "[user]\n  name = tester\n"),
                (
                    ".ssh/authorized_keys",
                    "ssh-ed25519 AAAAC3fake tester@example\n",
                ),
                (".ssh/config", "Host lab\n  User tester\n"),
                ("Downloads/data.csv", "sample,od600\ngina-1,0.42\n"),
            ] {
                fs::write(home.join(file), text).unwrap();
            }
            Self {
                _dir: dir,
                root,
                home,
            }
        }

        /// The fake HOME as the check's home. Linux has no `~/Library` and no cloud drives
        /// there, so they are named here explicitly and the macOS rows run on every unix;
        /// `macos_derives_library_from_home` proves the real derivation.
        fn locations(&self) -> SettingsLocations {
            SettingsLocations::new(
                vec![self.home.clone()],
                vec![self.home.join("Library")],
                macos_cloud_drives(&self.home),
            )
        }

        fn download(&self, path: &Path, overwrite: bool) -> Result<Selection> {
            select_with(path, Direction::Download, overwrite, &self.locations())
        }

        fn replay(&self, path: &Path, overwrite: bool) -> Result<Selection> {
            select_download_replay_with(path, overwrite, &self.locations())
        }
    }

    fn refusal(result: Result<Selection>) -> CredentialRefusal {
        let error = match result {
            Ok(selection) => panic!("expected a destination refusal for {}", selection.name()),
            Err(error) => error,
        };
        error
            .downcast_ref::<CredentialRefusal>()
            .cloned()
            .unwrap_or_else(|| panic!("expected a destination refusal, got: {error:#}"))
    }

    fn assert_refused(fake: &FakeHome, path: &Path, overwrite: bool) {
        assert_eq!(
            refusal(fake.download(path, overwrite)),
            CredentialRefusal::Destination,
            "download {}",
            path.display()
        );
        assert_eq!(
            refusal(fake.replay(path, overwrite)),
            CredentialRefusal::Destination,
            "replay {}",
            path.display()
        );
    }

    #[test]
    fn login_settings_and_autostart_files_are_refused() {
        let fake = FakeHome::new();
        for (spelled, overwrite) in [
            (".bashrc", true),
            (".zshrc", true),
            (".zshenv", false),
            (".profile", true),
            (".gitconfig", true),
            (".ssh/authorized_keys", true),
            (".ssh/authorized_keys", false),
            (".ssh/config", true),
            (".ssh/rc", false),
            (".config/autostart/sync.desktop", false),
            (".config/systemd/user/sync.service", false),
            (".local/bin/python3", false),
            ("Library/LaunchAgents/x.plist", false),
            ("project/.git/hooks/pre-commit", false),
            ("Downloads/.hidden-script", false),
        ] {
            assert_refused(&fake, &fake.home.join(spelled), overwrite);
        }
        // Refused before anything is written: the files are as they were.
        assert_eq!(
            fs::read_to_string(fake.home.join(".bashrc")).unwrap(),
            "# rc\n"
        );
        assert!(!fake.home.join(".zshenv").exists());
    }

    #[test]
    fn a_route_around_the_rule_is_refused_as_the_place_it_reaches() {
        let fake = FakeHome::new();
        let home = &fake.home;
        // A dot-dot route: no component of the tail is hidden until `..` is folded.
        assert_refused(&fake, &home.join("Downloads/../.ssh/authorized_keys"), true);
        assert_refused(&fake, &home.join("project/results/../../.bashrc"), true);
        // A linked folder: `Downloads/keys/authorized_keys` names nothing hidden until the
        // link is resolved to `~/.ssh`.
        symlink(home.join(".ssh"), home.join("Downloads/keys")).unwrap();
        assert_refused(&fake, &home.join("Downloads/keys/authorized_keys"), true);
        symlink(
            home.join("Library/LaunchAgents"),
            home.join("Downloads/agents"),
        )
        .unwrap();
        assert_refused(&fake, &home.join("Downloads/agents/x.plist"), false);
        // A link named like an ordinary file, pointing at an rc file.
        symlink(home.join(".bashrc"), home.join("Downloads/notes.txt")).unwrap();
        assert_refused(&fake, &home.join("Downloads/notes.txt"), true);
        // A case variant: on APFS and NTFS this opens `~/Library`.
        assert_refused(&fake, &home.join("LIBRARY/LaunchAgents/x.plist"), false);
        // The home itself named through a link, and a link used as the home.
        symlink(home, fake.root.join("home-link")).unwrap();
        assert_refused(&fake, &fake.root.join("home-link/.bashrc"), true);
        let linked = SettingsLocations::new(vec![fake.root.join("home-link")], vec![], vec![]);
        assert_eq!(
            refusal(select_with(
                &home.join(".bashrc"),
                Direction::Download,
                true,
                &linked
            )),
            CredentialRefusal::Destination
        );
    }

    #[test]
    fn an_executable_is_never_replaced() {
        let fake = FakeHome::new();
        for (spelled, mode) in [
            ("bin/tool", 0o755),
            ("bin/owner-only", 0o744),
            ("Downloads/run.sh", 0o700),
            ("project/results/group-exec.py", 0o650),
        ] {
            let path = fake.home.join(spelled);
            fs::write(&path, "#!/bin/sh\necho hi\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            assert_refused(&fake, &path, true);
        }
        // The same place is fine once nothing there would be run.
        let plain = fake.home.join("bin/notes.txt");
        fs::write(&plain, "plain\n").unwrap();
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(fake.download(&plain, true).is_ok());
        assert!(fake
            .download(&fake.home.join("bin/new-file"), false)
            .is_ok());
    }

    #[test]
    fn ordinary_destinations_are_still_saved() {
        let fake = FakeHome::new();
        for (spelled, overwrite) in [
            ("Downloads/results.csv", false),
            ("Downloads/data.csv", true),
            ("project/results/plot.png", false),
            ("summary.md", false),
        ] {
            let path = fake.home.join(spelled);
            let selected = fake
                .download(&path, overwrite)
                .unwrap_or_else(|error| panic!("{spelled}: {error:#}"));
            assert_eq!(selected.name(), path.file_name().unwrap().to_str().unwrap());
            assert!(fake.replay(&path, overwrite).is_ok(), "replay {spelled}");
        }
        // The rule is about the home: a hidden folder elsewhere is not a settings location.
        let elsewhere = fake.root.join("shared/.cache-of-results");
        private_folder(&elsewhere);
        assert!(fake.download(&elsewhere.join("run.csv"), false).is_ok());
    }

    #[test]
    fn a_partial_in_a_refused_folder_can_still_be_removed() {
        let fake = FakeHome::new();
        let partial = fake.home.join(".config/autostart").join(part_name("t1"));
        fs::write(&partial, b"partial").unwrap();
        assert!(select_cleanup(&partial).is_ok());
    }

    #[test]
    fn the_refusal_keeps_the_destination_sentence() {
        let fake = FakeHome::new();
        let error = fake
            .download(&fake.home.join(".bashrc"), true)
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            CredentialRefusal::Destination.to_string()
        );
    }

    #[test]
    fn a_root_or_relative_home_is_not_a_home() {
        let locations = SettingsLocations::new(
            vec![PathBuf::from("/"), PathBuf::from("relative/home")],
            vec![PathBuf::from("relative/settings")],
            macos_cloud_drives(Path::new("relative/home")),
        );
        assert!(locations.homes.is_empty());
        assert!(locations.settings.is_empty());
        assert!(locations.drives.is_empty());
    }

    /// Q4-55 round 2: every macOS cloud drive lives inside `~/Library`, so rule (b) refused
    /// Box, OneDrive, Dropbox, Google Drive and iCloud Drive, a main place a UCSF user saves
    /// to. Inside a drive only rules (a) and (d) apply.
    #[test]
    fn cloud_drives_inside_library_are_still_saved() {
        let fake = FakeHome::new();
        let library = fake.home.join("Library");
        fs::write(library.join("CloudStorage/OneDrive-UCSF/plan.csv"), "old\n").unwrap();
        for (spelled, overwrite) in [
            ("CloudStorage/Box-Box/results.csv", false),
            ("CloudStorage/Box-Box/Lab data/plot.png", false),
            ("CloudStorage/OneDrive-UCSF/plan.csv", true),
            ("CloudStorage/Dropbox-LCATeam/notes.md", false),
            (
                "CloudStorage/GoogleDrive-tester@example.org/My Drive/run.csv",
                false,
            ),
            ("Mobile Documents/com~apple~CloudDocs/summary.md", false),
            (
                "Mobile Documents/com~apple~CloudDocs/Papers/draft.docx",
                false,
            ),
        ] {
            let path = library.join(spelled);
            let selected = fake
                .download(&path, overwrite)
                .unwrap_or_else(|error| panic!("{spelled}: {error:#}"));
            assert_eq!(selected.name(), path.file_name().unwrap().to_str().unwrap());
            assert!(fake.replay(&path, overwrite).is_ok(), "replay {spelled}");
        }
        // Case folded, as for `~/Library` itself.
        let folded = fake
            .root
            .join("home/library/cloudstorage/box-box/lab data/x.csv");
        assert!(!download_destination_denied(&folded, &fake.locations()));
    }

    #[test]
    fn the_rest_of_library_and_the_other_rules_still_hold_beside_a_cloud_drive() {
        let fake = FakeHome::new();
        let library = fake.home.join("Library");
        let run = library.join("CloudStorage/Box-Box/run.sh");
        fs::write(&run, "#!/bin/sh\necho hi\n").unwrap();
        fs::set_permissions(&run, fs::Permissions::from_mode(0o755)).unwrap();
        for (spelled, overwrite) in [
            // Beside the drives, not inside one.
            ("CloudStorage/x.plist", false),
            // An app's own iCloud container is not iCloud Drive.
            (
                "Mobile Documents/com~apple~Numbers/Documents/x.plist",
                false,
            ),
            ("Mobile Documents/x.plist", false),
            // Rule (a) inside a drive.
            ("CloudStorage/Dropbox-LCATeam/.dropbox.cache/x", false),
            // Rule (d) inside a drive.
            ("CloudStorage/Box-Box/run.sh", true),
            // A dot-dot route out of a drive.
            ("CloudStorage/Box-Box/../../LaunchAgents/x.plist", false),
        ] {
            assert_refused(&fake, &library.join(spelled), overwrite);
        }
        // A link inside a drive is judged as the place it reaches.
        symlink(
            library.join("LaunchAgents"),
            library.join("CloudStorage/Box-Box/agents"),
        )
        .unwrap();
        assert_refused(
            &fake,
            &library.join("CloudStorage/Box-Box/agents/x.plist"),
            false,
        );
        // And a drive reached through a link elsewhere is still a drive. (Only the rule is
        // asked: `open_directory` refuses a linked folder for a reason of its own.)
        symlink(library.join("CloudStorage/Box-Box"), fake.home.join("Box")).unwrap();
        assert!(!download_destination_denied(
            &fake.home.join("Box/results.csv"),
            &fake.locations()
        ));
        assert_eq!(fs::read_to_string(&run).unwrap(), "#!/bin/sh\necho hi\n");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_derives_library_from_home() {
        let fake = FakeHome::new();
        let derived = SettingsLocations::new(vec![fake.home.clone()], vec![], vec![]);
        assert_eq!(
            refusal(select_with(
                &fake.home.join("Library/LaunchAgents/x.plist"),
                Direction::Download,
                false,
                &derived
            )),
            CredentialRefusal::Destination
        );
        // And the cloud drives inside it.
        for spelled in [
            "Library/CloudStorage/Box-Box/results.csv",
            "Library/Mobile Documents/com~apple~CloudDocs/summary.md",
        ] {
            let path = fake.home.join(spelled);
            select_with(&path, Direction::Download, false, &derived)
                .unwrap_or_else(|error| panic!("{spelled}: {error:#}"));
        }
        assert_eq!(
            refusal(select_with(
                &fake.home.join("Library/CloudStorage/x.plist"),
                Direction::Download,
                false,
                &derived
            )),
            CredentialRefusal::Destination
        );
    }

    /// A macOS firmlink spells the same folder a second way that `realpath` keeps as written, so
    /// no string comparison sees the home in it; the device-and-inode walk does.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_firmlinked_spelling_of_the_home_is_refused() {
        let fake = FakeHome::new();
        let firmlinked =
            Path::new("/System/Volumes/Data").join(fake.home.strip_prefix("/").unwrap());
        if fs::metadata(&firmlinked).is_err() {
            eprintln!("no /System/Volumes/Data route to {}", fake.home.display());
            return;
        }
        assert_refused(&fake, &firmlinked.join(".bashrc"), true);
        assert_refused(
            &fake,
            &firmlinked.join("Library/LaunchAgents/x.plist"),
            false,
        );
        // A cloud drive through the firmlink is still a drive, and beside it is not.
        let box_drive = firmlinked.join("Library/CloudStorage/Box-Box/results.csv");
        assert!(fake.download(&box_drive, false).is_ok());
        assert_refused(
            &fake,
            &firmlinked.join("Library/CloudStorage/x.plist"),
            false,
        );
        // The walk on its own, whatever `realpath` made of the spelling.
        let locations = fake.locations();
        let drives = &locations.drives;
        let name = std::ffi::OsStr::new(".bashrc");
        assert!(denied_by_identity(&firmlinked, name, &locations, drives));
        assert!(denied_by_identity(
            &firmlinked.join("Library/LaunchAgents"),
            std::ffi::OsStr::new("x.plist"),
            &locations,
            drives
        ));
        assert!(!denied_by_identity(
            &firmlinked.join("Downloads"),
            std::ffi::OsStr::new("data.csv"),
            &locations,
            drives
        ));
        assert!(!denied_by_identity(
            &firmlinked.join("Library/CloudStorage/Box-Box"),
            std::ffi::OsStr::new("results.csv"),
            &locations,
            drives
        ));
        assert!(denied_by_identity(
            &firmlinked.join("Library/CloudStorage"),
            std::ffi::OsStr::new("x.plist"),
            &locations,
            drives
        ));
    }
}
