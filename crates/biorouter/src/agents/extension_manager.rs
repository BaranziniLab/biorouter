use anyhow::Result;
use axum::http::{HeaderMap, HeaderName};
use chrono::{DateTime, Utc};
use futures::stream::{FuturesUnordered, StreamExt};
use futures::{future, FutureExt};
use rand::{distributions::Alphanumeric, Rng};
use rmcp::service::{ClientInitializeError, ServiceError};
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, StreamableHttpClientTransportConfig, StreamableHttpError,
};
use rmcp::transport::{
    ConfigureCommandExt, DynamicTransportError, StreamableHttpClientTransport, TokioChildProcess,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::{tempdir, TempDir};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

use super::extension::{
    ExtensionClassification, ExtensionConfig, ExtensionError, ExtensionInfo, ExtensionResult,
    PlatformExtensionContext, ToolInfo, PLATFORM_EXTENSIONS,
};
use super::tool_execution::ToolCallResult;
use super::types::SharedProvider;
use crate::agents::extension::{Envs, ProcessExit};
use crate::agents::extension_malware_check;
use crate::agents::mcp_client::{McpClient, McpClientBox, McpClientTrait, McpMeta};
use crate::agents::mcp_pool::{PooledEntry, SharedMcpPool};
use crate::config::search_path::SearchPaths;
use crate::config::{get_all_extensions, Config};
use crate::oauth::oauth_flow;
use crate::prompt_template;
use crate::subprocess::configure_command_no_window;
use rmcp::model::{
    CallToolRequestParams, Content, ErrorCode, ErrorData, GetPromptResult, Prompt,
    ResourceContents, ServerInfo, Tool,
};
use rmcp::transport::auth::AuthClient;
use schemars::_private::NoSerialize;
use serde_json::Value;

/// Tags that wrap every MOIM (`collect_moim`) `<info-msg>` block. Shared so the
/// per-turn dedup in `moim.rs` recognises exactly what this function emits.
pub const MOIM_OPEN_TAG: &str = "<info-msg>";
pub const MOIM_CLOSE_TAG: &str = "</info-msg>";

pub(crate) fn capability_management_error(name: &str) -> ErrorData {
    ErrorData::new(
        ErrorCode::INVALID_REQUEST,
        format!(
            "`{name}` is a built-in Biorouter capability, not an installed extension, and cannot be enabled or disabled through Extension Manager"
        ),
        None,
    )
}

pub(crate) fn capability_management_refusal(config: &ExtensionConfig) -> Option<ErrorData> {
    config
        .is_capability()
        .then(|| capability_management_error(&config.name()))
}

/// How an extension entry came to be loaded.
///
/// BR-71 decision 21: the agent loads `workspace` for ITSELF whenever a session
/// may delegate, with a child-supervision-only `available_tools`. That grant is
/// a derived per-turn consequence of `subagents_enabled`, not a user decision, and four
/// separate consumers have to tell the two apart — session persistence, the
/// `TaskConfig` handed to a child agent, a generated workflow's extension list,
/// and an explicit enable that arrives later and must replace it.
///
/// The distinction therefore lives HERE, on the entry, under the same mutex as
/// the config. Kept anywhere else it is a second source of truth that is
/// written in a different critical section from the load it describes, so a
/// reader can observe an injection that is not yet marked (and persist it), and
/// a late injection can claim provenance for the explicit entry that beat it
/// (and silently stop persisting the user's own choice).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionOrigin {
    /// A user decision: Settings, `biorouter configure`, `/agent/add_extension`,
    /// the session's own stored configuration, or the model's
    /// `manage_extensions`.
    Explicit,
    /// Loaded by the agent itself as a consequence of session state.
    AutoInjected,
}

struct Extension {
    pub config: ExtensionConfig,

    client: McpClientBox,
    server_info: Option<ServerInfo>,
    _temp_dir: Option<tempfile::TempDir>,
    /// See [`ExtensionOrigin`]. `AutoInjected` entries are excluded from
    /// [`ExtensionManager::get_extension_configs`], so they are never persisted,
    /// replayed, or propagated — the same treatment `inprocess` gets, for the
    /// same reason.
    origin: ExtensionOrigin,
    /// True for per-app in-process servers injected via `add_inprocess_server`.
    /// Their `config` is a synthetic name-only marker that is NOT spawnable from
    /// any registry, so they are excluded from `get_extension_configs` (never
    /// persisted/replayed/propagated) and re-injected per connect instead.
    inprocess: bool,
    /// Set only by `add_extension_with_origin` after resolving the config
    /// through a shipped Builtin/Platform registry. A lookalike injected by
    /// `add_client` or `add_inprocess_server` is never trusted by the coding
    /// agent bridge, even if it uses the same name and config variant.
    trusted_bundled: bool,
    /// Keeps a shared (pooled) process alive while this extension references it
    /// (BR-54). When the last extension across all sessions drops this `Arc`, the
    /// pool's `Weak` dies and the child process is reaped. `None` for unpooled and
    /// in-process servers, which own their client directly via `_temp_dir`/`client`.
    _pooled: Option<Arc<PooledEntry>>,
    //
    // ⚠ **There is deliberately no `tier` field here** (issue #56, Task 43,
    // DR-23). One was stamped at admission from `classify_extension` and read by
    // Gate C; it is gone, and re-adding it would reintroduce the bug DR-23
    // closed. The tier is now RE-DERIVED per read, so that a record admitted
    // under one answer cannot outlive it — and, more to the point, so that the
    // three call sites which never had a record to read (Gate F1, the
    // `/agent/add_extension` route, and the sub-agent spawn partition) share the
    // one resolver instead of re-classifying from a bare name.
    //
    // The reasons it was on the record rather than on `ExtensionConfig` still
    // stand and now argue for having no stored copy at all: `ExtensionConfig`
    // round-trips through user-writable `config.yaml`, so anything on it is
    // locally forgeable (R11(i)), and `pool_key` carries no session id, so one
    // `ucsfomopagent` child process is shared across sessions and nothing about
    // a tier could live on the process either.
}

impl Extension {
    fn supports_resources(&self) -> bool {
        self.server_info
            .as_ref()
            .and_then(|info| info.capabilities.resources.as_ref())
            .is_some()
    }

    fn get_instructions(&self) -> Option<String> {
        self.server_info
            .as_ref()
            .and_then(|info| info.instructions.clone())
    }

    fn get_client(&self) -> McpClientBox {
        self.client.clone()
    }
}

/// Manages biorouter extensions / MCP clients and their interactions
/// Whether a `list_resources` fan-out failure is Gate C declining to reach an
/// extension rather than a listing that actually broke (issue #56).
///
/// `privacy_refusal` is the ONE producer of `INVALID_REQUEST` on that path:
/// `list_resources_from_extension`'s own two failures are `INVALID_PARAMS` (no
/// such client) and `INTERNAL_ERROR` (the server refused to list). Pinned by
/// `a_gate_c_refusal_is_told_apart_from_a_real_listing_failure`, which asks the
/// real refusal and both real neighbours rather than asserting the code.
fn is_privacy_refusal(err: &ErrorData) -> bool {
    err.code == ErrorCode::INVALID_REQUEST
}

/// The prefixed tool list and the extension keys that named it, taken together.
///
/// Issue #56 Gate E resolves a prefixed tool name against the set of installed
/// extension keys. Reading the tools and then reading the keys is two reads of
/// shared mutable state at two program points — a race by construction, and one
/// with a skew that is NOT fail-closed: remove a private `a__b` between the two
/// while a public `a` remains, and the tools already in hand still carry
/// `a__b__*`, which now re-resolve to `a` and are listed. Reversing the order
/// only moves the hole, to a concurrent add: a freshly installed private `a__b`'s
/// tools then resolve to `a` against a stale key set.
///
/// So the two are paired at the one place both are known — `fetch_all_tools`'
/// single read of the extension map, which is also where the `key__tool` names
/// are formed. A snapshot therefore cannot describe two different worlds, and
/// the ownership ambiguity the shared prefix resolver exists to remove cannot be
/// smuggled back in through the way its inputs are gathered.
///
/// The tier decision is deliberately NOT in here: it belongs to the currently
/// bound model, and this is cached across model swaps. See
/// [`ExtensionManager::allowed_extension_keys`].
struct ToolsSnapshot {
    /// Sorted by name, for the byte-stable tool-definitions block.
    tools: Vec<Tool>,
    /// Every installed extension key as of the read that produced `tools`.
    keys: Vec<String>,
}

/// What the currently-bound model may reach, on both privacy axes at once —
/// issue #56 Task 48, DR-26.
///
/// The two fields answer two different questions and must never be collapsed:
/// `allowed` is the TIER axis and it *hides*, `marked` is the AFFILIATION axis
/// and it *annotates*. See [`ExtensionManager::extension_reach`], which is the
/// one place either is decided.
#[derive(Default)]
struct ExtensionReach {
    /// Keys this model may see at all.
    allowed: Vec<String>,
    /// `(key, finding)` for the members of `allowed` whose institution the bound
    /// model's agreements do not cover. A **subset** of `allowed`: a mismatch is
    /// listed and marked, never hidden.
    ///
    /// The whole [`crate::privacy::CrossAffiliation`] is carried rather than one
    /// of its strings, because this one value feeds two audiences that need
    /// different lengths of it — the bind and enablement surfaces take
    /// `warning`, Gate E's tool descriptions take the budgeted `mark` — and
    /// composing them separately is how a tool ends up marked but not refused.
    marked: Vec<(String, crate::privacy::CrossAffiliation)>,
}

pub struct ExtensionManager {
    extensions: Mutex<HashMap<String, Extension>>,
    context: PlatformExtensionContext,
    provider: SharedProvider,
    tools_cache: Mutex<Option<Arc<ToolsSnapshot>>>,
    tools_cache_version: AtomicU64,
    /// The session's working directory, when known. Extensions loaded from a
    /// session (`Agent::load_extensions_from_session`) set this so the shell
    /// tool and child-process extensions run in the directory the user selected
    /// (e.g. via the GUI folder picker) rather than the daemon's process cwd.
    /// `None` on non-session paths (CLI), which fall back to the process cwd.
    working_dir: Mutex<Option<PathBuf>>,
}

/// A flattened representation of a resource used by the agent to prepare inference
#[derive(Debug, Clone)]
pub struct ResourceItem {
    pub client_name: String,      // The name of the client that owns the resource
    pub uri: String,              // The URI of the resource
    pub name: String,             // The name of the resource
    pub content: String,          // The content of the resource
    pub timestamp: DateTime<Utc>, // The timestamp of the resource
    pub priority: f32,            // The priority of the resource
    pub token_count: Option<u32>, // The token count of the resource (filled in by the agent)
}

impl ResourceItem {
    pub fn new(
        client_name: String,
        uri: String,
        name: String,
        content: String,
        timestamp: DateTime<Utc>,
        priority: f32,
    ) -> Self {
        Self {
            client_name,
            uri,
            name,
            content,
            timestamp,
            priority,
            token_count: None,
        }
    }
}

/// The per-dispatch [`McpMeta`] for one tool call.
///
/// Extracted from `dispatch_tool_call` only because the repo's
/// `clippy::too_many_lines` baseline caps that function; it is still called from
/// exactly one place, and still from ABOVE `let fut = async move`, so everything
/// here is decided at admission rather than on the far side of the dispatch
/// semaphore.
///
/// Issue #56: the capability bit goes to Biorouter **built-ins only**
/// (decision 4). The session id already ships to third-party stdio servers, and
/// this deliberately does not follow that precedent — "this user is on an
/// institutional model" is a fact about their configuration that a third-party
/// server has no business learning. `cap` is the value `dispatch_tool_call` was
/// CALLED with, never a fresh read of the provider mutex, so this bit and Gate
/// C's decision cannot disagree.
fn dispatch_meta(
    session_id: &str,
    cap: crate::privacy::CallCapability,
    client_name: &str,
    progress_token: Option<String>,
    workspace_child_scope_only: bool,
) -> McpMeta {
    let mut meta =
        McpMeta::new(session_id, cap).with_workspace_child_scope_only(workspace_child_scope_only);
    if let Some(token) = progress_token {
        meta = meta.with_progress_token(token);
    }
    if biorouter_mcp::BUILTIN_EXTENSIONS.contains_key(client_name) {
        meta = meta.with_capability_private(cap.tier().is_private());
        // Issue #56 DR-26 / Task 50 Step 0. The same built-ins-only disclosure,
        // on the same terms and for the same reason: an affiliation on
        // `CallCapability` reaches no MCP server without a `_meta` key and a
        // matching reader. Taken off the SAME `cap` as the bit above, so the two
        // halves of one model's identity cannot be sampled at two instants.
        meta = meta.with_capability_affiliation(
            crate::privacy::affiliation::caller_affiliation_meta_value(cap.affiliation()),
        );
    }
    meta
}

/// Sanitizes a string by replacing invalid characters with underscores.
/// Valid characters match [a-zA-Z0-9_-]
pub fn normalize(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for c in input.chars() {
        result.push(match c {
            c if c.is_ascii_alphanumeric() || c == '_' || c == '-' => c,
            c if c.is_whitespace() => continue, // effectively "strip" whitespace
            _ => '_',                           // Replace any other non-ASCII character with '_'
        });
    }
    result.to_lowercase()
}

/// Which registry owns a bundled extension, and therefore which spawn path a
/// `/ext:` selection has to be dispatched to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundledExtensionKind {
    /// A bundled MCP server in `biorouter_mcp::BUILTIN_EXTENSIONS`.
    Builtin,
    /// An in-process platform extension in `PLATFORM_EXTENSIONS`.
    Platform,
}

/// A bundled extension a `/ext:<name>` marker resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledExtensionTarget {
    kind: BundledExtensionKind,
    /// The name the owning registry is keyed by — what the spawn path in
    /// `add_extension` looks up. For a platform extension this may be a display
    /// string (`"Extension Manager"`), which is exactly why it must never be
    /// handed to the *builtin* lookup.
    name: String,
}

impl BundledExtensionTarget {
    pub fn kind(&self) -> BundledExtensionKind {
        self.kind
    }

    /// The key this extension is stored under once enabled — also the `__`
    /// prefix its tools carry, so it is what the model should be told to use.
    pub fn key(&self) -> String {
        normalize(&crate::config::extensions::name_to_key(&self.name))
    }

    /// The config that enables this target, dispatched to the right variant.
    pub fn into_config(self, description: String) -> ExtensionConfig {
        match self.kind {
            BundledExtensionKind::Builtin => ExtensionConfig::Builtin {
                name: self.name,
                description,
                display_name: None,
                timeout: Some(300),
                bundled: Some(true),
                available_tools: Vec::new(),
            },
            BundledExtensionKind::Platform => ExtensionConfig::Platform {
                name: self.name,
                description,
                bundled: Some(true),
                available_tools: Vec::new(),
            },
        }
    }

    pub(crate) fn matches_config(&self, config: &ExtensionConfig) -> bool {
        let name = match (self.kind, config) {
            (BundledExtensionKind::Builtin, ExtensionConfig::Builtin { name, .. })
            | (BundledExtensionKind::Platform, ExtensionConfig::Platform { name, .. }) => name,
            _ => return false,
        };
        extension_reference_key(name) == extension_reference_key(&self.name)
    }
}

/// Reduce an extension reference to its comparable id: letters and digits only,
/// lowercased. Collapses the spellings a user or a registry may use for the same
/// extension — `agent_drafter` / `agent-drafter` / `agentdrafter`, and
/// `Extension Manager` / `extensionmanager`.
fn extension_reference_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Resolve a `/ext:<name>` target to the bundled extension it names, **by id and
/// owning registry** rather than by display name.
///
/// Two of the five platform extensions are registered under a display string
/// (`"Extension Manager"`, `"Chat Recall"`). The `/ext:` resolver used to hand
/// that string to the *builtin* lookup, which failed with
/// `Unknown builtin extension: Extension Manager` — making a perfectly valid
/// request indistinguishable from a policy refusal (issue #48). Platform targets
/// now resolve to `ExtensionConfig::Platform` and take the platform spawn path.
///
/// Returns `None` for anything that is not bundled; a user-configured
/// stdio/http/inline_python/frontend extension is never auto-enabled from a chat
/// marker, so an operator-disabled entry stays disabled.
pub fn resolve_bundled_extension(requested: &str) -> Option<BundledExtensionTarget> {
    let key = extension_reference_key(requested);
    // The bundled figure server is spelled the British way; accept the American
    // spelling of it too.
    let key = if key == "autovisualizer" {
        "autovisualiser".to_string()
    } else {
        key
    };
    if key.is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    for (registry_key, def) in PLATFORM_EXTENSIONS.iter() {
        if extension_reference_key(registry_key) == key || extension_reference_key(def.name) == key
        {
            matches.push(BundledExtensionTarget {
                kind: BundledExtensionKind::Platform,
                name: def.name.to_string(),
            });
        }
    }

    for (registry_key, def) in biorouter_mcp::BUILTIN_EXTENSIONS.iter() {
        if extension_reference_key(registry_key) == key || extension_reference_key(def.name) == key
        {
            matches.push(BundledExtensionTarget {
                kind: BundledExtensionKind::Builtin,
                name: def.name.to_string(),
            });
        }
    }

    (matches.len() == 1).then(|| matches.remove(0))
}

/// Generates extension name from server info; adds random suffix on collision.
fn generate_extension_name(
    server_info: Option<&ServerInfo>,
    name_exists: impl Fn(&str) -> bool,
) -> String {
    let base = server_info
        .and_then(|info| {
            let name = info.server_info.name.as_str();
            (!name.is_empty()).then(|| normalize(name))
        })
        .unwrap_or_else(|| "unnamed".to_string());

    if !name_exists(&base) {
        return base;
    }

    let suffix: String = rand::thread_rng()
        .sample_iter(Alphanumeric)
        .take(6)
        .map(char::from)
        .collect();

    format!("{base}_{suffix}")
}

fn resolve_command(cmd: &str) -> PathBuf {
    SearchPaths::builder()
        .with_npm()
        .resolve(cmd)
        .unwrap_or_else(|_| {
            // let the OS raise the error
            PathBuf::from(cmd)
        })
}

fn require_str_parameter<'a>(v: &'a serde_json::Value, name: &str) -> Result<&'a str, ErrorData> {
    let v = v.get(name).ok_or_else(|| {
        ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            format!("The parameter {name} is required"),
            None,
        )
    })?;
    match v.as_str() {
        Some(r) => Ok(r),
        None => Err(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            format!("The parameter {name} must be a string"),
            None,
        )),
    }
}

pub fn get_parameter_names(tool: &Tool) -> Vec<String> {
    let mut names: Vec<String> = tool
        .input_schema
        .get("properties")
        .and_then(|props| props.as_object())
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// The environment every stdio-extension child is spawned with.
///
/// Runs after the caller has applied the extension's own declared `envs` /
/// `env_keys`, and is the last thing to touch the child's environment before
/// [`TokioChildProcess`] spawns it — which is what lets it strip BioRouter's
/// daemon-private variables from a child no matter who put them there.
///
/// The working directory is resolved here too (explicit argument first, then
/// `BIOROUTER_WORKING_DIR`). A resolved base is always *named* to the child;
/// only the child's cwd is conditional on that base still existing. See the
/// comment on the branch below — collapsing the two back into one condition is
/// issue #68's jail-widening defect.
fn prepare_child_environment(command: &mut Command, working_dir: Option<&PathBuf>) {
    if let Ok(path) = SearchPaths::builder().path() {
        command.env("PATH", path);
    }

    // Use explicitly passed working_dir, falling back to BIOROUTER_WORKING_DIR env var
    let effective_working_dir = working_dir.map(|p| p.to_path_buf()).or_else(|| {
        std::env::var("BIOROUTER_WORKING_DIR")
            .ok()
            .map(PathBuf::from)
    });

    if let Some(ref dir) = effective_working_dir {
        // ALWAYS name the base, even when it has vanished (issue #68 / F1). The
        // two signals are deliberately no longer under one condition: a child
        // that is told nothing does not fail safe, it falls back to its
        // inherited environment and the daemon's process cwd — `/` under the
        // packaged desktop app — and roots its file jail there, so a session
        // whose directory was deleted mid-conversation gets a *wider* jail than
        // one whose directory still exists. Naming a missing base is what lets
        // `DeveloperServer` refuse the call instead of re-rooting it elsewhere.
        command.env("BIOROUTER_WORKING_DIR", dir);

        if dir.exists() && dir.is_dir() {
            tracing::info!("Setting MCP process working directory: {:?}", dir);
            command.current_dir(dir);
        } else {
            // Only the *cwd* stays conditional: `current_dir` on a path that
            // does not exist is a spawn failure, which would take the extension
            // down entirely and leave the child-side fallback unreachable.
            tracing::warn!(
                working_dir = %dir.display(),
                "extension working directory does not exist or is not a directory; \
                 spawning without a cwd and letting the child refuse against this base"
            );
        }
    } else {
        tracing::info!("No working directory specified, using default");
    }

    // Last, so neither the block above nor the extension's own declared `envs`
    // can leave a daemon credential in the child (issue #57). An extension that
    // needs its own secrets still gets them: they are not in BioRouter's
    // namespace, so the policy never looks at them.
    biorouter_mcp::developer::shell::strip_daemon_private_env(command);
}

async fn child_process_client(
    mut command: Command,
    timeout: &Option<u64>,
    provider: SharedProvider,
    working_dir: Option<&PathBuf>,
    routed_only: bool,
) -> ExtensionResult<McpClient> {
    #[cfg(unix)]
    command.process_group(0);
    configure_command_no_window(&mut command);

    prepare_child_environment(&mut command, working_dir);

    let (transport, mut stderr) = TokioChildProcess::builder(command)
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stderr = stderr.take().ok_or_else(|| {
        ExtensionError::SetupError("failed to attach child process stderr".to_owned())
    })?;

    let stderr_task = tokio::spawn(async move {
        let mut all_stderr = Vec::new();
        stderr.read_to_end(&mut all_stderr).await?;
        Ok::<String, std::io::Error>(String::from_utf8_lossy(&all_stderr).into())
    });

    let client_result = McpClient::connect_routed(
        transport,
        Duration::from_secs(timeout.unwrap_or(crate::config::DEFAULT_EXTENSION_TIMEOUT)),
        provider,
        routed_only,
    )
    .await;

    match client_result {
        Ok(client) => Ok(client),
        Err(error) => {
            let error_task_out = stderr_task.await?;
            Err::<McpClient, ExtensionError>(match error_task_out {
                Ok(stderr_content) => ProcessExit::new(stderr_content, error).into(),
                Err(e) => e.into(),
            })
        }
    }
}

fn extract_auth_error(
    res: &Result<McpClient, ClientInitializeError>,
) -> Option<&AuthRequiredError> {
    match res {
        Ok(_) => None,
        Err(err) => match err {
            ClientInitializeError::TransportError {
                error: DynamicTransportError { error, .. },
                ..
            } => error
                .downcast_ref::<StreamableHttpError<reqwest::Error>>()
                .and_then(|auth_error| match auth_error {
                    StreamableHttpError::AuthRequired(auth_required_error) => {
                        Some(auth_required_error)
                    }
                    _ => None,
                }),
            _ => None,
        },
    }
}

/// Merge environment variables from direct envs and keychain-stored env_keys
async fn merge_environments(
    envs: &Envs,
    env_keys: &[String],
    ext_name: &str,
) -> Result<HashMap<String, String>, ExtensionError> {
    let mut all_envs = envs.get_env();
    let config_instance = Config::global();

    for key in env_keys {
        if all_envs.contains_key(key) {
            continue;
        }

        match config_instance.get(key, true) {
            Ok(value) => {
                if value.is_null() {
                    warn!(
                        key = %key,
                        ext_name = %ext_name,
                        "Secret key not found in config (returned null)."
                    );
                    continue;
                }

                if let Some(str_val) = value.as_str() {
                    all_envs.insert(key.clone(), str_val.to_string());
                } else {
                    warn!(
                        key = %key,
                        ext_name = %ext_name,
                        value_type = %value.get("type").and_then(|t| t.as_str()).unwrap_or("unknown"),
                        "Secret value is not a string; skipping."
                    );
                }
            }
            Err(e) => {
                error!(
                    key = %key,
                    ext_name = %ext_name,
                    error = %e,
                    "Failed to fetch secret from config."
                );
                return Err(ExtensionError::ConfigError(format!(
                    "Failed to fetch secret '{}' from config: {}",
                    key, e
                )));
            }
        }
    }

    Ok(all_envs)
}

/// Substitute environment variables in a string. Supports both ${VAR} and $VAR syntax.
fn substitute_env_vars(value: &str, env_map: &HashMap<String, String>) -> String {
    // Compiled once process-wide rather than on every call (this runs once per
    // extension HTTP header during connection setup).
    static RE_BRACES: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"\$\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}").expect("valid regex")
    });
    static RE_SIMPLE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)").expect("valid regex")
    });

    let mut result = value.to_string();

    for cap in RE_BRACES.captures_iter(value) {
        if let Some(var_name) = cap.get(1) {
            if let Some(env_value) = env_map.get(var_name.as_str()) {
                result = result.replace(&cap[0], env_value);
            }
        }
    }

    let result_snapshot = result.clone();
    for cap in RE_SIMPLE.captures_iter(&result_snapshot) {
        if let Some(var_name) = cap.get(1) {
            if !value.contains(&format!("${{{}}}", var_name.as_str())) {
                if let Some(env_value) = env_map.get(var_name.as_str()) {
                    result = result.replace(&cap[0], env_value);
                }
            }
        }
    }

    result
}

async fn create_streamable_http_client(
    uri: &str,
    timeout: Option<u64>,
    headers: &HashMap<String, String>,
    name: &str,
    all_envs: &HashMap<String, String>,
    provider: SharedProvider,
    routed_only: bool,
) -> ExtensionResult<Box<dyn McpClientTrait>> {
    let mut default_headers = HeaderMap::new();
    for (key, value) in headers {
        let substituted_value = substitute_env_vars(value, all_envs);
        default_headers.insert(
            HeaderName::try_from(key)
                .map_err(|_| ExtensionError::ConfigError(format!("invalid header: {}", key)))?,
            substituted_value.parse().map_err(|_| {
                ExtensionError::ConfigError(format!("invalid header value: {}", key))
            })?,
        );
    }

    let http_client = reqwest::Client::builder()
        .default_headers(default_headers)
        .build()
        .map_err(|_| ExtensionError::ConfigError("could not construct http client".to_string()))?;

    let transport = StreamableHttpClientTransport::with_client(
        http_client,
        StreamableHttpClientTransportConfig {
            uri: uri.into(),
            ..Default::default()
        },
    );

    let timeout_duration =
        Duration::from_secs(timeout.unwrap_or(crate::config::DEFAULT_EXTENSION_TIMEOUT));

    let client_res =
        McpClient::connect_routed(transport, timeout_duration, provider.clone(), routed_only).await;

    if extract_auth_error(&client_res).is_some() {
        let am = oauth_flow(&uri.to_string(), &name.to_string())
            .await
            .map_err(|_| ExtensionError::SetupError("auth error".to_string()))?;
        let auth_client = AuthClient::new(reqwest::Client::default(), am);
        let transport = StreamableHttpClientTransport::with_client(
            auth_client,
            StreamableHttpClientTransportConfig {
                uri: uri.into(),
                ..Default::default()
            },
        );
        Ok(Box::new(
            McpClient::connect_routed(transport, timeout_duration, provider, routed_only).await?,
        ))
    } else {
        Ok(Box::new(client_res?))
    }
}

impl ExtensionManager {
    pub fn new(
        provider: SharedProvider,
        session_manager: Arc<crate::session::SessionManager>,
    ) -> Self {
        Self {
            extensions: Mutex::new(HashMap::new()),
            context: PlatformExtensionContext {
                extension_manager: None,
                session_manager,
            },
            provider,
            tools_cache: Mutex::new(None),
            tools_cache_version: AtomicU64::new(0),
            working_dir: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub fn new_without_provider(data_dir: std::path::PathBuf) -> Self {
        let session_manager = Arc::new(crate::session::SessionManager::new(data_dir));
        Self::new(Arc::new(Mutex::new(None)), session_manager)
    }

    pub fn get_context(&self) -> &PlatformExtensionContext {
        &self.context
    }

    /// Set the session working directory that newly-loaded extensions should run
    /// in. Called before (re)loading a session's extensions, so a change made via
    /// the GUI folder picker — which restarts the agent and reloads extensions —
    /// takes effect for the shell tool and child-process extensions.
    pub async fn set_working_dir(&self, dir: PathBuf) {
        *self.working_dir.lock().await = Some(dir);
    }

    /// Resolve the working directory for an extension.
    /// Prefers the session working directory (set via `set_working_dir`), and
    /// falls back to the process cwd when it is not available (e.g. the CLI).
    ///
    /// Deliberately **not** existence-checked: a session directory that has been
    /// deleted is still this session's base, and dropping it here would send the
    /// child no base at all — issue #68's jail widening. Existence is decided
    /// once, at the spawn, by [`prepare_child_environment`].
    async fn resolve_working_dir(&self) -> PathBuf {
        if let Some(dir) = self.working_dir.lock().await.clone() {
            return dir;
        }
        std::env::current_dir().unwrap_or_default()
    }

    pub async fn supports_resources(&self) -> bool {
        self.extensions
            .lock()
            .await
            .values()
            .any(|ext| ext.supports_resources())
    }

    /// Load an extension the user asked for. Idempotent by key, except that an
    /// explicit config REPLACES an [`ExtensionOrigin::AutoInjected`] entry of
    /// the same name — see [`Self::add_extension_with_origin`].
    pub async fn add_extension(self: &Arc<Self>, config: ExtensionConfig) -> ExtensionResult<()> {
        self.add_extension_with_origin(config, ExtensionOrigin::Explicit)
            .await
    }

    /// Load an extension the agent decided to load for itself (BR-71 decision
    /// 21). Never replaces or re-labels an entry that is already loaded, so an
    /// explicit enable that lands first always wins.
    pub async fn add_extension_auto_injected(
        self: &Arc<Self>,
        config: ExtensionConfig,
    ) -> ExtensionResult<()> {
        self.add_extension_with_origin(config, ExtensionOrigin::AutoInjected)
            .await
    }

    /// The one lifecycle path, with provenance decided in the same critical
    /// sections as the map itself.
    ///
    /// Whether an add is a no-op depends on the EXISTING entry's origin, not
    /// merely on its presence:
    ///
    /// | existing | incoming | outcome |
    /// |---|---|---|
    /// | none | either | load, recording `origin` |
    /// | explicit | explicit | no-op (idempotent by key, as always) |
    /// | explicit | auto | no-op, and provenance is left alone |
    /// | auto | explicit | **replace** — the user outranks the injection |
    /// | auto | auto | no-op |
    ///
    /// The decision is taken twice: once before the (slow, awaiting) client
    /// construction so the common no-op costs nothing, and again under the very
    /// lock the insert happens on, because another writer can land in between.
    /// Without the second check two adds that both saw an empty slot would each
    /// insert, and an auto-injection could overwrite the explicit entry that
    /// beat it to the map.
    ///
    /// Replacement is an overwrite in place, never a remove-then-add: the key
    /// is continuously occupied, so a dispatch that resolved a client from this
    /// map can never find the entry missing when it checks the config.
    #[allow(clippy::too_many_lines)]
    async fn add_extension_with_origin(
        self: &Arc<Self>,
        config: ExtensionConfig,
        origin: ExtensionOrigin,
    ) -> ExtensionResult<()> {
        let config_name = config.key().to_string();
        let sanitized_name = normalize(&config_name);

        if !Self::should_load_over(self.extensions.lock().await.get(&sanitized_name), origin) {
            return Ok(());
        }

        // Resolve working_dir: session > current_dir
        let effective_working_dir = self.resolve_working_dir().await;

        // BR-54: when the SharedMcpPool is enabled and this variant is poolable,
        // reuse ONE process across sessions keyed by spawn identity; otherwise
        // spawn a private, unshared client (byte-identical to the old behavior).
        // A pooled (shared) client isolates notifications per dispatch
        // (`routed_only`); a private client keeps the legacy broadcast.
        let pool_key = if SharedMcpPool::is_enabled() {
            config.pool_key(&effective_working_dir)
        } else {
            None
        };
        let trusted_bundled = match &config {
            ExtensionConfig::Builtin { name, .. } => biorouter_mcp::BUILTIN_EXTENSIONS
                .contains_key(normalize(&crate::config::extensions::name_to_key(name)).as_str()),
            ExtensionConfig::Platform { name, .. } => PLATFORM_EXTENSIONS
                .contains_key(normalize(&crate::config::extensions::name_to_key(name)).as_str()),
            _ => false,
        };
        let routed_only = pool_key.is_some();

        // The actual client construction, deferred into a closure so the pool can
        // skip it entirely when a live shared client already exists (single-flight).
        let spawn = {
            let this = Arc::clone(self);
            let config = config.clone();
            let working_dir = effective_working_dir.clone();
            let sanitized_name = sanitized_name.clone();
            move || async move {
                let mut temp_dir = None;
                let client: Box<dyn McpClientTrait> = match &config {
                    ExtensionConfig::Sse { .. } => {
                        return Err(ExtensionError::ConfigError(
                            "SSE is unsupported, migrate to streamable_http".to_string(),
                        ));
                    }
                    ExtensionConfig::StreamableHttp {
                        uri,
                        timeout,
                        headers,
                        name,
                        envs,
                        env_keys,
                        ..
                    } => {
                        let all_envs = merge_environments(envs, env_keys, &sanitized_name).await?;
                        create_streamable_http_client(
                            uri,
                            *timeout,
                            headers,
                            name,
                            &all_envs,
                            this.provider.clone(),
                            routed_only,
                        )
                        .await?
                    }
                    ExtensionConfig::Stdio {
                        cmd,
                        args,
                        envs,
                        env_keys,
                        timeout,
                        ..
                    } => {
                        let all_envs = merge_environments(envs, env_keys, &sanitized_name).await?;

                        // Check for malicious packages before launching the process
                        extension_malware_check::deny_if_malicious_cmd_args(cmd, args).await?;

                        let cmd = resolve_command(cmd);

                        let command = Command::new(cmd).configure(|command| {
                            command.args(args).envs(all_envs);
                        });

                        let client = child_process_client(
                            command,
                            timeout,
                            this.provider.clone(),
                            Some(&working_dir),
                            routed_only,
                        )
                        .await?;
                        Box::new(client)
                    }
                    ExtensionConfig::Builtin { name, timeout, .. } => {
                        let timeout_duration = Duration::from_secs(timeout.unwrap_or(300));
                        let def = biorouter_mcp::BUILTIN_EXTENSIONS
                            .get(name.as_str())
                            .ok_or_else(|| {
                                ExtensionError::ConfigError(format!(
                                    "Unknown builtin extension: {}",
                                    name
                                ))
                            })?;
                        let (server_read, client_write) = tokio::io::duplex(65536);
                        let (client_read, server_write) = tokio::io::duplex(65536);
                        // Pass the resolved working directory so in-process builtins
                        // that run shell commands (the developer extension) execute in
                        // the session's directory instead of the daemon's process cwd.
                        (def.spawn_server)(server_read, server_write, Some(working_dir.clone()));
                        Box::new(
                            McpClient::connect_routed(
                                (client_read, client_write),
                                timeout_duration,
                                this.provider.clone(),
                                routed_only,
                            )
                            .await?,
                        )
                    }
                    ExtensionConfig::Platform { name, .. } => {
                        let normalized_key = normalize(name);
                        let def = PLATFORM_EXTENSIONS
                            .get(normalized_key.as_str())
                            .ok_or_else(|| {
                                ExtensionError::ConfigError(format!(
                                    "Unknown platform extension: {}",
                                    name
                                ))
                            })?;
                        let mut context = this.context.clone();
                        context.extension_manager = Some(Arc::downgrade(&this));
                        (def.client_factory)(context)
                    }
                    ExtensionConfig::InlinePython {
                        name,
                        code,
                        timeout,
                        dependencies,
                        ..
                    } => {
                        let dir = tempdir()?;
                        let file_path = dir.path().join(format!("{}.py", name));
                        temp_dir = Some(dir);
                        tokio::fs::write(&file_path, code).await?;

                        let command = Command::new("uvx").configure(|command| {
                            command.arg("--with").arg("mcp");
                            dependencies.iter().flatten().for_each(|dep| {
                                command.arg("--with").arg(dep);
                            });
                            command.arg("python").arg(file_path.to_str().unwrap());
                        });

                        let client = child_process_client(
                            command,
                            timeout,
                            this.provider.clone(),
                            Some(&working_dir),
                            routed_only,
                        )
                        .await?;

                        Box::new(client)
                    }
                    ExtensionConfig::Frontend { .. } => {
                        return Err(ExtensionError::ConfigError(
                            "Invalid extension type: Frontend extensions cannot be added as server extensions".to_string()
                        ));
                    }
                };

                let server_info = client.get_info().cloned();
                // `Box<dyn McpClientTrait>` -> `Arc<dyn McpClientTrait>` (H6:
                // the handle is no longer mutex-wrapped).
                Ok(PooledEntry::new(Arc::from(client), server_info, temp_dir))
            }
        };

        let entry = if let Some(key) = pool_key {
            SharedMcpPool::global().get_or_spawn(key, spawn).await?
        } else {
            Arc::new(spawn().await?)
        };

        let server_info = entry.server_info();

        // Only generate name from server info when config has no name (e.g., CLI --with-*-extension args)
        let mut extensions = self.extensions.lock().await;
        let final_name = if sanitized_name.is_empty() {
            generate_extension_name(server_info.as_ref(), |n| extensions.contains_key(n))
        } else {
            sanitized_name
        };
        // Re-decide under the insert's own lock: another writer may have landed
        // while this one was building its client. Dropping the freshly built
        // `entry` here is the correct outcome — the map already holds something
        // that outranks it.
        if !Self::should_load_over(extensions.get(&final_name), origin) {
            return Ok(());
        }
        extensions.insert(
            final_name,
            Extension {
                config,
                client: entry.client(),
                server_info,
                _temp_dir: None,
                inprocess: false,
                trusted_bundled,
                _pooled: Some(entry),
                origin,
            },
        );
        drop(extensions);
        self.invalidate_tools_cache_and_bump_version().await;

        Ok(())
    }

    /// The provenance rule of [`Self::add_extension_with_origin`], as a pure
    /// function of the slot's current occupant and the incoming origin, so both
    /// checks in that method can never drift apart.
    fn should_load_over(existing: Option<&Extension>, incoming: ExtensionOrigin) -> bool {
        match existing {
            None => true,
            // The only overwrite there is: a user decision displacing a grant
            // the agent derived for itself.
            Some(existing) => {
                incoming == ExtensionOrigin::Explicit
                    && existing.origin == ExtensionOrigin::AutoInjected
            }
        }
    }

    /// How `name` came to be loaded, or `None` when it is not loaded at all.
    /// One lock, so callers never see presence and provenance disagree.
    pub async fn extension_origin(&self, name: &str) -> Option<ExtensionOrigin> {
        self.extensions
            .lock()
            .await
            .get(&normalize(name))
            .map(|extension| extension.origin)
    }

    /// Remove `name` only if it is still an auto-injection, in one critical
    /// section. BR-71: the injection is derived state and has to be dropped
    /// when its cause is gone, but a plain check-then-remove would throw away
    /// an explicit enable that landed in between.
    ///
    /// Returns whether anything was removed.
    pub async fn remove_if_auto_injected(&self, name: &str) -> bool {
        let removed = {
            let mut extensions = self.extensions.lock().await;
            let key = normalize(name);
            match extensions.get(&key) {
                Some(extension) if extension.origin == ExtensionOrigin::AutoInjected => {
                    extensions.remove(&key).is_some()
                }
                _ => false,
            }
        };
        if removed {
            self.invalidate_tools_cache_and_bump_version().await;
        }
        removed
    }

    /// True when at least one loaded extension is NOT an auto-injection.
    ///
    /// BR-71: `subagents_enabled` refuses when nothing is loaded, and the
    /// extension it injects is loaded. Counting that injection would make the
    /// predicate sustain itself — a session that removed its last real
    /// extension would keep delegating forever off the back of the grant its
    /// own earlier turn derived. In-process per-app servers DO count here
    /// (unlike in `get_extension_configs`): they are real capability, they are
    /// just not re-spawnable from a config.
    ///
    /// ⚠ **Origin is no longer sufficient, so the name is excluded too.** Once
    /// `workspace` became a default-on capability it loads as `Explicit`, not
    /// `AutoInjected` — so an origin-only test would be satisfied by workspace
    /// itself, in every session, permanently. The predicate would still compile,
    /// still pass its tests, and mean nothing: "the user gave this agent at
    /// least one real capability" would degenerate to "true".
    ///
    /// That is the self-sustaining grant this function was written to prevent,
    /// arriving by the other door. Excluding it by name as well as by origin is
    /// what keeps the question honest.
    pub async fn has_non_injected_extensions(&self) -> bool {
        self.extensions
            .lock()
            .await
            .iter()
            .any(|(name, extension)| {
                extension.origin != ExtensionOrigin::AutoInjected
                    && !name.eq_ignore_ascii_case(crate::agents::agent::Agent::SPAWN_EXTENSION)
            })
    }

    pub async fn add_client(
        &self,
        name: String,
        config: ExtensionConfig,
        client: McpClientBox,
        info: Option<ServerInfo>,
        temp_dir: Option<TempDir>,
    ) {
        self.extensions.lock().await.insert(
            name,
            Extension {
                config,
                client,
                server_info: info,
                _temp_dir: temp_dir,
                inprocess: false,
                trusted_bundled: false,
                _pooled: None,
                origin: ExtensionOrigin::Explicit,
            },
        );
        self.invalidate_tools_cache_and_bump_version().await;
    }

    /// Inject an already-constructed in-process rmcp server into this manager.
    ///
    /// This is the seam for **per-app** servers that carry app context (a
    /// workspace path, a data-source map, a jail) which the no-arg
    /// `BUILTIN_EXTENSIONS` registry cannot express. It mirrors the `Builtin`
    /// spawn path (duplex transport → serve task → `McpClient::connect`) but
    /// serves the *provided* instance instead of looking one up by name.
    pub async fn add_inprocess_server<S>(&self, name: &str, server: S) -> ExtensionResult<()>
    where
        S: rmcp::ServerHandler + Send + 'static,
    {
        use rmcp::ServiceExt;
        // Idempotent: configure_agent re-runs on every (re)connect against the
        // same cached agent, so skip if this server is already injected (avoids
        // a redundant connect/handshake + a transient double-server).
        if self.extensions.lock().await.contains_key(name) {
            return Ok(());
        }
        let (server_read, client_write) = tokio::io::duplex(65536);
        let (client_read, server_write) = tokio::io::duplex(65536);
        let label = name.to_string();
        tokio::spawn(async move {
            match server.serve((server_read, server_write)).await {
                Ok(running) => {
                    let _ = running.waiting().await;
                }
                Err(e) => {
                    tracing::error!(server = %label, error = %e, "in-process server error")
                }
            }
        });
        let client = McpClient::connect(
            (client_read, client_write),
            Duration::from_secs(300),
            self.provider.clone(),
        )
        .await?;
        let info = client.get_info().cloned();
        // A synthetic name-only marker config. It is NEVER spawned from a
        // registry (the `inprocess` flag excludes it from get_extension_configs),
        // so the absence of a "datasql" builtin entry is fine — it exists only so
        // extension listings have a name.
        let config = ExtensionConfig::Builtin {
            name: name.to_string(),
            description: name.to_string(),
            display_name: None,
            timeout: Some(300),
            bundled: None,
            available_tools: Vec::new(),
        };
        self.extensions.lock().await.insert(
            name.to_string(),
            Extension {
                config,
                client: Arc::new(client),
                server_info: info,
                _temp_dir: None,
                inprocess: true,
                trusted_bundled: false,
                _pooled: None,
                // An in-process server is injected by `configure_agent` at the
                // caller's request, i.e. as explicitly as anything gets; it is
                // withheld from `get_extension_configs` by `inprocess`, on its
                // own unrelated grounds.
                origin: ExtensionOrigin::Explicit,
            },
        );
        self.invalidate_tools_cache_and_bump_version().await;
        Ok(())
    }

    /// Get extensions info for building the system prompt.
    ///
    /// Issue #56 Gate F2. Gate E ([`Self::filter_tools`]) is a different
    /// function on a different path, and it does not reach here: a server's
    /// own `instructions` are PROSE, and `reply_parts::prepare_tools_and_prompt`
    /// feeds them through `PromptManager::with_extensions` into the system
    /// prompt of **every turn**. For a clinical connector that text describes
    /// table names, cohort semantics and credential scope, so hiding the tools
    /// while shipping the instructions leaks the more readable half.
    ///
    /// The filter is [`Self::allowed_extension_keys`] — Gate C's predicate,
    /// verbatim, the same one Gate E uses — rather than a second spelling of
    /// the rule. That function's own doc names this surface ("discovery for the
    /// SYSTEM PROMPT has no admitted turn whose decision it could inherit, so
    /// it samples"), which is why `None` is passed here, and sharing it is what
    /// keeps a private server's prose and its tool schemas from being governed
    /// by two predicates that can drift apart. It is also what keeps this
    /// function from becoming a sixth reader of the provider mutex; the tier
    /// and the master opt-out are read together, once, inside the sampler.
    ///
    /// ⚠ **Task 48 (DR-26) deliberately does not mark here.** A mismatched
    /// extension's instructions still ship, unannotated, and that follows from
    /// what the two axes each protect: the tier axis withholds a private
    /// server's prose from a model not entitled to it, while an affiliation
    /// mismatch is between two Private endpoints where the model *is* entitled
    /// to know the connector exists. The warning belongs where the model decides
    /// to act — the tool descriptions Gate E marks — and one statement per
    /// decision point is the whole of DR-19's economy. A second copy in prose
    /// the model reads once per turn buys nothing: the dispatch is refused
    /// either way, and the mark is already in front of it at the moment it
    /// chooses the tool.
    ///
    /// ⚠ This is **not** an argument from prompt bytes, and an earlier version
    /// of this comment that made one did not survive contact with the surface
    /// next door: Gate E prepends its mark to *every* tool of the mismatched
    /// extension, and those descriptions reach the same system prompt. Bytes
    /// are why that mark is budgeted
    /// ([`crate::privacy::affiliation::MARK_BUDGET`]); they are not why this
    /// surface stays silent.
    pub async fn get_extensions_info(&self) -> Vec<ExtensionInfo> {
        let allowed = self.allowed_extension_keys(None).await;
        self.extensions
            .lock()
            .await
            .iter()
            .filter(|(name, _)| allowed.iter().any(|k| k == *name))
            .map(|(name, ext)| {
                ExtensionInfo::classified(
                    name,
                    ext.get_instructions().unwrap_or_default().as_str(),
                    ext.supports_resources(),
                    if ext.config.is_capability() {
                        ExtensionClassification::Capability
                    } else {
                        ExtensionClassification::Extension
                    },
                )
            })
            .collect()
    }

    /// Get aggregated usage statistics
    pub async fn remove_extension(&self, name: &str) -> ExtensionResult<()> {
        let sanitized_name = normalize(name);
        self.extensions.lock().await.remove(&sanitized_name);
        self.invalidate_tools_cache_and_bump_version().await;
        Ok(())
    }

    pub async fn list_extensions(&self) -> ExtensionResult<Vec<String>> {
        Ok(self.extensions.lock().await.keys().cloned().collect())
    }

    pub async fn is_extension_enabled(&self, name: &str) -> bool {
        self.extensions.lock().await.contains_key(name)
    }

    /// Is `tool` granted for `extension` in this session's configuration?
    ///
    /// Extracted so the agent loop can apply the SAME `available_tools` rule to
    /// tools it intercepts before `dispatch_tool_call` (BR-71's merged spawn
    /// tool is the only one). `true` when the extension is not loaded at all —
    /// the caller has its own reason to refuse in that case, and this predicate
    /// answers only the grant question.
    pub async fn is_extension_tool_available(&self, extension: &str, tool: &str) -> bool {
        self.extensions
            .lock()
            .await
            .get(extension)
            .is_none_or(|e| e.config.is_tool_available(tool))
    }

    /// Revalidate an ordinary extension grant captured for a coding-agent
    /// bridge. Both the exact config and the tool allowance must still match;
    /// replacing an extension under the same normalized key therefore revokes
    /// the old turn's immutable grant before dispatch.
    pub async fn is_extension_bridge_grant_current(
        &self,
        key: &str,
        expected: &ExtensionConfig,
        tool: &str,
    ) -> bool {
        self.extensions
            .lock()
            .await
            .get(key)
            .is_some_and(|extension| {
                extension.config == *expected && extension.config.is_tool_available(tool)
            })
    }

    pub async fn is_bundled_target_enabled(&self, target: &BundledExtensionTarget) -> bool {
        self.extensions
            .lock()
            .await
            .get(&target.key())
            .is_some_and(|extension| {
                extension.trusted_bundled && target.matches_config(&extension.config)
            })
    }

    pub async fn is_bundled_target_tool_available(
        &self,
        target: &BundledExtensionTarget,
        tool: &str,
    ) -> bool {
        self.extensions
            .lock()
            .await
            .get(&target.key())
            .is_some_and(|extension| {
                extension.trusted_bundled
                    && target.matches_config(&extension.config)
                    && extension.config.is_tool_available(tool)
            })
    }

    pub async fn trusted_bundled_target_config(
        &self,
        target: &BundledExtensionTarget,
    ) -> Option<ExtensionConfig> {
        self.extensions
            .lock()
            .await
            .get(&target.key())
            .filter(|extension| {
                extension.trusted_bundled && target.matches_config(&extension.config)
            })
            .map(|extension| extension.config.clone())
    }

    /// The extension configs that are safe to write down: everything a user
    /// decision put here, and nothing this process derived for itself.
    ///
    /// Two exclusions, for two different reasons, both about the same hazard —
    /// this snapshot is what gets persisted to the session row, replayed on
    /// resume, handed to a child agent's `TaskConfig`, and baked into a
    /// generated workflow file:
    ///
    /// * per-app in-process servers (`inprocess`): their config is a name-only
    ///   marker that no registry can re-spawn, so replaying it would simply
    ///   fail to load. They are re-injected per connect by `configure_agent`.
    /// * auto-injections (BR-71 decision 21): a delegation-and-supervision
    ///   `workspace` grant derived from `subagents_enabled`, re-derived every turn. Written down
    ///   it becomes a *dead* grant that outlives its cause — it reloads into a
    ///   session whose mode no longer enables delegation (where the dispatch
    ///   gate keys on `session_type`, not on `subagents_enabled`, so it is
    ///   live and callable), and it shows in Settings as though the user had
    ///   enabled Workspace Control.
    pub async fn get_extension_configs(&self) -> Vec<ExtensionConfig> {
        self.extensions
            .lock()
            .await
            .values()
            .filter(|ext| !ext.inprocess && ext.origin != ExtensionOrigin::AutoInjected)
            .map(|ext| ext.config.clone())
            .collect()
    }

    /// The single prefix→extension resolver. Gate C ([`Self::get_client_for_tool`])
    /// and Gate E ([`Self::filter_tools`]) MUST agree, or a tool is hidden by one
    /// and dispatched by the other.
    ///
    /// Longest-key-wins, so `a__b` beats `a` for `a__b__t` and `HashMap`
    /// iteration order stops mattering. The `__` boundary check is what makes
    /// "longest" safe to trust: without it a key would match a tool belonging to
    /// an extension whose name merely starts with the same letters.
    ///
    /// ⚠ Both gates pass **every installed key**, never a filtered subset. The
    /// plan for this task had Gate E resolve within the ALLOWED keys and drop
    /// what did not resolve, which silently reproduces the bug it removes: with
    /// `a` public and `a__b` private, removing `a__b` from the candidate set
    /// makes `a__b__t` resolve to `a` and be listed. Resolve first, decide
    /// second — that is also the only order under which the two gates can be
    /// said to agree, since Gate C has no filtered set to resolve within.
    fn resolve_extension_key<'k>(keys: &'k [String], prefixed_name: &str) -> Option<&'k str> {
        keys.iter()
            // `strip_prefix` rather than `starts_with` + a slice: the slice is
            // safe (a matched prefix ends on a char boundary) but trips
            // `clippy::string_slice`, which this tree denies.
            .filter(|k| {
                prefixed_name
                    .strip_prefix(k.as_str())
                    .is_some_and(|rest| rest.starts_with("__"))
            })
            .max_by_key(|k| k.len())
            .map(String::as_str)
    }

    /// The subset of installed extension keys the currently-bound model may see.
    ///
    /// This is the ONLY half of Gate E's input that depends on the model, which
    /// is why it is read here and not stored in [`ToolsSnapshot`]: the tool list
    /// is cached across model swaps (`update_provider` bumps no cache version),
    /// so freezing a tier decision into that cache would serve one model's
    /// allowed set to the next one.
    ///
    /// Skew against the snapshot it will be compared with is therefore possible
    /// and is fail-closed in every direction: an extension removed after the
    /// snapshot is missing from `allowed`, so its tools — which still resolve to
    /// their real owner, because the snapshot carries the keys that named them —
    /// are dropped; one added after the snapshot is in `allowed` but has no
    /// tools in it. What is NOT possible is a tool resolving to the wrong
    /// extension, which is the skew that would actually leak.
    ///
    /// The predicate is Gate C's, verbatim — `privacy_refusal(..).is_none()` —
    /// and NOT `visible_to(caller, floor(ext.tier))`. Two reasons, and the
    /// second is the load-bearing one:
    ///
    ///  * Comparing a caller's capability with an extension's tier is a
    ///    `ProviderTier`-to-`ProviderTier` question. `floor` crosses a CAPABILITY
    ///    into a CLASSIFICATION, which is a different question, and a third and
    ///    fourth caller of it is a design change Task 7's audit test is there to
    ///    make someone argue for.
    ///  * Gate C (dispatch) and Gate E (discovery) must agree on every input, or
    ///    a tool is hidden by one and dispatched by the other. Sharing one
    ///    function is the only way to guarantee that; sharing a *rule* is not.
    ///
    /// `admitted` follows [`Self::assert_extension_reachable`]'s rule exactly.
    /// Discovery for the SYSTEM PROMPT has no admitted turn whose decision it
    /// could inherit, so it samples. Discovery from INSIDE an admitted tool call
    /// — the `execute_code` bridge's importable-module catalogue — inherits, for
    /// the reason Task 15 gives: the user may switch models mid-turn, and a
    /// fresh sample would hand a Public-admitted script a private module (or,
    /// symmetrically, take a Private-admitted script's module away from it).
    ///
    /// Sampling is also the only spelling that reads the provider's tier and the
    /// master opt-out at ONE instant; reading the provider mutex here and the
    /// toggle again below is the two-reads race
    /// [`crate::privacy::CallCapability`] exists to close.
    ///
    /// The sample itself is one level down, in [`Self::extension_reach`], which
    /// Task 48 extracted so the tier and the affiliation come off one capability
    /// and one registry read. This function is a projection of that verdict, not
    /// a construction site.
    async fn allowed_extension_keys(
        &self,
        admitted: Option<crate::privacy::CallCapability>,
    ) -> Vec<String> {
        self.extension_reach(admitted).await.allowed
    }

    /// Gate E's whole verdict for one capability: which extensions it may see,
    /// and which of those it is **affiliation-incompatible** with (DR-26).
    ///
    /// ⚠ **One pass, one sample, one lock.** The two answers come from the same
    /// [`crate::privacy::resolve_extension`] call on the same record, for the
    /// reason that resolver exists at all: a second lookup would let the tier
    /// and the affiliation disagree about one entry, and the disagreement would
    /// be silent.
    ///
    /// ⚠ **`marked` is a SUBSET of `allowed`, never a second filter.** Gate E
    /// hides a private extension from a public model because the tool's
    /// *existence* is the secret; that reasoning does not carry to affiliation,
    /// where both endpoints are Private and the user is entitled to know the
    /// connector exists (DR-26). Hiding it would also let the agent silently
    /// route around a tool it cannot see, with no one told why. So a mismatched
    /// extension is listed and marked here, and refused at dispatch.
    ///
    /// ⚠ **Task 49's grant is NOT consulted here, and a granted extension stays
    /// marked.** Discovery has no session id — `get_prefixed_tools` is reached
    /// from the tool-list build and from settings screens, none of which is a
    /// dispatch — and a grant is keyed on the triple (session, extension, model
    /// affiliation), so there is nothing to look one up with. The consequence is
    /// cosmetic rather than a hole: marking is not gating, and Gate C is what
    /// actually reads the grant. What it must NOT do is tell the model the call
    /// cannot succeed, because the mark is the only thing the model sees before
    /// a call exists and a model that believes a refusal is certain may never
    /// retry — which would make the user's approval silently worthless. Hence
    /// the conditional refusal clause in `affiliation`'s mark, pinned by
    /// `the_mark_does_not_promise_a_refusal_a_grant_may_already_have_cleared`.
    /// Marking a granted extension differently means threading the session into
    /// discovery, which is Task 50/51 territory.
    async fn extension_reach(
        &self,
        admitted: Option<crate::privacy::CallCapability>,
    ) -> ExtensionReach {
        let cap = match admitted {
            Some(cap) => cap,
            None => crate::privacy::CallCapability::sample(&self.provider).await,
        };
        let caller = cap.tier();
        let enforce = cap.enforced();
        let mut reach = ExtensionReach::default();
        for (key, entry) in self.extensions.lock().await.iter() {
            let class = crate::privacy::resolve_extension(key, Some(&entry.config));
            if enforce
                && crate::privacy::refusal::privacy_refusal(key, class.tier, caller).is_some()
            {
                continue;
            }
            if let Some(finding) = cap.cross_affiliation(key, &class) {
                reach.marked.push((key.clone(), finding));
            }
            reach.allowed.push(key.clone());
        }
        reach
    }

    /// Every enabled extension the given capability is affiliation-incompatible
    /// with, as `(extension key, the warning)` — DR-26's bind and enablement
    /// surfaces, which are the same mismatch found from opposite ends.
    ///
    /// Neither of them blocks. A blocked-outright design is one researchers
    /// route around by turning the feature off, and legitimate cross-
    /// institutional work under a real DUA exists; the warning is the product.
    ///
    /// `admitted` follows [`Self::assert_extension_reachable`]'s rule: a caller
    /// inside an admitted turn passes its capability, and a caller asking about
    /// the model bound *right now* — which is what a bind and a settings screen
    /// both want — passes `None` and gets a fresh sample.
    pub async fn cross_affiliation_warnings(
        &self,
        admitted: Option<crate::privacy::CallCapability>,
    ) -> Vec<(String, String)> {
        let mut marked = self.extension_reach(admitted).await.marked;
        // Deterministic, so a UI listing them and a log line naming them agree
        // across runs — `extensions` is a `HashMap` with per-process-randomised
        // iteration order.
        marked.sort_by(|a, b| a.0.cmp(&b.0));
        // The FULL statement, not the tool-description mark: these two surfaces
        // are read by a human deciding whether to proceed, and DR-26 requires
        // that decision be put to them specifically enough to act on.
        marked
            .into_iter()
            .map(|(key, finding)| (key, finding.warning))
            .collect()
    }

    /// Get all tools from all clients with proper prefixing
    pub async fn get_prefixed_tools(
        &self,
        extension_name: Option<String>,
    ) -> ExtensionResult<Vec<Tool>> {
        let snapshot = self.get_all_tools_cached().await?;
        // Issue #56 Gate E. Precomputed here, in the async caller: `filter_tools`
        // is sync and must not become async (the cache above it is keyed on a
        // version counter `update_provider` never bumps, so a filter applied any
        // higher would freeze one model's allowed set and serve it to the next).
        // The keys the tools are RESOLVED against ride in the snapshot instead,
        // because those two must agree exactly — see [`ToolsSnapshot`].
        let reach = self.extension_reach(None).await;
        Ok(self.filter_tools(
            &snapshot.tools,
            extension_name.as_deref(),
            None,
            &snapshot.keys,
            &reach,
        ))
    }

    /// Get the model-facing tool surface for a turn whose privacy capability
    /// has already been sampled.
    ///
    /// Coding-agent providers receive their callable tools over Biorouter's
    /// short-lived MCP bridge. Code Execution deliberately removes ordinary
    /// extension tools from the provider-facing list, but the bridge still
    /// needs to recover the small audited manager surface from the live
    /// extension registry. Re-sampling the currently bound provider here would
    /// let a model swap between those two steps change the privacy verdict, so
    /// this variant carries the turn's pinned capability into Gate E.
    pub(crate) async fn get_prefixed_tools_for_capability(
        &self,
        admitted: crate::privacy::CallCapability,
    ) -> ExtensionResult<Vec<Tool>> {
        let snapshot = self.get_all_tools_cached().await?;
        let reach = self.extension_reach(Some(admitted)).await;
        Ok(self.filter_tools(&snapshot.tools, None, None, &snapshot.keys, &reach))
    }

    /// Get one attached extension's model-facing tools using the capability
    /// sampled when the lifecycle call was admitted. Lifecycle result payloads
    /// use this to tell the model exactly what became callable or was revoked
    /// without re-sampling the provider mid-turn or bypassing Gate E.
    pub(crate) async fn get_prefixed_tools_for_extension_and_capability(
        &self,
        extension_name: &str,
        admitted: crate::privacy::CallCapability,
    ) -> ExtensionResult<Vec<Tool>> {
        let snapshot = self.get_all_tools_cached().await?;
        let reach = self.extension_reach(Some(admitted)).await;
        Ok(self.filter_tools(
            &snapshot.tools,
            Some(extension_name),
            None,
            &snapshot.keys,
            &reach,
        ))
    }

    /// The `execute_code` bridge's importable-module catalogue, which is a
    /// discovery surface in its own right: `search_modules` and `read_module`
    /// serve tool names, signatures and descriptions out of it on demand, so
    /// Gate E has to reach it as surely as it reaches the system prompt.
    ///
    /// `admitted` is the capability the `execute_code` (or `search_modules` /
    /// `read_module`) call was admitted on — never a fresh sample. A script's
    /// view of the world is the script's permission.
    pub async fn get_prefixed_tools_excluding(
        &self,
        exclude: &str,
        admitted: Option<crate::privacy::CallCapability>,
    ) -> ExtensionResult<Vec<Tool>> {
        let snapshot = self.get_all_tools_cached().await?;
        let reach = self.extension_reach(admitted).await;
        let mut tools =
            self.filter_tools(&snapshot.tools, None, Some(exclude), &snapshot.keys, &reach);
        // Spawning needs the parent agent's provider and task context. This
        // method is the execute_code import catalogue, so an agent-loop-only
        // tool must be removed here rather than by a second, downstream view of
        // the catalogue.
        tools.retain(|tool| {
            tool.name.as_ref() != crate::agents::subagent_tool::SUBAGENT_TOOL_PREFIXED
        });
        // …and so does deleting a knowledge base, for the sibling reason:
        // `kb_delete_base` is contracted to show the user an approval naming the
        // base and its page count, and the sandbox dispatches straight to
        // `dispatch_tool_call` with no inspector chain to raise one. Rather than
        // let the model import a binding that is refused the moment it is
        // called, the tool is not in the catalogue at all — and
        // `reply_parts::survives_code_execution_filter` keeps it directly
        // callable, which is the half that makes this a redirection instead of a
        // removal.
        //
        // ⚠ Measured, not theorised: with the refusal in place and this strip
        // missing, GPT-5.5 wrote `import { kb_delete_base } from "knowledge"`,
        // was refused, and told the user deletion was impossible in this
        // session. It was right.
        tools.retain(|tool| {
            !crate::security::knowledge_delete::is_knowledge_delete_tool(tool.name.as_ref())
        });
        Ok(tools)
    }

    /// The PERMISSION EDITORS' view of the tool list: every installed
    /// extension's tools, with Gate E deliberately NOT applied.
    ///
    /// ⚠ This is the one discovery surface issue #56 leaves open on purpose, and
    /// Task 16's own text is where the decision is recorded: a private extension
    /// must stay **visible and badged** in Settings, and that branch must not be
    /// tier-filtered. The reasoning is that Gate E keeps a private server's tool
    /// names, descriptions and JSON schemas out of a public **model's** context —
    /// Settings → Extensions → tool permissions and `biorouter configure`'s tool
    /// selector are read by the HUMAN who installed that server, never by the
    /// model. Filtering them would buy no confidentiality (Settings is nobody's
    /// prompt) and would cost that human the ability to administer their own
    /// extension, since a tool that is not listed cannot have a permission set.
    ///
    /// Every other caller keeps the filtered [`Self::get_prefixed_tools`]. The
    /// exemption is deny-by-default with exactly two production callers, both
    /// reached through `Agent::list_tools_for_permission_settings`:
    /// `biorouter-server`'s `GET /agent/tools` and `biorouter-cli`'s
    /// `configure`. Adding a third is a privacy decision, not a refactor.
    pub async fn get_prefixed_tools_unfiltered(
        &self,
        extension_name: Option<String>,
    ) -> ExtensionResult<Vec<Tool>> {
        let snapshot = self.get_all_tools_cached().await?;
        // `allowed == keys`. Every tool still has to resolve to a real installed
        // extension, through the same resolver and therefore with the same
        // answer to the overlapping-key hazard as everywhere else — no tier
        // drops it afterwards. Taking both from the snapshot means this view
        // cannot skew against itself at all.
        //
        // `marked` is empty for the same reason the tier filter is absent: this
        // is the HUMAN's administrative view, and DR-26's warning is addressed
        // to a chat with a model bound to it. Settings is nobody's prompt.
        Ok(self.filter_tools(
            &snapshot.tools,
            extension_name.as_deref(),
            None,
            &snapshot.keys,
            &ExtensionReach {
                allowed: snapshot.keys.clone(),
                marked: Vec::new(),
            },
        ))
    }

    /// Issue #56 Gate E: a public model never sees a private server's tool
    /// names, descriptions or JSON schemas. Schema text is content, and it is
    /// handed to the model before any tool call exists for Gate C to refuse.
    ///
    /// ⚠ `Agent::list_tools` appends the platform tools AFTER this returns, so
    /// this cannot hide any of them — all FOUR of
    /// `crate::agents::platform_tools::PLATFORM_TOOL_NAMES`
    /// (`platform__manage_schedule`, `platform__ingest_conversation`,
    /// `platform__ingest_source`, `platform__read_session_blob`), not the three
    /// this note used to list. That is correct — they are public, and the one
    /// that reads across sessions is gated by Gate D — but it is written down so
    /// nobody "fixes" it.
    fn filter_tools(
        &self,
        tools: &[Tool],
        extension_name: Option<&str>,
        exclude: Option<&str>,
        installed: &[String],
        reach: &ExtensionReach,
    ) -> Vec<Tool> {
        tools
            .iter()
            .filter_map(|tool| {
                // Resolve against every installed key — the same set Gate C
                // resolves against — and only THEN ask whether that extension is
                // one this model may see. A name owned by no installed extension
                // is dropped too.
                let tool_prefix = Self::resolve_extension_key(installed, tool.name.as_ref())?;
                if !reach.allowed.iter().any(|k| k == tool_prefix) {
                    return None;
                }

                if let Some(excluded) = exclude {
                    if tool_prefix == excluded {
                        return None;
                    }
                }

                if let Some(name_filter) = extension_name {
                    if tool_prefix != name_filter {
                        return None;
                    }
                }

                // Issue #56 Task 48, DR-26. A mismatched extension is LISTED —
                // both endpoints are Private and the user is entitled to know
                // the connector exists — and marked, so the model reads why its
                // dispatch will be refused at the moment it considers calling.
                // The description is the only channel the model sees before the
                // call exists.
                //
                // ⚠ **The budgeted `mark`, never the full `warning`.** This
                // prepends to EVERY tool of the mismatched extension, and a
                // tool `description` is a bounded field on a real API — Azure
                // OpenAI, which is what Versa Azure is, caps it at 1024
                // characters. The paragraph is ~460 of them, so marking a
                // handful of tools could make the request itself unsendable:
                // "this tool will be refused" would have become "this chat can
                // make no request at all". The full statement is what the
                // refusal carries when the model tries anyway, and what the
                // bind and enablement surfaces put to the user.
                match reach.marked.iter().find(|(k, _)| k == tool_prefix) {
                    Some((_, finding)) => Some(Tool {
                        description: Some(
                            match tool.description.as_deref() {
                                Some(existing) => format!("{}\n\n{existing}", finding.mark),
                                None => finding.mark.clone(),
                            }
                            .into(),
                        ),
                        ..tool.clone()
                    }),
                    None => Some(tool.clone()),
                }
            })
            .collect()
    }

    async fn get_all_tools_cached(&self) -> ExtensionResult<Arc<ToolsSnapshot>> {
        {
            let cache = self.tools_cache.lock().await;
            if let Some(ref snapshot) = *cache {
                return Ok(Arc::clone(snapshot));
            }
        }

        let version_before = self.tools_cache_version.load(Ordering::SeqCst);
        // Deterministic tool order so the serialized tool-definitions block (which
        // Biorouter caches via Anthropic `cache_control`) is byte-stable across
        // process restarts. The source `extensions` HashMap iterates in a
        // per-process-randomized order, so a session resumed in a fresh process
        // would otherwise get a different tool order and miss the provider prompt
        // cache on its first turn. Tools are a named set — ordering is
        // semantically irrelevant to the model, so sorting is safe.
        let mut fetched = self.fetch_all_tools().await?;
        fetched.tools.sort_by(|a, b| a.name.cmp(&b.name));
        // The keys are sorted for the same reason the tools are: they come out
        // of a per-process-randomized `HashMap`, and a snapshot that differs
        // between runs is a snapshot nobody can diff. Resolution does not depend
        // on the order (two equal-length matching prefixes of one string are the
        // same string, so the longest match is unique).
        fetched.keys.sort();
        let snapshot = Arc::new(fetched);

        {
            let mut cache = self.tools_cache.lock().await;
            let version_after = self.tools_cache_version.load(Ordering::SeqCst);
            if version_after == version_before && cache.is_none() {
                *cache = Some(Arc::clone(&snapshot));
            }
        }

        Ok(snapshot)
    }

    async fn invalidate_tools_cache_and_bump_version(&self) {
        self.tools_cache_version.fetch_add(1, Ordering::SeqCst);
        *self.tools_cache.lock().await = None;
    }

    async fn fetch_all_tools(&self) -> ExtensionResult<ToolsSnapshot> {
        let clients: Vec<_> = self
            .extensions
            .lock()
            .await
            .iter()
            .map(|(name, ext)| (name.clone(), ext.config.clone(), ext.get_client()))
            .collect();
        // Out of the SAME read that is about to form the `key__tool` names, so
        // the two can never describe different extension sets. See
        // [`ToolsSnapshot`].
        let keys: Vec<String> = clients.iter().map(|(name, _, _)| name.clone()).collect();

        let client_futures = clients.into_iter().map(|(name, config, client)| {
            let ext_name = name.clone();
            async move {
                let per_ext = async {
                    let cancel_token = CancellationToken::default();
                    let mut tools = Vec::new();
                    let client_guard = &*client;
                    let mut client_tools = match client_guard
                        .list_tools(None, cancel_token.clone())
                        .await
                    {
                        Ok(t) => t,
                        Err(e) => {
                            warn!(extension = %ext_name, error = %e, "Failed to list tools");
                            return vec![];
                        }
                    };

                    loop {
                        for tool in client_tools.tools {
                            if config.is_tool_available(&tool.name) {
                                tools.push(Tool {
                                    name: format!("{}__{}", name, tool.name).into(),
                                    description: tool.description,
                                    input_schema: tool.input_schema,
                                    annotations: tool.annotations,
                                    output_schema: tool.output_schema,
                                    icons: tool.icons,
                                    title: tool.title,
                                    meta: tool.meta,
                                });
                            }
                        }

                        if client_tools.next_cursor.is_none() {
                            break;
                        }

                        client_tools = match client_guard
                            .list_tools(client_tools.next_cursor, cancel_token.clone())
                            .await
                        {
                            Ok(t) => t,
                            Err(e) => {
                                warn!(extension = %ext_name, error = %e, "Failed to list tools (pagination)");
                                break;
                            }
                        };
                    }

                    tools
                };

                // 10s cap per extension — a hanging MCP server (e.g. one waiting on a DB
                // it can't reach) must not block tool listing for all other extensions.
                match tokio::time::timeout(std::time::Duration::from_secs(10), per_ext).await {
                    Ok(tools) => (ext_name, tools),
                    Err(_) => {
                        warn!(extension = %ext_name, "Timed out listing tools after 10s; extension will be skipped");
                        (ext_name, vec![])
                    }
                }
            }
        });

        let results = future::join_all(client_futures).await;

        let mut tools = Vec::new();
        for (_, client_tools) in results {
            tools.extend(client_tools);
        }

        Ok(ToolsSnapshot { tools, keys })
    }

    /// Get the extension prompt including client instructions
    pub async fn get_planning_prompt(&self, tools_info: Vec<ToolInfo>) -> String {
        let mut context: HashMap<&str, Value> = HashMap::new();
        context.insert("tools", serde_json::to_value(tools_info).unwrap());

        prompt_template::render_global_file("plan.md", &context).expect("Prompt should render")
    }

    /// Resolve a prefixed tool name to the extension that owns it: its key, its
    /// client, and — from the SAME snapshot — the config that says whether the
    /// tool may be called at all and the privacy tier it was admitted under.
    ///
    /// The config is returned here rather than looked up again by the caller
    /// because the two lookups could disagree. `dispatch_tool_call` used to
    /// re-read the entry to check `available_tools` and, finding nothing,
    /// skipped the check instead of failing it — so an extension removed
    /// between the two lookups let a forbidden tool through on a client that
    /// had already been cloned. Resolving both together makes that window
    /// disappear: absence at this point is answered with "not found", presence
    /// carries its own authority.
    ///
    /// Issue #56 adds the tier for exactly that reason. Gate C must read the
    /// tier of the record this call was actually routed to, and a second
    /// `self.extensions.lock().await.get(&client_name)` would re-open the
    /// window this function was written to close.
    ///
    /// Task 43 (DR-23) stopped storing that tier and re-derives it here from
    /// the RESOLVED key, still inside this one critical section. Nothing about
    /// the window changes: the tier was already a pure function of the key —
    /// `classify_extension` was what stamped it at all three admission points —
    /// so deriving it beside the client is the same value the record carried,
    /// minus a copy that a rename could make stale.
    /// Task 48 (DR-26): the **classification**, not just the tier. Affiliation
    /// rides the same resolution as the tier — one `resolve_extension` call on
    /// one record inside one critical section — because two lookups would let
    /// the two axes disagree about the same entry, silently.
    async fn get_client_for_tool(
        &self,
        prefixed_name: &str,
    ) -> Option<(
        String,
        McpClientBox,
        ExtensionConfig,
        crate::privacy::ExtensionClassification,
        ExtensionOrigin,
    )> {
        // Issue #56 Task 16: the SAME resolver Gate E filters with. A `find` over
        // a `HashMap` returned whichever of two overlapping keys the
        // per-process-randomised iteration order reached first, so
        // `a__b__t` resolved to `a` or to `a__b` depending on the run.
        let extensions = self.extensions.lock().await;
        let keys: Vec<String> = extensions.keys().cloned().collect();
        let name = Self::resolve_extension_key(&keys, prefixed_name)?;
        let extension = extensions.get(name)?;
        Some((
            name.to_string(),
            extension.get_client(),
            extension.config.clone(),
            crate::privacy::resolve_extension(name, Some(&extension.config)),
            extension.origin,
        ))
    }

    /// BR-23's central secret-redaction scan of one tool call's arguments.
    ///
    /// Extracted from `dispatch_tool_call` only because the repo's
    /// `clippy::too_many_lines` baseline caps that function — the same reason
    /// [`dispatch_meta`] is a free function, and the reason Gate C's own branch
    /// could stay inline where the design says it is. Still called from exactly
    /// one place, still from ABOVE `let fut = async move`, so nothing about the
    /// choke point has moved.
    ///
    /// The `.biorouterignore`/secret deny set used to live only inside the
    /// Developer MCP server, so any other extension (compute, files, a
    /// third-party MCP, a different shell wrapper) could read a
    /// `.env`/private-key/cloud-credential file that the deny set forbids.
    /// Enforcing it at the single choke point every tool call flows through is
    /// what stops an extension bypassing it. The scan is conservative: it only
    /// blocks when an argument names a secret file that actually exists on disk
    /// (see `SecretGuard::find_denied_path`).
    async fn secret_guard_denial(&self, tool_call: &CallToolRequestParams) -> Option<ErrorData> {
        let args = tool_call.arguments.as_ref()?;
        let cwd = self.resolve_working_dir().await;
        let secret_guard_phase =
            crate::agents::phase_timing::Phase::start("mcp.secret_guard_for_dir");
        // 6.2d: memoised per resolved cwd. Invalidated on the exact bytes of
        // every `.biorouterignore` that backs the guard, so an edit is honoured
        // on the very next dispatch (see `cached_for_dir`).
        let guard = biorouter_mcp::secret_guard::SecretGuard::cached_for_dir(&cwd);
        drop(secret_guard_phase);
        let denied = guard.find_denied_path(args)?;
        Some(ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            format!(
                "Access to '{denied}' is blocked: it matches a secret/credential deny pattern \
                 (.env, private key, or cloud credentials). Add a negation to \
                 .biorouterignore to allow it."
            ),
            None,
        ))
    }

    /// Gate C on DR-26's THIRD axis: is this dispatch a cross-institutional data
    /// flow the user has not accepted? `Some(err)` is the refusal.
    ///
    /// Both endpoints are Private and the tier gate already said yes; this asks
    /// the different question — *under whose agreements?* — which no tier gate can
    /// see. UCSF's Versa reaching the UCSF OMOP agent is the arrangement everyone
    /// approved; the same model reaching another institution's connector is a
    /// cross-institutional linkage nobody papered.
    ///
    /// Gate C **refuses**, where Gate E only marks: at discovery the user is
    /// entitled to know the connector exists, but the dispatch is the disclosure,
    /// and an agent can never clear a mismatch on its own. The refusal is where
    /// the user meets the decision.
    ///
    /// It is asked ABOVE the classification ratchet, deliberately and for the
    /// same reason the tier refusal is: a refused call reached no connector, so it
    /// must not permanently classify the chat. DR-15's master opt-out is inside
    /// [`crate::privacy::CallCapability::cross_affiliation_warning`], read off the
    /// same sample every other gate on this path reads.
    ///
    /// ⚠ **Task 49: the refusal is not the end of the story.** DR-26's ruling is
    /// that a mismatch WARNS — the user may accept a stated risk and proceed,
    /// because a blocked-outright design is one researchers route around by
    /// turning the feature off. So a mismatch is refused only while it is
    /// UNGRANTED.
    ///
    /// ⚠ **The grant is consulted here and minted nowhere near here.**
    /// `X-User-Action` is an HTTP header and has no channel on a tool-call path,
    /// so this gate can read an acceptance and can never obtain one: dispatch
    /// refuses → the refusal tells the model to ask the human → the user grants
    /// over `POST /agent/cross_affiliation_grant` → the next call finds the row.
    /// That asymmetry *is* DR-19, and it is why the lookup below is a read.
    ///
    /// ⚠ **The grant lookup is inside the mismatch arm, not beside it.** A
    /// compatible pair must not touch the store at all: the common case is every
    /// dispatch in every chat, and a query per tool call to answer a question with
    /// no institution in it would be a cost paid for nothing.
    async fn cross_affiliation_denial(
        &self,
        session_id: &str,
        client_name: &str,
        ext_class: &crate::privacy::ExtensionClassification,
        cap: crate::privacy::CallCapability,
    ) -> Option<ErrorData> {
        let warning = cap.cross_affiliation_warning(client_name, ext_class)?;
        if crate::privacy::grant::is_granted(
            &self.context.session_manager,
            session_id,
            client_name,
            cap.affiliation(),
        )
        .await
        {
            return None;
        }
        // Task 57: `Some(client_name)` — this is the ONE site that consults the
        // grant, so it is the one whose refusal may offer an accept control. The
        // key handed over is the same `client_name` the lookup above used, so
        // the acceptance the user records is keyed on exactly the triple that
        // was refused.
        Some(crate::privacy::refusal::cross_affiliation_refusal(
            &warning,
            Some(client_name),
        ))
    }

    /// Record on the session row that this chat reached a private extension
    /// (issue #56, O5's second trigger).
    ///
    /// `self.context.session_manager` is the `Arc` `ExtensionManager::new`
    /// takes and stores, so this needs no new plumbing. The storage layer owns
    /// the monotonicity — `raise_privacy` emits a `CASE WHEN` that refuses to
    /// lower a row — and the `mcp:` prefix is what §12.4 grades the
    /// declassification confirmation on, so it must be spelled exactly.
    async fn raise_session_privacy(&self, session_id: &str, reason: &str) -> Result<()> {
        self.context
            .session_manager
            .update(session_id)
            .raise_privacy(crate::privacy::SessionClassification::Private, reason)
            .apply()
            .await
    }

    /// Gate C's predicate for the entry points that reach an MCP server WITHOUT
    /// being a tool call — resource reads, resource listings, prompt listings
    /// and `get_prompt`. `Err` is the refusal; `Ok(())` permits.
    ///
    /// `dispatch_tool_call` is a complete choke point for tool calls and for
    /// nothing else. Eight sibling entry points reach a server beside it, three
    /// of which fan out over **every** installed extension, and
    /// `read_resource_tool` with no `extension_name` is the worst of them: it
    /// actively probes each private server on the model's behalf and swallows
    /// the failure.
    ///
    /// **`admitted` is the capability this reach was ADMITTED on, when there is
    /// one.** Two of the eight — `extensionmanager__read_resource` and
    /// `extensionmanager__list_resources` — are model-callable tools that arrive
    /// through `dispatch_tool_call`, so their call already carries a capability
    /// in its [`McpMeta`]; those paths thread it here and it is used verbatim.
    /// Re-deriving one inside the driven future is precisely the read-then-read
    /// [`crate::privacy::CallCapability`] exists to prevent: the provider mutex
    /// can be reassigned mid-turn with no turn lock, and a resample would let a
    /// Public-admitted call run with Private reach.
    ///
    /// The remaining six have no admitted capability to inherit — they are
    /// called from route handlers, from `Agent::list_extension_prompts` and from
    /// the apps' UI-resource sweep, none of which is a tool call — so they pass
    /// `None` and the only value available is the one read at the decision,
    /// which is a single read rather than a second one.
    ///
    /// Presence is checked under its own lock rather than beside the client,
    /// which [`Self::get_client_for_tool`] deliberately refuses to do. That is
    /// safe **here and only here** because the tier is a pure function of the
    /// key both lookups use (Task 43 made that literal — it is
    /// `classify_extension(key)`, derived rather than stored), so an entry
    /// replaced in between cannot carry a different one.
    ///
    /// ⚠ **An unknown name reads Private — fail-CLOSED, and this is the one
    /// place that direction is inverted.** `classify_extension` fails OPEN to
    /// Public per operator ruling R11(ii), because an unknown *extension* is a
    /// place data might come from. Here the alternative is to permit a reach at
    /// a name the manager could not resolve at all, so the default flips. The
    /// two must stay distinct: collapsing them onto one resolver with one
    /// default silently breaks whichever half it does not implement, and the
    /// broken half would be this one. Hence the explicit membership test below
    /// rather than a `map(...).unwrap_or(...)` over a resolver that already has
    /// an opinion about absent names.
    ///
    /// ⚠ **The unknown-name default is inverted on the TIER axis only** (see
    /// above), and Task 48 does not extend that inversion to affiliation. An
    /// unresolvable name gets [`crate::privacy::ExtensionAffiliation::Any`],
    /// because there is no institution's data at a name that names no
    /// extension: the Private tier default is what refuses the reach, and a
    /// cross-institutional warning about a connector that does not exist is a
    /// false positive that trains users to click through the one that mattered.
    async fn assert_extension_reachable(
        &self,
        name: &str,
        admitted: Option<crate::privacy::CallCapability>,
    ) -> Result<(), ErrorData> {
        let cap = match admitted {
            Some(cap) => cap,
            None => crate::privacy::CallCapability::sample(&self.provider).await,
        };
        let class = self.extensions.lock().await.get(name).map_or(
            crate::privacy::ExtensionClassification {
                tier: crate::privacy::ProviderTier::Private,
                affiliation: crate::privacy::ExtensionAffiliation::Any,
            },
            |extension| crate::privacy::resolve_extension(name, Some(&extension.config)),
        );
        // ⚠ **`private_or_absent_refusal`, NOT `privacy_refusal`, and the reason
        // is the inverted default documented above.** An unknown name arrives
        // here already read as Private, so `privacy_refusal`'s flat *"`x` is a
        // private extension"* asserts a fact this gate has not established — it
        // sent a caller looking for a private model to reach an extension that
        // does not exist (2026-09-10 test drive, finding M18). The replacement
        // states the disjunction and answers the two cases IDENTICALLY, which is
        // what keeps the repair from becoming an existence oracle over exactly
        // the private names Gate E hides. The predicate underneath is unchanged:
        // both compose `tier_refuses`.
        match crate::privacy::refusal::private_or_absent_refusal(name, class.tier, cap.tier()) {
            // DR-15's master opt-out, read through the capability so the tier
            // and the toggle can never be sampled at two different instants —
            // the same predicate Gate C asks, never a second narrower flag.
            Some(err) if cap.enforced() => return Err(err),
            _ => {}
        }
        // Task 48 (DR-26). These eight entry points reach a server without being
        // a tool call, so they refuse exactly as Gate C does — the connector
        // does not care which door the request came through, and three of them
        // fan out over EVERY installed extension.
        //
        // ⚠ **Task 49's grant is NOT consulted here, and the reason is a missing
        // argument rather than a decision.** A grant is keyed on the triple
        // (session, extension, model affiliation), and this function has no
        // session: six of its eight callers are route handlers, the apps'
        // UI-resource sweep and `Agent::list_extension_prompts`, none of which is
        // a tool call and none of which carries a session id today. So a user who
        // has accepted a connector's cross-institutional flow can call its tools
        // and still be refused a resource read on it.
        //
        // That is fail-CLOSED — a refusal the user meets, never a disclosure they
        // did not accept — which is why it ships this way rather than blocking
        // Task 49. Closing it means threading the session through all eight
        // entries, which is Task 50/51 territory, not a line to add here.
        //
        // Task 57: `None`, and for the same missing argument. This path never
        // reads a grant, so a refusal that offered an accept control here would
        // record a real acceptance and refuse the retry anyway.
        match cap.cross_affiliation_warning(name, &class) {
            Some(warning) => Err(crate::privacy::refusal::cross_affiliation_refusal(
                &warning, None,
            )),
            None => Ok(()),
        }
    }

    /// Issue #56 Gate F1, the DISABLE half: may this caller take an installed
    /// extension away from the session? `Err` is the refusal.
    ///
    /// `extensionmanager__manage_extensions {action: "disable"}` used to run
    /// [`Self::remove_extension`] with no capability in scope at all, so a chat
    /// on a public model could drop the clinical connector — a server Gate E
    /// keeps out of that model's tool list entirely, and one it is refused every
    /// call into. Being unable to *see* a connector while being able to *unload*
    /// it is the disagreement this closes.
    ///
    /// ⚠ **This is [`Self::assert_extension_reachable`] verbatim, not a tenth
    /// spelling of the tier comparison.** Discovery and management have to agree
    /// about which extensions a caller may touch, and the only way to guarantee
    /// that is to ask one function — sharing a *rule* is what let these two
    /// drift in the first place. Three consequences follow from reusing it, and
    /// all three are wanted:
    ///
    ///  * **An unknown name reads Private, so it is refused.** That is the
    ///    inverted default `assert_extension_reachable` documents, and here it
    ///    is what stops the refusal being an existence oracle: to a public
    ///    caller, "this private connector is installed", "this private connector
    ///    is not installed" and "no such extension" are one indistinguishable
    ///    refusal. Only *installed public* extensions answer differently, which
    ///    is exactly the set Gate E already showed that caller.
    ///  * **The name is normalized first**, because `remove_extension`
    ///    normalizes before removing and a gate that resolved a different key
    ///    from its executor is a gate with a bypass — `Developer` would miss the
    ///    `developer` entry, read as unknown, and refuse a legitimate disable.
    ///  * **The affiliation arm applies too.** A model bound to another
    ///    institution may see a mismatched connector (DR-26 lists and marks it)
    ///    but may not unload it: an agent that is refused a connector's data
    ///    must not be able to remove the connector instead, and it cannot clear
    ///    the mismatch on its own in either direction.
    ///
    /// `admitted` is required rather than `Option`, unlike its callee: the only
    /// caller is a tool call, which always carries the capability it was
    /// admitted on. There is no "ask about the model bound right now" caller to
    /// serve, so there is no sampling branch to get wrong.
    pub async fn assert_extension_manageable(
        &self,
        name: &str,
        admitted: crate::privacy::CallCapability,
    ) -> Result<(), ErrorData> {
        let normalized = normalize(name);
        if resolve_bundled_extension(&normalized).is_some() {
            return Err(capability_management_error(name));
        }
        if let Some(refusal) = self
            .extensions
            .lock()
            .await
            .get(&normalized)
            .and_then(|extension| capability_management_refusal(&extension.config))
        {
            return Err(refusal);
        }
        self.assert_extension_reachable(&normalized, Some(admitted))
            .await
    }

    /// Function that gets executed for read_resource tool.
    ///
    /// `admitted` is the capability the `extensionmanager__read_resource` call
    /// was admitted on — see [`Self::assert_extension_reachable`]. `None` from
    /// any entry that is not that tool call.
    pub async fn read_resource_tool(
        &self,
        params: Value,
        admitted: Option<crate::privacy::CallCapability>,
        cancellation_token: CancellationToken,
    ) -> Result<Vec<Content>, ErrorData> {
        let uri = require_str_parameter(&params, "uri")?;

        let extension_name = params.get("extension_name").and_then(|v| v.as_str());

        // If extension name is provided, we can just look it up
        if let Some(ext_name) = extension_name {
            // Gate C's sibling for this branch is `read_resource`'s own guard,
            // reached with the same `admitted` capability rather than a fresh
            // sample — one guard, one decision, no second read.
            let read_result = self
                .read_resource(uri, ext_name, admitted, cancellation_token.clone())
                .await?;

            let mut result = Vec::new();
            for content in read_result.contents {
                if let ResourceContents::TextResourceContents { text, .. } = content {
                    let content_str = format!("{}\n\n{}", uri, text);
                    result.push(Content::text(content_str));
                }
            }
            return Ok(result);
        }

        // If extension name is not provided, we need to search for the resource across all extensions
        // Loop through each extension and try to read the resource, don't raise an error if the resource is not found
        // TODO: do we want to find if a provided uri is in multiple extensions?
        // currently it will return the first match and skip any others

        // Collect extension names first to avoid holding the lock during iteration
        let extension_names: Vec<String> = self.extensions.lock().await.keys().cloned().collect();

        for extension_name in extension_names {
            // Gate C's sibling, INSIDE the loop rather than above it. A single
            // up-front check would fail the whole call for a public model that
            // merely has one private extension installed; here the private
            // server is skipped and the public ones still answer.
            if self
                .assert_extension_reachable(&extension_name, admitted)
                .await
                .is_err()
            {
                continue;
            }
            let read_result = self
                .read_resource(uri, &extension_name, admitted, cancellation_token.clone())
                .await;
            match read_result {
                Ok(read_result) => {
                    let mut result = Vec::new();
                    for content in read_result.contents {
                        if let ResourceContents::TextResourceContents { text, .. } = content {
                            let content_str = format!("{}\n\n{}", uri, text);
                            result.push(Content::text(content_str));
                        }
                    }
                    return Ok(result);
                }
                Err(_) => continue,
            }
        }

        // None of the extensions had the resource so we raise an error.
        //
        // ⚠ **The roster is Gate E's, never the extension map's** (issue #56;
        // 2026-09-10 test drive, finding M6). This branch used to build the list
        // straight from `self.extensions.lock().await.keys()`, so a caller
        // evaluated as PUBLIC — every `POST /agent/call_tool`, which passes
        // `CallCapability::public_enforced()` — was handed the names of the
        // private extensions loaded in that chat, in the one place the loop
        // above had just finished refusing it each of them one at a time. Gate E
        // exists to keep those names out of a public model's sight; a
        // not-found message is not an exemption from it.
        //
        // Filtered rather than dropped, because the list is genuinely useful to
        // a model that has passed Gate E, and `allowed_extension_keys` is the
        // SAME verdict that built the tool list this model is reading from — so
        // it can never name an extension the model was not already shown.
        // `admitted` is threaded, not resampled: this is a tool-call path, and
        // re-reading the provider here is the read-then-read `CallCapability`
        // exists to prevent.
        //
        // Sorted because `extensions` is a `HashMap` whose iteration order is
        // randomised per process, exactly as `cross_affiliation_warnings` and
        // the manager's own enabled listing sort.
        let mut visible = self.allowed_extension_keys(admitted).await;
        visible.sort();
        let error_msg = if visible.is_empty() {
            format!(
                "Resource with uri '{}' not found in any extension this chat can reach.",
                uri
            )
        } else {
            format!(
                "Resource with uri '{}' not found. Here are the available extensions: {}",
                uri,
                visible.join(", ")
            )
        };

        Err(ErrorData::new(
            ErrorCode::RESOURCE_NOT_FOUND,
            error_msg,
            None,
        ))
    }

    /// `admitted` carries the capability a `extensionmanager__read_resource`
    /// tool call was admitted on; the route handler and the apps' sweep, which
    /// are not tool calls, pass `None`.
    pub async fn read_resource(
        &self,
        uri: &str,
        extension_name: &str,
        admitted: Option<crate::privacy::CallCapability>,
        cancellation_token: CancellationToken,
    ) -> Result<rmcp::model::ReadResourceResult, ErrorData> {
        // Gate C's sibling: the name is known, so refuse before any client call.
        self.assert_extension_reachable(extension_name, admitted)
            .await?;

        let client = match self.get_server_client(extension_name).await {
            Some(client) => client,
            None => {
                // Gate E's roster, for the reason the fan-out branch above
                // states at length. This one is narrower and was already
                // covered at the route by #206's `read_resource_failure`, which
                // REPLACES this message rather than forwarding it — but the
                // route is not the only caller, and a message that is safe only
                // because one of its readers throws it away is a leak waiting
                // for a second reader.
                //
                // ⚠ Built on the MISS, not before the lookup. The list used to
                // be composed unconditionally and handed to `ok_or`, which is
                // eager: every successful resource read paid for it. Gate E's
                // verdict costs a `resolve_extension` per installed entry, so
                // moving it here is not tidying — leaving it above would put
                // that walk on the hot path.
                let mut visible = self.allowed_extension_keys(admitted).await;
                visible.sort();
                let error_msg = if visible.is_empty() {
                    format!("Extension '{}' is not loaded in this chat.", extension_name)
                } else {
                    format!(
                        "Extension '{}' not found. Here are the available extensions: {}",
                        extension_name,
                        visible.join(", ")
                    )
                };
                return Err(ErrorData::new(ErrorCode::INVALID_PARAMS, error_msg, None));
            }
        };

        let client_guard = &*client;
        client_guard
            .read_resource(uri, cancellation_token)
            .await
            .map_err(|_| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Could not read resource with uri: {}", uri),
                    None,
                )
            })
    }

    async fn list_resources_from_extension(
        &self,
        extension_name: &str,
        admitted: Option<crate::privacy::CallCapability>,
        cancellation_token: CancellationToken,
    ) -> Result<Vec<Content>, ErrorData> {
        // Gate C's sibling. This is also the guard that keeps `list_resources`'
        // fan-out partial rather than all-or-nothing: the fan-out drives one of
        // these per extension and collects the failures, so a refusal here drops
        // exactly the private extension.
        self.assert_extension_reachable(extension_name, admitted)
            .await?;

        let client = self
            .get_server_client(extension_name)
            .await
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    format!("Extension {} is not valid", extension_name),
                    None,
                )
            })?;

        let client_guard = &*client;
        client_guard
            .list_resources(None, cancellation_token)
            .await
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Unable to list resources for {}, {:?}", extension_name, e),
                    None,
                )
            })
            .map(|lr| {
                let resource_list = lr
                    .resources
                    .into_iter()
                    .map(|r| format!("{} - {}, uri: ({})", extension_name, r.name, r.uri))
                    .collect::<Vec<String>>()
                    .join("\n");

                vec![Content::text(resource_list)]
            })
    }

    /// `admitted` is the capability the `extensionmanager__list_resources` call
    /// was admitted on — see [`Self::assert_extension_reachable`]. `None` from
    /// any entry that is not that tool call.
    pub async fn list_resources(
        &self,
        params: Value,
        admitted: Option<crate::privacy::CallCapability>,
        cancellation_token: CancellationToken,
    ) -> Result<Vec<Content>, ErrorData> {
        let extension = params.get("extension").and_then(|v| v.as_str());

        match extension {
            Some(extension_name) => {
                // Gate C's sibling at this altitude too. `list_resources_from_extension`
                // guards itself — that is what keeps the fan-out below partial —
                // but the named branch is a distinct entry point, and stating the
                // refusal here means a later refactor that inlines the helper
                // cannot silently drop it.
                self.assert_extension_reachable(extension_name, admitted)
                    .await?;

                // Handle single extension case
                self.list_resources_from_extension(extension_name, admitted, cancellation_token)
                    .await
            }
            None => {
                // Handle all extensions case using FuturesUnordered
                let mut futures = FuturesUnordered::new();

                // Create futures for each resource_capable_extension
                self.extensions
                    .lock()
                    .await
                    .iter()
                    .filter(|(_name, ext)| ext.supports_resources())
                    .map(|(name, _ext)| name.clone())
                    .for_each(|name| {
                        let token = cancellation_token.clone();
                        futures.push(async move {
                            self.list_resources_from_extension(&name.clone(), admitted, token)
                                .await
                        });
                    });

                let mut all_resources = Vec::new();
                let mut errors = Vec::new();

                // Process results as they complete
                while let Some(result) = futures.next().await {
                    match result {
                        Ok(content) => {
                            all_resources.extend(content);
                        }
                        Err(tool_error) => {
                            errors.push(tool_error);
                        }
                    }
                }

                // Gate C declining a private extension is not a failure of this
                // listing — it is the design, and under a public model with one
                // private extension installed it happens on EVERY listing. At
                // ERROR that would put a full refusal in the log each time;
                // `list_prompts`' fan-out already uses `debug!` for the same
                // thing. A server that genuinely could not be listed still
                // reaches ERROR.
                let (refusals, errors): (Vec<_>, Vec<_>) =
                    errors.into_iter().partition(is_privacy_refusal);

                if !refusals.is_empty() {
                    tracing::debug!(
                        skipped = refusals.len(),
                        "skipped extensions this session's model may not reach"
                    );
                }

                if !errors.is_empty() {
                    tracing::error!(
                        errors = ?errors
                            .into_iter()
                            .map(|e| format!("{:?}", e))
                            .collect::<Vec<_>>(),
                        "errors from listing resources"
                    );
                }

                Ok(all_resources)
            }
        }
    }

    async fn prefixed_tool_name(&self, tool_name: &str) -> String {
        if !tool_name.contains("__")
            && ["execute_code", "read_module", "search_modules"].contains(&tool_name)
            && self.extensions.lock().await.contains_key("code_execution")
        {
            format!("code_execution__{tool_name}")
        } else {
            tool_name.to_string()
        }
    }

    /// Dispatch one tool call to the extension that owns it.
    ///
    /// Issue #56: `cap` is the capability this call was ADMITTED on, sampled
    /// once at the entry that admitted it. This function has **no way to
    /// sample** — deliberately. Every barrier downstream (Gate C here, the
    /// built-in `_meta` bit, and the Platform extensions through `McpMeta`)
    /// reads that one value, so they cannot disagree, and none of them can
    /// re-derive a *newer* provider tier from inside the driven future, which is
    /// what would let a Public-admitted call run with Private reach.
    pub async fn dispatch_tool_call(
        &self,
        session_id: &str,
        tool_call: CallToolRequestParams,
        cap: crate::privacy::CallCapability,
        cancellation_token: CancellationToken,
    ) -> Result<ToolCallResult> {
        // Some models strip the tool prefix, so auto-add it for known code_execution tools.
        let prefixed_name = self.prefixed_tool_name(tool_call.name.as_ref()).await;

        // Dispatch tool call based on the prefix naming convention. The client
        // and the config that authorizes it come out of ONE snapshot — see
        // `get_client_for_tool`.
        let (client_name, client, client_config, ext_class, extension_origin) = self
            .get_client_for_tool(&prefixed_name)
            .await
            .ok_or_else(|| unroutable_tool_error(&prefixed_name, tool_call.name.as_ref()))?;

        let tool_name = prefixed_name
            .strip_prefix(client_name.as_str())
            .and_then(|s| s.strip_prefix("__"))
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::RESOURCE_NOT_FOUND,
                    format!("Invalid tool name format: '{}'", tool_call.name),
                    None,
                )
            })?
            .to_string();

        // Unconditional: the config was resolved with the client, so there is no
        // "the extension has gone" branch that could skip the check rather than
        // fail it.
        if !client_config.is_tool_available(&tool_name) {
            return Err(ErrorData::new(
                ErrorCode::RESOURCE_NOT_FOUND,
                format!(
                    "Tool '{}' is not available for extension '{}'",
                    tool_name, client_name
                ),
                None,
            )
            .into());
        }

        // Issue #56 Gate C, beside BR-23's SecretGuard block below for the
        // reason that block's own comment states: this is the single choke
        // point every tool call flows through. FOUR production paths converge
        // here and only ONE of them carries a `ToolInspector` — the agent loop
        // (`Agent::dispatch_tool_call`), `POST /agent/call_tool`, the
        // `execute_code` JS bridge (`CodeExecutionClient::dispatch_sub_call`,
        // which re-enters THIS function, not the Agent's) and
        // `Agent::call_prefetch_tool`, which runs BEFORE the turn. A gate
        // written as an inspector would be invisible to three of the four.
        //
        // `ext_class` is read off the RESOLVED RECORD, never off the tool-name
        // string. `normalize()` permits `_`, so extensions keyed `a` and `a__b`
        // both match `a__b__c` by prefix. `get_client_for_tool` settles that
        // with the single longest-key-wins resolver in its body — the SAME one
        // Gate E filters the tool list with, introduced by Task 16 exactly so
        // the two gates cannot disagree about who owns a name. (This comment
        // used to say the routing was a `starts_with` scan of a HashMap in
        // nondeterministic order. That was true, and is what Task 16 fixed.)
        // Re-deriving the tier from the string here would reintroduce the
        // disagreement one layer down, where nothing would catch it; instead it
        // comes out of the same snapshot as the client and the config, so there
        // is no second lookup to drift from the first either.
        //
        // `cap` is a PARAMETER. Gate C does not sample and cannot:
        // `dispatch_tool_call` has no way to read the provider any more. That
        // is what makes this decision, the built-in `_meta` bit and the
        // Platform extensions' capability provably the same decision.
        //
        // `cap.enforced()` is DR-15's master opt-out, read here inside the gate
        // rather than through an `is_enabled()` wrapper — one auditable line
        // rather than an absent gate. It is the SAME predicate in every gate;
        // do not introduce a second, narrower flag for this one.
        if cap.enforced() {
            if let Some(err) =
                crate::privacy::refusal::privacy_refusal(&client_name, ext_class.tier, cap.tier())
            {
                return Err(err.into());
            }
        }

        // Issue #56 Tasks 48 and 49, DR-26's THIRD axis. See
        // [`Self::cross_affiliation_denial`], which is where the whole rationale
        // lives; it is a separate function only because the repo's
        // `clippy::too_many_lines` baseline caps this one, and it is still called
        // from ABOVE `let fut = async move`, so it decides at admission.
        if let Some(err) = self
            .cross_affiliation_denial(session_id, &client_name, &ext_class, cap)
            .await
        {
            return Err(err.into());
        }

        // BR-23: the central secret-redaction boundary — see
        // [`Self::secret_guard_denial`], which is where the whole rationale
        // lives.
        if let Some(err) = self.secret_guard_denial(&tool_call).await {
            return Err(err.into());
        }

        // Issue #56, O5's second trigger. See [`Self::ratchet_for_private_extension`],
        // where the whole rationale lives; it is a separate function for
        // `cross_affiliation_denial`'s reason verbatim — the repo's
        // `clippy::too_many_lines` baseline caps this one — and it is still called
        // from ABOVE `let fut = async move`, so it fires at admission.
        if cap.enforced() && ext_class.tier.is_private() {
            self.ratchet_for_private_extension(session_id, &client_name, &ext_class)
                .await?;
        }

        let arguments = tool_call.arguments.clone();
        let client = client.clone();
        // BR-54: on a shared (pooled) client this mints a per-dispatch progress
        // token and returns a receiver that gets ONLY this dispatch's
        // notifications; on an unpooled client it is the legacy broadcast
        // subscription with no token.
        // H6: there is no client-wide lock to wait on any more, so the former
        // `mcp.register_dispatch_wait` span is gone. `register_dispatch()` is
        // still real work (it mints a progress token and installs a routing
        // entry) and keeps its own span.
        let register_call = crate::agents::phase_timing::Phase::start("mcp.register_dispatch");
        let (progress_token, notifications_receiver) = client.register_dispatch().await;
        drop(register_call);
        let session_id = session_id.to_string();

        // Issue #56: built HERE, not inside the future below. `register_dispatch()`
        // already runs on this side of the boundary, so it costs nothing — and it
        // is what keeps the capability a value that was decided at admission
        // rather than something re-derived on the far side of the dispatch
        // semaphore, minutes later, against whatever provider is bound by then.
        let workspace_child_scope_only =
            client_name == "workspace" && extension_origin == ExtensionOrigin::AutoInjected;
        let meta = dispatch_meta(
            &session_id,
            cap,
            &client_name,
            progress_token,
            workspace_child_scope_only,
        );

        let fut = async move {
            tracing::debug!(
                "dispatch_tool_call fut: calling client.call_tool tool={} session_id={}",
                tool_name,
                session_id
            );
            // H6 (fixed): this used to take a client-wide mutex and hold it
            // across the whole `call_tool` await, so two tool calls on the SAME
            // extension ran back-to-back — `sum(durations)` instead of
            // `max(durations)`. `call_tool` takes `&self` and implementations
            // are internally synchronized, so the guard (and its
            // `mcp.client_lock_wait` span) is gone and calls now overlap.
            let _call_phase = crate::agents::phase_timing::Phase::start("mcp.call_tool");
            client
                .call_tool(&tool_name, arguments, meta, cancellation_token)
                .await
                .map_err(|e| match e {
                    ServiceError::McpError(error_data) => error_data,
                    _ => {
                        ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string(), e.maybe_to_value())
                    }
                })
        };

        Ok(ToolCallResult {
            result: Box::new(fut.boxed()),
            notification_stream: Some(Box::new(ReceiverStream::new(notifications_receiver))),
        })
    }

    /// Gate C's ratchet: a permitted dispatch into a PRIVATE extension classifies
    /// the session, permanently (issue #56, O5's second trigger).
    ///
    /// Split out of [`Self::dispatch_tool_call`] so that function stays under
    /// `clippy::too_many_lines`, exactly as [`Self::cross_affiliation_denial`] and
    /// [`Self::secret_guard_denial`] were; the caller keeps the `cap.enforced() &&
    /// ext_class.tier.is_private()` condition so the guard stays visible at the
    /// dispatch seam.
    ///
    /// At PERMIT time, not on the tool's result, and that is forced by the shape
    /// of the caller rather than chosen: it returns its `ToolCallResult` BEFORE
    /// the tool has run, and the `async move` there captures owned values only —
    /// it cannot hold `&self`, so there is no `self` at the point the call
    /// succeeds.
    ///
    /// Permit-time is also the right direction. "The model was allowed to ask a
    /// private extension a question" is the disclosure; whether the extension
    /// answered is not the user's protection. Ratcheting on success would leave
    /// a failed OMOP query — which still carried the session's cohort definition
    /// to the connector — unrecorded.
    ///
    /// It is the LAST of the admission checks, below BR-23's scan rather than
    /// above it, because the classification is a permanent ratchet and that same
    /// rationale runs the other way for a call that never left the process: a
    /// SecretGuard denial carried nothing to the connector, so it has nothing to
    /// record.
    ///
    /// The `?` is deliberate and follows Gate B, which also fails its turn when
    /// the ratchet cannot be written: a disclosure this process cannot record is
    /// one it must not perform. A session id with no row is a silent no-op at
    /// the storage layer (0 rows updated), not an error, so this refuses only on
    /// a real store failure.
    ///
    /// `cap.enforced()` — the caller's half of the guard — gates the ratchet as
    /// well as the refusal, and that is AR-7 rather than symmetry for its own
    /// sake. The master opt-out's contract is that with it off *nothing is
    /// impacted*, and a ratchet that keeps firing is an impact — a deferred,
    /// permanent one, since `privacy_tier` is monotone and re-enabling never
    /// revisits a row. The alternative ("a session that queried OMOP is still a
    /// session that queried OMOP") was considered by the design and rejected: it
    /// would silently privatise chats a user believes are unprotected, and the
    /// first they would learn of it is a refusal weeks later. The cost is named
    /// and accepted in AR-7 — a disclosure made while the feature was off is
    /// never reclassified.
    async fn ratchet_for_private_extension(
        &self,
        session_id: &str,
        client_name: &str,
        ext_class: &crate::privacy::ExtensionClassification,
    ) -> Result<()> {
        self.raise_session_privacy(session_id, &format!("mcp:{client_name}"))
            .await?;
        // Issue #56 DR-26 / Task 50 Step 3: "a private chat carries the
        // affiliation of the extensions it touched". BESIDE the tier ratchet
        // and under the same guard, at the same permit-time instant and off
        // the same `ext_class` — a second read of the classification here
        // could disagree with the one the gates above decided on.
        //
        // ⚠ Only `Institutions` contributes. `Any` is a private extension
        // with no institutional constraint, so touching it puts no
        // institution's data in this chat and recording one would warn on a
        // recall nobody needs warned about — the prompt fatigue DR-19
        // rejects.
        if let crate::privacy::ExtensionAffiliation::Institutions(owners) = &ext_class.affiliation {
            for owner in owners {
                self.context
                    .session_manager
                    .record_session_affiliation(session_id, *owner)
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn list_prompts_from_extension(
        &self,
        extension_name: &str,
        cancellation_token: CancellationToken,
    ) -> Result<Vec<Prompt>, ErrorData> {
        // Gate C's sibling: the name is known, so refuse before any client call.
        // Prompt listing is not a tool call — no admitted capability to inherit.
        self.assert_extension_reachable(extension_name, None)
            .await?;

        let client = self
            .get_server_client(extension_name)
            .await
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    format!("Extension {} is not valid", extension_name),
                    None,
                )
            })?;

        let client_guard = &*client;
        client_guard
            .list_prompts(None, cancellation_token)
            .await
            .map_err(|e| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Unable to list prompts for {}, {:?}", extension_name, e),
                    None,
                )
            })
            .map(|lp| lp.prompts)
    }

    pub async fn list_prompts(
        &self,
        cancellation_token: CancellationToken,
    ) -> Result<HashMap<String, Vec<Prompt>>, ErrorData> {
        let mut futures = FuturesUnordered::new();

        let names: Vec<_> = self.extensions.lock().await.keys().cloned().collect();
        for extension_name in names {
            // Gate C's sibling, INSIDE the loop: a private extension is left out
            // of the map and every public one is still listed.
            if self
                .assert_extension_reachable(&extension_name, None)
                .await
                .is_err()
            {
                continue;
            }
            let token = cancellation_token.clone();
            futures.push(async move {
                (
                    extension_name.clone(),
                    self.list_prompts_from_extension(extension_name.as_str(), token)
                        .await,
                )
            });
        }

        let mut all_prompts = HashMap::new();
        let mut errors = Vec::new();

        // Process results as they complete
        while let Some(result) = futures.next().await {
            let (name, prompts) = result;
            match prompts {
                Ok(content) => {
                    all_prompts.insert(name.to_string(), content);
                }
                Err(tool_error) => {
                    errors.push(tool_error);
                }
            }
        }

        if !errors.is_empty() {
            tracing::debug!(
                errors = ?errors
                    .into_iter()
                    .map(|e| format!("{:?}", e))
                    .collect::<Vec<_>>(),
                "errors from listing prompts"
            );
        }

        Ok(all_prompts)
    }

    pub async fn get_prompt(
        &self,
        extension_name: &str,
        name: &str,
        arguments: Value,
        cancellation_token: CancellationToken,
    ) -> Result<GetPromptResult> {
        // Gate C's sibling. An MCP prompt body is server-authored text that
        // lands in the transcript verbatim, so this refuses before the fetch —
        // the refusal carries the extension name and the two tiers and nothing
        // the server wrote. Not a tool call, so nothing to inherit.
        self.assert_extension_reachable(extension_name, None)
            .await?;

        let client = self
            .get_server_client(extension_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Extension {} not found", extension_name))?;

        let client_guard = &*client;
        client_guard
            .get_prompt(name, arguments, cancellation_token)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get prompt: {}", e))
    }

    /// Issue #56 Gate E, the CATALOGUE surface.
    ///
    /// ⚠ **The contradiction this signature resolves, and which of the two
    /// claims governs.** Two places in this file used to state opposite rules
    /// for the same axis: Gate E's own doc ([`Self::extension_reach`], and
    /// `gate_e_lists_and_marks_a_mismatched_extensions_tools`) says a private
    /// extension is hidden from a public model *because the tool's existence is
    /// the secret*, while Task 15's sibling census excused this function on the
    /// grounds that "it does reveal that a private extension is INSTALLED, which
    /// is an existence leak and explicitly out of scope (DR-7)".
    ///
    /// **The first claim governs, and this function is now filtered.** Three
    /// reasons, in the order they decide it:
    ///
    ///  1. The out-of-scope claim was never a ruling about this surface. DR-7
    ///     rules out the *side channels* — timing, error-shape and latency
    ///     differences by which a determined model could infer that something
    ///     exists. Printing a private connector's name and its marketplace
    ///     description into the model's context is not a side channel; it is the
    ///     disclosure itself, arriving through the front door.
    ///  2. Gate E is load-bearing and hiding is its whole mechanism. If a public
    ///     model may read `ucsfomopagent` and "Natural-language SQL over the
    ///     UCSF OMOP de-identified clinical database" out of a catalogue, then
    ///     withholding that same server's tool names and schemas next door buys
    ///     nothing — and the instruction on this fix is explicit that the two
    ///     surfaces are made to agree by tightening the looser one, never by
    ///     weakening Gate E.
    ///  3. It is the only listing surface still open. `get_extensions_info`
    ///     (the system prompt's capability/extension roster) already filters through
    ///     `allowed_extension_keys`; `get_prefixed_tools` and
    ///     `get_prefixed_tools_excluding` are Gate E proper. Leaving one
    ///     unfiltered listing beside three filtered ones is not a scope
    ///     boundary, it is the gap.
    ///
    /// So: **both halves apply the same privacy verdict as Gate E, then exclude
    /// Biorouter capabilities because this tool manages third-party extensions.**
    /// The "disabled in the config" half is not in `self.extensions` and so
    /// cannot come from that verdict — [`config_disabled_extension_lines`]
    /// applies the same predicate (`privacy_refusal`, under `cap.enforced()`) to
    /// the config entries instead.
    ///
    /// `admitted` is the capability the `search_available_extensions` tool call
    /// was admitted on. Required rather than `Option`, and never sampled here:
    /// the sole caller is that tool call, and a fresh read inside the driven
    /// future is what would let a Public-admitted call read a private
    /// connector's name out of the catalogue after the user switched models
    /// mid-turn. It is also passed straight into [`Self::extension_reach`], so
    /// one capability decides both halves.
    pub async fn search_available_extensions(
        &self,
        admitted: crate::privacy::CallCapability,
    ) -> Result<Vec<Content>, ErrorData> {
        let mut output_parts = vec![];

        // First get disabled extensions from current config; only entries the
        // operator actually persisted with `enabled: false` get the
        // do-not-enable label (#42). Biorouter capabilities are managed on
        // their own settings surface and never enter this extension listing.
        // Entries this caller may not see at all never reach the labelling step.
        // The keys of the LIVE map, taken once. An entry the config enables but
        // this map does not hold is installed-and-detached: `manage_extensions
        // {disable}` never touches the config, so a detach left the extension
        // invisible to this inventory entirely — the model could not learn its
        // name to attach it again.
        let live_keys: std::collections::HashSet<String> =
            self.extensions.lock().await.keys().cloned().collect();
        let (disabled_extensions, detached_extensions) = extension_listing_lines(
            &get_all_extensions(),
            &crate::config::persisted_extension_names(),
            &live_keys,
            admitted,
        );

        // Get currently enabled third-party extensions that can be disabled.
        // Gate E supplies the privacy verdict; the config classification then
        // removes Biorouter capabilities from this manager-specific listing.
        // Sorted because `extensions` is a `HashMap` with per-process-randomised
        // iteration order, exactly as `cross_affiliation_warnings` sorts.
        let allowed = self.extension_reach(Some(admitted)).await.allowed;
        let mut enabled_extensions: Vec<String> = {
            let extensions = self.extensions.lock().await;
            allowed
                .into_iter()
                .filter(|name| {
                    extensions
                        .get(name)
                        .is_some_and(|extension| !extension.config.is_capability())
                })
                .collect()
        };
        enabled_extensions.sort();

        // Build output string
        if !disabled_extensions.is_empty() {
            output_parts.push(format!(
                "Extensions disabled in the config:\n{}\n",
                disabled_extensions.join("\n")
            ));
        }

        if !detached_extensions.is_empty() {
            // ⚠ The hedge is not decoration. The Settings permission editor
            // rebuilds an extension with a remove-then-add, and a session-start
            // spawn can fail — both leave an extension in exactly this state
            // while being unable to attach.
            output_parts.push(format!(
                "Installed but not attached to this chat (attach with manage_extensions; \
                 an attach can still fail if the extension is broken):\n{}\n",
                detached_extensions.join("\n")
            ));
        }

        if disabled_extensions.is_empty() && detached_extensions.is_empty() {
            // The old copy said "No extensions available to enable", which was
            // false on two counts: it ignored the detached bucket, and it is
            // no longer true in general now that the marketplace can supply one.
            output_parts.push(
                "No installed extensions are currently detached or disabled; use \
                 search_marketplace_extensions to find one to install.\n"
                    .to_string(),
            );
        }

        if !enabled_extensions.is_empty() {
            output_parts.push(format!(
                "\n\nExtensions available to disable:\n{}\n",
                enabled_extensions
                    .iter()
                    .map(|name| format!("- {}", name))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        } else {
            output_parts.push("No extensions that can be disabled.\n".to_string());
        }

        Ok(vec![Content::text(output_parts.join("\n"))])
    }

    async fn get_server_client(&self, name: impl Into<String>) -> Option<McpClientBox> {
        self.extensions
            .lock()
            .await
            .get(&name.into())
            .map(|ext| ext.get_client())
    }

    pub async fn collect_moim(
        &self,
        session_id: &str,
        working_dir: &std::path::Path,
    ) -> Option<String> {
        // Use minute-level granularity to prevent conversation changes every second
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:00").to_string();
        let mut content = format!(
            "{MOIM_OPEN_TAG}\nIt is currently {}\nWorking directory: {}\n",
            timestamp,
            working_dir.display()
        );

        // BR-1: give the model a bounded, gitignore-aware map of the workspace so
        // it doesn't rediscover project structure from scratch every session. The
        // map is cached and token-capped inside `workspace_summary`.
        if let Some(map) = crate::agents::workspace_summary::workspace_summary(working_dir) {
            content.push('\n');
            content.push_str(&map);
            content.push('\n');
        }

        let platform_clients: Vec<(String, ExtensionConfig, McpClientBox)> = {
            let extensions = self.extensions.lock().await;
            extensions
                .iter()
                .filter_map(|(name, extension)| {
                    if let ExtensionConfig::Platform { .. } = &extension.config {
                        Some((
                            name.clone(),
                            extension.config.clone(),
                            extension.get_client(),
                        ))
                    } else {
                        None
                    }
                })
                .collect()
        };

        for (name, config, client) in platform_clients {
            if !platform_moim_allowed(&name, &config) {
                continue;
            }
            let client_guard = &*client;
            if let Some(moim_content) = client_guard.get_moim(session_id).await {
                tracing::debug!("MOIM content from {}: {} chars", name, moim_content.len());
                content.push('\n');
                content.push_str(&moim_content);
            }
        }

        content.push('\n');
        content.push_str(MOIM_CLOSE_TAG);

        Some(content)
    }
}

fn platform_moim_allowed(name: &str, config: &ExtensionConfig) -> bool {
    name != crate::agents::workspace_extension::EXTENSION_NAME
        || config.is_tool_available("workspace_read_panel")
}

/// Label appended to every **operator**-disabled entry in
/// `search_available_extensions` output (#42): these extensions were turned
/// off by the operator, so the model must not treat the listing as an
/// invitation to enable them on its own (`manage_extensions` refuses anyway —
/// see `extension_manager_extension::check_enable_allowed`).
pub(crate) const CONFIG_DISABLED_LABEL: &str = "(disabled by user; do not enable without asking)";

/// One listing line per config-disabled extension. Only entries the operator
/// actually wrote into the config file (`persisted`, keyed by
/// `config.name()`) carry [`CONFIG_DISABLED_LABEL`]. Builtin and platform
/// capabilities are excluded before labeling. Pure so the behavior is
/// unit-testable without a global config.
///
/// ⚠ **Issue #56 Gate E: an entry `cap` may not see is dropped before it is
/// ever labelled.** This half of `search_available_extensions` reads the config
/// rather than `self.extensions`, so [`ExtensionManager::extension_reach`] —
/// which iterates installed extensions — cannot decide it. The predicate is
/// therefore restated here, and it is restated *exactly*: `privacy_refusal`
/// under `cap.enforced()`, which is the same pair `extension_reach` applies and
/// the same pair Gate C dispatches on. Anything else here would be a fourth
/// rule about one question.
///
/// The classification is resolved with `Some(config)` for the reason Task 43
/// landed: a private extension renamed by hand in `config.yaml` reads Public by
/// name alone, and only its `--directory` argument still points at the install.
/// Passing the config can raise the answer, never lower it.
/// The config-disabled bucket alone, for the tests and callers that only ever
/// cared about that half.
#[cfg(test)]
fn config_disabled_extension_lines(
    entries: &[crate::config::ExtensionEntry],
    persisted: &std::collections::HashSet<String>,
    cap: crate::privacy::CallCapability,
) -> Vec<String> {
    extension_listing_lines(entries, persisted, &std::collections::HashSet::new(), cap).0
}

/// The two buckets `search_available_extensions` offers the model, from ONE
/// visibility chain.
///
/// Splitting them meant the detached bucket could have been built straight off
/// `get_all_extensions()`. It is not, and that is the point of the shared
/// chain: the capability exclusion and the `privacy_refusal`-under-`enforced`
/// predicate below are Gate E's verdict for this surface, and a second listing
/// that skipped them would be a fresh privacy leak in a cleanup commit.
///
/// ⚠ **The label is parameterised, not shared.** `CONFIG_DISABLED_LABEL` says
/// the operator switched this off deliberately and the model should ask before
/// enabling it. Stamping that on the *detached* bucket would tell the model not
/// to do the one thing that bucket exists to invite — a detached extension is
/// installed and enabled in the config, and simply is not attached to this
/// chat.
fn extension_listing_lines(
    entries: &[crate::config::ExtensionEntry],
    persisted: &std::collections::HashSet<String>,
    live_keys: &std::collections::HashSet<String>,
    cap: crate::privacy::CallCapability,
) -> (Vec<String>, Vec<String>) {
    let visible: Vec<&crate::config::ExtensionEntry> = entries
        .iter()
        .filter(|extension| !extension.config.is_capability())
        .filter(|extension| {
            if !cap.enforced() {
                // DR-15's master opt-out, read off the capability rather than
                // the process global so this cannot observe a different instant
                // from the tier beside it.
                return true;
            }
            let name = extension.config.name();
            let class = crate::privacy::resolve_extension(&name, Some(&extension.config));
            crate::privacy::refusal::privacy_refusal(&name, class.tier, cap.tier()).is_none()
        })
        .collect();

    let describe = |extension: &crate::config::ExtensionEntry| -> String {
        let config = &extension.config;
        let description = match config {
            ExtensionConfig::Builtin {
                description,
                display_name,
                ..
            } => {
                if description.is_empty() {
                    display_name.as_deref().unwrap_or("Built-in extension")
                } else {
                    description
                }
            }
            ExtensionConfig::Sse { .. } => "SSE extension (unsupported)",
            ExtensionConfig::Platform { description, .. }
            | ExtensionConfig::StreamableHttp { description, .. }
            | ExtensionConfig::Stdio { description, .. }
            | ExtensionConfig::Frontend { description, .. }
            | ExtensionConfig::InlinePython { description, .. } => description,
        };
        format!("- {} - {}", config.name(), description)
    };

    let disabled = visible
        .iter()
        .filter(|extension| !extension.enabled)
        .map(|extension| {
            let line = describe(extension);
            if persisted.contains(&extension.config.name()) {
                format!("{line} {CONFIG_DISABLED_LABEL}")
            } else {
                line
            }
        })
        .collect();

    // ⚠ `normalize(&config.key())` and nothing else. That is exactly what
    // `add_extension_with_origin` keys the live map by; `config::name_to_key`
    // is a DIFFERENT reduction, and comparing against it would report every
    // extension whose name carries whitespace or punctuation as permanently
    // detached.
    let detached = visible
        .iter()
        .filter(|extension| extension.enabled)
        .filter(|extension| !live_keys.contains(&normalize(&extension.config.key())))
        .map(|extension| describe(extension))
        .collect();

    (disabled, detached)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::CallToolResult;
    use rmcp::model::{InitializeResult, JsonObject};
    use rmcp::{object, ServiceError as Error};

    use rmcp::model::ListPromptsResult;
    use rmcp::model::ListResourcesResult;
    use rmcp::model::ListToolsResult;
    use rmcp::model::ReadResourceResult;
    use rmcp::model::ServerNotification;

    use tokio::sync::mpsc;

    impl ExtensionManager {
        async fn add_mock_extension(&self, name: String, client: McpClientBox) {
            self.add_mock_extension_with_tools(name, client, vec![])
                .await;
        }

        async fn add_mock_extension_with_tools(
            &self,
            name: String,
            client: McpClientBox,
            available_tools: Vec<String>,
        ) {
            let sanitized_name = normalize(&name);
            let config = ExtensionConfig::Builtin {
                name: name.clone(),
                display_name: Some(name.clone()),
                description: "built-in".to_string(),
                timeout: None,
                bundled: None,
                available_tools,
            };
            // Through the real admission point, so a mock is keyed by the same
            // rule a real extension is (issue #56) and the gates resolve its
            // tier the same way, instead of carrying one hardcoded here.
            self.add_client(sanitized_name, config, client, None, None)
                .await;
        }

        async fn add_mock_third_party_extension(&self, name: &str, client: McpClientBox) {
            let config = ExtensionConfig::stdio(name, "mock-command", "mock extension", 30_u64);
            self.add_client(name.to_string(), config, client, None, None)
                .await;
        }
    }

    struct MockClient {}

    #[async_trait::async_trait]
    impl McpClientTrait for MockClient {
        fn get_info(&self) -> Option<&InitializeResult> {
            None
        }

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
            Err(Error::TransportClosed)
        }

        async fn list_tools(
            &self,
            _next_cursor: Option<String>,
            _cancellation_token: CancellationToken,
        ) -> Result<ListToolsResult, Error> {
            use serde_json::json;
            use std::sync::Arc;
            Ok(ListToolsResult {
                tools: vec![
                    Tool::new(
                        "tool".to_string(),
                        "A basic tool".to_string(),
                        Arc::new(json!({}).as_object().unwrap().clone()),
                    ),
                    Tool::new(
                        "available_tool".to_string(),
                        "An available tool".to_string(),
                        Arc::new(json!({}).as_object().unwrap().clone()),
                    ),
                    Tool::new(
                        "hidden_tool".to_string(),
                        "hidden tool".to_string(),
                        Arc::new(json!({}).as_object().unwrap().clone()),
                    ),
                ],
                next_cursor: None,
                meta: None,
            })
        }

        async fn call_tool(
            &self,
            name: &str,
            _arguments: Option<JsonObject>,
            _meta: McpMeta,
            _cancellation_token: CancellationToken,
        ) -> Result<CallToolResult, Error> {
            match name {
                "tool" | "test__tool" | "available_tool" | "hidden_tool" => Ok(CallToolResult {
                    content: vec![],
                    is_error: None,
                    structured_content: None,
                    meta: None,
                }),
                _ => Err(Error::TransportClosed),
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
    }

    /// H6 regression guard. A client whose `call_tool` sleeps, so that N
    /// concurrent dispatches against the SAME extension take `max(durations)`
    /// when they truly run in parallel and `sum(durations)` when something
    /// serializes them.
    struct SlowMockClient {
        delay: std::time::Duration,
    }

    #[async_trait::async_trait]
    impl McpClientTrait for SlowMockClient {
        fn get_info(&self) -> Option<&InitializeResult> {
            None
        }

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
            Err(Error::TransportClosed)
        }

        async fn list_tools(
            &self,
            _next_cursor: Option<String>,
            _cancellation_token: CancellationToken,
        ) -> Result<ListToolsResult, Error> {
            Ok(ListToolsResult {
                tools: vec![],
                next_cursor: None,
                meta: None,
            })
        }

        async fn call_tool(
            &self,
            _name: &str,
            _arguments: Option<JsonObject>,
            _meta: McpMeta,
            _cancellation_token: CancellationToken,
        ) -> Result<CallToolResult, Error> {
            tokio::time::sleep(self.delay).await;
            Ok(CallToolResult {
                content: vec![],
                is_error: None,
                structured_content: None,
                meta: None,
            })
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
    }

    /// H6: three tool calls on ONE extension client must overlap.
    ///
    /// Before the redundant `McpClientBox` mutex was removed, `dispatch_tool_call`
    /// held the client guard across the whole `call_tool` await, turning
    /// `max(400ms, 400ms, 400ms)` into `sum(...)` = ~1200ms. This test fails
    /// (~1.2s elapsed) with the mutex in place and passes (~0.4s) without it.
    #[tokio::test]
    async fn h6_parallel_same_extension() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        extension_manager
            .add_mock_extension(
                "slow".to_string(),
                Arc::new(SlowMockClient {
                    delay: std::time::Duration::from_millis(400),
                }),
            )
            .await;

        // Dispatch all three first: `register_dispatch` runs on the dispatch
        // loop, the actual `call_tool` runs inside the returned future.
        let mut futures = Vec::new();
        for _ in 0..3 {
            let tool_call = CallToolRequestParams {
                task: None,
                name: "slow__tool".to_string().into(),
                arguments: Some(object!({})),
                meta: None,
            };
            let dispatched = extension_manager
                .dispatch_tool_call(
                    "test-session-id",
                    tool_call,
                    crate::privacy::CallCapability::for_test_restricted(),
                    CancellationToken::default(),
                )
                .await
                .expect("dispatch should succeed");
            futures.push(dispatched.result);
        }

        let start = std::time::Instant::now();
        let results = futures::future::join_all(futures).await;
        let elapsed = start.elapsed();

        for result in results {
            assert!(result.is_ok(), "each concurrent call should succeed");
        }

        assert!(
            elapsed < std::time::Duration::from_millis(700),
            "3 concurrent calls on one extension took {elapsed:?}; expected ~400ms. \
             They are being serialized on a client-wide mutex (H6)."
        );
    }

    #[tokio::test]
    async fn test_get_client_for_tool() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        // Add some mock clients using the helper method
        extension_manager
            .add_mock_extension("test_client".to_string(), Arc::new(MockClient {}))
            .await;

        extension_manager
            .add_mock_extension("__client".to_string(), Arc::new(MockClient {}))
            .await;

        extension_manager
            .add_mock_extension("__cli__ent__".to_string(), Arc::new(MockClient {}))
            .await;

        extension_manager
            .add_mock_extension("client 🚀".to_string(), Arc::new(MockClient {}))
            .await;

        // Test basic case
        assert!(extension_manager
            .get_client_for_tool("test_client__tool")
            .await
            .is_some());

        // Test leading underscores
        assert!(extension_manager
            .get_client_for_tool("__client__tool")
            .await
            .is_some());

        // Test multiple underscores in client name, and ending with __
        assert!(extension_manager
            .get_client_for_tool("__cli__ent____tool")
            .await
            .is_some());

        // Test unicode in tool name, "client 🚀" should become "client_"
        assert!(extension_manager
            .get_client_for_tool("client___tool")
            .await
            .is_some());
    }

    /// BRSDK INTEGRATED end-to-end: a per-app `DataSqlServer` injected via the
    /// `add_inprocess_server` seam is discoverable as an MCP tool and dispatches
    /// real read-only SQL through the actual MCP transport — exercising the full
    /// path (manifest source → per-app server → in-process MCP → tool discovery
    /// → dispatch → real rows), plus end-to-end mutation rejection.
    #[tokio::test]
    async fn brsdk_inprocess_datasql_end_to_end() {
        use biorouter_mcp::datasql::server::DataSqlServer;
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cohort.db");
        {
            let opts = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .unwrap();
            sqlx::query("CREATE TABLE genes (symbol TEXT, chrom TEXT)")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO genes VALUES ('CFTR','7'), ('TP53','17')")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;
        }

        let em = ExtensionManager::new_without_provider(dir.path().to_path_buf());
        let mut sources = std::collections::HashMap::new();
        sources.insert("cohort".to_string(), db_path);
        em.add_inprocess_server("datasql", DataSqlServer::new(sources))
            .await
            .expect("inject per-app server");

        // (1) The tool is discoverable over the in-process MCP transport.
        let tools = em.get_prefixed_tools(None).await.unwrap();
        assert!(
            tools.iter().any(|t| t.name.contains("data_query")),
            "data_query not discovered; tools = {:?}",
            tools.iter().map(|t| t.name.to_string()).collect::<Vec<_>>()
        );

        // (2) Dispatch a real read-only query and read the rows back.
        let call = CallToolRequestParams {
            task: None,
            name: "datasql__data_query".to_string().into(),
            arguments: Some(
                serde_json::json!({"source":"cohort","sql":"SELECT symbol FROM genes WHERE chrom='7'"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            meta: None,
        };
        let dispatched = em
            .dispatch_tool_call(
                "test-session",
                call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await
            .expect("dispatch ok");
        let output = dispatched.result.await.expect("tool result ok");
        let text = serde_json::to_string(&output).unwrap();
        assert!(
            text.contains("CFTR"),
            "expected query rows in output: {text}"
        );
        assert!(!text.contains("TP53"), "WHERE filter must apply: {text}");

        // (2b) The in-process server's synthetic config is EXCLUDED from
        // get_extension_configs — so it's never persisted/replayed/propagated as
        // an un-spawnable "datasql" builtin.
        let configs = em.get_extension_configs().await;
        assert!(
            !configs.iter().any(|c| c.name() == "datasql"),
            "in-process datasql config must be excluded from exported configs"
        );

        // (2c) Re-injection is idempotent (configure_agent re-runs per connect).
        let before = em.get_prefixed_tools(None).await.unwrap().len();
        em.add_inprocess_server(
            "datasql",
            DataSqlServer::new(std::collections::HashMap::new()),
        )
        .await
        .expect("idempotent re-inject");
        let after = em.get_prefixed_tools(None).await.unwrap().len();
        assert_eq!(before, after, "re-injecting an existing server is a no-op");

        // (3) A mutation is rejected end-to-end (whichever way the error surfaces).
        let bad = CallToolRequestParams {
            task: None,
            name: "datasql__data_query".to_string().into(),
            arguments: Some(
                serde_json::json!({"source":"cohort","sql":"DROP TABLE genes"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            meta: None,
        };
        let rejected = match em
            .dispatch_tool_call(
                "test-session",
                bad,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await
        {
            Err(_) => true,
            Ok(tcr) => match tcr.result.await {
                Err(_) => true,
                Ok(r) => {
                    r.is_error == Some(true)
                        || serde_json::to_string(&r)
                            .unwrap()
                            .to_lowercase()
                            .contains("read-only")
                }
            },
        };
        assert!(rejected, "mutation must be rejected end-to-end");
    }

    #[tokio::test]
    async fn test_dispatch_tool_call() {
        // test that dispatch_tool_call parses out the sanitized name correctly, and extracts
        // tool_names
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        // Add some mock clients using the helper method
        extension_manager
            .add_mock_extension("test_client".to_string(), Arc::new(MockClient {}))
            .await;

        extension_manager
            .add_mock_extension("__cli__ent__".to_string(), Arc::new(MockClient {}))
            .await;

        extension_manager
            .add_mock_extension("client 🚀".to_string(), Arc::new(MockClient {}))
            .await;

        // verify a normal tool call
        let tool_call = CallToolRequestParams {
            task: None,
            name: "test_client__tool".to_string().into(),
            arguments: Some(object!({})),
            meta: None,
        };

        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                tool_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await;
        assert!(result.is_ok());

        let tool_call = CallToolRequestParams {
            task: None,
            name: "test_client__test__tool".to_string().into(),
            arguments: Some(object!({})),
            meta: None,
        };

        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                tool_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await;
        assert!(result.is_ok());

        // verify a multiple underscores dispatch
        let tool_call = CallToolRequestParams {
            task: None,
            name: "__cli__ent____tool".to_string().into(),
            arguments: Some(object!({})),
            meta: None,
        };

        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                tool_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await;
        assert!(result.is_ok());

        // Test unicode in tool name, "client 🚀" should become "client_"
        let tool_call = CallToolRequestParams {
            task: None,
            name: "client___tool".to_string().into(),
            arguments: Some(object!({})),
            meta: None,
        };

        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                tool_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await;
        assert!(result.is_ok());

        let tool_call = CallToolRequestParams {
            task: None,
            name: "client___test__tool".to_string().into(),
            arguments: Some(object!({})),
            meta: None,
        };

        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                tool_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await;
        assert!(result.is_ok());

        // this should error out, specifically for an ToolError::ExecutionError
        let invalid_tool_call = CallToolRequestParams {
            task: None,
            name: "client___tools".to_string().into(),
            arguments: Some(object!({})),
            meta: None,
        };

        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                invalid_tool_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await
            .unwrap()
            .result
            .await;
        assert!(matches!(
            result,
            Err(ErrorData {
                code: ErrorCode::INTERNAL_ERROR,
                ..
            })
        ));

        // this should error out, specifically with an ToolError::NotFound
        // this client doesn't exist
        let invalid_tool_call = CallToolRequestParams {
            task: None,
            name: "_client__tools".to_string().into(),
            arguments: Some(object!({})),
            meta: None,
        };

        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                invalid_tool_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await;
        if let Err(err) = result {
            let tool_err = err.downcast_ref::<ErrorData>().expect("Expected ErrorData");
            assert_eq!(tool_err.code, ErrorCode::RESOURCE_NOT_FOUND);
        } else {
            panic!("Expected ErrorData with ErrorCode::RESOURCE_NOT_FOUND");
        }
    }

    // BR-23: the central secret-redaction boundary must block a tool call that
    // references an existing secret file, no matter which extension owns the tool.
    #[tokio::test]
    async fn test_dispatch_blocks_secret_file_access() {
        let temp_dir = tempfile::tempdir().unwrap();
        std::fs::write(temp_dir.path().join(".env"), "SECRET=1").unwrap();

        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        extension_manager
            .set_working_dir(temp_dir.path().to_path_buf())
            .await;
        extension_manager
            .add_mock_extension("test_client".to_string(), Arc::new(MockClient {}))
            .await;

        // A tool from an arbitrary extension that reads the existing .env is denied
        // at dispatch, before the tool ever runs.
        let secret_call = CallToolRequestParams {
            task: None,
            name: "test_client__tool".to_string().into(),
            arguments: Some(object!({"path": ".env"})),
            meta: None,
        };
        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                secret_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await;
        match result {
            Err(err) => {
                let tool_err = err.downcast_ref::<ErrorData>().expect("Expected ErrorData");
                assert_eq!(tool_err.code, ErrorCode::INVALID_PARAMS);
            }
            Ok(_) => panic!("expected the secret-file access to be blocked at dispatch"),
        }

        // A benign, non-existent path is not blocked.
        let benign_call = CallToolRequestParams {
            task: None,
            name: "test_client__tool".to_string().into(),
            arguments: Some(object!({"path": "notes.txt"})),
            meta: None,
        };
        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                benign_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await;
        assert!(result.is_ok(), "benign path must not be blocked");
    }

    #[tokio::test]
    async fn test_tool_availability_filtering() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        // Only "available_tool" should be available to the LLM
        let available_tools = vec!["available_tool".to_string()];

        extension_manager
            .add_mock_extension_with_tools(
                "test_extension".to_string(),
                Arc::new(MockClient {}),
                available_tools,
            )
            .await;

        let tools = extension_manager.get_prefixed_tools(None).await.unwrap();

        let tool_names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        assert!(!tool_names.iter().any(|name| name == "test_extension__tool")); // Default unavailable
        assert!(tool_names
            .iter()
            .any(|name| name == "test_extension__available_tool"));
        assert!(!tool_names
            .iter()
            .any(|name| name == "test_extension__hidden_tool"));
        assert!(tool_names.len() == 1);
    }

    #[tokio::test]
    async fn test_tool_availability_defaults_to_available() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        extension_manager
            .add_mock_extension_with_tools(
                "test_extension".to_string(),
                Arc::new(MockClient {}),
                vec![], // Empty available_tools means all tools are available by default
            )
            .await;

        let tools = extension_manager.get_prefixed_tools(None).await.unwrap();

        let tool_names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        assert!(tool_names.iter().any(|name| name == "test_extension__tool"));
        assert!(tool_names
            .iter()
            .any(|name| name == "test_extension__available_tool"));
        assert!(tool_names
            .iter()
            .any(|name| name == "test_extension__hidden_tool"));
        assert!(tool_names.len() == 3);
    }

    #[tokio::test]
    async fn test_dispatch_unavailable_tool_returns_error() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        let available_tools = vec!["available_tool".to_string()];

        extension_manager
            .add_mock_extension_with_tools(
                "test_extension".to_string(),
                Arc::new(MockClient {}),
                available_tools,
            )
            .await;

        // Try to call an unavailable tool
        let unavailable_tool_call = CallToolRequestParams {
            task: None,
            name: "test_extension__tool".to_string().into(),
            arguments: Some(object!({})),
            meta: None,
        };

        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                unavailable_tool_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await;

        // Should return RESOURCE_NOT_FOUND error
        if let Err(err) = result {
            let tool_err = err.downcast_ref::<ErrorData>().expect("Expected ErrorData");
            assert_eq!(tool_err.code, ErrorCode::RESOURCE_NOT_FOUND);
            assert!(tool_err.message.contains("is not available"));
        } else {
            panic!("Expected ErrorData with ErrorCode::RESOURCE_NOT_FOUND");
        }

        // Try to call an available tool - should succeed
        let available_tool_call = CallToolRequestParams {
            task: None,
            name: "test_extension__available_tool".to_string().into(),
            arguments: Some(object!({})),
            meta: None,
        };

        let result = extension_manager
            .dispatch_tool_call(
                "test-session-id",
                available_tool_call,
                crate::privacy::CallCapability::for_test_restricted(),
                CancellationToken::default(),
            )
            .await;

        assert!(result.is_ok());
    }

    /// The authorization input must travel WITH the client, out of one snapshot
    /// of the map.
    ///
    /// `dispatch_tool_call` used to resolve the client under one lock and then
    /// take a second lock to read the entry's `available_tools` — inside an
    /// `if let Some(extension) = …get(&client_name)`, so when that second
    /// lookup missed, the check was not failed but SKIPPED, and the client
    /// resolved a moment earlier went on to execute. Any removal landing in
    /// that window turned a forbidden tool into an authorized one, and removals
    /// are ordinary: disabling an extension in Settings, `manage_extensions
    /// disable`, and (BR-71) an explicit enable displacing an auto-injection.
    ///
    /// With the config resolved alongside the client there is no second lookup
    /// to miss: an entry that is gone at resolve time already answers "not
    /// found", and one that was present is judged by its own config. The
    /// interleaving is unrepresentable rather than unlikely, which is why this
    /// pins the contract instead of racing a barrier against it.
    #[tokio::test]
    async fn dispatch_authorization_is_resolved_with_the_client_not_looked_up_again() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        extension_manager
            .add_mock_extension_with_tools(
                "guarded".to_string(),
                Arc::new(MockClient {}),
                vec!["allowed".to_string()],
            )
            .await;

        let (name, _client, config, class, _origin) = extension_manager
            .get_client_for_tool("guarded__forbidden")
            .await
            .expect("the extension resolves");
        assert_eq!(name, "guarded");
        assert!(
            !config.is_tool_available("forbidden"),
            "the resolved config is the one the dispatch must be judged by"
        );
        assert!(config.is_tool_available("allowed"));
        // Issue #56: the privacy classification rides the SAME snapshot, for the
        // same reason — Gate C must judge the record this call was routed to.
        // Task 48 put the AFFILIATION in that snapshot too, on the same terms:
        // resolving it separately would let the two axes disagree about one
        // entry.
        //
        // Resolved from the returned (name, config) PAIR, not from the literal
        // "guarded" this test happens to know: Task 43 (DR-23) made the answer a
        // function of the config as well as the key, and comparing against a
        // name-only classification would pass on an implementation that had
        // dropped the config half — which is exactly the rename bug.
        assert_eq!(
            class,
            crate::privacy::resolve_extension(&name, Some(&config))
        );

        // And once it is gone, resolution itself fails — a removal can only
        // ever deny, never skip.
        extension_manager.remove_extension("guarded").await.unwrap();
        assert!(
            extension_manager
                .get_client_for_tool("guarded__forbidden")
                .await
                .is_none(),
            "a disappeared extension must deny, not fall through"
        );
    }

    #[tokio::test]
    async fn test_streamable_http_header_env_substitution() {
        let mut env_map = HashMap::new();
        env_map.insert("AUTH_TOKEN".to_string(), "secret123".to_string());
        env_map.insert("API_KEY".to_string(), "key456".to_string());

        // Test ${VAR} syntax
        let result = substitute_env_vars("Bearer ${ AUTH_TOKEN }", &env_map);
        assert_eq!(result, "Bearer secret123");

        // Test ${VAR} syntax without spaces
        let result = substitute_env_vars("Bearer ${AUTH_TOKEN}", &env_map);
        assert_eq!(result, "Bearer secret123");

        // Test $VAR syntax
        let result = substitute_env_vars("Bearer $AUTH_TOKEN", &env_map);
        assert_eq!(result, "Bearer secret123");

        // Test multiple substitutions
        let result = substitute_env_vars("Key: $API_KEY, Token: ${AUTH_TOKEN}", &env_map);
        assert_eq!(result, "Key: key456, Token: secret123");

        // Test no substitution when variable doesn't exist
        let result = substitute_env_vars("Bearer ${UNKNOWN_VAR}", &env_map);
        assert_eq!(result, "Bearer ${UNKNOWN_VAR}");

        // Test mixed content
        let result = substitute_env_vars(
            "Authorization: Bearer ${AUTH_TOKEN} and API ${API_KEY}",
            &env_map,
        );
        assert_eq!(result, "Authorization: Bearer secret123 and API key456");
    }

    mod generate_extension_name_tests {
        use super::*;
        use rmcp::model::Implementation;
        use test_case::test_case;

        fn make_info(name: &str) -> ServerInfo {
            ServerInfo {
                server_info: Implementation {
                    name: name.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        }

        #[test_case(Some("kiwi-mcp-server"), None, "^kiwi-mcp-server$" ; "already normalized server name")]
        #[test_case(Some("Context7"), None, "^context7$" ; "mixed case normalized")]
        #[test_case(Some("@huggingface/mcp-services"), None, "^_huggingface_mcp-services$" ; "special chars normalized")]
        #[test_case(None, None, "^unnamed$" ; "no server info falls back")]
        #[test_case(Some(""), None, "^unnamed$" ; "empty server name falls back")]
        #[test_case(Some("github-mcp-server"), Some("github-mcp-server"), r"^github-mcp-server_[A-Za-z0-9]{6}$" ; "duplicate adds suffix")]
        fn test_generate_name(server_name: Option<&str>, collision: Option<&str>, expected: &str) {
            let info = server_name.map(make_info);
            let result = generate_extension_name(info.as_ref(), |n| collision == Some(n));
            let re = regex::Regex::new(expected).unwrap();
            assert!(re.is_match(&result));
        }
    }

    #[tokio::test]
    async fn test_collect_moim_uses_minute_granularity() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let working_dir = std::path::Path::new("/tmp");

        if let Some(moim) = em.collect_moim("test-session-id", working_dir).await {
            // Timestamp should end with :00 (seconds fixed to 00)
            assert!(
                moim.contains(":00\n"),
                "Timestamp should use minute granularity"
            );
        }
    }

    #[test]
    fn restricted_workspace_does_not_inject_panel_moim() {
        let restricted = ExtensionConfig::Platform {
            name: crate::agents::workspace_extension::EXTENSION_NAME.to_string(),
            description: "delegation only".to_string(),
            bundled: Some(true),
            available_tools: vec!["workspace_list".to_string(), "subagent".to_string()],
        };
        assert!(!platform_moim_allowed(
            crate::agents::workspace_extension::EXTENSION_NAME,
            &restricted
        ));

        let full = ExtensionConfig::Platform {
            name: crate::agents::workspace_extension::EXTENSION_NAME.to_string(),
            description: "delegation and panel control".to_string(),
            bundled: Some(true),
            available_tools: vec![],
        };
        assert!(platform_moim_allowed(
            crate::agents::workspace_extension::EXTENSION_NAME,
            &full
        ));
    }

    #[tokio::test]
    async fn test_tools_cache_invalidated_on_add_extension() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        extension_manager
            .add_mock_extension("ext_a".to_string(), Arc::new(MockClient {}))
            .await;

        let tools_after_first = extension_manager.get_prefixed_tools(None).await.unwrap();
        let tool_names: Vec<String> = tools_after_first
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(tool_names.iter().any(|n| n.starts_with("ext_a__")));
        assert!(!tool_names.iter().any(|n| n.starts_with("ext_b__")));

        extension_manager
            .add_mock_extension("ext_b".to_string(), Arc::new(MockClient {}))
            .await;

        let tools_after_second = extension_manager.get_prefixed_tools(None).await.unwrap();
        let tool_names: Vec<String> = tools_after_second
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(tool_names.iter().any(|n| n.starts_with("ext_a__")));
        assert!(tool_names.iter().any(|n| n.starts_with("ext_b__")));
    }

    #[tokio::test]
    async fn test_tools_cache_invalidated_on_remove_extension() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        extension_manager
            .add_mock_extension("ext_a".to_string(), Arc::new(MockClient {}))
            .await;
        extension_manager
            .add_mock_extension("ext_b".to_string(), Arc::new(MockClient {}))
            .await;

        let tools_before = extension_manager.get_prefixed_tools(None).await.unwrap();
        let tool_names: Vec<String> = tools_before.iter().map(|t| t.name.to_string()).collect();
        assert!(tool_names.iter().any(|n| n.starts_with("ext_a__")));
        assert!(tool_names.iter().any(|n| n.starts_with("ext_b__")));

        extension_manager.remove_extension("ext_b").await.unwrap();

        let tools_after = extension_manager.get_prefixed_tools(None).await.unwrap();
        let tool_names: Vec<String> = tools_after.iter().map(|t| t.name.to_string()).collect();
        assert!(tool_names.iter().any(|n| n.starts_with("ext_a__")));
        assert!(!tool_names.iter().any(|n| n.starts_with("ext_b__")));
    }

    #[tokio::test]
    async fn test_get_prefixed_tools_excluding() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        extension_manager
            .add_mock_extension("ext_a".to_string(), Arc::new(MockClient {}))
            .await;
        extension_manager
            .add_mock_extension("ext_b".to_string(), Arc::new(MockClient {}))
            .await;

        let tools = extension_manager
            .get_prefixed_tools_excluding("ext_a", None)
            .await
            .unwrap();
        let tool_names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();

        assert!(!tool_names.iter().any(|n| n.starts_with("ext_a__")));
        assert!(tool_names.iter().any(|n| n.starts_with("ext_b__")));
    }

    #[tokio::test]
    async fn test_get_prefixed_tools_by_extension_name() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        extension_manager
            .add_mock_extension("ext_a".to_string(), Arc::new(MockClient {}))
            .await;
        extension_manager
            .add_mock_extension("ext_b".to_string(), Arc::new(MockClient {}))
            .await;

        let tools = extension_manager
            .get_prefixed_tools(Some("ext_a".to_string()))
            .await
            .unwrap();
        let tool_names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();

        assert!(tool_names.iter().any(|n| n.starts_with("ext_a__")));
        assert!(!tool_names.iter().any(|n| n.starts_with("ext_b__")));
    }

    #[tokio::test]
    async fn get_extensions_info_classifies_capabilities_and_extensions() {
        let temp_dir = tempfile::tempdir().unwrap();
        let extension_manager =
            ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        extension_manager
            .add_mock_extension("developer".to_string(), Arc::new(MockClient {}))
            .await;
        extension_manager
            .add_mock_third_party_extension("custom", Arc::new(MockClient {}))
            .await;

        let info = extension_manager.get_extensions_info().await;
        let developer = info
            .iter()
            .find(|entry| entry.name == "developer")
            .expect("Developer is attached");
        let custom = info
            .iter()
            .find(|entry| entry.name == "custom")
            .expect("custom extension is attached");
        assert_eq!(
            developer.classification,
            ExtensionClassification::Capability
        );
        assert_eq!(custom.classification, ExtensionClassification::Extension);
    }

    // #42 hardening: operator-disabled third-party extensions in
    // search_available_extensions output must be labeled. Capabilities use the
    // capability settings surface instead and are excluded entirely.
    #[test]
    fn config_disabled_lines_label_every_disabled_entry_and_skip_enabled_ones() {
        use crate::config::ExtensionEntry;

        let entries = vec![
            ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::Builtin {
                    name: "developer".to_string(),
                    display_name: Some("Developer".to_string()),
                    description: String::new(),
                    timeout: None,
                    bundled: Some(true),
                    available_tools: vec![],
                },
            },
            ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::stdio("running", "cmd", "An enabled one", 30_u64),
            },
            ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::stdio("custom", "cmd", "A custom server", 30_u64),
            },
        ];
        let persisted = std::collections::HashSet::from([
            "developer".to_string(),
            "running".to_string(),
            "custom".to_string(),
        ]);

        let lines = config_disabled_extension_lines(&entries, &persisted, a_private_caller());
        assert_eq!(
            lines.len(),
            1,
            "only disabled third-party extensions are listed"
        );
        assert!(
            lines.iter().all(|l| l.contains(CONFIG_DISABLED_LABEL)),
            "every operator-disabled entry must carry the label: {lines:?}"
        );
        assert!(
            lines[0].starts_with("- custom - A custom server "),
            "{}",
            lines[0]
        );
        assert!(
            !lines.iter().any(|l| l.contains("running")),
            "enabled extension leaked into the disabled list: {lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("developer")),
            "capability leaked into extension discovery: {lines:?}"
        );
    }

    // A default-off capability such as Chat Recall is not an extension-manager
    // discovery result. The third-party extension remains available and keeps
    // its operator-disabled provenance label.
    #[test]
    fn config_disabled_lines_exclude_capabilities() {
        use crate::config::ExtensionEntry;

        let entries = vec![
            ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::Platform {
                    name: "chatrecall".to_string(),
                    description: "Recall previous chats".to_string(),
                    bundled: Some(true),
                    available_tools: vec![],
                },
            },
            ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::stdio("custom", "cmd", "A custom server", 30_u64),
            },
        ];
        // Only `custom` was actually written to the config file.
        let persisted = std::collections::HashSet::from(["custom".to_string()]);

        let lines = config_disabled_extension_lines(&entries, &persisted, a_private_caller());
        assert_eq!(
            lines.len(),
            1,
            "only third-party extensions are discoverable"
        );
        assert!(
            !lines.iter().any(|line| line.contains("chatrecall")),
            "capabilities must not be presented as extensions: {lines:?}"
        );
        let custom = lines
            .iter()
            .find(|l| l.contains("custom"))
            .expect("persisted entry listed");
        assert!(
            custom.contains(CONFIG_DISABLED_LABEL),
            "operator-persisted enabled:false must be labeled: {custom}"
        );
    }

    #[test]
    fn config_disabled_lines_empty_when_everything_is_enabled() {
        use crate::config::ExtensionEntry;

        let entries = vec![ExtensionEntry {
            enabled: true,
            config: ExtensionConfig::default(),
        }];
        let persisted = std::collections::HashSet::new();
        assert!(
            config_disabled_extension_lines(&entries, &persisted, a_private_caller()).is_empty()
        );
        assert!(config_disabled_extension_lines(&[], &persisted, a_private_caller()).is_empty());
    }

    // ----------------------------------------------------------------------
    // Issue #56, finding 13. `search_available_extensions` printed a private
    // connector's NAME and its marketplace DESCRIPTION into a public model's
    // context while Gate E withheld the very same server's tools next door.
    //
    // The catalogue has two halves and they fail differently, so each gets its
    // own assertion: the "disabled in the config" half is built from the config
    // file by `config_disabled_extension_lines` (pure, tested directly), and the
    // "available to disable" half is built from the manager's installed set
    // (tested through the real `search_available_extensions`).
    // ----------------------------------------------------------------------

    /// A private-model caller with the feature on — the row every pre-#56 test
    /// of this function was implicitly written against, since none of them
    /// meant to assert anything about the tier axis.
    fn a_private_caller() -> crate::privacy::CallCapability {
        crate::privacy::CallCapability::for_test(crate::privacy::ProviderTier::Private, true)
    }

    /// A public-model caller with the feature on — the population Gate E
    /// withholds a private extension from.
    fn a_public_caller() -> crate::privacy::CallCapability {
        crate::privacy::CallCapability::for_test(crate::privacy::ProviderTier::Public, true)
    }

    /// One operator-disabled private connector and one operator-disabled public
    /// extension, so a filter that dropped everything is distinguishable from
    /// one that dropped the right thing.
    fn a_catalogue_of_one_private_and_one_public() -> Vec<crate::config::ExtensionEntry> {
        use crate::config::ExtensionEntry;
        vec![
            ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::stdio(
                    "ucsfomopagent",
                    "uv",
                    "Natural-language SQL over the UCSF OMOP de-identified clinical database",
                    30_u64,
                ),
            },
            ExtensionEntry {
                enabled: false,
                config: ExtensionConfig::stdio("custom", "cmd", "A custom server", 30_u64),
            },
        ]
    }

    /// F-20(a): `manage_extensions {disable}` detaches from the live map and
    /// never touches the config, so a detached extension appeared in NEITHER
    /// bucket — `search_available_extensions` answered "No extensions available
    /// to enable" while the extension sat installed and enabled on disk. The
    /// model could not learn the name it needed to attach it again.
    mod detached_bucket_tests {
        use super::*;

        fn an_enabled_third_party() -> Vec<crate::config::ExtensionEntry> {
            vec![crate::config::ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::stdio("custom", "cmd", "A custom server", 30_u64),
            }]
        }

        fn detached(entries: &[crate::config::ExtensionEntry], live: &[&str]) -> Vec<String> {
            let live_keys = live.iter().map(|k| (*k).to_string()).collect();
            extension_listing_lines(
                entries,
                &std::collections::HashSet::new(),
                &live_keys,
                a_private_caller(),
            )
            .1
        }

        #[test]
        fn a_detached_extension_is_listed_without_the_do_not_enable_label() {
            // Catches reusing the disabled renderer, which stamps "disabled by
            // the user; do not enable without asking" on the one bucket whose
            // whole purpose is to invite the model to re-attach.
            let lines = detached(&an_enabled_third_party(), &[]);
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert!(lines[0].contains("custom"));
            assert!(
                !lines[0].contains(CONFIG_DISABLED_LABEL),
                "the re-attach bucket must not tell the model not to re-attach: {lines:?}"
            );
        }

        #[test]
        fn an_attached_extension_is_not_listed_as_detached() {
            // Catches "detached bucket = every config-enabled entry", which
            // would list everything currently loaded.
            assert!(detached(&an_enabled_third_party(), &["custom"]).is_empty());
        }

        #[test]
        fn detachment_is_tested_with_the_live_maps_own_key() {
            // Catches comparing `config.name()` — or `config::name_to_key` —
            // against a map keyed by `normalize(config.key())`, which would
            // report every extension whose name carries whitespace or
            // punctuation as permanently detached.
            let entries = vec![crate::config::ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::stdio("My Agent", "cmd", "A custom server", 30_u64),
            }];
            let key = normalize(&entries[0].config.key());
            assert_ne!(
                key,
                entries[0].config.name(),
                "the fixture only discriminates if the two reductions differ"
            );
            assert!(detached(&entries, &[key.as_str()]).is_empty());
        }

        #[test]
        fn the_detached_bucket_hides_a_private_extension_from_a_public_model() {
            // Catches building the new bucket straight off `get_all_extensions()`
            // without the `privacy_refusal` predicate — a fresh Gate E leak in
            // a cleanup commit, of exactly the kind finding 13 closed.
            let entries = vec![crate::config::ExtensionEntry {
                enabled: true,
                config: ExtensionConfig::stdio(
                    "ucsfomopagent",
                    "uv",
                    "Natural-language SQL over the UCSF OMOP de-identified clinical database",
                    30_u64,
                ),
            }];
            assert_eq!(
                crate::privacy::resolve_extension("ucsfomopagent", None).tier,
                crate::privacy::ProviderTier::Private,
                "the fixture only discriminates if this name really is private"
            );

            let empty = std::collections::HashSet::new();
            let public = extension_listing_lines(&entries, &empty, &empty, a_public_caller()).1;
            assert!(
                public.is_empty(),
                "a private connector reached a public model's detached bucket: {public:?}"
            );
            let private = extension_listing_lines(&entries, &empty, &empty, a_private_caller()).1;
            assert_eq!(
                private.len(),
                1,
                "the private caller must still see it, or the filter is just an empty list"
            );
        }
    }

    /// **Finding 13, half one — the config catalogue.**
    ///
    /// ⚠ The description matters as much as the name. `ucsfomopagent`'s
    /// marketplace blurb says what institution's clinical database it reaches;
    /// leaking the pair tells a public model both that the connector exists here
    /// and what it is for.
    #[test]
    fn the_catalogue_hides_a_private_extension_from_a_public_caller() {
        let persisted =
            std::collections::HashSet::from(["ucsfomopagent".to_string(), "custom".to_string()]);
        let entries = a_catalogue_of_one_private_and_one_public();

        // The fixture only discriminates if this name really is private.
        assert_eq!(
            crate::privacy::resolve_extension("ucsfomopagent", None).tier,
            crate::privacy::ProviderTier::Private,
        );

        let public = config_disabled_extension_lines(&entries, &persisted, a_public_caller());
        assert!(
            !public.iter().any(|l| l.contains("ucsfomopagent")),
            "a private connector's NAME reached a public model's catalogue: {public:?}"
        );
        assert!(
            !public.iter().any(|l| l.contains("UCSF OMOP")),
            "a private connector's marketplace DESCRIPTION reached a public model: {public:?}"
        );
        assert!(
            public.iter().any(|l| l.contains("custom")),
            "the public entry must survive, or the filter is just an empty list: {public:?}"
        );

        // The same catalogue, unchanged, for the model that is entitled to it.
        let private = config_disabled_extension_lines(&entries, &persisted, a_private_caller());
        assert!(
            private.iter().any(|l| l.contains("ucsfomopagent")),
            "a private model must still see it: {private:?}"
        );
        assert_eq!(private.len(), 2);
    }

    /// DR-15's master opt-out reaches this filter through the capability, not
    /// through a second read of the process global.
    #[test]
    fn the_master_toggle_silences_the_catalogue_filter() {
        let persisted = std::collections::HashSet::from(["ucsfomopagent".to_string()]);
        let entries = a_catalogue_of_one_private_and_one_public();
        let off = config_disabled_extension_lines(
            &entries,
            &persisted,
            crate::privacy::CallCapability::for_test(crate::privacy::ProviderTier::Public, false),
        );
        assert!(
            off.iter().any(|l| l.contains("ucsfomopagent")),
            "with privacy tiers off nothing is withheld: {off:?}"
        );
    }

    /// **Finding 13, half two — the installed set**, through the real tool
    /// entry point rather than a helper.
    ///
    /// ⚠ This half is the one that contradicted Gate E most directly: the very
    /// extension `gate_e_lists_and_marks_a_mismatched_extensions_tools`' tier
    /// sibling hides from the tool list was named, in full, one tool call away.
    /// The assertion is therefore paired with a Gate E read on the same manager
    /// and the same capability — two surfaces, one answer.
    #[tokio::test]
    async fn search_available_extensions_hides_a_private_extension_from_a_public_model() {
        let (_dir, em, _handle) = affiliation_fixture(a_local_model()).await;

        let rendered = |content: Vec<Content>| -> String {
            content
                .iter()
                .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let public = rendered(
            em.search_available_extensions(a_public_caller())
                .await
                .unwrap(),
        );
        assert!(
            !public.contains("ucsfomopagent"),
            "the catalogue named an installed private connector to a public model:\n{public}"
        );
        assert!(
            public.contains("custom"),
            "a public third-party extension must still be listed, or this proves nothing:\n{public}"
        );
        assert!(
            !public.contains("developer"),
            "a Biorouter capability was presented as an extension:\n{public}"
        );

        // Gate E, same manager, same capability. The two surfaces must agree.
        let tools = em
            .get_prefixed_tools_excluding("nothing", Some(a_public_caller()))
            .await
            .unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            !names.iter().any(|n| n.starts_with("ucsfomopagent__")),
            "Gate E itself regressed, so this test would then pass for the wrong \
             reason: {names:?}"
        );

        let private = rendered(
            em.search_available_extensions(a_private_caller())
                .await
                .unwrap(),
        );
        assert!(
            private.contains("ucsfomopagent"),
            "a private model must still be able to discover it:\n{private}"
        );
    }

    /// **Finding 14.** `manage_extensions {disable}` ran `remove_extension`
    /// with no capability in scope, so a public chat could unload the clinical
    /// connector Gate E will not even show it.
    ///
    /// Asserted on the gate the tool calls, because the tool handler itself
    /// lives in `extension_manager_extension.rs` and needs a live
    /// `PlatformExtensionContext` to reach; the wiring is pinned separately by
    /// `the_disable_gate_has_a_production_caller` next door.
    #[tokio::test]
    async fn a_public_caller_may_not_disable_a_private_extension() {
        let (_dir, em, _handle) = affiliation_fixture(a_local_model()).await;

        let err = em
            .assert_extension_manageable("ucsfomopagent", a_public_caller())
            .await
            .expect_err("a public model may not unload the clinical connector");
        assert!(err.message.contains("private extension"), "{}", err.message);

        em.assert_extension_manageable("custom", a_public_caller())
            .await
            .expect("a public extension is still manageable, or the gate is a blanket refusal");

        let capability = em
            .assert_extension_manageable("developer", a_private_caller())
            .await
            .expect_err("built-in capabilities are not managed as extensions");
        assert!(
            capability.message.contains("capability"),
            "{}",
            capability.message
        );

        em.assert_extension_manageable("ucsfomopagent", a_private_caller())
            .await
            .expect("a private model may unload it");
    }

    /// The refusal must not become the existence oracle the leak it closes was.
    ///
    /// To a public caller, "the private connector is installed", "the private
    /// connector is not installed" and "no such extension" are one refusal —
    /// which is `assert_extension_reachable`'s inverted unknown-name default,
    /// inherited rather than restated.
    #[tokio::test]
    async fn the_disable_refusal_does_not_distinguish_installed_from_absent() {
        let (_dir, em, _handle) = affiliation_fixture(a_local_model()).await;

        let installed = em
            .assert_extension_manageable("ucsfomopagent", a_public_caller())
            .await
            .expect_err("installed private connector")
            .message
            .to_string();
        let absent = em
            .assert_extension_manageable("cdwagent", a_public_caller())
            .await
            .expect_err("a private connector that is NOT installed here")
            .message
            .to_string();
        let nonexistent = em
            .assert_extension_manageable("no-such-extension-anywhere", a_public_caller())
            .await
            .expect_err("a name that is no extension at all")
            .message
            .to_string();

        // The refusals name the extension the model asked about — that much the
        // model already knew, since it chose the word. What must not differ is
        // the SHAPE of the answer.
        let shape = |m: &str| {
            m.replace("ucsfomopagent", "X")
                .replace("cdwagent", "X")
                .replace("no-such-extension-anywhere", "X")
        };
        assert_eq!(shape(&installed), shape(&absent));
        assert_eq!(shape(&installed), shape(&nonexistent));
    }

    /// The gate resolves the same key its executor removes.
    ///
    /// `remove_extension` normalizes before removing, so a gate that looked up
    /// the raw spelling would read `Custom` as an unknown name — which
    /// `assert_extension_reachable` treats as Private and refuses. The bug that
    /// direction produces is a legitimate disable failing, but the same skew in
    /// a future executor that did NOT normalize would be a bypass.
    #[tokio::test]
    async fn the_disable_gate_normalizes_the_name_its_executor_normalizes() {
        let (_dir, em, _handle) = affiliation_fixture(a_local_model()).await;
        em.assert_extension_manageable("Custom", a_public_caller())
            .await
            .expect("`Custom` and `custom` are one extension to remove_extension");
    }

    // ---- issue #48: `/ext:` resolution by id + owning registry ----

    /// `/ext:extensionmanager` used to build an `ExtensionConfig::Builtin` named
    /// `"Extension Manager"` — the entry's *display* string — and the builtin
    /// lookup rejected it with `Unknown builtin extension: Extension Manager`,
    /// which the turn reported as if the extension had been refused.
    #[tokio::test]
    async fn ext_marker_enables_a_platform_extension() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = Arc::new(ExtensionManager::new_without_provider(
            temp_dir.path().to_path_buf(),
        ));

        let target = resolve_bundled_extension("extensionmanager")
            .expect("`extensionmanager` is a bundled extension");
        assert_eq!(target.kind(), BundledExtensionKind::Platform);
        assert_eq!(target.key(), "extensionmanager");

        let config = target.into_config("Selected via explicit resource marker".to_string());
        assert!(
            matches!(config, ExtensionConfig::Platform { .. }),
            "a platform target must take the platform spawn path, not the builtin one: {config:?}"
        );

        em.add_extension(config)
            .await
            .expect("/ext:extensionmanager must enable the Extension Manager");
        assert!(em.is_extension_enabled("extensionmanager").await);
    }

    /// The `builtin` case that already worked must keep working, end to end.
    #[tokio::test]
    async fn ext_marker_enables_a_builtin_extension() {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = Arc::new(ExtensionManager::new_without_provider(
            temp_dir.path().to_path_buf(),
        ));

        let target =
            resolve_bundled_extension("developer").expect("`developer` is a bundled extension");
        assert_eq!(target.kind(), BundledExtensionKind::Builtin);
        assert_eq!(target.key(), "developer");

        let config = target.into_config("Selected via explicit resource marker".to_string());
        assert!(matches!(config, ExtensionConfig::Builtin { .. }));

        em.add_extension(config)
            .await
            .expect("/ext:developer must enable the developer extension");
        assert!(em.is_extension_enabled("developer").await);
    }

    /// Every bundled extension must resolve to the registry that actually owns
    /// it, whether it is keyed by a lowercase id or by a display string.
    #[test]
    fn resolves_every_bundled_extension_to_its_owning_registry() {
        for (registry_key, def) in PLATFORM_EXTENSIONS.iter() {
            for reference in [*registry_key, def.name] {
                let target = resolve_bundled_extension(reference)
                    .unwrap_or_else(|| panic!("platform extension `{reference}` must resolve"));
                assert_eq!(
                    target.kind(),
                    BundledExtensionKind::Platform,
                    "`{reference}` must resolve to the platform registry"
                );
                assert!(
                    matches!(
                        target.clone().into_config(String::new()),
                        ExtensionConfig::Platform { .. }
                    ),
                    "`{reference}` must produce a Platform config"
                );
                // The name handed to the config is the one the spawn path looks
                // up, so it has to be present in PLATFORM_EXTENSIONS.
                let config = target.into_config(String::new());
                let ExtensionConfig::Platform { name, .. } = &config else {
                    unreachable!()
                };
                assert!(
                    PLATFORM_EXTENSIONS.contains_key(normalize(name).as_str()),
                    "`{name}` must be spawnable by the platform path"
                );
            }
        }

        for (registry_key, def) in biorouter_mcp::BUILTIN_EXTENSIONS.iter() {
            for reference in [*registry_key, def.name] {
                let target = resolve_bundled_extension(reference)
                    .unwrap_or_else(|| panic!("builtin extension `{reference}` must resolve"));
                assert_eq!(
                    target.kind(),
                    BundledExtensionKind::Builtin,
                    "`{reference}` must resolve to the builtin registry"
                );
                let config = target.into_config(String::new());
                let ExtensionConfig::Builtin { name, .. } = &config else {
                    panic!("`{reference}` must produce a Builtin config")
                };
                assert!(
                    biorouter_mcp::BUILTIN_EXTENSIONS.contains_key(name.as_str()),
                    "`{name}` must be spawnable by the builtin path"
                );
            }
        }
    }

    #[test]
    fn bundled_extension_reference_ids_are_nonempty_and_unique() {
        let mut owners = HashMap::<String, String>::new();
        let mut record = |kind: &str, registry_key: &str, name: &str| {
            let owner = format!("{kind}:{registry_key}");
            for reference in [registry_key, name] {
                let key = extension_reference_key(reference);
                assert!(!key.is_empty(), "`{owner}` has an empty comparable id");
                if let Some(existing) = owners.insert(key.clone(), owner.clone()) {
                    assert_eq!(
                        existing, owner,
                        "bundled extensions `{existing}` and `{owner}` collide on `{key}`"
                    );
                }
            }
        };

        for (registry_key, def) in PLATFORM_EXTENSIONS.iter() {
            record("platform", registry_key, def.name);
        }
        for (registry_key, def) in biorouter_mcp::BUILTIN_EXTENSIONS.iter() {
            record("builtin", registry_key, def.name);
        }
    }

    /// Moved here from `resource_refs`, which used to own this table.
    #[test]
    fn resolves_bundled_extension_spelling_aliases() {
        for reference in ["agentdrafter", "agent-drafter", "agent_drafter"] {
            let target = resolve_bundled_extension(reference).expect(reference);
            assert_eq!(target.kind(), BundledExtensionKind::Builtin);
            assert_eq!(target.key(), "agent_drafter");
        }
        for reference in [
            "autovisualizer",
            "auto-visualizer",
            "auto_visualiser",
            "autovisualiser",
        ] {
            let target = resolve_bundled_extension(reference).expect(reference);
            assert_eq!(target.kind(), BundledExtensionKind::Builtin);
            assert_eq!(target.key(), "autovisualiser");
        }
        for reference in [
            "extensionmanager",
            "extension-manager",
            "extension_manager",
            "Extension Manager",
            // Issue #60: the exact token the desktop mention popover and both
            // CLI completers now insert for a display name with a space, since
            // `extract_inline_refs` splits the message on whitespace and
            // `/ext:Extension Manager` would arrive truncated to `Extension`.
            "ExtensionManager",
        ] {
            let target = resolve_bundled_extension(reference).expect(reference);
            assert_eq!(target.kind(), BundledExtensionKind::Platform);
            assert_eq!(target.key(), "extensionmanager");
        }
        for reference in ["chatrecall", "chat-recall", "Chat Recall", "ChatRecall"] {
            let target = resolve_bundled_extension(reference).expect(reference);
            assert_eq!(target.kind(), BundledExtensionKind::Platform);
            assert_eq!(target.key(), "chatrecall");
        }
    }

    /// A non-bundled reference resolves to nothing, so a `/ext:` marker can
    /// never auto-enable a user-configured (stdio / streamable_http /
    /// inline_python / frontend / sse) extension the operator turned off.
    #[test]
    fn does_not_resolve_non_bundled_extensions() {
        for reference in ["", "東京", "pubmed", "spokeagent", "some-stdio-server"] {
            assert!(
                resolve_bundled_extension(reference).is_none(),
                "`{reference}` must not resolve to a bundled extension"
            );
        }
    }

    #[test]
    fn bundled_target_does_not_accept_a_user_extension_with_the_same_key() {
        let target =
            resolve_bundled_extension("developer").expect("`developer` is a bundled extension");
        let user_extension =
            ExtensionConfig::stdio("developer", "custom-server", "custom extension", 30_u64);
        assert!(!target.matches_config(&user_extension));

        let bundled = target.into_config(String::new());
        assert!(resolve_bundled_extension("developer")
            .expect("`developer` is a bundled extension")
            .matches_config(&bundled));
    }

    #[tokio::test]
    async fn bundled_target_enablement_rejects_injected_builtin_lookalikes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let manager = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let target = resolve_bundled_extension("developer").expect("Developer is bundled");
        manager
            .add_client(
                "developer".into(),
                ExtensionConfig::Builtin {
                    name: "developer".into(),
                    description: "lookalike".into(),
                    display_name: None,
                    timeout: None,
                    bundled: Some(true),
                    available_tools: vec!["text_editor".into()],
                },
                Arc::new(MockClient {}),
                None,
                None,
            )
            .await;

        assert!(
            !manager.is_bundled_target_enabled(&target).await,
            "matching config text is not trusted registry provenance"
        );
    }

    // ---- issue #57: the daemon's auth secret must not reach an extension ------

    /// The daemon-private name the hostile manifest declares to pin
    /// `doomed_env_keys`' `.chain(explicit)`. It is named once because the
    /// probe both declares it and asserts it is *absent* from the ambient
    /// environment — see `leak_probe_prints_extension_child_env`.
    #[cfg(unix)]
    const PROBE_DECLARED_ACP_KEY: &str = "BIOROUTER_ACP_WS_TOKEN";

    /// Child half of the stdio-extension leak probe.
    ///
    /// The leak is in the *inherited* environment, so exercising it means
    /// controlling this process's environment — and `set_var` is unsound in a
    /// threaded test binary. So the parent re-invokes this test binary with the
    /// canary exported, and this half spawns real children through the real
    /// [`prepare_child_environment`] — exactly what every stdio / inline-python
    /// extension is spawned with — and prints the environments they received.
    ///
    /// **Four children, not one.** Two independent axes, each of which a
    /// plausible wrong implementation could pass on one arm and leak on the
    /// other:
    ///
    /// - *what the manifest declares* — a **clean** manifest (so the inherited
    ///   `BIOROUTER_SERVER__SECRET_KEY` is the only copy on the command and the
    ///   inherited path is really under test) and a **hostile** one that names
    ///   daemon-private keys in its own `env_keys`. With only the hostile arm,
    ///   the declared copy shadows the inherited one on the `Command` and a
    ///   strip that handled explicit keys but not inherited ones would pass.
    /// - *the `working_dir` argument* — `None` and `Some(..)`. **Both production
    ///   spawns pass `Some(&working_dir)`** (`:850` stdio, `:920` inline-python);
    ///   a strip conditioned on `working_dir.is_none()` would leak on every real
    ///   extension while a `None`-only probe stayed green.
    #[cfg(unix)]
    #[tokio::test]
    #[ignore]
    async fn leak_probe_prints_extension_child_env() {
        // A precondition of the probe, not a behaviour of the product. The
        // explicit half below pins `doomed_env_keys`' `.chain(explicit)` by
        // declaring PROBE_DECLARED_ACP_KEY on the Command *only*. If the
        // surrounding environment already exports that name, the key is doomed
        // by the INHERITED half instead, and the explicit pin silently stops
        // discriminating — the same hollowing-out this probe exists to catch,
        // one level down. Fail loudly rather than degrade quietly.
        assert!(
            std::env::var_os(PROBE_DECLARED_ACP_KEY).is_none(),
            "{PROBE_DECLARED_ACP_KEY} is exported in this environment, which would let \
             the explicit-declaration half of this probe pass via the inherited path \
             instead of pinning what it claims to pin. Unset it and re-run."
        );

        let scratch = tempdir().expect("temp dir");
        let session_dir = scratch.path().to_path_buf();

        println!("BEGIN_CHILD_ENV");
        for (hostile_manifest, working_dir) in [
            (false, None),
            (false, Some(&session_dir)),
            (true, None),
            (true, Some(&session_dir)),
        ] {
            // What `merge_environments` hands the spawn path for an extension
            // that declares its own credentials — including, since #56, ones it
            // is not entitled to. A manifest may name any key in `env_keys`,
            // and merge_environments will resolve it out of the config or the
            // OS keyring and set it on the Command. `strip_daemon_private_env`
            // covers the explicitly-set case as well as the inherited one
            // (`doomed_env_keys` chains `env::vars_os()` with the command's own
            // envs); this is what says so at THIS layer rather than only at
            // developer/shell.rs.
            let mut declared = HashMap::from([
                (
                    "CLINICAL_RECORDS_TOKEN".to_string(),
                    "declared-credential-ok".to_string(),
                ),
                (
                    "EXTENSION_MODE".to_string(),
                    "declared-plain-ok".to_string(),
                ),
            ]);
            if hostile_manifest {
                // One key per branch of `is_daemon_private_env_key`, because
                // only the explicit path can reach both here: the real
                // `BIOROUTER_SERVER__SECRET_KEY` is also exported by the parent,
                // so its prefix branch would be satisfied by the inherited half
                // even if `.chain(explicit)` were dropped. `..__PROBE_DECLARED`
                // is set on the Command alone, so it pins the prefix branch of
                // the explicit path, and PROBE_DECLARED_ACP_KEY the
                // marker branch.
                declared.insert(
                    "BIOROUTER_SERVER__SECRET_KEY".to_string(),
                    "declared-daemon-secret-9f2c".to_string(),
                );
                declared.insert(
                    "BIOROUTER_SERVER__PROBE_DECLARED".to_string(),
                    "declared-server-prefix-9f2c".to_string(),
                );
                declared.insert(
                    PROBE_DECLARED_ACP_KEY.to_string(),
                    "declared-acp-token-9f2c".to_string(),
                );
            }
            let mut command = Command::new("printenv");
            command.envs(declared);
            prepare_child_environment(&mut command, working_dir);
            let out = command.output().await.expect("extension child must spawn");
            println!(
                "# probe arm: manifest={} working_dir={}",
                if hostile_manifest { "hostile" } else { "clean" },
                if working_dir.is_some() {
                    "some"
                } else {
                    "none"
                },
            );
            println!("{}", probe_report(&String::from_utf8_lossy(&out.stdout)));
        }
        println!("END_CHILD_ENV");
    }

    /// What the probe reports back: BioRouter's own namespace, the variables the
    /// parent injected, and — under *any* name — anything whose value carries
    /// the canary, so a copy of the secret under a different key is still
    /// caught. Everything else is dropped: printing the whole environment of
    /// whoever runs the suite would be its own small leak.
    #[cfg(unix)]
    fn probe_report(raw: &str) -> String {
        let canary = std::env::var("BR_TEST_CANARY").unwrap_or_default();
        raw.lines()
            .filter(|line| {
                let key = line.split('=').next().unwrap_or("");
                // The channel that carries the canary *to* the probe is itself
                // inherited; it is not the leak under test.
                if key == "BR_TEST_CANARY" {
                    return false;
                }
                key.starts_with("BIOROUTER_")
                    || key.starts_with("GOOSE_")
                    || matches!(
                        key,
                        "PATH"
                            | "HOME"
                            | "BR_TEST_USER_VAR"
                            | "CLINICAL_RECORDS_TOKEN"
                            | "EXTENSION_MODE"
                    )
                    || (!canary.is_empty() && line.contains(&canary))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[cfg(unix)]
    fn run_extension_leak_probe(canary: &str) -> String {
        let m = module_path!();
        let without_crate = m.split_once("::").map(|(_, rest)| rest).unwrap_or(m);
        let exe = std::env::current_exe().expect("test binary path");
        let out = std::process::Command::new(exe)
            .args([
                "--exact",
                &format!("{without_crate}::leak_probe_prints_extension_child_env"),
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("BIOROUTER_SERVER__SECRET_KEY", canary)
            // A second inherited daemon-private key carrying the same canary,
            // under a name no `declared` map in the probe sets. The line above
            // covers the *prefix* branch of `is_daemon_private_env_key` on the
            // inherited path; this covers the *marker* branch (`AUTH`/`TOKEN`),
            // which nothing else exercises from the inherited side.
            .env("BIOROUTER_INHERITED_AUTH_TOKEN", canary)
            .env("BR_TEST_CANARY", canary)
            .env("BIOROUTER_PORT", "54321")
            .env("BR_TEST_USER_VAR", "user-env-ok")
            .output()
            .expect("re-invoking the test binary must work");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let child_env = stdout
            .split_once("BEGIN_CHILD_ENV\n")
            .and_then(|(_, rest)| rest.split_once("END_CHILD_ENV"))
            .map(|(body, _)| body.to_string())
            .unwrap_or_else(|| {
                panic!(
                    "probe produced no child environment.\nstdout:\n{stdout}\nstderr:\n{}",
                    String::from_utf8_lossy(&out.stderr)
                )
            });

        // Everything the callers assert about a leak is `!contains`, which an
        // arm that never ran satisfies for free. Shrinking the child's loop to
        // its first entry — one *clean* manifest, no working dir — leaves every
        // assertion in both tests green while the hostile arms, the only place
        // the explicit-path negatives have anything to negate, silently stop
        // running. So the shape of the report is pinned here, once, where
        // neither test can be hollowed out without this failing first.
        let arms = child_env
            .lines()
            .filter(|line| line.starts_with("# probe arm: "))
            .collect::<Vec<_>>();
        assert_eq!(
            arms,
            [
                "# probe arm: manifest=clean working_dir=none",
                "# probe arm: manifest=clean working_dir=some",
                "# probe arm: manifest=hostile working_dir=none",
                "# probe arm: manifest=hostile working_dir=some",
            ],
            "the probe must run all four (manifest x working_dir) arms, once each; \
             a missing arm makes this run's negative assertions vacuous.\nchild env:\n{child_env}"
        );
        // And each of those arms must have had `command.envs(declared)` really
        // applied to it — otherwise the daemon-private names the hostile arms
        // declare were never on the Command in the first place, and their
        // absence from the child proves nothing. `EXTENSION_MODE` is the one
        // declared entry every arm sets and nothing strips.
        assert_eq!(
            child_env
                .matches("EXTENSION_MODE=declared-plain-ok")
                .count(),
            4,
            "every arm must reach its child with the manifest's declared \
             environment applied.\nchild env:\n{child_env}"
        );
        child_env
    }

    #[cfg(unix)]
    #[test]
    fn daemon_secret_never_reaches_an_extension_child() {
        const CANARY: &str = "canary-daemon-secret-4b71";
        let child_env = run_extension_leak_probe(CANARY);
        assert!(
            !child_env.contains(CANARY),
            "issue #57: the daemon's auth secret reached a stdio extension, which \
             can then call biorouterd as an authenticated client.\nchild env:\n{child_env}"
        );
        assert!(
            !child_env.contains("BIOROUTER_SERVER__SECRET_KEY"),
            "the key name itself must be gone, not just the value:\n{child_env}"
        );
        // A manifest that ASKS for the daemon's key does not get it either. The
        // inherited path is covered by CANARY above; this is the explicit path,
        // and it is the one a malicious extension author controls.
        //
        // ⚠ These are negative assertions, so they are only meaningful while
        // the hostile arms ran *and* `command.envs(declared)` was applied to
        // them. Both are pinned inside `run_extension_leak_probe`, which
        // refuses a report that does not carry all four arms with the declared
        // environment on each; without that, dropping the hostile arms would
        // make these three values vacuously absent and nothing would fail.
        for leaked in [
            "declared-daemon-secret-9f2c",
            "declared-server-prefix-9f2c",
            "declared-acp-token-9f2c",
        ] {
            assert!(
                !child_env.contains(leaked),
                "an extension declared a daemon-private key in its own envs and \
                 received it ({leaked}):\n{child_env}"
            );
        }
    }

    /// The other direction: an extension's own declared credentials, and the
    /// user's environment, must still arrive. Note `CLINICAL_RECORDS_TOKEN` is
    /// secret-shaped by name — the policy deliberately does not touch names
    /// outside BioRouter's own namespace.
    #[cfg(unix)]
    #[test]
    fn extension_child_still_receives_declared_and_user_environment() {
        let child_env = run_extension_leak_probe("canary-unused");
        for expected in [
            "PATH=",
            "HOME=",
            "BIOROUTER_PORT=54321",
            "BR_TEST_USER_VAR=user-env-ok",
            "CLINICAL_RECORDS_TOKEN=declared-credential-ok",
            "EXTENSION_MODE=declared-plain-ok",
        ] {
            assert!(
                child_env.lines().any(|l| l.starts_with(expected)),
                "extension child lost {expected:?}; removing too much is its own \
                 regression.\nchild env:\n{child_env}"
            );
        }
    }

    // ---- issue #68 / F1: the parent half of the jail-widening defect ---------

    /// The value of `BIOROUTER_WORKING_DIR` the child actually received, read
    /// out of a **real spawned process** rather than off the `Command` builder.
    /// Reading the builder would prove only that a field was set; the defect is
    /// about what crosses the process boundary, so the probe crosses it.
    #[cfg(unix)]
    async fn child_working_dir_env(command: &mut Command) -> Option<String> {
        let out = command.output().await.expect("extension child must spawn");
        assert!(
            out.status.success(),
            "probe child failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|line| line.strip_prefix("BIOROUTER_WORKING_DIR="))
            .map(str::to_owned)
    }

    /// Issue #68 / F1: an out-of-process extension spawned **after** its session
    /// directory has vanished must still be told what that directory was.
    ///
    /// This constructs the real condition rather than simulating it: a real
    /// directory is created, really deleted, and a real child is really spawned
    /// through the real [`prepare_child_environment`] — the same call
    /// `child_process_client` makes for every stdio extension.
    ///
    /// Before the fix, `current_dir` and `BIOROUTER_WORKING_DIR` were both set
    /// inside one `dir.exists() && dir.is_dir()` guard, so a vanished directory
    /// sent the child **neither** signal. `DeveloperServer::new()` then adopted
    /// the inherited environment and the daemon's process cwd — `/` under the
    /// packaged desktop app — and rooted its file jail there, widening it to the
    /// whole filesystem. Naming the base unconditionally is what lets the child
    /// refuse instead of re-rooting; the cwd stays conditional because a
    /// nonexistent `current_dir` is a spawn failure.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_vanished_working_dir_is_still_named_to_the_extension_child() {
        let scratch = tempdir().expect("temp dir");
        let vanished = scratch.path().to_path_buf();
        drop(scratch);
        assert!(
            !vanished.exists(),
            "the directory under test must really be gone"
        );

        let mut command = Command::new("printenv");
        prepare_child_environment(&mut command, Some(&vanished));

        // The other half of the fix: a directory that does not exist must NOT
        // become the child's cwd — that is a spawn error, and it is what makes
        // the child's own fallback unreachable.
        assert_eq!(
            command.as_std().get_current_dir(),
            None,
            "a vanished directory must not be handed to the child as its cwd"
        );

        assert_eq!(
            child_working_dir_env(&mut command).await.as_deref(),
            Some(vanished.to_string_lossy().as_ref()),
            "issue #68: an extension spawned after its session directory vanished \
             received no BIOROUTER_WORKING_DIR, so it adopts the daemon's cwd \
             (`/` under the packaged app) and widens its file jail to the whole \
             filesystem instead of refusing"
        );
    }

    /// The other direction, so the conditional-cwd half cannot be "fixed" by
    /// dropping `current_dir` altogether: a directory that still exists is both
    /// named to the child and made its working directory, and the child really
    /// runs there.
    #[cfg(unix)]
    #[tokio::test]
    async fn an_existing_working_dir_still_becomes_the_extension_child_cwd() {
        let scratch = tempdir().expect("temp dir");
        let dir = scratch.path().to_path_buf();

        let mut command = Command::new("sh");
        command.args(["-c", "printf ran > ./marker; printenv"]);
        prepare_child_environment(&mut command, Some(&dir));

        assert_eq!(command.as_std().get_current_dir(), Some(dir.as_path()));
        assert_eq!(
            child_working_dir_env(&mut command).await.as_deref(),
            Some(dir.to_string_lossy().as_ref())
        );
        assert!(
            dir.join("marker").exists(),
            "the child's relative write landed outside its session directory"
        );
    }

    /// The tier the ENFORCEMENT surface resolves for the entry stored under
    /// `key`, having first asserted the entry is really there.
    ///
    /// Task 43 (DR-23) deleted the stamped `Extension.tier` this used to read.
    /// The obvious replacement — `classify_extension(key)` — is a pure function
    /// of its own argument, which makes every assertion below a tautology true
    /// of any implementation, and review was right to call that a weakening.
    ///
    /// So it asks Gate E instead. `allowed_extension_keys` is what the tool list
    /// and the system prompt are filtered by, and it resolves the ADMITTED
    /// record — key and config together — rather than a string the test happens
    /// to hold. A wrong admission key, a config admitted under a different
    /// spelling, or a gate that stopped consulting the resolver all show up
    /// here; none of them showed up in the classifier call.
    ///
    /// The presence check stays, because an empty manager also has an empty
    /// allowed set and would read as "everything is private".
    async fn admitted_tier(em: &ExtensionManager, key: &str) -> crate::privacy::ProviderTier {
        use crate::privacy::ProviderTier;
        assert!(
            em.extensions.lock().await.contains_key(key),
            "nothing was admitted under `{key}`"
        );
        let allowed = em
            .allowed_extension_keys(Some(crate::privacy::CallCapability::for_test(
                ProviderTier::Public,
                true,
            )))
            .await;
        if allowed.iter().any(|k| k == key) {
            ProviderTier::Public
        } else {
            ProviderTier::Private
        }
    }

    async fn admit_via_add_extension(name: &str) -> crate::privacy::ProviderTier {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = Arc::new(ExtensionManager::new_without_provider(
            temp_dir.path().to_path_buf(),
        ));
        let target = resolve_bundled_extension(name).expect("a bundled extension");
        em.add_extension(target.into_config("privacy tier stamping".to_string()))
            .await
            .expect("admit the extension");
        admitted_tier(&em, name).await
    }

    async fn admit_via_add_client(name: &str) -> crate::privacy::ProviderTier {
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let config = ExtensionConfig::Builtin {
            name: name.to_string(),
            description: name.to_string(),
            display_name: None,
            timeout: None,
            bundled: None,
            available_tools: Vec::new(),
        };
        em.add_client(
            name.to_string(),
            config,
            Arc::new(MockClient {}),
            None,
            None,
        )
        .await;
        admitted_tier(&em, name).await
    }

    async fn admit_via_add_inprocess_server(name: &str) -> crate::privacy::ProviderTier {
        use biorouter_mcp::datasql::server::DataSqlServer;
        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        em.add_inprocess_server(name, DataSqlServer::new(std::collections::HashMap::new()))
            .await
            .expect("inject the per-app server");
        admitted_tier(&em, name).await
    }

    /// Issue #56. `add_extension`, `add_client` and `add_inprocess_server` each
    /// store the entry under a key the shared resolver answers for, because
    /// that key is what Gates C and E consult.
    ///
    /// ⚠ Task 43 (DR-23) deleted the stamped `Extension.tier` this test was
    /// written against, so what it now pins is the property that survived and
    /// is still load-bearing: all three admit under the SAME key the gates are
    /// filtered by later. `add_extension`'s key in particular is not always its
    /// config's name — a config with no name takes it from the server's own
    /// info — and an admission that stored one spelling while the gates asked
    /// about another would classify every private extension public.
    ///
    /// ⚠ The tier comes from **Gate E**, not from `classify_extension(key)`.
    /// The first replacement for the deleted stamp was the classifier, which is
    /// a pure function of its own argument and made every assertion below true
    /// of any implementation whatsoever. `allowed_extension_keys` resolves the
    /// admitted record instead, so the assertions have content again: see
    /// [`admitted_tier`].
    ///
    /// Two of the three admit an arbitrary NAME and are driven with a private
    /// one directly. `add_extension` cannot be: for every variant it can
    /// actually spawn in a hermetic test the name is also the SPAWN key
    /// (`Builtin` looks it up in `BUILTIN_EXTENSIONS`, `Platform` in
    /// `PLATFORM_EXTENSIONS`), and no private name names a bundled server — so
    /// only its public direction is reachable here.
    #[tokio::test]
    async fn all_three_admission_points_key_the_entry_the_resolver_will_be_asked_about() {
        use crate::privacy::ProviderTier;

        assert_eq!(
            admit_via_add_client("ucsfomopagent").await,
            ProviderTier::Private
        );
        assert_eq!(
            admit_via_add_inprocess_server("ucsfomopagent").await,
            ProviderTier::Private
        );
        assert_eq!(
            admit_via_add_inprocess_server("appcontrol").await,
            ProviderTier::Public
        );
        assert_eq!(
            admit_via_add_extension("developer").await,
            ProviderTier::Public
        );
    }

    /// A client that records the `McpMeta` its `call_tool` was handed, so a test
    /// can inspect exactly what `dispatch_tool_call` shipped to this extension.
    #[derive(Clone, Default)]
    struct MetaCapturingClient {
        seen: Arc<std::sync::Mutex<Option<McpMeta>>>,
    }

    #[async_trait::async_trait]
    impl McpClientTrait for MetaCapturingClient {
        fn get_info(&self) -> Option<&InitializeResult> {
            None
        }

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
            Err(Error::TransportClosed)
        }

        async fn list_tools(
            &self,
            _next_cursor: Option<String>,
            _cancellation_token: CancellationToken,
        ) -> Result<ListToolsResult, Error> {
            use serde_json::json;
            Ok(ListToolsResult {
                tools: vec![Tool::new(
                    "ping".to_string(),
                    "ping".to_string(),
                    Arc::new(json!({}).as_object().unwrap().clone()),
                )],
                next_cursor: None,
                meta: None,
            })
        }

        async fn call_tool(
            &self,
            _name: &str,
            _arguments: Option<JsonObject>,
            meta: McpMeta,
            _cancellation_token: CancellationToken,
        ) -> Result<CallToolResult, Error> {
            *self.seen.lock().unwrap() = Some(meta);
            Ok(CallToolResult {
                content: vec![],
                is_error: None,
                structured_content: None,
                meta: None,
            })
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
    }

    /// Issue #56, decision (4). The session id already goes to every MCP server
    /// including third-party stdio ones; the capability tier deliberately does
    /// not follow that precedent, because "this user is on an institutional
    /// model" is a fact about their configuration and a third-party server has
    /// no business learning it.
    ///
    /// ⚠ The wire key is taken from the const, never spelled here — a second
    /// hand-typed copy is how a barrier silently stops matching.
    #[tokio::test]
    async fn a_third_party_extension_never_learns_the_capability_tier() {
        use biorouter_mcp::knowledge::tier::CAPABILITY_TIER_META_KEY as KEY;
        use rmcp::model::{Extensions, Meta};

        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        let third_party = Arc::new(MetaCapturingClient::default());
        let builtin = Arc::new(MetaCapturingClient::default());
        em.add_mock_extension("thirdparty".to_string(), third_party.clone())
            .await;
        em.add_mock_extension("knowledge".to_string(), builtin.clone())
            .await;

        let private =
            crate::privacy::CallCapability::for_test(crate::privacy::ProviderTier::Private, true);

        for tool in ["thirdparty__ping", "knowledge__ping"] {
            let result = em
                .dispatch_tool_call(
                    "sess-1",
                    CallToolRequestParams {
                        task: None,
                        name: tool.to_string().into(),
                        arguments: Some(object!({})),
                        meta: None,
                    },
                    private,
                    CancellationToken::default(),
                )
                .await
                .expect("dispatch");
            result.result.await.expect("the mock client answers");
        }

        let meta_of = |c: &Arc<MetaCapturingClient>| -> Meta {
            let seen = c.seen.lock().unwrap().clone().expect("call_tool ran");
            seen.inject_into_extensions(Extensions::default())
                .get::<Meta>()
                .cloned()
                .unwrap_or_default()
        };

        assert_eq!(meta_of(&third_party).0.get(KEY), None);
        assert_eq!(
            meta_of(&builtin).0.get(KEY).and_then(|v| v.as_str()),
            Some("private")
        );
    }

    /// Issue #56 DR-26 / Task 50 Step 0. The affiliation crosses on the SAME
    /// terms as the tier bit above — built-ins only — and it really crosses:
    /// until this key existed, an affiliation on `CallCapability` reached no MCP
    /// server at all, so every knowledge-base gate on the far side was decided
    /// by a value that never arrived.
    ///
    /// ⚠ It is a SECOND key, not a richer value on the first. Both are asserted
    /// here, on one dispatch, because the failure this guards against is one of
    /// them being folded into the other: `tier::caller_is_private` compares
    /// against the exact word `private`, so a `private:institution:ucsf` on the
    /// tier key reads PUBLIC on any binary that has not been updated.
    #[tokio::test]
    async fn a_builtin_learns_the_callers_affiliation_and_a_third_party_never_does() {
        use biorouter_mcp::knowledge::affiliation::CAPABILITY_AFFILIATION_META_KEY as AFFILIATION;
        use biorouter_mcp::knowledge::tier::CAPABILITY_TIER_META_KEY as TIER;
        use rmcp::model::{Extensions, Meta};

        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());

        let third_party = Arc::new(MetaCapturingClient::default());
        let builtin = Arc::new(MetaCapturingClient::default());
        em.add_mock_extension("thirdparty".to_string(), third_party.clone())
            .await;
        em.add_mock_extension("knowledge".to_string(), builtin.clone())
            .await;

        let ucsf = crate::privacy::CallCapability::for_test_affiliated(
            crate::privacy::ProviderTier::Private,
            true,
            Some(crate::privacy::affiliation::ModelAffiliation::institution(
                crate::privacy::affiliation::InstitutionId::new("ucsf"),
            )),
        );

        for tool in ["thirdparty__ping", "knowledge__ping"] {
            let result = em
                .dispatch_tool_call(
                    "sess-affiliation",
                    CallToolRequestParams {
                        task: None,
                        name: tool.to_string().into(),
                        arguments: Some(object!({})),
                        meta: None,
                    },
                    ucsf,
                    CancellationToken::default(),
                )
                .await
                .expect("dispatch");
            result.result.await.expect("the mock client answers");
        }

        let meta_of = |c: &Arc<MetaCapturingClient>| -> Meta {
            let seen = c.seen.lock().unwrap().clone().expect("call_tool ran");
            seen.inject_into_extensions(Extensions::default())
                .get::<Meta>()
                .cloned()
                .unwrap_or_default()
        };

        // The third party learns neither axis.
        assert_eq!(meta_of(&third_party).0.get(AFFILIATION), None);
        assert_eq!(meta_of(&third_party).0.get(TIER), None);

        // The built-in learns both, under two distinct keys.
        let builtin_meta = meta_of(&builtin);
        assert_eq!(
            builtin_meta.0.get(TIER).and_then(|v| v.as_str()),
            Some("private"),
            "the tier key must keep its bare grammar"
        );
        assert_eq!(
            builtin_meta.0.get(AFFILIATION).and_then(|v| v.as_str()),
            Some("institution:ucsf")
        );
        // …and the reader's own parser agrees with what was written, so this is
        // a round trip through production's writer rather than a string match.
        assert_eq!(
            biorouter_mcp::knowledge::affiliation::caller_affiliation(&builtin_meta),
            biorouter_mcp::knowledge::affiliation::CallerAffiliation::Institution("ucsf".into())
        );
    }

    /// A model that states no affiliation leaves the key OFF rather than
    /// writing a word for it, which is exactly how an older daemon looks — and
    /// the reader treats both the same restrictive way.
    #[tokio::test]
    async fn an_unstated_affiliation_writes_no_key_at_all() {
        use biorouter_mcp::knowledge::affiliation::CAPABILITY_AFFILIATION_META_KEY as AFFILIATION;
        use rmcp::model::{Extensions, Meta};

        let temp_dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(temp_dir.path().to_path_buf());
        let builtin = Arc::new(MetaCapturingClient::default());
        em.add_mock_extension("knowledge".to_string(), builtin.clone())
            .await;

        let unstated = crate::privacy::CallCapability::for_test_affiliated(
            crate::privacy::ProviderTier::Private,
            true,
            None,
        );
        let result = em
            .dispatch_tool_call(
                "sess-unstated",
                CallToolRequestParams {
                    task: None,
                    name: "knowledge__ping".to_string().into(),
                    arguments: Some(object!({})),
                    meta: None,
                },
                unstated,
                CancellationToken::default(),
            )
            .await
            .expect("dispatch");
        result.result.await.expect("the mock client answers");

        let seen = builtin.seen.lock().unwrap().clone().expect("call_tool ran");
        let meta = seen
            .inject_into_extensions(Extensions::default())
            .get::<Meta>()
            .cloned()
            .unwrap_or_default();
        assert_eq!(meta.0.get(AFFILIATION), None);
        assert_eq!(
            biorouter_mcp::knowledge::affiliation::caller_affiliation(&meta),
            biorouter_mcp::knowledge::affiliation::CallerAffiliation::Unstated
        );
    }

    // ----------------------------------------------------------------------
    // Issue #56 Gate C: the dispatch choke point.
    // ----------------------------------------------------------------------

    /// A manager over an isolated session store, plus that store and one real
    /// session row in it. The `TempDir` is returned because dropping it deletes
    /// the SQLite file the manager still holds.
    async fn manager_with_a_session() -> (
        TempDir,
        ExtensionManager,
        Arc<crate::session::SessionManager>,
        String,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let em = ExtensionManager::new_without_provider(dir.path().to_path_buf());
        let sm = em.get_context().session_manager.clone();
        let session = sm
            .create_session(
                PathBuf::from("."),
                "gate-c".to_string(),
                crate::session::session_manager::SessionType::User,
            )
            .await
            .unwrap();
        (dir, em, sm, session.id)
    }

    fn call(name: &str) -> CallToolRequestParams {
        CallToolRequestParams {
            task: None,
            name: name.to_string().into(),
            arguments: Some(object!({})),
            meta: None,
        }
    }

    /// O5's second trigger. A PERMITTED private dispatch is a disclosure — the
    /// model was allowed to ask an institutional connector a question — so the
    /// session's classification ratchets at permit time, with `mcp:` provenance
    /// so §12.4 can grade the declassification confirmation on it.
    ///
    /// ⚠ **The row is re-read BEFORE the returned future is awaited, and that
    /// ordering is the whole test.** `dispatch_tool_call` hands back a
    /// `ToolCallResult` whose `result` has not run yet; a version of this test
    /// that awaited it first — which is how this was first written — passes
    /// identically against an implementation that ratchets on the tool's
    /// *success*, so it could not tell the two apart while its name claimed to.
    /// Permit-time is the design's choice for a reason the test now enforces: a
    /// failed OMOP query still carried the session's cohort definition to the
    /// connector. Awaiting the future afterwards is kept only so the mock's
    /// answer is not dropped on the floor.
    #[tokio::test]
    async fn a_permitted_private_dispatch_ratchets_the_session_at_permit_time() {
        let (_dir, em, sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;

        // No turn has run, so the row is still public.
        assert_eq!(
            sm.get_session(&id, false).await.unwrap().privacy_tier,
            crate::privacy::SessionClassification::Public
        );

        let dispatched = em
            .dispatch_tool_call(
                &id,
                call("ucsfomopagent__tool"),
                crate::privacy::CallCapability::for_test(
                    crate::privacy::ProviderTier::Private,
                    true,
                ),
                CancellationToken::default(),
            )
            .await
            .expect("a private caller may reach a private extension");

        // Permit time: the tool has NOT been called yet.
        let row = sm.get_session(&id, false).await.unwrap();
        assert_eq!(
            row.privacy_tier,
            crate::privacy::SessionClassification::Private,
            "the ratchet must land on the permit, not on the tool's answer"
        );
        assert_eq!(row.privacy_reason.as_deref(), Some("mcp:ucsfomopagent"));

        dispatched.result.await.expect("the mock client answers");
    }

    /// The ratchet sits BELOW BR-23's secret scan, which is a deviation from the
    /// plan's stated ordering and therefore owes an assertion rather than a
    /// comment.
    ///
    /// The premise is the same one that makes a Gate C refusal leave the row
    /// alone: a classification is permanent, so it may only be written for
    /// something that actually left the process. A SecretGuard denial is caught
    /// in the daemon's own dispatch path — the connector never heard from us —
    /// so there is nothing to record, and recording it would be an irreversible
    /// claim about a call that never happened.
    #[tokio::test]
    async fn a_secret_guard_denial_leaves_the_row_alone() {
        let (_dir, em, sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;

        // The guard only blocks an argument that names a secret file which
        // ACTUALLY EXISTS (`SecretGuard::find_denied_path`), so the fixture has
        // to create one. Absolute, so it is independent of the manager's
        // resolved working dir.
        let secrets = tempfile::tempdir().unwrap();
        let dotenv = secrets.path().join(".env");
        std::fs::write(&dotenv, "SECRET=1").unwrap();

        let denied = match em
            .dispatch_tool_call(
                &id,
                CallToolRequestParams {
                    task: None,
                    name: "ucsfomopagent__tool".to_string().into(),
                    arguments: Some(object!({ "path": dotenv.to_string_lossy() })),
                    meta: None,
                },
                crate::privacy::CallCapability::for_test(
                    crate::privacy::ProviderTier::Private,
                    true,
                ),
                CancellationToken::default(),
            )
            .await
        {
            Ok(_) => panic!("BR-23 must block an argument naming an existing .env"),
            Err(e) => e.to_string(),
        };
        assert!(
            denied.contains("secret/credential deny pattern"),
            "{denied}"
        );

        let row = sm.get_session(&id, false).await.unwrap();
        assert_eq!(
            row.privacy_tier,
            crate::privacy::SessionClassification::Public,
            "a call the daemon refused reached no connector, so it must not \
             permanently classify the chat"
        );
        assert_eq!(row.privacy_reason, None);
    }

    /// The capability `POST /agent/call_tool` hands the manager —
    /// `Public` + enforced, the most restrictive pair, spelled here with the
    /// test constructor so Task 51's census of the two production spellings
    /// keeps counting production entries only.
    ///
    /// Two assertions in one, because they fail differently: the call is
    /// refused with a message the caller can act on, AND nothing was recorded
    /// against the session — a refused call reached no connector, so classifying
    /// the chat private would be a lie that cannot be undone.
    #[tokio::test]
    async fn a_public_caller_is_refused_and_the_row_is_left_alone() {
        let (_dir, em, sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;

        // `ToolCallResult` is not `Debug`, so the outcome is matched rather
        // than `expect_err`'d.
        let text = match em
            .dispatch_tool_call(
                &id,
                call("ucsfomopagent__tool"),
                crate::privacy::CallCapability::for_test(
                    crate::privacy::ProviderTier::Public,
                    true,
                ),
                CancellationToken::default(),
            )
            .await
        {
            Ok(_) => panic!("a public caller must not reach a private extension"),
            Err(e) => e.to_string(),
        };
        // The WHOLE refusal: `Tool '…' not found` also names the extension, so
        // asserting on the name alone would pass on a fixture that never loaded
        // it.
        assert!(
            text.contains(
                &crate::privacy::refusal::privacy_refusal(
                    "ucsfomopagent",
                    crate::privacy::ProviderTier::Private,
                    crate::privacy::ProviderTier::Public,
                )
                .expect("the pure refusal")
                .message
                .to_string()
            ),
            "{text}"
        );

        let row = sm.get_session(&id, false).await.unwrap();
        assert_eq!(
            row.privacy_tier,
            crate::privacy::SessionClassification::Public,
            "a refused call disclosed nothing, so it must not classify the chat"
        );
        assert_eq!(row.privacy_reason, None);
    }

    /// The gate reads the RESOLVED RECORD, never the tool-name string.
    ///
    /// `get_client_for_tool` routes by `starts_with` over a `HashMap`, and
    /// `normalize()` permits `_`, so an extension may legitimately be keyed with
    /// a `__` inside it. An implementation that split the tool name at its first
    /// `__` would classify `ucsfomopagent__mirror__tool` off `ucsfomopagent`
    /// and refuse a call to an extension the registry has never heard of.
    ///
    /// Only the strict direction is constructible against the real registry:
    /// the divergence needs the resolved key to contain `__`, and neither of the
    /// two private names does, so no fixture can make the naive parse read
    /// *public* where the record says private. That leaky direction is Task 16's.
    #[tokio::test]
    async fn the_tier_comes_from_the_resolved_record_not_the_tool_name() {
        let (_dir, em, _sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent__mirror".to_string(), Arc::new(MockClient {}))
            .await;
        assert_eq!(
            crate::privacy::classify_extension("ucsfomopagent__mirror"),
            crate::privacy::ProviderTier::Public,
            "the fixture only discriminates if the resolved key is public"
        );

        let dispatched = em
            .dispatch_tool_call(
                &id,
                call("ucsfomopagent__mirror__tool"),
                crate::privacy::CallCapability::for_test(
                    crate::privacy::ProviderTier::Public,
                    true,
                ),
                CancellationToken::default(),
            )
            .await
            .expect("the resolved record is public, so a public caller may call it");
        dispatched.result.await.expect("the mock client answers");
    }

    /// What a hand-renamed `config.yaml` entry looks like: the map key and the
    /// entry's own `name` are BOTH the new name, and the only surviving link to
    /// the install is the `--directory` argument, which is where the server's
    /// code physically lives.
    fn renamed_entry(new_name: &str, install_dir: &str) -> ExtensionConfig {
        ExtensionConfig::Stdio {
            name: new_name.to_string(),
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
        }
    }

    /// **Task 43 / DR-23's gate, Step 3.1.** Install a private extension,
    /// rename it in `config.yaml`, and every enforcing gate must still refuse.
    ///
    /// This asserts on ENFORCEMENT, not on the badge the UI renders, because
    /// enforcement is what the rename actually removed: `classify_extension`
    /// keyed on the config name, and Gates C, E and F2 all read its answer, so
    /// `name: mystuff` in the entry the marketplace wrote as `cdwagent` turned a
    /// clinical connector into a public one for the model as well as for the
    /// badge.
    ///
    /// ⚠ **The fixture is a real rename**, i.e. the map key and the config's own
    /// `name` are BOTH `mystuff` and neither resembles the registry id. That is
    /// what a user editing `config.yaml` produces, and it is what defeats a
    /// resolver keyed on the name in any spelling. The only thing tying this
    /// entry to its install is the `--directory` argument, which is where the
    /// server's code actually lives.
    ///
    /// The three gates are asserted TOGETHER, in one test, on one fixture. They
    /// are three different functions reading one resolver, and a version of this
    /// split into three tests passes on an implementation where only the one
    /// under test consults provenance — which is exactly the shape of the bug
    /// (`Extension.tier` was stamped at admission and three other callers
    /// re-classified from the name instead).
    ///
    /// `developer` rides along in every assertion so a fixture that admitted
    /// nothing, or a filter that dropped everything, fails loudly instead of
    /// passing vacuously.
    #[tokio::test]
    async fn a_renamed_private_extension_is_still_refused_by_gates_c_e_and_f2() {
        let install_dir = "/home/researcher/.config/biorouter/extensions/CDWAgent";
        crate::privacy::provenance::insert_test_record_at(
            "cdwagent-as-installed",
            "cdwagent",
            Some(install_dir),
        );
        let (_dir, em, sm, id) = manager_with_a_session().await;
        em.add_client(
            "mystuff".to_string(),
            renamed_entry("mystuff", install_dir),
            Arc::new(MockClient {}),
            None,
            None,
        )
        .await;
        em.add_mock_extension("developer".to_string(), Arc::new(MockClient {}))
            .await;
        assert_eq!(
            crate::privacy::classify_extension("mystuff"),
            crate::privacy::ProviderTier::Public,
            "the fixture only discriminates if the NAME alone reads public"
        );

        // Gate E — the tool list a public model is shown.
        let tools = em.get_prefixed_tools(None).await.unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            !names.iter().any(|n| n.starts_with("mystuff__")),
            "the rename put a private server's tool schemas into a public model's prompt: {names:?}"
        );
        assert!(names.contains(&"developer__tool"), "{names:?}");

        // Gate F2 — the server's own instructions in the system prompt.
        let info = em.get_extensions_info().await;
        let listed: Vec<&str> = info.iter().map(|i| i.name.as_str()).collect();
        assert!(!listed.contains(&"mystuff"), "{listed:?}");
        assert!(listed.contains(&"developer"), "{listed:?}");

        // Gate C — dispatch.
        let text = match em
            .dispatch_tool_call(
                &id,
                call("mystuff__tool"),
                crate::privacy::CallCapability::for_test(
                    crate::privacy::ProviderTier::Public,
                    true,
                ),
                CancellationToken::default(),
            )
            .await
        {
            Ok(_) => panic!("a public caller reached a renamed private extension"),
            Err(e) => e.to_string(),
        };
        assert!(
            text.contains(
                &crate::privacy::refusal::privacy_refusal(
                    "mystuff",
                    crate::privacy::ProviderTier::Private,
                    crate::privacy::ProviderTier::Public,
                )
                .expect("the pure refusal")
                .message
                .to_string()
            ),
            "{text}"
        );
        assert_eq!(
            sm.get_session(&id, false).await.unwrap().privacy_tier,
            crate::privacy::SessionClassification::Public,
            "a refused call disclosed nothing"
        );
    }

    /// Finding M6: the roster a resource read hands back when it finds nothing
    /// is Gate E's, never the extension map's.
    ///
    /// `read_resource_tool` with no `extension_name` fans out over every
    /// installed extension, refusing the private ones one at a time — and then
    /// ended by composing *"Here are the available extensions: …"* from
    /// `self.extensions.lock().await.keys()`, handing back in one sentence every
    /// name it had just spent the loop withholding. `POST /agent/call_tool`
    /// reaches this with `CallCapability::public_enforced()`, so an HTTP client
    /// holding only the daemon secret could read a chat's private-connector
    /// names out of a not-found message.
    ///
    /// ⚠ **`developer` rides along, and the test is worth nothing without it.**
    /// An implementation that simply deleted the list passes every "does not
    /// contain `ucsfomopagent`" assertion. Requiring the public extension to
    /// still be named is what distinguishes "filtered through Gate E" from
    /// "silenced".
    #[tokio::test]
    async fn a_public_callers_resource_miss_is_answered_with_gate_es_roster_only() {
        let (_dir, em, _sm, _id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;
        em.add_mock_extension("developer".to_string(), Arc::new(MockClient {}))
            .await;

        let public =
            crate::privacy::CallCapability::for_test(crate::privacy::ProviderTier::Public, true);
        let text = em
            .read_resource_tool(
                serde_json::json!({ "uri": "nope://x" }),
                Some(public),
                CancellationToken::default(),
            )
            .await
            .expect_err("no extension holds that uri")
            .message
            .to_string();

        assert!(
            !text.contains("ucsfomopagent"),
            "a caller evaluated as public was handed the name of a private extension in a \
             not-found message — the set Gate E exists to withhold: {text}"
        );
        assert!(
            text.contains("developer"),
            "the roster was silenced rather than filtered, so this test would pass against an \
             implementation that told a legitimate model nothing: {text}"
        );

        // The same call on a PRIVATE model still sees everything, which is what
        // makes the assertion above a privacy filter rather than a deletion.
        let private =
            crate::privacy::CallCapability::for_test(crate::privacy::ProviderTier::Private, true);
        let as_private = em
            .read_resource_tool(
                serde_json::json!({ "uri": "nope://x" }),
                Some(private),
                CancellationToken::default(),
            )
            .await
            .expect_err("no extension holds that uri")
            .message
            .to_string();
        assert!(as_private.contains("ucsfomopagent"), "{as_private}");
        assert!(as_private.contains("developer"), "{as_private}");
    }

    /// The same roster one door further in: `read_resource`'s `get_server_client`
    /// miss, which composed the identical list from the raw extension map.
    ///
    /// ⚠ **This one is defence in depth, and the honest reason is worth stating
    /// rather than dressing up.** No capability can reach that branch with a
    /// roster that differs from the map. `get_server_client` reads the SAME map
    /// `assert_extension_reachable` just consulted, so presence implies a
    /// client: a public caller naming an absent extension is refused above and
    /// never arrives, and a private caller — who does arrive — is shown
    /// everything by Gate E anyway. What is left is the race the two locks
    /// admit, and a message that is safe only because one of its readers (#206's
    /// `read_resource_failure`, which replaces it) throws it away.
    ///
    /// So the composition is pinned at the SOURCE, because behaviour cannot see
    /// it, and the reachable half is asserted underneath so the branch is known
    /// to work rather than merely to be spelled correctly. A test that claimed
    /// to demonstrate a public-caller leak here would be vacuous: the first
    /// draft of this one passed against the unfixed tree.
    #[tokio::test]
    async fn the_named_branchs_not_found_message_is_composed_from_gate_es_roster() {
        let source = include_str!("extension_manager.rs");
        let miss = source
            .split("let client = match self.get_server_client(extension_name).await {")
            .nth(1)
            .expect("`read_resource` must still look its client up before reading")
            .split("\n        };")
            .next()
            .expect("the lookup must be a match block");
        assert!(
            miss.contains("self.allowed_extension_keys(admitted).await"),
            "the not-found message is composed from something other than Gate E's verdict \
             again: {miss}"
        );
        assert!(
            !miss.contains("self.extensions"),
            "the not-found message reaches back into the raw extension map, which is the \
             shape finding M6 closed next door: {miss}"
        );

        // The reachable half. A private caller naming something that is not
        // installed is the ONE path into this branch, and it must still answer
        // with a usable list rather than an empty one.
        let (_dir, em, _sm, _id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;
        em.add_mock_extension("developer".to_string(), Arc::new(MockClient {}))
            .await;
        let private =
            crate::privacy::CallCapability::for_test(crate::privacy::ProviderTier::Private, true);
        let text = em
            .read_resource(
                "ui://cohort",
                "nonexistent_ext",
                Some(private),
                CancellationToken::default(),
            )
            .await
            .expect_err("nothing is loaded under that name")
            .message
            .to_string();
        assert!(text.contains("developer"), "{text}");
        assert!(text.contains("ucsfomopagent"), "{text}");
    }

    /// Finding M18: a name this gate could not resolve is still refused, and the
    /// refusal no longer says it is a private extension.
    ///
    /// ⚠ **The two answers must stay IDENTICAL, and that is the assertion that
    /// matters.** `assert_extension_reachable` reads an unknown name as Private
    /// deliberately: to a public caller "this private connector is installed",
    /// "this private connector is not installed" and "no such extension" are one
    /// refusal, which is what stops the gate being an existence oracle over the
    /// names finding M6 closes next door. So the repair could not be "say `no
    /// such extension` for the absent case" — it had to be one sentence true of
    /// both. Comparing the two messages with the name held constant is what
    /// stops a later edit adding one helpful clause to the branch it can tell
    /// apart.
    #[tokio::test]
    async fn a_private_extension_and_a_name_that_is_not_installed_are_one_refusal() {
        let (_dir, em, _sm, _id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;

        let public =
            crate::privacy::CallCapability::for_test(crate::privacy::ProviderTier::Public, true);
        let refusal_for = |name: &'static str| {
            let em = &em;
            async move {
                em.read_resource(
                    "ui://cohort",
                    name,
                    Some(public),
                    CancellationToken::default(),
                )
                .await
                .expect_err("a public caller reaches neither")
                .message
                .to_string()
            }
        };

        let loaded_and_private = refusal_for("ucsfomopagent").await;
        let never_installed = refusal_for("nonexistent_ext").await;

        assert!(
            !never_installed.contains("`nonexistent_ext` is a private extension"),
            "the gate still asserts that a name it could not resolve IS a private extension, \
             which sends a model looking for a private model to reach something that does not \
             exist: {never_installed}"
        );
        assert_eq!(
            loaded_and_private.replace("ucsfomopagent", "NAME"),
            never_installed.replace("nonexistent_ext", "NAME"),
            "the two cases now read differently, so a public caller can walk names and learn \
             which private extensions this chat has loaded"
        );
        assert!(
            never_installed.contains("not installed"),
            "the refusal must state the other case it covers, or it is the same claim in \
             softer words: {never_installed}"
        );
    }

    /// **Step 3.2.** Take every optional input away and the private extension is
    /// still private.
    ///
    /// ⚠ What "the registry" is on this side of the app, because the honest
    /// answer decides whether this test means anything. There is no network
    /// path to BAAM from Rust — the only fetch is the Electron `registry:fetch`
    /// handler — so the resolver's registry IS the compiled-in
    /// `PRIVATE_EXTENSIONS` snapshot, which is linked into the binary and cannot
    /// go missing at runtime. Task 37's "raises and never lowers" therefore
    /// holds here by construction rather than by a retention cache: an
    /// unreachable network cannot subtract from a constant.
    ///
    /// What CAN go missing is the local provenance store, so that is what this
    /// test removes — the store empty, and separately a record naming an id the
    /// snapshot has never heard of. Neither may lower anything.
    ///
    /// ⚠ **The RENAMED entry is asserted here too, and it is the only case where
    /// any of this bites.** An entry still under its installed name is private by
    /// name alone; taking provenance away from it changes nothing, which is why a
    /// version of this test written only against `cdwagent` and `ucsfomopagent`
    /// passes identically against pre-Task-43 code. The renamed fixture below is
    /// what makes the retention claim mean something: its ONLY route to Private
    /// is the record, so a resolver that let a second, useless record displace
    /// the useful one — precedence instead of union — fails here. The one thing
    /// that genuinely does lower it is deleting the record outright, and that is
    /// the residual, asserted next door in
    /// [`the_residual_bar_is_a_rename_plus_removing_the_record`].
    #[tokio::test]
    async fn nothing_a_public_model_can_take_away_lowers_a_private_extension() {
        use crate::privacy::ProviderTier::{Private, Public};

        // No record at all: the compiled snapshot alone still classifies.
        assert_eq!(crate::privacy::classify_extension("cdwagent"), Private);

        // A record whose id the snapshot does not publish: retained, not
        // defaulted to public.
        crate::privacy::provenance::insert_test_record(
            "gate32-clinical",
            "an-id-this-build-has-never-seen",
        );
        assert_eq!(
            crate::privacy::classify_extension("gate32-clinical"),
            Public
        );
        crate::privacy::provenance::insert_test_record("ucsfomopagent", "an-id-nobody-publishes");
        assert_eq!(crate::privacy::classify_extension("ucsfomopagent"), Private);

        // A RENAMED entry, whose only route to Private is its record, keeps it
        // when a second record naming an unknown id is added alongside.
        let install_dir = "/home/researcher/.config/biorouter/extensions/Gate32Renamed";
        crate::privacy::provenance::insert_test_record_at(
            "gate32-renamed-as-installed",
            "cdwagent",
            Some(install_dir),
        );
        crate::privacy::provenance::insert_test_record_at(
            "gate32-renamed-decoy",
            "an-id-this-build-has-never-seen",
            Some(install_dir),
        );
        let renamed = renamed_entry("gate32-mystuff", install_dir);
        assert_eq!(
            crate::privacy::classify_extension("gate32-mystuff"),
            Public,
            "the fixture only discriminates if the NAME alone reads public"
        );
        assert_eq!(
            crate::privacy::classify_extension_entry("gate32-mystuff", Some(&renamed)),
            Private,
            "a record naming an id the snapshot does not publish displaced the one that did; \
             the sources are unioned, not tried in order"
        );

        // And the gate that reads it is still closed, for both.
        let (_dir, em, _sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;
        em.add_client(
            "gate32-mystuff".to_string(),
            renamed,
            Arc::new(MockClient {}),
            None,
            None,
        )
        .await;
        for tool in ["ucsfomopagent__tool", "gate32-mystuff__tool"] {
            assert!(
                em.dispatch_tool_call(
                    &id,
                    call(tool),
                    crate::privacy::CallCapability::for_test(Public, true),
                    CancellationToken::default(),
                )
                .await
                .is_err(),
                "with the provenance store carrying nothing useful, the compiled snapshot must \
                 still refuse {tool}"
            );
        }
    }

    /// **The residual, asserted rather than left to be discovered.**
    ///
    /// Step 2 says an unreachable registry retains the last known tier and never
    /// defaults to public. For a renamed entry there is no *last known tier* to
    /// retain — DR-23 forbids storing one, which is the whole point — so what is
    /// retained is the identity RECORD, and the honest statement of the bar is
    /// that removing that record returns the entry to its config-name answer.
    /// After a rename that answer is Public.
    ///
    /// So the evasion is **two** edits, not one: rename the entry, then delete
    /// the provenance file. `docs/security/privacy-tiers.md` §5.3 states it at
    /// that height; this test is what stops the statement drifting upward.
    ///
    /// It is not a regression — before Task 43 the rename alone sufficed — and it
    /// cannot be closed without reintroducing exactly the locally-forgeable
    /// stored tier DR-23 deleted.
    ///
    /// ⚠ "The store is gone" is modelled as "no record matches this fixture",
    /// which is what deletion reduces to at the resolver: `registry_ids_for`
    /// returns an empty vector for an absent file, an unreadable one and a
    /// fixture nothing was recorded for alike. The additive test store cannot be
    /// emptied (see `provenance::test_records`), so the fixture below is simply
    /// one nothing was ever recorded for.
    #[tokio::test]
    async fn the_residual_bar_is_a_rename_plus_removing_the_record() {
        let install_dir = "/home/researcher/.config/biorouter/extensions/ResidualNoRecord";
        let renamed = renamed_entry("residual-mystuff", install_dir);

        assert_eq!(
            crate::privacy::classify_extension_entry("residual-mystuff", Some(&renamed)),
            crate::privacy::ProviderTier::Public,
            "with no record to find, a renamed entry falls back to the config-name join, and this \
             is the documented residual, and if it ever changes §5.3 is now wrong"
        );

        // Stated at the gate as well as at the resolver, because §5.3's claim is
        // about enforcement and a resolver-only assertion would not notice a
        // gate that had grown a second, stricter source.
        let (_dir, em, _sm, id) = manager_with_a_session().await;
        em.add_client(
            "residual-mystuff".to_string(),
            renamed,
            Arc::new(MockClient {}),
            None,
            None,
        )
        .await;
        let dispatched = em
            .dispatch_tool_call(
                &id,
                call("residual-mystuff__tool"),
                crate::privacy::CallCapability::for_test(
                    crate::privacy::ProviderTier::Public,
                    true,
                ),
                CancellationToken::default(),
            )
            .await
            .expect("no record means the config-name join, which reads public");
        dispatched.result.await.expect("the mock client answers");
    }

    /// **Step 3.3, structural.** No tier is persisted on the config entry, so a
    /// future reader cannot resurrect the shadowing path DR-23 removed.
    ///
    /// Two halves, because they fail differently. A tier cannot be WRITTEN: no
    /// variant of `ExtensionConfig` serialises one. And a tier cannot be READ:
    /// a `tier:` line hand-added to `config.yaml` — by a user, or by the model
    /// that has `developer__shell` — round-trips away instead of being carried
    /// into the record, so an implementation that later grew such a field would
    /// have to fail this test to exist.
    ///
    /// ⚠ **This half does not guard the field DR-23 actually deleted**, and
    /// review was right to say so. The removed field lived on `struct Extension`,
    /// not on `ExtensionConfig`, which never carried a tier — so re-adding
    /// `Extension.tier` and stamping it at admission passes everything below.
    /// [`the_tier_is_resolved_at_read_time_not_stamped_at_admission`] is the half
    /// that catches that, and the two are deliberately separate: one is about
    /// what reaches `config.yaml`, the other about when the answer is computed.
    #[test]
    fn no_tier_is_persisted_on_the_config_entry() {
        let stdio = ExtensionConfig::Stdio {
            name: "cdwagent".to_string(),
            description: "clinical".to_string(),
            cmd: "uv".to_string(),
            args: vec!["run".to_string()],
            envs: crate::agents::extension::Envs::default(),
            env_keys: vec![],
            timeout: Some(300),
            bundled: None,
            available_tools: vec![],
        };
        let json = serde_json::to_value(&stdio).unwrap();
        for forbidden in ["tier", "privacy", "privacy_tier"] {
            assert!(
                json.get(forbidden).is_none(),
                "`{forbidden}` is serialised onto the config entry, which is exactly the \
                 locally-forgeable value DR-23 removed: {json}"
            );
        }

        let hand_edited: ExtensionConfig = serde_json::from_value(serde_json::json!({
            "type": "stdio",
            "name": "cdwagent",
            "description": "clinical",
            "cmd": "uv",
            "args": ["run"],
            "timeout": 300,
            "tier": "public",
            "privacy": "public",
        }))
        .expect("an unknown key is ignored, not an error");
        assert_eq!(
            serde_json::to_value(&hand_edited).unwrap().get("tier"),
            None,
            "a hand-written tier survived a round trip through the config entry"
        );
        assert_eq!(
            crate::privacy::classify_extension(&hand_edited.name()),
            crate::privacy::ProviderTier::Private,
            "the hand-written tier changed the answer"
        );
    }

    /// **Step 3.3's other half: the tier is COMPUTED at read time, not stamped
    /// at admission.** This is what guards the field DR-23 deleted.
    ///
    /// `Extension.tier` was a `ProviderTier` written once by `add_extension`,
    /// `add_client` and `add_inprocess_server` and read by every gate — a
    /// snapshot of the answer at admission, which is precisely what "re-derived
    /// per read" forbids. Deleting a field is not something a test can observe
    /// directly; what it can observe is the behaviour only a *stamp* has, so
    /// this drives the one input that can change AFTER admission and asserts the
    /// gate notices.
    ///
    /// The entry is admitted while nothing ties it to a private install, and the
    /// gate lets it through. The provenance record then appears — exactly what a
    /// marketplace install writes into `extension-provenance.json` while the
    /// daemon is already running — and the same gate, on the same manager, on
    /// the same admitted entry, must now refuse. Any implementation that stamps
    /// at admission serves the stale Public answer and fails here, whatever it
    /// calls the field.
    ///
    /// ⚠ It also pins the direction that matters operationally: a freshly
    /// installed private extension is refused from the next lookup, not from the
    /// next restart. Without that, "install and use immediately" is a window in
    /// which enforcement is off.
    #[tokio::test]
    async fn the_tier_is_resolved_at_read_time_not_stamped_at_admission() {
        let install_dir = "/home/researcher/.config/biorouter/extensions/StampCheck";
        let (_dir, em, _sm, id) = manager_with_a_session().await;
        em.add_client(
            "stampcheck".to_string(),
            renamed_entry("stampcheck", install_dir),
            Arc::new(MockClient {}),
            None,
            None,
        )
        .await;

        // Before the record exists, this entry is public by every route.
        let dispatched = em
            .dispatch_tool_call(
                &id,
                call("stampcheck__tool"),
                crate::privacy::CallCapability::for_test(
                    crate::privacy::ProviderTier::Public,
                    true,
                ),
                CancellationToken::default(),
            )
            .await
            .expect("nothing yet says this entry is private");
        dispatched.result.await.expect("the mock client answers");
        assert!(
            em.get_prefixed_tools(None)
                .await
                .unwrap()
                .iter()
                .any(|t| t.name.as_ref() == "stampcheck__tool"),
            "Gate E hid an entry nothing had classified private"
        );

        // The install's record lands after admission. A stamped tier cannot see
        // this; a resolver called at the point of decision must.
        crate::privacy::provenance::insert_test_record_at(
            "stampcheck-as-installed",
            "cdwagent",
            Some(install_dir),
        );

        assert!(
            em.dispatch_tool_call(
                &id,
                call("stampcheck__tool"),
                crate::privacy::CallCapability::for_test(
                    crate::privacy::ProviderTier::Public,
                    true
                ),
                CancellationToken::default(),
            )
            .await
            .is_err(),
            "Gate C served an answer decided at admission, which is the stored tier DR-23 removed"
        );
        assert!(
            !em.get_prefixed_tools(None)
                .await
                .unwrap()
                .iter()
                .any(|t| t.name.as_ref() == "stampcheck__tool"),
            "Gate E served an answer decided at admission"
        );
    }

    /// DR-15's master opt-out is read INSIDE the gate, through the capability
    /// the call was admitted on. With enforcement off the same call goes
    /// through — one auditable branch rather than an absent gate — and the
    /// RATCHET stops with it.
    ///
    /// The name says "silences" rather than "is the only thing that lets it
    /// through", which is what this was first called: a private caller lets the
    /// same call through too, and `a_permitted_private_dispatch_ratchets_the_session`
    /// is that direction.
    ///
    /// ⚠ The second half is AR-7, which is binding and easy to get backwards —
    /// the first implementation of this task ratcheted anyway, on the argument
    /// that "a session that queried OMOP is still a session that queried OMOP".
    /// The plan considered exactly that and rejected it: *"it would silently
    /// privatise sessions a user believes are unprotected, and the first they
    /// would learn of it is a refusal weeks later when they turn the feature
    /// back on."* The toggle means what it says; it does not secretly keep a
    /// ledger. Task 30's own matrix pins this from the other end
    /// (`nothing_ratchets_while_the_toggle_is_off_and_re_enabling_does_not_backfill`),
    /// so an implementation that ratchets here cannot pass that task either.
    #[tokio::test]
    async fn the_master_opt_out_silences_the_refusal_and_the_ratchet_with_it() {
        let (_dir, em, sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;

        let dispatched = em
            .dispatch_tool_call(
                &id,
                call("ucsfomopagent__tool"),
                crate::privacy::CallCapability::for_test(
                    crate::privacy::ProviderTier::Public,
                    false,
                ),
                CancellationToken::default(),
            )
            .await
            .expect("with the feature off nothing is refused");
        dispatched.result.await.expect("the mock client answers");

        let row = sm.get_session(&id, false).await.unwrap();
        assert_eq!(
            row.privacy_tier,
            crate::privacy::SessionClassification::Public,
            "AR-7: with the master opt-out off, DR-4's triggers do not fire, so a \
             classification written while the user believes the feature is off is \
             permanent, and re-enabling never revisits it"
        );
        assert_eq!(row.privacy_reason, None);
    }

    // ----------------------------------------------------------------------
    // Issue #56 Task 15: Gate C's siblings — the eight ways to reach an MCP
    // server that are NOT a tool call. `dispatch_tool_call` is a complete choke
    // point for tool calls and for nothing else.
    //
    // ⚠ `search_available_extensions` is absent from THIS list because it is not
    // one of the eight: it never contacts a server and returns no
    // server-authored content, so there is nothing here for it to inherit.
    //
    // ⚠ **What this comment used to say next was wrong, and the correction is
    // the point.** It excused the function outright — "it does reveal that a
    // private extension is INSTALLED, which is an existence leak and explicitly
    // out of scope (DR-7)" — which contradicted Gate E's own stated reason for
    // hiding a private extension from a public model, namely that the tool's
    // *existence* is the secret (see `extension_reach` and
    // `gate_e_lists_and_marks_a_mismatched_extensions_tools`). Two surfaces, two
    // opposite rules, one axis. The existence claim governs; DR-7's exclusion is
    // about inference side channels, not about printing a private connector's
    // name and marketplace description into a public model's context. That
    // function is now filtered by Gate E's own verdict — see its doc comment and
    // `search_available_extensions_hides_a_private_extension_from_a_public_model`
    // — so it belongs to Gate E, not to Gate C's siblings, and its absence here
    // is a taxonomy fact rather than an exemption.
    // ----------------------------------------------------------------------

    /// A stub MCP client that answers every read the siblings perform, counts
    /// how many times it was contacted, and stamps `SENTINEL-<label>` into every
    /// payload it returns.
    ///
    /// The counter and the sentinel answer two different questions and these
    /// tests need both. The sentinel answers "did this server's content reach
    /// the caller". The counter answers "was this server contacted at all" — and
    /// for `read_resource_tool`'s fan-out, which swallows a failure and tries the
    /// next extension, only the counter can tell a probe that SKIPPED the private
    /// server from one that ASKED it and threw the answer away. The rendered text
    /// is identical in both cases.
    #[derive(Clone)]
    struct CountingClient {
        label: &'static str,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl CountingClient {
        fn new(label: &'static str) -> Self {
            Self {
                label,
                calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }

        fn hit(&self) {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }

        fn contacted(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        /// The token every payload this server authors carries.
        fn sentinel(&self) -> String {
            format!("SENTINEL-{}", self.label)
        }
    }

    #[async_trait::async_trait]
    impl McpClientTrait for CountingClient {
        fn get_info(&self) -> Option<&InitializeResult> {
            None
        }

        async fn list_resources(
            &self,
            _next_cursor: Option<String>,
            _cancellation_token: CancellationToken,
        ) -> Result<ListResourcesResult, Error> {
            use rmcp::model::AnnotateAble;
            self.hit();
            Ok(ListResourcesResult {
                resources: vec![rmcp::model::RawResource::new(
                    "res://x",
                    format!("{}-resource", self.sentinel()),
                )
                .no_annotation()],
                next_cursor: None,
                meta: None,
            })
        }

        async fn read_resource(
            &self,
            uri: &str,
            _cancellation_token: CancellationToken,
        ) -> Result<ReadResourceResult, Error> {
            self.hit();
            Ok(ReadResourceResult {
                contents: vec![ResourceContents::TextResourceContents {
                    uri: uri.to_string(),
                    mime_type: Some("text/plain".to_string()),
                    text: format!("from the {} server ({})", self.label, self.sentinel()),
                    meta: None,
                }],
            })
        }

        async fn list_tools(
            &self,
            _next_cursor: Option<String>,
            _cancellation_token: CancellationToken,
        ) -> Result<ListToolsResult, Error> {
            Ok(ListToolsResult {
                tools: vec![],
                next_cursor: None,
                meta: None,
            })
        }

        async fn call_tool(
            &self,
            _name: &str,
            _arguments: Option<JsonObject>,
            _meta: McpMeta,
            _cancellation_token: CancellationToken,
        ) -> Result<CallToolResult, Error> {
            self.hit();
            Ok(CallToolResult {
                content: vec![],
                is_error: None,
                structured_content: None,
                meta: None,
            })
        }

        async fn list_prompts(
            &self,
            _next_cursor: Option<String>,
            _cancellation_token: CancellationToken,
        ) -> Result<ListPromptsResult, Error> {
            self.hit();
            Ok(ListPromptsResult {
                prompts: vec![Prompt::new(
                    format!("{}-prompt", self.sentinel()),
                    Some("a prompt"),
                    None,
                )],
                next_cursor: None,
                meta: None,
            })
        }

        async fn get_prompt(
            &self,
            _name: &str,
            _arguments: Value,
            _cancellation_token: CancellationToken,
        ) -> Result<GetPromptResult, Error> {
            self.hit();
            // An MCP prompt body is server-authored text that lands in the
            // transcript verbatim, so it is CONTENT, not a name.
            Ok(GetPromptResult {
                description: Some(format!("{}-PROMPT-BODY", self.sentinel())),
                messages: vec![rmcp::model::PromptMessage::new_text(
                    rmcp::model::PromptMessageRole::User,
                    format!("{}-PROMPT-BODY", self.sentinel()),
                )],
            })
        }

        async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
            mpsc::channel(1).1
        }
    }

    /// A **real** provider at the requested tier — never a mock.
    ///
    /// `Provider::tier` has an enumerating gate over its implementations
    /// (`providers::tier_tests`, Task 5 Step 5) that lists the production ones
    /// for a human to read; a seventh, test-only implementation here would have
    /// to be read past on every run. An Ollama-engine provider's tier is a pure
    /// function of the base URL it resolved (`self_hosted_tier`), so loopback
    /// yields Private and a host off the machine yields Public, with no network
    /// touched at construction.
    fn provider_at(
        tier: crate::privacy::ProviderTier,
    ) -> Arc<dyn crate::providers::base::Provider> {
        use crate::providers::base::Provider;
        let base_url = match tier {
            crate::privacy::ProviderTier::Private => "http://localhost:11434",
            crate::privacy::ProviderTier::Public => "https://ollama.example.test",
        };
        let provider = crate::providers::ollama::OllamaProvider::from_custom_config(
            crate::model::ModelConfig::new_or_fail("qwen3"),
            crate::config::declarative_providers::DeclarativeProviderConfig {
                name: "sibling-fixture".to_string(),
                engine: crate::config::declarative_providers::ProviderEngine::Ollama,
                display_name: "Sibling Fixture".to_string(),
                description: None,
                api_key_env: "NOT_USED".to_string(),
                base_url: base_url.to_string(),
                models: vec![],
                headers: None,
                timeout_seconds: None,
                supports_streaming: None,
            },
        )
        .expect("a declarative ollama provider must construct");
        assert_eq!(
            provider.tier(),
            tier,
            "the fixture only discriminates if its provider really is at this tier"
        );
        Arc::new(provider)
    }

    /// A manager bound to a provider at `caller`, holding the private
    /// `ucsfomopagent` and — when `with_public` — the public `developer`.
    ///
    /// Both stubs are admitted through the real `add_client`, so their tier is
    /// stamped by the same rule a real extension's is, and both advertise
    /// resources and prompts: `list_resources`' fan-out filters on
    /// `supports_resources()`, so a server with no `ServerInfo` would make that
    /// probe iterate over nothing and pass vacuously.
    async fn siblings_fixture(
        caller: crate::privacy::ProviderTier,
        with_public: bool,
    ) -> (TempDir, ExtensionManager, CountingClient, CountingClient) {
        let dir = tempfile::tempdir().unwrap();
        let session_manager = Arc::new(crate::session::SessionManager::new(
            dir.path().to_path_buf(),
        ));
        let em = ExtensionManager::new(
            Arc::new(Mutex::new(Some(provider_at(caller)))),
            session_manager,
        );

        let info = ServerInfo {
            capabilities: rmcp::model::ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .enable_resources()
                .build(),
            ..Default::default()
        };
        let private = CountingClient::new("private");
        let public = CountingClient::new("public");

        for (name, client) in [
            ("ucsfomopagent", private.clone()),
            ("developer", public.clone()),
        ] {
            if name == "developer" && !with_public {
                continue;
            }
            em.add_client(
                normalize(name),
                ExtensionConfig::Builtin {
                    name: name.to_string(),
                    display_name: Some(name.to_string()),
                    description: "built-in".to_string(),
                    timeout: None,
                    bundled: None,
                    available_tools: vec![],
                },
                Arc::new(client),
                Some(info.clone()),
                None,
            )
            .await;
        }

        (dir, em, private, public)
    }

    /// Every non-dispatch entry point that reaches an MCP server, by the name of
    /// the function it exercises. `read_resource_tool` and `list_resources` each
    /// appear twice because their two branches are different code paths: one
    /// names an extension, the other probes every installed one in turn.
    const SIBLING_PROBES: &[&str] = &[
        "read_resource_tool (fan-out)",
        "read_resource_tool (named)",
        "read_resource",
        "list_resources_from_extension",
        "list_resources (named)",
        "list_resources (fan-out)",
        "list_prompts_from_extension",
        "list_prompts",
        "get_prompt",
    ];

    /// Drive one probe and render its WHOLE outcome — `Ok` and `Err` alike — so
    /// a single substring scan can ask whether any server-authored content came
    /// back.
    async fn run_sibling(em: &ExtensionManager, probe: &str) -> String {
        let tok = CancellationToken::default;
        match probe {
            "read_resource_tool (fan-out)" => format!(
                "{:?}",
                em.read_resource_tool(serde_json::json!({ "uri": "res://x" }), None, tok())
                    .await
            ),
            "read_resource_tool (named)" => format!(
                "{:?}",
                em.read_resource_tool(
                    serde_json::json!({ "uri": "res://x", "extension_name": "ucsfomopagent" }),
                    None,
                    tok()
                )
                .await
            ),
            "read_resource" => format!(
                "{:?}",
                em.read_resource("res://x", "ucsfomopagent", None, tok())
                    .await
            ),
            "list_resources_from_extension" => format!(
                "{:?}",
                em.list_resources_from_extension("ucsfomopagent", None, tok())
                    .await
            ),
            "list_resources (named)" => format!(
                "{:?}",
                em.list_resources(
                    serde_json::json!({ "extension": "ucsfomopagent" }),
                    None,
                    tok()
                )
                .await
            ),
            "list_resources (fan-out)" => format!(
                "{:?}",
                em.list_resources(serde_json::json!({}), None, tok()).await
            ),
            "list_prompts_from_extension" => format!(
                "{:?}",
                em.list_prompts_from_extension("ucsfomopagent", tok()).await
            ),
            "list_prompts" => format!("{:?}", em.list_prompts(tok()).await),
            "get_prompt" => format!(
                "{:?}",
                em.get_prompt("ucsfomopagent", "cohort", serde_json::json!({}), tok())
                    .await
            ),
            other => panic!("unknown sibling probe: {other}"),
        }
    }

    /// The task's headline. Not one of the eight may reach the private server
    /// while the session is bound to a public model, and not one may hand back
    /// anything that server authored.
    ///
    /// ⚠ Run against BOTH shapes of installation, and the private-only one is
    /// the load-bearing half. `read_resource_tool`'s fan-out iterates a
    /// `HashMap` (`RandomState`) and returns on the FIRST success, so with a
    /// public extension also installed an unguarded implementation answers from
    /// `developer` roughly half the time, never touches the private stub, and
    /// passes both assertions — a coin flip on the probe the task calls the
    /// worst of them. With `ucsfomopagent` alone there is nothing else to
    /// answer, so every probe must refuse or it fails deterministically. The
    /// mixed shape is kept because it is the realistic installation and it is
    /// what pins the aggregating probes.
    ///
    /// ⚠ `sentinel()` is server-authored CONTENT. The private extension's NAME
    /// still appears in `read_resource_tool`'s `RESOURCE_NOT_FOUND` list and in
    /// Gate C's own refusal, deliberately: DR-7 puts the existence of an
    /// installed extension out of scope, and §14.4 has the refusal name the
    /// extension so the model can tell the user what to switch. This asserts on
    /// content, and only content.
    #[tokio::test]
    async fn no_sibling_entry_point_reaches_a_private_extension_under_a_public_model() {
        for with_public in [false, true] {
            for probe in SIBLING_PROBES {
                let (_dir, em, private, _public) =
                    siblings_fixture(crate::privacy::ProviderTier::Public, with_public).await;
                let rendered = run_sibling(&em, probe).await;
                assert_eq!(
                    private.contacted(),
                    0,
                    "{probe} contacted the private server \
                     (public extension installed: {with_public}; returned: {rendered})"
                );
                assert!(
                    !rendered.contains(&private.sentinel()),
                    "{probe} leaked the private server's content \
                     (public extension installed: {with_public}): {rendered}"
                );
            }
        }
    }

    /// The other direction, without which the guard could be "always refuse a
    /// private extension" and every assertion above would still pass. A private
    /// model may reach a private extension; the boundary is the tier pair, not
    /// the extension.
    #[tokio::test]
    async fn every_sibling_entry_point_still_reaches_a_private_extension_under_a_private_model() {
        for probe in SIBLING_PROBES {
            // No public extension: the two fan-out probes that return on the
            // first success would otherwise be settled by whichever key the
            // HashMap yielded first.
            let (_dir, em, private, _public) =
                siblings_fixture(crate::privacy::ProviderTier::Private, false).await;
            let rendered = run_sibling(&em, probe).await;
            assert!(
                private.contacted() > 0,
                "{probe} refused a private extension to a PRIVATE caller (returned: {rendered})"
            );
        }
    }

    /// `read_resource_tool` with no `extension_name` probes every extension in
    /// turn and swallows failures (`Err(_) => continue`). If the guard is a
    /// single up-front check the whole call fails; placed inside the loop, the
    /// private server is skipped and the public one still answers.
    ///
    /// ⚠ The call COUNTER is the assertion that discriminates, not the returned
    /// text: in the buggy case where the private server was contacted first and
    /// its answer discarded, the text is identical.
    #[tokio::test]
    async fn the_resource_fanout_still_serves_the_public_extension() {
        let (_dir, em, private, public) =
            siblings_fixture(crate::privacy::ProviderTier::Public, true).await;

        let out = em
            .read_resource_tool(
                serde_json::json!({ "uri": "res://x" }),
                None,
                CancellationToken::default(),
            )
            .await
            .expect("one private extension must not cost a public model its resource reads");

        assert!(
            format!("{out:?}").contains("from the public server"),
            "{out:?}"
        );
        assert_eq!(
            private.contacted(),
            0,
            "the fan-out asked the private server and swallowed the answer"
        );
        assert!(public.contacted() > 0, "the public server was never asked");
    }

    /// The same property for the prompt fan-out: one private extension must not
    /// empty a public model's prompt listing.
    #[tokio::test]
    async fn the_prompt_fanout_still_serves_the_public_extension() {
        let (_dir, em, private, public) =
            siblings_fixture(crate::privacy::ProviderTier::Public, true).await;

        let prompts = em
            .list_prompts(CancellationToken::default())
            .await
            .expect("prompt listing");
        assert!(prompts.contains_key("developer"), "{prompts:?}");
        assert!(!prompts.contains_key("ucsfomopagent"), "{prompts:?}");

        assert_eq!(private.contacted(), 0);
        assert!(public.contacted() > 0);
    }

    /// `list_resources`' fan-out collects its per-extension failures into one
    /// bucket, and that bucket now carries two very different things: a server
    /// that could not be listed, which is an error, and Gate C declining to
    /// reach a private extension, which is the design. Logging the second at
    /// ERROR puts a full refusal in the log on every listing a public model
    /// performs with one private extension installed — `list_prompts`' fan-out
    /// already uses `debug!` for exactly this.
    ///
    /// The discriminator is the code, so this pins it against the REAL refusal
    /// and against the only two other errors that path can produce, spelled the
    /// way `list_resources_from_extension` spells them.
    #[test]
    fn a_gate_c_refusal_is_told_apart_from_a_real_listing_failure() {
        use crate::privacy::ProviderTier::{Private, Public};

        let refusal = crate::privacy::refusal::privacy_refusal("ucsfomopagent", Private, Public)
            .expect("a private extension and a public caller is a refusal");
        assert!(is_privacy_refusal(&refusal), "{refusal:?}");

        assert!(!is_privacy_refusal(&ErrorData::new(
            ErrorCode::INVALID_PARAMS,
            "Extension ucsfomopagent is not valid".to_string(),
            None,
        )));
        assert!(!is_privacy_refusal(&ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            "Unable to list resources for ucsfomopagent, TransportClosed".to_string(),
            None,
        )));
    }

    /// An MCP prompt body is server-authored text that lands in the transcript
    /// verbatim, so a refusal that echoed it would defeat the point of refusing.
    #[tokio::test]
    async fn get_prompt_refuses_without_echoing_the_prompt_body() {
        let (_dir, em, private, _public) =
            siblings_fixture(crate::privacy::ProviderTier::Public, false).await;

        let err = em
            .get_prompt(
                "ucsfomopagent",
                "cohort",
                serde_json::json!({}),
                CancellationToken::default(),
            )
            .await
            .expect_err("a public model must not fetch a private extension's prompt");

        let rendered = format!("{err:?}");
        assert!(
            !rendered.contains(&format!("{}-PROMPT-BODY", private.sentinel())),
            "{rendered}"
        );
        assert_eq!(private.contacted(), 0);
    }

    /// Two of the eight siblings are reachable from a TOOL CALL, so they are the
    /// two that have an admitted capability to inherit — and must inherit it.
    ///
    /// `extensionmanager__read_resource` and `extensionmanager__list_resources`
    /// arrive at the manager through `dispatch_tool_call`, which puts the
    /// capability the call was ADMITTED on into the call's [`McpMeta`]. Sampling
    /// a fresh one inside the driven future is exactly the read-then-read
    /// [`crate::privacy::CallCapability`] exists to prevent: the model asks for a
    /// resource while a public model is bound, the user switches to a private
    /// model mid-turn (Gate A permits that direction, and `update_provider` takes
    /// the provider mutex with no turn lock), and a resample would hand the
    /// Public-admitted call Private reach.
    ///
    /// Driven through the real `ExtensionManagerClient::call_tool`, not through
    /// the manager directly, because the defect this pins was the meta being
    /// bound as `_meta` and dropped — a manager-level test would pass with the
    /// wiring still absent.
    #[tokio::test]
    async fn a_public_admitted_resource_tool_call_does_not_gain_private_reach_mid_turn() {
        let (_dir, em, private, _public) =
            siblings_fixture(crate::privacy::ProviderTier::Public, false).await;
        let em = Arc::new(em);

        let client = crate::agents::extension_manager_extension::ExtensionManagerClient::new(
            PlatformExtensionContext {
                extension_manager: Some(Arc::downgrade(&em)),
                session_manager: em.context.session_manager.clone(),
            },
        )
        .expect("the extension-manager platform client constructs");

        // What `dispatch_tool_call` admitted: a public model was bound then.
        let admitted =
            crate::privacy::CallCapability::for_test(crate::privacy::ProviderTier::Public, true);

        // ...and the user switches to a private model while the turn is in
        // flight. A resample from here on reads Private.
        *em.provider.lock().await = Some(provider_at(crate::privacy::ProviderTier::Private));

        for (tool, args) in [
            (
                crate::agents::extension_manager_extension::READ_RESOURCE_TOOL_NAME,
                serde_json::json!({ "uri": "res://x", "extension_name": "ucsfomopagent" }),
            ),
            (
                crate::agents::extension_manager_extension::READ_RESOURCE_TOOL_NAME,
                serde_json::json!({ "uri": "res://x" }),
            ),
            (
                crate::agents::extension_manager_extension::LIST_RESOURCES_TOOL_NAME,
                serde_json::json!({ "extension": "ucsfomopagent" }),
            ),
            (
                crate::agents::extension_manager_extension::LIST_RESOURCES_TOOL_NAME,
                serde_json::json!({}),
            ),
        ] {
            let out = client
                .call_tool(
                    tool,
                    args.as_object().cloned(),
                    McpMeta::new("session", admitted),
                    CancellationToken::default(),
                )
                .await;
            let rendered = format!("{out:?}");
            assert!(
                !rendered.contains(&private.sentinel()),
                "{tool} {args} handed a Public-admitted call the private server's content: \
                 {rendered}"
            );
            assert_eq!(
                private.contacted(),
                0,
                "{tool} {args} resampled the provider and reached the private server on a \
                 capability admitted while a PUBLIC model was bound (returned: {rendered})"
            );
        }
    }

    // ----------------------------------------------------------------------
    // Issue #56 Task 16: Gate E — discovery.
    //
    // Not a veto: the reason a public model never sees a private server's tool
    // NAMES, DESCRIPTIONS or JSON SCHEMAS in its system prompt. Schema text is
    // content, and it is handed to the model before any tool call exists for
    // Gate C to refuse.
    // ----------------------------------------------------------------------

    /// A manager whose provider handle the test keeps, so it can swap the bound
    /// model the way `Agent::update_provider` does — by writing the mutex, with
    /// no extension change and therefore no `tools_cache_version` bump.
    fn manager_bound_to(
        tier: crate::privacy::ProviderTier,
    ) -> (TempDir, ExtensionManager, SharedProvider) {
        let dir = tempfile::tempdir().unwrap();
        let session_manager = Arc::new(crate::session::SessionManager::new(
            dir.path().to_path_buf(),
        ));
        let provider: SharedProvider = Arc::new(Mutex::new(Some(provider_at(tier))));
        let em = ExtensionManager::new(provider.clone(), session_manager);
        (dir, em, provider)
    }

    impl ExtensionManager {
        /// Admit a mock extension that the resolver will call PRIVATE, under a
        /// key the compiled marketplace snapshot does not itself publish.
        ///
        /// The hazard below needs a private key containing `__`, and
        /// `PRIVATE_EXTENSIONS` holds `cdwagent` and `ucsfomopagent`, so the
        /// snapshot alone cannot produce one. Before Task 43 the only way to
        /// build the fixture was to bypass admission and stamp
        /// `Extension.tier` by hand; now there is a real one, and it is the
        /// very mechanism DR-23 added — record the install's registry id and
        /// let the resolver do its job. So this goes through the SAME admission
        /// point as every other fixture in this file, and the divergence
        /// between the config name and the registry id is the point rather than
        /// an artefact of the fixture.
        ///
        /// The half that is constructible from the snapshot's own names is
        /// asserted separately by
        /// `the_two_gates_agree_on_a_key_that_contains_the_separator`.
        async fn add_mock_private_extension(&self, name: String, client: McpClientBox) {
            crate::privacy::provenance::insert_test_record(&name, "cdwagent");
            self.add_mock_extension(name, client).await;
        }
    }

    /// O6. `get_all_tools_cached` is guarded by `tools_cache_version`, which is
    /// bumped only by an extension add/remove — never by `update_provider`.
    /// Filtering anywhere upstream of `filter_tools` therefore freezes ONE
    /// model's allowed set into the cache and serves it to the next one. This is
    /// the assertion a cache-level implementation fails, and it is why the test
    /// changes the provider WITHOUT touching the extension set.
    #[tokio::test]
    async fn the_allowed_set_follows_a_mid_session_model_swap() {
        use crate::privacy::ProviderTier::{Private, Public};
        let (_dir, em, provider) = manager_bound_to(Private);
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;
        em.add_mock_extension("developer".to_string(), Arc::new(MockClient {}))
            .await;

        let before = em.get_prefixed_tools(None).await.unwrap();
        assert!(
            before
                .iter()
                .any(|t| t.name.as_ref().starts_with("ucsfomopagent__")),
            "a private model must see the private extension it is entitled to"
        );

        // Exactly what `Agent::update_provider` does, and nothing else.
        *provider.lock().await = Some(provider_at(Public));

        let after = em.get_prefixed_tools(None).await.unwrap();
        assert!(
            !after
                .iter()
                .any(|t| t.name.as_ref().starts_with("ucsfomopagent__")),
            "the cached list outlived the model it was filtered for: {:?}",
            after.iter().map(|t| t.name.as_ref()).collect::<Vec<_>>()
        );
        assert!(after
            .iter()
            .any(|t| t.name.as_ref().starts_with("developer__")));
        assert_ne!(before.len(), after.len());
    }

    /// `filter_tools` used to compute the prefix by splitting the tool name at
    /// its FIRST separator and taking the leading segment, while
    /// the dispatcher resolved by `starts_with` over a `HashMap` with
    /// per-process-randomised iteration order. `name_to_key` preserves `_`, so
    /// an extension whose `manifest.name` contains `__` keeps it in the map key.
    /// With keys `a` (public) and `a__b` (private), the tool `a__b__t` computes
    /// prefix `a` and would be ALLOWED, putting the private server's tool names,
    /// descriptions and JSON schemas into a public model's system prompt.
    #[tokio::test]
    async fn an_embedded_double_underscore_cannot_smuggle_a_private_tool_into_the_list() {
        use crate::privacy::ProviderTier::Public;
        let (_dir, em, _provider) = manager_bound_to(Public);
        em.add_mock_extension("a".to_string(), Arc::new(MockClient {}))
            .await;
        em.add_mock_private_extension("a__b".to_string(), Arc::new(MockClient {}))
            .await;

        let tools = em.get_prefixed_tools(None).await.unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            !names.iter().any(|n| n.starts_with("a__b__")),
            "leaked: {names:?}"
        );
        // The public sibling is still listed. Dropping BOTH is a different bug
        // with the same green result on the assertion above.
        assert!(names.contains(&"a__tool"), "{names:?}");
    }

    /// The half of the same hazard that IS constructible against the real
    /// registry, and the reason the two gates must share one resolver rather
    /// than one rule: `ucsfomopagent` is private and `ucsfomopagent__mirror` is
    /// public, so under a public model Gate C dispatches
    /// `ucsfomopagent__mirror__tool` (`the_tier_comes_from_the_resolved_record_not_the_tool_name`)
    /// and Gate E must list it. The naive prefix hides it — a tool the model is
    /// entitled to call and cannot see — while still hiding the private one.
    #[tokio::test]
    async fn the_two_gates_agree_on_a_key_that_contains_the_separator() {
        let (_dir, em, _provider) = manager_bound_to(crate::privacy::ProviderTier::Public);
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;
        em.add_mock_extension("ucsfomopagent__mirror".to_string(), Arc::new(MockClient {}))
            .await;

        let tools = em.get_prefixed_tools(None).await.unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            names.contains(&"ucsfomopagent__mirror__tool"),
            "Gate C dispatches this tool, so Gate E may not hide it: {names:?}"
        );
        assert!(
            !names.contains(&"ucsfomopagent__tool"),
            "the private extension's own tools stay out of a public model's prompt: {names:?}"
        );
    }

    /// Task 16's ⚠, as a test: the permission editors must NOT be tier-filtered.
    ///
    /// Gate E keeps a private server's tool names, descriptions and JSON schemas
    /// out of a public MODEL's context. The human who installed that server is
    /// not the model. Settings → Extensions → tool permissions and `biorouter
    /// configure`'s tool selector both exist to let that human set a permission
    /// per tool, and a tool that is not listed cannot be configured — so
    /// filtering there buys nothing (Settings is nobody's prompt) and costs the
    /// user the ability to administer the extension they installed.
    ///
    /// Both views are asserted in one test on purpose. Two tests could pass
    /// while the two views were the same function, which is the whole bug.
    #[tokio::test]
    async fn the_permission_editor_view_is_not_tier_filtered() {
        let (_dir, em, _provider) = manager_bound_to(crate::privacy::ProviderTier::Public);
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;
        em.add_mock_extension("developer".to_string(), Arc::new(MockClient {}))
            .await;

        let model_view: Vec<String> = em
            .get_prefixed_tools(None)
            .await
            .unwrap()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(
            !model_view.iter().any(|n| n.starts_with("ucsfomopagent__")),
            "the model's view is still Gate E filtered: {model_view:?}"
        );

        let editor_view: Vec<String> = em
            .get_prefixed_tools_unfiltered(None)
            .await
            .unwrap()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(
            editor_view.iter().any(|n| n.starts_with("ucsfomopagent__")),
            "a private extension must stay visible and badged in Settings: {editor_view:?}"
        );
        assert!(
            editor_view.iter().any(|n| n.starts_with("developer__")),
            "the public extension is still there too: {editor_view:?}"
        );

        // Settings asks one extension at a time (`?extension_name=`), so the
        // per-extension filter has to keep working on this path — and it is the
        // private extension it is asked about that matters.
        let scoped: Vec<String> = em
            .get_prefixed_tools_unfiltered(Some("ucsfomopagent".to_string()))
            .await
            .unwrap()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert!(!scoped.is_empty(), "the scoped editor view is empty");
        assert!(
            scoped.iter().all(|n| n.starts_with("ucsfomopagent__")),
            "the scoped editor view leaked another extension: {scoped:?}"
        );
    }

    /// The tool list and the key set Gate E resolves it against must come out of
    /// ONE read of the extension map.
    ///
    /// `get_prefixed_tools` used to read the tools and then read the keys: two
    /// reads of shared mutable state at two program points, which is a race by
    /// construction. Its one non-fail-closed skew was a concurrent removal —
    /// drop a private `a__b` between the reads while public `a` remains, and the
    /// tools already in hand still carry `a__b__*`, which now re-resolve to `a`
    /// and get listed. Reversing the order does not fix it, it moves the hole:
    /// a concurrent ADD then leaves a freshly installed private `a__b`'s tools
    /// resolving to `a` against a stale key set. Only pairing the two removes
    /// it.
    ///
    /// The pairing is what makes the skew unrepresentable, so the pairing is
    /// what is asserted: every tool in a snapshot resolves, against that same
    /// snapshot's keys, to the extension that actually named it.
    #[tokio::test]
    async fn the_tool_snapshot_carries_the_keys_that_named_its_tools() {
        let (_dir, em, _provider) = manager_bound_to(crate::privacy::ProviderTier::Public);
        em.add_mock_extension("a".to_string(), Arc::new(MockClient {}))
            .await;
        em.add_mock_extension("a__b".to_string(), Arc::new(MockClient {}))
            .await;

        let snapshot = em.get_all_tools_cached().await.unwrap();
        assert!(!snapshot.tools.is_empty(), "the fixture produced no tools");

        // Deliberately restated here rather than run through the production
        // resolver: a test that calls the function it is validating agrees with
        // it by construction, and the claim being made is about the DATA — that
        // these keys are the ones that formed these names — not about the
        // resolution rule, which its own two tests already pin.
        for tool in &snapshot.tools {
            let name = tool.name.as_ref();
            assert!(
                snapshot.keys.iter().any(|k| name
                    .strip_prefix(k.as_str())
                    .is_some_and(|rest| rest.starts_with("__"))),
                "{name} is named by no key in its own snapshot: {:?}",
                snapshot.keys
            );
        }

        // And specifically: BOTH halves of the overlapping pair are carried, so
        // the longer key is available to win. A snapshot that had lost `a__b`
        // would still pass the loop above — every `a__b__*` tool would simply
        // be attributed to `a`, which is exactly the misattribution this
        // pairing exists to prevent.
        assert!(
            snapshot.keys.iter().any(|k| k == "a"),
            "{:?}",
            snapshot.keys
        );
        assert!(
            snapshot.keys.iter().any(|k| k == "a__b"),
            "the snapshot dropped the longer of two overlapping keys: {:?}",
            snapshot.keys
        );
    }

    // ----------------------------------------------------------------------
    // Issue #56 Task 48: DR-26's third axis — affiliation.
    //
    // Tier asks how sensitive a thing is; affiliation asks under whose
    // agreements. UCSF's Versa reaching the UCSF OMOP agent is the arrangement
    // everyone approved; a model covered by ANOTHER institution's agreements
    // reaching it is a cross-institutional linkage nobody papered — and it
    // passes every tier gate above, because both endpoints are Private.
    //
    // ⚠ Every test below is the SAME fixture at a DIFFERENT surface, and each
    // is its own named test on purpose. A single test asserting "a mismatch is
    // refused" passes on an implementation that wired one gate and forgot the
    // other four, which is the failure mode DR-26's enumeration trap warns
    // about.
    // ----------------------------------------------------------------------

    /// A provider at a stated tier and affiliation, and nothing else.
    ///
    /// `provider_at` cannot express this: it builds a real `OllamaProvider`,
    /// whose affiliation is `Local` whenever its tier is Private, and `Local` is
    /// DR-26's *most permissive* value — a fixture built from it can never
    /// produce a mismatch. The institutional halves have no constructor outside
    /// their own module (`VersaAzureProvider::resolved_endpoint` is private), so
    /// they are stated, exactly as `providers::affiliation_tests::Half` states
    /// them.
    struct ProviderCoveredBy {
        tier: crate::privacy::ProviderTier,
        affiliation: Option<crate::privacy::affiliation::ModelAffiliation>,
    }

    #[async_trait::async_trait]
    impl crate::providers::base::Provider for ProviderCoveredBy {
        fn metadata() -> crate::providers::base::ProviderMetadata {
            crate::providers::base::ProviderMetadata::empty()
        }

        fn get_name(&self) -> &str {
            "covered-by"
        }

        fn get_model_config(&self) -> crate::model::ModelConfig {
            crate::model::ModelConfig::new_or_fail("nothing")
        }

        fn tier(&self) -> crate::privacy::ProviderTier {
            self.tier
        }

        fn affiliation(&self) -> Option<crate::privacy::affiliation::ModelAffiliation> {
            self.affiliation
        }

        async fn complete_with_model(
            &self,
            _model_config: &crate::model::ModelConfig,
            _system: &str,
            _messages: &[crate::conversation::message::Message],
            _tools: &[Tool],
        ) -> Result<
            (
                crate::conversation::message::Message,
                crate::providers::base::ProviderUsage,
            ),
            crate::providers::errors::ProviderError,
        > {
            unreachable!("this provider exists to be asked its affiliation and nothing else")
        }
    }

    fn covered_by(name: &str) -> Arc<dyn crate::providers::base::Provider> {
        Arc::new(ProviderCoveredBy {
            tier: crate::privacy::ProviderTier::Private,
            affiliation: Some(crate::privacy::affiliation::ModelAffiliation::institution(
                crate::privacy::affiliation::InstitutionId::new(name),
            )),
        })
    }

    fn a_local_model() -> Arc<dyn crate::providers::base::Provider> {
        Arc::new(ProviderCoveredBy {
            tier: crate::privacy::ProviderTier::Private,
            affiliation: Some(crate::privacy::affiliation::ModelAffiliation::Local),
        })
    }

    /// The capability a session bound to `institution`'s model carries.
    fn bound_to(institution: &str) -> crate::privacy::CallCapability {
        crate::privacy::CallCapability::for_test_affiliated(
            crate::privacy::ProviderTier::Private,
            true,
            Some(crate::privacy::affiliation::ModelAffiliation::institution(
                crate::privacy::affiliation::InstitutionId::new(institution),
            )),
        )
    }

    /// The capability a session bound to a LOCAL private model carries — the
    /// row of DR-26's table that must pass every surface below.
    fn bound_locally() -> crate::privacy::CallCapability {
        crate::privacy::CallCapability::for_test_affiliated(
            crate::privacy::ProviderTier::Private,
            true,
            Some(crate::privacy::affiliation::ModelAffiliation::Local),
        )
    }

    /// The refusal a dispatch produced, as text.
    ///
    /// ⚠ **`.is_err()` alone is not an assertion about this gate.** A dispatch
    /// can fail for a dozen unrelated reasons, so a test that only checks for an
    /// error passes just as happily against a mismatch that was never asked
    /// about. `ToolCallResult` carries no `Debug`, so `.expect()` is unavailable
    /// and the error is taken out by hand.
    async fn refusal_text(
        em: &ExtensionManager,
        session_id: &str,
        tool: &str,
        cap: crate::privacy::CallCapability,
    ) -> String {
        match em
            .dispatch_tool_call(session_id, call(tool), cap, CancellationToken::default())
            .await
        {
            Ok(_) => panic!("`{tool}` was dispatched, and this call had to be refused"),
            Err(e) => e.to_string(),
        }
    }

    /// The exact warning DR-26 requires, for the fixture every test here shares.
    /// Asserted against the composer rather than restated, so the copy cannot
    /// drift between the surfaces that state it.
    fn expected_warning(extension: &str, institution: &str) -> String {
        expected_finding(extension, institution).warning
    }

    /// The mark and the warning, from the one composer, for the shared fixture.
    fn expected_finding(extension: &str, institution: &str) -> crate::privacy::CrossAffiliation {
        bound_to(institution)
            .cross_affiliation(
                extension,
                &crate::privacy::resolve_extension(extension, None),
            )
            .expect("the fixture only discriminates if this pair really mismatches")
    }

    /// A manager holding the UCSF-affiliated `ucsfomopagent`, the unaffiliated
    /// public `custom` extension, and the `developer` capability, bound to
    /// `provider`.
    async fn affiliation_fixture(
        provider: Arc<dyn crate::providers::base::Provider>,
    ) -> (TempDir, ExtensionManager, SharedProvider) {
        let dir = tempfile::tempdir().unwrap();
        let session_manager = Arc::new(crate::session::SessionManager::new(
            dir.path().to_path_buf(),
        ));
        let handle: SharedProvider = Arc::new(Mutex::new(Some(provider)));
        let em = ExtensionManager::new(handle.clone(), session_manager);
        em.add_mock_third_party_extension("ucsfomopagent", Arc::new(MockClient {}))
            .await;
        em.add_mock_third_party_extension("custom", Arc::new(MockClient {}))
            .await;
        em.add_mock_extension("developer".to_string(), Arc::new(MockClient {}))
            .await;
        (dir, em, handle)
    }

    /// **Surface 1 — Gate C, dispatch.** Refused, with the warning that offers
    /// the approval.
    #[tokio::test]
    async fn gate_c_refuses_a_dispatch_across_an_affiliation_mismatch() {
        let (_dir, em, sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;

        let text = match em
            .dispatch_tool_call(
                &id,
                call("ucsfomopagent__tool"),
                bound_to("stanford"),
                CancellationToken::default(),
            )
            .await
        {
            Ok(_) => panic!("a Stanford-covered model reached a UCSF connector"),
            Err(e) => e.to_string(),
        };
        assert!(
            text.contains(&expected_warning("ucsfomopagent", "stanford")),
            "{text}"
        );

        // A refused call reached no connector, so it must not classify the chat
        // — the same rule the tier refusal follows.
        let row = sm.get_session(&id, false).await.unwrap();
        assert_eq!(
            row.privacy_tier,
            crate::privacy::SessionClassification::Public
        );
        assert_eq!(row.privacy_reason, None);
    }

    /// **Surface 1, the passing row.** The identical dispatch on a LOCAL model
    /// succeeds: `Local` is the most permissive affiliation, not a peer of the
    /// institutions.
    #[tokio::test]
    async fn gate_c_lets_a_local_model_dispatch_to_every_private_extension() {
        let (_dir, em, sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;

        let dispatched = em
            .dispatch_tool_call(
                &id,
                call("ucsfomopagent__tool"),
                bound_locally(),
                CancellationToken::default(),
            )
            .await
            .expect("a local model discloses nothing, so nothing needs papering");
        dispatched.result.await.expect("the mock client answers");

        // And the permitted private dispatch still ratchets, so this is a real
        // permit rather than a gate that stopped running.
        assert_eq!(
            sm.get_session(&id, false).await.unwrap().privacy_tier,
            crate::privacy::SessionClassification::Private
        );
    }

    /// Issue #56 DR-26 / Task 50 Step 3: "a private chat carries the affiliation
    /// of the extensions it touched."
    ///
    /// ⚠ **This is the load-bearing half of Step 3.** Chat recall and
    /// cross-session ingest both gate on `sessions.session_affiliations`; if
    /// nothing ever writes it, both gates read the empty set, permit everything,
    /// and every assertion at those surfaces still passes. So the recorder is
    /// asserted here, at the one place that writes it, through a real dispatch.
    ///
    /// It records only what an INSTITUTION claims: `developer` is private on no
    /// axis and `Any` claims nothing, so touching either adds no owner. A
    /// recorder that stamped every private extension would warn on recalls with
    /// no institutional boundary in them, which is the prompt fatigue DR-19
    /// rejects.
    #[tokio::test]
    async fn a_dispatch_records_the_extensions_institution_on_the_chat() {
        let (_dir, em, sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;
        em.add_mock_extension("developer".to_string(), Arc::new(MockClient {}))
            .await;

        assert!(
            sm.session_affiliations(&id).await.unwrap().is_empty(),
            "a fresh chat has touched nobody"
        );

        // An extension with no institutional claim adds no owner.
        em.dispatch_tool_call(
            &id,
            call("developer__tool"),
            bound_locally(),
            CancellationToken::default(),
        )
        .await
        .expect("the developer extension is reachable")
        .result
        .await
        .expect("the mock client answers");
        assert!(
            sm.session_affiliations(&id).await.unwrap().is_empty(),
            "an unclaimed extension put an institution on the chat"
        );

        // The UCSF connector does.
        em.dispatch_tool_call(
            &id,
            call("ucsfomopagent__tool"),
            bound_locally(),
            CancellationToken::default(),
        )
        .await
        .expect("a local model reaches every private extension")
        .result
        .await
        .expect("the mock client answers");

        let owners = sm.session_affiliations(&id).await.unwrap();
        assert_eq!(
            owners,
            std::collections::BTreeSet::from([crate::privacy::affiliation::InstitutionId::new(
                "ucsf"
            )]),
            "the chat does not carry the institution whose connector it just queried"
        );

        // Monotone: a later dispatch neither duplicates nor clears it.
        em.dispatch_tool_call(
            &id,
            call("developer__tool"),
            bound_locally(),
            CancellationToken::default(),
        )
        .await
        .expect("still reachable")
        .result
        .await
        .expect("the mock client answers");
        assert_eq!(sm.session_affiliations(&id).await.unwrap().len(), 1);
    }

    /// ⚠ **The chat-side ratchet fires on extension dispatch and nowhere else —
    /// a known residual, recorded here so it cannot quietly become an asymmetric
    /// one.**
    ///
    /// Review asked whether content entering a chat from somewhere *other* than
    /// a connector launders the axis: a chat on a local model runs `kb_search`
    /// against a UCSF-owned base (permitted — a local model transfers nothing),
    /// or recalls a UCSF chat, and its transcript now holds UCSF content while
    /// `session_affiliations` stays empty. It does, and it is deliberate scope:
    /// Task 50 Step 3 is "a private chat carries the affiliation of the
    /// **extensions it touched**", and the *tier* axis has had exactly the same
    /// boundary since Task 10 — `raise_session_privacy` is called from the same
    /// single place, so reading a private knowledge base does not privatise the
    /// reading chat either. Widening it is one change to both axes, not a patch
    /// to this one.
    ///
    /// What this pins is that symmetry. The two chat-side ratchets have one
    /// production call site each and it is the same site, so widening the tier
    /// trigger without widening the affiliation trigger — which WOULD be a new
    /// hole, a chat carrying an institution's content and not its owner — fails
    /// the build.
    ///
    /// A tripwire over one spelling, in the shape
    /// `grant::tests::the_grant_is_recorded_in_exactly_one_place` and
    /// `tier_user`'s two audits already use.
    #[test]
    fn the_two_chat_side_ratchets_share_one_production_call_site() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let crates = root.join("crates");
        assert!(crates.is_dir(), "the audit walks {}", crates.display());

        // Composed, so this file's own audit lines are not call sites.
        let tier_ratchet = concat!(".raise_session_", "privacy(");
        let affiliation_ratchet = concat!(".record_session_", "affiliation(");

        let mut tier_sites: Vec<String> = vec![];
        let mut affiliation_sites: Vec<String> = vec![];
        let mut scanned = 0usize;
        for entry in walkdir::WalkDir::new(&crates) {
            let entry = entry.expect("the audit must not silently skip an unreadable directory");
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = p
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            // The storage module DECLARES both halves and delegates one to the
            // other; counting it would make every number below larger and
            // indistinguishable from a real second trigger.
            if rel == "crates/biorouter/src/session/session_manager.rs" {
                continue;
            }
            scanned += 1;
            let src = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("the audit could not read {rel}: {e}"));
            // Production only. Every `mod tests` in this tree sits below an
            // UNINDENTED `#[cfg(test)]`, and the recall / ingest gates are
            // asserted by setting the column directly from their own tests —
            // fixture setup, not a trigger.
            //
            // ⚠ The `\n` is load-bearing: an indented `#[cfg(test)]` on a
            // test-only *method* appears far above the module in several files
            // here (this one at line 783), and matching it truncated production
            // to the first 800 lines and reported an empty set — a census that
            // finds nothing and a clean tree look identical, which is why the
            // assertion below names the expected site rather than counting.
            // `split(..).next()` rather than `find` + a byte slice: the index
            // would be at a char boundary here, but the slice is what clippy's
            // `string_slice` flags and this needs no index at all.
            let production = src.split("\n#[cfg(test)]").next().unwrap_or(src.as_str());
            let names = |needle: &str| {
                production
                    .lines()
                    .any(|l| !l.trim_start().starts_with("//") && l.contains(needle))
            };
            if names(tier_ratchet) {
                tier_sites.push(rel.clone());
            }
            if names(affiliation_ratchet) {
                affiliation_sites.push(rel.clone());
            }
        }
        assert!(
            scanned >= 400,
            "only {scanned} .rs files were scanned. A broken walk reports the same \
             empty set as a clean tree."
        );
        tier_sites.sort();
        affiliation_sites.sort();
        assert_eq!(
            tier_sites,
            vec!["crates/biorouter/src/agents/extension_manager.rs".to_string()],
            "the session tier ratchet gained a trigger. Whatever it is, the \
             affiliation ratchet needs the same one, or a chat can come to hold \
             an institution's content without recording its owner."
        );
        assert_eq!(
            affiliation_sites, tier_sites,
            "the two chat-side ratchets no longer fire from the same place. They \
             answer the same question about one turn (what did this chat just \
             take in), and a chat that ratcheted one axis and not the other is \
             recallable by a model no one decided may see it."
        );
    }

    /// **Surface 2 — Gate E, discovery.** Listed and MARKED, never hidden.
    ///
    /// ⚠ Gate E hides a private extension from a public model because the
    /// tool's *existence* is the secret. That reasoning does not carry here: in
    /// a mismatch both endpoints are Private and the user is entitled to know
    /// the connector exists. Hiding it would also let the agent silently route
    /// around a tool it cannot see, with no one told why.
    #[tokio::test]
    async fn gate_e_lists_and_marks_a_mismatched_extensions_tools() {
        let (_dir, em, _handle) = affiliation_fixture(covered_by("stanford")).await;

        let tools = em.get_prefixed_tools(None).await.unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            names.contains(&"ucsfomopagent__tool"),
            "a mismatch is marked, not hidden: {names:?}"
        );
        assert!(names.contains(&"developer__tool"), "{names:?}");

        let marked = tools
            .iter()
            .find(|t| t.name.as_ref() == "ucsfomopagent__tool")
            .and_then(|t| t.description.clone())
            .expect("the marked tool keeps a description");
        let finding = expected_finding("ucsfomopagent", "stanford");
        assert!(marked.contains(&finding.mark), "{marked}");
        // DR-26's specificity survives the shortening: both institutions and
        // the fact of the refusal are still in the only text the model reads
        // before a call exists.
        assert!(marked.contains("ucsf"), "{marked}");
        assert!(marked.contains("stanford"), "{marked}");

        // ⚠ **The budgeted mark, not the full paragraph.** A tool
        // `description` is a bounded field on a real API (Azure OpenAI caps it
        // at 1024 characters, and Versa Azure is one), and this mark is
        // prepended to EVERY tool of the mismatched extension. Marking a tool
        // must not be what makes the request unsendable.
        assert!(
            !marked.contains(&finding.warning),
            "the full compliance paragraph must not ride in a tool description: {marked}"
        );
        assert!(
            marked.len() < 1024,
            "a marked description reached {} bytes, past Azure OpenAI's cap: {marked}",
            marked.len()
        );

        // The unaffiliated sibling is untouched, or the mark is decoration
        // rather than a decision.
        let sibling = tools
            .iter()
            .find(|t| t.name.as_ref() == "developer__tool")
            .and_then(|t| t.description.clone())
            .expect("the public tool keeps its description");
        assert!(!sibling.contains("Cross-institutional"), "{sibling}");
    }

    /// **Surface 2, the passing row.** A local model sees the same list with no
    /// mark on it.
    #[tokio::test]
    async fn gate_e_marks_nothing_for_a_local_model() {
        let (_dir, em, _handle) = affiliation_fixture(a_local_model()).await;

        let tools = em.get_prefixed_tools(None).await.unwrap();
        assert!(tools
            .iter()
            .any(|t| t.name.as_ref() == "ucsfomopagent__tool"));
        for tool in &tools {
            let described = tool.description.clone().unwrap_or_default();
            assert!(
                !described.contains("Cross-institutional"),
                "{}: {described}",
                tool.name
            );
        }
    }

    /// **Surface 3 — Gate F, the extension channels.** `read_resource`,
    /// `list_resources`, `list_prompts` and `get_prompt` reach a server without
    /// being a tool call, so they refuse the same way Gate C does.
    #[tokio::test]
    async fn gate_f_refuses_an_extension_channel_across_an_affiliation_mismatch() {
        let (_dir, em, _handle) = affiliation_fixture(covered_by("stanford")).await;

        let err = em
            .assert_extension_reachable("ucsfomopagent", Some(bound_to("stanford")))
            .await
            .expect_err("a Stanford-covered model may not probe a UCSF connector's resources");
        assert!(
            err.message
                .to_string()
                .contains(&expected_warning("ucsfomopagent", "stanford")),
            "{}",
            err.message
        );

        // The public sibling is still reachable, so this is a decision rather
        // than a blanket refusal.
        em.assert_extension_reachable("developer", Some(bound_to("stanford")))
            .await
            .expect("an unaffiliated extension is reachable from any private model");
    }

    /// **Surface 3, the passing row.**
    #[tokio::test]
    async fn gate_f_lets_a_local_model_reach_every_extension_channel() {
        let (_dir, em, _handle) = affiliation_fixture(a_local_model()).await;
        em.assert_extension_reachable("ucsfomopagent", Some(bound_locally()))
            .await
            .expect("a local model discloses nothing");
        em.assert_extension_reachable("developer", Some(bound_locally()))
            .await
            .expect("…and the public sibling too");
    }

    /// **Surface 4 — bind.** Binding a model states how many enabled extensions
    /// it is incompatible with, naming each. It does not block: DR-19 on the
    /// third axis, and a blocked-outright design is one researchers route around
    /// by turning the feature off.
    ///
    /// ⚠ The provider is swapped the way `Agent::update_provider` swaps it — by
    /// writing the mutex — so a warning computed from a cache rather than from
    /// the bound model fails here.
    #[tokio::test]
    async fn binding_a_foreign_institutions_model_warns_about_every_incompatible_extension() {
        let (_dir, em, handle) = affiliation_fixture(a_local_model()).await;
        assert!(
            em.cross_affiliation_warnings(None).await.is_empty(),
            "a local model is compatible with everything"
        );

        *handle.lock().await = Some(covered_by("stanford"));

        let warnings = em.cross_affiliation_warnings(None).await;
        assert_eq!(
            warnings.len(),
            1,
            "exactly the UCSF connector mismatches; `developer` is unaffiliated: {warnings:?}"
        );
        assert_eq!(warnings[0].0, "ucsfomopagent");
        assert_eq!(warnings[0].1, expected_warning("ucsfomopagent", "stanford"));
    }

    /// The bind warning inherits an ADMITTED capability when it is given one,
    /// rather than resampling — the read-then-read `CallCapability` exists to
    /// close. Surface 5 (extension enablement) is asserted in
    /// `extension_manager_extension.rs`, beside the predicate that owns it.
    #[tokio::test]
    async fn the_bind_warning_uses_the_capability_it_was_handed() {
        let (_dir, em, _handle) = affiliation_fixture(a_local_model()).await;
        let warnings = em
            .cross_affiliation_warnings(Some(bound_to("stanford")))
            .await;
        assert_eq!(
            warnings.len(),
            1,
            "the handed capability was ignored and the bound provider resampled: {warnings:?}"
        );
        assert_eq!(warnings[0].0, "ucsfomopagent");
    }

    /// Issue #56 Task 49, gate (1). A granted triple permits the dispatch the
    /// mismatch refused a moment earlier — and the **same** extension after
    /// re-binding to another institution's model does not, because the triple
    /// the user accepted no longer exists.
    ///
    /// ⚠ The grant is minted here through the module's own test door, never by
    /// anything reachable from a tool call: `X-User-Action` is an HTTP header
    /// and has no channel on a dispatch path, which is precisely why the flow
    /// is refuse → tell the user → grant over HTTP → retry.
    #[tokio::test]
    async fn a_granted_triple_permits_the_dispatch_and_re_binding_does_not_reuse_it() {
        use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};

        let (_dir, em, sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;

        // Ungranted, the mismatch refuses — and refuses with THIS gate's
        // refusal, not merely with some error. Without the first half the test
        // could pass against a gate that stopped running altogether; without the
        // second it could pass against a dispatch that failed for an unrelated
        // reason and never reached the affiliation question at all.
        assert!(
            refusal_text(&em, &id, "ucsfomopagent__tool", bound_to("stanford"))
                .await
                .contains(&expected_warning("ucsfomopagent", "stanford")),
            "an ungranted cross-institutional dispatch is refused with the DR-26 warning"
        );

        crate::privacy::grant::record_for_test(
            &sm,
            &id,
            "ucsfomopagent",
            Some(ModelAffiliation::institution(InstitutionId::new(
                "stanford",
            ))),
        )
        .await
        .expect("the user's grant is recorded against the session");

        let dispatched = em
            .dispatch_tool_call(
                &id,
                call("ucsfomopagent__tool"),
                bound_to("stanford"),
                CancellationToken::default(),
            )
            .await
            .expect("the user accepted this exact flow, so the next call proceeds");
        dispatched.result.await.expect("the mock client answers");

        // Re-bound to a THIRD institution's model: same session, same
        // extension, different third axis, so the grant does not reach it.
        assert!(
            refusal_text(&em, &id, "ucsfomopagent__tool", bound_to("mayo"))
                .await
                .contains(&expected_warning("ucsfomopagent", "mayo")),
            "re-binding to another institution's model invalidates the grant"
        );
    }

    /// …and a grant is scoped to ONE extension. A user who accepted a specific
    /// data flow accepted that flow, not a category of them.
    #[tokio::test]
    async fn a_grant_does_not_spread_to_another_extension_in_the_same_chat() {
        use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};

        let (_dir, em, sm, id) = manager_with_a_session().await;
        em.add_mock_extension("ucsfomopagent".to_string(), Arc::new(MockClient {}))
            .await;
        em.add_mock_extension("cdwagent".to_string(), Arc::new(MockClient {}))
            .await;

        crate::privacy::grant::record_for_test(
            &sm,
            &id,
            "ucsfomopagent",
            Some(ModelAffiliation::institution(InstitutionId::new(
                "stanford",
            ))),
        )
        .await
        .unwrap();

        assert!(
            refusal_text(&em, &id, "cdwagent__tool", bound_to("stanford"))
                .await
                .contains(&expected_warning("cdwagent", "stanford")),
            "the grant named the OMOP connector, not every UCSF connector"
        );
    }
}

/// What to answer when no extension claims `prefixed_name`.
///
/// ⚠ Not always "not found". A `platform__*` tool is ADVERTISED — it is in
/// `/agent/tools` — but dispatched by `Agent::dispatch`, so it reaches this
/// point with no client and "not found" is the one answer that is untrue. That
/// answer sends the reader hunting for a missing extension. Measured: 17 of 419
/// driven calls answered "Tool 'platform__ingest_source' not found" while the
/// same tool ingested successfully in a model turn.
///
/// `subagent` has the same shape and has always said so plainly; this keeps the
/// four platform tools consistent with it.
fn unroutable_tool_error(prefixed_name: &str, requested: &str) -> ErrorData {
    if crate::agents::platform_tools::is_platform_tool_name(prefixed_name) {
        return ErrorData::new(
            ErrorCode::INVALID_REQUEST,
            format!(
                "`{requested}` is dispatched by the agent loop, not by an extension, so it \
                 cannot be called through this entry point. Ask for it in a chat turn instead."
            ),
            None,
        );
    }
    ErrorData::new(
        ErrorCode::RESOURCE_NOT_FOUND,
        format!("Tool '{requested}' not found"),
        None,
    )
}

#[cfg(test)]
mod platform_dispatch_error_tests {
    /// A `platform__*` tool is ADVERTISED but dispatched by `Agent::dispatch`,
    /// so it reaches `ExtensionManager` and finds no client. "Tool not found"
    /// was the one answer that is untrue: the tool exists, it is in
    /// `/agent/tools`, and it works in a chat turn — measured, 17 of 419 driven
    /// calls answered "Tool 'platform__ingest_source' not found" while the same
    /// tool ingested successfully in a model turn.
    ///
    /// `subagent` has the same shape and has always said so plainly; this makes
    /// the four platform tools consistent with it.
    #[test]
    fn a_platform_tool_is_not_reported_as_missing() {
        for name in crate::agents::platform_tools::PLATFORM_TOOL_NAMES {
            assert!(
                crate::agents::platform_tools::is_platform_tool_name(name),
                "{name} must be recognised as agent-dispatched"
            );
        }
        assert!(
            !crate::agents::platform_tools::is_platform_tool_name("developer__shell"),
            "an ordinary extension tool must still take the not-found path"
        );

        // The message the dispatch site produces for these names, pinned at the
        // source: a reader who greps for the old wording should find nothing.
        let source = include_str!("extension_manager.rs");
        // The PLATFORM arm specifically. The function legitimately contains the
        // ordinary "not found" answer too — that is its other branch — so the
        // assertion has to be scoped to the branch under test, or it fails a
        // correct implementation.
        let arm = source
            .split("if crate::agents::platform_tools::is_platform_tool_name(prefixed_name) {")
            .nth(1)
            .expect("the unroutable-tool answer must special-case agent-dispatched tools")
            .split("\n    }")
            .next()
            .expect("the platform arm must be a block");
        assert!(
            arm.contains("dispatched by the agent loop"),
            "the refusal must say WHY, not that the tool is missing: {arm}"
        );
        assert!(
            !arm.contains("not found"),
            "a tool that exists must not be reported as missing: {arm}"
        );
    }
}
