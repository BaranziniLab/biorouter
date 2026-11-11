//! Whether a provider is set up on this computer: the one answer behind the
//! "Configured" mark on the Providers and Models settings page
//! (`GET /config/providers`, which re-exports this from
//! `biorouter-server`'s `routes::utils`) and behind the agent's view of which
//! chat providers it may start a subagent on (`subagent_tool`).
//!
//! It lived in the server crate until the `subagent` tool needed it: the agent
//! was told nothing about which providers were set up, guessed, and sent a
//! Versa model name to `claude_code` and `codex` (live QA, 2026-11-11). One
//! implementation, so the settings page and the agent cannot disagree.

use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::declarative_providers::load_provider;
use crate::config::Config;
use crate::providers::base::{ConfigKey, ProviderMetadata, ProviderType};
use crate::providers::coding_agent::discovery::{self, CodingAgentKind};

/// Whether a provider can be used, as `GET /config/providers` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderReadiness {
    /// Its keys are saved, and nothing it needs that can be checked cheaply is
    /// missing. The only state `is_configured` is true for.
    Configured,
    /// Not set up: a key it requires has not been saved.
    NotConfigured,
    /// The user set it up, but something it needs at runtime is missing — the
    /// one-line reason. Today that is a coding agent whose command key is saved
    /// and whose CLI does not resolve.
    ///
    /// ⚠ **Only what a `stat` can see.** Signed-out is deliberately NOT a reason
    /// here: learning it means spawning the vendor CLI, and this runs for every
    /// provider on every `GET /config/providers` (see
    /// `coding_agent::discovery`'s module header) and whenever the agent's tool
    /// list is assembled. The catalog's status pill says it, and a turn that
    /// reaches a signed-out CLI fails with the vendor's own login command.
    Unavailable(String),
}

pub fn check_provider_configured(metadata: &ProviderMetadata, provider_type: ProviderType) -> bool {
    provider_readiness(metadata, provider_type) == ProviderReadiness::Configured
}

/// [`check_provider_configured`], with the reason when a provider the user set
/// up still cannot run.
pub fn provider_readiness(
    metadata: &ProviderMetadata,
    provider_type: ProviderType,
) -> ProviderReadiness {
    if !keys_are_saved(metadata, provider_type) {
        return ProviderReadiness::NotConfigured;
    }

    // A coding agent's one key only NAMES a command, and a saved name is not an
    // installed CLI. Reporting it configured anyway is how the row read "Not
    // installed" and "Configured" on one line, and how the model picker offered
    // a provider whose `from_env` would refuse the bind. The same
    // `resolve_configured` backs `/coding_agents/status`, so the two answers
    // agree by construction rather than by a test remembering to compare them.
    if provider_type == ProviderType::Builtin {
        if let Some(kind) = CodingAgentKind::from_provider_id(&metadata.name) {
            if discovery::resolve_configured(kind).is_none() {
                return ProviderReadiness::Unavailable(kind.not_installed_summary());
            }
        }
    }

    ProviderReadiness::Configured
}

/// Whether every key the provider requires has been saved — the whole of what
/// "configured" meant before a saved key could fail to be enough.
fn keys_are_saved(metadata: &ProviderMetadata, provider_type: ProviderType) -> bool {
    let config = Config::global();

    if provider_type == ProviderType::Custom || provider_type == ProviderType::Declarative {
        if let Ok(loaded_provider) = load_provider(metadata.name.as_str()) {
            return config
                .get_secret::<String>(&loaded_provider.config.api_key_env)
                .is_ok();
        }
    }
    // Special case: Zero-config providers (no config keys)
    if metadata.config_keys.is_empty() {
        // Check if the provider has been explicitly configured via the UI
        let configured_marker = format!("{}_configured", metadata.name);
        return config.get_param::<bool>(&configured_marker).is_ok();
    }

    // Get all required keys
    let required_keys: Vec<&ConfigKey> = metadata
        .config_keys
        .iter()
        .filter(|key| key.required)
        .collect();

    // Special case: If a provider has exactly one required key and that key
    // has a default value, check if it's explicitly set
    if required_keys.len() == 1 && required_keys[0].default.is_some() {
        let key = &required_keys[0];

        // Check if the key is explicitly set (either in env or config)
        let is_set_in_env = env::var(&key.name).is_ok();
        let is_set_in_config = config.get(&key.name, key.secret).is_ok();

        return is_set_in_env || is_set_in_config;
    }

    // Special case: If a provider has only optional keys with defaults,
    // check if a configuration marker exists
    if required_keys.is_empty() && !metadata.config_keys.is_empty() {
        let all_optional_with_defaults = metadata
            .config_keys
            .iter()
            .all(|key| !key.required && key.default.is_some());

        if all_optional_with_defaults {
            // Check if the provider has been explicitly configured via the UI
            let configured_marker = format!("{}_configured", metadata.name);
            return config.get_param::<bool>(&configured_marker).is_ok();
        }
    }

    // For providers with multiple keys or keys without defaults:
    // Find required keys that don't have default values
    let required_non_default_keys: Vec<&ConfigKey> = required_keys
        .iter()
        .filter(|key| key.default.is_none())
        .cloned()
        .collect();

    // If there are no non-default keys, check ONLY Biorouter's stored config (not env vars).
    // Config::get_param() checks env vars first by design, so we use all_values()/all_secrets()
    // which read directly from the config file and keychain. This prevents a false "Configured"
    // state after Remove: providers like Bedrock set AWS_ env vars during initialization via
    // std::env::set_var(), and those vars can also exist from the system environment
    // (e.g. ~/.zshrc), surviving even after the user deletes the stored config.
    if required_non_default_keys.is_empty() {
        let file_values = config.all_values().unwrap_or_default();
        let secret_values = config.all_secrets().unwrap_or_default();
        return required_keys.iter().any(|key| {
            if key.secret {
                secret_values.contains_key(&key.name)
            } else {
                file_values.contains_key(&key.name)
            }
        });
    }

    // Otherwise, all non-default keys must be set (env vars are a valid source here since
    // keys without defaults won't be set accidentally by provider initialization)
    required_non_default_keys.iter().all(|key| {
        let is_set_in_env = env::var(&key.name).is_ok();
        let is_set_in_config = config.get(&key.name, key.secret).is_ok();

        is_set_in_env || is_set_in_config
    })
}

// --- The chat providers, as the agent is told about them --------------------

/// One chat provider and whether it is set up, with the model names its
/// registry metadata declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatProviderStatus {
    pub name: String,
    pub readiness: ProviderReadiness,
    /// The registry's `default_model`; empty when the provider declares none.
    pub default_model: String,
    /// The registry's `known_models`, in declared order. May be stale, and
    /// many providers accept names not on it; it is guidance, not a gate.
    pub known_models: Vec<String>,
}

/// Every chat provider in the registry with its readiness, sorted by name.
///
/// Decision-model providers are left out (`decision::chat::DECISION_CHAT_PROVIDERS`):
/// they are not chat models, a spawn on one is refused for its own reason, and
/// the settings page gives them no chat row either.
///
/// Sorted, because the registry is a map and the order would otherwise change
/// from one process to the next: this feeds a tool description the model reads
/// every turn, and a description that reorders itself defeats prompt caching.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatProviderRoster {
    pub providers: Vec<ChatProviderStatus>,
}

impl ChatProviderRoster {
    /// Build from registry rows already judged, dropping decision providers and
    /// sorting by name. Pure, so tests can describe any machine.
    pub fn from_rows(
        rows: impl IntoIterator<Item = (ProviderMetadata, ProviderReadiness)>,
    ) -> Self {
        let mut providers: Vec<ChatProviderStatus> = rows
            .into_iter()
            .filter(|(metadata, _)| {
                !crate::providers::decision::chat::is_decision_chat_provider(&metadata.name)
            })
            .map(|(metadata, readiness)| ChatProviderStatus {
                known_models: metadata
                    .known_models
                    .iter()
                    .map(|model| model.name.clone())
                    .collect(),
                name: metadata.name,
                readiness,
                default_model: metadata.default_model,
            })
            .collect();
        providers.sort_by(|a, b| a.name.cmp(&b.name));
        providers.dedup_by(|a, b| a.name == b.name);
        Self { providers }
    }

    /// The providers set up on this computer, by name.
    pub fn configured(&self) -> impl Iterator<Item = &ChatProviderStatus> {
        self.providers
            .iter()
            .filter(|status| status.readiness == ProviderReadiness::Configured)
    }

    /// The providers that are not set up, or set up and unavailable, by name.
    pub fn not_configured(&self) -> impl Iterator<Item = &ChatProviderStatus> {
        self.providers
            .iter()
            .filter(|status| status.readiness != ProviderReadiness::Configured)
    }

    pub fn get(&self, name: &str) -> Option<&ChatProviderStatus> {
        self.providers.iter().find(|status| status.name == name)
    }
}

/// The roster as it stands now: every registered provider judged by
/// [`provider_readiness`]. Spawns nothing (a coding agent's CLI is found with a
/// `stat`, never run), so it is safe on a turn's path.
pub async fn chat_provider_roster() -> ChatProviderRoster {
    let rows = crate::providers::providers().await;
    ChatProviderRoster::from_rows(rows.into_iter().map(|(metadata, provider_type)| {
        let readiness = provider_readiness(&metadata, provider_type);
        (metadata, readiness)
    }))
}

/// How long [`cached_chat_provider_roster`] reuses an answer. The tool list is
/// assembled once per model call, several times a turn, and the readiness pass
/// reads the config for ~45 providers each time; ten seconds absorbs a turn's
/// burst while a provider the person sets up in Settings reaches the next turn.
const ROSTER_TTL: Duration = Duration::from_secs(10);

static ROSTER_CACHE: Mutex<Option<(Instant, Arc<ChatProviderRoster>)>> = Mutex::new(None);

/// [`chat_provider_roster`], reused for [`ROSTER_TTL`]. For the tool
/// description only: a decision (the spawn refusal) reads the fresh one.
pub async fn cached_chat_provider_roster() -> Arc<ChatProviderRoster> {
    if let Some(roster) = last_chat_provider_roster_within(ROSTER_TTL) {
        return roster;
    }
    // A unit test that lists tools must not read the operator's OS keychain: on macOS
    // that read waits on a Keychain prompt, and every serial bridge test hung on it.
    // The description text is pinned by the fixture tests in `subagent_tool.rs`.
    #[cfg(test)]
    {
        return Arc::new(ChatProviderRoster::from_rows(std::iter::empty::<(
            ProviderMetadata,
            ProviderReadiness,
        )>()));
    }
    #[allow(unreachable_code)]
    let roster = Arc::new(chat_provider_roster().await);
    if let Ok(mut cache) = ROSTER_CACHE.lock() {
        *cache = Some((Instant::now(), roster.clone()));
    }
    roster
}

/// The last roster computed, if it is younger than `max_age`. Synchronous, for
/// a builder that cannot await.
fn last_chat_provider_roster_within(max_age: Duration) -> Option<Arc<ChatProviderRoster>> {
    let cache = ROSTER_CACHE.lock().ok()?;
    let (at, roster) = cache.as_ref()?;
    (at.elapsed() < max_age).then(|| roster.clone())
}

#[cfg(test)]
mod tests {
    //! Issue F6 of the 2026-09-10 provider QA run: with `CODEX_COMMAND` pointed
    //! at a path that does not exist, the Codex row read "Not installed" and
    //! "✓ Configured" on one line, and Codex stayed selectable in the model
    //! picker.
    //!
    //! ⚠ **Every case sets the command key through the environment**, under
    //! `env_lock`'s one process-wide mutex. `Config::get_param` reads the
    //! environment before the config file, so neither half of the check ever
    //! reaches the developer's real `~/.config/biorouter` — and a saved
    //! `CODEX_COMMAND` there cannot decide what these tests see.

    use super::*;
    use crate::providers::base::{ModelInfo, Provider};
    use crate::providers::claude_code::ClaudeCodeProvider;
    use crate::providers::codex::CodexProvider;

    /// Pin `kind`'s command key to `value` for the life of the guard.
    fn command_pinned_to(kind: CodingAgentKind, value: &str) -> env_lock::EnvGuard<'static> {
        env_lock::lock_env([(kind.command_config_key(), Some(value))])
    }

    fn metadata_for(kind: CodingAgentKind) -> ProviderMetadata {
        match kind {
            CodingAgentKind::ClaudeCode => ClaudeCodeProvider::metadata(),
            CodingAgentKind::Codex => CodexProvider::metadata(),
        }
    }

    /// The reported defect. A saved key naming a CLI that is not there is not a
    /// configured provider, and it says why.
    #[test]
    fn a_coding_agent_whose_cli_is_missing_is_not_configured() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-dir").join("codex");

        for kind in CodingAgentKind::all() {
            let _env = command_pinned_to(kind, missing.to_str().unwrap());
            let metadata = metadata_for(kind);

            assert!(
                !check_provider_configured(&metadata, ProviderType::Builtin),
                "{kind:?} pointed at {} must not report is_configured",
                missing.display()
            );
            assert_eq!(
                provider_readiness(&metadata, ProviderType::Builtin),
                ProviderReadiness::Unavailable(kind.not_installed_summary()),
                "{kind:?}: the reason is the not-installed sentence, not a silent false"
            );
        }
    }

    /// Without this, the test above passes for a check that simply refuses every
    /// coding agent. A command that resolves is configured, with no reason.
    #[test]
    fn a_coding_agent_whose_cli_resolves_is_configured() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("codex");
        std::fs::write(&exe, b"#!/bin/sh\n").unwrap();

        for kind in CodingAgentKind::all() {
            let _env = command_pinned_to(kind, exe.to_str().unwrap());
            let metadata = metadata_for(kind);

            assert!(check_provider_configured(&metadata, ProviderType::Builtin));
            assert_eq!(
                provider_readiness(&metadata, ProviderType::Builtin),
                ProviderReadiness::Configured
            );
        }
    }

    /// ⚠ **The row and the status route must agree.** The pill beside the name
    /// comes from `/coding_agents/status` (`probe`), the check beside it from this
    /// function; F6 was the two disagreeing on one line. A pinned path that does
    /// not exist resolves nothing, so the probe returns without spawning.
    #[tokio::test]
    async fn not_configured_is_exactly_what_the_status_route_calls_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("codex");
        let _env = command_pinned_to(CodingAgentKind::Codex, missing.to_str().unwrap());

        let status = discovery::probe(CodingAgentKind::Codex).await;
        assert_eq!(status.auth, discovery::AuthState::NotInstalled);
        assert!(!check_provider_configured(
            &metadata_for(CodingAgentKind::Codex),
            ProviderType::Builtin
        ));
    }

    /// The CLI requirement is the coding agents' alone. A provider with the very
    /// same key shape — one required key with a default, which is how
    /// `llamacpp` reports configured through `LLAMACPP_PORT` — is still judged
    /// on the saved key and nothing else.
    #[test]
    fn the_cli_requirement_applies_to_the_coding_agents_only() {
        let mut metadata = ProviderMetadata::empty();
        metadata.name = "same_shape_as_a_coding_agent_f6".to_string();
        metadata.config_keys = vec![ConfigKey::new("F6_SAME_SHAPE_PORT", true, false, Some("1"))];
        let _env = env_lock::lock_env([("F6_SAME_SHAPE_PORT", Some("11543"))]);

        assert_eq!(
            provider_readiness(&metadata, ProviderType::Builtin),
            ProviderReadiness::Configured
        );
    }

    fn metadata_named(name: &str, default_model: &str, models: &[&str]) -> ProviderMetadata {
        let mut metadata = ProviderMetadata::empty();
        metadata.name = name.to_string();
        metadata.default_model = default_model.to_string();
        metadata.known_models = models
            .iter()
            .map(|model| ModelInfo::new(*model, 128_000))
            .collect();
        metadata
    }

    /// The roster is what the agent is told, so it must not carry decision
    /// models (they cannot run a subagent) and must not reorder itself from one
    /// process to the next (the registry is a map).
    #[test]
    fn the_roster_drops_decision_providers_and_sorts_by_name() {
        let roster = ChatProviderRoster::from_rows([
            (
                metadata_named("versa_azure", "gpt-5.5", &["gpt-5.5", "gpt-5.4"]),
                ProviderReadiness::Configured,
            ),
            (
                metadata_named("baseten_decision", "joey-1", &["joey-1"]),
                ProviderReadiness::Configured,
            ),
            (
                metadata_named("codex", "gpt-5.5-codex", &[]),
                ProviderReadiness::Unavailable("Codex is not installed.".to_string()),
            ),
            (
                metadata_named("claude_code", "sonnet", &["sonnet", "opus"]),
                ProviderReadiness::Configured,
            ),
            (
                metadata_named("ollama_decision", "tev1:0.8b", &[]),
                ProviderReadiness::NotConfigured,
            ),
        ]);

        let names: Vec<_> = roster.providers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["claude_code", "codex", "versa_azure"]);
        let configured: Vec<_> = roster.configured().map(|p| p.name.as_str()).collect();
        assert_eq!(configured, ["claude_code", "versa_azure"]);
        let not: Vec<_> = roster.not_configured().map(|p| p.name.as_str()).collect();
        assert_eq!(not, ["codex"]);
        assert_eq!(
            roster.get("versa_azure").unwrap().known_models,
            ["gpt-5.5", "gpt-5.4"]
        );
    }

    /// The live roster is built from the real registry: no decision provider,
    /// in name order, and the coding agents judged by the same `stat` the
    /// settings page uses.
    #[tokio::test]
    async fn the_live_roster_is_the_registry_without_decision_models() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("codex");
        let _env = command_pinned_to(CodingAgentKind::Codex, missing.to_str().unwrap());

        let roster = chat_provider_roster().await;
        assert!(roster
            .providers
            .iter()
            .all(|p| !crate::providers::decision::chat::is_decision_chat_provider(&p.name)));
        let names: Vec<_> = roster.providers.iter().map(|p| p.name.clone()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        assert_eq!(
            roster.get("codex").map(|p| p.readiness.clone()),
            Some(ProviderReadiness::Unavailable(
                CodingAgentKind::Codex.not_installed_summary()
            ))
        );
    }
}
