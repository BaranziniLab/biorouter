//! The public `aws_bedrock` provider and UCSF's private `versa_bedrock` must not
//! steer each other. The Bedrock twin of `versa_azure`'s `routing_tests`.
//!
//! Declared in `providers/mod.rs` as
//! `#[cfg(all(test, feature = "aws-providers"))] mod bedrock_namespace_tests;`.
//! `aws-providers` is a default feature, so a plain `cargo test -p biorouter
//! --lib` runs every row here. A `--no-default-features` build compiles neither
//! provider, and this module goes with them.
//!
//! **What they shared.** Versa declared the public card's `AWS_REGION` and an
//! `AWS_ENDPOINT_URL_BEDROCK` key as its own, read both (and then the process
//! environment) as overrides, and its setup surfaces wrote both. `bedrock.rs`
//! exported every `AWS_*` config value and secret into the process environment
//! and promoted `AWS_ENDPOINT_URL_BEDROCK` to the variable the AWS SDK reads, so
//! the UCSF gateway Versa persisted became the public provider's endpoint. And
//! the SDK reads `AWS_BEARER_TOKEN_BEDROCK` from the environment on its own, and
//! authenticates with it instead of signing whenever it is there.
//!
//! **The export is gone** (A1). `bedrock.rs` and `sagemaker_tgi.rs` hand their
//! stored `AWS_*` settings to the client builder instead
//! ([`super::aws_stored_settings`]), because exporting them published the user's
//! real `AWS_SECRET_ACCESS_KEY` to every subprocess — the agent's own shell
//! included — and `std::env::set_var` is unsound here besides. The rows below
//! that pinned the *routing* consequences of the export are unchanged and still
//! pass; `a_process_the_agent_spawns_never_sees_a_stored_aws_secret` pins the
//! credential half.
//!
//! **How each row measures.** Every provider is built the way production builds
//! it, through `from_env`, and only its HTTP transport is then swapped for the
//! SDK's own capture client (`with_http_client`). So every assertion is on the
//! request production would have sent: its host, its path, its `Authorization`
//! header. Nothing leaves the process, even on the rows whose bug aims a request
//! at public AWS. The stand-in is the capture client rather than wiremock
//! because the thing under test is the host the SDK resolved, and aiming the
//! endpoint at a local server would overwrite exactly that.
//!
//! The config rows pin their inputs with `with_config_overrides`, which
//! `get_param` consults before the environment and the file. The environment
//! rows cannot: the SDK reads the environment through its own shim, and what a
//! provider does or does not write to the environment is part of what is under
//! test. Writing it from this multi-threaded binary is unsound, and the writes
//! would leak into every test running beside it. So those rows re-execute this
//! test binary and run in a child process that STARTS with the environment the
//! scenario describes and a config root of its own, as
//! `workflow::local_workflows::tests::listing_workflows_survives_a_deleted_working_directory`
//! does for a deleted working directory.

use super::base::Provider;
use super::bedrock::{BedrockProvider, BEDROCK_DEFAULT_MODEL};
use super::versa_bedrock::{
    VersaBedrockProvider, VERSA_BEDROCK_DEFAULT_ENDPOINT, VERSA_BEDROCK_DEFAULT_MODEL,
    VERSA_BEDROCK_DEFAULT_REGION,
};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::privacy::ProviderTier;
use aws_smithy_http_client::test_util::capture_request;
use std::collections::HashMap;

const VERSA_ACCESS_KEY: &str = "VERSATESTACCESSKEY";
const VERSA_SECRET_KEY: &str = "versa-test-secret-key";
const PUBLIC_ACCESS_KEY: &str = "PUBLICTESTACCESSKEY";
const PUBLIC_SECRET_KEY: &str = "public-test-secret-key";
/// The public card's Bedrock API key, in the variable the AWS SDK reads it from.
const PUBLIC_BEARER_TOKEN: &str = "public-bedrock-api-key";
/// What the public card could point at: its user's own AWS region.
const PUBLIC_ENDPOINT: &str = "https://bedrock-runtime.eu-central-1.amazonaws.com";
const PUBLIC_REGION: &str = "eu-central-1";
const UCSF_GATEWAY_HOST: &str = "unified-api.ucsf.edu";

/// One request, as it would have left the machine.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Sent {
    host: String,
    path: String,
    authorization: String,
}

impl Sent {
    fn of(request: &aws_smithy_runtime_api::client::orchestrator::HttpRequest) -> Self {
        let url = url::Url::parse(request.uri()).expect("the SDK sends an absolute URI");
        Self {
            host: url.host_str().unwrap_or_default().to_string(),
            path: url.path().to_string(),
            authorization: request
                .headers()
                .get("authorization")
                .unwrap_or_default()
                .to_string(),
        }
    }

    /// `(access key id, signing region)`, if the request was SigV4-signed.
    fn signed_by(&self) -> Option<(&str, &str)> {
        let credential = self
            .authorization
            .strip_prefix("AWS4-HMAC-SHA256 Credential=")?;
        let mut scope = credential.split(',').next()?.split('/');
        let key = scope.next()?;
        let _date = scope.next()?;
        let region = scope.next()?;
        Some((key, region))
    }
}

/// What a row observed: for Versa, the endpoint, region and tier the instance
/// resolved; for either provider, the request it sent.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Observed {
    resolved: Option<(String, String, String)>,
    sent: Sent,
}

/// Send one turn and return the request it made. The stand-in answers 200 with
/// an empty body, which does not parse, so the turn fails AFTER the request is
/// made — and the request is what is measured. Every row builds with
/// `BEDROCK_MAX_RETRIES=0`, because the stand-in answers once.
async fn turn(provider: &dyn Provider) {
    let _ = provider
        .complete("system", &[Message::user().with_text("hello")], &[])
        .await;
}

async fn versa_sent(provider: VersaBedrockProvider) -> Sent {
    let (http, captured) = capture_request(None);
    let provider = provider.with_http_client(http);
    turn(&provider).await;
    Sent::of(&captured.expect_request())
}

async fn public_sent(provider: BedrockProvider) -> Sent {
    let (http, captured) = capture_request(None);
    let provider = provider.with_http_client(http);
    turn(&provider).await;
    Sent::of(&captured.expect_request())
}

/// Versa's credentials and Versa's own two overrides as given, blank meaning
/// absent, so the machine running the suite cannot leak its own configuration
/// into what is measured.
fn versa_config(endpoint: &str, region: &str) -> HashMap<String, String> {
    HashMap::from([
        (
            "VERSA_BEDROCK_ACCESS_KEY_ID".into(),
            VERSA_ACCESS_KEY.into(),
        ),
        (
            "VERSA_BEDROCK_SECRET_ACCESS_KEY".into(),
            VERSA_SECRET_KEY.into(),
        ),
        ("VERSA_BEDROCK_ENDPOINT".into(), endpoint.into()),
        ("VERSA_BEDROCK_REGION".into(), region.into()),
        ("BEDROCK_MAX_RETRIES".into(), "0".into()),
    ])
}

async fn versa_bound(overrides: HashMap<String, String>) -> VersaBedrockProvider {
    crate::config::with_config_overrides(
        overrides,
        VersaBedrockProvider::from_env(ModelConfig::new_or_fail(VERSA_BEDROCK_DEFAULT_MODEL)),
    )
    .await
    .unwrap_or_else(|e| panic!("Versa Bedrock must construct from its credentials alone: {e}"))
}

/// The endpoint and region a bound instance will be restored with, and its tier.
fn resolved(provider: &VersaBedrockProvider) -> (String, String, String) {
    let binding = serde_json::to_value(provider.restore_binding()).unwrap();
    (
        binding["endpoint"].as_str().unwrap_or_default().to_string(),
        binding["region"].as_str().unwrap_or_default().to_string(),
        format!("{:?}", provider.tier()),
    )
}

fn shipped() -> (String, String, String) {
    (
        VERSA_BEDROCK_DEFAULT_ENDPOINT.to_string(),
        VERSA_BEDROCK_DEFAULT_REGION.to_string(),
        format!("{:?}", ProviderTier::Private),
    )
}

// ------------------------------------------------------------------ the rule

/// Configuring UCSF's PRIVATE Versa Bedrock must not configure the PUBLIC
/// Amazon Bedrock card, or hand it a value.
///
/// `aws_bedrock` declares two keys, both required and both defaulted, and for
/// such a provider `check_provider_configured` says Configured as soon as EITHER
/// is in `config.yaml`. Versa declared one of them, `AWS_REGION`, and its setup
/// surfaces persisted it: the Settings form seeds a declared key's default and
/// `DefaultSubmitHandler` submits it, and the onboarding card wrote it on every
/// connect. Every other `AWS_*` key belongs to the public side as well, declared
/// or not: `bedrock.rs` and `sagemaker_tgi.rs` export each one into the process
/// environment, where the AWS SDK reads them. So this asserts the namespace, not
/// the one key that happened to leak.
#[test]
fn versa_declares_no_key_the_public_bedrock_provider_reads() {
    let versa = VersaBedrockProvider::metadata();
    let public = BedrockProvider::metadata();
    let public_keys: Vec<&str> = public
        .config_keys
        .iter()
        .map(|key| key.name.as_str())
        .collect();
    assert!(
        public.config_keys.iter().any(|key| key.required),
        "if the public provider stops having a key its configured-check turns on, \
         this test is vacuous; re-derive it rather than deleting it"
    );

    let versa_keys = versa.config_keys.iter().map(|key| key.name.as_str());
    let shared: Vec<&str> = versa_keys
        .clone()
        .filter(|name| public_keys.contains(name))
        .collect();
    let outside: Vec<&str> = versa_keys
        .filter(|name| !name.starts_with("VERSA_BEDROCK_"))
        .collect();
    assert!(
        shared.is_empty() && outside.is_empty(),
        "versa_bedrock declares {shared:?}, which the PUBLIC aws_bedrock provider \
         declares too, so a Versa setup marks that card Configured and hands it the \
         value. It declares {outside:?} outside its own namespace, and every \
         `AWS_*` key is the public providers' too: `bedrock.rs` exports each one \
         into the process environment, where the AWS SDK reads it. Two providers \
         of different privacy tiers must not share a config key."
    );
}

// ------------------------------------------------------ public card → Versa

/// What the public Amazon Bedrock card, or `bedrock.rs`'s export of it, leaves
/// where Versa used to look: its user's own AWS region, and an endpoint in it.
/// Versa read both whenever its own were unset. Its requests, signed with
/// UCSF-issued keys, then went to that user's AWS region, which refused the
/// keys, and the instance turned Public. The region alone was enough to sign
/// every request for a region other than the gateway's.
#[tokio::test]
async fn the_public_cards_endpoint_and_region_never_reach_versa() {
    let mut public_card = versa_config("", "");
    public_card.insert("AWS_ENDPOINT_URL_BEDROCK".into(), PUBLIC_ENDPOINT.into());
    public_card.insert("AWS_REGION".into(), PUBLIC_REGION.into());

    let versa = versa_bound(public_card).await;
    assert_eq!(
        resolved(&versa),
        shipped(),
        "the public Amazon Bedrock card's endpoint or region reached a Versa chat"
    );

    let sent = versa_sent(versa).await;
    assert_eq!(sent.host, UCSF_GATEWAY_HOST, "{sent:?}");
    assert!(sent.path.starts_with("/general/awsai/model/"), "{sent:?}");
    assert_eq!(
        sent.signed_by(),
        Some((VERSA_ACCESS_KEY, VERSA_BEDROCK_DEFAULT_REGION)),
        "{sent:?}"
    );
}

/// The escape hatch survives, in Versa's own namespace. An operator can still
/// repoint the endpoint or the region, a blank value still means the shipped
/// default, and an endpoint off the gateway still demotes the instance: the
/// demotion guards a key anyone can write, not only the public card.
#[tokio::test]
async fn versas_own_overrides_still_steer_it() {
    let blank = versa_bound(versa_config("  ", "")).await;
    assert_eq!(resolved(&blank), shipped(), "blank must mean the default");

    let repointed = "https://unified-api.ucsf.edu/general/awsai-v2";
    let custom = versa_bound(versa_config(repointed, "us-east-2")).await;
    assert_eq!(
        resolved(&custom),
        (
            repointed.to_string(),
            "us-east-2".to_string(),
            format!("{:?}", ProviderTier::Private)
        )
    );
    let sent = versa_sent(custom).await;
    assert_eq!(sent.host, UCSF_GATEWAY_HOST, "{sent:?}");
    assert!(
        sent.path.starts_with("/general/awsai-v2/model/"),
        "{sent:?}"
    );
    assert_eq!(
        sent.signed_by(),
        Some((VERSA_ACCESS_KEY, "us-east-2")),
        "{sent:?}"
    );

    let off_site = versa_bound(versa_config(PUBLIC_ENDPOINT, "")).await;
    assert_eq!(
        resolved(&off_site).2,
        format!("{:?}", ProviderTier::Public),
        "an endpoint off the UCSF gateway must demote the instance"
    );
}

/// The public card's Bedrock API key, alone in the environment, where the AWS
/// SDK's own documentation tells its user to put it.
///
/// This one reaches past Versa's own code. `AWS_BEARER_TOKEN_BEDROCK` is the
/// SDK's variable for a Bedrock API key; the SDK reads it itself and, finding
/// it, authenticates with that bearer token instead of signing. So a Versa chat
/// that looked entirely right, with the UCSF gateway, the gateway's region and a
/// Private tier, sent the public card's API key to UCSF in its `Authorization`
/// header, and Versa's own keys signed nothing.
#[tokio::test]
async fn the_public_cards_api_key_never_rides_on_a_versa_request() {
    const SCENARIO: &str = "versa-beside-a-bedrock-api-key";
    if child_scenario().as_deref() == Some(SCENARIO) {
        report_versa().await;
        return;
    }

    let observed = run_child(
        "the_public_cards_api_key_never_rides_on_a_versa_request",
        SCENARIO,
        "{}\n",
        &[("AWS_BEARER_TOKEN_BEDROCK", PUBLIC_BEARER_TOKEN)],
    );
    assert_signed_by_versa_for_the_gateway(&observed);
}

/// Everything the environment can hold for the PUBLIC side, all at once, and
/// none of it may steer a Versa request: what `bedrock.rs` exports and promotes,
/// what a shell holds for the AWS CLI, and the public card's own credentials.
#[tokio::test]
async fn nothing_in_the_process_environment_steers_versa() {
    const SCENARIO: &str = "versa-in-a-public-environment";
    if child_scenario().as_deref() == Some(SCENARIO) {
        report_versa().await;
        return;
    }

    let observed = run_child(
        "nothing_in_the_process_environment_steers_versa",
        SCENARIO,
        "{}\n",
        &[
            ("AWS_ENDPOINT_URL_BEDROCK", PUBLIC_ENDPOINT),
            ("AWS_ENDPOINT_URL_BEDROCK_RUNTIME", PUBLIC_ENDPOINT),
            ("AWS_REGION", PUBLIC_REGION),
            ("AWS_BEARER_TOKEN_BEDROCK", PUBLIC_BEARER_TOKEN),
            ("AWS_ACCESS_KEY_ID", PUBLIC_ACCESS_KEY),
            ("AWS_SECRET_ACCESS_KEY", PUBLIC_SECRET_KEY),
        ],
    );
    assert_signed_by_versa_for_the_gateway(&observed);
}

/// The child half of the two rows above: a Versa chat bound with its
/// credentials and nothing else, reported with the one request it sent.
async fn report_versa() {
    let versa = versa_bound(versa_config("", "")).await;
    report(Observed {
        resolved: Some(resolved(&versa)),
        sent: versa_sent(versa).await,
    });
}

/// The shipped endpoint, region and tier, and a request to the gateway signed
/// with Versa's own keys for the gateway's region, carrying no bearer token. One
/// comparison, so a failure shows every part of the request at once.
fn assert_signed_by_versa_for_the_gateway(observed: &Observed) {
    let sent = &observed.sent;
    assert_eq!(
        (
            observed.resolved.clone(),
            sent.host.as_str(),
            sent.signed_by(),
        ),
        (
            Some(shipped()),
            UCSF_GATEWAY_HOST,
            Some((VERSA_ACCESS_KEY, VERSA_BEDROCK_DEFAULT_REGION)),
        ),
        "the process environment steered a Versa request: {sent:?}"
    );
    assert!(
        !sent.authorization.contains(PUBLIC_BEARER_TOKEN),
        "the public card's API key rode along on a Versa request: {sent:?}"
    );
}

// ------------------------------------------------------ Versa → public card

/// The other direction. Versa's setup persisted `AWS_ENDPOINT_URL_BEDROCK`
/// pointing at the UCSF gateway, and `bedrock.rs` promoted that key to the
/// variable the SDK reads. So once the PUBLIC provider was built, it sent the
/// user's own AWS-signed requests to UCSF's gateway, which refused them.
/// Existing installs keep that key, so the public provider has to ignore it; it
/// is not enough to stop writing it.
#[tokio::test]
async fn versas_persisted_endpoint_never_becomes_the_public_providers() {
    const SCENARIO: &str = "public-after-a-versa-setup";
    if child_scenario().as_deref() == Some(SCENARIO) {
        report(Observed {
            resolved: None,
            sent: public_sent(public_bound().await).await,
        });
        return;
    }

    // Exactly what the Versa Bedrock onboarding card wrote on every connect.
    let config_yaml = format!(
        "AWS_ENDPOINT_URL_BEDROCK: {VERSA_BEDROCK_DEFAULT_ENDPOINT}\n\
         AWS_REGION: {VERSA_BEDROCK_DEFAULT_REGION}\n"
    );
    let observed = run_child(
        "versas_persisted_endpoint_never_becomes_the_public_providers",
        SCENARIO,
        &config_yaml,
        &[
            ("AWS_ACCESS_KEY_ID", PUBLIC_ACCESS_KEY),
            ("AWS_SECRET_ACCESS_KEY", PUBLIC_SECRET_KEY),
        ],
    );
    let sent = &observed.sent;
    assert_eq!(
        sent.host, "bedrock-runtime.us-west-2.amazonaws.com",
        "Versa's persisted endpoint became the public provider's: {sent:?}"
    );
    assert_eq!(
        sent.signed_by(),
        Some((PUBLIC_ACCESS_KEY, "us-west-2")),
        "{sent:?}"
    );
}

/// …while the public provider still follows the AWS SDK's own variable for
/// this service, which is how a VPC endpoint or a proxy is meant to be set.
#[tokio::test]
async fn the_public_provider_still_follows_the_sdks_endpoint_variable() {
    const SCENARIO: &str = "public-with-its-own-endpoint";
    if child_scenario().as_deref() == Some(SCENARIO) {
        report(Observed {
            resolved: None,
            sent: public_sent(public_bound().await).await,
        });
        return;
    }

    let vpc_endpoint =
        "https://vpce-0123456789abcdef0-abcdefgh.bedrock-runtime.us-west-2.vpce.amazonaws.com";
    let observed = run_child(
        "the_public_provider_still_follows_the_sdks_endpoint_variable",
        SCENARIO,
        "AWS_REGION: us-west-2\n",
        &[
            ("AWS_ACCESS_KEY_ID", PUBLIC_ACCESS_KEY),
            ("AWS_SECRET_ACCESS_KEY", PUBLIC_SECRET_KEY),
            ("AWS_ENDPOINT_URL_BEDROCK_RUNTIME", vpc_endpoint),
        ],
    );
    assert_eq!(
        observed.sent.host,
        url::Url::parse(vpc_endpoint).unwrap().host_str().unwrap(),
        "{:?}",
        observed.sent
    );
}

async fn public_bound() -> BedrockProvider {
    crate::config::with_config_overrides(
        HashMap::from([("BEDROCK_MAX_RETRIES".into(), "0".into())]),
        BedrockProvider::from_env(ModelConfig::new_or_fail(BEDROCK_DEFAULT_MODEL)),
    )
    .await
    .unwrap_or_else(|e| panic!("the public provider must construct from env credentials: {e}"))
}

// -------------------------------------------- the credential never leaves (A1)

/// A secret that exists ONLY in BioRouter's own store. Nothing puts it in the
/// environment, so a process that can read it read it from an export.
const STORE_ONLY_SECRET: &str = "store-only-secret-must-never-be-exported";

/// What a process spawned after the provider was bound could see, and what the
/// provider sent — both, because the fix is only a fix if it keeps working.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Leak {
    /// Every `AWS_*` variable visible to a process this one spawned.
    spawned_env: std::collections::BTreeMap<String, String>,
    sent: Sent,
}

/// **The leak, and the property that closes it.**
///
/// Binding a provider must not publish the user's AWS credentials to every
/// process the agent later spawns. One of those processes is the agent's own
/// `developer__shell`, so before this was fixed a chat on a **public** model
/// could print the user's real `AWS_SECRET_ACCESS_KEY` — a credential the
/// privacy lattice never sees, because its gates decide which model may read a
/// conversation, not what a shell may read out of its own environment.
///
/// The scenario is the one an installed user is actually in: the credentials
/// live in BioRouter's credential store and **nowhere else**. The child starts
/// with no `AWS_*` variables at all (the harness scrubs them), binds the public
/// provider exactly as production does, and only then spawns a grandchild —
/// which inherits whatever binding the provider left behind.
///
/// **Fail-before evidence.** With `bedrock.rs`'s `set_aws_env_vars` closure in
/// place this row fails on its first assertion, reporting
/// `AWS_SECRET_ACCESS_KEY` among `spawned_env`.
///
/// The second half matters as much as the first: the same credential must still
/// reach the SDK. A fix that merely stopped exporting would break every install
/// whose keys live in the store, and would pass an assertion that only looked
/// for absence. So the row also measures the request the provider signed.
#[tokio::test]
async fn a_process_the_agent_spawns_never_sees_a_stored_aws_secret() {
    const SCENARIO: &str = "public-then-spawn";
    const SPAWNED: &str = "public-then-spawn/spawned";

    // Innermost: the process standing in for anything the agent starts. It
    // reports the AWS view of its own environment and nothing else.
    if child_scenario().as_deref() == Some(SPAWNED) {
        report_as(&aws_environment());
        return;
    }

    if child_scenario().as_deref() == Some(SCENARIO) {
        let sent = public_sent(public_bound().await).await;
        report_as(&Leak {
            spawned_env: spawn_and_read_environment(
                "a_process_the_agent_spawns_never_sees_a_stored_aws_secret",
                SPAWNED,
            ),
            sent,
        });
        return;
    }

    let leaked: Leak = run_child_reporting(
        "a_process_the_agent_spawns_never_sees_a_stored_aws_secret",
        SCENARIO,
        "AWS_REGION: us-west-2\n",
        Some(&format!(
            "AWS_ACCESS_KEY_ID: {PUBLIC_ACCESS_KEY}\n\
             AWS_SECRET_ACCESS_KEY: {STORE_ONLY_SECRET}\n"
        )),
        &[],
    );

    assert!(
        !leaked
            .spawned_env
            .values()
            .any(|value| value == STORE_ONLY_SECRET),
        "binding the provider published the user's AWS secret to every process \
         it spawns afterwards — `developer__shell` included: {:?}",
        leaked.spawned_env
    );
    assert!(
        leaked.spawned_env.is_empty(),
        "binding the provider must not write ANY `AWS_*` variable into the \
         process environment: {:?}",
        leaked.spawned_env
    );
    assert_eq!(
        leaked.sent.signed_by(),
        Some((PUBLIC_ACCESS_KEY, "us-west-2")),
        "the stored credential must still reach the SDK: {:?}",
        leaked.sent
    );
}

/// The same store, and an `AWS_BEARER_TOKEN_BEDROCK` in the environment beside
/// it: the scheme the SDK picks must not change.
///
/// The SDK reads that variable itself and prefers bearer auth over signing
/// unless a scheme was chosen in code. Under the export the stored keys landed
/// in the environment, where the bearer token still outranked them — so a fix
/// that pinned `sigv4` "while it was in there" would change which credential an
/// existing install authenticates with. `versa_bedrock` pins it because its
/// endpoint and keys are institutional; the public card must not.
#[tokio::test]
async fn a_stored_credential_does_not_change_which_auth_scheme_the_sdk_picks() {
    const SCENARIO: &str = "public-store-beside-a-bearer-token";
    if child_scenario().as_deref() == Some(SCENARIO) {
        report(Observed {
            resolved: None,
            sent: public_sent(public_bound().await).await,
        });
        return;
    }

    let observed = run_child_reporting::<Observed>(
        "a_stored_credential_does_not_change_which_auth_scheme_the_sdk_picks",
        SCENARIO,
        "AWS_REGION: us-west-2\n",
        Some(&format!(
            "AWS_ACCESS_KEY_ID: {PUBLIC_ACCESS_KEY}\n\
             AWS_SECRET_ACCESS_KEY: {STORE_ONLY_SECRET}\n"
        )),
        &[("AWS_BEARER_TOKEN_BEDROCK", PUBLIC_BEARER_TOKEN)],
    );
    assert!(
        observed.sent.authorization.contains(PUBLIC_BEARER_TOKEN)
            && observed.sent.signed_by().is_none(),
        "the environment's bearer token still wins the scheme, as it did when \
         the stored keys were exported into the environment beside it: {:?}",
        observed.sent
    );
}

/// The `AWS_*` variables this process can see, which is exactly what it would
/// pass to anything it spawns.
fn aws_environment() -> std::collections::BTreeMap<String, String> {
    std::env::vars()
        .filter(|(name, _)| name.starts_with("AWS_"))
        // The harness sets these three itself to isolate the child from the
        // developer's own AWS profile files and from instance metadata. They are
        // the test rig, not anything the provider wrote.
        .filter(|(name, _)| {
            !matches!(
                name.as_str(),
                "AWS_CONFIG_FILE" | "AWS_SHARED_CREDENTIALS_FILE" | "AWS_EC2_METADATA_DISABLED"
            )
        })
        .collect()
}

/// Spawn one more copy of this binary — the stand-in for anything the agent
/// starts — and read back the AWS view of the environment it inherited.
///
/// Re-executing the test binary rather than running a shell keeps this row
/// working on Windows, where the workspace's `--lib` job also runs.
fn spawn_and_read_environment(
    test: &str,
    scenario: &str,
) -> std::collections::BTreeMap<String, String> {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "--nocapture",
            &format!("providers::bedrock_namespace_tests::{test}"),
        ])
        // Nothing else is set: the whole measurement is what this process
        // passes on by inheritance.
        .env(CHILD, scenario)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(REPORT))
        .unwrap_or_else(|| {
            panic!(
                "the spawned half of `{test}` reported nothing.\n--- stdout ---\n{stdout}\n\
                 --- stderr ---\n{}",
                String::from_utf8_lossy(&output.stderr)
            )
        });
    serde_json::from_str(line).unwrap()
}

/// Neither public AWS provider may write the process environment again.
///
/// The closure this replaced was copied verbatim from one file into the other,
/// which is how one review missed it twice. A grep over both sources is the
/// only assertion that survives a third copy.
#[test]
fn neither_aws_provider_exports_anything_into_the_environment() {
    // Assembled, not written, so this line cannot match itself.
    let needle = concat!("set_", "var");
    for (name, source) in [
        ("bedrock.rs", include_str!("bedrock.rs")),
        ("sagemaker_tgi.rs", include_str!("sagemaker_tgi.rs")),
    ] {
        let writes = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .filter(|line| line.contains(needle))
            .count();
        assert_eq!(
            writes, 0,
            "{name} writes the process environment; hand the value to the \
             client builder instead (see `providers::aws_stored_settings`)"
        );
    }
}

// ------------------------------------------------------------ child process

/// Names the scenario a re-executed copy of this binary is to run. A
/// test-private key: no production reader resolves it.
const CHILD: &str = "BIOROUTER_TEST_BEDROCK_NAMESPACE_CHILD";
const REPORT: &str = "BEDROCK_NAMESPACE_OBSERVED ";

fn child_scenario() -> Option<String> {
    std::env::var(CHILD).ok()
}

fn report(observed: Observed) {
    report_as(&observed);
}

fn report_as<T: serde::Serialize>(observed: &T) {
    println!("{REPORT}{}", serde_json::to_string(observed).unwrap());
}

/// Re-run `test`, a test in this module, as a child process whose half of the
/// test runs `scenario`. It starts with `env` and with none of the AWS, Versa or
/// Bedrock settings this process inherited, over a config root of its own that
/// holds `config_yaml`, with no AWS profile files and no instance metadata.
fn run_child(test: &str, scenario: &str, config_yaml: &str, env: &[(&str, &str)]) -> Observed {
    run_child_reporting(test, scenario, config_yaml, None, env)
}

/// [`run_child`], for a scenario that reports something other than [`Observed`]
/// and may seed the child's **secret** store as well as its config file.
///
/// `secrets_yaml` lands at `<root>/config/secrets.yaml`, which is where
/// `Config::all_secrets` reads under `BIOROUTER_DISABLE_KEYRING=true` — the one
/// way a test can put a value in front of the code paths that handle real
/// credentials without touching the developer's own Keychain.
fn run_child_reporting<T: serde::de::DeserializeOwned>(
    test: &str,
    scenario: &str,
    config_yaml: &str,
    secrets_yaml: Option<&str>,
    env: &[(&str, &str)],
) -> T {
    // A child that reached a parent half would spawn its own child, and so on:
    // stop at the first one rather than fork without end.
    assert!(
        child_scenario().is_none(),
        "the child half of `{test}` did not claim scenario `{scenario}`"
    );
    let root = tempfile::tempdir().unwrap();
    let config_dir = root.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.yaml"), config_yaml).unwrap();
    if let Some(secrets) = secrets_yaml {
        std::fs::write(config_dir.join("secrets.yaml"), secrets).unwrap();
    }

    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "--nocapture",
        &format!("providers::bedrock_namespace_tests::{test}"),
    ]);
    for (name, _) in std::env::vars_os() {
        let name = name.to_string_lossy();
        if ["AWS_", "VERSA_", "BEDROCK_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            command.env_remove(name.as_ref());
        }
    }
    let output = command
        .env(CHILD, scenario)
        .env("BIOROUTER_PATH_ROOT", root.path())
        .env("BIOROUTER_DISABLE_KEYRING", "true")
        .env("AWS_CONFIG_FILE", root.path().join("aws-config"))
        .env(
            "AWS_SHARED_CREDENTIALS_FILE",
            root.path().join("aws-credentials"),
        )
        .env("AWS_EC2_METADATA_DISABLED", "true")
        .envs(env.iter().copied())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(REPORT))
        .unwrap_or_else(|| {
            panic!(
                "the child half of `{test}` reported nothing.\n\
                 --- child stdout ---\n{stdout}\n--- child stderr ---\n{stderr}"
            )
        });
    assert!(
        output.status.success(),
        "the child half of `{test}` failed.\n--- child stdout ---\n{stdout}\n\
         --- child stderr ---\n{stderr}"
    );
    serde_json::from_str(line).unwrap()
}
