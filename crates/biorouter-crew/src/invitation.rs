//! Joining a workspace (S3a): the `brcrew1:` invitation codec and the device code.
//!
//! See `docs/research/biorouter-crew/naming-design.md`, "The invitation" and "The device code".
//!
//! **The invitation** is the host's verified workspace descriptor (the four pinned fields: the
//! workspace ID and key, the broker socket and the host's UID) plus display labels and SSH
//! hints, as one line `brcrew1:<base64url(JSON)>` inside a human-readable message. Only the
//! daemon parses it. Labels are not authority: `hello` must verify against the pinned workspace
//! key before anything in it is trusted. [`parse`] also accepts the legacy
//! `biorouter-crew status` JSON a host pastes from an older broker.
//!
//! **The device code** is computed from the workspace key the joiner pinned and the joiner's own
//! device key, on the joiner's computer, so nothing the broker or a process in the bridge path
//! returns can change what the joiner's screen shows. The broker recomputes it from the claimed
//! key when checking `auth.join`. No broker response ever carries one.

use std::fmt;

use base64::alphabet;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig, URL_SAFE_NO_PAD};
use base64::engine::DecodePaddingMode;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

use crate::names::{valid_username, validate_display_name, validate_workspace_name};
use crate::{is_canonical_institution_id, Mode};

/// The token prefix, including its version digit.
pub const PREFIX: &str = "brcrew1:";
/// The only invitation version this build understands.
pub const VERSION: u64 = 1;
/// The largest decoded invitation JSON accepted or produced.
pub const MAX_DECODED_BYTES: usize = 4096;
/// The largest pasted text [`parse`] scans.
pub const MAX_PASTED_BYTES: usize = 64 * 1024;
/// The longest Unix socket path Linux can connect to (`sun_path` less its NUL).
const MAX_SOCKET_PATH_BYTES: usize = 107;
/// A DNS name is at most 253 bytes; a jump route is a few of them.
const MAX_HOST_BYTES: usize = 253;
const MAX_PROXY_JUMP_BYTES: usize = 1024;

/// A workspace invitation, as encoded by the host or recovered by [`parse`].
///
/// The first four fields pin the workspace and are always present. The rest are labels and
/// defaults; a legacy status JSON carries none of them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInvitation {
    /// Canonical lowercase hyphenated UUID.
    pub workspace_id: String,
    /// The Ed25519 workspace public key, 64 lowercase hex characters.
    pub workspace_public_key: String,
    /// The broker socket, `/tmp/<runtime dir>/<socket>`.
    pub socket_path: String,
    /// The host account's UID; never 0.
    pub owner_uid: u32,
    pub workspace_name: Option<String>,
    /// Required to [`encode`]; absent only from a legacy status JSON.
    pub host_username: Option<String>,
    pub host_display_name: Option<String>,
    /// The workspace's privacy mode. Required to [`encode`]; absent only from a legacy status
    /// JSON.
    pub mode: Option<Mode>,
    pub institution_id: Option<String>,
    /// The SSH host the host reaches the server by (never a local alias). Optional because the
    /// broker's own `start` output cannot know it.
    pub ssh_host: Option<String>,
    pub ssh_port: Option<u16>,
    pub proxy_jump: Option<String>,
    pub invitee_username: Option<String>,
}

/// Where a parsed invitation came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvitationSource {
    /// A `brcrew1:` line, alone or inside a message.
    Invitation,
    /// The JSON `biorouter-crew status` prints (an older broker, or a host pasting its own).
    LegacyStatus,
}

/// The result of [`parse`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedInvitation {
    pub source: InvitationSource,
    pub invitation: WorkspaceInvitation,
}

/// The workspace key fingerprint, as `hello` reports it in `workspace_key_fingerprint`: SHA-256
/// of the 32 key bytes, lowercase hex. `None` unless `key_hex` is 64 lowercase hex characters
/// encoding a valid Ed25519 public key.
pub fn workspace_key_fingerprint(key_hex: &str) -> Option<String> {
    key_fingerprint(key_hex)
}

/// The short form a person compares by eye: the first 16 hex digits of a fingerprint,
/// upper-cased, in groups of four (`3F2A 9C1E 77B0 D4E1`).
pub fn grouped_fingerprint(fingerprint_hex: &str) -> String {
    let digits: Vec<char> = fingerprint_hex
        .chars()
        .filter(char::is_ascii_hexdigit)
        .take(16)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    digits
        .chunks(4)
        .map(|group| group.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A field of the invitation, named for an error message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvitationField {
    WorkspaceId,
    WorkspacePublicKey,
    WorkspaceKeyFingerprint,
    SocketPath,
    OwnerUid,
    NodeId,
    WorkspaceName,
    HostUsername,
    HostDisplayName,
    Mode,
    InstitutionId,
    SshHost,
    SshPort,
    ProxyJump,
    InviteeUsername,
}

impl InvitationField {
    fn label(self) -> &'static str {
        match self {
            Self::WorkspaceId => "workspace ID",
            Self::WorkspacePublicKey => "workspace key",
            Self::WorkspaceKeyFingerprint => "workspace key fingerprint",
            Self::SocketPath => "server socket path",
            Self::OwnerUid => "host user ID",
            Self::NodeId => "server identity",
            Self::WorkspaceName => "workspace name",
            Self::HostUsername => "host username",
            Self::HostDisplayName => "host name",
            Self::Mode => "privacy mode",
            Self::InstitutionId => "institution",
            Self::SshHost => "server address",
            Self::SshPort => "SSH port",
            Self::ProxyJump => "jump host",
            Self::InviteeUsername => "invited username",
        }
    }
}

/// Why an invitation was refused. [`fmt::Display`] is a plain sentence; [`Self::code`] is the
/// machine code. Neither echoes the pasted text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InvitationError {
    /// No `brcrew1:` line and no status JSON in the text.
    NotFound,
    /// Longer than [`MAX_PASTED_BYTES`], or the decoded JSON over [`MAX_DECODED_BYTES`].
    TooLong,
    /// Not base64url, not JSON, not an object, or a field of the wrong type or unknown name.
    Malformed,
    /// A version this build does not understand.
    UnsupportedVersion,
    /// A field that is present but invalid, or missing when required.
    InvalidField(InvitationField),
}

impl InvitationError {
    /// The machine code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "invitation_not_found",
            Self::TooLong => "invitation_too_long",
            Self::Malformed => "invitation_malformed",
            Self::UnsupportedVersion => "invitation_unsupported_version",
            Self::InvalidField(_) => "invitation_invalid_field",
        }
    }
}

impl fmt::Display for InvitationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str(
                "Paste the whole invitation from your host. It has a line that starts with \
                 brcrew1:.",
            ),
            Self::TooLong => f.write_str("This is too long to be an invitation."),
            Self::Malformed => f.write_str(
                "This invitation is damaged or incomplete. Ask your host to copy it again.",
            ),
            Self::UnsupportedVersion => f.write_str(
                "This invitation needs a newer version of Biorouter. Update Biorouter and paste \
                 it again.",
            ),
            Self::InvalidField(field) => write!(
                f,
                "This invitation has an invalid {}. Ask your host to copy it again.",
                field.label()
            ),
        }
    }
}

impl std::error::Error for InvitationError {}

/// The v1 wire object. Field order is the encoding order.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireV1 {
    v: u64,
    workspace_id: String,
    workspace_public_key: String,
    socket_path: String,
    owner_uid: u32,
    #[serde(default)]
    workspace_name: Option<String>,
    host_username: String,
    #[serde(default)]
    host_display_name: Option<String>,
    mode: Mode,
    #[serde(default)]
    institution_id: Option<String>,
    #[serde(default)]
    ssh_host: Option<String>,
    #[serde(default)]
    ssh_port: Option<u16>,
    #[serde(default)]
    proxy_jump: Option<String>,
    #[serde(default)]
    invitee_username: Option<String>,
}

/// Decodes with or without `=` padding; encodes without.
const URL_SAFE_ANY_PADDING: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Encode an invitation as its one-line `brcrew1:` token.
///
/// Every field is validated exactly as [`parse`] validates it, so an invitation this returns
/// always parses back to itself. `host_username` and `mode` are required.
pub fn encode(invitation: &WorkspaceInvitation) -> Result<String, InvitationError> {
    validate(invitation)?;
    let wire = WireV1 {
        v: VERSION,
        workspace_id: invitation.workspace_id.clone(),
        workspace_public_key: invitation.workspace_public_key.clone(),
        socket_path: invitation.socket_path.clone(),
        owner_uid: invitation.owner_uid,
        workspace_name: invitation.workspace_name.clone(),
        host_username: invitation
            .host_username
            .clone()
            .ok_or(InvitationError::InvalidField(InvitationField::HostUsername))?,
        host_display_name: invitation.host_display_name.clone(),
        mode: invitation
            .mode
            .clone()
            .ok_or(InvitationError::InvalidField(InvitationField::Mode))?,
        institution_id: invitation.institution_id.clone(),
        ssh_host: invitation.ssh_host.clone(),
        ssh_port: invitation.ssh_port,
        proxy_jump: invitation.proxy_jump.clone(),
        invitee_username: invitation.invitee_username.clone(),
    };
    let json = serde_json::to_vec(&wire).expect("invitation JSON serializes");
    if json.len() > MAX_DECODED_BYTES {
        return Err(InvitationError::TooLong);
    }
    Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(json)))
}

/// The human-readable message a host sends: a sentence naming the workspace, one instruction,
/// and the `brcrew1:` line.
pub fn message(invitation: &WorkspaceInvitation) -> Result<String, InvitationError> {
    let line = encode(invitation)?;
    Ok(format!(
        "Join {} on Crew.\nIn Biorouter, open Crew, choose Join a workspace, and paste this \
         whole message.\n{line}",
        workspace_label(invitation)
    ))
}

/// How the workspace is named in a message: its name, else "{host}'s workspace", else
/// "a workspace".
pub fn workspace_label(invitation: &WorkspaceInvitation) -> String {
    if let Some(name) = &invitation.workspace_name {
        return name.clone();
    }
    match invitation
        .host_display_name
        .as_ref()
        .or(invitation.host_username.as_ref())
    {
        Some(host) => format!("{host}'s workspace"),
        None => "a workspace".to_string(),
    }
}

/// Parse pasted text: the first `brcrew1:` token anywhere in it (the whole message, or the bare
/// line), or else the legacy `biorouter-crew status` JSON. Refuses an unknown version, oversize
/// input and any field that `validate_connection` would refuse.
pub fn parse(text: &str) -> Result<ParsedInvitation, InvitationError> {
    if text.len() > MAX_PASTED_BYTES {
        return Err(InvitationError::TooLong);
    }
    if let Some(token) = find_token(text) {
        return parse_token(token).map(|invitation| ParsedInvitation {
            source: InvitationSource::Invitation,
            invitation,
        });
    }
    if let Some(status) = find_legacy_status(text) {
        return parse_legacy_status(&status).map(|invitation| ParsedInvitation {
            source: InvitationSource::LegacyStatus,
            invitation,
        });
    }
    Err(InvitationError::NotFound)
}

/// Validate every field. [`encode`] and [`parse`] both call this.
pub fn validate(invitation: &WorkspaceInvitation) -> Result<(), InvitationError> {
    use InvitationField as F;
    check(canonical_uuid(&invitation.workspace_id), F::WorkspaceId)?;
    check(
        key_fingerprint(&invitation.workspace_public_key).is_some(),
        F::WorkspacePublicKey,
    )?;
    check(valid_socket_path(&invitation.socket_path), F::SocketPath)?;
    check(invitation.owner_uid != 0, F::OwnerUid)?;
    check_optional(&invitation.workspace_name, F::WorkspaceName, |name| {
        validate_workspace_name(name).is_ok()
    })?;
    check_optional(&invitation.host_username, F::HostUsername, |name| {
        valid_username(name)
    })?;
    check_optional(&invitation.host_display_name, F::HostDisplayName, |name| {
        validate_display_name(name).is_ok_and(|clean| clean == *name)
    })?;
    check_optional(&invitation.institution_id, F::InstitutionId, |id| {
        is_canonical_institution_id(id)
    })?;
    check_optional(&invitation.ssh_host, F::SshHost, |host| {
        valid_ssh_host(host)
    })?;
    check(invitation.ssh_port != Some(0), F::SshPort)?;
    check_optional(&invitation.proxy_jump, F::ProxyJump, |jump| {
        jump.len() <= MAX_PROXY_JUMP_BYTES && jump.split(',').all(safe_atom)
    })?;
    check_optional(&invitation.invitee_username, F::InviteeUsername, |name| {
        valid_username(name)
    })?;
    Ok(())
}

fn check(ok: bool, field: InvitationField) -> Result<(), InvitationError> {
    if ok {
        Ok(())
    } else {
        Err(InvitationError::InvalidField(field))
    }
}

fn check_optional(
    value: &Option<String>,
    field: InvitationField,
    valid: impl Fn(&str) -> bool,
) -> Result<(), InvitationError> {
    check(value.as_deref().is_none_or(valid), field)
}

/// The base64url run after the first `brcrew1:` in `text`.
fn find_token(text: &str) -> Option<&str> {
    let (_, after) = text.split_once(PREFIX)?;
    let end = after
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '=')))
        .unwrap_or(after.len());
    after.get(..end)
}

fn parse_token(token: &str) -> Result<WorkspaceInvitation, InvitationError> {
    if token.len() > MAX_DECODED_BYTES.div_ceil(3) * 4 {
        return Err(InvitationError::TooLong);
    }
    let bytes = URL_SAFE_ANY_PADDING
        .decode(token)
        .map_err(|_| InvitationError::Malformed)?;
    if bytes.len() > MAX_DECODED_BYTES {
        return Err(InvitationError::TooLong);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| InvitationError::Malformed)?;
    match value.get("v") {
        Some(v) if v.as_u64() == Some(VERSION) => {}
        Some(v) if v.is_u64() => return Err(InvitationError::UnsupportedVersion),
        _ => return Err(InvitationError::Malformed),
    }
    let wire: WireV1 = serde_json::from_value(value).map_err(|_| InvitationError::Malformed)?;
    let invitation = WorkspaceInvitation {
        workspace_id: wire.workspace_id,
        workspace_public_key: wire.workspace_public_key,
        socket_path: wire.socket_path,
        owner_uid: wire.owner_uid,
        workspace_name: wire.workspace_name,
        host_username: Some(wire.host_username),
        host_display_name: wire.host_display_name,
        mode: Some(wire.mode),
        institution_id: wire.institution_id,
        ssh_host: wire.ssh_host,
        ssh_port: wire.ssh_port,
        proxy_jump: wire.proxy_jump,
        invitee_username: wire.invitee_username,
    };
    validate(&invitation)?;
    Ok(invitation)
}

/// The status JSON as a whole, or the first line that is a JSON object naming a workspace and
/// a socket (the line `biorouter-crew status` prints, pasted with a prompt around it).
fn find_legacy_status(text: &str) -> Option<serde_json::Map<String, Value>> {
    std::iter::once(text.trim())
        .chain(
            text.lines()
                .map(str::trim)
                .filter(|line| line.starts_with('{')),
        )
        .find_map(|candidate| match serde_json::from_str(candidate) {
            Ok(Value::Object(map))
                if map.contains_key("workspace_id") && map.contains_key("socket") =>
            {
                Some(map)
            }
            _ => None,
        })
}

fn parse_legacy_status(
    status: &serde_json::Map<String, Value>,
) -> Result<WorkspaceInvitation, InvitationError> {
    use InvitationField as F;
    let text = |key: &str, field| {
        status
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(InvitationError::InvalidField(field))
    };
    if let Some(protocol) = status.get("protocol") {
        match protocol.as_u64() {
            Some(1) => {}
            Some(_) => return Err(InvitationError::UnsupportedVersion),
            None => return Err(InvitationError::Malformed),
        }
    }
    let owner_uid = status
        .get("host_uid")
        .and_then(Value::as_u64)
        .and_then(|uid| u32::try_from(uid).ok())
        .ok_or(InvitationError::InvalidField(F::OwnerUid))?;
    let invitation = WorkspaceInvitation {
        workspace_id: text("workspace_id", F::WorkspaceId)?,
        workspace_public_key: text("workspace_public_key", F::WorkspacePublicKey)?,
        socket_path: text("socket", F::SocketPath)?,
        owner_uid,
        workspace_name: None,
        host_username: None,
        host_display_name: None,
        mode: None,
        institution_id: None,
        ssh_host: None,
        ssh_port: None,
        proxy_jump: None,
        invitee_username: None,
    };
    validate(&invitation)?;
    if let Some(fingerprint) = status.get("workspace_key_fingerprint") {
        check(
            fingerprint.as_str() == key_fingerprint(&invitation.workspace_public_key).as_deref(),
            F::WorkspaceKeyFingerprint,
        )?;
    }
    if let Some(node_id) = status.get("node_id") {
        check(node_id.as_str().is_some_and(is_lower_hex_64), F::NodeId)?;
    }
    Ok(invitation)
}

fn canonical_uuid(value: &str) -> bool {
    uuid::Uuid::try_parse(value).is_ok_and(|id| id.hyphenated().to_string() == value)
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// SHA-256 of the key bytes as lowercase hex, if `key_hex` is 64 lowercase hex characters
/// encoding a valid Ed25519 public key.
fn key_fingerprint(key_hex: &str) -> Option<String> {
    if !is_lower_hex_64(key_hex) {
        return None;
    }
    let bytes: [u8; 32] = hex::decode(key_hex).ok()?.try_into().ok()?;
    ed25519_dalek::VerifyingKey::from_bytes(&bytes).ok()?;
    Some(hex::encode(Sha256::digest(bytes)))
}

/// The characters the daemon allows in anything it puts on an SSH command line
/// (`safe_atom` in the core).
fn safe_atom(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:@%-".contains(&b))
        && !value.starts_with('-')
}

/// An absolute path the bridge can accept: `/tmp/<dir>/<socket>` or `/private/tmp/<dir>/<socket>`,
/// with no empty, `.` or `..` component, in `safe_atom` characters, short enough to connect.
fn valid_socket_path(path: &str) -> bool {
    if !safe_atom(path) || path.len() > MAX_SOCKET_PATH_BYTES {
        return false;
    }
    let Some(rest) = path
        .strip_prefix("/tmp/")
        .or_else(|| path.strip_prefix("/private/tmp/"))
    else {
        return false;
    };
    let components: Vec<&str> = rest.split('/').collect();
    components.len() == 2
        && components
            .iter()
            .all(|part| !part.is_empty() && *part != "." && *part != "..")
}

/// A host name or address to put after `user@`: `safe_atom` characters, no `@` or `/`.
fn valid_ssh_host(host: &str) -> bool {
    host.len() <= MAX_HOST_BYTES && safe_atom(host) && !host.contains(['@', '/'])
}

/// Characters in a device code: exactly 16 of these.
pub const DEVICE_CODE_LEN: usize = 16;
/// Crockford base32: no I, L, O or U.
const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const DEVICE_CODE_DOMAIN: &[u8] = b"biorouter-crew-device-code-v1\0";

/// Why a device code or its inputs were refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceCodeError {
    /// Not 16 characters once spaces and hyphens are removed.
    WrongLength,
    /// Contains `U`, which no device code contains.
    ContainsU,
    /// A character that is not a letter or digit of the code alphabet.
    InvalidCharacter,
    /// A key that is not 32 bytes of hex.
    InvalidKey,
}

impl DeviceCodeError {
    /// The machine code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidKey => "invalid_key",
            _ => "device_code_invalid",
        }
    }
}

impl fmt::Display for DeviceCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WrongLength => "A device code has 16 letters and numbers.",
            Self::ContainsU => "Device codes never contain the letter U. Check the code.",
            Self::InvalidCharacter => "A device code has only letters and numbers.",
            Self::InvalidKey => "The key is not a 32-byte hex key.",
        })
    }
}

impl std::error::Error for DeviceCodeError {}

/// `Crockford-base32(SHA-256("biorouter-crew-device-code-v1\0" ‖ workspace_id ‖ "\0" ‖ W ‖ K)[0..10])`:
/// 16 characters, unformatted (`7QK2M9XA3JTPWZ4D`). `W` is the 32-byte workspace public key the
/// joiner pinned and `K` the joiner's 32-byte device public key.
pub fn device_code(
    workspace_id: &str,
    workspace_public_key: &[u8; 32],
    device_public_key: &[u8; 32],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(DEVICE_CODE_DOMAIN);
    hasher.update(workspace_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(workspace_public_key);
    hasher.update(device_public_key);
    let digest = hasher.finalize();
    let bits = digest
        .iter()
        .take(10)
        .fold(0u128, |acc, byte| (acc << 8) | u128::from(*byte));
    (0..DEVICE_CODE_LEN)
        .map(|i| {
            let index = (bits >> (75 - 5 * i)) & 0x1f;
            char::from(CROCKFORD[index as usize])
        })
        .collect()
}

/// [`device_code`] over hex-encoded keys, as the daemon and broker store them.
pub fn device_code_from_hex(
    workspace_id: &str,
    workspace_public_key_hex: &str,
    device_public_key_hex: &str,
) -> Result<String, DeviceCodeError> {
    let key = |hex_key: &str| -> Result<[u8; 32], DeviceCodeError> {
        hex::decode(hex_key)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(DeviceCodeError::InvalidKey)
    };
    Ok(device_code(
        workspace_id,
        &key(workspace_public_key_hex)?,
        &key(device_public_key_hex)?,
    ))
}

/// Normalize a typed or pasted code to its 16 canonical characters: spaces, hyphens and
/// invisible characters removed, upper-cased, `I` and `L` read as `1`, `O` as `0`. `U` is
/// refused, as is anything outside the alphabet or a length other than 16.
pub fn normalize_device_code(input: &str) -> Result<String, DeviceCodeError> {
    let mut code = String::with_capacity(DEVICE_CODE_LEN);
    for c in input.chars() {
        if c.is_whitespace()
            || c.general_category() == GeneralCategory::DashPunctuation
            || crate::names::is_default_ignorable(c)
        {
            continue;
        }
        let upper = c.to_ascii_uppercase();
        let mapped = match upper {
            'I' | 'L' => '1',
            'O' => '0',
            'U' => return Err(DeviceCodeError::ContainsU),
            other if other.is_ascii() && CROCKFORD.contains(&(other as u8)) => other,
            _ => return Err(DeviceCodeError::InvalidCharacter),
        };
        if code.len() == DEVICE_CODE_LEN {
            return Err(DeviceCodeError::WrongLength);
        }
        code.push(mapped);
    }
    if code.len() == DEVICE_CODE_LEN {
        Ok(code)
    } else {
        Err(DeviceCodeError::WrongLength)
    }
}

/// Group a code for display, `7QK2-M9XA-3JTP-WZ4D`. The input is normalized first; a code that
/// does not normalize is returned unchanged.
pub fn format_device_code(code: &str) -> String {
    let Ok(code) = normalize_device_code(code) else {
        return code.to_string();
    };
    let chars: Vec<char> = code.chars().collect();
    chars
        .chunks(4)
        .map(|group| group.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Whether `approved` (as a person typed it, or already normalized) is the device code for
/// these inputs. The comparison takes the same time wherever the codes first differ, so a
/// caller that tries many keys learns nothing from timing about how close one came.
pub fn device_code_matches(
    approved: &str,
    workspace_id: &str,
    workspace_public_key: &[u8; 32],
    device_public_key: &[u8; 32],
) -> bool {
    let Ok(approved) = normalize_device_code(approved) else {
        return false;
    };
    let expected = device_code(workspace_id, workspace_public_key, device_public_key);
    approved.len() == expected.len()
        && approved
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}
