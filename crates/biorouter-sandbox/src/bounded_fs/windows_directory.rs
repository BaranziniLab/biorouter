use std::fs::File;
use std::io;
use std::mem::{size_of, MaybeUninit};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};

use cap_std::fs::{Dir, Metadata, OpenOptions};
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    NtCreateFile, FILE_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT,
    FILE_SYNCHRONOUS_IO_NONALERT,
};
use windows_sys::Win32::Foundation::{
    RtlNtStatusToDosError, STATUS_PENDING, STATUS_SUCCESS, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_GENERIC_READ, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

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
    let mut empty_name = [0u16];
    let name = UNICODE_STRING {
        Length: 0,
        MaximumLength: size_of::<u16>() as u16,
        Buffer: empty_name.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: file.as_raw_handle(),
        ObjectName: &name,
        ..OBJECT_ATTRIBUTES::default()
    };
    let mut status_block = IO_STATUS_BLOCK::default();
    status_block.Anonymous.Status = STATUS_PENDING;
    let mut handle = std::ptr::null_mut();
    // ReOpenFile can reject directories. An empty native name reopens this
    // retained object with explicit directory semantics, without pathname lookup.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            FILE_GENERIC_READ,
            &attributes,
            &mut status_block,
            std::ptr::null(),
            0,
            share,
            FILE_OPEN,
            FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT,
            std::ptr::null(),
            0,
        )
    };
    if status != STATUS_SUCCESS {
        let error = io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32);
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
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
    };

    use crate::bounded_fs::{AccessMode, BoundedFs};

    #[test]
    fn volume_root_and_ordinary_directories_can_be_captured_and_leased() {
        let fixture = tempfile::tempdir().unwrap();
        let mut volume_root = PathBuf::new();
        for component in fixture.path().components() {
            match component {
                std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                    volume_root.push(component.as_os_str());
                }
                _ => break,
            }
        }
        for path in [volume_root.as_path(), fixture.path()] {
            let original = Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap();
            let original_identity =
                identity(&original.try_clone().unwrap().into_std_file()).unwrap();
            let captured = PinnedDir::capture(original.into_std_file()).unwrap();
            let lease = captured.lease().unwrap();
            assert_eq!(captured.identity(), original_identity);
            assert_eq!(
                identity(&lease.directory.into_std_file()).unwrap(),
                original_identity
            );
        }
    }

    #[test]
    fn ordinary_directory_leases_allow_listing_and_file_mutations() {
        let fixture = tempfile::tempdir().unwrap();
        let selected = fixture.path().join("workspace");
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("file.txt"), "approved").unwrap();
        let filesystem = BoundedFs::open(&selected).unwrap();
        let entries = filesystem
            .bind(".", AccessMode::Read)
            .unwrap()
            .list()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "file.txt");
        let file = filesystem.bind("file.txt", AccessMode::Write).unwrap();
        assert_eq!(file.read().unwrap(), b"approved");
        file.atomic_write(b"edited").unwrap();
        let renamed = filesystem.bind("renamed.txt", AccessMode::Write).unwrap();
        file.rename_to(&renamed).unwrap();
        assert_eq!(renamed.read().unwrap(), b"edited");
        renamed.remove_file().unwrap();
        assert!(filesystem
            .bind(".", AccessMode::Read)
            .unwrap()
            .list()
            .unwrap()
            .is_empty());
    }

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
    fn captured_descendant_prevents_ancestor_move_until_released() {
        let fixture = tempfile::tempdir().unwrap();
        let selected = fixture.path().join("parent");
        let moved = fixture.path().join("original-parent");
        fs::create_dir_all(selected.join("new/nested")).unwrap();
        fs::write(selected.join("new/nested/file.txt"), "synthetic-approved").unwrap();
        let original = Dir::open_ambient_dir(&selected, cap_std::ambient_authority()).unwrap();
        let parent = PinnedDir::capture(original.into_std_file()).unwrap();
        let parent_identity = parent.identity();
        let descendant = parent.open_directory(Path::new("new/nested")).unwrap();
        let blocked = fs::rename(&selected, &moved).unwrap_err();
        assert_eq!(blocked.raw_os_error(), Some(5));
        assert_eq!(
            fs::read(selected.join("new/nested/file.txt")).unwrap(),
            b"synthetic-approved"
        );
        drop(descendant);
        fs::rename(&selected, &moved).unwrap();
        let lease = parent.lease().unwrap();
        assert_eq!(
            identity(&lease.directory.into_std_file()).unwrap(),
            parent_identity
        );
        assert_eq!(
            fs::read(moved.join("new/nested/file.txt")).unwrap(),
            b"synthetic-approved"
        );
        assert!(!selected.exists());
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
