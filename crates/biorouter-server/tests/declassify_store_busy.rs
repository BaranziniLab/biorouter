//! Item 8 of the 1.90.4 release hold, at `POST /sessions/{id}/declassify`: a
//! declassification that loses the chat store's write lock must SAY so, and must
//! say something different from a genuine failure.
//!
//! **Measured before this was written** (2026-09-13). Under a saturating
//! external writer on the same `sessions.db`, 2 of 30 declassifications came
//! back as a bodyless 500 at 5.40 s and 5.46 s, daemon log `(code: 5) database
//! is locked` — SQLite's busy timeout running out. Both rolled back, correctly,
//! and both succeeded on retry. Reproduced in the desktop app by holding the
//! write lock across one click: the toast read `Could not mark this chat public
//! [object Object]`, and the single-click dialog escalated to the typed phrase
//! with *"This chat's record has changed since this list was loaded"* — a claim
//! about the chat that nothing established.
//!
//! What is pinned here, over the real route and the real `check_token` layer:
//!
//! * a store held past the busy timeout answers **503** with `Retry-After` and
//!   `DECLASSIFY_STORE_BUSY`, writes no ledger row, leaves the chat private, and
//!   the same call succeeds once the store is free;
//! * any other failure answers **500** with `DECLASSIFY_FAILED`, not the busy
//!   sentence, and changes nothing either.
//!
//! ⚠ **Its own binary on purpose.** The busy test holds `sessions.db`'s write
//! lock for more than five seconds. In the lib's test binary, where the route's
//! other tests live, every test that touches the shared store in parallel would
//! wait out the same timeout and fail — a flake manufactured by the test. Here
//! the store is this binary's alone (`test_sandbox`), and `#[serial]` keeps the
//! two tests off each other.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so the lock below can never be taken on the developer's real
// `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::privacy::declassify::DECLASSIFY_STORE_BUSY;
use biorouter::privacy::SessionClassification;
use biorouter::session::session_manager::{SessionManager, SessionType, DB_NAME, SESSIONS_FOLDER};
use biorouter_server::routes::session::DECLASSIFY_FAILED;
use biorouter_server::state::AppState;
use serial_test::serial;
use sqlx::{ConnectOptions, Connection};
use tower::ServiceExt;

const TEST_SECRET: &str = "declassify-store-busy-secret";
const TEST_USER_ACTION_KEY: &str = "declassify-store-busy-user-action-key";

/// The desktop's daemon holds a user-action key, and the refusal for a caller
/// without one comes BEFORE the store is touched — so without this every
/// request here would be a 403 and measure nothing about the store.
fn install_user_action_key() {
    let digest: [u8; 32] =
        <sha2::Sha256 as sha2::Digest>::digest(TEST_USER_ACTION_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let mut headers = HeaderMap::new();
    headers.insert("X-User-Action", TEST_USER_ACTION_KEY.parse().unwrap());
    assert!(
        biorouter_server::auth::is_user_action(&headers),
        "the user-action digest did not take, so every request below would stop at the proof \
         check and never reach the store"
    );
}

/// A private chat that merely ran a turn on a private model: §12.4's single
/// click, so neither a phrase nor an operating-system prompt stands between the
/// request and the store.
async fn seed_turn_private(state: &Arc<AppState>) -> String {
    let manager = state.session_manager();
    let session = manager
        .create_session(
            std::env::temp_dir().join("declassify_store_busy"),
            "Store busy fixture".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    manager
        .add_message(&session.id, &Message::user().with_text("patient MRN 12345"))
        .await
        .unwrap();
    manager
        .update(&session.id)
        .provider_name("versa_azure")
        .model_config(ModelConfig::new("gpt-4o").unwrap())
        .raise_privacy(SessionClassification::Private, "turn:versa_azure")
        .apply()
        .await
        .unwrap();
    session.id
}

/// The request the desktop sends — secret, user-action proof, no confirmation —
/// through the same `check_token` layer `commands::agent::run` installs.
async fn post_declassify(
    state: Arc<AppState>,
    session_id: &str,
) -> (StatusCode, HeaderMap, String) {
    let app = biorouter_server::routes::session::routes(state).layer(
        axum::middleware::from_fn_with_state(
            TEST_SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    let request = Request::builder()
        .method("POST")
        .uri(format!("/sessions/{session_id}/declassify"))
        .header("content-type", "application/json")
        .header("X-Secret-Key", TEST_SECRET)
        .header("X-User-Action", TEST_USER_ACTION_KEY)
        .body(Body::from(r#"{"confirmation":null}"#))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// A connection of our own to THIS binary's `sessions.db`, outside the daemon's
/// pool — the shape of the external writer the measurement used.
async fn external_connection() -> sqlx::SqliteConnection {
    let path = SessionManager::shared_store_root()
        .join(SESSIONS_FOLDER)
        .join(DB_NAME);
    assert!(
        path.is_file(),
        "{} does not exist, so a lock taken on it would not be the daemon's store",
        path.display()
    );
    sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .connect()
        .await
        .unwrap()
}

/// What the store actually holds for this chat, read around the daemon.
async fn stored_state(session_id: &str) -> (String, i64) {
    let mut conn = external_connection().await;
    let tier: String = sqlx::query_scalar("SELECT privacy_tier FROM sessions WHERE id = ?1")
        .bind(session_id)
        .fetch_one(&mut conn)
        .await
        .unwrap();
    let ledger: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM classification_audit WHERE session_id = ?1")
            .bind(session_id)
            .fetch_one(&mut conn)
            .await
            .unwrap();
    conn.close().await.unwrap();
    (tier, ledger)
}

/// ⚠ **Fails on `origin/main`**, where this request answers a bodyless 500.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_store_held_past_the_busy_timeout_answers_503_in_words_and_changes_nothing() {
    install_user_action_key();
    let state = AppState::new().await.unwrap();
    let id = seed_turn_private(&state).await;

    // Hold the write lock for longer than the pool's five-second busy timeout.
    let mut holder = external_connection().await;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut holder)
        .await
        .unwrap();

    let started = Instant::now();
    let (status, headers, body) = post_declassify(state.clone(), &id).await;
    let waited = started.elapsed();

    sqlx::query("ROLLBACK").execute(&mut holder).await.unwrap();
    holder.close().await.unwrap();

    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a declassification that only waited out the store's lock must not read as a daemon \
         fault (body: {body:?})"
    );
    assert_eq!(
        body, DECLASSIFY_STORE_BUSY,
        "the 503 does not carry the sentence a person can act on"
    );
    assert_eq!(
        headers
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        Some("1")
    );
    assert!(
        waited >= Duration::from_secs(4),
        "answered after {waited:?}: it did not wait for the lock, so this did not measure a \
         lock timeout"
    );

    // Nothing landed. The one outcome this must never have is a private chat
    // lowered, or a ledger row claiming it was, by a request that failed.
    assert_eq!(
        stored_state(&id).await,
        ("private".to_string(), 0),
        "a declassification that answered 503 changed the store"
    );

    // And it is transient: the same request, with the store free, succeeds.
    let (status, _, body) = post_declassify(state.clone(), &id).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "retry after the lock cleared: {body}"
    );
    assert_eq!(stored_state(&id).await, ("public".to_string(), 1));
}

/// The busy sentence is for a busy store only. A fault that waiting cannot clear
/// — a trigger aborting the ledger insert stands in for one — is a 500 in its own
/// words, and must never tell the person to try again.
///
/// ⚠ **Fails on `origin/main`**, where it answers a bodyless 500.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_genuine_fault_answers_500_in_its_own_words_and_changes_nothing() {
    install_user_action_key();
    let state = AppState::new().await.unwrap();
    let id = seed_turn_private(&state).await;

    let trigger = format!(
        "fail_ledger_insert_{}",
        id.replace(|c: char| !c.is_alphanumeric(), "_")
    );
    let mut conn = external_connection().await;
    sqlx::query(&format!(
        "CREATE TRIGGER {trigger} BEFORE INSERT ON classification_audit \
         WHEN NEW.session_id = '{id}' BEGIN SELECT RAISE(ABORT, 'injected fault'); END"
    ))
    .execute(&mut conn)
    .await
    .unwrap();

    let (status, _, body) = post_declassify(state.clone(), &id).await;

    sqlx::query(&format!("DROP TRIGGER {trigger}"))
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body:?}");
    assert_eq!(body, DECLASSIFY_FAILED);
    assert_ne!(
        body, DECLASSIFY_STORE_BUSY,
        "a genuine fault was told to try again"
    );
    assert_eq!(stored_state(&id).await, ("private".to_string(), 0));
}
