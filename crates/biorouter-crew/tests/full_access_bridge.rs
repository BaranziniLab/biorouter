//! End to end through the real binaries (design OW-6, OW-7): a `biorouter-crew serve` broker,
//! a person's signed requests on its socket, and the member's `biorouter-crew bridge` running a
//! full-access command and copying an attachment to the server and back. The bridge runs from
//! the account's installed copy, which setup replaces while it runs, and a copy the broker has
//! no connection left for fails with the broker's reason.
//!
//! Linux only, and only as an ordinary account: the broker refuses to serve as root and full
//! access refuses to run as root. As root (a container's default) the test says so and returns;
//! run its binary as an ordinary user to exercise it there.
#![cfg(target_os = "linux")]

use biorouter_crew::signing_payload;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const BINARY: &str = env!("CARGO_BIN_EXE_biorouter-crew");

fn private_dir(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "crew-e2e-{label}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

/// The broker process, stopped on drop.
struct Served {
    child: Child,
    socket: PathBuf,
    workspace_id: String,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(state: &Path, bootstrap_key: &str) -> Served {
    let mut child = Command::new(BINARY)
        .args(["serve", "--state-dir"])
        .arg(state)
        .args(["--bootstrap-key", bootstrap_key])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let info: Value = serde_json::from_str(&line).unwrap_or_else(|_| panic!("serve said {line}"));
    Served {
        child,
        socket: PathBuf::from(info["socket"].as_str().unwrap()),
        workspace_id: info["workspace_id"].as_str().unwrap().to_owned(),
    }
}

/// A person's signed device on one connection to the broker.
struct Person {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    key: SigningKey,
    workspace_id: String,
    uid: u32,
    sequence: usize,
}

impl Person {
    fn frame(&mut self, request: Value) -> Value {
        self.stream
            .write_all(format!("{request}\n").as_bytes())
            .unwrap();
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn signed(&mut self, method: &str, mut params: Value) -> Value {
        self.sequence += 1;
        if method != "auth.bootstrap" {
            params["idempotency_key"] = json!(format!("e2e-{}", self.sequence));
        }
        let device_id = hex::encode(Sha256::digest(self.key.verifying_key().to_bytes()));
        let challenge = self.frame(json!({
            "version": 1, "id": format!("challenge-{}", self.sequence), "method": "auth.challenge",
            "params": {"device_id": device_id},
        }));
        let nonce = challenge["result"]["nonce"].as_str().unwrap().to_owned();
        let signature = self.key.sign(&signing_payload(
            &self.workspace_id,
            self.uid,
            &nonce,
            method,
            &params,
        ));
        let answer = self.frame(json!({
            "version": 1, "id": format!("request-{}", self.sequence), "method": method,
            "params": params,
            "auth": {"device_id": device_id, "nonce": nonce, "signature": hex::encode(signature.to_bytes())},
        }));
        assert!(answer["error"].is_null(), "{method}: {answer}");
        answer["result"].clone()
    }
}

/// The member's bridge, spoken to as the daemon does: one JSON line each way.
struct Bridge {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    credential: String,
    sequence: usize,
}

impl Bridge {
    fn ask(&mut self, method: &str, params: Value) -> Value {
        self.sequence += 1;
        let request = json!({
            "version": 1, "id": format!("worker-{}", self.sequence), "method": method,
            "params": params, "credential": self.credential,
        });
        self.input
            .write_all(format!("{request}\n").as_bytes())
            .unwrap();
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn ok(&mut self, method: &str, params: Value) -> Value {
        let answer = self.ask(method, params);
        assert!(answer["error"].is_null(), "{method}: {answer}");
        answer["result"].clone()
    }

    fn finished(&mut self, job_id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let status = self.ok("remote.job_status", json!({"job_id": job_id}));
            if !matches!(status["state"].as_str(), Some("starting" | "running")) {
                return status;
            }
            assert!(Instant::now() < deadline, "job never finished: {status}");
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_full_access_run_runs_commands_and_moves_files_through_the_real_bridge() {
    let uid = unsafe { libc::geteuid() };
    if uid == 0 {
        eprintln!(
            "skipped: the broker and full access refuse root; run this binary as an ordinary user"
        );
        return;
    }
    let state = private_dir("state");
    let home = private_dir("home");
    let work = home.join("crew-work/lab");
    fs::create_dir_all(&work).unwrap();
    let key = SigningKey::from_bytes(&[42; 32]);
    let served = serve(&state, &hex::encode(key.verifying_key().to_bytes()));

    let stream = UnixStream::connect(&served.socket).unwrap();
    let mut person = Person {
        reader: BufReader::new(stream.try_clone().unwrap()),
        stream,
        key,
        workspace_id: served.workspace_id.clone(),
        uid,
        sequence: 0,
    };
    let public_key = hex::encode(person.key.verifying_key().to_bytes());
    person.signed("auth.bootstrap", json!({"public_key": public_key}));
    person.signed(
        "policy.set",
        json!({"mode": "private", "institution_id": "ucsf"}),
    );
    let team = person.signed("team.create", json!({"name": "lab"}));
    let channel = team["channel"]["id"].as_str().unwrap().to_owned();

    // A person shares an attachment in the channel.
    let attachment: Vec<u8> = (0..600_000usize).map(|i| (i * 7 % 253) as u8).collect();
    let sha256 = hex::encode(Sha256::digest(&attachment));
    let blob = person.signed(
        "blob.begin",
        json!({"channel_id": channel, "size": attachment.len(), "sha256": sha256, "name": "data.bin", "media_type": "application/octet-stream"}),
    );
    let blob_id = blob["id"].as_str().unwrap().to_owned();
    for (index, chunk) in attachment.chunks(262_144).enumerate() {
        person.signed(
            "blob.chunk",
            json!({"blob_id": blob_id, "offset": index * 262_144, "data_hex": hex::encode(chunk)}),
        );
    }
    person.signed("blob.finish", json!({"blob_id": blob_id}));

    let snapshot = person.signed("workspace.snapshot", json!({}));
    let epoch = snapshot["workspace"]["policy_epoch"].clone();
    let run = person.signed(
        "run.create",
        json!({
            "channel_id": channel, "source_channels": [channel], "provider_policy_id": "private-test",
            "personal_mode": "private", "public_provider": false, "expires_in": 600,
            "expected_workspace_policy_epoch": epoch, "workspace_institution_id": "ucsf",
            "connection_institution_id": "ucsf", "provider_affiliation": {"kind": "local"},
            "expected_protected_context": true, "remote_root": work, "remote_execution": true,
            "remote_access": "full",
        }),
    );
    assert_eq!(run["run"]["remote_access"], "full");

    // The bridge runs from the copy setup installs, as the daemon starts it over SSH.
    let installed = home.join(".local/bin/biorouter-crew");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::copy(BINARY, &installed).unwrap();
    fs::set_permissions(&installed, fs::Permissions::from_mode(0o755)).unwrap();
    let mut child = Command::new(&installed)
        .args(["bridge", "--stdio", "--socket"])
        .arg(&served.socket)
        .args([
            "--owner-uid",
            &uid.to_string(),
            "--workspace-id",
            &served.workspace_id,
        ])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut bridge = Bridge {
        input: child.stdin.take().unwrap(),
        output: BufReader::new(child.stdout.take().unwrap()),
        child,
        credential: run["credential"].as_str().unwrap().to_owned(),
        sequence: 0,
    };

    let hello = bridge.ok("bridge.hello", json!({}));
    assert_eq!(hello["uid"], uid);
    assert!(hello["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("remote_full_v1")));

    // A command with the network and a child process, which folder mode could never run.
    let shell = bridge.ok(
        "remote.shell",
        json!({"command": "echo hi; python3 -c 'import socket; socket.socket(); print(\"net\")'; sh -c 'id -u'", "idempotency_key": "shell", "wait_seconds": 20}),
    );
    assert_eq!(shell["state"], "completed", "{shell}");
    assert_eq!(shell["stdout_tail"], format!("hi\nnet\n{uid}\n"));

    // Setup installs a newer copy while the bridge runs (`mv -f` over it): the bridge's own
    // file is gone, and its jobs still start, from the image it runs.
    let staged = home.join(".local/bin/.biorouter-crew.partial");
    fs::copy(BINARY, &staged).unwrap();
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).unwrap();
    fs::rename(&staged, &installed).unwrap();
    let after = bridge.ok(
        "remote.shell",
        json!({"command": "echo after", "idempotency_key": "after-update", "wait_seconds": 20}),
    );
    assert_eq!(after["state"], "completed", "{after}");
    assert_eq!(after["stdout_tail"], "after\n");
    let refused = bridge.ask(
        "remote.write",
        json!({"path": "~/.local/bin/biorouter-crew", "text": "x"}),
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|message| message.starts_with("forbidden:")),
        "{refused}"
    );

    // The attachment comes to the server whole and checked, and goes back as a new one.
    let fetch = bridge.ok(
        "remote.fetch_blob",
        json!({"blob_id": blob_id, "path": "inbox/data.bin", "idempotency_key": "fetch"}),
    );
    let fetched = bridge.finished(fetch["job_id"].as_str().unwrap());
    assert_eq!(fetched["state"], "completed", "{fetched}");
    assert_eq!(fs::read(work.join("inbox/data.bin")).unwrap(), attachment);
    let share = bridge.ok(
        "remote.share_file",
        json!({"path": "inbox/data.bin", "name": "copy.bin", "idempotency_key": "share"}),
    );
    let shared = bridge.finished(share["job_id"].as_str().unwrap());
    assert_eq!(shared["state"], "completed", "{shared}");
    assert_eq!(shared["size"], attachment.len());
    let copy = person.signed("blob.status", json!({"blob_id": shared["blob_id"]}));
    assert_eq!(copy["complete"], true);
    assert_eq!(copy["sha256"], sha256);
    assert_eq!(copy["channel_id"], channel.as_str());

    // The account's connections fill up: the next one is told why, and so is a copy that
    // needed it.
    let mut held = Vec::new();
    let refusal = loop {
        assert!(held.len() <= 8, "the broker never refused a connection");
        let mut stream = UnixStream::connect(&served.socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let _ = stream
            .write_all(b"{\"version\":1,\"id\":\"hello\",\"method\":\"hello\",\"params\":{}}\n");
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        let answer: Value = serde_json::from_str(&line).unwrap();
        if !answer["error"].is_null() {
            break answer;
        }
        held.push(stream);
    };
    assert_eq!(refusal["error"]["code"], "quota_exceeded", "{refusal}");
    let fetch = bridge.ok(
        "remote.fetch_blob",
        json!({"blob_id": blob_id, "path": "inbox/again.bin", "idempotency_key": "fetch-full"}),
    );
    let failed = bridge.finished(fetch["job_id"].as_str().unwrap());
    assert_eq!(failed["state"], "failed", "{failed}");
    assert!(
        failed["error"]
            .as_str()
            .is_some_and(|error| error.contains("8 connections")),
        "{failed}"
    );
    drop(held);

    // Revoking the run ends what its credential can do through the bridge.
    person.signed("run.revoke", json!({"run_id": run["run"]["id"]}));
    let refused = bridge.ask(
        "remote.shell",
        json!({"command": "true", "idempotency_key": "after", "wait_seconds": 1}),
    );
    assert_eq!(
        refused["error"]["code"], "remote_operation_denied",
        "{refused}"
    );
    drop(bridge);
    drop(served);
    let _ = fs::remove_dir_all(&state);
    let _ = fs::remove_dir_all(&home);
}
