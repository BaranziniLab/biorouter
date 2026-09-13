//! SD-8 at `POST /sessions/{id}/declassify`: on a daemon that holds no
//! proof-of-user key — the one `biorouter serve` starts (SD-7) — the refusal a
//! caller reads must be one its reader can act on.
//!
//! **Measured on a real `biorouter serve` on 2026-09-12**, before any of this
//! was written. A person opened Chat history, took "Make this chat public" from
//! a private row's `⋯` menu (`aria-disabled` absent, `title` absent — fully
//! offered, with no note), satisfied the destructive confirm, and watched
//! `POST /sessions/20260905_8/declassify {"confirmation":null}` answer **403**.
//! The toast then showed them the daemon's body verbatim:
//!
//! > Marking a private chat public is a decision only the person at the keyboard
//! > can make, and this request carried no proof it came from them. Nothing was
//! > changed. Do not retry; the same call will be refused again. If this chat no
//! > longer holds anything private, stop and ask the user to mark it public from
//! > the chat history.
//!
//! That text is addressed to an AI agent that tried to declassify on a user's
//! behalf, and its advice is *hand this to the person, in the chat history*.
//! The reader was the person, in the chat history. It is an inescapable loop,
//! and it is shown to a human.
//!
//! ⚠ **Nothing here is a permission change.** A keyless daemon refused this call
//! before and refuses it now, with the same 403 and the same untouched row. What
//! moves is which sentence it sends, and — in the renderer, which this binary
//! does not reach — whether the control is offered at all.
//!
//! ⚠ **Its own test binary on purpose**, for the reason `approval_no_user_key.rs`
//! and `turn_control_no_user_key.rs` give: the installed digest is a
//! process-global `OnceLock`, the lib's own `declassify_tests` install one, and
//! inside that binary the keyless state is unreachable once the first of them
//! wins. Nothing here installs a digest — which is exactly how `biorouter serve`
//! starts its daemon. What a daemon that DOES hold a key does with the same
//! request is pinned by those lib tests, and this change leaves them alone.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use biorouter::conversation::message::Message;
use biorouter::model::ModelConfig;
use biorouter::privacy::SessionClassification;
use biorouter::session::session_manager::SessionType;
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::routes::session::{DECLASSIFY_NEEDS_USER, DECLASSIFY_NO_USER_KEY};
use biorouter_server::state::AppState;
use serial_test::serial;
use tower::ServiceExt;

/// The server secret this binary's "daemon" was launched with.
const TEST_SECRET: &str = "sd8-declassify-no-user-key";

/// Every test here stands on this: the daemon under test holds no key.
fn assert_the_daemon_is_keyless() {
    assert_eq!(
        user_action_proof(&HeaderMap::new()),
        UserActionProof::NoKeyInstalled,
        "something in this binary installed a user-action digest, so these tests would be \
         measuring a desktop daemon rather than a `biorouter serve` one"
    );
}

/// A private chat with one message, bound to a private provider, raised the way
/// a real chat gets there rather than by writing the column.
async fn seed_private(state: &Arc<AppState>, reason: &str) -> String {
    let manager = state.session_manager();
    let session = manager
        .create_session(
            std::env::temp_dir().join("sd8_declassify"),
            "SD-8 declassify fixture".to_string(),
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
        .raise_privacy(SessionClassification::Private, reason)
        .apply()
        .await
        .unwrap();
    session.id
}

/// The request the browser page sends: the daemon secret it was handed in the
/// launch URL, and — because the page's shim has no key to offer — no
/// `X-User-Action` at all.
///
/// Layered through the SAME `check_token` middleware `commands::agent::run`
/// installs, so this is the request the real daemon answers and not a bare
/// router's.
async fn post_declassify(
    state: Arc<AppState>,
    session_id: &str,
    confirmation: Option<&str>,
) -> (StatusCode, String) {
    let app = biorouter_server::routes::session::routes(state).layer(
        axum::middleware::from_fn_with_state(
            TEST_SECRET.to_string(),
            biorouter_server::auth::check_token,
        ),
    );
    let body = match confirmation {
        Some(phrase) => serde_json::json!({ "confirmation": phrase }),
        None => serde_json::json!({ "confirmation": null }),
    };
    let request = Request::builder()
        .method("POST")
        .uri(format!("/sessions/{session_id}/declassify"))
        .header("content-type", "application/json")
        .header("X-Secret-Key", TEST_SECRET)
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The QA failure, as its own regression test.
///
/// ⚠ **Fails on `origin/main`**: there the handler asks `is_user_action`, which
/// collapses `Unproven` and `NoKeyInstalled`, so a keyless daemon answers the
/// model-facing sentence and the final assertion below fails on its closing
/// clause.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_refuses_in_words_a_person_can_act_on() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    // `turn:*`, so §12.4 grades this chat onto the single-click control: the
    // credential is the only thing under test, and the typed phrase is not in
    // the way.
    let id = seed_private(&state, "turn:versa_azure").await;

    let (status, body) = post_declassify(state.clone(), &id, None).await;

    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a keyless daemon lowered a chat's classification: {body}"
    );
    assert_eq!(
        state
            .session_manager()
            .get_session(&id, false)
            .await
            .unwrap()
            .privacy_tier,
        SessionClassification::Private,
        "the refused call changed the row anyway"
    );

    assert_eq!(
        body, DECLASSIFY_NO_USER_KEY,
        "a keyless daemon answered something other than its own sentence"
    );
    assert_ne!(
        body, DECLASSIFY_NEEDS_USER,
        "the person at the keyboard was handed the sentence written for an AI agent"
    );

    // The specific clause the QA run measured in a toast, quoted out of the
    // constant that owns it so a rewording cannot leave this passing against
    // words the daemon no longer sends.
    let hand_it_to_the_user = DECLASSIFY_NEEDS_USER
        .split_once("stop and ")
        .expect("the model-facing refusal has stopped delegating to the user")
        .1;
    assert!(
        !body.contains(hand_it_to_the_user),
        "the daemon told the person at the keyboard to go and do what they are already doing: \
         {body}"
    );

    state.session_manager().delete_session(&id).await.unwrap();
}

/// The refusal does not depend on the chat, so it cannot be read as an oracle
/// for which ids exist or which are private.
///
/// Three targets, one sentence: a private chat, a public one, and an id that was
/// never created. This is the property the gate's position buys — it fires
/// before the row is read — and it survives the split into two sentences only
/// because both of them are chosen from the CALLER's credential state alone.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_keyless_refusal_is_the_same_bytes_for_every_target() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let private = seed_private(&state, "mcp:ucsfomopagent").await;
    let public = state
        .session_manager()
        .create_session(
            std::env::temp_dir().join("sd8_declassify"),
            "SD-8 public fixture".to_string(),
            SessionType::User,
        )
        .await
        .unwrap()
        .id;

    let mut bodies = Vec::new();
    for target in [private.as_str(), public.as_str(), "20990101_404"] {
        let (status, body) = post_declassify(state.clone(), target, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{target}: {body}");
        bodies.push(body);
    }
    assert!(
        bodies.windows(2).all(|pair| pair[0] == pair[1]),
        "the keyless refusal varies with the target, so it reports which chats exist: {bodies:?}"
    );

    state
        .session_manager()
        .delete_session(&private)
        .await
        .unwrap();
    state
        .session_manager()
        .delete_session(&public)
        .await
        .unwrap();
}
