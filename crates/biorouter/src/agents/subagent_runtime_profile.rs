use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::extension::ExtensionConfig;
use super::extension_manager::ExtensionOrigin;
use super::Agent;
use crate::session::{EnabledExtensionsState, ExtensionData, ExtensionState, Session};
use crate::workflow::{Response, SubWorkflow};

const EXTENSION_NAME: &str = "subagent_runtime_profile";
const VERSION: &str = "v2";
const LEGACY_VERSION: &str = "v1";
const FORMAT_VERSION: u32 = 2;
const LEGACY_FORMAT_VERSION: u32 = 1;
const MAX_SYSTEM_PROMPT_BYTES: usize = 2 * 1024 * 1024;

fn install_profile_value(extension_data: &mut ExtensionData, value: serde_json::Value) {
    let profile_prefix = format!("{EXTENSION_NAME}.");
    let legacy_extensions_key = format!(
        "{}.{}",
        EnabledExtensionsState::EXTENSION_NAME,
        EnabledExtensionsState::VERSION
    );
    extension_data
        .extension_states
        .retain(|key, _| !key.starts_with(&profile_prefix));
    extension_data
        .extension_states
        .remove(&legacy_extensions_key);
    extension_data.set_extension_state(EXTENSION_NAME, VERSION, value);
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionGrant {
    // References and tool ids only. Never add ExtensionConfig here: stdio envs
    // and HTTP headers may contain legacy resolved credentials.
    name: String,
    kind: String,
    tools: Vec<String>,
}

/// The non-transcript state that makes a spawned child the same agent after its
/// live `Agent` is evicted. This record is daemon-authored session metadata; the
/// human-readable spawn-context message is deliberately not an input to it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubagentRuntimeProfile {
    format_version: u32,
    system_prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response: Option<Response>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sub_workflows: Vec<SubWorkflow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    extension_grants: Vec<ExtensionGrant>,
}

impl SubagentRuntimeProfile {
    pub(crate) fn new(
        system_prompt: String,
        response: Option<Response>,
        sub_workflows: Vec<SubWorkflow>,
        extensions: &[ExtensionConfig],
        tool_names: &[String],
    ) -> Result<Self> {
        let extension_grants = extensions
            .iter()
            .map(|extension| ExtensionGrant::from_runtime(extension, tool_names))
            .collect::<Result<Vec<_>>>()?;
        let profile = Self {
            format_version: FORMAT_VERSION,
            system_prompt,
            response,
            sub_workflows,
            extension_grants,
        };
        profile.validate()?;
        Ok(profile)
    }

    pub(crate) async fn persist(
        &self,
        session_manager: &crate::session::SessionManager,
        session_id: &str,
    ) -> Result<()> {
        self.validate()?;
        let value = serde_json::to_value(self).context("subagent runtime profile serialization")?;
        let written = session_manager
            .update_extension_data(session_id, move |extension_data| {
                install_profile_value(extension_data, value);
                Ok(())
            })
            .await?;
        if written.is_none() {
            bail!("cannot record subagent runtime profile: no session {session_id}");
        }
        Ok(())
    }

    fn load(extension_data: &ExtensionData) -> Result<Option<(Self, bool)>> {
        let prefix = format!("{EXTENSION_NAME}.");
        let versions: Vec<&str> = extension_data
            .extension_states
            .keys()
            .filter_map(|key| key.strip_prefix(&prefix))
            .collect();

        if versions.is_empty() {
            return Ok(None);
        }
        if versions.len() != 1 || !matches!(versions[0], VERSION | LEGACY_VERSION) {
            bail!(
                "unsupported subagent runtime profile version(s): {}",
                versions.join(", ")
            );
        }

        let version = versions[0];
        let current_key = format!("{EXTENSION_NAME}.{version}");
        let value = extension_data
            .extension_states
            .get(&current_key)
            .expect("the sole matching version is the current key");
        let mut profile: Self =
            serde_json::from_value(value.clone()).context("malformed subagent runtime profile")?;
        let legacy = version == LEGACY_VERSION;
        if legacy {
            if profile.format_version != LEGACY_FORMAT_VERSION {
                bail!(
                    "unsupported legacy subagent runtime profile format {}",
                    profile.format_version
                );
            }
            profile.format_version = FORMAT_VERSION;
        }
        profile.validate()?;
        Ok(Some((profile, legacy)))
    }

    fn validate(&self) -> Result<()> {
        if self.format_version != FORMAT_VERSION {
            bail!(
                "unsupported subagent runtime profile format {}",
                self.format_version
            );
        }
        if self.system_prompt.trim().is_empty() {
            bail!("subagent runtime profile has an empty system prompt");
        }
        if self.system_prompt.len() > MAX_SYSTEM_PROMPT_BYTES {
            bail!("subagent runtime profile system prompt is too large");
        }

        if let Some(response) = &self.response {
            let schema = response
                .json_schema
                .as_ref()
                .ok_or_else(|| anyhow!("subagent response is missing its JSON schema"))?;
            let object = schema
                .as_object()
                .ok_or_else(|| anyhow!("subagent response schema must be an object"))?;
            if object.is_empty() {
                bail!("subagent response schema must not be empty");
            }
            jsonschema::meta::validate(schema)
                .map_err(|error| anyhow!("invalid subagent response schema: {error:?}"))?;
        }

        let mut extension_names = HashSet::new();
        for grant in &self.extension_grants {
            let name = &grant.name;
            if name.trim().is_empty() {
                bail!("subagent runtime profile contains an unnamed extension");
            }
            let normalized = super::normalize(name);
            if normalized == "workspace" {
                bail!("subagent runtime profile cannot grant workspace control");
            }
            if !extension_names.insert(normalized) {
                bail!("subagent runtime profile repeats extension '{name}'");
            }
            if grant.kind == "sse" {
                bail!("subagent runtime profile cannot restore legacy SSE extension '{name}'");
            }
            if !matches!(
                grant.kind.as_str(),
                "stdio" | "builtin" | "platform" | "streamable_http" | "frontend" | "inline_python"
            ) {
                bail!("subagent runtime profile contains an unknown extension kind");
            }
            let mut tools = HashSet::new();
            if grant
                .tools
                .iter()
                .any(|tool| tool.trim().is_empty() || !tools.insert(tool))
            {
                bail!("subagent runtime profile contains invalid duplicate tool grants");
            }
        }

        let mut subworkflow_names = HashSet::new();
        for subworkflow in &self.sub_workflows {
            if subworkflow.name.trim().is_empty() || subworkflow.path.trim().is_empty() {
                bail!("subagent runtime profile contains an incomplete subworkflow");
            }
            if !subworkflow_names.insert(subworkflow.name.clone()) {
                bail!(
                    "subagent runtime profile repeats subworkflow '{}'",
                    subworkflow.name
                );
            }
        }
        Ok(())
    }
}

/// A secret-free projection for the child-tab extension badge. The runtime
/// profile is the single source of truth for new children, so the UI view and
/// the restoration clamp commit in one JSON value rather than two independently
/// writable session keys.
pub fn persisted_subagent_extension_projection(
    extension_data: &ExtensionData,
) -> Result<Option<Vec<ExtensionConfig>>> {
    let Some((profile, _)) = SubagentRuntimeProfile::load(extension_data)? else {
        return Ok(None);
    };
    profile
        .extension_grants
        .iter()
        .map(ExtensionGrant::ui_projection)
        .collect::<Result<Vec<_>>>()
        // A grant written under an extension id renamed since shows its
        // current id, as every other config read does.
        .map(crate::config::extension_renames::project_configs)
        .map(Some)
}

impl ExtensionGrant {
    fn from_runtime(extension: &ExtensionConfig, tool_names: &[String]) -> Result<Self> {
        let name = extension.name();
        let normalized = super::normalize(&name);
        if normalized == "workspace" {
            bail!("subagent runtime profile cannot grant workspace control");
        }
        if matches!(extension, ExtensionConfig::Sse { .. }) {
            bail!("subagent cannot persist non-restorable legacy SSE extension '{name}'");
        }
        let prefix = format!("{normalized}__");
        let mut tools: Vec<String> = tool_names
            .iter()
            .filter_map(|tool| tool.strip_prefix(&prefix).map(str::to_string))
            .collect();
        tools.sort();
        tools.dedup();
        Ok(Self {
            name,
            kind: extension_kind(extension).to_string(),
            tools,
        })
    }

    fn ui_projection(&self) -> Result<ExtensionConfig> {
        let name = self.name.clone();
        let available_tools = self.tools.clone();
        let config = match self.kind.as_str() {
            "sse" => bail!("subagent cannot project non-restorable legacy SSE extension '{name}'"),
            "stdio" => ExtensionConfig::Stdio {
                name,
                description: String::new(),
                cmd: String::new(),
                args: Vec::new(),
                envs: Default::default(),
                env_keys: Vec::new(),
                timeout: None,
                bundled: None,
                available_tools,
            },
            "builtin" => ExtensionConfig::Builtin {
                name,
                description: String::new(),
                display_name: None,
                timeout: None,
                bundled: None,
                available_tools,
            },
            "platform" => ExtensionConfig::Platform {
                name,
                description: String::new(),
                bundled: None,
                available_tools,
            },
            "streamable_http" => ExtensionConfig::StreamableHttp {
                name,
                description: String::new(),
                uri: String::new(),
                envs: Default::default(),
                env_keys: Vec::new(),
                headers: Default::default(),
                timeout: None,
                bundled: None,
                available_tools,
            },
            "frontend" => ExtensionConfig::Frontend {
                name,
                description: String::new(),
                tools: Vec::new(),
                instructions: None,
                bundled: None,
                available_tools,
            },
            "inline_python" => ExtensionConfig::InlinePython {
                name,
                description: String::new(),
                code: String::new(),
                timeout: None,
                dependencies: None,
                available_tools,
            },
            kind => bail!("subagent runtime profile contains an unknown extension kind '{kind}'"),
        };
        Ok(config)
    }
}

/// The manager key an extension named `name` answers to now. A grant written
/// before its extension was renamed names it by its old id, while the config
/// the catalog returns for it (and the one a live manager holds) carries the
/// current one (`config::extension_renames`), so a cold child's grant check
/// compares both by this key.
fn current_extension_key(name: &str) -> String {
    crate::config::extension_renames::current_key(&super::normalize(name)).to_string()
}

fn extension_kind(extension: &ExtensionConfig) -> &'static str {
    match extension {
        ExtensionConfig::Sse { .. } => "sse",
        ExtensionConfig::Stdio { .. } => "stdio",
        ExtensionConfig::Builtin { .. } => "builtin",
        ExtensionConfig::Platform { .. } => "platform",
        ExtensionConfig::StreamableHttp { .. } => "streamable_http",
        ExtensionConfig::Frontend { .. } => "frontend",
        ExtensionConfig::InlinePython { .. } => "inline_python",
    }
}

/// Clamp the configuration freshly resolved from the local catalog to the tool
/// names captured at spawn. An empty grant loads no client at all: in this
/// config format an empty `available_tools` means "all", so writing an empty
/// list back would silently widen when a catalog adds its first tool.
fn restricted_config(
    mut config: ExtensionConfig,
    grant: &ExtensionGrant,
) -> Result<Option<ExtensionConfig>> {
    if extension_kind(&config) != grant.kind {
        bail!(
            "subagent extension '{}' changed kind from '{}' to '{}'",
            grant.name,
            grant.kind,
            extension_kind(&config)
        );
    }
    if matches!(&config, ExtensionConfig::Sse { .. }) {
        bail!(
            "subagent cannot restore non-restorable legacy SSE extension '{}'",
            grant.name
        );
    }
    if grant.tools.is_empty() {
        return Ok(None);
    }
    match &mut config {
        ExtensionConfig::Sse { .. } => {
            bail!(
                "subagent cannot restore non-restorable legacy SSE extension '{}'",
                grant.name
            )
        }
        ExtensionConfig::Stdio {
            available_tools, ..
        }
        | ExtensionConfig::Builtin {
            available_tools, ..
        }
        | ExtensionConfig::Platform {
            available_tools, ..
        }
        | ExtensionConfig::StreamableHttp {
            available_tools, ..
        }
        | ExtensionConfig::Frontend {
            available_tools, ..
        }
        | ExtensionConfig::InlinePython {
            available_tools, ..
        } => *available_tools = grant.tools.clone(),
    }
    Ok(Some(config))
}

impl Agent {
    /// Restore a cold child's daemon-authored runtime. `Ok(false)` is a legacy
    /// child with no profile; `Ok(true)` means the profile is installed or the
    /// live agent already owns that exact runtime.
    pub async fn restore_subagent_runtime_profile(
        self: &Arc<Self>,
        session: &Session,
    ) -> Result<bool> {
        if session.session_type != crate::session::SessionType::SubAgent {
            return Ok(false);
        }

        if self
            .subagent_runtime_sessions
            .lock()
            .await
            .contains(&session.id)
        {
            return Ok(true);
        }

        let Some((profile, legacy_profile)) =
            SubagentRuntimeProfile::load(&session.extension_data)?
        else {
            return Ok(false);
        };

        self.extension_manager
            .set_working_dir(session.working_dir.clone())
            .await;

        let allowed: HashSet<String> = profile
            .extension_grants
            .iter()
            .map(|grant| current_extension_key(&grant.name))
            .collect();

        let mut expected = Vec::new();
        for grant in &profile.extension_grants {
            // `get_extension_by_name` finds the current entry for an old id.
            let config = crate::config::get_extension_by_name(&grant.name).ok_or_else(|| {
                anyhow!(
                    "subagent extension '{}' is no longer present in the local catalog",
                    grant.name
                )
            })?;
            if let Some(config) = restricted_config(config, grant)? {
                expected.push(config);
            }
        }

        for existing in self.persistable_extension_configs().await {
            let existing_name = existing.name();
            if !allowed.contains(&current_extension_key(&existing_name)) {
                bail!(
                    "cold subagent already holds extension '{}' outside its persisted grant",
                    existing_name
                );
            }
            let expected_config = expected
                .iter()
                .find(|grant| super::normalize(&grant.name()) == super::normalize(&existing_name))
                .ok_or_else(|| {
                    anyhow!(
                        "cold subagent already holds extension '{}' with no tool grant",
                        existing_name
                    )
                })?;
            if &existing != expected_config {
                bail!(
                    "cold subagent extension '{}' does not match its persisted grant",
                    existing_name
                );
            }
        }

        for extension in &expected {
            let normalized = super::normalize(&extension.name());
            if self.extension_manager.extension_origin(&normalized).await
                == Some(ExtensionOrigin::Explicit)
            {
                continue;
            }
            self.add_extension(extension.clone())
                .await
                .with_context(|| {
                    format!(
                        "failed to restore subagent extension '{}'",
                        extension.name()
                    )
                })?;
        }

        let restored = self.persistable_extension_configs().await;
        if restored.len() != expected.len()
            || expected.iter().any(|grant| !restored.contains(grant))
        {
            bail!("cold subagent extension set does not match its persisted grant");
        }

        if legacy_profile {
            profile
                .persist(&self.config.session_manager, &session.id)
                .await
                .context("failed to migrate legacy subagent runtime metadata")?;
        }

        self.apply_workflow_components(
            Some(profile.sub_workflows.clone()),
            profile.response.clone(),
            true,
        )
        .await;
        self.override_system_prompt(profile.system_prompt).await;
        self.mark_subagent_runtime_installed(&session.id).await;
        Ok(true)
    }

    pub(crate) async fn mark_subagent_runtime_installed(&self, session_id: &str) {
        self.subagent_runtime_sessions
            .lock()
            .await
            .insert(session_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use super::*;
    use crate::agents::AgentConfig;
    use crate::config::{BioRouterMode, PermissionManager};
    use crate::session::{SessionManager, SessionType};

    fn valid_profile() -> SubagentRuntimeProfile {
        SubagentRuntimeProfile::new("persisted child prompt".into(), None, Vec::new(), &[], &[])
            .unwrap()
    }

    #[test]
    fn malformed_and_unknown_profiles_are_rejected() {
        let mut malformed = ExtensionData::new();
        malformed.set_extension_state(EXTENSION_NAME, VERSION, json!({"format_version": 1}));
        assert!(SubagentRuntimeProfile::load(&malformed).is_err());

        let mut unknown = ExtensionData::new();
        unknown.set_extension_state(EXTENSION_NAME, "v999", json!({}));
        assert!(SubagentRuntimeProfile::load(&unknown).is_err());

        let mut mixed = ExtensionData::new();
        mixed.set_extension_state(
            EXTENSION_NAME,
            VERSION,
            serde_json::to_value(valid_profile()).unwrap(),
        );
        mixed.set_extension_state(EXTENSION_NAME, LEGACY_VERSION, json!({}));
        assert!(SubagentRuntimeProfile::load(&mixed).is_err());
    }

    #[test]
    fn workspace_control_cannot_be_persisted_as_a_child_grant() {
        let extension = ExtensionConfig::Platform {
            name: "workspace".into(),
            description: "Workspace".into(),
            bundled: Some(true),
            available_tools: Vec::new(),
        };
        let result =
            SubagentRuntimeProfile::new("prompt".into(), None, Vec::new(), &[extension], &[]);
        assert!(result.is_err());
    }

    #[test]
    fn spawn_profile_refuses_a_non_restorable_sse_grant_without_leaking_its_uri() {
        let secret = "SSE_URI_SECRET_MUST_NOT_LEAK";
        let extension = ExtensionConfig::Sse {
            name: "legacy_sse".into(),
            description: "Legacy SSE".into(),
            uri: Some(format!(
                "https://user:{secret}@example.invalid/events?token={secret}"
            )),
        };

        let error = SubagentRuntimeProfile::new(
            "prompt".into(),
            None,
            Vec::new(),
            &[extension],
            &["legacy_sse__search".into()],
        )
        .unwrap_err();

        assert!(error.to_string().contains("non-restorable legacy SSE"));
        assert!(error.to_string().contains("legacy_sse"));
        assert!(!error.to_string().contains(secret));
        assert!(!error.to_string().contains("example.invalid"));

        let empty_grant = ExtensionGrant {
            name: "legacy_sse".into(),
            kind: "sse".into(),
            tools: Vec::new(),
        };
        let restore_error = restricted_config(
            ExtensionConfig::Sse {
                name: "legacy_sse".into(),
                description: String::new(),
                uri: Some(format!("https://example.invalid/?token={secret}")),
            },
            &empty_grant,
        )
        .unwrap_err();
        assert!(restore_error
            .to_string()
            .contains("non-restorable legacy SSE"));
        assert!(!restore_error.to_string().contains(secret));
    }

    #[tokio::test]
    async fn cold_restore_rejects_a_legacy_sse_profile_without_silent_grant_loss() {
        let temp = tempfile::TempDir::new().unwrap();
        let session_manager = Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = session_manager
            .create_session(
                temp.path().to_path_buf(),
                "legacy SSE child".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let secret = "LEGACY_SSE_SECRET_MUST_STAY_OUT_OF_ERRORS";
        let sse = ExtensionConfig::Sse {
            name: "legacy_sse".into(),
            description: "Legacy SSE".into(),
            uri: Some(format!("https://example.invalid/events?token={secret}")),
        };
        let mut extension_data = ExtensionData::new();
        extension_data.set_extension_state(
            EXTENSION_NAME,
            LEGACY_VERSION,
            json!({
                "format_version": LEGACY_FORMAT_VERSION,
                "system_prompt": "legacy SSE prompt",
                "extension_grants": [{
                    "name": "legacy_sse",
                    "kind": "sse",
                    "tools": ["search"]
                }]
            }),
        );
        EnabledExtensionsState::new(vec![sse])
            .to_extension_data(&mut extension_data)
            .unwrap();
        session_manager
            .update(&child.id)
            .extension_data(extension_data)
            .apply()
            .await
            .unwrap();
        let child = session_manager.get_session(&child.id, false).await.unwrap();
        let agent = Arc::new(Agent::with_config(AgentConfig::new(
            session_manager.clone(),
            PermissionManager::instance(),
            None,
            BioRouterMode::Auto,
        )));

        let error = agent
            .restore_subagent_runtime_profile(&child)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("cannot restore legacy SSE"));
        assert!(error.to_string().contains("legacy_sse"));
        assert!(!error.to_string().contains(secret));
        assert!(agent.persistable_extension_configs().await.is_empty());
        assert!(!agent
            .subagent_runtime_sessions
            .lock()
            .await
            .contains(&child.id));

        let unchanged = session_manager.get_session(&child.id, false).await.unwrap();
        assert!(unchanged
            .extension_data
            .get_extension_state(EXTENSION_NAME, LEGACY_VERSION)
            .is_some());
        assert!(unchanged
            .extension_data
            .get_extension_state(EXTENSION_NAME, VERSION)
            .is_none());
        assert!(unchanged
            .extension_data
            .get_extension_state(
                EnabledExtensionsState::EXTENSION_NAME,
                EnabledExtensionsState::VERSION
            )
            .is_some());
    }

    #[test]
    fn persisted_profiles_do_not_copy_extension_auth_material() {
        let secret = "RESOLVED_SECRET_MUST_NOT_BE_PERSISTED";
        let extension = ExtensionConfig::StreamableHttp {
            name: "remote_kb".into(),
            description: "Remote KB".into(),
            uri: "https://example.invalid/mcp".into(),
            envs: crate::agents::extension::Envs::new(HashMap::from([(
                "API_TOKEN".into(),
                secret.into(),
            )])),
            env_keys: vec!["API_TOKEN".into()],
            headers: HashMap::from([("Authorization".into(), format!("Bearer {secret}"))]),
            timeout: Some(30),
            bundled: None,
            available_tools: Vec::new(),
        };
        let subworkflow = SubWorkflow {
            name: "safe_reference".into(),
            path: "safe.yaml".into(),
            values: Some(HashMap::from([(
                "token".into(),
                "{{vault:API_TOKEN}}".into(),
            )])),
            sequential_when_repeated: false,
            description: None,
        };
        let profile = SubagentRuntimeProfile::new(
            "safe prompt".into(),
            None,
            vec![subworkflow],
            &[extension],
            &["remote_kb__search".into()],
        )
        .unwrap();

        let persisted = serde_json::to_string(&profile).unwrap();
        assert!(!persisted.contains(secret));
        assert!(!persisted.contains("Authorization"));
        assert!(!persisted.contains("https://example.invalid"));
        assert!(persisted.contains("remote_kb"));
        assert!(persisted.contains("{{vault:API_TOKEN}}"));
    }

    #[tokio::test]
    async fn the_entire_persisted_child_row_excludes_legacy_env_and_header_secrets() {
        let temp = tempfile::TempDir::new().unwrap();
        let session_manager = SessionManager::new(temp.path().to_path_buf());
        let child = session_manager
            .create_session(
                temp.path().to_path_buf(),
                "secret-free child".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let secret = "LEGACY_ENV_AND_HEADER_SECRET";
        let extension = ExtensionConfig::StreamableHttp {
            name: "remote_kb".into(),
            description: "Remote KB".into(),
            uri: format!("https://user:{secret}@example.invalid/mcp?token={secret}"),
            envs: crate::agents::extension::Envs::new(HashMap::from([(
                "API_TOKEN".into(),
                secret.into(),
            )])),
            env_keys: vec!["API_TOKEN".into()],
            headers: HashMap::from([("Authorization".into(), format!("Bearer {secret}"))]),
            timeout: Some(30),
            bundled: None,
            available_tools: Vec::new(),
        };
        let mut legacy = ExtensionData::new();
        EnabledExtensionsState::new(vec![extension.clone()])
            .to_extension_data(&mut legacy)
            .unwrap();
        session_manager
            .update(&child.id)
            .extension_data(legacy)
            .apply()
            .await
            .unwrap();

        let profile = SubagentRuntimeProfile::new(
            "safe prompt".into(),
            None,
            Vec::new(),
            &[extension],
            &["remote_kb__search".into()],
        )
        .unwrap();
        profile.persist(&session_manager, &child.id).await.unwrap();

        let stored = session_manager.get_session(&child.id, false).await.unwrap();
        let persisted = serde_json::to_string(&stored.extension_data).unwrap();
        assert!(!persisted.contains(secret));
        assert!(!persisted.contains("Authorization"));
        assert!(!persisted.contains("example.invalid"));
        assert!(!persisted.contains("\"envs\""));
        assert!(!persisted.contains("\"headers\""));
        assert!(stored
            .extension_data
            .get_extension_state(
                EnabledExtensionsState::EXTENSION_NAME,
                EnabledExtensionsState::VERSION
            )
            .is_none());
        let projection = persisted_subagent_extension_projection(&stored.extension_data)
            .unwrap()
            .unwrap();
        assert_eq!(projection.len(), 1);
        assert_eq!(projection[0].name(), "remote_kb");
        assert!(!serde_json::to_string(&projection).unwrap().contains(secret));
    }

    /// A child spawned while gptr Agent was still `gptragent` shows it under
    /// the current id: the badge projection is a config read like any other.
    #[test]
    fn a_profile_granting_an_old_extension_id_projects_the_current_one() {
        let extension = ExtensionConfig::stdio("gptragent", "uv", "gptr Agent", 30_u64);
        let profile = SubagentRuntimeProfile::new(
            "safe prompt".into(),
            None,
            Vec::new(),
            &[extension],
            &["gptragent__r".into()],
        )
        .unwrap();
        let mut data = ExtensionData::new();
        install_profile_value(&mut data, serde_json::to_value(&profile).unwrap());
        let projection = persisted_subagent_extension_projection(&data)
            .unwrap()
            .unwrap();
        assert_eq!(projection.len(), 1);
        assert_eq!(projection[0].key(), "gptr");
        assert!(projection[0].is_tool_available("r"));
        assert!(!projection[0].is_tool_available("inspect"));
    }

    #[tokio::test]
    async fn a_failed_atomic_profile_write_cannot_leave_half_migrated_authority() {
        let temp = tempfile::TempDir::new().unwrap();
        let session_manager = SessionManager::new(temp.path().to_path_buf());
        let child = session_manager
            .create_session(
                temp.path().to_path_buf(),
                "atomic child".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let legacy = ExtensionConfig::Platform {
            name: "todo".into(),
            description: "Todo".into(),
            bundled: Some(true),
            available_tools: Vec::new(),
        };
        let mut before = ExtensionData::new();
        EnabledExtensionsState::new(vec![legacy])
            .to_extension_data(&mut before)
            .unwrap();
        before.set_extension_state("unrelated", "v1", json!({"kept": true}));
        session_manager
            .update(&child.id)
            .extension_data(before.clone())
            .apply()
            .await
            .unwrap();
        let value = serde_json::to_value(valid_profile()).unwrap();

        let error = session_manager
            .update_extension_data(&child.id, move |extension_data| -> Result<()> {
                install_profile_value(extension_data, value);
                bail!("injected failure after replacement was staged")
            })
            .await
            .unwrap_err();
        assert!(error.to_string().contains("injected failure"));

        let after = session_manager.get_session(&child.id, false).await.unwrap();
        assert_eq!(
            serde_json::to_value(after.extension_data).unwrap(),
            serde_json::to_value(before).unwrap(),
            "the transaction must preserve the complete old object on failure"
        );
    }

    #[tokio::test]
    async fn session_export_redacts_legacy_extension_auth_material() {
        let temp = tempfile::TempDir::new().unwrap();
        let session_manager = SessionManager::new(temp.path().to_path_buf());
        let child = session_manager
            .create_session(
                temp.path().to_path_buf(),
                "legacy export".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let secret = "SESSION_EXPORT_SECRET";
        let extension = ExtensionConfig::StreamableHttp {
            name: "legacy_remote".into(),
            description: "Legacy remote".into(),
            uri: format!("https://example.invalid/?token={secret}"),
            envs: crate::agents::extension::Envs::new(HashMap::from([(
                "TOKEN".into(),
                secret.into(),
            )])),
            env_keys: vec!["TOKEN".into()],
            headers: HashMap::from([("Authorization".into(), secret.into())]),
            timeout: None,
            bundled: None,
            available_tools: vec!["search".into()],
        };
        let mut extension_data = ExtensionData::new();
        EnabledExtensionsState::new(vec![extension])
            .to_extension_data(&mut extension_data)
            .unwrap();
        session_manager
            .update(&child.id)
            .extension_data(extension_data)
            .apply()
            .await
            .unwrap();

        let exported = session_manager.export_session(&child.id).await.unwrap();
        assert!(!exported.contains(secret));
        assert!(!exported.contains("Authorization"));
        assert!(!exported.contains("example.invalid"));
        assert!(exported.contains("legacy_remote"));
        assert!(exported.contains("TOKEN"));
    }

    #[tokio::test]
    async fn a_new_child_cannot_later_reintroduce_full_extension_configs() {
        let temp = tempfile::TempDir::new().unwrap();
        let session_manager = Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = session_manager
            .create_session(
                temp.path().to_path_buf(),
                "immutable child grants".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let agent = Agent::with_config(AgentConfig::new(
            session_manager.clone(),
            PermissionManager::instance(),
            None,
            BioRouterMode::Auto,
        ));
        agent
            .add_extension(ExtensionConfig::Platform {
                name: "todo".into(),
                description: "Todo".into(),
                bundled: Some(true),
                available_tools: vec!["todo_write".into()],
            })
            .await
            .unwrap();

        let error = agent.persist_extension_state(&child.id).await.unwrap_err();
        assert!(error.to_string().contains("immutable runtime-profile"));
        let stored = session_manager.get_session(&child.id, false).await.unwrap();
        assert!(stored
            .extension_data
            .get_extension_state(
                EnabledExtensionsState::EXTENSION_NAME,
                EnabledExtensionsState::VERSION
            )
            .is_none());
    }

    #[tokio::test]
    async fn a_legacy_profile_is_sanitized_on_successful_cold_touch() {
        let temp = tempfile::TempDir::new().unwrap();
        let session_manager = Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = session_manager
            .create_session(
                temp.path().to_path_buf(),
                "legacy profile child".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let todo = ExtensionConfig::Platform {
            name: "todo".into(),
            description: "Todo".into(),
            bundled: Some(true),
            available_tools: Vec::new(),
        };
        let profile = SubagentRuntimeProfile::new(
            "legacy prompt".into(),
            None,
            Vec::new(),
            std::slice::from_ref(&todo),
            &["todo__todo_write".into()],
        )
        .unwrap();
        let mut legacy_profile = serde_json::to_value(profile).unwrap();
        legacy_profile["format_version"] = json!(LEGACY_FORMAT_VERSION);
        let mut extension_data = ExtensionData::new();
        extension_data.set_extension_state(EXTENSION_NAME, LEGACY_VERSION, legacy_profile);
        EnabledExtensionsState::new(vec![todo])
            .to_extension_data(&mut extension_data)
            .unwrap();
        session_manager
            .update(&child.id)
            .extension_data(extension_data)
            .apply()
            .await
            .unwrap();
        let child = session_manager.get_session(&child.id, false).await.unwrap();
        let agent = Arc::new(Agent::with_config(AgentConfig::new(
            session_manager.clone(),
            PermissionManager::instance(),
            None,
            BioRouterMode::Auto,
        )));

        assert!(agent
            .restore_subagent_runtime_profile(&child)
            .await
            .unwrap());
        let migrated = session_manager.get_session(&child.id, false).await.unwrap();
        assert!(migrated
            .extension_data
            .get_extension_state(EXTENSION_NAME, VERSION)
            .is_some());
        assert!(migrated
            .extension_data
            .get_extension_state(EXTENSION_NAME, LEGACY_VERSION)
            .is_none());
        assert!(migrated
            .extension_data
            .get_extension_state(
                EnabledExtensionsState::EXTENSION_NAME,
                EnabledExtensionsState::VERSION
            )
            .is_none());
    }

    #[test]
    fn a_catalog_change_cannot_widen_a_persisted_tool_grant() {
        let extension = ExtensionConfig::Platform {
            name: "todo".into(),
            description: "Todo".into(),
            bundled: Some(true),
            // Empty means "all", including tools a future catalog may add.
            available_tools: Vec::new(),
        };
        let grant = ExtensionGrant::from_runtime(
            &extension,
            &["todo__todo_write".into(), "unrelated__new_tool".into()],
        )
        .unwrap();
        let restricted = restricted_config(extension, &grant).unwrap().unwrap();
        let ExtensionConfig::Platform {
            available_tools, ..
        } = restricted
        else {
            unreachable!()
        };
        assert_eq!(available_tools, vec!["todo_write"]);
    }

    #[tokio::test]
    async fn a_cold_child_recovers_its_prompt_output_contract_subworkflows_and_grants() {
        let temp = tempfile::TempDir::new().unwrap();
        let session_manager = Arc::new(SessionManager::new(temp.path().to_path_buf()));
        let child = session_manager
            .create_session(
                temp.path().to_path_buf(),
                "profile child".into(),
                SessionType::SubAgent,
            )
            .await
            .unwrap();
        let response = Response {
            json_schema: Some(json!({
                "type": "object",
                "properties": {"answer": {"type": "string"}},
                "required": ["answer"]
            })),
        };
        let subworkflow = SubWorkflow {
            name: "persisted_reviewer".into(),
            path: "reviewer.yaml".into(),
            values: None,
            sequential_when_repeated: true,
            description: Some("Persisted reviewer".into()),
        };
        let todo = ExtensionConfig::Platform {
            name: "todo".into(),
            description: "Todo".into(),
            bundled: Some(true),
            available_tools: Vec::new(),
        };
        let prompt = "COLD_PROFILE_PROMPT_SENTINEL";
        let profile = SubagentRuntimeProfile::new(
            prompt.into(),
            Some(response.clone()),
            vec![subworkflow.clone()],
            std::slice::from_ref(&todo),
            &["todo__todo_write".into()],
        )
        .unwrap();
        let enabled = EnabledExtensionsState::new(vec![todo]);
        let enabled_value = enabled.to_value().unwrap();
        session_manager
            .update_extension_state(
                &child.id,
                EnabledExtensionsState::EXTENSION_NAME,
                EnabledExtensionsState::VERSION,
                move |_| Ok(enabled_value),
            )
            .await
            .unwrap();
        profile.persist(&session_manager, &child.id).await.unwrap();
        let child = session_manager.get_session(&child.id, false).await.unwrap();

        let agent = Arc::new(Agent::with_config(AgentConfig::new(
            session_manager,
            PermissionManager::instance(),
            None,
            BioRouterMode::Auto,
        )));
        assert!(agent
            .restore_subagent_runtime_profile(&child)
            .await
            .unwrap());

        let system_prompt = agent.prompt_manager.lock().await.builder().build();
        assert!(system_prompt.contains(prompt));
        let final_output = agent.final_output_tool.lock().await;
        assert_eq!(
            final_output
                .as_ref()
                .and_then(|tool| tool.response.json_schema.as_ref()),
            response.json_schema.as_ref()
        );
        drop(final_output);
        assert_eq!(
            agent
                .sub_workflows
                .lock()
                .await
                .get(&subworkflow.name)
                .map(|workflow| workflow.path.as_str()),
            Some(subworkflow.path.as_str())
        );

        let tool_names: Vec<String> = agent
            .list_tools(&child.id, None)
            .await
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        assert!(tool_names.iter().any(|name| name == "todo__todo_write"));
        assert!(tool_names
            .iter()
            .any(|name| name == super::super::final_output_tool::FINAL_OUTPUT_TOOL_NAME));
        assert!(tool_names
            .iter()
            .all(|name| !name.starts_with("workspace__")));
    }

    #[tokio::test]
    async fn an_installed_live_child_is_not_reconfigured_from_its_row() {
        let agent = Arc::new(Agent::new());
        let mut session = Session {
            id: "live-child".into(),
            session_type: SessionType::SubAgent,
            ..Session::default()
        };
        agent.override_system_prompt("LIVE_PROMPT".into()).await;
        agent.mark_subagent_runtime_installed(&session.id).await;
        session.extension_data.set_extension_state(
            EXTENSION_NAME,
            "v999",
            json!({"forged": "cold state"}),
        );

        assert!(agent
            .restore_subagent_runtime_profile(&session)
            .await
            .unwrap());
        assert!(agent
            .prompt_manager
            .lock()
            .await
            .builder()
            .build()
            .contains("LIVE_PROMPT"));
    }

    #[tokio::test]
    async fn a_cold_child_rejects_unknown_profiles_and_preexisting_extra_grants() {
        let agent = Arc::new(Agent::new());
        let mut unknown = Session {
            id: "unknown-profile-child".into(),
            session_type: SessionType::SubAgent,
            ..Session::default()
        };
        unknown.extension_data.set_extension_state(
            EXTENSION_NAME,
            "v999",
            json!({"system_prompt": "do not install"}),
        );
        assert!(agent
            .restore_subagent_runtime_profile(&unknown)
            .await
            .is_err());
        assert!(agent.sub_workflows.lock().await.is_empty());
        assert!(agent.final_output_tool.lock().await.is_none());

        let agent = Arc::new(Agent::new());
        agent
            .add_extension(ExtensionConfig::Platform {
                name: "todo".into(),
                description: "Todo".into(),
                bundled: Some(true),
                available_tools: Vec::new(),
            })
            .await
            .unwrap();
        let mut narrowed = Session {
            id: "narrow-profile-child".into(),
            session_type: SessionType::SubAgent,
            ..Session::default()
        };
        narrowed.extension_data.set_extension_state(
            EXTENSION_NAME,
            VERSION,
            serde_json::to_value(valid_profile()).unwrap(),
        );
        let error = agent
            .restore_subagent_runtime_profile(&narrowed)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("outside its persisted grant"));
    }
}
