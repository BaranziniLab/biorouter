//! "A steer always reaches the agent loop" — the daemon half (fix/steer-always-lands).
//!
//! Each test drives the REAL reply loop against a provider double that stalls on
//! a channel the test holds. Nothing here waits on the clock to rendezvous: every
//! `timeout` is a failure bound around a handshake, so on a machine at any load
//! the test either observes the behaviour or fails loudly, never flakes on a
//! guess about scheduling. Every test was red against `origin/main` — most as a
//! hang cut short by its bound, which is exactly the operator's symptom.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig, SteerWaitReason};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent, SteerOutcome};
use biorouter::model::ModelConfig;
use biorouter::providers::base::{
    MessageStream, PendingToolCall, Provider, ProviderMetadata, ProviderSteerReceiver,
    ProviderStreamItem, ProviderUsage, Usage,
};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use rmcp::model::Tool;
use tempfile::TempDir;
use tokio::sync::{mpsc, oneshot};

/// Every bound in this file. Generous on purpose: it only ever decides how long
/// a FAILING test takes to say so.
const BOUND: Duration = Duration::from_secs(20);

fn shared_session_manager() -> Arc<SessionManager> {
    static SESSION_MANAGER: OnceLock<Arc<SessionManager>> = OnceLock::new();
    SESSION_MANAGER
        .get_or_init(|| {
            let data_dir = TempDir::new().expect("shared test data directory");
            let session_manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
            std::mem::forget(data_dir);
            session_manager
        })
        .clone()
}

fn usage() -> ProviderUsage {
    ProviderUsage::new(
        "mock-model".to_string(),
        Usage::new(Some(10), Some(5), Some(15)),
    )
}

fn texts(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

fn metadata() -> ProviderMetadata {
    ProviderMetadata {
        name: "mock".to_string(),
        display_name: "Mock Provider".to_string(),
        description: "Mock provider for steer tests".to_string(),
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

fn finished_stream(text: &str) -> MessageStream {
    let item: ProviderStreamItem = (
        Some(Message::assistant().with_text(text)),
        Some(usage()),
        None,
    );
    Box::pin(futures::stream::iter(vec![Ok(item)]))
}

fn channel_stream(
    mut rx: mpsc::UnboundedReceiver<Result<ProviderStreamItem, ProviderError>>,
) -> MessageStream {
    Box::pin(futures::stream::poll_fn(move |cx| rx.poll_recv(cx)))
}

async fn agent_with(provider: Arc<dyn Provider>) -> (Arc<Agent>, String, TempDir) {
    let work_dir = TempDir::new().unwrap();
    let session_manager = shared_session_manager();
    let agent = Agent::with_config(AgentConfig::new(
        session_manager.clone(),
        PermissionManager::instance(),
        None,
        BioRouterMode::Auto,
    ));
    let session = session_manager
        .create_session(
            work_dir.path().to_path_buf(),
            "steer-always-lands".to_string(),
            SessionType::Hidden,
        )
        .await
        .unwrap();
    agent.update_provider(provider, &session.id).await.unwrap();
    (Arc::new(agent), session.id, work_dir)
}

fn config(session_id: &str, max_turns: u32) -> SessionConfig {
    SessionConfig {
        id: session_id.to_string(),
        schedule_id: None,
        max_turns: Some(max_turns),
        max_tool_calls: None,
        retry_config: None,
        budget: None,
        reasoning_effort: None,
    }
}

/// Run one turn on a task, forwarding every event to the returned channel as it
/// happens (the handshakes read it) and collecting them for the end.
fn spawn_turn(
    agent: Arc<Agent>,
    session_id: String,
    user: &str,
    max_turns: u32,
) -> (
    tokio::task::JoinHandle<Vec<AgentEvent>>,
    mpsc::UnboundedReceiver<AgentEvent>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let user = user.to_string();
    let handle = tokio::spawn(async move {
        let stream = agent
            .reply(
                Message::user().with_text(user),
                config(&session_id, max_turns),
                None,
            )
            .await
            .expect("the reply stream opens");
        tokio::pin!(stream);
        let mut all = Vec::new();
        while let Some(event) = stream.next().await {
            let event = event.expect("the turn does not error");
            let _ = tx.send(event.clone());
            all.push(event);
        }
        all
    });
    (handle, rx)
}

/// Wait, within [`BOUND`], for an event matching `pred`.
async fn wait_for(
    events: &mut mpsc::UnboundedReceiver<AgentEvent>,
    what: &str,
    pred: impl Fn(&AgentEvent) -> bool,
) -> AgentEvent {
    tokio::time::timeout(BOUND, async {
        loop {
            match events.recv().await {
                Some(event) if pred(&event) => return event,
                Some(_) => continue,
                None => panic!("the turn ended before {what}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

fn message_text_is(event: &AgentEvent, text: &str) -> bool {
    matches!(event, AgentEvent::Message(m) if m.as_concat_text() == text)
}

// ---------------------------------------------------------------------------
// D6 — a steer before the provider's first byte abandons the open.
// ---------------------------------------------------------------------------

struct SlowOpenProvider {
    calls: AtomicUsize,
    entered: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<Option<oneshot::Receiver<()>>>,
    open_dropped: Arc<AtomicBool>,
    seen: Mutex<Vec<Vec<String>>>,
}

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl Provider for SlowOpenProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Err(ProviderError::NotImplemented("streaming only".into()))
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
        self.seen.lock().unwrap().push(texts(messages));
        if n == 0 {
            let _flag = DropFlag(Arc::clone(&self.open_dropped));
            if let Some(entered) = self.entered.lock().unwrap().take() {
                let _ = entered.send(());
            }
            let release = self.release.lock().unwrap().take();
            if let Some(release) = release {
                let _ = release.await;
            }
            return Err(ProviderError::RequestFailed(
                "the test never releases the first request".into(),
            ));
        }
        Ok(finished_stream("answered with the steer in view"))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_restart_steering(&self) -> bool {
        true
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        metadata()
    }

    fn get_name(&self) -> &str {
        "mock-slow-open"
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn d6_a_steer_before_the_first_byte_abandons_the_open_and_reissues() {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (_release_tx, release_rx) = oneshot::channel::<()>();
    let provider = Arc::new(SlowOpenProvider {
        calls: AtomicUsize::new(0),
        entered: Mutex::new(Some(entered_tx)),
        release: Mutex::new(Some(release_rx)),
        open_dropped: Arc::new(AtomicBool::new(false)),
        seen: Mutex::new(Vec::new()),
    });
    let (agent, session_id, _work) = agent_with(provider.clone()).await;
    let (run, _events) = spawn_turn(agent.clone(), session_id, "write the Python version", 8);

    tokio::time::timeout(BOUND, entered_rx)
        .await
        .expect("the first request must reach the provider")
        .unwrap();
    agent
        .try_queue_soft_interrupt("switch to the R version".into(), None)
        .expect("a turn waiting on its provider accepts a steer");

    // `_release_tx` is held and never sent: the first request never answers.
    tokio::time::timeout(BOUND, run)
        .await
        .expect("a steer must not wait out a provider request that never answers")
        .unwrap();
    assert!(
        provider.open_dropped.load(Ordering::SeqCst),
        "the pending request must be dropped, which is what cancels it"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert!(
        provider.seen.lock().unwrap()[1]
            .iter()
            .any(|text| text == "switch to the R version"),
        "the reissued request must carry the steer"
    );
}

// ---------------------------------------------------------------------------
// D7 + D16 — a restarted answer is told it was interrupted, and a tool-call
// skeleton the dropped stream announced is retracted.
// ---------------------------------------------------------------------------

type ItemSender = mpsc::UnboundedSender<Result<ProviderStreamItem, ProviderError>>;

struct PartialThenParkProvider {
    calls: AtomicUsize,
    first: Mutex<Option<mpsc::UnboundedReceiver<Result<ProviderStreamItem, ProviderError>>>>,
    seen: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl Provider for PartialThenParkProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Err(ProviderError::NotImplemented("streaming only".into()))
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
        self.seen.lock().unwrap().push(texts(messages));
        if n == 0 {
            let rx = self.first.lock().unwrap().take().expect("first stream");
            return Ok(channel_stream(rx));
        }
        Ok(finished_stream("revised answer in R"))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_restart_steering(&self) -> bool {
        true
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        metadata()
    }

    fn get_name(&self) -> &str {
        "mock-partial-then-park"
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn d7_d16_a_restart_retracts_its_skeletons_and_tells_the_model_it_was_interrupted() {
    let (first_tx, first_rx): (ItemSender, _) = mpsc::unbounded_channel();
    let provider = Arc::new(PartialThenParkProvider {
        calls: AtomicUsize::new(0),
        first: Mutex::new(Some(first_rx)),
        seen: Mutex::new(Vec::new()),
    });
    let (agent, session_id, _work) = agent_with(provider.clone()).await;
    let (run, mut events) = spawn_turn(agent.clone(), session_id, "compare the two", 8);

    first_tx
        .send(Ok((
            None,
            None,
            Some(PendingToolCall {
                id: "call-announced".into(),
                name: "developer__shell".into(),
                partial_args: None,
            }),
        )))
        .unwrap();
    first_tx
        .send(Ok((
            Some(Message::assistant().with_text("## Plan\nFirst, the Python version")),
            None,
            None,
        )))
        .unwrap();
    wait_for(
        &mut events,
        "the announced tool call",
        |e| matches!(e, AgentEvent::ToolCallPending(p) if p.id == "call-announced"),
    )
    .await;
    wait_for(
        &mut events,
        "the partial answer",
        |e| matches!(e, AgentEvent::Message(m) if m.as_concat_text().contains("First, the Python")),
    )
    .await;

    agent
        .try_queue_soft_interrupt("do it in R instead".into(), None)
        .expect("a streaming turn accepts a steer");

    let all = tokio::time::timeout(BOUND, run)
        .await
        .expect("the restarted turn finishes")
        .unwrap();
    // `first_tx` is still alive: the first stream never ended on its own.
    drop(first_tx);

    let retracted = all
        .iter()
        .position(|e| matches!(e, AgentEvent::ToolCallsRetracted { ids } if ids == &["call-announced".to_string()]))
        .expect("D16: the dropped stream's skeleton must be retracted");
    let revised = all
        .iter()
        .position(|e| message_text_is(e, "revised answer in R"))
        .expect("the reissued answer arrives");
    assert!(
        retracted < revised,
        "retract before the reissued request's first frame"
    );

    let second = provider.seen.lock().unwrap()[1].clone();
    let partial = second
        .iter()
        .position(|t| t.contains("First, the Python version"))
        .expect("the partial answer stays in context");
    let note = second
        .iter()
        .position(|t| t.contains("was interrupted because the user added a message"))
        .expect("D7: the reissued request must say the partial was interrupted");
    let steer = second
        .iter()
        .position(|t| t == "do it in R instead")
        .expect("the reissued request carries the steer");
    assert!(
        partial < note && note < steer,
        "partial, then the note, then the steer: {second:?}"
    );
}

// ---------------------------------------------------------------------------
// D13 + D14 — live steering never blocks the loop on the provider's ack.
// ---------------------------------------------------------------------------

struct LiveAckProvider {
    calls: AtomicUsize,
    steering: Mutex<Option<ProviderSteerReceiver>>,
    first: Mutex<Option<mpsc::UnboundedReceiver<Result<ProviderStreamItem, ProviderError>>>>,
    opened: Mutex<Option<oneshot::Sender<()>>>,
}

#[async_trait]
impl Provider for LiveAckProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        Err(ProviderError::NotImplemented("streaming only".into()))
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
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        Ok(finished_stream("a later turn"))
    }

    async fn stream_with_steering(
        &self,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
        steering: ProviderSteerReceiver,
    ) -> Result<MessageStream, ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n > 0 {
            return Ok(finished_stream("a later call"));
        }
        *self.steering.lock().unwrap() = Some(steering);
        if let Some(opened) = self.opened.lock().unwrap().take() {
            let _ = opened.send(());
        }
        let rx = self.first.lock().unwrap().take().expect("first stream");
        Ok(channel_stream(rx))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_live_steering(&self) -> bool {
        true
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        metadata()
    }

    fn get_name(&self) -> &str {
        "mock-live-ack"
    }
}

fn live_ack_provider() -> (Arc<LiveAckProvider>, ItemSender, oneshot::Receiver<()>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let (opened_tx, opened_rx) = oneshot::channel();
    (
        Arc::new(LiveAckProvider {
            calls: AtomicUsize::new(0),
            steering: Mutex::new(None),
            first: Mutex::new(Some(rx)),
            opened: Mutex::new(Some(opened_tx)),
        }),
        tx,
        opened_rx,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn d13_a_card_surfaces_while_a_live_steer_waits_for_its_acknowledgement() {
    let (provider, chunks, opened) = live_ack_provider();
    let (agent, session_id, _work) = agent_with(provider.clone()).await;
    let (run, mut events) = spawn_turn(agent.clone(), session_id.clone(), "go", 8);

    tokio::time::timeout(BOUND, opened).await.unwrap().unwrap();
    chunks
        .send(Ok((
            Some(Message::assistant().with_text("working on it")),
            None,
            None,
        )))
        .unwrap();
    wait_for(&mut events, "the first chunk", |e| {
        message_text_is(e, "working on it")
    })
    .await;

    agent
        .try_queue_soft_interrupt("also plot it".into(), None)
        .expect("a streaming turn accepts a steer");
    let mut steering = provider
        .steering
        .lock()
        .unwrap()
        .take()
        .expect("steer channel");
    let request = tokio::time::timeout(BOUND, steering.recv())
        .await
        .expect("the steer is handed to the provider")
        .expect("steer request");
    assert_eq!(request.text(), "also plot it");

    // The child is parked on a card, so it will not acknowledge until someone
    // answers — and the card can only surface from the loop.
    biorouter::action_required_manager::ActionRequiredManager::global().publish(
        Some(&session_id),
        Message::assistant().with_text("CARD: allow the bridged shell call?"),
    );
    wait_for(&mut events, "the card, while the ack is still held", |e| {
        message_text_is(e, "CARD: allow the bridged shell call?")
    })
    .await;

    request.acknowledge();
    let echo = wait_for(&mut events, "the acknowledged steer's echo", |e| {
        message_text_is(e, "also plot it")
    })
    .await;
    let AgentEvent::Message(echo) = echo else {
        unreachable!()
    };
    assert_eq!(
        echo.metadata.steer_outcome, None,
        "a delivered steer is not unanswered"
    );

    drop(chunks);
    tokio::time::timeout(BOUND, run).await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn d14_a_stop_during_the_ack_wait_keeps_the_accepted_steer() {
    let (provider, chunks, opened) = live_ack_provider();
    let (agent, session_id, _work) = agent_with(provider.clone()).await;
    let cancel = tokio_util::sync::CancellationToken::new();
    let stream = agent
        .reply(
            Message::user().with_text("go"),
            config(&session_id, 8),
            Some(cancel.clone()),
        )
        .await
        .unwrap();
    tokio::pin!(stream);

    // Drive the stream by hand until the provider holds the steer request.
    let mut opened = Some(opened);
    let mut steering: Option<ProviderSteerReceiver> = None;
    let request = tokio::time::timeout(BOUND, async {
        loop {
            if let Some(receiver) = steering.as_mut() {
                tokio::select! {
                    request = receiver.recv() => return request.expect("steer request"),
                    event = stream.next() => { event.expect("the turn continues").unwrap(); }
                }
                continue;
            }
            tokio::select! {
                _ = async { opened.as_mut().unwrap().await }, if opened.is_some() => {
                    opened = None;
                    chunks
                        .send(Ok((Some(Message::assistant().with_text("streaming")), None, None)))
                        .unwrap();
                    agent
                        .try_queue_soft_interrupt("keep me".into(), None)
                        .expect("accepted");
                    steering = provider.steering.lock().unwrap().take();
                }
                event = stream.next() => { event.expect("the turn continues").unwrap(); }
            }
        }
    })
    .await
    .expect("the steer reaches the provider");

    // Stop lands here: the runner trips the token and settles WITHOUT polling
    // the reply stream again, then drops it.
    cancel.cancel();
    let settled = agent
        .settle_carried_over_soft_interrupts(&session_id)
        .await
        .unwrap();
    drop(stream);
    drop(request);

    assert_eq!(
        settled
            .iter()
            .map(Message::as_concat_text)
            .collect::<Vec<_>>(),
        ["keep me"],
        "an accepted steer in flight to the provider must survive a Stop"
    );
    assert_eq!(
        settled[0].metadata.steer_outcome,
        Some(SteerOutcome::Unanswered)
    );
}

// ---------------------------------------------------------------------------
// D2 — a steer during a long tool call says what it waits behind, and never
// cancels the tool.
// ---------------------------------------------------------------------------

struct ShellOnFifoProvider {
    calls: AtomicUsize,
    fifo: String,
    seen: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl Provider for ShellOnFifoProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(texts(messages));
        if n == 0 {
            return Ok((
                Message::assistant().with_tool_request(
                    "long-shell",
                    Ok(rmcp::model::CallToolRequestParams {
                        task: None,
                        meta: None,
                        name: "developer__shell".into(),
                        arguments: Some(rmcp::object!({ "command": format!("cat {}", self.fifo) })),
                    }),
                ),
                usage(),
            ));
        }
        Ok((
            Message::assistant().with_text("done, with the steer"),
            usage(),
        ))
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

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        metadata()
    }

    fn get_name(&self) -> &str {
        "mock-shell-on-fifo"
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn d2_a_steer_during_a_long_tool_reports_what_it_waits_behind() {
    let fifo_dir = TempDir::new().unwrap();
    let fifo = fifo_dir.path().join("release");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    let provider = Arc::new(ShellOnFifoProvider {
        calls: AtomicUsize::new(0),
        fifo: fifo.display().to_string(),
        seen: Mutex::new(Vec::new()),
    });
    let (agent, session_id, _work) = agent_with(provider.clone()).await;
    agent
        .add_extension(biorouter::agents::extension::ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "Developer tools".to_string(),
            display_name: None,
            timeout: None,
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("developer extension registers");
    let (run, mut events) = spawn_turn(agent.clone(), session_id, "run the long job", 8);

    wait_for(&mut events, "the shell call", |e| {
        matches!(e, AgentEvent::Message(m) if m.content.iter().any(|c| matches!(c, MessageContent::ToolRequest(r) if r.id == "long-shell")))
    })
    .await;
    agent
        .try_queue_soft_interrupt("then summarise it".into(), None)
        .expect("a turn running a tool accepts a steer");

    let waiting = wait_for(
        &mut events,
        "a SteerWaiting frame while the tool runs",
        |e| matches!(e, AgentEvent::SteerWaiting { .. }),
    )
    .await;
    assert!(matches!(
        waiting,
        AgentEvent::SteerWaiting {
            reason: SteerWaitReason::Tool,
            ..
        }
    ));
    assert_eq!(
        provider.calls.load(Ordering::SeqCst),
        1,
        "the tool was not cut short"
    );

    // Release the tool: the steer lands at the next boundary.
    let fifo_path = fifo.clone();
    tokio::task::spawn_blocking(move || std::fs::write(fifo_path, "released\n"))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(BOUND, run).await.unwrap().unwrap();
    assert!(provider.seen.lock().unwrap()[1]
        .iter()
        .any(|t| t == "then summarise it"));
}

// ---------------------------------------------------------------------------
// The open question — a steer after a forced exit, while children are still
// running, is ACCEPTED and continues the turn.
// ---------------------------------------------------------------------------

struct AnswersProvider {
    calls: AtomicUsize,
    seen: Mutex<Vec<Vec<String>>>,
}

#[async_trait]
impl Provider for AnswersProvider {
    async fn complete(
        &self,
        _system_prompt: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(texts(messages));
        Ok((Message::assistant().with_text("an answer"), usage()))
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

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        metadata()
    }

    fn get_name(&self) -> &str {
        "mock-answers"
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steer_while_a_forced_exit_waits_for_children_is_accepted_and_continues_the_turn() {
    let provider = Arc::new(AnswersProvider {
        calls: AtomicUsize::new(0),
        seen: Mutex::new(Vec::new()),
    });
    let (agent, session_id, _work) = agent_with(provider.clone()).await;
    let child_id = format!("{session_id}-child");
    let delegated = biorouter::agents::subagent_handle::BackgroundSubagent::register(
        &session_id,
        &child_id,
        "still-running child",
        tokio_util::sync::CancellationToken::new(),
    );
    biorouter::agents::subagent_handle::begin_child_turn(&child_id);

    // The detached runner opens a continuable turn before `reply`.
    agent.prepare_continuable_soft_interrupt_turn();
    let (run, mut events) = spawn_turn(agent.clone(), session_id.clone(), "fan out", 1);

    wait_for(&mut events, "the action-limit stop", |e| {
        matches!(e, AgentEvent::Message(m) if m.as_concat_text().contains("reached my action limit"))
    })
    .await;
    agent
        .try_queue_soft_interrupt("never mind the children, summarise now".into(), None)
        .expect("a steer while the forced exit waits for children must be accepted");

    tokio::time::timeout(BOUND, run)
        .await
        .expect("the person's steer ends the wait for children")
        .unwrap();
    let steer = agent
        .take_continuation_steer(&session_id)
        .expect("the turn continues with the steer");
    assert_eq!(
        steer.as_concat_text(),
        "never mind the children, summarise now"
    );
    assert_eq!(steer.metadata.steer_outcome, None);

    delegated.complete(biorouter::agents::SubagentResult::from_error("done"));
}
