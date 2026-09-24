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
/// next line of `plan`: `serve` answers everything; `drop-after-1` answers one request and
/// then ends on the next without answering, as a bridge the broker dropped does
/// (`join-drop-after-1` too, announcing `join_by_name_v1` first); `v2-then-v1` answers its
/// first `hello` v2-signed and every later one v1 only, as a relay stripping the signature
/// would; `other-node-after-1` answers later `hello`s from a different node; `auth` and
/// `unreachable` fail before any request, as OpenSSH does. Every request line is logged as
/// `<spawn> <line>` to `requests.log`.
fn write_fake_ssh(root: &Path, plan: &[&str]) {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(root.join("plan"), format!("{}\n", plan.join("\n"))).unwrap();
    let hello = hello().to_string();
    let hello_v2 = signed_hello(NODE, true, &["human_chat"]).to_string();
    let hello_other = signed_hello(&"5d".repeat(32), false, &["human_chat"]).to_string();
    let hello_join = signed_hello(NODE, false, &["human_chat", "join_by_name_v1"]).to_string();
    for text in [&hello, &hello_v2, &hello_other, &hello_join] {
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
while IFS= read -r line; do
  printf '%s %s\n' "$n" "$line" >> "$root/requests.log"
  case "$plan" in
    *drop-after-1) [ "$answered" -ge 1 ] && exit 0 ;;
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
  else
    printf '{{"id":"%s","result":{{"accepted_method":"fixture"}}}}\n' "$id"
  fi
done
"#,
        root = root.display(),
    );
    let ssh = bin.join("ssh");
    fs::write(&ssh, script).unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
}

fn spawns(root: &Path) -> usize {
    fs::read_to_string(root.join("spawns"))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
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
    // The broker's own timeout is the number this pace is measured against.
    let broker = include_str!("../../../biorouter-crew/src/broker.rs");
    assert!(broker.contains("set_read_timeout(Some(Duration::from_secs(300)))"));
}
