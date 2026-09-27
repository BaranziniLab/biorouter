//! Revoke F1, defense in depth: the daemon refuses to rewrite or resend a chat whose Crew grant
//! has stopped, and it refuses BEFORE anything stored changes.
//!
//! The final acceptance measured Edit in place on a revoked chat deleting 20 and 24 stored rows
//! for a turn the daemon then refused. The renderer now holds that path first (F1), but the
//! daemon's own doors asked nothing: `POST /sessions/{id}/edit_message` truncated the chat and
//! `/reply`'s `conversation_so_far` replaced its history whatever the grant said, so a chat
//! revoked from the CLI or another window could still lose its transcript to a window that had
//! not re-read the grant. What is pinned here, against the real routes behind the real
//! `check_token`:
//!
//! - Edit in place on a chat whose grant was revoked, ended by the workspace (D-1), outlived its
//!   settings, or ran out of time answers 403 with the sentence the chat's next turn would be
//!   refused with, and the stored conversation is exactly what it was;
//! - the same chat's diverge (both doors) and `/reply` write-back say the same sentence and write
//!   nothing;
//! - a chat whose grant still stands edits as before, so the hold is not a blanket refusal of
//!   Crew chats;
//! - a caller without the person's proof is refused by the reach gate first and never learns
//!   where the grant stands.
//!
//! ⚠ **Its own binary**, because the installed user-action digest is a process-global
//! `OnceLock`, and because the Crew registry is read from disk once per process: it is written
//! here before anything opens it. Offline: nothing here opens a socket, so CI runs it in the
//! loopback-only integration step.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use biorouter::config::paths::Paths;
use biorouter::conversation::message::Message;
use biorouter::session::session_manager::SessionType;
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::sync::OnceCell;
use tower::ServiceExt;

const TEST_SECRET: &str = "crew-history-hold-secret";
const USER_KEY: &str = "crew-history-hold-user-action-key";
const CONNECTION: &str = "conn-history-hold";
/// The connection's policy epoch now; a grant made under another one outlived its settings.
const POLICY_EPOCH: u64 = 2;

/// The sentences the chat's next turn is refused with (`biorouter::crew`), as the person reads
/// them. Written out rather than imported, so a change of wording is a change here too.
const REVOKED: &str =
    "This chat's Crew access was removed. Start a new chat, or grant access again from Crew.";
const SETTINGS_CHANGED: &str =
    "Crew settings changed since access was granted. Grant access again from Crew.";
const TIMED_OUT: &str =
    "This chat's Crew access has ended. Grant access again from Crew to continue.";

/// Crew's development credential backend, as the other Crew route binaries select it: nothing
/// here should need a device key, and if something did, it must not be the OS keychain.
#[ctor::ctor]
fn select_the_file_credential_backend() {
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    if std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT").is_none() {
        std::env::set_var(
            "BIOROUTER_DEV_PROFILE_ROOT",
            std::env::temp_dir().join(format!(
                "biorouter-crew-history-hold-profile-{}",
                std::process::id()
            )),
        );
    }
}

fn install_the_desktop_key() {
    let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("X-User-Action", USER_KEY.parse().unwrap());
    assert!(
        biorouter_server::auth::is_user_action(&headers),
        "the digest this binary installs did not take, so every positive arm below would be \
         measuring a refusal"
    );
}

/// The chats the fixture seeded, one per state a grant can be in.
#[derive(Clone)]
struct Chats {
    /// Revoked from this device, and confirmed by the workspace.
    revoked: String,
    /// The workspace refused the run as ended because its policy moved (D-1).
    ended: String,
    /// Granted under an earlier policy epoch than the connection's now.
    settings_moved: String,
    /// Its run's recorded end has passed.
    timed_out: String,
    /// A grant that still stands.
    live: String,
    /// Revoked, for the `/reply` write-back alone, so no other test's cut can reach it.
    revoked_for_reply: String,
    /// Revoked, for the unproven caller alone.
    revoked_for_unproven: String,
}

static FIXTURE: OnceCell<Chats> = OnceCell::const_new();

fn user_at(created: i64, text: &str) -> Message {
    let mut message = Message::user().with_text(text);
    message.created = created;
    message
}

fn assistant_at(created: i64, text: &str) -> Message {
    let mut message = Message::assistant().with_text(text);
    message.created = created;
    message
}

/// A saved chat holding `u@1000, a@1010, u@1020`.
async fn seeded_chat(state: &Arc<AppState>, name: &str) -> String {
    let manager = state.session_manager();
    let session = manager
        .create_session(
            std::env::temp_dir().join("crew_history_hold"),
            name.to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    for message in [
        user_at(1_000, "What changed in the methods channel?"),
        assistant_at(1_010, "Two protocols were updated."),
        user_at(1_020, "Summarize the second one."),
    ] {
        manager.add_message(&session.id, &message).await.unwrap();
    }
    session.id
}

/// Seed, once per process and before anything opens the Crew registry, one chat per grant state
/// and the registry that holds their grants.
async fn fixture(state: &Arc<AppState>) -> Chats {
    FIXTURE
        .get_or_init(|| async {
            assert_eq!(
                std::env::var("BIOROUTER_DISABLE_KEYRING").as_deref(),
                Ok("true"),
                "Crew would read device keys from the OS keychain in this binary"
            );
            let chats = Chats {
                revoked: seeded_chat(state, "Revoked chat").await,
                ended: seeded_chat(state, "Ended chat").await,
                settings_moved: seeded_chat(state, "Settings moved chat").await,
                timed_out: seeded_chat(state, "Timed out chat").await,
                live: seeded_chat(state, "Live chat").await,
                revoked_for_reply: seeded_chat(state, "Revoked chat, reply").await,
                revoked_for_unproven: seeded_chat(state, "Revoked chat, unproven").await,
            };
            let crew_root = Paths::config_dir().join("crew");
            std::fs::create_dir_all(&crew_root).unwrap();
            assert!(
                !crew_root.join("connections.json").exists(),
                "something saved Crew connections before the fixture"
            );
            // Admitted under today's institution policy, at the connection's epoch, with an end
            // far in the future: a grant that stands. Each chat below moves one thing.
            let standing = |run: &str| {
                json!({
                    "connection_id": CONNECTION,
                    "run_id": run,
                    "channel_id": "chan-methods",
                    "source_channels": ["chan-methods"],
                    "epoch": POLICY_EPOCH,
                    "provider_binding": "versa_azure",
                    "public_provider": false,
                    "institution_policy": true,
                    "expired": false,
                    "expires_at": 4_000_000_000_u64,
                })
            };
            let revoked_run = |run: &str| {
                let mut revoked = standing(run);
                revoked["expired"] = json!(true);
                revoked["revocation"] = json!("confirmed");
                revoked
            };
            let mut ended = standing("run-ended");
            ended["expired"] = json!(true);
            ended["revocation"] = json!("ended_by_workspace");
            let mut settings_moved = standing("run-settings-moved");
            settings_moved["epoch"] = json!(POLICY_EPOCH - 1);
            let mut timed_out = standing("run-timed-out");
            timed_out["expires_at"] = json!(1_000_000_000_u64);
            let registry = json!({
                "connections": [{
                    "id": CONNECTION,
                    "name": "Methods lab",
                    "ssh_target": "crew@crew.invalid",
                    "port": null,
                    "identity_file": null,
                    "proxy_jump": null,
                    "socket_path": "/tmp/crew-history-hold.sock",
                    "owner_uid": 501,
                    "workspace_id": "workspace-methods",
                    "workspace_public_key": "00".repeat(32),
                    "cluster_connection_id": "cluster-methods",
                    "mode": "private",
                    "policy_epoch": POLICY_EPOCH,
                    "status": "disconnected",
                    "last_error": null,
                    "device_id": "11".repeat(32),
                    "public_key": "22".repeat(32),
                }],
                "scopes": {
                    chats.revoked.as_str(): revoked_run("run-revoked"),
                    chats.revoked_for_reply.as_str(): revoked_run("run-revoked-reply"),
                    chats.revoked_for_unproven.as_str(): revoked_run("run-revoked-unproven"),
                    chats.ended.as_str(): ended,
                    chats.settings_moved.as_str(): settings_moved,
                    chats.timed_out.as_str(): timed_out,
                    chats.live.as_str(): standing("run-live"),
                },
            });
            std::fs::write(
                crew_root.join("connections.json"),
                serde_json::to_vec(&registry).unwrap(),
            )
            .unwrap();
            chats
        })
        .await
        .clone()
}

async fn setup() -> (Arc<AppState>, Chats) {
    install_the_desktop_key();
    let state = AppState::new().await.unwrap();
    let chats = fixture(&state).await;
    (state, chats)
}

/// The session and reply routes behind the SAME `check_token` middleware the daemon installs.
fn app(state: Arc<AppState>) -> axum::Router {
    biorouter_server::routes::session::routes(state.clone())
        .merge(biorouter_server::routes::reply::routes(state))
        .layer(axum::middleware::from_fn_with_state(
            TEST_SECRET.to_string(),
            biorouter_server::auth::check_token,
        ))
}

/// POST `body` to `uri`; the answer's status and body (JSON when it is JSON, else its text).
/// Only the status is read of a `200` from `/reply`, whose body is a turn's live stream.
async fn post(
    state: &Arc<AppState>,
    uri: &str,
    body: Value,
    with_proof: bool,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("X-Secret-Key", TEST_SECRET)
        .header("content-type", "application/json");
    let builder = if with_proof {
        builder.header("X-User-Action", USER_KEY)
    } else {
        builder
    };
    let response = app(state.clone())
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let is_stream = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    if is_stream {
        return (status, Value::Null);
    }
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, body)
}

/// The chat's stored messages, as `(id, text)` in stored order.
async fn stored(state: &Arc<AppState>, session: &str) -> Vec<(Option<String>, String)> {
    state
        .session_manager()
        .get_session(session, true)
        .await
        .unwrap()
        .conversation
        .map(|conversation| {
            conversation
                .messages()
                .iter()
                .map(|message| (message.id.clone(), message.as_concat_text()))
                .collect()
        })
        .unwrap_or_default()
}

async fn session_count(state: &Arc<AppState>) -> usize {
    state.session_manager().list_sessions().await.unwrap().len()
}

fn edit(edit_type: &str) -> Value {
    json!({ "timestamp": 1_010, "editType": edit_type })
}

/// THE F1 CASE. Edit in place on a chat whose Crew access has ended, in each way it can end,
/// answers the sentence that chat's next turn would be refused with, and deletes nothing. It
/// used to truncate the chat at the edited message and only then start a turn the daemon
/// refused.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn edit_in_place_on_a_chat_whose_crew_access_ended_deletes_nothing_and_says_why() {
    let (state, chats) = setup().await;
    for (label, session, sentence) in [
        ("revoked", &chats.revoked, REVOKED),
        ("ended by the workspace", &chats.ended, SETTINGS_CHANGED),
        ("settings moved", &chats.settings_moved, SETTINGS_CHANGED),
        ("timed out", &chats.timed_out, TIMED_OUT),
    ] {
        let before = stored(&state, session).await;
        assert_eq!(before.len(), 3, "the {label} chat was not seeded");
        let (status, body) = post(
            &state,
            &format!("/sessions/{session}/edit_message"),
            edit("edit"),
            true,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "Edit in place on the {label} chat was not refused: {body}"
        );
        assert_eq!(body, json!(sentence), "the {label} chat's refusal");
        assert_eq!(
            stored(&state, session).await,
            before,
            "a refused Edit in place changed the {label} chat's stored history"
        );
    }
}

/// The control: a Crew chat whose grant stands is edited in place as before. Without this, a
/// hold that refused every Crew chat would pass the test above.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn edit_in_place_on_a_chat_whose_crew_access_stands_still_edits() {
    let (state, chats) = setup().await;
    let before = stored(&state, &chats.live).await;
    let (status, body) = post(
        &state,
        &format!("/sessions/{}/edit_message", chats.live),
        edit("edit"),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sessionId"], json!(chats.live));
    assert_eq!(
        stored(&state, &chats.live).await,
        before[..1].to_vec(),
        "Edit in place did not cut the live chat at the edited message"
    );
}

/// The reach gate answers first. A caller holding only the daemon secret is refused for a Crew
/// chat before the Crew hold is asked, so the refusal never says where the grant stands. (Its
/// diverge is refused as every Crew chat's copy is, as it was before the hold existed.)
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_caller_without_the_persons_proof_never_learns_where_the_grant_stands() {
    let (state, chats) = setup().await;
    let session = &chats.revoked_for_unproven;
    let before = stored(&state, session).await;
    let sessions = session_count(&state).await;
    for edit_type in ["edit", "diverge"] {
        let (status, body) = post(
            &state,
            &format!("/sessions/{session}/edit_message"),
            edit(edit_type),
            false,
        )
        .await;
        if edit_type == "edit" {
            assert_eq!(status, StatusCode::FORBIDDEN, "{edit_type}: {body}");
        } else {
            assert_ne!(status, StatusCode::OK, "{edit_type}: {body}");
        }
        let text = body.as_str().unwrap_or_default();
        for sentence in [REVOKED, SETTINGS_CHANGED, TIMED_OUT] {
            assert!(
                !text.contains(sentence),
                "{edit_type} told an unproven caller where the grant stands: {body}"
            );
        }
    }
    assert_eq!(stored(&state, session).await, before);
    assert_eq!(session_count(&state).await, sessions);
}

/// Diverge, through either door, copies the chat into a new one — which a Crew chat never is.
/// For a chat whose access ended, the person is told why in the same sentence, rather than a
/// bare 500, and no chat is created.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn diverging_a_chat_whose_crew_access_ended_says_why_and_creates_nothing() {
    let (state, chats) = setup().await;
    let sessions = session_count(&state).await;
    let (status, body) = post(
        &state,
        &format!("/sessions/{}/edit_message", chats.ended),
        edit("diverge"),
        true,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "edit_message diverge: {body}"
    );
    assert_eq!(body, json!(SETTINGS_CHANGED));
    let (status, body) = post(
        &state,
        &format!("/sessions/{}/diverge", chats.revoked),
        json!({}),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "/diverge: {body}");
    assert_eq!(body, json!(REVOKED));
    assert_eq!(
        session_count(&state).await,
        sessions,
        "a refused diverge created a chat"
    );
}

/// `/reply`'s `conversation_so_far` replaces the stored history before the turn starts. On a
/// chat whose access ended, that turn is refused, so the replacement is refused first and the
/// stored history is untouched.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn reply_with_a_history_to_write_back_on_a_chat_whose_crew_access_ended_writes_nothing() {
    let (state, chats) = setup().await;
    let session = &chats.revoked_for_reply;
    let before = stored(&state, session).await;
    let mut history: Vec<Message> = state
        .session_manager()
        .get_session(session, true)
        .await
        .unwrap()
        .conversation
        .unwrap()
        .messages()
        .to_vec();
    // Every stored message, so the write-back's own freshness check would admit it, with one of
    // them rewritten.
    history[1] = {
        let mut rewritten = assistant_at(1_010, "A rewritten answer.");
        rewritten.id = history[1].id.clone();
        rewritten
    };
    let (status, body) = post(
        &state,
        "/reply",
        json!({
            "session_id": session,
            "user_message": user_at(1_030, "Try again."),
            "conversation_so_far": history,
        }),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "/reply write-back: {body}");
    assert_eq!(body, json!(REVOKED));
    assert_eq!(
        stored(&state, session).await,
        before,
        "a refused write-back changed the stored history"
    );
}
