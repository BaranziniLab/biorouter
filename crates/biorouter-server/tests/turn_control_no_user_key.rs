//! SD-11: on a daemon that holds no proof-of-user key — the one `biorouter
//! serve` starts (SD-7), or a `biorouterd` started by hand — stopping and
//! settling a turn admit exactly the callers `POST /agent/stop` already admits
//! there: a chat the caller can reach, and never a subagent's.
//!
//! Measured on 2026-09-11 from a browser page on a real `biorouter serve`:
//! `POST /agent/cancel` and `POST /interrupt` both answered 403 with an empty
//! body, capability header and all, because each began with an unconditional
//! `is_user_action` check that a keyless daemon can never pass. The browser's
//! Stop button and mid-turn steering could not work on any `serve` host, one
//! configured with a public model included.
//!
//! ⚠ **Three of those four moved; `POST /interrupt` did not** (SD-11a, the
//! security review's correction). The keyless arm rests on a dominance
//! argument — the same caller already stops that turn through `/agent/stop`,
//! and already puts text in front of that chat's model through `/reply` — and
//! for the steer that argument is false: `/reply` is refused `409` by the BR-33
//! single-turn lock in the exact state where a steer is meaningful, so the
//! dominating route cannot reach it. `/interrupt` therefore still asks for the
//! proof on every daemon, and a keyless one refuses it **in words**, which is
//! what keeps `biorouter session attach` from asking for a key that does not
//! exist. Both halves are pinned below.
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
//!
//! ⚠ **The terminal reads what these refusals look like.** `biorouter session`
//! cannot ask a daemon whether it holds a key, so `session cancel`, `attach` and
//! `send` go out without one and read the answer (`commands/session_watch.rs`,
//! `key_verdict`): an EMPTY 403 means "this daemon holds a key", and only that
//! makes the terminal ask the person for it. Every refusal here must therefore
//! carry the daemon's sentence, or a `serve` user is asked for a key that does
//! not exist — `the_terminals_empty_steer_is_answered_by_the_gate_and_touches_nothing`
//! pins it for the question the terminal actually sends.

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
use biorouter_server::routes::reply::STEER_NO_KEY;
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

/// **The one of the four that did not move, and the shape of its refusal.**
///
/// ⚠ **Why the steer is excluded.** SD-11's keyless arm is admitted on a
/// dominance argument: the caller already stops that turn through `/agent/stop`
/// and already puts text in front of that chat's model through `/reply`. The
/// second half does not hold here. `/reply` takes the BR-33 single-turn lock
/// (`try_begin_turn_idempotent_with_continuation`) and answers `409` for a
/// *different* turn while one is running; `/interrupt` answers `409` when none
/// is. The two preconditions are disjoint, so in the exact state where a steer
/// lands, the route said to dominate it is refused. What admitting it would add
/// is genuinely new: attacker-chosen text injected into a turn already in
/// flight, without cancelling it, indistinguishable in the transcript from what
/// the person watching typed. Cancel-then-reply — the nearest thing a caller
/// holding only the daemon secret already has — kills the turn first, and is
/// therefore visible.
///
/// ⚠ **And why the refusal carries a sentence rather than being empty.**
/// `biorouter session attach` tells a daemon that wants the proof apart from one
/// that cannot check it by whether a turn-control 403 has a body
/// (`session_watch::key_verdict`): empty means "this daemon holds a key", and it
/// prompts for one. A keyless daemon has no key to be typed, so an empty refusal
/// here would send a `serve` user hunting for a credential that does not exist.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_refuses_the_steer_it_admits_the_stop_for() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let id = seed(&state, Chat::Public).await;
    let (guard, token) = begin_turn(&state, &id);
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
        StatusCode::FORBIDDEN,
        "a keyless daemon injected text into a running turn: {body}"
    );
    assert!(
        body.contains(STEER_NO_KEY),
        "the steer refusal must say it is this daemon that cannot check a proof, and must not \
         be empty — an empty turn-control 403 is how `biorouter session attach` decides to \
         prompt for a user-action key: {body:?}"
    );
    assert!(
        !agent.has_soft_interrupts(),
        "a refused steer reached the agent's queue"
    );
    assert!(matches!(agent.close_and_drain(), Drained::Empty));

    // …while the Stop the same caller aims at the same turn is admitted. This
    // pairing is the finding: three of the four routes moved and this one did
    // not, so the difference is asserted in one place rather than inferred from
    // two tests that could drift apart.
    let (status, body) = send(reply_routes(&state), stop_request(&id, None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(token.is_cancelled(), "a 200 that did not trip the turn");

    drop(guard);
    discard(&state, &id).await;
}

/// The steer refusal is the SAME refusal for every chat, because the proof is
/// asked for before anything reads the row.
///
/// Worth its own assertion rather than being left implicit: `session_reach`
/// takes care to answer a private chat and a nonexistent one identically so that
/// a refusal is not a per-id oracle, and a steer gate that refused three kinds of
/// chat in three different ways would rebuild exactly that oracle on a route
/// where no proof can ever be offered.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_steer_refusal_says_the_same_thing_about_every_chat() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let mut answers = Vec::new();
    for chat in [Chat::Public, Chat::Private, Chat::Subagent] {
        let id = seed(&state, chat).await;
        let (guard, token) = begin_turn(&state, &id);
        for caller in [None, Some("versa_azure")] {
            let (status, body) = send(
                reply_routes(&state),
                steer_request(&id, "pretend the user said this", caller),
            )
            .await;
            assert!(!token.is_cancelled(), "a refused steer reached the turn");
            answers.push((format!("{chat:?}/{caller:?}"), status, body));
        }
        drop(guard);
        discard(&state, &id).await;
    }
    // A chat that was never created, as the control: reach answers this one and
    // a private chat identically, and so must the steer gate.
    let (status, body) = send(
        reply_routes(&state),
        steer_request("no-such-session", "pretend the user said this", None),
    )
    .await;
    answers.push(("absent/None".to_string(), status, body));

    let (_, first_status, first_body) = &answers[0];
    for (label, status, body) in &answers {
        assert_eq!(status, first_status, "{label}: {body}");
        assert_eq!(body, first_body, "{label}");
    }
    assert_eq!(*first_status, StatusCode::FORBIDDEN);
    assert!(first_body.contains(STEER_NO_KEY), "{first_body}");
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
///
/// Stop only. The steer never reaches reach on this daemon — its own gate
/// refuses it first, identically for every chat, which is
/// `a_keyless_steer_refusal_says_the_same_thing_about_every_chat` above.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_private_chat_is_stopped_only_by_a_caller_whose_capability_covers_it() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let id = seed(&state, Chat::Private).await;
    let (guard, token) = begin_turn(&state, &id);
    let agent = state.get_agent(id.clone()).await.unwrap();
    agent.open_for_turn(TurnId::new("private-agent-turn"));

    for request in [
        stop_request(&id, None),
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
        "a refused request reached the agent's queue"
    );

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
///
/// The Stop is refused by the subagent rule inside `authorize_agent_control`;
/// the steer is refused one step earlier, by its own gate, which refuses every
/// chat on a keyless daemon. Both sentences open the same way, which is what
/// this asserts — a person is told about the daemon either way.
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

/// The empty steer exactly as `biorouter session` sends it to ask whether it may
/// steer (`steer_gate_question` in `commands/session_watch.rs`): no
/// `X-User-Action` header at all — the browser's shim sends an empty one — and
/// no turn id.
fn terminal_question(session_id: &str, caller_provider: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .uri("/interrupt")
        .method("POST")
        .header("content-type", "application/json");
    if let Some(provider) = caller_provider {
        request = request.header("X-Caller-Provider", provider);
    }
    request
        .body(Body::from(
            json!({ "session_id": session_id, "text": "" }).to_string(),
        ))
        .unwrap()
}

/// The question `biorouter session attach` asks as it joins, and `session send`
/// after a refusal: would this daemon take a steer from this terminal, and does
/// it want the user-action key for one?
///
/// The answer here is always **no, and here is why** — `STEER_NO_KEY`, a 403
/// carrying the daemon's own sentence. ⚠ This is the row where SD-11 settled
/// narrower than the branch that wrote this test assumed. `POST /interrupt` is
/// NOT admitted by the reach gate on a keyless daemon: it asks for the proof on
/// **both** kinds, so `reply::steer_refusal` answers from the HEADERS, before
/// the body is parsed and before any chat is resolved. So the chat and the
/// stated caller change nothing, where an earlier draft expected a 400 for a
/// chat the gate would have admitted.
///
/// What the terminal actually needs is unchanged, and is what this asserts:
/// never the EMPTY 403 that means "this daemon holds a key", which would send
/// the terminal to ask the person for one that does not exist; always a sentence
/// it can print instead; and the question touches neither the turn nor the
/// agent's queue — now trivially, since nothing is reached.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_terminals_empty_steer_is_answered_by_the_gate_and_touches_nothing() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    // Every chat, asked with and without a stated capability: the answer is the
    // same 403 and the same sentence, because the refusal is decided from the
    // headers alone. The rows are kept rather than collapsed so that a change
    // admitting the steer for SOME chat fails here instead of passing quietly.
    for (chat, caller, expected) in [
        (Chat::Public, None, StatusCode::FORBIDDEN),
        (Chat::Private, None, StatusCode::FORBIDDEN),
        (Chat::Private, Some("versa_azure"), StatusCode::FORBIDDEN),
        (Chat::Subagent, None, StatusCode::FORBIDDEN),
        (Chat::Subagent, Some("versa_azure"), StatusCode::FORBIDDEN),
    ] {
        let id = seed(&state, chat).await;
        let (guard, token) = begin_turn(&state, &id);
        let agent = state.get_agent(id.clone()).await.unwrap();
        agent.open_for_turn(TurnId::new("questioned-agent-turn"));

        let (status, body) = send(reply_routes(&state), terminal_question(&id, caller)).await;

        assert_eq!(status, expected, "{chat:?} asked by {caller:?}: {body}");
        if status == StatusCode::FORBIDDEN {
            assert!(
                body.contains("without a user-action key"),
                "a keyless refusal without the daemon's sentence reads to the terminal as \
                 'this daemon wants a key': {body:?}"
            );
        }
        assert!(
            !agent.has_soft_interrupts(),
            "the terminal's question reached the agent's queue"
        );
        assert!(
            !token.is_cancelled(),
            "the terminal's question reached the turn"
        );

        drop(guard);
        discard(&state, &id).await;
    }
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
