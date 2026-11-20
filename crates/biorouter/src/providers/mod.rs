#[cfg(test)]
mod affiliation_tests;
pub mod anthropic;
pub mod api_client;
pub mod auto_detect;
#[cfg(feature = "aws-providers")]
pub mod aws_stored_settings;
pub mod azure;
pub mod azureauth;
pub mod base;
#[cfg(feature = "aws-providers")]
pub mod bedrock;
#[cfg(all(test, feature = "aws-providers"))]
mod bedrock_namespace_tests;
pub mod canonical;
pub mod claude_code;
pub mod codex;
pub mod coding_agent;
pub mod databricks;
pub mod decision;
// DR-33: the slot's wrapper that folds a chat's decision models into its tier.
pub mod decision_bound;
pub mod destination_keys;
pub mod embedding;
pub mod errors;
mod factory;
pub mod formats;
mod gcpauth;
pub mod gcpvertexai;
pub mod githubcopilot;
pub mod google;
pub mod lead_worker;
pub mod litellm;
pub mod live_catalog;
pub mod llamacpp;
pub mod llamacpp_sidecar;
pub mod model_display;
pub mod oauth;
pub mod ollama;
pub mod openai;
pub mod openrouter;
pub mod pricing;
pub(crate) mod provider_binding;
pub mod provider_registry;
pub mod provider_test;
pub mod readiness;
pub(crate) mod retry;
#[cfg(feature = "aws-providers")]
pub mod sagemaker_tgi;
pub mod snowflake;
pub mod testprovider;
pub mod tetrate;
#[cfg(test)]
mod tier_tests;
pub mod tool_turn;
pub mod toolshim;
pub mod usage_estimator;
pub mod utils;
pub mod venice;
pub mod versa_azure;
#[cfg(feature = "aws-providers")]
pub mod versa_bedrock;
pub mod xai;
pub mod xiaomi_mimo;
pub mod zai;

pub use destination_keys::is_destination_key;
pub use factory::builtin_provider_metadata;
pub(crate) use factory::create_from_persisted;
pub use factory::{
    create, create_from_saved_session, create_with_default_model, create_with_named_model,
    providers, refresh_custom_providers,
};
pub use retry::{retry_operation, RetryConfig};

/// Session-safe construction state. Composites already publish their versioned
/// two-provider recipe; specialized ordinary providers need their exact
/// secret-free binding wrapped separately so a cold restore cannot drift with
/// process configuration. Registry providers retain their established shape.
pub(crate) fn persisted_model_config(
    provider: &dyn base::Provider,
) -> anyhow::Result<crate::model::ModelConfig> {
    if provider.as_lead_worker().is_some() {
        return Ok(provider.get_model_config());
    }

    persisted_model_config_from_binding(provider.get_name(), provider.restore_binding())
}

pub(crate) fn persisted_model_config_from_binding(
    provider_name: &str,
    binding: provider_binding::ProviderRestoreBinding,
) -> anyhow::Result<crate::model::ModelConfig> {
    anyhow::ensure!(
        binding.provider_name() == provider_name,
        "provider restore binding does not match provider '{}'",
        provider_name
    );
    if !binding.requires_exact_restore() {
        return Ok(binding.model().clone());
    }
    provider_binding::PersistedStandaloneProviderBinding::new(binding)?.to_model_config()
}

use std::sync::LazyLock;

use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};
use crate::privacy::ProviderTier;

/// The compiled-in UCSF gateway host. `versa_azure` and `versa_bedrock` are
/// Private only while their resolved endpoint is on it.
pub(crate) const UCSF_GATEWAY_HOST: &str = "unified-api.ucsf.edu";

/// The institution whose agreements cover [`UCSF_GATEWAY_HOST`] (DR-26).
///
/// It sits beside the host, not beside a provider name, because the host is
/// what the decision is made on — see [`ucsf_gateway_affiliation`].
pub(crate) const UCSF_INSTITUTION: &str = "ucsf";

pub(crate) fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

/// True only for a loopback host. R1 makes "self-hosted" private; a
/// non-loopback host is not evidence of self-hosting, and treating it as such
/// turns one writable config key into a forged private badge.
///
/// Purely lexical, and deliberately narrow: `127.0.0.2`, `::ffff:127.0.0.1` and
/// a scheme-less `localhost:11434` all answer **false** despite being this
/// machine. That direction is fail-safe — it costs an unusual setup a badge and
/// costs nobody a transcript — and every spelling added here is a spelling an
/// attacker may also write. The edges, and the residual assumption behind the
/// `.localhost` arm, are pinned in `tier_tests::only_a_loopback_host_reads_as_this_machine`.
pub(crate) fn is_loopback_host(url: &str) -> bool {
    match host_of(url).as_deref() {
        Some("localhost") | Some("127.0.0.1") | Some("::1") | Some("[::1]") => true,
        // RFC 6761 reserves `.localhost` for loopback. Note this does not match
        // `localhost.evil.example`, which has no leading dot before the label.
        Some(h) => h.ends_with(".localhost"),
        None => false,
    }
}

/// The phrase a provider uses when a credential was **never set**, as distinct
/// from "the credential store refused to hand it over". The two need opposite
/// responses from the user — add the key, versus do NOT re-enter it and answer
/// the Keychain prompt — and `versa_bedrock::from_env` carries a comment saying
/// exactly that about its own two arms.
///
/// ⚠ **It exists so the wording has ONE spelling.** The `anyhow::Error` that
/// leaves `from_env` has already discarded the `ConfigError` behind it, so a
/// caller further out — `biorouter-cli`'s `keyring_advice`, which decides whether
/// to print three lines about the system keychain — has nothing but the text to
/// go on. A literal repeated at both ends is a literal that drifts at one end;
/// the producers format with this constant and the consumer matches on it.
pub const CREDENTIAL_NEVER_SET: &str = "is not configured";

/// Whether `text` is a provider saying a credential was never set. See
/// [`CREDENTIAL_NEVER_SET`] for why this is a wording check and not a type one.
pub fn says_credential_never_set(text: &str) -> bool {
    text.contains(CREDENTIAL_NEVER_SET)
}

/// What to do about a missing key while `BIOROUTER_DISABLE_KEYRING` keeps
/// secrets in the plaintext file (#521). The system keychain was never read,
/// and the desktop app's Settings saves keys there, so "add it in Settings"
/// cannot help; these two remedies can.
pub const KEYCHAIN_DISABLED_REMEDY: &str =
    "BIOROUTER_DISABLE_KEYRING is set, so the system keychain was not checked. \
     Unset BIOROUTER_DISABLE_KEYRING to use keys saved in the system keychain, \
     or run `biorouter configure` to add the key to the file.";

/// What a provider says when credential `name` is not in `config`'s store.
/// `remedy` is where the key is usually added ("Add it under Versa API Bedrock
/// in Settings."), said while the system keychain is the store. While
/// `BIOROUTER_DISABLE_KEYRING` has selected the file store, the message names
/// the file that was read and gives [`KEYCHAIN_DISABLED_REMEDY`] instead. Both
/// keep [`CREDENTIAL_NEVER_SET`], which the CLI matches on.
pub fn credential_never_set(
    config: &crate::config::Config,
    name: &str,
    remedy: Option<&str>,
) -> String {
    if let Some(file) = config.secrets_file_selected_by_environment() {
        return format!(
            "{name} {CREDENTIAL_NEVER_SET} in the file store {}. {KEYCHAIN_DISABLED_REMEDY}",
            file.display()
        );
    }
    match remedy {
        Some(remedy) => format!("{name} {CREDENTIAL_NEVER_SET}. {remedy}"),
        None => format!("{name} {CREDENTIAL_NEVER_SET}"),
    }
}

/// Read credential `name` for a provider, saying why when it cannot.
///
/// ⚠ A blanket `map_err(|_| "... is not configured")` DISCARDS the reason, and
/// the two reasons need opposite responses from the user:
///
/// - `NotFound`: the key is not saved, so adding it is right
///   ([`credential_never_set`], with `remedy`).
/// - anything else: the key IS there and the store refused the read, so "add
///   it" sends the user to retype a credential that is already correct. On
///   macOS that happens whenever the Keychain grant no longer covers the
///   running binary: the grant is bound to the signature, so a newly installed
///   or re-signed build asks again, and an unanswered prompt fails the read.
///
/// The value never appears in an error.
pub(crate) fn read_provider_credential(
    config: &crate::config::Config,
    name: &str,
    remedy: Option<&str>,
) -> anyhow::Result<String> {
    match config.get_secret::<String>(name) {
        Ok(value) => Ok(value),
        Err(crate::config::ConfigError::NotFound(_)) => {
            Err(anyhow::anyhow!(credential_never_set(config, name, remedy)))
        }
        Err(error) => match config.secrets_file_selected_by_environment() {
            Some(file) => Err(anyhow::anyhow!(
                "Could not read {name} from the file store {}: {error}",
                file.display()
            )),
            None => Err(anyhow::anyhow!(
                "Could not read {name} from the credential store: {error}\n\n\
                 The credential appears to be configured, so this is the store \
                 refusing the read rather than a missing key -- do NOT re-enter it. \
                 On macOS a Keychain grant is tied to the application's signature, \
                 so a newly installed or re-signed build asks for permission again; \
                 answer that prompt with \u{201c}Always Allow\u{201d}."
            )),
        },
    }
}

/// What to tell a person whose provider could not start while
/// `BIOROUTER_DISABLE_KEYRING` keeps secrets in the file store: the file that
/// was read and [`KEYCHAIN_DISABLED_REMEDY`]. `None` while the system keychain
/// is the store. The CLI says it in place of its advice to check the keychain.
pub fn keychain_disabled_advice() -> Option<String> {
    crate::config::Config::global()
        .secrets_file_selected_by_environment()
        .map(|file| {
            format!(
                "Keys are read from the file store {} only. {KEYCHAIN_DISABLED_REMEDY}",
                file.display()
            )
        })
}

/// Whether a gateway's refusal names the CREDENTIAL, as opposed to anything
/// else a 403 can mean (no access to one model, a network rule, a policy).
///
/// T3-SH-3. `/config/check_provider` refuses to save a key its provider
/// rejects, and a refusal read too widely would roll a working key back: a
/// valid Versa pair that is not entitled to the model the check happens to use
/// answers 403 as well. So a Versa check refuses a 403 only in these words:
/// the UCSF gateway's own (`Invalid client id or secret`, `Invalid Client Id`,
/// the MuleSoft signature sentence) and AWS's two for a key that does not exist
/// or does not sign. A 401 is always the credential and needs no wording.
pub(crate) fn names_a_rejected_credential(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    [
        "invalid client id",
        "client id or secret",
        "invalid client secret",
        "mule client id",
        "signature we calculated does not match",
        "security token included in the request is invalid",
        "invalid subscription key",
    ]
    .iter()
    .any(|phrase| lowered.contains(phrase))
}

/// The tier of a provider that reaches the UCSF gateway and nothing else.
///
/// Demotion only, never promotion: each Versa provider's endpoint is
/// user-writable config (`VERSA_AZURE_ENDPOINT`, `VERSA_BEDROCK_ENDPOINT`), and
/// until 2026-09-11 each also read the public card's keys, so an endpoint that
/// is not the gateway means the transcript is going somewhere this build cannot
/// vouch for.
pub(crate) fn ucsf_gateway_tier(endpoint: &str) -> ProviderTier {
    if host_of(endpoint).as_deref() == Some(UCSF_GATEWAY_HOST) {
        ProviderTier::Private
    } else {
        ProviderTier::Public
    }
}

/// The tier of a provider whose inference is supposed to run on this machine.
/// Private exactly while the resolved base URL is loopback.
pub(crate) fn self_hosted_tier(base_url: &str) -> ProviderTier {
    if is_loopback_host(base_url) {
        ProviderTier::Private
    } else {
        ProviderTier::Public
    }
}

/// The affiliation of a provider that reaches the UCSF gateway and nothing else
/// (DR-26). `Institution("ucsf")` exactly while [`ucsf_gateway_tier`] says
/// Private, and `None` otherwise.
///
/// ⚠ **It routes through the tier predicate rather than repeating its host
/// check**, so the two cannot disagree for any endpoint — not merely for the
/// ones a test thought to list. A name-keyed table (`versa_* => ucsf`) would
/// keep claiming the institution for a Versa module repointed at another host,
/// which `tier()` had already demoted to Public: a private-looking badge on a
/// public flow. Each Versa endpoint is user-writable config, so that repointing
/// is a config edit away.
pub(crate) fn ucsf_gateway_affiliation(endpoint: &str) -> Option<ModelAffiliation> {
    match ucsf_gateway_tier(endpoint) {
        ProviderTier::Private => Some(*UCSF_AFFILIATION),
        ProviderTier::Public => None,
    }
}

/// The one affiliation this module can produce, built once.
///
/// [`InstitutionId::new`] normalises and then takes the **process-wide** interner
/// mutex (`privacy::affiliation::INTERNER`), and building the set around it takes
/// a second one. That is fine for a cold constructor and wrong for a decider:
/// Task 48 samples affiliation once per call, so under a parallel tool batch
/// every provider in the process would queue on those locks to re-derive a
/// constant. The value is identical either way — both `InstitutionId` and
/// `InstitutionSet` compare by contents rather than by pointer, which
/// `ids_with_equal_contents_but_distinct_pointers_are_equal` and
/// `sets_with_equal_contents_but_distinct_pointers_are_equal` pin — so this is
/// two locks removed, not a semantic change, and the existing assertions against
/// `InstitutionId::new("ucsf")` are what would catch a wrong one.
static UCSF_AFFILIATION: LazyLock<ModelAffiliation> =
    LazyLock::new(|| ModelAffiliation::institution(InstitutionId::new(UCSF_INSTITUTION)));

/// The affiliation of a provider whose inference is supposed to run on this
/// machine. [`ModelAffiliation::Local`] exactly while [`self_hosted_tier`] says
/// Private — i.e. while [`is_loopback_host`] holds.
///
/// ⚠ **A remote base URL yields `None`, not `Local`.** `OLLAMA_HOST` and
/// `LLAMACPP_EXTERNAL_HOST` are user-writable and need no auth, so a
/// non-loopback host is someone else's server: it is neither this machine nor an
/// institution this build can name. `Local` is the *most* permissive affiliation
/// in DR-26's model — it reaches every private extension, because no transfer
/// occurs at all — and inheriting it for a lab box in another building would
/// hand that box the blanket permission.
///
/// Like the tier above, it reuses [`is_loopback_host`] transitively rather than
/// re-deriving it. That predicate is deliberately narrow and lexical
/// (`127.0.0.2` and `::ffff:127.0.0.1` are **false**); a "more correct" second
/// copy would silently widen who counts as local.
pub(crate) fn self_hosted_affiliation(base_url: &str) -> Option<ModelAffiliation> {
    match self_hosted_tier(base_url) {
        ProviderTier::Private => Some(ModelAffiliation::Local),
        ProviderTier::Public => None,
    }
}

/// The affiliation of a **composite** provider — DR-26's third axis for the
/// tree's one lead/worker pair, and the sibling of [`ProviderTier::least`].
///
/// A composite discloses the whole transcript to *both* endpoints, so it may
/// reach an extension only where both halves may: the answer is the **meet** of
/// the two, never the lead's. Anything keyed on `get_name()` returns the lead's
/// alone (`LeadWorkerProvider::get_name`), which is the same mistake `tier()`
/// already had to override for.
///
/// ⚠ [`ModelAffiliation::Local`] is the **identity** of this fold, not an
/// absorbing element. That is DR-26's inversion arriving on a second axis: a
/// local half discloses nothing, so it neither dilutes the institution the other
/// half is covered by (which would forge reach it never had) nor overrides it
/// with `Local`'s blanket permission (which would grant the pair everything
/// private). Both wrong answers pass a fold written as equality.
///
/// ⚠ `None` from a half means that half is **Public** — affiliation is `Some`
/// exactly while a provider's tier is Private, which every affiliated provider
/// decides with the one predicate that decides its tier
/// (`ucsf_gateway_affiliation`, `self_hosted_affiliation`) and
/// `a_versa_endpoint_is_ucsf_exactly_when_it_is_private` pins. So a `None` half
/// has already made [`ProviderTier::least`] answer Public for the pair, and a
/// public model's affiliation is the absence of one. Returning the other half's
/// institution here would put a private-looking badge on a composite whose
/// transcript is going to a public endpoint.
///
/// ⚠ **Two institutional halves fold to their UNION, and the union is the meet
/// of their reach** rather than a widening of it. That reads backwards until the
/// direction is fixed: the model's set is what the pair *is covered by*, and
/// [`compatible`](crate::privacy::affiliation::compatible) asks whether it is a
/// **subset** of the extension's allowlist — so adding an institution to the
/// model side can only ever *narrow* what it reaches. `{ucsf} ∪ {stanford}`
/// clears exactly the connectors that admit both, which is the conjunction the
/// fold owes, and warns for either institution's own connector, which is the
/// disclosure it must not hide.
///
/// This arm used to have no representable answer and returned a fake institution
/// id. That was safe only while nobody registered an institution with that id,
/// and it refused the one flow the pair is genuinely entitled to — a connector
/// whose allowlist names both institutions, which is exactly what a DUA papers.
pub(crate) fn composite_affiliation(
    lead: Option<ModelAffiliation>,
    worker: Option<ModelAffiliation>,
) -> Option<ModelAffiliation> {
    match (lead, worker) {
        // A public half. `least` has already demoted the pair to Public.
        (None, _) | (_, None) => None,
        // `Local` is the identity: a half that discloses nothing contributes
        // nothing to whose agreements cover the pair.
        (Some(ModelAffiliation::Local), other) => other,
        (other, Some(ModelAffiliation::Local)) => other,
        (Some(ModelAffiliation::Institutions(a)), Some(ModelAffiliation::Institutions(b))) => {
            // The overwhelmingly common composite has both halves at one
            // institution, where the union is the identity — and `union` would
            // still take the set interner's mutex to rediscover that. This is a
            // lock removed on the hot path, not a second rule: the two branches
            // produce the same interned handle for equal sets.
            if a == b {
                return Some(ModelAffiliation::Institutions(a));
            }
            Some(ModelAffiliation::Institutions(a.union(b)))
        }
    }
}

#[cfg(test)]
mod credential_message_tests {
    use super::*;
    use crate::config::Config;

    const REMEDY: &str = "Add it under Versa API Bedrock in Settings.";

    /// #521. The file-store wording appears exactly while BIOROUTER_DISABLE_KEYRING
    /// has selected the file store; a keychain-backed store, or a file store
    /// without the variable, keeps today's message word for word.
    #[test]
    fn issue_521_a_missing_key_names_the_file_store_only_while_the_variable_selects_it() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        assert!(crate::config::test_binary_credentials::os_store_is_kept_off());
        let directory = tempfile::tempdir().unwrap();
        let secrets = directory.path().join("secrets.yaml");
        // The process-memory store this test binary uses in place of the OS one.
        let keychain =
            Config::new(directory.path().join("keychain.yaml"), "issue-521-keychain").unwrap();
        let file =
            Config::new_with_file_secrets(directory.path().join("file.yaml"), &secrets).unwrap();
        let unchanged = format!("SYNTHETIC_521_KEY {CREDENTIAL_NEVER_SET}. {REMEDY}");

        {
            let _env = env_lock::lock_env([("BIOROUTER_DISABLE_KEYRING", Some("1"))]);
            assert_eq!(keychain.secrets_file_selected_by_environment(), None);
            assert_eq!(
                credential_never_set(&keychain, "SYNTHETIC_521_KEY", Some(REMEDY)),
                unchanged
            );
            assert_eq!(
                credential_never_set(&keychain, "SYNTHETIC_521_KEY", None),
                format!("SYNTHETIC_521_KEY {CREDENTIAL_NEVER_SET}")
            );

            assert_eq!(
                file.secrets_file_selected_by_environment(),
                Some(secrets.as_path())
            );
            let message = credential_never_set(&file, "SYNTHETIC_521_KEY", Some(REMEDY));
            assert_eq!(
                message,
                format!(
                    "SYNTHETIC_521_KEY {CREDENTIAL_NEVER_SET} in the file store {}. \
                     {KEYCHAIN_DISABLED_REMEDY}",
                    secrets.display()
                )
            );
            assert!(says_credential_never_set(&message));
            assert!(!message.contains(REMEDY), "{message}");
        }
        {
            let _env = env_lock::lock_env([("BIOROUTER_DISABLE_KEYRING", None::<&str>)]);
            assert_eq!(file.secrets_file_selected_by_environment(), None);
            assert_eq!(
                credential_never_set(&file, "SYNTHETIC_521_KEY", Some(REMEDY)),
                unchanged
            );
        }
    }

    /// A key that is there is returned; a missing one is "not configured" in
    /// the file store; a file store that cannot be read is never reported as a
    /// missing key, which would send the person to re-enter a key they have.
    /// No value appears in any message.
    #[test]
    fn issue_521_a_store_that_cannot_be_read_is_not_a_missing_key() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let secrets = directory.path().join("secrets.yaml");
        let file =
            Config::new_with_file_secrets(directory.path().join("file.yaml"), &secrets).unwrap();
        let _env = env_lock::lock_env([
            ("BIOROUTER_DISABLE_KEYRING", Some("1")),
            ("SYNTHETIC_521_KEY", None),
            ("SYNTHETIC_521_OTHER", None),
        ]);

        file.set_secret("SYNTHETIC_521_KEY", &"synthetic-521-value")
            .unwrap();
        assert_eq!(
            read_provider_credential(&file, "SYNTHETIC_521_KEY", Some(REMEDY)).unwrap(),
            "synthetic-521-value"
        );

        let missing = read_provider_credential(&file, "SYNTHETIC_521_OTHER", Some(REMEDY))
            .unwrap_err()
            .to_string();
        assert!(says_credential_never_set(&missing), "{missing}");
        assert!(
            missing.contains(&secrets.display().to_string()),
            "{missing}"
        );
        assert!(!missing.contains("synthetic-521-value"), "{missing}");

        std::fs::remove_file(&secrets).unwrap();
        std::fs::create_dir(&secrets).unwrap();
        let refused = read_provider_credential(&file, "SYNTHETIC_521_KEY", Some(REMEDY))
            .unwrap_err()
            .to_string();
        assert!(!says_credential_never_set(&refused), "{refused}");
        assert!(refused.starts_with("Could not read SYNTHETIC_521_KEY from the file store"));
        assert!(!refused.contains("synthetic-521-value"), "{refused}");
    }
}
