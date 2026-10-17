//! Full-access jobs and transfers in the member's bridge (design OW-6, OW-7), driven through
//! `remote_full::handle_in` with a private home and the real `biorouter-crew` binary as the
//! supervisor. Linux only: jobs need `/proc`, `prctl` and process groups.
#![cfg(target_os = "linux")]

use biorouter_crew::remote_full::{handle_in, job_id, BrokerLink, Context};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A private home with a work folder, removed on drop.
struct Home {
    path: PathBuf,
    context: Context,
}

impl Home {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "crew-full-contract-{label}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(path.join("crew-work/lab")).unwrap();
        let path = fs::canonicalize(path).unwrap();
        let context = Context {
            home: path.clone(),
            jobs: path.join(".local/state/biorouter-crew/jobs"),
            executable: PathBuf::from(env!("CARGO_BIN_EXE_biorouter-crew")),
            program: PathBuf::from(env!("CARGO_BIN_EXE_biorouter-crew")),
        };
        Self { path, context }
    }

    fn work(&self) -> PathBuf {
        self.path.join("crew-work/lab")
    }

    fn scope(&self, owner: &str, run: &str) -> Value {
        json!({
            "root": self.work(),
            "allow_execute": true,
            "public_provider": false,
            "run_id": run,
            "policy_epoch": 1,
            "expires_at": now() + 600,
            "access": "full",
            "owner_id": owner,
            "channel_id": "channel-1",
        })
    }

    fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.call_as("owner-1", "run-1", method, params, &no_broker())
    }

    fn call_as(
        &self,
        owner: &str,
        run: &str,
        method: &str,
        params: Value,
        link: &BrokerLink,
    ) -> anyhow::Result<Value> {
        handle_in(
            &self.context,
            method,
            &params,
            &self.scope(owner, run),
            link,
        )
    }

    /// Poll `job_id` until it is no longer starting or running.
    fn finished(&self, job_id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let status = self
                .call("remote.job_status", json!({"job_id": job_id}))
                .unwrap();
            let state = status["state"].as_str().unwrap().to_owned();
            if !matches!(state.as_str(), "starting" | "running") {
                return status;
            }
            assert!(Instant::now() < deadline, "job never finished: {status}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn no_broker() -> BrokerLink {
    BrokerLink {
        connect: Arc::new(|| anyhow::bail!("no broker in this test")),
        credential: "credential".into(),
    }
}

fn shell(command: &str, key: &str, extra: Value) -> Value {
    let mut params = json!({"command": command, "idempotency_key": key, "wait_seconds": 20});
    for (name, value) in extra.as_object().unwrap() {
        params[name] = value.clone();
    }
    params
}

/// Whether a process with this PID exists and is not a zombie.
fn running(pid: u64) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            let rest = stat.get(stat.rfind(')')? + 1..)?.trim_start().to_owned();
            rest.chars().next()
        })
        .is_some_and(|state| state != 'Z' && state != 'X')
}

fn process(home: &Home, job_id: &str) -> Value {
    serde_json::from_slice(&fs::read(home.context.jobs.join(job_id).join("process.json")).unwrap())
        .unwrap()
}

/// A command runs as the account through its login shell, with network sockets and child
/// processes (none of which a confined command may have), in the work folder, and its output
/// lands in owner-only files.
#[test]
fn a_command_runs_as_the_account_with_network_and_children() {
    let home = Home::new("shell");
    let answer = home
        .call(
            "remote.shell",
            shell(
                "echo hi; echo oops >&2; pwd; sh -c 'echo child'; echo \"job=$BIOROUTER_CREW_JOB_ID\"; \
                 python3 -c 'import socket; socket.socket(); print(\"socket ok\")'",
                "k1",
                json!({}),
            ),
        )
        .unwrap();
    assert_eq!(answer["state"], "completed", "{answer}");
    assert_eq!(answer["exit_code"], 0);
    let stdout = answer["stdout_tail"].as_str().unwrap();
    assert!(stdout.contains("hi\n"), "{stdout}");
    assert!(stdout.contains(home.work().to_str().unwrap()), "{stdout}");
    assert!(stdout.contains("child\n"), "{stdout}");
    assert!(stdout.contains("socket ok"), "{stdout}");
    let id = job_id("owner-1", "k1");
    assert_eq!(answer["job_id"], id.as_str());
    assert!(stdout.contains(&format!("job={id}")), "{stdout}");
    // A login shell's startup files may add to standard error; the command's own line is there.
    assert!(
        answer["stderr_tail"].as_str().unwrap().contains("oops\n"),
        "{answer}"
    );
    assert_eq!(answer["truncated"], false);
    let dir = home.context.jobs.join(&id);
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for name in ["stdout", "stderr", "job.json", "exit.json", "process.json"] {
        let mode = fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{name}");
    }
}

#[test]
fn a_failing_command_reports_its_exit_code() {
    let home = Home::new("fail");
    let answer = home
        .call("remote.shell", shell("echo before; exit 3", "k", json!({})))
        .unwrap();
    assert_eq!(answer["state"], "failed");
    assert_eq!(answer["exit_code"], 3);
    assert_eq!(answer["stdout_tail"], "before\n");
}

/// An argv reaches the program exactly, through the login shell, without being reparsed.
#[test]
fn an_argv_runs_exactly() {
    let home = Home::new("argv");
    let answer = home
        .call(
            "remote.execute",
            json!({"argv": ["printf", "%s|", "a b", "$HOME", "c;d"], "idempotency_key": "k", "wait_seconds": 20}),
        )
        .unwrap();
    assert_eq!(answer["state"], "completed", "{answer}");
    assert_eq!(answer["stdout_tail"], "a b|$HOME|c;d|");
}

/// `cwd` may be `~/`-relative, absolute or work-folder-relative; a missing one is refused
/// before anything runs.
#[test]
fn the_working_directory_resolves_like_every_path() {
    let home = Home::new("cwd");
    fs::create_dir(home.work().join("sub")).unwrap();
    for (cwd, expected) in [
        ("~", home.path.clone()),
        ("sub", home.work().join("sub")),
        (home.path.to_str().unwrap(), home.path.clone()),
    ] {
        let answer = home
            .call("remote.shell", shell("pwd", cwd, json!({"cwd": cwd})))
            .unwrap();
        assert_eq!(
            answer["stdout_tail"].as_str().unwrap().trim(),
            expected.to_str().unwrap(),
            "{cwd}"
        );
    }
    let error = home
        .call(
            "remote.shell",
            shell("pwd", "missing", json!({"cwd": "nowhere"})),
        )
        .unwrap_err();
    assert!(error.to_string().starts_with("invalid_params:"), "{error}");
}

/// A job is the owner's, not the run's: a later run of the same person reads it, and another
/// person cannot.
#[test]
fn any_run_of_the_same_owner_reads_a_job_after_it_exits() {
    let home = Home::new("status");
    let started = home
        .call(
            "remote.shell",
            shell("sleep 1; echo done", "k", json!({"wait_seconds": 0})),
        )
        .unwrap();
    assert!(
        matches!(started["state"].as_str(), Some("starting" | "running")),
        "{started}"
    );
    let id = started["job_id"].as_str().unwrap().to_owned();
    let finished = home.finished(&id);
    assert_eq!(finished["state"], "completed");
    let later = home
        .call_as(
            "owner-1",
            "run-2",
            "remote.job_status",
            json!({"job_id": id, "tail_bytes": 2}),
            &no_broker(),
        )
        .unwrap();
    assert_eq!(later["stdout_tail"], "e\n");
    assert_eq!(later["truncated"], true);
    let stranger = home
        .call_as(
            "owner-2",
            "run-3",
            "remote.job_status",
            json!({"job_id": id}),
            &no_broker(),
        )
        .unwrap_err();
    assert!(stranger.to_string().starts_with("forbidden:"), "{stranger}");
    let unknown = home
        .call(
            "remote.job_status",
            json!({"job_id": job_id("owner-1", "nope")}),
        )
        .unwrap_err();
    assert!(unknown.to_string().starts_with("not_found:"), "{unknown}");
}

/// A retried request is the same job, run once; the same key for a different command is refused.
#[test]
fn an_idempotency_key_names_one_job() {
    let home = Home::new("idempotency");
    let command = "echo x >> counter";
    let first = home
        .call("remote.shell", shell(command, "k", json!({})))
        .unwrap();
    let second = home
        .call("remote.shell", shell(command, "k", json!({})))
        .unwrap();
    assert_eq!(first["job_id"], second["job_id"]);
    assert_eq!(
        fs::read_to_string(home.work().join("counter")).unwrap(),
        "x\n"
    );
    let conflict = home
        .call("remote.shell", shell("echo y", "k", json!({})))
        .unwrap_err();
    assert!(conflict.to_string().starts_with("conflict:"), "{conflict}");
}

/// Cancel stops the whole process group, children included, and only that.
#[test]
fn cancel_stops_the_jobs_process_group() {
    let home = Home::new("cancel");
    let started = home
        .call(
            "remote.shell",
            shell(
                "sleep 60 & echo $! > child.pid; sleep 60",
                "k",
                json!({"wait_seconds": 1}),
            ),
        )
        .unwrap();
    assert_eq!(started["state"], "running", "{started}");
    let id = started["job_id"].as_str().unwrap().to_owned();
    let leader = process(&home, &id)["pid"].as_u64().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !home.work().join("child.pid").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(100));
    let child: u64 = fs::read_to_string(home.work().join("child.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(running(leader) && running(child));
    let cancelled = home.call("remote.cancel", json!({"job_id": id})).unwrap();
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while running(child) || running(leader) {
        assert!(Instant::now() < deadline, "the group outlived its cancel");
        std::thread::sleep(Duration::from_millis(50));
    }
    // Cancelling a finished job changes nothing.
    let again = home.call("remote.cancel", json!({"job_id": id})).unwrap();
    assert_eq!(again["state"], "cancelled");
}

#[test]
fn a_command_past_its_timeout_is_stopped() {
    let home = Home::new("timeout");
    let started = Instant::now();
    let answer = home
        .call(
            "remote.shell",
            shell("sleep 30", "k", json!({"timeout_seconds": 1})),
        )
        .unwrap();
    assert_eq!(answer["state"], "timed_out", "{answer}");
    assert!(started.elapsed() < Duration::from_secs(10));
    for bad in [json!(0), json!(3601), json!("10")] {
        assert!(home
            .call(
                "remote.shell",
                shell("true", "bad", json!({"timeout_seconds": bad}))
            )
            .is_err());
    }
}

/// A job that is not detached ends with the bridge that started it (here, the thread standing
/// in for the bridge's relay); a detached one carries on and finishes.
#[test]
fn only_a_detached_job_outlives_its_bridge() {
    let home = Arc::new(Home::new("detach"));
    let start = |command: &'static str, key: &'static str, detach: bool| {
        let home = home.clone();
        std::thread::spawn(move || {
            let started = home
                .call(
                    "remote.shell",
                    shell(command, key, json!({"wait_seconds": 1, "detach": detach})),
                )
                .unwrap();
            // Running, so the supervisor has armed its parent-death signal before this
            // "bridge" goes away.
            assert_eq!(started["state"], "running", "{started}");
            started["job_id"].as_str().unwrap().to_owned()
        })
        .join()
        .unwrap()
    };
    let attached = start("sleep 60", "attached", false);
    let detached = start("sleep 2; echo survived", "detached", true);
    let ended = home.finished(&attached);
    assert_eq!(ended["state"], "cancelled", "{ended}");
    assert!(
        ended["message"]
            .as_str()
            .unwrap()
            .contains("connection ended"),
        "{ended}"
    );
    let survived = home.finished(&detached);
    assert_eq!(survived["state"], "completed", "{survived}");
    assert_eq!(survived["stdout_tail"], "survived\n");
}

/// Eight jobs may run at once in an account; a ninth is refused until one ends.
#[test]
fn at_most_eight_jobs_run_at_once() {
    let home = Home::new("concurrency");
    let mut ids = Vec::new();
    for index in 0..8 {
        let started = home
            .call(
                "remote.shell",
                shell("sleep 30", &format!("k{index}"), json!({"wait_seconds": 0})),
            )
            .unwrap();
        ids.push(started["job_id"].as_str().unwrap().to_owned());
    }
    let refused = home
        .call("remote.shell", shell("true", "ninth", json!({})))
        .unwrap_err();
    assert!(
        refused.to_string().starts_with("quota_exceeded:"),
        "{refused}"
    );
    for id in &ids {
        home.call("remote.cancel", json!({"job_id": id})).unwrap();
    }
    home.call("remote.shell", shell("true", "ninth", json!({})))
        .unwrap();
}

/// The supervisor never starts a command whose cancel was recorded first, and refuses a job
/// directory other accounts could write into.
#[test]
fn a_cancel_recorded_before_the_start_keeps_the_command_from_running() {
    let home = Home::new("early-cancel");
    let dir = home.path.join("job");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let marker = home.path.join("ran");
    let record = json!({
        "job_id": "early", "kind": "shell", "owner_id": "owner-1", "detach": false,
        "timeout_seconds": 10, "cwd": home.path, "parent_pid": std::process::id(),
        "argv": ["/bin/sh", "-c", format!("touch {}", marker.display())],
    });
    fs::write(dir.join("job.json"), record.to_string()).unwrap();
    fs::write(dir.join("cancel.json"), "{}").unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_biorouter-crew"))
        .arg("__crew-run")
        .arg(&dir)
        .status()
        .unwrap();
    assert!(status.success());
    let exit: Value = serde_json::from_slice(&fs::read(dir.join("exit.json")).unwrap()).unwrap();
    assert_eq!(exit["state"], "cancelled", "{exit}");
    assert!(!marker.exists(), "the command ran");
    fs::remove_file(dir.join("exit.json")).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).unwrap();
    let refused = std::process::Command::new(env!("CARGO_BIN_EXE_biorouter-crew"))
        .arg("__crew-run")
        .arg(&dir)
        .status()
        .unwrap();
    assert!(!refused.success());
    assert!(!dir.join("exit.json").exists());
}

/// What the fake broker holds: one attachment to read, and the uploads it was given.
#[derive(Default)]
struct Store {
    blob: Vec<u8>,
    blob_sha256: String,
    uploads: HashMap<String, (u64, String, Vec<u8>, bool)>,
    credentials: Vec<String>,
    delay: Duration,
}

/// A broker connection answering `blob.read`, `blob.begin`, `blob.chunk` and `blob.finish` the
/// way the real broker does, from `store`.
fn fake_broker(store: Arc<Mutex<Store>>) -> BrokerLink {
    BrokerLink {
        connect: Arc::new(move || {
            let (ours, theirs) = UnixStream::pair()?;
            let store = store.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(theirs.try_clone().unwrap());
                let mut writer = theirs;
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    let request: Value = serde_json::from_str(&line).unwrap();
                    line.clear();
                    let delay = store.lock().unwrap().delay;
                    std::thread::sleep(delay);
                    let mut store = store.lock().unwrap();
                    store
                        .credentials
                        .push(request["credential"].as_str().unwrap_or("").to_owned());
                    let p = &request["params"];
                    let answer = match request["method"].as_str().unwrap() {
                        "blob.read" => {
                            let offset = p["offset"].as_u64().unwrap() as usize;
                            let end = (offset + 262_144).min(store.blob.len());
                            json!({"result": {
                                "blob": {"size": store.blob.len(), "sha256": store.blob_sha256},
                                "offset": offset,
                                "data_hex": hex::encode(&store.blob[offset..end]),
                                "next_offset": end,
                                "complete": end == store.blob.len(),
                            }})
                        }
                        "blob.begin" => {
                            let id = format!("blob-{}", store.uploads.len());
                            store.uploads.insert(
                                id.clone(),
                                (
                                    p["size"].as_u64().unwrap(),
                                    p["sha256"].as_str().unwrap().to_owned(),
                                    Vec::new(),
                                    false,
                                ),
                            );
                            json!({"result": {"id": id, "channel_id": p["channel_id"], "name": p["name"]}})
                        }
                        "blob.chunk" => {
                            let upload = store
                                .uploads
                                .get_mut(p["blob_id"].as_str().unwrap())
                                .unwrap();
                            assert_eq!(p["offset"].as_u64().unwrap(), upload.2.len() as u64);
                            upload
                                .2
                                .extend(hex::decode(p["data_hex"].as_str().unwrap()).unwrap());
                            json!({"result": {"offset": upload.2.len()}})
                        }
                        "blob.finish" => {
                            let upload = store
                                .uploads
                                .get_mut(p["blob_id"].as_str().unwrap())
                                .unwrap();
                            if hex::encode(Sha256::digest(&upload.2)) == upload.1
                                && upload.2.len() as u64 == upload.0
                            {
                                upload.3 = true;
                                json!({"result": {"complete": true}})
                            } else {
                                json!({"error": {"code": "digest_mismatch", "message": "digest_mismatch: attachment content differs from declaration"}})
                            }
                        }
                        other => panic!("unexpected {other}"),
                    };
                    let mut answer = answer;
                    answer["id"] = request["id"].clone();
                    if writer.write_all(format!("{answer}\n").as_bytes()).is_err() {
                        break;
                    }
                }
            });
            Ok((ours.try_clone()?, BufReader::new(ours)))
        }),
        credential: "worker-credential".into(),
    }
}

fn pattern(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i * 31 % 251) as u8).collect()
}

/// A channel attachment is copied to the server in the background, whole and checked, with
/// the run's own credential, and lands owner-only.
#[test]
fn an_attachment_is_copied_to_the_server() {
    let home = Home::new("fetch");
    let blob = pattern(700_000);
    let store = Arc::new(Mutex::new(Store {
        blob_sha256: hex::encode(Sha256::digest(&blob)),
        blob,
        ..Store::default()
    }));
    let link = fake_broker(store.clone());
    let started = home
        .call_as(
            "owner-1",
            "run-1",
            "remote.fetch_blob",
            json!({"blob_id": "0123abcd-0000-4000-8000-000000000001", "path": "inbox/data.bin", "idempotency_key": "fetch"}),
            &link,
        )
        .unwrap();
    let finished = home.finished(started["job_id"].as_str().unwrap());
    assert_eq!(finished["state"], "completed", "{finished}");
    assert_eq!(finished["size"], 700_000);
    let target = home.work().join("inbox/data.bin");
    assert_eq!(fs::read(&target).unwrap(), store.lock().unwrap().blob);
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(store
        .lock()
        .unwrap()
        .credentials
        .iter()
        .all(|credential| credential == "worker-credential"));
    // Nothing but the copy was left in the folder.
    let names: Vec<_> = fs::read_dir(home.work().join("inbox"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
    // A file already there is never replaced.
    let refused = home
        .call_as(
            "owner-1",
            "run-1",
            "remote.fetch_blob",
            json!({"blob_id": "0123abcd-0000-4000-8000-000000000001", "path": "inbox/data.bin", "idempotency_key": "fetch-again"}),
            &link,
        )
        .unwrap_err();
    assert!(refused.to_string().starts_with("conflict:"), "{refused}");
}

#[test]
fn a_copy_that_does_not_match_its_digest_is_discarded() {
    let home = Home::new("fetch-bad");
    let store = Arc::new(Mutex::new(Store {
        blob: pattern(300_000),
        blob_sha256: "00".repeat(32),
        ..Store::default()
    }));
    let started = home
        .call_as(
            "owner-1",
            "run-1",
            "remote.fetch_blob",
            json!({"blob_id": "abcd", "path": "bad.bin", "idempotency_key": "fetch"}),
            &fake_broker(store),
        )
        .unwrap();
    let finished = home.finished(started["job_id"].as_str().unwrap());
    assert_eq!(finished["state"], "failed", "{finished}");
    assert!(finished["error"]
        .as_str()
        .unwrap()
        .starts_with("digest_mismatch:"));
    assert_eq!(
        fs::read_dir(home.work()).unwrap().count(),
        0,
        "no partial file left"
    );
}

/// A server file is uploaded to the run's channel in the background, and the job's result
/// names the attachment the daemon then posts.
#[test]
fn a_server_file_is_shared_to_the_channel() {
    let home = Home::new("share");
    let bytes = pattern(600_000);
    fs::write(home.work().join("result.csv"), &bytes).unwrap();
    let store = Arc::new(Mutex::new(Store::default()));
    let started = home
        .call_as(
            "owner-1",
            "run-1",
            "remote.share_file",
            json!({"path": "result.csv", "idempotency_key": "share"}),
            &fake_broker(store.clone()),
        )
        .unwrap();
    let finished = home.finished(started["job_id"].as_str().unwrap());
    assert_eq!(finished["state"], "completed", "{finished}");
    assert_eq!(finished["blob_id"], "blob-0");
    assert_eq!(finished["name"], "result.csv");
    assert_eq!(finished["size"], 600_000);
    let store = store.lock().unwrap();
    let upload = &store.uploads["blob-0"];
    assert!(upload.3, "finished");
    assert_eq!(upload.2, bytes);
}

/// A transfer answers at once with its job; the copy itself never holds the relay.
#[test]
fn a_transfer_returns_before_it_finishes() {
    let home = Home::new("share-slow");
    fs::write(home.work().join("big.bin"), pattern(900_000)).unwrap();
    let store = Arc::new(Mutex::new(Store {
        delay: Duration::from_millis(400),
        ..Store::default()
    }));
    let asked = Instant::now();
    let started = home
        .call_as(
            "owner-1",
            "run-1",
            "remote.share_file",
            json!({"path": "big.bin", "idempotency_key": "share"}),
            &fake_broker(store),
        )
        .unwrap();
    assert!(
        asked.elapsed() < Duration::from_millis(400),
        "{:?}",
        asked.elapsed()
    );
    assert!(
        matches!(started["state"].as_str(), Some("running" | "starting")),
        "{started}"
    );
    let finished = home.finished(started["job_id"].as_str().unwrap());
    assert_eq!(finished["state"], "completed", "{finished}");
}

/// Files outside the work folder are reachable by absolute and `~/` paths, through symlinks.
#[test]
fn file_operations_reach_the_whole_account() {
    let home = Home::new("files");
    let shared = home.path.join("datasets");
    fs::create_dir(&shared).unwrap();
    fs::write(shared.join("table.tsv"), b"a\tb\n").unwrap();
    std::os::unix::fs::symlink(&shared, home.work().join("data")).unwrap();
    let read = home
        .call("remote.read", json!({"path": "data/table.tsv"}))
        .unwrap();
    assert_eq!(read["text_utf8"], "a\tb\n");
    let stat = home
        .call("remote.stat", json!({"path": "~/datasets"}))
        .unwrap();
    assert_eq!(stat["type"], "directory");
    home.call(
        "remote.write",
        json!({"path": shared.join("out.txt"), "text": "x", "mode": "create_new"}),
    )
    .unwrap();
    let listing = home
        .call("remote.list", json!({"path": "~/datasets"}))
        .unwrap();
    assert_eq!(listing["entries"].as_array().unwrap().len(), 2);
    let hashed = home
        .call("remote.hash", json!({"path": "data/table.tsv"}))
        .unwrap();
    assert_eq!(hashed["sha256"], hex::encode(Sha256::digest(b"a\tb\n")));
}
