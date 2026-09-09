//! One event a chat's cached ROW invalidates from — the binding and the
//! classification, for a row rewritten by any process.
//!
//! # The problem
//!
//! The composer states the model a chat actually runs on, and that is the chat's
//! own session row: `restore_provider_from_session` binds `provider_name` +
//! `model_config` whenever either is set. The renderer's copy of that row is a
//! CACHE — read once from `/agent/resume` and thereafter only patched — so a
//! window is right about a chat only while it is the one changing it.
//!
//! #196 closed the two in-window cases (a switch announces itself; a turn
//! re-reads the row when it ends) and PR 1 of this pair closed turn START (the
//! reply stream states the binding and the ratcheted tier in its own first
//! frames). What is left is a row rewritten by something that is not this
//! window's turn loop:
//!
//! * another Biorouter window — covered in the renderer by a `BroadcastChannel`,
//!   because both windows share one daemon and one announcement;
//! * **another PROCESS** — `biorouter session --resume <id> --provider …` writes
//!   `provider_name` and `model_config_json` straight into SQLite through
//!   `bind_provider_if_allowed`, a schedule run may raise `privacy_tier`, and a
//!   second daemon over the same store can do either. No renderer-side channel
//!   can see any of that.
//!
//! This module is the daemon's answer to the second one, and it is deliberately
//! the same shape as [`crate::catalog`]: a revision, a bounded ring, a
//! `Notify`, and one long poll that parks until the number moves.
//!
//! # It watches the COLUMNS, not the call sites — and not the file
//!
//! Two designs were considered and both are wrong here.
//!
//! **Publishing from every write site.** `provider_name`/`model_config_json`
//! funnel through `SessionStorage::bind_provider_if_allowed` and `privacy_tier`
//! through `raise_privacy`, but between them they have well over a dozen callers
//! plus three deliberate bypasses (`subagent_tool`, derived sessions, the
//! composite-config update). Each is a place the feed can silently lose a write,
//! and — decisively — **none of them exists in the CLI's process**, which is the
//! case this module was written for. A publisher inside this daemon cannot
//! observe a write another daemon made.
//!
//! **Stamping the database file.** `sessions.db` is SQLite in WAL mode and its
//! mtime moves on every message insert and every token-accounting update, so a
//! file stamp fires continuously through any turn in any window.
//!
//! ⚠ **`updated_at` has the same defect, one layer up, and that is not
//! obvious.** `insert_message` and the token-usage update both stamp
//! `updated_at = datetime('now')` (`session_manager.rs`), so a diff that
//! included it would fire many times per turn — the file-mtime problem wearing a
//! column's clothes. The four columns compared here are the ones a reader of the
//! binding or the classification actually renders; `updated_at` is neither
//! compared nor carried.
//!
//! # The revision is the contract, not the payload
//!
//! Identical to the catalogue's, and for the identical reason: a consumer that
//! believes a change's fields and never refetches drifts the first time two
//! changes race. Every consumer here re-reads the row it was told about.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tracing::debug;

/// How many changes the ring holds before the oldest is dropped and a client
/// that fell further behind is told to refetch.
///
/// Smaller than the catalogue's 64 on purpose: a change here names ONE chat, a
/// client watches the handful it has open, and anything past a few is already a
/// client that has been away long enough to want a fresh read anyway.
const BUFFER: usize = 32;

/// What changed about one chat's row.
///
/// ⚠ **Advisory, exactly like the frame in `/reply`.** It is not a session
/// payload and no gate consults it: `privacy_tier` here is a REPORT of a
/// classification the daemon has already committed, and a consumer that wants
/// to act on it re-reads the row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SessionMetaChanged {
    /// The revision this change was stamped with.
    pub revision: u64,
    /// The chat whose row moved.
    pub session_id: String,
    /// `provider_name` as the row now holds it.
    pub provider_name: Option<String>,
    /// `model_config_json`'s `model_name`, as the row now holds it.
    pub model_name: Option<String>,
    /// The row's classification, as stored.
    pub privacy_tier: Option<String>,
    /// The row's classification provenance, as stored.
    pub privacy_reason: Option<String>,
}

/// Everything that happened after a client's `since`.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SessionMetaDelta {
    /// The revision the caller should send next time.
    pub revision: u64,
    /// The changes after `since`, oldest first.
    pub changes: Vec<SessionMetaChanged>,
    /// The caller fell further behind than the ring holds, so `changes` is a
    /// PARTIAL history. Applying it and believing yourself current is the stale
    /// -row bug this feed exists to end, one layer down. Refetch instead.
    #[serde(default)]
    pub truncated: bool,
}

/// The four fields this feed compares, for one chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetaRow {
    pub session_id: String,
    pub provider_name: Option<String>,
    pub model_name: Option<String>,
    pub privacy_tier: Option<String>,
    pub privacy_reason: Option<String>,
}

#[derive(Default)]
struct Buffer {
    changes: Vec<SessionMetaChanged>,
    /// The revision of the oldest change still held.
    oldest: u64,
}

/// The process-global session-row event stream.
pub struct SessionMetaEvents {
    revision: AtomicU64,
    buffer: Mutex<Buffer>,
    notify: Notify,
    /// Each watched chat's row as this process last saw it.
    ///
    /// ⚠ **A chat enters this map SILENTLY.** The first observation of an id
    /// adopts without publishing, exactly as `CatalogEvents::detect_external_change`
    /// does at startup: a client that has just opened a chat has not changed it,
    /// and reporting the whole row as new would make every client refetch on
    /// connect for nothing.
    last: Mutex<HashMap<String, SessionMetaRow>>,
}

impl Default for SessionMetaEvents {
    fn default() -> Self {
        Self {
            revision: AtomicU64::new(0),
            buffer: Mutex::new(Buffer::default()),
            notify: Notify::new(),
            last: Mutex::new(HashMap::new()),
        }
    }
}

impl SessionMetaEvents {
    pub fn global() -> &'static Arc<Self> {
        static INSTANCE: once_cell::sync::Lazy<Arc<SessionMetaEvents>> =
            once_cell::sync::Lazy::new(|| Arc::new(SessionMetaEvents::default()));
        &INSTANCE
    }

    fn buffer(&self) -> std::sync::MutexGuard<'_, Buffer> {
        self.buffer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    /// Record one changed row and wake every parked poll.
    ///
    /// Returns the revision it was stamped with.
    pub fn publish(&self, row: SessionMetaRow) -> u64 {
        let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let change = SessionMetaChanged {
            revision,
            session_id: row.session_id,
            provider_name: row.provider_name,
            model_name: row.model_name,
            privacy_tier: row.privacy_tier,
            privacy_reason: row.privacy_reason,
        };
        debug!(
            "session meta revision {revision}: {} is now {:?} / {:?} ({:?})",
            change.session_id, change.provider_name, change.model_name, change.privacy_tier
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

    /// Everything that happened after `since`.
    pub fn since(&self, since: u64) -> SessionMetaDelta {
        let revision = self.revision();
        let buffer = self.buffer();
        // A client at the current revision is up to date whatever the ring
        // holds — including the case where nothing has ever been published.
        if since >= revision {
            return SessionMetaDelta {
                revision,
                changes: Vec::new(),
                truncated: false,
            };
        }
        let truncated = buffer.oldest > since + 1 && buffer.oldest > 1;
        SessionMetaDelta {
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

    /// Compare freshly-read rows against what this process last saw, adopt them,
    /// and publish the ones that moved.
    ///
    /// Returns how many changes were published. An id seen for the first time is
    /// adopted silently — see [`Self::last`].
    ///
    /// ⚠ It reports a row this daemon changed itself as readily as one another
    /// process changed, and that is correct rather than sloppy. There is exactly
    /// ONE detector, so a change is reported once; the window that made it
    /// re-reads a row that already matches and its snapshot comes back
    /// identical, costing no render. The alternative — publishing from the write
    /// sites *and* diffing — is what would need the catalogue's
    /// see-it-once bookkeeping, and it buys latency at the price of a feed that
    /// can be bypassed.
    pub fn observe(&self, rows: Vec<SessionMetaRow>) -> usize {
        let mut moved = Vec::new();
        {
            let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            for row in rows {
                match last.get(&row.session_id) {
                    Some(before) if before == &row => {}
                    Some(_) => {
                        last.insert(row.session_id.clone(), row.clone());
                        moved.push(row);
                    }
                    None => {
                        last.insert(row.session_id.clone(), row);
                    }
                }
            }
        }
        let count = moved.len();
        for row in moved {
            self.publish(row);
        }
        count
    }

    /// Stop tracking chats nobody is watching any more, so a long-lived daemon's
    /// map does not grow with every chat ever opened.
    pub fn retain_watched(&self, watched: &[String]) {
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        if last.len() <= watched.len() {
            return;
        }
        last.retain(|id, _| watched.iter().any(|w| w == id));
    }

    /// Park until the revision moves past `since`, or `timeout` elapses.
    ///
    /// A timeout is not an error: the caller gets the current revision and polls
    /// again, which is also how it survives a daemon restart — the revision
    /// resets to 0, the client sees a LOWER number than it holds, and refetches.
    pub async fn wait_for_change(&self, since: u64, timeout: Duration) -> SessionMetaDelta {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::SessionClassification;
    use crate::session::session_manager::SessionType;
    use crate::session::SessionManager;
    use tempfile::TempDir;

    fn row(id: &str, provider: &str, model: &str, tier: &str) -> SessionMetaRow {
        SessionMetaRow {
            session_id: id.to_string(),
            provider_name: Some(provider.to_string()),
            model_name: Some(model.to_string()),
            privacy_tier: Some(tier.to_string()),
            privacy_reason: None,
        }
    }

    fn events() -> SessionMetaEvents {
        SessionMetaEvents::default()
    }

    #[test]
    fn a_chat_seen_for_the_first_time_is_adopted_without_an_event() {
        // A client opening a chat has not changed it. Reporting the whole row as
        // new would make every window refetch on connect, forever.
        let e = events();
        assert_eq!(
            e.observe(vec![row("s1", "versa_azure", "gpt-5.5", "public")]),
            0
        );
        assert_eq!(e.revision(), 0);
    }

    #[test]
    fn a_row_that_moved_is_published_once() {
        let e = events();
        e.observe(vec![row("s1", "versa_azure", "gpt-5.5", "public")]);
        assert_eq!(
            e.observe(vec![row("s1", "codex", "gpt-6-astra", "public")]),
            1
        );
        // Re-observing the SAME values is not a second event: the feed reports
        // transitions, and a poll that finds nothing new must park rather than
        // wake every window every interval.
        assert_eq!(
            e.observe(vec![row("s1", "codex", "gpt-6-astra", "public")]),
            0
        );
        assert_eq!(e.revision(), 1);

        let delta = e.since(0);
        assert_eq!(delta.changes.len(), 1);
        assert_eq!(delta.changes[0].session_id, "s1");
        assert_eq!(delta.changes[0].provider_name.as_deref(), Some("codex"));
        assert_eq!(delta.changes[0].model_name.as_deref(), Some("gpt-6-astra"));
    }

    #[test]
    fn a_ratchet_alone_is_a_change() {
        // The classification moves without the binding moving at all — a public
        // chat whose first turn touched a private model. It is the field #196
        // measured as the one that lagged, so a diff that only watched the
        // binding would miss precisely the case this exists for.
        let e = events();
        e.observe(vec![row("s1", "versa_azure", "gpt-5.5", "public")]);
        let mut ratcheted = row("s1", "versa_azure", "gpt-5.5", "private");
        ratcheted.privacy_reason = Some("turn:versa_azure".to_string());
        assert_eq!(e.observe(vec![ratcheted]), 1);
        assert_eq!(
            e.since(0).changes[0].privacy_tier.as_deref(),
            Some("private")
        );
        assert_eq!(
            e.since(0).changes[0].privacy_reason.as_deref(),
            Some("turn:versa_azure")
        );
    }

    #[test]
    fn one_chat_moving_does_not_report_another() {
        let e = events();
        e.observe(vec![
            row("s1", "versa_azure", "gpt-5.5", "public"),
            row("s2", "codex", "gpt-6-astra", "public"),
        ]);
        assert_eq!(
            e.observe(vec![
                row("s1", "claude_code", "claude-opus-5", "public"),
                row("s2", "codex", "gpt-6-astra", "public"),
            ]),
            1
        );
        let delta = e.since(0);
        assert_eq!(delta.changes.len(), 1);
        assert_eq!(delta.changes[0].session_id, "s1");
    }

    #[test]
    fn a_client_at_the_current_revision_is_told_nothing() {
        let e = events();
        e.observe(vec![row("s1", "versa_azure", "gpt-5.5", "public")]);
        e.observe(vec![row("s1", "codex", "gpt-6-astra", "public")]);
        let delta = e.since(e.revision());
        assert!(delta.changes.is_empty());
        assert!(!delta.truncated);
    }

    #[test]
    fn falling_further_behind_than_the_ring_is_reported_as_truncated() {
        // Not advisory. A partial history applied as if complete is the stale-row
        // bug one layer down.
        let e = events();
        e.observe(vec![row("s1", "versa_azure", "gpt-5.5", "public")]);
        for i in 0..(BUFFER + 5) {
            e.observe(vec![row("s1", &format!("p{i}"), "m", "public")]);
        }
        assert!(
            e.since(1).truncated,
            "a client at revision 1 missed the start"
        );
        assert!(!e.since(e.revision() - 1).truncated);
    }

    #[test]
    fn a_chat_nobody_watches_stops_being_tracked() {
        let e = events();
        e.observe(vec![
            row("s1", "versa_azure", "gpt-5.5", "public"),
            row("s2", "codex", "gpt-6-astra", "public"),
        ]);
        e.retain_watched(&["s1".to_string()]);
        // s2 is a stranger again, so re-observing it adopts silently rather than
        // announcing a change nobody asked about.
        assert_eq!(e.observe(vec![row("s2", "ollama", "qwen3.6", "public")]), 0);
        // s1 is still tracked.
        assert_eq!(e.observe(vec![row("s1", "ollama", "qwen3.6", "public")]), 1);
    }

    #[tokio::test]
    async fn a_parked_poll_wakes_on_the_change_it_was_waiting_for() {
        let e = Arc::new(events());
        e.observe(vec![row("s1", "versa_azure", "gpt-5.5", "public")]);
        let waiter = Arc::clone(&e);
        let parked = tokio::spawn(async move {
            waiter
                .wait_for_change(0, Duration::from_secs(5))
                .await
                .changes
                .len()
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        e.observe(vec![row("s1", "codex", "gpt-6-astra", "public")]);
        assert_eq!(parked.await.unwrap(), 1);
    }

    /// The narrow read behind `GET /sessions/changes`, against a real store.
    ///
    /// It is the SQL that matters here — `model_name` comes out of
    /// `json_extract(model_config_json, '$.model_name')`, which no unit test of
    /// the event ring can exercise, and the whole feed is silent if that returns
    /// `NULL` for a bound row.
    #[tokio::test]
    async fn the_change_feed_reads_four_columns_and_skips_ids_with_no_row() {
        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                std::path::PathBuf::from("."),
                "meta".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();

        // Bind through the production statement, so the fixture cannot produce
        // a row the bind path would have refused.
        let model_config_json =
            r#"{"model_name":"gpt-5.5-2026-04-24","toolshim":false,"context_limit":1050000}"#;
        sm.storage()
            .bind_provider_if_allowed(&session.id, "versa_azure", model_config_json, true)
            .await
            .unwrap();
        sm.update(&session.id)
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();

        let rows = sm
            .storage()
            .session_meta_rows(&[session.id.clone(), "no-such-chat".to_string()])
            .await
            .unwrap();

        // A missing id is absent, not an empty row: the feed reports changes to
        // rows, and a deleted chat is not a changed one.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, session.id);
        assert_eq!(rows[0].provider_name.as_deref(), Some("versa_azure"));
        assert_eq!(rows[0].model_name.as_deref(), Some("gpt-5.5-2026-04-24"));
        assert_eq!(rows[0].privacy_tier.as_deref(), Some("private"));
        assert_eq!(rows[0].privacy_reason.as_deref(), Some("turn:versa_azure"));

        // And an empty request costs no statement at all — the idle-app case.
        assert!(sm
            .storage()
            .session_meta_rows(&[])
            .await
            .unwrap()
            .is_empty());
    }

    /// The case the endpoint exists for, end to end through the detector: a row
    /// rewritten by something that is not this daemon's turn loop.
    ///
    /// ⚠ The FIRST observation is silent. A client opening a chat has not
    /// changed it, and announcing its whole row would wake every window on
    /// connect — the defect `CatalogEvents::detect_external_change` avoids by
    /// adopting silently at startup.
    #[tokio::test]
    async fn a_row_rewritten_out_of_band_is_reported_exactly_once() {
        use crate::session_meta::SessionMetaEvents;

        let temp_dir = TempDir::new().unwrap();
        let sm = SessionManager::new(temp_dir.path().to_path_buf());
        let session = sm
            .create_session(
                std::path::PathBuf::from("."),
                "meta-feed".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        sm.storage()
            .bind_provider_if_allowed(
                &session.id,
                "versa_azure",
                r#"{"model_name":"gpt-5.5-2026-04-24","toolshim":false}"#,
                true,
            )
            .await
            .unwrap();

        // A private instance, not the process-global one: two tests sharing the
        // global ring would report each other's chats.
        let events = SessionMetaEvents::default();
        let ids = vec![session.id.clone()];
        let read = || async { sm.storage().session_meta_rows(&ids).await.unwrap() };

        assert_eq!(events.observe(read().await), 0, "first sight is silent");
        assert_eq!(events.observe(read().await), 0, "and nothing has moved");

        // Exactly what `biorouter session --resume <id> --provider …` does from
        // another process: the same statement, no daemon involved.
        sm.storage()
            .bind_provider_if_allowed(
                &session.id,
                "versa_azure",
                r#"{"model_name":"gpt-5.2-2025-12-11","toolshim":false}"#,
                true,
            )
            .await
            .unwrap();

        assert_eq!(events.observe(read().await), 1);
        assert_eq!(
            events.observe(read().await),
            0,
            "reported once, not forever"
        );

        let delta = events.since(0);
        assert_eq!(delta.changes.len(), 1);
        assert_eq!(delta.changes[0].session_id, session.id);
        assert_eq!(
            delta.changes[0].model_name.as_deref(),
            Some("gpt-5.2-2025-12-11")
        );
    }

    #[tokio::test]
    async fn a_poll_that_times_out_is_not_an_error() {
        let e = events();
        let delta = e.wait_for_change(0, Duration::from_millis(20)).await;
        assert!(delta.changes.is_empty());
        assert_eq!(delta.revision, 0);
    }
}
