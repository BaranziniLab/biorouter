//! Decision-model monitors inside the real reply loop, and the one transcript
//! marker a turn gets. Offline: the chat model is a scripted provider and the
//! decision model a scripted [`DecisionModel`] behind the agent's own decision
//! service (`Agent::set_decision_service`), so every request the loop makes is
//! counted.
//!
//! Covers: a `stop` monitor ends the turn the way the other safety stops do
//! (never through the commit point, so an agent's injection cannot continue it
//! and the queue stays open for the person); a `notify`
//! monitor shows one notice and the turn goes on; a `steer` monitor's note
//! reaches the model; a steer the person typed beats a stop; a chat with no
//! monitors asks nothing; a private chat never reaches a public monitor model;
//! the error policies; an elicitation answer leaves the running turn's decision
//! turn alone; and a batch of 600 decisions in one turn is ONE marker.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{
    Message, MessageContent, MessageProvenance, ProvenanceKind, SteerOutcome,
};
use biorouter::decision::log::DecisionTurnMarker;
use biorouter::decision::model::DecisionCaps;
use biorouter::decision::settings::{
    DecisionSettings, MonitorAction, MonitorCriterion, MonitorErrorPolicy, MonitorTiming,
    MonitorWindow,
};
use biorouter::decision::types::{DecisionKindTag, ModelVerdicts, QuestionVerdict};
use biorouter::decision::{
    DecisionAdapter, DecisionChunk, DecisionContext, DecisionError, DecisionKind, DecisionModel,
    DecisionModelEntry, DecisionModelRef, DecisionModelSource, DecisionOrigin, DecisionRequest,
    DecisionResult, DecisionService, DecisionSession, ProbabilitySource, Question,
};
use biorouter::model::ModelConfig;
use biorouter::privacy::{ModelAffiliation, ProviderTier, SessionClassification};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use indexmap::IndexMap;
use rmcp::model::{CallToolRequestParams, Tool};
use tempfile::TempDir;

#[ctor::ctor]
fn sandbox() {
    let root = TempDir::new().expect("a sandbox");
    for sub in ["config", "data", "state"] {
        std::fs::create_dir_all(root.path().join(sub)).expect("a sandbox tree");
    }
    std::fs::write(root.path().join("config/config.yaml"), "{}\n").expect("a config file");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    static ROOT: OnceLock<TempDir> = OnceLock::new();
    let _ = ROOT.set(root);
}

// ---------------------------------------------------------------------------
// The scripted decision model.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Answer {
    /// P(yes) for every question of the request.
    Yes(f64),
    Fail,
}

struct Decisions {
    tier: ProviderTier,
    provider: &'static str,
    model: &'static str,
    answers: Mutex<VecDeque<Answer>>,
    /// Answered once the script runs out.
    otherwise: Answer,
    requests: AtomicUsize,
    opens: AtomicUsize,
    lists: AtomicUsize,
    seen: Mutex<Vec<DecisionRequest>>,
    /// The model's window, in tokens.
    context_tokens: usize,
}

impl Decisions {
    fn new(tier: ProviderTier, otherwise: Answer) -> Arc<Self> {
        Self::with_context(tier, otherwise, 16_384)
    }

    fn with_context(tier: ProviderTier, otherwise: Answer, context_tokens: usize) -> Arc<Self> {
        let (provider, model) = match tier {
            ProviderTier::Private => ("ollama_decision", "tev1:0.8b"),
            ProviderTier::Public => ("baseten_decision", "joey-1"),
        };
        Arc::new(Self {
            tier,
            provider,
            model,
            answers: Mutex::new(VecDeque::new()),
            otherwise,
            requests: AtomicUsize::new(0),
            opens: AtomicUsize::new(0),
            lists: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            context_tokens,
        })
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    fn entry(&self) -> DecisionModelEntry {
        DecisionModelEntry {
            provider: self.provider.to_string(),
            model: self.model.to_string(),
            display_name: self.model.to_string(),
            context_tokens: self.context_tokens,
            kinds: vec![
                DecisionKindTag::Binary,
                DecisionKindTag::Choice,
                DecisionKindTag::Score,
            ],
            tier: self.tier,
            affiliation: self.affiliation(),
            adapter: DecisionAdapter::SystemOne,
            local: false,
            load_refusal: None,
        }
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        (self.tier == ProviderTier::Private).then_some(ModelAffiliation::Local)
    }
}

struct ScriptedModel(Arc<Decisions>);

#[async_trait]
impl DecisionModel for ScriptedModel {
    fn model_ref(&self) -> DecisionModelRef {
        self.0.entry().model_ref()
    }
    fn display_name(&self) -> String {
        self.0.model.to_string()
    }
    fn tier(&self) -> ProviderTier {
        self.0.tier
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.0.affiliation()
    }
    fn caps(&self) -> DecisionCaps {
        DecisionCaps {
            kinds: self.0.entry().kinds,
            max_questions: 64,
            context_tokens: self.0.context_tokens,
            max_concurrency: 8,
            max_request_bytes: None,
            local: false,
        }
    }
    async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
        self.0.requests.fetch_add(1, Ordering::SeqCst);
        self.0.seen.lock().unwrap().push(request.clone());
        let answer = self
            .0
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(self.0.otherwise);
        match answer {
            Answer::Fail => Err(DecisionError::Backend {
                model: self.0.model.to_string(),
                status: Some(503),
                error_type: Some("unavailable".to_string()),
                message: "the decision server is unavailable".to_string(),
            }),
            Answer::Yes(p) => Ok(ModelVerdicts {
                verdicts: request
                    .questions
                    .keys()
                    .map(|name| QuestionVerdict {
                        question: name.clone(),
                        probabilities: IndexMap::from([
                            ("yes".to_string(), p),
                            ("no".to_string(), 1.0 - p),
                        ]),
                        source: ProbabilitySource::Native,
                        calibrated: true,
                        reported_choice: None,
                        raw: serde_json::Value::Null,
                    })
                    .collect(),
                usage: Default::default(),
                served_model: None,
            }),
        }
    }
}

struct ScriptedSource(Arc<Decisions>);

#[async_trait]
impl DecisionModelSource for ScriptedSource {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        self.0.lists.fetch_add(1, Ordering::SeqCst);
        vec![self.0.entry()]
    }
    async fn open(&self, _wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        self.0.opens.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(ScriptedModel(self.0.clone())))
    }
}

fn service(decisions: &Arc<Decisions>) -> Arc<DecisionService> {
    Arc::new(
        DecisionService::new(Arc::new(ScriptedSource(decisions.clone())))
            .with_default_model(None)
            .with_timeouts(Duration::from_secs(5), Duration::from_secs(5)),
    )
}

// ---------------------------------------------------------------------------
// The scripted chat model.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Reply {
    Text(&'static str),
    /// A call to a tool nobody offers, so the loop goes on with an error result.
    Tool,
}

struct Llm {
    tier: ProviderTier,
    script: Mutex<VecDeque<Reply>>,
    otherwise: Reply,
    calls: AtomicUsize,
    seen: Mutex<Vec<Vec<String>>>,
    agent: OnceLock<Arc<Agent>>,
    session_id: OnceLock<String>,
    /// Queued as a steer the person typed, during the first call.
    steer_on_first_call: Option<&'static str>,
    /// Queued as another chat's agent injection, during the first call.
    injection_on_first_call: Option<&'static str>,
    /// Run during the first call, as the `decide` tool would: a batch with
    /// this many chunks, through this service.
    batch_on_first_call: Option<(Arc<DecisionService>, Arc<SessionManager>, usize)>,
}

impl Llm {
    fn new(tier: ProviderTier, script: Vec<Reply>, otherwise: Reply) -> Self {
        Self {
            tier,
            script: Mutex::new(script.into()),
            otherwise,
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            agent: OnceLock::new(),
            session_id: OnceLock::new(),
            steer_on_first_call: None,
            injection_on_first_call: None,
            batch_on_first_call: None,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn texts_seen_on_call(&self, n: usize) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .get(n)
            .cloned()
            .unwrap_or_default()
    }

    async fn run_batch(&self, service: &DecisionService, sessions: &Arc<SessionManager>, n: usize) {
        let session_id = self.session_id.get().expect("a session").clone();
        let turn = biorouter::decision::turns::current_turn(&session_id);
        assert!(turn.is_some(), "a running reply has a decision turn");
        let session = DecisionSession::load(sessions.clone(), &session_id, true)
            .await
            .unwrap()
            .with_origin(DecisionOrigin::Tool, Some("call-1".to_string()))
            .with_origin_label(Some("decide".to_string()))
            .with_turn(turn);
        let questions = IndexMap::from([(
            "has_ms".to_string(),
            Question {
                instructions: "Does this note state the patient has multiple sclerosis?".into(),
                kind: DecisionKind::Binary {},
                threshold: None,
            },
        )]);
        let chunks = (0..n)
            .map(|i| DecisionChunk {
                id: format!("note-{i:04}"),
                context: DecisionContext::Text(format!("SYNTHETIC NOTE {i}")),
            })
            .collect();
        let outcomes = service
            .decide_many(Some(&session), None, &questions, chunks)
            .await
            .expect("the batch runs");
        assert_eq!(outcomes.len(), n);
    }
}

#[async_trait]
impl Provider for Llm {
    async fn complete(
        &self,
        system_prompt: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let usage = ProviderUsage::new(
            "scripted".to_string(),
            Usage::new(Some(10), Some(5), Some(15)),
        );
        if system_prompt.contains("You are a loop monitor") {
            return Ok((
                Message::assistant().with_text(r#"{"looping":false}"#),
                usage,
            ));
        }
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(
            messages
                .iter()
                .flat_map(|m| m.content.iter())
                .filter_map(|c| match c {
                    MessageContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect(),
        );
        if n == 0 {
            if let (Some(steer), Some(agent)) = (self.steer_on_first_call, self.agent.get()) {
                agent
                    .try_queue_soft_interrupt(steer.to_string(), None)
                    .expect("a steer offered mid-turn is accepted");
            }
            if let (Some(injection), Some(agent)) = (self.injection_on_first_call, self.agent.get())
            {
                agent
                    .try_queue_soft_interrupt(
                        injection.to_string(),
                        Some(MessageProvenance {
                            kind: ProvenanceKind::AgentInjection,
                            from_session_id: Some("orchestrator".to_string()),
                            from_session_name: Some("Orchestrator".to_string()),
                        }),
                    )
                    .expect("an injection offered mid-turn is accepted");
            }
            if let Some((service, sessions, chunks)) = &self.batch_on_first_call {
                self.run_batch(service, sessions, *chunks).await;
            }
        }
        let reply = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(self.otherwise);
        let message = match reply {
            Reply::Text(text) => Message::assistant().with_text(text),
            Reply::Tool => Message::assistant()
                .with_text("Working.")
                .with_tool_request(
                    format!("call-{n}"),
                    Ok(CallToolRequestParams {
                        task: None,
                        meta: None,
                        name: "nothing__missing".into(),
                        arguments: Some(rmcp::object!({"step": n})),
                    }),
                ),
        };
        Ok((message, usage))
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
        let mut config = ModelConfig::new_or_fail("scripted");
        config.context_limit = Some(1_000_000);
        config
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "scripted".to_string(),
            display_name: "Scripted".to_string(),
            description: "test".to_string(),
            default_model: "scripted".to_string(),
            known_models: vec![],
            model_doc_link: String::new(),
            config_keys: vec![],
            allows_unlisted_models: false,
            tier: Default::default(),
            runs_locally: true,
            institutions: Vec::new(),
        }
    }

    fn get_name(&self) -> &str {
        "scripted"
    }

    fn tier(&self) -> ProviderTier {
        self.tier
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        (self.tier == ProviderTier::Private).then_some(ModelAffiliation::Local)
    }
}

// ---------------------------------------------------------------------------
// The harness.
// ---------------------------------------------------------------------------

struct Chat {
    agent: Arc<Agent>,
    sessions: Arc<SessionManager>,
    session_id: String,
    _dir: TempDir,
}

async fn chat(
    llm: Arc<Llm>,
    decisions: &Arc<Decisions>,
    classification: SessionClassification,
    monitors: Vec<MonitorCriterion>,
) -> Chat {
    let dir = TempDir::new().unwrap();
    let sessions = Arc::new(SessionManager::new_ephemeral(dir.path().join("data")));
    let agent = Agent::with_config(AgentConfig::new(
        sessions.clone(),
        Arc::new(PermissionManager::new(dir.path().join("permissions"))),
        None,
        BioRouterMode::Auto,
    ));
    let session = sessions
        .create_session(
            dir.path().to_path_buf(),
            "decision monitor test".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    if classification == SessionClassification::Private {
        sessions
            .update(&session.id)
            .raise_privacy(SessionClassification::Private, "fixture")
            .apply()
            .await
            .unwrap();
    }
    if !monitors.is_empty() {
        biorouter::decision::settings::save(
            &sessions,
            &session.id,
            DecisionSettings {
                default_model: None,
                monitors,
            },
        )
        .await
        .expect("valid monitors are stored");
    }
    agent.set_decision_service(service(decisions));
    agent
        .update_provider(llm.clone() as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();
    let agent = Arc::new(agent);
    let _ = llm.agent.set(agent.clone());
    let _ = llm.session_id.set(session.id.clone());
    Chat {
        agent,
        sessions,
        session_id: session.id,
        _dir: dir,
    }
}

fn config(chat: &Chat) -> SessionConfig {
    SessionConfig {
        id: chat.session_id.clone(),
        schedule_id: None,
        max_turns: Some(8),
        max_tool_calls: None,
        retry_config: None,
        budget: None,
        reasoning_effort: None,
    }
}

async fn send(chat: &Chat, text: &str) -> Vec<Message> {
    let stream = chat
        .agent
        .reply(Message::user().with_text(text), config(chat), None)
        .await
        .expect("a reply stream");
    tokio::pin!(stream);
    let mut out = Vec::new();
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event.expect("a reply event") {
            out.push(message);
        }
    }
    out
}

fn monitor(id: &str, name: &str, action: MonitorAction, threshold: f64) -> MonitorCriterion {
    MonitorCriterion {
        id: id.into(),
        name: name.into(),
        question: Question {
            instructions: "Has the agent reached the consent form page?".into(),
            kind: DecisionKind::Binary {},
            threshold: None,
        },
        model: None,
        timing: MonitorTiming::PostIteration,
        window: MonitorWindow::default(),
        threshold,
        option: None,
        consecutive: 1,
        rearm_below: None,
        action,
        on_error: MonitorErrorPolicy::Notify,
        max_evals_per_turn: 50,
        enabled: true,
    }
}

fn notices(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::SystemNotification(n) => Some(n.msg.clone()),
            _ => None,
        })
        .collect()
}

fn markers(messages: &[Message]) -> Vec<DecisionTurnMarker> {
    messages
        .iter()
        .filter_map(|m| m.metadata.decision_summary.clone())
        .collect()
}

async fn stored(chat: &Chat) -> Vec<Message> {
    chat.sessions
        .get_session(&chat.session_id, true)
        .await
        .unwrap()
        .conversation
        .map(|c| c.messages().clone())
        .unwrap_or_default()
}

fn count_containing(texts: &[String], needle: &str) -> usize {
    texts.iter().filter(|t| t.contains(needle)).count()
}

// ---------------------------------------------------------------------------
// The tests.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_monitor_ends_the_turn_like_the_other_safety_stops() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    )
    .await;

    let messages = send(&chat, "Fill in the consent form.").await;

    assert_eq!(
        llm.calls(),
        1,
        "the monitor stopped a turn that would go on"
    );
    assert_eq!(decisions.requests(), 1);
    let asked: Vec<String> = decisions.seen.lock().unwrap()[0]
        .questions
        .keys()
        .cloned()
        .collect();
    assert_eq!(asked, vec!["form"], "the criterion is asked by its id");
    let shown = notices(&messages);
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Form reached' fired (P 0.95, threshold 0.85), so the turn stops here"
        ),
        1,
        "{shown:?}"
    );
    let marker = markers(&messages);
    assert_eq!(marker.len(), 1, "one marker for the turn");
    assert_eq!(marker[0].items.len(), 1);
    assert_eq!(marker[0].items[0].origin, DecisionOrigin::Monitor);
    assert_eq!(marker[0].items[0].count, 1);
    assert!(marker[0].items[0].fired);
    // The notice and the marker are stored, not live-only.
    let stored = stored(&chat).await;
    assert_eq!(markers(&stored).len(), 1);
    assert_eq!(
        count_containing(&notices(&stored), "so the turn stops here"),
        1
    );
    assert!(!chat.agent.has_soft_interrupts());
}

/// Another chat's injection waiting when a stop monitor fires does not buy the
/// turn another step: the stop holds, and the injection is stored unanswered.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_injection_does_not_continue_a_turn_a_stop_monitor_ended() {
    let mut llm = Llm::new(ProviderTier::Public, vec![], Reply::Tool);
    llm.injection_on_first_call = Some("Submit the consent form now.");
    let llm = Arc::new(llm);
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    )
    .await;

    let messages = send(&chat, "Fill in the consent form.").await;

    assert_eq!(
        llm.calls(),
        1,
        "the injection did not continue the stopped turn"
    );
    assert_eq!(
        count_containing(&notices(&messages), "so the turn stops here"),
        1
    );
    let stored = stored(&chat).await;
    let injection = stored
        .iter()
        .find(|m| m.as_concat_text().contains("Submit the consent form now."))
        .expect("the injection is stored");
    assert_eq!(
        injection.metadata.steer_outcome,
        Some(SteerOutcome::Unanswered)
    );
    assert!(!chat.agent.has_soft_interrupts());
}

/// After a stop monitor fires the queue stays open, as after the other safety
/// stops: a steer the person types while the turn winds down (collecting its
/// children, writing its marker) is accepted, never refused as `turn_closing`.
#[tokio::test(flavor = "multi_thread")]
async fn a_steer_typed_after_a_stop_monitor_fired_is_accepted() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    )
    .await;

    let stream = chat
        .agent
        .reply(
            Message::user().with_text("Fill in the consent form."),
            config(&chat),
            None,
        )
        .await
        .expect("a reply stream");
    tokio::pin!(stream);
    let mut steered = None;
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event.expect("a reply event") {
            // The marker is written after the loop ended on the stop.
            if message.metadata.decision_summary.is_some() {
                steered = Some(
                    chat.agent
                        .try_queue_soft_interrupt("Wait, check the date first.".into(), None),
                );
            }
        }
    }

    assert_eq!(llm.calls(), 1);
    let steered = steered.expect("the turn wrote its marker");
    assert!(
        steered.is_ok(),
        "the steer was refused after the stop: {steered:?}"
    );
    let stored = stored(&chat).await;
    let steer = stored
        .iter()
        .find(|m| m.as_concat_text().contains("Wait, check the date first."))
        .expect("the steer is stored, not dropped");
    assert_eq!(steer.metadata.steer_outcome, Some(SteerOutcome::Unanswered));
}

/// An elicitation answer goes to a tool call parked inside a turn that is still
/// running. It is not a turn of its own, so it must not replace that turn's
/// decision turn id, and its stream ending must not clear it.
#[tokio::test(flavor = "multi_thread")]
async fn an_elicitation_answer_leaves_the_running_decision_turn_alone() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Text("Done.")));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    )
    .await;
    // The running turn a tool is parked in.
    biorouter::decision::turns::begin_turn(&chat.session_id, "the-running-turn");

    let answer =
        Message::user().with_content(MessageContent::action_required_elicitation_response(
            "no-such-request",
            serde_json::json!({"confirmed": true}),
        ));
    let stream = chat
        .agent
        .reply(answer, config(&chat), None)
        .await
        .expect("a reply stream");
    {
        tokio::pin!(stream);
        while let Some(event) = stream.next().await {
            event.expect("a reply event");
        }
    }

    assert_eq!(
        biorouter::decision::turns::current_turn(&chat.session_id).as_deref(),
        Some("the-running-turn"),
        "the answer neither replaced nor ended the running turn"
    );
    assert_eq!(llm.calls(), 0, "an elicitation answer runs no loop");
    biorouter::decision::turns::end_turn(&chat.session_id, "the-running-turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notify_monitor_shows_one_notice_and_the_turn_goes_on() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool, Reply::Tool],
        Reply::Text("All done."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Notify, 0.5)],
    )
    .await;

    let messages = send(&chat, "Fill in the consent form.").await;

    assert_eq!(llm.calls(), 3, "notify does not end the turn");
    assert_eq!(
        decisions.requests(),
        3,
        "the window changed every iteration"
    );
    assert_eq!(
        count_containing(&notices(&messages), "Monitor 'Form reached' fired"),
        1,
        "a latched monitor fires once"
    );
    let marker = markers(&messages);
    assert_eq!(
        marker.len(),
        1,
        "one marker for the turn, not one per check"
    );
    let item = &marker[0].items[0];
    assert_eq!((item.count, item.passed), (3, 3));
    assert!(item.fired);
    let marker_text: Vec<String> = messages
        .iter()
        .filter(|m| m.metadata.decision_summary.is_some())
        .flat_map(|m| notices(std::slice::from_ref(m)))
        .collect();
    assert_eq!(
        marker_text,
        vec![
            "Decision models: monitor 'Form reached' checked 3 times (last P 0.90), fired"
                .to_string()
        ]
    );

    // Owner ruling 2026-11-05: a public chat may use a private (local)
    // decision model, and the decision raises nothing. The chat stays public,
    // so its public model runs the next turn; a ratchet here would have it
    // refused.
    let row = chat
        .sessions
        .get_session(&chat.session_id, false)
        .await
        .unwrap();
    assert_eq!(row.privacy_tier, SessionClassification::Public);
    assert_eq!(row.privacy_reason, None);
    let again = send(&chat, "Now submit it.").await;
    assert_eq!(llm.calls(), 4, "the next turn ran on the public model");
    assert!(
        again
            .iter()
            .any(|m| m.role == rmcp::model::Role::Assistant
                && m.as_concat_text().contains("All done.")),
        "{:?}",
        notices(&again)
    );
}

/// The window a monitor sends is fitted to its model's context before it is
/// built: a small model is asked about the newest lines, not the whole tail.
#[tokio::test(flavor = "multi_thread")]
async fn a_small_monitor_model_is_asked_about_the_newest_lines_that_fit_it() {
    const REPLY: &str = concat!(
        "ASSISTANT-NEWEST the consent form is open and every field is filled in. ",
        "The signature box is at the bottom of the page and the submit button is ",
        "greyed out until the box is ticked. The page also lists the study contact, ",
        "the withdrawal policy, the data retention period and a link to the full ",
        "protocol. Nothing has been submitted yet, and the agent is waiting for the ",
        "person to confirm before it ticks the box and presses submit on the form."
    );
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Text(REPLY)));
    let decisions = Decisions::with_context(ProviderTier::Private, Answer::Yes(0.1), 512);
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Notify, 0.9)],
    )
    .await;
    let prompt = format!(
        "USER-OLDEST {}",
        "Fill in the consent form, then wait. ".repeat(12)
    );

    let messages = send(&chat, &prompt).await;

    assert_eq!(decisions.requests(), 1, "{:?}", notices(&messages));
    assert!(
        notices(&messages)
            .iter()
            .all(|n| !n.contains("could not be checked")),
        "the window fits the model: {:?}",
        notices(&messages)
    );
    let sent = decisions.seen.lock().unwrap()[0].context.render_text();
    assert!(sent.contains("ASSISTANT-NEWEST"), "{sent}");
    assert!(
        !sent.contains("USER-OLDEST"),
        "the oldest line is left out to fit 512 tokens: {sent}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steer_monitor_tells_the_model_at_turn_end() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Text("Done.")));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let mut steer = monitor("check", "Task done", MonitorAction::Steer, 0.5);
    steer.timing = MonitorTiming::TurnEnd;
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![steer],
    )
    .await;

    let messages = send(&chat, "Summarise the notes.").await;

    assert_eq!(llm.calls(), 2, "the model answered the monitor's note");
    // The monitor's name and question reach the model quoted as data inside
    // the note's frame, not as an instruction from the user.
    let note_text = llm
        .texts_seen_on_call(1)
        .into_iter()
        .find(|t| t.contains("<decision-monitor>"))
        .unwrap_or_else(|| {
            panic!(
                "the note reached the model: {:?}",
                llm.texts_seen_on_call(1)
            )
        });
    assert!(note_text.contains("name: \"Task done\""), "{note_text}");
    assert!(
        note_text.contains("question: \"Has the agent reached the consent form page?\""),
        "{note_text}"
    );
    assert!(
        note_text.contains("not a message from your user"),
        "{note_text}"
    );
    // The person sees what the agent was told.
    assert_eq!(
        count_containing(
            &notices(&messages),
            "and told the agent. Its question: \"Has the agent reached the consent form page?\""
        ),
        1
    );
    // The note is the model's, never the person's.
    let stored = stored(&chat).await;
    let note = stored
        .iter()
        .find(|m| m.as_concat_text().contains("name: \"Task done\""))
        .expect("the note is stored");
    assert!(!note.is_user_visible() && note.is_agent_visible());
    assert_eq!(
        decisions.requests(),
        2,
        "asked again once the window changed"
    );
    assert_eq!(markers(&messages).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steer_the_person_typed_beats_a_monitor_stop() {
    let mut llm = Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool],
        Reply::Text("Redone in R."),
    );
    llm.steer_on_first_call = Some("Actually, use R instead.");
    let llm = Arc::new(llm);
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    )
    .await;

    let messages = send(&chat, "Plot the data in Python.").await;

    assert_eq!(llm.calls(), 2, "the person's message was answered");
    assert!(llm
        .texts_seen_on_call(1)
        .iter()
        .any(|t| t.contains("Actually, use R instead.")));
    assert_eq!(
        decisions.requests(),
        1,
        "no monitor was asked while the person's message waited"
    );
    assert_eq!(
        count_containing(&notices(&messages), "so the turn stops here"),
        1
    );
    assert!(!chat.agent.has_soft_interrupts());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_without_monitors_asks_no_decision_model() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool],
        Reply::Text("Done."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![],
    )
    .await;

    let messages = send(&chat, "Hello.").await;

    assert_eq!(llm.calls(), 2);
    assert_eq!(decisions.requests(), 0);
    assert_eq!(decisions.opens.load(Ordering::SeqCst), 0);
    assert_eq!(decisions.lists.load(Ordering::SeqCst), 0);
    assert!(
        markers(&messages).is_empty(),
        "a turn that asked nothing has no marker"
    );
    assert!(markers(&stored(&chat).await).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_private_chat_never_reaches_a_public_monitor_model() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Private,
        vec![Reply::Tool],
        Reply::Text("Done."),
    ));
    let decisions = Decisions::new(ProviderTier::Public, Answer::Yes(0.99));
    let mut named = monitor("form", "Form reached", MonitorAction::Stop, 0.5);
    named.model = Some(DecisionModelRef::new("baseten_decision", "joey-1"));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Private,
        vec![named],
    )
    .await;

    let messages = send(&chat, "SYNTHETIC PRIVATE NOTE: 34F, RRMS dx 2019.").await;

    assert_eq!(decisions.requests(), 0, "nothing reached the public model");
    assert_eq!(llm.calls(), 2, "the refusal does not stop the turn");
    let notices = notices(&messages);
    assert_eq!(
        count_containing(&notices, "Monitor 'Form reached' could not be checked"),
        1,
        "{notices:?}"
    );
    assert_eq!(
        count_containing(&notices, "off for the rest of this turn"),
        1,
        "a refusal turns the monitor off at once: {notices:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_notify_monitor_that_keeps_failing_is_off_after_three_errors() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Fail);
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.5)],
    )
    .await;

    let messages = send(&chat, "Fill in the form.").await;

    // The turn outlived the monitor: it went on past the third error until
    // the loop's own limits ended it.
    assert!(
        llm.calls() > 3,
        "errors never stop a notify-on-error monitor's turn: {} calls",
        llm.calls()
    );
    assert_eq!(decisions.requests(), 3, "off after three errors in a row");
    assert_eq!(
        count_containing(&notices(&messages), "could not be checked"),
        1,
        "one notice, not one per error"
    );
    let marker = markers(&messages);
    assert_eq!(marker.len(), 1);
    assert_eq!(marker[0].items[0].errors, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pause_monitor_stops_the_turn_on_an_error() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Fail);
    let mut pause = monitor("safety", "Safety check", MonitorAction::Notify, 0.5);
    pause.on_error = MonitorErrorPolicy::Pause;
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![pause],
    )
    .await;

    let messages = send(&chat, "Go.").await;

    assert_eq!(llm.calls(), 1);
    assert_eq!(decisions.requests(), 1);
    assert_eq!(
        count_containing(&notices(&messages), "It is set to pause when that happens"),
        1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn six_hundred_decisions_in_one_turn_are_one_marker() {
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.7));
    let mut llm = Llm::new(ProviderTier::Public, vec![], Reply::Text("Cohort built."));
    let decision_service = service(&decisions);
    // The batch runs through the same store the chat lives in.
    let dir = TempDir::new().unwrap();
    let sessions = Arc::new(SessionManager::new_ephemeral(dir.path().join("data")));
    llm.batch_on_first_call = Some((decision_service.clone(), sessions.clone(), 600));
    let llm = Arc::new(llm);
    let agent = Agent::with_config(AgentConfig::new(
        sessions.clone(),
        Arc::new(PermissionManager::new(dir.path().join("permissions"))),
        None,
        BioRouterMode::Auto,
    ));
    let session = sessions
        .create_session(dir.path().to_path_buf(), "batch".into(), SessionType::User)
        .await
        .unwrap();
    agent.set_decision_service(decision_service);
    agent
        .update_provider(llm.clone() as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();
    let agent = Arc::new(agent);
    let _ = llm.agent.set(agent.clone());
    let _ = llm.session_id.set(session.id.clone());
    let chat = Chat {
        agent,
        sessions,
        session_id: session.id,
        _dir: dir,
    };

    let messages = send(&chat, "Find the MS cohort in these notes.").await;

    assert_eq!(decisions.requests(), 600);
    let marker = markers(&messages);
    assert_eq!(marker.len(), 1, "600 calls, one marker");
    assert_eq!(marker[0].items.len(), 1);
    assert_eq!(marker[0].items[0].origin, DecisionOrigin::Tool);
    assert_eq!(marker[0].items[0].count, 600);
    assert_eq!(
        count_containing(&notices(&messages), "Decision models: decide: 600 chunks"),
        1
    );
    // Nothing about the calls reached the transcript but that one marker.
    let stored = stored(&chat).await;
    assert_eq!(markers(&stored).len(), 1);
    assert!(
        stored.len() <= 4,
        "the transcript holds the prompt, the reply and the marker, not the calls: {}",
        stored.len()
    );
}
