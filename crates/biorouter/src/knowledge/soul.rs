//! The built-in **Soul** knowledge base and its self-maintaining machinery.
//!
//! Soul is a personal, initially-empty knowledge base installed automatically
//! the first time a user runs Biorouter. It accumulates durable facts about the
//! user — how they approach scientific questions, which tools and commands they
//! reach for, the shape of their tool calls and the responses they act on, and
//! personal details they reveal (name, occupation, preferences). A built-in
//! **"Meditation"** workflow, an **update-soul** skill, and a daily 3:00 AM
//! scheduled job ("Daily Meditation") keep it growing from the user's
//! conversation history.
//!
//! [`install`] is idempotent and safe to call on every startup: it creates what
//! is missing and upgrades byte-identical shipped assets without overwriting
//! user edits.
//!
//! The skill is named `update-soul` (it was previously `soul-writer`) and is
//! seeded as a member of the knowledge skill bundle
//! ([`crate::agents::skills_extension::KNOWLEDGE_BUNDLE`]), so Settings offers
//! one "Knowledge" switch over it and the four format skills rather than five.
//! [`ensure_soul_skill`] removes both directories it used to live in — the old
//! `soul-writer` name and the old flat placement — so an upgraded user does not
//! end up with two candidates for one skill name.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::agents::skills_extension;
use crate::config::paths::Paths;
use crate::scheduler::ScheduledJob;
use crate::scheduler_trait::SchedulerTrait;
use anyhow::Context as _;
use biorouter_mcp::knowledge::git::GitRepo;
use biorouter_mcp::knowledge::service::{classify_base_format, BaseFormat, KnowledgeService};
use biorouter_mcp::knowledge::types::{
    ChangeKind, KbFormat, RegistryEntry, CURRENT_SCHEMA_VERSION,
};
use biorouter_mcp::knowledge::{manifest, okf, paths, registry, tier};
use fs2::FileExt as _;
use sha2::{Digest as _, Sha256};

/// The shipped base's id.
///
/// ⚠ Defined AS `biorouter-mcp`'s own constant rather than beside it. This
/// module is the seeder — `ensure_soul` creates the base with this id — and
/// `KnowledgeServer::kb_delete_base` refuses `service::DEFAULT_PRIMARY_KB_ID`.
/// Two `"soul"` literals in two crates with nothing pinning them equal meant a
/// rename on either side would leave the built-in base deletable by an agent,
/// silently, with every test still green.
pub const SOUL_KB_ID: &str = biorouter_mcp::knowledge::service::DEFAULT_PRIMARY_KB_ID;
pub const SOUL_KB_NAME: &str = "Soul";
pub const MEDITATION_WORKFLOW_FILE: &str = "meditation.yaml";
/// Job ids double as on-disk filenames in the scheduler, so the id stays a
/// slug; the UI renders it as "Daily Meditation".
pub const MEDITATION_SCHEDULE_ID: &str = "daily-meditation";
pub const SOUL_SKILL_DIR: &str = "update-soul";
/// The skill's previous directory name, removed on startup so upgraded users
/// don't see a stale duplicate alongside the renamed `update-soul` skill.
pub const SOUL_SKILL_DIR_LEGACY: &str = "soul-writer";
/// 6-field cron (sec min hour dom mon dow) — every day at 03:00 local time.
pub const MEDITATION_CRON: &str = "0 0 3 * * *";

/// Warm parchment tone distinct from the default KB colour.
pub const SOUL_COLOR: &str = "#9c6b3f";

const SOUL_RECONCILE_LOCK: &str = ".soul-reconcile.lock";
const PREVIOUS_MEDITATION_WORKFLOW_SHA256: &[&str] = &[
    "142edbf98aca3e521649ddec05ed156ab16936303cc42cbe8cce1bc79c630772",
    "a0558805057ffae5257fa805b73253a1b615300bec563a0126f4a23884f9884c",
];
const SOUL_OKF_SCHEMA: &str = include_str!("../../../biorouter-mcp/src/knowledge/schema_okf.md");
const SOUL_LOG: &str = "# Log\n\n";
const SOUL_GITIGNORE: &str =
    "raw/*/original.*\n.biorouter-knowledge/.crossref-cache/\n.biorouter-knowledge/write.lock\n";

struct SoulFileLock(File);

impl SoulFileLock {
    fn acquire(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.lock_exclusive()?;
        Ok(Self(file))
    }

    fn acquire_existing(path: &Path) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.lock_exclusive()?;
        Ok(Self(file))
    }
}

impl Drop for SoulFileLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

/// Install every Soul component that is missing. Best-effort: a failure in one
/// component is logged and does not abort the others or block startup.
pub async fn install(scheduler: &Arc<dyn SchedulerTrait>) {
    if let Err(e) = ensure_soul_kb() {
        tracing::warn!("Soul: failed to create knowledge base: {e}");
    }
    if let Err(e) = ensure_soul_skill() {
        tracing::warn!("Soul: failed to install skill: {e}");
    }
    match ensure_meditation_workflow() {
        Ok(path) => {
            if let Err(e) = ensure_meditation_schedule(scheduler, path).await {
                tracing::warn!("Soul: failed to register Daily Meditation schedule: {e}");
            }
        }
        Err(e) => tracing::warn!("Soul: failed to install Meditation workflow: {e}"),
    }
}

/// Install the assets that don't need a running scheduler (KB, skill, workflow
/// file). Used on surfaces that have no scheduler handy (e.g. CLI-only flows).
pub fn install_assets() {
    let _ = ensure_soul_kb_without_purge();
    let _ = ensure_soul_skill();
    let _ = ensure_meditation_workflow();
}

fn ensure_soul_kb_without_purge() -> anyhow::Result<()> {
    let svc = KnowledgeService::new_default()?;
    let _reconcile_lock = SoulFileLock::acquire(&svc.root().join(SOUL_RECONCILE_LOCK))?;
    ensure_registered_native_soul(&svc)
}

/// Remove every pre-OKF knowledge base and ensure the built-in Soul is current
/// plain OKF. Existing OKF and BioOKF bases are preserved.
pub fn ensure_soul_kb() -> anyhow::Result<()> {
    let svc = KnowledgeService::new_default()?;
    for id in reconcile_soul_kb(&svc)?.removed {
        tracing::info!("Soul: removed legacy knowledge base '{id}'");
    }
    Ok(())
}

/// What one reconciliation pass did, and what it refused to do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReconcileOutcome {
    /// Bases positively identified as pre-OKF **under the locks that authorized
    /// it** and removed by this pass. The startup log names these, so nothing
    /// may land here that this pass did not verify and destroy.
    pub removed: Vec<String>,
    /// Third-party bases this pass could not diagnose, or could not finish
    /// retiring. Nothing is moved: the directory is left exactly where it was
    /// found and excluded from the purge, and each entry has already been
    /// logged as a warning.
    pub quarantined: Vec<String>,
}

impl ReconcileOutcome {
    fn quarantine(&mut self, subject: &str, reason: &str) {
        tracing::warn!("Soul: leaving '{subject}' untouched: {reason}");
        self.quarantined.push(subject.to_string());
    }
}

/// The instants a test can interpose on.
///
/// The interesting one is between deciding a base is legacy and taking the
/// locks that let it be destroyed: that window is unbounded (`flock` has no
/// deadline) and it is where a concurrent writer replaces the base.
///
/// Production passes a closure that reads nothing, so the payloads are live
/// only under `cfg(test)`.
#[cfg_attr(not(test), allow(dead_code))]
enum ReconcileCheckpoint<'a> {
    ScannedUnregisteredEntry(&'a str),
    ClassifiedRegisteredLegacy(&'a str),
    ClassifiedUnregisteredLegacy(&'a str),
}

/// The storage migration used by startup and by isolated upgrade tests.
///
/// **Only the built-in Soul is worth failing over.** This runs on every
/// `AgentManager::new` and every `biorouter knowledge` subcommand, so an `Err`
/// raised for somebody else's damaged base is a daemon that never constructs
/// `AppState` — no HTTP listener, no window, no degraded mode, and no way for
/// the user to reach the store and repair the base the message names. A base
/// this pass cannot diagnose is therefore reported and left alone
/// ([`ReconcileOutcome::quarantined`]), exactly as `list_bases` already handles
/// a manifest it cannot read.
pub fn reconcile_soul_kb(svc: &KnowledgeService) -> anyhow::Result<ReconcileOutcome> {
    reconcile_soul_kb_with_checkpoint(svc, |_| Ok(()))
}

fn reconcile_soul_kb_with_checkpoint(
    svc: &KnowledgeService,
    mut checkpoint: impl FnMut(ReconcileCheckpoint<'_>) -> anyhow::Result<()>,
) -> anyhow::Result<ReconcileOutcome> {
    let _reconcile_lock = SoulFileLock::acquire(&svc.root().join(SOUL_RECONCILE_LOCK))?;
    let mut outcome = ReconcileOutcome::default();

    match svc.resume_pending_delete_cleanup() {
        Ok(ids) => {
            for id in ids {
                tracing::info!("Soul: finished interrupted deletion of knowledge base '{id}'");
            }
        }
        Err(error) => outcome.quarantine("interrupted deletions", &format!("{error:#}")),
    }

    purge_registered_legacy(svc, &mut outcome, &mut checkpoint)?;
    purge_unregistered_legacy(svc, &mut outcome, &mut checkpoint)?;

    ensure_registered_native_soul(svc).with_context(|| {
        format!("could not establish the built-in '{SOUL_KB_ID}' knowledge base")
    })?;
    Ok(outcome)
}

fn purge_registered_legacy(
    svc: &KnowledgeService,
    outcome: &mut ReconcileOutcome,
    checkpoint: &mut impl FnMut(ReconcileCheckpoint<'_>) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    for entry in registry::load(svc.root())? {
        let expected_path = paths::kb_root(svc.root(), &entry.id);
        if entry.path != expected_path {
            outcome.quarantine(
                &entry.id,
                &format!(
                    "registry path {} does not match {}",
                    entry.path.display(),
                    expected_path.display()
                ),
            );
            continue;
        }
        match classify_base_format(&entry.path) {
            BaseFormat::Legacy => {}
            BaseFormat::Current => continue,
            BaseFormat::Undiagnosable(reason) => {
                outcome.quarantine(&entry.id, &reason);
                continue;
            }
        }
        checkpoint(ReconcileCheckpoint::ClassifiedRegisteredLegacy(&entry.id))?;
        match svc.delete_registered_legacy_base(&entry.id) {
            Ok(true) => outcome.removed.push(entry.id),
            // Something moved the base off the legacy format, or off this path,
            // between the classification above and the locks under it. The
            // deletion declined; nothing was touched.
            Ok(false) => {}
            Err(error) => {
                let converged = svc
                    .base_is_current_or_fully_removed(&entry.id)
                    .unwrap_or(false);
                if !converged {
                    outcome.quarantine(&entry.id, &format!("{error:#}"));
                } else if !expected_path.exists() {
                    // Gone, and the store is consistent about it — another
                    // process finished the same retirement. Reported rather
                    // than counted: `removed` is what the startup log claims
                    // this pass verified and destroyed, and this is not that.
                    tracing::info!(
                        "Soul: legacy knowledge base '{}' was already retired: {error:#}",
                        entry.id
                    );
                }
            }
        }
    }
    Ok(())
}

fn purge_unregistered_legacy(
    svc: &KnowledgeService,
    outcome: &mut ReconcileOutcome,
    checkpoint: &mut impl FnMut(ReconcileCheckpoint<'_>) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let registered_ids = registry::load(svc.root())?
        .into_iter()
        .map(|entry| entry.id)
        .collect::<std::collections::HashSet<_>>();
    if !svc.root().exists() {
        return Ok(());
    }
    let listing = match std::fs::read_dir(svc.root()) {
        Ok(listing) => listing,
        Err(error) => {
            outcome.quarantine(
                "unregistered knowledge directories",
                &format!("cannot scan {}: {error}", svc.root().display()),
            );
            return Ok(());
        }
    };
    for entry in listing {
        let Ok(entry) = entry else { continue };
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if registered_ids.contains(&id) || paths::validate_kb_id(&id).is_err() {
            continue;
        }
        checkpoint(ReconcileCheckpoint::ScannedUnregisteredEntry(&id))?;
        // A directory can vanish under a concurrent delete by the other daemon
        // sharing this store. That is a reason to look at the next entry, not
        // to abort the machine's startup.
        let Ok(metadata) = std::fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            continue;
        }
        // An unregistered directory that is not positively a legacy base is
        // none of this pass's business: it may be a current base whose registry
        // row is being written right now, or something no part of Biorouter
        // created and no part of Biorouter may delete.
        if classify_base_format(&entry.path()) != BaseFormat::Legacy {
            continue;
        }
        checkpoint(ReconcileCheckpoint::ClassifiedUnregisteredLegacy(&id))?;
        match svc.delete_unregistered_legacy_base(&id) {
            Ok(true) => outcome.removed.push(id),
            Ok(false) => {}
            Err(error) => outcome.quarantine(&id, &format!("{error:#}")),
        }
    }
    Ok(())
}

fn ensure_registered_native_soul(svc: &KnowledgeService) -> anyhow::Result<()> {
    for _ in 0..4 {
        let soul_root = paths::kb_root(svc.root(), SOUL_KB_ID);
        let entries = registry::load(svc.root())?;
        let registered = entries
            .iter()
            .filter(|entry| entry.id == SOUL_KB_ID)
            .collect::<Vec<_>>();
        if registered.len() > 1 {
            anyhow::bail!("multiple registry entries use the built-in Soul id");
        }
        if let Some(entry) = registered.first() {
            if entry.path != soul_root {
                anyhow::bail!(
                    "registered Soul path {} does not match {}",
                    entry.path.display(),
                    soul_root.display()
                );
            }
            normalize_registered_soul(svc, &soul_root)?;
            return require_registered_native_soul(svc);
        }

        if soul_root.exists() {
            recover_unregistered_soul(svc, &soul_root)?;
            return require_registered_native_soul(svc);
        }

        match svc.create_base_in(SOUL_KB_ID, SOUL_KB_NAME, Some(SOUL_COLOR), KbFormat::Okf) {
            Ok(_) => {
                tracing::info!("Soul: created built-in OKF knowledge base '{SOUL_KB_ID}'");
                return require_registered_native_soul(svc);
            }
            Err(error) => {
                let now_registered = registry::load(svc.root())?
                    .iter()
                    .any(|entry| entry.id == SOUL_KB_ID && entry.path == soul_root);
                if !soul_root.exists() && !now_registered {
                    return Err(error).context("create built-in Soul knowledge base");
                }
            }
        }
    }
    anyhow::bail!("Soul did not converge to one registered native OKF base")
}

fn normalize_registered_soul(svc: &KnowledgeService, soul_root: &Path) -> anyhow::Result<()> {
    require_real_soul_directory(soul_root, "registered")?;
    let _kb_lock = SoulFileLock::acquire_existing(&soul_root.join(paths::KB_WRITE_LOCK_REL))?;
    let _root_lock = SoulFileLock::acquire(&svc.root().join(".knowledge-root.lock"))?;
    let entries = registry::load(svc.root())?;
    let matching = entries
        .iter()
        .filter(|entry| entry.id == SOUL_KB_ID)
        .collect::<Vec<_>>();
    let path_owners = entries
        .iter()
        .filter(|entry| entry.path == soul_root)
        .count();
    if matching.len() != 1 || matching[0].path != soul_root || path_owners != 1 {
        anyhow::bail!("Soul registration changed while it was being normalized");
    }
    normalize_soul_under_locks(soul_root)?;
    tier::register_public_if_absent_unlocked(svc.root(), SOUL_KB_ID)?;
    svc.rebuild_graph_cache(SOUL_KB_ID)
        .context("rebuild Soul graph cache")?;
    Ok(())
}

fn recover_unregistered_soul(svc: &KnowledgeService, soul_root: &Path) -> anyhow::Result<()> {
    require_real_soul_directory(soul_root, "unregistered")?;
    let _kb_lock = SoulFileLock::acquire_existing(&soul_root.join(paths::KB_WRITE_LOCK_REL))?;
    let _root_lock = SoulFileLock::acquire(&svc.root().join(".knowledge-root.lock"))?;
    let entries = registry::load(svc.root())?;
    if entries
        .iter()
        .any(|entry| entry.path == soul_root && entry.id != SOUL_KB_ID)
    {
        anyhow::bail!(
            "cannot recover Soul: another registry id already owns {}",
            soul_root.display()
        );
    }
    let registered = entries
        .iter()
        .filter(|entry| entry.id == SOUL_KB_ID)
        .collect::<Vec<_>>();
    if registered.len() > 1 {
        anyhow::bail!("multiple registry entries use the built-in Soul id");
    }
    if let Some(entry) = registered.first() {
        if entry.path != soul_root {
            anyhow::bail!(
                "registered Soul path {} does not match {}",
                entry.path.display(),
                soul_root.display()
            );
        }
        normalize_soul_under_locks(soul_root)?;
    } else {
        let current = manifest::load(soul_root)
            .context("recover unregistered Soul: read its current-format manifest")?;
        if current.profile().is_none() {
            anyhow::bail!("cannot recover unregistered Soul because it is a legacy pre-OKF base");
        }
        normalize_soul_under_locks(soul_root)?;
        registry::register(
            svc.root(),
            RegistryEntry {
                id: SOUL_KB_ID.to_string(),
                path: soul_root.to_path_buf(),
            },
        )?;
        tier::register_public_if_absent_unlocked(svc.root(), SOUL_KB_ID)?;
        tracing::info!("Soul: recovered interrupted creation of '{SOUL_KB_ID}'");
    }
    svc.rebuild_graph_cache(SOUL_KB_ID)
        .context("rebuild recovered Soul graph cache")?;
    Ok(())
}

fn require_real_soul_directory(soul_root: &Path, state: &str) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(soul_root).with_context(|| {
        format!(
            "{state} Soul directory is missing at {}",
            soul_root.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        anyhow::bail!(
            "{state} Soul path {} is not a real directory",
            soul_root.display()
        );
    }
    Ok(())
}

fn normalize_soul_under_locks(soul_root: &Path) -> anyhow::Result<()> {
    let git_exists = soul_root.join(".git").exists();
    let repo = if git_exists {
        let raw_repo = git2::Repository::open(soul_root)?;
        let unborn = raw_repo.is_empty()?;
        drop(raw_repo);
        let repo = GitRepo::open(soul_root)?;
        if !unborn {
            repo.recover_orphaned_txn()?;
        }
        repo
    } else {
        GitRepo::init(soul_root)?
    };

    let mut current = manifest::load(soul_root)?;
    let format = current
        .profile()
        .ok_or_else(|| anyhow::anyhow!("registered Soul is a legacy pre-OKF base"))?;
    let converting_biookf = format == KbFormat::Biookf;
    let mut changed = !git_exists;
    for path in [
        soul_root.join("knowledge/concept"),
        soul_root.join("knowledge/source"),
        soul_root.join("knowledge/note"),
        soul_root.join("raw"),
        soul_root.join(".biorouter-knowledge"),
    ] {
        let missing = !path.exists();
        std::fs::create_dir_all(&path)?;
        changed |= missing;
    }

    let schema_path = soul_root.join("schema.md");
    if converting_biookf {
        replace_file_atomically(&schema_path, SOUL_OKF_SCHEMA)?;
        changed = true;
    } else {
        changed |= create_built_in_file_if_missing(&schema_path, SOUL_OKF_SCHEMA)?;
    }
    let index = soul_index();
    changed |= create_built_in_file_if_missing(&soul_root.join("index.md"), &index)?;
    changed |= create_built_in_file_if_missing(&soul_root.join("log.md"), SOUL_LOG)?;
    changed |= create_built_in_file_if_missing(&soul_root.join(".gitignore"), SOUL_GITIGNORE)?;

    let manifest_needs_normalizing = current.id != SOUL_KB_ID
        || converting_biookf
        || current.okf_version.as_deref() != Some(okf::OKF_VERSION)
        || current.biookf_version.is_some();
    if manifest_needs_normalizing {
        current.id = SOUL_KB_ID.to_string();
        if converting_biookf {
            current.schema_version = CURRENT_SCHEMA_VERSION;
        }
        current.format = KbFormat::Okf;
        current.okf_version = Some(okf::OKF_VERSION.to_string());
        current.biookf_version = None;
        manifest::save(soul_root, &current)?;
        changed = true;
    }

    // The derived cache is rebuilt below; a refresh is not recovered knowledge.
    let mut status_options = git2::StatusOptions::new();
    status_options
        .include_untracked(true)
        .recurse_untracked_dirs(true);
    let dirty = git2::Repository::open(soul_root)?
        .statuses(Some(&mut status_options))?
        .iter()
        .any(|entry| entry.path() != Some(".biorouter-knowledge/graph-cache.json"));
    if changed || dirty {
        repo.commit_all(
            ChangeKind::Manual,
            if converting_biookf {
                "convert built-in Soul to native OKF"
            } else {
                "finish built-in Soul knowledge base recovery"
            },
            None,
        )?;
    }
    Ok(())
}

fn replace_file_atomically(path: &Path, content: &str) -> anyhow::Result<()> {
    use std::io::Write as _;

    let temp = path.with_extension(format!("md.tmp-{}", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    if let Err(error) = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = std::fs::remove_file(&temp);
        return Err(error.into());
    }
    drop(file);
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

fn soul_index() -> String {
    format!(
        "---\nokf_version: '{}'\n---\n\n# Pages\n\n_No pages yet._\n",
        okf::OKF_VERSION
    )
}

fn require_registered_native_soul(svc: &KnowledgeService) -> anyhow::Result<()> {
    let soul_root = paths::kb_root(svc.root(), SOUL_KB_ID);
    let entries = registry::load(svc.root())?;
    let registered = entries
        .iter()
        .filter(|entry| entry.id == SOUL_KB_ID && entry.path == soul_root)
        .count();
    let path_owners = entries
        .iter()
        .filter(|entry| entry.path == soul_root)
        .count();
    if registered != 1 || path_owners != 1 {
        anyhow::bail!("Soul is not registered exactly once at its canonical path");
    }
    let current = manifest::load(&soul_root)?;
    if current.id != SOUL_KB_ID || current.profile() != Some(KbFormat::Okf) {
        anyhow::bail!("registered Soul is not native plain OKF");
    }
    Ok(())
}

/// Keep the shipped "Meditation" workflow current in the global workflow
/// library. Returns the workflow file path.
pub fn ensure_meditation_workflow() -> anyhow::Result<PathBuf> {
    let dir = Paths::config_dir().join("workflows");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(MEDITATION_WORKFLOW_FILE);
    if create_or_upgrade_built_in_file(
        &path,
        MEDITATION_WORKFLOW_YAML,
        PREVIOUS_MEDITATION_WORKFLOW_SHA256,
    )? {
        tracing::info!(
            "Soul: installed or upgraded Meditation workflow at {}",
            path.display()
        );
    }
    Ok(path)
}

fn create_or_upgrade_built_in_file(
    path: &Path,
    content: &str,
    previous_sha256: &[&str],
) -> anyhow::Result<bool> {
    use std::io::Write as _;

    if create_built_in_file_if_missing(path, content)? {
        return Ok(true);
    }
    let installed = std::fs::read(path)?;
    let installed_sha256 = format!("{:x}", Sha256::digest(&installed));
    if !previous_sha256.contains(&installed_sha256.as_str()) {
        return Ok(false);
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("built-in asset has no parent directory"))?;
    let mut replacement = tempfile::NamedTempFile::new_in(parent)?;
    replacement.write_all(content.as_bytes())?;
    replacement.as_file_mut().sync_all()?;
    replacement
        .persist(path)
        .map_err(|error| anyhow::anyhow!(error.error))?;
    Ok(true)
}

fn create_built_in_file_if_missing(path: &std::path::Path, content: &str) -> anyhow::Result<bool> {
    use std::io::Write as _;

    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    file.write_all(content.as_bytes())?;
    Ok(true)
}

/// Write the update-soul skill if it is not already present.
///
/// ⚠ **Into the knowledge bundle, not flat at the skills root.** The skill is a
/// member of `skills_extension::KNOWLEDGE_BUNDLE` alongside the four
/// `include_str!`-shipped knowledge skills, and the bundle — not this skill —
/// is the Context row Settings offers. Seeding it flat would give it a
/// `bundle_name` of `None`, which is a standalone picker row that the bundle's
/// Context toggle does not reach.
pub fn ensure_soul_skill() -> anyhow::Result<()> {
    if place_soul_skill(&Paths::config_dir().join("skills"))? {
        // Creating `<bundle>/<child>/` bumps the bundle's mtime and not the
        // root's, and mtime is one-second granular — see `skill_catalog`'s
        // header. A writer says what it did rather than hoping to be noticed.
        crate::agents::skill_catalog::invalidate();
    }
    Ok(())
}

/// [`ensure_soul_skill`] against an explicit skills root. Returns whether
/// anything on disk moved, so the caller knows whether to invalidate.
///
/// Split out so a test can drive the migration over a `TempDir` without setting
/// `BIOROUTER_PATH_ROOT`, whose process-global reach would make it depend on
/// whatever ran before it.
fn place_soul_skill(skills_root: &Path) -> anyhow::Result<bool> {
    let bundle = skills_extension::knowledge_bundle_dir(skills_root);

    let dir = bundle.join(SOUL_SKILL_DIR);

    // Leave neither of the two directories this skill has previously occupied
    // behind. Not tidiness: discovery keys by frontmatter `name`, so a flat
    // `update-soul` and a bundled one are two candidates for one map key and
    // whichever `read_dir` yields last wins — half the installs would get the
    // stale one, with no bundle on it and so out of reach of the Context
    // switch.
    //
    // ⚠ The flat copy is **moved**, not deleted, when the bundle has no member
    // yet. This skill is written with `create_built_in_file_if_missing`
    // precisely so a user's edits to it survive every later startup, and
    // deleting the file that holds them on the one startup that relocates it
    // would take back that promise at the worst possible moment. (The four
    // `include_str!` knowledge skills are rewritten whenever their content
    // differs, so their migration can simply delete — each migration matches
    // its own seeder's semantics.)
    let mut migrated = false;
    let legacy_flat = skills_root.join(SOUL_SKILL_DIR);
    if legacy_flat.is_dir() && !dir.exists() {
        std::fs::create_dir_all(&bundle)?;
        match std::fs::rename(&legacy_flat, &dir) {
            Ok(()) => {
                tracing::info!(
                    "Soul: moved skill into the knowledge bundle at {}",
                    dir.display()
                );
                migrated = true;
            }
            Err(e) => tracing::warn!("Soul: failed to move skill into {}: {e}", dir.display()),
        }
    }
    // Whatever the move did not claim — the pre-rename `soul-writer` folder,
    // and a flat copy the bundle already had a member for — is a duplicate.
    for stale in [
        skills_root.join(SOUL_SKILL_DIR_LEGACY),
        skills_root.join(SOUL_SKILL_DIR),
    ] {
        if !stale.is_dir() {
            continue;
        }
        match std::fs::remove_dir_all(&stale) {
            Ok(()) => {
                tracing::info!("Soul: removed stale skill at {}", stale.display());
                migrated = true;
            }
            Err(e) => tracing::warn!(
                "Soul: failed to remove stale skill at {}: {e}",
                stale.display()
            ),
        }
    }

    let skill_file = dir.join("SKILL.md");
    std::fs::create_dir_all(&dir)?;
    let installed = create_built_in_file_if_missing(&skill_file, SOUL_SKILL_MD)?;
    if installed {
        tracing::info!("Soul: installed skill at {}", skill_file.display());
    }
    Ok(installed || migrated)
}

/// Register the daily 3:00 AM Meditation job if it is not already scheduled.
pub async fn ensure_meditation_schedule(
    scheduler: &Arc<dyn SchedulerTrait>,
    workflow_path: PathBuf,
) -> anyhow::Result<()> {
    let jobs = scheduler.list_scheduled_jobs().await;
    if let Some(upgraded) = upgrade_existing_meditation_workflow(
        &jobs,
        MEDITATION_WORKFLOW_YAML,
        PREVIOUS_MEDITATION_WORKFLOW_SHA256,
    )? {
        if upgraded {
            tracing::info!(
                "Soul: upgraded the workflow copy used by the existing Daily Meditation schedule"
            );
        }
        return Ok(());
    }
    let job = ScheduledJob {
        id: MEDITATION_SCHEDULE_ID.to_string(),
        source: workflow_path.to_string_lossy().into_owned(),
        cron: MEDITATION_CRON.to_string(),
        last_run: None,
        currently_running: false,
        paused: false,
        current_session_id: None,
        process_start_time: None,
        run_count: 0,
        max_runs: None,
        // A machine-wide meditation schedule belongs to no chat.
        creator_session_id: None,
        last_error: None,
        owns_source: None,
    };
    scheduler
        .add_scheduled_job(job, true)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    tracing::info!("Soul: registered Daily Meditation 03:00 schedule '{MEDITATION_SCHEDULE_ID}'");
    Ok(())
}

/// Upgrade the scheduler-owned workflow copy without replacing the schedule.
///
/// Returning `Some` means the job already exists. Keeping the existing job is
/// important: removing and adding it again would erase its successful-run
/// cursor, pause state, run count, and any active-run bookkeeping.
fn upgrade_existing_meditation_workflow(
    jobs: &[ScheduledJob],
    content: &str,
    previous_sha256: &[&str],
) -> anyhow::Result<Option<bool>> {
    let Some(job) = jobs.iter().find(|job| job.id == MEDITATION_SCHEDULE_ID) else {
        return Ok(None);
    };
    if job.source.trim().is_empty() {
        anyhow::bail!("the existing Daily Meditation schedule has no workflow source");
    }
    create_or_upgrade_built_in_file(Path::new(&job.source), content, previous_sha256).map(Some)
}

/// The "Meditation" workflow definition. It uses the user's configured
/// default provider/model (no `settings` override), loads the update-soul
/// skill, focuses on the Soul KB, and instructs the agent to digest recent
/// user interactions into durable, personalised knowledge.
pub const MEDITATION_WORKFLOW_YAML: &str = r#"version: 1.0.2
title: Meditation
description: >-
  Review the user's recent Biorouter sessions and save what matters about them
  into the built-in "Soul" knowledge base: how they approach scientific
  questions, the tools and commands they use, the tool responses they rely on,
  and lasting personal details such as name, role, and preferences. Runs daily
  at 3:00 AM by default as "Daily Meditation".
instructions: |-
  You are maintaining the user's personal "Soul" knowledge base (id: soul).
  Follow the `update-soul` skill exactly.

  Goal: turn the user's recent interaction history into durable, high-signal,
  personalised knowledge about THE USER, not a summary of every chat.

  Procedure:
  1. Find the user's recent REAL chat sessions with exactly one `chatrecall`
     recent-mode call: omit both `query` and `session_id`, pass the supplied
     `after_date` cursor, and set `limit` to 10. This lists recent sessions
     without guessing vocabulary; do not substitute a keyword or synonym
     search. Skip scheduled-job sessions (names starting with "Scheduled job:"),
     especially this very session. Select at most three real sessions,
     prioritising the most recent high-signal work over greetings or routine
     follow-ups.
  2. Call `platform__ingest_conversation` with EXPLICIT `session_ids` for the
     most relevant recent session(s), targeting the `soul` knowledge base.
     Never omit `session_ids`: omitting it defaults to the current scheduled
     session, which contains nothing about the user. Prefer sessions since
     the last Meditation. If chatrecall surfaces no real user sessions, stop
     and make no changes.
  3. Prioritise capturing:
       - the way the user approaches different scientific questions,
       - the tools and extensions they use and how they call them,
       - the commands they run and the responses they rely on,
       - personal information they reveal: name, role/occupation, affiliations,
         stated preferences and working style.
  4. Explicitly DISCARD low-value noise: greetings, chit-chat, one-off
     irrelevant details, and anything that would not help a future assistant
     serve this user better.
  5. Keep the Soul coherent: use ordinary OKF markdown links to related page
     paths, avoid duplicating facts already present, and prefer updating an
     existing page over creating a near-duplicate.

  If there is nothing new worth recording, say so and make no changes.
extensions:
- type: builtin
  name: knowledge
  display_name: Knowledge
  description: Read, search, validate, and update Biorouter knowledge bases
  timeout: 300
  bundled: true
  available_tools: []
- type: platform
  name: skills
  description: Search the skills installed on this machine and load the one that matches the task in hand
  bundled: true
  available_tools: []
- type: platform
  name: chatrecall
  description: Search your earlier chats and load a summary of one, so work you already did can be picked up here
  bundled: true
  available_tools: []
knowledge_bases:
  default: soul
  visible:
  - soul
skills:
- update-soul
activities:
- Update my Soul from recent interactions
- Learn my preferences and working style
- Record the tools and commands I use
parameters: []
"#;

/// The update-soul skill — guidance the agent loads when writing the Soul.
pub const SOUL_SKILL_MD: &str = r#"---
name: update-soul
description: >-
  Update the user's personal "Soul" knowledge base from their conversation
  history. Load this skill when running a Meditation, or whenever asked to learn
  about, remember, or record durable facts about the user. It defines what to
  keep: how the user approaches scientific questions, the tools and commands
  they use, the tool responses they rely on, and personal details such as name,
  role, affiliation, and stated preferences, and what to leave out (greetings,
  small talk, and one-off transient details). It also covers how to write Soul
  OKF pages: how to choose stable `type` and `identifier` values, cite sources,
  cross-link with ordinary markdown links, and prefer a few durable facts over
  many shallow ones.
---

# Writing the Soul

"Soul" is the user's personal knowledge base (`soul`). Its purpose is to make
future assistance better by remembering durable, high-signal facts about **the
user**, not to log conversations verbatim.

## What to capture (high value)

- **Approach to scientific questions.** How the user frames problems, the
  assumptions they make, the methods and statistical choices they prefer, the
  trade-offs they weigh.
- **Tools and extensions.** Which tools the user reaches for, in what order, and
  why. Note recurring workflows.
- **Commands and tool calls.** Concrete commands and the shape of tool calls the
  user runs (e.g. specific CLIs, query patterns, file layouts); generalise them
  into reusable knowledge rather than copying one-off arguments.
- **Tool responses they rely on.** What outputs the user treats as authoritative
  or acts upon.
- **Personal information.** Name, occupation/role, lab or affiliation, domain of
  expertise, and explicitly stated preferences (formatting, verbosity, tone,
  preferred models/providers, working hours).

## What to discard (low value)

- Greetings, sign-offs, and small talk ("hi", "thanks", "ok").
- One-off, irrelevant, or transient details that won't help future sessions.
- Anything already recorded; update the existing page instead of duplicating.

## How to write it

- Soul is an **OKF** knowledge base. Read its `schema.md` before writing and use
  the Knowledge MCP tools; never edit its on-disk directory directly.
- Search before creating. Create or update pages at
  `knowledge/<lowercase-type>/<slug>.md`, reusing a small, consistent set of
  open-vocabulary types such as `Person`, `Tool`, `Preference`, `Method`, and
  `Observation`.
- Give every page OKF frontmatter with a non-empty `type` and a stable,
  human-readable `identifier`. Record source metadata when a claim came from a
  document or conversation digest.
- Validate a draft with `kb_validate_page` before `kb_write_page`.
- Cross-reference related pages with ordinary markdown links to their page
  paths, for example `[prefers](/knowledge/preference/visualisation-tools.md)`.
  Do not write legacy `[[double bracket]]` links.
- Prefer a few well-formed, durable facts over many shallow ones.
- Each fact should read as a natural-language statement a future assistant could
  act on, e.g. "Prefers ggplot2 over base R for figures" rather than "used
  ggplot once".
- If a conversation yields nothing durable, record nothing.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::Workflow;

    #[test]
    fn workflow_yaml_parses_into_a_valid_workflow() {
        let wf: Workflow = serde_yaml::from_str(MEDITATION_WORKFLOW_YAML)
            .expect("Meditation workflow YAML must deserialize");
        assert_eq!(wf.title, "Meditation");
        assert_eq!(wf.version, "1.0.2");
        let kbs = wf.knowledge_bases.expect("knowledge_bases present");
        assert_eq!(kbs.default.as_deref(), Some("soul"));
        assert!(kbs.visible.iter().any(|k| k == "soul"));
        assert!(wf
            .skills
            .unwrap_or_default()
            .iter()
            .any(|s| s == "update-soul"));
    }

    #[test]
    fn meditation_keeps_discovery_bounded_and_omits_unneeded_todo_state() {
        let wf: Workflow = serde_yaml::from_str(MEDITATION_WORKFLOW_YAML)
            .expect("Meditation workflow YAML must deserialize");
        let instructions = wf.instructions.expect("Meditation instructions");
        assert!(instructions.contains("exactly one"), "{instructions}");
        assert!(instructions.contains("omit both `query`"), "{instructions}");
        assert!(instructions.contains("at most three"), "{instructions}");
        assert!(
            wf.extensions
                .unwrap_or_default()
                .iter()
                .all(|extension| !extension.name().eq_ignore_ascii_case("todo")),
            "Meditation does not need a task list for its fixed procedure"
        );
    }

    /// The skill lands in the knowledge bundle, and a pre-bundle install is
    /// migrated rather than duplicated.
    ///
    /// ⚠ **`bundle_name` is the assertion that matters.** Writing to the right
    /// path and being read back with `bundle_name: None` looks entirely correct
    /// on disk while leaving this skill a standalone picker row that the
    /// bundle's one Settings switch cannot reach — which is the whole point of
    /// moving it.
    #[test]
    fn the_soul_skill_is_seeded_into_the_knowledge_bundle() {
        use crate::agents::skills_extension::{SkillsClient, KNOWLEDGE_BUNDLE};

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("skills");

        assert!(place_soul_skill(&root).unwrap(), "a first seed is a change");
        let seeded = root
            .join(KNOWLEDGE_BUNDLE)
            .join(SOUL_SKILL_DIR)
            .join("SKILL.md");
        assert!(seeded.is_file(), "not seeded into the bundle");
        assert!(!root.join(SOUL_SKILL_DIR).exists(), "also seeded flat");

        let discovered = SkillsClient::discover_skills_in_directories(std::slice::from_ref(&root));
        assert_eq!(
            discovered[SOUL_SKILL_DIR].bundle_name.as_deref(),
            Some(KNOWLEDGE_BUNDLE),
            "discovered without its bundle, so no bundle toggle reaches it"
        );

        // Idempotent: a second pass changes nothing.
        assert!(!place_soul_skill(&root).unwrap());
    }

    /// ⚠ **The user's edits survive the move.** This skill is written with
    /// `create_built_in_file_if_missing` so that a user who tailors it keeps
    /// their version forever; a migration that deleted the flat directory would
    /// break that promise on the one startup that relocates it, silently, and
    /// with no way back.
    #[test]
    fn migrating_a_pre_bundle_soul_skill_keeps_what_the_user_wrote() {
        use crate::agents::skills_extension::{SkillsClient, KNOWLEDGE_BUNDLE};

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("skills");
        let flat = root.join(SOUL_SKILL_DIR);
        std::fs::create_dir_all(&flat).unwrap();
        let edited = format!("{SOUL_SKILL_MD}\n\nAlways call the user Dr Gu.\n");
        std::fs::write(flat.join("SKILL.md"), &edited).unwrap();
        // The pre-rename folder too, which has no edits worth keeping.
        let ancient = root.join(SOUL_SKILL_DIR_LEGACY);
        std::fs::create_dir_all(&ancient).unwrap();
        std::fs::write(ancient.join("SKILL.md"), "old").unwrap();

        assert!(place_soul_skill(&root).unwrap());

        let moved = root
            .join(KNOWLEDGE_BUNDLE)
            .join(SOUL_SKILL_DIR)
            .join("SKILL.md");
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), edited);
        assert!(
            !flat.exists(),
            "the flat copy would resurrect as a duplicate"
        );
        assert!(!ancient.exists(), "the pre-rename folder is still there");

        // One candidate for the name, and it carries the bundle.
        let discovered = SkillsClient::discover_skills_in_directories(&[root]);
        assert_eq!(
            discovered[SOUL_SKILL_DIR].bundle_name.as_deref(),
            Some(KNOWLEDGE_BUNDLE)
        );
    }

    /// A flat copy alongside an existing bundled one is a duplicate, and the
    /// bundled one wins. Without this arm the two would race in `read_dir`.
    #[test]
    fn a_flat_copy_beside_a_bundled_one_is_removed() {
        use crate::agents::skills_extension::KNOWLEDGE_BUNDLE;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("skills");
        let bundled = root.join(KNOWLEDGE_BUNDLE).join(SOUL_SKILL_DIR);
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::write(bundled.join("SKILL.md"), "the bundled one").unwrap();
        let flat = root.join(SOUL_SKILL_DIR);
        std::fs::create_dir_all(&flat).unwrap();
        std::fs::write(flat.join("SKILL.md"), "the stale one").unwrap();

        assert!(place_soul_skill(&root).unwrap());

        assert!(!flat.exists());
        assert_eq!(
            std::fs::read_to_string(bundled.join("SKILL.md")).unwrap(),
            "the bundled one",
            "the stale flat copy overwrote the bundled one"
        );
    }

    #[test]
    fn skill_md_has_frontmatter_name() {
        assert!(SOUL_SKILL_MD.starts_with("---\n"));
        assert!(SOUL_SKILL_MD.contains("name: update-soul"));
        assert!(SOUL_SKILL_MD.contains("Soul is an **OKF** knowledge base"));
        assert!(SOUL_SKILL_MD.contains("kb_validate_page"));
        assert!(!SOUL_SKILL_MD.contains("Cross-reference related pages with `[["));
        assert!(!MEDITATION_WORKFLOW_YAML.contains("[[wiki-links]]"));
    }

    #[test]
    fn shipped_assets_preserve_existing_content_and_create_only_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(MEDITATION_WORKFLOW_FILE);
        let customized = "user-customized workflow";
        std::fs::write(&path, customized).unwrap();

        assert!(!create_built_in_file_if_missing(&path, MEDITATION_WORKFLOW_YAML).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), customized);

        let missing = tmp.path().join("missing.yaml");
        assert!(create_built_in_file_if_missing(&missing, MEDITATION_WORKFLOW_YAML).unwrap());
        assert_eq!(
            std::fs::read_to_string(missing).unwrap(),
            MEDITATION_WORKFLOW_YAML
        );
    }

    #[test]
    fn shipped_workflow_upgrades_only_a_byte_identical_previous_version() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(MEDITATION_WORKFLOW_FILE);
        let previous = "previous shipped workflow\n";
        std::fs::write(&path, previous).unwrap();
        let previous_sha256 = format!("{:x}", Sha256::digest(previous.as_bytes()));

        assert!(create_or_upgrade_built_in_file(
            &path,
            MEDITATION_WORKFLOW_YAML,
            &[previous_sha256.as_str()],
        )
        .unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            MEDITATION_WORKFLOW_YAML
        );

        let customized = "user-customized workflow\n";
        std::fs::write(&path, customized).unwrap();
        assert!(!create_or_upgrade_built_in_file(
            &path,
            MEDITATION_WORKFLOW_YAML,
            &["not-a-match"]
        )
        .unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), customized);
    }

    #[test]
    fn existing_schedule_upgrades_its_stock_copy_without_resetting_the_job() {
        let tmp = tempfile::tempdir().unwrap();
        let scheduled_copy = tmp.path().join("daily-meditation.yaml");
        let previous = "previous shipped workflow\n";
        std::fs::write(&scheduled_copy, previous).unwrap();
        let previous_sha256 = format!("{:x}", Sha256::digest(previous.as_bytes()));
        let jobs = vec![ScheduledJob {
            id: MEDITATION_SCHEDULE_ID.to_string(),
            source: scheduled_copy.to_string_lossy().into_owned(),
            cron: MEDITATION_CRON.to_string(),
            last_run: Some(chrono::Utc::now()),
            currently_running: false,
            paused: true,
            current_session_id: None,
            process_start_time: None,
            run_count: 7,
            max_runs: None,
            creator_session_id: None,
            last_error: Some("preserved diagnostic".to_string()),
            owns_source: None,
        }];
        let metadata_before = serde_json::to_value(&jobs[0]).unwrap();

        assert_eq!(
            upgrade_existing_meditation_workflow(
                &jobs,
                MEDITATION_WORKFLOW_YAML,
                &[previous_sha256.as_str()],
            )
            .unwrap(),
            Some(true)
        );
        assert_eq!(
            std::fs::read_to_string(&scheduled_copy).unwrap(),
            MEDITATION_WORKFLOW_YAML
        );
        assert_eq!(serde_json::to_value(&jobs[0]).unwrap(), metadata_before);

        let customized = "user-customized scheduled workflow\n";
        std::fs::write(&scheduled_copy, customized).unwrap();
        assert_eq!(
            upgrade_existing_meditation_workflow(
                &jobs,
                MEDITATION_WORKFLOW_YAML,
                &[previous_sha256.as_str()],
            )
            .unwrap(),
            Some(false)
        );
        assert_eq!(std::fs::read_to_string(scheduled_copy).unwrap(), customized);
    }

    fn make_legacy(svc: &KnowledgeService, id: &str, name: &str) {
        svc.create_base(id, name, None).unwrap();
        let root = biorouter_mcp::knowledge::paths::kb_root(svc.root(), id);
        let mut manifest = biorouter_mcp::knowledge::manifest::load(&root).unwrap();
        manifest.schema_version = biorouter_mcp::knowledge::types::AUTOMATIC_SCHEMA_CEILING;
        biorouter_mcp::knowledge::manifest::save(&root, &manifest).unwrap();
    }

    #[test]
    fn startup_purges_pre_okf_bases_and_recreates_soul_as_okf() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        make_legacy(&svc, SOUL_KB_ID, SOUL_KB_NAME);
        make_legacy(&svc, "old-project", "Old project");
        svc.create_base_in("current-bio", "Current Bio", None, KbFormat::Biookf)
            .unwrap();

        let old_soul_page = tmp
            .path()
            .join(SOUL_KB_ID)
            .join("knowledge")
            .join("notes")
            .join("legacy.md");
        std::fs::create_dir_all(old_soul_page.parent().unwrap()).unwrap();
        std::fs::write(&old_soul_page, "---\ntitle: Old\nkind: note\n---\n").unwrap();

        let outcome = reconcile_soul_kb(&svc).unwrap();
        assert!(outcome.removed.iter().any(|id| id == SOUL_KB_ID));
        assert!(outcome.removed.iter().any(|id| id == "old-project"));
        assert!(outcome.quarantined.is_empty(), "{outcome:?}");
        assert!(!tmp.path().join("old-project").exists());
        assert!(!old_soul_page.exists());

        let soul = svc.get_base(SOUL_KB_ID).unwrap();
        assert_eq!(soul.profile(), Some(KbFormat::Okf));
        assert_eq!(
            soul.schema_version,
            biorouter_mcp::knowledge::types::CURRENT_SCHEMA_VERSION
        );
        assert!(tmp.path().join(SOUL_KB_ID).join("schema.md").exists());
        assert!(svc.get_base("current-bio").is_ok());
        assert_eq!(
            reconcile_soul_kb(&svc).unwrap(),
            ReconcileOutcome::default()
        );
    }

    #[test]
    fn legacy_purge_uses_registry_identity_not_untrusted_manifest_id() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        make_legacy(&svc, "old-project", "Old project");
        svc.create_base_in("current-bio", "Current Bio", None, KbFormat::Biookf)
            .unwrap();

        let legacy_root = tmp.path().join("old-project");
        let mut legacy = manifest::load(&legacy_root).unwrap();
        legacy.id = "current-bio".to_string();
        manifest::save(&legacy_root, &legacy).unwrap();

        let outcome = reconcile_soul_kb(&svc).unwrap();
        assert_eq!(outcome.removed, vec!["old-project"]);
        assert!(outcome.quarantined.is_empty(), "{outcome:?}");
        assert!(!legacy_root.exists());
        assert_eq!(
            svc.get_base("current-bio").unwrap().profile(),
            Some(KbFormat::Biookf)
        );
    }

    /// A registry row that names somebody else's directory is quarantined, and
    /// **neither** base is touched — that half is the one that matters and it
    /// is unchanged.
    ///
    /// What changed is the fatality: this used to `bail!`, which took the whole
    /// daemon down with it (no `AppState`, no listener, no window) over one
    /// third-party row the user then had no running app to go and repair.
    #[test]
    fn legacy_purge_quarantines_a_noncanonical_registry_path_without_deleting_either_base() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        make_legacy(&svc, "old-project", "Old project");
        svc.create_base_in("current-bio", "Current Bio", None, KbFormat::Biookf)
            .unwrap();
        registry::replace(
            tmp.path(),
            "old-project",
            RegistryEntry {
                id: "old-project".to_string(),
                path: tmp.path().join("current-bio"),
            },
        )
        .unwrap();

        let outcome = reconcile_soul_kb(&svc).unwrap();
        assert_eq!(outcome.quarantined, vec!["old-project".to_string()]);
        assert!(outcome.removed.is_empty(), "{outcome:?}");
        assert!(tmp.path().join("old-project").exists());
        assert_eq!(
            svc.get_base("current-bio").unwrap().profile(),
            Some(KbFormat::Biookf)
        );
        // …and Soul is still established, which is the whole point of not
        // aborting on somebody else's row.
        assert_eq!(
            svc.get_base(SOUL_KB_ID).unwrap().profile(),
            Some(KbFormat::Okf)
        );
    }

    #[test]
    fn startup_preserves_native_soul_customizations_and_converts_biookf_pages_in_place() {
        let current = tempfile::tempdir().unwrap();
        let current_svc = KnowledgeService::new(current.path().to_path_buf());
        current_svc
            .create_base_in(SOUL_KB_ID, SOUL_KB_NAME, None, KbFormat::Okf)
            .unwrap();
        let page = current
            .path()
            .join(SOUL_KB_ID)
            .join("knowledge")
            .join("note")
            .join("preference.md");
        let page_content = "---\ntype: Preference\nidentifier: Editor\n---\ncustom page\n";
        std::fs::write(&page, page_content).unwrap();
        let current_schema = current.path().join(SOUL_KB_ID).join("schema.md");
        let custom_schema = "# User-customized native OKF schema\n";
        std::fs::write(&current_schema, custom_schema).unwrap();
        assert_eq!(
            reconcile_soul_kb(&current_svc).unwrap(),
            ReconcileOutcome::default()
        );
        assert_eq!(std::fs::read_to_string(&page).unwrap(), page_content);
        assert_eq!(
            std::fs::read_to_string(&current_schema).unwrap(),
            custom_schema
        );

        let bio = tempfile::tempdir().unwrap();
        let bio_svc = KnowledgeService::new(bio.path().to_path_buf());
        bio_svc
            .create_base_in(SOUL_KB_ID, SOUL_KB_NAME, None, KbFormat::Biookf)
            .unwrap();
        let bio_page = bio
            .path()
            .join(SOUL_KB_ID)
            .join("knowledge")
            .join("Person")
            .join("user.md");
        std::fs::create_dir_all(bio_page.parent().unwrap()).unwrap();
        let bio_page_content = "---\ntype: Person\nidentifier: User\n---\nuser content\n";
        std::fs::write(&bio_page, bio_page_content).unwrap();
        assert_eq!(
            reconcile_soul_kb(&bio_svc).unwrap(),
            ReconcileOutcome::default()
        );
        let converted = bio_svc.get_base(SOUL_KB_ID).unwrap();
        assert_eq!(converted.profile(), Some(KbFormat::Okf));
        assert_eq!(converted.biookf_version, None);
        assert_eq!(
            std::fs::read_to_string(&bio_page).unwrap(),
            bio_page_content
        );
        assert_eq!(
            std::fs::read_to_string(bio.path().join(SOUL_KB_ID).join("schema.md")).unwrap(),
            SOUL_OKF_SCHEMA
        );
    }

    #[test]
    fn startup_does_not_record_graph_cache_refresh_as_soul_recovery() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base_in(SOUL_KB_ID, SOUL_KB_NAME, None, KbFormat::Okf)
            .unwrap();
        let soul_root = tmp.path().join(SOUL_KB_ID);
        let repo = git2::Repository::open(&soul_root).unwrap();
        let before = repo.head().unwrap().target().unwrap();
        std::fs::write(
            soul_root.join(".biorouter-knowledge/graph-cache.json"),
            "{\"stale_cache_fixture\": true}\n",
        )
        .unwrap();

        for _ in 0..2 {
            assert_eq!(
                reconcile_soul_kb(&svc).unwrap(),
                ReconcileOutcome::default()
            );
            assert_eq!(repo.head().unwrap().target().unwrap(), before);
            assert!(!std::fs::read_to_string(
                soul_root.join(".biorouter-knowledge/graph-cache.json")
            )
            .unwrap()
            .contains("stale_cache_fixture"));
        }

        let page = soul_root.join("knowledge/note/recovered.md");
        let content = "---\ntype: Note\nidentifier: Recovered fixture\n---\nKeep this page.\n";
        std::fs::write(&page, content).unwrap();
        reconcile_soul_kb(&svc).unwrap();
        let recovered = repo.head().unwrap().target().unwrap();
        assert_ne!(recovered, before);
        assert_eq!(std::fs::read_to_string(page).unwrap(), content);
        reconcile_soul_kb(&svc).unwrap();
        assert_eq!(repo.head().unwrap().target().unwrap(), recovered);
    }

    #[test]
    fn startup_recovers_unregistered_current_soul_without_overwriting_content() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base_in(SOUL_KB_ID, SOUL_KB_NAME, None, KbFormat::Okf)
            .unwrap();
        let soul_root = tmp.path().join(SOUL_KB_ID);
        let page = soul_root.join("knowledge/note/preference.md");
        let page_content = "---\ntype: Preference\nidentifier: Editor\n---\nkeep me\n";
        std::fs::write(&page, page_content).unwrap();
        let schema = soul_root.join("schema.md");
        let custom_schema = "# User-customized native OKF schema\n";
        std::fs::write(&schema, custom_schema).unwrap();
        registry::unregister(tmp.path(), SOUL_KB_ID).unwrap();
        tier::forget_unlocked(tmp.path(), SOUL_KB_ID).unwrap();
        std::fs::remove_dir_all(soul_root.join(".git")).unwrap();

        assert_eq!(
            reconcile_soul_kb(&svc).unwrap(),
            ReconcileOutcome::default()
        );

        let entries = registry::load(tmp.path()).unwrap();
        assert_eq!(
            entries,
            vec![RegistryEntry {
                id: SOUL_KB_ID.to_string(),
                path: soul_root.clone(),
            }]
        );
        assert_eq!(
            svc.get_base(SOUL_KB_ID).unwrap().profile(),
            Some(KbFormat::Okf)
        );
        assert_eq!(std::fs::read_to_string(page).unwrap(), page_content);
        assert_eq!(std::fs::read_to_string(schema).unwrap(), custom_schema);
        assert!(soul_root.join(".git").exists());
        assert!(!tier::is_private(tmp.path(), SOUL_KB_ID));
    }

    /// D1. The reconciler classifies a base with **no** lock held and only then
    /// blocks on `flock`, which has no deadline; the delete under it used to
    /// check nothing but `kb_root.exists()`. So the base it destroys need not be
    /// the base it looked at.
    ///
    /// The racing operation here is not contrived: there is no in-place legacy →
    /// OKF upgrade in this build, so delete-then-recreate at the same id is the
    /// *only* way a user can move an id off the legacy format — it is what this
    /// build's own refusal message tells them to do.
    ///
    /// The existing 8-thread "concurrency" tests cannot reach this: all eight
    /// run `reconcile_soul_kb`, which serializes on `SOUL_RECONCILE_LOCK`, a
    /// lock no writer anywhere takes. The racing party has to be a writer.
    #[test]
    fn a_legacy_base_replaced_while_the_purge_waits_survives_with_its_history() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        make_legacy(&svc, "ms-cohort", "MS cohort");

        let root = tmp.path().to_path_buf();
        let mut replaced = false;
        let outcome = reconcile_soul_kb_with_checkpoint(&svc, |checkpoint| {
            if let ReconcileCheckpoint::ClassifiedRegisteredLegacy("ms-cohort") = checkpoint {
                let other_process = KnowledgeService::new(root.clone());
                other_process.delete_base("ms-cohort")?;
                other_process.create_base_in("ms-cohort", "MS cohort", None, KbFormat::Biookf)?;
                std::fs::write(
                    root.join("ms-cohort/knowledge/finding.md"),
                    "---\ntype: Observation\nidentifier: Finding\n---\nnew work\n",
                )?;
                replaced = true;
            }
            Ok(())
        })
        .unwrap();

        assert!(replaced, "the classification checkpoint never fired");
        assert!(
            !outcome.removed.iter().any(|id| id == "ms-cohort"),
            "{outcome:?}"
        );
        assert_eq!(
            svc.get_base("ms-cohort").unwrap().profile(),
            Some(KbFormat::Biookf)
        );
        assert!(tmp.path().join("ms-cohort/knowledge/finding.md").exists());
        assert!(
            tmp.path().join("ms-cohort/.git").exists(),
            "the replacement's git history must survive too"
        );
    }

    /// D2, first half. A current BioOKF base whose `manifest.yaml` lost one line
    /// to a partial write is not a legacy base, however
    /// `Manifest::is_legacy_format` reads it — every field of `Manifest`
    /// defaults, so the deserializer cannot tell "generation 1" from "no
    /// generation stated".
    #[test]
    fn a_current_base_whose_manifest_lost_its_generation_is_kept_not_purged() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base_in("ms-cohort", "MS cohort", None, KbFormat::Biookf)
            .unwrap();
        let page = tmp.path().join("ms-cohort/knowledge/keep-me.md");
        let page_content = "---\ntype: Observation\nidentifier: Keep\n---\nkeep me\n";
        std::fs::write(&page, page_content).unwrap();

        let kb_root = tmp.path().join("ms-cohort");
        let damaged = std::fs::read_to_string(manifest::manifest_path(&kb_root))
            .unwrap()
            .lines()
            .filter(|line| !line.starts_with("schema_version:"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(manifest::manifest_path(&kb_root), format!("{damaged}\n")).unwrap();
        assert!(
            manifest::load(&kb_root).unwrap().is_legacy_format(),
            "the deserializer really does read this as legacy — that is the trap"
        );

        let outcome = reconcile_soul_kb(&svc).unwrap();

        assert!(outcome.removed.is_empty(), "{outcome:?}");
        assert_eq!(outcome.quarantined, vec!["ms-cohort".to_string()]);
        assert_eq!(std::fs::read_to_string(&page).unwrap(), page_content);
        assert!(kb_root.join(".git").exists());
    }

    /// D2, second half. `~/.config/biorouter/knowledge/` is a directory on the
    /// user's disk; anything may sit in it. Every manifest below deserializes
    /// into an all-defaults `Manifest` and therefore answers
    /// `is_legacy_format() == true`, and the purge used to `remove_dir_all` all
    /// three without recording a thing.
    #[test]
    fn an_unrelated_directory_under_the_knowledge_root_is_never_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        let foreign = [
            ("some-other-tool", "name: Something Else\nversion: 4\n"),
            ("empty-mapping", "{}\n"),
            // Even a *stated* pre-OKF generation is not enough on its own: the
            // tree has to look like a base this build wrote.
            ("stated-but-foreign", "schema_version: 1\nwhatever: true\n"),
        ];
        for (id, document) in foreign {
            let dir = tmp.path().join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("manifest.yaml"), document).unwrap();
            std::fs::write(dir.join("payload.txt"), "someone else's data").unwrap();
        }

        let outcome = reconcile_soul_kb(&svc).unwrap();

        assert!(outcome.removed.is_empty(), "{outcome:?}");
        for (id, _) in foreign {
            assert!(
                tmp.path().join(id).join("payload.txt").exists(),
                "'{id}' was destroyed"
            );
        }
    }

    /// D4. Every existing test builds its bases with `make_legacy`, which writes
    /// a well-formed manifest — so none of them could reach the chain that ends
    /// at `AppState::new`. One unrelated base nobody asked about used to be
    /// enough to leave the user with no daemon, no listener and no window.
    #[test]
    fn a_damaged_unrelated_base_does_not_stop_the_reconciliation() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        svc.create_base_in("healthy", "Healthy", None, KbFormat::Biookf)
            .unwrap();

        let broken = tmp.path().join("notes-2024");
        std::fs::create_dir_all(broken.join("knowledge")).unwrap();
        std::fs::write(broken.join("schema.md"), "# schema\n").unwrap();
        std::fs::write(broken.join("manifest.yaml"), "id: [unclosed\nname: Notes\n").unwrap();
        assert!(
            manifest::load(&broken).is_err(),
            "the fixture must be broken"
        );
        registry::register(
            tmp.path(),
            RegistryEntry {
                id: "notes-2024".to_string(),
                path: broken.clone(),
            },
        )
        .unwrap();

        let outcome = reconcile_soul_kb(&svc).unwrap();

        assert_eq!(outcome.quarantined, vec!["notes-2024".to_string()]);
        assert!(outcome.removed.is_empty(), "{outcome:?}");
        assert!(
            broken.join("manifest.yaml").exists(),
            "an undiagnosable base is left alone, not destroyed"
        );
        assert_eq!(
            svc.get_base(SOUL_KB_ID).unwrap().profile(),
            Some(KbFormat::Okf)
        );
        assert_eq!(
            svc.get_base("healthy").unwrap().profile(),
            Some(KbFormat::Biookf)
        );
    }

    /// D5. The other daemon sharing this store deletes a base while this one is
    /// walking the root. `symlink_metadata` then returns `NotFound`, and a bare
    /// `?` there aborted startup over a directory that was *supposed* to go
    /// away.
    #[test]
    fn a_directory_that_vanishes_mid_scan_does_not_abort_the_purge() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        make_legacy(&svc, "vanisher", "Vanisher");
        make_legacy(&svc, "witness", "Witness");
        registry::unregister(tmp.path(), "vanisher").unwrap();
        registry::unregister(tmp.path(), "witness").unwrap();

        let root = tmp.path().to_path_buf();
        let mut vanished = false;
        let mut classified = Vec::new();
        let outcome = reconcile_soul_kb_with_checkpoint(&svc, |checkpoint| {
            match checkpoint {
                ReconcileCheckpoint::ScannedUnregisteredEntry("vanisher") => {
                    std::fs::remove_dir_all(root.join("vanisher"))?;
                    vanished = true;
                }
                ReconcileCheckpoint::ClassifiedUnregisteredLegacy(id) => {
                    classified.push(id.to_string());
                }
                _ => {}
            }
            Ok(())
        })
        .unwrap();

        assert!(vanished, "the scan checkpoint never fired");
        assert_eq!(
            classified,
            vec!["witness".to_string()],
            "the vanished entry must never reach a delete decision"
        );
        assert!(!tmp.path().join("vanisher").exists());
        // `read_dir` order is not defined, so this covers both interleavings:
        // the run completing at all rules out an abort before `witness`, and
        // `witness` being gone rules out an abort after it.
        assert!(!tmp.path().join("witness").exists());
        assert!(outcome.removed.iter().any(|id| id == "witness"));
        assert_eq!(
            svc.get_base(SOUL_KB_ID).unwrap().profile(),
            Some(KbFormat::Okf)
        );
    }

    #[test]
    fn startup_purges_unregistered_legacy_residue_and_recreates_native_soul() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        make_legacy(&svc, SOUL_KB_ID, SOUL_KB_NAME);
        make_legacy(&svc, "orphaned-legacy", "Orphaned legacy");
        registry::unregister(tmp.path(), SOUL_KB_ID).unwrap();
        registry::unregister(tmp.path(), "orphaned-legacy").unwrap();

        let outcome = reconcile_soul_kb(&svc).unwrap();

        assert!(outcome.removed.iter().any(|id| id == SOUL_KB_ID));
        assert!(outcome.removed.iter().any(|id| id == "orphaned-legacy"));
        assert!(outcome.quarantined.is_empty(), "{outcome:?}");
        assert!(!tmp.path().join("orphaned-legacy").exists());
        assert_eq!(
            svc.get_base(SOUL_KB_ID).unwrap().profile(),
            Some(KbFormat::Okf)
        );
    }

    #[test]
    fn concurrent_zero_base_reconciliation_converges_without_startup_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let root = tmp.path().to_path_buf();
            let barrier = std::sync::Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                let svc = KnowledgeService::new(root);
                barrier.wait();
                reconcile_soul_kb(&svc)
            }));
        }
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        let bases = svc.list_bases().unwrap();
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0].id, SOUL_KB_ID);
        assert_eq!(bases[0].profile(), Some(KbFormat::Okf));
    }

    #[test]
    fn concurrent_legacy_purge_converges_with_no_stale_base_or_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        make_legacy(&svc, SOUL_KB_ID, SOUL_KB_NAME);
        make_legacy(&svc, "old-project", "Old project");
        svc.create_base_in("current-bio", "Current Bio", None, KbFormat::Biookf)
            .unwrap();
        std::fs::write(
            tmp.path().join("old-project/knowledge/notes.md"),
            "---\ntitle: old\nkind: note\n---\n",
        )
        .unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let root = tmp.path().to_path_buf();
            let barrier = std::sync::Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                let svc = KnowledgeService::new(root);
                barrier.wait();
                reconcile_soul_kb(&svc)
            }));
        }
        for thread in threads {
            thread.join().unwrap().unwrap();
        }

        let svc = KnowledgeService::new(tmp.path().to_path_buf());
        assert!(svc.base_is_current_or_fully_removed("old-project").unwrap());
        assert!(!tmp.path().join("old-project").exists());
        assert_eq!(
            svc.get_base(SOUL_KB_ID).unwrap().profile(),
            Some(KbFormat::Okf)
        );
        assert_eq!(
            svc.get_base("current-bio").unwrap().profile(),
            Some(KbFormat::Biookf)
        );
        assert_eq!(svc.list_bases().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn ensure_meditation_schedule_is_idempotent() {
        use crate::scheduler::Scheduler;
        use crate::session::session_manager::SessionManager;

        let tmp = tempfile::tempdir().unwrap();
        // A real workflow file the scheduler can copy.
        let wf = tmp.path().join("meditation.yaml");
        std::fs::write(&wf, MEDITATION_WORKFLOW_YAML).unwrap();

        let storage = tmp.path().join("schedule.json");
        let session_manager = Arc::new(SessionManager::new(tmp.path().to_path_buf()));
        let scheduler: Arc<dyn SchedulerTrait> =
            Scheduler::new(storage, session_manager).await.unwrap();

        ensure_meditation_schedule(&scheduler, wf.clone())
            .await
            .unwrap();
        // Second call must not create a duplicate.
        ensure_meditation_schedule(&scheduler, wf).await.unwrap();

        let jobs = scheduler.list_scheduled_jobs().await;
        let meditation_jobs: Vec<_> = jobs
            .iter()
            .filter(|j| j.id == MEDITATION_SCHEDULE_ID)
            .collect();
        assert_eq!(meditation_jobs.len(), 1, "exactly one Meditation job");
        assert_eq!(meditation_jobs[0].cron, MEDITATION_CRON);
    }

    #[test]
    fn cron_is_six_field_3am() {
        assert_eq!(MEDITATION_CRON.split_whitespace().count(), 6);
        // sec=0 min=0 hour=3
        let parts: Vec<&str> = MEDITATION_CRON.split_whitespace().collect();
        assert_eq!((parts[0], parts[1], parts[2]), ("0", "0", "3"));
    }
}
