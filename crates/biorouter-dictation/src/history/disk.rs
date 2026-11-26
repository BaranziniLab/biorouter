//! The history folder's lock, its private folder and files, its atomic rewrites and the
//! stamp a reader's cache is keyed by.
//!
//! These follow the dictionary store's own helpers (`store/lock.rs` and `Stamp` in
//! `store/mod.rs`) rule for rule: an exclusive `fs2` lock on `.lock`, the folder created
//! 0700 and every file opened 0600 at creation on Unix and never changed afterwards, so
//! no file is ever readable by another account, a rewrite through a temporary file that
//! is flushed and renamed over the old one, then the folder flushed. They live here
//! because the store's module keeps its helpers private to itself, and the history must
//! not depend on the dictionary's internals to stay private.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use super::schema::LOCK_FILE;

/// How often a bounded writer retries the lock.
const RETRY_EVERY: Duration = Duration::from_millis(5);

/// Held while a writer reads, changes and writes the history. Dropping it releases the
/// lock.
pub(crate) struct HistoryLock(File);

impl Drop for HistoryLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

/// Open `path` for writing, creating it 0600 on Unix when it is missing. An existing
/// file keeps its mode.
fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn open_lock_file(dir: &Path) -> io::Result<File> {
    create_dir_private(dir)?;
    let mut options = private_options();
    options.read(true).write(true).create(true).truncate(false);
    options.open(dir.join(LOCK_FILE))
}

impl HistoryLock {
    /// Wait for the lock. Rewrites (delete, clear, a tier change, a settings change) use
    /// this, as the dictionary's ordinary writes do.
    pub(crate) fn acquire(dir: &Path) -> io::Result<Self> {
        let file = open_lock_file(dir)?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(Self(file))
    }

    /// Try the lock every 5 ms until `budget` ends; `None` when it never came free. An
    /// append uses this and drops its entry on `None`, so a dictation never waits on the
    /// history.
    pub(crate) fn within(dir: &Path, budget: Duration) -> io::Result<Option<Self>> {
        let file = open_lock_file(dir)?;
        let contended = fs2::lock_contended_error().raw_os_error();
        let deadline = Instant::now() + budget;
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Some(Self(file))),
                Err(e)
                    if e.raw_os_error() == contended || e.kind() == io::ErrorKind::WouldBlock =>
                {
                    let now = Instant::now();
                    if now >= deadline {
                        return Ok(None);
                    }
                    std::thread::sleep(RETRY_EVERY.min(deadline - now));
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// Create the history folder (0700 on Unix) and any missing parent. An existing folder
/// keeps its mode.
pub(crate) fn create_dir_private(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Replace `path` with `bytes` atomically: a temporary file beside it (0600), written and
/// flushed to disk, renamed over it, and on Unix the folder flushed so the rename survives
/// a power loss. The caller holds the lock.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    let temporary = path.with_file_name(format!("{name}.tmp"));
    {
        let mut options = private_options();
        options.write(true).create(true).truncate(true);
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    rename_over(&temporary, path)?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Cut `path` back to its first `keep` bytes, the residue of a torn append removed. The
/// caller holds the lock.
pub(crate) fn truncate_to(path: &Path, keep: u64) -> io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(keep)?;
    file.sync_data()
}

/// Append `bytes` to `path` in one write, creating it 0600 when it is missing, and flush
/// the data to disk. The caller holds the lock and has repaired a torn last line.
pub(crate) fn append_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = private_options();
    options.append(true).create(true);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_data()
}

/// `rename`, retried on Windows up to 10 times over 200 ms while another process holds
/// the target open (an antivirus scan, a reader mid parse).
fn rename_over(from: &Path, to: &Path) -> io::Result<()> {
    let attempts = if cfg!(windows) { 10 } else { 1 };
    let mut last = None;
    for attempt in 0..attempts {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                if attempt + 1 < attempts {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
    }
    let _ = fs::remove_file(from);
    Err(last.unwrap_or_else(|| io::Error::other("rename failed")))
}

/// What identifies one version of a file without reading it: modification time, length,
/// the file's identity (the inode on Unix, the NTFS file index on Windows) and on Unix the
/// change time, which every write sets and no call sets back. A rewrite renames a new file
/// into place, so its identity changes even when its time and length do not; an append
/// moves the length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stamp {
    modified: Option<SystemTime>,
    pub(crate) len: u64,
    identity: u64,
    changed: Option<(i64, i64)>,
}

impl Stamp {
    fn with_identity(metadata: &fs::Metadata, identity: u64) -> Self {
        #[cfg(unix)]
        let changed = {
            use std::os::unix::fs::MetadataExt;
            Some((metadata.ctime(), metadata.ctime_nsec()))
        };
        #[cfg(not(unix))]
        let changed = None;
        Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
            identity,
            changed,
        }
    }

    /// The stamp of the version `file` has open.
    fn of(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            metadata.ino()
        };
        #[cfg(windows)]
        let identity = windows_file_index(file)?;
        #[cfg(not(any(unix, windows)))]
        let identity = 0;
        Ok(Self::with_identity(&metadata, identity))
    }

    /// `None` for a file that does not exist.
    pub(crate) fn at(path: &Path) -> io::Result<Option<Self>> {
        #[cfg(unix)]
        let stamp = fs::metadata(path).map(|metadata| {
            use std::os::unix::fs::MetadataExt;
            Self::with_identity(&metadata, metadata.ino())
        });
        #[cfg(not(unix))]
        let stamp = File::open(path).and_then(|file| Self::of(&file));
        match stamp {
            Ok(stamp) => Ok(Some(stamp)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// The NTFS file index of an open file, which a new file gets fresh and keeps across a
/// rename.
#[cfg(windows)]
fn windows_file_index(file: &File) -> io::Result<u64> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    // SAFETY: an all zero BY_HANDLE_FILE_INFORMATION is a valid value of the plain data
    // struct the call fills in.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: the handle is open for as long as `file` is borrowed, and `info` is a
    // valid, writable BY_HANDLE_FILE_INFORMATION.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
}

/// Read a file and the stamp of the version read, from one open handle, keeping only the
/// bytes the stamp counts: a reader that raced an append sees the file as it was when it
/// was stamped, so the value it caches is the one that stamp stands for. `None` when the
/// file does not exist.
pub(crate) fn read_stamped(path: &Path) -> io::Result<Option<(Vec<u8>, Stamp)>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let stamp = Stamp::of(&file)?;
    let mut bytes = Vec::with_capacity(stamp.len as usize);
    (&mut file).take(stamp.len).read_to_end(&mut bytes)?;
    Ok(Some((bytes, stamp)))
}

/// Read a small file whole. `None` when it does not exist.
pub(crate) fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bounded_writer_gives_up_while_the_lock_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let held = HistoryLock::acquire(dir.path()).unwrap();
        let second = HistoryLock::within(dir.path(), Duration::from_millis(30)).unwrap();
        assert!(second.is_none(), "the lock is held");
        drop(held);
        assert!(HistoryLock::within(dir.path(), Duration::from_millis(30))
            .unwrap()
            .is_some());
    }

    #[test]
    fn a_read_keeps_only_the_bytes_its_stamp_counts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.jsonl");
        append_bytes(&path, b"one\n").unwrap();
        let (bytes, stamp) = read_stamped(&path).unwrap().unwrap();
        assert_eq!(bytes, b"one\n");
        assert_eq!(stamp.len, 4);
        append_bytes(&path, b"two\n").unwrap();
        assert_ne!(Stamp::at(&path).unwrap(), Some(stamp), "an append moves it");
        truncate_to(&path, 4).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one\n");
    }
}
