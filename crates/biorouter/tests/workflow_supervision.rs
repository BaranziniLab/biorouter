//! Item 8: every door that starts a workflow in its own chat seeds the
//! supervision the workflow declares, through `workflow::runtime::
//! install_prepared` (what `biorouter run --workflow` and a schedule go
//! through; `POST /agent/start` asks `supervision::seed` itself and is tested
//! in the server's `workflow_supervision_start` binaries).
//!
//! A workflow file can declare `decision_settings` (a default decision model
//! and monitors) and an `orchestration_plan` (phases, and gates the workflow
//! owns). The install applies both to a chat that has no messages yet, once,
//! in one transaction, before anything else; a chat that already has messages
//! keeps what it has and is told so; and a seed that cannot apply what the
//! workflow declares writes nothing and fails the install, so the run never
//! starts unsupervised.
//!
//! Offline: the decision models are a scripted [`DecisionModelSource`] behind
//! the agent's own decision service (`Agent::set_decision_service`, which the
//! install asks), and the chat model is a scripted provider. Every decision
//! request is counted; a seed only checks, so it must send none.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::orchestration_plan::{
    apply_workflow_plan, plan_record, OrchestratePhaseInput, PlanPhaseInput, PLAN_STATE,
    PLAN_VERSION,
};
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::decision::model::DecisionCaps;
use biorouter::decision::settings::{
    self, DecisionSettings, MonitorAction, MonitorOwner, DECISION_SETTINGS_STATE,
    DECISION_SETTINGS_VERSION,
};
use biorouter::decision::types::{DecisionKindTag, ModelVerdicts, QuestionVerdict};
use biorouter::decision::{
    DecisionAdapter, DecisionError, DecisionModel, DecisionModelEntry, DecisionModelRef,
    DecisionModelSource, DecisionRequest, DecisionResult, DecisionService, ProbabilitySource,
};
use biorouter::model::ModelConfig;
use biorouter::privacy::{ModelAffiliation, ProviderTier};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use biorouter::workflow::runtime::{
    install_prepared, supervision_notice, SUPERVISION_APPLIED_AT_START,
};
use biorouter::workflow::supervision::{
    SeedOutcome, SeedRefusal, SeedRefusalKind, WORKFLOW_SUPERVISION_STATE,
    WORKFLOW_SUPERVISION_VERSION,
};
use biorouter::workflow::Workflow;
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
// The scripted decision models: two that run on this computer (private) and
// one hosted (public). Every one answers P(yes) = `yes`.
// ---------------------------------------------------------------------------

const MODELS: [(&str, &str, ProviderTier); 3] = [
    ("ollama_decision", "tev1:0.8b", ProviderTier::Private),
    (
        "ollama_decision",
        "clef-flash:latest",
        ProviderTier::Private,
    ),
    ("baseten_decision", "joey-1", ProviderTier::Public),
];

struct Decisions {
    yes: f64,
    requests: AtomicUsize,
}

impl Decisions {
    fn new(yes: f64) -> Arc<Self> {
        Arc::new(Self {
            yes,
            requests: AtomicUsize::new(0),
        })
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

fn entry(provider: &str, model: &str, tier: ProviderTier) -> DecisionModelEntry {
    DecisionModelEntry {
        provider: provider.to_string(),
        model: model.to_string(),
        display_name: model.to_string(),
        context_tokens: 16_384,
        kinds: vec![
            DecisionKindTag::Binary,
            DecisionKindTag::Choice,
            DecisionKindTag::Score,
        ],
        tier,
        affiliation: (tier == ProviderTier::Private).then_some(ModelAffiliation::Local),
        adapter: DecisionAdapter::SystemOne,
        local: false,
        load_refusal: None,
    }
}

struct ScriptedModel {
    entry: DecisionModelEntry,
    decisions: Arc<Decisions>,
}

#[async_trait]
impl DecisionModel for ScriptedModel {
    fn model_ref(&self) -> DecisionModelRef {
        self.entry.model_ref()
    }
    fn display_name(&self) -> String {
        self.entry.model.clone()
    }
    fn tier(&self) -> ProviderTier {
        self.entry.tier
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.entry.affiliation
    }
    fn caps(&self) -> DecisionCaps {
        DecisionCaps {
            kinds: self.entry.kinds.clone(),
            max_questions: 64,
            context_tokens: self.entry.context_tokens,
            max_concurrency: 8,
            max_request_bytes: None,
            local: false,
        }
    }
    async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
        self.decisions.requests.fetch_add(1, Ordering::SeqCst);
        let p = self.decisions.yes;
        Ok(ModelVerdicts {
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
        })
    }
}

struct ScriptedSource(Arc<Decisions>);

#[async_trait]
impl DecisionModelSource for ScriptedSource {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        MODELS
            .iter()
            .map(|(provider, model, tier)| entry(provider, model, *tier))
            .collect()
    }
    async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        match MODELS
            .iter()
            .find(|(provider, model, _)| *provider == wanted.provider && *model == wanted.model)
        {
            Some((provider, model, tier)) => Ok(Arc::new(ScriptedModel {
                entry: entry(provider, model, *tier),
                decisions: self.0.clone(),
            })),
            None => Err(DecisionError::NoModel(format!(
                "No decision model `{}` is listed.",
                wanted.label()
            ))),
        }
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
// The scripted chat model: every reply calls a tool nobody offers, so a turn
// goes on until something stops it.
// ---------------------------------------------------------------------------

struct Llm {
    tier: ProviderTier,
    calls: AtomicUsize,
}

impl Llm {
    fn new(tier: ProviderTier) -> Arc<Self> {
        Arc::new(Self {
            tier,
            calls: AtomicUsize::new(0),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for Llm {
    async fn complete(
        &self,
        system_prompt: &str,
        _messages: &[Message],
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
        let message = Message::assistant()
            .with_text("Echoing the next number.")
            .with_tool_request(
                format!("call-{n}"),
                Ok(CallToolRequestParams {
                    task: None,
                    meta: None,
                    name: "nothing__missing".into(),
                    arguments: Some(rmcp::object!({"step": n})),
                }),
            );
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
    llm: Arc<Llm>,
    _dir: TempDir,
}

/// A NEW chat (no messages), its agent in `mode` asking `decisions`, bound
/// to a chat model of `tier`.
async fn chat(mode: BioRouterMode, tier: ProviderTier, decisions: &Arc<Decisions>) -> Chat {
    let dir = TempDir::new().unwrap();
    let sessions = Arc::new(SessionManager::new_ephemeral(dir.path().join("data")));
    let agent = Agent::with_config(AgentConfig::new(
        sessions.clone(),
        Arc::new(PermissionManager::new(dir.path().join("permissions"))),
        None,
        mode,
    ));
    let session = sessions
        .create_session(
            dir.path().to_path_buf(),
            "workflow supervision test".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    agent.set_decision_service(service(decisions));
    let llm = Llm::new(tier);
    agent
        .update_provider(llm.clone() as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();
    Chat {
        agent: Arc::new(agent),
        sessions,
        session_id: session.id,
        llm,
        _dir: dir,
    }
}

/// What `biorouter run --workflow` and a schedule do: the shared install,
/// with the master switch on.
async fn install(chat: &Chat, workflow: &Workflow) -> anyhow::Result<SeedOutcome> {
    install_prepared(&chat.agent, &chat.session_id, workflow, true, None, true).await
}

fn refusal(error: &anyhow::Error) -> &SeedRefusal {
    error
        .downcast_ref::<SeedRefusal>()
        .unwrap_or_else(|| panic!("the install failed with a refused seed: {error:#}"))
}

async fn stored_settings(chat: &Chat) -> Option<serde_json::Value> {
    chat.sessions
        .get_extension_state(
            &chat.session_id,
            DECISION_SETTINGS_STATE,
            DECISION_SETTINGS_VERSION,
        )
        .await
        .unwrap()
}

async fn stored_plan(chat: &Chat) -> Option<serde_json::Value> {
    chat.sessions
        .get_extension_state(&chat.session_id, PLAN_STATE, PLAN_VERSION)
        .await
        .unwrap()
}

async fn marker(chat: &Chat) -> Option<serde_json::Value> {
    chat.sessions
        .get_extension_state(
            &chat.session_id,
            WORKFLOW_SUPERVISION_STATE,
            WORKFLOW_SUPERVISION_VERSION,
        )
        .await
        .unwrap()
}

/// Nothing of a refused seed is left in the chat.
async fn assert_nothing_written(chat: &Chat) {
    assert_eq!(stored_settings(chat).await, None, "no decision settings");
    assert_eq!(stored_plan(chat).await, None, "no phase plan");
    assert_eq!(marker(chat).await, None, "no seed marker");
}

async fn add_message(chat: &Chat) {
    chat.sessions
        .add_message(
            &chat.session_id,
            &Message::user().with_text("Start counting."),
        )
        .await
        .unwrap();
}

/// A workflow that counts to 30, declaring `blocks` (YAML at the top level).
fn workflow(blocks: &str) -> Workflow {
    Workflow::from_content(&format!(
        "version: 1.0.0\ntitle: Count to thirty\ndescription: probe\n\
         prompt: Using the shell, echo the numbers 1 to 30, one echo command per tool call.\n\
         {blocks}"
    ))
    .unwrap_or_else(|e| panic!("a valid workflow: {e}\n{blocks}"))
}

/// The `reached-ten` monitor, on `model` (or the chat's default when
/// `None`), as a workflow declares it.
fn reached_ten(model: Option<&str>) -> String {
    let model = model
        .map(|label| {
            let (provider, model) = label.split_once('/').unwrap();
            format!("    model: {{provider: {provider}, model: {model}}}\n")
        })
        .unwrap_or_default();
    format!(
        "  - id: reached-ten\n    name: Reached ten\n    question:\n      \
         instructions: Has the agent already echoed 10 or a larger number?\n      \
         kind: {{binary: {{}}}}\n{model}    threshold: 0.7\n    action: stop\n    \
         onError: pause\n"
    )
}

/// `decision_settings` with a default model and `monitors` (YAML items).
fn decision_settings(default_model: &str, monitors: &str) -> String {
    let (provider, model) = default_model.split_once('/').unwrap();
    format!(
        "decision_settings:\n  defaultModel: {{provider: {provider}, model: {model}}}\n  \
         monitors:\n{monitors}"
    )
}

/// A plan of two phases, the second gated.
const PLAN: &str = "orchestration_plan:\n  title: Count plan\n  phases:\n  \
     - {id: count, title: Count, kind: research}\n  - id: check\n    title: Check\n    \
     kind: verification\n    depends_on: [count]\n    gate:\n      \
     questions: {stopped: Did the counting stop at ten?}\n      threshold: 0.8\n      \
     max_attempts: 2\n";

// ---------------------------------------------------------------------------
// The tests.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn install_prepared_seeds_monitors_default_model_and_a_workflow_owned_plan() {
    let decisions = Decisions::new(0.1);
    let chat = chat(BioRouterMode::Auto, ProviderTier::Public, &decisions).await;
    let declared = workflow(&format!(
        "{}{PLAN}",
        decision_settings("ollama_decision/tev1:0.8b", &reached_ten(None))
    ));

    let outcome = install(&chat, &declared).await.expect("the install seeds");

    assert_eq!(
        outcome,
        SeedOutcome::Seeded {
            monitor_ids: vec!["reached-ten".to_string()],
            phase_ids: vec!["count".to_string(), "check".to_string()],
        }
    );
    let stored = settings::load(&chat.sessions, &chat.session_id)
        .await
        .unwrap();
    assert_eq!(
        stored.default_model.as_ref().map(DecisionModelRef::label),
        Some("ollama_decision/tev1:0.8b".to_string())
    );
    assert_eq!(stored.monitors.len(), 1);
    let monitor = &stored.monitors[0];
    assert_eq!(monitor.id, "reached-ten");
    assert_eq!(monitor.action, MonitorAction::Stop);
    assert_eq!(
        monitor.set_by,
        MonitorOwner::Person,
        "a workflow's monitors are the person's"
    );

    let plan = stored_plan(&chat).await.expect("the plan is stored");
    assert_eq!(plan["title"], "Count plan");
    assert_eq!(plan["phases"][0]["id"], "count");
    assert_eq!(plan["phases"][1]["id"], "check");
    assert_eq!(
        plan["phases"][1]["gate"]["owner"], "workflow",
        "a gate a workflow declares is the workflow's, so it binds the agent: {plan:#}"
    );
    assert_eq!(plan["phases"][1]["gate"]["threshold"], 0.8);
    assert_eq!(plan["phases"][1]["gate"]["maxAttempts"], 2);

    let marker = marker(&chat).await.expect("the seed leaves its marker");
    assert_eq!(marker["title"], "Count to thirty");
    assert_eq!(marker["monitorIds"], serde_json::json!(["reached-ten"]));
    assert_eq!(decisions.requests(), 0, "a seed asks no decision model");
}

#[tokio::test(flavor = "multi_thread")]
async fn seeding_happens_once_and_a_removed_monitor_stays_removed() {
    let decisions = Decisions::new(0.1);
    let chat = chat(BioRouterMode::Auto, ProviderTier::Public, &decisions).await;
    let declared = workflow(&decision_settings(
        "ollama_decision/tev1:0.8b",
        &reached_ten(None),
    ));
    assert!(matches!(
        install(&chat, &declared).await.unwrap(),
        SeedOutcome::Seeded { .. }
    ));

    // The person removes the workflow's monitor.
    let mut edited = settings::load(&chat.sessions, &chat.session_id)
        .await
        .unwrap();
    edited.monitors.clear();
    settings::save(&chat.sessions, &chat.session_id, edited)
        .await
        .unwrap();

    // A start that is retried before the first message is not seeded again…
    assert_eq!(
        install(&chat, &declared).await.unwrap(),
        SeedOutcome::AlreadySeeded
    );
    assert!(settings::load(&chat.sessions, &chat.session_id)
        .await
        .unwrap()
        .monitors
        .is_empty());

    // …and neither is the chat once it has started (`--resume`), which is
    // told its blocks were applied when it started and are not applied again,
    // never that they were not applied.
    add_message(&chat).await;
    let resumed = install(&chat, &declared).await.unwrap();
    assert_eq!(resumed, SeedOutcome::ChatHasMessages);
    assert_eq!(
        supervision_notice(&chat.sessions, &chat.session_id, &declared, &resumed).await,
        Some(SUPERVISION_APPLIED_AT_START)
    );
    assert!(
        settings::load(&chat.sessions, &chat.session_id)
            .await
            .unwrap()
            .monitors
            .is_empty(),
        "a monitor the person removed stays removed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_with_messages_is_not_seeded_and_the_outcome_says_so() {
    let decisions = Decisions::new(0.1);
    let chat = chat(BioRouterMode::Auto, ProviderTier::Public, &decisions).await;
    add_message(&chat).await;
    let declared = workflow(&format!(
        "{}{PLAN}",
        decision_settings("ollama_decision/tev1:0.8b", &reached_ten(None))
    ));

    let outcome = install(&chat, &declared).await.unwrap();
    assert_eq!(outcome, SeedOutcome::ChatHasMessages);
    assert_nothing_written(&chat).await;
    // A chat the workflow never seeded is told its blocks were not applied.
    assert_eq!(
        supervision_notice(&chat.sessions, &chat.session_id, &declared, &outcome).await,
        Some(biorouter::workflow::runtime::SUPERVISION_NOT_APPLIED)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_public_decision_model_for_a_chat_bound_to_a_private_model_refuses_and_writes_nothing() {
    let decisions = Decisions::new(0.1);
    let declared = workflow(&decision_settings(
        "baseten_decision/joey-1",
        &reached_ten(None),
    ));

    // The chat is Public at birth, but its model is private, so its first
    // turn makes it Private: the hosted decision model may not read it.
    let private = chat(BioRouterMode::Auto, ProviderTier::Private, &decisions).await;
    let error = install(&private, &declared)
        .await
        .expect_err("a public decision model for a chat bound to a private model");
    let refused = refusal(&error);
    assert_eq!(refused.kind, SeedRefusalKind::Refused);
    assert_eq!(refused.status(), 422);
    assert!(
        format!("{error:#}").starts_with("The workflow was not started: "),
        "{error:#}"
    );
    assert_nothing_written(&private).await;

    // The same workflow on a chat bound to a public model is seeded: the
    // refusal above is the bound model's.
    let public = chat(BioRouterMode::Auto, ProviderTier::Public, &decisions).await;
    assert!(matches!(
        install(&public, &declared).await.unwrap(),
        SeedOutcome::Seeded { .. }
    ));
    assert_eq!(decisions.requests(), 0, "a seed asks no decision model");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_monitor_naming_a_missing_model_refuses_the_run() {
    let decisions = Decisions::new(0.1);
    let chat = chat(BioRouterMode::Auto, ProviderTier::Public, &decisions).await;
    let declared = workflow(&format!(
        "decision_settings:\n  monitors:\n{}",
        reached_ten(Some("ollama_decision/absent:1b"))
    ));

    let error = install(&chat, &declared)
        .await
        .expect_err("a monitor on a model that is not there");
    let refused = refusal(&error);
    assert_eq!(refused.kind, SeedRefusalKind::Refused);
    assert!(
        refused.sentence.contains("absent:1b"),
        "the sentence names the model: {}",
        refused.sentence
    );
    assert_nothing_written(&chat).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plan_outside_auto_mode_refuses_the_run() {
    let decisions = Decisions::new(0.1);
    let chat = chat(BioRouterMode::Approve, ProviderTier::Public, &decisions).await;
    let declared = workflow(&format!(
        "{}{PLAN}",
        decision_settings("ollama_decision/tev1:0.8b", &reached_ten(None))
    ));

    let error = install(&chat, &declared)
        .await
        .expect_err("a phased plan outside the Autonomous approval mode");
    let refused = refusal(&error);
    assert_eq!(refused.kind, SeedRefusalKind::NeedsAutoMode);
    assert_eq!(refused.status(), 409);
    assert!(
        refused
            .sentence
            .contains("runs only in the Autonomous approval mode")
            && refused.sentence.contains("the Manual approval mode"),
        "{}",
        refused.sentence
    );
    // Not even the monitors, which on their own would have been applied.
    assert_nothing_written(&chat).await;
    assert_eq!(decisions.requests(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_merge_keeps_a_monitor_the_chat_already_has() {
    let decisions = Decisions::new(0.1);
    let chat = chat(BioRouterMode::Auto, ProviderTier::Public, &decisions).await;
    // Set by the person before the workflow ran: another default model, and
    // a `reached-ten` of their own that only notifies.
    let mut own: DecisionSettings = serde_json::from_value(serde_json::json!({
        "defaultModel": {"provider": "ollama_decision", "model": "clef-flash:latest"},
        "monitors": [{
            "id": "reached-ten",
            "name": "My reached ten",
            "question": {
                "instructions": "Has it echoed ten numbers?",
                "kind": {"binary": {}}
            },
            "threshold": 0.9,
            "action": "notify"
        }]
    }))
    .unwrap();
    own.monitors[0].enabled = true;
    settings::save(&chat.sessions, &chat.session_id, own)
        .await
        .unwrap();
    let off_task = "  - id: off-task\n    name: Off task\n    question:\n      \
                    instructions: Has the agent stopped counting?\n      \
                    kind: {binary: {}}\n    threshold: 0.8\n    action: notify\n";
    let declared = workflow(&decision_settings(
        "ollama_decision/tev1:0.8b",
        &format!("{}{off_task}", reached_ten(None)),
    ));

    let outcome = install(&chat, &declared).await.unwrap();

    assert_eq!(
        outcome,
        SeedOutcome::Seeded {
            monitor_ids: vec!["off-task".to_string()],
            phase_ids: vec![],
        },
        "only the monitor the chat did not have was added"
    );
    let merged = settings::load(&chat.sessions, &chat.session_id)
        .await
        .unwrap();
    assert_eq!(
        merged.default_model.as_ref().map(DecisionModelRef::label),
        Some("ollama_decision/clef-flash:latest".to_string()),
        "the chat's own default decision model wins"
    );
    let ids: Vec<&str> = merged.monitors.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["reached-ten", "off-task"]);
    assert_eq!(
        merged.monitors[0].name, "My reached ten",
        "the chat's monitor wins on a shared id"
    );
    assert_eq!(merged.monitors[0].action, MonitorAction::Notify);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_seeded_stop_monitor_stops_the_turn() {
    // The decision model answers yes, so the monitor fires at its first check.
    let decisions = Decisions::new(0.95);
    let chat = chat(BioRouterMode::Auto, ProviderTier::Public, &decisions).await;
    let declared = workflow(&decision_settings(
        "ollama_decision/tev1:0.8b",
        &reached_ten(None),
    ));
    install(&chat, &declared).await.expect("the install seeds");

    let stream = chat
        .agent
        .reply(
            Message::user().with_text("Count to thirty."),
            SessionConfig {
                id: chat.session_id.clone(),
                schedule_id: None,
                max_turns: Some(8),
                max_tool_calls: None,
                retry_config: None,
                budget: None,
                reasoning_effort: None,
            },
            None,
        )
        .await
        .expect("a reply stream");
    tokio::pin!(stream);
    let mut notices = Vec::new();
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event.expect("a reply event") {
            notices.extend(message.content.iter().filter_map(|c| match c {
                MessageContent::SystemNotification(n) => Some(n.msg.clone()),
                _ => None,
            }));
        }
    }

    assert_eq!(
        chat.llm.calls(),
        1,
        "the seeded monitor stopped a turn that would have gone on to its limit"
    );
    assert!(decisions.requests() >= 1, "the seeded monitor was asked");
    assert!(
        notices
            .iter()
            .any(|n| n.contains("Monitor 'Reached ten' fired") && n.contains("the turn stops")),
        "{notices:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn seeding_is_one_transaction() {
    let decisions = Decisions::new(0.1);
    let chat = chat(BioRouterMode::Auto, ProviderTier::Public, &decisions).await;
    // The chat already has a plan, so the seed's write must refuse, and
    // must take the settings and the marker it was writing down with it.
    let existing = apply_workflow_plan(
        "An earlier plan",
        None,
        vec![OrchestratePhaseInput {
            phase: PlanPhaseInput {
                id: "earlier".to_string(),
                title: "Earlier".to_string(),
                kind: Some("research".to_string()),
                goal: None,
                depends_on: vec![],
            },
            gate: None,
            monitors: None,
        }],
        chrono::Utc::now(),
    )
    .unwrap();
    chat.sessions
        .update_extension_state(&chat.session_id, PLAN_STATE, PLAN_VERSION, |_| {
            Ok(plan_record(&existing))
        })
        .await
        .unwrap();
    let declared = workflow(&format!(
        "{}{PLAN}",
        decision_settings("ollama_decision/tev1:0.8b", &reached_ten(None))
    ));

    let error = install(&chat, &declared)
        .await
        .expect_err("a chat that already has a plan is not given another");
    let refused = refusal(&error);
    assert_eq!(refused.kind, SeedRefusalKind::Invalid);
    assert!(
        refused.sentence.contains("already has a phase plan"),
        "{}",
        refused.sentence
    );
    assert_eq!(
        stored_settings(&chat).await,
        None,
        "the settings the seed was writing did not land"
    );
    assert_eq!(marker(&chat).await, None, "no marker");
    assert_eq!(
        stored_plan(&chat).await.expect("the earlier plan")["title"],
        "An earlier plan",
        "the earlier plan is untouched"
    );
}
