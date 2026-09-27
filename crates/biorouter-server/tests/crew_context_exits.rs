//! A chat a Crew grant restricts keeps its context inside the channel's permissions at the three
//! HTTP doors that turn a whole chat into something else: an exported file, a diagnostics
//! bundle, and a workflow a model writes from it.
//!
//! Export already refused a Crew chat, inside the store read, but the route answered that refusal
//! as a bare 404, so the desktop could only say the chat was not found. Creating a workflow asked
//! nothing at all: a person's proof passed the reach gate, the whole transcript (the hidden
//! `<crew_context>` included) went to the model, and the workflow it wrote could be saved, shared
//! and run in chats no grant scopes. What is pinned here, against the real routes behind the real
//! `check_token`:
//!
//! - export of a Crew chat, with the person's proof, answers 403 with the plain sentence the
//!   terminal prints for the same chat, and a chat no grant restricts still exports;
//! - the diagnostics bundle for a Crew chat, which the desktop's Diagnostics button asks for with
//!   the person's proof, carries neither the transcript nor the chat's request logs (full request
//!   payloads, `<crew_context>` included), and says why; a chat no grant restricts still ships
//!   both;
//! - `POST /workflows/create` for a Crew chat answers 403 with its own plain sentence, before the
//!   chat is loaded or a model is asked, so no workflow and none of the chat comes back;
//! - a caller without the person's proof is refused by the reach gate first at every door and
//!   never learns that the chat is a Crew chat.
//!
//! ⚠ **Its own binary**, because the installed user-action digest is a process-global `OnceLock`,
//! and because the Crew registry is read from disk once per process: it is written here before
//! anything opens it. Offline: nothing here opens a socket, so CI runs it in the loopback-only
//! integration step.

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

const TEST_SECRET: &str = "crew-context-exits-secret";
const USER_KEY: &str = "crew-context-exits-user-action-key";
const CONNECTION: &str = "conn-context-exits";

/// What the person reads, written out rather than imported, so a change of wording is a change
/// here too.
const EXPORT_REFUSED: &str = "Crew context cannot be exported without its channel permissions. \
     Share an authorized message or attachment from Crew instead.";
const WORKFLOW_REFUSED: &str = "Crew context cannot be turned into a workflow, because a saved \
     workflow can be shared and run outside the channel's permissions. Write the workflow \
     yourself instead.";

/// Text only the Crew chat holds, standing in for a channel's messages.
const CHANNEL_TEXT: &str = "cohort-7 enrolment notes from #clinical-cohort";

/// Crew's development credential backend, as the other Crew route binaries select it: nothing
/// here should need a device key, and if something did, it must not be the OS keychain.
#[ctor::ctor]
fn select_the_file_credential_backend() {
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    if std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT").is_none() {
        std::env::set_var(
            "BIOROUTER_DEV_PROFILE_ROOT",
            std::env::temp_dir().join(format!(
                "biorouter-crew-context-exits-profile-{}",
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

#[derive(Clone)]
struct Chats {
    /// Restricted by a Crew grant that stands, holding channel context.
    crew: String,
    /// No grant restricts it.
    plain: String,
}

static FIXTURE: OnceCell<Chats> = OnceCell::const_new();

async fn seeded_chat(state: &Arc<AppState>, name: &str, text: &str) -> String {
    let manager = state.session_manager();
    let session = manager
        .create_session(
            std::env::temp_dir().join("crew_context_exits"),
            name.to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    for message in [
        Message::user().with_text(text),
        Message::assistant().with_text("Two protocols changed this week."),
    ] {
        manager.add_message(&session.id, &message).await.unwrap();
    }
    session.id
}

/// Seed, once per process and before anything opens the Crew registry, the two chats and the
/// registry holding the Crew chat's grant.
async fn fixture(state: &Arc<AppState>) -> Chats {
    FIXTURE
        .get_or_init(|| async {
            assert_eq!(
                std::env::var("BIOROUTER_DISABLE_KEYRING").as_deref(),
                Ok("true"),
                "Crew would read device keys from the OS keychain in this binary"
            );
            let chats = Chats {
                crew: seeded_chat(
                    state,
                    "Crew task",
                    &format!("<crew_context>{CHANNEL_TEXT}</crew_context> Summarize it."),
                )
                .await,
                plain: seeded_chat(state, "Plain chat", "What changed this week?").await,
            };
            let crew_root = Paths::config_dir().join("crew");
            std::fs::create_dir_all(&crew_root).unwrap();
            assert!(
                !crew_root.join("connections.json").exists(),
                "something saved Crew connections before the fixture"
            );
            // A PUBLIC workspace and model: the chat stays public, so no tier rule refuses it
            // and only the Crew rule can.
            let registry = json!({
                "connections": [{
                    "id": CONNECTION,
                    "name": "Cohort lab",
                    "ssh_target": "crew@crew.invalid",
                    "port": null,
                    "identity_file": null,
                    "proxy_jump": null,
                    "socket_path": "/tmp/crew-context-exits.sock",
                    "owner_uid": 501,
                    "workspace_id": "workspace-cohort",
                    "workspace_public_key": "00".repeat(32),
                    "cluster_connection_id": "cluster-cohort",
                    "mode": "public",
                    "policy_epoch": 1,
                    "status": "disconnected",
                    "last_error": null,
                    "device_id": "11".repeat(32),
                    "public_key": "22".repeat(32),
                }],
                "scopes": {
                    chats.crew.as_str(): {
                        "connection_id": CONNECTION,
                        "run_id": "run-context-exits",
                        "channel_id": "chan-cohort",
                        "source_channels": ["chan-cohort"],
                        "epoch": 1,
                        "provider_binding": "openai",
                        "public_provider": true,
                        "institution_policy": false,
                        "expired": false,
                        "expires_at": 4_000_000_000_u64,
                    },
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

/// The session, diagnostics and workflow routes behind the SAME `check_token` middleware the
/// daemon installs.
fn app(state: Arc<AppState>) -> axum::Router {
    biorouter_server::routes::session::routes(state.clone())
        .merge(biorouter_server::routes::status::routes(state.clone()))
        .merge(biorouter_server::routes::workflow::routes(state))
        .layer(axum::middleware::from_fn_with_state(
            TEST_SECRET.to_string(),
            biorouter_server::auth::check_token,
        ))
}

/// Send `method uri` (with `body` as JSON when there is one); the answer's status and body
/// (JSON when it is JSON, else its text).
async fn send(
    state: &Arc<AppState>,
    method: &str,
    uri: &str,
    body: Option<Value>,
    with_proof: bool,
) -> (StatusCode, Value) {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", TEST_SECRET);
    let builder = if with_proof {
        builder.header("X-User-Action", USER_KEY)
    } else {
        builder
    };
    let request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .unwrap();
    let response = app(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
    (status, body)
}

/// THE CROSSCUT-2 CASE. The person exporting a Crew chat from the desktop is told why it cannot
/// leave, in the sentence the terminal prints for the same chat. It used to be a bare 404, which
/// the desktop could only read as "not found".
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn exporting_a_crew_chat_says_why_rather_than_not_found() {
    let (state, chats) = setup().await;
    let (status, body) = send(
        &state,
        "GET",
        &format!("/sessions/{}/export", chats.crew),
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body, json!(EXPORT_REFUSED));
}

/// The control: a chat no grant restricts exports as it always did, so the refusal above is
/// Crew's and not a refusal of every export.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_chat_no_grant_restricts_still_exports() {
    let (state, chats) = setup().await;
    let (status, body) = send(
        &state,
        "GET",
        &format!("/sessions/{}/export", chats.plain),
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.as_str()
            .is_some_and(|exported| exported.contains("What changed this week?")),
        "the export did not carry the chat: {body}"
    );
}

/// Write a request log for `chat` where the bundle reads them, in the shape `RequestLog` writes
/// under `PayloadPolicy::Full`: a header naming the chat, then the whole request.
fn seed_request_log(chat: &str, file: &str, text: &str) {
    let logs = Paths::in_state_dir("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let header = json!({ "session_id": chat, "model_config": {} });
    let request = json!({ "data": { "messages": [ { "role": "user", "content": text } ] } });
    std::fs::write(logs.join(file), format!("{header}\n{request}\n")).unwrap();
}

/// `GET /diagnostics/{chat}` with the person's proof, as the desktop's Diagnostics button sends
/// it; each entry of the zip it returns, as text.
async fn diagnostics_bundle(state: &Arc<AppState>, chat: &str) -> Vec<(String, String)> {
    use std::io::Read;
    let request = Request::builder()
        .method("GET")
        .uri(format!("/diagnostics/{chat}"))
        .header("X-Secret-Key", TEST_SECRET)
        .header("X-User-Action", USER_KEY)
        .body(Body::empty())
        .unwrap();
    let response = app(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents).unwrap();
            (
                entry.name().to_string(),
                String::from_utf8_lossy(&contents).into_owned(),
            )
        })
        .collect()
}

/// THE CROSSCUT-2 CASE, at the desktop's other door. A diagnostics bundle used to refuse only
/// `session.json` for a Crew chat and ship the chat's request logs beside it: full payloads for
/// this public model, the `<crew_context>` message included. Now it carries neither, and its
/// notes say why in the sentence every export door uses. The rest of the bundle still ships.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_crew_chat_s_diagnostics_bundle_carries_none_of_its_context() {
    let (state, chats) = setup().await;
    seed_request_log(
        &chats.crew,
        "llm_request.crew-context-exits-crew.jsonl",
        &format!("<crew_context>{CHANNEL_TEXT}</crew_context> Summarize it."),
    );
    seed_request_log(
        &chats.plain,
        "llm_request.crew-context-exits-plain.jsonl",
        "What changed this week?",
    );

    let entries = diagnostics_bundle(&state, &chats.crew).await;
    let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    assert!(
        !names.iter().any(|name| name.starts_with("logs/")),
        "the Crew chat's request logs are in its bundle: {names:?}"
    );
    assert!(
        !names.contains(&"session.json"),
        "the Crew chat's transcript is in its bundle: {names:?}"
    );
    assert!(names.contains(&"system.txt"), "{names:?}");
    for (name, contents) in &entries {
        assert!(
            !contents.contains(CHANNEL_TEXT),
            "{name} carried the channel's context"
        );
    }
    let notes = entries
        .iter()
        .find(|(name, _)| name == "collection-notes.txt")
        .map(|(_, contents)| contents.as_str())
        .expect("a bundle without the transcript must say why");
    assert!(notes.contains(EXPORT_REFUSED), "{notes}");

    // The control: a chat no grant restricts ships its transcript and its own log.
    let entries = diagnostics_bundle(&state, &chats.plain).await;
    let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    assert!(names.contains(&"session.json"), "{names:?}");
    assert!(
        names.contains(&"logs/llm_request.crew-context-exits-plain.jsonl"),
        "{names:?}"
    );
}

/// THE DAEMON-5 / CROSSCUT-4 CASE. Creating a workflow from a Crew chat is refused with its own
/// plain sentence, and nothing of the chat comes back. Without the refusal the route loaded the
/// whole transcript and handed it to the chat's model; here, with no model configured, that
/// surfaced as a 200 carrying the generator's failure.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_workflow_is_never_made_from_a_crew_chat() {
    let (state, chats) = setup().await;
    let (status, body) = send(
        &state,
        "POST",
        "/workflows/create",
        Some(json!({ "session_id": chats.crew })),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body, json!(WORKFLOW_REFUSED));
    assert!(
        !body.to_string().contains(CHANNEL_TEXT),
        "the refusal carried the channel's context: {body}"
    );
}

/// The reach gate answers first at every door. A caller holding only the daemon secret is
/// refused for a Crew chat before the Crew rule is asked, so the refusal never says the chat is
/// a Crew chat, and no door reads it.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_caller_without_the_persons_proof_never_learns_the_chat_is_a_crew_chat() {
    let (state, chats) = setup().await;
    for (method, uri, body) in [
        ("GET", format!("/sessions/{}/export", chats.crew), None),
        ("GET", format!("/diagnostics/{}", chats.crew), None),
        (
            "POST",
            "/workflows/create".to_string(),
            Some(json!({ "session_id": chats.crew })),
        ),
    ] {
        let (status, answer) = send(&state, method, &uri, body, false).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}: {answer}");
        let text = answer.as_str().unwrap_or_default();
        assert!(
            !text.contains("Crew") && !text.contains(CHANNEL_TEXT),
            "{method} {uri} told an unproven caller about the chat: {answer}"
        );
    }
}
