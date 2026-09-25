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
/// next line of `plan`: `serve` answers everything; `end-after-N` answers N requests and then
/// exits at once, before anyone writes another, as a bridge whose `ssh` was killed does;
/// `drop-after-N` answers N requests and
/// then ends on the next without answering, as a bridge the broker dropped does
/// (`join-drop-after-1` too, announcing `join_by_name_v1` first); `v2-then-v1` answers its
/// first `hello` v2-signed and every later one v1 only, as a relay stripping the signature
/// would; `other-node-after-1` answers later `hello`s from a different node; `revoked`
/// answers `hello` and `auth.challenge` but refuses every other request as a device the
/// workspace does not know (`unauthorized: unknown device`), and `member-then-revoked` does
/// so after answering the first; `grant-expired` refuses every worker request (one carrying a
/// run credential) as a run the workspace no longer honors (`grant_expired`), and
/// `channel-gone` as a channel the person is no longer in (`forbidden: channel unavailable`);
/// `refuse-revoke` refuses `run.revoke` as a run the workspace does not know; `auth` and
/// `unreachable` fail before any request, as OpenSSH does. `context.manifest` answers [`manifest`]; `blob.read` and `blob.status` answer for
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
  elif [ "$plan" = grant-expired ] && printf '%s\n' "$line" | grep -q '"credential":'; then
    printf '{{"id":"%s","error":{{"code":"grant_expired","message":"grant_expired: run revoked, expired or policy changed"}}}}\n' "$id"
  elif [ "$plan" = channel-gone ] && printf '%s\n' "$line" | grep -q '"credential":'; then
    printf '{{"id":"%s","error":{{"code":"forbidden","message":"forbidden: channel unavailable"}}}}\n' "$id"
  elif [ "$plan" = refuse-revoke ] && printf '%s\n' "$line" | grep -q '"method":"run.revoke"'; then
    printf '{{"id":"%s","error":{{"code":"forbidden","message":"forbidden: owned run unavailable"}}}}\n' "$id"
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
  case "$plan" in
    end-after-*) [ "$answered" -ge "${{plan##*end-after-}}" ] && exit 0 ;;
  esac
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
        ended_check: Duration::from_millis(40),
        revocation_retry_first: Duration::from_millis(40),
        revocation_retry_max: Duration::from_millis(160),
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
            ended_check: Duration::from_secs(600),
            revocation_retry_first: Duration::from_secs(600),
            revocation_retry_max: Duration::from_secs(600),
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
        ended_check: Duration::from_secs(600),
        revocation_retry_first: Duration::from_secs(600),
        revocation_retry_max: Duration::from_secs(600),
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
            revocation: None,
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
            revocation: None,
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
    // the line can tell the two apart: this one, the newest, by when it was shared, in the
    // daemon's local time (Q4-27), never a "newest copy" that is only true today.
    let when = super::shared_when(1_790_214_527, &chrono::Local::now()).unwrap();
    assert_eq!(
        f.manager.posted_source_line(WORKER).await,
        Some(format!(
            "Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina) {when}."
        ))
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

/// Q4-11: a connected chat's first `messages.history` or `messages.search` leaves
/// `channel_id` out (every model did); the grant has one channel, so the daemon sends that one.
/// With several, the model is told to choose, and nothing is sent.
#[tokio::test]
async fn a_one_channel_read_without_channel_id_reads_the_granted_channel() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("default-channel", &["serve"], quiet()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    let cap = CallCapability::for_test(ProviderTier::Private, true);
    for (method, params) in [
        ("messages.history", json!({"limit": 20})),
        ("messages.history", json!({"channel_id": null})),
        ("messages.history", json!({"channel_id": ""})),
        ("messages.search", json!({"query": "assay"})),
    ] {
        f.manager
            .agent_request(WORKER, &cap, CONNECTION_ID, method, params.clone())
            .await
            .unwrap_or_else(|error| panic!("{method} {params}: {error}"));
    }
    let sent: Vec<Value> = frames(&f.root)
        .into_iter()
        .filter(|frame| {
            frame["method"]
                .as_str()
                .is_some_and(|method| method.starts_with("messages."))
        })
        .map(|frame| frame["params"]["channel_id"].clone())
        .collect();
    assert_eq!(sent, vec![json!("keepalive-channel"); 4]);
    // A channel the model names is its own, and still checked against the grant.
    let outside = f
        .manager
        .agent_request(
            WORKER,
            &cap,
            CONNECTION_ID,
            "messages.history",
            json!({"channel_id": "another-channel"}),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        outside.contains("outside the approved context scope"),
        "{outside}"
    );

    // Two granted channels: the model must choose, and is told how, before anything is sent.
    f.manager
        .registry
        .lock()
        .await
        .scopes
        .get_mut(WORKER)
        .unwrap()
        .source_channels
        .push("second-channel".into());
    let before = frames(&f.root).len();
    let choose = f
        .manager
        .agent_request(WORKER, &cap, CONNECTION_ID, "messages.history", json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        choose,
        "messages.history reads one channel at a time: give channel_id, one of the \
         source_channel_ids crew__connections lists."
    );
    assert_eq!(frames(&f.root).len(), before, "nothing sent");
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

/// Nothing in the background notices a drop (no heartbeat, no probe before use, no check for
/// an ended bridge): only a request, or a person, finds it. The retries come every `retry`.
fn request_finds(retry: Duration) -> KeepaliveTiming {
    KeepaliveTiming {
        retry_delays: [retry; 3],
        late_retry_every: retry,
        ..quiet()
    }
}

/// Connect over a bridge whose `ssh` exits after the connect's `hello` (`end-after-1`), and
/// give it a moment to exit.
async fn connect_then_lose_the_bridge(f: &Fixture) {
    f.manager.connect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
}

/// Q4-01, Bob's F3: with Crew on screen, the desktop's own read finds the dead bridge before the
/// keepalive does. Its re-dial meets the network still down; the retries it arms reconnect once
/// the network is back, with no click, and the request itself is never sent again.
#[tokio::test]
async fn a_drop_a_request_finds_first_is_dialled_again_until_the_network_is_back() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "request-finds",
        &["end-after-1", "unreachable", "unreachable", "serve"],
        request_finds(Duration::from_millis(60)),
    )
    .await;
    connect_then_lose_the_bridge(&f).await;
    let refused = f
        .manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap_err();
    assert!(
        refused
            .chain()
            .any(|cause| cause.downcast_ref::<SshFailure>().is_some()),
        "{refused:#}"
    );
    assert_eq!(
        methods_on(&f.root, 1),
        ["hello"],
        "nothing written to the dead bridge"
    );
    let root = f.root.clone();
    let manager = Arc::clone(&f.manager);
    until(async || spawns(&root) == 4 && status(&manager).await == ("connected".to_owned(), None))
        .await;
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
    assert!(
        !requests(&f.root)
            .iter()
            .any(|(_, method)| method == "workspace.snapshot"),
        "the request is never sent again by itself"
    );
    // The new bridge is kept alive in turn: no fourth dial.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(spawns(&f.root), 4);
}

/// Q4-01: a bridge that breaks while carrying a request is a drop that request found. The
/// request is not sent again (its outcome is unknown); the connection is dialled again on the
/// same schedule.
#[tokio::test]
async fn a_bridge_that_breaks_under_a_request_is_dialled_again_later() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "request-breaks",
        &["drop-after-1", "unreachable", "serve"],
        request_finds(Duration::from_millis(60)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let error = f
        .manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.is_empty());
    let root = f.root.clone();
    let manager = Arc::clone(&f.manager);
    until(async || spawns(&root) == 3 && status(&manager).await == ("connected".to_owned(), None))
        .await;
    assert!(
        !requests(&f.root)
            .iter()
            .any(|(_, method)| method == "workspace.snapshot"),
        "never re-sent"
    );
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
}

/// Q4-01: a Disconnect during the retries a request armed stops them for good.
#[tokio::test]
async fn a_disconnect_stops_the_retries_a_request_armed() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "request-finds-disconnect",
        &["end-after-1", "unreachable", "serve"],
        request_finds(Duration::from_millis(400)),
    )
    .await;
    connect_then_lose_the_bridge(&f).await;
    f.manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap_err();
    assert_eq!(spawns(&f.root), 2);
    assert!(!f.manager.idle_redial.lock().unwrap().is_empty(), "armed");
    f.manager.disconnect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(spawns(&f.root), 2, "no retry after a Disconnect");
    assert_eq!(status(&f.manager).await.0, "disconnected");
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
}

/// Q4-01: a request's re-dial that fails for a reason only a person can fix arms nothing.
#[tokio::test]
async fn a_request_s_redial_that_needs_sign_in_schedules_nothing() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "request-finds-auth",
        &["end-after-1", "auth", "serve"],
        request_finds(Duration::from_millis(30)),
    )
    .await;
    connect_then_lose_the_bridge(&f).await;
    f.manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap_err();
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(spawns(&f.root), 2, "no retry: only a person can sign in");
    let (state, error) = status(&f.manager).await;
    assert_eq!(state, "disconnected");
    assert!(
        error
            .as_deref()
            .is_some_and(|error| error.starts_with("Couldn't sign in to example.test as crew")),
        "{error:?}"
    );
}

/// Q4-01: a person's Connect that fails for a network reason keeps being tried, so the network
/// coming back reconnects without a second click; one that needs sign-in is not.
#[tokio::test]
async fn a_connect_that_fails_for_a_network_reason_keeps_trying_by_itself() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "connect-network",
        &["unreachable", "unreachable", "serve"],
        request_finds(Duration::from_millis(60)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap_err();
    let root = f.root.clone();
    let manager = Arc::clone(&f.manager);
    until(async || spawns(&root) == 3 && status(&manager).await == ("connected".to_owned(), None))
        .await;
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
    drop(f);

    let f = fixture(
        "connect-auth",
        &["auth", "serve"],
        request_finds(Duration::from_millis(30)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap_err();
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(spawns(&f.root), 1, "only a person can sign in");
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
}

/// Q4-01: two finders of one drop (a request, then a person's Connect) run one schedule: the
/// second arming replaces the first, which stops at its next try.
#[tokio::test]
async fn two_finders_of_one_drop_run_one_schedule() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let plan = [&["end-after-1"][..], &["unreachable"; 12][..]].concat();
    let f = fixture(
        "two-finders",
        &plan,
        request_finds(Duration::from_millis(300)),
    )
    .await;
    connect_then_lose_the_bridge(&f).await;
    f.manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap_err();
    f.manager.connect(CONNECTION_ID).await.unwrap_err();
    assert_eq!(spawns(&f.root), 3);
    let root = f.root.clone();
    // The Connect's three retries, and none of the request's.
    until(async || spawns(&root) == 6).await;
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(spawns(&f.root), 6, "one schedule, not two");
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
}

/// Q4-01 keeps Q3-12: a device the workspace no longer knows is never dialled again by
/// itself, whoever meets the network down afterwards; a person's Connect still dials once.
#[tokio::test]
async fn a_revoked_device_is_never_scheduled() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "revoked-not-scheduled",
        &["member-then-revoked", "unreachable", "serve"],
        request_finds(Duration::from_millis(60)),
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    f.manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap();
    f.manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap_err();
    assert!(membership_ended(&f.manager).await);
    // A request finds no bridge and dials nothing.
    f.manager
        .human_request(CONNECTION_ID, "workspace.snapshot", json!({}), None)
        .await
        .unwrap_err();
    assert_eq!(spawns(&f.root), 1);
    // A person's Connect meets the network down: nothing is armed after it.
    f.manager.connect(CONNECTION_ID).await.unwrap_err();
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(spawns(&f.root), 2, "never dialled again by itself");
    assert!(f.manager.idle_redial.lock().unwrap().is_empty());
}

/// Q4-08: a bridge whose `ssh` exited is noticed between ticks, from local process state
/// alone, and dialled again, long before the next tick.
#[tokio::test]
async fn an_ended_bridge_is_noticed_between_ticks() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture(
        "ended-check",
        &["end-after-1", "serve"],
        KeepaliveTiming {
            ended_check: Duration::from_millis(40),
            ..quiet()
        },
    )
    .await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let root = f.root.clone();
    let manager = Arc::clone(&f.manager);
    until(async || spawns(&root) == 2 && status(&manager).await == ("connected".to_owned(), None))
        .await;
    // Nothing was sent to notice it: the old bridge carried only the connect's hello.
    assert_eq!(methods_on(&f.root, 1), ["hello"]);
    drop(f);

    // Without the check (the tick and idle times far off), nothing notices it.
    let f = fixture("no-ended-check", &["end-after-1", "serve"], quiet()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(spawns(&f.root), 1);
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
    // Q4-08: an ended bridge is looked for every few seconds between ticks.
    assert_eq!(timing.ended_check, Duration::from_secs(5));
    assert!(timing.ended_check < timing.tick);
    let gaps: Vec<Duration> = timing.redial_gaps().collect();
    assert_eq!(gaps.len(), 3 + 12);
    assert_eq!(gaps[..3], timing.retry_delays);
    assert!(gaps[3..].iter().all(|gap| *gap == timing.late_retry_every));
    // The broker's own timeout is the number this pace is measured against.
    let broker = include_str!("../../../biorouter-crew/src/broker.rs");
    assert!(broker.contains("set_read_timeout(Some(Duration::from_secs(300)))"));
}

/// A provider bound to a scoped chat, for the turn-start check (`check_provider_dispatch`).
struct TurnProvider;

#[async_trait::async_trait]
impl crate::providers::base::Provider for TurnProvider {
    fn metadata() -> crate::providers::base::ProviderMetadata {
        crate::providers::base::ProviderMetadata::new(
            "keepalive-turn",
            "Keepalive turn",
            "",
            "fixture",
            vec![],
            "",
            vec![],
        )
    }
    fn get_name(&self) -> &str {
        "keepalive-turn"
    }
    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }
    async fn complete_with_model(
        &self,
        _model_config: &crate::model::ModelConfig,
        _system: &str,
        _messages: &[crate::conversation::message::Message],
        _tools: &[rmcp::model::Tool],
    ) -> std::result::Result<
        (
            crate::conversation::message::Message,
            crate::providers::base::ProviderUsage,
        ),
        crate::providers::errors::ProviderError,
    > {
        unreachable!("the turn-start check never calls the model")
    }
    fn get_model_config(&self) -> crate::model::ModelConfig {
        crate::model::ModelConfig::new_or_fail("keepalive-turn-model")
    }
}

/// The saved registry's copy of `WORKER`'s grant.
fn saved_grant(f: &Fixture) -> Value {
    let saved: Value = serde_json::from_slice(
        &fs::read(f.root.join("manager").join("connections.json")).expect("the registry is saved"),
    )
    .unwrap();
    saved["scopes"][WORKER].clone()
}

/// `WORKER`'s row in the grants list.
async fn listed_grant(manager: &CrewManager) -> Value {
    let listed = manager.session_grants(CONNECTION_ID).await.unwrap();
    listed["grants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session_id"] == WORKER)
        .cloned()
        .expect("the grant is listed")
}

/// The requests any bridge received that carried the run's credential: the worker's own.
fn worker_frames(root: &Path) -> usize {
    frames(root)
        .iter()
        .filter(|frame| frame.get("credential").is_some())
        .count()
}

/// D-1: a policy change at the workspace (a channel's membership toggled) moves its epoch, and
/// the broker refuses the run as `grant_expired`. That used to reach the person as the broker's
/// raw envelope while the grant still read live. Now the refusal is the sentence a policy
/// change on this device gets, the grant stops here (and is saved stopped, as ended by the
/// workspace), the list says so, and nothing more is sent under it: no second worker request
/// and no revocation, because the workspace already refused the run.
#[tokio::test]
async fn a_run_the_workspace_ended_stops_here_with_the_policy_sentence() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("grant-expired", &["grant-expired"], quiet()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    let cap = CallCapability::for_test(ProviderTier::Private, true);
    let refused = f
        .manager
        .agent_request(WORKER, &cap, CONNECTION_ID, "context.manifest", json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(refused, GRANT_POLICY_CHANGED);
    assert!(!refused.contains('{') && !refused.contains("grant_expired"));
    assert_eq!(worker_frames(&f.root), 1);

    let stopped = f.manager.registry.lock().await.scopes[WORKER].clone();
    assert!(stopped.expired);
    assert_eq!(stopped.revocation, Some(Revocation::EndedByWorkspace));
    let saved = saved_grant(&f);
    assert_eq!(saved["expired"], true);
    assert_eq!(saved["revocation"], "ended_by_workspace");
    let listed = listed_grant(&f.manager).await;
    assert_eq!(listed["expired"], true);
    assert_eq!(listed["revocation"], "ended_by_workspace");
    assert!(listed["remote_revocation_confirmed"].is_null());

    // Every later use is refused here, in the same words, without asking the workspace.
    for _ in 0..2 {
        assert_eq!(
            f.manager
                .agent_request(WORKER, &cap, CONNECTION_ID, "messages.history", json!({}))
                .await
                .unwrap_err()
                .to_string(),
            GRANT_POLICY_CHANGED
        );
        assert_eq!(
            f.manager
                .check_dispatch(WORKER, &cap)
                .await
                .unwrap_err()
                .to_string(),
            GRANT_POLICY_CHANGED
        );
    }
    assert_eq!(worker_frames(&f.root), 1);
    assert!(
        !requests(&f.root)
            .iter()
            .any(|(_, method)| method == "run.revoke"),
        "the workspace ended the run itself; nothing is revoked there"
    );
    // A person's revoke afterwards is still theirs to make, and reads as a revocation.
    f.manager.revoke_session(WORKER).await.unwrap();
    assert_eq!(
        f.manager
            .check_dispatch(WORKER, &cap)
            .await
            .unwrap_err()
            .to_string(),
        GRANT_REVOKED
    );
}

/// D-1 at turn start: the check every model call makes on a scoped chat reads the workspace,
/// and the `grant_expired` answer is the policy sentence there too (the CLI's and the chat's
/// "Model request failed"), never `Crew broker refused request: {…}`.
#[tokio::test]
async fn a_turn_the_workspace_refuses_says_so_in_words() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("turn-grant-expired", &["grant-expired"], quiet()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    let provider = TurnProvider;
    f.manager
        .registry
        .lock()
        .await
        .scopes
        .get_mut(WORKER)
        .unwrap()
        .provider_binding = provider_binding(&provider);
    let refused = f
        .manager
        .check_provider_dispatch(WORKER, &provider)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(refused, GRANT_POLICY_CHANGED);
    assert_eq!(listed_grant(&f.manager).await["expired"], true);
    // The next turn is refused before anything is sent.
    assert_eq!(
        f.manager
            .check_provider_dispatch(WORKER, &provider)
            .await
            .unwrap_err()
            .to_string(),
        GRANT_POLICY_CHANGED
    );
    assert_eq!(worker_frames(&f.root), 1);
}

/// Any other refusal the workspace answers at turn start is a sentence too, and does not stop
/// the grant: only `grant_expired` means the workspace ended the run.
#[tokio::test]
async fn another_refusal_at_turn_start_is_a_sentence_and_leaves_the_grant_alone() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("turn-channel-gone", &["channel-gone"], quiet()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    let provider = TurnProvider;
    f.manager
        .registry
        .lock()
        .await
        .scopes
        .get_mut(WORKER)
        .unwrap()
        .provider_binding = provider_binding(&provider);
    let refused = f
        .manager
        .check_provider_dispatch(WORKER, &provider)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(refused, "Crew refused the request: channel unavailable.");
    let listed = listed_grant(&f.manager).await;
    assert_eq!(listed["expired"], false);
    assert!(listed["revocation"].is_null());
}

/// A run that simply ran out of time is refused as `grant_expired` too. It is not a policy
/// change and not a revocation: its own sentence, and the grant is left as it is, since its
/// `expires_at` already reads Expired everywhere.
#[tokio::test]
async fn a_run_that_ran_out_of_time_says_so_and_is_not_marked_revoked() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("grant-timed-out", &["grant-expired"], quiet()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    f.manager
        .registry
        .lock()
        .await
        .scopes
        .get_mut(WORKER)
        .unwrap()
        .expires_at = Some(1);
    let refused = f
        .manager
        .worker_request(WORKER, "context.manifest", json!({}))
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(refused, GRANT_TIMED_OUT);
    let listed = listed_grant(&f.manager).await;
    assert_eq!(listed["expired"], false);
    assert!(listed["revocation"].is_null());
}

/// F3: a revoke while the workspace cannot be reached stops the grant here and says the
/// workspace has not confirmed; the grants list says so too. When a person connects again,
/// the daemon asks the workspace to revoke the run by itself, with no Retry: the list then
/// reads confirmed, and the run is revoked exactly once.
#[tokio::test]
async fn an_unconfirmed_revocation_is_confirmed_by_itself_on_reconnect() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("revoke-reconnect", &["serve", "serve"], quiet()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    f.manager.disconnect(CONNECTION_ID).await.unwrap();
    let outcome = f.manager.revoke_session(WORKER).await.unwrap();
    assert!(!outcome.remote_confirmed);
    let listed = listed_grant(&f.manager).await;
    assert_eq!(listed["expired"], true);
    assert_eq!(listed["revocation"], "unconfirmed");
    assert_eq!(listed["remote_revocation_confirmed"], false);
    assert_eq!(saved_grant(&f)["revocation"], "unconfirmed");
    // Refused here at once, whatever the workspace says.
    let cap = CallCapability::for_test(ProviderTier::Private, true);
    assert_eq!(
        f.manager
            .check_dispatch(WORKER, &cap)
            .await
            .unwrap_err()
            .to_string(),
        GRANT_REVOKED
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!requests(&f.root)
        .iter()
        .any(|(_, method)| method == "run.revoke"));

    f.manager.connect(CONNECTION_ID).await.unwrap();
    let manager = f.manager.clone();
    until(async || listed_grant(&manager).await["remote_revocation_confirmed"] == true).await;
    let listed = listed_grant(&f.manager).await;
    assert_eq!(listed["revocation"], "confirmed");
    assert_eq!(listed["expired"], true);
    assert_eq!(saved_grant(&f)["revocation"], "confirmed");
    let revokes: Vec<Value> = frames(&f.root)
        .into_iter()
        .filter(|frame| frame["method"] == "run.revoke")
        .collect();
    assert_eq!(revokes.len(), 1, "{revokes:?}");
    assert_eq!(revokes[0]["params"]["run_id"], "keepalive-run");
    assert_eq!(
        methods_on(&f.root, 2).last().map(String::as_str),
        Some("run.revoke")
    );

    // Confirmed is final: a later connect asks nothing again.
    f.manager.disconnect(CONNECTION_ID).await.unwrap();
    f.manager.connect(CONNECTION_ID).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        frames(&f.root)
            .iter()
            .filter(|frame| frame["method"] == "run.revoke")
            .count(),
        1
    );
}

/// F3 without anyone pressing anything: the revoke's own request loses its bridge, the
/// keepalive's re-dial brings the connection back, and that connect asks the workspace again.
#[tokio::test]
async fn a_keepalive_redial_confirms_an_unconfirmed_revocation() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let timing = KeepaliveTiming {
        retry_delays: [Duration::from_millis(50); 3],
        ..quiet()
    };
    let f = fixture("revoke-redial", &["drop-after-1", "serve"], timing).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    let outcome = f.manager.revoke_session(WORKER).await.unwrap();
    assert!(!outcome.remote_confirmed, "the bridge dropped mid-request");
    let manager = f.manager.clone();
    until(async || listed_grant(&manager).await["remote_revocation_confirmed"] == true).await;
    assert!(spawns(&f.root) >= 2);
    assert_eq!(
        methods_on(&f.root, spawns(&f.root))
            .last()
            .map(String::as_str),
        Some("run.revoke")
    );
    assert_eq!(status(&f.manager).await, ("connected".into(), None));
}

/// A run the workspace answered and refused to revoke is not asked about again and again
/// while the connection stays up; it stays unconfirmed, is shown so, and is asked again at the
/// next reconnect.
#[tokio::test]
async fn a_revocation_the_workspace_refused_is_asked_again_only_at_reconnect() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let timing = KeepaliveTiming {
        revocation_retry_first: Duration::from_millis(20),
        revocation_retry_max: Duration::from_millis(40),
        ..quiet()
    };
    let f = fixture("revoke-refused", &["refuse-revoke", "serve"], timing).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    let outcome = f.manager.revoke_session(WORKER).await.unwrap();
    assert!(!outcome.remote_confirmed);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let revokes = |root: &Path| {
        requests(root)
            .iter()
            .filter(|(_, method)| method == "run.revoke")
            .count()
    };
    assert_eq!(revokes(&f.root), 1);
    assert_eq!(listed_grant(&f.manager).await["revocation"], "unconfirmed");

    f.manager.disconnect(CONNECTION_ID).await.unwrap();
    f.manager.connect(CONNECTION_ID).await.unwrap();
    let manager = f.manager.clone();
    until(async || listed_grant(&manager).await["revocation"] == "confirmed").await;
    assert_eq!(revokes(&f.root), 2);
}

/// F3 across a restart: the stop is saved as unconfirmed, and the next daemon to connect
/// asks the workspace again by itself.
#[tokio::test]
async fn an_unconfirmed_revocation_survives_a_restart_and_is_asked_again() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("revoke-restart", &["serve", "serve"], quiet()).await;
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    f.manager.disconnect(CONNECTION_ID).await.unwrap();
    assert!(
        !f.manager
            .revoke_session(WORKER)
            .await
            .unwrap()
            .remote_confirmed
    );
    assert_eq!(saved_grant(&f)["revocation"], "unconfirmed");

    let restarted = CrewManager::shared(f.root.join("manager")).unwrap();
    restarted.set_keepalive_timing(quiet());
    assert_eq!(listed_grant(&restarted).await["revocation"], "unconfirmed");
    restarted.connect(CONNECTION_ID).await.unwrap();
    let manager = restarted.clone();
    until(async || listed_grant(&manager).await["revocation"] == "confirmed").await;
    assert_eq!(saved_grant(&f)["revocation"], "confirmed");
    restarted.disconnect(CONNECTION_ID).await.unwrap();
}

/// A confirmation heard by one process is never replaced by another's "not yet" when their
/// registries meet (D8), whichever way round.
#[test]
fn a_confirmed_revocation_is_never_forgotten_across_processes() {
    let registry = |revocation: Option<Revocation>| {
        let mut scope = Scope {
            connection_id: CONNECTION_ID.into(),
            run_id: "keepalive-run".into(),
            channel_id: "keepalive-channel".into(),
            source_channels: vec![],
            epoch: 1,
            provider_binding: "keepalive-provider".into(),
            public_provider: false,
            origin_restricted: false,
            institution_ids: BTreeSet::new(),
            institution_policy: true,
            expired: true,
            expires_at: None,
            labels: None,
            session_incarnation: None,
            revocation: None,
        };
        scope.revocation = revocation;
        Registry {
            connections: vec![],
            scopes: HashMap::from([(WORKER.to_owned(), scope)]),
            pending_device: None,
            completed_preparations: HashMap::new(),
        }
    };
    for (here, theirs, merged) in [
        (
            Some(Revocation::Confirmed),
            Some(Revocation::Unconfirmed),
            Some(Revocation::Confirmed),
        ),
        (
            Some(Revocation::Unconfirmed),
            Some(Revocation::Confirmed),
            Some(Revocation::Confirmed),
        ),
        (
            Some(Revocation::Unconfirmed),
            None,
            Some(Revocation::Unconfirmed),
        ),
        (
            None,
            Some(Revocation::EndedByWorkspace),
            Some(Revocation::EndedByWorkspace),
        ),
        (
            Some(Revocation::EndedByWorkspace),
            Some(Revocation::Unconfirmed),
            Some(Revocation::EndedByWorkspace),
        ),
    ] {
        let mut file = registry(theirs);
        carry_process_state(&registry(here), &mut file);
        assert_eq!(
            file.scopes[WORKER].revocation, merged,
            "{here:?} over {theirs:?}"
        );
    }
}

/// P-1: saving a connection exactly as it is (`privacy set-personal` with the mode it already
/// has) is not a save: the policy epoch stays, the bridge stays up, and every grant keeps
/// working. A save that does change something still ends the grants, as it must.
#[tokio::test]
async fn saving_a_connection_unchanged_changes_nothing() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let f = fixture("unchanged-save", &["serve", "serve"], quiet()).await;
    // A save checks the server group's ID, which the other tests never need.
    f.manager.registry.lock().await.connections[0].cluster_connection_id =
        "5a5a5a5a-5a5a-45a5-85a5-5a5a5a5a5a5a".into();
    f.manager.connect(CONNECTION_ID).await.unwrap();
    grant_worker(&f).await;
    let before = f.manager.connection(CONNECTION_ID).await.unwrap();
    let bridge = f.manager.transport(CONNECTION_ID).await.unwrap();
    let same = |c: &Connection, mode: ClusterMode| SaveConnection {
        preparation_id: None,
        name: c.name.clone(),
        ssh_target: c.ssh_target.clone(),
        port: c.port,
        identity_file: c.identity_file.clone(),
        proxy_jump: c.proxy_jump.clone(),
        socket_path: c.socket_path.clone(),
        owner_uid: c.owner_uid,
        workspace_id: c.workspace_id.clone(),
        workspace_public_key: c.workspace_public_key.clone(),
        remote_root: c.remote_root.clone(),
        remote_execution: c.remote_execution,
        cluster_connection_id: Some(c.cluster_connection_id.clone()),
        mode,
        institution_id: c.institution_id.clone(),
    };
    for cluster in [true, false] {
        let mut input = same(&before, before.mode);
        if !cluster {
            input.cluster_connection_id = None;
        }
        let answered = f.manager.update(CONNECTION_ID, input).await.unwrap();
        assert_eq!(answered.policy_epoch, before.policy_epoch);
        assert_eq!(answered.status, "connected");
    }
    let after = f.manager.connection(CONNECTION_ID).await.unwrap();
    assert_eq!(after.policy_epoch, before.policy_epoch);
    assert_eq!(after.status, "connected");
    assert!(Arc::ptr_eq(
        &bridge,
        &f.manager.transport(CONNECTION_ID).await.unwrap()
    ));
    assert_eq!(spawns(&f.root), 1, "the bridge was never dropped");
    let listed = listed_grant(&f.manager).await;
    assert_eq!(listed["expired"], false);
    let cap = CallCapability::for_test(ProviderTier::Private, true);
    f.manager.check_dispatch(WORKER, &cap).await.unwrap();

    // A refused edit changes nothing either, the bridge included.
    let mut refused = same(&before, before.mode);
    refused.ssh_target = "bad target;".into();
    assert!(f.manager.update(CONNECTION_ID, refused).await.is_err());
    assert_eq!(
        f.manager.connection(CONNECTION_ID).await.unwrap().status,
        "connected"
    );

    // A real change is a save: the epoch moves, the bridge drops, and the grant ends.
    let mut renamed = same(&before, before.mode);
    renamed.name = "renamed fixture".into();
    let saved = f.manager.update(CONNECTION_ID, renamed).await.unwrap();
    assert!(saved.policy_epoch > before.policy_epoch);
    assert_eq!(saved.status, "disconnected");
    assert_eq!(
        f.manager
            .check_dispatch(WORKER, &cap)
            .await
            .unwrap_err()
            .to_string(),
        GRANT_POLICY_CHANGED
    );
}
