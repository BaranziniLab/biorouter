//! Shared broker test harness: a fake account directory, a bootstrapped workspace, signed
//! requests as any member, legacy enrollment, and a writer for **generated legacy journals**.
//!
//! The legacy journal is generated in the test rather than checked in: `recovered_state`
//! requires `host_uid == geteuid()` and `private_file` requires mode 0600, one link and the
//! current owner, which no checked-in file satisfies. [`Workspace::edit_journal`] closes the
//! broker, appends checksummed delta records carrying legacy shapes (each with its `sequence`
//! patch, which replay checks after every record), and reopens it.
//!
//! Every account lookup goes through [`FakeDirectory`], so tests choose any UID and name, rename
//! or recycle accounts, and count NSS calls. The host is always the process's own UID, because
//! a workspace belongs to the account that runs its broker.
#![allow(dead_code)]

use anyhow::{anyhow, Result};
use biorouter_crew::{
    signing_payload, Account, Broker, Connection, DeviceAuth, Directory, Request, Response,
};
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// The UID that owns (hosts) every test workspace: the test process's own.
pub fn host_uid() -> u32 {
    unsafe { libc::geteuid() }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A deterministic device key.
pub fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

pub fn key_hex(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

/// The device ID the broker derives from a key: SHA-256 of its 32 bytes.
pub fn device_id(key: &SigningKey) -> String {
    digest(&key.verifying_key().to_bytes())
}

pub fn request(id: &str, method: &str, params: Value) -> Request {
    Request {
        version: 1,
        id: id.into(),
        method: method.into(),
        params,
        auth: None,
        credential: None,
    }
}

/// A private temporary directory, removed on drop.
pub struct TempRoot(PathBuf);

impl TempRoot {
    pub fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "biorouter-crew-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    /// A directory with a short path directly under `/tmp`, for Unix sockets (whose paths
    /// are limited to 107 bytes).
    pub fn short() -> Self {
        let suffix: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(12)
            .collect();
        let path = Path::new("/tmp").join(format!("crt-{suffix}"));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// An in-memory account database. Clones share state, so a test keeps a handle after giving
/// the broker a boxed copy, and can rename, recycle or remove accounts mid-test.
#[derive(Clone, Default)]
pub struct FakeDirectory {
    accounts: Arc<Mutex<BTreeMap<u32, Account>>>,
    calls: Arc<AtomicUsize>,
    /// `UID_MIN` as this fake's `login.defs` would say it; `None` answers the trait's default.
    uid_min: Arc<Mutex<Option<u32>>>,
}

impl FakeDirectory {
    pub fn set(&self, uid: u32, name: &str, full_name: Option<&str>) {
        self.accounts.lock().unwrap().insert(
            uid,
            Account {
                uid,
                name: name.into(),
                full_name: full_name.map(str::to_owned),
                shell: None,
            },
        );
    }
    /// As [`Self::set`], with a login shell.
    pub fn set_with_shell(&self, uid: u32, name: &str, shell: &str) {
        self.accounts.lock().unwrap().insert(
            uid,
            Account {
                uid,
                name: name.into(),
                full_name: None,
                shell: Some(shell.into()),
            },
        );
    }
    /// Answer `uid_min` with `uid_min`, as a node whose `login.defs` sets it.
    pub fn set_uid_min(&self, uid_min: u32) {
        *self.uid_min.lock().unwrap() = Some(uid_min);
    }
    pub fn remove(&self, uid: u32) {
        self.accounts.lock().unwrap().remove(&uid);
    }
    /// How many lookups (`by_uid` or `by_name`) the broker has made.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    pub fn boxed(&self) -> Box<dyn Directory + Send> {
        Box::new(self.clone())
    }
}

impl Directory for FakeDirectory {
    fn by_uid(&self, uid: u32) -> Result<Account> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.accounts
            .lock()
            .unwrap()
            .get(&uid)
            .cloned()
            .ok_or_else(|| anyhow!("identity_unavailable: Unix account cannot be resolved"))
    }
    fn by_name(&self, name: &str) -> Result<Account> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.accounts
            .lock()
            .unwrap()
            .values()
            .find(|account| account.name == name)
            .cloned()
            .ok_or_else(|| anyhow!("identity_unavailable: Unix account cannot be resolved"))
    }
    fn uid_min(&self) -> u32 {
        (*self.uid_min.lock().unwrap()).unwrap_or(1000)
    }
}

/// One enrolled person: their UID, device key, connection (which holds their live
/// challenges) and principal ID.
pub struct Member {
    pub uid: u32,
    pub key: SigningKey,
    pub connection: Connection,
    pub principal_id: String,
}

impl Member {
    pub fn new(uid: u32, key: SigningKey) -> Self {
        Self {
            uid,
            key,
            connection: Connection::new(),
            principal_id: String::new(),
        }
    }
}

static REQUEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn next_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        REQUEST_COUNTER.fetch_add(1, Ordering::SeqCst)
    )
}

/// Sign `method(params)` with `member`'s device key over a fresh challenge and send it. A
/// unique `idempotency_key` is added unless `params` already has one (reads ignore it).
pub fn signed(
    broker: &mut Broker,
    member: &mut Member,
    method: &str,
    mut params: Value,
) -> Response {
    if let Some(fields) = params.as_object_mut() {
        fields
            .entry("idempotency_key")
            .or_insert_with(|| json!(next_id("key")));
    }
    signed_raw(broker, member, method, params)
}

/// [`signed`] without adding an idempotency key.
pub fn signed_raw(
    broker: &mut Broker,
    member: &mut Member,
    method: &str,
    params: Value,
) -> Response {
    let device_id = device_id(&member.key);
    let challenge = broker.handle(
        member.uid,
        &mut member.connection,
        request(
            &next_id("challenge"),
            "auth.challenge",
            json!({"device_id": device_id}),
        ),
    );
    let nonce = challenge.result.expect("challenge")["nonce"]
        .as_str()
        .unwrap()
        .to_owned();
    let signature = member.key.sign(&signing_payload(
        &broker.workspace().id,
        member.uid,
        &nonce,
        method,
        &params,
    ));
    let mut req = request(&next_id("request"), method, params);
    req.auth = Some(DeviceAuth {
        device_id,
        nonce,
        signature: hex::encode(signature.to_bytes()),
    });
    broker.handle(member.uid, &mut member.connection, req)
}

/// The result of a request that must succeed.
pub fn ok(response: Response) -> Value {
    match response.error {
        None => response.result.expect("result"),
        Some(error) => panic!("request refused: {}: {}", error.code, error.message),
    }
}

/// The `(code, message)` of a request that must be refused.
pub fn refused(response: Response) -> (String, String) {
    let error = response
        .error
        .unwrap_or_else(|| panic!("request should be refused: {:?}", response.result));
    (error.code, error.message)
}

/// Open the broker over `directory`, with the sibling-workspace probe pointed at a directory
/// that does not exist, so a real broker running on the test machine never answers it.
/// [`Broker::set_runtime_root`] points it elsewhere for the probe's own tests.
fn open(root: &Path, host_key: &SigningKey, directory: &FakeDirectory) -> Broker {
    let mut broker =
        Broker::open_with_directory(root, &key_hex(host_key), directory.boxed()).unwrap();
    broker.set_runtime_root(&root.join("no-sibling-runtimes"));
    broker
}

/// A bootstrapped workspace hosted by `@alice` (the process's own UID, full name "Alice
/// Chen"), opened over a [`FakeDirectory`].
pub struct Workspace {
    pub broker: Broker,
    pub directory: FakeDirectory,
    pub host: Member,
    pub root: TempRoot,
}

impl Workspace {
    /// Bootstrapped and labelled Private with institution `ucsf`.
    pub fn new(label: &str) -> Self {
        let mut workspace = Self::unlabelled(label);
        workspace.host_ok(
            "policy.set",
            json!({"mode": "private", "institution_id": "ucsf"}),
        );
        workspace
    }

    /// Bootstrapped only.
    pub fn unlabelled(label: &str) -> Self {
        let root = TempRoot::new(label);
        let directory = FakeDirectory::default();
        directory.set(host_uid(), "alice", Some("Alice Chen"));
        let host_key = key(7);
        let mut broker = open(root.path(), &host_key, &directory);
        let mut host = Member::new(host_uid(), host_key);
        let params = json!({"public_key": key_hex(&host.key)});
        let result = ok(signed_raw(&mut broker, &mut host, "auth.bootstrap", params));
        host.principal_id = result["principal"]["id"].as_str().unwrap().to_owned();
        Self {
            broker,
            directory,
            host,
            root,
        }
    }

    /// `method(params)` as the host.
    pub fn host_call(&mut self, method: &str, params: Value) -> Response {
        signed(&mut self.broker, &mut self.host, method, params)
    }

    pub fn host_ok(&mut self, method: &str, params: Value) -> Value {
        ok(self.host_call(method, params))
    }

    /// `method(params)` as `member`.
    pub fn call(&mut self, member: &mut Member, method: &str, params: Value) -> Response {
        signed(&mut self.broker, member, method, params)
    }

    pub fn call_ok(&mut self, member: &mut Member, method: &str, params: Value) -> Value {
        ok(self.call(member, method, params))
    }

    /// The host invites `uid` with `key` by the legacy token path; returns the token.
    pub fn legacy_invite(&mut self, uid: u32, key: &SigningKey) -> Response {
        self.host_call(
            "enrollment.invite",
            json!({"uid": uid, "public_key": key_hex(key)}),
        )
    }

    /// `member` redeems a legacy enrollment token with `auth.enroll`.
    pub fn legacy_enroll(&mut self, member: &mut Member, token: &str) -> Response {
        signed_raw(
            &mut self.broker,
            member,
            "auth.enroll",
            json!({"public_key": key_hex(&member.key), "invitation": token}),
        )
    }

    /// Give `uid` the account `username` and enroll it with a fresh key from `seed`.
    pub fn enroll(&mut self, uid: u32, username: &str, seed: u8) -> Member {
        self.directory.set(uid, username, None);
        let mut member = Member::new(uid, key(seed));
        let invitation = ok(self.legacy_invite(uid, &member.key));
        let token = invitation["invitation"].as_str().unwrap().to_owned();
        let result = ok(self.legacy_enroll(&mut member, &token));
        member.principal_id = result["principal"]["id"].as_str().unwrap().to_owned();
        member
    }

    /// The host revokes `principal_id` from the workspace.
    pub fn offboard(&mut self, principal_id: &str) {
        self.host_ok("enrollment.revoke", json!({"principal_id": principal_id}));
    }

    /// Create a team as `member`: `(team_id, general_channel_id)`.
    pub fn create_team(&mut self, member: &mut Member, name: &str) -> (String, String) {
        let result = self.call_ok(member, "team.create", json!({"name": name}));
        (
            result["team"]["id"].as_str().unwrap().to_owned(),
            result["channel"]["id"].as_str().unwrap().to_owned(),
        )
    }

    /// The host creates a team.
    pub fn host_team(&mut self, name: &str) -> (String, String) {
        let result = self.host_ok("team.create", json!({"name": name}));
        (
            result["team"]["id"].as_str().unwrap().to_owned(),
            result["channel"]["id"].as_str().unwrap().to_owned(),
        )
    }

    /// `inviter` invites `invitee` to a team and the invitee accepts.
    pub fn add_to_team(&mut self, inviter: &mut Member, invitee: &mut Member, team_id: &str) {
        let invitation = self.call_ok(
            inviter,
            "invitation.create",
            json!({"kind": "team", "target_id": team_id, "principal_id": invitee.principal_id}),
        );
        self.call_ok(
            invitee,
            "invitation.accept",
            json!({"invitation_id": invitation["id"]}),
        );
    }

    /// As [`Self::add_to_team`], with the host as the inviter.
    pub fn host_adds_to_team(&mut self, invitee: &mut Member, team_id: &str) {
        let invitation = self.host_ok(
            "invitation.create",
            json!({"kind": "team", "target_id": team_id, "principal_id": invitee.principal_id}),
        );
        self.call_ok(
            invitee,
            "invitation.accept",
            json!({"invitation_id": invitation["id"]}),
        );
    }

    pub fn snapshot(&mut self, member: &mut Member) -> Value {
        self.call_ok(member, "workspace.snapshot", json!({}))
    }

    pub fn host_snapshot(&mut self) -> Value {
        self.host_ok("workspace.snapshot", json!({}))
    }

    pub fn journal_path(&self) -> PathBuf {
        self.root.path().join("journal.jsonl")
    }

    pub fn journal_bytes(&self) -> Vec<u8> {
        fs::read(self.journal_path()).unwrap()
    }

    /// Close the broker (releasing the writer lock), let `edit` append records to the
    /// journal, and reopen it with the same directory.
    pub fn edit_journal(self, edit: impl FnOnce(&mut JournalWriter)) -> Self {
        let Self {
            broker,
            directory,
            host,
            root,
        } = self;
        drop(broker);
        let mut writer = JournalWriter::open(&root.path().join("journal.jsonl"));
        edit(&mut writer);
        let broker = open(root.path(), &host.key, &directory);
        Self {
            broker,
            directory,
            host,
            root,
        }
    }

    /// Close and reopen the broker over the same state directory.
    pub fn reopen(self) -> Self {
        self.edit_journal(|_| {})
    }
}

/// Appends checksummed v2 delta records to a closed broker's journal, exactly as the broker
/// writes them: `checksum = SHA-256(JSON [version, sequence, previous, actor, operation,
/// timestamp, patches])`, chained to the previous record, with a `Set [sequence]` patch so the
/// replayed state's sequence matches the record's.
pub struct JournalWriter {
    path: PathBuf,
    sequence: u64,
    checksum: String,
}

impl JournalWriter {
    pub fn open(path: &Path) -> Self {
        let file = fs::File::open(path).unwrap();
        let last = BufReader::new(file)
            .lines()
            .map(Result::unwrap)
            .filter(|line| !line.is_empty())
            .last()
            .expect("a bootstrapped journal");
        let record: Value = serde_json::from_str(&last).unwrap();
        Self {
            path: path.to_path_buf(),
            sequence: record["sequence"].as_u64().unwrap(),
            checksum: record["checksum"].as_str().unwrap().to_owned(),
        }
    }

    /// Append one record applying `patches` (plus the sequence patch).
    pub fn append(&mut self, actor: &str, operation: &str, mut patches: Vec<Value>) {
        let sequence = self.sequence + 1;
        patches.push(set(&["sequence"], json!(sequence)));
        let timestamp = now();
        let checksum = digest(
            &serde_json::to_vec(&(
                2u32,
                sequence,
                &self.checksum,
                actor,
                operation,
                timestamp,
                &patches,
            ))
            .unwrap(),
        );
        let record = json!({
            "version": 2,
            "sequence": sequence,
            "previous": self.checksum,
            "checksum": checksum,
            "actor": actor,
            "operation": operation,
            "timestamp": timestamp,
            "patches": patches,
        });
        let mut line = serde_json::to_vec(&record).unwrap();
        line.push(b'\n');
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(&self.path)
            .unwrap();
        file.write_all(&line).unwrap();
        file.sync_all().unwrap();
        self.sequence = sequence;
        self.checksum = checksum;
    }
}

/// A `set` patch.
pub fn set(path: &[&str], value: Value) -> Value {
    json!({"op": "set", "path": path, "value": value})
}

/// A `remove` patch.
pub fn remove(path: &[&str]) -> Value {
    json!({"op": "remove", "path": path})
}

/// A principal in its legacy stored shape.
pub fn legacy_principal(id: &str, uid: u32, username: &str, nickname: &str, active: bool) -> Value {
    json!({"id": id, "uid": uid, "username": username, "nickname": nickname, "avatar": null, "active": active})
}

/// A device in its legacy stored shape (no `added_at`/`added_via`).
pub fn legacy_device(principal_id: &str, key: &SigningKey) -> Value {
    json!({"principal_id": principal_id, "public_key": key_hex(key)})
}

/// A team in its stored shape.
pub fn legacy_team(
    id: &str,
    name: &str,
    created_by: &str,
    members: &[&str],
    general: &str,
) -> Value {
    json!({"id": id, "name": name, "created_by": created_by, "members": members, "general_channel_id": general})
}

/// A channel in its stored shape.
pub fn legacy_channel(id: &str, team_id: &str, name: &str, owner: &str, members: &[&str]) -> Value {
    json!({
        "id": id, "team_id": team_id, "name": name, "created_by": owner, "owner_id": owner,
        "members": members, "archived": false, "classification": "restricted", "pending_owner": null
    })
}

/// A fresh UUID, as the broker mints IDs.
pub fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Whether `text` contains a UUID or a 64-hex string anywhere.
pub fn contains_machine_id(text: &str) -> bool {
    let bytes = text.as_bytes();
    let hex_run = bytes
        .split(|b| !b.is_ascii_hexdigit())
        .any(|run| run.len() >= 32);
    hex_run
        || text
            .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
            .any(|token| uuid::Uuid::try_parse(token).is_ok())
}
