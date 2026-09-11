//! Task 5 (issue #56): the private set, and the two demotion rules behind
//! [`Provider::tier`].
//!
//! Declared in `providers/mod.rs` as `#[cfg(test)] mod tier_tests;` — **not**
//! `include!`, or the filter `providers::tier_tests` resolves to nothing and
//! prints `0 passed`.

use async_trait::async_trait;
use rmcp::model::Tool;
use std::sync::Arc;

use super::base::{Provider, ProviderMetadata, ProviderUsage};
use super::errors::ProviderError;
use super::lead_worker::LeadWorkerProvider;
use super::ollama::{OllamaProvider, OLLAMA_HOST};
use super::versa_azure::VERSA_AZURE_ENDPOINT;
use crate::config::declarative_providers::{DeclarativeProviderConfig, ProviderEngine};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::privacy::ProviderTier;

/// The tier a freshly-installed Biorouter publishes for `name`: the real
/// registry entry's metadata, which is exactly what the settings grid and every
/// other UI surface reads. A name with no entry gets the fail-safe default,
/// which is the whole point of the default being Public.
async fn tier_for_name_at_default_config(name: &str) -> ProviderTier {
    super::providers()
        .await
        .into_iter()
        .find(|(metadata, _)| metadata.name == name)
        .map(|(metadata, _)| metadata.tier)
        .unwrap_or_default()
}

/// Every registered provider whose shipped metadata claims Private.
async fn private_names_in_the_registry() -> Vec<String> {
    let mut names: Vec<String> = super::providers()
        .await
        .into_iter()
        .filter(|(metadata, _)| metadata.tier == ProviderTier::Private)
        .map(|(metadata, _)| metadata.name)
        .collect();
    names.sort();
    names
}

/// The contents of one writable `custom_providers/*.json` file: a self-hosted
/// engine, named and pointed wherever the caller says. Registering by
/// `config.name` after the built-ins is what lets one such file shadow the real
/// registry entry, so the name is a parameter here on purpose.
fn declarative_config(name: &str, base_url: &str) -> DeclarativeProviderConfig {
    DeclarativeProviderConfig {
        name: name.to_string(),
        engine: ProviderEngine::Ollama,
        display_name: "Not Versa At All".to_string(),
        description: None,
        api_key_env: "NOT_USED".to_string(),
        base_url: base_url.to_string(),
        models: vec![],
        headers: None,
        timeout_seconds: None,
        supports_streaming: None,
    }
}

/// A **real** self-hosted-engine provider built the way a declarative JSON file
/// builds one.
fn self_hosted_named(name: &str, base_url: &str) -> OllamaProvider {
    OllamaProvider::from_custom_config(
        ModelConfig::new_or_fail("qwen3"),
        declarative_config(name, base_url),
    )
    .expect("a declarative ollama provider must construct")
}

fn tier_for_self_hosted_base(base_url: &str) -> ProviderTier {
    self_hosted_named("versa_azure", base_url).tier()
}

/// One half of the composite below, at the requested tier — a **real**
/// provider, not a mock.
///
/// A mock would have to override `tier()`, and this file would then hold a
/// seventh implementation of it — which is what Step 5's enumerating gate
/// lists. That gate names the six production implementations and would need a
/// human to read past a test one every run, so it is deliberately not spelled
/// out here either. A real Ollama-engine provider's tier is a
/// function of the base URL it resolved, so loopback yields Private and a
/// remote host yields Public — pinned independently by
/// `a_self_hosted_provider_pointed_off_the_machine_is_not_private` below, and
/// asserted here too so a break in that mapping is named rather than
/// mis-attributed to the composite rule.
fn half(name: &str, tier: ProviderTier) -> Arc<dyn Provider> {
    let base_url = match tier {
        ProviderTier::Private => "http://localhost:11434",
        ProviderTier::Public => "https://api.example-saas.com",
    };
    let provider = self_hosted_named(name, base_url);
    assert_eq!(
        provider.tier(),
        tier,
        "the {name} half must actually resolve {tier:?}"
    );
    Arc::new(provider)
}

/// The real production composite, with a lead named `versa_azure`.
fn lead_worker_with_tiers(lead: ProviderTier, worker: ProviderTier) -> LeadWorkerProvider {
    LeadWorkerProvider::new(half("versa_azure", lead), half("anthropic", worker), None)
}

/// The production predicate both versa providers hand their resolved endpoint.
fn versa_tier_for_endpoint(endpoint: &str) -> ProviderTier {
    super::ucsf_gateway_tier(endpoint)
}

/// **The private set is a table of reviewed decisions, not a count** (issue #56,
/// Task 56 Step 4).
///
/// This was `the_private_set_is_the_four_the_operator_named`, and it closed the
/// dangerous direction by *cardinality*: the registry's private names had to
/// equal a hardcoded list of four. That works exactly once. A fifth private
/// provider — and the operator ruling of 2026-08-04 says there will be more,
/// possibly under other institutions — arrives as an arithmetic failure with
/// nothing to write down, so the repair is to edit a number rather than to
/// record a decision. A gate whose repair teaches nothing is a gate people
/// delete.
///
/// The shape Task 53 already uses for the tier census replaces it: every
/// provider appears in a table **with a reason**, and completeness is what is
/// asserted. Adding a fifth private provider is then a reviewed row next to four
/// others that each say why, which is the conversation the count was trying to
/// force and could not have.
///
/// ⚠ **This still closes the direction that leaks.** A provider that starts
/// claiming Private with no row fails here — the loop below is over the LIVE
/// registry, so it cannot be satisfied by editing this file alone. What changed
/// is only that the fix is a sentence rather than an increment.
///
/// ⚠ **It reads `factory`'s table rather than keeping its own.** Two lists of
/// which providers are Private is one to forget, and the forgotten one is the
/// guard.
#[tokio::test]
async fn the_private_set_is_a_table_of_reviewed_decisions() {
    use crate::privacy::ProviderTier::{Private, Public};

    let decided = super::factory::tests::private_tier_providers();
    assert!(
        !decided.is_empty(),
        "the tier table is empty, so every assertion below is vacuous"
    );

    for (name, why) in &decided {
        assert_eq!(
            tier_for_name_at_default_config(name).await,
            Private,
            "{name} is filed as private but does not ship Private"
        );
        // A row with no reason records nothing, which is the whole content of
        // this shape.
        assert!(!why.is_empty(), "{name} is private for no stated reason");
    }
    // Everything hosted by an AI company or a large cloud is public — including
    // the ones whose names look institutional. azure.rs ships the UCSF gateway
    // as AZURE_OPENAI_ENDPOINT's default, so a name-keyed rule would call
    // azure_openai Private; it must not.
    for name in [
        "anthropic",
        "openai",
        "azure_openai",
        "bedrock",
        "aws_bedrock",
        "databricks",
        "vertex",
        "google",
        "groq",
        "unknown_provider",
    ] {
        assert_eq!(
            tier_for_name_at_default_config(name).await,
            Public,
            "{name}"
        );
    }
    // ...and the set is CLOSED BY COMPLETENESS: a provider that claims Private
    // anywhere in the tree and has no row fails here, which the loop above
    // cannot do on its own. The failure names the provider and asks for the
    // decision, rather than reporting that a number is off by one.
    for name in private_names_in_the_registry().await {
        assert!(
            decided.iter().any(|(n, _why)| *n == name),
            "{name} ships ProviderTier::Private and is in no tier table. Declaring a provider \
             private is a decision about what a private session may be bound to, so it needs a \
             row saying why, in `private_tier_providers` in providers/factory.rs, beside the \
             ones already there, which each name the predicate or endpoint their tier is decided \
             from."
        );
    }

    // The registry above is the type-level claim every UI surface reads. Cross
    // -check it against a real instance at the shipped default host, where the
    // two must agree.
    assert_eq!(tier_for_self_hosted_base(OLLAMA_HOST), Private);
    // llamacpp's equivalent lives in `llamacpp.rs`, not here: `from_env` reads
    // `LLAMACPP_EXTERNAL_HOST` from the developer's real config, so calling it
    // would assert Private on a default machine and fail on one that
    // legitimately points at a lab box. That test covers all three arms from a
    // struct literal instead of the one the current environment happens to
    // produce.
}

#[tokio::test]
async fn a_composite_takes_the_least_privileged_of_its_two_halves() {
    use crate::privacy::ProviderTier::{Private, Public};
    // get_name() on a composite returns the LEAD's name, so a name-keyed tier
    // would badge private-lead/public-worker Private — the exact inverse of R2.
    let lw = lead_worker_with_tiers(Private, Public);
    assert_eq!(lw.get_name(), "versa_azure"); // the lead's name
    assert_eq!(lw.tier(), Public); // least(), not the name
    assert_eq!(lead_worker_with_tiers(Private, Private).tier(), Private);
    assert_eq!(lead_worker_with_tiers(Public, Public).tier(), Public);
}

#[test]
fn a_self_hosted_provider_pointed_off_the_machine_is_not_private() {
    use crate::privacy::ProviderTier::{Private, Public};
    // Open question 5 rates this ergonomics. It is a live bypass: config.yaml
    // is agent-writable (§9.3 C1 concedes SecretGuard cannot stop `shell`
    // writing it), and a declarative provider file whose engine is Ollama
    // yields an OllamaProvider with an arbitrary base_url. See the two
    // `declarative_providers` rows in this task's Files table for the anchors —
    // they are NOT repeated here, because a line number inside a code comment
    // is a citation no gate can check and no reviewer re-verifies.
    // Anyone who can write one JSON file would otherwise mint a Private-tier
    // provider pointing anywhere.
    assert_eq!(tier_for_self_hosted_base("http://localhost:11434"), Private);
    assert_eq!(tier_for_self_hosted_base("http://127.0.0.1:11434"), Private);
    assert_eq!(tier_for_self_hosted_base("http://[::1]:11434"), Private);
    assert_eq!(
        tier_for_self_hosted_base("http://gpu.lab.ucsf.edu:11434"),
        Public
    );
    assert_eq!(
        tier_for_self_hosted_base("https://api.example-saas.com"),
        Public
    );
}

#[test]
fn only_a_loopback_host_reads_as_this_machine() {
    use super::is_loopback_host;
    // The predicate behind both self-hosted providers, at its edges. Every case
    // below is one config field away from the one above it, and the direction
    // each one errs in is the whole argument for the rule.
    assert!(is_loopback_host("http://localhost:11434"));
    assert!(is_loopback_host("http://127.0.0.1:11434"));
    assert!(is_loopback_host("http://[::1]:11434"));
    // Hosts are case-insensitive; `host_of` lower-cases before comparing.
    assert!(is_loopback_host("http://LOCALHOST:11434"));

    // RFC 6761 §6.3 reserves the `.localhost` TLD for loopback, and both macOS
    // and systemd-resolved honour it. Accepting it is a deliberate ergonomic
    // concession with a residual assumption attached: unlike `localhost`, which
    // is in every /etc/hosts, a `*.localhost` name is only loopback because a
    // resolver says so, and an attacker who controls DNS but not /etc/hosts
    // could answer for it. That is a strictly harder position than the one this
    // rule defends against (a single writable JSON file), so the arm stays —
    // but it is the first thing to remove if that ever stops being true.
    assert!(is_loopback_host("http://sidecar.localhost:11434"));
    // ...and only as a TLD. These are what an attacker actually writes:
    assert!(!is_loopback_host("http://localhost.evil.example/"));
    assert!(!is_loopback_host("http://notlocalhost/"));
    assert!(!is_loopback_host("http://127.0.0.1.evil.example/"));
    assert!(!is_loopback_host("http://gpu.lab.ucsf.edu:11434"));

    // Fail-SAFE, not fail-open. These three genuinely are this machine and
    // still read Public, which costs a user with an unusual setup a private
    // badge and costs nobody a transcript. Do not "fix" them by broadening the
    // predicate without re-reading the paragraph above: every spelling added
    // here is a spelling an attacker may also write.
    assert!(!is_loopback_host("http://127.0.0.2:11434"));
    assert!(!is_loopback_host("http://[::ffff:127.0.0.1]:11434"));
    // A bare host:port parses as a scheme with an opaque path and has no host
    // at all. `OllamaProvider` normalises one to `http://…` before storing it,
    // so this shape cannot reach `tier()` from that path — but the predicate
    // is the thing under test, and it answers Public for anything hostless.
    assert!(!is_loopback_host("localhost:11434"));
    assert!(!is_loopback_host(""));
}

#[test]
fn versa_demotes_when_its_endpoint_is_not_the_ucsf_gateway() {
    use crate::privacy::ProviderTier::{Private, Public};
    // versa_azure's endpoint is user-writable config (VERSA_AZURE_ENDPOINT;
    // until 2026-09-11 also the public azure_openai card's
    // AZURE_OPENAI_ENDPOINT), and versa_bedrock falls back to
    // AWS_ENDPOINT_URL_BEDROCK_RUNTIME, which bedrock.rs sets PROCESS-GLOBALLY
    // with std::env::set_var. The shipped constants are asserted rather than
    // their text, so moving a default off the gateway fails here too.
    assert_eq!(versa_tier_for_endpoint(VERSA_AZURE_ENDPOINT), Private);
    assert_eq!(
        versa_tier_for_endpoint("https://unified-api.ucsf.edu/general"),
        Private
    );
    #[cfg(feature = "aws-providers")]
    assert_eq!(
        versa_tier_for_endpoint(super::versa_bedrock::VERSA_BEDROCK_DEFAULT_ENDPOINT),
        Private
    );
    assert_eq!(
        versa_tier_for_endpoint("https://unified-api.ucsf.edu/general/awsai"),
        Private
    );
    assert_eq!(
        versa_tier_for_endpoint("https://evil.example.com/general"),
        Public
    );
    // The comparison is on the HOST, so a name that merely contains the
    // gateway's does not pass. Both of these are one typo away in a config
    // field a user can edit.
    assert_eq!(
        versa_tier_for_endpoint("https://unified-api.ucsf.edu.evil.example/general"),
        Public
    );
    assert_eq!(
        versa_tier_for_endpoint("https://evil.example/unified-api.ucsf.edu"),
        Public
    );
    // ...and hosts are case-insensitive, so the shipped host in a different
    // case is still the gateway rather than a silent demotion.
    assert_eq!(
        versa_tier_for_endpoint("https://UNIFIED-API.UCSF.EDU/general"),
        Private
    );
    // Anything `url::Url` cannot parse has no host to vouch for. Public.
    assert_eq!(versa_tier_for_endpoint("unified-api.ucsf.edu"), Public);
    assert_eq!(versa_tier_for_endpoint(""), Public);
}

// ------------------------------------------------ The fail-safe default itself

/// A provider that declares no tier at all. Not a mock of any real provider —
/// it exists so the **default** can be asserted directly, which is the claim
/// every public provider in the registry rests on: they are Public because they
/// never say otherwise, not because someone wrote `with_tier(Public)` once per
/// module. `factory::every_registered_provider_is_classified_for_tier` is the
/// census that makes each of those silences a recorded decision.
///
/// `affiliation_tests.rs` has a near-identical `ProviderThatSaysNothing`, and
/// the duplication is deliberate: making that one reachable from here would
/// mean widening a test type's visibility across two modules so that a
/// *different* axis could borrow it, and the tier default would then be pinned
/// in a file whose subject is affiliation.
struct ProviderThatDeclaresNoTier;

#[async_trait]
impl Provider for ProviderThatDeclaresNoTier {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "declares_no_tier"
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("nothing")
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        unreachable!("this provider exists to be asked its tier and nothing else")
    }
}

/// Task 53 Step 1 — the operator's rule, made mechanical: *"if a provider
/// cannot be verified, then it is public (always assume the least
/// permission)."*
///
/// Public is the **least-permission** answer, not the unsafe one: a model
/// tagged Public is *refused* private data. The two mistakes are not
/// symmetric. A genuinely private provider tagged Public is over-restricted —
/// a usability loss. A genuinely public provider tagged Private is handed PHI
/// — a disclosure. Only the second is dangerous, and
/// `the_private_set_is_a_table_of_reviewed_decisions` above is what closes it:
/// declaring a *new* provider Private fails there until someone adds a row
/// saying why it may be.
///
/// So this test exists to stop the default from being "helpfully" inverted —
/// by a future refactor that infers Private from a local-looking base URL, a
/// familiar hostname, or a `runs_locally` flag. Both levels are pinned,
/// because the type-level claim and the instance-level one are read by
/// different code and can be changed independently:
///
/// * [`ProviderMetadata::tier`] is what `GET /config/providers` serves to
///   every UI surface, and what `the_private_set_is_a_table_of_reviewed_decisions`
///   enumerates.
/// * [`Provider::tier`] is what the enforcement path reads.
#[test]
fn a_provider_that_declares_no_tier_is_public() {
    use crate::privacy::ProviderTier::Public;

    // (a) The enum's own default, which both levels below are spelled in terms
    // of. Everything else here is downstream of this one line.
    assert_eq!(ProviderTier::default(), Public);

    // (b) The trait default — the value the enforcement path reads for a
    // provider module that never overrides `tier()`.
    assert_eq!(ProviderThatDeclaresNoTier.tier(), Public);

    // (c) The metadata default, through every constructor a provider module
    // can reach for. `with_tier` is the only way to leave Public, and a
    // constructor that forgot to initialise the field would not compile — but
    // one that initialised it to `Private` would, and would be invisible.
    // ⚠ These are the *constructors*; one more site builds a `ProviderMetadata`
    // literal without going through them —
    // `ProviderRegistry::register_with_name`, the declarative path — and it is
    // pinned separately by
    // `a_declaratively_registered_provider_publishes_public_metadata` below.
    assert_eq!(ProviderMetadata::empty().tier, Public);
    assert_eq!(
        ProviderMetadata::new("p", "P", "d", "m", vec!["m"], "link", vec![]).tier,
        Public
    );
    assert_eq!(
        ProviderMetadata::with_models("p", "P", "d", "m", vec![], "link", vec![]).tier,
        Public
    );

    // (d) The *deserialisation* default. `tier` is `#[serde(default)]`, so
    // metadata arriving without the field — an older daemon's response, a
    // hand-written declarative fixture — must land on Public rather than
    // failing open. Built by round-tripping a real value with the key removed,
    // so adding a field to the struct cannot silently turn this into a
    // different test.
    let mut json = serde_json::to_value(ProviderMetadata::empty()).expect("metadata serialises");
    assert!(
        json.as_object_mut()
            .expect("metadata is a JSON object")
            .remove("tier")
            .is_some(),
        "the field this test is about must be present before it is removed"
    );
    let without_tier: ProviderMetadata =
        serde_json::from_value(json).expect("metadata without a tier still deserialises");
    assert_eq!(without_tier.tier, Public);
}

/// The fifth site the test above cannot reach. The declarative path —
/// `ProviderRegistry::register_with_name` — builds its own `ProviderMetadata`
/// literal, so it neither inherits the borrowed engine's shipped tier nor
/// passes through any of the three constructors.
///
/// That distinction is load-bearing rather than incidental. `custom_providers/`
/// is a directory of writable JSON files — §9.3 C1 concedes `SecretGuard`
/// cannot stop `shell` writing one — and each file names an *engine* to borrow
/// and a *name* to register under, built-in names included. Inheriting the
/// engine's metadata would let one such file publish a Private badge for an
/// endpoint of the author's choosing, which is the disclosure direction: a
/// public provider tagged Private is handed PHI.
///
/// The instance is a separate question and stays honest either way — it
/// computes its tier from the base URL it actually resolved, which is
/// `a_self_hosted_provider_pointed_off_the_machine_is_not_private` above. This
/// test is about the *published* claim, the one `GET /config/providers` serves
/// to every UI surface, which is a claim about a name and must not be mintable
/// from a file.
#[test]
fn a_declaratively_registered_provider_publishes_public_metadata() {
    use super::base::ProviderType;
    use super::provider_registry::ProviderRegistry;
    use crate::config::declarative_providers::register_declarative_provider;
    use crate::privacy::ProviderTier::{Private, Public};

    // Guard the premise. The engine borrowed below ships Private, so inheriting
    // `base_metadata.tier` would be *observable* here rather than a no-op — if
    // ollama ever stops shipping Private this test silently stops discriminating
    // and this assertion is what says so.
    assert_eq!(OllamaProvider::metadata().tier, Private);

    // Registered under a built-in's name, pointed off this machine: the shape
    // the rule exists to refuse.
    let mut registry = ProviderRegistry::new();
    register_declarative_provider(
        &mut registry,
        declarative_config("versa_azure", "https://api.example-saas.com"),
        ProviderType::Builtin,
    );
    let published = |registry: &ProviderRegistry| {
        registry
            .all_metadata_with_types()
            .into_iter()
            .find(|(metadata, _)| metadata.name == "versa_azure")
            .expect("the declarative entry registers under the name its file gives")
            .0
            .tier
    };
    assert_eq!(published(&registry), Public);

    // ...and still Public when the file points at loopback, where the *instance*
    // legitimately resolves Private. The two are not the same claim: the
    // instance's tier is computed from what it reached, the metadata's is
    // asserted about a name, and only the first is checkable.
    let mut loopback_registry = ProviderRegistry::new();
    register_declarative_provider(
        &mut loopback_registry,
        declarative_config("versa_azure", "http://localhost:11434"),
        ProviderType::Builtin,
    );
    assert_eq!(
        self_hosted_named("versa_azure", "http://localhost:11434").tier(),
        Private,
        "the instance half of this test must actually resolve Private, or the \
         assertion below is comparing Public against Public"
    );
    assert_eq!(published(&loopback_registry), Public);
}
