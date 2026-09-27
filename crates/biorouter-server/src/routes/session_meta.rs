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
//! There is no background watcher. The request carries the ids its client has
//! open, and while parked it re-reads exactly those rows on a short interval,
//! hands them to [`SessionMetaEvents::observe`], and returns as soon as the
//! revision moves. An idle app with no chats open therefore reads nothing at
//! all.
//!
//! ⚠ **A poll does register its ids, and that registry is not bookkeeping.**
//! The row map behind `observe` is process-global while an id list is one
//! window's, so a poll that pruned that map against its OWN list would evict a
//! second window's chats — and a re-adopted id is adopted SILENTLY, so the two
//! windows would thrash and neither would ever be told a row moved. The claim
//! taken below lives for exactly this request; see [`SessionMetaEvents::watch`].
//!
//! ⚠ **`since=0` is a baseline request, not a replay.** A client establishing
//! itself gets the current revision and no changes; asking for a replay from
//! zero on every reconnect is what once exhausted Chromium's six-socket pool
//! (`utils/catalogSubscription.ts`).

use std::sync::Arc;
use std::time::Duration;

use axum::{extract::Query, extract::State, http::HeaderMap, routing::get, Json, Router};
use biorouter::privacy::SessionClassification;
use biorouter::session_meta::{SessionMetaChanged, SessionMetaDelta, SessionMetaEvents};
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
        (status = 200, description = "The session-row delta since `since`, holding only the chats \
                                      this caller could open: a change is omitted for a caller with \
                                      neither the user-action proof nor a private capability when \
                                      its chat was private when it changed, is private now, or no \
                                      longer exists, as the chat is from `GET /sessions`, and such \
                                      a change never answers that caller's poll early", body = SessionMetaDelta),
        (status = 401, description = "Unauthorized - invalid secret key"),
    )
)]
pub async fn session_changes(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
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

    // Issue #56. Every change carries a chat's provider, model and privacy
    // tier, and the poll reads whichever ids its caller names — so a caller
    // holding only the daemon secret named a private chat and was told its tier
    // and every model switch. Measured on `main` (1038a113): another process
    // switched a private chat's model and this route handed that caller
    // `{"session_id":…,"provider_name":"versa_azure","model_name":…,
    // "privacy_tier":"private"}` while `GET /sessions/{that id}` refused it.
    //
    // A change is shown only while `GET /sessions` would show its chat
    // (`HttpCaller::lists_session`), and the caller is resolved ONCE for the
    // life of the poll.
    //
    // ⚠ **Asked of the chat as it is NOW, not only as the change recorded it.**
    // The ring keeps a change for as long as it holds it, stamped with the tier
    // the row had then — so filtering on that stamp alone kept handing a
    // secret-only caller a chat's earlier public binding after the chat had gone
    // private, whatever ids the poll named, while `GET /sessions/{id}` refused
    // the same caller (independent QA, 2026-09-14). See `visible_changes`.
    let caller = crate::routes::session_reach::http_caller(&headers).await;
    let may_show = |tier: Option<&str>| {
        caller.lists_session(
            tier.map(SessionClassification::from_stored)
                .unwrap_or(SessionClassification::Private),
        )
    };
    // A caller admitted to a private chat is admitted to every chat, so it is
    // answered without reading a single row back.
    let shows_every_chat = caller.lists_session(SessionClassification::Private);

    // Claimed for the life of this poll and released when it answers, so the
    // row map is pruned against the union of every live watcher rather than
    // against whichever list arrived last.
    //
    // ⚠ It must be a NAMED binding. `let _ = events.watch(&ids)` drops the
    // guard on the spot and reinstates the defect in a shape that reads as a
    // fix.
    let _claim = events.watch(&ids);

    // The highest revision this poll has already looked at and found nothing
    // for this caller in. A change withheld from the caller still moves the
    // global revision, so parking on `query.since` would find it "moved" on
    // every turn of the loop and spin until the deadline.
    let mut examined = query.since;

    // Adopt this caller's ids before parking. A chat opened a moment ago has not
    // changed, and reporting its whole row as new would wake every window on
    // connect — the first observation of an id is silent by construction
    // (`SessionMetaEvents::observe`).
    loop {
        // Through `storage()` rather than a `SessionManager` wrapper: this read
        // is deliberately the narrow one (four columns, no transcript), and the
        // manager's API is where the wide reads live.
        let excluded = caller.excluded_crew_sessions().await;
        if let Ok(mut rows) = state
            .session_manager()
            .storage()
            .session_meta_rows(&ids)
            .await
        {
            // ⚠ Observed only for the chats this caller could open. A caller
            // that named a private chat and was then woken — or handed a higher
            // revision — the moment that chat's row moved would be timing it,
            // which is the oracle the change filter below exists to close.
            rows.retain(|row| {
                may_show(row.privacy_tier.as_deref())
                    && excluded
                        .as_ref()
                        .is_ok_and(|ids| !ids.contains(&row.session_id))
            });
            events.observe(rows);
        }

        let mut delta = events.since(query.since);
        delta.changes.retain(|change| {
            may_show(change.privacy_tier.as_deref())
                && excluded
                    .as_ref()
                    .is_ok_and(|ids| !ids.contains(&change.session_id))
        });
        if !shows_every_chat && !delta.changes.is_empty() {
            delta.changes =
                visible_changes(&state, std::mem::take(&mut delta.changes), &may_show).await;
        }
        if !delta.changes.is_empty() || delta.truncated {
            return Json(delta);
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Json(delta);
        }
        // ⚠ A change withheld from this caller does NOT answer the poll: it
        // parks on to the deadline, exactly as it would had nothing changed, so
        // how soon a poll returns says nothing about a chat the caller could not
        // open. The revision it finally answers with counts every chat's
        // changes, which says how many rows moved on this machine and never
        // which — the residual `docs/deployment/programmatic-session-access.md`
        // records.
        examined = examined.max(delta.revision);
        // Park on the notification OR the next read, whichever comes first: a
        // change this daemon publishes wakes us immediately, and one another
        // process made is found by the read.
        let wait = POLL_INTERVAL.min(deadline - now);
        let _ = events.wait_for_change(examined, wait).await;
    }
}

/// `changes`, keeping only those whose chat the caller could open AS IT IS NOW.
///
/// Every change here already passed the filter on the tier it was recorded
/// with. This is the second half: the chat's current row, read back through the
/// same narrow four-column read the poll observes with. A change is kept when
/// that row is still there and `may_show` admits its current tier, so the answer
/// never outruns `GET /sessions`:
///
/// * a chat that went private after the change was recorded — its earlier
///   public binding is withheld, as the chat is;
/// * a chat that is gone — `GET /sessions` lists nothing for it either;
/// * a store that cannot be read — withheld, the fail-closed side; the caller
///   polls again.
///
/// ⚠ Withheld here means withheld from the ANSWER only, and the poll then parks
/// exactly as it does for a change it never saw, so how soon it returns says
/// nothing about the chat either.
async fn visible_changes(
    state: &AppState,
    changes: Vec<SessionMetaChanged>,
    may_show: impl Fn(Option<&str>) -> bool,
) -> Vec<SessionMetaChanged> {
    let mut named: Vec<String> = changes.iter().map(|c| c.session_id.clone()).collect();
    named.sort();
    named.dedup();
    let Ok(rows) = state
        .session_manager()
        .storage()
        .session_meta_rows(&named)
        .await
    else {
        return Vec::new();
    };
    let still_shown: std::collections::HashSet<String> = rows
        .into_iter()
        .filter(|row| may_show(row.privacy_tier.as_deref()))
        .map(|row| row.session_id)
        .collect();
    changes
        .into_iter()
        .filter(|change| still_shown.contains(&change.session_id))
        .collect()
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

    /// The claim must be BOUND, and nothing that RUNS can see whether it is.
    ///
    /// ⚠ This is the one line carrying the two-window fix, and every
    /// behavioural test in the workspace passes without it: the row map lives
    /// in `biorouter`, the poll's claim is taken here, and a route test drives
    /// neither. Deleting `let _claim = …` restores the defect, and so does
    /// writing `let _ = …`, which drops the guard on the spot while reading as
    /// a fix. A source scan is the only instrument that can see either.
    #[test]
    fn the_poll_binds_its_watch_claim_for_the_life_of_the_request() {
        // Production only. This module names the shape it forbids, so a scan
        // that read its own tests would report itself as its first offender.
        let (production, _) = include_str!("session_meta.rs")
            .split_once("#[cfg(test)]")
            .expect("this route's tests sit at the end, behind one `#[cfg(test)]`");
        assert!(
            production.len() > 5000,
            "the production slice is {} bytes, far too short to be this route — the slice is              wrong and a clean result would mean nothing",
            production.len()
        );

        // Comments are stripped for the same reason, and it is not theoretical
        // here: the call site carries a warning that SPELLS the forbidden
        // `let _ = …` form, so an unstripped scan sees two claims and fails on
        // a correct tree. Truncating early can only lose a match, and a lost
        // match fails the count below rather than passing quietly.
        let claims: Vec<&str> = production
            .lines()
            .filter_map(|line| line.split("//").next())
            .filter(|code| code.contains(".watch("))
            .map(str::trim)
            .collect();
        assert_eq!(
            claims.len(),
            1,
            "the poll claims its ids exactly once, for the life of the request. Found: {claims:?}"
        );

        // Whitespace-insensitive, so `let _=` cannot slip past a spelling.
        let squashed: String = claims[0].chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            squashed.starts_with("let") && !squashed.starts_with("let_="),
            "the claim must be bound to a NAMED local. `let _ = …` drops the guard \
             immediately, which prunes the process-global row map against this one caller's \
             ids again — the defect this endpoint had, wearing a fix. Found: {}",
            claims[0]
        );
    }
}
