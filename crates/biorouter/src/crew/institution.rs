use super::refusal::{self, CrewRefusal};
use super::{ClusterMode, Connection, RunPolicy};
use crate::privacy::affiliation::{owners_compatible, InstitutionId, ModelAffiliation};
use crate::privacy::ProviderTier;
use crate::providers::base::Provider;
use anyhow::{ensure, Result};
use biorouter_crew::{is_canonical_institution_id, ProviderAffiliation};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// The words that mark the model-institution refusal. Clients older than its code
/// (`crew_institution_mismatch`) matched them, so the sentence stays as it was; the names a
/// person needs travel beside it as `institution_refusal` ([`refusal_details`]).
pub const AFFILIATION_REFUSAL: &str = "Crew institution does not match the model's resolved affiliation; choose a local model or a model approved for this institution";

/// What an institution refusal carries beside its sentence, so a client can say the same thing
/// everywhere (Q2-76, W2-DMN-9): `model` (as requested), `approved_for` (the institutions that
/// approved the model; `null` when it states none), `workspace` (the workspace's signed name,
/// else the saved connection's name) and `workspace_institution`.
pub fn refusal_details(
    model: &str,
    affiliation: Option<ModelAffiliation>,
    workspace: Option<String>,
    workspace_institution: Option<String>,
) -> Value {
    let approved_for = affiliation
        .and_then(|affiliation| affiliation.institution_set())
        .map(|set| {
            set.iter()
                .map(|institution| institution.as_str().to_owned())
                .collect::<Vec<_>>()
        });
    json!({
        "model": model,
        "approved_for": approved_for,
        "workspace": workspace,
        "workspace_institution": workspace_institution,
    })
}

/// `error` with `institution_refusal` details added when it is an institution refusal that
/// carries none yet. Every other error is returned as it came.
pub(super) fn with_details(error: anyhow::Error, details: impl FnOnce() -> Value) -> anyhow::Error {
    let Some(found) = CrewRefusal::find(&error) else {
        return error;
    };
    if found.code() != refusal::INSTITUTION_MISMATCH
        || found
            .fields()
            .iter()
            .any(|(key, _)| *key == "institution_refusal")
    {
        return error;
    }
    anyhow::Error::new(found.clone().with("institution_refusal", details()))
}

/// The workspace's name as `snapshot` signs it, else the saved connection's.
fn workspace_label(connection: &Connection, snapshot: &Value) -> String {
    snapshot["workspace"]["name"]
        .as_str()
        .filter(|name| biorouter_crew::workspace_name_valid(name))
        .map_or_else(|| connection.name.clone(), str::to_owned)
}

/// Institutions as a person reads them: `ucsf`, `ucsf and stanford`, `a, b and c`.
fn named(institutions: &BTreeSet<String>) -> String {
    let all: Vec<&str> = institutions.iter().map(String::as_str).collect();
    match all.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

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
    if values.len() > 1 {
        return Err(CrewRefusal::new(
            refusal::INSTITUTION_MISMATCH,
            format!(
                "Connections to one workspace share one institution, but these are set to {}. \
                 Set the same institution on each of them in Connection settings.",
                named(&values)
            ),
        )
        .with("institutions", json!(values))
        .into());
    }
    Ok(values.into_iter().next())
}

pub(super) fn check_provider(
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
    institutions: &BTreeSet<String>,
) -> Result<()> {
    if tier == ProviderTier::Public {
        if !institutions.is_empty() {
            return Err(CrewRefusal::public_model(format!(
                "This chat's Crew context belongs to {}, so a public model can't read it. \
                 Choose a private model.",
                named(institutions)
            ))
            .into());
        }
        return Ok(());
    }
    let owners = institutions
        .iter()
        .map(|id| normalize(id).map(|id| InstitutionId::new(&id)))
        .collect::<Result<BTreeSet<_>>>()?;
    if !owners_compatible(affiliation, &owners) {
        return Err(CrewRefusal::new(refusal::INSTITUTION_MISMATCH, AFFILIATION_REFUSAL).into());
    }
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

/// Whether `snapshot` lists fewer channels than it says the caller is in: the broker leaves
/// whole channels out of a snapshot too large for one frame and counts them all in `totals`.
fn lists_fewer_channels(snapshot: &Value) -> bool {
    let listed = snapshot["channels"].as_array().map_or(0, Vec::len);
    let listed = u64::try_from(listed).unwrap_or(u64::MAX);
    snapshot["totals"]
        .get("channels")
        .is_some_and(|total| total.as_u64().is_none_or(|total| total > listed))
}

/// Whether any of `sources` and `channel` is protected, refusing a channel the snapshot does not
/// list. An ID the workspace lists nowhere, in a snapshot that lists everything, is not one of
/// its channels (another workspace's, or never one): `crew_channel_not_in_workspace` says so
/// (W2-DMN-9). One missing from a partial list may be a real channel beyond it, so it is only
/// refused as unconfirmed. Either way nothing unlisted is granted, since its protection cannot
/// be read.
pub(super) fn protected_sources(
    snapshot: &Value,
    channel: &str,
    sources: &[String],
    workspace: &str,
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
    if !selected.iter().all(|id| {
        channels
            .iter()
            .any(|entry| entry["id"].as_str() == Some(*id))
    }) {
        if lists_fewer_channels(snapshot) {
            anyhow::bail!(
                "Biorouter couldn't confirm that channel in {workspace}. Refresh Crew, then try again."
            );
        }
        return Err(CrewRefusal::new(
            refusal::CHANNEL_NOT_IN_WORKSPACE,
            format!("That channel isn't in {workspace}."),
        )
        .with("workspace", json!(workspace))
        .into());
    }
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
    let label = workspace_label(&connection, snapshot);
    if provider.tier() == ProviderTier::Public && workspace_mode != ClusterMode::Public {
        return Err(CrewRefusal::public_model(format!(
            "{label} is Private, so a public model can't read it. Choose a private model."
        ))
        .with("workspace", json!(label))
        .into());
    }
    let details = || {
        refusal_details(
            &provider.get_model_config().model_name,
            provider.affiliation(),
            Some(label.clone()),
            workspace_institution_id.clone(),
        )
    };
    if let (Some(ours), Some(theirs)) = (&connection.institution_id, &workspace_institution_id) {
        let ours = normalize(ours)?;
        if &ours != theirs {
            return Err(CrewRefusal::new(
                refusal::INSTITUTION_MISMATCH,
                format!("This connection is for {ours}, but {label} belongs to {theirs}."),
            )
            .with("connection_institution", json!(ours))
            .with("institution_refusal", details())
            .into());
        }
    }
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
    if protected && provider.tier() == ProviderTier::Public {
        return Err(CrewRefusal::public_model(format!(
            "Restricted channels in {label} can't be read by a public model. Choose a private model."
        ))
        .with("workspace", json!(label))
        .into());
    }
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
    check_provider(provider.tier(), provider.affiliation(), &institution_ids)
        .map_err(|error| with_details(error, details))?;
    Ok(Admission {
        connection,
        workspace_institution_id,
        workspace_policy_epoch,
        institution_ids,
        protected_context: protected,
    })
}
