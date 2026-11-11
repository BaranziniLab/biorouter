//! The default worker model: the model a subagent runs on when the spawn names
//! none (owner, 2026-11-11).
//!
//! A chat runs on its own model. A person may also choose a default worker
//! model in Settings > Models, typically a lighter or more token-efficient one,
//! and subagents use it unless the agent names a provider or a model of its
//! own. Nothing is set by default, and then a subagent runs on the chat's model,
//! exactly as it did before this setting existed.
//!
//! The resolution order, decided once in [`child_model_for_spawn`]:
//!
//! 1. A provider or a model the spawn names.
//! 2. The default worker model, when one is set and it is not the chat's own.
//! 3. The chat's own model.
//!
//! The default never changes what a spawn may do. A spawn on the default worker
//! is judged by the same privacy rules as any other (R4, DR-19, DR-31 in
//! `subagent_tool::resolve_child_privacy`). When those rules, or anything else,
//! keep the default worker from running this chat's subagent, the subagent runs
//! on the chat's model instead and the parent is told once, in the spawn's
//! result ([`fallback_note`]). The default never fails a spawn.
//!
//! This replaces lead/worker mode, where a lead model answered a chat's first
//! turns and a worker took over. `config::retired_lead_worker` carries an old
//! setup across: its lead becomes the chat's model and its worker becomes the
//! default worker model.

use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::config::Config;
use crate::model::ModelConfig;
use crate::privacy::PrivacyRefusal;
use crate::providers::base::Provider;

/// The provider half of the default worker model.
pub const WORKER_PROVIDER_KEY: &str = "BIOROUTER_WORKER_PROVIDER";
/// The model half of the default worker model.
pub const WORKER_MODEL_KEY: &str = "BIOROUTER_WORKER_MODEL";
/// An optional context window for the default worker model, as
/// `BIOROUTER_CONTEXT_LIMIT` is for the chat's model. The key is older than this
/// module: lead/worker mode read it for its worker, which is the same role.
pub const WORKER_CONTEXT_LIMIT_KEY: &str = "BIOROUTER_WORKER_CONTEXT_LIMIT";

/// A configured default worker model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultWorker {
    pub provider: String,
    pub model: String,
}

impl DefaultWorker {
    /// `model (provider)`, the way the spawn note names it.
    pub fn label(&self) -> String {
        format!("{} ({})", self.model, self.provider)
    }
}

/// The default worker model in the global configuration, or `None` when it is
/// not set. Read per spawn, so a change in Settings applies to the next one.
pub fn configured_default_worker() -> Option<DefaultWorker> {
    configured_default_worker_in(Config::global())
}

/// [`configured_default_worker`] for a given configuration.
///
/// **Both keys or nothing.** A provider with no model, or a model with no
/// provider, is not a default: the desktop writes the model first and the
/// provider last, so a spawn that lands between its two writes reads "not set"
/// and runs on the chat's model rather than on a provider paired with another
/// provider's model.
pub fn configured_default_worker_in(config: &Config) -> Option<DefaultWorker> {
    let read = |key: &str| {
        config
            .get_param::<String>(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let provider = read(WORKER_PROVIDER_KEY)?;
    let model = read(WORKER_MODEL_KEY)?;
    Some(DefaultWorker { provider, model })
}

/// Which model a subagent runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildModel {
    /// The spawn named a provider or a model, which wins over the default.
    Named,
    /// The spawn named neither and a default worker model, other than the
    /// chat's own, is set.
    DefaultWorker(DefaultWorker),
    /// The chat's own model, as it was before the default existed.
    ChatModel,
}

/// The resolution order: named, then the default worker model, then the chat's
/// model. A default that IS the chat's model resolves to the chat's model, so
/// the child keeps the parent's live provider instance (and a composite parent
/// is forked as before) rather than a fresh copy of the same model.
pub fn child_model_for_spawn(
    named_provider: Option<&str>,
    named_model: Option<&str>,
    default_worker: Option<DefaultWorker>,
    chat_provider: &str,
    chat_model: &str,
) -> ChildModel {
    let named = |value: Option<&str>| value.is_some_and(|value| !value.trim().is_empty());
    if named(named_provider) || named(named_model) {
        return ChildModel::Named;
    }
    match default_worker {
        Some(worker) if worker.provider == chat_provider && worker.model == chat_model => {
            ChildModel::ChatModel
        }
        Some(worker) => ChildModel::DefaultWorker(worker),
        None => ChildModel::ChatModel,
    }
}

/// The model configuration the default worker runs with: its own model name,
/// with `BIOROUTER_WORKER_CONTEXT_LIMIT` (config file or environment) when set,
/// the reasoning effort the chat runs at, and a temperature the spawn named.
pub fn worker_model_config(
    worker: &DefaultWorker,
    chat_model: &ModelConfig,
    temperature: Option<f32>,
) -> Result<ModelConfig> {
    let mut model = ModelConfig::new(&worker.model)
        .map_err(|e| anyhow!("model '{}' is not usable: {e}", worker.model))?;
    if let Ok(limit) = Config::global().get_param::<usize>(WORKER_CONTEXT_LIMIT_KEY) {
        model = model.with_context_limit(Some(limit));
    }
    model.reasoning_effort = chat_model.reasoning_effort;
    if let Some(temperature) = temperature {
        model.temperature = Some(temperature);
    }
    Ok(model)
}

/// Construct the default worker's provider. A decision model is refused here,
/// before anything is built: it answers questions with probabilities and cannot
/// carry out a subagent's task.
pub async fn create_worker_provider(
    worker: &DefaultWorker,
    chat_model: &ModelConfig,
    temperature: Option<f32>,
) -> Result<Arc<dyn Provider>> {
    if crate::providers::decision::chat::is_decision_chat_provider(&worker.provider) {
        return Err(anyhow!(WorkerUnavailable::DecisionModel));
    }
    let model = worker_model_config(worker, chat_model, temperature)?;
    crate::providers::create(&worker.provider, model).await
}

/// Why the default worker could not run a subagent, as a reason the parent can
/// read. Only [`WorkerUnavailable::DecisionModel`] is ever constructed as an
/// error; the rest are read off the error the spawn produced.
#[derive(Debug, thiserror::Error)]
pub enum WorkerUnavailable {
    #[error(
        "it is a decision model, which answers questions with probabilities and cannot carry \
         out a subagent's task"
    )]
    DecisionModel,
}

/// The reason a refused or failed default worker gives the parent, read off
/// the error its spawn produced.
pub fn fallback_reason(error: &anyhow::Error) -> String {
    if let Some(refusal) = error.downcast_ref::<PrivacyRefusal>() {
        match refusal {
            PrivacyRefusal::PrivateChildOfPublicParent { .. } => {
                return "that model is private and this chat is public, and a subagent always \
                        runs at its chat's privacy tier"
                    .to_string();
            }
            PrivacyRefusal::PublicChildOfPrivateParent { .. } => {
                return "that model is public and this chat is private, and a subagent always \
                        runs at its chat's privacy tier"
                    .to_string();
            }
            PrivacyRefusal::SpawnCrossesAffiliation { .. } => {
                return "that model is covered by a different institution's agreements than \
                        this chat's model"
                    .to_string();
            }
            _ => {}
        }
    }
    if let Some(reason) = error.downcast_ref::<WorkerUnavailable>() {
        return reason.to_string();
    }
    format!("it could not be started ({error})")
}

/// What the parent reads when its subagent runs on the chat's model in place
/// of the default worker model. Once per spawn, beside the other spawn notes.
///
/// It names the model and the reason and says nothing needs doing: the spawn
/// succeeded, on the model the chat would have used before the default existed.
pub fn fallback_note(worker: &DefaultWorker, reason: &str) -> String {
    format!(
        "Note: this subagent runs on this chat's own model, not on the default worker model {}, \
         because {reason}. The subagent started normally; to use another model for it, name \
         one in `settings`.",
        worker.label()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn worker(provider: &str, model: &str) -> DefaultWorker {
        DefaultWorker {
            provider: provider.to_string(),
            model: model.to_string(),
        }
    }

    #[test]
    fn a_named_provider_or_model_wins_over_the_default_worker() {
        let set = Some(worker("openai", "gpt-4o-mini"));
        assert_eq!(
            child_model_for_spawn(Some("anthropic"), None, set.clone(), "openai", "gpt-4o"),
            ChildModel::Named
        );
        assert_eq!(
            child_model_for_spawn(None, Some("gpt-4.1"), set.clone(), "openai", "gpt-4o"),
            ChildModel::Named
        );
        assert_eq!(
            child_model_for_spawn(Some("anthropic"), Some("claude"), set, "openai", "gpt-4o"),
            ChildModel::Named
        );
    }

    #[test]
    fn an_unnamed_spawn_runs_on_the_default_worker_when_one_is_set() {
        assert_eq!(
            child_model_for_spawn(
                None,
                None,
                Some(worker("openai", "gpt-4o-mini")),
                "anthropic",
                "claude-opus"
            ),
            ChildModel::DefaultWorker(worker("openai", "gpt-4o-mini"))
        );
    }

    #[test]
    fn an_unnamed_spawn_runs_on_the_chat_model_when_no_default_is_set() {
        assert_eq!(
            child_model_for_spawn(None, None, None, "openai", "gpt-4o"),
            ChildModel::ChatModel
        );
    }

    #[test]
    fn a_default_that_is_the_chat_model_keeps_the_chat_model() {
        assert_eq!(
            child_model_for_spawn(
                None,
                None,
                Some(worker("openai", "gpt-4o")),
                "openai",
                "gpt-4o"
            ),
            ChildModel::ChatModel
        );
        // The same model name on another provider is another model.
        assert_eq!(
            child_model_for_spawn(
                None,
                None,
                Some(worker("openrouter", "gpt-4o")),
                "openai",
                "gpt-4o"
            ),
            ChildModel::DefaultWorker(worker("openrouter", "gpt-4o"))
        );
    }

    #[test]
    fn a_blank_name_in_settings_is_not_a_named_model() {
        assert_eq!(
            child_model_for_spawn(
                Some("  "),
                Some(""),
                Some(worker("openai", "gpt-4o-mini")),
                "anthropic",
                "claude"
            ),
            ChildModel::DefaultWorker(worker("openai", "gpt-4o-mini"))
        );
    }

    async fn read_with(overrides: &[(&str, &str)]) -> Option<DefaultWorker> {
        let overrides: HashMap<String, String> = overrides
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        crate::config::with_config_overrides(overrides, async { configured_default_worker() }).await
    }

    #[tokio::test]
    async fn the_setting_needs_both_halves() {
        assert_eq!(
            read_with(&[
                (WORKER_PROVIDER_KEY, "openai"),
                (WORKER_MODEL_KEY, " gpt-4o-mini ")
            ])
            .await,
            Some(worker("openai", "gpt-4o-mini"))
        );
        assert_eq!(
            read_with(&[(WORKER_PROVIDER_KEY, "openai"), (WORKER_MODEL_KEY, "")]).await,
            None,
            "a provider with an empty model is not a default"
        );
        assert_eq!(
            read_with(&[(WORKER_PROVIDER_KEY, ""), (WORKER_MODEL_KEY, "gpt-4o-mini")]).await,
            None,
            "a model with no provider is not a default"
        );
    }

    #[test]
    fn the_note_names_the_model_and_the_reason_and_asks_for_nothing() {
        let note = fallback_note(
            &worker("openai", "gpt-4o-mini"),
            "that model is public and this chat is private, and a subagent always runs at its \
             chat's privacy tier",
        );
        assert!(note.contains("gpt-4o-mini (openai)"), "{note}");
        assert!(note.contains("this chat is private"), "{note}");
        assert!(note.contains("started normally"), "{note}");
        // The hidden-directive shape the prompt-injection guard flags in tool
        // output (commit 313bbd88).
        assert!(!note.to_lowercase().contains("do not tell"), "{note}");
    }

    #[test]
    fn a_privacy_refusal_reads_as_the_tier_it_crossed() {
        use crate::privacy::ProviderTier;
        let upgrade = anyhow::Error::from(PrivacyRefusal::spawn_upgrade(ProviderTier::Private));
        assert!(fallback_reason(&upgrade).contains("this chat is public"));
        let downgrade = anyhow::Error::from(PrivacyRefusal::spawn_downgrade(ProviderTier::Public));
        assert!(fallback_reason(&downgrade).contains("this chat is private"));
        let affiliation = anyhow::Error::from(PrivacyRefusal::spawn_affiliation(None, None));
        assert!(fallback_reason(&affiliation).contains("institution"));
        let decision = anyhow!(WorkerUnavailable::DecisionModel);
        assert!(fallback_reason(&decision).contains("decision model"));
        let other = anyhow!("Unknown provider: nope");
        assert_eq!(
            fallback_reason(&other),
            "it could not be started (Unknown provider: nope)"
        );
    }
}
