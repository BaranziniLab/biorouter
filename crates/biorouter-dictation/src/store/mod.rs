//! The personal dictionary store (`docs/dictation/dictionary.md`, "The store", DD-33 to
//! DD-43): one file of terms per account, `<config>/dictionary/dictionary.json`, beside
//! `usage.json` and `.lock`. The daemon, the CLI and the agent's `dictionary`
//! capability all read and write it through [`DictionaryStore`], under one lock.
//!
//! * [`schema`]: the v1 file, snake_case on disk.
//! * [`validate`]: the one validator every writer calls.
//! * [`fold`]: fold keys, for tombstones and the "same letters" warning.
//! * [`ops`]: what each write does, with the tier, tombstone and cap rules.
//! * [`observe`]: what a correction the person made to dictated text does (PR-16).
//! * [`tier_user`]: tier changes, behind the person's proof.
//! * [`transfer`]: import and export.
//! * [`usage`]: the usage counters, in their own file.
//!
//! Writers take the lock, read both files fresh, change, and replace the file
//! atomically ([`lock`]). Readers take no lock and cache each file's parse by its
//! modification time, length, identity and, on Unix, change time, so a term one process
//! adds is listed by another at once. A write leaves its handle's cache holding what it wrote, keyed by the
//! stamp of the file it renamed into place, so the next read of that handle parses
//! nothing; a usage bump reparses only `usage.json`. A file this version cannot read fails
//! closed: listings are empty, dictation runs with no terms, every write is refused with
//! MSG-DICTIONARY-FILE, and the file is never rewritten from a failed parse.
//!
//! A leaf crate: the store holds no absolute path and takes its folder as an argument.
//! The core resolves it (`Paths::in_config_dir("dictionary")`).

mod lock;

pub mod fold;
pub mod learning;
pub mod observe;
pub mod ops;
pub mod schema;
pub mod tier_user;
pub mod transfer;
pub mod usage;
pub mod validate;

use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

pub use fold::fold_key;
pub use learning::ChatLearning;
pub use observe::{Observation, ObservationKind, ObservationStamp, Observed};
pub use ops::{AddOutcome, EntryEdit, NewTerm, Reach};
pub use schema::{
    AgentWrites, BiasPolicy, Entry, EntryId, EntrySource, EntryStatus, EntryTier, MatchPolicy,
    Misrecognition, Provenance, ProvenanceReason, RowAffiliation, Sighting, StoreFile,
    StoreSettings, TermKind, Tombstone,
};
pub use tier_user::UserDictionaryTierChange;
pub use transfer::{ImportFormat, ImportPlan, ImportRow, RowOutcome};
pub use usage::{UsageDelta, UsageFile, UsageRecord, UsageScope, USAGE_FLUSH_BUDGET};
pub use validate::ValidationCode;

use lock::{replace_unsynced, write_atomically, StoreLock};
use schema::{STORE_FILE, USAGE_FILE};

/// MSG-NOT-FOUND.
pub const MSG_NOT_FOUND: &str = "No dictionary entry with that id.";
/// MSG-DICTIONARY-CAP.
pub const MSG_DICTIONARY_CAP: &str = "Your dictionary holds 5,000 active terms here, the most dictation can use. Delete terms you no longer dictate, then add this one.";
/// MSG-DICTIONARY-STALE.
pub const MSG_DICTIONARY_STALE: &str =
    "This term changed since you opened it. Reload it and try again.";

/// MSG-DICTIONARY-DUPLICATE: an edit to a spelling another row of its tier has.
pub const MSG_DICTIONARY_DUPLICATE: &str =
    "That spelling is already a term here, so the edit was not made.";

/// MSG-DICTIONARY-FILE.
pub fn msg_dictionary_file(path: &Path) -> String {
    format!(
        "The dictionary file at {} could not be read, so nothing was changed. Fix or remove it, then try again.",
        path.display()
    )
}

/// Which cap a write met (DD-41).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapKind {
    Active,
    Suggestions,
    UnreviewedAgent,
}

/// Why a store operation did not happen.
#[derive(Debug)]
pub enum StoreError {
    /// A file this version cannot read (MSG-DICTIONARY-FILE). Nothing was written.
    Unreadable {
        path: PathBuf,
    },
    Invalid(ValidationCode),
    /// The row changed since the caller read it (`If-Match`).
    Stale {
        current_rev: u64,
    },
    /// No such row, also for a row the caller may not list (oracle rule 2).
    NotFound,
    Cap(CapKind),
    /// An edit would give two rows of one tier the same written form.
    Duplicate,
    Io(io::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable { path } => f.write_str(&msg_dictionary_file(path)),
            Self::Invalid(code) => f.write_str(code.message()),
            Self::Stale { .. } => f.write_str(MSG_DICTIONARY_STALE),
            Self::NotFound => f.write_str(MSG_NOT_FOUND),
            Self::Cap(_) => f.write_str(MSG_DICTIONARY_CAP),
            Self::Duplicate => f.write_str(MSG_DICTIONARY_DUPLICATE),
            Self::Io(e) => write!(f, "The dictionary could not be written: {e}."),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<ValidationCode> for StoreError {
    fn from(code: ValidationCode) -> Self {
        Self::Invalid(code)
    }
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A parsed store, as of one read.
#[derive(Debug)]
pub struct StoreView {
    file: Arc<StoreFile>,
    usage: Arc<UsageFile>,
    content_stamp: u64,
    usage_stamp: u64,
}

impl StoreView {
    pub fn file(&self) -> &StoreFile {
        &self.file
    }

    pub fn settings(&self) -> &StoreSettings {
        &self.file.settings
    }

    pub fn entries(&self) -> &[Entry] {
        &self.file.entries
    }

    pub fn find(&self, id: &EntryId) -> Option<&Entry> {
        self.file.find(id)
    }

    /// A row's counters in the map `scope` reads: `All` for a caller with private
    /// standing, `Public` for a public scope ranking (oracle rule 11).
    pub fn usage(&self, id: &EntryId, scope: UsageScope) -> Option<&UsageRecord> {
        match scope {
            UsageScope::All => self.usage.all.get(id.as_str()),
            UsageScope::Public => self.usage.public.get(id.as_str()),
        }
    }

    /// Which version of `dictionary.json` this view holds: a digest of the file's stamp
    /// (modification time, length, identity and, on Unix, change time), 0 for a store with
    /// no file. Every view of
    /// an unchanged file has the same one, in every handle and whatever `usage.json` does,
    /// and every write gives a new one, because a write renames a new file into place. The
    /// bonus table's revision is keyed on it, so an unchanged store sends the worker no new
    /// table while every change, in either tier, rebuilds every table (oracle rule 9).
    /// Never exposed by a route (DD-40).
    pub fn content_stamp(&self) -> u64 {
        self.content_stamp
    }

    /// Which version of `usage.json` this view holds, in the same way: a digest of its
    /// stamp, 0 with no file. A daemon that keeps what its dictations built from an
    /// unchanged store (`DictionaryCache` in `biorouter::dictation`) reads it to tell when
    /// only the counters moved.
    pub fn usage_stamp(&self) -> u64 {
        self.usage_stamp
    }

    /// A view of `file` that no store read, with no usage and content stamp 0, so a caller
    /// can ask the bias set's own rules about a few rows (whether a private row's
    /// institutions admit a chat's model, for an observation) without a second spelling of
    /// them. It is never cached, never a store's view and never what a dictation reads.
    pub fn detached(file: StoreFile) -> Self {
        Self {
            file: Arc::new(file),
            usage: Arc::new(UsageFile::default()),
            content_stamp: 0,
            usage_stamp: 0,
        }
    }
}

/// What identifies one version of a file without reading it: modification time,
/// length, the file's identity (the inode on Unix and the NTFS file index on Windows), and
/// on Unix the inode change time. A write renames a new file into place, so the identity
/// changes even when the time and the length do not. The creation time would not do on
/// Windows: NTFS tunnelling gives a file created or renamed onto a name removed within the
/// last 15 s that file's creation time, so every write within 15 s would carry the same
/// one.
///
/// The change time is there for an edit made in place, on the same inode, that keeps the
/// length and sets the modification time back (`touch -r`, `rsync -t`, a restore that keeps
/// both): the other three fields then match the version cached before it. The kernel sets
/// the change time to its clock at every write to the file and every change of its times,
/// and no call sets it back, so it is the one field such an edit cannot keep. It is read by
/// the same `stat` as the rest, so it costs nothing. A change of mode or owner moves it
/// too, and costs one parse. Windows keeps no such field in what is read here, so there an
/// edit of that kind is seen at the next write (the PERF-1 review, 2026-11-07).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
    identity: u64,
    /// Seconds and nanoseconds; `None` where it is not read.
    changed: Option<(i64, i64)>,
}

impl Stamp {
    /// A number for this stamp, stable within the process ([`StoreView::content_stamp`]).
    /// Never 0, which stands for no file.
    fn digest(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish().max(1)
    }

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

    /// `None` for a file that does not exist. On Unix one `stat`; on Windows the file
    /// index needs an open handle, which `fs::metadata` usually opens there too.
    fn at(path: &Path) -> io::Result<Option<Self>> {
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

/// The NTFS file index of an open file (`nFileIndexHigh` and `nFileIndexLow`), which a
/// new file gets fresh and keeps across a rename. A reused record carries a new sequence
/// number in the high bits, so the index of a file removed and the next one created never
/// match.
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

/// Read a file and the stamp of the version read, from one open handle. `None` when it
/// does not exist.
fn read_stamped(path: &Path) -> io::Result<Option<(Vec<u8>, Stamp)>> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let stamp = Stamp::of(&file)?;
    let mut bytes = Vec::with_capacity(stamp.len as usize);
    file.read_to_end(&mut bytes)?;
    Ok(Some((bytes, stamp)))
}

/// One file's parse, keyed by the stamp of the version it was read from or written as;
/// a stamp of `None` for a file that does not exist.
#[derive(Debug)]
struct Slot<T> {
    stamp: Option<Stamp>,
    value: Arc<T>,
}

/// What a handle has parsed. Each file has its own slot, keyed by its own stamp, so a
/// usage bump (after every dictation that wrote a term) reparses only `usage.json`, never
/// the entries. `view` is the two put together, kept until either slot changes.
///
/// The cache stays honest because a slot's stamp is always the stamp of the very version
/// its value holds: the one read, or for a write the file this handle renamed into place,
/// stamped through a handle on that very file (`rename_stamped` in `lock.rs`). Any other
/// writer's rename puts a new file, with a new identity, at the path, and any edit in place
/// moves the change time, so the next read sees a stamp no slot holds.
#[derive(Debug, Default)]
struct Cache {
    store: Option<Slot<StoreFile>>,
    usage: Option<Slot<UsageFile>>,
    view: Option<Arc<StoreView>>,
}

impl Cache {
    fn store_is(&self, stamp: &Option<Stamp>) -> bool {
        self.store.as_ref().is_some_and(|slot| slot.stamp == *stamp)
    }

    fn usage_is(&self, stamp: &Option<Stamp>) -> bool {
        self.usage.as_ref().is_some_and(|slot| slot.stamp == *stamp)
    }

    /// The view, when both files are still the versions it holds.
    fn current(&self, store: &Option<Stamp>, usage: &Option<Stamp>) -> Option<Arc<StoreView>> {
        if self.store_is(store) && self.usage_is(usage) {
            self.view.clone()
        } else {
            None
        }
    }

    /// Keep `slot` as the entries file's parse. A slot for the version already held changes
    /// nothing, so views handed out before stay the current one.
    fn set_store(&mut self, slot: Slot<StoreFile>) {
        if !self.store_is(&slot.stamp) {
            self.store = Some(slot);
            self.view = None;
        }
    }

    fn set_usage(&mut self, slot: Slot<UsageFile>) {
        if !self.usage_is(&slot.stamp) {
            self.usage = Some(slot);
            self.view = None;
        }
    }

    /// Keep the parse a write just made, when it knows the stamp of the file it renamed into
    /// place, and otherwise hold nothing for that file, so the next read parses it. A slot
    /// keyed by a stamp it does not hold would make the cache lie.
    fn seed_store(&mut self, stamp: Option<Option<Stamp>>, value: StoreFile) {
        match stamp {
            Some(stamp) => self.set_store(Slot {
                stamp,
                value: Arc::new(value),
            }),
            None => {
                self.store = None;
                self.view = None;
            }
        }
    }

    /// [`Self::seed_store`] for `usage.json`.
    fn seed_usage(&mut self, stamp: Option<Stamp>, value: UsageFile) {
        match stamp {
            Some(stamp) => self.set_usage(Slot {
                stamp: Some(stamp),
                value: Arc::new(value),
            }),
            None => {
                self.usage = None;
                self.view = None;
            }
        }
    }

    /// The view of the two slots, built once per pair; `None` while either is missing.
    fn assemble(&mut self) -> Option<Arc<StoreView>> {
        if self.view.is_none() {
            let (store, usage) = (self.store.as_ref()?, self.usage.as_ref()?);
            self.view = Some(Arc::new(StoreView {
                file: Arc::clone(&store.value),
                usage: Arc::clone(&usage.value),
                content_stamp: store.stamp.as_ref().map_or(0, Stamp::digest),
                usage_stamp: usage.stamp.as_ref().map_or(0, Stamp::digest),
            }));
        }
        self.view.clone()
    }
}

/// One account's dictionary.
#[derive(Debug)]
pub struct DictionaryStore {
    dir: PathBuf,
    cache: Mutex<Cache>,
    /// Held while a reader reparses, so that a reader arriving meanwhile waits for that
    /// parse and takes it instead of parsing the same file again.
    refresh: Mutex<()>,
    /// How many times this handle has parsed `dictionary.json` ([`Self::store_parses`]).
    parses: AtomicU64,
    /// How many times a read of this handle parsed `usage.json` ([`Self::usage_parses`]).
    usage_parses: AtomicU64,
}

impl DictionaryStore {
    /// The store in `dir`. Nothing is read or created until it is used.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            cache: Mutex::new(Cache::default()),
            refresh: Mutex::new(()),
            parses: AtomicU64::new(0),
            usage_parses: AtomicU64::new(0),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `dictionary.json`.
    pub fn store_path(&self) -> PathBuf {
        self.dir.join(STORE_FILE)
    }

    /// `usage.json`.
    pub fn usage_path(&self) -> PathBuf {
        self.dir.join(USAGE_FILE)
    }

    /// How many times this handle has parsed `dictionary.json`, in a read or in a write's
    /// fresh read under the lock. The latency tests read it: a dictation over an unchanged
    /// store parses nothing, and a write parses once.
    pub fn store_parses(&self) -> u64 {
        self.parses.load(Ordering::Relaxed)
    }

    /// How many times a read of this handle parsed `usage.json`. A usage flush through
    /// the handle leaves the counters it wrote in the cache, so the read after it adds
    /// none; a flush through another handle makes the next read parse the file once.
    pub fn usage_parses(&self) -> u64 {
        self.usage_parses.load(Ordering::Relaxed)
    }

    /// `usage.json` as it is now: a file this version cannot read counts as empty.
    fn read_usage(&self, path: &Path) -> io::Result<Slot<UsageFile>> {
        Ok(match read_stamped(path)? {
            Some((bytes, stamp)) => {
                self.usage_parses.fetch_add(1, Ordering::Relaxed);
                Slot {
                    stamp: Some(stamp),
                    value: Arc::new(UsageFile::parse(&bytes).unwrap_or_default()),
                }
            }
            None => Slot {
                stamp: None,
                value: Arc::new(UsageFile::default()),
            },
        })
    }

    fn cache(&self) -> MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Read and parse `dictionary.json`, counted in [`Self::store_parses`].
    fn read_store(&self, path: &Path) -> Result<Slot<StoreFile>, StoreError> {
        match read_stamped(path)? {
            Some((bytes, stamp)) => {
                self.parses.fetch_add(1, Ordering::Relaxed);
                match StoreFile::parse(&bytes) {
                    Some(file) => Ok(Slot {
                        stamp: Some(stamp),
                        value: Arc::new(file),
                    }),
                    None => Err(StoreError::Unreadable {
                        path: path.to_path_buf(),
                    }),
                }
            }
            None => Ok(Slot {
                stamp: None,
                value: Arc::new(StoreFile::default()),
            }),
        }
    }

    /// The parsed store: two `stat` calls while neither file changed, and otherwise a
    /// reparse of the file that changed alone. A missing store is a new, empty one; a store
    /// this version cannot read is [`StoreError::Unreadable`]. A usage file that cannot be
    /// read counts as empty: a lost usage figure costs ranking, never a term.
    pub fn view(&self) -> Result<Arc<StoreView>, StoreError> {
        let store_path = self.store_path();
        let usage_path = self.usage_path();
        let store_now = Stamp::at(&store_path)?;
        let usage_now = Stamp::at(&usage_path)?;
        if let Some(view) = self.cache().current(&store_now, &usage_now) {
            return Ok(view);
        }
        let _refresh = self.refresh.lock().unwrap_or_else(PoisonError::into_inner);
        let (store_fresh, usage_fresh) = {
            let cache = self.cache();
            if let Some(view) = cache.current(&store_now, &usage_now) {
                return Ok(view);
            }
            (cache.store_is(&store_now), cache.usage_is(&usage_now))
        };
        let store = if store_fresh {
            None
        } else {
            Some(self.read_store(&store_path)?)
        };
        let usage = if usage_fresh {
            None
        } else {
            Some(self.read_usage(&usage_path)?)
        };
        let mut cache = self.cache();
        if let Some(slot) = store {
            cache.set_store(slot);
        }
        if let Some(slot) = usage {
            cache.set_usage(slot);
        }
        // Both slots are set by now: each was fresh in the cache or was just read.
        cache
            .assemble()
            .ok_or_else(|| StoreError::Io(io::Error::other("the dictionary was not read")))
    }

    /// Read the store fresh under `lock`, apply `change`, and write the result if it
    /// differs from what was read. An `Err` from `change` writes nothing. The cache then
    /// holds what is on disk, keyed by that file's own stamp: what this write renamed into
    /// place, or what it read when it changed nothing. So a write parses the store once,
    /// and the read that follows it, such as the answer's view or the next dictation's,
    /// parses nothing.
    fn change_locked<T>(
        &self,
        _lock: &StoreLock,
        change: impl FnOnce(&mut StoreFile) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let path = self.store_path();
        let (mut file, before, read) = match read_stamped(&path)? {
            Some((bytes, stamp)) => {
                self.parses.fetch_add(1, Ordering::Relaxed);
                match StoreFile::parse(&bytes) {
                    Some(file) => (file, bytes, Some(stamp)),
                    None => return Err(StoreError::Unreadable { path }),
                }
            }
            None => (StoreFile::default(), Vec::new(), None),
        };
        let result = change(&mut file)?;
        let after = file.to_bytes();
        // `None` when the write could not stamp the file it renamed into place: the cache
        // then holds nothing for the entries and the next read parses them.
        let stamp = if after != before {
            write_atomically(&path, &after)?.map(Some)
        } else {
            Some(read)
        };
        self.cache().seed_store(stamp, file);
        Ok(result)
    }

    /// A bounded write only when the store is still the opt-in version admitted by
    /// the caller. A cross-process OFF to ON change gets a new file stamp and drops
    /// the stale observation, even if its final setting is ON again.
    pub fn try_write_within_at_stamp<T>(
        &self,
        budget: Duration,
        expected_stamp: u64,
        change: impl FnOnce(&mut StoreFile) -> Result<T, StoreError>,
    ) -> Result<Option<T>, StoreError> {
        let Some(lock) = StoreLock::within(&self.dir, budget)? else {
            return Ok(None);
        };
        let current = Stamp::at(&self.store_path())?.map_or(0, |stamp| stamp.digest());
        if current != expected_stamp {
            return Ok(None);
        }
        self.change_locked(&lock, change).map(Some)
    }

    /// Change the store: an exclusive lock, a fresh parse, `change`, then the temporary
    /// file, flushed and renamed. The daemon's and the CLI's ordinary writes wait for the
    /// lock. A change that leaves the file as it was writes nothing, so the file stays
    /// byte identical.
    pub fn write<T>(
        &self,
        change: impl FnOnce(&mut StoreFile) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let lock = StoreLock::acquire(&self.dir)?;
        self.change_locked(&lock, change)
    }

    /// [`Self::write`] for the extractor: `None` when the lock did not come free within
    /// `budget`, and the write is dropped.
    pub fn try_write_within<T>(
        &self,
        budget: Duration,
        change: impl FnOnce(&mut StoreFile) -> Result<T, StoreError>,
    ) -> Result<Option<T>, StoreError> {
        match StoreLock::within(&self.dir, budget)? {
            Some(lock) => self.change_locked(&lock, change).map(Some),
            None => Ok(None),
        }
    }

    /// The entries file as it is now, for a caller that holds the lock: the cached parse
    /// while the file is still the version it holds, else a fresh one, which the cache then
    /// keeps. `None` when there is no file or it cannot be read.
    fn entries_now(&self) -> io::Result<Option<Arc<StoreFile>>> {
        let path = self.store_path();
        let Some(now) = Stamp::at(&path)? else {
            return Ok(None);
        };
        if let Some(slot) = self
            .cache()
            .store
            .as_ref()
            .filter(|slot| slot.stamp.as_ref() == Some(&now))
        {
            return Ok(Some(Arc::clone(&slot.value)));
        }
        match self.read_store(&path) {
            Ok(slot) => {
                let file = Arc::clone(&slot.value);
                self.cache().set_store(slot);
                Ok(Some(file))
            }
            Err(StoreError::Io(e)) => Err(e),
            Err(_) => Ok(None),
        }
    }

    /// Add a batch of usage bumps to `usage.json` under the store's lock, waiting at most
    /// `budget` for it; `false` when the budget ran out and the batch was dropped. Never
    /// touches `dictionary.json`, so no row's `rev` changes. Counters of rows that no
    /// longer exist are dropped on the way, read from the cached entries while the file is
    /// unchanged. A usage file this version cannot read is [`StoreError::Unreadable`] and is
    /// left as it is; an empty one starts over. The file is replaced by a rename with no
    /// flush to disk (`replace_unsynced` in `lock.rs`), because its counters are loss
    /// tolerant and the flush would sit before a one shot transcription's exit. This
    /// handle's cache then holds the counters written, so its next read parses neither
    /// file; the daemon flushes through its one shared handle for that reason.
    pub fn flush_usage(
        &self,
        deltas: &[(EntryId, UsageScope, UsageDelta)],
        budget: Duration,
    ) -> Result<bool, StoreError> {
        let Some(_lock) = StoreLock::within(&self.dir, budget)? else {
            return Ok(false);
        };
        let usage_path = self.usage_path();
        let mut usage = match read_stamped(&usage_path)? {
            // An empty file is what an unsynced replace can leave after a power cut, never
            // a newer version's counters, so it starts over rather than stopping usage.
            Some((bytes, _)) if bytes.is_empty() => UsageFile::default(),
            Some((bytes, _)) => match UsageFile::parse(&bytes) {
                Some(usage) => usage,
                None => return Err(StoreError::Unreadable { path: usage_path }),
            },
            None => UsageFile::default(),
        };
        usage.apply(deltas);
        if let Some(file) = self.entries_now()? {
            let ids: std::collections::HashSet<&str> =
                file.entries.iter().map(|e| e.id.as_str()).collect();
            usage.retain_ids(|id| ids.contains(id));
        }
        let stamp = replace_unsynced(&usage_path, &usage.to_bytes())?;
        self.cache().seed_usage(stamp, usage);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema::now;

    fn store() -> (tempfile::TempDir, DictionaryStore) {
        let root = tempfile::tempdir().unwrap();
        let store = DictionaryStore::open(root.path().join("config").join("dictionary"));
        (root, store)
    }

    fn add(store: &DictionaryStore, written: &str, tier: EntryTier) -> EntryId {
        match store
            .write(|file| file.add(NewTerm::person(written, tier), Reach::All, now()))
            .unwrap()
        {
            AddOutcome::Created { id, .. } => id,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_store_is_a_new_one_with_on_request() {
        let (_root, store) = store();
        let view = store.view().unwrap();
        assert!(view.entries().is_empty());
        assert_eq!(view.settings().agent_writes, AgentWrites::OnRequest);
        assert!(!store.dir().exists(), "reading creates nothing");
    }

    /// A term one handle adds is listed by another at once, through its cache: the
    /// in process picture of two processes sharing the store.
    #[test]
    fn a_term_added_by_one_handle_is_listed_by_another_at_once() {
        let (_root, writer) = store();
        let reader = DictionaryStore::open(writer.dir());
        assert!(reader.view().unwrap().entries().is_empty());
        let cached = reader.view().unwrap();
        assert!(
            Arc::ptr_eq(&cached, &reader.view().unwrap()),
            "unchanged: cached"
        );
        add(&writer, "EPAS1", EntryTier::Public);
        let view = reader.view().unwrap();
        assert_eq!(view.entries().len(), 1);
        assert_ne!(view.content_stamp(), cached.content_stamp());
        // Two writes within one clock tick that leave the length alone still reparse,
        // because each write renames a new file into place.
        let id = view.entries()[0].id.clone();
        writer
            .write(|file| {
                file.edit(
                    &id,
                    None,
                    EntryEdit {
                        written: Some("EPAS2".into()),
                        ..EntryEdit::default()
                    },
                    now(),
                )
            })
            .unwrap();
        assert_eq!(reader.view().unwrap().entries()[0].written, "EPAS2");
    }

    /// A corrupt store refuses every write and stays byte identical; a reader gets the
    /// refusal, and nothing is listed.
    #[test]
    fn a_corrupt_store_refuses_writes_and_is_never_rewritten() {
        let (_root, store) = store();
        add(&store, "EPAS1", EntryTier::Public);
        let path = store.store_path();
        let mut bytes = fs::read(&path).unwrap();
        bytes.truncate(bytes.len() / 2);
        fs::write(&path, &bytes).unwrap();
        let refused = store.write(|file| {
            file.add(
                NewTerm::person("UCSF", EntryTier::Public),
                Reach::All,
                now(),
            )
        });
        assert!(matches!(&refused, Err(StoreError::Unreadable { path: p }) if p == &path));
        assert_eq!(
            refused.unwrap_err().to_string(),
            format!(
                "The dictionary file at {} could not be read, so nothing was changed. Fix or remove it, then try again.",
                path.display()
            )
        );
        assert!(matches!(store.view(), Err(StoreError::Unreadable { .. })));
        assert!(matches!(
            store.flush_usage(&[], Duration::from_millis(50)),
            Ok(true)
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes, "byte identical");
        assert!(!store.dir().join("dictionary.json.tmp").exists());
    }

    /// Two writes of the same length give two identities, which is what keeps a reader's
    /// cache honest when they also share a timestamp tick.
    #[test]
    fn each_write_gives_the_file_a_new_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STORE_FILE);
        let written = write_atomically(&path, b"EPAS1").unwrap();
        let first = Stamp::at(&path).unwrap().unwrap();
        assert_eq!(
            written,
            Some(first.clone()),
            "a write answers the stamp the path then has"
        );
        write_atomically(&path, b"EPAS2").unwrap();
        let second = Stamp::at(&path).unwrap().unwrap();
        assert_eq!(first.len, second.len);
        assert_ne!(first.identity, second.identity);
        let (_, read) = read_stamped(&path).unwrap().unwrap();
        assert_eq!(read, second, "a read stamps the version it read");
        let usage = dir.path().join(USAGE_FILE);
        let written = replace_unsynced(&usage, b"{}").unwrap();
        assert_eq!(written, Stamp::at(&usage).unwrap());
    }

    /// A write that could not stamp the file it renamed into place (`rename_stamped` in
    /// `lock.rs` is best effort) leaves the handle holding nothing for that file, so the
    /// next read parses it rather than trust a slot keyed by a stamp it does not hold; the
    /// other file's slot stays.
    #[test]
    fn a_write_with_no_stamp_leaves_the_next_read_to_parse() {
        let (_root, store) = store();
        add(&store, "EPAS1", EntryTier::Public);
        let before = store.view().unwrap();
        let parses = store.store_parses();
        let file = (*before.file).clone();
        store.cache().seed_store(None, file);
        let view = store.view().unwrap();
        assert_eq!(store.store_parses(), parses + 1, "the entries were parsed");
        assert_eq!(view.entries(), before.entries());
        assert_eq!(view.content_stamp(), before.content_stamp());
        assert!(
            Arc::ptr_eq(&view.usage, &before.usage),
            "usage kept its slot"
        );
        store.cache().seed_usage(None, UsageFile::default());
        let again = store.view().unwrap();
        assert_eq!(store.store_parses(), parses + 1);
        assert!(
            Arc::ptr_eq(&again.file, &view.file),
            "the entries kept theirs"
        );
        assert!(
            !Arc::ptr_eq(&again.usage, &view.usage),
            "usage was read again"
        );
    }

    /// PERF-1: a write leaves the handle's cache holding what it wrote, so the read that
    /// follows it parses nothing, and what the cache holds is exactly what a reader that
    /// parses the file gets.
    #[test]
    fn a_write_parses_once_and_leaves_the_cache_holding_what_it_wrote() {
        let (_root, store) = store();
        let id = add(&store, "EPAS1", EntryTier::Public);
        assert_eq!(store.store_parses(), 0, "a new store has nothing to parse");
        store
            .write(|file| {
                file.edit(
                    &id,
                    None,
                    EntryEdit {
                        spoken_forms: Some(vec!["e pass one".into()]),
                        ..EntryEdit::default()
                    },
                    now(),
                )?;
                file.add(
                    NewTerm::person("UCSF", EntryTier::Private),
                    Reach::All,
                    now(),
                )
            })
            .unwrap();
        assert_eq!(
            store.store_parses(),
            1,
            "the write's fresh read under the lock"
        );
        let seeded = store.view().unwrap();
        assert_eq!(
            store.store_parses(),
            1,
            "the read after the write parsed nothing"
        );
        assert!(Arc::ptr_eq(&seeded, &store.view().unwrap()));
        let parsed = StoreFile::parse(&fs::read(store.store_path()).unwrap()).unwrap();
        assert_eq!(*seeded.file(), parsed, "the cache holds what is on disk");
        let other = DictionaryStore::open(store.dir());
        assert_eq!(
            other.view().unwrap().content_stamp(),
            seeded.content_stamp()
        );

        // A write that changes nothing writes nothing and keeps the view current.
        let outcome = store
            .write(|file| {
                file.add(
                    NewTerm::person("EPAS1", EntryTier::Public),
                    Reach::All,
                    now(),
                )
            })
            .unwrap();
        assert!(matches!(outcome, AddOutcome::Exists { .. }));
        assert!(Arc::ptr_eq(&seeded, &store.view().unwrap()));
        assert_eq!(store.store_parses(), 2);
    }

    /// The cache stays honest after a seeded write: another handle's write (another
    /// process, the CLI) renames a new file into place, and the next read sees it, within
    /// one clock tick and at the same length too.
    #[test]
    fn a_write_another_handle_makes_after_a_seeded_write_is_seen() {
        let (_root, store) = store();
        add(&store, "EPAS1", EntryTier::Public);
        let id = store.view().unwrap().entries()[0].id.clone();
        let other = DictionaryStore::open(store.dir());
        other
            .write(|file| {
                file.edit(
                    &id,
                    None,
                    EntryEdit {
                        written: Some("EPAS2".into()),
                        ..EntryEdit::default()
                    },
                    now(),
                )
            })
            .unwrap();
        let parses = store.store_parses();
        let view = store.view().unwrap();
        assert_eq!(view.entries()[0].written, "EPAS2");
        assert_eq!(store.store_parses(), parses + 1);
        // And a write through this handle after that one starts from the other's file.
        add(&store, "UCSF", EntryTier::Public);
        let written: Vec<String> = store
            .view()
            .unwrap()
            .entries()
            .iter()
            .map(|e| e.written.clone())
            .collect();
        assert_eq!(written, ["EPAS2", "UCSF"]);
    }

    /// The PERF-1 review's hazard: an edit made in place, on the same inode, that keeps the
    /// length and sets the modification time back (`touch -r`, `rsync -t`, a restore that
    /// keeps both) is seen at the next read, through the warm cache of the handle that
    /// wrote the file and through one that only read it. Before the stamp held the change
    /// time, both kept serving the rows as they were until a later write. The pause lets a
    /// file system whose clock is coarse (Linux stamps files at the kernel's tick) give the
    /// edit a change time of its own.
    #[cfg(unix)]
    #[test]
    fn an_edit_in_place_that_keeps_the_length_and_the_times_is_seen() {
        use std::io::Write;
        use std::os::unix::fs::MetadataExt;
        let (_root, store) = store();
        add(&store, "disease-phenotypes", EntryTier::Public);
        let reader = DictionaryStore::open(store.dir());
        for handle in [&store, &reader] {
            assert_eq!(
                handle.view().unwrap().entries()[0].written,
                "disease-phenotypes"
            );
        }
        let path = store.store_path();
        let before = fs::metadata(&path).unwrap();
        let bytes = fs::read(&path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let edited = text.replace("disease-phenotypes", "disease-phenotypez");
        std::thread::sleep(Duration::from_millis(50));
        let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.write_all(edited.as_bytes()).unwrap();
        file.set_modified(before.modified().unwrap()).unwrap();
        drop(file);
        let after = fs::metadata(&path).unwrap();
        assert_eq!(
            (after.len(), after.modified().unwrap(), after.ino()),
            (before.len(), before.modified().unwrap(), before.ino()),
            "the edit kept the length, the time and the inode"
        );
        for handle in [&store, &reader] {
            let parses = handle.store_parses();
            let view = handle.view().unwrap();
            assert_eq!(view.entries()[0].written, "disease-phenotypez");
            assert_eq!(handle.store_parses(), parses + 1);
        }
    }

    /// A usage bump, which follows every dictation that wrote a term, reparses only
    /// `usage.json`: the entries stay the parse already held, and the content stamp, which
    /// the bonus table's revision follows, does not move.
    #[test]
    fn a_usage_bump_reparses_only_the_usage_file() {
        let (_root, store) = store();
        let id = add(&store, "EPAS1", EntryTier::Public);
        let before = store.view().unwrap();
        let bump = [(
            id.clone(),
            UsageScope::Public,
            UsageDelta {
                uses: 1,
                corrected_to: 0,
                last_used_at: Some(now()),
            },
        )];
        // From another handle, as a one shot transcription or the daemon's usage task.
        let other = DictionaryStore::open(store.dir());
        assert!(other.flush_usage(&bump, Duration::from_millis(50)).unwrap());
        let parses = store.store_parses();
        let usage_parses = store.usage_parses();
        let after = store.view().unwrap();
        assert_eq!(
            store.store_parses(),
            parses,
            "the entries were not reparsed"
        );
        assert_eq!(
            store.usage_parses(),
            usage_parses + 1,
            "another handle's flush: the usage file is read again"
        );
        assert_eq!(after.usage_stamp(), view_usage_stamp(&store));
        assert!(Arc::ptr_eq(&before.file, &after.file));
        assert_eq!(after.usage(&id, UsageScope::Public).unwrap().uses, 1);
        assert_eq!(after.content_stamp(), before.content_stamp());
        // Through this handle: the flush reads the entries from the cache and leaves the
        // counters it wrote there, so the next read parses neither file.
        assert!(store.flush_usage(&bump, Duration::from_millis(50)).unwrap());
        assert_eq!(store.store_parses(), parses);
        let flushed = store.view().unwrap();
        assert_eq!(
            store.usage_parses(),
            usage_parses + 1,
            "this handle's flush: the next read parses neither file"
        );
        assert_ne!(flushed.usage_stamp(), after.usage_stamp());
        assert_eq!(flushed.usage_stamp(), view_usage_stamp(&store));
        assert!(Arc::ptr_eq(&before.file, &flushed.file));
        assert_eq!(flushed.usage(&id, UsageScope::Public).unwrap().uses, 2);
        let parsed = UsageFile::parse(&fs::read(store.usage_path()).unwrap()).unwrap();
        assert_eq!(*flushed.usage, parsed);
    }

    /// The usage stamp a fresh handle reads for `store`'s folder.
    fn view_usage_stamp(store: &DictionaryStore) -> u64 {
        DictionaryStore::open(store.dir())
            .view()
            .unwrap()
            .usage_stamp()
    }

    /// Readers that arrive together while the store changed parse it once between them:
    /// one parses, the others wait for its parse and take it. The barrier releases the eight
    /// readers at once, and 4,000 rows make the parse last far longer than the skew between
    /// them, so without the refresh lock several would parse and the count would exceed 1.
    #[test]
    fn readers_that_arrive_together_parse_once() {
        let (_root, store) = store();
        let lines: String = (0..4_000).map(|i| format!("TERM{i}\n")).collect();
        let rows = transfer::parse_import(ImportFormat::Lines, &lines).unwrap();
        store
            .write(|file| Ok(file.import(rows, EntryTier::Public, &|_| false, now())))
            .unwrap();
        let store = Arc::new(DictionaryStore::open(store.dir()));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let readers: Vec<_> = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    store.view().unwrap()
                })
            })
            .collect();
        let views: Vec<Arc<StoreView>> = readers.into_iter().map(|r| r.join().unwrap()).collect();
        assert_eq!(store.store_parses(), 1);
        assert!(views.iter().all(|v| Arc::ptr_eq(v, &views[0])));
    }

    /// The content stamp: the same for every view of an unchanged file, whatever handle
    /// reads it, new at every write, and 0 with no file.
    #[test]
    fn the_content_stamp_follows_the_entries_file() {
        let (_root, store) = store();
        assert_eq!(store.view().unwrap().content_stamp(), 0);
        add(&store, "EPAS1", EntryTier::Public);
        let first = store.view().unwrap().content_stamp();
        assert_ne!(first, 0);
        assert_eq!(
            DictionaryStore::open(store.dir())
                .view()
                .unwrap()
                .content_stamp(),
            first
        );
        add(&store, "UCSF", EntryTier::Private);
        assert_ne!(store.view().unwrap().content_stamp(), first);
    }

    #[test]
    fn a_write_that_changes_nothing_leaves_the_file_alone() {
        let (_root, store) = store();
        add(&store, "EPAS1", EntryTier::Public);
        let before = Stamp::at(&store.store_path()).unwrap();
        let outcome = store
            .write(|file| {
                file.add(
                    NewTerm::person("EPAS1", EntryTier::Public),
                    Reach::All,
                    now(),
                )
            })
            .unwrap();
        assert!(matches!(outcome, AddOutcome::Exists { .. }));
        let after = Stamp::at(&store.store_path()).unwrap();
        assert!(before.is_some());
        assert_eq!(before, after);
    }

    /// DD-33: usage lives in its own file, so a bump never changes a row's `rev` or the
    /// entries file at all.
    #[test]
    fn a_usage_bump_leaves_rev_unchanged() {
        let (_root, store) = store();
        let id = add(&store, "EPAS1", EntryTier::Public);
        let entries_before = fs::read(store.store_path()).unwrap();
        let bump = UsageDelta {
            uses: 1,
            corrected_to: 0,
            last_used_at: Some(now()),
        };
        assert!(store
            .flush_usage(
                &[
                    (id.clone(), UsageScope::Public, bump),
                    (EntryId::random(), UsageScope::All, bump),
                ],
                Duration::from_millis(50)
            )
            .unwrap());
        assert_eq!(fs::read(store.store_path()).unwrap(), entries_before);
        let view = store.view().unwrap();
        assert_eq!(view.find(&id).unwrap().rev, 1);
        assert_eq!(view.usage(&id, UsageScope::All).unwrap().uses, 1);
        assert_eq!(view.usage(&id, UsageScope::Public).unwrap().uses, 1);
        let usage = UsageFile::parse(&fs::read(store.usage_path()).unwrap()).unwrap();
        assert_eq!(usage.all.len(), 1, "a gone row's counters are dropped");
    }

    /// The usage file is replaced with no flush to disk, so a power cut can leave it
    /// empty: an empty file starts over, while one this version cannot parse (a newer
    /// version's counters) is still refused and left byte identical.
    #[test]
    fn an_empty_usage_file_starts_over_and_an_unreadable_one_is_kept() {
        let (_root, store) = store();
        let id = add(&store, "EPAS1", EntryTier::Public);
        let bump = [(
            id.clone(),
            UsageScope::All,
            UsageDelta {
                uses: 1,
                corrected_to: 0,
                last_used_at: Some(now()),
            },
        )];
        fs::write(store.usage_path(), b"").unwrap();
        assert!(store.flush_usage(&bump, Duration::ZERO).unwrap());
        assert_eq!(
            store
                .view()
                .unwrap()
                .usage(&id, UsageScope::All)
                .unwrap()
                .uses,
            1
        );
        let newer = br#"{"all":{},"public":{},"later":1}"#;
        fs::write(store.usage_path(), newer).unwrap();
        assert!(matches!(
            store.flush_usage(&bump, Duration::ZERO),
            Err(StoreError::Unreadable { .. })
        ));
        assert_eq!(fs::read(store.usage_path()).unwrap(), newer);
    }

    #[test]
    fn a_bounded_write_is_dropped_while_another_writer_holds_the_lock() {
        let (_root, store) = store();
        add(&store, "EPAS1", EntryTier::Public);
        let held = StoreLock::acquire(store.dir()).unwrap();
        let dropped = store
            .try_write_within(Duration::from_millis(50), |file| {
                file.add(
                    NewTerm::person("UCSF", EntryTier::Public),
                    Reach::All,
                    now(),
                )
            })
            .unwrap();
        assert_eq!(dropped, None);
        assert!(!store.flush_usage(&[], Duration::from_millis(20)).unwrap());
        drop(held);
        assert!(store
            .try_write_within(Duration::from_millis(50), |file| {
                file.add(
                    NewTerm::person("UCSF", EntryTier::Public),
                    Reach::All,
                    now(),
                )
            })
            .unwrap()
            .is_some());
    }

    /// A suggestion racing a delete loses nothing: each writer reads fresh under the
    /// lock, so neither overwrites the other's change.
    #[test]
    fn a_suggestion_racing_a_delete_loses_nothing() {
        let (_root, store) = store();
        let doomed: Vec<EntryId> = (0..40)
            .map(|i| add(&store, &format!("TERM{i}"), EntryTier::Public))
            .collect();
        let dir = store.dir().to_path_buf();
        let suggester = {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let store = DictionaryStore::open(dir);
                for i in 0..40 {
                    let term = NewTerm {
                        source: EntrySource::Auto,
                        status: EntryStatus::Suggested,
                        ..NewTerm::person(format!("SUGGESTED{i}"), EntryTier::Public)
                    };
                    store
                        .write(|file| file.add(term, Reach::All, now()))
                        .unwrap();
                }
            })
        };
        let deleter = std::thread::spawn(move || {
            let store = DictionaryStore::open(dir);
            for id in doomed {
                store.write(|file| file.remove(&id, now())).unwrap();
            }
        });
        suggester.join().unwrap();
        deleter.join().unwrap();
        let view = store.view().unwrap();
        let written: Vec<&str> = view.entries().iter().map(|e| e.written.as_str()).collect();
        assert_eq!(written.len(), 40, "{written:?}");
        assert!(written.iter().all(|w| w.starts_with("SUGGESTED")));
        assert_eq!(view.file().tombstones.len(), 40);
    }

    fn median(mut samples: Vec<Duration>) -> Duration {
        samples.sort();
        samples[samples.len() / 2]
    }

    /// Where the store sits on dictation's path: the bias set reads `view()` when a
    /// recording starts, and the usage flush runs after the text is out. A timing run, by
    /// hand, at the caps (5,000 rows in each tier):
    /// `cargo test --release -p biorouter-dictation --lib store::tests::measure -- --ignored --nocapture`.
    #[test]
    #[ignore = "a timing run, by hand"]
    fn measure_the_store_at_the_caps() {
        use std::time::Instant;
        let (_root, store) = store();
        let at = now();
        let mut file = StoreFile::default();
        // Below the active cap by enough room for the adds and the import timed below.
        for tier in [EntryTier::Public, EntryTier::Private] {
            for i in 0..ops::MAX_ACTIVE_PER_TIER - 1_100 {
                file.entries.push(Entry {
                    id: EntryId::random(),
                    rev: 1,
                    written: format!("TERM{i}X{}", tier.as_str()),
                    tier,
                    status: EntryStatus::Active,
                    source: EntrySource::User,
                    reviewed: true,
                    spoken_forms: vec![format!("term {i} spoken")],
                    misrecognitions: vec![Misrecognition {
                        text: format!("term {i} heard"),
                        count: 2,
                        last_seen: at,
                    }],
                    kind: None,
                    match_policy: MatchPolicy::Exact,
                    bias: BiasPolicy::Auto,
                    context_cues: vec![],
                    note: Some("a note of a few words".into()),
                    reason: None,
                    suggested_in: None,
                    seen: 1,
                    seen_in: Vec::new(),
                    created_at: at,
                    updated_at: at,
                    provenance: None,
                    affiliation: tier.is_private().then(|| RowAffiliation {
                        institutions: vec!["ucsf".into()],
                    }),
                });
            }
        }
        let bytes = file.to_bytes();
        lock::create_dir_private(store.dir()).unwrap();
        write_atomically(&store.store_path(), &bytes).unwrap();
        println!(
            "store: {} rows, {} bytes, {} bytes a row",
            file.entries.len(),
            bytes.len(),
            bytes.len() / file.entries.len()
        );

        let cold = median(
            (0..7)
                .map(|_| {
                    let fresh = DictionaryStore::open(store.dir());
                    let started = Instant::now();
                    fresh.view().unwrap();
                    started.elapsed()
                })
                .collect(),
        );
        store.view().unwrap();
        let warm = median(
            (0..1000)
                .map(|_| {
                    let started = Instant::now();
                    store.view().unwrap();
                    started.elapsed()
                })
                .collect(),
        );
        let write = median(
            (0..15)
                .map(|i| {
                    let started = Instant::now();
                    store
                        .write(|file| {
                            file.add(
                                NewTerm::person(format!("ADDED{i}"), EntryTier::Public),
                                Reach::All,
                                now(),
                            )
                        })
                        .unwrap();
                    started.elapsed()
                })
                .collect(),
        );
        // PERF-1: the read that follows a write, as the next dictation's is, parses nothing.
        let parses = store.store_parses();
        let after_write = median(
            (0..15)
                .map(|i| {
                    store
                        .write(|file| {
                            file.add(
                                NewTerm::person(format!("SEEDED{i}"), EntryTier::Public),
                                Reach::All,
                                now(),
                            )
                        })
                        .unwrap();
                    let started = Instant::now();
                    store.view().unwrap();
                    started.elapsed()
                })
                .collect(),
        );
        assert_eq!(store.store_parses(), parses + 15, "one parse a write");
        let ids: Vec<EntryId> = file.entries.iter().take(30).map(|e| e.id.clone()).collect();
        let flush = median(
            (0..15)
                .map(|_| {
                    let deltas: Vec<_> = ids
                        .iter()
                        .map(|id| {
                            (
                                id.clone(),
                                UsageScope::Public,
                                UsageDelta {
                                    uses: 1,
                                    corrected_to: 0,
                                    last_used_at: Some(now()),
                                },
                            )
                        })
                        .collect();
                    let started = Instant::now();
                    assert!(store
                        .flush_usage(&deltas, Duration::from_millis(50))
                        .unwrap());
                    started.elapsed()
                })
                .collect(),
        );
        let usage_reread = median(
            (0..15)
                .map(|_| {
                    let bump = [(
                        ids[0].clone(),
                        UsageScope::Public,
                        UsageDelta {
                            uses: 1,
                            corrected_to: 0,
                            last_used_at: Some(now()),
                        },
                    )];
                    // From another handle, as the daemon's usage task and `transcribe` do.
                    assert!(DictionaryStore::open(store.dir())
                        .flush_usage(&bump, Duration::from_millis(50))
                        .unwrap());
                    let started = Instant::now();
                    store.view().unwrap();
                    started.elapsed()
                })
                .collect(),
        );
        assert_eq!(
            store.store_parses(),
            parses + 15,
            "a usage bump reparses no entries"
        );
        let lines: String = (0..1_000)
            .map(|i| format!("IMPORTED{i} = imported {i}\n"))
            .collect();
        let rows = transfer::parse_import(ImportFormat::Lines, &lines).unwrap();
        let started = Instant::now();
        let plan = store
            .write(|file| Ok(file.import(rows, EntryTier::Public, &|_| false, now())))
            .unwrap();
        let import = started.elapsed();
        assert_eq!(plan.rows.len(), 1_000);
        println!(
            "view cold (parse): {cold:?}; view warm (two stats): {warm:?}; add one term: {write:?}; view after a write: {after_write:?}; flush 30 usage bumps: {flush:?}; view after another handle's usage bump: {usage_reread:?}; import 1,000 new rows: {import:?}"
        );
    }
    #[test]
    fn stale_opt_in_stamp_cannot_write_after_a_setting_change() {
        let (_root, store) = store();
        let before = store.view().unwrap().content_stamp();
        store
            .write(|file| {
                file.settings.learn_from_chats = true;
                Ok(())
            })
            .unwrap();
        let outcome = store
            .try_write_within_at_stamp(Duration::from_millis(50), before, |file| {
                file.settings.learn_from_chats = false;
                Ok(())
            })
            .unwrap();
        assert!(outcome.is_none());
        assert!(store.view().unwrap().settings().learn_from_chats);
    }
}
