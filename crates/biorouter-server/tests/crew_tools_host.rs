//! A chat's Crew person tools on the daemon's side (account ownership AO-12): the host the
//! daemon installs (`routes::crew_tools`), driven the way the `crew` extension drives it, one
//! `PersonToolCall` at a time.
//!
//! What is pinned here, against the real host, the real `CrewManager` and the real approval
//! registry and route:
//!
//! - every action is described, with parameters the host then holds a call to;
//! - each read reaches the same function as its route (`GET /crew/connections`, the invitation
//!   preview, the workspace snapshot);
//! - every change parks exactly one proof-backed card, written from the resolved target,
//!   changes nothing when denied, and sends exactly what the card showed when allowed;
//! - the first content read in a chat asks the person once, and the chat is raised Private
//!   before content returns;
//! - each privacy layer: a public model, protected channels, institutions, an unlabelled
//!   Private workspace, withheld names in refusals, and the grants list's filter;
//! - without a person's proof nothing is parked;
//! - `POST /agent/call_tool` refuses both person tools.
//!
//! **Offline.** The daemon's `ssh` is this binary, as in `crew_join_routes.rs`, whose fake
//! broker is copied below: a `#[ctor]` puts a two-line `ssh` script first on `PATH` that runs
//! this executable as a fake OpenSSH and bridge, answering from a per-host scenario file and
//! logging every frame it receives.
//!
//! **One runtime.** The host holds the daemon's `AppState`, and the Crew manager keeps bridges
//! alive on the runtime that dialled them, so every test runs on one shared runtime rather than
//! a runtime of its own. **Its own binary**: the host and the user-action digest are
//! process-global `OnceLock`s.
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
use biorouter::privacy::affiliation::{InstitutionId, InstitutionSet, ModelAffiliation};
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
const FAKE_SSH_ROOT: &str = "BIOROUTER_CREW_TOOLS_HOST_FAKE_SSH";
/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-tools-host-user-action-key";
/// The daemon secret the full router checks.
const SECRET: &str = "crew-tools-host-secret";
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
            resolves_to: None,
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
            let hostname = load(root, &host)
                .and_then(|scenario| scenario.resolves_to)
                .unwrap_or_else(|| host.clone());
            let mut out = std::io::stdout().lock();
            // First wins in both of the daemon's readers, so the host's own lines go first.
            let _ = writeln!(out, "hostname {hostname}\nport 22");
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

/// This test process's fixture directory: the `ssh` script, scenarios and logs, and the
/// development profile the file credential backend writes under.
fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!("biorouter-crew-tools-host-{}", std::process::id()))
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
            "crew_tools_host: could not prepare the fixture at {}: {error}. Refusing to run: \
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
            std::env::temp_dir().join("crew-tools-host"),
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

/// A private model only another institution's agreements cover.
async fn private_stanford() -> CallCapability {
    sampled(
        ProviderTier::Private,
        Some(ModelAffiliation::Institutions(InstitutionSet::of(
            InstitutionId::new("stanford"),
        ))),
    )
    .await
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

/// What the person sees of the workspace: two members and two channels, `c-patients`
/// protected when `protected` says so.
fn snapshot(mode: &str, institution: Option<&str>, protected: bool) -> Value {
    json!({"result": {
        "workspace": {
            "id": "w", "name": "lab", "mode": mode, "institution_id": institution,
            "host_principal_id": "p-alice", "policy_epoch": 1,
        },
        "actor": {
            "id": "p-alice", "username": "alice", "display_name": "Alice Chen",
            "devices": [{"fingerprint": "AAAA AAAA AAAA AAAA", "added_via": "invitation",
                "added_at": 1, "this_device": true}],
        },
        "principals": [
            {"id": "p-alice", "username": "alice", "display_name": "Alice Chen"},
            {"id": "p-bob", "username": "bob", "display_name": "Bob Lee"},
        ],
        "teams": [],
        "channels": [{"id": "c-general", "name": "general"}, {"id": "c-patients", "name": "patients"}],
        "protected_channel_ids": if protected { json!(["c-patients"]) } else { json!([]) },
        "invitations": [],
    }})
}

/// A workspace on `host`: staged, saved on this computer from its invitation (Private and
/// `ucsf` unless `public`), and connected when `connect` says so. Answers the connection.
async fn workspace(
    workspace_id: &str,
    host: &str,
    public: bool,
    snapshot_answer: Value,
    connect: bool,
) -> biorouter::crew::Connection {
    let mut scenario = Scenario::new(workspace_id, &ACCOUNT_CAPABILITIES)
        .answer("workspace.snapshot", snapshot_answer)
        .answer("workspace.rename", json!({"result": {"name": "lab-2"}}))
        .answer("enrollment.invite", json!({"result": {"username": "bob"}}))
        .answer("enrollment.revoke", json!({"result": {"revoked": true}}))
        .answer("policy.set", json!({"result": {"mode": "private"}}))
        .answer(
            "workspace.context.get",
            json!({"result": {"context": {"text": "old line\nkept", "revision": 3}}}),
        )
        .answer("workspace.context.set", json!({"result": {"revision": 4}}));
    if public {
        scenario.mode = "public".into();
    }
    stage(host, &scenario);
    let overrides = biorouter::crew::authentication::InvitationOverrides {
        mode: public.then_some(biorouter::crew::ClusterMode::Public),
        ..Default::default()
    };
    let saved = match crew()
        .connection_from_invitation(
            &invitation(workspace_id, host, Some("ucsf")),
            false,
            overrides,
        )
        .await
        .expect("the invitation saves")
    {
        biorouter::crew::authentication::InvitationOutcome::Saved(connection) => *connection,
        biorouter::crew::authentication::InvitationOutcome::Preview(_) => panic!("a preview"),
    };
    if connect {
        crew()
            .connect(&saved.id)
            .await
            .expect("the fake workspace verifies");
    }
    saved
}

async fn remove(connection: &biorouter::crew::Connection) {
    let _ = crew().remove(&connection.id).await;
}

fn sent(host: &str, method: &str) -> Vec<Value> {
    received(host, method)
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

/// `describe` answers every action of both tools with its parameters and an example, and a
/// parameter the table does not list is refused with how to see them. None of it parks a card.
/// That every action now runs is `crew_tools_actions::every_action_runs_now`.
#[test]
#[serial]
fn the_host_describes_every_action_and_holds_calls_to_it() {
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
            let answer = described.ok();
            assert_eq!(answer["action"], *action);
            assert_eq!(answer["params"]["type"], "object", "{answer}");
            assert!(answer["example"]["params"].is_object(), "{answer}");
            assert!(described.cards.is_empty());
        }
        let wrong = call(
            &session,
            public(),
            PersonTool::Read,
            "status",
            json!({"bogus": 1}),
            &[],
        )
        .await;
        assert!(
            wrong.refusal().contains("takes no parameter bogus"),
            "{}",
            wrong.refusal()
        );
        assert!(wrong.refusal().contains("describe"), "{}", wrong.refusal());
    });
}

/// The connection reads answer what their routes answer: `status` lists what
/// `GET /crew/connections` lists, `connections_show` is `GET /crew/connections/{id}`, and
/// `connections_preview` is the invitation preview. A connection list carries no workspace
/// content, so a public model gets it with no card.
#[test]
#[serial]
fn the_connection_reads_answer_what_their_routes_answer() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c0001";
        const HOST: &str = "reads.example.test";
        let connection = workspace(
            WORKSPACE,
            HOST,
            false,
            snapshot("private", Some("ucsf"), false),
            false,
        )
        .await;
        let session = chat("reads").await;

        let listed = call(
            &session,
            public(),
            PersonTool::Read,
            "status",
            json!({}),
            &[],
        )
        .await;
        let (status, routed) = http("GET", "/crew/connections", None).await;
        assert_eq!(status, StatusCode::OK, "{routed}");
        let ids = |list: &Value| -> Vec<String> {
            list["connections"]
                .as_array()
                .unwrap()
                .iter()
                .map(|connection| connection["id"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(ids(listed.ok()), ids(&routed));
        assert!(ids(&routed).contains(&connection.id));
        assert!(listed.cards.is_empty());

        let shown = call(
            &session,
            public(),
            PersonTool::Read,
            "connections_show",
            json!({"connection": connection.id}),
            &[],
        )
        .await;
        let (status, routed) =
            http("GET", &format!("/crew/connections/{}", connection.id), None).await;
        assert_eq!(status, StatusCode::OK, "{routed}");
        for key in [
            "id",
            "name",
            "ssh_target",
            "workspace_id",
            "server_label",
            "mode",
        ] {
            assert_eq!(shown.ok()[key], routed[key], "{key}");
        }

        let text = invitation(WORKSPACE, HOST, Some("ucsf"));
        let previewed = call(
            &session,
            public(),
            PersonTool::Read,
            "connections_preview",
            json!({"invitation": text}),
            &[],
        )
        .await;
        let (status, routed) = http(
            "POST",
            "/crew/connections/from-invitation",
            Some(json!({"invitation": text, "preview": true})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{routed}");
        assert_eq!(previewed.ok(), &routed["preview"]);
        remove(&connection).await;
    });
}

/// A change parks exactly one proof-backed card, written from the resolved target, and changes
/// nothing when the person denies it; allowed, it changes exactly what the card showed.
#[test]
#[serial]
fn a_change_parks_one_card_and_changes_nothing_when_denied() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c0002";
        const HOST: &str = "changes.example.test";
        let connection = workspace(
            WORKSPACE,
            HOST,
            false,
            snapshot("private", Some("ucsf"), false),
            false,
        )
        .await;
        let session = chat("changes").await;
        let rename = json!({"connection": connection.id, "name": "lab-renamed"});

        let denied = call(
            &session,
            public(),
            PersonTool::Change,
            "connections_update",
            rename.clone(),
            &["deny"],
        )
        .await;
        assert_eq!(
            denied.refusal(),
            "The person declined. Nothing was changed."
        );
        assert_eq!(denied.cards.len(), 1);
        let card = &denied.cards[0];
        assert_eq!(card.tool, "crew__change");
        let server = biorouter::crew::server_label(&connection.ssh_target, connection.port).await;
        assert!(
            card.prompt.starts_with(&format!(
                "Change the saved connection to {} on {server}",
                connection.name
            )),
            "{}",
            card.prompt
        );
        assert_eq!(card.arguments["workspace"], connection.name.as_str());
        assert_eq!(card.arguments["server"], server.as_str());
        assert_eq!(
            card.arguments["name"], "lab-renamed",
            "{:?}",
            card.arguments
        );
        assert_eq!(card.arguments["action"], "connections_update");
        assert_eq!(
            crew().connection(&connection.id).await.unwrap().name,
            connection.name
        );

        let allowed = call(
            &session,
            public(),
            PersonTool::Change,
            "connections_update",
            rename,
            &["allow_once"],
        )
        .await;
        assert_eq!(allowed.ok()["connection"]["name"], "lab-renamed");
        assert_eq!(allowed.cards.len(), 1);
        assert_eq!(
            crew().connection(&connection.id).await.unwrap().name,
            "lab-renamed"
        );

        let model = biorouter::config::Config::global()
            .get_param::<Value>("CREW_TASK_MODEL")
            .ok();
        let choice = json!({"provider": "versa_azure", "model": "gpt-5.5"});
        let denied = call(
            &session,
            public(),
            PersonTool::Change,
            "defaults_set",
            choice.clone(),
            &["deny"],
        )
        .await;
        assert_eq!(denied.cards.len(), 1);
        assert!(
            denied.cards[0].prompt.contains("gpt-5.5 (versa_azure)"),
            "{}",
            denied.cards[0].prompt
        );
        assert_eq!(
            biorouter::config::Config::global()
                .get_param::<Value>("CREW_TASK_MODEL")
                .ok(),
            model
        );
        let allowed = call(
            &session,
            public(),
            PersonTool::Change,
            "defaults_set",
            choice.clone(),
            &["allow_once"],
        )
        .await;
        allowed.ok();
        assert_eq!(
            biorouter::config::Config::global()
                .get_param::<Value>("CREW_TASK_MODEL")
                .ok(),
            Some(choice)
        );
        let cleared = call(
            &session,
            public(),
            PersonTool::Change,
            "defaults_clear",
            json!({}),
            &["allow_once"],
        )
        .await;
        assert_eq!(cleared.ok()["cleared"], true);

        let kept = call(
            &session,
            public(),
            PersonTool::Change,
            "connections_remove",
            json!({"connection": connection.id}),
            &["deny"],
        )
        .await;
        assert_eq!(kept.cards.len(), 1);
        assert!(
            kept.cards[0]
                .prompt
                .contains("can't get it through a new connection"),
            "{}",
            kept.cards[0].prompt
        );
        assert!(crew().connection(&connection.id).await.is_ok());
        let removed = call(
            &session,
            public(),
            PersonTool::Change,
            "connections_remove",
            json!({"connection": connection.id}),
            &["allow_once"],
        )
        .await;
        assert_eq!(removed.ok()["removed"], true);
        assert!(crew().connection(&connection.id).await.is_err());
    });
}

/// A workspace change reaches the workspace through the same request `biorouter crew` sends,
/// only once allowed, with exactly what the card named: a person resolved to their ID and
/// guarded by their username, a new name, a policy.
#[test]
#[serial]
fn workspace_changes_send_what_the_card_showed() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c0003";
        const HOST: &str = "workspace-changes.example.test";
        let connection = workspace(
            WORKSPACE,
            HOST,
            false,
            snapshot("private", Some("ucsf"), false),
            true,
        )
        .await;
        let session = chat("workspace changes").await;

        let denied = call(
            &session,
            public(),
            PersonTool::Change,
            "workspace_rename",
            json!({"name": "lab-2"}),
            &["deny"],
        )
        .await;
        denied.refusal();
        assert_eq!(denied.cards.len(), 1);
        assert!(
            denied.cards[0].prompt.starts_with("Rename lab on"),
            "{}",
            denied.cards[0].prompt
        );
        assert!(
            denied.cards[0].prompt.contains("to lab-2, for everyone"),
            "{}",
            denied.cards[0].prompt
        );
        assert!(
            sent(HOST, "workspace.rename").is_empty(),
            "a denied rename reached the workspace"
        );

        let allowed = call(
            &session,
            public(),
            PersonTool::Change,
            "workspace_rename",
            json!({"name": "lab-2"}),
            &["allow_once"],
        )
        .await;
        assert_eq!(allowed.ok()["renamed"], true);
        let frames = sent(HOST, "workspace.rename");
        assert_eq!(frames.len(), 1, "{frames:?}");
        assert_eq!(frames[0]["params"]["name"], "lab-2");
        assert!(frames[0]["params"]["idempotency_key"].is_string());

        let revoked = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "enroll_revoke",
            json!({"member": "@bob"}),
            &["allow_once"],
        )
        .await;
        revoked.ok();
        assert_eq!(revoked.cards.len(), 1);
        assert!(
            revoked.cards[0].prompt.contains("Bob Lee"),
            "{}",
            revoked.cards[0].prompt
        );
        assert!(
            revoked.cards[0]
                .prompt
                .contains("for everyone; this cannot be undone"),
            "{}",
            revoked.cards[0].prompt
        );
        let frames = sent(HOST, "enrollment.revoke");
        assert_eq!(frames.len(), 1, "{frames:?}");
        assert_eq!(frames[0]["params"]["principal_id"], "p-bob");
        assert_eq!(frames[0]["params"]["expected_username"], "bob");

        let policy = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "privacy_set_workspace",
            json!({"mode": "private", "institution_id": "ucsf"}),
            &["allow_once"],
        )
        .await;
        policy.ok();
        let frames = sent(HOST, "policy.set");
        assert_eq!(frames.len(), 1, "{frames:?}");
        assert_eq!(frames[0]["params"]["mode"], "private");
        assert_eq!(frames[0]["params"]["institution_id"], "ucsf");

        // The context document's card shows the change as a diff of the current one.
        let context = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "context_set",
            json!({"text": "new line\nkept"}),
            &["deny"],
        )
        .await;
        context.refusal();
        assert_eq!(context.cards.len(), 1);
        assert_eq!(context.cards[0].arguments["text"], "new line\nkept");
        assert_eq!(context.cards[0].arguments["revision"], 3);
        assert!(sent(HOST, "workspace.context.set").is_empty());
        remove(&connection).await;
    });
}

/// The first read of a workspace's content in a chat parks one card, "Let this chat read lab
/// as you", which says where the consent is taken back. Declined, the read says so, and a retry
/// before the person writes again parks no second card. Allowed, the chat is raised Private
/// before the content returns, and later reads ask nothing. The consent is listed by
/// `grants_list` and taken back by `grants_revoke`.
#[test]
#[serial]
fn a_chat_reads_a_workspace_only_after_the_person_lets_it() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c0004";
        const HOST: &str = "consent.example.test";
        let connection = workspace(
            WORKSPACE,
            HOST,
            false,
            snapshot("private", Some("ucsf"), true),
            true,
        )
        .await;
        let session = chat("consent").await;

        // Fresh, the person has not written yet: a read before their first message is still
        // asked once and answered.
        let denied = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "workspace_show",
            json!({}),
            &["deny"],
        )
        .await;
        // A decline is said as one, never as a card still waiting.
        assert_eq!(denied.refusal(), "The person declined. Nothing was read.");
        assert_eq!(denied.cards.len(), 1);
        assert_eq!(denied.cards[0].tool, "crew__read");
        let (heading, body) = denied.cards[0]
            .prompt
            .split_once('\n')
            .expect("a heading and a body");
        assert_eq!(heading, "Let this chat read lab as you");
        // Where to take it back: this chat's Crew bar, or the command that takes back this
        // chat's consent, not a Crew grant it never had.
        assert!(body.contains("in this chat's Crew bar"), "{body}");
        assert!(
            body.ends_with(&format!(
                "or with biorouter crew grants revoke {session} --reads."
            )),
            "{body}"
        );
        assert!(!crew().has_person_read(&session, &connection.id).await);
        assert_eq!(
            classification(&session).await,
            SessionClassification::Public
        );

        // A retry in the same turn parks no second card: the person already answered.
        let retried = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "members",
            json!({}),
            &["allow_once"],
        )
        .await;
        assert!(retried.cards.is_empty(), "{:?}", retried.cards);
        assert!(
            retried.refusal().contains("The person declined"),
            "{}",
            retried.refusal()
        );
        assert!(!crew().has_person_read(&session, &connection.id).await);

        // Once the person writes again, the read asks again.
        state()
            .await
            .session_manager()
            .add_message(
                &session,
                &biorouter::conversation::message::Message::user().with_text("Go ahead, read it."),
            )
            .await
            .unwrap();

        let allowed = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "workspace_show",
            json!({}),
            &["allow_once"],
        )
        .await;
        assert_eq!(
            allowed.ok()["channels"].as_array().unwrap().len(),
            2,
            "a private model sees every channel"
        );
        assert!(crew().has_person_read(&session, &connection.id).await);
        assert_eq!(
            classification(&session).await,
            SessionClassification::Private,
            "raised before the answer returned"
        );

        let again = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "members",
            json!({}),
            &[],
        )
        .await;
        assert!(again.cards.is_empty(), "the consent is asked once per chat");
        let members = again.ok()["members"].as_array().unwrap().clone();
        assert_eq!(members[0]["username"], "alice");
        assert_eq!(members[0]["is_host"], true);

        let listed = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "grants_list",
            json!({}),
            &[],
        )
        .await;
        let reads = listed.ok()["person_reads"].as_array().unwrap().clone();
        assert!(
            reads
                .iter()
                .any(|read| read["session_id"] == session.as_str()),
            "{reads:?}"
        );

        let revoked = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "grants_revoke",
            json!({"session": session, "reads": true}),
            &["allow_once"],
        )
        .await;
        assert_eq!(revoked.ok()["revoked"], true);
        assert!(!crew().has_person_read(&session, &connection.id).await);
        // Its card names the chat as `grants list` does, and shows that it takes back the
        // permission to read, not a Crew grant.
        assert_eq!(revoked.cards.len(), 1);
        let server = biorouter::crew::server_label(&connection.ssh_target, connection.port).await;
        assert_eq!(
            revoked.cards[0].prompt,
            format!(
                "Take back permission for chat \"consent\" (chat {session}) to read lab on {server} as you"
            )
        );
        assert_eq!(
            revoked.cards[0].arguments["takes_back"],
            "permission to read as you"
        );
        assert_eq!(revoked.cards[0].arguments["session"], session.as_str());
        remove(&connection).await;
    });
}

/// Each privacy layer of AO-12: a public model never reads a Private workspace and is told so
/// without its name; through a Public connection to a Public workspace it gets only the
/// public-safe channels and the chat stays Public; a private model another institution covers
/// is refused; nobody reads an unlabelled Private workspace; and a name that does not resolve in
/// a workspace the call may not read lists nothing from it.
#[test]
#[serial]
fn each_privacy_layer_holds() {
    run(async {
        const PRIVATE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c0005";
        const PRIVATE_HOST: &str = "privacy-private.example.test";
        let private = workspace(
            PRIVATE,
            PRIVATE_HOST,
            false,
            snapshot("private", Some("ucsf"), false),
            true,
        )
        .await;
        let session = chat("privacy").await;

        let refused = call(
            &session,
            public(),
            PersonTool::Read,
            "members",
            json!({}),
            &[],
        )
        .await;
        assert!(
            refused
                .refusal()
                .starts_with("This chat uses a public model"),
            "{}",
            refused.refusal()
        );
        assert!(
            !refused.refusal().contains("lab"),
            "a refusal named the workspace: {}",
            refused.refusal()
        );
        assert!(refused.cards.is_empty());

        let unknown = call(
            &session,
            public(),
            PersonTool::Change,
            "enroll_revoke",
            json!({"member": "@carol"}),
            &[],
        )
        .await;
        assert!(
            unknown
                .refusal()
                .contains("Nothing you can see in this workspace is named @carol."),
            "{}",
            unknown.refusal()
        );
        assert!(!unknown.refusal().contains("bob"), "{}", unknown.refusal());
        assert!(unknown.cards.is_empty());

        let other = call(
            &session,
            private_stanford().await,
            PersonTool::Read,
            "members",
            json!({}),
            &[],
        )
        .await;
        assert!(
            other.refusal().contains("crew_institution_mismatch"),
            "{}",
            other.refusal()
        );
        assert!(other.cards.is_empty());

        stage(
            PRIVATE_HOST,
            &Scenario::new(PRIVATE, &ACCOUNT_CAPABILITIES)
                .answer("workspace.snapshot", snapshot("private", None, false)),
        );
        let unlabelled = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "members",
            json!({}),
            &[],
        )
        .await;
        assert!(
            unlabelled
                .refusal()
                .contains("and no institution, so a chat can't read it"),
            "{}",
            unlabelled.refusal()
        );
        assert!(unlabelled.cards.is_empty());
        remove(&private).await;

        const PUBLIC: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c0006";
        const PUBLIC_HOST: &str = "privacy-public.example.test";
        let open = workspace(
            PUBLIC,
            PUBLIC_HOST,
            true,
            snapshot("public", Some("ucsf"), true),
            true,
        )
        .await;
        let shown = call(
            &session,
            public(),
            PersonTool::Read,
            "workspace_show",
            json!({}),
            &["allow_once"],
        )
        .await;
        let channels = shown.ok()["channels"].as_array().unwrap().clone();
        assert_eq!(
            channels,
            vec![json!({"id": "c-general", "name": "general"})]
        );
        assert_eq!(shown.ok()["withheld"]["channels"], 1);
        assert!(shown.ok().get("protected_channel_ids").is_none());
        assert_eq!(
            classification(&session).await,
            SessionClassification::Public,
            "a public read raises nothing"
        );

        // With nothing protected in it, a Public workspace's content is no institution's, so
        // a private model of another institution reads it too, and nothing is raised.
        stage(
            PUBLIC_HOST,
            &Scenario {
                mode: "public".into(),
                ..Scenario::new(PUBLIC, &ACCOUNT_CAPABILITIES)
            }
            .answer(
                "workspace.snapshot",
                snapshot("public", Some("ucsf"), false),
            ),
        );
        let read = call(
            &session,
            private_stanford().await,
            PersonTool::Read,
            "members",
            json!({}),
            &[],
        )
        .await;
        assert_eq!(read.ok()["members"].as_array().unwrap().len(), 2);
        assert!(read.cards.is_empty(), "the consent was given once already");
        assert_eq!(
            classification(&session).await,
            SessionClassification::Public
        );
        remove(&open).await;
    });
}

/// `grants_list` lists only the chats this chat may list: a public model is never shown a
/// private chat's consent.
#[test]
#[serial]
fn grants_list_shows_only_chats_this_chat_may_list() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c0007";
        const HOST: &str = "grants.example.test";
        let connection = workspace(
            WORKSPACE,
            HOST,
            false,
            snapshot("private", Some("ucsf"), false),
            false,
        )
        .await;
        let open_chat = chat("public chat").await;
        let private_chat = chat("private chat").await;
        state()
            .await
            .session_manager()
            .update(&private_chat)
            .raise_privacy(SessionClassification::Private, "turn:fixture")
            .apply()
            .await
            .unwrap();
        crew()
            .grant_person_read(&open_chat, &connection.id)
            .await
            .unwrap();
        crew()
            .grant_person_read(&private_chat, &connection.id)
            .await
            .unwrap();
        let asker = chat("asker").await;

        let listed = |capability| {
            let asker = asker.clone();
            async move {
                let answer = call(
                    &asker,
                    capability,
                    PersonTool::Read,
                    "grants_list",
                    json!({}),
                    &[],
                )
                .await;
                answer.ok()["person_reads"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|read| read["session_id"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            }
        };
        let to_public = listed(public()).await;
        assert!(to_public.contains(&open_chat));
        assert!(
            !to_public.contains(&private_chat),
            "a public model was shown a private chat"
        );
        let to_private = listed(private_local().await).await;
        assert!(to_private.contains(&open_chat) && to_private.contains(&private_chat));

        let hidden = call(
            &asker,
            public(),
            PersonTool::Change,
            "grants_revoke",
            json!({"session": private_chat, "reads": true}),
            &[],
        )
        .await;
        assert!(
            hidden
                .refusal()
                .contains("No chat this chat can see has that ID."),
            "{}",
            hidden.refusal()
        );
        assert!(hidden.cards.is_empty());
        remove(&connection).await;
    });
}

/// Checking a server no saved connection names signs in there as the person, so it is a change:
/// it asks first, and a denial starts no job.
#[test]
#[serial]
fn checking_an_unsaved_server_asks_first() {
    run(async {
        let session = chat("check").await;
        let before = biorouter::crew::setup_jobs().len();
        let denied = call(
            &session,
            public(),
            PersonTool::Read,
            "setup_check",
            json!({"server": "nobody@unsaved.example.test"}),
            &["deny"],
        )
        .await;
        assert_eq!(
            denied.refusal(),
            "The person declined. Nothing was changed."
        );
        assert_eq!(denied.cards.len(), 1);
        assert!(
            denied.cards[0]
                .prompt
                .starts_with("Check nobody@unsaved.example.test as you"),
            "{}",
            denied.cards[0].prompt
        );
        assert_eq!(
            biorouter::crew::setup_jobs().len(),
            before,
            "a denied check started a job"
        );
    });
}

/// On a daemon that cannot obtain a person's proof (`biorouter serve`), nothing is parked: a
/// change says it needs the desktop app or a terminal, and a content read says so with
/// `crew_read_consent_required` rather than parking a card nobody can answer.
#[test]
#[serial]
fn without_a_person_s_proof_nothing_is_parked() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c0008";
        const HOST: &str = "no-proof.example.test";
        let connection = workspace(
            WORKSPACE,
            HOST,
            false,
            snapshot("private", Some("ucsf"), false),
            true,
        )
        .await;
        let session = chat("no proof").await;
        biorouter::pending_user_action::set_user_proof_available(false);
        let change = call(
            &session,
            public(),
            PersonTool::Change,
            "disconnect",
            json!({}),
            &[],
        )
        .await;
        let read = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "members",
            json!({}),
            &[],
        )
        .await;
        biorouter::pending_user_action::set_user_proof_available(true);
        assert!(
            change.refusal().starts_with(
                "This needs your approval, which this Biorouter can't ask you for here."
            ),
            "{}",
            change.refusal()
        );
        assert!(change.cards.is_empty());
        assert!(
            read.refusal().contains("crew_read_consent_required"),
            "{}",
            read.refusal()
        );
        assert!(
            read.refusal().contains("can't ask you for here"),
            "{}",
            read.refusal()
        );
        assert!(read.cards.is_empty());
        assert_eq!(
            crew().connection(&connection.id).await.unwrap().status,
            "connected"
        );
        remove(&connection).await;
    });
}

/// A read whose consent card waited for the person is judged again once it is allowed: a
/// workspace made Private while the card was open is not read by a public model, and nothing
/// is fetched from it.
#[test]
#[serial]
fn a_read_is_judged_again_after_its_consent_card_waits() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c0009";
        const HOST: &str = "consent-wait.example.test";
        let open = workspace(
            WORKSPACE,
            HOST,
            true,
            snapshot("public", Some("ucsf"), false),
            true,
        )
        .await;
        let session = chat("consent wait").await;
        let made_private = async {
            let mut scenario = Scenario::new(WORKSPACE, &ACCOUNT_CAPABILITIES)
                .answer(
                    "workspace.snapshot",
                    snapshot("private", Some("ucsf"), false),
                )
                .answer(
                    "workspace.context.get",
                    json!({"result": {"context": {"text": "patients' notes", "revision": 4}}}),
                );
            scenario.mode = "public".into();
            stage(HOST, &scenario);
        };
        let read = call_meanwhile(
            &session,
            public(),
            PersonTool::Read,
            "context_show",
            json!({"connection": open.id}),
            "allow_once",
            made_private,
        )
        .await;
        assert_eq!(read.cards.len(), 1, "the consent card");
        assert!(
            read.refusal().starts_with("This chat uses a public model"),
            "{}",
            read.refusal()
        );
        assert!(!read.refusal().contains("patients"), "{}", read.refusal());
        assert!(
            sent(HOST, "workspace.context.get").is_empty(),
            "the document was fetched after the workspace was made Private"
        );
        remove(&open).await;
    });
}

/// A connection's card is acted on only as it showed it: when the connection changes while the
/// card waits, allowing the card saves nothing over the change, and says why.
#[test]
#[serial]
fn a_connection_change_saves_nothing_over_a_change_made_while_it_waited() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c000a";
        const HOST: &str = "stale-card.example.test";
        let connection = workspace(
            WORKSPACE,
            HOST,
            false,
            snapshot("private", Some("ucsf"), false),
            false,
        )
        .await;
        let session = chat("stale card").await;
        let id = connection.id.clone();
        let changed_meanwhile = async {
            // The person renames it in the Crew view while the chat's card is open.
            let saved = serde_json::to_value(crew().connection(&id).await.unwrap()).unwrap();
            let mut input = serde_json::Map::new();
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
                "remote_root",
                "remote_execution",
                "remote_access",
                "cluster_connection_id",
                "mode",
                "institution_id",
            ] {
                if let Some(value) = saved.get(key) {
                    input.insert(key.into(), value.clone());
                }
            }
            input.insert("name".into(), json!("lab-meanwhile"));
            let request: biorouter::crew::SaveConnection =
                serde_json::from_value(Value::Object(input)).unwrap();
            crew().update(&id, request).await.unwrap();
        };
        let allowed = call_meanwhile(
            &session,
            public(),
            PersonTool::Change,
            "connections_update",
            json!({"connection": connection.id, "name": "lab-renamed"}),
            "allow_once",
            changed_meanwhile,
        )
        .await;
        assert_eq!(allowed.cards.len(), 1);
        assert!(
            allowed
                .refusal()
                .contains("changed while this waited for your approval, so nothing was changed"),
            "{}",
            allowed.refusal()
        );
        assert_eq!(
            crew().connection(&connection.id).await.unwrap().name,
            "lab-meanwhile",
            "the card's save went over a change it never showed"
        );
        remove(&connection).await;
    });
}

/// A check through a saved login signs in as the person without a card only on that
/// connection's own route: the same login through another jump host asks first.
#[test]
#[serial]
fn checking_a_saved_login_through_another_route_asks_first() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c000b";
        const HOST: &str = "saved-route.example.test";
        let connection = workspace(
            WORKSPACE,
            HOST,
            false,
            snapshot("private", Some("ucsf"), false),
            false,
        )
        .await;
        let session = chat("another route").await;
        let before = biorouter::crew::setup_jobs().len();
        let denied = call(
            &session,
            public(),
            PersonTool::Read,
            "setup_check",
            json!({"server": connection.ssh_target, "proxy_jump": "other@jump.example.test"}),
            &["deny"],
        )
        .await;
        assert_eq!(denied.cards.len(), 1, "{:?}", denied.answer);
        assert_eq!(
            denied.refusal(),
            "The person declined. Nothing was changed."
        );
        assert!(
            denied.cards[0]
                .prompt
                .starts_with(&format!("Check {} as you", connection.ssh_target)),
            "{}",
            denied.cards[0].prompt
        );
        assert_eq!(
            biorouter::crew::setup_jobs().len(),
            before,
            "a denied check started a job"
        );
        remove(&connection).await;
    });
}

/// Joining through a saved connection names the workspace's host on its card, as joining from
/// an invitation and opening one do.
#[test]
#[serial]
fn joining_a_saved_connection_names_its_host() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c000c";
        const HOST: &str = "join-host.example.test";
        let connection = workspace(
            WORKSPACE,
            HOST,
            false,
            snapshot("private", Some("ucsf"), false),
            true,
        )
        .await;
        stage(
            HOST,
            &Scenario::new(WORKSPACE, &ACCOUNT_CAPABILITIES)
                .answer("workspace.snapshot", snapshot("private", Some("ucsf"), false))
                .answer(
                    "account.status",
                    json!({"result": {"role": "invited", "host_username": "alice", "expired": false}}),
                ),
        );
        let session = chat("join host").await;
        let denied = call(
            &session,
            public(),
            PersonTool::Change,
            "join",
            json!({"connection": connection.id}),
            &["deny"],
        )
        .await;
        assert_eq!(denied.cards.len(), 1, "{:?}", denied.answer);
        let prompt = &denied.cards[0].prompt;
        let server = biorouter::crew::server_label(&connection.ssh_target, connection.port).await;
        assert!(
            prompt.starts_with(&format!(
                "Join {} on {server}, hosted by @alice, as you",
                connection.name
            )),
            "{prompt}"
        );
        assert!(prompt.contains("Workspace key fingerprint:"), "{prompt}");
        remove(&connection).await;
    });
}

/// Setting full access up on a connection whose account hosts the workspace is host authority,
/// and its card says so in the desktop's words, after what full access lets agents do. On a
/// workspace someone else hosts, it does not.
#[test]
#[serial]
fn full_access_where_the_person_hosts_says_agents_can_act_as_the_host() {
    run(async {
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000c000d";
        const HOST: &str = "full-host.example.test";
        let connection = workspace(
            WORKSPACE,
            HOST,
            false,
            snapshot("private", Some("ucsf"), false),
            true,
        )
        .await;
        let session = chat("full access").await;
        let setup = |session: String, id: String| async move {
            call(
                &session,
                private_local().await,
                PersonTool::Change,
                "setup",
                json!({"connection": id, "agent_access": "full"}),
                &["deny"],
            )
            .await
        };
        let before = biorouter::crew::setup_jobs().len();
        let hosted = setup(session.clone(), connection.id.clone()).await;
        assert_eq!(hosted.cards.len(), 1, "{:?}", hosted.answer);
        assert_eq!(
            hosted.refusal(),
            "The person declined. Nothing was changed."
        );
        let lines: Vec<&str> = hosted.cards[0].prompt.lines().collect();
        let name = &connection.name;
        let at = lines
            .iter()
            .position(|line| line.starts_with("Agents get full access"))
            .unwrap_or_else(|| panic!("no access line: {lines:?}"));
        let hosting = format!(
            "You host {name} from this account, so agents can also read the workspace's own files on the server, including channels you're not in. They can also act as the host of {name}."
        );
        assert_eq!(
            lines.get(at + 1).copied(),
            Some(hosting.as_str()),
            "{lines:?}"
        );

        // Bob hosts it: full access is no host authority, and the card says nothing of it.
        let mut bobs = snapshot("private", Some("ucsf"), false);
        bobs["result"]["workspace"]["host_principal_id"] = json!("p-bob");
        stage(
            HOST,
            &Scenario::new(WORKSPACE, &ACCOUNT_CAPABILITIES).answer("workspace.snapshot", bobs),
        );
        let member = setup(session.clone(), connection.id.clone()).await;
        assert_eq!(member.cards.len(), 1, "{:?}", member.answer);
        assert!(
            member.cards[0].prompt.contains("Agents get full access"),
            "{}",
            member.cards[0].prompt
        );
        assert!(
            !member.cards[0].prompt.contains("act as the host"),
            "{}",
            member.cards[0].prompt
        );
        assert_eq!(
            biorouter::crew::setup_jobs().len(),
            before,
            "a denied setup started a job"
        );
        remove(&connection).await;
    });
}

/// AO-12, "Which chats": a chat's tool listing (`GET /agent/tools`, which the desktop draws a
/// chat's tools from) shows the Crew tools that chat is offered and no other: a person's chat
/// with no grant lists `crew__read` and `crew__change`, never the grant's worker tools it would
/// be refused. Every other extension's tools stay listed.
#[test]
#[serial]
fn a_chat_s_tool_listing_shows_only_the_crew_tools_it_is_offered() {
    run(async {
        let session = chat("tool listing").await;
        let agent = state()
            .await
            .get_agent_for_route(session.clone())
            .await
            .expect("the chat's agent");
        agent
            .add_extension(biorouter::agents::ExtensionConfig::Platform {
                name: "crew".into(),
                description: "Crew".into(),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .expect("the crew extension");
        let (status, body) = http(
            "GET",
            &format!("/agent/tools?session_id={session}&extension_name=crew"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let mut names: Vec<&str> = body
            .as_array()
            .expect("a list of tools")
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(names, ["crew__change", "crew__read"], "{body}");
    });
}

/// `POST /agent/call_tool` has no turn and no surface for a card, so it refuses both person
/// tools, for any chat, in plain words.
#[test]
#[serial]
fn the_call_tool_route_refuses_the_person_tools() {
    run(async {
        let session = chat("call tool").await;
        for tool in ["crew__read", "crew__change"] {
            let (status, body) = http(
                "POST",
                "/agent/call_tool",
                Some(
                    json!({"session_id": session, "name": tool, "arguments": {"action": "status"}}),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["is_error"], true, "{body}");
            assert!(
                body["content"][0]["text"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("crew__read and crew__change act as you")),
                "{body}"
            );
        }
    });
}
