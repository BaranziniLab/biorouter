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
//! ⚠ **What it does instead is refuse to be SILENT about an OFF answer** (H3 of
//! the 2026-09-10 security test drive). The drive wrote `{"enabled": false}`
//! over this record from a chat's shell; at the next launch every gate was off,
//! and nothing — no log line, no banner — said so. Three things now do:
//!
//! 1. **[`load`] warns, once, whenever the record resolves to OFF**, naming the
//!    file and the fact.
//! 2. **Each door stamps the record it writes** ([`ChangeStamp`]): which door,
//!    and the value it wrote. A record whose value no stamp vouches for is
//!    [`SwitchOrigin::Unrecorded`], and the WARN says "turned off outside the
//!    app". The stamp names its VALUE, not only its door, because the edit an
//!    agent is likeliest to make is a one-field flip (`jq '.enabled = false'`)
//!    that keeps every other byte — a stamp naming only its door would then
//!    vouch for a value it never wrote.
//! 3. **The report is served beside the switch** on the two config read paths
//!    ([`remember`] / [`remembered`], `super::PRIVACY_TIERS_RECORD_KEY`), and
//!    the desktop app shows a standing note above the composer from it.
//!
//! ⚠ **None of the three is a barrier, and the stamp is not a proof.** Anything
//! that can write this file can write a plausible stamp into it; what the stamp
//! reliably catches is the write the drive measured and a one-field flip, not a
//! forger who read this comment. And the file is created owner-only (`0600`),
//! which keeps OTHER local accounts from reading or writing it and does nothing
//! about the agent: the agent's shell runs as the user, so to the operating
//! system it IS the owner — a permission, ownership or integrity check at load
//! cannot tell the two apart, and one keyed to a secret on this machine would
//! only be as strong as the agent's inability to read that secret, which DR-17
//! does not give. The signal exists so the OFF state cannot hide; the switch
//! still obeys the file, exactly as DR-17 accepted.
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
    /// Which door wrote this record, and what it wrote (H3). Absent on a record
    /// no door wrote — and on one written before doors stamped.
    ///
    /// ⚠ **Read leniently, for `changed_at`'s reason.** A stamp that does not
    /// parse — a hand edit, a door a later version adds — reads as *no stamp*
    /// and never fails the record: whether the record reads is `enabled`'s
    /// question alone, and a stamp that could turn a user's `off` into an
    /// unreadable record would fail towards ON in a way they did not ask for.
    #[serde(
        default,
        deserialize_with = "lenient_stamp",
        skip_serializing_if = "Option::is_none"
    )]
    pub changed_by: Option<ChangeStamp>,
}

/// A stamp that fails to parse is no stamp; see [`MasterSwitchRecord::changed_by`].
fn lenient_stamp<'de, D>(deserializer: D) -> Result<Option<ChangeStamp>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(raw).ok())
}

/// The two things in the tree that write the record. There is no third.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeDoor {
    /// Settings → Privacy's typed confirmation, through `/config/upsert`'s
    /// gated arm — [`write_in`].
    Settings,
    /// DR-22's one-time migration, carrying a `config.yaml` value across —
    /// [`migrate_once`].
    Migration,
}

/// What a door leaves on the record it writes (H3): which door, and the value.
///
/// ⚠ **`set_to` is load-bearing, not redundant with `enabled`.** A stamp is a
/// statement about one write. A record whose `enabled` disagrees with its
/// stamp's `set_to` was edited after that write, and [`SwitchReport::of`] reads
/// it as [`SwitchOrigin::Unrecorded`] — which is what makes a one-field flip of
/// a stamped record visible at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeStamp {
    pub via: ChangeDoor,
    /// The value this door wrote.
    pub set_to: bool,
    /// DR-20: the operating system confirmed the person at the keyboard. Only
    /// ever true for an OFF written through Settings, which cannot be written
    /// without it.
    #[serde(default)]
    pub system_authenticated: bool,
    /// DR-16: the request carried the app's own proof of a user, as opposed to
    /// only the daemon secret.
    #[serde(default)]
    pub user_action: bool,
}

/// What the confirmed `/config/upsert` arm knows about who asked — the "who" of
/// a deliberate change, recorded rather than claimed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Confirmation {
    /// DR-20's system prompt was raised for this write and approved.
    pub system_authenticated: bool,
    /// The request carried `X-User-Action` and it verified.
    pub user_action: bool,
}

/// How the record got the value it has — the verdict [`SwitchReport::of`]
/// reaches, and what the WARN and the app's note are worded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchOrigin {
    /// No readable record: the fail-safe ON. Never describes an OFF switch.
    Default,
    /// Written by Settings → Privacy and unchanged since.
    Settings,
    /// Carried across by the migration and unchanged since.
    Migration,
    /// No door recorded writing this value: the record was edited directly, or
    /// written by a version too old to stamp it. For an OFF record this is
    /// "turned off outside the app".
    Unrecorded,
}

/// The last change a door recorded, as the record carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordedChange {
    pub via: ChangeDoor,
    /// What that door wrote — which, on an [`SwitchOrigin::Unrecorded`] record,
    /// is not what the record says now. That disagreement is the evidence.
    pub set_to: bool,
    /// The record's `changed_at`. As forgeable as the rest of the file.
    pub at: String,
    pub system_authenticated: bool,
    pub user_action: bool,
}

/// What the switch's record says and how it got there: logged by [`load`],
/// remembered for the config surface, and served to the renderer as
/// `super::PRIVACY_TIERS_RECORD_KEY`.
///
/// ⚠ **It explains the switch; it never decides it.** `enabled` is what the
/// load resolved, and the process-global atomic every gate reads is set from
/// it by the loader alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SwitchReport {
    pub enabled: bool,
    pub origin: SwitchOrigin,
    /// The record's absolute path, on or off, so a notice can say where to look.
    pub path: String,
    pub last_change: Option<RecordedChange>,
}

impl SwitchReport {
    /// Classify a record read from (or just written to) `path`. `None` is the
    /// absent-or-unreadable record, which the loader resolves to ON.
    pub fn of(path: &Path, record: Option<&MasterSwitchRecord>) -> Self {
        let path = path.display().to_string();
        let Some(record) = record else {
            return Self {
                enabled: true,
                origin: SwitchOrigin::Default,
                path,
                last_change: None,
            };
        };
        let origin = match record.changed_by {
            Some(stamp) if stamp.set_to == record.enabled => match stamp.via {
                ChangeDoor::Settings => SwitchOrigin::Settings,
                ChangeDoor::Migration => SwitchOrigin::Migration,
            },
            _ => SwitchOrigin::Unrecorded,
        };
        let last_change = record.changed_by.map(|stamp| RecordedChange {
            via: stamp.via,
            set_to: stamp.set_to,
            at: record.changed_at.clone(),
            system_authenticated: stamp.system_authenticated,
            user_action: stamp.user_action,
        });
        Self {
            enabled: record.enabled,
            origin,
            path,
            last_change,
        }
    }

    /// The one line [`load`] logs at WARN, or `None` for an enforcing switch —
    /// ON is the default and the common state, and announcing it would teach
    /// every reader of the log to skip the line that matters.
    pub fn off_warning(&self) -> Option<String> {
        if self.enabled {
            return None;
        }
        let path = &self.path;
        let consequence =
            "Every privacy gate and the classification ratchet are disabled in this process.";
        Some(match (self.origin, &self.last_change) {
            (SwitchOrigin::Settings, Some(change)) => format!(
                "privacy tiers are OFF: {path} records that they were turned off in \
                 Settings > Privacy at {at}{confirmed}. {consequence}",
                at = change.at,
                confirmed = if change.system_authenticated {
                    ", confirmed by the operating system"
                } else {
                    ""
                },
            ),
            (SwitchOrigin::Migration, Some(change)) => format!(
                "privacy tiers are OFF: {path} records that they were carried over as off \
                 from config.yaml at {at}. {consequence}",
                at = change.at,
            ),
            (_, last_change) => {
                // Two shapes, and only one of them can be an older version: a
                // Biorouter from before the stamp rewrites the whole record
                // without one, so a stamp that CONTRADICTS the value can only
                // be an edit made after the write it describes.
                let (history, how) = match last_change {
                    Some(change) if change.set_to => (
                        format!(
                            "the last change Biorouter recorded turned them ON at {}",
                            change.at
                        ),
                        "It has been edited since, outside the app — by hand, by a script or \
                         by an agent's shell.",
                    ),
                    _ => (
                        "it records no change made in Settings > Privacy".to_string(),
                        "It was edited directly — by hand, by a script or by an agent's shell \
                         — or written by a Biorouter too old to record changes.",
                    ),
                };
                format!(
                    "privacy tiers are OFF, and they were turned off outside the app: {path} \
                     says they are off, but {history}. {how} {consequence} Turn them back on \
                     in Settings > Privacy."
                )
            }
        })
    }
}

/// What the config surface reports beside the live switch, for this process.
///
/// Written by exactly the two writers of the switch's atomic, each beside its
/// own write — start-up's loader and `/config/upsert`'s gated arm — so the
/// report and the value move together. Nothing else may call [`remember`].
static REPORTED: std::sync::RwLock<Option<SwitchReport>> = std::sync::RwLock::new(None);

/// Replace the report the config surface serves. See [`REPORTED`] for who may.
pub fn remember(report: SwitchReport) {
    *REPORTED.write().unwrap_or_else(|e| e.into_inner()) = Some(report);
}

/// The report, or `None` in a process that has neither loaded nor written the
/// switch — every test binary that pokes the atomic directly, for one.
pub fn remembered() -> Option<SwitchReport> {
    REPORTED.read().unwrap_or_else(|e| e.into_inner()).clone()
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
    read_record_in(config_dir).map(|record| record.enabled)
}

/// The whole record, stamp included, or `None` exactly when [`read_in`] is.
pub fn read_record_in(config_dir: &Path) -> Option<MasterSwitchRecord> {
    let raw = std::fs::read_to_string(path_in(config_dir)).ok()?;
    serde_json::from_str::<MasterSwitchRecord>(&raw).ok()
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
///
/// **This is the Settings door** — `/config/upsert`'s confirmed arm is its only
/// caller — so it stamps [`ChangeDoor::Settings`] with what `confirmation`
/// says, and returns the report the config surface should now serve.
pub fn write_in(
    config_dir: &Path,
    enabled: bool,
    confirmation: Confirmation,
) -> std::io::Result<SwitchReport> {
    std::fs::create_dir_all(config_dir)?;
    let record = stamped(
        enabled,
        ChangeStamp {
            via: ChangeDoor::Settings,
            set_to: enabled,
            system_authenticated: confirmation.system_authenticated,
            user_action: confirmation.user_action,
        },
    );
    let staging = staging_path(config_dir);
    write_owner_only(&staging, &serialise(&record)?)?;
    match std::fs::rename(&staging, path_in(config_dir)) {
        Ok(()) => Ok(SwitchReport::of(&path_in(config_dir), Some(&record))),
        Err(e) => {
            // Do not leave the staging file in the user's config directory.
            let _ = std::fs::remove_file(&staging);
            Err(e)
        }
    }
}

/// A record written now, by the door `stamp` names.
fn stamped(enabled: bool, stamp: ChangeStamp) -> MasterSwitchRecord {
    MasterSwitchRecord {
        enabled,
        changed_at: chrono::Utc::now().to_rfc3339(),
        changed_by: Some(stamp),
    }
}

/// The record's serialised body.
fn serialise(record: &MasterSwitchRecord) -> std::io::Result<String> {
    serde_json::to_string_pretty(record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Create `path` readable and writable by its owner only, where the platform
/// has the notion (H3).
///
/// ⚠ **Hygiene against OTHER accounts, not a control against the agent.** The
/// agent's shell runs as the user, so it owns this file as surely as the user
/// does and can `chmod` it back. What `0600` buys is that a second local
/// account cannot read or rewrite a security record in someone else's config
/// directory — the default umask would leave it world-readable.
///
/// The mode is requested at creation, so a freshly staged record never exists
/// with looser bits, AND set again on the open handle: a staging file left by a
/// crashed process whose pid has since been reused is opened rather than
/// created, and `mode` applies only to a file it creates. Truncating rather than
/// `create_new`, because refusing that leftover would fail the user's flip for a
/// reason they cannot see — `fs::write`, which this replaces, overwrote it.
///
/// ⚠ **The second `chmod` is best-effort, and must stay so.** A configuration
/// directory on a filesystem without Unix modes — exFAT, some network mounts —
/// can refuse it, and hygiene must never be the reason a user's flip fails
/// where `fs::write` used to succeed.
fn write_owner_only(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(path)?;
    #[cfg(unix)]
    let _ = file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600));
    let written = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all());
    if written.is_err() {
        // A torn staging file is litter in the user's config directory; the
        // caller has not published it, so nothing else can be pointing at it.
        let _ = std::fs::remove_file(path);
    }
    written
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
///
/// Stamped [`ChangeDoor::Migration`]: the carried value came out of a
/// `config.yaml` DR-22 names as agent-writable, so it is attributed to the
/// migration that carried it rather than to a Settings change nobody made.
fn claim_in(config_dir: &Path, enabled: bool) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let target = path_in(config_dir);
    let staging = staging_path(config_dir);
    let record = stamped(
        enabled,
        ChangeStamp {
            via: ChangeDoor::Migration,
            set_to: enabled,
            system_authenticated: false,
            user_action: false,
        },
    );
    write_owner_only(&staging, &serialise(&record)?)?;
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
pub fn write_for(
    config: &Config,
    enabled: bool,
    confirmation: Confirmation,
) -> std::io::Result<SwitchReport> {
    write_in(&dir_of(config), enabled, confirmation)
}

/// Resolve the switch from disk — the one-time migration, the read, the
/// classification — and **say so when the answer is OFF** (H3).
///
/// The whole of the disk resolution: `super::resolve_privacy_tiers` is
/// `load(config).enabled`, and `super::load_privacy_tiers_from_config` is this
/// plus [`remember`] plus the atomic. Once per process, so the WARN is once per
/// process — at the moment the gates go dark, which is the moment the drive
/// measured nothing being said.
///
/// ⚠ **The WARN does not change the answer.** An OFF record is obeyed exactly
/// as before — the file channel is DR-17's accepted risk, and a loader that
/// second-guessed the record would be a redesign of the switch, not a signal
/// about it. Absent and unreadable still resolve to ON, silently: that is the
/// fail-safe direction and nothing to announce.
pub fn load(config: &Config) -> SwitchReport {
    migrate_once(config);
    let dir = dir_of(config);
    let report = SwitchReport::of(&path_in(&dir), read_record_in(&dir).as_ref());
    if let Some(warning) = report.off_warning() {
        tracing::warn!(origin = ?report.origin, "{warning}");
    }
    report
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
        write_in(dir.path(), false, Confirmation::default()).unwrap();
        assert_eq!(read_in(dir.path()), Some(false));
        assert!(exists_in(dir.path()));
        write_in(dir.path(), true, Confirmation::default()).unwrap();
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

    /// Formatted tracing output, so a test can assert what a load SAID and at
    /// what level — the same seam `slash_commands.rs` uses.
    #[derive(Clone, Default)]
    struct CapturedLogs(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLogs {
        type Writer = CapturedLogs;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture<T>(f: impl FnOnce() -> T) -> (T, String) {
        let logs = CapturedLogs::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .finish();
        let value = tracing::subscriber::with_default(subscriber, f);
        let text = String::from_utf8_lossy(&logs.0.lock().unwrap()).to_string();
        (value, text)
    }

    fn warnings(logs: &str) -> Vec<&str> {
        logs.lines()
            .filter(|line| line.contains(" WARN "))
            .collect()
    }

    /// H3 (2026-09-10 security test drive): overwriting the record with
    /// `{"enabled": false}` from a chat's shell turned every gate off at the next
    /// launch and NOTHING said so. The file channel is the accepted risk; the
    /// silence was not. The load now says it once, naming the file and the fact.
    #[test]
    fn loading_an_off_record_warns_once_naming_the_file_and_the_fact() {
        let (dir, config) = scratch();
        std::fs::write(path_in(dir.path()), r#"{"enabled": false}"#).unwrap();

        let (on, logs) = capture(|| super::super::resolve_privacy_tiers(&config));

        assert!(!on, "the record says off and the loader must still obey it");
        let warns = warnings(&logs);
        assert_eq!(
            warns.len(),
            1,
            "one WARN at load, not zero and not one per gate: {logs}"
        );
        let path = path_in(dir.path()).display().to_string();
        assert!(
            warns[0].contains(&path),
            "the WARN must name the record, so the reader knows what to inspect: {logs}"
        );
        assert!(
            warns[0].contains("privacy tiers are OFF"),
            "the WARN must state the fact: {logs}"
        );
    }

    /// The measured write carries no trace of the door it did not come
    /// through, and the load says so rather than presenting it like a choice the
    /// user made in Settings → Privacy.
    #[test]
    fn an_off_record_with_no_deliberate_change_recorded_is_flagged_as_outside_the_app() {
        let (dir, config) = scratch();
        std::fs::write(path_in(dir.path()), r#"{"enabled": false}"#).unwrap();

        let (_on, logs) = capture(|| super::super::resolve_privacy_tiers(&config));

        let warns = warnings(&logs);
        assert!(
            warns
                .iter()
                .any(|line| line.contains("turned off outside the app")),
            "an OFF record no door recorded writing must be flagged: {logs}"
        );
    }

    /// The unit half of "reports OFF through the status surface": what the load
    /// resolves is what the loader remembers, and what the config surface
    /// serialises is the report verbatim — OFF, where, and how.
    #[test]
    #[serial_test::serial]
    fn loading_an_off_record_reports_off_through_the_status_surface() {
        let (dir, config) = scratch();
        std::fs::write(path_in(dir.path()), r#"{"enabled": false}"#).unwrap();

        let (report, logs) = capture(|| load(&config));
        assert_eq!(warnings(&logs).len(), 1, "{logs}");
        assert!(!report.enabled);
        assert_eq!(report.origin, SwitchOrigin::Unrecorded);
        assert_eq!(report.path, path_in(dir.path()).display().to_string());

        let previous = remembered();
        remember(report.clone());
        assert_eq!(remembered().as_ref(), Some(&report));
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({
                "enabled": false,
                "origin": "unrecorded",
                "path": path_in(dir.path()).display().to_string(),
                "last_change": null,
            }),
            "the wire shape `privacyTiers.ts` parses"
        );
        if let Some(previous) = previous {
            remember(previous);
        }
    }

    /// The deliberate door stamps what it wrote and who confirmed it, and the
    /// load reads that back as deliberate — no "outside the app".
    #[test]
    fn a_settings_write_is_stamped_and_reads_back_as_deliberate() {
        let (dir, config) = scratch();
        let written = write_for(
            &config,
            false,
            Confirmation {
                system_authenticated: true,
                user_action: true,
            },
        )
        .unwrap();
        assert_eq!(written.origin, SwitchOrigin::Settings);

        let record = read_record_in(dir.path()).expect("the door writes a readable record");
        assert_eq!(
            record.changed_by,
            Some(ChangeStamp {
                via: ChangeDoor::Settings,
                set_to: false,
                system_authenticated: true,
                user_action: true,
            })
        );

        let (report, logs) = capture(|| load(&config));
        assert_eq!(
            report, written,
            "the restart reads back what the door wrote"
        );
        let warns = warnings(&logs);
        assert_eq!(
            warns.len(),
            1,
            "OFF is announced however it got there: {logs}"
        );
        assert!(warns[0].contains("Settings > Privacy"), "{logs}");
        assert!(
            warns[0].contains("confirmed by the operating system"),
            "{logs}"
        );
        assert!(
            !warns[0].contains("outside the app"),
            "a deliberate change was reported as tampering: {logs}"
        );
    }

    /// `jq '.enabled = false'` over a stamped ON record keeps the stamp. The
    /// stamp names the value it wrote, so the flip is still flagged — and the
    /// report keeps the stamp as the evidence.
    #[test]
    fn a_one_field_flip_of_a_stamped_record_is_flagged() {
        let (dir, config) = scratch();
        write_for(&config, true, Confirmation::default()).unwrap();
        let mut record: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path_in(dir.path())).unwrap()).unwrap();
        record["enabled"] = serde_json::Value::Bool(false);
        std::fs::write(path_in(dir.path()), record.to_string()).unwrap();

        let (report, logs) = capture(|| load(&config));
        assert!(!report.enabled);
        assert_eq!(report.origin, SwitchOrigin::Unrecorded);
        assert_eq!(report.last_change.as_ref().map(|c| c.set_to), Some(true));
        let warns = warnings(&logs);
        assert!(
            warns[0].contains("turned off outside the app")
                && warns[0].contains("the last change Biorouter recorded turned them ON"),
            "{logs}"
        );
    }

    /// A carried `off` came out of a `config.yaml` DR-22 names as
    /// agent-writable. It is attributed to the migration that carried it —
    /// neither to a Settings change nobody made nor to tampering nobody did.
    #[test]
    fn a_migrated_off_is_attributed_to_the_migration() {
        let (_dir, config) = scratch();
        config
            .set(
                super::super::PRIVACY_TIERS_CONFIG_KEY,
                &serde_json::Value::String("off".to_string()),
                false,
            )
            .unwrap();

        let (report, logs) = capture(|| load(&config));
        assert!(!report.enabled);
        assert_eq!(report.origin, SwitchOrigin::Migration);
        let warns = warnings(&logs);
        assert!(
            warns[0].contains("carried over as off from config.yaml"),
            "{logs}"
        );
    }

    /// A stamp that does not parse must never make the RECORD unreadable: that
    /// would turn a user's `off` into the fail-safe ON. It reads as no stamp.
    #[test]
    fn a_malformed_stamp_never_fails_the_record() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            path_in(dir.path()),
            r#"{"enabled": false, "changed_by": {"via": "a-door-from-the-future", "set_to": 7}}"#,
        )
        .unwrap();
        let record = read_record_in(dir.path()).expect("the record still reads");
        assert!(!record.enabled);
        assert_eq!(record.changed_by, None);
        assert_eq!(
            SwitchReport::of(&path_in(dir.path()), Some(&record)).origin,
            SwitchOrigin::Unrecorded
        );
    }

    /// Both doors create the record owner-only. Hygiene against OTHER local
    /// accounts — the agent runs as the owner, as the module doc says.
    #[cfg(unix)]
    #[test]
    fn both_doors_create_the_record_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |dir: &Path| {
            std::fs::metadata(path_in(dir))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };

        let settings = tempfile::tempdir().unwrap();
        write_in(settings.path(), false, Confirmation::default()).unwrap();
        assert_eq!(mode(settings.path()), 0o600, "the Settings door");

        let migration = tempfile::tempdir().unwrap();
        claim_in(migration.path(), true).unwrap();
        assert_eq!(mode(migration.path()), 0o600, "the migration's claim");

        // A leftover staging file with looser bits — a crashed process whose pid
        // was reused — is tightened, not published as it was.
        let leftover = tempfile::tempdir().unwrap();
        let staging = staging_path(leftover.path());
        std::fs::write(&staging, "stale").unwrap();
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_owner_only(&staging, "{}").unwrap();
        assert_eq!(
            std::fs::metadata(&staging).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(std::fs::read_to_string(&staging).unwrap(), "{}");
    }

    /// ON is the default and the overwhelmingly common state; announcing it
    /// would train every reader of the log to skip the line that matters.
    #[test]
    fn an_on_record_loads_without_a_warning() {
        let (dir, config) = scratch();
        std::fs::write(path_in(dir.path()), r#"{"enabled": true}"#).unwrap();

        let (on, logs) = capture(|| super::super::resolve_privacy_tiers(&config));

        assert!(on);
        assert!(
            warnings(&logs).is_empty(),
            "an enforcing load must not warn: {logs}"
        );
    }
}
