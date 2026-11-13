//! Issue #56: a model switch that lands while a turn runs.
//!
//! The reply loop pins the provider a turn's requests run on for the whole
//! turn, while Gate C (`Agent::dispatch_tool_call`) and Gate E
//! (`Agent::list_tools`) read the agent's slot on every call. A person who
//! switched a public chat to a private model while a turn ran therefore had
//! private extension calls admitted for the rest of that turn, and their
//! results went to the public model the turn was still running on.
//!
//! A turn's capability is now never wider than the one it started with: a
//! switch to a more private model applies from the next turn, and a switch to a
//! less private one applies at once.

use super::*;
use crate::agents::AgentConfig;
use crate::config::permission::PermissionManager;
use crate::config::BioRouterMode;
use crate::model::ModelConfig;
use crate::privacy::{CallCapability, ModelAffiliation};
use crate::providers::base::{ProviderMetadata, ProviderUsage, Usage};
use std::sync::atomic::AtomicUsize;
use std::sync::{Mutex as StdMutex, Weak};
use tempfile::TempDir;

/// One of the two extensions the compiled-in BAAM baseline calls private.
const PRIVATE_EXTENSION: &str = "ucsfomopagent";
const PRIVATE_TOOL: &str = "ucsfomopagent__data_sources";
/// The words every tier refusal of a private extension carries.
const REFUSED: &str = "is a private extension";
/// The words only the refusal after a mid-turn switch to a private model carries.
const NEXT_MESSAGE: &str = "takes effect from the next message";
/// The words the usual refusal asks the person to act on.
const ASK_TO_SWITCH: &str = "switch this chat to a private model";

/// What a scripted model does to the chat on its first call, before it answers.
enum Switch {
    /// A person's switch, through the bind door (`update_provider`, Gate A).
    Bind(Arc<dyn Provider>),
    /// A less private model reaching the slot. No door does this to a chat
    /// that is already private (Gate A refuses it), so the test installs it
    /// through the slot's one writer, as a lowering would.
    Install(Arc<dyn Provider>),
}

/// A model that answers from a script, can switch the chat it serves on its
/// first call, and records what it was offered and shown.
struct Scripted {
    name: &'static str,
    tier: ProviderTier,
    script: Vec<Message>,
    calls: AtomicUsize,
    chat: StdMutex<Option<(Weak<Agent>, String)>>,
    switch: StdMutex<Option<Switch>>,
    /// Per call: whether the model-facing roster, read after any switch, held
    /// the private tool.
    offered: StdMutex<Vec<bool>>,
    /// Per call: the newest tool result the model was shown, as text.
    seen: StdMutex<Vec<String>>,
}

impl Scripted {
    fn new(name: &'static str, tier: ProviderTier, script: Vec<Message>) -> Arc<Self> {
        Arc::new(Self {
            name,
            tier,
            script,
            calls: AtomicUsize::new(0),
            chat: StdMutex::new(None),
            switch: StdMutex::new(None),
            offered: StdMutex::new(Vec::new()),
            seen: StdMutex::new(Vec::new()),
        })
    }

    fn serve(&self, agent: &Arc<Agent>, session_id: &str) {
        *self.chat.lock().unwrap() = Some((Arc::downgrade(agent), session_id.to_string()));
    }

    fn switch_on_first_call(&self, switch: Switch) {
        *self.switch.lock().unwrap() = Some(switch);
    }

    fn offered(&self) -> Vec<bool> {
        self.offered.lock().unwrap().clone()
    }

    fn shown(&self, call: usize) -> String {
        self.seen
            .lock()
            .unwrap()
            .get(call)
            .cloned()
            .unwrap_or_else(|| panic!("{} was called fewer than {} times", self.name, call + 1))
    }
}

/// The newest tool result in `messages`, as text: what the model is shown of
/// the call it made one step earlier.
fn newest_tool_result(messages: &[Message]) -> String {
    messages
        .iter()
        .rev()
        .flat_map(|message| message.content.iter().rev())
        .find_map(|content| match content {
            MessageContent::ToolResponse(response) => Some(match &response.tool_result {
                Ok(result) => flatten_result(result),
                Err(error) => error.message.to_string(),
            }),
            _ => None,
        })
        .unwrap_or_default()
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        self.name
    }

    fn tier(&self) -> ProviderTier {
        self.tier
    }

    /// Every real private provider states an affiliation (DR-26). `Local` is
    /// the identity, so these tests stay about the tier axis.
    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.tier.is_private().then_some(ModelAffiliation::Local)
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("scripted-model")
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let chat = self.chat.lock().unwrap().clone();
        let switch = self.switch.lock().unwrap().take();
        if let Some((agent, session_id)) = chat.as_ref() {
            let agent = agent.upgrade().expect("the agent outlives its turn");
            match switch {
                Some(Switch::Bind(to)) => agent
                    .update_provider(to, session_id)
                    .await
                    .expect("a public chat may switch to a private model"),
                Some(Switch::Install(to)) => {
                    agent
                        .install_bound_provider(None, BoundInstall::Bind(to))
                        .await;
                }
                None => {}
            }
            let offered = agent
                .list_tools(session_id, None)
                .await
                .iter()
                .any(|tool| tool.name.as_ref() == PRIVATE_TOOL);
            self.offered.lock().unwrap().push(offered);
        }
        self.seen.lock().unwrap().push(newest_tool_result(messages));
        let reply = self
            .script
            .get(n)
            .cloned()
            .unwrap_or_else(|| Message::assistant().with_text("(script exhausted)"));
        Ok((
            reply,
            ProviderUsage::new(
                "scripted-model".to_string(),
                Usage::new(Some(10), Some(5), Some(15)),
            ),
        ))
    }
}

fn call_private_tool(id: &str) -> Message {
    Message::assistant().with_tool_request(
        id,
        Ok(CallToolRequestParams {
            task: None,
            meta: None,
            name: PRIVATE_TOOL.into(),
            arguments: Some(rmcp::object!({})),
        }),
    )
}

fn done() -> Message {
    Message::assistant().with_text("done")
}

struct Fixture {
    agent: Arc<Agent>,
    session_id: String,
    _dirs: Vec<TempDir>,
}

/// An agent on its own store, in Completely Autonomous mode, with one chat
/// bound to `provider` and the private extension loaded.
async fn fixture(provider: Arc<dyn Provider>) -> Fixture {
    let data = TempDir::new().unwrap();
    let work = TempDir::new().unwrap();
    let permissions = TempDir::new().unwrap();
    let session_manager = Arc::new(SessionManager::new(data.path().to_path_buf()));
    let agent = Arc::new(Agent::with_config(
        AgentConfig::new(
            Arc::clone(&session_manager),
            Arc::new(PermissionManager::new(permissions.path().to_path_buf())),
            None,
            BioRouterMode::Auto,
        )
        .with_project_hooks(false),
    ));
    // A real in-process MCP server admitted under a private NAME, so the tier
    // the gates read is stamped by the production admission path.
    agent
        .extension_manager
        .add_inprocess_server(
            PRIVATE_EXTENSION,
            biorouter_mcp::datasql::server::DataSqlServer::new(HashMap::new()),
        )
        .await
        .expect("inject the private extension");
    let session = session_manager
        .create_session(
            work.path().to_path_buf(),
            "mid-turn switch".to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    agent
        .update_provider(provider, &session.id)
        .await
        .expect("bind the chat's first model");
    Fixture {
        agent,
        session_id: session.id,
        _dirs: vec![data, work, permissions],
    }
}

/// Drive one user turn to its end.
async fn turn(fixture: &Fixture, prompt: &str) {
    let stream = fixture
        .agent
        .reply(
            Message::user().with_text(prompt),
            SessionConfig {
                id: fixture.session_id.clone(),
                schedule_id: None,
                max_turns: Some(10),
                max_tool_calls: None,
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            None,
        )
        .await
        .expect("the turn starts");
    tokio::pin!(stream);
    while let Some(event) = stream.next().await {
        event.expect("the turn runs");
    }
}

/// The headline. A public chat with a private extension loaded; the person
/// switches it to a private model while the public model's turn runs. For the
/// rest of that turn the private tool is neither offered (Gate E) nor run
/// (Gate C), so its result never reaches the public model. The next turn runs
/// on the private model, which is offered and may call it.
#[tokio::test]
async fn a_switch_to_a_private_model_waits_for_the_next_turn() {
    let private = Scripted::new(
        "mid_turn_private",
        ProviderTier::Private,
        vec![call_private_tool("second"), done()],
    );
    let public = Scripted::new(
        "mid_turn_public",
        ProviderTier::Public,
        vec![call_private_tool("first"), done()],
    );
    public.switch_on_first_call(Switch::Bind(private.clone()));
    let fx = fixture(public.clone()).await;
    public.serve(&fx.agent, &fx.session_id);
    private.serve(&fx.agent, &fx.session_id);

    turn(&fx, "use the clinical data").await;
    assert_eq!(
        fx.agent.bound_model_tier().await,
        ProviderTier::Private,
        "precondition: the switch landed while the public model's turn ran"
    );
    assert_eq!(
        public.offered(),
        vec![false, false],
        "the public model's turn was offered {PRIVATE_TOOL} after the switch"
    );
    let shown = public.shown(1);
    assert!(
        shown.contains(REFUSED),
        "the public model was handed the private extension's result instead of a \
         refusal: {shown}"
    );
    assert!(
        shown.contains(NEXT_MESSAGE) && !shown.contains(ASK_TO_SWITCH),
        "the person just switched this chat to a private model, and the refusal asked \
         for that switch again instead of saying it starts with the next message: {shown}"
    );
    assert_eq!(
        fx.agent
            .config
            .session_manager
            .get_session(&fx.session_id, false)
            .await
            .unwrap()
            .privacy_tier,
        SessionClassification::Public,
        "a refused call must not ratchet the chat"
    );

    turn(&fx, "now use it").await;
    assert_eq!(
        private.offered(),
        vec![true, true],
        "the next turn runs on the private model and is offered {PRIVATE_TOOL}"
    );
    let shown = private.shown(1);
    assert!(
        !shown.contains(REFUSED),
        "the next turn on the private model was refused: {shown}"
    );
}

/// The other direction applies at once. A turn that started on a private model
/// whose slot then holds a public one: from that moment the private tool is
/// neither offered nor run, and the turn's own model is shown the refusal.
///
/// No door does this to a chat already ratcheted Private (Gate A refuses a
/// public bind there), so the public model reaches the slot through its one
/// writer, the way a lowering would.
#[tokio::test]
async fn a_switch_to_a_less_private_model_applies_at_once() {
    let public = Scripted::new("mid_turn_public", ProviderTier::Public, vec![]);
    let private = Scripted::new(
        "mid_turn_private",
        ProviderTier::Private,
        vec![call_private_tool("first"), done()],
    );
    private.switch_on_first_call(Switch::Install(public.clone()));
    let fx = fixture(private.clone()).await;
    private.serve(&fx.agent, &fx.session_id);

    let errors = turn_collecting_errors(&fx, "use the clinical data").await;
    assert_eq!(
        fx.agent.bound_model_tier().await,
        ProviderTier::Public,
        "precondition: the public model reached the slot while the turn ran ({errors:?})"
    );
    assert_eq!(
        private.offered().first(),
        Some(&false),
        "the private tool was still offered after the slot moved to a public model"
    );
    let shown = private.shown(1);
    assert!(
        shown.contains(REFUSED),
        "the private tool ran after the slot moved to a public model: {shown}"
    );
    assert!(
        shown.contains(ASK_TO_SWITCH) && !shown.contains(NEXT_MESSAGE),
        "a chat now on a public model keeps the usual refusal: {shown}"
    );
}

/// Drive one user turn to its end, returning the errors the stream carried
/// rather than failing on them.
async fn turn_collecting_errors(fixture: &Fixture, prompt: &str) -> Vec<String> {
    let stream = fixture
        .agent
        .reply(
            Message::user().with_text(prompt),
            SessionConfig {
                id: fixture.session_id.clone(),
                schedule_id: None,
                max_turns: Some(10),
                max_tool_calls: None,
                budget: None,
                retry_config: None,
                reasoning_effort: None,
            },
            None,
        )
        .await
        .expect("the turn starts");
    tokio::pin!(stream);
    let mut errors = Vec::new();
    while let Some(event) = stream.next().await {
        if let Err(error) = event {
            errors.push(error.to_string());
        }
    }
    errors
}

fn spawn_call() -> CallToolRequestParams {
    CallToolRequestParams {
        task: None,
        meta: None,
        name: crate::agents::subagent_tool::SUBAGENT_TOOL_PREFIXED.into(),
        arguments: Some(rmcp::object!({ "instructions": "summarise the clinical data" })),
    }
}

async fn dispatch_private_tool(fx: &Fixture) -> std::result::Result<String, String> {
    let session = fx
        .agent
        .config
        .session_manager
        .get_session(&fx.session_id, false)
        .await
        .unwrap();
    let (_id, result) = fx
        .agent
        .dispatch_tool_call(
            CallToolRequestParams {
                task: None,
                meta: None,
                name: PRIVATE_TOOL.into(),
                arguments: Some(rmcp::object!({})),
            },
            "req".to_string(),
            None,
            &session,
        )
        .await;
    match result {
        Ok(call) => match call.result.await {
            Ok(result) => Ok(flatten_result(&result)),
            Err(error) => Err(error.message.to_string()),
        },
        Err(error) => Err(error.message.to_string()),
    }
}

fn flatten_result(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A turn's ceiling holds exactly as long as the turn: while it is open, a
/// slot moved to a private model gains nothing (Gate C refuses, Gate E hides);
/// once the turn's hold is dropped the same slot reaches the private tool.
#[tokio::test]
async fn the_ceiling_holds_for_the_turn_and_no_longer() {
    let public = Scripted::new("mid_turn_public", ProviderTier::Public, vec![]);
    let private = Scripted::new("mid_turn_private", ProviderTier::Private, vec![]);
    let fx = fixture(public.clone()).await;
    let started_public = fx.agent.call_capability(&fx.session_id).await;
    assert_eq!(started_public.tier(), ProviderTier::Public);

    let held = fx.agent.turn_ceilings.open(&fx.session_id, started_public);
    fx.agent
        .update_provider(private.clone(), &fx.session_id)
        .await
        .unwrap();
    let refusal = dispatch_private_tool(&fx)
        .await
        .expect_err("a turn that started public reached the private tool");
    assert!(refusal.contains(REFUSED), "{refusal}");
    assert_eq!(
        refusal,
        crate::privacy::refusal::private_from_next_turn_sentence(PRIVATE_EXTENSION),
        "the refusal after a switch to a private model must say it starts with the next \
         message"
    );
    assert!(!fx
        .agent
        .list_tools(&fx.session_id, None)
        .await
        .iter()
        .any(|tool| tool.name.as_ref() == PRIVATE_TOOL));

    drop(held);
    assert!(
        fx.agent.turn_ceilings.get(&fx.session_id).is_none(),
        "the ceiling outlived its turn"
    );
    dispatch_private_tool(&fx)
        .await
        .expect("with no turn running, a private model reaches the private tool");
    assert!(fx
        .agent
        .list_tools(&fx.session_id, None)
        .await
        .iter()
        .any(|tool| tool.name.as_ref() == PRIVATE_TOOL));
}

/// A later turn's ceiling is not removed by an earlier turn's hold. A Stop can
/// drop the earlier stream after the next turn has opened its own.
#[tokio::test]
async fn an_earlier_turns_hold_leaves_a_later_turns_ceiling_alone() {
    let public = Scripted::new("mid_turn_public", ProviderTier::Public, vec![]);
    let fx = fixture(public).await;
    let first = fx.agent.turn_ceilings.open(
        &fx.session_id,
        CallCapability::for_test(ProviderTier::Private, true),
    );
    let _second = fx.agent.turn_ceilings.open(
        &fx.session_id,
        CallCapability::for_test(ProviderTier::Public, true),
    );
    drop(first);
    assert_eq!(
        fx.agent
            .turn_ceilings
            .get(&fx.session_id)
            .map(|ceiling| ceiling.tier()),
        Some(ProviderTier::Public)
    );
}

/// A subagent spawned while a switch to a more private model waits for the
/// next turn would take the slot's reach and report to the model this turn
/// still runs on. It is refused, in a sentence that says why, and the same
/// spawn is not refused for that reason once the turn is over.
#[tokio::test]
async fn a_spawn_waits_for_the_next_turn_after_a_switch() {
    let public = Scripted::new("mid_turn_public", ProviderTier::Public, vec![]);
    let private = Scripted::new("mid_turn_private", ProviderTier::Private, vec![]);
    let fx = fixture(public.clone()).await;
    // The spawn tool is loaded the way a turn loads it, by listing the roster.
    fx.agent.list_tools(&fx.session_id, None).await;
    let started_public = fx.agent.call_capability(&fx.session_id).await;
    let held = fx.agent.turn_ceilings.open(&fx.session_id, started_public);
    fx.agent
        .update_provider(private.clone(), &fx.session_id)
        .await
        .unwrap();
    let session = fx
        .agent
        .config
        .session_manager
        .get_session(&fx.session_id, false)
        .await
        .unwrap();

    let (_id, result) = fx
        .agent
        .dispatch_tool_call(spawn_call(), "spawn".to_string(), None, &session)
        .await;
    let refusal = match result {
        Err(error) => error.message.to_string(),
        Ok(_) => panic!("a spawn ran while the switch waited for the next turn"),
    };
    assert_eq!(refusal, SPAWN_WAITS_FOR_NEXT_TURN);

    // Once the turn is over, the same slot no longer waits for anything. The
    // spawn itself is not driven: what it would start is the subagent tests'.
    drop(held);
    let inherited: SharedProvider = Arc::new(Mutex::new(Some(
        fx.agent
            .provider()
            .await
            .expect("the private model is bound"),
    )));
    assert!(
        !fx.agent
            .sample_turn_call(&fx.session_id, &inherited)
            .await
            .waits_for_next_turn(),
        "a spawn would be refused for a switch after its turn was over"
    );
}

/// Whether a spawn made now on a turn that started on `started_with`, with the
/// slot sampled as `slot`, waits for the next turn. Built the way
/// [`Agent::sample_turn_call`] builds it: the turn's ceiling held open, and the
/// admitted capability taken through [`Agent::capped_to_turn`].
fn spawn_waits(fx: &Fixture, started_with: CallCapability, slot: CallCapability) -> bool {
    let _held = fx.agent.turn_ceilings.open(&fx.session_id, started_with);
    TurnCall {
        slot,
        admitted: fx.agent.capped_to_turn(&fx.session_id, slot),
    }
    .waits_for_next_turn()
}

fn private_at(affiliation: ModelAffiliation, enforced: bool) -> CallCapability {
    CallCapability::for_test_affiliated(ProviderTier::Private, enforced, Some(affiliation))
}

fn institution(name: &str) -> ModelAffiliation {
    ModelAffiliation::institution(crate::privacy::affiliation::InstitutionId::new(name))
}

/// The spawn rule compares the affiliation as well as the tier, deliberately
/// (review of PR #484). Between two private models at different institutions
/// the tier alone would admit a spawn, but the child would take the slot's
/// institution, reach its connectors with no warning, and report to the model
/// this turn still runs on: the change DR-31 refuses across a spawn. A local
/// slot after an institutional turn waits too, because a local child reaches
/// every connector. A local turn is the identity, so the opposite switch waits
/// for nothing.
#[tokio::test]
async fn a_spawn_waits_for_a_switch_to_another_institution() {
    let fx = fixture(Scripted::new(
        "mid_turn_public",
        ProviderTier::Public,
        vec![],
    ))
    .await;
    let ucsf = private_at(institution("ucsf"), true);
    let stanford = private_at(institution("stanford"), true);
    let local = private_at(ModelAffiliation::Local, true);
    let public = CallCapability::for_test(ProviderTier::Public, true);

    // Precondition: the tier alone admits this spawn, so only the affiliation
    // arm refuses it.
    {
        let _held = fx.agent.turn_ceilings.open(&fx.session_id, ucsf);
        let admitted = fx.agent.capped_to_turn(&fx.session_id, stanford);
        assert_eq!(admitted.tier(), stanford.tier());
        assert_ne!(admitted.affiliation(), stanford.affiliation());
    }
    assert!(
        spawn_waits(&fx, ucsf, stanford),
        "a spawn after a switch from a UCSF model to a Stanford one was admitted, so a \
         child at Stanford would report Stanford's data to the UCSF model"
    );
    assert!(spawn_waits(&fx, stanford, ucsf));
    assert!(
        spawn_waits(&fx, ucsf, local),
        "a local child reaches every institution's connectors and would report to the \
         UCSF model this turn runs on"
    );
    assert!(
        !spawn_waits(&fx, local, ucsf),
        "a turn that started on a local model is covered by the slot's institution"
    );
    assert!(
        !spawn_waits(&fx, ucsf, ucsf),
        "no switch, nothing to wait for"
    );

    // The tier rows, for the whole table in one place.
    assert!(spawn_waits(&fx, public, ucsf));
    assert!(
        !spawn_waits(&fx, ucsf, public),
        "a less private slot applies at once"
    );
    // A ceiling and a slot that differ only in the decision-model flag, which
    // decides nothing, wait for nothing.
    let lowered = CallCapability::for_test_lowered_by_decision(true);
    assert!(!spawn_waits(&fx, lowered, public));
    assert!(!spawn_waits(&fx, public, lowered));

    // With the master switch off nothing is refused, on either axis.
    assert!(!spawn_waits(
        &fx,
        ucsf,
        private_at(institution("stanford"), false)
    ));
    assert!(!spawn_waits(
        &fx,
        public,
        CallCapability::for_test(ProviderTier::Private, false)
    ));
}

/// The refusal Gate C gives on a chat that was already public when its turn
/// started and is still public: the usual sentence, unchanged. Only a slot that
/// moved to a private model while the turn ran is told about the next message.
#[tokio::test]
async fn a_chat_that_stays_public_keeps_the_usual_refusal() {
    let public = Scripted::new("mid_turn_public", ProviderTier::Public, vec![]);
    let fx = fixture(public).await;
    let started_public = fx.agent.call_capability(&fx.session_id).await;
    let _held = fx.agent.turn_ceilings.open(&fx.session_id, started_public);
    let refusal = dispatch_private_tool(&fx)
        .await
        .expect_err("a public chat reached the private tool");
    assert!(
        refusal.contains(ASK_TO_SWITCH) && !refusal.contains(NEXT_MESSAGE),
        "{refusal}"
    );
}

/// The two knowledge-ingest tools whose `resolve_target_kb` result is shown
/// to the model, each asked with no target base so they answer with the
/// candidate list. Returns (conversation ingest, source ingest).
async fn knowledge_candidates(fx: &Fixture) -> (String, String) {
    let session = fx
        .agent
        .config
        .session_manager
        .get_session(&fx.session_id, false)
        .await
        .unwrap();
    let conversation = fx
        .agent
        .handle_ingest_conversation(serde_json::json!({}), &session, None)
        .await
        .expect_err("no target base, so the conversation ingest lists the candidates")
        .message
        .to_string();
    let source = fx
        .agent
        .handle_ingest_source(
            serde_json::json!({ "text": "meeting notes" }),
            &session,
            None,
        )
        .await
        .expect_err("no target base, so the source ingest lists the candidates")
        .message
        .to_string();
    (conversation, source)
}

/// Issue #56: the two knowledge-ingest branches of `dispatch_tool_call` return
/// before the loop's own sample, and their capability decides which knowledge
/// base ids `resolve_target_kb` names to the model. A public turn whose chat
/// was switched to a private model while it ran still answers on the public
/// model, so it is named no private base. Once the turn is over, the private
/// model is named both.
#[tokio::test]
async fn knowledge_ingest_names_no_private_base_after_a_mid_turn_switch() {
    // `KnowledgeService::new_default()` resolves the knowledge root from
    // `Paths`, so this runs alone, under a root no other test reads.
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let public = Scripted::new("mid_turn_public", ProviderTier::Public, vec![]);
    let private = Scripted::new("mid_turn_private", ProviderTier::Private, vec![]);
    let fx = fixture(public).await;
    let svc = biorouter_mcp::knowledge::service::KnowledgeService::new_default().unwrap();
    svc.create_base("open-notes", "Open notes", None).unwrap();
    svc.create_base("clinic-notes", "Clinic notes", None)
        .unwrap();
    crate::knowledge::tier::raise_unlocked(svc.root(), "clinic-notes", true).unwrap();
    // An explicit "no primary", so a KB-less ingest lists its candidates.
    svc.set_primary_for_session(&fx.session_id, None).unwrap();

    let started_public = fx.agent.call_capability(&fx.session_id).await;
    let held = fx.agent.turn_ceilings.open(&fx.session_id, started_public);
    fx.agent
        .update_provider(private, &fx.session_id)
        .await
        .unwrap();
    let (conversation, source) = knowledge_candidates(&fx).await;
    for named in [&conversation, &source] {
        assert!(named.contains("open-notes"), "{named}");
        assert!(
            !named.contains("clinic-notes"),
            "a turn that started public was named a private knowledge base after a switch \
             to a private model: {named}"
        );
    }

    drop(held);
    let (conversation, source) = knowledge_candidates(&fx).await;
    for named in [&conversation, &source] {
        assert!(
            named.contains("open-notes") && named.contains("clinic-notes"),
            "with no turn running, the private model is named its own bases: {named}"
        );
    }
}
