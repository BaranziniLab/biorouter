//! Biorouter's own folder on Windows, and the one-time move out of the folder
//! earlier releases used (R-31).
//!
//! etcetera's Windows strategy builds every directory as
//! `<base>\<author>\<app>\<sub>`: `config` and `data` under the roaming base
//! (`%APPDATA%`), `cache` under the local one (`%LOCALAPPDATA%`), and no state
//! dir, so `Paths` puts state (logs included) under `data`. The author is now
//! [`APP_AUTHOR`]. macOS and Linux use the XDG strategy, which never reads the
//! author, so nothing in this module does anything there.
//!
//! # The rule
//!
//! For each base, `current` is `<base>\UCSF\biorouter` and `legacy` is the same
//! path under [`LEGACY_APP_AUTHOR`]. Each is checked as a [`Presence`]: there,
//! not there, or unknown (the check itself failed).
//!
//! 1. **Resolve, on every call** ([`AppRoot::resolve`]): `legacy` while it is,
//!    or may be, there and `current` is known not to be; otherwise `current`.
//!    An unknown legacy folder counts as there, so a check that fails for a
//!    moment never sends a process into a new, empty current folder that would
//!    then outrank a full legacy one. When both exist, `current` wins and
//!    `legacy` is left alone; the desktop app tells the person, naming it.
//! 2. **Move, only from the daemon** ([`move_for_this_process`], which
//!    `biorouterd agent` calls before logging): when `legacy` is there and
//!    `current` is not, both known, and no other Biorouter process is using
//!    `legacy`. Every process that resolves a legacy folder holds that
//!    folder's [`MOVE_LOCK`] shared for the rest of its life
//!    ([`Pins::hold_while_legacy`], through `platform_dirs`), because it may
//!    have frozen the path (`Config::global`, the session store, the log
//!    directory); the mover needs the lock exclusively. The move drops the
//!    empty [`MOVE_MARKER`] into `legacy`, creates `<base>\UCSF` and renames
//!    `legacy` to `current`: one rename inside one base, so it is atomic and
//!    never a copy. Nothing is deleted except the lock file and a legacy
//!    author folder the move left empty.
//! 3. **Rewrite** stored absolute paths that pointed into `legacy`
//!    ([`rewrite`]) while the marker sits in `current`. The marker is written
//!    *before* the rename so it travels with the tree: a crash between the
//!    rename and the rewrite is harmless, because the next start finds the
//!    marker and finishes the job.
//!
//! The command line tool, `biorouterd mcp` children and the desktop main
//! process never move anything. The first two hold the lock while they use a
//! legacy folder; the desktop resolves on every call and freezes nothing, so a
//! move under it costs nothing. Its half of rule 1 is
//! `ui/desktop/src/utils/appFolder.ts`; both run one table of cases,
//! `resolution-cases.json`, and a test here pins the shared constants equal.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

pub mod rewrite;

#[cfg(test)]
mod tests;

/// The author segment of Biorouter's Windows folder: `%APPDATA%\UCSF\biorouter`.
///
/// Not the product name, on purpose: Electron's own user data folder on Windows
/// is `%APPDATA%\Biorouter` (the desktop `productName`), and the two must stay
/// apart.
pub const APP_AUTHOR: &str = "UCSF";

/// The app segment, which is also the whole folder name under XDG.
pub const APP_NAME: &str = "biorouter";

/// The author folder earlier releases used on Windows, where their config,
/// data and logs lived under `%APPDATA%` and their cache under `%LOCALAPPDATA%`.
///
/// The move has to name the folder to find it, so this is the one place the
/// Rust tree writes the name. Everything else (comments, docs, tests, log
/// lines) says "the legacy folder" or builds the path from this constant. The
/// desktop main process carries the same value in `appFolder.ts`, pinned equal
/// to this one by `the_desktop_constants_match_these`.
pub const LEGACY_APP_AUTHOR: &str = "Block";

/// An empty file the mover drops into the legacy folder just before renaming
/// it. Its presence in the current folder means "stored paths still need
/// rewriting".
pub const MOVE_MARKER: &str = ".folder-move-pending";

/// The lock that keeps the move away from processes using a legacy folder. It
/// sits beside that folder, in its author folder ([`AppRoot::move_lock`]), so
/// it never travels with the tree and nothing is created under the current
/// author before a move; the move removes it.
pub const MOVE_LOCK: &str = ".biorouter-folder.lock";

/// What a check of one folder found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Present,
    /// Not there, or a segment above it is a file.
    Absent,
    /// The check failed (permission denied, an unreachable share), so it may
    /// be there.
    Unknown,
}

impl Presence {
    pub fn of(path: &Path) -> Self {
        match fs::metadata(path) {
            Ok(_) => Presence::Present,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                Presence::Absent
            }
            Err(_) => Presence::Unknown,
        }
    }
}

/// Rule 1 for one consistent pair of checks.
fn resolves_legacy(legacy: Presence, current: Presence) -> bool {
    current == Presence::Absent && legacy != Presence::Absent
}

/// One base directory's two candidate folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRoot {
    pub current: PathBuf,
    pub legacy: PathBuf,
}

impl AppRoot {
    pub fn under(base: &Path) -> Self {
        Self {
            current: base.join(APP_AUTHOR).join(APP_NAME),
            legacy: base.join(LEGACY_APP_AUTHOR).join(APP_NAME),
        }
    }

    /// This root's [`MOVE_LOCK`], in the legacy author folder.
    pub fn move_lock(&self) -> PathBuf {
        self.legacy.with_file_name(MOVE_LOCK)
    }

    /// `(legacy, current)` as of one instant.
    ///
    /// Two separate checks can straddle another process's rename and see the
    /// legacy folder before it and the current one after it, which reads as
    /// "both exist" when only one does. The rename is the one transition that
    /// matters, and it makes `legacy` vanish, so reading `legacy` on both sides
    /// of `current` and retrying when the two disagree gives a consistent pair.
    /// Bounded, so a check whose answer keeps changing cannot spin forever.
    pub fn snapshot(&self) -> (Presence, Presence) {
        let mut pair = (Presence::of(&self.legacy), Presence::of(&self.current));
        for _ in 0..16 {
            let again = Presence::of(&self.legacy);
            if again == pair.0 {
                break;
            }
            pair = (again, Presence::of(&self.current));
        }
        pair
    }

    /// Whether rule 1 sends this process to the legacy folder right now.
    pub fn resolves_legacy(&self) -> bool {
        let (legacy, current) = self.snapshot();
        resolves_legacy(legacy, current)
    }

    /// Rule 1: the folder to read and write right now.
    pub fn resolve(&self) -> &Path {
        if self.resolves_legacy() {
            &self.legacy
        } else {
            &self.current
        }
    }
}

/// What the move found and did for one base directory.
#[derive(Debug)]
pub enum MoveOutcome {
    /// Neither folder exists: a fresh install, which starts in `current`.
    Fresh,
    /// Only `current` exists: moved on an earlier run, or by another process
    /// that won the race.
    AlreadyMoved,
    /// This call renamed `legacy` to `current`.
    Moved,
    /// Both exist. `current` is used and `legacy` is left untouched (usually an
    /// older copy of the command line tool recreated it).
    BothExist,
    /// Only `legacy` exists, and another Biorouter process is using it. It
    /// stays in use for this run, and a later start moves it once none is.
    InUse,
    /// Whether one of the folders exists could not be checked, so nothing was
    /// moved.
    Unreadable,
    /// Only `legacy` exists and it could not be renamed. It stays in use for
    /// this run, and the next start tries again.
    MoveFailed(io::Error),
}

impl MoveOutcome {
    /// The token `resolution-cases.json` uses for this outcome.
    pub fn token(&self) -> &'static str {
        match self {
            MoveOutcome::Fresh => "fresh",
            MoveOutcome::AlreadyMoved => "already-moved",
            MoveOutcome::Moved => "moved",
            MoveOutcome::BothExist => "both-exist",
            MoveOutcome::InUse => "in-use",
            MoveOutcome::Unreadable => "unreadable",
            MoveOutcome::MoveFailed(_) => "move-failed",
        }
    }
}

/// The outcome a consistent snapshot settles without a move, or `None` for the
/// one state a move starts from.
fn settled(snapshot: (Presence, Presence)) -> Option<MoveOutcome> {
    use Presence::{Absent, Present};
    Some(match snapshot {
        (Absent, Absent) => MoveOutcome::Fresh,
        (Absent, Present) => MoveOutcome::AlreadyMoved,
        (Present, Present) => MoveOutcome::BothExist,
        (Present, Absent) => return None,
        _ => MoveOutcome::Unreadable,
    })
}

/// How a move renames, and how long it waits between attempts. Production
/// uses [`std::fs::rename`]; tests swap in a rename that refuses.
pub struct MoveOps<'a> {
    pub rename: &'a (dyn Fn(&Path, &Path) -> io::Result<()> + Sync),
    pub retry_delays: &'a [Duration],
}

/// Waits between attempts, for the lock and for the rename. Another Biorouter
/// process that is just starting holds the lock for a moment, and a directory
/// rename on Windows fails while a file under it is open, which an antivirus
/// scan of freshly written files causes for a moment. A process that lasts
/// longer keeps the legacy folder in use until a later start.
const RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
];

fn std_rename(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

pub const PRODUCTION_OPS: MoveOps<'static> = MoveOps {
    rename: &std_rename,
    retry_delays: &RETRY_DELAYS,
};

fn open_move_lock(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

/// Hold a [`MOVE_LOCK`] shared: waits while a move holds it, and keeps any
/// later move from starting until the returned file is dropped.
pub fn hold_shared(lock: &Path) -> io::Result<File> {
    let file = open_move_lock(lock)?;
    file.lock_shared()?;
    Ok(file)
}

enum Exclusive {
    Held(File),
    /// Another process holds it.
    Busy,
    Failed(io::Error),
}

fn take_exclusive(lock: &Path, delays: &[Duration]) -> Exclusive {
    let file = match open_move_lock(lock) {
        Ok(file) => file,
        Err(error) => return Exclusive::Failed(error),
    };
    let mut delays = delays.iter();
    loop {
        match file.try_lock() {
            Ok(()) => return Exclusive::Held(file),
            Err(TryLockError::WouldBlock) => match delays.next() {
                Some(delay) => std::thread::sleep(*delay),
                None => return Exclusive::Busy,
            },
            Err(TryLockError::Error(error)) => return Exclusive::Failed(error),
        }
    }
}

/// Rule 2 for one base directory. `write_marker` is true for the roaming
/// folder, the only one whose stores hold paths that need rewriting.
pub fn move_root(root: &AppRoot, write_marker: bool, ops: &MoveOps<'_>) -> MoveOutcome {
    let outcome = settled(root.snapshot()).unwrap_or_else(|| gated_move(root, write_marker, ops));
    if Presence::of(&root.legacy) == Presence::Absent {
        // The author folder may hold other apps' folders, and `remove_dir`
        // only ever removes an empty one. This also tidies a folder an
        // interrupted earlier run left behind.
        let _ = fs::remove_file(root.move_lock());
        if let Some(parent) = root.legacy.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
    outcome
}

fn gated_move(root: &AppRoot, write_marker: bool, ops: &MoveOps<'_>) -> MoveOutcome {
    let lock = match take_exclusive(&root.move_lock(), ops.retry_delays) {
        Exclusive::Held(lock) => lock,
        Exclusive::Busy => return settled(root.snapshot()).unwrap_or(MoveOutcome::InUse),
        Exclusive::Failed(error) => {
            return settled(root.snapshot()).unwrap_or(MoveOutcome::MoveFailed(error))
        }
    };
    // Another mover may have finished while this one waited for the lock.
    let outcome = settled(root.snapshot()).unwrap_or_else(|| attempt_move(root, write_marker, ops));
    let _ = lock.unlock();
    outcome
}

fn attempt_move(root: &AppRoot, write_marker: bool, ops: &MoveOps<'_>) -> MoveOutcome {
    if write_marker {
        // Never removed again on failure: an empty file in a folder that is
        // not moved costs nothing, and the next attempt writes it anyway.
        if let Err(error) = fs::File::create(root.legacy.join(MOVE_MARKER)) {
            return settled_by_another_process(root).unwrap_or(MoveOutcome::MoveFailed(error));
        }
    }
    let parent = root.current.parent().map(Path::to_path_buf);
    let created_parent = match &parent {
        Some(parent) if Presence::of(parent) == Presence::Absent => {
            match fs::create_dir_all(parent) {
                Ok(()) => true,
                Err(error) => {
                    return settled_by_another_process(root)
                        .unwrap_or(MoveOutcome::MoveFailed(error))
                }
            }
        }
        _ => false,
    };

    let mut delays = ops.retry_delays.iter();
    let error = loop {
        let error = match (ops.rename)(&root.legacy, &root.current) {
            Ok(()) => return MoveOutcome::Moved,
            Err(error) => error,
        };
        if let Some(settled) = settled_by_another_process(root) {
            return settled;
        }
        match delays.next() {
            Some(delay) => std::thread::sleep(*delay),
            None => break error,
        }
    };

    if created_parent {
        if let Some(parent) = &parent {
            let _ = fs::remove_dir(parent);
        }
    }
    MoveOutcome::MoveFailed(error)
}

/// After a failed step: did another process finish the move, or create the
/// current folder, in the meantime?
fn settled_by_another_process(root: &AppRoot) -> Option<MoveOutcome> {
    match root.snapshot() {
        (Presence::Absent, Presence::Present) => Some(MoveOutcome::AlreadyMoved),
        (Presence::Present, Presence::Present) => Some(MoveOutcome::BothExist),
        _ => None,
    }
}

/// The [`MOVE_LOCK`]s one process holds shared, one per legacy folder it
/// resolved, until it exits.
#[derive(Debug, Default)]
pub struct Pins(Mutex<Vec<(PathBuf, File)>>);

impl Pins {
    pub const fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    fn held(&self) -> MutexGuard<'_, Vec<(PathBuf, File)>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Hold the lock of every root that rule 1 sends this process to the
    /// legacy folder of, once each. Waits while a move is under way, so a
    /// process that starts during one resolves after it.
    ///
    /// A lock that cannot be opened (a legacy folder that cannot be read) is
    /// not held; nothing can move that folder either, because the mover needs
    /// the same file.
    pub fn hold_while_legacy(&self, roots: &[&AppRoot]) {
        let mut held = self.held();
        for root in roots {
            let lock = root.move_lock();
            if held.iter().any(|(path, _)| *path == lock) || !root.resolves_legacy() {
                continue;
            }
            if let Ok(file) = hold_shared(&lock) {
                held.push((lock, file));
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.held().is_empty()
    }

    pub fn into_inner(self) -> Vec<(PathBuf, File)> {
        self.0.into_inner().unwrap_or_else(PoisonError::into_inner)
    }

    fn adopt(&self, pins: Vec<(PathBuf, File)>) {
        self.held().extend(pins);
    }
}

/// One base directory's report.
#[derive(Debug)]
pub struct RootReport {
    pub root: AppRoot,
    pub outcome: MoveOutcome,
}

/// Everything one process's move found and did.
#[derive(Debug, Default)]
pub struct FolderMove {
    /// `BIOROUTER_PATH_ROOT` is set: the sandbox owns every path, so the real
    /// folders were not looked at.
    pub sandboxed: bool,
    pub roaming: Option<RootReport>,
    pub local: Option<RootReport>,
    pub rewrite: Option<rewrite::RewriteReport>,
    /// The locks of the legacy folders this process still resolves after the
    /// move (it failed, or another process was using them), held shared.
    pub pins: Vec<(PathBuf, File)>,
}

/// Rules 2 and 3 for one process, from explicit inputs so every branch is
/// testable on any OS. `bases` is `(roaming, local)`, and is only asked for
/// when there is no sandbox root.
pub fn move_for_process(
    path_root: Option<String>,
    bases: impl FnOnce() -> Option<(PathBuf, PathBuf)>,
    ops: &MoveOps<'_>,
) -> FolderMove {
    if path_root.is_some() {
        return FolderMove {
            sandboxed: true,
            ..FolderMove::default()
        };
    }
    let Some((roaming_base, local_base)) = bases() else {
        return FolderMove::default();
    };

    let roaming = AppRoot::under(&roaming_base);
    let roaming_outcome = move_root(&roaming, true, ops);
    let local = AppRoot::under(&local_base);
    let local_outcome = move_root(&local, false, ops);

    let rewrite = (!roaming.resolves_legacy() && roaming.current.join(MOVE_MARKER).exists())
        .then(|| rewrite::finish_pending_rewrite(&roaming));

    // From here on this process resolves like any other, and holds what it
    // resolves.
    let pins = Pins::new();
    pins.hold_while_legacy(&[&roaming, &local]);

    FolderMove {
        sandboxed: false,
        roaming: Some(RootReport {
            root: roaming,
            outcome: roaming_outcome,
        }),
        local: Some(RootReport {
            root: local,
            outcome: local_outcome,
        }),
        rewrite,
        pins: pins.into_inner(),
    }
}

/// The two Windows base directories, read the way etcetera's Windows strategy
/// reads them (`APPDATA` or `LOCALAPPDATA`, then the known-folder API, then
/// `<home>\AppData\...`).
pub fn windows_bases() -> Option<(PathBuf, PathBuf)> {
    use etcetera::BaseStrategy;
    let base = etcetera::base_strategy::Windows::new().ok()?;
    Some((base.data_dir(), base.cache_dir()))
}

static PROCESS_MOVE: OnceLock<FolderMove> = OnceLock::new();
static PROCESS_PINS: Pins = Pins::new();
static REPORTED: AtomicBool = AtomicBool::new(false);

/// This process's locks: `platform_dirs` holds through them on Windows,
/// outside a sandbox, before every answer.
pub fn process_pins() -> &'static Pins {
    &PROCESS_PINS
}

/// Run the move for this process, once. Only `biorouterd agent` calls it, as
/// it starts and before logging creates its first directory: the daemon owns
/// the folder for its whole life, and the desktop app starts it before
/// anything else of Biorouter's runs. A no-op off Windows and under
/// `BIOROUTER_PATH_ROOT`.
///
/// ⚠ Nothing on the move's path may call a resolver: `OnceLock` does not allow
/// re-entry. Nothing before it should either: a resolver that found the legacy
/// folder holds its lock, and this process's own move then finds it held and
/// reports it in use (safe, but the folder does not move that run).
pub fn move_for_this_process() -> &'static FolderMove {
    PROCESS_MOVE.get_or_init(|| {
        if !cfg!(windows) {
            return FolderMove::default();
        }
        let mut outcome = move_for_process(
            crate::paths::path_root_override(),
            windows_bases,
            &PRODUCTION_OPS,
        );
        PROCESS_PINS.adopt(std::mem::take(&mut outcome.pins));
        outcome
    })
}

/// What this process's move did, if it ran one. For the test binaries' guard
/// that none of them ever does.
pub fn process_move() -> Option<&'static FolderMove> {
    PROCESS_MOVE.get()
}

/// Whether this process holds any legacy folder's lock. For the same guard.
pub fn process_holds_a_lock() -> bool {
    !PROCESS_PINS.is_empty()
}

/// Log what this process's move did, once. The daemon calls it right after
/// logging is set up, because the move itself runs before there is anywhere to
/// log to.
pub fn report_once() {
    let Some(outcome) = PROCESS_MOVE.get() else {
        return;
    };
    if REPORTED.swap(true, Ordering::SeqCst) {
        return;
    }
    for report in [&outcome.roaming, &outcome.local].into_iter().flatten() {
        let current = report.root.current.display();
        let legacy = report.root.legacy.display();
        match &report.outcome {
            MoveOutcome::Moved => tracing::info!(
                "moved Biorouter's folder from {legacy}, the folder earlier releases used, \
                 to {current}"
            ),
            MoveOutcome::BothExist => tracing::warn!(
                "both {current} and {legacy} (the folder earlier releases used) exist; using \
                 {current} and leaving the other untouched, so nothing saved there is shown. \
                 An older copy of the biorouter command line tool may still be writing there: \
                 update it from the desktop app, then copy anything you need out of {legacy}."
            ),
            MoveOutcome::InUse => tracing::warn!(
                "{legacy} (the folder earlier releases used) stays in use for this run because \
                 another Biorouter process is using it. It moves to {current} at a later \
                 start, once none is."
            ),
            MoveOutcome::Unreadable => tracing::warn!(
                "could not check whether {legacy} (the folder earlier releases used) or \
                 {current} exists, so nothing was moved; {legacy} stays in use while it may \
                 hold data"
            ),
            MoveOutcome::MoveFailed(error) => tracing::warn!(
                "could not move {legacy} (the folder earlier releases used) to {current}: \
                 {error}. Using it as it is for this run; the move is tried again at the \
                 next start, once no Biorouter process has a file open in it."
            ),
            MoveOutcome::Fresh | MoveOutcome::AlreadyMoved => {}
        }
    }
    if let Some(rewrite) = &outcome.rewrite {
        rewrite.log();
    }
}
