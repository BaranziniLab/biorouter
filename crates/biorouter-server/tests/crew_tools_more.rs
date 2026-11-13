//! The chat's Crew actions the parity review found missing (WI-4): `crew__read setup_list`,
//! `files_pending` and `connections_invitation`, and `crew__change context_draft`,
//! `profile_devices_acknowledge`, `files_pause`, `files_resume` and `files_forget`, against the
//! real host, the real `CrewManager`, the real transfer service and the real approval registry
//! and routes.
//!
//! What is pinned here:
//!
//! - each action reaches the same function as its route (`GET /crew/setup`,
//!   `GET /crew/transfers?connection_id=`, the invitation route, the context draft route and
//!   the transfer routes' effects);
//! - every change parks exactly one approval whose summary names the resolved targets, and
//!   changes nothing when denied;
//! - `files_pending` and `context_draft` obey the public-tier refusal and withholding, and
//!   raise the chat Private before returning protected content, and so does resuming a
//!   download, which brings a workspace file onto this computer;
//! - acknowledging new computers never happens on a read;
//! - forgetting an unfinished download names its partial file's path on the card;
//! - `setup_list` finds a job waiting for a sign-in that a new chat did not start, and `auth`
//!   with that job's ID shows its Sign in card;
//! - `connections_invitation` is refused to a member who is not the host.
//!
//! **Offline.** The daemon's `ssh` is this binary, as in `crew_tools_host.rs`, whose fake
//! broker is copied below, with three additions: a host named `needs-sign-in…` asks for a
//! password, the server probe answers fixed facts, and a scenario can hold an answer back for a
//! while (a transfer under way). **One runtime and its own binary**, for the reasons
//! `crew_tools_host.rs` gives.
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
use biorouter::crew::tool_host::{self, PersonTool, PersonToolCall};
use biorouter::pending_user_action::PendingUserActions;
use biorouter::privacy::affiliation::ModelAffiliation;
use biorouter::privacy::{CallCapability, ProviderTier, SessionClassification};
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
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// Set only in the fake `ssh` process: the fixture directory it reads scenarios from.
const FAKE_SSH_ROOT: &str = "BIOROUTER_CREW_TOOLS_MORE_FAKE_SSH";
/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-tools-more-user-action-key";
/// The daemon secret the full router checks.
const SECRET: &str = "crew-tools-more-secret";
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
/// What the fake server's probe says about itself.
const PROBE_HOST: &str = "probe-login-1.example.test";
/// A host whose name starts with this asks for a password on every connection.
const NEEDS_SIGN_IN: &str = "needs-sign-in";
/// What `files_resume` says, whatever path it was given, for an upload the chat may not name.
const UPLOAD_NOT_NAMEABLE: &str = "This chat can't resume that upload, because it may not see which file it was sharing. Resume it in Crew, under Files.";

// ---------------------------------------------------------------------------------------------
// The workspace key, signed with `rustls`' own Ed25519.
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
// Scenarios: what the fake broker answers, per SSH host, re-read on every frame.
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
    /// Method → how long the fake waits before it answers, in milliseconds.
    #[serde(default)]
    delays: serde_json::Map<String, Value>,
}

impl Scenario {
    fn new(workspace_id: &str, capabilities: &[&str]) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            node_id: hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
                workspace_id.as_bytes(),
            )),
            name: Some("lab".into()),
            mode: "private".into(),
            institution_id: Some("ucsf".into()),
            policy_epoch: 1,
            capabilities: capabilities.iter().map(|c| (*c).to_owned()).collect(),
            answers: serde_json::Map::new(),
            delays: serde_json::Map::new(),
        }
    }

    fn answer(mut self, method: &str, envelope: Value) -> Self {
        self.answers.insert(method.into(), envelope);
        self
    }

    fn delay(mut self, method: &str, milliseconds: u64) -> Self {
        self.delays.insert(method.into(), json!(milliseconds));
        self
    }
}

fn scenario_path(root: &Path, host: &str) -> PathBuf {
    root.join("scenarios").join(format!("{host}.json"))
}

/// Write `scenario` for `host`, atomically, so the fake never reads half a file. Its snapshot
/// names the scenario's own workspace, as a real broker's does: an invitation is built only
/// from a snapshot of the connection's workspace.
fn stage(host: &str, scenario: &Scenario) {
    let mut scenario = scenario.clone();
    if let Some(workspace) = scenario
        .answers
        .get_mut("workspace.snapshot")
        .and_then(|snapshot| snapshot.pointer_mut("/result/workspace"))
    {
        workspace["id"] = json!(scenario.workspace_id);
    }
    let path = scenario_path(&fixture_root(), host);
    let partial = path.with_extension("partial");
    fs::write(&partial, serde_json::to_vec(&scenario).unwrap()).unwrap();
    fs::rename(partial, path).unwrap();
}

/// The scenario staged for `host`.
fn staged(host: &str) -> Scenario {
    serde_json::from_slice(&fs::read(scenario_path(&fixture_root(), host)).unwrap()).unwrap()
}

// ---------------------------------------------------------------------------------------------
// The fake `ssh`, run by this same binary before `main`.
// ---------------------------------------------------------------------------------------------

mod fake_ssh {
    use super::{scenario_path, workspace_key, Scenario, JOINER_UID, NEEDS_SIGN_IN, PROBE_HOST};
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
        let host = target(&args).map(host_of).map(str::to_owned);
        let asks_password = host
            .as_deref()
            .is_some_and(|host| host.starts_with(NEEDS_SIGN_IN));
        // `-O exit` / `-O check` against a control master this fake never creates.
        if args.iter().any(|arg| arg == "-O") {
            if asks_password {
                eprintln!("Control socket connect(/tmp/fake): No such file or directory");
                return 255;
            }
            return 0;
        }
        let Some(host) = host else {
            eprintln!("fake ssh: no destination in {args:?}");
            return 255;
        };
        if args.first().map(String::as_str) == Some("-G") {
            let mut out = std::io::stdout().lock();
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
        if asks_password {
            eprintln!("you@{host}: Permission denied (publickey,password,keyboard-interactive).");
            return 255;
        }
        if args
            .iter()
            .any(|arg| arg.contains("probe --output-format json"))
        {
            println!("{}", json!({"host": PROBE_HOST, "os": "Linux"}));
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
            let method = frame["method"].as_str().unwrap_or_default();
            if let Some(wait) = scenario.delays.get(method).and_then(Value::as_u64) {
                std::thread::sleep(std::time::Duration::from_millis(wait));
            }
            let mut envelope = match method {
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

fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!("biorouter-crew-tools-more-{}", std::process::id()))
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
        for directory in ["bin", "profile", "scenarios", "home/Downloads"] {
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
            "crew_tools_more: could not prepare the fixture at {}: {error}. Refusing to run: \
             the daemon would reach the real ssh and the OS keychain.",
            root.display()
        );
        std::process::abort();
    }
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{path}", root.join("bin").display()));
    // The development plaintext credential backend, under this fixture: never the keychain.
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    std::env::set_var("BIOROUTER_DEV_PROFILE_ROOT", root.join("profile"));
    // A home of the fixture's own: forgetting a download looks for its folder in the person's
    // home and Downloads, which must never be the developer's.
    std::env::set_var("HOME", root.join("home"));
}

#[ctor::dtor]
fn remove_fixture() {
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

fn run<F: std::future::Future<Output = ()>>(test: F) {
    RUNTIME.block_on(test);
}

async fn daemon() -> Router {
    biorouter_server::routes::configure(state().await, SECRET.into())
}

async fn http(method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Secret-Key", SECRET)
        .header("X-User-Action", USER_KEY);
    let request = match body {
        Some(body) => {
            request = request.header("content-type", "application/json");
            request.body(Body::from(body.to_string()))
        }
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = daemon().await.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned())),
    )
}

fn crew() -> Arc<biorouter::crew::CrewManager> {
    biorouter::crew::manager().expect("the sandboxed Crew manager")
}

/// A person's own chat, as the agent loop would hold it, working in `folder`.
async fn chat_in(name: &str, folder: &Path) -> String {
    state()
        .await
        .session_manager()
        .create_session(folder.to_owned(), name.to_owned(), SessionType::User)
        .await
        .expect("a chat")
        .id
}

async fn chat(name: &str) -> String {
    chat_in(name, &std::env::temp_dir().join("crew-tools-more")).await
}

async fn classification(session: &str) -> SessionClassification {
    state()
        .await
        .session_manager()
        .get_session(session, false)
        .await
        .expect("the chat")
        .privacy_tier
}

// ---------------------------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------------------------

struct Model {
    tier: ProviderTier,
    affiliation: Option<ModelAffiliation>,
}

#[async_trait::async_trait]
impl Provider for Model {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::empty()
    }

    fn get_name(&self) -> &str {
        "crew-tools-more-fixture"
    }

    fn get_model_config(&self) -> biorouter::model::ModelConfig {
        biorouter::model::ModelConfig::new_or_fail("synthetic")
    }

    fn tier(&self) -> ProviderTier {
        self.tier
    }

    fn affiliation(&self) -> Option<ModelAffiliation> {
        self.affiliation
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
        Arc::new(tokio::sync::Mutex::new(Some(Arc::new(Model {
            tier: ProviderTier::Private,
            affiliation: Some(ModelAffiliation::Local),
        }))));
    CallCapability::sample(&provider).await
}

fn public() -> CallCapability {
    CallCapability::public_enforced()
}

// ---------------------------------------------------------------------------------------------
// Calls, and the person at the card
// ---------------------------------------------------------------------------------------------

/// One card the call parked, as the person saw it.
#[derive(Debug)]
struct SeenCard {
    /// `toolConfirmation` for an approval or a read consent, `crewStep` for a Crew step.
    kind: String,
    tool: String,
    prompt: String,
    arguments: Value,
    /// A Crew step card's step, as the daemon wrote it.
    step: Value,
}

impl SeenCard {
    /// The read consent card, as against a change's approval.
    fn is_consent(&self) -> bool {
        self.arguments["action"] == "read as you"
    }
}

/// What a call answered, as the model reads it, and the cards it parked.
struct Called {
    answer: Result<Value, String>,
    cards: Vec<SeenCard>,
}

impl Called {
    fn ok(&self) -> &Value {
        match &self.answer {
            Ok(answer) => answer,
            Err(error) => panic!("refused: {error}"),
        }
    }

    fn refusal(&self) -> &str {
        match &self.answer {
            Ok(answer) => panic!("answered: {answer}"),
            Err(error) => error,
        }
    }

    /// The one approval the change parked: every card but the read consent and a Crew step.
    fn approval(&self) -> &SeenCard {
        let approvals: Vec<&SeenCard> = self
            .cards
            .iter()
            .filter(|card| card.kind == "toolConfirmation" && !card.is_consent())
            .collect();
        assert_eq!(approvals.len(), 1, "{:?}", self.cards);
        approvals[0]
    }
}

/// Call `tool`'s `action` from `session` on `capability`, and answer each card it parks with the
/// next of `answers` (`allow_once` or `deny`) through `POST /action-required/tool-confirmation`
/// with the desktop's proof, as the person's click does. A card beyond `answers` is denied. A
/// Crew step card, which nothing can answer as done, is closed through
/// `POST /action-required/crew-step`, as the person's Close does.
async fn call(
    session: &str,
    capability: CallCapability,
    tool: PersonTool,
    action: &str,
    params: Value,
    answers: &[&str],
) -> Called {
    let host = tool_host::get().expect("the host is installed");
    let call = PersonToolCall {
        tool,
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
    let mut cards = Vec::new();
    let mut answered = std::collections::HashSet::new();
    loop {
        for message in PendingUserActions::global().pending_cards_for_session(session) {
            let card = serde_json::to_value(&message).unwrap();
            let data = &card["content"][0]["data"];
            let id = data["id"].as_str().unwrap_or_default().to_owned();
            if id.is_empty() || !answered.insert(id.clone()) {
                continue;
            }
            let kind = data["actionType"].as_str().unwrap_or_default().to_owned();
            cards.push(SeenCard {
                kind: kind.clone(),
                tool: data["toolName"].as_str().unwrap_or_default().to_owned(),
                prompt: data["prompt"].as_str().unwrap_or_default().to_owned(),
                arguments: data["arguments"].clone(),
                step: data["step"].clone(),
            });
            if kind == "crewStep" {
                let (status, body) = http(
                    "POST",
                    "/action-required/crew-step",
                    Some(json!({"id": id, "session_id": session, "outcome": "dismiss"})),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{body}");
                continue;
            }
            let answer = answers.get(cards.len() - 1).copied().unwrap_or("deny");
            let (status, body) = http(
                "POST",
                "/action-required/tool-confirmation",
                Some(json!({"id": id, "principalType": "Tool", "action": answer, "sessionId": session})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        if task.is_finished() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Called {
        answer: task.await.expect("the call ran"),
        cards,
    }
}

// ---------------------------------------------------------------------------------------------
// Workspaces
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

/// What the person sees of the workspace: Alice (the person, the host unless `host` names
/// another) and Bob, and two channels, `c-patients` protected when `protected` says so.
/// `devices` are the person's own computers. The workspace's ID is the scenario's, set when it
/// is staged ([`stage`]).
fn snapshot_with(mode: &str, protected: bool, host: &str, devices: Value) -> Value {
    json!({"result": {
        "workspace": {
            "id": "set-when-staged", "name": "lab", "mode": mode, "institution_id": "ucsf",
            "host_principal_id": host, "policy_epoch": 1,
        },
        "actor": {
            "id": "p-alice", "username": "alice", "display_name": "Alice Chen", "uid": HOST_UID,
            "devices": devices,
        },
        "principals": [
            {"id": "p-alice", "username": "alice", "display_name": "Alice Chen", "uid": HOST_UID, "active": true},
            {"id": "p-bob", "username": "bob", "display_name": "Bob Lee", "uid": JOINER_UID, "active": true},
        ],
        "teams": [],
        "channels": [{"id": "c-general", "name": "general"}, {"id": "c-patients", "name": "patients"}],
        "protected_channel_ids": if protected { json!(["c-patients"]) } else { json!([]) },
        "invitations": [],
    }})
}

/// This computer's own device on every fixture snapshot.
fn own_device() -> Value {
    json!({"fingerprint": "AAAA AAAA AAAA AAAA", "added_via": "invitation", "added_at": 1, "this_device": true})
}

fn snapshot(mode: &str, protected: bool) -> Value {
    snapshot_with(mode, protected, "p-alice", json!([own_device()]))
}

/// A workspace on `host`: staged with `scenario`'s answers, saved on this computer from its
/// invitation (Private and `ucsf` unless `public`), and connected. Answers the connection.
async fn workspace(
    workspace_id: &str,
    host: &str,
    public: bool,
    scenario: impl FnOnce(Scenario) -> Scenario,
) -> biorouter::crew::Connection {
    let mut staged = scenario(Scenario::new(workspace_id, &ACCOUNT_CAPABILITIES));
    if public {
        staged.mode = "public".into();
    }
    stage(host, &staged);
    let overrides = biorouter::crew::authentication::InvitationOverrides {
        mode: public.then_some(biorouter::crew::ClusterMode::Public),
        ..Default::default()
    };
    let saved = match crew()
        .connection_from_invitation(&invitation(workspace_id, host), false, overrides)
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

async fn remove(connection: &biorouter::crew::Connection) {
    let _ = crew().remove(&connection.id).await;
}

/// `lab on <server>`, as a change card names the connection's workspace.
async fn here(connection: &biorouter::crew::Connection) -> String {
    let server = biorouter::crew::server_label(&connection.ssh_target, connection.port).await;
    format!("{} on {server}", connection.name)
}

// ---------------------------------------------------------------------------------------------
// Transfers, started through their routes as the Crew view starts them
// ---------------------------------------------------------------------------------------------

/// Whether the transfer store can be opened in this run. It opens every folder on its way
/// without following links, and macOS reaches its temporary folder through one, so a test root
/// there can't hold it (`crew_tools_actions.rs` meets the same). A transfer test then says so
/// and stops; it runs in full on Linux, as CI runs it, or on macOS with `TMPDIR` set to the
/// temporary folder's real path.
fn transfer_store_reachable(test: &str) -> bool {
    let store = biorouter::config::paths::Paths::state_dir();
    fs::create_dir_all(&store).unwrap();
    if fs::canonicalize(&store).ok().as_deref() == Some(store.as_path()) {
        return true;
    }
    eprintln!(
        "{test} not run: {} is reached through a link. Set TMPDIR to its real path to run it.",
        store.display()
    );
    false
}

/// A folder only this account may change, as Crew wants a download's folder, with no symbolic
/// link on its way.
fn private_folder() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    let base = fs::canonicalize(std::env::temp_dir()).unwrap();
    let root = tempfile::tempdir_in(base).unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    root
}

/// `bytes` as the workspace's attachment of them would describe itself.
fn attachment(id: &str, channel: &str, bytes: &[u8], offset: u64, complete: bool) -> Value {
    json!({"result": {
        "id": id,
        "channel_id": channel,
        "size": bytes.len(),
        "sha256": hex::encode(<sha2::Sha256 as sha2::Digest>::digest(bytes)),
        "offset": offset,
        "complete": complete,
        "media_type": "image/png",
        "restricted": false,
    }})
}

/// Start a transfer through `POST /crew/files` and `POST /crew/transfers`, as the Crew view
/// does. Answers the receipt.
async fn start_transfer(
    connection: &biorouter::crew::Connection,
    channel: &str,
    direction: &str,
    path: &Path,
    blob_id: Option<&str>,
) -> Value {
    let (status, capability) = http(
        "POST",
        "/crew/files",
        Some(json!({
            "connection_id": connection.id, "channel_id": channel, "direction": direction,
            "path": path, "blob_id": blob_id, "transfer_id": null,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{capability}");
    let (status, receipt) = http(
        "POST",
        "/crew/transfers",
        Some(json!({
            "request_id": uuid::Uuid::new_v4().to_string(), "connection_id": connection.id,
            "channel_id": channel, "direction": direction,
            "file_capability": capability["capability_id"], "blob_id": blob_id,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    receipt
}

/// The receipt `GET /crew/transfers/{id}` reads, or the refusal.
async fn transfer(id: &str) -> (StatusCode, Value) {
    http("GET", &format!("/crew/transfers/{id}"), None).await
}

/// Wait for the transfer `id` to reach a state `done` names, and answer its receipt.
async fn reached(id: &str, done: impl Fn(&str) -> bool) -> Value {
    for _ in 0..1500 {
        let (status, receipt) = transfer(id).await;
        assert_eq!(status, StatusCode::OK, "{receipt}");
        if done(receipt["state"].as_str().unwrap_or_default()) {
            return receipt;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the transfer never got there: {:?}", transfer(id).await);
}

/// Whether a receipt state is one the daemon is no longer moving.
fn stopped(state: &str) -> bool {
    !matches!(
        state,
        "starting" | "uploading" | "downloading" | "publishing" | "pause_requested"
    )
}

/// A transfer's ID as the routes answer it.
fn id_of(receipt: &Value) -> String {
    receipt["id"].as_str().expect("a transfer ID").to_owned()
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

/// `setup_list` lists the jobs `GET /crew/setup` lists, each by its ID, state, stage and
/// server, and finds a job waiting for the person to sign in that this chat did not start
/// (Crew started it), with the sign-in's code and step; `auth` with the listed ID then shows
/// that job's Sign in card.
#[test]
#[serial]
fn setup_list_finds_a_job_a_new_chat_did_not_start() {
    run(async {
        let login = format!("you@{NEEDS_SIGN_IN}.example.test");
        let (status, started) = http(
            "POST",
            "/crew/setup",
            Some(json!({"target": {"kind": "server", "ssh_target": login}, "intent": "check"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{started}");
        let job_id = started["job_id"].as_str().unwrap().to_owned();
        let mut waiting = false;
        for _ in 0..1500 {
            let (_, job) = http("GET", &format!("/crew/setup/{job_id}"), None).await;
            if job["state"] == "needs_sign_in" {
                waiting = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(waiting, "the job never asked for a sign-in");

        let session = chat("a new chat").await;
        let listed = call(
            &session,
            public(),
            PersonTool::Read,
            "setup_list",
            json!({}),
            &[],
        )
        .await;
        assert!(listed.cards.is_empty());
        let jobs = listed.ok()["jobs"].as_array().unwrap().clone();
        let (status, routed) = http("GET", "/crew/setup", None).await;
        assert_eq!(status, StatusCode::OK, "{routed}");
        let ids = |jobs: &[Value]| -> Vec<String> {
            jobs.iter()
                .map(|job| job["job_id"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(ids(&jobs), ids(routed.as_array().unwrap()));
        let row = jobs
            .iter()
            .find(|job| job["job_id"] == job_id.as_str())
            .expect("the waiting job is listed");
        assert_eq!(row["state"], "needs_sign_in");
        assert_eq!(row["stage"], "sign_in");
        assert_eq!(row["intent"], "check");
        assert_eq!(row["server"], login.as_str());
        assert_eq!(row["code"], "crew_ssh_auth_required");
        assert!(row["next_step"].is_string(), "{row}");
        // Nothing the job found on the server is listed.
        for key in ["stages", "workspaces", "installed", "outcome"] {
            assert!(row.get(key).is_none(), "{key}: {row}");
        }

        // The listed ID is the one `auth` signs in to: from this chat, the job's own Sign in
        // card. The person closes it here, so nothing is signed in and the job still waits.
        let asked = call(
            &session,
            public(),
            PersonTool::Change,
            "auth",
            json!({"job_id": job_id}),
            &[],
        )
        .await;
        assert_eq!(asked.cards.len(), 1, "{:?}", asked.cards);
        assert_eq!(asked.cards[0].kind, "crewStep", "{:?}", asked.cards);
        assert_eq!(asked.cards[0].step["kind"], "signIn");
        assert_eq!(
            asked.cards[0].step["target"],
            json!({"kind": "setup", "jobId": job_id})
        );
        assert_eq!(asked.cards[0].step["login"], login.as_str());
        assert_eq!(asked.ok()["job_id"], job_id.as_str());
        assert_eq!(asked.ok()["state"], "needs_sign_in");

        let (status, _) = http("DELETE", &format!("/crew/setup/{job_id}"), None).await;
        assert_eq!(status, StatusCode::OK);
    });
}

/// The host's chat gets the invitation `GET /crew/connections/{id}/invitation` builds, naming
/// the person invited; a member who is not the host is refused, and a name that is not an
/// account is refused before anything is asked.
#[test]
#[serial]
fn the_invitation_is_the_route_s_and_only_the_host_gets_it() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0001";
        const HOST: &str = "invite.example.test";
        let connection = workspace(WORKSPACE, HOST, false, |scenario| {
            scenario.answer("workspace.snapshot", snapshot("private", false))
        })
        .await;
        let session = chat("invite").await;

        let invited = call(
            &session,
            public(),
            PersonTool::Read,
            "connections_invitation",
            json!({"connection": connection.id, "invitee": "@bob"}),
            &[],
        )
        .await;
        assert!(invited.cards.is_empty(), "an invitation needs no card");
        let (status, routed) = http(
            "GET",
            &format!("/crew/connections/{}/invitation?invitee=bob", connection.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{routed}");
        assert_eq!(invited.ok()["message"], routed["message"]);
        assert_eq!(invited.ok()["line"], routed["line"]);
        assert_eq!(invited.ok()["invitee"], "@bob");
        assert!(
            invited.ok()["line"]
                .as_str()
                .unwrap()
                .starts_with("brcrew1:"),
            "{}",
            invited.ok()
        );
        assert_eq!(
            classification(&session).await,
            SessionClassification::Public,
            "an invitation is not workspace content"
        );

        let wrong = call(
            &session,
            public(),
            PersonTool::Read,
            "connections_invitation",
            json!({"connection": connection.id, "invitee": "not a name"}),
            &[],
        )
        .await;
        assert!(
            wrong.refusal().contains("crew_invalid_selector"),
            "{}",
            wrong.refusal()
        );

        // The same workspace, now hosted by Bob: Alice is a member.
        let mut scenario = staged(HOST);
        scenario.answers.insert(
            "workspace.snapshot".into(),
            snapshot_with("private", false, "p-bob", json!([own_device()])),
        );
        stage(HOST, &scenario);
        let member = call(
            &session,
            public(),
            PersonTool::Read,
            "connections_invitation",
            json!({"connection": connection.id}),
            &[],
        )
        .await;
        assert!(
            member
                .refusal()
                .contains("Only the workspace host can send an invitation"),
            "{}",
            member.refusal()
        );
        assert!(member.refusal().contains("crew_host_required"));
        remove(&connection).await;
    });
}

/// `context_draft` asks the person once, runs the probe the route runs and answers the
/// route's draft, saving nothing; the chat is raised Private before it returns. A public model
/// is refused for a Private workspace before any card.
#[test]
#[serial]
fn a_context_draft_asks_first_and_returns_as_content() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0002";
        const HOST: &str = "draft.example.test";
        let connection = workspace(WORKSPACE, HOST, false, |scenario| {
            scenario.answer("workspace.snapshot", snapshot("private", false))
        })
        .await;

        let public_chat = chat("draft public").await;
        let refused = call(
            &public_chat,
            public(),
            PersonTool::Change,
            "context_draft",
            json!({"connection": connection.id}),
            &[],
        )
        .await;
        assert!(refused.cards.is_empty(), "{:?}", refused.cards);
        assert!(
            refused.refusal().contains("public model"),
            "{}",
            refused.refusal()
        );

        let session = chat("draft").await;
        crew()
            .grant_person_read(&session, &connection.id)
            .await
            .unwrap();
        let denied = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "context_draft",
            json!({"connection": connection.id}),
            &["deny"],
        )
        .await;
        assert_eq!(denied.cards.len(), 1, "{:?}", denied.cards);
        assert!(
            denied.refusal().contains("declined"),
            "{}",
            denied.refusal()
        );
        assert_eq!(
            classification(&session).await,
            SessionClassification::Public
        );

        let drafted = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "context_draft",
            json!({"connection": connection.id}),
            &["allow_once"],
        )
        .await;
        let card = drafted.approval();
        assert_eq!(drafted.cards.len(), 1);
        assert_eq!(card.tool, "crew__change");
        // The workspace as its snapshot names it, on its server.
        let server = biorouter::crew::server_label(&connection.ssh_target, connection.port).await;
        assert!(
            card.prompt.starts_with(&format!(
                "Draft the context document for lab on {server} from what the server reports about itself"
            )),
            "{}",
            card.prompt
        );
        let (status, routed) = http(
            "POST",
            &format!("/crew/connections/{}/context/draft", connection.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{routed}");
        assert_eq!(drafted.ok()["draft"], routed["draft"]);
        assert!(
            drafted.ok()["draft"].as_str().unwrap().contains(PROBE_HOST),
            "{}",
            drafted.ok()
        );
        assert_eq!(drafted.ok()["saved"], false);
        assert_eq!(
            classification(&session).await,
            SessionClassification::Private
        );

        // In a chat the person has not let read the workspace, the consent comes first.
        let fresh = chat("draft fresh").await;
        let asked = call(
            &fresh,
            private_local().await,
            PersonTool::Change,
            "context_draft",
            json!({"connection": connection.id}),
            &["allow_once", "allow_once"],
        )
        .await;
        assert_eq!(asked.cards.len(), 2, "{:?}", asked.cards);
        assert!(asked.cards[0].is_consent());
        assert_eq!(asked.ok()["draft"], routed["draft"]);
        remove(&connection).await;
    });
}

/// New computers are marked as seen only by `profile_devices_acknowledge`, on a card that
/// lists each one with how and when it was added. Reads that show them change nothing, a
/// denied card changes nothing, and a fingerprint that is not new is refused with no card.
#[test]
#[serial]
fn acknowledging_takes_a_card_and_never_happens_on_a_read() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0003";
        const HOST: &str = "devices.example.test";
        const LAPTOP: &str = "BBBB BBBB BBBB BBBB";
        let connection = workspace(WORKSPACE, HOST, false, |scenario| {
            scenario.answer("workspace.snapshot", snapshot("private", false))
        })
        .await;
        // The first snapshot is what this computer knows; a computer added after it is new.
        crew()
            .human_request(&connection.id, "workspace.snapshot", json!({}), None)
            .await
            .unwrap();
        let mut scenario = staged(HOST);
        scenario.answers.insert(
            "workspace.snapshot".into(),
            snapshot_with(
                "private",
                false,
                "p-alice",
                json!([own_device(), {"fingerprint": LAPTOP, "added_via": "account", "added_at": 1_790_200_000u64}]),
            ),
        );
        stage(HOST, &scenario);
        crew()
            .human_request(&connection.id, "workspace.snapshot", json!({}), None)
            .await
            .unwrap();
        let new = || async {
            crew()
                .new_devices(&connection.id)
                .await
                .unwrap()
                .into_iter()
                .map(|device| device.fingerprint)
                .collect::<Vec<_>>()
        };
        assert_eq!(new().await, vec![LAPTOP.to_owned()]);

        let session = chat("devices").await;
        for action in ["connections_show", "profile_devices"] {
            let read = call(
                &session,
                public(),
                PersonTool::Read,
                action,
                json!({"connection": connection.id}),
                &["allow_once"],
            )
            .await;
            read.ok();
        }
        assert_eq!(new().await, vec![LAPTOP.to_owned()], "a read acknowledged");

        let unknown = call(
            &session,
            public(),
            PersonTool::Change,
            "profile_devices_acknowledge",
            json!({"connection": connection.id, "fingerprints": ["1111 2222 3333 4444"]}),
            &[],
        )
        .await;
        assert!(unknown.cards.is_empty());
        assert!(
            unknown.refusal().contains("isn't one of the new computers"),
            "{}",
            unknown.refusal()
        );

        let denied = call(
            &session,
            public(),
            PersonTool::Change,
            "profile_devices_acknowledge",
            json!({"connection": connection.id, "fingerprints": ["bbbb-bbbb-bbbb-bbbb"]}),
            &["deny"],
        )
        .await;
        let card = denied.approval();
        assert!(
            card.prompt.starts_with(&format!(
                "Mark a new computer added to your account in {} as seen",
                here(&connection).await
            )),
            "{}",
            card.prompt
        );
        assert!(
            card.prompt.contains(&format!("{LAPTOP}, added ")),
            "{}",
            card.prompt
        );
        assert!(card.prompt.contains("by signing in to the server"));
        assert_eq!(card.arguments["fingerprints"], json!([LAPTOP]));
        assert_eq!(
            new().await,
            vec![LAPTOP.to_owned()],
            "a denial acknowledged"
        );

        let allowed = call(
            &session,
            public(),
            PersonTool::Change,
            "profile_devices_acknowledge",
            json!({"connection": connection.id, "fingerprints": [LAPTOP]}),
            &["allow_once"],
        )
        .await;
        assert_eq!(allowed.cards.len(), 1);
        assert_eq!(allowed.ok()["acknowledged"], json!([LAPTOP]));
        assert!(new().await.is_empty());
        let (status, routed) =
            http("GET", &format!("/crew/connections/{}", connection.id), None).await;
        assert_eq!(status, StatusCode::OK, "{routed}");
        assert!(
            routed["new_devices"].as_array().is_none_or(Vec::is_empty),
            "{routed}"
        );
        remove(&connection).await;
    });
}

/// `files_pending` lists the transfers `GET /crew/transfers?connection_id=` lists. A public
/// model reads only a Public workspace: a transfer in a protected channel is counted, not
/// listed; a restricted file in a channel it may see is listed by its ID, state, direction and
/// size alone, and `files_status` does not name it either; resuming an upload it may not name
/// says one sentence whatever file it names; and the chat stays Public. A private model reads
/// every name, and the chat is raised Private before the answer returns.
#[test]
#[serial]
fn pending_transfers_are_read_as_content() {
    run(async {
        if !transfer_store_reachable("pending_transfers_are_read_as_content") {
            return;
        }
        let folder = private_folder();
        let general = folder.path().join("general.csv");
        let patients = folder.path().join("patients.csv");
        fs::write(&general, b"a,b\n1,2\n").unwrap();
        fs::write(&patients, b"id\n7\n").unwrap();

        const OPEN: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0004";
        const OPEN_HOST: &str = "pending-public.example.test";
        // An attachment in the public-safe channel that the workspace restricts.
        const RESTRICTED: &str = "6d1f2a3b-0000-4000-8000-00000000d0d4";
        let mut restricted = attachment(RESTRICTED, "c-general", b"0123456789", 10, true);
        restricted["result"]["restricted"] = json!(true);
        let open = workspace(OPEN, OPEN_HOST, true, |scenario| {
            scenario
                .answer("workspace.snapshot", snapshot("public", true))
                .answer("blob.status", restricted)
        })
        .await;
        let shown = id_of(&start_transfer(&open, "c-general", "upload", &general, None).await);
        let hidden = id_of(&start_transfer(&open, "c-patients", "upload", &patients, None).await);
        let downloads = private_folder();
        let secret = downloads.path().join("secret-cohort.csv");
        let kept =
            id_of(&start_transfer(&open, "c-general", "download", &secret, Some(RESTRICTED)).await);
        reached(&shown, stopped).await;
        reached(&hidden, stopped).await;
        reached(&kept, stopped).await;

        let public_chat = chat("pending public").await;
        let pending = call(
            &public_chat,
            public(),
            PersonTool::Read,
            "files_pending",
            json!({"connection": open.id}),
            &["allow_once"],
        )
        .await;
        let (status, routed) = http(
            "GET",
            &format!("/crew/transfers?connection_id={}", open.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{routed}");
        let ids = |list: &Value| -> Vec<String> {
            let mut ids: Vec<String> = list
                .as_array()
                .unwrap()
                .iter()
                .map(|transfer| {
                    transfer["transfer_id"]
                        .as_str()
                        .or_else(|| transfer["id"].as_str())
                        .unwrap()
                        .to_owned()
                })
                .collect();
            ids.sort();
            ids
        };
        // A transfer in a channel the read may not see is counted, not listed: not even its ID.
        let listed = &pending.ok()["transfers"];
        let mut expected = ids(&routed["transfers"]);
        expected.retain(|id| *id != hidden);
        assert_eq!(ids(listed), expected);
        assert_eq!(pending.ok()["withheld"]["transfers"], 1, "{}", pending.ok());
        let entry = |id: &str| {
            listed
                .as_array()
                .unwrap()
                .iter()
                .find(|transfer| transfer["transfer_id"] == id)
                .unwrap()
                .clone()
        };
        assert_eq!(entry(&shown)["name"], "general.csv");
        // The restricted download is listed, but as its ID, state, direction and size alone:
        // nothing that would name it through another read.
        let unnamed = entry(&kept);
        let mut keys: Vec<&str> = unnamed
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["direction", "size", "state", "transfer_id"],
            "{unnamed}"
        );
        assert_eq!(pending.ok()["withheld"]["names"], 1, "{}", pending.ok());
        let said = pending.ok().to_string();
        for never in [
            hidden.as_str(),
            "patients.csv",
            "secret-cohort.csv",
            RESTRICTED,
            "c-patients",
        ] {
            assert!(!said.contains(never), "{never} in {said}");
        }
        assert_eq!(
            classification(&public_chat).await,
            SessionClassification::Public
        );

        // `files_status` names a transfer only where `files_pending` would: the withheld
        // download stays unnamed, by the ID the listing gave.
        for id in [&kept, &hidden] {
            let status = call(
                &public_chat,
                public(),
                PersonTool::Read,
                "files_status",
                json!({"transfer_id": id}),
                &[],
            )
            .await;
            assert!(status.cards.is_empty(), "{:?}", status.cards);
            let said = status.ok().to_string();
            assert!(status.ok().get("name").is_none(), "{said}");
            for never in [
                "patients.csv",
                "secret-cohort.csv",
                RESTRICTED,
                "c-patients",
            ] {
                assert!(!said.contains(never), "{never} in {said}");
            }
        }
        assert_eq!(
            classification(&public_chat).await,
            SessionClassification::Public
        );

        // Resuming the hidden upload can't tell the model whether a file of its choosing is
        // the one it shares: the same file and another one get the same sentence, and no card.
        for path in [&patients, &general] {
            let resume = call(
                &public_chat,
                public(),
                PersonTool::Change,
                "files_resume",
                json!({"transfer_id": hidden, "path": path}),
                &[],
            )
            .await;
            assert!(resume.cards.is_empty(), "{:?}", resume.cards);
            assert_eq!(resume.refusal(), UPLOAD_NOT_NAMEABLE);
        }

        const SHUT: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0005";
        const SHUT_HOST: &str = "pending-private.example.test";
        let shut = workspace(SHUT, SHUT_HOST, false, |scenario| {
            scenario.answer("workspace.snapshot", snapshot("private", true))
        })
        .await;
        let private = id_of(&start_transfer(&shut, "c-patients", "upload", &patients, None).await);
        reached(&private, stopped).await;
        let refused = call(
            &public_chat,
            public(),
            PersonTool::Read,
            "files_pending",
            json!({"connection": shut.id}),
            &[],
        )
        .await;
        assert!(refused.cards.is_empty());
        assert!(
            refused.refusal().contains("public model"),
            "{}",
            refused.refusal()
        );

        let private_chat = chat("pending private").await;
        let read = call(
            &private_chat,
            private_local().await,
            PersonTool::Read,
            "files_pending",
            json!({"connection": shut.id}),
            &["allow_once"],
        )
        .await;
        assert_eq!(read.ok()["transfers"][0]["name"], "patients.csv");
        assert_eq!(read.ok()["withheld"]["names"], 0);
        assert_eq!(
            classification(&private_chat).await,
            SessionClassification::Private
        );
        for id in [&shown, &hidden, &kept, &private] {
            let _ = http("DELETE", &format!("/crew/transfers/{id}"), None).await;
        }
        remove(&open).await;
        remove(&shut).await;
    });
}

/// Pausing, resuming and forgetting a transfer each park exactly one card naming the transfer
/// and where it goes, change nothing when denied, and do what their routes do when allowed.
/// The answers carry states, never the file's name. A model that may not see which file an
/// upload shares can't resume it.
#[test]
#[serial]
fn pause_resume_and_forget_each_take_one_card() {
    run(async {
        if !transfer_store_reachable("pause_resume_and_forget_each_take_one_card") {
            return;
        }
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0006";
        const HOST: &str = "transfers.example.test";
        const BLOB: &str = "6d1f2a3b-0000-4000-8000-00000000b10b";
        let folder = private_folder();
        let path = folder.path().join("plot.png");
        let bytes = vec![7u8; 4096];
        fs::write(&path, &bytes).unwrap();
        // The workspace takes its time to begin the attachment: the upload is under way.
        let connection = workspace(WORKSPACE, HOST, false, |scenario| {
            scenario
                .answer("workspace.snapshot", snapshot("private", false))
                .answer(
                    "blob.begin",
                    attachment(BLOB, "c-general", &bytes, 0, false),
                )
                .delay("blob.begin", 8000)
        })
        .await;
        let id = id_of(&start_transfer(&connection, "c-general", "upload", &path, None).await);
        reached(&id, |state| state == "uploading").await;
        let session = chat("transfers").await;
        let where_to = here(&connection).await;

        let paused = call(
            &session,
            public(),
            PersonTool::Change,
            "files_pause",
            json!({"transfer_id": id}),
            &["allow_once"],
        )
        .await;
        let card = paused.approval();
        assert_eq!(paused.cards.len(), 1);
        assert_eq!(
            card.prompt.lines().next().unwrap(),
            format!("Pause the upload of plot.png (4 KB) to {where_to}")
        );
        assert_eq!(card.arguments["transfer_id"], id.as_str());
        assert_eq!(paused.ok()["paused"], true, "{}", paused.ok());
        assert!(paused.ok().get("name").is_none(), "{}", paused.ok());
        let receipt = reached(&id, stopped).await;
        assert_eq!(receipt["state"], "needs_file_selection", "{receipt}");

        // Nothing is moving now, so there is nothing to pause, and no card.
        let again = call(
            &session,
            public(),
            PersonTool::Change,
            "files_pause",
            json!({"transfer_id": id}),
            &[],
        )
        .await;
        assert!(again.cards.is_empty());
        assert!(
            again.refusal().contains("isn't moving"),
            "{}",
            again.refusal()
        );

        // From here the workspace answers at once, and refuses the rest of the file.
        let mut scenario = staged(HOST);
        scenario.delays.clear();
        scenario.answers.insert(
            "blob.status".into(),
            attachment(BLOB, "c-general", &bytes, 0, false),
        );
        stage(HOST, &scenario);
        let wrong = folder.path().join("other.png");
        fs::write(&wrong, &bytes).unwrap();
        // A public model may not see which file an upload into a Private workspace shares, so
        // it can't resume one: the file at its path is never compared with the transfer's.
        let public_resume = call(
            &session,
            public(),
            PersonTool::Change,
            "files_resume",
            json!({"transfer_id": id, "path": path}),
            &[],
        )
        .await;
        assert!(public_resume.cards.is_empty(), "{:?}", public_resume.cards);
        assert_eq!(public_resume.refusal(), UPLOAD_NOT_NAMEABLE);
        let mistaken = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "files_resume",
            json!({"transfer_id": id, "path": wrong}),
            &[],
        )
        .await;
        assert!(mistaken.cards.is_empty());
        assert!(
            mistaken
                .refusal()
                .contains("isn't the file this transfer was sharing"),
            "{}",
            mistaken.refusal()
        );
        let denied = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "files_resume",
            json!({"transfer_id": id, "path": path}),
            &["deny"],
        )
        .await;
        let card = denied.approval();
        assert!(
            card.prompt
                .starts_with("Resume sharing plot.png (4 KB) from this computer in #general in "),
            "{}",
            card.prompt
        );
        assert!(
            card.prompt.contains(&format!("From {}.", path.display())),
            "{}",
            card.prompt
        );
        assert_eq!(card.arguments["path"], path.display().to_string());
        assert_eq!(card.arguments["size_bytes"], 4096);
        assert_eq!(transfer(&id).await.1["state"], "needs_file_selection");

        let resumed = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "files_resume",
            json!({"transfer_id": id, "path": path}),
            &["allow_once"],
        )
        .await;
        assert_eq!(resumed.cards.len(), 1);
        assert_eq!(resumed.ok()["resumed"], true, "{}", resumed.ok());
        assert!(resumed.ok().get("name").is_none());
        reached(&id, stopped).await;

        let kept = call(
            &session,
            public(),
            PersonTool::Change,
            "files_forget",
            json!({"transfer_id": id}),
            &["deny"],
        )
        .await;
        assert!(
            kept.approval().prompt.starts_with(&format!(
                "Remove from your list: the upload of plot.png (4 KB) to {where_to}\nRemoves the record on this computer."
            )),
            "{}",
            kept.approval().prompt
        );
        assert_eq!(transfer(&id).await.0, StatusCode::OK, "a denial forgot it");
        let forgotten = call(
            &session,
            public(),
            PersonTool::Change,
            "files_forget",
            json!({"transfer_id": id}),
            &["allow_once"],
        )
        .await;
        assert_eq!(forgotten.cards.len(), 1);
        assert_eq!(forgotten.ok()["forgotten"], true);
        assert_eq!(transfer(&id).await.0, StatusCode::BAD_REQUEST);
        assert!(path.exists(), "the shared file stays");
        remove(&connection).await;
    });
}

/// Forgetting an unfinished download deletes its partial file, and its card names that file's
/// path; a denial leaves it, and the download's own file is never made.
#[test]
#[serial]
fn forgetting_a_download_names_and_deletes_its_partial_file() {
    run(async {
        if !transfer_store_reachable("forgetting_a_download_names_and_deletes_its_partial_file") {
            return;
        }
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0007";
        const HOST: &str = "partial.example.test";
        const BLOB: &str = "6d1f2a3b-0000-4000-8000-00000000d0d0";
        let bytes = b"0123456789".to_vec();
        // The workspace describes the attachment and then never sends a piece of it.
        let connection = workspace(WORKSPACE, HOST, false, |scenario| {
            scenario
                .answer("workspace.snapshot", snapshot("private", false))
                .answer(
                    "blob.status",
                    attachment(BLOB, "c-general", &bytes, 10, true),
                )
        })
        .await;
        let folder = private_folder();
        let saved = folder.path().join("report.csv");
        let id =
            id_of(&start_transfer(&connection, "c-general", "download", &saved, Some(BLOB)).await);
        let receipt = reached(&id, stopped).await;
        assert!(receipt["destination_identity"].is_string(), "{receipt}");
        let part = folder.path().join(format!(".biorouter-crew-{id}.part"));
        assert!(part.exists(), "the download began writing");

        // The chat works in the folder the download was saving to.
        let session = chat_in("partial", folder.path()).await;
        let kept = call(
            &session,
            public(),
            PersonTool::Change,
            "files_forget",
            json!({"transfer_id": id}),
            &["deny"],
        )
        .await;
        let card = kept.approval();
        assert!(
            card.prompt.starts_with(&format!(
                "Remove from your list: the download of report.csv (10 bytes) from {}",
                here(&connection).await
            )),
            "{}",
            card.prompt
        );
        assert!(
            card.prompt.contains(&format!(
                "Its partial file is deleted from this computer: {}.",
                part.display()
            )),
            "{}",
            card.prompt
        );
        assert_eq!(card.arguments["partial_file"], part.display().to_string());
        assert!(part.exists(), "a denial deleted it");

        let forgotten = call(
            &session,
            public(),
            PersonTool::Change,
            "files_forget",
            json!({"transfer_id": id}),
            &["allow_once"],
        )
        .await;
        assert_eq!(forgotten.cards.len(), 1);
        assert_eq!(forgotten.ok()["partial_file_deleted"], true);
        assert!(!part.exists());
        assert!(!saved.exists());
        assert_eq!(transfer(&id).await.0, StatusCode::BAD_REQUEST);

        // A chat elsewhere can't tell where a download went, and says so with no card.
        let again =
            id_of(&start_transfer(&connection, "c-general", "download", &saved, Some(BLOB)).await);
        reached(&again, stopped).await;
        let elsewhere = chat("partial elsewhere").await;
        let lost = call(
            &elsewhere,
            public(),
            PersonTool::Change,
            "files_forget",
            json!({"transfer_id": again}),
            &[],
        )
        .await;
        assert!(lost.cards.is_empty());
        assert!(
            lost.refusal()
                .contains("can't tell from a chat where this download was saving to"),
            "{}",
            lost.refusal()
        );
        let _ = call(
            &session,
            public(),
            PersonTool::Change,
            "files_forget",
            json!({"transfer_id": again}),
            &["allow_once"],
        )
        .await;
        remove(&connection).await;
    });
}

/// Resuming a download brings a workspace file onto this computer, where the chat's own tools
/// can read it, so it is read as content, as `files_download` is: a public model is refused for
/// a Private workspace before any card, and a private model's chat is asked for its consent,
/// then for the resume, and is raised Private before the answer returns.
#[test]
#[serial]
fn resuming_a_download_is_read_as_content() {
    run(async {
        if !transfer_store_reachable("resuming_a_download_is_read_as_content") {
            return;
        }
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0008";
        const HOST: &str = "resume-download.example.test";
        const BLOB: &str = "6d1f2a3b-0000-4000-8000-00000000d0d1";
        let bytes = b"0123456789".to_vec();
        let connection = workspace(WORKSPACE, HOST, false, |scenario| {
            scenario
                .answer("workspace.snapshot", snapshot("private", false))
                .answer(
                    "blob.status",
                    attachment(BLOB, "c-general", &bytes, 10, true),
                )
        })
        .await;
        let folder = private_folder();
        let saved = folder.path().join("report.csv");
        let id =
            id_of(&start_transfer(&connection, "c-general", "download", &saved, Some(BLOB)).await);
        reached(&id, stopped).await;
        let resume = json!({"transfer_id": id, "path": saved});

        let public_chat = chat_in("resume public", folder.path()).await;
        let refused = call(
            &public_chat,
            public(),
            PersonTool::Change,
            "files_resume",
            resume.clone(),
            &[],
        )
        .await;
        assert!(refused.cards.is_empty(), "{:?}", refused.cards);
        assert!(
            refused.refusal().contains("public model"),
            "{}",
            refused.refusal()
        );
        assert_eq!(
            classification(&public_chat).await,
            SessionClassification::Public
        );

        let session = chat_in("resume private", folder.path()).await;
        let resumed = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "files_resume",
            resume,
            &["allow_once", "allow_once"],
        )
        .await;
        assert_eq!(resumed.cards.len(), 2, "{:?}", resumed.cards);
        assert!(resumed.cards[0].is_consent(), "{:?}", resumed.cards);
        let server = biorouter::crew::server_label(&connection.ssh_target, connection.port).await;
        assert!(
            resumed.approval().prompt.starts_with(&format!(
                "Resume downloading report.csv from #general in lab on {server} to {} on this computer",
                saved.display()
            )),
            "{}",
            resumed.approval().prompt
        );
        assert_eq!(resumed.ok()["resumed"], true, "{}", resumed.ok());
        assert!(resumed.ok().get("name").is_none(), "{}", resumed.ok());
        assert_eq!(
            classification(&session).await,
            SessionClassification::Private
        );

        reached(&id, stopped).await;
        let _ = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "files_forget",
            json!({"transfer_id": id}),
            &["allow_once"],
        )
        .await;
        remove(&connection).await;
    });
}
