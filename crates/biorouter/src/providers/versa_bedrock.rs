use super::base::{ConfigKey, ModelInfo, Provider, ProviderMetadata, ProviderUsage};
use super::errors::ProviderError;
use super::retry::{ProviderRetry, RetryConfig};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::privacy::ProviderTier;
use crate::providers::utils::RequestLog;
use anyhow::Result;
use async_trait::async_trait;
use aws_sdk_bedrockruntime::config::{Credentials, ProvideCredentials};
use aws_sdk_bedrockruntime::{types as bedrock, Client};
use rmcp::model::Tool;

use aws_sdk_bedrockruntime::operation::converse_stream::ConverseStreamOutput as ConverseStreamResponse;

use super::base::MessageStream;
use super::formats::bedrock::{
    bedrock_blocking_inference_config, bedrock_inference_config, bedrock_message_stream,
    classify_bedrock_converse_error, classify_bedrock_converse_stream_error, from_bedrock_message,
    from_bedrock_usage, gateway_message, map_bedrock_stop_reason, to_bedrock_messages,
    to_bedrock_tool_config,
};

/// W2-PRV-9 — a key pair the Versa gateway refuses, said as that: whose
/// refusal it is, the gateway's own reason, and where the pair is replaced.
///
/// The gateway answers a bad pair with a code-less 403 (`{"message": "Invalid
/// Client Id"}`), and a real pair that does not sign with a different sentence,
/// so the reason is what tells the two apart. It used to read "Bedrock endpoint
/// returned HTTP 403 (unauthorized) ... no further detail was returned" for
/// both, and named nowhere to fix it.
///
/// T3-SH-8: the place named is where the row really is. "Settings > Models >
/// Versa API Bedrock" named a page that does not exist; the row is in the
/// provider catalog, which Settings > Models opens with Configure providers.
///
/// A 403 that refuses the MODEL is not about the pair, and is said as that.
/// AWS answers a working pair asking for a model UCSF's account may not invoke
/// with a typed `AccessDeniedException`, which classifies as `Authentication`
/// like a bad pair does. Measured 2026-10-05 for Opus 5.5, Opus 5, Sonnet 5.5,
/// Opus 4.7 and Fable 5.1 ("User: arn:aws:iam::…:user/managed-service-account-
/// mulesoft-ai is not authorized to perform: bedrock:InvokeModel on resource:
/// …inference-profile/us.anthropic.…"). Read as a rejected pair, a chat still
/// bound to Opus 5.5 after it left the list told the user to replace a key
/// pair that works. It is a `RequestFailed` whose words classify as
/// `ModelUnavailable`, the same shape `versa_azure` gives a model with no
/// deployment, so the turn stops on the first try and names the models that do
/// answer.
///
/// ⚠ The same refusal for a model ON the list is a different sentence. Saying
/// "does not serve `X` ... Available: X, ..." would contradict itself, and the
/// cause is then UCSF's account losing a permission it had (on 2026-09-25 every
/// id, the default included, was refused for a while), which switching models
/// may not fix. So it says the model is normally served, offers only the
/// others, and names UCSF's support address (versa@ucsf.edu, from UCSF's
/// 2026-10-01 announcement).
pub(crate) fn versa_refusal(
    error: ProviderError,
    said: Option<String>,
    model: &str,
) -> ProviderError {
    match error {
        ProviderError::Authentication(detail)
            if !said
                .as_deref()
                .is_some_and(super::names_a_rejected_credential)
                && (refuses_the_model(&detail)
                    || said.as_deref().is_some_and(refuses_the_model)) =>
        {
            let others = VERSA_BEDROCK_KNOWN_MODELS
                .iter()
                .filter(|offered| **offered != model)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            if VERSA_BEDROCK_KNOWN_MODELS.contains(&model) {
                ProviderError::RequestFailed(format!(
                    "Versa API Bedrock normally serves model `{model}`, but UCSF's gateway \
                     account was refused it just now (model not found among the models \
                     UCSF's account may use right now, so nothing ran). Your key pair works. \
                     Try another model ({others}); if the refusal persists, contact \
                     versa@ucsf.edu."
                ))
            } else {
                ProviderError::RequestFailed(format!(
                    "Versa API Bedrock does not serve model `{model}`: UCSF's gateway account \
                     is not authorized to invoke it (model not found among the models UCSF's \
                     account may use, so nothing ran). Your key pair works. Available: \
                     {others}. Choose one of those models."
                ))
            }
        }
        ProviderError::Authentication(_) => ProviderError::Authentication(format!(
            "Versa rejected this key pair ({}). Replace it in Settings > Models > Configure \
             providers > Institutional > Versa API Bedrock.",
            said.unwrap_or_else(|| "no reason was given".to_string())
        )),
        other => other,
    }
}

/// Whether a Bedrock refusal is about the model rather than the caller: AWS's
/// IAM sentence for an identity that may not invoke one model or inference
/// profile (`bedrock:InvokeModel` for Converse,
/// `bedrock:InvokeModelWithResponseStream` for ConverseStream), and its
/// sentence for a model the account has not been given access to.
fn refuses_the_model(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("not authorized to perform: bedrock:invoke")
        || lowered.contains("don't have access to the model")
}

/// T3-SH-3 — the model id [`VersaBedrockProvider::check_credentials`] asks for:
/// one that no gateway serves, so the probe can never run a model.
///
/// The UCSF gateway checks the key pair before it looks at the model. Measured
/// on 2026-09-28 with a made-up pair: `Invalid Client Id` (403) for this id, for
/// Haiku 4.5 and for Opus 4.8 alike, each in about a second. A pair it accepts
/// is forwarded, and AWS refuses the id as invalid without running anything,
/// so the check costs nothing and does not depend on which models the pair may
/// use.
pub(crate) const CREDENTIAL_PROBE_MODEL: &str = "biorouter-credential-check";
use super::provider_binding::{
    model_without_restore_marker, PersistedRetryConfig, ProviderRestoreBinding, SecretFreeEndpoint,
};

pub const VERSA_BEDROCK_DOC_LINK: &str = "http://biorouter.ucsf.edu/docs";
// Opus 4.8: verified end-to-end through the UCSF MuleSoft proxy (a converse
// round-trip on 2026-07-26, and again on 2026-10-05). Same price and window as
// the Opus 4.6 default it replaced (1M context, $5.50/$27.50 us geo), so a new
// user's first chat cannot get worse. Kept as the default when Sonnet 5 was
// verified on 2026-10-05: that was a decision to leave the default alone, not
// a claim that no newer model answers. It must also stay FIRST in the list
// below: the UI auto-selects `known_models[0]` when a user switches providers,
// not this constant (SwitchModelModal `findFirstAvailableModel`).
pub const VERSA_BEDROCK_DEFAULT_MODEL: &str = "us.anthropic.claude-opus-4-8";
// Model IDs follow the AWS Bedrock format documented at
// https://platform.claude.com/docs/en/about-claude/models/overview, prefixed
// with the `us.` cross-region inference profile required by the UCSF MuleSoft
// proxy. The proxy forwards ids verbatim — AWS's own ValidationException text
// comes back for a wrong spelling — so the id shape is whatever AWS publishes;
// the only open question for a new id is UCSF's entitlement.
//
// Every entry below answered a real Converse round-trip through the UCSF proxy
// on 2026-10-05, newest first within each generation. Users can type any other
// ID via the "Enter a model not listed..." option once UCSF enables it.
//
// Measured on 2026-10-05 and deliberately NOT listed:
//   * Refused by UCSF's IAM policy: AWS answered `AccessDeniedException`
//     ("User: arn:aws:iam::…:user/managed-service-account-mulesoft-ai is not
//     authorized to perform: bedrock:InvokeModel on resource:
//     …inference-profile/us.anthropic.…") for Opus 5.5
//     (`us.anthropic.claude-opus-5-5`), Opus 5 (`-opus-5`), Opus 4.7
//     (`-opus-4-7`), Fable 5.1 and Fable 5 (`-fable-5-1`, `-fable-5`), Sonnet
//     5.5 (`-sonnet-5-5`), Sonnet 4.5 (`-sonnet-4-5-20250929-v1:0`) and Sonnet
//     4 (`-sonnet-4-20250514-v1:0`), and for every `global.` and in-region
//     `anthropic.` profile. The key pair works; the account may not invoke
//     them. Opus 5.5 and Opus 5 were listed here, unverified, from 2026-09-25
//     until this measurement removed them. `versa_refusal` turns that 403
//     into a model refusal, not a rejected key pair, for a chat still bound
//     to one of them.
//   * Fable, every version, stays off this list even if a later probe finds
//     UCSF's account may invoke it: Bedrock serves Fable only to accounts
//     whose data-retention mode is `aws_review` (see `bedrock.rs`), which an
//     institutional PHI account should not use. That reason predates the IAM
//     refusal above and outlives it; a test pins the whole family out.
//   * Opus 4.1 (`-opus-4-1-20250805-v1:0`) answers, but Bedrock lists it as
//     Legacy (public extended access at a higher price from 2026-10-08, end of
//     life 2027-01-08; docs.aws.amazon.com/bedrock/latest/userguide/
//     model-lifecycle-legacy.html), so it is not offered: the policy this
//     list already applied to Sonnet 4 once Bedrock marked it Legacy.
//   * Opus 4 (`-opus-4-20250514-v1:0`) and every Claude 3.x id: AWS answered
//     `ResourceNotFoundException`, "reached the end of its life".
pub const VERSA_BEDROCK_KNOWN_MODELS: &[&str] = &[
    // Claude 4.8 (1M context). Added 2026-07 (issue #29). Verified live
    // through the MuleSoft proxy on 2026-07-26: the short un-suffixed form
    // below answered a real converse round-trip, while the `-v1` spelling
    // (which opus-4-6 uses) was rejected with "The provided model
    // identifier is invalid" — 4.8 and 4.6 genuinely differ in id shape on
    // this account. Still the default (see above).
    "us.anthropic.claude-opus-4-8",
    // Sonnet 5 (1M context, $2/$10). First answered through the proxy on
    // 2026-10-05. Adaptive-only, so `formats::bedrock` sends it no
    // temperature, and it takes the Claude 64K default output allowance.
    "us.anthropic.claude-sonnet-5",
    // Claude 4.6 (1M context)
    "us.anthropic.claude-opus-4-6-v1",
    "us.anthropic.claude-sonnet-4-6",
    // Claude 4.5 (200K context)
    "us.anthropic.claude-opus-4-5-20251101-v1:0",
    // Haiku 4.5 (200K context)
    "us.anthropic.claude-haiku-4-5-20251001-v1:0",
];

// UCSF MuleSoft Bedrock proxy. UCSF-issued access keys are signed against this
// endpoint instead of public AWS, so this must be set for Versa Bedrock to work.
pub const VERSA_BEDROCK_DEFAULT_ENDPOINT: &str = "https://unified-api.ucsf.edu/general/awsai";
pub const VERSA_BEDROCK_DEFAULT_REGION: &str = "us-west-2";

pub const VERSA_BEDROCK_DEFAULT_MAX_RETRIES: usize = 6;
pub const VERSA_BEDROCK_DEFAULT_INITIAL_RETRY_INTERVAL_MS: u64 = 2000;
pub const VERSA_BEDROCK_DEFAULT_BACKOFF_MULTIPLIER: f64 = 2.0;
pub const VERSA_BEDROCK_DEFAULT_MAX_RETRY_INTERVAL_MS: u64 = 120_000;

#[cfg(test)]
fn with_test_http_client(loader: aws_config::ConfigLoader) -> aws_config::ConfigLoader {
    let (http_client, _captured) = aws_smithy_http_client::test_util::capture_request(None);
    loader.http_client(http_client)
}

#[derive(Debug, serde::Serialize)]
pub struct VersaBedrockProvider {
    #[serde(skip)]
    client: Client,
    model: ModelConfig,
    #[serde(skip)]
    retry_config: RetryConfig,
    #[serde(skip)]
    name: String,
    /// The endpoint this instance resolved at construction. `tier()` reads it,
    /// never the provider's name — `VERSA_BEDROCK_ENDPOINT` is user-writable, so
    /// an instance can resolve somewhere that is not the UCSF gateway.
    #[serde(skip)]
    resolved_endpoint: String,
    #[serde(skip)]
    region: String,
    #[serde(skip)]
    operation_timeout_secs: Option<u64>,
}

impl VersaBedrockProvider {
    pub async fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();

        // Overrides come from this provider's OWN namespace and nowhere else. A
        // blank value is absent, and absent is the UCSF gateway, so a fresh
        // install with just the key and secret works out of the box.
        //
        // ⚠ Not `AWS_ENDPOINT_URL_BEDROCK` or `AWS_REGION`, and not the process
        // environment. The `AWS_*` namespace is the public `aws_bedrock` card's:
        // it declares `AWS_REGION`, and `bedrock.rs` exports every `AWS_*`
        // config value and secret into the environment. Sharing it went wrong in
        // both directions. Versa read those two keys, then
        // `AWS_ENDPOINT_URL_BEDROCK` and `AWS_ENDPOINT_URL_BEDROCK_RUNTIME` from
        // the environment, as fallbacks. So the public card's region, or an
        // endpoint left by its export or by a shell, steered Versa: UCSF-issued
        // keys signed requests for someone's own AWS region, which refused them,
        // and the instance turned Public. Versa's setup also WROTE both keys, so
        // connecting it marked the public card Configured, and handed it UCSF's
        // gateway as an endpoint. The fallbacks are gone (2026-09-11).
        //
        // Where nothing was set, every Versa setup surface prefilled the shipped
        // defaults, and neither default has changed: the region has been
        // us-west-2 since Versa Bedrock shipped (2026-05-07), and the endpoint
        // has been UCSF's gateway since it became configurable (2026-05-12). So a
        // value this drops was either typed over that prefill or came from the
        // public side, and the second is the bug itself.
        //
        // ⚠ Each key is a STRING LITERAL passed straight to `get_param`, as in
        // `versa_azure`, because `privacy::config_keys` scans this file for them.
        let endpoint_url: String = config
            .get_param::<String>("VERSA_BEDROCK_ENDPOINT")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| VERSA_BEDROCK_DEFAULT_ENDPOINT.to_string());

        let region: String = config
            .get_param::<String>("VERSA_BEDROCK_REGION")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| VERSA_BEDROCK_DEFAULT_REGION.to_string());

        let retry_config = Self::load_retry_config(config);
        let operation_timeout_secs = Self::load_operation_timeout_secs(config);

        Self::from_resolved(
            model,
            SecretFreeEndpoint::new(endpoint_url)?,
            region,
            PersistedRetryConfig {
                max_retries: retry_config.max_retries,
                initial_interval_ms: retry_config.initial_interval_ms,
                backoff_multiplier: retry_config.backoff_multiplier,
                max_interval_ms: retry_config.max_interval_ms,
            },
            operation_timeout_secs,
        )
        .await
    }

    pub(crate) async fn from_resolved(
        model: ModelConfig,
        endpoint: SecretFreeEndpoint,
        region: String,
        retry: PersistedRetryConfig,
        operation_timeout_secs: Option<u64>,
    ) -> Result<Self> {
        let binding = ProviderRestoreBinding::VersaBedrock {
            model: model.clone(),
            endpoint: endpoint.clone(),
            region: region.clone(),
            retry: retry.clone(),
            operation_timeout_secs,
        };
        binding.validate()?;

        let config = crate::config::Config::global();
        // ⚠ `map_err(|_| "... is not configured")` DISCARDS the reason, and the
        // two reasons need opposite responses from the user:
        //
        //   NotFound        -> "add it in Settings" is right
        //   anything else   -> the key IS there and the store refused the read,
        //                      so "add it in Settings" sends the user to retype
        //                      a credential that is already correct
        //
        // On macOS the second happens whenever the Keychain grant no longer
        // covers the running binary -- the grant is bound to the signature, so a
        // newly installed or re-signed build asks again, and an unanswered
        // prompt fails the read.
        let read_key = |name: &str| -> anyhow::Result<String> {
            match config.get_secret::<String>(name) {
                Ok(value) => Ok(value),
                Err(crate::config::ConfigError::NotFound(_)) => Err(anyhow::anyhow!(
                    "{name} {}. Add it under Versa API Bedrock in Settings.",
                    super::CREDENTIAL_NEVER_SET
                )),
                Err(error) => Err(anyhow::anyhow!(
                    "Could not read {name} from the credential store: {error}\n\n\
                     The credential appears to be configured, so this is the store \
                     refusing the read rather than a missing key -- do NOT re-enter it. \
                     On macOS a Keychain grant is tied to the application's signature, \
                     so a newly installed or re-signed build asks for permission again; \
                     answer that prompt with \u{201c}Always Allow\u{201d}."
                )),
            }
        };
        let access_key_id: String = read_key("VERSA_BEDROCK_ACCESS_KEY_ID")?;
        let secret_access_key: String = read_key("VERSA_BEDROCK_SECRET_ACCESS_KEY")?;
        anyhow::ensure!(
            !access_key_id.trim().is_empty() && !secret_access_key.trim().is_empty(),
            "Versa Bedrock access key id / secret access key is empty"
        );

        let credentials =
            Credentials::new(access_key_id, secret_access_key, None, None, "VersaBedrock");
        let loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .credentials_provider(credentials)
            // ⚠ SigV4 with the credentials above, chosen in code. The AWS SDK
            // reads `AWS_BEARER_TOKEN_BEDROCK` from the process environment by
            // itself, and unless the auth scheme was chosen in code it then
            // authenticates with that bearer token instead of signing. That
            // variable is where AWS tells a user to put a Bedrock API key, i.e.
            // the PUBLIC card's credential. Without this line a Versa chat that
            // looked entirely right (UCSF gateway, us-west-2, Private) sent the
            // public card's API key to UCSF, and Versa's own keys signed
            // nothing. A preference set on this loader counts as chosen in code
            // (`Origin::is_client_config`), so the SDK leaves it alone.
            .auth_scheme_preference(["sigv4".into()])
            .region(aws_config::Region::new(region.clone()))
            .endpoint_url(endpoint.as_str());
        #[cfg(test)]
        let loader = with_test_http_client(loader);
        let mut loader = loader;
        if let Some(secs) = operation_timeout_secs {
            loader = loader.timeout_config(
                aws_smithy_types::timeout::TimeoutConfig::builder()
                    .operation_timeout(std::time::Duration::from_secs(secs))
                    .build(),
            );
        }
        let sdk_config = loader.load().await;

        sdk_config
            .credentials_provider()
            .ok_or_else(|| anyhow::anyhow!("No AWS credentials provider configured"))?
            .provide_credentials()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load Versa Bedrock credentials: {}", e))?;

        let client = Client::new(&sdk_config);
        let retry_config = RetryConfig {
            max_retries: retry.max_retries,
            initial_interval_ms: retry.initial_interval_ms,
            backoff_multiplier: retry.backoff_multiplier,
            max_interval_ms: retry.max_interval_ms,
        };

        Ok(Self {
            client,
            model,
            retry_config,
            name: Self::metadata().name,
            resolved_endpoint: endpoint.into_string(),
            region,
            operation_timeout_secs,
        })
    }

    /// The same client with only its HTTP transport replaced. Everything the
    /// constructor resolved — endpoint, region, credentials, auth scheme — is
    /// kept, so a request captured through it is the request production would
    /// have sent.
    #[cfg(test)]
    pub(crate) fn with_http_client(
        mut self,
        http_client: impl aws_sdk_bedrockruntime::config::HttpClient + 'static,
    ) -> Self {
        let config = self.client.config().to_builder().http_client(http_client);
        self.client = Client::from_conf(config.build());
        self
    }

    fn load_retry_config(config: &crate::config::Config) -> RetryConfig {
        let max_retries = config
            .get_param::<usize>("BEDROCK_MAX_RETRIES")
            .unwrap_or(VERSA_BEDROCK_DEFAULT_MAX_RETRIES);
        let initial_interval_ms = config
            .get_param::<u64>("BEDROCK_INITIAL_RETRY_INTERVAL_MS")
            .unwrap_or(VERSA_BEDROCK_DEFAULT_INITIAL_RETRY_INTERVAL_MS);
        let backoff_multiplier = config
            .get_param::<f64>("BEDROCK_BACKOFF_MULTIPLIER")
            .unwrap_or(VERSA_BEDROCK_DEFAULT_BACKOFF_MULTIPLIER);
        let max_interval_ms = config
            .get_param::<u64>("BEDROCK_MAX_RETRY_INTERVAL_MS")
            .unwrap_or(VERSA_BEDROCK_DEFAULT_MAX_RETRY_INTERVAL_MS);
        RetryConfig {
            max_retries,
            initial_interval_ms,
            backoff_multiplier,
            max_interval_ms,
        }
    }

    fn load_operation_timeout_secs(config: &crate::config::Config) -> Option<u64> {
        let secs = config
            .get_param::<u64>("BEDROCK_OPERATION_TIMEOUT_SECS")
            .ok()
            .or_else(|| {
                std::env::var("BEDROCK_OPERATION_TIMEOUT_SECS")
                    .ok()
                    .and_then(|value| value.trim().parse::<u64>().ok())
            })
            .unwrap_or(super::formats::bedrock::BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS);
        (secs != 0).then_some(secs)
    }

    async fn converse(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(bedrock::Message, Option<bedrock::TokenUsage>, String), ProviderError> {
        let model_name = &model_config.model_name;

        let mut request = self
            .client
            .converse()
            .system(bedrock::SystemContentBlock::Text(system.to_string()))
            .model_id(model_name.to_string())
            .inference_config(bedrock_blocking_inference_config(model_config))
            .set_messages(Some(to_bedrock_messages(model_name, messages)?));

        if !tools.is_empty() {
            request = request.tool_config(to_bedrock_tool_config(tools)?);
        }

        let response = request.send().await.map_err(|err| {
            let said = gateway_message(&err);
            versa_refusal(classify_bedrock_converse_error(err), said, model_name)
        })?;

        let finish_reason = map_bedrock_stop_reason(&response.stop_reason);
        match response.output {
            Some(bedrock::ConverseOutput::Message(message)) => {
                Ok((message, response.usage, finish_reason))
            }
            _ => Err(ProviderError::RequestFailed(
                "No output from Bedrock".to_string(),
            )),
        }
    }

    /// Open a `ConverseStream` response. Mirrors [`Self::converse`] exactly —
    /// same system prompt, messages and tool config — so the streaming and
    /// blocking paths cannot drift in what they send.
    async fn converse_stream(
        &self,
        model_config: &ModelConfig,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<ConverseStreamResponse, ProviderError> {
        let mut request = self
            .client
            .converse_stream()
            .system(bedrock::SystemContentBlock::Text(system.to_string()))
            .model_id(model_config.model_name.clone())
            .inference_config(bedrock_inference_config(model_config))
            .set_messages(Some(to_bedrock_messages(
                &model_config.model_name,
                messages,
            )?));

        if !tools.is_empty() {
            request = request.tool_config(to_bedrock_tool_config(tools)?);
        }

        request.send().await.map_err(|err| {
            let said = gateway_message(&err);
            versa_refusal(
                classify_bedrock_converse_stream_error(err),
                said,
                &model_config.model_name,
            )
        })
    }
}

#[async_trait]
impl Provider for VersaBedrockProvider {
    fn metadata() -> ProviderMetadata {
        let models: Vec<ModelInfo> = VERSA_BEDROCK_KNOWN_MODELS
            .iter()
            .map(|&name| {
                ModelInfo::new(name, ModelConfig::new_or_fail(name).context_limit()).with_vision()
            })
            .collect();

        ProviderMetadata::with_models(
            "versa_bedrock",
            "Versa API Bedrock",
            "UCSF Anthropic models via Amazon Bedrock. Access key + secret only; endpoint and region are pre-configured.",
            VERSA_BEDROCK_DEFAULT_MODEL,
            models,
            VERSA_BEDROCK_DOC_LINK,
            // ⚠ The key and secret, and NOTHING else, as the description above
            // says. This used to declare `AWS_ENDPOINT_URL_BEDROCK` and
            // `AWS_REGION`, and the setup form persists a declared key's default
            // (DefaultProviderSetupForm seeds it as a value; DefaultSubmitHandler
            // submits it). `AWS_REGION` is one of the two keys the PUBLIC
            // `aws_bedrock` card declares, both required and both defaulted, so
            // `check_provider_configured` calls that card Configured once either
            // is in `config.yaml`: setting up UCSF's private Versa lit up the
            // public, commercial Amazon Bedrock card. `versa_azure` had the same
            // defect with the public Azure card (2026-09-03).
            //
            // Dropping them costs nothing. An install that sets nothing still
            // reaches the UCSF gateway through the constants above, and an
            // operator overrides through Versa's own `VERSA_BEDROCK_*` keys (see
            // `from_env`).
            vec![
                ConfigKey::new("VERSA_BEDROCK_ACCESS_KEY_ID", true, true, None),
                ConfigKey::new("VERSA_BEDROCK_SECRET_ACCESS_KEY", true, true, None),
            ],
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

    fn computer_use_destination(&self) -> Option<String> {
        super::base::computer_use_destination_origin(&self.resolved_endpoint)
    }

    fn computer_use_destination_identity(&self) -> Option<String> {
        Some(super::base::computer_use_destination_digest(
            &self.resolved_endpoint,
        ))
    }

    fn restore_binding(&self) -> ProviderRestoreBinding {
        ProviderRestoreBinding::VersaBedrock {
            model: model_without_restore_marker(self.model.clone()),
            endpoint: SecretFreeEndpoint::new(self.resolved_endpoint.clone())
                .expect("resolved Versa Bedrock endpoint must remain valid"),
            region: self.region.clone(),
            retry: PersistedRetryConfig {
                max_retries: self.retry_config.max_retries,
                initial_interval_ms: self.retry_config.initial_interval_ms,
                backoff_multiplier: self.retry_config.backoff_multiplier,
                max_interval_ms: self.retry_config.max_interval_ms,
            },
            operation_timeout_secs: self.operation_timeout_secs,
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

    fn retry_config(&self) -> RetryConfig {
        self.retry_config.clone()
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
        let model_name = model_config.model_name.clone();

        let debug_payload = serde_json::json!({
            "system": system,
            "messages": messages,
            "tools": tools
        });
        let mut log = RequestLog::start(&self.model, &debug_payload)?;

        let (bedrock_message, bedrock_usage, finish_reason) = self
            .with_retry(|| self.converse(model_config, system, messages, tools))
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        let usage = bedrock_usage
            .as_ref()
            .map(from_bedrock_usage)
            .unwrap_or_default();

        let message = from_bedrock_message(&bedrock_message)?;

        log.write(
            &serde_json::to_value(&message).unwrap_or_default(),
            Some(&usage),
        )?;

        let mut provider_usage = ProviderUsage::new(model_name.to_string(), usage);
        provider_usage.finish_reason = Some(finish_reason);
        Ok((message, provider_usage))
    }

    /// Stream a turn via Bedrock `ConverseStream`.
    ///
    /// Only opening the stream is retried (via `with_retry`, so the existing
    /// Versa retry budget and error classification are preserved). Once events
    /// start arriving, a failure is terminal: partial output has already reached
    /// the agent and replaying the request would duplicate it.
    async fn stream(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<MessageStream, ProviderError> {
        let model_name = self.model.model_name.clone();

        let debug_payload = serde_json::json!({
            "system": system,
            "messages": messages,
            "tools": tools,
            "stream": true
        });
        let mut log = RequestLog::start(&self.model, &debug_payload)?;

        let response = self
            .with_retry(|| self.converse_stream(&self.model, system, messages, tools))
            .await
            .inspect_err(|e| {
                let _ = log.error(e);
            })?;

        Ok(bedrock_message_stream(response, model_name, log))
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_restart_steering(&self) -> bool {
        true
    }

    /// T3-SH-3. Bedrock's runtime has no model listing, so the default check
    /// sent nothing and a made-up key pair was saved as Configured.
    ///
    /// One `Converse` call for [`CREDENTIAL_PROBE_MODEL`], which the gateway
    /// authenticates before anything else. Only a refusal that names the
    /// credential counts ([`super::names_a_rejected_credential`]): an
    /// `AccessDenied` for a model the pair may not use is a 403 too, and a check
    /// that refused it would roll a working pair back. A timeout, a network
    /// failure or a server error says nothing about the pair and is passed on
    /// as such; any other answer means the gateway let the pair through.
    async fn check_credentials(&self) -> Result<(), ProviderError> {
        let probe = bedrock::Message::builder()
            .role(bedrock::ConversationRole::User)
            .content(bedrock::ContentBlock::Text("ping".to_string()))
            .build()
            .map_err(|error| ProviderError::ExecutionError(error.to_string()))?;
        let answer = self
            .client
            .converse()
            .model_id(CREDENTIAL_PROBE_MODEL)
            .messages(probe)
            .inference_config(
                bedrock::InferenceConfiguration::builder()
                    .max_tokens(1)
                    .build(),
            )
            .send()
            .await;
        let error = match answer {
            Ok(_) => return Ok(()),
            Err(error) => error,
        };
        let status = error
            .raw_response()
            .map(|response| response.status().as_u16());
        let said = gateway_message(&error);
        credential_probe_outcome(status, said, classify_bedrock_converse_error(error))
    }
}

/// What [`VersaBedrockProvider::check_credentials`] makes of the gateway's
/// answer: its HTTP status (`None` when nothing came back), its own sentence,
/// and the error as the chat path classifies it.
///
/// A 401 is the pair. A 403 is the pair only in the words of a credential
/// refusal. Any other answer below 500 means the gateway let the pair through
/// and said something about the request (for the probe's model id, AWS's
/// `ValidationException`, which the proxy may pass on untyped). A server error
/// or no answer at all says nothing, and is handed back as it was classified.
fn credential_probe_outcome(
    status: Option<u16>,
    said: Option<String>,
    classified: ProviderError,
) -> Result<(), ProviderError> {
    match (status, classified) {
        (Some(401), _) => {
            Err(ProviderError::Authentication(said.unwrap_or_else(|| {
                "the gateway answered 401 Unauthorized".to_string()
            })))
        }
        (_, ProviderError::Authentication(_))
            if said
                .as_deref()
                .is_some_and(super::names_a_rejected_credential) =>
        {
            Err(ProviderError::Authentication(said.unwrap_or_default()))
        }
        (Some(status), _) if status < 500 => Ok(()),
        (_, unanswered) => Err(unanswered),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_smithy_http_client::test_util::capture_request;

    /// W2-PRV-9: a refused pair names Versa, the gateway's reason and where to
    /// replace the pair; every other failure is left as it was classified.
    #[test]
    fn a_refused_key_pair_says_whose_refusal_and_where_to_fix_it() {
        let refused = versa_refusal(
            ProviderError::Authentication("Bedrock endpoint returned HTTP 403".to_string()),
            Some("Invalid Client Id".to_string()),
            VERSA_BEDROCK_DEFAULT_MODEL,
        );
        assert_eq!(
            refused.to_string(),
            ProviderError::Authentication(
                "Versa rejected this key pair (Invalid Client Id). Replace it in Settings > \
                 Models > Configure providers > Institutional > Versa API Bedrock."
                    .to_string()
            )
            .to_string()
        );
        let throttled = versa_refusal(
            ProviderError::ServerError("HTTP 503".to_string()),
            Some("busy".to_string()),
            VERSA_BEDROCK_DEFAULT_MODEL,
        );
        assert!(matches!(throttled, ProviderError::ServerError(_)));
    }

    /// AWS's sentence for a working pair asking for a model UCSF's account may
    /// not invoke, as the gateway returned it on 2026-10-05 (account id elided).
    const MODEL_REFUSED: &str = "User: arn:aws:iam::000000000000:user/\
         managed-service-account-mulesoft-ai is not authorized to perform: \
         bedrock:InvokeModel on resource: arn:aws:bedrock:us-west-2:000000000000:\
         inference-profile/us.anthropic.claude-opus-5-5";

    /// The refusal a chat still bound to a model that left the list gets: the
    /// model is named, the pair is said to work, the offered models are listed,
    /// and the turn stops rather than retrying something that cannot succeed.
    fn assert_refuses_the_model(error: &ProviderError, model: &str) {
        let text = error.to_string();
        assert!(matches!(error, ProviderError::RequestFailed(_)), "{text}");
        assert!(
            text.contains(&format!("does not serve model `{model}`")),
            "{text}"
        );
        assert!(text.contains("Your key pair works"), "{text}");
        assert!(!text.contains("Replace it"), "{text}");
        for offered in VERSA_BEDROCK_KNOWN_MODELS {
            assert!(
                text.contains(offered),
                "the refusal must offer {offered}: {text}"
            );
        }
        assert_eq!(
            error.kind(),
            crate::providers::errors::ProviderErrorKind::ModelUnavailable,
            "{text}"
        );
        assert!(!crate::providers::retry::should_retry(error), "{text}");
        assert!(
            !crate::agents::mistakes::is_recoverable(error),
            "a retry can never succeed, so the turn must stop on the first one: {text}"
        );
    }

    #[test]
    fn a_model_the_account_may_not_invoke_is_not_a_rejected_key_pair() {
        let model = "us.anthropic.claude-opus-5-5";
        // The sentence reaches `versa_refusal` either as the gateway's own
        // words or inside the typed exception the classifier formatted.
        for (detail, said) in [
            (
                "Failed to call Bedrock".to_string(),
                Some(MODEL_REFUSED.to_string()),
            ),
            (format!("Failed to call Bedrock: {MODEL_REFUSED}"), None),
            (
                "Failed to call Bedrock".to_string(),
                Some("You don't have access to the model with the specified model ID.".into()),
            ),
        ] {
            let refused = versa_refusal(ProviderError::Authentication(detail), said, model);
            assert_refuses_the_model(&refused, model);
        }

        // A sentence that names the credential still wins: a bad pair is a bad
        // pair whatever else the detail carries.
        let bad_pair = versa_refusal(
            ProviderError::Authentication(format!("Failed to call Bedrock: {MODEL_REFUSED}")),
            Some("Invalid Client Id".to_string()),
            model,
        );
        assert!(
            matches!(&bad_pair, ProviderError::Authentication(text) if text.contains("Replace it")),
            "{bad_pair:?}"
        );
    }

    /// The same refusal for a model the list offers must not tell the user the
    /// model is not served and then offer it: it says the model is normally
    /// served, offers only the others, and names UCSF's support address. It
    /// still stops the turn on the first try.
    #[test]
    fn a_refused_listed_model_is_not_offered_back_as_available() {
        for model in VERSA_BEDROCK_KNOWN_MODELS {
            let refused = versa_refusal(
                ProviderError::Authentication("Failed to call Bedrock".to_string()),
                Some(MODEL_REFUSED.replace("us.anthropic.claude-opus-5-5", model)),
                model,
            );
            let text = refused.to_string();
            assert!(matches!(refused, ProviderError::RequestFailed(_)), "{text}");
            assert!(
                text.contains(&format!("normally serves model `{model}`")),
                "{text}"
            );
            assert!(!text.contains("does not serve"), "{text}");
            assert!(text.contains("Your key pair works"), "{text}");
            assert!(text.contains("versa@ucsf.edu"), "{text}");
            assert_eq!(
                text.matches(*model).count(),
                1,
                "{model} is named once, as the refused model, never as an alternative: {text}"
            );
            for other in VERSA_BEDROCK_KNOWN_MODELS.iter().filter(|o| *o != model) {
                assert!(
                    text.contains(other),
                    "the refusal must offer {other}: {text}"
                );
            }
            assert_eq!(
                refused.kind(),
                crate::providers::errors::ProviderErrorKind::ModelUnavailable,
                "{text}"
            );
            assert!(!crate::providers::retry::should_retry(&refused), "{text}");
            assert!(!crate::agents::mistakes::is_recoverable(&refused), "{text}");
        }
    }

    /// The same refusal on the real wire: a typed `AccessDeniedException`, as
    /// AWS returned it through the proxy, on both the blocking and the
    /// streaming request.
    #[tokio::test]
    async fn a_typed_access_denied_for_the_model_reaches_the_user_as_a_model_refusal() {
        let model = "us.anthropic.claude-opus-5-5";
        let refusal = || {
            axum::http::Response::builder()
                .status(403)
                .header("content-type", "application/json")
                .header("x-amzn-errortype", "AccessDeniedException")
                .body(aws_smithy_types::body::SdkBody::from(
                    serde_json::json!({ "message": MODEL_REFUSED }).to_string(),
                ))
                .unwrap()
        };
        let config = ModelConfig::new_or_fail(model);
        let prompt = [Message::user().with_text("hello")];

        let (http, _captured) = capture_request(Some(refusal()));
        let provider = provider_at("https://versa-bedrock.invalid")
            .await
            .with_http_client(http);
        let completed = provider
            .converse(&config, "system", &prompt, &[])
            .await
            .expect_err("the gateway refused the model");
        assert_refuses_the_model(&completed, model);

        let (http, _captured) = capture_request(Some(refusal()));
        let provider = provider_at("https://versa-bedrock.invalid")
            .await
            .with_http_client(http);
        let streamed = match provider
            .converse_stream(&config, "system", &prompt, &[])
            .await
        {
            Ok(_) => panic!("the gateway refused the model"),
            Err(error) => error,
        };
        assert_refuses_the_model(&streamed, model);
    }

    /// A provider wired the way `from_env` builds one, minus the credential and
    /// global-config lookups — `from_env` needs UCSF-issued secrets, so it
    /// cannot run here. Everything below is a pure function of
    /// `resolved_endpoint`, and the client is built through the same
    /// `aws_config` loader production uses so the struct literal cannot drift
    /// from a real one.
    async fn provider_at(endpoint: &str) -> VersaBedrockProvider {
        let sdk_config =
            with_test_http_client(aws_config::defaults(aws_config::BehaviorVersion::latest()))
                .credentials_provider(Credentials::new(
                    "test-access-key",
                    "test-secret-key",
                    None,
                    None,
                    "VersaBedrockTest",
                ))
                .region(aws_config::Region::new(VERSA_BEDROCK_DEFAULT_REGION))
                .endpoint_url(endpoint.to_string())
                .load()
                .await;

        VersaBedrockProvider {
            client: Client::new(&sdk_config),
            model: ModelConfig::new_or_fail(VERSA_BEDROCK_DEFAULT_MODEL),
            retry_config: RetryConfig::default(),
            name: "versa_bedrock".to_string(),
            resolved_endpoint: endpoint.to_string(),
            region: VERSA_BEDROCK_DEFAULT_REGION.to_string(),
            operation_timeout_secs: Some(
                crate::providers::formats::bedrock::BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS,
            ),
        }
    }

    async fn capturing_provider() -> (
        VersaBedrockProvider,
        aws_smithy_http_client::test_util::CaptureRequestReceiver,
    ) {
        let (http_client, captured) = capture_request(None);
        let endpoint = "https://versa-bedrock.invalid";
        let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .credentials_provider(Credentials::new(
                "test-access-key",
                "test-secret-key",
                None,
                None,
                "VersaBedrockWireTest",
            ))
            .region(aws_config::Region::new(VERSA_BEDROCK_DEFAULT_REGION))
            .endpoint_url(endpoint)
            .http_client(http_client)
            .load()
            .await;
        (
            VersaBedrockProvider {
                client: Client::new(&sdk_config),
                model: ModelConfig::new_or_fail(VERSA_BEDROCK_DEFAULT_MODEL),
                retry_config: RetryConfig::default(),
                name: "versa_bedrock".to_string(),
                resolved_endpoint: endpoint.to_string(),
                region: VERSA_BEDROCK_DEFAULT_REGION.to_string(),
                operation_timeout_secs: Some(
                    crate::providers::formats::bedrock::BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS,
                ),
            },
            captured,
        )
    }

    #[tokio::test]
    async fn restore_binding_keeps_route_and_transport_policy_without_credentials() {
        let endpoint = "https://versa-bedrock.invalid/exact-route";
        let provider = provider_at(endpoint).await;
        let encoded = serde_json::to_value(provider.restore_binding()).unwrap();
        assert_eq!(encoded["kind"], "versa_bedrock");
        assert_eq!(encoded["endpoint"], endpoint);
        assert_eq!(encoded["region"], VERSA_BEDROCK_DEFAULT_REGION);
        assert_eq!(
            encoded["operation_timeout_secs"],
            crate::providers::formats::bedrock::BEDROCK_DEFAULT_OPERATION_TIMEOUT_SECS
        );
        let text = encoded.to_string();
        assert!(!text.contains("test-access-key"));
        assert!(!text.contains("test-secret-key"));
    }

    #[tokio::test]
    async fn provider_advertises_restart_steering() {
        let provider = provider_at(VERSA_BEDROCK_DEFAULT_ENDPOINT).await;
        assert!(provider.supports_streaming());
        assert!(!provider.supports_live_steering());
        assert!(
            provider.supports_restart_steering(),
            "Versa Bedrock cannot inject into ConverseStream, so a queued steer must restart it"
        );
    }

    #[tokio::test]
    async fn restore_reloads_rotated_credentials_and_fails_closed_when_they_are_missing() {
        use std::collections::HashMap;

        async fn restore_with(access_key: &str, secret_key: &str) -> Result<VersaBedrockProvider> {
            crate::config::with_config_overrides(
                HashMap::from([
                    ("VERSA_BEDROCK_ACCESS_KEY_ID".into(), access_key.into()),
                    ("VERSA_BEDROCK_SECRET_ACCESS_KEY".into(), secret_key.into()),
                ]),
                VersaBedrockProvider::from_resolved(
                    ModelConfig::new_or_fail(VERSA_BEDROCK_DEFAULT_MODEL),
                    SecretFreeEndpoint::new("https://versa-bedrock.invalid/exact".into()).unwrap(),
                    VERSA_BEDROCK_DEFAULT_REGION.into(),
                    PersistedRetryConfig {
                        max_retries: 6,
                        initial_interval_ms: 2_000,
                        backoff_multiplier: 2.0,
                        max_interval_ms: 120_000,
                    },
                    Some(300),
                ),
            )
            .await
        }

        let missing = restore_with("", "").await;
        assert!(missing.is_err());

        let first = restore_with("first-access-sentinel", "first-secret-sentinel")
            .await
            .unwrap();
        let second = restore_with("second-access-sentinel", "second-secret-sentinel")
            .await
            .unwrap();
        let first = serde_json::to_value(first.restore_binding()).unwrap();
        let second = serde_json::to_value(second.restore_binding()).unwrap();
        assert_eq!(
            first, second,
            "credential rotation must not change the binding"
        );
        let encoded = second.to_string();
        assert!(!encoded.contains("second-access-sentinel"));
        assert!(!encoded.contains("second-secret-sentinel"));
    }

    fn assert_inference_wire(
        captured: aws_smithy_http_client::test_util::CaptureRequestReceiver,
        expected_tokens: i64,
        expected_temperature: Option<f64>,
    ) {
        let request = captured.expect_request();
        let body = request.body().bytes().expect("buffered request body");
        let json: serde_json::Value = serde_json::from_slice(body).expect("JSON request body");
        assert_eq!(json["inferenceConfig"]["maxTokens"], expected_tokens);
        assert_eq!(
            json["inferenceConfig"]["temperature"],
            expected_temperature.map_or(serde_json::Value::Null, serde_json::Value::from)
        );
    }

    #[tokio::test]
    async fn converse_sends_configured_inference_fields_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6")
            .with_max_tokens(Some(34_567))
            .with_temperature(Some(0.25));
        let _ = provider
            .converse(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 21_333, Some(0.25));
    }

    #[tokio::test]
    async fn converse_uses_transport_safe_default_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6");
        let _ = provider
            .converse(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 21_333, None);
    }

    #[tokio::test]
    async fn converse_stream_sends_configured_inference_fields_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6")
            .with_max_tokens(Some(45_678))
            .with_temperature(Some(0.5));
        let _ = provider
            .converse_stream(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 45_678, Some(0.5));
    }

    /// The UI auto-selects `known_models[0]` on a provider switch, so the
    /// default and the first entry must agree. The list is exactly the ids that
    /// answered through the UCSF proxy on 2026-10-05, and none that UCSF's
    /// account refused or that Bedrock retired.
    #[test]
    fn the_offered_list_is_exactly_what_the_ucsf_proxy_answered() {
        assert_eq!(VERSA_BEDROCK_DEFAULT_MODEL, "us.anthropic.claude-opus-4-8");
        assert_eq!(VERSA_BEDROCK_KNOWN_MODELS[0], VERSA_BEDROCK_DEFAULT_MODEL);
        assert_eq!(
            VERSA_BEDROCK_KNOWN_MODELS,
            [
                "us.anthropic.claude-opus-4-8",
                "us.anthropic.claude-sonnet-5",
                "us.anthropic.claude-opus-4-6-v1",
                "us.anthropic.claude-sonnet-4-6",
                "us.anthropic.claude-opus-4-5-20251101-v1:0",
                "us.anthropic.claude-haiku-4-5-20251001-v1:0",
            ]
        );
        for refused in [
            // AccessDeniedException from UCSF's IAM policy (2026-10-05).
            "us.anthropic.claude-opus-5-5",
            "us.anthropic.claude-opus-5",
            "us.anthropic.claude-opus-4-7",
            "us.anthropic.claude-fable-5-1",
            "us.anthropic.claude-fable-5",
            "us.anthropic.claude-sonnet-5-5",
            "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
            "us.anthropic.claude-sonnet-4-20250514-v1:0",
            // Answers, but Bedrock Legacy with an end of life of 2027-01-08.
            "us.anthropic.claude-opus-4-1-20250805-v1:0",
            // End of life on Bedrock.
            "us.anthropic.claude-opus-4-20250514-v1:0",
        ] {
            assert!(
                !VERSA_BEDROCK_KNOWN_MODELS.contains(&refused),
                "{refused} is not served to UCSF's account"
            );
        }
        assert!(
            !VERSA_BEDROCK_KNOWN_MODELS
                .iter()
                .any(|model| model.contains("claude-3")),
            "every Claude 3.x id is end of life on Bedrock"
        );
        // Family-wide, not per id: a Fable id AWS publishes later, or one UCSF
        // enables, is still one that needs `aws_review` retention.
        assert!(
            !VERSA_BEDROCK_KNOWN_MODELS
                .iter()
                .any(|model| model.contains("fable")),
            "Fable needs Bedrock's aws_review retention mode; not offered on Versa"
        );
    }

    /// Every offered id sizes and prices as itself: its own context-window row
    /// and a price. Opus 4.5 and Haiku 4.5 were unpriced here until 2026-10-05,
    /// when `versa_bedrock` joined the public card on the first-party Claude
    /// table in `pricing.rs`. Sonnet 5 is checked by value, because it is the
    /// entry this list gained and the cheapest 1M model on it.
    #[test]
    fn every_offered_model_has_its_own_window_and_a_price() {
        // context_limit() and metadata() honour BIOROUTER_CONTEXT_LIMIT, which
        // other tests in this binary set process-wide under env_lock, so the
        // window assertions below must hold it (as versa_azure's do).
        let _guard = env_lock::lock_env([
            ("BIOROUTER_CONTEXT_LIMIT", None::<&str>),
            ("BIOROUTER_PREDEFINED_MODELS", None::<&str>),
        ]);
        for model in VERSA_BEDROCK_KNOWN_MODELS {
            assert!(
                ModelConfig::has_declared_context_window(model),
                "{model} borrows its context window from a pattern"
            );
            assert!(
                crate::providers::pricing::provider_model_pricing("versa_bedrock", model).is_some(),
                "{model} is unpriced on versa_bedrock"
            );
        }

        let sonnet_5 = "us.anthropic.claude-sonnet-5";
        assert_eq!(
            ModelConfig::new_or_fail(sonnet_5).context_limit(),
            1_000_000
        );
        let info = VersaBedrockProvider::metadata()
            .known_models
            .into_iter()
            .find(|info| info.name == sonnet_5)
            .expect("Sonnet 5 is offered");
        assert_eq!(info.context_limit, 1_000_000);
        assert_eq!(info.supports_vision, Some(true));
        let price = crate::providers::pricing::provider_model_pricing("versa_bedrock", sonnet_5)
            .expect("Sonnet 5 is priced");
        assert!(
            (price.input_token_cost * 1_000_000.0 - 2.0).abs() < 1e-9,
            "{price:?}"
        );
        assert!(
            (price.output_token_cost * 1_000_000.0 - 10.0).abs() < 1e-9,
            "{price:?}"
        );
    }

    /// A Converse request for a preserved-thinking model carries no replayed
    /// reasoning, on the real wire, while the same history for Opus 4.8 keeps
    /// it — the stripping is keyed on the model and nothing else.
    ///
    /// Opus 5.5 is no longer offered here (UCSF's account is refused it,
    /// 2026-10-05), but a typed-in or previously bound id still reaches this
    /// request builder, and the builder is the one the public card shares.
    #[tokio::test]
    async fn converse_strips_replayed_reasoning_only_for_preserved_thinking_models() {
        let history = [
            Message::user().with_text("first"),
            Message::assistant()
                .with_thinking("", "sig-1")
                .with_text("answer"),
            Message::user().with_text("second"),
        ];

        for (model, expect_reasoning) in [
            ("us.anthropic.claude-opus-5-5", false),
            ("us.anthropic.claude-opus-4-8", true),
        ] {
            let (provider, captured) = capturing_provider().await;
            let _ = provider
                .converse(&ModelConfig::new_or_fail(model), "system", &history, &[])
                .await;
            let request = captured.expect_request();
            let body = request.body().bytes().expect("buffered request body");
            let json: serde_json::Value = serde_json::from_slice(body).expect("JSON body");
            let rendered = json["messages"].to_string();
            assert_eq!(
                rendered.contains("reasoningContent"),
                expect_reasoning,
                "{model}: {rendered}"
            );
            assert_eq!(json["messages"].as_array().unwrap().len(), 3, "{model}");
        }
    }

    #[tokio::test]
    async fn converse_stream_keeps_large_model_default_on_the_wire() {
        let (provider, captured) = capturing_provider().await;
        let config = ModelConfig::new_or_fail("us.anthropic.claude-sonnet-4-6");
        let _ = provider
            .converse_stream(
                &config,
                "system",
                &[Message::user().with_text("hello")],
                &[],
            )
            .await;
        assert_inference_wire(captured, 64_000, None);
    }

    /// Task 5 rule 2, **wired** — not just the predicate behind it.
    ///
    /// `providers::ucsf_gateway_tier` is unit-tested on its own in
    /// `tier_tests.rs`, but a test of the predicate alone cannot see whether
    /// this provider calls it, or hands it the right field. Replace the body of
    /// `tier()` with an unconditional `Private` and every one of those tests
    /// still passes. This one does not. The demotion is still needed although
    /// `from_env` no longer falls back to the public side's keys:
    /// `VERSA_BEDROCK_ENDPOINT` is user-writable config.
    #[tokio::test]
    async fn tier_follows_the_endpoint_this_instance_resolved() {
        let shipped = provider_at(VERSA_BEDROCK_DEFAULT_ENDPOINT).await;
        assert_eq!(shipped.tier(), ProviderTier::Private);

        let elsewhere = provider_at("https://bedrock-runtime.us-west-2.amazonaws.com").await;
        // Same name, same metadata, same everything a name-keyed rule can see.
        assert_eq!(elsewhere.get_name(), shipped.get_name());
        assert_eq!(
            VersaBedrockProvider::metadata().tier,
            ProviderTier::Private,
            "the type-level claim is still Private; only the instance demotes"
        );
        assert_eq!(elsewhere.tier(), ProviderTier::Public);
    }

    /// DR-26 (Task 46) rule, **wired** — the same argument as the tier test
    /// above, for the third axis. `VERSA_BEDROCK_ENDPOINT` is user-writable, and
    /// an affiliation keyed on the provider's name would keep claiming `ucsf`
    /// for an instance repointed at a plain AWS region.
    #[tokio::test]
    async fn affiliation_follows_the_endpoint_this_instance_resolved() {
        use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};

        let shipped = provider_at(VERSA_BEDROCK_DEFAULT_ENDPOINT).await;
        assert_eq!(
            shipped.affiliation(),
            Some(ModelAffiliation::institution(InstitutionId::new("ucsf")))
        );

        let elsewhere = provider_at("https://bedrock-runtime.us-west-2.amazonaws.com").await;
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
        let institutions = VersaBedrockProvider::metadata().institutions.clone();
        assert_eq!(institutions.len(), 1);
        assert_eq!(institutions[0].id, "ucsf");
        assert_eq!(institutions[0].display_name.as_deref(), Some("UCSF"));
    }

    /// T3-SH-3: the credential probe, answered the way the UCSF gateway was
    /// measured to answer (2026-09-28), and the request it made.
    async fn probed(status: u16, body: &str) -> (Result<(), ProviderError>, String) {
        let response = axum::http::Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(aws_smithy_types::body::SdkBody::from(body.to_string()))
            .unwrap();
        let (http, captured) = capture_request(Some(response));
        let provider = provider_at("https://versa-bedrock.invalid")
            .await
            .with_http_client(http);
        let outcome = provider.check_credentials().await;
        let path = captured.expect_request().uri().to_string();
        (outcome, path)
    }

    #[tokio::test]
    async fn a_pair_the_gateway_does_not_know_is_refused_in_its_words() {
        let (outcome, path) = probed(403, r#"{"message": "Invalid Client Id"}"#).await;
        match outcome {
            Err(ProviderError::Authentication(reason)) => assert_eq!(reason, "Invalid Client Id"),
            other => panic!("the gateway refused the pair, got {other:?}"),
        }
        // The probe asks for a model nothing serves, so it can never run one.
        assert!(
            path.contains(&format!("/model/{CREDENTIAL_PROBE_MODEL}/converse")),
            "{path}"
        );
        let (outcome, _) = probed(
            403,
            r#"{"message": "The request signature we calculated does not match the signature you provided. Check your Mule client id and signing method."}"#,
        )
        .await;
        assert!(
            matches!(outcome, Err(ProviderError::Authentication(_))),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_pair_the_gateway_lets_through_is_accepted() {
        // What AWS says about a model id it does not know, once the gateway has
        // accepted the pair.
        let (outcome, _) = probed(
            400,
            r#"{"message": "The provided model identifier is invalid."}"#,
        )
        .await;
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    #[tokio::test]
    async fn a_403_that_is_not_about_the_pair_never_rolls_it_back() {
        // A valid pair not entitled to a model gets a 403 as well.
        let (outcome, _) = probed(
            403,
            r#"{"message": "You don't have access to the model with the specified model ID."}"#,
        )
        .await;
        assert!(outcome.is_ok(), "{outcome:?}");
        let (outcome, _) = probed(403, "").await;
        assert!(
            outcome.is_ok(),
            "an unexplained 403 is not proof: {outcome:?}"
        );
    }

    /// A 5xx is retried by the SDK, which a one-shot capture cannot answer, so
    /// the rule is asserted where it is decided.
    #[test]
    fn a_gateway_that_does_not_answer_says_nothing_about_the_pair() {
        for (status, classified) in [
            (Some(503), ProviderError::ServerError("busy".to_string())),
            (None, ProviderError::ServerError("timed out".to_string())),
            (
                None,
                ProviderError::RequestFailed("connection refused".to_string()),
            ),
        ] {
            let outcome = credential_probe_outcome(status, None, classified);
            assert!(
                matches!(
                    outcome,
                    Err(ProviderError::ServerError(_) | ProviderError::RequestFailed(_))
                ),
                "{status:?}: {outcome:?}"
            );
        }
        // A 401 is the pair whatever else was said.
        assert!(matches!(
            credential_probe_outcome(Some(401), None, ProviderError::ServerError(String::new())),
            Err(ProviderError::Authentication(_))
        ));
    }
}
