//! Tests for the session auto-naming fix: after a real exchange a session must
//! never be left as the "New chat" placeholder. The LLM namer is
//! best-effort; on error or empty output a deterministic fallback derived from
//! the first user message is used.

use std::sync::Arc;

use async_trait::async_trait;
use biorouter::conversation::message::Message;
use biorouter::conversation::Conversation;
use biorouter::model::ModelConfig;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::session_manager::SessionType;
use biorouter::session::SessionManager;
use rmcp::model::Tool;
use tempfile::TempDir;

/// Controls what the mock namer returns.
enum NamerBehavior {
    Fail,
    Empty,
    Returns(&'static str),
}

struct NamerProvider {
    behavior: NamerBehavior,
}

#[async_trait]
impl Provider for NamerProvider {
    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        // Not exercised: we override generate_session_name directly.
        Err(ProviderError::ExecutionError("unused".to_string()))
    }

    async fn generate_session_name(
        &self,
        _messages: &Conversation,
    ) -> Result<String, ProviderError> {
        match self.behavior {
            NamerBehavior::Fail => Err(ProviderError::ExecutionError("namer down".to_string())),
            NamerBehavior::Empty => Ok("   ".to_string()),
            NamerBehavior::Returns(s) => Ok(s.to_string()),
        }
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new("mock-model").unwrap()
    }

    fn metadata() -> ProviderMetadata {
        ProviderMetadata {
            name: "mock".to_string(),
            display_name: "Mock".to_string(),
            description: "mock".to_string(),
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
        "mock"
    }
}

async fn session_with_first_user_message(text: &str) -> (Arc<SessionManager>, String) {
    let data_dir = TempDir::new().unwrap();
    let manager = Arc::new(SessionManager::new(data_dir.path().to_path_buf()));
    std::mem::forget(data_dir); // keep the sqlite dir alive for the test
    let session = manager
        .create_session(
            std::path::PathBuf::from("/tmp"),
            biorouter::session::DEFAULT_SESSION_NAME.to_string(),
            SessionType::User,
        )
        .await
        .unwrap();
    manager
        .add_message(&session.id, &Message::user().with_text(text))
        .await
        .unwrap();
    manager
        .add_message(&session.id, &Message::assistant().with_text("ok"))
        .await
        .unwrap();
    (manager, session.id)
}

async fn name_of(manager: &SessionManager, id: &str) -> String {
    manager.get_session(id, false).await.unwrap().name
}

#[tokio::test]
async fn llm_namer_failure_falls_back_to_first_message() {
    let (manager, id) = session_with_first_user_message(
        "Explain the difference between DNA and RNA in two sentences.",
    )
    .await;
    assert_eq!(
        name_of(&manager, &id).await,
        biorouter::session::DEFAULT_SESSION_NAME
    );

    manager
        .maybe_update_name(
            &id,
            Arc::new(NamerProvider {
                behavior: NamerBehavior::Fail,
            }),
        )
        .await
        .unwrap();

    let name = name_of(&manager, &id).await;
    assert_ne!(
        name,
        biorouter::session::DEFAULT_SESSION_NAME,
        "session must not stay as the placeholder"
    );
    assert_eq!(name, "Explain the difference between DNA and RNA in");
}

#[tokio::test]
async fn empty_llm_name_falls_back() {
    let (manager, id) = session_with_first_user_message("Summarize this paper for me please").await;

    manager
        .maybe_update_name(
            &id,
            Arc::new(NamerProvider {
                behavior: NamerBehavior::Empty,
            }),
        )
        .await
        .unwrap();

    let name = name_of(&manager, &id).await;
    assert_ne!(name, biorouter::session::DEFAULT_SESSION_NAME);
    assert_eq!(name, "Summarize this paper for me please");
}

#[tokio::test]
async fn good_llm_name_is_used() {
    let (manager, id) = session_with_first_user_message("hello there").await;

    manager
        .maybe_update_name(
            &id,
            Arc::new(NamerProvider {
                behavior: NamerBehavior::Returns("DNA vs RNA"),
            }),
        )
        .await
        .unwrap();

    assert_eq!(name_of(&manager, &id).await, "DNA vs RNA");
}

/// Name the session once, then let a second exchange happen, so the session is
/// a young chat (inside the first few user messages) that already wears a
/// generated name.
async fn named_session_with_a_second_exchange(
    first: &str,
    generated: &'static str,
    second: &str,
) -> (Arc<SessionManager>, String) {
    let (manager, id) = session_with_first_user_message(first).await;
    manager
        .maybe_update_name(
            &id,
            Arc::new(NamerProvider {
                behavior: NamerBehavior::Returns(generated),
            }),
        )
        .await
        .unwrap();
    assert_eq!(name_of(&manager, &id).await, generated);
    manager
        .add_message(&id, &Message::user().with_text(second))
        .await
        .unwrap();
    manager
        .add_message(&id, &Message::assistant().with_text("ok"))
        .await
        .unwrap();
    (manager, id)
}

#[tokio::test]
async fn a_failing_namer_keeps_the_name_a_young_chat_already_has() {
    // The 2026-10-11 vision run: an expired key made the namer fail on the
    // chat's third message, and the first-message snippet overwrote the
    // generated title.
    let (manager, id) = named_session_with_a_second_exchange(
        "Explain in two sentences why mitochondria are called the powerhouse",
        "Mitochondria powerhouse explanation",
        "Reply with the single word ok.",
    )
    .await;
    let before = manager.get_session(&id, false).await.unwrap();

    manager
        .maybe_update_name(
            &id,
            Arc::new(NamerProvider {
                behavior: NamerBehavior::Fail,
            }),
        )
        .await
        .unwrap();

    let after = manager.get_session(&id, false).await.unwrap();
    assert_eq!(after.name, "Mitochondria powerhouse explanation");
    assert!(!after.user_set_name);
    assert_eq!(
        after.updated_at, before.updated_at,
        "a naming attempt that changes nothing must not move the chat"
    );
}

#[tokio::test]
async fn an_empty_namer_answer_keeps_the_name_a_young_chat_already_has() {
    let (manager, id) =
        named_session_with_a_second_exchange("hello", "Initial greeting", "how are you").await;

    manager
        .maybe_update_name(
            &id,
            Arc::new(NamerProvider {
                behavior: NamerBehavior::Empty,
            }),
        )
        .await
        .unwrap();

    assert_eq!(name_of(&manager, &id).await, "Initial greeting");
}

#[tokio::test]
async fn a_succeeding_namer_still_refines_a_young_chats_name() {
    // The converse: the guard is on the fallback, not on regeneration. Inside
    // the first few messages a good generated name may still improve.
    let (manager, id) =
        named_session_with_a_second_exchange("hello", "Initial greeting", "how are you").await;

    manager
        .maybe_update_name(
            &id,
            Arc::new(NamerProvider {
                behavior: NamerBehavior::Returns("Greeting and check-in"),
            }),
        )
        .await
        .unwrap();

    assert_eq!(name_of(&manager, &id).await, "Greeting and check-in");
}

#[tokio::test]
async fn a_person_chosen_name_survives_every_namer_outcome() {
    let (manager, id) = session_with_first_user_message("hello there").await;
    manager
        .update(&id)
        .user_provided_name("My own title")
        .apply()
        .await
        .unwrap();

    for behavior in [
        NamerBehavior::Fail,
        NamerBehavior::Empty,
        NamerBehavior::Returns("Something else"),
    ] {
        manager
            .maybe_update_name(&id, Arc::new(NamerProvider { behavior }))
            .await
            .unwrap();
        assert_eq!(name_of(&manager, &id).await, "My own title");
    }
}

#[tokio::test]
async fn a_chat_still_on_the_placeholder_is_named_even_after_it_grows() {
    // The reason the fallback exists at all: a chat whose early naming missed
    // must not stay "New chat" once it crosses the message-count threshold.
    let (manager, id) = session_with_first_user_message("Summarize this paper for me").await;
    for text in ["one", "two", "three"] {
        manager
            .add_message(&id, &Message::user().with_text(text))
            .await
            .unwrap();
        manager
            .add_message(&id, &Message::assistant().with_text("ok"))
            .await
            .unwrap();
    }

    manager
        .maybe_update_name(
            &id,
            Arc::new(NamerProvider {
                behavior: NamerBehavior::Fail,
            }),
        )
        .await
        .unwrap();

    assert_eq!(name_of(&manager, &id).await, "Summarize this paper for me");
}
