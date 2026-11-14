//! The store's lock and its atomic writes (DD-33), memory's pattern
//! (`biorouter-mcp/src/memory/mod.rs` `StoreLock`): an exclusive `fs2` lock on `.lock`,
//! a fresh read under it, a temporary file that is flushed to disk and renamed over the
//! old one, then the folder flushed on Unix. Readers take no lock: a rename is atomic,
//! so a reader sees the old file or the new one, never half of either.
//!
//! The folder is created 0700 and the files 0600 on Unix, because private rows may sit
//! there.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use super::schema::LOCK_FILE;
use super::Stamp;

/// How often a bounded writer retries the lock.
const RETRY_EVERY: Duration = Duration::from_millis(5);

/// Held while a writer reads, changes and writes the store. Dropping it releases the
/// lock.
pub(crate) struct StoreLock(File);

impl Drop for StoreLock {
    fn drop(&mut self) {
        // Closing the descriptor would release it too; releasing it here means the
        // release does not wait for the `File` to drop.
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn open_lock_file(dir: &Path) -> io::Result<File> {
    create_dir_private(dir)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(dir.join(LOCK_FILE))
}

impl StoreLock {
    /// Wait for the lock. The daemon's and the CLI's ordinary writes use this.
    pub(crate) fn acquire(dir: &Path) -> io::Result<Self> {
        let file = open_lock_file(dir)?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(Self(file))
    }

    /// Try the lock every 5 ms until `budget` ends; `None` when it never came free. The
    /// extractor's writes and the usage flush use this, and drop their write on `None`.
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

/// Create the store folder (0700 on Unix) if it is missing. An existing folder keeps
/// its mode.
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

/// Renames `temporary`, written and closed, over `path`, and answers the stamp `path` then
/// has, so the cache can be keyed by it ([`Stamp`]). The stamp is read through a handle
/// opened on the temporary file after the handle that wrote it closed, because Windows may
/// update the last write time only then, and that handle names this very file: a stamp
/// taken from the path after the rename could be some other writer's file. A rename keeps
/// the identity, the length and the modification time, but on Unix it sets the change time
/// (measured on APFS on 2026-11-07; Linux file systems set it too), so there the handle is
/// asked after the rename. Elsewhere the stamp holds no change time, and the handle is
/// asked and closed before the rename, as it always was there.
///
/// Best effort: the stamp only spares the next read a parse, so a handle that cannot be
/// opened (on Windows, a scanner or indexer that opened the new file without sharing it
/// for reading) answers `None` and the rename goes ahead. The caller then leaves its cache
/// empty and the next read parses the file.
fn rename_stamped(temporary: &Path, path: &Path) -> io::Result<Option<Stamp>> {
    let handle = File::open(temporary).ok();
    #[cfg(unix)]
    {
        rename_over(temporary, path)?;
        Ok(handle.and_then(|file| Stamp::of(&file).ok()))
    }
    #[cfg(not(unix))]
    {
        let stamp = handle.and_then(|file| Stamp::of(&file).ok());
        rename_over(temporary, path)?;
        Ok(stamp)
    }
}

/// Replace `path` with `bytes` atomically: a temporary file beside it, written and
/// flushed to disk, renamed over it, and on Unix the folder flushed so the rename
/// survives a power loss. The caller holds the lock. Answers the stamp of the file now at
/// `path`, or `None` when it could not be taken ([`rename_stamped`]).
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<Option<Stamp>> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    let temporary = path.with_file_name(format!("{name}.tmp"));
    {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    let stamp = rename_stamped(&temporary, path)?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(stamp)
}

/// Replace `path` with `bytes` by the same temporary file and rename as
/// [`write_atomically`], with no flush to disk. A reader still sees the old file or the new
/// one, never half of either; only a power cut can lose the write, or leave the file empty.
/// For `usage.json` alone, whose counters are loss tolerant ("Usage"): the two flushes cost
/// a one shot `biorouter transcribe` milliseconds between its text and its exit (on macOS
/// each is an `F_FULLFSYNC`). The caller holds the lock. Answers the stamp of the file now
/// at `path`, or `None` when it could not be taken ([`rename_stamped`]).
pub(crate) fn replace_unsynced(path: &Path, bytes: &[u8]) -> io::Result<Option<Stamp>> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    let temporary = path.with_file_name(format!("{name}.tmp"));
    {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&temporary)?.write_all(bytes)?;
    }
    rename_stamped(&temporary, path)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bounded_writer_gives_up_while_the_lock_is_held() {
        let dir = tempfile::tempdir().unwrap();
        let held = StoreLock::acquire(dir.path()).unwrap();
        let started = Instant::now();
        let second = StoreLock::within(dir.path(), Duration::from_millis(50)).unwrap();
        assert!(second.is_none(), "the lock is held");
        assert!(started.elapsed() >= Duration::from_millis(50));
        drop(held);
        assert!(StoreLock::within(dir.path(), Duration::from_millis(50))
            .unwrap()
            .is_some());
    }

    #[test]
    fn writes_replace_the_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dictionary.json");
        write_atomically(&path, b"first").unwrap();
        write_atomically(&path, b"second, longer").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second, longer");
        assert!(!dir.path().join("dictionary.json.tmp").exists());
        // The usage file's replace: the same rename, no flush to disk.
        let usage = dir.path().join("usage.json");
        replace_unsynced(&usage, b"a longer first").unwrap();
        replace_unsynced(&usage, b"second").unwrap();
        assert_eq!(fs::read(&usage).unwrap(), b"second");
        assert!(!dir.path().join("usage.json.tmp").exists());
    }

    /// The stamp is a cache's convenience, never a condition of the write: a temporary file
    /// no handle can open gives `None`, and the rename still lands. A file the account may
    /// not read stands in for a Windows scanner that holds it unshared.
    #[cfg(unix)]
    #[test]
    fn a_stamp_that_cannot_be_taken_is_none_and_keeps_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dictionary.json");
        let temporary = dir.path().join("dictionary.json.tmp");
        fs::write(&temporary, b"{}").unwrap();
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o000)).unwrap();
        if File::open(&temporary).is_ok() {
            // Run as root, which opens any file: nothing to show.
            return;
        }
        assert_eq!(rename_stamped(&temporary, &path).unwrap(), None);
        assert!(path.exists(), "the rename landed");
        assert!(!temporary.exists());
    }

    /// The stamp a write answers is the stamp its path has once the rename landed, the
    /// change time included, which the rename itself sets on Unix: the cache keyed by it
    /// matches the next read's `stat`, so that read parses nothing.
    #[test]
    fn the_stamp_a_write_answers_is_the_one_its_path_then_has() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dictionary.json");
        for bytes in [&b"first"[..], b"second, longer", b"third"] {
            let written = write_atomically(&path, bytes).unwrap();
            assert_eq!(written, Stamp::at(&path).unwrap(), "{bytes:?}");
            let usage = dir.path().join("usage.json");
            let replaced = replace_unsynced(&usage, bytes).unwrap();
            assert_eq!(replaced, Stamp::at(&usage).unwrap(), "{bytes:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_folder_and_its_files_are_private_to_the_account() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("config").join("dictionary");
        let _lock = StoreLock::acquire(&dir).unwrap();
        write_atomically(&dir.join("dictionary.json"), b"{}").unwrap();
        replace_unsynced(&dir.join("usage.json"), b"{}").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("dictionary.json")), 0o600);
        assert_eq!(mode(&dir.join("usage.json")), 0o600);
        assert_eq!(mode(&dir.join(LOCK_FILE)), 0o600);
    }
}
