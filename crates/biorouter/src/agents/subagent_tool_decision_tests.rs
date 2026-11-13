//! DR-33, design §3.4: a child runs at its parent's EFFECTIVE tier at the
//! moment of the spawn. The parent's slot holds the decision-bound wrapper when
//! a decision model made it public; R4 compares models, DR-19 and DR-31 compare
//! effective values, and a child of a public parent carries the public ceiling.

use super::*;
use crate::privacy::decision_axes::{
    self, DecisionAxes, HostLabel, LoweredBy, NameSource, PrivacyCeiling,
};
use crate::privacy::{ModelAffiliation, PrivacyRefusal, ProviderTier, SessionClassification};
use crate::providers::base::Provider;
use crate::providers::decision_bound::DecisionBoundProvider;
use std::sync::Arc;

/// A model stating both privacy axes.
struct Model {
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
}

#[async_trait::async_trait]
impl Provider for Model {
    fn metadata() -> crate::providers::base::ProviderMetadata {
        crate::providers::base::ProviderMetadata::new(
            "model-mock",
            "Model mock",
            "",
            "mock-model",
            vec![],
            "",
            vec![],
        )
    }
    fn get_name(&self) -> &str {
        "model-mock"
    }
    fn tier(&self) -> ProviderTier {
        self.tier
    }
    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.affiliation
    }
    fn get_model_config(&self) -> crate::model::ModelConfig {
        crate::model::ModelConfig::new_or_fail("mock-model")
    }
    async fn complete_with_model(
        &self,
        _model_config: &crate::model::ModelConfig,
        _system: &str,
        _messages: &[crate::conversation::message::Message],
        _tools: &[Tool],
    ) -> std::result::Result<
        (
            crate::conversation::message::Message,
            crate::providers::base::ProviderUsage,
        ),
        crate::providers::errors::ProviderError,
    > {
        Err(crate::providers::errors::ProviderError::Authentication(
            "the spawn rows never run a turn".to_string(),
        ))
    }
}

fn ucsf() -> Option<ModelAffiliation> {
    Some(ModelAffiliation::institution(
        crate::privacy::affiliation::InstitutionId::new("ucsf"),
    ))
}

fn versa() -> Arc<dyn Provider> {
    Arc::new(Model {
        tier: ProviderTier::Private,
        affiliation: ucsf(),
    })
}

fn claude_code() -> Arc<dyn Provider> {
    Arc::new(Model {
        tier: ProviderTier::Public,
        affiliation: None,
    })
}

fn done_monitor() -> LoweredBy {
    LoweredBy {
        source: NameSource::Monitor,
        label: "Done".to_string(),
        model: "joey-1".to_string(),
        host: HostLabel::Baseten,
        others: 0,
    }
}

/// The parent's slot as `Agent` installs it on a Versa chat that a person made
/// public with a Baseten monitor.
fn lowered_versa() -> Arc<dyn Provider> {
    DecisionBoundProvider::bind(versa(), DecisionAxes::public_for_test(done_monitor()))
}

fn extension(name: &str) -> crate::agents::ExtensionConfig {
    crate::agents::ExtensionConfig::Builtin {
        name: name.to_string(),
        display_name: Some(name.to_string()),
        description: String::new(),
        timeout: Some(30),
        bundled: Some(true),
        available_tools: Vec::new(),
    }
}

fn params(body: serde_json::Value) -> SubagentParams {
    serde_json::from_value(body).unwrap()
}

fn inherit() -> SubagentParams {
    params(json!({ "instructions": "do the thing" }))
}

/// `settings.provider = ollama`, resolved Private (loopback) or Public (off
/// this machine) by the `OLLAMA_HOST` the row pins.
async fn resolve_with_ollama(
    parent: Arc<dyn Provider>,
    loopback: bool,
) -> Result<(TaskConfig, ChildPrivacy)> {
    let host = if loopback {
        "http://localhost:11434"
    } else {
        "https://ollama.example.com"
    };
    let task_config = TaskConfig::new(parent, "parent-1", std::path::Path::new("."), vec![]);
    let asked = params(json!({
        "instructions": "do the thing",
        "settings": { "provider": "ollama" }
    }));
    crate::config::with_config_overrides(
        HashMap::from([("OLLAMA_HOST".to_string(), host.to_string())]),
        resolve_child_privacy(task_config, &asked),
    )
    .await
}

#[tokio::test]
async fn a_lowered_parents_inheriting_child_is_born_public_under_the_ceiling() {
    let parent = lowered_versa();
    assert!(
        !parent.tier().is_private(),
        "precondition: the slot reads Public"
    );
    let task_config = TaskConfig::new(
        parent,
        "parent-1",
        std::path::Path::new("."),
        vec![extension("ucsfomopagent"), extension("developer")],
    );

    let (child, privacy) = resolve_child_privacy(task_config, &inherit())
        .await
        .expect("R4 compares models (Private, Private) and DR-31 effective (none, none)");

    assert_eq!(child.privacy_tier, SessionClassification::Public);
    // The child is handed the parent's MODEL, never its wrapper.
    assert!(child.provider.decision_bound_inner().is_none());
    assert!(child.provider.tier().is_private());
    assert_eq!(
        privacy.ceiling,
        Some(PrivacyCeiling::public_from(
            "parent-1",
            Some(done_monitor())
        ))
    );
    assert!(privacy.public_by_ceiling);
    // Its private extensions are dropped, and the note names the cause rather
    // than claiming the subagent runs on a public model.
    assert_eq!(child.dropped_private_extensions, ["ucsfomopagent"]);
    let note = privacy
        .dropped_note(&child.dropped_private_extensions)
        .expect("a drop is reported");
    assert!(note.contains("public decision model"), "{note}");
    assert!(note.contains("joey-1"), "{note}");
    assert!(!note.contains("running on a public model"), "{note}");
}

/// The row that changed: a Versa parent made public by a decision model may
/// give its child a private model too, because R4 asks about the models.
#[tokio::test]
async fn a_lowered_parent_may_name_a_private_model_for_its_child() {
    let (child, privacy) = resolve_with_ollama(lowered_versa(), true)
        .await
        .expect("a private child model is no more private than the parent's model");
    assert!(child.provider.tier().is_private());
    assert_eq!(child.privacy_tier, SessionClassification::Public);
    assert!(privacy.ceiling.is_some());
    assert!(privacy.public_by_ceiling);
}

#[tokio::test]
async fn a_lowered_parent_may_give_its_child_a_public_model() {
    let (child, privacy) = resolve_with_ollama(lowered_versa(), false)
        .await
        .expect("Public to Public on the effective tiers");
    assert!(!child.provider.tier().is_private());
    assert_eq!(child.privacy_tier, SessionClassification::Public);
    assert!(
        privacy.ceiling.is_some(),
        "removing names on it can never raise it"
    );
    assert!(
        !privacy.public_by_ceiling,
        "a public model is public on its own, so the ordinary drop note applies"
    );
}

/// The pin that keeps the lowered rows from being "fixed" by weakening R4: a
/// parent public because its OWN model is public still may not spawn a
/// private child.
#[tokio::test]
async fn a_parent_public_by_its_own_model_still_refuses_a_private_child() {
    let err = resolve_with_ollama(claude_code(), true)
        .await
        .expect_err("R4");
    assert!(
        matches!(
            err.downcast_ref::<PrivacyRefusal>(),
            Some(PrivacyRefusal::PrivateChildOfPublicParent { .. })
        ),
        "{err}"
    );

    // Its inheriting child is public and carries the ceiling, with no cause.
    let task_config = TaskConfig::new(claude_code(), "p", std::path::Path::new("."), vec![]);
    let (child, privacy) = resolve_child_privacy(task_config, &inherit())
        .await
        .unwrap();
    assert_eq!(child.privacy_tier, SessionClassification::Public);
    assert_eq!(
        privacy.ceiling,
        Some(PrivacyCeiling::public_from("p", None))
    );
    assert!(!privacy.public_by_ceiling);
}

#[tokio::test]
async fn a_private_parent_passes_no_ceiling_and_still_refuses_a_public_child() {
    let task_config = TaskConfig::new(versa(), "p", std::path::Path::new("."), vec![]);
    let (child, privacy) = resolve_child_privacy(task_config, &inherit())
        .await
        .unwrap();
    assert_eq!(child.privacy_tier, SessionClassification::Private);
    assert!(privacy.ceiling.is_none());

    let err = resolve_with_ollama(versa(), false)
        .await
        .expect_err("DR-19");
    assert!(
        matches!(
            err.downcast_ref::<PrivacyRefusal>(),
            Some(PrivacyRefusal::PublicChildOfPrivateParent { .. })
        ),
        "{err}"
    );
}

async fn store() -> (
    tempfile::TempDir,
    Arc<crate::session::SessionManager>,
    AgentConfig,
) {
    let temp = tempfile::TempDir::new().unwrap();
    let sm = Arc::new(crate::session::SessionManager::new(
        temp.path().to_path_buf(),
    ));
    let config = AgentConfig::new(
        sm.clone(),
        crate::config::permission::PermissionManager::instance(),
        None,
        crate::config::BioRouterMode::Auto,
    );
    (temp, sm, config)
}

/// The ceiling lands in the statement that writes the child's classification,
/// so the child's first fold already reads it, and only where it was written.
#[tokio::test]
async fn the_childs_row_is_stamped_with_the_ceiling_and_folds_public() {
    let (temp, sm, config) = store().await;
    let parent = sm
        .create_session(
            temp.path().to_path_buf(),
            "parent".into(),
            crate::session::session_manager::SessionType::User,
        )
        .await
        .unwrap();
    let privacy = ChildPrivacy {
        ceiling: Some(PrivacyCeiling::public_from(
            parent.id.as_str(),
            Some(done_monitor()),
        )),
        public_by_ceiling: true,
        default_worker_note: None,
    };
    let child = create_subagent_session_under(
        &config,
        temp.path().to_path_buf(),
        &parent.id,
        &inherit(),
        SessionClassification::Public,
        &Model {
            tier: ProviderTier::Private,
            affiliation: ucsf(),
        },
        &privacy,
        None,
    )
    .await
    .unwrap();

    let row = sm.get_session(&child.id, false).await.unwrap();
    assert_eq!(row.privacy_tier, SessionClassification::Public);
    assert!(decision_axes::ceiling_names_caller(&row, &parent.id));
    let axes = decision_axes::from_row(&row);
    assert_eq!(axes.tier(), Some(ProviderTier::Public));
    assert!(axes.lowers(ProviderTier::Private));

    // A child born Private (its parent is classified Private) gets no ceiling:
    // it would be invalid where it lands.
    sm.update(&parent.id)
        .raise_privacy(SessionClassification::Private, "turn:versa_azure")
        .apply()
        .await
        .unwrap();
    let child = create_subagent_session_under(
        &config,
        temp.path().to_path_buf(),
        &parent.id,
        &inherit(),
        SessionClassification::Public,
        &Model {
            tier: ProviderTier::Private,
            affiliation: ucsf(),
        },
        &privacy,
        None,
    )
    .await
    .unwrap();
    let row = sm.get_session(&child.id, false).await.unwrap();
    assert_eq!(row.privacy_tier, SessionClassification::Private);
    assert!(!decision_axes::ceiling_names_caller(&row, &parent.id));
}

fn settings_with_copy(model: Option<&str>) -> DecisionSettings {
    serde_json::from_value(json!({
        "defaultModel": null,
        "monitors": [{
            "id": "done", "name": "Done", "threshold": 0.9, "option": null,
            "model": model.map(|m| {
                let (provider, model) = m.split_once('/').unwrap();
                json!({ "provider": provider, "model": model })
            }),
            "rearmBelow": null,
            "inheritedFrom": "parent-1",
            "question": {"instructions": "Is it done?", "kind": {"binary": {}}}
        }]
    }))
    .unwrap()
}

/// Design §3.4 item 1: a copy of the parent's "also watch" monitor is checked
/// against the classification the child is born with, so a public copy is never
/// stranded on a Private child, where Gate H would refuse every check.
#[tokio::test]
async fn a_public_copy_on_a_child_born_private_refuses_the_spawn() {
    let copy = settings_with_copy(Some("baseten_decision/joey-1"));
    let refusal =
        stranded_copy_refusal(Some(&copy), SessionClassification::Private).expect("stranded");
    assert!(refusal.contains("'Done'"), "{refusal}");
    assert!(refusal.contains("a Baseten model"), "{refusal}");
    assert!(refusal.contains("Nothing was started"), "{refusal}");
    // On a public child it can run, so it is carried.
    assert!(stranded_copy_refusal(Some(&copy), SessionClassification::Public).is_none());
    // A model-less copy with no default follows the tier.
    assert!(stranded_copy_refusal(
        Some(&settings_with_copy(None)),
        SessionClassification::Private
    )
    .is_none());

    // And at the door: nothing is created.
    let (temp, sm, config) = store().await;
    let parent = sm
        .create_session(
            temp.path().to_path_buf(),
            "parent".into(),
            crate::session::session_manager::SessionType::User,
        )
        .await
        .unwrap();
    let err = create_subagent_session_under(
        &config,
        temp.path().to_path_buf(),
        &parent.id,
        &inherit(),
        SessionClassification::Private,
        &Model {
            tier: ProviderTier::Private,
            affiliation: ucsf(),
        },
        &ChildPrivacy::default(),
        Some(&copy),
    )
    .await
    .expect_err("a stranded copy refuses the spawn");
    assert!(err.message.contains("'Done'"), "{}", err.message);
    let children = sm
        .list_sessions_by_types_including_empty(&[
            crate::session::session_manager::SessionType::SubAgent,
        ])
        .await
        .unwrap();
    assert!(
        children.is_empty(),
        "no child row was created: {children:?}"
    );
}
