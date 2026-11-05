//! A decision model bound as a chat's main model, through the real
//! `Agent::reply`: the prompt is the state, the chat's question (or a question
//! line) is asked of it, and the reply is one stored assistant message whose
//! `decision_id` names the row the decision log stored. Offline: the decision
//! model is scripted behind the agent's own decision service, and the log is
//! the real SQLite sink over the chat's own store.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, AgentEvent, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::{Message, MessageContent};
use biorouter::decision::log::SqliteDecisionSink;
use biorouter::decision::model::DecisionCaps;
use biorouter::decision::settings::DecisionChatSettings;
use biorouter::decision::types::{DecisionKindTag, ModelVerdicts, QuestionVerdict};
use biorouter::decision::{
    DecisionAdapter, DecisionContext, DecisionKind, DecisionModel, DecisionModelEntry,
    DecisionModelRef, DecisionModelSource, DecisionOrigin, DecisionRequest, DecisionResult,
    DecisionService, ProbabilitySource, Question,
};
use biorouter::model::ModelConfig;
use biorouter::privacy::{ModelAffiliation, ProviderTier, SessionClassification};
use biorouter::providers::base::Provider;
use biorouter::providers::decision::{BasetenDecisionChatProvider, OllamaDecisionChatProvider};
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use futures::StreamExt;
use indexmap::IndexMap;
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

struct Decisions {
    tier: ProviderTier,
    answers: Mutex<VecDeque<f64>>,
    requests: AtomicUsize,
    seen: Mutex<Vec<DecisionRequest>>,
}

impl Decisions {
    fn new(tier: ProviderTier, answers: Vec<f64>) -> Arc<Self> {
        Arc::new(Self {
            tier,
            answers: Mutex::new(answers.into()),
            requests: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn entry(&self) -> DecisionModelEntry {
        let (provider, model) = match self.tier {
            ProviderTier::Private => ("ollama_decision", "tev1:0.8b"),
            ProviderTier::Public => ("baseten_decision", "joey-1"),
        };
        DecisionModelEntry {
            provider: provider.into(),
            model: model.into(),
            display_name: model.into(),
            context_tokens: 8_192,
            kinds: vec![
                DecisionKindTag::Binary,
                DecisionKindTag::Choice,
                DecisionKindTag::Score,
            ],
            tier: self.tier,
            affiliation: (self.tier == ProviderTier::Private).then_some(ModelAffiliation::Local),
            adapter: DecisionAdapter::SystemOne,
            local: false,
        }
    }
}

struct Model(Arc<Decisions>);

#[async_trait]
impl DecisionModel for Model {
    fn model_ref(&self) -> DecisionModelRef {
        self.0.entry().model_ref()
    }
    fn display_name(&self) -> String {
        self.0.entry().display_name
    }
    fn tier(&self) -> ProviderTier {
        self.0.tier
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.0.entry().affiliation
    }
    fn caps(&self) -> DecisionCaps {
        DecisionCaps {
            kinds: self.0.entry().kinds,
            max_questions: 64,
            context_tokens: 8_192,
            max_concurrency: 2,
            max_request_bytes: None,
            local: false,
        }
    }
    async fn decide(&self, request: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
        self.0.requests.fetch_add(1, Ordering::SeqCst);
        self.0.seen.lock().unwrap().push(request.clone());
        let p = self.0.answers.lock().unwrap().pop_front().unwrap_or(0.5);
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

struct Source(Arc<Decisions>);

#[async_trait]
impl DecisionModelSource for Source {
    async fn list(&self) -> Vec<DecisionModelEntry> {
        vec![self.0.entry()]
    }
    async fn open(&self, _wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
        Ok(Arc::new(Model(self.0.clone())))
    }
}

struct Chat {
    agent: Agent,
    sessions: Arc<SessionManager>,
    session_id: String,
    _dir: TempDir,
}

async fn decision_chat(decisions: &Arc<Decisions>, provider: Arc<dyn Provider>) -> Chat {
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
            "decision chat test".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    let service = DecisionService::new(Arc::new(Source(decisions.clone())))
        .with_default_model(None)
        .with_timeouts(Duration::from_secs(5), Duration::from_secs(5))
        .with_sink(Arc::new(SqliteDecisionSink {
            session_manager: sessions.clone(),
        }));
    agent.set_decision_service(Arc::new(service));
    agent.update_provider(provider, &session.id).await.unwrap();
    Chat {
        agent,
        sessions,
        session_id: session.id,
        _dir: dir,
    }
}

async fn send(chat: &Chat, text: &str) -> Vec<Message> {
    let config = SessionConfig {
        id: chat.session_id.clone(),
        schedule_id: None,
        max_turns: Some(4),
        max_tool_calls: None,
        retry_config: None,
        budget: None,
        reasoning_effort: None,
    };
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

fn assistant_texts(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter(|m| m.role == rmcp::model::Role::Assistant)
        .flat_map(|m| m.content.iter())
        .filter_map(|c| match c {
            MessageContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

fn ollama_tev1() -> Arc<dyn Provider> {
    Arc::new(OllamaDecisionChatProvider::with_model(
        ModelConfig::new_or_fail("tev1:0.8b"),
    ))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_line_is_asked_and_the_reply_names_the_logged_decision() {
    let decisions = Decisions::new(ProviderTier::Private, vec![0.93]);
    let chat = decision_chat(&decisions, ollama_tev1()).await;

    let messages = send(
        &chat,
        "Q: Does this note state the patient has multiple sclerosis?\n\
         SYNTHETIC NOTE: Neuro f/u. 34F, RRMS dx 2019.",
    )
    .await;

    assert_eq!(decisions.requests.load(Ordering::SeqCst), 1);
    let seen = decisions.seen.lock().unwrap().clone();
    assert_eq!(
        seen[0].questions["question"].instructions,
        "Does this note state the patient has multiple sclerosis?"
    );
    assert_eq!(
        seen[0].context,
        DecisionContext::Text("SYNTHETIC NOTE: Neuro f/u. 34F, RRMS dx 2019.".into()),
        "the state is the text below the question line"
    );
    assert_eq!(
        assistant_texts(&messages),
        vec!["Yes · P(yes) 0.93 · tev1:0.8b".to_string()],
        "one assistant message, readable everywhere; no LLM turn ran"
    );
    let reply = messages
        .iter()
        .find(|m| m.role == rmcp::model::Role::Assistant)
        .unwrap();
    let id = reply
        .metadata
        .decision_id
        .clone()
        .expect("the reply names its decision");
    let detail = biorouter::decision::log::get(&chat.sessions, &chat.session_id, &id)
        .await
        .unwrap()
        .expect("the named decision is in the log");
    assert_eq!(detail.record.origin, DecisionOrigin::Chat);
    assert!(
        detail.record.turn_id.is_some(),
        "logged under the reply's turn"
    );
    assert_eq!(detail.record.decisions.len(), 1);

    // Stored, with its id: a reload shows the same card.
    let stored = chat
        .sessions
        .get_session(&chat.session_id, true)
        .await
        .unwrap()
        .conversation
        .unwrap();
    let stored_reply = stored
        .messages()
        .iter()
        .find(|m| m.role == rmcp::model::Role::Assistant)
        .unwrap();
    assert_eq!(
        stored_reply.metadata.decision_id.as_deref(),
        Some(id.as_str())
    );
    // A decision chat's own decision is its card, so the turn has no marker.
    assert!(stored
        .messages()
        .iter()
        .all(|m| m.metadata.decision_summary.is_none()));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_chats_question_is_asked_of_the_whole_message() {
    let decisions = Decisions::new(ProviderTier::Private, vec![0.12]);
    let chat = decision_chat(&decisions, ollama_tev1()).await;
    biorouter::decision::settings::save_chat(
        &chat.sessions,
        &chat.session_id,
        DecisionChatSettings {
            question: Question {
                instructions: "Is an EDSS score stated in this note?".into(),
                kind: DecisionKind::Binary {},
                threshold: None,
            },
        },
    )
    .await
    .unwrap();

    let messages = send(&chat, "SYNTHETIC NOTE: gait unsteady, no score given.").await;

    let seen = decisions.seen.lock().unwrap().clone();
    assert_eq!(
        seen[0].questions["question"].instructions,
        "Is an EDSS score stated in this note?"
    );
    assert_eq!(
        seen[0].context,
        DecisionContext::Text("SYNTHETIC NOTE: gait unsteady, no score given.".into())
    );
    assert_eq!(
        assistant_texts(&messages),
        vec!["No · P(yes) 0.12 · tev1:0.8b".to_string()]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_without_a_question_is_answered_with_how_to_ask() {
    let decisions = Decisions::new(ProviderTier::Private, vec![]);
    let chat = decision_chat(&decisions, ollama_tev1()).await;

    let messages = send(&chat, "SYNTHETIC NOTE: just a note.").await;

    assert_eq!(decisions.requests.load(Ordering::SeqCst), 0);
    let texts = assistant_texts(&messages);
    assert_eq!(texts.len(), 1);
    assert!(
        texts[0].contains("start your message with a question line"),
        "{texts:?}"
    );
    assert!(messages.iter().all(|m| m.metadata.decision_id.is_none()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chat_that_became_private_never_reaches_a_public_decision_model() {
    let decisions = Decisions::new(ProviderTier::Public, vec![0.99]);
    let public: Arc<dyn Provider> = Arc::new(BasetenDecisionChatProvider::with_model(
        ModelConfig::new_or_fail("joey-1"),
    ));
    let chat = decision_chat(&decisions, public.clone()).await;
    chat.sessions
        .update(&chat.session_id)
        .raise_privacy(SessionClassification::Private, "fixture")
        .apply()
        .await
        .unwrap();

    let messages = send(
        &chat,
        "Q: Does this note state MS?\nSYNTHETIC PRIVATE NOTE: RRMS dx 2019.",
    )
    .await;

    assert_eq!(
        decisions.requests.load(Ordering::SeqCst),
        0,
        "nothing reached the public model"
    );
    assert!(messages.iter().all(|m| m.metadata.decision_id.is_none()));
    // Gate A refuses binding it to the private chat in the first place.
    assert!(chat
        .agent
        .update_provider(public, &chat.session_id)
        .await
        .is_err());
}
