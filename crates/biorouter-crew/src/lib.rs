//! Rootless Crew protocol and authoritative, single-writer collaboration state.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

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
pub use broker::{bridge, lifecycle, serve, Broker, Connection};

#[cfg(unix)]
pub mod remote;
