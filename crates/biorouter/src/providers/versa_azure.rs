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
/// The model a fresh Versa Azure chat starts on.
///
/// ⚠ A MODEL, not a deployment. This constant used to be `VERSA_AZURE_DEPLOYMENT`
/// and every request posted to it, whatever model the chat named — so choosing
/// any of the other eight models changed the label, the context gauge and the
/// cost basis while `gpt-5.5` kept answering (2026-09-10 QA run, finding F1).
/// A request now posts to the deployment [`VERSA_AZURE_DEPLOYMENTS`] maps its
/// own model to.
pub const VERSA_AZURE_DEFAULT_MODEL: &str = "gpt-5.5-2026-04-24";
pub const VERSA_AZURE_API_VERSION: &str = "2025-01-01-preview";
pub const VERSA_AZURE_DOC_URL: &str = "http://biorouter.ucsf.edu/docs";

/// Every model this provider offers, paired with the Azure deployment that
/// serves it at the UCSF gateway. The ONE list: `metadata()` advertises exactly
/// these models, and a request for one of them posts to exactly this deployment.
///
/// Measured on 2026-09-11, not inferred. The gateway has no listing endpoint —
/// `GET openai/deployments` and `GET openai/models` both answer 405 — so every
/// deployment was sent a one-shot completion, and each answered 200 with its own
/// name as `model`. The names are the full dated ids: the short aliases
/// (`gpt-5.5`, `gpt-4.1`, `gpt-4o`) are all `DeploymentNotFound`. The same run
/// sent each deployment a prompt just over its `MODEL_CONTEXT_WINDOWS` entry,
/// and every refusal named that window — for the gpt-5 family as an INPUT limit,
/// the window less the 128k reserved for output — so the registry needed no
/// change.
///
/// The authoritative list lives on the login-gated UCSF wiki ("Models,
/// deployments, and API endpoints in UCSF Versa"). o1-2024-12-17 and
/// o3-mini-2025-01-31 were removed earlier (deprecated on Azure, retiring
/// Jul/Aug 2026) and still answered on 2026-09-11; they stay removed.
/// `gpt-5-mini-2025-08-07` and `gpt-5-nano-2025-08-07` also answered and are
/// not offered yet.
pub const VERSA_AZURE_DEPLOYMENTS: &[(&str, &str)] = &[
    ("gpt-5.5-2026-04-24", "gpt-5.5-2026-04-24"),
    ("gpt-5.4-mini-2026-03-17", "gpt-5.4-mini-2026-03-17"),
    ("gpt-5.4-nano-2026-03-17", "gpt-5.4-nano-2026-03-17"),
    ("gpt-5.2-2025-12-11", "gpt-5.2-2025-12-11"),
    ("gpt-5-2025-08-07", "gpt-5-2025-08-07"),
    ("gpt-4.1-2025-04-14", "gpt-4.1-2025-04-14"),
    ("gpt-4.1-mini-2025-04-14", "gpt-4.1-mini-2025-04-14"),
    ("gpt-4o-2024-11-20", "gpt-4o-2024-11-20"),
    ("o4-mini-2025-04-16", "o4-mini-2025-04-16"),
];

/// The deployment the UCSF gateway serves `model` from, if the catalog knows one.
pub fn deployment_for_model(model: &str) -> Option<&'static str> {
    VERSA_AZURE_DEPLOYMENTS
        .iter()
        .find(|(name, _)| *name == model)
        .map(|(_, deployment)| *deployment)
}

fn is_catalog_deployment(value: &str) -> bool {
    VERSA_AZURE_DEPLOYMENTS
        .iter()
        .any(|(_, deployment)| *deployment == value)
}

/// What a restore binding stores in `deployment` for a model no deployment
/// serves, so a restored chat refuses it exactly as the live one did.
///
/// ⚠ A marker inside the existing `deployment: String`, not an `Option`, because
/// the binding is format v1 and has shipped (1.89.x–1.90.x): a changed shape is
/// a row those builds cannot parse, and a failed restore is a 500 on resume —
/// the chat would not open at all. The marker passes `validate_path_component`,
/// names what it means when someone reads the row, and a build that predates
/// this change posts to it and gets the gateway's `DeploymentNotFound`
/// (measured 2026-09-11) — a failed turn, never an answer from another
/// deployment.
const NO_DEPLOYMENT_ROUTE: &str =
    "no-versa-deployment-serves-this-model.biorouter-refuses-to-send-the-turn";

/// The deployment override `value` amounts to, if any.
///
/// Judges BOTH sources of one — a configured `VERSA_AZURE_DEPLOYMENT_NAME` and
/// the `deployment` a session's restore binding stored — so a live provider and
/// a restored one cannot disagree about whether an override is in force.
///
/// ⚠ A value that names a CATALOG deployment is not an override, and the fix
/// turns on it:
///
///   * Nobody chose it. The onboarding card upserts `VERSA_AZURE_DEPLOYMENT_NAME`
///     with the shipped default on every connect. Honouring it would pin every
///     onboarded install to one model — F1 again, in exactly the installs the
///     QA sandbox did not have.
///   * It is what every stored binding says. Before this change `deployment`
///     was the fixed default whatever the model, and a subagent's model override
///     still rewrites the binding's model without touching its route
///     (`subagent_tool.rs`). Re-deriving a catalog value from the model is what
///     heals those rows instead of carrying the wrong route forward.
///   * It adds nothing. Choosing that model reaches that deployment honestly;
///     an override can only make some other model's label point at it.
///
/// What is left is the real escape hatch: a deployment the catalog does not
/// know, which then serves every request.
///
/// A row written by a build that still read the public `azure_openai` card's
/// `AZURE_OPENAI_DEPLOYMENT_NAME` may carry that card's deployment here, and
/// nothing in the row tells it from a chosen one — so it is honoured like one
/// until the chat's model is picked again, which rebuilds through `from_env`.
fn explicit_override(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty() && value != NO_DEPLOYMENT_ROUTE && !is_catalog_deployment(value))
        .then(|| value.to_string())
}

/// The route a restore binding stores, and the one place that decides it: the
/// override if one is in force, else the model's catalog deployment, else
/// [`NO_DEPLOYMENT_ROUTE`]. [`explicit_override`] reads it back.
fn stored_route(deployment_override: Option<&str>, model: &str) -> String {
    deployment_override
        .or_else(|| deployment_for_model(model))
        .unwrap_or(NO_DEPLOYMENT_ROUTE)
        .to_string()
}

/// The refusal for a model no deployment serves, raised before the payload is
/// built — so nothing is logged and nothing leaves the machine.
///
/// `RequestFailed` whose text says "deployment not found", which
/// `ProviderError::kind` classifies as `ModelUnavailable`: not transient, so the
/// turn stops on the first attempt instead of retrying a request that can never
/// succeed, and the desktop titles it "Model unavailable".
fn no_deployment_error(model: &str) -> ProviderError {
    let available = VERSA_AZURE_DEPLOYMENTS
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    ProviderError::RequestFailed(format!(
        "no Versa deployment for model `{model}` (Azure deployment not found, so nothing \
         was sent); available: {available}. Switch this chat to one of those models."
    ))
}

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
    /// A deployment EVERY request posts to, whatever model it names — the
    /// operator's escape hatch for a deployment the catalog does not know yet.
    /// `None`, the normal case: each request posts to the deployment its own
    /// model maps to (see [`explicit_override`] for what can set it).
    deployment_override: Option<String>,
    api_version: String,
    model: ModelConfig,
    name: String,
    /// The endpoint this instance resolved at construction. `tier()` reads it,
    /// never the provider's name — `VERSA_AZURE_ENDPOINT` is user-writable, so
    /// an instance can resolve somewhere that is not the UCSF gateway.
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
        state.serialize_field(
            "deployment_name",
            &stored_route(self.deployment_override.as_deref(), &self.model.model_name),
        )?;
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

/// Resolve one override: this provider's own key, else the shipped UCSF
/// default. There is no third source — see `from_env` for the one there was.
///
/// A blank stored value is treated as absent -- writing an empty string into
/// the box in Advanced means "use the default", not "point at nowhere".
fn resolve_override(fallback: &str, own: Option<String>) -> String {
    own.filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

impl VersaAzureProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();

        // Overrides are read from this provider's OWN namespace, and nowhere
        // else.
        //
        // ⚠ `AZURE_OPENAI_ENDPOINT` and its two siblings belong to the public
        // `azure_openai` card — `check_provider_configured` reads them to decide
        // whether that card says Configured — and sharing them went wrong in
        // both directions. Onboarding WROTE them on Versa's behalf, so
        // connecting UCSF's PRIVATE Versa lit up the PUBLIC Azure OpenAI card as
        // configured; that is why these `VERSA_AZURE_*` keys exist
        // (2026-09-03). And Versa went on READING them as a fallback, so
        // whatever that card was set up with steered Versa: a company Azure
        // resource's endpoint received every Versa request — the transcript,
        // with `VERSA_AZURE_API_KEY` in its `api-key` header — refused the key,
        // and the instance turned Public. Its deployment and API version rode
        // along. The fallback is gone (2026-09-11).
        //
        // Every Versa setup form prefilled the endpoint constant, so an endpoint
        // this drops was either typed over the prefill or carried in from the
        // public card — the bug itself. The API version is the exception: the
        // onboarding card of 2026-05-30 to 2026-07-02 prefilled `2024-10-21`
        // into the legacy key, and those installs now send the shipped version
        // every other install sends.
        //
        // ⚠ Every key below is a STRING LITERAL passed straight to `get_param`,
        // and it has to stay that way. `privacy::config_keys` scans this file for
        // literal-keyed `get_param` calls to build the list of keys that move a
        // provider's tier, and a key assembled at runtime — even one as innocent
        // as an `|own, legacy|` closure parameter — is invisible to that scan.
        // The first draft of the 2026-09-03 namespacing did exactly that and
        // took three keys off the privacy surface without anyone deciding to;
        // two tests in that module caught it.
        let endpoint = resolve_override(
            VERSA_AZURE_ENDPOINT,
            config.get_param::<String>("VERSA_AZURE_ENDPOINT").ok(),
        );
        // There is no shipped default deployment any more: the model picks it.
        // What the configuration names is a CANDIDATE override, and
        // `explicit_override` decides whether it is one — which is what keeps
        // the default the onboarding card persists from pinning every model.
        //
        // ⚠ Versa's OWN key, and no fallback. `AZURE_OPENAI_DEPLOYMENT_NAME`
        // used to be read after it, and it is not Versa's to read: it is the
        // one key the public `azure_openai` card requires and ships no default
        // for, so it names a deployment on whatever Azure resource the user set
        // THAT card up with (`my-gpt4o`, or `gpt-4o` — Azure's habit of naming a
        // deployment after its model). The catalog knows no such name, so it
        // became an override serving every Versa request: DeploymentNotFound on
        // each turn, or — where it is a real UCSF deployment the catalog does
        // not offer — a silent answer from the wrong model. The fallback was
        // kept for installs whose pre-2026-09-03 Versa forms wrote that key,
        // and those forms prefilled catalog deployments, which are not
        // overrides; only a value someone typed over the prefill is dropped.
        let configured_deployment = config
            .get_param::<String>("VERSA_AZURE_DEPLOYMENT_NAME")
            .unwrap_or_default();
        let deployment_override = explicit_override(Some(&configured_deployment));
        match &deployment_override {
            Some(deployment) => tracing::info!(
                deployment = %deployment,
                "Versa Azure: a configured deployment override is in force; every request \
                 posts to it, whatever model the chat selected"
            ),
            None if !configured_deployment.trim().is_empty() => tracing::debug!(
                configured = %configured_deployment.trim(),
                "Versa Azure: the configured deployment is one the catalog already maps to a \
                 model, so it is not an override; each model posts to its own deployment"
            ),
            None => {}
        }
        let api_version = resolve_override(
            VERSA_AZURE_API_VERSION,
            config.get_param::<String>("VERSA_AZURE_API_VERSION").ok(),
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

        Self::build(
            model,
            SecretFreeEndpoint::new(endpoint)?,
            deployment_override,
            api_version,
            credential_source,
        )
    }

    /// Rebuild the provider a restore binding describes.
    ///
    /// `deployment` is the route the binding stored (see [`stored_route`]), and
    /// it is read back through [`explicit_override`] — the rule `from_env`
    /// applies to configuration — rather than trusted as the deployment to post
    /// to. That is what makes a row the old code wrote (a catalog deployment
    /// beside a DIFFERENT model) post to its own model's deployment now, instead
    /// of carrying the wrong route forward forever.
    ///
    /// Never fails for want of a deployment: a model none serves is refused per
    /// request, before anything is sent. Failing here would fail the restore,
    /// and a failed restore is a chat that will not open.
    pub(crate) fn from_resolved(
        model: ModelConfig,
        endpoint: SecretFreeEndpoint,
        deployment: String,
        api_version: String,
        credential_source: VersaAzureCredentialSource,
    ) -> Result<Self> {
        Self::build(
            model,
            endpoint,
            explicit_override(Some(&deployment)),
            api_version,
            credential_source,
        )
    }

    fn build(
        model: ModelConfig,
        endpoint: SecretFreeEndpoint,
        deployment_override: Option<String>,
        api_version: String,
        credential_source: VersaAzureCredentialSource,
    ) -> Result<Self> {
        let binding = ProviderRestoreBinding::VersaAzure {
            model: model.clone(),
            endpoint: endpoint.clone(),
            deployment: stored_route(deployment_override.as_deref(), &model.model_name),
            api_version: api_version.clone(),
            credential_source,
        };
        binding.validate()?;

        let config = crate::config::Config::global();
        let api_key = match credential_source {
            VersaAzureCredentialSource::ApiKey => {
                let key = config
                    .get_secret::<String>("VERSA_AZURE_API_KEY")
                    .map_err(|_| {
                        anyhow::anyhow!("VERSA_AZURE_API_KEY {}", super::CREDENTIAL_NEVER_SET)
                    })?;
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
            deployment_override,
            api_version,
            model,
            name: Self::metadata().name,
            resolved_endpoint: endpoint.into_string(),
            credential_source,
        })
    }

    /// The deployment a request for `model` posts to: the override if one is in
    /// force, else the deployment the catalog maps `model` to.
    ///
    /// It takes the model the REQUEST names, not the one this provider was built
    /// with, because they differ: `complete_fast` sends the fast model through
    /// `complete_with_model`, and it used to land on the main deployment too.
    fn deployment_for(&self, model: &str) -> Result<&str, ProviderError> {
        match &self.deployment_override {
            Some(deployment) => Ok(deployment),
            None => deployment_for_model(model).ok_or_else(|| no_deployment_error(model)),
        }
    }

    /// The single source of truth for the Azure deployment path, shared by the
    /// blocking and streaming paths so they cannot drift.
    fn chat_completions_path(&self, model: &str) -> Result<String, ProviderError> {
        Ok(build_chat_completions_path(
            self.deployment_for(model)?,
            &self.api_version,
        ))
    }

    async fn post(&self, path: &str, payload: &Value) -> Result<Value, ProviderError> {
        let response = self.api_client.response_post(path, payload).await?;
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
        let models = VERSA_AZURE_DEPLOYMENTS
            .iter()
            .map(|&(name, _)| {
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
            VERSA_AZURE_DEFAULT_MODEL,
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
            // Removing them costs nothing: an install that sets nothing still
            // gets the UCSF gateway, because `from_env` falls back to the
            // `VERSA_AZURE_*` constants, and an operator overrides through
            // Versa's own `VERSA_AZURE_*` keys (see `from_env`).
            vec![ConfigKey::new("VERSA_AZURE_API_KEY", true, true, None)],
        )
        // ⚠ No `with_unlisted_models()`, which this provider used to declare. A
        // model the catalog does not map has no deployment and is refused
        // before it is sent, so "Enter a model not listed..." could only offer
        // a choice that fails — or, with an override in force, one that changes
        // nothing but the label.
        //
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
            // Decided afresh on every call — the override if one is in force,
            // else THIS model's deployment — so a rebind can never leave a route
            // behind that names some other model's deployment.
            deployment: stored_route(self.deployment_override.as_deref(), &self.model.model_name),
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
        // First, before the payload exists: a model no deployment serves is
        // refused here, so nothing is logged and nothing is sent.
        let path = self.chat_completions_path(&model_config.model_name)?;
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
                self.post(&path, &payload_clone).await
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
        // Same order as `complete_with_model`: refuse before anything is built.
        let path = self.chat_completions_path(&self.model.model_name)?;
        let payload = self.build_stream_payload(system, messages, tools)?;
        let mut log = RequestLog::start(&self.model, &payload)?;

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

    /// That the public card's keys are never read is asserted where it can be
    /// seen — on a real `from_env` in `routing_tests` — not here.
    #[test]
    fn an_override_is_versas_own_key_or_the_default() {
        assert_eq!(resolve_override("default", Some("own".into())), "own");
        // Nothing stored: the shipped UCSF default, which is why onboarding never
        // needed to write these keys at all.
        assert_eq!(resolve_override("default", None), "default");
        // Blank is absent, not "point at nowhere".
        assert_eq!(resolve_override("default", Some("   ".into())), "default");
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
            deployment_override: None,
            api_version: VERSA_AZURE_API_VERSION.to_string(),
            model: ModelConfig::new_or_fail(VERSA_AZURE_DEFAULT_MODEL),
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
    /// This one does not: `VERSA_AZURE_ENDPOINT` is user-writable config, so a
    /// `tier()` that ignores the endpoint hands a private badge to a provider
    /// posting transcripts wherever that config points.
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
    /// badge to an instance posting prompts wherever the user's
    /// `VERSA_AZURE_ENDPOINT` points.
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
        assert_eq!(
            encoded["deployment"],
            deployment_for_model(VERSA_AZURE_DEFAULT_MODEL).unwrap()
        );
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
                    ModelConfig::new_or_fail(VERSA_AZURE_DEFAULT_MODEL),
                    endpoint(),
                    VERSA_AZURE_DEFAULT_MODEL.into(),
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
                    ModelConfig::new_or_fail(VERSA_AZURE_DEFAULT_MODEL),
                    endpoint(),
                    VERSA_AZURE_DEFAULT_MODEL.into(),
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
            provider
                .chat_completions_path(&provider.model.model_name)
                .unwrap(),
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
        let model = ModelConfig::new_or_fail(VERSA_AZURE_DEFAULT_MODEL);
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
        let model = ModelConfig::new_or_fail(VERSA_AZURE_DEFAULT_MODEL);
        let payload = create_request(&model, "sys", &[], &[], &ImageFormat::OpenAi, false)
            .expect("request should build");

        assert!(payload.get("stream").is_none());
        assert!(payload.get("stream_options").is_none());
    }

    /// The map the requests are routed by IS the measured snapshot, and the
    /// default model is on it.
    #[test]
    fn the_deployment_map_is_the_measured_snapshot() {
        assert_eq!(VERSA_AZURE_DEPLOYMENTS, super::routing_tests::MEASURED);
        for (model, deployment) in VERSA_AZURE_DEPLOYMENTS {
            assert_eq!(deployment_for_model(model), Some(*deployment));
        }
        assert!(deployment_for_model(VERSA_AZURE_DEFAULT_MODEL).is_some());
        assert_eq!(deployment_for_model("gpt-4.1-bogus-qa-probe"), None);
        // The short aliases are DeploymentNotFound at the gateway (measured).
        for alias in ["gpt-5.5", "gpt-4.1", "gpt-4o"] {
            assert_eq!(deployment_for_model(alias), None, "{alias}");
        }
    }

    #[test]
    fn only_a_deployment_the_catalog_does_not_know_is_an_override() {
        assert_eq!(explicit_override(None), None);
        assert_eq!(explicit_override(Some("")), None);
        assert_eq!(explicit_override(Some("   ")), None);
        assert_eq!(explicit_override(Some(NO_DEPLOYMENT_ROUTE)), None);
        for (_, deployment) in VERSA_AZURE_DEPLOYMENTS {
            assert_eq!(
                explicit_override(Some(deployment)),
                None,
                "{deployment} is a catalog deployment; the model picks it, an override cannot"
            );
        }
        assert_eq!(
            explicit_override(Some(" ucsf-preview-deployment ")).as_deref(),
            Some("ucsf-preview-deployment")
        );
        assert!(!is_catalog_deployment(NO_DEPLOYMENT_ROUTE));

        // Every default a setup surface ever persisted must stay a non-override:
        // the onboarding card writes gpt-5.5 into `VERSA_AZURE_DEPLOYMENT_NAME`,
        // and a session row written by a build that still read the legacy
        // `AZURE_OPENAI_DEPLOYMENT_NAME` stored that key's default as its route.
        // Today each is a catalog deployment, which is the only reason it is
        // ignored — so trimming one from the catalog would silently turn it into
        // an override and pin every chat that carries it. If this fails after a
        // trim, keep recognising the value (a retired-defaults list beside the
        // catalog) rather than editing it out of this test.
        for shipped_default in [
            "gpt-5.2-2025-12-11", // provider default 2026-05-07 .. 2026-07-02
            "gpt-5.5-2026-04-24", // provider default since; the onboarding card's
        ] {
            assert_eq!(
                explicit_override(Some(shipped_default)),
                None,
                "{shipped_default} was persisted by a setup form nobody chose it in"
            );
        }
    }

    /// A model no deployment serves still needs a VALID binding: `build`
    /// validates it, and a binding that fails validation is a restore that
    /// fails — a chat that will not open. Through the exact envelope a session
    /// row stores, and back, the refusal survives.
    #[tokio::test]
    async fn an_unmapped_models_refusal_survives_the_session_row() {
        use crate::providers::provider_binding::PersistedStandaloneProviderBinding;
        use std::collections::HashMap;

        let unmapped = "gpt-4.1-bogus-qa-probe";
        let endpoint = || SecretFreeEndpoint::new(VERSA_AZURE_ENDPOINT.into()).unwrap();
        let restored = crate::config::with_config_overrides(
            HashMap::from([("VERSA_AZURE_API_KEY".into(), "test-key".into())]),
            async {
                let bound = VersaAzureProvider::build(
                    ModelConfig::new_or_fail(unmapped),
                    endpoint(),
                    None,
                    VERSA_AZURE_API_VERSION.into(),
                    VersaAzureCredentialSource::ApiKey,
                )
                .expect("binding a model no deployment serves must not fail");
                let binding = bound.restore_binding();
                assert_eq!(
                    serde_json::to_value(&binding).unwrap()["deployment"],
                    NO_DEPLOYMENT_ROUTE
                );
                let row =
                    crate::providers::persisted_model_config_from_binding("versa_azure", binding)
                        .expect("the marker must pass the binding's own validation");
                let ProviderRestoreBinding::VersaAzure {
                    model,
                    endpoint,
                    deployment,
                    api_version,
                    credential_source,
                } = PersistedStandaloneProviderBinding::from_model_config(&row)
                    .unwrap()
                    .expect("a Versa row carries its exact route")
                    .into_binding("versa_azure")
                    .unwrap()
                else {
                    panic!("a Versa binding came back as another provider's");
                };
                VersaAzureProvider::from_resolved(
                    model,
                    endpoint,
                    deployment,
                    api_version,
                    credential_source,
                )
                .unwrap()
            },
        )
        .await;
        let error = restored
            .chat_completions_path(unmapped)
            .expect_err("the restored chat forgot that this model has no deployment");
        assert!(error.to_string().contains("no Versa deployment for model"));
    }

    /// `subagent_tool.rs` gives a child another model by rewriting only the
    /// binding's MODEL, leaving the parent's route in place. Through the factory
    /// path the child really takes, it must still post to its own model's
    /// deployment.
    #[tokio::test]
    async fn a_binding_whose_model_was_rewritten_follows_the_new_model() {
        use std::collections::HashMap;

        crate::config::with_config_overrides(
            HashMap::from([("VERSA_AZURE_API_KEY".into(), "test-key".into())]),
            async {
                let parent = VersaAzureProvider::from_resolved(
                    ModelConfig::new_or_fail(VERSA_AZURE_DEFAULT_MODEL),
                    SecretFreeEndpoint::new(VERSA_AZURE_ENDPOINT.into()).unwrap(),
                    VERSA_AZURE_DEFAULT_MODEL.into(),
                    VERSA_AZURE_API_VERSION.into(),
                    VersaAzureCredentialSource::ApiKey,
                )
                .unwrap();
                let mut binding = parent.restore_binding();
                binding.model_mut().model_name = "gpt-4o-2024-11-20".into();
                let row =
                    crate::providers::persisted_model_config_from_binding("versa_azure", binding)
                        .unwrap();
                let child = crate::providers::create_from_persisted("versa_azure", row)
                    .await
                    .unwrap();
                assert_eq!(child.get_model_config().model_name, "gpt-4o-2024-11-20");
                assert_eq!(
                    serde_json::to_value(child.restore_binding()).unwrap()["deployment"],
                    "gpt-4o-2024-11-20",
                    "the child kept its parent's deployment"
                );
            },
        )
        .await;
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

/// F1 of the 2026-09-10 QA run: the model a chat names must be the model that
/// answers, or the turn must fail and say so. It was neither — every request
/// posted to gpt-5.5's deployment, and a model that does not exist at all
/// completed a turn normally.
///
/// Each provider here is built the way production builds it, through
/// `from_env` or `from_resolved`, and only its HTTP client is then pointed at a
/// local stand-in, so every assertion is on the path a real request took. (Both
/// constructors insist on an HTTPS endpoint, which a local server cannot offer;
/// re-aiming the client is the one liberty taken.)
#[cfg(test)]
mod routing_tests {
    use super::*;
    use crate::providers::errors::ProviderErrorKind;
    use std::collections::HashMap;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    /// The catalog as measured on 2026-09-11 (see `VERSA_AZURE_DEPLOYMENTS`),
    /// written out rather than read back from the constant, so changing the
    /// catalog is a deliberate edit here as well — re-probe the gateway first.
    pub(super) const MEASURED: &[(&str, &str)] = &[
        ("gpt-5.5-2026-04-24", "gpt-5.5-2026-04-24"),
        ("gpt-5.4-mini-2026-03-17", "gpt-5.4-mini-2026-03-17"),
        ("gpt-5.4-nano-2026-03-17", "gpt-5.4-nano-2026-03-17"),
        ("gpt-5.2-2025-12-11", "gpt-5.2-2025-12-11"),
        ("gpt-5-2025-08-07", "gpt-5-2025-08-07"),
        ("gpt-4.1-2025-04-14", "gpt-4.1-2025-04-14"),
        ("gpt-4.1-mini-2025-04-14", "gpt-4.1-mini-2025-04-14"),
        ("gpt-4o-2024-11-20", "gpt-4o-2024-11-20"),
        ("o4-mini-2025-04-16", "o4-mini-2025-04-16"),
    ];

    /// The QA run's own probe: a deployment that does not exist.
    const UNMAPPED: &str = "gpt-4.1-bogus-qa-probe";

    /// A stand-in gateway. It answers every completion with the deployment named
    /// in the request path as `model` — which is what the real one does
    /// (measured), and what `token_events.model_id` ends up recording.
    async fn gateway() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(|request: &Request| {
                let deployment = request.url.path().split('/').nth(3).unwrap_or_default();
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "chatcmpl-routing-test",
                    "object": "chat.completion",
                    "created": 0,
                    "model": deployment,
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "ready"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                }))
            })
            .mount(&server)
            .await;
        server
    }

    async fn requested_paths(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .map(|request| request.url.path().to_string())
            .collect()
    }

    /// The `api-version` each request carried, in order.
    async fn requested_api_versions(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|request| {
                request
                    .url
                    .query_pairs()
                    .find(|(name, _)| name == "api-version")
                    .map(|(_, version)| version.into_owned())
            })
            .collect()
    }

    fn path_of(deployment: &str) -> String {
        format!("/openai/deployments/{deployment}/chat/completions")
    }

    fn aimed_at(mut provider: VersaAzureProvider, server: &MockServer) -> VersaAzureProvider {
        provider.api_client = ApiClient::new(
            server.uri(),
            AuthMethod::ApiKey {
                header_name: "api-key".to_string(),
                key: "test-key".to_string(),
            },
        )
        .expect("api client builds");
        provider
    }

    /// A key, Versa's deployment key as given, and its other two override keys
    /// blank — blank is absent — so the machine running the suite cannot leak
    /// its own configuration into what is being measured. Plus the public
    /// `azure_openai` card's deployment key as given, which `from_env` must not
    /// read; the tests that set it are how that is known.
    fn config(deployment: &str, public_deployment: &str) -> HashMap<String, String> {
        HashMap::from([
            ("VERSA_AZURE_API_KEY".into(), "test-key".into()),
            ("VERSA_AZURE_ENDPOINT".into(), String::new()),
            ("VERSA_AZURE_API_VERSION".into(), String::new()),
            ("VERSA_AZURE_DEPLOYMENT_NAME".into(), deployment.into()),
            (
                "AZURE_OPENAI_DEPLOYMENT_NAME".into(),
                public_deployment.into(),
            ),
        ])
    }

    async fn bound(model: &str, overrides: HashMap<String, String>) -> VersaAzureProvider {
        crate::config::with_config_overrides(
            overrides,
            VersaAzureProvider::from_env(ModelConfig::new_or_fail(model)),
        )
        .await
        .unwrap_or_else(|e| panic!("binding {model} must never fail for want of a route: {e}"))
    }

    /// A provider rebuilt from a restore binding that stored `deployment`.
    async fn restored(model: &str, deployment: &str) -> VersaAzureProvider {
        crate::config::with_config_overrides(config("", ""), async {
            VersaAzureProvider::from_resolved(
                ModelConfig::new_or_fail(model),
                SecretFreeEndpoint::new(VERSA_AZURE_ENDPOINT.into()).unwrap(),
                deployment.into(),
                VERSA_AZURE_API_VERSION.into(),
                VersaAzureCredentialSource::ApiKey,
            )
        })
        .await
        .unwrap_or_else(|e| panic!("a restore of {model} must never fail for want of a route: {e}"))
    }

    fn prompt() -> Vec<Message> {
        vec![Message::user().with_text("Reply with the single word ready.")]
    }

    /// The model the stand-in says answered — what `token_events.model_id` would
    /// record.
    async fn answered_by(provider: &VersaAzureProvider) -> Result<String, ProviderError> {
        provider
            .complete("system", &prompt(), &[])
            .await
            .map(|(_, usage)| usage.model)
    }

    #[tokio::test]
    async fn each_catalog_model_posts_to_its_own_deployment() {
        let server = gateway().await;
        for (model, _) in MEASURED {
            let provider = aimed_at(bound(model, config("", "")).await, &server);
            let answered = answered_by(&provider)
                .await
                .unwrap_or_else(|e| panic!("{model}: {e}"));
            assert_eq!(
                answered, *model,
                "the chat named {model}; another deployment answered"
            );
        }
        let expected: Vec<String> = MEASURED.iter().map(|(_, d)| path_of(d)).collect();
        assert_eq!(requested_paths(&server).await, expected);
    }

    /// `complete_fast` hands the FAST model to `complete_with_model`, so the
    /// deployment has to follow the model a request names, not the one the chat
    /// was bound to. It used to land on the chat's deployment too.
    #[tokio::test]
    async fn a_request_posts_to_the_deployment_of_the_model_it_names() {
        let server = gateway().await;
        let provider = aimed_at(bound("gpt-5.5-2026-04-24", config("", "")).await, &server);
        let (_, usage) = provider
            .complete_with_model(
                &ModelConfig::new_or_fail("gpt-5.4-mini-2026-03-17"),
                "system",
                &prompt(),
                &[],
            )
            .await
            .unwrap();
        assert_eq!(usage.model, "gpt-5.4-mini-2026-03-17");
        assert_eq!(
            requested_paths(&server).await,
            vec![path_of("gpt-5.4-mini-2026-03-17")]
        );
    }

    /// The QA probe itself. Refused readably, and NOTHING reaches the gateway —
    /// where it used to complete normally on gpt-5.5.
    #[tokio::test]
    async fn an_unmapped_model_is_refused_before_any_request_is_sent() {
        let server = gateway().await;
        let provider = aimed_at(bound(UNMAPPED, config("", "")).await, &server);

        let completed = answered_by(&provider)
            .await
            .expect_err("a model no deployment serves was answered");
        let streamed = match provider.stream("system", &prompt(), &[]).await {
            Ok(_) => panic!("a model no deployment serves was streamed"),
            Err(error) => error,
        };
        for error in [&completed, &streamed] {
            let text = error.to_string();
            assert!(
                text.contains(&format!("no Versa deployment for model `{UNMAPPED}`")),
                "{text}"
            );
            for (model, _) in MEASURED {
                assert!(
                    text.contains(model),
                    "the refusal must offer {model}: {text}"
                );
            }
            assert_eq!(error.kind(), ProviderErrorKind::ModelUnavailable, "{text}");
            assert!(
                !crate::agents::mistakes::is_recoverable(error),
                "a retry can never succeed, so the turn must stop on the first one: {text}"
            );
        }
        assert!(
            requested_paths(&server).await.is_empty(),
            "a refused turn reached the gateway"
        );

        // Positive control: the same stand-in DOES see a catalog model, so the
        // silence above is the refusal and not a server nobody could reach.
        let control = aimed_at(bound("gpt-5.5-2026-04-24", config("", "")).await, &server);
        answered_by(&control).await.unwrap();
        assert_eq!(
            requested_paths(&server).await,
            vec![path_of("gpt-5.5-2026-04-24")]
        );
    }

    /// The onboarding card upserts `VERSA_AZURE_DEPLOYMENT_NAME` with the
    /// shipped default on every connect, and until 2026-09-03 the setup form
    /// wrote the legacy key with the default of its day. Neither was a choice,
    /// so neither may pin the chat to one model.
    #[tokio::test]
    async fn a_persisted_default_deployment_does_not_pin_the_model() {
        let server = gateway().await;
        for overrides in [
            config("gpt-5.5-2026-04-24", ""),
            config("", "gpt-5.2-2025-12-11"),
            config("gpt-5.5-2026-04-24", "gpt-5.2-2025-12-11"),
        ] {
            let provider = aimed_at(bound("gpt-4.1-2025-04-14", overrides).await, &server);
            assert_eq!(answered_by(&provider).await.unwrap(), "gpt-4.1-2025-04-14");
        }
        // Nor is one of them licence to answer a model no deployment serves.
        let probe = aimed_at(
            bound(UNMAPPED, config("gpt-5.5-2026-04-24", "")).await,
            &server,
        );
        assert!(answered_by(&probe).await.is_err());
        assert_eq!(
            requested_paths(&server).await,
            vec![path_of("gpt-4.1-2025-04-14"); 3]
        );
    }

    /// `AZURE_OPENAI_DEPLOYMENT_NAME` belongs to the PUBLIC `azure_openai`
    /// provider: it is the one key that card requires and ships no default for,
    /// so whoever set it up typed a deployment on THEIR Azure resource into it.
    /// Versa used to read it as a fallback, and a name the catalog does not know
    /// then served every Versa request. None of these may route one — each
    /// fails differently at the real gateway, so each is here. A catalog name in
    /// that key was never an override; for those see
    /// `a_persisted_default_deployment_does_not_pin_the_model`.
    #[tokio::test]
    async fn the_public_azure_cards_deployment_never_routes_a_versa_request() {
        let server = gateway().await;
        let public_deployments = [
            // A company deployment: DeploymentNotFound on every Versa turn.
            "my-gpt4o",
            // Azure's habit of naming a deployment after its model. The short
            // aliases are DeploymentNotFound at the gateway (measured).
            "gpt-4o",
            // A real UCSF deployment the catalog does not offer: the gateway
            // ANSWERS, so the wrong model replies and nothing says so — F1.
            "gpt-5-mini-2025-08-07",
        ];
        for public_deployment in public_deployments {
            let chat = aimed_at(
                bound("gpt-4.1-2025-04-14", config("", public_deployment)).await,
                &server,
            );
            let answered = answered_by(&chat).await.unwrap();
            assert_eq!(
                requested_paths(&server).await.pop().unwrap_or_default(),
                path_of("gpt-4.1-2025-04-14"),
                "the public azure_openai card's deployment `{public_deployment}` routed a \
                 Versa request for gpt-4.1-2025-04-14"
            );
            assert_eq!(answered, "gpt-4.1-2025-04-14");
            // The route a reopened chat reuses is the model's own too, so the
            // other card's value cannot be persisted into the session row.
            assert_eq!(
                serde_json::to_value(chat.restore_binding()).unwrap()["deployment"],
                "gpt-4.1-2025-04-14",
                "`{public_deployment}` was written into the restore binding"
            );

            // Nor may it rescue a model no deployment serves.
            let probe = aimed_at(
                bound(UNMAPPED, config("", public_deployment)).await,
                &server,
            );
            assert!(
                answered_by(&probe).await.is_err(),
                "`{public_deployment}` answered for a model no Versa deployment serves"
            );
        }
        assert_eq!(
            requested_paths(&server).await,
            vec![path_of("gpt-4.1-2025-04-14"); public_deployments.len()]
        );
    }

    /// The public card's other two keys. Wherever `VERSA_AZURE_ENDPOINT` was
    /// blank, its endpoint used to become Versa's, so a company's Azure
    /// resource received every Versa request — the transcript, with
    /// `VERSA_AZURE_API_KEY` in its `api-key` header — refused the key, and the
    /// instance turned Public. Its API version rode along on every request to
    /// the gateway.
    #[tokio::test]
    async fn the_public_azure_cards_endpoint_and_api_version_never_reach_versa() {
        let server = gateway().await;
        let mut public_card = config("", "my-gpt4o");
        public_card.insert(
            "AZURE_OPENAI_ENDPOINT".into(),
            "https://contoso.openai.azure.com".into(),
        );
        // Azure's GA version — and what Versa's own onboarding card wrote into
        // this key from 2026-05-30 to 2026-07-02, which is why it is this one.
        public_card.insert("AZURE_OPENAI_API_VERSION".into(), "2024-10-21".into());

        let chat = bound("gpt-4.1-2025-04-14", public_card).await;
        let binding = serde_json::to_value(chat.restore_binding()).unwrap();
        assert_eq!(
            (&binding["endpoint"], chat.tier(), &binding["api_version"]),
            (
                &serde_json::json!(VERSA_AZURE_ENDPOINT),
                ProviderTier::Private,
                &serde_json::json!(VERSA_AZURE_API_VERSION),
            ),
            "the public azure_openai card's endpoint or API version reached a Versa chat"
        );

        // And on the wire: the version the gateway was actually sent.
        answered_by(&aimed_at(chat, &server)).await.unwrap();
        assert_eq!(
            requested_api_versions(&server).await,
            vec![VERSA_AZURE_API_VERSION]
        );
    }

    /// Every row written before this change stores gpt-5.5's deployment,
    /// whatever its model — the QA run read exactly that off a rebound chat. A
    /// restore must re-derive the route from the model, not carry it forward.
    #[tokio::test]
    async fn a_row_written_before_this_change_posts_to_its_own_models_deployment() {
        let server = gateway().await;
        let rebound = aimed_at(
            restored("gpt-4.1-2025-04-14", "gpt-5.5-2026-04-24").await,
            &server,
        );
        assert_eq!(answered_by(&rebound).await.unwrap(), "gpt-4.1-2025-04-14");
        assert_eq!(
            serde_json::to_value(rebound.restore_binding()).unwrap()["deployment"],
            "gpt-4.1-2025-04-14",
            "the stale route was carried into the next binding"
        );

        let probe = aimed_at(restored(UNMAPPED, "gpt-5.5-2026-04-24").await, &server);
        assert!(
            answered_by(&probe).await.is_err(),
            "the QA probe's own row still answers from gpt-5.5"
        );
        assert_eq!(
            requested_paths(&server).await,
            vec![path_of("gpt-4.1-2025-04-14")]
        );
    }

    /// The escape hatch survives: a deployment the catalog does not know serves
    /// every request, whatever the model names, and it survives a restore.
    #[tokio::test]
    async fn an_explicit_override_still_wins_for_every_model() {
        let server = gateway().await;
        let custom = "ucsf-preview-deployment";

        let live = aimed_at(
            bound("gpt-4.1-2025-04-14", config(custom, "")).await,
            &server,
        );
        assert_eq!(answered_by(&live).await.unwrap(), custom);
        // Versa's own key is the one way to set it, and the public card's key
        // naming some other deployment does not compete with it.
        let beside_public = aimed_at(
            bound("gpt-4.1-2025-04-14", config(custom, "my-gpt4o")).await,
            &server,
        );
        assert_eq!(answered_by(&beside_public).await.unwrap(), custom);
        // An override is the operator saying where requests go, so it serves a
        // model the catalog does not list as well.
        let unlisted = aimed_at(bound(UNMAPPED, config(custom, "")).await, &server);
        assert_eq!(answered_by(&unlisted).await.unwrap(), custom);

        let binding = serde_json::to_value(live.restore_binding()).unwrap();
        assert_eq!(binding["deployment"], custom);
        let restored = aimed_at(restored("gpt-4.1-2025-04-14", custom).await, &server);
        assert_eq!(answered_by(&restored).await.unwrap(), custom);
        assert_eq!(requested_paths(&server).await, vec![path_of(custom); 4]);
    }

    /// The advertised catalog is exactly the measured one, and nothing outside
    /// it can be chosen.
    #[test]
    fn the_advertised_catalog_is_exactly_the_measured_deployments() {
        let metadata = VersaAzureProvider::metadata();
        let advertised: Vec<&str> = metadata
            .known_models
            .iter()
            .map(|model| model.name.as_str())
            .collect();
        let measured: Vec<&str> = MEASURED.iter().map(|(model, _)| *model).collect();
        assert_eq!(
            advertised, measured,
            "the advertised catalog changed; re-probe the gateway and update MEASURED"
        );
        assert_eq!(metadata.default_model, "gpt-5.5-2026-04-24");
        assert!(measured.contains(&metadata.default_model.as_str()));
        assert!(
            !metadata.allows_unlisted_models,
            "\"Enter a model not listed...\" would offer a model that can only be refused"
        );
        for model in measured {
            assert!(
                ModelConfig::has_declared_context_window(model),
                "{model} has no MODEL_CONTEXT_WINDOWS entry of its own"
            );
        }
    }
}
