//! Every daemon door the person cancels a turn through leaves exactly one cancel
//! note, which the model reads on its next turn (owner ruling 2026-11-17).
//!
//! `POST /agent/cancel` is the desktop's Stop, a `biorouter serve` page's Stop
//! and `biorouter session cancel`; `POST /agent/stop` stops a chat's agent.
//! Both reach the same runner (`workspace::turn::drive_stream`), which settles
//! a turn its token ended. A steer (`POST /interrupt`) adds the person's words
//! to a running turn and is never a cancel, and a cancel that lands after the
//! turn finished cancels nothing.
//!
//! Each test runs a real turn (`workspace::turn::start_turn`) against a
//! streaming provider double that stalls until the test lets it go, and holds a
//! user-action key the way the desktop's daemon does, so the requests carry the
//! person's proof. Its own binary because the installed digest is a
//! process-global `OnceLock`.

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use biorouter::agents::stopped_turn::TURN_STOPPED_NOTICE;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{
    MessageStream, Provider, ProviderMetadata, ProviderStreamItem, ProviderUsage, Usage,
};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session_events::{self, SessionBusEvent};
use biorouter_server::state::AppState;
use biorouter_server::workspace::turn::{start_turn, TurnRequest};
use futures::StreamExt;
use rmcp::model::Tool;
use serde_json::{json, Value};
use serial_test::serial;
use tokio::sync::{mpsc, Notify};
use tower::ServiceExt;

const USER_ACTION_KEY: &str = "cancelled-turn-note-routes-user-action-key";

/// Every bound in this file. It only decides how long a FAILING test takes.
const BOUND: Duration = Duration::from_secs(60);

/// What the stalled reply had streamed when the person cancelled it.
const HALF_REPLY: &str = "The telescope was invented in";

fn install_user_action_key() {
    let digest: [u8; 32] =
        <sha2::Sha256 as sha2::Digest>::digest(USER_ACTION_KEY.as_bytes()).into();
    biorouter_server::auth::install_user_action_digest(Some(digest));
    let mut headers = HeaderMap::new();
    headers.insert("X-User-Action", USER_ACTION_KEY.parse().unwrap());
    assert!(
        biorouter_server::auth::is_user_action(&headers),
        "the user-action digest did not take, so every request below would be refused"
    );
}

fn usage() -> ProviderUsage {
    ProviderUsage::new(
        "mock-model".to_string(),
        Usage::new(Some(10), Some(5), Some(15)),
    )
}

/// A streaming provider whose first request streams [`HALF_REPLY`] and then
/// waits for `release` (forever when nothing releases it). Every later request
/// is answered at once. Every request is kept.
struct StallingProvider {
    calls: AtomicUsize,
    reached: Mutex<Option<mpsc::UnboundedSender<()>>>,
    release: Arc<Notify>,
    seen: Mutex<Vec<Vec<Message>>>,
}

impl StallingProvider {
    fn new() -> (Arc<Self>, mpsc::UnboundedReceiver<()>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                reached: Mutex::new(Some(tx)),
                release: Arc::new(Notify::new()),
                seen: Mutex::new(Vec::new()),
            }),
            rx,
        )
    }

    fn texts_of_request(&self, n: usize) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .get(n)
            .map(|messages| texts(messages))
            .unwrap_or_default()
    }
}

#[async_trait]
impl Provider for StallingProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Ok((Message::assistant().with_text("A title"), usage()))
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        system_prompt: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.complete(system_prompt, messages, tools).await
    }

    async fn stream(
        &self,
        _system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(messages.to_vec());
        if n == 0 {
            let reached = self.reached.lock().unwrap().take();
            let release = Arc::clone(&self.release);
            let half: ProviderStreamItem = (
                Some(
                    Message::assistant()
                        .with_id("reply-1")
                        .with_text(HALF_REPLY),
                ),
                None,
                None,
            );
            let rest = futures::stream::once(async move {
                if let Some(reached) = reached {
                    let _ = reached.send(());
                }
                release.notified().await;
                let end: ProviderStreamItem = (
                    Some(Message::assistant().with_id("reply-1").with_text(" 1608.")),
                    Some(usage()),
                    None,
                );
                Ok(end)
            });
            return Ok(Box::pin(futures::stream::iter(vec![Ok(half)]).chain(rest)));
        }
        let item: ProviderStreamItem = (
            Some(Message::assistant().with_text("Answered.")),
            Some(usage()),
            None,
        );
        Ok(Box::pin(futures::stream::iter(vec![Ok(item)])))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "mock".to_string(),
            display_name: "Mock Provider".to_string(),
            description: "Mock provider for the cancel note route tests".to_string(),
            default_model: "mock-model".to_string(),
            known_models: vec![],
            model_doc_link: String::new(),
            config_keys: vec![],
            allows_unlisted_models: false,
            tier: Default::default(),
            runs_locally: false,
            institutions: Vec::new(),
        }
    }

    fn get_name(&self) -> &str {
        "mock-cancel-note-routes"
    }
}

fn texts(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|content| match content {
            MessageContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect()
}

/// The rows that are the cancel note, by its text, in any shape.
fn notes(messages: &[Message]) -> Vec<Message> {
    messages
        .iter()
        .filter(|message| {
            message.content.iter().any(|content| match content {
                MessageContent::Text(text) => text.text.trim() == TURN_STOPPED_NOTICE,
                MessageContent::SystemNotification(notice) => notice.msg == TURN_STOPPED_NOTICE,
                _ => false,
            })
        })
        .cloned()
        .collect()
}

struct Turn {
    state: Arc<AppState>,
    session_id: String,
    provider: Arc<StallingProvider>,
    events: session_events::Subscription,
    _work_dir: tempfile::TempDir,
}

/// A chat whose agent runs on the stalling provider, with a turn started on it
/// and its first request streamed.
async fn a_running_turn(label: &str) -> Turn {
    install_user_action_key();
    let state = AppState::new().await.unwrap();
    let work_dir = tempfile::TempDir::new().unwrap();
    let session = state
        .session_manager()
        .create_session(
            work_dir.path().to_path_buf(),
            format!("cancel note routes: {label}"),
            SessionType::User,
        )
        .await
        .unwrap();
    let (provider, mut reached) = StallingProvider::new();
    let agent = state.get_agent(session.id.clone()).await.unwrap();
    agent
        .update_provider(Arc::clone(&provider) as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();
    let started = start_turn(
        Arc::clone(&state),
        TurnRequest::new(
            session.id.clone(),
            Message::user().with_text("When was the telescope invented?"),
        ),
    )
    .await
    .expect("the turn starts");
    tokio::time::timeout(BOUND, reached.recv())
        .await
        .expect("the first request streams")
        .expect("the provider says so");
    Turn {
        state,
        session_id: session.id,
        provider,
        events: started.events,
        _work_dir: work_dir,
    }
}

impl Turn {
    /// Wait for the turn's terminal frame, and say which it was.
    async fn finished(&mut self) -> String {
        tokio::time::timeout(BOUND, async {
            loop {
                match self.events.recv().await {
                    Ok(SessionBusEvent::TurnFinished { reason, .. }) => return reason,
                    Ok(SessionBusEvent::TurnError { message, .. }) => {
                        panic!("the turn failed: {message}")
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(e) => panic!("the turn's events ended without a terminal: {e}"),
                }
            }
        })
        .await
        .expect("the turn ends")
    }

    async fn stored(&self) -> Vec<Message> {
        self.state
            .session_manager()
            .get_session(&self.session_id, true)
            .await
            .unwrap()
            .conversation
            .unwrap_or_default()
            .messages()
            .to_vec()
    }

    /// Run the chat's next turn to its end, so its request can be read.
    async fn next_turn(&mut self, text: &str) {
        let started = start_turn(
            Arc::clone(&self.state),
            TurnRequest::new(self.session_id.clone(), Message::user().with_text(text)),
        )
        .await
        .expect("the next turn starts");
        self.events = started.events;
        assert_eq!(self.finished().await, "stop", "the next turn finishes");
    }

    async fn discard(self) {
        self.state.clear_cached_agents().await;
        let _ = self
            .state
            .session_manager()
            .delete_session(&self.session_id)
            .await;
    }
}

fn post(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .method("POST")
        .header("content-type", "application/json")
        .header("X-User-Action", USER_ACTION_KEY)
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn send(app: Router, request: Request<Body>) -> (StatusCode, String) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = tokio::time::timeout(
        BOUND,
        axum::body::to_bytes(response.into_body(), usize::MAX),
    )
    .await
    .expect("the response body finishes")
    .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// The stored note: exactly one, after the prose the person was reading, read
/// by both the person and the model.
fn assert_one_note(stored: &[Message]) {
    let notes = notes(stored);
    assert_eq!(notes.len(), 1, "one note for one cancel: {stored:#?}");
    let note = &notes[0];
    assert!(
        note.is_user_visible() && note.is_agent_visible(),
        "the person and the model both read the note: {note:?}"
    );
    let at = stored.iter().position(|row| row.id == note.id).unwrap();
    assert!(
        stored[..at]
            .iter()
            .any(|row| row.as_concat_text() == HALF_REPLY),
        "the note comes after the prose the person was reading: {stored:#?}"
    );
}

/// The desktop's Stop (and a `serve` page's, and `biorouter session cancel`).
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn agent_cancel_leaves_one_note() {
    let mut turn = a_running_turn("agent cancel").await;

    let (status, body) = send(
        biorouter_server::routes::reply::routes(Arc::clone(&turn.state)),
        post(
            "/agent/cancel",
            json!({ "session_id": turn.session_id, "wait_for_idle": true }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["cancelled"], json!(true), "{body}");
    let returned: Vec<Message> =
        serde_json::from_value(body["stop_messages"].clone()).unwrap_or_default();
    assert_eq!(
        notes(&returned).len(),
        1,
        "the cancel hands the window that pressed Stop the one note it stored: {body}"
    );
    assert_eq!(turn.finished().await, "cancelled");
    assert_one_note(&turn.stored().await);

    turn.next_turn("Go on.").await;
    let next = turn.provider.texts_of_request(1);
    assert_eq!(
        next.iter()
            .map(|text| text.matches(TURN_STOPPED_NOTICE).count())
            .sum::<usize>(),
        1,
        "the next request carries the note exactly once: {next:#?}"
    );
    assert!(
        next.iter().any(|text| text == TURN_STOPPED_NOTICE
            || text.ends_with(&format!("\n\n{TURN_STOPPED_NOTICE}"))),
        "the note opens its own paragraph after the half reply: {next:#?}"
    );
    assert_eq!(notes(&turn.stored().await).len(), 1);
    turn.discard().await;
}

/// `POST /agent/stop`: the chat's agent is stopped and evicted, and the turn it
/// was running ends on the same one note.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn agent_stop_leaves_one_note() {
    let mut turn = a_running_turn("agent stop").await;

    let (status, body) = send(
        biorouter_server::routes::agent::routes(Arc::clone(&turn.state)),
        post("/agent/stop", json!({ "session_id": turn.session_id })),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(turn.finished().await, "cancelled");
    assert_one_note(&turn.stored().await);
    turn.discard().await;
}

/// A steer adds the person's words to the running turn; it cancels nothing, so
/// it leaves no note, and the turn goes on to finish.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_steer_leaves_no_note() {
    let mut turn = a_running_turn("steer").await;

    let (status, body) = send(
        biorouter_server::routes::reply::routes(Arc::clone(&turn.state)),
        post(
            "/interrupt",
            json!({
                "session_id": turn.session_id,
                "text": "Give the year only.",
                "turn_id": "steer-1",
            }),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "premise: the steer landed: {body}"
    );
    turn.provider.release.notify_one();

    assert_eq!(turn.finished().await, "stop", "the steered turn finishes");
    let stored = turn.stored().await;
    assert!(
        texts(&stored)
            .iter()
            .any(|text| text == "Give the year only."),
        "premise: the steer is in the chat: {stored:#?}"
    );
    assert!(notes(&stored).is_empty(), "{stored:#?}");
    turn.discard().await;
}

/// A cancel that lands after the turn finished cancels nothing and leaves no
/// note: the reply is whole.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_cancel_after_the_turn_finished_leaves_no_note() {
    let mut turn = a_running_turn("cancel after the end").await;
    turn.provider.release.notify_one();
    assert_eq!(turn.finished().await, "stop", "premise: the turn finished");

    let (status, body) = send(
        biorouter_server::routes::reply::routes(Arc::clone(&turn.state)),
        post(
            "/agent/cancel",
            json!({ "session_id": turn.session_id, "wait_for_idle": true }),
        ),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["cancelled"], json!(false), "{body}");
    assert!(notes(&turn.stored().await).is_empty());
    turn.discard().await;
}
