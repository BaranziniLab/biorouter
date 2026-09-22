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

pub fn select(path: &Path, direction: Direction, overwrite: bool) -> Result<Selection> {
    select_local(path, direction, overwrite, false)
}

pub fn select_cleanup(path: &Path) -> Result<Selection> {
    select_local(path, Direction::Download, false, true)
}

pub fn select_download_replay(path: &Path, overwrite: bool) -> Result<Selection> {
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
    let directory = open_directory(parent, false)?;
    match direction {
        Direction::Upload => {
            reject_link(path)?;
            let file = directory
                .open_with(&name, nofollow_options().read(true))?
                .into_std();
            reject_reparse(&file)?;
            let stamp = Stamp::read(&file)?;
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
