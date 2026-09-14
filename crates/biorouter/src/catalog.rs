//! One event every extension and skill inventory invalidates from (#112).
//!
//! # The problem
//!
//! An install can succeed on disk while four separate inventories keep serving
//! stale answers: `ConfigContext.extensionsList` in the renderer (which reloads
//! only when its own key changes), the Settings list, the composer's extension
//! picker, and the running agent's `ExtensionManager`. Each was repaired
//! independently, and an install from outside the GUI — the CLI, an agent, a
//! deep link, a hand-edited `config.yaml` — repaired none of them. The reported
//! symptom was two extensions that installed correctly, showed as enabled in
//! `biorouter extension list`, and could not be attached to the chat that had
//! just asked for them; a new chat saw both, which is what proved the install
//! itself was fine.
//!
//! # The shape
//!
//! ```text
//!   set_extension / remove_extension / set_extension_enabled   (this process)
//!   config.yaml changed underneath us                          (any process)
//!                          │
//!                          ▼
//!              CatalogEvents::global().publish(..)   → revision += 1
//!                          │
//!         GET /catalog/changes?since=N  (long poll, parks until it moves)
//!                          │
//!        ConfigContext ──┬── Settings list
//!                        ├── composer picker
//!                        └── window `catalog:changed`  → non-React consumers
//! ```
//!
//! # The revision is the contract, not the payload
//!
//! A consumer that treats a `CatalogChanged`'s `config` as authoritative and
//! never refetches will drift the first time two changes race. A consumer that
//! compares revisions and refetches on a gap cannot. That is why the buffer is
//! bounded and reports [`CatalogDelta::truncated`] rather than silently
//! dropping: a client that missed more than the buffer holds is told to refetch,
//! not handed a partial history it would mistake for a complete one.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tracing::debug;
use utoipa::ToSchema;

use crate::agents::extension::ExtensionConfig;
use crate::config::extensions::{name_to_key, ExtensionEntry};

/// How many changes are remembered for a client that fell behind.
///
/// Small on purpose: the buffer exists so a client that blinked can catch up
/// without a full refetch, not so it can reconstruct history. Past this it is
/// told to refetch, which is always correct and never wrong by one event.
const BUFFER: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum CatalogChangeReason {
    Install,
    Uninstall,
    Update,
    Enable,
    Disable,
    /// The catalogue changed underneath this process — a CLI install, a deep
    /// link, a hand-edited `config.yaml`.
    Import,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum CatalogEntryChange {
    Added,
    Removed,
    Updated,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogExtensionChange {
    /// `name_to_key(name)` — the join every surface already uses, and the only
    /// identifier that survives a display-name change.
    pub key: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub change: CatalogEntryChange,
    /// The normalized config, exactly as written to `config.yaml`. Absent on
    /// removal.
    ///
    /// Carried so a consumer can repair its row without a refetch — which is
    /// the point, since the refetch is what was racing. It is still not
    /// authoritative: see the module header on revisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<ExtensionConfig>,
    pub enabled: bool,
    /// Skills that shipped inside this extension's bundle.
    #[serde(default)]
    pub bundled_skill_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSkillChange {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub change: CatalogEntryChange,
    /// The extension whose bundle carried it, when it came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_extension_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogChanged {
    /// Monotonic per-daemon-process counter. A consumer that has applied a
    /// revision >= this may drop the event; one that missed a gap refetches.
    pub revision: u64,
    pub reason: CatalogChangeReason,
    #[serde(default)]
    pub extensions: Vec<CatalogExtensionChange>,
    #[serde(default)]
    pub skills: Vec<CatalogSkillChange>,
    /// The session the change was made from, when it was made from one.
    ///
    /// ⚠ Present so a surface can offer "attach to this chat" — **not** a
    /// delivery scope. The change is machine-wide and every session's inventory
    /// is stale after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// What a client gets back from a poll.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDelta {
    /// The current revision. A client stores this and passes it as `since`.
    pub revision: u64,
    #[serde(default)]
    pub changes: Vec<CatalogChanged>,
    /// The client fell further behind than the buffer holds, so `changes` is
    /// not a complete history. **Refetch the inventory** rather than applying
    /// it.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Default)]
struct Buffer {
    changes: Vec<CatalogChanged>,
    /// The revision of the oldest change still held.
    oldest: u64,
}

/// The process-global catalogue event stream.
pub struct CatalogEvents {
    revision: AtomicU64,
    buffer: Mutex<Buffer>,
    notify: Notify,
    /// The extension map as this process last knew it.
    ///
    /// Every in-process publisher refreshes it, so [`Self::detect_external_change`]
    /// — the file watcher's eye — only ever reports changes this process did
    /// *not* make. Without it every write would be seen twice: once by the
    /// choke point that made it and once by the watcher a moment later.
    last: Mutex<Option<CatalogSnapshot>>,
}

impl Default for CatalogEvents {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            buffer: Mutex::new(Buffer::default()),
            notify: Notify::new(),
            last: Mutex::new(None),
        }
    }
}

fn skill_rows(skills: &[String], change: CatalogEntryChange) -> Vec<CatalogSkillChange> {
    skills
        .iter()
        .map(|name| CatalogSkillChange {
            id: name.clone(),
            name: Some(name.clone()),
            change,
            source_extension_key: None,
        })
        .collect()
}

impl CatalogEvents {
    pub fn global() -> &'static Arc<Self> {
        static INSTANCE: once_cell::sync::Lazy<Arc<CatalogEvents>> =
            once_cell::sync::Lazy::new(|| Arc::new(CatalogEvents::default()));
        &INSTANCE
    }

    fn buffer(&self) -> std::sync::MutexGuard<'_, Buffer> {
        self.buffer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    /// Record a change and wake every parked poll.
    ///
    /// Returns the revision it was stamped with. Callers that publish from a
    /// config write choke point do not care; a test does.
    pub fn publish(
        &self,
        reason: CatalogChangeReason,
        extensions: Vec<CatalogExtensionChange>,
        skills: Vec<CatalogSkillChange>,
        session_id: Option<String>,
    ) -> u64 {
        if extensions.is_empty() && skills.is_empty() {
            // A write that changed nothing is not an event. Publishing one
            // would wake every client to refetch an identical inventory — and
            // the file watcher below rewrites its snapshot on every touch.
            return self.revision();
        }
        let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let change = CatalogChanged {
            revision,
            reason,
            extensions,
            skills,
            session_id,
        };
        debug!(
            "catalog revision {revision}: {:?}, {} extension change(s)",
            change.reason,
            change.extensions.len()
        );
        {
            let mut buffer = self.buffer();
            buffer.changes.push(change);
            if buffer.changes.len() > BUFFER {
                let dropped = buffer.changes.remove(0);
                buffer.oldest = dropped.revision + 1;
            } else if buffer.oldest == 0 {
                buffer.oldest = revision;
            }
        }
        self.notify.notify_waiters();
        revision
    }

    /// Wake catalog consumers after a per-session attachment change is
    /// durable. The machine-wide extension and skill inventory may be
    /// unchanged, but chat-scoped pickers still need to refetch their enabled
    /// state. This deliberately bypasses [`Self::publish`]'s empty-change
    /// guard: the session id is the changed state.
    pub fn publish_session_refresh(&self, session_id: impl Into<String>) -> u64 {
        let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let change = CatalogChanged {
            revision,
            reason: CatalogChangeReason::Update,
            extensions: Vec::new(),
            skills: Vec::new(),
            session_id: Some(session_id.into()),
        };
        debug!("catalog revision {revision}: session attachment state changed");
        {
            let mut buffer = self.buffer();
            buffer.changes.push(change);
            if buffer.changes.len() > BUFFER {
                let dropped = buffer.changes.remove(0);
                buffer.oldest = dropped.revision + 1;
            } else if buffer.oldest == 0 {
                buffer.oldest = revision;
            }
        }
        self.notify.notify_waiters();
        revision
    }

    /// A skill package landed on disk: `skills` are its component skill names,
    /// and `replaced` says whether it overwrote an install of the same id.
    ///
    /// ⚠ **Every in-process install publishes, whichever door it came through.**
    /// The agent's `importSkillPackage` tool did and `POST
    /// /skills/packages/install` did not, so a skill installed from Settings in
    /// one window never reached another window's catalog: its Browse skills
    /// dialog kept offering the package, installed it a second time over the
    /// first, and announced the replacement as a fresh install. This is the one
    /// definition of that event, so the two doors cannot drift again.
    pub fn publish_skill_package_installed(
        &self,
        skills: &[String],
        replaced: bool,
        session_id: Option<String>,
    ) -> u64 {
        let (reason, change) = if replaced {
            (CatalogChangeReason::Update, CatalogEntryChange::Updated)
        } else {
            (CatalogChangeReason::Install, CatalogEntryChange::Added)
        };
        self.publish(reason, Vec::new(), skill_rows(skills, change), session_id)
    }

    /// A skill package left the disk. `skills` names what went with it — its
    /// components where the caller knows them, the package id where it does
    /// not. Consumers refetch on the revision either way; see the module header.
    pub fn publish_skill_package_removed(
        &self,
        skills: &[String],
        session_id: Option<String>,
    ) -> u64 {
        self.publish(
            CatalogChangeReason::Uninstall,
            Vec::new(),
            skill_rows(skills, CatalogEntryChange::Removed),
            session_id,
        )
    }

    /// Everything that happened after `since`.
    pub fn since(&self, since: u64) -> CatalogDelta {
        let revision = self.revision();
        let buffer = self.buffer();
        // A client at the current revision is up to date, whatever the buffer
        // holds — including the case where nothing has ever been published.
        if since >= revision {
            return CatalogDelta {
                revision,
                changes: Vec::new(),
                truncated: false,
            };
        }
        let truncated = buffer.oldest > since + 1 && buffer.oldest > 1;
        CatalogDelta {
            revision,
            changes: buffer
                .changes
                .iter()
                .filter(|c| c.revision > since)
                .cloned()
                .collect(),
            truncated,
        }
    }

    /// Record the extension map as it stands now, without publishing.
    ///
    /// Called after every in-process write, so the watcher has nothing left to
    /// find. Also called once at startup to establish the baseline.
    pub fn sync_snapshot(&self, entries: &[ExtensionEntry]) {
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) = Some(snapshot(entries));
    }

    /// Compare `entries` against the last known map and adopt it.
    ///
    /// Returns the rows that changed — empty when this process already knew.
    /// The first call after startup adopts silently: a daemon starting up has
    /// not "changed" anything, and reporting its whole catalogue as new would
    /// make every client refetch on connect for nothing.
    pub fn detect_external_change(
        &self,
        entries: &[ExtensionEntry],
    ) -> Vec<CatalogExtensionChange> {
        let now = snapshot(entries);
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let changes = match last.as_ref() {
            None => Vec::new(),
            Some(before) => diff(before, &now, entries),
        };
        *last = Some(now);
        changes
    }

    /// Park until the revision moves past `since`, or `timeout` elapses.
    ///
    /// The long-poll behind `GET /catalog/changes`. A timeout is not an error:
    /// the client gets the current revision and polls again, which is also how
    /// it survives a daemon restart (the revision resets to 0, the client sees
    /// a *lower* number than it holds, and refetches).
    pub async fn wait_for_change(&self, since: u64, timeout: Duration) -> CatalogDelta {
        if self.revision() > since {
            return self.since(since);
        }
        // Registered BEFORE the second read: `notify_waiters` only wakes waiters
        // already registered, so checking first and awaiting after would drop a
        // change published in between and park for the full timeout.
        let notified = self.notify.notified();
        tokio::pin!(notified);
        if self.revision() > since {
            return self.since(since);
        }
        let _ = tokio::time::timeout(timeout, notified).await;
        self.since(since)
    }
}

/// How often the daemon looks at `config.yaml` for a change it did not make.
///
/// A `stat` every two seconds, not a filesystem-notification API. The point is
/// to catch a `biorouter extension install` run in another terminal within the
/// time it takes the user to switch back to the app; two seconds does that, a
/// `stat` costs nothing, and it behaves identically on macOS, Windows, Linux,
/// and over the network mounts a shared config directory sometimes lives on —
/// which is more than can be said for the notification APIs.
const WATCH_INTERVAL: Duration = Duration::from_secs(2);

/// Watch `config.yaml` for changes made outside this process.
///
/// ⚠ **This is what makes a CLI install visible to a running app.** The GUI's
/// own writes go through the choke points above, and an agent's go through the
/// install transaction — but `biorouter extension install` in another terminal
/// writes the same file from a different process, and before this the desktop
/// had no way to learn about it short of a restart. That was the reported bug.
///
/// Spawned once by the daemon. Cheap enough to run unconditionally: it wakes,
/// stats one file, and goes back to sleep.
pub fn spawn_config_watcher() {
    tokio::spawn(async move {
        let path = crate::config::paths::Paths::config_dir().join("config.yaml");
        // Establish the baseline without publishing: a daemon that has just
        // started has not changed anything.
        CatalogEvents::global().sync_snapshot(&crate::config::extensions::get_all_extensions());
        let mut seen = file_stamp(&path);
        loop {
            tokio::time::sleep(WATCH_INTERVAL).await;
            let stamp = file_stamp(&path);
            if stamp == seen {
                continue;
            }
            seen = stamp;
            let entries = crate::config::extensions::get_all_extensions();
            let changes = CatalogEvents::global().detect_external_change(&entries);
            // A rewrite with identical contents — which `save_extensions_map`
            // produces on every write — moves the mtime and nothing else.
            CatalogEvents::global().publish(CatalogChangeReason::Import, changes, Vec::new(), None);
        }
    });
}

/// Size and mtime, as one comparable value. `None` when the file is absent,
/// which is itself a state worth noticing (a reset, a fresh install).
fn file_stamp(path: &std::path::Path) -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// A comparable snapshot of the extension map, for diffing across a write this
/// process did not make.
pub type CatalogSnapshot = BTreeMap<String, (bool, String)>;

/// Reduce the extension map to `key -> (enabled, fingerprint)`.
///
/// The fingerprint is the serialized config, so a changed command, argument or
/// env key registers as an `Updated` even though the name did not move.
pub fn snapshot(entries: &[ExtensionEntry]) -> CatalogSnapshot {
    entries
        .iter()
        .map(|entry| {
            let key = name_to_key(&entry.config.name());
            let fingerprint =
                serde_json::to_string(&entry.config).unwrap_or_else(|_| entry.config.name());
            (key, (entry.enabled, fingerprint))
        })
        .collect()
}

/// What changed between two snapshots, as catalogue rows.
///
/// `current` supplies the config and display name for rows that still exist; a
/// removed row carries neither, because there is nothing left to read them off.
pub fn diff(
    before: &CatalogSnapshot,
    after: &CatalogSnapshot,
    current: &[ExtensionEntry],
) -> Vec<CatalogExtensionChange> {
    let by_key: BTreeMap<String, &ExtensionEntry> = current
        .iter()
        .map(|e| (name_to_key(&e.config.name()), e))
        .collect();

    let row = |key: &str, change: CatalogEntryChange| -> CatalogExtensionChange {
        let entry = by_key.get(key);
        CatalogExtensionChange {
            key: key.to_string(),
            name: entry
                .map(|e| e.config.name())
                .unwrap_or_else(|| key.to_string()),
            display_name: None,
            change,
            config: entry.map(|e| e.config.clone()),
            enabled: entry.map(|e| e.enabled).unwrap_or(false),
            bundled_skill_ids: Vec::new(),
        }
    };

    let mut changes = Vec::new();
    for (key, (enabled, fingerprint)) in after {
        match before.get(key) {
            None => changes.push(row(key, CatalogEntryChange::Added)),
            Some((was_enabled, was_fingerprint)) => {
                if was_fingerprint != fingerprint {
                    changes.push(row(key, CatalogEntryChange::Updated));
                } else if was_enabled != enabled {
                    changes.push(row(
                        key,
                        if *enabled {
                            CatalogEntryChange::Enabled
                        } else {
                            CatalogEntryChange::Disabled
                        },
                    ));
                }
            }
        }
    }
    for key in before.keys() {
        if !after.contains_key(key) {
            changes.push(row(key, CatalogEntryChange::Removed));
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(rows: &[(&str, bool, &str)]) -> CatalogSnapshot {
        rows.iter()
            .map(|(k, e, f)| (k.to_string(), (*e, f.to_string())))
            .collect()
    }

    #[test]
    fn an_added_entry_is_an_addition_and_a_dropped_one_a_removal() {
        let before = snap(&[("spokeagent", true, "a")]);
        let after = snap(&[("spokeagent", true, "a"), ("bioroffice", true, "b")]);
        let changes = diff(&before, &after, &[]);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].key, "bioroffice");
        assert_eq!(changes[0].change, CatalogEntryChange::Added);

        let back = diff(&after, &before, &[]);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].change, CatalogEntryChange::Removed);
    }

    /// A toggle and a reconfiguration are different events, and a surface that
    /// treats them alike would rebuild a row for a checkbox.
    #[test]
    fn a_toggle_and_a_reconfiguration_are_told_apart() {
        let before = snap(&[("spokeagent", true, "a")]);
        assert_eq!(
            diff(&before, &snap(&[("spokeagent", false, "a")]), &[])[0].change,
            CatalogEntryChange::Disabled
        );
        assert_eq!(
            diff(&before, &snap(&[("spokeagent", true, "b")]), &[])[0].change,
            CatalogEntryChange::Updated
        );
    }

    #[test]
    fn an_unchanged_catalogue_produces_no_rows() {
        let s = snap(&[("spokeagent", true, "a")]);
        assert!(diff(&s, &s, &[]).is_empty());
    }

    fn row(key: &str) -> CatalogExtensionChange {
        CatalogExtensionChange {
            key: key.to_string(),
            name: key.to_string(),
            display_name: None,
            change: CatalogEntryChange::Added,
            config: None,
            enabled: true,
            bundled_skill_ids: Vec::new(),
        }
    }

    /// A write that changed nothing is not an event. The file watcher rewrites
    /// its snapshot on every touch, so without this a `config.yaml` rewritten
    /// with identical contents would wake every client.
    #[test]
    fn publishing_nothing_does_not_move_the_revision() {
        let events = CatalogEvents::default();
        let before = events.revision();
        assert_eq!(
            events.publish(CatalogChangeReason::Import, vec![], vec![], None),
            before
        );
        assert_eq!(events.revision(), before);
    }

    #[test]
    fn an_installed_skill_package_names_each_component_and_whether_it_replaced() {
        let events = CatalogEvents::default();
        let skills = vec!["primer-design".to_string(), "primer-blast".to_string()];
        events.publish_skill_package_installed(&skills, false, None);
        events.publish_skill_package_installed(&skills, true, Some("s".into()));
        events.publish_skill_package_removed(&skills[..1], None);

        let delta = events.since(0);
        let shape: Vec<_> = delta
            .changes
            .iter()
            .map(|c| {
                (
                    c.reason,
                    c.skills
                        .iter()
                        .map(|s| (s.id.as_str(), s.change))
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            vec![
                (
                    CatalogChangeReason::Install,
                    vec![
                        ("primer-design", CatalogEntryChange::Added),
                        ("primer-blast", CatalogEntryChange::Added)
                    ]
                ),
                (
                    CatalogChangeReason::Update,
                    vec![
                        ("primer-design", CatalogEntryChange::Updated),
                        ("primer-blast", CatalogEntryChange::Updated)
                    ]
                ),
                (
                    CatalogChangeReason::Uninstall,
                    vec![("primer-design", CatalogEntryChange::Removed)]
                ),
            ]
        );
    }

    #[test]
    fn a_session_refresh_moves_the_revision_without_inventory_rows() {
        let events = CatalogEvents::default();
        let revision = events.publish_session_refresh("session-42");
        assert_eq!(revision, 1);

        let delta = events.since(0);
        assert_eq!(delta.revision, 1);
        assert_eq!(delta.changes.len(), 1);
        let change = &delta.changes[0];
        assert!(change.extensions.is_empty());
        assert!(change.skills.is_empty());
        assert_eq!(change.session_id.as_deref(), Some("session-42"));
    }

    #[test]
    fn a_client_at_the_current_revision_is_told_nothing_happened() {
        let events = CatalogEvents::default();
        events.publish(
            CatalogChangeReason::Install,
            vec![row("spokeagent")],
            vec![],
            None,
        );
        let delta = events.since(events.revision());
        assert!(delta.changes.is_empty());
        assert!(!delta.truncated);
    }

    #[test]
    fn a_client_one_behind_gets_exactly_the_change_it_missed() {
        let events = CatalogEvents::default();
        let first = events.publish(
            CatalogChangeReason::Install,
            vec![row("spokeagent")],
            vec![],
            None,
        );
        events.publish(
            CatalogChangeReason::Install,
            vec![row("bioroffice")],
            vec![],
            None,
        );
        let delta = events.since(first);
        assert_eq!(delta.changes.len(), 1);
        assert_eq!(delta.changes[0].extensions[0].key, "bioroffice");
        assert!(!delta.truncated);
    }

    /// ⚠ The bounded buffer must SAY it dropped something. A client handed a
    /// partial history it mistakes for a complete one applies a few rows and
    /// believes it is current — which is the stale-inventory bug this whole
    /// module exists to end, reintroduced one layer down.
    #[test]
    fn falling_further_behind_than_the_buffer_is_reported_not_hidden() {
        let events = CatalogEvents::default();
        for i in 0..(BUFFER + 5) {
            events.publish(
                CatalogChangeReason::Install,
                vec![row(&format!("ext-{i}"))],
                vec![],
                None,
            );
        }
        let delta = events.since(0);
        assert!(
            delta.truncated,
            "a client that missed more than the buffer holds must be told to refetch"
        );
    }

    #[tokio::test]
    async fn a_poll_returns_immediately_when_it_is_already_behind() {
        let events = CatalogEvents::default();
        events.publish(
            CatalogChangeReason::Install,
            vec![row("spokeagent")],
            vec![],
            None,
        );
        let delta = events.wait_for_change(0, Duration::from_millis(50)).await;
        assert_eq!(delta.changes.len(), 1);
    }

    /// A timeout is not an error: the client gets the current revision and
    /// polls again.
    #[tokio::test]
    async fn a_poll_with_nothing_to_report_times_out_quietly() {
        let events = CatalogEvents::default();
        let delta = events.wait_for_change(0, Duration::from_millis(20)).await;
        assert!(delta.changes.is_empty());
        assert_eq!(delta.revision, 0);
    }

    /// The race the ordering in `wait_for_change` exists to close: a change
    /// published between the first read and the park must still wake it.
    #[tokio::test]
    async fn a_change_published_while_a_poll_is_arriving_is_not_missed() {
        let events = Arc::new(CatalogEvents::default());
        let writer = Arc::clone(&events);
        let poll = tokio::spawn({
            let events = Arc::clone(&events);
            async move { events.wait_for_change(0, Duration::from_secs(5)).await }
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        writer.publish(
            CatalogChangeReason::Install,
            vec![row("spokeagent")],
            vec![],
            None,
        );
        let delta = poll.await.expect("the poll task");
        assert_eq!(delta.changes.len(), 1);
    }
}
