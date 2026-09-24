use crate::*;
use anyhow::{anyhow, bail, ensure, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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
fn username(uid: u32) -> Result<String> {
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0u8; 65536];
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    ensure!(
        status == 0 && !found.is_null(),
        "identity_unavailable: Unix account cannot be resolved"
    );
    Ok(unsafe { std::ffi::CStr::from_ptr((*found).pw_name) }
        .to_str()?
        .to_owned())
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
}
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
fn recovered_state(state_value: Value, sequence: u64, bootstrap_key: &str) -> Result<State> {
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
                    name: None,
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
            }
        }
    };
    Ok(state)
}
impl Broker {
    pub fn open(root: &Path, bootstrap_key: &str) -> Result<Self> {
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
        let state = recovered_state(state_value, sequence, bootstrap_key)?;
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
                principal.active && principal.uid == uid && principal.username == username(uid)?,
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
                self.channel(&self.state, &actor.id, channel, false)?;
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
        let mut state = self.state.clone();
        let result = self.mutate(&mut state, actor, req)?;
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
        let payload = serde_json::to_vec(&json!([
            self.state.workspace.id,
            self.state.workspace.host_uid,
            nonce,
            public_key,
            node_id
        ]))?;
        let signature = hex::encode(key.sign(&payload).to_bytes());
        Ok(
            json!({"protocol":1,"workspace_id":self.state.workspace.id,"host_uid":self.state.workspace.host_uid,"mode":self.state.workspace.mode,"institution_id":self.state.workspace.institution_id,"policy_epoch":self.state.workspace.policy_epoch,"workspace_public_key":public_key,"node_id":node_id,"workspace_key_fingerprint":digest(&key.verifying_key().to_bytes()),"challenge_nonce":nonce,"signature":signature,"capabilities":["human_chat","signed_devices","resumable_blobs","scoped_runs"],"unsupported":["arbitrary_shell","remote_filesystem","network_filesystem","cross_workspace_release"]}),
        )
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
        let current_username = username(uid)?;
        let active = state.principals.values().find(|p| p.uid == uid && p.active);
        let principal = match (existing_principal_id.as_deref(), active) {
            (Some(expected), Some(principal)) if principal.id == expected && principal.username == current_username => principal.clone(),
            (None, None) => Principal {
                id: id(), uid, username: current_username.clone(), nickname: current_username,
                avatar: None, active: true,
            },
            _ => bail!("identity_mismatch: enrollment principal changed; request a new invitation explicitly identifying the existing principal or offboard the old account"),
        };
        let device_id = digest(&hex::decode(public_key)?);
        state.devices.insert(
            device_id.clone(),
            Device {
                principal_id: principal.id.clone(),
                public_key: public_key.into(),
            },
        );
        state
            .principals
            .insert(principal.id.clone(), principal.clone());
        if req.method == "auth.enroll" {
            state
                .enrollments
                .remove(&digest(text(&req.params, "invitation")?.as_bytes()));
        }
        self.commit(state, &principal.id, &req.method)?;
        Ok(json!({"principal":principal,"device_id":device_id,"workspace":self.state.workspace}))
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
                && p.username == username(uid)?
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
            _ => bail!("unsupported: method unavailable"),
        }
    }
    fn message_wire(message: &Message) -> Value {
        let mut value = json!(message);
        value["sequence"] = json!(message.id);
        value
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
                Ok(Self::message_wire(message))
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
        Ok(
            json!({"workspace":s.workspace,"protected_channel_ids":protected_channel_ids,"actor":s.principals.get(&actor.id),"principals":s.principals.values().filter(|p|p.active).collect::<Vec<_>>(),"teams":s.teams.values().filter(|t|t.members.contains(&actor.id)).collect::<Vec<_>>(),"channels":s.channels.values().filter(|c|c.members.contains(&actor.id)).collect::<Vec<_>>(),"invitations":s.invitations.values().filter(|i|i.principal_id==actor.id || i.inviter_id==actor.id).collect::<Vec<_>>(),"runs":s.runs.values().filter(|r|r.owner_id==actor.id).collect::<Vec<_>>(),"read_positions":positions,"unread":unread,"references":s.references.values().filter(|r|self.reference_authorized(s,actor,r).is_ok()).collect::<Vec<_>>()}),
        )
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
        let messages: Vec<_> = messages.into_iter().map(Self::message_wire).collect();
        Ok(json!({"messages":messages,"cursor":cursor}))
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
        let messages: Vec<_> = messages.into_iter().map(Self::message_wire).collect();
        Ok(
            json!({"run_id":run.id,"policy_epoch":s.workspace.policy_epoch,"source_channels":run.source_channels,"messages":messages,"restricted":restricted}),
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
    fn mutate(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        match req.method.as_str() {
            "channel.read" => self.mutate_channel_read(s, actor, req),
            "profile.update" => self.mutate_profile_update(s, actor, req),
            "enrollment.invite" => self.mutate_enrollment_invite(s, actor, req),
            "enrollment.revoke" => self.mutate_enrollment_revoke(s, actor, req),
            "policy.set" => self.mutate_policy_set(s, actor, req),
            "team.create" => self.mutate_team_create(s, actor, req),
            "channel.create" => self.mutate_channel_create(s, actor, req),
            "invitation.create" => self.mutate_invitation_create(s, actor, req),
            "invitation.accept" => self.mutate_invitation_accept(s, actor, req),
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
        let nickname = text(p, "nickname")?;
        ensure!(nickname.len() <= 120, "invalid_params: nickname too long");
        let avatar = p.get("avatar").and_then(Value::as_str);
        ensure!(
            avatar.is_none_or(|a| a.chars().count() <= 12 && !a.chars().any(char::is_control)),
            "invalid_params: avatar must be up to 12 printable characters"
        );
        let profile = s
            .principals
            .get_mut(who)
            .ok_or_else(|| anyhow!("unauthorized"))?;
        profile.nickname = nickname.into();
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
        let current_username = username(uid)?;
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
            None => ensure!(
                existing_principal_id.is_none(),
                "invalid_params: no active principal exists for this UID"
            ),
        }
        let key = text(p, "public_key")?;
        let bytes: [u8; 32] = hex::decode(key)?
            .try_into()
            .map_err(|_| anyhow!("invalid_params: Ed25519 key"))?;
        VerifyingKey::from_bytes(&bytes)?;
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
        s.principals
            .get_mut(target)
            .ok_or_else(|| anyhow!("forbidden: principal unavailable"))?
            .active = false;
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
        let name = text(p, "name")?;
        ensure!(name.len() <= 120, "invalid_params: name too long");
        let team_id = id();
        let channel_id = id();
        let members = BTreeSet::from([who.clone()]);
        let team = Team {
            id: team_id.clone(),
            name: name.into(),
            created_by: who.clone(),
            members: members.clone(),
            general_channel_id: channel_id.clone(),
        };
        let channel = Channel {
            id: channel_id.clone(),
            team_id: team_id.clone(),
            name: "general".into(),
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
        let name = text(p, "name")?;
        ensure!(name.len() <= 120, "invalid_params: name too long");
        let classification = match p.get("classification") {
            Some(v) => serde_json::from_value(v.clone())?,
            None => Classification::Restricted,
        };
        let c = Channel {
            id: id(),
            team_id: team_id.into(),
            name: name.into(),
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
    fn mutate_channel_archive(&self, s: &mut State, actor: &Actor, req: &Request) -> Result<Value> {
        let p = &req.params;
        let who = &actor.id;
        let channel = text(p, "channel_id")?;
        let c = self.channel(s, who, channel, true)?;
        ensure!(c.owner_id == *who, "forbidden: current owner required");
        let c = s.channels.get_mut(channel).expect("authorized channel");
        match req.method.as_str() {
            "channel.archive" => c.archived = true,
            "channel.transfer" => {
                let successor = text(p, "successor_id")?;
                ensure!(
                    successor != who && c.members.contains(successor),
                    "forbidden: eligible successor required"
                );
                c.pending_owner = Some(successor.into());
            }
            _ => {
                let target = text(p, "principal_id")?;
                ensure!(target != who, "forbidden: transfer before owner removal");
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
pub fn serve(root: &Path, bootstrap_key: &str) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "unsupported: serve requires Linux"
    );
    let node_id = node_identity()?;
    let mut broker = Broker::open(root, bootstrap_key)?;
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
pub fn lifecycle(command: &str, root: &Path, key: &str) -> Result<Value> {
    match command {
        "start" => {
            private_dir(root)?;
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
            let child = command
                .arg("serve")
                .arg("--state-dir")
                .arg(root)
                .arg("--bootstrap-key")
                .arg(key)
                .stdin(std::process::Stdio::null())
                .stdout(log.try_clone()?)
                .stderr(log)
                .spawn()?;
            Ok(json!({"started_pid":child.id(),"state":"starting","status_command":"status"}))
        }
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
