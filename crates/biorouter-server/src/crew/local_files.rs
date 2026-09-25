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
    /// A download destination.
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
                "Crew won't save into a credential location. Choose another folder.",
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

pub fn select(path: &Path, direction: Direction, overwrite: bool) -> Result<Selection> {
    if direction == Direction::Download && credential_floor_denies(path) {
        return Err(CredentialRefusal::Destination.into());
    }
    select_local(path, direction, overwrite, false)
}

/// Removing this transfer's own `.part` file is the one selection the floor does not judge: a
/// partial left in a folder the floor names (by a receipt older than the floor) must still be
/// removable.
pub fn select_cleanup(path: &Path) -> Result<Selection> {
    select_local(path, Direction::Download, false, true)
}

pub fn select_download_replay(path: &Path, overwrite: bool) -> Result<Selection> {
    if credential_floor_denies(path) {
        return Err(CredentialRefusal::Destination.into());
    }
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
            "Crew won't save into a credential location. Choose another folder."
        );
        // Removing a transfer's own partial is not judged: it must stay possible.
        assert!(select_cleanup(&home.home.join(".ssh/id_ecdsa")).is_ok());
        // An ordinary destination still works.
        let saved = select(&home.data.join("results.csv"), Direction::Download, false).unwrap();
        assert_eq!(saved.name(), "results.csv");
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
