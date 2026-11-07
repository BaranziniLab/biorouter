//! The personal dictionary store (`docs/dictation/dictionary.md`, "The store", DD-33 to
//! DD-43): one file of terms per account, `<config>/dictionary/dictionary.json`, beside
//! `usage.json` and `.lock`. The daemon, the CLI and the agent's `dictionary`
//! capability all read and write it through [`DictionaryStore`], under one lock.
//!
//! * [`schema`]: the v1 file, snake_case on disk.
//! * [`validate`]: the one validator every writer calls.
//! * [`fold`]: fold keys, for tombstones and the "same letters" warning.
//! * [`ops`]: what each write does, with the tier, tombstone and cap rules.
//! * [`tier_user`]: tier changes, behind the person's proof.
//! * [`transfer`]: import and export.
//! * [`usage`]: the usage counters, in their own file.
//!
//! Writers take the lock, read both files fresh, change, and replace the file
//! atomically ([`lock`]). Readers take no lock and cache the parsed view by each
//! file's modification time, length and identity, so a term one process adds is listed
//! by another at once. A file this version cannot read fails closed: listings are
//! empty, dictation runs with no terms, every write is refused with
//! MSG-DICTIONARY-FILE, and the file is never rewritten from a failed parse.
//!
//! A leaf crate: the store holds no absolute path and takes its folder as an argument.
//! The core resolves it (`Paths::in_config_dir("dictionary")`).

mod lock;

pub mod fold;
pub mod ops;
pub mod schema;
pub mod tier_user;
pub mod transfer;
pub mod usage;
pub mod validate;

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

pub use fold::fold_key;
pub use ops::{AddOutcome, EntryEdit, NewTerm, Reach};
pub use schema::{
    AgentWrites, BiasPolicy, Entry, EntryId, EntrySource, EntryStatus, EntryTier, MatchPolicy,
    Misrecognition, Provenance, ProvenanceReason, RowAffiliation, StoreFile, StoreSettings,
    TermKind, Tombstone,
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

/// Bumped at every reparse in this process. Never exposed by a route (DD-40): it tells
/// the service when to rebuild its bias tables.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// A parsed store, as of one read.
#[derive(Debug)]
pub struct StoreView {
    file: StoreFile,
    usage: UsageFile,
    generation: u64,
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

    /// The in process generation this view was parsed at.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// What identifies one version of a file without reading it: modification time,
/// length, and the file's identity, the inode on Unix and the NTFS file index on Windows.
/// A write renames a new file into place, so the identity changes even when the time and
/// the length do not. The creation time would not do on Windows: NTFS tunnelling gives a
/// file created or renamed onto a name removed within the last 15 s that file's creation
/// time, so every write within 15 s would carry the same one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
    identity: u64,
}

impl Stamp {
    fn with_identity(metadata: &fs::Metadata, identity: u64) -> Self {
        Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
            identity,
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

type CacheKey = (Option<Stamp>, Option<Stamp>);

/// One account's dictionary.
#[derive(Debug)]
pub struct DictionaryStore {
    dir: PathBuf,
    cache: Mutex<Option<(CacheKey, Arc<StoreView>)>>,
}

impl DictionaryStore {
    /// The store in `dir`. Nothing is read or created until it is used.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            cache: Mutex::new(None),
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

    /// The parsed store, reparsed only when either file changed since the last call. A
    /// missing store is a new, empty one; a store this version cannot read is
    /// [`StoreError::Unreadable`]. A usage file that cannot be read counts as empty: a
    /// lost usage figure costs ranking, never a term.
    pub fn view(&self) -> Result<Arc<StoreView>, StoreError> {
        let store_path = self.store_path();
        let usage_path = self.usage_path();
        let key = (Stamp::at(&store_path)?, Stamp::at(&usage_path)?);
        {
            let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((cached, view)) = cache.as_ref() {
                if *cached == key {
                    return Ok(Arc::clone(view));
                }
            }
        }
        let (file, store_stamp) = match read_stamped(&store_path)? {
            Some((bytes, stamp)) => match StoreFile::parse(&bytes) {
                Some(file) => (file, Some(stamp)),
                None => return Err(StoreError::Unreadable { path: store_path }),
            },
            None => (StoreFile::default(), None),
        };
        let (usage, usage_stamp) = match read_stamped(&usage_path)? {
            Some((bytes, stamp)) => (UsageFile::parse(&bytes).unwrap_or_default(), Some(stamp)),
            None => (UsageFile::default(), None),
        };
        let view = Arc::new(StoreView {
            file,
            usage,
            generation: GENERATION.fetch_add(1, Ordering::Relaxed) + 1,
        });
        *self.cache.lock().unwrap_or_else(PoisonError::into_inner) =
            Some(((store_stamp, usage_stamp), Arc::clone(&view)));
        Ok(view)
    }

    /// Read the store fresh under `lock`, apply `change`, and write the result if it
    /// differs from what was read. An `Err` from `change` writes nothing.
    fn change_locked<T>(
        &self,
        _lock: &StoreLock,
        change: impl FnOnce(&mut StoreFile) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let path = self.store_path();
        let (mut file, before) = match read_stamped(&path)? {
            Some((bytes, _)) => match StoreFile::parse(&bytes) {
                Some(file) => (file, bytes),
                None => return Err(StoreError::Unreadable { path }),
            },
            None => (StoreFile::default(), Vec::new()),
        };
        let result = change(&mut file)?;
        let after = file.to_bytes();
        if after != before {
            write_atomically(&path, &after)?;
        }
        Ok(result)
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

    /// Add a batch of usage bumps to `usage.json` under the store's lock, waiting at most
    /// `budget` for it; `false` when the budget ran out and the batch was dropped. Never
    /// touches `dictionary.json`, so no row's `rev` changes. Counters of rows that no
    /// longer exist are dropped on the way. A usage file this version cannot read is
    /// [`StoreError::Unreadable`] and is left as it is; an empty one starts over. The file
    /// is replaced by a rename with no flush to disk (`replace_unsynced` in `lock.rs`), because
    /// its counters are loss tolerant and the flush would sit before a one shot
    /// transcription's exit.
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
        if let Some(file) =
            read_stamped(&self.store_path())?.and_then(|(bytes, _)| StoreFile::parse(&bytes))
        {
            let ids: std::collections::HashSet<&str> =
                file.entries.iter().map(|e| e.id.as_str()).collect();
            usage.retain_ids(|id| ids.contains(id));
        }
        replace_unsynced(&usage_path, &usage.to_bytes())?;
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
        assert!(view.generation() > cached.generation());
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
        write_atomically(&path, b"EPAS1").unwrap();
        let first = Stamp::at(&path).unwrap().unwrap();
        write_atomically(&path, b"EPAS2").unwrap();
        let second = Stamp::at(&path).unwrap().unwrap();
        assert_eq!(first.len, second.len);
        assert_ne!(first.identity, second.identity);
        let (_, read) = read_stamped(&path).unwrap().unwrap();
        assert_eq!(read, second, "a read stamps the version it read");
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
            "view cold (parse): {cold:?}; view warm (two stats): {warm:?}; add one term: {write:?}; flush 30 usage bumps: {flush:?}; import 1,000 new rows: {import:?}"
        );
    }
}
