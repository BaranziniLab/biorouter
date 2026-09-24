use crate::*;
use anyhow::{anyhow, bail, ensure, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

fn projection_status(p: &Value) -> Result<Option<String>> {
    let status = p.get("status").and_then(Value::as_str).map(str::to_owned);
    if let Some(status) = &status {
        ensure!(
            matches!(
                status.as_str(),
                "progress" | "completed" | "failed" | "cancelled"
            ),
            "invalid_params: run status"
        );
    }
    Ok(status)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn id() -> String {
    Uuid::new_v4().to_string()
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
#[cfg(target_os = "linux")]
fn node_identity() -> Result<String> {
    for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .context("node_identity_unavailable: cannot read trusted machine identity")
            }
        };
        let metadata = file.metadata()?;
        ensure!(metadata.is_file()&&metadata.uid()==0&&metadata.mode()&0o022==0&&metadata.len()<=128,"node_identity_unavailable: machine identity must be a root-owned non-writable regular file");
        let mut value = String::new();
        file.read_to_string(&mut value)?;
        let value = value.trim();
        ensure!(
            value.len() == 32
                && value.bytes().all(|b| b.is_ascii_hexdigit())
                && value.bytes().any(|b| b != b'0'),
            "node_identity_unavailable: invalid machine identity"
        );
        let mut identity = b"biorouter-crew-node-v1\0".to_vec();
        identity.extend_from_slice(value.to_ascii_lowercase().as_bytes());
        return Ok(digest(&identity));
    }
    bail!("node_identity_unavailable: Linux machine-id is required; ask the host operator to provide its standard node identity")
}
#[cfg(not(target_os = "linux"))]
fn node_identity() -> Result<String> {
    bail!("unsupported: stable Crew node identity requires Linux")
}
fn text<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= MAX_FRAME)
        .ok_or_else(|| anyhow!("invalid_params: missing or invalid {key}"))
}
fn number(p: &Value, key: &str) -> Result<u64> {
    p.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("invalid_params: missing {key}"))
}
fn strings(p: &Value, key: &str) -> Result<BTreeSet<String>> {
    match p.get(key) {
        None => Ok(BTreeSet::new()),
        Some(v) => serde_json::from_value(v.clone()).map_err(Into::into),
    }
}
fn mode(p: &Value, key: &str) -> Result<Mode> {
    serde_json::from_value(
        p.get(key)
            .cloned()
            .ok_or_else(|| anyhow!("invalid_params: missing {key}"))?,
    )
    .map_err(Into::into)
}
/// One Unix account as the node's account database reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub uid: u32,
    /// The account name (`pw_name`).
    pub name: String,
    /// The first field of the account's GECOS entry, trimmed; `None` when empty or not UTF-8.
    /// A label only: never validated here and never used for authority.
    pub full_name: Option<String>,
    /// The account's login shell (`pw_shell`); `None` when empty or not UTF-8. Read only to
    /// refuse system accounts (a `nologin` or `false` shell) at invitation time.
    pub shell: Option<String>,
}

/// `UID_MIN` when `/etc/login.defs` does not say, as shadow-utils and every major distribution
/// default it.
const DEFAULT_UID_MIN: u32 = 1000;
/// `UID_MIN` from the text of `/etc/login.defs`: the last uncommented `UID_MIN <n>` line, else
/// `None`.
fn parse_uid_min(login_defs: &str) -> Option<u32> {
    login_defs.lines().rev().find_map(|line| {
        let line = line.split('#').next().unwrap_or_default();
        let mut fields = line.split_whitespace();
        (fields.next() == Some("UID_MIN"))
            .then(|| fields.next())
            .flatten()
            .and_then(|value| value.parse().ok())
    })
}

/// The node's account database, as the broker reads it: point lookups only, never an
/// enumeration. The shipped broker uses `getpwuid_r`/`getpwnam_r`; tests inject a fake through
/// [`Broker::open_with_directory`] (feature `test-seams`).
///
/// Every authenticated request checks `by_uid(uid).name` against the principal's username, so
/// an account renamed or recycled under an enrolled UID stops authenticating.
pub trait Directory {
    /// The account holding `uid`.
    fn by_uid(&self, uid: u32) -> Result<Account>;
    /// The account named exactly `name`, as NSS resolves it (which may be an alias whose
    /// canonical name differs; callers that care compare `by_uid(account.uid)`).
    fn by_name(&self, name: &str) -> Result<Account>;
    /// The lowest UID of an ordinary login account. The shipped directory reads `UID_MIN` from
    /// `/etc/login.defs`, and 1000 when that file does not say; anything else answers 1000
    /// unless it overrides this.
    fn uid_min(&self) -> u32 {
        DEFAULT_UID_MIN
    }
}

/// The node's NSS account database.
struct SystemDirectory;

/// `getpwuid_r`/`getpwnam_r` buffers start here and grow on `ERANGE` up to the cap.
const PASSWD_BUFFER_START: usize = 16 * 1024;
const PASSWD_BUFFER_CAP: usize = 1024 * 1024;

impl SystemDirectory {
    fn lookup(
        mut call: impl FnMut(
            *mut libc::passwd,
            *mut libc::c_char,
            libc::size_t,
            *mut *mut libc::passwd,
        ) -> libc::c_int,
    ) -> Result<Account> {
        let mut size = PASSWD_BUFFER_START;
        loop {
            let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
            let mut found = std::ptr::null_mut();
            let mut buffer = vec![0u8; size];
            let status = call(
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            );
            if status == libc::ERANGE && size < PASSWD_BUFFER_CAP {
                size *= 4;
                continue;
            }
            ensure!(
                status == 0 && !found.is_null(),
                "identity_unavailable: Unix account cannot be resolved"
            );
            // SAFETY: on success `found` points at `entry`, whose strings live in `buffer`,
            // both alive for the rest of this iteration.
            let entry = unsafe { &*found };
            let name = unsafe { std::ffi::CStr::from_ptr(entry.pw_name) }
                .to_str()
                .map_err(|_| anyhow!("identity_unavailable: Unix account name is not UTF-8"))?
                .to_owned();
            let full_name = if entry.pw_gecos.is_null() {
                None
            } else {
                unsafe { std::ffi::CStr::from_ptr(entry.pw_gecos) }
                    .to_str()
                    .ok()
                    .and_then(|gecos| gecos.split(',').next())
                    .map(str::trim)
                    .filter(|first| !first.is_empty())
                    .map(str::to_owned)
            };
            let shell = if entry.pw_shell.is_null() {
                None
            } else {
                unsafe { std::ffi::CStr::from_ptr(entry.pw_shell) }
                    .to_str()
                    .ok()
                    .map(str::trim)
                    .filter(|shell| !shell.is_empty())
                    .map(str::to_owned)
            };
            return Ok(Account {
                uid: entry.pw_uid,
                name,
                full_name,
                shell,
            });
        }
    }
}

impl Directory for SystemDirectory {
    fn by_uid(&self, uid: u32) -> Result<Account> {
        Self::lookup(|entry, buffer, length, found| unsafe {
            libc::getpwuid_r(uid, entry, buffer, length, found)
        })
    }
    fn by_name(&self, name: &str) -> Result<Account> {
        let name = std::ffi::CString::new(name)
            .map_err(|_| anyhow!("identity_unavailable: Unix account cannot be resolved"))?;
        Self::lookup(|entry, buffer, length, found| unsafe {
            libc::getpwnam_r(name.as_ptr(), entry, buffer, length, found)
        })
    }
    fn uid_min(&self) -> u32 {
        fs::read_to_string("/etc/login.defs")
            .ok()
            .and_then(|text| parse_uid_min(&text))
            .unwrap_or(DEFAULT_UID_MIN)
    }
}
fn private_dir(path: &Path) -> Result<()> {
    if !path.exists() {
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.is_dir() && m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0,
        "unsafe_storage: state directory must be owner-only and not a symlink"
    );
    Ok(())
}
fn private_file(path: &Path, append: bool) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .append(append)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let m = file.metadata()?;
    ensure!(
        m.is_file()
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0
            && m.nlink() == 1,
        "unsafe_storage: file ownership, mode or links invalid"
    );
    Ok(file)
}
fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all().map_err(Into::into)
}

#[derive(Clone, Serialize, Deserialize)]
struct Device {
    principal_id: String,
    public_key: String,
    /// When the key was bound (seconds since the Unix epoch). Absent on devices bound before
    /// this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    added_at: Option<u64>,
    /// How the key was bound: `bootstrap`, `token` (legacy enrollment) or `invitation_code`
    /// (S3a). Absent on devices bound before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    added_via: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Enrollment {
    #[serde(default)]
    existing_principal_id: Option<String>,
    uid: u32,
    public_key: String,
    expires_at: u64,
}
#[derive(Clone, Serialize, Deserialize)]
struct Cached {
    digest: String,
    result: Value,
}
struct RunPolicyConsent {
    public_provider: bool,
    personal_mode: Mode,
    expected_protected_context: bool,
    workspace_institution_id: Option<String>,
    connection_institution_id: Option<String>,
    provider_affiliation: ProviderAffiliation,
}

#[derive(Clone, Serialize, Deserialize)]
struct State {
    workspace: Workspace,
    bootstrap_key: String,
    workspace_signing_key: String,
    #[serde(default)]
    writer_node_id: Option<String>,
    #[serde(default)]
    runtime_basename: Option<String>,
    principals: BTreeMap<String, Principal>,
    devices: BTreeMap<String, Device>,
    enrollments: BTreeMap<String, Enrollment>,
    teams: BTreeMap<String, Team>,
    channels: BTreeMap<String, Channel>,
    invitations: BTreeMap<String, Invitation>,
    messages: Vec<Message>,
    runs: BTreeMap<String, Run>,
    grants: BTreeMap<String, String>,
    blobs: BTreeMap<String, Blob>,
    #[serde(default)]
    references: BTreeMap<String, RemoteReference>,
    dedupe: BTreeMap<String, Cached>,
    sequence: u64,
    #[serde(default)]
    read_positions: BTreeMap<String, u64>,
    /// Host-issued workspace joins (S3a), keyed by the decimal UID. Serialized only when
    /// non-empty, so a journal that never held one replays and re-serializes byte for byte, no
    /// upgrade record is written on open, and an older broker (which ignores unknown fields)
    /// replays a journal that did. Kept by every build, with or without `join-by-name`, so a
    /// broker built without the feature never drops a pending join from state.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pending_joins: BTreeMap<String, PendingJoin>,
}
#[derive(Serialize, Deserialize)]
struct Record {
    version: u32,
    sequence: u64,
    previous: String,
    checksum: String,
    actor: String,
    operation: String,
    timestamp: u64,
    state: State,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Patch {
    Set {
        path: Vec<String>,
        value: Value,
    },
    Remove {
        path: Vec<String>,
    },
    Append {
        path: Vec<String>,
        values: Vec<Value>,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeltaRecord {
    version: u32,
    sequence: u64,
    previous: String,
    checksum: String,
    actor: String,
    operation: String,
    timestamp: u64,
    patches: Vec<Patch>,
}
fn delta(before: &Value, after: &Value, path: &mut Vec<String>, patches: &mut Vec<Patch>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(old), Value::Object(new)) => {
            for key in old.keys().filter(|key| !new.contains_key(*key)) {
                path.push(key.clone());
                patches.push(Patch::Remove { path: path.clone() });
                path.pop();
            }
            for (key, value) in new {
                path.push(key.clone());
                if let Some(old) = old.get(key) {
                    delta(old, value, path, patches);
                } else {
                    patches.push(Patch::Set {
                        path: path.clone(),
                        value: value.clone(),
                    });
                }
                path.pop();
            }
        }
        (Value::Array(old), Value::Array(new)) if new.starts_with(old) => {
            patches.push(Patch::Append {
                path: path.clone(),
                values: new[old.len()..].to_vec(),
            })
        }
        _ => patches.push(Patch::Set {
            path: path.clone(),
            value: after.clone(),
        }),
    }
}
fn apply_patch(state: &mut Value, patch: Patch) -> Result<()> {
    let path = match &patch {
        Patch::Set { path, .. } | Patch::Remove { path } | Patch::Append { path, .. } => path,
    };
    ensure!(
        path.len() <= 32,
        "journal_corrupt: patch nesting exceeds limit"
    );
    if path.is_empty() {
        if let Patch::Set { value, .. } = patch {
            *state = value;
            return Ok(());
        }
        bail!("journal_corrupt: invalid root patch");
    }
    let mut parent = state;
    for segment in &path[..path.len() - 1] {
        parent = parent
            .get_mut(segment)
            .ok_or_else(|| anyhow!("journal_corrupt: missing patch ancestor"))?;
    }
    let key = path.last().expect("nonempty patch").clone();
    let object = parent
        .as_object_mut()
        .ok_or_else(|| anyhow!("journal_corrupt: patch parent is not an object"))?;
    match patch {
        Patch::Set { value, .. } => {
            object.insert(key, value);
        }
        Patch::Remove { .. } => {
            ensure!(
                object.remove(&key).is_some(),
                "journal_corrupt: removed key missing"
            );
        }
        Patch::Append { values, .. } => object
            .get_mut(&key)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| anyhow!("journal_corrupt: append target is not an array"))?
            .extend(values),
    }
    Ok(())
}
#[derive(Default)]
pub struct Connection {
    challenges: BTreeMap<String, (String, u64)>,
}
impl Connection {
    pub fn new() -> Self {
        Self::default()
    }
}
pub struct Broker {
    state: State,
    root: PathBuf,
    journal: File,
    _lock: File,
    checksum: String,
    poisoned: bool,
    /// The account database every UID-to-name check goes through.
    directory: Box<dyn Directory + Send>,
    /// Where sibling runtime directories (`crew-<uid>-<hex>/broker.sock`) live: `/tmp`.
    runtime_root: PathBuf,
    /// Per-actor times of recent name-collision refusals (in memory only), for the rate limit
    /// that bounds the name-existence oracle (D5).
    name_refusals: BTreeMap<String, VecDeque<u64>>,
    #[cfg(feature = "join-by-name")]
    join_runtime: join::Runtime,
}

/// Collision refusals allowed per actor within [`NAME_REFUSAL_WINDOW_SECS`] before every
/// name-bearing create or rename is answered with the generic [`NAME_RATE_LIMITED`].
const NAME_REFUSAL_LIMIT: usize = 10;
const NAME_REFUSAL_WINDOW_SECS: u64 = 600;
/// The one refusal for a name that collides with another team, whether or not the caller can
/// see it. It carries no ID, creator, member count or colliding spelling (D5).
const TEAM_NAME_TAKEN: &str = "name_taken: A team with this name, or one that looks like it, already exists in this workspace. Choose a different name.";
/// As [`TEAM_NAME_TAKEN`], for channels in one team (archived channels included).
const CHANNEL_NAME_TAKEN: &str = "name_taken: A channel with this name, or one that looks like it, already exists in this team. Choose a different name.";
/// A workspace name a running sibling workspace of the same host account already uses.
const WORKSPACE_NAME_TAKEN: &str = "name_taken: Another workspace you host on this server is already using this name. Choose a different name.";
const NAME_RATE_LIMITED: &str = "rate_limited: Too many name attempts. Try again later.";
const GENERAL_RESERVED: &str =
    "name_invalid: Channel name general is reserved for the team's first channel.";
const TARGET_MISMATCH: &str =
    "target_mismatch: The person you chose no longer has that username. Refresh and choose again.";
/// A direct add (`team.add_member`, `channel.add_member`) through an agent's grant. The worker
/// allowlist refuses it first; this is the second, method-level wall.
const DIRECT_ADD_AGENT_REFUSED: &str = "forbidden: only a person can add people";
/// A listed channel that is not in the team, or not visible to the caller.
const DIRECT_ADD_UNKNOWN_CHANNEL: &str =
    "invalid_params: One of the chosen channels isn't in this team. Refresh and choose again.";
/// Methods whose collision refusals count toward, and are blocked by, the rate limit.
const NAME_METHODS: [&str; 5] = [
    "team.create",
    "team.rename",
    "channel.create",
    "channel.rename",
    "workspace.rename",
];
/// At most this many sibling runtime directories are probed for a workspace name.
const SIBLING_PROBE_LIMIT: usize = 32;
/// How long one sibling broker may take to answer `hello` during the probe.
const SIBLING_PROBE_TIMEOUT: Duration = Duration::from_secs(1);

#[cfg(feature = "join-by-name")]
mod join;
#[derive(Clone)]
struct Actor {
    id: String,
    run: Option<Run>,
}
enum Admission {
    Actor(Box<Actor>),
    Replay(Value),
}
struct JournalReplay {
    state_value: Value,
    checksum: String,
    sequence: u64,
    committed: usize,
    torn_tail: Option<Vec<u8>>,
}
fn replay_record(
    line: &[u8],
    state_value: &mut Value,
    checksum: &mut String,
    sequence: &mut u64,
) -> Result<()> {
    let envelope: Value =
        serde_json::from_slice(line).context("journal_corrupt: complete record is invalid")?;
    match envelope.get("version").and_then(Value::as_u64) {
        Some(1) => {
            let record: Record =
                serde_json::from_slice(line).context("journal_corrupt: legacy record invalid")?;
            ensure!(
                record.sequence == *sequence + 1 && record.previous == *checksum,
                "journal_corrupt: sequence/hash chain invalid"
            );
            let text = std::str::from_utf8(line)?.trim_end();
            let start = text
                .find(",\"state\":")
                .ok_or_else(|| anyhow!("journal_corrupt: legacy state missing"))?
                + 9;
            let mut payload = serde_json::to_vec(&(
                &record.version,
                &record.sequence,
                &record.previous,
                &record.actor,
                &record.operation,
                &record.timestamp,
            ))?;
            payload.pop();
            payload.push(b',');
            payload.extend_from_slice(&text.as_bytes()[start..text.len() - 1]);
            payload.push(b']');
            let actual = digest(&payload);
            ensure!(
                actual == record.checksum,
                "journal_corrupt: checksum mismatch"
            );
            *checksum = actual;
            *sequence = record.sequence;
            *state_value = envelope
                .get("state")
                .cloned()
                .ok_or_else(|| anyhow!("journal_corrupt: state missing"))?;
        }
        Some(2) => {
            let record: DeltaRecord =
                serde_json::from_slice(line).context("journal_corrupt: delta record invalid")?;
            ensure!(
                record.sequence == *sequence + 1 && record.previous == *checksum,
                "journal_corrupt: sequence/hash chain invalid"
            );
            let actual = digest(&serde_json::to_vec(&(
                &record.version,
                &record.sequence,
                &record.previous,
                &record.actor,
                &record.operation,
                &record.timestamp,
                &record.patches,
            ))?);
            ensure!(
                actual == record.checksum,
                "journal_corrupt: checksum mismatch"
            );
            for patch in record.patches {
                apply_patch(state_value, patch)?;
            }
            *checksum = actual;
            *sequence = record.sequence;
        }
        _ => bail!("journal_corrupt: unsupported journal version"),
    }
    Ok(())
}
fn replay_journal(journal: &File) -> Result<JournalReplay> {
    let mut replay = BufReader::new(journal.try_clone()?);
    let mut torn_tail = None;
    let mut state_value = Value::Null;
    let mut checksum = String::new();
    let mut sequence = 0;
    let mut committed = 0;
    loop {
        let mut line = Vec::new();
        let count = std::io::Read::by_ref(&mut replay)
            .take(16 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        ensure!(
            count <= 16 * 1024 * 1024,
            "journal_corrupt: record exceeds 16 MiB replay bound"
        );
        if !line.ends_with(b"\n") {
            torn_tail = Some(line);
            break;
        }
        replay_record(&line, &mut state_value, &mut checksum, &mut sequence)?;
        ensure!(
            state_value.get("sequence").and_then(Value::as_u64) == Some(sequence),
            "journal_corrupt: state sequence mismatch"
        );
        if let Some(object) = state_value.as_object_mut() {
            object.entry("references").or_insert_with(|| json!({}));
            object.entry("read_positions").or_insert_with(|| json!({}));
        }
        committed += line.len();
    }
    Ok(JournalReplay {
        state_value,
        checksum,
        sequence,
        committed,
        torn_tail,
    })
}
fn recovered_state(
    state_value: Value,
    sequence: u64,
    bootstrap_key: &str,
    initial_name: Option<&str>,
) -> Result<State> {
    let recovered = if sequence > 0 {
        Some(
            serde_json::from_value::<State>(state_value)
                .context("journal_corrupt: invalid recovered state")?,
        )
    } else {
        None
    };
    let state = match recovered {
        Some(s) => {
            ensure!(
                s.workspace.host_uid == unsafe { libc::geteuid() },
                "host_identity_changed"
            );
            #[cfg(target_os = "linux")]
            if let Some(expected) = &s.writer_node_id {
                ensure!(
                    expected == &node_identity()?,
                    "node_identity_changed: workspace belongs to another writer node; automatic failover is disabled"
                );
            }
            s
        }
        None => {
            let key: [u8; 32] = hex::decode(bootstrap_key)?
                .try_into()
                .map_err(|_| anyhow!("bootstrap_key must be a 32-byte Ed25519 public key"))?;
            VerifyingKey::from_bytes(&key)?;
            State {
                workspace: Workspace {
                    id: id(),
                    host_uid: unsafe { libc::geteuid() },
                    mode: Mode::Private,
                    institution_id: None,
                    policy_epoch: 1,
                    name: initial_name.map(str::to_owned),
                },
                bootstrap_key: bootstrap_key.into(),
                workspace_signing_key: digest(token().as_bytes()),
                writer_node_id: None,
                runtime_basename: None,
                principals: BTreeMap::new(),
                devices: BTreeMap::new(),
                enrollments: BTreeMap::new(),
                teams: BTreeMap::new(),
                channels: BTreeMap::new(),
                invitations: BTreeMap::new(),
                messages: vec![],
                runs: BTreeMap::new(),
                grants: BTreeMap::new(),
                blobs: BTreeMap::new(),
                references: BTreeMap::new(),
                dedupe: BTreeMap::new(),
                sequence: 0,
                read_positions: BTreeMap::new(),
                pending_joins: BTreeMap::new(),
            }
        }
    };
    Ok(state)
}
impl Broker {
    pub fn open(root: &Path, bootstrap_key: &str) -> Result<Self> {
        Self::open_inner(root, bootstrap_key, Box::new(SystemDirectory), None)
    }
    /// [`Broker::open`] with an injected account directory. Test builds only: the shipped
    /// binary never enables `test-seams`, so it always reads the node's NSS database.
    #[cfg(feature = "test-seams")]
    pub fn open_with_directory(
        root: &Path,
        bootstrap_key: &str,
        directory: Box<dyn Directory + Send>,
    ) -> Result<Self> {
        Self::open_inner(root, bootstrap_key, directory, None)
    }
    /// Point the sibling-workspace probe of `workspace.rename` at another directory than
    /// `/tmp`. Test builds only.
    #[cfg(feature = "test-seams")]
    pub fn set_runtime_root(&mut self, runtime_root: &Path) {
        self.runtime_root = runtime_root.to_path_buf();
    }
    /// Open (or initialize) a workspace. `initial_name` names a workspace this call creates; an
    /// existing workspace keeps its stored name.
    fn open_inner(
        root: &Path,
        bootstrap_key: &str,
        directory: Box<dyn Directory + Send>,
        initial_name: Option<&str>,
    ) -> Result<Self> {
        if let Some(name) = initial_name {
            validate_workspace_name(name).map_err(|error| anyhow!(error.wire()))?;
        }
        private_dir(root)?;
        let canonical_root = fs::canonicalize(root)?;
        for ancestor in canonical_root.ancestors().skip(1) {
            let metadata = fs::symlink_metadata(ancestor)?;
            ensure!(
                metadata.is_dir()
                    && (metadata.mode() & 0o022 == 0 || metadata.mode() & 0o1000 != 0),
                "unsafe_storage: writable non-sticky ancestor"
            );
        }
        let root = canonical_root.as_path();
        #[cfg(target_os = "linux")]
        {
            let path = std::ffi::CString::new(root.as_os_str().as_encoded_bytes())?;
            let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
            ensure!(
                unsafe { libc::statfs(path.as_ptr(), info.as_mut_ptr()) } == 0,
                "storage_probe_failed"
            );
            let kind = unsafe { info.assume_init() }.f_type;
            ensure!(kind != 0x6969 && kind != 0xff534d42_u64 as libc::c_long && kind != 0x517B, "unsupported_storage: network filesystem requires qualified single-writer fencing; use local persistent HOME storage");
        }
        let lock = private_file(&root.join("writer.lock"), false)?;
        ensure!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "writer_active: another broker holds this workspace"
        );
        let journal = private_file(&root.join("journal.jsonl"), true)?;
        ensure!(
            journal.metadata()?.len() <= 1024 * 1024 * 1024,
            "quota_exceeded: journal exceeds supported replay size of 1 GiB"
        );
        let JournalReplay {
            state_value,
            checksum,
            sequence,
            committed,
            torn_tail,
        } = replay_journal(&journal)?;
        let state = recovered_state(state_value, sequence, bootstrap_key, initial_name)?;
        if let Some(torn_bytes) = torn_tail {
            let tail = root.join(format!("torn-tail-{}", id()));
            let mut file = private_file(&tail, false)?;
            file.write_all(&torn_bytes)?;
            file.sync_all()?;
            journal.set_len(committed as u64)?;
            journal.sync_all()?;
            sync_dir(root)?;
        }
        private_dir(&root.join("blobs"))?;
        let mut broker = Self {
            state,
            root: root.into(),
            journal,
            _lock: lock,
            checksum,
            poisoned: false,
            directory,
            runtime_root: PathBuf::from("/tmp"),
            name_refusals: BTreeMap::new(),
            #[cfg(feature = "join-by-name")]
            join_runtime: join::Runtime::default(),
        };
        if sequence == 0 {
            broker.commit(broker.state.clone(), "system", "workspace.initialize")?;
        }
        Ok(broker)
    }
    fn commit(&mut self, mut state: State, actor: &str, operation: &str) -> Result<()> {
        ensure!(
            !self.poisoned,
            "storage_failed: restart and recover before further mutations"
        );
        state.sequence = self.state.sequence + 1;
        let after = serde_json::to_value(&state)?;
        ensure!(serde_json::to_vec(&after)?.len()<=16*1024*1024,"quota_exceeded: workspace logical state exceeds 16 MiB; reads remain available but further mutations require a new workspace or a supported retention upgrade; in-place pruning is not supported");
        let before = if self.state.sequence == 0 {
            Value::Null
        } else {
            serde_json::to_value(&self.state)?
        };
        let mut patches = Vec::new();
        delta(&before, &after, &mut Vec::new(), &mut patches);
        let mut record = DeltaRecord {
            version: 2,
            sequence: state.sequence,
            previous: self.checksum.clone(),
            checksum: String::new(),
            actor: actor.into(),
            operation: operation.into(),
            timestamp: now(),
            patches,
        };
        record.checksum = digest(&serde_json::to_vec(&(
            &record.version,
            &record.sequence,
            &record.previous,
            &record.actor,
            &record.operation,
            &record.timestamp,
            &record.patches,
        ))?);
        let mut bytes = serde_json::to_vec(&record)?;
        bytes.push(b'\n');
        ensure!(
            bytes.len() <= 16 * 1024 * 1024
                && self.journal.metadata()?.len() + bytes.len() as u64 <= 1024 * 1024 * 1024,
            "quota_exceeded: retained audit journal exceeds 1 GiB; preserve the complete store and use a new workspace; in-place audit deletion is not supported"
        );
        self.poisoned = true;
        self.journal.write_all(&bytes)?;
        self.journal.sync_all()?;
        sync_dir(&self.root)?;
        self.state = state;
        self.checksum = record.checksum;
        self.poisoned = false;
        Ok(())
    }
    pub fn workspace(&self) -> &Workspace {
        &self.state.workspace
    }
    pub fn handle(&mut self, uid: u32, connection: &mut Connection, request: Request) -> Response {
        let result = self.process(uid, connection, &request);
        match result {
            Ok(value) => Response {
                id: request.id,
                result: Some(value),
                error: None,
            },
            Err(error) => {
                let message = error.to_string();
                let code = message
                    .split(':')
                    .next()
                    .filter(|s| s.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
                    .unwrap_or("request_denied")
                    .to_owned();
                Response {
                    id: request.id,
                    result: None,
                    error: Some(ProtocolError { code, message }),
                }
            }
        }
    }
    fn process(&mut self, uid: u32, conn: &mut Connection, req: &Request) -> Result<Value> {
        ensure!(
            req.version == 1 && !req.id.is_empty() && req.id.len() <= 128,
            "invalid_request: version/id"
        );
        if req.method == "hello" {
            return self.hello(req);
        }
        if req.method == "auth.challenge" {
            return self.challenge(uid, conn, req);
        }
        if req.method == "auth.bootstrap" || req.method == "auth.enroll" {
            return self.enroll(uid, conn, req);
        }
        // `enrollment.pending` and `auth.join` are pre-authentication, like `hello` and
        // `auth.*`: the kernel UID and (for `auth.join`) a signature by the claimed key.
        #[cfg(feature = "join-by-name")]
        if let Some(result) = join::pre_auth(self, uid, conn, req) {
            return result;
        }
        let actor = match self.authenticate_actor(uid, conn, req)? {
            Admission::Actor(actor) => *actor,
            Admission::Replay(result) => return Ok(result),
        };
        if matches!(
            req.method.as_str(),
            "workspace.snapshot"
                | "messages.history"
                | "messages.search"
                | "context.manifest"
                | "run.remote_scope"
                | "blob.read"
                | "blob.status"
                | "reference.get"
                | "profile.suggest"
        ) {
            return self.read(&actor, req);
        }
        let result = self.apply_mutation(&actor, req)?;
        self.project_mutation_result(&actor, req, result)
    }
    fn authenticate_actor(
        &self,
        uid: u32,
        conn: &mut Connection,
        req: &Request,
    ) -> Result<Admission> {
        let actor = if let Some(credential) = &req.credential {
            ensure!(req.auth.is_none(), "invalid_request: mixed authentication");
            let run_id = self
                .state
                .grants
                .get(&digest(credential.as_bytes()))
                .ok_or_else(|| anyhow!("unauthorized: invalid grant"))?;
            let run = self
                .state
                .runs
                .get(run_id)
                .ok_or_else(|| anyhow!("unauthorized: invalid grant"))?
                .clone();
            if let Some(result) = self.terminal_replay(&run, uid, req)? {
                return Ok(Admission::Replay(result));
            }
            self.validate_run(&run, uid)?;
            ensure!(
                matches!(
                    req.method.as_str(),
                    "messages.history"
                        | "messages.search"
                        | "context.manifest"
                        | "run.project"
                        | "blob.begin"
                        | "blob.chunk"
                        | "blob.finish"
                        | "run.remote_scope"
                        | "blob.read"
                        | "blob.status"
                        | "reference.create"
                        | "reference.get"
                ),
                "forbidden: worker operation unavailable"
            );
            Actor {
                id: run.owner_id.clone(),
                run: Some(run),
            }
        } else {
            let auth = req
                .auth
                .as_ref()
                .ok_or_else(|| anyhow!("unauthorized: signed device required"))?;
            let device = self
                .state
                .devices
                .get(&auth.device_id)
                .ok_or_else(|| anyhow!("unauthorized: unknown device"))?
                .clone();
            let principal = self
                .state
                .principals
                .get(&device.principal_id)
                .ok_or_else(|| anyhow!("unauthorized"))?;
            ensure!(
                principal.active
                    && principal.uid == uid
                    && principal.username == self.directory.by_uid(uid)?.name,
                "unauthorized: account enrollment changed"
            );
            self.verify(uid, conn, req, &device.public_key)?;
            Actor {
                id: device.principal_id,
                run: None,
            }
        };
        Ok(Admission::Actor(Box::new(actor)))
    }
    fn terminal_replay(&self, run: &Run, uid: u32, req: &Request) -> Result<Option<Value>> {
        if run.revoked && req.method == "run.project" {
            if let Some(key) = req.params.get("idempotency_key").and_then(Value::as_str) {
                let scope_key = format!("{}:{}:{key}", run.owner_id, run.id);
                if let Some(cached) = self.state.dedupe.get(&scope_key) {
                    let mut live = run.clone();
                    live.revoked = false;
                    self.validate_run(&live, uid)?;
                    let fingerprint = digest(&serde_json::to_vec(&json!([
                        req.method,
                        canonical(&req.params)
                    ]))?);
                    ensure!(
                        fingerprint == cached.digest,
                        "conflict: idempotency key reused with different request"
                    );
                    ensure!(
                        cached
                            .result
                            .get("status")
                            .and_then(Value::as_str)
                            .is_some_and(|s| matches!(s, "completed" | "failed" | "cancelled")),
                        "grant_expired: run revoked"
                    );
                    let actor = Actor {
                        id: run.owner_id.clone(),
                        run: Some(live),
                    };
                    return self
                        .project_mutation_result(&actor, req, cached.result.clone())
                        .map(Some);
                }
            }
        }
        Ok(None)
    }
    fn apply_mutation(&mut self, actor: &Actor, req: &Request) -> Result<Value> {
        let key = text(&req.params, "idempotency_key")?;
        ensure!(key.len() <= 128, "invalid_params: idempotency key too long");
        let scope = actor.run.as_ref().map(|r| r.id.as_str()).unwrap_or("human");
        let key = format!("{}:{scope}:{key}", actor.id);
        let fingerprint = digest(&serde_json::to_vec(&json!([
            req.method,
            canonical(&req.params)
        ]))?);
        if let Some(saved) = self.state.dedupe.get(&key) {
            if let Some(channel) = req.params.get("channel_id").and_then(Value::as_str) {
                // The host may add people to a channel it is not in (direct add), so its
                // retry of that add is re-authorized as the host, not as a member.
                let host_add = req.method == "channel.add_member"
                    && self.manager(&self.state, &actor.id).is_ok();
                if !host_add {
                    self.channel(&self.state, &actor.id, channel, false)?;
                }
            }
            if let Some(blob_id) = req.params.get("blob_id").and_then(Value::as_str) {
                let blob = self
                    .state
                    .blobs
                    .get(blob_id)
                    .ok_or_else(|| anyhow!("forbidden: attachment unavailable"))?;
                self.blob_authorized(&self.state, actor, blob)?;
            }
            ensure!(
                saved.digest == fingerprint,
                "conflict: idempotency key reused with different request"
            );
            return Ok(saved.result.clone());
        }
        ensure!(
            !self.poisoned,
            "storage_failed: restart and recover before further mutations"
        );
        ensure!(
            self.state.dedupe.len() < 100_000,
            "quota_exceeded: workspace operation quota requires maintenance"
        );
        let naming = NAME_METHODS.contains(&req.method.as_str());
        if naming {
            // Checked before anything about the name is evaluated, so the answer to a
            // rate-limited attempt is the same whether or not the name is taken.
            ensure!(
                self.recent_name_refusals(&actor.id, now()) < NAME_REFUSAL_LIMIT,
                NAME_RATE_LIMITED
            );
        }
        let mut state = self.state.clone();
        #[cfg(feature = "join-by-name")]
        join::prune_expired(&mut state, now());
        let result = match self.mutate(&mut state, actor, req) {
            Ok(result) => result,
            Err(error) => {
                if naming && error.to_string().starts_with("name_taken:") {
                    self.record_name_refusal(&actor.id, now());
                }
                return Err(error);
            }
        };
        state.dedupe.insert(
            key,
            Cached {
                digest: fingerprint,
                result: result.clone(),
            },
        );
        if let Err(error) = self.commit(state, &actor.id, &req.method) {
            if req.method == "blob.begin" && !self.poisoned {
                if let Some(blob_id) = result.get("id").and_then(Value::as_str) {
                    let _ = fs::remove_file(self.root.join("blobs").join(blob_id));
                }
            }
            return Err(error);
        }
        Ok(result)
    }
    fn hello(&self, req: &Request) -> Result<Value> {
        let secret: [u8; 32] = hex::decode(&self.state.workspace_signing_key)?
            .try_into()
            .map_err(|_| anyhow!("storage_corrupt: workspace identity"))?;
        let key = SigningKey::from_bytes(&secret);
        let public_key = hex::encode(key.verifying_key().to_bytes());
        let nonce = req
            .params
            .get("challenge_nonce")
            .and_then(Value::as_str)
            .unwrap_or("");
        ensure!(
            nonce.len() <= 128,
            "invalid_params: challenge nonce too long"
        );
        let node_id = node_identity()?;
        ensure!(
            self.state
                .writer_node_id
                .as_ref()
                .is_none_or(|expected| expected == &node_id),
            "node_identity_changed: workspace belongs to another writer node"
        );
        let workspace = &self.state.workspace;
        let signature = hex::encode(
            key.sign(&hello_v1_payload(
                &workspace.id,
                workspace.host_uid,
                nonce,
                &public_key,
                &node_id,
            ))
            .to_bytes(),
        );
        let capabilities = Self::capabilities();
        // v2 is sent beside v1: a daemon that verifies it may trust the name, mode,
        // institution, policy epoch and capabilities for display. PROTOCOL_VERSION stays 1.
        let signature_v2 = hex::encode(
            key.sign(
                &HelloV2 {
                    workspace_id: &workspace.id,
                    host_uid: workspace.host_uid,
                    challenge_nonce: nonce,
                    workspace_public_key: &public_key,
                    node_id: &node_id,
                    mode: &workspace.mode,
                    institution_id: workspace.institution_id.as_deref(),
                    policy_epoch: workspace.policy_epoch,
                    name: workspace.name.as_deref(),
                    capabilities: &capabilities,
                }
                .signing_payload(),
            )
            .to_bytes(),
        );
        Ok(
            json!({"protocol":1,"workspace_id":workspace.id,"host_uid":workspace.host_uid,"mode":workspace.mode,"institution_id":workspace.institution_id,"policy_epoch":workspace.policy_epoch,"name":workspace.name,"workspace_public_key":public_key,"node_id":node_id,"workspace_key_fingerprint":digest(&key.verifying_key().to_bytes()),"challenge_nonce":nonce,"signature":signature,"signature_v2":signature_v2,"capabilities":capabilities,"unsupported":["arbitrary_shell","remote_filesystem","network_filesystem","cross_workspace_release"]}),
        )
    }
    /// What `hello` advertises, in the order it advertises it (the v2 signature covers the
    /// order).
    fn capabilities() -> Vec<&'static str> {
        #[allow(unused_mut)]
        let mut capabilities = vec![
            "human_chat",
            "signed_devices",
            "resumable_blobs",
            "scoped_runs",
            "human_names_v1",
            "unique_names_v1",
            "direct_add_v1",
        ];
        #[cfg(feature = "join-by-name")]
        capabilities.push("join_by_name_v1");
        capabilities
    }
    fn challenge(&self, uid: u32, conn: &mut Connection, req: &Request) -> Result<Value> {
        let device = text(&req.params, "device_id")?;
        ensure!(device.len() == 64, "invalid_params: device fingerprint");
        conn.challenges.retain(|_, (_, expiry)| *expiry >= now());
        ensure!(
            conn.challenges.len() < 16,
            "rate_limited: too many live challenges"
        );
        let nonce = token();
        let expiry = now() + 60;
        conn.challenges
            .insert(nonce.clone(), (device.into(), expiry));
        Ok(
            json!({"nonce":nonce,"workspace_id":self.state.workspace.id,"uid":uid,"expires_at":expiry}),
        )
    }
    fn verify(
        &self,
        uid: u32,
        conn: &mut Connection,
        req: &Request,
        public_key: &str,
    ) -> Result<()> {
        let auth = req
            .auth
            .as_ref()
            .ok_or_else(|| anyhow!("unauthorized: signature required"))?;
        let (device, expiry) = conn
            .challenges
            .remove(&auth.nonce)
            .ok_or_else(|| anyhow!("unauthorized: challenge missing or consumed"))?;
        let bytes = hex::decode(public_key)?;
        ensure!(
            device == auth.device_id,
            "unauthorized: challenge belongs to another device"
        );
        ensure!(
            auth.device_id == digest(&bytes),
            "unauthorized: enrollment public key does not match the signing device"
        );
        ensure!(
            expiry >= now(),
            "unauthorized: authentication challenge expired; retry the action"
        );
        let key: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow!("unauthorized: invalid key"))?;
        let signature = Signature::from_slice(&hex::decode(&auth.signature)?)?;
        VerifyingKey::from_bytes(&key)?
            .verify_strict(
                &signing_payload(
                    &self.state.workspace.id,
                    uid,
                    &auth.nonce,
                    &req.method,
                    &req.params,
                ),
                &signature,
            )
            .map_err(|_| anyhow!("unauthorized: invalid signature"))
    }
    fn enroll(&mut self, uid: u32, conn: &mut Connection, req: &Request) -> Result<Value> {
        let public_key = text(&req.params, "public_key")?;
        let mut existing_principal_id = None;
        if req.method == "auth.bootstrap" {
            ensure!(
                uid == self.state.workspace.host_uid
                    && self.state.devices.is_empty()
                    && public_key == self.state.bootstrap_key,
                "forbidden: bootstrap key/identity mismatch or already enrolled"
            );
        } else {
            let invitation = text(&req.params, "invitation")?;
            let expected = self
                .state
                .enrollments
                .get(&digest(invitation.as_bytes()))
                .ok_or_else(|| anyhow!("forbidden: enrollment invitation unavailable"))?;
            ensure!(
                expected.uid == uid
                    && expected.public_key == public_key
                    && expected.expires_at >= now(),
                "forbidden: enrollment binding mismatch or expired"
            );
            existing_principal_id = expected.existing_principal_id.clone();
        }
        self.verify(uid, conn, req, public_key)?;
        let mut state = self.state.clone();
        #[cfg(feature = "join-by-name")]
        join::prune_expired(&mut state, now());
        let current_username = self.directory.by_uid(uid)?.name;
        let active = state.principals.values().find(|p| p.uid == uid && p.active);
        let principal = match (existing_principal_id.as_deref(), active) {
            (Some(expected), Some(principal)) if principal.id == expected && principal.username == current_username => principal.clone(),
            (None, None) => {
                Self::ensure_username_free(&state, uid, &current_username)?;
                Principal {
                    id: id(), uid, username: current_username.clone(), nickname: current_username,
                    avatar: None, active: true,
                }
            }
            _ => bail!("identity_mismatch: enrollment principal changed; request a new invitation explicitly identifying the existing principal or offboard the old account"),
        };
        let device_id = digest(&hex::decode(public_key)?);
        Self::ensure_new_device(&state, &device_id)?;
        state.devices.insert(
            device_id.clone(),
            Device {
                principal_id: principal.id.clone(),
                public_key: public_key.into(),
                added_at: Some(now()),
                added_via: Some(
                    if req.method == "auth.bootstrap" {
                        "bootstrap"
                    } else {
                        "token"
                    }
                    .into(),
                ),
            },
        );
        state
            .principals
            .insert(principal.id.clone(), principal.clone());
        if req.method == "auth.enroll" {
            state
                .enrollments
                .remove(&digest(text(&req.params, "invitation")?.as_bytes()));
            #[cfg(feature = "join-by-name")]
            join::on_legacy_enrolled(&mut state, uid);
        }
        self.commit(state, &principal.id, &req.method)?;
        Ok(json!({"principal":principal,"device_id":device_id,"workspace":self.state.workspace}))
    }
    /// D3: no two active principals share a canonical username. Refuses creating a principal
    /// for `uid` named `username` while another UID's active principal has a colliding name
    /// (case, width, separators, invisible characters or lookalikes ignored).
    fn ensure_username_free(s: &State, uid: u32, username: &str) -> Result<()> {
        if let Some(other) = s
            .principals
            .values()
            .find(|p| p.active && p.uid != uid && names_collide(&p.username, username))
        {
            bail!(
                "identity_conflict: another active member is @{0}; remove the old @{0} first",
                other.username
            );
        }
        Ok(())
    }
    /// SR11: a key that is already a device of this workspace is never bound again, to the
    /// same principal or another.
    fn ensure_new_device(s: &State, device_id: &str) -> Result<()> {
        ensure!(
            !s.devices.contains_key(device_id),
            "device_conflict: this device key is already enrolled in this workspace; use a new device key"
        );
        Ok(())
    }
    fn protected_channel_ids(s: &State) -> BTreeSet<&str> {
        s.channels
            .values()
            .filter(|channel| channel.classification == Classification::Restricted)
            .map(|channel| channel.id.as_str())
            .chain(
                s.messages
                    .iter()
                    .filter(|message| message.restricted)
                    .map(|message| message.channel_id.as_str()),
            )
            .chain(
                s.blobs
                    .values()
                    .filter(|blob| blob.restricted)
                    .map(|blob| blob.channel_id.as_str()),
            )
            .chain(
                s.references
                    .values()
                    .filter(|reference| reference.restricted)
                    .map(|reference| reference.channel_id.as_str()),
            )
            .collect()
    }
    fn is_protected_context(
        s: &State,
        sources: &BTreeSet<String>,
        personal_mode: &Mode,
        remote_root: Option<&str>,
    ) -> bool {
        let protected_channels = Self::protected_channel_ids(s);
        s.workspace.mode == Mode::Private
            || *personal_mode == Mode::Private
            || remote_root.is_some()
            || sources
                .iter()
                .any(|channel| protected_channels.contains(channel.as_str()))
    }
    fn enforce_institution_policy(
        s: &State,
        sources: &BTreeSet<String>,
        personal_mode: &Mode,
        remote_root: Option<&str>,
        affiliation: &ProviderAffiliation,
        connection_institution_id: Option<&str>,
    ) -> Result<()> {
        if let Some(institution) = &s.workspace.institution_id {
            ensure!(
                is_canonical_institution_id(institution),
                "privacy_denied: workspace institution is invalid"
            );
        }
        if let Some(institution) = connection_institution_id {
            ensure!(
                is_canonical_institution_id(institution),
                "privacy_denied: connection institution is invalid"
            );
            if let Some(workspace_institution) = &s.workspace.institution_id {
                ensure!(
                    institution == workspace_institution,
                    "privacy_denied: connection and workspace institutions differ"
                );
            }
        }
        if let ProviderAffiliation::Institutions { institution_ids } = affiliation {
            ensure!(
                !institution_ids.is_empty()
                    && institution_ids.len() <= 32
                    && institution_ids
                        .iter()
                        .all(|id| is_canonical_institution_id(id)),
                "invalid_params: invalid provider institution affiliation"
            );
        }
        if Self::is_protected_context(s, sources, personal_mode, remote_root) {
            let institution = s.workspace.institution_id.as_deref().ok_or_else(|| anyhow!("privacy_denied: workspace institution must be labelled before granting agent access"))?;
            ensure!(
                *personal_mode != Mode::Private || connection_institution_id == Some(institution),
                "privacy_denied: Private connection requires the workspace institution label"
            );
            ensure!(
                affiliation.allows_institution(institution),
                "privacy_denied: provider affiliation does not match the workspace institution"
            );
        }
        Ok(())
    }
    fn validate_run(&self, run: &Run, uid: u32) -> Result<()> {
        let p = self
            .state
            .principals
            .get(&run.owner_id)
            .ok_or_else(|| anyhow!("unauthorized"))?;
        ensure!(
            p.active
                && p.uid == uid
                && p.username == self.directory.by_uid(uid)?.name
                && !run.revoked
                && run.expires_at >= now()
                && run.policy_epoch == self.state.workspace.policy_epoch,
            "grant_expired: run revoked, expired or policy changed"
        );
        ensure!(
            run.protected_context || !Self::is_protected_context(
                &self.state,
                &run.source_channels,
                &run.personal_mode,
                run.remote_root.as_deref(),
            ),
            "grant_expired: selected context became protected; refresh and obtain a fresh human grant"
        );
        if run.public_provider {
            ensure!(
                self.state.workspace.mode == Mode::Public && run.personal_mode == Mode::Public,
                "privacy_denied: Private workspace or connection"
            );
        }
        for channel in &run.source_channels {
            let source = self.channel(&self.state, &run.owner_id, channel, false)?;
            if run.public_provider {
                ensure!(source.classification==Classification::PublicSafe&&!self.state.messages.iter().any(|message|message.channel_id==*channel&&message.restricted),"privacy_denied: selected context gained restricted material; create a fresh compatible run");
            }
        }
        self.channel(&self.state, &run.owner_id, &run.channel_id, true)?;
        ensure!(
            run.workspace_institution_id == self.state.workspace.institution_id,
            "grant_expired: workspace institution changed"
        );
        Self::enforce_institution_policy(
            &self.state,
            &run.source_channels,
            &run.personal_mode,
            run.remote_root.as_deref(),
            &run.provider_affiliation,
            run.connection_institution_id.as_deref(),
        )?;
        if let Some(root) = &run.remote_root {
            self.protect_authority_root(root, p.uid)?;
        }
        Ok(())
    }
    fn protect_authority_root(&self, root: &str, uid: u32) -> Result<()> {
        if uid == self.state.workspace.host_uid {
            let root =
                fs::canonicalize(root).context("invalid_scope: host work directory must exist")?;
            ensure!(
                !root.starts_with(&self.root) && !self.root.starts_with(&root),
                "forbidden: remote work scope overlaps broker authority state"
            );
        }
        Ok(())
    }
    fn channel<'a>(
        &self,
        state: &'a State,
        actor: &str,
        channel: &str,
        writing: bool,
    ) -> Result<&'a Channel> {
        let c = state
            .channels
            .get(channel)
            .filter(|c| c.members.contains(actor))
            .ok_or_else(|| anyhow!("forbidden: channel unavailable"))?;
        ensure!(
            !writing || !c.archived,
            "channel_archived: channel is read-only"
        );
        Ok(c)
    }
    fn visible(&self, state: &State, actor: &Actor, message: &Message) -> bool {
        self.channel(state, &actor.id, &message.channel_id, false)
            .is_ok()
            && message
                .source_channels
                .iter()
                .all(|s| self.channel(state, &actor.id, s, false).is_ok())
            && actor.run.as_ref().is_none_or(|r| {
                (!r.public_provider || !message.restricted)
                    && message.source_channels.is_subset(&r.source_channels)
            })
    }
    fn read(&self, actor: &Actor, req: &Request) -> Result<Value> {
        match req.method.as_str() {
            "workspace.snapshot" => self.read_workspace_snapshot(actor, req),
            "messages.history" | "messages.search" => self.read_messages_history(actor, req),
            "run.remote_scope" => self.read_run_remote_scope(actor, req),
            "context.manifest" => self.read_context_manifest(actor, req),
            "reference.get" => self.read_reference_get(actor, req),
            "blob.status" => self.read_blob_status(actor, req),
            "blob.read" => self.read_blob_read(actor, req),
            "profile.suggest" => self.read_profile_suggest(actor),
            _ => bail!("unsupported: method unavailable"),
        }
    }
    fn message_wire(message: &Message) -> Value {
        let mut value = json!(message);
        value["sequence"] = json!(message.id);
        value
    }
    /// The display-only names that go beside messages in a result: `people` for every author,
    /// and `channel_names` for the channels the messages name, each checked at runtime against
    /// what the actor can read (never a `debug_assert!`), so no name leaves the actor's view.
    fn message_names<'a>(
        &self,
        s: &State,
        actor: &Actor,
        messages: impl IntoIterator<Item = &'a Message>,
    ) -> (Value, Value) {
        let index = PeopleIndex::new(s);
        let mut people = serde_json::Map::new();
        let mut channels = BTreeSet::new();
        for message in messages {
            if !people.contains_key(&message.actor_id) {
                if let Some(principal) = s.principals.get(&message.actor_id) {
                    people.insert(message.actor_id.clone(), index.person_wire(principal));
                }
            }
            channels.insert(message.channel_id.as_str());
            channels.extend(message.source_channels.iter().map(String::as_str));
        }
        let channel_names: serde_json::Map<String, Value> = channels
            .into_iter()
            .filter_map(|id| {
                self.channel(s, &actor.id, id, false)
                    .ok()
                    .map(|channel| (id.to_owned(), json!(sanitize_channel_name(&channel.name))))
            })
            .collect();
        (Value::Object(people), Value::Object(channel_names))
    }
    fn read_profile_suggest(&self, actor: &Actor) -> Result<Value> {
        let s = &self.state;
        let principal = s
            .principals
            .get(&actor.id)
            .ok_or_else(|| anyhow!("unauthorized"))?;
        // The actor's own account only: one NSS lookup, never on the snapshot path. The
        // suggestion passes the same rules a person's own display name must pass, or nothing
        // is suggested.
        let full_name = self
            .directory
            .by_uid(principal.uid)?
            .full_name
            .and_then(|name| {
                validate_display_name_for(
                    &name,
                    &principal.username,
                    s.principals
                        .values()
                        .filter(|other| other.id != principal.id)
                        .map(|other| other.username.as_str()),
                )
                .ok()
            });
        Ok(json!({ "full_name": full_name }))
    }
    fn read_position_wire(&self, s: &State, actor: &Actor, channel: &str, sequence: u64) -> Value {
        json!(s
            .messages
            .iter()
            .rev()
            .find(|message| {
                message.channel_id == channel
                    && message.sequence <= sequence
                    && self.visible(s, actor, message)
            })
            .map(|message| &message.id))
    }
    fn resolve_cursor(
        &self,
        s: &State,
        actor: &Actor,
        channel: &str,
        value: Option<&Value>,
        default: u64,
    ) -> Result<u64> {
        let Some(value) = value.filter(|value| !value.is_null()) else {
            return Ok(default);
        };
        let token = value.as_str().ok_or_else(|| {
            anyhow!(
                "invalid_params: cursor must be an opaque message token; refresh channel history"
            )
        })?;
        s.messages
            .iter()
            .find(|message| {
                message.id == token
                    && message.channel_id == channel
                    && self.visible(s, actor, message)
                    && actor
                        .run
                        .as_ref()
                        .is_none_or(|run| run.source_channels.contains(channel))
            })
            .map(|message| message.sequence)
            .ok_or_else(|| {
                anyhow!("stale_cursor: cursor unavailable; refresh authorized channel history")
            })
    }
    fn project_mutation_result(
        &self,
        actor: &Actor,
        req: &Request,
        mut result: Value,
    ) -> Result<Value> {
        match req.method.as_str() {
            "message.post" | "run.project" => {
                let message = self
                    .state
                    .messages
                    .iter()
                    .find(|message| {
                        Some(message.id.as_str()) == result.get("id").and_then(Value::as_str)
                            && self.visible(&self.state, actor, message)
                    })
                    .ok_or_else(|| anyhow!("forbidden: message unavailable"))?;
                let mut wire = Self::message_wire(message);
                let (people, channel_names) =
                    self.message_names(&self.state, actor, std::iter::once(message));
                wire["people"] = people;
                wire["channel_names"] = channel_names;
                Ok(wire)
            }
            "channel.read" => {
                let channel = text(&req.params, "channel_id")?;
                let token = text(&req.params, "sequence")?;
                self.resolve_cursor(&self.state, actor, channel, Some(&json!(token)), 0)?;
                let sequence = result
                    .get("sequence")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| anyhow!("storage_corrupt: read watermark unavailable"))?;
                result["sequence"] = self.read_position_wire(&self.state, actor, channel, sequence);
                Ok(result)
            }
            _ => Ok(result),
        }
    }
    fn read_workspace_snapshot(&self, actor: &Actor, _req: &Request) -> Result<Value> {
        let s = &self.state;
        let protected_channel_ids: Vec<&str> = Self::protected_channel_ids(s)
            .into_iter()
            .filter(|channel| self.channel(s, &actor.id, channel, false).is_ok())
            .collect();
        let (positions, unread) = self.read_state(s, actor);
        let host = self.manager(s, &actor.id).is_ok();
        let now = now();
        let index = PeopleIndex::new(s);
        let teams: Vec<&Team> = s
            .teams
            .values()
            .filter(|t| t.members.contains(&actor.id))
            .collect();
        let channels: Vec<&Channel> = s
            .channels
            .values()
            .filter(|c| c.members.contains(&actor.id))
            .collect();
        // An invitee no longer sees an invitation once it has expired (it can never be
        // accepted); its inviter still does, marked `expired`.
        let invitations: Vec<&Invitation> = s
            .invitations
            .values()
            .filter(|i| {
                i.inviter_id == actor.id || (i.principal_id == actor.id && i.expires_at >= now)
            })
            .collect();
        let mut workspace = json!(s.workspace);
        workspace["host_principal_id"] = json!(host_principal_id(s));
        let stale = if host {
            self.stale_principals(s)
        } else {
            BTreeSet::new()
        };
        let principals: Vec<Value> = s
            .principals
            .values()
            .filter(|p| p.active)
            .map(|p| {
                let mut wire = index.principal_wire(p);
                if stale.contains(p.id.as_str()) {
                    wire["account_stale"] = json!(true);
                }
                wire
            })
            .collect();
        let former_principals = Self::former_principals(s, &index, &teams, &channels, &invitations);
        let actor_wire = Self::actor_wire(s, &index, &actor.id);
        let teams_wire = Self::teams_wire(&teams);
        let channels_wire = Self::channels_wire(&channels);
        let invitations_wire: Vec<Value> = invitations
            .iter()
            .map(|invitation| Self::invitation_wire(s, &index, invitation, now))
            .collect();
        let mut snapshot = json!({"workspace":workspace,"protected_channel_ids":protected_channel_ids,"actor":actor_wire,"principals":principals,"former_principals":former_principals,"teams":teams_wire,"channels":channels_wire,"invitations":invitations_wire,"runs":s.runs.values().filter(|r|r.owner_id==actor.id).collect::<Vec<_>>(),"read_positions":positions,"unread":unread,"references":s.references.values().filter(|r|self.reference_authorized(s,actor,r).is_ok()).collect::<Vec<_>>()});
        if host {
            let refusals: BTreeMap<&str, usize> = self
                .name_refusals
                .keys()
                .map(|id| (id.as_str(), self.recent_name_refusals(id, now)))
                .filter(|(_, count)| *count > 0)
                .collect();
            snapshot["name_collision_refusals"] = json!(refusals);
            #[cfg(feature = "join-by-name")]
            {
                snapshot["pending_joins"] = join::project_for_manager(self, s);
            }
        }
        Ok(snapshot)
    }
    /// Per visible channel, the actor's read watermark (as an opaque message token) and the
    /// number of unread messages from others.
    fn read_state(
        &self,
        s: &State,
        actor: &Actor,
    ) -> (BTreeMap<String, Value>, BTreeMap<String, usize>) {
        let mut positions = BTreeMap::new();
        let mut unread = BTreeMap::new();
        for channel in s
            .channels
            .values()
            .filter(|c| c.members.contains(&actor.id))
        {
            let sequence = *s
                .read_positions
                .get(&format!("{}:{}", actor.id, channel.id))
                .unwrap_or(&0);
            positions.insert(
                channel.id.clone(),
                self.read_position_wire(s, actor, &channel.id, sequence),
            );
            let count = s
                .messages
                .iter()
                .filter(|m| {
                    m.channel_id == channel.id
                        && m.sequence > sequence
                        && m.actor_id != actor.id
                        && self.visible(s, actor, m)
                })
                .count();
            unread.insert(channel.id.clone(), count);
        }
        (positions, unread)
    }
    /// The actor's own principal with its display name and its devices
    /// (`{fingerprint, added_at, added_via}`; the fingerprint is the grouped first 16 hex digits
    /// of the device ID). Nobody else's devices are ever projected.
    fn actor_wire(s: &State, index: &PeopleIndex, actor_id: &str) -> Option<Value> {
        s.principals.get(actor_id).map(|principal| {
            let mut wire = index.principal_wire(principal);
            wire["devices"] = Value::Array(
                s.devices
                    .iter()
                    .filter(|(_, device)| device.principal_id == actor_id)
                    .map(|(device_id, device)| {
                        json!({
                            "fingerprint": crate::invitation::grouped_fingerprint(device_id),
                            "added_at": device.added_at,
                            "added_via": device.added_via,
                        })
                    })
                    .collect(),
            );
            wire
        })
    }
    /// Inactive principals referenced by the actor's visible objects: the members, creator,
    /// owner and pending owner of visible teams and channels, and the invitee and inviter of
    /// visible invitations. Display only; bounded by team and channel sizes.
    fn former_principals(
        s: &State,
        index: &PeopleIndex,
        teams: &[&Team],
        channels: &[&Channel],
        invitations: &[&Invitation],
    ) -> Vec<Value> {
        let mut referenced: BTreeSet<&str> = BTreeSet::new();
        for team in teams {
            referenced.extend(team.members.iter().map(String::as_str));
            referenced.insert(&team.created_by);
        }
        for channel in channels {
            referenced.extend(channel.members.iter().map(String::as_str));
            referenced.insert(&channel.created_by);
            referenced.insert(&channel.owner_id);
            referenced.extend(channel.pending_owner.as_deref());
        }
        for invitation in invitations {
            referenced.insert(&invitation.principal_id);
            referenced.insert(&invitation.inviter_id);
        }
        referenced
            .into_iter()
            .filter_map(|id| s.principals.get(id))
            .filter(|p| !p.active)
            .map(|p| {
                json!({
                    "id": p.id,
                    "username": p.username,
                    "display_name": index.display_name(p),
                    "avatar": p.avatar,
                    "active": false,
                })
            })
            .collect()
    }
    /// Visible teams with their computed, never stored, name fields: the sanitized
    /// `display_name`, the `handle` a resolver matches against, `name_conflict` (another team
    /// **the viewer can see** has the same name) and `name_invalid` (a legacy name the current
    /// rules refuse).
    fn teams_wire(teams: &[&Team]) -> Vec<Value> {
        let keys: Vec<(String, String)> = teams
            .iter()
            .map(|t| (name_key(&t.name), skeleton_key(&t.name)))
            .collect();
        teams
            .iter()
            .enumerate()
            .map(|(position, team)| {
                let conflict = keys.iter().enumerate().any(|(other, key)| {
                    other != position && (key.0 == keys[position].0 || key.1 == keys[position].1)
                });
                let mut wire = json!(team);
                wire["display_name"] = json!(sanitize_team_name(&team.name));
                wire["handle"] = json!(keys[position].0);
                wire["name_conflict"] = json!(conflict);
                wire["name_invalid"] = json!(validate_team_name(&team.name).is_err());
                wire
            })
            .collect()
    }
    /// Visible channels with the same computed fields as [`Self::teams_wire`]; a conflict is
    /// another visible channel **in the same team**. A stored name that is not a canonical
    /// slug (a legacy `Data Analysis`) is `name_invalid`.
    fn channels_wire(channels: &[&Channel]) -> Vec<Value> {
        let keys: Vec<(String, String)> = channels
            .iter()
            .map(|c| (name_key(&c.name), skeleton_key(&c.name)))
            .collect();
        channels
            .iter()
            .enumerate()
            .map(|(position, channel)| {
                let conflict = channels.iter().enumerate().any(|(other, peer)| {
                    other != position
                        && peer.team_id == channel.team_id
                        && (keys[other].0 == keys[position].0 || keys[other].1 == keys[position].1)
                });
                let mut wire = json!(channel);
                wire["display_name"] = json!(sanitize_channel_name(&channel.name));
                wire["handle"] = json!(keys[position].0);
                wire["name_conflict"] = json!(conflict);
                wire["name_invalid"] =
                    json!(canonical_channel_name(&channel.name)
                        .map_or(true, |slug| slug != channel.name));
                wire
            })
            .collect()
    }
    /// An invitation as its invitee or inviter sees it: the target's current name (the
    /// team's, or the channel's plus its team's), who invited, and whether it has expired.
    /// Naming the target to its intended invitee is the purpose of an invitation.
    fn invitation_wire(s: &State, index: &PeopleIndex, invitation: &Invitation, now: u64) -> Value {
        let (target_name, team_name) = match invitation.kind.as_str() {
            "team" => (
                s.teams
                    .get(&invitation.target_id)
                    .map(|team| sanitize_team_name(&team.name)),
                None,
            ),
            _ => {
                let channel = s.channels.get(&invitation.target_id);
                (
                    channel.map(|channel| sanitize_channel_name(&channel.name)),
                    channel
                        .and_then(|channel| s.teams.get(&channel.team_id))
                        .map(|team| sanitize_team_name(&team.name)),
                )
            }
        };
        let mut wire = json!(invitation);
        wire["target_name"] = json!(target_name);
        if invitation.kind != "team" {
            wire["team_name"] = json!(team_name);
        }
        wire["inviter"] = s
            .principals
            .get(&invitation.inviter_id)
            .map(|inviter| {
                json!({"username": inviter.username, "display_name": index.display_name(inviter)})
            })
            .unwrap_or(Value::Null);
        wire["expired"] = json!(invitation.expires_at < now);
        wire
    }
    /// Host snapshot only: of the active principals whose usernames collide with another
    /// active principal's (a legacy journal from before D3), those whose UID no longer maps to
    /// their username. One NSS lookup per principal in a colliding pair, nothing otherwise.
    fn stale_principals<'s>(&self, s: &'s State) -> BTreeSet<&'s str> {
        let active: Vec<&Principal> = s.principals.values().filter(|p| p.active).collect();
        let keys: Vec<(String, String)> = active
            .iter()
            .map(|p| (name_key(&p.username), skeleton_key(&p.username)))
            .collect();
        active
            .iter()
            .enumerate()
            .filter(|(position, _)| {
                keys.iter().enumerate().any(|(other, key)| {
                    other != *position && (key.0 == keys[*position].0 || key.1 == keys[*position].1)
                })
            })
            .filter(|(_, principal)| {
                !self
                    .directory
                    .by_uid(principal.uid)
                    .is_ok_and(|account| account.name == principal.username)
            })
            .map(|(_, principal)| principal.id.as_str())
            .collect()
    }
    fn read_messages_history(&self, actor: &Actor, req: &Request) -> Result<Value> {
        let s = &self.state;
        let p = &req.params;
        let channel = text(p, "channel_id")?;
        self.channel(s, &actor.id, channel, false)?;
        if let Some(r) = &actor.run {
            ensure!(
                r.source_channels.contains(channel),
                "forbidden: channel outside run grant"
            );
        }
        let after = self.resolve_cursor(s, actor, channel, p.get("after"), 0)?;
        let before = self.resolve_cursor(s, actor, channel, p.get("before"), u64::MAX)?;
        let limit = p
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .min(200) as usize;
        let query = if req.method == "messages.search" {
            text(p, "query")?.to_lowercase()
        } else {
            String::new()
        };
        let matching = s.messages.iter().filter(|m| {
            m.channel_id == channel
                && m.sequence > after
                && m.sequence < before
                && self.visible(s, actor, m)
                && m.body.to_lowercase().contains(&query)
        });
        let messages: Vec<_> = if p.get("latest").and_then(Value::as_bool) == Some(true) {
            let mut latest: Vec<_> = matching.rev().take(limit).collect();
            latest.reverse();
            latest
        } else {
            matching.take(limit).collect()
        };
        let cursor = messages
            .last()
            .map(|message| message.id.clone())
            .or_else(|| p.get("after").and_then(Value::as_str).map(str::to_owned));
        let (people, channel_names) = self.message_names(s, actor, messages.iter().copied());
        let messages: Vec<_> = messages.into_iter().map(Self::message_wire).collect();
        Ok(
            json!({"messages":messages,"cursor":cursor,"people":people,"channel_names":channel_names}),
        )
    }
    fn read_run_remote_scope(&self, actor: &Actor, _req: &Request) -> Result<Value> {
        let run = actor
            .run
            .as_ref()
            .ok_or_else(|| anyhow!("forbidden: worker grant required"))?;
        ensure!(
            !run.public_provider,
            "privacy_denied: remote filesystem is unavailable to public providers"
        );
        let root = run
            .remote_root
            .as_ref()
            .ok_or_else(|| anyhow!("forbidden: run has no remote path approval"))?;
        Ok(
            json!({"root":root,"allow_execute":run.remote_execution,"public_provider":run.public_provider,"run_id":run.id,"policy_epoch":run.policy_epoch,"expires_at":run.expires_at}),
        )
    }
    fn read_context_manifest(&self, actor: &Actor, _req: &Request) -> Result<Value> {
        let s = &self.state;
        let run = actor
            .run
            .as_ref()
            .ok_or_else(|| anyhow!("forbidden: worker grant required"))?;
        let messages: Vec<_> = s
            .messages
            .iter()
            .filter(|m| run.source_channels.contains(&m.channel_id) && self.visible(s, actor, m))
            .rev()
            .take(200)
            .collect();
        let restricted = messages.iter().any(|message| message.restricted);
        let (people, channel_names) = self.message_names(s, actor, messages.iter().copied());
        let messages: Vec<_> = messages.into_iter().map(Self::message_wire).collect();
        Ok(
            json!({"run_id":run.id,"policy_epoch":s.workspace.policy_epoch,"source_channels":run.source_channels,"messages":messages,"restricted":restricted,"people":people,"channel_names":channel_names}),
        )
    }
    fn read_reference_get(&self, actor: &Actor, req: &Request) -> Result<Value> {
        let s = &self.state;
        let p = &req.params;
        let reference = s
            .references
            .get(text(p, "reference_id")?)
            .ok_or_else(|| anyhow!("forbidden: reference unavailable"))?;
        self.reference_authorized(s, actor, reference)?;
        Ok(json!(reference))
    }
    fn read_blob_status(&self, actor: &Actor, req: &Request) -> Result<Value> {
        let s = &self.state;
        let p = &req.params;
        let blob = s
            .blobs
            .get(text(p, "blob_id")?)
            .ok_or_else(|| anyhow!("forbidden: attachment unavailable"))?;
        self.blob_authorized(s, actor, blob)?;
        ensure!(
            blob.complete || blob.owner_id == actor.id,
            "forbidden: incomplete attachment unavailable"
        );
        Ok(json!(blob))
    }
    fn read_blob_read(&self, actor: &Actor, req: &Request) -> Result<Value> {
        let s = &self.state;
        let p = &req.params;
        let blob = s
            .blobs
            .get(text(p, "blob_id")?)
            .filter(|b| b.complete)
            .ok_or_else(|| anyhow!("forbidden: attachment unavailable"))?;
        self.blob_authorized(s, actor, blob)?;
        let offset = number(p, "offset")?;
        ensure!(
            offset <= blob.size,
            "invalid_params: offset beyond attachment"
        );
        let mut file = private_file(&self.root.join("blobs").join(&blob.id), false)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; MAX_CHUNK.min((blob.size - offset) as usize)];
        file.read_exact(&mut bytes)?;
        Ok(
            json!({"blob":blob,"offset":offset,"data_hex":hex::encode(&bytes),"next_offset":offset+bytes.len() as u64,"complete":offset+bytes.len() as u64==blob.size}),
        )
    }
    fn reference_authorized(
        &self,
        s: &State,
        actor: &Actor,
        reference: &RemoteReference,
    ) -> Result<()> {
        self.channel(s, &actor.id, &reference.channel_id, false)?;
        for source in &reference.source_channels {
            self.channel(s, &actor.id, source, false)?;
        }
        if let Some(run) = &actor.run {
            ensure!(
                !run.public_provider
                    && run.source_channels.contains(&reference.channel_id)
                    && reference.source_channels.is_subset(&run.source_channels),
                "privacy_denied: remote reference outside run policy"
            );
        }
        Ok(())
    }
    fn blob_authorized(&self, s: &State, actor: &Actor, blob: &Blob) -> Result<()> {
        self.channel(s, &actor.id, &blob.channel_id, false)?;
        for source in &blob.source_channels {
            self.channel(s, &actor.id, source, false)?;
        }
        if let Some(run) = &actor.run {
            ensure!(
                run.source_channels.contains(&blob.channel_id)
                    && blob.source_channels.is_subset(&run.source_channels)
                    && (!run.public_provider || !blob.restricted),
                "privacy_denied: attachment outside run policy"
            );
        }
        Ok(())
    }
    fn mutate(&mut self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        // S3a: the new form of `enrollment.invite` (params carry `username`),
        // `enrollment.approve` and `enrollment.cancel`.
        #[cfg(feature = "join-by-name")]
        if let Some(result) = join::mutate(self, s, actor, req) {
            return result;
        }
        match req.method.as_str() {
            "channel.read" => self.mutate_channel_read(s, actor, req),
            "profile.update" => self.mutate_profile_update(s, actor, req),
            "enrollment.invite" => self.mutate_enrollment_invite(s, actor, req),
            "enrollment.revoke" => self.mutate_enrollment_revoke(s, actor, req),
            "policy.set" => self.mutate_policy_set(s, actor, req),
            "team.create" => self.mutate_team_create(s, actor, req),
            "team.rename" => self.mutate_team_rename(s, actor, req),
            "channel.create" => self.mutate_channel_create(s, actor, req),
            "channel.rename" => self.mutate_channel_rename(s, actor, req),
            "workspace.rename" => self.mutate_workspace_rename(s, actor, req),
            "invitation.create" => self.mutate_invitation_create(s, actor, req),
            "invitation.accept" => self.mutate_invitation_accept(s, actor, req),
            "team.add_member" => self.mutate_team_add_member(s, actor, req),
            "channel.add_member" => self.mutate_channel_add_member(s, actor, req),
            "channel.archive" | "channel.transfer" | "membership.revoke" => {
                self.mutate_channel_archive(s, actor, req)
            }
            "transfer.accept" => self.mutate_transfer_accept(s, actor, req),
            "message.post" | "run.project" => self.mutate_message_post(s, actor, req),
            "run.create" => self.mutate_run_create(s, actor, req),
            "run.revoke" => self.mutate_run_revoke(s, actor, req),
            "reference.create" => self.mutate_reference_create(s, actor, req),
            "blob.begin" => self.mutate_blob_begin(s, actor, req),
            "blob.chunk" => self.mutate_blob_chunk(s, actor, req),
            "blob.finish" => self.mutate_blob_finish(s, actor, req),
            _ => bail!("unsupported: operation is not supported by this broker"),
        }
    }
    fn mutate_channel_read(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let channel = text(p, "channel_id")?;
        self.channel(s, who, channel, false)?;
        let token = text(p, "sequence")?;
        let sequence = self.resolve_cursor(s, actor, channel, Some(&json!(token)), 0)?;
        let entry = s
            .read_positions
            .entry(format!("{who}:{channel}"))
            .or_default();
        *entry = (*entry).max(sequence);
        Ok(json!({"channel_id":channel,"sequence":*entry}))
    }
    fn mutate_profile_update(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let own = s
            .principals
            .get(who)
            .ok_or_else(|| anyhow!("unauthorized"))?
            .username
            .clone();
        // `nickname: null` resets the display name to the username (D2); a string must pass
        // the display-name rules, including not being another person's username.
        let nickname = match p.get("nickname") {
            Some(Value::Null) => own.clone(),
            Some(Value::String(nickname)) => validate_display_name_for(
                nickname,
                &own,
                s.principals
                    .values()
                    .filter(|other| other.id != *who)
                    .map(|other| other.username.as_str()),
            )
            .map_err(|error| anyhow!(error.wire()))?,
            _ => bail!("invalid_params: nickname must be a string or null"),
        };
        let avatar = p.get("avatar").and_then(Value::as_str);
        ensure!(
            avatar.is_none_or(|a| a.chars().count() <= 12 && !a.chars().any(char::is_control)),
            "invalid_params: avatar must be up to 12 printable characters"
        );
        let profile = s
            .principals
            .get_mut(who)
            .ok_or_else(|| anyhow!("unauthorized"))?;
        profile.nickname = nickname;
        profile.avatar = avatar.map(str::to_owned);
        Ok(json!(profile))
    }
    fn mutate_enrollment_invite(
        &self,
        s: &mut State,
        actor: &Actor,
        req: &Request,
    ) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        self.manager(s, who)?;
        let uid: u32 = number(p, "uid")?.try_into()?;
        let current_username = self.directory.by_uid(uid)?.name;
        let existing_principal_id = match p.get("existing_principal_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) => Some(value.clone()),
            _ => bail!("invalid_params: existing_principal_id must be a string"),
        };
        match s
            .principals
            .values()
            .find(|principal| principal.uid == uid && principal.active)
        {
            Some(principal) => {
                ensure!(principal.username == current_username, "identity_mismatch: UID account name changed; offboard the old principal before enrollment");
                ensure!(existing_principal_id.as_deref() == Some(principal.id.as_str()), "invalid_params: adding a device requires the active existing_principal_id; offboard first if this is a replacement account");
            }
            None => {
                ensure!(
                    existing_principal_id.is_none(),
                    "invalid_params: no active principal exists for this UID"
                );
                Self::ensure_username_free(s, uid, &current_username)?;
            }
        }
        let key = text(p, "public_key")?;
        let bytes: [u8; 32] = hex::decode(key)?
            .try_into()
            .map_err(|_| anyhow!("invalid_params: Ed25519 key"))?;
        VerifyingKey::from_bytes(&bytes)?;
        Self::ensure_new_device(s, &digest(&bytes))?;
        let invitation = token();
        s.enrollments.insert(
            digest(invitation.as_bytes()),
            Enrollment {
                existing_principal_id,
                uid,
                public_key: key.into(),
                expires_at: now() + 3600,
            },
        );
        Ok(
            json!({"invitation":invitation,"expires_at":now()+3600,"uid":uid,"device_id":digest(&bytes)}),
        )
    }
    fn mutate_enrollment_revoke(
        &self,
        s: &mut State,
        actor: &Actor,
        req: &Request,
    ) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        self.manager(s, who)?;
        let target = text(p, "principal_id")?;
        ensure!(target != who, "forbidden: cannot revoke workspace host");
        ensure!(
            s.principals.contains_key(target),
            "forbidden: principal unavailable"
        );
        check_expected_username(s, p, target)?;
        let principal = s.principals.get_mut(target).expect("checked principal");
        principal.active = false;
        #[cfg(feature = "join-by-name")]
        {
            let (uid, username) = (principal.uid, principal.username.clone());
            join::on_principal_revoked(s, uid, &username);
        }
        s.devices.retain(|_, d| d.principal_id != target);
        s.enrollments.retain(|_, e| {
            s.principals
                .values()
                .all(|p| p.id != target || p.uid != e.uid)
        });
        s.workspace.policy_epoch += 1;
        Ok(json!({"revoked":true}))
    }
    fn mutate_policy_set(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        self.manager(s, who)?;
        ensure!(
            actor.run.is_none(),
            "forbidden: human host policy decision required"
        );
        if let Some(value) = p.get("institution_id") {
            let requested: Option<String> =
                serde_json::from_value(value.clone()).map_err(|_| {
                    anyhow!("invalid_params: institution_id must be a canonical string or null")
                })?;
            if let Some(institution) = &requested {
                ensure!(is_canonical_institution_id(institution), "invalid_params: institution_id must be 1..64 lowercase ASCII letters, digits, underscores or hyphens, starting with a letter or digit");
            }
            ensure!(s.workspace.institution_id.is_none() || s.workspace.institution_id == requested, "privacy_denied: workspace institution cannot be cleared or changed; use a new workspace");
            s.workspace.institution_id = requested;
        }
        s.workspace.mode = mode(p, "mode")?;
        s.workspace.policy_epoch += 1;
        for run in s.runs.values_mut() {
            run.revoked = true;
        }
        Ok(json!(s.workspace))
    }
    fn mutate_team_create(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        ensure!(s.teams.len() < 100, "quota_exceeded: maximum teams");
        let name = Self::team_name_for(s, p, None)?;
        let team_id = id();
        let channel_id = id();
        let members = BTreeSet::from([who.clone()]);
        let team = Team {
            id: team_id.clone(),
            name,
            created_by: who.clone(),
            members: members.clone(),
            general_channel_id: channel_id.clone(),
        };
        let channel = Channel {
            id: channel_id.clone(),
            team_id: team_id.clone(),
            name: names::RESERVED_CHANNEL_NAME.into(),
            created_by: who.clone(),
            owner_id: who.clone(),
            members,
            archived: false,
            classification: if s.workspace.mode == Mode::Private {
                Classification::Restricted
            } else {
                Classification::PublicSafe
            },
            pending_owner: None,
        };
        s.teams.insert(team_id, team.clone());
        s.channels.insert(channel_id, channel.clone());
        Ok(json!({"team":team,"channel":channel}))
    }
    fn mutate_channel_create(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let team_id = text(p, "team_id")?;
        ensure!(
            s.teams
                .get(team_id)
                .is_some_and(|t| t.members.contains(who)),
            "forbidden: team unavailable"
        );
        ensure!(s.channels.len() < 1000, "quota_exceeded: maximum channels");
        let name = Self::channel_name_for(s, p, team_id, None)?;
        let classification = match p.get("classification") {
            Some(v) => serde_json::from_value(v.clone())?,
            None => Classification::Restricted,
        };
        let c = Channel {
            id: id(),
            team_id: team_id.into(),
            name,
            created_by: who.clone(),
            owner_id: who.clone(),
            members: BTreeSet::from([who.clone()]),
            archived: false,
            classification,
            pending_owner: None,
        };
        s.channels.insert(c.id.clone(), c.clone());
        Ok(json!(c))
    }
    /// The validated, cleaned team name in `p["name"]`, refused when it collides with any
    /// **other** team in the workspace (whatever the caller's membership, with one wording
    /// either way). `renaming` excludes the team being renamed, so changing only the case or
    /// spacing of its own name is allowed.
    fn team_name_for(s: &State, p: &Value, renaming: Option<&str>) -> Result<String> {
        let name = validate_team_name(text(p, "name")?).map_err(|error| anyhow!(error.wire()))?;
        ensure!(
            !s.teams
                .values()
                .any(|team| Some(team.id.as_str()) != renaming && names_collide(&team.name, &name)),
            TEAM_NAME_TAKEN
        );
        Ok(name)
    }
    /// The canonical channel slug for `p["name"]` in `team_id` (older clients sending
    /// `"Data Analysis"` get `data-analysis`), refused when it collides with any other channel
    /// in the team, archived and hidden ones included. `general` belongs to the channel the
    /// team was created with.
    fn channel_name_for(
        s: &State,
        p: &Value,
        team_id: &str,
        renaming: Option<&str>,
    ) -> Result<String> {
        let name =
            canonical_channel_name(text(p, "name")?).map_err(|error| anyhow!(error.wire()))?;
        let general = s
            .teams
            .get(team_id)
            .map(|team| team.general_channel_id.as_str());
        ensure!(
            name != names::RESERVED_CHANNEL_NAME || (renaming.is_some() && renaming == general),
            GENERAL_RESERVED
        );
        ensure!(
            !s.channels.values().any(|channel| {
                channel.team_id == team_id
                    && Some(channel.id.as_str()) != renaming
                    && names_collide(&channel.name, &name)
            }),
            CHANNEL_NAME_TAKEN
        );
        Ok(name)
    }
    /// `team.rename {team_id, name}`: the team's creator only. The ID, memberships and
    /// invitations are unchanged; the old name is free at once.
    fn mutate_team_rename(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let team_id = text(p, "team_id")?;
        ensure!(
            s.teams
                .get(team_id)
                .is_some_and(|team| team.created_by == *who && team.members.contains(who)),
            "forbidden: team creator required"
        );
        let name = Self::team_name_for(s, p, Some(team_id))?;
        let team = s.teams.get_mut(team_id).expect("authorized team");
        team.name = name;
        Ok(json!(team))
    }
    /// `channel.rename {channel_id, name}`: the channel's current owner only, and not while
    /// archived (an archived channel is read-only and keeps its name reserved).
    fn mutate_channel_rename(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let channel_id = text(p, "channel_id")?;
        let channel = self.channel(s, who, channel_id, true)?;
        ensure!(
            channel.owner_id == *who,
            "forbidden: current owner required"
        );
        let team_id = channel.team_id.clone();
        let name = Self::channel_name_for(s, p, &team_id, Some(channel_id))?;
        let channel = s.channels.get_mut(channel_id).expect("authorized channel");
        channel.name = name;
        Ok(json!(channel))
    }
    /// `workspace.rename {name}`: the host only. Best-effort uniqueness per host account: a
    /// name a running sibling workspace of this account already answers `hello` with is
    /// refused.
    fn mutate_workspace_rename(
        &self,
        s: &mut State,
        actor: &Actor,
        req: &Request,
    ) -> Result<Value> {
        self.manager(s, &actor.id)?;
        ensure!(
            actor.run.is_none(),
            "forbidden: human host decision required"
        );
        let name = text(&req.params, "name")?;
        validate_workspace_name(name).map_err(|error| anyhow!(error.wire()))?;
        if s.workspace.name.as_deref() != Some(name) {
            ensure!(
                !sibling_workspace_names(
                    &self.runtime_root,
                    s.workspace.host_uid,
                    s.runtime_basename.as_deref(),
                    &s.workspace.id,
                )
                .iter()
                .any(|sibling| sibling == name),
                WORKSPACE_NAME_TAKEN
            );
            s.workspace.name = Some(name.to_owned());
        }
        Ok(json!(s.workspace))
    }
    fn mutate_invitation_create(
        &self,
        s: &mut State,
        actor: &Actor,
        req: &Request,
    ) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let kind = text(p, "kind")?;
        let target = text(p, "target_id")?;
        let principal = text(p, "principal_id")?;
        ensure!(
            s.principals.get(principal).is_some_and(|p| p.active),
            "forbidden: principal unavailable"
        );
        match kind {
            "team" => ensure!(
                s.teams.get(target).is_some_and(|t| t.created_by == *who),
                "forbidden: team owner required"
            ),
            "channel" => {
                let c = self.channel(s, who, target, true)?;
                ensure!(c.owner_id == *who, "forbidden: current owner required");
                ensure!(
                    s.teams
                        .get(&c.team_id)
                        .is_some_and(|t| t.members.contains(principal)),
                    "forbidden: target must first join team"
                );
            }
            _ => bail!("invalid_params: invitation kind"),
        }
        check_expected_username(s, p, principal)?;
        let invitation = Invitation {
            id: id(),
            kind: kind.into(),
            target_id: target.into(),
            principal_id: principal.into(),
            inviter_id: who.clone(),
            expires_at: now() + 86400,
        };
        s.invitations
            .insert(invitation.id.clone(), invitation.clone());
        Ok(json!(invitation))
    }
    fn mutate_invitation_accept(
        &self,
        s: &mut State,
        actor: &Actor,
        req: &Request,
    ) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let invitation = s
            .invitations
            .get(text(p, "invitation_id")?)
            .filter(|i| i.principal_id == *who && i.expires_at >= now())
            .ok_or_else(|| anyhow!("forbidden: invitation unavailable"))?
            .clone();
        if invitation.kind == "team" {
            let team = s
                .teams
                .get_mut(&invitation.target_id)
                .ok_or_else(|| anyhow!("forbidden: team unavailable"))?;
            ensure!(
                team.created_by == invitation.inviter_id,
                "forbidden: invitation authority changed"
            );
            team.members.insert(who.clone());
            s.channels
                .get_mut(&team.general_channel_id)
                .ok_or_else(|| anyhow!("storage_corrupt"))?
                .members
                .insert(who.clone());
        } else {
            let c = s
                .channels
                .get_mut(&invitation.target_id)
                .ok_or_else(|| anyhow!("forbidden: channel unavailable"))?;
            ensure!(
                c.owner_id == invitation.inviter_id && !c.archived,
                "forbidden: invitation authority changed"
            );
            c.members.insert(who.clone());
        }
        s.invitations.remove(&invitation.id);
        s.workspace.policy_epoch += 1;
        Ok(json!({"accepted":true}))
    }
    /// `team.add_member {team_id, principal_id, expected_username, channel_ids?}` (direct add,
    /// `direct_add_v1`): the team's owner or the workspace host, from a person's own signed
    /// device, adds an admitted member straight into the team, its `#general`, and each listed
    /// channel of the team the caller owns (any of them, for the host). No acceptance step: the
    /// member consented when they joined the workspace, and this record is the owner's.
    ///
    /// Every check runs before anything changes, so one refused channel refuses the whole add.
    /// A member already in the team and every listed channel is a successful no-op that leaves
    /// the policy epoch alone; any real addition moves it, as `invitation.accept` does.
    fn mutate_team_add_member(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        ensure!(actor.run.is_none(), DIRECT_ADD_AGENT_REFUSED);
        let host = self.manager(s, who).is_ok();
        let team_id = text(p, "team_id")?;
        let team = s
            .teams
            .get(team_id)
            .filter(|team| host || team.members.contains(who))
            .ok_or_else(|| anyhow!("forbidden: team unavailable"))?;
        ensure!(
            host || team.created_by == *who,
            "forbidden: Only the team's owner or the workspace host can add people to it."
        );
        let principal_id = text(p, "principal_id")?;
        let target = self.direct_add_target(s, p, principal_id)?;
        let requested: BTreeSet<String> = match p.get("channel_ids") {
            None | Some(Value::Null) => BTreeSet::new(),
            Some(value) => serde_json::from_value(value.clone()).map_err(|_| {
                anyhow!("invalid_params: channel_ids must be a list of channel IDs")
            })?,
        };
        ensure!(
            requested.len() <= 1000,
            "invalid_params: too many channel_ids"
        );
        let general = team.general_channel_id.clone();
        for channel_id in requested.iter().filter(|id| **id != general) {
            // A channel of another team, or one the caller can't see, reads exactly as one
            // that doesn't exist: the refusal is never an oracle for channel names.
            let channel = s
                .channels
                .get(channel_id)
                .filter(|c| c.team_id == team_id && (host || c.members.contains(who)))
                .ok_or_else(|| anyhow!(DIRECT_ADD_UNKNOWN_CHANNEL))?;
            ensure!(
                host || channel.owner_id == *who,
                "forbidden: You can only add people to channels you own. Uncheck #{} and try again.",
                channel.name
            );
            ensure!(
                !channel.archived,
                "channel_archived: #{} is archived, so no one can be added to it.",
                channel.name
            );
        }
        let already_member = team.members.contains(&target);
        let mut added_channels = Vec::new();
        s.teams
            .get_mut(team_id)
            .expect("authorized team")
            .members
            .insert(target.clone());
        for channel_id in
            std::iter::once(&general).chain(requested.iter().filter(|id| **id != general))
        {
            let channel = s
                .channels
                .get_mut(channel_id)
                .ok_or_else(|| anyhow!("storage_corrupt"))?;
            if channel.members.insert(target.clone()) {
                added_channels.push(channel_id.clone());
            }
        }
        if !already_member || !added_channels.is_empty() {
            s.invitations.retain(|_, invitation| {
                invitation.principal_id != target
                    || (invitation.target_id != team_id
                        && !added_channels.contains(&invitation.target_id))
            });
            s.workspace.policy_epoch += 1;
        }
        Ok(json!({
            "team_id": team_id,
            "principal_id": target,
            "added_channels": added_channels,
            "already_member": already_member,
        }))
    }
    /// `channel.add_member {channel_id, principal_id, expected_username}` (direct add): the
    /// channel's owner or the workspace host adds a member of the channel's team. As
    /// [`Self::mutate_team_add_member`], an existing member is a no-op that leaves the epoch
    /// alone.
    fn mutate_channel_add_member(
        &self,
        s: &mut State,
        actor: &Actor,
        req: &Request,
    ) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        ensure!(actor.run.is_none(), DIRECT_ADD_AGENT_REFUSED);
        let host = self.manager(s, who).is_ok();
        let channel_id = text(p, "channel_id")?;
        let channel = if host {
            s.channels
                .get(channel_id)
                .ok_or_else(|| anyhow!("forbidden: channel unavailable"))?
        } else {
            self.channel(s, who, channel_id, false)?
        };
        ensure!(
            host || channel.owner_id == *who,
            "forbidden: Only the channel's owner or the workspace host can add people to it."
        );
        ensure!(
            !channel.archived,
            "channel_archived: #{} is archived, so no one can be added to it.",
            channel.name
        );
        let principal_id = text(p, "principal_id")?;
        let target = self.direct_add_target(s, p, principal_id)?;
        let username = s
            .principals
            .get(&target)
            .map(|principal| principal.username.clone())
            .unwrap_or_default();
        ensure!(
            s.teams
                .get(&channel.team_id)
                .is_some_and(|team| team.members.contains(&target)),
            "forbidden: @{username} isn't in this channel's team yet. Add them to the team first."
        );
        let channel = s.channels.get_mut(channel_id).expect("authorized channel");
        let already_member = !channel.members.insert(target.clone());
        if !already_member {
            s.invitations.retain(|_, invitation| {
                invitation.principal_id != target || invitation.target_id != channel_id
            });
            s.workspace.policy_epoch += 1;
        }
        Ok(json!({
            "channel_id": channel_id,
            "principal_id": target,
            "already_member": already_member,
        }))
    }
    /// The principal a direct add names, once it is shown to be an admitted person: active
    /// (never a pending join or a former member), still holding exactly the `expected_username`
    /// the caller confirmed (required here, unlike the optional check elsewhere), and still the
    /// account of that name on this server (a renamed or recycled UID is refused).
    fn direct_add_target(&self, s: &State, p: &Value, principal_id: &str) -> Result<String> {
        let expected = text(p, "expected_username")?;
        let expected = expected.strip_prefix('@').unwrap_or(expected);
        let target = s
            .principals
            .get(principal_id)
            .filter(|principal| principal.active)
            .ok_or_else(|| {
                anyhow!("forbidden: @{expected} isn't a member of this workspace. Invite them to the workspace first.")
            })?;
        ensure!(target.username == expected, TARGET_MISMATCH);
        ensure!(
            self.directory
                .by_uid(target.uid)
                .is_ok_and(|account| account.name == target.username),
            "target_mismatch: @{}'s account on this server changed since they joined, so they can't be added. The host can remove @{} and invite them again.",
            target.username,
            target.username
        );
        Ok(target.id.clone())
    }
    fn mutate_channel_archive(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let channel = text(p, "channel_id")?;
        let c = self.channel(s, who, channel, true)?;
        ensure!(c.owner_id == *who, "forbidden: current owner required");
        match req.method.as_str() {
            "channel.transfer" => {
                let successor = text(p, "successor_id")?;
                // The successor must still be an active principal: an offboarded member's ID
                // in a stale snapshot is refused, as invitation.create refuses it (FR18).
                ensure!(
                    successor != who
                        && c.members.contains(successor)
                        && s.principals.get(successor).is_some_and(|p| p.active),
                    "forbidden: eligible successor required"
                );
                check_expected_username(s, p, successor)?;
            }
            "membership.revoke" => {
                let target = text(p, "principal_id")?;
                ensure!(target != who, "forbidden: transfer before owner removal");
                check_expected_username(s, p, target)?;
            }
            _ => {}
        }
        let c = s.channels.get_mut(channel).expect("authorized channel");
        match req.method.as_str() {
            "channel.archive" => c.archived = true,
            "channel.transfer" => {
                c.pending_owner = Some(text(p, "successor_id")?.into());
            }
            _ => {
                let target = text(p, "principal_id")?;
                c.members.remove(target);
                if c.pending_owner.as_deref() == Some(target) {
                    c.pending_owner = None;
                }
            }
        }
        s.invitations.retain(|_, i| i.target_id != channel);
        s.workspace.policy_epoch += 1;
        Ok(json!(c))
    }
    fn mutate_transfer_accept(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let channel = text(p, "channel_id")?;
        self.channel(s, who, channel, true)?;
        let c = s.channels.get_mut(channel).expect("authorized channel");
        ensure!(
            c.pending_owner.as_deref() == Some(who),
            "forbidden: no transfer offered to this principal"
        );
        let previous_owner = c.owner_id.clone();
        c.members.remove(&previous_owner);
        c.owner_id = who.clone();
        c.pending_owner = None;
        s.invitations.retain(|_, i| i.target_id != channel);
        s.workspace.policy_epoch += 1;
        Ok(json!(c))
    }
    fn message_restricted(
        &self,
        s: &State,
        actor: &Actor,
        p: &Value,
        c: &Channel,
        sources: &BTreeSet<String>,
    ) -> bool {
        let mut restricted = actor
            .run
            .as_ref()
            .map(|r| r.personal_mode == Mode::Private)
            .unwrap_or_else(|| p.get("personal_mode").and_then(Value::as_str) != Some("public"))
            || s.workspace.mode == Mode::Private
            || c.classification == Classification::Restricted;
        restricted |= s
            .messages
            .iter()
            .any(|m| sources.contains(&m.channel_id) && m.restricted);
        if let Some(run) = &actor.run {
            restricted |= run.personal_mode == Mode::Private || run.remote_root.is_some();
        }
        restricted
    }
    fn mutate_message_post(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        ensure!(
            req.method != "run.project" || actor.run.is_some(),
            "forbidden: run projection requires an admitted worker grant"
        );
        ensure!(s.messages.len() < 100_000, "quota_exceeded: message limit");
        let body = p
            .get("body")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid_params: body must be a string"))?;
        ensure!(body.len() <= 65536, "invalid_params: message too long");
        let channel = if let Some(run) = &actor.run {
            run.channel_id.as_str()
        } else {
            text(p, "channel_id")?
        };
        let c = self.channel(s, who, channel, true)?;
        let sources = actor
            .run
            .as_ref()
            .map(|r| r.source_channels.clone())
            .unwrap_or_else(|| BTreeSet::from([channel.into()]));
        let mut restricted = self.message_restricted(s, actor, p, c, &sources);
        let attachments: Vec<String> = strings(p, "attachments")?.into_iter().collect();
        for blob_id in &attachments {
            let b = s
                .blobs
                .get(blob_id)
                .filter(|b| b.complete)
                .ok_or_else(|| anyhow!("forbidden: attachment unavailable"))?;
            self.blob_authorized(s, actor, b)?;
            ensure!(
                b.channel_id == channel && b.source_channels.is_subset(&sources),
                "forbidden: attachment provenance cannot be dropped"
            );
            restricted |= b.restricted;
        }
        if let Some(run) = &actor.run {
            ensure!(
                !run.public_provider || !restricted,
                "privacy_denied: source policy changed"
            );
        }
        let status = projection_status(p)?;
        let references: Vec<String> = strings(p, "references")?.into_iter().collect();
        for reference_id in &references {
            let reference = s
                .references
                .get(reference_id)
                .ok_or_else(|| anyhow!("forbidden: reference unavailable"))?;
            self.reference_authorized(s, actor, reference)?;
            ensure!(
                reference.channel_id == channel && reference.source_channels.is_subset(&sources),
                "forbidden: reference provenance cannot be dropped"
            );
            restricted = true;
        }
        if let Some(run) = &actor.run {
            ensure!(
                !run.public_provider || !restricted,
                "privacy_denied: reference is restricted"
            );
        }
        ensure!(
            !body.is_empty() || (actor.run.is_none() && (!attachments.is_empty() || !references.is_empty())),
            "invalid_params: body must be nonempty unless a human message includes attachments or references"
        );
        let message = Message {
            references,

            id: id(),
            sequence: s.sequence + 1,
            channel_id: channel.into(),
            actor_id: who.clone(),
            run_id: actor.run.as_ref().map(|r| r.id.clone()),
            body: body.into(),
            created_at: now(),
            restricted,
            source_channels: sources,
            attachments,
            status: status.clone(),
        };
        s.messages.push(message.clone());
        if status.as_deref().is_some_and(|v| v != "progress") {
            if let Some(run) = &actor.run {
                s.runs.get_mut(&run.id).expect("admitted run").revoked = true;
            }
        }
        Ok(json!(message))
    }
    fn run_policy_consent(s: &State, p: &Value) -> Result<RunPolicyConsent> {
        let public = p
            .get("public_provider")
            .and_then(Value::as_bool)
            .ok_or_else(|| anyhow!("invalid_params: public_provider required"))?;
        let personal = mode(p, "personal_mode")?;
        let expected_protected_context = p
            .get("expected_protected_context")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                anyhow!("invalid_params: expected_protected_context boolean required")
            })?;
        ensure!(
            number(p, "expected_workspace_policy_epoch")? == s.workspace.policy_epoch,
            "stale_policy: workspace policy changed; refresh and authorize again"
        );
        let institution_field = |name: &str| -> Result<Option<String>> {
            serde_json::from_value(
                p.get(name)
                    .cloned()
                    .ok_or_else(|| anyhow!("invalid_params: missing {name}"))?,
            )
            .map_err(|_| anyhow!("invalid_params: {name} must be a canonical string or null"))
        };
        let workspace_institution_id = institution_field("workspace_institution_id")?;
        let connection_institution_id = institution_field("connection_institution_id")?;
        ensure!(
            workspace_institution_id == s.workspace.institution_id,
            "stale_policy: workspace institution changed; refresh and authorize again"
        );
        let provider_affiliation: ProviderAffiliation = serde_json::from_value(
            p.get("provider_affiliation")
                .cloned()
                .ok_or_else(|| anyhow!("invalid_params: provider_affiliation required"))?,
        )
        .map_err(|_| anyhow!("invalid_params: invalid provider_affiliation"))?;
        Ok(RunPolicyConsent {
            public_provider: public,
            personal_mode: personal,
            expected_protected_context,
            workspace_institution_id,
            connection_institution_id,
            provider_affiliation,
        })
    }
    fn mutate_run_create(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let channel = text(p, "channel_id")?;
        self.channel(s, who, channel, true)?;
        let mut sources = strings(p, "source_channels")?;
        sources.insert(channel.into());
        ensure!(
            sources.len() <= 20,
            "quota_exceeded: too many context channels"
        );
        let consent = Self::run_policy_consent(s, p)?;
        if consent.public_provider {
            ensure!(
                consent.personal_mode == Mode::Public && s.workspace.mode == Mode::Public,
                "privacy_denied: Private workspace or connection"
            );
        }
        for source in &sources {
            let c = self.channel(s, who, source, false)?;
            if consent.public_provider {
                ensure!(
                    c.classification == Classification::PublicSafe
                        && !s
                            .messages
                            .iter()
                            .any(|m| m.channel_id == *source && m.restricted),
                    "privacy_denied: retained context is restricted"
                );
            }
        }
        let expires = number(p, "expires_in")?;
        ensure!(
            (1..=3600).contains(&expires),
            "invalid_params: expiry must be 1..3600 seconds"
        );
        let remote_root = p
            .get("remote_root")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let remote_execution = p
            .get("remote_execution")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(root) = &remote_root {
            ensure!(
                !consent.public_provider && Path::new(root).is_absolute() && root != "/",
                "privacy_denied: remote root must be explicitly scoped and private-provider only"
            );
            self.protect_authority_root(
                root,
                s.principals
                    .get(who)
                    .ok_or_else(|| anyhow!("unauthorized"))?
                    .uid,
            )?;
        }
        ensure!(
            !remote_execution || remote_root.is_some(),
            "invalid_params: execution requires remote root"
        );
        let protected_context =
            Self::is_protected_context(s, &sources, &consent.personal_mode, remote_root.as_deref());
        ensure!(
            consent.expected_protected_context == protected_context,
            "stale_policy: selected context protection changed; refresh and authorize again"
        );
        Self::enforce_institution_policy(
            s,
            &sources,
            &consent.personal_mode,
            remote_root.as_deref(),
            &consent.provider_affiliation,
            consent.connection_institution_id.as_deref(),
        )?;
        let run = Run {
            protected_context,
            provider_affiliation: consent.provider_affiliation,
            workspace_institution_id: consent.workspace_institution_id,
            connection_institution_id: consent.connection_institution_id,
            remote_root,
            remote_execution,
            id: id(),
            owner_id: who.clone(),
            channel_id: channel.into(),
            source_channels: sources,
            provider_policy_id: text(p, "provider_policy_id")?.into(),
            public_provider: consent.public_provider,
            personal_mode: consent.personal_mode,
            policy_epoch: s.workspace.policy_epoch,
            expires_at: now() + expires,
            revoked: false,
        };
        let credential = token();
        s.grants
            .insert(digest(credential.as_bytes()), run.id.clone());
        s.runs.insert(run.id.clone(), run.clone());
        Ok(json!({"run":run,"credential":credential}))
    }
    fn mutate_run_revoke(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let run = s
            .runs
            .get_mut(text(p, "run_id")?)
            .filter(|r| r.owner_id == *who)
            .ok_or_else(|| anyhow!("forbidden: owned run unavailable"))?;
        run.revoked = true;
        Ok(json!(run))
    }
    fn mutate_reference_create(
        &self,
        s: &mut State,
        actor: &Actor,
        req: &Request,
    ) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let channel = text(p, "channel_id")?;
        self.channel(s, who, channel, true)?;
        let path = text(p, "path")?;
        ensure!(
            path.len() <= 4096
                && Path::new(path).is_absolute()
                && !Path::new(path)
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
            "invalid_params: absolute remote path without parent traversal required"
        );
        if let Some(run) = &actor.run {
            ensure!(
                run.channel_id == channel && !run.public_provider,
                "forbidden: reference outside run destination"
            );
            let root = run
                .remote_root
                .as_ref()
                .ok_or_else(|| anyhow!("forbidden: run has no remote path approval"))?;
            ensure!(
                Path::new(path).starts_with(root),
                "forbidden: reference outside approved remote root"
            );
        }
        ensure!(
            s.references.len() < 10000,
            "quota_exceeded: remote reference limit"
        );
        let label = text(p, "label")?;
        ensure!(
            label.len() <= 255 && !label.chars().any(char::is_control),
            "invalid_params: reference label"
        );
        let reference = RemoteReference {
            id: id(),
            channel_id: channel.into(),
            owner_id: who.clone(),
            path: path.into(),
            label: label.into(),
            restricted: true,
            source_channels: actor
                .run
                .as_ref()
                .map(|r| r.source_channels.clone())
                .unwrap_or_else(|| BTreeSet::from([channel.into()])),
            verified: false,
        };
        s.references.insert(reference.id.clone(), reference.clone());
        Ok(json!(reference))
    }
    fn mutate_blob_begin(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let channel = text(p, "channel_id")?;
        if let Some(run) = &actor.run {
            ensure!(
                run.channel_id == channel,
                "forbidden: attachment destination outside run grant"
            );
        }
        let c = self.channel(s, who, channel, true)?;
        let size = number(p, "size")?;
        ensure!(
            size <= 1024 * 1024 * 1024,
            "quota_exceeded: maximum attachment is 1 GiB"
        );
        ensure!(
            s.blobs.len() < 10000
                && s.blobs.values().map(|b| b.size).sum::<u64>() + size <= 10 * 1024 * 1024 * 1024,
            "quota_exceeded: workspace attachment quota"
        );
        let sha = text(p, "sha256")?;
        ensure!(
            sha.len() == 64 && hex::decode(sha)?.len() == 32,
            "invalid_params: sha256"
        );
        let name = text(p, "name")?;
        ensure!(
            name.len() <= 255 && !name.chars().any(char::is_control),
            "invalid_params: attachment display name"
        );
        let blob = Blob {
            run_id: actor.run.as_ref().map(|r| r.id.clone()),
            id: id(),
            owner_id: who.clone(),
            channel_id: channel.into(),
            name: name.into(),
            media_type: text(p, "media_type")?.into(),
            size,
            sha256: sha.to_lowercase(),
            offset: 0,
            complete: false,
            restricted: actor.run.as_ref().is_some_and(|r| {
                r.remote_root.is_some()
                    || r.personal_mode == Mode::Private
                    || s.messages
                        .iter()
                        .any(|m| r.source_channels.contains(&m.channel_id) && m.restricted)
            }) || actor
                .run
                .as_ref()
                .map(|r| r.personal_mode == Mode::Private)
                .unwrap_or_else(|| {
                    p.get("personal_mode").and_then(Value::as_str) != Some("public")
                })
                || s.workspace.mode == Mode::Private
                || c.classification == Classification::Restricted,
            source_channels: actor
                .run
                .as_ref()
                .map(|r| r.source_channels.clone())
                .unwrap_or_else(|| BTreeSet::from([channel.into()])),
        };
        let file = private_file(&self.root.join("blobs").join(&blob.id), false)?;
        file.sync_all()?;
        sync_dir(&self.root.join("blobs"))?;
        s.blobs.insert(blob.id.clone(), blob.clone());
        Ok(json!(blob))
    }
    fn mutate_blob_chunk(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let blob_id = text(p, "blob_id")?;
        let b = s
            .blobs
            .get(blob_id)
            .ok_or_else(|| anyhow!("forbidden: attachment unavailable"))?;
        self.blob_authorized(s, actor, b)?;
        self.channel(s, who, &b.channel_id, true)?;
        if let Some(run) = &actor.run {
            ensure!(
                b.run_id.as_deref() == Some(run.id.as_str()),
                "forbidden: upload belongs to another authority scope"
            );
        }
        ensure!(
            b.owner_id == *who && !b.complete,
            "forbidden: upload unavailable"
        );
        let offset = number(p, "offset")?;
        let bytes = hex::decode(text(p, "data_hex")?)?;
        ensure!(
            !bytes.is_empty()
                && bytes.len() <= MAX_CHUNK
                && offset == b.offset
                && offset + bytes.len() as u64 <= b.size,
            "conflict: chunk size/offset invalid"
        );
        let mut file = private_file(&self.root.join("blobs").join(blob_id), false)?;
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        let blob = s.blobs.get_mut(blob_id).expect("authorized blob");
        blob.offset += bytes.len() as u64;
        Ok(json!(blob))
    }
    fn mutate_blob_finish(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let blob_id = text(p, "blob_id")?;
        let b = s
            .blobs
            .get(blob_id)
            .ok_or_else(|| anyhow!("forbidden: attachment unavailable"))?;
        self.blob_authorized(s, actor, b)?;
        self.channel(s, who, &b.channel_id, true)?;
        if let Some(run) = &actor.run {
            ensure!(
                b.run_id.as_deref() == Some(run.id.as_str()),
                "forbidden: upload belongs to another authority scope"
            );
        }
        ensure!(
            b.owner_id == *who && !b.complete && b.offset == b.size,
            "conflict: attachment incomplete"
        );
        let mut file = private_file(&self.root.join("blobs").join(blob_id), false)?;
        file.set_len(b.size)?;
        let mut hash = Sha256::new();
        let mut buffer = vec![0; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        ensure!(
            hex::encode(hash.finalize()) == b.sha256,
            "digest_mismatch: attachment content differs from declaration"
        );
        file.sync_all()?;
        let blob = s.blobs.get_mut(blob_id).expect("authorized blob");
        blob.complete = true;
        Ok(json!(blob))
    }
    fn manager(&self, s: &State, actor: &str) -> Result<()> {
        ensure!(
            s.principals
                .get(actor)
                .is_some_and(|p| p.uid == s.workspace.host_uid && p.active),
            "forbidden: workspace host device required"
        );
        Ok(())
    }
    /// Collision refusals `actor` received within the window ending at `now`.
    fn recent_name_refusals(&self, actor: &str, now: u64) -> usize {
        self.name_refusals.get(actor).map_or(0, |times| {
            times
                .iter()
                .filter(|at| now.saturating_sub(**at) < NAME_REFUSAL_WINDOW_SECS)
                .count()
        })
    }
    fn record_name_refusal(&mut self, actor: &str, now: u64) {
        let times = self.name_refusals.entry(actor.to_owned()).or_default();
        times.retain(|at| now.saturating_sub(*at) < NAME_REFUSAL_WINDOW_SECS);
        times.push_back(now);
        while times.len() > NAME_REFUSAL_LIMIT {
            times.pop_front();
        }
    }
}

/// The host's active principal, injected into the snapshot's `workspace` at projection time
/// and never stored (it would be journaled otherwise).
fn host_principal_id(s: &State) -> Option<&str> {
    s.principals
        .values()
        .find(|p| p.active && p.uid == s.workspace.host_uid)
        .map(|p| p.id.as_str())
}

/// If `params` carries `expected_username` (the `@username` the person confirmed, inside
/// their signature), the target principal must still have exactly that username. Closes the
/// window between a (possibly tampered) snapshot and the mutation that acts on it.
fn check_expected_username(s: &State, params: &Value, principal_id: &str) -> Result<()> {
    let expected = match params.get("expected_username") {
        None | Some(Value::Null) => return Ok(()),
        Some(Value::String(expected)) => expected,
        Some(_) => bail!("invalid_params: expected_username must be a string"),
    };
    let expected = expected.strip_prefix('@').unwrap_or(expected);
    ensure!(
        s.principals
            .get(principal_id)
            .is_some_and(|target| target.username == expected),
        TARGET_MISMATCH
    );
    Ok(())
}

/// Display names for one projection. The username keys of every principal (active or
/// former) are computed once, so projecting N people costs N key computations rather than N².
struct PeopleIndex<'a> {
    keys: BTreeMap<String, BTreeSet<&'a str>>,
    skeletons: BTreeMap<String, BTreeSet<&'a str>>,
}

impl<'a> PeopleIndex<'a> {
    fn new(s: &'a State) -> Self {
        let mut index = Self {
            keys: BTreeMap::new(),
            skeletons: BTreeMap::new(),
        };
        for principal in s.principals.values() {
            let username = principal.username.as_str();
            index
                .keys
                .entry(name_key(username))
                .or_default()
                .insert(username);
            index
                .skeletons
                .entry(skeleton_key(username))
                .or_default()
                .insert(username);
        }
        index
    }
    /// [`sanitize_display_name`] against every other principal's username: the stored
    /// nickname with the characters the validator refuses stripped, or the username when
    /// nothing valid is left or the result reads as someone else's username.
    fn display_name(&self, principal: &Principal) -> String {
        let own = principal.username.as_str();
        let name = sanitize_display_name(&principal.nickname, own, std::iter::empty::<&str>());
        let key = name_key(&name);
        if key == name_key(own) {
            return name;
        }
        let other = |set: Option<&BTreeSet<&str>>| {
            set.is_some_and(|usernames| usernames.iter().any(|username| *username != own))
        };
        if other(self.keys.get(&key)) || other(self.skeletons.get(&skeleton_key(&name))) {
            own.to_owned()
        } else {
            name
        }
    }
    /// A principal as the snapshot lists it: every stored field plus `display_name`.
    fn principal_wire(&self, principal: &Principal) -> Value {
        let mut wire = json!(principal);
        wire["display_name"] = json!(self.display_name(principal));
        wire
    }
    /// A message author in a result's `people` map.
    fn person_wire(&self, principal: &Principal) -> Value {
        json!({
            "username": principal.username,
            "display_name": self.display_name(principal),
            "active": principal.active,
        })
    }
}

#[cfg(target_os = "linux")]
fn peer_uid(stream: &UnixStream) -> Result<u32> {
    let mut cred = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    ensure!(
        unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                cred.as_mut_ptr().cast(),
                &mut len,
            )
        } == 0
            && len as usize == std::mem::size_of::<libc::ucred>(),
        "unsupported: kernel peer credentials unavailable"
    );
    Ok(unsafe { cred.assume_init() }.uid)
}
#[cfg(not(target_os = "linux"))]
fn peer_uid(_stream: &UnixStream) -> Result<u32> {
    bail!("unsupported: Crew broker and bridge require Linux SO_PEERCRED")
}
fn read_frame(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let n = reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut bytes)?;
    if n == 0 {
        return Ok(None);
    }
    ensure!(
        n <= MAX_FRAME && bytes.ends_with(b"\n"),
        "invalid_frame: bounded newline-terminated JSON required"
    );
    Ok(Some(bytes))
}
fn validate_socket(path: &Path, owner: u32) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "unsafe_socket: absolute socket path required"
    );
    let parent = path.parent().ok_or_else(|| anyhow!("unsafe_socket"))?;
    ensure!(
        parent.parent() == Some(Path::new("/tmp"))
            || parent.parent() == Some(Path::new("/private/tmp")),
        "unsafe_socket: runtime must be dedicated node-local temporary directory"
    );
    let dir = fs::symlink_metadata(parent)?;
    let socket = fs::symlink_metadata(path)?;
    use std::os::unix::fs::FileTypeExt;
    ensure!(
        dir.is_dir()
            && dir.uid() == owner
            && dir.mode() & 0o022 == 0
            && socket.file_type().is_socket()
            && socket.uid() == owner,
        "unsafe_socket: ownership or runtime permissions invalid"
    );
    Ok(())
}
/// The workspace names running sibling brokers of `uid` answer `hello` with: every
/// `crew-<uid>-<32 hex>/broker.sock` under `runtime_root` (at most [`SIBLING_PROBE_LIMIT`])
/// whose directory and socket `uid` owns and whose listener runs as `uid`, except `own_basename`
/// and a broker answering with `own_workspace_id`. Best effort: a stale, slow or unexpected
/// socket is skipped, each probe is bounded by [`SIBLING_PROBE_TIMEOUT`], and nothing is
/// trusted beyond the name used to refuse a duplicate. Names are what any node user can
/// already learn from `hello`.
fn sibling_workspace_names(
    runtime_root: &Path,
    uid: u32,
    own_basename: Option<&str>,
    own_workspace_id: &str,
) -> Vec<String> {
    let prefix = format!("crew-{uid}-");
    let Ok(entries) = fs::read_dir(runtime_root) else {
        return Vec::new();
    };
    let mut candidates: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| {
            name.strip_prefix(&prefix).is_some_and(|suffix| {
                suffix.len() == 32
                    && suffix
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            }) && Some(name.as_str()) != own_basename
        })
        .collect();
    candidates.sort();
    candidates.truncate(SIBLING_PROBE_LIMIT);
    candidates
        .into_iter()
        .filter_map(|basename| {
            let directory = runtime_root.join(&basename);
            let socket = directory.join("broker.sock");
            let (hello_workspace, name) = probe_hello(&directory, &socket, uid).ok()?;
            (hello_workspace != own_workspace_id).then_some(name?)
        })
        .collect()
}
/// `hello` on one sibling socket: `(workspace_id, name)`.
fn probe_hello(directory: &Path, socket: &Path, uid: u32) -> Result<(String, Option<String>)> {
    use std::os::unix::fs::FileTypeExt;
    let dir = fs::symlink_metadata(directory)?;
    let file = fs::symlink_metadata(socket)?;
    ensure!(
        dir.is_dir() && dir.uid() == uid && file.file_type().is_socket() && file.uid() == uid,
        "unsafe_runtime: not this account's runtime"
    );
    let mut stream = UnixStream::connect(socket)?;
    ensure!(
        peer_uid(&stream)? == uid,
        "identity_mismatch: sibling broker UID"
    );
    stream.set_read_timeout(Some(SIBLING_PROBE_TIMEOUT))?;
    stream.set_write_timeout(Some(SIBLING_PROBE_TIMEOUT))?;
    stream
        .write_all(b"{\"version\":1,\"id\":\"name-probe\",\"method\":\"hello\",\"params\":{}}\n")?;
    let response: Response = serde_json::from_slice(
        &read_frame(&mut BufReader::new(stream))?.ok_or_else(|| anyhow!("unavailable"))?,
    )?;
    let result = response.result.ok_or_else(|| anyhow!("unavailable"))?;
    Ok((
        text(&result, "workspace_id")?.to_owned(),
        result
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned),
    ))
}
fn recorded_runtime(broker: &Broker, node_id: &str) -> Result<Option<String>> {
    let uid = broker.state.workspace.host_uid;
    let descriptor_path = broker.root.join("runtime.json");
    let recorded = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&descriptor_path)
    {
        Ok(file) => {
            let metadata = file.metadata()?;
            ensure!(
                metadata.is_file()
                    && metadata.uid() == uid
                    && metadata.mode() & 0o077 == 0
                    && metadata.nlink() == 1
                    && metadata.len() <= MAX_FRAME as u64,
                "unsafe_runtime: invalid private runtime descriptor"
            );
            let value: Value = serde_json::from_reader(file)
                .context("unsafe_runtime: runtime descriptor is corrupt")?;
            ensure!(
                value["workspace_id"].as_str() == Some(broker.state.workspace.id.as_str())
                    && value["host_uid"].as_u64() == Some(uid as u64)
                    && value["node_id"].as_str() == Some(node_id),
                "unsafe_runtime: recorded workspace, owner or node identity mismatch"
            );
            let socket = PathBuf::from(text(&value, "socket")?);
            ensure!(
                socket.file_name().and_then(|s| s.to_str()) == Some("broker.sock"),
                "unsafe_runtime: invalid socket filename"
            );
            let directory = socket
                .parent()
                .ok_or_else(|| anyhow!("unsafe_runtime: runtime directory missing"))?;
            ensure!(
                directory.parent() == Some(Path::new("/tmp")),
                "unsafe_runtime: runtime directory must be directly under /tmp"
            );
            Some(
                directory
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| anyhow!("unsafe_runtime: invalid directory name"))?
                    .to_owned(),
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("unsafe_runtime: cannot read runtime descriptor"),
    };
    Ok(recorded)
}
fn persisted_runtime(broker: &mut Broker, node_id: &str) -> Result<PathBuf> {
    let uid = broker.state.workspace.host_uid;
    let recorded = recorded_runtime(broker, node_id)?;
    let basename = match (&broker.state.runtime_basename, recorded) {
        (Some(expected), Some(recorded)) => {
            ensure!(
                *expected == recorded,
                "unsafe_runtime: runtime basename changed"
            );
            expected.clone()
        }
        (Some(expected), None) => expected.clone(),
        (None, Some(recorded)) => recorded,
        (None, None) => format!("crew-{uid}-{}", Uuid::new_v4().simple()),
    };
    let prefix = format!("crew-{uid}-");
    let suffix = basename
        .strip_prefix(&prefix)
        .ok_or_else(|| anyhow!("unsafe_runtime: runtime owner prefix mismatch"))?;
    ensure!(
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "unsafe_runtime: malformed persisted runtime basename"
    );
    if broker.state.runtime_basename.is_none() {
        let mut state = broker.state.clone();
        state.runtime_basename = Some(basename.clone());
        broker.commit(state, "system", "workspace.bind_runtime")?;
    }
    reclaim_runtime_socket(uid, &basename)
}
fn reclaim_runtime_socket(uid: u32, basename: &str) -> Result<PathBuf> {
    let directory = PathBuf::from("/tmp").join(basename);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) => ensure!(
            metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o7777 == 0o711,
            "unsafe_runtime: persisted directory ownership, type or permissions changed"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::DirBuilder::new().mode(0o700).create(&directory)?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o711))?;
        }
        Err(error) => return Err(error.into()),
    }
    for entry in fs::read_dir(&directory)? {
        ensure!(
            entry?.file_name() == "broker.sock",
            "unsafe_runtime: unexpected entry occupies the persisted runtime directory"
        );
    }
    let socket = directory.join("broker.sock");
    match fs::symlink_metadata(&socket) {
        Ok(metadata) => {
            use std::os::unix::fs::FileTypeExt;
            ensure!(
                metadata.file_type().is_socket()
                    && metadata.uid() == uid
                    && metadata.mode() & 0o7777 == 0o666,
                "unsafe_runtime: socket ownership, type or permissions changed"
            );
            match UnixStream::connect(&socket) {
                Ok(_) => bail!(
                    "runtime_in_use: persisted socket has a live listener; it will not be replaced"
                ),
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                    let current = fs::symlink_metadata(&socket)?;
                    ensure!(
                        current.dev() == metadata.dev()
                            && current.ino() == metadata.ino()
                            && current.uid() == uid
                            && current.file_type().is_socket(),
                        "unsafe_runtime: socket changed during stale-listener check"
                    );
                    fs::remove_file(&socket)?;
                }
                Err(error) => {
                    return Err(error).context(
                        "unsafe_runtime: cannot establish that the recorded socket is stale",
                    )
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(socket)
}
fn write_runtime(root: &Path, info: &Value) -> Result<()> {
    let path = root.join(format!(".runtime-{}.tmp", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    file.write_all(&serde_json::to_vec(info)?)?;
    file.sync_all()?;
    drop(file);
    fs::rename(path, root.join("runtime.json"))?;
    sync_dir(root)
}
struct ConnectionPermit {
    counts: Arc<Mutex<BTreeMap<u32, usize>>>,
    uid: u32,
}
impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        if let Ok(mut counts) = self.counts.lock() {
            if let Some(count) = counts.get_mut(&self.uid) {
                *count -= 1;
                if *count == 0 {
                    counts.remove(&self.uid);
                }
            }
        }
    }
}
/// Run the broker in the foreground. `name` names a workspace this call initializes, or an
/// existing workspace that has no name yet; a workspace already named otherwise is refused
/// (rename it with `workspace.rename`).
pub fn serve(root: &Path, bootstrap_key: &str, name: Option<&str>) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "unsupported: serve requires Linux"
    );
    let node_id = node_identity()?;
    let mut broker = Broker::open_inner(root, bootstrap_key, Box::new(SystemDirectory), name)?;
    if let Some(name) = name {
        match broker.state.workspace.name.as_deref() {
            Some(stored) if stored == name => {}
            Some(_) => bail!(
                "name_mismatch: this workspace already has another name; start it without --name, or rename it in Crew"
            ),
            None => {
                let mut state = broker.state.clone();
                state.workspace.name = Some(name.to_owned());
                broker.commit(state, "system", "workspace.name")?;
            }
        }
    }
    ensure!(broker.state.writer_node_id.as_ref().is_none_or(|expected|expected==&node_id),"node_identity_changed: workspace belongs to another writer node; automatic failover is disabled");
    if broker.state.writer_node_id.is_none() {
        let mut state = broker.state.clone();
        state.writer_node_id = Some(node_id.clone());
        broker.commit(state, "system", "workspace.bind_node")?;
    }
    let uid = unsafe { libc::geteuid() };
    ensure!(uid != 0, "unsupported: Crew must run as an ordinary user");
    let socket = persisted_runtime(&mut broker, &node_id)?;
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o666))?;
    let secret: [u8; 32] = hex::decode(&broker.state.workspace_signing_key)?
        .try_into()
        .map_err(|_| anyhow!("storage_corrupt: workspace identity"))?;
    let public_key = SigningKey::from_bytes(&secret).verifying_key().to_bytes();
    let info = json!({"pid":std::process::id(),"socket":socket,"workspace_id":broker.workspace().id,"host_uid":uid,"protocol":1,"node_id":node_id,"workspace_public_key":hex::encode(public_key),"workspace_key_fingerprint":digest(&public_key)});
    let state_root = broker.root.clone();
    let root = state_root.as_path();
    write_runtime(root, &info)?;
    println!("{}", info);
    let shared = Arc::new(Mutex::new(broker));
    let active = Arc::new(Mutex::new(BTreeMap::<u32, usize>::new()));
    for stream in listener.incoming() {
        let stream = stream?;
        let uid = peer_uid(&stream)?;
        let permit = {
            let mut counts = active
                .lock()
                .map_err(|_| anyhow!("connection limits unavailable"))?;
            if counts.values().sum::<usize>() >= 256 || counts.get(&uid).copied().unwrap_or(0) >= 8
            {
                continue;
            }
            *counts.entry(uid).or_default() += 1;
            ConnectionPermit {
                counts: Arc::clone(&active),
                uid,
            }
        };
        let shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            let _permit = permit;
            let _ = serve_client(stream, uid, shared);
        });
    }
    Ok(())
}
fn serve_client(mut stream: UnixStream, uid: u32, broker: Arc<Mutex<Broker>>) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(300)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut connection = Connection::new();
    while let Some(bytes) = read_frame(&mut reader)? {
        let request: Request = serde_json::from_slice(&bytes)?;
        let response = broker
            .lock()
            .map_err(|_| anyhow!("broker unavailable"))?
            .handle(uid, &mut connection, request);
        let mut bytes = serde_json::to_vec(&response)?;
        if bytes.len() >= MAX_FRAME {
            bytes = serde_json::to_vec(&Response {
                id: response.id,
                result: None,
                error: Some(ProtocolError {
                    code: "response_too_large".into(),
                    message: "Request a smaller history window".into(),
                }),
            })?;
        }
        bytes.push(b'\n');
        stream.write_all(&bytes)?;
        stream.flush()?;
    }
    Ok(())
}
pub fn bridge(socket: &Path, owner: u32, workspace: &str) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "unsupported: bridge requires Linux"
    );
    validate_socket(socket, owner)?;
    let mut stream = UnixStream::connect(socket)?;
    ensure!(peer_uid(&stream)? == owner, "identity_mismatch: broker UID");
    let mut reader = BufReader::new(stream.try_clone()?);
    stream
        .write_all(b"{\"version\":1,\"id\":\"bridge-pin\",\"method\":\"hello\",\"params\":{}}\n")?;
    let hello: Response = serde_json::from_slice(
        &read_frame(&mut reader)?.ok_or_else(|| anyhow!("broker disconnected"))?,
    )?;
    ensure!(
        hello
            .result
            .as_ref()
            .and_then(|v| v.get("workspace_id"))
            .and_then(Value::as_str)
            == Some(workspace),
        "identity_mismatch: pinned workspace"
    );
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    while let Some(frame) = read_frame(&mut input)? {
        let request: Request = serde_json::from_slice(&frame)?;
        if request.method.starts_with("remote.") {
            let result = (|| -> Result<Value> {
                ensure!(
                    request.auth.is_none() && request.credential.is_some(),
                    "forbidden: remote helper requires scoped worker credential"
                );
                let scope_request = Request {
                    version: request.version,
                    id: request.id.clone(),
                    method: "run.remote_scope".into(),
                    params: json!({}),
                    auth: None,
                    credential: request.credential.clone(),
                };
                let mut scope_bytes = serde_json::to_vec(&scope_request)?;
                scope_bytes.push(b'\n');
                stream.write_all(&scope_bytes)?;
                stream.flush()?;
                let scope_response: Response = serde_json::from_slice(
                    &read_frame(&mut reader)?.ok_or_else(|| anyhow!("broker disconnected"))?,
                )?;
                if let Some(error) = scope_response.error {
                    bail!("{}: {}", error.code, error.message);
                }
                let scope = scope_response
                    .result
                    .ok_or_else(|| anyhow!("invalid_response"))?;
                let result = crate::remote::handle(&request.method, &request.params, &scope)?;
                stream.write_all(&scope_bytes)?;
                stream.flush()?;
                let recheck: Response = serde_json::from_slice(
                    &read_frame(&mut reader)?.ok_or_else(|| anyhow!("broker disconnected"))?,
                )?;
                if let Some(error) = recheck.error {
                    bail!("{}: permission changed during remote operation; effect may already have occurred: {}",error.code,error.message);
                }
                ensure!(recheck.result.as_ref()==Some(&scope),"permission_changed: remote scope changed during operation; effect may already have occurred");
                Ok(result)
            })();
            let response = match result {
                Ok(value) => Response {
                    id: request.id,
                    result: Some(value),
                    error: None,
                },
                Err(error) => Response {
                    id: request.id,
                    result: None,
                    error: Some(ProtocolError {
                        code: "remote_operation_denied".into(),
                        message: error.to_string(),
                    }),
                },
            };
            let mut bytes = serde_json::to_vec(&response)?;
            ensure!(bytes.len() < MAX_FRAME, "response_too_large");
            bytes.push(b'\n');
            output.write_all(&bytes)?;
            output.flush()?;
        } else {
            stream.write_all(&frame)?;
            stream.flush()?;
            let response =
                read_frame(&mut reader)?.ok_or_else(|| anyhow!("broker disconnected"))?;
            output.write_all(&response)?;
            output.flush()?;
        }
    }
    Ok(())
}
/// `start`, `status` or `stop` a broker for the state directory `root`. `name` (`start` only)
/// names the workspace; see [`serve`].
pub fn lifecycle(command: &str, root: &Path, key: &str, name: Option<&str>) -> Result<Value> {
    match command {
        "start" => start(root, key, name),
        "status" => {
            let mut file = private_file(&root.join("runtime.json"), false)?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            let info: Value = serde_json::from_slice(&bytes)?;
            let socket = PathBuf::from(text(&info, "socket")?);
            let uid = number(&info, "host_uid")? as u32;
            validate_socket(&socket, uid)?;
            let mut stream = UnixStream::connect(socket)?;
            ensure!(peer_uid(&stream)? == uid, "identity_mismatch");
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            stream.write_all(
                b"{\"version\":1,\"id\":\"status\",\"method\":\"hello\",\"params\":{}}\n",
            )?;
            let response: Response = serde_json::from_slice(
                &read_frame(&mut BufReader::new(stream))?.ok_or_else(|| anyhow!("unavailable"))?,
            )?;
            ensure!(
                response.result.as_ref().and_then(|v| v.get("workspace_id"))
                    == info.get("workspace_id"),
                "identity_mismatch"
            );
            Ok(info)
        }
        "stop" => stop(root),
        _ => bail!("invalid_command"),
    }
}

/// How long `start` waits for the broker it launched to answer `hello`.
const START_READY_TIMEOUT: Duration = Duration::from_secs(15);

/// Launch `serve` detached, wait until it answers `hello`, and print the workspace's
/// invitation line (`brcrew1:…`) for the host to paste into Crew. A name is validated and
/// checked against the host account's running sibling workspaces **before** anything is
/// spawned.
fn start(root: &Path, key: &str, name: Option<&str>) -> Result<Value> {
    if let Some(name) = name {
        validate_workspace_name(name).map_err(|error| anyhow!(error.wire()))?;
    }
    private_dir(root)?;
    if let Some(name) = name {
        let stored = stored_workspace(root)?;
        if let Some(stored_name) = stored.as_ref().and_then(|stored| stored.name.as_deref()) {
            ensure!(
                stored_name == name,
                "name_mismatch: this workspace already has another name; start it without --name, or rename it in Crew"
            );
        }
        let (own_workspace, own_basename) = stored
            .map(|stored| (stored.id, stored.runtime_basename))
            .unwrap_or_default();
        ensure!(
            !sibling_workspace_names(
                Path::new("/tmp"),
                unsafe { libc::geteuid() },
                own_basename.as_deref(),
                &own_workspace,
            )
            .iter()
            .any(|sibling| sibling == name),
            WORKSPACE_NAME_TAKEN
        );
    }
    let log = private_file(&root.join("broker.log"), true)?;
    use std::os::unix::process::CommandExt;
    let mut command = std::process::Command::new(std::env::current_exe()?);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
        .arg("serve")
        .arg("--state-dir")
        .arg(root)
        .arg("--bootstrap-key")
        .arg(key);
    if let Some(name) = name {
        command.arg("--name").arg(name);
    }
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    let pid = child.id();
    let deadline = Instant::now() + START_READY_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            bail!(
                "start_failed: the broker stopped during startup ({status}); last log line: {}",
                last_log_line(root)
            );
        }
        if let Some(info) = read_runtime_descriptor(root)? {
            if info.get("pid").and_then(Value::as_u64) == Some(u64::from(pid)) {
                if let Ok(hello) = verified_hello(&info) {
                    let mut result = json!({
                        "started_pid": pid,
                        "state": "running",
                        "status_command": "status",
                        "workspace_id": hello["workspace_id"],
                        "name": hello["name"],
                    });
                    match start_invitation(&info, &hello) {
                        Ok(line) => result["invitation"] = json!(line),
                        Err(error) => {
                            result["invitation"] = Value::Null;
                            result["invitation_error"] = json!(error.to_string());
                        }
                    }
                    return Ok(result);
                }
            }
        }
        if Instant::now() >= deadline {
            return Ok(json!({"started_pid":pid,"state":"starting","status_command":"status"}));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
/// What `start` needs to know about a workspace already in its state directory.
struct StoredWorkspace {
    id: String,
    name: Option<String>,
    runtime_basename: Option<String>,
}
/// The workspace already in `root`, read from its journal without the writer lock (a live
/// writer only ever appends complete records).
fn stored_workspace(root: &Path) -> Result<Option<StoredWorkspace>> {
    let journal = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("journal.jsonl"))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("unsafe_storage: cannot read the journal"),
    };
    let metadata = journal.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe_storage: file ownership, mode or links invalid"
    );
    let replay = replay_journal(&journal)?;
    if replay.sequence == 0 {
        return Ok(None);
    }
    let workspace = &replay.state_value["workspace"];
    Ok(Some(StoredWorkspace {
        id: text(workspace, "id")?.to_owned(),
        name: workspace
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        runtime_basename: replay
            .state_value
            .get("runtime_basename")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }))
}
/// `runtime.json`, read without creating it.
fn read_runtime_descriptor(root: &Path) -> Result<Option<Value>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("runtime.json"))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("unsafe_runtime: cannot read runtime descriptor"),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= MAX_FRAME as u64,
        "unsafe_runtime: invalid private runtime descriptor"
    );
    Ok(serde_json::from_reader(file).ok())
}
/// `hello` from the broker `info` describes, checked as `status` checks it: the socket and its
/// listener belong to the host account, and the broker answers for the recorded workspace and
/// key.
fn verified_hello(info: &Value) -> Result<Value> {
    let socket = PathBuf::from(text(info, "socket")?);
    let uid = number(info, "host_uid")? as u32;
    validate_socket(&socket, uid)?;
    let mut stream = UnixStream::connect(socket)?;
    ensure!(peer_uid(&stream)? == uid, "identity_mismatch");
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(b"{\"version\":1,\"id\":\"start\",\"method\":\"hello\",\"params\":{}}\n")?;
    let response: Response = serde_json::from_slice(
        &read_frame(&mut BufReader::new(stream))?.ok_or_else(|| anyhow!("unavailable"))?,
    )?;
    let hello = response.result.ok_or_else(|| anyhow!("unavailable"))?;
    ensure!(
        hello.get("workspace_id") == info.get("workspace_id")
            && hello.get("workspace_public_key") == info.get("workspace_public_key"),
        "identity_mismatch"
    );
    Ok(hello)
}
/// The `brcrew1:` line for the host: the four pinned fields from the verified runtime, the
/// workspace's name, privacy mode and institution from `hello`, and the host's own username.
/// SSH hints are left out; the broker cannot know how people reach the server.
fn start_invitation(info: &Value, hello: &Value) -> Result<String> {
    let owner_uid = u32::try_from(number(hello, "host_uid")?)?;
    let optional = |key: &str| hello.get(key).and_then(Value::as_str).map(str::to_owned);
    let invitation = crate::invitation::WorkspaceInvitation {
        workspace_id: text(hello, "workspace_id")?.to_owned(),
        workspace_public_key: text(hello, "workspace_public_key")?.to_owned(),
        socket_path: text(info, "socket")?.to_owned(),
        owner_uid,
        workspace_name: optional("name"),
        host_username: Some(SystemDirectory.by_uid(owner_uid)?.name),
        host_display_name: None,
        mode: Some(mode(hello, "mode")?),
        institution_id: optional("institution_id"),
        ssh_host: None,
        ssh_port: None,
        proxy_jump: None,
        invitee_username: None,
    };
    crate::invitation::encode(&invitation).map_err(|error| anyhow!("{}: {error}", error.code()))
}
/// The last non-empty line of `broker.log`, stripped of control characters, for a start
/// failure message.
fn last_log_line(root: &Path) -> String {
    let read = || -> Result<String> {
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root.join("broker.log"))?;
        let length = file.metadata()?.len();
        file.seek(SeekFrom::Start(length.saturating_sub(4096)))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes)
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(300)
            .collect())
    };
    read().unwrap_or_default()
}
#[cfg(target_os = "linux")]
fn stop(root: &Path) -> Result<Value> {
    use std::os::fd::FromRawFd;
    private_dir(root)?;
    let mut file = private_file(&root.join("runtime.json"), false)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let info: Value = serde_json::from_slice(&bytes)?;
    let owner = number(&info, "host_uid")? as u32;
    ensure!(
        owner == unsafe { libc::geteuid() },
        "forbidden: only host account can stop broker"
    );
    let socket = PathBuf::from(text(&info, "socket")?);
    validate_socket(&socket, owner)?;
    let mut stream = UnixStream::connect(&socket)?;
    let mut cred = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    ensure!(
        unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                cred.as_mut_ptr().cast(),
                &mut length,
            )
        } == 0,
        "identity_unavailable"
    );
    let cred = unsafe { cred.assume_init() };
    ensure!(
        cred.uid == owner && cred.pid as u64 == number(&info, "pid")?,
        "identity_mismatch"
    );
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, cred.pid, 0) };
    ensure!(fd >= 0, "unsupported: safe stop requires Linux pidfd_open");
    let process = unsafe { File::from_raw_fd(fd as i32) };
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.write_all(
        b"{\"version\":1,\"id\":\"stop-verify\",\"method\":\"hello\",\"params\":{}}\n",
    )?;
    let response: Response = serde_json::from_slice(
        &read_frame(&mut BufReader::new(stream))?.ok_or_else(|| anyhow!("unavailable"))?,
    )?;
    ensure!(
        response.result.as_ref().and_then(|v| v.get("workspace_id")) == info.get("workspace_id"),
        "identity_mismatch"
    );
    ensure!(
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                process.as_raw_fd(),
                libc::SIGTERM,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        } == 0,
        "stop_failed"
    );
    let mut poll = libc::pollfd {
        fd: process.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let stopped = unsafe { libc::poll(&mut poll, 1, 5000) } > 0 && poll.revents & libc::POLLIN != 0;
    // Retain the descriptor and socket inode for the next writer to validate and reclaim.
    // A new writer may already have started after pidfd confirmed the old process exited.
    Ok(
        json!({"signal_sent":true,"stopped":stopped,"pid":cred.pid,"workspace_id":info.get("workspace_id")}),
    )
}
#[cfg(not(target_os = "linux"))]
fn stop(_root: &Path) -> Result<Value> {
    bail!("unsupported: stop requires Linux pidfd")
}

#[cfg(test)]
mod uid_min_tests {
    use super::*;

    #[test]
    fn uid_min_is_the_last_uncommented_setting_in_login_defs() {
        assert_eq!(parse_uid_min(""), None);
        assert_eq!(parse_uid_min("UID_MAX 60000\n"), None);
        assert_eq!(
            parse_uid_min("# UID_MIN 10\nUID_MIN\t\t 1000\n"),
            Some(1000)
        );
        assert_eq!(
            parse_uid_min("UID_MIN 500 # old RHEL\nUID_MIN 1000\n"),
            Some(1000)
        );
        assert_eq!(parse_uid_min("SYS_UID_MIN 100\nUID_MIN 2000\n"), Some(2000));
        assert_eq!(parse_uid_min("UID_MIN lots\n"), None);
    }
}
