//! Storing a chat's decision settings after checking that what they turn on
//! can run.
//!
//! [`settings::save`] and [`settings::save_chat`] only validate the record's
//! shape: they are **unchecked** writes, and store a monitor whose model is
//! not there, cannot take its question or may not read the chat. A door a
//! person saves through (the settings routes, the CLI, a workflow's seed)
//! calls [`save_checked`] or [`save_chat_checked`] instead, which ask the
//! decision service first ([`DecisionService::check_settings`],
//! [`DecisionService::check_chat_question`]) and send nothing of the chat.
//!
//! Two error types say which kind of refusal it was, by
//! `downcast_ref` on the returned error:
//!
//! - [`InvalidDecisionSettings`]: the record itself is malformed or out of
//!   bounds (a route answers 400);
//! - [`UnusableDecisionSettings`]: it is valid, but what it turns on cannot
//!   run for this chat now (a route answers 422).

use super::service::{DecisionService, DecisionSession};
use super::settings::{self, DecisionChatSettings, DecisionSettings, InvalidDecisionSettings};
use super::types::DecisionModelRef;
use crate::session::SessionManager;

/// Settings that are valid but turn on something that cannot run for this
/// chat now: a monitor whose model is not there, cannot answer its question,
/// will not load, or may not read the chat. The message is the sentence a
/// person reads, starting "Nothing was saved".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusableDecisionSettings(pub String);

impl std::fmt::Display for UnusableDecisionSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UnusableDecisionSettings {}

/// Validate `settings`, check that what they turn on could run for
/// `session` ([`DecisionService::check_settings`], against the record stored
/// now, where an unreadable one counts as none), then store them. Returns the
/// stored value.
pub async fn save_checked(
    service: &DecisionService,
    manager: &SessionManager,
    session: &DecisionSession,
    settings: DecisionSettings,
) -> anyhow::Result<DecisionSettings> {
    settings::validate(&settings).map_err(InvalidDecisionSettings)?;
    let stored = settings::load(manager, &session.session_id).await.ok();
    service
        .check_settings(session, &settings, stored.as_ref())
        .await
        .map_err(UnusableDecisionSettings)?;
    settings::save(manager, &session.session_id, settings).await
}

/// [`save_checked`] for a decision chat's question: validated, checked
/// against the decision model the chat is bound to now (read off its
/// session row) when its question is new or of another kind, then stored.
pub async fn save_chat_checked(
    service: &DecisionService,
    manager: &SessionManager,
    session: &DecisionSession,
    chat: DecisionChatSettings,
) -> anyhow::Result<DecisionChatSettings> {
    settings::validate_chat(&chat).map_err(InvalidDecisionSettings)?;
    let stored = settings::load_chat(manager, &session.session_id)
        .await
        .ok()
        .flatten();
    let row = manager.get_session(&session.session_id, false).await?;
    let model = bound_decision_model(row.provider_name.as_deref(), row.model_config.as_ref());
    service
        .check_chat_question(
            session,
            model.as_ref(),
            &chat.question,
            stored.as_ref().map(|stored| &stored.question),
        )
        .await
        .map_err(UnusableDecisionSettings)?;
    settings::save_chat(manager, &session.session_id, chat).await
}

/// The decision model a chat is bound to, when its provider is a decision
/// provider.
fn bound_decision_model(
    provider: Option<&str>,
    model: Option<&crate::model::ModelConfig>,
) -> Option<DecisionModelRef> {
    let provider = provider
        .filter(|provider| crate::providers::decision::chat::is_decision_chat_provider(provider))?;
    let model = model?.model_name.trim();
    (!model.is_empty()).then(|| DecisionModelRef::new(provider, model))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::error::{DecisionError, DecisionResult};
    use crate::decision::model::{DecisionCaps, DecisionModel, DecisionModelEntry};
    use crate::decision::service::DecisionUseGate;
    use crate::decision::types::{
        DecisionAdapter, DecisionKindTag, DecisionRequest, DecisionUsage, ModelVerdicts,
    };
    use crate::decision::DecisionModelSource;
    use crate::privacy::{ProviderTier, SessionClassification};
    use crate::session::session_manager::SessionType;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::TempDir;

    struct Cloud(Arc<AtomicUsize>);

    #[async_trait]
    impl DecisionModel for Cloud {
        fn model_ref(&self) -> DecisionModelRef {
            DecisionModelRef::new("test_decision", "cloud").with_adapter(DecisionAdapter::SystemOne)
        }
        fn display_name(&self) -> String {
            "cloud".into()
        }
        fn tier(&self) -> ProviderTier {
            ProviderTier::Public
        }
        fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
            None
        }
        fn caps(&self) -> DecisionCaps {
            DecisionCaps {
                kinds: vec![DecisionKindTag::Binary],
                max_questions: 64,
                context_tokens: 4_096,
                max_concurrency: 1,
                max_request_bytes: None,
                local: false,
            }
        }
        async fn decide(&self, _: &DecisionRequest) -> DecisionResult<ModelVerdicts> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ModelVerdicts {
                verdicts: Vec::new(),
                usage: DecisionUsage::default(),
                served_model: None,
            })
        }
    }

    struct OnlyCloud(Arc<AtomicUsize>);

    #[async_trait]
    impl DecisionModelSource for OnlyCloud {
        async fn list(&self) -> Vec<DecisionModelEntry> {
            vec![DecisionModelEntry {
                provider: "test_decision".into(),
                model: "cloud".into(),
                display_name: "cloud".into(),
                context_tokens: 4_096,
                kinds: vec![DecisionKindTag::Binary],
                tier: ProviderTier::Public,
                affiliation: None,
                adapter: DecisionAdapter::SystemOne,
                local: false,
                load_refusal: None,
            }]
        }
        async fn open(&self, wanted: &DecisionModelRef) -> DecisionResult<Arc<dyn DecisionModel>> {
            if wanted.model == "cloud" {
                Ok(Arc::new(Cloud(self.0.clone())))
            } else {
                Err(DecisionError::NoModel(format!(
                    "No decision model `{}` is listed.",
                    wanted.label()
                )))
            }
        }
    }

    struct Allow;

    #[async_trait]
    impl DecisionUseGate for Allow {
        async fn check(&self, _: &DecisionSession, _: &dyn DecisionModel) -> DecisionResult<()> {
            Ok(())
        }
    }

    fn settings(monitor_model: &str) -> DecisionSettings {
        serde_json::from_value(serde_json::json!({
            "defaultModel": null,
            "monitors": [{
                "id": "done",
                "name": "Done",
                "question": {"instructions": "Is the task done?", "kind": {"binary": {}}},
                "model": {"provider": "test_decision", "model": monitor_model},
                "threshold": 0.8,
                "option": null,
                "rearmBelow": null
            }]
        }))
        .unwrap()
    }

    /// A checked save stores what can run, refuses what cannot with
    /// [`UnusableDecisionSettings`] and stores nothing, and refuses a malformed
    /// record with [`InvalidDecisionSettings`]. Nothing is sent either way.
    #[tokio::test]
    async fn a_checked_save_stores_only_what_can_run() {
        let dir = TempDir::new().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf());
        let id = manager
            .create_session(dir.path().into(), "chat".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let calls = Arc::new(AtomicUsize::new(0));
        let service = DecisionService::new(Arc::new(OnlyCloud(calls.clone())))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None);
        let public = DecisionSession::new(&id, SessionClassification::Public, true);

        let stored = save_checked(&service, &manager, &public, settings("cloud"))
            .await
            .unwrap();
        assert_eq!(settings::load(&manager, &id).await.unwrap(), stored);

        let error = save_checked(&service, &manager, &public, settings("gone"))
            .await
            .unwrap_err();
        let unusable = error
            .downcast_ref::<UnusableDecisionSettings>()
            .expect("an unusable save is typed");
        assert!(
            unusable
                .0
                .starts_with("Nothing was saved: monitor \"Done\" could not run."),
            "{unusable}"
        );
        assert_eq!(
            settings::load(&manager, &id).await.unwrap(),
            stored,
            "nothing was saved"
        );

        let mut malformed = settings("cloud");
        malformed.monitors[0].threshold = 7.0;
        let error = save_checked(&service, &manager, &public, malformed)
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<InvalidDecisionSettings>().is_some());

        // Saving the stored monitor again asks nothing, even where it could
        // not run now; a new chat where it is new is asked.
        let private = DecisionSession::new(&id, SessionClassification::Private, true);
        save_checked(&service, &manager, &private, stored.clone())
            .await
            .unwrap();
        let other = manager
            .create_session(dir.path().into(), "private chat".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let private = DecisionSession::new(&other, SessionClassification::Private, true);
        let error = save_checked(&service, &manager, &private, settings("cloud"))
            .await
            .unwrap_err();
        let unusable = error
            .downcast_ref::<UnusableDecisionSettings>()
            .expect("an unusable save is typed");
        assert!(unusable.0.contains("This chat is private"), "{unusable}");
        assert_eq!(
            settings::load(&manager, &other).await.unwrap(),
            DecisionSettings::default()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a check sends nothing");
    }

    /// A decision chat's question is checked against the model its session
    /// row binds, and a chat bound to no decision model is not asked.
    #[tokio::test]
    async fn a_decision_chat_question_is_checked_against_the_bound_model() {
        let dir = TempDir::new().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf());
        let calls = Arc::new(AtomicUsize::new(0));
        let service = DecisionService::new(Arc::new(OnlyCloud(calls.clone())))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None);
        let question: DecisionChatSettings = serde_json::from_value(serde_json::json!({
            "question": {"instructions": "Does the note say MS?", "kind": {"binary": {}}}
        }))
        .unwrap();

        let bound = manager
            .create_session(dir.path().into(), "bound".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        manager
            .update(&bound)
            .provider_name("ollama_decision")
            .model_config(crate::model::ModelConfig::new_or_fail("tev1:0.8b"))
            .apply()
            .await
            .unwrap();
        let session = DecisionSession::new(&bound, SessionClassification::Public, true);
        let error = save_chat_checked(&service, &manager, &session, question.clone())
            .await
            .unwrap_err();
        let unusable = error
            .downcast_ref::<UnusableDecisionSettings>()
            .expect("an unusable save is typed");
        assert!(
            unusable.0.starts_with(
                "Nothing was saved: this decision chat's model ollama_decision/tev1:0.8b could \
                 not answer this question."
            ),
            "{unusable}"
        );
        assert_eq!(settings::load_chat(&manager, &bound).await.unwrap(), None);

        let unbound = manager
            .create_session(dir.path().into(), "unbound".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let session = DecisionSession::new(&unbound, SessionClassification::Public, true);
        save_chat_checked(&service, &manager, &session, question.clone())
            .await
            .unwrap();
        assert_eq!(
            settings::load_chat(&manager, &unbound).await.unwrap(),
            Some(question)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn only_a_decision_provider_binds_a_decision_chat() {
        let config = crate::model::ModelConfig::new_or_fail("tev1:0.8b");
        assert_eq!(
            bound_decision_model(Some("ollama_decision"), Some(&config)),
            Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b"))
        );
        assert_eq!(bound_decision_model(Some("openai"), Some(&config)), None);
        assert_eq!(bound_decision_model(None, Some(&config)), None);
        assert_eq!(bound_decision_model(Some("ollama_decision"), None), None);
    }
}
