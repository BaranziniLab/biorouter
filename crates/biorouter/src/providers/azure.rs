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
use super::formats::openai::{
    create_request, get_usage, model_uses_responses_api, response_to_message,
};
use super::formats::openai_responses::{
    create_responses_request, get_responses_usage, responses_api_to_message, stream_responses_api,
    ResponsesApiResponse,
};
use super::retry::ProviderRetry;
use super::utils::{
    azure_chat_completions_path, get_model, handle_response_openai_compat,
    handle_status_openai_compat, stream_openai_compat, ImageFormat,
};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::providers::utils::RequestLog;
use rmcp::model::Tool;

// GPT-6 Sol is the default (2026-09-25). It has been GA on Azure Global
// Standard since 2026-09-22, costs less than gpt-5.4 ($2 in / $10 out per MTok
// against $2.50 / $15) with the same 1,050,000-token window, and is the
// like-for-like successor of the GPT-5.x default tier. Like gpt-5.4 and 5.5
// before it, the Azure models page notes that "some quota tiers require quota
// requests for the GPT-6 family", so a low-tier subscription may need one.
// It posts to the Responses route below, never to Chat Completions.
pub const AZURE_DEFAULT_MODEL: &str = "gpt-6-sol-2026-09-22";
pub const AZURE_DOC_URL: &str =
    "https://learn.microsoft.com/en-us/azure/foundry/foundry-models/concepts/models-sold-directly-by-azure";
// Use 2025-01-01-preview to support o-series alongside GPT models. Applies to
// the Chat Completions route only: the v1 Responses route takes no api-version.
pub const AZURE_DEFAULT_API_VERSION: &str = "2025-01-01-preview";

/// Azure OpenAI's v1 Responses route, relative to the resource endpoint.
///
/// Microsoft Learn, "Azure OpenAI Responses API" (ms.date 2026-08-18,
/// learn.microsoft.com/en-us/azure/foundry/openai/how-to/responses) and the v1
/// REST reference (learn.microsoft.com/en-us/azure/foundry/openai/latest):
///
///   * `POST {endpoint}/openai/v1/responses`, with no `api-version` — the v1
///     GA surface dropped the dated parameter ("`api-version` is no longer a
///     required parameter with the v1 GA API", api-version-lifecycle).
///   * `model` is the DEPLOYMENT name, not the model name ("Replace with your
///     model deployment name").
///   * Auth is the `api-key` header or an Entra bearer token; the REST
///     reference lists `https://cognitiveservices.azure.com/.default` as the
///     OAuth2 scope, which is the token `AzureAuth` already fetches. So both of
///     this provider's credential modes reach it unchanged.
///
/// The dated form (`openai/responses?api-version=2025-04-01-preview`) is not
/// used: measured 2026-09-25 against the UCSF Versa gateway (an Azure OpenAI
/// resource), the v1 route answered 200 for deployment `gpt-5.5-2026-04-24`
/// while the dated one answered 404 "Resource not found". The same day, this
/// provider itself — built by `from_env` with an API key, bound to that
/// deployment — got a function call back through this route with reasoning
/// effort set, both blocking and streamed; that is the combination Chat
/// Completions refuses for GPT-5.6 and GPT-6. No GPT-6 deployment was
/// reachable to repeat it on (the gateway serves none yet).
const AZURE_RESPONSES_PATH: &str = "openai/v1/responses";

// Verified against the Azure Foundry models page (ms.date 2026-09-21) and the
// model retirement schedule (updated 2026-09-23). Every id is the model NAME
// plus its Azure model VERSION; the deployment a request posts to is
// configured separately (AZURE_OPENAI_DEPLOYMENT_NAME).
//
// Removed because Azure moved them to Deprecated (no new deployments; existing
// ones keep working until retirement):
//   * o1-2024-12-17 and o3-mini-2025-01-31 — retire 2026-11-19.
//   * o3-2025-04-16 and o4-mini-2025-04-16 — retire 2026-11-19. Standard and
//     Global Standard deployments auto-upgrade at retirement (o3 to
//     gpt-5.6-sol, o4-mini to gpt-5.6-terra). A chat still configured as
//     o4-mini keeps working after that, because o4-mini already takes the
//     Responses route the gpt-5.6 replacement needs. A chat configured as o3
//     does NOT (nor one configured as o1 or o3-mini, which are upgraded the
//     same day): `model_uses_responses_api` matches only `o3-pro`, so it stays
//     on Chat Completions, where gpt-5.6 refuses function tools combined with
//     reasoning. Such a chat has to be switched to the gpt-5.6 id.
//   * gpt-4.1-2025-04-14 and gpt-4.1-mini-2025-04-14 — retire 2027-04-14.
//
// Not offered: gpt-6-terra, which does not exist (Terra exists only as
// gpt-5.6-terra), and the Responses-only codex and -pro models.
pub const AZURE_OPENAI_KNOWN_MODELS: &[&str] = &[
    // GPT-6 [responses] — Sol and Luna version 2026-09-22, Astra version
    // 2026-09-03 (not the 2026-09-04 aggregators give; Azure deploys by
    // version). All three: 1,050,000-token window (922,000 in / 128,000 out),
    // text + image input. OpenAI: GPT-6 calls tools only through Responses.
    "gpt-6-sol-2026-09-22",
    "gpt-6-astra-2026-09-03",
    "gpt-6-luna-2026-09-22",
    // GPT-5.6 [responses] — version 2026-07-09, 1,050,000 tokens. Azure's
    // function-calling page: these "can't combine function tools with
    // reasoning" on Chat Completions, so a tool-bearing turn there is refused
    // and the fix the error names is /v1/responses.
    "gpt-5.6-sol-2026-07-09",
    "gpt-5.6-terra-2026-07-09",
    "gpt-5.6-luna-2026-07-09",
    // GPT-5.5 [responses] (may need a quota request on lower tiers)
    "gpt-5.5-2026-04-24",
    // GPT-5.4 family [responses] (gpt-5.4 GA, retires 2027-09-02)
    "gpt-5.4-2026-03-05",
    "gpt-5.4-mini-2026-03-17",
    "gpt-5.4-nano-2026-03-17",
    // GPT-5.x previous generation (still GA; Chat Completions)
    "gpt-5.2-2025-12-11",
    "gpt-5.1-2025-11-13",
    "gpt-5-2025-08-07",
    // GPT-4o (Legacy — still deployable; retires 2027-04-14, replacement
    // gpt-5.1). Legacy is not Deprecated, so it stays listed.
    "gpt-4o-2024-11-20",
];

fn azure_model_supports_vision(name: &str) -> bool {
    !name.contains("codex")
        && (name.starts_with("gpt-6")
            || name.starts_with("gpt-5")
            || name.starts_with("gpt-4.1")
            || name.starts_with("gpt-4o")
            || name.starts_with("o3")
            || name.starts_with("o4"))
}

#[derive(Debug)]
pub struct AzureProvider {
    api_client: ApiClient,
    deployment_name: String,
    api_version: String,
    model: ModelConfig,
    name: String,
}

impl Serialize for AzureProvider {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AzureProvider", 2)?;
        state.serialize_field("deployment_name", &self.deployment_name)?;
        state.serialize_field("api_version", &self.api_version)?;
        state.end()
    }
}

// Custom auth provider that wraps AzureAuth
struct AzureAuthProvider {
    auth: AzureAuth,
}

#[async_trait]
impl AuthProvider for AzureAuthProvider {
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

impl AzureProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();
        let endpoint: String = config.get_param("AZURE_OPENAI_ENDPOINT")?;
        let deployment_name: String = config.get_param("AZURE_OPENAI_DEPLOYMENT_NAME")?;
        let api_version: String = config
            .get_param("AZURE_OPENAI_API_VERSION")
            .unwrap_or_else(|_| AZURE_DEFAULT_API_VERSION.to_string());

        let api_key = config
            .get_secret("AZURE_OPENAI_API_KEY")
            .ok()
            .filter(|key: &String| !key.is_empty());
        let auth = AzureAuth::new(api_key).map_err(|e| match e {
            AuthError::Credentials(msg) => anyhow::anyhow!("Credentials error: {}", msg),
            AuthError::TokenExchange(msg) => anyhow::anyhow!("Token exchange error: {}", msg),
        })?;

        let auth_provider = AzureAuthProvider { auth };
        let api_client = ApiClient::new(endpoint, AuthMethod::Custom(Box::new(auth_provider)))?;

        Ok(Self {
            api_client,
            deployment_name,
            api_version,
            model,
            name: Self::metadata().name,
        })
    }

    /// Whether a request for `model_name` takes the Responses route.
    ///
    /// The same predicate the OpenAI provider routes by, decided by the MODEL
    /// the request names — the configured deployment is an opaque label and
    /// says nothing about what it serves. Request shaping (reasoning effort,
    /// no temperature) has always been decided by that name too.
    ///
    /// Until 2026-09-25 this provider posted every model to Chat Completions.
    /// Sharing the predicate moved gpt-5.4 and gpt-5.5 (and a stored o4-mini
    /// chat) onto Responses along with GPT-6 and GPT-5.6. Measured at the UCSF
    /// gateway on 2026-09-25: gpt-5.5-2026-04-24 (through this provider),
    /// and gpt-5.4-mini-2026-03-17, gpt-5.4-nano-2026-03-17 and
    /// o4-mini-2025-04-16 (a direct POST to [`AZURE_RESPONSES_PATH`]) each
    /// returned a function call with a function tool and reasoning effort set.
    /// The same probe showed why the Responses builder drops `temperature`:
    /// gpt-5.5 and gpt-5.4-mini answered 400 "Unsupported parameter:
    /// 'temperature'" on this route.
    fn uses_responses_api(model_name: &str) -> bool {
        model_uses_responses_api(model_name)
    }

    /// The single source of truth for the Azure deployment path, shared by the
    /// blocking and streaming paths so they cannot drift.
    fn chat_completions_path(&self) -> String {
        // Shared with versa_azure via providers::utils so the two cannot drift
        // — a change made in one file used to be silently missable in the other.
        azure_chat_completions_path(&self.deployment_name, &self.api_version)
    }

    /// A Responses request body for `model_config`, addressed to this provider's
    /// deployment.
    ///
    /// ⚠ `model` is overwritten with the DEPLOYMENT name. The v1 route has no
    /// deployment in its path, so this field is the only thing that tells Azure
    /// which deployment to run; left as the model name, every request whose
    /// deployment is not named exactly after its model version would 404.
    fn build_responses_payload(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
        for_streaming: bool,
    ) -> Result<Value, ProviderError> {
        let mut payload = create_responses_request(model_config, system, messages, tools)?;
        payload["model"] = Value::String(self.deployment_name.clone());
        if for_streaming {
            // Responses reports usage on its terminal `response.completed`
            // event unasked; there is no `stream_options` to set.
            payload["stream"] = Value::Bool(true);
        }
        Ok(payload)
    }

    async fn post(&self, path: &str, payload: &Value) -> Result<Value, ProviderError> {
        let response = self.api_client.response_post(path, payload).await?;
        handle_response_openai_compat(response).await
    }

    /// The exact path and request body `stream()` posts. See the equivalent on
    /// `VersaAzureProvider` for why this is extracted rather than inlined.
    fn build_stream_request(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(String, Value), ProviderError> {
        if Self::uses_responses_api(&self.model.model_name) {
            let payload =
                self.build_responses_payload(&self.model, system, messages, tools, true)?;
            return Ok((AZURE_RESPONSES_PATH.to_string(), payload));
        }
        // `for_streaming = true` also sets stream_options.include_usage, which
        // is what makes Azure emit a final usage-bearing chunk.
        let payload = create_request(
            &self.model,
            system,
            messages,
            tools,
            &ImageFormat::OpenAi,
            true,
        )?;
        Ok((self.chat_completions_path(), payload))
    }
}

#[async_trait]
impl Provider for AzureProvider {
    fn metadata() -> ProviderMetadata {
        let models = AZURE_OPENAI_KNOWN_MODELS
            .iter()
            .map(|&name| {
                let info = ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit());
                if azure_model_supports_vision(name) {
                    info.with_vision()
                } else {
                    info
                }
            })
            .collect();

        ProviderMetadata::with_models(
            "azure_openai",
            "Azure OpenAI",
            "Models through Azure OpenAI Service (uses Azure credential chain by default).",
            AZURE_DEFAULT_MODEL,
            models,
            AZURE_DOC_URL,
            vec![
                ConfigKey::new(
                    "AZURE_OPENAI_ENDPOINT",
                    true,
                    false,
                    Some("https://unified-api.ucsf.edu/general"),
                ),
                ConfigKey::new("AZURE_OPENAI_DEPLOYMENT_NAME", true, false, None),
                ConfigKey::new(
                    "AZURE_OPENAI_API_VERSION",
                    true,
                    false,
                    Some(AZURE_DEFAULT_API_VERSION),
                ),
                ConfigKey::new("AZURE_OPENAI_API_KEY", false, true, Some("")),
            ],
        )
        .with_unlisted_models()
    }

    fn get_name(&self) -> &str {
        &self.name
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
        if Self::uses_responses_api(&model_config.model_name) {
            let payload =
                self.build_responses_payload(model_config, system, messages, tools, false)?;
            let mut log = RequestLog::start(model_config, &payload)?;
            let response = self
                .with_retry(|| async {
                    let payload_clone = payload.clone();
                    self.post(AZURE_RESPONSES_PATH, &payload_clone).await
                })
                .await
                .inspect_err(|e| {
                    let _ = log.error(e);
                })?;

            let responses_api_response: ResponsesApiResponse =
                serde_json::from_value(response.clone()).map_err(|e| {
                    ProviderError::ExecutionError(format!(
                        "Failed to parse responses API response: {}",
                        e
                    ))
                })?;
            let message = responses_api_to_message(&responses_api_response)?;
            let usage = get_responses_usage(&responses_api_response);
            let response_model = responses_api_response.model.clone();
            log.write(&response, Some(&usage))?;
            return Ok((message, ProviderUsage::new(response_model, usage)));
        }

        let payload = create_request(
            model_config,
            system,
            messages,
            tools,
            &ImageFormat::OpenAi,
            false,
        )?;
        let mut log = RequestLog::start(model_config, &payload)?;
        let path = self.chat_completions_path();
        let response = self
            .with_retry(|| async {
                let payload_clone = payload.clone();
                self.post(&path, &payload_clone).await
            })
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
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

    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let (path, payload) = self.build_stream_request(system, messages, tools)?;
        let mut log = RequestLog::start(&self.model, &payload)?;

        let response = self
            .with_retry(|| async {
                let resp = self.api_client.response_post(&path, &payload).await?;
                handle_status_openai_compat(resp).await
            })
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        // The decoder must match the route: a Responses stream is typed
        // `response.*` events, not Chat Completions chunks.
        if Self::uses_responses_api(&self.model.model_name) {
            return Ok(stream_responses_api(response, log));
        }
        stream_openai_compat(response, log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// azure and versa_azure received the identical streaming change, but only
    /// versa_azure was tested. These assertions exist so the Azure path
    /// convention is pinned in both files: `supports_streaming()` is hardcoded
    /// true here, so a wrong path 404s every streaming turn with no fallback to
    /// `complete()` — the provider is broken outright, not degraded.
    fn test_provider() -> AzureProvider {
        provider_for(
            "https://example-resource.openai.azure.com",
            AZURE_DEFAULT_MODEL,
            AZURE_DEFAULT_MODEL,
        )
    }

    /// A provider bound to `model`, posting to `deployment` on `host`, wired as
    /// `from_env` wires one minus the global config lookup.
    fn provider_for(host: &str, model: &str, deployment: &str) -> AzureProvider {
        let api_client = ApiClient::new(
            host.to_string(),
            AuthMethod::ApiKey {
                header_name: "api-key".to_string(),
                key: "test-key".to_string(),
            },
        )
        .expect("api client builds");

        AzureProvider {
            api_client,
            deployment_name: deployment.to_string(),
            api_version: AZURE_DEFAULT_API_VERSION.to_string(),
            model: ModelConfig::new_or_fail(model),
            name: "azure_openai".to_string(),
        }
    }

    #[test]
    fn chat_completions_path_is_azure_deployment_shaped() {
        let provider = provider_for(
            "https://example-resource.openai.azure.com",
            "gpt-4o-2024-11-20",
            "gpt-4o-2024-11-20",
        );
        let path = provider.chat_completions_path();

        assert_eq!(
            path,
            "openai/deployments/gpt-4o-2024-11-20/chat/completions?api-version=2025-01-01-preview"
        );
        assert!(
            !path.starts_with("chat/completions"),
            "azure must not post to the plain OpenAI path"
        );
    }

    #[test]
    fn chat_completions_stream_payload_opts_into_streaming_with_usage() {
        let provider = provider_for(
            "https://example-resource.openai.azure.com",
            "gpt-5.2-2025-12-11",
            "my-gpt-5-2",
        );
        let (path, payload) = provider
            .build_stream_request("sys", &[], &[])
            .expect("streaming payload builds");

        assert_eq!(
            path,
            "openai/deployments/my-gpt-5-2/chat/completions?api-version=2025-01-01-preview"
        );
        assert_eq!(payload["stream"], serde_json::json!(true));
        assert_eq!(
            payload["stream_options"]["include_usage"],
            serde_json::json!(true),
            "Azure needs stream_options.include_usage or usage/cost tracking breaks"
        );
    }

    /// The default streams through Responses, addressed to its deployment.
    #[test]
    fn default_model_streams_through_the_v1_responses_route() {
        let provider = test_provider();
        let (path, payload) = provider
            .build_stream_request("sys", &[], &[])
            .expect("streaming payload builds");

        assert_eq!(path, "openai/v1/responses");
        assert_eq!(payload["stream"], serde_json::json!(true));
        assert_eq!(payload["model"], serde_json::json!(AZURE_DEFAULT_MODEL));
        assert!(payload.get("messages").is_none(), "a Chat Completions body");
        assert!(
            payload.get("stream_options").is_none(),
            "Responses has no stream_options; usage arrives on response.completed"
        );
    }

    #[test]
    fn provider_advertises_streaming() {
        assert!(
            test_provider().supports_streaming(),
            "azure must advertise streaming; without it the agent takes the blocking \
             complete() path and tool cards only appear at end of generation"
        );
    }

    /// Which catalog models take which route. GPT-6 and GPT-5.6 are the
    /// load-bearing rows: on Chat Completions neither can combine function
    /// tools with reasoning, so every tool-bearing turn would be refused.
    #[test]
    fn each_catalog_model_takes_the_route_it_needs() {
        let responses = [
            "gpt-6-sol-2026-09-22",
            "gpt-6-astra-2026-09-03",
            "gpt-6-luna-2026-09-22",
            "gpt-5.6-sol-2026-07-09",
            "gpt-5.6-terra-2026-07-09",
            "gpt-5.6-luna-2026-07-09",
            "gpt-5.5-2026-04-24",
            "gpt-5.4-2026-03-05",
            "gpt-5.4-mini-2026-03-17",
            "gpt-5.4-nano-2026-03-17",
        ];
        let chat_completions = [
            "gpt-5.2-2025-12-11",
            "gpt-5.1-2025-11-13",
            "gpt-5-2025-08-07",
            "gpt-4o-2024-11-20",
        ];
        for id in AZURE_OPENAI_KNOWN_MODELS {
            assert!(
                responses.contains(id) || chat_completions.contains(id),
                "{id} is in the catalog but its route is not pinned here"
            );
        }
        for id in responses {
            assert!(AZURE_OPENAI_KNOWN_MODELS.contains(&id), "{id}");
            assert!(
                AzureProvider::uses_responses_api(id),
                "{id} needs Responses"
            );
        }
        for id in chat_completions {
            assert!(AZURE_OPENAI_KNOWN_MODELS.contains(&id), "{id}");
            assert!(
                !AzureProvider::uses_responses_api(id),
                "{id} stays on Chat Completions"
            );
        }
    }

    #[test]
    fn catalog_leads_with_the_default_and_drops_deprecated_models() {
        assert_eq!(AZURE_DEFAULT_MODEL, "gpt-6-sol-2026-09-22");
        assert_eq!(
            AZURE_OPENAI_KNOWN_MODELS.first(),
            Some(&AZURE_DEFAULT_MODEL)
        );
        // Azure lifecycle "Deprecated": no new deployments.
        for deprecated in [
            "o1-2024-12-17",
            "o3-mini-2025-01-31",
            "o3-2025-04-16",
            "o4-mini-2025-04-16",
            "gpt-4.1-2025-04-14",
            "gpt-4.1-mini-2025-04-14",
        ] {
            assert!(
                !AZURE_OPENAI_KNOWN_MODELS.contains(&deprecated),
                "{deprecated} is Deprecated on Azure and must not be offered"
            );
        }
        assert!(
            !AZURE_OPENAI_KNOWN_MODELS
                .iter()
                .any(|id| id.starts_with("gpt-6-terra")),
            "gpt-6-terra does not exist"
        );
        // A chat bound before the removal can still name them.
        assert!(AzureProvider::metadata().allows_unlisted_models);
    }

    /// The route a chat configured with a removed o-series model takes once
    /// Azure auto-upgrades its deployment to GPT-5.6 (2026-11-19), as the
    /// removal note on `AZURE_OPENAI_KNOWN_MODELS` describes: o4-mini already
    /// takes Responses, while o3, o1 and o3-mini stay on Chat Completions,
    /// where GPT-5.6 refuses tools combined with reasoning. If this changes,
    /// that note changes with it.
    #[test]
    fn removed_o_series_models_take_the_route_the_removal_note_describes() {
        assert!(AzureProvider::uses_responses_api("o4-mini-2025-04-16"));
        for id in ["o3-2025-04-16", "o1-2024-12-17", "o3-mini-2025-01-31"] {
            assert!(
                !AzureProvider::uses_responses_api(id),
                "{id}: the removal note says this stays on Chat Completions"
            );
        }
    }

    #[test]
    fn gpt_6_and_gpt_5_6_advertise_vision_and_their_full_window() {
        // metadata() sizes each model through ModelConfig::context_limit(),
        // which honours BIOROUTER_CONTEXT_LIMIT; other tests in this binary set
        // it process-wide under env_lock, so a window assertion must hold it.
        let _guard = env_lock::lock_env([
            ("BIOROUTER_CONTEXT_LIMIT", None::<&str>),
            ("BIOROUTER_PREDEFINED_MODELS", None::<&str>),
        ]);
        let meta = AzureProvider::metadata();
        for id in [
            "gpt-6-sol-2026-09-22",
            "gpt-6-astra-2026-09-03",
            "gpt-6-luna-2026-09-22",
            "gpt-5.6-sol-2026-07-09",
            "gpt-5.6-terra-2026-07-09",
            "gpt-5.6-luna-2026-07-09",
        ] {
            let model = meta
                .known_models
                .iter()
                .find(|m| m.name == id)
                .unwrap_or_else(|| panic!("{id} missing from provider metadata"));
            assert_eq!(model.supports_vision, Some(true), "{id} takes image input");
            assert_eq!(model.context_limit, 1_050_000, "{id}");
        }
    }

    /// Everything a real request carries, measured at a local stand-in for the
    /// Azure resource.
    mod routes {
        use super::*;
        use crate::agents::effort::ReasoningEffort;
        use futures::TryStreamExt;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, Request, ResponseTemplate};

        fn tool() -> Tool {
            Tool::new(
                "lookup_record",
                "Look up a record",
                serde_json::json!({
                    "type": "object",
                    "properties": { "id": { "type": "string" } },
                    "required": ["id"]
                })
                .as_object()
                .unwrap()
                .clone(),
            )
        }

        fn prompt() -> Vec<Message> {
            vec![Message::user().with_text("Reply with the single word ready.")]
        }

        /// Answers a Responses body as Responses and anything else as Chat
        /// Completions, echoing the body's `model`. Whether real Azure echoes
        /// the deployment name or the underlying model version here is NOT
        /// established: every UCSF gateway deployment is named after its model
        /// version, so the 2026-09-25 probe could not tell the two apart. The
        /// echo is a stand-in, not a measured contract.
        async fn resource() -> MockServer {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(|request: &Request| {
                    let body: Value = serde_json::from_slice(&request.body).unwrap();
                    let model = body["model"].clone();
                    if request.url.path().ends_with("/responses") {
                        if body["stream"] == true {
                            let frames = [
                                serde_json::json!({
                                    "type": "response.output_text.delta",
                                    "sequence_number": 1,
                                    "item_id": "msg_1",
                                    "output_index": 0,
                                    "content_index": 0,
                                    "delta": "ready"
                                }),
                                serde_json::json!({
                                    "type": "response.completed",
                                    "sequence_number": 2,
                                    "response": {
                                        "id": "resp_azure",
                                        "object": "response",
                                        "created_at": 1,
                                        "status": "completed",
                                        "model": model,
                                        "output": [{
                                            "type": "message",
                                            "id": "msg_1",
                                            "status": "completed",
                                            "role": "assistant",
                                            "content": [{"type": "output_text", "text": "ready"}]
                                        }],
                                        "usage": {"input_tokens": 7, "output_tokens": 2, "total_tokens": 9}
                                    }
                                }),
                            ];
                            let sse: String = frames
                                .iter()
                                .map(|frame| format!("data: {frame}\n\n"))
                                .collect();
                            return ResponseTemplate::new(200)
                                .insert_header("content-type", "text/event-stream")
                                .set_body_string(sse);
                        }
                        return ResponseTemplate::new(200).set_body_json(serde_json::json!({
                            "id": "resp_azure",
                            "object": "response",
                            "created_at": 1,
                            "status": "completed",
                            "model": model,
                            "output": [{
                                "type": "message",
                                "id": "msg_1",
                                "status": "completed",
                                "role": "assistant",
                                "content": [{"type": "output_text", "text": "ready"}]
                            }],
                            "usage": {"input_tokens": 7, "output_tokens": 2, "total_tokens": 9}
                        }));
                    }
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "id": "chatcmpl-azure",
                        "object": "chat.completion",
                        "created": 0,
                        "model": model,
                        "choices": [{
                            "index": 0,
                            "message": {"role": "assistant", "content": "ready"},
                            "finish_reason": "stop"
                        }],
                        "usage": {"prompt_tokens": 7, "completion_tokens": 2, "total_tokens": 9}
                    }))
                })
                .mount(&server)
                .await;
            server
        }

        async fn last_request(server: &MockServer) -> Request {
            server
                .received_requests()
                .await
                .unwrap_or_default()
                .pop()
                .expect("the stand-in saw a request")
        }

        fn body(request: &Request) -> Value {
            serde_json::from_slice(&request.body).unwrap()
        }

        fn text(message: &Message) -> String {
            message
                .content
                .iter()
                .filter_map(|content| content.as_text())
                .collect()
        }

        #[tokio::test]
        async fn a_responses_model_posts_to_v1_responses_with_its_deployment_as_model() {
            let server = resource().await;
            let mut provider =
                provider_for(&server.uri(), "gpt-6-sol-2026-09-22", "contoso-gpt-6-sol");
            provider.model = provider
                .model
                .clone()
                .with_temperature(Some(0.4))
                .with_max_tokens(Some(4096))
                .with_reasoning_effort(Some(ReasoningEffort::Deep));

            let (message, usage) = provider
                .complete("system", &prompt(), &[tool()])
                .await
                .expect("the Responses route answers");
            assert_eq!(text(&message), "ready");
            assert_eq!(usage.model, "contoso-gpt-6-sol");
            assert_eq!(usage.usage.total_tokens, Some(9));

            let request = last_request(&server).await;
            assert_eq!(request.url.path(), "/openai/v1/responses");
            assert_eq!(
                request.url.query(),
                None,
                "the v1 route takes no api-version"
            );
            assert_eq!(
                request
                    .headers
                    .get("api-key")
                    .and_then(|value| value.to_str().ok()),
                Some("test-key")
            );
            let sent = body(&request);
            assert_eq!(
                sent["model"], "contoso-gpt-6-sol",
                "the v1 route names the deployment in the body, not the model"
            );
            assert!(sent.get("messages").is_none());
            assert!(sent["input"].is_array());
            assert_eq!(sent["tools"][0]["type"], "function");
            assert_eq!(sent["tools"][0]["name"], "lookup_record");
            assert_eq!(sent["reasoning"]["effort"], "high");
            assert_eq!(sent["max_output_tokens"], 4096);
            assert!(
                sent.get("temperature").is_none(),
                "GPT-6 refuses temperature"
            );
            assert!(sent.get("reasoning_effort").is_none());
            assert!(sent.get("stream").is_none());
        }

        #[tokio::test]
        async fn a_responses_model_streams_from_v1_responses() {
            let server = resource().await;
            let provider = provider_for(&server.uri(), "gpt-5.6-terra-2026-07-09", "terra");

            let items = provider
                .stream("system", &prompt(), &[tool()])
                .await
                .expect("the Responses route streams")
                .try_collect::<Vec<_>>()
                .await
                .expect("the stream decodes");
            let streamed: String = items
                .iter()
                .filter_map(|(message, _, _)| message.as_ref())
                .map(text)
                .collect();
            assert!(streamed.contains("ready"), "streamed {streamed:?}");
            let usage = items
                .iter()
                .find_map(|(_, usage, _)| usage.as_ref())
                .expect("response.completed carries the usage");
            assert_eq!(usage.usage.total_tokens, Some(9));

            let request = last_request(&server).await;
            assert_eq!(request.url.path(), "/openai/v1/responses");
            assert_eq!(request.url.query(), None);
            let sent = body(&request);
            assert_eq!(sent["model"], "terra");
            assert_eq!(sent["stream"], true);
            assert_eq!(sent["tools"][0]["name"], "lookup_record");
        }

        /// The route follows the model a REQUEST names, so a fast model on the
        /// other route than the chat's is not sent down the chat's.
        #[tokio::test]
        async fn the_route_follows_the_model_the_request_names() {
            let server = resource().await;
            let provider = provider_for(&server.uri(), "gpt-6-sol-2026-09-22", "shared");

            provider
                .complete_with_model(
                    &ModelConfig::new_or_fail("gpt-4o-2024-11-20"),
                    "system",
                    &prompt(),
                    &[],
                )
                .await
                .expect("the Chat Completions route answers");
            let request = last_request(&server).await;
            assert_eq!(
                request.url.path(),
                "/openai/deployments/shared/chat/completions"
            );
            assert!(body(&request)["messages"].is_array());
        }

        #[tokio::test]
        async fn a_chat_completions_model_keeps_its_deployment_path_and_api_version() {
            let server = resource().await;
            let provider = provider_for(&server.uri(), "gpt-5.2-2025-12-11", "my-gpt-5-2");

            let (message, usage) = provider
                .complete("system", &prompt(), &[tool()])
                .await
                .expect("the Chat Completions route answers");
            assert_eq!(text(&message), "ready");
            assert_eq!(usage.model, "gpt-5.2-2025-12-11");

            let request = last_request(&server).await;
            assert_eq!(
                request.url.path(),
                "/openai/deployments/my-gpt-5-2/chat/completions"
            );
            assert_eq!(request.url.query(), Some("api-version=2025-01-01-preview"));
            let sent = body(&request);
            assert!(sent["messages"].is_array());
            assert!(sent.get("input").is_none());
            assert_eq!(sent["tools"][0]["function"]["name"], "lookup_record");

            drop(
                provider
                    .stream("system", &prompt(), &[])
                    .await
                    .expect("the Chat Completions route streams"),
            );
            let request = last_request(&server).await;
            assert_eq!(
                request.url.path(),
                "/openai/deployments/my-gpt-5-2/chat/completions"
            );
            assert_eq!(body(&request)["stream_options"]["include_usage"], true);
        }
    }
}
