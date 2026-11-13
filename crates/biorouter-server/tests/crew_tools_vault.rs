//! Crew's vault from a chat (the owner's D1 ruling on AO-12), against a real encrypted vault,
//! the real host (`routes::crew_tools`), the real approval registry and the real vault routes.
//!
//! What is pinned here:
//!
//! - `credentials_status` answers the vault's state and nothing else;
//! - `credentials_unlock` parks one vault card that a forged Allow cannot end and a wrong
//!   passphrase does not end; the vault route, as the desktop's main process posts it with
//!   the person's proof, unlocks the vault, and the call then takes the card down and answers;
//! - a dismissed card answers that the vault is still locked;
//! - a change that needs the vault while it is locked parks the vault card first and then
//!   exactly one approval card;
//! - `credentials_lock` parks an ordinary approval card and locks only after Allow;
//! - without a person's proof (`biorouter serve`) no card is parked;
//! - no card, answer or stored message carries a passphrase.
//!
//! **A vault, not the development key files.** The fixture leaves `BIOROUTER_DEV_PROFILE_ROOT`
//! unset, so the sandboxed Crew profile can set up an encrypted vault; every test sets it up
//! ([`vault`]) before anything else, so no Crew key is ever written outside it and the OS
//! keychain is never touched. `BIOROUTER_DISABLE_KEYRING` keeps the config's own secrets in a
//! file.
//!
//! **Offline.** The daemon's `ssh` is this binary, as in `crew_tools_host.rs`, whose fake
//! broker is copied below: a `#[ctor]` puts a two-line `ssh` script first on `PATH` that runs
//! this executable as a fake OpenSSH and bridge, answering from a per-host scenario file.
//!
//! **One runtime, its own binary**, for the reasons `crew_tools_host.rs` gives: the host and
//! the user-action digest are process-global, and the Crew manager keeps bridges on the
//! runtime that dialled them.
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
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// Set only in the fake `ssh` process: the fixture directory it reads scenarios from.
const FAKE_SSH_ROOT: &str = "BIOROUTER_CREW_TOOLS_VAULT_FAKE_SSH";
/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-tools-vault-user-action-key";
/// The daemon secret the full router checks.
const SECRET: &str = "crew-tools-vault-secret";
/// The vault's passphrase. A canary: it must never be found in a card, an answer or a stored
/// message.
const PASSPHRASE: &str = "CANARY-7f3a vault passphrase for the chat tests";
/// A wrong passphrase the person types first. A canary too.
const WRONG: &str = "CANARY-7f3a wrong passphrase";
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
// The workspace key (copied from `crew_tools_host.rs`)
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
// Scenarios: what the fake broker answers, per SSH host (copied from `crew_tools_host.rs`)
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
    resolves_to: Option<String>,
    answers: serde_json::Map<String, Value>,
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
fn sent(host: &str, method: &str) -> Vec<Value> {
    fs::read_to_string(log_path(&fixture_root(), host))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|frame| frame["method"] == method)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The fake `ssh`, run by this same binary before `main` (copied from `crew_tools_host.rs`)
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
// The fixture directory, `PATH` and credential backend, set before `main`
// ---------------------------------------------------------------------------------------------

fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!("biorouter-crew-tools-vault-{}", std::process::id()))
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
        for directory in ["bin", "scenarios"] {
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
            "crew_tools_vault: could not prepare the fixture at {}: {error}. Refusing to run: \
             the daemon would reach the real ssh.",
            root.display()
        );
        std::process::abort();
    }
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{path}", root.join("bin").display()));
    // The config's secrets in a file. Crew's keys go to the encrypted vault every test sets up
    // first, so the development key files stay off: with them on, no vault can be set up.
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    std::env::remove_var("BIOROUTER_DEV_PROFILE_ROOT");
}

#[ctor::dtor]
fn remove_fixture() {
    if std::env::var_os(FAKE_SSH_ROOT).is_none() {
        let _ = fs::remove_dir_all(fixture_root());
    }
}

// ---------------------------------------------------------------------------------------------
// The daemon: one runtime, one AppState, the host installed once, one vault
// ---------------------------------------------------------------------------------------------

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("a runtime")
});

static STATE: tokio::sync::OnceCell<Arc<AppState>> = tokio::sync::OnceCell::const_new();
static VAULT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

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

/// Set up the profile's encrypted vault once, through the route the desktop's main process
/// posts to, before anything could write a Crew key elsewhere. Every test calls it first.
async fn vault() {
    VAULT
        .get_or_init(|| async {
            let (status, body) = http(
                "POST",
                "/crew/credentials/init",
                Some(json!({"passphrase": PASSPHRASE})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["backend"], "encrypted_vault", "{body}");
        })
        .await;
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

/// The vault as `GET /crew/credentials` answers it.
async fn credentials() -> Value {
    let (status, body) = http("GET", "/crew/credentials", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

async fn lock() {
    let (status, body) = http("POST", "/crew/credentials/lock", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["locked"], true);
}

/// Unlock as the person does: through the vault route, with their proof.
async fn unlock_with(passphrase: &str) -> StatusCode {
    http(
        "POST",
        "/crew/credentials/unlock",
        Some(json!({"passphrase": passphrase})),
    )
    .await
    .0
}

async fn unlocked() {
    if credentials().await["locked"] == true {
        assert_eq!(unlock_with(PASSPHRASE).await, StatusCode::OK);
    }
}

fn crew() -> Arc<biorouter::crew::CrewManager> {
    biorouter::crew::manager().expect("the sandboxed Crew manager")
}

async fn chat(name: &str) -> String {
    state()
        .await
        .session_manager()
        .create_session(
            std::env::temp_dir().join("crew-tools-vault"),
            name.to_owned(),
            SessionType::User,
        )
        .await
        .expect("a chat")
        .id
}

/// A chat already holding private content, as a chat that posts to a private workspace is.
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

/// Every message stored in `session`, serialized.
async fn stored(session: &str) -> String {
    let session = state()
        .await
        .session_manager()
        .get_session(session, true)
        .await
        .expect("the chat");
    serde_json::to_string(&session.conversation).unwrap()
}

/// Neither passphrase anywhere in `text`.
fn assert_no_passphrase(text: &str) {
    assert!(!text.contains(PASSPHRASE), "the passphrase leaked: {text}");
    assert!(!text.contains(WRONG), "the wrong passphrase leaked: {text}");
    assert!(!text.contains("CANARY-7f3a"), "a canary leaked: {text}");
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
        "crew-tools-vault-fixture"
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
// Calls, and the person at each card
// ---------------------------------------------------------------------------------------------

/// What the person does at one card.
#[derive(Clone, Copy, Debug)]
enum Reply {
    /// Allow a tool approval card, with the desktop's proof.
    Allow,
    /// Deny a tool approval card.
    Deny,
    /// At the vault card: a forged Allow (refused), then a wrong passphrase at the prompt
    /// (refused), then the right one, posted to the vault route as the desktop's main process
    /// posts it. Checks after each that the card is still up and the vault still locked.
    Unlock,
    /// Dismiss a Crew step card.
    Dismiss,
}

/// One card a call parked, as the person saw it: its `actionType` and its data.
#[derive(Debug)]
struct SeenCard {
    kind: String,
    id: String,
    data: Value,
}

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

    fn kinds(&self) -> Vec<&str> {
        self.cards.iter().map(|card| card.kind.as_str()).collect()
    }

    /// Nothing the call parked or answered carries a passphrase.
    fn assert_no_passphrase(&self) {
        for card in &self.cards {
            assert_no_passphrase(&card.data.to_string());
        }
        match &self.answer {
            Ok(answer) => assert_no_passphrase(&answer.to_string()),
            Err(error) => assert_no_passphrase(error),
        }
    }
}

/// Call `tool`'s `action` from `session` on `capability`, and answer each card it parks with the
/// next of `replies`, as the person does. A card beyond `replies` is denied or dismissed.
async fn call(
    session: &str,
    capability: CallCapability,
    tool: PersonTool,
    action: &str,
    params: Value,
    replies: &[Reply],
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
        tokio::spawn(async move { host.call(call).await.map_err(|error| format!("{error:#}")) });
    let mut cards = Vec::new();
    let mut answered = std::collections::HashSet::new();
    loop {
        for message in PendingUserActions::global().pending_cards_for_session(session) {
            let card = serde_json::to_value(&message).unwrap();
            let data = card["content"][0]["data"].clone();
            let id = data["id"].as_str().unwrap_or_default().to_owned();
            if id.is_empty() || !answered.insert(id.clone()) {
                continue;
            }
            let kind = data["actionType"].as_str().unwrap_or_default().to_owned();
            let reply = replies.get(cards.len()).copied();
            cards.push(SeenCard {
                kind: kind.clone(),
                id: id.clone(),
                data,
            });
            answer(session, &id, &kind, reply).await;
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

/// The person's answer to the card `id`, of `kind`, in `session`.
async fn answer(session: &str, id: &str, kind: &str, reply: Option<Reply>) {
    match (kind, reply) {
        ("crewStep", Some(Reply::Unlock)) => person_unlocks(session, id).await,
        ("crewStep", _) => {
            let (status, body) = http(
                "POST",
                "/action-required/crew-step",
                Some(json!({"id": id, "session_id": session, "outcome": "dismiss"})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        (_, reply) => {
            let action = if matches!(reply, Some(Reply::Allow)) {
                "allow_once"
            } else {
                "deny"
            };
            let (status, body) = http(
                "POST",
                "/action-required/tool-confirmation",
                Some(json!({"id": id, "principalType": "Tool", "action": action, "sessionId": session})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
    }
}

/// The person at a vault card: see [`Reply::Unlock`].
async fn person_unlocks(session: &str, id: &str) {
    let still_waiting = || async {
        assert!(
            PendingUserActions::global().is_pending(id),
            "the card ended"
        );
        assert_eq!(credentials().await["locked"], true, "the vault opened");
    };
    // A forged Allow, even with the person's proof, ends nothing: the card accepts no success.
    let (status, body) = http(
        "POST",
        "/action-required/tool-confirmation",
        Some(json!({"id": id, "principalType": "Tool", "action": "allow_once", "sessionId": session})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_ne!(body["status"], "delivered", "{body}");
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    still_waiting().await;
    // A wrong passphrase at the prompt is refused by the vault, and the card stays up.
    assert_eq!(unlock_with(WRONG).await, StatusCode::BAD_REQUEST);
    still_waiting().await;
    assert_eq!(unlock_with(PASSPHRASE).await, StatusCode::OK);
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

/// What the person sees of the workspace: two members and two channels.
fn snapshot() -> Value {
    json!({"result": {
        "workspace": {
            "id": "w", "name": "lab", "mode": "private", "institution_id": "ucsf",
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
        "channels": [{"id": "c-general", "name": "general"}, {"id": "c-notes", "name": "notes"}],
        "protected_channel_ids": [],
        "invitations": [],
    }})
}

/// A Private `ucsf` workspace on `host`, saved on this computer from its invitation and
/// connected, with the vault open. Its device key is in the vault.
async fn workspace(workspace_id: &str, host: &str) -> biorouter::crew::Connection {
    unlocked().await;
    stage(
        host,
        &Scenario::new(workspace_id, &ACCOUNT_CAPABILITIES)
            .answer("workspace.snapshot", snapshot())
            .answer(
                "message.post",
                json!({"result": {"id": "m-new", "sequence": "m-new", "restricted": true}}),
            ),
    );
    let saved = match crew()
        .connection_from_invitation(
            &invitation(workspace_id, host),
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

async fn remove(connection: &biorouter::crew::Connection) {
    unlocked().await;
    let _ = crew().remove(&connection.id).await;
}

/// Restores the daemon's proof flag when a test that turned it off ends, even by a panic.
struct ProofRestored;

impl Drop for ProofRestored {
    fn drop(&mut self) {
        biorouter::pending_user_action::set_user_proof_available(true);
    }
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

/// `credentials_status` answers where the keys are and whether the vault is set up and locked,
/// under the route's names, and nothing else; it parks no card.
#[test]
#[serial]
fn the_vault_status_carries_no_secret() {
    run(async {
        vault().await;
        lock().await;
        let session = chat("vault status").await;
        let status = call(
            &session,
            public(),
            PersonTool::Read,
            "credentials_status",
            json!({}),
            &[],
        )
        .await;
        let answer = status.ok();
        let mut keys: Vec<&str> = answer
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["available", "backend", "initialized", "locked"]);
        assert_eq!(answer["backend"], "encrypted_vault");
        assert_eq!(answer["initialized"], true);
        assert_eq!(answer["locked"], true);
        assert!(status.cards.is_empty());
        status.assert_no_passphrase();
    });
}

/// Unlock parks one vault card that only the vault itself can end: a forged Allow and a wrong
/// passphrase leave it up and the vault locked; the right passphrase, through the vault route,
/// opens the vault, and the call takes the card down and answers. The card carries the action
/// and nothing else, and no passphrase is found anywhere afterwards.
#[test]
#[serial]
fn unlocking_takes_one_card_that_only_the_vault_route_can_end() {
    run(async {
        vault().await;
        lock().await;
        let session = chat("vault unlock").await;
        let unlocked = call(
            &session,
            public(),
            PersonTool::Change,
            "credentials_unlock",
            json!({}),
            &[Reply::Unlock],
        )
        .await;
        assert_eq!(unlocked.kinds(), ["crewStep"]);
        let card = &unlocked.cards[0];
        assert_eq!(
            card.data,
            json!({"actionType": "crewStep", "id": card.id, "step": {"kind": "vault", "action": "unlock"}})
        );
        let answer = unlocked.ok();
        assert_eq!(answer["locked"], false, "{answer}");
        assert_eq!(answer["note"], "Crew's vault is unlocked.");
        assert!(!PendingUserActions::global().is_pending(&card.id));
        assert_eq!(credentials().await["locked"], false);
        unlocked.assert_no_passphrase();
        assert_no_passphrase(&stored(&session).await);
    });
}

/// Dismissed, the vault is still locked, and the call says so.
#[test]
#[serial]
fn a_dismissed_vault_card_leaves_it_locked() {
    run(async {
        vault().await;
        lock().await;
        let session = chat("vault dismissed").await;
        let dismissed = call(
            &session,
            public(),
            PersonTool::Change,
            "credentials_unlock",
            json!({}),
            &[Reply::Dismiss],
        )
        .await;
        assert_eq!(dismissed.kinds(), ["crewStep"]);
        assert_eq!(
            dismissed.refusal(),
            "The person closed the card without unlocking Crew's vault, so it is still locked."
        );
        assert_eq!(credentials().await["locked"], true);
    });
}

/// A change that needs the vault while it is locked asks for the vault first, then for the
/// change itself on exactly one approval card, and then does it.
#[test]
#[serial]
fn a_locked_vault_is_asked_for_before_a_changes_one_approval() {
    run(async {
        const WORKSPACE: &str = "6b1f0a52-3c3d-4f5e-9a10-0000000e0001";
        const HOST: &str = "vault-connect.example.test";
        vault().await;
        let connection = workspace(WORKSPACE, HOST).await;
        crew().disconnect(&connection.id).await.unwrap();
        lock().await;
        let session = chat("vault before connect").await;
        let connected = call(
            &session,
            public(),
            PersonTool::Change,
            "connect",
            json!({"connection": connection.id}),
            &[Reply::Unlock, Reply::Allow],
        )
        .await;
        assert_eq!(connected.kinds(), ["crewStep", "toolConfirmation"]);
        assert_eq!(connected.cards[0].data["step"]["kind"], "vault");
        let prompt = connected.cards[1].data["prompt"].as_str().unwrap();
        assert!(prompt.starts_with("Connect to "), "{prompt}");
        assert_eq!(connected.ok()["status"], "connected", "{}", connected.ok());
        connected.assert_no_passphrase();
        remove(&connection).await;
    });
}

/// Lock asks on an ordinary approval card: denied, the vault stays open; allowed, it locks.
#[test]
#[serial]
fn lock_parks_an_approval_and_locks_only_after_allow() {
    run(async {
        vault().await;
        unlocked().await;
        let session = chat("vault lock").await;
        let denied = call(
            &session,
            public(),
            PersonTool::Change,
            "credentials_lock",
            json!({}),
            &[Reply::Deny],
        )
        .await;
        assert_eq!(denied.kinds(), ["toolConfirmation"]);
        assert_eq!(
            denied.refusal(),
            "The person declined. Nothing was changed."
        );
        assert_eq!(credentials().await["locked"], false);

        let allowed = call(
            &session,
            public(),
            PersonTool::Change,
            "credentials_lock",
            json!({}),
            &[Reply::Allow],
        )
        .await;
        assert_eq!(allowed.kinds(), ["toolConfirmation"]);
        let card = &allowed.cards[0].data;
        assert_eq!(card["toolName"], "crew__change");
        assert_eq!(
            card["prompt"],
            "Lock Crew's vault on this computer\nEvery chat and Crew task using it stops until you unlock it."
        );
        assert_eq!(allowed.ok()["locked"], true);
        assert_eq!(credentials().await["locked"], true);
    });
}

/// Where no person's proof can be had (`biorouter serve`), no card is parked: unlock says where
/// the person does it, and a change says it needs an approval this Biorouter can't ask for.
#[test]
#[serial]
fn without_proof_there_is_no_card() {
    run(async {
        const WORKSPACE: &str = "6b1f0a52-3c3d-4f5e-9a10-0000000e0002";
        const HOST: &str = "vault-no-proof.example.test";
        vault().await;
        let connection = workspace(WORKSPACE, HOST).await;
        crew().disconnect(&connection.id).await.unwrap();
        lock().await;
        let session = chat("vault without proof").await;
        let restore = ProofRestored;
        biorouter::pending_user_action::set_user_proof_available(false);

        let unlock = call(
            &session,
            public(),
            PersonTool::Change,
            "credentials_unlock",
            json!({}),
            &[],
        )
        .await;
        assert!(unlock.cards.is_empty(), "{:?}", unlock.cards);
        assert!(
            unlock
                .refusal()
                .contains("biorouter crew credentials unlock"),
            "{}",
            unlock.refusal()
        );

        let connect = call(
            &session,
            public(),
            PersonTool::Change,
            "connect",
            json!({"connection": connection.id}),
            &[],
        )
        .await;
        assert!(connect.cards.is_empty(), "{:?}", connect.cards);
        assert!(
            connect.refusal().starts_with("This needs your approval"),
            "{}",
            connect.refusal()
        );
        assert_eq!(credentials().await["locked"], true);
        drop(restore);
        remove(&connection).await;
    });
}

// ---------------------------------------------------------------------------------------------
// The host seam: `vault::run_unlocked`, called by `ChatCrewHost::call` in place of
// `run(&ctx)` (`routes/crew_tools/mod.rs`). A read, and a change that resolves its targets
// through the workspace (a send), need the device key before they reach any approval, so only
// that seam can ask for a locked vault in time.
// ---------------------------------------------------------------------------------------------

/// A read through a locked vault asks for the vault, then for the read consent, then answers.
#[test]
#[serial]
fn a_read_through_a_locked_vault_asks_for_it_first() {
    run(async {
        const WORKSPACE: &str = "6b1f0a52-3c3d-4f5e-9a10-0000000e0003";
        const HOST: &str = "vault-read.example.test";
        vault().await;
        let connection = workspace(WORKSPACE, HOST).await;
        lock().await;
        let session = chat("vault before read").await;
        let read = call(
            &session,
            private_local().await,
            PersonTool::Read,
            "workspace_show",
            json!({}),
            &[Reply::Unlock, Reply::Allow],
        )
        .await;
        assert_eq!(read.kinds(), ["crewStep", "toolConfirmation"]);
        assert_eq!(read.ok()["workspace"]["name"], "lab", "{}", read.ok());
        read.assert_no_passphrase();
        assert_no_passphrase(&stored(&session).await);
        remove(&connection).await;
    });
}

/// A send through a locked vault parks the vault card first and then exactly one approval card,
/// and posts once.
#[test]
#[serial]
fn a_locked_vault_before_a_send_parks_the_vault_card_then_one_approval() {
    run(async {
        const WORKSPACE: &str = "6b1f0a52-3c3d-4f5e-9a10-0000000e0004";
        const HOST: &str = "vault-send.example.test";
        vault().await;
        let connection = workspace(WORKSPACE, HOST).await;
        lock().await;
        let session = private_chat("vault before send").await;
        let posted = call(
            &session,
            private_local().await,
            PersonTool::Change,
            "send",
            json!({"connection": connection.id, "channel": "#general", "text": "hello"}),
            &[Reply::Unlock, Reply::Allow],
        )
        .await;
        assert_eq!(posted.kinds(), ["crewStep", "toolConfirmation"]);
        assert_eq!(posted.ok()["message_id"], "m-new", "{}", posted.ok());
        assert_eq!(sent(HOST, "message.post").len(), 1);
        posted.assert_no_passphrase();
        remove(&connection).await;
    });
}
