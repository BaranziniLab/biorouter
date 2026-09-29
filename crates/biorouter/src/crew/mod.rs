//! Saved native SSH connections and owner-scoped Crew capabilities.
pub mod authentication;
mod credentials;
mod freshness;
#[cfg(test)]
#[path = "freshness_tests.rs"]
mod freshness_tests;
mod host_start;
#[cfg(test)]
#[path = "host_start_tests.rs"]
mod host_start_tests;
mod institution;
#[cfg(test)]
#[path = "institution_tests.rs"]
mod institution_tests;
pub use host_start::{
    cancel_host_start, host_start_command, host_start_status, read_start_output, HostStartError,
    HostStartRefused, HostStartRequest, HostStartState, HostStartStatus, StartOutput,
};
mod keepalive;
mod local_host;
mod revocation;
mod server_label;
pub use server_label::server_label;
#[cfg(test)]
#[path = "keepalive_tests.rs"]
mod keepalive_tests;
pub mod observation;
#[cfg(test)]
#[path = "provider_identity_tests.rs"]
mod provider_identity_tests;
#[cfg(test)]
#[path = "registry_lock_tests.rs"]
mod registry_lock_tests;
#[cfg(test)]
#[path = "scope_binding_tests.rs"]
mod scope_binding_tests;
pub use credentials::CredentialStatus;
pub mod refusal;
pub mod source_line;
pub use institution::{refusal_details as institution_refusal_details, AFFILIATION_REFUSAL};
pub use refusal::CrewRefusal;
mod ssh_policy;
mod transport;
use crate::{
    privacy::{CallCapability, ProviderTier},
    providers::base::Provider,
};
use anyhow::{ensure, Result};
use chrono::{DateTime, Datelike, Offset, TimeZone};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
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
/// A connection to save or edit (`POST /crew/connections`, `PATCH /crew/connections/{id}`).
#[derive(Clone, Debug, Deserialize, Serialize, utoipa::ToSchema)]
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
/// A connection saved on this computer, as the registry keeps it.
#[derive(Clone, Debug, Deserialize, Serialize, utoipa::ToSchema)]
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
    /// Always sent; the default only reads a registry saved before it was kept.
    #[serde(default)]
    #[schema(required = true)]
    pub remote_execution: bool,
    pub cluster_connection_id: String,
    pub mode: ClusterMode,
    #[serde(default)]
    pub institution_id: Option<String>,
    pub policy_epoch: u64,
    /// `connected` or `disconnected`.
    pub status: String,
    /// Why the connection last failed or dropped, for a person; `null` when it has not.
    pub last_error: Option<String>,
    pub device_id: String,
    pub public_key: String,
}
/// How to sign in to a connection's server in a terminal (`POST /crew/connections/{id}/auth-plan`).
#[derive(Serialize, utoipa::ToSchema)]
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
    /// The session store the chat was saved in: the directory of the `sessions.db` whose row
    /// held the session id at grant time (CROSSCUT-8). Processes that share this registry need
    /// not share a store (a terminal whose shell sets its own `XDG_DATA_HOME` reads the same
    /// Crew settings as the desktop and another `sessions.db`), and two stores mint the same
    /// ids, so what one store holds, or lacks, says nothing about a chat saved in another. See
    /// [`GrantStore`]. `None` for a grant recorded before this was kept, until a process whose
    /// store holds the chat, or saw it deleted, records it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_store: Option<String>,
    /// Where a stopped grant stands with the workspace (F3, D-1). `None` while the grant is
    /// live, and for one stopped before this was recorded or by removing its connection,
    /// whose standing with the workspace is not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revocation: Option<Revocation>,
}

/// Where a grant that stopped on this device stands with the workspace.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Revocation {
    /// Stopped here; the workspace has not yet confirmed `run.revoke`. The daemon asks it
    /// again by itself whenever the connection comes back, until it does (F3).
    Unconfirmed,
    /// The workspace confirmed `run.revoke`.
    Confirmed,
    /// The workspace itself refused the run as ended (`grant_expired`: its policy moved since
    /// the grant, the run's context became protected, or the run was ended there), so there
    /// is nothing left to revoke at the workspace (D-1).
    EndedByWorkspace,
}

impl Revocation {
    /// How much the workspace is known to have said: a confirmation (or the workspace's own
    /// refusal of the run) is never forgotten for a later "not yet".
    fn rank(revocation: Option<Self>) -> u8 {
        match revocation {
            None => 0,
            Some(Self::Unconfirmed) => 1,
            Some(Self::EndedByWorkspace) => 2,
            Some(Self::Confirmed) => 3,
        }
    }
}

/// A stopped grant whose revocation the workspace had not confirmed when its chat's id stopped
/// holding it: a new grant to the same chat replaced it, or it was the grant of a deleted chat
/// whose id another chat now holds (SCOPE-BIND). Only [`Registry::scopes`] is ever asked what
/// a chat may do, so dropping such a grant there dropped the one record that made the daemon
/// ask the workspace to revoke its run: the run stayed live at the workspace until it lapsed,
/// and the grants list stopped showing it (F3). It is kept here instead, asked about again
/// like any unconfirmed stop, listed, and forgotten only once it is settled
/// ([`revocation`]).
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
struct ReplacedGrant {
    /// The id the grant was stored under. The chat holding that id now may be another chat.
    session_id: String,
    /// The grant as it stood, always stopped (`expired`).
    scope: Scope,
}

impl Scope {
    /// What a stopped grant's chat is told when it tries to use Crew: the workspace ending the
    /// run because its policy moved reads as the policy change it is (D-1), anything else as
    /// the access that was removed.
    fn stopped_text(&self) -> &'static str {
        if self.revocation == Some(Revocation::EndedByWorkspace) {
            GRANT_POLICY_CHANGED
        } else {
            GRANT_REVOKED
        }
    }
    /// [`Self::stopped_text`], typed `crew_grant_ended` with its reason (T3-BE-7).
    fn stopped(&self) -> CrewRefusal {
        if self.revocation == Some(Revocation::EndedByWorkspace) {
            CrewRefusal::grant_ended(refusal::GRANT_ENDED_SETTINGS_CHANGED, GRANT_POLICY_CHANGED)
        } else {
            CrewRefusal::grant_ended(refusal::GRANT_ENDED_ENDED, GRANT_REVOKED)
        }
    }
}

/// The display names of a run's identifiers, captured under the person's action when the
/// run is admitted (naming design D13 and D14). Never authority: every check still compares
/// the IDs, and the labels are not refreshed afterwards.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
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
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
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
    /// Whether the workspace server has stopped saving changes, as this `hello` said (T3-BE-13).
    /// Unsigned under either signature, so it is shown and never relied on; `None` when the
    /// server is saving, or is an older one that does not say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<ServerStorage>,
}

/// A workspace server that has stopped saving changes (its disk or quota is full, or it could
/// not write its storage), as its `hello` says since W2-BRK-3. Reading still works; every
/// change is refused until the host frees space and restarts Crew (T3-BE-13). Served as a saved
/// connection's `server_storage`, so a person is told before trying to write, not after.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct ServerStorage {
    /// Always `storage_failed`: the server has stopped saving changes.
    pub state: String,
    /// Why: `storage_full` (the server's disk or the account's quota is full) or
    /// `storage_failed` (another storage error).
    pub code: String,
    /// When the server stopped saving, in seconds since the Unix epoch, as it says; `null` when
    /// it did not say.
    pub since: Option<u64>,
}

/// [`ServerStorage::state`].
const STORAGE_STOPPED: &str = "storage_failed";

impl ServerStorage {
    /// What `hello` says about the server's storage: `Some` only for a server that says it has
    /// stopped saving (`"state": "storage_failed"`). Its own sentence is not kept: it is the
    /// workspace's unauthenticated words, and each client says this in its own.
    fn from_hello(hello: &Value) -> Option<Self> {
        if hello.get("state").and_then(Value::as_str) != Some(STORAGE_STOPPED) {
            return None;
        }
        let storage = hello.get("storage");
        let code = match storage.and_then(|storage| storage["code"].as_str()) {
            Some("storage_full") => "storage_full",
            _ => "storage_failed",
        };
        Some(Self {
            state: STORAGE_STOPPED.to_owned(),
            code: code.to_owned(),
            since: storage.and_then(|storage| storage["since"].as_u64()),
        })
    }
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
/// Shown when the connection's privacy or policy moved after the grant was made, and when the
/// workspace refuses the run as ended (`grant_expired`) because its own policy moved (D-1).
const GRANT_POLICY_CHANGED: &str =
    "Crew settings changed since access was granted. Grant access again from Crew.";
/// Shown when the workspace refuses a run whose own task ended it: its result was posted, or
/// its owner stopped it (W2-DMN-14). Its access ended with it; nothing about the settings
/// changed.
const TASK_ENDED: &str = "This task has ended, so its access to the workspace has ended too.";
/// How many ended runs [`CrewManager::ended_runs`] remembers before it starts again.
const MAX_ENDED_RUNS: usize = 4096;
/// Shown when the workspace refuses a run whose time ran out (`expires_at` has passed).
const GRANT_TIMED_OUT: &str =
    "This chat's Crew access has ended. Grant access again from Crew to continue.";
/// The broker's code for a run it no longer honors: revoked, expired or its policy changed.
const BROKER_GRANT_EXPIRED: &str = "grant_expired";
/// Shown when a chat with no Crew access asks for a Crew run, or a revocation finds none. It
/// names the one step that connects this chat, in this chat (Q3-29): "Grant it access from
/// Crew" sent people to Crew, whose empty state sent them back. It is written to the person,
/// because a person reads it too (a revocation that lost a race), and the model repeats it to
/// them as it stands.
pub(crate) const NO_GRANT: &str =
    "This chat isn't connected to a Crew channel. To connect it, type /crew in this chat.";
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
/// A grant asked for a chat whose id a chat saved in another Biorouter data folder on this
/// computer also has, and that chat holds Crew access (CROSSCUT-8). One grant is kept per id,
/// so granting this chat would take that chat's grant away.
const GRANT_ID_HELD_ELSEWHERE: &str = "A chat saved in another Biorouter data folder on this computer has the same ID as this chat and holds Crew access, so this chat can't be granted access. Start a new chat, then grant it access from Crew.";
/// The grant changed between two reads of one check.
const ACCESS_CHANGED: &str = "Crew access or settings changed while this was in progress. Check whether it already took effect before you grant access again.";
/// A bridge that failed carrying a request: what was sent may or may not have reached the
/// workspace.
const BRIDGE_FAILED: &str = "SSH bridge failed. Reconnect; inspect any submitted operation before retrying because its outcome may be unknown.";
/// The last error of a bridge that broke under a read: nothing can have changed (W2-DMN-6).
const READ_DROPPED: &str = "The connection to this workspace dropped.";

/// A wait for the next dial this long or shorter reads as "in a moment" (T3-BE-16).
const RECONNECT_SOON: std::time::Duration = std::time::Duration::from_secs(10);

/// What a request refused with `crew_reconnecting` is told, when the daemon dials `workspace`
/// again in `wait` (T3-BE-16). A short or unknown wait is "a moment", as it always was. A longer
/// one (after two breaks in a row the schedule waits 60 s, then 180 s) says how long and that
/// Connect tries at once: every send meanwhile used to say "try again in a moment" for up to
/// three minutes.
fn reconnecting_sentence(workspace: &str, wait: Option<std::time::Duration>) -> String {
    match wait.filter(|wait| *wait > RECONNECT_SOON) {
        None => format!("Reconnecting to {workspace}. Nothing was sent; try again in a moment."),
        Some(wait) => {
            let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
            let when = if seconds < 120 {
                format!("{seconds} seconds")
            } else {
                format!("{} minutes", seconds.div_ceil(60))
            };
            format!(
                "Reconnecting to {workspace} in about {when}. Nothing was sent. Connect now to try at once."
            )
        }
    }
}

/// The last error of a bridge that broke carrying `method`, whose answer was `answer`
/// (T3-BE-1). [`BRIDGE_FAILED`], which asks the person to inspect what was submitted, only when a
/// change was written and its answer lost: `crew_outcome_unknown`, an SSH failure that says so,
/// or an error of no known kind for a method that could have changed something. A read, a
/// request lost before it was written (`crew_not_sent`: only its challenge was lost, or the
/// bridge could not deliver it) and anything that was answered changed nothing:
/// [`READ_DROPPED`]. It used to follow the method alone, so a post answered `crew_not_sent` left
/// the alarm.
fn bridge_loss_message<T>(method: &str, answer: &Result<T>) -> &'static str {
    let Err(error) = answer else {
        return READ_DROPPED;
    };
    let lost = match CrewRefusal::find(error) {
        Some(refused) => refused.code() == refusal::OUTCOME_UNKNOWN,
        None => match error
            .chain()
            .find_map(|cause| cause.downcast_ref::<SshFailure>())
        {
            Some(failure) => failure.outcome_unknown,
            None => !is_read_only(method),
        },
    };
    if lost {
        BRIDGE_FAILED
    } else {
        READ_DROPPED
    }
}

/// Whether `method` only reads: whatever became of it, nothing changed at the workspace. An
/// explicit list, so a method not named here is treated as one that may have changed something.
/// `channel.read` moves a read position, so it is not one.
fn is_read_only(method: &str) -> bool {
    matches!(
        method,
        "hello"
            | "auth.challenge"
            | "enrollment.pending"
            | "workspace.snapshot"
            | "messages.history"
            | "messages.search"
            | "context.manifest"
            | "blob.read"
            | "blob.status"
            | "reference.get"
            | "profile.suggest"
            | "run.remote_scope"
            | "remote.list"
            | "remote.read"
            | "remote.hash"
            | "remote.job_status"
    )
}

/// Where a request's bridge was lost (W2-DMN-7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lost {
    /// Before the request was written: a dial, or the challenge ahead of it.
    BeforeSending,
    /// With the request written and no answer read.
    WhileCarrying,
}

/// A request's failure, typed by what the person can know about it (W2-DMN-7). The transport's
/// own error stays underneath, so a caller that classifies the SSH failure (connect, the
/// keepalive's retries) still finds it; the routes answer the refusal on top:
///
/// - lost before anything was written, or the bridge said it could not deliver the request:
///   `crew_not_sent`, nothing reached the workspace;
/// - a read lost while it was carried: `crew_not_sent` too, since nothing can have changed;
/// - anything else lost while it was carried: `crew_outcome_unknown` with the request's
///   idempotency key as `request_id`, so a retry with it is applied at most once.
///
/// Both answer 503. Every other error (a refusal the workspace answered, a check here) is
/// returned as it came. It used to reach the routes as `400 crew_request_refused`, so a post the
/// workspace had applied read as refused, and a client dropped its "retry with the same request
/// ID" advice.
fn lost_request(
    error: anyhow::Error,
    lost: Lost,
    method: &str,
    request_key: Option<String>,
    workspace: &str,
) -> anyhow::Error {
    if CrewRefusal::find(&error).is_some() {
        return error;
    }
    if keepalive::broker_refusal(&error).is_some_and(|(code, _)| code == "not_delivered") {
        let refusal = CrewRefusal::new(
            refusal::NOT_SENT,
            format!("{workspace}'s server couldn't be reached, so nothing was sent. Try again once Crew reconnects."),
        )
        .status(503);
        return error.context(refusal);
    }
    let Some(failure) = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<SshFailure>())
    else {
        return error;
    };
    let ssh_code = failure.api_code();
    let refusal = match lost {
        Lost::WhileCarrying if !is_read_only(method) => CrewRefusal::new(
            refusal::OUTCOME_UNKNOWN,
            format!(
                "Crew couldn't confirm whether this reached {workspace}. Check the channel, then retry with the same request ID."
            ),
        )
        .with("request_id", json!(request_key)),
        Lost::WhileCarrying => CrewRefusal::new(
            refusal::NOT_SENT,
            format!(
                "The connection to {workspace} dropped before it answered. Nothing changed; try again."
            ),
        ),
        Lost::BeforeSending if failure.code == "ssh_sign_in_refused" => CrewRefusal::new(
            refusal::NOT_SENT,
            format!("{} Nothing was sent.", failure.description),
        ),
        Lost::BeforeSending => CrewRefusal::new(
            refusal::NOT_SENT,
            format!("Biorouter couldn't reach {workspace}, so nothing was sent."),
        ),
    };
    error.context(refusal.status(503).with("ssh_code", json!(ssh_code)))
}

/// Whether a grant stored under a session id is the grant of the chat that holds that id now
/// (SCOPE-BIND). See [`CrewManager::standing`].
enum Standing {
    /// No grant is this chat's: none was made, the one stored under its id was made to an
    /// earlier chat that has since been replaced under the same id (and has been pruned), or
    /// it was made to a chat under the same id in another session store ([`GrantStore`]).
    None,
    /// The chat's own grant: bound to its incarnation, or recorded before grants were bound.
    Own(Scope),
    /// A grant sits under the id, but this chat cannot be confirmed as the one it was made
    /// to: that chat is gone from this device, or its identity could not be read. It keeps
    /// every restriction and authorizes nothing; the text says why.
    Unconfirmed(Scope, &'static str),
    /// The saved registry cannot be read, and this chat is one it may restrict (DAEMON-6): it
    /// restricts and authorizes nothing, as an unconfirmed grant does, but there is no grant
    /// to show or revoke until the file can be read again. See `freshness.rs`.
    Unreadable(&'static str),
}

/// Whether this process resolves chats in the session store a grant's chat was saved in
/// ([`Scope::session_store`], CROSSCUT-8). Only that store can say the grant's chat is gone,
/// or that a chat holding its id is a later one: in any other store the id is one both
/// stores minted, and its absence or its holder says nothing about the grant's chat.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GrantStore {
    /// This process's own store.
    Here,
    /// Another store that shares this registry.
    Elsewhere,
    /// Not recorded: the grant predates recording it, so no process may forget or prune it
    /// on the strength of what its own store lacks.
    Unrecorded,
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
    /// Stopped grants that no chat's id holds any more, kept for their revocation (F3): see
    /// [`ReplacedGrant`]. Never authority: nothing reads them to decide what a chat may do.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    replaced: Vec<ReplacedGrant>,
    #[serde(default)]
    pending_device: Option<PreparedDevice>,
    #[serde(default)]
    completed_preparations: HashMap<String, String>,
}
impl Registry {
    /// Whether this profile holds no Crew identity at all: no connection, grant, stopped grant,
    /// prepared device or finished preparation. Only such a profile may have an encrypted vault
    /// set up, since keys already in the keyring are never silently replaced.
    fn holds_no_identity(&self) -> bool {
        self.connections.is_empty()
            && self.scopes.is_empty()
            && self.replaced.is_empty()
            && self.pending_device.is_none()
            && self.completed_preparations.is_empty()
    }
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
/// - a grant's binding to its chat and that chat's store, when this process bound a grant
///   recorded before either was kept ([`CrewManager::adopt_binding`]) and the file still has
///   it unbound;
/// - a stop: a grant this process expired stays expired even if its save failed, because
///   nothing may bring a revoked run back to life;
/// - what the workspace said about a stop ([`Revocation`]), so a confirmation heard here is
///   never replaced by an older "not yet confirmed";
/// - an earlier grant this process keeps for its revocation ([`ReplacedGrant`]) whose
///   revocation is still unconfirmed, while its connection is saved, so a re-read never drops
///   the record that makes the daemon ask the workspace again (F3).
///
/// Only for the same grant, matched by run: a grant the file no longer holds, or holds for a
/// newer run, is the file's to decide — except an unconfirmed earlier grant, which the file
/// may simply never have been told about. Carrying one another process has since had confirmed
/// and forgotten costs one more `run.revoke`, which the workspace answers the same way again.
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
        let Some(mine) = heard_here(here, session, &scope.run_id) else {
            continue;
        };
        scope.expired |= mine.expired;
        if scope.session_incarnation.is_none() {
            scope.session_incarnation = mine.session_incarnation;
        }
        if scope.session_store.is_none() {
            scope.session_store = mine.session_store.clone();
        }
        // What the workspace said about a stop, whichever process heard it: a confirmation
        // this process got (and failed to save) is not lost to the file's "not yet".
        if Revocation::rank(mine.revocation) > Revocation::rank(scope.revocation) {
            scope.revocation = mine.revocation;
        }
    }
    for kept in &mut theirs.replaced {
        if let Some(mine) = heard_here(here, &kept.session_id, &kept.scope.run_id) {
            if Revocation::rank(mine.revocation) > Revocation::rank(kept.scope.revocation) {
                kept.scope.revocation = mine.revocation;
            }
        }
    }
    for mine in &here.replaced {
        let run_id = &mine.scope.run_id;
        let recorded = theirs.scopes.values().any(|scope| &scope.run_id == run_id)
            || theirs
                .replaced
                .iter()
                .any(|kept| &kept.scope.run_id == run_id);
        let saved = theirs
            .connections
            .iter()
            .any(|connection| connection.id == mine.scope.connection_id);
        if !recorded && saved && mine.scope.revocation == Some(Revocation::Unconfirmed) {
            theirs.replaced.push(mine.clone());
        }
    }
}
/// Whether a chat's own grant still stands on this device, and its connection when it does:
/// not stopped here, admitted under today's institution policy, and made under the policy
/// epoch its connection holds now. Each refusal is the sentence the person reads. Shared by
/// every turn's check ([`CrewManager::check_tier`]) and the history-rewrite refusal
/// ([`CrewManager::history_rewrite_refusal`]), so the two can never word one grant
/// differently.
fn grant_stands<'a>(scope: &Scope, connection: Option<&'a Connection>) -> Result<&'a Connection> {
    ensure!(!scope.expired, scope.stopped_text());
    // A scope granted before institution policy existed never passed today's admission.
    ensure!(scope.institution_policy, GRANT_POLICY_CHANGED);
    let connection = connection.ok_or_else(|| anyhow::anyhow!("Crew connection was removed"))?;
    ensure!(scope.epoch == connection.policy_epoch, GRANT_POLICY_CHANGED);
    Ok(connection)
}
/// This process's record of `run_id`, the grant stored under `session`: that chat's grant, or
/// an earlier grant it keeps for its revocation ([`ReplacedGrant`]).
fn heard_here<'a>(here: &'a Registry, session: &str, run_id: &str) -> Option<&'a Scope> {
    here.scopes
        .get(session)
        .filter(|mine| mine.run_id == run_id)
        .or_else(|| {
            here.replaced
                .iter()
                .find(|kept| kept.session_id == session && kept.scope.run_id == run_id)
                .map(|kept| &kept.scope)
        })
}
/// One row of the grants list ([`CrewManager::session_grant_rows`]): the grant stored under
/// `session_id`. The HTTP grants list answers these rows, so this type is that wire shape.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
pub struct GrantRow {
    /// The chat or task the grant is stored under.
    pub session_id: String,
    pub run_id: String,
    pub connection_id: String,
    /// The channel the grant may post in.
    pub channel_id: String,
    /// Every channel the grant may read.
    pub source_channels: Vec<String>,
    /// The connection's policy epoch when the grant was made.
    pub policy_epoch: u64,
    /// Stopped on this device: revoked, ended, or its connection was removed.
    pub expired: bool,
    /// When the workspace ends the grant on its own, in Unix seconds; `null` for a grant
    /// recorded before that was kept.
    pub expires_at: Option<u64>,
    /// The names the person saw when granting (D14); `null` when none were recorded.
    pub labels: Option<AdmissionLabels>,
    /// Where a stopped grant stands with the workspace (F3, D-1); `null` for a live grant and
    /// for a stop whose standing is not known.
    pub revocation: Option<Revocation>,
    /// `false` only while the daemon is still asking the workspace to confirm a revocation,
    /// `true` once it has, and `null` for a live grant or a stop whose standing is not a
    /// revocation this device sent (see `revocation`).
    pub remote_revocation_confirmed: Option<bool>,
}

/// A connection's grants ([`CrewManager::session_grant_rows`]).
#[derive(Clone, Debug, Serialize)]
pub struct SessionGrants {
    /// The grant each chat or task holds.
    pub grants: Vec<GrantRow>,
    /// Earlier grants kept until the workspace confirms their revocation (F3).
    pub replaced_grants: Vec<GrantRow>,
}

fn grant_row(session: &str, scope: &Scope) -> GrantRow {
    let confirmed = match scope.revocation {
        Some(Revocation::Unconfirmed) => Some(false),
        Some(Revocation::Confirmed) => Some(true),
        Some(Revocation::EndedByWorkspace) | None => None,
    };
    GrantRow {
        session_id: session.to_owned(),
        run_id: scope.run_id.clone(),
        connection_id: scope.connection_id.clone(),
        channel_id: scope.channel_id.clone(),
        source_channels: scope.source_channels.clone(),
        policy_epoch: scope.epoch,
        expired: scope.expired,
        expires_at: scope.expires_at,
        labels: scope.labels.clone(),
        revocation: scope.revocation,
        remote_revocation_confirmed: confirmed,
    }
}
/// A device key prepared for a new connection (`POST /crew/devices/prepare`). Save the
/// connection with its `preparation_id`.
#[derive(Clone, Debug, Deserialize, Serialize, utoipa::ToSchema)]
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
    /// When the daemon last dialled each connection by itself (a re-dial, never a person's
    /// Connect), by the wall clock, so a bridge that breaks again right after it is not dialled
    /// at once a second time (W2-DMN-6); see `keepalive.rs`.
    own_dials: StdMutex<HashMap<String, std::time::SystemTime>>,
    /// When the armed re-dial schedule of each connection (by its token) dials next, by the wall
    /// clock, so a request refused meanwhile can say how long the wait is (T3-BE-16).
    next_redial: StdMutex<HashMap<String, (u64, std::time::SystemTime)>>,
    /// Connections whose device the workspace accepted a person-signed request from in this
    /// process (Q3-12). Only these can have a membership that *ended*: a device the workspace
    /// never knew is one still joining, which keeps its bridge (see `keepalive.rs`).
    members: StdMutex<std::collections::HashSet<String>>,
    /// The typed code beside a connection's `last_error`, with the text it was set with, so
    /// it never outlives that text: process state, like `status` and `last_error` (D8).
    error_codes: StdMutex<HashMap<String, (&'static str, String)>>,
    /// What each chat's Crew requests read (Q3-02), for the provenance line a task's result
    /// ends with. Memory only; see [`RunReads`].
    run_reads: StdMutex<HashMap<String, RunReads>>,
    /// Connections whose unconfirmed revocations the daemon is asking the workspace about
    /// again (F3), each with the token of the pass doing it; see `revocation.rs`.
    revocation_retries: StdMutex<HashMap<String, u64>>,
    /// Each chat's last live admission by the workspace (CROSSCUT-7), which
    /// [`CrewManager::check_provider_use`] may reuse for [`LIVE_ADMISSION_REUSE`]. Memory only.
    live_admissions: StdMutex<HashMap<String, LiveAdmission>>,
    /// The saved registry's file as this process last read it (CROSSCUT-1): `None` before it
    /// looked, `Some(None)` for no file. See `freshness.rs`.
    seen_file: StdMutex<Option<Option<freshness::FileStamp>>>,
    /// What this process could not read of the saved registry, while it cannot (DAEMON-6).
    unreadable: StdMutex<Option<freshness::Unreadable>>,
    /// Runs whose own task ended them in this process: the workspace was sent the task's
    /// terminal result, which ends the run there (W2-DMN-14). Memory only, and bounded.
    ended_runs: StdMutex<HashSet<String>>,
    /// Runs whose `run.revoke` is on its way to the workspace now, so the retry pass never
    /// sends a second one beside it (W2-DMN-13). Memory only.
    revoking: StdMutex<HashSet<String>>,
}
/// How long one live admission of a chat's grant by the workspace stands for the provider
/// uses that are not the reply loop's own model requests ([`CrewManager::check_provider_use`]).
const LIVE_ADMISSION_REUSE: std::time::Duration = std::time::Duration::from_secs(15);
/// Whether a provider check must ask the workspace now, or may reuse a recent admission.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Admission {
    Fresh,
    Recent,
}
/// One live admission of a chat's grant: the grant and provider it was for, and when.
struct LiveAdmission {
    /// The run, the chat it was made to, the policy epoch and the provider's identity: a
    /// re-grant, a settings change or another model is never admitted by an earlier answer.
    grant: (String, Option<i64>, u64, String),
    at: std::time::Instant,
}
impl LiveAdmission {
    fn of(scope: &Scope, provider: &dyn Provider) -> Self {
        Self {
            grant: (
                scope.run_id.clone(),
                scope.session_incarnation,
                scope.epoch,
                provider_binding(provider),
            ),
            at: std::time::Instant::now(),
        }
    }
}
/// A save refused because `other`, another connection on this computer to the same
/// workspace, is under another institution than `given` (W2-DMN-9: typed, with both named).
fn other_institution_refusal(other: &Connection, given: &str) -> anyhow::Error {
    let theirs = other.institution_id.as_deref().unwrap_or_default();
    CrewRefusal::new(
        refusal::INSTITUTION_MISMATCH,
        format!(
            "{name} is also saved on this computer for the same workspace, under institution \
             {theirs}. Connections to one workspace share one institution, so remove {name} \
             before you use {given} here.",
            name = other.name,
        ),
    )
    .with("connection", json!(other.name))
    .with("connection_institution", json!(theirs))
    .with("institution", json!(given))
    .into()
}
/// Whether saving `input` over `saved` keeps the route to the workspace: the same login,
/// server, port, key, jump host, socket, owner and pinned workspace. What else may change (the
/// name, the privacy mode, the institution, the remote folder) does not change where a bridge
/// goes or what it verifies (W2-DMN-8).
fn same_route(saved: &Connection, input: &SaveConnection) -> bool {
    saved.ssh_target == input.ssh_target
        && saved.port == input.port
        && saved.identity_file == input.identity_file
        && saved.proxy_jump == input.proxy_jump
        && saved.socket_path == input.socket_path
        && saved.owner_uid == input.owner_uid
        && saved.workspace_id == input.workspace_id
        && saved.workspace_public_key == input.workspace_public_key
}
/// The interactive sign-in's `ssh` arguments: a control master that keeps the session open
/// for the bridge (`-M -N`, ControlPersist 600) over the same hardening every Crew `ssh`
/// carries.
///
/// It is the one Crew `ssh` a person answers, so it passes `BatchMode=no` explicitly: a
/// `BatchMode yes` in the person's own ssh configuration otherwise suppresses the password
/// and verification-code prompts, and the Sign in window shows only "Permission denied"
/// (W2-DMN-4). Every unattended `ssh` (the bridge, probes, host start) keeps `BatchMode=yes`.
/// A `-o` option reaches only the destination, not a ProxyJump hop, so a jump host that
/// needs a password stays unpromptable here.
pub(super) fn sign_in_args(c: &Connection, control: &Path) -> Vec<String> {
    let mut args = transport::ssh_args(c, control);
    args.extend([
        "-M".into(),
        "-N".into(),
        "-o".into(),
        "ControlPersist=600".into(),
        "-o".into(),
        "BatchMode=no".into(),
        c.ssh_target.clone(),
    ]);
    args
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
            session_store: None,
            revocation: None,
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
/// The model boundary a Crew grant is bound to: the provider's secret-free route and model
/// identity (what [`Provider::restore_binding`] rebuilds it from), its tier and its affiliation.
///
/// ⚠ **Crew provider binding; a change here needs human review.** The identity leaves out
/// what a provider carries per turn or learns at run time and is not a model boundary: the
/// context window (Llama Server reads the loaded model's real window back after its first
/// request, PROVIDERS-1), the sampling settings and the reasoning effort (Quick and Deep
/// rebuild the provider with `reasoning_effort` and, for Quick, a temperature,
/// PROVIDERS-2), and the output cap. Hashing them made a granted chat refuse every turn as
/// "bound to another provider" once any of them moved, although every byte still went to the
/// same endpoint and model. Everything that does decide where the chat's context goes stays
/// in: the provider, its route (endpoint, deployment, region, command), the model name, the
/// fast and toolshim models, provider request parameters, the tier and the affiliation.
fn provider_binding(provider: &dyn Provider) -> String {
    let mut binding = provider.restore_binding();
    let model = binding.model_mut();
    model.context_limit = None;
    model.temperature = None;
    model.max_tokens = None;
    model.reasoning_effort = None;
    binding_digest(&binding, provider)
}
/// The binding a grant recorded before [`provider_binding`] left out per-turn state: the hash
/// of the whole restore binding. Such a grant is still honored for the same provider as it
/// stood (see [`binds`]); a new grant always records [`provider_binding`].
fn legacy_provider_binding(provider: &dyn Provider) -> String {
    binding_digest(&provider.restore_binding(), provider)
}
fn binding_digest(
    binding: &crate::providers::provider_binding::ProviderRestoreBinding,
    provider: &dyn Provider,
) -> String {
    let resolved = serde_json::to_vec(binding).expect("provider binding serializes");
    format!(
        "{}:{:?}:{:?}",
        hex(&Sha256::digest(resolved)),
        provider.tier(),
        provider.affiliation()
    )
}
/// Whether `provider` is the model boundary a grant recorded as `recorded`: its identity
/// ([`provider_binding`]), or — for a grant recorded before that identity existed — the whole
/// binding exactly as it was hashed then. Two different digests of SHA-256 never collide, so
/// accepting either never admits another provider.
fn binds(recorded: &str, provider: &dyn Provider) -> bool {
    recorded == provider_binding(provider) || recorded == legacy_provider_binding(provider)
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
            storage: None,
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
            storage: None,
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
/// What a public model is told when the chat asking for Crew access has used a private one: its
/// history carries private context a public model must not continue with (W2-DMN-9).
const ORIGIN_RESTRICTED_PUBLIC: &str = "This chat has used a private model, so a public model can't continue it with Crew context. Choose a private model.";

/// What `crew_credential_store_unavailable` says to a profile that holds no Crew identity yet,
/// where an encrypted vault can still be set up (W2-DMN-1).
pub const CREDENTIAL_STORE_UNAVAILABLE_TEXT: &str = "This computer has no keyring service Biorouter can use. Run `biorouter crew credentials init` to keep Crew keys in an encrypted vault, then try again.";
/// What `crew_credential_store_unavailable` says to a profile whose Crew identities keep their
/// keys in the keyring: a vault can't be set up over them (`credentials init` refuses), so the
/// keyring has to answer again.
pub const KEYRING_NOT_RUNNING_TEXT: &str = "This computer's keyring service isn't answering, and Biorouter keeps this profile's Crew keys there. Start it (for example, by signing in to this computer's desktop), then try again.";
/// What `crew_credential_store_refused` says: the keyring is there and did not let Biorouter
/// use a Crew key (denied at its prompt, locked, or no session to ask in).
pub const CREDENTIAL_STORE_REFUSED_TEXT: &str = if cfg!(target_os = "macos") {
    "macOS Keychain didn't let Biorouter use this computer's Crew keys. Allow access when Keychain asks, or unlock your login keychain, then try again."
} else if cfg!(target_os = "windows") {
    "Windows Credential Manager didn't let Biorouter use this computer's Crew keys. Allow access if Windows asks, then try again."
} else {
    "This computer's keyring is locked or didn't let Biorouter use its Crew keys. Unlock it, or allow access when it asks, then try again."
};

/// Whether this platform's credential store is always there: the macOS Keychain and the
/// Windows Credential Manager are. Only a Secret Service (Linux and the other Unixes) can be
/// missing, on a headless node or in an SSH-only session.
const PLATFORM_STORE_ALWAYS_PRESENT: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// What a keyring failure says about the store behind it (W2-DMN-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyringTrouble {
    /// No keyring service answers on this computer.
    Absent,
    /// The keyring is there and refused: locked, a prompt denied or dismissed, no session to
    /// ask in.
    Refused,
}

impl KeyringTrouble {
    /// The refusal a person can act on. An absent keyring never gets the vault's advice here:
    /// only the first key of a profile that holds no identity may ([`first_key_failure`]),
    /// because `credentials init` refuses any other.
    fn refusal(self) -> CrewRefusal {
        match self {
            Self::Absent => CrewRefusal::new(
                refusal::CREDENTIAL_STORE_UNAVAILABLE,
                KEYRING_NOT_RUNNING_TEXT,
            ),
            Self::Refused => CrewRefusal::new(
                refusal::CREDENTIAL_STORE_REFUSED,
                CREDENTIAL_STORE_REFUSED_TEXT,
            ),
        }
    }
}

/// What `error` says about the keyring, when it says anything. Where the store is always
/// there (`store_always_present`), a failure is a refusal and never an absence: keyring maps
/// the Keychain's errSecUserCanceled, errSecAuthFailed and errSecInteractionNotAllowed (Deny at
/// its prompt, or no session to show one in) to `PlatformFailure`. A Secret Service that is
/// locked or whose prompt was dismissed answers `NoStorageAccess`, a refusal too. A
/// Secret Service `PlatformFailure` is an absence only when `service_absent` confirms it (a
/// read of an entry Crew never writes fails the same way); otherwise it is not about the store
/// being there, and keeps its own words, as any other keyring error does.
fn keyring_trouble(
    error: &keyring::Error,
    store_always_present: bool,
    service_absent: impl FnOnce() -> bool,
) -> Option<KeyringTrouble> {
    match error {
        keyring::Error::NoStorageAccess(_) => Some(KeyringTrouble::Refused),
        keyring::Error::PlatformFailure(_) if store_always_present => Some(KeyringTrouble::Refused),
        keyring::Error::PlatformFailure(_) => service_absent().then_some(KeyringTrouble::Absent),
        _ => None,
    }
}

/// A keyring error as the refusal a person can act on (W2-DMN-1). A store that does not answer
/// (no Secret Service on a headless Linux node: "The name is not activatable") fails every
/// Crew key the same way, and "try again" never helps; a store that refused says so, and that
/// allowing access or unlocking it will. Crew never falls back to plaintext for a device key.
/// Any other keyring error keeps its own words.
fn keyring_failure(error: keyring::Error) -> anyhow::Error {
    let trouble = keyring_trouble(&error, PLATFORM_STORE_ALWAYS_PRESENT, || {
        keyring_service_absent(keyring::Entry::new(CREDENTIAL_SERVICE, KEYRING_PROBE))
    });
    match trouble {
        Some(trouble) => {
            tracing::warn!(cause = %error, ?trouble, "Crew can't use this computer's keyring");
            trouble.refusal().into()
        }
        None => error.into(),
    }
}

/// What a refused keyring adds for a profile that holds no identity yet (W2-DMN-1).
pub const VAULT_INSTEAD_TEXT: &str =
    "Or run `biorouter crew credentials init` to keep Crew keys in an encrypted vault instead.";

/// `error`, from saving the first Crew key of a profile: while `registry` holds no identity at
/// all, the encrypted vault that `biorouter crew credentials init` can still set up here is
/// offered, as the way out of a keyring with no service behind it and as the other way out of
/// one that refused (W2-DMN-1). Once any identity exists that command refuses, so the vault is
/// named nowhere else.
fn first_key_failure(error: anyhow::Error, registry: &Registry) -> anyhow::Error {
    if !registry.holds_no_identity() {
        return error;
    }
    match CrewRefusal::find(&error).map(CrewRefusal::code) {
        Some(refusal::CREDENTIAL_STORE_UNAVAILABLE) => CrewRefusal::new(
            refusal::CREDENTIAL_STORE_UNAVAILABLE,
            CREDENTIAL_STORE_UNAVAILABLE_TEXT,
        )
        .into(),
        Some(refusal::CREDENTIAL_STORE_REFUSED) => CrewRefusal::new(
            refusal::CREDENTIAL_STORE_REFUSED,
            format!("{CREDENTIAL_STORE_REFUSED_TEXT} {VAULT_INSTEAD_TEXT}"),
        )
        .into(),
        _ => error,
    }
}

/// A keyring failure a test makes the manager rooted at each path meet on every key it saves,
/// in place of the platform store's answer (W2-DMN-1).
#[cfg(test)]
static TEST_KEYRING_TROUBLE: StdMutex<Vec<(PathBuf, KeyringTrouble)>> = StdMutex::new(Vec::new());

#[cfg(test)]
impl CrewManager {
    fn set_test_keyring_trouble(&self, trouble: Option<KeyringTrouble>) {
        let mut all = TEST_KEYRING_TROUBLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        all.retain(|(root, _)| *root != self.root);
        if let Some(trouble) = trouble {
            all.push((self.root.clone(), trouble));
        }
    }

    fn test_keyring_trouble(&self) -> Option<KeyringTrouble> {
        TEST_KEYRING_TROUBLE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|(root, _)| *root == self.root)
            .map(|(_, trouble)| *trouble)
    }
}

/// The keyring service every Crew key is kept under.
const CREDENTIAL_SERVICE: &str = "org.biorouter.crew";
/// An entry Crew never writes, read to learn whether the keyring answers.
const KEYRING_PROBE: &str = "keyring-probe";

/// Whether the OS keyring answers, by reading an entry Crew never writes. A working keyring
/// answers "no entry" (or, harmlessly, a value); one with no service behind it fails.
///
/// Only a Secret Service can be missing, so macOS and Windows are not asked: a read there can
/// raise a Keychain prompt for nothing, and their stores are always present.
fn keyring_answers(probe: Result<keyring::Entry>) -> bool {
    if PLATFORM_STORE_ALWAYS_PRESENT {
        return true;
    }
    let Ok(entry) = probe else {
        return false;
    };
    !matches!(
        entry.get_password(),
        Err(keyring::Error::PlatformFailure(_) | keyring::Error::NoStorageAccess(_))
    )
}

/// Whether no keyring service answers at all: a read of an entry Crew never writes fails as
/// the platform failing, not as the entry being missing or the store being locked. A probe that
/// cannot be built confirms nothing.
fn keyring_service_absent(probe: keyring::Result<keyring::Entry>) -> bool {
    probe.is_ok_and(|entry| {
        matches!(
            entry.get_password(),
            Err(keyring::Error::PlatformFailure(_))
        )
    })
}
fn file_credentials_enabled() -> bool {
    std::env::var("BIOROUTER_DISABLE_KEYRING").as_deref() == Ok("true")
        && std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT")
            .is_some_and(|p| PathBuf::from(p).is_absolute())
}
/// Whether `value` has the shape of a run ID: 1 to 128 ASCII letters, digits, `-` or `_`. The
/// broker mints UUIDs. A run ID travels as a path segment (the renderer's Stop, `POST
/// /crew/connections/{id}/runs/{run_id}/cancel`), so one the workspace answers with any other
/// shape is refused where it arrives, in `run.create`, before anything here records or shows
/// it: `../../credentials/lock?` would otherwise have sent the person's proof to another
/// daemon route (RENDERER-2).
pub fn is_run_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
/// Refuse a request that required the privacy mode `expected` (as the caller sent it) of a
/// connection that is in `actual`. A GUI's verified mode and a terminal's `--expected-mode`
/// both reach here, so the refusal names both modes rather than guessing that one changed
/// (W2-DMN-9). An absent `expected` requires nothing.
/// The privacy in force for a connection in `own` mode to a workspace in `workspace` mode
/// (T3-BE-5), the manual's definition and what every surface reports as the effective mode:
/// Private when either is, since a workspace that is Private for everyone restricts every
/// message whatever the connection's own mode (the broker's `message_restricted`); the
/// connection's own when the workspace's mode is not known.
fn effective_mode(own: ClusterMode, workspace: Option<ClusterMode>) -> ClusterMode {
    if own == ClusterMode::Private || workspace == Some(ClusterMode::Private) {
        ClusterMode::Private
    } else {
        own
    }
}
/// Whether a request that required `expected` may go ahead on a connection in `own` mode to a
/// workspace in `workspace` mode. It may when `expected` is the privacy in force
/// ([`effective_mode`], T3-BE-5: `--expected-mode private` from a personal Public connection in
/// a workspace that is Private for everyone used to be refused, although every surface said
/// Private), or the connection's own mode, which is what the desktop sends as the mode it last
/// verified. The one refused is the mismatch that matters: a request that required Private on
/// a connection that is Public in force, or the reverse; its refusal names the mode in force.
/// `None` (no expectation) always may. Pass `None` for `workspace` to hold the connection's own
/// mode alone.
fn require_mode(
    expected: Option<&Value>,
    own: ClusterMode,
    workspace: Option<ClusterMode>,
) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let expected: ClusterMode = serde_json::from_value(expected.clone())
        .map_err(|_| anyhow::anyhow!("A Crew privacy mode is either public or private"))?;
    let in_force = effective_mode(own, workspace);
    if expected == own || expected == in_force {
        return Ok(());
    }
    Err(CrewRefusal::mode_mismatch(in_force, expected).into())
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
        keyring::Entry::new(
            CREDENTIAL_SERVICE,
            &format!(
                "{}:{id}",
                hex(&Sha256::digest(self.root.to_string_lossy().as_bytes()))
            ),
        )
        .map_err(keyring_failure)
    }
    fn write_credential(&self, id: &str, value: &str) -> Result<()> {
        self.credential_vault
            .write(id, value, || self.write_legacy_credential(id, value))
    }
    fn write_legacy_credential(&self, id: &str, value: &str) -> Result<()> {
        #[cfg(test)]
        if let Some(trouble) = self.test_keyring_trouble() {
            return Err(trouble.refusal().into());
        }
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
            self.credential_entry(id)?
                .set_password(value)
                .map_err(keyring_failure)?;
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
                Err(error) => Err(keyring_failure(error)),
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
            self.credential_entry(id)?
                .get_password()
                .map_err(keyring_failure)
        }
    }
    /// Load the registry under `root`. One that cannot be read no longer fails the manager,
    /// and with it every chat that asks it anything (DAEMON-6): it restricts the chats it may
    /// name until it can be read again, and nothing is saved over it (see `freshness.rs`).
    pub fn new(root: PathBuf) -> Result<Self> {
        let freshness::Loaded {
            mut registry,
            saved_digest,
            seen_file,
            unreadable,
        } = freshness::load(&root.join("connections.json"))?;
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
            own_dials: StdMutex::new(HashMap::new()),
            next_redial: StdMutex::new(HashMap::new()),
            members: StdMutex::new(std::collections::HashSet::new()),
            error_codes: StdMutex::new(HashMap::new()),
            run_reads: StdMutex::new(HashMap::new()),
            revocation_retries: StdMutex::new(HashMap::new()),
            live_admissions: StdMutex::new(HashMap::new()),
            seen_file: StdMutex::new(Some(seen_file)),
            unreadable: StdMutex::new(unreadable),
            ended_runs: StdMutex::new(HashSet::new()),
            revoking: StdMutex::new(HashSet::new()),
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
    /// The workspace's own privacy mode, as its last `hello` signed it (v2); `None` when no
    /// signed `hello` said (not connected, or an older broker).
    fn signed_workspace_mode(&self, id: &str) -> Option<ClusterMode> {
        self.broker_hello(id).and_then(|hello| hello.mode)
    }
    /// Whether `connection`'s workspace server has stopped saving changes, as its last verified
    /// `hello` said (T3-BE-13). Only while it is connected: a `hello` from before a drop says
    /// nothing about the server now, so a disconnected connection is unknown (`None`). Every
    /// verified `hello` replaces the last (a connect, a heartbeat, a refresh after a change the
    /// server refused for its storage), so a server that saves again clears it.
    pub fn server_storage(&self, connection: &Connection) -> Option<ServerStorage> {
        if connection.status != "connected" {
            return None;
        }
        self.broker_hello(&connection.id)?.storage
    }
    /// A request the workspace refused because its server could not save it (`storage_full`,
    /// `storage_failed`): ask `hello` again, in the background, so the connection says whether
    /// the server has stopped saving (T3-BE-13). The broker also refuses one attachment with
    /// the same codes when only that file could not be written and it keeps saving; its `hello`
    /// tells the two apart, never the refusal's words. A manager built without
    /// [`CrewManager::shared`] asks nothing.
    fn heed_storage_refusal(&self, id: &str, answer: &Result<Value>) {
        let Err(error) = answer else {
            return;
        };
        if !keepalive::broker_refusal(error)
            .is_some_and(|(code, _)| matches!(code.as_str(), "storage_full" | "storage_failed"))
        {
            return;
        }
        let Some(manager) = self.this.get().cloned() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let id = id.to_owned();
        runtime.spawn(async move {
            let Some(manager) = manager.upgrade() else {
                return;
            };
            if let Err(error) = manager.refresh_broker_hello(&id).await {
                tracing::debug!(connection = %id, error = %error, "Couldn't ask a Crew workspace whether it still saves changes");
            }
        });
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
        let probe = self.credential_entry(KEYRING_PROBE);
        tokio::task::spawn_blocking(move || vault.status(|| keyring_answers(probe))).await?
    }
    pub async fn init_vault(&self, passphrase: zeroize::Zeroizing<String>) -> Result<()> {
        // A registry that cannot be read is not an empty one: the identities it names may hold
        // keyring credentials a new vault would shadow (DAEMON-6).
        self.refresh_registry().await;
        if self.unreadable_sessions().is_some() {
            return Err(CrewRefusal::new(
                refusal::REGISTRY_UNREADABLE,
                freshness::UNREADABLE_VAULT,
            )
            .status(409)
            .into());
        }
        ensure!(!file_credentials_enabled(), "Encrypted vault initialization requires a production credential profile, not the development plaintext backend");
        let registry = self.registry.lock().await;
        ensure!(registry.holds_no_identity(), "Initialize an encrypted vault in a fresh Crew profile before creating identities; existing keyring credentials are never silently replaced");
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
    ///
    /// A stopped grant (`expired`) says where it stands with the workspace: `revocation` is
    /// `unconfirmed` (stopped here; the daemon keeps asking the workspace to confirm),
    /// `confirmed`, `ended_by_workspace` (the workspace itself refused the run, D-1), or
    /// `null` when not known; `remote_revocation_confirmed` is the same as a boolean for the
    /// first two and `null` otherwise.
    ///
    /// `replaced_grants` lists, in the same shape, the earlier grants on the connection that no
    /// chat's id holds any more but whose revocation is kept (F3, [`ReplacedGrant`]): replaced
    /// by a new grant to the same chat, or left by a deleted chat whose id another chat now
    /// holds. Each is stopped; `unconfirmed` ones are still being asked about. They are a list
    /// of their own, after every chat's current grant, so a reader that looks a chat up by its
    /// id in `grants` never finds one of them.
    pub async fn session_grants(&self, connection_id: &str) -> Result<Value> {
        Ok(json!(self.session_grant_rows(connection_id).await?))
    }
    /// The grants each chat and task holds on `connection_id`, and the earlier grants kept
    /// for their revocation, as the HTTP grants list answers them.
    pub async fn session_grant_rows(&self, connection_id: &str) -> Result<SessionGrants> {
        // As another process saved them (CROSSCUT-1).
        self.refresh_registry().await;
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
                grants.push(grant_row(&session, &scope));
            }
        }
        let replaced_grants: Vec<GrantRow> = self
            .registry
            .lock()
            .await
            .replaced
            .iter()
            .filter(|kept| kept.scope.connection_id == connection_id)
            .map(|kept| grant_row(&kept.session_id, &kept.scope))
            .collect();
        Ok(SessionGrants {
            grants,
            replaced_grants,
        })
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
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|error| Self::unreadable_save_error(&error))?,
            None => serde_json::to_value(Registry::default())?,
        };
        let registry = if digest == self.saved_digest() {
            here.clone()
        } else {
            let mut theirs: Registry = serde_json::from_value(saved.clone())
                .map_err(|error| Self::unreadable_save_error(&error))?;
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
        // The file was just read back whole (or is the copy this process last read whole).
        self.registry_readable_again();
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
        // As another process saved them (CROSSCUT-1).
        self.refresh_registry().await;
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
            self.write_credential(&format!("device:{preparation_id}"), &hex(&key.to_bytes()))
                .map_err(|error| first_key_failure(error, registry))?;
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
    /// Save `input` over the connection `id`. A save changes the connection's policy (its epoch
    /// moves, which ends every grant on it) and drops its bridge, so one that would change
    /// nothing is not a save at all (P-1): the connection is answered as it stands, still
    /// connected, and every grant keeps working. A refused edit changes nothing either.
    pub async fn update(&self, id: &str, input: SaveConnection) -> Result<Connection> {
        let _lifecycle = self.connection_guard(id).await?;
        Self::validate_connection(&input)?;
        if let Some(unchanged) = self.unchanged_by(id, &input).await? {
            return Ok(unchanged);
        }
        // P-1: a save that would be refused is refused while the bridge is still up.
        self.refusal_before_saving(id, &input).await?;
        // W2-DMN-8: a person's save of a connected connection that keeps its route (the same
        // login, server, socket and pinned workspace: a privacy, institution or name change)
        // reconnects it at once. The save still drops the bridge and moves the policy epoch,
        // which ends every grant; the new bridge is verified from scratch, and grants are not
        // revived. Nothing reconnected it before, and the desktop must not connect by itself,
        // so the workspace read "offline" until someone pressed Connect.
        let reconnect = self.transports.lock().await.contains_key(id)
            && self
                .connection(id)
                .await
                .is_ok_and(|saved| same_route(&saved, &input));
        self.disconnect_locked(id).await?;
        let saved = self.save_inner(Some(id), input).await?;
        if !reconnect || authentication::ensure_connect_available(id).is_err() {
            return Ok(saved);
        }
        match self.connect_locked(id).await {
            Ok(connected) => Ok(connected),
            Err(error) => {
                let message = error.to_string();
                tracing::info!(connection = id, %error, "Crew couldn't reconnect after saving a connection");
                {
                    let mut registry = self.registry.lock().await;
                    if let Some(c) = registry.connections.iter_mut().find(|c| c.id == id) {
                        c.status = "disconnected".into();
                        c.last_error = Some(message.clone());
                    }
                }
                self.note_error_code(id, &error, &message);
                self.connection(id).await
            }
        }
    }
    /// The refusal saving `input` over `id` would meet, if any, asked before anything changes:
    /// the save's own checks — an institution that does not normalize, a private connection
    /// with no institution, aliases whose institutions differ, a prepared identity on an
    /// edit — run against a copy of the saved registry as it is now (D8), which is then
    /// thrown away. Only [`Self::validate_connection`]'s refusals were asked early before, so
    /// these dropped a working bridge for an edit that changed nothing (P-1). Saving itself
    /// checks again, so another process's save in between is still judged by the save.
    async fn refusal_before_saving(&self, id: &str, input: &SaveConnection) -> Result<()> {
        let mut input = input.clone();
        input.institution_id = input
            .institution_id
            .as_deref()
            .map(institution::normalize)
            .transpose()?;
        self.update_registry(|registry| {
            let mut trial = registry.clone();
            self.save_edit(&mut trial, Some(id), input).map(drop)
        })
        .await
    }
    /// The connection `id` as it stands, when saving `input` over it would change nothing
    /// the save decides: every setting the person gave, the privacy mode and the institution
    /// (an omitted institution keeps the saved one, as a save does), and the server group the
    /// save would put it in. `None` when anything would change, and for input a save would
    /// refuse, which the save then refuses in its own words. Compared with the saved registry
    /// as it is now (D8), not this process's copy, so a change another process saved since is
    /// never mistaken for the setting it replaced; nothing is written.
    async fn unchanged_by(&self, id: &str, input: &SaveConnection) -> Result<Option<Connection>> {
        if input.preparation_id.is_some() {
            return Ok(None);
        }
        let institution_id = match input.institution_id.as_deref() {
            Some(given) => match institution::normalize(given) {
                Ok(normalized) => Some(normalized),
                Err(_) => return Ok(None),
            },
            None => None,
        };
        self.update_registry(|registry| {
            Ok(Self::unchanged_in(
                registry,
                id,
                input,
                institution_id.as_deref(),
            ))
        })
        .await
    }
    /// [`Self::unchanged_by`]'s comparison, against `registry`.
    fn unchanged_in(
        registry: &Registry,
        id: &str,
        input: &SaveConnection,
        institution_id: Option<&str>,
    ) -> Option<Connection> {
        let current = registry.connections.iter().find(|c| c.id == id)?;
        // The group a save would put it in: the first saved connection with this workspace
        // or SSH target, as `build_connection` finds it.
        let group = registry
            .connections
            .iter()
            .find(|c| c.workspace_id == input.workspace_id || c.ssh_target == input.ssh_target)
            .map(|c| c.cluster_connection_id.as_str());
        let unchanged = current.name == input.name
            && current.ssh_target == input.ssh_target
            && current.port == input.port
            && current.identity_file == input.identity_file
            && current.proxy_jump == input.proxy_jump
            && current.socket_path == input.socket_path
            && current.owner_uid == input.owner_uid
            && current.workspace_id == input.workspace_id
            && current.workspace_public_key == input.workspace_public_key
            && current.remote_root == input.remote_root
            && current.remote_execution == input.remote_execution
            && input
                .cluster_connection_id
                .as_deref()
                .is_none_or(|cluster| cluster == current.cluster_connection_id)
            && group == Some(current.cluster_connection_id.as_str())
            && current.mode == input.mode
            && institution_id.is_none_or(|given| current.institution_id.as_deref() == Some(given));
        unchanged.then(|| current.clone())
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
        r: &Registry,
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
            self.write_credential(&format!("device:{connection_id}"), &hex(&key.to_bytes()))
                .map_err(|error| first_key_failure(error, r))?;
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
            self.connection_device(r, &connection_id, old.as_ref(), prepared)?;
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
        // The connection being edited is judged by its new institution alone, as its mode is
        // above: its saved one is what the edit replaces. Counting it made every change of
        // institution a conflict with itself (DAEMON-3), refused as "aliases" there were none
        // of. Another connection to the same workspace still has to agree, and is named
        // when it does not.
        if let Some(given) = input.institution_id.as_deref() {
            if let Some(other) = r.connections.iter().find(|c| {
                c.cluster_connection_id == cluster
                    && Some(c.id.as_str()) != id
                    && c.institution_id
                        .as_deref()
                        .is_some_and(|theirs| theirs != given)
            }) {
                return Err(other_institution_refusal(other, given));
            }
        }
        let institution_id = institution::merge(
            r.connections
                .iter()
                .filter(|c| c.cluster_connection_id == cluster && Some(c.id.as_str()) != id)
                .filter_map(|c| c.institution_id.as_deref())
                .chain(input.institution_id.as_deref()),
        )?;
        // The workspace's own institution is fixed by its host, and admission refuses a
        // connection for another (T3-BE-4); a save that would set one is refused here instead,
        // when a signed `hello` from this workspace says which it is. One not known yet is
        // accepted as it always was.
        if let Some(given) = institution_id.as_deref() {
            if let Some((label, theirs)) = self.workspace_institution(
                r,
                &input.workspace_id,
                &input.workspace_public_key,
                given,
            ) {
                return Err(institution::save_mismatch(given, &label, &theirs));
            }
        }
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
    /// The workspace pinned as `workspace_id` and `workspace_public_key`, as a person calls it,
    /// and its institution, when a signed (v2) `hello` from it says the institution is another
    /// than `given`: the hello of any saved connection to that workspace this daemon has
    /// verified since it connected. `None` when none says, or all agree.
    fn workspace_institution(
        &self,
        r: &Registry,
        workspace_id: &str,
        workspace_public_key: &str,
        given: &str,
    ) -> Option<(String, String)> {
        r.connections
            .iter()
            .filter(|c| {
                c.workspace_id == workspace_id && c.workspace_public_key == workspace_public_key
            })
            .find_map(|c| {
                let hello = self
                    .broker_hello(&c.id)
                    .filter(|hello| hello.signature_version >= 2)?;
                let theirs = institution::normalize(hello.institution_id.as_deref()?).ok()?;
                (theirs != given).then(|| {
                    let label = hello
                        .workspace_name
                        .filter(|name| biorouter_crew::workspace_name_valid(name))
                        .unwrap_or_else(|| plain_label(&c.name));
                    (label, theirs)
                })
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
    /// Remove a saved connection: stop every grant on it and ask the workspace to revoke their
    /// runs, disconnect it, drop it from the registry (its grants stay, stopped), delete its
    /// grants' run credentials, then delete its device private key (T-50). The key goes last,
    /// so a registry write that fails never leaves a saved connection without its key; a key
    /// that is already gone is not an error.
    ///
    /// ⚠ **Grant and revocation state; needs human review.** Removing used to mark the grants
    /// expired here and delete the device key, the only key that can sign `run.revoke`, without
    /// asking the workspace anything (DAEMON-7). Every live run stayed honored at the workspace
    /// until it lapsed, remote jobs started under it included, its run credential stayed in the
    /// keychain or vault, and nothing on this device could revoke it or even list it. Now the
    /// runs are revoked first, over the bridge when it is up; removal never dials a connection
    /// that is down, so a run it cannot reach still ends only when it expires (an hour at most).
    pub async fn remove(&self, id: &str) -> Result<()> {
        // Before the lifecycle guard: a request whose bridge dropped dials again under that
        // guard, and this one would then wait on itself.
        self.revoke_before_removal(id).await;
        let _lifecycle = self.connection_guard(id).await?;
        self.disconnect_locked(id).await?;
        let (pending_preparation, sessions) = self
            .update_registry_keeping(|r| {
                r.connections.retain(|c| c.id != id);
                let mut sessions = Vec::new();
                for (session, scope) in r.scopes.iter_mut().filter(|(_, s)| s.connection_id == id) {
                    // One granted after the revocations above is stopped here all the same.
                    scope.expired = true;
                    sessions.push(session.clone());
                }
                // Nothing can ask the workspace about them any more: the device key goes next.
                r.replaced.retain(|kept| kept.scope.connection_id != id);
                // A prepared device not yet saved keeps its key under the same kind of ID; that
                // one belongs to the next save, not to this removal.
                Ok((
                    r.pending_device
                        .as_ref()
                        .is_some_and(|prepared| prepared.preparation_id == id),
                    sessions,
                ))
            })
            .await??;
        if pending_preparation {
            return Ok(());
        }
        // No grant here names these credentials' runs any more.
        for session in sessions {
            self.forget_run_reads(&session);
            self.forget_live_admission(&session);
            if let Err(error) = self.delete_credential(&format!("run:{session}")) {
                tracing::warn!(
                    connection = id,
                    session,
                    %error,
                    "couldn't delete the run credential of a removed Crew connection's grant"
                );
            }
        }
        self.delete_credential(&format!("device:{id}"))
            .map_err(|error| {
                error.context(
                    "The connection was removed, but its device key couldn't be deleted from this computer",
                )
            })
    }
    /// [`Self::remove`]'s first step: stop every grant on `id` here (saved, or held in memory
    /// when it cannot be), then, if its bridge is up, ask the workspace to revoke each run whose
    /// revocation is not yet confirmed, earlier grants kept for their revocation included.
    /// Best effort: removal goes on whatever the workspace answers.
    async fn revoke_before_removal(&self, id: &str) {
        let stopped = self
            .update_registry_keeping(|r| {
                for scope in r.scopes.values_mut().filter(|s| s.connection_id == id) {
                    *scope = scope.clone().into_stopped();
                }
                Ok(())
            })
            .await;
        if let Err(error) = stopped {
            // It holds in this process all the same (`update_registry_keeping`).
            tracing::warn!(connection = id, %error, "Couldn't save the stop of a removed Crew connection's grants");
        }
        if !self.transports.lock().await.contains_key(id) {
            return;
        }
        for (session, run_id) in self.unconfirmed_revocations(id).await {
            if let Err(error) = self.confirm_revocation(id, &session, &run_id).await {
                tracing::warn!(
                    connection = id,
                    session,
                    run_id,
                    %error,
                    "the workspace did not confirm revoking a removed Crew connection's run; it \
                     ends when it expires"
                );
            }
        }
    }
    pub async fn authentication_plan(&self, id: &str) -> Result<AuthenticationPlan> {
        let c = self.connection(id).await?;
        let args = sign_in_args(&c, &self.control_path(id)?);
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
        let broker = BrokerHello {
            storage: ServerStorage::from_hello(hello),
            ..broker
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
    /// A person's Connect. One that fails for a network reason keeps being tried by the daemon
    /// on the keepalive's schedule, so a network that comes back reconnects without a second
    /// click (Q4-01, [`Self::connect_failed`]); any other failure is final and left showing.
    pub async fn connect(&self, id: &str) -> Result<Connection> {
        let _lifecycle = self.connection_guard(id).await?;
        authentication::ensure_connect_available(id)?;
        let connected = self.connect_locked(id).await;
        if let Err(error) = &connected {
            self.connect_failed(id, error).await;
        }
        connected
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
        // Connected again, by whoever asked: no idle re-dial is still owed, and an ended
        // membership's code went with the error this connect cleared.
        self.disarm_idle_redial(id);
        self.clear_error_code(id);
        self.start_keepalive(id, &transport);
        // Whoever connected (a person, the keepalive's re-dial, a request finding its bridge
        // gone), a revocation the workspace has not confirmed is asked again now (F3).
        if !self.unconfirmed_revocations(id).await.is_empty() {
            self.schedule_revocation_retries(id);
        }
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
        let transport = self.live_transport(id).await?;
        let (answer, usable) = self.hello_over(id, &c, &pinned, &transport).await;
        if !usable {
            self.retire_broken_bridge(id, &transport, "hello", &answer)
                .await?;
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
            return Err(CrewRefusal::new(refusal::INSTITUTION_MISMATCH, refusal).into());
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
    /// Retire `failed`, a bridge that broke carrying `method`, whose answer was `answer`, while
    /// it is still `id`'s bridge, with [`bridge_loss_message`]. Dials nothing.
    async fn retire_failed_transport<T>(
        &self,
        id: &str,
        failed: &Arc<Mutex<transport::Transport>>,
        method: &str,
        answer: &Result<T>,
    ) -> Result<()> {
        // Callers release the transport mutex before taking lifecycle ownership.
        // Connect/update/remove hold this same guard while replacing publication.
        let _lifecycle = self.connection_guard(id).await?;
        self.retire_locked(id, failed, bridge_loss_message(method, answer))
            .await;
        Ok(())
    }
    /// [`Self::retire_failed_transport`] for a bridge that broke carrying `method`, whose
    /// answer was `answer`. While it was still `id`'s bridge, a network failure also dials the
    /// connection again, at once, never the request (Q4-01, W2-DMN-6, see
    /// [`Self::request_bridge_failed`]). Only a request whose outcome was lost leaves the
    /// "inspect any submitted operation" alarm ([`bridge_loss_message`]).
    async fn retire_broken_bridge<T>(
        &self,
        id: &str,
        failed: &Arc<Mutex<transport::Transport>>,
        method: &str,
        answer: &Result<T>,
    ) -> Result<()> {
        let _lifecycle = self.connection_guard(id).await?;
        let message = bridge_loss_message(method, answer);
        if self.retire_locked(id, failed, message).await {
            if let Err(error) = answer {
                self.request_bridge_failed(id, error);
            }
        }
        Ok(())
    }
    /// Unpublish `failed` if it is still `id`'s bridge, mark the connection disconnected with
    /// `message`, and end its `ssh`; `true` when it was. The caller holds the connection's
    /// lifecycle guard.
    pub(super) async fn retire_locked(
        &self,
        id: &str,
        failed: &Arc<Mutex<transport::Transport>>,
        message: &str,
    ) -> bool {
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
        let Some(removed) = removed else {
            return false;
        };
        let mut registry = self.registry.lock().await;
        if let Some(connection) = registry.connections.iter_mut().find(|c| c.id == id) {
            connection.status = "disconnected".into();
            connection.last_error = Some(message.into());
        }
        drop(registry);
        removed.lock().await.close().await;
        true
    }
    async fn transport(&self, id: &str) -> Result<Arc<Mutex<transport::Transport>>> {
        if let Some(transport) = self.transports.lock().await.get(id).cloned() {
            return Ok(transport);
        }
        // A bridge that broke is being dialled again by itself: nothing needs signing in, and
        // the request can be sent again in a moment (W2-DMN-6).
        if self.redial_pending(id) {
            let workspace = self.workspace_label(id).await;
            return Err(CrewRefusal::new(
                refusal::RECONNECTING,
                reconnecting_sentence(&workspace, self.next_redial_in(id)),
            )
            .status(503)
            .with("workspace", json!(workspace))
            .into());
        }
        // Nothing is dialling it: a person connects it. Typed, so each client can say so in its
        // own words; the sentence stays the one older clients match (T3-BE-3).
        let workspace = self.workspace_label(id).await;
        Err(CrewRefusal::not_connected(&workspace).into())
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
            // The daemon's own check that the connection's mode did not move since admission,
            // which judged the person's expectation ([`Self::checked_run_admission`]).
            let expected = params.as_object_mut().unwrap().remove("expected_mode");
            require_mode(expected.as_ref(), c.mode, None)?;
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
            require_mode(
                params.get("personal_mode"),
                c.mode,
                self.signed_workspace_mode(id),
            )?;
            // The workspace combines the connection's own mode with its own; it is always told
            // the connection's.
            params["personal_mode"] = json!(c.mode);
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
        let transport = match self.live_transport(id).await {
            Ok(transport) => transport,
            Err(error) => {
                let workspace = self.workspace_label(id).await;
                return Err(lost_request(
                    error,
                    Lost::BeforeSending,
                    method,
                    None,
                    &workspace,
                ));
            }
        };
        let mut locked = transport.lock().await;
        let result = self
            .signed_exchange(&mut locked, &c, method, params, request_id, &signer)
            .await;
        let usable = locked.is_usable();
        drop(locked);
        if !usable {
            self.retire_broken_bridge(id, &transport, method, &result)
                .await?;
        }
        // Q3-12: whether the workspace still knows this device (see `keepalive.rs`).
        self.heed_membership(door == SignedDoor::Join, id, method, &result, &transport)
            .await;
        self.heed_storage_refusal(id, &result);
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
        let challenge = match t
            .request(
                "auth.challenge",
                json!({"device_id":c.device_id}),
                None,
                None,
                None,
            )
            .await
        {
            Ok(challenge) => challenge,
            // Only the challenge was lost: the request itself was never written (W2-DMN-7).
            Err(error) => {
                let workspace = self.workspace_label(id).await;
                return Err(lost_request(
                    error,
                    Lost::BeforeSending,
                    method,
                    None,
                    &workspace,
                ));
            }
        };
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
        let request_key = params["idempotency_key"].as_str().map(str::to_owned);
        match t
            .request(
                method,
                params,
                Some(json!({"device_id":c.device_id,"nonce":nonce,"signature":signature})),
                None,
                request_id,
            )
            .await
        {
            Ok(answer) => Ok(answer),
            Err(error) => {
                let workspace = self.workspace_label(id).await;
                Err(lost_request(
                    error,
                    Lost::WhileCarrying,
                    method,
                    request_key,
                    &workspace,
                ))
            }
        }
    }

    /// The run of the chat's grant, including one that is [`Standing::Unconfirmed`] — it is
    /// still the grant to show and to revoke — but never one made to an earlier chat under
    /// the same id.
    pub async fn run_metadata(&self, session: &str) -> Option<RunMetadata> {
        match self.standing(session).await {
            Standing::None | Standing::Unreadable(_) => None,
            Standing::Own(scope) | Standing::Unconfirmed(scope, _) => Some(RunMetadata {
                run_id: scope.run_id,
                connection_id: scope.connection_id,
                channel_id: scope.channel_id,
            }),
        }
    }
    /// Every chat a grant restricts: each id whose grant is its chat's own or cannot be
    /// confirmed, and none whose grant was made to an earlier chat under the same id.
    ///
    /// Read against the saved registry as it is now (CROSSCUT-1). While it cannot be read, the
    /// chats it names are listed too, and every saved chat when it names none this build can
    /// read (DAEMON-6).
    pub async fn scoped_session_ids(&self) -> std::collections::HashSet<String> {
        self.refresh_registry().await;
        let mut sessions: Vec<String> = self.registry.lock().await.scopes.keys().cloned().collect();
        match self.unreadable_sessions() {
            None => {}
            Some(Some(named)) => sessions.extend(named),
            Some(None) => sessions.extend(self.every_chat().await),
        }
        sessions.sort();
        sessions.dedup();
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
            json!({"connections":[{"id":c.id,"name":c.name,"status":c.status,"mode":c.mode,"workspace_id":c.workspace_id,"destination_channel_id":s.channel_id,"source_channel_ids":s.source_channels,"labels":s.labels,"naming":NAMING_RULE,"context_discovery":"Use context.manifest with empty params for recent authorized selected-channel context. Search each relevant source_channel_id with messages.search using channel_id and query; history and search are per-channel.","remote_files_enabled":!s.public_provider && c.remote_root.is_some(),"remote_execution_enabled":!s.public_provider && c.remote_root.is_some() && c.remote_execution,"remote_path_base":"the granted SSH work directory, not the local task directory; supply relative paths"}]}),
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
            Standing::Unconfirmed(_, reason) | Standing::Unreadable(reason) => {
                Err(anyhow::anyhow!(reason))
            }
            Standing::None => Err(anyhow::anyhow!(NO_GRANT)),
        }
    }
    /// The chat's own grant as the registry holds it now, with its connection (if it still
    /// exists), read under one lock so a check sees one snapshot. `None` when no grant is
    /// the chat's.
    async fn checked_scope(&self, session: &str) -> Result<Option<(Scope, Option<Connection>)>> {
        let own = match self.standing(session).await {
            Standing::None => return Ok(None),
            Standing::Unconfirmed(_, reason) | Standing::Unreadable(reason) => {
                anyhow::bail!(reason)
            }
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
        let c = grant_stands(&s, c.as_ref())?;
        if tier == ProviderTier::Public
            && !(c.mode == ClusterMode::Public && s.public_provider && !s.origin_restricted)
        {
            return Err(CrewRefusal::public_model(
                "This chat's Crew context is private, so a public model can't read it.",
            )
            .into());
        }
        institution::check_provider(tier, affiliation, &s.institution_ids)?;
        Ok(())
    }
    /// Why `session`'s stored history must not be rewritten or resent under its Crew grant, or
    /// `None` when no grant restricts the chat or its grant still stands (revoke F1, defense in
    /// depth). The reason is the sentence the chat's next turn is refused with: removed, ended
    /// by the workspace or by a settings change (D-1), or run out of time.
    ///
    /// A door that truncates or replaces a chat's history and then starts a turn — Edit in
    /// place, `/reply`'s `conversation_so_far` — asks this BEFORE it changes anything. The turn
    /// is refused at dispatch (`check_tier`) whatever the door does, so a rewrite let
    /// through would destroy history for a turn that can never run: measured at 20 and 24
    /// stored rows by the final acceptance. The renderer holds those paths first; this is the
    /// daemon's own answer for a client that does not, such as another window that has not
    /// re-read the grant since the CLI revoked it.
    ///
    /// Local only: it never asks the workspace. A run the workspace ended that this device has
    /// not yet heard about (D-1) passes here and is refused at the turn, as before. A run whose
    /// recorded end (`expires_at`) has passed on this device's clock is refused here, as the
    /// chat already holds it, although a turn leaves that decision to the workspace: refusing
    /// a rewrite changes nothing, so erring towards it costs nothing.
    pub async fn history_rewrite_refusal(&self, session: &str) -> Option<String> {
        let (scope, connection) = match self.checked_scope(session).await {
            Ok(None) => return None,
            Ok(Some(found)) => found,
            // A grant that cannot be confirmed as this chat's, or that changed under the read:
            // it restricts either way, and its text says why.
            Err(error) => return Some(error.to_string()),
        };
        if let Err(error) = grant_stands(&scope, connection.as_ref()) {
            return Some(error.to_string());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        if scope.expires_at.is_some_and(|end| end <= now) {
            return Some(GRANT_TIMED_OUT.to_owned());
        }
        None
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
        // Before the tier: a chat's model is fixed by its grant whatever tier the new one has,
        // and that is the one sentence a person can act on (W2-DMN-10).
        if !binds(&scope.provider_binding, provider) {
            return Err(CrewRefusal::model_fixed().into());
        }
        let connection =
            connection.ok_or_else(|| anyhow::anyhow!("Crew connection was removed"))?;
        if provider.tier() == ProviderTier::Public
            && !(connection.mode == ClusterMode::Public
                && scope.public_provider
                && !scope.origin_restricted)
        {
            return Err(CrewRefusal::public_model(
                "This chat's Crew context is private, so a public model can't read it.",
            )
            .into());
        }
        institution::check_provider(
            provider.tier(),
            provider.affiliation(),
            &scope.institution_ids,
        )?;
        Ok(())
    }
    /// Every check a model request on `session` must pass: the binding, the tier, and — for a
    /// scoped chat — a live `context.manifest` through the worker path, asked now whatever was
    /// asked before. The reply loop calls this before every request it sends; everything else
    /// calls [`Self::check_provider_use`].
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
        Box::pin(self.check_provider_dispatch_inner(session, provider, Admission::Fresh)).await
    }
    /// [`Self::check_provider_dispatch`] for a use of the provider that is not the reply loop's
    /// own model request: `Agent::provider`, which the loop and its helpers read many times a
    /// turn (to name the model, count tokens, or run a title, a summary or a stall check).
    ///
    /// The binding and the tier are checked on every call, exactly as the fresh check does,
    /// so a grant stopped, re-granted or re-bound on this device is refused at once. Only the
    /// live `context.manifest` is shared: when this grant, with this provider, was admitted by
    /// the workspace within [`LIVE_ADMISSION_REUSE`], that admission stands for this call too.
    /// Each call used to send its own, several per model request and one per streamed chunk
    /// (CROSSCUT-7, PROVIDERS-3): hundreds of SSH round trips for one answer, and a bridge
    /// hiccup on any of them aborted a turn that was already answering.
    ///
    /// ⚠ **Crew admission; a change here needs human review.** What the reuse can miss is a
    /// run the workspace ended in the last [`LIVE_ADMISSION_REUSE`] that this device has not
    /// yet heard about, and only for a request outside the reply loop, sent to the model this
    /// chat's context was already admitted to. The loop's next request asks the workspace
    /// again, and a refusal it answers ends the grant here ([`Self::heed_worker_refusal`]).
    pub async fn check_provider_use(&self, session: &str, provider: &dyn Provider) -> Result<()> {
        Box::pin(self.check_provider_dispatch_inner(session, provider, Admission::Recent)).await
    }
    async fn check_provider_dispatch_inner(
        &self,
        session: &str,
        provider: &dyn Provider,
        admission: Admission,
    ) -> Result<()> {
        self.check_provider_binding(session, provider).await?;
        self.check_tier(session, provider.tier(), provider.affiliation())
            .await?;
        let scope = match self.standing(session).await {
            Standing::None => return Ok(()),
            // The checks above refuse such a grant first; this keeps it refused if it
            // changed in between.
            Standing::Unconfirmed(_, reason) | Standing::Unreadable(reason) => {
                anyhow::bail!(reason)
            }
            Standing::Own(scope) => scope,
        };
        let admitted = LiveAdmission::of(&scope, provider);
        if admission == Admission::Recent && self.admitted_recently(session, &admitted) {
            return Ok(());
        }
        self.forget_live_admission(session);
        // A turn refused here is shown to the person as it stands ("Model request
        // failed"), in the chat and in the terminal: a refusal the workspace answered is
        // said as a sentence, never as the broker's envelope (D-1).
        self.worker_request(session, "context.manifest", json!({}))
            .await
            .map_err(|error| {
                if refused_by_workspace(&error) {
                    anyhow::anyhow!(agent_error_text(&error))
                } else {
                    error
                }
            })?;
        self.live_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session.to_owned(), admitted);
        Ok(())
    }
    /// Whether the workspace admitted `admitted` — this grant, with this provider — for
    /// `session` within [`LIVE_ADMISSION_REUSE`].
    fn admitted_recently(&self, session: &str, admitted: &LiveAdmission) -> bool {
        self.live_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session)
            .is_some_and(|last| {
                last.grant == admitted.grant && last.at.elapsed() < LIVE_ADMISSION_REUSE
            })
    }
    /// Forget `session`'s last live admission, so its next provider use asks the workspace.
    fn forget_live_admission(&self, session: &str) {
        self.live_admissions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(session);
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
        require_mode(
            policy.expected_mode.map(|mode| json!(mode)).as_ref(),
            connection.mode,
            self.signed_workspace_mode(id),
        )?;
        ensure!(
            policy
                .expected_policy_epoch
                .is_none_or(|epoch| epoch == connection.policy_epoch),
            "Crew connection policy changed; refresh before granting agent access"
        );
        let public = provider.tier() == ProviderTier::Public;
        let workspace = self.workspace_label(id).await;
        if public && connection.mode != ClusterMode::Public {
            return Err(CrewRefusal::public_model(format!(
                "Your connection to {workspace} is Private, so a public model can't read it. \
                 Choose a private model."
            ))
            .with("workspace", json!(workspace))
            .into());
        }
        if public && policy.origin_restricted {
            return Err(CrewRefusal::public_model(ORIGIN_RESTRICTED_PUBLIC).into());
        }
        let snapshot = self
            .human_request(id, "workspace.snapshot", json!({}), None)
            .await?;
        let protected = institution::protected_sources(&snapshot, channel, sources, &workspace)?;
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
        if public && origin_restricted {
            return Err(CrewRefusal::public_model(ORIGIN_RESTRICTED_PUBLIC).into());
        }
        institution::check_origin(
            &institution_ids,
            admission.workspace_institution_id.as_deref(),
        )?;
        // Admission asked this of the same institutions, with the names a person needs beside
        // its refusal; this is the check again at the point of use.
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
                is_run_id(&run_id),
                "Crew workspace answered with a run ID this computer does not accept; the run was revoked"
            );
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
                session_store: self.own_store(),
                revocation: None,
            };
            // Recorded here even when the save fails, as it always was: the abandon below
            // then finds it and stops it here too. An unconfirmed stop of the chat's earlier
            // grant is kept and asked about again (F3).
            self.record_grant(session, scope).await?;
            // A new task starts from nothing read, even under a chat ID used before.
            self.forget_run_reads(session);
            let context = self
                .worker_request(
                    session,
                    "messages.history",
                    json!({"channel_id":channel,"limit":50,"latest":true}),
                )
                .await?;
            let shared_files = self.list_shared_files(session, &context).await;
            Ok(RunAdmission {
                run_id: run_id.clone(),
                institution_ids: institution_ids.clone(),
                expires_at,
                context: wrapper_safe_json(&serde_json::to_string(&admission_context(
                    AdmissionContext {
                        connection_id: id,
                        channel,
                        sources: &sources,
                        labels: &labels,
                        remote_files_enabled: !public && c.remote_root.is_some(),
                        shared_files,
                        history: context,
                    },
                ))?),
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
        let previous = match self.standing(session).await {
            Standing::None => return Ok(()),
            // Restrictions that cannot be read cannot be carried, so nothing is granted.
            Standing::Unreadable(reason) => anyhow::bail!(reason),
            Standing::Own(previous) | Standing::Unconfirmed(previous, _) => previous,
        };
        policy.origin_restricted |= previous.origin_restricted;
        policy
            .origin_institution_ids
            .extend(previous.institution_ids);
        ensure!(
            previous.connection_id == id && previous.channel_id == channel,
            "This chat already has Crew access to another channel. Start a new chat to give it access to this one."
        );
        if !binds(&previous.provider_binding, provider) {
            return Err(CrewRefusal::model_fixed().into());
        }
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
                "blob.status",
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
        // Typed, so a client can word an ended grant itself; the sentence is the one the chat
        // already reads (T3-BE-7).
        if s.expired {
            return Err(s.stopped().into());
        }
        let c = self.connection(&s.connection_id).await?;
        if s.epoch != c.policy_epoch {
            return Err(CrewRefusal::grant_ended(
                refusal::GRANT_ENDED_SETTINGS_CHANGED,
                GRANT_POLICY_CHANGED,
            )
            .into());
        }
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
        // As every request: a bridge that ended, or sat idle long enough for the broker to drop
        // it, is checked (and dialled again without a prompt) before anything is written.
        let transport = match self.live_transport(&s.connection_id).await {
            Ok(transport) => transport,
            Err(error) => {
                let workspace = self.workspace_label(&s.connection_id).await;
                return Err(lost_request(
                    error,
                    Lost::BeforeSending,
                    method,
                    None,
                    &workspace,
                ));
            }
        };
        let mut locked = transport.lock().await;
        self.validate_worker_scope(session, &s, &c).await?;
        let credential = self.read_credential(&format!("run:{session}"))?;
        let request_key = params["idempotency_key"].as_str().map(str::to_owned);
        // A task's result or failure, not a progress note: the workspace ends the run with it.
        let terminal_post = method == "run.project"
            && params["status"]
                .as_str()
                .is_some_and(|status| status != "progress");
        let result = locked
            .request(method, params, None, Some(&credential), None)
            .await;
        let usable = locked.is_usable();
        drop(locked);
        if !usable {
            self.retire_broken_bridge(&s.connection_id, &transport, method, &result)
                .await?;
        }
        self.heed_storage_refusal(&s.connection_id, &result);
        let mut result = match result {
            Ok(result) => result,
            Err(error) => {
                let workspace = self.workspace_label(&s.connection_id).await;
                let error =
                    lost_request(error, Lost::WhileCarrying, method, request_key, &workspace);
                return Err(self.heed_worker_refusal(session, &s, error).await);
            }
        };
        self.validate_worker_scope(session, &s, &c).await?;
        if terminal_post {
            self.note_run_ended(&s.run_id);
        }
        match method {
            "messages.history" | "messages.search" | "context.manifest" => {
                self.note_run_context(session, method, &result);
                mark_agent_posts(&mut result);
            }
            "blob.status" => self.note_run_status(session, &result),
            _ => {}
        }
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
    /// What a worker request the workspace refused becomes (D-1). A `grant_expired` refusal
    /// means the workspace no longer honors the run: its policy moved since the grant (another
    /// channel's membership changed, say), the run's context became protected, its institution
    /// changed, or the run was ended there. So the grant stops here too, recorded as ended by
    /// the workspace: lists read it stopped, the chat shows it, and every later request is
    /// refused here without asking. The person reads the sentence a policy change on this
    /// device gets, never the broker's envelope. A run that simply ran out of time is left as
    /// it is (its `expires_at` already says so) and gets its own sentence. Any other error is
    /// returned unchanged.
    async fn heed_worker_refusal(
        &self,
        session: &str,
        scope: &Scope,
        error: anyhow::Error,
    ) -> anyhow::Error {
        let Some((code, _)) = keepalive::broker_refusal(&error) else {
            return error;
        };
        if code != BROKER_GRANT_EXPIRED {
            return error;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        if scope.expires_at.is_some_and(|at| at <= now) {
            return CrewRefusal::grant_ended(refusal::GRANT_ENDED_ENDED, GRANT_TIMED_OUT).into();
        }
        // The run's own task ended it here (W2-DMN-14): a finished task's terminal post ends
        // the run at the workspace, and a request just behind it (background compaction, a
        // last tool call) meets `grant_expired`. That is not a policy change, and the grant's
        // own stop is already on its way, so nothing is stamped here.
        if self.run_ended_here(session, &scope.run_id).await {
            return CrewRefusal::grant_ended(refusal::GRANT_ENDED_ENDED, TASK_ENDED).into();
        }
        let run_id = scope.run_id.clone();
        let stopped = self
            .update_registry_keeping(|registry| {
                if let Some(current) = registry
                    .scopes
                    .get_mut(session)
                    .filter(|current| current.run_id == run_id)
                {
                    current.expired = true;
                    if current.revocation.is_none() {
                        current.revocation = Some(Revocation::EndedByWorkspace);
                    }
                }
                Ok(())
            })
            .await;
        if let Err(error) = stopped {
            // It holds in this process all the same (`update_registry_keeping`).
            tracing::warn!(session, %error, "Couldn't save a Crew grant the workspace ended");
        }
        self.forget_run_reads(session);
        self.forget_live_admission(session);
        CrewRefusal::grant_ended(refusal::GRANT_ENDED_SETTINGS_CHANGED, GRANT_POLICY_CHANGED).into()
    }
    /// Remember that `run_id`'s own task ended it (see [`Self::ended_runs`]).
    fn note_run_ended(&self, run_id: &str) {
        let mut ended = self
            .ended_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if ended.len() >= MAX_ENDED_RUNS {
            ended.clear();
        }
        ended.insert(run_id.to_owned());
    }
    /// Whether `session`'s run `run_id` was ended by its own task or stopped by this device:
    /// its terminal result was posted here, or the chat's grant for that run is stopped here
    /// (a person's Stop or Revoke marks it before the workspace is asked). A run the workspace
    /// ended by itself is neither.
    async fn run_ended_here(&self, session: &str, run_id: &str) -> bool {
        if self
            .ended_runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(run_id)
        {
            return true;
        }
        self.registry
            .lock()
            .await
            .scopes
            .get(session)
            .is_some_and(|scope| {
                scope.run_id == run_id
                    && matches!(
                        scope.revocation,
                        Some(Revocation::Unconfirmed | Revocation::Confirmed)
                    )
            })
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
        let (connection_id, run_id, confirmed) = self
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
                // Until the workspace confirms, the stop is recorded as not yet confirmed, so a
                // restart still knows to ask again (F3). A confirmation heard earlier stands.
                let confirmed = current.revocation == Some(Revocation::Confirmed);
                if !confirmed {
                    current.revocation = Some(Revocation::Unconfirmed);
                }
                Ok((
                    current.connection_id.clone(),
                    current.run_id.clone(),
                    confirmed,
                ))
            })
            .await
            .map_err(|error| {
                error.context("Couldn't save the revocation on this device; retry to finish it")
            })??;
        self.forget_run_reads(session);
        self.forget_live_admission(session);
        // The workspace already confirmed this run's revocation: asking again would only be
        // journalled there once more (W2-DMN-13), so the confirmation stands as the answer.
        if confirmed {
            return Ok(RevokeOutcome {
                remote_confirmed: true,
                run: Some(json!({"id": run_id, "revoked": true})),
                remote_error: None,
            });
        }
        Ok(
            match self
                .confirm_revocation(&connection_id, session, &run_id)
                .await
            {
                Ok(run) => RevokeOutcome {
                    remote_confirmed: true,
                    run: Some(run),
                    remote_error: None,
                },
                Err(error) => {
                    // Asked again by the daemon itself, with growing gaps while the connection is
                    // up, and at every reconnect after that, until the workspace confirms (F3). A
                    // refusal the workspace answered is asked again at the next reconnect only:
                    // asking now would only repeat its answer.
                    if !refused_by_workspace(&error) {
                        self.schedule_revocation_retries(&connection_id);
                    }
                    RevokeOutcome {
                        remote_confirmed: false,
                        run: None,
                        remote_error: Some(error),
                    }
                }
            },
        )
    }
    /// Whether the workspace has confirmed revoking `run_id`, the run of `session`'s stopped
    /// grant (F3). The daemon's own retry confirms without anyone pressing Retry, so a task's
    /// ledger reads this to follow it — including for a grant no chat's id holds any more,
    /// which is kept for this ([`ReplacedGrant`]).
    pub async fn remote_revocation_confirmed(&self, session: &str, run_id: &str) -> bool {
        heard_here(&*self.registry.lock().await, session, run_id)
            .is_some_and(|scope| scope.expired && scope.revocation == Some(Revocation::Confirmed))
    }
    /// Ask the workspace to revoke `run_id`, the run of `session`'s stopped grant, and record
    /// its confirmation on that grant while it is still the session's (F3). Revoking is
    /// idempotent at the workspace, so asking again after an answer that was lost is safe.
    pub(super) async fn confirm_revocation(
        &self,
        connection_id: &str,
        session: &str,
        run_id: &str,
    ) -> Result<Value> {
        let run = {
            let _in_flight = RevokeInFlight::begin(&self.revoking, run_id);
            self.human_request(connection_id, "run.revoke", json!({"run_id":run_id}), None)
                .await?
        };
        let recorded = self
            .update_registry_keeping(|registry| {
                if let Some(current) = registry
                    .scopes
                    .get_mut(session)
                    .filter(|current| current.run_id == run_id && current.expired)
                {
                    current.revocation = Some(Revocation::Confirmed);
                }
                registry.confirm_replaced(session, run_id);
                Ok(())
            })
            .await;
        if let Err(error) = recorded {
            // It holds in this process all the same, and the next save carries it.
            tracing::warn!(session, run_id, %error, "Couldn't save a confirmed Crew revocation");
        }
        Ok(run)
    }
    /// Post a connected chat's own update to its destination (`run.project`, always
    /// `progress`), ending with the daemon's line (W2-DMN-12): the files the chat read since its
    /// last post, or that it read none. Every post a chat makes goes through here, its own
    /// `run.project` and `remote.attach`'s alike, so none reaches the channel without the line,
    /// and a "Source:" line the model wrote (in the body, or as a file's name) is never the last
    /// thing in it. A post without a text body is refused here: it would carry no line.
    ///
    /// A request sent again under the same idempotency key carries the line it was first sent
    /// with ([`SentPost`]), so a retry after an uncertain answer is the identical request the
    /// workspace replays, not a different one it refuses.
    async fn post_from_chat(&self, session: &str, params: Value) -> Result<Value> {
        let mut params = params;
        ensure!(params.is_object(), "Crew params must be an object");
        ensure!(
            params.get("status").and_then(Value::as_str).is_none_or(|status| status == "progress"),
            "Agent updates must use progress; the task owner or runner controls completion and cancellation"
        );
        params["status"] = json!("progress");
        if params.get("idempotency_key").is_none_or(Value::is_null) {
            params["idempotency_key"] = json!(uuid::Uuid::new_v4().to_string());
        }
        let body = params
            .get("body")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow::anyhow!("A Crew post needs its text in body"))?;
        let request: [u8; 32] = Sha256::digest(serde_json::to_vec(&canonical(&params))?).into();
        let key = params["idempotency_key"].as_str().map(str::to_owned);
        let (line, mark) = self.chat_post_line(session, key.as_deref(), &request).await;
        params["body"] = json!(source_line::with_source_line(
            body,
            line,
            source_line::NO_FILE_READ_FOR_POST,
        ));
        let answer = self.worker_request(session, "run.project", params).await?;
        self.mark_reads_posted(session, mark);
        Ok(answer)
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
        // W2-DMN-12: the attachment's post is the chat's post like any other, so it ends with
        // the daemon's line. The name is the model's choice (a remote path's last part), so it
        // is written as a code span and can never pass for that line.
        self.post_from_chat(
            session,
            json!({
                "body": attached_body(name),
                "status": "progress",
                "attachments": [blob_id],
                "idempotency_key": format!("{key}:post"),
            }),
        )
        .await
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
            return self.post_from_chat(session, params).await;
        }
        if method == "remote.attach" {
            return self.attach_remote(session, params).await;
        }
        let params = with_default_channel(method, params, &s)?;
        if method == "blob.read" {
            // Q3-17: the broker requires `offset`, and a first read starts at 0. Every model
            // left it out, so every file read began with a refusal row.
            let mut params = params;
            ensure!(params.is_object(), "Crew params must be an object");
            if params.get("offset").is_none_or(Value::is_null) {
                params["offset"] = json!(0);
            }
            let read = self.worker_request(session, method, params).await?;
            // Q3-02: what was read is recorded before the model sees it, so the result's
            // provenance line names it whatever the model writes.
            self.note_blob_read(session, &read);
            return Ok(readable_blob(read));
        }
        self.worker_request(session, method, params).await
    }
}

/// What one chat's Crew requests read, for the line a task's posted result ends with (Q3-02):
/// the model's own reply named its file only when it happened to, and of two uploads with one
/// name it silently read the older. Recorded by the daemon from the broker's answers, so the
/// line is true whatever the model writes. Display only: it grants and checks nothing.
///
/// Cleared when a run is admitted, so a reused chat ID never lists an earlier task's files.
#[derive(Clone, Debug, Default)]
struct RunReads {
    /// Each file a `blob.read` returned, in the order first read, once each.
    files: Vec<ReadFile>,
    /// Files first read after [`MAX_READ_FILES`] were listed, at most [`MAX_READ_ATTACHMENTS`],
    /// so a task's line can say how many it leaves out instead of dropping them silently, and a
    /// chat post's line can name them from [`Self::named`] ([`Self::since_last_post`]).
    unlisted: HashSet<String>,
    /// The [`ReadMark`] of the latest read of each file in [`Self::files`] and
    /// [`Self::unlisted`]: a file read again is read again since the chat's last post, though
    /// it is listed once (W2-DMN-12).
    read_at: HashMap<String, ReadMark>,
    /// The mark of the latest read of a file past both bounds, which no entry above records.
    /// Such a read is still a read: the line counts it as at least one more file, and never
    /// says the chat read nothing (W2-DMN-12).
    unrecorded_at: Option<ReadMark>,
    /// The mark the chat's last post of its own was built at: its line named every read up to
    /// it. A chat post's line names what the chat read after it; a task's result names all.
    posted_through: ReadMark,
    /// A person label (D13) for each principal a message read named: the broker's `people`
    /// map beside `messages.history`, `messages.search` and `context.manifest`.
    people: HashMap<String, String>,
    /// When each attachment was shared, as far as the reads showed: the messages carrying it
    /// that no other message carrying it is known to postdate (one, unless the reads never
    /// ordered two posted in the same second). Empty when the reads showed it too often to
    /// keep, which makes its time unknown for good rather than guessed.
    shared_at: HashMap<String, Vec<Posted>>,
    /// Messages posted in the same second, in the order one page listed them, oldest first:
    /// for each second, one run of message IDs per page. `created_at` counts whole seconds and
    /// the broker's wire `sequence` is the message's opaque ID, so a page's own order is the
    /// only thing that tells two such messages apart. Orders from two pages are never merged:
    /// a pair no single page listed together stays unordered.
    same_second: HashMap<u64, Vec<Vec<String>>>,
    /// How many message IDs [`Self::same_second`] holds, against [`MAX_READ_ATTACHMENTS`].
    same_second_ids: usize,
    /// The broker's name and sharer for each complete attachment the daemon learned of: every
    /// file read, and every one `blob.status` named (the shared-file list given at admission,
    /// and the look-ups made before a result is posted). A copy the run did not read counts
    /// here, which is how the line knows a newer one was left unread.
    named: HashMap<String, ReadFile>,
    /// The lines the chat's own latest posts were first sent with, by idempotency key, oldest
    /// first, at most [`MAX_SENT_POSTS`] ([`SentPost`]).
    sent_posts: VecDeque<SentPost>,
}

/// The line a chat's post was first sent with under its idempotency key (W2-DMN-12). The
/// workspace answers a key it has already seen only for the identical request, so a post sent
/// again under the same key (a retry after an uncertain answer, or `remote.attach` asked again)
/// must carry the same line. One built again could differ: the chat may have read more since,
/// named a newer copy, or reached another day, and the retry would be refused as a different
/// request while the first post stood.
#[derive(Clone, Debug)]
struct SentPost {
    /// The post's idempotency key.
    key: String,
    /// The digest of the request as the chat made it, before the line was added. Another
    /// request under the same key gets a line of its own; the workspace refuses it anyway when
    /// the first one landed.
    request: [u8; 32],
    /// The line it was sent with; `None` when it said no file was read.
    line: Option<String>,
    /// The [`ReadMark`] that line was built at.
    mark: ReadMark,
}

/// How many of a chat's latest posts keep the line they were first sent with. A retry comes
/// right after the answer it retries; an older key sent again gets a line built anew.
const MAX_SENT_POSTS: usize = 16;

/// Where a `blob.read` falls among every read this process recorded: later reads have larger
/// marks, whichever chat made them. One counter for all chats, so a mark taken before a chat's
/// reads were forgotten (a new grant) never covers a read made after.
type ReadMark = u64;

/// The last [`ReadMark`] given out; the next read gets one more.
static READ_MARKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A mark for a read being recorded now.
fn next_read_mark() -> ReadMark {
    READ_MARKS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
}

/// The mark of the latest read recorded so far. Taken under `run_reads`' lock, where every
/// read is recorded, it covers exactly the reads that lock has seen.
fn current_read_mark() -> ReadMark {
    READ_MARKS.load(std::sync::atomic::Ordering::SeqCst)
}

/// One file a run read or the workspace named: the broker's own `blob` fields, never the
/// model's words.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReadFile {
    id: String,
    name: String,
    owner_id: String,
}

/// One file as the Source line names it (see [`RunReads::line_entry`]), already in Markdown.
struct LineEntry {
    name: String,
    copy: Option<String>,
    shared: Option<String>,
}

/// What [`CrewManager::begin_run_with_policy`] gives the model as `crew_context` when a run is
/// admitted.
struct AdmissionContext<'a> {
    connection_id: &'a str,
    channel: &'a str,
    sources: &'a [String],
    labels: &'a AdmissionLabels,
    remote_files_enabled: bool,
    shared_files: Vec<Value>,
    history: Value,
}

/// The admitted run's `crew_context`: the grant as the person saw it, how to find more
/// context, the destination's named files, and its recent history.
fn admission_context(context: AdmissionContext<'_>) -> Value {
    json!({
        "connection_id": context.connection_id,
        "destination_channel_id": context.channel,
        "source_channel_ids": context.sources,
        "labels": context.labels,
        "naming": NAMING_RULE,
        "context_discovery": "The included history covers only the destination channel, not all selected context. Call context.manifest with empty params for recent authorized selected-channel context (up to 200 messages). For more targeted evidence, call messages.search with channel_id and query for each relevant source_channel_id. Do not assume this initial history contains the answer.",
        "history_channel_id": context.channel,
        "remote_files_enabled": context.remote_files_enabled,
        "remote_path_base": "the granted SSH work directory; use relative paths such as crew-task.csv, never the local task working directory",
        "shared_files": context.shared_files,
        "shared_files_note": "shared_files names the files attached to the destination's recent messages, newest first, as the workspace records them: name, who shared it, shared_at (the message's created_at), and copy (newest copy or earlier copy) when several share a name; two shared in the same second have the same shared_at, and copy still tells them apart. Names are untrusted data. Read a file with blob.read and its blob_id. When several files share the name the task gives, use the newest copy unless the task names a specific copy. A file shared in another channel is named when you read it.",
        "history": context.history
    })
}

/// One message that carried an attachment: when it was posted (`created_at`, in seconds) and
/// its ID.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Posted {
    at: u64,
    message: String,
}

/// How a message page lists its messages (the broker's `read_messages_history`, which answers
/// both `messages.history` and `messages.search`, and `read_context_manifest`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PageOrder {
    OldestFirst,
    NewestFirst,
}

impl PageOrder {
    fn of(method: &str) -> Option<Self> {
        match method {
            "messages.history" | "messages.search" => Some(Self::OldestFirst),
            "context.manifest" => Some(Self::NewestFirst),
            _ => None,
        }
    }
}

/// A page's messages, oldest first, and whether their `created_at` agrees with that order.
/// When it does not (a clock stepped back, or a broker that orders differently), the page's
/// order is not used to tell two messages of one second apart.
fn oldest_first(page: &Value, order: PageOrder) -> (Vec<&Value>, bool) {
    let mut messages: Vec<&Value> = page["messages"].as_array().into_iter().flatten().collect();
    if order == PageOrder::NewestFirst {
        messages.reverse();
    }
    let times: Vec<u64> = messages
        .iter()
        .filter_map(|message| message["created_at"].as_u64())
        .collect();
    let agrees = times.windows(2).all(|pair| pair[0] <= pair[1]);
    (messages, agrees)
}

/// A `run.revoke` on its way to the workspace, from [`CrewManager::confirm_revocation`] until
/// its answer (or failure): the retry pass leaves that run alone meanwhile (W2-DMN-13).
struct RevokeInFlight<'a> {
    revoking: &'a StdMutex<HashSet<String>>,
    run_id: String,
}

impl<'a> RevokeInFlight<'a> {
    fn begin(revoking: &'a StdMutex<HashSet<String>>, run_id: &str) -> Self {
        revoking
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(run_id.to_owned());
        Self {
            revoking,
            run_id: run_id.to_owned(),
        }
    }
}

impl Drop for RevokeInFlight<'_> {
    fn drop(&mut self) {
        self.revoking
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.run_id);
    }
}

/// Mark each message in a page that an agent wrote: `by_agent: true` beside a non-null `run_id`
/// (W2-DMN-11). An agent's post carries its owner's `actor_id`, and `people` names only people,
/// so the model read "Dave Patel: 3 messages" where people see two of them as "Dave Patel's
/// agent". The page is otherwise unchanged; nothing here reaches the broker.
fn mark_agent_posts(page: &mut Value) {
    for message in page
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if message
            .get("run_id")
            .and_then(Value::as_str)
            .is_some_and(|run| !run.is_empty())
        {
            message["by_agent"] = json!(true);
        }
    }
}

/// How the model is told to name authors: people as the person sees them, an agent's post as
/// that person's agent, and never an ID (naming design B5, W2-DMN-11).
const NAMING_RULE: &str = "labels gives the names of the IDs above as the person saw them when granting access. Refer to people as Display name (@username) and to channels as #name. Never quote IDs to people. A message with by_agent true was written by that person's agent: call it Display name's agent, never the person.";

/// Bounds on what one chat's [`RunReads`] keeps; beyond them nothing more is recorded.
const MAX_READ_FILES: usize = 32;
const MAX_READ_PEOPLE: usize = 512;
const MAX_READ_ATTACHMENTS: usize = 4096;
const MAX_NAMED_FILES: usize = 1024;
/// How many unordered messages one attachment may carry before its time is given up as
/// unknown (see [`RunReads::shared_at`]).
const MAX_CARRIERS: usize = 8;
/// How many of the destination's newest attachments admission names for the model.
const MAX_LISTED_FILES: usize = 16;
/// How many attachments the run saw but never named are looked up before its result is posted.
const MAX_POSTING_LOOKUPS: usize = 16;

/// A file's name as a person sees it: plain text, or "an untitled file".
fn shown_file_name(name: &str) -> String {
    let name = plain_label(name);
    if name.is_empty() {
        "an untitled file".to_owned()
    } else {
        name
    }
}

/// The body of the post `remote.attach` makes for the file it attached (W2-DMN-12): its name
/// is the model's choice, so it is a code span ([`markdown_file_name`]) and never reads as the
/// daemon's line that follows it.
fn attached_body(name: &str) -> String {
    format!("Attached {}", markdown_file_name(name))
}

/// A file's name in the line's Markdown: a code span ([`markdown_code`]), or "an untitled
/// file" as words when it has no visible name.
fn markdown_file_name(name: &str) -> String {
    let shown = plain_label(name);
    if shown.is_empty() {
        "an untitled file".to_owned()
    } else {
        markdown_code(&shown)
    }
}

/// `text` as one Markdown code span, so a name shows as typed and never as a link, emphasis
/// or anything else: nothing inside a code span is Markdown. The fence is one backtick longer
/// than the longest run of backticks in `text`, and padded with a space when `text` starts or
/// ends with a backtick (CommonMark strips one such space from each side).
fn markdown_code(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    if text.starts_with('`')
        || text.ends_with('`')
        || (text.starts_with(' ') && text.ends_with(' '))
    {
        format!("{fence} {text} {fence}")
    } else {
        format!("{fence}{text}{fence}")
    }
}

/// A person label (D13) in the line's Markdown: as it stands when nothing in it can be read as
/// Markdown, else as a code span ([`markdown_code`]). Backslash escapes are not enough: the
/// channel's GFM renderer finds `www.` and `https://` links and email addresses in the text
/// after escapes are resolved, so an escaped display name such as `(https://evil.example)`
/// still became a link. As it stands means: no ASCII punctuation but `( ) , ' - .`, an `@` that
/// no email address can end at (the label's own `(@username)`, or `@username` alone), and `_`
/// between two letters or digits (`crew_gina`, never emphasis); and no `www.`.
fn markdown_label(label: &str) -> String {
    let chars: Vec<char> = label.chars().collect();
    let plain = !label.to_lowercase().contains("www.")
        && chars.iter().enumerate().all(|(i, &c)| {
            let before = i.checked_sub(1).map(|j| chars[j]);
            let after = chars.get(i + 1).copied();
            !c.is_ascii_punctuation()
                || matches!(c, '(' | ')' | ',' | '\'' | '-' | '.')
                || (c == '@' && before.is_none_or(|b| b == '(' || b.is_whitespace()))
                || (c == '_'
                    && before.is_some_and(char::is_alphanumeric)
                    && after.is_some_and(char::is_alphanumeric))
        });
    if plain {
        label.to_owned()
    } else {
        markdown_code(label)
    }
}

impl RunReads {
    /// Take a message page's names and attachment times. `order` is how the method that
    /// answered lists its messages: the only thing that orders two posted in one second.
    fn note_context(&mut self, page: &Value, order: PageOrder) {
        if let Some(people) = page["people"].as_object() {
            let usernames: Vec<String> = people
                .values()
                .filter_map(|person| person["username"].as_str())
                .map(plain_label)
                .collect();
            for (id, person) in people {
                if self.people.len() >= MAX_READ_PEOPLE && !self.people.contains_key(id) {
                    break;
                }
                if let Some(label) = person_label(person, &usernames) {
                    self.people.insert(id.clone(), label);
                }
            }
        }
        let (messages, agrees) = oldest_first(page, order);
        let carriers: Vec<(Posted, Vec<&str>)> = messages
            .into_iter()
            .filter_map(|message| {
                let posted = Posted {
                    at: message["created_at"].as_u64()?,
                    message: message["id"].as_str()?.to_owned(),
                };
                let attachments: Vec<&str> = message["attachments"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                (!attachments.is_empty()).then_some((posted, attachments))
            })
            .collect();
        // The page's order first, so the attachments below are compared with it.
        if agrees {
            let mut start = 0;
            while start < carriers.len() {
                let at = carriers[start].0.at;
                let end = carriers[start..]
                    .iter()
                    .position(|(posted, _)| posted.at != at)
                    .map_or(carriers.len(), |n| start + n);
                let mut run: Vec<String> = Vec::new();
                for (posted, _) in &carriers[start..end] {
                    if !run.contains(&posted.message) {
                        run.push(posted.message.clone());
                    }
                }
                self.note_same_second(at, run);
                start = end;
            }
        }
        for (posted, attachments) in carriers {
            for id in attachments {
                self.note_carrier(id, posted.clone());
            }
        }
    }

    /// Keep one page's order of messages posted in second `at`, when it orders two or more.
    fn note_same_second(&mut self, at: u64, run: Vec<String>) {
        if run.len() < 2 || self.same_second_ids + run.len() > MAX_READ_ATTACHMENTS {
            return;
        }
        let runs = self.same_second.entry(at).or_default();
        if !runs.contains(&run) {
            self.same_second_ids += run.len();
            runs.push(run);
        }
    }

    /// Record that attachment `id` was carried by `posted`. A message known to be no later
    /// than one already kept adds nothing; one known later replaces those it follows.
    fn note_carrier(&mut self, id: &str, posted: Posted) {
        let known = match self.shared_at.get(id) {
            // Given up as unknown: stays so.
            Some(known) if known.is_empty() => return,
            Some(known) => known.clone(),
            None if self.shared_at.len() >= MAX_READ_ATTACHMENTS => return,
            None => Vec::new(),
        };
        if known.iter().any(|kept| {
            matches!(
                self.order(kept, &posted),
                Some(Ordering::Greater | Ordering::Equal)
            )
        }) {
            return;
        }
        let mut kept: Vec<Posted> = known
            .into_iter()
            .filter(|kept| self.order(&posted, kept) != Some(Ordering::Greater))
            .collect();
        kept.push(posted);
        if kept.len() > MAX_CARRIERS {
            kept.clear();
        }
        self.shared_at.insert(id.to_owned(), kept);
    }

    /// How two messages were posted, when the reads showed it: by second, then by one page's
    /// order of that second. `None` for two messages of one second no page listed together.
    fn order(&self, a: &Posted, b: &Posted) -> Option<Ordering> {
        if a.at != b.at {
            return Some(a.at.cmp(&b.at));
        }
        if a.message == b.message {
            return Some(Ordering::Equal);
        }
        self.same_second.get(&a.at)?.iter().find_map(|run| {
            let a = run.iter().position(|id| *id == a.message)?;
            let b = run.iter().position(|id| *id == b.message)?;
            Some(a.cmp(&b))
        })
    }

    /// Whether attachment `a` is known to have been shared after `b`: some message carrying
    /// `a` is known later than every message carrying `b`. `false` when either time is
    /// unknown or the two cannot be ordered.
    fn shared_after(&self, a: &str, b: &str) -> bool {
        let (Some(a), Some(b)) = (self.shared_at.get(a), self.shared_at.get(b)) else {
            return false;
        };
        !b.is_empty()
            && a.iter().any(|later| {
                b.iter()
                    .all(|earlier| self.order(later, earlier) == Some(Ordering::Greater))
            })
    }

    /// When attachment `id` was shared, in seconds, when the reads showed it.
    fn shared_second(&self, id: &str) -> Option<u64> {
        self.shared_at.get(id)?.first().map(|posted| posted.at)
    }

    /// Record a blob's name and sharer from the broker's `blob` fields. `always` records it
    /// past [`MAX_NAMED_FILES`] (a file the run read and [`Self::files`] or
    /// [`Self::unlisted`] records, of which there are at most [`MAX_READ_FILES`] and
    /// [`MAX_READ_ATTACHMENTS`] more).
    fn note_blob(&mut self, blob: &Value, always: bool) -> Option<ReadFile> {
        let (Some(id), Some(name)) = (blob["id"].as_str(), blob["name"].as_str()) else {
            return None;
        };
        let file = ReadFile {
            id: id.to_owned(),
            name: name.to_owned(),
            owner_id: blob["owner_id"].as_str().unwrap_or_default().to_owned(),
        };
        if always || self.named.len() < MAX_NAMED_FILES || self.named.contains_key(id) {
            self.named.insert(file.id.clone(), file.clone());
        }
        Some(file)
    }

    /// Record the file a successful `blob.read` returned: listed once, and marked as read now
    /// every time, so a read after the chat's last post is named by its next one even when an
    /// earlier post named the same file (W2-DMN-12). A file past every bound is not recorded
    /// by itself, but its read still is ([`Self::unrecorded_at`]).
    fn note_file(&mut self, read: &Value) {
        let blob = &read["blob"];
        let recorded = blob["id"].as_str().is_some_and(|id| self.was_read(id))
            || self.files.len() < MAX_READ_FILES
            || self.unlisted.len() < MAX_READ_ATTACHMENTS;
        let Some(file) = self.note_blob(blob, recorded) else {
            return;
        };
        let mark = next_read_mark();
        if !recorded {
            self.unrecorded_at = Some(mark);
            return;
        }
        self.read_at.insert(file.id.clone(), mark);
        if self.was_read(&file.id) {
            return;
        }
        if self.files.len() < MAX_READ_FILES {
            self.files.push(file);
        } else {
            self.unlisted.insert(file.id);
        }
    }

    /// Whether these reads hold no read at all: then, and only then, a line says no file was
    /// read.
    fn read_nothing(&self) -> bool {
        self.files.is_empty() && self.unlisted.is_empty() && self.unrecorded_at.is_none()
    }

    /// Record the name a successful `blob.status` gave, for a complete file (the only kind a
    /// run can read).
    fn note_status(&mut self, status: &Value) {
        if status["complete"].as_bool() == Some(true) {
            self.note_blob(status, false);
        }
    }

    /// The other known files shown under `file`'s name: its copies, to the people reading.
    fn copies_of<'a>(&'a self, file: &'a ReadFile) -> impl Iterator<Item = &'a ReadFile> {
        let shown = plain_label(&file.name);
        self.named.values().filter(move |other| {
            other.id != file.id && !shown.is_empty() && plain_label(&other.name) == shown
        })
    }

    /// `earlier copy` when another known copy was shared after `file`, `newest copy` when
    /// `file` was shared after every other known copy, else nothing: one file alone has no
    /// copies, and a time the reads did not show, or two they did not order, decides neither.
    /// A copy known to be newer makes `file` an earlier copy whatever the others are.
    fn copy_of(&self, file: &ReadFile) -> Option<&'static str> {
        let copies: Vec<&ReadFile> = self.copies_of(file).collect();
        if copies.is_empty() {
            None
        } else if copies
            .iter()
            .any(|copy| self.shared_after(&copy.id, &file.id))
        {
            Some("earlier copy")
        } else if copies
            .iter()
            .all(|copy| self.shared_after(&file.id, &copy.id))
        {
            Some("newest copy")
        } else {
            None
        }
    }

    /// Whether the run read `id`, listed or past [`MAX_READ_FILES`].
    fn was_read(&self, id: &str) -> bool {
        self.files.iter().any(|file| file.id == id) || self.unlisted.contains(id)
    }

    /// Whether a copy of `file` the run did not read is known to be newer than every copy of
    /// it the run read.
    fn newer_copy_unread(&self, file: &ReadFile) -> bool {
        let read: Vec<&str> = std::iter::once(file)
            .chain(self.copies_of(file))
            .filter(|copy| self.was_read(&copy.id))
            .map(|copy| copy.id.as_str())
            .collect();
        self.copies_of(file).any(|copy| {
            !self.was_read(&copy.id)
                && read
                    .iter()
                    .all(|earlier| self.shared_after(&copy.id, earlier))
        })
    }

    /// The line a task's posted result ends with, in Markdown, as of the daemon's local clock
    /// now ([`Self::source_line_at`]).
    fn source_line(&self) -> Option<String> {
        self.source_line_at(&chrono::Local::now())
    }

    /// These reads as they stand for the chat's next post: only the files read since its last
    /// one (W2-DMN-12), a file read again included. Everything that names them (people, times,
    /// copies) is kept whole.
    ///
    /// The post gets its own [`MAX_READ_FILES`], not what is left of the chat's: the chat's
    /// list holds the first files it ever read, so a file first read after them is listed
    /// here by its name in [`Self::named`], after the listed ones, in the order last read.
    /// Only past this post's own bound is a file counted instead of named. Otherwise a chat
    /// that had read [`MAX_READ_FILES`] files named none of its later reads, and its next post
    /// said it read no file.
    fn since_last_post(&self) -> Self {
        let posted = self.posted_through;
        let read_since = |id: &str| self.read_at.get(id).is_some_and(|&mark| mark > posted);
        let mut since = self.clone();
        since.files.retain(|file| read_since(&file.id));
        since.unlisted.clear();
        let mut later: Vec<(ReadMark, &String)> = self
            .unlisted
            .iter()
            .filter_map(|id| {
                let mark = *self.read_at.get(id)?;
                (mark > posted).then_some((mark, id))
            })
            .collect();
        later.sort();
        for (_, id) in later {
            match self.named.get(id) {
                Some(file) if since.files.len() < MAX_READ_FILES => since.files.push(file.clone()),
                _ => {
                    since.unlisted.insert(id.clone());
                }
            }
        }
        since.unrecorded_at = self.unrecorded_at.filter(|&mark| mark > posted);
        since
    }

    /// A post of the chat's own went out with a line built at `mark`: every read up to it has
    /// been named. A read recorded after the line was built (another tool call of the same
    /// batch, while the post was on its way) is left for the next post.
    fn mark_posted(&mut self, mark: ReadMark) {
        self.posted_through = self.posted_through.max(mark);
    }

    /// The line and mark the post sent under `key` as `request` was first sent with, if it was
    /// ([`SentPost`]).
    fn sent_post(&self, key: &str, request: &[u8; 32]) -> Option<(Option<String>, ReadMark)> {
        self.sent_posts
            .iter()
            .find(|sent| sent.key == key && sent.request == *request)
            .map(|sent| (sent.line.clone(), sent.mark))
    }

    /// Keep the line a post is sent with under its key, in place of another request's under
    /// the same key. Past [`MAX_SENT_POSTS`] the oldest is dropped.
    fn note_sent_post(&mut self, sent: SentPost) {
        self.sent_posts.retain(|kept| kept.key != sent.key);
        if self.sent_posts.len() >= MAX_SENT_POSTS {
            self.sent_posts.pop_front();
        }
        self.sent_posts.push_back(sent);
    }

    /// The line a task's posted result ends with, in Markdown, written at `now` (whose time
    /// zone is the one the line's times are given in). For one file:
    ///
    /// ``Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina) at 2:20 AM UTC-7.``
    ///
    /// and for several, ``Sources: `a.csv` (earlier copy, shared by …), `b.csv`.``, with
    /// `and N more files` when more were read than it lists ([`Self::counted_files`]), or
    /// ``Sources: N shared files, not listed.`` when it can name none of them. A file name is
    /// a code span, and so is a person label Markdown could read as anything
    /// ([`markdown_label`]), so neither can become a link or pose as the line's own notes.
    /// "shared by" is left out when the reads never named the person: never an ID. When another
    /// file with the same name is known, the line tells them apart: a copy known to be the
    /// newest is given the time it was shared ([`shared_when`]; Q4-27: "newest copy" was true
    /// only when it was written, so two results posted a day apart both said it of different
    /// files), and an earlier one says `earlier copy`. When the run read only earlier copies of
    /// a name, a second sentence says a newer one was left unread. `None` only when nothing
    /// was read ([`Self::read_nothing`]).
    fn source_line_at<Tz: TimeZone>(&self, now: &DateTime<Tz>) -> Option<String> {
        let entries: Vec<LineEntry> = self
            .files
            .iter()
            .map(|file| self.line_entry(file, now))
            .collect();
        let counted = self.counted_files();
        let mut line = match entries.as_slice() {
            [] => {
                // Files were read, and this line can name none of them: it counts them, and
                // never reads as no file read.
                let (count, noun) = counted?;
                let source = if count == "1" { "Source" } else { "Sources" };
                format!("{source}: {count} shared {noun}, not listed.")
            }
            [entry] if counted.is_none() => {
                let mut line = format!("Source: {}", entry.name);
                if let Some(copy) = &entry.copy {
                    line.push_str(&format!(" ({copy})"));
                }
                if let Some(shared) = &entry.shared {
                    line.push_str(&format!(", {shared}"));
                }
                line.push('.');
                line
            }
            several => {
                let mut parts: Vec<String> = several
                    .iter()
                    .map(|entry| {
                        let notes: Vec<&str> = entry
                            .copy
                            .iter()
                            .chain(entry.shared.iter())
                            .map(String::as_str)
                            .collect();
                        if notes.is_empty() {
                            entry.name.clone()
                        } else {
                            format!("{} ({})", entry.name, notes.join(", "))
                        }
                    })
                    .collect();
                if let Some((count, noun)) = &counted {
                    parts.push(format!("and {count} more {noun}"));
                }
                format!("Sources: {}.", parts.join(", "))
            }
        };
        // A name with a copy known to be newer than every copy the run read: the numbers came
        // from an older upload, and the line says so, whatever the reply claims. Once a read
        // went unrecorded, that read may have been the newer copy: the line still warns, and
        // says only what the daemon saw, so reading past every bound never removes the warning.
        let unread = if self.unrecorded_at.is_none() {
            "was not read"
        } else {
            "may not have been read"
        };
        let mut warned: Vec<String> = Vec::new();
        for file in &self.files {
            let shown = plain_label(&file.name);
            if warned.contains(&shown) {
                continue;
            }
            if self.newer_copy_unread(file) {
                line.push_str(&format!(
                    " A newer copy of {} was shared and {unread}.",
                    markdown_file_name(&file.name)
                ));
                warned.push(shown);
            }
        }
        Some(line)
    }

    /// How many files the line counts rather than names, and the noun for them: those in
    /// [`Self::unlisted`], and at least one more when a read went unrecorded
    /// ([`Self::unrecorded_at`]), which cannot say how many distinct files it read. `None` when
    /// the line names every file read.
    fn counted_files(&self) -> Option<(String, &'static str)> {
        let unrecorded = self.unrecorded_at.is_some();
        let n = self.unlisted.len() + usize::from(unrecorded);
        if n == 0 {
            return None;
        }
        let count = if unrecorded {
            format!("at least {n}")
        } else {
            n.to_string()
        };
        Some((count, if n == 1 { "file" } else { "files" }))
    }

    /// What the line says of one file it read: its name, which copy it is when that is
    /// `earlier copy`, and who shared it and, for the newest of several copies, when
    /// (`shared by Gina Rossi (@crew_gina) at 2:20 AM UTC-7`, `shared at …` when nobody was
    /// named). A newest copy whose time cannot be placed keeps `newest copy`.
    fn line_entry<Tz: TimeZone>(&self, file: &ReadFile, now: &DateTime<Tz>) -> LineEntry {
        let sharer = self
            .people
            .get(&file.owner_id)
            .map(|label| markdown_label(label));
        let mut copy = self.copy_of(file).map(str::to_owned);
        let mut when = None;
        if copy.as_deref() == Some("newest copy") {
            when = self
                .shared_second(&file.id)
                .and_then(|at| shared_when(at, now));
            if when.is_some() {
                copy = None;
            }
        }
        let shared = match (sharer, when) {
            (Some(sharer), Some(when)) => Some(format!("shared by {sharer} {when}")),
            (Some(sharer), None) => Some(format!("shared by {sharer}")),
            (None, Some(when)) => Some(format!("shared {when}")),
            (None, None) => None,
        };
        LineEntry {
            name: markdown_file_name(&file.name),
            copy,
            shared,
        }
    }

    /// The listed attachments the workspace named, in the order given, as the model reads them
    /// in `crew_context`: name, who shared it, when (the message's `created_at`), which copy,
    /// and the `blob_id` to read it with. Names are plain text and untrusted data.
    fn shared_files(&self, ids: &[String]) -> Vec<Value> {
        ids.iter()
            .filter_map(|id| {
                let file = self.named.get(id)?;
                Some(json!({
                    "blob_id": file.id,
                    "name": shown_file_name(&file.name),
                    "shared_by": self.people.get(&file.owner_id),
                    "shared_at": self.shared_second(id),
                    "copy": self.copy_of(file),
                }))
            })
            .collect()
    }

    /// Up to `limit` attachments the reads showed, with a time, but no answer named, newest
    /// second first.
    fn unnamed_newest_first(&self, limit: usize) -> Vec<String> {
        let mut unnamed: Vec<(u64, &String)> = self
            .shared_at
            .keys()
            .filter(|id| !self.named.contains_key(*id))
            .filter_map(|id| Some((self.shared_second(id)?, id)))
            .collect();
        unnamed.sort_by(|a, b| b.cmp(a));
        unnamed
            .into_iter()
            .take(limit)
            .map(|(_, id)| id.clone())
            .collect()
    }
}

/// A page's attachments, newest first by the page's own order (see [`PageOrder`]), at most
/// `limit`. Where the page's `created_at` disagrees with its order, by `created_at` instead.
fn newest_attachments(page: &Value, order: PageOrder, limit: usize) -> Vec<String> {
    let (mut messages, agrees) = oldest_first(page, order);
    if !agrees {
        messages.sort_by_key(|message| message["created_at"].as_u64().unwrap_or(0));
    }
    let mut found: Vec<String> = Vec::new();
    for message in messages.into_iter().rev() {
        for id in message["attachments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if found.len() == limit {
                return found;
            }
            if !found.iter().any(|known| known == id) {
                found.push(id.to_owned());
            }
        }
    }
    found
}

/// When a file was shared, as the Source line says it (Q4-27): `at 2:20 AM UTC-7` in `now`'s
/// time zone (the daemon's local one when the line is posted), with the day in front, `on Sep
/// 24 at 2:20 AM UTC-7`, when it was not shared on the day the line is written, and the year too
/// when that differs. `at` is the message's `created_at`, in seconds. `None` for a time the
/// zone cannot place.
fn shared_when<Tz: TimeZone>(at: u64, now: &DateTime<Tz>) -> Option<String> {
    let shared = now
        .timezone()
        .timestamp_opt(i64::try_from(at).ok()?, 0)
        .single()?;
    let offset = shared.offset().fix().local_minus_utc();
    let zone = match (offset / 3600, (offset % 3600).abs() / 60) {
        (0, 0) => "UTC".to_owned(),
        (hours, 0) => format!("UTC{hours:+}"),
        (hours, minutes) => {
            let sign = if offset < 0 { '-' } else { '+' };
            format!("UTC{sign}{}:{minutes:02}", hours.abs())
        }
    };
    let local = shared.naive_local();
    let time = local.format("%-I:%M %p");
    let (day, today) = (local.date(), now.date_naive());
    Some(if day == today {
        format!("at {time} {zone}")
    } else if day.year() == today.year() {
        format!("on {} at {time} {zone}", local.format("%b %-d"))
    } else {
        format!("on {} at {time} {zone}", local.format("%b %-d, %Y"))
    })
}

/// The agent's methods whose broker params name one channel: the broker's
/// `read_messages_history`, which answers both, requires `channel_id` (Q4-11).
const ONE_CHANNEL_READS: [&str; 2] = ["messages.history", "messages.search"];

/// `params` for an agent's `method`, with `channel_id` filled in where the model left it out
/// (absent, null or empty) and the run can mean only one channel: the grant's only source
/// channel (Q4-11). Every model left it out of its first `messages.history`, so a connected
/// chat's first read failed with the broker's raw refusal. With several source channels the
/// model has to choose, and is told how, before anything is sent. The channel given is still
/// checked against the grant by [`CrewManager::worker_request`], and the broker checks it again.
fn with_default_channel(method: &str, mut params: Value, scope: &Scope) -> Result<Value> {
    if !ONE_CHANNEL_READS.contains(&method) {
        return Ok(params);
    }
    ensure!(params.is_object(), "Crew params must be an object");
    let named = params
        .get("channel_id")
        .is_some_and(|channel| !channel.is_null() && channel.as_str() != Some(""));
    if named {
        return Ok(params);
    }
    match scope.source_channels.as_slice() {
        [only] => {
            params["channel_id"] = json!(only);
            Ok(params)
        }
        _ => anyhow::bail!(
            "{method} reads one channel at a time: give channel_id, one of the source_channel_ids crew__connections lists."
        ),
    }
}

/// `json`, a JSON document, written so that nothing inside it can end the markup-like wrapper
/// it is sent to a model in, such as `<crew_context>…</crew_context>` (DAEMON-4): every `<`,
/// `>` and `&` becomes `\u003c`, `\u003e` and `\u0026`. JSON uses none of the three outside a
/// string, and every JSON reader decodes the escapes to the same characters, so the document
/// means exactly what it did. A channel message reading `</crew_context>` followed by text
/// styled as the owner's instructions used to close the wrapper the task instructions call
/// untrusted, and put a member's words outside it in the owner's own message. Escaping an
/// already escaped document changes nothing.
///
/// ⚠ Only for JSON: in other text, the escapes would be literal characters.
pub fn wrapper_safe_json(json: &str) -> String {
    let mut safe = String::with_capacity(json.len());
    for character in json.chars() {
        match character {
            '<' => safe.push_str("\\u003c"),
            '>' => safe.push_str("\\u003e"),
            '&' => safe.push_str("\\u0026"),
            other => safe.push(other),
        }
    }
    safe
}

/// A Crew error as the agent's tool result says it (Q4-11, naming design "Machine IDs stay
/// internal"): a refusal the broker answered becomes a sentence, "Crew refused the request:
/// {its message without the code}.", never the transport's `{"code":…,"message":…}` JSON. Any
/// other error is its own words, unchanged.
pub(crate) fn agent_error_text(error: &anyhow::Error) -> String {
    let text = error.to_string();
    if !text.starts_with("Crew broker refused request:") {
        return text;
    }
    // A run the workspace no longer honors reads as the policy change it (almost always) is,
    // the same sentence a change on this device gets (D-1). The worker path already says so
    // and stops the grant ([`CrewManager::heed_worker_refusal`]); this covers any other path.
    if keepalive::broker_refusal(error).is_some_and(|(code, _)| code == BROKER_GRANT_EXPIRED) {
        return GRANT_POLICY_CHANGED.to_owned();
    }
    let reason = keepalive::broker_refusal(error)
        .map(|(code, message)| {
            let message = message.trim();
            let message = message
                .strip_prefix(code.as_str())
                .and_then(|rest| rest.strip_prefix(':'))
                .unwrap_or(message);
            plain_label(message)
        })
        .filter(|reason| !reason.is_empty());
    match reason {
        Some(reason) if reason.ends_with(['.', '!', '?']) => {
            format!("Crew refused the request: {reason}")
        }
        Some(reason) => format!("Crew refused the request: {reason}."),
        None => "Crew refused the request.".to_owned(),
    }
}

/// The broker's technical texts that name something a person can act on, said as sentences
/// (Q2-76), matched on the text after its code, in lowercase, without a closing full stop. The
/// CLI keeps the same table for the refusals it reads over HTTP (`commands/crew/output.rs`).
const REFUSAL_SENTENCES: &[(&str, &str)] = &[
    (
        "unknown device",
        "This computer isn't a member of this workspace.",
    ),
    (
        "signed device required",
        "This computer isn't signed in to this workspace.",
    ),
    (
        "channel unavailable",
        "You're not in that channel.",
    ),
    (
        "principal unavailable",
        "That person isn't a member of this workspace.",
    ),
    (
        "invalid grant",
        "This task's access to the workspace has ended.",
    ),
    (
        "attachment unavailable",
        "That file isn't available to you. It may have been removed, or you may not be in its channel.",
    ),
];

/// The code of a refusal the workspace answered, found anywhere in `error`'s chain
/// (`storage_full`, `forbidden`, ...): `None` when the workspace did not answer with a refusal.
pub fn workspace_refusal_code(error: &anyhow::Error) -> Option<String> {
    error.chain().find_map(|link| {
        let text = link.to_string();
        let envelope: Value =
            serde_json::from_str(text.strip_prefix("Crew broker refused request: ")?).ok()?;
        Some(envelope.get("code")?.as_str()?.to_owned())
    })
}

/// A refusal the workspace answered, found anywhere in `error`'s chain, as a sentence for a
/// person (F-1): a known technical text in words, else the broker's own message without its
/// code, with a capital and a full stop. `None` when the workspace did not answer with a
/// refusal (the connection or the transport failed, or this device refused first).
pub fn refusal_sentence(error: &anyhow::Error) -> Option<String> {
    let (code, message) = error.chain().find_map(|link| {
        let text = link.to_string();
        let envelope: Value =
            serde_json::from_str(text.strip_prefix("Crew broker refused request: ")?).ok()?;
        let code = envelope.get("code")?.as_str()?.to_owned();
        let message = envelope
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Some((code, message))
    })?;
    let message = message.trim();
    let reason = message
        .strip_prefix(code.as_str())
        .and_then(|rest| rest.strip_prefix(':'))
        .unwrap_or(message);
    let reason = plain_label(reason);
    let key = reason
        .trim_end_matches(['.', '!', '?'])
        .to_ascii_lowercase();
    if let Some((_, sentence)) = REFUSAL_SENTENCES.iter().find(|(text, _)| *text == key) {
        return Some((*sentence).to_owned());
    }
    if reason.is_empty() || key == code {
        return Some("The workspace refused this request.".to_owned());
    }
    let mut chars = reason.chars();
    let mut sentence: String = chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !sentence.ends_with(['.', '!', '?']) {
        sentence.push('.');
    }
    Some(sentence)
}

/// Whether the workspace refused this one request (for one file, say), as against the
/// connection or the grant failing: after the first, the rest of a batch of look-ups is sent.
fn refused_by_workspace(error: &anyhow::Error) -> bool {
    error
        .to_string()
        .starts_with("Crew broker refused request:")
}

/// A `blob.read` answer as the model reads it best (Q3-17): a chunk that is UTF-8 text with no
/// NUL comes back as `text` in place of `data_hex`, so a CSV is not a hex puzzle. A chunk that
/// ends inside a character is cut before it, and `next_offset` points at that character, so
/// the next read starts there. Anything else (binary, or text that does not decode) keeps
/// `data_hex`. Offsets always count bytes.
fn readable_blob(mut read: Value) -> Value {
    let Some(bytes) = read["data_hex"].as_str().and_then(|data| unhex(data).ok()) else {
        return read;
    };
    let complete = read["complete"].as_bool() == Some(true);
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        // Cut inside a character at the end of an unfinished read: keep what decoded.
        Err(error) if error.error_len().is_none() && !complete && error.valid_up_to() > 0 => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or_default()
        }
        Err(_) => return read,
    };
    if text.contains('\0') {
        return read;
    }
    let used = text.len() as u64;
    let text = text.to_owned();
    let Some(fields) = read.as_object_mut() else {
        return read;
    };
    if used < bytes.len() as u64 {
        let Some(offset) = fields.get("offset").and_then(Value::as_u64) else {
            return read;
        };
        fields.insert("next_offset".into(), json!(offset + used));
    }
    fields.remove("data_hex");
    fields.insert("text".into(), json!(text));
    read
}

impl CrewManager {
    fn with_run_reads(&self, session: &str, record: impl FnOnce(&mut RunReads)) {
        let mut reads = self
            .run_reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        record(reads.entry(session.to_owned()).or_default());
    }

    /// Keep the names and attachment times a message read showed this chat. `method` is the
    /// request that answered, which says how the page is ordered.
    fn note_run_context(&self, session: &str, method: &str, page: &Value) {
        let Some(order) = PageOrder::of(method) else {
            return;
        };
        self.with_run_reads(session, |reads| reads.note_context(page, order));
    }

    /// Keep the file a successful `blob.read` returned to this chat.
    fn note_blob_read(&self, session: &str, read: &Value) {
        if let Some(blob) = read["blob"]["id"].as_str() {
            tracing::info!(session, blob, "Crew run read a shared file");
        }
        self.with_run_reads(session, |reads| reads.note_file(read));
    }

    /// Keep the name a successful `blob.status` gave this chat.
    fn note_run_status(&self, session: &str, status: &Value) {
        self.with_run_reads(session, |reads| reads.note_status(status));
    }

    /// Ask the workspace for each attachment's name with `blob.status`, in order. A file the
    /// workspace refuses (one the run may not see) is skipped; any other failure (the bridge,
    /// the grant) ends the batch. Each answer is recorded by [`Self::worker_request`].
    async fn name_attachments(&self, session: &str, ids: &[String]) {
        for id in ids {
            match self
                .worker_request(session, "blob.status", json!({ "blob_id": id }))
                .await
            {
                Ok(_) => {}
                Err(error) if refused_by_workspace(&error) => {}
                Err(error) => {
                    tracing::debug!(session, error = %error, "Stopped naming a Crew run's shared files");
                    break;
                }
            }
        }
    }

    /// The files attached to `page`'s newest messages, named (Q3-02): the model is told each
    /// one's name, who shared it, when, and which copy is newest before it reads any, where a
    /// message page lists attachments by ID only. At most [`MAX_LISTED_FILES`], newest first;
    /// one the run may not see is left out. `page` is admission's `messages.history`, oldest
    /// first.
    async fn list_shared_files(&self, session: &str, page: &Value) -> Vec<Value> {
        let ids = newest_attachments(page, PageOrder::OldestFirst, MAX_LISTED_FILES);
        if ids.is_empty() {
            return Vec::new();
        }
        self.name_attachments(session, &ids).await;
        self.run_reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session)
            .map(|reads| reads.shared_files(&ids))
            .unwrap_or_default()
    }

    /// The provenance line for what this chat's Crew requests read (see
    /// [`RunReads::source_line`]); `None` when it read no file. Kept until
    /// [`Self::forget_run_reads`], so a failed post can build it again.
    pub fn run_source_line(&self, session: &str) -> Option<String> {
        self.run_reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session)
            .and_then(RunReads::source_line)
    }

    /// [`Self::run_source_line`] for the result about to be posted: first the workspace names
    /// up to [`MAX_POSTING_LOOKUPS`] attachments the run's reads showed and nothing named yet,
    /// newest first, so a newer copy of a file the run read is found wherever it was shared,
    /// and the line can say the run left it unread.
    pub async fn posted_source_line(&self, session: &str) -> Option<String> {
        let unnamed = {
            let reads = self
                .run_reads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let reads = reads.get(session)?;
            if reads.read_nothing() {
                return None;
            }
            reads.unnamed_newest_first(MAX_POSTING_LOOKUPS)
        };
        self.name_attachments(session, &unnamed).await;
        self.run_source_line(session)
    }

    /// The line a connected chat's post ends with: the files it read since its last post, named
    /// as [`Self::posted_source_line`] names a task's (W2-DMN-12), or `None` when it read none;
    /// and the mark the line was built at, for [`Self::mark_reads_posted`] once the post went
    /// out. The line and its mark are taken in one hold of the lock every read is recorded
    /// under, so the mark covers exactly the reads the line names.
    async fn chat_post_source_line(&self, session: &str) -> (Option<String>, ReadMark) {
        let unnamed = {
            let reads = self
                .run_reads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mark = current_read_mark();
            let Some(since) = reads.get(session).map(RunReads::since_last_post) else {
                return (None, mark);
            };
            if since.read_nothing() {
                return (None, mark);
            }
            since.unnamed_newest_first(MAX_POSTING_LOOKUPS)
        };
        self.name_attachments(session, &unnamed).await;
        let reads = self
            .run_reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let line = reads
            .get(session)
            .and_then(|reads| reads.since_last_post().source_line());
        (line, current_read_mark())
    }

    /// The line a chat's post under `key` ends with, and the mark it was built at: the line the
    /// same request was first sent with under that key, or else a new one
    /// ([`Self::chat_post_source_line`]), kept for a retry ([`SentPost`]). A key that is not
    /// text (the workspace refuses such a post) keeps nothing.
    async fn chat_post_line(
        &self,
        session: &str,
        key: Option<&str>,
        request: &[u8; 32],
    ) -> (Option<String>, ReadMark) {
        let Some(key) = key else {
            return self.chat_post_source_line(session).await;
        };
        let sent = self
            .run_reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session)
            .and_then(|reads| reads.sent_post(key, request));
        if let Some(sent) = sent {
            return sent;
        }
        let (line, mark) = self.chat_post_source_line(session).await;
        self.with_run_reads(session, |reads| {
            reads.note_sent_post(SentPost {
                key: key.to_owned(),
                request: *request,
                line: line.clone(),
                mark,
            })
        });
        (line, mark)
    }

    /// The chat's post went out with a line built at `mark`: it named every read up to there,
    /// and none after.
    fn mark_reads_posted(&self, session: &str, mark: ReadMark) {
        if let Some(reads) = self
            .run_reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_mut(session)
        {
            reads.mark_posted(mark);
        }
    }

    /// Drop what this chat's Crew requests read: its result was posted, or its grant ended.
    pub fn forget_run_reads(&self, session: &str) {
        self.run_reads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(session);
    }
}

/// Whether a deleted chat's grant is settled, and can be forgotten: its run ended more than
/// [`revocation::REPLACED_KEPT_PAST_END`] ago, so the workspace has not honored it for a week
/// whatever it was told, and no turn of the deleted chat is still unwinding with its context. A
/// grant whose end was never recorded is kept.
fn gone_grant_settled(scope: &Scope, now: u64) -> bool {
    scope
        .expires_at
        .is_some_and(|end| end.saturating_add(revocation::REPLACED_KEPT_PAST_END) <= now)
}

/// A session store's directory as [`Scope::session_store`] records it: a string, since a
/// `PathBuf` that is not UTF-8 would fail to save the whole registry. Lossy only for such a
/// path, where two stores that differ only in their non-UTF-8 bytes would read as one.
fn store_name(dir: &Path) -> String {
    dir.to_string_lossy().into_owned()
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
/// restricting, still listed and revocable, never acting.
///
/// It goes once it is settled: a week past its run's own end, when the workspace has long
/// stopped honoring the run and no turn of the deleted chat is still unwinding
/// ([`CrewManager::forget_gone_grant`], CROSSCUT-8). Session ids are single use now, so no
/// later chat ever holds a deleted chat's id to show its grant was an earlier chat's, and
/// waiting for one kept every deleted chat's grant for good: listed, and read on every scope
/// question, one database query each. A grant under an id another chat holds is still pruned
/// at once, for stores that reissue ids (a restored backup).
///
/// ⚠ **Only the grant's own session store may forget or prune it** ([`GrantStore`]). The
/// registry lives under the config folder and the chats under the data folder, which the
/// environment sets apart (`XDG_CONFIG_HOME`, `XDG_DATA_HOME`), so a terminal and the desktop
/// can share one registry over two `sessions.db`s, each minting `<date>_1` and onward. A
/// process asking about another store's chat finds no chat under its id, or finds its own
/// chat there, on every scope question it asks (and [`CrewManager::scoped_session_ids`] asks
/// about every grant). When that was read as "deleted" or "an earlier chat's", the other
/// store's live Crew chat lost its grant: once the saved registry dropped it, that chat, with
/// the channel's messages in its history, answered [`Standing::None`] and every restriction
/// was lifted, and a pruned live grant was also revoked at the workspace. So each grant
/// records its store, and a grant from another store, or one whose store was never recorded,
/// is never forgotten or pruned here: absent, it stays [`Standing::Unconfirmed`], and a chat
/// of this store under its id simply does not hold it.
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

    /// [`Self::own_store_dir`] as [`Scope::session_store`] records it.
    fn own_store(&self) -> Option<String> {
        self.own_store_dir().map(|dir| store_name(&dir))
    }

    /// Whether this process resolves chats in the store `scope`'s chat was saved in. A process
    /// that has not opened its store can confirm none, so every recorded store is elsewhere.
    fn grant_store(&self, scope: &Scope) -> GrantStore {
        match &scope.session_store {
            None => GrantStore::Unrecorded,
            Some(recorded) if self.own_store().as_ref() == Some(recorded) => GrantStore::Here,
            Some(_) => GrantStore::Elsewhere,
        }
    }

    /// Whether the grant stored under `session` is the grant of the chat holding that id
    /// now:
    ///
    /// - bound to that chat's incarnation: its own, and when its store was never recorded,
    ///   recorded in memory as this process's (the store holding the chat);
    /// - bound to another incarnation: none. Made to an earlier chat under the id when this
    ///   is the grant's own store, and pruned; otherwise another store's chat's grant under an
    ///   id both stores minted, and left alone ([`GrantStore`]);
    /// - bound, with no chat under the id: [`Standing::Unconfirmed`], since the chat it was
    ///   made to is gone from this store (a turn still unwinding may yet hold its context), or
    ///   was never in it. Forgotten once settled, and only when this is the grant's own store;
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
        // Another process may have granted, revoked or re-granted since this one last read
        // the saved registry (CROSSCUT-1).
        self.refresh_registry().await;
        if let Some(reason) = self.unreadable_restriction(session).await {
            return Standing::Unreadable(reason);
        }
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
        let store = self.grant_store(&scope);
        match (scope.session_incarnation, current) {
            (Some(bound), Some(current)) if bound == current => {
                if store == GrantStore::Unrecorded {
                    Standing::Own(self.adopt_binding(session, scope, current).await)
                } else {
                    Standing::Own(scope)
                }
            }
            (Some(_), Some(_)) => {
                if store == GrantStore::Here {
                    self.prune_stale_grant(session, &scope).await;
                }
                Standing::None
            }
            (Some(_), None) => {
                if store == GrantStore::Here && gone_grant_settled(&scope, revocation::unix_now()) {
                    self.forget_gone_grant(session, &scope).await;
                    return Standing::None;
                }
                Standing::Unconfirmed(scope, GRANT_GONE)
            }
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
    ///
    /// The store is recorded the same way, for a grant bound to its chat before stores were
    /// kept: the store holding that chat is its own (CROSSCUT-8), and only with it recorded may
    /// the grant ever be forgotten or pruned.
    async fn adopt_binding(&self, session: &str, legacy: Scope, current: i64) -> Scope {
        let store = self.own_store();
        let mut registry = self.registry.lock().await;
        let adopt = |scope: &mut Scope| {
            scope.session_incarnation.get_or_insert(current);
            if scope.session_store.is_none() {
                scope.session_store = store.clone();
            }
        };
        match registry.scopes.get_mut(session) {
            Some(scope) if scope.run_id == legacy.run_id => {
                adopt(scope);
                scope.clone()
            }
            _ => {
                let mut scope = legacy;
                adopt(&mut scope);
                scope
            }
        }
    }

    /// Drop a grant made to an earlier chat under `session`'s id, from memory and from the
    /// saved registry. Matched by its run, which the workspace mints once per grant, so a
    /// newer grant under the same id — made here or saved by another process — never goes
    /// with it. It is stopped as it goes ([`Scope::into_stopped`]): kept for its revocation and
    /// asked about again (F3, [`ReplacedGrant`]) unless the workspace already confirmed or
    /// ended its run. A grant still live here was dropped with no `run.revoke` before
    /// (DAEMON-1), and its run stayed live at the workspace, listed nowhere.
    ///
    /// ⚠ Only for a grant of this process's own store ([`GrantStore::Here`]): a chat of
    /// another store under the same id is not a later chat, and pruning its grant revoked a
    /// live run and lifted that chat's restriction (CROSSCUT-8).
    async fn prune_stale_grant(&self, session: &str, stale: &Scope) {
        let mut kept = false;
        let pruned = self
            .update_registry_keeping(|registry| {
                if registry
                    .scopes
                    .get(session)
                    .is_some_and(|scope| scope.run_id == stale.run_id)
                {
                    if let Some(scope) = registry.scopes.remove(session) {
                        kept = registry.keep_replaced(session, scope.into_stopped());
                    }
                }
                Ok(())
            })
            .await;
        if kept {
            self.schedule_revocation_retries(&stale.connection_id);
        }
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

    /// Forget the grant of a deleted chat that is settled ([`gone_grant_settled`]), from
    /// memory and the saved registry, with its run credential (CROSSCUT-8). Matched by its run,
    /// the chat it was made to and that chat's store, so a newer grant under the id never goes
    /// with it.
    ///
    /// ⚠ Only for a grant of this process's own store ([`GrantStore::Here`]), where no chat
    /// under the id means the chat is gone. Every chat of another store is missing from this
    /// one, so forgetting there took the grant of a live chat saved in that store, and with it
    /// every restriction on that chat.
    async fn forget_gone_grant(&self, session: &str, gone: &Scope) {
        let mut forgotten = false;
        let saved = self
            .update_registry_keeping(|registry| {
                if registry.scopes.get(session).is_some_and(|scope| {
                    scope.run_id == gone.run_id
                        && scope.session_incarnation == gone.session_incarnation
                        && scope.session_store == gone.session_store
                }) {
                    registry.scopes.remove(session);
                    forgotten = true;
                }
                Ok(())
            })
            .await;
        if let Err(error) = saved {
            tracing::warn!(
                session,
                %error,
                "could not remove a deleted chat's settled Crew grant from the saved registry"
            );
        }
        if !forgotten {
            return;
        }
        self.forget_run_reads(session);
        self.forget_live_admission(session);
        if let Err(error) = self.delete_credential(&format!("run:{session}")) {
            tracing::warn!(session, %error, "couldn't delete a settled Crew grant's run credential");
        }
        tracing::info!(
            session,
            run_id = %gone.run_id,
            "forgot the settled Crew grant of a deleted chat"
        );
    }

    /// The identity a new grant is bound to: the incarnation of the chat holding `session`
    /// in the store grants name. A chat that is not saved there cannot be granted.
    ///
    /// ⚠ Nor can a chat whose id holds the grant of a chat saved in another store (CROSSCUT-8):
    /// one grant is kept per id, and the run credential is stored under the id, so granting
    /// this chat would overwrite that chat's credential, stop and revoke its run
    /// ([`Self::record_grant`]), and leave it, Crew context and all, with no restriction. Asked
    /// before anything is created at the workspace. A grant whose store was never recorded is
    /// replaced as it always was: which chat it is cannot be told, and refusing would block
    /// the chat under its id for good.
    async fn grantable_chat(&self, session: &str) -> Result<i64> {
        let incarnation = match self.chat_incarnation(session).await {
            Ok(Some(incarnation)) => incarnation,
            Ok(None) => return Err(anyhow::anyhow!(UNSAVED_CHAT)),
            Err(error) => return Err(error.context(GRANT_UNCONFIRMED)),
        };
        self.refresh_registry().await;
        let held_elsewhere = self
            .registry
            .lock()
            .await
            .scopes
            .get(session)
            .is_some_and(|scope| {
                scope.session_incarnation != Some(incarnation)
                    && self.grant_store(scope) == GrantStore::Elsewhere
            });
        ensure!(!held_elsewhere, GRANT_ID_HELD_ELSEWHERE);
        Ok(incarnation)
    }

    /// Retire the grants of chats just deleted from the store at `store_dir`, each named
    /// with the incarnation its row carried: every one is **kept**, bound to the chat it was
    /// made to.
    ///
    /// ⚠ **Security-relevant; needs human review.** Keeping is the point. The workspace still
    /// honors the run, so the grant must stay listed and revocable (a deleted task's cancel
    /// revokes through it), and a delete only signals the chat's turn to stop, so the turn
    /// may still be dispatching tool calls with Crew context in hand. With no chat under its
    /// id, a bound grant resolves to [`Standing::Unconfirmed`]: it restricts that turn and
    /// authorizes nothing. `expired` is left alone: that flag says the grant was stopped
    /// here, which is what the access list shows and what hides its Revoke control, and
    /// deleting a chat stops nothing at the workspace.
    ///
    /// It goes in one of two ways, both decided by [`Self::standing`] and both only in a
    /// process whose own store is the grant's ([`GrantStore::Here`]): pruned at once if a later
    /// chat of that store holds the id, or forgotten once settled, a week past its run's own
    /// end ([`Self::forget_gone_grant`], [`gone_grant_settled`]). A process on another store
    /// never removes it, whatever its own store holds under the id (CROSSCUT-8).
    ///
    /// So a grant already bound to its chat and store needs nothing. What this does is bind a
    /// grant recorded before grants were bound, which would otherwise read as its chat's own
    /// with no chat under the id: acting for the unwinding turn in any process that had not
    /// yet bound it in memory, and handed to whichever chat next held the id. Such a grant is
    /// the deleted chat's only when it sits under the id in the very store grants name. And it
    /// records `store_dir` as the store of a grant made to a deleted chat before stores were
    /// kept: the delete is that store seeing the chat go, the one proof its grant may later be
    /// forgotten. A grant under one of these ids bound to another incarnation, or recorded
    /// against another store, belongs to a chat elsewhere and is not touched. One update of
    /// the saved registry as it is now ([`Self::update_registry_keeping`]), so a grant another
    /// process saved since this one loaded is never written away, and is bound too when it is
    /// the deleted chat's; memory takes the same edit even if the save fails.
    pub(crate) async fn retire_deleted_sessions(
        &self,
        deleted: &[(String, i64)],
        store_dir: &Path,
    ) -> Result<()> {
        let from_own_store = self.own_store_dir().is_some_and(|own| own == store_dir);
        let deleting_store = store_name(store_dir);
        let deleted: HashMap<&str, i64> = deleted
            .iter()
            .map(|(session, incarnation)| (session.as_str(), *incarnation))
            .collect();
        // The deleted chat's incarnation, when the grant under `session` was made to it.
        let deleted_chat = |session: &str, scope: &Scope| {
            let same_store = scope
                .session_store
                .as_ref()
                .is_none_or(|recorded| *recorded == deleting_store);
            deleted
                .get(session)
                .copied()
                .filter(|&incarnation| match scope.session_incarnation {
                    Some(bound) => same_store && bound == incarnation,
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
                    if scope.session_store.is_none() {
                        scope.session_store = Some(deleting_store.clone());
                    }
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

    /// W2-DMN-12 (review): through the manager, a `blob.read` that lands after a chat post's
    /// line was built and before the post is marked (another tool call of the same batch runs
    /// meanwhile) is named by the next post; and a file read again after a post is named again.
    #[tokio::test]
    async fn a_read_while_a_chat_post_is_on_its_way_is_named_by_the_next_post() {
        let root = fixture_root("chat-post-race");
        let manager = CrewManager::new(root.join("manager")).unwrap();
        let read = |id: &str, name: &str| {
            json!({"blob": {"id": id, "name": name, "owner_id": "p-gina"}, "offset": 0,
                "data_hex": "", "next_offset": 0, "complete": true})
        };
        let (line, mark) = manager.chat_post_source_line("chat").await;
        assert_eq!(line, None, "nothing read yet");
        manager.mark_reads_posted("chat", mark);

        manager.note_blob_read("chat", &read("b1", "assay.csv"));
        let (line, mark) = manager.chat_post_source_line("chat").await;
        assert_eq!(line.as_deref(), Some("Source: `assay.csv`."));
        manager.note_blob_read("chat", &read("b2", "plate.csv"));
        manager.mark_reads_posted("chat", mark);

        let (line, mark) = manager.chat_post_source_line("chat").await;
        assert_eq!(line.as_deref(), Some("Source: `plate.csv`."));
        manager.mark_reads_posted("chat", mark);
        assert_eq!(manager.chat_post_source_line("chat").await.0, None);

        manager.note_blob_read("chat", &read("b1", "assay.csv"));
        assert_eq!(
            manager.chat_post_source_line("chat").await.0.as_deref(),
            Some("Source: `assay.csv`.")
        );

        // A mark taken before the chat's reads were forgotten (a new grant) never covers a read
        // made after.
        let (_, stale) = manager.chat_post_source_line("chat").await;
        manager.forget_run_reads("chat");
        manager.note_blob_read("chat", &read("b3", "counts.tsv"));
        manager.mark_reads_posted("chat", stale);
        assert_eq!(
            manager.chat_post_source_line("chat").await.0.as_deref(),
            Some("Source: `counts.tsv`.")
        );
        let _ = fs::remove_dir_all(root);
    }

    /// T3-BE-13: only a `hello` that says its server stopped saving is read as one, and its
    /// own sentence is never kept.
    #[test]
    fn a_hello_says_whether_its_server_stopped_saving() {
        assert_eq!(
            ServerStorage::from_hello(&json!({"state": "running"})),
            None
        );
        assert_eq!(
            ServerStorage::from_hello(&json!({})),
            None,
            "an older broker"
        );
        assert_eq!(
            ServerStorage::from_hello(&json!({"state": "storage_failed",
                "storage": {"code": "storage_full", "message": "Ask the host.", "since": 7}})),
            Some(ServerStorage {
                state: "storage_failed".into(),
                code: "storage_full".into(),
                since: Some(7),
            })
        );
        // A code it does not know reads as the general one; a missing time as unknown.
        assert_eq!(
            ServerStorage::from_hello(&json!({"state": "storage_failed",
                "storage": {"code": "<b>made up</b>"}})),
            Some(ServerStorage {
                state: "storage_failed".into(),
                code: "storage_failed".into(),
                since: None,
            })
        );
    }

    /// W2-DMN-7: a request's lost bridge is typed by where it was lost, with the SSH failure
    /// kept underneath; anything the workspace answered, or that is not a transport failure,
    /// passes through untouched.
    #[test]
    fn a_lost_request_says_whether_anything_was_sent() {
        let ssh = || {
            anyhow::Error::new(SshFailure {
                kind: SshFailureKind::Other,
                code: "ssh_eof".into(),
                status: "exit_0".into(),
                description: "SSH connection closed".into(),
                detail: None,
                host: None,
                outcome_unknown: false,
            })
        };
        let unknown = lost_request(
            ssh(),
            Lost::WhileCarrying,
            "message.post",
            Some("key-1".into()),
            "lab",
        );
        let typed = CrewRefusal::find(&unknown).unwrap();
        assert_eq!(typed.code(), "crew_outcome_unknown");
        assert_eq!(typed.http_status(), 503);
        assert_eq!(
            unknown.to_string(),
            "Crew couldn't confirm whether this reached lab. Check the channel, then retry with the same request ID."
        );
        assert!(typed.fields().contains(&("request_id", json!("key-1"))));
        assert!(unknown
            .chain()
            .any(|cause| cause.downcast_ref::<SshFailure>().is_some()));

        let read = lost_request(ssh(), Lost::WhileCarrying, "messages.history", None, "lab");
        assert_eq!(
            CrewRefusal::find(&read).map(CrewRefusal::code),
            Some("crew_not_sent")
        );
        assert!(read.to_string().contains("Nothing changed"), "{read}");

        let before = lost_request(ssh(), Lost::BeforeSending, "message.post", None, "lab");
        assert_eq!(
            CrewRefusal::find(&before).map(CrewRefusal::code),
            Some("crew_not_sent")
        );
        assert_eq!(
            before.to_string(),
            "Biorouter couldn't reach lab, so nothing was sent."
        );

        let not_delivered = lost_request(
            anyhow::anyhow!(
                "Crew broker refused request: {}",
                json!({"code": "not_delivered", "message": "not_delivered: not sent"})
            ),
            Lost::WhileCarrying,
            "message.post",
            None,
            "lab",
        );
        assert_eq!(
            CrewRefusal::find(&not_delivered).map(CrewRefusal::code),
            Some("crew_not_sent")
        );

        let answered = lost_request(
            anyhow::anyhow!(
                "Crew broker refused request: {}",
                json!({"code": "forbidden", "message": "forbidden: channel unavailable"})
            ),
            Lost::WhileCarrying,
            "message.post",
            None,
            "lab",
        );
        assert!(CrewRefusal::find(&answered).is_none());
        let plain = lost_request(
            anyhow::anyhow!("Crew params must be an object"),
            Lost::BeforeSending,
            "message.post",
            None,
            "lab",
        );
        assert_eq!(plain.to_string(), "Crew params must be an object");
        // An unknown method is treated as one that may have changed something.
        assert!(!is_read_only("channel.read") && !is_read_only("future.method"));
    }

    /// W2-DMN-13: a run whose `run.revoke` is on its way is that request's to confirm; the
    /// retry pass never sends a second one beside it.
    #[tokio::test]
    async fn the_retry_pass_leaves_a_revoke_in_flight_alone() {
        let root = fixture_root("revoke-in-flight");
        let manager = CrewManager::new(root.clone()).unwrap();
        let connection_id = "revoke-in-flight-connection";
        let session = "revoke-in-flight-session";
        let (_, mut scope) = worker_race_connection(connection_id, ClusterMode::Public, 1, true);
        scope.expired = true;
        scope.revocation = Some(Revocation::Unconfirmed);
        let run_id = scope.run_id.clone();
        manager
            .registry
            .lock()
            .await
            .scopes
            .insert(session.into(), scope);
        assert_eq!(
            manager.unconfirmed_revocations(connection_id).await,
            vec![(session.to_owned(), run_id.clone())]
        );
        {
            let _in_flight = RevokeInFlight::begin(&manager.revoking, &run_id);
            assert!(manager
                .unconfirmed_revocations(connection_id)
                .await
                .is_empty());
        }
        assert_eq!(
            manager.unconfirmed_revocations(connection_id).await.len(),
            1
        );
        let _ = fs::remove_dir_all(root);
    }

    /// W2-DMN-11: only a message with a run behind it is an agent's.
    #[test]
    fn only_a_message_with_a_run_is_marked_as_an_agents() {
        let mut page = json!({"run_id": "reader-run", "messages": [
            {"id": "a", "run_id": "writer-run"},
            {"id": "b", "run_id": null},
            {"id": "c"},
            {"id": "d", "run_id": ""},
        ]});
        mark_agent_posts(&mut page);
        let marked: Vec<bool> = page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message.get("by_agent") == Some(&json!(true)))
            .collect();
        assert_eq!(marked, [true, false, false, false]);
        assert!(
            page.get("by_agent").is_none(),
            "the page's own run is the reader's"
        );
        let mut none = json!({"results": []});
        mark_agent_posts(&mut none);
        assert_eq!(none, json!({"results": []}));
    }

    /// W2-DMN-1: a Secret Service that nothing provides (a headless node: "The name is not
    /// activatable"), confirmed by a probe that fails the same way, is a keyring that is not
    /// there. One that is locked or whose prompt was dismissed is there and refused. A platform
    /// failure the probe does not confirm, and any other keyring error, keeps its own words.
    #[test]
    fn a_secret_service_is_absent_only_when_the_probe_agrees() {
        let dbus = || {
            keyring::Error::PlatformFailure(Box::new(std::io::Error::other(
                "DBus error: The name is not activatable",
            )))
        };
        assert_eq!(
            keyring_trouble(&dbus(), false, || true),
            Some(KeyringTrouble::Absent)
        );
        assert_eq!(keyring_trouble(&dbus(), false, || false), None);
        let locked = keyring::Error::NoStorageAccess(Box::new(std::io::Error::other(
            "Secret Service: object locked",
        )));
        for always_present in [false, true] {
            assert_eq!(
                keyring_trouble(&locked, always_present, || panic!(
                    "a refusal needs no probe"
                )),
                Some(KeyringTrouble::Refused)
            );
            assert_eq!(
                keyring_trouble(&keyring::Error::NoEntry, always_present, || {
                    panic!("not about the store")
                }),
                None
            );
        }
        let absent = KeyringTrouble::Absent.refusal();
        assert_eq!(absent.code(), "crew_credential_store_unavailable");
        assert_eq!(absent.message(), KEYRING_NOT_RUNNING_TEXT);
        assert!(!absent.message().contains("DBus"));
        let other = keyring_failure(keyring::Error::NoEntry);
        assert!(CrewRefusal::find(&other).is_none());
    }

    /// W2-DMN-1 (review): keyring maps the macOS Keychain's errSecUserCanceled (Deny at its
    /// prompt), errSecAuthFailed and errSecInteractionNotAllowed (no session to ask in) to
    /// `PlatformFailure`. The Keychain is always there, so each is a refusal the person can
    /// answer: never "no keyring service", never the vault's advice, and the Keychain is never
    /// probed (a read could raise a prompt for nothing).
    #[test]
    fn a_keychain_that_refused_is_a_refusal_never_a_missing_service() {
        for cause in [
            "User canceled the operation.",
            "The user name or passphrase you entered is not correct.",
            "User interaction is not allowed.",
        ] {
            let error = keyring::Error::PlatformFailure(Box::new(std::io::Error::other(cause)));
            let trouble = keyring_trouble(&error, true, || panic!("the Keychain is never probed"));
            assert_eq!(trouble, Some(KeyringTrouble::Refused), "{cause}");
            let refused = KeyringTrouble::Refused.refusal();
            assert_eq!(refused.code(), "crew_credential_store_refused");
            assert_eq!(refused.message(), CREDENTIAL_STORE_REFUSED_TEXT);
            for words in ["no keyring service", "credentials init", "isn't answering"] {
                assert!(!refused.message().contains(words), "{words}");
            }
            // Where this test runs on such a platform, the whole path says the same.
            if PLATFORM_STORE_ALWAYS_PRESENT {
                let error = keyring_failure(keyring::Error::PlatformFailure(Box::new(
                    std::io::Error::other(cause),
                )));
                let found = CrewRefusal::find(&error).expect("typed");
                assert_eq!(found.code(), "crew_credential_store_refused");
                assert_eq!(error.to_string(), CREDENTIAL_STORE_REFUSED_TEXT);
            }
        }
    }

    /// W2-DMN-1 (review): `biorouter crew credentials init` refuses a profile that holds any
    /// identity, so only a profile that holds none is pointed at it. A profile with a
    /// connection, a grant, a prepared device or a finished preparation is told to bring its
    /// keyring back, and a refusal stays a refusal.
    #[test]
    fn the_vault_is_offered_only_to_a_profile_that_holds_no_identity() {
        let absent = || {
            anyhow::Error::new(KeyringTrouble::Absent.refusal())
                .context("Couldn't save this connection's device key")
        };
        let refused = || anyhow::Error::new(KeyringTrouble::Refused.refusal());
        let message = |error: &anyhow::Error| {
            let found = CrewRefusal::find(error).expect("typed");
            (found.code(), found.message().to_owned())
        };

        let fresh = Registry::default();
        assert!(fresh.holds_no_identity());
        assert_eq!(
            message(&first_key_failure(absent(), &fresh)),
            (
                "crew_credential_store_unavailable",
                CREDENTIAL_STORE_UNAVAILABLE_TEXT.to_owned()
            )
        );
        assert_eq!(
            message(&first_key_failure(refused(), &fresh)),
            (
                "crew_credential_store_refused",
                format!("{CREDENTIAL_STORE_REFUSED_TEXT} {VAULT_INSTEAD_TEXT}")
            )
        );
        let other = first_key_failure(anyhow::anyhow!("disk full"), &fresh);
        assert_eq!(other.to_string(), "disk full");

        let (connection, scope) =
            worker_race_connection("keyring-held", ClusterMode::Public, 1, true);
        let prepared = PreparedDevice {
            preparation_id: "prepared".into(),
            public_key: "11".repeat(32),
            device_id: "22".repeat(32),
        };
        let held = [
            Registry {
                connections: vec![connection],
                ..Default::default()
            },
            Registry {
                scopes: HashMap::from([("chat".to_owned(), scope)]),
                ..Default::default()
            },
            Registry {
                pending_device: Some(prepared),
                ..Default::default()
            },
            Registry {
                completed_preparations: HashMap::from([("done".to_owned(), "saved".to_owned())]),
                ..Default::default()
            },
        ];
        for registry in &held {
            assert!(!registry.holds_no_identity());
            let kept = first_key_failure(absent(), registry);
            assert_eq!(
                message(&kept),
                (
                    "crew_credential_store_unavailable",
                    KEYRING_NOT_RUNNING_TEXT.to_owned()
                )
            );
            assert!(
                !format!("{kept:#}").contains("credentials init"),
                "{kept:#}"
            );
            assert_eq!(
                message(&first_key_failure(refused(), registry)),
                (
                    "crew_credential_store_refused",
                    CREDENTIAL_STORE_REFUSED_TEXT.to_owned()
                )
            );
        }
    }

    /// W2-DMN-1 (review): end to end, the first key a fresh profile saves (a prepared device, a
    /// new connection) is refused with the vault's advice when no keyring answers; once the
    /// profile holds a connection, the same failure says to bring the keyring back, since
    /// `credentials init` would now refuse, and a keyring that refused says so.
    #[cfg(unix)]
    #[tokio::test]
    async fn only_a_profile_with_no_identity_is_told_to_set_up_the_vault() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("keyring-first-key");
        let _env = isolated_crew_env(&root);
        let manager = CrewManager::new(root.join("manager")).unwrap();
        let input = |workspace: &str, target: &str| SaveConnection {
            preparation_id: None,
            name: format!("saved {target}"),
            ssh_target: target.into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/tmp/crew-keyring.sock".into(),
            owner_uid: 10001,
            workspace_id: workspace.into(),
            workspace_public_key: "44".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: None,
            mode: ClusterMode::Public,
            institution_id: None,
        };
        let said = |error: anyhow::Error| {
            let found = CrewRefusal::find(&error).expect("typed");
            (found.code(), found.message().to_owned())
        };
        let vault = (
            "crew_credential_store_unavailable",
            CREDENTIAL_STORE_UNAVAILABLE_TEXT.to_owned(),
        );
        manager.set_test_keyring_trouble(Some(KeyringTrouble::Absent));
        assert_eq!(said(manager.prepare_device().await.unwrap_err()), vault);
        assert_eq!(
            said(
                manager
                    .save(input(
                        "11111111-1111-4111-8111-111111111111",
                        "bob@a.example.org"
                    ))
                    .await
                    .unwrap_err()
            ),
            vault
        );
        assert!(manager.registry.lock().await.holds_no_identity());

        manager.set_test_keyring_trouble(None);
        manager
            .save(input(
                "22222222-2222-4222-8222-222222222222",
                "bob@b.example.org",
            ))
            .await
            .unwrap();
        manager.set_test_keyring_trouble(Some(KeyringTrouble::Absent));
        let restart = (
            "crew_credential_store_unavailable",
            KEYRING_NOT_RUNNING_TEXT.to_owned(),
        );
        assert_eq!(said(manager.prepare_device().await.unwrap_err()), restart);
        assert_eq!(
            said(
                manager
                    .save(input(
                        "33333333-3333-4333-8333-333333333333",
                        "bob@c.example.org"
                    ))
                    .await
                    .unwrap_err()
            ),
            restart
        );
        manager.set_test_keyring_trouble(Some(KeyringTrouble::Refused));
        assert_eq!(
            said(manager.prepare_device().await.unwrap_err()),
            (
                "crew_credential_store_refused",
                CREDENTIAL_STORE_REFUSED_TEXT.to_owned()
            )
        );
        manager.set_test_keyring_trouble(None);
        let _ = fs::remove_dir_all(root);
    }

    /// W2-DMN-4: a `BatchMode yes` in the person's ssh configuration must not suppress the
    /// Sign in window's password prompt, so the interactive plan says `BatchMode=no` itself,
    /// before the destination, where `ssh` reads options. The bridge stays unattended.
    #[test]
    fn the_interactive_sign_in_asks_for_prompts_and_the_bridge_never_does() {
        let connection = Connection {
            id: "sign-in-plan".into(),
            node_id: None,
            name: "sign in plan".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: Some("gate.example.test".into()),
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "sign-in-plan-workspace".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "sign-in-plan-cluster".into(),
            mode: ClusterMode::Private,
            institution_id: None,
            policy_epoch: 1,
            status: "offline".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        };
        let args = sign_in_args(&connection, Path::new("/tmp/crew-control"));
        let batch: Vec<usize> = args
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| pair[0] == "-o" && pair[1].starts_with("BatchMode="))
            .map(|(at, _)| at)
            .collect();
        assert_eq!(batch.len(), 1, "exactly one BatchMode option: {args:?}");
        assert_eq!(args[batch[0] + 1], "BatchMode=no");
        let target = args.iter().position(|arg| arg == "crew@example.test");
        assert_eq!(
            target,
            Some(args.len() - 1),
            "the destination is last: {args:?}"
        );
        assert!(batch[0] < args.len() - 1);
        for flag in [
            "-M",
            "-N",
            "ControlPersist=600",
            "StrictHostKeyChecking=yes",
        ] {
            assert!(args.iter().any(|arg| arg == flag), "{flag} kept: {args:?}");
        }
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
                "Your connection is Public, but this request required Private. Nothing was sent."
            );
            let typed = CrewRefusal::find(&mismatch).expect("a typed refusal");
            assert_eq!(typed.code(), "crew_mode_mismatch");
            assert_eq!(
                typed.fields(),
                &[
                    ("actual_mode", json!("public")),
                    ("expected_mode", json!("private"))
                ]
            );

            let missing = manager
                .human_request(connection_id, method, json!({}), None)
                .await
                .expect_err("the fixture intentionally has no device credential");
            assert!(
                CrewRefusal::find(&missing).is_none(),
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
                CrewRefusal::find(&matching).is_none(),
                "matching mode was rejected by the privacy guard: {matching}"
            );
        }

        let _ = fs::remove_dir_all(root);
    }

    /// T3-BE-5: an expected mode is compared with the privacy in force as well as the
    /// connection's own mode. A personal Public connection in a workspace that is Private for
    /// everyone is Private in force, so a request that required Private goes ahead; one that
    /// required Public is the connection's own mode, which the desktop sends as the mode it last
    /// verified, and goes ahead too. The mismatch that matters is still refused, naming the mode
    /// in force.
    #[test]
    fn an_expected_mode_is_judged_against_the_privacy_in_force() {
        use ClusterMode::{Private, Public};
        assert_eq!(effective_mode(Public, Some(Private)), Private);
        assert_eq!(effective_mode(Public, Some(Public)), Public);
        assert_eq!(effective_mode(Public, None), Public, "not known: its own");
        assert_eq!(effective_mode(Private, Some(Public)), Private);
        for (own, workspace, expected, passes) in [
            (Public, Some(Private), Private, true),
            (Public, Some(Private), Public, true),
            (Public, Some(Public), Private, false),
            (Public, None, Private, false),
            (Private, Some(Public), Public, false),
            (Private, None, Private, true),
        ] {
            let judged = require_mode(Some(&json!(expected)), own, workspace);
            assert_eq!(
                judged.is_ok(),
                passes,
                "{own:?} in {workspace:?}, {expected:?}"
            );
            if let Err(refused) = judged {
                let typed = CrewRefusal::find(&refused).expect("typed");
                assert_eq!(typed.code(), "crew_mode_mismatch");
                assert_eq!(
                    typed.fields()[0],
                    ("actual_mode", json!(effective_mode(own, workspace)))
                );
            }
        }
        assert!(require_mode(None, Public, Some(Private)).is_ok());
    }

    /// T3-BE-5, through the manager: the workspace's signed `hello` says it is Private for
    /// everyone, and a personal Public connection's post that required Private, or a task that
    /// did, is no longer refused as a mismatch. Before, both read "Your connection is Public,
    /// but this request required Private." while every surface said Private.
    #[tokio::test]
    async fn a_request_that_requires_the_privacy_in_force_is_not_a_mismatch() {
        let root = fixture_root("mode-in-force");
        let manager = CrewManager::new(root.clone()).unwrap();
        let connection_id = "mode-in-force-connection";
        manager.registry.lock().await.connections.push(Connection {
            id: connection_id.into(),
            node_id: None,
            name: "mode in force fixture".into(),
            ssh_target: "crew@example.test".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/run/crew.sock".into(),
            owner_uid: 10001,
            workspace_id: "mode-in-force-workspace".into(),
            workspace_public_key: "11".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: "mode-in-force-cluster".into(),
            mode: ClusterMode::Public,
            institution_id: None,
            policy_epoch: 1,
            status: "connected".into(),
            last_error: None,
            device_id: "22".repeat(32),
            public_key: "33".repeat(32),
        });
        let workspace_mode = |mode: ClusterMode| {
            manager.brokers.lock().unwrap().insert(
                connection_id.into(),
                BrokerHello {
                    signature_version: 2,
                    capabilities: vec![],
                    workspace_name: Some("okafor-lab".into()),
                    mode: Some(mode),
                    institution_id: None,
                    policy_epoch: Some(1),
                    storage: None,
                },
            );
        };
        let provider = crate::providers::testprovider::TestProvider::new_replaying(
            root.join("missing-cassette.json")
                .to_string_lossy()
                .into_owned(),
        )
        .unwrap();
        let task = |expected| {
            manager.begin_run_with_policy(
                "mode-in-force-session",
                connection_id,
                "destination-channel",
                vec![],
                &provider,
                RunPolicy {
                    expected_mode: Some(expected),
                    ..RunPolicy::default()
                },
            )
        };
        let code = |error: &anyhow::Error| CrewRefusal::find(error).map(CrewRefusal::code);

        workspace_mode(ClusterMode::Private);
        for expected in ["private", "public"] {
            let post = manager
                .human_request(
                    connection_id,
                    "message.post",
                    json!({"personal_mode": expected}),
                    None,
                )
                .await
                .expect_err("the fixture has no device key");
            assert_ne!(
                code(&post),
                Some("crew_mode_mismatch"),
                "{expected}: {post}"
            );
        }
        let admitted = task(ClusterMode::Private)
            .await
            .err()
            .expect("the fixture has no device key");
        assert_ne!(code(&admitted), Some("crew_mode_mismatch"), "{admitted}");

        // In a workspace that allows Public, the connection is Public in force.
        workspace_mode(ClusterMode::Public);
        let post = manager
            .human_request(
                connection_id,
                "message.post",
                json!({"personal_mode": "private"}),
                None,
            )
            .await
            .expect_err("a mismatch");
        assert_eq!(code(&post), Some("crew_mode_mismatch"));
        assert_eq!(
            post.to_string(),
            "Your connection is Public, but this request required Private. Nothing was sent."
        );
        let refused = task(ClusterMode::Private).await.err().expect("a mismatch");
        assert_eq!(code(&refused), Some("crew_mode_mismatch"));
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
        // W2-DMN-9: in the manual's words, naming the workspace, with its own code.
        assert_eq!(
            private_public.to_string(),
            "Your connection to run mode policy fixture is Private, so a public model can't \
             read it. Choose a private model."
        );
        assert_eq!(
            CrewRefusal::find(&private_public).map(CrewRefusal::code),
            Some("crew_public_model_refused")
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
            "Your connection is Public, but this request required Private. Nothing was sent."
        );
        assert_eq!(
            CrewRefusal::find(&mismatch).map(CrewRefusal::code),
            Some("crew_mode_mismatch")
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
            CrewRefusal::find(&legacy).is_none(),
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
            "This chat has used a private model, so a public model can't continue it with Crew \
             context. Choose a private model."
        );
        assert_eq!(
            CrewRefusal::find(&private_origin).map(CrewRefusal::code),
            Some("crew_public_model_refused")
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
            session_store: None,
            revocation: None,
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
    printf '{{"id":"%s","result":{{"messages":[{{"channel_id":"destination-channel","id":"destination-message","text":"destination-only","created_at":100,"sequence":"destination-message","attachments":["blob-old","blob-outside"]}},{{"channel_id":"destination-channel","id":"newer-message","text":"newer","created_at":100,"sequence":"newer-message","attachments":["blob-new"]}}],"people":{{"principal-gina":{{"username":"crew_gina","display_name":"Gina Rossi","active":true}}}}}}}}\n' "$id"
  elif printf '%s\n' "$line" | grep -q '"blob_id":"blob-outside"'; then
    printf '{{"id":"%s","error":{{"code":"privacy_denied","message":"privacy_denied: attachment outside run policy"}}}}\n' "$id"
  elif printf '%s\n' "$line" | grep -q '"blob_id":"blob-new"'; then
    printf '{{"id":"%s","result":{{"id":"blob-new","name":"gina-assay.csv","owner_id":"principal-gina","complete":true}}}}\n' "$id"
  elif printf '%s\n' "$line" | grep -q '"blob_id":"blob-old"'; then
    printf '{{"id":"%s","result":{{"id":"blob-old","name":"gina-assay.csv","owner_id":"principal-gina","complete":true}}}}\n' "$id"
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
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["channel_id"], "destination-channel");
        assert_eq!(history[0]["text"], "destination-only");
        // Q3-02: the destination's files are named for the model, newest first, with the
        // newest copy marked; one the workspace refuses to this run is left out. The two copies
        // were shared in one second, and the wire's sequence is an opaque ID, as the broker
        // sends it: the history's own order (oldest first) tells them apart.
        assert_eq!(
            context["shared_files"],
            json!([
                {"blob_id": "blob-new", "name": "gina-assay.csv",
                    "shared_by": "Gina Rossi (@crew_gina)", "shared_at": 100,
                    "copy": "newest copy"},
                {"blob_id": "blob-old", "name": "gina-assay.csv",
                    "shared_by": "Gina Rossi (@crew_gina)", "shared_at": 100,
                    "copy": "earlier copy"},
            ])
        );
        assert!(context["shared_files_note"]
            .as_str()
            .unwrap()
            .contains("use the newest copy unless the task names a specific copy"));
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
            session_store: None,
            revocation: None,
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
            replaced: Vec::new(),
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
            session_store: None,
            revocation: None,
        };
        let registry = Registry {
            connections: vec![connection],
            scopes: HashMap::from([("session-1".into(), scope.clone())]),
            replaced: Vec::new(),
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
            session_store: None,
            revocation: None,
        };
        let registry = Registry {
            connections: vec![connection.clone()],
            scopes: HashMap::from([("session-public".into(), scope.clone())]),
            replaced: Vec::new(),
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
        // Q3-29: the model is told the one step that connects this chat, in this chat.
        assert_eq!(result_text(&result), NO_GRANT);
        // Q4-11: as an answer, not a failed call; it did nothing.
        assert_eq!(result.is_error, Some(false));
        // The connections tool says the same.
        let listed = client
            .call_tool(
                "connections",
                None,
                McpMeta::new(
                    "omitted-connection-id-without-grant-session",
                    CallCapability::for_test(ProviderTier::Private, true),
                ),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(result_text(&listed), NO_GRANT);
        assert_eq!(listed.is_error, Some(false));
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
            session_store: None,
            revocation: None,
        };
        let registry = Registry {
            connections: vec![connection],
            scopes: HashMap::from([("session-1".into(), scope)]),
            replaced: Vec::new(),
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
                    session_store: None,
                    revocation: None,
                },
            )]),
            replaced: Vec::new(),
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
            replaced: Vec::new(),
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

        // Revoking a grant whose revocation the workspace confirmed answers with that
        // confirmation and asks nothing again: each ask would be journalled there once more
        // (W2-DMN-13).
        assert_eq!(
            manager.cancel_run("worker-race-session").await.unwrap(),
            revoked_run
        );
        assert_eq!(logged_methods(&log, "run.revoke"), 1);
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
            replaced: Vec::new(),
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
            replaced: Vec::new(),
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

    /// F-1: a refusal the workspace answered, wherever it sits in an error's chain, as a
    /// sentence a person can read; `None` for anything the workspace did not answer.
    #[test]
    fn a_workspace_refusal_reads_as_a_sentence() {
        let refused = |code: &str, message: &str| {
            anyhow::anyhow!(
                "Crew broker refused request: {}",
                json!({"code": code, "message": message})
            )
        };
        for (error, sentence) in [
            (
                refused("forbidden", "forbidden: channel unavailable"),
                "You're not in that channel.",
            ),
            (
                refused("unauthorized", "unauthorized: unknown device"),
                "This computer isn't a member of this workspace.",
            ),
            (
                refused("forbidden", "forbidden: attachment unavailable"),
                "That file isn't available to you. It may have been removed, or you may not be in its channel.",
            ),
            (
                refused("forbidden", "forbidden: incomplete attachment unavailable"),
                "Incomplete attachment unavailable.",
            ),
            (
                refused("forbidden", "forbidden"),
                "The workspace refused this request.",
            ),
            (
                refused("forbidden", ""),
                "The workspace refused this request.",
            ),
            (
                refused("request_denied", "Message too long."),
                "Message too long.",
            ),
            (
                refused("forbidden", "forbidden: channel unavailable")
                    .context("Couldn't start the transfer"),
                "You're not in that channel.",
            ),
        ] {
            assert_eq!(
                refusal_sentence(&error).as_deref(),
                Some(sentence),
                "{error:#}"
            );
        }
        assert_eq!(
            refusal_sentence(&anyhow::anyhow!(
                "Crew connection is disconnected; authenticate and connect in Crew"
            )),
            None
        );
        assert_eq!(
            refusal_sentence(&anyhow::anyhow!("Crew broker refused request: <garbled>")),
            None
        );
    }

    /// D-1: the broker's `grant_expired` reads as the policy sentence wherever an agent error
    /// is shown, never as its envelope.
    #[test]
    fn a_grant_the_workspace_ended_reads_as_the_policy_sentence() {
        let error = anyhow::anyhow!(
            "Crew broker refused request: {}",
            json!({"code": "grant_expired", "message": "grant_expired: run revoked, expired or policy changed"})
        );
        assert_eq!(agent_error_text(&error), GRANT_POLICY_CHANGED);
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

    /// T3-BE-7: a worker request on a grant that ended (the context route's, among others) is
    /// refused typed, `crew_grant_ended` with `409` and a `reason`, so a client can say what to do
    /// in its own words. The sentence is the one the chat already reads.
    #[tokio::test]
    async fn an_ended_grant_is_refused_as_crew_grant_ended_with_its_reason() {
        let root = fixture_root("grant-ended-typed");
        let (connection, scope) = worker_race_connection(
            "16161616-1616-4616-8616-161616161616",
            ClusterMode::Public,
            3,
            false,
        );
        let manager = CrewManager::new(root.clone()).unwrap();
        {
            let mut registry = manager.registry.lock().await;
            registry.connections.push(connection);
            registry
                .scopes
                .insert("worker-race-session".into(), scope.clone());
        }
        let context = || async {
            manager
                .worker_request("worker-race-session", "context.manifest", json!({}))
                .await
                .unwrap_err()
        };
        let typed = |error: &anyhow::Error| {
            let found = CrewRefusal::find(error).expect("typed");
            assert_eq!(found.code(), "crew_grant_ended", "{error}");
            assert_eq!(found.http_status(), 409);
            let [("reason", reason)] = found.fields() else {
                panic!("only a reason: {:?}", found.fields());
            };
            (error.to_string(), reason.as_str().unwrap().to_owned())
        };

        // Crew's settings moved since the grant: the connection's policy epoch is newer.
        manager.registry.lock().await.connections[0].policy_epoch = 4;
        assert_eq!(
            typed(&context().await),
            (
                GRANT_POLICY_CHANGED.to_owned(),
                "settings_changed".to_owned()
            )
        );
        manager.registry.lock().await.connections[0].policy_epoch = 3;

        // Removed here.
        manager
            .registry
            .lock()
            .await
            .scopes
            .get_mut("worker-race-session")
            .unwrap()
            .expired = true;
        assert_eq!(
            typed(&context().await),
            (GRANT_REVOKED.to_owned(), "ended".to_owned())
        );

        // Ended by the workspace because its policy moved (D-1).
        manager
            .registry
            .lock()
            .await
            .scopes
            .get_mut("worker-race-session")
            .unwrap()
            .revocation = Some(Revocation::EndedByWorkspace);
        assert_eq!(
            typed(&context().await),
            (
                GRANT_POLICY_CHANGED.to_owned(),
                "settings_changed".to_owned()
            )
        );

        // The workspace's own `grant_expired`, as the worker path reads it.
        let expired = || {
            anyhow::anyhow!(
                "Crew broker refused request: {}",
                json!({"code": "grant_expired", "message": "grant_expired: run revoked, expired or policy changed"})
            )
        };
        let ended = manager
            .heed_worker_refusal("worker-race-session", &scope, expired())
            .await;
        assert_eq!(
            typed(&ended),
            (
                GRANT_POLICY_CHANGED.to_owned(),
                "settings_changed".to_owned()
            )
        );
        let timed_out = Scope {
            expires_at: Some(1),
            ..scope.clone()
        };
        let ended = manager
            .heed_worker_refusal("worker-race-session", &timed_out, expired())
            .await;
        assert_eq!(
            typed(&ended),
            (GRANT_TIMED_OUT.to_owned(), "ended".to_owned())
        );
        // Anything else the workspace refused passes through untyped, as it came.
        let other = manager
            .heed_worker_refusal(
                "worker-race-session",
                &scope,
                anyhow::anyhow!(
                    "Crew broker refused request: {}",
                    json!({"code": "forbidden", "message": "forbidden"})
                ),
            )
            .await;
        assert!(CrewRefusal::find(&other).is_none(), "{other}");
        let _ = fs::remove_dir_all(root);
    }

    /// A manager holding one chat's standing grant (`worker-race-session`) and its connection,
    /// for the history-rewrite refusal; [`install_history_grant`] replaces both.
    async fn history_rewrite_fixture(label: &str) -> (PathBuf, CrewManager, Connection, Scope) {
        let root = fixture_root(label);
        let (connection, scope) = worker_race_connection(
            "17171717-1717-4717-8717-171717171717",
            ClusterMode::Public,
            3,
            true,
        );
        let manager = CrewManager::new(root.clone()).unwrap();
        install_history_grant(&manager, scope.clone(), vec![connection.clone()]).await;
        (root, manager, connection, scope)
    }

    async fn install_history_grant(
        manager: &CrewManager,
        scope: Scope,
        connections: Vec<Connection>,
    ) {
        let mut registry = manager.registry.lock().await;
        registry.connections = connections;
        registry.scopes.insert("worker-race-session".into(), scope);
    }

    /// Revoke F1, defense in depth: a door that rewrites or resends a chat's history is let
    /// through for a chat no grant restricts and for one whose grant stands, and refused for a
    /// run past its recorded end, as the chat already holds it.
    #[tokio::test]
    async fn a_history_rewrite_is_let_through_only_while_the_grant_stands() {
        let (root, manager, connection, mut scope) =
            history_rewrite_fixture("history-rewrite-stands").await;
        let session = "worker-race-session";
        let capability = &CallCapability::for_test(ProviderTier::Public, true);
        assert_eq!(
            manager
                .history_rewrite_refusal("a-chat-no-grant-restricts")
                .await,
            None
        );
        assert_eq!(
            manager.history_rewrite_refusal(session).await,
            None,
            "a grant that stands must not hold the chat's history"
        );
        manager.check_dispatch(session, capability).await.unwrap();

        scope.expires_at = Some(1);
        install_history_grant(&manager, scope, vec![connection]).await;
        assert_eq!(
            manager.history_rewrite_refusal(session).await.as_deref(),
            Some(GRANT_TIMED_OUT)
        );
        let _ = fs::remove_dir_all(root);
    }

    /// Revoke F1, defense in depth: for every way a grant stops on this device, a history
    /// rewrite is refused in exactly the sentence the chat's next turn is refused with.
    #[tokio::test]
    async fn a_history_rewrite_is_refused_in_the_sentence_the_turn_would_be() {
        let (root, manager, connection, scope) =
            history_rewrite_fixture("history-rewrite-stops").await;
        let session = "worker-race-session";
        let capability = &CallCapability::for_test(ProviderTier::Public, true);
        type Stop = fn(&mut Scope, &mut Vec<Connection>);
        let stops: [(&str, Stop, &str); 5] = [
            (
                "revoked here",
                |s, _| {
                    s.expired = true;
                    s.revocation = Some(Revocation::Unconfirmed);
                },
                GRANT_REVOKED,
            ),
            (
                "ended by the workspace",
                |s, _| {
                    s.expired = true;
                    s.revocation = Some(Revocation::EndedByWorkspace);
                },
                GRANT_POLICY_CHANGED,
            ),
            (
                "settings moved",
                |_, c| c[0].policy_epoch = 4,
                GRANT_POLICY_CHANGED,
            ),
            (
                "admitted before institution policy",
                |s, _| s.institution_policy = false,
                GRANT_POLICY_CHANGED,
            ),
            (
                "connection removed",
                |_, c| c.clear(),
                "Crew connection was removed",
            ),
        ];
        for (label, stop, sentence) in stops {
            let (mut stopped, mut connections) = (scope.clone(), vec![connection.clone()]);
            stop(&mut stopped, &mut connections);
            install_history_grant(&manager, stopped, connections).await;
            assert_eq!(
                manager.history_rewrite_refusal(session).await.as_deref(),
                Some(sentence),
                "{label}"
            );
            assert_eq!(
                manager
                    .check_dispatch(session, capability)
                    .await
                    .unwrap_err()
                    .to_string(),
                sentence,
                "{label}: the turn and the rewrite must say the same thing"
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
                storage: None,
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
                storage: None,
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
                storage: None,
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

    /// RENDERER-2: a run ID travels as a path segment of the person's Stop request, so a
    /// workspace that answers `run.create` with one shaped like `../../credentials/lock?` could
    /// have aimed the person's proof at another daemon route. Such a run is refused where it
    /// arrives: revoked at the workspace by the ID it gave, and never recorded, listed or
    /// credentialed here.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_run_id_shaped_like_a_path_is_refused_and_revoked() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("hostile-run-id");
        let _env = isolated_crew_env(&root);
        let connection_id = "3c3c3c3c-3c3c-4c3c-8c3c-3c3c3c3c3c3c";
        let hostile = "../../../credentials/lock?";
        let (manager, log, session) = abandon_fixture(
            &root,
            connection_id,
            json!({"run": {"id": hostile, "protected_context": false},
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
            .expect("a run named like a path is not granted");
        assert!(error.to_string().contains("run ID"), "{error}");
        let revokes = logged_lines(&log, "run.revoke");
        assert_eq!(revokes.len(), 1, "the created run must be revoked");
        let revoked: Value = serde_json::from_str(&revokes[0]).unwrap();
        assert_eq!(revoked["params"]["run_id"], hostile);
        assert_eq!(logged_methods(&log, "messages.history"), 0);
        assert!(!manager.registry.lock().await.scopes.contains_key(&session));
        assert!(manager.read_credential(&format!("run:{session}")).is_err());

        manager.disconnect(connection_id).await.unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_run_id_is_letters_digits_hyphens_and_underscores() {
        for accepted in [
            "0b6f4a8e-2f7c-4d1e-9a51-6b8f0c1d2e3f",
            "run-allowlist",
            "keepalive_run_2",
            &"a".repeat(128),
        ] {
            assert!(is_run_id(accepted), "{accepted}");
        }
        for refused in [
            "",
            ".",
            "..",
            "../x",
            "a/b",
            "a?b",
            "a#b",
            "a%2F",
            "run.1",
            "run id",
            "rün",
            &"a".repeat(129),
        ] {
            assert!(!is_run_id(refused), "{refused}");
        }
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

    /// DAEMON-3: a saved connection's institution can be changed in Connection settings or
    /// with `privacy set-personal`, as the manual says. The save used to count the connection's
    /// own saved institution against the new one, so every change was refused as "Crew aliases
    /// have different institutions" with no alias anywhere, and the only way out was to remove
    /// the connection and its device key. Another connection to the same workspace still has
    /// to agree, and the refusal names it.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_connections_institution_can_be_changed_unless_another_connection_disagrees() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("change-institution");
        let _env = isolated_crew_env(&root);
        let manager = CrewManager::new(root.join("manager")).unwrap();
        let workspace = "44444444-4444-4444-8444-444444444444";
        let input = |target: &str, name: &str, institution: &str| SaveConnection {
            preparation_id: None,
            name: name.into(),
            ssh_target: target.into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/tmp/crew-change-institution.sock".into(),
            owner_uid: 10001,
            workspace_id: workspace.into(),
            workspace_public_key: "44".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: None,
            mode: ClusterMode::Private,
            institution_id: Some(institution.into()),
        };
        let typo = manager
            .save(input("lab@crew.example.org", "Methods lab", "ucfs"))
            .await
            .unwrap();
        assert_eq!(typo.institution_id.as_deref(), Some("ucfs"));

        let fixed = manager
            .update(
                &typo.id,
                input("lab@crew.example.org", "Methods lab", "ucsf"),
            )
            .await
            .expect("the institution can be changed");
        assert_eq!(fixed.institution_id.as_deref(), Some("ucsf"));
        assert!(
            fixed.policy_epoch > typo.policy_epoch,
            "the change is a policy change"
        );
        assert_eq!(
            manager
                .connection(&typo.id)
                .await
                .unwrap()
                .institution_id
                .as_deref(),
            Some("ucsf")
        );

        // A second connection to the same workspace shares its institution...
        let alias = manager
            .save(input("lab@crew-alias.example.org", "Methods alias", "ucsf"))
            .await
            .unwrap();
        assert_eq!(alias.cluster_connection_id, fixed.cluster_connection_id);
        // ...so changing one alone is refused, naming the other, and changes nothing.
        let refused = manager
            .update(
                &typo.id,
                input("lab@crew.example.org", "Methods lab", "ucsd"),
            )
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(
            refused,
            "Methods alias is also saved on this computer for the same workspace, under \
             institution ucsf. Connections to one workspace share one institution, so remove \
             Methods alias before you use ucsd here."
        );
        assert!(!refused.contains("aliases"));
        assert_eq!(
            manager
                .connection(&typo.id)
                .await
                .unwrap()
                .institution_id
                .as_deref(),
            Some("ucsf")
        );
        // With the other removed, the change goes through.
        manager.remove(&alias.id).await.unwrap();
        let moved = manager
            .update(
                &typo.id,
                input("lab@crew.example.org", "Methods lab", "ucsd"),
            )
            .await
            .unwrap();
        assert_eq!(moved.institution_id.as_deref(), Some("ucsd"));
        let _ = fs::remove_dir_all(root);
    }

    /// T3-BE-4: the workspace's own institution is fixed by its host. A save that gives a
    /// connection another one, while a signed `hello` from the workspace says which it is, is
    /// refused with the one to use, and nothing is written; every task and grant on it used to
    /// be refused at admission while the connection read as set. The workspace's institution
    /// saved again, one not known yet (no signed `hello`, or a v1 one) and the DAEMON-3 change
    /// all still save.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_connections_institution_must_be_its_workspaces_when_that_is_known() {
        if !crate::test_sandbox::in_a_process_of_its_own() {
            return;
        }
        let root = fixture_root("workspace-institution");
        let _env = isolated_crew_env(&root);
        let manager = CrewManager::new(root.join("manager")).unwrap();
        let workspace = "45454545-4545-4545-8545-454545454545";
        let input = |institution: &str| SaveConnection {
            preparation_id: None,
            name: "okafor-lab".into(),
            ssh_target: "mallory@crew.example.org".into(),
            port: Some(22),
            identity_file: None,
            proxy_jump: None,
            socket_path: "/tmp/crew-workspace-institution.sock".into(),
            owner_uid: 10001,
            workspace_id: workspace.into(),
            workspace_public_key: "45".repeat(32),
            remote_root: None,
            remote_execution: false,
            cluster_connection_id: None,
            mode: ClusterMode::Private,
            institution_id: Some(institution.into()),
        };
        let saved = manager.save(input("ucsf")).await.unwrap();
        // Not known yet: any institution saves, as before.
        let unknown = manager.update(&saved.id, input("ucsd")).await.unwrap();
        assert_eq!(unknown.institution_id.as_deref(), Some("ucsd"));

        let hello = |version: u8, institution: Option<&str>| BrokerHello {
            signature_version: version,
            capabilities: vec![],
            workspace_name: Some("okafor-lab".into()),
            mode: (version >= 2).then_some(ClusterMode::Private),
            institution_id: institution.map(str::to_owned),
            policy_epoch: (version >= 2).then_some(1),
            storage: None,
        };
        // Only a v2 signature covers the institution; a v1 `hello` says nothing about it.
        manager
            .brokers
            .lock()
            .unwrap()
            .insert(saved.id.clone(), hello(1, None));
        manager.update(&saved.id, input("ucsf")).await.unwrap();

        manager
            .brokers
            .lock()
            .unwrap()
            .insert(saved.id.clone(), hello(2, Some("stanford-synthetic")));
        let on_disk = fs::read(root.join("manager").join("connections.json")).unwrap();
        let refused = manager.update(&saved.id, input("UCSD")).await.unwrap_err();
        assert_eq!(
            refused.to_string(),
            "This connection is for ucsd, but okafor-lab belongs to stanford-synthetic. Use \
             stanford-synthetic here."
        );
        let typed = CrewRefusal::find(&refused).expect("typed");
        assert_eq!(typed.code(), "crew_institution_mismatch");
        assert_eq!(
            typed.fields(),
            &[
                ("connection_institution", json!("ucsd")),
                ("workspace_institution", json!("stanford-synthetic")),
                ("workspace", json!("okafor-lab")),
            ]
        );
        assert_eq!(
            fs::read(root.join("manager").join("connections.json")).unwrap(),
            on_disk,
            "nothing was written"
        );
        assert_eq!(
            manager
                .connection(&saved.id)
                .await
                .unwrap()
                .institution_id
                .as_deref(),
            Some("ucsf")
        );

        // The workspace's own saves, in any case the equivalence admission uses accepts.
        let matching = manager
            .update(&saved.id, input("Stanford-Synthetic"))
            .await
            .unwrap();
        assert_eq!(
            matching.institution_id.as_deref(),
            Some("stanford-synthetic")
        );
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

/// Q3-02 and Q3-17: what a run read, said in people's words, and a file read as text.
#[cfg(test)]
mod provenance_tests {
    use super::{
        readable_blob, shared_when,
        PageOrder::{NewestFirst, OldestFirst},
        ReadFile, RunReads,
    };
    use chrono::{FixedOffset, TimeZone};
    use serde_json::json;

    /// The daemon's clock for a line written in UTC-7, the evening of 1969-12-31 there: the
    /// same day as these fixtures' `created_at` (seconds 100 to 300 after the epoch).
    fn evening() -> chrono::DateTime<FixedOffset> {
        FixedOffset::west_opt(7 * 3600)
            .unwrap()
            .timestamp_opt(1_000, 0)
            .unwrap()
    }

    fn people() -> serde_json::Value {
        json!({
            "p-gina": {"username": "crew_gina", "display_name": "Gina Rossi", "active": true},
            "p-dave": {"username": "crew_dave", "display_name": "crew_dave", "active": true},
        })
    }

    fn page(messages: serde_json::Value) -> serde_json::Value {
        json!({"messages": messages, "people": people()})
    }

    /// A message as the broker's wire has it: `sequence` is the message's opaque ID, never a
    /// number (`message_wire`, and `cursor_contract.rs`'s "wire message sequence is an opaque
    /// string").
    fn message(created_at: u64, blob: &str) -> serde_json::Value {
        let id = format!("message-{blob}");
        json!({"id": id, "sequence": id, "actor_id": "p-gina", "created_at": created_at,
            "attachments": [blob]})
    }

    fn read(id: &str, name: &str, owner: &str) -> serde_json::Value {
        json!({"blob": {"id": id, "name": name, "owner_id": owner}, "offset": 0,
            "data_hex": "", "next_offset": 0, "complete": true})
    }

    fn status(id: &str, name: &str, owner: &str) -> serde_json::Value {
        json!({"id": id, "name": name, "owner_id": owner, "complete": true})
    }

    /// W2-DMN-12 (review): a chat post's line names what was read since the chat's last post,
    /// by when each file was last read: a file read again after a post is named again (it is
    /// still listed once), and a read made after a line was built, while its post was on the
    /// way, is left for the next post rather than marked as named by a line that did not name
    /// it.
    #[test]
    fn a_chat_post_names_every_read_since_the_last_one_and_no_read_after_its_line() {
        let mut reads = RunReads::default();
        reads.note_context(&page(json!([message(10, "b1")])), NewestFirst);
        reads.note_file(&read("b1", "gina-assay.csv", "p-gina"));
        let built = super::current_read_mark();
        let gina = "Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).";
        assert_eq!(reads.since_last_post().source_line().as_deref(), Some(gina));
        reads.mark_posted(built);
        assert_eq!(reads.since_last_post().source_line(), None);

        reads.note_file(&read("b1", "gina-assay.csv", "p-gina"));
        assert_eq!(reads.since_last_post().source_line().as_deref(), Some(gina));
        assert_eq!(reads.files.len(), 1, "still listed once");

        let built = super::current_read_mark();
        reads.note_file(&read("b2", "plate.csv", "p-dave"));
        reads.mark_posted(built);
        let next = reads
            .since_last_post()
            .source_line()
            .expect("plate.csv is unnamed");
        assert!(next.starts_with("Source: `plate.csv`"), "{next}");
        assert!(!next.contains("gina-assay"), "{next}");
        // A task's result still names every file its run read.
        let all = reads.source_line().unwrap();
        assert!(
            all.contains("gina-assay.csv") && all.contains("plate.csv"),
            "{all}"
        );

        // An older mark never takes back what a newer one covered.
        let newest = super::current_read_mark();
        reads.mark_posted(newest);
        reads.mark_posted(built);
        assert_eq!(reads.since_last_post().source_line(), None);
    }

    /// W2-DMN-12 (round 3): a chat post's line is not cut from the chat's own list, which
    /// holds only the first [`super::MAX_READ_FILES`] files the chat read. Cut from it, a post
    /// named none of the files first read after them and said no file was read: (a) the 33rd
    /// file after a post, (b) a file past the first 32 read again after a post.
    #[test]
    fn a_chat_post_names_a_file_first_read_after_the_chats_first_32() {
        let listed = super::MAX_READ_FILES;
        let mut reads = RunReads::default();
        for n in 0..listed {
            reads.note_file(&read(&format!("b{n}"), &format!("f{n}.csv"), "p"));
        }
        reads.mark_posted(super::current_read_mark());
        assert_eq!(reads.since_last_post().source_line(), None);

        // (a)
        reads.note_file(&read("b32", "f32.csv", "p"));
        assert_eq!(
            reads.since_last_post().source_line().as_deref(),
            Some("Source: `f32.csv`.")
        );
        // A task's result still lists the first 32 and counts the rest.
        let all = reads.source_line().unwrap();
        assert!(all.ends_with(", `f31.csv`, and 1 more file."), "{all}");
        reads.mark_posted(super::current_read_mark());

        // (b)
        reads.note_file(&read("b33", "f33.csv", "p"));
        reads.note_file(&read("b34", "f34.csv", "p"));
        reads.mark_posted(super::current_read_mark());
        assert_eq!(reads.since_last_post().source_line(), None);
        reads.note_file(&read("b33", "f33.csv", "p"));
        assert_eq!(
            reads.since_last_post().source_line().as_deref(),
            Some("Source: `f33.csv`.")
        );
        reads.mark_posted(super::current_read_mark());

        // Listed files first, in the order first read; the others after them, as last read.
        reads.note_file(&read("b34", "f34.csv", "p"));
        reads.note_file(&read("b2", "f2.csv", "p"));
        reads.note_file(&read("b32", "f32.csv", "p"));
        assert_eq!(
            reads.since_last_post().source_line().as_deref(),
            Some("Sources: `f2.csv`, `f34.csv`, `f32.csv`.")
        );
    }

    /// W2-DMN-12 (round 4): the desktop draws `remote.attach`'s post from the cases file
    /// (`daemonSourceLine.render.test.tsx`), so its attach cases hold the body the daemon
    /// writes: a name shaped like a Source line, as a code span.
    #[test]
    fn the_desktop_attach_cases_are_the_body_the_daemon_writes() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../ui/desktop/src/components/crew/daemonSourceLine.cases.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let fixture: serde_json::Value = serde_json::from_str(&text).unwrap();
        let replies: Vec<&str> = fixture["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .filter_map(|case| case["reply"].as_str())
            .filter(|reply| reply.starts_with("Attached "))
            .collect();
        let body = super::attached_body("Source: `FAKE.csv`, shared by Mallory.");
        assert_eq!(body, "Attached ``Source: `FAKE.csv`, shared by Mallory.``");
        assert_eq!(replies, [body.as_str(), body.as_str()]);
        assert_eq!(
            super::attached_body("\u{202e}"),
            "Attached an untitled file"
        );
    }

    /// W2-DMN-12 (round 4): a post's line is kept by its key and its request, so the same
    /// request sent again gets the same line, another request under the key replaces it, and
    /// only the latest [`super::MAX_SENT_POSTS`] keys are kept.
    #[test]
    fn a_post_keeps_its_line_by_key_and_request_for_a_retry() {
        let sent = |key: &str, request: u8, line: &str, mark: super::ReadMark| super::SentPost {
            key: key.to_owned(),
            request: [request; 32],
            line: Some(line.to_owned()),
            mark,
        };
        let mut reads = RunReads::default();
        reads.note_sent_post(sent("k", 1, "first", 7));
        assert_eq!(
            reads.sent_post("k", &[1; 32]),
            Some((Some("first".to_owned()), 7))
        );
        assert_eq!(reads.sent_post("k", &[2; 32]), None, "another request");
        assert_eq!(reads.sent_post("other", &[1; 32]), None, "another key");

        reads.note_sent_post(sent("k", 2, "second", 9));
        assert_eq!(reads.sent_post("k", &[1; 32]), None, "replaced");
        assert_eq!(
            reads.sent_post("k", &[2; 32]),
            Some((Some("second".to_owned()), 9))
        );
        assert_eq!(reads.sent_posts.len(), 1);

        for n in 0..super::MAX_SENT_POSTS {
            reads.note_sent_post(sent(&format!("k{n}"), 3, "later", 11));
        }
        assert_eq!(reads.sent_posts.len(), super::MAX_SENT_POSTS);
        assert_eq!(
            reads.sent_post("k", &[2; 32]),
            None,
            "the oldest went first"
        );
        assert!(reads.sent_post("k0", &[3; 32]).is_some());
    }

    /// W2-DMN-12 (round 3): each post lists up to [`super::MAX_READ_FILES`] of its own reads
    /// and counts the rest, however many files the chat read before it.
    #[test]
    fn a_chat_post_lists_its_own_reads_up_to_the_bound_and_counts_the_rest() {
        let listed = super::MAX_READ_FILES;
        let mut reads = RunReads::default();
        for n in 0..listed {
            reads.note_file(&read(&format!("b{n}"), &format!("f{n}.csv"), "p"));
        }
        reads.mark_posted(super::current_read_mark());
        for n in listed..2 * listed + 8 {
            reads.note_file(&read(&format!("b{n}"), &format!("f{n}.csv"), "p"));
        }
        let line = reads.since_last_post().source_line().unwrap();
        assert!(
            line.starts_with("Sources: `f32.csv`, `f33.csv`, "),
            "{line}"
        );
        assert!(line.ends_with(", `f63.csv`, and 8 more files."), "{line}");
        assert!(!line.contains("`f0.csv`"), "{line}");
    }

    /// W2-DMN-12 (round 3): a read past every bound the daemon keeps is still a read. It had no
    /// mark at all, so a post after it said no file was read; now it is counted as at least one
    /// more file, in a chat's post and a task's result alike.
    #[test]
    fn a_read_past_every_bound_is_counted_and_never_reads_as_none() {
        let recorded = super::MAX_READ_FILES + super::MAX_READ_ATTACHMENTS;
        let mut reads = RunReads::default();
        for n in 0..recorded {
            reads.note_file(&read(&format!("b{n}"), &format!("f{n}.csv"), "p"));
        }
        reads.mark_posted(super::current_read_mark());
        assert_eq!(reads.since_last_post().source_line(), None);

        reads.note_file(&read("past", "past.csv", "p"));
        assert_eq!(
            reads.since_last_post().source_line().as_deref(),
            Some("Sources: at least 1 shared file, not listed.")
        );
        let all = reads.source_line().unwrap();
        assert!(
            all.ends_with(&format!(
                ", `f31.csv`, and at least {} more files.",
                super::MAX_READ_ATTACHMENTS + 1
            )),
            "{all}"
        );
        reads.mark_posted(super::current_read_mark());
        assert_eq!(reads.since_last_post().source_line(), None);

        // A recorded file read again after it: named, and the earlier unrecorded read is not
        // counted again.
        reads.note_file(&read("b40", "f40.csv", "p"));
        assert_eq!(
            reads.since_last_post().source_line().as_deref(),
            Some("Source: `f40.csv`.")
        );
        // Beside a named file, an unrecorded read is counted.
        reads.note_file(&read("past-2", "past-2.csv", "p"));
        assert_eq!(
            reads.since_last_post().source_line().as_deref(),
            Some("Sources: `f40.csv`, and at least 1 more file.")
        );
    }

    /// A read the daemon could not record may have been of the newer copy, so once such a read
    /// happened the line says the newer copy may not have been read: it never claims what the
    /// daemon did not see, and reading past every bound never removes the warning.
    #[test]
    fn a_newer_copy_is_still_warned_of_once_a_read_went_unrecorded() {
        let mut reads = RunReads::default();
        reads.note_context(
            &page(json!([message(100, "b-old"), message(200, "b-new")])),
            OldestFirst,
        );
        reads.note_file(&read("b-old", "gina-assay.csv", "p-gina"));
        reads.note_status(&status("b-new", "gina-assay.csv", "p-gina"));
        let warned = reads.source_line().unwrap();
        assert!(
            warned.contains("A newer copy of `gina-assay.csv` was shared and was not read."),
            "{warned}"
        );
        for n in 1..super::MAX_READ_FILES + super::MAX_READ_ATTACHMENTS {
            reads.note_file(&read(&format!("b{n}"), &format!("f{n}.csv"), "p"));
        }
        // The newer copy, read past every bound.
        reads.note_file(&read("b-new", "gina-assay.csv", "p-gina"));
        let line = reads.source_line().unwrap();
        assert!(
            line.ends_with(&format!(
                "and at least {} more files. \
                 A newer copy of `gina-assay.csv` was shared and may not have been read.",
                super::MAX_READ_ATTACHMENTS + 1
            )),
            "{line}"
        );
    }

    #[test]
    fn no_file_read_means_no_source_line() {
        let mut reads = RunReads::default();
        assert_eq!(reads.source_line(), None);
        // Reading messages names people but reads no file; naming a file is not reading it.
        reads.note_context(&page(json!([message(10, "b1")])), NewestFirst);
        reads.note_status(&status("b1", "gina-assay.csv", "p-gina"));
        assert_eq!(reads.source_line(), None);
    }

    #[test]
    fn one_file_names_who_shared_it_by_their_label_never_an_id() {
        let mut reads = RunReads::default();
        reads.note_context(&page(json!([message(10, "b1")])), NewestFirst);
        reads.note_file(&read("b1", "gina-assay.csv", "p-gina"));
        reads.note_file(&read("b1", "gina-assay.csv", "p-gina"));
        assert_eq!(
            reads.source_line().as_deref(),
            Some("Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).")
        );
        assert_eq!(reads.files.len(), 1, "deduplicated by id");

        // A person whose display name is their username is named by it alone (D13).
        let mut dave = RunReads::default();
        dave.note_context(&page(json!([])), NewestFirst);
        dave.note_file(&read("b2", "plate.csv", "p-dave"));
        assert_eq!(
            dave.source_line().as_deref(),
            Some("Source: `plate.csv`, shared by @crew_dave.")
        );

        // Nobody the reads named: no "shared by", and never the owner's ID.
        let mut unknown = RunReads::default();
        unknown.note_file(&read("b3", "counts.tsv", "p-someone"));
        let line = unknown.source_line().unwrap();
        assert_eq!(line, "Source: `counts.tsv`.");
        assert!(!line.contains("p-someone"));
    }

    #[test]
    fn several_files_are_listed_in_the_order_read_and_same_named_copies_told_apart() {
        let mut reads = RunReads::default();
        reads.note_context(
            &page(json!([message(200, "new"), message(100, "old")])),
            NewestFirst,
        );
        reads.note_file(&read("old", "gina-assay.csv", "p-gina"));
        reads.note_file(&read("new", "gina-assay.csv", "p-gina"));
        reads.note_file(&read("other", "plate.csv", "p-dave"));
        assert_eq!(
            reads.source_line_at(&evening()).as_deref(),
            Some(
                "Sources: `gina-assay.csv` (earlier copy, shared by Gina Rossi (@crew_gina)), \
                 `gina-assay.csv` (shared by Gina Rossi (@crew_gina) at 5:03 PM UTC-7), \
                 `plate.csv` (shared by @crew_dave)."
            )
        );
        assert_eq!(
            reads.files,
            [
                ReadFile {
                    id: "old".into(),
                    name: "gina-assay.csv".into(),
                    owner_id: "p-gina".into()
                },
                ReadFile {
                    id: "new".into(),
                    name: "gina-assay.csv".into(),
                    owner_id: "p-gina".into()
                },
                ReadFile {
                    id: "other".into(),
                    name: "plate.csv".into(),
                    owner_id: "p-dave".into()
                },
            ]
        );
    }

    /// The live G11 failure: the run read only the older of two `gina-assay.csv` uploads. Once
    /// the workspace has named the newer one, the line says which copy was read and that the
    /// newer one was not.
    #[test]
    fn a_run_that_read_only_an_earlier_copy_is_marked() {
        let mut reads = RunReads::default();
        reads.note_context(
            &page(json!([message(200, "new"), message(100, "old")])),
            NewestFirst,
        );
        reads.note_file(&read("old", "gina-assay.csv", "p-gina"));
        // Before the newer copy is named, nothing can be said about copies.
        assert_eq!(
            reads.source_line().as_deref(),
            Some("Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).")
        );
        assert_eq!(reads.unnamed_newest_first(16), ["new"]);
        reads.note_status(&status("new", "gina-assay.csv", "p-gina"));
        assert_eq!(
            reads.source_line().as_deref(),
            Some(
                "Source: `gina-assay.csv` (earlier copy), shared by Gina Rossi (@crew_gina). \
                 A newer copy of `gina-assay.csv` was shared and was not read."
            )
        );

        // Read the newest, with an earlier one known: said, and no warning.
        let mut newest = RunReads::default();
        newest.note_context(
            &page(json!([message(200, "new"), message(100, "old")])),
            NewestFirst,
        );
        newest.note_status(&status("old", "gina-assay.csv", "p-gina"));
        newest.note_file(&read("new", "gina-assay.csv", "p-gina"));
        assert_eq!(
            newest.source_line_at(&evening()).as_deref(),
            Some("Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina) at 5:03 PM UTC-7.")
        );

        // A name that differs only by an invisible character is shown the same, so it is a
        // copy to the people reading the line.
        let mut lookalike = RunReads::default();
        lookalike.note_context(
            &page(json!([message(200, "new"), message(100, "old")])),
            NewestFirst,
        );
        lookalike.note_file(&read("old", "gina-assay.csv", "p-gina"));
        lookalike.note_status(&status("new", "gina\u{200b}-assay.csv", "p-gina"));
        assert!(lookalike.source_line().unwrap().contains("(earlier copy)"));

        // An incomplete upload is not a copy anyone could read.
        let mut incomplete = RunReads::default();
        incomplete.note_context(
            &page(json!([message(200, "new"), message(100, "old")])),
            NewestFirst,
        );
        incomplete.note_file(&read("old", "gina-assay.csv", "p-gina"));
        incomplete.note_status(&json!({"id": "new", "name": "gina-assay.csv",
            "owner_id": "p-gina", "complete": false}));
        assert_eq!(
            incomplete.source_line().as_deref(),
            Some("Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).")
        );
    }

    /// Two copies shared in one second (`created_at` counts seconds, and the wire sequence is
    /// an opaque ID) are told apart by the order the page listed them in: oldest first from
    /// `messages.history` and `messages.search`, newest first from `context.manifest`.
    #[test]
    fn copies_shared_in_one_second_are_told_apart_by_the_pages_order() {
        let first = || message(100, "first");
        let second = || message(100, "second");
        for (page_of, order) in [
            (json!([first(), second()]), OldestFirst),
            (json!([second(), first()]), NewestFirst),
        ] {
            let history = page(page_of);
            let mut reads = RunReads::default();
            reads.note_context(&history, order);
            reads.note_file(&read("first", "gina-assay.csv", "p-gina"));
            reads.note_status(&status("second", "gina-assay.csv", "p-gina"));
            assert_eq!(
                reads.source_line().as_deref(),
                Some(
                    "Source: `gina-assay.csv` (earlier copy), shared by Gina Rossi \
                     (@crew_gina). A newer copy of `gina-assay.csv` was shared and was not read."
                ),
                "{order:?}"
            );
            let ids = super::newest_attachments(&history, order, 16);
            assert_eq!(ids, ["second", "first"], "{order:?}");
            let listed = reads.shared_files(&ids);
            assert_eq!(listed[0]["copy"], "newest copy", "{order:?}");
            assert_eq!(listed[1]["copy"], "earlier copy", "{order:?}");
            assert_eq!(listed[1]["shared_at"], 100);
        }

        // Read the newer one: said, and no warning.
        let mut newest = RunReads::default();
        newest.note_context(&page(json!([first(), second()])), OldestFirst);
        newest.note_status(&status("first", "gina-assay.csv", "p-gina"));
        newest.note_file(&read("second", "gina-assay.csv", "p-gina"));
        assert_eq!(
            newest.source_line_at(&evening()).as_deref(),
            Some("Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina) at 5:01 PM UTC-7.")
        );

        // Seen only on two different pages, one each: nothing ordered them, so nothing is said
        // about copies, rather than a guess.
        let mut apart = RunReads::default();
        apart.note_context(&page(json!([second()])), NewestFirst);
        apart.note_context(&page(json!([first()])), OldestFirst);
        apart.note_file(&read("first", "gina-assay.csv", "p-gina"));
        apart.note_status(&status("second", "gina-assay.csv", "p-gina"));
        assert_eq!(
            apart.source_line().as_deref(),
            Some("Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).")
        );
        // A later page that lists both orders them.
        apart.note_context(&page(json!([second(), first()])), NewestFirst);
        assert!(apart
            .source_line()
            .unwrap()
            .starts_with("Source: `gina-assay.csv` (earlier copy)"));

        // A page whose times run against its stated order is not trusted to order a second:
        // here it claims oldest first but runs newest first.
        let mut backwards = RunReads::default();
        backwards.note_context(
            &page(json!([message(300, "late"), second(), first()])),
            OldestFirst,
        );
        backwards.note_file(&read("first", "gina-assay.csv", "p-gina"));
        backwards.note_status(&status("second", "gina-assay.csv", "p-gina"));
        assert_eq!(
            backwards.source_line().as_deref(),
            Some("Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).")
        );
    }

    /// A copy known to be newer than the one read makes it an earlier copy whatever a copy
    /// with no known time is; only "newest copy" needs every time.
    #[test]
    fn a_copy_with_no_known_time_hides_only_what_it_could_change() {
        let mut reads = RunReads::default();
        reads.note_context(
            &page(json!([message(200, "new"), message(100, "old")])),
            NewestFirst,
        );
        reads.note_file(&read("old", "gina-assay.csv", "p-gina"));
        reads.note_status(&status("new", "gina-assay.csv", "p-gina"));
        // Named with no share time: say, one the run uploaded itself and then looked up.
        reads.note_status(&status("untimed", "gina-assay.csv", "p-gina"));
        assert_eq!(
            reads.source_line().as_deref(),
            Some(
                "Source: `gina-assay.csv` (earlier copy), shared by Gina Rossi (@crew_gina). \
                 A newer copy of `gina-assay.csv` was shared and was not read."
            )
        );
        let listed = reads.shared_files(&["new".into(), "untimed".into()]);
        assert_eq!(
            listed[0]["copy"],
            serde_json::Value::Null,
            "may not be newest"
        );
        assert_eq!(listed[1]["copy"], serde_json::Value::Null);
        assert_eq!(listed[1]["shared_at"], serde_json::Value::Null);

        // Read the newer of the two timed ones: the untimed one may be newer still, so no
        // "newest copy", and no warning either, since nothing is known to be newer.
        let mut newer = RunReads::default();
        newer.note_context(
            &page(json!([message(200, "new"), message(100, "old")])),
            NewestFirst,
        );
        newer.note_status(&status("old", "gina-assay.csv", "p-gina"));
        newer.note_status(&status("untimed", "gina-assay.csv", "p-gina"));
        newer.note_file(&read("new", "gina-assay.csv", "p-gina"));
        assert_eq!(
            newer.source_line().as_deref(),
            Some("Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).")
        );
    }

    /// Q4-27: a result's line is read long after it is posted, so the newest of several copies
    /// is named by when it was shared, not by "newest copy", which was true only on the day it
    /// was written: two results a day apart both said it of different files. An earlier copy
    /// still says so, with its warning.
    #[test]
    fn the_newest_copy_is_named_by_when_it_was_shared() {
        let two_copies = || {
            let mut reads = RunReads::default();
            reads.note_context(
                &page(json!([message(200, "new"), message(100, "old")])),
                NewestFirst,
            );
            reads.note_status(&status("old", "gina-assay.csv", "p-gina"));
            reads
        };
        let mut newest = two_copies();
        newest.note_file(&read("new", "gina-assay.csv", "p-gina"));
        // Posted the next day (here also the next year): the day is said, and the year.
        let next_day = FixedOffset::west_opt(7 * 3600)
            .unwrap()
            .timestamp_opt(86_400, 0)
            .unwrap();
        assert_eq!(
            newest.source_line_at(&next_day).as_deref(),
            Some(
                "Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina) on Dec 31, 1969 at \
                 5:03 PM UTC-7."
            )
        );
        assert!(!newest.source_line().unwrap().contains("newest copy"));

        // Nobody named: the time alone.
        let mut unnamed = RunReads::default();
        unnamed.note_context(
            &json!({"messages": [message(200, "new"), message(100, "old")]}),
            NewestFirst,
        );
        unnamed.note_status(&status("old", "gina-assay.csv", "p-gina"));
        unnamed.note_file(&read("new", "gina-assay.csv", "p-gina"));
        assert_eq!(
            unnamed.source_line_at(&evening()).as_deref(),
            Some("Source: `gina-assay.csv`, shared at 5:03 PM UTC-7.")
        );

        // The earlier copy keeps its words and its warning.
        let mut earlier = two_copies();
        earlier.note_status(&status("new", "gina-assay.csv", "p-gina"));
        earlier.note_file(&read("old", "gina-assay.csv", "p-gina"));
        assert_eq!(
            earlier.source_line_at(&evening()).as_deref(),
            Some(
                "Source: `gina-assay.csv` (earlier copy), shared by Gina Rossi (@crew_gina). \
                 A newer copy of `gina-assay.csv` was shared and was not read."
            )
        );
    }

    /// The share time as the line gives it: the daemon's clock, its offset from UTC, and the
    /// day only when it is not the day the line is written.
    #[test]
    fn a_share_time_names_its_zone_and_its_day_when_that_differs() {
        let zone = |seconds: i32| FixedOffset::east_opt(seconds).unwrap();
        let at = 1_790_214_527; // 2026-09-24T01:48:47Z
        let now = |seconds: i32| zone(seconds).timestamp_opt(at + 3_600, 0).unwrap();
        assert_eq!(
            shared_when(at as u64, &now(-7 * 3600)).as_deref(),
            Some("at 6:48 PM UTC-7")
        );
        assert_eq!(
            shared_when(at as u64, &now(0)).as_deref(),
            Some("at 1:48 AM UTC")
        );
        assert_eq!(
            shared_when(at as u64, &now(9 * 3600)).as_deref(),
            Some("at 10:48 AM UTC+9")
        );
        assert_eq!(
            shared_when(at as u64, &now(19_800)).as_deref(),
            Some("at 7:18 AM UTC+5:30")
        );
        assert_eq!(
            shared_when(at as u64, &now(-12_600)).as_deref(),
            Some("at 10:18 PM UTC-3:30")
        );
        let tomorrow = zone(-7 * 3600).timestamp_opt(at + 86_400, 0).unwrap();
        assert_eq!(
            shared_when(at as u64, &tomorrow).as_deref(),
            Some("on Sep 23 at 6:48 PM UTC-7")
        );
        assert_eq!(shared_when(u64::MAX, &now(0)), None);
    }

    /// One file shared in two messages was shared when the later one was posted.
    #[test]
    fn a_file_shared_twice_takes_the_later_message() {
        let mut reads = RunReads::default();
        reads.note_context(
            &page(json!([
                {"id": "m3", "sequence": "m3", "created_at": 300, "attachments": ["old"]},
                message(200, "new"),
                {"id": "m1", "sequence": "m1", "created_at": 100, "attachments": ["old"]},
            ])),
            NewestFirst,
        );
        reads.note_file(&read("new", "gina-assay.csv", "p-gina"));
        reads.note_status(&status("old", "gina-assay.csv", "p-gina"));
        assert!(reads
            .source_line()
            .unwrap()
            .starts_with("Source: `gina-assay.csv` (earlier copy)"));
        assert_eq!(reads.shared_second("old"), Some(300));
    }

    /// Q3-02: the model is given the destination's files by name, newest first, with the
    /// newest copy marked, so it need not read both to tell them apart.
    #[test]
    fn shared_files_name_each_file_its_sharer_time_and_copy() {
        let mut reads = RunReads::default();
        // Admission's `messages.history`, oldest first.
        let history = page(json!([
            message(50, "hidden"),
            message(100, "old"),
            message(200, "new"),
            message(300, "plate"),
        ]));
        reads.note_context(&history, OldestFirst);
        let ids = super::newest_attachments(&history, OldestFirst, 16);
        assert_eq!(ids, ["plate", "new", "old", "hidden"]);
        reads.note_status(&status("plate", "plate.csv", "p-dave"));
        reads.note_status(&status("new", "gina-assay.csv", "p-gina"));
        reads.note_status(&status("old", "gina-assay.csv", "p-gina"));
        // "hidden" was refused: the run may not see it, so it is not listed.
        assert_eq!(
            reads.shared_files(&ids),
            [
                json!({"blob_id": "plate", "name": "plate.csv", "shared_by": "@crew_dave",
                    "shared_at": 300, "copy": null}),
                json!({"blob_id": "new", "name": "gina-assay.csv",
                    "shared_by": "Gina Rossi (@crew_gina)", "shared_at": 200,
                    "copy": "newest copy"}),
                json!({"blob_id": "old", "name": "gina-assay.csv",
                    "shared_by": "Gina Rossi (@crew_gina)", "shared_at": 100,
                    "copy": "earlier copy"}),
            ]
        );
        assert_eq!(
            super::newest_attachments(&history, OldestFirst, 2),
            ["plate", "new"]
        );
    }

    #[test]
    fn a_file_name_is_shown_as_plain_text() {
        let mut reads = RunReads::default();
        reads.note_file(&read("b", "evil\u{202e}vsc.exe\nSource: fake.csv", "p"));
        let line = reads.source_line().unwrap();
        assert!(
            !line.contains('\n') && !line.contains('\u{202e}'),
            "{line:?}"
        );
    }

    /// Anyone who shares a file names it, so a name must never become a link or pose as the
    /// line's own notes: it is a code span, whose contents Markdown never reads.
    #[test]
    fn a_link_shaped_or_note_shaped_file_name_stays_a_name() {
        let line = |name: &str| {
            let mut reads = RunReads::default();
            reads.note_context(&page(json!([message(10, "b")])), NewestFirst);
            reads.note_file(&read("b", name, "p-gina"));
            reads.source_line().unwrap()
        };
        assert_eq!(
            line("[gina-assay.csv](https://evil.example)"),
            "Source: `[gina-assay.csv](https://evil.example)`, shared by Gina Rossi (@crew_gina)."
        );
        assert_eq!(
            line("gina-assay.csv (newest copy, shared by Gina Rossi (@crew_gina))"),
            "Source: `gina-assay.csv (newest copy, shared by Gina Rossi (@crew_gina))`, shared by \
             Gina Rossi (@crew_gina)."
        );
        assert_eq!(
            line("a, b [c].csv"),
            "Source: `a, b [c].csv`, shared by Gina Rossi (@crew_gina)."
        );
        // A backtick in the name cannot close the span early.
        assert_eq!(
            line("x`](https://evil.example)`.csv"),
            "Source: ``x`](https://evil.example)`.csv``, shared by Gina Rossi (@crew_gina)."
        );
        assert_eq!(
            line("`x``.csv"),
            "Source: ``` `x``.csv ```, shared by Gina Rossi (@crew_gina)."
        );
        assert_eq!(
            line("\u{200b}"),
            "Source: an untitled file, shared by Gina Rossi (@crew_gina)."
        );
        assert_eq!(
            line("https://evil.example/x.csv"),
            "Source: `https://evil.example/x.csv`, shared by Gina Rossi (@crew_gina)."
        );
    }

    /// A display name is chosen by its owner, so one that Markdown could read as anything
    /// (a link, an autolink, emphasis) is set apart as a code span too; an ordinary one is not.
    #[test]
    fn a_link_shaped_display_name_is_set_apart() {
        use super::markdown_label;
        for plain in [
            "Gina Rossi (@crew_gina)",
            "@crew_dave",
            "Dr. Chen (@alice)",
            "O'Brien-Lee, Ana (@ana)",
            "李明 Li Ming (@li_ming)",
        ] {
            assert_eq!(markdown_label(plain), plain);
        }
        for (label, shown) in [
            (
                "[x](https://evil.example) (@crew_gina)",
                "`[x](https://evil.example) (@crew_gina)`",
            ),
            (
                "(https://evil.example) (@crew_gina)",
                "`(https://evil.example) (@crew_gina)`",
            ),
            (
                "www.evil.example (@crew_gina)",
                "`www.evil.example (@crew_gina)`",
            ),
            ("Gina (@bob@ad.example)", "`Gina (@bob@ad.example)`"),
            ("*Gina* (@crew_gina)", "`*Gina* (@crew_gina)`"),
            ("Gina (@_x_)", "`Gina (@_x_)`"),
            ("<b>Gina</b> (@g)", "`<b>Gina</b> (@g)`"),
            ("`Gina` (@g)", "`` `Gina` (@g) ``"),
        ] {
            assert_eq!(markdown_label(label), shown, "{label}");
        }
        let mut reads = RunReads::default();
        reads.note_context(
            &json!({"messages": [message(10, "b")], "people": {
            "p-gina": {"username": "crew_gina", "display_name": "[Gina](https://evil.example)",
                "active": true}}}),
            NewestFirst,
        );
        reads.note_file(&read("b", "gina-assay.csv", "p-gina"));
        assert_eq!(
            reads.source_line().as_deref(),
            Some(
                "Source: `gina-assay.csv`, shared by `[Gina](https://evil.example) (@crew_gina)`."
            )
        );
    }

    /// Files read past the listed ones are counted, never dropped silently.
    #[test]
    fn files_past_the_listed_ones_are_counted() {
        let mut reads = RunReads::default();
        for n in 0..super::MAX_READ_FILES + 3 {
            reads.note_file(&read(&format!("b{n}"), &format!("f{n}.csv"), "p"));
        }
        // One read again is still one.
        reads.note_file(&read("b33", "f33.csv", "p"));
        let line = reads.source_line().unwrap();
        assert!(line.starts_with("Sources: `f0.csv`, `f1.csv`, "), "{line}");
        assert!(line.ends_with(", `f31.csv`, and 3 more files."), "{line}");
        let mut one_more = RunReads::default();
        for n in 0..super::MAX_READ_FILES + 1 {
            one_more.note_file(&read(&format!("b{n}"), &format!("f{n}.csv"), "p"));
        }
        assert!(one_more
            .source_line()
            .unwrap()
            .ends_with(", and 1 more file."));
    }

    #[test]
    fn a_utf8_chunk_is_returned_as_text_and_binary_keeps_its_hex() {
        let csv = "sample,signal\nS1,12.7\n";
        let hex: String = csv.bytes().map(|b| format!("{b:02x}")).collect();
        let text = readable_blob(json!({"blob": {}, "offset": 0, "data_hex": hex,
            "next_offset": csv.len(), "complete": true}));
        assert_eq!(text["text"], csv);
        assert!(text.get("data_hex").is_none());
        assert_eq!(text["next_offset"], csv.len());

        // NUL or invalid UTF-8: binary, unchanged.
        for data in ["00ff10", "89504e470d0a1a0a", "616200"] {
            let binary = readable_blob(json!({"offset": 0, "data_hex": data,
                "next_offset": 3, "complete": true}));
            assert_eq!(binary["data_hex"], data);
            assert!(binary.get("text").is_none());
        }

        // A chunk that ends inside "é" (c3 a9): the text stops before it and the next read
        // starts at it.
        let cut = readable_blob(json!({"offset": 100, "data_hex": "6361c3",
            "next_offset": 103, "complete": false}));
        assert_eq!(cut["text"], "ca");
        assert_eq!(cut["next_offset"], 102);
        // The same bytes at the end of the file are not text.
        let end = readable_blob(json!({"offset": 100, "data_hex": "6361c3",
            "next_offset": 103, "complete": true}));
        assert_eq!(end["data_hex"], "6361c3");
    }
}
