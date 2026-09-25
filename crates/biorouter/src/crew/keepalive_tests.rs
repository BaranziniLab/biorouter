//! D-KEEPALIVE (Q2-01): an idle connection's bridge is kept alive by a verified heartbeat, and a
//! bridge found gone is dialled again the way Connect does it, with no prompt, never after a
//! Disconnect, and never in a tight loop. Each test drives a real `connect` against a scripted
//! `ssh` whose every bridge (one per spawn) follows a line of `plan`.
#![cfg(unix)]

use super::keepalive::KeepaliveTiming;
use super::*;
use std::fs;
use std::path::Path;
use std::time::Duration;

const WORKSPACE_ID: &str = "4b4b4b4b-4b4b-44b4-84b4-4b4b4b4b4b4b";
const CONNECTION_ID: &str = "keepalive-connection";
const NONCE: &str = "keepalive-nonce";
const NODE: &str = "7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e7e";

fn workspace_key() -> SigningKey {
    SigningKey::from_bytes(&[23; 32])
}

fn device_key() -> SigningKey {
    SigningKey::from_bytes(&[24; 32])
}

fn connection() -> Connection {
    let device = device_key().verifying_key().to_bytes();
    Connection {
        id: CONNECTION_ID.into(),
        node_id: None,
        name: "keepalive fixture".into(),
        ssh_target: "crew@example.test".into(),
        port: None,
        identity_file: None,
        proxy_jump: None,
        socket_path: "/run/crew.sock".into(),
        owner_uid: 10001,
        workspace_id: WORKSPACE_ID.into(),
        workspace_public_key: hex(&workspace_key().verifying_key().to_bytes()),
        remote_root: None,
        remote_execution: false,
        cluster_connection_id: "keepalive-cluster".into(),
        mode: ClusterMode::Public,
        institution_id: None,
        policy_epoch: 1,
        status: "disconnected".into(),
        last_error: None,
        device_id: hex(&Sha256::digest(device)),
        public_key: hex(&device),
    }
}

/// The broker's `hello` over [`NONCE`], signed (v1) by the pinned workspace key.
fn hello() -> Value {
    signed_hello(NODE, false, &["human_chat"])
}

/// A `hello` from `node` over [`NONCE`], v1-signed by the pinned workspace key and, with `v2`,
/// also v2-signed over the workspace's name, mode, institution, epoch and `capabilities`.
fn signed_hello(node: &str, v2: bool, capabilities: &[&str]) -> Value {
    let c = connection();
    let key = workspace_key();
    let signature = hex(&key
        .sign(&biorouter_crew::hello_v1_payload(
            &c.workspace_id,
            c.owner_uid,
            NONCE,
            &c.workspace_public_key,
            node,
        ))
        .to_bytes());
    let mut hello = json!({
        "protocol": 1,
        "workspace_id": c.workspace_id,
        "host_uid": c.owner_uid,
        "workspace_public_key": c.workspace_public_key,
        "challenge_nonce": NONCE,
        "node_id": node,
        "capabilities": capabilities,
        "signature": signature,
    });
    if v2 {
        let signature_v2 = hex(&key
            .sign(
                &biorouter_crew::HelloV2 {
                    workspace_id: &c.workspace_id,
                    host_uid: c.owner_uid,
                    challenge_nonce: NONCE,
                    workspace_public_key: &c.workspace_public_key,
                    node_id: node,
                    mode: &biorouter_crew::Mode::Public,
                    institution_id: None,
                    policy_epoch: 1,
                    name: Some("lab"),
                    capabilities,
                }
                .signing_payload(),
            )
            .to_bytes());
        hello["mode"] = json!("public");
        hello["institution_id"] = Value::Null;
        hello["policy_epoch"] = json!(1);
        hello["name"] = json!("lab");
        hello["signature_v2"] = json!(signature_v2);
    }
    hello
}

/// A scripted `ssh`. `-G` answers settings the preflight accepts. Each bridge spawn takes the
/// next line of `plan`: `serve` answers everything; `drop-after-N` answers N requests and
/// then ends on the next without answering, as a bridge the broker dropped does
/// (`join-drop-after-1` too, announcing `join_by_name_v1` first); `v2-then-v1` answers its
/// first `hello` v2-signed and every later one v1 only, as a relay stripping the signature
/// would; `other-node-after-1` answers later `hello`s from a different node; `revoked`
/// answers `hello` and `auth.challenge` but refuses every other request as a device the
/// workspace does not know (`unauthorized: unknown device`), and `member-then-revoked` does
/// so after answering the first; `auth` and `unreachable` fail before any request, as OpenSSH
/// does. `context.manifest` answers [`manifest`]; `blob.read` and `blob.status` answer for
/// `blob-new` and `blob-old` ([`blob_read`], [`blob_status`]) and refuse any other blob, as
/// the broker refuses one outside the run. Every request line is logged as `<spawn> <line>` to
/// `requests.log`.
fn write_fake_ssh(root: &Path, plan: &[&str]) {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(root.join("plan"), format!("{}\n", plan.join("\n"))).unwrap();
    let hello = hello().to_string();
    let hello_v2 = signed_hello(NODE, true, &["human_chat"]).to_string();
    let hello_other = signed_hello(&"5d".repeat(32), false, &["human_chat"]).to_string();
    let hello_join = signed_hello(NODE, false, &["human_chat", "join_by_name_v1"]).to_string();
    let manifest = manifest().to_string();
    let read_new = blob_read("blob-new", NEW_CSV).to_string();
    let read_old = blob_read("blob-old", OLD_CSV).to_string();
    let status_new = blob_status("blob-new", NEW_CSV).to_string();
    let status_old = blob_status("blob-old", OLD_CSV).to_string();
    for text in [
        &hello,
        &hello_v2,
        &hello_other,
        &hello_join,
        &manifest,
        &read_new,
        &read_old,
        &status_new,
        &status_old,
    ] {
        assert!(!text.contains('\'') && !text.contains('%'));
    }
    let challenge = json!({"workspace_id": WORKSPACE_ID, "nonce": "nonce", "uid": 10001});
    let script = format!(
        r#"#!/bin/sh
root='{root}'
if [ "$1" = "-G" ]; then
  printf '%s\n' 'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
for arg; do
  if [ "$arg" = "-O" ]; then exit 0; fi
done
n=$(cat "$root/spawns" 2>/dev/null || echo 0)
n=$((n+1))
printf '%s\n' "$n" > "$root/spawns"
plan=$(sed -n "${{n}}p" "$root/plan")
[ -z "$plan" ] && plan=serve
case "$plan" in
  auth)
    printf '%s\n' 'crew@example.test: Permission denied (publickey,keyboard-interactive).' >&2
    exit 255 ;;
  unreachable)
    printf '%s\n' 'ssh: connect to host example.test port 22: Connection refused' >&2
    exit 255 ;;
esac
answered=0
signed=0
while IFS= read -r line; do
  printf '%s %s\n' "$n" "$line" >> "$root/requests.log"
  case "$plan" in
    *drop-after-*) [ "$answered" -ge "${{plan##*drop-after-}}" ] && exit 0 ;;
  esac
  answered=$((answered+1))
  id=$(printf '%s\n' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  if printf '%s\n' "$line" | grep -q '"method":"hello"'; then
    body='{hello}'
    case "$plan" in
      v2-then-v1) [ "$answered" -eq 1 ] && body='{hello_v2}' ;;
      other-node-after-1) [ "$answered" -gt 1 ] && body='{hello_other}' ;;
      join-drop-after-1) body='{hello_join}' ;;
    esac
    printf '{{"id":"%s","result":%s}}\n' "$id" "$body"
  elif printf '%s\n' "$line" | grep -q '"method":"enrollment.pending"'; then
    printf '{{"id":"%s","result":{{"invited":false}}}}\n' "$id"
  elif printf '%s\n' "$line" | grep -q '"method":"auth.challenge"'; then
    printf '{{"id":"%s","result":%s}}\n' "$id" '{challenge}'
  elif printf '%s\n' "$line" | grep -q '"method":"context.manifest"'; then
    printf '{{"id":"%s","result":%s}}\n' "$id" '{manifest}'
  elif printf '%s\n' "$line" | grep -qE '"method":"blob[.](read|status)"'; then
    body=''
    case "$line" in
      *'"method":"blob.read"'*'"blob_id":"blob-new"'*|*'"blob_id":"blob-new"'*'"method":"blob.read"'*) body='{read_new}' ;;
      *'"method":"blob.read"'*'"blob_id":"blob-old"'*|*'"blob_id":"blob-old"'*'"method":"blob.read"'*) body='{read_old}' ;;
      *'"method":"blob.status"'*'"blob_id":"blob-new"'*|*'"blob_id":"blob-new"'*'"method":"blob.status"'*) body='{status_new}' ;;
      *'"method":"blob.status"'*'"blob_id":"blob-old"'*|*'"blob_id":"blob-old"'*'"method":"blob.status"'*) body='{status_old}' ;;
    esac
    if [ -n "$body" ]; then
      printf '{{"id":"%s","result":%s}}\n' "$id" "$body"
    else
      printf '{{"id":"%s","error":{{"code":"forbidden","message":"forbidden: attachment unavailable"}}}}\n' "$id"
    fi
  else
    signed=$((signed+1))
    refuse=0
    case "$plan" in
      revoked) refuse=1 ;;
      member-then-revoked) [ "$signed" -gt 1 ] && refuse=1 ;;
    esac
    if [ "$refuse" = 1 ]; then
      printf '{{"id":"%s","error":{{"code":"unauthorized","message":"unauthorized: unknown device"}}}}\n' "$id"
    else
      printf '{{"id":"%s","result":{{"accepted_method":"fixture"}}}}\n' "$id"
    fi
  fi
done
"#,
        root = root.display(),
    );
    let ssh = bin.join("ssh");
    fs::write(&ssh, script).unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
}

/// The CSV the fixture's newest `gina-assay.csv` holds.
const NEW_CSV: &str = "sample,signal\nS1,12.7\nS2,7.8\n";
/// The CSV its earlier `gina-assay.csv` holds.
const OLD_CSV: &str = "sample,signal\nS1,99.9\nS2,7.8\n";

/// `context.manifest`: two messages in `#data`, each sharing a `gina-assay.csv`, with the
/// broker's `people` map naming their author.
fn manifest() -> Value {
    let message = |id: &str, created_at: u64, blob: &str| {
        json!({"id": id, "sequence": id, "channel_id": "keepalive-channel",
            "actor_id": "principal-gina", "run_id": null, "body": "Shared a file",
            "created_at": created_at, "restricted": false, "source_channels": [],
            "attachments": [blob], "references": [], "status": null})
    };
    json!({
        "run_id": "keepalive-run", "policy_epoch": 1, "source_channels": ["keepalive-channel"],
        "messages": [
            message("message-new", 1_790_214_527, "blob-new"),
            message("message-old", 1_790_214_441, "blob-old"),
        ],
        "restricted": false,
        "people": {"principal-gina": {"username": "crew_gina", "display_name": "Gina Rossi", "active": true}},
        "channel_names": {"keepalive-channel": "data"},
    })
}

/// `blob.status` of one of the fixture's two `gina-assay.csv` uploads: the broker's blob.
fn blob_status(id: &str, csv: &str) -> Value {
    let size = csv.len();
    json!({"run_id": null, "id": id, "owner_id": "principal-gina",
        "channel_id": "keepalive-channel", "name": "gina-assay.csv",
        "media_type": "application/octet-stream", "size": size, "sha256": "00",
        "offset": size, "complete": true, "restricted": false,
        "source_channels": ["keepalive-channel"]})
}

/// `blob.read` of one of them, as the broker answers it: the chunk hex-encoded.
fn blob_read(id: &str, csv: &str) -> Value {
    let size = csv.len();
    json!({
        "blob": blob_status(id, csv),
        "offset": 0,
        "data_hex": hex(csv.as_bytes()),
        "next_offset": size,
        "complete": true,
    })
}

fn spawns(root: &Path) -> usize {
    fs::read_to_string(root.join("spawns"))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

/// Every request frame any bridge received, in order.
fn frames(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("requests.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line.split_once(' ')?.1).ok())
        .collect()
}

/// `(spawn, method)` for every request any bridge received, in order.
fn requests(root: &Path) -> Vec<(usize, String)> {
    fs::read_to_string(root.join("requests.log"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (spawn, frame) = line.split_once(' ')?;
            let frame: Value = serde_json::from_str(frame).ok()?;
            Some((spawn.parse().ok()?, frame["method"].as_str()?.to_owned()))
        })
        .collect()
}

fn fast(retry: Duration) -> KeepaliveTiming {
    KeepaliveTiming {
        tick: Duration::from_millis(40),
        idle: Duration::from_millis(80),
        probe_before_use: Duration::from_secs(600),
        retry_delays: [retry; 3],
        // No later tries unless a test asks for them (see `with_late_retries`).
        late_retry_every: retry,
        late_retry_for: Duration::ZERO,
    }
}

/// `timing` with later network retries every `every` for `window` (Q3-11).
fn with_late_retries(
    timing: KeepaliveTiming,
    every: Duration,
    window: Duration,
) -> KeepaliveTiming {
    KeepaliveTiming {
        late_retry_every: every,
        late_retry_for: window,
        ..timing
    }
}

struct Fixture {
    root: PathBuf,
    manager: Arc<CrewManager>,
    _env: env_lock::EnvGuard<'static>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

async fn fixture(label: &str, plan: &[&str], timing: KeepaliveTiming) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "biorouter-crew-keepalive-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    write_fake_ssh(&root, plan);
    let profile = root.join("profile");
    fs::create_dir_all(&profile).unwrap();
    let original_path = std::env::var("PATH").unwrap_or_default();
    let path = format!("{}:{original_path}", root.join("bin").display());
    let profile = profile.to_string_lossy().into_owned();
    let env = crate::test_sandbox::relocate_path_root_and(
        profile.as_str(),
        [
            ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile.as_str())),
            ("BIOROUTER_DISABLE_KEYRING", Some("true")),
            ("PATH", Some(path.as_str())),
        ],
    );
    *super::TEST_HELLO_NONCE.lock().unwrap() = Some(NONCE.into());
    let manager = CrewManager::shared(root.join("manager")).unwrap();
    manager.set_keepalive_timing(timing);
    manager.registry.lock().await.connections.push(connection());
    manager
        .write_credential(
            &format!("device:{CONNECTION_ID}"),
            &hex(&device_key().to_bytes()),
        )
        .unwrap();
    Fixture {
        root,
        manager,
        _env: env,
    }
}

async fn status(manager: &CrewManager) -> (String, Option<String>) {
    let c = manager.connection(CONNECTION_ID).await.unwrap();
    (c.status, c.last_error)
}

/// Wait up to five seconds for `condition`.
async fn until(mut condition: impl AsyncFnMut() -> bool) {
    for _ in 0..250 {
        if condition().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the condition never held");
}

#[tokio::test]
async fn an_idle_bridge_gets_verified_heartbeats_and_stays_the_same_bridge() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("heartbeat", &["serve"], fast(Duration::from_secs(60))).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let root = f.root.clone();
    until(async || {
        requests(&root)
            .iter()
            .filter(|(_, method)| method == "hello")
            .count()
            >= 3
    })
    .await;
    assert_eq!(spawns(&f.root), 1, "the same bridge, never a new one");
    assert!(
        requests(&f.root)
            .iter()
            .all(|(_, method)| method == "hello"),
        "a heartbeat is only ever a hello: {:?}",
        requests(&f.root)
    );
    assert_eq!(status(&f.manager).await, ("connected".into(), None));
}

#[tokio::test]
async fn a_bridge_dropped_while_idle_is_dialled_again_without_a_prompt() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "redial",
        &["drop-after-1", "serve"],
        fast(Duration::from_secs(60)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let root = f.root.clone();
    let manager = Arc::clone(&f.manager);
    until(async || spawns(&root) == 2 && status(&manager).await == ("connected".to_owned(), None))
        .await;
    // The re-dialled bridge verified the same pinned node, and is kept alive in turn.
    let c = f.manager.connection(CONNECTION_ID).await.unwrap();
    assert_eq!(c.node_id.as_deref(), Some(NODE));
    until(async || {
        requests(&root)
            .iter()
            .filter(|(spawn, method)| *spawn == 2 && method == "hello")
            .count()
            >= 2
    })
    .await;
    assert_eq!(spawns(&f.root), 2);
}

#[tokio::test]
async fn a_redial_that_needs_sign_in_stops_at_once_and_says_why() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "redial-auth",
        &["drop-after-1", "auth", "serve"],
        fast(Duration::from_millis(30)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let manager = Arc::clone(&f.manager);
    until(async || status(&manager).await.0 == "disconnected").await;
    // Several retry gaps later: no retry, because only a person can sign in.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(spawns(&f.root), 2);
    let (state, error) = status(&f.manager).await;
    assert_eq!(state, "disconnected");
    let error = error.expect("the real reason is shown");
    assert!(
        error.starts_with("Couldn't sign in to example.test as crew"),
        "{error}"
    );
    assert!(f.manager.transport(CONNECTION_ID).await.is_err());
}

#[tokio::test]
async fn a_network_failure_is_retried_a_few_times_with_gaps_then_left_showing() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "redial-network",
        &[
            "drop-after-1",
            "unreachable",
            "unreachable",
            "unreachable",
            "unreachable",
            "serve",
        ],
        fast(Duration::from_millis(60)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let root = f.root.clone();
    until(async || spawns(&root) == 5).await;
    // The first re-dial and three retries, then nothing more: never a tight loop.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(spawns(&f.root), 5);
    let (state, error) = status(&f.manager).await;
    assert_eq!(state, "disconnected");
    assert!(error.is_some(), "the failure stays visible");
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());

    // A person's Connect still works, with no keepalive debt left over.
    f.manager.connect(CONNECTION_ID).await.unwrap();
    assert_eq!(status(&f.manager).await, ("connected".into(), None));
}

#[tokio::test]
async fn a_network_failure_that_clears_reconnects_on_a_retry() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "redial-network-clears",
        &["drop-after-1", "unreachable", "serve"],
        fast(Duration::from_millis(60)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let root = f.root.clone();
    let manager = Arc::clone(&f.manager);
    until(async || spawns(&root) == 3 && status(&manager).await == ("connected".to_owned(), None))
        .await;
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_disconnect_is_never_undone_by_the_keepalive() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    // Idle and connected, then the person disconnects: nothing dials again.
    let f = fixture("disconnect", &["serve"], fast(Duration::from_millis(30))).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    f.manager.disconnect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(spawns(&f.root), 1);
    assert_eq!(status(&f.manager).await.0, "disconnected");
    // The fixture holds the environment lock; let it go before the next one takes it.
    drop(f);

    // A re-dial waiting out its gap after a network failure: the person disconnects in the
    // gap, and the retry never happens.
    let f = fixture(
        "disconnect-during-retry",
        &["drop-after-1", "unreachable", "serve"],
        fast(Duration::from_millis(400)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let root = f.root.clone();
    until(async || spawns(&root) == 2).await;
    f.manager.disconnect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(spawns(&f.root), 2, "no retry after a Disconnect");
    assert_eq!(status(&f.manager).await.0, "disconnected");
}

#[tokio::test]
async fn a_request_after_a_long_idle_is_never_written_to_a_dropped_bridge() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    // The heartbeat never runs here (this computer "slept"); the request itself probes.
    let f = fixture(
        "probe-before-use",
        &["drop-after-1", "serve"],
        KeepaliveTiming {
            tick: Duration::from_secs(600),
            idle: Duration::from_secs(600),
            probe_before_use: Duration::from_millis(50),
            retry_delays: [Duration::from_secs(600); 3],
            late_retry_every: Duration::from_secs(600),
            late_retry_for: Duration::ZERO,
        },
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    let answer = f
        .manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap();
    assert_eq!(answer["accepted_method"], "fixture");
    let sent = requests(&f.root);
    // Bridge 1 carried the connect's hello and the probe's, and nothing else.
    assert_eq!(
        sent.iter()
            .filter(|(spawn, _)| *spawn == 1)
            .map(|(_, method)| method.as_str())
            .collect::<Vec<_>>(),
        ["hello", "hello"]
    );
    // The request went once, over the new bridge, after its own verified hello.
    assert_eq!(
        sent.iter()
            .filter(|(spawn, _)| *spawn == 2)
            .map(|(_, method)| method.as_str())
            .collect::<Vec<_>>(),
        ["hello", "auth.challenge", "workspace.snapshot"]
    );
    assert_eq!(status(&f.manager).await, ("connected".into(), None));
}

/// Methods bridge `spawn` received, in order.
fn methods_on(root: &Path, spawn: usize) -> Vec<String> {
    requests(root)
        .into_iter()
        .filter(|(n, _)| *n == spawn)
        .map(|(_, method)| method)
        .collect()
}

#[tokio::test]
async fn a_heartbeat_whose_answer_lost_its_signature_is_final_and_never_redialled() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    // Connected over a v2-signed hello; the next heartbeat is answered v1 only (a relay that
    // strips the signature covering the institution). A re-dial would take that v1 answer at
    // connect, silently: the refusal is final instead, and shown.
    let f = fixture(
        "downgrade",
        &["v2-then-v1", "serve"],
        fast(Duration::from_millis(30)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    assert_eq!(
        f.manager
            .broker_hello(CONNECTION_ID)
            .map(|hello| hello.signature_version),
        Some(2)
    );
    let manager = Arc::clone(&f.manager);
    until(async || status(&manager).await.0 == "disconnected").await;
    // Several retry gaps later: still the one bridge, never a second dial.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(spawns(&f.root), 1, "no re-dial took the v1 answer");
    assert_eq!(
        status(&f.manager).await,
        (
            "disconnected".into(),
            Some("The workspace's answer lost its signature; reconnect to this workspace.".into())
        )
    );
    assert!(f.manager.transport(CONNECTION_ID).await.is_err());
    assert_eq!(
        f.manager
            .broker_hello(CONNECTION_ID)
            .map(|hello| hello.signature_version),
        Some(2),
        "the v1 answer was never cached"
    );
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_heartbeat_from_a_different_node_is_final_and_never_redialled() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "node-changed",
        &["other-node-after-1", "serve"],
        fast(Duration::from_millis(30)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let manager = Arc::clone(&f.manager);
    until(async || status(&manager).await.0 == "disconnected").await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(spawns(&f.root), 1, "no re-dial");
    let (_, error) = status(&f.manager).await;
    assert_eq!(
        error.as_deref(),
        Some("Verified SSH node identity changed; create a newly verified connection")
    );
    let c = f.manager.connection(CONNECTION_ID).await.unwrap();
    assert_eq!(c.node_id.as_deref(), Some(NODE), "the pinned node is kept");
}

#[tokio::test]
async fn a_request_whose_probe_is_refused_fails_with_why_and_writes_nothing() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("probe-refused", &["v2-then-v1", "serve"], slept()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    let error = f
        .manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("lost its signature"), "{error}");
    assert_eq!(methods_on(&f.root, 1), ["hello", "hello"]);
    assert_eq!(spawns(&f.root), 1, "no re-dial");
    assert_eq!(status(&f.manager).await.0, "disconnected");
}

/// A long idle before use, with the heartbeat not running (this computer slept): the bridge
/// is gone, and each of these requests finds it first.
fn slept() -> KeepaliveTiming {
    KeepaliveTiming {
        tick: Duration::from_secs(600),
        idle: Duration::from_secs(600),
        probe_before_use: Duration::from_millis(50),
        retry_delays: [Duration::from_secs(600); 3],
        late_retry_every: Duration::from_secs(600),
        late_retry_for: Duration::ZERO,
    }
}

/// No heartbeat and no probe before use: only what a test sends reaches a bridge.
fn quiet() -> KeepaliveTiming {
    KeepaliveTiming {
        probe_before_use: Duration::from_secs(600),
        ..slept()
    }
}

#[tokio::test]
async fn a_scoped_worker_request_after_a_long_idle_dials_again_first() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("worker-probe", &["drop-after-1", "serve"], slept()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let epoch = f
        .manager
        .connection(CONNECTION_ID)
        .await
        .unwrap()
        .policy_epoch;
    f.manager.registry.lock().await.scopes.insert(
        "keepalive-worker".into(),
        Scope {
            connection_id: CONNECTION_ID.into(),
            run_id: "keepalive-run".into(),
            channel_id: "keepalive-channel".into(),
            source_channels: vec!["keepalive-channel".into()],
            epoch,
            provider_binding: "keepalive-provider".into(),
            public_provider: false,
            origin_restricted: false,
            institution_ids: BTreeSet::new(),
            institution_policy: true,
            expired: false,
            expires_at: None,
            labels: None,
            session_incarnation: None,
        },
    );
    f.manager
        .write_credential("run:keepalive-worker", "run-credential")
        .unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    let answer = f
        .manager
        .worker_request(
            "keepalive-worker",
            "messages.history",
            json!({"channel_id": "keepalive-channel", "limit": 1}),
        )
        .await
        .unwrap();
    assert_eq!(answer["accepted_method"], "fixture");
    // Nothing was written to the dropped bridge but the probe's hello; the request went once,
    // over the new one.
    assert_eq!(methods_on(&f.root, 1), ["hello", "hello"]);
    assert_eq!(methods_on(&f.root, 2), ["hello", "messages.history"]);
    assert_eq!(status(&f.manager).await, ("connected".into(), None));
}

#[tokio::test]
async fn a_hello_refresh_after_a_long_idle_dials_again_first() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("refresh-probe", &["drop-after-1", "serve"], slept()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    f.manager.refresh_broker_hello(CONNECTION_ID).await.unwrap();
    assert_eq!(methods_on(&f.root, 1), ["hello", "hello"]);
    assert_eq!(methods_on(&f.root, 2), ["hello", "hello"]);
    assert_eq!(status(&f.manager).await, ("connected".into(), None));
}

#[tokio::test]
async fn a_join_status_read_after_a_long_idle_dials_again_first() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("pre-auth-probe", &["join-drop-after-1", "serve"], slept()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    f.manager.join_status(CONNECTION_ID).await.unwrap();
    // The unsigned `enrollment.pending` never met the dropped bridge.
    assert_eq!(methods_on(&f.root, 1), ["hello", "hello"]);
    assert_eq!(
        methods_on(&f.root, 2).first().map(String::as_str),
        Some("hello")
    );
    assert!(methods_on(&f.root, 2).contains(&"enrollment.pending".to_owned()));
    assert_eq!(status(&f.manager).await, ("connected".into(), None));
}

/// Grant `WORKER` a run on the fixture's connection, as an admission would.
async fn grant_worker(f: &Fixture) {
    let epoch = f
        .manager
        .connection(CONNECTION_ID)
        .await
        .unwrap()
        .policy_epoch;
    f.manager.registry.lock().await.scopes.insert(
        WORKER.into(),
        Scope {
            connection_id: CONNECTION_ID.into(),
            run_id: "keepalive-run".into(),
            channel_id: "keepalive-channel".into(),
            source_channels: vec!["keepalive-channel".into()],
            epoch,
            provider_binding: "keepalive-provider".into(),
            public_provider: false,
            origin_restricted: false,
            institution_ids: BTreeSet::new(),
            institution_policy: true,
            expired: false,
            expires_at: None,
            labels: None,
            session_incarnation: None,
        },
    );
    f.manager
        .write_credential(&format!("run:{WORKER}"), "run-credential")
        .unwrap();
}

const WORKER: &str = "keepalive-worker";

async fn membership_ended(manager: &CrewManager) -> bool {
    let c = manager.connection(CONNECTION_ID).await.unwrap();
    manager.last_error_code(&c) == Some(super::keepalive::MEMBERSHIP_ENDED)
}

fn hellos(root: &Path) -> usize {
    requests(root)
        .iter()
        .filter(|(_, method)| method == "hello")
        .count()
}

/// Q3-17 and Q3-02: an agent's first `blob.read` leaves `offset` out; the daemon sends 0, the
/// CSV comes back as text, and what was read (with who shared it, from the manifest the run
/// read) is recorded for the result's source line.
#[tokio::test]
async fn a_first_file_read_starts_at_zero_comes_back_as_text_and_is_recorded() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("blob-read", &["serve"], quiet()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    let cap = CallCapability::for_test(ProviderTier::Private, true);
    assert_eq!(f.manager.run_source_line(WORKER), None, "nothing read yet");
    f.manager
        .agent_request(WORKER, &cap, CONNECTION_ID, "context.manifest", json!({}))
        .await
        .unwrap();
    assert_eq!(
        f.manager.run_source_line(WORKER),
        None,
        "reading messages is not reading a file"
    );
    let read = f
        .manager
        .agent_request(
            WORKER,
            &cap,
            CONNECTION_ID,
            "blob.read",
            json!({"blob_id": "blob-new"}),
        )
        .await
        .unwrap();
    assert_eq!(read["text"], NEW_CSV);
    assert!(read.get("data_hex").is_none(), "{read}");
    assert_eq!(read["next_offset"], NEW_CSV.len());
    assert_eq!(read["complete"], true);
    let sent = frames(&f.root)
        .into_iter()
        .find(|frame| frame["method"] == "blob.read")
        .expect("the read reached the workspace");
    assert_eq!(sent["params"]["offset"], 0);
    assert_eq!(sent["params"]["blob_id"], "blob-new");
    assert_eq!(
        f.manager.run_source_line(WORKER).as_deref(),
        Some("Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).")
    );
    // Read twice, named once.
    f.manager
        .agent_request(
            WORKER,
            &cap,
            CONNECTION_ID,
            "blob.read",
            json!({"blob_id": "blob-new", "offset": null}),
        )
        .await
        .unwrap();
    assert_eq!(
        f.manager.run_source_line(WORKER).as_deref(),
        Some("Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).")
    );
    // An explicit offset is the caller's.
    f.manager
        .agent_request(
            WORKER,
            &cap,
            CONNECTION_ID,
            "blob.read",
            json!({"blob_id": "blob-new", "offset": 7}),
        )
        .await
        .unwrap();
    let offsets: Vec<Value> = frames(&f.root)
        .into_iter()
        .filter(|frame| frame["method"] == "blob.read")
        .map(|frame| frame["params"]["offset"].clone())
        .collect();
    assert_eq!(offsets, [json!(0), json!(0), json!(7)]);
    // Before posting, the workspace names the copy the manifest showed and nothing read, so
    // the line says this was the newest of the two.
    assert_eq!(
        f.manager.posted_source_line(WORKER).await.as_deref(),
        Some("Source: `gina-assay.csv` (newest copy), shared by Gina Rossi (@crew_gina).")
    );
    let named: Vec<Value> = frames(&f.root)
        .into_iter()
        .filter(|frame| frame["method"] == "blob.status")
        .map(|frame| frame["params"]["blob_id"].clone())
        .collect();
    assert_eq!(
        named,
        [json!("blob-old")],
        "only the copy nothing named yet"
    );
    // Another chat read nothing, and a posted result forgets its reads.
    assert_eq!(f.manager.run_source_line("another-chat"), None);
    assert_eq!(f.manager.posted_source_line("another-chat").await, None);
    f.manager.forget_run_reads(WORKER);
    assert_eq!(f.manager.run_source_line(WORKER), None);
}

/// Q3-02, the live G11 failure: a run that reads only the older of two `gina-assay.csv`
/// uploads posts a line that says so, because the workspace names the newer copy before the
/// result is posted.
#[tokio::test]
async fn a_run_that_read_only_the_earlier_copy_says_a_newer_one_was_not_read() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("blob-read-old", &["serve"], quiet()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    let cap = CallCapability::for_test(ProviderTier::Private, true);
    f.manager
        .agent_request(WORKER, &cap, CONNECTION_ID, "context.manifest", json!({}))
        .await
        .unwrap();
    let read = f
        .manager
        .agent_request(
            WORKER,
            &cap,
            CONNECTION_ID,
            "blob.read",
            json!({"blob_id": "blob-old"}),
        )
        .await
        .unwrap();
    assert_eq!(read["text"], OLD_CSV);
    assert_eq!(
        f.manager.posted_source_line(WORKER).await.as_deref(),
        Some(
            "Source: `gina-assay.csv` (earlier copy), shared by Gina Rossi (@crew_gina). \
             A newer copy of `gina-assay.csv` was shared and was not read."
        )
    );
    // A file the workspace will not name is skipped, and the rest are still asked.
    f.manager.forget_run_reads(WORKER);
    f.manager.note_run_context(
        WORKER,
        "context.manifest",
        &json!({"messages": [
            {"id": "m3", "sequence": "m3", "created_at": 30, "attachments": ["blob-new"]},
            {"id": "m2", "sequence": "m2", "created_at": 20, "attachments": ["blob-outside"]},
            {"id": "m1", "sequence": "m1", "created_at": 10, "attachments": ["blob-old"]},
        ]}),
    );
    f.manager
        .agent_request(
            WORKER,
            &cap,
            CONNECTION_ID,
            "blob.read",
            json!({"blob_id": "blob-old"}),
        )
        .await
        .unwrap();
    assert_eq!(
        f.manager.posted_source_line(WORKER).await.as_deref(),
        Some(
            "Source: `gina-assay.csv` (earlier copy). \
             A newer copy of `gina-assay.csv` was shared and was not read."
        )
    );
}

/// Q3-12: a device the workspace accepted, then no longer knows, is identity-final: the bridge
/// is retired, no heartbeat or re-dial follows, and the connection says why with a typed code.
#[tokio::test]
async fn a_revoked_device_stops_its_keepalive_and_says_its_membership_ended() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "revoked",
        &["member-then-revoked", "serve"],
        fast(Duration::from_millis(30)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    f.manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap();
    let refused = f
        .manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("unknown device"), "{refused}");
    assert_eq!(
        status(&f.manager).await,
        (
            "disconnected".into(),
            Some("This computer is no longer a member of keepalive fixture.".into())
        )
    );
    assert!(membership_ended(&f.manager).await);
    assert!(f.manager.transport(CONNECTION_ID).await.is_err());
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
    // Many heartbeat and retry gaps later: not one more hello, and no second bridge.
    let heard = hellos(&f.root);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(hellos(&f.root), heard, "no heartbeat after the refusal");
    assert_eq!(spawns(&f.root), 1, "nothing dialled it again");
    assert_eq!(status(&f.manager).await.0, "disconnected");

    // A person's Connect still may, and the code goes with the error it cleared.
    f.manager.connect(CONNECTION_ID).await.unwrap();
    assert_eq!(status(&f.manager).await, ("connected".into(), None));
    assert!(!membership_ended(&f.manager).await);
}

/// A device the workspace never accepted in this process is one still joining: its refusal
/// changes nothing, and its bridge is kept alive while the host approves it.
#[tokio::test]
async fn a_device_that_is_still_joining_keeps_its_bridge_when_refused() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("joining", &["revoked"], fast(Duration::from_millis(30))).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let refused = f
        .manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(refused.contains("unknown device"), "{refused}");
    assert_eq!(status(&f.manager).await, ("connected".into(), None));
    assert!(!membership_ended(&f.manager).await);
    let root = f.root.clone();
    let heard = hellos(&root);
    until(async || hellos(&root) >= heard + 2).await;
    assert_eq!(spawns(&f.root), 1);
    assert_eq!(status(&f.manager).await, ("connected".into(), None));
}

/// Q3-12: after a keepalive re-dial, one person-signed read finds a revocation that happened
/// while the bridge was down, without waiting for anyone to use Crew.
#[tokio::test]
async fn a_redial_checks_membership_and_stops_a_revoked_device() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    // Bridge 1 answers the connect's hello and one signed read, then drops at the next
    // heartbeat; bridge 2 reaches a workspace that no longer knows this device.
    let f = fixture(
        "redial-revoked",
        &["drop-after-3", "revoked", "serve"],
        fast(Duration::from_millis(30)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    f.manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap();
    let manager = Arc::clone(&f.manager);
    until(async || membership_ended(&manager).await).await;
    assert_eq!(
        methods_on(&f.root, 2),
        ["hello", "auth.challenge", "profile.suggest"],
        "the re-dial's own hello, then the membership check"
    );
    let heard = hellos(&f.root);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(hellos(&f.root), heard, "no heartbeat after the refusal");
    assert_eq!(spawns(&f.root), 2, "no third dial");
    assert_eq!(status(&f.manager).await.0, "disconnected");
}

/// Q3-11: once the quick retries are spent on network failures, the keepalive keeps trying
/// now and then, so a network that comes back reconnects without anyone pressing Connect.
#[tokio::test]
async fn a_network_that_comes_back_after_the_quick_retries_reconnects_by_itself() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "late-retry",
        &[
            "drop-after-1",
            "unreachable",
            "unreachable",
            "unreachable",
            "unreachable",
            "unreachable",
            "unreachable",
            "serve",
        ],
        with_late_retries(
            fast(Duration::from_millis(30)),
            Duration::from_millis(60),
            Duration::from_secs(30),
        ),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let root = f.root.clone();
    let manager = Arc::clone(&f.manager);
    // The first re-dial, three quick retries, two later ones, then the network is back.
    until(async || spawns(&root) == 8 && status(&manager).await == ("connected".to_owned(), None))
        .await;
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
}

/// Q3-11: the later tries end when their window does, never in a tight loop, and never after
/// a Disconnect.
#[tokio::test]
async fn later_network_retries_end_with_their_window_and_at_a_disconnect() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let plan = [&["drop-after-1"][..], &["unreachable"; 20][..]].concat();
    let f = fixture(
        "late-retry-window",
        &plan,
        with_late_retries(
            fast(Duration::from_millis(30)),
            Duration::from_millis(50),
            Duration::from_millis(200),
        ),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let root = f.root.clone();
    // The first re-dial, three quick retries and four later ones (200 ms / 50 ms).
    until(async || spawns(&root) == 9).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(spawns(&f.root), 9);
    assert_eq!(status(&f.manager).await.0, "disconnected");
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
    drop(f);

    let f = fixture(
        "late-retry-disconnect",
        &plan,
        with_late_retries(
            fast(Duration::from_millis(30)),
            Duration::from_millis(150),
            Duration::from_secs(30),
        ),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let root = f.root.clone();
    until(async || spawns(&root) == 5).await;
    f.manager.disconnect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(spawns(&f.root), 5, "no later retry after a Disconnect");
}

#[test]
fn only_a_refusal_that_names_the_device_or_account_ends_a_membership() {
    use super::keepalive::membership_refused;
    let refusal = |code: &str, message: &str| {
        anyhow::anyhow!(
            "Crew broker refused request: {}",
            json!({"code": code, "message": message})
        )
    };
    assert!(membership_refused(&refusal(
        "unauthorized",
        "unauthorized: unknown device"
    )));
    assert!(membership_refused(&refusal("unauthorized", "unauthorized")));
    assert!(membership_refused(&refusal(
        "principal_revoked",
        "principal_revoked: removed"
    )));
    // A consumed challenge or a missing signature is fixed by a retry, not final.
    for message in [
        "unauthorized: challenge missing or consumed",
        "unauthorized: signature required",
        "unauthorized: invalid grant",
    ] {
        assert!(
            !membership_refused(&refusal("unauthorized", message)),
            "{message}"
        );
    }
    assert!(!membership_refused(&refusal(
        "forbidden",
        "forbidden: channel unavailable"
    )));
    assert!(!membership_refused(&anyhow::anyhow!(
        "unauthorized: unknown device (not the broker's words)"
    )));
}

#[test]
fn the_default_pace_keeps_every_gap_well_inside_the_brokers_timeout() {
    let timing = KeepaliveTiming::default();
    let broker_idle_timeout = Duration::from_secs(300);
    // The longest silence: idle, plus up to one tick before it is noticed, plus one exchange.
    assert!(timing.idle + timing.tick + Duration::from_secs(45) < broker_idle_timeout);
    assert!(timing.probe_before_use > timing.idle + timing.tick);
    assert!(timing.probe_before_use < broker_idle_timeout);
    // Never a tight loop: every retry waits, and each waits longer than the last.
    assert!(timing.retry_delays[0] >= Duration::from_secs(10));
    assert!(timing.retry_delays.windows(2).all(|pair| pair[0] < pair[1]));
    // Q3-11: then every 5 minutes for an hour, never faster than the last quick retry.
    assert_eq!(timing.late_retry_every, Duration::from_secs(300));
    assert_eq!(timing.late_retry_for, Duration::from_secs(3600));
    assert!(timing.late_retry_every > timing.retry_delays[2]);
    let gaps: Vec<Duration> = timing.redial_gaps().collect();
    assert_eq!(gaps.len(), 3 + 12);
    assert_eq!(gaps[..3], timing.retry_delays);
    assert!(gaps[3..].iter().all(|gap| *gap == timing.late_retry_every));
    // The broker's own timeout is the number this pace is measured against.
    let broker = include_str!("../../../biorouter-crew/src/broker.rs");
    assert!(broker.contains("set_read_timeout(Some(Duration::from_secs(300)))"));
}
