use crate::agents::extension::ExtensionConfig;
use crate::agents::extension::PlatformExtensionContext;
use crate::agents::mcp_client::{Error, McpClientTrait, McpMeta};
use crate::config::{
    extension_entry_is_persisted, get_all_extensions, get_extension_entry_by_name, ExtensionEntry,
};
use anyhow::Result;
use async_trait::async_trait;
use indoc::indoc;
use rmcp::model::{
    CallToolResult, Content, ErrorCode, ErrorData, GetPromptResult, Implementation,
    InitializeResult, JsonObject, ListPromptsResult, ListResourcesResult, ListToolsResult,
    ProtocolVersion, ReadResourceResult, ServerCapabilities, ServerNotification, Tool,
    ToolAnnotations, ToolsCapability,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::error;

pub static EXTENSION_NAME: &str = "Extension Manager";
// pub static DISPLAY_NAME: &str = "Extension Manager";

#[derive(Debug, thiserror::Error)]
pub enum ExtensionManagerToolError {
    #[error("Unknown tool: {tool_name}")]
    UnknownTool { tool_name: String },

    #[error("Extension manager not available")]
    ManagerUnavailable,

    #[error("Missing required parameter: {param_name}")]
    MissingParameter { param_name: String },

    #[error("Invalid action: {action}. Must be 'enable' or 'disable'")]
    InvalidAction { action: String },

    #[error("Extension operation failed: {message}")]
    OperationFailed { message: String },

    #[error("Failed to deserialize parameters: {0}")]
    DeserializationError(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ManageExtensionAction {
    Enable,
    Disable,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ManageExtensionsParams {
    pub action: ManageExtensionAction,
    /// Exact installed name returned by search_available_extensions, not a marketplace registry id.
    // ⚠ `alias`, not `rename`, and a `//` comment rather than a `///` one: the
    // SCHEMA still teaches the snake_case name (a doc comment here would ship
    // this paragraph to the model as the property's description), and the alias
    // only forgives the camelCase spelling. It exists because the caller is not
    // guessing — our own result payloads hand it `extensionName`, since `json!` keys are
    // camelCase for the GUI that reads the same payloads — and it passes back
    // the spelling we gave it. Rejecting that costs a whole round-trip to say
    // "expected extension_name", a correction the caller should never have had to make.
    #[serde(alias = "extensionName")]
    pub extension_name: String,
}

/// Install a marketplace extension (#117).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallExtensionParams {
    /// The BAAM registry `id` of the extension to install, e.g.
    /// `playwright-agent`. Recorded as provenance so the privacy tier is
    /// re-derived from a stable id rather than from a renameable config name.
    // ⚠ `alias`, not `rename`, and a `//` comment rather than a `///` one: the
    // SCHEMA still teaches the snake_case name (a doc comment here would ship
    // this paragraph to the model as the property's description), and the alias
    // only forgives the camelCase spelling. It exists because the caller is not
    // guessing — our own result payloads hand it `registryId`, since `json!` keys are
    // camelCase for the GUI that reads the same payloads — and it passes back
    // the spelling we gave it. Rejecting that costs a whole round-trip to say
    // "expected registry_id", a correction the caller should never have had to make.
    #[serde(alias = "registryId")]
    pub registry_id: String,
    /// Enable the extension after installing it. Defaults to true.
    #[serde(default = "default_true")]
    pub enable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchMarketplaceExtensionsParams {
    /// Match a registry id, name, organization, description or tag. Omit to
    /// list every entry visible to this model.
    ///
    /// ⚠ The doc comment is the contract, not decoration: schemars emits it as
    /// the property's `description`, and that is the only channel through which
    /// a Gemini-bound model learns that omitting the field lists everything —
    /// `google.rs` keeps `description` under `properties` and strips `default`.
    #[serde(default)]
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteExtensionPackageParams {
    /// One exact trusted BAAM registry id.
    // ⚠ `alias`, not `rename`, and a `//` comment rather than a `///` one: the
    // SCHEMA still teaches the snake_case name (a doc comment here would ship
    // this paragraph to the model as the property's description), and the alias
    // only forgives the camelCase spelling. It exists because the caller is not
    // guessing — our own result payloads hand it `registryId`, since `json!` keys are
    // camelCase for the GUI that reads the same payloads — and it passes back
    // the spelling we gave it. Rejecting that costs a whole round-trip to say
    // "expected registry_id", a correction the caller should never have had to make.
    #[serde(alias = "registryId")]
    pub registry_id: Option<String>,
    /// Several exact trusted BAAM registry ids. The whole batch is validated
    /// before any package is removed.
    #[serde(default)]
    #[schemars(length(max = 50))]
    // ⚠ `alias`, not `rename`, and a `//` comment rather than a `///` one: the
    // SCHEMA still teaches the snake_case name (a doc comment here would ship
    // this paragraph to the model as the property's description), and the alias
    // only forgives the camelCase spelling. It exists because the caller is not
    // guessing — our own result payloads hand it `registryIds`, since `json!` keys are
    // camelCase for the GUI that reads the same payloads — and it passes back
    // the spelling we gave it. Rejecting that costs a whole round-trip to say
    // "expected registry_ids", a correction the caller should never have had to make.
    #[serde(alias = "registryIds")]
    pub registry_ids: Vec<String>,
}

/// Uninstall an extension that the marketplace cannot name (#164).
///
/// ⚠ **Keyed on the INSTALLED NAME, and that is the whole point of the tool.**
/// [`DeleteExtensionPackageParams`] takes a BAAM registry id and resolves it
/// against the marketplace catalog, so an extension with no registry id — a
/// sideloaded `.brxt`, an MCP server somebody added to `config.yaml` by hand,
/// anything installed outside BAAM — cannot be *named* by that tool at all, let
/// alone deleted by it. Removing one degenerated into the agent editing config
/// and provenance files a step at a time, which is a worse thing to hand a
/// model than one audited transaction behind one approval.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveExtensionParams {
    /// One exact installed extension name, as returned by
    /// `search_available_extensions` — not a marketplace title or registry id.
    // ⚠ `alias`, not `rename`, and a `//` comment rather than a `///` one: the
    // SCHEMA still teaches the snake_case name (a doc comment here would ship
    // this paragraph to the model as the property's description), and the alias
    // only forgives the camelCase spelling. It exists because the caller is not
    // guessing — our own result payloads hand it `extensionName`, since `json!` keys are
    // camelCase for the GUI that reads the same payloads — and it passes back
    // the spelling we gave it. Rejecting that costs a whole round-trip to say
    // "expected extension_name", a correction the caller should never have had to make.
    #[serde(alias = "extensionName")]
    pub extension_name: Option<String>,
    /// Several exact installed extension names. The whole batch is validated
    /// before any extension is removed.
    #[serde(default)]
    #[schemars(length(max = 50))]
    // ⚠ `alias`, not `rename` — see `extension_name` above.
    #[serde(alias = "extensionNames")]
    pub extension_names: Vec<String>,
}

/// The most identifiers either uninstall door acts on under one approval.
///
/// ⚠ The bound is a property of the APPROVAL, not of the machinery: the card
/// lists what is about to be deleted, and a list nobody reads to the end is a
/// card nobody meaningfully approved.
const MAX_DELETION_BATCH: usize = 50;

/// Which uninstall door named the batch, and so what its refusals call the
/// things in it.
#[derive(Clone, Copy)]
enum DeletionIdentifier {
    /// `delete_extension_package` — a trusted BAAM registry id.
    MarketplaceRegistryId,
    /// `remove_extension` — the exact installed name.
    InstalledExtensionName,
}

impl DeletionIdentifier {
    fn empty_batch(self) -> &'static str {
        match self {
            Self::MarketplaceRegistryId => {
                "Give registry_id for one package or registry_ids for a batch"
            }
            Self::InstalledExtensionName => {
                "Give extension_name for one extension or extension_names for a batch"
            }
        }
    }

    fn over_cap(self) -> &'static str {
        match self {
            Self::MarketplaceRegistryId => {
                "An extension deletion batch may contain at most 50 packages"
            }
            Self::InstalledExtensionName => {
                "An extension removal batch may contain at most 50 extensions"
            }
        }
    }

    fn empty_item(self) -> &'static str {
        match self {
            Self::MarketplaceRegistryId => "Marketplace registry ids cannot be empty",
            Self::InstalledExtensionName => "Installed extension names cannot be empty",
        }
    }

    fn ambiguous_batch(self) -> &'static str {
        match self {
            Self::MarketplaceRegistryId => {
                "Two registry ids resolve to the same installed package; nothing was deleted"
            }
            Self::InstalledExtensionName => {
                "Two extension names resolve to the same installed package; nothing was removed"
            }
        }
    }

    fn duplicate(self, identifier: &str) -> String {
        match self {
            Self::MarketplaceRegistryId => {
                format!("`{identifier}` duplicates a package in this batch")
            }
            Self::InstalledExtensionName => {
                format!("`{identifier}` duplicates an extension in this batch")
            }
        }
    }
}

/// The batch shape both uninstall doors accept: the single-identifier field
/// first, then the list, bounded and free of duplicates before anything is
/// touched.
///
/// Shared rather than mirrored. The two doors spell their identifiers
/// differently — a registry id names a marketplace entry, an extension name
/// names what is installed — but the cap, the empty-batch refusal and the
/// duplicate refusal are ONE rule, and a second copy is how the two would come
/// to disagree about the bound that makes the approval card readable.
fn preflight_delete_identifiers(
    single: Option<String>,
    mut requested: Vec<String>,
    kind: DeletionIdentifier,
) -> Result<Vec<String>, ExtensionManagerToolError> {
    if let Some(identifier) = single {
        requested.insert(0, identifier);
    }
    if requested.is_empty() {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: kind.empty_batch().to_owned(),
        });
    }
    if requested.len() > MAX_DELETION_BATCH {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: kind.over_cap().to_owned(),
        });
    }
    let mut seen = std::collections::BTreeSet::new();
    for identifier in &requested {
        if identifier.is_empty() {
            return Err(ExtensionManagerToolError::OperationFailed {
                message: kind.empty_item().to_owned(),
            });
        }
        if !seen.insert(identifier.clone()) {
            return Err(ExtensionManagerToolError::OperationFailed {
                message: kind.duplicate(identifier),
            });
        }
    }
    Ok(requested)
}

fn preflight_delete_registry_ids(
    params: DeleteExtensionPackageParams,
) -> Result<Vec<String>, ExtensionManagerToolError> {
    preflight_delete_identifiers(
        params.registry_id,
        params.registry_ids,
        DeletionIdentifier::MarketplaceRegistryId,
    )
}

fn preflight_remove_extension_names(
    params: RemoveExtensionParams,
) -> Result<Vec<String>, ExtensionManagerToolError> {
    preflight_delete_identifiers(
        params.extension_name,
        params.extension_names,
        DeletionIdentifier::InstalledExtensionName,
    )
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadResourceParams {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    // ⚠ `alias`, not `rename`, and a `//` comment rather than a `///` one: the
    // SCHEMA still teaches the snake_case name (a doc comment here would ship
    // this paragraph to the model as the property's description), and the alias
    // only forgives the camelCase spelling. It exists because the caller is not
    // guessing — our own result payloads hand it `extensionName`, since `json!` keys are
    // camelCase for the GUI that reads the same payloads — and it passes back
    // the spelling we gave it. Rejecting that costs a whole round-trip to say
    // "expected extension_name", a correction the caller should never have had to make.
    #[serde(alias = "extensionName")]
    pub extension_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListResourcesParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    // ⚠ `alias`, not `rename`, and a `//` comment rather than a `///` one: the
    // SCHEMA still teaches the snake_case name (a doc comment here would ship
    // this paragraph to the model as the property's description), and the alias
    // only forgives the camelCase spelling. It exists because the caller is not
    // guessing — our own result payloads hand it `extensionName`, since `json!` keys are
    // camelCase for the GUI that reads the same payloads — and it passes back
    // the spelling we gave it. Rejecting that costs a whole round-trip to say
    // "expected extension_name", a correction the caller should never have had to make.
    #[serde(alias = "extensionName")]
    pub extension_name: Option<String>,
}

pub const READ_RESOURCE_TOOL_NAME: &str = "read_resource";
pub const LIST_RESOURCES_TOOL_NAME: &str = "list_resources";
pub const SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME: &str = "search_available_extensions";
pub const MANAGE_EXTENSIONS_TOOL_NAME: &str = "manage_extensions";
pub const INSTALL_EXTENSION_TOOL_NAME: &str = "install_extension";
pub const BROWSE_MARKETPLACE_EXTENSIONS_TOOL_NAME: &str = "browse_marketplace_extensions";
pub const SEARCH_MARKETPLACE_EXTENSIONS_TOOL_NAME: &str = "search_marketplace_extensions";
pub const DELETE_EXTENSION_PACKAGE_TOOL_NAME: &str = "delete_extension_package";
pub const REMOVE_EXTENSION_TOOL_NAME: &str = "remove_extension";
pub const MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE: &str = "extensionmanager__manage_extensions";

const MARKETPLACE_APPROVAL_TTL: Duration = Duration::from_secs(5 * 60);

pub struct ExtensionManagerClient {
    info: InitializeResult,
    #[allow(dead_code)]
    context: PlatformExtensionContext,
}

#[derive(Clone, Copy)]
enum MarketplaceMutation {
    Install,
}

impl MarketplaceMutation {
    fn tool_name(self) -> &'static str {
        match self {
            Self::Install => INSTALL_EXTENSION_TOOL_NAME,
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Install => "install",
        }
    }

    fn risk(self) -> crate::permission::tool_risk::ToolRisk {
        match self {
            Self::Install => crate::permission::tool_risk::ToolRisk::Medium,
        }
    }
}

fn marketplace_descriptor_json(
    descriptor: &crate::marketplace::MarketplaceExtensionDescriptor,
) -> Value {
    let affiliation = match &descriptor.affiliation {
        crate::privacy::ExtensionAffiliation::Any => Value::String("any".to_owned()),
        crate::privacy::ExtensionAffiliation::Institutions(ids) => Value::Array(
            ids.iter()
                .map(|id| Value::String(id.as_str().to_owned()))
                .collect(),
        ),
    };
    // ⚠ Only a name the REGISTRY stated. When it did not, `extension_name` is
    // the registry id by fallback, and advertising that taught the model a name
    // `manage_extensions` would refuse. The real name is in the bundle's
    // manifest, which is not knowable until after the download.
    let mut payload = serde_json::json!({
        "registryId": descriptor.registry_id,
        "name": descriptor.name,
        "organization": descriptor.organization,
        "version": descriptor.version,
        "description": descriptor.description,
        "tags": descriptor.tags,
        "downloadUrl": descriptor.download_url,
        "filename": descriptor.filename,
        "license": descriptor.license,
        "privacy": descriptor.privacy,
        "affiliation": affiliation,
    });
    if descriptor.advertises_name {
        if let Some(fields) = payload.as_object_mut() {
            fields.insert(
                "extensionName".to_owned(),
                Value::String(descriptor.extension_name.clone()),
            );
        }
    }
    payload
}

/// The `search_marketplace_extensions` result, from a catalog already loaded —
/// split from the load so it is testable without the network.
///
/// `caller` is the admitted capability's tier. Everything below — the hits, the
/// browse list and the count the guidance quotes — is taken from what that
/// caller may see, so a public model's answer never counts a private row.
fn marketplace_extensions_json(
    loaded: &crate::marketplace::MarketplaceCatalogLoad,
    query: Option<&str>,
    caller: crate::privacy::ProviderTier,
) -> Value {
    let visible = loaded.catalog.browse_extensions(caller);
    // Ranked, and each hit says which query terms it matched: the search is
    // shared with the skills catalog (finding F5).
    let search = query.map(|query| loaded.catalog.search_extensions(caller, query));
    let extensions: Vec<Value> = match &search {
        Some(search) => search
            .hits
            .iter()
            .map(|hit| {
                let mut payload = marketplace_descriptor_json(hit.entry);
                if let Some(fields) = payload.as_object_mut() {
                    fields.insert(
                        "matchedTerms".to_owned(),
                        serde_json::json!(hit.matched_terms),
                    );
                }
                payload
            })
            .collect(),
        None => visible
            .iter()
            .copied()
            .map(marketplace_descriptor_json)
            .collect(),
    };
    let source = match loaded.source {
        crate::marketplace::MarketplaceCatalogSource::Live => "live",
        crate::marketplace::MarketplaceCatalogSource::LastGood => "lastGood",
        crate::marketplace::MarketplaceCatalogSource::Embedded => "embedded",
    };
    let mut body = serde_json::json!({
        "source": source,
        "stale": loaded.is_stale(),
        "extensions": extensions,
    });
    if let (Some(query), Some(search), Some(fields)) = (query, &search, body.as_object_mut()) {
        fields.insert("terms".to_owned(), serde_json::json!(&search.terms));
        if search.is_empty() {
            fields.insert(
                "guidance".to_owned(),
                Value::String(no_marketplace_extension_matched(
                    &search.describe_query(query),
                    visible.len(),
                )),
            );
        }
    }
    body
}

/// What an empty `search_marketplace_extensions` says instead of an empty list
/// — the extension half of finding F5, whose skills half let a model tell a
/// user the marketplace had nothing.
///
/// ⚠ `visible` is the count shown to THIS caller. For a public model that is
/// the public rows only; the registry's full size would count the private
/// extensions Gate E keeps out of its sight.
fn no_marketplace_extension_matched(asked: &str, visible: usize) -> String {
    let available = match visible {
        1 => "1 extension is".to_owned(),
        n => format!("{n} extensions are"),
    };
    format!(
        "No marketplace extension available to this model matched {asked}. {available} \
         available to this model, so this does not mean there is nothing relevant: try a \
         shorter or more general term (one tool, data source or topic name), or call \
         search_marketplace_extensions with no query to list them all."
    )
}

fn marketplace_approval_request(
    mutation: MarketplaceMutation,
    descriptor: &crate::marketplace::MarketplaceExtensionDescriptor,
    package: Option<&ValidatedPackageInstall>,
) -> crate::pending_user_action::UserActionRequest {
    let mut arguments = marketplace_descriptor_json(descriptor)
        .as_object()
        .expect("marketplace descriptor is an object")
        .clone();
    arguments.insert(
        "action".to_owned(),
        Value::String(mutation.verb().to_owned()),
    );
    if let Some(package) = package {
        arguments.insert(
            "installedExtensionName".to_owned(),
            Value::String(package.extension_name.clone()),
        );
        arguments.insert(
            "installDirectory".to_owned(),
            Value::String(package.install_dir.display().to_string()),
        );
    }
    crate::pending_user_action::UserActionRequest::ToolApproval(
        crate::pending_user_action::ToolApprovalRequest {
            tool_name: mutation.tool_name().to_owned(),
            arguments,
            prompt: Some(format!(
                "Allow Biorouter to {} {} {} from the trusted BAAM registry?",
                mutation.verb(),
                descriptor.name,
                descriptor.version
            )),
            risk: Some(mutation.risk()),
            preview: None,
            requires_user_proof: true,
        },
    )
}

fn extension_enable_approval_request(
    extension_name: &str,
    entry: &ExtensionEntry,
) -> crate::pending_user_action::UserActionRequest {
    crate::pending_user_action::UserActionRequest::ToolApproval(
        crate::pending_user_action::ToolApprovalRequest {
            tool_name: MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE.to_owned(),
            arguments: serde_json::json!({
                "action": "enable",
                "extensionName": extension_name,
                "configKey": entry.config.key(),
                "scope": "currentChatOnly",
            })
            .as_object()
            .expect("extension enable approval is an object")
            .clone(),
            prompt: Some(format!(
                "Allow Biorouter to enable {extension_name} in this chat? Its persistent configuration remains disabled."
            )),
            risk: Some(crate::permission::tool_risk::ToolRisk::Medium),
            preview: None,
            requires_user_proof: true,
        },
    )
}

async fn await_extension_change_approval(
    actions: &Arc<crate::pending_user_action::PendingUserActions>,
    session_id: &str,
    request: crate::pending_user_action::UserActionRequest,
    ttl: Duration,
    cancel: Option<&CancellationToken>,
) -> Result<(), ExtensionManagerToolError> {
    if session_id.is_empty() {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: "Extension changes require a visible chat session for user approval"
                .to_owned(),
        });
    }
    let parked = actions.park(Some(session_id), None, request);
    match parked.wait(ttl, cancel).await {
        crate::pending_user_action::UserActionOutcome::Approved { .. } => {
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                Err(ExtensionManagerToolError::OperationFailed {
                    message:
                        "The extension change was cancelled after approval; nothing was changed"
                            .to_owned(),
                })
            } else {
                Ok(())
            }
        }
        outcome => Err(ExtensionManagerToolError::OperationFailed {
            message: format!(
                "The extension change was not made because the approval request {}.",
                outcome.refusal_detail()
            ),
        }),
    }
}

async fn trusted_marketplace_extension(
    registry_id: &str,
    caller: crate::privacy::ProviderTier,
) -> Result<crate::marketplace::MarketplaceExtensionDescriptor, ExtensionManagerToolError> {
    let loaded = crate::marketplace::load_marketplace_catalog()
        .await
        .map_err(|error| ExtensionManagerToolError::OperationFailed {
            message: error.to_string(),
        })?;
    resolve_marketplace_extension(&loaded.catalog, registry_id, caller)
}

fn resolve_marketplace_extension(
    catalog: &crate::marketplace::MarketplaceCatalog,
    registry_id: &str,
    caller: crate::privacy::ProviderTier,
) -> Result<crate::marketplace::MarketplaceExtensionDescriptor, ExtensionManagerToolError> {
    catalog
        .resolve_extension_for_install(registry_id, caller)
        .cloned()
        .map_err(|error| ExtensionManagerToolError::OperationFailed {
            message: error.to_string(),
        })
}

fn ensure_descriptor_unchanged(
    approved: &crate::marketplace::MarketplaceExtensionDescriptor,
    current: &crate::marketplace::MarketplaceExtensionDescriptor,
) -> Result<(), ExtensionManagerToolError> {
    if approved == current {
        Ok(())
    } else {
        Err(ExtensionManagerToolError::OperationFailed {
            message: format!(
                "Marketplace entry `{}` changed after approval; nothing was changed. Review and approve the current entry instead.",
                approved.registry_id
            ),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ValidatedPackageInstall {
    provenance: crate::privacy::provenance::MarketplaceInstallProvenance,
    extension_name: String,
    enabled: bool,
    config: ExtensionConfig,
    install_dir: PathBuf,
    extensions_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
struct ValidatedMarketplaceDeletion {
    descriptor: crate::marketplace::MarketplaceExtensionDescriptor,
    package: ValidatedPackageInstall,
}

async fn preflight_marketplace_deletions(
    registry_ids: &[String],
    caller: crate::privacy::ProviderTier,
) -> Result<Vec<ValidatedMarketplaceDeletion>, ExtensionManagerToolError> {
    let loaded = crate::marketplace::load_marketplace_catalog()
        .await
        .map_err(|error| ExtensionManagerToolError::OperationFailed {
            message: error.to_string(),
        })?;
    let mut plans = Vec::with_capacity(registry_ids.len());
    for registry_id in registry_ids {
        let descriptor = resolve_marketplace_extension(&loaded.catalog, registry_id, caller)?;
        let package = validated_marketplace_package(
            &descriptor,
            crate::privacy::provenance::marketplace_installs_for_registry_id(registry_id),
        )?;
        plans.push(ValidatedMarketplaceDeletion {
            descriptor,
            package,
        });
    }
    validate_unique_deletion_targets(
        plans.iter().map(|plan| {
            (
                plan.package.provenance.config_key.as_str(),
                Some(plan.package.install_dir.as_path()),
            )
        }),
        DeletionIdentifier::MarketplaceRegistryId,
    )?;
    Ok(plans)
}

/// Refuse a batch in which two identifiers name one installed thing.
///
/// Shared by both uninstall doors, because the failure it prevents is the same
/// on either: the second pass over one directory finds it already renamed
/// aside and reports a rollback nobody can act on, and the approval card the
/// user read claimed two removals when there was one. A `None` install
/// directory does NOT collide with another `None` — an MCP server configured
/// by hand owns no directory, and two of them are two distinct removals.
fn validate_unique_deletion_targets<'a>(
    targets: impl IntoIterator<Item = (&'a str, Option<&'a std::path::Path>)>,
    kind: DeletionIdentifier,
) -> Result<(), ExtensionManagerToolError> {
    let mut config_keys = std::collections::BTreeSet::new();
    let mut install_dirs = std::collections::BTreeSet::new();
    let collides = targets.into_iter().any(|(config_key, install_dir)| {
        !config_keys.insert(config_key.to_owned())
            || install_dir.is_some_and(|dir| !install_dirs.insert(dir.to_path_buf()))
    });
    if collides {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: kind.ambiguous_batch().to_owned(),
        });
    }
    Ok(())
}

fn marketplace_batch_delete_approval_request(
    plans: &[ValidatedMarketplaceDeletion],
) -> crate::pending_user_action::UserActionRequest {
    let arguments = serde_json::json!({
        "operation": "deleteExtensionPackage",
        "registryIds": plans.iter().map(|plan| plan.descriptor.registry_id.clone()).collect::<Vec<_>>(),
        "packages": plans.iter().map(|plan| serde_json::json!({
            "registryId": plan.descriptor.registry_id,
            "name": plan.descriptor.name,
            "version": plan.descriptor.version,
            "extensionName": plan.package.extension_name,
            "installDirectory": plan.package.install_dir,
        })).collect::<Vec<_>>(),
        "credentialsPreserved": true,
    })
    .as_object()
    .expect("batch deletion approval is an object")
    .clone();
    let preview = crate::conversation::tool_preview::ToolPreview::for_tool_call(
        DELETE_EXTENSION_PACKAGE_TOOL_NAME,
        &arguments,
    );
    crate::pending_user_action::UserActionRequest::ToolApproval(
        crate::pending_user_action::ToolApprovalRequest {
            tool_name: DELETE_EXTENSION_PACKAGE_TOOL_NAME.to_owned(),
            arguments,
            prompt: Some(format!(
                "Permanently delete {} installed BAAM extension package(s)?",
                plans.len()
            )),
            risk: Some(crate::permission::tool_risk::ToolRisk::High),
            preview,
            requires_user_proof: true,
        },
    )
}

fn validated_marketplace_package(
    descriptor: &crate::marketplace::MarketplaceExtensionDescriptor,
    candidates: Vec<crate::privacy::provenance::MarketplaceInstallProvenance>,
) -> Result<ValidatedPackageInstall, ExtensionManagerToolError> {
    validated_marketplace_package_at(
        descriptor,
        candidates,
        crate::extension_install::brxt::extensions_root(),
        get_all_extensions(),
    )
}

fn validated_marketplace_package_at(
    descriptor: &crate::marketplace::MarketplaceExtensionDescriptor,
    candidates: Vec<crate::privacy::provenance::MarketplaceInstallProvenance>,
    root: PathBuf,
    configured: Vec<ExtensionEntry>,
) -> Result<ValidatedPackageInstall, ExtensionManagerToolError> {
    let [provenance] = candidates.as_slice() else {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: match candidates.len() {
                0 => format!(
                    "No validated marketplace package is installed for `{}`",
                    descriptor.registry_id
                ),
                count => format!(
                    "Found {count} installed packages for `{}`; refusing an ambiguous deletion",
                    descriptor.registry_id
                ),
            },
        });
    };
    // Same predicate the lookup used, so the two cannot disagree about whether a
    // record belongs to this entry — a legacy versioned id would otherwise pass
    // the lookup and then fail here, which reads as tampering rather than as a
    // renamed id.
    if !crate::privacy::provenance::registry_id_matches(
        &provenance.registry_id,
        &descriptor.registry_id,
    ) {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: "Marketplace provenance no longer matches the selected registry entry"
                .to_owned(),
        });
    }

    let source = url::Url::parse(&provenance.source_url).map_err(|_| {
        ExtensionManagerToolError::OperationFailed {
            message: "Marketplace install provenance has an invalid source URL".to_owned(),
        }
    })?;
    if source.scheme() != "https"
        || !source.username().is_empty()
        || source.password().is_some()
        || source.port().is_some()
        || source.query().is_some()
        || source.fragment().is_some()
        || source.host_str() != descriptor.download_url.host_str()
        || !source.path().ends_with(".brxt")
    {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: "Marketplace install provenance is not a trusted .brxt source".to_owned(),
        });
    }

    let install_dir = PathBuf::from(&provenance.install_dir);
    if !install_dir.is_absolute() {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: "Marketplace install provenance does not name an absolute package directory"
                .to_owned(),
        });
    }
    let canonical_root = std::fs::canonicalize(&root).map_err(|error| {
        ExtensionManagerToolError::OperationFailed {
            message: format!("Could not validate the extensions directory: {error}"),
        }
    })?;
    let canonical_install = std::fs::canonicalize(&install_dir).map_err(|error| {
        ExtensionManagerToolError::OperationFailed {
            message: format!("Could not validate the installed package: {error}"),
        }
    })?;
    if !canonical_install.is_dir()
        || canonical_install.parent() != Some(canonical_root.as_path())
        || canonical_install
            .file_name()
            .and_then(|name| name.to_str())
            .map(crate::config::extensions::name_to_key)
            .as_deref()
            != Some(provenance.config_key.as_str())
    {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: "Marketplace package is not one direct child of the extensions directory"
                .to_owned(),
        });
    }

    let entry = configured
        .into_iter()
        .find(|entry| entry.config.key() == provenance.config_key)
        .ok_or_else(|| ExtensionManagerToolError::OperationFailed {
            message: "Marketplace package no longer has a matching extension configuration"
                .to_owned(),
        })?;
    let references_recorded_dir = match &entry.config {
        ExtensionConfig::Stdio { args, .. } => args
            .iter()
            .any(|argument| argument == &provenance.install_dir),
        _ => false,
    };
    if !references_recorded_dir {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: "Marketplace package configuration does not reference its recorded directory"
                .to_owned(),
        });
    }

    Ok(ValidatedPackageInstall {
        provenance: provenance.clone(),
        extension_name: entry.config.name(),
        enabled: entry.enabled,
        config: entry.config,
        install_dir: canonical_install,
        extensions_root: canonical_root,
    })
}

/// Unload the extension from THIS chat's live manager before its files move,
/// reporting whether it had been attached so a rollback can put it back.
///
/// Shared by both uninstall doors: the marketplace one reaches it with the
/// config key its provenance record carries, the general one with the key of
/// the entry it resolved. Nothing here is marketplace-shaped — it is the live
/// manager and a key.
async fn detach_extension_from_session(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    config_key: &str,
    config: &ExtensionConfig,
    cancel: &CancellationToken,
) -> Result<bool, ExtensionManagerToolError> {
    if cancel.is_cancelled() {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: "The extension deletion was cancelled before detaching the package".to_owned(),
        });
    }
    let was_attached = manager.is_extension_enabled(config_key).await;
    if cancel.is_cancelled() {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: "The extension deletion was cancelled before detaching the package".to_owned(),
        });
    }
    if was_attached {
        manager
            .remove_extension(&config.name())
            .await
            .map_err(|error| ExtensionManagerToolError::OperationFailed {
                message: format!("Could not detach the package from this chat: {error}"),
            })?;
        if cancel.is_cancelled() {
            return Err(restore_detached_attachment(
                manager,
                config,
                true,
                "The extension deletion was cancelled before package files changed",
            )
            .await);
        }
    }
    Ok(was_attached)
}

/// Undo a staging rename and, if the extension had been attached, re-attach it.
///
/// `staged` is `None` for an extension with no package directory of its own —
/// an MCP server configured by hand has nothing on disk to move — in which case
/// only the attachment is restored. That case is why the parameter is an
/// option rather than the caller skipping the call: a rollback that is
/// sometimes performed and sometimes silently not is the shape a missed
/// re-attach hides in.
async fn restore_staged_package(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    config: &ExtensionConfig,
    staged: Option<(&std::path::Path, &std::path::Path)>,
    was_attached: bool,
) -> Result<(), String> {
    if let Some((quarantine, install_dir)) = staged {
        std::fs::rename(quarantine, install_dir)
            .map_err(|error| format!("the staged package could not be restored: {error}"))?;
    }
    if was_attached {
        manager.add_extension(config.clone()).await.map_err(|error| {
            format!(
                "the package files were restored, but the chat attachment could not be restored: {error}"
            )
        })?;
    }
    Ok(())
}

fn provenance_removal_error(
    result: std::io::Result<bool>,
    restoration: Result<(), String>,
) -> ExtensionManagerToolError {
    let cause = match result {
        Ok(false) => "Marketplace provenance changed before deletion".to_owned(),
        Err(error) => format!("Could not update marketplace provenance: {error}"),
        Ok(true) => unreachable!("successful provenance removal has no error"),
    };
    let message = match restoration {
        Ok(()) => format!("{cause}; the staged package was restored"),
        Err(error) => format!("{cause}; {error}"),
    };
    ExtensionManagerToolError::OperationFailed { message }
}

async fn restore_detached_attachment(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    config: &ExtensionConfig,
    was_attached: bool,
    reason: &str,
) -> ExtensionManagerToolError {
    let message = if !was_attached {
        reason.to_owned()
    } else {
        match manager.add_extension(config.clone()).await {
            Ok(()) => format!("{reason}; the chat attachment was restored"),
            Err(error) => {
                format!("{reason}; the chat attachment could not be restored: {error}")
            }
        }
    };
    ExtensionManagerToolError::OperationFailed { message }
}

/// What an uninstall must drop from the provenance store, if anything.
///
/// Two shapes because there are two conditional removals, not because there are
/// two policies. [`crate::privacy::provenance::remove_marketplace_install_provenance`]
/// compares a record's install id, registry id, install directory and source
/// URL — the four fields a validated BAAM package always has — and can
/// therefore not name a record missing any of them.
/// [`crate::privacy::provenance::remove_extension_provenance_if_matches`]
/// compares the whole record instead, which is what `remove_extension` needs:
/// the records it meets were written by older builds, or by a `.brxt` install
/// with no registry id at all. Both refuse to delete a record that changed
/// under them, so a concurrent reinstall survives either way.
#[derive(Clone, Debug, PartialEq)]
enum ProvenanceTarget {
    Install(crate::privacy::provenance::MarketplaceInstallProvenance),
    Record {
        key: String,
        record: crate::privacy::provenance::ExtensionProvenance,
    },
    /// Nothing recorded where this extension came from — the ordinary case for
    /// a hand-configured MCP server, and not an error.
    Nothing,
}

impl ProvenanceTarget {
    /// `None` when there is nothing to remove; otherwise the conditional
    /// removal's own answer, where `Ok(false)` means the record changed.
    fn remove(&self) -> Option<std::io::Result<bool>> {
        match self {
            Self::Install(expected) => {
                Some(crate::privacy::provenance::remove_marketplace_install_provenance(expected))
            }
            Self::Record { key, record } => Some(
                crate::privacy::provenance::remove_extension_provenance_if_matches(key, record),
            ),
            Self::Nothing => None,
        }
    }
}

/// One uninstall's rollback subject: what to put back, and where.
///
/// Both doors stage the same way — detach, rename the package aside, remove the
/// config row, drop the provenance record — so they roll back through one
/// implementation rather than two that drift. `install_dir` is `None` for an
/// extension with no package directory of its own.
struct StagedUninstall<'a> {
    config_key: &'a str,
    entry: ExtensionEntry,
    install_dir: Option<&'a std::path::Path>,
    quarantine: Option<&'a std::path::Path>,
}

impl StagedUninstall<'_> {
    fn staged_rename(&self) -> Option<(&std::path::Path, &std::path::Path)> {
        self.quarantine.zip(self.install_dir)
    }
}

/// Returns the removed entry AND the map key it actually sat under, because the
/// rollback has to restore it under that key rather than the derived one.
async fn remove_staged_config(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    staged: &StagedUninstall<'_>,
    was_attached: bool,
) -> Result<(String, ExtensionEntry), ExtensionManagerToolError> {
    let expected_entry = staged.entry.clone();
    let config_removed = match crate::config::extensions::remove_extension_if_matches(
        staged.config_key,
        &expected_entry,
    ) {
        Ok(removed) => removed,
        Err(error) => {
            let restoration = restore_staged_package(
                manager,
                &expected_entry.config,
                staged.staged_rename(),
                was_attached,
            )
            .await;
            return Err(ExtensionManagerToolError::OperationFailed {
                message: match restoration {
                    Ok(()) => format!(
                        "Could not update the extension configuration; the staged package was restored: {error}"
                    ),
                    Err(restoration) => format!(
                        "Could not update the extension configuration: {error}; {restoration}"
                    ),
                },
            });
        }
    };
    let Some(stored_key) = config_removed else {
        let restoration = restore_staged_package(
            manager,
            &expected_entry.config,
            staged.staged_rename(),
            was_attached,
        )
        .await;
        return Err(ExtensionManagerToolError::OperationFailed {
            message: match restoration {
                Ok(()) => "The extension configuration changed before deletion; the staged package was restored"
                    .to_owned(),
                Err(error) => {
                    format!("The extension configuration changed before deletion; {error}")
                }
            },
        });
    };
    Ok((stored_key, expected_entry))
}

async fn remove_staged_provenance(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    staged: &StagedUninstall<'_>,
    provenance: &ProvenanceTarget,
    was_attached: bool,
    stored_key: String,
    expected_entry: ExtensionEntry,
) -> Result<bool, ExtensionManagerToolError> {
    let Some(provenance_result) = provenance.remove() else {
        return Ok(false);
    };
    if matches!(&provenance_result, Ok(true)) {
        return Ok(true);
    }

    let config = expected_entry.config.clone();
    let config_restored =
        crate::config::extensions::restore_extension_if_absent(stored_key, expected_entry)
            .map_err(|error| error.to_string())
            .and_then(|restored| {
                restored.then_some(()).ok_or_else(|| {
                    "a concurrent configuration replacement was preserved".to_owned()
                })
            });
    let package_restored =
        restore_staged_package(manager, &config, staged.staged_rename(), was_attached).await;
    let restoration = match (config_restored, package_restored) {
        (Ok(()), Ok(())) => Ok(()),
        (config, package) => Err(format!(
            "rollback was incomplete (config: {}; package: {})",
            config.err().unwrap_or_else(|| "restored".to_owned()),
            package.err().unwrap_or_else(|| "restored".to_owned())
        )),
    };
    Err(provenance_removal_error(provenance_result, restoration))
}

async fn delete_staged_marketplace_package(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    plan: &ValidatedMarketplaceDeletion,
    was_attached: bool,
    cancel: &CancellationToken,
    caller: crate::privacy::ProviderTier,
) -> Result<(), ExtensionManagerToolError> {
    let package = &plan.package;
    if cancel.is_cancelled() {
        return Err(restore_detached_attachment(
            manager,
            &package.config,
            was_attached,
            "The extension deletion was cancelled before package files changed",
        )
        .await);
    }
    if let Err(error) = revalidate_approved_marketplace_deletion(plan, caller).await {
        return Err(restore_detached_attachment(
            manager,
            &package.config,
            was_attached,
            &format!("The installed package changed immediately before deletion: {error}"),
        )
        .await);
    }
    let quarantine = package
        .extensions_root
        .join(format!(".delete-{}", uuid::Uuid::new_v4()));
    if let Err(error) = std::fs::rename(&package.install_dir, &quarantine) {
        return Err(restore_detached_attachment(
            manager,
            &package.config,
            was_attached,
            &format!("Could not stage the package for deletion: {error}"),
        )
        .await);
    }
    let staged = StagedUninstall {
        config_key: &package.provenance.config_key,
        entry: ExtensionEntry {
            enabled: package.enabled,
            config: package.config.clone(),
        },
        install_dir: Some(&package.install_dir),
        quarantine: Some(&quarantine),
    };
    if cancel.is_cancelled() {
        let restoration = restore_staged_package(
            manager,
            &package.config,
            staged.staged_rename(),
            was_attached,
        )
        .await;
        return Err(ExtensionManagerToolError::OperationFailed {
            message: match restoration {
                Ok(()) => "The extension deletion was cancelled; the staged package was restored"
                    .to_owned(),
                Err(error) => format!("The extension deletion was cancelled; {error}"),
            },
        });
    }

    let (stored_key, expected_entry) = remove_staged_config(manager, &staged, was_attached).await?;
    remove_staged_provenance(
        manager,
        &staged,
        &ProvenanceTarget::Install(package.provenance.clone()),
        was_attached,
        stored_key,
        expected_entry,
    )
    .await?;

    std::fs::remove_dir_all(&quarantine).map_err(|error| {
        ExtensionManagerToolError::OperationFailed {
            message: format!(
                "The extension was detached and unregistered, but its quarantined package could not be removed: {error}"
            ),
        }
    })
}

async fn revalidate_approved_marketplace_deletion(
    approved: &ValidatedMarketplaceDeletion,
    caller: crate::privacy::ProviderTier,
) -> Result<(), ExtensionManagerToolError> {
    let registry_id = approved.descriptor.registry_id.clone();
    let current = preflight_marketplace_deletions(&[registry_id], caller).await?;
    if current.first() == Some(approved) {
        Ok(())
    } else {
        Err(ExtensionManagerToolError::OperationFailed {
            message: "The marketplace entry or installed package changed after approval".to_owned(),
        })
    }
}

fn untouched_deletion_result(
    plan: &ValidatedMarketplaceDeletion,
    status: &str,
    reason: &str,
) -> Value {
    serde_json::json!({
        "registryId": plan.descriptor.registry_id,
        "extensionName": plan.package.extension_name,
        "status": status,
        "error": reason,
        "untouched": true,
        "credentialsPreserved": true,
    })
}

fn remaining_deletion_results(
    plans: &[ValidatedMarketplaceDeletion],
    status: &str,
    reason: &str,
) -> Vec<Value> {
    plans
        .iter()
        .map(|plan| untouched_deletion_result(plan, status, reason))
        .collect()
}

async fn delete_one_marketplace_package(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    plan: &ValidatedMarketplaceDeletion,
    cancel: &CancellationToken,
    caller: crate::privacy::ProviderTier,
) -> (bool, Value) {
    let result = match detach_extension_from_session(
        manager,
        &plan.package.provenance.config_key,
        &plan.package.config,
        cancel,
    )
    .await
    {
        Ok(was_attached) => {
            match delete_staged_marketplace_package(manager, plan, was_attached, cancel, caller)
                .await
            {
                Ok(()) => {
                    return (
                        true,
                        serde_json::json!({
                            "registryId": plan.descriptor.registry_id,
                            "extensionName": plan.package.extension_name,
                            "status": "deleted",
                            "detachedFromCurrentSession": was_attached,
                            "credentialsPreserved": true,
                        }),
                    );
                }
                Err(error) => error,
            }
        }
        Err(error) => error,
    };
    (
        false,
        serde_json::json!({
            "registryId": plan.descriptor.registry_id,
            "extensionName": plan.package.extension_name,
            "status": "error",
            "error": result.to_string(),
            "credentialsPreserved": true,
        }),
    )
}

async fn execute_marketplace_deletion_batch(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    plans: &[ValidatedMarketplaceDeletion],
    cancel: &CancellationToken,
    caller: crate::privacy::ProviderTier,
) -> (bool, Vec<Value>) {
    let mut all_deleted = true;
    let mut results = Vec::with_capacity(plans.len());
    for (index, plan) in plans.iter().enumerate() {
        if cancel.is_cancelled() {
            results.extend(remaining_deletion_results(
                &plans[index..],
                "cancelled",
                "The deletion batch was cancelled before this package was changed",
            ));
            return (false, results);
        }
        if let Err(error) = revalidate_approved_marketplace_deletion(plan, caller).await {
            results.extend(remaining_deletion_results(
                &plans[index..],
                "notDeleted",
                &format!(
                    "The approved batch changed before this package could be deleted: {error}"
                ),
            ));
            return (false, results);
        }
        if cancel.is_cancelled() {
            results.extend(remaining_deletion_results(
                &plans[index..],
                "cancelled",
                "The deletion batch was cancelled before this package was changed",
            ));
            return (false, results);
        }

        let (deleted, result) = delete_one_marketplace_package(manager, plan, cancel, caller).await;
        all_deleted &= deleted;
        results.push(result);
    }
    (all_deleted, results)
}

fn marketplace_deletion_report(
    registry_ids: Vec<String>,
    results: Vec<Value>,
    all_deleted: bool,
) -> Value {
    let mut report = serde_json::json!({
        "state": if all_deleted { "deleted" } else { "partial" },
        "registryIds": registry_ids,
        "results": results,
        "credentialsPreserved": true,
    });
    let single = report
        .get("results")
        .and_then(Value::as_array)
        .filter(|results| results.len() == 1)
        .and_then(|results| results.first())
        .cloned();
    if let (Some(report), Some(single)) = (
        report.as_object_mut(),
        single.as_ref().and_then(Value::as_object),
    ) {
        for key in ["registryId", "extensionName", "detachedFromCurrentSession"] {
            if let Some(value) = single.get(key) {
                report.insert(key.to_owned(), value.clone());
            }
        }
    }
    report
}

/// Everything one `remove_extension` call is about to touch, resolved from the
/// INSTALLED inventory and validated before the user is asked anything.
///
/// ⚠ `PartialEq` is load-bearing exactly as it is on
/// [`ValidatedMarketplaceDeletion`]: the post-approval re-validation compares a
/// freshly resolved plan against the approved one, so every field that could
/// change while the card was on screen has to participate in `==`.
#[derive(Clone, Debug, PartialEq)]
struct ValidatedExtensionRemoval {
    extension_name: String,
    config_key: String,
    entry: ExtensionEntry,
    extensions_root: Option<PathBuf>,
    /// The package directory this extension owns, when it owns one. `None` for
    /// an MCP server somebody configured by hand — `npx -y some-server` has no
    /// tree under `extensions/`, and deleting a directory merely NAMED after it
    /// would be deleting somebody else's package.
    install_dir: Option<PathBuf>,
    /// Slugs of the skills that directory contributes to the catalog.
    ///
    /// They are not copied into the skills root at install time:
    /// [`crate::agents::skill_catalog::roots`] treats
    /// `<extensions>/<name>/skills/` as a root of its own, so deleting the
    /// directory IS the removal. They are resolved here so the approval card
    /// can say what else disappears, and so the catalog event afterwards can
    /// say what did.
    bundled_skills: Vec<String>,
    provenance: ProvenanceTarget,
}

impl ValidatedExtensionRemoval {
    fn staged<'a>(&'a self, quarantine: Option<&'a std::path::Path>) -> StagedUninstall<'a> {
        StagedUninstall {
            config_key: &self.config_key,
            entry: self.entry.clone(),
            install_dir: self.install_dir.as_deref(),
            quarantine,
        }
    }
}

/// Resolve, gate and validate every named extension before anything is removed.
///
/// The marketplace sibling resolves each identifier against the BAAM catalog;
/// this one resolves against the machine's own inventory. That difference IS
/// the tool: an extension with no registry id is unnameable there and ordinary
/// here.
fn preflight_extension_removals(
    names: &[String],
    cap: crate::privacy::CallCapability,
) -> Result<Vec<ValidatedExtensionRemoval>, ExtensionManagerToolError> {
    preflight_extension_removals_at(
        names,
        cap,
        crate::extension_install::brxt::extensions_root(),
        get_all_extensions(),
    )
}

fn preflight_extension_removals_at(
    names: &[String],
    cap: crate::privacy::CallCapability,
    root: PathBuf,
    configured: Vec<ExtensionEntry>,
) -> Result<Vec<ValidatedExtensionRemoval>, ExtensionManagerToolError> {
    // A machine that has never installed a `.brxt` has no extensions root, and
    // that is not an error here — every entry simply resolves to no install
    // directory. The marketplace door treats the same absence as fatal because
    // a validated package IS a directory under it.
    let canonical_root = std::fs::canonicalize(&root).ok();
    let mut plans = Vec::with_capacity(names.len());
    for name in names {
        plans.push(validated_extension_removal(
            name,
            cap,
            canonical_root.as_deref(),
            &configured,
        )?);
    }
    validate_unique_deletion_targets(
        plans
            .iter()
            .map(|plan| (plan.config_key.as_str(), plan.install_dir.as_deref())),
        DeletionIdentifier::InstalledExtensionName,
    )?;
    Ok(plans)
}

/// One installed extension, gated and resolved.
///
/// ⚠ **The order of the three refusals below is the same order
/// [`check_enable_allowed_impl`] uses, and for the same reasons.** Capability
/// extensions are refused first because they are not extensions the user
/// installed; the privacy gate comes next; and the not-found answer comes LAST,
/// below the gate, because "Extension 'ucsfomopagent' not found" tells a public
/// model what this machine has installed (issue #56, finding 13). A public
/// caller asking about a private name meets one refusal whether it is
/// installed, configured off, or absent.
fn validated_extension_removal(
    name: &str,
    cap: crate::privacy::CallCapability,
    canonical_root: Option<&std::path::Path>,
    configured: &[ExtensionEntry],
) -> Result<ValidatedExtensionRemoval, ExtensionManagerToolError> {
    let refuse = |error: ErrorData| ExtensionManagerToolError::OperationFailed {
        message: error.message.to_string(),
    };

    // 1. Bundled and platform capabilities are managed elsewhere and own no
    // package directory. The skills side calls the same rule `refuse_shipped`:
    // nothing Biorouter itself put there is uninstallable through a tool.
    if crate::agents::extension_manager::resolve_bundled_extension(name).is_some() {
        return Err(refuse(
            crate::agents::extension_manager::capability_management_error(name),
        ));
    }
    let key = crate::config::extensions::name_to_key(name);
    let entry = configured
        .iter()
        .find(|entry| entry.config.key() == key)
        .cloned();
    if let Some(refusal) = entry.as_ref().and_then(|entry| {
        crate::agents::extension_manager::capability_management_refusal(&entry.config)
    }) {
        return Err(refuse(refusal));
    }

    // 2. Gate F1, through the extension manager's own door rather than a
    // second spelling of it. `persisted` is passed FALSE deliberately, and it
    // is the one argument that differs from an enable: issue #42's arm refuses
    // to re-enable what the operator pinned off, which is an argument about
    // turning something ON. Removal is the opposite direction, it is behind a
    // proof-backed approval that names the extension, and the marketplace door
    // does not consult the pin either. The tier and affiliation arms above it
    // apply unchanged, which is the whole reason to come through here.
    if let Some(refusal) = crate::privacy::refusal::extension_manager_enable_refusal(
        cap,
        name,
        entry.as_ref(),
        false,
        false,
    ) {
        return Err(refuse(refusal));
    }

    // 3. Only now may an absent name be admitted to.
    let Some(entry) = entry else {
        return Err(ExtensionManagerToolError::OperationFailed {
            message: format!(
                "Extension '{name}' is not installed. Use the exact installed name from \
                 search_available_extensions; do not retry guessed names."
            ),
        });
    };

    let install_dir = removal_install_dir(&key, &entry.config, canonical_root)?;
    let bundled_skills = install_dir
        .as_deref()
        .map(bundled_skill_slugs)
        .unwrap_or_default();
    let provenance = match crate::privacy::provenance::extension_provenance_for_key(&key) {
        Some(record) => ProvenanceTarget::Record {
            key: key.clone(),
            record,
        },
        None => ProvenanceTarget::Nothing,
    };

    Ok(ValidatedExtensionRemoval {
        extension_name: entry.config.name(),
        config_key: key,
        entry,
        extensions_root: canonical_root.map(std::path::Path::to_path_buf),
        install_dir,
        bundled_skills,
        provenance,
    })
}

/// The package directory this configuration owns, or `None` when it owns none.
///
/// ⚠ **A directory is only this extension's when the config key agrees.** The
/// marketplace door proves ownership from a provenance record; there is no such
/// record to lean on here, so ownership is established the two ways an install
/// actually creates: the entry launches out of a directory under the extensions
/// root (`--directory <dir>` in a stdio command's arguments), or a directory
/// named after the entry exists there. Both are then held to
/// `name_to_key(<directory name>) == <config key>`, which is exactly the link a
/// `.brxt` install makes — `extensions_root().join(&manifest.name)` for an
/// entry whose name is that same `manifest.name`.
///
/// An entry that launches out of ANOTHER package's directory is refused rather
/// than resolved to nothing: removing it would leave a tree with no
/// configuration pointing at it, and deleting that tree would break the
/// extension that owns it. Neither is a decision to take without saying so.
fn removal_install_dir(
    config_key: &str,
    config: &ExtensionConfig,
    canonical_root: Option<&std::path::Path>,
) -> Result<Option<PathBuf>, ExtensionManagerToolError> {
    let Some(canonical_root) = canonical_root else {
        return Ok(None);
    };
    let owns = |candidate: &std::path::Path| {
        candidate.is_dir()
            && candidate.parent() == Some(canonical_root)
            && candidate
                .file_name()
                .and_then(|name| name.to_str())
                .map(crate::config::extensions::name_to_key)
                .as_deref()
                == Some(config_key)
    };

    let mut referenced = None;
    if let ExtensionConfig::Stdio { args, .. } = config {
        for argument in args {
            let Ok(candidate) = std::fs::canonicalize(argument) else {
                continue;
            };
            if candidate.parent() != Some(canonical_root) {
                continue;
            }
            if !owns(&candidate) {
                return Err(ExtensionManagerToolError::OperationFailed {
                    message: format!(
                        "`{}` runs out of `{}`, which belongs to a different installed package; \
                         refusing to remove it. Remove that package by its own name instead.",
                        config.name(),
                        candidate.display()
                    ),
                });
            }
            referenced = Some(candidate);
        }
    }
    if referenced.is_some() {
        return Ok(referenced);
    }

    let named = std::fs::canonicalize(canonical_root.join(config.name()))
        .ok()
        .filter(|candidate| owns(candidate));
    Ok(named)
}

/// The skills `<install_dir>/skills/` contributes, by directory slug.
///
/// The same shape `read_bundled_skills` reads out of the archive — one
/// `SKILL.md` one level down — because a slug this disagreed with would name a
/// skill in the catalog event that never existed.
fn bundled_skill_slugs(install_dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(install_dir.join("skills")) else {
        return Vec::new();
    };
    let mut slugs = entries
        .flatten()
        .filter(|entry| entry.path().join("SKILL.md").is_file())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect::<Vec<_>>();
    slugs.sort();
    slugs
}

fn extension_removal_approval_request(
    plans: &[ValidatedExtensionRemoval],
) -> crate::pending_user_action::UserActionRequest {
    let arguments = serde_json::json!({
        "operation": "removeExtension",
        "extensionNames": plans.iter().map(|plan| plan.extension_name.clone()).collect::<Vec<_>>(),
        "extensions": plans.iter().map(|plan| serde_json::json!({
            "extensionName": plan.extension_name,
            "configKey": plan.config_key,
            "enabled": plan.entry.enabled,
            "installDirectory": plan.install_dir,
            "bundledSkills": plan.bundled_skills,
            "provenanceRecorded": plan.provenance != ProvenanceTarget::Nothing,
        })).collect::<Vec<_>>(),
        "credentialsPreserved": true,
    })
    .as_object()
    .expect("extension removal approval is an object")
    .clone();
    let preview = crate::conversation::tool_preview::ToolPreview::for_tool_call(
        REMOVE_EXTENSION_TOOL_NAME,
        &arguments,
    );
    crate::pending_user_action::UserActionRequest::ToolApproval(
        crate::pending_user_action::ToolApprovalRequest {
            tool_name: REMOVE_EXTENSION_TOOL_NAME.to_owned(),
            arguments,
            prompt: Some(format!(
                "Permanently remove {} installed extension(s) and their package files?",
                plans.len()
            )),
            risk: Some(crate::permission::tool_risk::ToolRisk::High),
            preview,
            requires_user_proof: true,
        },
    )
}

/// Re-resolve one approved plan and refuse if anything about it moved.
///
/// The marketplace sibling's reason applies verbatim: the approval card was on
/// screen for up to [`MARKETPLACE_APPROVAL_TTL`], and what the user approved was
/// the tree as it stood then.
fn revalidate_approved_extension_removal(
    approved: &ValidatedExtensionRemoval,
    cap: crate::privacy::CallCapability,
) -> Result<(), ExtensionManagerToolError> {
    let current =
        preflight_extension_removals(std::slice::from_ref(&approved.extension_name), cap)?;
    if current.first() == Some(approved) {
        Ok(())
    } else {
        Err(ExtensionManagerToolError::OperationFailed {
            message: "The installed extension changed after approval".to_owned(),
        })
    }
}

/// Drop `config_key` from every stored session roster that still names it.
///
/// ⚠ **Without this the uninstall is not finished, and the symptom appears
/// somewhere else entirely.** A session's `enabled_extensions.v0` holds whole
/// `ExtensionConfig`s, and `Agent::load_extensions_from_session` spawns each one
/// on resume — so an old chat reopened after the package directory is gone tries
/// to launch a deleted server and logs a load failure that looks like a broken
/// extension rather than a removed one.
///
/// Best-effort by design, and it runs AFTER the package is definitively gone:
/// pruning is cleanup of dangling references, so a partial sweep leaves exactly
/// the state that existed before the tool was written, not a half-removed
/// extension. What could not be pruned is reported rather than swallowed.
async fn prune_extension_from_sessions(
    session_manager: &Arc<crate::session::SessionManager>,
    extension_name: &str,
    config_key: &str,
) -> (usize, Vec<String>) {
    use crate::session::extension_data::{EnabledExtensionsState, ExtensionState};

    let candidates = match session_manager
        .sessions_mentioning_extension(extension_name)
        .await
    {
        Ok(candidates) => candidates,
        Err(error) => return (0, vec![format!("could not scan sessions: {error}")]),
    };

    let mut updated = 0_usize;
    let mut problems = Vec::new();
    for session_id in candidates {
        let holds_it = session_manager
            .get_extension_state(
                &session_id,
                EnabledExtensionsState::EXTENSION_NAME,
                EnabledExtensionsState::VERSION,
            )
            .await
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_value::<EnabledExtensionsState>(value).ok())
            .is_some_and(|state| {
                state
                    .extensions
                    .iter()
                    .any(|config| config.key() == config_key)
            });
        // The `LIKE` is a prefilter over one JSON column shared by every
        // per-session extension, so most candidates hold no roster entry at
        // all. Skipping them is not an optimization: `update_extension_state`
        // always writes, and a write bumps `updated_at`, which would reorder
        // the History sidebar for every session that merely quoted the name.
        if !holds_it {
            continue;
        }
        let result = session_manager
            .update_extension_state(
                &session_id,
                EnabledExtensionsState::EXTENSION_NAME,
                EnabledExtensionsState::VERSION,
                |current| {
                    let mut state = current
                        .map(|value| {
                            serde_json::from_value::<EnabledExtensionsState>(value.clone())
                        })
                        .transpose()?
                        .unwrap_or_else(|| EnabledExtensionsState::new(Vec::new()));
                    state.extensions.retain(|config| config.key() != config_key);
                    state.to_value()
                },
            )
            .await;
        match result {
            Ok(Some(_)) => updated += 1,
            // The session was deleted between the scan and the write. Nothing
            // to prune and nothing to report.
            Ok(None) => {}
            Err(error) => problems.push(format!("{session_id}: {error}")),
        }
    }
    (updated, problems)
}

/// Announce the skills that vanished with the package directory.
///
/// The install path publishes the mirror of this from `announce_install`;
/// without it a live interface keeps offering skills whose files are gone.
/// `remove_extension_if_matches` has already published the extension's own
/// removal row, which is why only the skills are named here.
fn announce_removed_skills(plan: &ValidatedExtensionRemoval) {
    if plan.bundled_skills.is_empty() {
        return;
    }
    // The catalog is a process-global snapshot behind an mtime check with a
    // one-second window, so an in-process writer must invalidate rather than
    // rely on the stat — the rule `skill_catalog` states for every writer.
    crate::agents::skill_catalog::invalidate();
    let skills = plan
        .bundled_skills
        .iter()
        .map(|slug| crate::catalog::CatalogSkillChange {
            id: slug.clone(),
            name: None,
            change: crate::catalog::CatalogEntryChange::Removed,
            source_extension_key: Some(plan.config_key.clone()),
        })
        .collect();
    crate::catalog::CatalogEvents::global().publish(
        crate::catalog::CatalogChangeReason::Uninstall,
        Vec::new(),
        skills,
        None,
    );
}

async fn remove_staged_extension(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    session_manager: &Arc<crate::session::SessionManager>,
    plan: &ValidatedExtensionRemoval,
    was_attached: bool,
    cancel: &CancellationToken,
    cap: crate::privacy::CallCapability,
) -> Result<Value, ExtensionManagerToolError> {
    let config = &plan.entry.config;
    if cancel.is_cancelled() {
        return Err(restore_detached_attachment(
            manager,
            config,
            was_attached,
            "The extension removal was cancelled before package files changed",
        )
        .await);
    }
    if let Err(error) = revalidate_approved_extension_removal(plan, cap) {
        return Err(restore_detached_attachment(
            manager,
            config,
            was_attached,
            &format!("The installed extension changed immediately before removal: {error}"),
        )
        .await);
    }

    // Staging is a rename inside the extensions root, so the removal is
    // reversible right up to the final `remove_dir_all`. An extension with no
    // directory of its own stages nothing and the config row is the whole
    // removal.
    let quarantine = match (&plan.install_dir, &plan.extensions_root) {
        (Some(install_dir), Some(root)) => {
            let quarantine = root.join(format!(".delete-{}", uuid::Uuid::new_v4()));
            if let Err(error) = std::fs::rename(install_dir, &quarantine) {
                return Err(restore_detached_attachment(
                    manager,
                    config,
                    was_attached,
                    &format!("Could not stage the package for removal: {error}"),
                )
                .await);
            }
            Some(quarantine)
        }
        _ => None,
    };
    let staged = plan.staged(quarantine.as_deref());
    if cancel.is_cancelled() {
        let restoration =
            restore_staged_package(manager, config, staged.staged_rename(), was_attached).await;
        return Err(ExtensionManagerToolError::OperationFailed {
            message: match restoration {
                Ok(()) => "The extension removal was cancelled; the staged package was restored"
                    .to_owned(),
                Err(error) => format!("The extension removal was cancelled; {error}"),
            },
        });
    }

    let (stored_key, expected_entry) = remove_staged_config(manager, &staged, was_attached).await?;
    let provenance_removed = remove_staged_provenance(
        manager,
        &staged,
        &plan.provenance,
        was_attached,
        stored_key,
        expected_entry,
    )
    .await?;

    // Past this point the removal is not reversible, which is why session
    // pruning and the catalog announcement come after it rather than before.
    let (package_removed, package_problem) = match &quarantine {
        Some(quarantine) => delete_quarantined_package(quarantine, &plan.extension_name),
        None => (None, None),
    };
    let (sessions_updated, session_problems) =
        prune_extension_from_sessions(session_manager, &plan.extension_name, &plan.config_key)
            .await;
    announce_removed_skills(plan);

    Ok(serde_json::json!({
        "extensionName": plan.extension_name,
        "status": "removed",
        "detachedFromCurrentSession": was_attached,
        "configurationRemoved": true,
        "installDirectoryRemoved": package_removed,
        "installDirectoryProblem": package_problem,
        "provenanceRecordRemoved": provenance_removed,
        "removedSkills": plan.bundled_skills,
        "sessionsUpdated": sessions_updated,
        "sessionProblems": session_problems,
        "credentialsPreserved": true,
    }))
}

/// Delete the staged package, and if that fails make sure it can no longer
/// contribute anything.
///
/// ⚠ **A quarantine directory that survives is still a SKILL ROOT.**
/// [`crate::agents::skill_catalog::roots`] enumerates every child of the
/// extensions directory that has a `skills/` subdirectory — it does not skip
/// dot-names — so a `.delete-<uuid>` left behind by a failed delete keeps
/// serving the very skills the removal just told the user were gone, now under
/// a garbage extension name. Removing the `skills/` subtree on its own is the
/// targeted fallback: whatever made the full delete fail (a busy `.venv`, a
/// permission on a build artefact) rarely applies to a directory of markdown.
///
/// The removal itself is NOT reported as a failure. The extension is genuinely
/// gone — unregistered, detached, its provenance dropped, and it cannot load
/// again — so the honest report is a success carrying the leftover as a
/// problem, not an error that hides everything that did happen.
fn delete_quarantined_package(
    quarantine: &std::path::Path,
    extension_name: &str,
) -> (Option<bool>, Option<String>) {
    let Err(error) = std::fs::remove_dir_all(quarantine) else {
        return (Some(true), None);
    };
    let skills = quarantine.join("skills");
    let orphaned_skills = skills.is_dir() && std::fs::remove_dir_all(&skills).is_err();
    tracing::warn!(
        "removed extension {extension_name} but could not delete its quarantined package at {}: {error}",
        quarantine.display()
    );
    let problem = if orphaned_skills {
        format!(
            "the extension is removed, but its files could not be deleted from {} ({error}), and              its bundled skills are still on disk there",
            quarantine.display()
        )
    } else {
        format!(
            "the extension is removed, but its files could not be deleted from {} ({error})",
            quarantine.display()
        )
    };
    (Some(false), Some(problem))
}

async fn remove_one_extension(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    session_manager: &Arc<crate::session::SessionManager>,
    plan: &ValidatedExtensionRemoval,
    cancel: &CancellationToken,
    cap: crate::privacy::CallCapability,
) -> (bool, Value) {
    let error =
        match detach_extension_from_session(manager, &plan.config_key, &plan.entry.config, cancel)
            .await
        {
            Ok(was_attached) => {
                match remove_staged_extension(
                    manager,
                    session_manager,
                    plan,
                    was_attached,
                    cancel,
                    cap,
                )
                .await
                {
                    Ok(result) => return (true, result),
                    Err(error) => error,
                }
            }
            Err(error) => error,
        };
    (
        false,
        serde_json::json!({
            "extensionName": plan.extension_name,
            "status": "error",
            "error": error.to_string(),
            "credentialsPreserved": true,
        }),
    )
}

fn untouched_removal_result(plan: &ValidatedExtensionRemoval, status: &str, reason: &str) -> Value {
    serde_json::json!({
        "extensionName": plan.extension_name,
        "status": status,
        "error": reason,
        "untouched": true,
        "credentialsPreserved": true,
    })
}

async fn execute_extension_removal_batch(
    manager: &Arc<crate::agents::extension_manager::ExtensionManager>,
    session_manager: &Arc<crate::session::SessionManager>,
    plans: &[ValidatedExtensionRemoval],
    cancel: &CancellationToken,
    cap: crate::privacy::CallCapability,
) -> (bool, Vec<Value>) {
    let mut all_removed = true;
    let mut results = Vec::with_capacity(plans.len());
    for (index, plan) in plans.iter().enumerate() {
        if cancel.is_cancelled() {
            results.extend(plans[index..].iter().map(|plan| {
                untouched_removal_result(
                    plan,
                    "cancelled",
                    "The removal batch was cancelled before this extension was changed",
                )
            }));
            return (false, results);
        }
        if let Err(error) = revalidate_approved_extension_removal(plan, cap) {
            results.extend(plans[index..].iter().map(|plan| {
                untouched_removal_result(
                    plan,
                    "notRemoved",
                    &format!(
                        "The approved batch changed before this extension could be removed: {error}"
                    ),
                )
            }));
            return (false, results);
        }

        let (removed, result) =
            remove_one_extension(manager, session_manager, plan, cancel, cap).await;
        all_removed &= removed;
        results.push(result);
    }
    (all_removed, results)
}

fn extension_removal_report(
    extension_names: Vec<String>,
    results: Vec<Value>,
    all_removed: bool,
) -> Value {
    let mut report = serde_json::json!({
        "state": if all_removed { "removed" } else { "partial" },
        "extensionNames": extension_names,
        "results": results,
        "credentialsPreserved": true,
    });
    // A single removal is reported flat as well as in `results`, so a caller
    // that asked about one extension does not have to index an array to learn
    // what happened to it. The same courtesy `marketplace_deletion_report` does.
    let single = report
        .get("results")
        .and_then(Value::as_array)
        .filter(|results| results.len() == 1)
        .and_then(|results| results.first())
        .cloned();
    if let (Some(report), Some(single)) = (
        report.as_object_mut(),
        single.as_ref().and_then(Value::as_object),
    ) {
        for key in [
            "extensionName",
            "detachedFromCurrentSession",
            "removedSkills",
            "sessionsUpdated",
        ] {
            if let Some(value) = single.get(key) {
                report.insert(key.to_owned(), value.clone());
            }
        }
    }
    report
}

/// The `manage_extensions` enable door: ask the shared enable gate, then resolve
/// the config to load.
///
/// ⚠ **Every refusal this function can give comes from
/// [`refusal::extension_enable_refusal`], which is the WHOLE of Gate F1 plus
/// #42's operator pin, in one clause order.** It used to hand-write the tier arm
/// here — `class.tier.is_private() && caller == ProviderTier::Public`, with its
/// own sentence — while the workspace's two enable doors expressed the same rule
/// through `refusal::privacy_refusal` and asked the operator pin *first*. Two
/// spellings, two orders; the workspace order reopened at `workspace_open
/// {new:{extensions}}` the exact install-state oracle finding 13 had just closed
/// here. There is now one function, called from all three doors. If a fourth
/// enable door appears, give it this one — do not write a fifth arm here.
///
/// `persisted` is #42's provenance signal (`extension_entry_is_persisted`):
/// `get_extension_entry_by_name` reads the post-injection map, where an
/// absent platform extension is injected with its default — so a default-off
/// one (e.g. `chatrecall`) shows up as `enabled: false` without any operator
/// ever writing that. Only an entry actually present in the on-disk config
/// counts as operator-disabled; injected defaults stay agent-enableable. It is a
/// parameter rather than a lookup so this whole path stays pure: testable with no
/// global config, no registry and no live extension.
///
/// `cap` is handed on WHOLE — both axes and DR-15's master opt-out off one
/// sample — rather than collapsed at the call site, so the toggle half of the
/// decision has a subject a test can hold and Task 30's structural inventory can
/// name a single `(file, fn)` pair for this row.
///
/// ⚠ **The not-found branch is BELOW the gate, and that is finding 13.**
/// "Extension 'ucsfomopagent' not found" tells a public model what this machine
/// has installed — the same secret the sibling finding stopped the catalogue
/// printing outright. Asking the gate first collapses every private name a public
/// caller can ask about onto one refusal, whether it is installed,
/// configured-off, or absent entirely. Nothing is lost in the other direction: a
/// PRIVATE caller reaches this branch exactly as before, and so does a public
/// caller asking about a public extension — which is every case it was written
/// for.
///
/// [`refusal::extension_enable_refusal`]: crate::privacy::refusal::extension_enable_refusal
fn check_enable_allowed(
    entry: Option<ExtensionEntry>,
    persisted: bool,
    extension_name: &str,
    cap: crate::privacy::CallCapability,
) -> Result<ExtensionConfig, ErrorData> {
    check_enable_allowed_impl(entry, persisted, extension_name, cap, false)
}

fn check_enable_allowed_with_user_grant(
    entry: Option<ExtensionEntry>,
    persisted: bool,
    extension_name: &str,
    cap: crate::privacy::CallCapability,
) -> Result<ExtensionConfig, ErrorData> {
    check_enable_allowed_impl(entry, persisted, extension_name, cap, true)
}

fn check_enable_allowed_impl(
    entry: Option<ExtensionEntry>,
    persisted: bool,
    extension_name: &str,
    cap: crate::privacy::CallCapability,
    user_granted: bool,
) -> Result<ExtensionConfig, ErrorData> {
    if crate::agents::extension_manager::resolve_bundled_extension(extension_name).is_some() {
        return Err(crate::agents::extension_manager::capability_management_error(extension_name));
    }
    if let Some(refusal) = entry.as_ref().and_then(|entry| {
        crate::agents::extension_manager::capability_management_refusal(&entry.config)
    }) {
        return Err(refusal);
    }
    if let Some(err) = crate::privacy::refusal::extension_manager_enable_refusal(
        cap,
        extension_name,
        entry.as_ref(),
        persisted,
        user_granted,
    ) {
        return Err(err);
    }

    let Some(entry) = entry else {
        return Err(ErrorData::new(
            ErrorCode::RESOURCE_NOT_FOUND,
            format!(
                "Extension '{}' not found. Use the exact installed name from search_available_extensions; do not retry guessed names. If absent from that inventory, use search_marketplace_extensions, then install_extension with its registry id after user approval.",
                extension_name
            ),
            None,
        ));
    };
    Ok(entry.config)
}

impl ExtensionManagerClient {
    pub fn new(context: PlatformExtensionContext) -> Result<Self> {
        let info = InitializeResult {
            protocol_version: ProtocolVersion::V_2025_03_26,
            capabilities: ServerCapabilities {
                tasks: None,
                tools: Some(ToolsCapability {
                    list_changed: Some(false),
                }),
                resources: None,
                prompts: None,
                completions: None,
                experimental: None,
                logging: None,
            },
            server_info: Implementation {
                name: EXTENSION_NAME.to_string(),
                title: Some(EXTENSION_NAME.to_string()),
                version: "1.0.0".to_string(),
                icons: None,
                website_url: None,
            },
            instructions: Some(indoc! {r#"
                Extension Management

                Use these tools to discover installed extensions, attach and detach them, search the
                trusted BAAM marketplace, install a package from it, permanently delete an installed
                package, and review resources.

                Available tools:
                - search_available_extensions: List installed extensions and their exact names, including any that are installed but not attached to this chat
                - manage_extensions: enable or disable an installed extension (`action` is `enable` or `disable`, never `attach`/`detach`)
                - search_marketplace_extensions: Search trusted BAAM entries; omit the query to browse everything visible to this model
                - install_extension: Install an exact trusted registry id after user approval
                - delete_extension_package: Permanently delete one or several validated marketplace packages after one user approval
                - remove_extension: Permanently remove one or several installed extensions by installed name, marketplace or not
                - list_resources/read_resource: Resource tools, when they are advertised for the current session

                When you lack the tools needed to complete a task, use search_available_extensions first
                to discover what extensions can help.

                Use manage_extensions with the exact installed name returned by
                search_available_extensions, not a marketplace title or registry id. If absent,
                use search_marketplace_extensions and install_extension; do not retry guessed names.
                A bundled skill or package files alone do not mean an extension is configured.
                Built-in and platform capabilities are managed separately and this tool refuses them.
                A successful change applies immediately in the current turn. Its response names the
                exact availableTools or removedTools; call an available tool directly by that name,
                and never call a removed tool unless the extension is attached again.
                Use search_marketplace_extensions (omit query to browse) to obtain an exact registry
                id, then install_extension when the extension is not installed at all. An install
                result with state attached also names
                immediately callable availableTools; state installed means attach it before use.
                Never provide a download URL or install
                one by running shell commands, and NEVER ask the user to type an API key,
                password or token into the chat — install_extension opens Biorouter's own
                approval and credential dialogs, and a credential in a chat message cannot configure anything.
                delete_extension_package validates the entire bounded batch before removing any package,
                reports each result, and deliberately preserves shared credentials.
                Use remove_extension to uninstall anything that did not come from the marketplace — a
                sideloaded .brxt or an MCP server configured by hand — naming it by its exact installed
                name; delete_extension_package cannot name one at all, and neither tool is a reason to
                edit config.yaml or the provenance store yourself. It removes the configuration entry,
                the package directory and its bundled skills, detaches the extension from this chat,
                clears it from the saved rosters of other chats so reopening one does not relaunch it,
                and preserves shared credentials. Another chat already running keeps the extension
                loaded until it is reopened.
                Use list_resources and read_resource only when they appear in the current tool catalog;
                they are omitted when no loaded extension supports resources.
            "#}.to_string()),
        };

        Ok(Self { info, context })
    }

    /// `admitted` is the capability THIS tool call was admitted on, taken
    /// straight off its `McpMeta` and threaded into Gate E's catalogue filter
    /// (issue #56, finding 13). The manager must not sample its own, for the
    /// reason every other handler in this file gives: the read would happen
    /// inside the driven future, an unbounded wall-clock gap past admission, and
    /// a fresh one there is what would let a Public-admitted call read a private
    /// connector's name and marketplace description out of the catalogue after
    /// the user switched models mid-turn.
    async fn handle_search_available_extensions(
        &self,
        admitted: crate::privacy::CallCapability,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        if let Some(weak_ref) = &self.context.extension_manager {
            if let Some(extension_manager) = weak_ref.upgrade() {
                match extension_manager
                    .search_available_extensions(admitted)
                    .await
                {
                    Ok(content) => Ok(content),
                    Err(e) => Err(ExtensionManagerToolError::OperationFailed {
                        message: format!("Failed to search available extensions: {}", e.message),
                    }),
                }
            } else {
                Err(ExtensionManagerToolError::ManagerUnavailable)
            }
        } else {
            Err(ExtensionManagerToolError::ManagerUnavailable)
        }
    }

    async fn handle_search_marketplace_extensions(
        &self,
        arguments: Option<JsonObject>,
        cap: crate::privacy::CallCapability,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        // No arguments at all is the browse case, not a missing-parameter error:
        // this tool absorbed `browse_marketplace_extensions`, whose whole schema
        // was `{}`.
        let params: SearchMarketplaceExtensionsParams = match arguments {
            Some(arguments) => serde_json::from_value(Value::Object(arguments))?,
            None => SearchMarketplaceExtensionsParams { query: None },
        };
        let query = params
            .query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty());
        self.marketplace_extensions(query, cap).await
    }

    async fn marketplace_extensions(
        &self,
        query: Option<&str>,
        cap: crate::privacy::CallCapability,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        let loaded = crate::marketplace::load_marketplace_catalog()
            .await
            .map_err(|error| ExtensionManagerToolError::OperationFailed {
                message: error.to_string(),
            })?;
        let body = marketplace_extensions_json(&loaded, query, cap.tier());
        Ok(vec![Content::text(
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_owned()),
        )])
    }

    /// `cap` is the capability THIS tool call was admitted on, taken straight
    /// off its `McpMeta` — never re-sampled here, for the reason
    /// [`crate::privacy::CallCapability`] exists: enabling an extension runs
    /// inside the driven future, an unbounded wall-clock gap past admission,
    /// and a fresh read there is what would let a Public-admitted call spawn a
    /// private server after the user switched models mid-turn.
    async fn handle_manage_extensions(
        &self,
        arguments: Option<JsonObject>,
        session_id: String,
        cap: crate::privacy::CallCapability,
        cancel: CancellationToken,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        let arguments = arguments.ok_or(ExtensionManagerToolError::MissingParameter {
            param_name: "arguments".to_string(),
        })?;

        let params: ManageExtensionsParams =
            serde_json::from_value(serde_json::Value::Object(arguments))?;

        let action = params.action;
        let extension_name = params.extension_name;
        let first = self
            .manage_extensions_impl(
                action.clone(),
                extension_name.clone(),
                cap,
                false,
                &session_id,
            )
            .await
            .map_err(|error| ExtensionManagerToolError::OperationFailed {
                message: error.message.to_string(),
            });
        let result = if action == ManageExtensionAction::Enable && first.is_err() {
            let approved_entry = get_extension_entry_by_name(&extension_name);
            let persisted = approved_entry
                .as_ref()
                .is_some_and(|entry| extension_entry_is_persisted(&entry.config.name()));
            let can_grant = approved_entry.as_ref().is_some_and(|entry| {
                !entry.enabled
                    && persisted
                    && check_enable_allowed_with_user_grant(
                        Some(entry.clone()),
                        true,
                        &extension_name,
                        cap,
                    )
                    .is_ok()
            });
            if can_grant {
                let approved_entry = approved_entry.expect("grant candidate has an entry");
                await_extension_change_approval(
                    crate::pending_user_action::PendingUserActions::global(),
                    &session_id,
                    extension_enable_approval_request(&extension_name, &approved_entry),
                    MARKETPLACE_APPROVAL_TTL,
                    Some(&cancel),
                )
                .await?;

                let current_entry = get_extension_entry_by_name(&extension_name);
                let unchanged = current_entry.as_ref().is_some_and(|current| {
                    extension_entry_is_persisted(&current.config.name())
                        && current.enabled == approved_entry.enabled
                        && current.config == approved_entry.config
                });
                if !unchanged {
                    return Err(ExtensionManagerToolError::OperationFailed {
                        message: "The extension configuration changed after approval; nothing was enabled"
                            .to_owned(),
                    });
                }
                let current_entry = current_entry.expect("unchanged entry exists");
                let config = check_enable_allowed_with_user_grant(
                    Some(current_entry),
                    true,
                    &extension_name,
                    cap,
                )
                .map_err(|error| ExtensionManagerToolError::OperationFailed {
                    message: error.message.to_string(),
                })?;
                self.attach_extension_to_session(extension_name, config, cap, &session_id)
                    .await
                    .map_err(|error| ExtensionManagerToolError::OperationFailed {
                        message: error.message.to_string(),
                    })
            } else {
                first
            }
        } else {
            first
        };

        result
    }

    async fn install_report_json(
        &self,
        report: &crate::extension_install::InstallReport,
        cap: crate::privacy::CallCapability,
    ) -> String {
        use crate::extension_install::InstallState;

        let mut payload = serde_json::to_value(report).unwrap_or_else(|_| serde_json::json!({}));
        let Some(fields) = payload.as_object_mut() else {
            return "{}".to_owned();
        };
        match &report.state {
            InstallState::Attached => {
                let mut available_tools = Vec::new();
                if let (Some(extension_name), Some(manager)) = (
                    report.extension_name.as_deref(),
                    self.context
                        .extension_manager
                        .as_ref()
                        .and_then(|weak| weak.upgrade()),
                ) {
                    let extension_key = crate::config::extensions::name_to_key(extension_name);
                    if let Ok(tools) = manager
                        .get_prefixed_tools_for_extension_and_capability(&extension_key, cap)
                        .await
                    {
                        available_tools = tools
                            .into_iter()
                            .map(|tool| tool.name.to_string())
                            .collect();
                        available_tools.sort();
                    }
                }
                fields.insert(
                    "availableTools".to_owned(),
                    serde_json::json!(available_tools),
                );
                fields.insert(
                    "toolAvailability".to_owned(),
                    serde_json::json!("immediate"),
                );
                fields.insert(
                    "guidance".to_owned(),
                    serde_json::json!(
                        "The availableTools are callable now in this turn. Use the exact tool name needed for the user's task."
                    ),
                );
            }
            InstallState::Installed => {
                fields.insert(
                    "toolAvailability".to_owned(),
                    serde_json::json!("notAttached"),
                );
                // The operator pin is a DIFFERENT reason for the same state, and
                // conflating them tells the model to do the one thing it must
                // not: retry the enable. Say which it is.
                fields.insert(
                    "guidance".to_owned(),
                    serde_json::json!(if report.operator_pinned_off {
                        "The package was updated, but the operator has this extension disabled in \
                         the Biorouter configuration and an install does not overturn that. Do not \
                         try to enable it. Tell the user it is installed and switched off, and that \
                         they can turn it on in Settings > Extensions."
                    } else {
                        "The package is installed but its tools are not callable in this chat. \
                         Attach the extension before using them."
                    }),
                );
            }
            _ => {}
        }
        serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Install a marketplace extension, asking the *user* for any credentials.
    ///
    /// ⚠ **This tool exists so that the model never has to ask for a secret.**
    /// Before it, an agent told to "install the Playwright extension" had three
    /// options and all of them were wrong: tell the user to paste a token into
    /// the chat, run a shell command with the token in `ps` and the shell
    /// history, or install without it and report success for something that
    /// cannot authenticate. The install parks on a credential card the user
    /// answers in Biorouter's own dialog, and this tool learns only which key
    /// NAMES were configured.
    ///
    /// The report it returns is the same [`InstallReport`] the CLI prints, and
    /// its shape has nowhere a value can sit — deliberately, because this one is
    /// serialised straight into a tool result.
    ///
    /// [`InstallReport`]: crate::extension_install::InstallReport
    async fn handle_install_extension(
        &self,
        arguments: Option<JsonObject>,
        session_id: String,
        cap: crate::privacy::CallCapability,
        cancel: CancellationToken,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        use crate::extension_install::{
            CredentialPolicy, ExtensionInstallTransaction, InstallSource, InstallState,
            DEFAULT_CREDENTIAL_TTL,
        };

        let arguments = arguments.ok_or(ExtensionManagerToolError::MissingParameter {
            param_name: "arguments".to_string(),
        })?;
        let params: InstallExtensionParams =
            serde_json::from_value(serde_json::Value::Object(arguments))?;

        // This preflight deliberately ignores the diagnostic master switch.
        // Public model capability never authorizes a private marketplace entry.
        let approved = trusted_marketplace_extension(&params.registry_id, cap.tier()).await?;
        let approval = marketplace_approval_request(MarketplaceMutation::Install, &approved, None);
        await_extension_change_approval(
            crate::pending_user_action::PendingUserActions::global(),
            &session_id,
            approval,
            MARKETPLACE_APPROVAL_TTL,
            Some(&cancel),
        )
        .await?;

        // Approval binds the exact daemon-owned descriptor, not merely its id.
        // A registry update between the card and the click gets a new card.
        let current = trusted_marketplace_extension(&params.registry_id, cap.tier()).await?;
        ensure_descriptor_unchanged(&approved, &current)?;

        let attach_refusal = crate::privacy::refusal::extension_enable_refusal(
            cap,
            &current.extension_name,
            None,
            false,
        )
        .map(|error| error.message.to_string());
        let mut transaction = ExtensionInstallTransaction::new(InstallSource::Marketplace {
            registry_id: current.registry_id.clone(),
            url: current.download_url.as_str().to_owned(),
        })
        .enabled(params.enable);
        if attach_refusal.is_none() {
            if let Some(weak) = &self.context.extension_manager {
                transaction = transaction.attach_to(weak.clone());
            }
            // ⚠ The pre-flight above asked about `current.extension_name`, which
            // is the REGISTRY's name — and when the registry omits one, that is
            // the registry ID. The extension's real name comes from the
            // downloaded bundle's manifest, and the two demonstrably differ in
            // production (SPOKEAgent advertised `spokeagent-0.4.1`, installed as
            // `spokeagent`). So the same gate is asked again with the real name,
            // at the only point it is knowable.
            transaction = transaction.guard_attach(move |installed_name| {
                crate::privacy::refusal::extension_enable_refusal(cap, installed_name, None, false)
                    .map(|error| error.message.to_string())
            });
        }

        // `session_id` is moved into the credential policy below, and the
        // durability write after the install still needs it.
        let owning_session = session_id.clone();
        let report = transaction
            .run(
                CredentialPolicy::Ask {
                    session_id: Some(session_id),
                    owner: None,
                    ttl: DEFAULT_CREDENTIAL_TTL,
                },
                Some(&cancel),
            )
            .await;

        let json = self.install_report_json(&report, cap).await;
        // An install that ATTACHED is the same mutation as an enable, so it is
        // made durable and announced the same way. `InstallState::Attached` is
        // the correct condition and must not be widened: `attach_to` is wired
        // only when the pre-flight passed and `enable` was asked for, and a
        // second `guard_attach` can still refuse against the *installed*
        // manifest name (registry `spokeagent-0.4.1` vs installed
        // `spokeagent`), in which case the state is `Installed`, not
        // `Attached`.
        if matches!(report.state, InstallState::Attached) {
            if let Some(extension_manager) = self
                .context
                .extension_manager
                .as_ref()
                .and_then(|weak| weak.upgrade())
            {
                if let Err(error) = crate::agents::session_extensions::record(
                    &self.context.session_manager,
                    &extension_manager,
                    &owning_session,
                )
                .await
                {
                    return Err(ExtensionManagerToolError::OperationFailed {
                        message: format!(
                            "the extension was installed and attached for this turn but the \
                             session could not record it, so the live state and the saved \
                             roster have diverged: {error}"
                        ),
                    });
                }
                crate::catalog::CatalogEvents::global().publish_session_refresh(&owning_session);
            }
        }
        match &report.state {
            InstallState::NeedsCredentials { .. } | InstallState::Cancelled => {
                // Not an error: a person declined or could not be asked. Say so
                // plainly, and — critically — do NOT suggest asking them for the
                // value in chat.
                Ok(vec![Content::text(format!(
                    "{json}\n\nThe extension was not registered. Do not ask the user to type any \
                     credential into this chat: a value in a chat message cannot configure \
                     anything and would expose it. Tell them the Biorouter dialog is waiting, or \
                     that they can run `biorouter extension configure <name>` at a terminal."
                ))])
            }
            InstallState::Failed { reason } => Err(ExtensionManagerToolError::OperationFailed {
                message: reason.clone(),
            }),
            _ => Ok(vec![Content::text(match attach_refusal {
                Some(refusal) if params.enable => format!(
                    "{json}\n\nThe package is installed but was not attached to this chat: {refusal}"
                ),
                _ => json,
            })]),
        }
    }

    async fn handle_delete_extension_package(
        &self,
        arguments: Option<JsonObject>,
        session_id: String,
        cap: crate::privacy::CallCapability,
        cancel: CancellationToken,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        let arguments = arguments.ok_or(ExtensionManagerToolError::MissingParameter {
            param_name: "arguments".to_owned(),
        })?;
        let params: DeleteExtensionPackageParams =
            serde_json::from_value(Value::Object(arguments))?;

        let registry_ids = preflight_delete_registry_ids(params)?;
        let approved = preflight_marketplace_deletions(&registry_ids, cap.tier()).await?;
        await_extension_change_approval(
            crate::pending_user_action::PendingUserActions::global(),
            &session_id,
            marketplace_batch_delete_approval_request(&approved),
            MARKETPLACE_APPROVAL_TTL,
            Some(&cancel),
        )
        .await?;
        if cancel.is_cancelled() {
            return Err(ExtensionManagerToolError::OperationFailed {
                message: "The extension deletion was cancelled; nothing was deleted".to_owned(),
            });
        }
        let current = preflight_marketplace_deletions(&registry_ids, cap.tier()).await?;
        if current != approved {
            return Err(ExtensionManagerToolError::OperationFailed {
                message: "A marketplace entry or installed package changed after approval; nothing was deleted"
                    .to_owned(),
            });
        }

        let manager = self
            .context
            .extension_manager
            .as_ref()
            .and_then(|weak| weak.upgrade())
            .ok_or(ExtensionManagerToolError::ManagerUnavailable)?;
        let (all_deleted, results) =
            execute_marketplace_deletion_batch(&manager, &current, &cancel, cap.tier()).await;
        let report = marketplace_deletion_report(registry_ids, results, all_deleted);
        Ok(vec![Content::text(
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_owned()),
        )])
    }

    /// Uninstall an installed extension the marketplace cannot name (#164).
    ///
    /// The sibling of [`Self::handle_delete_extension_package`], keyed on the
    /// installed name instead of a BAAM registry id, and running the SAME
    /// transaction: one bounded, de-duplicated, fully preflighted batch; one
    /// proof-backed approval; a re-validation after it that aborts on anything
    /// that moved while the card was on screen; then per-extension detach,
    /// stage, unregister, drop provenance, delete, prune sessions, announce.
    ///
    /// ⚠ **Credentials are deliberately NOT touched**, exactly as on the
    /// marketplace path. An API key may be shared between extensions — the same
    /// UCSF credential unlocks more than one connector — so revoking it here
    /// would break an extension the user never asked to remove, and the value
    /// is unrecoverable. `package_deletion_path_never_revokes_or_removes_credentials`
    /// holds both handlers to it.
    async fn handle_remove_extension(
        &self,
        arguments: Option<JsonObject>,
        session_id: String,
        cap: crate::privacy::CallCapability,
        cancel: CancellationToken,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        let arguments = arguments.ok_or(ExtensionManagerToolError::MissingParameter {
            param_name: "arguments".to_owned(),
        })?;
        let params: RemoveExtensionParams = serde_json::from_value(Value::Object(arguments))?;

        let extension_names = preflight_remove_extension_names(params)?;
        let approved = preflight_extension_removals(&extension_names, cap)?;
        await_extension_change_approval(
            crate::pending_user_action::PendingUserActions::global(),
            &session_id,
            extension_removal_approval_request(&approved),
            MARKETPLACE_APPROVAL_TTL,
            Some(&cancel),
        )
        .await?;
        if cancel.is_cancelled() {
            return Err(ExtensionManagerToolError::OperationFailed {
                message: "The extension removal was cancelled; nothing was removed".to_owned(),
            });
        }
        // The re-validation that makes the batch atomic against a tree that
        // changed while the user was deciding. Without it an approval for one
        // configuration can be spent on another.
        let current = preflight_extension_removals(&extension_names, cap)?;
        if current != approved {
            return Err(ExtensionManagerToolError::OperationFailed {
                message: "An installed extension changed after approval; nothing was removed"
                    .to_owned(),
            });
        }

        let manager = self
            .context
            .extension_manager
            .as_ref()
            .and_then(|weak| weak.upgrade())
            .ok_or(ExtensionManagerToolError::ManagerUnavailable)?;
        let (all_removed, results) = execute_extension_removal_batch(
            &manager,
            &self.context.session_manager,
            &current,
            &cancel,
            cap,
        )
        .await;
        // The extension rows were announced by `remove_extension_if_matches` and
        // the skill rows by `announce_removed_skills`; this wakes the chat that
        // asked, whose own tool roster just changed under it.
        if !session_id.is_empty() {
            crate::catalog::CatalogEvents::global().publish_session_refresh(&session_id);
        }
        let report = extension_removal_report(extension_names, results, all_removed);
        Ok(vec![Content::text(
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_owned()),
        )])
    }

    async fn manage_extensions_impl(
        &self,
        action: ManageExtensionAction,
        extension_name: String,
        cap: crate::privacy::CallCapability,
        user_granted: bool,
        session_id: &str,
    ) -> Result<Vec<Content>, ErrorData> {
        let extension_manager = self
            .context
            .extension_manager
            .as_ref()
            .and_then(|weak| weak.upgrade())
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Extension manager is no longer available".to_string(),
                    None,
                )
            })?;

        if action == ManageExtensionAction::Disable {
            // Issue #56 Gate F1, the DISABLE half (finding 14). This branch used
            // to return before any privacy decision existed on the path: `cap`
            // was in scope, and `remove_extension` ran without consulting it. A
            // chat on a public model could therefore unload the clinical
            // connector — a server Gate E keeps out of that model's tool list
            // entirely, refuses every call into, and (since finding 13) will not
            // even name in the catalogue. Being unable to see a connector while
            // being able to unload it is the disagreement this closes.
            //
            // The gate is `assert_extension_manageable`, which is
            // `assert_extension_reachable` verbatim, so discovery and management
            // answer with one function rather than two rules — including the
            // inverted unknown-name default that keeps this refusal from being
            // the existence oracle the leak next door was.
            extension_manager
                .assert_extension_manageable(&extension_name, cap)
                .await?;
            let extension_key = crate::config::extensions::name_to_key(&extension_name);
            let mut removed_tools = extension_manager
                .get_prefixed_tools_for_extension_and_capability(&extension_key, cap)
                .await
                .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?
                .into_iter()
                .map(|tool| tool.name.to_string())
                .collect::<Vec<_>>();
            removed_tools.sort();
            extension_manager
                .remove_extension(&extension_name)
                .await
                .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
            // A mutation is persisted before it is reported — the rule
            // `routes/skills.rs` states for the other half of the catalog.
            // Reporting `"detached"` over an unwritten row is what left the
            // popup showing an extension the turn had already unloaded, and
            // restored it on the next reload.
            //
            // There is deliberately NO rollback here: the config has already
            // left the manager, and a re-add can fail on its own (subprocess
            // spawn), so a best-effort restore would add a second failure path
            // no test could tell from the first. The error names the divergence
            // instead.
            crate::agents::session_extensions::record(
                &self.context.session_manager,
                &extension_manager,
                session_id,
            )
            .await
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!(
                        "the extension was detached for this turn but the session could not \
                         record it, so the live state and the saved roster have diverged: {e}"
                    ),
                    None,
                )
            })?;
            crate::catalog::CatalogEvents::global().publish_session_refresh(session_id);
            return Ok(vec![Content::text(
                serde_json::json!({
                    "extensionName": extension_name,
                    "sessionState": "detached",
                    "persistentConfigurationChanged": false,
                    "removedTools": removed_tools,
                    "toolAvailability": "revokedImmediately",
                    "guidance": "The removedTools are unavailable now. Do not call them unless the extension is attached again.",
                })
                .to_string(),
            )]);
        }

        let entry = get_extension_entry_by_name(&extension_name);
        let persisted = entry
            .as_ref()
            .is_some_and(|entry| extension_entry_is_persisted(&entry.config.name()));
        // The capability is handed to Gate F1 WHOLE — both axes, one sample.
        // The collapse DR-15's opt-out performs lives inside
        // `check_enable_allowed`, so the toggle half of this decision has a
        // subject a test can hold; doing it here left it with none.
        let config = if user_granted {
            check_enable_allowed_with_user_grant(entry, persisted, &extension_name, cap)?
        } else {
            check_enable_allowed(entry, persisted, &extension_name, cap)?
        };

        self.attach_extension_to_session(extension_name, config, cap, session_id)
            .await
    }

    async fn attach_extension_to_session(
        &self,
        extension_name: String,
        config: ExtensionConfig,
        cap: crate::privacy::CallCapability,
        session_id: &str,
    ) -> Result<Vec<Content>, ErrorData> {
        let extension_manager = self
            .context
            .extension_manager
            .as_ref()
            .and_then(|weak| weak.upgrade())
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Extension manager is no longer available".to_owned(),
                    None,
                )
            })?;
        let extension_key = config.key();
        // Sampled BEFORE the add, so the rollback below can tell a true
        // false->true transition from a re-add of something `/ext:`, a config
        // default or an earlier call had already loaded. The manager holds one
        // entry per key, so an unconditional rollback would unload an extension
        // this call never brought in.
        let was_enabled = extension_manager.is_extension_enabled(&extension_key).await;
        extension_manager
            .add_extension(config)
            .await
            .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?;
        let mut available_tools = extension_manager
            .get_prefixed_tools_for_extension_and_capability(&extension_key, cap)
            .await
            .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), None))?
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        available_tools.sort();
        // A mutation is persisted before it is reported (`routes/skills.rs`).
        // Leaving the write to the reply loop's post-batch block meant
        // `"attached"` was returned over a row that had not been written yet,
        // and a failure there was a `warn!` with no signal to any surface.
        if let Err(error) = crate::agents::session_extensions::record(
            &self.context.session_manager,
            &extension_manager,
            session_id,
        )
        .await
        {
            if !was_enabled {
                let _ = extension_manager.remove_extension(&extension_name).await;
            }
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!(
                    "the extension could not be recorded on this session, so it was not \
                     attached: {error}"
                ),
                None,
            ));
        }
        crate::catalog::CatalogEvents::global().publish_session_refresh(session_id);
        Ok(vec![Content::text(
            serde_json::json!({
                "extensionName": extension_name,
                "sessionState": "attached",
                "persistentConfigurationChanged": false,
                "availableTools": available_tools,
                "toolAvailability": "immediate",
                "guidance": "The availableTools are callable now in this turn. Use the exact tool name needed for the user's task.",
            })
            .to_string(),
        )])
    }

    /// `admitted` is the capability THIS tool call was admitted on, taken
    /// straight off its `McpMeta` and threaded into Gate C's sibling guard
    /// (issue #56). The manager must not sample its own: this runs inside the
    /// driven future, an unbounded wall-clock gap past admission, and a fresh
    /// read there is what would let a Public-admitted call list a private
    /// extension's resources after the user switched models mid-turn.
    async fn handle_list_resources(
        &self,
        arguments: Option<JsonObject>,
        admitted: crate::privacy::CallCapability,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        if let Some(weak_ref) = &self.context.extension_manager {
            if let Some(extension_manager) = weak_ref.upgrade() {
                let params = arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                match extension_manager
                    .list_resources(
                        params,
                        Some(admitted),
                        tokio_util::sync::CancellationToken::default(),
                    )
                    .await
                {
                    Ok(content) => Ok(content),
                    Err(e) => Err(ExtensionManagerToolError::OperationFailed {
                        message: format!("Failed to list resources: {}", e.message),
                    }),
                }
            } else {
                Err(ExtensionManagerToolError::ManagerUnavailable)
            }
        } else {
            Err(ExtensionManagerToolError::ManagerUnavailable)
        }
    }

    /// `admitted`: see [`Self::handle_list_resources`]. `read_resource` with no
    /// `extension_name` fans out over every installed extension, so the value
    /// threaded here decides which servers this call is allowed to probe.
    async fn handle_read_resource(
        &self,
        arguments: Option<JsonObject>,
        admitted: crate::privacy::CallCapability,
    ) -> Result<Vec<Content>, ExtensionManagerToolError> {
        if let Some(weak_ref) = &self.context.extension_manager {
            if let Some(extension_manager) = weak_ref.upgrade() {
                let params = arguments
                    .map(serde_json::Value::Object)
                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                match extension_manager
                    .read_resource_tool(
                        params,
                        Some(admitted),
                        tokio_util::sync::CancellationToken::default(),
                    )
                    .await
                {
                    Ok(content) => Ok(content),
                    Err(e) => Err(ExtensionManagerToolError::OperationFailed {
                        message: format!("Failed to read resource: {}", e.message),
                    }),
                }
            } else {
                Err(ExtensionManagerToolError::ManagerUnavailable)
            }
        } else {
            Err(ExtensionManagerToolError::ManagerUnavailable)
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn get_tools(&self) -> Vec<Tool> {
        let mut tools = Self::tools_for(crate::pending_user_action::user_proof_available());

        // Only add resource tools if extension manager supports resources
        if let Some(weak_ref) = &self.context.extension_manager {
            if let Some(extension_manager) = weak_ref.upgrade() {
                if extension_manager.supports_resources().await {
                    tools.extend([
                        Tool::new(
                            LIST_RESOURCES_TOOL_NAME.to_string(),
                            indoc! {r#"
            List resources from an extension(s).

            Resources allow extensions to share data that provide context to LLMs, such as
            files, database schemas, or application-specific information. This tool lists resources
            in the provided extension, and returns a list for the user to browse. If no extension
            is provided, the tool will search all extensions for the resource.
        "#}.to_string(),
                            Arc::new(
                                serde_json::to_value(schema_for!(ListResourcesParams))
                                    .expect("Failed to serialize schema")
                                    .as_object()
                                    .expect("Schema must be an object")
                                    .clone()
                            ),
                        ).annotate(ToolAnnotations {
                            title: Some("List resources".to_string()),
                            read_only_hint: Some(true),
                            destructive_hint: Some(false),
                            idempotent_hint: Some(false),
                            open_world_hint: Some(false),
                        }),
                        Tool::new(
                            READ_RESOURCE_TOOL_NAME.to_string(),
                            indoc! {r#"
            Read a resource from an extension.

            Resources allow extensions to share data that provide context to LLMs, such as
            files, database schemas, or application-specific information. This tool searches for the
            resource URI in the provided extension, and reads in the resource content. If no extension
            is provided, the tool will search all extensions for the resource.
        "#}.to_string(),
                            Arc::new(
                                serde_json::to_value(schema_for!(ReadResourceParams))
                                    .expect("Failed to serialize schema")
                                    .as_object()
                                    .expect("Schema must be an object")
                                    .clone()
                            ),
                        ).annotate(ToolAnnotations {
                            title: Some("Read a resource".to_string()),
                            read_only_hint: Some(true),
                            destructive_hint: Some(false),
                            idempotent_hint: Some(false),
                            open_world_hint: Some(false),
                        }),
                    ]);
                }
            }
        }

        tools
    }
}

#[async_trait]
impl McpClientTrait for ExtensionManagerClient {
    async fn list_resources(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListResourcesResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn read_resource(
        &self,
        _uri: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<ReadResourceResult, Error> {
        // Extension manager doesn't expose resources directly
        Err(Error::TransportClosed)
    }

    async fn list_tools(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: self.get_tools().await,
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        name: &str,
        arguments: Option<JsonObject>,
        meta: McpMeta,
        _cancellation_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        let result = match name {
            // Issue #56 Gate E, finding 13: the CATALOGUE is a discovery
            // surface, so it carries the admitted capability exactly as the
            // three below do. It reaches no server, which is why Task 15 left it
            // out of Gate C's siblings — but naming a private connector and its
            // marketplace blurb to a public model is the disclosure Gate E
            // exists to prevent, arriving through a different door.
            SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME => {
                self.handle_search_available_extensions(meta.capability)
                    .await
            }
            // ⚠ The retired name still dispatches. It is no longer advertised —
            // browsing is this tool with no `query` — but a model that read the
            // old name in an earlier transcript, or a stored `always allow`
            // grant keyed on it, would otherwise meet an unknown-tool error it
            // cannot act on.
            BROWSE_MARKETPLACE_EXTENSIONS_TOOL_NAME | SEARCH_MARKETPLACE_EXTENSIONS_TOOL_NAME => {
                self.handle_search_marketplace_extensions(arguments, meta.capability)
                    .await
            }
            // Issue #56 Gate F1: enabling an extension SPAWNS its server, so it
            // carries the admitted capability for the same reason the two reads
            // below do.
            MANAGE_EXTENSIONS_TOOL_NAME => {
                self.handle_manage_extensions(
                    arguments,
                    meta.session_id.clone(),
                    meta.capability,
                    _cancellation_token.clone(),
                )
                .await
            }
            // Issue #117. Carries the session id because the credential card is
            // published to that session's queue — a card with no session is a
            // dialog nobody's chat renders.
            INSTALL_EXTENSION_TOOL_NAME => {
                self.handle_install_extension(
                    arguments,
                    meta.session_id.clone(),
                    meta.capability,
                    _cancellation_token.clone(),
                )
                .await
            }
            DELETE_EXTENSION_PACKAGE_TOOL_NAME => {
                self.handle_delete_extension_package(
                    arguments,
                    meta.session_id.clone(),
                    meta.capability,
                    _cancellation_token.clone(),
                )
                .await
            }
            // #164. Carries the admitted capability for the same reason its
            // marketplace sibling does: the removal runs inside the driven
            // future, and a fresh sample there would let a Public-admitted call
            // uninstall a private connector after the user switched models
            // mid-turn.
            REMOVE_EXTENSION_TOOL_NAME => {
                self.handle_remove_extension(
                    arguments,
                    meta.session_id.clone(),
                    meta.capability,
                    _cancellation_token.clone(),
                )
                .await
            }
            // Issue #56: these two reach an MCP server, so they carry the
            // capability this call was ADMITTED on into Gate C's sibling guard
            // rather than letting the manager sample a newer one.
            LIST_RESOURCES_TOOL_NAME => {
                self.handle_list_resources(arguments, meta.capability).await
            }
            READ_RESOURCE_TOOL_NAME => self.handle_read_resource(arguments, meta.capability).await,
            _ => Err(ExtensionManagerToolError::UnknownTool {
                tool_name: name.to_string(),
            }),
        };

        match result {
            Ok(content) => Ok(CallToolResult::success(content)),
            Err(error) => {
                // Log the error for debugging
                error!("Extension manager tool '{}' failed: {}", name, error);

                // Return proper error result with is_error flag set
                Ok(CallToolResult {
                    content: vec![Content::text(error.to_string())],
                    is_error: Some(true), // ✅ Properly mark as error
                    structured_content: None,
                    meta: None,
                })
            }
        }
    }

    async fn list_prompts(
        &self,
        _next_cursor: Option<String>,
        _cancellation_token: CancellationToken,
    ) -> Result<ListPromptsResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn get_prompt(
        &self,
        _name: &str,
        _arguments: Value,
        _cancellation_token: CancellationToken,
    ) -> Result<GetPromptResult, Error> {
        Err(Error::TransportClosed)
    }

    async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
        mpsc::channel(1).1
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        Some(&self.info)
    }
}

impl ExtensionManagerClient {
    /// The advertised roster, as a pure function of whether a person can be
    /// asked. Sampled once by `get_tools` and threaded, in the spirit of
    /// `CallCapability`: two reads of a process-global could disagree, and a
    /// roster that half-believes a person is reachable is exactly the state
    /// this gate exists to prevent.
    #[allow(clippy::too_many_lines)]
    fn tools_for(can_ask_a_person: bool) -> Vec<Tool> {
        let mut tools = vec![
            Tool::new(
                SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME.to_string(),
                "List installed third-party extensions visible to this model, their exact names, and attachment state. Use the returned name with manage_extensions. For extensions absent from this inventory, search_marketplace_extensions finds packages available to install.".to_string(),
                Arc::new(
                    serde_json::json!({
                        "type": "object",
                        "required": [],
                        "properties": {}
                    })
                    .as_object()
                    .expect("Schema must be an object")
                    .clone()
                ),
            ).annotate(ToolAnnotations {
                title: Some("Discover extensions".to_string()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
                open_world_hint: Some(false),
            }),
            Tool::new(
                MANAGE_EXTENSIONS_TOOL_NAME.to_string(),
                "Enable or disable an installed third-party extension in this chat — `action` is exactly `enable` or `disable`.
            Enable is what \"attach\" means here and disable is \"detach\"; those two words are not accepted values.
            Use the exact installed name from search_available_extensions, not a marketplace title or registry id.
            Changes apply immediately in the current turn. The result lists exact availableTools
            after attach or removedTools after detach; use or stop using those names accordingly.
            ".to_string(),
                Arc::new(
                    serde_json::to_value(schema_for!(ManageExtensionsParams))
                        .expect("Failed to serialize schema")
                        .as_object()
                        .expect("Schema must be an object")
                        .clone()
                ),
            ).annotate(ToolAnnotations {
                title: Some("Enable or disable an extension".to_string()),
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
                open_world_hint: Some(false),
            }),
        ];

        // Browse/search are read-only and stay. Install and delete each park on a
        // proof-backed approval, so on a daemon that cannot obtain one they are
        // withheld rather than advertised-and-refused. See
        // `pending_user_action::user_proof_available`.
        //
        // ⚠ `manage_extensions` stays advertised even though its *enable* arm
        // can raise a proof-backed approval: that approval is a fallback for an
        // operator-pinned-off extension, and the tool's ordinary path needs no
        // approval at all. Withholding it would remove working functionality.
        tools.extend([
            Tool::new(
                SEARCH_MARKETPLACE_EXTENSIONS_TOOL_NAME.to_owned(),
                "Browse or search trusted BAAM marketplace extensions. Pass `query` to search ids, names, organizations, descriptions and tags — an entry matching any of its words is returned, best match first; omit it to list everything visible to this model. Private entries are hidden from public models. Results carry `registryId` (camelCase); pass that exact value as install_extension's `registry_id` (snake_case) — the two tools spell the same field differently."
                    .to_owned(),
                Arc::new(
                    serde_json::to_value(schema_for!(SearchMarketplaceExtensionsParams))
                        .expect("Failed to serialize schema")
                        .as_object()
                        .expect("Schema must be an object")
                        .clone(),
                ),
            )
            .annotate(ToolAnnotations {
                title: Some("Browse or search BAAM marketplace".to_owned()),
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(true),
            }),
        ]);

        if can_ask_a_person {
            tools.extend([
            Tool::new(
                INSTALL_EXTENSION_TOOL_NAME.to_owned(),
                "Install a BAAM extension by its exact trusted registry id. Biorouter resolves the download URL itself and requires the user's proof-backed approval. A result attached to this chat lists exact availableTools that are callable immediately; an installed-only result must be attached before use."
                    .to_owned(),
                Arc::new(
                    serde_json::to_value(schema_for!(InstallExtensionParams))
                        .expect("Failed to serialize schema")
                        .as_object()
                        .expect("Schema must be an object")
                        .clone(),
                ),
            )
            .annotate(ToolAnnotations {
                title: Some("Install a BAAM extension".to_owned()),
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                idempotent_hint: Some(false),
                open_world_hint: Some(true),
            }),
            Tool::new(
                DELETE_EXTENSION_PACKAGE_TOOL_NAME.to_owned(),
                "Permanently delete one or up to 50 validated marketplace-installed .brxt packages by exact registry id. The whole batch is preflighted before one proof-backed approval, every result is reported, and credentials are preserved."
                    .to_owned(),
                Arc::new(
                    serde_json::to_value(schema_for!(DeleteExtensionPackageParams))
                        .expect("Failed to serialize schema")
                        .as_object()
                        .expect("Schema must be an object")
                        .clone(),
                ),
            )
            .annotate(ToolAnnotations {
                title: Some("Delete an installed BAAM package".to_owned()),
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                idempotent_hint: Some(false),
                open_world_hint: Some(false),
            }),
            Tool::new(
                REMOVE_EXTENSION_TOOL_NAME.to_owned(),
                "Permanently remove one or up to 50 installed extensions by exact installed name, whether or not they came from the BAAM marketplace. Use this for a sideloaded .brxt or a hand-configured MCP server; delete_extension_package only accepts a marketplace registry id. One proof-backed approval covers the batch, and it detaches the extension, removes its configuration entry, deletes its package directory and bundled skills, and drops its provenance record. Shared credentials are preserved."
                    .to_owned(),
                Arc::new(
                    serde_json::to_value(schema_for!(RemoveExtensionParams))
                        .expect("Failed to serialize schema")
                        .as_object()
                        .expect("Schema must be an object")
                        .clone(),
                ),
            )
            .annotate(ToolAnnotations {
                title: Some("Remove an installed extension".to_owned()),
                read_only_hint: Some(false),
                destructive_hint: Some(true),
                idempotent_hint: Some(false),
                open_world_hint: Some(false),
            }),
        ]);
        }

        tools
    }
}

#[cfg(test)]
mod argument_visibility_tests {
    //! The model can only pass the arguments something told it about.
    //!
    //! In code-execution mode NO JSON schema reaches the model — the prompt
    //! carries `Modules: <server names>` and nothing else, and the rendered
    //! signature keeps only the FIRST LINE of a tool's description and drops
    //! per-parameter docs entirely. So for these two tools the description's
    //! opening line is, in practice, the whole specification.
    //!
    //! Both of the failures these guard were observed in a real session:
    //!   * `action: "attach"` — our own description and server instructions both
    //!     said "Attach or detach", and neither named the accepted values.
    //!   * `registryId` instead of `registry_id` — the model did not invent it;
    //!     `search_marketplace_extensions` RETURNS `registryId` (camelCase) and
    //!     `install_extension` demands `registry_id` (snake_case), so it copied
    //!     the key out of our own output one call earlier.

    /// `manage_extensions` must name its accepted values on the first line, and
    /// must not teach the verb that is not accepted.
    #[test]
    fn manage_extensions_states_its_action_values_before_anything_else() {
        let source = include_str!("extension_manager_extension.rs");
        let description = source
            .split("MANAGE_EXTENSIONS_TOOL_NAME.to_string(),")
            .nth(1)
            .expect("the manage_extensions tool must be constructed here")
            .split(".to_string(),")
            .next()
            .expect("its description literal");
        let first_line = description
            .lines()
            .find(|line| !line.trim().is_empty())
            .expect("a non-empty first line");

        assert!(
            first_line.contains("`enable`") && first_line.contains("`disable`"),
            "the FIRST line must name both accepted values — it is the only line \
             that survives into the code-execution signature: {first_line}"
        );
        assert!(
            !first_line.starts_with("                \"Attach"),
            "the first line must not open by teaching \"Attach\", which is not an \
             accepted value: {first_line}"
        );
    }

    /// The camelCase/snake_case seam between the two tools has to be stated
    /// where the model reads it, because our own output is the source of the
    /// wrong key.
    #[test]
    fn the_marketplace_search_says_which_key_its_result_feeds() {
        let source = include_str!("extension_manager_extension.rs");
        let description = source
            .split("SEARCH_MARKETPLACE_EXTENSIONS_TOOL_NAME.to_owned(),")
            .nth(1)
            .expect("the search tool must be constructed here")
            .split(".to_owned(),")
            .next()
            .expect("its description literal");

        assert!(
            description.contains("registryId") && description.contains("registry_id"),
            "the description must name BOTH spellings — the result's `registryId` \
             and install_extension's `registry_id` — or the case flip is invisible \
             to the model that just read one and must now write the other: \
             {description}"
        );
    }

    /// The spelling we HAND the model round-trips back in.
    ///
    /// `install_extension` accepts `registry_id`; the search result whose value
    /// the caller is copying prints it as `registryId`, because these payloads
    /// are also read by the GUI and `json!` keys are camelCase there. So the
    /// obvious call — take the id out of the result, put it in the argument —
    /// was refused, and the caller spent a round-trip discovering a case
    /// convention it had no way to know. Same trap on `registry_ids` and
    /// `extension_name`.
    ///
    /// Two halves, and the second is what keeps this from being a rename:
    /// the alias is accepted, AND the schema still teaches only the snake_case
    /// name, so nothing learns the camelCase spelling from us.
    #[test]
    fn a_camel_case_argument_is_accepted_but_never_taught() {
        use super::{DeleteExtensionPackageParams, InstallExtensionParams, ManageExtensionsParams};
        // Accepted.
        let installed: InstallExtensionParams =
            serde_json::from_value(serde_json::json!({ "registryId": "spoke-agent" }))
                .expect("the spelling our own search result prints must be accepted");
        assert_eq!(installed.registry_id, "spoke-agent");
        let managed: ManageExtensionsParams = serde_json::from_value(
            serde_json::json!({ "action": "enable", "extensionName": "SPOKEAgent" }),
        )
        .expect("the spelling our own result payloads print must be accepted");
        assert_eq!(managed.extension_name, "SPOKEAgent");
        let deleted: DeleteExtensionPackageParams =
            serde_json::from_value(serde_json::json!({ "registryIds": ["a", "b"] }))
                .expect("the batch spelling must be accepted too");
        assert_eq!(deleted.registry_ids, vec!["a".to_owned(), "b".to_owned()]);

        // Never taught. schemars emits a field's DOC comment as the property
        // description, so the note explaining the alias must not be a `///` —
        // six copies of it would ride along in every tool schema.
        let schema = serde_json::to_value(schemars::schema_for!(InstallExtensionParams)).unwrap();
        let text = schema.to_string();
        assert!(
            text.contains("registry_id"),
            "the schema must still teach the snake_case name: {text}"
        );
        assert!(
            !text.contains("registryId"),
            "an alias must not reach the schema — that would teach the spelling              it exists to forgive: {text}"
        );
        assert!(
            !text.contains("round-trip"),
            "the alias rationale is for a code reader, not for the model's tool              schema; make it a `//` comment: {text}"
        );
    }

    /// …and the server instructions must not contradict the tool description.
    /// They are a second copy of the same claim in the same system prompt.
    #[test]
    fn the_server_instructions_do_not_teach_the_rejected_verb() {
        let source = include_str!("extension_manager_extension.rs");
        let line = source
            .lines()
            .find(|line| line.contains("- manage_extensions:"))
            .expect("the instruction bullet must exist");
        assert!(
            line.contains("enable") && line.contains("disable"),
            "the instruction bullet must name the accepted values: {line}"
        );
        assert!(
            !line.contains("Attach or detach"),
            "the bullet still teaches the rejected verb, which is where the model \
             read it the first time: {line}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::CallCapability;
    // The production half of this file no longer names a tier: every arm of the
    // enable decision moved into `refusal::extension_enable_refusal`, so the
    // import lives with the tests that still state one.
    use crate::privacy::ProviderTier;

    /// The capability a public caller carries with the feature ON — the pair
    /// every pre-#56 test in this module was implicitly written against.
    fn public_enforcing() -> CallCapability {
        CallCapability::for_test(ProviderTier::Public, true)
    }

    fn entry(enabled: bool) -> ExtensionEntry {
        ExtensionEntry {
            enabled,
            config: ExtensionConfig::stdio(
                "publicfixture",
                "fixture-command",
                "shell and file tools",
                30_u64,
            ),
        }
    }

    fn marketplace_catalog() -> crate::marketplace::MarketplaceCatalog {
        crate::marketplace::MarketplaceCatalog::from_bytes(
            &serde_json::to_vec(&serde_json::json!({
                "version": 2,
                "source": "https://biorouter.ucsf.edu/baam",
                "institutions": { "ucsf": "UCSF" },
                "extensions": [
                    {
                        "id": "manager-public-fixture",
                        "name": "Manager Public Fixture",
                        "organization": "Example",
                        "version": "v1.0.0",
                        "description": "Public fixture",
                        "tags": ["fixture"],
                        "github": "https://github.com/example/manager-public-fixture",
                        "download": "https://github.com/example/manager-public-fixture/releases/download/v1.0.0/manager-public-fixture.brxt",
                        "filename": "manager-public-fixture.brxt",
                        "license": "Apache-2.0",
                        "privacy": "public"
                    },
                    {
                        "id": "manager-private-fixture",
                        "name": "Manager Private Fixture",
                        "organization": "Example",
                        "version": "v1.0.0",
                        "description": "Private fixture",
                        "tags": ["fixture"],
                        "github": "https://github.com/example/manager-private-fixture",
                        "download": "https://github.com/example/manager-private-fixture/releases/download/v1.0.0/manager-private-fixture.brxt",
                        "filename": "manager-private-fixture.brxt",
                        "license": "Apache-2.0",
                        "privacy": "private",
                        "extension_name": "manager-private-fixture",
                        "affiliation": ["ucsf"]
                    }
                ],
                "skills": []
            }))
            .unwrap(),
        )
        .unwrap()
    }

    fn marketplace_descriptor() -> crate::marketplace::MarketplaceExtensionDescriptor {
        resolve_marketplace_extension(
            &marketplace_catalog(),
            "manager-public-fixture",
            ProviderTier::Private,
        )
        .unwrap()
    }

    /// The extension half of finding F5, at the tool's output: a phrase is
    /// matched by any of its words and ranked, and a query matching nothing
    /// explains itself instead of returning an empty list.
    ///
    /// ⚠ The explanation counts what THIS caller may see. A public caller
    /// whose query matches only the private row it is not shown must read
    /// exactly what a query matching nothing at all reads — otherwise the
    /// guidance, not the hit list, becomes the private catalog's oracle.
    #[test]
    fn marketplace_search_ranks_a_phrase_and_explains_an_empty_result_from_visible_rows() {
        let loaded = crate::marketplace::MarketplaceCatalogLoad {
            catalog: marketplace_catalog(),
            source: crate::marketplace::MarketplaceCatalogSource::Embedded,
            cache_warning: None,
        };
        let ids = |body: &Value| -> Vec<String> {
            body["extensions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["registryId"].as_str().unwrap().to_owned())
                .collect()
        };

        let as_public = marketplace_extensions_json(
            &loaded,
            Some("public fixture manager"),
            ProviderTier::Public,
        );
        assert_eq!(
            as_public["terms"],
            serde_json::json!(["public", "fixture", "manager"])
        );
        assert_eq!(ids(&as_public), ["manager-public-fixture"]);
        assert_eq!(
            as_public["extensions"][0]["matchedTerms"],
            serde_json::json!(["public", "fixture", "manager"])
        );
        let as_private = marketplace_extensions_json(
            &loaded,
            Some("public fixture manager"),
            ProviderTier::Private,
        );
        assert_eq!(
            ids(&as_private),
            ["manager-public-fixture", "manager-private-fixture"],
            "the row matching every term ranks first"
        );

        let guidance = |query: &str| -> String {
            let body = marketplace_extensions_json(&loaded, Some(query), ProviderTier::Public);
            assert!(ids(&body).is_empty(), "{body}");
            body["guidance"]
                .as_str()
                .expect("an empty result explains itself")
                .replace(&format!("`{query}`"), "`QUERY`")
        };
        let hidden_match = guidance("private");
        assert!(
            hidden_match.contains("1 extension is available to this model"),
            "{hidden_match}"
        );
        assert_eq!(
            hidden_match,
            guidance("zzqx"),
            "a public caller's miss on a hidden private row reads differently from a miss on \
             nothing"
        );
        assert!(
            marketplace_extensions_json(&loaded, None, ProviderTier::Public)
                .get("guidance")
                .is_none(),
            "browsing needs no explanation"
        );
    }

    #[test]
    fn install_schema_accepts_only_a_registry_id_and_enable_flag() {
        let schema = serde_json::to_value(schema_for!(InstallExtensionParams)).unwrap();
        let properties = schema
            .pointer("/properties")
            .and_then(Value::as_object)
            .expect("install schema properties");
        assert!(properties.contains_key("registry_id"));
        assert!(properties.contains_key("enable"));
        assert!(!properties.contains_key("url"));
        assert!(
            serde_json::from_value::<InstallExtensionParams>(serde_json::json!({
                "registry_id": "manager-public-fixture",
                "url": "https://attacker.invalid/payload.brxt"
            }))
            .is_err()
        );
    }

    #[test]
    fn public_to_private_install_preflight_is_absolute_before_mutation() {
        let catalog = marketplace_catalog();
        let public_with_master_switch_off = CallCapability::for_test(ProviderTier::Public, false);
        let denied = resolve_marketplace_extension(
            &catalog,
            "manager-private-fixture",
            public_with_master_switch_off.tier(),
        )
        .expect_err("the master switch must not authorize marketplace installation");
        assert!(denied.to_string().contains("unavailable"));

        assert!(resolve_marketplace_extension(
            &catalog,
            "manager-private-fixture",
            ProviderTier::Private,
        )
        .is_ok());
        assert!(resolve_marketplace_extension(
            &catalog,
            "manager-public-fixture",
            ProviderTier::Public,
        )
        .is_ok());
    }

    #[test]
    fn marketplace_mutations_construct_proof_required_approval_cards() {
        let mutation = MarketplaceMutation::Install;
        let request = marketplace_approval_request(mutation, &marketplace_descriptor(), None);
        let crate::pending_user_action::UserActionRequest::ToolApproval(request) = request else {
            panic!("marketplace mutation did not create an approval")
        };
        assert!(request.requires_user_proof);
        assert_eq!(request.tool_name, mutation.tool_name());
        assert_eq!(request.risk, Some(mutation.risk()));
        assert!(request.arguments.contains_key("registryId"));
        assert!(request.arguments.contains_key("downloadUrl"));
    }

    #[test]
    fn operator_disabled_enable_constructs_a_proof_bound_session_only_card() {
        let request = extension_enable_approval_request("publicfixture", &entry(false));
        let crate::pending_user_action::UserActionRequest::ToolApproval(request) = request else {
            panic!("extension enable did not create an approval")
        };
        assert!(request.requires_user_proof);
        assert_eq!(request.tool_name, MANAGE_EXTENSIONS_TOOL_NAME_COMPLETE);
        assert_eq!(
            request.arguments.get("scope").and_then(Value::as_str),
            Some("currentChatOnly")
        );
        assert!(!request.arguments.contains_key("persistedEntry"));
        let encoded = serde_json::to_string(&request.arguments).unwrap();
        for secret_config_field in ["envs", "env_keys", "args", "cmd"] {
            assert!(!encoded.contains(secret_config_field), "{encoded}");
        }
    }

    #[tokio::test]
    async fn marketplace_approval_cancellation_and_timeout_stop_the_mutation() {
        let descriptor = marketplace_descriptor();

        let cancelled_actions = Arc::new(crate::pending_user_action::PendingUserActions::default());
        let cancel = CancellationToken::new();
        cancel.cancel();
        let cancelled = await_extension_change_approval(
            &cancelled_actions,
            "manager-cancel-fixture",
            marketplace_approval_request(MarketplaceMutation::Install, &descriptor, None),
            Duration::from_secs(5),
            Some(&cancel),
        )
        .await
        .unwrap_err();
        assert!(cancelled.to_string().contains("cancelled"));

        let timed_out_actions = Arc::new(crate::pending_user_action::PendingUserActions::default());
        let timed_out = await_extension_change_approval(
            &timed_out_actions,
            "manager-timeout-fixture",
            marketplace_approval_request(MarketplaceMutation::Install, &descriptor, None),
            Duration::ZERO,
            None,
        )
        .await
        .unwrap_err();
        assert!(timed_out.to_string().contains("expired"));
    }

    #[test]
    fn descriptor_changes_invalidate_an_existing_approval() {
        let approved = marketplace_descriptor();
        let mut changed = approved.clone();
        changed.version = "v2.0.0".to_owned();
        changed.download_url =
            url::Url::parse("https://github.com/example/v2/manager-public-fixture.brxt").unwrap();
        assert!(ensure_descriptor_unchanged(&approved, &approved).is_ok());
        assert!(ensure_descriptor_unchanged(&approved, &changed).is_err());
    }

    #[tokio::test]
    async fn marketplace_install_and_delete_are_advertised_tools() {
        let (_dir, _manager, client, _session_id) = a_live_tool_client().await;
        let names = client
            .get_tools()
            .await
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        assert!(
            !names
                .iter()
                .any(|name| name == BROWSE_MARKETPLACE_EXTENSIONS_TOOL_NAME),
            "browsing is `{SEARCH_MARKETPLACE_EXTENSIONS_TOOL_NAME}` with no query; \
             re-advertising it puts a second tool on the surface for one job"
        );
        for expected in [
            SEARCH_MARKETPLACE_EXTENSIONS_TOOL_NAME,
            INSTALL_EXTENSION_TOOL_NAME,
            DELETE_EXTENSION_PACKAGE_TOOL_NAME,
            REMOVE_EXTENSION_TOOL_NAME,
        ] {
            assert!(names.iter().any(|name| name == expected), "{expected}");
        }
        for resource_tool in [LIST_RESOURCES_TOOL_NAME, READ_RESOURCE_TOOL_NAME] {
            assert!(
                !names.iter().any(|name| name == resource_tool),
                "{resource_tool} must not be advertised without resource support"
            );
        }
        let instructions = client
            .get_info()
            .and_then(|info| info.instructions.as_deref())
            .expect("Extension Manager instructions");
        assert!(instructions.contains("only when they appear in the current tool catalog"));
        assert!(instructions.contains("omitted when no loaded extension supports resources"));
        assert!(instructions.contains("immediately callable availableTools"));
        assert!(instructions.contains("state installed means attach it before use"));

        // ⚠ A retired name has TWO spellings in prose, and a single-token
        // check sees only one. `browse/search` names the retired verb while
        // containing no retired token, so deleting the roster line and
        // leaving that sentence passes a bare
        // `!contains(BROWSE_MARKETPLACE_EXTENSIONS_TOOL_NAME)` — and the
        // model still reads an instruction to browse.
        for retired in [BROWSE_MARKETPLACE_EXTENSIONS_TOOL_NAME, "browse/search"] {
            assert!(
                !instructions.contains(retired),
                "the instructions still name a retired tool ('{retired}'), which teaches \
                 the model to call something it is never offered"
            );
        }
        assert!(instructions.contains("omit the query to browse"));
        // The capability's own one-line summary must name what it can now do,
        // including the irreversible half.
        assert!(instructions.contains("permanently delete"));
        // ⚠ #164. The instructions are where the model learns WHICH door a
        // non-marketplace extension goes through. Without this sentence it
        // reaches for `delete_extension_package`, is told the registry id does
        // not resolve, and falls back to editing `config.yaml` by hand — the
        // exact behaviour `remove_extension` exists to replace.
        assert!(instructions.contains("remove_extension"));
        assert!(instructions.contains("did not come from the marketplace"));
        assert!(instructions.contains("edit config.yaml"));

        // ⚠ What the removal actually reaches is THIS chat's live manager plus
        // the SAVED rosters of the others (`prune_extension_from_sessions`); a
        // chat already running keeps the extension loaded in memory until it is
        // reopened. The instructions claimed "every chat that had it", and an
        // overclaim here is one the model repeats to the user as a fact.
        assert!(
            !instructions.contains("every chat that had it"),
            "the instructions promise a detach the removal does not perform"
        );
        assert!(instructions.contains("detaches the extension from this chat"));
        assert!(instructions.contains("saved rosters of other chats"));
    }

    fn package_entry(name: &str, install_dir: &std::path::Path) -> ExtensionEntry {
        ExtensionEntry {
            enabled: true,
            config: ExtensionConfig::Stdio {
                name: name.to_owned(),
                description: "marketplace package fixture".to_owned(),
                cmd: "uv".to_owned(),
                args: vec![
                    "run".to_owned(),
                    "--directory".to_owned(),
                    install_dir.display().to_string(),
                    "server.py".to_owned(),
                ],
                envs: crate::agents::extension::Envs::default(),
                env_keys: vec!["SHARED_CREDENTIAL".to_owned()],
                timeout: Some(300),
                bundled: None,
                available_tools: Vec::new(),
            },
        }
    }

    fn package_provenance(
        registry_id: &str,
        config_key: &str,
        install_dir: &std::path::Path,
    ) -> crate::privacy::provenance::MarketplaceInstallProvenance {
        crate::privacy::provenance::MarketplaceInstallProvenance {
            config_key: config_key.to_owned(),
            install_id: Some(format!("test-install-{registry_id}")),
            registry_id: registry_id.to_owned(),
            install_dir: install_dir.display().to_string(),
            source_url: format!(
                "https://github.com/example/{registry_id}/releases/download/v1.0.0/{registry_id}.brxt"
            ),
        }
    }

    struct DeletionFixture {
        registry_id: String,
        config_key: String,
        extension_name: String,
        install_dir: PathBuf,
        entry: ExtensionEntry,
    }

    impl DeletionFixture {
        fn assert_provenance_present(&self) {
            assert!(
                crate::privacy::provenance::marketplace_installs_for_registry_id(&self.registry_id)
                    .iter()
                    .any(|provenance| provenance.config_key == self.config_key),
                "the marketplace provenance for {} must still be current",
                self.registry_id
            );
        }

        fn assert_provenance_removed(&self) {
            assert!(
                crate::privacy::provenance::marketplace_installs_for_registry_id(&self.registry_id)
                    .iter()
                    .all(|provenance| provenance.config_key != self.config_key),
                "the marketplace provenance for {} survived deletion",
                self.registry_id
            );
        }

        fn remove_persisted_artifacts(&self) {
            crate::config::extensions::remove_extension(&self.config_key);
            if let Some(provenance) =
                crate::privacy::provenance::marketplace_installs_for_registry_id(&self.registry_id)
                    .into_iter()
                    .find(|provenance| provenance.config_key == self.config_key)
            {
                let _ =
                    crate::privacy::provenance::remove_marketplace_install_provenance(&provenance);
            }
            if self.install_dir.exists() {
                let _ = std::fs::remove_dir_all(&self.install_dir);
            }
        }
    }

    impl Drop for DeletionFixture {
        fn drop(&mut self) {
            self.remove_persisted_artifacts();
        }
    }

    /// Hold `BIOROUTER_PATH_ROOT` still for the duration of a test, so the
    /// fixture below and the uninstall path under test resolve
    /// `extensions_root()` to the same directory every time either asks.
    ///
    /// ⚠ It pins the **sandbox** root, not the variable's current value. The
    /// difference is the whole bug: reading the variable to decide what to pin
    /// happens before the lock is taken, so the value read is whichever of this
    /// binary's ~30 relocating tests is holding `env_lock` at that instant —
    /// and by the time the pin acquires the lock, that test has finished and
    /// deleted its `TempDir`. See `crate::test_sandbox::pin_sandbox_path_root`.
    fn pinned_path_root() -> env_lock::EnvGuard<'static> {
        crate::test_sandbox::pin_sandbox_path_root()
    }

    async fn install_deletion_fixture(registry_id: &str, label: &str) -> DeletionFixture {
        let descriptor = trusted_marketplace_extension(registry_id, ProviderTier::Public)
            .await
            .unwrap_or_else(|error| panic!("the shipped {registry_id} descriptor loads: {error}"));
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let extension_name = format!("ManagerDelete{label}{suffix}");
        let config_key = crate::config::extensions::name_to_key(&extension_name);
        let install_dir = crate::extension_install::brxt::extensions_root().join(&extension_name);
        std::fs::create_dir_all(&install_dir).unwrap();
        let entry = package_entry(&extension_name, &install_dir);
        crate::config::extensions::set_extension(entry.clone());
        crate::privacy::provenance::record(
            &extension_name,
            crate::privacy::provenance::ExtensionProvenance {
                install_id: Some(format!("delete-fixture-{suffix}")),
                registry_id: registry_id.to_owned(),
                install_dir: Some(install_dir.display().to_string()),
                source_url: Some(descriptor.download_url.as_str().to_owned()),
                bundle_sha256: None,
                recorded_at: None,
            },
        )
        .unwrap();
        let fixture = DeletionFixture {
            registry_id: registry_id.to_owned(),
            config_key,
            extension_name,
            install_dir,
            entry,
        };
        fixture.assert_provenance_present();
        fixture
    }

    fn tool_result_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|content| content.as_text().map(|text| text.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// ⚠ Parameterized by tool name rather than duplicated: both uninstall
    /// doors publish the same proof-backed card, and a second copy of this
    /// fixture is how one door would quietly stop being checked for the proof
    /// requirement the assertion at the bottom pins.
    async fn wait_for_delete_card(session_id: &str, tool_name: &str) -> String {
        let expected_tool = tool_name;
        tokio::time::timeout(
            Duration::from_secs(30),
            crate::action_required_manager::ActionRequiredManager::global()
                .request_arrived(session_id),
        )
        .await
        .expect("the deletion call must publish its approval card");
        let messages = crate::action_required_manager::ActionRequiredManager::global()
            .drain_requests(session_id);
        let approval_id = messages
            .iter()
            .flat_map(|message| &message.content)
            .find_map(|content| {
                let crate::conversation::message::MessageContent::ActionRequired(action) = content
                else {
                    return None;
                };
                let crate::conversation::message::ActionRequiredData::ToolConfirmation {
                    id,
                    tool_name: card_tool,
                    ..
                } = &action.data
                else {
                    return None;
                };
                (card_tool == expected_tool).then(|| id.clone())
            })
            .expect("the deletion call must publish a tool-confirmation card");
        assert!(crate::pending_user_action::PendingUserActions::global()
            .requires_user_proof_in_session(session_id, &approval_id));
        approval_id
    }

    async fn approve_delete_card(session_id: &str, tool_name: &str) {
        let approval_id = wait_for_delete_card(session_id, tool_name).await;
        assert_eq!(
            crate::pending_user_action::PendingUserActions::global().resolve_in_session(
                session_id,
                &approval_id,
                crate::pending_user_action::UserActionOutcome::Approved {
                    permission: crate::permission::Permission::AllowOnce,
                },
                // These stand in for the desktop dialog answering a
                // proof-backed card, which is what the test is a fixture for —
                // the gate itself is exercised in `decision_authority_tests`.
                crate::pending_user_action::DecisionAuthority::for_test_proven(),
            ),
            crate::pending_user_action::ResolveOutcome::Delivered
        );
    }

    async fn run_approved_delete(
        client: Arc<ExtensionManagerClient>,
        session_id: String,
        arguments: Value,
    ) -> CallToolResult {
        run_approved_uninstall(
            client,
            session_id,
            DELETE_EXTENSION_PACKAGE_TOOL_NAME,
            arguments,
        )
        .await
    }

    async fn run_approved_uninstall(
        client: Arc<ExtensionManagerClient>,
        session_id: String,
        tool_name: &'static str,
        arguments: Value,
    ) -> CallToolResult {
        let running = tokio::spawn({
            let session_id = session_id.clone();
            async move {
                client
                    .call_tool(
                        tool_name,
                        Some(arguments.as_object().unwrap().clone()),
                        McpMeta::new(
                            session_id,
                            CallCapability::for_test(ProviderTier::Public, true),
                        ),
                        CancellationToken::new(),
                    )
                    .await
            }
        });
        approve_delete_card(&session_id, tool_name).await;
        tokio::time::timeout(Duration::from_secs(30), running)
            .await
            .expect("the approved deletion must finish promptly")
            .expect("the deletion task must not panic")
            .expect("the extension manager must return a tool result")
    }

    #[tokio::test]
    async fn approved_single_package_deletion_removes_package_config_provenance_and_session_state()
    {
        let _path_root = pinned_path_root();
        let fixture = install_deletion_fixture("playwrightagent", "Single").await;
        let (_manager_root, manager, client, _session_id) = a_live_tool_client().await;
        assert!(!manager.is_extension_enabled(&fixture.config_key).await);
        let session_id = format!("delete-single-{}", uuid::Uuid::new_v4());

        let result = run_approved_delete(
            Arc::new(client),
            session_id,
            serde_json::json!({ "registry_id": fixture.registry_id.clone() }),
        )
        .await;

        assert_ne!(result.is_error, Some(true), "{}", tool_result_text(&result));
        let report: Value = serde_json::from_str(&tool_result_text(&result)).unwrap();
        assert_eq!(report["state"], "deleted");
        assert_eq!(report["results"][0]["status"], "deleted");
        assert!(!fixture.install_dir.exists());
        assert!(get_extension_entry_by_name(&fixture.extension_name).is_none());
        fixture.assert_provenance_removed();
        assert!(!manager.is_extension_enabled(&fixture.config_key).await);
    }

    #[tokio::test]
    async fn approved_batch_deletion_removes_every_package_config_provenance_and_session_state() {
        let _path_root = pinned_path_root();
        let first = install_deletion_fixture("codegraphagent", "BatchOne").await;
        let second = install_deletion_fixture("bioroffice", "BatchTwo").await;
        let (_manager_root, manager, client, _session_id) = a_live_tool_client().await;
        let session_id = format!("delete-batch-{}", uuid::Uuid::new_v4());

        let result = run_approved_delete(
            Arc::new(client),
            session_id,
            serde_json::json!({
                "registry_ids": [first.registry_id.clone(), second.registry_id.clone()]
            }),
        )
        .await;

        assert_ne!(result.is_error, Some(true), "{}", tool_result_text(&result));
        let report: Value = serde_json::from_str(&tool_result_text(&result)).unwrap();
        assert_eq!(report["state"], "deleted");
        assert_eq!(report["results"].as_array().map(Vec::len), Some(2));
        for fixture in [&first, &second] {
            assert!(!fixture.install_dir.exists());
            assert!(get_extension_entry_by_name(&fixture.extension_name).is_none());
            fixture.assert_provenance_removed();
            assert!(!manager.is_extension_enabled(&fixture.config_key).await);
        }
    }

    /// A `.brxt` somebody sideloaded, or an MCP server they added by hand:
    /// installed, on disk, and invisible to the marketplace. No provenance
    /// record is written, which is exactly the state
    /// `delete_extension_package` cannot act on.
    struct RemovalFixture {
        config_key: String,
        extension_name: String,
        install_dir: PathBuf,
        skill_slug: String,
        /// The `config.yaml` map key the entry really sits under, once
        /// [`RemovalFixture::rekey_by_hand`] has moved it off the derived one.
        hand_written_key: Option<String>,
    }

    impl Drop for RemovalFixture {
        fn drop(&mut self) {
            crate::config::extensions::remove_extension(&self.config_key);
            if let Some(hand_written) = &self.hand_written_key {
                crate::config::extensions::remove_extension(hand_written);
            }
            if self.install_dir.exists() {
                let _ = std::fs::remove_dir_all(&self.install_dir);
            }
        }
    }

    impl RemovalFixture {
        /// Move the entry to a map key an operator typed, which need not be
        /// derived from the entry's name at all — the state an MCP server added
        /// to `config.yaml` by hand is in, and the one every writer in this
        /// process is incapable of producing (they all key by `config.key()`).
        fn rekey_by_hand(&mut self) -> String {
            let hand_written = format!("hand-written-{}", uuid::Uuid::new_v4().simple());
            let entry =
                crate::config::extensions::get_extension_entry_by_name(&self.extension_name)
                    .expect("the fixture entry is configured");
            // The literal is `EXTENSIONS_CONFIG_KEY`, which is private to
            // `config::extensions`; nothing else in this process writes a map
            // key by hand, so there is no helper to borrow.
            crate::config::Config::global()
                .update_param::<indexmap::IndexMap<String, ExtensionEntry>, _, _>(
                    "extensions",
                    |extensions| {
                        extensions.shift_remove(&self.config_key);
                        extensions.insert(hand_written.clone(), entry);
                    },
                )
                .expect("the fixture config is writable");
            self.hand_written_key = Some(hand_written.clone());
            hand_written
        }
    }

    fn install_sideloaded_fixture(label: &str) -> RemovalFixture {
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let extension_name = format!("ManagerRemove{label}{suffix}");
        let config_key = crate::config::extensions::name_to_key(&extension_name);
        let install_dir = crate::extension_install::brxt::extensions_root().join(&extension_name);
        let skill_slug = format!("bundled-{label}").to_lowercase();
        std::fs::create_dir_all(install_dir.join("skills").join(&skill_slug)).unwrap();
        std::fs::write(
            install_dir
                .join("skills")
                .join(&skill_slug)
                .join("SKILL.md"),
            "---\nname: bundled-fixture\n---\n",
        )
        .unwrap();
        crate::config::extensions::set_extension(package_entry(&extension_name, &install_dir));
        assert!(
            crate::privacy::provenance::extension_provenance_for_key(&config_key).is_none(),
            "a sideloaded fixture must have no provenance, or it is not the case under test"
        );
        RemovalFixture {
            config_key,
            extension_name,
            install_dir,
            skill_slug,
            hand_written_key: None,
        }
    }

    /// ⚠ The leftover of a failed delete is still a SKILL ROOT — `roots()`
    /// takes every child of the extensions directory with a `skills/`
    /// subdirectory, dot-names included — so a quarantine that survives keeps
    /// serving skills the user was just told were gone.
    #[cfg(unix)]
    #[test]
    fn a_package_that_cannot_be_deleted_still_stops_contributing_skills() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let quarantine = root.path().join(".delete-fixture");
        let skills = quarantine.join("skills").join("bundled");
        std::fs::create_dir_all(&skills).unwrap();
        std::fs::write(skills.join("SKILL.md"), "---\nname: bundled\n---\n").unwrap();
        // A directory whose contents cannot be unlinked, so the whole-tree
        // delete fails the way a busy `.venv` or a root-owned build artefact
        // makes it fail in the field.
        let locked = quarantine.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("pinned"), "x").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();

        let (removed, problem) = delete_quarantined_package(&quarantine, "Fixture");
        // Running as root ignores the mode above, and a test that silently
        // asserts nothing is worse than one that says why.
        if removed == Some(true) {
            assert!(problem.is_none());
            return;
        }

        assert_eq!(removed, Some(false));
        let problem = problem.expect("a package that survived must be reported");
        assert!(problem.contains("could not be deleted"), "{problem}");
        assert!(
            !quarantine.join("skills").exists(),
            "the skills subtree must be gone even when the package could not be, or the \
             catalog keeps serving them under the quarantine's name: {problem}"
        );
        assert!(
            !problem.contains("bundled skills are still on disk"),
            "the fallback succeeded, so the report must not claim otherwise: {problem}"
        );

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// The gap this tool exists to close, end to end: an extension with no
    /// registry id, uninstalled in one call.
    #[tokio::test]
    async fn approved_removal_uninstalls_a_non_marketplace_extension_and_its_bundled_skills() {
        let _path_root = pinned_path_root();
        let fixture = install_sideloaded_fixture("Single");
        let (_manager_root, manager, client, _session_id) = a_live_tool_client().await;
        let session_id = format!("remove-single-{}", uuid::Uuid::new_v4());

        // The marketplace door cannot even NAME it — that is the gap, not a
        // shortcoming of this fixture.
        assert!(
            preflight_marketplace_deletions(
                std::slice::from_ref(&fixture.extension_name),
                ProviderTier::Private,
            )
            .await
            .is_err(),
            "an extension with no registry id must be unnameable by the marketplace door"
        );

        let result = run_approved_uninstall(
            Arc::new(client),
            session_id,
            REMOVE_EXTENSION_TOOL_NAME,
            serde_json::json!({ "extension_name": fixture.extension_name.clone() }),
        )
        .await;

        assert_ne!(result.is_error, Some(true), "{}", tool_result_text(&result));
        let report: Value = serde_json::from_str(&tool_result_text(&result)).unwrap();
        assert_eq!(report["state"], "removed");
        assert_eq!(report["results"][0]["status"], "removed");
        assert_eq!(report["results"][0]["configurationRemoved"], true);
        assert_eq!(report["results"][0]["installDirectoryRemoved"], true);
        // Nothing recorded where a sideloaded package came from, so there was
        // no record to drop — reported honestly rather than as a success.
        assert_eq!(report["results"][0]["provenanceRecordRemoved"], false);
        assert_eq!(
            report["removedSkills"],
            serde_json::json!([fixture.skill_slug])
        );
        assert_eq!(report["credentialsPreserved"], true);

        assert!(!fixture.install_dir.exists());
        assert!(get_extension_entry_by_name(&fixture.extension_name).is_none());
        assert!(!manager.is_extension_enabled(&fixture.config_key).await);
    }

    /// ⚠ **The headline case, and the one every fixture above routes around.**
    /// An extension is resolved here BY NAME, so the plan carries
    /// `name_to_key(<installed name>)` — but the `config.yaml` mapping keys an
    /// entry by whatever the operator typed, and "map keys are not names".
    /// `remove_extension_if_matches` looking that entry up by the derived key
    /// alone found nothing, answered "the extension configuration changed
    /// before deletion", rolled the whole uninstall back and left the server
    /// configured: the tool silently failed on exactly the extension it exists
    /// for, an MCP server added to `config.yaml` by hand.
    ///
    /// Every other removal fixture goes through `set_extension`, which keys by
    /// `config.key()` — so none of them can ever be in the state under test.
    #[tokio::test]
    async fn approved_removal_uninstalls_an_extension_whose_config_key_was_written_by_hand() {
        let _path_root = pinned_path_root();
        let mut fixture = install_sideloaded_fixture("HandKeyed");
        let hand_written = fixture.rekey_by_hand();
        assert_ne!(hand_written, fixture.config_key);
        assert!(
            get_extension_entry_by_name(&fixture.extension_name).is_some(),
            "the entry must still be configured under its hand-written key"
        );
        let (_manager_root, manager, client, _session_id) = a_live_tool_client().await;
        let session_id = format!("remove-hand-keyed-{}", uuid::Uuid::new_v4());

        let result = run_approved_uninstall(
            Arc::new(client),
            session_id,
            REMOVE_EXTENSION_TOOL_NAME,
            serde_json::json!({ "extension_name": fixture.extension_name.clone() }),
        )
        .await;

        assert_ne!(result.is_error, Some(true), "{}", tool_result_text(&result));
        let report: Value = serde_json::from_str(&tool_result_text(&result)).unwrap();
        assert_eq!(report["state"], "removed");
        assert_eq!(report["results"][0]["status"], "removed");
        assert_eq!(report["results"][0]["configurationRemoved"], true);
        assert_eq!(report["results"][0]["installDirectoryRemoved"], true);

        assert!(!fixture.install_dir.exists());
        assert!(
            get_extension_entry_by_name(&fixture.extension_name).is_none(),
            "the hand-keyed entry survived the removal that reported success"
        );
        assert!(!manager.is_extension_enabled(&fixture.config_key).await);
    }

    /// The stored roster holds whole `ExtensionConfig`s and
    /// `Agent::load_extensions_from_session` spawns each one on resume, so a
    /// session left naming a removed extension tries to launch a deleted server
    /// and reports it as a broken extension rather than a removed one.
    #[tokio::test]
    async fn a_removal_prunes_the_extension_from_every_stored_session_roster() {
        let _path_root = pinned_path_root();
        let fixture = install_sideloaded_fixture("Sessions");
        let (_manager_root, _manager, client, roster_session) = a_live_tool_client().await;
        let session_manager = client.context.session_manager.clone();

        use crate::session::extension_data::{EnabledExtensionsState, ExtensionState};
        let entry = get_extension_entry_by_name(&fixture.extension_name)
            .expect("the fixture entry is configured");
        let stored = EnabledExtensionsState::new(vec![entry.config.clone()])
            .to_value()
            .unwrap();
        session_manager
            .update_extension_state(
                &roster_session,
                EnabledExtensionsState::EXTENSION_NAME,
                EnabledExtensionsState::VERSION,
                |_| Ok(stored),
            )
            .await
            .expect("the fixture roster is stored")
            .expect("the fixture session exists");

        let result = run_approved_uninstall(
            Arc::new(client),
            format!("remove-sessions-{}", uuid::Uuid::new_v4()),
            REMOVE_EXTENSION_TOOL_NAME,
            serde_json::json!({ "extension_name": fixture.extension_name.clone() }),
        )
        .await;
        assert_ne!(result.is_error, Some(true), "{}", tool_result_text(&result));
        let report: Value = serde_json::from_str(&tool_result_text(&result)).unwrap();
        assert_eq!(report["state"], "removed");
        assert_eq!(report["sessionsUpdated"], 1);

        let remaining = session_manager
            .get_extension_state(
                &roster_session,
                EnabledExtensionsState::EXTENSION_NAME,
                EnabledExtensionsState::VERSION,
            )
            .await
            .expect("the roster is readable")
            .and_then(|value| serde_json::from_value::<EnabledExtensionsState>(value).ok())
            .expect("the roster is still a roster");
        assert!(
            remaining
                .extensions
                .iter()
                .all(|config| config.key() != fixture.config_key),
            "a removed extension must not survive in a stored session roster"
        );
    }

    #[tokio::test]
    async fn post_approval_config_revalidation_leaves_the_replacement_and_package_untouched() {
        let _path_root = pinned_path_root();
        let fixture = install_deletion_fixture("opennotebookagent", "Revalidate").await;
        let (_manager_root, manager, client, _session_id) = a_live_tool_client().await;
        let client = Arc::new(client);
        let session_id = format!("delete-revalidate-{}", uuid::Uuid::new_v4());
        let running = tokio::spawn({
            let client = Arc::clone(&client);
            let session_id = session_id.clone();
            let registry_id = fixture.registry_id.clone();
            async move {
                client
                    .call_tool(
                        DELETE_EXTENSION_PACKAGE_TOOL_NAME,
                        Some(
                            serde_json::json!({ "registry_id": registry_id })
                                .as_object()
                                .unwrap()
                                .clone(),
                        ),
                        McpMeta::new(
                            session_id,
                            CallCapability::for_test(ProviderTier::Public, true),
                        ),
                        CancellationToken::new(),
                    )
                    .await
            }
        });
        let approval_id =
            wait_for_delete_card(&session_id, DELETE_EXTENSION_PACKAGE_TOOL_NAME).await;
        let mut replacement = fixture.entry.clone();
        replacement.enabled = false;
        crate::config::extensions::set_extension(replacement.clone());
        assert_eq!(
            crate::pending_user_action::PendingUserActions::global().resolve_in_session(
                &session_id,
                &approval_id,
                crate::pending_user_action::UserActionOutcome::Approved {
                    permission: crate::permission::Permission::AllowOnce,
                },
                // These stand in for the desktop dialog answering a
                // proof-backed card, which is what the test is a fixture for —
                // the gate itself is exercised in `decision_authority_tests`.
                crate::pending_user_action::DecisionAuthority::for_test_proven(),
            ),
            crate::pending_user_action::ResolveOutcome::Delivered
        );

        let result = tokio::time::timeout(Duration::from_secs(30), running)
            .await
            .expect("the stale approved deletion must finish promptly")
            .expect("the deletion task must not panic")
            .expect("the extension manager must return a tool result");
        assert_eq!(result.is_error, Some(true), "{}", tool_result_text(&result));
        assert!(tool_result_text(&result).contains("changed after approval"));
        assert!(fixture.install_dir.is_dir());
        let current = get_extension_entry_by_name(&fixture.extension_name)
            .expect("the replacement config must remain registered");
        assert!(!current.enabled);
        assert_eq!(current.config, replacement.config);
        fixture.assert_provenance_present();
        assert!(!manager.is_extension_enabled(&fixture.config_key).await);
        fixture.remove_persisted_artifacts();
    }

    #[tokio::test]
    async fn package_deletion_accepts_only_one_direct_validated_marketplace_child() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extensions");
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let install_dir = root.join("ManagerPublicFixture");
        std::fs::create_dir(&install_dir).unwrap();
        let descriptor = marketplace_descriptor();
        let provenance = package_provenance(
            &descriptor.registry_id,
            "managerpublicfixture",
            &install_dir,
        );

        let validated = validated_marketplace_package_at(
            &descriptor,
            vec![provenance.clone()],
            root.clone(),
            vec![package_entry("ManagerPublicFixture", &install_dir)],
        )
        .unwrap();
        assert_eq!(validated.install_dir, install_dir);
        assert_eq!(validated.provenance, provenance);
        assert!(matches!(
            &validated.config,
            ExtensionConfig::Stdio { env_keys, .. }
                if env_keys.contains(&"SHARED_CREDENTIAL".to_owned())
        ));
        let plan = ValidatedMarketplaceDeletion {
            descriptor: descriptor.clone(),
            package: validated.clone(),
        };
        let approval = marketplace_batch_delete_approval_request(std::slice::from_ref(&plan));
        let crate::pending_user_action::UserActionRequest::ToolApproval(approval) = approval else {
            panic!("batch deletion did not create a tool approval")
        };
        assert!(approval.requires_user_proof);
        assert_eq!(
            approval.risk,
            Some(crate::permission::tool_risk::ToolRisk::High)
        );
        assert_eq!(
            approval
                .arguments
                .get("registryIds")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert!(approval.preview.is_some());

        let (_manager_root, manager, _client, _session_id) = a_live_tool_client().await;
        let cancel = CancellationToken::new();
        cancel.cancel();
        let cancelled = delete_staged_marketplace_package(
            &manager,
            &plan,
            false,
            &cancel,
            ProviderTier::Private,
        )
        .await
        .expect_err("cancellation before staging must stop package deletion");
        assert!(cancelled.to_string().contains("cancelled"));
        assert!(
            validated.install_dir.is_dir(),
            "a pre-staging cancellation must leave the approved package untouched"
        );

        let alias = validate_unique_deletion_targets(
            [
                (
                    validated.provenance.config_key.as_str(),
                    Some(validated.install_dir.as_path()),
                ),
                (
                    validated.provenance.config_key.as_str(),
                    Some(validated.install_dir.as_path()),
                ),
            ],
            DeletionIdentifier::MarketplaceRegistryId,
        )
        .expect_err("two registry ids may not alias one package");
        assert!(alias.to_string().contains("same installed package"));

        let ambiguous = validated_marketplace_package_at(
            &descriptor,
            vec![provenance.clone(), provenance],
            root,
            vec![package_entry("ManagerPublicFixture", &install_dir)],
        )
        .unwrap_err();
        assert!(ambiguous.to_string().contains("ambiguous"));
    }

    #[test]
    fn package_deletion_rejects_paths_outside_the_extensions_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("extensions");
        let outside = temp.path().join("other-package");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let outside = std::fs::canonicalize(outside).unwrap();
        let descriptor = marketplace_descriptor();
        let error = validated_marketplace_package_at(
            &descriptor,
            vec![package_provenance(
                &descriptor.registry_id,
                "other-package",
                &outside,
            )],
            root,
            vec![package_entry("other-package", &outside)],
        )
        .unwrap_err();
        assert!(error.to_string().contains("direct child"));
        assert!(outside.is_dir(), "validation must not mutate the target");
    }

    #[test]
    fn extension_package_batch_preflight_is_bounded_ordered_and_unique() {
        let ids = preflight_delete_registry_ids(DeleteExtensionPackageParams {
            registry_id: Some("first".to_owned()),
            registry_ids: vec!["second".to_owned()],
        })
        .expect("valid batch");
        assert_eq!(ids, vec!["first", "second"]);

        assert!(preflight_delete_registry_ids(DeleteExtensionPackageParams {
            registry_id: Some("same".to_owned()),
            registry_ids: vec!["same".to_owned()],
        })
        .unwrap_err()
        .to_string()
        .contains("duplicates"));
        assert!(preflight_delete_registry_ids(DeleteExtensionPackageParams {
            registry_id: None,
            registry_ids: Vec::new(),
        })
        .is_err());
        assert!(preflight_delete_registry_ids(DeleteExtensionPackageParams {
            registry_id: None,
            registry_ids: (0..51).map(|index| format!("pkg-{index}")).collect(),
        })
        .unwrap_err()
        .to_string()
        .contains("50"));

        let schema = serde_json::to_value(schema_for!(DeleteExtensionPackageParams)).unwrap();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("deletion schema properties");
        assert!(properties.contains_key("registry_id"));
        assert!(properties.contains_key("registry_ids"));
        assert_eq!(
            properties
                .get("registry_ids")
                .and_then(|value| value.get("maxItems"))
                .and_then(Value::as_u64),
            Some(50)
        );
        assert!(
            serde_json::from_value::<DeleteExtensionPackageParams>(serde_json::json!({
                "registry_id": "one",
                "unexpected": true
            }))
            .is_err()
        );
    }

    /// ⚠ **Uninstalling an extension must never touch a credential**, on
    /// EITHER door. An API key can be shared between extensions — one UCSF
    /// credential unlocks more than one connector — so revoking it while
    /// removing one of them breaks an extension the user did not ask about,
    /// and the value is not recoverable from anywhere.
    ///
    /// The four regions are named separately rather than taken as one span,
    /// because a span defined by its two ends silently swallows whatever is
    /// inserted between them: `handle_remove_extension` was written directly
    /// underneath `handle_delete_extension_package`, and a guard that still ran
    /// from there to `manage_extensions_impl` would have "covered" the new
    /// handler by accident — and stopped covering it the moment either moved.
    #[test]
    fn extension_uninstall_paths_never_revoke_or_remove_credentials() {
        let source = include_str!("extension_manager_extension.rs");
        let region = |label: &'static str, start: &str, end: &str| -> String {
            source
                .split(start)
                .nth(1)
                .and_then(|tail| tail.split(end).next())
                .unwrap_or_else(|| panic!("{label} boundaries"))
                .to_owned()
        };

        // The two handler regions ABUT — the first ends exactly where the
        // second begins — so nothing can be added between them and fall outside
        // both. Slicing the second from its doc comment rather than its `fn`
        // line is deliberate: the promise not to touch a credential is stated
        // there, and a guard that could not see the promise could not check it.
        let marketplace_handler = region(
            "marketplace delete handler",
            "async fn handle_delete_extension_package",
            "/// Uninstall an installed extension the marketplace cannot name",
        );
        let removal_handler = region(
            "extension removal handler",
            "/// Uninstall an installed extension the marketplace cannot name",
            "async fn manage_extensions_impl",
        );
        let marketplace_report = region(
            "marketplace delete report",
            "fn marketplace_deletion_report",
            "struct ValidatedExtensionRemoval",
        );
        let removal_execution = region(
            "extension removal execution",
            "struct ValidatedExtensionRemoval",
            "/// The `manage_extensions` enable door",
        );

        for (label, body) in [
            ("marketplace delete handler", &marketplace_handler),
            ("extension removal handler", &removal_handler),
            ("marketplace delete report", &marketplace_report),
            ("extension removal execution", &removal_execution),
        ] {
            for forbidden in [
                "revoke(",
                "remove_secret",
                "delete_secret",
                "env_keys.clear",
            ] {
                assert!(
                    !body.contains(forbidden),
                    "{label} must preserve possibly shared credentials: {forbidden}"
                );
            }
        }

        // Preserving them silently is not enough: the result and the approval
        // card both have to say so, or a user reading either concludes the key
        // went with the extension.
        assert!(marketplace_report.contains("credentialsPreserved"));
        assert!(removal_execution.contains("credentialsPreserved"));
        assert!(removal_handler.contains("credentials"));
    }

    #[test]
    fn extension_removal_batch_preflight_is_bounded_ordered_and_unique() {
        let names = preflight_remove_extension_names(RemoveExtensionParams {
            extension_name: Some("first".to_owned()),
            extension_names: vec!["second".to_owned()],
        })
        .expect("valid batch");
        assert_eq!(names, vec!["first", "second"]);

        assert!(preflight_remove_extension_names(RemoveExtensionParams {
            extension_name: Some("same".to_owned()),
            extension_names: vec!["same".to_owned()],
        })
        .unwrap_err()
        .to_string()
        .contains("duplicates"));
        assert!(preflight_remove_extension_names(RemoveExtensionParams {
            extension_name: None,
            extension_names: Vec::new(),
        })
        .is_err());
        assert!(preflight_remove_extension_names(RemoveExtensionParams {
            extension_name: None,
            extension_names: (0..51).map(|index| format!("ext-{index}")).collect(),
        })
        .unwrap_err()
        .to_string()
        .contains("50"));

        let schema = serde_json::to_value(schema_for!(RemoveExtensionParams)).unwrap();
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .expect("removal schema properties");
        assert!(properties.contains_key("extension_name"));
        assert!(properties.contains_key("extension_names"));
        assert_eq!(
            properties
                .get("extension_names")
                .and_then(|value| value.get("maxItems"))
                .and_then(Value::as_u64),
            Some(50)
        );
        assert!(
            serde_json::from_value::<RemoveExtensionParams>(serde_json::json!({
                "extension_name": "one",
                "unexpected": true
            }))
            .is_err()
        );
        // The camelCase spelling our own payloads hand back is forgiven, not
        // taught — the schema above still says `extension_name`.
        assert_eq!(
            serde_json::from_value::<RemoveExtensionParams>(
                serde_json::json!({ "extensionName": "one" })
            )
            .expect("the camelCase alias is accepted")
            .extension_name
            .as_deref(),
            Some("one")
        );
    }

    /// ⚠ **The directory is only this extension's when the config key agrees.**
    /// Without that test `remove_extension` would delete
    /// `<extensions>/<config name>` on nothing more than a name — which for a
    /// hand-configured MCP server that never installed anything means deleting
    /// whatever happens to sit at that name.
    #[test]
    fn a_package_directory_is_only_removable_when_it_belongs_to_the_entry_being_removed() {
        let root = tempfile::tempdir().unwrap();
        let root_path = std::fs::canonicalize(root.path()).unwrap();
        for name in ["Alpha", "Beta", "Delta"] {
            std::fs::create_dir_all(root_path.join(name)).unwrap();
        }

        let owns_its_own = package_entry("Alpha", &root_path.join("Alpha"));
        assert_eq!(
            removal_install_dir("alpha", &owns_its_own.config, Some(&root_path)).unwrap(),
            Some(root_path.join("Alpha"))
        );

        // Named after nothing on disk, and launching out of nothing on disk:
        // an `npx` server somebody added by hand. Removable, but its removal is
        // the config row alone.
        let hand_configured = ExtensionEntry {
            enabled: true,
            config: ExtensionConfig::Stdio {
                name: "Gamma".to_owned(),
                description: "hand-configured".to_owned(),
                cmd: "npx".to_owned(),
                args: vec!["-y".to_owned(), "gamma-server".to_owned()],
                envs: crate::agents::extension::Envs::default(),
                env_keys: Vec::new(),
                timeout: Some(300),
                bundled: None,
                available_tools: Vec::new(),
            },
        };
        assert_eq!(
            removal_install_dir("gamma", &hand_configured.config, Some(&root_path)).unwrap(),
            None
        );

        // No arguments naming a directory, but one exists under its own name.
        let named_only = ExtensionEntry {
            enabled: true,
            config: ExtensionConfig::stdio("Delta", "delta-server", "named only", 30_u64),
        };
        assert_eq!(
            removal_install_dir("delta", &named_only.config, Some(&root_path)).unwrap(),
            Some(root_path.join("Delta"))
        );

        // Launching out of ANOTHER package's directory is refused rather than
        // resolved to nothing: silently removing only the config row orphans a
        // tree, and removing the tree breaks the package that owns it.
        let squatter = package_entry("Alpha", &root_path.join("Beta"));
        let refusal = removal_install_dir("alpha", &squatter.config, Some(&root_path))
            .expect_err("an entry pointed at another package's directory is refused");
        assert!(
            refusal.to_string().contains("different installed package"),
            "{refusal}"
        );

        // A machine that never installed a `.brxt` has no extensions root at
        // all, and that is not an error — nothing owns a directory.
        assert_eq!(
            removal_install_dir("alpha", &owns_its_own.config, None).unwrap(),
            None
        );
    }

    /// The three refusals, in the order [`validated_extension_removal`] asks
    /// them. The last assertion is the one worth keeping: a public caller must
    /// not be able to tell an installed private extension from an absent one,
    /// which is why the privacy gate sits ABOVE the not-installed answer.
    #[test]
    fn removal_refuses_capabilities_and_gates_private_names_above_the_not_installed_answer() {
        let capability = validated_extension_removal("developer", public_enforcing(), None, &[])
            .expect_err("a built-in capability is not an installed extension");
        assert!(capability.to_string().contains("developer"), "{capability}");

        let private = validated_extension_removal("ucsfomopagent", public_enforcing(), None, &[])
            .expect_err("a public caller may not remove a private extension");
        assert!(private.to_string().contains("private"), "{private}");
        assert!(
            !private.to_string().contains("not installed"),
            "a refusal that says whether a private extension is installed is the \
             install-state oracle finding 13 closed: {private}"
        );

        let absent =
            validated_extension_removal("no-such-extension", public_enforcing(), None, &[])
                .expect_err("an absent public name is reported absent");
        assert!(absent.to_string().contains("not installed"), "{absent}");
        assert!(
            absent.to_string().contains("search_available_extensions"),
            "a not-installed answer must name the inventory to look in, or the \
             model guesses another name: {absent}"
        );
    }

    #[test]
    fn enable_of_unknown_extension_is_not_found() {
        let err = check_enable_allowed(None, false, "ghost", public_enforcing()).unwrap_err();
        assert_eq!(err.code, ErrorCode::RESOURCE_NOT_FOUND);
        assert!(err.message.contains("not found"), "{}", err.message);
    }

    #[test]
    fn missing_extension_guidance_prevents_name_guessing_and_distinguishes_installation() {
        let err = check_enable_allowed(None, false, "Spoke Agent", public_enforcing()).unwrap_err();
        for instruction in [
            "exact installed name",
            "search_available_extensions",
            "do not retry guessed names",
            "search_marketplace_extensions",
            "install_extension",
        ] {
            assert!(
                err.message.contains(instruction),
                "{instruction}: {}",
                err.message
            );
        }
        assert!(
            !err.message.contains("spokeagent"),
            "do not disclose or guess an installed alias"
        );
    }

    #[test]
    fn enable_of_operator_disabled_extension_is_refused_with_guidance() {
        // #42: `enabled: false` written into config.yaml must be a dependable
        // pin — the agent may not silently re-enable what the operator turned
        // off. `persisted: true` = the entry exists in the on-disk config.
        let err = check_enable_allowed(
            Some(entry(false)),
            true,
            "publicfixture",
            public_enforcing(),
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
        assert!(err.message.contains("disabled"), "{}", err.message);
        assert!(
            err.message.contains("operator"),
            "must say who disabled it: {}",
            err.message
        );
        assert!(
            err.message.contains("ask the user"),
            "must direct the model to the user: {}",
            err.message
        );
    }

    #[test]
    fn explicit_user_grant_can_attach_a_public_operator_disabled_extension() {
        let config = check_enable_allowed_with_user_grant(
            Some(entry(false)),
            true,
            "publicfixture",
            public_enforcing(),
        )
        .expect("proof-backed user approval may override the operator pin for this chat");
        assert_eq!(config.name(), "publicfixture");

        let private = check_enable_allowed_with_user_grant(
            Some(ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::stdio(
                    "ucsfomopagent",
                    "fixture-command",
                    "private fixture",
                    30_u64,
                ),
            }),
            true,
            "ucsfomopagent",
            public_enforcing(),
        )
        .expect_err("user approval cannot cross the public-to-private boundary");
        assert!(
            private.message.contains("private extension"),
            "{}",
            private.message
        );

        let pinned_private = check_enable_allowed_with_user_grant(
            Some(ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::stdio(
                    "ucsfomopagent",
                    "fixture-command",
                    "private fixture",
                    30_u64,
                ),
            }),
            true,
            "ucsfomopagent",
            CallCapability::for_test(ProviderTier::Private, true),
        )
        .expect_err("explicit grant never overrides the operator pin for a private extension");
        assert!(
            pinned_private.message.contains("operator"),
            "{}",
            pinned_private.message
        );
    }

    #[test]
    fn enable_of_default_off_injected_extension_is_allowed() {
        // An injected non-capability entry can carry `enabled: false` without
        // an operator writing it. That is not an operator pin.
        let config = check_enable_allowed(
            Some(entry(false)),
            false,
            "publicfixture",
            public_enforcing(),
        )
        .expect("allowed");
        assert_eq!(config.name(), "publicfixture");
    }

    #[test]
    fn enable_of_config_enabled_extension_passes_the_config_through() {
        let config =
            check_enable_allowed(Some(entry(true)), true, "publicfixture", public_enforcing())
                .expect("allowed");
        assert_eq!(config.name(), "publicfixture");
    }

    /// The same entry shape as [`entry`], but under an arbitrary name and
    /// always enabled — the tier is resolved from the NAME the model asked
    /// for, never from the config record, so this fixture only has to carry a
    /// name that is not itself a refusal for some other reason.
    fn entry_for(name: &str) -> ExtensionEntry {
        ExtensionEntry {
            enabled: true,
            config: ExtensionConfig::stdio(name, "fixture-command", "fixture", 30_u64),
        }
    }

    fn capability_entries() -> [ExtensionEntry; 2] {
        [
            ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::Builtin {
                    name: "developer".to_owned(),
                    display_name: Some("Developer".to_owned()),
                    description: "built-in fixture".to_owned(),
                    timeout: Some(30),
                    bundled: Some(true),
                    available_tools: Vec::new(),
                },
            },
            ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::Platform {
                    name: "Extension Manager".to_owned(),
                    description: "platform fixture".to_owned(),
                    bundled: Some(true),
                    available_tools: Vec::new(),
                },
            },
        ]
    }

    #[test]
    fn builtin_and_platform_capabilities_cannot_be_enabled_as_extensions() {
        for entry in capability_entries() {
            let name = entry.config.name();
            let error = check_enable_allowed(
                Some(entry),
                true,
                &name,
                CallCapability::for_test(ProviderTier::Private, true),
            )
            .expect_err("capabilities are managed outside Extension Manager");
            assert!(error.message.contains("capability"), "{}", error.message);
        }
    }

    /// Gate F1 (#56): `extensionmanager__manage_extensions` is not a tool call
    /// into a private server — it is the call that SPAWNS one. Enabling
    /// `ucsfomopagent` pulls its `CLINICAL_RECORDS_*` secrets out of the
    /// keychain and opens a session to the UCSF CDW; being refused afterwards,
    /// at the first tool call, is already too late.
    #[test]
    fn a_public_caller_may_not_enable_a_private_extension() {
        use ProviderTier::{Private, Public};
        // Pure, like its four siblings above, so it needs no global config.
        let e = check_enable_allowed(
            Some(entry_for("ucsfomopagent")),
            false,
            "ucsfomopagent",
            CallCapability::for_test(Public, true),
        )
        .unwrap_err();
        assert!(e.message.contains("marketplace"), "{}", e.message);
        assert!(e.message.contains("private"), "{}", e.message);
        check_enable_allowed(
            Some(entry_for("ucsfomopagent")),
            false,
            "ucsfomopagent",
            CallCapability::for_test(Private, true),
        )
        .expect("a private caller may enable it");
        check_enable_allowed(
            Some(entry_for("publicfixture")),
            false,
            "publicfixture",
            CallCapability::for_test(Public, true),
        )
        .expect("public extensions are unaffected");
    }

    /// Task 30's matrix ROW 11, asserted where the gate lives.
    ///
    /// ⚠ **Why this row is here and not in `crates/biorouter/tests/privacy_toggle.rs`
    /// with the other seventeen.** Gate F1 is an ON-THE-TOOL-CALL-PATH gate: the
    /// master toggle reaches it through the [`CallCapability`] that
    /// `Agent::dispatch_tool_call` already sampled, never through a second read
    /// of the global. So the OFF column of this row is *by construction* a
    /// capability whose `enforced` is false — and the only constructor for one
    /// is `CallCapability::for_test`, which is `#[cfg(test)] pub(crate)` and
    /// therefore invisible to an integration binary. Driving it end-to-end
    /// instead would mean writing `ucsfomopagent` into the developer's real
    /// `config.yaml`, because `get_extension_entry_by_name` reads the global
    /// config: the ON column would otherwise stop at "not found" and never
    /// reach the arm this row is about.
    ///
    /// That `CallCapability::sample` itself reads the master toggle is asserted
    /// end-to-end by the matrix's rows 3, 4/5 and 7, which flip the global and
    /// watch a real dispatch change its answer. This row is the other half: the
    /// gate honours what the sample handed it.
    #[test]
    fn a_public_model_never_enables_a_private_extension_through_the_manager() {
        use ProviderTier::Public;
        // ON — the shipped default: a public model may not spawn the clinical
        // connector's process.
        assert!(check_enable_allowed(
            Some(entry_for("ucsfomopagent")),
            false,
            "ucsfomopagent",
            CallCapability::for_test(Public, true),
        )
        .is_err());
        // The manager's product contract is stricter than the diagnostic
        // privacy toggle: a public model may never install or spawn a private
        // extension through this agent-controlled door.
        let off = check_enable_allowed(
            Some(entry_for("ucsfomopagent")),
            false,
            "ucsfomopagent",
            CallCapability::for_test(Public, false),
        )
        .expect_err("the manager never grants a private extension to a public model");
        assert!(off.message.contains("private extension"), "{}", off.message);
        // …and the toggle silences GATE F1 ONLY. #42's operator pin is not a
        // privacy control and must survive the master switch being off, or
        // turning privacy tiers off would quietly hand the agent the power to
        // re-enable everything the operator disabled.
        let err = check_enable_allowed(
            Some(entry(false)),
            true,
            "publicfixture",
            CallCapability::for_test(Public, false),
        )
        .unwrap_err();
        assert!(err.message.contains("operator"), "{}", err.message);
    }

    /// **Task 43 / DR-23's gate, Step 3.1 — the Gate F1 arm.**
    ///
    /// F1 fires BEFORE the extension is spawned, off the name the model asked
    /// for. Enabling a private connector is not a call into it, it is the call
    /// that SPAWNS it — pulling its credentials out of the keychain and opening
    /// the session — so a rename that got past this gate would already have
    /// disclosed something by the time Gate C looked.
    ///
    /// Both the name asked for and the entry's own `name` are the renamed one;
    /// the `--directory` argument is the only surviving link to the install.
    #[test]
    fn a_renamed_private_extension_is_still_refused_by_gate_f1() {
        let install_dir = "/home/researcher/.config/biorouter/extensions/UCSFOMOPAgent";
        crate::privacy::provenance::insert_test_record_at(
            "f1-omop-as-installed",
            "ucsfomopagent",
            Some(install_dir),
        );
        let renamed = ExtensionEntry {
            enabled: true,
            config: ExtensionConfig::Stdio {
                name: "f1-mystuff".to_string(),
                description: "renamed by hand in config.yaml".to_string(),
                cmd: "uv".to_string(),
                args: vec![
                    "run".to_string(),
                    "--directory".to_string(),
                    install_dir.to_string(),
                    "server.py".to_string(),
                ],
                envs: crate::agents::extension::Envs::default(),
                env_keys: vec![],
                timeout: Some(300),
                bundled: None,
                available_tools: vec![],
            },
        };
        assert_eq!(
            crate::privacy::classify_extension("f1-mystuff"),
            ProviderTier::Public,
            "the fixture only discriminates if the NAME alone reads public"
        );

        let err = check_enable_allowed(Some(renamed), false, "f1-mystuff", public_enforcing())
            .expect_err("renaming the config entry must not let a public model spawn it");
        assert!(err.message.contains("private extension"), "{}", err.message);
    }

    /// The capability a session bound to `institution`'s private model carries.
    fn bound_to(institution: &str) -> CallCapability {
        CallCapability::for_test_affiliated(
            ProviderTier::Private,
            true,
            Some(crate::privacy::affiliation::ModelAffiliation::institution(
                crate::privacy::affiliation::InstitutionId::new(institution),
            )),
        )
    }

    /// **Task 48, surface 5 — extension enablement.** Bind and enablement are
    /// the same mismatch found from opposite ends; this is the end where the
    /// model is already bound and the extension arrives.
    ///
    /// ⚠ **The agent is refused rather than warned, and that is DR-26's
    /// user/agent asymmetry rather than an inconsistency with the bind
    /// surface.** A user who insists may proceed past a warning; an agent never
    /// clears one automatically — it escalates to the user or the call does not
    /// happen. `extensionmanager__manage_extensions` is the agent's enable path,
    /// so the call does not happen. The USER's enable path is
    /// `/agent/add_extension`, which warns and proceeds.
    ///
    /// Enabling is also the earliest point at which this can be caught:
    /// spawning `ucsfomopagent` pulls its `CLINICAL_RECORDS_*` secrets out of
    /// the keychain and opens a session to the UCSF CDW, which is a disclosure
    /// no later refusal takes back.
    #[test]
    fn an_agent_may_not_enable_an_extension_its_model_is_affiliation_incompatible_with() {
        let err = check_enable_allowed(
            Some(entry_for("ucsfomopagent")),
            false,
            "ucsfomopagent",
            bound_to("stanford"),
        )
        .expect_err("a Stanford-covered model may not spawn a UCSF connector");
        let expected = bound_to("stanford")
            .cross_affiliation_warning(
                "ucsfomopagent",
                &crate::privacy::resolve_extension("ucsfomopagent", None),
            )
            .expect("the fixture only discriminates if this pair really mismatches");
        assert!(err.message.contains(&expected), "{}", err.message);
    }

    /// The same call on a LOCAL private model is allowed — `Local` is DR-26's
    /// most permissive affiliation, not a peer of the institutions.
    #[test]
    fn a_local_model_may_enable_every_private_extension() {
        let config = check_enable_allowed(
            Some(entry_for("ucsfomopagent")),
            false,
            "ucsfomopagent",
            CallCapability::for_test_affiliated(
                ProviderTier::Private,
                true,
                Some(crate::privacy::affiliation::ModelAffiliation::Local),
            ),
        )
        .expect("a local model discloses nothing, so nothing needs papering");
        assert_eq!(config.name(), "ucsfomopagent");
    }

    /// **Finding 13's ordering consequence, on the enable path.**
    ///
    /// Gate F1 now runs above the not-found branch. A public caller asking to
    /// enable a private connector this machine does not have gets the tier
    /// refusal, not `not found` — so the two answers no longer tell it which
    /// private connectors are installed here. That is the same secret the
    /// catalogue fix stopped printing outright one function away.
    #[test]
    fn a_public_caller_cannot_tell_an_absent_private_extension_from_an_installed_one() {
        let absent =
            check_enable_allowed(None, false, "ucsfomopagent", public_enforcing()).unwrap_err();
        let installed = check_enable_allowed(
            Some(entry_for("ucsfomopagent")),
            false,
            "ucsfomopagent",
            public_enforcing(),
        )
        .unwrap_err();
        assert_eq!(absent.code, installed.code);
        assert_eq!(absent.message, installed.message);
        assert!(
            absent.message.contains("private extension"),
            "{}",
            absent.message
        );
        assert_ne!(
            absent.code,
            ErrorCode::RESOURCE_NOT_FOUND,
            "`not found` for a private name is an install-state oracle"
        );

        // The operator pin is the other install-state answer, and it is behind
        // the same gate now. #42's refusal must still be what a PUBLIC extension
        // gets, or the reorder swallowed it.
        let pinned = check_enable_allowed(
            Some(entry(false)),
            true,
            "publicfixture",
            public_enforcing(),
        )
        .unwrap_err();
        assert!(pinned.message.contains("operator"), "{}", pinned.message);
        // …and a name that is public and absent still says so, which is what
        // keeps `enable_of_unknown_extension_is_not_found` a live assertion
        // rather than one the reorder made unreachable.
        assert_eq!(
            check_enable_allowed(None, false, "ghost", public_enforcing())
                .unwrap_err()
                .code,
            ErrorCode::RESOURCE_NOT_FOUND
        );
    }

    /// **The oracle, asserted at this door against EVERY install state — the
    /// gate for the seam finding 4's fix left between the two enable doors.**
    ///
    /// The test above compares two states (absent, installed). The third is the
    /// one the workspace's copy of this gate got wrong: an extension that is
    /// installed *and pinned off by the operator*. That copy asked #42's pin
    /// first, so `workspace_open {new:{extensions}}` answered "…is disabled in
    /// the Biorouter configuration (enabled: false)" to a public caller who may
    /// not have the connector at all — an install-state oracle, reopened one
    /// function away from where finding 13 had just closed it. Both doors now
    /// call `refusal::extension_enable_refusal`, so this asserts the property at
    /// the shared gate through this door and its twin asserts it through the
    /// other two.
    ///
    /// The last two assertions are what stop the fix being "refuse everything":
    /// a caller who MAY have the extension still meets the pin, and a public
    /// caller still meets it for a public extension.
    #[test]
    fn no_install_state_reaches_a_caller_who_may_not_enable_the_extension() {
        const NAME: &str = "ucsfomopagent";
        let pinned_off = || {
            let mut e = entry_for(NAME);
            e.enabled = false;
            e
        };

        let absent = check_enable_allowed(None, false, NAME, public_enforcing()).unwrap_err();
        let installed =
            check_enable_allowed(Some(entry_for(NAME)), false, NAME, public_enforcing())
                .unwrap_err();
        let pinned =
            check_enable_allowed(Some(pinned_off()), true, NAME, public_enforcing()).unwrap_err();

        for (state, err) in [
            ("installed and enabled", &installed),
            ("installed and pinned off by the operator", &pinned),
        ] {
            assert_eq!(
                (absent.code, absent.message.to_string()),
                (err.code, err.message.to_string()),
                "the refusal tells a public caller that the private connector is {state}"
            );
        }
        assert!(
            absent.message.contains("private extension"),
            "{}",
            absent.message
        );
        for leak in ["enabled: false", "not found", "operator"] {
            assert!(
                !absent.message.contains(leak),
                "the refusal a caller who may not see this connector gets names local \
                 state (`{leak}`): {}",
                absent.message
            );
        }

        // …and the pin is not swallowed. A caller ENTITLED to the connector still
        // meets #42, which is the half a reorder breaks silently.
        let entitled = check_enable_allowed(
            Some(pinned_off()),
            true,
            NAME,
            CallCapability::for_test(ProviderTier::Private, true),
        )
        .unwrap_err();
        assert!(
            entitled.message.contains("operator"),
            "{}",
            entitled.message
        );
        // Same for a public caller and a PUBLIC extension — every case #42 was
        // written for.
        let public_pin = check_enable_allowed(
            Some(entry(false)),
            true,
            "publicfixture",
            public_enforcing(),
        )
        .unwrap_err();
        assert!(
            public_pin.message.contains("operator"),
            "{}",
            public_pin.message
        );
    }

    /// A PRIVATE caller reaches every branch the reorder moved past, unchanged.
    #[test]
    fn the_reorder_does_not_touch_a_private_callers_answers() {
        use ProviderTier::Private;
        let cap = CallCapability::for_test(Private, true);
        assert_eq!(
            check_enable_allowed(None, false, "ucsfomopagent", cap)
                .unwrap_err()
                .code,
            ErrorCode::RESOURCE_NOT_FOUND,
            "a private caller is still told when a private extension is absent"
        );
        let pinned =
            check_enable_allowed(Some(entry(false)), true, "publicfixture", cap).unwrap_err();
        assert!(pinned.message.contains("operator"), "{}", pinned.message);
    }

    /// DR-15's master opt-out reaches the affiliation arm through the same
    /// capability, not through a second read of the global.
    #[test]
    fn the_master_toggle_silences_the_affiliation_arm_of_the_enable_gate() {
        let cap = CallCapability::for_test_affiliated(
            ProviderTier::Private,
            false,
            Some(crate::privacy::affiliation::ModelAffiliation::institution(
                crate::privacy::affiliation::InstitutionId::new("stanford"),
            )),
        );
        let config = check_enable_allowed(
            Some(entry_for("ucsfomopagent")),
            false,
            "ucsfomopagent",
            cap,
        )
        .expect("with privacy tiers off, nothing is refused");
        assert_eq!(config.name(), "ucsfomopagent");
    }

    // ----------------------------------------------------------------------
    // Issue #56 findings 13 and 14, WIRING. Nine guards in this campaign shipped
    // correct, tested and called by nothing, so each new gate is asserted twice:
    // once through the real `call_tool` entry point (below), and once
    // structurally against this file's production text
    // (`both_new_gates_have_production_callers`), because a behavioural test can
    // be satisfied by a helper the tool dispatch does not actually reach.
    // ----------------------------------------------------------------------

    /// A live `ExtensionManagerClient` over a real `ExtensionManager`, reached
    /// exactly as `configure_agent` builds it.
    /// The fourth element is a **real** session id, created in the same
    /// temporary store the manager owns. The handler now writes
    /// `enabled_extensions.v0` before it reports `attached`/`detached`, and
    /// `update_extension_state` answers `Ok(None)` for a session that does not
    /// exist — so a fixture with an invented id would fail the write and read
    /// as a persistence bug rather than as the fixture gap it is.
    async fn a_live_tool_client() -> (
        tempfile::TempDir,
        std::sync::Arc<crate::agents::extension_manager::ExtensionManager>,
        ExtensionManagerClient,
        String,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let em = std::sync::Arc::new(
            crate::agents::extension_manager::ExtensionManager::new_without_provider(
                dir.path().to_path_buf(),
            ),
        );
        let context = PlatformExtensionContext {
            extension_manager: Some(std::sync::Arc::downgrade(&em)),
            session_manager: em.get_context().session_manager.clone(),
        };
        let session = em
            .get_context()
            .session_manager
            .create_session(
                dir.path().to_path_buf(),
                "fixture".to_owned(),
                crate::session::session_manager::SessionType::User,
            )
            .await
            .expect("the fixture session store accepts a session");
        let client = ExtensionManagerClient::new(context).expect("the platform client builds");
        (dir, em, client, session.id)
    }

    #[tokio::test]
    async fn attach_result_names_every_tool_that_is_immediately_callable() {
        let (_dir, em, client, session_id) = a_live_tool_client().await;
        let config = ExtensionConfig::Platform {
            name: "extensionmanager".to_owned(),
            description: "Extension Manager".to_owned(),
            bundled: Some(true),
            available_tools: Vec::new(),
        };

        let content = client
            .attach_extension_to_session(
                "Extension Manager".to_owned(),
                config,
                CallCapability::for_test(ProviderTier::Public, true),
                &session_id,
            )
            .await
            .expect("the platform extension attaches");
        let text = content[0].as_text().expect("JSON text response");
        let payload: serde_json::Value = serde_json::from_str(&text.text).unwrap();
        let reported = payload["availableTools"]
            .as_array()
            .expect("attach reports an exact tool roster")
            .iter()
            .map(|name| name.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let mut actual = em
            .get_prefixed_tools_for_extension_and_capability(
                "extensionmanager",
                CallCapability::for_test(ProviderTier::Public, true),
            )
            .await
            .unwrap()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        actual.sort();

        assert_eq!(reported, actual);
        assert!(!reported.is_empty(), "the fixture must expose real tools");
        assert_eq!(payload["toolAvailability"], "immediate");
        assert!(payload["guidance"]
            .as_str()
            .unwrap()
            .contains("callable now in this turn"));
    }

    #[tokio::test]
    async fn attached_install_result_names_every_tool_that_is_immediately_callable() {
        let (_dir, em, client, _session_id) = a_live_tool_client().await;
        let context = PlatformExtensionContext {
            extension_manager: Some(std::sync::Arc::downgrade(&em)),
            session_manager: em.get_context().session_manager.clone(),
        };
        let config = ExtensionConfig::stdio(
            "publicfixture",
            "unused-fixture-command",
            "ordinary public install fixture",
            30_u64,
        );
        em.add_client(
            config.key(),
            config,
            std::sync::Arc::new(
                ExtensionManagerClient::new(context).expect("fixture client builds"),
            ),
            None,
            None,
        )
        .await;
        let report = crate::extension_install::InstallReport {
            install_id: "install-fixture".to_owned(),
            state: crate::extension_install::InstallState::Attached,
            extension_name: Some("publicfixture".to_owned()),
            display_name: Some("Public Fixture".to_owned()),
            configured_keys: Vec::new(),
            skills: Vec::new(),
            enabled: true,
            operator_pinned_off: false,
        };
        let cap = CallCapability::for_test(ProviderTier::Public, true);
        let payload: serde_json::Value =
            serde_json::from_str(&client.install_report_json(&report, cap).await).unwrap();
        let reported = payload["availableTools"]
            .as_array()
            .expect("attached install reports an exact tool roster")
            .iter()
            .map(|name| name.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();
        let mut actual = em
            .get_prefixed_tools_for_extension_and_capability("publicfixture", cap)
            .await
            .unwrap()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        actual.sort();

        assert_eq!(reported, actual);
        assert!(!reported.is_empty(), "the fixture must expose real tools");
        assert_eq!(payload["state"]["state"], "attached");
        assert_eq!(payload["toolAvailability"], "immediate");
    }

    #[tokio::test]
    async fn installed_but_unattached_report_does_not_claim_tools_are_callable() {
        let (_dir, _em, client, _session_id) = a_live_tool_client().await;
        let report = crate::extension_install::InstallReport {
            install_id: "install-fixture".to_owned(),
            state: crate::extension_install::InstallState::Installed,
            extension_name: Some("publicfixture".to_owned()),
            display_name: Some("Public Fixture".to_owned()),
            configured_keys: Vec::new(),
            skills: Vec::new(),
            enabled: false,
            operator_pinned_off: false,
        };
        let payload: serde_json::Value = serde_json::from_str(
            &client
                .install_report_json(
                    &report,
                    CallCapability::for_test(ProviderTier::Public, true),
                )
                .await,
        )
        .unwrap();

        assert_eq!(payload["state"]["state"], "installed");
        assert_eq!(payload["toolAvailability"], "notAttached");
        assert!(payload.get("availableTools").is_none());
        assert!(payload["guidance"]
            .as_str()
            .unwrap()
            .contains("Attach the extension"));
    }

    #[tokio::test]
    async fn detach_result_names_every_tool_that_is_revoked_immediately() {
        let (_dir, em, client, session_id) = a_live_tool_client().await;
        let context = PlatformExtensionContext {
            extension_manager: Some(std::sync::Arc::downgrade(&em)),
            session_manager: em.get_context().session_manager.clone(),
        };
        let config = ExtensionConfig::stdio(
            "publicfixture",
            "unused-fixture-command",
            "ordinary public extension fixture",
            30_u64,
        );
        em.add_client(
            config.key(),
            config,
            std::sync::Arc::new(
                ExtensionManagerClient::new(context).expect("fixture client builds"),
            ),
            None,
            None,
        )
        .await;
        let cap = CallCapability::for_test(ProviderTier::Public, true);
        let mut before = em
            .get_prefixed_tools_for_extension_and_capability("publicfixture", cap)
            .await
            .unwrap()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        before.sort();
        assert!(!before.is_empty(), "the fixture must expose real tools");

        let content = client
            .manage_extensions_impl(
                ManageExtensionAction::Disable,
                "publicfixture".to_owned(),
                cap,
                false,
                &session_id,
            )
            .await
            .expect("the ordinary public extension detaches");
        let text = content[0].as_text().expect("JSON text response");
        let payload: serde_json::Value = serde_json::from_str(&text.text).unwrap();
        let reported = payload["removedTools"]
            .as_array()
            .expect("detach reports an exact revoked roster")
            .iter()
            .map(|name| name.as_str().unwrap().to_owned())
            .collect::<Vec<_>>();

        assert_eq!(reported, before);
        assert_eq!(payload["toolAvailability"], "revokedImmediately");
        assert!(!em.is_extension_enabled("publicfixture").await);
        assert!(em
            .get_prefixed_tools_for_extension_and_capability("publicfixture", cap)
            .await
            .unwrap()
            .is_empty());
    }

    /// A fixture extension the manager can hold without spawning anything.
    async fn load_public_fixture(
        em: &std::sync::Arc<crate::agents::extension_manager::ExtensionManager>,
    ) -> String {
        let context = PlatformExtensionContext {
            extension_manager: Some(std::sync::Arc::downgrade(em)),
            session_manager: em.get_context().session_manager.clone(),
        };
        let config = ExtensionConfig::stdio(
            "publicfixture",
            "unused-fixture-command",
            "ordinary public extension fixture",
            30_u64,
        );
        let key = config.key();
        em.add_client(
            key.clone(),
            config,
            std::sync::Arc::new(
                ExtensionManagerClient::new(context).expect("fixture client builds"),
            ),
            None,
            None,
        )
        .await;
        key
    }

    async fn session_roster(
        em: &std::sync::Arc<crate::agents::extension_manager::ExtensionManager>,
        session_id: &str,
    ) -> Vec<String> {
        use crate::session::extension_data::ExtensionState;
        let session = em
            .get_context()
            .session_manager
            .get_session(session_id, false)
            .await
            .expect("the fixture session is readable");
        crate::session::EnabledExtensionsState::from_extension_data(&session.extension_data)
            .map(|state| {
                state
                    .extensions
                    .iter()
                    .map(|config| config.key())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    /// **The ordering, not merely the write.** The row must be on disk by the
    /// time the tool says `"attached"`; a fix that publishes the catalog event
    /// and still leaves the write to the reply loop's post-batch block passes
    /// every behavioural test except this one.
    #[tokio::test]
    async fn an_attach_writes_the_session_row_before_it_reports_attached() {
        let (_dir, em, client, session_id) = a_live_tool_client().await;
        let config = ExtensionConfig::Platform {
            name: "extensionmanager".to_owned(),
            description: "Extension Manager".to_owned(),
            bundled: Some(true),
            available_tools: Vec::new(),
        };
        let key = config.key();

        let content = client
            .attach_extension_to_session(
                "Extension Manager".to_owned(),
                config,
                public_enforcing(),
                &session_id,
            )
            .await
            .expect("the platform extension attaches");
        let text = content[0].as_text().expect("JSON text response");
        let payload: serde_json::Value = serde_json::from_str(&text.text).unwrap();
        assert_eq!(payload["sessionState"], "attached");

        let roster = session_roster(&em, &session_id).await;
        assert!(
            roster.contains(&key),
            "the row must already name {key} when the tool reports attached: {roster:?}"
        );
    }

    /// A failed write must not be a `warn!` behind a success — the failure mode
    /// the reply loop's post-batch block still has. And the extension this call
    /// brought in must not be left loaded against a row that never recorded it.
    #[tokio::test]
    async fn a_failed_session_write_does_not_report_attached_and_leaves_no_orphan() {
        let (_dir, em, client, _session_id) = a_live_tool_client().await;
        let config = ExtensionConfig::Platform {
            name: "extensionmanager".to_owned(),
            description: "Extension Manager".to_owned(),
            bundled: Some(true),
            available_tools: Vec::new(),
        };
        let key = config.key();

        let error = client
            .attach_extension_to_session(
                "Extension Manager".to_owned(),
                config,
                public_enforcing(),
                "no-such-session",
            )
            .await
            .expect_err("an unwritable session must not report attached");
        assert!(
            error.message.contains("could not be recorded"),
            "{}",
            error.message
        );
        assert!(
            !em.is_extension_enabled(&key).await,
            "a refused attach left {key} loaded"
        );
    }

    /// The rollback is scoped to a true false->true transition. An
    /// unconditional `remove_extension` would unload an extension that `/ext:`,
    /// a config default or an earlier call had already put in the manager.
    #[tokio::test]
    async fn an_attach_over_an_already_loaded_extension_does_not_unload_it_when_the_write_fails() {
        let (_dir, em, client, _session_id) = a_live_tool_client().await;
        let key = load_public_fixture(&em).await;
        assert!(em.is_extension_enabled(&key).await);

        let error = client
            .attach_extension_to_session(
                "publicfixture".to_owned(),
                ExtensionConfig::stdio(
                    "publicfixture",
                    "unused-fixture-command",
                    "ordinary public extension fixture",
                    30_u64,
                ),
                public_enforcing(),
                "no-such-session",
            )
            .await
            .expect_err("an unwritable session must not report attached");
        assert!(
            error.message.contains("could not be recorded"),
            "{}",
            error.message
        );
        assert!(
            em.is_extension_enabled(&key).await,
            "a rollback unloaded an extension this call never added"
        );
    }

    /// Symmetric to the attach test. A fix applied only to the attach arm
    /// leaves a detached extension still checked in the popup and restored on
    /// the next reload.
    #[tokio::test]
    async fn a_disable_removes_the_extension_from_the_session_row() {
        let (_dir, em, client, session_id) = a_live_tool_client().await;
        let key = load_public_fixture(&em).await;
        crate::agents::session_extensions::record(
            &em.get_context().session_manager,
            em.as_ref(),
            &session_id,
        )
        .await
        .expect("the fixture roster is recordable");
        assert!(session_roster(&em, &session_id).await.contains(&key));

        let content = client
            .manage_extensions_impl(
                ManageExtensionAction::Disable,
                "publicfixture".to_owned(),
                public_enforcing(),
                false,
                &session_id,
            )
            .await
            .expect("the ordinary public extension detaches");
        let text = content[0].as_text().expect("JSON text response");
        let payload: serde_json::Value = serde_json::from_str(&text.text).unwrap();
        assert_eq!(payload["sessionState"], "detached");

        let roster = session_roster(&em, &session_id).await;
        assert!(
            !roster.contains(&key),
            "the row still names {key} after a detach: {roster:?}"
        );
    }

    async fn disable(
        client: &ExtensionManagerClient,
        name: &str,
        cap: CallCapability,
        session_id: &str,
    ) -> String {
        let result = client
            .call_tool(
                MANAGE_EXTENSIONS_TOOL_NAME,
                Some(
                    serde_json::json!({ "action": "disable", "extension_name": name })
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
                McpMeta::new(session_id, cap),
                CancellationToken::default(),
            )
            .await
            .expect("the platform client always answers, error or not");
        // The handler reports refusals as `is_error` content rather than a
        // transport error, so the text is where the verdict is.
        result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// **Finding 14, through the production tool call.** `manage_extensions
    /// {disable}` returned before any privacy decision existed on the path:
    /// `cap` was already a parameter, and this branch simply never read it.
    #[tokio::test]
    async fn a_public_chat_may_not_disable_a_private_extension_through_the_tool() {
        let (_dir, em, client, session_id) = a_live_tool_client().await;

        let refused = disable(
            &client,
            "ucsfomopagent",
            CallCapability::for_test(ProviderTier::Public, true),
            &session_id,
        )
        .await;
        assert!(
            refused.contains("private extension"),
            "a public chat dropped the clinical connector: {refused}"
        );

        // The SAME call on a private model succeeds, or the test is passing
        // against a tool that refuses everything.
        let allowed = disable(
            &client,
            "ucsfomopagent",
            CallCapability::for_test(ProviderTier::Private, true),
            &session_id,
        )
        .await;
        assert!(
            allowed.contains(r#""sessionState":"detached""#),
            "{allowed}"
        );

        assert!(
            !em.is_extension_enabled("ucsfomopagent").await,
            "the entitled call must have actually detached the extension"
        );
    }

    #[tokio::test]
    async fn builtin_and_platform_capabilities_cannot_be_disabled_as_extensions() {
        let (_dir, em, client, session_id) = a_live_tool_client().await;
        for entry in capability_entries() {
            let name = entry.config.name();
            em.add_extension(entry.config)
                .await
                .unwrap_or_else(|error| panic!("the {name} capability loads: {error}"));
            let refusal = disable(
                &client,
                &name,
                CallCapability::for_test(ProviderTier::Private, true),
                &session_id,
            )
            .await;
            assert!(refusal.contains("capability"), "{name}: {refusal}");
            assert!(
                em.is_extension_enabled(&crate::config::extensions::name_to_key(&name))
                    .await,
                "a rejected disable detached {name}"
            );
        }
    }

    /// DR-15's master opt-out reaches the disable gate through the capability.
    #[tokio::test]
    async fn the_master_toggle_silences_the_disable_gate() {
        let (_dir, _em, client, session_id) = a_live_tool_client().await;
        let text = disable(
            &client,
            "ucsfomopagent",
            CallCapability::for_test(ProviderTier::Public, false),
            &session_id,
        )
        .await;
        assert!(
            text.contains(r#""sessionState":"detached""#),
            "with privacy tiers off nothing is refused: {text}"
        );
    }

    /// ⚠ **A guard with no caller is the failure mode this campaign has shipped
    /// nine times.** Both gates added for findings 13 and 14 are asserted here
    /// against this file's PRODUCTION text — the tool-dispatch `match` arm and
    /// the disable branch — so a later refactor that keeps the gates compiling
    /// while routing around them fails loudly.
    ///
    /// The behavioural tests above cannot substitute: they call the gate, and a
    /// gate can be called by a test while the dispatch arm no longer reaches it.
    #[test]
    fn both_new_gates_have_production_callers() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/agents/extension_manager_extension.rs");
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("the audit could not read {}: {e}", path.display()));
        assert!(
            src.len() > 10_000,
            "the audit read {} bytes; a truncated read reports the same absence \
             as a removed call site",
            src.len()
        );
        // Production only. `mod tests` in this file sits below an unindented
        // `#[cfg(test)]`, and the assertions in this very function would
        // otherwise satisfy themselves.
        let production = src
            .split("\n#[cfg(test)]")
            .next()
            .expect("split always yields a first element");
        assert!(
            production.len() < src.len(),
            "the production/test split found no `#[cfg(test)]` boundary, so this \
             function is asserting against its own source"
        );
        let calls = |needle: &str| {
            production
                .lines()
                .any(|l| !l.trim_start().starts_with("//") && l.contains(needle))
        };
        assert!(
            calls("self.handle_search_available_extensions(meta.capability)"),
            "finding 13's catalogue filter has no production caller: the \
             `search_available_extensions` dispatch arm no longer threads the \
             admitted capability"
        );
        assert!(
            calls(".assert_extension_manageable(&extension_name, cap)"),
            "finding 14's disable gate has no production caller: \
             `manage_extensions {{disable}}` no longer asks it"
        );

        // The seam: this door's enable gate is the SHARED one, and this file
        // holds no second spelling of it. A behavioural test cannot see the
        // difference — a local re-derivation that happens to agree today passes
        // every assertion above — and agreeing-today is exactly what the two
        // copies did until one of them was reordered.
        assert!(
            calls("crate::privacy::refusal::extension_enable_refusal("),
            "the `manage_extensions` enable door no longer asks the shared enable \
             gate. Its arms (tier, affiliation, operator pin) and their ORDER are \
             the workspace doors' too; a copy here is how the two drifted apart \
             the first time"
        );
        // `extension_entry_is_persisted` is deliberately NOT in this list: the
        // gate takes `persisted` as an argument precisely so it stays pure, and
        // asking that helper is this door's job. What must not come back is an
        // arm of the DECISION.
        //
        // ⚠ **The list is shared, and that is the fix.** This scan used to carry
        // its own literal naming only the idiom THIS file's history produced
        // (`class.tier.is_private()` / `ASK_THE_USER_TO_SWITCH`), while
        // `workspace_extension.rs`'s scan carried a different literal naming
        // only ITS idiom (`resolve_extension(` / `privacy_refusal(` / the
        // affiliation helpers). Two files, two lists, one rule — so a
        // re-derivation written in the other file's vocabulary satisfied both
        // scans and neither reported anything. The guard against duplicating a
        // rule had been duplicated. One const, read by both, in
        // `workspace_extension::tests::ENABLE_GATE_RESPELLINGS`.
        crate::agents::workspace_extension::tests::asserts_no_respellings(
            production,
            "extension_manager_extension.rs",
        );
    }
}

/// F-07: a `biorouter serve` daemon is started with `Stdio::null()` (SD-7), so
/// no proof-of-user digest is ever installed and every approval that sets
/// `requires_user_proof` is refused — for anyone, always. A tool whose only
/// path runs through such an approval must not be advertised there.
#[cfg(test)]
mod proof_gated_roster_tests {
    use super::*;

    fn names(can_ask_a_person: bool) -> Vec<String> {
        ExtensionManagerClient::tools_for(can_ask_a_person)
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect()
    }

    #[test]
    fn the_proof_backed_mutations_are_withheld_when_no_person_is_reachable() {
        let offered = names(false);
        for withheld in [
            INSTALL_EXTENSION_TOOL_NAME,
            DELETE_EXTENSION_PACKAGE_TOOL_NAME,
            // #164. It parks on the same proof-backed approval as the door it
            // sits beside, so a daemon that cannot obtain one must not advertise
            // it either — advertised-and-always-refused is the state SD-8 exists
            // to prevent.
            REMOVE_EXTENSION_TOOL_NAME,
        ] {
            assert!(
                !offered.contains(&withheld.to_string()),
                "{withheld} parks on a proof-backed approval nobody can answer here"
            );
        }
    }

    #[test]
    fn everything_that_can_still_work_stays_offered() {
        let offered = names(false);
        // ⚠ The value of the gate is entirely in what it does NOT withhold. A
        // browser session that can no longer look at its own extensions has
        // been broken, not protected.
        for still_useful in [
            SEARCH_AVAILABLE_EXTENSIONS_TOOL_NAME,
            SEARCH_MARKETPLACE_EXTENSIONS_TOOL_NAME,
            MANAGE_EXTENSIONS_TOOL_NAME,
        ] {
            assert!(
                offered.contains(&still_useful.to_string()),
                "{still_useful} was withheld, and it does not need a person's approval"
            );
        }
    }

    #[test]
    fn a_desktop_daemon_is_offered_the_complete_roster() {
        let offered = names(true);
        for proof_backed in [
            INSTALL_EXTENSION_TOOL_NAME,
            DELETE_EXTENSION_PACKAGE_TOOL_NAME,
            REMOVE_EXTENSION_TOOL_NAME,
        ] {
            assert!(
                offered.contains(&proof_backed.to_string()),
                "{proof_backed}"
            );
        }
        // The gate removes exactly those entries and nothing else.
        assert_eq!(offered.len(), names(false).len() + 3);
    }
}
