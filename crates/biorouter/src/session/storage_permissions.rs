use std::fs::{self, File, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    GetSecurityInfo, SetEntriesInAclW, SetSecurityInfo, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE,
    SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    EqualSid, GetTokenInformation, IsValidSid, TokenUser, DACL_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSID,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ALL_ACCESS,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL, WRITE_DAC,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

struct LocalAllocation(*mut std::ffi::c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

fn status(code: u32) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

fn current_user() -> io::Result<Vec<usize>> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
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
    if !(size_of::<TOKEN_USER>() as u32..=65536).contains(&size) {
        return Err(io::Error::other("invalid current-user token size"));
    }
    let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle().cast(),
            TokenUser,
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

fn open_owned(path: &Path, user: PSID) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .access_mode(READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "session storage must not contain junctions, symlinks, or other reparse points",
        ));
    }
    if !metadata.is_dir() {
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "session storage must contain only regular files and directories",
            ));
        }
        let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        if unsafe {
            GetFileInformationByHandle(file.as_raw_handle().cast(), information.as_mut_ptr())
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if unsafe { information.assume_init() }.nNumberOfLinks != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "session storage files must not have hard-link aliases",
            ));
        }
    }
    let mut owner = null_mut();
    let mut descriptor = null_mut();
    status(unsafe {
        GetSecurityInfo(
            file.as_raw_handle().cast(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    })?;
    let _descriptor = LocalAllocation(descriptor);
    if owner.is_null() || unsafe { IsValidSid(owner) } == 0 || unsafe { EqualSid(owner, user) } == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "session storage must be owned by the current Windows account",
        ));
    }
    Ok(file)
}

fn restrict(file: &File, user: PSID) -> io::Result<()> {
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
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
    let _acl = LocalAllocation(acl.cast());
    status(unsafe {
        SetSecurityInfo(
            file.as_raw_handle().cast(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        )
    })
}

fn owned_tree(path: &Path, user: PSID, directory: bool, files: &mut Vec<File>) -> io::Result<()> {
    let file = open_owned(path, user)?;
    if directory && !file.metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "session storage must be a directory",
        ));
    }
    let is_directory = file.metadata()?.is_dir();
    files.push(file);
    if is_directory {
        for entry in fs::read_dir(path)? {
            owned_tree(&entry?.path(), user, false, files)?;
        }
    }
    Ok(())
}

pub(super) fn protect(path: &Path) -> io::Result<()> {
    let token = current_user()?;
    let user = unsafe { (*(token.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    for name in ["sessions.db", "sessions.db-wal", "sessions.db-shm"] {
        let file = match open_owned(&path.join(name), user) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "session database and sidecars must be regular files",
            ));
        }
    }
    // Windows normally permits bypassing directory traversal checks. Repair
    // existing child ACLs as well; a private parent alone does not protect them.
    let mut files = Vec::new();
    owned_tree(path, user, true, &mut files)?;
    for file in &files {
        restrict(file, user)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Security::{GetAce, ACCESS_ALLOWED_ACE, ACL};

    fn assert_current_user_only(path: &Path) {
        let token = current_user().unwrap();
        let user = unsafe { (*(token.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        let file = open_owned(path, user).unwrap();
        let mut acl: *mut ACL = null_mut();
        let mut descriptor = null_mut();
        status(unsafe {
            GetSecurityInfo(
                file.as_raw_handle().cast(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut acl,
                null_mut(),
                &mut descriptor,
            )
        })
        .unwrap();
        let _descriptor = LocalAllocation(descriptor);
        assert!(!acl.is_null());
        assert_eq!(unsafe { (*acl).AceCount }, 1);
        let mut entry = null_mut();
        assert_ne!(unsafe { GetAce(acl, 0, &mut entry) }, 0);
        let entry = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
        assert_eq!(entry.Mask, FILE_ALL_ACCESS);
        assert_ne!(
            unsafe { EqualSid((&entry.SidStart as *const u32).cast_mut().cast(), user) },
            0
        );
    }

    #[test]
    fn repairs_existing_history_and_sidecars_and_protects_new_children() {
        let fixture = tempfile::tempdir().unwrap();
        let sessions = fixture.path().join("sessions");
        fs::create_dir(&sessions).unwrap();
        for name in ["sessions.db", "sessions.db-wal", "sessions.db-shm"] {
            fs::write(sessions.join(name), b"SYNTHETIC_HISTORY").unwrap();
        }
        let nested = sessions.join("legacy");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("session.json"), b"SYNTHETIC_LEGACY").unwrap();
        protect(&sessions).unwrap();
        for path in [
            sessions.clone(),
            sessions.join("sessions.db"),
            sessions.join("sessions.db-wal"),
            sessions.join("sessions.db-shm"),
            nested.join("session.json"),
        ] {
            assert_current_user_only(&path);
        }
        fs::write(sessions.join("new-history"), b"SYNTHETIC_NEW").unwrap();
        assert_current_user_only(&sessions.join("new-history"));
    }
}
