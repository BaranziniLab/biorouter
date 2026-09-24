//! Saved native SSH connections and owner-scoped Crew capabilities.
pub mod authentication;
mod credentials;
mod institution;
#[cfg(test)]
#[path = "institution_tests.rs"]
mod institution_tests;
mod keepalive;
mod server_label;
pub use server_label::server_label;
#[cfg(test)]
#[path = "keepalive_tests.rs"]
mod keepalive_tests;
pub mod observation;
#[cfg(test)]
#[path = "registry_lock_tests.rs"]
mod registry_lock_tests;
#[cfg(test)]
#[path = "scope_binding_tests.rs"]
mod scope_binding_tests;
pub use credentials::CredentialStatus;
mod ssh_policy;
mod transport;
use crate::{
    privacy::{CallCapability, ProviderTier},
    providers::base::Provider,
};
use anyhow::{ensure, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex as StdMutex},
};
use tokio::sync::Mutex;
pub use transport::{SshFailure, SshFailureKind};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ClusterMode {
    Public,
    #[default]
    Private,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SaveConnection {
    #[serde(default)]
    pub preparation_id: Option<String>,
    pub name: String,
    pub ssh_target: String,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
    pub socket_path: String,
    pub owner_uid: u32,
    pub workspace_id: String,
    pub workspace_public_key: String,
    #[serde(default)]
    pub remote_root: Option<String>,
    #[serde(default)]
    pub remote_execution: bool,
    pub cluster_connection_id: Option<String>,
    #[serde(default)]
    pub mode: ClusterMode,
    #[serde(default)]
    pub institution_id: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Connection {
    pub id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub name: String,
    pub ssh_target: String,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
    pub socket_path: String,
    pub owner_uid: u32,
    pub workspace_id: String,
    pub workspace_public_key: String,
    #[serde(default)]
    pub remote_root: Option<String>,
    #[serde(default)]
    pub remote_execution: bool,
    pub cluster_connection_id: String,
    pub mode: ClusterMode,
    #[serde(default)]
    pub institution_id: Option<String>,
    pub policy_epoch: u64,
    pub status: String,
    pub last_error: Option<String>,
    pub device_id: String,
    pub public_key: String,
}
#[derive(Serialize)]
pub struct AuthenticationPlan {
    pub program: String,
    pub args: Vec<String>,
    pub connection_id: String,
    pub authentication_id: String,
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
struct Scope {
    connection_id: String,
    run_id: String,
    channel_id: String,
    source_channels: Vec<String>,
    epoch: u64,
    provider_binding: String,
    public_provider: bool,
    #[serde(default)]
    origin_restricted: bool,
    #[serde(default)]
    institution_ids: BTreeSet<String>,
    #[serde(default)]
    institution_policy: bool,
    expired: bool,
    /// When the broker stops honoring the run on its own (seconds since the Unix epoch), as
    /// `run.create` answered. Display only: the broker enforces it, so a list can show
    /// Expired without a network call. `None` for a scope granted before this was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
    /// The names the person saw when they granted the run (D14). Display only and possibly
    /// stale; captured from the admission snapshot because a worker may not read one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    labels: Option<AdmissionLabels>,
    /// Which chat the grant was made to: the `sessions.incarnation` of the row that held the
    /// session id at grant time (SCOPE-BIND). The id alone is not one chat — see
    /// [`CrewManager::standing`]. `None` only for a grant recorded before this was kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_incarnation: Option<i64>,
}

/// The display names of a run's identifiers, captured under the person's action when the
/// run is admitted (naming design D13 and D14). Never authority: every check still compares
/// the IDs, and the labels are not refreshed afterwards.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AdmissionLabels {
    /// The person the agent acts for: `Display name (@username)`, or `@username` when they
    /// never set a display name of their own (D13). `None` when the snapshot named no actor.
    #[serde(default)]
    pub you: Option<String>,
    /// The workspace's own name, or this device's name for the connection when the
    /// workspace has none.
    pub workspace: String,
    pub destination: ChannelLabel,
    /// Every channel the run may read, the destination included, in the run's order.
    #[serde(default)]
    pub sources: Vec<ChannelLabel>,
}

/// One channel's display name.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChannelLabel {
    pub channel_id: String,
    /// `#methods`.
    pub label: String,
    /// The channel's team, when the snapshot showed it; qualifies `#general` and its kin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
}

/// What `hello` told this daemon about the broker it is connected to. Held in memory only:
/// capabilities are not identity (D12), so they never enter the saved [`Connection`] and a
/// refresh can never change its binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BrokerHello {
    /// `2` when the v2 signature verified, so everything below is the workspace's own word;
    /// `1` when only v1 did, so `capabilities` are unauthenticated hints and the other fields
    /// are withheld.
    pub signature_version: u8,
    /// In the order the broker listed them.
    pub capabilities: Vec<String>,
    /// The workspace's name. Signed only under v2; `None` otherwise or when it has none.
    pub workspace_name: Option<String>,
    /// The workspace's privacy mode. Signed only under v2.
    pub mode: Option<ClusterMode>,
    /// The workspace's institution. Signed only under v2.
    pub institution_id: Option<String>,
    /// The workspace's policy epoch. Signed only under v2.
    pub policy_epoch: Option<u64>,
}

/// A `hello` whose signature verified, with the node identity to pin.
struct VerifiedHello {
    node_id: String,
    broker: BrokerHello,
}

/// The broker's workspace identity did not verify, or no longer matches what this connection
/// pinned. Its text is the specific check that failed, unchanged; the type lets a route answer
/// `crew_workspace_identity_mismatch` without matching on words.
#[derive(Debug)]
pub struct WorkspaceIdentityError(anyhow::Error);

impl WorkspaceIdentityError {
    fn wrap(error: anyhow::Error) -> anyhow::Error {
        anyhow::Error::new(Self(error))
    }
}

impl std::fmt::Display for WorkspaceIdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for WorkspaceIdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

/// What revoking a chat's or task's grant achieved. It is returned only once the grant is
/// stopped on this device **and** that stop is saved (RV-D1): the workspace's answer never
/// decides whether this daemon keeps honoring the grant.
#[derive(Debug)]
pub struct RevokeOutcome {
    /// The workspace confirmed `run.revoke`.
    pub remote_confirmed: bool,
    /// The broker's revoked run, when it confirmed.
    pub run: Option<Value>,
    /// Why the workspace did not confirm: the transport's or the broker's own error, so an
    /// [`SshFailure`] can still be recognized.
    pub remote_error: Option<anyhow::Error>,
}

impl RevokeOutcome {
    /// The confirmed run, or a [`RevocationUnconfirmed`] error carrying the workspace's
    /// refusal, for callers that treat anything short of confirmation as a failure.
    pub fn into_confirmed(self) -> Result<Value> {
        match (self.remote_confirmed, self.run, self.remote_error) {
            (true, Some(run), _) => Ok(run),
            (_, _, Some(error)) => Err(anyhow::Error::new(RevocationUnconfirmed(error))),
            _ => Err(anyhow::Error::new(RevocationUnconfirmed(anyhow::anyhow!(
                "The workspace did not confirm the revocation"
            )))),
        }
    }
}

/// The grant stopped on this device, but the workspace has not confirmed `run.revoke`. Its
/// text is the workspace's (or the transport's) error, unchanged.
#[derive(Debug)]
pub struct RevocationUnconfirmed(anyhow::Error);

impl RevocationUnconfirmed {
    /// Why the workspace did not confirm.
    pub fn remote_error(&self) -> &anyhow::Error {
        &self.0
    }
}

impl std::fmt::Display for RevocationUnconfirmed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for RevocationUnconfirmed {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

/// Shown to the person (and the model) when a revoked chat tries to use Crew.
const GRANT_REVOKED: &str =
    "This chat's Crew access was removed. Start a new chat, or grant access again from Crew.";
/// Shown when the connection's privacy or policy moved after the grant was made.
const GRANT_POLICY_CHANGED: &str =
    "Crew settings changed since access was granted. Grant access again from Crew.";
/// Shown when a chat that was never granted asks for a Crew run.
const NO_GRANT: &str = "This chat doesn't have Crew access. Grant it access from Crew first.";
/// A task whose grant was replaced by a newer one.
const REPLACED_RUN: &str = "This task was replaced by a newer explicitly granted run; cancel it from its current conversation";
/// A grant whose chat is no longer on this device: it keeps restricting, never acts.
const GRANT_GONE: &str =
    "This chat's Crew access is no longer available. Start a new chat, or grant access again from Crew.";
/// The chat's identity could not be read, so its grant can be neither trusted nor dropped.
const GRANT_UNCONFIRMED: &str =
    "Couldn't confirm this chat's Crew access on this device. Try again in a moment.";
/// A grant asked for a chat that is not saved on this device (`--no-session`, or gone).
const UNSAVED_CHAT: &str = "Crew can only grant access to a chat saved on this device. Start a saved chat, then grant it access from Crew.";
/// The grant changed between two reads of one check.
const ACCESS_CHANGED: &str = "Crew access or settings changed while this was in progress. Check whether it already took effect before you grant access again.";
/// A bridge that failed carrying a request: what was sent may or may not have reached the
/// workspace.
const BRIDGE_FAILED: &str = "SSH bridge failed. Reconnect; inspect any submitted operation before retrying because its outcome may be unknown.";

/// Whether a grant stored under a session id is the grant of the chat that holds that id now
/// (SCOPE-BIND). See [`CrewManager::standing`].
enum Standing {
    /// No grant is this chat's: none was made, or the one stored under its id was made to an
    /// earlier chat that has since been replaced under the same id (and has been pruned).
    None,
    /// The chat's own grant: bound to its incarnation, or recorded before grants were bound.
    Own(Scope),
    /// A grant sits under the id, but this chat cannot be confirmed as the one it was made
    /// to: that chat is gone from this device, or its identity could not be read. It keeps
    /// every restriction and authorizes nothing; the text says why.
    Unconfirmed(Scope, &'static str),
}

/// Which door a signed request came through. Only the daemon's own join sends `auth.join`,
/// so no route, tool or pass-through can.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SignedDoor {
    /// `human_request` and the daemon's own grant path.
    Generic,
    /// The S3a join (`authentication.rs`), which sends `auth.join` and nothing else.
    Join,
}
#[derive(Clone, Default, Deserialize, Serialize)]
struct Registry {
    connections: Vec<Connection>,
    scopes: HashMap<String, Scope>,
    #[serde(default)]
    pending_device: Option<PreparedDevice>,
    #[serde(default)]
    completed_preparations: HashMap<String, String>,
}
/// How long a registry update waits for another Biorouter process to finish its own.
const REGISTRY_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// A registry update that waited [`REGISTRY_LOCK_WAIT`] for another process and gave up.
const REGISTRY_BUSY: &str =
    "Crew settings are being saved by another Biorouter process. Try again in a moment.";
/// What a registry update does with this process's copy when the saved registry cannot be
/// locked, read or written.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Unsaved {
    /// Leave it as it was: the change happened nowhere.
    Discard,
    /// Apply the change to it anyway: the change holds in this process, just not durably.
    KeepHere,
}
/// The saved registry as an update found it, holding both locks.
struct SavedRegistry {
    /// The copy to edit.
    registry: Registry,
    /// The file's contents (an empty registry when there is none), to tell whether the edit
    /// changed anything that needs writing.
    saved: Value,
    /// The digest of the file's bytes; `None` when there is no file.
    digest: Option<[u8; 32]>,
}
fn registry_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
/// Carry into a registry just read back from disk what this process holds that the file does
/// not speak for (D8), so that re-reading never drops it:
///
/// - each connection's status and last error, which describe this process's own transports
///   (the file's copy is whatever process wrote last, and is reset on load for that reason);
///   a connection this process has not seen reads as a fresh load reads it, disconnected;
/// - a grant's binding to its chat, when this process bound a grant recorded before grants
///   were bound ([`CrewManager::adopt_binding`]) and the file still has it unbound;
/// - a stop: a grant this process expired stays expired even if its save failed, because
///   nothing may bring a revoked run back to life.
///
/// Only for the same grant, matched by run: a grant the file no longer holds, or holds for a
/// newer run, is the file's to decide.
fn carry_process_state(here: &Registry, theirs: &mut Registry) {
    for connection in &mut theirs.connections {
        match here
            .connections
            .iter()
            .find(|mine| mine.id == connection.id)
        {
            Some(mine) => {
                connection.status = mine.status.clone();
                connection.last_error = mine.last_error.clone();
            }
            None => {
                connection.status = "disconnected".into();
                connection.last_error = None;
            }
        }
    }
    for (session, scope) in &mut theirs.scopes {
        let Some(mine) = here
            .scopes
            .get(session)
            .filter(|mine| mine.run_id == scope.run_id)
        else {
            continue;
        };
        scope.expired |= mine.expired;
        if scope.session_incarnation.is_none() {
            scope.session_incarnation = mine.session_incarnation;
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PreparedDevice {
    pub preparation_id: String,
    pub public_key: String,
    pub device_id: String,
}
#[derive(Serialize)]
pub struct RunAdmission {
    pub run_id: String,
    pub context: String,
    pub institution_ids: BTreeSet<String>,
    /// The names captured at admission; `context` carries the same labels for the model.
    pub labels: AdmissionLabels,
    /// When the broker stops honoring the run on its own, if it said.
    pub expires_at: Option<u64>,
}
#[derive(Default)]
pub struct RunPolicy {
    pub origin_restricted: bool,
    pub expected_mode: Option<ClusterMode>,
    pub expected_policy_epoch: Option<u64>,
    pub expected_workspace_policy_epoch: Option<u64>,
    pub origin_institution_ids: BTreeSet<String>,
}
pub struct RunMetadata {
    pub run_id: String,
    pub connection_id: String,
    pub channel_id: String,
}
pub struct CrewManager {
    root: PathBuf,
    credential_vault: Arc<credentials::CredentialVault>,
    registry: Mutex<Registry>,
    transports: Mutex<HashMap<String, Arc<Mutex<transport::Transport>>>>,
    lifecycle: StdMutex<HashMap<String, std::sync::Weak<Mutex<()>>>>,
    /// What each connected broker's `hello` said, keyed by connection ID. Memory only (D12).
    brokers: StdMutex<HashMap<String, BrokerHello>>,
    /// The digest of `connections.json` as `registry` was last read from or written to it
    /// (`None`: there was no file), so an update can tell whether another process has written
    /// since (D8). Read and set only under `registry`'s lock.
    saved_digest: StdMutex<Option<[u8; 32]>>,
    /// The store a test resolves chats against in place of the shared one (SCOPE-BIND).
    #[cfg(test)]
    session_store: StdMutex<Option<Arc<crate::session::SessionManager>>>,
    /// This manager, for the keepalive task a connect starts (D-KEEPALIVE). Set by
    /// [`CrewManager::shared`]; a manager built by [`CrewManager::new`] alone starts none.
    this: std::sync::OnceLock<std::sync::Weak<CrewManager>>,
    /// How often an idle bridge is kept alive, and how a dropped one is dialled again.
    keepalive: StdMutex<keepalive::KeepaliveTiming>,
    /// Connections whose bridge dropped while idle and whose re-dial may be tried again,
    /// each with the token of the attempt that armed it. A person's Connect or Disconnect
    /// (any connect or disconnect) clears it, so a retry never undoes what someone chose.
    idle_redial: StdMutex<HashMap<String, u64>>,
}
pub(super) fn connection_binding(connection: &Connection) -> Result<Value> {
    let mut value = serde_json::to_value(connection)?;
    if let Some(fields) = value.as_object_mut() {
        fields.remove("status");
        fields.remove("last_error");
    }
    Ok(value)
}
static MANAGERS: LazyLock<StdMutex<HashMap<PathBuf, Arc<CrewManager>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));
pub fn manager() -> Result<Arc<CrewManager>> {
    let path = crate::config::paths::Paths::config_dir().join("crew");
    let mut managers = MANAGERS
        .lock()
        .map_err(|_| anyhow::anyhow!("Crew registry lock poisoned"))?;
    if let Some(manager) = managers.get(&path) {
        return Ok(manager.clone());
    }
    let manager = CrewManager::shared(path.clone())?;
    managers.insert(path, manager.clone());
    Ok(manager)
}

#[cfg(test)]
pub(crate) async fn install_test_scope(
    session_id: &str,
    provider: Option<&dyn Provider>,
) -> Arc<CrewManager> {
    let manager = manager().expect("the test Crew manager should initialize");
    let connection_id = format!("context-fixture-connection-{session_id}");
    let mut registry = manager.registry.lock().await;
    registry.connections.push(Connection {
        id: connection_id.clone(),
        node_id: None,
        name: "context fixture".into(),
        ssh_target: "fixture@example.test".into(),
        port: Some(22),
        identity_file: None,
        proxy_jump: None,
        socket_path: "/tmp/context-fixture.sock".into(),
        owner_uid: 10001,
        workspace_id: format!("workspace-{session_id}"),
        workspace_public_key: "11".repeat(32),
        remote_root: None,
        remote_execution: false,
        cluster_connection_id: format!("cluster-{session_id}"),
        mode: ClusterMode::Public,
        institution_id: None,
        policy_epoch: 1,
        status: "connected".into(),
        last_error: None,
        device_id: "22".repeat(32),
        public_key: "33".repeat(32),
    });
    registry.scopes.insert(
        session_id.into(),
        Scope {
            connection_id,
            run_id: format!("run-{session_id}"),
            channel_id: format!("channel-{session_id}"),
            source_channels: vec![],
            epoch: 1,
            provider_binding: provider.map_or_else(|| "test-context".into(), provider_binding),
            public_provider: true,
            origin_restricted: false,
            institution_ids: BTreeSet::new(),
            institution_policy: false,
            expired: false,
            expires_at: None,
            labels: None,
            session_incarnation: None,
        },
    );
    drop(registry);
    manager
}

#[cfg(test)]
pub(crate) async fn remove_test_scope(manager: &CrewManager, session_id: &str) {
    let mut registry = manager.registry.lock().await;
    registry.scopes.remove(session_id);
    registry
        .connections
        .retain(|connection| connection.id != format!("context-fixture-connection-{session_id}"));
}

fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => serde_json::to_value(
            map.iter()
                .map(|(k, v)| (k.clone(), canonical(v)))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .expect("JSON values serialize"),
        Value::Array(a) => Value::Array(a.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|v| format!("{v:02x}")).collect()
}
fn unhex(value: &str) -> Result<Vec<u8>> {
    ensure!(
        value.len().is_multiple_of(2) && value.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid encoded key"
    );
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Ok(u8::from_str_radix(std::str::from_utf8(pair)?, 16)?))
        .collect()
}
fn provider_binding(provider: &dyn Provider) -> String {
    let resolved =
        serde_json::to_vec(&provider.restore_binding()).expect("provider binding serializes");
    format!(
        "{}:{:?}:{:?}",
        hex(&Sha256::digest(resolved)),
        provider.tier(),
        provider.affiliation()
    )
}
/// The fields `hello` v2 signs besides the pinned identity, read strictly: the signature covers
/// their exact values, so a malformed field fails verification rather than being skipped.
struct HelloV2Fields {
    mode: biorouter_crew::Mode,
    institution_id: Option<String>,
    policy_epoch: u64,
    name: Option<String>,
    capabilities: Vec<String>,
}
impl HelloV2Fields {
    fn read(hello: &Value) -> Result<Self> {
        let invalid = || anyhow::anyhow!("Workspace identity signature is invalid");
        let optional_text = |field: &str| match hello.get(field) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(invalid()),
        };
        Ok(Self {
            mode: serde_json::from_value(hello.get("mode").cloned().ok_or_else(invalid)?)
                .map_err(|_| invalid())?,
            institution_id: optional_text("institution_id")?,
            policy_epoch: hello["policy_epoch"].as_u64().ok_or_else(invalid)?,
            name: optional_text("name")?,
            capabilities: hello["capabilities"]
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .map(|entry| entry.as_str().map(str::to_owned).ok_or_else(invalid))
                .collect::<Result<_>>()?,
        })
    }
    /// Verify the v2 signature over the pinned identity and these fields, and only then hand
    /// them on as the workspace's own word.
    fn verify(
        self,
        c: &Connection,
        challenge_nonce: &str,
        node_id: &str,
        verifying: &VerifyingKey,
        signature: &Signature,
    ) -> Result<BrokerHello> {
        let capabilities: Vec<&str> = self.capabilities.iter().map(String::as_str).collect();
        let signed = biorouter_crew::HelloV2 {
            workspace_id: &c.workspace_id,
            host_uid: c.owner_uid,
            challenge_nonce,
            workspace_public_key: &c.workspace_public_key,
            node_id,
            mode: &self.mode,
            institution_id: self.institution_id.as_deref(),
            policy_epoch: self.policy_epoch,
            name: self.name.as_deref(),
            capabilities: &capabilities,
        }
        .signing_payload();
        verifying.verify(&signed, signature)?;
        Ok(BrokerHello {
            signature_version: 2,
            capabilities: self.capabilities,
            workspace_name: self
                .name
                .filter(|name| biorouter_crew::workspace_name_valid(name)),
            mode: Some(match self.mode {
                biorouter_crew::Mode::Private => ClusterMode::Private,
                biorouter_crew::Mode::Public => ClusterMode::Public,
            }),
            institution_id: self.institution_id,
            policy_epoch: Some(self.policy_epoch),
        })
    }
}
impl BrokerHello {
    /// A `hello` only v1 signed: the capabilities are kept as unauthenticated hints (malformed
    /// entries dropped), and nothing else it said is passed on.
    fn unsigned(hello: &Value) -> Self {
        Self {
            signature_version: 1,
            capabilities: hello["capabilities"]
                .as_array()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            workspace_name: None,
            mode: None,
            institution_id: None,
            policy_epoch: None,
        }
    }
}
/// One of `hello`'s workspace signatures, if present. A present but malformed one is an error,
/// never treated as absent.
fn hello_signature(hello: &Value, field: &str) -> Result<Option<Signature>> {
    hello
        .get(field)
        .filter(|value| !value.is_null())
        .map(|value| {
            let encoded = value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Workspace identity signature is invalid"))?;
            Ok(Signature::from_slice(&unhex(encoded)?)?)
        })
        .transpose()
}
/// Text from a broker snapshot made safe to show and to hand a model: invisible and control
/// characters removed and whitespace collapsed. Empty when nothing visible is left.
fn plain_label(text: &str) -> String {
    biorouter_crew::clean(
        &biorouter_crew::strip_ignorable(text)
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>(),
    )
}
/// A person as the design's display rule shows them to a model or in a list:
/// `Display name (@username)`, or `@username` when they never set a display name of their own
/// (D13: a nickname equal to the username, or one that sanitizes to it, is not a choice).
fn person_label(principal: &Value, other_usernames: &[String]) -> Option<String> {
    let username = plain_label(principal["username"].as_str()?);
    if username.is_empty() {
        return None;
    }
    let nickname = principal["display_name"]
        .as_str()
        .or_else(|| principal["nickname"].as_str())
        .unwrap_or_default();
    let display = biorouter_crew::sanitize_display_name(
        nickname,
        &username,
        other_usernames
            .iter()
            .map(String::as_str)
            .filter(|other| *other != username),
    );
    Some(if display == username {
        format!("@{username}")
    } else {
        format!("{display} (@{username})")
    })
}
/// The display labels of a run's identifiers, from the snapshot the person's own admission
/// fetched. `sources` must already be the run's final list (destination included).
fn admission_labels(
    snapshot: &Value,
    connection: &Connection,
    channel: &str,
    sources: &[String],
) -> AdmissionLabels {
    let usernames: Vec<String> = snapshot["principals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|principal| principal["username"].as_str())
        .map(plain_label)
        .collect();
    let workspace = snapshot["workspace"]["name"]
        .as_str()
        .filter(|name| biorouter_crew::workspace_name_valid(name))
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let local = plain_label(&connection.name);
            if local.is_empty() {
                "this workspace".into()
            } else {
                local
            }
        });
    let channel_label = |id: &str| {
        let found = snapshot["channels"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|entry| entry["id"].as_str() == Some(id));
        let name = found
            .and_then(|entry| entry["name"].as_str())
            .map(biorouter_crew::names::sanitize_channel_name)
            .unwrap_or_else(|| biorouter_crew::names::UNTITLED_CHANNEL.to_owned());
        let team = found
            .and_then(|entry| entry["team_id"].as_str())
            .and_then(|team_id| {
                snapshot["teams"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|team| team["id"].as_str() == Some(team_id))
            })
            .and_then(|team| team["name"].as_str())
            .map(biorouter_crew::sanitize_team_name);
        ChannelLabel {
            channel_id: id.to_owned(),
            label: format!("#{name}"),
            team,
        }
    };
    AdmissionLabels {
        you: person_label(&snapshot["actor"], &usernames),
        workspace,
        destination: channel_label(channel),
        sources: sources.iter().map(|id| channel_label(id)).collect(),
    }
}
/// A fresh challenge nonce for a `hello` sent on a live connection. Tests pin it, because a
/// scripted broker can only answer with a signature made in advance.
fn hello_nonce() -> String {
    #[cfg(test)]
    if let Some(nonce) = TEST_HELLO_NONCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
    {
        return nonce;
    }
    uuid::Uuid::new_v4().to_string()
}
#[cfg(test)]
pub(super) static TEST_HELLO_NONCE: StdMutex<Option<String>> = StdMutex::new(None);
fn file_credentials_enabled() -> bool {
    std::env::var("BIOROUTER_DISABLE_KEYRING").as_deref() == Ok("true")
        && std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT")
            .is_some_and(|p| PathBuf::from(p).is_absolute())
}
pub(super) fn safe_atom(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:@%-".contains(&b))
        && !value.starts_with('-')
}
impl CrewManager {
    fn credential_path(&self, id: &str) -> PathBuf {
        self.root
            .join("credentials")
            .join(hex(&Sha256::digest(id.as_bytes())))
    }
    fn credential_entry(&self, id: &str) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new(
            "org.biorouter.crew",
            &format!(
                "{}:{id}",
                hex(&Sha256::digest(self.root.to_string_lossy().as_bytes()))
            ),
        )?)
    }
    fn write_credential(&self, id: &str, value: &str) -> Result<()> {
        self.credential_vault
            .write(id, value, || self.write_legacy_credential(id, value))
    }
    fn write_legacy_credential(&self, id: &str, value: &str) -> Result<()> {
        if file_credentials_enabled() {
            let path = self.credential_path(id);
            let parent = path.parent().expect("credential parent");
            std::fs::create_dir_all(parent)?;
            ensure!(
                !std::fs::symlink_metadata(parent)?.file_type().is_symlink(),
                "Credential directory must not be a symlink"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            }
            let mut file = tempfile::NamedTempFile::new_in(parent)?;
            use std::io::Write;
            file.write_all(value.as_bytes())?;
            file.as_file().sync_all()?;
            file.persist(path)?;
            Ok(())
        } else {
            self.credential_entry(id)?.set_password(value)?;
            Ok(())
        }
    }
    /// Remove a credential; one that is already absent is not an error.
    fn delete_credential(&self, id: &str) -> Result<()> {
        self.credential_vault
            .delete(id, || self.delete_legacy_credential(id))
    }
    fn delete_legacy_credential(&self, id: &str) -> Result<()> {
        if file_credentials_enabled() {
            match std::fs::remove_file(self.credential_path(id)) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            }
        } else {
            match self.credential_entry(id)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(error.into()),
            }
        }
    }
    fn read_credential(&self, id: &str) -> Result<zeroize::Zeroizing<String>> {
        self.credential_vault.read(id, || {
            self.read_legacy_credential(id).map(zeroize::Zeroizing::new)
        })
    }
    fn read_legacy_credential(&self, id: &str) -> Result<String> {
        if file_credentials_enabled() {
            Ok(std::fs::read_to_string(self.credential_path(id))?)
        } else {
            Ok(self.credential_entry(id)?.get_password()?)
        }
    }
    pub fn new(root: PathBuf) -> Result<Self> {
        let path = root.join("connections.json");
        let (mut registry, saved_digest): (Registry, _) = match std::fs::read(&path) {
            Ok(bytes) => (
                serde_json::from_slice(&bytes)?,
                Some(registry_digest(&bytes)),
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Registry::default(), None),
            Err(e) => return Err(e.into()),
        };
        for c in &mut registry.connections {
            c.status = "disconnected".into();
            c.last_error = None;
        }
        Ok(Self {
            credential_vault: Arc::new(credentials::CredentialVault::new(root.clone())),
            root,
            registry: Mutex::new(registry),
            transports: Mutex::new(HashMap::new()),
            lifecycle: StdMutex::new(HashMap::new()),
            brokers: StdMutex::new(HashMap::new()),
            saved_digest: StdMutex::new(saved_digest),
            #[cfg(test)]
            session_store: StdMutex::new(None),
            this: std::sync::OnceLock::new(),
            keepalive: StdMutex::new(keepalive::KeepaliveTiming::default()),
            idle_redial: StdMutex::new(HashMap::new()),
        })
    }
    /// [`CrewManager::new`], shared, and able to keep its connections' bridges alive.
    pub fn shared(root: PathBuf) -> Result<Arc<Self>> {
        let manager = Arc::new(Self::new(root)?);
        let _ = manager.this.set(Arc::downgrade(&manager));
        Ok(manager)
    }
    /// The capabilities the connected broker announced in its last verified `hello`, or
    /// `None` when this process has not connected to it (or has since disconnected). They
    /// are signed only when [`Self::broker_hello`] reports `signature_version` 2; either way
    /// they only decide which requests to *offer*, and the broker still refuses what it does
    /// not support.
    pub fn capabilities(&self, id: &str) -> Option<Vec<String>> {
        self.broker_hello(id).map(|hello| hello.capabilities)
    }
    /// Everything the connected broker's last verified `hello` said, for display.
    pub fn broker_hello(&self, id: &str) -> Option<BrokerHello> {
        self.brokers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()
    }
    fn forget_broker(&self, id: &str) {
        self.brokers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
    }
    /// Remember what a verified `hello` announced for `id`. Connect pins the node first
    /// ([`Self::adopt_verified_hello`]); a refresh ([`Self::refresh_broker_hello`]) pins
    /// nothing and only replaces this.
    fn remember_broker(&self, id: &str, broker: BrokerHello) {
        self.brokers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.into(), broker);
    }
    pub async fn credential_status(&self) -> Result<CredentialStatus> {
        let vault = self.credential_vault.clone();
        tokio::task::spawn_blocking(move || vault.status()).await?
    }
    pub async fn init_vault(&self, passphrase: zeroize::Zeroizing<String>) -> Result<()> {
        ensure!(!file_credentials_enabled(), "Encrypted vault initialization requires a production credential profile, not the development plaintext backend");
        let registry = self.registry.lock().await;
        ensure!(registry.connections.is_empty() && registry.scopes.is_empty() && registry.pending_device.is_none() && registry.completed_preparations.is_empty(), "Initialize an encrypted vault in a fresh Crew profile before creating identities; existing keyring credentials are never silently replaced");
        let vault = self.credential_vault.clone();
        let result = tokio::task::spawn_blocking(move || vault.init(passphrase)).await?;
        drop(registry);
        result
    }
    pub async fn unlock_vault(&self, passphrase: zeroize::Zeroizing<String>) -> Result<()> {
        let vault = self.credential_vault.clone();
        tokio::task::spawn_blocking(move || vault.unlock(passphrase)).await?
    }
    pub async fn lock_vault(&self) -> Result<()> {
        let vault = self.credential_vault.clone();
        tokio::task::spawn_blocking(move || vault.lock()).await?
    }
    /// The grants on `connection_id`, one per chat. A grant stored under an id now held by
    /// another chat is not listed, and is pruned (SCOPE-BIND); one whose chat is gone is,
    /// so the person can still revoke it at the workspace.
    pub async fn session_grants(&self, connection_id: &str) -> Result<Value> {
        self.connection(connection_id).await?;
        let sessions: Vec<String> = self
            .registry
            .lock()
            .await
            .scopes
            .iter()
            .filter(|(_, scope)| scope.connection_id == connection_id)
            .map(|(session, _)| session.clone())
            .collect();
        let mut grants = Vec::with_capacity(sessions.len());
        for session in sessions {
            let (Standing::Own(scope) | Standing::Unconfirmed(scope, _)) =
                self.standing(&session).await
            else {
                continue;
            };
            if scope.connection_id == connection_id {
                grants.push(json!({"session_id":session,"run_id":scope.run_id,"connection_id":scope.connection_id,"channel_id":scope.channel_id,"source_channels":scope.source_channels,"policy_epoch":scope.epoch,"expired":scope.expired,"expires_at":scope.expires_at,"labels":scope.labels}));
            }
        }
        Ok(json!({ "grants": grants }))
    }
    /// Write `registry` over the saved one as it stands, for a test that plays another
    /// process's save (or seeds one). Production writes go through
    /// [`Self::update_registry`], which never writes a stale copy back.
    #[cfg(test)]
    fn persist(&self, registry: &Registry) -> Result<()> {
        self.write_registry(registry).map(drop)
    }
    /// Create the registry's directory — private, never a symlink — and answer the
    /// directories whose entries a save must sync: the registry's own, then each ancestor this
    /// call created, then the first that already existed.
    fn prepare_registry_directory(&self) -> Result<Vec<PathBuf>> {
        let mut directories = vec![self.root.clone()];
        let mut directory = self.root.as_path();
        while !directory.exists() {
            directory = directory
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Crew registry parent unavailable"))?;
            directories.push(directory.to_path_buf());
        }
        std::fs::create_dir_all(&self.root)?;
        ensure!(
            !std::fs::symlink_metadata(&self.root)?
                .file_type()
                .is_symlink(),
            "Crew registry must not be a symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(directories)
    }
    /// Replace `connections.json` with `registry`, atomically and durably, and answer the
    /// digest of what was written. Only [`Self::try_update_registry`] calls this outside tests,
    /// under both locks, with a copy it has just read back.
    fn write_registry(&self, registry: &Registry) -> Result<[u8; 32]> {
        #[cfg_attr(not(unix), allow(unused_variables))]
        let directories_to_sync = self.prepare_registry_directory()?;
        let bytes = serde_json::to_vec(registry)?;
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        use std::io::Write;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(self.root.join("connections.json"))?;
        #[cfg(unix)]
        for directory in directories_to_sync {
            std::fs::File::open(directory)?.sync_all()?;
        }
        Ok(registry_digest(&bytes))
    }
    fn set_saved_digest(&self, digest: Option<[u8; 32]>) {
        *self
            .saved_digest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = digest;
    }
    fn saved_digest(&self) -> Option<[u8; 32]> {
        *self
            .saved_digest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// Take the advisory lock that orders registry writes across every process sharing this
    /// profile (D8); it is released when the returned file is dropped. The wait is bounded: a
    /// lock another process holds for longer than [`REGISTRY_LOCK_WAIT`] is refused with
    /// [`REGISTRY_BUSY`], never waited on forever, and the runtime is never blocked on it.
    async fn lock_saved_registry(&self) -> Result<std::fs::File> {
        #[cfg_attr(not(unix), allow(unused_variables))]
        let created = self.prepare_registry_directory()?;
        // A directory this call created must survive a crash as the save's would.
        #[cfg(unix)]
        for directory in created.iter().skip(1) {
            std::fs::File::open(directory)?.sync_all()?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let file = options.open(self.root.join("connections.lock"))?;
        let deadline = tokio::time::Instant::now() + REGISTRY_LOCK_WAIT;
        let mut pause = std::time::Duration::from_millis(2);
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(file),
                Err(std::fs::TryLockError::WouldBlock) => {
                    ensure!(tokio::time::Instant::now() < deadline, REGISTRY_BUSY);
                    tokio::time::sleep(pause).await;
                    pause = (pause * 2).min(std::time::Duration::from_millis(50));
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }
    /// The saved registry as it is now, to edit: the file's copy with this process's own state
    /// carried in ([`carry_process_state`]), or — when the file is exactly what `here` was last
    /// read from or written as — `here` itself, which keeps what this process holds and has
    /// not saved (a stop whose save failed, a grant a test put in memory). Call it holding
    /// both locks.
    fn read_saved_registry(&self, here: &Registry) -> Result<SavedRegistry> {
        let bytes = match std::fs::read(self.root.join("connections.json")) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let digest = bytes.as_deref().map(registry_digest);
        let saved: Value = match &bytes {
            Some(bytes) => serde_json::from_slice(bytes)?,
            None => serde_json::to_value(Registry::default())?,
        };
        let registry = if digest == self.saved_digest() {
            here.clone()
        } else {
            let mut theirs: Registry = serde_json::from_value(saved.clone())?;
            carry_process_state(here, &mut theirs);
            theirs
        };
        Ok(SavedRegistry {
            registry,
            saved,
            digest,
        })
    }
    /// Change the saved registry: the only way this process writes it (D8).
    ///
    /// ⚠ **Grant and revocation state; needs human review.** Every process that opens this
    /// profile — the desktop's daemon, a CLI run, another daemon — holds its own copy of the
    /// registry, loaded once. Writing that whole copy back, as every save once did, put back
    /// whatever another process had changed since it loaded: a connection it saved vanished,
    /// a grant it revoked came back live. So an update:
    ///
    /// 1. takes this process's registry lock, which orders it with every other write here;
    /// 2. takes the advisory lock on `connections.lock`, which orders it with every other
    ///    process's ([`Self::lock_saved_registry`]);
    /// 3. reads `connections.json` again ([`Self::read_saved_registry`]);
    /// 4. applies `edit` to that copy, and writes the result atomically if it differs from
    ///    the file;
    /// 5. makes the result this process's copy, then releases the file lock.
    ///
    /// When `edit` refuses, nothing changes anywhere and its error is returned. When the file
    /// cannot be locked, read or written, this process's copy is left as it was too; an edit
    /// that must hold here even then uses [`Self::update_registry_keeping`].
    async fn update_registry<R>(&self, edit: impl FnOnce(&mut Registry) -> Result<R>) -> Result<R> {
        self.try_update_registry(Unsaved::Discard, edit).await?
    }
    /// [`Self::update_registry`] for an edit that takes effect in this process even when the
    /// saved registry cannot be locked, read or written: a stop above all, which must hold
    /// here whether or not it could be saved, and the edits that have always behaved so. The
    /// outer error is the file's, and the edit already holds here when it is returned; the
    /// inner result is `edit`'s own, including its refusal.
    async fn update_registry_keeping<R>(
        &self,
        edit: impl FnOnce(&mut Registry) -> Result<R>,
    ) -> Result<Result<R>> {
        self.try_update_registry(Unsaved::KeepHere, edit).await
    }
    async fn try_update_registry<R>(
        &self,
        unsaved: Unsaved,
        edit: impl FnOnce(&mut Registry) -> Result<R>,
    ) -> Result<Result<R>> {
        let mut registry = self.registry.lock().await;
        let opened = match self.lock_saved_registry().await {
            Ok(lock) => self
                .read_saved_registry(&registry)
                .map(|saved| (lock, saved)),
            Err(error) => Err(error),
        };
        let (lock, mut saved) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                // The saved registry could not be seen, so nothing is written. An edit that
                // must hold here applies to this process's copy alone; any other is not run.
                if unsaved == Unsaved::KeepHere {
                    let mut here = registry.clone();
                    if let Err(refused) = edit(&mut here) {
                        return Ok(Err(refused));
                    }
                    *registry = here;
                }
                return Err(error);
            }
        };
        let out = match edit(&mut saved.registry) {
            Ok(out) => out,
            Err(refused) => return Ok(Err(refused)),
        };
        let changed = match serde_json::to_value(&saved.registry) {
            Ok(now) => now != saved.saved,
            Err(_) => true,
        };
        if changed {
            match self.write_registry(&saved.registry) {
                Ok(digest) => saved.digest = Some(digest),
                Err(error) => {
                    if unsaved == Unsaved::KeepHere {
                        // This copy now derives from the file as it was read, plus the edit.
                        *registry = saved.registry;
                        self.set_saved_digest(saved.digest);
                    }
                    return Err(error);
                }
            }
        }
        *registry = saved.registry;
        self.set_saved_digest(saved.digest);
        drop(lock);
        Ok(Ok(out))
    }
    fn control_path(&self, id: &str) -> Result<PathBuf> {
        #[cfg(unix)]
        {
            use std::os::unix::{
                ffi::OsStrExt,
                fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt},
            };
            // Darwin's per-user TMPDIR can consume most of sockaddr_un before
            // the socket name is appended. A private directory under the short,
            // verified system temporary directory keeps OpenSSH portable.
            let temporary = std::fs::canonicalize("/tmp")?;
            let parent = std::fs::symlink_metadata(&temporary)?;
            let uid = unsafe { libc::geteuid() };
            ensure!(
                parent.is_dir()
                    && ((parent.uid() == 0 && parent.mode() & 0o1000 != 0)
                        || (parent.uid() == uid && parent.mode() & 0o077 == 0)),
                "SSH temporary directory must be root-owned and sticky or private to this user"
            );
            let mut namespace = Sha256::new();
            namespace.update(uid.to_be_bytes());
            namespace.update([0]);
            namespace.update(self.root.as_os_str().as_bytes());
            let profile = hex(&namespace.finalize()[..16]);
            let root = temporary.join(format!("brc-{profile}"));
            match std::fs::DirBuilder::new().mode(0o700).create(&root) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            let directory = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&root)?;
            let metadata = directory.metadata()?;
            ensure!(
                metadata.uid() == uid && metadata.mode() & 0o077 == 0,
                "SSH runtime directory must be private and owned by this user"
            );
            let path = root.join(hex(&Sha256::digest(id.as_bytes())[..16]));
            // OpenSSH first binds a temporary socket with a dot and sixteen
            // random characters. Reserve those seventeen bytes plus the NUL
            // terminator within Darwin's 104-byte sockaddr_un.sun_path.
            ensure!(
                path.as_os_str().as_bytes().len() <= 86,
                "SSH control socket path exceeds the portable length limit including its temporary suffix"
            );
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => ensure!(
                    metadata.file_type().is_socket() && metadata.uid() == uid,
                    "SSH control path must be an owned socket, not a symlink or another file"
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            Ok(path)
        }
        #[cfg(not(unix))]
        {
            let root = std::env::temp_dir().join(format!(
                "brcrew-{}",
                hex(&Sha256::digest(self.root.to_string_lossy().as_bytes())[..6])
            ));
            std::fs::create_dir_all(&root)?;
            ensure!(
                !std::fs::symlink_metadata(&root)?.file_type().is_symlink(),
                "SSH runtime directory must not be a symlink"
            );
            Ok(root.join(id))
        }
    }
    pub async fn list(&self) -> Vec<Connection> {
        self.registry.lock().await.connections.clone()
    }
    pub async fn connection(&self, id: &str) -> Result<Connection> {
        self.registry
            .lock()
            .await
            .connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Unknown Crew connection"))
    }
    pub async fn prepare_device(&self) -> Result<PreparedDevice> {
        self.update_registry_keeping(|registry| {
            // Another process's pending identity is this profile's too: reuse it.
            if let Some(prepared) = &registry.pending_device {
                return Ok(prepared.clone());
            }
            let preparation_id = uuid::Uuid::new_v4().to_string();
            let key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
            let public = key.verifying_key().to_bytes();
            self.write_credential(&format!("device:{preparation_id}"), &hex(&key.to_bytes()))?;
            let prepared = PreparedDevice {
                preparation_id,
                public_key: hex(&public),
                device_id: hex(&Sha256::digest(public)),
            };
            registry.pending_device = Some(prepared.clone());
            Ok(prepared)
        })
        .await?
    }
    pub async fn save(&self, input: SaveConnection) -> Result<Connection> {
        self.save_inner(None, input).await
    }
    pub async fn update(&self, id: &str, input: SaveConnection) -> Result<Connection> {
        let _lifecycle = self.connection_guard(id).await?;
        self.disconnect_locked(id).await?;
        self.save_inner(Some(id), input).await
    }
    fn validate_connection(input: &SaveConnection) -> Result<()> {
        ensure!(
            safe_atom(&input.ssh_target),
            "SSH target must be a host alias or user@host"
        );
        ensure!(
            safe_atom(&input.socket_path) && input.socket_path.starts_with('/'),
            "Socket path must be an absolute path without shell syntax"
        );
        uuid::Uuid::parse_str(&input.workspace_id)?;
        ensure!(
            unhex(&input.workspace_public_key)?.len() == 32,
            "Workspace public key must be 32 bytes encoded as hex"
        );
        ensure!(
            input.name.len() <= 120 && !input.name.trim().is_empty(),
            "Connection name must contain 1–120 characters"
        );
        if let Some(jump) = &input.proxy_jump {
            ensure!(jump.split(',').all(safe_atom), "Invalid ProxyJump route");
        }
        if let Some(identity) = &input.identity_file {
            ensure!(
                Path::new(identity).is_absolute() && !identity.contains('\n'),
                "Identity file must be an absolute path"
            );
        }
        Ok(())
    }
    fn connection_device(
        &self,
        connection_id: &str,
        old: Option<&Connection>,
        prepared: Option<&PreparedDevice>,
    ) -> Result<(String, String)> {
        Ok(if let Some(c) = old {
            (c.device_id.clone(), c.public_key.clone())
        } else if let Some(prepared) = prepared {
            let secret: [u8; 32] =
                unhex(&self.read_credential(&format!("device:{connection_id}"))?)?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Prepared device key is invalid"))?;
            ensure!(
                hex(&SigningKey::from_bytes(&secret).verifying_key().to_bytes())
                    == prepared.public_key,
                "Prepared public key does not match profile credential"
            );
            (prepared.device_id.clone(), prepared.public_key.clone())
        } else {
            let key = SigningKey::from_bytes(&rand::random::<[u8; 32]>());
            let public = key.verifying_key().to_bytes();
            let device = hex(&Sha256::digest(public));
            self.write_credential(&format!("device:{connection_id}"), &hex(&key.to_bytes()))?;
            (device, hex(&public))
        })
    }
    fn build_connection(
        &self,
        r: &mut Registry,
        id: Option<&str>,
        input: SaveConnection,
        prepared: Option<&PreparedDevice>,
    ) -> Result<Connection> {
        let old = if let Some(id) = id {
            Some(
                r.connections
                    .iter()
                    .find(|c| c.id == id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Unknown Crew connection"))?,
            )
        } else {
            None
        };
        let connection_id = old
            .as_ref()
            .map(|c| c.id.clone())
            .or_else(|| {
                prepared
                    .as_ref()
                    .map(|prepared| prepared.preparation_id.clone())
            })
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let (device_id, public_key) =
            self.connection_device(&connection_id, old.as_ref(), prepared)?;
        // Workspace aliases share the most restrictive existing cluster identity automatically.
        let canonical = r
            .connections
            .iter()
            .find(|c| c.workspace_id == input.workspace_id || c.ssh_target == input.ssh_target)
            .map(|c| c.cluster_connection_id.clone());
        let cluster = canonical
            .or_else(|| old.as_ref().map(|c| c.cluster_connection_id.clone()))
            .or(input.cluster_connection_id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        uuid::Uuid::parse_str(&cluster)?;
        let epoch = r
            .connections
            .iter()
            .filter(|c| c.cluster_connection_id == cluster)
            .map(|c| c.policy_epoch)
            .max()
            .unwrap_or(0)
            + 1;
        let mode = if r.connections.iter().any(|c| {
            c.cluster_connection_id == cluster
                && Some(c.id.as_str()) != id
                && c.mode == ClusterMode::Private
        }) {
            ClusterMode::Private
        } else {
            input.mode
        };
        let institution_id = institution::merge(
            r.connections
                .iter()
                .filter(|c| c.cluster_connection_id == cluster)
                .filter_map(|c| c.institution_id.as_deref())
                .chain(input.institution_id.as_deref()),
        )?;
        for c in r
            .connections
            .iter_mut()
            .filter(|c| c.cluster_connection_id == cluster)
        {
            c.mode = mode;
            c.institution_id = institution_id.clone();
            c.policy_epoch = epoch;
        }
        Ok(Connection {
            id: connection_id,
            node_id: old.as_ref().and_then(|c| c.node_id.clone()),
            name: input.name,
            ssh_target: input.ssh_target,
            port: input.port,
            identity_file: input.identity_file,
            proxy_jump: input.proxy_jump,
            socket_path: input.socket_path,
            owner_uid: input.owner_uid,
            workspace_id: input.workspace_id,
            workspace_public_key: input.workspace_public_key,
            remote_root: input.remote_root,
            remote_execution: input.remote_execution,
            cluster_connection_id: cluster,
            mode,
            institution_id,
            policy_epoch: epoch,
            status: "disconnected".into(),
            last_error: None,
            device_id,
            public_key,
        })
    }
    async fn save_inner(&self, id: Option<&str>, mut input: SaveConnection) -> Result<Connection> {
        Self::validate_connection(&input)?;
        input.institution_id = input
            .institution_id
            .as_deref()
            .map(institution::normalize)
            .transpose()?;
        // The refusals that need no saved state come before the registry is touched, so a
        // refused new connection creates nothing on disk.
        ensure!(
            id.is_some() || input.mode != ClusterMode::Private || input.institution_id.is_some(),
            "Choose this private SSH connection's institution before saving"
        );
        // Edits the saved registry as it is now, so a connection another process saved since
        // this one loaded is kept (D8). A refusal changes nothing.
        self.update_registry(|r| self.save_edit(r, id, input)).await
    }
    /// [`Self::save_inner`]'s edit of the saved registry, with `input` already validated and
    /// its institution normalized.
    fn save_edit(
        &self,
        r: &mut Registry,
        id: Option<&str>,
        mut input: SaveConnection,
    ) -> Result<Connection> {
        if input.institution_id.is_none() {
            input.institution_id = r
                .connections
                .iter()
                .find(|connection| Some(connection.id.as_str()) == id)
                .and_then(|connection| connection.institution_id.clone());
        }
        ensure!(
            input.mode != ClusterMode::Private || input.institution_id.is_some(),
            "Choose this private SSH connection's institution before saving"
        );
        let preparation_hash = hex(&Sha256::digest(serde_json::to_vec(&input)?));
        let prepared = if let Some(preparation_id) = &input.preparation_id {
            ensure!(
                id.is_none(),
                "Prepared device identities are used only when saving a new connection"
            );
            uuid::Uuid::parse_str(preparation_id)?;
            if let Some(expected) = r.completed_preparations.get(preparation_id) {
                ensure!(
                    expected == &preparation_hash,
                    "Prepared identity was already saved with different connection settings"
                );
                let connection = r
                    .connections
                    .iter()
                    .find(|connection| &connection.id == preparation_id)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Prepared connection was removed; prepare a new device identity"
                        )
                    })?;
                return Ok(connection);
            }
            Some(
                r.pending_device
                    .as_ref()
                    .filter(|prepared| &prepared.preparation_id == preparation_id)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Prepared identity is unavailable in this profile")
                    })?,
            )
        } else {
            None
        };
        let c = self.build_connection(r, id, input, prepared.as_ref())?;
        r.connections.retain(|old| old.id != c.id);
        r.connections.push(c.clone());
        if let Some(prepared) = prepared {
            r.completed_preparations
                .insert(prepared.preparation_id, preparation_hash);
            r.pending_device = None;
        }
        Ok(c)
    }
    /// Remove a saved connection: disconnect it, drop it from the registry (its grants stay,
    /// expired), then delete its device private key (T-50). The key goes last, so a registry
    /// write that fails never leaves a saved connection without its key; a key that is already
    /// gone is not an error.
    pub async fn remove(&self, id: &str) -> Result<()> {
        let _lifecycle = self.connection_guard(id).await?;
        self.disconnect_locked(id).await?;
        let pending_preparation = self
            .update_registry_keeping(|r| {
                r.connections.retain(|c| c.id != id);
                for scope in r.scopes.values_mut().filter(|s| s.connection_id == id) {
                    scope.expired = true;
                }
                // A prepared device not yet saved keeps its key under the same kind of ID; that
                // one belongs to the next save, not to this removal.
                Ok(r.pending_device
                    .as_ref()
                    .is_some_and(|prepared| prepared.preparation_id == id))
            })
            .await??;
        if pending_preparation {
            return Ok(());
        }
        self.delete_credential(&format!("device:{id}"))
            .map_err(|error| {
                error.context(
                    "The connection was removed, but its device key couldn't be deleted from this computer",
                )
            })
    }
    pub async fn authentication_plan(&self, id: &str) -> Result<AuthenticationPlan> {
        let c = self.connection(id).await?;
        let mut args = transport::ssh_args(&c, &self.control_path(id)?);
        args.extend([
            "-M".into(),
            "-N".into(),
            "-o".into(),
            "ControlPersist=600".into(),
            c.ssh_target.clone(),
        ]);
        ssh_policy::preflight(&args, &c.ssh_target).await?;
        Ok(AuthenticationPlan {
            program: "ssh".into(),
            args,
            connection_id: id.into(),
            authentication_id: uuid::Uuid::new_v4().to_string(),
        })
    }
    /// Check `hello` against what this connection pinned, and verify every workspace signature
    /// it carries: v1 (`signature`) over the identity, and v2 (`signature_v2`) over the identity
    /// plus the workspace's name, mode, institution, policy epoch and capabilities. At least one
    /// must be present, and any present must verify, so a relay can strip v2 (downgrading those
    /// fields to unauthenticated hints, which the caller withholds) but never alter them.
    fn verify_workspace_identity(
        c: &Connection,
        hello: &Value,
        challenge_nonce: &str,
    ) -> Result<VerifiedHello> {
        Self::verify_workspace_identity_inner(c, hello, challenge_nonce)
            .map_err(WorkspaceIdentityError::wrap)
    }
    fn verify_workspace_identity_inner(
        c: &Connection,
        hello: &Value,
        challenge_nonce: &str,
    ) -> Result<VerifiedHello> {
        ensure!(
            hello["workspace_id"].as_str() == Some(&c.workspace_id),
            "Workspace identity mismatch"
        );
        ensure!(
            hello["host_uid"].as_u64() == Some(c.owner_uid as u64),
            "Broker owner identity mismatch"
        );
        ensure!(hello["workspace_public_key"].as_str()==Some(c.workspace_public_key.as_str()), "Workspace public key changed or is missing; verify the workspace descriptor before reconnecting");
        ensure!(
            hello["challenge_nonce"].as_str() == Some(challenge_nonce),
            "Workspace identity challenge mismatch"
        );
        let public_key: [u8; 32] = unhex(&c.workspace_public_key)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid workspace key"))?;
        let verifying = VerifyingKey::from_bytes(&public_key)?;
        let v1 = hello_signature(hello, "signature")?;
        let v2 = hello_signature(hello, "signature_v2")?;
        ensure!(
            v1.is_some() || v2.is_some(),
            "Workspace identity signature missing"
        );
        let node_id = hello["node_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Verified node identity is missing"))?;
        ensure!(
            node_id.len() == 64
                && node_id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "Invalid node identity"
        );
        if let Some(v1) = v1 {
            let signed = biorouter_crew::hello_v1_payload(
                &c.workspace_id,
                c.owner_uid,
                challenge_nonce,
                &c.workspace_public_key,
                node_id,
            );
            verifying.verify(&signed, &v1)?;
        }
        let broker = match v2 {
            Some(v2) => {
                HelloV2Fields::read(hello)?.verify(c, challenge_nonce, node_id, &verifying, &v2)?
            }
            None => BrokerHello::unsigned(hello),
        };
        Ok(VerifiedHello {
            node_id: node_id.to_string(),
            broker,
        })
    }
    pub async fn connection_guard(&self, id: &str) -> Result<tokio::sync::OwnedMutexGuard<()>> {
        let lock = {
            let mut locks = self
                .lifecycle
                .lock()
                .map_err(|_| anyhow::anyhow!("Connection lifecycle unavailable"))?;
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(id).and_then(std::sync::Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(id.into(), Arc::downgrade(&lock));
                lock
            }
        };
        Ok(lock.lock_owned().await)
    }
    pub async fn connect(&self, id: &str) -> Result<Connection> {
        let _lifecycle = self.connection_guard(id).await?;
        authentication::ensure_connect_available(id)?;
        self.connect_locked(id).await
    }
    pub(super) async fn connect_locked(&self, id: &str) -> Result<Connection> {
        let c = self.connection(id).await?;
        let mut transport = transport::Transport::connect(&c, &self.control_path(id)?).await?;
        let challenge_nonce = hello_nonce();
        let hello = transport
            .request(
                "hello",
                json!({"challenge_nonce":challenge_nonce}),
                None,
                None,
                None,
            )
            .await?;
        let verified = Self::verify_workspace_identity(&c, &hello, &challenge_nonce)?;
        let connected = self.adopt_verified_hello(id, &c, verified).await?;
        let transport = Arc::new(Mutex::new(transport));
        let replaced = self
            .transports
            .lock()
            .await
            .insert(id.into(), Arc::clone(&transport));
        if let Some(replaced) = replaced {
            // Its keepalive sees it is no longer current and stops; its `ssh` ends here, or
            // when a request still holding it lets go.
            if let Ok(mut old) = replaced.try_lock() {
                old.close().await;
            }
        }
        // Connected again, by whoever asked: no idle re-dial is still owed.
        self.disarm_idle_redial(id);
        self.start_keepalive(id, &transport);
        Ok(connected)
    }
    /// Pin the verified node, merge its cluster, persist, and only then remember what the
    /// broker announced. Nothing from `hello` other than the node enters the saved connection.
    async fn adopt_verified_hello(
        &self,
        id: &str,
        c: &Connection,
        verified: VerifiedHello,
    ) -> Result<Connection> {
        let VerifiedHello { node_id, broker } = verified;
        let connected = self
            .update_registry_keeping(|registry| Self::adopt_node(registry, id, c, node_id))
            .await??;
        self.remember_broker(id, broker);
        Ok(connected)
    }
    /// Ask the connected broker for a fresh `hello` over a fresh challenge nonce, verify it
    /// against the pinned workspace identity **and** the node this connection already pinned,
    /// and replace the cached announcement with it. What `hello` says changes while a
    /// connection stays up (a host sets the institution, renames the workspace), and the cache
    /// is otherwise only written at connect (T-10).
    ///
    /// Never re-pins and never writes the saved connection: a different node is an identity
    /// error, and a v1-only answer where v2 was cached (a relay stripping the signature that
    /// covers the institution) is refused, not cached. The cache is written only while the
    /// transport that answered is still this connection's live one, so a disconnect racing the
    /// refresh never has a stale announcement written back.
    pub(super) async fn refresh_broker_hello(&self, id: &str) -> Result<BrokerHello> {
        let c = self.connection(id).await?;
        let pinned = c
            .node_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Connect to this workspace first."))?;
        let transport = self.transport(id).await?;
        let (answer, usable) = self.hello_over(id, &c, &pinned, &transport).await;
        if !usable {
            self.retire_failed_transport(id, &transport).await?;
        }
        answer
    }
    /// [`Self::refresh_broker_hello`]'s exchange over one given bridge, and whether that bridge
    /// is still usable afterwards. Never retires it: the caller decides what a dead bridge
    /// means (a refresh retires it; the keepalive dials again).
    pub(super) async fn hello_over(
        &self,
        id: &str,
        c: &Connection,
        pinned: &str,
        transport: &Arc<Mutex<transport::Transport>>,
    ) -> (Result<BrokerHello>, bool) {
        let cached = self.broker_hello(id);
        let challenge_nonce = hello_nonce();
        let mut locked = transport.lock().await;
        let answer = locked
            .request(
                "hello",
                json!({"challenge_nonce": challenge_nonce}),
                None,
                None,
                None,
            )
            .await;
        let usable = locked.is_usable();
        drop(locked);
        let verified = match answer
            .and_then(|hello| Self::verify_workspace_identity(c, &hello, &challenge_nonce))
        {
            Ok(verified) => verified,
            Err(error) => return (Err(error), usable),
        };
        (
            self.adopt_refreshed_hello(id, pinned, cached, verified, transport)
                .await,
            usable,
        )
    }
    async fn adopt_refreshed_hello(
        &self,
        id: &str,
        pinned: &str,
        cached: Option<BrokerHello>,
        verified: VerifiedHello,
        transport: &Arc<Mutex<transport::Transport>>,
    ) -> Result<BrokerHello> {
        if verified.node_id != pinned {
            return Err(WorkspaceIdentityError::wrap(anyhow::anyhow!(
                "Verified SSH node identity changed; create a newly verified connection"
            )));
        }
        ensure!(
            cached
                .as_ref()
                .is_none_or(|cached| verified.broker.signature_version >= cached.signature_version),
            "The workspace's answer lost its signature; reconnect to this workspace."
        );
        let transports = self.transports.lock().await;
        ensure!(
            transports
                .get(id)
                .is_some_and(|current| Arc::ptr_eq(current, transport)),
            "Crew connection changed while checking the workspace; reconnect and try again."
        );
        self.remember_broker(id, verified.broker.clone());
        drop(transports);
        Ok(verified.broker)
    }
    /// [`Self::adopt_verified_hello`]'s edit of the saved registry, as it is now.
    fn adopt_node(
        registry: &mut Registry,
        id: &str,
        c: &Connection,
        node_id: String,
    ) -> Result<Connection> {
        let current = registry
            .connections
            .iter()
            .find(|current| current.id == id)
            .ok_or_else(|| anyhow::anyhow!("Connection removed while connecting"))?;
        ensure!(
            connection_binding(current)? == connection_binding(c)?,
            "Connection changed while authentication was pending; reconnect"
        );
        if let Some(previous) = &current.node_id {
            if previous != &node_id {
                return Err(WorkspaceIdentityError::wrap(anyhow::anyhow!(
                    "Verified SSH node identity changed; create a newly verified connection"
                )));
            }
        }
        let mut groups: std::collections::BTreeSet<String> = registry
            .connections
            .iter()
            .filter(|entry| entry.node_id.as_deref() == Some(node_id.as_str()))
            .map(|entry| entry.cluster_connection_id.clone())
            .collect();
        groups.insert(c.cluster_connection_id.clone());
        let canonical = groups.first().cloned().expect("current cluster exists");
        let mode = if registry.connections.iter().any(|entry| {
            groups.contains(&entry.cluster_connection_id) && entry.mode == ClusterMode::Private
        }) {
            ClusterMode::Private
        } else {
            ClusterMode::Public
        };
        if let Some(refusal) = Self::mixed_institutions(registry, &groups, c) {
            return Err(anyhow::anyhow!(refusal));
        }
        let institution_id = institution::merge(
            registry
                .connections
                .iter()
                .filter(|entry| groups.contains(&entry.cluster_connection_id))
                .filter_map(|entry| entry.institution_id.as_deref()),
        )?;
        let changed = groups.len() > 1
            || c.node_id.as_deref() != Some(node_id.as_str())
            || registry.connections.iter().any(|entry| {
                groups.contains(&entry.cluster_connection_id)
                    && entry.institution_id != institution_id
            });
        let epoch = registry
            .connections
            .iter()
            .filter(|entry| groups.contains(&entry.cluster_connection_id))
            .map(|entry| entry.policy_epoch)
            .max()
            .unwrap_or(c.policy_epoch)
            + u64::from(changed);
        for entry in registry
            .connections
            .iter_mut()
            .filter(|entry| groups.contains(&entry.cluster_connection_id))
        {
            entry.cluster_connection_id = canonical.clone();
            entry.mode = mode;
            entry.institution_id = institution_id.clone();
            entry.policy_epoch = epoch;
            if entry.id == id {
                entry.node_id = Some(node_id.clone());
                entry.status = "connected".into();
                entry.last_error = None;
            }
        }
        Ok(registry
            .connections
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
            .expect("validated connection"))
    }
    /// T-52: connections on one server share one institution, because the server's node is
    /// one privacy boundary. When joining `c` would mix two, the refusal (it stays a refusal)
    /// says so in people's words: which saved connection already uses this server, and for
    /// which institution.
    fn mixed_institutions(
        registry: &Registry,
        groups: &std::collections::BTreeSet<String>,
        c: &Connection,
    ) -> Option<String> {
        let normalized = |entry: &Connection| {
            entry
                .institution_id
                .as_deref()
                .map(|id| institution::normalize(id).unwrap_or_else(|_| id.to_owned()))
        };
        let on_server: Vec<&Connection> = registry
            .connections
            .iter()
            .filter(|entry| groups.contains(&entry.cluster_connection_id))
            .collect();
        let joining = on_server
            .iter()
            .find(|entry| entry.id == c.id)
            .copied()
            .unwrap_or(c);
        let (this, this_institution) = match normalized(joining) {
            Some(institution) => (joining, institution),
            None => on_server
                .iter()
                .find_map(|entry| normalized(entry).map(|institution| (*entry, institution)))?,
        };
        let (other, other_institution) = on_server.iter().find_map(|entry| {
            normalized(entry)
                .filter(|institution| *institution != this_institution)
                .map(|institution| (*entry, institution))
        })?;
        Some(format!(
            "You already use this server for {} ({}). {} uses {}; one computer can't mix institutions on the same server.",
            plain_label(&other.name),
            plain_label(&other_institution),
            plain_label(&this.name),
            plain_label(&this_institution),
        ))
    }
    pub async fn disconnect(&self, id: &str) -> Result<()> {
        let _lifecycle = self.connection_guard(id).await?;
        self.disconnect_locked(id).await
    }
    pub(super) async fn disconnect_locked(&self, id: &str) -> Result<()> {
        // A disconnect (the person's, or an edit or removal) ends any idle re-dial for good.
        self.disarm_idle_redial(id);
        authentication::cancel_connection(id);
        if let Ok(connection) = self.connection(id).await {
            let control = self.control_path(id)?;
            if control.exists() {
                let mut args = transport::ssh_args(&connection, &control);
                args.extend(["-O".into(), "exit".into(), connection.ssh_target]);
                let mut command = tokio::process::Command::new("ssh");
                command
                    .args(args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true);
                crate::subprocess::prepare_agent_child_command(&mut command);
                let mut child = command.spawn()?;
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
            }
        }
        let removed = { self.transports.lock().await.remove(id) };
        if let Some(t) = removed {
            t.lock().await.close().await;
        }
        // The next connect may reach another broker binary (an upgrade, or an edited target),
        // so what this one announced is not carried over.
        self.forget_broker(id);
        let mut r = self.registry.lock().await;
        if let Some(c) = r.connections.iter_mut().find(|c| c.id == id) {
            c.status = "disconnected".into();
        }
        Ok(())
    }
    async fn retire_failed_transport(
        &self,
        id: &str,
        failed: &Arc<Mutex<transport::Transport>>,
    ) -> Result<()> {
        // Callers release the transport mutex before taking lifecycle ownership.
        // Connect/update/remove hold this same guard while replacing publication.
        let _lifecycle = self.connection_guard(id).await?;
        self.retire_locked(id, failed, BRIDGE_FAILED).await;
        Ok(())
    }
    /// Unpublish `failed` if it is still `id`'s bridge, mark the connection disconnected with
    /// `message`, and end its `ssh`. The caller holds the connection's lifecycle guard.
    pub(super) async fn retire_locked(
        &self,
        id: &str,
        failed: &Arc<Mutex<transport::Transport>>,
        message: &str,
    ) {
        let removed = {
            let mut transports = self.transports.lock().await;
            if transports
                .get(id)
                .is_some_and(|current| Arc::ptr_eq(current, failed))
            {
                transports.remove(id)
            } else {
                None
            }
        };
        if let Some(removed) = removed {
            let mut registry = self.registry.lock().await;
            if let Some(connection) = registry.connections.iter_mut().find(|c| c.id == id) {
                connection.status = "disconnected".into();
                connection.last_error = Some(message.into());
            }
            drop(registry);
            removed.lock().await.close().await;
        }
    }
    async fn transport(&self, id: &str) -> Result<Arc<Mutex<transport::Transport>>> {
        self.transports
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("Crew connection is disconnected; authenticate and connect in Crew")
            })
    }
    pub async fn human_request(
        &self,
        id: &str,
        method: &str,
        params: Value,
        request_id: Option<String>,
    ) -> Result<Value> {
        ensure!(
            method != "run.create",
            "Agent grants must be created through the trusted provider-bound session action"
        );
        let result = self.signed_request(id, method, params, request_id).await?;
        // A host's policy or name change is signed into the next `hello`; take it now, so
        // what this connection shows and puts in invitations is never the connect-time answer
        // (T-10). The change itself succeeded either way; a failed refresh only leaves the
        // cache stale, and `invitation_for` refreshes (or refuses) again before it relies on it.
        if matches!(method, "policy.set" | "workspace.rename") {
            if let Err(error) = self.refresh_broker_hello(id).await {
                tracing::warn!(
                    connection = id,
                    method,
                    error = %error,
                    "Couldn't refresh the workspace's signed hello after a change"
                );
            }
        }
        Ok(result)
    }
    async fn signed_request(
        &self,
        id: &str,
        method: &str,
        params: Value,
        request_id: Option<String>,
    ) -> Result<Value> {
        self.signed_request_via(SignedDoor::Generic, id, method, params, request_id)
            .await
    }
    /// The one door `auth.join` leaves this daemon through: the S3a join in
    /// `authentication.rs`, signed with the connection's saved device key. Everything else,
    /// every route and tool included, reaches the broker through [`Self::human_request`],
    /// which refuses it.
    #[allow(
        dead_code,
        reason = "the S3a join in authentication.rs is its caller; until then it keeps the guard's second arm reachable"
    )]
    async fn signed_join_request(
        &self,
        id: &str,
        params: Value,
        request_id: Option<String>,
    ) -> Result<Value> {
        self.signed_request_via(SignedDoor::Join, id, "auth.join", params, request_id)
            .await
    }
    async fn signed_request_via(
        &self,
        door: SignedDoor,
        id: &str,
        method: &str,
        mut params: Value,
        request_id: Option<String>,
    ) -> Result<Value> {
        // Joining is the daemon's own act, with the key it saved for this connection; the
        // join status is read unsigned before authentication, as `hello` is. Neither may be
        // sent on a caller's say-so, and the join door sends nothing else.
        ensure!(
            method != "enrollment.pending",
            "Biorouter checks join status itself; it is never sent as a signed request"
        );
        ensure!(
            (method == "auth.join") == (door == SignedDoor::Join),
            "Only Biorouter's own join sends auth.join; it can't be sent as a Crew request"
        );
        ensure!(params.is_object(), "Crew params must be an object");
        let c = self.connection(id).await?;
        if method == "run.create" {
            let expected = params.as_object_mut().unwrap().remove("expected_mode");
            ensure!(
                expected.as_ref().is_none_or(|mode| mode == &json!(c.mode)),
                "Crew connection privacy changed; refresh the verified workspace before granting agent access"
            );
            let expected_epoch = params
                .as_object_mut()
                .unwrap()
                .remove("expected_policy_epoch");
            ensure!(
                expected_epoch
                    .as_ref()
                    .is_none_or(|epoch| epoch.as_u64() == Some(c.policy_epoch)),
                "Crew connection policy changed; refresh before granting agent access"
            );
        }
        if method == "policy.set" {
            if let Some(value) = params
                .get("institution_id")
                .filter(|value| !value.is_null())
            {
                params["institution_id"] =
                    json!(institution::normalize(value.as_str().ok_or_else(
                        || anyhow::anyhow!("Invalid Crew institution ID")
                    )?)?);
            }
        }
        if matches!(method, "message.post" | "blob.begin") {
            let mode = json!(c.mode);
            ensure!(
                params
                    .get("personal_mode")
                    .is_none_or(|expected| expected == &mode),
                "Crew connection privacy changed; refresh the verified workspace before sending"
            );
            params["personal_mode"] = mode;
        }
        if params.get("idempotency_key").is_none() {
            params["idempotency_key"] = json!(request_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()));
        }
        let key: [u8; 32] = unhex(&self.read_credential(&format!("device:{id}"))?)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid device key"))?;
        let signer = SigningKey::from_bytes(&key);
        let public = signer.verifying_key().to_bytes();
        ensure!(
            hex(&public) == c.public_key && hex(&Sha256::digest(public)) == c.device_id,
            "Saved Crew device identity does not match its signing credential; reconnect using a verified device identity"
        );
        if matches!(method, "auth.bootstrap" | "auth.enroll" | "auth.join") {
            ensure!(
                params["public_key"].as_str() == Some(c.public_key.as_str()),
                "Enrollment identity changed; refresh the saved connection before joining"
            );
        }
        // A bridge that ended, or sat idle long enough for the broker to drop it, is checked
        // (and dialled again without a prompt) before anything is written to it.
        let transport = self.live_transport(id).await?;
        let mut locked = transport.lock().await;
        let result = self
            .signed_exchange(&mut locked, &c, method, params, request_id, &signer)
            .await;
        let usable = locked.is_usable();
        drop(locked);
        if !usable {
            self.retire_failed_transport(id, &transport).await?;
        }
        result
    }
    async fn signed_exchange(
        &self,
        t: &mut transport::Transport,
        c: &Connection,
        method: &str,
        params: Value,
        request_id: Option<String>,
        signer: &SigningKey,
    ) -> Result<Value> {
        let id = &c.id;
        let fresh = self.connection(id).await?;
        ensure!(
            fresh.policy_epoch == c.policy_epoch
                && fresh.mode == c.mode
                && fresh.institution_id == c.institution_id
                && fresh.workspace_id == c.workspace_id
                && fresh.workspace_public_key == c.workspace_public_key,
            "Crew connection policy changed while this action was queued; review and retry"
        );
        let challenge = t
            .request(
                "auth.challenge",
                json!({"device_id":c.device_id}),
                None,
                None,
                None,
            )
            .await?;
        ensure!(
            challenge["workspace_id"].as_str() == Some(&c.workspace_id),
            "Challenge workspace mismatch"
        );
        let nonce = challenge["nonce"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing authentication challenge"))?;
        let fresh = self.connection(id).await?;
        ensure!(
            fresh.policy_epoch == c.policy_epoch
                && fresh.mode == c.mode
                && fresh.institution_id == c.institution_id,
            "Crew connection policy changed during authentication; review and retry"
        );
        let bytes = serde_json::to_vec(&json!([
            c.workspace_id,
            challenge["uid"],
            nonce,
            method,
            canonical(&params)
        ]))?;
        let signature = hex(&signer.sign(&bytes).to_bytes());
        t.request(
            method,
            params,
            Some(json!({"device_id":c.device_id,"nonce":nonce,"signature":signature})),
            None,
            request_id,
        )
        .await
    }

    /// The run of the chat's grant, including one that is [`Standing::Unconfirmed`] — it is
    /// still the grant to show and to revoke — but never one made to an earlier chat under
    /// the same id.
    pub async fn run_metadata(&self, session: &str) -> Option<RunMetadata> {
        match self.standing(session).await {
            Standing::None => None,
            Standing::Own(scope) | Standing::Unconfirmed(scope, _) => Some(RunMetadata {
                run_id: scope.run_id,
                connection_id: scope.connection_id,
                channel_id: scope.channel_id,
            }),
        }
    }
    /// Every chat a grant restricts: each id whose grant is its chat's own or cannot be
    /// confirmed, and none whose grant was made to an earlier chat under the same id.
    pub async fn scoped_session_ids(&self) -> std::collections::HashSet<String> {
        let sessions: Vec<String> = self.registry.lock().await.scopes.keys().cloned().collect();
        let mut scoped = std::collections::HashSet::with_capacity(sessions.len());
        for session in sessions {
            if self.is_scoped(&session).await {
                scoped.insert(session);
            }
        }
        scoped
    }
    pub async fn is_scoped_session(&self, session: &str) -> bool {
        self.is_scoped(session).await
    }
    pub async fn authorize_session_tool(&self, session: &str, name: &str) -> Result<()> {
        if self.is_scoped(session).await {
            ensure!(name.starts_with("crew__") || name.starts_with("todo__"), "Crew-scoped conversations permit only Crew and checklist tools until worker isolation is verified");
        }
        Ok(())
    }
    pub async fn agent_connections(&self, session: &str) -> Result<Value> {
        let s = self.scope(session).await?;
        let c = self.connection(&s.connection_id).await?;
        self.validate_worker_scope(session, &s, &c).await?;
        Ok(
            json!({"connections":[{"id":c.id,"name":c.name,"status":c.status,"mode":c.mode,"workspace_id":c.workspace_id,"destination_channel_id":s.channel_id,"source_channel_ids":s.source_channels,"labels":s.labels,"naming":"labels gives the names of the IDs above as the person saw them when granting access. Refer to people as Display name (@username) and to channels as #name. Never quote IDs to people.","context_discovery":"Use context.manifest with empty params for recent authorized selected-channel context. Search each relevant source_channel_id with messages.search using channel_id and query; history and search are per-channel.","remote_files_enabled":!s.public_provider && c.remote_root.is_some(),"remote_execution_enabled":!s.public_provider && c.remote_root.is_some() && c.remote_execution,"remote_path_base":"the granted SSH work directory, not the local task directory; supply relative paths"}]}),
        )
    }
    /// Whether a grant restricts this chat: its own, or one it cannot be confirmed not to
    /// hold. Never a grant made to an earlier chat under the same id (SCOPE-BIND).
    pub async fn is_scoped(&self, session: &str) -> bool {
        !matches!(self.standing(session).await, Standing::None)
    }
    /// The chat's own grant, for acting under it. An unconfirmed grant is refused here: it
    /// restricts, but it never authorizes.
    async fn scope(&self, session: &str) -> Result<Scope> {
        match self.standing(session).await {
            Standing::Own(scope) => Ok(scope),
            Standing::Unconfirmed(_, reason) => Err(anyhow::anyhow!(reason)),
            Standing::None => Err(anyhow::anyhow!(NO_GRANT)),
        }
    }
    /// The chat's own grant as the registry holds it now, with its connection (if it still
    /// exists), read under one lock so a check sees one snapshot. `None` when no grant is
    /// the chat's.
    async fn checked_scope(&self, session: &str) -> Result<Option<(Scope, Option<Connection>)>> {
        let own = match self.standing(session).await {
            Standing::None => return Ok(None),
            Standing::Unconfirmed(_, reason) => anyhow::bail!(reason),
            Standing::Own(scope) => scope,
        };
        let registry = self.registry.lock().await;
        let scope = registry
            .scopes
            .get(session)
            .filter(|current| {
                current.run_id == own.run_id
                    && current.session_incarnation == own.session_incarnation
            })
            .cloned()
            .ok_or_else(|| anyhow::anyhow!(ACCESS_CHANGED))?;
        let connection = registry
            .connections
            .iter()
            .find(|c| c.id == scope.connection_id)
            .cloned();
        Ok(Some((scope, connection)))
    }
    pub async fn check_dispatch(&self, session: &str, cap: &CallCapability) -> Result<()> {
        self.check_tier(session, cap.tier(), cap.affiliation())
            .await
    }
    async fn check_tier(
        &self,
        session: &str,
        tier: ProviderTier,
        affiliation: Option<crate::privacy::affiliation::ModelAffiliation>,
    ) -> Result<()> {
        let Some((s, c)) = self.checked_scope(session).await? else {
            return Ok(());
        };
        ensure!(!s.expired, GRANT_REVOKED);
        // A scope granted before institution policy existed never passed today's admission.
        ensure!(s.institution_policy, GRANT_POLICY_CHANGED);
        let c = c.ok_or_else(|| anyhow::anyhow!("Crew connection was removed"))?;
        ensure!(s.epoch == c.policy_epoch, GRANT_POLICY_CHANGED);
        ensure!(
            tier != ProviderTier::Public
                || (c.mode == ClusterMode::Public && s.public_provider && !s.origin_restricted),
            "Private Crew context cannot be sent to a public model"
        );
        institution::check_provider(tier, affiliation, &s.institution_ids)?;
        Ok(())
    }
    pub async fn check_provider_binding(
        &self,
        session: &str,
        provider: &dyn Provider,
    ) -> Result<()> {
        let Some((scope, connection)) = self.checked_scope(session).await? else {
            return Ok(());
        };
        ensure!(
            !provider.uses_tool_bridge(),
            "Crew cannot bind a provider with unscoped external tools"
        );
        ensure!(scope.provider_binding==provider_binding(provider),"Crew conversation remains bound to its original resolved provider; start a fresh conversation for another model boundary");
        let connection =
            connection.ok_or_else(|| anyhow::anyhow!("Crew connection was removed"))?;
        ensure!(
            provider.tier() != ProviderTier::Public
                || (connection.mode == ClusterMode::Public
                    && scope.public_provider
                    && !scope.origin_restricted),
            "Private Crew context cannot be bound to a public model"
        );
        institution::check_provider(
            provider.tier(),
            provider.affiliation(),
            &scope.institution_ids,
        )?;
        Ok(())
    }
    /// Every check a provider call on `session` must pass: the binding, the tier, and — for a
    /// scoped chat — a live `context.manifest` through the worker path.
    ///
    /// ⚠ The body runs **boxed**, and must stay that way. `Agent::provider` awaits this, and
    /// `Agent::reply` reaches `Agent::provider` through the tool-surface preparation, so an
    /// inline future here (binding + tier + standing + a worker request) is laid out inside
    /// `Agent::reply`'s own frame — the known stack cliff. Inline, it overflowed the default
    /// 2 MiB test-thread stack in `privacy_toggle`'s
    /// `the_master_toggle_governs_every_gate_in_both_directions`. Boxing keeps this frame one
    /// pointer wide for every caller, whatever the checks grow into.
    pub async fn check_provider_dispatch(
        &self,
        session: &str,
        provider: &dyn Provider,
    ) -> Result<()> {
        Box::pin(self.check_provider_dispatch_inner(session, provider)).await
    }
    async fn check_provider_dispatch_inner(
        &self,
        session: &str,
        provider: &dyn Provider,
    ) -> Result<()> {
        self.check_provider_binding(session, provider).await?;
        self.check_tier(session, provider.tier(), provider.affiliation())
            .await?;
        if self.is_scoped(session).await {
            self.worker_request(session, "context.manifest", json!({}))
                .await?;
        }
        Ok(())
    }
    pub async fn preflight_run(
        &self,
        id: &str,
        channel: &str,
        sources: &[String],
        provider: &dyn Provider,
        policy: &RunPolicy,
    ) -> Result<Connection> {
        Ok(self
            .checked_run_admission(id, channel, sources, provider, policy)
            .await?
            .0
            .connection)
    }
    /// Admission under the person's action: the one place a run's snapshot is fetched, so the
    /// display labels are captured here (D14) and never by a worker. The labels cover
    /// `sources` followed by `channel` when `sources` lacks it, which is the run's final list.
    async fn checked_run_admission(
        &self,
        id: &str,
        channel: &str,
        sources: &[String],
        provider: &dyn Provider,
        policy: &RunPolicy,
    ) -> Result<(institution::Admission, AdmissionLabels)> {
        ensure!(!provider.uses_tool_bridge(), "Crew cannot admit providers with external tools outside its scoped capability boundary");
        let connection = self.connection(id).await?;
        ensure!(
            policy
                .expected_mode
                .is_none_or(|mode| mode == connection.mode),
            "Crew connection privacy changed; refresh the verified workspace before granting agent access"
        );
        ensure!(
            policy
                .expected_policy_epoch
                .is_none_or(|epoch| epoch == connection.policy_epoch),
            "Crew connection policy changed; refresh before granting agent access"
        );
        let public = provider.tier() == ProviderTier::Public;
        ensure!(
            !public || connection.mode == ClusterMode::Public,
            "Private cluster blocks public models"
        );
        ensure!(
            !public || !policy.origin_restricted,
            "Private-origin local conversation cannot be admitted to a public Crew worker"
        );
        let snapshot = self
            .human_request(id, "workspace.snapshot", json!({}), None)
            .await?;
        let protected = institution::protected_sources(&snapshot, channel, sources)?;
        let mut listed = sources.to_vec();
        if !listed.iter().any(|source| source == channel) {
            listed.push(channel.into());
        }
        let labels = admission_labels(&snapshot, &connection, channel, &listed);
        Ok((
            institution::admission(connection, provider, policy, &snapshot, protected)?,
            labels,
        ))
    }
    pub async fn begin_run(
        &self,
        session: &str,
        id: &str,
        channel: &str,
        sources: Vec<String>,
        provider: &dyn Provider,
    ) -> Result<RunAdmission> {
        self.begin_run_with_policy(
            session,
            id,
            channel,
            sources,
            provider,
            RunPolicy::default(),
        )
        .await
    }
    pub async fn begin_run_with_policy(
        &self,
        session: &str,
        id: &str,
        channel: &str,
        mut sources: Vec<String>,
        provider: &dyn Provider,
        mut policy: RunPolicy,
    ) -> Result<RunAdmission> {
        let public = provider.tier() == ProviderTier::Public;
        self.carry_previous_grant(session, id, channel, &mut sources, provider, &mut policy)
            .await?;
        let origin_restricted = policy.origin_restricted;
        let (admission, labels) = self
            .checked_run_admission(id, channel, &sources, provider, &policy)
            .await?;
        let c = admission.connection;
        let institution_ids = admission.institution_ids;
        ensure!(
            !public || !origin_restricted,
            "Private-origin local conversation cannot be admitted to a public Crew worker"
        );
        institution::check_origin(
            &institution_ids,
            admission.workspace_institution_id.as_deref(),
        )?;
        institution::check_provider(provider.tier(), provider.affiliation(), &institution_ids)?;
        if !sources.iter().any(|s| s == channel) {
            sources.push(channel.into());
        }
        // Bound before anything is created at the workspace, so a chat that is not saved
        // here (`--no-session`, or deleted meanwhile) never leaves a live run behind.
        let session_incarnation = self.grantable_chat(session).await?;
        let result=self.signed_request(id,"run.create",json!({"expected_mode":c.mode,"expected_policy_epoch":c.policy_epoch,"expected_workspace_policy_epoch":admission.workspace_policy_epoch,"expected_protected_context":admission.protected_context,"workspace_institution_id":admission.workspace_institution_id,"connection_institution_id":c.institution_id,"provider_affiliation":institution::provider_affiliation(provider),"channel_id":channel,"source_channels":sources,"provider_policy_id":provider_binding(provider),"personal_mode":if origin_restricted {ClusterMode::Private}else{c.mode},"public_provider":public,"expires_in":3600,"remote_root":if public {None}else{c.remote_root.clone()},"remote_execution":!public && c.remote_execution}),None).await?;
        // Without its ID there is no run to revoke; with it, the workspace now honors a run,
        // and every failure from here on must take it back (D7).
        let run_id = result["run"]["id"]
            .as_str()
            .or_else(|| result["run"]["run_id"].as_str())
            .ok_or_else(|| anyhow::anyhow!("Broker did not return a run ID"))?
            .to_string();
        let mut credential_written = false;
        let admitted: Result<RunAdmission> = async {
            ensure!(
                result["run"]["protected_context"].as_bool() == Some(admission.protected_context),
                "Crew broker returned a different protected-context policy; refresh before granting agent access"
            );
            let credential = result["credential"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Broker did not return a scoped credential"))?;
            let expires_at = result["run"]["expires_at"].as_u64();
            self.write_credential(&format!("run:{session}"), credential)?;
            credential_written = true;
            let scope = Scope {
                connection_id: id.into(),
                run_id: run_id.clone(),
                channel_id: channel.into(),
                source_channels: sources.clone(),
                epoch: c.policy_epoch,
                provider_binding: provider_binding(provider),
                public_provider: public,
                origin_restricted,
                institution_ids: institution_ids.clone(),
                institution_policy: true,
                expired: false,
                expires_at,
                labels: Some(labels.clone()),
                session_incarnation: Some(session_incarnation),
            };
            // Recorded here even when the save fails, as it always was: the abandon below
            // then finds it and stops it here too.
            self.update_registry_keeping(|r| {
                r.scopes.insert(session.into(), scope);
                Ok(())
            })
            .await??;
            let context = self
                .worker_request(
                    session,
                    "messages.history",
                    json!({"channel_id":channel,"limit":50,"latest":true}),
                )
                .await?;
            Ok(RunAdmission {
                run_id: run_id.clone(),
                institution_ids: institution_ids.clone(),
                expires_at,
                context: serde_json::to_string(&json!({
                    "connection_id": id,
                    "destination_channel_id": channel,
                    "source_channel_ids": sources,
                    "labels": labels,
                    "naming": "labels gives the names of the IDs above as the person saw them when granting access. Refer to people as Display name (@username) and to channels as #name. Never quote IDs to people.",
                    "context_discovery": "The included history covers only the destination channel, not all selected context. Call context.manifest with empty params for recent authorized selected-channel context (up to 200 messages). For more targeted evidence, call messages.search with channel_id and query for each relevant source_channel_id. Do not assume this initial history contains the answer.",
                    "history_channel_id": channel,
                    "remote_files_enabled": !public && c.remote_root.is_some(),
                    "remote_path_base": "the granted SSH work directory; use relative paths such as crew-task.csv, never the local task working directory",
                    "history": context
                }))?,
                labels: labels.clone(),
            })
        }
        .await;
        match admitted {
            Ok(admitted) => Ok(admitted),
            Err(error) => {
                self.abandon_run(session, id, &run_id, credential_written)
                    .await;
                Err(error)
            }
        }
    }
    /// Take back a run the workspace created for a grant that then failed to be set up (D7),
    /// best effort: the caller answers with the failure that brought it here, never this.
    ///
    /// ⚠ **Grant and revocation state; needs human review.** Before this, a grant whose
    /// setup failed after `run.create` (a broker answer that did not match, a credential or
    /// registry that could not be saved, the first `messages.history`) left the run live at the
    /// workspace until it lapsed, and — once recorded — left the grant active here.
    ///
    /// - The grant was recorded: revoke it as the person would, which expires it here (saved,
    ///   or held in memory when it cannot be) and then asks the workspace. It stays listed,
    ///   expired, so the person can see and retry the revocation; `grant_session` shares this
    ///   path, so a failed grant is left expired, never active.
    /// - It was not: ask the workspace to revoke the run, and delete the run credential this
    ///   setup wrote, since no grant here names it.
    ///
    /// When the local revoke fails before it asked the workspace (the grant is not this chat's
    /// any more, or the stop could not be saved), the workspace is asked directly.
    async fn abandon_run(
        &self,
        session: &str,
        connection_id: &str,
        run_id: &str,
        credential_written: bool,
    ) {
        let recorded = self
            .registry
            .lock()
            .await
            .scopes
            .get(session)
            .is_some_and(|scope| scope.run_id == run_id);
        if recorded {
            match self.revoke_session_if_current(session, run_id).await {
                Ok(outcome) => {
                    if let Some(error) = outcome.remote_error {
                        tracing::warn!(
                            session,
                            run_id,
                            %error,
                            "stopped a Crew grant whose setup failed; the workspace did not \
                             confirm the revocation"
                        );
                    }
                    return;
                }
                Err(error) => tracing::warn!(
                    session,
                    run_id,
                    %error,
                    "could not stop a Crew grant whose setup failed on this device; asking the \
                     workspace to revoke its run"
                ),
            }
        } else if credential_written {
            if let Err(error) = self.delete_credential(&format!("run:{session}")) {
                tracing::warn!(
                    session,
                    %error,
                    "could not delete the credential of a Crew run whose setup failed"
                );
            }
        }
        if let Err(error) = self
            .human_request(
                connection_id,
                "run.revoke",
                json!({ "run_id": run_id }),
                None,
            )
            .await
        {
            tracing::warn!(
                session,
                run_id,
                %error,
                "could not revoke a Crew run whose setup failed; it lapses when it expires"
            );
        }
    }
    /// A re-grant keeps every restriction the chat's current grant carries: its boundary,
    /// its private origin, its institutions and its sources. Only the chat's own grant (or
    /// one it cannot be confirmed not to hold) counts; a grant made to an earlier chat under
    /// the same id is pruned instead, and never constrains this one (SCOPE-BIND).
    async fn carry_previous_grant(
        &self,
        session: &str,
        id: &str,
        channel: &str,
        sources: &mut Vec<String>,
        provider: &dyn Provider,
        policy: &mut RunPolicy,
    ) -> Result<()> {
        let (Standing::Own(previous) | Standing::Unconfirmed(previous, _)) =
            self.standing(session).await
        else {
            return Ok(());
        };
        policy.origin_restricted |= previous.origin_restricted;
        policy
            .origin_institution_ids
            .extend(previous.institution_ids);
        ensure!(previous.connection_id == id && previous.channel_id == channel && previous.provider_binding == provider_binding(provider), "An existing Crew conversation retains its original connection, destination and model boundary; start a fresh conversation for another boundary");
        ensure!(
            provider.tier() != ProviderTier::Public || previous.public_provider,
            "Private-origin Crew conversation cannot be rebound to a public model"
        );
        for source in previous.source_channels {
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn grant_session(
        &self,
        session: &str,
        id: &str,
        channel: &str,
        sources: Vec<String>,
        provider: &dyn Provider,
        origin_restricted: bool,
    ) -> Result<RunAdmission> {
        self.begin_run_with_policy(
            session,
            id,
            channel,
            sources,
            provider,
            RunPolicy {
                origin_restricted,
                expected_mode: None,
                ..RunPolicy::default()
            },
        )
        .await
    }
    pub async fn worker_request(
        &self,
        session: &str,
        method: &str,
        mut params: Value,
    ) -> Result<Value> {
        ensure!(
            [
                "messages.history",
                "messages.search",
                "context.manifest",
                "run.project",
                "blob.read",
                "blob.begin",
                "blob.chunk",
                "blob.finish",
                "remote.list",
                "remote.read",
                "remote.write",
                "remote.hash",
                "remote.execute",
                "remote.job_status",
                "remote.cancel"
            ]
            .contains(&method),
            "Operation unavailable to a scoped Crew worker"
        );
        let s = self.scope(session).await?;
        ensure!(
            !method.starts_with("remote.") || !s.public_provider,
            "Public models cannot access remote files/jobs"
        );
        ensure!(!s.expired, GRANT_REVOKED);
        let c = self.connection(&s.connection_id).await?;
        ensure!(s.epoch == c.policy_epoch, GRANT_POLICY_CHANGED);
        ensure!(params.is_object(), "Crew params must be an object");
        if let Some(channel) = params.get("channel_id").and_then(Value::as_str) {
            ensure!(
                s.source_channels.iter().any(|v| v == channel),
                "Channel is outside the approved context scope"
            );
        }
        if method == "run.project" {
            params["run_id"] = json!(s.run_id);
            params["channel_id"] = json!(s.channel_id);
            if params.get("idempotency_key").is_none() {
                params["idempotency_key"] = json!(uuid::Uuid::new_v4().to_string());
            }
        }
        let transport = self.transport(&s.connection_id).await?;
        let mut locked = transport.lock().await;
        self.validate_worker_scope(session, &s, &c).await?;
        let credential = self.read_credential(&format!("run:{session}"))?;
        let result = locked
            .request(method, params, None, Some(&credential), None)
            .await;
        let usable = locked.is_usable();
        drop(locked);
        if !usable {
            self.retire_failed_transport(&s.connection_id, &transport)
                .await?;
        }
        let result = result?;
        self.validate_worker_scope(session, &s, &c).await?;
        Ok(result)
    }
    async fn validate_worker_scope(
        &self,
        session: &str,
        expected_scope: &Scope,
        expected_connection: &Connection,
    ) -> Result<()> {
        let registry = self.registry.lock().await;
        let scope = registry.scopes.get(session).ok_or_else(|| {
            anyhow::anyhow!(
                "This chat's Crew access is no longer available. Grant access again from Crew."
            )
        })?;
        let connection = registry
            .connections
            .iter()
            .find(|connection| connection.id == scope.connection_id)
            .ok_or_else(|| anyhow::anyhow!("Crew connection was removed"))?;
        ensure!(
            scope == expected_scope
                && !scope.expired
                && scope.institution_policy
                && scope.epoch == connection.policy_epoch
                && connection.policy_epoch == expected_connection.policy_epoch
                && connection.mode == expected_connection.mode
                && connection.institution_id == expected_connection.institution_id
                && connection.workspace_id == expected_connection.workspace_id
                && connection.workspace_public_key == expected_connection.workspace_public_key
                && (!scope.public_provider
                    || (connection.mode == ClusterMode::Public && !scope.origin_restricted)),
            ACCESS_CHANGED
        );
        Ok(())
    }
    pub async fn publish_run(&self, session: &str, body: &str, status: &str) -> Result<Value> {
        self.worker_request(session, "run.project", json!({"body":body,"status":status}))
            .await
    }
    /// Revoke the session's grant while it still holds `expected_run_id`; a newer grant is
    /// refused, not stopped. The grant stops here first (RV-D1): an `Err` means it could not be
    /// found, was replaced, or the stop could not be saved; an `Ok` means it is stopped on this
    /// device, whatever the workspace said.
    pub async fn revoke_session_if_current(
        &self,
        session: &str,
        expected_run_id: &str,
    ) -> Result<RevokeOutcome> {
        self.revoke_scope(session, Some(expected_run_id)).await
    }
    /// [`Self::revoke_session_if_current`] for whichever run the session holds now.
    pub async fn revoke_session(&self, session: &str) -> Result<RevokeOutcome> {
        self.revoke_scope(session, None).await
    }
    /// Revoke, treating anything short of the workspace's confirmation as an error. The grant
    /// is stopped on this device either way; an unconfirmed stop is a [`RevocationUnconfirmed`].
    pub async fn cancel_run_if_current(
        &self,
        session: &str,
        expected_run_id: &str,
    ) -> Result<Value> {
        self.revoke_session_if_current(session, expected_run_id)
            .await?
            .into_confirmed()
    }
    pub async fn cancel_run(&self, session: &str) -> Result<Value> {
        self.revoke_session(session).await?.into_confirmed()
    }
    async fn revoke_scope(
        &self,
        session: &str,
        expected_run_id: Option<&str>,
    ) -> Result<RevokeOutcome> {
        // A grant made to an earlier chat under this id is not this chat's to revoke; the
        // lookup prunes it (SCOPE-BIND). One that cannot be confirmed still can be: revoking
        // only ever takes authority away.
        ensure!(
            !matches!(self.standing(session).await, Standing::None),
            NO_GRANT
        );
        // Fail closed here before asking the workspace: a transport that is down, or sign-in
        // that lapsed, must not leave the grant usable on this device. If the save fails the
        // flag still stands in memory, which stops this process, and the error says the stop
        // is not yet durable. The saved registry is edited as it is now (D8), so this never
        // writes back a grant another process changed, and no later write here revives it.
        let (connection_id, run_id) = self
            .update_registry_keeping(|r| {
                let current = r
                    .scopes
                    .get_mut(session)
                    .ok_or_else(|| anyhow::anyhow!(NO_GRANT))?;
                ensure!(
                    expected_run_id.is_none_or(|expected| current.run_id == expected),
                    REPLACED_RUN
                );
                current.expired = true;
                Ok((current.connection_id.clone(), current.run_id.clone()))
            })
            .await
            .map_err(|error| {
                error.context("Couldn't save the revocation on this device; retry to finish it")
            })??;
        Ok(
            match self
                .human_request(&connection_id, "run.revoke", json!({"run_id":run_id}), None)
                .await
            {
                Ok(run) => RevokeOutcome {
                    remote_confirmed: true,
                    run: Some(run),
                    remote_error: None,
                },
                Err(error) => RevokeOutcome {
                    remote_confirmed: false,
                    run: None,
                    remote_error: Some(error),
                },
            },
        )
    }
    async fn attach_remote(&self, session: &str, params: Value) -> Result<Value> {
        let scope = self.scope(session).await?;
        ensure!(
            !scope.public_provider,
            "Public models cannot attach remote files"
        );
        let key = params["idempotency_key"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("idempotency_key is required for remote.attach"))?;
        let path = params["path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("path is required"))?;
        let file = self
            .worker_request(session, "remote.read", json!({"path":path}))
            .await?;
        let name = Path::new(path)
            .file_name()
            .and_then(|p| p.to_str())
            .ok_or_else(|| anyhow::anyhow!("attachment filename unavailable"))?;
        let blob=self.worker_request(session,"blob.begin",json!({"channel_id":scope.channel_id,"name":name,"media_type":params.get("media_type").and_then(Value::as_str).unwrap_or("application/octet-stream"),"size":file["size"],"sha256":file["sha256"],"personal_mode":"private","idempotency_key":format!("{key}:begin")})).await?;
        let blob_id = blob["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("attachment ID missing"))?;
        if file["size"].as_u64().unwrap_or(0) > 0 {
            self.worker_request(session,"blob.chunk",json!({"blob_id":blob_id,"offset":0,"data_hex":file["data_hex"],"idempotency_key":format!("{key}:chunk")})).await?;
        }
        self.worker_request(
            session,
            "blob.finish",
            json!({"blob_id":blob_id,"idempotency_key":format!("{key}:finish")}),
        )
        .await?;
        self.worker_request(session,"run.project",json!({"body":format!("Attached {name}"),"status":"progress","attachments":[blob_id],"idempotency_key":format!("{key}:post")})).await
    }
    pub async fn agent_request(
        &self,
        session: &str,
        cap: &CallCapability,
        id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        self.check_dispatch(session, cap).await?;
        let s = self.scope(session).await?;
        ensure!(
            s.connection_id == id,
            "Connection is outside the approved run scope"
        );
        if method == "run.project" {
            ensure!(
                params.get("status").and_then(Value::as_str).is_none_or(|status| status == "progress"),
                "Agent updates must use progress; the task owner or runner controls completion and cancellation"
            );
            let mut params = params;
            ensure!(params.is_object(), "Crew params must be an object");
            params["status"] = json!("progress");
            return self.worker_request(session, method, params).await;
        }
        if method == "remote.attach" {
            return self.attach_remote(session, params).await;
        }
        self.worker_request(session, method, params).await
    }
}

/// SCOPE-BIND: a grant belongs to one chat, not to a session id.
///
/// ⚠ **Security-relevant; needs human review.** Grants are stored by session id, and an id
/// is not one chat. `biorouter run --no-session` minted `<date>_1` in a private store and
/// inherited the desktop's first chat of the day's expired grant; a restored backup, a reset
/// database or an older build sharing the file can hand a granted chat's id to a new chat,
/// which then either lost its tools to someone else's grant or — with the grant live —
/// acted under a grant nobody gave it. So each grant records the incarnation of the chat it
/// was made to (a random token minted with the session row, never reused under the id), and
/// every read of a grant asks [`CrewManager::standing`] whether the chat holding the id now
/// is that chat. Ephemeral stores mint ids no saved chat can hold
/// ([`crate::session::SessionManager::new_ephemeral`]), and deleting a chat binds its grant
/// to it for good ([`CrewManager::retire_deleted_sessions`]).
///
/// The directions are deliberate. A grant made to an earlier chat under the id restricts
/// nothing and is pruned. A grant whose chat cannot be confirmed — gone from this device, or
/// its identity unreadable — keeps every restriction and authorizes nothing, because the
/// chat's history may hold Crew context and a lookup failure must never lift a restriction.
///
/// ⚠ **Deleting a chat keeps its grant.** It is the one record of a run the workspace still
/// honors: dropping it left nothing to revoke — the revoke route answered "not found" and a
/// deleted task's cancel failed on every retry until the run lapsed — and it lifted the
/// restriction from the chat's turn while that turn was still unwinding, since a delete only
/// signals the cancel. A deleted chat's grant resolves to [`Standing::Unconfirmed`]: still
/// restricting, still listed and revocable, never acting. It goes only when a later chat
/// under the id shows it was an earlier chat's.
impl CrewManager {
    /// Resolve chats against `store` in place of the shared one, for a test.
    #[cfg(test)]
    pub(crate) fn use_session_store(&self, store: Arc<crate::session::SessionManager>) {
        *self
            .session_store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(store);
    }

    #[cfg(test)]
    fn test_session_store(&self) -> Option<Arc<crate::session::SessionManager>> {
        self.session_store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The incarnation of the chat holding `session` in the store grants are made in: the
    /// process's shared store, where the daemon's chats live.
    async fn chat_incarnation(&self, session: &str) -> Result<Option<i64>> {
        #[cfg(test)]
        if let Some(store) = self.test_session_store() {
            return store.session_incarnation(session).await;
        }
        crate::session::SessionManager::instance()
            .session_incarnation(session)
            .await
    }

    /// The directory of the store grants name, when this process has opened it; `None`
    /// otherwise, which no deleting store can match.
    fn own_store_dir(&self) -> Option<PathBuf> {
        #[cfg(test)]
        if let Some(store) = self.test_session_store() {
            return Some(store.storage().session_dir().to_path_buf());
        }
        crate::session::SessionManager::shared_store_root_if_resolved()
            .map(|root| root.join(crate::session::session_manager::SESSIONS_FOLDER))
    }

    /// Whether the grant stored under `session` is the grant of the chat holding that id
    /// now:
    ///
    /// - bound to that chat's incarnation: its own;
    /// - bound to another incarnation: made to an earlier chat under the id, so none — and
    ///   pruned;
    /// - bound, with no chat under the id: [`Standing::Unconfirmed`], since the chat it was
    ///   made to is gone (a turn still unwinding may yet hold its context);
    /// - the chat's identity unreadable: [`Standing::Unconfirmed`];
    /// - recorded before grants were bound: its own, as it always was, and bound in memory
    ///   to the chat holding the id when there is one.
    ///
    /// Boxed for the reason [`Self::check_provider_dispatch`] is: every tool gate and
    /// `Agent::provider` reach this, and its store read and pruning write must not be laid
    /// out in their callers' frames.
    async fn standing(&self, session: &str) -> Standing {
        Box::pin(self.standing_inner(session)).await
    }
    async fn standing_inner(&self, session: &str) -> Standing {
        let Some(scope) = self.registry.lock().await.scopes.get(session).cloned() else {
            return Standing::None;
        };
        let current = match self.chat_incarnation(session).await {
            Ok(current) => current,
            Err(error) => {
                tracing::warn!(
                    session,
                    %error,
                    "could not confirm which chat holds a Crew grant; keeping it restricted"
                );
                return Standing::Unconfirmed(scope, GRANT_UNCONFIRMED);
            }
        };
        match (scope.session_incarnation, current) {
            (Some(bound), Some(current)) if bound == current => Standing::Own(scope),
            (Some(_), Some(_)) => {
                self.prune_stale_grant(session, &scope).await;
                Standing::None
            }
            (Some(_), None) => Standing::Unconfirmed(scope, GRANT_GONE),
            (None, Some(current)) => {
                Standing::Own(self.adopt_binding(session, scope, current).await)
            }
            (None, None) => Standing::Own(scope),
        }
    }

    /// Bind a grant recorded before grants were bound to the chat holding its id — the chat
    /// it was made to, since the store mints an id once — so a later chat under the id can
    /// never inherit it. In memory here: every process derives the same binding from the
    /// same row. This process's next registry update carries it into the saved registry
    /// ([`carry_process_state`]), which — reading the file back first (D8) — can no longer
    /// overwrite a grant another process saved since this one loaded.
    async fn adopt_binding(&self, session: &str, legacy: Scope, current: i64) -> Scope {
        let mut registry = self.registry.lock().await;
        match registry.scopes.get_mut(session) {
            Some(scope) if scope.run_id == legacy.run_id => {
                scope.session_incarnation.get_or_insert(current);
                scope.clone()
            }
            _ => Scope {
                session_incarnation: Some(current),
                ..legacy
            },
        }
    }

    /// Drop a grant made to an earlier chat under `session`'s id, from memory and from the
    /// saved registry. Matched by its run, which the workspace mints once per grant, so a
    /// newer grant under the same id — made here or saved by another process — never goes
    /// with it.
    async fn prune_stale_grant(&self, session: &str, stale: &Scope) {
        let is_stale = |id: &str, scope: &Scope| id == session && scope.run_id == stale.run_id;
        let pruned = self
            .update_registry_keeping(|registry| {
                registry.scopes.retain(|id, scope| !is_stale(id, scope));
                Ok(())
            })
            .await;
        if let Err(error) = pruned {
            tracing::warn!(
                session,
                %error,
                "could not remove a Crew grant made to an earlier chat under this id from the \
                 saved registry; it stays inert there"
            );
        }
        tracing::info!(
            session,
            run_id = %stale.run_id,
            "ignored a Crew grant that was made to an earlier chat under this id"
        );
    }

    /// The identity a new grant is bound to: the incarnation of the chat holding `session`
    /// in the store grants name. A chat that is not saved there cannot be granted.
    async fn grantable_chat(&self, session: &str) -> Result<i64> {
        match self.chat_incarnation(session).await {
            Ok(Some(incarnation)) => Ok(incarnation),
            Ok(None) => Err(anyhow::anyhow!(UNSAVED_CHAT)),
            Err(error) => Err(error.context(GRANT_UNCONFIRMED)),
        }
    }

    /// Retire the grants of chats just deleted from the store at `store_dir`, each named
    /// with the incarnation its row carried: every one is **kept**, bound to the chat it was
    /// made to.
    ///
    /// ⚠ **Security-relevant; needs human review.** Keeping is the point. The workspace still
    /// honors the run, so the grant must stay listed and revocable — a deleted task's cancel
    /// revokes through it — and a delete only signals the chat's turn to stop, so the turn
    /// may still be dispatching tool calls with Crew context in hand. With no chat under its
    /// id, a bound grant resolves to [`Standing::Unconfirmed`]: it restricts that turn and
    /// authorizes nothing. It is pruned only once a later chat holds the id
    /// ([`Self::standing`]), and `expired` is left alone: that flag says the grant was
    /// stopped here, which is what the access list shows and what hides its Revoke control,
    /// and deleting a chat stops nothing at the workspace.
    ///
    /// So a grant already bound to its chat needs nothing. What this does is bind a grant
    /// recorded before grants were bound, which would otherwise read as its chat's own with
    /// no chat under the id — acting for the unwinding turn in any process that had not yet
    /// bound it in memory, and handed to whichever chat next held the id. Such a grant is
    /// the deleted chat's only when it sits under the id in the very store grants name; a
    /// grant under one of these ids bound to another incarnation, or recorded against
    /// another store, belongs to a chat elsewhere and is not touched. One update of the saved
    /// registry as it is now ([`Self::update_registry_keeping`]), so a grant another process
    /// saved since this one loaded is never written away, and is bound too when it is the
    /// deleted chat's; memory takes the same edit even if the save fails.
    pub(crate) async fn retire_deleted_sessions(
        &self,
        deleted: &[(String, i64)],
        store_dir: &Path,
    ) -> Result<()> {
        let from_own_store = self.own_store_dir().is_some_and(|own| own == store_dir);
        let deleted: HashMap<&str, i64> = deleted
            .iter()
            .map(|(session, incarnation)| (session.as_str(), *incarnation))
            .collect();
        // The deleted chat's incarnation, when the grant under `session` was made to it.
        let deleted_chat = |session: &str, scope: &Scope| {
            deleted
                .get(session)
                .copied()
                .filter(|&incarnation| match scope.session_incarnation {
                    Some(bound) => bound == incarnation,
                    None => from_own_store,
                })
        };
        {
            let registry = self.registry.lock().await;
            let mut kept = false;
            for (session, scope) in &registry.scopes {
                if deleted_chat(session, scope).is_some() {
                    kept = true;
                    tracing::info!(
                        session,
                        run_id = %scope.run_id,
                        "kept the Crew grant of a deleted chat: it still restricts that chat and \
                         can be revoked, and it authorizes nothing"
                    );
                }
            }
            // Every chat delete lands here. With no grant in memory and no saved registry
            // there is nothing to retire, and no reason to create Crew's directory (and its
            // lock) for a profile that never used Crew.
            if !kept && !self.root.join("connections.json").exists() {
                return Ok(());
            }
        }
        // Bind it there for good, in memory and in the saved registry read back as it is now.
        self.update_registry_keeping(|registry| {
            for (session, scope) in registry.scopes.iter_mut() {
                if let Some(incarnation) = deleted_chat(session, scope) {
                    scope.session_incarnation = Some(incarnation);
                }
            }
            Ok(())
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agents::{
            crew_extension::CrewClient,
            extension::PlatformExtensionContext,
            mcp_client::{McpClientTrait, McpMeta},
            ExtensionManager,
        },
        privacy::{CallCapability, ProviderTier},
        session::SessionManager,
    };
    use rmcp::model::CallToolResult;
    #[cfg(unix)]
    use std::time::Duration;
    use std::{
        fs,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };
    use tokio_util::sync::CancellationToken;

    /// A chat saved in this process's shared store: the only kind a grant can be made to
    /// (SCOPE-BIND). Call it only in a process of the test's own.
    async fn saved_chat(root: &Path) -> String {
        SessionManager::instance()
            .create_session(
                root.to_path_buf(),
                "granted chat".into(),
                crate::session::session_manager::SessionType::User,
            )
            .await
            .unwrap()
            .id
    }

    fn fixture_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "biorouter-crew-manager-{label}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[tokio::test]
    async fn message_and_blob_mode_mismatches_refuse_before_credentials_or_transport() {
        let root = fixture_root("mode-guard");
        let manager = CrewManager::new(root.clone()).unwrap();
        let connection_id = "mode-guard-connection";
        manager.registry.lock().await.connections.push(Connection {
            id: connection_id.into(),
            node_id: None,
            name: "mode guard fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "mode-guard-workspace".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "mode-guard-cluster".into(),
            mode: ClusterMode::Public,
            institution_id: None,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        });

        for method in ["message.post", "blob.begin"] {
            let mismatch = manager
                .human_request(
                    connection_id,
                    method,
                    json!({"personal_mode":"private"}),
                    None,
                )
                .await
                .expect_err("a stale private mode must be refused before I/O");
            assert_eq!(
                mismatch.to_string(),
                "Crew connection privacy changed; refresh the verified workspace before sending"
            );

            let missing = manager
                .human_request(connection_id, method, json!({}), None)
                .await
                .expect_err("the fixture intentionally has no device credential");
            assert!(
                !missing.to_string().contains("privacy changed"),
                "omitted mode should remain backward-compatible: {missing}"
            );

            let matching = manager
                .human_request(
                    connection_id,
                    method,
                    json!({"personal_mode":"public"}),
                    None,
                )
                .await
                .expect_err("matching mode reaches the credential boundary in this fixture");
            assert!(
                !matching.to_string().contains("privacy changed"),
                "matching mode was rejected by the privacy guard: {matching}"
            );
        }

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn run_admission_mode_policy_refuses_before_transport_and_preserves_private_origin() {
        let root = fixture_root("run-mode-policy");
        let manager = CrewManager::new(root.clone()).unwrap();
        let connection_id = "run-mode-policy-connection";
        manager.registry.lock().await.connections.push(Connection {
            id: connection_id.into(),
            node_id: None,
            name: "run mode policy fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "run-mode-policy-workspace".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "run-mode-policy-cluster".into(),
            mode: ClusterMode::Public,
            institution_id: None,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        });
        let provider = crate::providers::testprovider::TestProvider::new_replaying(
            root.join("missing-cassette.json")
                .to_string_lossy()
                .into_owned(),
        )
        .unwrap();

        let private_connection_id = "run-mode-policy-private-connection";
        let mut private_connection = manager.registry.lock().await.connections[0].clone();
        private_connection.id = private_connection_id.into();
        private_connection.mode = ClusterMode::Private;
        manager
            .registry
            .lock()
            .await
            .connections
            .push(private_connection);
        let private_public = match manager
            .begin_run_with_policy(
                "run-mode-policy-private-cluster",
                private_connection_id,
                "destination-channel",
                vec![],
                &provider,
                RunPolicy::default(),
            )
            .await
        {
            Err(error) => error,
            Ok(_) => panic!(
                "a public provider must be refused by a private cluster before manager reservation"
            ),
        };
        assert_eq!(
            private_public.to_string(),
            "Private cluster blocks public models"
        );
        assert!(!manager
            .registry
            .lock()
            .await
            .scopes
            .contains_key("run-mode-policy-private-cluster"));

        let mismatch = manager
            .begin_run_with_policy(
                "run-mode-policy-session",
                connection_id,
                "destination-channel",
                vec![],
                &provider,
                RunPolicy {
                    origin_restricted: false,
                    expected_mode: Some(ClusterMode::Private),
                    ..RunPolicy::default()
                },
            )
            .await
            .err()
            .expect("a stale private mode must stop admission before signing");
        assert_eq!(
            mismatch.to_string(),
            "Crew connection privacy changed; refresh the verified workspace before granting agent access"
        );

        let legacy = manager
            .begin_run_with_policy(
                "run-mode-policy-legacy-session",
                connection_id,
                "destination-channel",
                vec![],
                &provider,
                RunPolicy::default(),
            )
            .await
            .err()
            .expect("the fixture intentionally has no device credential");
        assert!(
            !legacy.to_string().contains("privacy changed"),
            "missing expected_mode must preserve the legacy path: {legacy}"
        );

        let private_origin = manager
            .begin_run_with_policy(
                "run-mode-policy-private-origin",
                connection_id,
                "destination-channel",
                vec![],
                &provider,
                RunPolicy {
                    origin_restricted: true,
                    expected_mode: Some(ClusterMode::Public),
                    ..RunPolicy::default()
                },
            )
            .await
            .err()
            .expect("a private-origin run must not be admitted to a public provider");
        assert_eq!(
            private_origin.to_string(),
            "Private-origin local conversation cannot be admitted to a public Crew worker"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn authoritative_crew_context_omits_local_moim_context() -> anyhow::Result<()> {
        let working_dir = tempfile::tempdir()?;
        let canary = format!("crew-local-context-canary-{}", uuid::Uuid::new_v4());
        fs::write(working_dir.path().join("AGENTS.md"), &canary)?;
        let canary_file = format!("{canary}.txt");
        fs::write(working_dir.path().join(&canary_file), b"local-only")?;
        let path_text = working_dir.path().display().to_string();
        let session_id = format!("crew-context-{canary}");
        let manager = crate::crew::install_test_scope(&session_id, None).await;

        let extension_manager = ExtensionManager::new_without_provider(working_dir.path().into());
        let moim = extension_manager
            .collect_moim(&session_id, working_dir.path(), None)
            .await
            .expect("Crew scope should still provide its remote guidance");
        assert!(moim.contains("Crew scope: remote file paths"), "{moim}");
        assert!(
            !moim.contains(&path_text),
            "Crew MOIM leaked local path: {moim}"
        );
        assert!(
            !moim.contains(&canary),
            "Crew MOIM leaked local workspace context: {moim}"
        );

        crate::crew::remove_test_scope(&manager, &session_id).await;
        Ok(())
    }

    #[tokio::test]
    async fn ordinary_context_retains_local_moim_and_prompt_context() -> anyhow::Result<()> {
        let working_dir = tempfile::tempdir()?;
        let canary = format!("ordinary-local-context-canary-{}", uuid::Uuid::new_v4());
        fs::write(working_dir.path().join("AGENTS.md"), &canary)?;
        let canary_file = format!("{canary}.txt");
        fs::write(working_dir.path().join(&canary_file), b"local-only")?;
        let path_text = working_dir.path().display().to_string();
        let session_id = format!("ordinary-context-{canary}");

        let extension_manager = ExtensionManager::new_without_provider(working_dir.path().into());
        let moim = extension_manager
            .collect_moim(&session_id, working_dir.path(), None)
            .await
            .expect("ordinary sessions should receive local context");
        assert!(
            moim.contains(&path_text) && moim.contains(&canary_file),
            "ordinary MOIM lost local context: {moim}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn agent_connections_exposes_only_live_authorized_context_channels() -> anyhow::Result<()>
    {
        let root = fixture_root("agent-connections-context-scope");
        let connection = Connection {
            id: "authorized-connection".into(),
            node_id: None,
            name: "authorized".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "workspace-authorized".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "cluster-authorized".into(),
            mode: ClusterMode::Public,
            institution_id: None,
            policy_epoch: 7,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        };
        let mut other = connection.clone();
        other.id = "other-connection".into();
        other.name = "other".into();
        let scope = Scope {
            connection_id: connection.id.clone(),
            run_id: "run-authorized".into(),
            channel_id: "destination-channel".into(),
            source_channels: vec![
                "source-a".into(),
                "source-b".into(),
                "destination-channel".into(),
            ],
            epoch: connection.policy_epoch,
            provider_binding: "public-test-provider".into(),
            public_provider: true,
            origin_restricted: false,
            institution_ids: BTreeSet::new(),
            institution_policy: true,
            expired: false,
            expires_at: None,
            labels: None,
            session_incarnation: None,
        };
        let manager = CrewManager::new(root.clone())?;
        {
            let mut registry = manager.registry.lock().await;
            registry.connections = vec![connection.clone(), other];
            registry.scopes.insert("live-session".into(), scope.clone());
        }

        let discovery = manager.agent_connections("live-session").await?;
        let entry = &discovery["connections"][0];
        assert_eq!(entry["id"], "authorized-connection");
        assert_eq!(entry["destination_channel_id"], "destination-channel");
        assert_eq!(
            entry["source_channel_ids"],
            json!(["source-a", "source-b", "destination-channel"])
        );
        assert!(entry["context_discovery"]
            .as_str()
            .is_some_and(|guidance| guidance.contains("messages.search")));
        let serialized = serde_json::to_string(&discovery)?;
        assert!(!serialized.contains("other-connection"));

        manager
            .registry
            .lock()
            .await
            .scopes
            .get_mut("live-session")
            .unwrap()
            .expired = true;
        let expired = manager.agent_connections("live-session").await.unwrap_err();
        assert!(expired
            .to_string()
            .contains("changed while this was in progress"));

        manager
            .registry
            .lock()
            .await
            .scopes
            .get_mut("live-session")
            .unwrap()
            .expired = false;
        manager.registry.lock().await.connections[0].policy_epoch += 1;
        let changed = manager.agent_connections("live-session").await.unwrap_err();
        assert!(changed
            .to_string()
            .contains("changed while this was in progress"));

        let missing = manager
            .agent_connections("missing-session")
            .await
            .unwrap_err();
        assert_eq!(missing.to_string(), NO_GRANT);

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn admitted_context_keeps_history_on_destination_and_lists_selected_sources(
    ) -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        if !crate::test_sandbox::in_a_process_of_its_own() {
            return Ok(());
        }
        let root = fixture_root("admission-context-discovery");
        let profile_root = root.join("profile");
        let fake_bin = root.join("bin");
        fs::create_dir_all(&fake_bin)?;
        let workspace_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let log = root.join("requests.log");
        let ssh = format!(
            r#"#!/bin/sh
log='{}'
if [ "$1" = "-G" ]; then
  printf '%s\n' \
    'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  if printf '%s\n' "$line" | grep -q 'auth.challenge'; then
    printf '{{"id":"%s","result":{{"workspace_id":"{workspace_id}","nonce":"nonce"}}}}\n' "$id"
  elif printf '%s\n' "$line" | grep -q 'run.create'; then
    printf '{{"id":"%s","result":{{"run":{{"id":"run-admitted","protected_context":false}},"credential":"run-credential"}}}}\n' "$id"
  elif printf '%s\n' "$line" | grep -q 'workspace.snapshot'; then
    printf '{{"id":"%s","result":{{"workspace":{{"mode":"public","institution_id":null,"policy_epoch":1}},"channels":[{{"id":"destination-channel"}},{{"id":"source-a"}},{{"id":"source-b"}}],"protected_channel_ids":[]}}}}\n' "$id"
  elif printf '%s\n' "$line" | grep -q 'messages.history'; then
    printf '{{"id":"%s","result":{{"messages":[{{"channel_id":"destination-channel","id":"destination-message","text":"destination-only"}}]}}}}\n' "$id"
  else
    printf '{{"id":"%s","result":{{"accepted_method":"fixture"}}}}\n' "$id"
  fi
done
"#,
            log.display()
        );
        let ssh_path = fake_bin.join("ssh");
        fs::write(&ssh_path, ssh)?;
        fs::set_permissions(&ssh_path, fs::Permissions::from_mode(0o700))?;
        let original_path = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{original_path}", fake_bin.display());
        let profile_string = profile_root.to_string_lossy().into_owned();
        let _env = crate::test_sandbox::relocate_path_root_and(
            profile_string.as_str(),
            [
                ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile_string.as_str())),
                ("BIOROUTER_DISABLE_KEYRING", Some("true")),
                ("PATH", Some(path.as_str())),
            ],
        );
        let device_key = SigningKey::from_bytes(&[7; 32]);
        let connection = Connection {
            id: "admission-connection".into(),
            node_id: None,
            name: "admission fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: workspace_id.into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "admission-cluster".into(),
            mode: ClusterMode::Public,
            institution_id: None,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: hex(&Sha256::digest(device_key.verifying_key().to_bytes())),
            public_key: hex(&device_key.verifying_key().to_bytes()),
        };
        let manager = CrewManager::new(root.join("manager"))?;
        manager
            .registry
            .lock()
            .await
            .connections
            .push(connection.clone());
        manager.write_credential(
            &format!("device:{}", connection.id),
            &hex(&device_key.to_bytes()),
        )?;
        let control = manager.control_path(&connection.id)?;
        let transport = transport::Transport::connect(&connection, &control).await?;
        manager
            .transports
            .lock()
            .await
            .insert(connection.id.clone(), Arc::new(Mutex::new(transport)));

        let provider = crate::providers::testprovider::TestProvider::new_replaying(
            root.join("provider-cassette.json").to_string_lossy(),
        )?;
        let session = saved_chat(&root).await;
        let admission = manager
            .begin_run(
                &session,
                &connection.id,
                "destination-channel",
                vec!["source-a".into(), "source-b".into()],
                &provider,
            )
            .await?;
        let context: Value = serde_json::from_str(&admission.context)?;
        assert_eq!(context["destination_channel_id"], "destination-channel");
        assert_eq!(context["history_channel_id"], "destination-channel");
        assert_eq!(
            context["source_channel_ids"],
            json!(["source-a", "source-b", "destination-channel"])
        );
        let history = context["history"]["messages"].as_array().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["channel_id"], "destination-channel");
        assert_eq!(history[0]["text"], "destination-only");
        let requests = fs::read_to_string(&log)?;
        let history_request = requests
            .lines()
            .find(|line| line.contains("messages.history"))
            .expect("admission should fetch destination history");
        assert!(history_request.contains("destination-channel"));
        assert!(!history_request.contains("source-a"));
        assert!(!history_request.contains("source-b"));

        let discovery = manager.agent_connections(&session).await?;
        assert_eq!(
            discovery["connections"][0]["source_channel_ids"],
            json!(["source-a", "source-b", "destination-channel"])
        );
        assert!(discovery["connections"][0]["context_discovery"]
            .as_str()
            .unwrap()
            .contains("messages.search"));

        manager.disconnect(&connection.id).await?;
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    fn worker_race_connection(
        connection_id: &str,
        mode: ClusterMode,
        policy_epoch: u64,
        public_provider: bool,
    ) -> (Connection, Scope) {
        let connection = Connection {
            id: connection_id.into(),
            node_id: None,
            name: "worker-race".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".into(),
            mode,
            institution_id: None,
            policy_epoch,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        };
        let scope = Scope {
            connection_id: connection_id.into(),
            run_id: "worker-race-run".into(),
            channel_id: "worker-race-channel".into(),
            source_channels: vec!["worker-race-channel".into()],
            epoch: policy_epoch,
            provider_binding: "worker-race-provider".into(),
            public_provider,
            origin_restricted: false,
            institution_ids: BTreeSet::new(),
            institution_policy: true,
            expired: false,
            expires_at: None,
            labels: None,
            session_incarnation: None,
        };
        (connection, scope)
    }

    #[cfg(unix)]
    fn write_worker_race_ssh(root: &Path, wait_for_release: bool) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let fake_bin = root.join("bin");
        fs::create_dir_all(&fake_bin).unwrap();
        let log = root.join("requests.log");
        let gate = root.join("response-held");
        let release = root.join("release-response");
        if wait_for_release {
            fs::write(&gate, b"hold").unwrap();
        }
        let ssh = format!(
            r#"#!/bin/sh
log='{}'
gate='{}'
release='{}'
if [ "$1" = "-G" ]; then
  printf '%s\n' \
    'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  while [ -f "$gate" ] && [ ! -f "$release" ]; do sleep 0.01; done
  printf '{{"id":"%s","result":{{"accepted_method":"messages.history"}}}}\n' "$id"
done
"#,
            log.display(),
            gate.display(),
            release.display(),
        );
        let ssh_path = fake_bin.join("ssh");
        fs::write(&ssh_path, ssh).unwrap();
        fs::set_permissions(&ssh_path, fs::Permissions::from_mode(0o700)).unwrap();
        (log, release)
    }

    #[cfg(unix)]
    async fn worker_race_manager(
        root: &Path,
        connection: &Connection,
        scope: &Scope,
    ) -> Arc<CrewManager> {
        let registry = Registry {
            connections: vec![connection.clone()],
            scopes: HashMap::from([("worker-race-session".into(), scope.clone())]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = Arc::new(CrewManager::new(root.to_owned()).unwrap());
        manager
            .write_credential("run:worker-race-session", "run-credential")
            .unwrap();
        let control = manager.control_path(&connection.id).unwrap();
        let transport = transport::Transport::connect(connection, &control)
            .await
            .unwrap();
        manager
            .transports
            .lock()
            .await
            .insert(connection.id.clone(), Arc::new(Mutex::new(transport)));
        manager
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worker_request_rechecks_policy_before_writing_transport() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("worker-race-before-write");
        let fake_bin = root.join("bin");
        let (log, _) = write_worker_race_ssh(&root, false);
        let profile_root = root.join("profile");
        fs::create_dir_all(&profile_root).unwrap();
        let original_path = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{original_path}", fake_bin.display());
        let profile_string = profile_root.to_string_lossy().into_owned();
        let _env = crate::test_sandbox::relocate_path_root_and(
            profile_string.as_str(),
            [
                ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile_string.as_str())),
                ("BIOROUTER_DISABLE_KEYRING", Some("true")),
                ("PATH", Some(path.as_str())),
            ],
        );
        let connection_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
        let (connection, scope) =
            worker_race_connection(connection_id, ClusterMode::Public, 1, true);
        let manager = worker_race_manager(&root, &connection, &scope).await;
        let transport = manager
            .transports
            .lock()
            .await
            .get(connection_id)
            .cloned()
            .unwrap();
        let control = manager
            .worker_request(
                "worker-race-session",
                "messages.history",
                json!({
                    "channel_id": "worker-race-channel",
                    "limit": 1,
                }),
            )
            .await
            .unwrap();
        assert_eq!(control["accepted_method"], "messages.history");
        let baseline_requests = fs::read_to_string(&log).unwrap_or_default();
        assert_eq!(baseline_requests.lines().count(), 1);
        let held = transport.lock().await;
        let manager_for_worker = manager.clone();
        let worker = tokio::spawn(async move {
            manager_for_worker
                .worker_request(
                    "worker-race-session",
                    "messages.history",
                    json!({
                        "channel_id": "worker-race-channel",
                        "limit": 1,
                    }),
                )
                .await
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while tokio::time::Instant::now() < deadline && Arc::strong_count(&transport) < 3 {
            tokio::task::yield_now().await;
        }
        assert!(Arc::strong_count(&transport) >= 3);
        assert_eq!(
            fs::read_to_string(&log).unwrap_or_default(),
            baseline_requests
        );
        {
            let mut registry = manager.registry.lock().await;
            let current = registry
                .connections
                .iter_mut()
                .find(|candidate| candidate.id == connection_id)
                .unwrap();
            current.mode = ClusterMode::Private;
            current.policy_epoch = 2;
        }
        drop(held);
        let error = worker.await.unwrap().unwrap_err().to_string();
        assert!(
            error.contains("settings changed while this was in progress"),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(&log).unwrap_or_default(),
            baseline_requests
        );
        let _ = manager.transports.lock().await.remove(connection_id);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worker_request_reports_possible_effects_after_inflight_policy_change() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("worker-race-inflight-response");
        let fake_bin = root.join("bin");
        let (log, release) = write_worker_race_ssh(&root, true);
        let profile_root = root.join("profile");
        fs::create_dir_all(&profile_root).unwrap();
        let original_path = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{original_path}", fake_bin.display());
        let profile_string = profile_root.to_string_lossy().into_owned();
        let _env = crate::test_sandbox::relocate_path_root_and(
            profile_string.as_str(),
            [
                ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile_string.as_str())),
                ("BIOROUTER_DISABLE_KEYRING", Some("true")),
                ("PATH", Some(path.as_str())),
            ],
        );
        let connection_id = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
        let (connection, scope) =
            worker_race_connection(connection_id, ClusterMode::Private, 1, false);
        let manager = worker_race_manager(&root, &connection, &scope).await;
        let manager_for_worker = manager.clone();
        let worker = tokio::spawn(async move {
            manager_for_worker
                .worker_request(
                    "worker-race-session",
                    "messages.history",
                    json!({
                        "channel_id": "worker-race-channel",
                        "limit": 1,
                    }),
                )
                .await
        });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while tokio::time::Instant::now() < deadline
            && fs::read_to_string(&log).unwrap_or_default().is_empty()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!fs::read_to_string(&log).unwrap_or_default().is_empty());
        {
            let mut registry = manager.registry.lock().await;
            let current = registry
                .connections
                .iter_mut()
                .find(|candidate| candidate.id == connection_id)
                .unwrap();
            current.policy_epoch = 2;
        }
        fs::write(&release, b"release").unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Check whether it already took effect before you grant access again"),
            "{error}"
        );
        let _ = manager.transports.lock().await.remove(connection_id);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn failed_policy_save_preserves_connection_and_scope_authority() {
        let root = fixture_root("failed-policy-save");
        let connection_id = "11111111-1111-4111-8111-111111111111".to_owned();
        let cluster_id = "22222222-2222-4222-8222-222222222222".to_owned();
        let workspace_id = "33333333-3333-4333-8333-333333333333".to_owned();
        let connection = Connection {
            id: connection_id.clone(),
            node_id: None,
            name: "fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: workspace_id.clone(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: cluster_id.clone(),
            mode: ClusterMode::Private,
            institution_id: None,
            policy_epoch: 7,
            status: "disconnected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        };
        let scope = Scope {
            connection_id: connection_id.clone(),
            run_id: "run-1".into(),
            channel_id: "channel-1".into(),
            source_channels: vec!["channel-1".into()],
            epoch: 7,
            provider_binding: "private".into(),
            public_provider: false,
            origin_restricted: false,
            institution_ids: BTreeSet::new(),
            institution_policy: true,
            expired: false,
            expires_at: None,
            labels: None,
            session_incarnation: None,
        };
        let registry = Registry {
            connections: vec![connection],
            scopes: HashMap::from([("session-1".into(), scope.clone())]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = CrewManager::new(root.clone()).unwrap();
        fs::remove_file(root.join("connections.json")).unwrap();
        fs::create_dir(root.join("connections.json")).unwrap();

        let input = SaveConnection {
            preparation_id: None,
            name: "fixture-public-alias".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id,
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: Some(cluster_id),
            mode: ClusterMode::Public,
            institution_id: None,
        };
        assert!(manager
            .save_inner(Some(&connection_id), input)
            .await
            .is_err());
        let live = manager.connection(&connection_id).await.unwrap();
        assert_eq!(live.mode, ClusterMode::Private);
        assert_eq!(live.policy_epoch, 7);
        let live_scope = manager
            .registry
            .lock()
            .await
            .scopes
            .get("session-1")
            .cloned()
            .unwrap();
        assert!(live_scope == scope);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn worker_scope_validation_rejects_alias_epoch_and_expiry_then_accepts_regrant() {
        let root = fixture_root("worker-scope-validation");
        let connection_id = "44444444-4444-4444-8444-444444444444".to_owned();
        let workspace_id = "55555555-5555-4555-8555-555555555555".to_owned();
        let connection = Connection {
            id: connection_id.clone(),
            node_id: None,
            name: "fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id,
            workspace_public_key: "44".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "66666666-6666-4666-8666-666666666666".into(),
            mode: ClusterMode::Public,
            institution_id: None,
            policy_epoch: 4,
            status: "connected".into(),
            last_error: None,
            device_id: "55".repeat(32),
            public_key: "66".repeat(32),
        };
        let scope = Scope {
            connection_id: connection_id.clone(),
            run_id: "run-public".into(),
            channel_id: "channel-public".into(),
            source_channels: vec!["channel-public".into()],
            epoch: 4,
            provider_binding: "public".into(),
            public_provider: true,
            origin_restricted: false,
            institution_ids: BTreeSet::new(),
            institution_policy: true,
            expired: false,
            expires_at: None,
            labels: None,
            session_incarnation: None,
        };
        let registry = Registry {
            connections: vec![connection.clone()],
            scopes: HashMap::from([("session-public".into(), scope.clone())]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = CrewManager::new(root.clone()).unwrap();
        assert!(manager
            .validate_worker_scope("session-public", &scope, &connection)
            .await
            .is_ok());

        let mut wrong_epoch = scope.clone();
        wrong_epoch.epoch = 3;
        assert!(manager
            .validate_worker_scope("session-public", &wrong_epoch, &connection)
            .await
            .is_err());
        let mut expired = scope.clone();
        expired.expired = true;
        assert!(manager
            .validate_worker_scope("session-public", &expired, &connection)
            .await
            .is_err());

        let mut aliased = connection.clone();
        aliased.mode = ClusterMode::Private;
        aliased.policy_epoch = 5;
        manager.registry.lock().await.connections[0] = aliased.clone();
        assert!(manager
            .validate_worker_scope("session-public", &scope, &connection)
            .await
            .is_err());

        let mut regranted = scope;
        regranted.run_id = "run-private-regrant".into();
        regranted.epoch = 5;
        regranted.public_provider = false;
        manager
            .registry
            .lock()
            .await
            .scopes
            .insert("session-public".into(), regranted.clone());
        assert!(manager
            .validate_worker_scope("session-public", &regranted, &aliased)
            .await
            .is_ok());
        let _ = fs::remove_dir_all(root);
    }

    fn result_text(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|content| content.as_text().map(|text| text.text.clone()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn crew_client(label: &str) -> CrewClient {
        CrewClient::new(PlatformExtensionContext {
            extension_manager: None,
            session_manager: Arc::new(SessionManager::new(fixture_root(label))),
        })
    }

    #[tokio::test]
    async fn crew_extension_rejects_invalid_explicit_connection_id_shapes() {
        let _root = crate::test_sandbox::pin_sandbox_path_root();
        let client = crew_client("invalid-connection-id-session");
        let meta = McpMeta::new(
            "invalid-connection-id-session",
            CallCapability::for_test(ProviderTier::Private, true),
        );

        for connection_id in [Value::Null, json!(7), json!({"id": "connection"})] {
            let result = client
                .call_tool(
                    "request",
                    Some(
                        json!({
                            "connection_id": connection_id,
                            "method": "remote.list"
                        })
                        .as_object()
                        .cloned()
                        .unwrap(),
                    ),
                    meta.clone(),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            assert!(
                result_text(&result).contains("connection_id must be a string"),
                "unexpected refusal: {}",
                result_text(&result)
            );
        }
    }

    #[tokio::test]
    async fn crew_extension_omitted_connection_id_refuses_without_a_grant() {
        let _root = crate::test_sandbox::pin_sandbox_path_root();
        let client = crew_client("omitted-connection-id-without-grant-session");
        let result = client
            .call_tool(
                "request",
                Some(
                    json!({"method": "remote.list"})
                        .as_object()
                        .cloned()
                        .unwrap(),
                ),
                McpMeta::new(
                    "omitted-connection-id-without-grant-session",
                    CallCapability::for_test(ProviderTier::Private, true),
                ),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(result_text(&result).contains("Request a Crew grant"));
    }

    #[tokio::test]
    async fn agent_request_rejects_an_explicit_connection_outside_the_run_scope() {
        let root = fixture_root("wrong-connection-id");
        let connection_id = "77777777-7777-4777-8777-777777777777".to_owned();
        let connection = Connection {
            id: connection_id.clone(),
            node_id: None,
            name: "fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "88888888-8888-4888-8888-888888888888".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "99999999-9999-4999-8999-999999999999".into(),
            mode: ClusterMode::Private,
            institution_id: None,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        };
        let scope = Scope {
            connection_id: connection_id.clone(),
            run_id: "run-1".into(),
            channel_id: "channel-1".into(),
            source_channels: vec!["channel-1".into()],
            epoch: 1,
            provider_binding: "private".into(),
            public_provider: false,
            origin_restricted: false,
            institution_ids: BTreeSet::new(),
            institution_policy: true,
            expired: false,
            expires_at: None,
            labels: None,
            session_incarnation: None,
        };
        let registry = Registry {
            connections: vec![connection],
            scopes: HashMap::from([("session-1".into(), scope)]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = CrewManager::new(root.clone()).unwrap();
        let error = manager
            .agent_request(
                "session-1",
                &CallCapability::for_test(ProviderTier::Private, true),
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                "remote.list",
                json!({}),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("outside the approved run scope"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn control_path_uses_short_system_tmp_even_when_tmpdir_is_long() {
        use std::os::unix::fs::MetadataExt;

        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("control-path-length");
        let long_tmpdir = root.join("a".repeat(160));
        fs::create_dir_all(&long_tmpdir).unwrap();
        let long_tmpdir_string = long_tmpdir.to_string_lossy().into_owned();
        let _env = crate::test_sandbox::relocate_path_root_and(
            crate::test_sandbox::sandbox_path_root(),
            [("TMPDIR", Some(long_tmpdir_string.as_str()))],
        );
        let manager = CrewManager::new(root.clone()).unwrap();
        let path = manager.control_path("length-check").unwrap();
        assert!(path.starts_with(std::fs::canonicalize("/tmp").unwrap()));
        assert!(path.as_os_str().len() <= 86);
        let metadata = fs::metadata(path.parent().unwrap()).unwrap();
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o077, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn control_path_rejects_a_symlink_collision() {
        use std::os::unix::fs::symlink;

        let root = fixture_root("control-path-symlink");
        let manager = CrewManager::new(root.clone()).unwrap();
        let path = manager.control_path("symlink-check").unwrap();
        symlink("/tmp/does-not-exist", &path).unwrap();
        let error = manager
            .control_path("symlink-check")
            .unwrap_err()
            .to_string();
        assert!(error.contains("owned socket"), "{error}");
        fs::remove_file(&path).unwrap();
        fs::remove_dir(path.parent().unwrap()).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn control_path_runtime_directory_is_private_and_owned() {
        use std::os::unix::fs::MetadataExt;

        let root = fixture_root("control-path-permissions");
        let manager = CrewManager::new(root.clone()).unwrap();
        let path = manager.control_path("permission-check").unwrap();
        let metadata = fs::metadata(path.parent().unwrap()).unwrap();
        assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
        assert_eq!(metadata.mode() & 0o077, 0);
        assert!(!fs::symlink_metadata(path.parent().unwrap())
            .unwrap()
            .file_type()
            .is_symlink());
        let _ = fs::remove_dir(path.parent().unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn crew_extension_omitted_connection_id_reaches_the_bound_request_target() {
        use std::os::unix::fs::PermissionsExt;

        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("omitted-connection-id-target");
        let profile_root = root.join("profile");
        let fake_bin = root.join("bin");
        fs::create_dir_all(&fake_bin).unwrap();
        let workspace_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let ssh = format!(
            r#"#!/bin/sh
if [ "$1" = "-G" ]; then
  printf '%s\n' \
    'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  if printf '%s' "$line" | grep -q 'auth.challenge'; then
    printf '{{"id":"%s","result":{{"workspace_id":"{workspace_id}","nonce":"nonce"}}}}\n' "$id"
  else
    printf '{{"id":"%s","result":{{"accepted_method":"remote.list"}}}}\n' "$id"
  fi
done
"#
        );
        let ssh_path = fake_bin.join("ssh");
        fs::write(&ssh_path, ssh).unwrap();
        fs::set_permissions(&ssh_path, fs::Permissions::from_mode(0o700)).unwrap();
        let original_path = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{original_path}", fake_bin.display());
        let profile_string = profile_root.to_string_lossy().into_owned();
        let _env = crate::test_sandbox::relocate_path_root_and(
            profile_string.as_str(),
            [
                ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile_string.as_str())),
                ("BIOROUTER_DISABLE_KEYRING", Some("true")),
                ("PATH", Some(path.as_str())),
            ],
        );
        let manager_root = crate::config::paths::Paths::config_dir().join("crew");
        fs::create_dir_all(&manager_root).unwrap();
        let connection_id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_owned();
        let device_key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let connection = Connection {
            id: connection_id.clone(),
            node_id: None,
            name: "fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: workspace_id.into(),
            workspace_public_key: "11".repeat(32),
            remote_root: Some("/srv/crew".into()),
            remote_execution: true,
            cluster_connection_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc".into(),
            mode: ClusterMode::Private,
            institution_id: None,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: hex(&Sha256::digest(device_key.verifying_key().to_bytes())),
            public_key: hex(&device_key.verifying_key().to_bytes()),
        };
        let registry = Registry {
            connections: vec![connection.clone()],
            scopes: HashMap::from([(
                "session-bound-target".into(),
                Scope {
                    connection_id: connection_id.clone(),
                    run_id: "run-bound-target".into(),
                    channel_id: "channel-bound-target".into(),
                    source_channels: vec!["channel-bound-target".into()],
                    epoch: 1,
                    provider_binding: "private".into(),
                    public_provider: false,
                    origin_restricted: false,
                    institution_ids: BTreeSet::new(),
                    institution_policy: true,
                    expired: false,
                    expires_at: None,
                    labels: None,
                    session_incarnation: None,
                },
            )]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            manager_root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = manager().unwrap();
        manager
            .write_credential(
                &format!("device:{connection_id}"),
                &hex(&device_key.to_bytes()),
            )
            .unwrap();
        manager
            .write_credential("run:session-bound-target", "run-credential")
            .unwrap();
        let control = manager.control_path(&connection_id).unwrap();
        let transport = transport::Transport::connect(&connection, &control)
            .await
            .unwrap();
        manager
            .transports
            .lock()
            .await
            .insert(connection_id.clone(), Arc::new(Mutex::new(transport)));

        let client = crew_client("omitted-connection-id-target-session");
        let result = client
            .call_tool(
                "request",
                Some(
                    json!({"method": "remote.list"})
                        .as_object()
                        .cloned()
                        .unwrap(),
                ),
                McpMeta::new(
                    "session-bound-target",
                    CallCapability::for_test(ProviderTier::Private, true),
                ),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result_text(&result), r#"{"accepted_method":"remote.list"}"#);

        manager.disconnect(&connection_id).await.unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn institution_provider_and_origin_matrix_rejects_cross_boundary_inputs() {
        use crate::privacy::affiliation::{InstitutionId, ModelAffiliation};

        let mut ucsf = BTreeSet::new();
        ucsf.insert("ucsf".to_owned());
        let mut stanford = BTreeSet::new();
        stanford.insert("stanford".to_owned());
        let empty = BTreeSet::new();

        assert!(institution::check_provider(ProviderTier::Public, None, &empty).is_ok());
        assert!(institution::check_provider(ProviderTier::Public, None, &ucsf).is_err());
        assert!(institution::check_provider(
            ProviderTier::Private,
            Some(ModelAffiliation::Local),
            &ucsf
        )
        .is_ok());
        assert!(institution::check_provider(
            ProviderTier::Private,
            Some(ModelAffiliation::institution(InstitutionId::new("ucsf"))),
            &ucsf,
        )
        .is_ok());
        assert!(institution::check_provider(
            ProviderTier::Private,
            Some(ModelAffiliation::institution(InstitutionId::new(
                "stanford"
            ))),
            &ucsf,
        )
        .is_err());
        assert!(institution::check_provider(ProviderTier::Private, None, &ucsf).is_err());

        assert!(institution::check_origin(&empty, None).is_ok());
        assert!(institution::check_origin(&ucsf, Some("ucsf")).is_ok());
        assert!(institution::check_origin(&ucsf, Some("stanford")).is_err());
        assert!(institution::check_origin(&stanford, None).is_err());
    }

    /// A fake `ssh` that logs every request line and answers by method: `auth.challenge` for
    /// `workspace_id`, each of `answers` with its JSON result, and anything else with
    /// `{"accepted_method":"fixture"}`. Returns the log's path.
    #[cfg(unix)]
    fn write_answering_ssh(root: &Path, workspace_id: &str, answers: &[(&str, Value)]) -> PathBuf {
        write_scripted_ssh(root, workspace_id, answers, &[])
    }

    /// [`write_answering_ssh`], refusing each method of `refusals` with a broker error whose
    /// code and message are the given text.
    #[cfg(unix)]
    fn write_scripted_ssh(
        root: &Path,
        workspace_id: &str,
        answers: &[(&str, Value)],
        refusals: &[(&str, &str)],
    ) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let fake_bin = root.join("bin");
        fs::create_dir_all(&fake_bin).unwrap();
        let log = root.join("requests.log");
        let challenge = json!({"workspace_id": workspace_id, "nonce": "nonce", "uid": 10001});
        let mut branches = String::new();
        for (index, (method, result)) in [("auth.challenge", &challenge)]
            .into_iter()
            .chain(answers.iter().map(|(method, result)| (*method, result)))
            .enumerate()
        {
            let result = result.to_string();
            assert!(
                !result.contains('\'') && !result.contains('%'),
                "fixture JSON must survive sh quoting and printf: {result}"
            );
            let keyword = if index == 0 { "if" } else { "elif" };
            branches.push_str(&format!(
                "  {keyword} printf '%s\\n' \"$line\" | grep -q '\"method\":\"{method}\"'; then\n    printf '{{\"id\":\"%s\",\"result\":%s}}\\n' \"$id\" '{result}'\n"
            ));
        }
        for (method, code) in refusals {
            assert!(
                code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                "fixture refusal codes are plain words: {code}"
            );
            let error = json!({"code": code, "message": code}).to_string();
            branches.push_str(&format!(
                "  elif printf '%s\\n' \"$line\" | grep -q '\"method\":\"{method}\"'; then\n    printf '{{\"id\":\"%s\",\"error\":%s}}\\n' \"$id\" '{error}'\n"
            ));
        }
        let ssh = format!(
            r#"#!/bin/sh
log='{log}'
if [ "$1" = "-G" ]; then
  printf '%s\n' \
    'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$log"
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
{branches}  else
    printf '{{"id":"%s","result":{{"accepted_method":"fixture"}}}}\n' "$id"
  fi
done
"#,
            log = log.display()
        );
        let ssh_path = fake_bin.join("ssh");
        fs::write(&ssh_path, ssh).unwrap();
        fs::set_permissions(&ssh_path, fs::Permissions::from_mode(0o700)).unwrap();
        log
    }

    /// Point the profile, credentials and `ssh` of this process at `root`: file credentials
    /// under a development profile, never the OS keychain. Only inside a process of its own.
    #[cfg(unix)]
    fn isolated_crew_env(root: &Path) -> env_lock::EnvGuard<'static> {
        let profile_root = root.join("profile");
        fs::create_dir_all(&profile_root).unwrap();
        let original_path = std::env::var("PATH").unwrap_or_default();
        let path = format!("{}:{original_path}", root.join("bin").display());
        let profile = profile_root.to_string_lossy().into_owned();
        crate::test_sandbox::relocate_path_root_and(
            profile.as_str(),
            [
                ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile.as_str())),
                ("BIOROUTER_DISABLE_KEYRING", Some("true")),
                ("PATH", Some(path.as_str())),
            ],
        )
    }

    /// [`worker_race_connection`] with a real device key, so signed requests reach the wire.
    #[cfg(unix)]
    fn signed_fixture_connection(connection_id: &str) -> (Connection, Scope, SigningKey) {
        let device_key = SigningKey::from_bytes(&[7; 32]);
        let (mut connection, scope) =
            worker_race_connection(connection_id, ClusterMode::Public, 1, true);
        connection.device_id = hex(&Sha256::digest(device_key.verifying_key().to_bytes()));
        connection.public_key = hex(&device_key.verifying_key().to_bytes());
        (connection, scope, device_key)
    }

    #[cfg(unix)]
    async fn attach_fixture_transport(manager: &CrewManager, connection: &Connection) {
        let control = manager.control_path(&connection.id).unwrap();
        let transport = transport::Transport::connect(connection, &control)
            .await
            .unwrap();
        manager
            .transports
            .lock()
            .await
            .insert(connection.id.clone(), Arc::new(Mutex::new(transport)));
    }

    /// A saved connection and its `worker-race-session` grant, with the device and run
    /// credentials written, and a transport only when asked for.
    #[cfg(unix)]
    async fn signed_fixture_manager(
        root: &Path,
        connection: &Connection,
        scope: &Scope,
        device_key: &SigningKey,
        with_transport: bool,
    ) -> Arc<CrewManager> {
        let registry = Registry {
            connections: vec![connection.clone()],
            scopes: HashMap::from([("worker-race-session".into(), scope.clone())]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = Arc::new(CrewManager::new(root.to_owned()).unwrap());
        manager
            .write_credential(
                &format!("device:{}", connection.id),
                &hex(&device_key.to_bytes()),
            )
            .unwrap();
        manager
            .write_credential("run:worker-race-session", "run-credential")
            .unwrap();
        if with_transport {
            attach_fixture_transport(&manager, connection).await;
        }
        manager
    }

    #[cfg(unix)]
    fn logged_methods(log: &Path, method: &str) -> usize {
        let needle = format!("\"method\":\"{method}\"");
        fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains(&needle))
            .count()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn revoke_expires_local_scope_even_when_the_transport_is_down() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("revoke-transport-down");
        let connection_id = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
        let (connection, scope, device_key) = signed_fixture_connection(connection_id);
        let log = write_answering_ssh(&root, &connection.workspace_id, &[]);
        let _env = isolated_crew_env(&root);
        let manager = signed_fixture_manager(&root, &connection, &scope, &device_key, false).await;

        let outcome = manager
            .revoke_session_if_current("worker-race-session", "worker-race-run")
            .await
            .expect("the local stop lands even with the transport down");
        assert!(!outcome.remote_confirmed);
        assert!(outcome.run.is_none());
        let remote = outcome
            .remote_error
            .expect("why the workspace did not confirm")
            .to_string();
        assert!(remote.contains("disconnected"), "{remote}");

        assert!(manager.registry.lock().await.scopes["worker-race-session"].expired);
        let persisted: Value =
            serde_json::from_slice(&fs::read(root.join("connections.json")).unwrap()).unwrap();
        assert_eq!(
            persisted["scopes"]["worker-race-session"]["expired"],
            json!(true)
        );
        // A restarted daemon reads the stop back rather than reviving the grant.
        let restarted = CrewManager::new(root.clone()).unwrap();
        assert!(restarted.registry.lock().await.scopes["worker-race-session"].expired);

        let dispatch = manager
            .check_dispatch(
                "worker-race-session",
                &CallCapability::for_test(ProviderTier::Public, true),
            )
            .await
            .unwrap_err();
        assert_eq!(dispatch.to_string(), GRANT_REVOKED);
        let worker = manager
            .worker_request(
                "worker-race-session",
                "messages.history",
                json!({"channel_id": "worker-race-channel"}),
            )
            .await
            .unwrap_err();
        assert_eq!(worker.to_string(), GRANT_REVOKED);

        // The confirming callers keep their meaning: short of confirmation is an error, typed
        // so a route can answer 503, and its text is the workspace's, unchanged.
        let cancel = manager
            .cancel_run_if_current("worker-race-session", "worker-race-run")
            .await
            .unwrap_err();
        let unconfirmed = cancel
            .downcast_ref::<RevocationUnconfirmed>()
            .expect("an unconfirmed revoke is typed");
        assert_eq!(cancel.to_string(), unconfirmed.remote_error().to_string());
        assert!(cancel.to_string().contains("disconnected"), "{cancel}");

        assert_eq!(
            fs::read_to_string(&log).unwrap_or_default(),
            "",
            "nothing may reach the transport"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn revoke_is_idempotent_and_confirms_on_retry() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("revoke-idempotent");
        let connection_id = "ffffffff-ffff-4fff-8fff-ffffffffffff";
        let (connection, scope, device_key) = signed_fixture_connection(connection_id);
        let revoked_run = json!({"id": "worker-race-run", "revoked": true});
        let log = write_answering_ssh(
            &root,
            &connection.workspace_id,
            &[("run.revoke", revoked_run.clone())],
        );
        let _env = isolated_crew_env(&root);
        let manager = signed_fixture_manager(&root, &connection, &scope, &device_key, false).await;

        let offline = manager
            .revoke_session_if_current("worker-race-session", "worker-race-run")
            .await
            .unwrap();
        assert!(!offline.remote_confirmed);
        assert_eq!(logged_methods(&log, "run.revoke"), 0);

        attach_fixture_transport(&manager, &connection).await;
        let retry = manager
            .revoke_session_if_current("worker-race-session", "worker-race-run")
            .await
            .unwrap();
        assert!(retry.remote_confirmed);
        assert!(retry.remote_error.is_none());
        assert_eq!(retry.run, Some(revoked_run.clone()));
        assert_eq!(logged_methods(&log, "run.revoke"), 1);
        let sent = fs::read_to_string(&log).unwrap();
        assert!(sent
            .lines()
            .any(|line| line.contains("\"method\":\"run.revoke\"")
                && line.contains("worker-race-run")));

        // Revoking a revoked grant asks the workspace again (its run.revoke is idempotent),
        // and the confirming path answers with the run.
        assert_eq!(
            manager.cancel_run("worker-race-session").await.unwrap(),
            revoked_run
        );
        assert_eq!(logged_methods(&log, "run.revoke"), 2);
        assert!(manager.registry.lock().await.scopes["worker-race-session"].expired);

        manager.disconnect(connection_id).await.unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn revoke_with_a_replaced_run_refuses() {
        let root = fixture_root("revoke-replaced-run");
        let connection_id = "13131313-1313-4313-8313-131313131313";
        let (connection, scope) =
            worker_race_connection(connection_id, ClusterMode::Public, 1, true);
        let registry = Registry {
            connections: vec![connection],
            scopes: HashMap::from([("worker-race-session".into(), scope)]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let saved = fs::read(root.join("connections.json")).unwrap();
        let manager = CrewManager::new(root.clone()).unwrap();

        let refused = manager
            .revoke_session_if_current("worker-race-session", "an-older-run")
            .await
            .unwrap_err();
        assert_eq!(refused.to_string(), REPLACED_RUN);
        let cancel = manager
            .cancel_run_if_current("worker-race-session", "an-older-run")
            .await
            .unwrap_err();
        assert_eq!(cancel.to_string(), REPLACED_RUN);
        assert!(cancel.downcast_ref::<RevocationUnconfirmed>().is_none());
        assert!(!manager.registry.lock().await.scopes["worker-race-session"].expired);
        assert_eq!(fs::read(root.join("connections.json")).unwrap(), saved);

        let missing = manager.revoke_session("no-such-session").await.unwrap_err();
        assert_eq!(missing.to_string(), NO_GRANT);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn revoke_keeps_the_local_stop_when_it_cannot_be_saved() {
        let root = fixture_root("revoke-unsaved");
        let connection_id = "14141414-1414-4414-8414-141414141414";
        let (connection, scope) =
            worker_race_connection(connection_id, ClusterMode::Public, 1, true);
        let registry = Registry {
            connections: vec![connection],
            scopes: HashMap::from([("worker-race-session".into(), scope)]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        };
        fs::write(
            root.join("connections.json"),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let manager = CrewManager::new(root.clone()).unwrap();
        fs::remove_file(root.join("connections.json")).unwrap();
        fs::create_dir(root.join("connections.json")).unwrap();

        let error = manager
            .revoke_session_if_current("worker-race-session", "worker-race-run")
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Couldn't save the revocation on this device; retry to finish it"
        );
        assert!(manager.registry.lock().await.scopes["worker-race-session"].expired);
        assert_eq!(
            manager
                .check_dispatch(
                    "worker-race-session",
                    &CallCapability::for_test(ProviderTier::Public, true),
                )
                .await
                .unwrap_err()
                .to_string(),
            GRANT_REVOKED
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn grant_refusals_are_plain_words() {
        let root = fixture_root("plain-refusals");
        let connection_id = "15151515-1515-4515-8515-151515151515";
        let (connection, scope) =
            worker_race_connection(connection_id, ClusterMode::Public, 3, true);
        let manager = CrewManager::new(root.clone()).unwrap();
        {
            let mut registry = manager.registry.lock().await;
            registry.connections.push(connection);
            registry.scopes.insert("worker-race-session".into(), scope);
        }
        let capability = &CallCapability::for_test(ProviderTier::Public, true);
        let manager = &manager;
        let refusals = move || async move {
            let dispatch = manager
                .check_dispatch("worker-race-session", capability)
                .await
                .unwrap_err()
                .to_string();
            let worker = manager
                .worker_request(
                    "worker-race-session",
                    "messages.history",
                    json!({"channel_id": "worker-race-channel"}),
                )
                .await
                .unwrap_err()
                .to_string();
            (dispatch, worker)
        };

        manager
            .registry
            .lock()
            .await
            .scopes
            .get_mut("worker-race-session")
            .unwrap()
            .expired = true;
        assert_eq!(
            refusals().await,
            (GRANT_REVOKED.to_owned(), GRANT_REVOKED.to_owned())
        );

        {
            let mut registry = manager.registry.lock().await;
            registry
                .scopes
                .get_mut("worker-race-session")
                .unwrap()
                .expired = false;
            registry.connections[0].policy_epoch = 4;
        }
        assert_eq!(
            refusals().await,
            (
                GRANT_POLICY_CHANGED.to_owned(),
                GRANT_POLICY_CHANGED.to_owned()
            )
        );

        {
            let mut registry = manager.registry.lock().await;
            registry.connections[0].policy_epoch = 3;
            registry
                .scopes
                .get_mut("worker-race-session")
                .unwrap()
                .institution_policy = false;
        }
        assert_eq!(refusals().await.0, GRANT_POLICY_CHANGED);

        for text in [GRANT_REVOKED, GRANT_POLICY_CHANGED, NO_GRANT] {
            assert!(
                !text.contains("human grant") && !text.contains("Crew run"),
                "refusals speak of access, not of runs and grants: {text}"
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn connection_binding_is_unchanged_by_a_capability_refresh() {
        let root = fixture_root("capability-refresh");
        let connection_id = "16161616-1616-4616-8616-161616161616";
        let (connection, _) = worker_race_connection(connection_id, ClusterMode::Public, 1, true);
        let manager = CrewManager::new(root.clone()).unwrap();
        manager
            .registry
            .lock()
            .await
            .connections
            .push(connection.clone());
        let node = "cd".repeat(32);
        let hello = |version: u8, capabilities: &[&str], name: Option<&str>| VerifiedHello {
            node_id: node.clone(),
            broker: BrokerHello {
                signature_version: version,
                capabilities: capabilities.iter().map(|c| (*c).to_owned()).collect(),
                workspace_name: name.map(str::to_owned),
                mode: (version == 2).then_some(ClusterMode::Public),
                institution_id: None,
                policy_epoch: (version == 2).then_some(1),
            },
        };
        assert_eq!(manager.capabilities(connection_id), None);

        let first = manager
            .adopt_verified_hello(connection_id, &connection, hello(1, &["human_chat"], None))
            .await
            .unwrap();
        let pinned = connection_binding(&first).unwrap();
        assert_eq!(
            manager.capabilities(connection_id),
            Some(vec!["human_chat".to_owned()])
        );

        let refreshed = manager
            .adopt_verified_hello(
                connection_id,
                &first,
                hello(
                    2,
                    &["human_chat", "human_names_v1", "unique_names_v1"],
                    Some("lab"),
                ),
            )
            .await
            .unwrap();
        assert_eq!(connection_binding(&refreshed).unwrap(), pinned);
        assert_eq!(
            connection_binding(&manager.connection(connection_id).await.unwrap()).unwrap(),
            pinned
        );
        assert_eq!(
            manager.capabilities(connection_id),
            Some(vec![
                "human_chat".to_owned(),
                "human_names_v1".to_owned(),
                "unique_names_v1".to_owned()
            ])
        );
        let broker = manager.broker_hello(connection_id).unwrap();
        assert_eq!(broker.signature_version, 2);
        assert_eq!(broker.workspace_name.as_deref(), Some("lab"));
        let saved = fs::read_to_string(root.join("connections.json")).unwrap();
        assert!(
            !saved.contains("human_names_v1") && !saved.contains("capabilities"),
            "capabilities are not identity and never persist: {saved}"
        );

        manager.disconnect(connection_id).await.unwrap();
        assert_eq!(manager.capabilities(connection_id), None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hello_v1_and_v2_both_verify_and_a_tampered_v2_field_fails() {
        let key = SigningKey::from_bytes(&[9; 32]);
        let (mut connection, _) =
            worker_race_connection("hello-v2-connection", ClusterMode::Public, 1, true);
        connection.workspace_public_key = hex(&key.verifying_key().to_bytes());
        let node = "ab".repeat(32);
        let nonce = "hello-nonce";
        let capabilities = [
            "human_chat",
            "scoped_runs",
            "human_names_v1",
            "unique_names_v1",
        ];
        let v1 = hex(&key
            .sign(&biorouter_crew::hello_v1_payload(
                &connection.workspace_id,
                connection.owner_uid,
                nonce,
                &connection.workspace_public_key,
                &node,
            ))
            .to_bytes());
        let v2 = hex(&key
            .sign(
                &biorouter_crew::HelloV2 {
                    workspace_id: &connection.workspace_id,
                    host_uid: connection.owner_uid,
                    challenge_nonce: nonce,
                    workspace_public_key: &connection.workspace_public_key,
                    node_id: &node,
                    mode: &biorouter_crew::Mode::Public,
                    institution_id: Some("ucsf"),
                    policy_epoch: 4,
                    name: Some("lab"),
                    capabilities: &capabilities,
                }
                .signing_payload(),
            )
            .to_bytes());
        let hello = json!({
            "protocol": 1,
            "workspace_id": connection.workspace_id,
            "host_uid": connection.owner_uid,
            "workspace_public_key": connection.workspace_public_key,
            "challenge_nonce": nonce,
            "node_id": node,
            "mode": "public",
            "institution_id": "ucsf",
            "policy_epoch": 4,
            "name": "lab",
            "capabilities": capabilities,
            "signature": v1,
            "signature_v2": v2,
        });
        let verify =
            |hello: &Value| CrewManager::verify_workspace_identity(&connection, hello, nonce);
        let without = |field: &str| {
            let mut hello = hello.clone();
            hello.as_object_mut().unwrap().remove(field);
            hello
        };
        let all_capabilities: Vec<String> = capabilities.iter().map(|c| (*c).to_owned()).collect();

        // Both signatures: the fields v2 signs are the workspace's own word.
        let both = verify(&hello).unwrap();
        assert_eq!(both.node_id, node);
        assert_eq!(
            both.broker,
            BrokerHello {
                signature_version: 2,
                capabilities: all_capabilities.clone(),
                workspace_name: Some("lab".into()),
                mode: Some(ClusterMode::Public),
                institution_id: Some("ucsf".into()),
                policy_epoch: Some(4),
            }
        );
        assert_eq!(
            verify(&without("signature")).unwrap().broker,
            both.broker,
            "v2 alone is enough"
        );

        // v1 alone (an older broker, or a relay that stripped v2) still verifies, and nothing
        // v1 does not sign is passed on as trusted.
        let legacy = verify(&without("signature_v2")).unwrap();
        assert_eq!(legacy.node_id, node);
        assert_eq!(
            legacy.broker,
            BrokerHello {
                signature_version: 1,
                capabilities: all_capabilities,
                workspace_name: None,
                mode: None,
                institution_id: None,
                policy_epoch: None,
            }
        );

        // Any field v2 signs, altered, removed, reordered or added to, fails verification.
        let mut tampered_cases = vec![
            ("name", json!("lab-2")),
            ("mode", json!("private")),
            ("institution_id", Value::Null),
            ("institution_id", json!("stanford")),
            ("policy_epoch", json!(5)),
            (
                "capabilities",
                json!([
                    "human_chat",
                    "scoped_runs",
                    "human_names_v1",
                    "unique_names_v1",
                    "join_by_name_v1"
                ]),
            ),
            (
                "capabilities",
                json!([
                    "scoped_runs",
                    "human_chat",
                    "human_names_v1",
                    "unique_names_v1"
                ]),
            ),
            (
                "capabilities",
                json!(["human_chat", "scoped_runs", "human_names_v1"]),
            ),
        ];
        tampered_cases.push(("capabilities", json!(null)));
        for (field, value) in tampered_cases {
            let mut tampered = hello.clone();
            tampered[field] = value.clone();
            let Some(error) = verify(&tampered).err() else {
                panic!("hello with {field} = {value} verified");
            };
            assert!(
                error.downcast_ref::<WorkspaceIdentityError>().is_some(),
                "{error}"
            );
        }
        assert!(
            verify(&without("name")).is_err(),
            "a signed name was dropped"
        );

        // A v1 signature that does not verify fails even beside a valid v2.
        let mut bad_v1 = hello.clone();
        bad_v1["signature"] = json!(hex(&key.sign(b"another payload").to_bytes()));
        assert!(verify(&bad_v1).is_err());

        let mut unsigned = without("signature_v2");
        unsigned.as_object_mut().unwrap().remove("signature");
        assert_eq!(
            verify(&unsigned).err().unwrap().to_string(),
            "Workspace identity signature missing"
        );
        let wrong_nonce =
            CrewManager::verify_workspace_identity(&connection, &hello, "another-nonce")
                .err()
                .unwrap();
        assert_eq!(
            wrong_nonce.to_string(),
            "Workspace identity challenge mismatch"
        );
        assert!(wrong_nonce
            .downcast_ref::<WorkspaceIdentityError>()
            .is_some());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn auth_join_is_refused_through_human_request() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("auth-join-guard");
        let connection_id = "17171717-1717-4717-8717-171717171717";
        let (connection, scope, device_key) = signed_fixture_connection(connection_id);
        let log = write_answering_ssh(
            &root,
            &connection.workspace_id,
            &[("auth.join", json!({"joined": true}))],
        );
        let _env = isolated_crew_env(&root);
        let manager = signed_fixture_manager(&root, &connection, &scope, &device_key, true).await;
        let params = json!({"public_key": connection.public_key, "join_id": "join-1"});

        let refused = manager
            .human_request(connection_id, "auth.join", params.clone(), None)
            .await
            .unwrap_err();
        assert_eq!(
            refused.to_string(),
            "Only Biorouter's own join sends auth.join; it can't be sent as a Crew request"
        );
        let pending = manager
            .human_request(connection_id, "enrollment.pending", json!({}), None)
            .await
            .unwrap_err();
        assert_eq!(
            pending.to_string(),
            "Biorouter checks join status itself; it is never sent as a signed request"
        );
        assert_eq!(
            fs::read_to_string(&log).unwrap_or_default(),
            "",
            "a refused join must not reach the transport"
        );

        // The join's own door keeps the enrollment identity guard...
        let other_key = json!({"public_key": "44".repeat(32), "join_id": "join-1"});
        let mismatch = manager
            .signed_join_request(connection_id, other_key, None)
            .await
            .unwrap_err();
        assert_eq!(
            mismatch.to_string(),
            "Enrollment identity changed; refresh the saved connection before joining"
        );
        assert_eq!(fs::read_to_string(&log).unwrap_or_default(), "");

        // ...and is the one way auth.join reaches the workspace.
        assert_eq!(
            manager
                .signed_join_request(connection_id, params, None)
                .await
                .unwrap(),
            json!({"joined": true})
        );
        assert_eq!(logged_methods(&log, "auth.join"), 1);

        manager.disconnect(connection_id).await.unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn admission_labels_name_every_id_and_expiry_is_recorded() -> anyhow::Result<()> {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return Ok(());
        }
        let root = fixture_root("admission-labels");
        let connection_id = "18181818-1818-4818-8818-181818181818";
        let (connection, _, device_key) = signed_fixture_connection(connection_id);
        let alice = json!({"id": "principal-alice", "uid": 10001, "username": "alice",
            "nickname": "Alice Chen", "avatar": null, "active": true});
        let snapshot = json!({
            "workspace": {"id": connection.workspace_id, "host_uid": 10001, "mode": "public",
                "institution_id": null, "policy_epoch": 1, "name": "lab"},
            "actor": alice,
            "principals": [alice, {"id": "principal-bob", "uid": 10002, "username": "bob",
                "nickname": "bob", "avatar": null, "active": true}],
            "teams": [{"id": "team-analysis", "name": "Analysis Lab"},
                {"id": "team-core", "name": "Methods Core"}],
            "channels": [
                {"id": "destination-channel", "team_id": "team-analysis", "name": "methods"},
                {"id": "source-a", "team_id": "team-analysis", "name": "raw-data"},
                {"id": "source-b", "team_id": "team-core", "name": "general"}
            ],
            "protected_channel_ids": []
        });
        let log = write_answering_ssh(
            &root,
            &connection.workspace_id,
            &[
                ("workspace.snapshot", snapshot),
                (
                    "run.create",
                    json!({"run": {"id": "run-labelled", "protected_context": false,
                        "expires_at": 1_790_000_000u64}, "credential": "run-credential"}),
                ),
                ("messages.history", json!({"messages": []})),
            ],
        );
        let _env = isolated_crew_env(&root);
        let manager = CrewManager::new(root.join("manager"))?;
        manager
            .registry
            .lock()
            .await
            .connections
            .push(connection.clone());
        manager.write_credential(
            &format!("device:{}", connection.id),
            &hex(&device_key.to_bytes()),
        )?;
        attach_fixture_transport(&manager, &connection).await;
        let provider = crate::providers::testprovider::TestProvider::new_replaying(
            root.join("provider-cassette.json").to_string_lossy(),
        )?;

        let session = saved_chat(&root).await;
        let admission = manager
            .begin_run(
                &session,
                connection_id,
                "destination-channel",
                vec!["source-a".into(), "source-b".into()],
                &provider,
            )
            .await?;
        let channel = |id: &str, label: &str, team: &str| ChannelLabel {
            channel_id: id.into(),
            label: label.into(),
            team: Some(team.into()),
        };
        let expected = AdmissionLabels {
            you: Some("Alice Chen (@alice)".into()),
            workspace: "lab".into(),
            destination: channel("destination-channel", "#methods", "Analysis Lab"),
            sources: vec![
                channel("source-a", "#raw-data", "Analysis Lab"),
                channel("source-b", "#general", "Methods Core"),
                channel("destination-channel", "#methods", "Analysis Lab"),
            ],
        };
        assert_eq!(admission.labels, expected);
        assert_eq!(admission.expires_at, Some(1_790_000_000));

        // Every ID the model is handed has a label beside it.
        let labelled_everywhere = |carrier: &Value| {
            assert_eq!(carrier["labels"], serde_json::to_value(&expected).unwrap());
            assert_eq!(
                carrier["labels"]["destination"]["channel_id"],
                carrier["destination_channel_id"]
            );
            let labelled: Vec<&Value> = carrier["labels"]["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|source| &source["channel_id"])
                .collect();
            for id in carrier["source_channel_ids"].as_array().unwrap() {
                assert!(labelled.contains(&id), "{id} has no label");
            }
            for source in carrier["labels"]["sources"].as_array().unwrap() {
                assert!(source["label"].as_str().is_some_and(|l| l.starts_with('#')));
            }
        };
        let context: Value = serde_json::from_str(&admission.context)?;
        labelled_everywhere(&context);
        assert_eq!(logged_methods(&log, "workspace.snapshot"), 1);

        // agent_connections is on the worker path: it reads the stored labels and sends
        // nothing, least of all a human-signed snapshot.
        let before = fs::read_to_string(&log)?;
        let discovery = manager.agent_connections(&session).await?;
        let entry = &discovery["connections"][0];
        labelled_everywhere(entry);
        assert_eq!(entry["labels"]["workspace"], "lab");
        assert!(entry["workspace_id"].is_string() && entry["labels"]["you"].is_string());
        assert_eq!(fs::read_to_string(&log)?, before);

        let grants = manager.session_grants(connection_id).await?;
        let grant = &grants["grants"][0];
        assert_eq!(grant["expires_at"], json!(1_790_000_000u64));
        assert_eq!(grant["labels"]["destination"]["label"], "#methods");
        let persisted: Value =
            serde_json::from_slice(&fs::read(root.join("manager").join("connections.json"))?)?;
        assert_eq!(
            persisted["scopes"][&session]["expires_at"],
            json!(1_790_000_000u64)
        );
        assert_eq!(
            persisted["scopes"][&session]["labels"]["you"],
            "Alice Chen (@alice)"
        );
        // The grant is bound to the chat it was made to (SCOPE-BIND).
        assert_eq!(
            persisted["scopes"][&session]["session_incarnation"],
            json!(SessionManager::instance()
                .session_incarnation(&session)
                .await?
                .expect("the granted chat is saved"))
        );

        // A chat that is not saved on this device — a `--no-session` run's — is refused
        // before any run exists at the workspace.
        let unsaved = manager
            .begin_run(
                "e0000000_1",
                connection_id,
                "destination-channel",
                vec![],
                &provider,
            )
            .await
            .err()
            .expect("a chat that is not saved here must not be granted");
        assert_eq!(unsaved.to_string(), UNSAVED_CHAT);
        assert_eq!(logged_methods(&log, "run.create"), 1);
        assert!(!manager
            .registry
            .lock()
            .await
            .scopes
            .contains_key("e0000000_1"));

        manager.disconnect(connection_id).await?;
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    /// A workspace, a connection to it and a manager with a live transport, for driving a grant
    /// through `begin_run` against a fake broker that answers `run.create` with `run_create`
    /// and refuses each method of `refusals`. Returns the manager, the request log and the
    /// grantable chat. Only inside a process of its own.
    #[cfg(unix)]
    async fn abandon_fixture(
        root: &Path,
        connection_id: &str,
        run_create: Value,
        refusals: &[(&str, &str)],
    ) -> (CrewManager, PathBuf, String) {
        let (connection, _, device_key) = signed_fixture_connection(connection_id);
        let snapshot = json!({
            "workspace": {"id": connection.workspace_id, "host_uid": 10001, "mode": "public",
                "institution_id": null, "policy_epoch": 1, "name": "lab"},
            "principals": [],
            "teams": [],
            "channels": [{"id": "destination-channel", "name": "methods"},
                {"id": "source-a", "name": "raw-data"}],
            "protected_channel_ids": []
        });
        let log = write_scripted_ssh(
            root,
            &connection.workspace_id,
            &[("workspace.snapshot", snapshot), ("run.create", run_create)],
            refusals,
        );
        let manager = CrewManager::new(root.join("manager")).unwrap();
        manager
            .registry
            .lock()
            .await
            .connections
            .push(connection.clone());
        manager
            .write_credential(
                &format!("device:{}", connection.id),
                &hex(&device_key.to_bytes()),
            )
            .unwrap();
        attach_fixture_transport(&manager, &connection).await;
        let session = saved_chat(root).await;
        (manager, log, session)
    }

    /// The request lines of `method` in `log`, in order.
    #[cfg(unix)]
    fn logged_lines(log: &Path, method: &str) -> Vec<String> {
        let needle = format!("\"method\":\"{method}\"");
        fs::read_to_string(log)
            .unwrap_or_default()
            .lines()
            .filter(|line| line.contains(&needle))
            .map(str::to_owned)
            .collect()
    }

    /// D7. ⚠ Grant and revocation state; a change here needs human review. The workspace
    /// created the run, the grant was recorded, and then the first `messages.history` failed:
    /// the grant used to stay active here and the run live at the workspace. Now the run is
    /// revoked, the grant is left expired (saved, and in memory), and the caller still gets
    /// the failure that caused it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_grant_whose_history_fails_is_revoked_and_left_expired() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("abandon-history");
        let _env = isolated_crew_env(&root);
        let connection_id = "19191919-1919-4919-8919-191919191919";
        let (manager, log, session) = abandon_fixture(
            &root,
            connection_id,
            json!({"run": {"id": "run-abandoned", "protected_context": false,
                "expires_at": 1_790_000_000u64}, "credential": "run-credential"}),
            &[("messages.history", "history_unavailable")],
        )
        .await;
        let provider = crate::providers::testprovider::TestProvider::new_replaying(
            root.join("provider-cassette.json").to_string_lossy(),
        )
        .unwrap();

        let error = manager
            .begin_run(
                &session,
                connection_id,
                "destination-channel",
                vec!["source-a".into()],
                &provider,
            )
            .await
            .err()
            .expect("a grant whose first history read fails is not granted");
        assert!(
            error.to_string().contains("history_unavailable"),
            "the caller must get the failure that stopped the grant, not the cleanup's: {error}"
        );

        assert_eq!(logged_methods(&log, "run.create"), 1);
        assert_eq!(logged_methods(&log, "messages.history"), 1);
        let revokes = logged_lines(&log, "run.revoke");
        assert_eq!(revokes.len(), 1, "the created run must be revoked once");
        assert!(revokes[0].contains("run-abandoned"), "{}", revokes[0]);
        let sent = fs::read_to_string(&log).unwrap();
        assert!(
            sent.find("messages.history").unwrap() < sent.find("run.revoke").unwrap(),
            "the run is revoked after the failure, not before: {sent}"
        );

        let scope = manager.registry.lock().await.scopes[&session].clone();
        assert_eq!(scope.run_id, "run-abandoned");
        assert!(scope.expired, "a failed grant must never be left active");
        let persisted: Value = serde_json::from_slice(
            &fs::read(root.join("manager").join("connections.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            persisted["scopes"][&session]["run_id"],
            json!("run-abandoned")
        );
        assert_eq!(persisted["scopes"][&session]["expired"], json!(true));
        assert_eq!(
            manager
                .check_dispatch(
                    &session,
                    &CallCapability::for_test(ProviderTier::Public, true),
                )
                .await
                .unwrap_err()
                .to_string(),
            GRANT_REVOKED
        );
        // Still listed, as revoked, so the person sees what happened.
        let grants = manager.session_grants(connection_id).await.unwrap();
        assert_eq!(grants["grants"][0]["run_id"], json!("run-abandoned"));
        assert_eq!(grants["grants"][0]["expired"], json!(true));

        manager.disconnect(connection_id).await.unwrap();
        let _ = fs::remove_dir_all(root);
    }

    /// D7, before the grant is recorded: the broker's answer did not match what was admitted,
    /// so nothing was recorded here — but the run exists at the workspace. It is revoked, and
    /// no grant or run credential is left behind.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_refused_before_its_grant_is_recorded_is_revoked_and_leaves_nothing() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("abandon-unrecorded");
        let _env = isolated_crew_env(&root);
        let connection_id = "1a1a1a1a-1a1a-4a1a-8a1a-1a1a1a1a1a1a";
        let (manager, log, session) = abandon_fixture(
            &root,
            connection_id,
            // The admission expected no protected context; the broker claims otherwise.
            json!({"run": {"id": "run-unrecorded", "protected_context": true},
                "credential": "run-credential"}),
            &[],
        )
        .await;
        let provider = crate::providers::testprovider::TestProvider::new_replaying(
            root.join("provider-cassette.json").to_string_lossy(),
        )
        .unwrap();

        let error = manager
            .begin_run(
                &session,
                connection_id,
                "destination-channel",
                vec!["source-a".into()],
                &provider,
            )
            .await
            .err()
            .expect("a run the broker answered differently is not granted");
        assert!(
            error.to_string().contains("protected-context"),
            "the caller must get the refusal itself: {error}"
        );

        assert_eq!(logged_methods(&log, "run.create"), 1);
        let revokes = logged_lines(&log, "run.revoke");
        assert_eq!(revokes.len(), 1, "the created run must be revoked");
        assert!(revokes[0].contains("run-unrecorded"), "{}", revokes[0]);
        assert_eq!(logged_methods(&log, "messages.history"), 0);
        assert!(!manager.registry.lock().await.scopes.contains_key(&session));
        assert!(
            !manager.credential_path(&format!("run:{session}")).exists(),
            "no run credential may outlive a run that was never granted"
        );
        assert!(manager.read_credential(&format!("run:{session}")).is_err());
        let saved = root.join("manager").join("connections.json");
        if saved.exists() {
            let persisted: Value = serde_json::from_slice(&fs::read(saved).unwrap()).unwrap();
            assert!(persisted["scopes"].get(&session).is_none());
        }

        manager.disconnect(connection_id).await.unwrap();
        let _ = fs::remove_dir_all(root);
    }

    /// The run credential a failed setup wrote goes with it, from whichever backend holds it;
    /// deleting one that is already gone is not an error.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_deleted_credential_is_gone_and_deleting_it_again_is_harmless() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("delete-credential");
        let _env = isolated_crew_env(&root);
        let manager = CrewManager::new(root.join("manager")).unwrap();
        manager
            .write_credential("run:doomed", "run-credential")
            .unwrap();
        manager
            .write_credential("run:kept", "other-credential")
            .unwrap();
        manager.delete_credential("run:doomed").unwrap();
        assert!(!manager.credential_path("run:doomed").exists());
        assert!(manager.read_credential("run:doomed").is_err());
        manager.delete_credential("run:doomed").unwrap();
        assert_eq!(
            manager.read_credential("run:kept").unwrap().as_str(),
            "other-credential"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// T-50: removing a saved connection deletes its device private key, so no key file is
    /// left behind in development file mode; another connection's key stays, and a prepared
    /// device not yet saved is never removed through the connection path.
    #[cfg(unix)]
    #[tokio::test]
    async fn removing_a_connection_deletes_its_device_key() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("remove-device-key");
        let _env = isolated_crew_env(&root);
        let manager = CrewManager::new(root.join("manager")).unwrap();
        let input = |workspace: &str, target: &str| SaveConnection {
            preparation_id: None,
            name: format!("saved {target}"),
            ssh_target: target.into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/tmp/crew-remove-key.sock".into(),
            owner_uid: 10001,
            workspace_id: workspace.into(),
            workspace_public_key: "44".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: None,
            mode: ClusterMode::Public,
            institution_id: None,
        };
        let doomed = manager
            .save(input(
                "11111111-1111-4111-8111-111111111111",
                "bob@doomed.example.org",
            ))
            .await
            .unwrap();
        let kept = manager
            .save(input(
                "22222222-2222-4222-8222-222222222222",
                "bob@kept.example.org",
            ))
            .await
            .unwrap();
        let key_file = manager.credential_path(&format!("device:{}", doomed.id));
        assert!(key_file.exists(), "saving wrote the device key");

        manager.remove(&doomed.id).await.unwrap();
        assert!(
            !key_file.exists(),
            "the removed connection's key file is gone"
        );
        assert!(manager
            .read_credential(&format!("device:{}", doomed.id))
            .is_err());
        assert!(manager
            .read_credential(&format!("device:{}", kept.id))
            .is_ok());
        // Removing it again (or a connection another process already removed) is harmless.
        manager.remove(&doomed.id).await.unwrap();

        let prepared = manager.prepare_device().await.unwrap();
        manager.remove(&prepared.preparation_id).await.unwrap();
        assert!(manager
            .read_credential(&format!("device:{}", prepared.preparation_id))
            .is_ok());
        let _ = fs::remove_dir_all(root);
    }

    /// T-52: connecting a second workspace on the same server under another institution is
    /// still refused (one node is one privacy boundary), in words a person can act on.
    #[test]
    fn mixing_institutions_on_one_server_is_refused_in_plain_words() {
        let node = "ab".repeat(32);
        let (mut foreign, _) =
            worker_race_connection("mixed-foreign", ClusterMode::Private, 3, true);
        foreign.name = "Foreign lab".into();
        foreign.institution_id = Some("foreign-synthetic".into());
        foreign.node_id = Some(node.clone());
        let (mut joining, _) = worker_race_connection("mixed-lab", ClusterMode::Private, 1, true);
        joining.id = "a9a9a9a9-a9a9-49a9-89a9-a9a9a9a9a9a9".into();
        joining.name = "chen-lab".into();
        joining.institution_id = Some("ucsf".into());
        joining.node_id = None;
        joining.cluster_connection_id = uuid::Uuid::new_v4().to_string();
        let mut registry = Registry {
            connections: vec![foreign.clone(), joining.clone()],
            ..Default::default()
        };
        let error = CrewManager::adopt_node(&mut registry, &joining.id, &joining, node.clone())
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "You already use this server for Foreign lab (foreign-synthetic). chen-lab uses ucsf; one computer can't mix institutions on the same server."
        );
        assert!(!error.to_string().contains("aliases"));
        // Refused, not merged: nothing about either connection changed.
        assert_eq!(
            registry.connections[0].institution_id,
            foreign.institution_id
        );
        assert_eq!(registry.connections[1].node_id, None);

        // The same institution on the same server is one boundary, and connects.
        registry.connections[1].institution_id = Some("foreign-synthetic".into());
        let joining = registry.connections[1].clone();
        let connected =
            CrewManager::adopt_node(&mut registry, &joining.id, &joining, node.clone()).unwrap();
        assert_eq!(connected.node_id, Some(node));
    }

    /// The same deletion from an encrypted vault: it leaves the vault, other entries stay, and
    /// a locked vault is an error — never a fall back to the keyring.
    #[test]
    #[serial_test::serial(crew_credentials)]
    fn a_vault_credential_is_deleted_there_and_never_through_the_keyring() {
        let root = tempfile::tempdir().unwrap();
        let vault = credentials::CredentialVault::new(root.path().to_path_buf());
        vault
            .init(zeroize::Zeroizing::new(
                "correct horse battery staple".into(),
            ))
            .unwrap();
        let no_keyring = || -> Result<()> { anyhow::bail!("the keyring was reached") };
        let no_keyring_read =
            || -> Result<zeroize::Zeroizing<String>> { anyhow::bail!("the keyring was reached") };
        vault
            .write("run:doomed", "run-credential", no_keyring)
            .unwrap();
        vault
            .write("run:kept", "other-credential", no_keyring)
            .unwrap();

        vault.delete("run:doomed", no_keyring).unwrap();
        vault.delete("run:doomed", no_keyring).unwrap();
        let gone = vault.read("run:doomed", no_keyring_read).unwrap_err();
        assert!(gone.to_string().contains("absent"), "{gone}");
        assert_eq!(
            vault.read("run:kept", no_keyring_read).unwrap().as_str(),
            "other-credential"
        );

        vault.lock().unwrap();
        let locked = vault.delete("run:kept", no_keyring).unwrap_err();
        assert!(
            !locked.to_string().contains("keyring was reached"),
            "{locked}"
        );
    }

    #[test]
    fn admission_labels_follow_the_display_rule_and_fall_back_without_names() {
        let (connection, _) =
            worker_race_connection("labels-connection", ClusterMode::Public, 1, true);
        let snapshot = |actor: Value| {
            json!({
                "workspace": {"name": null},
                "actor": actor,
                "principals": [{"username": "alice"}, {"username": "bob"}],
                "teams": [],
                "channels": [{"id": "c1", "team_id": "hidden-team", "name": "Methods\u{202e}"}],
            })
        };
        let labels = admission_labels(
            &snapshot(json!({"username": "alice", "nickname": "alice"})),
            &connection,
            "c1",
            &["c1".into(), "c-missing".into()],
        );
        // A nickname equal to the username was never chosen (D13).
        assert_eq!(labels.you.as_deref(), Some("@alice"));
        // No workspace name: this device's name for the connection.
        assert_eq!(labels.workspace, "worker-race");
        // Invisible characters removed; a team the snapshot did not show is not guessed.
        assert_eq!(labels.destination.label, "#Methods");
        assert_eq!(labels.destination.team, None);
        assert_eq!(labels.sources.len(), 2);
        assert_eq!(labels.sources[1].channel_id, "c-missing");
        assert_eq!(labels.sources[1].label, "#untitled");

        let you = |actor: Value| {
            admission_labels(&snapshot(actor), &connection, "c1", &["c1".into()]).you
        };
        assert_eq!(
            you(json!({"username": "alice", "nickname": "Alice Chen"})).as_deref(),
            Some("Alice Chen (@alice)")
        );
        assert_eq!(
            you(json!({"username": "alice", "nickname": "bob"})).as_deref(),
            Some("@alice"),
            "a nickname posing as another person's username is not shown"
        );
        assert_eq!(
            you(json!({"username": "alice", "nickname": "Al\u{202e}ice"})).as_deref(),
            Some("Alice (@alice)")
        );
        assert_eq!(
            you(json!({"username": "alice", "nickname": "x", "display_name": "Dr. Chen"}))
                .as_deref(),
            Some("Dr. Chen (@alice)")
        );
        assert_eq!(you(Value::Null), None);
    }
}
