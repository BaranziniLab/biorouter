use super::{institution, ClusterMode, Connection, CrewRefusal, RunPolicy};
use crate::conversation::message::Message;
use crate::model::ModelConfig;
use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};
use crate::privacy::ProviderTier;
use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use crate::providers::errors::ProviderError;
use async_trait::async_trait;
use rmcp::model::Tool;
use serde_json::{json, Value};
use std::collections::BTreeSet;

struct FixtureProvider {
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
}

#[async_trait]
impl Provider for FixtureProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            "institution-fixture",
            "Institution fixture",
            "",
            "fixture",
            vec![],
            "",
            vec![],
        )
    }

    fn get_name(&self) -> &str {
        "institution-fixture"
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
        Ok((
            Message::assistant().with_text("fixture"),
            ProviderUsage::new("test-model".into(), Usage::default()),
        ))
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("test-model")
    }
}

fn ids(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn connection(
    mode: ClusterMode,
    institution_id: Option<&str>,
    remote_root: Option<&str>,
) -> Connection {
    Connection {
        id: "institution-fixture-connection".into(),
        node_id: None,
        name: "institution fixture".into(),
        ssh_target: "fixture@example.test".into(),
        port: Some(22),
        identity_file: None,
        proxy_jump: None,
        socket_path: "/run/crew-fixture.sock".into(),
        owner_uid: 10001,
        workspace_id: "institution-fixture-workspace".into(),
        workspace_public_key: "11".repeat(32),
        remote_root: remote_root.map(str::to_owned),
        remote_execution: false,
        cluster_connection_id: "institution-fixture-cluster".into(),
        mode,
        institution_id: institution_id.map(str::to_owned),
        policy_epoch: 7,
        status: "connected".into(),
        last_error: None,
        device_id: "22".repeat(32),
        public_key: "33".repeat(32),
    }
}

fn snapshot(
    workspace_institution_id: Value,
    mode: ClusterMode,
    epoch: u64,
    protected: Value,
) -> Value {
    json!({
        "workspace": {
            "institution_id": workspace_institution_id,
            "policy_epoch": epoch,
            "mode": mode,
        },
        "channels": [{"id": "general"}, {"id": "private"}],
        "protected_channel_ids": protected,
    })
}

#[test]
fn normalize_rejects_noncanonical_ids_and_preserves_canonical_labels() {
    assert_eq!(institution::normalize(" UCSF ").unwrap(), "ucsf");
    assert!(institution::normalize("").is_err());
    assert!(institution::normalize("has\nnewline").is_err());
    assert!(institution::normalize("école").is_err());
    assert!(institution::normalize(&"a".repeat(65)).is_err());
}

#[test]
fn merge_accepts_aliases_but_rejects_conflicting_labels() {
    assert_eq!(
        institution::merge(["UCSF", " ucsf "]).unwrap(),
        Some("ucsf".into())
    );
    assert!(institution::merge(["ucsf", "stanford"]).is_err());
}

#[test]
fn provider_affiliation_matrix_enforces_local_and_institutional_boundaries() {
    let ucsf = ids(&["ucsf"]);
    let both = ids(&["ucsf", "stanford"]);
    let local = FixtureProvider {
        tier: ProviderTier::Private,
        affiliation: Some(ModelAffiliation::Local),
    };
    let ucsf_provider = FixtureProvider {
        tier: ProviderTier::Private,
        affiliation: Some(ModelAffiliation::institution(InstitutionId::new("ucsf"))),
    };
    let unstated = FixtureProvider {
        tier: ProviderTier::Private,
        affiliation: None,
    };
    let public = FixtureProvider {
        tier: ProviderTier::Public,
        affiliation: None,
    };
    let multi = FixtureProvider {
        tier: ProviderTier::Private,
        affiliation: Some(
            ModelAffiliation::institutions([
                InstitutionId::new("ucsf"),
                InstitutionId::new("stanford"),
            ])
            .unwrap(),
        ),
    };

    assert!(institution::check_provider(local.tier(), local.affiliation(), &ucsf).is_ok());
    assert!(institution::check_provider(local.tier(), local.affiliation(), &both).is_ok());
    assert!(
        institution::check_provider(ucsf_provider.tier(), ucsf_provider.affiliation(), &ucsf)
            .is_ok()
    );
    assert!(
        institution::check_provider(ucsf_provider.tier(), ucsf_provider.affiliation(), &both)
            .is_err()
    );
    assert!(institution::check_provider(multi.tier(), multi.affiliation(), &both).is_err());
    assert!(institution::check_provider(unstated.tier(), unstated.affiliation(), &ucsf).is_err());
    assert!(
        institution::check_provider(public.tier(), public.affiliation(), &BTreeSet::new()).is_ok()
    );
    assert!(institution::check_provider(public.tier(), public.affiliation(), &ucsf).is_err());
}

#[test]
fn origin_mismatch_is_rejected_even_for_local_context() {
    let ucsf = ids(&["ucsf"]);
    assert!(institution::check_origin(&ucsf, Some("ucsf")).is_ok());
    assert!(institution::check_origin(&ucsf, Some("stanford")).is_err());
    assert!(institution::check_origin(&ucsf, None).is_err());
    assert!(institution::check_origin(&BTreeSet::new(), None).is_ok());
}

#[test]
fn protected_sources_requires_known_channels_and_marks_hidden_sources() {
    let visible = snapshot(json!(null), ClusterMode::Public, 1, json!(["private"]));
    assert!(institution::protected_sources(&visible, "private", &[], "lab").unwrap());
    assert!(
        !institution::protected_sources(&visible, "general", &["general".into()], "lab").unwrap()
    );
    assert!(institution::protected_sources(&visible, "missing", &[], "lab").is_err());
    assert!(
        institution::protected_sources(&visible, "general", &["missing".into()], "lab").is_err()
    );

    let mut channels = Vec::new();
    for index in 0..21 {
        channels.push(json!({"id": format!("channel-{index}")}));
    }
    let too_many = json!({
        "channels": channels,
        "protected_channel_ids": [],
    });
    let sources = (0..20)
        .map(|index| format!("channel-{index}"))
        .collect::<Vec<_>>();
    assert!(institution::protected_sources(&too_many, "channel-20", &sources, "lab").is_err());
    assert!(institution::protected_sources(
        &json!({"channels": [{"id": "general"}]}),
        "general",
        &[],
        "lab"
    )
    .is_err());
}

/// W2-DMN-9: a channel ID a complete snapshot lists nowhere (another workspace's, say) is not in
/// this workspace, and says so with its own code. One missing from a partial list may be a real
/// channel beyond it: still refused, since its protection cannot be read, but never called
/// foreign.
#[test]
fn a_channel_the_workspace_does_not_list_is_named_as_not_in_it() {
    let visible = snapshot(json!(null), ClusterMode::Public, 1, json!([]));
    let foreign = institution::protected_sources(&visible, "general", &["elsewhere".into()], "lab")
        .unwrap_err();
    assert_eq!(foreign.to_string(), "That channel isn't in lab.");
    assert_eq!(
        CrewRefusal::find(&foreign).map(CrewRefusal::code),
        Some("crew_channel_not_in_workspace")
    );

    let mut partial = visible.clone();
    partial["totals"] = json!({"channels": 150});
    let unconfirmed =
        institution::protected_sources(&partial, "general", &["beyond".into()], "lab").unwrap_err();
    assert!(CrewRefusal::find(&unconfirmed).is_none());
    assert_eq!(
        unconfirmed.to_string(),
        "Biorouter couldn't confirm that channel in lab. Refresh Crew, then try again."
    );
    // A whole list, counted, is not partial.
    let mut whole = visible;
    whole["totals"] = json!({"channels": 2});
    let foreign = institution::protected_sources(&whole, "general", &["elsewhere".into()], "lab")
        .unwrap_err();
    assert!(CrewRefusal::find(&foreign).is_some());
}

#[test]
fn public_admission_has_no_owner_taint() {
    let provider = FixtureProvider {
        tier: ProviderTier::Public,
        affiliation: None,
    };
    let admission = institution::admission(
        connection(ClusterMode::Public, None, None),
        &provider,
        &RunPolicy::default(),
        &snapshot(json!(null), ClusterMode::Public, 3, json!([])),
        false,
    )
    .unwrap();
    assert!(!admission.protected_context);
    assert!(admission.institution_ids.is_empty());
}

#[test]
fn legacy_unlabelled_private_admission_is_denied() {
    let provider = FixtureProvider {
        tier: ProviderTier::Private,
        affiliation: Some(ModelAffiliation::Local),
    };
    let error = match institution::admission(
        connection(ClusterMode::Private, None, None),
        &provider,
        &RunPolicy::default(),
        &snapshot(json!(null), ClusterMode::Private, 3, json!([])),
        false,
    ) {
        Ok(_) => panic!("unlabelled private admission unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("Confirm this workspace's institution"));
}

#[test]
fn admission_rejects_stale_epoch_and_connection_workspace_institution_mismatch() {
    let provider = FixtureProvider {
        tier: ProviderTier::Private,
        affiliation: Some(ModelAffiliation::Local),
    };
    let stale = RunPolicy {
        expected_workspace_policy_epoch: Some(2),
        ..RunPolicy::default()
    };
    let stale_error = match institution::admission(
        connection(ClusterMode::Public, None, None),
        &provider,
        &stale,
        &snapshot(json!(null), ClusterMode::Public, 3, json!([])),
        false,
    ) {
        Ok(_) => panic!("stale policy admission unexpectedly succeeded"),
        Err(error) => error,
    };
    assert!(stale_error.to_string().contains("policy changed"));

    let mismatch = match institution::admission(
        connection(ClusterMode::Private, Some("ucsf"), None),
        &provider,
        &RunPolicy::default(),
        &snapshot(json!("stanford"), ClusterMode::Private, 3, json!([])),
        false,
    ) {
        Ok(_) => panic!("institution mismatch admission unexpectedly succeeded"),
        Err(error) => error,
    };
    // W2-DMN-9: both institutions named, in the manual's words, with the code and details.
    assert_eq!(
        mismatch.to_string(),
        "This connection is for ucsf, but institution fixture belongs to stanford."
    );
    let typed = CrewRefusal::find(&mismatch).expect("a typed refusal");
    assert_eq!(typed.code(), "crew_institution_mismatch");
    let details = typed
        .fields()
        .iter()
        .find(|(key, _)| *key == "institution_refusal")
        .map(|(_, value)| value.clone())
        .expect("institution_refusal details");
    assert_eq!(
        details,
        json!({
            "model": "test-model",
            "approved_for": null,
            "workspace": "institution fixture",
            "workspace_institution": "stanford",
        })
    );
}

/// W2-DMN-9: every public-model refusal in admission carries `crew_public_model_refused` and
/// says what is private in the manual's terms, never "cluster".
#[test]
fn public_model_refusals_are_typed_and_plain() {
    let public = FixtureProvider {
        tier: ProviderTier::Public,
        affiliation: None,
    };
    let private_workspace = match institution::admission(
        connection(ClusterMode::Public, None, None),
        &public,
        &RunPolicy::default(),
        &snapshot(json!(null), ClusterMode::Private, 3, json!([])),
        false,
    ) {
        Ok(_) => panic!("a private workspace admitted a public model"),
        Err(error) => error,
    };
    assert_eq!(
        private_workspace.to_string(),
        "institution fixture is Private, so a public model can't read it. Choose a private model."
    );
    let restricted = match institution::admission(
        connection(ClusterMode::Public, None, None),
        &public,
        &RunPolicy::default(),
        &snapshot(json!(null), ClusterMode::Public, 3, json!([])),
        true,
    ) {
        Ok(_) => panic!("restricted context admitted a public model"),
        Err(error) => error,
    };
    assert_eq!(
        restricted.to_string(),
        "Restricted channels in institution fixture can't be read by a public model. Choose a \
         private model."
    );
    let owned =
        institution::check_provider(ProviderTier::Public, None, &ids(&["stanford", "ucsf"]))
            .unwrap_err();
    assert_eq!(
        owned.to_string(),
        "This chat's Crew context belongs to stanford and ucsf, so a public model can't read \
         it. Choose a private model."
    );
    for error in [&private_workspace, &restricted, &owned] {
        assert_eq!(
            CrewRefusal::find(error).map(CrewRefusal::code),
            Some("crew_public_model_refused")
        );
        assert!(!error.to_string().contains("cluster"));
    }
}

/// W2-DMN-9: the model-institution refusal keeps the words older clients match, and gains the
/// code and, in admission, the names a person needs.
#[test]
fn the_model_institution_refusal_is_typed_with_its_details() {
    let stanford = FixtureProvider {
        tier: ProviderTier::Private,
        affiliation: Some(ModelAffiliation::institution(InstitutionId::new(
            "stanford",
        ))),
    };
    let refused = match institution::admission(
        connection(ClusterMode::Private, Some("ucsf"), None),
        &stanford,
        &RunPolicy::default(),
        &snapshot(json!("ucsf"), ClusterMode::Private, 3, json!([])),
        false,
    ) {
        Ok(_) => panic!("a stanford model admitted to ucsf context"),
        Err(error) => error,
    };
    assert_eq!(refused.to_string(), institution::AFFILIATION_REFUSAL);
    let typed = CrewRefusal::find(&refused).expect("typed");
    assert_eq!(typed.code(), "crew_institution_mismatch");
    let details = &typed
        .fields()
        .iter()
        .find(|(key, _)| *key == "institution_refusal")
        .expect("details")
        .1;
    assert_eq!(details["approved_for"], json!(["stanford"]));
    assert_eq!(details["workspace_institution"], "ucsf");
    assert_eq!(details["workspace"], "institution fixture");

    let merged = institution::merge(["ucsf", "stanford"]).unwrap_err();
    assert_eq!(
        CrewRefusal::find(&merged).map(CrewRefusal::code),
        Some("crew_institution_mismatch")
    );
    assert!(merged.to_string().contains("stanford and ucsf"), "{merged}");
}
