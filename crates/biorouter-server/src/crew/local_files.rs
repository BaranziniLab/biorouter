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

/// How much of each end of a source file the content check reads. A credential store is
/// small, and a key added to a large file is appended far more often than spliced in, so the
/// check reads the first and the last this many bytes (FILES-F5); a file up to twice this
/// size is read whole. A key buried in the middle of a larger file is beyond this check, as it
/// is beyond the name check, and the manual says so.
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
    /// login or autostart location ([`download_destination_rule`]'s
    /// [`DestinationRule::SettingsLocation`]). One sentence covers both, so it names both. A
    /// refusal the person can fix by renaming (a dotted name, a program) is a
    /// [`SelectionRefusal`] instead, because this sentence's advice, another folder, cannot
    /// help with those (FILES-F3).
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
        Self::Source {
            name: printable_name(path),
        }
    }
}

/// The file name `path` ends in, made printable for a sentence. A path that names no file
/// reads as "This file".
fn printable_name(path: &Path) -> String {
    let raw = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = biorouter::utils::sanitize_untrusted_label(&raw, 255);
    if name.is_empty() {
        "This file".into()
    } else {
        name
    }
}

/// The code `POST /crew/files` answers [`SelectionRefusal::HiddenName`] with.
pub const FILE_NAME_HIDDEN_CODE: &str = "crew_file_name_hidden";
/// The code `POST /crew/files` answers [`SelectionRefusal::SharedFolder`] with.
pub const FOLDER_SHARED_CODE: &str = "crew_folder_shared";
/// The code `POST /crew/files` answers [`SelectionRefusal::Folder`] with.
pub const DESTINATION_IS_FOLDER_CODE: &str = "crew_destination_is_folder";
/// The code `POST /crew/files` answers [`SelectionRefusal::Exists`] with.
pub const DESTINATION_EXISTS_CODE: &str = "crew_destination_exists";
/// The code `POST /crew/files` answers [`SelectionRefusal::Program`] with.
pub const FILE_IS_PROGRAM_CODE: &str = "crew_file_is_program";

/// A download destination refused for a reason other than credentials, named so the person
/// knows what to change (FILES-F3, FILES-F9).
///
/// Every one of these used to reach the person as one of two sentences that could not help: a
/// peer's `.Rprofile` was "a credential or settings location. Choose another folder." (no
/// folder in the home would ever take that name), and a folder given as the file, or an
/// existing file without approval to replace it, both read "Destination exists; explicitly
/// approve replacement or select another filename" (approval can never help the first). The
/// refusals are unchanged; only the words and the code are the cause's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionRefusal {
    /// Q4-55 rule (a) where only the file's own name starts with a dot: renaming it is enough.
    HiddenName { name: String },
    /// The folder is not the person's alone: another account owns it, or may change it.
    SharedFolder,
    /// The destination is a folder, or is spelled as one (it ends in a separator). `path` is
    /// that folder, made printable.
    Folder { path: String },
    /// A file of that name exists, and replacing it was not approved.
    Exists { name: String },
    /// Q4-55 rule (d): the destination is a program, which a replacement would take over.
    Program { name: String },
}

impl SelectionRefusal {
    /// The code the transfer routes answer this refusal with.
    pub fn code(&self) -> &'static str {
        match self {
            Self::HiddenName { .. } => FILE_NAME_HIDDEN_CODE,
            Self::SharedFolder => FOLDER_SHARED_CODE,
            Self::Folder { .. } => DESTINATION_IS_FOLDER_CODE,
            Self::Exists { .. } => DESTINATION_EXISTS_CODE,
            Self::Program { .. } => FILE_IS_PROGRAM_CODE,
        }
    }

    fn folder(path: &Path) -> Self {
        let spelled = path.to_string_lossy();
        let trimmed = spelled.trim_end_matches(std::path::is_separator);
        let folder = if trimmed.is_empty() {
            spelled.as_ref()
        } else {
            trimmed
        };
        Self::Folder {
            path: biorouter::utils::sanitize_untrusted_label(folder, 1024),
        }
    }
}

impl std::fmt::Display for SelectionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HiddenName { name } => write!(
                f,
                "\u{201c}{name}\u{201d} starts with a dot, which Crew doesn't save into your home. Choose a name without the leading dot."
            ),
            Self::SharedFolder => f.write_str(
                "Choose a folder owned by your account that other accounts can't change.",
            ),
            Self::Folder { path } => write!(
                f,
                "That is a folder. Give a file name, for example {}.",
                Path::new(path).join("counts.csv").display()
            ),
            Self::Exists { name } => write!(
                f,
                "A file named \u{201c}{name}\u{201d} already exists. Replace it, or choose another name."
            ),
            Self::Program { name } => write!(
                f,
                "\u{201c}{name}\u{201d} is a program, and Crew won't replace one. Choose another name."
            ),
        }
    }
}

impl std::error::Error for SelectionRefusal {}

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

/// True when the first or the last [`CREDENTIAL_SNIFF_BYTES`] of `file` hold credential
/// material a credential file's *name* would not reveal: a hard-linked or renamed copy of a
/// private key, an AWS credentials file, a provider-key store, or a key pasted onto the end of
/// a large CSV or log (FILES-F5 uploaded a 78 KB CSV with a key at byte 78,000, when only the
/// first 64 KiB were read). A file that starts with a UTF-16 byte-order mark is decoded as
/// UTF-16 first, because read as UTF-8 its `-----BEGIN` line is not the bytes the detectors
/// match. Leaves the file positioned at its start.
fn holds_credential_material(file: &File) -> Result<bool> {
    let length = file.metadata()?.len();
    let mut reader = file;
    let mut read = |start: u64, bytes: u64| -> Result<Vec<u8>> {
        reader.seek(SeekFrom::Start(start))?;
        let mut buffer = Vec::new();
        (&mut reader).take(bytes).read_to_end(&mut buffer)?;
        Ok(buffer)
    };
    // One window when the ends meet, so a key across the middle of a small file is read whole.
    let windows = if length <= 2 * CREDENTIAL_SNIFF_BYTES {
        vec![(0, read(0, 2 * CREDENTIAL_SNIFF_BYTES)?)]
    } else {
        let tail = length - CREDENTIAL_SNIFF_BYTES;
        vec![
            (0, read(0, CREDENTIAL_SNIFF_BYTES)?),
            (tail, read(tail, CREDENTIAL_SNIFF_BYTES)?),
        ]
    };
    reader.seek(SeekFrom::Start(0))?;
    let encoding = windows
        .first()
        .map_or(TextEncoding::Utf8, |(_, head)| TextEncoding::sniff(head));
    Ok(windows.iter().any(|(start, bytes)| {
        biorouter::guardrails::secret_output::redact_text(&encoding.decode(*start, bytes)).is_some()
    }))
}

/// How the content check reads a file's bytes as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

impl TextEncoding {
    /// From the byte-order mark at the start of the file, if there is one.
    fn sniff(head: &[u8]) -> Self {
        match head {
            [0xFF, 0xFE, ..] => Self::Utf16Le,
            [0xFE, 0xFF, ..] => Self::Utf16Be,
            _ => Self::Utf8,
        }
    }

    /// `bytes`, read from byte `start` of the file, as text. A UTF-16 window that starts on an
    /// odd byte drops that byte so its code units line up with the file's.
    fn decode(self, start: u64, bytes: &[u8]) -> String {
        let pair = |bytes: &[u8]| -> Vec<u16> {
            bytes
                .chunks_exact(2)
                .map(|unit| match self {
                    Self::Utf16Be => u16::from_be_bytes([unit[0], unit[1]]),
                    _ => u16::from_le_bytes([unit[0], unit[1]]),
                })
                .collect()
        };
        match self {
            Self::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
            Self::Utf16Le | Self::Utf16Be => {
                let aligned = bytes.get(usize::from(start % 2 == 1)..).unwrap_or_default();
                String::from_utf16_lossy(&pair(aligned))
            }
        }
    }
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
#[cfg(test)]
fn download_destination_denied(path: &Path, locations: &SettingsLocations) -> bool {
    download_destination_rule(path, locations).is_some()
}

/// Which of the Q4-55 rules refuses a download destination, and so what the refusal says
/// (FILES-F3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DestinationRule {
    /// Rule (a) through a folder (`~/.ssh/…`, `~/.config/autostart/…`, `.git/…`), a dotted
    /// file reached by a link named like an ordinary one, or rule (b) or (c): a settings, login
    /// or autostart location, answered with [`CredentialRefusal::Destination`].
    SettingsLocation,
    /// Rule (a) where only the name the person gave starts with a dot (`~/.bashrc`,
    /// `~/Downloads/.Rprofile`): the same place under another name is fine.
    HiddenName,
    /// Rule (d).
    Program,
}

/// What rule (a) finds below a home: nothing, a dotted name only, or a dotted folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Hidden {
    No,
    NameOnly,
    Folder,
}

impl Hidden {
    /// Rule (a) over the components below a home, the file's own name last.
    fn of<S: AsRef<str>>(rest: &[S]) -> Self {
        let dotted = |part: &S| part.as_ref().starts_with('.');
        match rest.split_last() {
            Some((_, folders)) if folders.iter().any(dotted) => Self::Folder,
            Some((name, _)) if dotted(name) => Self::NameOnly,
            _ => Self::No,
        }
    }
}

/// [`download_destination_denied`]'s rules, with which one refused. A settings location
/// outranks a dotted name, and both outrank a program, so the sentence names the place when
/// the place is the problem.
fn download_destination_rule(
    path: &Path,
    locations: &SettingsLocations,
) -> Option<DestinationRule> {
    if !path.is_absolute() {
        return None;
    }
    let (Some(name), Some(parent)) = (path.file_name(), path.parent()) else {
        return None;
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
    let mut in_settings = false;
    let mut hidden = Hidden::No;
    for spelled in &spellings {
        for home in &homes {
            if let Some(rest) = below(spelled, home) {
                hidden = hidden.max(Hidden::of(&rest));
            }
        }
        in_settings |= in_settings_folder(spelled, &settings, &drives);
    }
    let (by_identity_settings, by_identity_hidden) =
        denied_by_identity(&folder, name, locations, &drives);
    in_settings |= by_identity_settings;
    hidden = hidden.max(by_identity_hidden);
    if in_settings || hidden == Hidden::Folder {
        return Some(DestinationRule::SettingsLocation);
    }
    if hidden == Hidden::NameOnly {
        // Only a name the person wrote with a dot is one they can rename. A link named like an
        // ordinary file that reaches `~/.bashrc` is that settings file under another name.
        return Some(if name.to_string_lossy().starts_with('.') {
            DestinationRule::HiddenName
        } else {
            DestinationRule::SettingsLocation
        });
    }
    names_an_executable(path).then_some(DestinationRule::Program)
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
///
/// Answers whether a settings folder was found, and what rule (a) found below a home.
#[cfg(unix)]
fn denied_by_identity(
    folder: &Path,
    name: &std::ffi::OsStr,
    locations: &SettingsLocations,
    drives: &[CloudDrive],
) -> (bool, Hidden) {
    use std::os::unix::fs::MetadataExt;
    let identity = |path: &Path| std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()));
    let homes: Vec<_> = locations.homes.iter().filter_map(|p| identity(p)).collect();
    let settings: Vec<_> = locations
        .settings
        .iter()
        .filter_map(|p| identity(p).map(|id| (id, p)))
        .collect();
    let mut in_settings = false;
    let mut hidden = Hidden::No;
    for ancestor in folder.ancestors() {
        let Some(this) = identity(ancestor) else {
            continue;
        };
        let rest = folder
            .strip_prefix(ancestor)
            .unwrap_or(Path::new(""))
            .join(name);
        in_settings |= settings
            .iter()
            .any(|(id, root)| *id == this && !in_cloud_drive(&root.join(&rest), drives));
        if homes.contains(&this) {
            let parts: Vec<String> = rest
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            hidden = hidden.max(Hidden::of(&parts));
        }
    }
    (in_settings, hidden)
}

#[cfg(not(unix))]
fn denied_by_identity(
    _: &Path,
    _: &std::ffi::OsStr,
    _: &SettingsLocations,
    _: &[CloudDrive],
) -> (bool, Hidden) {
    (false, Hidden::No)
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
/// floor (Q3-01) and the settings locations (Q4-55). A credential, settings, login or
/// autostart location is one refusal; a dotted name and a program each say what to rename
/// (FILES-F3).
fn refuse_destination(path: &Path, locations: &SettingsLocations) -> Result<()> {
    if credential_floor_denies(path) {
        return Err(CredentialRefusal::Destination.into());
    }
    match download_destination_rule(path, locations) {
        None => Ok(()),
        Some(DestinationRule::SettingsLocation) => Err(CredentialRefusal::Destination.into()),
        Some(DestinationRule::HiddenName) => Err(SelectionRefusal::HiddenName {
            name: printable_name(path),
        }
        .into()),
        Some(DestinationRule::Program) => Err(SelectionRefusal::Program {
            name: printable_name(path),
        }
        .into()),
    }
}

/// Whether `path` is spelled as a folder: it ends in a separator, or in `.` or `..`. `Path`
/// drops all three (`Path::new("trav/").file_name()` is `trav`), so a download given
/// `--output ~/Downloads/trav/` would otherwise be saved as a FILE named `trav` (FILES-F9).
fn spelled_as_folder(path: &Path) -> bool {
    let spelled = path.as_os_str().to_string_lossy();
    if spelled.ends_with(std::path::is_separator) {
        return true;
    }
    matches!(
        spelled.rsplit(std::path::is_separator).next(),
        Some("." | "..")
    )
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
    if direction == Direction::Download && !cleanup && spelled_as_folder(path) {
        return Err(SelectionRefusal::folder(path).into());
    }
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
            // The folder's own words first: `/tmp` and a shared lab folder are refused because
            // other accounts can change what lands there, and saying so is what helps.
            #[cfg(unix)]
            if !directory_is_private(&directory)? {
                return Err(SelectionRefusal::SharedFolder.into());
            }
            protected_directory(&directory)?;
            if !cleanup && directory.try_exists(&name)? {
                reject_link(path)?;
                let metadata = directory.metadata(&name)?;
                // Three causes that used to share one sentence (FILES-F9): approval to replace
                // can never help a folder, and a file only needs that approval.
                if metadata.is_dir() {
                    return Err(SelectionRefusal::folder(path).into());
                }
                ensure!(
                    metadata.is_file(),
                    "That is not an ordinary file, so Crew won't replace it. Choose another name."
                );
                if !overwrite {
                    return Err(SelectionRefusal::Exists {
                        name: printable_name(path),
                    }
                    .into());
                }
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

/// Whether `directory` is this account's alone: owned by it, writable by no group or other
/// account, and (macOS) granting no one an extended ACL allow. An error is a folder whose
/// ownership could not be read, which is not an answer either way.
#[cfg(unix)]
fn directory_is_private(directory: &Dir) -> Result<bool> {
    use cap_std::fs::MetadataExt;
    let metadata = directory.dir_metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
        return Ok(false);
    }
    #[cfg(target_os = "macos")]
    if acl_allows_anyone(&directory.try_clone()?.into_std_file())? {
        return Ok(false);
    }
    Ok(true)
}

pub fn protected_directory(directory: &Dir) -> Result<()> {
    #[cfg(unix)]
    ensure!(
        directory_is_private(directory)?,
        "Choose an output directory owned by your account that other accounts cannot modify"
    );
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
    ensure!(
        !acl_allows_anyone(file)?,
        "Choose a private output directory without extended ACL allow grants"
    );
    Ok(())
}

/// Whether `file`'s extended ACL holds any allow entry.
#[cfg(target_os = "macos")]
fn acl_allows_anyone(file: &File) -> Result<bool> {
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
            return Ok(false);
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
            if tag == 1 {
                return Ok(true);
            }
            selector = -1;
        }
        Ok(false)
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

    /// An ordinary CSV `bytes` long, then `tail` appended.
    fn csv_ending_with(bytes: usize, tail: &str) -> String {
        let mut csv = String::from("sample,od600\n");
        while csv.len() < bytes {
            csv.push_str("gina-1,0.42\n");
        }
        csv.truncate(bytes);
        csv.push_str(tail);
        csv
    }

    fn utf16(text: &str, big_endian: bool) -> Vec<u8> {
        let mut bytes = if big_endian {
            vec![0xFE, 0xFF]
        } else {
            vec![0xFF, 0xFE]
        };
        for unit in text.encode_utf16() {
            bytes.extend(if big_endian {
                unit.to_be_bytes()
            } else {
                unit.to_le_bytes()
            });
        }
        bytes
    }

    /// FILES-F5: only the first 64 KiB were read, so a 78,419-byte CSV with a private key at
    /// byte 78,000 uploaded, and so would any key appended to a large log or table. Both ends
    /// are read now, and a file small enough is read whole.
    #[test]
    fn a_key_at_the_end_of_a_large_file_is_refused_by_its_content() {
        let home = FakeHome::new();
        for (name, bytes) in [
            // The measured file: the key starts at byte 78,000.
            ("results-late-key.csv", 78_000),
            // Straddling the old 64 KiB edge.
            ("results-edge-key.csv", 65_500),
            // Far past both windows' meeting point: only the tail window sees it.
            ("results-appended-key.csv", 300_000),
        ] {
            let path = home.data.join(name);
            fs::write(&path, csv_ending_with(bytes, &fake_private_key())).unwrap();
            assert_eq!(
                refusal(upload(&path)),
                CredentialRefusal::Source { name: name.into() },
                "{name}"
            );
        }
        // An AWS key pasted onto the end of a large log, not a PEM block.
        let log = home.data.join("pipeline.log");
        fs::write(&log, csv_ending_with(500_000, &fake_aws_credentials())).unwrap();
        assert!(matches!(
            refusal(upload(&log)),
            CredentialRefusal::Source { .. }
        ));
        // A large file with nothing in it is still shared.
        let clean = home.data.join("big-clean.csv");
        fs::write(&clean, csv_ending_with(500_000, "gina-9,0.99\n")).unwrap();
        assert!(upload(&clean).is_ok());
    }

    /// FILES-F5's variant: a UTF-16 file's `-----BEGIN` line is not the bytes the detectors
    /// match when it is read as UTF-8, so a key saved as UTF-16 (Windows Notepad's "Unicode")
    /// passed. A file with a UTF-16 byte-order mark is decoded first, at either end.
    #[test]
    fn a_key_in_a_utf16_file_is_refused_by_its_content() {
        let home = FakeHome::new();
        for big_endian in [false, true] {
            let small = home.data.join(format!("notes-utf16-{big_endian}.txt"));
            fs::write(
                &small,
                utf16(&format!("exported\n{}", fake_private_key()), big_endian),
            )
            .unwrap();
            assert!(
                matches!(refusal(upload(&small)), CredentialRefusal::Source { .. }),
                "big endian: {big_endian}"
            );
            // At the end of a large file, where the tail window may start on an odd byte.
            for pad in [0, 1] {
                let large = home
                    .data
                    .join(format!("table-utf16-{big_endian}-{pad}.csv"));
                let mut text = csv_ending_with(200_000 + pad, "");
                text.push_str(&fake_aws_credentials());
                fs::write(&large, utf16(&text, big_endian)).unwrap();
                assert!(
                    matches!(refusal(upload(&large)), CredentialRefusal::Source { .. }),
                    "big endian: {big_endian}, pad: {pad}"
                );
            }
        }
        let clean = home.data.join("clean-utf16.csv");
        fs::write(&clean, utf16(&csv_ending_with(200_000, ""), false)).unwrap();
        assert!(upload(&clean).is_ok());
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

    fn selection_refusal(result: Result<Selection>) -> SelectionRefusal {
        let error = match result {
            Ok(selection) => panic!("expected a selection refusal for {}", selection.name()),
            Err(error) => error,
        };
        error
            .downcast_ref::<SelectionRefusal>()
            .cloned()
            .unwrap_or_else(|| panic!("expected a selection refusal, got: {error:#}"))
    }

    /// Refused by download and by replay alike, with the dotted-name sentence: the place is
    /// fine, the name is not (FILES-F3).
    fn assert_refused_as_hidden_name(fake: &FakeHome, path: &Path, overwrite: bool) {
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        for (door, result) in [
            ("download", fake.download(path, overwrite)),
            ("replay", fake.replay(path, overwrite)),
        ] {
            assert_eq!(
                selection_refusal(result),
                SelectionRefusal::HiddenName { name: name.clone() },
                "{door} {}",
                path.display()
            );
        }
    }

    /// Refused by download and by replay alike, with the program sentence.
    fn assert_refused_as_program(fake: &FakeHome, path: &Path, overwrite: bool) {
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        for (door, result) in [
            ("download", fake.download(path, overwrite)),
            ("replay", fake.replay(path, overwrite)),
        ] {
            assert_eq!(
                selection_refusal(result),
                SelectionRefusal::Program { name: name.clone() },
                "{door} {}",
                path.display()
            );
        }
    }

    #[test]
    fn login_settings_and_autostart_files_are_refused() {
        let fake = FakeHome::new();
        // Inside a settings folder: another folder is the fix.
        for (spelled, overwrite) in [
            (".ssh/authorized_keys", true),
            (".ssh/authorized_keys", false),
            (".ssh/config", true),
            (".ssh/rc", false),
            (".config/autostart/sync.desktop", false),
            (".config/systemd/user/sync.service", false),
            (".local/bin/python3", false),
            ("Library/LaunchAgents/x.plist", false),
            ("project/.git/hooks/pre-commit", false),
        ] {
            assert_refused(&fake, &fake.home.join(spelled), overwrite);
        }
        // A dotted file name in an ordinary folder of the home: another name is the fix, and
        // the refusal says so (FILES-F3).
        for (spelled, overwrite) in [
            (".bashrc", true),
            (".zshrc", true),
            (".zshenv", false),
            (".profile", true),
            (".gitconfig", true),
            ("Downloads/.hidden-script", false),
            ("Downloads/.Rprofile", false),
        ] {
            assert_refused_as_hidden_name(&fake, &fake.home.join(spelled), overwrite);
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
        assert_refused_as_hidden_name(&fake, &home.join("project/results/../../.bashrc"), true);
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
        // A link named like an ordinary file, pointing at an rc file: renaming cannot help, so
        // it is refused as the settings file it reaches, not as a dotted name.
        symlink(home.join(".bashrc"), home.join("Downloads/notes.txt")).unwrap();
        assert_refused(&fake, &home.join("Downloads/notes.txt"), true);
        // A case variant: on APFS and NTFS this opens `~/Library`.
        assert_refused(&fake, &home.join("LIBRARY/LaunchAgents/x.plist"), false);
        // The home itself named through a link, and a link used as the home.
        symlink(home, fake.root.join("home-link")).unwrap();
        assert_refused_as_hidden_name(&fake, &fake.root.join("home-link/.bashrc"), true);
        let linked = SettingsLocations::new(vec![fake.root.join("home-link")], vec![], vec![]);
        assert_eq!(
            selection_refusal(select_with(
                &home.join(".bashrc"),
                Direction::Download,
                true,
                &linked
            )),
            SelectionRefusal::HiddenName {
                name: ".bashrc".into()
            }
        );
        assert_eq!(
            refusal(select_with(
                &home.join(".ssh/authorized_keys"),
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
            assert_refused_as_program(&fake, &path, true);
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

    /// FILES-F9: a folder given as the file, and a file that exists, used to share one sentence
    /// telling the person to approve a replacement, which can never help a folder; a trailing
    /// separator even slipped past the "not a directory" check. FILES-F3: a folder other
    /// accounts can change was refused in words that reached the person as a generic refusal.
    #[test]
    fn a_folder_an_existing_file_and_a_shared_folder_each_say_so() {
        let fake = FakeHome::new();
        let downloads = fake.home.join("Downloads");
        for overwrite in [false, true] {
            assert_eq!(
                selection_refusal(fake.download(&downloads, overwrite)),
                SelectionRefusal::Folder {
                    path: downloads.to_string_lossy().into_owned()
                },
                "overwrite: {overwrite}"
            );
        }
        let with_separator =
            |p: &Path| PathBuf::from(format!("{}{}", p.display(), std::path::MAIN_SEPARATOR));
        for spelled in [
            with_separator(&downloads),
            // Spelled as a folder that does not exist yet: saving it as a FILE named
            // `new-folder` is not what the person asked for.
            with_separator(&downloads.join("new-folder")),
            downloads.join("."),
            downloads.join("project").join(".."),
        ] {
            assert!(
                matches!(
                    selection_refusal(fake.download(&spelled, true)),
                    SelectionRefusal::Folder { .. }
                ),
                "{}",
                spelled.display()
            );
            assert!(!downloads.join("new-folder").exists());
        }
        assert_eq!(
            selection_refusal(fake.download(&downloads.join("data.csv"), false)),
            SelectionRefusal::Exists {
                name: "data.csv".into()
            }
        );
        assert!(fake.download(&downloads.join("data.csv"), true).is_ok());

        for mode in [0o777, 0o775, 0o757] {
            let shared = fake.root.join(format!("shared-{mode:o}"));
            fs::create_dir(&shared).unwrap();
            fs::set_permissions(&shared, fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                selection_refusal(fake.download(&shared.join("counts.csv"), false)),
                SelectionRefusal::SharedFolder,
                "mode {mode:o}"
            );
        }
        // Removing a transfer's own partial is judged by the folder check it always had.
        let private = fake.root.join("private-results");
        private_folder(&private);
        assert!(fake.download(&private.join("counts.csv"), false).is_ok());
    }

    #[test]
    fn a_partial_in_a_refused_folder_can_still_be_removed() {
        let fake = FakeHome::new();
        let partial = fake.home.join(".config/autostart").join(part_name("t1"));
        fs::write(&partial, b"partial").unwrap();
        assert!(select_cleanup(&partial).is_ok());
    }

    #[test]
    fn each_refusal_names_its_own_cause() {
        let fake = FakeHome::new();
        let settings = fake
            .download(&fake.home.join(".config/autostart/sync.desktop"), false)
            .err()
            .unwrap();
        assert_eq!(
            settings.to_string(),
            "Crew won't save into a credential or settings location. Choose another folder."
        );
        // FILES-F3: a peer's `.Rprofile` could never be saved in any folder of the home, and the
        // old advice, another folder, could not help.
        let dotted = fake
            .download(&fake.home.join("Downloads/.Rprofile"), false)
            .err()
            .unwrap();
        assert_eq!(
            dotted.to_string(),
            "\u{201c}.Rprofile\u{201d} starts with a dot, which Crew doesn't save into your home. Choose a name without the leading dot."
        );
        let run = fake.home.join("bin/run");
        fs::write(&run, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&run, fs::Permissions::from_mode(0o755)).unwrap();
        let program = fake.download(&run, true).err().unwrap();
        assert_eq!(
            program.to_string(),
            "\u{201c}run\u{201d} is a program, and Crew won't replace one. Choose another name."
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
            // A dot-dot route out of a drive.
            ("CloudStorage/Box-Box/../../LaunchAgents/x.plist", false),
        ] {
            assert_refused(&fake, &library.join(spelled), overwrite);
        }
        // Rule (a) for a dotted name, and rule (d), inside a drive.
        assert_refused_as_hidden_name(
            &fake,
            &library.join("CloudStorage/Box-Box/.Rprofile"),
            false,
        );
        assert_refused_as_program(&fake, &library.join("CloudStorage/Box-Box/run.sh"), true);
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
        assert_refused_as_hidden_name(&fake, &firmlinked.join(".bashrc"), true);
        assert_refused(&fake, &firmlinked.join(".ssh/authorized_keys"), true);
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
        assert_eq!(
            denied_by_identity(&firmlinked, name, &locations, drives),
            (false, Hidden::NameOnly)
        );
        assert_eq!(
            denied_by_identity(
                &firmlinked.join(".ssh"),
                std::ffi::OsStr::new("authorized_keys"),
                &locations,
                drives
            ),
            (false, Hidden::Folder)
        );
        assert!(
            denied_by_identity(
                &firmlinked.join("Library/LaunchAgents"),
                std::ffi::OsStr::new("x.plist"),
                &locations,
                drives
            )
            .0
        );
        assert_eq!(
            denied_by_identity(
                &firmlinked.join("Downloads"),
                std::ffi::OsStr::new("data.csv"),
                &locations,
                drives
            ),
            (false, Hidden::No)
        );
        assert_eq!(
            denied_by_identity(
                &firmlinked.join("Library/CloudStorage/Box-Box"),
                std::ffi::OsStr::new("results.csv"),
                &locations,
                drives
            ),
            (false, Hidden::No)
        );
        assert!(
            denied_by_identity(
                &firmlinked.join("Library/CloudStorage"),
                std::ffi::OsStr::new("x.plist"),
                &locations,
                drives
            )
            .0
        );
    }
}
