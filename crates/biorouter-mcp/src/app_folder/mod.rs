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
//! path under [`LEGACY_APP_AUTHOR`].
//!
//! 1. **Move, once per process, before first use** ([`ensure_moved`]). When
//!    `legacy` exists and `current` does not: drop the empty [`MOVE_MARKER`]
//!    into `legacy`, create `<base>\UCSF`, and rename `legacy` to `current`.
//!    One rename inside one base, so it is atomic and never a copy; nothing is
//!    deleted except a legacy author folder the move left empty.
//! 2. **Resolve, on every call** ([`AppRoot::resolve`]): `current` if it
//!    exists, else `legacy` if it still exists (the rename could not happen
//!    this run, usually because a running older process holds a file in it),
//!    else `current`. When both exist, `current` wins and `legacy` is left
//!    alone.
//! 3. **Rewrite** stored absolute paths that pointed into `legacy`
//!    ([`rewrite`]) while the marker sits in `current`. The marker is written
//!    *before* the rename so it travels with the tree: a crash between the
//!    rename and the rewrite is harmless, because the next start finds the
//!    marker and finishes the job.
//!
//! Several processes may start together (the daemon, the CLI, an `mcp` child,
//! the desktop main process). Every step tolerates that: one rename wins and
//! the others see `current`; a rewrite only touches strings still under
//! `legacy`, so a second pass changes nothing.
//!
//! The desktop main process runs rules 1 and 2 (not 3: it cannot open the
//! session database) before it starts the daemon, in
//! `ui/desktop/src/utils/appFolder.ts`. Both sides run one table of cases,
//! `resolution-cases.json`, and a test here pins the constants equal.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
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

    /// `(legacy exists, current exists)` as of one instant.
    ///
    /// Two separate checks can straddle another process's rename and see the
    /// legacy folder before it and the current one after it, which reads as
    /// "both exist" when only one does. The rename is the one transition that
    /// matters, and it makes `legacy` vanish, so reading `legacy` on both sides
    /// of `current` and retrying when the two disagree gives a consistent pair.
    fn snapshot(&self) -> (bool, bool) {
        loop {
            let legacy = self.legacy.exists();
            let current = self.current.exists();
            if self.legacy.exists() == legacy {
                return (legacy, current);
            }
        }
    }

    /// Rule 2: the folder to read and write right now.
    pub fn resolve(&self) -> &Path {
        if self.current.exists() {
            &self.current
        } else if self.legacy.exists() {
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
            MoveOutcome::MoveFailed(_) => "move-failed",
        }
    }
}

/// How a move renames, and how long it waits between attempts. Production
/// uses [`std::fs::rename`]; tests swap in a rename that refuses.
pub struct MoveOps<'a> {
    pub rename: &'a (dyn Fn(&Path, &Path) -> io::Result<()> + Sync),
    pub retry_delays: &'a [Duration],
}

/// Waits between rename attempts. A directory rename on Windows fails while a
/// file under it is open, which an antivirus scan of freshly written files
/// causes for a moment. A handle a running process holds lasts longer, and then
/// the legacy folder stays in use until the next start.
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

/// Rule 1 for one base directory. `write_marker` is true for the roaming
/// folder, the only one whose stores hold paths that need rewriting.
pub fn move_root(root: &AppRoot, write_marker: bool, ops: &MoveOps<'_>) -> MoveOutcome {
    let outcome = match root.snapshot() {
        (false, false) => MoveOutcome::Fresh,
        (false, true) => MoveOutcome::AlreadyMoved,
        (true, true) => MoveOutcome::BothExist,
        (true, false) => attempt_move(root, write_marker, ops),
    };
    if !root.legacy.exists() {
        // The author folder may hold other apps' folders, and `remove_dir`
        // only ever removes an empty one. This also tidies a folder an
        // interrupted earlier run left empty.
        if let Some(parent) = root.legacy.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
    outcome
}

fn attempt_move(root: &AppRoot, write_marker: bool, ops: &MoveOps<'_>) -> MoveOutcome {
    if write_marker {
        // Never removed again on failure: another process may be about to
        // rename the tree with it inside, and an empty file in a folder that is
        // not moved costs nothing.
        if let Err(error) = fs::File::create(root.legacy.join(MOVE_MARKER)) {
            return settled_by_another_process(root).unwrap_or(MoveOutcome::MoveFailed(error));
        }
    }
    let parent = root.current.parent().map(Path::to_path_buf);
    let created_parent = match &parent {
        Some(parent) if !parent.exists() => match fs::create_dir_all(parent) {
            Ok(()) => true,
            Err(error) => {
                return settled_by_another_process(root).unwrap_or(MoveOutcome::MoveFailed(error))
            }
        },
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
        (false, true) => Some(MoveOutcome::AlreadyMoved),
        (true, true) => Some(MoveOutcome::BothExist),
        _ => None,
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
}

/// Rules 1 and 3 for one process, from explicit inputs so every branch is
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

    let rewrite = (roaming.resolve() == roaming.current.as_path()
        && roaming.current.join(MOVE_MARKER).exists())
    .then(|| rewrite::finish_pending_rewrite(&roaming));

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
static REPORTED: AtomicBool = AtomicBool::new(false);

/// Run the move for this process, once. Every platform resolver calls this
/// before answering, and `biorouterd` and `biorouter` call it before logging
/// creates its first directory. A no-op off Windows and under
/// `BIOROUTER_PATH_ROOT`.
///
/// ⚠ Nothing on the move's path may call a resolver: this runs inside the
/// first one, and `OnceLock` does not allow re-entry.
pub fn ensure_moved() -> &'static FolderMove {
    let mut first = false;
    let outcome = PROCESS_MOVE.get_or_init(|| {
        first = true;
        if !cfg!(windows) {
            return FolderMove::default();
        }
        move_for_process(
            crate::paths::path_root_override(),
            windows_bases,
            &PRODUCTION_OPS,
        )
    });
    // The move usually runs before logging exists, and then the hosts call
    // `report_once` themselves. A process whose first resolver call comes after
    // its logging is up still gets its lines.
    if first && tracing::dispatcher::has_been_set() {
        report_once();
    }
    outcome
}

/// Log what this process's move did, once. The hosts call it right after
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
                 {current} and leaving the other untouched. An older copy of the biorouter \
                 command line tool may still be writing there: run `biorouter setup-path` \
                 from the current release to replace it."
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
