//! Tests of [`super`] (DR-33): the wrapper's fold, its unwrap, and that it
//! delegates every `Provider` method.

use super::*;
use crate::privacy::affiliation::InstitutionId;
use crate::privacy::decision_axes::{HostLabel, NameSource};

struct Plain {
    name: &'static str,
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
}

#[async_trait]
impl Provider for Plain {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }
    fn get_name(&self) -> &str {
        self.name
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
        unreachable!("no test here completes")
    }
    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("gpt-5.5")
    }
    fn supports_streaming(&self) -> bool {
        true
    }
}

fn versa() -> Arc<dyn Provider> {
    Arc::new(Plain {
        name: "versa_azure",
        tier: ProviderTier::Private,
        affiliation: Some(ModelAffiliation::institution(InstitutionId::new("ucsf"))),
    })
}

fn joey() -> LoweredBy {
    LoweredBy {
        source: NameSource::Monitor,
        label: "Done".to_string(),
        model: "joey-1".to_string(),
        host: HostLabel::Baseten,
        others: 0,
    }
}

#[test]
fn identity_axes_install_the_inner_provider_itself() {
    let inner = versa();
    let bound = DecisionBoundProvider::bind(Arc::clone(&inner), DecisionAxes::identity());
    assert!(
        Arc::ptr_eq(&bound, &inner),
        "a chat naming nothing got a wrapper"
    );
    assert!(bound.decision_bound_inner().is_none());
    assert!(bound.decision_lowered_by().is_none());
}

#[test]
fn public_axes_lower_a_private_model_and_say_why() {
    let inner = versa();
    let bound =
        DecisionBoundProvider::bind(Arc::clone(&inner), DecisionAxes::public_for_test(joey()));
    assert_eq!(bound.tier(), ProviderTier::Public);
    assert_eq!(
        bound.affiliation(),
        None,
        "Public must carry no affiliation"
    );
    assert_eq!(bound.decision_lowered_by(), Some(&joey()));
    // The row records the real provider, never the wrapper.
    assert_eq!(bound.get_name(), "versa_azure");
    assert_eq!(bound.get_model_config().model_name, "gpt-5.5");
    assert_eq!(bound.restore_binding().model().model_name, "gpt-5.5");
    assert!(bound.supports_streaming());
    let unwrapped = unwrap_decision_bound(Arc::clone(&bound));
    assert!(Arc::ptr_eq(&unwrapped, &inner));
}

#[test]
fn a_public_model_is_public_without_a_decision_cause() {
    let claude: Arc<dyn Provider> = Arc::new(Plain {
        name: "claude_code",
        tier: ProviderTier::Public,
        affiliation: None,
    });
    let bound = DecisionBoundProvider::bind(claude, DecisionAxes::public_for_test(joey()));
    assert_eq!(bound.tier(), ProviderTier::Public);
    // Its model made it public; a refusal must not blame the decision model.
    assert!(bound.decision_lowered_by().is_none());
}

#[test]
fn a_wrapper_is_never_wrapped_twice() {
    let inner = versa();
    let once =
        DecisionBoundProvider::bind(Arc::clone(&inner), DecisionAxes::public_for_test(joey()));
    // Handed to a child with no names of its own: the child gets the parent's
    // INNER provider, and none of the parent's axes.
    let child = DecisionBoundProvider::bind(Arc::clone(&once), DecisionAxes::identity());
    assert!(Arc::ptr_eq(&child, &inner));
    assert_eq!(child.tier(), ProviderTier::Private);
    let twice = DecisionBoundProvider::bind(once, DecisionAxes::public_for_test(joey()));
    let first_inner = twice.decision_bound_inner().unwrap();
    assert!(
        first_inner.decision_bound_inner().is_none(),
        "a wrapper wrapped a wrapper"
    );
}

#[test]
fn the_install_comparison_sees_a_changed_inner_or_changed_axes() {
    let inner = versa();
    let lowered =
        DecisionBoundProvider::bind(Arc::clone(&inner), DecisionAxes::public_for_test(joey()));
    assert!(install_is_current(
        &lowered,
        &inner,
        &DecisionAxes::public_for_test(joey())
    ));
    assert!(!install_is_current(
        &lowered,
        &inner,
        &DecisionAxes::identity()
    ));
    assert!(install_is_current(
        &inner,
        &inner,
        &DecisionAxes::identity()
    ));
    assert!(!install_is_current(
        &inner,
        &inner,
        &DecisionAxes::public_for_test(joey())
    ));
    let other = versa();
    assert!(!install_is_current(
        &inner,
        &other,
        &DecisionAxes::identity()
    ));
}

/// The names of the `fn`s declared directly in the block that opens at
/// `header` and closes at the first line that is exactly `}`.
fn fns_in_block(src: &str, header: &str) -> Vec<String> {
    let start = src
        .find(header)
        .unwrap_or_else(|| panic!("{header:?} is not in the source"));
    let body = src.get(start..).expect("the header is on a char boundary");
    let end = body.find("\n}\n").expect("the block closes");
    let block = body.get(..end).expect("the block ends on a char boundary");
    let fn_re = regex::Regex::new(r"(?m)^    (?:async )?fn ([a-z_][a-z0-9_]*)").unwrap();
    fn_re
        .captures_iter(block)
        .map(|caps| caps[1].to_string())
        .collect()
}

/// A trait method the wrapper forgets silently takes the trait's default, and
/// every default is a guess about the provider it wraps: `tier()` would answer
/// Public, `uses_tool_bridge()` would look up the wrapper's name. So the wrapper
/// must implement every method the trait declares.
#[test]
fn the_decision_bound_provider_delegates_every_provider_method() {
    let base = include_str!("base.rs");
    let wrapper = include_str!("decision_bound.rs");
    let trait_fns = fns_in_block(base, "pub trait Provider: Send + Sync {");
    let impl_fns = fns_in_block(wrapper, "impl Provider for DecisionBoundProvider {");
    assert!(trait_fns.len() > 30, "the trait scan found {trait_fns:?}");
    let missing: Vec<&String> = trait_fns.iter().filter(|f| !impl_fns.contains(f)).collect();
    assert!(
        missing.is_empty(),
        "DecisionBoundProvider does not implement {missing:?}; each would silently take the \
         trait default instead of the wrapped provider's answer"
    );
}
