//! The dictation history (DD-96): the text of every dictation a person started, kept on
//! the computer that transcribed it, so text that was dictated and then lost can be found
//! again. Audio is never kept (P-1).
//!
//! * [`schema`]: the two v1 files, `history.jsonl` and `settings.json`.
//! * [`ops`]: what a search walks and what a full history drops.
//!
//! The folder holds `history.jsonl` (a header line, then one entry per line, oldest
//! first), `settings.json` and `.lock`. It is created 0700 and its files 0600 on Unix.
//!
//! * An append takes the lock within 2 s or drops its entry, cuts a torn last line back
//!   to the last newline, writes one line and flushes it. Passing either cap (5,000
//!   entries or 8 MiB) drops the oldest entries in the same write until both are at 90
//!   percent.
//! * Every other write (delete, clear, a tier change, a settings change) takes the lock,
//!   reads fresh, and replaces the file atomically. A write that changes nothing writes
//!   nothing.
//! * Readers take no lock and cache the parse by the file's modification time, length,
//!   identity and, on Unix, change time. A final line no newline ends is left out.
//! * A file this version cannot read (an unknown format, version or key, a middle line
//!   that does not parse, two entries with one id) fails closed: reads answer
//!   [`HistoryError::Unreadable`], every write is refused, and the file is never
//!   rewritten from a failed parse. An unreadable `settings.json` reads as
//!   [`HistorySettings::UNREADABLE`]: nothing recorded, nothing public.
//!
//! A leaf store: it decides nothing about privacy. A search takes the caller's visibility
//! as a closure and applies it before the limit, and every write that names entries takes
//! a closure that says which ones the caller may touch. The core stamps each entry's tier
//! (`biorouter::dictation::history`), and the routes and the agent's tool decide who sees
//! what. The history never feeds recognition (P-18).

mod disk;
pub mod ops;
pub mod schema;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;
use std::{fmt, io};

use chrono::{DateTime, Timelike, Utc};

pub use ops::{HistoryCursor, HistoryPage, TimeWindow};
pub use schema::{
    cut_text, is_entry_id, new_entry_id, EntryProblem, HistoryEntry, HistorySettings,
    SettingsChange, Surface, HISTORY_FILE, LOCK_FILE, MAX_ENTRIES, MAX_FILE_BYTES, MAX_TEXT_BYTES,
    SETTINGS_FILE,
};

pub use crate::store::EntryTier;

use disk::{HistoryLock, Stamp};
use schema::{entry_line, header_line, parse_history, parse_settings, settings_bytes};

/// How long an append waits for the lock before it drops its entry.
pub const APPEND_BUDGET: Duration = Duration::from_secs(2);

/// MSG-HISTORY-NOT-FOUND.
pub const MSG_HISTORY_NOT_FOUND: &str = "No dictation with that id.";

/// MSG-HISTORY-FILE, naming the file that could not be read.
pub fn msg_history_file(path: &Path) -> String {
    format!(
        "The dictation history file at {} could not be read, so nothing was changed. Fix or remove it, then try again.",
        path.display()
    )
}

/// Why a history operation did not happen.
#[derive(Debug)]
pub enum HistoryError {
    /// A file this version cannot read (MSG-HISTORY-FILE). Nothing was written.
    Unreadable {
        path: PathBuf,
    },
    /// An entry this version does not keep. Nothing was written.
    Invalid(EntryProblem),
    /// The lock did not come free within [`APPEND_BUDGET`]; the entry was dropped.
    Busy,
    Io(io::Error),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path } => f.write_str(&msg_history_file(path)),
            Self::Invalid(problem) => f.write_str(problem.message()),
            Self::Busy => f.write_str("The dictation history is busy, so nothing was saved."),
            Self::Io(e) => write!(f, "The dictation history could not be written: {e}."),
        }
    }
}

impl std::error::Error for HistoryError {}

impl From<io::Error> for HistoryError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// A parsed history, as of one read.
#[derive(Debug)]
struct HistoryView {
    /// Oldest first.
    entries: Vec<HistoryEntry>,
    /// The bytes a reader counts: up to and including the last newline.
    committed_len: u64,
    /// Whether the file held bytes after its last newline.
    torn: bool,
    /// Each entry's fold key, made by the first search that needs them.
    folded: OnceLock<Vec<String>>,
}

impl HistoryView {
    fn empty() -> Self {
        Self {
            entries: Vec::new(),
            committed_len: 0,
            torn: false,
            folded: OnceLock::new(),
        }
    }

    fn folded(&self) -> &[String] {
        self.folded.get_or_init(|| {
            self.entries
                .iter()
                .map(|entry| crate::store::fold_key(&entry.text))
                .collect()
        })
    }
}

/// The parse of one version of `history.jsonl`, keyed by that version's stamp (`None`
/// for no file).
#[derive(Debug)]
struct Slot {
    stamp: Option<Stamp>,
    view: Arc<HistoryView>,
}

/// One account's dictation history.
#[derive(Debug)]
pub struct HistoryStore {
    dir: PathBuf,
    cache: Mutex<Option<Slot>>,
    /// How many times this handle has parsed `history.jsonl` ([`Self::parses`]).
    parses: AtomicU64,
}

impl HistoryStore {
    /// The history in `dir`. Nothing is read or created until it is used.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            cache: Mutex::new(None),
            parses: AtomicU64::new(0),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `history.jsonl`.
    pub fn history_path(&self) -> PathBuf {
        self.dir.join(HISTORY_FILE)
    }

    /// `settings.json`.
    pub fn settings_path(&self) -> PathBuf {
        self.dir.join(SETTINGS_FILE)
    }

    /// How many times this handle has parsed `history.jsonl`.
    pub fn parses(&self) -> u64 {
        self.parses.load(Ordering::Relaxed)
    }

    fn cache(&self) -> MutexGuard<'_, Option<Slot>> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The person's switches, or [`HistorySettings::UNREADABLE`] when `settings.json`
    /// cannot be read. A missing file is the defaults: history on, nothing public.
    pub fn settings(&self) -> HistorySettings {
        self.read_settings().unwrap_or(HistorySettings::UNREADABLE)
    }

    /// The person's switches, or why `settings.json` could not be read.
    pub fn read_settings(&self) -> Result<HistorySettings, HistoryError> {
        let path = self.settings_path();
        match disk::read_optional(&path) {
            Ok(None) => Ok(HistorySettings::default()),
            Ok(Some(bytes)) => parse_settings(&bytes).ok_or(HistoryError::Unreadable { path }),
            Err(_) => Err(HistoryError::Unreadable { path }),
        }
    }

    /// Change the switches `change` sets. Refused, and the file left byte identical, when
    /// `settings.json` cannot be read; a change that changes nothing writes nothing.
    pub fn set_settings(&self, change: SettingsChange) -> Result<HistorySettings, HistoryError> {
        let _lock = HistoryLock::acquire(&self.dir)?;
        let current = self.read_settings()?;
        let next = HistorySettings {
            keep_history: change.keep_history.unwrap_or(current.keep_history),
            public_by_default: change
                .public_by_default
                .unwrap_or(current.public_by_default),
        };
        if next != current {
            disk::write_atomically(&self.settings_path(), &settings_bytes(next))?;
        }
        Ok(next)
    }

    /// The history as it is now, from the cache while the file is unchanged.
    fn view(&self) -> Result<Arc<HistoryView>, HistoryError> {
        let path = self.history_path();
        let stamp = Stamp::at(&path)?;
        if let Some(slot) = self.cache().as_ref() {
            if slot.stamp == stamp {
                return Ok(Arc::clone(&slot.view));
            }
        }
        let slot = self.read_slot(&path)?;
        let view = Arc::clone(&slot.view);
        *self.cache() = Some(slot);
        Ok(view)
    }

    /// Read and parse `history.jsonl`, counted in [`Self::parses`].
    fn read_slot(&self, path: &Path) -> Result<Slot, HistoryError> {
        let Some((bytes, stamp)) = disk::read_stamped(path)? else {
            return Ok(Slot {
                stamp: None,
                view: Arc::new(HistoryView::empty()),
            });
        };
        self.parses.fetch_add(1, Ordering::Relaxed);
        let parsed = parse_history(&bytes).ok_or_else(|| HistoryError::Unreadable {
            path: path.to_path_buf(),
        })?;
        Ok(Slot {
            stamp: Some(stamp),
            view: Arc::new(HistoryView {
                entries: parsed.entries,
                committed_len: parsed.committed_len,
                torn: parsed.torn,
                folded: OnceLock::new(),
            }),
        })
    }

    /// A writer's fresh read under the lock: the cache when the file is the version it
    /// holds, a parse otherwise.
    fn fresh(&self) -> Result<Arc<HistoryView>, HistoryError> {
        self.view()
    }

    /// Keep `entries` as the parse of the file now at `path`, stamped after the write
    /// while the lock is still held, so no other writer's version can be stamped instead.
    fn seed(&self, path: &Path, entries: Vec<HistoryEntry>, committed_len: u64) {
        let slot = Stamp::at(path).ok().flatten().map(|stamp| Slot {
            stamp: Some(stamp),
            view: Arc::new(HistoryView {
                entries,
                committed_len,
                torn: false,
                folded: OnceLock::new(),
            }),
        });
        *self.cache() = slot;
    }

    /// Record one dictation. The text is cut to [`MAX_TEXT_BYTES`] (setting `truncated`)
    /// and the time to whole seconds. Refused when the history cannot be read, when the
    /// entry does not validate or its id is taken, and dropped with
    /// [`HistoryError::Busy`] when the lock does not come free within [`APPEND_BUDGET`].
    pub fn append(&self, mut entry: HistoryEntry) -> Result<(), HistoryError> {
        let (cut, was_cut) = cut_text(&entry.text, MAX_TEXT_BYTES);
        if was_cut {
            entry.text = cut.to_string();
            entry.truncated = Some(true);
        }
        entry.created_at = whole_seconds(entry.created_at);
        schema::validate_entry(&entry).map_err(HistoryError::Invalid)?;

        let Some(_lock) = HistoryLock::within(&self.dir, APPEND_BUDGET)? else {
            return Err(HistoryError::Busy);
        };
        let path = self.history_path();
        let current = self.fresh()?;
        if current.entries.iter().any(|e| e.id == entry.id) {
            return Err(HistoryError::Invalid(EntryProblem::Id));
        }
        let header = header_line();
        let line = entry_line(&entry);
        let mut committed = current.committed_len;
        let before = if committed == 0 {
            header.len() as u64
        } else {
            committed
        };
        let over_a_cap =
            current.entries.len() + 1 > MAX_ENTRIES || before + line.len() as u64 > MAX_FILE_BYTES;
        let drop = if over_a_cap {
            let mut line_lens: Vec<u64> = current
                .entries
                .iter()
                .map(|e| entry_line(e).len() as u64)
                .collect();
            line_lens.push(line.len() as u64);
            ops::prune_count(header.len() as u64, &line_lens)
        } else {
            0
        };

        let mut entries = current.entries.clone();
        entries.push(entry);
        if drop > 0 {
            entries.drain(..drop);
            let bytes = ops::file_bytes(&entries);
            disk::write_atomically(&path, &bytes)?;
            self.seed(&path, entries, bytes.len() as u64);
            return Ok(());
        }
        if current.torn {
            disk::truncate_to(&path, committed)?;
        }
        let mut bytes = Vec::with_capacity(header.len() + line.len());
        if committed == 0 {
            // No complete line: an empty or new file, or only a torn header.
            bytes.extend_from_slice(header.as_bytes());
        }
        bytes.extend_from_slice(line.as_bytes());
        disk::append_bytes(&path, &bytes)?;
        committed += bytes.len() as u64;
        self.seed(&path, entries, committed);
        Ok(())
    }

    /// Search newest first. `visible` is the caller's reach, and it, `window` and `query`
    /// (matched by fold key, as the dictionary matches terms) all run before a match
    /// counts toward `limit`, so entries the caller may not see never use up its page;
    /// `total` counts only what `visible` admits.
    pub fn search(
        &self,
        query: Option<&str>,
        window: TimeWindow,
        cursor: Option<&HistoryCursor>,
        limit: usize,
        visible: &dyn Fn(&HistoryEntry) -> bool,
    ) -> Result<HistoryPage, HistoryError> {
        let view = self.view()?;
        let folded_query = query
            .map(crate::store::fold_key)
            .filter(|folded| !folded.is_empty());
        let folded: &[String] = if folded_query.is_some() {
            view.folded()
        } else {
            &[]
        };
        Ok(ops::walk(
            &view.entries,
            folded,
            folded_query.as_deref(),
            window,
            cursor,
            limit,
            visible,
        ))
    }

    /// One entry by id, whoever may see it: the caller decides.
    pub fn get(&self, id: &str) -> Result<Option<HistoryEntry>, HistoryError> {
        Ok(self.view()?.entries.iter().find(|e| e.id == id).cloned())
    }

    /// Delete the named entries `admits` lets the caller touch; the others are skipped as
    /// missing. Answers how many went.
    pub fn delete(
        &self,
        ids: &[String],
        admits: &dyn Fn(&HistoryEntry) -> bool,
    ) -> Result<usize, HistoryError> {
        self.rewrite(|entries| {
            let before = entries.len();
            entries.retain(|e| !(ids.contains(&e.id) && admits(e)));
            before - entries.len()
        })
    }

    /// Delete every entry `admits` lets the caller touch created at or before `through`,
    /// so a dictation recorded while a confirmation was open survives.
    pub fn clear_through(
        &self,
        through: DateTime<Utc>,
        admits: &dyn Fn(&HistoryEntry) -> bool,
    ) -> Result<usize, HistoryError> {
        self.rewrite(|entries| {
            let before = entries.len();
            entries.retain(|e| !(e.created_at <= through && admits(e)));
            before - entries.len()
        })
    }

    /// Set one entry's tier. `None` when no entry has that id or `admits` refuses it. A
    /// public entry names no institution, and one made private names none either: the
    /// person who changes a tier decides it alone.
    pub fn set_tier(
        &self,
        id: &str,
        tier: EntryTier,
        admits: &dyn Fn(&HistoryEntry) -> bool,
    ) -> Result<Option<HistoryEntry>, HistoryError> {
        let mut changed = None;
        self.rewrite(|entries| {
            let Some(entry) = entries.iter_mut().find(|e| e.id == id && admits(e)) else {
                return 0;
            };
            if entry.tier == tier {
                changed = Some(entry.clone());
                return 0;
            }
            entry.tier = tier;
            entry.institutions = None;
            changed = Some(entry.clone());
            1
        })?;
        Ok(changed)
    }

    /// Change the entries under the lock from a fresh read, and replace the file when
    /// `change` says it changed any. An unreadable file is refused before `change` runs.
    fn rewrite(
        &self,
        change: impl FnOnce(&mut Vec<HistoryEntry>) -> usize,
    ) -> Result<usize, HistoryError> {
        let _lock = HistoryLock::acquire(&self.dir)?;
        let path = self.history_path();
        let current = self.fresh()?;
        let mut entries = current.entries.clone();
        let changed = change(&mut entries);
        if changed == 0 {
            return Ok(0);
        }
        let bytes = ops::file_bytes(&entries);
        disk::write_atomically(&path, &bytes)?;
        self.seed(&path, entries, bytes.len() as u64);
        Ok(changed)
    }
}

/// `at` with its fraction of a second dropped.
fn whole_seconds(at: DateTime<Utc>) -> DateTime<Utc> {
    at.with_nanosecond(0).unwrap_or(at)
}

#[cfg(test)]
mod tests;
