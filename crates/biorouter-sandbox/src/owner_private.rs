use std::fs::File;
use std::io;

#[cfg(unix)]
pub(crate) fn protect(file: &File, directory: bool) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = file.metadata()?;
    if !directory && metadata.nlink() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Private storage must not share a hard-linked file inode",
        ));
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Private storage must be owned by the current account",
        ));
    }
    let mode = if directory { 0o700 } else { 0o600 };
    file.set_permissions(std::fs::Permissions::from_mode(mode))?;
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        unsafe extern "C" {
            fn acl_init(count: libc::c_int) -> *mut libc::c_void;
            fn acl_set_fd(fd: libc::c_int, acl: *mut libc::c_void) -> libc::c_int;
            fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
        }
        let acl = unsafe { acl_init(0) };
        if acl.is_null() {
            return Err(io::Error::last_os_error());
        }
        let result = unsafe { acl_set_fd(file.as_raw_fd(), acl) };
        let error = (result != 0).then(io::Error::last_os_error);
        unsafe {
            acl_free(acl);
        }
        if let Some(error) = error {
            return Err(error);
        }
    }
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::fremovexattr(file.as_raw_fd(), c"system.posix_acl_access".as_ptr()) } != 0
        {
            let error = io::Error::last_os_error();
            if !matches!(error.raw_os_error(), Some(libc::ENODATA | libc::ENOTSUP)) {
                return Err(error);
            }
        }
        if directory
            && unsafe { libc::fremovexattr(file.as_raw_fd(), c"system.posix_acl_default".as_ptr()) }
                != 0
        {
            let error = io::Error::last_os_error();
            if !matches!(error.raw_os_error(), Some(libc::ENODATA | libc::ENOTSUP)) {
                return Err(error);
            }
        }
    }
    if file.metadata()?.permissions().mode() & 0o777 != mode {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "The filesystem does not enforce owner-only permissions",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn current_token_information(
    information: windows_sys::Win32::Security::TOKEN_INFORMATION_CLASS,
    minimum: usize,
) -> io::Result<Vec<usize>> {
    use std::mem::size_of;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr::null_mut;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut size = 0;
    unsafe {
        GetTokenInformation(token.as_raw_handle(), information, null_mut(), 0, &mut size);
    }
    if !(minimum as u32..=65536).contains(&size) {
        return Err(io::Error::other("Invalid current-account token size"));
    }
    let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            information,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(buffer)
}

#[cfg(windows)]
fn owner_is_current(
    owner: windows_sys::Win32::Security::PSID,
    user: windows_sys::Win32::Security::PSID,
    default_owner: windows_sys::Win32::Security::PSID,
) -> bool {
    use windows_sys::Win32::Security::{EqualSid, IsValidSid};
    // Elevated processes can create files owned by their token's default group.
    !owner.is_null()
        && unsafe { IsValidSid(owner) } != 0
        && (unsafe { EqualSid(owner, user) } != 0
            || (!default_owner.is_null()
                && unsafe { IsValidSid(default_owner) } != 0
                && privileged_sid(default_owner)
                && unsafe { EqualSid(owner, default_owner) } != 0))
}

#[cfg(windows)]
fn privileged_sid(sid: windows_sys::Win32::Security::PSID) -> bool {
    use windows_sys::Win32::Security::{
        IsValidSid, IsWellKnownSid, WinBuiltinAdministratorsSid, WinLocalSystemSid,
    };
    !sid.is_null()
        && unsafe { IsValidSid(sid) } != 0
        && (unsafe { IsWellKnownSid(sid, WinLocalSystemSid) } != 0
            || unsafe { IsWellKnownSid(sid, WinBuiltinAdministratorsSid) } != 0)
}

#[cfg(windows)]
fn unsafe_directory_migration() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Private storage has a directory writable by another account or an unrecognized ACL. Close other applications, restrict that directory to your Windows account using Windows security settings, and retry; this directory was not automatically repaired",
    )
}

#[cfg(windows)]
pub(crate) fn validate_directory_acl(file: &File) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{
        LocalFree, GENERIC_ALL, GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE,
    };
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        EqualSid, GetAce, IsValidAcl, IsValidSid, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, INHERITED_ACE, INHERIT_ONLY_ACE,
        NO_PROPAGATE_INHERIT_ACE, OBJECT_INHERIT_ACE, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD, FILE_EXECUTE,
        FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_READ_EA, FILE_WRITE_ATTRIBUTES, FILE_WRITE_EA,
        READ_CONTROL, SYNCHRONIZE, WRITE_DAC, WRITE_OWNER,
    };

    let buffer = current_token_information(TokenUser, size_of::<TOKEN_USER>())?;
    let user = unsafe { (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let mut acl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    struct Allocation(*mut std::ffi::c_void);
    impl Drop for Allocation {
        fn drop(&mut self) {
            unsafe { LocalFree(self.0) };
        }
    }
    let _descriptor = Allocation(descriptor);
    if acl.is_null() || unsafe { IsValidAcl(acl) } == 0 {
        return Err(unsafe_directory_migration());
    }
    let acl_start = acl as usize;
    let acl_end = acl_start
        .checked_add(usize::from(unsafe { (*acl).AclSize }))
        .ok_or_else(unsafe_directory_migration)?;
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;
    const ACCESS_SYSTEM_SECURITY: u32 = 0x0100_0000;
    const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
    let mutation = FILE_ADD_FILE
        | FILE_ADD_SUBDIRECTORY
        | FILE_DELETE_CHILD
        | FILE_WRITE_ATTRIBUTES
        | FILE_WRITE_EA
        | DELETE
        | WRITE_DAC
        | WRITE_OWNER
        | GENERIC_WRITE
        | GENERIC_ALL
        | ACCESS_SYSTEM_SECURITY
        | MAXIMUM_ALLOWED;
    let recognized = mutation
        | FILE_READ_DATA
        | FILE_READ_EA
        | FILE_EXECUTE
        | FILE_READ_ATTRIBUTES
        | READ_CONTROL
        | SYNCHRONIZE
        | GENERIC_READ
        | GENERIC_EXECUTE;
    let recognized_flags = OBJECT_INHERIT_ACE
        | CONTAINER_INHERIT_ACE
        | NO_PROPAGATE_INHERIT_ACE
        | INHERIT_ONLY_ACE
        | INHERITED_ACE;
    for index in 0..unsafe { (*acl).AceCount } {
        let mut entry = null_mut();
        if unsafe { GetAce(acl, index.into(), &mut entry) } == 0 || entry.is_null() {
            return Err(unsafe_directory_migration());
        }
        let entry_start = entry as usize;
        if entry_start < acl_start + size_of::<ACL>()
            || entry_start > acl_end.saturating_sub(size_of::<ACE_HEADER>())
        {
            return Err(unsafe_directory_migration());
        }
        let header = unsafe { &*entry.cast::<ACE_HEADER>() };
        // Object and callback ACEs need additional authorization semantics.
        if !matches!(
            header.AceType,
            ACCESS_ALLOWED_ACE_TYPE | ACCESS_DENIED_ACE_TYPE
        ) || header.AceSize < 16
            || usize::from(header.AceSize) > acl_end - entry_start
            || u32::from(header.AceFlags) & !recognized_flags != 0
        {
            return Err(unsafe_directory_migration());
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(entry.cast::<u8>(), usize::from(header.AceSize)) };
        let sid_size = 8 + usize::from(bytes[9]) * 4;
        if sid_size + 8 > bytes.len() {
            return Err(unsafe_directory_migration());
        }
        let allowed = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = (&allowed.SidStart as *const u32).cast_mut().cast();
        if unsafe { IsValidSid(sid) } == 0 || allowed.Mask & !recognized != 0 {
            return Err(unsafe_directory_migration());
        }
        if header.AceType == ACCESS_ALLOWED_ACE_TYPE
            && u32::from(header.AceFlags) & INHERIT_ONLY_ACE == 0
            && allowed.Mask & mutation != 0
            && unsafe { EqualSid(sid, user) } == 0
            && !privileged_sid(sid)
        {
            return Err(unsafe_directory_migration());
        }
    }
    Ok(())
}

#[cfg(windows)]
fn preflight_directory_tree(file: &File) -> io::Result<Vec<File>> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
    use cap_std::fs::{Dir, OpenOptions, OpenOptionsExt};
    use std::mem::size_of;
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::io::AsRawHandle;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        TokenOwner, TokenUser, OWNER_SECURITY_INFORMATION, TOKEN_OWNER, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let token = current_token_information(TokenUser, size_of::<TOKEN_USER>())?;
    let user = unsafe { (*(token.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let owner_token = current_token_information(TokenOwner, size_of::<TOKEN_OWNER>())?;
    let default_owner = unsafe { (*(owner_token.as_ptr().cast::<TOKEN_OWNER>())).Owner };
    let mut pending = vec![file.try_clone()?];
    let mut retained = Vec::new();
    while let Some(file) = pending.pop() {
        let metadata = file.metadata()?;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || (!metadata.is_dir() && !metadata.is_file())
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Private storage must contain only regular files and directories without reparse points",
            ));
        }
        if metadata.is_file() {
            let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) }
                == 0
            {
                return Err(io::Error::last_os_error());
            }
            if unsafe { information.assume_init() }.nNumberOfLinks != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Private storage must not share a hard-linked file inode",
                ));
            }
        }
        let mut owner = null_mut();
        let mut descriptor = null_mut();
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let owned = owner_is_current(owner, user, default_owner);
        unsafe { LocalFree(descriptor) };
        if !owned {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Private storage must be owned by the current Windows account",
            ));
        }
        if metadata.is_dir() {
            validate_directory_acl(&file)?;
            let directory = Dir::from_std_file(file.try_clone()?);
            for entry in directory.entries()? {
                let mut options = OpenOptions::new();
                options
                    .read(true)
                    .follow(FollowSymlinks::No)
                    .access_mode(FILE_GENERIC_READ)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
                pending.push(entry?.open_with(&options)?.into_std());
            }
        }
        retained.push(file);
    }
    Ok(retained)
}

#[cfg(windows)]
pub(crate) fn protect(file: &File, directory: bool) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE,
        SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        TokenOwner, TokenUser, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_OWNER,
        TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ALL_ACCESS,
    };
    struct Allocation(*mut std::ffi::c_void);
    impl Drop for Allocation {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    fn status(code: u32) -> io::Result<()> {
        if code == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(code as i32))
        }
    }
    if !directory {
        let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe { information.assume_init() }.nNumberOfLinks != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Private storage must not share a hard-linked file inode",
            ));
        }
    }
    let buffer = current_token_information(TokenUser, size_of::<TOKEN_USER>())?;
    let user = unsafe { (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let owner_buffer = current_token_information(TokenOwner, size_of::<TOKEN_OWNER>())?;
    let default_owner = unsafe { (*(owner_buffer.as_ptr().cast::<TOKEN_OWNER>())).Owner };
    let mut owner = null_mut();
    let mut descriptor = null_mut();
    status(unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    })?;
    let _descriptor = Allocation(descriptor);
    if !owner_is_current(owner, user, default_owner) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Private storage must be owned by the current Windows account",
        ));
    }
    // SetSecurityInfo propagates inheritable ACEs into existing children.
    // Capture and validate those objects before any permission change.
    let _retained = if directory {
        preflight_directory_tree(file)?
    } else {
        Vec::new()
    };
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: if directory {
            SUB_CONTAINERS_AND_OBJECTS_INHERIT
        } else {
            0
        },
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: user.cast(),
        },
    };
    let mut acl = null_mut();
    status(unsafe { SetEntriesInAclW(1, &entry, null(), &mut acl) })?;
    let _acl = Allocation(acl.cast());
    status(unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        )
    })
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn protect(_file: &File, _directory: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Owner-private storage is unavailable on this platform",
    ))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSidToSidW, GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo,
        EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID,
        TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        EqualSid, TokenOwner, TokenUser, ACL, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_OWNER, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ALL_ACCESS, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, WRITE_DAC,
    };

    fn open_directory(path: &Path) -> File {
        std::fs::OpenOptions::new()
            .read(true)
            .access_mode(FILE_GENERIC_READ | WRITE_DAC)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .unwrap()
    }

    fn acl_bytes(path: &Path) -> Vec<u8> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .unwrap();
        let mut acl: *mut ACL = null_mut();
        let mut descriptor = null_mut();
        assert_eq!(
            unsafe {
                GetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    &mut acl,
                    null_mut(),
                    &mut descriptor,
                )
            },
            0
        );
        assert!(!acl.is_null());
        let bytes = unsafe {
            std::slice::from_raw_parts(acl.cast::<u8>(), usize::from((*acl).AclSize)).to_vec()
        };
        unsafe { LocalFree(descriptor) };
        bytes
    }

    fn allow_everyone(path: &Path, access: u32) {
        let token =
            current_token_information(TokenUser, std::mem::size_of::<TOKEN_USER>()).unwrap();
        let user = unsafe { (*(token.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        let wide: Vec<u16> = "S-1-1-0\0".encode_utf16().collect();
        let mut everyone = null_mut();
        assert_ne!(
            unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut everyone) },
            0
        );
        let entries =
            [(user, FILE_ALL_ACCESS), (everyone, access)].map(|(sid, mask)| EXPLICIT_ACCESS_W {
                grfAccessPermissions: mask,
                grfAccessMode: SET_ACCESS,
                grfInheritance: 0,
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: null_mut(),
                    MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_USER,
                    ptstrName: sid.cast(),
                },
            });
        let mut acl = null_mut();
        assert_eq!(
            unsafe { SetEntriesInAclW(2, entries.as_ptr(), null(), &mut acl) },
            0
        );
        let file = open_directory(path);
        assert_eq!(
            unsafe {
                SetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    acl,
                    null(),
                )
            },
            0
        );
        unsafe {
            LocalFree(acl.cast());
            LocalFree(everyone);
        }
    }

    #[test]
    fn ownership_accepts_only_user_or_privileged_exact_default_owner() {
        let token =
            current_token_information(TokenUser, std::mem::size_of::<TOKEN_USER>()).unwrap();
        let user = unsafe { (*(token.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        let default_token =
            current_token_information(TokenOwner, std::mem::size_of::<TOKEN_OWNER>()).unwrap();
        let default_owner = unsafe { (*(default_token.as_ptr().cast::<TOKEN_OWNER>())).Owner };
        let foreign_sid: Vec<u16> = "S-1-5-21-987654321-123456789-777777777-4242\0"
            .encode_utf16()
            .collect();
        let mut foreign = std::ptr::null_mut();
        assert_ne!(
            unsafe { ConvertStringSidToSidW(foreign_sid.as_ptr(), &mut foreign) },
            0
        );
        assert_eq!(unsafe { EqualSid(foreign, user) }, 0);
        assert_eq!(unsafe { EqualSid(foreign, default_owner) }, 0);
        assert!(!owner_is_current(foreign, user, default_owner));
        assert!(!owner_is_current(foreign, user, foreign));
        assert!(!owner_is_current(std::ptr::null_mut(), user, default_owner));
        assert!(owner_is_current(user, user, default_owner));
        assert_eq!(
            owner_is_current(default_owner, user, default_owner),
            unsafe { EqualSid(default_owner, user) } != 0 || privileged_sid(default_owner)
        );
        unsafe {
            LocalFree(foreign);
        }
    }

    #[test]
    fn directory_migration_refuses_nested_foreign_writers_before_acl_changes() {
        let fixture = tempfile::tempdir().unwrap();
        let history = fixture.path().join("history");
        let nested = history.join("nested");
        std::fs::create_dir(&history).unwrap();
        protect(&open_directory(&history), true).unwrap();
        std::fs::create_dir(&nested).unwrap();
        let outside = fixture.path().join("outside-canary");
        std::fs::write(&outside, b"SYNTHETIC_OUTSIDE").unwrap();
        allow_everyone(&nested, FILE_GENERIC_WRITE);
        let before = [&history, &nested, &outside].map(|path| acl_bytes(path));
        let error = protect(&open_directory(&history), true).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("restrict that directory"));
        let after = [&history, &nested, &outside].map(|path| acl_bytes(path));
        assert_eq!(before, after);
        assert_eq!(std::fs::read(&outside).unwrap(), b"SYNTHETIC_OUTSIDE");
    }

    #[test]
    fn directory_migration_refuses_existing_hardlinks_without_touching_outside_acl() {
        let fixture = tempfile::tempdir().unwrap();
        let history = fixture.path().join("history");
        std::fs::create_dir(&history).unwrap();
        protect(&open_directory(&history), true).unwrap();
        let outside = fixture.path().join("outside-canary");
        std::fs::write(&outside, b"SYNTHETIC_OUTSIDE").unwrap();
        std::fs::hard_link(&outside, history.join("inserted-alias")).unwrap();
        let before = [&history, &outside].map(|path| acl_bytes(path));
        let error = protect(&open_directory(&history), true).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("hard-linked"));
        assert_eq!(before, [&history, &outside].map(|path| acl_bytes(path)));
        assert_eq!(std::fs::read(&outside).unwrap(), b"SYNTHETIC_OUTSIDE");
    }

    #[test]
    fn directory_migration_can_repair_foreign_read_only_acl() {
        let fixture = tempfile::tempdir().unwrap();
        let history = fixture.path().join("history");
        std::fs::create_dir(&history).unwrap();
        allow_everyone(&history, FILE_GENERIC_READ);
        protect(&open_directory(&history), true).unwrap();
        validate_directory_acl(&open_directory(&history)).unwrap();
    }
}
