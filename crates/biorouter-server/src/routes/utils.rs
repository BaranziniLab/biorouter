//! Provider readiness, as `GET /config/providers` reports it.
//!
//! The implementation lives in the core crate
//! (`biorouter::providers::readiness`), because the agent's `subagent` tool
//! asks the same question: which chat providers are set up on this computer.
//! Re-exported here under the names the routes have always used, so there is
//! exactly one answer and the settings page and the agent cannot disagree.

pub use biorouter::providers::readiness::{
    check_provider_configured, provider_readiness, ProviderReadiness,
};

#[cfg(test)]
mod tests {
    //! The behaviour is tested beside the logic, in
    //! `biorouter::providers::readiness`. What is pinned here is only that the
    //! names the routes import are that logic, not a copy of it.

    use super::*;
    use biorouter::providers::base::{ConfigKey, ProviderMetadata, ProviderType};

    /// The routes' names answer with the core function: the type is the core
    /// one (this assignment would not compile against a copy), and the same
    /// key shape the core tests use is judged through the re-export.
    #[test]
    fn the_routes_answer_with_the_core_readiness() {
        let _same_type: fn(
            &ProviderMetadata,
            ProviderType,
        ) -> biorouter::providers::readiness::ProviderReadiness = provider_readiness;

        let mut metadata = ProviderMetadata::empty();
        metadata.name = "routes_utils_reexport_probe".to_string();
        metadata.config_keys = vec![ConfigKey::new(
            "ROUTES_UTILS_REEXPORT_PORT",
            true,
            false,
            Some("1"),
        )];
        let _env = env_lock::lock_env([("ROUTES_UTILS_REEXPORT_PORT", Some("11543"))]);
        assert_eq!(
            provider_readiness(&metadata, ProviderType::Builtin),
            ProviderReadiness::Configured
        );
        assert!(check_provider_configured(&metadata, ProviderType::Builtin));
    }
}
