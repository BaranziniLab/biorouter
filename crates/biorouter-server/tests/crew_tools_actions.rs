//! A chat's Crew person tools on the daemon's side, step 2 (account ownership AO-12): the team,
//! channel, membership, message, file and task actions of the host the daemon installs
//! (`routes::crew_tools`), driven the way the `crew` extension drives it, one `PersonToolCall` at
//! a time.
//!
//! What is pinned here, against the real host, the real `CrewManager`, the real transfer service
//! and the real approval registry and route:
//!
//! - every action runs now, and none answers that it is not available yet;
//! - every change parks exactly one proof-backed card naming its resolved targets (deletes,
//!   archives and removals "for everyone; this cannot be undone"), sends nothing when denied,
//!   and sends exactly the IDs and guards the card named when allowed;
//! - message, file, context and task reads ask the person once per chat, and withhold from a
//!   public model every restricted message and file, and every channel that is not public-safe,
//!   counting what they withheld; a private model's read raises the chat Private first;
//! - a write from a Private chat is marked Private, its card says when it crosses into
//!   something Public, and where it could not be kept private it is refused before any card;
//! - a task asked from a Private chat starts with its origin restricted, so a public model is
//!   refused as a grant would refuse it.
//!
//! **Offline.** The daemon's `ssh` is this binary, a fake OpenSSH and bridge answering from a
//! per-host scenario file and logging every frame it receives (copied from `crew_tools_host.rs`).
//! **One runtime** and **its own binary**, for the same reasons as there.
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
use biorouter::crew::tool_host::{self, PersonTool, PersonToolCall, CHANGE_ACTIONS, READ_ACTIONS};
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
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// Set only in the fake `ssh` process: the fixture directory it reads scenarios from.
const FAKE_SSH_ROOT: &str = "BIOROUTER_CREW_TOOLS_ACTIONS_FAKE_SSH";
/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-tools-actions-user-action-key";
/// The daemon secret the full router checks.
const SECRET: &str = "crew-tools-actions-secret";
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
// Scenarios: what the fake broker answers, per SSH host. Written by the tests, re-read by the
// fake on every frame so a test can change the answers mid-connection.
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
    /// `ssh -G` resolves this scenario's host to this name (a local alias's real server).
    resolves_to: Option<String>,
    /// What `ssh -G` says this host's `StrictHostKeyChecking` is; `yes` when absent.
    #[serde(default)]
    host_key_checking: Option<String>,
    /// The account `ssh -G` says this host (an alias with no user in it) signs in as.
    #[serde(default)]
    user: Option<String>,
    /// Method → the whole envelope, `{"result": …}` or `{"error": …}`. `method#name` answers
    /// only a frame whose `name` (or `label`) parameter is `name`, before `method` does.
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
            resolves_to: None,
            host_key_checking: None,
            user: None,
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

fn log_path(root: &Path, host: &str) -> PathBuf {
    root.join("scenarios").join(format!("{host}.log"))
}

/// Write `scenario` for `host`, atomically, so the fake never reads half a file.
fn stage(host: &str, scenario: &Scenario) {
    let path = scenario_path(&fixture_root(), host);
    let partial = path.with_extension("partial");
    fs::write(&partial, serde_json::to_vec(scenario).unwrap()).unwrap();
    fs::rename(partial, path).unwrap();
}

/// Every frame the fake bridge for `host` received with `method`, parsed.
fn received(host: &str, method: &str) -> Vec<Value> {
    fs::read_to_string(log_path(&fixture_root(), host))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|frame| frame["method"] == method)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The fake `ssh`, run by this same binary before `main`.
// ---------------------------------------------------------------------------------------------

mod fake_ssh {
    use super::{log_path, scenario_path, workspace_key, Scenario, JOINER_UID};
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
            let scenario = load(root, &host);
            let hostname = scenario
                .as_ref()
                .and_then(|scenario| scenario.resolves_to.clone())
                .unwrap_or_else(|| host.clone());
            let user = scenario.as_ref().and_then(|scenario| scenario.user.clone());
            let checking = scenario
                .and_then(|scenario| scenario.host_key_checking)
                .unwrap_or_else(|| "yes".into());
            let mut out = std::io::stdout().lock();
            // First wins in both of the daemon's readers, so the host's own lines go first: its
            // jump route as `-J` names it, as OpenSSH's own `-G` reports it, then its policy.
            let _ = writeln!(out, "hostname {hostname}\nport 22");
            if let Some(user) = user {
                let _ = writeln!(out, "user {user}");
            }
            if let Some(jump) = args
                .windows(2)
                .find(|pair| pair[0] == "-J")
                .map(|pair| pair[1].clone())
            {
                let _ = writeln!(out, "proxyjump {jump}");
            }
            let _ = writeln!(out, "stricthostkeychecking {checking}");
            for line in [
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
            if let Ok(mut log) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path(root, host))
            {
                let _ = writeln!(log, "{line}");
            }
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
                method => {
                    let named = frame["params"]["name"]
                        .as_str()
                        .or_else(|| frame["params"]["label"].as_str())
                        .and_then(|name| scenario.answers.get(&format!("{method}#{name}")));
                    named.or_else(|| scenario.answers.get(method)).cloned().unwrap_or_else(|| {
                        json!({"error": {"code": "unsupported", "message": "unsupported: fixture has no answer"}})
                    })
                }
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

/// This test process's fixture directory: the `ssh` script, scenarios and logs, and the
/// development profile the file credential backend writes under.
fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "biorouter-crew-tools-actions-{}",
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
            "crew_tools_actions: could not prepare the fixture at {}: {error}. Refusing to run: \
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
    // A public model a task can be asked to run on. Nothing is ever sent to it: a task from a
    // Private chat is refused before the model is asked anything.
    std::env::set_var("OPENAI_API_KEY", "fixture-key-never-sent");
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

/// A person's own chat, as the agent loop would hold it.
async fn chat(name: &str) -> String {
    state()
        .await
        .session_manager()
        .create_session(
            std::env::temp_dir().join("crew-tools-actions"),
            name.to_owned(),
            SessionType::User,
        )
        .await
        .expect("a chat")
        .id
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
// Capabilities: what the chat's model is, sampled as dispatch samples it
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
        "crew-tools-host-fixture"
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

async fn sampled(tier: ProviderTier, affiliation: Option<ModelAffiliation>) -> CallCapability {
    let provider: biorouter::agents::types::SharedProvider =
        Arc::new(tokio::sync::Mutex::new(Some(Arc::new(Model {
            tier,
            affiliation,
        }))));
    CallCapability::sample(&provider).await
}

/// A model on this machine: Private, and compatible with every institution.
async fn private_local() -> CallCapability {
    sampled(ProviderTier::Private, Some(ModelAffiliation::Local)).await
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
    tool: String,
    prompt: String,
    arguments: Value,
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
}

/// Call `tool`'s `action` from `session` on `capability`, and answer each card it parks with the
/// next of `answers` (`allow_once` or `deny`) through `POST /action-required/tool-confirmation`
/// with the desktop's proof, as the person's click does. A card beyond `answers` is denied.
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
            cards.push(SeenCard {
                tool: data["toolName"].as_str().unwrap_or_default().to_owned(),
                prompt: data["prompt"].as_str().unwrap_or_default().to_owned(),
                arguments: data["arguments"].clone(),
            });
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
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Called {
        answer: task.await.expect("the call ran"),
        cards,
    }
}

/// [`call`] with one card, answered with `answer` only after `meanwhile` ran while it waited:
/// what changes while a card is open for the person.
async fn call_meanwhile(
    session: &str,
    capability: CallCapability,
    tool: PersonTool,
    action: &str,
    params: Value,
    answer: &str,
    meanwhile: impl std::future::Future<Output = ()>,
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
    let mut meanwhile = Some(meanwhile);
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
            cards.push(SeenCard {
                tool: data["toolName"].as_str().unwrap_or_default().to_owned(),
                prompt: data["prompt"].as_str().unwrap_or_default().to_owned(),
                arguments: data["arguments"].clone(),
            });
            if let Some(change) = meanwhile.take() {
                change.await;
            }
            let answer = if cards.len() == 1 { answer } else { "deny" };
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
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Called {
        answer: task.await.expect("the call ran"),
        cards,
    }
}

// ---------------------------------------------------------------------------------------------
// Workspaces
// ---------------------------------------------------------------------------------------------

fn invitation(workspace_id: &str, ssh_host: &str, institution: Option<&str>) -> String {
    biorouter_crew::invitation::message(&biorouter_crew::invitation::WorkspaceInvitation {
        workspace_id: workspace_id.into(),
        workspace_public_key: workspace_key::workspace().public_hex(),
        socket_path: SOCKET.into(),
        owner_uid: HOST_UID,
        workspace_name: Some("lab".into()),
        host_username: Some("alice".into()),
        host_display_name: Some("Alice Chen".into()),
        mode: Some(biorouter_crew::Mode::Private),
        institution_id: institution.map(str::to_owned),
        ssh_host: Some(ssh_host.into()),
        ssh_port: None,
        proxy_jump: None,
        invitee_username: Some("alice".into()),
    })
    .unwrap()
}

const GENERAL: &str = "c-general";
const METHODS: &str = "c-methods";
const PATIENTS: &str = "c-patients";
/// An attachment's ID, UUID-shaped as the workspace makes them.
const BLOB: &str = "6b1f0c2e-1d4a-4c5e-9a0b-00000000b10b";

/// What Alice sees: herself (the host) and Bob in one team, Analysis Lab, with #general and
/// #methods (public-safe) and #patients (restricted, and protected when `protected` says so),
/// and an invitation to another team.
fn snapshot(mode: &str, protected: bool) -> Value {
    json!({
        "workspace": {
            "id": "w", "name": "lab", "mode": mode, "institution_id": "ucsf",
            "host_principal_id": "p-alice", "policy_epoch": 1, "governance": "managed",
        },
        "actor": {"id": "p-alice", "username": "alice", "display_name": "Alice Chen", "devices": []},
        "principals": [
            {"id": "p-alice", "username": "alice", "display_name": "Alice Chen"},
            {"id": "p-bob", "username": "bob", "display_name": "Bob Lee"},
        ],
        "former_principals": [],
        "teams": [{
            "id": "t-lab", "name": "analysis-lab", "display_name": "Analysis Lab",
            "general_channel_id": "c-general", "members": ["p-alice", "p-bob"],
            "name_conflict": false,
        }],
        "channels": [
            {"id": "c-general", "team_id": "t-lab", "name": "general",
             "classification": "public_safe", "name_conflict": false},
            {"id": "c-methods", "team_id": "t-lab", "name": "methods",
             "classification": "public_safe", "name_conflict": false},
            {"id": "c-patients", "team_id": "t-lab", "name": "patients",
             "classification": "restricted", "name_conflict": false},
        ],
        "protected_channel_ids": if protected { json!([PATIENTS]) } else { json!([]) },
        "invitations": [{
            "id": "i-field", "kind": "team", "target_id": "t-field", "principal_id": "p-alice",
            "inviter_id": "p-bob", "expires_at": 4_000_000_000u64, "target_name": "field-team",
        }],
        "unread": {"c-general": 2},
        "totals": {"teams": 1, "channels": 3},
    })
}

/// `snapshot` in an open workspace, with a team and two channels Alice may join, one of them
/// restricted.
fn open_snapshot(mode: &str, protected: bool) -> Value {
    let mut open = snapshot(mode, protected);
    open["workspace"]["governance"] = json!("open");
    open["joinable_teams"] = json!([
        {"id": "t-field", "name": "field-team", "display_name": "Field Team", "handle": "field-team"},
    ]);
    open["joinable_channels"] = json!([
        {"id": "c-field-notes", "team_id": "t-lab", "name": "field-notes",
         "classification": "public_safe"},
        {"id": "c-secret", "team_id": "t-lab", "name": "secret-notes",
         "classification": "restricted"},
    ]);
    open
}

/// Two messages in #general, the second restricted and drawn from #patients, each with an
/// attachment.
fn history() -> Value {
    json!({
        "messages": [
            {"id": "m-1", "sequence": "m-1", "channel_id": "c-general", "actor_id": "p-bob",
             "body": "open notes", "restricted": false, "source_channels": ["c-general"],
             "attachments": ["a-open"], "references": [], "created_at": 1},
            {"id": "m-2", "sequence": "m-2", "channel_id": "c-general", "actor_id": "p-alice",
             "body": "patient counts", "restricted": true,
             "source_channels": ["c-general", "c-patients"], "attachments": ["a-private"],
             "references": [], "created_at": 2},
        ],
        "cursor": "m-2",
        "people": {"p-bob": {"username": "bob"}, "p-alice": {"username": "alice"}},
        "channel_names": {"c-general": "general", "c-patients": "patients"},
    })
}

/// An attachment in #general as `blob.status` answers it.
fn blob(restricted: bool) -> Value {
    json!({"result": {
        "id": BLOB, "channel_id": GENERAL, "name": "notes.csv", "size": 12,
        "media_type": "text/csv", "restricted": restricted,
        "sha256": "0".repeat(64), "offset": 12, "complete": true,
    }})
}

/// What the fake workspace answers: `snapshot` for the person's view, and every change step 2
/// sends.
fn scenario(workspace_id: &str, mode: &str, snapshot: Value) -> Scenario {
    let mut scenario = Scenario::new(workspace_id, &ACCOUNT_CAPABILITIES)
        .answer("workspace.snapshot", json!({"result": snapshot}))
        .answer(
            "team.create",
            json!({"result": {"id": "t-new", "name": "analysis-2", "general_channel_id": "c-new-general"}}),
        )
        .answer(
            "team.rename",
            json!({"result": {"id": "t-lab", "name": "analysis-two", "display_name": "Analysis Two"}}),
        )
        .answer("team.delete", json!({"result": {"deleted": true, "channels": 3}}))
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
            json!({"result": {"id": "m-new", "sequence": "m-new", "restricted": true}}),
        )
        .answer(
            "reference.create",
            json!({"result": {"id": "r-new", "restricted": true}}),
        )
        .answer("messages.history", json!({"result": history()}))
        .answer("messages.search", json!({"result": history()}))
        .answer("blob.status", blob(false))
        .answer(
            "workspace.context.get",
            json!({"result": {"context": {"text": "Run jobs with sbatch.", "revision": 3}}}),
        );
    scenario.mode = mode.into();
    scenario
}

/// A workspace on `host` answering from `scenario`, saved on this computer from its invitation
/// (with a personal Public connection when `connection_public`, else Private and `ucsf`), and
/// connected. Answers the connection.
async fn workspace(
    host: &str,
    connection_public: bool,
    scenario: Scenario,
) -> biorouter::crew::Connection {
    stage(host, &scenario);
    let overrides = biorouter::crew::authentication::InvitationOverrides {
        mode: connection_public.then_some(biorouter::crew::ClusterMode::Public),
        ..Default::default()
    };
    let saved = match crew()
        .connection_from_invitation(
            &invitation(&scenario.workspace_id, host, Some("ucsf")),
            false,
            overrides,
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

async fn remove(connection: &biorouter::crew::Connection) {
    let _ = crew().remove(&connection.id).await;
}

fn sent(host: &str, method: &str) -> Vec<Value> {
    received(host, method)
}

/// `params` for an action on `connection`, named by its ID, so a connection another test left
/// saved never makes the choice ambiguous.
fn on(connection: &biorouter::crew::Connection, mut params: Value) -> Value {
    params["connection"] = json!(connection.id);
    params
}

/// A folder of this test's fixture, by its real path: the transfer service opens every folder on
/// the way without following links, and macOS reaches its temporary folder through one.
fn local_folder(name: &str) -> PathBuf {
    let folder = fixture_root().join(name);
    fs::create_dir_all(&folder).unwrap();
    fs::canonicalize(folder).unwrap()
}

/// A person's own chat, already classified Private, as one that used a private model is.
async fn private_chat(name: &str) -> String {
    let session = chat(name).await;
    state()
        .await
        .session_manager()
        .update(&session)
        .raise_privacy(SessionClassification::Private, "turn:fixture")
        .apply()
        .await
        .unwrap();
    session
}

/// One change, allowed or denied with `answer`, that must park exactly one card on
/// `crew__change`. Answers the call.
async fn change(
    session: &str,
    capability: CallCapability,
    action: &str,
    params: Value,
    answer: &str,
) -> Called {
    let called = call(
        session,
        capability,
        PersonTool::Change,
        action,
        params,
        &[answer],
    )
    .await;
    assert_eq!(called.cards.len(), 1, "{action}: {:?}", called.cards);
    assert_eq!(called.cards[0].tool, "crew__change", "{action}");
    assert_eq!(called.cards[0].arguments["action"], action);
    names_where_it_goes(action, &called.cards[0]);
    called
}

/// Every change card says where it goes the same way (AO-12, "Approvals"): its heading names
/// the workspace and its server together, `lab on teams.example.test`, and its payload shows
/// both, so two workspaces that each have a `#general` in a same-named team are told apart.
fn names_where_it_goes(action: &str, card: &SeenCard) {
    let workspace = card.arguments["workspace"]
        .as_str()
        .unwrap_or_else(|| panic!("{action}: no workspace on the card: {card:?}"));
    let server = card.arguments["server"]
        .as_str()
        .unwrap_or_else(|| panic!("{action}: no server on the card: {card:?}"));
    let heading = card.prompt.lines().next().unwrap_or_default();
    assert!(
        heading.contains(&format!("{workspace} on {server}")),
        "{action}: {heading}"
    );
}

/// The server a saved connection reaches, as a card names it.
async fn server_of(connection: &biorouter::crew::Connection) -> String {
    biorouter::crew::server_label(&connection.ssh_target, connection.port).await
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

/// Every action of both tools runs in a chat now: `describe` says each is available, and none
/// answers that it is not available yet.
#[test]
#[serial]
fn every_action_runs_now() {
    run(async {
        let session = chat("describe").await;
        for action in READ_ACTIONS.iter().chain(CHANGE_ACTIONS) {
            let described = call(
                &session,
                public(),
                PersonTool::Read,
                "describe",
                json!({"action": action}),
                &[],
            )
            .await;
            assert_eq!(described.ok()["available"], true, "{action}");
            assert!(described.cards.is_empty());
        }
    });
}

/// Every team, channel and membership change parks one card naming its resolved targets, sends
/// nothing when denied, and sends exactly the IDs and guards it named when allowed: a deletion
/// with the name the card showed, a person with their username.
#[test]
#[serial]
fn team_and_channel_changes_send_what_the_card_showed() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0001";
        const HOST: &str = "teams.example.test";
        let connection = workspace(
            HOST,
            false,
            scenario(WORKSPACE, "private", snapshot("private", true)),
        )
        .await;
        let session = chat("teams and channels").await;
        let model = private_local().await;
        let server = server_of(&connection).await;

        let denied = change(
            &session,
            model,
            "teams_delete",
            on(&connection, json!({"team": "Analysis Lab"})),
            "deny",
        )
        .await;
        assert_eq!(
            denied.refusal(),
            "The person declined. Nothing was changed."
        );
        assert!(
            denied.cards[0].prompt.starts_with(&format!(
                "Delete the team Analysis Lab in lab on {server} for everyone; this cannot be undone"
            )),
            "{}",
            denied.cards[0].prompt
        );
        assert!(sent(HOST, "team.delete").is_empty());
        change(
            &session,
            model,
            "teams_delete",
            on(&connection, json!({"team": "Analysis Lab"})),
            "allow_once",
        )
        .await
        .ok();
        let frames = sent(HOST, "team.delete");
        assert_eq!(frames.len(), 1, "{frames:?}");
        assert_eq!(frames[0]["params"]["team_id"], "t-lab");
        assert_eq!(frames[0]["params"]["expected_name"], "analysis-lab");

        let created = change(
            &session,
            model,
            "teams_create",
            on(&connection, json!({"name": "analysis-2"})),
            "allow_once",
        )
        .await;
        assert_eq!(created.ok()["team_id"], "t-new");
        assert_eq!(sent(HOST, "team.create")[0]["params"]["name"], "analysis-2");

        change(
            &session,
            model,
            "teams_rename",
            on(
                &connection,
                json!({"team": "Analysis Lab", "name": "Analysis Two"}),
            ),
            "allow_once",
        )
        .await
        .ok();
        let frames = sent(HOST, "team.rename");
        assert_eq!(frames[0]["params"]["team_id"], "t-lab");
        assert_eq!(frames[0]["params"]["name"], "Analysis Two");

        let made = change(
            &session,
            model,
            "channels_create",
            on(
                &connection,
                json!({"name": "methods-2", "team": "Analysis Lab"}),
            ),
            "allow_once",
        )
        .await;
        assert!(
            made.cards[0].prompt.contains("It is restricted"),
            "{}",
            made.cards[0].prompt
        );
        let frames = sent(HOST, "channel.create");
        assert_eq!(frames[0]["params"]["team_id"], "t-lab");
        assert_eq!(frames[0]["params"]["classification"], "restricted");

        change(
            &session,
            model,
            "channels_rename",
            on(
                &connection,
                json!({"channel": "#methods", "name": "methods-3"}),
            ),
            "allow_once",
        )
        .await
        .ok();
        assert_eq!(
            sent(HOST, "channel.rename")[0]["params"]["channel_id"],
            METHODS
        );

        let archived = change(
            &session,
            model,
            "channels_archive",
            on(&connection, json!({"channel": "#methods"})),
            "allow_once",
        )
        .await;
        archived.ok();
        assert!(
            archived.cards[0].prompt.starts_with(&format!(
                "Archive #methods (Analysis Lab) in lab on {server} for everyone; this cannot be undone"
            )),
            "{}",
            archived.cards[0].prompt
        );
        assert_eq!(
            sent(HOST, "channel.archive")[0]["params"]["channel_id"],
            METHODS
        );

        let general = call(
            &session,
            model,
            PersonTool::Change,
            "channels_delete",
            on(&connection, json!({"channel": "#general"})),
            &[],
        )
        .await;
        assert!(
            general.refusal().contains("belongs to its team"),
            "{}",
            general.refusal()
        );
        assert!(general.cards.is_empty());
        change(
            &session,
            model,
            "channels_delete",
            on(&connection, json!({"channel": "#methods"})),
            "allow_once",
        )
        .await
        .ok();
        let frames = sent(HOST, "channel.delete");
        assert_eq!(frames.len(), 1, "{frames:?}");
        assert_eq!(frames[0]["params"]["channel_id"], METHODS);
        assert_eq!(frames[0]["params"]["expected_name"], "methods");

        change(
            &session,
            model,
            "channels_read",
            on(&connection, json!({"channel": "#general"})),
            "allow_once",
        )
        .await
        .ok();
        let frames = sent(HOST, "channel.read");
        assert_eq!(frames[0]["params"]["channel_id"], GENERAL);
        assert_eq!(frames[0]["params"]["sequence"], "m-2");

        let added = change(
            &session,
            model,
            "members_add",
            on(
                &connection,
                json!({"member": "@bob", "team": "Analysis Lab", "channels": ["methods"]}),
            ),
            "allow_once",
        )
        .await;
        added.ok();
        assert!(
            added.cards[0].prompt.contains("Bob Lee (@bob)"),
            "{}",
            added.cards[0].prompt
        );
        let frames = sent(HOST, "team.add_member");
        assert_eq!(frames[0]["params"]["principal_id"], "p-bob");
        assert_eq!(frames[0]["params"]["expected_username"], "bob");
        assert_eq!(frames[0]["params"]["channel_ids"], json!([METHODS]));

        change(
            &session,
            model,
            "members_add",
            on(
                &connection,
                json!({"member": "@bob", "channels": ["#methods", "#patients"]}),
            ),
            "allow_once",
        )
        .await
        .ok();
        let frames = sent(HOST, "channel.add_member");
        assert_eq!(frames.len(), 2, "{frames:?}");
        assert_eq!(frames[1]["params"]["channel_id"], PATIENTS);
        assert_ne!(
            frames[0]["params"]["idempotency_key"],
            frames[1]["params"]["idempotency_key"]
        );

        change(
            &session,
            model,
            "invites_send",
            on(
                &connection,
                json!({"member": "@bob", "channel": "#methods"}),
            ),
            "allow_once",
        )
        .await
        .ok();
        let frames = sent(HOST, "invitation.create");
        assert_eq!(frames[0]["params"]["kind"], "channel");
        assert_eq!(frames[0]["params"]["target_id"], METHODS);
        assert_eq!(frames[0]["params"]["expected_username"], "bob");

        let accepted = change(
            &session,
            model,
            "invites_accept",
            on(&connection, json!({})),
            "allow_once",
        )
        .await;
        assert!(
            accepted.cards[0].prompt.starts_with(&format!(
                "Accept the invitation to field-team in lab on {server}"
            )),
            "{}",
            accepted.cards[0].prompt
        );
        assert_eq!(
            sent(HOST, "invitation.accept")[0]["params"]["invitation_id"],
            "i-field"
        );

        change(
            &session,
            model,
            "ownership_offer",
            on(
                &connection,
                json!({"channel": "#methods", "successor": "@bob"}),
            ),
            "allow_once",
        )
        .await
        .ok();
        let frames = sent(HOST, "channel.transfer");
        assert_eq!(frames[0]["params"]["successor_id"], "p-bob");
        assert_eq!(frames[0]["params"]["expected_username"], "bob");

        change(
            &session,
            model,
            "ownership_accept",
            on(&connection, json!({"channel": "#methods"})),
            "allow_once",
        )
        .await
        .ok();
        assert_eq!(
            sent(HOST, "transfer.accept")[0]["params"]["channel_id"],
            METHODS
        );

        let removed = change(
            &session,
            model,
            "remove_member",
            on(
                &connection,
                json!({"channel": "#methods", "member": "@bob"}),
            ),
            "allow_once",
        )
        .await;
        removed.ok();
        assert!(
            removed.cards[0]
                .prompt
                .contains("for everyone; this cannot be undone"),
            "{}",
            removed.cards[0].prompt
        );
        let frames = sent(HOST, "membership.revoke");
        assert_eq!(frames[0]["params"]["principal_id"], "p-bob");
        assert_eq!(frames[0]["params"]["expected_username"], "bob");
        remove(&connection).await;
    });
}

/// In an open workspace, `channels_list` and `teams_list` show what the person may join, and a
/// public model is shown only public-safe channels: the protected one and the restricted one it
/// could join are counted, not named. Joining sends the joinable item's ID, and nothing raises
/// the chat.
#[test]
#[serial]
fn an_open_workspace_lists_and_joins_what_a_public_model_may_see() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0002";
        const HOST: &str = "open.example.test";
        let connection = workspace(
            HOST,
            true,
            scenario(WORKSPACE, "public", open_snapshot("public", true)),
        )
        .await;
        let session = chat("open workspace").await;

        let listed = call(
            &session,
            public(),
            PersonTool::Read,
            "channels_list",
            on(&connection, json!({})),
            &["allow_once"],
        )
        .await;
        assert_eq!(listed.cards.len(), 1, "the read consent");
        assert_eq!(listed.cards[0].tool, "crew__read");
        let answer = listed.ok();
        let ids = |list: &Value| -> Vec<String> {
            list.as_array()
                .unwrap()
                .iter()
                .map(|item| item["id"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(ids(&answer["channels"]), [GENERAL, METHODS]);
        assert_eq!(ids(&answer["joinable_channels"]), ["c-field-notes"]);
        assert_eq!(answer["withheld"]["channels"], 2);
        assert_eq!(answer["channels"][0]["unread"], 2);
        assert!(
            !answer.to_string().contains("patients") && !answer.to_string().contains("secret"),
            "{answer}"
        );

        let teams = call(
            &session,
            public(),
            PersonTool::Read,
            "teams_list",
            on(&connection, json!({})),
            &[],
        )
        .await;
        assert!(teams.cards.is_empty(), "the consent was given once already");
        assert_eq!(ids(&teams.ok()["teams"]), ["t-lab"]);
        assert_eq!(ids(&teams.ok()["joinable_teams"]), ["t-field"]);

        change(
            &session,
            public(),
            "teams_join",
            on(&connection, json!({"team": "Field Team"})),
            "allow_once",
        )
        .await
        .ok();
        assert_eq!(sent(HOST, "team.join")[0]["params"]["team_id"], "t-field");
        let joined = change(
            &session,
            public(),
            "channels_join",
            on(&connection, json!({"channel": "field-notes"})),
            "allow_once",
        )
        .await;
        let server = server_of(&connection).await;
        assert!(
            joined.cards[0].prompt.starts_with(&format!(
                "Join #field-notes (Analysis Lab) in lab on {server} as you"
            )),
            "{}",
            joined.cards[0].prompt
        );
        assert_eq!(
            sent(HOST, "channel.join")[0]["params"]["channel_id"],
            "c-field-notes"
        );
        assert_eq!(
            classification(&session).await,
            SessionClassification::Public
        );
        remove(&connection).await;
    });
}

/// A public model reading a Public workspace gets only messages and files the workspace does
/// not restrict, in channels that are public-safe, with what was withheld counted and the names
/// beside them narrowed; a protected channel is unknown to it. A private model reads all of it,
/// and the chat is raised Private before the answer returns.
#[test]
#[serial]
fn message_and_file_reads_withhold_what_a_public_model_may_not_read() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0003";
        const HOST: &str = "messages.example.test";
        let connection = workspace(
            HOST,
            true,
            scenario(WORKSPACE, "public", snapshot("public", true)),
        )
        .await;
        let session = chat("public reads").await;

        let read = call(
            &session,
            public(),
            PersonTool::Read,
            "history",
            on(&connection, json!({"channel": "#general"})),
            &["allow_once"],
        )
        .await;
        assert_eq!(read.cards.len(), 1, "the read consent");
        let page = read.ok();
        assert_eq!(page["messages"].as_array().unwrap().len(), 1, "{page}");
        assert_eq!(page["messages"][0]["id"], "m-1");
        assert_eq!(page["withheld"]["messages"], 1);
        assert!(page["channel_names"].get(PATIENTS).is_none(), "{page}");
        assert!(page["people"].get("p-alice").is_none(), "{page}");
        let frames = sent(HOST, "messages.history");
        assert_eq!(frames.last().unwrap()["params"]["latest"], true);

        let found = call(
            &session,
            public(),
            PersonTool::Read,
            "search",
            on(
                &connection,
                json!({"channel": "#general", "query": "notes"}),
            ),
            &[],
        )
        .await;
        assert!(found.cards.is_empty());
        assert_eq!(found.ok()["withheld"]["messages"], 1);
        assert_eq!(
            sent(HOST, "messages.search").last().unwrap()["params"]["query"],
            "notes"
        );

        let protected = call(
            &session,
            public(),
            PersonTool::Read,
            "history",
            on(&connection, json!({"channel": "#patients"})),
            &[],
        )
        .await;
        assert!(
            protected
                .refusal()
                .contains("Nothing you can see is named #patients."),
            "{}",
            protected.refusal()
        );

        let files = call(
            &session,
            public(),
            PersonTool::Read,
            "files_list",
            on(&connection, json!({"channel": "#general"})),
            &[],
        )
        .await;
        let listed = files.ok();
        assert_eq!(listed["files"].as_array().unwrap().len(), 1, "{listed}");
        assert_eq!(listed["files"][0]["attachment_id"], "a-open");
        assert_eq!(listed["withheld"]["files"], 1);

        let context = call(
            &session,
            public(),
            PersonTool::Read,
            "context_show",
            on(&connection, json!({})),
            &[],
        )
        .await;
        assert_eq!(context.ok()["context"]["text"], "Run jobs with sbatch.");
        assert_eq!(
            classification(&session).await,
            SessionClassification::Public,
            "a public read raises nothing"
        );

        let private = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "history",
            on(&connection, json!({"channel": "#general", "after": "m-0"})),
            &[],
        )
        .await;
        let page = private.ok();
        assert_eq!(page["messages"].as_array().unwrap().len(), 2, "{page}");
        assert_eq!(page["withheld"]["messages"], 0);
        assert_eq!(
            sent(HOST, "messages.history").last().unwrap()["params"]["after"],
            "m-0"
        );
        assert_eq!(
            classification(&session).await,
            SessionClassification::Private,
            "raised before the answer returned"
        );
        remove(&connection).await;
    });
}

/// A post from a Private chat is marked Private, its card shows the whole message and says when
/// it crosses into something Public; where the connection's privacy in force is Public, so the
/// post could not be kept private, it is refused before any card. A public chat's post is not
/// marked.
#[test]
#[serial]
fn a_private_chat_s_posts_are_marked_private() {
    run(async {
        const PRIVATE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0004";
        const PRIVATE_HOST: &str = "send-private.example.test";
        let connection = workspace(
            PRIVATE_HOST,
            false,
            scenario(PRIVATE, "private", snapshot("private", true)),
        )
        .await;
        let session = private_chat("private sender").await;
        let text = "line one\nline two";
        let denied = change(
            &session,
            private_local().await,
            "send",
            on(&connection, json!({"channel": "#general", "text": text})),
            "deny",
        )
        .await;
        denied.refusal();
        assert_eq!(denied.cards[0].arguments["text"], text);
        assert!(sent(PRIVATE_HOST, "message.post").is_empty());
        let posted = change(
            &session,
            private_local().await,
            "send",
            on(&connection, json!({"channel": "#general", "text": text})),
            "allow_once",
        )
        .await;
        assert_eq!(posted.ok()["message_id"], "m-new");
        // What a chat row needs to open the post in Crew (WI-16).
        assert_eq!(posted.ok()["connection_id"], connection.id.as_str());
        assert_eq!(posted.ok()["channel_id"], GENERAL);
        let prompt = &posted.cards[0].prompt;
        // The live run's card named neither: AO-12's summary names the workspace and server.
        let server = server_of(&connection).await;
        assert!(
            prompt.starts_with(&format!(
                "Post this in #general (Analysis Lab) in lab on {server} as you\n"
            )),
            "{prompt}"
        );
        assert_eq!(posted.cards[0].arguments["workspace"], "lab");
        assert_eq!(posted.cards[0].arguments["server"], server.as_str());
        assert!(prompt.contains("It is marked Private"), "{prompt}");
        assert!(
            prompt.contains("a public-safe channel: a private-to-public crossing"),
            "{prompt}"
        );
        let frames = sent(PRIVATE_HOST, "message.post");
        assert_eq!(frames.len(), 1, "{frames:?}");
        assert_eq!(frames[0]["params"]["body"], text);
        assert_eq!(frames[0]["params"]["personal_mode"], "private");
        assert_eq!(frames[0]["params"]["channel_id"], GENERAL);
        remove(&connection).await;

        // A Public workspace through a Private connection: still Private in force, so the post
        // goes marked Private, and the card says it crosses into a Public workspace.
        const MIXED: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0005";
        const MIXED_HOST: &str = "send-mixed.example.test";
        let mixed = workspace(
            MIXED_HOST,
            false,
            scenario(MIXED, "public", snapshot("public", true)),
        )
        .await;
        let crossed = change(
            &session,
            private_local().await,
            "send",
            on(&mixed, json!({"channel": "#patients", "text": "counts"})),
            "allow_once",
        )
        .await;
        crossed.ok();
        assert!(
            crossed.cards[0]
                .prompt
                .contains("this goes to a Public workspace: a private-to-public crossing"),
            "{}",
            crossed.cards[0].prompt
        );
        assert_eq!(
            sent(MIXED_HOST, "message.post")[0]["params"]["personal_mode"],
            "private"
        );
        remove(&mixed).await;

        // Public in force: a Private chat's post couldn't be kept private there.
        const PUBLIC: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0006";
        const PUBLIC_HOST: &str = "send-public.example.test";
        let open = workspace(
            PUBLIC_HOST,
            true,
            scenario(PUBLIC, "public", snapshot("public", true)),
        )
        .await;
        let refused = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "send",
            on(&open, json!({"channel": "#general", "text": "counts"})),
            &[],
        )
        .await;
        assert!(
            refused.refusal().contains("crew_mode_mismatch"),
            "{}",
            refused.refusal()
        );
        assert!(refused.cards.is_empty());
        assert!(sent(PUBLIC_HOST, "message.post").is_empty());
        let open_chat = chat("public sender").await;
        change(
            &open_chat,
            public(),
            "send",
            on(&open, json!({"channel": "#general", "text": "hello"})),
            "allow_once",
        )
        .await
        .ok();
        assert_eq!(
            sent(PUBLIC_HOST, "message.post")[0]["params"]["personal_mode"],
            "public",
            "the connection's own mode, as the daemon fills it in"
        );
        remove(&open).await;
    });
}

/// Sharing a file names its resolved path and size on the card, uploads it through the transfer
/// service (marked Private from a Private chat) and posts it; a reference to a server file is
/// created and posted the same way, and a path that is not absolute is refused before any card.
#[test]
#[serial]
fn files_are_shared_and_referenced_as_the_card_said() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0007";
        const HOST: &str = "files.example.test";
        let contents = b"a,b\n1,2\n";
        let sha256 = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(contents));
        let begun = json!({"result": {
            "id": BLOB, "channel_id": GENERAL, "size": contents.len(), "sha256": sha256,
            "offset": contents.len(), "complete": true, "restricted": true,
        }});
        let connection = workspace(
            HOST,
            false,
            scenario(WORKSPACE, "private", snapshot("private", true)).answer("blob.begin", begun),
        )
        .await;
        let folder = local_folder("share");
        let file = folder.join("notes.csv");
        fs::write(&file, contents).unwrap();
        let session = private_chat("sharer").await;
        let params = on(
            &connection,
            json!({"channel": "#general", "path": file.display().to_string(), "text": "the counts"}),
        );

        let denied = change(
            &session,
            private_local().await,
            "files_share",
            params.clone(),
            "deny",
        )
        .await;
        denied.refusal();
        let server = server_of(&connection).await;
        assert!(
            denied.cards[0].prompt.starts_with(&format!(
                "Share notes.csv (8 bytes) from this computer in #general (Analysis Lab) in lab on {server} as you"
            )),
            "{}",
            denied.cards[0].prompt
        );
        assert_eq!(
            denied.cards[0].arguments["path"],
            file.display().to_string()
        );
        assert_eq!(denied.cards[0].arguments["size_bytes"], 8);
        assert!(sent(HOST, "blob.begin").is_empty());

        let referenced = change(
            &session,
            private_local().await,
            "files_reference",
            on(
                &connection,
                json!({"channel": "#methods", "path": "/data/lab/results.csv", "text": "see this"}),
            ),
            "allow_once",
        )
        .await;
        assert_eq!(referenced.ok()["reference_id"], "r-new");
        assert_eq!(referenced.ok()["connection_id"], connection.id.as_str());
        assert_eq!(referenced.ok()["channel_id"], METHODS);
        let references = sent(HOST, "reference.create");
        assert_eq!(references[0]["params"]["path"], "/data/lab/results.csv");
        assert_eq!(references[0]["params"]["label"], "results.csv");
        let posts = sent(HOST, "message.post");
        assert_eq!(posts.len(), 1, "{posts:?}");
        assert_eq!(posts[0]["params"]["references"], json!(["r-new"]));
        assert_eq!(posts[0]["params"]["channel_id"], METHODS);

        let relative = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "files_reference",
            on(
                &connection,
                json!({"channel": "#methods", "path": "data/../results.csv"}),
            ),
            &[],
        )
        .await;
        assert!(
            relative.refusal().contains("full path on the server"),
            "{}",
            relative.refusal()
        );
        assert!(relative.cards.is_empty());

        // The transfer store opens every folder on its way without following links. A test
        // root reached through one (macOS's temporary folder is) can't hold it, so there the
        // upload itself is left to a run with BIOROUTER_PATH_ROOT on a real folder, and to CI.
        let store = biorouter::config::paths::Paths::state_dir();
        fs::create_dir_all(&store).unwrap();
        if fs::canonicalize(&store).ok().as_deref() != Some(store.as_path()) {
            eprintln!(
                "files_share's upload not run: {} is reached through a link",
                store.display()
            );
            remove(&connection).await;
            return;
        }

        let shared = change(
            &session,
            private_local().await,
            "files_share",
            params,
            "allow_once",
        )
        .await;
        let answer = shared.ok().clone();
        assert_eq!(answer["shared"], true, "{answer}");
        assert_eq!(answer["blob_id"], BLOB);
        assert_eq!(answer["connection_id"], connection.id.as_str());
        assert_eq!(answer["channel_id"], GENERAL);
        let begins = sent(HOST, "blob.begin");
        assert_eq!(begins.len(), 1, "{begins:?}");
        assert_eq!(begins[0]["params"]["personal_mode"], "private");
        assert_eq!(begins[0]["params"]["name"], "notes.csv");
        let posts = sent(HOST, "message.post");
        assert_eq!(posts.len(), 2, "{posts:?}");
        assert_eq!(posts[1]["params"]["attachments"], json!([BLOB]));
        assert_eq!(posts[1]["params"]["body"], "the counts");
        assert_eq!(posts[1]["params"]["personal_mode"], "private");

        // Once shared, the file's name is the workspace's: the status names it only to a chat
        // that may read the workspace, after the person's consent.
        let status = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "files_status",
            json!({"transfer_id": answer["transfer_id"]}),
            &["allow_once"],
        )
        .await;
        assert_eq!(status.ok()["state"], "completed");
        assert_eq!(status.ok()["name"], "notes.csv");
        assert_eq!(status.cards.len(), 1, "the read consent");
        assert_eq!(status.cards[0].tool, "crew__read");
        let to_public = call(
            &chat("public status").await,
            public(),
            PersonTool::Read,
            "files_status",
            json!({"transfer_id": answer["transfer_id"]}),
            &[],
        )
        .await;
        assert_eq!(to_public.ok()["state"], "completed");
        assert!(to_public.ok().get("name").is_none(), "{}", to_public.ok());
        assert!(to_public.cards.is_empty());
        let unknown = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "files_status",
            json!({"transfer_id": "0".repeat(32)}),
            &[],
        )
        .await;
        assert!(
            unknown.refusal().contains("No transfer with that ID"),
            "{}",
            unknown.refusal()
        );

        remove(&connection).await;
    });
}

/// A download names the file, its size, its channel and where it goes on its card, and a
/// restricted file is refused to a public model before any card. A name that does not resolve
/// in a workspace a public model may not read lists nothing from it.
#[test]
#[serial]
fn a_download_names_the_file_and_a_public_model_gets_no_restricted_one() {
    run(async {
        const PUBLIC: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0008";
        const PUBLIC_HOST: &str = "download-public.example.test";
        let open = workspace(
            PUBLIC_HOST,
            true,
            scenario(PUBLIC, "public", snapshot("public", true)).answer("blob.status", blob(true)),
        )
        .await;
        let session = chat("downloader").await;
        crew().grant_person_read(&session, &open.id).await.unwrap();
        let refused = call(
            &session,
            public(),
            PersonTool::Change,
            "files_download",
            on(&open, json!({"file": BLOB})),
            &[],
        )
        .await;
        assert!(
            refused
                .refusal()
                .starts_with("This chat can't download that file"),
            "{}",
            refused.refusal()
        );
        assert!(!refused.refusal().contains("notes.csv"));
        assert!(refused.cards.is_empty());
        remove(&open).await;

        const PRIVATE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0009";
        const PRIVATE_HOST: &str = "download-private.example.test";
        let connection = workspace(
            PRIVATE_HOST,
            false,
            scenario(PRIVATE, "private", snapshot("private", true)),
        )
        .await;
        crew()
            .grant_person_read(&session, &connection.id)
            .await
            .unwrap();
        let folder = local_folder("downloads");
        let denied = change(
            &session,
            private_local().await,
            "files_download",
            on(
                &connection,
                json!({"file": "notes.csv", "channel": "#general", "destination": folder.display().to_string()}),
            ),
            "deny",
        )
        .await;
        denied.refusal();
        let server = server_of(&connection).await;
        let expected = format!(
            "Download notes.csv (12 bytes) from #general in lab on {server} to {} on this computer",
            folder.join("notes.csv").display()
        );
        assert!(
            denied.cards[0].prompt.starts_with(&expected),
            "{}",
            denied.cards[0].prompt
        );
        assert!(sent(PRIVATE_HOST, "blob.read").is_empty());

        let hidden = call(
            &session,
            public(),
            PersonTool::Change,
            "channels_archive",
            on(&connection, json!({"channel": "#nothing"})),
            &[],
        )
        .await;
        assert!(
            hidden
                .refusal()
                .contains("doesn't name exactly one thing you can see in this workspace"),
            "{}",
            hidden.refusal()
        );
        assert!(
            !hidden.refusal().contains("general") && !hidden.refusal().contains("methods"),
            "{}",
            hidden.refusal()
        );
        assert!(hidden.cards.is_empty());
        remove(&connection).await;
    });
}

/// Tasks: the list and a watch of an unknown task read like any content; stopping an unknown
/// task parks nothing; asking for a task shows its channel, the channels it reads, its model,
/// its server access and the whole prompt on one card; and from a Private chat a task on a
/// public model is refused as a grant to that chat would be refused, before anything is sent to
/// the workspace.
#[test]
#[serial]
fn tasks_are_asked_for_on_one_card_and_a_private_chat_s_cannot_use_a_public_model() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d000a";
        const HOST: &str = "tasks.example.test";
        let connection = workspace(
            HOST,
            false,
            scenario(WORKSPACE, "private", snapshot("private", true)),
        )
        .await;
        let session = private_chat("tasks").await;
        crew()
            .grant_person_read(&session, &connection.id)
            .await
            .unwrap();

        let listed = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "tasks_list",
            on(&connection, json!({})),
            &[],
        )
        .await;
        assert_eq!(listed.ok()["tasks"], json!([]));
        assert!(listed.cards.is_empty());
        let watched = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "tasks_watch",
            on(
                &connection,
                json!({"task": "run-missing", "wait_seconds": 0}),
            ),
            &[],
        )
        .await;
        assert!(
            watched.refusal().contains("No task of this computer"),
            "{}",
            watched.refusal()
        );
        let stopped = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "tasks_cancel",
            on(&connection, json!({"task": "run-missing"})),
            &[],
        )
        .await;
        assert!(
            stopped.refusal().contains("No task of this computer"),
            "{}",
            stopped.refusal()
        );
        assert!(stopped.cards.is_empty());

        let asked = change(
            &session,
            private_local().await,
            "tasks_ask",
            on(
                &connection,
                json!({
                    "channel": "#general",
                    "prompt": "Summarize this week's runs.",
                    "context_channels": ["#methods"],
                    "provider": "openai",
                    "model": "gpt-4o",
                }),
            ),
            "deny",
        )
        .await;
        asked.refusal();
        let prompt = &asked.cards[0].prompt;
        let server = server_of(&connection).await;
        assert!(
            prompt.starts_with(&format!(
                "Start an agent task in #general (Analysis Lab) in lab on {server}, on gpt-4o (openai), with its own Crew access"
            )),
            "{prompt}"
        );
        assert!(
            prompt.contains("It may also read #methods in Analysis Lab."),
            "{prompt}"
        );
        assert!(prompt.contains("It can't work on"), "{prompt}");
        assert!(
            prompt.contains("This chat is Private, so the task is too"),
            "{prompt}"
        );
        assert_eq!(
            asked.cards[0].arguments["prompt"],
            "Summarize this week's runs."
        );
        assert!(sent(HOST, "run.create").is_empty());
        remove(&connection).await;

        const PUBLIC: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d000b";
        const PUBLIC_HOST: &str = "tasks-public.example.test";
        let open = workspace(
            PUBLIC_HOST,
            true,
            scenario(PUBLIC, "public", snapshot("public", false)),
        )
        .await;
        let refused = change(
            &session,
            private_local().await,
            "tasks_ask",
            on(
                &open,
                json!({
                    "channel": "#general",
                    "prompt": "Summarize.",
                    "provider": "openai",
                    "model": "gpt-4o",
                }),
            ),
            "allow_once",
        )
        .await;
        assert!(
            refused.refusal().contains(
                "This chat has used a private model, so a public model can't continue it"
            ),
            "{}",
            refused.refusal()
        );
        assert!(
            refused.refusal().contains("crew_public_model_refused"),
            "{}",
            refused.refusal()
        );
        assert!(sent(PUBLIC_HOST, "run.create").is_empty());
        remove(&open).await;
    });
}

/// A task asked through a connection with full access runs as the account, so its card says
/// what every full-access card says (W1) right after what the task can do on the server, and,
/// since Alice hosts the workspace, the host's sentence (W2) after that. Nothing is sent when
/// the person declines.
#[test]
#[serial]
fn a_task_with_full_server_access_says_what_full_access_is() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d00f1";
        const HOST: &str = "tasks-full.example.test";
        let connection = workspace(
            HOST,
            false,
            scenario(WORKSPACE, "private", snapshot("private", false)),
        )
        .await;
        // Full access, saved as `connections set-access full` saves it.
        let whole = serde_json::to_value(&connection).unwrap();
        let mut fields = serde_json::Map::new();
        for key in [
            "name",
            "ssh_target",
            "port",
            "identity_file",
            "proxy_jump",
            "socket_path",
            "owner_uid",
            "workspace_id",
            "workspace_public_key",
            "cluster_connection_id",
            "mode",
            "institution_id",
        ] {
            if let Some(value) = whole.get(key) {
                fields.insert(key.into(), value.clone());
            }
        }
        fields.insert("remote_root".into(), json!("/home/alice/crew-work/lab"));
        fields.insert("remote_execution".into(), json!(true));
        fields.insert("remote_access".into(), json!("full"));
        let full = crew()
            .update(
                &connection.id,
                serde_json::from_value(Value::Object(fields)).unwrap(),
            )
            .await
            .expect("full access saves");
        assert_eq!(full.remote_access, biorouter::crew::RemoteAccess::Full);
        let session = private_chat("tasks full").await;
        crew()
            .grant_person_read(&session, &connection.id)
            .await
            .unwrap();

        let asked = change(
            &session,
            private_local().await,
            "tasks_ask",
            on(
                &connection,
                json!({
                    "channel": "#general",
                    "prompt": "Summarize this week's runs.",
                    "provider": "openai",
                    "model": "gpt-4o",
                }),
            ),
            "deny",
        )
        .await;
        asked.refusal();
        let lines: Vec<&str> = asked.cards[0].prompt.lines().collect();
        let server = server_of(&connection).await;
        let at = lines
            .iter()
            .position(|line| line.starts_with("With a private model it can run any command"))
            .unwrap_or_else(|| panic!("no access line: {lines:?}"));
        assert_eq!(
            lines[at],
            format!(
                "With a private model it can run any command as you on {server}, and read and change any file your account can. Crew treats anything running as your account as you, so your agent can also add a computer to your account and post in Crew as you, with no agent mark."
            ),
            "{lines:?}"
        );
        assert_eq!(
            lines.get(at + 1).copied(),
            Some("You host lab from this account, so your agent can also read the workspace's own files on the server, including channels you're not in. It can also act as the host of lab."),
            "{lines:?}"
        );
        assert!(sent(HOST, "run.create").is_empty());
        remove(&connection).await;
    });
}

/// Refusing to delete a team's `#general` names the team only to a call that may be shown names
/// from the workspace: a public model in a Private workspace is told what to do without it.
#[test]
#[serial]
fn a_refused_delete_names_no_team_to_a_model_that_may_not_read() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d000c";
        const HOST: &str = "general-delete.example.test";
        let connection = workspace(
            HOST,
            false,
            scenario(WORKSPACE, "private", snapshot("private", true)),
        )
        .await;
        let session = chat("delete general").await;
        let refused = call(
            &session,
            public(),
            PersonTool::Change,
            "channels_delete",
            on(&connection, json!({"channel": "#general"})),
            &[],
        )
        .await;
        assert!(
            refused
                .refusal()
                .starts_with("That channel belongs to its team. Delete the team instead"),
            "{}",
            refused.refusal()
        );
        assert!(
            !refused.refusal().contains("Analysis Lab"),
            "{}",
            refused.refusal()
        );
        assert!(refused.cards.is_empty());
        assert!(sent(HOST, "channel.delete").is_empty());
        remove(&connection).await;
    });
}

/// A name that matches more than one thing lists the workspace's names only once the chat may
/// read them: a private model is not shown the candidates before the person let the chat read
/// the workspace, and once they have, the names return only as content does, after the chat is
/// raised Private, on a change and on a read alike.
#[test]
#[serial]
fn candidate_names_wait_for_the_read_consent_and_the_raise() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d000d";
        const HOST: &str = "candidates.example.test";
        // A second team with its own #methods, so `#methods` alone names two channels.
        let mut twins = snapshot("private", true);
        twins["teams"].as_array_mut().unwrap().push(json!({
            "id": "t-field", "name": "field-team", "display_name": "Field Team",
            "general_channel_id": "c-field-general", "members": ["p-alice"],
            "name_conflict": false,
        }));
        twins["channels"].as_array_mut().unwrap().push(json!({
            "id": "c-field-methods", "team_id": "t-field", "name": "methods",
            "classification": "restricted", "name_conflict": false,
        }));
        twins["totals"] = json!({"teams": 2, "channels": 4});
        let connection = workspace(HOST, false, scenario(WORKSPACE, "private", twins)).await;
        let session = chat("candidates").await;
        let post = || {
            on(
                &connection,
                json!({"channel": "#methods", "text": "the counts"}),
            )
        };
        let before = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "send",
            post(),
            &[],
        )
        .await;
        assert!(
            before
                .refusal()
                .contains("doesn't name exactly one thing you can see in this workspace"),
            "{}",
            before.refusal()
        );
        assert!(
            !before.refusal().contains("Field Team") && !before.refusal().contains("Analysis Lab"),
            "{}",
            before.refusal()
        );
        assert!(before.cards.is_empty());
        assert_eq!(
            classification(&session).await,
            SessionClassification::Public
        );

        crew()
            .grant_person_read(&session, &connection.id)
            .await
            .unwrap();
        let after = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "send",
            post(),
            &[],
        )
        .await;
        assert!(
            after.refusal().contains("Field Team"),
            "{}",
            after.refusal()
        );
        assert_eq!(
            classification(&session).await,
            SessionClassification::Private,
            "the names returned before the chat was raised"
        );
        assert!(sent(HOST, "message.post").is_empty());

        let reader = chat("candidates, reading").await;
        crew()
            .grant_person_read(&reader, &connection.id)
            .await
            .unwrap();
        let read = call(
            &reader,
            private_local().await,
            PersonTool::Read,
            "history",
            on(&connection, json!({"channel": "#methods"})),
            &[],
        )
        .await;
        assert!(read.refusal().contains("Field Team"), "{}", read.refusal());
        assert_eq!(
            classification(&reader).await,
            SessionClassification::Private,
            "a read's refusal named channels before the chat was raised"
        );
        assert!(sent(HOST, "messages.history").is_empty());
        remove(&connection).await;
    });
}

/// A public model reads only channels the snapshot lists: in a workspace whose channel list was
/// cut short, a channel given by an ID it does not list may be protected, so it is unknown to
/// the call and nothing is asked of the workspace.
#[test]
#[serial]
fn a_public_model_reads_no_channel_beyond_a_list_cut_short() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d000e";
        const HOST: &str = "cut-short.example.test";
        let mut partial = snapshot("public", true);
        partial["totals"]["channels"] = json!(5);
        let open = workspace(HOST, true, scenario(WORKSPACE, "public", partial)).await;
        let session = chat("cut short").await;
        crew().grant_person_read(&session, &open.id).await.unwrap();
        let unlisted = "6b1f0c2e-1d4a-4c5e-9a0b-0000000c4a11";
        let refused = call(
            &session,
            public(),
            PersonTool::Read,
            "history",
            on(&open, json!({"channel": unlisted})),
            &[],
        )
        .await;
        assert!(
            refused.refusal().contains("Nothing you can see is named"),
            "{}",
            refused.refusal()
        );
        assert!(refused.cards.is_empty());
        assert!(
            sent(HOST, "messages.history").is_empty(),
            "an unlisted channel's history was asked for a public model"
        );
        remove(&open).await;
    });
}

/// A share uploads the file its card showed or nothing: a file rewritten while the card waited
/// is refused after the approval, and nothing reaches the workspace.
#[test]
#[serial]
fn a_share_refuses_a_file_changed_while_its_card_waited() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d000f";
        const HOST: &str = "share-changed.example.test";
        let connection = workspace(
            HOST,
            false,
            scenario(WORKSPACE, "private", snapshot("private", true)),
        )
        .await;
        let folder = local_folder("share-changed");
        let file = folder.join("plot.csv");
        fs::write(&file, b"a,b\n").unwrap();
        let session = private_chat("share changed").await;
        let rewritten = async {
            fs::write(&file, b"another file entirely,\nwith other rows\n").unwrap();
        };
        let shared = call_meanwhile(
            &session,
            private_local().await,
            PersonTool::Change,
            "files_share",
            on(
                &connection,
                json!({"channel": "#general", "path": file.display().to_string()}),
            ),
            "allow_once",
            rewritten,
        )
        .await;
        assert_eq!(shared.cards.len(), 1);
        assert!(
            shared.cards[0]
                .prompt
                .starts_with("Share plot.csv (4 bytes)"),
            "{}",
            shared.cards[0].prompt
        );
        assert!(
            shared.refusal().contains(
                "plot.csv changed on this computer while this waited for your approval, so nothing was shared"
            ),
            "{}",
            shared.refusal()
        );
        assert!(sent(HOST, "blob.begin").is_empty());
        assert!(sent(HOST, "message.post").is_empty());
        remove(&connection).await;
    });
}

/// AO-12, "Privacy", by a grant's own origin rule: a chat that already holds content another
/// institution owns reads no workspace of a different institution, so two institutions' private
/// Crew content never meet in one chat. It is refused before the person is asked and before any
/// content is fetched, and it names nothing from the workspace. A chat whose content is the
/// workspace's own institution's reads it.
#[test]
#[serial]
fn a_chat_holding_another_institution_s_content_reads_no_workspace_of_a_different_one() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0010";
        const HOST: &str = "origin.example.test";
        // A ucsf workspace, through a Private ucsf connection.
        let connection = workspace(
            HOST,
            false,
            scenario(WORKSPACE, "private", snapshot("private", true)),
        )
        .await;
        let app = state().await;
        let sessions = app.session_manager();
        let stanford_chat = chat("origin stanford").await;
        sessions
            .record_required_session_affiliation(
                &stanford_chat,
                biorouter::privacy::affiliation::InstitutionId::new("stanford"),
            )
            .await
            .unwrap();
        let refused = call(
            &stanford_chat,
            private_local().await,
            PersonTool::Read,
            "history",
            on(&connection, json!({"channel": "#general"})),
            &["allow_once"],
        )
        .await;
        assert!(refused.cards.is_empty(), "{:?}", refused.cards);
        assert!(
            refused.refusal().contains("crew_institution_mismatch"),
            "{}",
            refused.refusal()
        );
        assert!(
            refused.refusal().contains("another institution"),
            "{}",
            refused.refusal()
        );
        for name in ["lab", "general", "Analysis"] {
            assert!(!refused.refusal().contains(name), "{}", refused.refusal());
        }
        assert!(sent(HOST, "messages.history").is_empty());
        assert!(!crew().has_person_read(&stanford_chat, &connection.id).await);

        let ucsf_chat = chat("origin ucsf").await;
        sessions
            .record_required_session_affiliation(
                &ucsf_chat,
                biorouter::privacy::affiliation::InstitutionId::new("ucsf"),
            )
            .await
            .unwrap();
        let read = call(
            &ucsf_chat,
            private_local().await,
            PersonTool::Read,
            "history",
            on(&connection, json!({"channel": "#general"})),
            &["allow_once"],
        )
        .await;
        assert_eq!(read.cards.len(), 1, "the read consent");
        assert_eq!(read.ok()["channel_id"], GENERAL);
        remove(&connection).await;
    });
}

/// The connection a workspace's invitation was saved as on this computer, if any.
async fn saved_for(workspace_id: &str) -> Option<biorouter::crew::Connection> {
    crew()
        .list()
        .await
        .into_iter()
        .find(|connection| connection.workspace_id == workspace_id)
}

/// A join from an invitation with the Join screen's Advanced route (WI-5): its one card lists
/// every choice (the login and route it signs in by, the key file, the work folder and agent
/// access, full access in OW-6's words, and the host's sentence only for the account that hosts
/// the workspace), and nothing is saved when it is denied. Choices that can't apply are refused
/// before any card.
#[test]
#[serial]
fn a_join_s_card_lists_every_choice_of_the_advanced_route() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0051";
        const HOST: &str = "join-choices.example.test";
        const JUMP: &str = "jump-checked.example.test";
        stage(
            HOST,
            &scenario(WORKSPACE, "private", snapshot("private", true)),
        );
        let key = local_folder("keys").join("id_lab").display().to_string();
        let text = invitation(WORKSPACE, HOST, Some("ucsf"));
        let session = chat("join choices").await;

        let denied = call(
            &session,
            public(),
            PersonTool::Change,
            "join",
            json!({
                "invitation": text, "username": "bob", "port": 2222, "proxy_jump": JUMP,
                "identity_file": key, "agent_access": "full", "folder": "/home/bob/crew-work/lab",
            }),
            &["deny"],
        )
        .await;
        denied.refusal();
        assert_eq!(denied.cards.len(), 1, "{:?}", denied.cards);
        let card = &denied.cards[0];
        assert_eq!(card.tool, "crew__change");
        assert_eq!(card.arguments["action"], "join");
        for (name, value) in [
            ("port", json!(2222)),
            ("proxy_jump", json!(JUMP)),
            ("identity_file", json!(key)),
            ("folder", json!("/home/bob/crew-work/lab")),
            ("agent_access", json!("full")),
            ("invitation", json!(text)),
        ] {
            assert_eq!(card.arguments[name], value, "{name}: {card:?}");
        }
        let lines: Vec<&str> = card.prompt.lines().collect();
        let route = format!(
            "It signs in as bob@{HOST} on port 2222 through {JUMP}, with the key file {key}."
        );
        assert!(lines.contains(&route.as_str()), "{lines:?}");
        let server = card.arguments["server"]
            .as_str()
            .expect("the card's server");
        let full = format!(
            "Agents get full access: they can run any command as you on {server}, read and change any file your account can, and use the network. Crew treats anything running as your account as you, so your agent can also add a computer to your account and post in Crew as you, with no agent mark."
        );
        assert!(lines.contains(&full.as_str()), "{lines:?}");
        assert!(
            lines.contains(&"Their work folder is /home/bob/crew-work/lab. To set this up, it installs or updates Crew in your account there first."),
            "{lines:?}"
        );
        assert!(
            !card.prompt.contains("You host"),
            "bob doesn't host lab: {lines:?}"
        );
        assert!(saved_for(WORKSPACE).await.is_none(), "a denied join saved");

        // Alice, invited to her own workspace, hosts it: full access says so too. No route was
        // chosen, so none is said.
        let hosted = call(
            &session,
            public(),
            PersonTool::Change,
            "join",
            json!({"invitation": text, "agent_access": "full"}),
            &["deny"],
        )
        .await;
        let prompt = &hosted.cards[0].prompt;
        assert!(
            prompt.contains("You host lab from this account, so your agent can also read the workspace's own files on the server, including channels you're not in. It can also act as the host of lab."),
            "{prompt}"
        );
        assert!(!prompt.contains("It signs in as"), "{prompt}");
        assert!(
            prompt.contains("To set this up, it installs or updates Crew in your account there first, and makes their work folder."),
            "{prompt}"
        );

        // A login from the person's SSH settings names no user, so the account is the one
        // `ssh -G` says it signs in as: the host's alias gets the host's sentence, another's
        // doesn't.
        for (alias, user, hosts) in [("lab-alice", "alice", true), ("lab-bob", "bob", false)] {
            let mut resolved = scenario(WORKSPACE, "private", snapshot("private", true));
            resolved.resolves_to = Some(HOST.into());
            resolved.user = Some(user.into());
            stage(alias, &resolved);
            let card = call(
                &session,
                public(),
                PersonTool::Change,
                "join",
                json!({"invitation": text, "ssh_target": alias, "agent_access": "full"}),
                &["deny"],
            )
            .await;
            assert_eq!(card.cards.len(), 1, "{alias}: {:?}", card.cards);
            let prompt = &card.cards[0].prompt;
            assert_eq!(
                prompt.contains("You host lab from this account"),
                hosts,
                "{alias}: {prompt}"
            );
        }
        assert!(saved_for(WORKSPACE).await.is_none(), "a denied join saved");

        for (params, refusal) in [
            (
                json!({"invitation": text, "folder": "/home/bob/crew-work/lab"}),
                "folder is for agent_access folder or full",
            ),
            (
                json!({"invitation": text, "agent_access": "off", "folder": "/home/bob/x"}),
                "folder is for agent_access folder or full",
            ),
            (
                json!({"invitation": text, "port": 70000}),
                "Choose a port between 1 and 65535.",
            ),
            (
                json!({"invitation": text, "agent_access": "folder", "folder": "~/crew-work"}),
                "Give the work folder's full path on the server",
            ),
            (
                json!({"invitation": text, "identity_file": "keys/id_lab"}),
                "Give the SSH key file's full path on this computer.",
            ),
            (
                json!({"connection": "lab", "port": 2222}),
                "Those settings are for joining from an invitation.",
            ),
        ] {
            let refused = call(&session, public(), PersonTool::Change, "join", params, &[]).await;
            assert!(refused.cards.is_empty(), "{refusal}: {:?}", refused.cards);
            assert!(
                refused.refusal().contains(refusal),
                "{refusal}: {}",
                refused.refusal()
            );
        }
        assert!(saved_for(WORKSPACE).await.is_none());
    });
}

/// Allowed, a join from an invitation saves the connection with the port and jump host it was
/// given before it connects, and every hop still answers to `ssh_policy`: a jump host whose own
/// settings would take an unknown host key (no `StrictHostKeyChecking yes`) is refused, with the
/// connection kept as saved and nothing sent to the server. The same invitation can't then take
/// route choices over that saved connection. Through a jump host that checks host keys, the join
/// connects and joins, and answers the connection's ID.
#[test]
#[serial]
fn a_join_saves_its_route_before_connecting_and_every_hop_is_checked() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0052";
        const HOST: &str = "join-route.example.test";
        const JUMP: &str = "jump-route-checked.example.test";
        const LAX_JUMP: &str = "jump-route-lax.example.test";
        stage(
            HOST,
            &scenario(WORKSPACE, "private", snapshot("private", true)).answer(
                "account.status",
                json!({"result": {"role": "member", "host_username": "alice"}}),
            ),
        );
        let mut lax = Scenario::new(WORKSPACE, &ACCOUNT_CAPABILITIES);
        lax.host_key_checking = Some("no".into());
        stage(LAX_JUMP, &lax);
        let text = invitation(WORKSPACE, HOST, Some("ucsf"));
        let session = chat("join route").await;

        let through_lax = call(
            &session,
            public(),
            PersonTool::Change,
            "join",
            json!({"invitation": text, "username": "bob", "port": 2222, "proxy_jump": LAX_JUMP}),
            &["allow_once"],
        )
        .await;
        assert_eq!(through_lax.cards.len(), 1, "{:?}", through_lax.cards);
        // The model is told what the policy refused and what to change, not "try again".
        let told = through_lax.refusal();
        assert!(
            told.contains("StrictHostKeyChecking yes")
                && told.contains(&format!("{LAX_JUMP} (the jump host"))
                && told.contains("(code: crew_ssh_settings_refused)"),
            "{told}"
        );
        let kept = saved_for(WORKSPACE)
            .await
            .expect("the join saved the connection before connecting");
        assert_eq!(kept.ssh_target, format!("bob@{HOST}"));
        assert_eq!(kept.port, Some(2222));
        assert_eq!(kept.proxy_jump.as_deref(), Some(LAX_JUMP));
        assert_ne!(kept.status, "connected");
        assert!(
            received(HOST, "hello").is_empty(),
            "nothing may reach the server through that jump host"
        );
        // What refused it: ssh_policy, on the jump hop of the route the join saved.
        let policy = crew()
            .connect(&kept.id)
            .await
            .expect_err("a jump host that takes unknown host keys");
        let policy = format!("{policy:#}");
        assert!(
            policy.contains("StrictHostKeyChecking yes")
                && policy.contains(&format!("{LAX_JUMP} (the jump host")),
            "{policy}"
        );

        let again = call(
            &session,
            public(),
            PersonTool::Change,
            "join",
            json!({"invitation": text, "username": "bob", "port": 2222, "proxy_jump": JUMP}),
            &[],
        )
        .await;
        assert!(again.cards.is_empty(), "{:?}", again.cards);
        assert!(
            again.refusal().contains(
                "This computer already has a connection to lab, and joining uses it as it is."
            ),
            "{}",
            again.refusal()
        );
        remove(&kept).await;

        let joined = call(
            &session,
            public(),
            PersonTool::Change,
            "join",
            json!({"invitation": text, "username": "bob", "port": 2222, "proxy_jump": JUMP, "agent_access": "off"}),
            &["allow_once"],
        )
        .await;
        assert_eq!(joined.cards.len(), 1, "{:?}", joined.cards);
        let answer = joined.ok();
        assert_eq!(answer["joined"], true, "{answer}");
        let connection = saved_for(WORKSPACE).await.expect("the joined connection");
        assert_eq!(answer["connection_id"], connection.id.as_str());
        assert_eq!(connection.port, Some(2222));
        assert_eq!(connection.proxy_jump.as_deref(), Some(JUMP));
        assert!(
            !received(HOST, "hello").is_empty(),
            "it connected through the jump host"
        );
        assert!(
            joined.cards[0].prompt.contains("Agents can't work on"),
            "{}",
            joined.cards[0].prompt
        );
        remove(&connection).await;
    });
}

/// One `send` with two files from this computer and two references to server files parks one
/// card showing everything it posts, marked Private from a Private chat and with the crossing
/// into a public-safe channel said, sends nothing when denied, and posts exactly one message
/// carrying them all when allowed (WI-5). What can't be sent is refused before any card.
#[test]
#[serial]
fn one_send_posts_several_files_and_references_in_one_message() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0053";
        const HOST: &str = "send-items.example.test";
        const OTHER_BLOB: &str = "6b1f0c2e-1d4a-4c5e-9a0b-00000000b10c";
        let files: [(&str, &[u8], &str); 2] = [
            ("notes.csv", b"a,b\n1,2\n", BLOB),
            ("counts.csv", b"x\n3\n", OTHER_BLOB),
        ];
        let folder = local_folder("send-items");
        let mut scenario = scenario(WORKSPACE, "private", snapshot("private", true))
            .answer(
                "reference.create#a.csv",
                json!({"result": {"id": "r-a", "restricted": true}}),
            )
            .answer(
                "reference.create#b.csv",
                json!({"result": {"id": "r-b", "restricted": true}}),
            );
        let mut paths = Vec::new();
        for (name, contents, blob) in files {
            let path = folder.join(name);
            fs::write(&path, contents).unwrap();
            paths.push(path.display().to_string());
            let sha256 = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(contents));
            scenario = scenario.answer(
                &format!("blob.begin#{name}"),
                json!({"result": {
                    "id": blob, "channel_id": GENERAL, "size": contents.len(), "sha256": sha256,
                    "offset": contents.len(), "complete": true, "restricted": true,
                }}),
            );
        }
        let connection = workspace(HOST, false, scenario).await;
        let session = private_chat("send items").await;
        let params = on(
            &connection,
            json!({"channel": "#general", "text": "two files", "attachments": paths}),
        );

        let denied = change(
            &session,
            private_local().await,
            "send",
            params.clone(),
            "deny",
        )
        .await;
        denied.refusal();
        let card = &denied.cards[0];
        let server = server_of(&connection).await;
        assert!(
            card.prompt.starts_with(&format!(
                "Post this with 2 files from this computer in #general (Analysis Lab) in lab on {server} as you\n"
            )),
            "{}",
            card.prompt
        );
        let lines: Vec<&str> = card.prompt.lines().collect();
        for (line, path) in [
            ("notes.csv (8 bytes)", &paths[0]),
            ("counts.csv (4 bytes)", &paths[1]),
        ] {
            let shown = format!("{line}, from {path}.");
            assert!(lines.contains(&shown.as_str()), "{lines:?}");
        }
        assert!(
            lines.contains(
                &"The files and the post are marked Private, so public models never read them."
            ),
            "{lines:?}"
        );
        assert!(
            card.prompt
                .contains("a public-safe channel: a private-to-public crossing. Everything it sends is shown here."),
            "{}",
            card.prompt
        );
        assert_eq!(card.arguments["text"], "two files");
        assert_eq!(
            card.arguments["attachments"],
            json!([
                {"path": paths[0], "size_bytes": 8},
                {"path": paths[1], "size_bytes": 4},
            ])
        );
        assert!(sent(HOST, "blob.begin").is_empty());
        assert!(sent(HOST, "message.post").is_empty());

        let too_many: Vec<String> = (0..11).map(|n| format!("/data/lab/{n}.csv")).collect();
        for (params, refusal) in [
            (
                json!({"channel": "#general"}),
                "send needs text, attachments or references.",
            ),
            (
                json!({"channel": "#general", "attachments": ["notes.csv"]}),
                "Give the file's full path on this computer",
            ),
            (
                json!({"channel": "#general", "attachments": [paths[0], paths[0]]}),
                "is named twice",
            ),
            (
                json!({"channel": "#general", "attachments": [folder.display().to_string()]}),
                "is not a file",
            ),
            (
                json!({"channel": "#general", "references": too_many}),
                "One message carries at most 10 references.",
            ),
            (
                json!({"channel": "#general", "references": ["/data/../etc/passwd"]}),
                "Give the file's full path on the server",
            ),
        ] {
            let refused = call(
                &session,
                private_local().await,
                PersonTool::Change,
                "send",
                on(&connection, params),
                &[],
            )
            .await;
            assert!(refused.cards.is_empty(), "{refusal}: {:?}", refused.cards);
            assert!(
                refused.refusal().contains(refusal),
                "{refusal}: {}",
                refused.refusal()
            );
        }

        // References alone: created, then posted in one message.
        let referenced = change(
            &session,
            private_local().await,
            "send",
            on(
                &connection,
                json!({"channel": "#methods", "references": ["/data/lab/a.csv", "/data/lab/b.csv"]}),
            ),
            "allow_once",
        )
        .await;
        let prompt = &referenced.cards[0].prompt;
        assert!(
            prompt
                .starts_with("Post 2 references to files on the server in #methods (Analysis Lab)"),
            "{prompt}"
        );
        assert!(
            prompt.contains("/data/lab/a.csv on the server."),
            "{prompt}"
        );
        assert_eq!(
            referenced.cards[0].arguments["references"],
            json!(["/data/lab/a.csv", "/data/lab/b.csv"])
        );
        let answer = referenced.ok();
        assert_eq!(answer["reference_ids"], json!(["r-a", "r-b"]), "{answer}");
        assert_eq!(answer["connection_id"], connection.id.as_str());
        assert_eq!(answer["channel_id"], METHODS);
        let posts = sent(HOST, "message.post");
        assert_eq!(posts.len(), 1, "{posts:?}");
        assert_eq!(posts[0]["params"]["references"], json!(["r-a", "r-b"]));
        assert_eq!(posts[0]["params"]["body"], "");

        // The transfer store opens every folder on its way without following links: as for
        // files_share, the upload runs only where the test's state folder is reached directly
        // (BIOROUTER_PATH_ROOT on a real folder, and CI).
        let store = biorouter::config::paths::Paths::state_dir();
        fs::create_dir_all(&store).unwrap();
        if fs::canonicalize(&store).ok().as_deref() != Some(store.as_path()) {
            eprintln!(
                "send's uploads not run: {} is reached through a link",
                store.display()
            );
            remove(&connection).await;
            return;
        }
        let shared = change(
            &session,
            private_local().await,
            "send",
            params,
            "allow_once",
        )
        .await;
        let answer = shared.ok().clone();
        assert_eq!(answer["shared"], true, "{answer}");
        assert_eq!(answer["connection_id"], connection.id.as_str());
        assert_eq!(answer["channel_id"], GENERAL);
        assert_eq!(answer["message_id"], "m-new");
        let begins = sent(HOST, "blob.begin");
        assert_eq!(begins.len(), 2, "{begins:?}");
        let posts = sent(HOST, "message.post");
        assert_eq!(
            posts.len(),
            2,
            "one post for the references, one for the files: {posts:?}"
        );
        assert_eq!(posts[1]["params"]["attachments"], json!([BLOB, OTHER_BLOB]));
        assert_eq!(posts[1]["params"]["body"], "two files");
        assert_eq!(posts[1]["params"]["personal_mode"], "private");
        assert_eq!(
            answer["attachments"],
            json!([
                {"name": "notes.csv", "blob_id": BLOB, "transfer_id": answer["transfer_ids"][0]},
                {"name": "counts.csv", "blob_id": OTHER_BLOB, "transfer_id": answer["transfer_ids"][1]},
            ])
        );
        remove(&connection).await;
    });
}

/// The saved-connection changes answer the connection they changed, as a chat row needs it to
/// open the connection in Crew (WI-16): connecting, and changing the saved connection.
#[test]
#[serial]
fn connection_changes_answer_the_connection_s_id() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0054";
        const HOST: &str = "connection-ids.example.test";
        let connection = workspace(
            HOST,
            false,
            scenario(WORKSPACE, "private", snapshot("private", true)),
        )
        .await;
        let session = chat("connection ids").await;
        let renamed = call(
            &session,
            public(),
            PersonTool::Change,
            "connections_update",
            json!({"connection": connection.id, "name": "lab-renamed"}),
            &["allow_once"],
        )
        .await;
        assert_eq!(renamed.ok()["connection_id"], connection.id.as_str());
        crew().disconnect(&connection.id).await.unwrap();
        let connected = call(
            &session,
            public(),
            PersonTool::Change,
            "connect",
            json!({"connection": connection.id}),
            &["allow_once"],
        )
        .await;
        assert_eq!(connected.ok()["connection_id"], connection.id.as_str());
        remove(&connection).await;
    });
}
