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
//! reaches the model, and one that fires beside a stop waits for the turn the
//! person carries on; a steer the person typed beats a stop; a chat with no
//! monitors asks nothing; a private chat never reaches a public monitor model;
//! the error policies; a monitor at its per-turn check limit says so once, where
//! a check is really skipped, and a fail-closed one stops the turn there (but
//! not a turn that ends at its last check, a structured final output included,
//! not where another monitor stops the turn, and not before its own note or
//! another monitor's reached the agent); an elicitation answer leaves the
//! running turn's decision turn alone; and a batch of 600 decisions in one turn
//! is ONE marker.
//!
//! And, since the monitors are read at every point: a monitor added, edited or
//! removed while a turn runs applies at that turn's next point; a stop monitor
//! with no re-arm level stops every turn while its answer holds; a stopped
//! turn leaves the next turn's model a note saying why; nothing is skipped in
//! silence (unreadable settings, a check made while the person's message
//! waits, a slow one, a text-only window over a tool run, a subagent's
//! `turn_end` point, a wound-down turn); the latches survive a restart; a
//! monitor's stop stops the subagents the chat started, and waits for them
//! only so long; and a stop made by a copy of a person's monitor in a subagent
//! stops its siblings and the chat that started it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::final_output_tool::FINAL_OUTPUT_TOOL_NAME;
use biorouter::agents::subagent_handle::{self, BackgroundSubagent, ChildStopCause};
use biorouter::agents::subagent_result::SubagentResult;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, ReplyBudget, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{
    Message, MessageContent, MessageProvenance, ProvenanceKind, SteerOutcome,
};
use biorouter::decision::log::DecisionTurnMarker;
use biorouter::decision::model::DecisionCaps;
use biorouter::decision::monitor::{EscalatedStop, MonitorRegistry, Step};
use biorouter::decision::settings::{
    DecisionSettings, MonitorAction, MonitorCriterion, MonitorDirection, MonitorErrorPolicy,
    MonitorOwner, MonitorStopReason, MonitorTiming, MonitorWindow,
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
use tokio_util::sync::CancellationToken;

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
    /// How long each request takes, in order; none once the list runs out.
    delays: Mutex<VecDeque<Duration>>,
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
            delays: Mutex::new(VecDeque::new()),
        })
    }

    /// The answers to the first requests, in order; `otherwise` after them.
    fn answering(self: Arc<Self>, answers: &[Answer]) -> Arc<Self> {
        self.answers.lock().unwrap().extend(answers.iter().copied());
        self
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
        let delay = self.0.delays.lock().unwrap().pop_front();
        if let Some(delay) = delay {
            tokio::time::sleep(delay).await;
        }
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
    /// A workflow's structured final output: a call to the final-output tool
    /// with this answer, which [`final_output_schema`] accepts.
    FinalOutput(&'static str),
}

/// The response schema [`Reply::FinalOutput`] answers.
fn final_output_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {"answer": {"type": "string"}},
        "required": ["answer"]
    })
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
    /// Saved as the chat's monitors during the call with this index, as a
    /// person (or another process) would while the turn runs. An empty list
    /// removes them all.
    monitors_on_call: Option<(usize, Vec<MonitorCriterion>)>,
    /// Run during the call with this index.
    hook_on_call: Option<(usize, Arc<dyn Fn() + Send + Sync>)>,
    sessions: OnceLock<Arc<SessionManager>>,
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
            monitors_on_call: None,
            hook_on_call: None,
            sessions: OnceLock::new(),
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
        if let Some((_, monitors)) = self.monitors_on_call.as_ref().filter(|(at, _)| *at == n) {
            let sessions = self.sessions.get().expect("a chat's store");
            let session_id = self.session_id.get().expect("a session");
            biorouter::decision::settings::save(
                sessions,
                session_id,
                DecisionSettings {
                    default_model: None,
                    monitors: monitors.clone(),
                },
            )
            .await
            .expect("valid monitors are stored mid-turn");
        }
        if let Some((_, hook)) = self.hook_on_call.as_ref().filter(|(at, _)| *at == n) {
            hook();
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
            Reply::FinalOutput(answer) => Message::assistant().with_tool_request(
                format!("call-{n}"),
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: FINAL_OUTPUT_TOOL_NAME.into(),
                    arguments: Some(rmcp::object!({"answer": answer})),
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
    chat_of(SessionType::User, llm, decisions, classification, monitors).await
}

/// [`chat`], for a chat of `session_type` (a subagent's, say).
async fn chat_of(
    session_type: SessionType,
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
            session_type,
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
    let _ = llm.sessions.set(sessions.clone());
    Chat {
        agent,
        sessions,
        session_id: session.id,
        _dir: dir,
    }
}

/// A restart: a new agent over the same store, asking the same decision
/// models and the same chat model, as the daemon's would after it came back.
/// Nothing of the old agent's memory carries over.
async fn restart(chat: &mut Chat, llm: &Arc<Llm>, decisions: &Arc<Decisions>) {
    let agent = Agent::with_config(AgentConfig::new(
        chat.sessions.clone(),
        Arc::new(PermissionManager::new(chat._dir.path().join("permissions"))),
        None,
        BioRouterMode::Auto,
    ));
    agent.set_decision_service(service(decisions));
    agent
        .update_provider(llm.clone() as Arc<dyn Provider>, &chat.session_id)
        .await
        .unwrap();
    chat.agent = Arc::new(agent);
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
    send_with(chat, text, config(chat)).await
}

async fn send_with(chat: &Chat, text: &str, config: SessionConfig) -> Vec<Message> {
    let stream = chat
        .agent
        .reply(Message::user().with_text(text), config, None)
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
        direction: Default::default(),
        rearm_above: None,
        set_by: Default::default(),
        set_by_session: None,
        inherited_from: None,
        apply_to_subagents: false,
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
    // Was 1: the point at which the person's message waited was dropped, so
    // nothing was checked there. A point now checks within its grace and
    // holds the stop for the person's message; the second check, after the
    // message was answered, makes the stop. The person's message still wins
    // (it was answered, call 1), which is what this test is about.
    assert_eq!(
        decisions.requests(),
        2,
        "the monitor was checked while the person's message waited, and again after it"
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

const LIMIT_NOTICE: &str = "Monitor 'Stuck in a loop' has been checked 2 times this turn, its \
                            limit, so it stops checking until the next turn. Raise its check \
                            limit (maxEvalsPerTurn, up to 200) to watch longer turns.";

/// A monitor that reaches its per-turn check limit in a long turn says so,
/// once, instead of going quiet; the turn goes on, and the next turn checks
/// again.
#[tokio::test(flavor = "multi_thread")]
async fn a_monitor_at_its_check_limit_says_so_once_and_the_turn_goes_on() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![
            Reply::Tool,
            Reply::Tool,
            Reply::Tool,
            Reply::Text("All done."),
            Reply::Tool,
            Reply::Tool,
        ],
        Reply::Text("Submitted."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
    let mut looping = monitor("loop", "Stuck in a loop", MonitorAction::Stop, 0.9);
    looping.max_evals_per_turn = 2;
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![looping],
    )
    .await;

    let messages = send(&chat, "Fill in the consent form.").await;

    assert_eq!(llm.calls(), 4, "the limit does not end the turn");
    assert_eq!(decisions.requests(), 2, "two checks, its limit");
    let shown = notices(&messages);
    assert_eq!(count_containing(&shown, LIMIT_NOTICE), 1, "{shown:?}");
    assert_eq!(count_containing(&shown, "has been checked"), 1, "{shown:?}");
    assert_eq!(
        count_containing(&shown, "the turn stops here"),
        0,
        "{shown:?}"
    );
    let marker = markers(&messages);
    assert_eq!(marker.len(), 1);
    assert_eq!(marker[0].items[0].count, 2);
    assert!(!marker[0].items[0].fired);
    assert_eq!(
        count_containing(&notices(&stored(&chat).await), LIMIT_NOTICE),
        1,
        "the notice is stored, not live-only"
    );

    let again = send(&chat, "Now submit it.").await;
    assert_eq!(llm.calls(), 7);
    assert_eq!(decisions.requests(), 4, "the next turn checks again");
    assert_eq!(count_containing(&notices(&again), LIMIT_NOTICE), 1);
}

/// A fail-closed monitor (`onError: pause`) that can no longer check stops the
/// turn, the resumable way its errors do, instead of failing open.
#[tokio::test(flavor = "multi_thread")]
async fn a_fail_closed_monitor_at_its_check_limit_stops_the_turn() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
    let mut looping = monitor("loop", "Stuck in a loop", MonitorAction::Notify, 0.9);
    looping.max_evals_per_turn = 2;
    looping.on_error = MonitorErrorPolicy::Pause;
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![looping],
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
    let mut messages = Vec::new();
    let mut steered = None;
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event.expect("a reply event") {
            if message.metadata.decision_summary.is_some() {
                steered = Some(
                    chat.agent
                        .try_queue_soft_interrupt("Carry on with the form.".into(), None),
                );
            }
            messages.push(message);
        }
    }

    assert_eq!(
        llm.calls(),
        2,
        "the turn stopped after the second check; the third request was never sent"
    );
    assert_eq!(decisions.requests(), 2);
    let shown = notices(&messages);
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Stuck in a loop' has been checked 2 times this turn, its limit. It is set \
             to pause when it cannot check, so the turn stops here. Send a message to carry on."
        ),
        1,
        "{shown:?}"
    );
    assert_eq!(
        count_containing(&shown, "the turn stops here"),
        1,
        "{shown:?}"
    );
    let marker = markers(&messages);
    assert_eq!(marker.len(), 1);
    assert_eq!(marker[0].items[0].count, 2);
    // Resumable, as the other safety stops: the queue stays open.
    let steered = steered.expect("the turn wrote its marker");
    assert!(
        steered.is_ok(),
        "the steer was refused after the stop: {steered:?}"
    );
}

/// `send`, offering a steer the person typed as the turn's marker arrives,
/// once the loop has ended: whether it was accepted says how the turn ended,
/// as a stop that waits for the person (accepted) or the ordinary way, its
/// queue closed (refused). `None` when the turn wrote no marker.
async fn send_steering_at_marker(chat: &Chat, text: &str) -> (Vec<Message>, Option<bool>) {
    let stream = chat
        .agent
        .reply(Message::user().with_text(text), config(chat), None)
        .await
        .expect("a reply stream");
    tokio::pin!(stream);
    let mut messages = Vec::new();
    let mut accepted = None;
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event.expect("a reply event") {
            if message.metadata.decision_summary.is_some() {
                accepted = Some(
                    chat.agent
                        .try_queue_soft_interrupt("Carry on with the form.".into(), None)
                        .is_ok(),
                );
            }
            messages.push(message);
        }
    }
    (messages, accepted)
}

/// The check that reaches a monitor's limit skips nothing when the model's
/// reply ends the turn there: no notice, and a fail-closed monitor does not
/// stop a turn that has finished, which ends the ordinary way.
#[tokio::test(flavor = "multi_thread")]
async fn a_monitor_whose_last_check_ends_the_turn_says_nothing_of_its_limit() {
    for on_error in [MonitorErrorPolicy::Notify, MonitorErrorPolicy::Pause] {
        let llm = Arc::new(Llm::new(
            ProviderTier::Public,
            vec![Reply::Tool],
            Reply::Text("All done."),
        ));
        let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
        let mut looping = monitor("loop", "Stuck in a loop", MonitorAction::Stop, 0.9);
        looping.max_evals_per_turn = 2;
        looping.on_error = on_error;
        let chat = chat(
            llm.clone(),
            &decisions,
            SessionClassification::Public,
            vec![looping],
        )
        .await;

        let (messages, steered) = send_steering_at_marker(&chat, "Fill in the consent form.").await;

        assert_eq!(llm.calls(), 2, "{on_error:?}");
        assert_eq!(decisions.requests(), 2, "both checks ran: {on_error:?}");
        let shown = notices(&messages);
        assert_eq!(
            count_containing(&shown, "has been checked"),
            0,
            "{on_error:?}: {shown:?}"
        );
        assert_eq!(
            count_containing(&shown, "the turn stops here"),
            0,
            "{on_error:?}: {shown:?}"
        );
        let marker = markers(&messages);
        assert_eq!(marker.len(), 1);
        assert_eq!(marker[0].items[0].count, 2);
        assert!(!marker[0].items[0].fired, "{on_error:?}");
        assert_eq!(
            steered,
            Some(false),
            "{on_error:?}: the turn ended the ordinary way, its queue closed"
        );
    }
}

/// A `turn_end` monitor with a check limit of 1 is checked once a turn, as
/// the turn finishes. That skips no check, so no turn hears of the limit and
/// a fail-closed one stops none.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_end_monitor_checked_once_a_turn_says_nothing_of_its_limit() {
    for on_error in [MonitorErrorPolicy::Notify, MonitorErrorPolicy::Pause] {
        let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Text("Done.")));
        let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
        let mut check = monitor("check", "Task done", MonitorAction::Stop, 0.9);
        check.timing = MonitorTiming::TurnEnd;
        check.max_evals_per_turn = 1;
        check.on_error = on_error;
        let chat = chat(
            llm.clone(),
            &decisions,
            SessionClassification::Public,
            vec![check],
        )
        .await;

        for (turn, text) in [(1, "Summarise the notes."), (2, "Now the next note.")] {
            let messages = send(&chat, text).await;
            assert_eq!(llm.calls(), turn, "{on_error:?}");
            assert_eq!(decisions.requests(), turn, "{on_error:?}");
            let shown = notices(&messages);
            assert_eq!(
                count_containing(&shown, "has been checked"),
                0,
                "{on_error:?}: {shown:?}"
            );
            assert_eq!(
                count_containing(&shown, "the turn stops here"),
                0,
                "{on_error:?}: {shown:?}"
            );
        }
    }
}

/// A fail-closed `turn_end` monitor at its limit cannot check a turn its own
/// note reopened: the agent reads the note, and at the next `turn_end` point,
/// which would have checked it, the monitor says so and stops the turn.
#[tokio::test(flavor = "multi_thread")]
async fn a_fail_closed_turn_end_monitor_at_its_limit_stops_a_turn_its_note_reopened() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Text("Done."), Reply::Text("Done, and checked.")],
        Reply::Text("Extra."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let mut check = monitor("check", "Task done", MonitorAction::Steer, 0.5);
    check.timing = MonitorTiming::TurnEnd;
    check.max_evals_per_turn = 1;
    check.on_error = MonitorErrorPolicy::Pause;
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![check],
    )
    .await;

    let (messages, steered) = send_steering_at_marker(&chat, "Summarise the notes.").await;

    assert_eq!(llm.calls(), 2, "the model answered the note once");
    assert_eq!(decisions.requests(), 1, "one check, its limit");
    assert!(
        llm.texts_seen_on_call(1)
            .iter()
            .any(|t| t.contains("<decision-monitor>")),
        "the note reached the model: {:?}",
        llm.texts_seen_on_call(1)
    );
    let shown = notices(&messages);
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Task done' fired (P 0.90, threshold 0.50) and told the agent."
        ),
        1,
        "{shown:?}"
    );
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Task done' has been checked 1 time this turn, its limit. It is set to pause \
             when it cannot check, so the turn stops here. Send a message to carry on."
        ),
        1,
        "{shown:?}"
    );
    assert_eq!(
        count_containing(&shown, "the turn stops here"),
        1,
        "{shown:?}"
    );
    let marker = markers(&messages);
    assert_eq!(marker.len(), 1);
    assert_eq!(marker[0].items[0].count, 1);
    assert!(marker[0].items[0].fired);
    assert_eq!(steered, Some(true), "a resumable stop keeps the queue open");
}

/// A fail-closed `steer` monitor that fires on its last check tells the agent,
/// as its notice says, and stops the turn at the next point, which would have
/// checked it, before the model's next request.
#[tokio::test(flavor = "multi_thread")]
async fn a_fail_closed_steer_monitor_at_its_limit_tells_the_agent_before_it_stops() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let mut looping = monitor("loop", "Stuck in a loop", MonitorAction::Steer, 0.5);
    looping.max_evals_per_turn = 1;
    looping.on_error = MonitorErrorPolicy::Pause;
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![looping],
    )
    .await;

    let messages = send(&chat, "Fill in the consent form.").await;

    assert_eq!(
        llm.calls(),
        2,
        "the agent read the note; the third request was never sent"
    );
    assert_eq!(decisions.requests(), 1);
    assert!(
        llm.texts_seen_on_call(1)
            .iter()
            .any(|t| t.contains("<decision-monitor>")),
        "the note reached the model: {:?}",
        llm.texts_seen_on_call(1)
    );
    let shown = notices(&messages);
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Stuck in a loop' fired (P 0.90, threshold 0.50) and told the agent."
        ),
        1,
        "{shown:?}"
    );
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Stuck in a loop' has been checked 1 time this turn, its limit. It is set \
             to pause when it cannot check, so the turn stops here."
        ),
        1,
        "{shown:?}"
    );
    assert_eq!(
        count_containing(&shown, "the turn stops here"),
        1,
        "{shown:?}"
    );
}

/// A monitor whose last check comes where another monitor stops the turn
/// skipped no check: the stop is the turn's one notice, under either error
/// policy, and the capped monitor is not marked as having acted.
#[tokio::test(flavor = "multi_thread")]
async fn a_monitor_capped_where_another_stops_the_turn_says_nothing_of_its_limit() {
    for on_error in [MonitorErrorPolicy::Notify, MonitorErrorPolicy::Pause] {
        let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
        let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
        let mut capped = monitor("loop", "Stuck in a loop", MonitorAction::Stop, 0.95);
        capped.max_evals_per_turn = 1;
        capped.on_error = on_error;
        let chat = chat(
            llm.clone(),
            &decisions,
            SessionClassification::Public,
            vec![
                capped,
                monitor("form", "Form reached", MonitorAction::Stop, 0.85),
            ],
        )
        .await;

        let messages = send(&chat, "Fill in the consent form.").await;

        assert_eq!(llm.calls(), 1, "{on_error:?}");
        assert_eq!(
            decisions.requests(),
            1,
            "one request asks both: {on_error:?}"
        );
        let shown = notices(&messages);
        assert_eq!(
            count_containing(
                &shown,
                "Monitor 'Form reached' fired (P 0.90, threshold 0.85)"
            ),
            1,
            "{on_error:?}: {shown:?}"
        );
        assert_eq!(
            count_containing(&shown, "has been checked"),
            0,
            "{on_error:?}: {shown:?}"
        );
        assert_eq!(
            count_containing(&shown, "the turn stops here"),
            1,
            "{on_error:?}: {shown:?}"
        );
        let items = &markers(&messages)[0].items;
        assert_eq!(items.len(), 2);
        let fired: Vec<(String, bool)> = items
            .iter()
            .map(|item| (item.label.clone(), item.fired))
            .collect();
        assert_eq!(
            fired,
            vec![
                ("Stuck in a loop".to_string(), false),
                ("Form reached".to_string(), true)
            ],
            "{on_error:?}"
        );
    }
}

/// A fail-closed monitor whose last check comes where another monitor steers
/// the agent does not stop the turn there: the note is stored and the agent
/// reads it, as the steer's notice says, and the next point, which would have
/// checked the capped monitor, stops the turn.
#[tokio::test(flavor = "multi_thread")]
async fn a_fail_closed_cap_beside_another_monitors_steer_lets_the_agent_read_the_note() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let mut capped = monitor("safety", "Safety check", MonitorAction::Notify, 0.95);
    capped.max_evals_per_turn = 1;
    capped.on_error = MonitorErrorPolicy::Pause;
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![
            capped,
            monitor("loop", "Stuck in a loop", MonitorAction::Steer, 0.5),
        ],
    )
    .await;

    let messages = send(&chat, "Fill in the consent form.").await;

    assert_eq!(
        llm.calls(),
        2,
        "the agent read the note; the third request was never sent"
    );
    assert_eq!(decisions.requests(), 1);
    assert!(
        llm.texts_seen_on_call(1)
            .iter()
            .any(|t| t.contains("<decision-monitor>")),
        "the note reached the model: {:?}",
        llm.texts_seen_on_call(1)
    );
    let shown = notices(&messages);
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Stuck in a loop' fired (P 0.90, threshold 0.50) and told the agent."
        ),
        1,
        "{shown:?}"
    );
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Safety check' has been checked 1 time this turn, its limit. It is set to \
             pause when it cannot check, so the turn stops here."
        ),
        1,
        "{shown:?}"
    );
    assert_eq!(
        count_containing(&shown, "the turn stops here"),
        1,
        "{shown:?}"
    );
    let stored = stored(&chat).await;
    assert!(
        stored
            .iter()
            .any(|m| m.as_concat_text().contains("name: \"Stuck in a loop\"")),
        "the note is stored"
    );
}

/// A `steer` monitor that fires where a `stop` monitor ends the turn is held
/// back: the agent is never told in a turn that stops, so no note is stored
/// and no notice says the agent was told. The monitor stays armed, and in the
/// turn the person carries on its first check tells the agent.
#[tokio::test(flavor = "multi_thread")]
async fn a_steer_beside_a_stop_tells_the_agent_in_the_turn_carried_on() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool, Reply::Tool],
        Reply::Text("Done."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    // The stop monitor opts into stopping once (a re-arm level). Without one
    // it would stop the carried-on turn too while P holds, which is now the
    // default (`a_stop_monitor_stops_again_after_the_person_carries_on`);
    // this test is about the steer held beside it.
    let mut form = monitor("form", "Form reached", MonitorAction::Stop, 0.85);
    form.rearm_below = Some(0.3);
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![
            form,
            monitor("loop", "Stuck in a loop", MonitorAction::Steer, 0.5),
        ],
    )
    .await;
    let told = "Monitor 'Stuck in a loop' fired (P 0.90, threshold 0.50) and told the agent.";

    let first = send(&chat, "Fill in the consent form.").await;

    assert_eq!(llm.calls(), 1, "the stop ended the turn");
    assert_eq!(decisions.requests(), 1, "one request asks both");
    let shown = notices(&first);
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Form reached' fired (P 0.90, threshold 0.85), so the turn stops here"
        ),
        1,
        "{shown:?}"
    );
    assert_eq!(count_containing(&shown, told), 0, "{shown:?}");
    assert!(
        !stored(&chat)
            .await
            .iter()
            .any(|m| m.as_concat_text().contains("<decision-monitor>")),
        "no note is stored for an agent that never reads it"
    );
    let fired: Vec<(String, bool)> = markers(&first)[0]
        .items
        .iter()
        .map(|item| (item.label.clone(), item.fired))
        .collect();
    assert_eq!(
        fired,
        vec![
            ("Form reached".to_string(), true),
            ("Stuck in a loop".to_string(), false)
        ]
    );

    let second = send(&chat, "Carry on.").await;

    assert_eq!(
        llm.calls(),
        3,
        "the stop monitor stays latched; the agent read the note"
    );
    assert!(
        llm.texts_seen_on_call(2)
            .iter()
            .any(|t| t.contains("<decision-monitor>")),
        "the note reached the model: {:?}",
        llm.texts_seen_on_call(2)
    );
    let shown = notices(&second);
    assert_eq!(count_containing(&shown, told), 1, "{shown:?}");
    assert_eq!(
        count_containing(&shown, "the turn stops here"),
        0,
        "{shown:?}"
    );
}

/// A workflow with a response schema ends its turn with a call to the
/// final-output tool, not with a reply that sets the turn's exit, and nothing
/// is checked after that call. A monitor whose last check comes there skipped
/// no check: no notice, no stop, and the structured output is given now, not
/// after the person's next message.
#[tokio::test(flavor = "multi_thread")]
async fn a_last_check_before_a_structured_final_output_says_nothing_of_its_limit() {
    for on_error in [MonitorErrorPolicy::Notify, MonitorErrorPolicy::Pause] {
        let llm = Arc::new(Llm::new(
            ProviderTier::Public,
            vec![Reply::Tool, Reply::FinalOutput("42")],
            Reply::Text("Extra."),
        ));
        let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
        let mut looping = monitor("loop", "Stuck in a loop", MonitorAction::Stop, 0.9);
        looping.max_evals_per_turn = 2;
        looping.on_error = on_error;
        let chat = chat(
            llm.clone(),
            &decisions,
            SessionClassification::Public,
            vec![looping],
        )
        .await;
        chat.agent
            .add_final_output_tool(biorouter::workflow::Response {
                json_schema: Some(final_output_schema()),
            })
            .await;

        let messages = send(&chat, "Answer in the schema.").await;

        assert_eq!(llm.calls(), 2, "{on_error:?}");
        assert_eq!(decisions.requests(), 2, "both checks ran: {on_error:?}");
        let shown = notices(&messages);
        assert_eq!(
            count_containing(&shown, "has been checked"),
            0,
            "{on_error:?}: {shown:?}"
        );
        assert_eq!(
            count_containing(&shown, "the turn stops here"),
            0,
            "{on_error:?}: {shown:?}"
        );
        assert!(
            messages
                .iter()
                .any(|m| m.role == rmcp::model::Role::Assistant
                    && m.as_concat_text().contains(r#"{"answer":"42"}"#)),
            "{on_error:?}: the structured output was given"
        );
        let marker = markers(&messages);
        assert_eq!(marker.len(), 1);
        assert_eq!(marker[0].items[0].count, 2);
        assert!(!marker[0].items[0].fired, "{on_error:?}");
    }
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

// ---------------------------------------------------------------------------
// Monitors read at every point, and what a stop leaves behind.
// ---------------------------------------------------------------------------

/// The frame of the note a stopped turn leaves its model.
const STOP_NOTE: &str = "<decision-monitor-stop>";

/// The stored rows the model reads and the person is never shown, holding
/// `needle`.
fn model_only_rows<'a>(messages: &'a [Message], needle: &str) -> Vec<&'a Message> {
    messages
        .iter()
        .filter(|m| !m.is_user_visible() && m.is_agent_visible())
        .filter(|m| m.as_concat_text().contains(needle))
        .collect()
}

/// Where the first of `messages` that `matches` sits.
fn position(messages: &[Message], matches: impl Fn(&Message) -> bool) -> usize {
    messages
        .iter()
        .position(matches)
        .expect("the message is stored")
}

/// Whether `message` shows the person a notice holding `needle`.
fn shows(message: &Message, needle: &str) -> bool {
    notices(std::slice::from_ref(message))
        .iter()
        .any(|n| n.contains(needle))
}

const FORM_STOPS: &str =
    "Monitor 'Form reached' fired (P 0.95, threshold 0.85), so the turn stops here.";

/// A stop monitor with no re-arm level stops every turn in which its answer
/// holds (PR #469's rule): the person carries on, and the first check that
/// still holds stops the turn again, as its notice says it will.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_monitor_stops_again_after_the_person_carries_on() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    )
    .await;

    let first = send(&chat, "Fill in the form.").await;
    let second = send(&chat, "Carry on.").await;

    assert_eq!(llm.calls(), 2, "each turn stopped at its first check");
    assert_eq!(decisions.requests(), 2);
    assert_eq!(count_containing(&notices(&first), FORM_STOPS), 1);
    let shown = notices(&second);
    assert_eq!(count_containing(&shown, FORM_STOPS), 1, "{shown:?}");
    assert_eq!(
        count_containing(&shown, "stops the turn again while its answer holds"),
        1,
        "{shown:?}"
    );
}

/// With a re-arm level a stop monitor stops once and stays quiet until P
/// falls below it, so the turn the person carries on runs to its end.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_monitor_with_a_rearm_level_does_not_stop_the_turn_carried_on() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool],
        Reply::Text("Done."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let mut form = monitor("form", "Form reached", MonitorAction::Stop, 0.85);
    form.rearm_below = Some(0.3);
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![form],
    )
    .await;

    let first = send(&chat, "Fill in the form.").await;
    let second = send(&chat, "Carry on.").await;

    assert_eq!(llm.calls(), 2);
    assert_eq!(decisions.requests(), 2, "checked in both turns");
    let shown = notices(&first);
    assert_eq!(count_containing(&shown, FORM_STOPS), 1, "{shown:?}");
    assert_eq!(
        count_containing(&shown, "It stays quiet until P falls below 0.30."),
        1,
        "{shown:?}"
    );
    assert_eq!(
        count_containing(&notices(&second), "so the turn stops here"),
        0
    );
    assert!(second
        .iter()
        .any(|m| m.role == rmcp::model::Role::Assistant && m.as_concat_text().contains("Done.")));
}

/// A monitor saved while a turn runs (from another window, the CLI, the
/// agent that started this chat) is checked at that turn's next point, not
/// from the next turn.
#[tokio::test(flavor = "multi_thread")]
async fn a_monitor_added_while_the_turn_runs_is_checked_at_the_next_point() {
    let mut llm = Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool, Reply::Tool, Reply::Tool],
        Reply::Text("All done."),
    );
    llm.monitors_on_call = Some((
        1,
        vec![monitor("form", "Form reached", MonitorAction::Notify, 0.5)],
    ));
    let llm = Arc::new(llm);
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![],
    )
    .await;

    let messages = send(&chat, "Fill in the consent form.").await;

    assert_eq!(llm.calls(), 4);
    assert_eq!(
        decisions.requests(),
        3,
        "checked at each point after it was saved, and none before"
    );
    assert_eq!(
        count_containing(&notices(&messages), "Monitor 'Form reached' fired"),
        1
    );
    let marker = markers(&messages);
    assert_eq!(marker.len(), 1, "the running turn's marker counts it");
    assert_eq!(marker[0].items.len(), 1);
    assert_eq!(marker[0].items[0].count, 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_monitor_added_mid_turn_stops_that_turn() {
    let mut llm = Llm::new(ProviderTier::Public, vec![], Reply::Tool);
    llm.monitors_on_call = Some((
        1,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    ));
    let llm = Arc::new(llm);
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![],
    )
    .await;

    let messages = send(&chat, "Fill in the consent form.").await;

    assert_eq!(
        llm.calls(),
        2,
        "the point after the call that saved it stopped the turn"
    );
    assert_eq!(decisions.requests(), 1);
    assert_eq!(count_containing(&notices(&messages), FORM_STOPS), 1);
}

/// A monitor edited while the turn runs is asked its new question at the next
/// point.
#[tokio::test(flavor = "multi_thread")]
async fn a_monitor_edited_mid_turn_is_asked_its_new_question() {
    let mut edited = monitor("form", "Form reached", MonitorAction::Notify, 0.95);
    edited.question.instructions = "Has the agent submitted the consent form?".into();
    let mut llm = Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool, Reply::Tool],
        Reply::Text("All done."),
    );
    llm.monitors_on_call = Some((1, vec![edited]));
    let llm = Arc::new(llm);
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Notify, 0.95)],
    )
    .await;

    send(&chat, "Fill in the consent form.").await;

    let asked: Vec<String> = decisions
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.questions["form"].instructions.clone())
        .collect();
    assert_eq!(
        asked,
        vec![
            "Has the agent reached the consent form page?".to_string(),
            "Has the agent submitted the consent form?".to_string(),
            "Has the agent submitted the consent form?".to_string(),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_monitor_removed_mid_turn_is_not_asked_again() {
    let mut llm = Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool, Reply::Tool, Reply::Tool],
        Reply::Text("All done."),
    );
    llm.monitors_on_call = Some((1, vec![]));
    let llm = Arc::new(llm);
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Notify, 0.5)],
    )
    .await;

    let messages = send(&chat, "Fill in the consent form.").await;

    assert_eq!(llm.calls(), 4);
    assert_eq!(
        decisions.requests(),
        1,
        "asked once, before the save that removed it"
    );
    let marker = markers(&messages);
    assert_eq!(marker.len(), 1, "the check it made is still the turn's");
    assert_eq!(marker[0].items[0].count, 1);
}

/// A stopped turn leaves its model a note, stored but never shown to the
/// person, naming the monitor and why it stopped, between the person's
/// notice and the turn's marker. The next turn's model reads it.
#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_turn_tells_the_agent_why_on_the_next_turn() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool],
        Reply::Text("Done."),
    ));
    let decisions =
        Decisions::new(ProviderTier::Private, Answer::Yes(0.1)).answering(&[Answer::Yes(0.95)]);
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    )
    .await;

    let first = send(&chat, "Fill in the form.").await;

    assert_eq!(llm.calls(), 1);
    let stored_rows = stored(&chat).await;
    let notes = model_only_rows(&stored_rows, STOP_NOTE);
    assert_eq!(notes.len(), 1, "one note for the one stop");
    let note = notes[0].as_concat_text();
    assert!(note.contains("name: \"Form reached\""), "{note}");
    assert!(
        note.contains(
            "reason: fired: the probability of the answer it watches for was 0.95 (threshold \
             0.85)"
        ),
        "{note}"
    );
    assert!(
        !notices(&first).iter().any(|n| n.contains(STOP_NOTE)),
        "the person is never shown it"
    );
    let notice_at = position(&stored_rows, |m| shows(m, "so the turn stops here"));
    let note_at = position(&stored_rows, |m| m.as_concat_text().contains(STOP_NOTE));
    let marker_at = position(&stored_rows, |m| m.metadata.decision_summary.is_some());
    assert!(
        notice_at < note_at && note_at < marker_at,
        "notice {notice_at}, note {note_at}, marker {marker_at}"
    );
    // The stop is the turn's, for the chat that started it to read, once.
    let report = chat
        .agent
        .take_monitor_stop(&chat.session_id)
        .expect("the turn's stop");
    assert_eq!(report.name, "Form reached");
    assert_eq!(report.reason, MonitorStopReason::Fired);
    assert_eq!(report.p, Some(0.95));
    assert!(chat.agent.take_monitor_stop(&chat.session_id).is_none());

    send(&chat, "Carry on.").await;

    assert!(
        llm.texts_seen_on_call(1)
            .iter()
            .any(|t| t.contains("stopped here by a decision model")),
        "the next turn's model read why: {:?}",
        llm.texts_seen_on_call(1)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fail_closed_stop_tells_the_agent_the_check_failed() {
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

    send(&chat, "Go.").await;

    assert_eq!(llm.calls(), 1);
    let stored_rows = stored(&chat).await;
    let notes = model_only_rows(&stored_rows, STOP_NOTE);
    assert_eq!(notes.len(), 1);
    let note = notes[0].as_concat_text();
    assert!(note.contains("name: \"Safety check\""), "{note}");
    assert!(
        note.contains("reason: could not be checked (")
            && note.contains("it is set to stop the turn when that happens"),
        "{note}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cap_stop_tells_the_agent_its_limit() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
    let mut looping = monitor("loop", "Stuck in a loop", MonitorAction::Notify, 0.9);
    looping.max_evals_per_turn = 2;
    looping.on_error = MonitorErrorPolicy::Pause;
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![looping],
    )
    .await;

    send(&chat, "Fill in the consent form.").await;

    assert_eq!(llm.calls(), 2);
    let stored_rows = stored(&chat).await;
    let notes = model_only_rows(&stored_rows, STOP_NOTE);
    assert_eq!(notes.len(), 1);
    let note = notes[0].as_concat_text();
    assert!(
        note.contains("reason: reached its limit of 2 checks this turn"),
        "{note}"
    );
}

/// A stop held back for the person's message is no stop: no notice, no
/// note. Here the check after the message was answered no longer fires.
#[tokio::test(flavor = "multi_thread")]
async fn a_held_stop_writes_no_note() {
    let mut llm = Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool],
        Reply::Text("Redone in R."),
    );
    llm.steer_on_first_call = Some("Actually, use R instead.");
    let llm = Arc::new(llm);
    let decisions =
        Decisions::new(ProviderTier::Private, Answer::Yes(0.1)).answering(&[Answer::Yes(0.95)]);
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    )
    .await;

    let messages = send(&chat, "Plot the data in Python.").await;

    assert_eq!(llm.calls(), 2, "the person's message was answered");
    assert_eq!(
        decisions.requests(),
        2,
        "checked while the message waited, and again after"
    );
    assert_eq!(
        count_containing(&notices(&messages), "the turn stops here"),
        0
    );
    assert!(model_only_rows(&stored(&chat).await, STOP_NOTE).is_empty());
    assert!(chat.agent.take_monitor_stop(&chat.session_id).is_none());
}

/// Stores `monitor` as a record this build cannot read: one unknown key.
async fn store_unreadable_monitors(chat: &Chat, monitor: MonitorCriterion) {
    let mut value = serde_json::to_value(DecisionSettings {
        default_model: None,
        monitors: vec![monitor],
    })
    .unwrap();
    value["monitors"][0]["bogus"] = serde_json::json!(1);
    chat.sessions
        .update_extension_state(
            &chat.session_id,
            biorouter::decision::settings::DECISION_SETTINGS_STATE,
            biorouter::decision::settings::DECISION_SETTINGS_VERSION,
            move |_| Ok(value),
        )
        .await
        .unwrap()
        .expect("the chat exists");
}

/// Monitors this build cannot read, one of them set to pause when it cannot
/// check, stop the turn rather than letting it run unwatched.
#[tokio::test(flavor = "multi_thread")]
async fn unreadable_monitors_naming_pause_stop_the_turn() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![],
    )
    .await;
    let mut pause = monitor("safety", "Safety check", MonitorAction::Notify, 0.5);
    pause.on_error = MonitorErrorPolicy::Pause;
    store_unreadable_monitors(&chat, pause).await;

    let messages = send(&chat, "Run echo 1, then echo 2.").await;

    assert_eq!(llm.calls(), 1);
    assert_eq!(decisions.requests(), 0, "nothing could be read to ask");
    let shown = notices(&messages);
    assert_eq!(
        shown
            .iter()
            .filter(|n| n.contains("This chat's monitors could not be read (")
                && n.contains("so the turn stops here"))
            .count(),
        1,
        "{shown:?}"
    );
    let stored_rows = stored(&chat).await;
    assert_eq!(
        model_only_rows(&stored_rows, "this chat's monitors could not be read").len(),
        1,
        "the next turn's model is told why"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unreadable_monitors_without_pause_say_so_once_and_the_turn_goes_on() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool, Reply::Tool],
        Reply::Text("Done."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![],
    )
    .await;
    store_unreadable_monitors(
        &chat,
        monitor("form", "Form reached", MonitorAction::Notify, 0.5),
    )
    .await;

    let messages = send(&chat, "Run echo 1, then echo 2.").await;

    assert_eq!(llm.calls(), 3, "the turn went on");
    let shown = notices(&messages);
    assert_eq!(
        count_containing(&shown, "so none are checked this turn"),
        1,
        "{shown:?}"
    );
    assert_eq!(
        count_containing(&shown, "could not be read"),
        1,
        "{shown:?}"
    );
    assert_eq!(count_containing(&shown, "the turn stops here"), 0);
}

/// A check made while a message the person typed waits (within the grace)
/// is made and folded in: its stop is held for the person's message, which
/// is answered, and the next check makes the stop.
#[tokio::test(flavor = "multi_thread")]
async fn a_check_made_while_the_persons_message_waits_holds_the_stop_until_it_is_answered() {
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

    send(&chat, "Plot the data in Python.").await;

    assert_eq!(llm.calls(), 2, "the person's message was answered");
    assert_eq!(decisions.requests(), 2);
    let stored_rows = stored(&chat).await;
    let steer_at = position(&stored_rows, |m| {
        m.as_concat_text().contains("Actually, use R instead.")
    });
    let stop_at = position(&stored_rows, |m| shows(m, "so the turn stops here"));
    assert!(
        steer_at < stop_at,
        "the stop waited for the person's message: steer {steer_at}, stop {stop_at}"
    );
    assert_eq!(model_only_rows(&stored_rows, STOP_NOTE).len(), 1);
}

/// A notify check made while the person's message waits is shown where it
/// was made, before that message, rather than dropped and remade after it.
#[tokio::test(flavor = "multi_thread")]
async fn a_notify_check_made_while_the_persons_message_waits_is_shown() {
    let mut llm = Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool],
        Reply::Text("Redone in R."),
    );
    llm.steer_on_first_call = Some("Actually, use R instead.");
    let llm = Arc::new(llm);
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Notify, 0.5)],
    )
    .await;

    let messages = send(&chat, "Plot the data in Python.").await;

    assert_eq!(llm.calls(), 2);
    assert_eq!(decisions.requests(), 2);
    assert_eq!(
        count_containing(&notices(&messages), "Monitor 'Form reached' fired"),
        1
    );
    let stored_rows = stored(&chat).await;
    let notice_at = position(&stored_rows, |m| shows(m, "Monitor 'Form reached' fired"));
    let steer_at = position(&stored_rows, |m| {
        m.as_concat_text().contains("Actually, use R instead.")
    });
    assert!(
        notice_at < steer_at,
        "shown where it was checked: notice {notice_at}, steer {steer_at}"
    );
}

/// A check slower than the grace gives way to the person's message: the
/// message is not held up for it, and the monitor is checked at the next
/// point.
#[tokio::test(flavor = "multi_thread")]
async fn a_slow_check_gives_way_to_the_persons_message_and_is_made_at_the_next_point() {
    let mut llm = Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool],
        Reply::Text("Redone in R."),
    );
    llm.steer_on_first_call = Some("Actually, use R instead.");
    let llm = Arc::new(llm);
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    decisions
        .delays
        .lock()
        .unwrap()
        .push_back(Duration::from_millis(500));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Notify, 0.5)],
    )
    .await;
    chat.agent
        .set_monitor_steer_grace(Duration::from_millis(50));

    let messages = send(&chat, "Plot the data in Python.").await;

    assert_eq!(llm.calls(), 2);
    assert_eq!(
        decisions.requests(),
        2,
        "the slow check was started, then made again at the next point"
    );
    assert_eq!(
        count_containing(&notices(&messages), "Monitor 'Form reached' fired"),
        1
    );
    let stored_rows = stored(&chat).await;
    let steer_at = position(&stored_rows, |m| {
        m.as_concat_text().contains("Actually, use R instead.")
    });
    let notice_at = position(&stored_rows, |m| shows(m, "Monitor 'Form reached' fired"));
    assert!(
        steer_at < notice_at,
        "the slow check gave way: steer {steer_at}, notice {notice_at}"
    );
}

/// A text-only window counts the messages with text, so a run of tool steps
/// never empties it, and every point of the run asks.
#[tokio::test(flavor = "multi_thread")]
async fn a_text_only_monitor_is_asked_every_step_of_a_tool_run() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool; 5],
        Reply::Text("Done."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
    let mut text_only = monitor("form", "Form reached", MonitorAction::Notify, 0.5);
    text_only.window = MonitorWindow {
        entries: 2,
        max_chars: 2000,
        tool_calls: false,
    };
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![text_only],
    )
    .await;

    let messages = send(&chat, "Run echo 1 through echo 5.").await;

    assert_eq!(llm.calls(), 6);
    assert_eq!(decisions.requests(), 6, "asked at every point");
    assert_eq!(
        count_containing(&notices(&messages), "not checked"),
        0,
        "{:?}",
        notices(&messages)
    );
}

/// A subagent's chat runs its `turn_end` monitors as its turn ends.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_end_monitor_runs_in_a_subagent_chat() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Text("Done.")));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let mut check = monitor("check", "Task done", MonitorAction::Stop, 0.5);
    check.timing = MonitorTiming::TurnEnd;
    let chat = chat_of(
        SessionType::SubAgent,
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![check],
    )
    .await;

    let messages = send(&chat, "Summarise the notes.").await;

    assert_eq!(llm.calls(), 1);
    assert_eq!(
        decisions.requests(),
        1,
        "checked as the subagent's turn ended"
    );
    assert_eq!(
        count_containing(
            &notices(&messages),
            "Monitor 'Task done' fired (P 0.90, threshold 0.50), so the turn stops here."
        ),
        1,
        "{:?}",
        notices(&messages)
    );
    let report = chat
        .agent
        .take_monitor_stop(&chat.session_id)
        .expect("the subagent's stop, for the chat that started it");
    assert_eq!(report.name, "Task done");
}

/// …and a `turn_end` steer there sends the subagent back to work once, with
/// the monitor's note.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_end_steer_monitor_continues_a_subagent_once() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Text("Done.")],
        Reply::Text("Done, and checked."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let mut check = monitor("check", "Task done", MonitorAction::Steer, 0.5);
    check.timing = MonitorTiming::TurnEnd;
    let chat = chat_of(
        SessionType::SubAgent,
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![check],
    )
    .await;

    send(&chat, "Summarise the notes.").await;

    assert_eq!(llm.calls(), 2, "the subagent answered the note once");
    assert!(
        llm.texts_seen_on_call(1)
            .iter()
            .any(|t| t.contains("<decision-monitor>")),
        "{:?}",
        llm.texts_seen_on_call(1)
    );
}

/// A `turn_end` point reached as the budget winds the turn down asks nothing
/// (a billed check would spend past the budget) and says which monitor it did
/// not check.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_end_monitor_skipped_by_a_budget_wrap_up_says_so() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool],
        Reply::Text("Done."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let mut check = monitor("check", "Task done", MonitorAction::Stop, 0.5);
    check.timing = MonitorTiming::TurnEnd;
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![check],
    )
    .await;
    let mut budgeted = config(&chat);
    // The chat model's first call spends 15 tokens, so the second iteration
    // finds the budget spent and the model is told to wrap up.
    budgeted.budget = Some(ReplyBudget {
        max_seconds: None,
        max_tokens: Some(10),
        max_usd: None,
    });

    let messages = send_with(&chat, "Summarise the notes.", budgeted).await;

    let shown = notices(&messages);
    assert_eq!(count_containing(&shown, "Budget reached"), 1, "{shown:?}");
    assert_eq!(decisions.requests(), 0, "nothing was asked past the budget");
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Task done' was not checked at the end of this turn: the turn was being \
             wound down because it reached its budget."
        ),
        1,
        "{shown:?}"
    );
    assert_eq!(count_containing(&shown, "the turn stops here"), 0);
}

/// A notify monitor that fired stays quiet in the next turn while its answer
/// holds, across a restart too: its latch is kept with the chat.
#[tokio::test(flavor = "multi_thread")]
async fn a_latched_notify_monitor_stays_quiet_after_a_restart() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Text("Done.")));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let mut chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Notify, 0.5)],
    )
    .await;

    let first = send(&chat, "Fill in the form.").await;
    assert_eq!(
        count_containing(&notices(&first), "Monitor 'Form reached' fired"),
        1
    );
    restart(&mut chat, &llm, &decisions).await;
    let second = send(&chat, "Again.").await;

    assert_eq!(decisions.requests(), 2, "still checked after the restart");
    assert_eq!(
        count_containing(&notices(&second), "Monitor 'Form reached' fired"),
        0,
        "the latch outlived the restart: {:?}",
        notices(&second)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stop_monitor_with_a_rearm_level_stays_latched_after_a_restart() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool],
        Reply::Text("Done."),
    ));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let mut form = monitor("form", "Form reached", MonitorAction::Stop, 0.85);
    form.rearm_below = Some(0.3);
    let mut chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![form],
    )
    .await;

    let first = send(&chat, "Fill in the form.").await;
    assert_eq!(count_containing(&notices(&first), FORM_STOPS), 1);
    restart(&mut chat, &llm, &decisions).await;
    let second = send(&chat, "Carry on.").await;

    assert_eq!(llm.calls(), 2);
    assert_eq!(
        count_containing(&notices(&second), "the turn stops here"),
        0,
        "{:?}",
        notices(&second)
    );
}

/// A latch is kept for what the monitor asked: a monitor edited after the
/// restart asks anew, and fires again.
#[tokio::test(flavor = "multi_thread")]
async fn an_edited_monitor_does_not_inherit_a_persisted_latch() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Text("Done.")));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let mut chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Notify, 0.5)],
    )
    .await;

    send(&chat, "Fill in the form.").await;
    restart(&mut chat, &llm, &decisions).await;
    let mut edited = monitor("form", "Form reached", MonitorAction::Notify, 0.5);
    edited.question.instructions = "Has the agent submitted the consent form?".into();
    biorouter::decision::settings::save(
        &chat.sessions,
        &chat.session_id,
        DecisionSettings {
            default_model: None,
            monitors: vec![edited],
        },
    )
    .await
    .unwrap();
    let second = send(&chat, "Again.").await;

    assert_eq!(
        count_containing(&notices(&second), "Monitor 'Form reached' fired"),
        1,
        "{:?}",
        notices(&second)
    );
}

/// The latch record is written by a turn's points only: never when the
/// chat's monitors are saved, which is not activity in the chat (R-30).
#[tokio::test(flavor = "multi_thread")]
async fn the_state_record_is_written_inside_the_turn_only() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Text("Done.")));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.9));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Notify, 0.5)],
    )
    .await;
    let record = || async {
        chat.sessions
            .get_extension_state_json(
                &chat.session_id,
                biorouter::decision::settings::DECISION_MONITOR_STATE,
                biorouter::decision::settings::DECISION_MONITOR_STATE_VERSION,
            )
            .await
            .unwrap()
    };
    assert_eq!(record().await, None, "saving the monitors wrote none");

    send(&chat, "Fill in the form.").await;

    let written = record().await.expect("the turn's point wrote the latch");
    let value: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(
        value["criteria"]["form"]["latched"],
        serde_json::json!(true)
    );
    assert_eq!(
        value["criteria"]["form"]["fingerprint"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    biorouter::decision::settings::save(
        &chat.sessions,
        &chat.session_id,
        DecisionSettings {
            default_model: None,
            monitors: vec![
                monitor("form", "Form reached", MonitorAction::Notify, 0.5),
                monitor("loop", "Stuck in a loop", MonitorAction::Notify, 0.9),
            ],
        },
    )
    .await
    .unwrap();
    assert_eq!(
        record().await.as_deref(),
        Some(written.as_str()),
        "a save outside a turn left the record alone"
    );
}

/// A monitor set to fire below its threshold fires when P falls under it,
/// and not before.
#[tokio::test(flavor = "multi_thread")]
async fn a_below_monitor_fires_when_p_falls() {
    let llm = Arc::new(Llm::new(
        ProviderTier::Public,
        vec![Reply::Tool, Reply::Tool],
        Reply::Text("Done."),
    ));
    let decisions =
        Decisions::new(ProviderTier::Private, Answer::Yes(0.1)).answering(&[Answer::Yes(0.9)]);
    let mut falling = monitor("form", "Form reached", MonitorAction::Notify, 0.3);
    falling.direction = MonitorDirection::Below;
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![falling],
    )
    .await;

    let messages = send(&chat, "Fill in the form.").await;

    assert_eq!(decisions.requests(), 3);
    let shown = notices(&messages);
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Form reached' fired (P 0.10, below its threshold 0.30)."
        ),
        1,
        "{shown:?}"
    );
    // One fire notice in all: the first check (P 0.90, above the threshold)
    // did not fire. The turn's marker also says "fired", so it is left out.
    assert_eq!(
        count_containing(&shown, "Monitor 'Form reached' fired"),
        1,
        "{shown:?}"
    );
}

/// A running subagent of `parent` that finishes, with what it did so far,
/// when it is asked to stop.
fn running_child(parent: &str, title: &str) -> Arc<BackgroundSubagent> {
    let token = CancellationToken::new();
    let handle = BackgroundSubagent::register(
        parent,
        format!("child-{}", uuid::Uuid::new_v4()),
        title,
        token.clone(),
    );
    let finishing = handle.clone();
    tokio::spawn(async move {
        token.cancelled().await;
        finishing.complete(SubagentResult::from_error("stopped with its work so far"));
    });
    handle
}

/// A monitor's stop stops the subagents the chat started (they run on their
/// own tokens, so they would go on spending against it), collects what each
/// had done, and says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_monitor_stops_the_subagents_the_chat_started() {
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    )
    .await;
    let trials = running_child(&chat.session_id, "Search the trials");
    let notes = running_child(&chat.session_id, "Read the notes");

    let messages = tokio::time::timeout(Duration::from_secs(20), send(&chat, "Fan out."))
        .await
        .expect("the stopped turn ends without waiting on its subagents' own pace");

    assert_eq!(llm.calls(), 1);
    for child in [&trials, &notes] {
        assert!(child.is_cancelled(), "{}", child.title);
        assert_eq!(
            child.stop_cause(),
            Some(ChildStopCause::ParentMonitor {
                monitor: "Form reached".into()
            })
        );
    }
    let shown = notices(&messages);
    let cascade = "It also stopped 2 subagents this chat started ('Search the trials', 'Read \
                   the notes'). What each had done is collected below.";
    assert_eq!(count_containing(&shown, cascade), 1, "{shown:?}");
    let stored_rows = stored(&chat).await;
    assert_eq!(
        stored_rows
            .iter()
            .filter(|m| m.as_concat_text().contains("reached a terminal result"))
            .count(),
        2,
        "what each had done is collected"
    );
    assert!(
        position(&stored_rows, |m| shows(m, "so the turn stops here"))
            < position(&stored_rows, |m| shows(m, "It also stopped 2 subagents"))
    );
}

/// The wait for the subagents a monitor's stop stopped is bounded: one that
/// does not stop is named, and the turn ends without it.
#[tokio::test(flavor = "multi_thread")]
async fn the_wait_after_a_monitor_stop_is_bounded() {
    std::env::set_var("BIOROUTER_MONITOR_STOP_SETTLE_SECS", "1");
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![monitor("form", "Form reached", MonitorAction::Stop, 0.85)],
    )
    .await;
    // Asked to stop, it never does (a tool that ignores cancellation).
    let stuck = BackgroundSubagent::register(
        &chat.session_id,
        format!("child-{}", uuid::Uuid::new_v4()),
        "Sleep for five minutes",
        CancellationToken::new(),
    );

    let messages = tokio::time::timeout(Duration::from_secs(20), send(&chat, "Fan out."))
        .await
        .expect("the wait is bounded");
    std::env::remove_var("BIOROUTER_MONITOR_STOP_SETTLE_SECS");

    assert!(stuck.is_cancelled());
    let shown = notices(&messages);
    assert_eq!(
        count_containing(&shown, "It also stopped 1 subagent this chat started"),
        1,
        "{shown:?}"
    );
    assert_eq!(
        count_containing(
            &shown,
            "1 subagent had not stopped 1 second after the monitor stopped this chat: 'Sleep for \
             five minutes'. They are still being stopped; their results are collected when this \
             chat carries on."
        ),
        1,
        "{shown:?}"
    );
}

/// A stop made in a subagent by a copy of a person's monitor that also
/// watches every subagent of the chat that started it stops the whole run:
/// the subagent's running siblings now, and that chat at its next point.
#[tokio::test(flavor = "multi_thread")]
async fn an_inherited_stop_in_a_subagent_stops_its_siblings_and_reaches_the_chat_that_started_it() {
    let parent = format!("orchestrator-{}", uuid::Uuid::new_v4());
    let llm = Arc::new(Llm::new(ProviderTier::Public, vec![], Reply::Tool));
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.95));
    let mut inherited = monitor("form", "Form reached", MonitorAction::Stop, 0.85);
    inherited.inherited_from = Some(parent.clone());
    let chat = chat_of(
        SessionType::SubAgent,
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![inherited],
    )
    .await;
    let own = BackgroundSubagent::register(
        &parent,
        chat.session_id.clone(),
        "Fill in the form",
        CancellationToken::new(),
    );
    let sibling = running_child(&parent, "Check the dates");

    let messages = tokio::time::timeout(Duration::from_secs(20), send(&chat, "Fill it in."))
        .await
        .expect("the subagent's turn ends");

    assert_eq!(llm.calls(), 1);
    assert_eq!(count_containing(&notices(&messages), FORM_STOPS), 1);
    assert!(sibling.is_cancelled());
    assert_eq!(
        sibling.stop_cause(),
        Some(ChildStopCause::SiblingMonitor {
            monitor: "Form reached".into(),
            sibling: "Fill in the form".into(),
        })
    );
    assert!(
        !own.is_cancelled(),
        "the subagent stopped itself; its own run is not cancelled from outside"
    );
    let report = chat
        .agent
        .take_monitor_stop(&chat.session_id)
        .expect("the subagent's stop");
    assert_eq!(report.set_by, MonitorOwner::Person);
    assert_eq!(report.inherited_from.as_deref(), Some(parent.as_str()));
    // Carried to the chat that started it, for its next point.
    let escalated = MonitorRegistry::default().take_escalation_events(&parent, false);
    assert_eq!(escalated.len(), 1);
    let Step::ChildStopped(stop) = &escalated[0].step else {
        panic!("a stop carried from the subagent: {:?}", escalated[0].step)
    };
    assert_eq!(stop.child_session_id, chat.session_id);
    assert_eq!(stop.child_title, "Fill in the form");
    assert_eq!(stop.p, Some(0.95));
    subagent_handle::open_parent_continuation_admission(&parent);
}

/// A stop carried from a subagent stops the chat that started it at its next
/// point, monitors of its own or none, and tells its model why.
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_escalated_from_a_subagent_stops_the_chat_that_started_it() {
    let target: Arc<OnceLock<String>> = Arc::new(OnceLock::new());
    let mut llm = Llm::new(ProviderTier::Public, vec![], Reply::Tool);
    let parent = target.clone();
    llm.hook_on_call = Some((
        0,
        Arc::new(move || {
            biorouter::decision::monitor::escalate(
                parent.get().expect("the chat"),
                EscalatedStop {
                    child_session_id: "child-1".into(),
                    child_title: "Fill in the form".into(),
                    monitor_id: "form".into(),
                    name: "Form reached".into(),
                    question: "Has the agent reached the consent form page?".into(),
                    threshold: 0.85,
                    below: false,
                    p: Some(0.95),
                },
            );
        }),
    ));
    let llm = Arc::new(llm);
    let decisions = Decisions::new(ProviderTier::Private, Answer::Yes(0.1));
    let chat = chat(
        llm.clone(),
        &decisions,
        SessionClassification::Public,
        vec![],
    )
    .await;
    target.set(chat.session_id.clone()).unwrap();

    let messages = send(&chat, "Fan out.").await;

    assert_eq!(llm.calls(), 1);
    assert_eq!(decisions.requests(), 0, "the stop came from the subagent");
    let shown = notices(&messages);
    assert_eq!(
        count_containing(
            &shown,
            "Monitor 'Form reached' (set to also watch this chat's subagents) stopped subagent \
             'Fill in the form' (P 0.95, threshold 0.85), so this chat and its other subagents \
             stop here."
        ),
        1,
        "{shown:?}"
    );
    let stored_rows = stored(&chat).await;
    let notes = model_only_rows(&stored_rows, STOP_NOTE);
    assert_eq!(notes.len(), 1);
    assert!(
        notes[0]
            .as_concat_text()
            .contains("it stopped subagent \"Fill in the form\""),
        "{}",
        notes[0].as_concat_text()
    );
    let report = chat
        .agent
        .take_monitor_stop(&chat.session_id)
        .expect("this chat's stop");
    assert_eq!(
        report.reason,
        MonitorStopReason::ChildStopped {
            child_session_id: "child-1".into(),
            child_title: "Fill in the form".into(),
        }
    );
}
