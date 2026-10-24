use std::{fs, path::Path};
fn protect_session_directory(path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

        let directory = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(path)?;
        let metadata = directory.metadata()?;
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "session directory must be a directory owned by the current user",
            ));
        }
        directory.set_permissions(fs::Permissions::from_mode(0o700))
    }
fn main() { let path = std::env::args_os().nth(1).expect("directory"); if let Err(error) = protect_session_directory(Path::new(&path)) {eprintln!("{error}"); std::process::exit(2); }}
