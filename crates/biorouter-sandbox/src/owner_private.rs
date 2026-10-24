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
                && unsafe { EqualSid(owner, default_owner) } != 0))
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
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::{EqualSid, TokenOwner, TokenUser, TOKEN_OWNER, TOKEN_USER};

    #[test]
    fn ownership_accepts_only_the_process_user_or_its_exact_default_owner() {
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
        assert!(!owner_is_current(std::ptr::null_mut(), user, default_owner));
        assert!(owner_is_current(user, user, default_owner));
        assert!(owner_is_current(default_owner, user, default_owner));
        unsafe {
            LocalFree(foreign);
        }
    }
}
