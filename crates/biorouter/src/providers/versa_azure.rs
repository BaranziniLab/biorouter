use anyhow::Result;
use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use super::api_client::{ApiClient, AuthMethod, AuthProvider};
use super::azureauth::{AuthError, AzureAuth};
use super::base::{
    ConfigKey, MessageStream, ModelInfo, Provider, ProviderMetadata, ProviderUsage, Usage,
};
use super::errors::ProviderError;
use super::formats::openai::{create_request, get_usage, response_to_message};
use super::provider_binding::{
    model_without_restore_marker, ProviderRestoreBinding, SecretFreeEndpoint,
    VersaAzureCredentialSource,
};
use super::retry::ProviderRetry;
use super::utils::{
    azure_chat_completions_path as build_chat_completions_path, get_model,
    handle_response_openai_compat, handle_status_openai_compat, stream_openai_compat, ImageFormat,
};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::privacy::ProviderTier;
use crate::providers::utils::RequestLog;
use rmcp::model::Tool;

pub const VERSA_AZURE_ENDPOINT: &str = "https://unified-api.ucsf.edu/general";
pub const VERSA_AZURE_DEPLOYMENT: &str = "gpt-5.5-2026-04-24";
pub const VERSA_AZURE_API_VERSION: &str = "2025-01-01-preview";
pub const VERSA_AZURE_DOC_URL: &str = "http://biorouter.ucsf.edu/docs";

// Versa proxies Azure OpenAI deployments; the authoritative list lives on the
// (login-gated) UCSF wiki "Models, deployments, and API endpoints in UCSF
// Versa". Public UCSF Versa docs are MyAccess-gated, so keep this list to
// deployments verified against the UCSF endpoint. Removed: o1-2024-12-17 and
// o3-mini-2025-01-31 (deprecated on Azure, retiring Jul/Aug 2026).
pub const VERSA_AZURE_KNOWN_MODELS: &[&str] = &[
    "gpt-5.5-2026-04-24",
    "gpt-5.4-mini-2026-03-17",
    "gpt-5.4-nano-2026-03-17",
    "gpt-5.2-2025-12-11",
    "gpt-5-2025-08-07",
    "gpt-4.1-2025-04-14",
    "gpt-4.1-mini-2025-04-14",
    "gpt-4o-2024-11-20",
    "o4-mini-2025-04-16",
];

fn versa_azure_model_supports_vision(name: &str) -> bool {
    !name.contains("codex")
        && (name.starts_with("gpt-5")
            || name.starts_with("gpt-4.1")
            || name.starts_with("gpt-4o")
            || name.starts_with("o3")
            || name.starts_with("o4"))
}

#[derive(Debug)]
pub struct VersaAzureProvider {
    api_client: ApiClient,
    deployment_name: String,
    api_version: String,
    model: ModelConfig,
    name: String,
    /// The endpoint this instance resolved at construction. `tier()` reads it,
    /// never the provider's name — the three `AZURE_OPENAI_*` keys are shared
    /// with the public `azure_openai` provider and are user-writable.
    resolved_endpoint: String,
    credential_source: VersaAzureCredentialSource,
}

impl Serialize for VersaAzureProvider {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("VersaAzureProvider", 2)?;
        state.serialize_field("deployment_name", &self.deployment_name)?;
        state.serialize_field("api_version", &self.api_version)?;
        state.end()
    }
}

struct VersaAzureAuthProvider {
    auth: AzureAuth,
}

#[async_trait]
impl AuthProvider for VersaAzureAuthProvider {
    async fn get_auth_header(&self) -> Result<(String, String)> {
        let auth_token = self
            .auth
            .get_token()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get authentication token: {}", e))?;

        match self.auth.credential_type() {
            super::azureauth::AzureCredentials::ApiKey(_) => {
                Ok(("api-key".to_string(), auth_token.token_value))
            }
            super::azureauth::AzureCredentials::DefaultCredential => Ok((
                "Authorization".to_string(),
                format!("Bearer {}", auth_token.token_value),
            )),
        }
    }
}

/// Resolve one endpoint override: this provider's own key first, the public
/// Azure provider's key second, the shipped UCSF default last.
///
/// Pure, and split out from `from_env`, because the ORDER is the whole fix and
/// a closure inside a function that reads global config cannot be tested.
/// A blank stored value is treated as absent -- writing an empty string into
/// the box in Advanced means "use the default", not "point at nowhere".
fn resolve_override(fallback: &str, own: Option<String>, legacy: Option<String>) -> String {
    [own, legacy]
        .into_iter()
        .flatten()
        .find(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

impl VersaAzureProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();

        // Overrides are read from this provider's OWN namespace first, and only
        // then from the public Azure provider's keys.
        //
        // ⚠ The order is the fix, not a preference. `AZURE_OPENAI_ENDPOINT` and
        // its two siblings belong to the `azure_openai` card, and
        // `check_provider_configured` reads exactly those to decide whether that
        // card says Configured. While onboarding wrote them on Versa's behalf,
        // connecting UCSF's PRIVATE Versa lit up the PUBLIC Azure OpenAI card as
        // configured -- a provider the user never set up, sitting one click away
        // in the same grid, and Public where Versa is Private. The legacy read
        // stays so an install that customised those keys before this change
        // keeps resolving to the same endpoint.
        // ⚠ Every key below is a STRING LITERAL passed straight to `get_param`,
        // and it has to stay that way. `privacy::config_keys` scans this file for
        // literal-keyed `get_param` calls to build the list of keys that move a
        // provider's tier, and a key assembled at runtime — even one as innocent
        // as a `|own, legacy|` closure parameter — is invisible to that scan. The
        // first draft of this fix did exactly that and took the three
        // `AZURE_OPENAI_*` keys off the privacy surface without anyone deciding
        // to; two tests in that module caught it.
        let endpoint = resolve_override(
            VERSA_AZURE_ENDPOINT,
            config.get_param::<String>("VERSA_AZURE_ENDPOINT").ok(),
            config.get_param::<String>("AZURE_OPENAI_ENDPOINT").ok(),
        );
        let deployment_name = resolve_override(
            VERSA_AZURE_DEPLOYMENT,
            config
                .get_param::<String>("VERSA_AZURE_DEPLOYMENT_NAME")
                .ok(),
            config
                .get_param::<String>("AZURE_OPENAI_DEPLOYMENT_NAME")
                .ok(),
        );
        let api_version = resolve_override(
            VERSA_AZURE_API_VERSION,
            config.get_param::<String>("VERSA_AZURE_API_VERSION").ok(),
            config.get_param::<String>("AZURE_OPENAI_API_VERSION").ok(),
        );

        // ⚠ `.ok()` here used to discard the REASON the key was unavailable, and
        // that is what produced the field report
        //
        //     Failed to get authentication token: Token exchange failed:
        //     Failed to execute Azure CLI: No such file or directory (os error 2)
        //
        // from a user who had a perfectly good `VERSA_AZURE_API_KEY`. Two very
        // different situations collapsed into one:
        //
        //   * no key configured        -> falling back to the Azure CLI is right
        //   * the key could not be READ -> falling back is wrong, and the CLI
        //     error names a tool the user never configured and does not have
        //
        // The second happens on macOS whenever the credential store refuses the
        // read: a Keychain ACL grant is bound to the binary's signature, so a
        // freshly signed build asks again, and a prompt nobody answers (or a
        // locked keychain) fails the read. This provider is "API Key only" --
        // its own description says so -- so a failed read must be reported, not
        // routed around.
        let credential_source = match config.get_secret::<String>("VERSA_AZURE_API_KEY") {
            Ok(key) if !key.trim().is_empty() => VersaAzureCredentialSource::ApiKey,
            // Configured but blank, or genuinely absent: the Azure CLI is the
            // documented alternative and the user may well intend it.
            Ok(_) | Err(crate::config::ConfigError::NotFound(_)) => {
                VersaAzureCredentialSource::AzureCli
            }
            // Anything else is the store failing, not the key being absent.
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "Could not read VERSA_AZURE_API_KEY from the credential store: {error}\n\n\
                     The key appears to be configured, so this is the store refusing the \
                     read rather than a missing key. On macOS a Keychain grant is tied to \
                     the application's signature, so a newly installed or re-signed build \
                     asks for permission again -- answer the prompt with \u{201c}Always \
                     Allow\u{201d}. Biorouter did NOT silently fall back to the Azure CLI, \
                     because this provider signs in with the API key."
                ));
            }
        };

        Self::from_resolved(
            model,
            SecretFreeEndpoint::new(endpoint)?,
            deployment_name,
            api_version,
            credential_source,
        )
    }

    pub(crate) fn from_resolved(
        model: ModelConfig,
        endpoint: SecretFreeEndpoint,
        deployment_name: String,
        api_version: String,
        credential_source: VersaAzureCredentialSource,
    ) -> Result<Self> {
        let binding = ProviderRestoreBinding::VersaAzure {
            model: model.clone(),
            endpoint: endpoint.clone(),
            deployment: deployment_name.clone(),
            api_version: api_version.clone(),
            credential_source,
        };
        binding.validate()?;

        let config = crate::config::Config::global();
        let api_key = match credential_source {
            VersaAzureCredentialSource::ApiKey => {
                let key = config
                    .get_secret::<String>("VERSA_AZURE_API_KEY")
                    .map_err(|_| anyhow::anyhow!("VERSA_AZURE_API_KEY is not configured"))?;
                anyhow::ensure!(!key.trim().is_empty(), "VERSA_AZURE_API_KEY is empty");
                Some(key)
            }
            VersaAzureCredentialSource::AzureCli => None,
        };
        let auth = AzureAuth::new(api_key).map_err(|e| match e {
            AuthError::Credentials(msg) => anyhow::anyhow!("Credentials error: {}", msg),
            AuthError::TokenExchange(msg) => anyhow::anyhow!("Token exchange error: {}", msg),
        })?;

        let auth_provider = VersaAzureAuthProvider { auth };
        let api_client = ApiClient::new(
            endpoint.as_str().to_string(),
            AuthMethod::Custom(Box::new(auth_provider)),
        )?;

        Ok(Self {
            api_client,
            deployment_name,
            api_version,
            model,
            name: Self::metadata().name,
            resolved_endpoint: endpoint.into_string(),
            credential_source,
        })
    }

    /// The single source of truth for the Azure deployment path, shared by the
    /// blocking and streaming paths so they cannot drift.
    fn chat_completions_path(&self) -> String {
        build_chat_completions_path(&self.deployment_name, &self.api_version)
    }

    async fn post(&self, payload: &Value) -> Result<Value, ProviderError> {
        let path = self.chat_completions_path();
        let response = self.api_client.response_post(&path, payload).await?;
        handle_response_openai_compat(response).await
    }

    /// The exact request body `stream()` posts. Extracted so a test can assert
    /// on the payload the *provider* builds rather than on `create_request`'s
    /// output — the `for_streaming = true` argument below is the whole change,
    /// and a test that calls `create_request` directly re-supplies that
    /// argument itself and so cannot detect it being flipped here.
    fn build_stream_payload(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<Value> {
        // `for_streaming = true` sets both `stream: true` and
        // `stream_options: {"include_usage": true}`, which is what makes Azure
        // OpenAI emit a final usage-bearing chunk. Without it, usage/cost
        // tracking silently reports zeros on this path.
        create_request(
            &self.model,
            system,
            messages,
            tools,
            &ImageFormat::OpenAi,
            true,
        )
    }
}

#[async_trait]
impl Provider for VersaAzureProvider {
    fn metadata() -> ProviderMetadata {
        let models = VERSA_AZURE_KNOWN_MODELS
            .iter()
            .map(|&name| {
                let info = ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit());
                if versa_azure_model_supports_vision(name) {
                    info.with_vision()
                } else {
                    info
                }
            })
            .collect();

        ProviderMetadata::with_models(
            "versa_azure",
            "Versa API Azure",
            "UCSF ChatGPT via Azure OpenAI. API Key only; endpoint and deployment are pre-configured.",
            VERSA_AZURE_DEPLOYMENT,
            models,
            VERSA_AZURE_DOC_URL,
            // ⚠ The API key, and NOTHING else. This provider's own description
            // says "endpoint and deployment are pre-configured", and declaring
            // them here contradicted it with a user-visible consequence.
            //
            // `AZURE_OPENAI_ENDPOINT` / `_DEPLOYMENT_NAME` / `_API_VERSION` are
            // the generic Azure provider's namespace, and
            // `AZURE_OPENAI_DEPLOYMENT_NAME` is the ONE key `azure_openai`
            // requires without a default — i.e. the key its
            // `check_provider_configured` turns on. The setup form persists a
            // declared key's default (DefaultProviderSetupForm seeds it as a
            // value; DefaultSubmitHandler submits it), so configuring UCSF's
            // PRIVATE Versa lit up the PUBLIC, COMMERCIAL "Azure OpenAI" card as
            // Configured — pointing at the UCSF endpoint while carrying the
            // weaker Public tier. Reported from the field.
            //
            // Removing them costs nothing: `from_env` reads the same names with
            // `unwrap_or_else(|_| VERSA_AZURE_*)`, so an operator who sets them
            // in the environment or config still overrides, and an install that
            // sets nothing still gets the UCSF gateway.
            vec![ConfigKey::new("VERSA_AZURE_API_KEY", true, true, None)],
        )
        .with_unlisted_models()
        // The shipped endpoint is the UCSF gateway, so a default install is
        // Private. An instance that resolved elsewhere says so itself, below.
        .with_tier(ProviderTier::Private)
        // The type-level half of the same claim, for the surfaces that must
        // name the institution BEFORE anything is configured — see
        // `ProviderMetadata::institutions`. `affiliation()` below is the
        // instance answer and outranks it wherever the daemon resolved one.
        .with_institution(super::UCSF_INSTITUTION)
    }

    fn get_name(&self) -> &str {
        &self.name
    }

    fn restore_binding(&self) -> ProviderRestoreBinding {
        ProviderRestoreBinding::VersaAzure {
            model: model_without_restore_marker(self.model.clone()),
            endpoint: SecretFreeEndpoint::new(self.resolved_endpoint.clone())
                .expect("resolved Versa Azure endpoint must remain valid"),
            deployment: self.deployment_name.clone(),
            api_version: self.api_version.clone(),
            credential_source: self.credential_source,
        }
    }

    fn tier(&self) -> ProviderTier {
        crate::providers::ucsf_gateway_tier(&self.resolved_endpoint)
    }

    /// DR-26: `Institution("ucsf")` — decided by the **same** resolved endpoint
    /// as the tier above, through the same host check, so the two can never
    /// disagree about a repointed instance.
    fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
        crate::providers::ucsf_gateway_affiliation(&self.resolved_endpoint)
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model.clone()
    }

    #[tracing::instrument(
        skip(self, model_config, system, messages, tools),
        fields(model_config, input, output, input_tokens, output_tokens, total_tokens)
    )]
    async fn complete_with_model(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let payload = create_request(
            model_config,
            system,
            messages,
            tools,
            &ImageFormat::OpenAi,
            false,
        )?;
        let mut log = RequestLog::start(model_config, &payload)?;
        let response = self
            .with_retry(|| async {
                let payload_clone = payload.clone();
                self.post(&payload_clone).await
            })
            .await
            .inspect_err(|e| {
                let _ = log.provider_error(e);
            })?;

        let message = response_to_message(&response)?;
        let usage = response.get("usage").map(get_usage).unwrap_or_else(|| {
            tracing::debug!("Failed to get usage data");
            Usage::default()
        });
        let response_model = get_model(&response);
        log.write(&response, Some(&usage))?;
        Ok((message, ProviderUsage::new(response_model, usage)))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_restart_steering(&self) -> bool {
        true
    }

    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let payload = self.build_stream_payload(system, messages, tools)?;
        let mut log = RequestLog::start(&self.model, &payload)?;

        let path = self.chat_completions_path();
        let response = self
            .with_retry(|| async {
                let resp = self.api_client.response_post(&path, &payload).await?;
                handle_status_openai_compat(resp).await
            })
            .await
            .inspect_err(|e| {
                let _ = log.provider_error(e);
            })?;

        stream_openai_compat(response, log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_prefers_versas_own_key_then_the_legacy_one_then_the_default() {
        // Own key wins.
        assert_eq!(
            resolve_override("default", Some("own".into()), Some("legacy".into())),
            "own"
        );
        // An install that customised the shared key BEFORE the namespace existed
        // still resolves to the endpoint it chose.
        assert_eq!(
            resolve_override("default", None, Some("legacy".into())),
            "legacy"
        );
        // Nothing stored: the shipped UCSF default, which is why onboarding never
        // needed to write these keys at all.
        assert_eq!(resolve_override("default", None, None), "default");
        // Blank is absent, not "point at nowhere".
        assert_eq!(
            resolve_override("default", Some("   ".into()), None),
            "default"
        );
    }

    use crate::providers::api_client::AuthMethod;
    use crate::providers::formats::openai::create_request;

    /// A provider wired exactly like `from_env` builds one, minus the global
    /// config lookup. The point is that the assertions below run against a real
    /// `VersaAzureProvider`, so they gate `stream()`'s own code rather than
    /// re-asserting what `create_request` does when the test hands it the same
    /// arguments.
    fn test_provider() -> VersaAzureProvider {
        let api_client = ApiClient::new(
            VERSA_AZURE_ENDPOINT.to_string(),
            AuthMethod::ApiKey {
                header_name: "api-key".to_string(),
                key: "test-key".to_string(),
            },
        )
        .expect("api client builds");

        VersaAzureProvider {
            api_client,
            deployment_name: VERSA_AZURE_DEPLOYMENT.to_string(),
            api_version: VERSA_AZURE_API_VERSION.to_string(),
            model: ModelConfig::new_or_fail(VERSA_AZURE_DEPLOYMENT),
            name: "versa_azure".to_string(),
            resolved_endpoint: VERSA_AZURE_ENDPOINT.to_string(),
            credential_source: VersaAzureCredentialSource::ApiKey,
        }
    }

    /// Task 5 rule 2, **wired** — not just the predicate behind it.
    ///
    /// `providers::ucsf_gateway_tier` is unit-tested on its own in
    /// `tier_tests.rs`, but a test of the predicate alone cannot see whether
    /// this provider calls it, or hands it the right field. Replace the body of
    /// `tier()` with an unconditional `Private`, or point it at a field that is
    /// not the resolved endpoint, and every one of those tests still passes.
    /// This one does not: the three `AZURE_OPENAI_*` keys are shared with the
    /// public `azure_openai` provider, so a `tier()` that ignores the endpoint
    /// hands a private badge to a provider posting transcripts wherever the
    /// user's config points.
    #[test]
    fn tier_follows_the_endpoint_this_instance_resolved() {
        let shipped = test_provider();
        assert_eq!(shipped.resolved_endpoint, VERSA_AZURE_ENDPOINT);
        assert_eq!(shipped.tier(), ProviderTier::Private);

        let mut elsewhere = test_provider();
        elsewhere.resolved_endpoint = "https://evil.example.com/general".to_string();
        // Same name, same metadata, same everything a name-keyed rule can see.
        assert_eq!(elsewhere.get_name(), shipped.get_name());
        assert_eq!(
            VersaAzureProvider::metadata().tier,
            ProviderTier::Private,
            "the type-level claim is still Private; only the instance demotes"
        );
        assert_eq!(elsewhere.tier(), ProviderTier::Public);
    }

    /// DR-26 (Task 46) rule, **wired** — the same argument as the tier test
    /// above, for the third axis.
    ///
    /// `providers::ucsf_gateway_affiliation` is unit-tested on its own in
    /// `affiliation_tests.rs`, but that cannot see whether this provider calls
    /// it or hands it the right field. Returning an unconditional
    /// `Institution("ucsf")` here — or keying it on `get_name()`, which is the
    /// obvious implementation — passes every one of those tests and hands a UCSF
    /// badge to an instance posting prompts wherever the user's shared
    /// `AZURE_OPENAI_ENDPOINT` points.
    #[test]
    fn affiliation_follows_the_endpoint_this_instance_resolved() {
        use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};

        let shipped = test_provider();
        assert_eq!(
            shipped.affiliation(),
            Some(ModelAffiliation::institution(InstitutionId::new("ucsf")))
        );

        let mut elsewhere = test_provider();
        elsewhere.resolved_endpoint = "https://evil.example.com/general".to_string();
        assert_eq!(elsewhere.get_name(), shipped.get_name());
        assert_eq!(
            elsewhere.affiliation(),
            None,
            "an instance that lost Private must lose `ucsf` with it"
        );
    }

    /// The **type-level** half, which is what a catalog can group by before
    /// anything is configured. `GET /config/providers` resolves the instance
    /// affiliation above only for a CONFIGURED provider, so on a fresh machine
    /// this metadata claim is the only thing that can name the institution.
    ///
    /// ⚠ It is deliberately weaker than `affiliation()`: it says where this
    /// provider SHIPS pointed, so a repointed instance still carries `ucsf`
    /// here while the test above proves `affiliation()` correctly drops it.
    /// A surface must prefer the instance answer wherever the daemon resolved
    /// one — see `ProviderMetadata::institutions`.
    #[test]
    fn the_shipped_metadata_names_the_institution_for_an_unconfigured_row() {
        let institutions = VersaAzureProvider::metadata().institutions.clone();
        assert_eq!(institutions.len(), 1);
        assert_eq!(institutions[0].id, "ucsf");
        assert_eq!(institutions[0].display_name.as_deref(), Some("UCSF"));
    }

    #[test]
    fn restore_binding_keeps_the_exact_route_and_auth_mode_without_the_api_key() {
        let provider = test_provider();
        let encoded = serde_json::to_value(provider.restore_binding()).unwrap();
        assert_eq!(encoded["kind"], "versa_azure");
        assert_eq!(encoded["endpoint"], VERSA_AZURE_ENDPOINT);
        assert_eq!(encoded["deployment"], VERSA_AZURE_DEPLOYMENT);
        assert_eq!(encoded["api_version"], VERSA_AZURE_API_VERSION);
        assert_eq!(encoded["credential_source"], "api_key");
        assert!(!encoded.to_string().contains("test-key"));
    }

    #[tokio::test]
    async fn exact_credential_mode_never_switches_during_restore() {
        use std::collections::HashMap;

        let endpoint = || SecretFreeEndpoint::new(VERSA_AZURE_ENDPOINT.into()).unwrap();
        let missing = crate::config::with_config_overrides(
            HashMap::from([("VERSA_AZURE_API_KEY".into(), String::new())]),
            async {
                VersaAzureProvider::from_resolved(
                    ModelConfig::new_or_fail(VERSA_AZURE_DEPLOYMENT),
                    endpoint(),
                    VERSA_AZURE_DEPLOYMENT.into(),
                    VERSA_AZURE_API_VERSION.into(),
                    VersaAzureCredentialSource::ApiKey,
                )
            },
        )
        .await;
        assert!(
            missing.is_err(),
            "API-key mode must fail closed on an empty key"
        );

        let cli = crate::config::with_config_overrides(
            HashMap::from([(
                "VERSA_AZURE_API_KEY".into(),
                "new-key-that-must-not-change-mode".into(),
            )]),
            async {
                VersaAzureProvider::from_resolved(
                    ModelConfig::new_or_fail(VERSA_AZURE_DEPLOYMENT),
                    endpoint(),
                    VERSA_AZURE_DEPLOYMENT.into(),
                    VERSA_AZURE_API_VERSION.into(),
                    VersaAzureCredentialSource::AzureCli,
                )
            },
        )
        .await
        .unwrap();
        let encoded = serde_json::to_string(&cli.restore_binding()).unwrap();
        assert!(encoded.contains("azure_cli"));
        assert!(!encoded.contains("new-key-that-must-not-change-mode"));
    }

    /// The regression this file exists for: `stream()` must build a *streaming*
    /// payload. Flipping `for_streaming` to false in `build_stream_payload`
    /// sends a non-streaming body down a streaming-decoding path, which fails
    /// at the first chunk with "Failed to parse streaming chunk" and breaks
    /// every turn on this provider. Asserting on `create_request` directly
    /// cannot catch that, because the test supplies the flag itself.
    #[test]
    fn provider_stream_payload_opts_into_streaming_with_usage() {
        let provider = test_provider();
        let payload = provider
            .build_stream_payload("sys", &[], &[])
            .expect("streaming payload builds");

        assert_eq!(
            payload["stream"],
            serde_json::json!(true),
            "stream() must request a streamed response"
        );
        assert_eq!(
            payload["stream_options"]["include_usage"],
            serde_json::json!(true),
            "Azure needs stream_options.include_usage or usage/cost tracking breaks"
        );
    }

    /// `stream()` and `complete()` must post to the same deployment path, and
    /// `supports_streaming()` must stay true — it is hardcoded, so if it were
    /// removed the provider would quietly fall back to blocking generation and
    /// the latency win this change exists for would vanish with a green suite.
    #[test]
    fn provider_streams_posts_and_advertises_restart_steering() {
        let provider = test_provider();

        assert!(
            provider.supports_streaming(),
            "versa_azure must advertise streaming; without it the agent takes the \
             blocking complete() path and tool cards only appear at end of generation"
        );
        assert!(!provider.supports_live_steering());
        assert!(
            provider.supports_restart_steering(),
            "Versa Azure cannot inject into a running HTTP response, so a queued steer must restart it"
        );
        assert_eq!(
            provider.chat_completions_path(),
            "openai/deployments/gpt-5.5-2026-04-24/chat/completions?api-version=2025-01-01-preview"
        );
    }

    /// The Azure deployment path is shared by `complete` and `stream`; if these
    /// ever drift, streaming silently 404s while completion keeps working.
    #[test]
    fn chat_completions_path_is_azure_deployment_shaped() {
        let path = build_chat_completions_path("gpt-5.5-2026-04-24", "2025-01-01-preview");
        assert_eq!(
            path,
            "openai/deployments/gpt-5.5-2026-04-24/chat/completions?api-version=2025-01-01-preview"
        );
        assert!(
            !path.starts_with("chat/completions"),
            "versa_azure must not post to the plain OpenAI path"
        );
    }

    /// Guards the real trap: Azure only reports token usage on a streamed
    /// response when `stream_options.include_usage` is set.
    #[test]
    fn streaming_payload_sets_stream_and_usage_options() {
        let model = ModelConfig::new_or_fail(VERSA_AZURE_DEPLOYMENT);
        let payload = create_request(&model, "sys", &[], &[], &ImageFormat::OpenAi, true)
            .expect("streaming request should build");

        assert_eq!(payload["stream"], serde_json::json!(true));
        assert_eq!(
            payload["stream_options"]["include_usage"],
            serde_json::json!(true),
            "Azure needs stream_options.include_usage or usage/cost tracking breaks"
        );
    }

    #[test]
    fn non_streaming_payload_does_not_set_stream() {
        let model = ModelConfig::new_or_fail(VERSA_AZURE_DEPLOYMENT);
        let payload = create_request(&model, "sys", &[], &[], &ImageFormat::OpenAi, false)
            .expect("request should build");

        assert!(payload.get("stream").is_none());
        assert!(payload.get("stream_options").is_none());
    }
}

#[cfg(test)]
mod shared_namespace_tests {
    use super::*;

    /// Configuring UCSF's PRIVATE Versa must not configure the PUBLIC Azure card.
    ///
    /// Reported from the field: setting up "Versa API Azure" made "Azure OpenAI"
    /// — public, commercial, explicitly not HIPAA-compliant — show a green
    /// Configured check. The mechanism was a shared config namespace:
    ///
    ///   * `azure_openai` requires `AZURE_OPENAI_DEPLOYMENT_NAME`, and it is the
    ///     ONE required key it declares WITHOUT a default, so
    ///     `check_provider_configured` turns on precisely that key.
    ///   * `versa_azure` used to declare the same key (plus ENDPOINT and
    ///     API_VERSION) with UCSF defaults, and the setup form persists a
    ///     declared key's default — `DefaultProviderSetupForm` seeds it as a
    ///     value and `DefaultSubmitHandler` submits it.
    ///
    /// So one provider's setup silently satisfied another's configured-check,
    /// and the other was the weaker tier. This asserts the rule that prevents
    /// it, rather than the single key that happened to leak.
    #[test]
    fn versa_azure_declares_no_key_that_belongs_to_the_public_azure_provider() {
        let versa = VersaAzureProvider::metadata();
        let public = crate::providers::azure::AzureProvider::metadata();

        let public_required_without_default: Vec<&str> = public
            .config_keys
            .iter()
            .filter(|key| key.required && key.default.is_none())
            .map(|key| key.name.as_str())
            .collect();
        assert!(
            !public_required_without_default.is_empty(),
            "if the public provider stops having a deciding key this test is \
             vacuous — re-derive it rather than deleting it"
        );

        for key in &versa.config_keys {
            assert!(
                !public_required_without_default.contains(&key.name.as_str()),
                "versa_azure declares `{}`, which is what flips the PUBLIC \
                 azure_openai provider to Configured. Two providers must not \
                 share a config namespace when one of them is a different \
                 privacy tier.",
                key.name
            );
        }
    }

    /// …and the endpoint still resolves, because the provider falls back to its
    /// own constants rather than to a persisted key.
    #[test]
    fn dropping_the_shared_keys_does_not_change_where_versa_points() {
        assert_eq!(VERSA_AZURE_ENDPOINT, "https://unified-api.ucsf.edu/general");
        let versa = VersaAzureProvider::metadata();
        assert_eq!(
            versa.config_keys.len(),
            1,
            "the API key, and nothing else — the description says endpoint and \
             deployment are pre-configured"
        );
        assert_eq!(versa.config_keys[0].name, "VERSA_AZURE_API_KEY");
    }
}
