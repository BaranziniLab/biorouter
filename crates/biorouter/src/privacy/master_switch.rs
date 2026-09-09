//! Where the master privacy switch is **recorded** (issue #56, DR-22).
//!
//! The value itself is [`biorouter_mcp::privacy_toggle`]'s process-global
//! atomic, and it still has exactly two writers — start-up's
//! [`super::load_privacy_tiers_from_config`] and `/config/upsert`'s gated arm.
//! This module is only the disk half: what start-up reads, and what the gated
//! arm writes.
//!
//! ⚠ **Why not `config.yaml`, which is where it used to live.** Task 30 closed
//! the HTTP channel — a bare `/config/upsert` of the key is refused, and the
//! confirmed one carries a typed phrase — but the value's home was a file, and
//! [DR-17] descoped the filesystem barrier that DR-14 had put around
//! `config.yaml`. So *writing the key into the file by hand* stayed a
//! **next-launch** disable of the control the agent is subject to, and "only on
//! restart" is not a control: daemons restart routinely and a model can simply
//! wait. DR-22 moves the value somewhere the retired key cannot reach it.
//!
//! ⚠ **What that does and does not buy, stated plainly, because the honest
//! bound matters more than the flattering one.**
//!
//! * It **closes the key**. `BIOROUTER_PRIVACY_TIERS: off` in `config.yaml` is
//!   inert from the migration onward — the documented, discoverable, widely
//!   quoted recipe (it is in this repo's own docs, in the design, and therefore
//!   in every model's memory of it) no longer does anything. That is the channel
//!   DR-22 names.
//! * It gives the value **one writer**. `config.yaml` is written by nearly every
//!   settings interaction, so a write to it is unremarkable; nothing else in the
//!   tree writes this file, so a write to it is the switch moving and nothing
//!   else.
//! * It does **not** make the file unwritable. DR-17 left every file in the
//!   config directory reachable by an agent holding `developer__shell`, and this
//!   one is no different — the same residual [`super::disclosure`]'s
//!   acknowledgement record carries, and it is recorded here rather than left
//!   for a reader to discover. Closing it needs the filesystem barrier DR-17
//!   deferred, or an OS-authenticated store; neither is in v1, and this module
//!   must not be cited as though it were either.
//!
//! ⚠ **The store is created even when the answer is the default**, and that is
//! load-bearing rather than tidy. Its existence is the migration's "already
//! done" marker (see [`migrate_once`]), and on the overwhelming majority of
//! installs the answer *is* the default — so a store written only on `off` would
//! leave the migration live for ever on almost every machine, which is to say it
//! would leave the retired key live for ever on almost every machine.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Config;

/// The record's filename, beside `config.yaml` in the configuration directory.
pub const SWITCH_FILE_NAME: &str = "privacy-tiers.json";

/// What is written when the switch moves.
///
/// A timestamp beside the flag for the same reason
/// [`super::disclosure::Acknowledgement`] carries one: "when did this machine
/// stop enforcing" is the first question anyone auditing an incident asks, and a
/// bare boolean cannot answer it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterSwitchRecord {
    /// `true` — the default — means every gate and the classification ratchet
    /// are live.
    pub enabled: bool,
    /// RFC 3339, in UTC. `default` so a hand-written `{"enabled": false}` still
    /// reads: this field is an audit aid, not a checksum, and refusing a record
    /// for missing it would fail towards *on* in a way the user did not ask for.
    #[serde(default)]
    pub changed_at: String,
}

/// The directory the record lives in: the one holding `config.yaml`.
///
/// ⚠ **Derived from [`Config::path`], not from
/// [`crate::config::paths::Paths::config_dir`].** `Config::global()` is a
/// `OnceCell` that resolves its path on first access and keeps it, while
/// `Paths::config_dir()` re-reads `BIOROUTER_PATH_ROOT` on every call — so in
/// any process where that variable moves after the config was first touched
/// (every integration-test binary in this tree), the two answer differently.
/// The migration reads one file and writes another; if those two could be
/// resolved from different roots it would migrate across installs. Following
/// the `Config` makes them the same directory by construction, with no second
/// environment read to keep in step.
fn dir_of(config: &Config) -> PathBuf {
    Path::new(&config.path())
        .parent()
        .map(Path::to_path_buf)
        // A `Config` whose path has no parent is not reachable through any
        // constructor in this tree; the current directory keeps this total
        // rather than panicking inside a start-up path.
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Where the record is, for a given configuration directory.
pub fn path_in(config_dir: &Path) -> PathBuf {
    config_dir.join(SWITCH_FILE_NAME)
}

/// Where the record is, for a given [`Config`].
pub fn path_for(config: &Config) -> PathBuf {
    path_in(&dir_of(config))
}

/// The recorded answer, or `None` if this install has not recorded one.
///
/// ⚠ **Fail-safe means fail towards enforcing.** Absent, unreadable and
/// malformed all read `None`, and every caller resolves that to ON — the same
/// polarity the loader has always had, and for the same reason: the failure of
/// the reader must not be a way to disable the control.
///
/// A malformed record is deliberately *not* repaired here. `None` already
/// enforces, and rewriting a file this function is only supposed to read would
/// make a reader into a third writer.
pub fn read_in(config_dir: &Path) -> Option<bool> {
    let raw = std::fs::read_to_string(path_in(config_dir)).ok()?;
    serde_json::from_str::<MasterSwitchRecord>(&raw)
        .ok()
        .map(|record| record.enabled)
}

/// [`read_in`], for the directory a given [`Config`] lives in.
pub fn read_for(config: &Config) -> Option<bool> {
    read_in(&dir_of(config))
}

/// Has this install recorded an answer at all? The migration's "already done"
/// marker.
///
/// Deliberately the file's **existence** and not `read_in(..).is_some()`: a
/// record that fails to parse must not re-open the migration, or a single
/// corrupt byte would make the retired key live again.
pub fn exists_in(config_dir: &Path) -> bool {
    path_in(config_dir).exists()
}

/// Record the answer.
///
/// ⚠ **Staged and renamed, never written in place** — the same hazard
/// [`super::disclosure::record_acknowledgement_in`] documents. `fs::write` opens
/// with `truncate`, so between the truncate and the write the record on disk is
/// empty, and an empty record reads as *nothing recorded*, which resolves to ON.
/// A process that dies inside that window would silently re-enable a feature the
/// user turned off. A rename within one directory is atomic, so the record is
/// only ever absent or complete.
pub fn write_in(config_dir: &Path, enabled: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let staging = staging_path(config_dir);
    std::fs::write(&staging, body(enabled)?)?;
    match std::fs::rename(&staging, path_in(config_dir)) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Do not leave the staging file in the user's config directory.
            let _ = std::fs::remove_file(&staging);
            Err(e)
        }
    }
}

/// The record's serialised body, timestamped now.
fn body(enabled: bool) -> std::io::Result<String> {
    let record = MasterSwitchRecord {
        enabled,
        changed_at: chrono::Utc::now().to_rfc3339(),
    };
    serde_json::to_string_pretty(&record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Where a write stages before it is published. **A fresh path per call.**
///
/// The process id keeps two Biorouter processes writing at the same moment out
/// of each other's staging file. ⚠ **That is not enough on its own**, and the
/// counter is the rest of it: axum runs `/config/upsert` handlers concurrently,
/// so two confirmed flips can be mid-write inside ONE process, and the migration
/// stages through here too. Sharing a path there has their `fs::write` calls
/// interleave into one file — the first publish can land a torn record, which
/// reads as *nothing recorded* (ON) while its caller is told the write
/// succeeded, and the second fails `ENOENT`.
fn staging_path(config_dir: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    config_dir.join(format!(
        "{SWITCH_FILE_NAME}.{}.{seq}.tmp",
        std::process::id()
    ))
}

/// Record the answer **only if this install has none** — the migration's write.
///
/// ⚠ **Why this is not `exists_in` followed by [`write_in`].** That pair is
/// check-then-act, and across processes the interleaving it admits loses the
/// user's answer: P1 records the carried `off` and removes the key; P2, which
/// passed `exists_in` before P1's write, then reads a `config.yaml` P1 has
/// already cleaned, carries nothing, and writes the default `on` over it. Both
/// hosts run the migration at start-up, so the two processes are `biorouterd`
/// and the CLI launched together on the one boot where it runs at all. `Err`
/// with [`std::io::ErrorKind::AlreadyExists`] means somebody else migrated.
///
/// ⚠ **The publish is a `hard_link`, not a `rename`.** It needs to be both
/// *exclusive* (rename silently replaces) and *all-or-nothing* (a `create_new`
/// followed by a write leaves a zero-length record if the process dies between
/// them, and a zero-length record reads as nothing recorded — ON — while
/// permanently blocking the migration that would have carried the user's `off`
/// across). Linking a fully-written staging file into place is the one operation
/// that is both.
fn claim_in(config_dir: &Path, enabled: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let target = path_in(config_dir);
    let staging = staging_path(config_dir);
    std::fs::write(&staging, body(enabled)?)?;
    let claimed = match std::fs::hard_link(&staging, &target) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(e),
        // A filesystem with no hard links at all — FAT, some network mounts —
        // rejects the call for its own reason rather than reporting the target.
        // Ask directly, so a record that is already there still reads as
        // "somebody else migrated" and not as an unexplained I/O failure.
        Err(_) if target.exists() => Err(std::io::ErrorKind::AlreadyExists.into()),
        // Otherwise fall back to the check-then-rename `write_in` uses: the
        // exclusivity is then only best-effort, which is what every platform had
        // before this, and stranding the user's carried answer on such a
        // filesystem would be the worse trade.
        Err(_) => std::fs::rename(&staging, &target),
    };
    // Do not leave the staging file in the user's config directory — on the
    // winning path it is now a second link to the record, on every other path it
    // is litter.
    let _ = std::fs::remove_file(&staging);
    claimed
}

/// [`write_in`], for the directory a given [`Config`] lives in.
pub fn write_for(config: &Config, enabled: bool) -> std::io::Result<()> {
    write_in(&dir_of(config), enabled)
}

/// Carry a pre-DR-22 `config.yaml` value into the store, **once**, and retire
/// the key. Returns whether it did anything.
///
/// ⚠ **This function contains the only read of the retired key in the tree**,
/// and that is the whole of Step 2. A reader that still consults `config.yaml`
/// has not closed the channel, it has added a second one — so the key is
/// *ignored*, not read-and-overridden and not honoured "for compatibility".
///
/// ⚠ **Gated on the STORE's existence, never on the key's.** "Migrate whenever
/// the key is present" is the same compatibility reader wearing a different hat:
/// it would re-run every time the key reappeared, which is exactly the write an
/// agent would make. Because the store is written even for the default answer,
/// one start-up after the upgrade is enough to close the door for good.
///
/// ⚠ **Write first, delete second.** If the store cannot be written — an
/// unwritable configuration directory, or another process claiming it first —
/// the key stays where it is and this returns `false`, so the user's answer is
/// preserved for the next attempt rather than destroyed by a half-finished
/// migration. The interim resolves to ON, which is the safe direction and is
/// what a failed read has always meant.
///
/// ⚠ **The write is [`claim_in`], not [`write_in`].** The `exists_in` short
/// circuit above is an optimisation — it keeps the common start-up from reading
/// the whole values map — and not the exclusion: two processes can both pass it
/// and the loser would otherwise overwrite the winner's carried answer with the
/// default. See `claim_in` for the interleaving.
///
/// ⚠ **The key is deleted only when it was there.** [`Config::delete`] does not
/// check presence — it loads the mapping, removes nothing, and saves it back
/// through `save_values`, which takes a backup and re-serialises the whole file.
/// Called unconditionally it would rewrite every user's hand-maintained
/// `config.yaml` on the first start after the upgrade, stripping comments and
/// formatting to remove a key that was never there, and would make both hosts
/// write that file at startup on that one launch.
///
/// ⚠ This used to add "through a staging path (`config.tmp`) that is not per
/// process". That is **no longer true** — `Config::staging_path` has been per
/// process *and* per call since #188, so two hosts can no longer collide on one
/// staging file. What survives is the plainer hazard, and it is the reason to
/// keep the delete conditional: two hosts each `rename`ing onto `config.yaml`
/// at startup is exactly the storm that made Windows readers of that name fail
/// with "Access is denied.". The config layer tolerates it now; the cheapest
/// place to not need the tolerance is still here.
///
/// ⚠ **The residual, recorded here rather than left to be discovered.** The
/// migration is closed by the STORE's existence, so deleting the store *and*
/// writing the key back re-opens it — two coordinated writes in the
/// configuration directory DR-17 leaves an agent holding `developer__shell` able
/// to reach. It buys that agent nothing it did not already have: one write of
/// `{"enabled": false}` into the store is shorter and does the same thing. The
/// bound is the module header's — DR-22 closes the *documented key*, not the
/// file channel — and this residual is inside it, not beside it.
pub fn migrate_once(config: &Config) -> bool {
    let dir = dir_of(config);
    if exists_in(&dir) {
        return false;
    }
    // Read from the loaded values map, NEVER through `Config::get_param`, whose
    // middle branch resolves an environment variable — the agent holds
    // `developer__shell`, so an env-readable value would make
    // `BIOROUTER_PRIVACY_TIERS=off biorouterd` a one-token disable, and a
    // migration that honoured the environment would hand that lever back on the
    // one start-up where it still mattered.
    let values = config.all_values().ok();
    let recorded = values
        .as_ref()
        .and_then(|values| values.get(super::PRIVACY_TIERS_CONFIG_KEY));
    let carried = recorded.and_then(super::privacy_tiers_value_is_on);
    let key_was_present = recorded.is_some();
    if claim_in(&dir, carried.unwrap_or(true)).is_err() {
        return false;
    }
    if key_was_present {
        // An error here is not worth reporting: the store is already written, so
        // the key is inert whether or not it goes.
        let _ = config.delete(super::PRIVACY_TIERS_CONFIG_KEY);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().expect("temp config dir");
        let config = Config::new_with_file_secrets(
            dir.path().join("config.yaml"),
            dir.path().join("secrets.yaml"),
        )
        .expect("scratch config");
        (dir, config)
    }

    #[test]
    fn nothing_recorded_reads_as_nothing_recorded() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_in(dir.path()), None);
        assert!(!exists_in(dir.path()));
    }

    #[test]
    fn the_record_round_trips_in_both_positions() {
        let dir = tempfile::tempdir().unwrap();
        write_in(dir.path(), false).unwrap();
        assert_eq!(read_in(dir.path()), Some(false));
        assert!(exists_in(dir.path()));
        write_in(dir.path(), true).unwrap();
        assert_eq!(read_in(dir.path()), Some(true));
    }

    /// Fail-safe means fail towards enforcing: a truncated or scribbled-on
    /// record must read as *nothing recorded*, which every caller resolves to
    /// ON. The opposite polarity would make corrupting one file a way to
    /// disable the feature.
    #[test]
    fn a_malformed_record_reads_as_nothing_recorded_but_still_blocks_the_migration() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(path_in(dir.path()), "{").unwrap();
        assert_eq!(read_in(dir.path()), None);
        assert!(
            exists_in(dir.path()),
            "a corrupt record must not re-open the migration: one bad byte would \
             otherwise make the retired key live again"
        );
    }

    /// A record written by hand with only the flag must read. `changed_at` is
    /// an audit aid; refusing a record for missing it would fail towards ON in a
    /// way the user did not ask for.
    #[test]
    fn a_record_without_the_timestamp_still_reads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(path_in(dir.path()), r#"{"enabled": false}"#).unwrap();
        assert_eq!(read_in(dir.path()), Some(false));
    }

    /// The migration's write is an exclusive **claim**, not a plain write.
    ///
    /// `exists_in` then [`write_in`] is check-then-act across processes, and the
    /// interleaving it admits ends with the loser's DEFAULT overwriting the
    /// winner's carried answer: P1 records `off` and removes the key; P2, which
    /// passed `exists_in` before P1's write, then reads a `config.yaml` P1 has
    /// already cleaned, carries nothing, and records the default `on`. The two
    /// processes are not hypothetical — both hosts run the migration at start-up,
    /// so they are `biorouterd` and the CLI launched together on the one boot
    /// where the migration runs at all.
    #[test]
    fn a_second_claim_never_overwrites_the_first() {
        let dir = tempfile::tempdir().unwrap();
        claim_in(dir.path(), false).expect("the first claim wins");

        let err = claim_in(dir.path(), true).expect_err("the second claim must not overwrite");
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            read_in(dir.path()),
            Some(false),
            "the loser's default overwrote the winner's carried answer"
        );

        // Neither the winning nor the losing path may leave staging files in the
        // user's configuration directory.
        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging files left behind: {leftovers:?}"
        );
    }

    /// Two writes in flight **inside one process** must not stage through the
    /// same file.
    ///
    /// Axum runs `/config/upsert` handlers concurrently, so two confirmed flips
    /// can be mid-write at once — and a staging path keyed only on the pid has
    /// their `fs::write` calls interleave into one file: the first publish can
    /// land a torn record, which reads as *nothing recorded* (ON) while its
    /// caller is told the write succeeded, and the second fails `ENOENT`. The
    /// migration's claim stages through the same helper, so on the one start-up
    /// where both run the pair is reachable within a single process too.
    #[test]
    fn two_writes_in_one_process_do_not_stage_through_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_ne!(
            staging_path(dir.path()),
            staging_path(dir.path()),
            "two writes in one process staged through the same file"
        );
    }

    #[test]
    fn the_store_sits_beside_the_config_file_it_migrates_from() {
        let (dir, config) = scratch();
        assert_eq!(path_for(&config), dir.path().join(SWITCH_FILE_NAME));
    }

    #[test]
    fn the_migration_carries_the_retired_value_across_and_removes_the_key() {
        let (_dir, config) = scratch();
        config
            .set(
                super::super::PRIVACY_TIERS_CONFIG_KEY,
                &serde_json::Value::String("off".to_string()),
                false,
            )
            .unwrap();

        assert!(migrate_once(&config));
        assert_eq!(read_for(&config), Some(false));
        assert!(
            !config
                .all_values()
                .unwrap()
                .contains_key(super::super::PRIVACY_TIERS_CONFIG_KEY),
            "the migration must remove the key, not leave it beside the store to drift"
        );
    }

    /// The migration must not touch `config.yaml` on an install that never had
    /// the key — which is ~every install.
    ///
    /// [`Config::delete`] does not check presence: it loads the mapping, removes
    /// nothing, and hands the result to `save_values`, which takes a backup and
    /// re-serialises the whole file. So an unconditional delete rewrites every
    /// user's hand-maintained `config.yaml` on the first start after the upgrade
    /// — stripping their comments and formatting to remove a key that was never
    /// there — and it makes BOTH hosts, `biorouterd` and the CLI, write that
    /// file at startup on that one launch. (This used to add "through a staging
    /// path (`config.tmp`) that is not per process"; staging has been per
    /// process and per call since #188. Two hosts renaming onto `config.yaml`
    /// at once is the hazard that remains — see the doc on `claim_in`.)
    #[test]
    fn an_install_that_never_had_the_key_keeps_its_config_file_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        let hand_written = "# the user wrote this comment\nBIOROUTER_MODEL: gpt-4o\n";
        std::fs::write(&config_path, hand_written).unwrap();
        let config =
            Config::new_with_file_secrets(config_path.clone(), dir.path().join("secrets.yaml"))
                .expect("scratch config");

        assert!(migrate_once(&config));
        assert_eq!(
            std::fs::read_to_string(&config_path).unwrap(),
            hand_written,
            "the migration rewrote a config.yaml it had nothing to remove from"
        );
    }

    /// The default answer is recorded too, and that is what stops the migration
    /// running again on the ~all installs that never disabled anything.
    #[test]
    fn an_install_that_never_set_the_key_still_gets_a_store() {
        let (_dir, config) = scratch();
        assert!(migrate_once(&config));
        assert_eq!(read_for(&config), Some(true));
        assert!(!migrate_once(&config), "the migration runs once");
    }

    /// The failure this closes: writing the key back after the migration must
    /// not migrate a second time.
    #[test]
    fn the_key_written_back_after_the_migration_is_ignored() {
        let (_dir, config) = scratch();
        assert!(migrate_once(&config));
        config
            .set(
                super::super::PRIVACY_TIERS_CONFIG_KEY,
                &serde_json::Value::String("off".to_string()),
                false,
            )
            .unwrap();
        assert!(!migrate_once(&config));
        assert_eq!(
            read_for(&config),
            Some(true),
            "the retired key was read a second time; 'once, at migration' means once"
        );
    }

    /// An environment variable must not reach the one read of the retired key
    /// either. `Config::get_param` resolves env before the file; the migration
    /// reads the values map instead, so this stays true on the single start-up
    /// where the key is still consulted at all.
    #[test]
    #[serial_test::serial]
    fn no_environment_variable_can_reach_the_migration() {
        let (_dir, config) = scratch();
        let _env = env_lock::lock_env([(super::super::PRIVACY_TIERS_CONFIG_KEY, Some("off"))]);
        assert!(migrate_once(&config));
        assert_eq!(
            read_for(&config),
            Some(true),
            "the environment reached the migration's read of the retired key"
        );
    }
}
