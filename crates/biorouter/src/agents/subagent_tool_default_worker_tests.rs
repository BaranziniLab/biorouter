//! The default worker model at the spawn (owner, 2026-11-11): named beats the
//! default worker, the default worker beats the chat's model, and a default
//! the privacy rules refuse (or that cannot start) falls back to the chat's
//! model with one note to the parent, never a failed spawn.
//!
//! Every row runs the real resolver, `resolve_child_privacy`, with the setting
//! given through `with_config_overrides`, and builds real `ollama` instances:
//! the one registry entry that constructs with no credential and no network
//! and reads its tier off `OLLAMA_HOST` (loopback Private, off-box Public).

use super::*;
use crate::agents::default_worker::{WORKER_MODEL_KEY, WORKER_PROVIDER_KEY};
use crate::privacy::{ModelAffiliation, ProviderTier};
use crate::providers::base::Provider;
use std::sync::Arc;

const LOOPBACK: &str = "http://localhost:11434";
const OFF_BOX: &str = "https://ollama.example.com";

/// A chat's model, stating both privacy axes.
struct ChatModel {
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
}

#[async_trait::async_trait]
impl Provider for ChatModel {
    fn metadata() -> crate::providers::base::ProviderMetadata {
        crate::providers::base::ProviderMetadata::new(
            "chat-model-mock",
            "Chat model mock",
            "",
            "chat-model",
            vec![],
            "",
            vec![],
        )
    }
    fn get_name(&self) -> &str {
        "chat-model-mock"
    }
    fn tier(&self) -> ProviderTier {
        self.tier
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.affiliation
    }
    fn get_model_config(&self) -> crate::model::ModelConfig {
        crate::model::ModelConfig::new_or_fail("chat-model")
    }
    async fn complete_with_model(
        &self,
        _model_config: &crate::model::ModelConfig,
        _system: &str,
        _messages: &[crate::conversation::message::Message],
        _tools: &[Tool],
    ) -> std::result::Result<
        (
            crate::conversation::message::Message,
            crate::providers::base::ProviderUsage,
        ),
        crate::providers::errors::ProviderError,
    > {
        Err(crate::providers::errors::ProviderError::Authentication(
            "the spawn rows never run a turn".to_string(),
        ))
    }
}

/// A public chat (a hosted model with no institution).
fn public_chat() -> Arc<dyn Provider> {
    Arc::new(ChatModel {
        tier: ProviderTier::Public,
        affiliation: None,
    })
}

/// A private chat on this machine, the affiliation a loopback Ollama states.
fn local_private_chat() -> Arc<dyn Provider> {
    Arc::new(ChatModel {
        tier: ProviderTier::Private,
        affiliation: Some(ModelAffiliation::Local),
    })
}

fn params(body: Value) -> SubagentParams {
    serde_json::from_value(body).unwrap()
}

fn unnamed() -> SubagentParams {
    params(json!({ "instructions": "do the thing" }))
}

fn overrides(worker: Option<(&str, &str)>, host: &str) -> HashMap<String, String> {
    let (provider, model) = worker.unwrap_or(("", ""));
    HashMap::from([
        (WORKER_PROVIDER_KEY.to_string(), provider.to_string()),
        (WORKER_MODEL_KEY.to_string(), model.to_string()),
        ("OLLAMA_HOST".to_string(), host.to_string()),
    ])
}

async fn resolve(
    parent: Arc<dyn Provider>,
    params: &SubagentParams,
    worker: Option<(&str, &str)>,
    host: &str,
) -> Result<(TaskConfig, ChildPrivacy)> {
    let task_config = TaskConfig::new(parent, "parent-1", std::path::Path::new("."), vec![]);
    crate::config::with_config_overrides(
        overrides(worker, host),
        resolve_child_privacy(task_config, params),
    )
    .await
}

#[tokio::test]
async fn an_unnamed_spawn_runs_on_the_default_worker_model() {
    let (child, privacy) = resolve(
        public_chat(),
        &unnamed(),
        Some(("ollama", "worker-model")),
        OFF_BOX,
    )
    .await
    .expect("a public worker under a public chat runs");
    assert_eq!(child.provider.get_name(), "ollama");
    assert_eq!(child.provider.get_model_config().model_name, "worker-model");
    assert_eq!(privacy.default_worker_note, None, "nothing to explain");
}

#[tokio::test]
async fn with_no_default_worker_the_child_keeps_the_chat_model_instance() {
    let parent = public_chat();
    let task_config = TaskConfig::new(
        Arc::clone(&parent),
        "parent-1",
        std::path::Path::new("."),
        vec![],
    );
    let (child, privacy) = crate::config::with_config_overrides(
        overrides(None, OFF_BOX),
        resolve_child_privacy(task_config, &unnamed()),
    )
    .await
    .unwrap();
    assert!(
        Arc::ptr_eq(&child.provider, &parent),
        "the chat's own instance, exactly as before the default existed"
    );
    assert_eq!(privacy.default_worker_note, None);
}

#[tokio::test]
async fn a_default_worker_that_is_the_chat_model_keeps_the_chat_instance() {
    let parent = public_chat();
    let task_config = TaskConfig::new(
        Arc::clone(&parent),
        "parent-1",
        std::path::Path::new("."),
        vec![],
    );
    let (child, privacy) = crate::config::with_config_overrides(
        overrides(Some(("chat-model-mock", "chat-model")), OFF_BOX),
        resolve_child_privacy(task_config, &unnamed()),
    )
    .await
    .unwrap();
    assert!(Arc::ptr_eq(&child.provider, &parent));
    assert_eq!(privacy.default_worker_note, None);
}

#[tokio::test]
async fn a_named_provider_and_model_win_over_the_default_worker() {
    let named = params(json!({
        "instructions": "do the thing",
        "settings": { "provider": "ollama", "model": "named-model" }
    }));
    let (child, privacy) = resolve(
        public_chat(),
        &named,
        Some(("ollama", "worker-model")),
        OFF_BOX,
    )
    .await
    .unwrap();
    assert_eq!(child.provider.get_name(), "ollama");
    assert_eq!(child.provider.get_model_config().model_name, "named-model");
    assert_eq!(privacy.default_worker_note, None);
}

/// A spawn that names a provider and no model is a named spawn: it runs on
/// that provider's own default model, never the default worker's model, even
/// when the default worker is on the same provider.
#[tokio::test]
async fn a_named_provider_without_a_model_gets_that_providers_default_model() {
    let provider_only = params(json!({
        "instructions": "do the thing",
        "settings": { "provider": "ollama" }
    }));
    let (child, privacy) = resolve(
        public_chat(),
        &provider_only,
        Some(("ollama", "worker-model")),
        OFF_BOX,
    )
    .await
    .unwrap();
    assert_eq!(child.provider.get_name(), "ollama");
    assert_eq!(
        child.provider.get_model_config().model_name,
        crate::providers::ollama::OLLAMA_DEFAULT_MODEL
    );
    assert_eq!(privacy.default_worker_note, None);
}

/// The early refusal of a provider that is not set up judges only a provider
/// the spawn NAMES. A default worker on a provider that is not usable here is
/// never refused: the subagent runs on the chat's model and the parent is told.
#[tokio::test]
async fn an_unusable_default_worker_is_not_refused_and_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = overrides(Some(("codex", "gpt-5.5")), OFF_BOX);
    config.insert(
        "CODEX_COMMAND".to_string(),
        dir.path()
            .join("no-such-codex")
            .to_string_lossy()
            .into_owned(),
    );
    let parent = public_chat();
    let task_config = TaskConfig::new(
        Arc::clone(&parent),
        "parent-1",
        std::path::Path::new("."),
        vec![],
    );
    let (refusal, resolved) = crate::config::with_config_overrides(config, async {
        (
            provider_setup_refusal(&unnamed(), parent.get_name()).await,
            resolve_child_privacy(task_config, &unnamed()).await,
        )
    })
    .await;
    assert_eq!(refusal, None, "the default worker is not a named provider");
    let (child, privacy) = resolved.expect("the default never fails a spawn");
    assert!(Arc::ptr_eq(&child.provider, &parent));
    let note = privacy.default_worker_note.expect("the parent is told");
    assert!(note.contains("gpt-5.5 (codex)"), "{note}");
    assert!(note.contains("could not be started"), "{note}");
}

#[tokio::test]
async fn a_temperature_alone_still_runs_on_the_default_worker() {
    let warm = params(json!({
        "instructions": "do the thing",
        "settings": { "temperature": 0.25 }
    }));
    let (child, _) = resolve(
        public_chat(),
        &warm,
        Some(("ollama", "worker-model")),
        OFF_BOX,
    )
    .await
    .unwrap();
    let model = child.provider.get_model_config();
    assert_eq!(model.model_name, "worker-model");
    assert_eq!(model.temperature, Some(0.25));
}

#[tokio::test]
async fn a_private_chat_with_a_private_worker_runs_on_the_worker() {
    let (child, privacy) = resolve(
        local_private_chat(),
        &unnamed(),
        Some(("ollama", "worker-model")),
        LOOPBACK,
    )
    .await
    .expect("a private worker under a private chat runs");
    assert_eq!(child.provider.get_name(), "ollama");
    assert_eq!(
        child.privacy_tier,
        crate::privacy::SessionClassification::Private
    );
    assert_eq!(privacy.default_worker_note, None);
}

/// DR-19 for the default: a private chat's subagent never moves to a public
/// worker. It runs on the chat's model and the parent is told why, once.
#[tokio::test]
async fn a_public_worker_under_a_private_chat_falls_back_to_the_chat_model() {
    let parent = local_private_chat();
    let task_config = TaskConfig::new(
        Arc::clone(&parent),
        "parent-1",
        std::path::Path::new("."),
        vec![],
    );
    let (child, privacy) = crate::config::with_config_overrides(
        overrides(Some(("ollama", "worker-model")), OFF_BOX),
        resolve_child_privacy(task_config, &unnamed()),
    )
    .await
    .expect("the default never fails a spawn");
    assert!(Arc::ptr_eq(&child.provider, &parent));
    assert_eq!(
        child.privacy_tier,
        crate::privacy::SessionClassification::Private
    );
    let note = privacy.default_worker_note.expect("the parent is told");
    assert!(note.contains("worker-model (ollama)"), "{note}");
    assert!(note.contains("this chat is private"), "{note}");
}

/// R4 for the default: a public chat's subagent never gains private reach
/// through a private worker.
#[tokio::test]
async fn a_private_worker_under_a_public_chat_falls_back_to_the_chat_model() {
    let parent = public_chat();
    let task_config = TaskConfig::new(
        Arc::clone(&parent),
        "parent-1",
        std::path::Path::new("."),
        vec![],
    );
    let (child, privacy) = crate::config::with_config_overrides(
        overrides(Some(("ollama", "worker-model")), LOOPBACK),
        resolve_child_privacy(task_config, &unnamed()),
    )
    .await
    .expect("the default never fails a spawn");
    assert!(Arc::ptr_eq(&child.provider, &parent));
    assert_eq!(
        child.privacy_tier,
        crate::privacy::SessionClassification::Public
    );
    let note = privacy.default_worker_note.expect("the parent is told");
    assert!(note.contains("this chat is public"), "{note}");
}

/// The same rule a named spawn meets: naming the private model outright is
/// still refused, so the fallback is only ever the default's.
#[tokio::test]
async fn naming_the_refused_model_outright_is_still_refused() {
    let named = params(json!({
        "instructions": "do the thing",
        "settings": { "provider": "ollama", "model": "worker-model" }
    }));
    let error = resolve(public_chat(), &named, None, LOOPBACK)
        .await
        .expect_err("R4 refuses a named private child under a public chat");
    assert!(
        error
            .downcast_ref::<crate::privacy::PrivacyRefusal>()
            .is_some(),
        "{error}"
    );
}

#[tokio::test]
async fn a_decision_model_as_the_default_worker_falls_back() {
    let (child, privacy) = resolve(
        public_chat(),
        &unnamed(),
        Some(("baseten_decision", "joey-1")),
        OFF_BOX,
    )
    .await
    .expect("the default never fails a spawn");
    assert_eq!(child.provider.get_name(), "chat-model-mock");
    let note = privacy.default_worker_note.expect("the parent is told");
    assert!(note.contains("decision model"), "{note}");
}

#[tokio::test]
async fn a_default_worker_that_cannot_start_falls_back() {
    let (child, privacy) = resolve(
        public_chat(),
        &unnamed(),
        Some(("no-such-provider", "some-model")),
        OFF_BOX,
    )
    .await
    .expect("the default never fails a spawn");
    assert_eq!(child.provider.get_name(), "chat-model-mock");
    let note = privacy.default_worker_note.expect("the parent is told");
    assert!(note.contains("could not be started"), "{note}");
}

#[test]
fn the_tool_description_says_which_model_an_unnamed_subagent_uses() {
    let description = build_tool_description(&[]);
    assert!(
        description.contains(
            "With no provider and no model named in `settings`, a subagent runs on the user's \
             default worker model when one is set, and otherwise on this chat's model."
        ),
        "{description}"
    );
}
