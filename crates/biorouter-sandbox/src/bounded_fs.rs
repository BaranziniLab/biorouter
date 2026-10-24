//! Directory-object authority for host-side file operations.
//!
//! Pathnames returned by this module are labels only. Actual I/O stays relative
//! to captured directory handles, including after their original names change.
//! An authorized directory remains that object after a rename, including its
//! captured child directories. This is not isolation from arbitrary host
//! commands or from another process that already has access to those objects.
//! Hard links are readable when present inside an authorized root; writes replace
//! the authorized directory entry and do not edit another hard-link alias.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fs::{File, Metadata};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const MAX_SYMLINKS: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    Read,
    Write,
}

impl AccessMode {
    pub fn permits(self, requested: Self) -> bool {
        self == Self::Write || requested == Self::Read
    }
}

#[derive(Clone, Debug)]
pub struct BoundedFs {
    inner: Arc<Root>,
}

#[derive(Debug)]
struct Root {
    directory: Dir,
    canonical: PathBuf,
    selected: PathBuf,
    identity: RootIdentity,
    access: AccessMode,
    authorization: Option<Authorization>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootIdentity {
    pub device: u64,
    pub file: u64,
    pub created_nanos: Option<u128>,
}

#[derive(Clone, Debug)]
pub struct BoundedPath {
    filesystem: BoundedFs,
    parent: Arc<Dir>,
    missing_parents: Vec<OsString>,
    name: OsString,
    relative: PathBuf,
    access: AccessMode,
    authorization: Option<Authorization>,
    allow_subdirectory: bool,
}

#[derive(Clone, Debug)]
struct Authorization {
    live: Vec<Arc<AtomicBool>>,
}

impl Authorization {
    fn check(&self) -> io::Result<()> {
        if self.live.iter().all(|live| live.load(Ordering::Acquire)) {
            Ok(())
        } else {
            Err(refused(
                "This file authorization has expired or was revoked",
            ))
        }
    }
}

#[derive(Clone, Debug)]
pub struct BoundedDirEntry {
    pub name: OsString,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub len: u64,
}

fn refused(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message.into())
}

fn nofollow_options(directory: bool) -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true);
    options.follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        let flags = libc::O_NOFOLLOW | libc::O_NONBLOCK;
        options.custom_flags(if directory {
            flags | libc::O_DIRECTORY
        } else {
            flags
        });
    }
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        options
            .custom_flags(0x0020_0000 | 0x0200_0000)
            .share_mode(if directory { 3 } else { 7 });
    }
    options
}

fn private_options(directory: bool) -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = nofollow_options(directory);
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, WRITE_DAC};
        options.access_mode(FILE_GENERIC_READ | WRITE_DAC);
    }
    options
}

fn private_directory_at(directory: &Dir, name: &Path) -> io::Result<Dir> {
    #[allow(unused_mut)]
    let mut builder = cap_std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match directory.create_dir_with(name, &builder) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let file = directory
        .open_with(name, &private_options(true))?
        .into_std();
    let metadata = file.metadata()?;
    reject_reparse(&metadata)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "Private storage must be a directory",
        ));
    }
    crate::owner_private::protect(&file, true)?;
    Ok(Dir::from_std_file(file))
}

fn reject_reparse(metadata: &Metadata) -> io::Result<()> {
    if metadata.file_type().is_symlink() {
        return Err(refused("The selected path changed to a symbolic link"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(refused("The selected path changed to a reparse point"));
        }
    }
    Ok(())
}

fn open_directory_at(directory: &Dir, name: &Path) -> io::Result<Dir> {
    let file = directory
        .open_with(name, &nofollow_options(true))?
        .into_std();
    let metadata = file.metadata()?;
    reject_reparse(&metadata)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "A parent path is not a directory",
        ));
    }
    Ok(Dir::from_std_file(file))
}

fn open_root(path: &Path) -> io::Result<Dir> {
    #[cfg(not(any(unix, windows)))]
    return Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Bounded filesystem access requires Unix or Windows directory handles",
    ));
    #[cfg(any(unix, windows))]
    {
        let mut prefix = PathBuf::new();
        let mut names = Vec::new();
        for component in path.components() {
            match component {
                Component::Prefix(value) => prefix.push(value.as_os_str()),
                Component::RootDir => prefix.push(component.as_os_str()),
                Component::Normal(name) => names.push(name),
                _ => return Err(refused("An absolute normalized root is required")),
            }
        }
        let mut directory = Dir::open_ambient_dir(prefix, cap_std::ambient_authority())?;
        for name in names {
            directory = open_directory_at(&directory, Path::new(name))?;
        }
        Ok(directory)
    }
}

fn directory_identity(directory: &Dir) -> io::Result<RootIdentity> {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        let metadata = directory.dir_metadata()?;
        let created_nanos = directory
            .try_clone()?
            .into_std_file()
            .metadata()?
            .created()
            .ok()
            .and_then(|created| created.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|created| created.as_nanos());
        Ok(RootIdentity {
            device: metadata.dev(),
            file: metadata.ino(),
            created_nanos,
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let file = directory.try_clone()?.into_std_file();
        let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        let information = unsafe { information.assume_init() };
        Ok(RootIdentity {
            device: u64::from(information.dwVolumeSerialNumber),
            file: (u64::from(information.nFileIndexHigh) << 32)
                | u64::from(information.nFileIndexLow),
            created_nanos: Some(
                u128::from(
                    (u64::from(information.ftCreationTime.dwHighDateTime) << 32)
                        | u64::from(information.ftCreationTime.dwLowDateTime),
                ) * 100,
            ),
        })
    }
    #[cfg(not(any(unix, windows)))]
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Directory identity is unavailable",
    ))
}

#[cfg(unix)]
fn copy_ownership(source: &File, destination: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;
    let source = source.metadata()?;
    let current = destination.metadata()?;
    if source.uid() == current.uid() && source.gid() == current.gid() {
        return Ok(());
    }
    if unsafe { libc::fchown(destination.as_raw_fd(), source.uid(), source.gid()) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn copy_ownership(source: &File, destination: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        EqualSid, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    };
    let mut source_owner = std::ptr::null_mut();
    let mut source_group = std::ptr::null_mut();
    let mut source_descriptor = std::ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            source.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION,
            &mut source_owner,
            &mut source_group,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut source_descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let mut owner = std::ptr::null_mut();
    let mut group = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            destination.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION,
            &mut owner,
            &mut group,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    let matches = status == 0
        && unsafe { EqualSid(source_owner, owner) != 0 && EqualSid(source_group, group) != 0 };
    unsafe {
        LocalFree(source_descriptor);
        if !descriptor.is_null() {
            LocalFree(descriptor);
        }
    }
    if status != 0 {
        Err(io::Error::from_raw_os_error(status as i32))
    } else if matches {
        Ok(())
    } else {
        Err(refused(
            "Atomic replacement cannot preserve this file's owner and group",
        ))
    }
}

#[cfg(not(any(unix, windows)))]
fn copy_ownership(_source: &File, _destination: &File) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Preserving file ownership is unavailable",
    ))
}

#[cfg(windows)]
fn normalized_component(directory: &Dir, name: OsString) -> io::Result<OsString> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let file = directory
        .open_with(&name, &nofollow_options(false))?
        .into_std();
    reject_reparse(&file.metadata()?)?;
    let required =
        unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), std::ptr::null_mut(), 0, 0) };
    if required == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0_u16; required as usize + 1];
    let written = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    };
    if written == 0 || written as usize >= buffer.len() {
        return Err(io::Error::last_os_error());
    }
    let path = PathBuf::from(OsString::from_wide(&buffer[..written as usize]));
    path.file_name()
        .map(OsString::from)
        .ok_or_else(|| refused("The actual file name is unavailable"))
}

#[cfg(not(windows))]
fn normalized_component(_directory: &Dir, name: OsString) -> io::Result<OsString> {
    Ok(name)
}

#[cfg(target_os = "macos")]
fn copy_access_controls(source: &File, destination: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    if unsafe {
        libc::fcopyfile(
            source.as_raw_fd(),
            destination.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_ACL,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn copy_access_controls(source: &File, destination: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let name = c"system.posix_acl_access";
    let size =
        unsafe { libc::fgetxattr(source.as_raw_fd(), name.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        let error = io::Error::last_os_error();
        return match error.raw_os_error() {
            Some(libc::ENODATA | libc::ENOTSUP) => {
                let result = unsafe { libc::fremovexattr(destination.as_raw_fd(), name.as_ptr()) };
                if result == 0 {
                    Ok(())
                } else {
                    let error = io::Error::last_os_error();
                    match error.raw_os_error() {
                        Some(libc::ENODATA | libc::ENOTSUP) => Ok(()),
                        _ => Err(error),
                    }
                }
            }
            _ => Err(error),
        };
    }
    let mut value = vec![0_u8; size as usize];
    let read = unsafe {
        libc::fgetxattr(
            source.as_raw_fd(),
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            value.len(),
        )
    };
    if read < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe {
        libc::fsetxattr(
            destination.as_raw_fd(),
            name.as_ptr(),
            value.as_ptr().cast(),
            read as usize,
            0,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn copy_access_controls(source: &File, destination: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        GetSecurityInfo, SetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    let mut dacl = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    let error = unsafe {
        GetSecurityInfo(
            source.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    // Protect the copied effective ACL from inheriting extra destination access.
    let error = unsafe {
        SetSecurityInfo(
            destination.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            dacl,
            std::ptr::null_mut(),
        )
    };
    unsafe {
        LocalFree(descriptor);
    }
    if error == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(error as i32))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn copy_access_controls(_source: &File, _destination: &File) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Preserving file access controls is unavailable",
    ))
}

fn relative_components(path: &Path, allow_parent: bool) -> io::Result<Vec<OsString>> {
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                #[cfg(windows)]
                if name.to_string_lossy().contains(':')
                    || name.to_string_lossy().ends_with(['.', ' '])
                {
                    return Err(refused("Alternate streams and normalized path aliases are not authorized file paths"));
                }
                names.push(name.to_os_string());
            }
            Component::CurDir => {}
            Component::ParentDir if allow_parent && !names.is_empty() => {
                names.pop();
            }
            _ => {
                return Err(refused(
                    "The requested path escapes its authorized directory",
                ))
            }
        }
    }
    Ok(names)
}

impl BoundedFs {
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let selected = if root.as_ref().is_absolute() {
            root.as_ref().to_path_buf()
        } else {
            std::env::current_dir()?.join(root)
        };
        let canonical = selected.canonicalize()?;
        let directory = open_root(&canonical)?;
        let identity = directory_identity(&directory)?;
        Ok(Self {
            inner: Arc::new(Root {
                directory,
                canonical,
                selected,
                identity,
                access: AccessMode::Write,
                authorization: None,
            }),
        })
    }

    pub fn root(&self) -> &Path {
        &self.inner.canonical
    }

    pub fn identity(&self) -> RootIdentity {
        self.inner.identity
    }

    pub fn subdirectory(&self, relative: impl AsRef<Path>) -> io::Result<Self> {
        self.bind(relative, self.inner.access)?.open_dir()
    }

    pub fn protect_owner_private(&self) -> io::Result<()> {
        self.bind(".", AccessMode::Write)?
            .check(AccessMode::Write)?;
        let file = self
            .inner
            .directory
            .open_with(".", &private_options(true))?
            .into_std();
        crate::owner_private::protect(&file, true)
    }

    pub fn create_private_subdirectory(&self, relative: impl AsRef<Path>) -> io::Result<Self> {
        let names = relative_components(relative.as_ref(), false)?;
        if names.is_empty() {
            return Err(refused("A private subdirectory name is required"));
        }
        let mut directory = self.inner.directory.try_clone()?;
        let mut label = self.root().to_path_buf();
        for name in names {
            self.bind(".", AccessMode::Write)?
                .check(AccessMode::Write)?;
            directory = private_directory_at(&directory, Path::new(&name))?;
            label.push(name);
        }
        let identity = directory_identity(&directory)?;
        Ok(Self {
            inner: Arc::new(Root {
                directory,
                canonical: label.clone(),
                selected: label,
                identity,
                access: self.inner.access,
                authorization: self.inner.authorization.clone(),
            }),
        })
    }

    fn relative_request(&self, path: &Path) -> io::Result<Vec<OsString>> {
        let relative = if path.is_absolute() {
            path.strip_prefix(&self.inner.canonical)
                .or_else(|_| path.strip_prefix(&self.inner.selected))
                .map_err(|_| refused("The requested path is outside its authorized directory"))?
        } else {
            path
        };
        relative_components(relative, false)
    }

    fn link_target(&self, parent: &Path, target: &Path) -> io::Result<Vec<OsString>> {
        let relative = if target.is_absolute() {
            target
                .strip_prefix(&self.inner.canonical)
                .or_else(|_| target.strip_prefix(&self.inner.selected))
                .map_err(|_| refused("A symbolic link points outside its authorized directory"))?
                .to_path_buf()
        } else {
            parent.join(target)
        };
        relative_components(&relative, true)
    }

    pub fn bind(&self, path: impl AsRef<Path>, access: AccessMode) -> io::Result<BoundedPath> {
        if let Some(authorization) = &self.inner.authorization {
            authorization.check()?;
        }
        if !self.inner.access.permits(access) {
            return Err(refused("This directory authorization is read-only"));
        }
        let mut names = self.relative_request(path.as_ref())?;
        let mut directory = self.inner.directory.try_clone()?;
        let mut resolved = PathBuf::new();
        let mut index = 0;
        let mut links = 0;
        let mut required_existing: usize = 0;
        while let Some(name) = names.get(index) {
            let name = name.clone();
            let metadata = match directory.symlink_metadata(&name) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if index < required_existing {
                        return Err(refused("A symbolic link target no longer exists"));
                    }
                    let tail = &names[index..];
                    resolved.extend(tail);
                    return Ok(BoundedPath {
                        filesystem: self.clone(),
                        parent: Arc::new(directory),
                        missing_parents: tail[..tail.len() - 1].to_vec(),
                        name: tail[tail.len() - 1].clone(),
                        relative: resolved,
                        access,
                        authorization: self.inner.authorization.clone(),
                        allow_subdirectory: true,
                    });
                }
                Err(error) => return Err(error),
            };
            if metadata.file_type().is_symlink() {
                links += 1;
                if links > MAX_SYMLINKS {
                    return Err(refused("Too many symbolic links in the requested path"));
                }
                let target = directory.read_link_contents(&name)?;
                let mut target = self.link_target(&resolved, &target)?;
                required_existing = target.len() + required_existing.saturating_sub(index + 1);
                target.extend_from_slice(&names[index + 1..]);
                names = target;
                directory = self.inner.directory.try_clone()?;
                resolved.clear();
                index = 0;
                continue;
            }
            let name = normalized_component(&directory, name)?;
            if index + 1 == names.len() {
                resolved.push(&name);
                return Ok(BoundedPath {
                    filesystem: self.clone(),
                    parent: Arc::new(directory),
                    missing_parents: Vec::new(),
                    name,
                    relative: resolved,
                    access,
                    authorization: self.inner.authorization.clone(),
                    allow_subdirectory: true,
                });
            }
            directory = open_directory_at(&directory, Path::new(&name))?;
            resolved.push(&name);
            index += 1;
        }
        Ok(BoundedPath {
            filesystem: self.clone(),
            parent: Arc::new(directory),
            missing_parents: Vec::new(),
            name: OsString::from("."),
            relative: resolved,
            access,
            authorization: self.inner.authorization.clone(),
            allow_subdirectory: true,
        })
    }
}

impl BoundedPath {
    pub fn root(&self) -> &Path {
        self.filesystem.root()
    }

    pub fn history_key(&self) -> PathBuf {
        let identity = self.filesystem.identity();
        PathBuf::from(format!(
            "root-{:x}-{:x}-{:x}",
            identity.device,
            identity.file,
            identity.created_nanos.unwrap_or(0)
        ))
        .join(&self.relative)
    }

    pub fn try_history_key(&self) -> io::Result<PathBuf> {
        self.check(AccessMode::Read)?;
        let identity = directory_identity(&self.parent(false)?)?;
        Ok(PathBuf::from(format!(
            "parent-{:x}-{:x}-{:x}",
            identity.device,
            identity.file,
            identity.created_nanos.unwrap_or(0)
        ))
        .join(&self.name))
    }

    pub fn protect_owner_private(&self) -> io::Result<()> {
        self.check(AccessMode::Write)?;
        let file = self
            .parent(false)?
            .open_with(&self.name, &private_options(false))?
            .into_std();
        let metadata = file.metadata()?;
        reject_reparse(&metadata)?;
        if !metadata.is_file() {
            return Err(refused("Private file storage must be a regular file"));
        }
        crate::owner_private::protect(&file, false)
    }

    pub fn open_dir(&self) -> io::Result<BoundedFs> {
        self.check(AccessMode::Read)?;
        if !self.allow_subdirectory {
            return Err(refused(
                "An exact-target authorization cannot grant its descendants",
            ));
        }
        let directory = open_directory_at(&self.parent(false)?, Path::new(&self.name))?;
        let identity = directory_identity(&directory)?;
        Ok(BoundedFs {
            inner: Arc::new(Root {
                directory,
                canonical: self.absolute_path(),
                selected: self.absolute_path(),
                identity,
                access: self.access,
                authorization: self.authorization.clone(),
            }),
        })
    }

    pub fn relative_path(&self) -> &Path {
        &self.relative
    }

    pub fn absolute_path(&self) -> PathBuf {
        self.filesystem.root().join(&self.relative)
    }

    pub fn read_root_policy(&self) -> io::Result<Vec<u8>> {
        self.check(AccessMode::Read)?;
        let mut policy = self.filesystem.bind(".biorouterignore", AccessMode::Read)?;
        policy.authorization = self.authorization.clone();
        let bytes = policy.read_limited(1024 * 1024)?;
        self.check(AccessMode::Read)?;
        Ok(bytes)
    }

    fn check(&self, access: AccessMode) -> io::Result<()> {
        if !self.access.permits(access) {
            return Err(refused("This file authorization is read-only"));
        }
        if let Some(authorization) = &self.authorization {
            authorization.check()?;
        }
        Ok(())
    }

    fn parent(&self, create: bool) -> io::Result<Dir> {
        let mut directory = self.parent.try_clone()?;
        for name in &self.missing_parents {
            self.check(if create {
                AccessMode::Write
            } else {
                AccessMode::Read
            })?;
            if create {
                match directory.create_dir(name) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            directory = open_directory_at(&directory, Path::new(name))?;
        }
        Ok(directory)
    }

    fn open(&self) -> io::Result<File> {
        self.check(AccessMode::Read)?;
        let file = self
            .parent(false)?
            .open_with(&self.name, &nofollow_options(false))?
            .into_std();
        reject_reparse(&file.metadata()?)?;
        Ok(file)
    }

    pub fn metadata(&self) -> io::Result<Metadata> {
        self.open()?.metadata()
    }

    pub fn exists(&self) -> io::Result<bool> {
        match self.metadata() {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn is_file(&self) -> io::Result<bool> {
        match self.metadata() {
            Ok(metadata) => Ok(metadata.is_file()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn is_dir(&self) -> io::Result<bool> {
        match self.metadata() {
            Ok(metadata) => Ok(metadata.is_dir()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn read(&self) -> io::Result<Vec<u8>> {
        let mut file = self.open()?;
        if !file.metadata()?.is_file() {
            return Err(refused("Only regular files can be read"));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    pub fn read_to_string(&self) -> io::Result<String> {
        String::from_utf8(self.read()?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub fn read_limited(&self, max_bytes: u64) -> io::Result<Vec<u8>> {
        let file = self.open()?;
        if !file.metadata()?.is_file() {
            return Err(refused("Only regular files can be read"));
        }
        let mut bytes = Vec::new();
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "The file exceeds its read limit",
            ));
        }
        Ok(bytes)
    }

    pub fn read_to_string_limited(&self, max_bytes: u64) -> io::Result<String> {
        String::from_utf8(self.read_limited(max_bytes)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub fn list(&self) -> io::Result<Vec<BoundedDirEntry>> {
        self.check(AccessMode::Read)?;
        let directory = open_directory_at(&self.parent(false)?, Path::new(&self.name))?;
        directory
            .entries()?
            .map(|entry| {
                let entry = entry?;
                let metadata = directory.symlink_metadata(entry.file_name())?;
                Ok(BoundedDirEntry {
                    name: entry.file_name(),
                    is_dir: metadata.is_dir(),
                    is_symlink: metadata.file_type().is_symlink(),
                    len: metadata.len(),
                })
            })
            .collect()
    }

    pub fn atomic_write(&self, bytes: &[u8]) -> io::Result<()> {
        self.atomic_write_inner(bytes, false)
    }

    pub fn atomic_write_private(&self, bytes: &[u8]) -> io::Result<()> {
        self.atomic_write_inner(bytes, true)
    }

    fn atomic_write_inner(&self, bytes: &[u8], private: bool) -> io::Result<()> {
        self.check(AccessMode::Write)?;
        if self.relative.as_os_str().is_empty() {
            return Err(refused("The authorized root is a directory"));
        }
        let parent = if private {
            let mut directory = self.parent.try_clone()?;
            for name in &self.missing_parents {
                self.check(AccessMode::Write)?;
                directory = private_directory_at(&directory, Path::new(name))?;
            }
            directory
        } else {
            self.parent(true)?
        };
        let previous_options = if private {
            private_options(false)
        } else {
            nofollow_options(false)
        };
        let previous = match parent.open_with(&self.name, &previous_options) {
            Ok(file) => {
                let file = file.into_std();
                let metadata = file.metadata()?;
                reject_reparse(&metadata)?;
                if !metadata.is_file() || metadata.permissions().readonly() {
                    return Err(refused("The destination is not a writable regular file"));
                }
                if private {
                    crate::owner_private::protect(&file, false)?;
                }
                Some((file, metadata.permissions()))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let temporary = OsString::from(format!(".biorouter-write-{}", Uuid::new_v4()));
        #[cfg(target_os = "macos")]
        let staging = private_directory_at(&parent, Path::new(&temporary))?;
        #[cfg(target_os = "macos")]
        let (temporary_parent, temporary_file) = (&staging, OsString::from("content"));
        #[cfg(not(target_os = "macos"))]
        let (temporary_parent, temporary_file) = (&parent, temporary.clone());
        let mut options = nofollow_options(false);
        options.read(false).write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(if private || previous.is_some() {
                0o600
            } else {
                0o666
            });
        }
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_GENERIC_WRITE, READ_CONTROL, WRITE_DAC,
            };
            options
                .access_mode(FILE_GENERIC_WRITE | READ_CONTROL | WRITE_DAC)
                .share_mode(4);
        }
        let mut file = temporary_parent
            .open_with(&temporary_file, &options)?
            .into_std();
        let result = (|| {
            if private {
                crate::owner_private::protect(&file, false)?;
            } else if let Some((original, permissions)) = previous {
                copy_ownership(&original, &file)?;
                file.set_permissions(permissions)?;
                copy_access_controls(&original, &file)?;
            }
            file.write_all(bytes)?;
            file.sync_all()?;
            self.check(AccessMode::Write)?;
            temporary_parent.rename(&temporary_file, &parent, &self.name)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = temporary_parent.remove_file(&temporary_file);
        }
        #[cfg(target_os = "macos")]
        {
            drop(staging);
            let _ = parent.remove_dir(&temporary);
        }
        result
    }

    pub fn remove_file(&self) -> io::Result<()> {
        self.check(AccessMode::Write)?;
        if !self.metadata()?.is_file() {
            return Err(refused("Only regular files can be removed"));
        }
        self.check(AccessMode::Write)?;
        self.parent(false)?.remove_file(&self.name)
    }

    pub fn rename_to(&self, target: &Self) -> io::Result<()> {
        self.check(AccessMode::Write)?;
        target.check(AccessMode::Write)?;
        if !self.metadata()?.is_file() {
            return Err(refused("Only regular files can be renamed"));
        }
        let destination = target.parent(true)?;
        self.check(AccessMode::Write)?;
        target.check(AccessMode::Write)?;
        self.parent(false)?
            .rename(&self.name, &destination, &target.name)
    }
}

pub(crate) fn sandbox_path_error(error: io::Error, requested: &str) -> crate::SandboxError {
    if error.kind() == io::ErrorKind::PermissionDenied {
        crate::SandboxError::PathEscape(requested.to_owned())
    } else {
        crate::SandboxError::Io(error)
    }
}

#[derive(Clone, Debug)]
pub struct RootGrant {
    filesystem: Arc<BoundedFs>,
    access: AccessMode,
    targets: Option<Vec<PathBuf>>,
    live: Arc<AtomicBool>,
}

impl RootGrant {
    pub fn new(filesystem: Arc<BoundedFs>, access: AccessMode) -> Self {
        Self {
            filesystem,
            access,
            targets: None,
            live: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn with_targets(mut self, targets: Vec<PathBuf>) -> io::Result<Self> {
        self.targets = Some(
            targets
                .into_iter()
                .map(|target| {
                    self.filesystem
                        .bind(target, self.access)
                        .map(|bound| bound.relative)
                })
                .collect::<io::Result<_>>()?,
        );
        Ok(self)
    }

    pub fn revoke(&self) {
        self.live.store(false, Ordering::Release);
    }

    pub fn filesystem(&self) -> &Arc<BoundedFs> {
        &self.filesystem
    }

    pub fn access(&self) -> AccessMode {
        self.access
    }
}

struct DispatchEntry {
    session: String,
    action: String,
    grants: Vec<RootGrant>,
    aliases: HashMap<PathBuf, PathBuf>,
    live: Arc<AtomicBool>,
}

static DISPATCHES: LazyLock<Mutex<HashMap<String, Arc<DispatchEntry>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug)]
pub struct DispatchAuthority {
    token: String,
    live: Arc<AtomicBool>,
}

impl DispatchAuthority {
    pub fn register(session: &str, action: &str, grants: Vec<RootGrant>) -> io::Result<Self> {
        Self::register_aliases(session, action, grants, Vec::new())
    }

    pub fn register_aliases(
        session: &str,
        action: &str,
        grants: Vec<RootGrant>,
        aliases: Vec<(PathBuf, PathBuf)>,
    ) -> io::Result<Self> {
        if session.is_empty() || action.is_empty() || grants.is_empty() {
            return Err(refused(
                "File authority requires a session, action, and roots",
            ));
        }
        let mut mappings = HashMap::new();
        for (alias, target) in aliases {
            if !alias.is_absolute()
                || !target.is_absolute()
                || mappings.insert(alias, target).is_some()
            {
                return Err(refused("File aliases must map distinct absolute requested paths to approved absolute targets"));
            }
        }
        let token = Uuid::new_v4().to_string();
        let live = Arc::new(AtomicBool::new(true));
        DISPATCHES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                token.clone(),
                Arc::new(DispatchEntry {
                    session: session.to_owned(),
                    action: action.to_owned(),
                    grants,
                    aliases: mappings,
                    live: live.clone(),
                }),
            );
        Ok(Self { token, live })
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for DispatchAuthority {
    fn drop(&mut self) {
        self.live.store(false, Ordering::Release);
        DISPATCHES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.token);
    }
}

pub fn authorized_path(
    token: &str,
    session: &str,
    action: &str,
    requested: impl AsRef<Path>,
    access: AccessMode,
) -> io::Result<BoundedPath> {
    let entry = DISPATCHES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(token)
        .cloned()
        .ok_or_else(|| refused("This file authority is unavailable or expired"))?;
    if entry.session != session || entry.action != action || !entry.live.load(Ordering::Acquire) {
        return Err(refused(
            "File authority does not belong to this session and action",
        ));
    }
    let mut errors = VecDeque::new();
    let requested = entry
        .aliases
        .get(requested.as_ref())
        .map(PathBuf::as_path)
        .unwrap_or(requested.as_ref());
    for grant in &entry.grants {
        if !grant.access.permits(access) || !grant.live.load(Ordering::Acquire) {
            continue;
        }
        let mut bound = match grant.filesystem.bind(requested, access) {
            Ok(bound) => bound,
            Err(error) => {
                errors.push_back(error);
                continue;
            }
        };
        if grant
            .targets
            .as_ref()
            .is_some_and(|targets| !targets.contains(&bound.relative))
        {
            continue;
        }
        let mut authorization = bound
            .authorization
            .unwrap_or(Authorization { live: Vec::new() });
        authorization
            .live
            .extend([entry.live.clone(), grant.live.clone()]);
        bound.authorization = Some(authorization);
        bound.allow_subdirectory = grant.targets.is_none();
        bound.check(access)?;
        return Ok(bound);
    }
    Err(errors
        .pop_back()
        .unwrap_or_else(|| refused("The pending action does not authorize this path or access")))
}
