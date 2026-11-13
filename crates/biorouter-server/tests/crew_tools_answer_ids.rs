//! What a chat's Crew changes answer about where they went (chat parity WI-16): every team,
//! channel, membership and message change of the person tools names the saved connection it was
//! made through (`connection_id`), and the channel and message it went to where it has one. The
//! chat's row builds its "Open in Crew" from exactly these IDs, and Crew's change feed names them
//! (`ChangeIds::from_answer`).
//!
//! Pinned against the real host, the real `CrewManager` and the real approval registry and route,
//! one `PersonToolCall` at a time, with each change's one card allowed as the person's click
//! allows it. The task changes are pinned in `routes/crew_tools/tasks.rs`: starting one needs a
//! model, and stopping one needs a task this computer started.
//!
//! **Offline.** The daemon's `ssh` is this binary, a fake OpenSSH and bridge answering from a
//! per-host scenario file (copied from `crew_tools_actions.rs`). **One runtime** and **its own
//! binary**, for the same reasons as there.
#![cfg(unix)]

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root before `main`,
// so nothing here can open the developer's real `sessions.db` or Crew profile.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use biorouter::crew::tool_host::{self, PersonTool, PersonToolCall, CHANGE_ACTIONS};
use biorouter::pending_user_action::PendingUserActions;
use biorouter::privacy::affiliation::ModelAffiliation;
use biorouter::privacy::{CallCapability, ProviderTier};
use biorouter::providers::base::{Provider, ProviderMetadata, ProviderUsage};
use biorouter::session::session_manager::SessionType;
use biorouter_server::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serial_test::serial;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// Set only in the fake `ssh` process: the fixture directory it reads scenarios from.
const FAKE_SSH_ROOT: &str = "BIOROUTER_CREW_TOOLS_ANSWER_IDS_FAKE_SSH";
/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-tools-answer-ids-user-action-key";
/// The daemon secret the full router checks.
const SECRET: &str = "crew-tools-answer-ids-secret";
/// The host account's UID the invitations pin.
const HOST_UID: u32 = 1000;
/// The joiner's UID as the fake broker reports it in a challenge.
const JOINER_UID: u32 = 1001;
const SOCKET: &str = "/tmp/crew-1000-0123456789abcdef0123456789abcdef/broker.sock";
const ACCOUNT_CAPABILITIES: [&str; 4] = [
    "human_names_v1",
    "unique_names_v1",
    "join_by_name_v1",
    "account_identity_v1",
];

// ---------------------------------------------------------------------------------------------
// The workspace key, signed with `rustls`' own Ed25519 (the daemon verifies with
// `ed25519-dalek`, so a mismatch in either would fail every connect below).
// ---------------------------------------------------------------------------------------------

mod workspace_key {
    use rustls::crypto::ring::sign::any_eddsa_type;
    use rustls::pki_types::PrivatePkcs8KeyDer;
    use rustls::sign::SigningKey;
    use rustls::SignatureScheme;
    use std::sync::Arc;

    /// PKCS#8 v1 for an Ed25519 private key, before the 32-byte seed.
    const PKCS8_ED25519_PREFIX: [u8; 16] = [
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];

    pub struct Key(Arc<dyn SigningKey>);

    impl Key {
        pub fn from_seed(seed: [u8; 32]) -> Self {
            let mut der = PKCS8_ED25519_PREFIX.to_vec();
            der.extend_from_slice(&seed);
            Self(any_eddsa_type(&PrivatePkcs8KeyDer::from(der)).expect("an Ed25519 seed"))
        }

        /// The raw 32-byte public key, lowercase hex, as `hello` and an invitation carry it.
        pub fn public_hex(&self) -> String {
            let spki = self.0.public_key().expect("Ed25519 keys have a public key");
            let spki: &[u8] = spki.as_ref();
            hex::encode(&spki[spki.len() - 32..])
        }

        pub fn sign_hex(&self, message: &[u8]) -> String {
            let signer = self
                .0
                .choose_scheme(&[SignatureScheme::ED25519])
                .expect("an Ed25519 signer");
            hex::encode(signer.sign(message).expect("Ed25519 signs"))
        }
    }

    /// The workspace every scenario's broker signs `hello` with.
    pub fn workspace() -> Key {
        Key::from_seed([9; 32])
    }
}

// ---------------------------------------------------------------------------------------------
// Scenarios: what the fake broker answers, per SSH host.
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Scenario {
    workspace_id: String,
    node_id: String,
    name: Option<String>,
    mode: String,
    institution_id: Option<String>,
    policy_epoch: u64,
    capabilities: Vec<String>,
    /// Method → the whole envelope, `{"result": …}` or `{"error": …}`.
    answers: serde_json::Map<String, Value>,
}

impl Scenario {
    fn new(workspace_id: &str, capabilities: &[&str]) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            // One node per workspace, so no two tests' connections are merged into a cluster.
            node_id: hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
                workspace_id.as_bytes(),
            )),
            name: Some("lab".into()),
            mode: "private".into(),
            institution_id: Some("ucsf".into()),
            policy_epoch: 1,
            capabilities: capabilities.iter().map(|c| (*c).to_owned()).collect(),
            answers: serde_json::Map::new(),
        }
    }

    fn answer(mut self, method: &str, envelope: Value) -> Self {
        self.answers.insert(method.into(), envelope);
        self
    }
}

fn scenario_path(root: &Path, host: &str) -> PathBuf {
    root.join("scenarios").join(format!("{host}.json"))
}

/// Write `scenario` for `host`, atomically, so the fake never reads half a file.
fn stage(host: &str, scenario: &Scenario) {
    let path = scenario_path(&fixture_root(), host);
    let partial = path.with_extension("partial");
    fs::write(&partial, serde_json::to_vec(scenario).unwrap()).unwrap();
    fs::rename(partial, path).unwrap();
}

// ---------------------------------------------------------------------------------------------
// The fake `ssh`, run by this same binary before `main`.
// ---------------------------------------------------------------------------------------------

mod fake_ssh {
    use super::{scenario_path, workspace_key, Scenario, JOINER_UID};
    use serde_json::{json, Value};
    use std::io::{BufRead, Write};
    use std::path::Path;

    /// OpenSSH options that take a value, so the target is found the way `ssh` finds it.
    const WITH_VALUE: &[&str] = &[
        "-B", "-b", "-c", "-D", "-E", "-e", "-F", "-I", "-i", "-J", "-L", "-l", "-m", "-O", "-o",
        "-p", "-Q", "-R", "-S", "-W", "-w",
    ];

    /// The destination: the first argument that is neither an option nor an option's value.
    fn target(args: &[String]) -> Option<&str> {
        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            if WITH_VALUE.contains(&arg.as_str()) {
                rest.next();
            } else if !arg.starts_with('-') {
                return Some(arg);
            }
        }
        None
    }

    fn host_of(target: &str) -> &str {
        target.rsplit_once('@').map_or(target, |(_, host)| host)
    }

    fn load(root: &Path, host: &str) -> Option<Scenario> {
        serde_json::from_slice(&std::fs::read(scenario_path(root, host)).ok()?).ok()
    }

    pub fn run(root: &Path) -> i32 {
        let args: Vec<String> = std::env::args().skip(1).collect();
        // `-O exit` / `-O check` against a control master this fake never creates.
        if args.iter().any(|arg| arg == "-O") {
            return 0;
        }
        let Some(host) = target(&args).map(host_of).map(str::to_owned) else {
            eprintln!("fake ssh: no destination in {args:?}");
            return 255;
        };
        if args.first().map(String::as_str) == Some("-G") {
            let mut out = std::io::stdout().lock();
            // First wins in both of the daemon's readers, so the host's own lines go first.
            let _ = writeln!(out, "hostname {host}\nport 22");
            for line in [
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
                let _ = writeln!(out, "{line}");
            }
            let _ = out.flush();
            return 0;
        }
        bridge(root, &host)
    }

    /// Answer broker frames on stdin until the daemon closes the pipe.
    fn bridge(root: &Path, host: &str) -> i32 {
        let mut nonces = 0u64;
        let stdin = std::io::stdin();
        let mut out = std::io::stdout().lock();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { return 1 };
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                return 1;
            };
            let Some(scenario) = load(root, host) else {
                eprintln!("fake bridge: no scenario for {host}");
                return 1;
            };
            let mut envelope = match frame["method"].as_str().unwrap_or_default() {
                "hello" => json!({"result": hello(
                    &scenario,
                    frame["params"]["challenge_nonce"].as_str().unwrap_or_default(),
                )}),
                "auth.challenge" => {
                    nonces += 1;
                    json!({"result": {
                        "nonce": format!("fixture-nonce-{nonces}"),
                        "workspace_id": scenario.workspace_id,
                        "uid": JOINER_UID,
                        "expires_at": 4_000_000_000u64,
                    }})
                }
                method => scenario.answers.get(method).cloned().unwrap_or_else(|| {
                    json!({"error": {"code": "unsupported", "message": "unsupported: fixture has no answer"}})
                }),
            };
            envelope["id"] = frame["id"].clone();
            if writeln!(out, "{envelope}")
                .and_then(|()| out.flush())
                .is_err()
            {
                return 1;
            }
        }
        0
    }

    /// A `hello` signed v1 and v2 by the workspace key, as a current broker sends it.
    fn hello(scenario: &Scenario, nonce: &str) -> Value {
        let key = workspace_key::workspace();
        let public = key.public_hex();
        let host_uid = super::HOST_UID;
        let mode: biorouter_crew::Mode =
            serde_json::from_value(json!(scenario.mode)).expect("a fixture mode");
        let capabilities: Vec<&str> = scenario.capabilities.iter().map(String::as_str).collect();
        let v1 = key.sign_hex(&biorouter_crew::hello_v1_payload(
            &scenario.workspace_id,
            host_uid,
            nonce,
            &public,
            &scenario.node_id,
        ));
        let v2 = key.sign_hex(
            &biorouter_crew::HelloV2 {
                workspace_id: &scenario.workspace_id,
                host_uid,
                challenge_nonce: nonce,
                workspace_public_key: &public,
                node_id: &scenario.node_id,
                mode: &mode,
                institution_id: scenario.institution_id.as_deref(),
                policy_epoch: scenario.policy_epoch,
                name: scenario.name.as_deref(),
                capabilities: &capabilities,
            }
            .signing_payload(),
        );
        json!({
            "protocol": 1,
            "workspace_id": scenario.workspace_id,
            "host_uid": host_uid,
            "mode": mode,
            "institution_id": scenario.institution_id,
            "policy_epoch": scenario.policy_epoch,
            "name": scenario.name,
            "workspace_public_key": public,
            "node_id": scenario.node_id,
            "challenge_nonce": nonce,
            "signature": v1,
            "signature_v2": v2,
            "capabilities": capabilities,
        })
    }
}

// ---------------------------------------------------------------------------------------------
// The fixture directory, `PATH` and credential backend, set before `main`.
// ---------------------------------------------------------------------------------------------

/// This test process's fixture directory: the `ssh` script, scenarios, and the development
/// profile the file credential backend writes under.
fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "biorouter-crew-tools-answer-ids-{}",
        std::process::id()
    ))
}

#[ctor::ctor]
fn fake_ssh_or_fixture() {
    if let Some(root) = std::env::var_os(FAKE_SSH_ROOT) {
        let code = fake_ssh::run(Path::new(&root));
        std::process::exit(code);
    }
    let root = fixture_root();
    let prepared = (|| -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        for directory in ["bin", "profile", "scenarios"] {
            fs::create_dir_all(root.join(directory))?;
        }
        let executable = std::env::current_exe()?;
        let (root_text, executable_text) = (root.display(), executable.display());
        if format!("{root_text}{executable_text}").contains('\'') {
            return Err(std::io::Error::other("a fixture path contains a quote"));
        }
        let ssh = root.join("bin").join("ssh");
        fs::write(
            &ssh,
            format!("#!/bin/sh\n{FAKE_SSH_ROOT}='{root_text}' exec '{executable_text}' \"$@\"\n"),
        )?;
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700))
    })();
    if let Err(error) = prepared {
        eprintln!(
            "crew_tools_answer_ids: could not prepare the fixture at {}: {error}. Refusing to \
             run: the daemon would reach the real ssh and the OS keychain.",
            root.display()
        );
        std::process::abort();
    }
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{path}", root.join("bin").display()));
    // The development plaintext credential backend, under this fixture: never the keychain.
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    std::env::set_var("BIOROUTER_DEV_PROFILE_ROOT", root.join("profile"));
}

#[ctor::dtor]
fn remove_fixture() {
    // The fake `ssh` shares this binary; only the test process owns the fixture.
    if std::env::var_os(FAKE_SSH_ROOT).is_none() {
        let _ = fs::remove_dir_all(fixture_root());
    }
}

// ---------------------------------------------------------------------------------------------
// The daemon: one runtime, one AppState, the host installed once
// ---------------------------------------------------------------------------------------------

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("a runtime")
});

static STATE: tokio::sync::OnceCell<Arc<AppState>> = tokio::sync::OnceCell::const_new();

/// The daemon's state, made once, with the desktop's user-action digest and the person-tool
/// host installed, as `biorouterd agent` installs them.
async fn state() -> Arc<AppState> {
    STATE
        .get_or_init(|| async {
            let digest: [u8; 32] =
                <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
            biorouter_server::auth::install_user_action_digest(Some(digest));
            let state = AppState::new().await.expect("app state");
            biorouter_server::routes::crew_tools::install(state.clone());
            assert!(tool_host::get().is_some(), "the host did not install");
            state
        })
        .await
        .clone()
}

/// Run `test` on the shared runtime.
fn run<F: std::future::Future<Output = ()>>(test: F) {
    RUNTIME.block_on(test);
}

async fn daemon() -> Router {
    biorouter_server::routes::configure(state().await, SECRET.into())
}

/// The person's click on a card, through the route the desktop calls, with its proof.
async fn answer_card(id: &str, session: &str, answer: &str) {
    let request = Request::builder()
        .method("POST")
        .uri("/action-required/tool-confirmation")
        .header("X-Secret-Key", SECRET)
        .header("X-User-Action", USER_KEY)
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"id": id, "principalType": "Tool", "action": answer, "sessionId": session})
                .to_string(),
        ))
        .unwrap();
    let response = daemon().await.oneshot(request).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the card was not answered"
    );
}

fn crew() -> Arc<biorouter::crew::CrewManager> {
    biorouter::crew::manager().expect("the sandboxed Crew manager")
}

/// A person's own chat, as the agent loop would hold it.
async fn chat(name: &str) -> String {
    state()
        .await
        .session_manager()
        .create_session(
            std::env::temp_dir().join("crew-tools-answer-ids"),
            name.to_owned(),
            SessionType::User,
        )
        .await
        .expect("a chat")
        .id
}

// ---------------------------------------------------------------------------------------------
// The chat's model, sampled as dispatch samples it
// ---------------------------------------------------------------------------------------------

struct Model;

#[async_trait::async_trait]
impl Provider for Model {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "crew-tools-answer-ids-fixture"
    }

    fn get_model_config(&self) -> biorouter::model::ModelConfig {
        biorouter::model::ModelConfig::new_or_fail("synthetic")
    }

    fn tier(&self) -> ProviderTier {
        ProviderTier::Private
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        Some(ModelAffiliation::Local)
    }

    async fn complete_with_model(
        &self,
        _: &biorouter::model::ModelConfig,
        _: &str,
        _: &[biorouter::conversation::message::Message],
        _: &[rmcp::model::Tool],
    ) -> Result<
        (biorouter::conversation::message::Message, ProviderUsage),
        biorouter::providers::errors::ProviderError,
    > {
        unreachable!("the person tools never ask a model")
    }
}

/// A model on this machine: Private, and compatible with every institution.
async fn private_local() -> CallCapability {
    let provider: biorouter::agents::types::SharedProvider =
        Arc::new(tokio::sync::Mutex::new(Some(Arc::new(Model))));
    CallCapability::sample(&provider).await
}

// ---------------------------------------------------------------------------------------------
// One change, its card allowed
// ---------------------------------------------------------------------------------------------

/// `crew__change action` from `session`, its one card allowed by the person's click. Answers
/// what the model reads: the answer, or the refusal's words.
async fn change(
    session: &str,
    capability: CallCapability,
    action: &str,
    params: Value,
) -> Result<Value, String> {
    let host = tool_host::get().expect("the host is installed");
    let call = PersonToolCall {
        tool: PersonTool::Change,
        action: action.to_owned(),
        params,
        session_id: session.to_owned(),
        tool_call_id: Some(format!("call-{}", uuid::Uuid::new_v4())),
        capability,
        cancel: CancellationToken::new(),
        approvals: Arc::clone(PendingUserActions::global()),
    };
    let task =
        tokio::spawn(async move { host.call(call).await.map_err(|error| error.to_string()) });
    let mut cards = 0usize;
    let mut answered = std::collections::HashSet::new();
    loop {
        for message in PendingUserActions::global().pending_cards_for_session(session) {
            let card = serde_json::to_value(&message).unwrap();
            let id = card["content"][0]["data"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if id.is_empty() || !answered.insert(id.clone()) {
                continue;
            }
            cards += 1;
            answer_card(&id, session, "allow_once").await;
        }
        if task.is_finished() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(cards, 1, "{action}: one card, allowed");
    task.await.expect("the call ran")
}

// ---------------------------------------------------------------------------------------------
// The workspace
// ---------------------------------------------------------------------------------------------

fn invitation(workspace_id: &str, ssh_host: &str) -> String {
    biorouter_crew::invitation::message(&biorouter_crew::invitation::WorkspaceInvitation {
        workspace_id: workspace_id.into(),
        workspace_public_key: workspace_key::workspace().public_hex(),
        socket_path: SOCKET.into(),
        owner_uid: HOST_UID,
        workspace_name: Some("lab".into()),
        host_username: Some("alice".into()),
        host_display_name: Some("Alice Chen".into()),
        mode: Some(biorouter_crew::Mode::Private),
        institution_id: Some("ucsf".into()),
        ssh_host: Some(ssh_host.into()),
        ssh_port: None,
        proxy_jump: None,
        invitee_username: Some("alice".into()),
    })
    .unwrap()
}

const GENERAL: &str = "c-general";
const METHODS: &str = "c-methods";
const FIELD_NOTES: &str = "c-field-notes";

/// What Alice sees in an open workspace: herself (the host) and Bob in one team, Analysis Lab,
/// with #general and #methods; a team and a channel she may join; and two invitations of hers,
/// to another team and to a channel.
fn snapshot() -> Value {
    json!({
        "workspace": {
            "id": "w", "name": "lab", "mode": "private", "institution_id": "ucsf",
            "host_principal_id": "p-alice", "policy_epoch": 1, "governance": "open",
        },
        "actor": {"id": "p-alice", "username": "alice", "display_name": "Alice Chen", "devices": []},
        "principals": [
            {"id": "p-alice", "username": "alice", "display_name": "Alice Chen"},
            {"id": "p-bob", "username": "bob", "display_name": "Bob Lee"},
        ],
        "former_principals": [],
        "teams": [{
            "id": "t-lab", "name": "analysis-lab", "display_name": "Analysis Lab",
            "general_channel_id": GENERAL, "members": ["p-alice", "p-bob"],
            "name_conflict": false,
        }],
        "channels": [
            {"id": GENERAL, "team_id": "t-lab", "name": "general",
             "classification": "public_safe", "name_conflict": false},
            {"id": METHODS, "team_id": "t-lab", "name": "methods",
             "classification": "public_safe", "name_conflict": false},
        ],
        "protected_channel_ids": [],
        "joinable_teams": [
            {"id": "t-field", "name": "field-team", "display_name": "Field Team", "handle": "field-team"},
        ],
        "joinable_channels": [
            {"id": FIELD_NOTES, "team_id": "t-lab", "name": "field-notes",
             "classification": "public_safe"},
        ],
        "invitations": [
            {"id": "i-field", "kind": "team", "target_id": "t-field", "principal_id": "p-alice",
             "inviter_id": "p-bob", "expires_at": 4_000_000_000u64, "target_name": "field-team"},
            {"id": "i-notes", "kind": "channel", "target_id": FIELD_NOTES,
             "principal_id": "p-alice", "inviter_id": "p-bob", "expires_at": 4_000_000_000u64,
             "target_name": "field-notes", "team_name": "analysis-lab"},
        ],
        "unread": {},
        "totals": {"teams": 1, "channels": 2},
    })
}

/// What the fake workspace answers: the snapshot, and every change the table sends. A post's
/// answer is the message as the broker stores it, its channel among its fields.
fn scenario(workspace_id: &str) -> Scenario {
    Scenario::new(workspace_id, &ACCOUNT_CAPABILITIES)
        .answer("workspace.snapshot", json!({"result": snapshot()}))
        .answer(
            "team.create",
            json!({"result": {"id": "t-new", "name": "analysis-2", "general_channel_id": "c-new-general"}}),
        )
        .answer(
            "team.rename",
            json!({"result": {"id": "t-lab", "name": "analysis-two", "display_name": "Analysis Two"}}),
        )
        .answer("team.delete", json!({"result": {"deleted": true, "channels": 2}}))
        .answer("team.join", json!({"result": {"joined": true}}))
        .answer(
            "team.add_member",
            json!({"result": {"added_channels": [METHODS]}}),
        )
        .answer(
            "channel.create",
            json!({"result": {"id": "c-new", "name": "methods-2", "classification": "restricted"}}),
        )
        .answer(
            "channel.rename",
            json!({"result": {"id": METHODS, "name": "methods-3"}}),
        )
        .answer("channel.archive", json!({"result": {"id": METHODS}}))
        .answer("channel.delete", json!({"result": {"deleted": true}}))
        .answer("channel.join", json!({"result": {"joined": true}}))
        .answer("channel.read", json!({"result": {"sequence": "m-2"}}))
        .answer(
            "channel.add_member",
            json!({"result": {"already_member": false}}),
        )
        .answer(
            "channel.transfer",
            json!({"result": {"pending_owner": "p-bob"}}),
        )
        .answer("transfer.accept", json!({"result": {"owner_id": "p-alice"}}))
        .answer("membership.revoke", json!({"result": {"revoked": true}}))
        .answer("invitation.create", json!({"result": {"id": "i-new"}}))
        .answer("invitation.accept", json!({"result": {"accepted": true}}))
        .answer(
            "message.post",
            json!({"result": {"id": "m-new", "sequence": "m-new", "restricted": false,
                "channel_id": GENERAL}}),
        )
        .answer(
            "reference.create",
            json!({"result": {"id": "r-new", "restricted": true}}),
        )
        .answer(
            "messages.history",
            json!({"result": {"messages": [], "cursor": "m-2"}}),
        )
}

/// A workspace on `host`, saved on this computer from its invitation and connected.
async fn workspace(host: &str, scenario: Scenario) -> biorouter::crew::Connection {
    stage(host, &scenario);
    let saved = match crew()
        .connection_from_invitation(
            &invitation(&scenario.workspace_id, host),
            false,
            biorouter::crew::authentication::InvitationOverrides::default(),
        )
        .await
        .expect("the invitation saves")
    {
        biorouter::crew::authentication::InvitationOutcome::Saved(connection) => *connection,
        biorouter::crew::authentication::InvitationOutcome::Preview(_) => panic!("a preview"),
    };
    crew()
        .connect(&saved.id)
        .await
        .expect("the fake workspace verifies");
    saved
}

/// `params` for an action on `connection`, named by its ID.
fn on(connection: &biorouter::crew::Connection, mut params: Value) -> Value {
    params["connection"] = json!(connection.id);
    params
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

/// One row of the table: a change, what the call typed, and the channel and message its answer
/// must name besides the connection (`None`: it names no channel, or no message).
struct Row {
    action: &'static str,
    params: Value,
    channel: Option<&'static str>,
    message: Option<&'static str>,
}

fn row(action: &'static str, params: Value, channel: Option<&'static str>) -> Row {
    Row {
        action,
        params,
        channel,
        message: None,
    }
}

/// The changes, in an order the fixture's fixed answers allow (the deletions last).
fn table() -> Vec<Row> {
    vec![
        row("teams_create", json!({"name": "analysis-2"}), None),
        row(
            "teams_rename",
            json!({"team": "Analysis Lab", "name": "Analysis Two"}),
            None,
        ),
        row("teams_join", json!({"team": "field-team"}), None),
        row(
            "channels_create",
            json!({"name": "methods-2", "team": "Analysis Lab"}),
            Some("c-new"),
        ),
        row(
            "channels_rename",
            json!({"channel": "#methods", "name": "methods-3"}),
            Some(METHODS),
        ),
        row(
            "channels_archive",
            json!({"channel": "#methods"}),
            Some(METHODS),
        ),
        row(
            "channels_join",
            json!({"channel": "field-notes"}),
            Some(FIELD_NOTES),
        ),
        row(
            "channels_read",
            json!({"channel": "#general"}),
            Some(GENERAL),
        ),
        row(
            "members_add",
            json!({"member": "@bob", "team": "Analysis Lab", "channels": ["methods"]}),
            None,
        ),
        row(
            "members_add",
            json!({"member": "@bob", "channels": ["#methods"]}),
            None,
        ),
        row(
            "invites_send",
            json!({"member": "@bob", "channel": "#methods"}),
            Some(METHODS),
        ),
        row(
            "invites_send",
            json!({"member": "@bob", "team": "Analysis Lab"}),
            None,
        ),
        row("invites_accept", json!({"invitation": "field-team"}), None),
        row(
            "invites_accept",
            json!({"invitation": "analysis-lab/field-notes"}),
            Some(FIELD_NOTES),
        ),
        row(
            "ownership_offer",
            json!({"channel": "#methods", "successor": "@bob"}),
            Some(METHODS),
        ),
        row(
            "ownership_accept",
            json!({"channel": "#methods"}),
            Some(METHODS),
        ),
        row(
            "remove_member",
            json!({"channel": "#methods", "member": "@bob"}),
            Some(METHODS),
        ),
        Row {
            action: "send",
            params: json!({"channel": "#general", "text": "status update"}),
            channel: Some(GENERAL),
            message: Some("m-new"),
        },
        Row {
            action: "files_reference",
            params: json!({"channel": "#general", "path": "/data/lab/results.csv"}),
            channel: Some(GENERAL),
            message: Some("m-new"),
        },
        row(
            "channels_delete",
            json!({"channel": "#methods"}),
            Some(METHODS),
        ),
        row("teams_delete", json!({"team": "Analysis Lab"}), None),
    ]
}

/// Every team, channel, membership and message change answers the connection it was made
/// through, and the channel and message it went to where it has one, so the chat's row can open
/// Crew there (WI-16).
#[test]
#[serial]
fn every_change_answer_names_where_it_went() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000e0001";
        const HOST: &str = "answer-ids.example.test";
        let connection = workspace(HOST, scenario(WORKSPACE)).await;
        let session = chat("answer ids").await;
        let model = private_local().await;
        let rows = table();
        for row in &rows {
            assert!(CHANGE_ACTIONS.contains(&row.action), "{}", row.action);
            let answer = change(
                &session,
                model,
                row.action,
                on(&connection, row.params.clone()),
            )
            .await
            .unwrap_or_else(|error| panic!("{}: refused: {error}", row.action));
            assert_eq!(
                answer["connection_id"], connection.id,
                "{}: {answer}",
                row.action
            );
            if let Some(channel) = row.channel {
                assert_eq!(answer["channel_id"], channel, "{}: {answer}", row.action);
            }
            if let Some(message) = row.message {
                assert_eq!(answer["message_id"], message, "{}: {answer}", row.action);
            }
            // A new team shows in its #general.
            if row.action == "teams_create" {
                assert_eq!(answer["general_channel_id"], "c-new-general", "{answer}");
            }
        }
        let _ = crew().remove(&connection.id).await;
    });
}
