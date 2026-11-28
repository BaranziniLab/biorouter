//! Where an installed extension came from — issue #56, [DR-23].
//!
//! DR-23 says an extension's tier is **re-derived from the registry at read
//! time, keyed on a stable identifier, and never written onto the local config
//! entry**. Measurement said the stable identifier did not exist: a `.brxt`
//! install recorded no provenance whatsoever. `BrxtInstallModal.tsx` wrote
//! `{name, description, type, cmd, args, envs, env_keys, timeout}` into
//! `ExtensionConfig::Stdio` and `installBrxtBundle` returned only
//! `{ installDir }`, so the only join available was the accident that
//! `name_to_key` reduces the registry `id` (`cdwagent`) and the bundle
//! `manifest.name` (`CDWAgent`) to the same string. Rename the config entry and
//! the join misses — which removed **enforcement**, since Gates C, E, F1 and F2
//! all read that answer, not merely the badge.
//!
//! This module is the missing half: the install path records the registry id
//! (plus the download URL and the bundle's hash, which cost nothing to keep and
//! are the two facts an incident response would want) beside the config entry,
//! and [`classify_extension`](super::classify_extension) resolves through it.
//!
//! # Why this is not on `ExtensionConfig`
//!
//! `extension_manager.rs` already records the reason `Extension.tier` was kept
//! off that struct: `ExtensionConfig` round-trips through user-writable
//! `config.yaml`, so anything on it is agent-editable. That is the exact hole
//! DR-23 exists to close, so provenance lives in its own file.
//!
//! # Why this file needs no gated writer — for the TIER
//!
//! Because the resolver **unions** rather than replaces. `classify_extension`
//! returns Private when *either* the config name or the recorded registry id is
//! in the compiled marketplace snapshot, so a record can only ever RAISE a
//! tier. Forging one, corrupting the file, or deleting it outright cannot lower
//! the tier below what the name alone already said — which is precisely
//! DR-23's own argument that re-deriving *removes* the problem instead of
//! guarding it. Sessions have exactly one gated writer because a session's tier
//! is stored; an extension's is not, and this file stores identity, not tier.
//!
//! ⚠ **That argument is about the tier and does not extend to the AFFILIATION,
//! which Task 47 added to the same resolution.** Read this paragraph before
//! citing the one above as the reason a new consumer needs no writer gate.
//!
//! Union raises a tier because Private is the restrictive end of that lattice.
//! On the third axis the direction inverts: an
//! [`ExtensionAffiliation::Institutions`](super::affiliation::ExtensionAffiliation::Institutions)
//! set is an **allowlist**, so unioning two of them produces a *more* permissive
//! extension — reachable from more institutions' models without the
//! cross-affiliation warning DR-26 requires. An extra identity therefore raises
//! the tier and relaxes the affiliation in the same step.
//!
//! It is reachable the same way the tier's identities are: `config.yaml` is
//! agent-writable (DR-17 descoped the filesystem barrier), and an entry whose
//! `args` name another affiliated extension's recorded `install_dir` acquires
//! that extension's institutions. Where the name alone answers `{ucsf}`, the
//! unioned answer `{ucsf, stanford}` clears a Stanford-bound model's mismatch
//! silently — and DR-26 is explicit that an agent may never clear a mismatch
//! automatically.
//!
//! ⚠ **Latent, not live, and the distinction is the whole of the risk
//! assessment.** Every affiliated extension this build ships is `ucsf`, so every
//! union that can currently be formed is `{ucsf}` and no relaxation exists to
//! reach. It goes live the day a second institution enters `INSTITUTIONS` — i.e.
//! before Phase 6 is finished, not at some indefinite later date. The union is
//! what Task 47 specifies ("a base matching several institutions carries all of
//! them"), and *carrying* several institutions' data argues for the opposite
//! compatibility rule from *permitting* several institutions' models, so this
//! needs an explicit ruling rather than an implementer's guess. Recorded here
//! because whoever builds Tasks 48-51 will arrive at this header looking for the
//! reason forged records are harmless, and for the affiliation they are not.
//!
//! ⚠ **What a forged record CAN do is deny, and that is worth saying out loud.**
//! Raise-only means no disclosure, not no harm: a record naming a private
//! registry id and an `install_dir` shared by many configs would mark all of
//! them Private, and a public model would then be refused by every one. So the
//! directory match is deliberately narrow — whole-argument equality, and only
//! against a recorded value shaped like a path (`looks_like_a_path`), which is
//! what stops a record claiming `install_dir: "run"` from matching the `run` in every
//! `uv run --directory …` config the marketplace writes. Nothing here is a
//! privilege boundary; it is the difference between a wrong record costing one
//! extension and costing all of them.
//!
//! # Two implementations, one format
//!
//! The writer that matters today is TypeScript
//! (`ui/desktop/src/utils/extensionProvenance.ts`), because the registry id only
//! exists where the marketplace install happens. So [`PROVENANCE_FILE`], the
//! schema version, the field names and the [`name_to_key`] reduction all exist
//! twice, and each side unit-tests its own half against a hand-written fixture
//! rather than a shared one.
//!
//! ⚠ **Drift is silent and it is a DOWNGRADE**, which is the direction that
//! matters: a record the reader cannot find reads as "no provenance", and no
//! provenance returns a renamed entry to the config-name join. It does not throw
//! and it does not warn. The known divergence is deliberate and test-only — the
//! desktop hardcodes `~/.config/biorouter` while this side resolves through
//! [`Paths`], so the `BIOROUTER_PATH_ROOT` seam moves one and not the other.
//! Anything else that diverges is a bug in whichever side moved.
//!
//! # What is stored
//!
//! `<config dir>/extension-provenance.json`, keyed by
//! [`name_to_key`](crate::config::extensions::name_to_key) of the config
//! entry's name — the same reduction the extension manager stores its map keys
//! under, so a lookup cannot miss on spelling:
//!
//! ```json
//! {
//!   "version": 1,
//!   "extensions": {
//!     "cdwagent": {
//!       "registry_id": "cdwagent",
//!       "source_url": "https://github.com/…/cdwagent.brxt",
//!       "bundle_sha256": "…",
//!       "recorded_at": "2026-08-03T19:00:00Z"
//!     }
//!   }
//! }
//! ```
//!
//! [DR-23]: ../../../../docs/security/privacy-tiers-implementation-plan.md

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::config::extensions::name_to_key;
use crate::config::paths::Paths;

/// The file's name inside the config dir. Duplicated in the desktop app's main
/// process (`ui/desktop/src/main.ts`), which is the process that performs a
/// `.brxt` install and therefore the only one that knows the registry id — the
/// same way that handler already hardcodes `~/.config/biorouter/extensions`.
/// The two spellings must agree; `the_store_lives_in_the_config_dir_under_the_documented_name`
/// pins this half and the TypeScript side names this constant in a comment.
pub const PROVENANCE_FILE: &str = "extension-provenance.json";
const MUTATIONS_DIR_SUFFIX: &str = ".d";

/// The current on-disk schema version. A file written by a newer build is read
/// **for the fields this build understands** rather than discarded: discarding
/// it would silently drop provenance, and dropping provenance lowers a tier.
const SCHEMA_VERSION: u32 = 1;

/// Where one installed extension came from.
///
/// `registry_id` and `install_dir` participate in the tier decision — the first
/// is what is looked up, the second is one of the two ways the record is found.
/// `source_url` and `bundle_sha256` are recorded because the install already
/// has them and an incident response would not: they are evidence, not inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionProvenance {
    /// Unique identity of this install. New writers always set it so a delete
    /// of an older package cannot match a concurrent reinstall of the same
    /// version into the same directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_id: Option<String>,
    /// The BAAM registry `id` this extension was installed from — the stable
    /// identifier DR-23 keys on. Reduced with `name_to_key` before it is
    /// compared with the compiled snapshot, so either spelling the registry
    /// publishes (`cdwagent` or `CDWAgent`) matches.
    pub registry_id: String,
    /// Where the bundle was unpacked — `~/.config/biorouter/extensions/<name>`.
    ///
    /// ⚠ **This is the field that survives a rename**, and it is the reason the
    /// map key alone is not enough. `config.yaml`'s entry can be renamed after
    /// the install, which changes both the map key and the entry's `name` — but
    /// not the `--directory` argument the install wrote, because moving that
    /// would stop the extension launching at all. See [`registry_ids_for`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_dir: Option<String>,
    /// The URL the `.brxt` was downloaded from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// Hex SHA-256 of the downloaded bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_sha256: Option<String>,
    /// RFC 3339 timestamp of the install that wrote this record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<String>,
}

/// The non-secret identity needed to validate deletion of one marketplace
/// package. The config key and install directory are captured together so a
/// caller can re-read and compare the same record after user approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketplaceInstallProvenance {
    pub config_key: String,
    pub install_id: Option<String>,
    pub registry_id: String,
    pub install_dir: String,
    pub source_url: String,
}

/// ⚠ **`PartialEq`/`Eq` are load-bearing, not derive-everything hygiene.**
/// `compaction_is_lossless` is the only test that catches the failure mode that
/// actually matters — a compactor that "shrinks" the journal by dropping
/// records rather than by folding them — and it can only do that by comparing
/// the whole store either side of the fold. A derive on the *field* type
/// ([`ExtensionProvenance`]) does not give the container `==`, so without this
/// the assertion does not compile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    extensions: HashMap<String, ExtensionProvenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum ProvenanceMutation {
    Upsert {
        key: String,
        record: ExtensionProvenance,
    },
    DeleteIfMatches {
        expected: MarketplaceInstallProvenance,
    },
    /// Whole-record tombstone for an uninstall that has no marketplace shape.
    ///
    /// [`Self::DeleteIfMatches`] can only name a record that carries BOTH an
    /// `install_dir` and a `source_url`, because those two fields are what
    /// [`record_matches_install`] compares. A record written before those
    /// fields existed, or by a writer that had neither to hand, is therefore
    /// unnameable by it — and `remove_extension`, which deletes extensions that
    /// never came from the marketplace at all, is exactly the caller that meets
    /// those records. It compares the WHOLE record instead, so a concurrent
    /// reinstall that rewrote any field is left alone for the same reason the
    /// narrower tombstone leaves one alone.
    DeleteRecordIfMatches {
        key: String,
        record: ExtensionProvenance,
    },
}

/// One pending deletion, whichever shape named it.
///
/// The journal replay asks this rather than matching on the mutation twice: the
/// `retain` below and the pointer sweep further down must agree about which
/// records are gone, and two separately written predicates are how they would
/// come to disagree.
enum Tombstone {
    Install(MarketplaceInstallProvenance),
    Record {
        key: String,
        record: ExtensionProvenance,
    },
}

impl Tombstone {
    fn covers(&self, key: &str, record: &ExtensionProvenance) -> bool {
        match self {
            Self::Install(expected) => {
                expected.config_key == key && record_matches_install(record, expected)
            }
            Self::Record {
                key: expected_key,
                record: expected,
            } => expected_key == key && expected == record,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CurrentPointer {
    key: String,
    install_id: String,
}

/// The store's path: `<config dir>/extension-provenance.json`.
///
/// Resolved through [`Paths`], so the `BIOROUTER_PATH_ROOT` test seam relocates
/// it with the rest of the config root.
pub fn provenance_path() -> PathBuf {
    Paths::in_config_dir(PROVENANCE_FILE)
}

/// Every registry id recorded for an extension now configured under any of
/// `keys`, **and** for any whose recorded `install_dir` appears among
/// `referenced_paths`.
///
/// ⚠ **All matches, not the first one**, because the caller unions them.
/// Returning one record makes the answer depend on which way in matched first,
/// and the ordering that loses is the one that matters: an entry named after a
/// public extension whose arguments point at a private one's install directory
/// would resolve public on a first-match rule and private on this one.
///
/// # Two ways in, because one of them survives a rename and the other does not
///
/// **By key** is the direct hit and covers the common case, including the one
/// where the registry `id` and the installed name already disagree —
/// `spokeagent-0.4.1` does so in today's catalogue.
///
/// **By install directory** is what closes the actual bug. Renaming the entry
/// in `config.yaml` rewrites the map key and the entry's `name`, so a
/// key-only lookup misses and the tier drops to public — which is precisely
/// the enforcement failure DR-23 names. It does *not* rewrite the
/// `--directory` argument the install wrote into `args`, because that path is
/// where the server's code physically lives: repoint it and the extension
/// stops launching. So the install directory is the link a rename cannot
/// break, and `referenced_paths` is the config's own argument list, matched
/// exactly rather than parsed for a flag.
///
/// ⚠ **The honest limit.** Editing `args` to point at a copy of the
/// directory would evade this, and `config.yaml` is agent-writable
/// ([DR-17](privacy-tiers-execution-plan.md) descoped the filesystem
/// barrier). That is a strictly higher bar than renaming a key — it requires
/// relocating the server's code, not editing a label — and evasion still only
/// returns the answer the config-name join would have given, never anything
/// lower.
///
/// A missing, unreadable or malformed store yields nothing rather than
/// erroring. That is safe because of the union in
/// [`super::classify_extension`]: an empty answer falls back to the
/// config-name join, which is exactly the behaviour that shipped before this
/// task.
pub fn registry_ids_for(keys: &[String], referenced_paths: &[String]) -> Vec<String> {
    let mut ids = Vec::new();
    #[cfg(test)]
    {
        let records = test_records().lock().unwrap();
        collect_ids(records.iter(), keys, referenced_paths, &mut ids);
    }
    let store = cached_store_at(&provenance_path());
    collect_ids(store.extensions.iter(), keys, referenced_paths, &mut ids);
    ids
}

/// Installed marketplace packages carrying this exact trusted registry id.
/// Missing install-directory or source-URL evidence is excluded: it is enough
/// for privacy classification, but not enough to authorize package deletion.
pub fn marketplace_installs_for_registry_id(
    registry_id: &str,
) -> Vec<MarketplaceInstallProvenance> {
    marketplace_installs_in_store(&cached_store_at(&provenance_path()), registry_id)
}

/// Does a RECORDED registry id name the same entry as `wanted`?
///
/// ⚠ Exact, plus one back-compat shape. A registry id is a stable name, but
/// SPOKEAgent's shipped as `spokeagent-0.4.1` — the version baked into the id —
/// and every machine that installed it recorded that string. Making the id
/// version-free is the right fix (the descriptor was advertising it to the model
/// as the extension's NAME, which it never was), but a bare equality test would
/// then orphan those records: `delete_extension_package` resolves the new id,
/// finds no provenance, and refuses to remove a package that is plainly there.
///
/// So a recorded id also matches when it is `wanted` followed by a `-` and a
/// version-shaped tail. Deliberately narrow — `spokeagent-0.4.1` matches
/// `spokeagent`, `spokeagent-nightly` does not — because this widens what a
/// deletion will accept as the same package.
pub fn registry_id_matches(recorded: &str, wanted: &str) -> bool {
    if recorded == wanted {
        return true;
    }
    recorded
        .strip_prefix(wanted)
        .and_then(|tail| tail.strip_prefix('-'))
        .is_some_and(|version| {
            !version.is_empty()
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        })
}

fn marketplace_installs_in_store(
    store: &Store,
    registry_id: &str,
) -> Vec<MarketplaceInstallProvenance> {
    let mut installs = store
        .extensions
        .iter()
        .filter_map(|(config_key, record)| {
            if !registry_id_matches(&record.registry_id, registry_id) {
                return None;
            }
            Some(MarketplaceInstallProvenance {
                config_key: config_key.clone(),
                install_id: record.install_id.clone(),
                registry_id: record.registry_id.clone(),
                install_dir: record.install_dir.clone()?,
                source_url: record.source_url.clone()?,
            })
        })
        .collect::<Vec<_>>();
    installs.sort_by(|left, right| left.config_key.cmp(&right.config_key));
    installs
}

/// Remove exactly the record revalidated after approval. A changed record is
/// left intact so a stale approval cannot apply to a replacement install.
pub fn remove_marketplace_install_provenance(
    expected: &MarketplaceInstallProvenance,
) -> std::io::Result<bool> {
    remove_marketplace_install_provenance_at(&provenance_path(), expected)
}

fn remove_marketplace_install_provenance_at(
    path: &Path,
    expected: &MarketplaceInstallProvenance,
) -> std::io::Result<bool> {
    remove_marketplace_install_provenance_at_with_hook(path, expected, || {})
}

fn remove_marketplace_install_provenance_at_with_hook<F>(
    path: &Path,
    expected: &MarketplaceInstallProvenance,
    after_read: F,
) -> std::io::Result<bool>
where
    F: FnOnce(),
{
    let store = read_store_at(path);
    after_read();
    let matches = store
        .extensions
        .get(&expected.config_key)
        .is_some_and(|record| record_matches_install(record, expected));
    if !matches {
        return Ok(false);
    }
    append_mutation(
        path,
        &ProvenanceMutation::DeleteIfMatches {
            expected: expected.clone(),
        },
    )?;
    invalidate_cache();
    maybe_compact(path);
    Ok(true)
}

/// The provenance record stored under `config_key`, if any.
///
/// Deliberately keyed and NOT joined by install directory, unlike
/// [`registry_ids_for`]. That function unions every way in because it is
/// deciding a privacy tier and a missed record lowers one; this one is naming a
/// record to DELETE, where a second way in could name a different extension's
/// record.
pub fn extension_provenance_for_key(config_key: &str) -> Option<ExtensionProvenance> {
    cached_store_at(&provenance_path())
        .extensions
        .get(config_key)
        .cloned()
}

/// Remove the record at `config_key`, but only while it is still byte-for-byte
/// `expected`.
///
/// The uninstall counterpart of [`remove_marketplace_install_provenance`] for
/// every record that one cannot name — see
/// [`ProvenanceMutation::DeleteRecordIfMatches`]. Returns `false` when the
/// record changed or vanished, so a caller that read it before asking a user
/// can tell "already gone" from "replaced underneath me" and leave a
/// replacement alone.
pub fn remove_extension_provenance_if_matches(
    config_key: &str,
    expected: &ExtensionProvenance,
) -> std::io::Result<bool> {
    remove_extension_provenance_if_matches_at(&provenance_path(), config_key, expected)
}

fn remove_extension_provenance_if_matches_at(
    path: &Path,
    config_key: &str,
    expected: &ExtensionProvenance,
) -> std::io::Result<bool> {
    let store = read_store_at(path);
    if store.extensions.get(config_key) != Some(expected) {
        return Ok(false);
    }
    append_mutation(
        path,
        &ProvenanceMutation::DeleteRecordIfMatches {
            key: config_key.to_owned(),
            record: expected.clone(),
        },
    )?;
    invalidate_cache();
    maybe_compact(path);
    Ok(true)
}

fn record_matches_install(
    record: &ExtensionProvenance,
    expected: &MarketplaceInstallProvenance,
) -> bool {
    record.install_id == expected.install_id
        && record.registry_id == expected.registry_id
        && record.install_dir.as_deref() == Some(expected.install_dir.as_str())
        && record.source_url.as_deref() == Some(expected.source_url.as_str())
}

/// [`registry_ids_for`] over `records` alone, for a test that supplies the
/// whole store (`extensions::tier_with_records`).
#[cfg(test)]
pub(crate) fn registry_ids_in(
    records: &HashMap<String, ExtensionProvenance>,
    keys: &[String],
    referenced_paths: &[String],
) -> Vec<String> {
    let mut ids = Vec::new();
    collect_ids(records.iter(), keys, referenced_paths, &mut ids);
    ids
}

fn collect_ids<'a>(
    records: impl Iterator<Item = (&'a String, &'a ExtensionProvenance)>,
    keys: &[String],
    referenced_paths: &[String],
    into: &mut Vec<String>,
) {
    for (key, record) in records {
        if keys.iter().any(|k| k == key) || matches_install_dir(record, referenced_paths) {
            into.push(record.registry_id.clone());
        }
    }
}

fn matches_install_dir(record: &ExtensionProvenance, referenced_paths: &[String]) -> bool {
    record
        .install_dir
        .as_deref()
        .is_some_and(|dir| looks_like_a_path(dir) && referenced_paths.iter().any(|p| p == dir))
}

/// A recorded `install_dir` is matched only if it is shaped like a path.
///
/// The arguments this is compared against are a whole command line, not a list
/// of paths — every `.brxt` config is `uv run --directory <dir> <entry>`, so
/// `run`, `--directory` and the entry point are all in it. Without this, a
/// record claiming `install_dir: "run"` would match EVERY marketplace-installed
/// extension at once and lend them all its registry id.
///
/// That is denial rather than disclosure, since the union only ever raises — but
/// a single forged line that refuses every extension to every public model is
/// worth one predicate. The predicate is a separator rather than
/// [`Path::is_absolute`] on purpose: the writer is the Electron main process and
/// may be recording a Windows path this build is not compiled for, and losing a
/// legitimate record IS a downgrade.
fn looks_like_a_path(dir: &str) -> bool {
    dir.contains('/') || dir.contains('\\')
}

/// Record where `config_name` came from as an immutable mutation plus an
/// atomically replaced current-install pointer.
///
/// Both the audited manager install path and Electron marketplace installer
/// call this contract. Local `.brxt` installs without a registry id correctly
/// record nothing and retain the config-name privacy fallback.
pub fn record(config_name: &str, provenance: ExtensionProvenance) -> std::io::Result<()> {
    record_at(&provenance_path(), config_name, provenance)
}

fn record_at(
    path: &Path,
    config_name: &str,
    mut provenance: ExtensionProvenance,
) -> std::io::Result<()> {
    if provenance.install_id.is_none() {
        provenance.install_id = Some(uuid::Uuid::new_v4().to_string());
    }
    let key = name_to_key(config_name);
    let install_id = provenance
        .install_id
        .clone()
        .expect("record_at assigns an install id");
    append_mutation(
        path,
        &ProvenanceMutation::Upsert {
            key: key.clone(),
            record: provenance,
        },
    )?;
    write_current_pointer(path, &key, &install_id)?;
    invalidate_cache();
    maybe_compact(path);
    Ok(())
}

fn mutations_dir(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(MUTATIONS_DIR_SUFFIX);
    PathBuf::from(value)
}

fn current_pointers_dir(path: &Path) -> PathBuf {
    mutations_dir(path).join("current")
}

fn pointer_filename(key: &str) -> String {
    key.as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn write_current_pointer(path: &Path, key: &str, install_id: &str) -> std::io::Result<()> {
    let directory = current_pointers_dir(path);
    std::fs::create_dir_all(&directory)?;
    let pointer = CurrentPointer {
        key: key.to_owned(),
        install_id: install_id.to_owned(),
    };
    let body = serde_json::to_vec(&pointer)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut temp = tempfile::NamedTempFile::new_in(&directory)?;
    temp.write_all(&body)?;
    temp.as_file_mut().sync_all()?;
    temp.persist(directory.join(pointer_filename(key)))
        .map_err(|error| error.error)?;
    Ok(())
}

fn append_mutation(path: &Path, mutation: &ProvenanceMutation) -> std::io::Result<()> {
    let directory = mutations_dir(path);
    std::fs::create_dir_all(&directory)?;
    let body = serde_json::to_vec(mutation)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut temp = tempfile::NamedTempFile::new_in(&directory)?;
    temp.write_all(&body)?;
    temp.as_file_mut().sync_all()?;
    let filename = format!("{}.json", uuid::Uuid::new_v4());
    temp.persist(directory.join(filename))
        .map_err(|error| error.error)?;
    Ok(())
}

/// How long a mutation file must have sat on disk before it may be folded.
///
/// ⚠ **This is a correctness window, not a tuning knob.** The writer that
/// matters today is TypeScript (`ui/desktop/src/utils/extensionProvenance.ts`),
/// and it writes the mutation file FIRST and the `current/` pointer SECOND —
/// two independent `rename`s with nothing atomic between them. A compactor that
/// snapshots inside that gap computes a base store that does not contain the
/// record, deletes the mutation file that was its only copy, and leaves the
/// pointer landing a moment later aimed at nothing. The record is then gone
/// with no error anywhere: a silent tier DOWNGRADE, which this module's header
/// names as the one direction that must never happen. Sixty seconds is orders
/// of magnitude wider than the gap between two `renameSync` calls, and being
/// generous costs only that the journal stays long for a minute.
///
/// [`record_at`] on this side has the identical two-step shape
/// (`append_mutation` then `write_current_pointer`), so the window covers a
/// concurrent Rust writer as well as the desktop one — a thread compacting
/// while another is between those two calls is the same hazard.
const COMPACTION_GRACE: Duration = Duration::from_secs(60);

/// How many fold-eligible mutation files must pile up before a writer pays for
/// a compaction pass.
///
/// The cost being bounded is a READ cost, not disk space: [`mutation_stamp`] is
/// part of the cache key, so it walks this directory on every cache HIT, and
/// [`apply_mutations`] re-reads and re-parses every file on every miss — once
/// per installed extension per turn, via Gate E. Thirty-two keeps that walk to a
/// few dozen `stat`s while leaving the ordinary case (a handful of extensions,
/// each installed once) never rewriting the base file at all. It is
/// deliberately not 1: a pass rewrites the whole store, so folding on every
/// install would trade an unbounded read cost for a quadratic write cost.
const COMPACTION_THRESHOLD: usize = 32;

/// Fold the settled part of the mutation journal into the base store file and
/// delete the files that were folded.
///
/// ⚠ **Writers only.** Never call this from [`cached_store_at`]: that runs on
/// the privacy gate path, once per installed extension per turn, and putting a
/// store rewrite there would make every tool dispatch a potential writer.
fn compact_mutations_at(path: &Path) -> std::io::Result<()> {
    // ── Grace window ── see [`COMPACTION_GRACE`]: a file younger than this may
    // still be one half of a writer's two-step mutation-then-pointer write.
    let Some(cutoff) = SystemTime::now().checked_sub(COMPACTION_GRACE) else {
        return Ok(());
    };
    let journal = read_mutations(path);
    let mut foldable = journal
        .iter()
        .map(|(file, _)| settled_before(file, cutoff))
        .collect::<Vec<_>>();

    // ── Tombstone pairing ── a tombstone may be folded ONLY together with
    // every `Upsert` sharing its `install_id`, or not at all.
    // `record_matches_install` compares install ids, so an unsettled upsert is
    // exactly the record a folded tombstone would stop suppressing: folding the
    // tombstone applies the deletion to a base that does not hold the record
    // yet and then removes the tombstone, so the next read re-inserts the
    // record through its `current/` pointer (`if !deleted { … insert(…) }`) —
    // resurrection of provenance the user deleted.
    //
    // ⚠ **Both tombstone shapes, not just the marketplace one.** The pointer
    // sweep in `apply_mutation_list` asks `Tombstone::covers`, which knows both,
    // so a `DeleteRecordIfMatches` left out of this pairing is resurrected by
    // exactly the mechanism above. It is paired on the deleted RECORD's
    // `install_id` because that is the id its `Upsert` carries — the whole-record
    // comparison in `covers` includes that field, so the two agree.
    let mut pinned: HashSet<Option<String>> = HashSet::new();
    for (index, (_, mutation)) in journal.iter().enumerate() {
        if foldable[index] {
            continue;
        }
        if let ProvenanceMutation::Upsert { record, .. } = mutation {
            pinned.insert(record.install_id.clone());
        }
    }
    for (index, (_, mutation)) in journal.iter().enumerate() {
        let install_id = match mutation {
            ProvenanceMutation::DeleteIfMatches { expected } => &expected.install_id,
            ProvenanceMutation::DeleteRecordIfMatches { record, .. } => &record.install_id,
            ProvenanceMutation::Upsert { .. } => continue,
        };
        if pinned.contains(install_id) {
            foldable[index] = false;
        }
    }

    let folded = journal
        .iter()
        .enumerate()
        .filter(|(index, _)| foldable[*index])
        .map(|(_, (_, mutation))| mutation.clone())
        .collect::<Vec<_>>();
    if folded.is_empty() {
        return Ok(());
    }
    let mut store = read_base_store(path);
    apply_mutation_list(path, &mut store, folded);

    // ── Atomic base write ── a torn base file does NOT fail loudly:
    // `read_base_store` turns any parse failure into `Store::default()` behind a
    // `warn!`, so half a file strips every extension's provenance at once and
    // silently. [`write_store_at`] is temp + `write_all` + `sync_all` +
    // `persist`, the shape `append_mutation` and `write_current_pointer` use.
    write_store_at(path, &store)?;

    // Base first, journal second. A crash between the two replays folded
    // mutations onto a base that already holds them, which is idempotent; the
    // other order loses records outright.
    //
    // ── Never delete pointer files ── `current/` is untouched. A pointer whose
    // `install_id` no longer has an upsert is already skipped harmlessly by
    // `apply_mutation_list`, and removing one by name would race a concurrent
    // `write_current_pointer` for the same key.
    for (index, (file, _)) in journal.iter().enumerate() {
        if foldable[index] {
            let _ = std::fs::remove_file(file);
        }
    }
    invalidate_cache();
    Ok(())
}

/// Has `file` sat still for long enough to fold? An unreadable mtime answers
/// no, because the conservative direction here is "leave it alone".
fn settled_before(file: &Path, cutoff: SystemTime) -> bool {
    std::fs::metadata(file)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .is_some_and(|modified| modified <= cutoff)
}

/// Replace the base store file atomically.
///
/// The same temp + `write_all` + `sync_all` + `persist` shape as
/// [`append_mutation`] and [`write_current_pointer`], for the reason spelled out
/// at the call site: a partially written base reads as "no provenance at all".
fn write_store_at(path: &Path, store: &Store) -> std::io::Result<()> {
    let directory = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(directory)?;
    let body = serde_json::to_vec(store)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut temp = tempfile::NamedTempFile::new_in(directory)?;
    temp.write_all(&body)?;
    temp.as_file_mut().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Mutation files old enough to fold — one `read_dir` plus one `metadata` per
/// entry, the same walk [`mutation_stamp`] already performs on every cache hit,
/// so gating a write on it adds no new class of work.
///
/// An UPPER bound: it does not apply the tombstone-pairing rule, which can only
/// ever hold a file back. Over-counting costs one pass that folds less than it
/// hoped; under-counting would let the journal grow past the threshold unseen.
fn fold_eligible_count(path: &Path) -> usize {
    let Some(cutoff) = SystemTime::now().checked_sub(COMPACTION_GRACE) else {
        return 0;
    };
    let Ok(entries) = std::fs::read_dir(mutations_dir(path)) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter(|entry| {
            entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .is_some_and(|modified| modified <= cutoff)
        })
        .count()
}

/// Compact if enough of the journal has settled, from a WRITER.
///
/// A compaction failure is swallowed with a warning: an over-long journal is a
/// performance problem, whereas failing an install or a package deletion
/// because the base file could not be rewritten is a correctness one.
fn maybe_compact(path: &Path) {
    if fold_eligible_count(path) < COMPACTION_THRESHOLD {
        return;
    }
    if let Err(error) = compact_mutations_at(path) {
        tracing::warn!(
            path = %path.display(),
            error = %error,
            "could not compact the extension provenance journal; the next write will retry"
        );
    }
}

/// Parse the store at `path`. Any failure — absent, unreadable, not JSON, JSON
/// of the wrong shape — is an empty store. See [`registry_ids_for`] for why
/// that is not a silent downgrade.
fn read_store_at(path: &Path) -> Store {
    let mut store = read_base_store(path);
    apply_mutations(path, &mut store);
    store
}

/// The base file alone, with none of the journal replayed over it.
///
/// Split out for [`compact_mutations_at`], which has to re-derive the base from
/// a chosen SUBSET of the journal and therefore cannot use [`read_store_at`] —
/// that would fold everything back in and defeat the point.
fn read_base_store(path: &Path) -> Store {
    match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<Store>(&raw) {
            Ok(store) => store,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "extension provenance store is unreadable; falling back to the config-name join"
                );
                Store::default()
            }
        },
        Err(_) => Store::default(),
    }
}

/// Every parseable mutation file, paired with the file it came from, in the
/// order the reader folds them (sorted by path).
///
/// A file that will not parse is skipped rather than reported — the reader has
/// always behaved that way, and [`compact_mutations_at`] deliberately inherits
/// it: a file whose contents cannot be read is a file whose contribution cannot
/// be folded, so it must never be deleted either.
fn read_mutations(path: &Path) -> Vec<(PathBuf, ProvenanceMutation)> {
    let Ok(entries) = std::fs::read_dir(mutations_dir(path)) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let mutation = std::fs::read(&path)
                .ok()
                .and_then(|body| serde_json::from_slice::<ProvenanceMutation>(&body).ok())?;
            Some((path, mutation))
        })
        .collect()
}

fn apply_mutations(path: &Path, store: &mut Store) {
    let mutations = read_mutations(path)
        .into_iter()
        .map(|(_, mutation)| mutation)
        .collect::<Vec<_>>();
    apply_mutation_list(path, store, mutations);
}

/// Fold `mutations` into `store`, resolving live records through the `current/`
/// pointers on disk.
///
/// ⚠ **The one folding implementation.** [`compact_mutations_at`] calls this
/// with a subset rather than reimplementing it, because the pointer indirection
/// is what decides which of several upserts for one key is live — a compactor
/// that folded upserts in file order would resurrect a superseded install.
fn apply_mutation_list(path: &Path, store: &mut Store, mutations: Vec<ProvenanceMutation>) {
    let tombstones = mutations
        .iter()
        .filter_map(|mutation| match mutation {
            ProvenanceMutation::DeleteIfMatches { expected } => {
                Some(Tombstone::Install(expected.clone()))
            }
            ProvenanceMutation::DeleteRecordIfMatches { key, record } => Some(Tombstone::Record {
                key: key.clone(),
                record: record.clone(),
            }),
            ProvenanceMutation::Upsert { .. } => None,
        })
        .collect::<Vec<_>>();
    store
        .extensions
        .retain(|key, record| !tombstones.iter().any(|dead| dead.covers(key, record)));
    let records = mutations
        .into_iter()
        .filter_map(|mutation| match mutation {
            ProvenanceMutation::Upsert { key, record } => record
                .install_id
                .clone()
                .map(|install_id| (install_id, (key, record))),
            ProvenanceMutation::DeleteIfMatches { .. }
            | ProvenanceMutation::DeleteRecordIfMatches { .. } => None,
        })
        .collect::<HashMap<_, _>>();
    let Ok(pointers) = std::fs::read_dir(current_pointers_dir(path)) else {
        store.version = SCHEMA_VERSION;
        return;
    };
    for pointer in pointers.filter_map(Result::ok).filter_map(|entry| {
        std::fs::read(entry.path())
            .ok()
            .and_then(|body| serde_json::from_slice::<CurrentPointer>(&body).ok())
    }) {
        let Some((key, record)) = records.get(&pointer.install_id) else {
            continue;
        };
        if key != &pointer.key {
            continue;
        }
        let deleted = tombstones.iter().any(|dead| dead.covers(key, record));
        if !deleted {
            store.extensions.insert(key.clone(), record.clone());
        } else {
            store.extensions.remove(key);
        }
    }
    store.version = SCHEMA_VERSION;
}

/// Stat-keyed cache over [`read_store_at`].
///
/// The resolver runs on hot paths — Gate E iterates every installed extension
/// on every turn, Gate C runs per dispatch — so the file may not be parsed each
/// time. It is `stat`ed each time instead, and re-parsed only when the path,
/// mtime or length changes; the `stat` is also what makes an install visible to
/// an already-running daemon without a restart.
type MutationStamp = (usize, Option<SystemTime>, u64);
type StatKey = (PathBuf, Option<(SystemTime, u64)>, MutationStamp);
type CachedStore = Mutex<Option<(StatKey, Arc<Store>)>>;

fn cache() -> &'static CachedStore {
    static CACHE: OnceLock<CachedStore> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// The cache lock, taken so that a poisoned mutex cannot take the daemon with
/// it.
///
/// Nothing inside the critical section can panic today — it is a comparison, a
/// clone and an assignment — so poisoning should be unreachable. But this lock
/// is on the path of every privacy gate, and `unwrap()` here turns a panic
/// somewhere else in the process into a panic in the security resolver on the
/// next tool call. The cached value is a plain snapshot with no invariant a
/// half-finished write could break, so recovering the inner value is always
/// sound; the worst case is one stale read, which the stat key corrects on the
/// call after.
fn lock_cache() -> std::sync::MutexGuard<'static, Option<(StatKey, Arc<Store>)>> {
    cache().lock().unwrap_or_else(|e| e.into_inner())
}

fn invalidate_cache() {
    *lock_cache() = None;
}

fn cached_store_at(path: &Path) -> Arc<Store> {
    // ⚠ A blocking `stat` (and, when the file has changed, a blocking read and
    // parse) on an async gate path — `allowed_extension_keys` calls this once
    // per installed extension that is not already private by name, while it
    // holds the extension map's tokio mutex. That is deliberate and measured
    // rather than overlooked: it is one syscall against a file in the config
    // dir, no `await` is held across the std mutex below, and the alternative —
    // reading the store once per process — is what makes a freshly installed
    // private extension classify Public until the daemon restarts. Freshness is
    // the direction that costs disclosure; latency is not.
    let stamp = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok().map(|t| (t, m.len())));
    let key: StatKey = (path.to_path_buf(), stamp, mutation_stamp(path));
    let mut guard = lock_cache();
    if let Some((cached_key, store)) = guard.as_ref() {
        if *cached_key == key {
            return store.clone();
        }
    }
    let store = Arc::new(read_store_at(path));
    *guard = Some((key, store.clone()));
    store
}

fn mutation_stamp(path: &Path) -> MutationStamp {
    let Ok(entries) = std::fs::read_dir(mutations_dir(path)) else {
        return (0, None, 0);
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .fold((0, None, 0), |(count, latest, bytes), metadata| {
            let modified = metadata.modified().ok();
            (
                count + 1,
                match (latest, modified) {
                    (Some(left), Some(right)) => Some(left.max(right)),
                    (left, None) => left,
                    (None, right) => right,
                },
                bytes + metadata.len(),
            )
        })
}

/// Additive test provenance, consulted by [`registry_ids_for`] ahead of the
/// file.
///
/// ⚠ Process-global and never cleared, on purpose. Tests in this crate run in
/// parallel in one process, so a store a test could *clear* would be a race; one
/// it can only add to is not.
///
/// ⚠ **What is required of a caller, stated as the rule it actually is.** A
/// record is found by its key OR by its install directory, so:
///
///  * The **install directory** must be unique to the test that writes it.
///    Sharing one lends a registry id to another test's fixture, and because the
///    union only raises, the borrowing test goes green on provenance it did not
///    state. This is the one that bites.
///  * The **key** need not be unique, only consistent: two records under one key
///    overwrite, so two tests that both want `cdwagent` under `cdwagent` agree
///    by construction. Where they would disagree, use distinct names.
///
/// An earlier version of this comment claimed both were unique of every caller,
/// which was false: `privacy::extensions` and `agents::extension_manager` both
/// recorded `…/extensions/CDWAgent`. They agreed on `cdwagent`, so nothing was
/// wrong — but deleting either test would have left the other silently propped
/// up by it, and a comment asserting a property the tests do not hold is worse
/// than no comment. The resolver's fixture now uses a directory of its own, so
/// the rule above is true rather than aspirational.
#[cfg(test)]
fn test_records() -> &'static Mutex<HashMap<String, ExtensionProvenance>> {
    static RECORDS: OnceLock<Mutex<HashMap<String, ExtensionProvenance>>> = OnceLock::new();
    RECORDS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Pretend the install of `config_name` recorded `registry_id`. See
/// [`test_records`] for why this only ever adds.
#[cfg(test)]
pub(crate) fn insert_test_record(config_name: &str, registry_id: &str) {
    insert_test_record_at(config_name, registry_id, None);
}

/// As [`insert_test_record`], plus the install directory the bundle was
/// unpacked into — the field a later rename cannot touch.
#[cfg(test)]
pub(crate) fn insert_test_record_at(
    config_name: &str,
    registry_id: &str,
    install_dir: Option<&str>,
) {
    test_records().lock().unwrap().insert(
        name_to_key(config_name),
        ExtensionProvenance {
            install_id: None,
            registry_id: registry_id.to_string(),
            install_dir: install_dir.map(str::to_string),
            source_url: None,
            bundle_sha256: None,
            recorded_at: None,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join(PROVENANCE_FILE);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn a_record_round_trips_through_the_documented_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            r#"{
              "version": 1,
              "extensions": {
                "mystuff": {
                  "registry_id": "cdwagent",
                  "install_dir": "/home/r/.config/biorouter/extensions/CDWAgent",
                  "source_url": "https://example.invalid/cdwagent.brxt",
                  "bundle_sha256": "abc123",
                  "recorded_at": "2026-08-03T19:00:00Z"
                }
              }
            }"#,
        );
        let store = read_store_at(&path);
        assert_eq!(store.version, 1);
        let record = store.extensions.get("mystuff").expect("the record");
        assert_eq!(record.registry_id, "cdwagent");
        assert_eq!(
            record.install_dir.as_deref(),
            Some("/home/r/.config/biorouter/extensions/CDWAgent")
        );
        assert_eq!(
            record.source_url.as_deref(),
            Some("https://example.invalid/cdwagent.brxt")
        );
        assert_eq!(record.bundle_sha256.as_deref(), Some("abc123"));
    }

    #[test]
    fn marketplace_deletion_evidence_requires_an_exact_complete_record() {
        let mut store = Store::default();
        store.extensions.insert(
            "complete".to_owned(),
            ExtensionProvenance {
                install_id: None,
                registry_id: "fixture-agent".to_owned(),
                install_dir: Some("/tmp/extensions/FixtureAgent".to_owned()),
                source_url: Some("https://github.com/example/fixture-agent.brxt".to_owned()),
                bundle_sha256: None,
                recorded_at: None,
            },
        );
        store.extensions.insert(
            "missing-url".to_owned(),
            ExtensionProvenance {
                install_id: None,
                registry_id: "fixture-agent".to_owned(),
                install_dir: Some("/tmp/extensions/Other".to_owned()),
                source_url: None,
                bundle_sha256: None,
                recorded_at: None,
            },
        );

        assert_eq!(
            marketplace_installs_in_store(&store, "fixture-agent"),
            vec![MarketplaceInstallProvenance {
                config_key: "complete".to_owned(),
                install_id: None,
                registry_id: "fixture-agent".to_owned(),
                install_dir: "/tmp/extensions/FixtureAgent".to_owned(),
                source_url: "https://github.com/example/fixture-agent.brxt".to_owned(),
            }]
        );
        assert!(marketplace_installs_in_store(&store, "FIXTURE-AGENT").is_empty());
    }

    #[test]
    fn stale_marketplace_deletion_evidence_cannot_remove_a_replacement_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROVENANCE_FILE);
        let expected = MarketplaceInstallProvenance {
            config_key: "fixture".to_owned(),
            install_id: None,
            registry_id: "fixture-agent".to_owned(),
            install_dir: "/tmp/extensions/FixtureAgent".to_owned(),
            source_url: "https://github.com/example/v1/fixture-agent.brxt".to_owned(),
        };
        record_at(
            &path,
            "fixture",
            ExtensionProvenance {
                install_id: None,
                registry_id: expected.registry_id.clone(),
                install_dir: Some(expected.install_dir.clone()),
                source_url: Some("https://github.com/example/v2/fixture-agent.brxt".to_owned()),
                bundle_sha256: None,
                recorded_at: None,
            },
        )
        .unwrap();

        assert!(!remove_marketplace_install_provenance_at(&path, &expected).unwrap());
        assert!(read_store_at(&path).extensions.contains_key("fixture"));

        let current = marketplace_installs_in_store(&read_store_at(&path), "fixture-agent")
            .pop()
            .unwrap();
        assert!(remove_marketplace_install_provenance_at(&path, &current).unwrap());
        assert!(read_store_at(&path).extensions.is_empty());
    }

    #[test]
    fn concurrent_reinstall_cannot_be_erased_by_a_stale_deletion_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROVENANCE_FILE);
        let expected = MarketplaceInstallProvenance {
            config_key: "fixture".to_owned(),
            install_id: Some("old-install".to_owned()),
            registry_id: "fixture-agent".to_owned(),
            install_dir: "/tmp/extensions/FixtureAgent".to_owned(),
            source_url: "https://github.com/example/v1/fixture-agent.brxt".to_owned(),
        };
        record_at(
            &path,
            "fixture",
            ExtensionProvenance {
                install_id: expected.install_id.clone(),
                registry_id: expected.registry_id.clone(),
                install_dir: Some(expected.install_dir.clone()),
                source_url: Some(expected.source_url.clone()),
                bundle_sha256: None,
                recorded_at: None,
            },
        )
        .unwrap();

        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let removal_path = path.clone();
        let removal_expected = expected.clone();
        let remover = std::thread::spawn(move || {
            remove_marketplace_install_provenance_at_with_hook(
                &removal_path,
                &removal_expected,
                || {
                    read_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
            )
            .unwrap()
        });
        read_rx.recv().unwrap();

        let replacement_source = "https://github.com/example/v2/fixture-agent.brxt";
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || {
            record_at(
                &writer_path,
                "fixture",
                ExtensionProvenance {
                    install_id: Some("replacement-install".to_owned()),
                    registry_id: "fixture-agent".to_owned(),
                    install_dir: Some("/tmp/extensions/FixtureAgent-v2".to_owned()),
                    source_url: Some(replacement_source.to_owned()),
                    bundle_sha256: Some("replacement-digest".to_owned()),
                    recorded_at: None,
                },
            )
        });
        writer.join().unwrap().unwrap();

        release_tx.send(()).unwrap();
        assert!(remover.join().unwrap());

        let store = read_store_at(&path);
        let replacement = store.extensions.get("fixture").unwrap();
        assert_eq!(
            replacement.install_id.as_deref(),
            Some("replacement-install")
        );
        assert_eq!(replacement.source_url.as_deref(), Some(replacement_source));
        assert_eq!(
            replacement.install_dir.as_deref(),
            Some("/tmp/extensions/FixtureAgent-v2")
        );
    }

    fn dir_record(install_dir: Option<&str>) -> ExtensionProvenance {
        ExtensionProvenance {
            install_id: None,
            registry_id: "cdwagent".to_string(),
            install_dir: install_dir.map(str::to_string),
            source_url: None,
            bundle_sha256: None,
            recorded_at: None,
        }
    }

    /// An empty or absent `install_dir` matches nothing.
    ///
    /// A record written by a build that did not have the field, or by a writer
    /// that could not determine it, must not become a wildcard that lends its
    /// registry id to every config with no arguments — which is what a
    /// `Some("") == Some("")` comparison would have produced.
    #[test]
    fn an_empty_install_dir_matches_nothing() {
        for dir in [None, Some("")] {
            let record = dir_record(dir);
            assert!(!matches_install_dir(&record, &[]));
            assert!(!matches_install_dir(&record, &[String::new()]));
            assert!(!matches_install_dir(&record, &["run".to_string()]));
        }
    }

    /// **A record cannot claim a bare word and match every extension at once.**
    ///
    /// The list it is compared against is a whole command line, not a list of
    /// paths: every `.brxt` config is `uv run --directory <dir> <entry>`, so
    /// `run`, `--directory` and `server.py` are all in it. A forged record
    /// claiming `install_dir: "run"` would therefore have matched EVERY
    /// marketplace-installed extension and lent them all its registry id.
    ///
    /// That is denial rather than disclosure — the union only ever raises, so
    /// the result is refusals, not leaks — but one forged line refusing every
    /// extension to every public model is worth a predicate. A real recorded
    /// directory still matches, in both platforms' spellings, because losing a
    /// legitimate record IS a downgrade.
    #[test]
    fn a_recorded_install_dir_that_is_not_a_path_matches_nothing() {
        let uv_args = [
            "run".to_string(),
            "--directory".to_string(),
            "/home/r/.config/biorouter/extensions/CDWAgent".to_string(),
            "server.py".to_string(),
        ];
        for bare in ["run", "--directory", "server.py", "uv", "."] {
            assert!(
                !matches_install_dir(&dir_record(Some(bare)), &uv_args),
                "`{bare}` was accepted as an install directory, so one forged record marks \
                 every uv-launched extension private"
            );
        }
        for real in [
            "/home/r/.config/biorouter/extensions/CDWAgent",
            "C:\\Users\\r\\.config\\biorouter\\extensions\\CDWAgent",
        ] {
            assert!(
                matches_install_dir(&dir_record(Some(real)), &[real.to_string()]),
                "a real recorded directory stopped matching, which loses the record and \
                 downgrades the entry: {real}"
            );
        }
    }

    /// Only `registry_id` is required. The desktop writes the other three; a
    /// future writer that cannot compute a hash must still produce a record
    /// that resolves, because a record that fails to parse is a downgrade.
    #[test]
    fn the_evidence_fields_are_optional() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            r#"{"version":1,"extensions":{"mystuff":{"registry_id":"cdwagent"}}}"#,
        );
        assert_eq!(
            read_store_at(&path).extensions["mystuff"].registry_id,
            "cdwagent"
        );
    }

    /// A file from a newer build keeps the fields this build understands. The
    /// alternative — refusing to read an unknown `version` — drops provenance,
    /// and dropping provenance lowers a tier.
    #[test]
    fn a_newer_schema_version_is_read_rather_than_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            r#"{"version":99,"extensions":{"mystuff":{"registry_id":"cdwagent","future":1}},"whatever":true}"#,
        );
        assert_eq!(
            read_store_at(&path).extensions["mystuff"].registry_id,
            "cdwagent"
        );
    }

    #[test]
    fn an_absent_or_malformed_store_is_empty_rather_than_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_store_at(&dir.path().join(PROVENANCE_FILE))
            .extensions
            .is_empty());
        let path = write(dir.path(), "{ this is not json");
        assert!(read_store_at(&path).extensions.is_empty());
        let path = write(dir.path(), r#"{"version":1,"extensions":[]}"#);
        assert!(read_store_at(&path).extensions.is_empty());
    }

    /// The cache is keyed on the file's stat, so an install performed while the
    /// daemon is running is visible on the next lookup rather than at the next
    /// restart. Without this the store would be read once per process and a
    /// freshly installed private extension would classify public until relaunch.
    #[test]
    fn a_rewritten_store_is_re_read_rather_than_served_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            r#"{"version":1,"extensions":{"mystuff":{"registry_id":"playwrightagent"}}}"#,
        );
        assert_eq!(
            cached_store_at(&path).extensions["mystuff"].registry_id,
            "playwrightagent"
        );
        // Same path, different contents. `len` differs here, and mtime differs
        // on any filesystem with sub-second timestamps; the key carries both.
        write(
            dir.path(),
            r#"{"version":1,"extensions":{"mystuff":{"registry_id":"cdwagent","source_url":"x"}}}"#,
        );
        assert_eq!(
            cached_store_at(&path).extensions["mystuff"].registry_id,
            "cdwagent"
        );
    }

    #[test]
    fn the_store_lives_in_the_config_dir_under_the_documented_name() {
        let path = provenance_path();
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some(PROVENANCE_FILE)
        );
        assert_eq!(path.parent(), Some(Paths::config_dir().as_path()));
    }

    /// The writer and the reader agree, and a second install does not orphan
    /// the first record. The same two properties are asserted on the
    /// TypeScript writer (`extensionProvenance.test.ts`), because the desktop
    /// is what actually writes this file today and the two implementations have
    /// to produce one format.
    #[test]
    fn the_writer_round_trips_and_preserves_what_an_earlier_install_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROVENANCE_FILE);

        record_at(
            &path,
            "CDWAgent",
            ExtensionProvenance {
                install_id: None,
                registry_id: "cdwagent".to_string(),
                install_dir: Some("/home/r/.config/biorouter/extensions/CDWAgent".to_string()),
                source_url: Some("https://example.invalid/cdwagent.brxt".to_string()),
                bundle_sha256: None,
                recorded_at: Some("2026-08-03T19:00:00Z".to_string()),
            },
        )
        .unwrap();
        record_at(
            &path,
            "My Renamed Connector",
            ExtensionProvenance {
                install_id: None,
                registry_id: "ucsfomopagent".to_string(),
                install_dir: None,
                source_url: None,
                bundle_sha256: None,
                recorded_at: None,
            },
        )
        .unwrap();

        let store = read_store_at(&path);
        assert_eq!(store.version, SCHEMA_VERSION);
        // Keyed on the reduced CONFIG name, which is what diverges from the
        // registry id and is the entire reason this file exists.
        assert_eq!(store.extensions["cdwagent"].registry_id, "cdwagent");
        assert_eq!(
            store.extensions["myrenamedconnector"].registry_id,
            "ucsfomopagent"
        );
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file left behind"
        );
    }

    /// The lookup key is the extension manager's map key, not the raw config
    /// name, or a record written for `CDWAgent` would never be found for the
    /// entry the manager stores under `cdwagent`.
    #[test]
    fn a_record_is_found_under_the_reduced_key() {
        insert_test_record("Provenance Key Fixture", "cdwagent");
        assert_eq!(
            registry_ids_for(&[name_to_key("  provenancekeyfixture ")], &[]),
            vec!["cdwagent".to_string()]
        );
    }

    /// **Every match, not the first.** An entry named after a public extension
    /// whose arguments point at a private one's install directory must yield
    /// both ids, or the caller's union silently becomes first-match-wins and
    /// the losing order is the one that lets a private connector through.
    #[test]
    fn a_key_match_does_not_mask_an_install_directory_match() {
        let dir = "/home/researcher/.config/biorouter/extensions/CollisionFixture";
        insert_test_record("collision-fixture-public", "playwrightagent");
        insert_test_record_at("collision-fixture-private", "cdwagent", Some(dir));

        let ids = registry_ids_for(
            &[name_to_key("collision-fixture-public")],
            &[dir.to_string()],
        );
        assert!(ids.contains(&"playwrightagent".to_string()), "{ids:?}");
        assert!(ids.contains(&"cdwagent".to_string()), "{ids:?}");
    }

    /// The `*.json` mutation files **directly in** `mutations_dir(path)`, sorted.
    ///
    /// Both halves of that sentence are load-bearing. `current_pointers_dir` is
    /// `mutations_dir(path).join("current")`, so a recursive walk would count
    /// pointer files as journal entries — and it would happen to give the right
    /// answer today only because pointer filenames are bare hex with no
    /// extension. That is an accident of `pointer_filename`, not a guarantee, so
    /// the one directory is counted explicitly.
    fn mutation_files(path: &Path) -> Vec<PathBuf> {
        let mut files = std::fs::read_dir(mutations_dir(path))
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|file| file.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        files.sort();
        files
    }

    /// Move a file's mtime an hour into the past. Waiting out a sixty-second
    /// grace window is not a test; moving the clock on the files is.
    fn backdate(file: &Path) {
        let when = SystemTime::now() - Duration::from_secs(3600);
        let handle = std::fs::OpenOptions::new().write(true).open(file).unwrap();
        handle
            .set_times(
                std::fs::FileTimes::new()
                    .set_accessed(when)
                    .set_modified(when),
            )
            .unwrap();
    }

    /// Age every mutation whose contents satisfy `wanted`, leaving the rest
    /// inside the grace window.
    fn age_mutations(path: &Path, wanted: impl Fn(&ProvenanceMutation) -> bool) {
        for (file, mutation) in read_mutations(path) {
            if wanted(&mutation) {
                backdate(&file);
            }
        }
    }

    /// A record complete enough for `marketplace_installs_in_store` to return
    /// it, with an `install_dir`/`source_url` pair unique to `index` so a
    /// deletion cannot be ambiguous about which install it names.
    fn marketplace_record(index: usize) -> ExtensionProvenance {
        ExtensionProvenance {
            install_id: None,
            registry_id: "fixture-agent".to_owned(),
            install_dir: Some(format!("/tmp/extensions/FixtureAgent-{index}")),
            source_url: Some(format!(
                "https://example.invalid/v{index}/fixture-agent.brxt"
            )),
            bundle_sha256: None,
            recorded_at: None,
        }
    }

    /// **Catches a compactor that shrinks by DROPPING records** — the real
    /// failure mode here, which is not "fails to shrink". Two plausible wrong
    /// implementations empty the directory and pass any file-count assertion:
    /// one that writes the new base from the folded mutations alone rather than
    /// from base ∪ folded (losing everything an earlier pass already folded),
    /// and one that folds upserts in file order rather than through the
    /// `current/` pointer (resurrecting a superseded install over the live one).
    /// Comparing the whole store either side of the fold is what sees both,
    /// which is why `Store` derives `PartialEq`.
    #[test]
    fn compaction_is_lossless() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROVENANCE_FILE);

        record_at(&path, "alpha", marketplace_record(1)).unwrap();
        record_at(&path, "beta", marketplace_record(2)).unwrap();
        // A reinstall of `alpha`. The superseded upsert stays on disk forever,
        // so a pointer-blind compactor has something to get wrong.
        record_at(&path, "alpha", marketplace_record(3)).unwrap();
        record_at(&path, "gamma", marketplace_record(4)).unwrap();

        let victim = marketplace_installs_in_store(&read_store_at(&path), "fixture-agent")
            .into_iter()
            .find(|install| install.config_key == "gamma")
            .expect("the gamma install");
        assert!(remove_marketplace_install_provenance_at(&path, &victim).unwrap());

        let before = read_store_at(&path);
        assert_eq!(
            before.extensions.len(),
            2,
            "the fixture did not build the store it claims, so equality after would prove nothing"
        );
        assert_eq!(
            before.extensions["alpha"].install_dir.as_deref(),
            Some("/tmp/extensions/FixtureAgent-3"),
            "the reinstall did not supersede the first install"
        );

        age_mutations(&path, |_| true);
        compact_mutations_at(&path).unwrap();

        assert_eq!(read_store_at(&path), before);
    }

    /// **Catches folding an eligible tombstone away from an ineligible
    /// `Upsert`.** The deletion is applied to a base that does not hold the
    /// record yet, the tombstone file is then deleted, and the next read
    /// re-inserts the record through its `current/` pointer — the marketplace
    /// package the user removed comes back, and with it the provenance that
    /// classifies it.
    ///
    /// `compaction_is_lossless` cannot see this. It compares a single instant,
    /// and at that instant the pointer and the upsert agree with each other.
    #[test]
    fn compaction_does_not_resurrect_a_deleted_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROVENANCE_FILE);

        record_at(&path, "fixture", marketplace_record(1)).unwrap();
        let install = marketplace_installs_in_store(&read_store_at(&path), "fixture-agent")
            .pop()
            .expect("the install");
        assert!(remove_marketplace_install_provenance_at(&path, &install).unwrap());
        assert!(!read_store_at(&path).extensions.contains_key("fixture"));

        // The tombstone has settled. The `Upsert` and pointer it suppresses have
        // not, so the tombstone must be held back with them.
        age_mutations(&path, |mutation| {
            matches!(mutation, ProvenanceMutation::DeleteIfMatches { .. })
        });
        compact_mutations_at(&path).unwrap();

        assert!(
            !read_store_at(&path).extensions.contains_key("fixture"),
            "a deleted marketplace package came back after compaction"
        );
        assert!(
            read_mutations(&path).iter().any(|(_, mutation)| matches!(
                mutation,
                ProvenanceMutation::DeleteIfMatches { .. }
            )),
            "the tombstone was folded away from the fresh Upsert it has to outlive"
        );
    }

    /// The same resurrection, through the OTHER tombstone shape. The pointer
    /// sweep asks `Tombstone::covers`, which knows both, so a
    /// `DeleteRecordIfMatches` left out of the pairing rule is folded away from
    /// the `Upsert` it suppresses and the record comes back on the next read.
    ///
    /// The fixture record deliberately carries neither `install_dir` nor
    /// `source_url`, so `DeleteIfMatches` could not name it at all — that is
    /// why the second shape exists, and it is what `remove_extension` meets.
    #[test]
    fn compaction_does_not_resurrect_a_record_deleted_by_the_whole_record_tombstone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROVENANCE_FILE);

        record_at(
            &path,
            "fixture",
            ExtensionProvenance {
                install_id: None,
                registry_id: "fixture-agent".to_owned(),
                install_dir: None,
                source_url: None,
                bundle_sha256: None,
                recorded_at: None,
            },
        )
        .unwrap();
        let stored = read_store_at(&path).extensions["fixture"].clone();
        assert!(
            marketplace_installs_in_store(&read_store_at(&path), "fixture-agent").is_empty(),
            "the narrower tombstone must be unable to name this record, or the fixture \
             is not the case under test"
        );
        assert!(remove_extension_provenance_if_matches_at(&path, "fixture", &stored).unwrap());
        assert!(!read_store_at(&path).extensions.contains_key("fixture"));

        // The tombstone has settled. The `Upsert` and pointer it suppresses have
        // not, so the tombstone must be held back with them.
        age_mutations(&path, |mutation| {
            matches!(mutation, ProvenanceMutation::DeleteRecordIfMatches { .. })
        });
        compact_mutations_at(&path).unwrap();

        assert!(
            !read_store_at(&path).extensions.contains_key("fixture"),
            "a record deleted through the whole-record tombstone came back after compaction"
        );
        assert!(
            read_mutations(&path).iter().any(|(_, mutation)| matches!(
                mutation,
                ProvenanceMutation::DeleteRecordIfMatches { .. }
            )),
            "the whole-record tombstone was folded away from the fresh Upsert it has to outlive"
        );
    }

    /// **Catches a compactor that snapshots between the TypeScript writer's two
    /// writes.** `ui/desktop/src/utils/extensionProvenance.ts` renames the
    /// mutation file first and the `current/` pointer second; folding inside
    /// that gap computes a base missing the record and then deletes the only
    /// copy of it, leaving the pointer that lands a moment later aimed at
    /// nothing — a silent tier downgrade. A file younger than the grace window
    /// must still be on disk after a pass.
    #[test]
    fn compaction_respects_the_grace_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROVENANCE_FILE);

        record_at(&path, "fixture", marketplace_record(1)).unwrap();
        let before = mutation_files(&path);
        assert_eq!(before.len(), 1, "one install writes one mutation file");

        compact_mutations_at(&path).unwrap();

        assert_eq!(
            mutation_files(&path),
            before,
            "a mutation younger than the grace window was folded"
        );
        assert_eq!(
            read_store_at(&path).extensions["fixture"].registry_id,
            "fixture-agent"
        );
    }

    /// **Catches the unbounded growth the compactor exists for.** Deliberately
    /// paired with `compaction_is_lossless`: a file count on its own is passed
    /// by a compactor that simply empties the directory, so the two assertions
    /// sit together — the count says the journal shrank, the equality says it
    /// shrank by folding rather than by discarding.
    #[test]
    fn compaction_bounds_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROVENANCE_FILE);

        for index in 0..50 {
            record_at(
                &path,
                &format!("fixture-{}", index % 5),
                marketplace_record(index),
            )
            .unwrap();
        }
        let victim = marketplace_installs_in_store(&read_store_at(&path), "fixture-agent")
            .pop()
            .expect("an install to delete");
        assert!(remove_marketplace_install_provenance_at(&path, &victim).unwrap());

        let before = mutation_files(&path).len();
        assert_eq!(before, 51, "fifty installs plus one delete, one file each");
        let store_before = read_store_at(&path);
        assert_eq!(store_before.extensions.len(), 4, "five keys, one deleted");

        age_mutations(&path, |_| true);
        compact_mutations_at(&path).unwrap();

        let after = mutation_files(&path).len();
        assert!(
            after < before,
            "the journal did not shrink: {before} files before, {after} after"
        );
        assert_eq!(read_store_at(&path), store_before);
    }
}

#[cfg(test)]
mod registry_id_tests {
    use super::registry_id_matches;

    /// SPOKEAgent shipped as `spokeagent-0.4.1` — the version baked into the id
    /// — and every machine that installed it recorded that string. The id is
    /// version-free now, and a bare equality test would orphan those records:
    /// a delete would resolve the new id, find no provenance, and refuse to
    /// remove a package that is plainly installed.
    #[test]
    fn a_legacy_versioned_id_still_names_its_package() {
        assert!(registry_id_matches("spokeagent", "spokeagent"));
        assert!(registry_id_matches("spokeagent-0.4.1", "spokeagent"));
        assert!(registry_id_matches("spokeagent-1.0", "spokeagent"));
    }

    /// Deliberately narrow: this widens what a DELETION accepts as the same
    /// package, so anything that is not a version-shaped tail must not match.
    #[test]
    fn nothing_but_a_version_tail_matches() {
        for recorded in [
            "spokeagent-nightly",
            "spokeagentx",
            "spokeagent-",
            "spokeagent-0.4.1-beta",
            "other-0.4.1",
            "",
        ] {
            assert!(
                !registry_id_matches(recorded, "spokeagent"),
                "`{recorded}` must not be taken for `spokeagent`"
            );
        }
        // And it is not symmetric: a bare id does not answer for a versioned one.
        assert!(!registry_id_matches("spokeagent", "spokeagent-0.4.1"));
    }
}
