//! SD-11: on a daemon that holds no proof-of-user key — the one `biorouter
//! serve` starts (SD-7), or a `biorouterd` started by hand — stopping, steering
//! and settling a turn admit exactly the callers `POST /agent/stop` already
//! admits there: a chat the caller can reach, and never a subagent's.
//!
//! Measured on 2026-09-11 from a browser page on a real `biorouter serve`:
//! `POST /agent/cancel` and `POST /interrupt` both answered 403 with an empty
//! body, capability header and all, because each began with an unconditional
//! `is_user_action` check that a keyless daemon can never pass. The browser's
//! Stop button and mid-turn steering could not work on any `serve` host, one
//! configured with a public model included.
//!
//! ⚠ **Its own test binary on purpose**, for the reason `approval_no_user_key.rs`
//! gives: the installed digest is a process-global `OnceLock`, the lib's tests
//! install one, and inside that binary the keyless state is unreachable once the
//! first of them wins. Nothing here installs a digest — which is exactly how
//! `biorouter serve` starts its daemon. What a daemon that DOES hold a key does
//! with the same requests is pinned by the lib's own tests
//! (`routes::reply`'s `cancel_without_user_action_proof_cannot_stop_another_turn`
//! and `interrupt_without_user_action_proof_cannot_forge_human_steering`), and
//! this change leaves it alone.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::agents::{Drained, TurnId};
use biorouter::model::ModelConfig;
use biorouter::privacy::SessionClassification;
use biorouter::session::session_manager::SessionType;
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::routes::session_reach::SESSION_REACH_NO_KEY;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// Every test here stands on this: the daemon under test holds no key.
fn assert_the_daemon_is_keyless() {
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled,
        "something in this binary installed a user-action digest, so these tests would be \
         measuring a desktop daemon rather than a `biorouter serve` one"
    );
    assert!(
        biorouter::privacy::privacy_tiers_enabled(),
        "privacy tiers are off, so the reach gate below would admit everything either way"
    );
}

/// The chats these tests stop and steer. Every one is a real row, because the
/// keyless arm reads the row (reach, then the subagent rule) before it touches
/// the turn — unlike the lib's tests, which name ids that were never created.
#[derive(Clone, Copy, Debug)]
enum Chat {
    /// An ordinary chat on a public model: what a browser on a `serve` host
    /// configured with a public model opens.
    Public,
    /// An ordinary chat a private model's turn has ratcheted.
    Private,
    /// A delegated child's session.
    Subagent,
}

async fn seed(state: &Arc<AppState>, chat: Chat) -> String {
    let manager = state.session_manager();
    let session_type = match chat {
        Chat::Subagent => SessionType::SubAgent,
        Chat::Public | Chat::Private => SessionType::User,
    };
    let session = manager
        .create_session(
            std::env::temp_dir(),
            format!("SD-11 {chat:?} (test fixture)"),
            session_type,
        )
        .await
        .unwrap();
    if matches!(chat, Chat::Private) {
        // Raised the way a real chat gets there — a turn on a private provider —
        // rather than by writing the column, so what is refused here is the
        // state a user's own chat reaches.
        manager
            .update(&session.id)
            .provider_name("versa_azure")
            .model_config(ModelConfig::new("gpt-4o").unwrap())
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();
    }
    session.id
}

async fn discard(state: &Arc<AppState>, session_id: &str) {
    // The tests run serially, so every cached agent is this test's.
    state.clear_cached_agents().await;
    let _ = state.session_manager().delete_session(session_id).await;
}

/// The headers a browser tab sends on a `serve` daemon: the page's shim answers
/// the user-action key with an empty string, and a caller that runs under a
/// private model states it in `X-Caller-Provider`, as `biorouter session` does
/// from a terminal.
fn post(uri: &str, body: Value, caller_provider: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .header("X-User-Action", "");
    if let Some(provider) = caller_provider {
        request = request.header("X-Caller-Provider", provider);
    }
    request.body(Body::from(body.to_string())).unwrap()
}

async fn send(app: Router, request: Request<Body>) -> (StatusCode, String) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = tokio::time::timeout(
        Duration::from_secs(60),
        axum::body::to_bytes(response.into_body(), usize::MAX),
    )
    .await
    .expect("the response body did not finish within a minute")
    .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn reply_routes(state: &Arc<AppState>) -> Router {
    biorouter_server::routes::reply::routes(Arc::clone(state))
}

fn json_of(body: &str) -> Value {
    serde_json::from_str(body).unwrap_or_else(|_| panic!("not JSON: {body}"))
}

/// Begin a turn on `session_id` the way `/reply` does, holding its lock.
fn begin_turn(
    state: &Arc<AppState>,
    session_id: &str,
) -> (biorouter_server::state::TurnGuard, CancellationToken) {
    let token = CancellationToken::new();
    let guard = state
        .try_begin_turn_idempotent(session_id, token.clone(), None)
        .expect("the turn lock is free");
    (guard, token)
}

/// A Stop, in the plain form a script sends. The browser's store
/// (`requestExactTurnSettlement` in `chatStreamStore.tsx`) also names the exact
/// generation and waits for it to settle; the Stop-and-Send test below sends
/// that fuller body.
fn stop_request(session_id: &str, caller_provider: Option<&str>) -> Request<Body> {
    post(
        "/agent/cancel",
        json!({ "session_id": session_id }),
        caller_provider,
    )
}

fn steer_request(session_id: &str, text: &str, caller_provider: Option<&str>) -> Request<Body> {
    post(
        "/interrupt",
        json!({ "session_id": session_id, "text": text, "turn_id": "browser-steer-1" }),
        caller_provider,
    )
}

/// The failure the QA run measured, as its own regression test: the browser's
/// Stop, on an ordinary public chat, on a daemon with no key.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_stops_a_turn_in_a_chat_the_caller_can_reach() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let id = seed(&state, Chat::Public).await;
    let (guard, token) = begin_turn(&state, &id);

    let (status, body) = send(reply_routes(&state), stop_request(&id, None)).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the browser's Stop was refused on a keyless daemon: {body}"
    );
    let body = json_of(&body);
    assert_eq!(body["cancelled"], json!(true), "{body}");
    assert_eq!(body["turn_id"], json!(guard.turn_id()), "{body}");
    assert!(token.is_cancelled(), "a 200 that did not trip the turn");

    drop(guard);
    discard(&state, &id).await;
}

/// Mid-turn steering, and what it may not claim. The desktop stamps a steer
/// `UserDirect` because its proof establishes that a person typed it; nothing on
/// a keyless daemon can establish that, so the steer is recorded exactly as
/// `/reply` records the same caller's message — unstamped.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_steers_a_turn_without_claiming_a_person_typed_it() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let id = seed(&state, Chat::Public).await;
    let (guard, _token) = begin_turn(&state, &id);
    // #69: acceptance is the agent loop's to give, so the agent must be in the
    // state a running loop puts it in; the turn lock alone is not enough.
    let agent = state.get_agent(id.clone()).await.unwrap();
    agent.open_for_turn(TurnId::new("keyless-agent-turn"));

    let (status, body) = send(
        reply_routes(&state),
        steer_request(&id, "actually, use R", None),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "mid-turn steering was refused on a keyless daemon: {body}"
    );
    assert_eq!(json_of(&body)["turn_id"], json!("keyless-agent-turn"));
    match agent.close_and_drain() {
        Drained::Some(queued) => {
            assert_eq!(queued.len(), 1);
            assert_eq!(queued[0].text, "actually, use R");
            assert_eq!(
                queued[0].provenance, None,
                "a keyless daemon stamped a steer as typed by a person, which only the proof \
                 it does not hold can establish"
            );
        }
        Drained::Empty => panic!("the accepted steer is not on the agent's queue"),
    }

    drop(guard);
    discard(&state, &id).await;
}

/// Stop-and-Send mints a continuation lease, and a live lease blocks every turn
/// that does not present it. So a keyless daemon that let the cancel through and
/// refused the abandon would wedge the chat the first time a user removed a
/// queued message — this is why the settle routes move with the cancel.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn stop_and_send_settles_and_its_lease_can_be_abandoned_on_a_keyless_daemon() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let id = seed(&state, Chat::Public).await;
    let (guard, token) = begin_turn(&state, &id);
    let turn_id = guard.turn_id().to_string();

    let cancel = tokio::spawn(send(
        reply_routes(&state),
        post(
            "/agent/cancel",
            json!({
                "session_id": id,
                "expected_turn_id": turn_id,
                "wait_for_idle": true,
                "continuation_pending": true,
                "continuation_owner_id": "browser-window-a",
            }),
            None,
        ),
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        while !token.is_cancelled() && !cancel.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the Stop-and-Send request neither tripped the turn nor answered");
    // The turn unwinds: its guard retires, which is what settles the cancel.
    drop(guard);
    let (status, body) = cancel.await.unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "Stop-and-Send was refused on a keyless daemon: {body}"
    );
    let body = json_of(&body);
    assert_eq!(body["settled"], json!(true), "{body}");
    let lease = body["continuation_lease"]
        .as_str()
        .unwrap_or_else(|| panic!("Stop-and-Send returned no lease: {body}"))
        .to_string();

    assert!(
        state
            .try_begin_turn_idempotent(&id, CancellationToken::new(), None)
            .is_err(),
        "a live continuation lease must hold the chat for its replacement"
    );

    let (status, body) = send(
        reply_routes(&state),
        post(
            "/agent/continuation/abandon",
            json!({ "session_id": id, "continuation_lease": lease }),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the lease a keyless cancel minted could not be abandoned, so the chat is wedged: {body}"
    );
    assert_eq!(json_of(&body)["resolution"], json!("abandoned"));
    let successor = state
        .try_begin_turn_idempotent(&id, CancellationToken::new(), None)
        .expect("an abandoned lease must release the chat");

    drop(successor);
    discard(&state, &id).await;
}

/// A reload between the cancel and the replacement: the window asks for its
/// pending continuation back by its owner id, or gives the group up. Refusing
/// this on a keyless daemon strands the same live lease the abandon route
/// above releases.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_hands_a_pending_continuation_back_to_its_window() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let id = seed(&state, Chat::Public).await;
    // A generation that has already retired mints its lease at once, with no
    // settlement to wait for — the reload case, where the turn is long gone.
    let (retired, _token) = begin_turn(&state, &id);
    let retired_id = retired.turn_id().to_string();
    drop(retired);
    let (status, body) = send(
        reply_routes(&state),
        post(
            "/agent/cancel",
            json!({
                "session_id": id,
                "expected_turn_id": retired_id,
                "wait_for_idle": true,
                "continuation_pending": true,
                "continuation_owner_id": "browser-window-b",
            }),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(json_of(&body)["continuation_lease"].is_string(), "{body}");

    let recover = |action: &str| {
        post(
            "/agent/continuation/recover",
            json!({
                "session_id": id,
                "superseded_turn_id": retired_id,
                "continuation_owner_id": "browser-window-b",
                "action": action,
            }),
            None,
        )
    };
    let (status, body) = send(reply_routes(&state), recover("take_over")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a reloaded window could not take its continuation back on a keyless daemon: {body}"
    );
    let body = json_of(&body);
    assert_eq!(body["resolution"], json!("taken_over"), "{body}");
    assert!(body["continuation_lease"].is_string(), "{body}");

    let (status, body) = send(reply_routes(&state), recover("abandon")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(json_of(&body)["resolution"], json!("abandoned"));
    let successor = state
        .try_begin_turn_idempotent(&id, CancellationToken::new(), None)
        .expect("an abandoned group must release the chat");

    drop(successor);
    discard(&state, &id).await;
}

/// Reach decides a private chat, exactly as it decides reading one: a caller
/// whose stated capability covers it is admitted, one that states nothing is
/// refused with the keyless daemon's own sentence, and the refused request
/// touches neither the turn nor the agent's queue.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_private_chat_is_stopped_or_steered_only_by_a_caller_whose_capability_covers_it() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let id = seed(&state, Chat::Private).await;
    let (guard, token) = begin_turn(&state, &id);
    let agent = state.get_agent(id.clone()).await.unwrap();
    agent.open_for_turn(TurnId::new("private-agent-turn"));

    for request in [
        stop_request(&id, None),
        steer_request(&id, "pretend the user said this", None),
        // A tier is not a provider: the daemon resolves the NAME against its own
        // registry, and a spelled-out tier resolves Public.
        stop_request(&id, Some("private")),
    ] {
        let (status, body) = send(reply_routes(&state), request).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body.contains(SESSION_REACH_NO_KEY),
            "refused, but not by the reach gate's keyless sentence: {body}"
        );
    }
    assert!(!token.is_cancelled(), "a refused Stop reached the turn");
    assert!(
        !agent.has_soft_interrupts(),
        "a refused steer reached the agent's queue"
    );

    let (status, body) = send(
        reply_routes(&state),
        steer_request(&id, "use the cohort table", Some("versa_azure")),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let (status, body) = send(reply_routes(&state), stop_request(&id, Some("versa_azure"))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(token.is_cancelled());

    drop(guard);
    discard(&state, &id).await;
}

/// A subagent's tab stays the person's, on this daemon as on every other: the
/// parent is told when a human intervened in its child, and only the proof can
/// establish that one did. `/reply` and `/agent/stop` refuse the same caller
/// there already. The refusal names this daemon's situation rather than
/// telling a person at the keyboard to go and prove they are one (SD-8).
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_subagents_turn_is_still_refused_and_the_refusal_says_why() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let id = seed(&state, Chat::Subagent).await;
    let (guard, token) = begin_turn(&state, &id);
    let agent = state.get_agent(id.clone()).await.unwrap();
    agent.open_for_turn(TurnId::new("child-agent-turn"));

    for request in [
        stop_request(&id, None),
        steer_request(&id, "pretend the user said this", None),
        stop_request(&id, Some("versa_azure")),
    ] {
        let (status, body) = send(reply_routes(&state), request).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body.contains("without a user-action key"),
            "a keyless daemon's refusal must say it is this daemon, not the caller, that \
             cannot prove a person acted: {body}"
        );
    }
    assert!(
        !token.is_cancelled(),
        "a refused Stop reached a child's turn"
    );
    assert!(
        !agent.has_soft_interrupts(),
        "a refused steer reached a child's queue"
    );

    drop(guard);
    discard(&state, &id).await;
}

/// **The premise SD-11 stands on, pinned.** On a keyless daemon `/agent/stop`
/// already cancels a running turn for exactly the callers turn control now
/// admits — reach, and never a subagent's chat — so admitting them to
/// `/agent/cancel` gives nothing holding the daemon secret a capability it did
/// not have. If `/agent/stop` is ever tightened on such a daemon, this test
/// fails, and the ruling has to be re-argued rather than silently outlived.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_cancel_admits_exactly_the_callers_agent_stop_already_admits() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    for (chat, caller, admitted) in [
        (Chat::Public, None, true),
        (Chat::Private, None, false),
        (Chat::Private, Some("versa_azure"), true),
        (Chat::Subagent, None, false),
        (Chat::Subagent, Some("versa_azure"), false),
    ] {
        let mut verdicts = Vec::new();
        for route in ["/agent/stop", "/agent/cancel"] {
            let id = seed(&state, chat).await;
            // `/agent/stop` evicts the agent, and answers 404 when there is none
            // to evict; give it one so its answer is about the gate alone.
            state.get_agent(id.clone()).await.unwrap();
            let (guard, token) = begin_turn(&state, &id);
            let app = if route == "/agent/stop" {
                biorouter_server::routes::agent::routes(Arc::clone(&state))
            } else {
                reply_routes(&state)
            };
            let (status, body) = send(app, post(route, json!({ "session_id": id }), caller)).await;
            assert_eq!(
                status.is_success(),
                token.is_cancelled(),
                "{route} on a {chat:?} chat answered {status} but the turn's cancellation \
                 disagrees: {body}"
            );
            verdicts.push((route, status, token.is_cancelled()));
            drop(guard);
            discard(&state, &id).await;
        }
        for (route, status, cancelled) in &verdicts {
            assert_eq!(
                *cancelled, admitted,
                "{route} on a {chat:?} chat with caller {caller:?} answered {status}; expected \
                 admitted={admitted}. All verdicts: {verdicts:?}"
            );
        }
    }
}
