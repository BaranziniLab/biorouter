//! Moving a renamed extension's installed state to its current id, once, when
//! the daemon starts.
//!
//! [`crate::config::extension_renames`] is the map (`gptragent` -> `gptr`) and
//! the read-time projection every store goes through. This module is the other
//! half: the three stores that describe what is installed NOW are moved to the
//! current id, so the projection has less and less to do.
//!
//! - the config entry in `config.yaml` (its map key, its `name`, and the
//!   `uv run --directory <folder>` argument the installer wrote);
//! - the install folder, `<config>/extensions/<id>`;
//! - the provenance record in `extension-provenance.json`, keyed by the id.
//!
//! Saved chats, workflows, transcripts, `permission.yaml`, hooks and managed
//! policy keep the name they were written with and are resolved when read.
//!
//! ⚠ **Only the daemon moves anything** (`biorouterd agent` calls
//! [`migrate_on_start`]), like `app_folder::move_for_this_process`. The command
//! line tool runs an unmoved install correctly through the projection, and
//! keeping the move out of it means a newer standalone CLI never moves a folder
//! an older desktop daemon's saved chats still point at.
//!
//! # The steps, and why a crash between any two of them is harmless
//!
//! For each rename, under both ids' install locks:
//!
//! 1. Fast exit when nothing names the old id: no config entry, no movable
//!    folder, no provenance record and no journal (every start after the first).
//! 2. Try-lock both install locks, in sorted order. Contended: an install is
//!    running, so skip until the next start.
//! 3. Refuse a conflict (both ids configured, both folders in use, another entry
//!    running from the folder, an entry naming the folder in a way the move
//!    cannot rewrite): log one warning and change nothing.
//! 4. Write the journal `<config>/extension-renames/<from>.json`.
//! 5. Copy the provenance record to the new id, with the moved folder as its
//!    `install_dir` and the same registry id (registry ids never move). A local
//!    install has none, and is given one ([`identity_record`]) so that a build
//!    older than the rename still classifies it once the config says the new id.
//! 6. Move the folder with one `fs::rename`, then resync its environment off the
//!    start path (`uv sync` wrote absolute paths into the `.venv`). Only a
//!    folder a config entry runs from moves: one no entry runs from (beside an
//!    install already under the new id, parked at its credential step, or left
//!    by an uninstall) is left where it is.
//! 7. Rewrite the config entry, in one `Config::edit_values` write.
//! 8. Retire the record under the old id.
//! 9. Remove the journal.
//!
//! | Stopped after | Why it still runs, and stays private | Next start |
//! | --- | --- | --- |
//! | 4 | nothing moved | redo 5 to 9 |
//! | 5 | two records, one registry id; both match | redo 6 to 9 |
//! | 6 | the config still names the old folder, and every reader projects the path to the new one | 7 to 9 |
//! | 7 | config names the new id; the old record is still there | 8 to 9 |
//! | 8 | complete | remove the journal |
//!
//! Nothing is ever rolled back. Every step checks the state it finds, so a
//! failed or interrupted step is simply done again, and the journal is what
//! makes "every start after the first" exit fast only once the move is whole.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use crate::agents::ExtensionConfig;
use crate::config::extension_renames::{self, project_config_in, ExtensionRename};
use crate::config::extensions::{name_to_key, ExtensionEntry};
use crate::config::paths::Paths;
use crate::config::Config;
use crate::privacy::provenance::ExtensionProvenance;

/// `<config>/extension-renames/`, where a move in progress keeps its journal.
pub const JOURNAL_DIR: &str = "extension-renames";
const EXTENSIONS_CONFIG_KEY: &str = "extensions";
const JOURNAL_VERSION: u32 = 1;

/// The name an installer gives a bundle whose manifest declares
/// `manifest_name`: the current id when the manifest names an old one
/// (`gptragent` and `GptrAgent` both install as `gptr`), and the manifest's
/// own spelling otherwise (`CDWAgent` stays `CDWAgent`).
pub fn installed_name(manifest_name: &str) -> String {
    match extension_renames::rename_of(&name_to_key(manifest_name)) {
        Some(rename) => rename.to.clone(),
        None => manifest_name.to_owned(),
    }
}

/// Whether a bundle declaring `manifest_name` is the extension a registry row
/// names `expected`. Every id the extension has had counts, so a row that still
/// says `gptragent` accepts a bundle that says `gptr`, and the reverse.
pub fn is_same_extension(manifest_name: &str, expected: &str) -> bool {
    let expected = name_to_key(expected);
    let bundled = name_to_key(manifest_name);
    extension_renames::all_keys(&expected).contains(&bundled.as_str())
}

/// The provenance record that makes a renamed extension's CURRENT id private in
/// a build that predates the rename, when nothing else records where it came
/// from.
///
/// ⚠ **Why it exists.** Only a marketplace install records provenance. A local
/// install (`biorouter extension install ./gptragent.brxt`, a double-clicked
/// bundle, the desktop's file install) records none, and it is private only by
/// name. Once its config says `gptr` (moved at start, installed under the
/// current id, or written back through the projection), a build that shares
/// this config folder but predates the rename (a lagging terminal CLI, an older
/// desktop beside a newer one) knows neither `gptr` in its compiled set nor any
/// record, and classifies it Public. Every build since DR-23 unions the
/// registry ids of the records found by an entry's key, so a record keyed `gptr`
/// naming the OLD id, which every such build has in its set, keeps it private
/// there. A record can only raise a tier, so a stray one costs nothing.
///
/// No `source_url`: it is not a marketplace install, and a marketplace delete
/// (`marketplace_installs_for_registry_id`) must not take it for one.
fn identity_record(rename: &ExtensionRename, install_dir: Option<&Path>) -> ExtensionProvenance {
    ExtensionProvenance {
        install_id: None,
        registry_id: rename.from.clone(),
        install_dir: install_dir.map(|dir| dir.display().to_string()),
        source_url: None,
        bundle_sha256: None,
        recorded_at: Some(chrono::Utc::now().to_rfc3339()),
    }
}

/// The folder a `uv run --directory <folder>` config runs from, as written.
fn directory_arg(config: &ExtensionConfig) -> Option<&str> {
    let args = args_of(config);
    args.iter()
        .position(|arg| arg == "--directory")
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

/// Record [`identity_record`] for `config` when it is an entry of a renamed
/// extension's current id that no record names yet: none keyed by the current
/// id, and none keyed by the old id that an older build would find through
/// this config's folder (that one moves with the start migration). Returns
/// whether a record was written.
pub fn record_identity_if_missing(
    records: &dyn ProvenanceRecords,
    config: &ExtensionConfig,
) -> std::io::Result<bool> {
    if matches!(
        config,
        ExtensionConfig::Builtin { .. } | ExtensionConfig::Platform { .. }
    ) {
        return Ok(false);
    }
    let key = config.key();
    let Some(rename) = extension_renames::old_keys(&key)
        .next()
        .and_then(extension_renames::rename_of)
    else {
        return Ok(false);
    };
    if records.get(&key).is_some() {
        return Ok(false);
    }
    let folder = directory_arg(config);
    // An older build joins a record to an entry by its key or by an argument
    // EQUAL to the record's folder, so this is the string test it makes.
    let old_record_finds_it = records
        .get(&rename.from)
        .is_some_and(|old| folder.is_some() && old.install_dir.as_deref() == folder);
    if old_record_finds_it {
        return Ok(false);
    }
    records.record(&key, identity_record(rename, folder.map(Path::new)))?;
    Ok(true)
}

/// [`record_identity_if_missing`] for every entry of a config write, on this
/// process's store. Called by `config::extensions` after it saves the
/// extensions map, because every installer and every write of the projection
/// passes there. Never fatal: a missing record costs the older build's tier,
/// which is logged, and failing the write would cost the extension.
pub fn record_identities_of_renamed_installs<'a>(
    configs: impl IntoIterator<Item = &'a ExtensionConfig>,
) {
    for config in configs {
        if let Err(error) = record_identity_if_missing(&ProvenanceFile, config) {
            tracing::warn!(
                extension = config.key().as_str(),
                %error,
                "could not record where a renamed extension came from; a build older than \
                 its rename may read it as public"
            );
        }
    }
}

/// Where the move of one rename got to. Recorded in the journal after each
/// step; the steps themselves decide from what they find on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The journal is on disk; nothing has moved yet.
    Journaled,
    /// The provenance record is under the new id as well.
    ProvenanceCopied,
    /// The install folder is under the new id.
    FolderMoved,
    /// `config.yaml` names the new id.
    ConfigWritten,
    /// The record under the old id is gone.
    OldRecordRetired,
}

impl Phase {
    /// Every phase, in order.
    pub const ALL: [Phase; 5] = [
        Phase::Journaled,
        Phase::ProvenanceCopied,
        Phase::FolderMoved,
        Phase::ConfigWritten,
        Phase::OldRecordRetired,
    ];
}

/// What one start did for one rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameOutcome {
    /// Nothing names the old id.
    NothingToDo,
    /// The move is complete.
    Migrated,
    /// An install of either id holds its lock. Retried at the next start.
    Contended,
    /// Both ids are in use, or something else points into the old folder.
    /// Nothing was changed; both names stay private by name.
    Conflict(String),
    /// [`migrate_stopping_after`] stopped here, as a crash would.
    Stopped(Phase),
    /// A step failed. Everything before it stands, and the next start retries.
    Failed {
        reached: Option<Phase>,
        error: String,
    },
}

/// The provenance store, as the move uses it. The file store in production
/// ([`ProvenanceFile`]); an in-memory one in tests.
pub trait ProvenanceRecords {
    /// The record keyed `key`, if any.
    fn get(&self, key: &str) -> Option<ExtensionProvenance>;
    /// Record `record` under `key`. A record without an `install_id` is given
    /// a fresh one.
    fn record(&self, key: &str, record: ExtensionProvenance) -> std::io::Result<()>;
    /// Remove the record keyed `key` while it is still exactly `expected`.
    fn remove_if_matches(&self, key: &str, expected: &ExtensionProvenance)
        -> std::io::Result<bool>;
}

/// `extension-provenance.json`, through [`crate::privacy::provenance`].
pub struct ProvenanceFile;

impl ProvenanceRecords for ProvenanceFile {
    fn get(&self, key: &str) -> Option<ExtensionProvenance> {
        crate::privacy::provenance::extension_provenance_for_key(key)
    }

    fn record(&self, key: &str, record: ExtensionProvenance) -> std::io::Result<()> {
        crate::privacy::provenance::record(key, record)
    }

    fn remove_if_matches(
        &self,
        key: &str,
        expected: &ExtensionProvenance,
    ) -> std::io::Result<bool> {
        crate::privacy::provenance::remove_extension_provenance_if_matches(key, expected)
    }
}

/// Everything the move reads and writes. [`MigrationEnv::global`] in
/// production; temporary directories and an in-memory record store in tests.
pub struct MigrationEnv<'a> {
    pub config: &'a Config,
    /// `<config>/extensions`.
    pub extensions_root: PathBuf,
    /// `<config>/extension-renames`.
    pub journal_dir: PathBuf,
    /// `<config>/extension-install-locks`, shared with the install transaction.
    pub lock_dir: PathBuf,
    pub provenance: &'a dyn ProvenanceRecords,
    /// Rebuild the moved folder's environment. Production runs `uv sync` on a
    /// thread of its own, so the daemon's start never waits for it.
    pub resync: &'a dyn Fn(&Path),
}

impl MigrationEnv<'static> {
    /// The real stores of this process.
    pub fn global() -> Self {
        Self {
            config: Config::global(),
            extensions_root: crate::extension_install::brxt::extensions_root(),
            journal_dir: Paths::in_config_dir(JOURNAL_DIR),
            lock_dir: crate::extension_install::transaction::install_lock_root(),
            provenance: &ProvenanceFile,
            resync: &resync_in_background,
        }
    }
}

/// Best effort, never on the start path: `uv run` syncs again at the first
/// start of the extension if this fails.
fn resync_in_background(dir: &Path) {
    let dir = dir.to_path_buf();
    let spawned = std::thread::Builder::new()
        .name("extension-rename-resync".to_owned())
        .spawn(move || {
            if let Err(error) = crate::extension_install::brxt::run_uv_sync(&dir) {
                tracing::warn!(
                    dir = %dir.display(),
                    error = %error,
                    "could not rebuild a renamed extension's environment; its first start will"
                );
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the renamed extension's environment rebuild");
    }
}

/// The move for every rename, on this process's real stores, logged. Called
/// once at the start of `biorouterd agent`, and nowhere else.
pub fn migrate_on_start() {
    let env = MigrationEnv::global();
    for (rename, outcome) in migrate(&env) {
        log_outcome(rename, &outcome);
    }
}

fn log_outcome(rename: &ExtensionRename, outcome: &RenameOutcome) {
    let (from, to) = (rename.from.as_str(), rename.to.as_str());
    match outcome {
        RenameOutcome::NothingToDo => {}
        RenameOutcome::Migrated => {
            tracing::info!(from, to, "moved a renamed extension to its current id")
        }
        RenameOutcome::Contended => tracing::info!(
            from,
            to,
            "an install of this extension is running; its rename is retried at the next start"
        ),
        RenameOutcome::Conflict(reason) => tracing::warn!(
            from,
            to,
            reason = reason.as_str(),
            "left a renamed extension under both ids; nothing was changed"
        ),
        RenameOutcome::Stopped(phase) => {
            tracing::info!(from, to, ?phase, "stopped a rename part way, as asked")
        }
        RenameOutcome::Failed { reached, error } => tracing::warn!(
            from,
            to,
            ?reached,
            error = error.as_str(),
            "could not finish moving a renamed extension; the next start retries"
        ),
    }
}

/// The move for every rename in the table, on `env`.
pub fn migrate(env: &MigrationEnv<'_>) -> Vec<(&'static ExtensionRename, RenameOutcome)> {
    extension_renames::renames()
        .iter()
        .map(|rename| (rename, migrate_one(env, rename, None)))
        .collect()
}

/// [`migrate`], stopping after `phase` as a crash there would. A test seam: the
/// state it leaves is one the crash table above says runs.
#[doc(hidden)]
pub fn migrate_stopping_after(
    env: &MigrationEnv<'_>,
    phase: Phase,
) -> Vec<(&'static ExtensionRename, RenameOutcome)> {
    extension_renames::renames()
        .iter()
        .map(|rename| (rename, migrate_one(env, rename, Some(phase))))
        .collect()
}

fn migrate_one(
    env: &MigrationEnv<'_>,
    rename: &ExtensionRename,
    stop_after: Option<Phase>,
) -> RenameOutcome {
    let job = Move::new(env, rename);
    match job.is_needed() {
        Ok(true) => {}
        Ok(false) => return RenameOutcome::NothingToDo,
        Err(error) => {
            return RenameOutcome::Failed {
                reached: None,
                error,
            }
        }
    }
    // Held until this function returns, whatever it returns.
    let _locks = match job.lock() {
        Ok(locks) => locks,
        Err(outcome) => return outcome,
    };
    match job.conflict() {
        Ok(None) => {}
        Ok(Some(reason)) => return RenameOutcome::Conflict(reason),
        Err(error) => {
            return RenameOutcome::Failed {
                reached: None,
                error,
            }
        }
    }
    let mut journal = job.journal();
    let mut reached = None;
    for phase in Phase::ALL {
        if let Err(error) = job
            .step(phase)
            .and_then(|()| job.record_phase(&mut journal, phase))
        {
            return RenameOutcome::Failed { reached, error };
        }
        reached = Some(phase);
        if stop_after == Some(phase) {
            return RenameOutcome::Stopped(phase);
        }
    }
    match std::fs::remove_file(&job.journal_path) {
        Ok(()) => RenameOutcome::Migrated,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => RenameOutcome::Migrated,
        Err(error) => RenameOutcome::Failed {
            reached,
            error: format!("could not remove the rename journal: {error}"),
        },
    }
}

/// One rename's move, on one environment.
struct Move<'e, 'a> {
    env: &'e MigrationEnv<'a>,
    rename: &'e ExtensionRename,
    old_dir: PathBuf,
    new_dir: PathBuf,
    journal_path: PathBuf,
}

impl<'e, 'a> Move<'e, 'a> {
    fn new(env: &'e MigrationEnv<'a>, rename: &'e ExtensionRename) -> Self {
        Self {
            env,
            rename,
            old_dir: env.extensions_root.join(&rename.from),
            new_dir: env.extensions_root.join(&rename.to),
            journal_path: env.journal_dir.join(format!("{}.json", rename.from)),
        }
    }

    /// `config.yaml` as the move judges it. `resolve_spellings` also resolves
    /// every absolute path the entries name, to find one that reaches the old
    /// folder under another spelling; only the conflict check under the locks
    /// asks for it, so the fast exit stats nothing.
    fn view(&self, resolve_spellings: bool) -> Result<ConfigView, String> {
        let values = read_values(self.env.config)
            .map_err(|error| format!("could not read config.yaml: {error}"))?;
        let canonical_old = resolve_spellings
            .then(|| std::fs::canonicalize(&self.old_dir).ok())
            .flatten();
        Ok(view_config(
            &values,
            self.rename,
            &self.old_dir,
            canonical_old.as_deref(),
        ))
    }

    /// Whether the old folder is an install this move carries: it is there,
    /// the new one is not, and a config entry of either id runs from it. A
    /// folder no entry runs from, such as an install parked at its credential
    /// step (its claim names the folder) or what an uninstall left behind
    /// (perhaps with its provenance record), is left where it is: moving it
    /// would only break the claim or carry the leftover under the new id.
    fn folder_moves(&self, view: &ConfigView) -> bool {
        present(&self.old_dir) && !present(&self.new_dir) && view.entries_use_old_folder
    }

    /// Where the extension's folder is once this move is done: the new folder
    /// when it moves or is already there, otherwise wherever its entry runs
    /// from (the old folder included, when nothing moves it).
    fn folder_after_move(&self, view: &ConfigView) -> Option<PathBuf> {
        if present(&self.new_dir) || self.folder_moves(view) {
            return Some(self.new_dir.clone());
        }
        view.directories.first().map(PathBuf::from)
    }

    /// Step 1, the fast exit: whether anything still names the old id. Every
    /// start after the first answers `false` here, without taking a lock.
    fn is_needed(&self) -> Result<bool, String> {
        let view = self.view(false)?;
        Ok(!view.old_entries.is_empty()
            || self.folder_moves(&view)
            || self.env.provenance.get(&self.rename.from).is_some()
            || present(&self.journal_path))
    }

    /// Step 2: both install locks, in sorted order, so a move and an install
    /// of either id never interleave.
    fn lock(&self) -> Result<Vec<std::fs::File>, RenameOutcome> {
        let failed = |error: String| RenameOutcome::Failed {
            reached: None,
            error,
        };
        let mut keys = [self.rename.from.as_str(), self.rename.to.as_str()];
        keys.sort_unstable();
        let mut locks = Vec::with_capacity(keys.len());
        for key in keys {
            let lock = crate::extension_install::transaction::open_install_lock_file_in(
                &self.env.lock_dir,
                key,
            )
            .map_err(|error| failed(format!("could not open the install lock: {error}")))?;
            if let Err(error) = fs2::FileExt::try_lock_exclusive(&lock) {
                if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
                    return Err(RenameOutcome::Contended);
                }
                return Err(failed(format!("could not take the install lock: {error}")));
            }
            locks.push(lock);
        }
        Ok(locks)
    }

    /// Step 3, judged under the locks.
    fn conflict(&self) -> Result<Option<String>, String> {
        let view = self.view(true)?;
        Ok(conflict(
            &view,
            self.rename,
            present(&self.old_dir),
            present(&self.new_dir),
        ))
    }

    /// Step 4: the journal this move left, or a new one.
    fn journal(&self) -> Journal {
        read_journal(&self.journal_path)
            .filter(|journal| journal.from == self.rename.from && journal.to == self.rename.to)
            .unwrap_or_else(|| Journal {
                version: JOURNAL_VERSION,
                from: self.rename.from.clone(),
                to: self.rename.to.clone(),
                old_dir: self.old_dir.display().to_string(),
                new_dir: self.new_dir.display().to_string(),
                phase: Phase::Journaled,
            })
    }

    fn record_phase(&self, journal: &mut Journal, phase: Phase) -> Result<(), String> {
        journal.phase = journal.phase.max(phase);
        write_journal(&self.journal_path, journal)
            .map_err(|error| format!("could not write the rename journal: {error}"))
    }

    /// Steps 4 to 8. Each decides from what it finds, so a step a crash
    /// interrupted, or one a previous start already did, is simply done again.
    fn step(&self, phase: Phase) -> Result<(), String> {
        match phase {
            // The journal itself, which `record_phase` writes.
            Phase::Journaled => Ok(()),
            Phase::ProvenanceCopied => self.copy_record(),
            Phase::FolderMoved => self.move_folder(),
            Phase::ConfigWritten => self.rewrite_config(),
            Phase::OldRecordRetired => self.retire_old_record(),
        }
    }

    /// Step 5. The record under the new id first: from here on the extension
    /// is found by either key, so an older build that knows only the old id
    /// still classifies it through the old record, and this build through
    /// either.
    ///
    /// A local install has no record to copy. It is private by name until
    /// step 7 names it `gptr`, which a build older than the rename does not
    /// know, so it is given the record such a build classifies it by
    /// ([`identity_record`]) before the config says the new id.
    ///
    /// A record already under the new id decides what happens:
    ///
    /// - a marketplace record of the same package (same registry id, with a
    ///   `source_url`): a previous start's copy, or a reinstall since. Kept.
    /// - an identity record (same registry id, no `source_url`), which a
    ///   config write leaves when it cannot tell the old record finds the
    ///   entry (the folder spelled another way), or a local install's own,
    ///   written while it still ran from the old folder: REPLACED by this
    ///   copy, so the marketplace record's `source_url` and digest are not
    ///   lost when step 8 retires it, and the folder is the one the move left.
    /// - a record of another package (another registry id): kept, and step 8
    ///   then keeps the old record too, the only one saying where this
    ///   install came from.
    fn copy_record(&self) -> Result<(), String> {
        let (from, to) = (&self.rename.from, &self.rename.to);
        let view = self.view(false)?;
        let folder = self.folder_after_move(&view);
        let wanted = match self.env.provenance.get(from) {
            Some(old) => moved_record(&old, &self.old_dir, folder.as_deref()),
            None => identity_record(self.rename, folder.as_deref()),
        };
        match self.env.provenance.get(to) {
            None => {}
            Some(existing) if existing.registry_id != wanted.registry_id => return Ok(()),
            Some(existing) if existing.source_url.is_some() => return Ok(()),
            Some(existing)
                if existing.source_url == wanted.source_url
                    && existing.bundle_sha256 == wanted.bundle_sha256
                    && existing.install_dir == wanted.install_dir =>
            {
                return Ok(())
            }
            Some(_) => {}
        }
        self.env
            .provenance
            .record(to, wanted)
            .map_err(|error| format!("could not record provenance under `{to}`: {error}"))
    }

    /// Step 6: one rename within one directory.
    fn move_folder(&self) -> Result<(), String> {
        let (old_dir, new_dir) = (&self.old_dir, &self.new_dir);
        if !present(old_dir) {
            return Ok(());
        }
        if !self.folder_moves(&self.view(false)?) {
            tracing::info!(
                folder = %old_dir.display(),
                "left the unused folder of a renamed extension in place; it can be removed"
            );
            return Ok(());
        }
        std::fs::rename(old_dir, new_dir).map_err(|error| {
            format!(
                "could not move {} to {}: {error}",
                old_dir.display(),
                new_dir.display()
            )
        })?;
        crate::agents::skill_catalog::invalidate();
        (self.env.resync)(new_dir);
        Ok(())
    }

    /// Step 7: `config.yaml`, in one write.
    fn rewrite_config(&self) -> Result<(), String> {
        let root = &self.env.extensions_root;
        self.env
            .config
            .edit_values(|values| rewrite_config(values, self.rename, root))
            .map_err(|error| error.to_string())
            .and_then(|rewritten| rewritten)
            .map(|_| ())
            .map_err(|error| format!("could not rewrite config.yaml: {error}"))
    }

    /// Step 8: the old record goes only once the new one stands for the same
    /// package (the same registry id, and a `source_url` whenever the old
    /// record had one), and only while it is still the record found now (a
    /// reinstall since is left alone). Beside another package's record, or an
    /// identity record step 5 could not replace, it stays.
    fn retire_old_record(&self) -> Result<(), String> {
        let (from, to) = (&self.rename.from, &self.rename.to);
        let Some(old) = self.env.provenance.get(from) else {
            return Ok(());
        };
        let Some(new) = self.env.provenance.get(to) else {
            return Ok(());
        };
        if new.registry_id != old.registry_id
            || (old.source_url.is_some() && new.source_url.is_none())
        {
            tracing::info!(
                from = from.as_str(),
                to = to.as_str(),
                "kept the record under the old id: the record under the new id is not its copy"
            );
            return Ok(());
        }
        self.env
            .provenance
            .remove_if_matches(from, &old)
            .map(|_| ())
            .map_err(|error| format!("could not retire the record under `{from}`: {error}"))
    }
}

/// Why the move may not run on this state, or `None` when it may.
fn conflict(
    view: &ConfigView,
    rename: &ExtensionRename,
    old_present: bool,
    new_present: bool,
) -> Option<String> {
    let (from, to) = (&rename.from, &rename.to);
    if !view.old_entries.is_empty() && !view.new_entries.is_empty() {
        return Some(format!("config.yaml configures both `{from}` and `{to}`"));
    }
    if view.old_entries.len() > 1 {
        return Some(format!(
            "config.yaml configures `{from}` more than once ({})",
            view.old_entries.join(", ")
        ));
    }
    if view.new_entries.len() > 1 {
        return Some(format!(
            "config.yaml configures `{to}` more than once ({})",
            view.new_entries.join(", ")
        ));
    }
    if view.new_key_taken {
        return Some(format!(
            "config.yaml holds another extension under the key `{to}`"
        ));
    }
    // The move rewrites an argument that IS the folder, and nothing else.
    if old_present && !new_present && !view.unfollowable_uses_of_old_folder.is_empty() {
        return Some(format!(
            "the `{from}` folder is named by {} in a way the move cannot follow (a path \
             inside it, or the folder under another spelling)",
            view.unfollowable_uses_of_old_folder.join(", ")
        ));
    }
    // Only a folder that would move can break another entry running from it.
    if old_present && !new_present && !view.foreign_uses_of_old_folder.is_empty() {
        return Some(format!(
            "another extension ({}) runs from the `{from}` folder",
            view.foreign_uses_of_old_folder.join(", ")
        ));
    }
    if old_present
        && new_present
        && (!view.old_entries.is_empty()
            || view.new_entry_uses_old_folder
            || !view.unfollowable_uses_of_old_folder.is_empty())
    {
        return Some(format!(
            "both the `{from}` and the `{to}` folders exist, and the configuration still runs \
             from `{from}`"
        ));
    }
    None
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    version: u32,
    from: String,
    to: String,
    old_dir: String,
    new_dir: String,
    phase: Phase,
}

/// What `config.yaml` holds that the move cares about.
#[derive(Debug, Default)]
struct ConfigView {
    /// Map keys of the entries whose config names the old id.
    old_entries: Vec<String>,
    /// Map keys of the entries whose config names the new id.
    new_entries: Vec<String>,
    /// Whether an entry of the new id still runs from the old folder.
    new_entry_uses_old_folder: bool,
    /// Whether an entry of either id has an argument that IS the old folder,
    /// the one shape the move rewrites.
    entries_use_old_folder: bool,
    /// The `--directory` folders the entries of either id run from, as written.
    directories: Vec<String>,
    /// Map keys of entries of OTHER extensions that run from the old folder or
    /// from a path inside it.
    foreign_uses_of_old_folder: Vec<String>,
    /// Map keys of entries of either id that name the old folder in a way the
    /// move cannot rewrite: a path inside it, or the folder itself under
    /// another spelling (a symlinked base, found only when the view was asked
    /// to resolve spellings).
    unfollowable_uses_of_old_folder: Vec<String>,
    /// A map key equal to the new id held by an entry of another extension.
    new_key_taken: bool,
}

/// How a config names the old folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FolderUse {
    None,
    /// An argument that is the folder, as a path. The move rewrites it.
    Exact,
    /// A path inside the folder, or the folder under another spelling. The
    /// move cannot rewrite it, so the config would stop starting.
    Unfollowable,
}

fn folder_use(config: &ExtensionConfig, old_dir: &Path, canonical_old: Option<&Path>) -> FolderUse {
    let cmd = match config {
        ExtensionConfig::Stdio { cmd, .. } => Some(cmd.as_str()),
        _ => None,
    };
    let mut found = FolderUse::None;
    for value in cmd
        .into_iter()
        .chain(args_of(config).iter().map(String::as_str))
    {
        let path = Path::new(value);
        if !path.is_absolute() {
            continue;
        }
        let this = if path == old_dir {
            FolderUse::Exact
        } else if path.starts_with(old_dir)
            || canonical_old.is_some_and(|canonical_old| {
                std::fs::canonicalize(path).is_ok_and(|real| real.starts_with(canonical_old))
            })
        {
            FolderUse::Unfollowable
        } else {
            FolderUse::None
        };
        found = found.max(this);
    }
    found
}

fn view_config(
    values: &Mapping,
    rename: &ExtensionRename,
    old_dir: &Path,
    canonical_old: Option<&Path>,
) -> ConfigView {
    let mut view = ConfigView::default();
    let Some(Value::Mapping(extensions)) = values.get(EXTENSIONS_CONFIG_KEY) else {
        return view;
    };
    for (stored, raw) in extensions {
        let stored = match stored {
            Value::String(stored) => stored.clone(),
            _ => continue,
        };
        let Ok(entry) = serde_yaml::from_value::<ExtensionEntry>(raw.clone()) else {
            continue;
        };
        let key = entry.config.key();
        // The worse of the config's values: one that names the folder exactly
        // AND one that reaches inside it make the config unfollowable.
        let uses = folder_use(&entry.config, old_dir, canonical_old);
        let uses_old_folder = uses == FolderUse::Exact;
        if key == rename.from || key == rename.to {
            view.entries_use_old_folder |= uses_old_folder;
            if uses == FolderUse::Unfollowable {
                view.unfollowable_uses_of_old_folder.push(stored.clone());
            }
            if let Some(directory) = directory_arg(&entry.config) {
                view.directories.push(directory.to_owned());
            }
        }
        if key == rename.from {
            view.old_entries.push(stored);
        } else if key == rename.to {
            view.new_entry_uses_old_folder |= uses_old_folder;
            view.new_entries.push(stored);
        } else {
            if uses != FolderUse::None {
                view.foreign_uses_of_old_folder.push(stored.clone());
            }
            if stored == rename.to {
                view.new_key_taken = true;
            }
        }
    }
    view
}

fn args_of(config: &ExtensionConfig) -> &[String] {
    match config {
        ExtensionConfig::Stdio { args, .. } => args,
        _ => &[],
    }
}

/// `config.yaml`'s own values, never the environment's. A read through
/// `edit_values` writes nothing, because nothing changes.
fn read_values(config: &Config) -> Result<Mapping, String> {
    config
        .edit_values(|values| values.clone())
        .map_err(|error| error.to_string())
}

fn present(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn read_journal(path: &Path) -> Option<Journal> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_journal(path: &Path, journal: &Journal) -> std::io::Result<()> {
    use std::io::Write;
    let directory = path
        .parent()
        .ok_or_else(|| std::io::Error::other("the journal has no folder"))?;
    std::fs::create_dir_all(directory)?;
    let body = serde_json::to_vec_pretty(journal).map_err(std::io::Error::other)?;
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    temp.write_all(&body)?;
    temp.as_file_mut().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Rewrite every entry of the renamed extension in `values` as the projection
/// reads it, under the new id's map key, in the same position. Other entries
/// keep their raw value, a malformed one included. Returns whether anything
/// changed.
fn rewrite_config(
    values: &mut Mapping,
    rename: &ExtensionRename,
    root: &Path,
) -> Result<bool, String> {
    let Some(Value::Mapping(extensions)) = values.get(EXTENSIONS_CONFIG_KEY) else {
        return Ok(false);
    };
    let mut rewritten = Mapping::with_capacity(extensions.len());
    let mut changed = false;
    for (stored, raw) in extensions {
        let entry = serde_yaml::from_value::<ExtensionEntry>(raw.clone()).ok();
        let Some(entry) = entry.filter(|entry| {
            let key = entry.config.key();
            key == rename.from || key == rename.to
        }) else {
            rewritten.insert(stored.clone(), raw.clone());
            continue;
        };
        let projected = ExtensionEntry {
            enabled: entry.enabled,
            config: project_config_in(entry.config.clone(), root),
        };
        let target = Value::String(rename.to.clone());
        if projected == entry && *stored == target {
            rewritten.insert(stored.clone(), raw.clone());
            continue;
        }
        changed = true;
        let value = serde_yaml::to_value(&projected).map_err(|error| error.to_string())?;
        rewritten.insert(target, value);
    }
    if changed {
        values.insert(
            Value::String(EXTENSIONS_CONFIG_KEY.to_owned()),
            Value::Mapping(rewritten),
        );
    }
    Ok(changed)
}

/// The record the new id gets: the old one with the folder the extension runs
/// from after the move (`folder`) as its `install_dir` when it recorded the old
/// folder, and no `install_id`, so the store gives it a fresh one. A folder
/// that does not move keeps its recorded path.
///
/// ⚠ **Not the same `install_id`.** The provenance journal resolves which of
/// several upserts is live through `current/<key>` pointers that name an
/// install id, and folds its upserts into a map keyed by install id. One id
/// under two keys would let whichever upsert sorts last hide the other, so the
/// record under the new id could vanish while both exist.
fn moved_record(
    old: &ExtensionProvenance,
    old_dir: &Path,
    folder: Option<&Path>,
) -> ExtensionProvenance {
    let mut record = old.clone();
    record.install_id = None;
    if record.install_dir.as_deref().map(Path::new) == Some(old_dir) {
        if let Some(folder) = folder {
            record.install_dir = Some(folder.display().to_string());
        }
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::extension::Envs;
    use indexmap::IndexMap;
    use std::collections::HashMap;
    use std::sync::Mutex;

    const OLD_RECORD_ID: &str = "11111111-2222-3333-4444-555555555555";

    #[derive(Default)]
    struct MemoryRecords {
        records: Mutex<HashMap<String, ExtensionProvenance>>,
        fail_writes: Mutex<bool>,
    }

    impl MemoryRecords {
        fn all(&self) -> HashMap<String, ExtensionProvenance> {
            self.records.lock().unwrap().clone()
        }
    }

    impl ProvenanceRecords for MemoryRecords {
        fn get(&self, key: &str) -> Option<ExtensionProvenance> {
            self.records.lock().unwrap().get(key).cloned()
        }

        fn record(&self, key: &str, mut record: ExtensionProvenance) -> std::io::Result<()> {
            if *self.fail_writes.lock().unwrap() {
                return Err(std::io::Error::other("synthetic provenance write failure"));
            }
            if record.install_id.is_none() {
                record.install_id = Some(uuid::Uuid::new_v4().to_string());
            }
            self.records
                .lock()
                .unwrap()
                .insert(name_to_key(key), record);
            Ok(())
        }

        fn remove_if_matches(
            &self,
            key: &str,
            expected: &ExtensionProvenance,
        ) -> std::io::Result<bool> {
            let mut records = self.records.lock().unwrap();
            if records.get(key) == Some(expected) {
                records.remove(key);
                return Ok(true);
            }
            Ok(false)
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        config: Config,
        records: MemoryRecords,
        resynced: Mutex<Vec<PathBuf>>,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(dir.path().join("extensions")).unwrap();
            let config = Config::new_with_file_secrets(
                dir.path().join("config.yaml"),
                dir.path().join("secrets.yaml"),
            )
            .unwrap();
            Self {
                dir,
                config,
                records: MemoryRecords::default(),
                resynced: Mutex::new(Vec::new()),
            }
        }

        fn root(&self) -> PathBuf {
            self.dir.path().join("extensions")
        }

        fn old_dir(&self) -> PathBuf {
            self.root().join("gptragent")
        }

        fn new_dir(&self) -> PathBuf {
            self.root().join("gptr")
        }

        fn journal(&self) -> PathBuf {
            self.dir.path().join(JOURNAL_DIR).join("gptragent.json")
        }

        fn lock_dir(&self) -> PathBuf {
            self.dir.path().join("extension-install-locks")
        }

        fn stdio(&self, name: &str, dir: &Path) -> ExtensionConfig {
            ExtensionConfig::Stdio {
                name: name.to_owned(),
                description: "Work in a live R session.".to_owned(),
                cmd: "uv".to_owned(),
                args: vec![
                    "run".to_owned(),
                    "--directory".to_owned(),
                    dir.display().to_string(),
                    "gptragent".to_owned(),
                ],
                envs: Envs::new(HashMap::from([(
                    "GPTR_RSCRIPT".to_owned(),
                    "/opt/R/bin/Rscript".to_owned(),
                )])),
                env_keys: vec!["GPTR_TOKEN".to_owned()],
                timeout: Some(120),
                bundled: None,
                available_tools: vec!["sessions".to_owned(), "r".to_owned()],
            }
        }

        fn other(&self, name: &str) -> ExtensionEntry {
            ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::stdio(name, "run-me", "another extension", 30_u64),
            }
        }

        fn write_entries(&self, entries: &[(&str, ExtensionEntry)]) {
            let map: IndexMap<String, ExtensionEntry> = entries
                .iter()
                .map(|(key, entry)| ((*key).to_owned(), entry.clone()))
                .collect();
            self.config.set_param(EXTENSIONS_CONFIG_KEY, &map).unwrap();
        }

        fn entries(&self) -> IndexMap<String, ExtensionEntry> {
            self.config
                .get_param(EXTENSIONS_CONFIG_KEY)
                .unwrap_or_default()
        }

        fn config_bytes(&self) -> Vec<u8> {
            std::fs::read(self.dir.path().join("config.yaml")).unwrap_or_default()
        }

        fn old_record(&self) -> ExtensionProvenance {
            ExtensionProvenance {
                install_id: Some(OLD_RECORD_ID.to_owned()),
                registry_id: "gptragent".to_owned(),
                install_dir: Some(self.old_dir().display().to_string()),
                source_url: Some(
                    "https://github.com/Broccolito/GptrAgent/releases/download/v0.1.2/gptragent.brxt"
                        .to_owned(),
                ),
                bundle_sha256: Some("6de59047".to_owned()),
                recorded_at: Some("2026-11-01T00:00:00Z".to_owned()),
            }
        }

        /// A marketplace install of `gptragent` as an earlier release left it.
        fn install_old(&self) {
            std::fs::create_dir_all(self.old_dir().join(".venv")).unwrap();
            std::fs::write(self.old_dir().join(".venv/marker"), "venv").unwrap();
            self.write_entries(&[
                ("alpha", self.other("alpha")),
                (
                    "gptragent",
                    ExtensionEntry {
                        enabled: false,
                        config: self.stdio("gptragent", &self.old_dir()),
                    },
                ),
                ("omega", self.other("omega")),
            ]);
            self.records
                .records
                .lock()
                .unwrap()
                .insert("gptragent".to_owned(), self.old_record());
        }

        fn run(&self, stop_after: Option<Phase>) -> RenameOutcome {
            let resync = |dir: &Path| self.resynced.lock().unwrap().push(dir.to_path_buf());
            let env = MigrationEnv {
                config: &self.config,
                extensions_root: self.root(),
                journal_dir: self.dir.path().join(JOURNAL_DIR),
                lock_dir: self.lock_dir(),
                provenance: &self.records,
                resync: &resync,
            };
            let rename = extension_renames::rename_of("gptragent").expect("gptragent is renamed");
            migrate_one(&env, rename, stop_after)
        }

        /// The end state every path to completion must reach.
        fn assert_migrated(&self) {
            let entries = self.entries();
            assert_eq!(
                entries.keys().collect::<Vec<_>>(),
                ["alpha", "gptr", "omega"],
                "the entry moved to the new id in place"
            );
            let entry = &entries["gptr"];
            assert!(!entry.enabled, "the operator's switch is kept");
            let ExtensionConfig::Stdio {
                name,
                description,
                args,
                envs,
                env_keys,
                timeout,
                available_tools,
                ..
            } = &entry.config
            else {
                panic!("a stdio entry stays one: {:?}", entry.config)
            };
            assert_eq!(name, "gptr");
            assert_eq!(description, "Work in a live R session.");
            assert_eq!(
                args,
                &[
                    "run".to_owned(),
                    "--directory".to_owned(),
                    self.new_dir().display().to_string(),
                    "gptragent".to_owned(),
                ],
                "the folder follows the move; the entry point is the bundle's own"
            );
            assert_eq!(
                envs.get_env().get("GPTR_RSCRIPT").map(String::as_str),
                Some("/opt/R/bin/Rscript")
            );
            assert_eq!(env_keys, &["GPTR_TOKEN".to_owned()]);
            assert_eq!(*timeout, Some(120));
            assert_eq!(available_tools, &["sessions".to_owned(), "r".to_owned()]);
            assert_eq!(entries["alpha"], self.other("alpha"));
            assert_eq!(entries["omega"], self.other("omega"));

            assert!(!present(&self.old_dir()), "the old folder is gone");
            assert!(
                self.new_dir().join(".venv/marker").is_file(),
                "the folder moved whole, its environment included"
            );

            let records = self.records.all();
            assert_eq!(
                records.keys().collect::<Vec<_>>(),
                ["gptr"],
                "one record, under the new id"
            );
            let record = &records["gptr"];
            assert_eq!(record.registry_id, "gptragent", "registry ids never move");
            assert_eq!(
                record.install_dir.as_deref(),
                Some(self.new_dir().display().to_string().as_str())
            );
            assert_eq!(record.source_url, self.old_record().source_url);
            assert_eq!(record.bundle_sha256, self.old_record().bundle_sha256);
            assert!(record.install_id.is_some());
            assert_ne!(record.install_id.as_deref(), Some(OLD_RECORD_ID));

            assert!(
                !present(&self.journal()),
                "a finished move leaves no journal"
            );
        }

        /// What every reader loads in an intermediate state: the projected
        /// entry runs (its folder exists) and a provenance record still finds
        /// it, by its key or by its folder, so it stays private.
        fn assert_runs_and_stays_private(&self, phase: Phase) {
            let entries = self.entries();
            let touched: Vec<&ExtensionEntry> = entries
                .values()
                .filter(|entry| ["gptr", "gptragent"].contains(&entry.config.key().as_str()))
                .collect();
            assert_eq!(touched.len(), 1, "after {phase:?}: {entries:?}");
            let projected = project_config_in(touched[0].config.clone(), &self.root());
            assert_eq!(projected.key(), "gptr", "after {phase:?}");
            let folder = PathBuf::from(&args_of(&projected)[2]);
            assert!(
                folder.join(".venv/marker").is_file(),
                "after {phase:?} the projected entry runs from {}",
                folder.display()
            );
            let records = self.records.all();
            let found = records.contains_key("gptr")
                || records.values().any(|record| {
                    record.install_dir.as_deref().map(Path::new) == Some(folder.as_path())
                });
            assert!(
                found,
                "after {phase:?} no record finds the entry: {records:?}"
            );
            assert!(
                records
                    .values()
                    .all(|record| record.registry_id == "gptragent"),
                "after {phase:?}"
            );
        }
    }

    #[test]
    fn installers_use_the_current_id_and_keep_every_other_spelling() {
        assert_eq!(installed_name("gptragent"), "gptr");
        assert_eq!(installed_name("GptrAgent"), "gptr");
        assert_eq!(installed_name("gptr"), "gptr");
        assert_eq!(installed_name("CDWAgent"), "CDWAgent");
        assert_eq!(installed_name("spokeagent"), "spokeagent");
    }

    #[test]
    fn a_bundle_matches_a_row_under_any_id_the_extension_has_had() {
        assert!(is_same_extension("gptragent", "gptragent"));
        assert!(is_same_extension("gptr", "gptragent"));
        assert!(is_same_extension("GptrAgent", "gptr"));
        assert!(is_same_extension("CDWAgent", "cdwagent"));
        assert!(!is_same_extension("spokeagent", "gptragent"));
        assert!(!is_same_extension("gptragentx", "gptragent"));
        assert!(!is_same_extension("gptr", "cdwagent"));
    }

    #[test]
    fn a_machine_that_never_had_the_old_id_exits_without_writing() {
        let fixture = Fixture::new();
        fixture.write_entries(&[("alpha", fixture.other("alpha"))]);
        let before = fixture.config_bytes();
        assert_eq!(fixture.run(None), RenameOutcome::NothingToDo);
        assert_eq!(fixture.config_bytes(), before);
        assert!(!present(&fixture.dir.path().join(JOURNAL_DIR)));
        assert!(!present(&fixture.lock_dir()), "a fast exit takes no lock");
        assert!(fixture.resynced.lock().unwrap().is_empty());
    }

    #[test]
    fn an_install_under_the_old_id_moves_whole_and_once() {
        let fixture = Fixture::new();
        fixture.install_old();
        assert_eq!(fixture.run(None), RenameOutcome::Migrated);
        fixture.assert_migrated();
        assert_eq!(
            *fixture.resynced.lock().unwrap(),
            vec![fixture.new_dir()],
            "the moved environment is rebuilt in its new folder"
        );

        // Every start after the first: nothing to do, nothing written.
        let before = fixture.config_bytes();
        assert_eq!(fixture.run(None), RenameOutcome::NothingToDo);
        assert_eq!(fixture.config_bytes(), before);
        fixture.assert_migrated();
        assert_eq!(fixture.resynced.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_move_stopped_after_any_step_runs_meanwhile_and_finishes_next_time() {
        for phase in Phase::ALL {
            let fixture = Fixture::new();
            fixture.install_old();
            assert_eq!(fixture.run(Some(phase)), RenameOutcome::Stopped(phase));
            assert!(
                present(&fixture.journal()),
                "after {phase:?} the journal keeps the move open"
            );
            fixture.assert_runs_and_stays_private(phase);

            assert_eq!(
                fixture.run(None),
                RenameOutcome::Migrated,
                "after {phase:?}"
            );
            fixture.assert_migrated();
            assert_eq!(
                fixture.run(None),
                RenameOutcome::NothingToDo,
                "after {phase:?}"
            );
        }
    }

    #[test]
    fn a_failed_record_write_moves_nothing_and_the_next_start_finishes() {
        let fixture = Fixture::new();
        fixture.install_old();
        let before = fixture.config_bytes();
        *fixture.records.fail_writes.lock().unwrap() = true;
        assert!(matches!(
            fixture.run(None),
            RenameOutcome::Failed {
                reached: Some(Phase::Journaled),
                ..
            }
        ));
        assert_eq!(fixture.config_bytes(), before);
        assert!(fixture.old_dir().join(".venv/marker").is_file());
        assert!(!present(&fixture.new_dir()));
        assert_eq!(
            fixture.records.all().keys().collect::<Vec<_>>(),
            ["gptragent"]
        );

        *fixture.records.fail_writes.lock().unwrap() = false;
        assert_eq!(fixture.run(None), RenameOutcome::Migrated);
        fixture.assert_migrated();
    }

    /// Both ids in use is not ours to settle: both stay private by name, and
    /// the projection leaves the pair alone.
    #[test]
    fn both_ids_configured_or_both_folders_in_use_change_nothing() {
        let both_entries = Fixture::new();
        both_entries.install_old();
        std::fs::create_dir_all(both_entries.new_dir()).unwrap();
        let mut entries: Vec<(String, ExtensionEntry)> =
            both_entries.entries().into_iter().collect();
        entries.push((
            "gptr".to_owned(),
            ExtensionEntry {
                enabled: true,
                config: both_entries.stdio("gptr", &both_entries.new_dir()),
            },
        ));
        let borrowed: Vec<(&str, ExtensionEntry)> = entries
            .iter()
            .map(|(key, entry)| (key.as_str(), entry.clone()))
            .collect();
        both_entries.write_entries(&borrowed);

        let both_folders = Fixture::new();
        both_folders.install_old();
        std::fs::create_dir_all(both_folders.new_dir()).unwrap();

        let foreign = Fixture::new();
        foreign.install_old();
        let mut entries: Vec<(String, ExtensionEntry)> = foreign.entries().into_iter().collect();
        entries.push((
            "rtools".to_owned(),
            ExtensionEntry {
                enabled: true,
                config: foreign.stdio("R tools", &foreign.old_dir()),
            },
        ));
        let borrowed: Vec<(&str, ExtensionEntry)> = entries
            .iter()
            .map(|(key, entry)| (key.as_str(), entry.clone()))
            .collect();
        foreign.write_entries(&borrowed);

        for (label, fixture) in [
            ("both entries", &both_entries),
            ("both folders", &both_folders),
            ("another entry runs from the folder", &foreign),
        ] {
            let config = fixture.config_bytes();
            let records = fixture.records.all();
            let outcome = fixture.run(None);
            assert!(
                matches!(outcome, RenameOutcome::Conflict(_)),
                "{label}: {outcome:?}"
            );
            assert_eq!(fixture.config_bytes(), config, "{label}");
            assert_eq!(fixture.records.all(), records, "{label}");
            assert!(fixture.old_dir().join(".venv/marker").is_file(), "{label}");
            assert!(!present(&fixture.journal()), "{label}");
            assert!(fixture.resynced.lock().unwrap().is_empty(), "{label}");
        }
    }

    /// A newer bundle installed under the new id before the daemon ever ran
    /// leaves the old folder unused. That is not a conflict: the config and the
    /// record move, and the old folder is left where it is.
    #[test]
    fn an_unused_old_folder_beside_the_new_install_is_left_alone() {
        let fixture = Fixture::new();
        fixture.install_old();
        std::fs::create_dir_all(fixture.new_dir().join(".venv")).unwrap();
        std::fs::write(fixture.new_dir().join(".venv/marker"), "venv").unwrap();
        fixture.write_entries(&[
            ("alpha", fixture.other("alpha")),
            (
                "gptr",
                ExtensionEntry {
                    enabled: false,
                    config: fixture.stdio("gptr", &fixture.new_dir()),
                },
            ),
            ("omega", fixture.other("omega")),
        ]);
        assert_eq!(fixture.run(None), RenameOutcome::Migrated);
        assert!(fixture.old_dir().join(".venv/marker").is_file());
        assert_eq!(fixture.records.all().keys().collect::<Vec<_>>(), ["gptr"]);
        assert!(!present(&fixture.journal()));
        assert!(fixture.resynced.lock().unwrap().is_empty());
        assert_eq!(fixture.run(None), RenameOutcome::NothingToDo);
    }

    #[test]
    fn an_install_holding_either_lock_defers_the_move() {
        for key in ["gptr", "gptragent"] {
            let fixture = Fixture::new();
            fixture.install_old();
            let config = fixture.config_bytes();
            let lock = crate::extension_install::transaction::open_install_lock_file_in(
                &fixture.lock_dir(),
                key,
            )
            .unwrap();
            fs2::FileExt::lock_exclusive(&lock).unwrap();

            assert_eq!(fixture.run(None), RenameOutcome::Contended, "{key}");
            assert_eq!(fixture.config_bytes(), config, "{key}");
            assert!(fixture.old_dir().is_dir(), "{key}");
            assert!(!present(&fixture.journal()), "{key}");

            fs2::FileExt::unlock(&lock).unwrap();
            drop(lock);
            assert_eq!(fixture.run(None), RenameOutcome::Migrated, "{key}");
            fixture.assert_migrated();
        }
    }

    /// The private set compiled into every build before the rename: it names
    /// `gptragent` and not `gptr`.
    const SET_BEFORE_THE_RENAME: &[&str] = &["cdwagent", "gptragent", "ucsfomopagent"];

    /// A local install (`biorouter extension install ./gptragent.brxt`, a
    /// double-clicked bundle) records no provenance. Once the move names it
    /// `gptr`, a build that predates the rename and shares this config folder
    /// knows neither `gptr` nor any record, and would offer its tools to a
    /// public model. The move leaves the record such a build classifies it by.
    #[test]
    fn a_record_less_install_is_left_a_record_an_older_build_classifies_it_by() {
        let fixture = Fixture::new();
        fixture.install_old();
        fixture.records.records.lock().unwrap().clear();
        let before = fixture.entries()["gptragent"].config.clone();
        assert_eq!(
            crate::privacy::extensions::tier_with_records(
                SET_BEFORE_THE_RENAME,
                "gptragent",
                Some(&before),
                &HashMap::new()
            ),
            crate::privacy::ProviderTier::Private,
            "before the move an older build is private by name"
        );

        assert_eq!(fixture.run(None), RenameOutcome::Migrated);
        let records = fixture.records.all();
        assert_eq!(records.keys().collect::<Vec<_>>(), ["gptr"]);
        let record = &records["gptr"];
        assert_eq!(
            record.registry_id, "gptragent",
            "the id an older build knows"
        );
        assert_eq!(
            record.install_dir.as_deref(),
            Some(fixture.new_dir().display().to_string().as_str())
        );
        assert_eq!(
            record.source_url, None,
            "not a marketplace install: a marketplace delete must not take it for one"
        );

        let after = fixture.entries()["gptr"].config.clone();
        assert_eq!(after.key(), "gptr");
        assert_eq!(
            crate::privacy::extensions::tier_with_records(
                SET_BEFORE_THE_RENAME,
                "gptr",
                Some(&after),
                &records
            ),
            crate::privacy::ProviderTier::Private,
            "an older build classifies the moved install by the record"
        );
        assert_eq!(
            crate::privacy::extensions::tier_with_records(
                SET_BEFORE_THE_RENAME,
                "gptr",
                Some(&after),
                &HashMap::new()
            ),
            crate::privacy::ProviderTier::Public,
            "non-vacuity: without the record the older build reads it as public"
        );
        assert_eq!(fixture.run(None), RenameOutcome::NothingToDo);
    }

    /// The same, for a config written under the current id by an installer or
    /// by a writer that persisted the projection, with no record anywhere.
    #[test]
    fn a_current_id_entry_without_any_record_is_given_one() {
        let fixture = Fixture::new();
        let config = fixture.stdio("gptr", &fixture.new_dir());
        assert!(record_identity_if_missing(&fixture.records, &config).unwrap());
        let records = fixture.records.all();
        assert_eq!(records.keys().collect::<Vec<_>>(), ["gptr"]);
        assert_eq!(records["gptr"].registry_id, "gptragent");
        assert_eq!(
            records["gptr"].install_dir.as_deref(),
            Some(fixture.new_dir().display().to_string().as_str())
        );
        assert_eq!(
            crate::privacy::extensions::tier_with_records(
                SET_BEFORE_THE_RENAME,
                "gptr",
                Some(&config),
                &records
            ),
            crate::privacy::ProviderTier::Private
        );

        // Never a second record, and never over a marketplace record.
        assert!(!record_identity_if_missing(&fixture.records, &config).unwrap());
        let marketplace = Fixture::new();
        marketplace.install_old();
        let moved = marketplace.stdio("gptr", &marketplace.old_dir());
        assert!(!record_identity_if_missing(&marketplace.records, &moved).unwrap());
        assert_eq!(
            marketplace.records.all().keys().collect::<Vec<_>>(),
            ["gptragent"],
            "the old record already finds this folder, and the move carries it over"
        );

        // An extension that was never renamed, or an old-id entry, is not ours.
        for other in [
            fixture.stdio("spokeagent", &fixture.root().join("spokeagent")),
            fixture.stdio("gptragent", &fixture.old_dir()),
        ] {
            let records = MemoryRecords::default();
            assert!(!record_identity_if_missing(&records, &other).unwrap());
            assert!(records.all().is_empty());
        }
    }

    /// An identity record a config write left under the new id (its folder
    /// spelled unlike the marketplace record's, so the write could not tell
    /// the old record found it) stands for the marketplace record. The move
    /// replaces it with that record's copy rather than discarding the
    /// marketplace record: its `source_url` and digest are what
    /// `delete_extension_package` finds the package by.
    #[test]
    fn an_identity_record_under_the_new_id_gives_way_to_the_marketplace_record() {
        let fixture = Fixture::new();
        fixture.install_old();
        let rename = extension_renames::rename_of("gptragent").unwrap();
        let spelled_otherwise = PathBuf::from(format!("{}/", fixture.old_dir().display()));
        fixture
            .records
            .record("gptr", identity_record(rename, Some(&spelled_otherwise)))
            .unwrap();

        assert_eq!(fixture.run(None), RenameOutcome::Migrated);
        fixture.assert_migrated();
    }

    /// A local install's identity record, written while the install still ran
    /// from the old folder, follows the folder when the move carries it.
    #[test]
    fn an_identity_record_naming_the_old_folder_follows_the_move() {
        let fixture = Fixture::new();
        fixture.install_old();
        fixture.records.records.lock().unwrap().clear();
        let rename = extension_renames::rename_of("gptragent").unwrap();
        fixture
            .records
            .record("gptr", identity_record(rename, Some(&fixture.old_dir())))
            .unwrap();

        assert_eq!(fixture.run(None), RenameOutcome::Migrated);
        let records = fixture.records.all();
        assert_eq!(records.keys().collect::<Vec<_>>(), ["gptr"]);
        assert_eq!(records["gptr"].registry_id, "gptragent");
        assert_eq!(records["gptr"].source_url, None);
        assert_eq!(
            records["gptr"].install_dir.as_deref(),
            Some(fixture.new_dir().display().to_string().as_str())
        );
    }

    /// A record of ANOTHER package under the new id (another registry id) is
    /// not this install's copy, so the record under the old id is kept: it is
    /// the only one that says where this install came from.
    #[test]
    fn the_old_record_stays_beside_another_package_s_record_under_the_new_id() {
        let fixture = Fixture::new();
        fixture.install_old();
        let other_package = ExtensionProvenance {
            install_id: Some("99999999-2222-3333-4444-555555555555".to_owned()),
            registry_id: "gptr".to_owned(),
            install_dir: Some(fixture.new_dir().display().to_string()),
            source_url: Some(
                "https://github.com/Broccolito/gptr/releases/download/bridge-v1.4.0/gptr.brxt"
                    .to_owned(),
            ),
            bundle_sha256: Some("abcdef".to_owned()),
            recorded_at: None,
        };
        fixture
            .records
            .records
            .lock()
            .unwrap()
            .insert("gptr".to_owned(), other_package.clone());

        fixture.run(None);

        let records = fixture.records.all();
        assert_eq!(records.get("gptragent"), Some(&fixture.old_record()));
        assert_eq!(records.get("gptr"), Some(&other_package));
    }

    /// A folder no config entry runs from and no record names is not an
    /// install the move has to carry: an install parked at its credential step
    /// (its claim names this folder), or what an uninstall left behind. It is
    /// left where it is.
    #[test]
    fn a_folder_nothing_runs_from_is_left_where_it_is() {
        let fixture = Fixture::new();
        std::fs::create_dir_all(fixture.old_dir().join(".venv")).unwrap();
        fixture.write_entries(&[("alpha", fixture.other("alpha"))]);
        assert_eq!(fixture.run(None), RenameOutcome::NothingToDo);
        assert!(fixture.old_dir().join(".venv").is_dir());
        assert!(!present(&fixture.new_dir()));
        assert!(fixture.records.all().is_empty());

        // An uninstall that left the folder AND its record: the record moves to
        // the new id (registry ids never move) and still names where the
        // folder is; the folder stays.
        let leftover = Fixture::new();
        leftover.install_old();
        leftover.write_entries(&[("alpha", leftover.other("alpha"))]);
        assert_eq!(leftover.run(None), RenameOutcome::Migrated);
        assert!(leftover.old_dir().join(".venv/marker").is_file());
        assert!(!present(&leftover.new_dir()));
        let records = leftover.records.all();
        assert_eq!(records.keys().collect::<Vec<_>>(), ["gptr"]);
        assert_eq!(
            records["gptr"].install_dir.as_deref(),
            Some(leftover.old_dir().display().to_string().as_str())
        );
        assert_eq!(leftover.run(None), RenameOutcome::NothingToDo);
    }

    /// `<root>/gptragent/` is the same folder, and both the move and the
    /// projection follow it.
    #[test]
    fn a_folder_argument_with_a_trailing_separator_moves_with_the_folder() {
        let fixture = Fixture::new();
        fixture.install_old();
        let trailing = PathBuf::from(format!(
            "{}{}",
            fixture.old_dir().display(),
            std::path::MAIN_SEPARATOR
        ));
        fixture.write_entries(&[
            ("alpha", fixture.other("alpha")),
            (
                "gptragent",
                ExtensionEntry {
                    enabled: false,
                    config: fixture.stdio("gptragent", &trailing),
                },
            ),
            ("omega", fixture.other("omega")),
        ]);
        assert_eq!(fixture.run(None), RenameOutcome::Migrated);
        fixture.assert_migrated();
    }

    /// The move rewrites only an argument that names the old folder itself.
    /// An entry that runs from a path inside it, or from the folder under
    /// another spelling, would stop starting once the folder moved, so the
    /// move is refused and nothing changes.
    #[test]
    fn an_entry_the_move_could_not_follow_is_a_conflict() {
        let inside = Fixture::new();
        inside.install_old();
        let mut config = inside.stdio("gptragent", &inside.old_dir());
        if let ExtensionConfig::Stdio { cmd, args, .. } = &mut config {
            *cmd = inside
                .old_dir()
                .join(".venv/bin/gptragent")
                .display()
                .to_string();
            args.clear();
        }
        inside.write_entries(&[(
            "gptragent",
            ExtensionEntry {
                enabled: true,
                config,
            },
        )]);
        let mut fixtures = vec![("a path inside the folder", inside)];

        #[cfg(unix)]
        {
            let spelled = Fixture::new();
            spelled.install_old();
            let alias = spelled.dir.path().join("alias");
            std::os::unix::fs::symlink(spelled.root(), &alias).unwrap();
            spelled.write_entries(&[(
                "gptragent",
                ExtensionEntry {
                    enabled: true,
                    config: spelled.stdio("gptragent", &alias.join("gptragent")),
                },
            )]);
            fixtures.push(("the folder under another spelling", spelled));
        }

        for (label, fixture) in &fixtures {
            let config = fixture.config_bytes();
            let records = fixture.records.all();
            let outcome = fixture.run(None);
            assert!(
                matches!(&outcome, RenameOutcome::Conflict(reason) if reason.contains("gptragent")),
                "{label}: {outcome:?}"
            );
            assert_eq!(fixture.config_bytes(), config, "{label}");
            assert_eq!(fixture.records.all(), records, "{label}");
            assert!(fixture.old_dir().join(".venv/marker").is_file(), "{label}");
            assert!(!present(&fixture.new_dir()), "{label}");
        }
    }

    /// A config entry without a folder or a record (hand-written, or a folder
    /// someone deleted) still moves to the new id, and its arguments are left
    /// alone because nothing moved.
    #[test]
    fn a_config_entry_alone_moves_to_the_new_id() {
        let fixture = Fixture::new();
        let elsewhere = fixture.dir.path().join("elsewhere");
        fixture.write_entries(&[(
            "GptrAgent",
            ExtensionEntry {
                enabled: true,
                config: fixture.stdio("GptrAgent", &elsewhere),
            },
        )]);
        assert_eq!(fixture.run(None), RenameOutcome::Migrated);
        let entries = fixture.entries();
        assert_eq!(entries.keys().collect::<Vec<_>>(), ["gptr"]);
        assert_eq!(entries["gptr"].config.name(), "gptr");
        assert_eq!(
            args_of(&entries["gptr"].config)[2],
            elsewhere.display().to_string()
        );
        assert_eq!(fixture.run(None), RenameOutcome::NothingToDo);
    }
}
