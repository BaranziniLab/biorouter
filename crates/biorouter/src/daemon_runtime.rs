//! Private discovery records for a single daemon serving one local profile.
use crate::config::paths::Paths;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;
pub const SHARED_ENV: &str = "BIOROUTER_SHARED_DAEMON";
const MAX_RECORD: u64 = 16 * 1024;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileIdentity {
    pub version: u32,
    pub profile_id: String,
    pub config_dir: PathBuf,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Endpoint {
    Unix { path: PathBuf },
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub version: u32,
    pub profile_id: String,
    pub instance_id: String,
    pub pid: u32,
    pub endpoint: Endpoint,
    pub api_secret: String,
    pub user_action_installed: bool,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub version: u32,
    pub profile_id: String,
    pub instance_id: String,
    pub pid: u32,
    pub user_action_installed: bool,
}

impl Descriptor {
    pub fn identity(&self) -> Identity {
        Identity {
            version: self.version,
            profile_id: self.profile_id.clone(),
            instance_id: self.instance_id.clone(),
            pid: self.pid,
            user_action_installed: self.user_action_installed,
        }
    }
}

pub fn runtime_directory() -> PathBuf {
    Paths::state_dir().join("daemon")
}
pub fn descriptor_path() -> PathBuf {
    runtime_directory().join("runtime.json")
}

pub fn private_directory(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "Daemon profile paths must be absolute");
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "Private daemon directory must be a real directory: {}",
                path.display()
            );
            check_owner(&metadata)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                ensure!(
                    metadata.permissions().mode() & 0o077 == 0,
                    "Private daemon directory must have mode 0700: {}",
                    path.display()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().context("Daemon directory has no parent")?;
            std::fs::create_dir_all(parent)?;
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(path) {
                Ok(()) => (),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return private_directory(path)
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn check_owner(metadata: &std::fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            "Daemon discovery belongs to another user"
        );
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}

pub fn read_private<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_RECORD,
        "Daemon discovery must be a bounded regular file"
    );
    check_owner(&metadata)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            metadata.permissions().mode() & 0o077 == 0 && metadata.nlink() == 1,
            "Daemon discovery must be private and have one link"
        );
    }
    let mut bytes = Vec::new();
    (&mut file).take(MAX_RECORD + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_RECORD,
        "Daemon discovery exceeds its limit"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn write_private<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("Private record has no parent")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut temporary, value)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn lock_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "Daemon lock is not a regular file");
    check_owner(&metadata)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            metadata.nlink() == 1 && metadata.permissions().mode() & 0o077 == 0,
            "Daemon lock must be private and have one link"
        );
    }
    Ok(file)
}

pub fn profile_record() -> Result<ProfileIdentity> {
    let config = Paths::config_dir();
    std::fs::create_dir_all(&config)?;
    let config = std::fs::canonicalize(config)?;
    let lock = lock_file(&config.join("daemon-profile.lock"))?;
    fs2::FileExt::lock_exclusive(&lock)?;
    let path = config.join("daemon-profile.json");
    let profile = if path.try_exists()? {
        read_private::<ProfileIdentity>(&path)?
    } else {
        // A dangling symlink is an invalid existing record, not permission to replace it.
        ensure!(
            std::fs::symlink_metadata(&path).is_err(),
            "Invalid existing daemon profile identity"
        );
        let profile = ProfileIdentity {
            version: VERSION,
            profile_id: uuid::Uuid::new_v4().to_string(),
            config_dir: config.clone(),
        };
        write_private(&path, &profile)?;
        profile
    };
    ensure!(
        profile.version == VERSION
            && profile.config_dir == config
            && uuid::Uuid::parse_str(&profile.profile_id).is_ok(),
        "Daemon profile identity does not match this profile"
    );
    Ok(profile)
}

pub fn profile_identity() -> Result<String> {
    Ok(profile_record()?.profile_id)
}

pub fn read_descriptor() -> Result<Descriptor> {
    private_directory(&runtime_directory())?;
    let descriptor: Descriptor = read_private(&descriptor_path())?;
    ensure!(
        descriptor.version == VERSION
            && descriptor.profile_id == profile_identity()?
            && uuid::Uuid::parse_str(&descriptor.instance_id).is_ok()
            && descriptor.pid > 0
            && descriptor.api_secret.len() >= 32,
        "Daemon discovery does not match this profile or protocol"
    );
    let Endpoint::Unix { path } = &descriptor.endpoint;
    ensure!(
        path == &runtime_directory().join("daemon.sock"),
        "Daemon socket is outside this profile's private runtime directory"
    );
    Ok(descriptor)
}

pub struct RuntimeOwner {
    pub descriptor: Descriptor,
    _lock: File,
}

impl RuntimeOwner {
    pub fn acquire(api_secret: String, user_action_installed: bool) -> Result<Self> {
        #[cfg(not(unix))]
        anyhow::bail!("Shared Crew daemon IPC is not yet available on this platform");
        #[cfg(unix)]
        {
            private_directory(&runtime_directory())?;
            let lock = lock_file(&runtime_directory().join("owner.lock"))?;
            fs2::FileExt::try_lock_exclusive(&lock)
                .context("Another daemon already owns this profile; connect to it instead")?;
            let descriptor = Descriptor {
                version: VERSION,
                profile_id: profile_identity()?,
                instance_id: uuid::Uuid::new_v4().to_string(),
                pid: std::process::id(),
                endpoint: Endpoint::Unix {
                    path: runtime_directory().join("daemon.sock"),
                },
                api_secret,
                user_action_installed,
            };
            Ok(Self {
                descriptor,
                _lock: lock,
            })
        }
    }

    #[cfg(unix)]
    pub fn bind(&self) -> Result<tokio::net::UnixListener> {
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        let Endpoint::Unix { path } = &self.descriptor.endpoint;
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                ensure!(
                    metadata.file_type().is_socket(),
                    "Refusing to replace a non-socket daemon endpoint"
                );
                check_owner(&metadata)?;
                std::fs::remove_file(path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        let listener = tokio::net::UnixListener::bind(path).context("Cannot bind private daemon socket (the profile path may exceed the Unix socket length limit)")?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }

    pub fn publish(&self) -> Result<()> {
        write_private(&descriptor_path(), &self.descriptor)
    }
}

impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        if read_private::<Descriptor>(&descriptor_path())
            .is_ok_and(|d| d.instance_id == self.descriptor.instance_id)
        {
            let _ = std::fs::remove_file(descriptor_path());
            let Endpoint::Unix { path } = &self.descriptor.endpoint;
            let _ = std::fs::remove_file(path);
        }
    }
}
