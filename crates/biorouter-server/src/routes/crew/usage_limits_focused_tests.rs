//! Runtime-shaped US-11 / CREW-28 task-end fixtures. Only the signed synthetic
//! bridge and an in-memory provider are used; no real SSH or inference is needed.

#![cfg(unix)]

use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use biorouter::agents::usage_limit::{self, ChatLimitOverride};
use biorouter::agents::{Agent, AgentConfig};
use biorouter::config::permission::PermissionManager;
use biorouter::config::BioRouterMode;
use biorouter::privacy::ProviderTier;
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
use biorouter::providers::errors::ProviderError;
use biorouter::session::SessionManager;
use rmcp::model::CallToolRequestParams;
use serial_test::serial;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;
use tower::ServiceExt;

const CHANNEL: &str = "focused-crew-usage-channel";
const SECRET: &str = "focused-crew-end-daemon-secret";
const PARTIAL: &str = "Synthetic partial task output must never be posted as completed.";
const FINAL: &str = "Synthetic task control completed.";

struct CountingTaskProvider {
    calls: AtomicUsize,
    entered: Notify,
    release: Notify,
}

#[async_trait::async_trait]
impl Provider for CountingTaskProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "focused-crew-usage-provider"
    }

    fn tier(&self) -> ProviderTier {
        ProviderTier::Public
    }

    fn get_model_config(&self) -> ModelConfig {
        ModelConfig::new_or_fail("focused-crew-usage-model")
    }

    async fn complete_with_model(
        &self,
        _: &ModelConfig,
        _: &str,
        _: &[Message],
        _: &[rmcp::model::Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            call < 2,
            "the fixture must never request a third model answer"
        );
        let (message, input, output) = if call == 0 {
            self.entered.notify_one();
            self.release.notified().await;
            (
                Message::assistant().with_text(PARTIAL).with_tool_request(
                    "focused-usage-unavailable-tool",
                    Ok(CallToolRequestParams {
                        task: None,
                        name: "focused_fixture_unavailable_tool".into(),
                        arguments: None,
                        meta: None,
                    }),
                ),
                6,
                1,
            )
        } else {
            (Message::assistant().with_text(FINAL), 2, 1)
        };
        Ok((
            message,
            ProviderUsage::new(
                "focused-crew-usage-model".into(),
                Usage {
                    input_tokens: Some(input),
                    output_tokens: Some(output),
                    total_tokens: Some(input + output),
                    cache_read_input_tokens: Some(0),
                    cache_creation_input_tokens: Some(0),
                    ..Usage::default()
                },
            ),
        ))
    }
}

async fn person_sets_one_token_limit(state: &Arc<AppState>, session: &str) {
    use crate::routes::session::diverge_tests::{
        install_test_user_action_key, TEST_USER_ACTION_KEY,
    };
    install_test_user_action_key();
    let response = crate::routes::configure(state.clone(), SECRET.into())
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/sessions/{session}/usage-limit"))
                .header("X-Secret-Key", SECRET)
                .header("X-User-Action", TEST_USER_ACTION_KEY)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"mode": "limit", "maxTokens": 1}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
}

async fn execute_fixture_task(root: &std::path::Path, limit_while_waiting: bool) {
    let state = AppState::new().await.unwrap();
    let crew = biorouter::crew::manager().unwrap();
    let connection = signed_bridge::workspace(root).await;
    let manager = Arc::new(SessionManager::instance());
    let session = manager
        .create_session(
            root.to_path_buf(),
            "Focused Crew task usage".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    let provider = Arc::new(CountingTaskProvider {
        calls: AtomicUsize::new(0),
        entered: Notify::new(),
        release: Notify::new(),
    });
    let admission = crew
        .begin_run(
            &session.id,
            &connection.id,
            CHANNEL,
            Vec::new(),
            provider.as_ref(),
        )
        .await
        .unwrap();
    assert!(crew.is_scoped_session(&session.id).await);
    assert!(crew.history_rewrite_refusal(&session.id).await.is_none());
    let agent = Agent::with_config(
        AgentConfig::new(
            manager.clone(),
            Arc::new(PermissionManager::new(root.to_path_buf())),
            None,
            BioRouterMode::Auto,
        )
        .with_project_hooks(false),
    );
    agent
        .update_provider(provider.clone(), &session.id)
        .await
        .unwrap();
    usage_limit::set_for_session(&manager, &session.id, Some(ChatLimitOverride::Unlimited))
        .await
        .unwrap();
    let view = RunView {
        run_id: admission.run_id,
        connection_id: connection.id.clone(),
        channel_id: CHANNEL.into(),
        session_id: session.id.clone(),
        status: "running".into(),
        error: None,
        started_at: Some(unix_millis()),
    };
    let input = RunInput {
        prompt: "Exercise only the synthetic unavailable tool, then return the fixture result."
            .into(),
        context: admission.context,
        labels: admission.labels,
    };
    let ledger = run_ledger().await.unwrap();
    let cancel = CancellationToken::new();
    record_starting_run(&ledger, "focused-end-fixture", &view, &cancel)
        .await
        .unwrap();
    let execute = execute_run(&ledger, &state, &agent, &view, &input, &cancel);
    let change_limit = async {
        tokio::time::timeout(Duration::from_secs(10), provider.entered.notified())
            .await
            .expect("the first synthetic provider request must actually start");
        if limit_while_waiting {
            person_sets_one_token_limit(&state, &session.id).await;
        }
        provider.release.notify_one();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(execute, change_limit)
    })
    .await
    .expect("the bounded synthetic task must settle");
    let posts = signed_bridge::sent(root, "run.project");
    let completed: Vec<_> = posts
        .iter()
        .filter(|frame| frame["params"]["status"] == "completed")
        .collect();
    let usage = manager.get_session_model_usage(&session.id).await.unwrap();
    assert_eq!(usage.len(), 1, "{usage:?}");
    if limit_while_waiting {
        assert_eq!(
            result.unwrap_err().to_string(),
            usage_limit::CREW_TASK_STOPPED_AT_LIMIT
        );
        assert!(agent.usage_limit_interrupted_reply());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
        assert_eq!(usage[0].turns, 1);
        assert_eq!(usage[0].total_tokens, Some(7));
        assert!(
            completed.is_empty(),
            "a partial task became a completed post: {posts:?}"
        );
    } else {
        result.unwrap();
        assert!(!agent.usage_limit_interrupted_reply());
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
        assert_eq!(usage[0].turns, 2);
        assert_eq!(usage[0].total_tokens, Some(10));
        assert_eq!(completed.len(), 1, "{posts:?}");
        assert!(completed[0]["params"]["body"]
            .as_str()
            .unwrap()
            .contains(FINAL));
    }
    assert!(
        !posts.is_empty(),
        "the bridge must observe actual task projection"
    );
    assert!(!signed_bridge::sent(root, "hello").is_empty());
    assert!(!signed_bridge::sent(root, "auth.challenge").is_empty());
    let admitted = signed_bridge::sent(root, "run.create");
    assert_eq!(admitted.len(), 1);
    assert_eq!(admitted[0]["has_auth"], true);
    let manifests = signed_bridge::sent(root, "context.manifest");
    assert!(!manifests.is_empty());
    assert!(manifests
        .iter()
        .all(|frame| frame["has_credential"] == true));
    assert!(posts.iter().all(|frame| frame["has_credential"] == true));
    crew.remove(&connection.id).await.unwrap();
}

async fn isolated_task_fixture(limit_while_waiting: bool) {
    let root = tempfile::TempDir::new().unwrap();
    let home = root.path().join("home");
    let profile = root.path().join("profile");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&profile).unwrap();
    let bin = signed_bridge::prepare(root.path());
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let _env = crate::test_sandbox::relocate_path_root_and_home(
        root.path(),
        &home,
        [
            ("PATH", Some(path)),
            ("BIOROUTER_DISABLE_KEYRING", Some("true".into())),
            (
                "BIOROUTER_DEV_PROFILE_ROOT",
                Some(profile.to_str().unwrap().into()),
            ),
        ],
    );
    execute_fixture_task(root.path(), limit_while_waiting).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn crew_usage_tiny_limit_stops_a_task_without_publishing_its_partial_result() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    isolated_task_fixture(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn crew_usage_unlimited_control_posts_one_completed_result_after_two_requests() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    isolated_task_fixture(false).await;
}

mod signed_bridge {
    use biorouter::crew::Connection;
    use serde_json::{json, Value};
    use std::io::{BufRead, Write};
    use std::path::{Path, PathBuf};

    const FAKE_SSH_ROOT: &str = "BIOROUTER_FOCUSED_CREW_USAGE_FAKE_SSH_ROOT";
    const HOST: &str = "focused-crew-usage.invalid";
    const HOST_UID: u32 = 1000;
    const JOINER_UID: u32 = 1001;
    const WORKSPACE: &str = "f5b576cc-bdf0-4df0-9da7-05820caf6f42";
    const NODE: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    const SOCKET: &str = "/tmp/crew-1000-0123456789abcdef0123456789abcdef/broker.sock";
    pub(super) const CHANNEL: &str = "focused-crew-usage-channel";
    const CAPABILITIES: [&str; 4] = [
        "human_names_v1",
        "unique_names_v1",
        "join_by_name_v1",
        "account_identity_v1",
    ];

    mod workspace_key {
        use rustls::crypto::ring::sign::any_eddsa_type;
        use rustls::pki_types::PrivatePkcs8KeyDer;
        use rustls::sign::SigningKey;
        use rustls::SignatureScheme;
        use std::sync::Arc;

        const PREFIX: [u8; 16] = [
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        pub struct Key(Arc<dyn SigningKey>);

        impl Key {
            pub fn new() -> Self {
                let mut der = PREFIX.to_vec();
                der.extend_from_slice(&[19; 32]);
                Self(any_eddsa_type(&PrivatePkcs8KeyDer::from(der)).expect("synthetic Ed25519 key"))
            }
            pub fn public_hex(&self) -> String {
                let spki = self.0.public_key().expect("Ed25519 public key");
                let bytes: &[u8] = spki.as_ref();
                hex::encode(&bytes[bytes.len() - 32..])
            }
            pub fn sign_hex(&self, message: &[u8]) -> String {
                let signer = self
                    .0
                    .choose_scheme(&[SignatureScheme::ED25519])
                    .expect("Ed25519 signer");
                hex::encode(signer.sign(message).expect("synthetic signature"))
            }
        }
    }

    fn hello(nonce: &str) -> Value {
        let key = workspace_key::Key::new();
        let public = key.public_hex();
        let mode = biorouter_crew::Mode::Public;
        let v1 = key.sign_hex(&biorouter_crew::hello_v1_payload(
            WORKSPACE, HOST_UID, nonce, &public, NODE,
        ));
        let v2 = key.sign_hex(
            &biorouter_crew::HelloV2 {
                workspace_id: WORKSPACE,
                host_uid: HOST_UID,
                challenge_nonce: nonce,
                workspace_public_key: &public,
                node_id: NODE,
                mode: &mode,
                institution_id: None,
                policy_epoch: 1,
                name: Some("focused-usage-lab"),
                capabilities: &CAPABILITIES,
            }
            .signing_payload(),
        );
        json!({
            "protocol": 1, "workspace_id": WORKSPACE, "host_uid": HOST_UID,
            "mode": mode, "institution_id": null, "policy_epoch": 1,
            "name": "focused-usage-lab", "workspace_public_key": public,
            "node_id": NODE, "challenge_nonce": nonce,
            "signature": v1, "signature_v2": v2, "capabilities": CAPABILITIES,
        })
    }

    fn snapshot() -> Value {
        json!({
            "workspace": {
                "id": WORKSPACE, "name": "focused-usage-lab", "mode": "public",
                "institution_id": null, "policy_epoch": 1,
                "host_principal_id": "focused-principal-alice",
            },
            "actor": {
                "id": "focused-principal-alice", "username": "alice",
                "display_name": "Synthetic Alice",
                "devices": [{"fingerprint": "FOCUSED SYNTHETIC DEVICE",
                    "added_via": "invitation", "added_at": 1, "this_device": true}],
            },
            "principals": [{"id": "focused-principal-alice", "username": "alice",
                "display_name": "Synthetic Alice"}],
            "teams": [], "channels": [{"id": CHANNEL, "name": "general"}],
            "protected_channel_ids": [], "invitations": [], "context": null,
        })
    }

    fn log_path(root: &Path) -> PathBuf {
        root.join("signed-bridge-requests.jsonl")
    }

    fn append_request(root: &Path, frame: &Value) -> std::io::Result<()> {
        let saved = json!({
            "id": frame["id"], "method": frame["method"], "params": frame["params"],
            "has_auth": frame.get("auth").is_some_and(|value| !value.is_null()),
            "has_credential": frame.get("credential").is_some_and(|value| !value.is_null()),
        });
        let mut bytes = serde_json::to_vec(&saved)?;
        bytes.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path(root))?
            .write_all(&bytes)
    }

    fn bridge(root: &Path) -> i32 {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout().lock();
        let mut nonces = 0u64;
        let mut runs = 0u64;
        for line in stdin.lock().lines() {
            let Ok(line) = line else { return 1 };
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                return 1;
            };
            if append_request(root, &frame).is_err() {
                return 1;
            }
            let mut envelope = match frame["method"].as_str().unwrap_or_default() {
                "hello" => json!({"result": hello(
                    frame["params"]["challenge_nonce"].as_str().unwrap_or_default()
                )}),
                "auth.challenge" => {
                    nonces += 1;
                    json!({"result": {
                        "nonce": format!("focused-nonce-{}-{nonces}", std::process::id()),
                        "workspace_id": WORKSPACE, "uid": JOINER_UID,
                        "expires_at": 4_000_000_000u64,
                    }})
                }
                "workspace.snapshot" => json!({"result": snapshot()}),
                "run.create" => {
                    runs += 1;
                    json!({"result": {
                        "run": {
                            "id": format!("focused-run-{}-{runs}", std::process::id()),
                            "protected_context": false, "expires_at": 4_000_000_000u64,
                        },
                        "credential": "focused-synthetic-scoped-credential",
                    }})
                }
                "messages.history" | "context.manifest" => {
                    json!({"result": {"messages": [], "files": []}})
                }
                "run.project" => json!({"result": {"posted": true}}),
                "run.revoke" => json!({"result": {"revoked": true}}),
                _ => json!({"error": {"code": "unsupported",
                    "message": "unsupported synthetic fixture method"}}),
            };
            envelope["id"] = frame["id"].clone();
            if writeln!(stdout, "{envelope}")
                .and_then(|()| stdout.flush())
                .is_err()
            {
                return 1;
            }
        }
        0
    }

    fn fake_ssh(root: &Path) -> i32 {
        const WITH_VALUE: &[&str] = &[
            "-B", "-b", "-c", "-D", "-E", "-e", "-F", "-I", "-i", "-J", "-L", "-l", "-m", "-O",
            "-o", "-p", "-Q", "-R", "-S", "-W", "-w",
        ];
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.iter().any(|arg| arg == "-O") {
            return 0;
        }
        let mut rest = args.iter();
        let mut target = None;
        while let Some(arg) = rest.next() {
            if WITH_VALUE.contains(&arg.as_str()) {
                rest.next();
            } else if !arg.starts_with('-') {
                target = Some(arg.as_str());
                break;
            }
        }
        let host = target.map(|target| target.rsplit_once('@').map_or(target, |(_, host)| host));
        if host != Some(HOST) {
            eprintln!("focused fake ssh refused an unexpected destination");
            return 255;
        }
        if args.iter().any(|arg| arg == "-G") {
            let mut stdout = std::io::stdout().lock();
            for line in [
                "hostname focused-crew-usage.invalid",
                "port 22",
                "stricthostkeychecking yes",
                "forwardagent no",
                "forwardx11 no",
                "permitlocalcommand no",
                "clearallforwardings yes",
                "nohostauthenticationforlocalhost no",
                "tunnel no",
                "forkafterauthentication no",
                "gssapidelegatecredentials no",
                "proxycommand none",
                "proxyjump none",
                "controlmaster no",
                "controlpersist no",
                "controlpath none",
            ] {
                if writeln!(stdout, "{line}").is_err() {
                    return 1;
                }
            }
            return if stdout.flush().is_ok() { 0 } else { 1 };
        }
        bridge(root)
    }

    #[ctor::ctor]
    fn focused_fake_ssh_child_only() {
        if let Some(root) = std::env::var_os(FAKE_SSH_ROOT) {
            std::process::exit(fake_ssh(Path::new(&root)));
        }
    }

    fn shell_quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }

    pub(super) fn prepare(root: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        assert!(root.is_absolute(), "owned fixture root must be absolute");
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).expect("owned fixture bin directory");
        let executable = std::env::current_exe().expect("own test executable");
        let script = format!(
            "#!/bin/sh\n{FAKE_SSH_ROOT}={} exec {} \"$@\"\n",
            shell_quote(root.to_str().expect("UTF-8 fixture root")),
            shell_quote(executable.to_str().expect("UTF-8 test executable")),
        );
        let ssh = bin.join("ssh");
        std::fs::write(&ssh, script).expect("owned fake ssh script");
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o700))
            .expect("owned fake ssh executable");
        bin
    }

    pub(super) async fn workspace(root: &Path) -> Connection {
        assert!(root.is_absolute());
        assert_eq!(
            std::env::var("BIOROUTER_DISABLE_KEYRING").as_deref(),
            Ok("true")
        );
        let profile = PathBuf::from(
            std::env::var_os("BIOROUTER_DEV_PROFILE_ROOT").expect("owned development profile"),
        );
        assert!(profile.is_absolute() && profile.starts_with(root));
        let path = std::env::var_os("PATH").expect("isolated PATH");
        assert_eq!(std::env::split_paths(&path).next(), Some(root.join("bin")));
        assert!(root.join("bin/ssh").is_file());
        let invitation =
            biorouter_crew::invitation::message(&biorouter_crew::invitation::WorkspaceInvitation {
                workspace_id: WORKSPACE.into(),
                workspace_public_key: workspace_key::Key::new().public_hex(),
                socket_path: SOCKET.into(),
                owner_uid: HOST_UID,
                workspace_name: Some("focused-usage-lab".into()),
                host_username: Some("alice".into()),
                host_display_name: Some("Synthetic Alice".into()),
                mode: Some(biorouter_crew::Mode::Public),
                institution_id: None,
                ssh_host: Some(HOST.into()),
                ssh_port: None,
                proxy_jump: None,
                invitee_username: Some("alice".into()),
            })
            .expect("synthetic invitation");
        let manager = biorouter::crew::manager().expect("isolated Crew manager");
        let saved = match manager
            .connection_from_invitation(
                &invitation,
                false,
                biorouter::crew::authentication::InvitationOverrides {
                    mode: Some(biorouter::crew::ClusterMode::Public),
                    ..Default::default()
                },
            )
            .await
            .expect("synthetic invitation saves")
        {
            biorouter::crew::authentication::InvitationOutcome::Saved(connection) => *connection,
            biorouter::crew::authentication::InvitationOutcome::Preview(_) => {
                panic!("synthetic invitation unexpectedly previewed")
            }
        };
        manager
            .connect(&saved.id)
            .await
            .expect("signed fake workspace verifies");
        saved
    }

    pub(super) fn sent(root: &Path, method: &str) -> Vec<Value> {
        match std::fs::read_to_string(log_path(root)) {
            Ok(log) => log
                .lines()
                .map(|line| {
                    serde_json::from_str::<Value>(line).expect("complete fixture log frame")
                })
                .filter(|frame| frame["method"].as_str() == Some(method))
                .collect(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => panic!("read owned fixture log: {error}"),
        }
    }
}
