use std::fs::File;
use std::io;
use std::mem::MaybeUninit;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};

use cap_std::fs::{Dir, Metadata, OpenOptions};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, ReOpenFile, BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

use super::{
    nofollow_options, private_options, refused, reject_reparse, BoundedDirEntry, RootIdentity,
};

#[derive(Debug)]
pub(super) struct PinnedDir {
    file: File,
    identity: RootIdentity,
}

pub(super) struct DirectoryLease {
    directory: Dir,
}

fn identity(file: &File) -> io::Result<RootIdentity> {
    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let information = unsafe { information.assume_init() };
    Ok(RootIdentity {
        device: u64::from(information.dwVolumeSerialNumber),
        file: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
        created_nanos: Some(
            u128::from(
                (u64::from(information.ftCreationTime.dwHighDateTime) << 32)
                    | u64::from(information.ftCreationTime.dwLowDateTime),
            ) * 100,
        ),
    })
}

fn reopen(file: &File, share_delete: bool) -> io::Result<File> {
    let share =
        FILE_SHARE_READ | FILE_SHARE_WRITE | if share_delete { FILE_SHARE_DELETE } else { 0 };
    let handle = unsafe {
        ReOpenFile(
            file.as_raw_handle(),
            FILE_GENERIC_READ,
            share,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        return Err(io::Error::new(
            error.kind(),
            format!("The authorized folder is busy or unavailable: {error}. Finish moving or closing this folder, then retry the original operation."),
        ));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn validate(file: &File, expected: RootIdentity) -> io::Result<()> {
    let metadata = file.metadata()?;
    reject_reparse(&metadata)?;
    if !metadata.is_dir() || identity(file)? != expected {
        return Err(refused("The authorized directory object changed"));
    }
    Ok(())
}

impl PinnedDir {
    pub(super) fn capture(file: File) -> io::Result<Self> {
        let expected = identity(&file)?;
        validate(&file, expected)?;
        let identity_file = reopen(&file, true)?;
        validate(&identity_file, expected)?;
        Ok(Self {
            file: identity_file,
            identity: expected,
        })
    }

    pub(super) fn identity(&self) -> RootIdentity {
        self.identity
    }

    pub(super) fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            file: self.file.try_clone()?,
            identity: self.identity,
        })
    }

    pub(super) fn lease(&self) -> io::Result<DirectoryLease> {
        // cap-primitives derives some operation paths from the handle, so forbid moves until each operation finishes.
        let file = reopen(&self.file, false)?;
        validate(&file, self.identity)?;
        Ok(DirectoryLease {
            directory: Dir::from_std_file(file),
        })
    }

    pub(super) fn open_with(
        &self,
        path: impl AsRef<Path>,
        options: &OpenOptions,
    ) -> io::Result<cap_std::fs::File> {
        self.lease()?.open_with(path, options)
    }

    pub(super) fn open_directory(&self, path: &Path) -> io::Result<Self> {
        let lease = self.lease()?;
        let file = lease.open_with(path, &nofollow_options(true))?.into_std();
        Self::capture(file)
    }

    pub(super) fn private_directory(&self, path: &Path) -> io::Result<Self> {
        let lease = self.lease()?;
        match lease.directory.create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let file = lease.open_with(path, &private_options(true))?.into_std();
        let expected = identity(&file)?;
        validate(&file, expected)?;
        crate::owner_private::protect(&file, true)?;
        Self::capture(file)
    }

    pub(super) fn protect_owner_private(&self) -> io::Result<()> {
        let lease = self.lease()?;
        let file = lease.open_with(".", &private_options(true))?.into_std();
        crate::owner_private::protect(&file, true)
    }

    pub(super) fn symlink_metadata(&self, path: impl AsRef<Path>) -> io::Result<Metadata> {
        let lease = self.lease()?;
        lease.directory.symlink_metadata(path)
    }

    pub(super) fn read_link_contents(&self, path: impl AsRef<Path>) -> io::Result<PathBuf> {
        let lease = self.lease()?;
        lease.directory.read_link_contents(path)
    }

    pub(super) fn create_dir(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let lease = self.lease()?;
        lease.directory.create_dir(path)
    }

    pub(super) fn remove_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.lease()?.remove_file(path)
    }

    pub(super) fn rename(
        &self,
        source: impl AsRef<Path>,
        target: &Self,
        destination: impl AsRef<Path>,
    ) -> io::Result<()> {
        let source_lease = self.lease()?;
        let target_lease = target.lease()?;
        source_lease.rename(source, &target_lease, destination)
    }

    pub(super) fn list_limited(&self, max_entries: usize) -> io::Result<Vec<BoundedDirEntry>> {
        let lease = self.lease()?;
        let entries = lease.directory.entries()?;
        entries.enumerate().map(|(index, entry)| {
            if index >= max_entries {
                return Err(io::Error::new(io::ErrorKind::InvalidInput,
                    "Analysis contains too many directory entries; choose a narrower folder or depth"));
            }
            let entry = entry?;
            let metadata = lease.directory.symlink_metadata(entry.file_name())?;
            Ok(BoundedDirEntry {
                name: entry.file_name(),
                is_dir: metadata.is_dir(),
                is_symlink: metadata.file_type().is_symlink(),
                len: metadata.len(),
            })
        }).collect()
    }
}

impl DirectoryLease {
    pub(super) fn open_with(
        &self,
        path: impl AsRef<Path>,
        options: &OpenOptions,
    ) -> io::Result<cap_std::fs::File> {
        self.directory.open_with(path, options)
    }

    pub(super) fn rename(
        &self,
        source: impl AsRef<Path>,
        target: &Self,
        destination: impl AsRef<Path>,
    ) -> io::Result<()> {
        self.directory
            .rename(source, &target.directory, destination)
    }

    pub(super) fn remove_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.directory.remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::windows::fs::OpenOptionsExt;
    use std::sync::mpsc;
    use std::time::Duration;
    use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_READ_ATTRIBUTES};

    use crate::bounded_fs::{AccessMode, BoundedFs};

    #[test]
    fn directory_moves_wait_for_the_operation_lease_then_keep_the_captured_object() {
        let fixture = tempfile::tempdir().unwrap();
        let selected = fixture.path().join("workspace");
        let moved = fixture.path().join("original");
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("file.txt"), "approved").unwrap();
        let filesystem = BoundedFs::open(&selected).unwrap();
        let directory = filesystem.inner.directory.try_clone().unwrap();
        let (arrived_sender, arrived) = mpsc::channel();
        let (release, worker_release) = mpsc::channel();
        let operation = std::thread::spawn(move || {
            let lease = directory.lease().unwrap();
            arrived_sender.send(()).unwrap();
            worker_release.recv_timeout(Duration::from_secs(5)).unwrap();
            lease
                .open_with("file.txt", &nofollow_options(false))
                .unwrap()
                .metadata()
                .unwrap();
        });
        arrived.recv_timeout(Duration::from_secs(5)).unwrap();
        let rename_while_active = fs::rename(&selected, &moved);
        release.send(()).unwrap();
        operation.join().unwrap();
        assert!(rename_while_active.is_err());
        fs::rename(&selected, &moved).unwrap();
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("file.txt"), "outside-substitute").unwrap();
        let file = filesystem.bind("file.txt", AccessMode::Write).unwrap();
        assert_eq!(file.read().unwrap(), b"approved");
        file.atomic_write(b"approved-edit").unwrap();
        assert_eq!(fs::read(moved.join("file.txt")).unwrap(), b"approved-edit");
        assert_eq!(
            fs::read(selected.join("file.txt")).unwrap(),
            b"outside-substitute"
        );
    }

    #[test]
    fn an_existing_delete_handle_refuses_a_lease_without_pathname_fallback() {
        let fixture = tempfile::tempdir().unwrap();
        let selected = fixture.path().join("workspace");
        let outside = fixture.path().join("outside.txt");
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("file.txt"), "approved").unwrap();
        fs::write(&outside, "outside-canary").unwrap();
        let filesystem = BoundedFs::open(&selected).unwrap();
        let delete_handle = fs::OpenOptions::new()
            .access_mode(DELETE | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&selected)
            .unwrap();
        let error = filesystem.bind("file.txt", AccessMode::Read).unwrap_err();
        assert!(error.to_string().contains("busy or unavailable"));
        assert!(error.to_string().contains("retry the original operation"));
        assert_eq!(fs::read(&outside).unwrap(), b"outside-canary");
        drop(delete_handle);
        assert_eq!(
            filesystem
                .bind("file.txt", AccessMode::Read)
                .unwrap()
                .read()
                .unwrap(),
            b"approved"
        );
        assert!(filesystem.bind(&outside, AccessMode::Read).is_err());
    }
}
