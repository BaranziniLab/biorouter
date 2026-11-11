//! Shared helpers for the chat extractor's `Agent::reply` tests (PR-24, DD-57): a
//! synthetic provider that counts its requests, an agent with the Dictionary capability
//! loaded, chats in a scratch session store, and the waits that make an absent
//! suggestion mean something. No real provider and no owner data.
//!
//! The extractor's settings watcher, its worker queue and the dictionary store are
//! process wide. So a binary that uses these helpers gets a scratch root before `main`
//! (`isolate_paths`) and runs its tests one at a time ([`SERIAL`]).
//!
//! ⚠ Every write to the store gives it a new version, the extractor's own writes
//! included, and an observation admitted at an older version is dropped by design. So
//! each turn starts with [`ready`], which waits until the watcher has read the store as
//! it is now and no observation is in flight, and a test that asserts nothing landed
//! first waits with [`settled`] until the worker queue is empty. Without the first, a
//! working extractor drops a sighting and a count test fails; without the second, a
//! broken one has not written yet and an exclusion test passes.

#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use biorouter::agents::{Agent, AgentConfig, ExtensionConfig, SessionConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::conversation::message::Message;
use biorouter::dictation::extractor::{admission_stamp, observations_in_flight};
use biorouter::model::ModelConfig;
use biorouter::privacy::{ModelAffiliation, ProviderTier};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::security::dictionary_store::{open_store, store};
use biorouter::session::{Session, SessionManager, SessionType};
use futures::StreamExt;
use rmcp::model::Tool;

/// One test at a time; see the module doc.
pub static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How long a wait may take before the test fails rather than hangs.
const WAIT: Duration = Duration::from_secs(10);

/// Points `BIOROUTER_PATH_ROOT` at a scratch folder for the whole process, before `main`,
/// so nothing resolves the person's own configuration or dictionary. Always a fresh
/// folder, even under an outer root, because these tests write the dictionary.
#[ctor::ctor]
fn isolate_paths() {
    let root = tempfile::TempDir::new().expect("a scratch root");
    std::env::set_var("BIOROUTER_PATH_ROOT", root.path());
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let _ = ROOT.set(root);
}

/// A provider that answers "done" and counts its requests. A turn that made one got past
/// the point where `reply_internal` queues the extractor's observation, so a test that
/// finds nothing suggested also shows the turn reached the place where it could have been.
pub struct CountingProvider {
    tier: ProviderTier,
    calls: AtomicUsize,
}

impl CountingProvider {
    pub fn public() -> Arc<Self> {
        Arc::new(Self {
            tier: ProviderTier::Public,
            calls: AtomicUsize::new(0),
        })
    }

    /// A model on this computer: Private, stating `Local` as every private double must
    /// (DR-26), so it adds no institution to a row.
    pub fn local() -> Arc<Self> {
        Arc::new(Self {
            tier: ProviderTier::Private,
            calls: AtomicUsize::new(0),
        })
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for CountingProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "pr24-synthetic"
    }

    fn tier(&self) -> ProviderTier {
        self.tier
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        match self.tier {
            ProviderTier::Private => Some(ModelAffiliation::Local),
            ProviderTier::Public => None,
        }
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok((
            Message::assistant().with_text("done"),
            ProviderUsage::new("pr24-model".to_string(), Usage::default()),
        ))
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("pr24-model")
    }
}

/// A scratch session store and the agents that use it.
pub struct Fixture {
    root: tempfile::TempDir,
    pub sessions: Arc<SessionManager>,
}

impl Fixture {
    pub fn scratch() -> Self {
        let root = tempfile::TempDir::new().expect("a scratch folder");
        let sessions = Arc::new(SessionManager::new(root.path().join("sessions")));
        Self { root, sessions }
    }

    pub async fn chat(&self, kind: SessionType) -> Session {
        self.sessions
            .create_session(self.root.path().to_path_buf(), format!("{kind} chat"), kind)
            .await
            .unwrap()
    }

    /// An agent with the Dictionary capability loaded, as new chats have it once the
    /// person ticked **Suggest dictionary words from my chats**. `dictionary` is a
    /// platform extension: a `Builtin` config naming it is refused as an unknown builtin.
    pub async fn agent(&self) -> Agent {
        let agent = Agent::with_config(AgentConfig::new(
            Arc::clone(&self.sessions),
            Arc::new(PermissionManager::new(self.root.path().to_path_buf())),
            None,
            BioRouterMode::Auto,
        ));
        agent
            .add_extension(ExtensionConfig::Platform {
                name: "dictionary".into(),
                description: "Dictionary".into(),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .unwrap();
        agent
    }
}

/// Turns the store's `learn_from_chats` on or off, as the Dictionary page does.
pub fn learn_from_chats(on: bool) {
    open_store()
        .write(|file| {
            file.settings.learn_from_chats = on;
            Ok(())
        })
        .unwrap();
}

/// Waits until `done` holds, and fails the test after [`WAIT`].
pub async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The watcher has read the store as it is now, and no observation is in flight.
pub async fn ready() {
    wait_for(
        "the settings watcher to read the store as it is now",
        || {
            let now = open_store().view().unwrap().content_stamp();
            observations_in_flight() == 0 && admission_stamp() == Some(now)
        },
    )
    .await;
}

/// No observation is queued or running.
pub async fn settled() {
    wait_for("the extractor's worker queue to empty", || {
        observations_in_flight() == 0
    })
    .await;
}

/// The store's rows written exactly `written`, in either tier.
pub fn rows_named(written: &str) -> Vec<store::Entry> {
    open_store()
        .view()
        .unwrap()
        .entries()
        .iter()
        .filter(|row| row.written == written)
        .cloned()
        .collect()
}

/// One turn through `Agent::reply`, read to its end. Answers how many requests the turn
/// made of `provider`.
pub async fn reply(
    agent: &Agent,
    chat: &Session,
    message: Message,
    provider: &CountingProvider,
) -> usize {
    let before = provider.calls();
    let config = SessionConfig {
        id: chat.id.clone(),
        schedule_id: None,
        max_turns: Some(1),
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };
    let mut stream = agent.reply(message, config, None).await.unwrap();
    while let Some(event) = stream.next().await {
        event.unwrap();
    }
    provider.calls() - before
}

/// [`ready`], then a person's message of `text` through [`reply`], which must reach the
/// model.
pub async fn say(agent: &Agent, chat: &Session, text: &str, provider: &CountingProvider) {
    ready().await;
    let requests = reply(agent, chat, Message::user().with_text(text), provider).await;
    assert!(
        requests > 0,
        "the turn with {text:?} never reached the model, so it says nothing about the extractor"
    );
}
