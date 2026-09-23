use super::{ClusterMode, Connection, RunPolicy};
use crate::privacy::affiliation::{owners_compatible, InstitutionId, ModelAffiliation};
use crate::privacy::ProviderTier;
use crate::providers::base::Provider;
use anyhow::{ensure, Result};
use biorouter_crew::{is_canonical_institution_id, ProviderAffiliation};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) struct Admission {
    pub connection: Connection,
    pub workspace_institution_id: Option<String>,
    pub workspace_policy_epoch: u64,
    pub institution_ids: BTreeSet<String>,
    pub protected_context: bool,
}

pub(super) fn normalize(value: &str) -> Result<String> {
    let canonical = value.trim().to_ascii_lowercase();
    ensure!(is_canonical_institution_id(&canonical), "Crew institution must be a canonical institution ID of 1–64 ASCII letters, digits, underscores or hyphens");
    Ok(InstitutionId::new(&canonical).as_str().to_owned())
}

pub(super) fn merge<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<Option<String>> {
    let values: BTreeSet<_> = values.into_iter().map(normalize).collect::<Result<_>>()?;
    ensure!(
        values.len() <= 1,
        "Crew aliases have different institutions; use a separately verified cluster connection"
    );
    Ok(values.into_iter().next())
}

pub(super) fn check_provider(
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
    institutions: &BTreeSet<String>,
) -> Result<()> {
    if tier == ProviderTier::Public {
        ensure!(
            institutions.is_empty(),
            "Institution-owned Crew context cannot be sent to a public model"
        );
        return Ok(());
    }
    let owners = institutions
        .iter()
        .map(|id| normalize(id).map(|id| InstitutionId::new(&id)))
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(owners_compatible(affiliation, &owners), "Crew institution does not match the model's resolved affiliation; choose a local model or a model approved for this institution");
    Ok(())
}

pub(super) fn check_origin(
    institutions: &BTreeSet<String>,
    workspace_institution_id: Option<&str>,
) -> Result<()> {
    ensure!(
        institutions
            .iter()
            .all(|id| Some(id.as_str()) == workspace_institution_id),
        "This conversation contains another institution's context; start a fresh conversation for this workspace"
    );
    Ok(())
}

pub(super) fn provider_affiliation(provider: &dyn Provider) -> ProviderAffiliation {
    match provider.affiliation() {
        Some(ModelAffiliation::Local) => ProviderAffiliation::Local,
        Some(ModelAffiliation::Institutions(ids)) => ProviderAffiliation::Institutions {
            institution_ids: ids.iter().map(|id| id.as_str().to_owned()).collect(),
        },
        None => ProviderAffiliation::Unstated,
    }
}

pub(super) fn protected_sources(
    snapshot: &Value,
    channel: &str,
    sources: &[String],
) -> Result<bool> {
    let channels = snapshot["channels"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid Crew channel policy"))?;
    let selected: BTreeSet<_> = sources
        .iter()
        .map(String::as_str)
        .chain([channel])
        .collect();
    ensure!(selected.len() <= 20, "Too many Crew context channels");
    ensure!(
        selected.iter().all(|id| channels
            .iter()
            .any(|entry| entry["id"].as_str() == Some(*id))),
        "Crew context channel is unavailable; refresh before granting agent access"
    );
    let protected: Vec<String> = serde_json::from_value(snapshot["protected_channel_ids"].clone())
        .map_err(|_| anyhow::anyhow!("Crew broker does not support protected-context policy; upgrade the broker before granting an agent"))?;
    Ok(protected.iter().any(|id| selected.contains(id.as_str())))
}

pub(super) fn admission(
    connection: Connection,
    provider: &dyn Provider,
    policy: &RunPolicy,
    snapshot: &Value,
    protected_context: bool,
) -> Result<Admission> {
    let workspace = &snapshot["workspace"];
    ensure!(workspace.get("institution_id").is_some(), "Crew broker does not support institution policy; upgrade the broker before granting an agent");
    let workspace_institution_id = match &workspace["institution_id"] {
        Value::Null => None,
        Value::String(id) => Some(normalize(id)?),
        _ => anyhow::bail!("Invalid Crew workspace institution"),
    };
    let workspace_policy_epoch = workspace["policy_epoch"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Invalid Crew workspace policy epoch"))?;
    ensure!(
        policy
            .expected_workspace_policy_epoch
            .is_none_or(|epoch| epoch == workspace_policy_epoch),
        "Crew workspace policy changed; refresh before granting agent access"
    );
    let workspace_mode: ClusterMode = serde_json::from_value(workspace["mode"].clone())?;
    ensure!(
        provider.tier() != ProviderTier::Public || workspace_mode == ClusterMode::Public,
        "Private workspace blocks public models"
    );
    merge(
        connection
            .institution_id
            .iter()
            .chain(workspace_institution_id.iter())
            .map(String::as_str),
    )?;
    check_origin(
        &policy.origin_institution_ids,
        workspace_institution_id.as_deref(),
    )?;
    let protected = connection.mode == ClusterMode::Private
        || workspace_mode == ClusterMode::Private
        || policy.origin_restricted
        || protected_context
        || (provider.tier() != ProviderTier::Public && connection.remote_root.is_some());
    ensure!(
        !protected || provider.tier() != ProviderTier::Public,
        "Restricted Crew context cannot be sent to a public model"
    );
    if protected {
        ensure!(workspace_institution_id.is_some(), "Confirm this workspace's institution before granting an agent; unlabelled private workspaces allow human collaboration only");
        ensure!(
            connection.mode != ClusterMode::Private || connection.institution_id.is_some(),
            "Set this private SSH connection's institution before granting an agent"
        );
    }
    let mut institution_ids = policy.origin_institution_ids.clone();
    if protected {
        institution_ids.extend(connection.institution_id.iter().cloned());
        institution_ids.extend(workspace_institution_id.iter().cloned());
    }
    check_provider(provider.tier(), provider.affiliation(), &institution_ids)?;
    Ok(Admission {
        connection,
        workspace_institution_id,
        workspace_policy_epoch,
        institution_ids,
        protected_context: protected,
    })
}
