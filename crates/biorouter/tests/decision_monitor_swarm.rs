//! Decision-model monitors over a whole swarm: an orchestrator and the
//! subagents it starts, driven through the real `subagent`, `workspace_watch`
//! and `workspace_monitor` tools and the real background runs. Offline: one
//! scripted provider plays the orchestrator and every child (a child's system
//! prompt opens "You are a specialized subagent" and carries its `TASK:`
//! script, as in `subagent_support`), and the decision model is a scripted
//! [`DecisionModel`] behind the orchestrator's own decision service
//! ([`Agent::set_decision_service`]), which every child it starts asks too.
//!
//! Covers, at the swarm level:
//! - item A: a monitor the agent puts on a subagent at spawn stops that
//!   subagent, and the parent reads a stopped result saying which monitor did
//!   it; a subagent's `turn_end` monitors run; a copy of the person's monitor
//!   that watches every subagent stops the subagent it fired in, its siblings
//!   and the orchestrator; `workspace_monitor` on a running subagent applies
//!   at its next check.
//! - item B: a monitor's stop on the orchestrator stops the subagents it
//!   started and collects what they did; the wait for them is bounded, and a
//!   steer the person types still ends it; a turn-limit stop is not a monitor
//!   stop and still waits for its subagents; a steer already waiting holds the
//!   stop and with it the cascade.
//! - item 1: a monitor saved on the orchestrator after its fan-out is checked
//!   during the same swarm turn.
//!
//! Every test runs on the agent runtime the daemon uses
//! ([`biorouter::execution::runtime::build_agent_runtime`]), because a
//! subagent's reply loop runs on a worker thread.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use biorouter::agents::extension::ExtensionConfig;
use biorouter::agents::loop_phase::LoopPhase;
use biorouter::agents::subagent_handle::{self, BackgroundSubagent, ChildStopCause};
use biorouter::agents::subagent_result::{SubagentResult, SubagentStatus};
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{ActionRequiredData, Message, MessageContent};
use biorouter::decision::model::DecisionCaps;
use biorouter::decision::settings::{
    DecisionSettings, MonitorAction, MonitorCriterion, MonitorErrorPolicy, MonitorOwner,
    MonitorStopReason, MonitorTiming, MonitorWindow,
};
use biorouter::decision::types::{DecisionKindTag, ModelVerdicts, QuestionVerdict};
use biorouter::decision::{
    DecisionAdapter, DecisionKind, DecisionModel, DecisionModelEntry, DecisionModelRef,
    DecisionModelSource, DecisionRequest, DecisionResult, DecisionService, ProbabilitySource,
    Question,
};
use biorouter::model::ModelConfig;
use biorouter::privacy::{ModelAffiliation, ProviderTier};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::future::BoxFuture;
use futures::{Stream, StreamExt};
use indexmap::IndexMap;
use rmcp::model::{CallToolRequestParams, Tool};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// A sandboxed process: a throwaway config root, no keychain, and a turn cap
/// and concurrency cap that fit these runs.
///
/// The process's decision service ([`biorouter::decision::global`]) is what
/// `workspace_monitor` asks whether a subagent may run a monitor's model
/// before it stores it. Here it lists the Baseten catalog (a key that is not
/// one) and no Ollama models (a closed loopback port), so that check is the
/// same on every machine, and nothing it could open reaches past loopback.
/// The checks themselves are answered by the orchestrator's scripted
/// service, which lists the same model.
#[ctor::ctor]
fn sandbox() {
    let root = TempDir::new().expect("a sandbox");
    for sub in ["config", "data", "state"] {
        std::fs::create_dir_all(root.path().join(sub)).expect("a sandbox tree");
    }
    std::fs::write(root.path().join("config/config.yaml"), "{}\n").expect("a config file");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    std::env::set_var("BIOROUTER_SUBAGENT_MAX_TURNS", "6");
    std::env::set_var("BIOROUTER_SUBAGENT_MAX_CONCURRENT", "32");
    std::env::set_var("BASETEN_API_KEY", "swarm-test-not-a-key");
    std::env::set_var("BASETEN_DECISION_HOST", "http://127.0.0.1:9");
    std::env::set_var("OLLAMA_HOST", "http://127.0.0.1:9");
    static ROOT: OnceLock<TempDir> = OnceLock::new();
    let _ = ROOT.set(root);
}

/// `BIOROUTER_MONITOR_STOP_SETTLE_SECS` is read from the process's
/// environment at every stop. The test that lowers it holds this for
/// writing; every test whose subagents a monitor's stop stops holds it for
/// reading, so none of them sees the lowered bound.
static SETTLE: LazyLock<tokio::sync::RwLock<()>> = LazyLock::new(|| tokio::sync::RwLock::new(()));

/// Run one test on the agent runtime, on a worker thread as the daemon does.
fn run(test: impl std::future::Future<Output = ()> + Send + 'static) {
    biorouter::execution::runtime::build_agent_runtime()
        .expect("the agent runtime builds")
        .block_on(async move {
            if let Err(error) = tokio::spawn(test).await {
                match error.try_into_panic() {
                    Ok(panic) => std::panic::resume_unwind(panic),
                    Err(error) => panic!("the test did not finish: {error}"),
                }
            }
        });
}

async fn within<T>(limit: Duration, what: &str, work: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(limit, work)
        .await
        .unwrap_or_else(|_| panic!("{what} did not happen within {limit:?}"))
}

// ---------------------------------------------------------------------------
// The scripted decision model.
// ---------------------------------------------------------------------------

/// The one decision model listed: Baseten's joey-1, as a monitor names it.
const JOEY: &str = "baseten_decision/joey-1";

/// What a subagent writes when it reaches what an inherited monitor watches
/// for, and nothing else in these runs says.
const TRIGGER: &str = "CONSENT-FORM-REACHED";

struct Decisions {
    /// P(yes) for a check whose messages show this text, `otherwise` for the
    /// rest. `None`: every check answers `otherwise`.
    needle: Option<&'static str>,
    hit: f64,
    otherwise: f64,
    requests: AtomicUsize,
    /// What each check was shown, in order.
    shown: Mutex<Vec<String>>,
}

impl Decisions {
    /// Every check answers P(yes) = `p`.
    fn fixed(p: f64) -> Arc<Self> {
        Arc::new(Self {
            needle: None,
            hit: p,
            otherwise: p,
            requests: AtomicUsize::new(0),
            shown: Mutex::new(Vec::new()),
        })
    }

    /// A check answers `hit` where its messages show `needle`, else `miss`.
    fn showing(needle: &'static str, hit: f64, miss: f64) -> Arc<Self> {
        Arc::new(Self {
            needle: Some(needle),
            hit,
            otherwise: miss,
            requests: AtomicUsize::new(0),
            shown: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    /// How many checks were shown `needle`.
    fn shown_containing(&self, needle: &str) -> usize {
        self.shown
            .lock()
            .unwrap()
            .iter()
            .filter(|shown| shown.contains(needle))
            .count()
    }

    fn entry() -> DecisionModelEntry {
        DecisionModelEntry {
            provider: "baseten_decision".to_string(),
            model: "joey-1".to_string(),
            display_name: "joey-1".to_string(),
            context_tokens: 16_384,
            kinds: vec![
                DecisionKindTag::Binary,
                DecisionKindTag::Choice,
                DecisionKindTag::Score,
            ],
            tier: ProviderTier::Public,
            affiliation: None,
            adapter: DecisionAdapter::SystemOne,
            local: false,
            load_refusal: None,
        }
    }
}

struct ScriptedModel(Arc<Decisions>);

#[async_trait]
impl DecisionModel for ScriptedModel {
    fn model_ref(&self) -> DecisionModelRef {
        Decisions::entry().model_ref()
    }
    fn display_name(&self) -> String {
        "joey-1".to_string()
    }
    fn tier(&self) -> ProviderTier {
        ProviderTier::Public
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        None
    }
    fn caps(&self) -> DecisionCaps {
        DecisionCaps {
            kinds: Decisions::entry().kinds,
            max_questions: 64,
            context_tokens: 16_384,
            max_concurrency: 8,
            max_request_bytes: None,
            local: false,
        }
    }
    async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
        self.0.requests.fetch_add(1, Ordering::SeqCst);
        let shown = request.context.render_text();
        self.0.shown.lock().unwrap().push(shown.clone());
        let p = match self.0.needle {
            Some(needle) if shown.contains(needle) => self.0.hit,
            _ => self.0.otherwise,
        };
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
                    raw: Value::Null,
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
        vec![Decisions::entry()]
    }
    async fn open(&self, _wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
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
// The scripted chat model: the orchestrator and every subagent.
// ---------------------------------------------------------------------------

/// The opening line of a subagent's system prompt.
const CHILD_PROMPT_MARKER: &str = "You are a specialized subagent";
/// The stall judge's system prompt, answered "not looping".
const LOOP_MONITOR_MARKER: &str = "You are a loop monitor";
/// How long a `hang:` subagent's call takes when nothing stops it.
const HANG: Duration = Duration::from_secs(120);

/// One subagent the orchestrator starts. Its script (after `TASK:`):
///
/// | script | the subagent |
/// |---|---|
/// | `done:<name>` | answers `child <name> done` |
/// | `steps:<name>` | takes a tool step per call (four), then answers |
/// | `steer-once:<name>` | answers; asked again, answers `…, and checked` |
/// | `hang:<name>` | its call takes [`HANG`] unless it is stopped |
/// | `slow:<name>:<ms>` | its call takes `<ms>`, then it answers |
/// | `trip:<name>` | answers that it reached [`TRIGGER`] |
/// | `await-monitor:<name>` | a step; then, once a monitor is on its chat, another; then answers |
#[derive(Clone)]
struct Child {
    script: String,
    monitors: Option<Value>,
}

impl Child {
    fn new(script: &str) -> Self {
        Self {
            script: script.to_string(),
            monitors: None,
        }
    }

    fn with_monitors(mut self, monitors: Value) -> Self {
        self.monitors = Some(monitors);
        self
    }

    fn instructions(&self) -> String {
        instructions_of(&self.script)
    }
}

fn name_of(script: &str) -> &str {
    script.split(':').nth(1).unwrap_or("anon")
}

/// A spawn's instructions. Its handle's title is these on one line, with the
/// summary instructions the spawn appends, cut at 80 characters.
fn instructions_of(script: &str) -> String {
    format!("Work on {}. TASK:{script}", name_of(script))
}

#[derive(Clone)]
enum Step {
    /// One `subagent` call per child, in one batch.
    Spawn(Vec<Child>),
    /// `workspace_watch` on every subagent this chat started, mode `all`.
    WatchChildren,
    /// `workspace_monitor` `add` on the first subagent this chat started.
    MonitorChild(Value),
    Text(&'static str),
}

/// What a hook can reach.
#[derive(Clone)]
struct Cx {
    agent: Arc<Agent>,
    sessions: Arc<SessionManager>,
    parent: String,
    /// The scripted model itself, to read what the subagents have done.
    llm: std::sync::Weak<SwarmLlm>,
}

type Hook = Arc<dyn Fn(Cx) -> BoxFuture<'static, ()> + Send + Sync>;

struct SwarmLlm {
    script: Mutex<VecDeque<Step>>,
    otherwise: &'static str,
    parent_calls: AtomicUsize,
    /// The texts each orchestrator call was shown, in order.
    parent_seen: Mutex<Vec<Vec<String>>>,
    /// Run at the start of the orchestrator call with this index.
    hooks: Mutex<HashMap<usize, Hook>>,
    /// Calls per subagent, by name.
    child_calls: Mutex<HashMap<String, usize>>,
    /// The texts each of a subagent's calls was shown, by name.
    child_seen: Mutex<HashMap<String, Vec<Vec<String>>>>,
    cx: OnceLock<Cx>,
}

impl SwarmLlm {
    fn new(script: Vec<Step>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            otherwise: "Waiting on the subagents.",
            parent_calls: AtomicUsize::new(0),
            parent_seen: Mutex::new(Vec::new()),
            hooks: Mutex::new(HashMap::new()),
            child_calls: Mutex::new(HashMap::new()),
            child_seen: Mutex::new(HashMap::new()),
            cx: OnceLock::new(),
        }
    }

    fn on_call(self, index: usize, hook: Hook) -> Self {
        self.hooks.lock().unwrap().insert(index, hook);
        self
    }

    fn parent_calls(&self) -> usize {
        self.parent_calls.load(Ordering::SeqCst)
    }

    fn parent_saw(&self, needle: &str) -> bool {
        self.parent_seen
            .lock()
            .unwrap()
            .iter()
            .any(|call| call.iter().any(|text| text.contains(needle)))
    }

    fn child_calls(&self, name: &str) -> usize {
        self.child_calls
            .lock()
            .unwrap()
            .get(name)
            .copied()
            .unwrap_or(0)
    }

    fn child_saw_on_call(&self, name: &str, call: usize) -> Vec<String> {
        self.child_seen
            .lock()
            .unwrap()
            .get(name)
            .and_then(|calls| calls.get(call))
            .cloned()
            .unwrap_or_default()
    }

    fn cx(&self) -> Cx {
        self.cx.get().expect("the swarm is built").clone()
    }

    async fn parent_turn(&self, seen: Vec<String>) -> Message {
        let n = self.parent_calls.fetch_add(1, Ordering::SeqCst);
        self.parent_seen.lock().unwrap().push(seen);
        let hook = self.hooks.lock().unwrap().get(&n).cloned();
        if let Some(hook) = hook {
            hook(self.cx()).await;
        }
        let step = self
            .script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Step::Text(self.otherwise));
        let parent = self.cx().parent;
        match step {
            Step::Text(text) => Message::assistant().with_text(text),
            Step::Spawn(children) => {
                let mut message = Message::assistant().with_text("Fanning the work out.");
                for (i, child) in children.iter().enumerate() {
                    let mut arguments = json!({ "instructions": child.instructions() });
                    if let Some(monitors) = &child.monitors {
                        arguments["monitors"] = monitors.clone();
                    }
                    message = message.with_tool_request(
                        format!("p{n}-spawn-{i}"),
                        Ok(tool_call("subagent", arguments)),
                    );
                }
                message
            }
            Step::WatchChildren => {
                let ids: Vec<String> = subagent_handle::list_for_session(&parent)
                    .iter()
                    .map(|handle| handle.child_session_id.clone())
                    .collect();
                Message::assistant().with_tool_request(
                    format!("p{n}-watch"),
                    Ok(tool_call(
                        "workspace__workspace_watch",
                        json!({ "session_ids": ids, "mode": "all", "timeout_s": 60 }),
                    )),
                )
            }
            Step::MonitorChild(spec) => {
                let child = subagent_handle::list_for_session(&parent)
                    .first()
                    .map(|handle| handle.child_session_id.clone())
                    .expect("a subagent to put the monitor on");
                Message::assistant().with_tool_request(
                    format!("p{n}-monitor"),
                    Ok(tool_call(
                        "workspace__workspace_monitor",
                        json!({ "action": "add", "session_id": child, "monitors": [spec] }),
                    )),
                )
            }
        }
    }

    async fn child_turn(&self, script: &str, seen: Vec<String>) -> Message {
        let mut parts = script.split(':');
        let kind = parts.next().unwrap_or("done");
        let name = parts.next().unwrap_or("anon").to_string();
        let arg = parts.next().map(str::to_string);
        let turn = {
            let mut calls = self.child_calls.lock().unwrap();
            let count = calls.entry(name.clone()).or_insert(0);
            *count += 1;
            *count
        };
        self.child_seen
            .lock()
            .unwrap()
            .entry(name.clone())
            .or_default()
            .push(seen);
        let done = || Message::assistant().with_text(format!("child {name} done"));
        match kind {
            "steps" if turn <= 4 => tool_step(&name, turn, &format!("Working on step {turn}.")),
            "steer-once" if turn > 1 => {
                Message::assistant().with_text(format!("child {name} done, and checked"))
            }
            "hang" => {
                tokio::time::sleep(HANG).await;
                done()
            }
            "slow" => {
                let ms = arg.and_then(|ms| ms.parse().ok()).unwrap_or(500);
                tokio::time::sleep(Duration::from_millis(ms)).await;
                done()
            }
            "trip" if turn == 1 => {
                Message::assistant().with_text(format!("The {TRIGGER} page is open."))
            }
            "await-monitor" if turn <= 2 => {
                if turn == 2 {
                    self.wait_for_a_monitor_on(&name).await;
                }
                tool_step(&name, turn, &format!("Reading the notes, part {turn}."))
            }
            _ => done(),
        }
    }

    /// Until a monitor is on the chat of the subagent `name` (at most 30 s).
    async fn wait_for_a_monitor_on(&self, name: &str) {
        let cx = self.cx();
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let child = subagent_handle::list_for_session(&cx.parent)
                .into_iter()
                .find(|handle| handle.title.contains(&format!("Work on {name}.")));
            if let Some(child) = child {
                let stored =
                    biorouter::decision::settings::load(&cx.sessions, &child.child_session_id)
                        .await;
                if stored.is_ok_and(|settings| !settings.monitors.is_empty()) {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

fn tool_call(name: &str, arguments: Value) -> CallToolRequestParams {
    CallToolRequestParams {
        task: None,
        meta: None,
        name: name.to_string().into(),
        arguments: Some(
            arguments
                .as_object()
                .cloned()
                .expect("tool arguments are an object"),
        ),
    }
}

/// One step of a subagent's work: some text and a call to a tool nobody
/// offers, so the loop goes on with an error result.
fn tool_step(name: &str, turn: usize, text: &str) -> Message {
    Message::assistant().with_text(text).with_tool_request(
        format!("{name}-step-{turn}"),
        Ok(tool_call("nothing__missing", json!({ "step": turn }))),
    )
}

/// The `TASK:<script>` a subagent's system prompt carries.
fn script_of(system_prompt: &str) -> Option<String> {
    let rest = system_prompt.split("TASK:").nth(1)?;
    Some(
        rest.lines()
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches('.')
            .to_string(),
    )
}

fn texts_of(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

#[async_trait]
impl Provider for SwarmLlm {
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
        if system_prompt.contains(LOOP_MONITOR_MARKER) {
            return Ok((
                Message::assistant().with_text(r#"{"looping":false}"#),
                usage,
            ));
        }
        let seen = texts_of(messages);
        if system_prompt.contains(CHILD_PROMPT_MARKER) {
            let script = script_of(system_prompt).unwrap_or_else(|| "done:unscripted".to_string());
            return Ok((self.child_turn(&script, seen).await, usage));
        }
        Ok((self.parent_turn(seen).await, usage))
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
            runs_locally: false,
            institutions: Vec::new(),
        }
    }

    fn get_name(&self) -> &str {
        "scripted-swarm"
    }

    fn tier(&self) -> ProviderTier {
        ProviderTier::Public
    }
}

// ---------------------------------------------------------------------------
// The harness.
// ---------------------------------------------------------------------------

/// An orchestrator chat that may delegate: Completely Autonomous, a model
/// whose name does not start with `gemini`, a user's chat (not a subagent's),
/// and one extension that was not injected (`developer`).
struct Swarm {
    agent: Arc<Agent>,
    sessions: Arc<SessionManager>,
    session_id: String,
    llm: Arc<SwarmLlm>,
    _dir: TempDir,
}

async fn swarm(
    llm: SwarmLlm,
    decisions: &Arc<Decisions>,
    monitors: Vec<MonitorCriterion>,
) -> Swarm {
    let dir = TempDir::new().unwrap();
    let sessions = Arc::new(SessionManager::new(dir.path().join("data")));
    let agent = Agent::with_config(AgentConfig::new(
        sessions.clone(),
        Arc::new(PermissionManager::new(dir.path().join("permissions"))),
        None,
        BioRouterMode::Auto,
    ));
    let session = sessions
        .create_session(
            dir.path().to_path_buf(),
            "decision monitor swarm".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    if !monitors.is_empty() {
        save_monitors(&sessions, &session.id, monitors).await;
    }
    agent.set_decision_service(service(decisions));
    let llm = Arc::new(llm);
    agent
        .update_provider(llm.clone() as Arc<dyn Provider>, &session.id)
        .await
        .unwrap();
    agent
        .add_extension(ExtensionConfig::Builtin {
            name: "developer".to_string(),
            description: "Developer tools".to_string(),
            display_name: None,
            timeout: None,
            bundled: Some(true),
            available_tools: vec![],
        })
        .await
        .expect("the developer extension registers");
    let agent = Arc::new(agent);
    let _ = llm.cx.set(Cx {
        agent: agent.clone(),
        sessions: sessions.clone(),
        parent: session.id.clone(),
        llm: Arc::downgrade(&llm),
    });
    Swarm {
        agent,
        sessions,
        session_id: session.id,
        llm,
        _dir: dir,
    }
}

/// The chat's monitors, saved as the person saves them.
async fn save_monitors(
    sessions: &SessionManager,
    session_id: &str,
    monitors: Vec<MonitorCriterion>,
) {
    biorouter::decision::settings::save(
        sessions,
        session_id,
        DecisionSettings {
            default_model: None,
            monitors,
        },
    )
    .await
    .expect("valid monitors are stored");
}

fn config(s: &Swarm, max_turns: u32) -> SessionConfig {
    SessionConfig {
        id: s.session_id.clone(),
        schedule_id: None,
        max_turns: Some(max_turns),
        max_tool_calls: None,
        retry_config: None,
        budget: None,
        reasoning_effort: None,
    }
}

const ASK: &str = "Fan the work out to subagents.";

/// One orchestrator turn, read to its end.
async fn send(s: &Swarm, max_turns: u32) -> Vec<Message> {
    let stream = s
        .agent
        .reply(Message::user().with_text(ASK), config(s, max_turns), None)
        .await
        .expect("a reply stream");
    drain(&s.agent, stream).await
}

/// Every message a reply yields, approving any tool card.
async fn drain(
    agent: &Agent,
    stream: impl Stream<Item = anyhow::Result<AgentEvent>>,
) -> Vec<Message> {
    tokio::pin!(stream);
    let mut out = Vec::new();
    while let Some(event) = stream.next().await {
        if let AgentEvent::Message(message) = event.expect("a reply event") {
            if let Some(MessageContent::ActionRequired(action)) = message.content.first() {
                if let ActionRequiredData::ToolConfirmation { id, .. } = &action.data {
                    agent
                        .handle_confirmation(
                            id.clone(),
                            biorouter::permission::PermissionConfirmation {
                                principal_type:
                                    biorouter::permission::permission_confirmation::PrincipalType::Tool,
                                permission: biorouter::permission::Permission::AllowOnce,
                            },
                        )
                        .await;
                }
            }
            out.push(message);
        }
    }
    out
}

/// A monitor as the person sets it on the orchestrator.
fn person_monitor(id: &str, name: &str, threshold: f64) -> MonitorCriterion {
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
        action: MonitorAction::Stop,
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

/// A monitor as the agent writes it for a subagent (`MonitorSpec`).
fn agent_spec(name: &str, question: &str, action: &str, timing: &str, threshold: f64) -> Value {
    json!({
        "name": name,
        "question": question,
        "threshold": threshold,
        "action": action,
        "timing": timing,
        "model": JOEY,
    })
}

const OFF_TASK_QUESTION: &str = "Is the agent editing files outside src/?";

fn off_task() -> Value {
    agent_spec("Off task", OFF_TASK_QUESTION, "stop", "each_step", 0.8)
}

fn children(parent: &str) -> Vec<Arc<BackgroundSubagent>> {
    subagent_handle::list_for_session(parent)
}

fn child_named(parent: &str, name: &str) -> Arc<BackgroundSubagent> {
    children(parent)
        .into_iter()
        .find(|handle| handle.title.contains(&format!("Work on {name}.")))
        .unwrap_or_else(|| panic!("no subagent '{name}' was started"))
}

fn result_of(handle: &BackgroundSubagent) -> SubagentResult {
    handle
        .result()
        .unwrap_or_else(|| panic!("'{}' has not finished", handle.title))
}

/// A subagent of `parent` that a request to stop does not stop (a tool that
/// ignores cancellation), registered as the spawn registers one.
fn stuck_child(parent: &str, title: &str) -> Arc<BackgroundSubagent> {
    BackgroundSubagent::register(
        parent,
        format!("stuck-{}", uuid::Uuid::new_v4()),
        title,
        CancellationToken::new(),
    )
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

fn count_containing(texts: &[String], needle: &str) -> usize {
    texts.iter().filter(|t| t.contains(needle)).count()
}

/// How many results of delegated subagents the orchestrator collected.
fn collected(messages: &[Message]) -> usize {
    messages
        .iter()
        .filter(|m| m.as_concat_text().contains("reached a terminal result"))
        .count()
}

fn shows(message: &Message, needle: &str) -> bool {
    message.content.iter().any(|c| match c {
        MessageContent::Text(t) => t.text.contains(needle),
        MessageContent::SystemNotification(n) => n.msg.contains(needle),
        _ => false,
    })
}

fn position(messages: &[Message], needle: &str) -> usize {
    messages
        .iter()
        .position(|m| shows(m, needle))
        .unwrap_or_else(|| panic!("no stored row shows {needle:?}"))
}

async fn stored(sessions: &SessionManager, session_id: &str) -> Vec<Message> {
    sessions
        .get_session(session_id, true)
        .await
        .unwrap()
        .conversation
        .map(|c| c.messages().clone())
        .unwrap_or_default()
}

/// Every `(tool_response_id, text, is_error)` the turn yielded.
fn tool_responses(messages: &[Message]) -> Vec<(String, String, bool)> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::ToolResponse(r) => {
                let (text, is_error) = match &r.tool_result {
                    Ok(result) => (
                        result
                            .content
                            .iter()
                            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        result.is_error.unwrap_or(false),
                    ),
                    Err(e) => (e.to_string(), true),
                };
                Some((r.id.clone(), text, is_error))
            }
            _ => None,
        })
        .collect()
}

fn response(messages: &[Message], id: &str) -> String {
    let (_, text, is_error) = tool_responses(messages)
        .into_iter()
        .find(|(call, ..)| call == id)
        .unwrap_or_else(|| panic!("no response to {id}"));
    assert!(!is_error, "{id} failed: {text}");
    text
}

const FORM_STOP_SENTENCE: &str =
    "Stopped because monitor 'Form reached' stopped the chat that started this subagent.";

// ---------------------------------------------------------------------------
// Item A.
// ---------------------------------------------------------------------------

/// A monitor the agent puts on a subagent as it spawns it stops that
/// subagent, and the orchestrator reads a stopped result: `incomplete`, the
/// monitor's reason first, and the stop on the result's own field.
#[test]
fn a_child_stop_monitor_reaches_the_parent_as_a_stopped_result() {
    run(async {
        let _settle = SETTLE.read().await;
        let decisions = Decisions::fixed(0.95);
        let llm = SwarmLlm::new(vec![
            Step::Spawn(vec![
                Child::new("steps:worker").with_monitors(json!([off_task()]))
            ]),
            Step::WatchChildren,
            Step::Text("The worker was stopped by its monitor."),
        ]);
        let s = swarm(llm, &decisions, vec![]).await;

        let messages = within(Duration::from_secs(60), "the turn", send(&s, 8)).await;

        assert!(
            response(&messages, "p0-spawn-0")
                .contains("Monitors on it: 'Off task' (yours, stops at 0.80)."),
            "the spawn reply names the monitor"
        );
        let worker = child_named(&s.session_id, "worker");
        let result = result_of(&worker);
        assert_eq!(
            result.status,
            SubagentStatus::Incomplete,
            "{}",
            result.summary
        );
        assert!(
            result.summary.starts_with(
                "Stopped by monitor 'Off task' (set by this chat's agent): P 0.95 reached its \
                 threshold 0.80. Its question: \"Is the agent editing files outside src/?\". The \
                 run did not finish."
            ),
            "{}",
            result.summary
        );
        let stop = result
            .stopped_by_monitor
            .expect("the stop rides on the result");
        assert_eq!(stop.name, "Off task");
        assert_eq!(stop.set_by, MonitorOwner::Agent);
        assert!(!stop.inherited);
        assert_eq!(stop.p, Some(0.95));
        assert_eq!(stop.reason, MonitorStopReason::Fired);
        assert_eq!(
            s.llm.child_calls("worker"),
            1,
            "the subagent stopped at the check after its first step"
        );
        assert_eq!(
            decisions.requests(),
            1,
            "the subagent asked the orchestrator's decision service"
        );
        assert!(
            response(&messages, "p1-watch").contains("Stopped by monitor 'Off task'"),
            "the orchestrator's watch read the reason"
        );
    });
}

/// A subagent's `turn_end` monitors run as its turn ends: a `turn_end` steer
/// the agent put on it sends it back to work once, with the monitor's note.
#[test]
fn turn_end_monitors_run_in_a_subagent_session() {
    run(async {
        let decisions = Decisions::fixed(0.9);
        let check = agent_spec(
            "Checked its work",
            "Has the agent finished without checking its work?",
            "steer",
            "turn_end",
            0.5,
        );
        let llm = SwarmLlm::new(vec![
            Step::Spawn(vec![
                Child::new("steer-once:checker").with_monitors(json!([check]))
            ]),
            Step::WatchChildren,
            Step::Text("The checker is done."),
        ]);
        let s = swarm(llm, &decisions, vec![]).await;

        let messages = within(Duration::from_secs(60), "the turn", send(&s, 8)).await;

        assert_eq!(
            s.llm.child_calls("checker"),
            2,
            "the subagent answered the monitor's note once"
        );
        assert!(
            s.llm
                .child_saw_on_call("checker", 1)
                .iter()
                .any(|text| text.contains("<decision-monitor>")),
            "{:?}",
            s.llm.child_saw_on_call("checker", 1)
        );
        let result = result_of(&child_named(&s.session_id, "checker"));
        assert_eq!(
            result.status,
            SubagentStatus::Completed,
            "{}",
            result.summary
        );
        assert!(
            result.summary.contains("child checker done, and checked"),
            "{}",
            result.summary
        );
        assert!(result.stopped_by_monitor.is_none());
        assert!(
            result
                .decisions
                .as_deref()
                .is_some_and(|marker| marker.contains("Checked its work")),
            "the result carries what the subagent's monitors did: {:?}",
            result.decisions
        );
        assert!(response(&messages, "p1-watch").contains("child checker done, and checked"));
    });
}

/// A copy of the person's monitor, set on the orchestrator to also watch
/// every subagent it starts, fires in one subagent: that subagent stops, its
/// running siblings are stopped at once with a sentence saying why, and the
/// orchestrator stops at its next point, naming the subagent.
#[test]
fn an_inherited_person_stop_in_one_child_stops_its_siblings_and_the_parent() {
    run(async {
        let _settle = SETTLE.read().await;
        let decisions = Decisions::showing(TRIGGER, 0.95, 0.05);
        let mut form = person_monitor("form", "Form reached", 0.85);
        form.apply_to_subagents = true;
        let llm = SwarmLlm::new(vec![
            Step::Spawn(vec![
                Child::new("trip:filler"),
                Child::new("hang:dates"),
                Child::new("hang:trials"),
            ]),
            Step::Text("Waiting on the subagents."),
        ])
        // The orchestrator's next call comes once the filler has stopped.
        .on_call(
            1,
            Arc::new(|cx: Cx| {
                Box::pin(async move {
                    let deadline = Instant::now() + Duration::from_secs(30);
                    while Instant::now() < deadline
                        && child_named(&cx.parent, "filler").result().is_none()
                    {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
            }),
        );
        let s = swarm(llm, &decisions, vec![form]).await;

        let messages = within(Duration::from_secs(40), "the turn", send(&s, 8)).await;

        let filler = child_named(&s.session_id, "filler");
        let filler_title = filler.title.clone();
        for handle in children(&s.session_id) {
            let monitors =
                biorouter::decision::settings::load(&s.sessions, &handle.child_session_id)
                    .await
                    .unwrap()
                    .monitors;
            assert_eq!(monitors.len(), 1, "{}", handle.title);
            assert_eq!(monitors[0].id, "form");
            assert_eq!(monitors[0].set_by, MonitorOwner::Person);
            assert_eq!(
                monitors[0].inherited_from.as_deref(),
                Some(s.session_id.as_str())
            );
        }
        let stopped = result_of(&filler);
        assert_eq!(stopped.status, SubagentStatus::Incomplete);
        assert!(
            stopped.summary.starts_with(
                "Stopped by monitor 'Form reached' (the person's, from this chat): P 0.95 \
                 reached its threshold 0.85."
            ),
            "{}",
            stopped.summary
        );
        let stop = stopped.stopped_by_monitor.expect("the filler's stop");
        assert!(stop.inherited);
        assert_eq!(stop.set_by, MonitorOwner::Person);
        assert_eq!(
            decisions.shown_containing(TRIGGER),
            1,
            "the filler's own check read what it wrote"
        );
        assert!(
            decisions.requests() >= 2,
            "the orchestrator's own copy was checked too, and did not fire"
        );

        let sibling_cause = ChildStopCause::SiblingMonitor {
            monitor: "Form reached".into(),
            sibling: filler_title.clone(),
        };
        for name in ["dates", "trials"] {
            let sibling = child_named(&s.session_id, name);
            assert!(sibling.is_cancelled(), "{name}");
            assert_eq!(sibling.stop_cause(), Some(sibling_cause.clone()), "{name}");
            let result = result_of(&sibling);
            assert_eq!(result.status, SubagentStatus::Incomplete, "{name}");
            assert!(
                result.summary.starts_with(&sibling_cause.sentence()),
                "{name}: {}",
                result.summary
            );
            assert!(
                s.llm.child_calls(name) <= 1,
                "{name} was stopped in its first call"
            );
        }

        let shown = notices(&messages);
        let escalated: Vec<&String> = shown
            .iter()
            .filter(|n| {
                n.starts_with(
                    "Monitor 'Form reached' (set to also watch this chat's subagents) stopped \
                     subagent 'Work on filler. TASK:trip:filler",
                ) && n.contains(
                    "' (P 0.95, threshold 0.85), so this chat and its other subagents stop here.",
                )
            })
            .collect();
        assert_eq!(escalated.len(), 1, "{shown:?}");
        assert_eq!(
            s.llm.parent_calls(),
            2,
            "the orchestrator stopped at its next point"
        );
        let report = s
            .agent
            .take_monitor_stop(&s.session_id)
            .expect("the orchestrator's stop");
        assert_eq!(
            report.reason,
            MonitorStopReason::ChildStopped {
                child_session_id: filler.child_session_id.clone(),
                child_title: filler_title,
            }
        );
        assert_eq!(
            collected(&messages),
            3,
            "what each subagent did is collected"
        );
    });
}

/// `workspace_monitor` puts a monitor on a subagent that is already running,
/// and the subagent applies it at its next check.
#[test]
fn workspace_monitor_on_a_running_child_applies_at_its_next_check() {
    run(async {
        let _settle = SETTLE.read().await;
        let decisions = Decisions::fixed(0.95);
        let llm = SwarmLlm::new(vec![
            Step::Spawn(vec![Child::new("await-monitor:worker")]),
            Step::MonitorChild(off_task()),
            Step::WatchChildren,
            Step::Text("The worker was stopped by its monitor."),
        ])
        // The monitor goes on once the subagent's turn is under way: it has
        // read its monitors (none) and taken its first step.
        .on_call(
            1,
            Arc::new(|cx: Cx| {
                Box::pin(async move {
                    let llm = cx.llm.upgrade().expect("the scripted model");
                    let deadline = Instant::now() + Duration::from_secs(30);
                    while Instant::now() < deadline && llm.child_calls("worker") == 0 {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
            }),
        );
        let s = swarm(llm, &decisions, vec![]).await;

        let messages = within(Duration::from_secs(60), "the turn", send(&s, 8)).await;

        let worker = child_named(&s.session_id, "worker");
        let added = response(&messages, "p1-monitor");
        assert!(
            added.contains(&format!(
                "Monitor 'Off task' is on subagent {}, set by you: it stops the subagent when \
                 P(yes) reaches 0.80, checked after each of its steps with {JOEY}. A running \
                 subagent applies it from its next check.",
                worker.child_session_id
            )),
            "{added}"
        );
        let monitors = biorouter::decision::settings::load(&s.sessions, &worker.child_session_id)
            .await
            .unwrap()
            .monitors;
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].set_by, MonitorOwner::Agent);
        assert_eq!(
            monitors[0].set_by_session.as_deref(),
            Some(s.session_id.as_str())
        );

        let result = result_of(&worker);
        assert_eq!(
            result.status,
            SubagentStatus::Incomplete,
            "{}",
            result.summary
        );
        let stop = result
            .stopped_by_monitor
            .expect("the monitor added mid-run stopped it");
        assert_eq!(stop.name, "Off task");
        assert_eq!(stop.set_by, MonitorOwner::Agent);
        assert_eq!(stop.reason, MonitorStopReason::Fired);
        assert_eq!(stop.p, Some(0.95), "answered by the orchestrator's service");
        assert_eq!(decisions.requests(), 1, "checked once, and it stopped");
        assert!(
            s.llm.child_calls("worker") <= 2,
            "nothing ran after the check that stopped it"
        );
        assert!(response(&messages, "p2-watch").contains("Stopped by monitor 'Off task'"));
    });
}

// ---------------------------------------------------------------------------
// Item B.
// ---------------------------------------------------------------------------

/// A monitor's stop on the orchestrator stops the subagents it started
/// (they would go on spending against a stop the person set), collects what
/// each had done, and says so. Without the cascade the turn would wait out
/// their two-minute calls.
#[test]
fn a_monitor_stop_on_an_orchestrator_stops_its_children_and_collects_them() {
    run(async {
        let _settle = SETTLE.read().await;
        let decisions = Decisions::fixed(0.95);
        let llm = SwarmLlm::new(vec![Step::Spawn(vec![
            Child::new("hang:trials"),
            Child::new("hang:notes"),
        ])]);
        let s = swarm(
            llm,
            &decisions,
            vec![person_monitor("form", "Form reached", 0.85)],
        )
        .await;

        let messages = within(
            Duration::from_secs(10),
            "the stopped turn ending without its subagents' own pace",
            send(&s, 8),
        )
        .await;

        assert_eq!(s.llm.parent_calls(), 1);
        assert_eq!(decisions.requests(), 1);
        let started = children(&s.session_id);
        assert_eq!(started.len(), 2);
        for child in &started {
            assert!(child.is_cancelled(), "{}", child.title);
            assert_eq!(
                child.stop_cause(),
                Some(ChildStopCause::ParentMonitor {
                    monitor: "Form reached".into()
                }),
                "{}",
                child.title
            );
            let result = result_of(child);
            assert_eq!(result.status, SubagentStatus::Incomplete, "{}", child.title);
            assert!(
                result.summary.starts_with(FORM_STOP_SENTENCE),
                "{}",
                result.summary
            );
            let own_chat = stored(&s.sessions, &child.child_session_id).await;
            assert!(
                own_chat.iter().any(|m| shows(m, FORM_STOP_SENTENCE)),
                "the subagent's own chat says why it stopped"
            );
        }
        let shown = notices(&messages);
        let cascade: Vec<&String> = shown
            .iter()
            .filter(|n| {
                n.starts_with("It also stopped 2 subagents this chat started ('Work on ")
                    && n.contains("'Work on trials. TASK:hang:trials")
                    && n.contains("'Work on notes. TASK:hang:notes")
                    && n.ends_with("'). What each had done is collected below.")
            })
            .collect();
        assert_eq!(cascade.len(), 1, "{shown:?}");
        assert_eq!(collected(&messages), 2, "what each had done is collected");
        let rows = stored(&s.sessions, &s.session_id).await;
        assert!(
            position(&rows, "so the turn stops here") < position(&rows, "It also stopped 2"),
            "the stop's notice comes first"
        );
    });
}

/// The wait for the subagents a monitor's stop stopped is bounded: one that
/// does not stop is named, and the turn ends without it.
#[test]
fn the_wait_after_a_monitor_stop_is_bounded() {
    run(async {
        let _settle = SETTLE.write().await;
        struct Unset;
        impl Drop for Unset {
            fn drop(&mut self) {
                std::env::remove_var("BIOROUTER_MONITOR_STOP_SETTLE_SECS");
            }
        }
        std::env::set_var("BIOROUTER_MONITOR_STOP_SETTLE_SECS", "1");
        let _unset = Unset;
        let decisions = Decisions::fixed(0.95);
        let llm = SwarmLlm::new(vec![Step::Spawn(vec![Child::new("hang:notes")])]);
        let s = swarm(
            llm,
            &decisions,
            vec![person_monitor("form", "Form reached", 0.85)],
        )
        .await;
        let stuck = stuck_child(&s.session_id, "Sleep for five minutes");

        let messages = within(Duration::from_secs(20), "the bounded wait", send(&s, 8)).await;

        assert!(stuck.is_cancelled(), "it was asked to stop");
        assert!(stuck.is_running(), "and never did");
        let shown = notices(&messages);
        assert_eq!(
            count_containing(&shown, "It also stopped 2 subagents this chat started"),
            1,
            "{shown:?}"
        );
        let settle: Vec<&String> = shown
            .iter()
            .filter(|n| n.contains("had not stopped 1 second after the monitor stopped this chat"))
            .collect();
        assert_eq!(settle.len(), 1, "{shown:?}");
        assert!(
            settle[0].contains("'Sleep for five minutes'")
                && settle[0].contains(
                    "They are still being stopped; their results are collected when this chat \
                     carries on."
                ),
            "{}",
            settle[0]
        );
        let notes = child_named(&s.session_id, "notes");
        within(
            Duration::from_secs(10),
            "the real subagent stopping",
            async {
                while notes.is_running() {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            },
        )
        .await;
        assert!(result_of(&notes).summary.starts_with(FORM_STOP_SENTENCE));
        stuck.complete(SubagentResult::from_error("stopped at last"));
    });
}

/// A steer the person types while the turn a monitor stopped waits for its
/// subagents ends the wait, and the turn goes on with it.
#[test]
fn a_steer_still_ends_the_wait_after_a_monitor_stop() {
    run(async {
        let _settle = SETTLE.read().await;
        const STEER: &str = "Leave the slow one and summarise what you have.";
        let decisions = Decisions::fixed(0.95);
        let mut form = person_monitor("form", "Form reached", 0.85);
        // Stops once, so the turn the steer continues is not stopped again.
        form.rearm_below = Some(0.3);
        let llm = SwarmLlm::new(vec![Step::Spawn(vec![Child::new("hang:notes")])]);
        let s = swarm(llm, &decisions, vec![form]).await;
        let stuck = stuck_child(&s.session_id, "Sleep for five minutes");

        // The daemon's runner opens a turn it may continue for a steer.
        s.agent.prepare_continuable_soft_interrupt_turn();
        let agent = s.agent.clone();
        let first = config(&s, 8);
        let turn = tokio::spawn(async move {
            let stream = agent
                .reply(Message::user().with_text(ASK), first, None)
                .await
                .expect("a reply stream");
            drain(&agent, stream).await
        });
        within(
            Duration::from_secs(20),
            "the stop's wait for its subagents",
            async {
                while !(stuck.is_cancelled()
                    && s.agent.loop_phase_snapshot().0 == LoopPhase::SupervisionWait)
                {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            },
        )
        .await;
        s.agent
            .try_queue_soft_interrupt(STEER.to_string(), None)
            .expect("a steer while the stop waits for its subagents is accepted");
        let messages = within(Duration::from_secs(15), "the steer ending the wait", turn)
            .await
            .expect("the turn task joins");

        assert!(
            stuck.is_running(),
            "the steer, not the subagent, ended the wait"
        );
        let shown = notices(&messages);
        assert_eq!(
            count_containing(&shown, "It also stopped 2 subagents this chat started"),
            1,
            "{shown:?}"
        );
        assert_eq!(count_containing(&shown, "had not stopped"), 0, "{shown:?}");
        let steer = s
            .agent
            .take_continuation_steer(&s.session_id)
            .expect("the turn continues with the steer");
        assert_eq!(steer.as_concat_text(), STEER);

        stuck.complete(SubagentResult::from_error("stopped at last"));
        let continued = s
            .agent
            .continue_turn_for_steer(steer, config(&s, 3), None)
            .await
            .expect("the continuation opens");
        within(
            Duration::from_secs(20),
            "the continuation",
            drain(&s.agent, continued),
        )
        .await;
        assert!(
            s.llm.parent_saw(STEER),
            "the orchestrator answered the steer"
        );
    });
}

/// A turn-limit stop is not a monitor's stop: the orchestrator's subagents
/// are not stopped, and the turn waits for them as before.
#[test]
fn a_turn_limit_stop_still_waits_for_its_children() {
    run(async {
        let decisions = Decisions::fixed(0.05);
        let llm = SwarmLlm::new(vec![Step::Spawn(vec![
            Child::new("slow:alpha:1500"),
            Child::new("slow:beta:1500"),
        ])]);
        let s = swarm(
            llm,
            &decisions,
            vec![person_monitor("form", "Form reached", 0.85)],
        )
        .await;

        let messages = within(Duration::from_secs(40), "the turn", send(&s, 2)).await;

        assert!(
            messages
                .iter()
                .any(|m| m.as_concat_text().contains("reached my action limit")),
            "the turn ended at its action limit"
        );
        assert!(
            decisions.requests() >= 1,
            "the monitor was checked, and did not fire"
        );
        let shown = notices(&messages);
        assert_eq!(count_containing(&shown, "It also stopped"), 0, "{shown:?}");
        assert_eq!(count_containing(&shown, "so the turn stops here"), 0);
        for name in ["alpha", "beta"] {
            let child = child_named(&s.session_id, name);
            assert!(!child.is_cancelled(), "{name}");
            assert_eq!(child.stop_cause(), None, "{name}");
            let result = result_of(&child);
            assert_eq!(result.status, SubagentStatus::Completed, "{name}");
            assert!(result.summary.contains(&format!("child {name} done")));
        }
        assert_eq!(collected(&messages), 2, "the turn waited for both");
    });
}

/// A steer the person typed is waiting when the orchestrator's monitor
/// fires: the stop is held for the steer, and so is the cascade. The steer is
/// answered with the subagents still running; the next check stops the turn
/// and them.
#[test]
fn a_steer_waiting_in_the_orchestrator_suppresses_the_stop_and_the_cascade() {
    run(async {
        let _settle = SETTLE.read().await;
        const STEER: &str = "Check the dates first.";
        let decisions = Decisions::fixed(0.95);
        let at_the_steer: Arc<Mutex<Option<(usize, usize)>>> = Arc::new(Mutex::new(None));
        let record = at_the_steer.clone();
        let llm = SwarmLlm::new(vec![
            Step::Spawn(vec![Child::new("hang:dates"), Child::new("hang:trials")]),
            Step::Text("Checking the dates."),
        ])
        .on_call(
            0,
            Arc::new(|cx: Cx| {
                Box::pin(async move {
                    cx.agent
                        .try_queue_soft_interrupt(STEER.to_string(), None)
                        .expect("a steer offered mid-turn is accepted");
                })
            }),
        )
        .on_call(
            1,
            Arc::new(move |cx: Cx| {
                let record = record.clone();
                Box::pin(async move {
                    let started = children(&cx.parent);
                    let cancelled = started.iter().filter(|c| c.is_cancelled()).count();
                    let running = started.iter().filter(|c| c.is_running()).count();
                    *record.lock().unwrap() = Some((cancelled, running));
                })
            }),
        );
        let s = swarm(
            llm,
            &decisions,
            vec![person_monitor("form", "Form reached", 0.85)],
        )
        .await;

        let messages = within(Duration::from_secs(20), "the turn", send(&s, 8)).await;

        assert_eq!(s.llm.parent_calls(), 2, "the person's message was answered");
        assert!(s.llm.parent_saw(STEER));
        assert_eq!(
            *at_the_steer.lock().unwrap(),
            Some((0, 2)),
            "(cancelled, running) when the steer was answered: the held stop stopped no subagent"
        );
        assert_eq!(
            decisions.requests(),
            2,
            "checked while the steer waited, and after it"
        );
        let shown = notices(&messages);
        assert_eq!(
            count_containing(&shown, "so the turn stops here"),
            1,
            "{shown:?}"
        );
        assert_eq!(
            count_containing(&shown, "It also stopped 2 subagents this chat started"),
            1,
            "{shown:?}"
        );
        for child in children(&s.session_id) {
            assert_eq!(
                child.stop_cause(),
                Some(ChildStopCause::ParentMonitor {
                    monitor: "Form reached".into()
                }),
                "{}",
                child.title
            );
        }
        let rows = stored(&s.sessions, &s.session_id).await;
        assert!(
            position(&rows, STEER) < position(&rows, "so the turn stops here"),
            "the stop waited for the person's message"
        );
    });
}

// ---------------------------------------------------------------------------
// Item 1.
// ---------------------------------------------------------------------------

/// A monitor saved on the orchestrator after it fanned out (in another
/// window, say) is checked at its next point in the same swarm turn: a swarm
/// run is one long turn, so a monitor read only at turn start would never run.
#[test]
fn a_monitor_added_after_the_fan_out_runs_during_the_same_swarm_turn() {
    run(async {
        let _settle = SETTLE.read().await;
        let decisions = Decisions::fixed(0.95);
        let llm = SwarmLlm::new(vec![
            Step::Spawn(vec![Child::new("hang:dates"), Child::new("hang:trials")]),
            Step::Text("Waiting on the subagents."),
        ])
        .on_call(
            1,
            Arc::new(|cx: Cx| {
                Box::pin(async move {
                    save_monitors(
                        &cx.sessions,
                        &cx.parent,
                        vec![person_monitor("fanned", "Fanned out", 0.85)],
                    )
                    .await;
                })
            }),
        );
        let s = swarm(llm, &decisions, vec![]).await;

        let messages = within(Duration::from_secs(20), "the turn", send(&s, 8)).await;

        assert_eq!(s.llm.parent_calls(), 2);
        assert_eq!(decisions.requests(), 1, "the new monitor was checked once");
        let shown = notices(&messages);
        assert_eq!(
            count_containing(
                &shown,
                "Monitor 'Fanned out' fired (P 0.95, threshold 0.85), so the turn stops here."
            ),
            1,
            "{shown:?}"
        );
        assert_eq!(
            count_containing(&shown, "It also stopped 2 subagents this chat started"),
            1,
            "{shown:?}"
        );
        for child in children(&s.session_id) {
            assert_eq!(
                child.stop_cause(),
                Some(ChildStopCause::ParentMonitor {
                    monitor: "Fanned out".into()
                }),
                "{}",
                child.title
            );
        }
    });
}
