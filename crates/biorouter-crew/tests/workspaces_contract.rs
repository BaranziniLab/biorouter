//! `biorouter-crew workspaces` (account ownership, AO-6): what is on a server for the account
//! that runs it.
//!
//! The hosted half (state directories, deleted and unreadable ones, the folder cap, nothing
//! written) runs on any Unix. Everything that probes a broker needs Linux, since the kernel's
//! peer credentials are what make an answer trustworthy.
//!
//! Tests run as one Unix account, so another account's broker is played by a fake broker of
//! this account while the listing is told it runs as another UID
//! (`Broker::list_workspaces_with`). That covers candidate selection, the ownership checks,
//! decoys, the per-account and total caps, the deadlines and the signature checks. What only a
//! live run with two accounts proves: that the kernel refuses a probe across accounts where a
//! directory or socket is not that account's, and that `/proc/<pid>/exe` of another account's
//! broker is unreadable (its `binary_current` stays `null`).
#![cfg(unix)]

mod support;

use biorouter_crew::Broker;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use support::*;

/// A home directory with the workspace store in it, as setup leaves one.
fn home(label: &str) -> (TempRoot, PathBuf) {
    let home = TempRoot::new(label);
    let store = home.path().join(".local/share/biorouter-crew");
    fs::create_dir_all(&store).unwrap();
    for dir in [
        home.path().join(".local"),
        home.path().join(".local/share"),
        store.clone(),
    ] {
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    (home, store)
}

/// A directory holding `@alice` as the test's own account.
fn alice() -> FakeDirectory {
    let directory = FakeDirectory::default();
    directory.set(host_uid(), "alice", Some("Alice Chen"));
    directory
}

/// A workspace hosted in `store/folder`, bootstrapped, named `name`, labelled Private with
/// institution `ucsf`, its runtime directory bound under `runtime_root`, and then stopped.
/// Returns its ID, state directory, socket path and state.
fn hosted(
    store: &Path,
    folder: &str,
    name: &str,
    runtime_root: &Path,
) -> (String, PathBuf, PathBuf, Value) {
    let dir = store.join(folder);
    let directory = alice();
    let host_key = key(7);
    let public_key = key_hex(&host_key);
    let mut broker = Broker::open_with_directory(&dir, &public_key, directory.boxed()).unwrap();
    broker.set_runtime_root(runtime_root);
    let mut host = Member::new(host_uid(), host_key);
    ok(signed_raw(
        &mut broker,
        &mut host,
        "auth.bootstrap",
        json!({"public_key": public_key}),
    ));
    ok(signed(
        &mut broker,
        &mut host,
        "workspace.rename",
        json!({"name": name}),
    ));
    ok(signed(
        &mut broker,
        &mut host,
        "policy.set",
        json!({"mode": "private", "institution_id": "ucsf"}),
    ));
    let socket = broker.prepare_runtime("test-node").unwrap();
    let id = broker.workspace().id.clone();
    let state = broker.state_json();
    drop(broker);
    (id, dir, socket, state)
}

/// The workspace public key a journal's state holds, hex.
fn public_key_of(state: &Value) -> String {
    let secret: [u8; 32] = hex::decode(state["workspace_signing_key"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    hex::encode(
        ed25519_dalek::SigningKey::from_bytes(&secret)
            .verifying_key()
            .to_bytes(),
    )
}

fn entries(answer: &Value) -> &Vec<Value> {
    answer["workspaces"].as_array().expect("a workspaces list")
}

fn entry<'a>(answer: &'a Value, workspace_id: &str) -> &'a Value {
    entries(answer)
        .iter()
        .find(|entry| entry["workspace_id"] == workspace_id)
        .unwrap_or_else(|| panic!("{workspace_id} missing from {answer:#}"))
}

/// Every path under `root` with its size and mode, to show a listing wrote nothing.
fn tree(root: &Path) -> Vec<(PathBuf, u64, u32)> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let metadata = fs::symlink_metadata(&path).unwrap();
        found.push((path.clone(), metadata.len(), metadata.permissions().mode()));
        if metadata.is_dir() {
            for child in fs::read_dir(&path).unwrap() {
                stack.push(child.unwrap().path());
            }
        }
    }
    found.sort();
    found
}

/// Serializes the tests that use `/tmp` itself as the runtime root, where an invitation's
/// socket path must live.
static TMP: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A runtime directory of this account directly under `/tmp`, removed on drop.
struct TmpRuntime(PathBuf);
impl TmpRuntime {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn new(prefix: &str) -> Self {
        let tail: String = uuid()
            .replace('-', "")
            .chars()
            .take(32 - prefix.len())
            .collect();
        let path = Path::new("/tmp").join(format!("crew-{}-{prefix}{tail}", host_uid()));
        Self(path)
    }
}
impl Drop for TmpRuntime {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn hosted_workspaces_are_listed_stopped_deleted_or_unknown_and_nothing_is_written() {
    let _tmp = TMP.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (home, store) = home("ws-hosted");
    let tmp = Path::new("/tmp");
    let (lab, lab_dir, lab_socket, lab_state) = hosted(&store, "lab", "lab", tmp);
    let _lab_runtime = TmpRuntime(lab_socket.parent().unwrap().to_path_buf());
    let (notes, notes_dir, notes_socket, _) = hosted(&store, "notes", "notes", tmp);
    let _notes_runtime = TmpRuntime(notes_socket.parent().unwrap().to_path_buf());
    // An erase a crash interrupted: its marker names the workspace.
    fs::write(notes_dir.join("DELETED"), notes.as_bytes()).unwrap();
    fs::set_permissions(notes_dir.join("DELETED"), fs::Permissions::from_mode(0o600)).unwrap();
    // A journal that does not replay, and a folder other accounts may read.
    let broken = store.join("broken");
    fs::create_dir(&broken).unwrap();
    fs::set_permissions(&broken, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        broken.join("journal.jsonl"),
        b"{\"version\":2,\"oops\":true}\n",
    )
    .unwrap();
    fs::set_permissions(
        broken.join("journal.jsonl"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let open = store.join("open");
    fs::create_dir(&open).unwrap();
    fs::write(open.join("journal.jsonl"), b"").unwrap();
    fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
    // Neither a folder without a journal nor the restart lock is a workspace.
    fs::create_dir(store.join("empty")).unwrap();
    fs::write(store.join(".restart-lock"), b"").unwrap();

    let before = tree(home.path());
    let answer = Broker::list_workspaces_with(home.path(), tmp, host_uid(), &alice());
    assert_eq!(tree(home.path()), before, "the listing changes nothing");

    assert_eq!(
        answer["account"],
        json!({"uid": host_uid(), "username": "alice"})
    );
    assert_eq!(answer["truncated"], false);
    let lab_entry = entry(&answer, &lab);
    let public_key = public_key_of(&lab_state);
    assert_eq!(lab_entry["role"], "host");
    assert_eq!(lab_entry["state"], "stopped");
    assert_eq!(lab_entry["name"], "lab");
    assert_eq!(lab_entry["mode"], "private");
    assert_eq!(lab_entry["institution_id"], "ucsf");
    assert_eq!(lab_entry["host_uid"], host_uid());
    assert_eq!(lab_entry["host_username"], "alice");
    assert_eq!(lab_entry["socket_path"], json!(lab_socket));
    assert_eq!(lab_entry["state_dir"], json!(lab_dir));
    assert_eq!(lab_entry["found_by"], "state_dir");
    assert!(lab_entry["version"].is_null());
    assert!(lab_entry["binary_current"].is_null());
    assert!(lab_entry["others_connected"].is_null());
    assert_eq!(
        lab_entry["workspace_key_fingerprint"],
        json!(digest(&hex::decode(&public_key).unwrap()))
    );
    let invitation = biorouter_crew::invitation::parse(lab_entry["invitation"].as_str().unwrap())
        .unwrap()
        .invitation;
    assert_eq!(invitation.workspace_id, lab);
    assert_eq!(invitation.workspace_public_key, public_key);
    assert_eq!(invitation.socket_path, lab_socket.to_str().unwrap());
    assert_eq!(invitation.owner_uid, host_uid());
    assert_eq!(invitation.workspace_name.as_deref(), Some("lab"));
    assert_eq!(invitation.host_username.as_deref(), Some("alice"));
    assert_eq!(
        invitation.ssh_host, None,
        "the server cannot know how people reach it"
    );

    let deleted = entry(&answer, &notes);
    assert_eq!(deleted["state"], "deleted");
    assert!(deleted["invitation"].is_null());

    let unknown: Vec<&Value> = entries(&answer)
        .iter()
        .filter(|entry| entry["state"] == "unknown")
        .collect();
    assert_eq!(unknown.len(), 2, "{answer:#}");
    for (dir, code) in [(&broken, "journal_corrupt"), (&open, "unsafe_storage")] {
        let found = unknown
            .iter()
            .find(|entry| entry["state_dir"] == json!(dir))
            .unwrap_or_else(|| panic!("{} missing: {answer:#}", dir.display()));
        assert!(
            found["error"].as_str().unwrap().starts_with(code),
            "{found:#}"
        );
        assert!(found["invitation"].is_null());
    }
    let listed_dirs: Vec<&Value> = entries(&answer)
        .iter()
        .map(|entry| &entry["state_dir"])
        .collect();
    assert!(!listed_dirs.contains(&&json!(store.join("empty"))));
    assert!(
        serde_json::to_vec(&answer).unwrap().len() <= 48 * 1024,
        "the answer stays within 48 KiB"
    );
}

#[test]
fn more_than_sixty_four_hosted_folders_leave_the_answer_truncated() {
    let (home, store) = home("ws-cap");
    let runtime = TempRoot::short();
    let directory = alice();
    for index in 0..66 {
        let dir = store.join(format!("w{index:02}"));
        drop(Broker::open_with_directory(&dir, &key_hex(&key(7)), directory.boxed()).unwrap());
    }
    let answer = Broker::list_workspaces_with(home.path(), runtime.path(), host_uid(), &alice());
    let hosted = entries(&answer)
        .iter()
        .filter(|entry| entry["found_by"] == "state_dir")
        .count();
    assert_eq!(hosted, 64, "{answer:#}");
    assert_eq!(answer["truncated"], true);
    assert!(serde_json::to_vec(&answer).unwrap().len() <= 48 * 1024);
}

#[test]
fn an_account_that_cannot_be_resolved_is_listed_without_a_name() {
    let (home, store) = home("ws-noname");
    let runtime = TempRoot::short();
    let (id, ..) = hosted(&store, "lab", "lab", runtime.path());
    let answer = Broker::list_workspaces_with(
        home.path(),
        runtime.path(),
        host_uid(),
        &FakeDirectory::default(),
    );
    assert_eq!(
        answer["account"],
        json!({"uid": host_uid(), "username": null})
    );
    let lab = entry(&answer, &id);
    assert_eq!(lab["state"], "stopped");
    assert!(lab["host_username"].is_null());
    assert!(
        lab["invitation"].is_null(),
        "a line needs the host's name: {lab:#}"
    );
}

/// The command itself: one line of JSON, whatever is there.
#[test]
fn the_command_prints_one_line_of_json() {
    let (home, _store) = home("ws-command");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_biorouter-crew"))
        .args(["workspaces", "--output-format", "json"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.trim_end().lines().count(), 1, "{text}");
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(answer["account"]["uid"], host_uid());
    for count in ["skipped", "unsupported"] {
        assert!(answer[count].is_u64(), "{count}: {answer:#}");
    }
    assert!(answer["truncated"].is_boolean(), "{answer:#}");
    assert!(answer["workspaces"].is_array());
    let refused = std::process::Command::new(env!("CARGO_BIN_EXE_biorouter-crew"))
        .args(["workspaces", "--output-format", "text"])
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(!refused.status.success());
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use biorouter_crew::{hello_v1_payload, HelloV2, Mode};
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::time::{Duration, Instant};

    /// How a fake broker behaves. (A test fixture: the size of its variants does not matter.)
    #[derive(Clone)]
    #[allow(clippy::large_enum_variant)]
    enum Behaviour {
        /// Answer `hello` (signed with `key` over the nonce it is sent, unless `wrong_nonce`)
        /// and `account.status` with `status`.
        Answer {
            key: SigningKey,
            workspace_id: String,
            host_uid: u32,
            name: Option<String>,
            capabilities: Vec<&'static str>,
            status: Value,
            version: Option<&'static str>,
            wrong_nonce: bool,
        },
        /// Read the request and never answer.
        Stall,
        /// Answer one byte at a time, slowly, forever.
        Drip,
    }

    fn capabilities(account: bool) -> Vec<&'static str> {
        let mut list = vec!["human_chat", "signed_devices", "join_by_name_v1"];
        if account {
            list.push("account_identity_v1");
        }
        list
    }

    fn member_broker(role: &str) -> Behaviour {
        Behaviour::Answer {
            key: SigningKey::from_bytes(&rand_seed()),
            workspace_id: uuid(),
            host_uid: host_uid(),
            name: Some("lab".into()),
            capabilities: capabilities(true),
            status: json!({"role": role, "workspace_name": "lab", "host_username": "mallory"}),
            version: Some("1.94.1"),
            wrong_nonce: false,
        }
    }

    fn rand_seed() -> [u8; 32] {
        let id = uuid().replace('-', "");
        let mut seed = [0u8; 32];
        seed[..16].copy_from_slice(&hex::decode(&id).unwrap());
        seed[16..].copy_from_slice(&hex::decode(&id).unwrap());
        seed
    }

    fn hello_answer(behaviour: &Behaviour, nonce: &str) -> Value {
        let Behaviour::Answer {
            key,
            workspace_id,
            host_uid,
            name,
            capabilities,
            version,
            wrong_nonce,
            ..
        } = behaviour
        else {
            unreachable!()
        };
        let public_key = hex::encode(key.verifying_key().to_bytes());
        let signed_nonce = if *wrong_nonce { "another-nonce" } else { nonce };
        let v1 = key.sign(&hello_v1_payload(
            workspace_id,
            *host_uid,
            signed_nonce,
            &public_key,
            "test-node",
        ));
        let v2 = key.sign(
            &HelloV2 {
                workspace_id,
                host_uid: *host_uid,
                challenge_nonce: signed_nonce,
                workspace_public_key: &public_key,
                node_id: "test-node",
                mode: &Mode::Private,
                institution_id: Some("ucsf"),
                policy_epoch: 3,
                name: name.as_deref(),
                capabilities,
            }
            .signing_payload(),
        );
        let mut hello = json!({
            "protocol": 1, "workspace_id": workspace_id, "host_uid": host_uid,
            "mode": "private", "institution_id": "ucsf", "policy_epoch": 3, "name": name,
            "workspace_public_key": public_key, "node_id": "test-node",
            "workspace_key_fingerprint": digest(&key.verifying_key().to_bytes()),
            "challenge_nonce": nonce, "signature": hex::encode(v1.to_bytes()),
            "signature_v2": hex::encode(v2.to_bytes()), "capabilities": capabilities,
            "state": "running",
        });
        if let Some(version) = version {
            hello["version"] = json!(version);
        }
        hello
    }

    fn serve_one(stream: UnixStream, behaviour: Behaviour) {
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let request: Value = serde_json::from_str(&line).unwrap_or_default();
            match &behaviour {
                Behaviour::Stall => {
                    std::thread::sleep(Duration::from_secs(30));
                    return;
                }
                Behaviour::Drip => {
                    for _ in 0..150 {
                        if writer.write_all(b" ").is_err() {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    return;
                }
                Behaviour::Answer { status, .. } => {
                    let result = match request["method"].as_str() {
                        Some("hello") => hello_answer(
                            &behaviour,
                            request["params"]["challenge_nonce"].as_str().unwrap_or(""),
                        ),
                        Some("account.status") => status.clone(),
                        _ => json!({}),
                    };
                    let answer = json!({"id": request["id"], "result": result});
                    if writer.write_all(format!("{answer}\n").as_bytes()).is_err() {
                        return;
                    }
                }
            }
        }
    }

    /// A fake broker listening at `directory/broker.sock` (the directory is made private).
    fn fake_broker(directory: &Path, behaviour: Behaviour) -> PathBuf {
        fs::create_dir(directory).unwrap();
        fs::set_permissions(directory, fs::Permissions::from_mode(0o711)).unwrap();
        let socket = directory.join("broker.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let behaviour = behaviour.clone();
                std::thread::spawn(move || serve_one(stream, behaviour));
            }
        });
        socket
    }

    /// The name of one of this account's runtime directories: `crew-<uid>-` and 32 hex digits
    /// starting with `prefix`.
    fn runtime_name(prefix: &str) -> String {
        let tail: String = uuid()
            .replace('-', "")
            .chars()
            .take(32 - prefix.len())
            .collect();
        format!("crew-{}-{prefix}{tail}", host_uid())
    }

    fn workspace_id_of(behaviour: &Behaviour) -> String {
        match behaviour {
            Behaviour::Answer { workspace_id, .. } => workspace_id.clone(),
            _ => unreachable!(),
        }
    }

    /// The listing as another account sees this one's brokers: the test account's runtime
    /// directories stand in for another account's.
    fn as_another_account(home: &Path, runtime_root: &Path) -> Value {
        Broker::list_workspaces_with(home, runtime_root, host_uid() + 1, &alice())
    }

    #[test]
    fn another_accounts_workspaces_are_listed_by_role_with_the_host_named_locally() {
        let (home, _store) = home("ws-scan-roles");
        let runtime = TempRoot::short();
        let mut ids = Vec::new();
        for (index, role) in ["member", "invited", "removed", "none"].iter().enumerate() {
            let behaviour = member_broker(role);
            ids.push(workspace_id_of(&behaviour));
            fake_broker(
                &runtime.path().join(runtime_name(&format!("{index}"))),
                behaviour,
            );
        }
        let answer = as_another_account(home.path(), runtime.path());
        assert_eq!(answer["skipped"], 0, "{answer:#}");
        assert_eq!(answer["unsupported"], 0);
        assert_eq!(entries(&answer).len(), 3, "{answer:#}");
        for (id, role) in ids.iter().zip(["member", "invited", "removed"]) {
            let found = entry(&answer, id);
            assert_eq!(found["role"], role);
            assert_eq!(found["found_by"], "scan");
            assert_eq!(found["host_uid"], host_uid());
            // From resolving the socket's owner, never from the broker's answer ("mallory").
            assert_eq!(found["host_username"], "alice");
            assert_eq!(found["name"], "lab");
            assert_eq!(found["state"], "running");
            assert_eq!(found["version"], "1.94.1");
            assert!(found["binary_current"].is_null());
            assert!(found["state_dir"].is_null());
            assert!(found["others_connected"].is_null());
        }
    }

    #[test]
    fn four_brokers_per_account_are_probed_and_the_rest_are_counted() {
        let (home, _store) = home("ws-scan-cap");
        let runtime = TempRoot::short();
        for index in 0..6 {
            fake_broker(
                &runtime.path().join(runtime_name(&format!("{index}"))),
                member_broker("member"),
            );
        }
        let answer = as_another_account(home.path(), runtime.path());
        assert_eq!(entries(&answer).len(), 4, "{answer:#}");
        assert_eq!(answer["skipped"], 2);
    }

    #[test]
    fn a_stalled_or_dripping_broker_cannot_hold_the_scan_past_its_deadline() {
        let (home, _store) = home("ws-scan-deadline");
        let runtime = TempRoot::short();
        fake_broker(&runtime.path().join(runtime_name("0")), Behaviour::Stall);
        fake_broker(&runtime.path().join(runtime_name("1")), Behaviour::Drip);
        let good = member_broker("member");
        let good_id = workspace_id_of(&good);
        fake_broker(&runtime.path().join(runtime_name("2")), good);
        let started = Instant::now();
        let answer = as_another_account(home.path(), runtime.path());
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_secs(3), "took {elapsed:?}");
        assert_eq!(answer["skipped"], 2, "{answer:#}");
        assert_eq!(entry(&answer, &good_id)["role"], "member");
    }

    #[test]
    fn decoys_take_no_slot_and_are_not_counted() {
        let (home, _store) = home("ws-scan-decoys");
        let runtime = TempRoot::short();
        // The real broker sorts after every decoy named for the same account.
        let good = member_broker("member");
        let good_id = workspace_id_of(&good);
        let real = runtime.path().join(runtime_name("f"));
        fake_broker(&real, good);
        let decoy = |prefix: &str| runtime.path().join(runtime_name(prefix));
        // A plain file, a link to the real directory, a directory without a socket, a
        // directory others can write into holding a live socket, and a file named broker.sock.
        fs::write(decoy("0"), b"").unwrap();
        std::os::unix::fs::symlink(&real, decoy("1")).unwrap();
        fs::create_dir(decoy("2")).unwrap();
        let writable = decoy("3");
        fake_broker(&writable, member_broker("member"));
        fs::set_permissions(&writable, fs::Permissions::from_mode(0o777)).unwrap();
        let fake_socket = decoy("4");
        fs::create_dir(&fake_socket).unwrap();
        fs::write(fake_socket.join("broker.sock"), b"").unwrap();
        // A directory named for another UID that this account owns, and a name with the UID
        // written with a leading zero.
        let hex: String = "5".repeat(32);
        fake_broker(
            &runtime
                .path()
                .join(format!("crew-{}-{hex}", host_uid() + 7)),
            member_broker("member"),
        );
        fake_broker(
            &runtime.path().join(format!("crew-0{}-{hex}", host_uid())),
            member_broker("member"),
        );
        let answer = as_another_account(home.path(), runtime.path());
        assert_eq!(entries(&answer).len(), 1, "{answer:#}");
        assert_eq!(entry(&answer, &good_id)["role"], "member");
        assert_eq!(answer["skipped"], 0);
        assert_eq!(answer["unsupported"], 0);
    }

    #[test]
    fn an_older_broker_is_counted_and_a_forged_answer_is_skipped() {
        let (home, _store) = home("ws-scan-forged");
        let runtime = TempRoot::short();
        let Behaviour::Answer { key, status, .. } = member_broker("member") else {
            unreachable!()
        };
        let older = Behaviour::Answer {
            key: key.clone(),
            workspace_id: uuid(),
            host_uid: host_uid(),
            name: Some("old-lab".into()),
            capabilities: capabilities(false),
            status: status.clone(),
            version: None,
            wrong_nonce: false,
        };
        let replayed = Behaviour::Answer {
            key: key.clone(),
            workspace_id: uuid(),
            host_uid: host_uid(),
            name: Some("replayed".into()),
            capabilities: capabilities(true),
            status: status.clone(),
            version: None,
            wrong_nonce: true,
        };
        let not_its_host = Behaviour::Answer {
            key,
            workspace_id: uuid(),
            host_uid: host_uid() + 9,
            name: Some("elsewhere".into()),
            capabilities: capabilities(true),
            status,
            version: None,
            wrong_nonce: false,
        };
        fake_broker(&runtime.path().join(runtime_name("0")), older);
        fake_broker(&runtime.path().join(runtime_name("1")), replayed);
        fake_broker(&runtime.path().join(runtime_name("2")), not_its_host);
        let answer = as_another_account(home.path(), runtime.path());
        assert!(entries(&answer).is_empty(), "{answer:#}");
        assert_eq!(answer["unsupported"], 1);
        assert_eq!(answer["skipped"], 2);
    }

    /// A member's entry carries the line the ordinary join path opens, pointing at the socket
    /// the broker answered at.
    #[test]
    fn a_found_workspace_carries_its_invitation_line() {
        let _tmp = TMP.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let (home, _store) = home("ws-scan-line");
        let behaviour = member_broker("member");
        let id = workspace_id_of(&behaviour);
        let directory = TmpRuntime::new("0000");
        let socket = fake_broker(&directory.0, behaviour);
        let answer = as_another_account(home.path(), Path::new("/tmp"));
        let found = entry(&answer, &id);
        let line = biorouter_crew::invitation::parse(found["invitation"].as_str().unwrap())
            .unwrap()
            .invitation;
        assert_eq!(line.workspace_id, id);
        assert_eq!(line.socket_path, socket.to_str().unwrap());
        assert_eq!(line.owner_uid, host_uid());
        assert_eq!(line.host_username.as_deref(), Some("alice"));
        assert_eq!(line.workspace_name.as_deref(), Some("lab"));
        assert_eq!(line.ssh_host, None);
    }

    /// The runtime descriptor a running broker of the store's workspace in `dir` leaves.
    fn write_runtime(dir: &Path, socket: &Path, workspace_id: &str, key: &SigningKey) {
        let info = json!({
            "pid": std::process::id(),
            "socket": socket,
            "workspace_id": workspace_id,
            "host_uid": host_uid(),
            "protocol": 1,
            "node_id": "test-node",
            "workspace_public_key": hex::encode(key.verifying_key().to_bytes()),
            "version": "1.94.0",
        });
        let path = dir.join("runtime.json");
        fs::write(&path, serde_json::to_vec(&info).unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn a_running_hosted_workspace_is_described_by_its_broker() {
        let (home, store) = home("ws-running");
        let runtime = TempRoot::short();
        let (id, dir, ..) = hosted(&store, "lab", "lab", runtime.path());
        let key = SigningKey::from_bytes(&rand_seed());
        let behaviour = Behaviour::Answer {
            key: key.clone(),
            workspace_id: id.clone(),
            host_uid: host_uid(),
            name: Some("lab".into()),
            capabilities: capabilities(true),
            status: json!({"role": "host", "workspace_name": "lab", "others_connected": 2}),
            version: None,
            wrong_nonce: false,
        };
        let socket = fake_broker(&runtime.path().join("live"), behaviour);
        write_runtime(&dir, &socket, &id, &key);
        // The installed program is this test's own executable, which the listener runs.
        let bin = home.path().join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        std::os::unix::fs::symlink(std::env::current_exe().unwrap(), bin.join("biorouter-crew"))
            .unwrap();

        let answer =
            Broker::list_workspaces_with(home.path(), runtime.path(), host_uid(), &alice());
        let lab = entry(&answer, &id);
        assert_eq!(lab["state"], "running", "{answer:#}");
        assert_eq!(lab["found_by"], "state_dir");
        assert_eq!(lab["state_dir"], json!(dir));
        assert_eq!(lab["socket_path"], json!(socket));
        // An older broker's hello says no version; its runtime.json may.
        assert_eq!(lab["version"], "1.94.0");
        assert_eq!(lab["binary_current"], true);
        assert_eq!(lab["others_connected"], 2);
        assert_eq!(lab["host_username"], "alice");

        // A newer copy installed over it: the running broker no longer runs the installed file.
        fs::remove_file(bin.join("biorouter-crew")).unwrap();
        fs::copy(std::env::current_exe().unwrap(), bin.join("biorouter-crew")).unwrap();
        let answer =
            Broker::list_workspaces_with(home.path(), runtime.path(), host_uid(), &alice());
        assert_eq!(entry(&answer, &id)["binary_current"], false);
    }

    #[test]
    fn a_broker_answering_for_another_workspace_is_unknown_and_a_gone_one_is_stopped() {
        let (home, store) = home("ws-mismatch");
        let runtime = TempRoot::short();
        let (id, dir, ..) = hosted(&store, "lab", "lab", runtime.path());
        let key = SigningKey::from_bytes(&rand_seed());
        let impostor = Behaviour::Answer {
            key: key.clone(),
            workspace_id: uuid(),
            host_uid: host_uid(),
            name: Some("lab".into()),
            capabilities: capabilities(true),
            status: json!({"role": "host", "others_connected": 0}),
            version: None,
            wrong_nonce: false,
        };
        let socket = fake_broker(&runtime.path().join("live"), impostor);
        write_runtime(&dir, &socket, &id, &key);
        let answer =
            Broker::list_workspaces_with(home.path(), runtime.path(), host_uid(), &alice());
        let lab = entry(&answer, &id);
        assert_eq!(lab["state"], "unknown", "{answer:#}");
        assert!(lab["error"]
            .as_str()
            .unwrap()
            .starts_with("identity_mismatch"));

        // Nothing listens at the recorded socket any more: the journal says what it is.
        fs::remove_file(&socket).unwrap();
        let answer =
            Broker::list_workspaces_with(home.path(), runtime.path(), host_uid(), &alice());
        assert_eq!(entry(&answer, &id)["state"], "stopped", "{answer:#}");
    }

    #[test]
    fn a_workspace_started_from_a_custom_state_dir_is_found_by_its_runtime_directory() {
        let (home, store) = home("ws-custom");
        let runtime = TempRoot::short();
        let custom = Behaviour::Answer {
            key: SigningKey::from_bytes(&rand_seed()),
            workspace_id: uuid(),
            host_uid: host_uid(),
            name: Some("scratch".into()),
            capabilities: capabilities(true),
            status: json!({"role": "host", "others_connected": 1}),
            version: Some("1.94.1"),
            wrong_nonce: false,
        };
        let custom_id = workspace_id_of(&custom);
        fake_broker(&runtime.path().join(runtime_name("1")), custom);
        // A workspace of the store whose runtime.json went missing while its broker runs.
        let (stored_id, stored_dir, ..) = hosted(&store, "lab", "lab", runtime.path());
        fake_broker(
            &runtime.path().join(runtime_name("2")),
            Behaviour::Answer {
                key: SigningKey::from_bytes(&rand_seed()),
                workspace_id: stored_id.clone(),
                host_uid: host_uid(),
                name: Some("lab".into()),
                capabilities: capabilities(false),
                status: Value::Null,
                version: None,
                wrong_nonce: false,
            },
        );
        // A socket nothing listens on any more is passed over.
        let stale = runtime.path().join(runtime_name("3"));
        fs::create_dir(&stale).unwrap();
        drop(UnixListener::bind(stale.join("broker.sock")).unwrap());

        let answer =
            Broker::list_workspaces_with(home.path(), runtime.path(), host_uid(), &alice());
        let found = entry(&answer, &custom_id);
        assert_eq!(found["found_by"], "runtime", "{answer:#}");
        assert!(found["state_dir"].is_null());
        assert_eq!(found["state"], "running");
        assert_eq!(found["role"], "host");
        assert_eq!(found["others_connected"], 1);
        let stored = entry(&answer, &stored_id);
        assert_eq!(stored["state"], "running", "{answer:#}");
        assert_eq!(stored["state_dir"], json!(stored_dir));
        assert!(
            stored["others_connected"].is_null(),
            "no account_identity_v1"
        );
        assert_eq!(entries(&answer).len(), 2, "{answer:#}");
        assert_eq!(answer["skipped"], 0);
    }

    /// A probe never reads more than 16 KiB of an answer.
    #[test]
    fn an_oversized_answer_is_skipped() {
        let (home, _store) = home("ws-scan-large");
        let runtime = TempRoot::short();
        let directory = runtime.path().join(runtime_name("0"));
        fs::create_dir(&directory).unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o711)).unwrap();
        let listener = UnixListener::bind(directory.join("broker.sock")).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                std::thread::spawn(move || {
                    let mut line = String::new();
                    let _ = BufReader::new(stream.try_clone().unwrap()).read_line(&mut line);
                    let _ = stream.write_all(&vec![b'x'; 64 * 1024]);
                    std::thread::sleep(Duration::from_secs(5));
                });
            }
        });
        let started = Instant::now();
        let answer = as_another_account(home.path(), runtime.path());
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(answer["skipped"], 1, "{answer:#}");
        assert!(entries(&answer).is_empty());
    }
}
