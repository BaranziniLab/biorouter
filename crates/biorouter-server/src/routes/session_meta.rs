//! `GET /sessions/changes` — how a client learns a chat's ROW moved (handoff 04).
//!
//! One long poll, deliberately, and for the reasons `routes/catalog.rs` already
//! records for the extension catalogue: every reader of a session row already
//! refetches over HTTP, what was missing was the *signal*, and a parked GET
//! delivers that with no new transport and no ordering to keep in step with the
//! session stream.
//!
//! # What this covers that nothing else does
//!
//! * A per-chat model switch in **this** window announces itself in-renderer.
//! * A switch in a **second window** crosses on a `BroadcastChannel` — both
//!   windows are one renderer process family talking about one daemon.
//! * A turn states its own binding and classification in the reply stream.
//!
//! None of those can see `biorouter session --resume <id> --provider …`, which
//! is a different OS process writing SQLite directly, or a schedule run in
//! another daemon over the same store. That is what this endpoint is for, and it
//! is why the detector diffs the COLUMNS rather than hooking the write sites: a
//! publisher inside this daemon cannot observe a write another daemon made.
//!
//! # The poll does the reading
//!
//! There is no background watcher and no registry of watched ids. The request
//! carries the ids its client has open, and while parked it re-reads exactly
//! those rows on a short interval, hands them to
//! [`SessionMetaEvents::observe`], and returns as soon as the revision moves.
//! An idle app with no chats open therefore reads nothing at all.
//!
//! ⚠ **`since=0` is a baseline request, not a replay.** A client establishing
//! itself gets the current revision and no changes; asking for a replay from
//! zero on every reconnect is what once exhausted Chromium's six-socket pool
//! (`utils/catalogSubscription.ts`).

use std::sync::Arc;
use std::time::Duration;

use axum::{extract::Query, extract::State, routing::get, Json, Router};
use biorouter::session_meta::{SessionMetaDelta, SessionMetaEvents};
use serde::Deserialize;
use utoipa::IntoParams;

use crate::state::AppState;

/// The longest a poll parks before answering with "nothing yet".
///
/// The catalogue's 25 s, for the same reason: under the 30 s most proxies and
/// clients give up at, and long enough that an idle app is not re-establishing a
/// request every few seconds.
const MAX_WAIT: Duration = Duration::from_secs(25);

/// How often a parked poll re-reads the rows it was asked about.
///
/// A handful of indexed row reads every two seconds, which is what
/// `routes/catalog.rs` already spends on a `stat` of `config.yaml` for the same
/// class of problem — catching a change another process made within the time it
/// takes the user to switch back to the app.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// The most chats one poll will watch.
///
/// A bound, not a policy: the ids come from a query string, and an unbounded
/// list is an unbounded `IN (…)` built from client input.
const MAX_IDS: usize = 64;

#[derive(Debug, Deserialize, IntoParams)]
pub struct SessionChangesQuery {
    /// The last revision this client applied. `0` (or absent) means "tell me
    /// the current revision", which is how a fresh client establishes a
    /// baseline without a refetch.
    #[serde(default)]
    since: u64,
    /// Comma-separated session ids this client has open. Only these rows are
    /// read, so a client watching nothing costs nothing.
    ///
    /// ⚠ It scopes what can be DETECTED, not what is returned: the ring is
    /// process-wide, so a delta may name a chat this caller did not list. The
    /// caller ignores those, exactly as `CatalogChanged.session_id` is ignored
    /// by clients it does not concern.
    #[serde(default)]
    ids: Option<String>,
    /// How long to park, in milliseconds. Clamped to [`MAX_WAIT`].
    #[serde(default)]
    timeout_ms: Option<u64>,
}

fn parse_ids(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .take(MAX_IDS)
        .map(str::to_string)
        .collect()
}

/// Wait for one of `ids`' rows to change, then say what changed.
///
/// Returns immediately when the caller is already behind. A timeout is not an
/// error: the body carries the current revision and the caller polls again,
/// which is also how it survives a daemon restart — the revision resets to 0,
/// the client sees a LOWER number than it holds, and refetches.
///
/// ⚠ **`truncated` is not advisory.** It means the caller fell further behind
/// than the ring holds, so `changes` is a partial history. Applying it and
/// believing yourself current is the stale-row bug this endpoint exists to end.
#[utoipa::path(
    get,
    path = "/sessions/changes",
    params(SessionChangesQuery),
    responses(
        (status = 200, description = "The session-row delta since `since`", body = SessionMetaDelta),
        (status = 401, description = "Unauthorized - invalid secret key"),
    )
)]
pub async fn session_changes(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SessionChangesQuery>,
) -> Json<SessionMetaDelta> {
    let events = SessionMetaEvents::global();
    let ids = parse_ids(query.ids.as_deref());
    let deadline = tokio::time::Instant::now()
        + query
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(MAX_WAIT)
            .min(MAX_WAIT);

    // Adopt this caller's ids before parking. A chat opened a moment ago has not
    // changed, and reporting its whole row as new would wake every window on
    // connect — the first observation of an id is silent by construction
    // (`SessionMetaEvents::observe`).
    loop {
        // Through `storage()` rather than a `SessionManager` wrapper: this read
        // is deliberately the narrow one (four columns, no transcript), and the
        // manager's API is where the wide reads live.
        if let Ok(rows) = state
            .session_manager()
            .storage()
            .session_meta_rows(&ids)
            .await
        {
            events.observe(rows);
        }
        events.retain_watched(&ids);

        let delta = events.since(query.since);
        if !delta.changes.is_empty() || delta.truncated {
            return Json(delta);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Json(delta);
        }
        // Park on the notification OR the next read, whichever comes first: a
        // change this daemon publishes wakes us immediately, and one another
        // process made is found by the read.
        let wait = POLL_INTERVAL.min(deadline - now);
        let _ = events.wait_for_change(query.since, wait).await;
    }
}

pub fn routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/sessions/changes", get(session_changes))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_split_trimmed_and_bounded() {
        assert_eq!(parse_ids(Some("a, b ,c")), vec!["a", "b", "c"]);
        // An empty or absent list is "watch nothing", which costs no read at
        // all — the idle-app case, and the reason this endpoint is cheap.
        assert!(parse_ids(Some("")).is_empty());
        assert!(parse_ids(None).is_empty());
        assert!(parse_ids(Some(" , , ")).is_empty());
        // The list comes from a query string, so it is bounded before it
        // becomes an `IN (…)`.
        let many = (0..200)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(parse_ids(Some(&many)).len(), MAX_IDS);
    }
}
