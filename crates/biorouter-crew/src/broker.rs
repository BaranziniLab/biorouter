use crate::*;
use anyhow::{anyhow, bail, ensure, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
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

/// The kernel's overflow UID, which NSS reports as `nobody`: never a person.
const OVERFLOW_UID: u32 = 65534;

/// Whether an account is a system account that can never join a workspace: UID 0, a UID below
/// `uid_min` or the overflow UID, or a `nologin`/`false` login shell.
fn is_system_account(account: &Account, uid_min: u32) -> bool {
    let shell = account.shell.as_deref().map(|shell| {
        shell
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(shell)
    });
    account.uid == 0
        || account.uid < uid_min
        || account.uid == OVERFLOW_UID
        || account.uid == u32::MAX
        || shell.is_some_and(|name| name.ends_with("nologin") || name == "false")
}

/// T-54: UID 0, a UID below the node's `UID_MIN` (or the overflow UID `nobody` holds), and an
/// account whose login shell is `nologin` or `false` are the server's own, never a person's.
/// Checked on the canonical account, through the [`Directory`] seam, before anything is
/// recorded.
///
/// Every door that records an invitation asks this, whatever the build: the S3a invitation by
/// name (`broker/join.rs`, feature `join-by-name`) and the legacy `enrollment.invite {uid,
/// public_key}` token path (Q2-14), which used to skip it because the check lived only in the
/// feature-gated module.
fn refuse_system_account(broker: &Broker, account: &Account) -> Result<()> {
    ensure!(
        !is_system_account(account, broker.directory.uid_min()),
        "name_invalid: @{} is a system account on this server and can't join a workspace.",
        account.name
    );
    Ok(())
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
    /// When the result was cached (seconds since the Unix epoch). A result is kept for
    /// [`Quotas::dedupe_ttl_secs`]; one cached before this field existed is pruned first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    at: Option<u64>,
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
    /// The person whose signed device last spoke on this connection, for presence. A member's
    /// bridge holds one connection for as long as their computer is connected.
    principal: Option<String>,
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
    /// Set once a journal write or sync failed: the broker then saves nothing more until it is
    /// restarted (fail-stop), and says so in `hello`, `status` and every refused change.
    storage_fault: Option<StorageFault>,
    /// A test's injected journal failure (feature `test-seams`).
    #[cfg(feature = "test-seams")]
    injected_journal_fault: Option<(JournalCall, i32)>,
    /// The account database every UID-to-name check goes through.
    directory: Box<dyn Directory + Send>,
    /// Where sibling runtime directories (`crew-<uid>-<hex>/broker.sock`) live: `/tmp`.
    runtime_root: PathBuf,
    /// Per-actor times of recent name-collision refusals (in memory only), for the rate limit
    /// that bounds the name-existence oracle (D5).
    name_refusals: BTreeMap<String, VecDeque<u64>>,
    /// The limits this broker enforces.
    quotas: Quotas,
    /// Journal bytes each actor's records take, tallied at replay and kept up to date by
    /// [`Broker::commit_with`], for [`Quotas::member_journal_bytes`].
    journal_actor_bytes: BTreeMap<String, u64>,
    /// The runs retention removed, so `run.revoke` can still answer their owners.
    removed_runs: RemovedRuns,
    /// Who is connected, per principal ([`Presence`]). In memory only: never journaled, never
    /// counted in the state or any quota, and consulted by no authorization.
    presence: BTreeMap<String, Presence>,
    /// How long after a person's last signed request they still count as online without a
    /// connection open.
    presence_window: Duration,
    /// The serialized size of the committed state, kept by every commit for the host's
    /// `usage`.
    state_bytes: usize,
    #[cfg(feature = "join-by-name")]
    join_runtime: join::Runtime,
}
/// One person's presence: when their device last made a signed request, and how many
/// connections it made them on are still open.
#[derive(Default)]
struct Presence {
    last_request: Option<Instant>,
    open_connections: usize,
}
/// A person counts as online while a connection they signed on is open, and for this long
/// after their last signed request. The member's daemon heartbeats an idle connection every
/// 120 s, so this outlasts one missed beat.
const PRESENCE_WINDOW: Duration = Duration::from_secs(180);

/// Which journal call a test makes fail ([`Broker::inject_journal_fault`]).
#[cfg(feature = "test-seams")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalCall {
    /// The record's `write`: nothing of it is saved.
    Write,
    /// The `fsync` after it: whether the record was saved is unknown.
    Sync,
}
/// Why the broker stopped saving changes. Kept in memory only: a restart repairs the journal
/// (a torn tail is set aside) and starts clean.
#[derive(Clone, Debug)]
struct StorageFault {
    /// The disk or the account's quota is full (`ENOSPC`, `EDQUOT`), rather than another
    /// storage error.
    full: bool,
    /// When it happened, in seconds since the Unix epoch.
    at: u64,
    /// The operating system's error, for the host's log only. Never sent to a member.
    detail: String,
    /// The journal's length before the record that failed.
    committed: u64,
    /// Whether the line for it is in `broker.log` yet. A full disk refuses that write too, so
    /// it is tried again on later requests until space is freed.
    logged: bool,
}
impl StorageFault {
    fn code(&self) -> &'static str {
        if self.full {
            "storage_full"
        } else {
            "storage_failed"
        }
    }
    /// The sentence every change is refused with once the broker has stopped saving, and what
    /// `hello` reports, without its code.
    fn stopped_sentence(&self) -> &'static str {
        if self.full {
            STORAGE_FULL_STOPPED
        } else {
            STORAGE_FAILED_STOPPED
        }
    }
    fn refusal(&self) -> anyhow::Error {
        anyhow!("{}: {}", self.code(), self.stopped_sentence())
    }
    /// What the host is told to do, naming the state directory.
    fn host_instruction(&self, root: &Path) -> String {
        let first = if self.full {
            "Free space on this server"
        } else {
            "Check this server's storage"
        };
        format!(
            "{first}, then restart Crew: biorouter-crew stop --state-dir {root}, then \
             biorouter-crew start --state-dir {root}.",
            root = root.display()
        )
    }
}
const STORAGE_FULL_STOPPED: &str = "The workspace server ran out of disk space and has stopped saving changes. Reading still works. Ask the host to free space on the server and restart Crew.";
const STORAGE_FAILED_STOPPED: &str = "The workspace server could not save a change to disk and has stopped saving changes. Reading still works. Ask the host to check the server's storage and restart Crew.";
const STORAGE_FULL_NOT_SAVED: &str = "storage_full: The workspace server is out of disk space, so this change was not saved. Reading still works. Ask the host to free space on the server and restart Crew.";
const STORAGE_FULL_UNCERTAIN: &str = "storage_full: The workspace server ran out of disk space while saving this change, so it may not have been saved. Reading still works. Ask the host to free space on the server and restart Crew.";
const STORAGE_FAILED_NOT_SAVED: &str = "storage_failed: The workspace server could not write this change to disk, so it was not saved. Reading still works. Ask the host to check the server's storage and restart Crew.";
const STORAGE_FAILED_UNCERTAIN: &str = "storage_failed: The workspace server could not confirm this change was saved to disk, so it may not have been saved. Reading still works. Ask the host to check the server's storage and restart Crew.";
/// A storage error outside the journal (an attachment's file): nothing was recorded, so the
/// broker keeps saving, and the same request can be sent again once there is space.
const STORAGE_FULL_RETRY: &str = "storage_full: The workspace server is out of disk space, so this could not be saved. Ask the host to free space on the server, then try again.";
const STORAGE_FAILED_RETRY: &str = "storage_failed: The workspace server could not read or write its storage. Ask the host to check the server's storage, then try again.";
/// Whether `error` means the disk, or the account's disk quota, is full.
fn is_space_error(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(code) if code == libc::ENOSPC || code == libc::EDQUOT)
}
/// `at` (seconds since the Unix epoch) as UTC, `2026-09-27T21:03:04Z`.
fn utc_timestamp(at: u64) -> String {
    let days = at / 86_400;
    let seconds = at % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3_600,
        seconds % 3_600 / 60,
        seconds % 60
    )
}
/// A refusal as the wire carries it: the code is the message's `code:` prefix, else
/// `request_denied`. A raw operating-system error (an attachment's file could not be written)
/// is a storage fault, said in words, never `request_denied` with the OS's own text.
fn protocol_error(error: &anyhow::Error) -> ProtocolError {
    let code_of = |message: &str| {
        message
            .split(':')
            .next()
            .filter(|s| s.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .map(str::to_owned)
    };
    let mut message = error.to_string();
    if code_of(&message).is_none() {
        if let Some(io) = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        {
            message = if is_space_error(io) {
                STORAGE_FULL_RETRY
            } else {
                STORAGE_FAILED_RETRY
            }
            .to_owned();
        }
    }
    let code = code_of(&message).unwrap_or_else(|| "request_denied".to_owned());
    ProtocolError { code, message }
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
/// Q2-78: a direct add naming a principal ID this workspace has never held.
const DIRECT_ADD_UNKNOWN_PERSON: &str =
    "forbidden: That person isn't a member of this workspace. Refresh and choose again.";
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

const MIB: u64 = 1024 * 1024;
const DAY_SECS: u64 = 24 * 60 * 60;

/// The workspace's resource limits. Every broker enforces [`Quotas::STANDARD`]; test builds may
/// scale them down (`Broker::set_quotas`, feature `test-seams`).
///
/// Three rules keep one member from exhausting what everyone shares:
///
/// - **Shares.** No member other than the host may hold more than a set part of any
///   workspace-wide budget: a quarter of the state, the journal, the attachment bytes and
///   counts and the references, a tenth of the teams and channels, and a bounded number of
///   live invitations. The host, who owns the workspace, has no share.
/// - **Headroom.** Ordinary changes stop short of the state and journal limits, so the host can
///   always remove a member, change policy and let people in once they are reached.
/// - **Retention.** Idempotency results, unfinished uploads, long-expired invitations and runs
///   age out, so none of them becomes a permanent cap. Messages and completed attachments are
///   history and are never removed.
#[derive(Clone, Copy, Debug)]
pub struct Quotas {
    /// The serialized logical state.
    pub state_bytes: usize,
    /// The part of `state_bytes` only the host's administrative operations and enrollment
    /// may use.
    pub state_admin_headroom: usize,
    /// The retained journal.
    pub journal_bytes: u64,
    /// The part of `journal_bytes` only the host's administrative operations, enrollment and
    /// the broker's own records may use.
    pub journal_admin_headroom: u64,
    /// The state one member (not the host) may hold in records they created: their messages,
    /// runs, attachments, references, invitations, teams and channels.
    pub member_state_bytes: usize,
    /// The journal one member (not the host) may write.
    pub member_journal_bytes: u64,
    /// Teams one member (not the host) may create.
    pub member_teams: usize,
    /// Channels one member (not the host) may create, including their teams' `#general`.
    pub member_channels: usize,
    /// Attachments one member (not the host) may hold, finished or in progress.
    pub member_blobs: usize,
    /// Declared attachment bytes one member (not the host) may hold.
    pub member_blob_bytes: u64,
    /// Remote references one member (not the host) may create.
    pub member_references: usize,
    /// Unexpired invitations one member (not the host) may have outstanding.
    pub member_live_invitations: usize,
    /// How long an idempotency result is kept.
    pub dedupe_ttl_secs: u64,
    /// At most this many idempotency results are kept per principal (newest first) ...
    pub dedupe_actor_entries: usize,
    /// ... in at most this many serialized bytes.
    pub dedupe_actor_bytes: usize,
    /// At most this many idempotency results are kept in all (newest first).
    pub dedupe_entries: usize,
}

impl Quotas {
    pub const STANDARD: Quotas = Quotas {
        state_bytes: 16 * MIB as usize,
        state_admin_headroom: MIB as usize,
        journal_bytes: 1024 * MIB,
        journal_admin_headroom: 16 * MIB,
        member_state_bytes: 4 * MIB as usize,
        member_journal_bytes: 256 * MIB,
        member_teams: 10,
        member_channels: 100,
        member_blobs: 2_500,
        member_blob_bytes: 10 * 1024 * MIB / 4,
        member_references: 2_500,
        member_live_invitations: 100,
        dedupe_ttl_secs: DAY_SECS,
        dedupe_actor_entries: 512,
        dedupe_actor_bytes: 128 * 1024,
        dedupe_entries: 100_000,
    };
}

/// An upload that has not begun or received a chunk for this long is removed.
const BLOB_UPLOAD_TTL_SECS: u64 = DAY_SECS;
/// An invitation is removed this long after it expired (its inviter sees it marked expired
/// until then).
const INVITATION_RETENTION_SECS: u64 = 7 * DAY_SECS;
/// A run (and its grant) is removed this long after it expired.
const RUN_RETENTION_SECS: u64 = DAY_SECS;
/// At most this many records of each kind are pruned by one mutation, so one journal record
/// stays small however much has aged out at once.
const PRUNE_BATCH: usize = 1024;
/// The largest single attachment, and every attachment together.
const BLOB_MAX_BYTES: u64 = 1024 * MIB;
const WORKSPACE_BLOB_BYTES: u64 = 10 * 1024 * MIB;
const WORKSPACE_BLOBS: usize = 10_000;
/// The host's own operations that may use the administrative headroom.
const ADMIN_METHODS: [&str; 6] = [
    "enrollment.invite",
    "enrollment.approve",
    "enrollment.cancel",
    "enrollment.revoke",
    "policy.set",
    "workspace.rename",
];
/// Operations that only take access away. Anyone may use the administrative headroom for them,
/// so a full workspace never keeps a person in a channel or an agent running: each adds at
/// most a bounded idempotency result, and one member's results are capped.
const ACCESS_REMOVAL_METHODS: [&str; 3] = ["run.revoke", "membership.revoke", "channel.archive"];
/// Methods that add records to the actor's share of the state: a member past
/// [`Quotas::member_state_bytes`] is refused them. Everything else changes records in place,
/// removes them, or adds a bounded amount (a read position, a membership).
const SHARE_METHODS: [&str; 8] = [
    "message.post",
    "run.project",
    "run.create",
    "blob.begin",
    "reference.create",
    "invitation.create",
    "team.create",
    "channel.create",
];
const MEMBER_STATE_QUOTA: &str = "quota_exceeded: You have used your share of this workspace's storage. Reading still works; ask the workspace host about starting a new workspace.";
const MEMBER_JOURNAL_QUOTA: &str = "quota_exceeded: You have made as many changes as one member's share of this workspace's audit journal allows. Reading still works; ask the workspace host about starting a new workspace.";
/// A snapshot's invitations, runs and references are each capped at this many serialized
/// bytes, and its teams and channels take what the rest leaves under the frame limit less
/// [`SNAPSHOT_FRAME_MARGIN`], so what other members do can never push a snapshot past the frame
/// limit. The full counts are in `totals`.
const SNAPSHOT_SECTION_BYTES: usize = 64 * 1024;
/// What a snapshot leaves unused of the frame: the response around it (`{"id":…,"result":…}`
/// and its newline) and room to spare.
const SNAPSHOT_FRAME_MARGIN: usize = 1024;
/// A worker's `context.manifest` carries at most this many serialized bytes of messages.
const CONTEXT_MANIFEST_BYTES: usize = 640 * 1024;
/// A worker's `messages.history` or `messages.search` page carries at most this many
/// serialized bytes of messages, and always at least one: a single message stays under the
/// frame limit even at the workspace's attachment and reference limits.
const WORKER_HISTORY_BYTES: usize = CONTEXT_MANIFEST_BYTES;
/// A message body's JSON form: twice its 65,536-byte limit, what quotes and backslashes can
/// double it to. Only control characters escape to more.
const MESSAGE_ESCAPED_BYTES: usize = 2 * 65_536;

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
    /// Journal bytes of each actor's complete records.
    actor_bytes: BTreeMap<String, u64>,
    /// The runs the journal's records removed from the state.
    removed_runs: RemovedRuns,
}
/// The runs retention removed from the state ([`prune_retained`]), each as a short digest of
/// its ID with a short digest of its owner's principal ID. Kept in memory only, and rebuilt at
/// every open from the journal, which keeps the record of every removal for as long as the
/// workspace exists: it costs the logical state nothing and never forgets a run. The journal
/// also bounds it: each removed run was created by a record of several hundred bytes, so even a
/// full 1 GiB journal names at most a couple of million, at about 40 bytes each here.
///
/// ⚠ **Grant and revocation state; needs human review.** `run.revoke` asks it. A daemon that
/// stopped a grant while the workspace was out of reach asks to revoke the run whenever it
/// reconnects, however long after the run expired, and a daemon that replaced or re-granted a
/// chat's grant asks about the earlier run for a week past its end. Once retention removed the
/// run that request was refused as a run the owner does not hold, so the stop could never be
/// confirmed and was asked about again at every reconnect. A removed run is honored by nothing
/// (it expired, and its grant went with it), so its owner is now told it is revoked; anyone
/// else is answered as for a run that never existed.
#[derive(Default)]
struct RemovedRuns(HashMap<[u8; 16], [u8; 16]>);
impl RemovedRuns {
    fn key(text: &str) -> [u8; 16] {
        let digest = Sha256::digest(text.as_bytes());
        let mut key = [0; 16];
        key.copy_from_slice(&digest[..16]);
        key
    }
    fn insert(&mut self, run_id: &str, owner_id: &str) {
        self.0.insert(Self::key(run_id), Self::key(owner_id));
    }
    /// Whether `run_id` was removed while `owner_id` owned it.
    fn owned_by(&self, run_id: &str, owner_id: &str) -> bool {
        self.0.get(&Self::key(run_id)) == Some(&Self::key(owner_id))
    }
    /// Note a run `patch` removes from `state`, before it is applied.
    fn note(&mut self, state: &Value, patch: &Patch) {
        if let Patch::Remove { path } = patch {
            if let [map, run_id] = path.as_slice() {
                if map == "runs" {
                    if let Some(owner) = state
                        .get("runs")
                        .and_then(|runs| runs.get(run_id))
                        .and_then(|run| run.get("owner_id"))
                        .and_then(Value::as_str)
                    {
                        self.insert(run_id, owner);
                    }
                }
            }
        }
    }
}
/// Replay one complete journal record onto `state_value`, noting the runs it removes in
/// `removed_runs`; returns the record's actor.
fn replay_record(
    line: &[u8],
    state_value: &mut Value,
    checksum: &mut String,
    sequence: &mut u64,
    removed_runs: &mut RemovedRuns,
) -> Result<String> {
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
            Ok(record.actor)
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
                removed_runs.note(state_value, &patch);
                apply_patch(state_value, patch)?;
            }
            *checksum = actual;
            *sequence = record.sequence;
            Ok(record.actor)
        }
        _ => bail!("journal_corrupt: unsupported journal version"),
    }
}
fn replay_journal(journal: &File) -> Result<JournalReplay> {
    let mut replay = BufReader::new(journal.try_clone()?);
    let mut torn_tail = None;
    let mut state_value = Value::Null;
    let mut checksum = String::new();
    let mut sequence = 0;
    let mut committed = 0;
    let mut actor_bytes: BTreeMap<String, u64> = BTreeMap::new();
    let mut removed_runs = RemovedRuns::default();
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
        let actor = replay_record(
            &line,
            &mut state_value,
            &mut checksum,
            &mut sequence,
            &mut removed_runs,
        )?;
        *actor_bytes.entry(actor).or_default() += line.len() as u64;
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
        actor_bytes,
        removed_runs,
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
/// Which limits a commit answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Allowance {
    /// May use the state's administrative headroom, up to the full state limit: the broker's
    /// own records, enrollment, the host's administrative operations and anyone's removal of
    /// access. Everything else stops short of the limit by the headroom.
    state_headroom: bool,
    /// May use the journal's administrative headroom, up to the full journal limit: the
    /// broker's own records, enrollment and the host's administrative operations only. A
    /// member's removals of access do not get it, since repeating one writes a record each
    /// time and would let one member use it up.
    journal_headroom: bool,
    /// Answers to the actor's own journal share: everyone but the host.
    member: bool,
}
impl Allowance {
    const FULL: Allowance = Allowance {
        state_headroom: true,
        journal_headroom: true,
        member: false,
    };
}
/// Counts the bytes written to it.
struct ByteCount(usize);
impl Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
/// The length of `value`'s JSON serialization, without building it.
fn json_len<T: Serialize + ?Sized>(value: &T) -> usize {
    let mut count = ByteCount(0);
    serde_json::to_writer(&mut count, value).map_or(usize::MAX, |()| count.0)
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
    /// Point the sibling-workspace probe of `workspace.rename`, and the runtime directory
    /// [`Broker::prepare_runtime`] binds, at another directory than `/tmp`. Test builds only.
    #[cfg(feature = "test-seams")]
    pub fn set_runtime_root(&mut self, runtime_root: &Path) {
        self.runtime_root = runtime_root.to_path_buf();
    }
    /// What `serve` does before it binds its socket: read the recorded runtime, choose (and
    /// journal) the runtime directory under the runtime root, and return the socket path.
    /// `node_id` stands in for the node identity `runtime.json` must name. Test builds only.
    #[cfg(feature = "test-seams")]
    pub fn prepare_runtime(&mut self, node_id: &str) -> Result<PathBuf> {
        persisted_runtime(self, node_id)
    }
    /// Enforce `quotas` instead of [`Quotas::STANDARD`], so a test can reach a limit without
    /// writing a gigabyte. Test builds only.
    #[cfg(feature = "test-seams")]
    pub fn set_quotas(&mut self, quotas: Quotas) {
        self.quotas = quotas;
    }
    /// Count a person as online for `window` after their last signed request instead of
    /// [`PRESENCE_WINDOW`], so a test need not wait three minutes. Test builds only.
    #[cfg(feature = "test-seams")]
    pub fn set_presence_window(&mut self, window: Duration) {
        self.presence_window = window;
    }
    /// Make the next journal `call` fail with `errno` (`ENOSPC`, `EIO`, ...), as a full or
    /// failing disk would. Test builds only.
    #[cfg(feature = "test-seams")]
    pub fn inject_journal_fault(&mut self, call: JournalCall, errno: i32) {
        self.injected_journal_fault = Some((call, errno));
    }
    /// The authoritative state as JSON, for tests that check what is retained. Test builds
    /// only.
    #[cfg(feature = "test-seams")]
    pub fn state_json(&self) -> Value {
        serde_json::to_value(&self.state).expect("state serializes")
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
            actor_bytes,
            removed_runs,
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
            storage_fault: None,
            #[cfg(feature = "test-seams")]
            injected_journal_fault: None,
            directory,
            runtime_root: PathBuf::from("/tmp"),
            name_refusals: BTreeMap::new(),
            quotas: Quotas::STANDARD,
            journal_actor_bytes: actor_bytes,
            removed_runs,
            presence: BTreeMap::new(),
            presence_window: PRESENCE_WINDOW,
            state_bytes: 0,
            #[cfg(feature = "join-by-name")]
            join_runtime: join::Runtime::default(),
        };
        broker.state_bytes = json_len(&broker.state);
        if sequence == 0 {
            broker.commit(broker.state.clone(), "system", "workspace.initialize")?;
        }
        Ok(broker)
    }
    /// Commit with the full limits: the broker's own records, enrollment and the host's
    /// administrative operations.
    fn commit(&mut self, state: State, actor: &str, operation: &str) -> Result<()> {
        self.commit_with(state, actor, operation, Allowance::FULL)
    }
    fn commit_with(
        &mut self,
        mut state: State,
        actor: &str,
        operation: &str,
        allowance: Allowance,
    ) -> Result<()> {
        if let Some(fault) = &self.storage_fault {
            return Err(fault.refusal());
        }
        let quotas = self.quotas;
        state.sequence = self.state.sequence + 1;
        let after = serde_json::to_value(&state)?;
        let before = if self.state.sequence == 0 {
            Value::Null
        } else {
            serde_json::to_value(&self.state)?
        };
        let size = json_len(&after);
        // A commit that does not grow the state (removing what aged out) is never refused
        // for size: it can only help a workspace at its limit.
        let shrinks = size <= json_len(&before);
        if allowance.state_headroom {
            ensure!(shrinks || size <= quotas.state_bytes, "quota_exceeded: workspace logical state exceeds 16 MiB; reads remain available but further mutations require a new workspace or a supported retention upgrade; in-place pruning is not supported");
        } else {
            ensure!(shrinks || size <= quotas.state_bytes - quotas.state_admin_headroom, "quota_exceeded: workspace logical state is full; reads remain available and the host can still remove members and change policy, but further changes require a new workspace; in-place pruning of history is not supported");
        }
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
        let length = bytes.len() as u64;
        let committed = self.journal.metadata()?.len();
        let journal = committed + length;
        ensure!(
            bytes.len() <= 16 * 1024 * 1024 && journal <= quotas.journal_bytes,
            "quota_exceeded: retained audit journal exceeds 1 GiB; preserve the complete store and use a new workspace; in-place audit deletion is not supported"
        );
        ensure!(
            allowance.journal_headroom
                || journal <= quotas.journal_bytes - quotas.journal_admin_headroom,
            "quota_exceeded: retained audit journal is nearly full; reads remain available and the host can still remove members and change policy; preserve the complete store and use a new workspace"
        );
        let written = self.journal_actor_bytes.get(actor).copied().unwrap_or(0);
        ensure!(
            !allowance.member || written.saturating_add(length) <= quotas.member_journal_bytes,
            MEMBER_JOURNAL_QUOTA
        );
        if let Err((written, error)) = self.append_record(&bytes) {
            return Err(self.stop_saving(written, &error, committed));
        }
        self.state = state;
        self.state_bytes = size;
        self.checksum = record.checksum;
        *self
            .journal_actor_bytes
            .entry(actor.to_owned())
            .or_default() += length;
        Ok(())
    }
    /// Write `bytes` to the journal and make them durable. On failure, whether any of the
    /// record may have reached the file (`true` once the write itself succeeded) and why.
    fn append_record(&mut self, bytes: &[u8]) -> std::result::Result<(), (bool, std::io::Error)> {
        #[cfg(feature = "test-seams")]
        let injected = self.injected_journal_fault.take();
        #[cfg(feature = "test-seams")]
        if let Some((JournalCall::Write, errno)) = injected {
            return Err((false, std::io::Error::from_raw_os_error(errno)));
        }
        self.journal
            .write_all(bytes)
            .map_err(|error| (false, error))?;
        #[cfg(feature = "test-seams")]
        if let Some((JournalCall::Sync, errno)) = injected {
            return Err((true, std::io::Error::from_raw_os_error(errno)));
        }
        self.journal.sync_all().map_err(|error| (true, error))?;
        File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| (true, error))
    }
    /// Stop saving (fail-stop) after the journal write or sync failed with `error`, tell the
    /// host in `broker.log`, and return the refusal for the request that hit it. `written`:
    /// the record's write succeeded, so whether it was saved is unknown. The broker cannot
    /// know what reached the disk, so nothing is saved again until a restart repairs the
    /// journal (a torn tail is set aside) and replays what was really written.
    fn stop_saving(
        &mut self,
        written: bool,
        error: &std::io::Error,
        committed: u64,
    ) -> anyhow::Error {
        let full = is_space_error(error);
        self.storage_fault = Some(StorageFault {
            full,
            at: now(),
            detail: error.to_string(),
            committed,
            logged: false,
        });
        self.log_storage_fault();
        anyhow!(match (full, written) {
            (true, false) => STORAGE_FULL_NOT_SAVED,
            (true, true) => STORAGE_FULL_UNCERTAIN,
            (false, false) => STORAGE_FAILED_NOT_SAVED,
            (false, true) => STORAGE_FAILED_UNCERTAIN,
        })
    }
    /// Append the line for the storage fault to `broker.log` in the state directory, once.
    /// Best effort: on a full disk this fails too, and a later request tries again.
    fn log_storage_fault(&mut self) {
        let root = &self.root;
        let Some(fault) = self.storage_fault.as_mut().filter(|fault| !fault.logged) else {
            return;
        };
        let line = format!(
            "{} {}: Crew stopped saving changes: {}. journal.jsonl held {} bytes before the change that failed. Reading still works. {}\n",
            utc_timestamp(fault.at),
            fault.code(),
            fault.detail,
            fault.committed,
            fault.host_instruction(root)
        );
        fault.logged = private_file(&root.join("broker.log"), true)
            .and_then(|mut log| {
                log.write_all(line.as_bytes())?;
                log.sync_all()?;
                Ok(())
            })
            .is_ok();
    }
    pub fn workspace(&self) -> &Workspace {
        &self.state.workspace
    }
    /// `principal`'s device made a signed request on `conn`.
    fn note_presence(&mut self, conn: &mut Connection, principal: &str) {
        let now = Instant::now();
        if conn.principal.as_deref() == Some(principal) {
            // The connection is already counted: only the time moves.
            if let Some(presence) = self.presence.get_mut(principal) {
                presence.last_request = Some(now);
                return;
            }
        }
        if let Some(previous) = conn.principal.take() {
            self.release_presence(&previous);
        }
        let presence = self.presence.entry(principal.to_owned()).or_default();
        presence.open_connections += 1;
        presence.last_request = Some(now);
        conn.principal = Some(principal.to_owned());
    }
    fn release_presence(&mut self, principal: &str) {
        if let Some(presence) = self.presence.get_mut(principal) {
            presence.open_connections = presence.open_connections.saturating_sub(1);
        }
    }
    /// `connection` has closed: the person it was signed on for stops counting as connected
    /// through it. They stay online until [`PRESENCE_WINDOW`] after their last request.
    pub fn connection_closed(&mut self, connection: &mut Connection) {
        if let Some(principal) = connection.principal.take() {
            self.release_presence(&principal);
        }
    }
    /// The active people who are online: a connection they signed on is open, or they made a
    /// signed request within the presence window.
    fn online_principal_ids<'s>(&self, s: &'s State) -> Vec<&'s str> {
        let now = Instant::now();
        s.principals
            .values()
            .filter(|p| p.active)
            .filter(|p| {
                self.presence.get(&p.id).is_some_and(|presence| {
                    presence.open_connections > 0
                        || presence
                            .last_request
                            .is_some_and(|at| now.duration_since(at) <= self.presence_window)
                })
            })
            .map(|p| p.id.as_str())
            .collect()
    }
    /// How full the workspace's non-renewable budgets are, for its host: the logical state,
    /// the audit journal and the attachment space. Only the host can act on them (a new
    /// workspace, removals), so only the host's snapshot carries this.
    fn usage(&self, s: &State) -> Value {
        let quotas = &self.quotas;
        json!({
            "state_bytes": self.state_bytes,
            "state_limit": quotas.state_bytes,
            "state_admin_headroom": quotas.state_admin_headroom,
            "journal_bytes": self.journal.metadata().map(|m| m.len()).unwrap_or_default(),
            "journal_limit": quotas.journal_bytes,
            "journal_admin_headroom": quotas.journal_admin_headroom,
            "attachment_bytes": s.blobs.values().map(|b| b.size).sum::<u64>(),
            "attachment_limit": WORKSPACE_BLOB_BYTES,
            "attachments": s.blobs.len(),
            "attachments_limit": WORKSPACE_BLOBS,
        })
    }
    pub fn handle(&mut self, uid: u32, connection: &mut Connection, request: Request) -> Response {
        let result = self.process(uid, connection, &request);
        match result {
            Ok(value) => Response {
                id: request.id,
                result: Some(value),
                error: None,
            },
            Err(error) => Response {
                id: request.id,
                result: None,
                error: Some(protocol_error(&error)),
            },
        }
    }
    fn process(&mut self, uid: u32, conn: &mut Connection, req: &Request) -> Result<Value> {
        ensure!(
            req.version == 1 && !req.id.is_empty() && req.id.len() <= 128,
            "invalid_request: version/id"
        );
        self.log_storage_fault();
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
        // A person's signed device, not an agent's grant: presence is about people.
        if actor.run.is_none() {
            self.note_presence(conn, &actor.id);
        }
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
            self.recheck_replay(actor, req)?;
            ensure!(
                saved.digest == fingerprint,
                "conflict: idempotency key reused with different request"
            );
            return Ok(saved.result.clone());
        }
        if let Some(fault) = &self.storage_fault {
            return Err(fault.refusal());
        }
        let naming = NAME_METHODS.contains(&req.method.as_str());
        if naming {
            // Checked before anything about the name is evaluated, so the answer to a
            // rate-limited attempt is the same whether or not the name is taken.
            ensure!(
                self.recent_name_refusals(&actor.id, now()) < NAME_REFUSAL_LIMIT,
                NAME_RATE_LIMITED
            );
        }
        let now = now();
        let mut state = self.state.clone();
        #[cfg(feature = "join-by-name")]
        join::prune_expired(&mut state, now);
        let Pruned {
            uploads: expired_uploads,
            runs: removed_runs,
        } = prune_retained(&mut state, now, &self.quotas);
        // Noted now, before the commit and before `run.revoke` might ask about one of them: a
        // run removed here expired more than a day ago, so nothing honors it whether or not
        // this commit lands, and the index is asked only about a run the state does not hold.
        for (run_id, owner_id) in &removed_runs {
            self.removed_runs.insert(run_id, owner_id);
        }
        let result = match self.mutate(&mut state, actor, req) {
            Ok(result) => result,
            Err(error) => {
                if naming && error.to_string().starts_with("name_taken:") {
                    self.record_name_refusal(&actor.id, now);
                }
                return Err(error);
            }
        };
        let host = self.manager(&self.state, &actor.id).is_ok();
        let method = req.method.as_str();
        let administrative = host && actor.run.is_none() && ADMIN_METHODS.contains(&method);
        let allowance = Allowance {
            state_headroom: administrative || ACCESS_REMOVAL_METHODS.contains(&method),
            journal_headroom: administrative,
            member: !host,
        };
        let committed = (|| -> Result<()> {
            let share = self.quotas.member_state_bytes;
            // A terminal projection ends its run, and with it the grant, so a member still
            // within their share before it may post it even when it takes them past: an agent
            // task finishes with its result rather than failing for the last few bytes. Only
            // from within the share, so a member is never more than one message past it,
            // however many runs they created while under it. Past it, a terminal projection is
            // refused like any other addition, and the run's owner still ends the run with
            // `run.revoke`, which is never refused for space.
            let terminal = method == "run.project"
                && result["status"]
                    .as_str()
                    .is_some_and(|status| status != "progress");
            ensure!(
                host || !SHARE_METHODS.contains(&method)
                    || member_state_bytes(&state, &actor.id) <= share
                    || (terminal && member_state_bytes(&self.state, &actor.id) <= share),
                MEMBER_STATE_QUOTA
            );
            // A message is re-projected from the stored message on replay, so its cached
            // result needs only what finds it (and the status a terminal replay checks).
            let cached = match req.method.as_str() {
                "message.post" | "run.project" => {
                    json!({"id": result["id"], "status": result["status"]})
                }
                _ => result.clone(),
            };
            remember(
                &mut state,
                &self.quotas,
                &actor.id,
                key,
                Cached {
                    digest: fingerprint,
                    result: cached,
                    at: Some(now),
                },
            );
            self.commit_with(state, &actor.id, &req.method, allowance)
        })();
        if let Err(error) = committed {
            if req.method == "blob.begin" && self.storage_fault.is_none() {
                if let Some(blob_id) = result.get("id").and_then(Value::as_str) {
                    let _ = fs::remove_file(self.root.join("blobs").join(blob_id));
                }
            }
            return Err(error);
        }
        // Only once the removal is journaled: a failed commit keeps both.
        for blob_id in expired_uploads {
            let _ = fs::remove_file(self.root.join("blobs").join(blob_id));
        }
        Ok(result)
    }
    /// A retried mutation's cached result is returned only while the actor may still reach
    /// what it names: the channel (as a member) and the attachment.
    fn recheck_replay(&self, actor: &Actor, req: &Request) -> Result<()> {
        if let Some(channel) = req.params.get("channel_id").and_then(Value::as_str) {
            // The host may add people to a channel it is not in (direct add), so its retry of
            // that add is re-authorized as the host, not as a member. Likewise a member's retry
            // of leaving a channel (they are no longer in it, and the result is the one they
            // were given), and the host's of acting on a channel whose owner left the workspace.
            let host_add =
                req.method == "channel.add_member" && self.manager(&self.state, &actor.id).is_ok();
            let outside = Self::is_leave(req, &actor.id)
                || self.stewards_orphaned_channel(&self.state, actor, req);
            if !host_add && !outside {
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
        Ok(())
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
        let mut hello = json!({"protocol":1,"workspace_id":workspace.id,"host_uid":workspace.host_uid,"mode":workspace.mode,"institution_id":workspace.institution_id,"policy_epoch":workspace.policy_epoch,"name":workspace.name,"workspace_public_key":public_key,"node_id":node_id,"workspace_key_fingerprint":digest(&key.verifying_key().to_bytes()),"challenge_nonce":nonce,"signature":signature,"signature_v2":signature_v2,"capabilities":capabilities,"unsupported":["arbitrary_shell","remote_filesystem","network_filesystem","cross_workspace_release"]});
        // Unsigned, for display only: whether this broker is still saving changes. An older
        // broker sends no `state`, which a client reads as unknown.
        match &self.storage_fault {
            None => hello["state"] = json!("running"),
            Some(fault) => {
                hello["state"] = json!("storage_failed");
                hello["storage"] = json!({
                    "code": fault.code(),
                    "message": fault.stopped_sentence(),
                    "since": fault.at,
                });
            }
        }
        Ok(hello)
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
            "presence_v1",
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
    fn read_workspace_snapshot(&self, actor: &Actor, req: &Request) -> Result<Value> {
        let s = &self.state;
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
        let (invitations, invitations_wire, invitations_total) =
            Self::snapshot_invitations(s, &index, &actor.id, now);
        let mut workspace = json!(s.workspace);
        workspace["host_principal_id"] = json!(host_principal_id(s));
        let principals = self.snapshot_principals(s, &index, host);
        let actor_wire = Self::actor_wire(s, &index, &actor.id);
        // The actor's live runs only (a revoked, expired or superseded run grants nothing),
        // newest first.
        let mut runs: Vec<&Run> = s
            .runs
            .values()
            .filter(|r| {
                r.owner_id == actor.id
                    && !r.revoked
                    && r.expires_at >= now
                    && r.policy_epoch == s.workspace.policy_epoch
            })
            .collect();
        runs.sort_by_key(|r| std::cmp::Reverse(r.expires_at));
        let runs_total = runs.len();
        let runs = within_budget(runs);
        // Other members add references to shared channels: the actor's own come first, then
        // each other owner's in turn.
        let (mut references, others): (Vec<&RemoteReference>, Vec<&RemoteReference>) = s
            .references
            .values()
            .filter(|r| self.reference_authorized(s, actor, r).is_ok())
            .partition(|r| r.owner_id == actor.id);
        references.extend(in_turns(others, |r| r.owner_id.as_str()));
        let references_total = references.len();
        let references = within_budget(references);
        let totals = json!({"invitations": invitations_total, "runs": runs_total, "references": references_total, "teams": teams.len(), "channels": channels.len()});
        let mut snapshot = json!({"workspace":workspace,"protected_channel_ids":[],"actor":actor_wire,"principals":principals,"former_principals":[],"teams":[],"channels":[],"invitations":invitations_wire,"runs":runs,"read_positions":{},"unread":{},"references":references,"totals":totals});
        // Presence is display only: it is in memory, never journaled, and grants nothing.
        snapshot["online_principal_ids"] = json!(self.online_principal_ids(s));
        if host {
            snapshot["usage"] = self.usage(s);
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
        self.fill_places(
            &mut snapshot,
            actor,
            req,
            &index,
            &teams,
            &channels,
            &invitations,
        );
        Ok(snapshot)
    }
    /// Every invitation the actor sees, the wires of those a snapshot lists, and how many there
    /// are in all. An invitee no longer sees an invitation once it has expired (it can never be
    /// accepted); its inviter still does, marked `expired`, until it is pruned. What the invitee
    /// can act on comes first, taking each inviter's newest in turn so no one inviter can crowd
    /// out the rest, then the actor's own live and expired invitations, newest first, all within
    /// one section budget. The full list is returned, not only what is listed, because the
    /// former members it names are all listed ([`Self::fill_places`]).
    fn snapshot_invitations<'s>(
        s: &'s State,
        index: &PeopleIndex<'_>,
        actor_id: &str,
        now: u64,
    ) -> (Vec<&'s Invitation>, Vec<Value>, usize) {
        let mut invitations: Vec<&Invitation> = s
            .invitations
            .values()
            .filter(|i| {
                i.inviter_id == actor_id || (i.principal_id == actor_id && i.expires_at >= now)
            })
            .collect();
        invitations.sort_by_key(|i| {
            (
                i.principal_id != actor_id,
                i.expires_at < now,
                std::cmp::Reverse(i.expires_at),
            )
        });
        let received = invitations
            .iter()
            .take_while(|i| i.principal_id == actor_id)
            .count();
        let sent = invitations.split_off(received);
        let mut invitations = in_turns(invitations, |i| i.inviter_id.as_str());
        invitations.extend(sent);
        let total = invitations.len();
        let wire = within_budget(
            invitations
                .iter()
                .map(|invitation| Self::invitation_wire(s, index, invitation, now))
                .collect(),
        );
        (invitations, wire, total)
    }
    /// The workspace's active members as a snapshot lists them; the host's also marks those
    /// whose account no longer matches ([`Self::stale_principals`]).
    fn snapshot_principals(&self, s: &State, index: &PeopleIndex<'_>, host: bool) -> Vec<Value> {
        let stale = if host {
            self.stale_principals(s)
        } else {
            BTreeSet::new()
        };
        s.principals
            .values()
            .filter(|p| p.active)
            .map(|p| {
                let mut wire = index.principal_wire(p);
                if stale.contains(p.id.as_str()) {
                    wire["account_stale"] = json!(true);
                }
                wire
            })
            .collect()
    }
    /// Fill `snapshot`'s teams and channels, with what hangs off them (`protected_channel_ids`,
    /// `read_positions`, `unread`), and the former members the actor's teams, channels and
    /// invitations name, once everything else is in it.
    ///
    /// Teams and channels take the room everything else leaves under the frame limit. Their
    /// size is not the actor's to choose: any team owner may add any member to their teams and
    /// channels without asking, and each lists every member, so one member with a full share of
    /// teams and channels could otherwise push everyone else's snapshot past the limit. What
    /// does not fit is left out, whole, and counted in `totals`; nothing listed is ever cut
    /// short.
    ///
    /// `former_principals` does not shrink with them: it names every former member any of the
    /// actor's teams, channels or invitations names, listed or not. A resolver matches a former
    /// member's username against that list, and former members may share a username (only
    /// active ones may not), so a list short of one would resolve `@name` to the other.
    #[allow(clippy::too_many_arguments)]
    fn fill_places(
        &self,
        snapshot: &mut Value,
        actor: &Actor,
        req: &Request,
        index: &PeopleIndex<'_>,
        teams: &[&Team],
        channels: &[&Channel],
        invitations: &[&Invitation],
    ) {
        let s = &self.state;
        let former_principals = json!(Self::former_principals(
            s,
            index,
            teams,
            channels,
            invitations,
        ));
        let reserved = json_len(&*snapshot)
            .saturating_add(json_len(&req.id))
            .saturating_add(SNAPSHOT_FRAME_MARGIN)
            .saturating_add(json_len(&former_principals));
        let places = Self::places_within(
            s,
            &actor.id,
            teams,
            channels,
            MAX_FRAME.saturating_sub(reserved),
        );
        let protected = Self::protected_channel_ids(s);
        let protected_channel_ids: Vec<&str> = places
            .channels
            .iter()
            .map(|channel| channel.id.as_str())
            .filter(|channel| protected.contains(channel))
            .collect();
        let (positions, unread) = self.read_state(s, actor, &places.channels);
        snapshot["protected_channel_ids"] = json!(protected_channel_ids);
        snapshot["former_principals"] = former_principals;
        snapshot["teams"] = Value::Array(places.teams_wire);
        snapshot["channels"] = Value::Array(places.channels_wire);
        snapshot["read_positions"] = json!(positions);
        snapshot["unread"] = json!(unread);
    }
    /// Of the teams and channels the actor is in, those that fit in `room` serialized bytes,
    /// with their wires, each in state order. Nothing is cut short: a team or channel is listed
    /// whole or left out.
    ///
    /// Who gets the room first: what the actor created (their teams) or owns (their channels),
    /// then the host's, then every other owner's, taking turns so that no one owner can crowd
    /// the rest out; within a turn a team comes before a channel. A channel comes with its team
    /// when the actor is in that team, so it can be shown where it belongs. Something that does
    /// not fit is skipped and what follows is still tried, so one large team or channel never
    /// stops the smaller ones after it.
    fn places_within<'s>(
        s: &'s State,
        actor_id: &str,
        teams: &[&'s Team],
        channels: &[&'s Channel],
        room: usize,
    ) -> Places<'s> {
        #[derive(Clone, Copy)]
        enum Place {
            Team(usize),
            Channel(usize),
        }
        // Every team and channel the actor is in, before any is left out, so that each listed
        // one's `name_conflict` still counts those that are not: a resolver holding only the
        // listed ones reads it to tell whether a name it found is the only one of its kind.
        let teams_wire = Self::teams_wire(teams);
        let channels_wire = Self::channels_wire(channels);
        let team_cost: Vec<usize> = teams_wire.iter().map(|wire| json_len(wire) + 1).collect();
        // A channel also has its `read_positions` entry (an opaque message token, or null), its
        // `unread` count and perhaps its `protected_channel_ids` entry, each keyed by its ID.
        let channel_cost: Vec<usize> = channels
            .iter()
            .zip(&channels_wire)
            .map(|(channel, wire)| json_len(wire) + 1 + 3 * (json_len(&channel.id) + 2) + 96)
            .collect();
        let team_of: BTreeMap<&str, usize> = teams
            .iter()
            .enumerate()
            .map(|(position, team)| (team.id.as_str(), position))
            .collect();
        let host = host_principal_id(s);
        let owner = |place: &Place| -> &'s str {
            match *place {
                Place::Team(position) => teams[position].created_by.as_str(),
                Place::Channel(position) => channels[position].owner_id.as_str(),
            }
        };
        let rank = |place: &Place| {
            let owner = owner(place);
            if owner == actor_id {
                0
            } else if Some(owner) == host {
                1
            } else {
                2
            }
        };
        // Teams first, so the stable sort keeps a team ahead of a channel of the same rank,
        // and each owner's own turns do too.
        let mut order: Vec<Place> = (0..teams.len())
            .map(Place::Team)
            .chain((0..channels.len()).map(Place::Channel))
            .collect();
        order.sort_by_key(rank);
        let others = order.split_off(
            order
                .iter()
                .position(|place| rank(place) == 2)
                .unwrap_or(order.len()),
        );
        order.extend(in_turns(others, owner));
        let mut room = room;
        let mut kept_teams = vec![false; teams.len()];
        let mut kept_channels = vec![false; channels.len()];
        for place in order {
            match place {
                Place::Team(position) => {
                    if !kept_teams[position] && team_cost[position] <= room {
                        room -= team_cost[position];
                        kept_teams[position] = true;
                    }
                }
                Place::Channel(position) => {
                    let team = team_of
                        .get(channels[position].team_id.as_str())
                        .copied()
                        .filter(|team| !kept_teams[*team]);
                    let cost = channel_cost[position] + team.map_or(0, |team| team_cost[team]);
                    if cost <= room {
                        room -= cost;
                        kept_channels[position] = true;
                        if let Some(team) = team {
                            kept_teams[team] = true;
                        }
                    }
                }
            }
        }
        let mut places = Places {
            teams: Vec::new(),
            teams_wire: Vec::new(),
            channels: Vec::new(),
            channels_wire: Vec::new(),
        };
        for ((team, wire), kept) in teams.iter().zip(teams_wire).zip(kept_teams) {
            if kept {
                places.teams.push(*team);
                places.teams_wire.push(wire);
            }
        }
        for ((channel, wire), kept) in channels.iter().zip(channels_wire).zip(kept_channels) {
            if kept {
                places.channels.push(*channel);
                places.channels_wire.push(wire);
            }
        }
        places
    }
    /// Per listed channel, the actor's read watermark (as an opaque message token) and the
    /// number of unread messages from others.
    fn read_state(
        &self,
        s: &State,
        actor: &Actor,
        channels: &[&Channel],
    ) -> (BTreeMap<String, Value>, BTreeMap<String, usize>) {
        let mut positions = BTreeMap::new();
        let mut unread = BTreeMap::new();
        for channel in channels {
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
    /// owner and pending owner of the teams and channels the actor is in, and the invitee and
    /// inviter of the invitations the actor sees, whether or not the snapshot lists them.
    /// Bounded by the number of former members.
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
    /// `display_name`, the `handle` a resolver matches against, `name_conflict` (another of
    /// `teams`, every team **the viewer is in**, has the same name, listed or not) and
    /// `name_invalid` (a legacy name the current rules refuse).
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
    /// another of `channels`, every channel the viewer is in, **in the same team**, listed or
    /// not. A stored name that is not a canonical slug (a legacy `Data Analysis`) is
    /// `name_invalid`.
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
        let latest = p.get("latest").and_then(Value::as_bool) == Some(true);
        // Nearest the page's anchor first: the newest for a latest window, the oldest after a
        // cursor.
        let mut messages: Vec<_> = if latest {
            matching.rev().take(limit).collect()
        } else {
            matching.take(limit).collect()
        };
        // An agent's page is bounded in bytes as well: every agent task starts by reading its
        // destination's newest 50, and what other members post there must never push that
        // past the frame limit, or no one's agent could start in the channel. The messages
        // nearest the anchor that fit are kept and `truncated` says the rest were left out: a
        // latest window continues `before` its first message, any other page after `cursor`.
        // A person's page is not cut short, since a client offers an older page only when the
        // one it has is full: past the frame limit the request is refused `response_too_large`,
        // and the client asks for fewer.
        let mut truncated = false;
        if actor.run.is_some() {
            let asked = messages.len();
            messages = within(messages, WORKER_HISTORY_BYTES);
            truncated = messages.len() < asked;
        }
        if latest {
            messages.reverse();
        }
        let cursor = messages
            .last()
            .map(|message| message.id.clone())
            .or_else(|| p.get("after").and_then(Value::as_str).map(str::to_owned));
        let (people, channel_names) = self.message_names(s, actor, messages.iter().copied());
        let messages: Vec<_> = messages.into_iter().map(Self::message_wire).collect();
        let mut page = json!({"messages":messages,"cursor":cursor,"people":people,"channel_names":channel_names});
        if truncated {
            page["truncated"] = json!(true);
        }
        Ok(page)
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
        // Decided over all 200, before the byte budget drops any.
        let restricted = messages.iter().any(|message| message.restricted);
        // Every agent turn asks for this manifest, so what other members post in a source
        // channel must never push it past the frame limit: the newest messages that fit.
        let messages = within(messages, CONTEXT_MANIFEST_BYTES);
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
        let account = self.directory.by_uid(uid)?;
        // Q2-14: the legacy token path refuses the server's own accounts exactly as the
        // invitation by name does, before anything is recorded.
        refuse_system_account(self, &account)?;
        let current_username = account.name;
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
        let requested_mode = mode(p, "mode")?;
        let mut institution = s.workspace.institution_id.clone();
        if let Some(value) = p.get("institution_id") {
            let requested: Option<String> =
                serde_json::from_value(value.clone()).map_err(|_| {
                    anyhow!("invalid_params: institution_id must be a canonical string or null")
                })?;
            if let Some(institution) = &requested {
                ensure!(is_canonical_institution_id(institution), "invalid_params: institution_id must be 1..64 lowercase ASCII letters, digits, underscores or hyphens, starting with a letter or digit");
            }
            ensure!(s.workspace.institution_id.is_none() || s.workspace.institution_id == requested, "privacy_denied: workspace institution cannot be cleared or changed; use a new workspace");
            institution = requested;
        }
        // Re-sending the policy the workspace already has changes nothing: the epoch moves,
        // and every grant ends, only when the mode or the institution really changes. A host
        // who re-runs a setup step must not silently end every member's agent access.
        if requested_mode == s.workspace.mode && institution == s.workspace.institution_id {
            return Ok(json!(s.workspace));
        }
        s.workspace.institution_id = institution;
        s.workspace.mode = requested_mode;
        s.workspace.policy_epoch += 1;
        for run in s.runs.values_mut() {
            run.revoked = true;
        }
        Ok(json!(s.workspace))
    }
    fn mutate_team_create(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        ensure!(
            s.teams.len() < 100 && s.channels.len() < 1000,
            "quota_exceeded: maximum teams"
        );
        ensure!(
            self.manager(s, who).is_ok()
                || s.teams.values().filter(|t| t.created_by == *who).count()
                    < self.quotas.member_teams,
            "quota_exceeded: You have created as many teams as one member may in this workspace."
        );
        ensure!(
            self.manager(s, who).is_ok()
                || s.channels.values().filter(|c| c.created_by == *who).count()
                    < self.quotas.member_channels,
            "quota_exceeded: You have created as many channels as one member may in this workspace."
        );
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
        ensure!(
            self.manager(s, who).is_ok()
                || s.channels.values().filter(|c| c.created_by == *who).count()
                    < self.quotas.member_channels,
            "quota_exceeded: You have created as many channels as one member may in this workspace."
        );
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
        // One invitation per inviter, invitee and target: inviting again renews it (same ID,
        // a fresh day) rather than adding another to the invitee's snapshot.
        if let Some(existing) = s.invitations.values_mut().find(|i| {
            i.inviter_id == *who
                && i.principal_id == principal
                && i.kind == kind
                && i.target_id == target
        }) {
            existing.expires_at = now() + 86400;
            return Ok(json!(existing));
        }
        ensure!(
            self.manager(s, who).is_ok()
                || s.invitations
                    .values()
                    .filter(|i| i.inviter_id == *who && i.expires_at >= now())
                    .count()
                    < self.quotas.member_live_invitations,
            "quota_exceeded: You have as many invitations waiting as one member may. Wait for some to be accepted or to expire."
        );
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
    ///
    /// Q2-78: a refusal names only what this workspace itself records. A former member is
    /// named by the username the workspace holds for them; an ID it has never held is named by
    /// nothing, so the caller's own `expected_username` is never echoed back as if the
    /// workspace had confirmed it.
    fn direct_add_target(&self, s: &State, p: &Value, principal_id: &str) -> Result<String> {
        let expected = text(p, "expected_username")?;
        let expected = expected.strip_prefix('@').unwrap_or(expected);
        let target = match s.principals.get(principal_id) {
            Some(principal) if principal.active => principal,
            Some(former) => bail!(
                "forbidden: @{} isn't a member of this workspace any more. Invite them to the workspace first.",
                former.username
            ),
            None => bail!(DIRECT_ADD_UNKNOWN_PERSON),
        };
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
    /// `channel.archive`, `channel.transfer` and `membership.revoke`: the channel's current
    /// owner archives it, offers it to another member, or removes a member.
    ///
    /// Two others may remove. A member leaves a channel they do not own by removing themselves
    /// ([`Self::is_leave`]), archived or not: a team owner may add any member to their channels
    /// without asking, and leaving is how that member undoes it. And a channel whose owner is no
    /// longer an active member of the workspace has nobody left to manage it, so the host
    /// archives it or removes its members, from outside it too
    /// ([`Self::stewards_orphaned_channel`]). Nobody removes a channel's owner, the host never
    /// overrides an owner who is still a member, and only the owner transfers.
    fn mutate_channel_archive(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let method = req.method.as_str();
        let channel = text(p, "channel_id")?;
        let stewarding = self.stewards_orphaned_channel(s, actor, req);
        let c = if stewarding {
            s.channels
                .get(channel)
                .ok_or_else(|| anyhow!("forbidden: channel unavailable"))?
        } else {
            self.channel(s, who, channel, false)?
        };
        let owner = c.owner_id == *who;
        let leaving = !owner && Self::is_leave(req, who);
        ensure!(
            leaving || !c.archived,
            "channel_archived: channel is read-only"
        );
        ensure!(
            owner || leaving || stewarding,
            "forbidden: current owner required"
        );
        match method {
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
                ensure!(
                    target != c.owner_id,
                    "forbidden: transfer before owner removal"
                );
                check_expected_username(s, p, target)?;
            }
            _ => {}
        }
        let c = s.channels.get_mut(channel).expect("authorized channel");
        match method {
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
        let result = json!(c);
        if method == "membership.revoke" && !owner {
            // Someone who leaves, or whom the host removes, loses only their own invitations
            // to the channel: nobody else's pending invitation is theirs to cancel.
            let target = text(p, "principal_id")?;
            s.invitations
                .retain(|_, i| i.target_id != channel || i.principal_id != target);
        } else {
            s.invitations.retain(|_, i| i.target_id != channel);
        }
        s.workspace.policy_epoch += 1;
        Ok(result)
    }
    /// Whether `req` removes the actor from a channel: `membership.revoke` naming the actor's
    /// own principal. For anyone but the channel's owner that is leaving it.
    fn is_leave(req: &Request, actor_id: &str) -> bool {
        req.method == "membership.revoke"
            && req.params.get("principal_id").and_then(Value::as_str) == Some(actor_id)
    }
    /// Whether `actor` is the host, as a person, archiving (`channel.archive`) or removing a
    /// member from (`membership.revoke`) a channel whose owner is no longer an active member of
    /// the workspace, and so can act on it without being in it. An offboarded owner's channels
    /// would otherwise stay as they are for good, with every member someone added to them.
    fn stewards_orphaned_channel(&self, s: &State, actor: &Actor, req: &Request) -> bool {
        matches!(req.method.as_str(), "channel.archive" | "membership.revoke")
            && actor.run.is_none()
            && self.manager(s, &actor.id).is_ok()
            && req
                .params
                .get("channel_id")
                .and_then(Value::as_str)
                .and_then(|channel| s.channels.get(channel))
                .is_some_and(|channel| {
                    channel.owner_id != actor.id
                        && !s
                            .principals
                            .get(&channel.owner_id)
                            .is_some_and(|owner| owner.active)
                })
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
        ensure!(
            body.len() <= 65536 && json_len(body) - 2 <= MESSAGE_ESCAPED_BYTES,
            "invalid_params: message too long"
        );
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
        let remote_root = match p.get("remote_root") {
            None | Some(Value::Null) => None,
            Some(_) => Some(display_text(p, "remote_root", 4096)?.to_owned()),
        };
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
            provider_policy_id: display_text(p, "provider_policy_id", 1024)?.into(),
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
        let run_id = text(p, "run_id")?;
        match s.runs.get_mut(run_id) {
            Some(run) if run.owner_id == *who => {
                run.revoked = true;
                Ok(json!(run))
            }
            // Retention removed it, a day after it expired ([`RemovedRuns`]). Nothing honors
            // it, so it is revoked, and its owner is told so however long after it ended they
            // ask. Anyone else is answered as for a run that never existed.
            None if self.removed_runs.owned_by(run_id, who) => {
                Ok(json!({"id": run_id, "owner_id": who, "revoked": true, "removed": true}))
            }
            _ => bail!("forbidden: owned run unavailable"),
        }
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
        let path = display_text(p, "path", 4096).map_err(|_| {
            anyhow!("invalid_params: absolute remote path without parent traversal required; at most 4096 bytes, without control or invisible formatting characters")
        })?;
        ensure!(
            Path::new(path).is_absolute()
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
        ensure!(
            self.manager(s, who).is_ok()
                || s.references.values().filter(|r| r.owner_id == *who).count()
                    < self.quotas.member_references,
            "quota_exceeded: You have created as many remote references as one member may."
        );
        let label = display_text(p, "label", 255)
            .map_err(|_| anyhow!("invalid_params: reference label"))?;
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
            size <= BLOB_MAX_BYTES,
            "quota_exceeded: maximum attachment is 1 GiB"
        );
        // Declared sizes count, finished or not: an unfinished upload is a reservation, and it
        // lapses a day after its last chunk (`prune_retained`).
        ensure!(
            s.blobs.len() < WORKSPACE_BLOBS
                && s.blobs.values().map(|b| b.size).sum::<u64>() + size <= WORKSPACE_BLOB_BYTES,
            "quota_exceeded: workspace attachment quota"
        );
        if self.manager(s, who).is_err() {
            let (count, bytes) = s
                .blobs
                .values()
                .filter(|b| b.owner_id == *who)
                .fold((0usize, 0u64), |(count, bytes), b| {
                    (count + 1, bytes + b.size)
                });
            ensure!(
                count < self.quotas.member_blobs
                    && bytes + size <= self.quotas.member_blob_bytes,
                "quota_exceeded: You have used your share of this workspace's attachment space. Unfinished uploads free their space a day after their last progress."
            );
        }
        let sha = text(p, "sha256")?;
        ensure!(
            sha.len() == 64 && hex::decode(sha)?.len() == 32,
            "invalid_params: sha256"
        );
        let name = display_text(p, "name", 255)
            .map_err(|_| anyhow!("invalid_params: attachment display name"))?;
        let media_type = text(p, "media_type")?;
        ensure!(
            media_type.len() <= 255 && media_type.bytes().all(|b| (0x20..0x7f).contains(&b)),
            "invalid_params: media_type must be 1 to 255 printable ASCII characters"
        );
        let blob = Blob {
            touched_at: Some(now()),
            run_id: actor.run.as_ref().map(|r| r.id.clone()),
            id: id(),
            owner_id: who.clone(),
            channel_id: channel.into(),
            name: name.into(),
            media_type: media_type.into(),
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
        blob.touched_at = Some(now());
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

/// `items` reordered so each group (by `group`) takes a turn: every group's first item, then
/// every group's second, and so on, groups in order of first appearance and each group in its
/// own order. A budget applied afterwards then shares its room among the groups.
fn in_turns<T, K: Ord>(items: Vec<T>, group: impl Fn(&T) -> K) -> Vec<T> {
    let mut seen: BTreeMap<K, usize> = BTreeMap::new();
    let mut ranked: Vec<(usize, usize, T)> = items
        .into_iter()
        .enumerate()
        .map(|(position, item)| {
            let turn = seen.entry(group(&item)).or_default();
            *turn += 1;
            (*turn, position, item)
        })
        .collect();
    ranked.sort_by_key(|(turn, position, _)| (*turn, *position));
    ranked.into_iter().map(|(_, _, item)| item).collect()
}
/// The leading `items` whose JSON fits in [`SNAPSHOT_SECTION_BYTES`] (at least the first one),
/// so a snapshot section other members can grow never pushes the snapshot past the frame
/// limit. The snapshot's `totals` says how many there were.
fn within_budget<T: Serialize>(items: Vec<T>) -> Vec<T> {
    within(items, SNAPSHOT_SECTION_BYTES)
}
/// The leading `items` whose JSON fits in `budget` bytes (at least the first one).
fn within<T: Serialize>(items: Vec<T>, budget: usize) -> Vec<T> {
    let mut used: usize = 0;
    items
        .into_iter()
        .enumerate()
        .take_while(|(index, item)| {
            used = used.saturating_add(json_len(item)).saturating_add(1);
            *index == 0 || used <= budget
        })
        .map(|(_, item)| item)
        .collect()
}

/// Remove, from the state a mutation is about to commit, what no longer serves anyone:
/// idempotency results older than [`Quotas::dedupe_ttl_secs`] (and any cached before results
/// carried a time), unfinished uploads untouched for [`BLOB_UPLOAD_TTL_SECS`], invitations
/// expired more than [`INVITATION_RETENTION_SECS`] ago, and runs, with their grants, expired
/// more than [`RUN_RETENTION_SECS`] ago. At most [`PRUNE_BATCH`] of each per call, so one
/// journal record stays small however much aged out at once.
fn prune_retained(s: &mut State, now: u64, quotas: &Quotas) -> Pruned {
    fn aged(at: Option<u64>, now: u64, ttl: u64) -> bool {
        at.is_none_or(|at| now.saturating_sub(at) >= ttl)
    }
    let dedupe_ttl = quotas.dedupe_ttl_secs;
    let stale: Vec<String> = s
        .dedupe
        .iter()
        .filter(|(_, cached)| aged(cached.at, now, dedupe_ttl))
        .map(|(key, _)| key.clone())
        .take(PRUNE_BATCH)
        .collect();
    for key in stale {
        s.dedupe.remove(&key);
    }
    let uploads: Vec<String> = s
        .blobs
        .values()
        .filter(|blob| !blob.complete && aged(blob.touched_at, now, BLOB_UPLOAD_TTL_SECS))
        .map(|blob| blob.id.clone())
        .take(PRUNE_BATCH)
        .collect();
    for id in &uploads {
        s.blobs.remove(id);
    }
    let invitations: Vec<String> = s
        .invitations
        .values()
        .filter(|i| i.expires_at.saturating_add(INVITATION_RETENTION_SECS) <= now)
        .map(|i| i.id.clone())
        .take(PRUNE_BATCH)
        .collect();
    for id in invitations {
        s.invitations.remove(&id);
    }
    let runs: BTreeMap<String, String> = s
        .runs
        .values()
        .filter(|run| run.expires_at.saturating_add(RUN_RETENTION_SECS) <= now)
        .map(|run| (run.id.clone(), run.owner_id.clone()))
        .take(PRUNE_BATCH)
        .collect();
    if !runs.is_empty() {
        s.runs.retain(|id, _| !runs.contains_key(id));
        s.grants.retain(|_, run| !runs.contains_key(run));
    }
    Pruned { uploads, runs }
}
/// What [`prune_retained`] removed that outlives the state.
struct Pruned {
    /// Unfinished uploads, whose files are deleted once the commit lands.
    uploads: Vec<String>,
    /// Runs, by ID, with their owners (see [`RemovedRuns`]).
    runs: BTreeMap<String, String>,
}

/// Cache `cached` under `key` for `actor`, first dropping the actor's oldest results so it
/// keeps at most [`Quotas::dedupe_actor_entries`] in [`Quotas::dedupe_actor_bytes`] (the new
/// result always stays), then the workspace's oldest beyond [`Quotas::dedupe_entries`]. A
/// retry comes within seconds or minutes, long before hundreds of newer results; bounding
/// the cache is what keeps it from ever becoming a cap on the workspace. The actor's agents'
/// results go before the person's own, so a burst of agent projections never evicts the
/// message the person may be about to retry.
fn remember(s: &mut State, quotas: &Quotas, actor: &str, key: String, cached: Cached) {
    let size = |key: &str, cached: &Cached| json_len(key) + json_len(cached);
    let prefix = format!("{actor}:");
    let human = format!("{actor}:human:");
    let mut own: Vec<(bool, Option<u64>, String, usize)> = s
        .dedupe
        .range(prefix.clone()..)
        .take_while(|(k, _)| k.starts_with(&prefix))
        .map(|(k, c)| (k.starts_with(&human), c.at, k.clone(), size(k, c)))
        .collect();
    own.sort();
    let mut count = own.len();
    let mut bytes = own
        .iter()
        .fold(0usize, |total, (_, _, _, size)| total.saturating_add(*size));
    let incoming = size(&key, &cached);
    for (_, _, old, size) in own {
        if count < quotas.dedupe_actor_entries
            && bytes.saturating_add(incoming) <= quotas.dedupe_actor_bytes
        {
            break;
        }
        s.dedupe.remove(&old);
        count -= 1;
        bytes = bytes.saturating_sub(size);
    }
    if s.dedupe.len() >= quotas.dedupe_entries {
        let mut all: Vec<(Option<u64>, String)> =
            s.dedupe.iter().map(|(k, c)| (c.at, k.clone())).collect();
        all.sort();
        // Down to 99% of the limit at once, so the sort runs once per hundredth of the
        // limit rather than on every mutation.
        let target = quotas.dedupe_entries.max(1) - quotas.dedupe_entries / 100 - 1;
        let excess = s.dedupe.len().saturating_sub(target);
        for (_, old) in all.into_iter().take(excess) {
            s.dedupe.remove(&old);
        }
    }
    s.dedupe.insert(key, cached);
}

/// The serialized bytes of the records `principal` created and holds against
/// [`Quotas::member_state_bytes`]: their messages (their agents' included), runs with their
/// grants, attachments, references, invitations sent, and the teams and channels they
/// created. Idempotency results and read positions are bounded separately.
fn member_state_bytes(s: &State, principal: &str) -> usize {
    fn total<T: Serialize>(items: impl Iterator<Item = T>) -> usize {
        items.fold(0, |sum, item| sum.saturating_add(json_len(&item)))
    }
    let runs = s.runs.values().filter(|r| r.owner_id == principal);
    // Each run also holds one grant: a 64-hex digest naming its ID.
    let grants = runs.clone().count().saturating_mul(110);
    [
        total(s.messages.iter().filter(|m| m.actor_id == principal)),
        total(runs),
        grants,
        total(s.blobs.values().filter(|b| b.owner_id == principal)),
        total(s.references.values().filter(|r| r.owner_id == principal)),
        total(s.invitations.values().filter(|i| i.inviter_id == principal)),
        total(s.teams.values().filter(|t| t.created_by == principal)),
        total(s.channels.values().filter(|c| c.created_by == principal)),
    ]
    .into_iter()
    .fold(0, usize::saturating_add)
}

/// `p[key]` as display text of 1 to `max_bytes` bytes with no control, format, line or
/// paragraph separator characters: nothing that hides, reorders or spoofs the text around it,
/// and nothing JSON escapes to more than two bytes, so its stored size is bounded too.
fn display_text<'a>(p: &'a Value, key: &str, max_bytes: usize) -> Result<&'a str> {
    use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};
    let value = text(p, key)?;
    ensure!(
        value.len() <= max_bytes
            && !value.chars().any(|c| matches!(
                c.general_category(),
                GeneralCategory::Control
                    | GeneralCategory::Format
                    | GeneralCategory::LineSeparator
                    | GeneralCategory::ParagraphSeparator
            )),
        "invalid_params: {key} must be 1 to {max_bytes} bytes without control or invisible formatting characters"
    );
    Ok(value)
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

/// The teams and channels a snapshot lists ([`Broker::places_within`]), with their wires.
struct Places<'s> {
    teams: Vec<&'s Team>,
    teams_wire: Vec<Value>,
    channels: Vec<&'s Channel>,
    channels_wire: Vec<Value>,
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
    runtime_root_of(path)?;
    check_runtime_socket(path, owner)
}
/// The node-local temporary directory (`/tmp`) a runtime socket path's directory sits
/// directly in, or `unsafe_socket` when the path is not shaped like one.
fn runtime_root_of(path: &Path) -> Result<&Path> {
    ensure!(
        path.is_absolute(),
        "unsafe_socket: absolute socket path required"
    );
    let root = path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("unsafe_socket"))?;
    ensure!(
        root == Path::new("/tmp") || root == Path::new("/private/tmp"),
        "unsafe_socket: runtime must be dedicated node-local temporary directory"
    );
    Ok(root)
}
/// `path` is a socket `owner` owns, in a directory (not a symbolic link) `owner` owns that no
/// other account can write into. Only `owner` and root can make either.
fn check_runtime_socket(path: &Path, owner: u32) -> Result<()> {
    let parent = path.parent().ok_or_else(|| anyhow!("unsafe_socket"))?;
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
/// Whether `name` is shaped like a runtime directory of `uid`: `crew-<uid>-` and 32 lowercase
/// hex digits. Any account can create an entry with such a name in `/tmp`; the shape says
/// nothing about who did.
fn runtime_basename_of(name: &str, uid: u32) -> bool {
    name.strip_prefix(&format!("crew-{uid}-"))
        .is_some_and(|suffix| {
            suffix.len() == 32
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}
/// The workspace names running sibling brokers of `uid` answer `hello` with: every
/// `crew-<uid>-<32 hex>/broker.sock` under `runtime_root` (at most [`SIBLING_PROBE_LIMIT`])
/// whose directory and socket `uid` owns and whose listener runs as `uid`, except `own_basename`
/// and a broker answering with `own_workspace_id`. Best effort: a stale, slow or unexpected
/// socket is skipped, each probe is bounded by [`SIBLING_PROBE_TIMEOUT`], and nothing is
/// trusted beyond the name used to refuse a duplicate. Names are what any node user can
/// already learn from `hello`.
///
/// Ownership is checked **before** the probe limit is applied: any account can create entries
/// named `crew-<uid>-…` in `/tmp`, and decoys that sort first must never take the probe slots
/// of the account's real runtime directories.
fn sibling_workspace_names(
    runtime_root: &Path,
    uid: u32,
    own_basename: Option<&str>,
    own_workspace_id: &str,
) -> Vec<String> {
    let Ok(entries) = fs::read_dir(runtime_root) else {
        return Vec::new();
    };
    let mut candidates: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| runtime_basename_of(name, uid) && Some(name.as_str()) != own_basename)
        .filter(|name| owned_runtime_socket(&runtime_root.join(name), uid))
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
/// Whether `directory` is a directory `uid` owns (not a symlink) holding a `broker.sock` socket
/// `uid` owns. Metadata only: nothing is opened or connected.
fn owned_runtime_socket(directory: &Path, uid: u32) -> bool {
    use std::os::unix::fs::FileTypeExt;
    let (Ok(dir), Ok(socket)) = (
        fs::symlink_metadata(directory),
        fs::symlink_metadata(directory.join("broker.sock")),
    ) else {
        return false;
    };
    dir.is_dir() && dir.uid() == uid && socket.file_type().is_socket() && socket.uid() == uid
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
            // An empty descriptor records nothing: older `status` and `stop` created one when
            // runtime.json was missing, and the first start must not read it as corrupt.
            if metadata.len() == 0 {
                return Ok(None);
            }
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
                directory.parent() == Some(broker.runtime_root.as_path()),
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
    match reclaim_runtime_socket(&broker.runtime_root, uid, &basename)? {
        RuntimeDirectory::Ready(socket) => Ok(socket),
        RuntimeDirectory::Unusable(reason) => relocate_runtime(broker, uid, &basename, reason),
    }
}
/// What [`reclaim_runtime_socket`] found at the recorded runtime path.
#[derive(Debug)]
enum RuntimeDirectory {
    /// The socket path, in a directory this account owns with mode 0711, free to bind.
    Ready(PathBuf),
    /// The path cannot be used as it stands: another account's entry, a symbolic link or other
    /// non-directory, a directory other accounts can write into, or an unexpected entry. After
    /// `/tmp` is cleaned any account can create an entry at the recorded name, and a sticky
    /// `/tmp` lets only that account and root remove it, so none of these is repaired in place.
    /// The reason is logged. (This account's own directory with another owner-only mode is
    /// repaired in place: nobody else can have written into it.)
    Unusable(&'static str),
}
/// Move the workspace to a fresh runtime directory when its recorded one cannot be used
/// ([`RuntimeDirectory::Unusable`]). The new name is random and created exclusively, so nobody
/// can have claimed it first, and it is journaled before the broker binds. Members' saved
/// connections, and the old invitation line, still name the old path: a bridge of this version
/// or later finds the new directory itself ([`moved_workspace`]), and a member with an older
/// `biorouter-crew` updates it.
fn relocate_runtime(
    broker: &mut Broker,
    uid: u32,
    previous: &str,
    reason: &str,
) -> Result<PathBuf> {
    // A descriptor naming the old path must not outlive the move, or a broker stopped between
    // the journal record and its new descriptor would find the two disagreeing at next start.
    match fs::remove_file(broker.root.join("runtime.json")) {
        Ok(()) => sync_dir(&broker.root)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).context("unsafe_runtime: cannot retire runtime descriptor")
        }
    }
    for _ in 0..8 {
        let basename = format!("crew-{uid}-{}", Uuid::new_v4().simple());
        let directory = broker.runtime_root.join(&basename);
        match fs::DirBuilder::new().mode(0o700).create(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o711))?;
        let metadata = fs::symlink_metadata(&directory)?;
        ensure!(
            metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o7777 == 0o711,
            "unsafe_runtime: new runtime directory ownership, type or permissions invalid"
        );
        let mut state = broker.state.clone();
        state.runtime_basename = Some(basename);
        broker.commit(state, "system", "workspace.move_runtime")?;
        let socket = directory.join("broker.sock");
        eprintln!(
            "runtime_moved: the recorded runtime directory {previous} can't be used ({reason}); this workspace now listens at {}. Members keep their connections: a biorouter-crew of this version or later in their account finds the new directory, and anyone whose Crew then can't connect updates ~/.local/bin/biorouter-crew. A saved connection is never re-pinned by pasting a new invitation line, so members don't need one.",
            socket.display()
        );
        return Ok(socket);
    }
    bail!("unsafe_runtime: could not create a new runtime directory")
}
/// Check (and prepare) the recorded runtime directory `runtime_root/basename`: create it when
/// it is missing, and remove a stale socket this account left behind. A live listener is an
/// error (`runtime_in_use`); anything that makes the path unusable is
/// [`RuntimeDirectory::Unusable`].
fn reclaim_runtime_socket(
    runtime_root: &Path,
    uid: u32,
    basename: &str,
) -> Result<RuntimeDirectory> {
    use RuntimeDirectory::Unusable;
    let directory = runtime_root.join(basename);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if !metadata.is_dir() => {
            return Ok(Unusable("it is not a directory"));
        }
        Ok(metadata) if metadata.uid() != uid => {
            return Ok(Unusable("another account owns it"));
        }
        Ok(metadata) if metadata.mode() & 0o022 != 0 => {
            return Ok(Unusable("other accounts can write into it"));
        }
        Ok(metadata) if metadata.mode() & 0o7777 != 0o711 => {
            // This account's own directory, which no other account could ever write into (only
            // its owner and root can change its mode): nothing in it can be another account's,
            // so its mode is put back rather than the workspace moved, and members keep the
            // path they were given.
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o711))?;
            let repaired = fs::symlink_metadata(&directory)?;
            ensure!(
                repaired.is_dir()
                    && repaired.uid() == uid
                    && repaired.mode() & 0o7777 == 0o711
                    && repaired.dev() == metadata.dev()
                    && repaired.ino() == metadata.ino(),
                "unsafe_runtime: runtime directory changed while its permissions were repaired"
            );
            eprintln!(
                "runtime_repaired: {} had mode {:o}; set it back to 711",
                directory.display(),
                metadata.mode() & 0o7777
            );
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::DirBuilder::new().mode(0o700).create(&directory) {
                Ok(()) => {}
                // Created by someone else between the two calls.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Ok(Unusable("another account created it"));
                }
                Err(error) => return Err(error.into()),
            }
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o711))?;
        }
        Err(error) => return Err(error.into()),
    }
    for entry in fs::read_dir(&directory)? {
        if entry?.file_name() != "broker.sock" {
            return Ok(Unusable("an unexpected entry occupies it"));
        }
    }
    let socket = directory.join("broker.sock");
    match fs::symlink_metadata(&socket) {
        Ok(metadata) => {
            use std::os::unix::fs::FileTypeExt;
            if !(metadata.file_type().is_socket()
                && metadata.uid() == uid
                && metadata.mode() & 0o7777 == 0o666)
            {
                return Ok(Unusable(
                    "its socket's ownership, type or permissions changed",
                ));
            }
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
    Ok(RuntimeDirectory::Ready(socket))
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
    let bound = path_identity(&socket)
        .ok_or_else(|| anyhow!("unsafe_runtime: the bound socket disappeared"))?;
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
    watch_runtime(socket.clone(), bound, Arc::clone(&shared));
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
/// How often a running broker checks that its socket path still leads to the socket it bound.
const RUNTIME_WATCH_INTERVAL: Duration = Duration::from_secs(30);
/// The `(device, inode)` of the entry at `path`, without following a final symbolic link, or
/// `None` when it cannot be read.
fn path_identity(path: &Path) -> Option<(u64, u64)> {
    fs::symlink_metadata(path)
        .ok()
        .map(|metadata| (metadata.dev(), metadata.ino()))
}
/// Whether `socket` still leads to the socket the broker bound, identified by `bound`.
fn runtime_intact(socket: &Path, bound: (u64, u64)) -> bool {
    path_identity(socket) == Some(bound)
}
/// Stop the broker once its socket path no longer leads to the socket it bound: the runtime
/// directory was removed (by root or a `/tmp` cleaner) and perhaps re-created by another
/// account. Nobody can connect any more and `stop` can no longer verify the process, while it
/// still holds the writer lock, so it exits and lets the next `start` restore the path or move
/// the workspace to a new one. It waits for the request in progress, so no commit is cut short.
fn watch_runtime(socket: PathBuf, bound: (u64, u64), broker: Arc<Mutex<Broker>>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(RUNTIME_WATCH_INTERVAL);
        if !runtime_intact(&socket, bound) {
            let _held = broker.lock();
            eprintln!(
                "runtime_lost: {} no longer leads to this broker; stopping so the next start can restore or move it",
                socket.display()
            );
            std::process::exit(75);
        }
    });
}
fn serve_client(stream: UnixStream, uid: u32, broker: Arc<Mutex<Broker>>) -> Result<()> {
    let mut connection = Connection::new();
    let served = serve_requests(stream, uid, &broker, &mut connection);
    // However the connection ended, the person it was signed on for is no longer connected
    // through it.
    if let Ok(mut broker) = broker.lock() {
        broker.connection_closed(&mut connection);
    }
    served
}
fn serve_requests(
    mut stream: UnixStream,
    uid: u32,
    broker: &Mutex<Broker>,
    connection: &mut Connection,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(300)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    while let Some(bytes) = read_frame(&mut reader)? {
        let request: Request = serde_json::from_slice(&bytes)?;
        let response = broker
            .lock()
            .map_err(|_| anyhow!("broker unavailable"))?
            .handle(uid, connection, request);
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
/// A connection to `workspace`'s broker, which `owner` runs, with the `hello` it answered: at
/// `socket`, the path the member's saved connection names, or, when that path no longer leads
/// to the workspace, wherever in `root` the broker has moved to ([`moved_workspace`]).
fn open_workspace(
    root: &Path,
    socket: &Path,
    owner: u32,
    workspace: &str,
) -> Result<(UnixStream, BufReader<UnixStream>, Value)> {
    match connect_workspace(socket, owner, workspace, None) {
        Ok(found) => Ok(found),
        Err(error) => {
            let pinned = socket.parent().and_then(Path::file_name);
            moved_workspace(root, owner, workspace, pinned).ok_or(error)
        }
    }
}
/// Connect to the broker at `socket` and check it is `workspace`'s, run by `owner`: the socket
/// and its directory are `owner`'s ([`check_runtime_socket`]), the listener runs as `owner`,
/// and it answers `hello` for `workspace`. `timeout` bounds the `hello`; the connection is
/// returned without one.
fn connect_workspace(
    socket: &Path,
    owner: u32,
    workspace: &str,
    timeout: Option<Duration>,
) -> Result<(UnixStream, BufReader<UnixStream>, Value)> {
    check_runtime_socket(socket, owner)?;
    let mut stream = UnixStream::connect(socket)?;
    ensure!(peer_uid(&stream)? == owner, "identity_mismatch: broker UID");
    stream.set_read_timeout(timeout)?;
    stream.set_write_timeout(timeout)?;
    let mut reader = BufReader::new(stream.try_clone()?);
    stream
        .write_all(b"{\"version\":1,\"id\":\"bridge-pin\",\"method\":\"hello\",\"params\":{}}\n")?;
    let hello: Response = serde_json::from_slice(
        &read_frame(&mut reader)?.ok_or_else(|| anyhow!("broker disconnected"))?,
    )?;
    let hello = hello.result.unwrap_or(Value::Null);
    ensure!(
        hello.get("workspace_id").and_then(Value::as_str) == Some(workspace),
        "identity_mismatch: pinned workspace"
    );
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(None)?;
    Ok((stream, reader, hello))
}
/// `workspace`'s broker in another runtime directory of `owner` in `root`, after it moved away
/// from the one a member's connection was saved with (`pinned`, skipped here).
///
/// A broker moves when its recorded directory cannot be used ([`relocate_runtime`]), typically
/// because another account created an entry at that name after `/tmp` was cleaned, and every
/// saved connection still names the old path. Without this, each member would have to remove
/// their connection and enroll again, since a pasted invitation never re-pins a workspace
/// someone already saved. Only a runtime directory of `owner`'s that nobody else can write
/// into, holding a socket `owner` owns, whose listener runs as `owner` and answers `hello` for
/// `workspace`, is ever used, and no other account can make any of those. The daemon then
/// checks the workspace key's signature on its own `hello`, exactly as for the saved path.
/// As when a start checks its siblings, ownership is checked before at most
/// [`SIBLING_PROBE_LIMIT`] directories are tried, each bounded by [`SIBLING_PROBE_TIMEOUT`], so
/// entries another account creates can neither take those slots nor stall the search.
fn moved_workspace(
    root: &Path,
    owner: u32,
    workspace: &str,
    pinned: Option<&std::ffi::OsStr>,
) -> Option<(UnixStream, BufReader<UnixStream>, Value)> {
    let mut candidates: Vec<PathBuf> = fs::read_dir(root)
        .ok()?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| {
            runtime_basename_of(name, owner) && Some(std::ffi::OsStr::new(name)) != pinned
        })
        .map(|name| root.join(name).join("broker.sock"))
        .filter(|socket| check_runtime_socket(socket, owner).is_ok())
        .collect();
    candidates.sort();
    candidates.truncate(SIBLING_PROBE_LIMIT);
    candidates.into_iter().find_map(|socket| {
        connect_workspace(&socket, owner, workspace, Some(SIBLING_PROBE_TIMEOUT)).ok()
    })
}
pub fn bridge(socket: &Path, owner: u32, workspace: &str) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "unsupported: bridge requires Linux"
    );
    let root = runtime_root_of(socket)?;
    let (stream, reader, _) = open_workspace(root, socket, owner, workspace)?;
    // Standard input is read through a buffer the relay can see into: it waits on the input
    // descriptor only when that buffer is empty.
    use std::os::fd::AsFd;
    let mut input = BufReader::new(File::from(std::io::stdin().as_fd().try_clone_to_owned()?));
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    relay(&mut input, &mut output, stream, reader)
}
/// What a bridge answers a request it could not hand to the broker: nothing of it reached the
/// workspace, so nothing changed and it is safe to send again once reconnected. The member's
/// daemon can then say "not sent" instead of "the outcome may be unknown".
const NOT_DELIVERED: &str = "not_delivered: The workspace server was not reachable, so this request was not sent and nothing changed. Reconnect and try again.";
/// Why a bridge exits when its broker has gone: the member's `ssh` ends with it, which is how
/// the member's daemon learns, within seconds, that the connection dropped.
const BROKER_GONE: &str =
    "broker_unavailable: the workspace server closed the connection; reconnect to continue";
/// Relay frames between the member's daemon (`input`, `output`) and the broker, until the
/// daemon closes its end (`Ok`) or the broker goes away (`Err`).
///
/// It waits on the daemon **and** the broker together. The broker only ever answers, so while
/// no request is in flight anything it signals is a hang-up; the bridge then exits at once
/// (R-5). Waiting on the daemon alone, it noticed a broker that died only when the next
/// request failed, and the member read "Connected" for up to two and a half minutes. A request
/// that could not be written to the broker is answered [`NOT_DELIVERED`] before it exits.
fn relay(
    input: &mut BufReader<File>,
    output: &mut impl Write,
    mut stream: UnixStream,
    mut reader: BufReader<UnixStream>,
) -> Result<()> {
    loop {
        if input.buffer().is_empty() {
            let (input_ready, broker_gone) = wait_for_input(input.get_ref(), &stream)?;
            if broker_gone {
                // A request already waiting was never sent; say so rather than leave it lost.
                if input_ready {
                    if let Some(frame) = read_frame(input)? {
                        write_not_delivered(output, &frame)?;
                    }
                }
                bail!(BROKER_GONE);
            }
        }
        let Some(frame) = read_frame(input)? else {
            return Ok(());
        };
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
            if stream
                .write_all(&frame)
                .and_then(|()| stream.flush())
                .is_err()
            {
                // The frame did not reach the broker whole, and a partial frame is never
                // processed: nothing was sent.
                write_not_delivered(output, &frame)?;
                bail!(BROKER_GONE);
            }
            let response =
                read_frame(&mut reader)?.ok_or_else(|| anyhow!("broker disconnected"))?;
            output.write_all(&response)?;
            output.flush()?;
        }
    }
}
/// Wait until the daemon's `input` has something to read or the broker's `stream` signals
/// anything at all: `(input_ready, broker_gone)`. Readable, hung up or failed, a broker that
/// was asked nothing has gone.
fn wait_for_input(input: &File, stream: &UnixStream) -> Result<(bool, bool)> {
    loop {
        let mut fds = [
            libc::pollfd {
                fd: input.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) } < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        let input_ready = fds[0].revents != 0;
        let broker_gone = fds[1].revents != 0;
        if input_ready || broker_gone {
            return Ok((input_ready, broker_gone));
        }
    }
}
/// Answer the request in `frame` with [`NOT_DELIVERED`], under its own ID.
fn write_not_delivered(output: &mut impl Write, frame: &[u8]) -> Result<()> {
    let id = serde_json::from_slice::<Value>(frame)
        .ok()
        .and_then(|request| request.get("id").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default();
    let mut bytes = serde_json::to_vec(&Response {
        id,
        result: None,
        error: Some(ProtocolError {
            code: "not_delivered".into(),
            message: NOT_DELIVERED.into(),
        }),
    })?;
    bytes.push(b'\n');
    output.write_all(&bytes)?;
    output.flush()?;
    Ok(())
}
/// `start`, `status` or `stop` a broker for the state directory `root`. `name` (`start` only)
/// names the workspace; see [`serve`].
pub fn lifecycle(command: &str, root: &Path, key: &str, name: Option<&str>) -> Result<Value> {
    match command {
        "start" => start(root, key, name),
        "status" => {
            let info = existing_runtime_descriptor(root)?.ok_or_else(|| anyhow!(NOT_RUNNING))?;
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
            let hello = response.result.unwrap_or_default();
            status_answer(root, info, &hello)
        }
        "stop" => stop(root),
        _ => bail!("invalid_command"),
    }
}

/// What `status` prints for the broker `info` describes, which answered `hello`: the runtime
/// descriptor with `"state":"running"` and the workspace's name, or, when the broker has
/// stopped saving changes, a refusal telling the host what to do.
fn status_answer(root: &Path, mut info: Value, hello: &Value) -> Result<Value> {
    if hello.get("state").and_then(Value::as_str) == Some("storage_failed") {
        let full = hello["storage"]["code"].as_str() == Some("storage_full");
        let fault = StorageFault {
            full,
            at: hello["storage"]["since"].as_u64().unwrap_or_default(),
            detail: String::new(),
            committed: 0,
            logged: true,
        };
        let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        bail!(
            "{}: Crew on this server stopped saving changes at {} because {}. Reading still works. {} broker.log in the state directory has the error.",
            fault.code(),
            utc_timestamp(fault.at),
            if full {
                "the disk is full"
            } else {
                "it could not write to its storage"
            },
            fault.host_instruction(&root)
        );
    }
    info["state"] = json!("running");
    info["name"] = hello.get("name").cloned().unwrap_or(Value::Null);
    Ok(info)
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
    // The socket members were given last time, to tell the host when the broker had to move.
    let previous_socket = read_runtime_descriptor(root)
        .ok()
        .flatten()
        .and_then(|info| info.get("socket").cloned());
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
                    if previous_socket
                        .as_ref()
                        .is_some_and(|previous| Some(previous) != info.get("socket"))
                    {
                        // The recorded runtime directory could not be used (see broker.log),
                        // and the workspace moved. A bridge of this version or later follows
                        // it from the old path; an older one needs updating.
                        result["socket_changed"] = json!(true);
                    }
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
/// `status` and `stop` for a state directory that has no runtime descriptor.
const NOT_RUNNING: &str = "not_running: no broker is running from this state directory; start it with biorouter-crew start";
/// `runtime.json` for `status` and `stop`, read without creating anything: `None` when the
/// state directory or the descriptor is missing, or the descriptor is empty (older `status` and
/// `stop` created an empty one). The state directory must be private, and the descriptor a
/// private regular file with one link.
fn existing_runtime_descriptor(root: &Path) -> Result<Option<Value>> {
    match fs::symlink_metadata(root) {
        Ok(metadata) => ensure!(
            metadata.is_dir()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "unsafe_storage: state directory must be owner-only and not a symlink"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut file = match OpenOptions::new()
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
            && metadata.nlink() == 1
            && metadata.len() <= MAX_FRAME as u64,
        "unsafe_runtime: invalid private runtime descriptor"
    );
    if metadata.len() == 0 {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .context("unsafe_runtime: runtime descriptor is corrupt")
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
    let info = existing_runtime_descriptor(root)?.ok_or_else(|| anyhow!(NOT_RUNNING))?;
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

#[cfg(test)]
mod system_account_tests {
    use super::*;

    fn account(uid: u32, shell: Option<&str>) -> Account {
        Account {
            uid,
            name: "someone".into(),
            full_name: None,
            shell: shell.map(str::to_owned),
        }
    }

    #[test]
    fn system_accounts_are_root_low_uids_nobody_and_accounts_without_a_login_shell() {
        for (uid, shell) in [
            (0, Some("/bin/bash")),
            (1, Some("/usr/sbin/nologin")),
            (999, Some("/bin/bash")),
            (65_534, None),
            (65_534, Some("/bin/sh")),
            (u32::MAX, Some("/bin/bash")),
            (1_001, Some("/usr/sbin/nologin")),
            (1_001, Some("/sbin/nologin")),
            (1_001, Some("/bin/false")),
            (1_001, Some("/usr/bin/false")),
        ] {
            assert!(
                is_system_account(&account(uid, shell), 1000),
                "{uid} {shell:?}"
            );
        }
        for (uid, shell) in [
            (1_000, Some("/bin/bash")),
            (1_001, None),
            (71_001, Some("/usr/bin/zsh")),
            (1_001, Some("/opt/false-positive/bin/bash")),
        ] {
            assert!(
                !is_system_account(&account(uid, shell), 1000),
                "{uid} {shell:?}"
            );
        }
        // A node whose login.defs starts people at 500.
        assert!(!is_system_account(&account(600, Some("/bin/bash")), 500));
        assert!(is_system_account(&account(0, Some("/bin/bash")), 0));
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    fn short_root() -> PathBuf {
        let suffix: String = Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(12)
            .collect();
        let path = Path::new("/tmp").join(format!("crt-u-{suffix}"));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        path
    }

    #[test]
    fn another_accounts_runtime_directory_is_unusable_not_an_error() {
        let root = short_root();
        let uid = unsafe { libc::geteuid() };
        let basename = format!("crew-{uid}-{}", Uuid::new_v4().simple());
        // This account's own directory, as it looks to a host of another UID: exactly what a
        // host sees when another account created the recorded name after /tmp was cleaned.
        let directory = root.join(&basename);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o711)).unwrap();
        let foreign = reclaim_runtime_socket(&root, uid.wrapping_add(1), &basename).unwrap();
        assert!(
            matches!(
                foreign,
                RuntimeDirectory::Unusable("another account owns it")
            ),
            "{foreign:?}"
        );
        // Its own host reclaims it.
        let own = reclaim_runtime_socket(&root, uid, &basename).unwrap();
        assert!(
            matches!(&own, RuntimeDirectory::Ready(socket) if *socket == directory.join("broker.sock")),
            "{own:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_watchdog_notices_a_removed_or_replaced_socket() {
        let root = short_root();
        let socket = root.join("broker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let bound = path_identity(&socket).unwrap();
        assert!(runtime_intact(&socket, bound));
        fs::remove_file(&socket).unwrap();
        assert!(!runtime_intact(&socket, bound), "removed");
        let _other = UnixListener::bind(&socket).unwrap();
        assert!(
            !runtime_intact(&socket, bound),
            "replaced by another socket"
        );
        drop(listener);
        let _ = fs::remove_dir_all(&root);
    }

    /// A broker stand-in at `root/basename/broker.sock`, in a directory with `mode`, answering
    /// every `hello` for `workspace_id` with `name`.
    #[cfg(target_os = "linux")]
    fn fake_broker(root: &Path, basename: &str, mode: u32, workspace_id: &str, name: &str) {
        let directory = root.join(basename);
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(mode)).unwrap();
        let listener = UnixListener::bind(directory.join("broker.sock")).unwrap();
        let answer =
            json!({"id": "bridge-pin", "result": {"workspace_id": workspace_id, "name": name}});
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut line = String::new();
                if BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .is_ok()
                {
                    let _ = stream.write_all(format!("{answer}\n").as_bytes());
                }
                // Held open, as a broker holds a bridge's connection.
                std::mem::forget(stream);
            }
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_bridge_follows_its_workspace_to_the_directory_it_moved_to() {
        let root = short_root();
        let uid = unsafe { libc::geteuid() };
        let workspace = Uuid::new_v4().to_string();
        let basename = |fill: char| format!("crew-{uid}-{}", fill.to_string().repeat(32));
        // The saved connection names `a…`, which the broker had to leave (another account took
        // it once /tmp was cleaned); it moved to `f…`. Everything that sorts between the two is
        // something the bridge must never take for the workspace: a directory other accounts
        // can write into, whose listener claims the workspace, and one of this account's
        // other workspaces.
        let pinned = root.join(basename('a')).join("broker.sock");
        fake_broker(&root, &basename('b'), 0o777, &workspace, "writable");
        fake_broker(
            &root,
            &basename('c'),
            0o711,
            &Uuid::new_v4().to_string(),
            "sibling",
        );
        std::os::unix::fs::symlink(root.join(basename('f')), root.join(basename('d'))).unwrap();
        fake_broker(&root, &basename('f'), 0o711, &workspace, "moved");
        let (_, _, hello) = open_workspace(&root, &pinned, uid, &workspace).unwrap();
        assert_eq!(hello["name"], "moved");

        // What stands at the saved path, when it is unusable, changes nothing.
        std::os::unix::fs::symlink(root.join(basename('f')), root.join(basename('a'))).unwrap();
        let (_, _, hello) = open_workspace(&root, &pinned, uid, &workspace).unwrap();
        assert_eq!(hello["name"], "moved");
        fs::remove_file(root.join(basename('a'))).unwrap();
        fake_broker(&root, &basename('a'), 0o777, &workspace, "squatted");
        let (_, _, hello) = open_workspace(&root, &pinned, uid, &workspace).unwrap();
        assert_eq!(hello["name"], "moved");
        let _ = fs::remove_dir_all(&root);

        // The saved path, while it still leads to the workspace, is used first.
        let root = short_root();
        let pinned = root.join(basename('e')).join("broker.sock");
        fake_broker(&root, &basename('e'), 0o711, &workspace, "saved");
        fake_broker(&root, &basename('a'), 0o711, &workspace, "elsewhere");
        let (_, _, hello) = open_workspace(&root, &pinned, uid, &workspace).unwrap();
        assert_eq!(hello["name"], "saved");
        let _ = fs::remove_dir_all(&root);

        // With nowhere to go, the saved path's own refusal is what the member sees.
        let root = short_root();
        let pinned = root.join(basename('a')).join("broker.sock");
        fake_broker(&root, &basename('b'), 0o777, &workspace, "writable");
        let error = open_workspace(&root, &pinned, uid, &workspace)
            .expect_err("no directory of this account's answers for the workspace");
        assert!(
            error.downcast_ref::<std::io::Error>().is_some(),
            "{error:#}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod status_tests {
    use super::*;

    fn info() -> Value {
        json!({"pid": 42, "socket": "/tmp/crew-1000-x/broker.sock", "workspace_id": "w", "host_uid": 1000})
    }

    /// SF-F7: the restart instructions tell the host to look for `"state":"running"`, so
    /// `status` says it, with the workspace's name, beside the runtime descriptor.
    #[test]
    fn status_says_running_and_names_the_workspace() {
        let root = std::env::temp_dir();
        let answer =
            status_answer(&root, info(), &json!({"state": "running", "name": "lab"})).unwrap();
        assert_eq!(answer["state"], "running");
        assert_eq!(answer["name"], "lab");
        assert_eq!(answer["pid"], 42);
        assert_eq!(answer["socket"], "/tmp/crew-1000-x/broker.sock");
        // A workspace with no name yet says so rather than leaving the key out.
        let unnamed = status_answer(&root, info(), &json!({"state": "running"})).unwrap();
        assert_eq!(unnamed["state"], "running");
        assert!(unnamed["name"].is_null());
    }

    /// R-2: a broker that stopped saving is not reported as healthy. `status` fails, naming the
    /// cause and the two commands that bring the workspace back.
    #[test]
    fn status_of_a_broker_that_stopped_saving_fails_and_says_what_to_run() {
        let root = std::env::temp_dir();
        let canonical = fs::canonicalize(&root).unwrap();
        for (code, cause, first) in [
            (
                "storage_full",
                "the disk is full",
                "Free space on this server",
            ),
            (
                "storage_failed",
                "it could not write to its storage",
                "Check this server's storage",
            ),
        ] {
            let hello = json!({
                "state": "storage_failed",
                "storage": {"code": code, "message": "…", "since": 1_790_000_000u64},
            });
            let error = status_answer(&root, info(), &hello)
                .expect_err("a broker that stopped saving is not running")
                .to_string();
            assert!(error.starts_with(&format!("{code}: ")), "{error}");
            for expected in [
                cause.to_owned(),
                first.to_owned(),
                "2026-09-21T14:13:20Z".to_owned(),
                format!("biorouter-crew stop --state-dir {}", canonical.display()),
                format!("biorouter-crew start --state-dir {}", canonical.display()),
            ] {
                assert!(
                    error.contains(&expected),
                    "{expected:?} missing from {error}"
                );
            }
        }
    }

    #[test]
    fn utc_timestamps_are_civil_dates() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc_timestamp(1_790_000_000), "2026-09-21T14:13:20Z");
        assert_eq!(utc_timestamp(4_107_542_399), "2100-02-28T23:59:59Z");
    }

    /// A raw operating-system error is a storage fault in words; a coded refusal keeps its
    /// code, even when an I/O error is its cause.
    #[test]
    fn a_raw_os_error_is_worded_as_a_storage_fault() {
        let full = protocol_error(&anyhow::Error::from(std::io::Error::from_raw_os_error(
            libc::ENOSPC,
        )));
        assert_eq!(full.code, "storage_full");
        assert_eq!(full.message, STORAGE_FULL_RETRY);
        let quota = protocol_error(&anyhow::Error::from(std::io::Error::from_raw_os_error(
            libc::EDQUOT,
        )));
        assert_eq!(quota.code, "storage_full");
        let failed = protocol_error(&anyhow::Error::from(std::io::Error::from_raw_os_error(
            libc::EIO,
        )));
        assert_eq!(failed.code, "storage_failed");
        assert_eq!(failed.message, STORAGE_FAILED_RETRY);
        let coded = protocol_error(
            &anyhow::Error::from(std::io::Error::from_raw_os_error(libc::EACCES))
                .context("unsafe_storage: cannot read the journal"),
        );
        assert_eq!(coded.code, "unsafe_storage");
        let plain = protocol_error(&anyhow!("forbidden: not yours"));
        assert_eq!(plain.code, "forbidden");
        let uncoded = protocol_error(&anyhow!("Something else"));
        assert_eq!(uncoded.code, "request_denied");
    }

    /// `status` against a real broker on a real socket: running with its name, then, once it
    /// stopped saving, a refusal saying so.
    #[cfg(target_os = "linux")]
    #[test]
    fn status_of_a_live_broker_follows_its_state() {
        let short = |label: &str| {
            let suffix: String = Uuid::new_v4()
                .simple()
                .to_string()
                .chars()
                .take(12)
                .collect();
            let path = Path::new("/tmp").join(format!("crt-{label}-{suffix}"));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            path
        };
        let state = short("s");
        let runtime = short("r");
        let key = hex::encode(SigningKey::from_bytes(&[7; 32]).verifying_key().to_bytes());
        let broker =
            Broker::open_inner(&state, &key, Box::new(SystemDirectory), Some("lab")).unwrap();
        let socket = runtime.join("broker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let info = json!({
            "pid": std::process::id(),
            "socket": socket,
            "workspace_id": broker.workspace().id,
            "host_uid": unsafe { libc::geteuid() },
            "protocol": 1,
        });
        write_runtime(&state, &info).unwrap();
        let shared = Arc::new(Mutex::new(broker));
        let serving = Arc::clone(&shared);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let uid = peer_uid(&stream).unwrap();
                let broker = Arc::clone(&serving);
                std::thread::spawn(move || serve_client(stream, uid, broker));
            }
        });

        let running = lifecycle("status", &state, "", None).unwrap();
        assert_eq!(running["state"], "running");
        assert_eq!(running["name"], "lab");
        assert_eq!(running["workspace_id"], info["workspace_id"]);

        shared.lock().unwrap().storage_fault = Some(StorageFault {
            full: true,
            at: now(),
            detail: "No space left on device (os error 28)".into(),
            committed: 0,
            logged: true,
        });
        let error = lifecycle("status", &state, "", None)
            .expect_err("a broker that stopped saving is not running")
            .to_string();
        assert!(error.starts_with("storage_full: "), "{error}");
        assert!(
            error.contains(&format!(
                "biorouter-crew start --state-dir {}",
                fs::canonicalize(&state).unwrap().display()
            )),
            "{error}"
        );
        assert!(!error.contains("os error"), "{error}");
        let _ = fs::remove_dir_all(&state);
        let _ = fs::remove_dir_all(&runtime);
    }
}

#[cfg(test)]
mod bridge_tests {
    use super::*;
    use std::os::fd::FromRawFd;
    use std::sync::mpsc;

    /// A pipe standing in for the member's `ssh` stdin: the relay's end, and the writer.
    fn stdin_pipe() -> (BufReader<File>, File) {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (read, write) = unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
        (BufReader::new(read), write)
    }

    /// Run the relay against `bridge_side` of a socket pair on its own thread; its result and
    /// everything it wrote to the daemon arrive on the channel.
    fn spawn_relay(
        mut input: BufReader<File>,
        bridge_side: UnixStream,
    ) -> mpsc::Receiver<(Result<()>, Vec<u8>)> {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(bridge_side.try_clone().unwrap());
            let mut output = Vec::new();
            let result = relay(&mut input, &mut output, bridge_side, reader);
            let _ = sender.send((result, output));
        });
        receiver
    }

    fn frame(id: &str) -> Vec<u8> {
        format!("{{\"version\":1,\"id\":\"{id}\",\"method\":\"message.post\",\"params\":{{}}}}\n")
            .into_bytes()
    }

    fn not_delivered(output: &[u8], id: &str) {
        let response: Value = serde_json::from_slice(output).unwrap();
        assert_eq!(response["id"], id);
        assert_eq!(response["error"]["code"], "not_delivered");
        assert_eq!(response["error"]["message"], NOT_DELIVERED);
    }

    /// R-5: nothing in flight, the broker dies, and the bridge exits within seconds, so the
    /// member's `ssh` ends and the daemon re-dials instead of reading "Connected".
    #[test]
    fn an_idle_bridge_exits_as_soon_as_its_broker_hangs_up() {
        let (input, _daemon) = stdin_pipe();
        let (bridge_side, broker_side) = UnixStream::pair().unwrap();
        let finished = spawn_relay(input, bridge_side);
        assert!(
            finished.recv_timeout(Duration::from_millis(300)).is_err(),
            "an idle bridge with a live broker keeps running"
        );
        drop(broker_side);
        let (result, output) = finished
            .recv_timeout(Duration::from_secs(2))
            .expect("the bridge exits within 2 s of its broker hanging up");
        assert_eq!(result.unwrap_err().to_string(), BROKER_GONE);
        assert!(
            output.is_empty(),
            "nothing was asked, so nothing is answered"
        );
    }

    /// A request already waiting when the broker hung up never reached it: it is answered
    /// `not_delivered`, not left to read as lost.
    #[test]
    fn a_request_waiting_when_the_broker_hangs_up_is_answered_not_delivered() {
        let (input, mut daemon) = stdin_pipe();
        let (bridge_side, broker_side) = UnixStream::pair().unwrap();
        drop(broker_side);
        daemon.write_all(&frame("post-1")).unwrap();
        let (result, output) = spawn_relay(input, bridge_side)
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert_eq!(result.unwrap_err().to_string(), BROKER_GONE);
        not_delivered(&output, "post-1");
    }

    /// A request whose frame the broker can no longer take is answered `not_delivered`, and
    /// the bridge exits rather than leaving the daemon to guess the outcome.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_request_the_broker_can_no_longer_take_is_answered_not_delivered() {
        let (input, mut daemon) = stdin_pipe();
        let (bridge_side, broker_side) = UnixStream::pair().unwrap();
        // The broker stops reading without hanging up: the bridge sees nothing until it
        // writes, and the write fails.
        broker_side.shutdown(std::net::Shutdown::Read).unwrap();
        let finished = spawn_relay(input, bridge_side);
        daemon.write_all(&frame("post-2")).unwrap();
        let (result, output) = finished.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(result.unwrap_err().to_string(), BROKER_GONE);
        not_delivered(&output, "post-2");
        drop(broker_side);
    }

    /// The ordinary relay is unchanged: each request is forwarded and answered in turn, and
    /// the bridge ends cleanly when the daemon closes its end.
    #[test]
    fn requests_are_relayed_until_the_daemon_closes_its_end() {
        let (input, mut daemon) = stdin_pipe();
        let (bridge_side, broker_side) = UnixStream::pair().unwrap();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(broker_side.try_clone().unwrap());
            let mut writer = broker_side;
            while let Ok(Some(request)) = read_frame(&mut reader) {
                let request: Value = serde_json::from_slice(&request).unwrap();
                let answer = json!({"id": request["id"], "result": {"ok": true}});
                writer.write_all(format!("{answer}\n").as_bytes()).unwrap();
            }
        });
        let finished = spawn_relay(input, bridge_side);
        daemon.write_all(&frame("one")).unwrap();
        daemon.write_all(&frame("two")).unwrap();
        drop(daemon);
        let (result, output) = finished.recv_timeout(Duration::from_secs(5)).unwrap();
        result.unwrap();
        let ids: Vec<Value> = output
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<Value>(line).unwrap()["id"].clone())
            .collect();
        assert_eq!(ids, vec![json!("one"), json!("two")]);
    }
}
