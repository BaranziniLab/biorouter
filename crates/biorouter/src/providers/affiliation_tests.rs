//! Task 46 (issue #56, DR-26): which institution's agreements cover the model a
//! session is bound to, and the two predicates that decide it.
//!
//! Affiliation is a **sibling of [`Provider::tier`]**, not a member of
//! `privacy/`: tier is a trait method on `Provider` whose deciding logic lives
//! in three free functions in `providers/mod.rs`, and affiliation answers the
//! second question — *under whose agreements?* — from the same inputs. The
//! vocabulary it answers in lives in `privacy::affiliation`; the *decision*
//! lives here, beside the tier decision it must never contradict.
//!
//! Declared in `providers/mod.rs` as `#[cfg(test)] mod affiliation_tests;` —
//! **not** `include!`, or the filter `providers::affiliation_tests` resolves to
//! nothing and prints `0 passed`.
//!
//! The **completeness** half of this task is deliberately not here. It
//! enumerates what `factory::register_builtin_providers` registers, so it sits
//! directly beneath that function in `providers/factory.rs`, where the person
//! adding a provider is already looking.

use async_trait::async_trait;
use rmcp::model::Tool;
use std::sync::Arc;

use super::base::{Provider, ProviderMetadata, ProviderUsage};
use super::errors::ProviderError;
use super::lead_worker::LeadWorkerProvider;
use super::ollama::{OllamaProvider, OLLAMA_HOST};
use super::versa_azure::VERSA_AZURE_ENDPOINT;
use super::{self_hosted_affiliation, ucsf_gateway_affiliation};
use crate::config::declarative_providers::{DeclarativeProviderConfig, ProviderEngine};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::privacy::affiliation::{
    compatible, ExtensionAffiliation, InstitutionId, ModelAffiliation,
};
use crate::privacy::ProviderTier;

fn ucsf() -> ModelAffiliation {
    ModelAffiliation::institution(InstitutionId::new("ucsf"))
}

/// A **real** self-hosted-engine provider built the way a declarative JSON file
/// builds one, pointed wherever the caller says. The same construction
/// `tier_tests.rs` uses, and for the same reason: `from_env` would read the
/// developer's own `OLLAMA_HOST`, so a test through it asserts one thing on this
/// machine and another on a colleague's.
fn self_hosted_at(base_url: &str) -> OllamaProvider {
    let config = DeclarativeProviderConfig {
        name: "ollama".to_string(),
        engine: ProviderEngine::Ollama,
        display_name: "Not Versa At All".to_string(),
        description: None,
        api_key_env: "NOT_USED".to_string(),
        base_url: base_url.to_string(),
        models: vec![],
        headers: None,
        timeout_seconds: None,
        supports_streaming: None,
    };
    OllamaProvider::from_custom_config(ModelConfig::new_or_fail("qwen3"), config)
        .expect("a declarative ollama provider must construct")
}

// ------------------------------------------------------------------ Versa/UCSF

/// The assignment DR-26 names: both Versa providers are `Institution("ucsf")`.
///
/// Asserted against the **shipped constants**, not their text, so moving a
/// default endpoint off the gateway fails here rather than silently keeping the
/// badge.
#[test]
fn the_ucsf_gateway_is_what_makes_a_versa_provider_ucsf() {
    assert_eq!(ucsf_gateway_affiliation(VERSA_AZURE_ENDPOINT), Some(ucsf()));
    #[cfg(feature = "aws-providers")]
    assert_eq!(
        ucsf_gateway_affiliation(super::versa_bedrock::VERSA_BEDROCK_DEFAULT_ENDPOINT),
        Some(ucsf())
    );
    assert_eq!(
        ucsf_gateway_affiliation("https://unified-api.ucsf.edu/general/awsai"),
        Some(ucsf())
    );
    // Hosts are case-insensitive, exactly as the tier predicate treats them.
    assert_eq!(
        ucsf_gateway_affiliation("https://UNIFIED-API.UCSF.EDU/general"),
        Some(ucsf())
    );
}

/// The reason affiliation is derived from the gateway host rather than keyed on
/// the provider's name. A name-keyed table would keep claiming `ucsf` for a
/// `versa_azure` repointed at another host — while `tier()`, reading the
/// endpoint, had already demoted it to Public. The two would disagree, and the
/// disagreement is a private-looking badge on a public flow.
#[test]
fn a_versa_module_repointed_elsewhere_loses_ucsf_with_its_tier() {
    for elsewhere in [
        "https://evil.example.com/general",
        // One typo away in a user-editable config field, both directions.
        "https://unified-api.ucsf.edu.evil.example/general",
        "https://evil.example/unified-api.ucsf.edu",
        // Unparseable, so there is no host to vouch for.
        "unified-api.ucsf.edu",
        "",
    ] {
        assert_eq!(
            ucsf_gateway_affiliation(elsewhere),
            None,
            "{elsewhere} must not claim an institution"
        );
    }
}

/// The corpus guard on a claim `ucsf_gateway_affiliation` makes *structurally*:
/// tier and affiliation are decided by **one** host check, so they cannot
/// disagree.
///
/// ⚠ **The universality is the implementation's, not this test's** — the comment
/// here used to claim it, which is a stronger promise than the assertion keeps.
/// While affiliation is a `match` on the tier predicate, the equivalence below
/// is a tautology and these ten endpoints establish nothing on their own.
///
/// What the corpus is for is the day someone "simplifies" affiliation into its
/// own `host_of(..) == UCSF_GATEWAY_HOST` comparison. Two implementations
/// reading the same constant look right, pass every other test in this file, and
/// drift the first time one of them is edited. Every row below is an endpoint
/// where a plausible second copy — a `contains`, a case-sensitive compare, a
/// missing parse failure — answers differently from the first.
#[test]
fn a_versa_endpoint_is_ucsf_exactly_when_it_is_private() {
    for endpoint in [
        VERSA_AZURE_ENDPOINT,
        "https://unified-api.ucsf.edu/general",
        "https://unified-api.ucsf.edu/general/awsai",
        "https://UNIFIED-API.UCSF.EDU/general",
        "https://unified-api.ucsf.edu.evil.example/general",
        "https://evil.example/unified-api.ucsf.edu",
        "https://bedrock-runtime.us-west-2.amazonaws.com",
        "http://localhost:11434",
        "unified-api.ucsf.edu",
        "",
    ] {
        let private = super::ucsf_gateway_tier(endpoint) == ProviderTier::Private;
        let affiliation = ucsf_gateway_affiliation(endpoint);
        assert_eq!(
            private,
            affiliation.is_some(),
            "{endpoint}: tier says private={private} but affiliation says {affiliation:?}"
        );
        if private {
            assert_eq!(affiliation, Some(ucsf()), "{endpoint}");
        }
    }
}

// ------------------------------------------------------------------- Self-hosted

/// `Local` is what a model running on this machine gets, and it is decided by
/// the same loopback predicate that decides the tier.
///
/// ⚠ These are **resolved base URLs**, not config values. The shipped
/// `OLLAMA_HOST` is the bare string `localhost`, which `is_loopback_host`
/// answers *false* for — it has no scheme, so `url::Url` parses it as an opaque
/// path with no host at all. Both `from_env` and `from_custom_config` normalise
/// it to `http://localhost:11434/` before storing, so the predicate never sees
/// the bare form; that the shipped default really does come out `Local` is
/// asserted through a real instance in
/// `a_real_ollama_instance_follows_the_base_url_it_resolved` below.
#[test]
fn inference_that_stays_on_this_machine_is_local() {
    for loopback in [
        "http://localhost:11434",
        "http://127.0.0.1:11434",
        "http://[::1]:11434",
        "http://LOCALHOST:11434",
        "http://sidecar.localhost:11543",
    ] {
        assert_eq!(
            self_hosted_affiliation(loopback),
            Some(ModelAffiliation::Local),
            "{loopback}"
        );
    }
}

/// A remote `OLLAMA_HOST` (or `LLAMACPP_EXTERNAL_HOST`) is **someone else's
/// server**. It is neither `Local` nor an `Institution`, and the first half of
/// that matters most: `Local` carries blanket permission over every private
/// extension, so inheriting it here would hand a lab box in another building the
/// most permissive affiliation in the model.
#[test]
fn a_self_hosted_provider_pointed_off_the_machine_is_not_local() {
    for remote in [
        "http://gpu.lab.ucsf.edu:11434",
        "https://api.example-saas.com",
        // The gateway itself: a Versa endpoint reached through the self-hosted
        // rule is still not this machine.
        "https://unified-api.ucsf.edu/general",
    ] {
        let affiliation = self_hosted_affiliation(remote);
        assert_ne!(
            affiliation,
            Some(ModelAffiliation::Local),
            "{remote} must not inherit Local's blanket permission"
        );
        assert_eq!(affiliation, None, "{remote}");
    }
}

/// The loopback predicate is **reused, not reimplemented**. It is deliberately
/// narrow and lexical, and a "more correct" second copy would silently widen who
/// counts as local — which is the one direction that grants reach. Every case
/// below genuinely is (or looks like) this machine and still answers `None`;
/// they are here so a reimplementation that "fixes" them fails.
#[test]
fn the_narrow_loopback_predicate_decides_local() {
    for not_local in [
        "http://127.0.0.2:11434",
        "http://[::ffff:127.0.0.1]:11434",
        "localhost:11434",
        "",
        "http://localhost.evil.example/",
        "http://notlocalhost/",
        "http://127.0.0.1.evil.example/",
    ] {
        assert_eq!(
            self_hosted_affiliation(not_local),
            None,
            "{not_local}: `is_loopback_host` answers false, so affiliation must too"
        );
        // ...and it agrees with the tier the same predicate produced.
        assert_eq!(super::self_hosted_tier(not_local), ProviderTier::Public);
    }
}

/// Wired, on a **real** `OllamaProvider` — Step 4's explicit requirement.
///
/// The predicate tests above cannot see whether the provider calls it, or hands
/// it the right field. `from_custom_config` builds an Ollama-engine provider
/// from a user-writable JSON file whose `base_url` points anywhere and whose
/// `name` can shadow a built-in, so an affiliation keyed on the name would badge
/// this instance `Local` while it posts prompts to a SaaS endpoint.
#[test]
fn a_real_ollama_instance_follows_the_base_url_it_resolved() {
    let here = self_hosted_at("http://localhost:11434");
    assert_eq!(here.affiliation(), Some(ModelAffiliation::Local));
    assert_eq!(here.tier(), ProviderTier::Private);

    // The shipped default, through construction — `OLLAMA_HOST` is the bare
    // `localhost`, which only becomes a loopback *URL* once the provider
    // normalises it. Asserted on the constant, not on its text, so moving the
    // default off this machine fails here.
    let shipped = self_hosted_at(OLLAMA_HOST);
    assert_eq!(shipped.affiliation(), Some(ModelAffiliation::Local));
    assert_eq!(shipped.tier(), ProviderTier::Private);

    let elsewhere = self_hosted_at("https://api.example-saas.com");
    // Same name, same metadata, same everything a name-keyed rule can see.
    assert_eq!(elsewhere.get_name(), here.get_name());
    assert_ne!(elsewhere.affiliation(), Some(ModelAffiliation::Local));
    assert_eq!(elsewhere.affiliation(), None);
    assert_eq!(elsewhere.tier(), ProviderTier::Public);
}

// -------------------------------------------------------------- The trait default

/// A provider that overrides nothing. Not a mock of anything — it exists so the
/// **default** can be asserted directly, which is the claim every public
/// provider in the registry rests on: they are unaffiliated because they never
/// say otherwise, not because someone wrote `None` twenty times.
struct ProviderThatSaysNothing;

#[async_trait]
impl Provider for ProviderThatSaysNothing {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "says_nothing"
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
        unreachable!("this provider exists to be asked its affiliation and nothing else")
    }
}

/// DEFAULT = no affiliation, matching `tier`'s DEFAULT = Public. Fail-safe in
/// the same direction: a provider module that forgets to say gets *less* reach,
/// never more. A default of `Local` would be catastrophic — it is the most
/// permissive value in the model — and a default of `Institution(..)` would
/// claim an agreement nobody signed.
#[test]
fn a_provider_that_says_nothing_has_no_affiliation() {
    assert_eq!(ProviderThatSaysNothing.affiliation(), None);
    assert_eq!(ProviderThatSaysNothing.tier(), ProviderTier::Public);
}

// ------------------------------------------------------------------- Composites

/// One half of a lead/worker pair, stating the tier and affiliation the
/// composite should see.
///
/// The `Local` and public halves below are **real** `OllamaProvider`s built by
/// `self_hosted_at`; only the *institutional* halves are stated, because
/// `VersaAzureProvider`'s `resolved_endpoint` is private to its own module and
/// there is no other way to obtain an `Institution` half from outside it. The
/// wiring these tests defend — that the composite reads *both* halves — is
/// visible either way: every pair below is asymmetric.
struct Half {
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
}

#[async_trait]
impl Provider for Half {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "half"
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("nothing")
    }

    fn tier(&self) -> ProviderTier {
        self.tier
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.affiliation
    }

    async fn complete_with_model(
        &self,
        _model_config: &ModelConfig,
        _system: &str,
        _messages: &[Message],
        _tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        unreachable!("this provider exists to be asked its affiliation and nothing else")
    }
}

fn half(tier: ProviderTier, affiliation: Option<ModelAffiliation>) -> Arc<dyn Provider> {
    Arc::new(Half { tier, affiliation })
}

fn institution(name: &str) -> ModelAffiliation {
    ModelAffiliation::institution(InstitutionId::new(name))
}

fn composite(lead: Arc<dyn Provider>, worker: Arc<dyn Provider>) -> LeadWorkerProvider {
    LeadWorkerProvider::new(lead, worker, None)
}

/// `LeadWorkerProvider` is the tree's only composite, and it is the exact shape
/// DR-26's *"may hand back something other than what was asked for"* warning
/// describes: a chat saved in the retired lead/worker mode restores as one
/// (`factory::create_from_persisted`), and `get_name()` answers for the **lead
/// alone**.
///
/// It already overrides `tier()` for that reason. Leaving `affiliation()` on the
/// trait default produces the one combination DR-26's vocabulary says cannot
/// exist — tier `Private` with affiliation `None` — and `None` is specified to
/// mean *"a public model; the tier gates already hold, affiliation never
/// applies"*. A gate that short-circuits on it would skip the cross-affiliation
/// check for a private composite: fail-**open**, in the scenario DR-26 exists to
/// catch.
///
/// ⚠ The census in `factory.rs` structurally cannot see this: `lead_worker` is
/// never `register`ed, it is constructed directly.
#[test]
fn a_private_composite_always_states_an_affiliation() {
    for (lead, worker) in [
        (
            half(ProviderTier::Private, Some(ModelAffiliation::Local)),
            half(ProviderTier::Private, Some(ModelAffiliation::Local)),
        ),
        (
            half(ProviderTier::Private, Some(institution("ucsf"))),
            half(ProviderTier::Private, Some(institution("ucsf"))),
        ),
        (
            half(ProviderTier::Private, Some(ModelAffiliation::Local)),
            half(ProviderTier::Private, Some(institution("ucsf"))),
        ),
    ] {
        let pair = composite(lead, worker);
        assert_eq!(
            pair.tier(),
            ProviderTier::Private,
            "fixture is only meaningful while both halves are private"
        );
        assert!(
            pair.affiliation().is_some(),
            "a Private composite with no affiliation reads as a public model to every gate"
        );
    }
}

/// The composite discloses the whole transcript to **both** endpoints, so its
/// affiliation is what both halves agree on — not the lead's, which is what
/// anything keyed on `get_name()` would return.
///
/// [`ModelAffiliation::Local`] is the identity of that fold, and it is the
/// inversion DR-26 warns about arriving on a second axis: a local worker
/// discloses nothing, so it neither dilutes nor widens the institution the lead
/// is covered by. Every pair below is asymmetric and asserted in both orders, so
/// an implementation that reads one half and ignores the other fails.
#[test]
fn a_composite_agrees_with_both_halves_and_local_is_the_identity() {
    let local = || half(ProviderTier::Private, Some(ModelAffiliation::Local));
    let ucsf_half = || half(ProviderTier::Private, Some(institution("ucsf")));

    // Two halves on this machine disclose nothing at all.
    assert_eq!(
        composite(local(), local()).affiliation(),
        Some(ModelAffiliation::Local)
    );

    // A local half beside an institutional one: the transfer that happens is
    // the institutional one, so that is what covers the pair — in both orders.
    assert_eq!(
        composite(local(), ucsf_half()).affiliation(),
        Some(institution("ucsf"))
    );
    assert_eq!(
        composite(ucsf_half(), local()).affiliation(),
        Some(institution("ucsf"))
    );

    // Both halves at the same institution.
    assert_eq!(
        composite(ucsf_half(), ucsf_half()).affiliation(),
        Some(institution("ucsf"))
    );
}

/// A public half makes the composite public — `tier()` already says so — and a
/// public model's affiliation is the *absence* of one. This is the only arm
/// where `None` is the right answer, and it is right for the reason DR-26 gives:
/// the tier gates keep a public model away from private data, so affiliation
/// never applies to it.
///
/// Asserted in both orders and with a **real** remote Ollama instance, which is
/// how a public half actually arises in production.
#[test]
fn a_composite_with_a_public_half_is_public_and_unaffiliated() {
    let elsewhere =
        || -> Arc<dyn Provider> { Arc::new(self_hosted_at("https://api.example-saas.com")) };
    let here = || -> Arc<dyn Provider> { Arc::new(self_hosted_at("http://localhost:11434")) };
    let ucsf_half = || half(ProviderTier::Private, Some(institution("ucsf")));

    for (lead, worker) in [
        (here(), elsewhere()),
        (elsewhere(), here()),
        (ucsf_half(), elsewhere()),
        (elsewhere(), ucsf_half()),
        (
            ucsf_half(),
            Arc::new(ProviderThatSaysNothing) as Arc<dyn Provider>,
        ),
    ] {
        let pair = composite(lead, worker);
        assert_eq!(pair.tier(), ProviderTier::Public);
        assert_eq!(
            pair.affiliation(),
            None,
            "a public composite must not carry an institution its private half had"
        );
    }
}

/// The property the fold exists to hold, checked against the **one** comparison
/// in `privacy::affiliation` rather than restated as a table.
///
/// A composite discloses to both endpoints, so it may reach an extension only
/// where *both* halves may. The assertion is that direction of the implication —
/// the composite is never more permissive than the conjunction — because that is
/// the direction that leaks. Being stricter is a warning the user can clear;
/// being looser is a cross-institutional transfer nobody was told about.
///
/// It is not a table of expected answers, so it cannot be satisfied by an
/// implementation that agrees with a hand-written expectation and disagrees with
/// the real gate. Equality is asserted separately, on the pairs where DR-26's
/// vocabulary can express the conjunction exactly.
#[test]
fn a_composite_never_out_reaches_either_half() {
    let halves = [
        ModelAffiliation::Local,
        institution("ucsf"),
        institution("stanford"),
        institution("broad"),
    ];
    let extensions = [
        ExtensionAffiliation::Any,
        ExtensionAffiliation::institution(InstitutionId::new("ucsf")),
        ExtensionAffiliation::institution(InstitutionId::new("stanford")),
        ExtensionAffiliation::institutions([
            InstitutionId::new("ucsf"),
            InstitutionId::new("stanford"),
        ]),
        ExtensionAffiliation::Institutions(Default::default()),
    ];

    for lead in halves {
        for worker in halves {
            let folded = super::composite_affiliation(Some(lead), Some(worker))
                .expect("two private halves must yield an affiliation, never `None`");
            for ext in &extensions {
                let both = compatible(&lead, ext) && compatible(&worker, ext);
                assert!(
                    !compatible(&folded, ext) || both,
                    "{lead:?} + {worker:?} folded to {folded:?}, which reaches {ext:?} \
                     though a half may not"
                );

                // ...and where the conjunction *is* expressible — the halves
                // agree, or one of them is `Local` and discloses nothing — the
                // fold must be exactly it, not merely stricter. A fold that
                // demoted every pair to a value reaching nothing would satisfy
                // the implication above and destroy the feature.
                if lead == worker
                    || lead == ModelAffiliation::Local
                    || worker == ModelAffiliation::Local
                {
                    assert_eq!(
                        compatible(&folded, ext),
                        both,
                        "{lead:?} + {worker:?} vs {ext:?}"
                    );
                }
            }
        }
    }
}

/// A composite spanning two institutions clears **neither** of them.
///
/// ⚠ **This arm is reachable now, and that is what Task 56 changed.** It used to
/// have no representable answer: `composite_affiliation` returned a fake
/// institution id standing in for a missing variant — safe only while nobody
/// registered an institution spelled that way — and a cardinality pin on the
/// factory kept a second institution out of the tree so no lead/worker pair
/// could reach it. `ModelAffiliation::Institutions` represents the pair,
/// `compatible`'s subset rule answers it, and the type is still `Copy`.
///
/// What is asserted here is the refusal direction: the pair clears an extension
/// with no institutional claim, and warns for **both** of the institutions
/// involved — including the lead's, which is the one a fold written as "keep the
/// lead" would silently clear. The half a placeholder could never have had — the
/// pair DOES reach a connector whose allowlist names both — is the test below.
#[test]
fn a_composite_spanning_two_institutions_clears_neither_of_them() {
    let folded =
        super::composite_affiliation(Some(institution("ucsf")), Some(institution("stanford")))
            .expect("two private halves must yield an affiliation, never `None`");

    assert!(
        compatible(&folded, &ExtensionAffiliation::Any),
        "an extension with no institutional claim is unaffected by which two the pair spans"
    );
    for named in ["ucsf", "stanford"] {
        assert!(
            !compatible(
                &folded,
                &ExtensionAffiliation::institution(InstitutionId::new(named))
            ),
            "{named}'s extension must warn: the pair also discloses to the other institution"
        );
    }
    // Order cannot matter — the lead is not privileged on this axis.
    assert_eq!(
        folded,
        super::composite_affiliation(Some(institution("stanford")), Some(institution("ucsf")))
            .unwrap()
    );
    // ...and it is not either half, nor `Local`'s blanket permission.
    assert_ne!(folded, institution("ucsf"));
    assert_ne!(folded, institution("stanford"));
    assert_ne!(folded, ModelAffiliation::Local);
}

/// Task 56 Step 5 gate (1): **a two-institution build works end to end.** The
/// half of the spanning composite the sentinel could not express.
///
/// The sentinel made the pair *safe* — it matched no real allowlist, so every
/// named institution warned — at the price of making it useless: a connector
/// whose registry entry names **both** institutions is exactly the
/// cross-institutional arrangement a DUA papers, and a pair covered by both is
/// exactly who may use it. A fake institution is not in that allowlist either,
/// so the one legitimate cross-institutional flow was refused along with the
/// illegitimate ones and no grant could distinguish them.
///
/// Set-valued with SUBSET semantics answers it: `{ucsf, stanford} ⊆
/// {ucsf, stanford}`. This is the assertion that cannot be made to pass by
/// widening the sentinel, only by representing the pair.
#[test]
fn a_composite_spanning_two_institutions_reaches_an_extension_that_allows_both() {
    let folded =
        super::composite_affiliation(Some(institution("ucsf")), Some(institution("stanford")))
            .expect("two private halves must yield an affiliation, never `None`");

    let allows_both = ExtensionAffiliation::institutions([
        InstitutionId::new("ucsf"),
        InstitutionId::new("stanford"),
    ]);
    assert!(
        compatible(&folded, &allows_both),
        "a pair covered by ucsf AND stanford must reach a connector whose allowlist \
         names both: it is the flow both institutions' agreements already cover. \
         {folded:?} did not, which means the fold is still encoding the pair as \
         something other than the two institutions it spans"
    );

    // ...and the refusal direction is unchanged, so this is not the whole
    // allowlist being cleared: a third institution's connector still warns.
    assert!(
        !compatible(
            &folded,
            &ExtensionAffiliation::institution(InstitutionId::new("broad"))
        ),
        "the pair is covered by neither of broad's agreements"
    );
}

/// Task 56 Step 5 gate (1), end to end: a **real** `LeadWorkerProvider` whose
/// two halves are covered by two different institutions, asked the question a
/// gate asks.
///
/// ⚠ **Through the provider, not through the fold.** `composite_affiliation` is
/// tested directly above; this is the path production takes —
/// `LeadWorkerProvider::affiliation()`, the override that exists because
/// `get_name()` answers for the lead alone — so a pair that folds correctly and
/// is then read wrongly still fails.
#[test]
fn a_real_two_institution_pair_reaches_any_and_is_refused_by_one_institution() {
    let pair = composite(
        half(ProviderTier::Private, Some(institution("ucsf"))),
        half(ProviderTier::Private, Some(institution("stanford"))),
    );
    assert_eq!(
        pair.tier(),
        ProviderTier::Private,
        "fixture is only meaningful while both halves are private"
    );
    let bound = pair
        .affiliation()
        .expect("two private halves must yield an affiliation, never `None`");

    // An extension with no institutional claim is unaffected by which two
    // institutions the pair spans.
    assert!(compatible(&bound, &ExtensionAffiliation::Any));

    // A single-institution connector refuses it — including the LEAD's own,
    // which a fold written as "keep the lead" would have cleared.
    for named in ["ucsf", "stanford"] {
        assert!(
            !compatible(
                &bound,
                &ExtensionAffiliation::institution(InstitutionId::new(named))
            ),
            "{named}'s connector must refuse a pair that also discloses to the other institution"
        );
    }

    // ...and the connector both institutions cleared admits it. This is the flow
    // the sentinel encoding made unreachable.
    assert!(compatible(
        &bound,
        &ExtensionAffiliation::institutions([
            InstitutionId::new("ucsf"),
            InstitutionId::new("stanford"),
        ])
    ));
}

/// Task 56 Step 5 gate (4): **the sentinel is gone**, so it cannot be
/// reintroduced as a shortcut.
///
/// ⚠ **A fake institution id doing the work of a missing variant is wrong in a
/// specific, silent way**: it is only safe while no real institution is spelled
/// the way the sentinel was, which is a property of a *string nobody has chosen
/// yet* rather than of the design. The day one is registered — by a registry
/// entry, a hand-edited snapshot, or a provider deciding its own institution —
/// the composite spanning two institutions silently becomes *compatible* with
/// that one's connector.
///
/// A type check cannot state this: the repair deletes the symbol, so any test
/// naming it stops compiling and would be deleted with it. So the assertion is
/// over the source text, which is what a reintroduction would have to add back.
///
/// ⚠ **The needles are assembled from fragments, and no file is exempt.** They
/// used to be written out, which forced the scan to skip this file by name —
/// the one file in the tree that a shortcut is most likely to be written in,
/// and the one this task edits. Splitting each needle across a `format!` means
/// the source of the scan does not contain what the scan looks for, so the
/// exemption can go and the walk covers every `.rs` file under `crates/`.
#[test]
fn the_spans_institutions_sentinel_is_gone() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/biorouter sits two levels below the workspace root")
        .join("crates");
    assert!(
        root.is_dir(),
        "the scan walks {}; if that path is wrong, this test passes for the wrong reason",
        root.display()
    );

    // Split so this file does not contain either string — see the doc above.
    let needles = [
        format!("SPANS_{}", "INSTITUTIONS"),
        format!("<spans-{}>", "institutions"),
    ];
    for needle in &needles {
        assert!(
            !needle.is_empty(),
            "a needle assembled to nothing would make the scan trivially clean"
        );
    }

    let mut scanned = 0usize;
    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("the scan must not skip an unreadable dir") {
            let path = entry.expect("unreadable directory entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            scanned += 1;
            let src = std::fs::read_to_string(&path).expect("unreadable source file");
            for (i, line) in src.lines().enumerate() {
                if needles.iter().any(|needle| line.contains(needle.as_str())) {
                    offenders.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        scanned >= 400,
        "only {scanned} .rs files were scanned, so an empty result proves nothing"
    );
    assert!(
        offenders.is_empty(),
        "the spanning-institutions sentinel is back. A fake institution id is not a \
         representation: it is correct only until someone registers an institution \
         with that id, and then a pair spanning two institutions silently clears one \
         of them. `ModelAffiliation::Institutions` represents the pair; use it:\n{offenders:#?}"
    );
}
