//! The `AWS_*` settings BioRouter's own stores hold, handed to the AWS SDK **in
//! code** rather than through the process environment.
//!
//! # What this replaces, and why it had to go
//!
//! `bedrock.rs` and `sagemaker_tgi.rs` both opened `from_env` with the same
//! closure: read `config.all_values()` and `config.all_secrets()`, keep every
//! key starting with `AWS_`, and `std::env::set_var` each one so the AWS SDK's
//! environment-based chain would find it. Two things were wrong with it, and
//! only the first is the one people notice.
//!
//! * **It is unsound.** `std::env::set_var` mutates a process-global table with
//!   no synchronization; any concurrent `getenv` anywhere in the process — the
//!   SDK's own shim included — is a data race. Rust 2024 makes the function
//!   `unsafe` for exactly this reason. Binding a provider is not a startup-only
//!   act here: a user switches models mid-session, and a subagent binds its own.
//!
//! * **It published the user's credentials to every subprocess.** `all_secrets`
//!   is the keyring (or `secrets.yaml`), so the keys being exported are real
//!   `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN` values, not merely a region.
//!   Every process spawned afterwards inherits the environment — and one of the
//!   things the agent spawns is its own shell. A chat on a **public** model with
//!   `developer__shell` could simply `echo $AWS_SECRET_ACCESS_KEY`. Nothing in
//!   the privacy lattice stops it: the tier gates decide which *model* may see a
//!   conversation, not what a shell can read out of its own environment, and the
//!   general filesystem read-deny (§9.5, DR-14) is deferred.
//!
//! The alternative was already written down in this workspace, at
//! [`super::auto_detect`]: values reach a provider through a task-local override
//! or through the client builder, "never `std::env::set_var`".
//!
//! # The rule this module keeps
//!
//! **The store beats the environment, exactly as the export did.**
//! `std::env::set_var` overwrites, so a key held in `config.yaml` or the keyring
//! took precedence over the same variable already in the environment. Every
//! setting applied below is applied *last*, for that reason: an explicit value
//! on the loader wins over what the SDK would have read for itself. Dropping
//! that precedence would be a silent configuration regression riding inside a
//! security fix.
//!
//! **Absent means absent.** When the store holds no credentials, no endpoint or
//! no region, nothing is set and the SDK's own chain — environment, SSO, profile
//! files, instance metadata — runs untouched. Two rows in
//! `providers::bedrock_namespace_tests` depend on that: one where the
//! credentials live only in the environment, one where the endpoint does.
//!
//! # The SigV4-vs-bearer trap, and the call made here
//!
//! The SDK reads `AWS_BEARER_TOKEN_BEDROCK` from the environment by itself and,
//! unless an auth scheme was chosen **in code**, authenticates with that bearer
//! token instead of signing. [`super::versa_bedrock`] pins
//! `auth_scheme_preference(["sigv4"])` for that reason, and its comment records
//! the incident.
//!
//! The public cards deliberately do **not** pin it when the store supplies an
//! access key and secret. Under the export those two landed in the environment
//! and an environment bearer token still won the scheme, so pinning would change
//! *which credential an existing install authenticates with* — a behaviour
//! change smuggled inside a security fix. Handing the same credentials to the
//! builder instead of to the environment leaves that resolution where it was.
//!
//! A bearer token held in **BioRouter's own store** is the one case that must
//! pin, because without the export the SDK never sees it at all; there,
//! `httpBearerAuth` is chosen explicitly. Both halves are pinned by tests.

use std::collections::BTreeMap;

use aws_sdk_bedrockruntime::config::{Credentials, Token};
use serde_json::Value;

use crate::config::Config;

/// The generic endpoint override the SDK honours for every service.
pub const ENDPOINT_URL: &str = "AWS_ENDPOINT_URL";

const ACCESS_KEY_ID: &str = "AWS_ACCESS_KEY_ID";
const SECRET_ACCESS_KEY: &str = "AWS_SECRET_ACCESS_KEY";
const SESSION_TOKEN: &str = "AWS_SESSION_TOKEN";
const BEARER_TOKEN_BEDROCK: &str = "AWS_BEARER_TOKEN_BEDROCK";
const REGION: &str = "AWS_REGION";
const PROFILE: &str = "AWS_PROFILE";

/// Every `AWS_*` key BioRouter's own stores hold, and nothing from the
/// environment.
///
/// The environment is deliberately absent: the SDK reads it for itself, and the
/// whole point of this type is to stop BioRouter writing to it. Secrets are read
/// after config values and win a clash, which is the order the export applied
/// them in (`all_values()` then `all_secrets()`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredAwsSettings {
    values: BTreeMap<String, String>,
}

impl StoredAwsSettings {
    /// Read the stored `AWS_*` keys. A store that cannot be read yields an empty
    /// set — the same outcome the export's `if let Ok(map)` produced, and the
    /// right one: a provider whose credentials live in the environment or in an
    /// AWS profile must still bind when the keyring refuses a read.
    #[must_use]
    pub fn read(config: &Config) -> Self {
        let mut settings = Self::default();
        for source in [config.all_values(), config.all_secrets()] {
            let Ok(map) = source else { continue };
            settings.absorb(map);
        }
        settings
    }

    /// Take the `AWS_*` string keys out of one store's map.
    ///
    /// A blank value reads as **absent**, the rule `Paths::get_dir` and every
    /// other resolver in this workspace already apply: a field the user cleared
    /// in Settings is persisted as `""`, and honouring it would aim the SDK at
    /// an empty endpoint or sign with an empty key rather than falling through.
    fn absorb(&mut self, map: std::collections::HashMap<String, Value>) {
        for (key, value) in map {
            if !key.starts_with("AWS_") {
                continue;
            }
            let Value::String(text) = value else { continue };
            if text.trim().is_empty() {
                continue;
            }
            self.values.insert(key, text);
        }
    }

    /// A stored key's value, or `None` when the store does not hold it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// The stored region, for a caller that resolves its own region first.
    #[must_use]
    pub fn region(&self) -> Option<&str> {
        self.get(REGION)
    }

    /// Static credentials, when the store holds a complete pair.
    ///
    /// A session token alone is not credentials, and neither is an access key id
    /// without its secret: a partial pair falls through to the SDK's own chain
    /// rather than binding the provider to something that cannot sign.
    #[must_use]
    pub fn credentials(&self, provider_name: &'static str) -> Option<Credentials> {
        let access_key_id = self.get(ACCESS_KEY_ID)?;
        let secret_access_key = self.get(SECRET_ACCESS_KEY)?;
        Some(Credentials::new(
            access_key_id,
            secret_access_key,
            self.get(SESSION_TOKEN).map(str::to_string),
            None,
            provider_name,
        ))
    }

    /// The first endpoint override the store holds, tried in the order given.
    ///
    /// Callers pass their service's own variable ahead of [`ENDPOINT_URL`], so a
    /// service-specific endpoint beats the global one exactly as it does inside
    /// the SDK.
    #[must_use]
    pub fn endpoint_url(&self, keys: &[&str]) -> Option<&str> {
        keys.iter().find_map(|key| self.get(key))
    }

    /// Apply everything the store holds to `loader`, and say nothing when it
    /// holds nothing.
    ///
    /// Call this **after** any region or profile the caller resolved for itself:
    /// the store beat the environment under the export, and these assignments
    /// are what keeps that true.
    #[must_use]
    pub fn apply(
        &self,
        mut loader: aws_config::ConfigLoader,
        provider_name: &'static str,
        endpoint_keys: &[&str],
    ) -> aws_config::ConfigLoader {
        if let Some(profile) = self.get(PROFILE) {
            loader = loader.profile_name(profile);
        }
        if let Some(region) = self.region() {
            loader = loader.region(aws_config::Region::new(region.to_string()));
        }
        if let Some(endpoint) = self.endpoint_url(endpoint_keys) {
            loader = loader.endpoint_url(endpoint);
        }
        if let Some(credentials) = self.credentials(provider_name) {
            // No `auth_scheme_preference` here, deliberately — see the module
            // docs. These credentials used to be exported into the environment,
            // where an environment bearer token still outranked them.
            loader = loader.credentials_provider(credentials);
        } else if let Some(bearer) = self.get(BEARER_TOKEN_BEDROCK) {
            // The opposite case, and the one that must choose in code: the SDK
            // reads this variable only from the environment, so a token held in
            // BioRouter's store reaches it through nothing but these two lines.
            loader = loader
                .token_provider(Token::new(bearer.to_string(), None))
                .auth_scheme_preference(["httpBearerAuth".into()]);
        }
        loader
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> StoredAwsSettings {
        let mut settings = StoredAwsSettings::default();
        settings.absorb(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), Value::String((*v).to_string())))
                .collect(),
        );
        settings
    }

    #[test]
    fn a_complete_pair_becomes_static_credentials() {
        let creds = settings(&[
            (ACCESS_KEY_ID, "AKIAEXAMPLE"),
            (SECRET_ACCESS_KEY, "shhh"),
            (SESSION_TOKEN, "temporary"),
        ])
        .credentials("test")
        .expect("a complete pair");
        assert_eq!(creds.access_key_id(), "AKIAEXAMPLE");
        assert_eq!(creds.secret_access_key(), "shhh");
        assert_eq!(creds.session_token(), Some("temporary"));
    }

    /// Half a pair cannot sign. Binding the provider to it would turn a
    /// perfectly good SSO or profile setup into an authentication failure.
    #[test]
    fn a_partial_pair_is_not_credentials() {
        assert!(settings(&[(ACCESS_KEY_ID, "AKIAEXAMPLE")])
            .credentials("test")
            .is_none());
        assert!(settings(&[(SECRET_ACCESS_KEY, "shhh")])
            .credentials("test")
            .is_none());
        assert!(settings(&[(SESSION_TOKEN, "temporary")])
            .credentials("test")
            .is_none());
    }

    /// A blank value is a field the user cleared, not a setting. Honouring it
    /// would aim the SDK at an empty endpoint.
    #[test]
    fn blank_and_non_aws_values_are_absent() {
        let stored = settings(&[
            (REGION, "   "),
            (ENDPOINT_URL, ""),
            ("OPENAI_API_KEY", "not-ours"),
            ("AWS_PROFILE", "research"),
        ]);
        assert_eq!(stored.region(), None);
        assert_eq!(stored.get(ENDPOINT_URL), None);
        assert_eq!(stored.get("OPENAI_API_KEY"), None);
        assert_eq!(stored.get(PROFILE), Some("research"));
    }

    #[test]
    fn a_service_endpoint_beats_the_generic_one() {
        let keys = ["AWS_ENDPOINT_URL_BEDROCK_RUNTIME", ENDPOINT_URL];
        let stored = settings(&[
            (ENDPOINT_URL, "https://generic.example"),
            (
                "AWS_ENDPOINT_URL_BEDROCK_RUNTIME",
                "https://service.example",
            ),
        ]);
        assert_eq!(stored.endpoint_url(&keys), Some("https://service.example"));
        assert_eq!(
            settings(&[(ENDPOINT_URL, "https://generic.example")]).endpoint_url(&keys),
            Some("https://generic.example")
        );
        assert_eq!(settings(&[]).endpoint_url(&keys), None);
    }

    /// Secrets are absorbed after config values, so a key held in both resolves
    /// to the secret — the order the export applied them in.
    #[test]
    fn a_secret_beats_a_config_value_on_the_same_key() {
        let mut stored = settings(&[(ACCESS_KEY_ID, "from-config")]);
        stored.absorb(
            [(
                ACCESS_KEY_ID.to_string(),
                Value::String("from-secrets".to_string()),
            )]
            .into_iter()
            .collect(),
        );
        assert_eq!(stored.get(ACCESS_KEY_ID), Some("from-secrets"));
    }

    /// The whole point: nothing here writes to the process environment. A grep
    /// is the only assertion that can prove a negative about a module.
    ///
    /// The needle is assembled rather than written, so this line does not match
    /// itself — the first draft failed for exactly that reason.
    #[test]
    fn this_module_never_writes_the_environment() {
        let needle = concat!("set_", "var");
        let source = include_str!("aws_stored_settings.rs");
        let writes = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| line.contains(needle))
            .count();
        assert_eq!(writes, 0, "the replacement must not export anything");
    }
}
