//! Windows local-NTFS publication. Data is flushed before a write-through move;
//! this does not claim POSIX directory-fsync semantics. Callers must reload
//! receipts and reconcile an unconfirmed publication before retrying it.
use anyhow::{ensure, Context, Result};
use cap_std::fs::Dir;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr::{addr_of, null_mut};
use windows_sys::Win32::Foundation::{
    LocalFree, GENERIC_ALL, GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    EqualSid, GetAce, GetLengthSid, GetTokenInformation, IsValidAcl, IsValidSid, IsWellKnownSid,
    LookupAccountNameW, TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    ACCESS_ALLOWED_ACE, ACE_HEADER, DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE,
    OWNER_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx, GetFileType,
    GetFinalPathNameByHandleW, GetVolumeInformationByHandleW, MoveFileExW,
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_APPEND_DATA, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_DELETE_CHILD, FILE_EXECUTE,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_READ_DATA,
    FILE_READ_EA, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK,
    FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA, MOVEFILE_REPLACE_EXISTING,
    MOVEFILE_WRITE_THROUGH, READ_CONTROL, WRITE_DAC, WRITE_OWNER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

const MUTATION: u32 = DELETE
    | WRITE_DAC
    | WRITE_OWNER
    | FILE_DELETE_CHILD
    | FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | GENERIC_WRITE
    | GENERIC_ALL;
const ANCESTOR_MUTATION: u32 = DELETE
    | WRITE_DAC
    | WRITE_OWNER
    | FILE_DELETE_CHILD
    | FILE_WRITE_ATTRIBUTES
    | GENERIC_WRITE
    | GENERIC_ALL;
const PRIVATE_ACCESS: u32 =
    MUTATION | FILE_READ_DATA | FILE_READ_EA | FILE_EXECUTE | GENERIC_READ | GENERIC_EXECUTE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    pub volume: u32,
    pub index: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamespaceDurability {
    RecoverByReloadingReceipts,
}

#[derive(Debug)]
pub enum Publication {
    Published(FileIdentity),
    Unconfirmed {
        expected: FileIdentity,
        diagnostic: String,
    },
}

pub struct DirectoryLease {
    path: PathBuf,
    identity: FileIdentity,
    directory: File,
    ancestors: Vec<File>,
}

fn handle(file: &File) -> HANDLE {
    file.as_raw_handle().cast()
}

fn checked(success: i32) -> Result<()> {
    ensure!(success != 0, "{}", std::io::Error::last_os_error());
    Ok(())
}

fn information(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info = unsafe { zeroed() };
    checked(unsafe { GetFileInformationByHandle(handle(file), &mut info) })?;
    ensure!(
        unsafe { GetFileType(handle(file)) } == FILE_TYPE_DISK,
        "Select a disk file"
    );
    ensure!(
        info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0,
        "Reparse points are not supported"
    );
    Ok(info)
}

pub fn file_identity(file: &File) -> Result<FileIdentity> {
    let info = information(file)?;
    Ok(FileIdentity {
        volume: info.dwVolumeSerialNumber,
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

pub fn change_time(file: &File) -> Result<i64> {
    let mut basic: FILE_BASIC_INFO = unsafe { zeroed() };
    checked(unsafe {
        GetFileInformationByHandleEx(
            handle(file),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    })?;
    Ok(basic.ChangeTime)
}

fn current_user() -> Result<Vec<usize>> {
    let mut token = null_mut();
    checked(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) })?;
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut size = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            TokenUser,
            null_mut(),
            0,
            &mut size,
        );
    }
    ensure!(
        size >= size_of::<TOKEN_USER>() as u32 && size <= 65536,
        "Invalid Windows user token size"
    );
    let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    checked(unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    })?;
    Ok(buffer)
}

fn trusted_installer_sid() -> Option<Vec<usize>> {
    let name: Vec<u16> = "NT SERVICE\\TrustedInstaller"
        .encode_utf16()
        .chain([0])
        .collect();
    let (mut size, mut domain_size, mut kind) = (0, 0, 0);
    unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            null_mut(),
            &mut size,
            null_mut(),
            &mut domain_size,
            &mut kind,
        );
    }
    if !(8..=1024).contains(&size) || domain_size == 0 || domain_size > 256 {
        return None;
    }
    let mut sid = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    let mut domain = vec![0u16; domain_size as usize];
    if unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut size,
            domain.as_mut_ptr(),
            &mut domain_size,
            &mut kind,
        )
    } == 0
    {
        return None;
    }
    let end = domain
        .iter()
        .position(|ch| *ch == 0)
        .unwrap_or(domain.len());
    if !String::from_utf16_lossy(&domain[..end]).eq_ignore_ascii_case("NT SERVICE")
        || unsafe { IsValidSid(sid.as_ptr().cast_mut().cast()) } == 0
    {
        return None;
    }
    Some(sid)
}

fn trusted(sid: PSID, user: PSID, ancestor: bool) -> bool {
    let base = unsafe {
        EqualSid(sid, user) != 0
            || IsWellKnownSid(sid, WinLocalSystemSid) != 0
            || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
    };
    if base || !ancestor {
        return base;
    }
    // Windows Resource Protection uses this exact OS service principal. It is
    // trusted only for ancestors, never as owner of user files or destinations.
    static INSTALLER: std::sync::OnceLock<Option<Vec<usize>>> = std::sync::OnceLock::new();
    INSTALLER
        .get_or_init(trusted_installer_sid)
        .as_ref()
        .is_some_and(|installer| unsafe {
            EqualSid(sid, installer.as_ptr().cast_mut().cast()) != 0
        })
}

fn check_acl(file: &File, private: bool, ancestor: bool) -> Result<()> {
    let token = current_user()?;
    let user = unsafe { (*(token.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let mut owner = null_mut();
    let mut dacl = null_mut();
    let mut descriptor = null_mut();
    let status = unsafe {
        GetSecurityInfo(
            handle(file),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    ensure!(
        status == 0,
        "Could not inspect Windows file security: {}",
        std::io::Error::from_raw_os_error(status as i32)
    );
    let result = (|| {
        ensure!(
            !owner.is_null() && unsafe { IsValidSid(owner) } != 0,
            "Invalid file owner SID"
        );
        ensure!(
            if ancestor {
                trusted(owner, user, true)
            } else {
                unsafe { EqualSid(owner, user) != 0 }
            },
            "File or directory is not owned by the required trusted account"
        );
        ensure!(
            !dacl.is_null() && unsafe { IsValidAcl(dacl) } != 0,
            "An explicit valid Windows DACL is required"
        );
        let prohibited = if private {
            PRIVATE_ACCESS
        } else if ancestor {
            ANCESTOR_MUTATION
        } else {
            MUTATION
        };
        for index in 0..u32::from(unsafe { (*dacl).AceCount }) {
            let mut entry = null_mut();
            checked(unsafe { GetAce(dacl, index, &mut entry) })?;
            let header = unsafe { &*entry.cast::<ACE_HEADER>() };
            if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 || header.AceType == 1 {
                continue;
            }
            ensure!(header.AceType == 0 && header.AceSize >= 16, "Unsupported Windows ACL entry; choose a directory with simple allow/deny permissions");
            let ace = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
            let sid = addr_of!(ace.SidStart).cast_mut().cast();
            ensure!(
                unsafe { IsValidSid(sid) } != 0
                    && 8 + unsafe { GetLengthSid(sid) } <= u32::from(header.AceSize),
                "Invalid ACL principal SID"
            );
            ensure!(
                ace.Mask & prohibited == 0 || trusted(sid, user, ancestor),
                "Another account can access or modify this protected transfer location"
            );
        }
        Ok(())
    })();
    unsafe {
        LocalFree(descriptor);
    }
    result
}

pub fn protected_directory(directory: &Dir) -> Result<()> {
    let file = directory.try_clone()?.into_std_file();
    ensure!(
        information(&file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        "Expected a directory"
    );
    check_acl(&file, false, false)
}

pub fn private_directory(directory: &Dir) -> Result<()> {
    let file = directory.try_clone()?.into_std_file();
    ensure!(
        information(&file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        "Expected private directory"
    );
    check_acl(&file, true, false)
}

pub fn validate_cleanup(file: &File) -> Result<FileIdentity> {
    let info = information(file)?;
    ensure!(
        info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 && info.nNumberOfLinks >= 1,
        "Cleanup requires a regular linked file"
    );
    check_acl(file, true, false)?;
    file_identity(file)
}

pub fn validate_partial(file: &File) -> Result<FileIdentity> {
    let info = information(file)?;
    ensure!(
        info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 && info.nNumberOfLinks == 1,
        "Transfer file must be regular and have exactly one link"
    );
    check_acl(file, true, false)?;
    file_identity(file)
}

fn final_path(file: &File) -> Result<PathBuf> {
    let mut buffer = vec![0u16; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(handle(file), buffer.as_mut_ptr(), buffer.len() as u32, 0)
    };
    ensure!(
        length > 0 && (length as usize) < buffer.len(),
        "Could not obtain the complete selected Windows path"
    );
    buffer.truncate(length as usize);
    Ok(PathBuf::from(OsString::from_wide(&buffer)))
}

fn open_pinned(path: &Path, directory: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options
        .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES)
        .share_mode(
            FILE_SHARE_READ | FILE_SHARE_WRITE | if directory { 0 } else { FILE_SHARE_DELETE },
        )
        .custom_flags(
            FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
        );
    Ok(options.open(path)?)
}

fn local_ntfs(file: &File) -> Result<()> {
    let mut filesystem = [0u16; 32];
    checked(unsafe {
        GetVolumeInformationByHandleW(
            handle(file),
            null_mut(),
            0,
            null_mut(),
            null_mut(),
            null_mut(),
            filesystem.as_mut_ptr(),
            filesystem.len() as u32,
        )
    })?;
    let end = filesystem
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(filesystem.len());
    ensure!(
        String::from_utf16_lossy(&filesystem[..end]) == "NTFS",
        "Secure Windows transfers currently require a local NTFS volume"
    );
    Ok(())
}

fn simple_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name != "."
            && name != ".."
            && !name.ends_with(['.', ' '])
            && !name
                .chars()
                .any(|ch| ch < ' '
                    || matches!(ch, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')),
        "Select a regular Windows filename without streams or path components"
    );
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    ensure!(
        !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !(stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit()),
        "Windows device names are not file destinations"
    );
    Ok(())
}

fn wide(path: &Path) -> Result<Vec<u16>> {
    let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
    ensure!(!value.contains(&0), "Windows path contains NUL");
    value.push(0);
    Ok(value)
}

impl DirectoryLease {
    pub fn acquire(directory: &Dir) -> Result<Self> {
        protected_directory(directory)?;
        let file = directory.try_clone()?.into_std_file();
        local_ntfs(&file)?;
        let identity = file_identity(&file)?;
        let path = final_path(&file)?;
        ensure!(
            matches!(path.components().next(), Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_))),
            "Secure publication requires a local drive path, not a network share"
        );
        let mut current = PathBuf::new();
        let mut ancestors = Vec::new();
        for component in path.components() {
            current.push(component.as_os_str());
            if matches!(component, Component::Prefix(_)) {
                continue;
            }
            ensure!(
                matches!(component, Component::RootDir | Component::Normal(_)),
                "Invalid Windows directory path"
            );
            let pinned = open_pinned(&current, true)?;
            ensure!(
                information(&pinned)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                "Directory path was substituted"
            );
            check_acl(&pinned, false, current != path)?;
            ancestors.push(pinned);
        }
        ensure!(
            file_identity(
                ancestors
                    .last()
                    .context("Missing selected directory handle")?
            )? == identity,
            "Selected directory was substituted"
        );
        Ok(Self {
            path,
            identity,
            directory: file,
            ancestors,
        })
    }

    pub fn namespace_durability(&self) -> Result<NamespaceDurability> {
        self.revalidate()?;
        Ok(NamespaceDurability::RecoverByReloadingReceipts)
    }

    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub fn revalidate(&self) -> Result<()> {
        ensure!(
            file_identity(&self.directory)? == self.identity,
            "Directory identity changed"
        );
        for (index, ancestor) in self.ancestors.iter().enumerate() {
            information(ancestor)?;
            check_acl(ancestor, false, index + 1 != self.ancestors.len())?;
        }
        let named = open_pinned(&self.path, true)?;
        ensure!(
            file_identity(&named)? == self.identity,
            "Selected directory pathname changed"
        );
        Ok(())
    }

    /// The caller retains a writable source handle opened with FILE_SHARE_DELETE.
    /// Directory ACLs exclude untrusted mutation while MoveFileEx needs delete
    /// sharing on the source itself. No-delete sharing remains on every ancestor.
    pub fn publish(
        &self,
        source: &File,
        part: &str,
        name: &str,
        overwrite: bool,
    ) -> Result<Publication> {
        self.publish_selected(source, part, name, overwrite, None)
    }

    pub fn publish_selected(
        &self,
        source: &File,
        part: &str,
        name: &str,
        overwrite: bool,
        target: Option<&super::TargetApproval>,
    ) -> Result<Publication> {
        simple_name(part)?;
        simple_name(name)?;
        ensure!(part != name, "Partial and final file names must differ");
        self.revalidate()?;
        let expected = validate_partial(source)?;
        ensure!(
            expected.volume == self.identity.volume,
            "Publication cannot cross volumes"
        );
        let named = open_pinned(&self.path.join(part), false)?;
        ensure!(
            validate_partial(&named)? == expected,
            "Partial filename was substituted"
        );
        match open_pinned(&self.path.join(name), false) {
            Ok(target) => {
                ensure!(
                    overwrite,
                    "Destination exists; approve replacement explicitly"
                );
                validate_partial(&target)?;
            }
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
        source.sync_all()?;
        let source_path = wide(&self.path.join(part))?;
        let target_path = wide(&self.path.join(name))?;
        // MoveFileEx documents WRITE_THROUGH; neither COPY_ALLOWED nor deferred
        // operations are permitted. This is not a directory FlushFileBuffers.
        if let Some(target) = target {
            target.verify(&Dir::from_std_file(self.directory.try_clone()?), name)?;
        }
        let moved = unsafe {
            MoveFileExW(
                source_path.as_ptr(),
                target_path.as_ptr(),
                MOVEFILE_WRITE_THROUGH
                    | if overwrite {
                        MOVEFILE_REPLACE_EXISTING
                    } else {
                        0
                    },
            )
        };
        if moved == 0 {
            return Ok(Publication::Unconfirmed {
                expected,
                diagnostic: std::io::Error::last_os_error().to_string(),
            });
        }
        let verified = (|| {
            self.revalidate()?;
            let target = open_pinned(&self.path.join(name), false)?;
            ensure!(
                validate_partial(&target)? == expected,
                "Published file identity changed"
            );
            Ok::<_, anyhow::Error>(())
        })();
        Ok(match verified {
            Ok(()) => Publication::Published(expected),
            Err(error) => Publication::Unconfirmed {
                expected,
                diagnostic: error.to_string(),
            },
        })
    }
}
