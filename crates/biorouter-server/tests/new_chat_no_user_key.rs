//! SD-9: on a daemon that holds no proof-of-user key — `biorouter serve` (SD-7)
//! or a hand-run `biorouterd` — a new chat starts on the operator's configured
//! provider, private or not, and nothing on the HTTP surface can move it onto a
//! private model the operator did not configure.
//!
//! The 2026-09-10 QA run (finding F1) measured the first half failing: with
//! `versa_azure` configured, every `POST /agent/start` on a `serve` daemon was
//! refused 409, because the new-chat gate asked for a proof this daemon can
//! never check. The browser showed nothing at all.
//!
//! ⚠ **Its own test binary on purpose**, for the reason `approval_no_user_key.rs`
//! gives: the installed digest is a process-global `OnceLock`, the lib's tests
//! install one, and inside that binary the keyless state is unreachable once the
//! first of them wins. Nothing here installs a digest — which is exactly how
//! `biorouter serve` starts its daemon.

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root
// before `main`, so nothing here can open the developer's real `sessions.db`
// or write the developer's real `config.yaml`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::config::{with_config_overrides, Config};
use biorouter::conversation::message::Message;
use biorouter::privacy::refusal::USER_ACTION_REFUSAL_MARKER;
use biorouter::privacy::SessionClassification;
use biorouter::providers::versa_azure::VERSA_AZURE_DEPLOYMENT;
use biorouter_server::auth::{user_action_proof, UserActionProof};
use biorouter_server::state::AppState;
use serde_json::{json, Value};
use serial_test::serial;
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The posture the QA run used: `biorouter configure` chose Versa. The key is a
/// placeholder — these tests construct the provider and never send it a request.
fn versa_is_the_configured_default() -> HashMap<String, String> {
    HashMap::from([
        ("BIOROUTER_PROVIDER".to_string(), "versa_azure".to_string()),
        (
            "BIOROUTER_MODEL".to_string(),
            VERSA_AZURE_DEPLOYMENT.to_string(),
        ),
        (
            "VERSA_AZURE_API_KEY".to_string(),
            "placeholder-never-sent".to_string(),
        ),
    ])
}

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
        "privacy tiers are off, so no gate below would fire either way"
    );
}

async fn post_json(app: Router, uri: &str, body: Value) -> (StatusCode, String) {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = tokio::time::timeout(
        Duration::from_secs(120),
        axum::body::to_bytes(response.into_body(), usize::MAX),
    )
    .await
    .expect("the response body did not finish within two minutes")
    .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn start_request(working_dir: &std::path::Path) -> Value {
    // No extensions: the chat under test needs a model and nothing else, and
    // the machine default set is not this test's subject.
    json!({ "working_dir": working_dir, "extension_overrides": [] })
}

async fn discard(state: &Arc<AppState>, session_id: &str) {
    // The tests run serially, so every cached agent is this test's.
    state.clear_cached_agents().await;
    let _ = state.session_manager().delete_session(session_id).await;
}

/// F1, the half the QA run measured: the configured private provider is the
/// person's choice, made at the terminal, so binding it to a brand-new chat is
/// not a switch and needs no proof — on the one kind of daemon where no proof
/// can exist.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_starts_a_new_chat_on_its_configured_private_model() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let dir = tempfile::tempdir().unwrap();

    let (status, body) = with_config_overrides(
        versa_is_the_configured_default(),
        post_json(
            biorouter_server::routes::agent::routes(Arc::clone(&state)),
            "/agent/start",
            start_request(dir.path()),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a keyless daemon refused to start a chat on its own configured model: {body}"
    );
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_str()
        .expect("the started session carries an id")
        .to_string();

    let row = state
        .session_manager()
        .get_session(&id, false)
        .await
        .unwrap();
    assert_eq!(row.provider_name.as_deref(), Some("versa_azure"));
    assert_eq!(
        row.model_config.map(|config| config.model_name).as_deref(),
        Some(VERSA_AZURE_DEPLOYMENT)
    );
    // O5: the ratchet fires on the first turn, never on the bind. A chat that
    // has touched nothing is not yet private — it is private-CAPABLE.
    assert_eq!(row.privacy_tier, SessionClassification::Public);

    let agent = state.get_agent_for_route(id.clone()).await.unwrap();
    assert_eq!(
        agent
            .provider()
            .await
            .expect("the first turn must not fail with `Provider not set`")
            .get_name(),
        "versa_azure"
    );

    discard(&state, &id).await;
}

/// The complement the exemption must not leak into: a request that asks a new
/// chat for a DIFFERENT private model than the one the operator configured is
/// still refused. `/agent/start` cannot name one, so the request that can is
/// `/agent/update_provider` on the chat it just made — `Private -> Private`,
/// which the raise predicate alone would call sideways and wave through.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_will_not_move_a_new_chat_to_a_private_model_nobody_configured() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let dir = tempfile::tempdir().unwrap();

    let (status, body) = with_config_overrides(
        versa_is_the_configured_default(),
        post_json(
            biorouter_server::routes::agent::routes(Arc::clone(&state)),
            "/agent/start",
            start_request(dir.path()),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // A loopback Ollama is Private (`self_hosted_tier`), and constructing one
    // opens no connection, so port 1 is never dialled.
    let (status, body) = with_config_overrides(
        HashMap::from([("OLLAMA_HOST".to_string(), "http://127.0.0.1:1".to_string())]),
        post_json(
            biorouter_server::routes::agent::routes(Arc::clone(&state)),
            "/agent/update_provider",
            json!({ "session_id": id, "provider": "ollama", "model": "stub-model" }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a keyless daemon moved a new chat onto a private model the operator never configured: \
         {body}"
    );
    assert!(
        body.contains(USER_ACTION_REFUSAL_MARKER),
        "refused, but not by the tier gate: {body}"
    );

    let row = state
        .session_manager()
        .get_session(&id, false)
        .await
        .unwrap();
    assert_eq!(
        row.provider_name.as_deref(),
        Some("versa_azure"),
        "the refused switch rewrote the row anyway"
    );
    let agent = state.get_agent_for_route(id.clone()).await.unwrap();
    assert_eq!(agent.provider().await.unwrap().get_name(), "versa_azure");

    discard(&state, &id).await;
}

/// SD-1 is untouched by SD-9: the configured default itself still cannot be
/// changed from a keyless daemon, which is what makes it the operator's choice.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_keyless_daemon_still_refuses_to_change_the_configured_default() {
    assert_the_daemon_is_keyless();
    let state = AppState::new().await.unwrap();
    let (status, body) = post_json(
        biorouter_server::routes::config_management::routes(state),
        "/config/set_provider",
        json!({ "provider": "versa_azure", "model": VERSA_AZURE_DEPLOYMENT }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

/// "privacy_tier ratchets on the first turn as usual": the exemption changes who
/// may bind the configured model, and nothing about what a turn on it does.
///
/// The configured default here is an Ollama endpoint on loopback, because a
/// Versa module re-pointed at a stub server is no longer Private
/// (`ucsf_gateway_tier` reads the resolved host) and a test must not send
/// traffic to the real gateway. ⚠ No model runs: the endpoint is a `wiremock`
/// stub that returns one canned completion.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_first_turn_on_a_keyless_default_chat_ratchets_it_as_usual() {
    assert_the_daemon_is_keyless();

    let stub = MockServer::start().await;
    let chunk = |delta: Value, finish: Value| {
        json!({
            "id": "stub", "object": "chat.completion.chunk", "model": "stub-model",
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
        })
    };
    let sse = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        chunk(
            json!({ "role": "assistant", "content": "ready" }),
            Value::Null
        ),
        chunk(json!({ "content": "" }), json!("stop")),
    );
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_string_contains("\"stream\":true"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse),
        )
        .with_priority(1)
        .mount(&stub)
        .await;
    // Anything that asks without streaming — the chat's auto-title — gets a
    // plain completion rather than an event stream it cannot parse.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "stub", "object": "chat.completion", "model": "stub-model",
            "choices": [{ "index": 0, "finish_reason": "stop",
                          "message": { "role": "assistant", "content": "Stub title" } }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2 }
        })))
        .mount(&stub)
        .await;

    // Written to this binary's sandboxed config.yaml rather than scoped to a
    // task: the turn runs on a spawned task, which a task-local override would
    // not reach.
    let config = Config::global();
    config.set_param("BIOROUTER_PROVIDER", "ollama").unwrap();
    config.set_param("BIOROUTER_MODEL", "stub-model").unwrap();
    config.set_param("OLLAMA_HOST", stub.uri()).unwrap();

    let state = AppState::new().await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (status, body) = post_json(
        biorouter_server::routes::agent::routes(Arc::clone(&state)),
        "/agent/start",
        start_request(dir.path()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let id = serde_json::from_str::<Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let before = state
        .session_manager()
        .get_session(&id, false)
        .await
        .unwrap();
    assert_eq!(before.provider_name.as_deref(), Some("ollama"));
    assert_eq!(before.privacy_tier, SessionClassification::Public);

    let message = Message::user().with_text("Reply with the single word ready.");
    let (status, stream) = post_json(
        biorouter_server::routes::reply::routes(Arc::clone(&state)),
        "/reply",
        json!({ "user_message": message, "session_id": id }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{stream}");
    assert!(
        stream.contains("\"Finish\""),
        "the turn did not finish: {stream}"
    );
    assert!(
        stream.contains("ready"),
        "the turn did not run on the stub: {stream}"
    );

    let after = state
        .session_manager()
        .get_session(&id, false)
        .await
        .unwrap();
    assert_eq!(
        after.privacy_tier,
        SessionClassification::Private,
        "a turn on the configured private model did not ratchet the chat"
    );
    assert_eq!(after.privacy_reason.as_deref(), Some("turn:ollama"));

    // The chat's NEXT request, now that it is private. A keyless daemon reaches a
    // private chat only for a caller whose stated capability covers it, so a
    // browser tab that states nothing loses the chat it just started; the host's
    // provider, which is what a tab on this host states (SD-9), keeps it.
    let reach = |caller: Option<&str>| {
        let mut request = Request::builder().uri(format!("/sessions/{id}"));
        if let Some(provider) = caller {
            request = request.header("X-Caller-Provider", provider);
        }
        let app = biorouter_server::routes::session::routes(Arc::clone(&state));
        async move {
            app.oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(reach(None).await, StatusCode::FORBIDDEN);
    assert_eq!(reach(Some("ollama")).await, StatusCode::OK);

    discard(&state, &id).await;
    for key in ["BIOROUTER_PROVIDER", "BIOROUTER_MODEL", "OLLAMA_HOST"] {
        let _ = config.delete(key);
    }
}
