//! Rootless Crew protocol and authoritative, single-writer collaboration state.
//!
//! Besides the wire and state types, this crate holds the pure functions the broker and the
//! desktop daemon must agree on byte for byte: the naming rules ([`names`]), the workspace
//! invitation codec and device code ([`invitation`]), and the signing payloads.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

mod default_ignorable;
pub mod invitation;
pub mod names;

pub use invitation::{
    device_code, device_code_from_hex, device_code_matches, format_device_code,
    normalize_device_code, DeviceCodeError, DEVICE_CODE_LEN,
};
pub use names::{
    canonical_channel_name, clean, display_name_claims_username, display_name_valid,
    is_default_ignorable, is_uuid_shaped, name_key, names_collide, restriction_level_ok,
    sanitize_channel_name, sanitize_display_name, sanitize_team_name, skeleton_key,
    strip_ignorable, team_handle, valid_username, validate_display_name, validate_display_name_for,
    validate_team_name, validate_workspace_name, workspace_name_valid, NameError, NameKind,
    NameProblem,
};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 1_048_576;
pub const MAX_CHUNK: usize = 262_144;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub auth: Option<DeviceAuth>,
    #[serde(default)]
    pub credential: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceAuth {
    pub device_id: String,
    pub nonce: String,
    pub signature: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: String,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Private,
    Public,
}
pub fn is_canonical_institution_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderAffiliation {
    Local,
    Institutions {
        institution_ids: Vec<String>,
    },
    #[default]
    Unstated,
}

impl ProviderAffiliation {
    pub fn allows_institution(&self, institution_id: &str) -> bool {
        is_canonical_institution_id(institution_id)
            && match self {
                Self::Local => true,
                Self::Institutions { institution_ids } => {
                    institution_ids.len() == 1 && institution_ids[0] == institution_id
                }
                Self::Unstated => false,
            }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub host_uid: u32,
    #[serde(default)]
    pub institution_id: Option<String>,
    pub mode: Mode,
    pub policy_epoch: u64,
    /// The workspace's name (`lab`), validated by [`names::validate_workspace_name`] when set.
    /// A legacy workspace has none. Serialized only when set, so a journal written before names
    /// existed replays and re-serializes byte for byte, and an older broker ignores the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Principal {
    pub id: String,
    pub uid: u32,
    pub username: String,
    pub nickname: String,
    pub avatar: Option<String>,
    pub active: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub created_by: String,
    pub members: BTreeSet<String>,
    pub general_channel_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Classification {
    PublicSafe,
    Restricted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub created_by: String,
    pub owner_id: String,
    pub members: BTreeSet<String>,
    pub archived: bool,
    pub classification: Classification,
    pub pending_owner: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invitation {
    pub id: String,
    pub kind: String,
    pub target_id: String,
    pub principal_id: String,
    pub inviter_id: String,
    pub expires_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub sequence: u64,
    pub channel_id: String,
    pub actor_id: String,
    pub run_id: Option<String>,
    pub body: String,
    pub created_at: u64,
    pub restricted: bool,
    pub source_channels: BTreeSet<String>,
    pub attachments: Vec<String>,
    #[serde(default)]
    pub references: Vec<String>,
    pub status: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub owner_id: String,
    pub channel_id: String,
    pub source_channels: BTreeSet<String>,
    pub provider_policy_id: String,
    #[serde(default)]
    pub protected_context: bool,
    #[serde(default)]
    pub provider_affiliation: ProviderAffiliation,
    #[serde(default)]
    pub workspace_institution_id: Option<String>,
    #[serde(default)]
    pub connection_institution_id: Option<String>,
    pub public_provider: bool,
    pub personal_mode: Mode,
    pub policy_epoch: u64,
    pub expires_at: u64,
    pub revoked: bool,
    #[serde(default)]
    pub remote_root: Option<String>,
    #[serde(default)]
    pub remote_execution: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Blob {
    #[serde(default)]
    pub run_id: Option<String>,
    pub id: String,
    pub owner_id: String,
    pub channel_id: String,
    pub name: String,
    pub media_type: String,
    pub size: u64,
    pub sha256: String,
    pub offset: u64,
    pub complete: bool,
    pub restricted: bool,
    pub source_channels: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteReference {
    pub id: String,
    pub channel_id: String,
    pub owner_id: String,
    pub path: String,
    pub label: String,
    pub restricted: bool,
    pub source_channels: BTreeSet<String>,
    pub verified: bool,
}

/// How long a pending workspace join stays valid after the host invites (24 hours).
pub const PENDING_JOIN_LIFETIME_SECS: u64 = 24 * 60 * 60;

/// The host's invitation for one Unix account to join the workspace (S3a), stored in the
/// broker's `pending_joins` map keyed by the decimal UID. Public so the daemon's tests can build
/// one.
///
/// It carries the approved device code, so it must never be serialized into a response or the
/// dedupe cache: the manager's projection lists only the username, full name, `add_device`,
/// approval state and times, never a code, key, UID or join ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingJoin {
    /// 128 random bits; a re-invite mints a new one.
    pub join_id: String,
    pub uid: u32,
    /// The canonical account name, bound at invite and rechecked at join.
    pub username: String,
    /// The validated first GECOS field; a label only.
    #[serde(default)]
    pub full_name: Option<String>,
    /// The host principal's UUID.
    pub inviter_id: String,
    /// `Some` when the join adds a device to an existing principal.
    #[serde(default)]
    pub existing_principal_id: Option<String>,
    /// Principal IDs holding this UID or username when the host invited. Required: a record
    /// without it fails to load rather than comparing against an empty generation.
    pub generation: Vec<String>,
    /// The normalized 16-character code the host approved, if any.
    #[serde(default)]
    pub approved_code: Option<String>,
    pub created_at: u64,
    /// `created_at + PENDING_JOIN_LIFETIME_SECS`.
    pub expires_at: u64,
}

impl PendingJoin {
    /// Whether the join has expired at `now` (seconds since the Unix epoch).
    pub fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }
}

/// Signing bytes of the `hello` v1 workspace signature: the JSON array
/// `[workspace_id, host_uid, challenge_nonce, workspace_public_key, node_id]`.
pub fn hello_v1_payload(
    workspace_id: &str,
    host_uid: u32,
    challenge_nonce: &str,
    workspace_public_key: &str,
    node_id: &str,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!([
        workspace_id,
        host_uid,
        challenge_nonce,
        workspace_public_key,
        node_id
    ]))
    .expect("JSON values serialize")
}

/// The fields the `hello` v2 workspace signature covers. v2 is sent beside v1 so a daemon that
/// verifies v2 can trust the workspace name, privacy mode, institution, policy epoch and
/// capabilities for display, which v1 leaves unauthenticated. `PROTOCOL_VERSION` is unchanged.
#[derive(Clone, Copy, Debug)]
pub struct HelloV2<'a> {
    pub workspace_id: &'a str,
    pub host_uid: u32,
    pub challenge_nonce: &'a str,
    /// Hex, exactly as `hello` returns it.
    pub workspace_public_key: &'a str,
    pub node_id: &'a str,
    pub mode: &'a Mode,
    pub institution_id: Option<&'a str>,
    pub policy_epoch: u64,
    pub name: Option<&'a str>,
    /// In the order `hello` returns them; the signature covers the order.
    pub capabilities: &'a [&'a str],
}

impl HelloV2<'_> {
    /// Signing bytes: the JSON array `[workspace_id, host_uid, challenge_nonce,
    /// workspace_public_key, node_id, mode, institution_id, policy_epoch, name, capabilities]`,
    /// with `null` for an absent institution or name. Ten elements, so it can never equal a v1
    /// payload (five elements) signed by the same workspace key.
    pub fn signing_payload(&self) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([
            self.workspace_id,
            self.host_uid,
            self.challenge_nonce,
            self.workspace_public_key,
            self.node_id,
            self.mode,
            self.institution_id,
            self.policy_epoch,
            self.name,
            self.capabilities
        ]))
        .expect("JSON values serialize")
    }
}

/// Signing bytes are JSON serialization of this array, with lexically sorted object keys.
pub fn signing_payload(
    workspace: &str,
    uid: u32,
    nonce: &str,
    method: &str,
    params: &Value,
) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!([
        workspace,
        uid,
        nonce,
        method,
        canonical(params)
    ]))
    .expect("JSON values serialize")
}
fn canonical(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> =
                map.iter().map(|(k, v)| (k.clone(), canonical(v))).collect();
            serde_json::to_value(sorted).expect("JSON values serialize")
        }
        Value::Array(array) => Value::Array(array.iter().map(canonical).collect()),
        _ => value.clone(),
    }
}
#[cfg(unix)]
mod broker;
#[cfg(unix)]
pub use broker::{bridge, lifecycle, serve, Account, Broker, Connection, Directory};

#[cfg(unix)]
pub mod remote;
