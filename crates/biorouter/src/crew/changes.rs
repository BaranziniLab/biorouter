//! Crew's change feed: how an open Crew view learns that something it shows changed somewhere
//! else (chat parity, WI-15).
//!
//! The Crew view reads this computer's own Crew state (saved connections, setup jobs, grants,
//! read consents, transfers, the Crew task model) when it opens and after its own actions. A
//! change made anywhere else used to stay invisible until a reload: a chat's `crew__change`, a
//! second window, `biorouter crew` in a terminal. Three doors announce here: the `crew` extension
//! after a change it ran, the daemon's Crew routes after a change they made, and every setup job
//! when its state moves ([`super::setup`]). `GET /crew/changes` serves the feed to the renderer,
//! which re-reads what it shows.
//!
//! ⚠ **IDs only, never content.** A change says what kind of change it was and which
//! connection, channel, chat, task or setup job it touched, by ID. Nothing a person wrote, and
//! no name, ever goes in: every value is checked to look like an ID ([`id_like`]) and dropped
//! otherwise, so a field that happened to carry a name cannot carry it here. What changed is
//! read again through the routes that already decide who may read it.
//!
//! ⚠ **The revision is the contract, not the list.** A reader that applies `changes` and never
//! re-reads drifts the first time it falls behind more than [`KEPT`] changes. `truncated` (also
//! answered after a daemon restart, when the revision went back) means: re-read everything.

use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::watch;

/// How many changes the feed keeps. A reader further behind is told `truncated`.
pub const KEPT: usize = 256;
/// The longest an ID may be. Every ID Crew makes is far shorter.
const MAX_ID_LEN: usize = 128;
/// The longest a kind may be.
const MAX_KIND_LEN: usize = 64;
/// The kind a change gets when the one it was given does not look like a kind.
const UNNAMED_KIND: &str = "change";

/// One change, as `GET /crew/changes` lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CrewChange {
    /// The feed's revision this change raised it to.
    pub revision: u64,
    /// What changed: a `crew__change` action (`channels_create`, `grants_revoke`, `rejoin`), a
    /// setup job's new state (`setup_running`, `setup_needs_sign_in`, `setup_finished`,
    /// `setup_failed`, `setup_cancelled`), or `route` for any other change a Crew route made.
    pub kind: String,
    /// The saved connection it touched, when one is known.
    pub connection_id: Option<String>,
    /// The channel it touched, when one is known.
    pub channel_id: Option<String>,
    /// The chat it touched (a grant's or a read consent's chat, a task's own chat).
    pub session_id: Option<String>,
    /// The task it touched.
    pub run_id: Option<String>,
    /// The setup job it touched.
    pub job_id: Option<String>,
}

/// What `GET /crew/changes` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CrewChangesDelta {
    /// The feed's revision now. Ask again with it as `since`.
    pub revision: u64,
    /// The changes after `since`, oldest first.
    pub changes: Vec<CrewChange>,
    /// `changes` is not every change after `since`: the reader fell further behind than the
    /// feed keeps, or the daemon restarted. Re-read everything.
    pub truncated: bool,
}

/// The IDs a change touched. Each is kept only when it looks like an ID ([`id_like`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeIds {
    pub connection_id: Option<String>,
    pub channel_id: Option<String>,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub job_id: Option<String>,
}

impl ChangeIds {
    /// The IDs a `crew__change` answer names: its own `connection_id`, `channel_id`,
    /// `session_id`, `run_id` and `job_id`, and for the connection, a saved connection's `id`
    /// (`connection`) or a setup job's (`outcome.connection_id`) when the answer names none at
    /// its top.
    pub fn from_answer(answer: &Value) -> Self {
        let id = |value: &Value| {
            value
                .as_str()
                .filter(|text| id_like(text))
                .map(str::to_owned)
        };
        let top = |key: &str| answer.get(key).and_then(id);
        Self {
            connection_id: top("connection_id")
                .or_else(|| answer.pointer("/connection/id").and_then(id))
                .or_else(|| answer.pointer("/outcome/connection_id").and_then(id)),
            channel_id: top("channel_id"),
            session_id: top("session_id"),
            run_id: top("run_id"),
            job_id: top("job_id"),
        }
    }

    /// The same IDs, each dropped when it does not look like one.
    fn checked(self) -> Self {
        let keep = |value: Option<String>| value.filter(|text| id_like(text));
        Self {
            connection_id: keep(self.connection_id),
            channel_id: keep(self.channel_id),
            session_id: keep(self.session_id),
            run_id: keep(self.run_id),
            job_id: keep(self.job_id),
        }
    }
}

/// Whether `text` looks like an ID Crew makes: 1 to 128 ASCII letters, digits, `-`, `_`, `.` or
/// `:`. A name with a space, a sentence or anything longer is not one.
pub fn id_like(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_ID_LEN
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
}

/// Whether `text` looks like a kind: 1 to 64 lowercase ASCII letters, digits or `_`.
fn kind_like(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_KIND_LEN
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// The feed: a revision anyone can wait on, and the last [`KEPT`] changes.
pub struct CrewChanges {
    revision: watch::Sender<u64>,
    kept: Mutex<VecDeque<CrewChange>>,
}

impl Default for CrewChanges {
    fn default() -> Self {
        Self::new()
    }
}

impl CrewChanges {
    /// An empty feed at revision 0. The daemon uses [`feed`]; a test makes its own.
    pub fn new() -> Self {
        Self {
            revision: watch::Sender::new(0),
            kept: Mutex::new(VecDeque::with_capacity(KEPT)),
        }
    }

    fn kept(&self) -> MutexGuard<'_, VecDeque<CrewChange>> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The revision now.
    pub fn revision(&self) -> u64 {
        *self.revision.borrow()
    }

    /// Record a change and wake every reader waiting on the feed. Answers its revision.
    pub fn publish(&self, kind: &str, ids: ChangeIds) -> u64 {
        let ids = ids.checked();
        let kind = if kind_like(kind) { kind } else { UNNAMED_KIND };
        // The revision moves under the same lock the list is kept under, so a reader never sees
        // a revision whose change is not listed yet.
        let mut kept = self.kept();
        let revision = self.revision().saturating_add(1);
        kept.push_back(CrewChange {
            revision,
            kind: kind.to_owned(),
            connection_id: ids.connection_id,
            channel_id: ids.channel_id,
            session_id: ids.session_id,
            run_id: ids.run_id,
            job_id: ids.job_id,
        });
        while kept.len() > KEPT {
            kept.pop_front();
        }
        self.revision.send_replace(revision);
        revision
    }

    /// Where the feed stands, with no changes: how a new reader learns where to start.
    pub fn current(&self) -> CrewChangesDelta {
        CrewChangesDelta {
            revision: self.revision(),
            changes: Vec::new(),
            truncated: false,
        }
    }

    /// Every change after `since`. A `since` past the revision means the daemon restarted
    /// since the reader last asked: `truncated`, so it re-reads everything.
    pub fn since(&self, since: u64) -> CrewChangesDelta {
        let kept = self.kept();
        let revision = self.revision();
        if since >= revision {
            return CrewChangesDelta {
                revision,
                changes: Vec::new(),
                truncated: since > revision,
            };
        }
        let oldest = kept.front().map_or(revision, |change| change.revision);
        CrewChangesDelta {
            revision,
            changes: kept
                .iter()
                .filter(|change| change.revision > since)
                .cloned()
                .collect(),
            truncated: oldest > since.saturating_add(1),
        }
    }

    /// Wait until the revision is no longer `since`, or `timeout` passes, then answer what
    /// changed after `since` ([`Self::since`]). A timeout is no error: the answer is empty and
    /// the reader asks again. A change published while this waits is never missed: the watch
    /// channel is read before it is waited on.
    pub async fn wait(&self, since: u64, timeout: Duration) -> CrewChangesDelta {
        let mut seen = self.revision.subscribe();
        let _ = tokio::time::timeout(timeout, seen.wait_for(|revision| *revision != since)).await;
        self.since(since)
    }
}

/// The daemon's one feed.
pub fn feed() -> &'static CrewChanges {
    static FEED: LazyLock<CrewChanges> = LazyLock::new(CrewChanges::new);
    &FEED
}

/// Announce a change on the daemon's feed. Answers its revision.
pub fn publish(kind: &str, ids: ChangeIds) -> u64 {
    feed().publish(kind, ids)
}

/// Announce the `crew__change` action `action` that ran and answered `answer`, with the IDs the
/// answer names ([`ChangeIds::from_answer`]). Answers its revision.
pub fn publish_answer(action: &str, answer: &Value) -> u64 {
    publish(action, ChangeIds::from_answer(answer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn on(connection: &str) -> ChangeIds {
        ChangeIds {
            connection_id: Some(connection.into()),
            ..ChangeIds::default()
        }
    }

    #[test]
    fn every_change_raises_the_revision_by_one() {
        let feed = CrewChanges::new();
        assert_eq!(feed.revision(), 0);
        let first = feed.publish("channels_create", on("c-1"));
        let second = feed.publish("grants_revoke", on("c-2"));
        assert_eq!((first, second), (1, 2));
        let delta = feed.since(0);
        assert_eq!(delta.revision, 2);
        assert!(!delta.truncated);
        let kinds: Vec<_> = delta.changes.iter().map(|c| c.kind.as_str()).collect();
        assert_eq!(kinds, ["channels_create", "grants_revoke"]);
        assert_eq!(
            delta.changes.iter().map(|c| c.revision).collect::<Vec<_>>(),
            [1, 2]
        );
        // Caught up: nothing, and not truncated.
        assert_eq!(feed.since(2), feed.current());
    }

    #[test]
    fn a_reader_further_behind_than_the_feed_keeps_is_told_to_re_read() {
        let feed = CrewChanges::new();
        for _ in 0..KEPT + 5 {
            feed.publish("route", ChangeIds::default());
        }
        let behind = feed.since(0);
        assert!(behind.truncated, "five changes were dropped");
        assert_eq!(behind.changes.len(), KEPT);
        assert_eq!(behind.changes[0].revision, 6);
        // The oldest change still kept follows `since` directly: nothing was missed.
        let just = feed.since(5);
        assert!(!just.truncated);
        assert_eq!(just.changes.len(), KEPT);
    }

    #[test]
    fn a_reader_ahead_of_the_feed_learns_the_daemon_restarted() {
        let feed = CrewChanges::new();
        feed.publish("route", ChangeIds::default());
        let delta = feed.since(40);
        assert_eq!(delta.revision, 1);
        assert!(delta.changes.is_empty());
        assert!(delta.truncated);
    }

    #[tokio::test]
    async fn a_wait_answers_at_the_timeout_with_nothing() {
        let feed = CrewChanges::new();
        let started = std::time::Instant::now();
        let delta = feed.wait(0, Duration::from_millis(30)).await;
        assert!(started.elapsed() >= Duration::from_millis(25));
        assert_eq!(delta, feed.current());
    }

    #[tokio::test]
    async fn a_wait_answers_as_soon_as_a_change_lands() {
        let feed = Arc::new(CrewChanges::new());
        let waiting = {
            let feed = Arc::clone(&feed);
            tokio::spawn(async move { feed.wait(0, Duration::from_secs(20)).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        feed.publish("rejoin", on("c-1"));
        let delta = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the wait ended when the change landed")
            .unwrap();
        assert_eq!(delta.revision, 1);
        assert_eq!(delta.changes[0].kind, "rejoin");
        assert_eq!(delta.changes[0].connection_id.as_deref(), Some("c-1"));
    }

    #[tokio::test]
    async fn a_wait_behind_the_feed_answers_at_once() {
        let feed = CrewChanges::new();
        feed.publish("route", ChangeIds::default());
        let delta = tokio::time::timeout(
            Duration::from_secs(1),
            feed.wait(0, Duration::from_secs(20)),
        )
        .await
        .expect("a reader behind is answered without waiting");
        assert_eq!(delta.changes.len(), 1);
    }

    #[test]
    fn an_answer_gives_its_ids_and_nothing_else() {
        let answer = json!({
            "connection_id": "6f1c2d3e-0000-4000-8000-000000000001",
            "channel_id": "ch_42",
            "session_id": "20261106_7",
            "run_id": "a1b2c3",
            "job_id": "job-9",
            "name": "Lab notes",
            "body": "the secret plan",
        });
        assert_eq!(
            ChangeIds::from_answer(&answer),
            ChangeIds {
                connection_id: Some("6f1c2d3e-0000-4000-8000-000000000001".into()),
                channel_id: Some("ch_42".into()),
                session_id: Some("20261106_7".into()),
                run_id: Some("a1b2c3".into()),
                job_id: Some("job-9".into()),
            }
        );
        // A saved connection's view, or a setup job's outcome, names the connection.
        let saved = json!({"connection": {"id": "c-7", "name": "Lab"}});
        assert_eq!(
            ChangeIds::from_answer(&saved).connection_id.as_deref(),
            Some("c-7")
        );
        let job = json!({"job_id": "j-1", "outcome": {"connection_id": "c-8"}});
        assert_eq!(
            ChangeIds::from_answer(&job).connection_id.as_deref(),
            Some("c-8")
        );
    }

    #[test]
    fn a_value_that_is_not_an_id_never_reaches_the_feed() {
        let feed = CrewChanges::new();
        feed.publish(
            "Channels Create!",
            ChangeIds {
                connection_id: Some("Lab notes".into()),
                channel_id: Some("#methods".into()),
                session_id: Some(String::new()),
                run_id: Some("x".repeat(MAX_ID_LEN + 1)),
                job_id: Some("job-1".into()),
            },
        );
        let change = &feed.since(0).changes[0];
        assert_eq!(change.kind, UNNAMED_KIND);
        assert_eq!(change.connection_id, None);
        assert_eq!(change.channel_id, None);
        assert_eq!(change.session_id, None);
        assert_eq!(change.run_id, None);
        assert_eq!(change.job_id.as_deref(), Some("job-1"));
        let answered = json!({"connection_id": "Ann's lab", "channel_id": 42});
        assert_eq!(ChangeIds::from_answer(&answered), ChangeIds::default());
    }

    #[test]
    fn a_change_serializes_to_its_ids_alone() {
        let feed = CrewChanges::new();
        feed.publish("send", on("c-1"));
        let value = serde_json::to_value(feed.since(0)).unwrap();
        let mut keys: Vec<&str> = value["changes"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "channel_id",
                "connection_id",
                "job_id",
                "kind",
                "revision",
                "run_id",
                "session_id"
            ]
        );
    }
}
