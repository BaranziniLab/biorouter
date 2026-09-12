use biorouter::config::declarative_providers::load_provider;
use biorouter::config::Config;
use biorouter::providers::base::{ConfigKey, ProviderMetadata, ProviderType};
use biorouter::providers::coding_agent::discovery::{self, CodingAgentKind};
use std::env;

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
    /// `coding_agent::discovery`'s module header). The catalog's status pill says
    /// it, and a turn that reaches a signed-out CLI fails with the vendor's own
    /// login command.
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
    use biorouter::providers::base::Provider;
    use biorouter::providers::claude_code::ClaudeCodeProvider;
    use biorouter::providers::codex::CodexProvider;

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
}
