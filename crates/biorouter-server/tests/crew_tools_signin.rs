//! Signing in from a chat (the owner's D1 ruling on AO-12), on the daemon's side: the host the
//! daemon installs (`routes::crew_tools`), driven one `PersonToolCall` at a time as the `crew`
//! extension drives it, against the real `CrewManager`, the real setup jobs, the real sign-in
//! terminal and handoff, and the real approval registry and routes.
//!
//! What is pinned here:
//!
//! - a setup job that waits for a sign-in parks exactly one Sign in card in the calling chat,
//!   written from the job's own server and login, and the call carries on once the job does;
//! - nothing can answer the card as done: a forged allow is refused and changes nothing;
//! - Dismiss and Stop take the card down with nothing signed in, and leave the job waiting;
//! - without a person's proof (`biorouter serve`) no card is parked and the step is said;
//! - a connect whose server asks for a sign-in shows the card after its one approval, and
//!   connects once the sign-in is adopted, with no second approval;
//! - the call's 20-second wait does not run out while the card is up;
//! - the answers carry the job's and the connection's state, never what was typed.
//!
//! **Offline.** The daemon's `ssh` is this binary, as in `crew_tools_host.rs`: a `#[ctor]` puts
//! a two-line `ssh` script first on `PATH` that runs this executable as a fake OpenSSH. Per host
//! it answers as a server that wants a password (`need-auth`), refuses the key (`refuse-key`),
//! runs a Crew bridge (a scenario), or runs nothing (a setup job's server), and `-M` stands in
//! for the master a person's sign-in starts. **Its own binary**: the host and the user-action
//! digest are process-global, and every test runs on one shared runtime.
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
use biorouter::crew::{SetupState, SetupStatus};
use biorouter::pending_user_action::{set_user_proof_available, PendingUserActions};
use biorouter::privacy::CallCapability;
use biorouter::session::session_manager::SessionType;
use biorouter_server::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serial_test::serial;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

/// Set only in the fake `ssh` process: the fixture directory it reads scenarios from.
const FAKE_SSH_ROOT: &str = "BIOROUTER_CREW_TOOLS_SIGNIN_FAKE_SSH";
/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-tools-signin-user-action-key";
/// The daemon secret the full router checks.
const SECRET: &str = "crew-tools-signin-secret";
const HOST_UID: u32 = 1000;
const JOINER_UID: u32 = 1001;
const SOCKET: &str = "/tmp/crew-1000-0123456789abcdef0123456789abcdef/broker.sock";
const ACCOUNT_CAPABILITIES: [&str; 4] = [
    "human_names_v1",
    "unique_names_v1",
    "join_by_name_v1",
    "account_identity_v1",
];
/// Longer than the call's own wait (`JOB_WAIT`, 20 seconds), which must not run out while the
/// person is at the card.
const LONGER_THAN_THE_CALL_WAITS: Duration = Duration::from_secs(22);

// ---------------------------------------------------------------------------------------------
// The workspace key, signed with `rustls`' own Ed25519, as in `crew_tools_host.rs`.
// ---------------------------------------------------------------------------------------------

mod workspace_key {
    use rustls::crypto::ring::sign::any_eddsa_type;
    use rustls::pki_types::PrivatePkcs8KeyDer;
    use rustls::sign::SigningKey;
    use rustls::SignatureScheme;
    use std::sync::Arc;

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

    pub fn workspace() -> Key {
        Key::from_seed([9; 32])
    }
}

// ---------------------------------------------------------------------------------------------
// Scenarios and flags: what the fake server does, per SSH host.
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
    answers: serde_json::Map<String, Value>,
}

impl Scenario {
    fn new(workspace_id: &str) -> Self {
        Self {
            workspace_id: workspace_id.into(),
            node_id: hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
                workspace_id.as_bytes(),
            )),
            name: Some("lab".into()),
            mode: "private".into(),
            institution_id: Some("ucsf".into()),
            policy_epoch: 1,
            capabilities: ACCOUNT_CAPABILITIES
                .iter()
                .map(|c| (*c).to_owned())
                .collect(),
            answers: serde_json::Map::new(),
        }
    }
}

fn scenario_path(root: &Path, host: &str) -> PathBuf {
    root.join("scenarios").join(format!("{host}.json"))
}

/// A per-host file: a flag (`need-auth`, `refuse-key`) or the signed-in master's pid (`master`).
fn host_file(root: &Path, host: &str, name: &str) -> PathBuf {
    root.join("scenarios").join(format!("{host}.{name}"))
}

/// Write `scenario` for `host`, atomically, so the fake never reads half a file.
fn stage(host: &str, scenario: &Scenario) {
    let path = scenario_path(&fixture_root(), host);
    let partial = path.with_extension("partial");
    fs::write(&partial, serde_json::to_vec(scenario).unwrap()).unwrap();
    fs::rename(partial, path).unwrap();
}

/// Make `host` a server that takes a password or a code only (`need-auth`), or one that refuses
/// this computer's key (`refuse-key`).
fn flag(host: &str, name: &str) {
    fs::write(host_file(&fixture_root(), host, name), "").unwrap();
}

struct MasterCleanup(&'static str);

impl Drop for MasterCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(host_file(&fixture_root(), self.0, "master"));
    }
}

// ---------------------------------------------------------------------------------------------
// The fake `ssh`, run by this same binary before `main`.
// ---------------------------------------------------------------------------------------------

mod fake_ssh {
    use super::{host_file, scenario_path, workspace_key, Scenario, JOINER_UID};
    use serde_json::{json, Value};
    use std::io::{BufRead, Write};
    use std::path::Path;
    use std::time::Duration;

    const WITH_VALUE: &[&str] = &[
        "-B", "-b", "-c", "-D", "-E", "-e", "-F", "-I", "-i", "-J", "-L", "-l", "-m", "-O", "-o",
        "-p", "-Q", "-R", "-S", "-W", "-w",
    ];

    fn target(args: &[String]) -> Option<&str> {
        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            if WITH_VALUE.contains(&arg.as_str()) {
                rest.next();
            } else if arg == "--" {
                return rest.next().map(String::as_str);
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

    /// The pid of `host`'s signed-in master, while it runs.
    fn live_master(root: &Path, host: &str) -> Option<i32> {
        let pid: i32 = std::fs::read_to_string(host_file(root, host, "master"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        (unsafe { libc::kill(pid, 0) } == 0).then_some(pid)
    }

    pub fn run(root: &Path) -> i32 {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let Some(login) = target(&args).map(str::to_owned) else {
            eprintln!("fake ssh: no destination in {args:?}");
            return 255;
        };
        let host = host_of(&login).to_owned();
        if args.first().map(String::as_str) == Some("-G") {
            return settings(&host);
        }
        if let Some(at) = args.iter().position(|arg| arg == "-O") {
            return control(root, &host, args.get(at + 1).map_or("", String::as_str));
        }
        if args.iter().any(|arg| arg == "-M") {
            return master(root, &host);
        }
        if host_file(root, &host, "unknown-host-key").exists()
            || (host_file(root, &host, "unknown-host-key-after-master").exists()
                && live_master(root, &host).is_some())
        {
            eprintln!(
                "No ED25519 host key is known for {host} and you have requested strict checking."
            );
            eprintln!("Host key verification failed.");
            return 255;
        }
        if host_file(root, &host, "refuse-key").exists() {
            eprintln!("{login}: Permission denied (publickey).");
            return 255;
        }
        if host_file(root, &host, "need-auth").exists() && live_master(root, &host).is_none() {
            eprintln!("{login}: Permission denied (publickey,password).");
            return 255;
        }
        if load(root, &host).is_some() {
            return bridge(root, &host);
        }
        // A server with no Crew bridge: a setup job's commands get nothing they can read.
        1
    }

    fn settings(host: &str) -> i32 {
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
        0
    }

    /// `-O check` reports the signed-in master as OpenSSH does; `-O exit` ends it.
    fn control(root: &Path, host: &str, command: &str) -> i32 {
        match (command, live_master(root, host)) {
            ("check", Some(pid)) => {
                eprintln!("Master running (pid={pid})");
                0
            }
            ("check", None) => 255,
            ("exit", Some(pid)) => {
                unsafe { libc::kill(pid, libc::SIGTERM) };
                let _ = std::fs::remove_file(host_file(root, host, "master"));
                0
            }
            _ => 0,
        }
    }

    /// The master a person's sign-in starts: as if the password was typed and taken, it runs
    /// until it is ended, its record is removed, or the test process is gone.
    fn master(root: &Path, host: &str) -> i32 {
        let record = host_file(root, host, "master");
        if std::fs::write(&record, std::process::id().to_string()).is_err() {
            return 255;
        }
        for _ in 0..1200 {
            std::thread::sleep(Duration::from_millis(100));
            if !record.exists() || unsafe { libc::getppid() } == 1 {
                break;
            }
        }
        0
    }

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
// The fixture directory, `PATH` and credential backend, set before `main`.
// ---------------------------------------------------------------------------------------------

fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "biorouter-crew-tools-signin-{}",
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
        for directory in ["bin", "profile/home/.ssh", "scenarios"] {
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
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700))?;
        let keyscan = root.join("bin").join("ssh-keyscan");
        fs::write(
            &keyscan,
            r#"#!/bin/sh
for arg in "$@"; do host="$arg"; done
if [ "$host" = "signin-invitation-key.example.test" ]; then
  printf '%s\n' 'signin-invitation-key.example.test ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDVMyBQ3psSwstgc8+Mce5VHIZN5GFuu3xE6WV8pfFkD'
  exit 0
fi
exit 1
"#,
        )?;
        fs::set_permissions(&keyscan, fs::Permissions::from_mode(0o700))
    })();
    if let Err(error) = prepared {
        eprintln!(
            "crew_tools_signin: could not prepare the fixture at {}: {error}. Refusing to run: \
             the daemon would reach the real ssh and the OS keychain.",
            root.display()
        );
        std::process::abort();
    }
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{path}", root.join("bin").display()));
    std::env::set_var("BIOROUTER_DISABLE_KEYRING", "true");
    std::env::set_var("BIOROUTER_DEV_PROFILE_ROOT", root.join("profile"));
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

async fn chat(name: &str) -> String {
    state()
        .await
        .session_manager()
        .create_session(
            std::env::temp_dir().join("crew-tools-signin"),
            name.to_owned(),
            SessionType::User,
        )
        .await
        .expect("a chat")
        .id
}

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

/// A workspace on `host`, saved on this computer from its invitation and not connected.
async fn saved_workspace(workspace_id: &str, host: &str) -> biorouter::crew::Connection {
    stage(host, &Scenario::new(workspace_id));
    match crew()
        .connection_from_invitation(&invitation(workspace_id, host), false, Default::default())
        .await
        .expect("the invitation saves")
    {
        biorouter::crew::authentication::InvitationOutcome::Saved(connection) => *connection,
        biorouter::crew::authentication::InvitationOutcome::Preview(_) => panic!("a preview"),
    }
}

// ---------------------------------------------------------------------------------------------
// A call, and the person at its cards
// ---------------------------------------------------------------------------------------------

/// One person-tool call under way, and the cards it parked so far.
struct Running {
    session: String,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<Result<Value, String>>,
    seen: HashSet<String>,
    cards: Vec<Value>,
}

impl Running {
    async fn start(session: &str, tool: PersonTool, action: &str, params: Value) -> Self {
        let host = tool_host::get().expect("the host is installed");
        let cancel = CancellationToken::new();
        let call = PersonToolCall {
            tool,
            action: action.to_owned(),
            params,
            session_id: session.to_owned(),
            tool_call_id: Some(format!("call-{}", uuid::Uuid::new_v4())),
            capability: CallCapability::public_enforced(),
            cancel: cancel.clone(),
            approvals: Arc::clone(PendingUserActions::global()),
        };
        let task =
            tokio::spawn(async move { host.call(call).await.map_err(|error| error.to_string()) });
        Self {
            session: session.to_owned(),
            cancel,
            task,
            seen: HashSet::new(),
            cards: Vec::new(),
        }
    }

    /// The next card the call parks, as the published message carries it (`data`). Panics if
    /// none comes, or the call answers first.
    async fn next_card(&mut self) -> Value {
        self.next_card_or_finished()
            .await
            .expect("the call answered before parking another card")
    }

    async fn next_card_or_finished(&mut self) -> Option<Value> {
        for _ in 0..1500 {
            for message in PendingUserActions::global().pending_cards_for_session(&self.session) {
                let card = serde_json::to_value(&message).unwrap();
                let data = card["content"][0]["data"].clone();
                let id = data["id"].as_str().unwrap_or_default().to_owned();
                if !id.is_empty() && self.seen.insert(id) {
                    self.cards.push(data.clone());
                    return Some(data);
                }
            }
            if self.task.is_finished() {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("no card came");
    }

    /// Allow the approval card `card` as the person's click does.
    async fn allow(&self, card: &Value) {
        assert_eq!(card["actionType"], "toolConfirmation", "{card}");
        let (status, body) = http(
            "POST",
            "/action-required/tool-confirmation",
            Some(
                json!({"id": card["id"], "principalType": "Tool", "action": "allow_once",
                "sessionId": self.session}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    /// The call's answer, as the model reads it.
    async fn answer(self) -> (Result<Value, String>, Vec<Value>) {
        let answer = tokio::time::timeout(Duration::from_secs(60), self.task)
            .await
            .expect("the call answered")
            .expect("the call ran");
        (answer, self.cards)
    }
}

/// The setup job a check of `login` started, as the daemon has it.
fn job_for(login: &str) -> SetupStatus {
    biorouter::crew::setup_jobs()
        .into_iter()
        .find(|job| job.server.login == login)
        .expect("a setup job for the login")
}

/// Sign in to the setup job `job_id` as a person does through its Sign in window: prepare it,
/// open its terminal (the fake's `-M` stands in for typing the password), and hand it off.
/// Answers the terminal's stream, which must outlive the sign-in.
async fn sign_in_to_job(
    job_id: &str,
) -> tokio::sync::mpsc::Receiver<biorouter::crew::authentication::TerminalEvent> {
    let controller = uuid::Uuid::new_v4().to_string();
    let session = biorouter::crew::prepare_setup_sign_in(
        job_id,
        &uuid::Uuid::new_v4().to_string(),
        &controller,
        80,
        24,
    )
    .await
    .expect("the job's sign-in prepares");
    handed_off(&session.authentication_id, &controller).await
}

/// [`sign_in_to_job`] for a saved connection.
async fn sign_in_to_connection(
    connection_id: &str,
) -> tokio::sync::mpsc::Receiver<biorouter::crew::authentication::TerminalEvent> {
    let controller = uuid::Uuid::new_v4().to_string();
    let session = biorouter::crew::authentication::prepare(
        connection_id,
        &uuid::Uuid::new_v4().to_string(),
        &controller,
        80,
        24,
    )
    .await
    .expect("the connection's sign-in prepares");
    handed_off(&session.authentication_id, &controller).await
}

async fn handed_off(
    id: &str,
    controller: &str,
) -> tokio::sync::mpsc::Receiver<biorouter::crew::authentication::TerminalEvent> {
    let output = biorouter::crew::authentication::attach(id, controller)
        .await
        .expect("the sign-in terminal opens");
    for _ in 0..400 {
        if biorouter::crew::authentication::handoff(id, controller)
            .await
            .expect("the sign-in hands off")
        {
            return output;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the sign-in was never adopted");
}

/// Every key in `value`, at any depth.
fn keys(value: &Value, into: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                into.push(key.clone());
                keys(inner, into);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| keys(item, into)),
        _ => {}
    }
}

/// The answer carries state, never a value: no key anywhere could hold what was typed.
fn holds_no_value(answer: &Value) {
    let mut found = Vec::new();
    keys(answer, &mut found);
    for forbidden in ["data", "input", "output"] {
        assert!(
            !found.iter().any(|key| key == forbidden),
            "the answer has a {forbidden} key: {answer}"
        );
    }
}

/// Wait for `job` to stop waiting for a sign-in in the state it is in, so a test leaves no job
/// running for the next.
async fn end_job(job: &str) {
    biorouter::crew::cancel_setup(job);
    for _ in 0..250 {
        if biorouter::crew::setup_status(job).is_none_or(|status| {
            !matches!(status.state, SetupState::Running | SetupState::NeedsSignIn)
        }) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Puts a person's proof back when dropped, so a test that takes it away (`biorouter serve`)
/// leaves it for the next test even when one of its assertions fails.
struct ProofRestored;

impl Drop for ProofRestored {
    fn drop(&mut self) {
        set_user_proof_available(true);
    }
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

/// A check whose server wants a password parks one approval, then one Sign in card written
/// from the job's own server and login. Nothing can answer that card as done, the call's wait
/// does not run out while it is up, and once the person signs in through the job's window the
/// card comes down and the call answers where the job went, with state and nothing typed.
#[test]
#[serial]
fn a_setup_job_that_waits_for_a_sign_in_shows_one_card_and_carries_on() {
    run(async {
        const HOST: &str = "signin-setup.example.test";
        let login = format!("wanjun@{HOST}");
        flag(HOST, "need-auth");
        let session = chat("setup sign in").await;
        let mut call = Running::start(
            &session,
            PersonTool::Read,
            "setup_check",
            json!({"server": login}),
        )
        .await;
        let approval = call.next_card().await;
        call.allow(&approval).await;
        let card = call.next_card().await;
        let job = job_for(&login);
        assert_eq!(job.state, SetupState::NeedsSignIn);
        assert_eq!(card["actionType"], "crewStep", "{card}");
        assert_eq!(
            card["step"],
            json!({
                "kind": "signIn",
                "target": {"kind": "setup", "jobId": job.job_id},
                "server": biorouter::crew::server_label(&job.server.login, None).await,
                "login": job.server.login,
            })
        );
        let id = card["id"].as_str().unwrap().to_owned();

        // A forged allow, through the route with the person's own proof: the route takes the
        // proof (a 200, not the 403 an unproven answer gets), and the registry still refuses an
        // allow for a card that can only be dismissed, so nothing was delivered and nothing
        // changed. (The registry's own refusal is pinned where `CrewStep` is defined, in
        // `pending_user_action`; a test here mints no privileged authority of its own.)
        let (status, forged) = http(
            "POST",
            "/action-required/tool-confirmation",
            Some(
                json!({"id": id, "principalType": "Tool", "action": "allow_once",
                "sessionId": session}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{forged}");
        assert_eq!(forged["status"], "unknown", "{forged}");
        assert!(PendingUserActions::global().is_pending(&id));
        assert_eq!(
            biorouter::crew::setup_status(&job.job_id).unwrap().state,
            SetupState::NeedsSignIn
        );

        // The person takes their time: the call's own wait does not run out meanwhile.
        tokio::time::sleep(LONGER_THAN_THE_CALL_WAITS).await;
        assert!(
            !call.task.is_finished(),
            "the call answered while the card was up"
        );
        assert!(PendingUserActions::global().is_pending(&id));

        let _terminal = sign_in_to_job(&job.job_id).await;
        let (answer, cards) = call.answer().await;
        let answer = answer.expect("the call answers the job");
        assert!(!PendingUserActions::global().is_pending(&id));
        assert_eq!(
            cards.len(),
            2,
            "one approval and one Sign in card: {cards:?}"
        );
        assert_eq!(answer["job_id"], job.job_id);
        assert_ne!(answer["state"], "needs_sign_in", "{answer}");
        assert!(answer
            .get("note")
            .is_none_or(|note| !note.as_str().unwrap_or_default().contains("Still running")));
        holds_no_value(&answer);

        // Asked again, the job waits for nothing.
        let asked = Running::start(
            &session,
            PersonTool::Change,
            "auth",
            json!({"job_id": job.job_id}),
        )
        .await;
        let (asked, cards) = asked.answer().await;
        let asked = asked.unwrap();
        assert!(cards.is_empty());
        assert_eq!(
            asked["note"],
            "Nothing waits for a sign-in: this setup isn't waiting for one."
        );
        end_job(&job.job_id).await;
    });
}

/// A setup job can fail on its host key while the wait loop is still waiting for its sign-in
/// card. The fresh failed state must reach the host-key hook before the call answers.
#[test]
#[serial]
fn a_host_key_failure_after_sign_in_reaches_the_host_key_hook() {
    run(async {
        const HOST: &str = "signin-after-auth-key.example.test";
        let login = format!("wanjun@{HOST}");
        flag(HOST, "need-auth");
        flag(HOST, "unknown-host-key-after-master");
        let _master_cleanup = MasterCleanup(HOST);
        let session = chat("sign in then host key").await;
        let mut call = Running::start(
            &session,
            PersonTool::Read,
            "setup_check",
            json!({"server": login, "proxy_jump": "gate.example.test"}),
        )
        .await;
        let approval = call.next_card().await;
        call.allow(&approval).await;
        let sign_in = call.next_card().await;
        assert_eq!(sign_in["step"]["kind"], "signIn", "{sign_in}");
        let job = job_for(&login);
        assert_eq!(job.state, SetupState::NeedsSignIn);

        let _terminal = sign_in_to_job(&job.job_id).await;
        let host_key = call.next_card_or_finished().await;
        assert_eq!(
            biorouter::crew::setup_status(&job.job_id).unwrap().state,
            SetupState::Failed
        );
        let offered = if let Some(host_key) = &host_key {
            assert_eq!(host_key["actionType"], "crewStep", "{host_key}");
            assert_eq!(host_key["step"]["kind"], "hostKey", "{host_key}");
            assert_eq!(host_key["step"]["login"], login, "{host_key}");
            assert_eq!(host_key["step"]["host"], HOST, "{host_key}");
            assert!(host_key["step"].get("offered").is_none());
            let (status, body) = http(
                "POST",
                "/action-required/crew-step",
                Some(json!({"id": host_key["id"], "session_id": session, "outcome": "dismiss"})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            true
        } else {
            false
        };
        let (answer, cards) = call.answer().await;
        end_job(&job.job_id).await;
        let _ = fs::remove_file(host_file(&fixture_root(), HOST, "master"));
        assert!(
            offered,
            "the fresh failed setup state must reach the host-key hook"
        );
        assert!(answer.is_ok(), "{answer:?}");
        assert_eq!(cards.len(), 3, "approval, sign-in, host-key: {cards:?}");
    });
}

/// The model names a saved connection by its name, and the card still carries only what the
/// daemon has: the job's own server and login. Dismissed, the card comes down with nothing
/// signed in, and the job still waits; the answer says so, with its next step offering the card
/// again.
#[test]
#[serial]
fn dismissing_the_sign_in_card_signs_nothing_in() {
    run(async {
        const HOST: &str = "signin-dismiss.example.test";
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0003";
        let connection = saved_workspace(WORKSPACE, HOST).await;
        flag(HOST, "need-auth");
        let session = chat("dismiss").await;
        // A saved connection's check asks for no approval: the first card is the Sign in card.
        let mut call = Running::start(
            &session,
            PersonTool::Read,
            "setup_check",
            json!({"connection": connection.name}),
        )
        .await;
        let card = call.next_card().await;
        let job = job_for(&connection.ssh_target);
        assert_eq!(
            card["step"],
            json!({
                "kind": "signIn",
                "target": {"kind": "setup", "jobId": job.job_id},
                "server": biorouter::crew::server_label(&job.server.login, connection.port).await,
                "login": job.server.login,
            })
        );
        assert_ne!(card["step"]["login"], json!(connection.name));
        let (status, body) = http(
            "POST",
            "/action-required/crew-step",
            Some(json!({"id": card["id"], "session_id": session, "outcome": "dismiss"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (answer, cards) = call.answer().await;
        let answer = answer.unwrap();
        assert_eq!(cards.len(), 1, "one Sign in card: {cards:?}");
        let job = job_for(&connection.ssh_target);
        assert_eq!(job.state, SetupState::NeedsSignIn);
        assert_eq!(answer["state"], "needs_sign_in");
        assert_eq!(answer["code"], "crew_ssh_auth_required");
        let note = answer["note"].as_str().unwrap();
        assert!(note.contains("closed the Sign in card"), "{note}");
        assert!(note.contains("still waits"), "{note}");
        let step = answer["next_step"].as_str().unwrap();
        assert!(
            step.contains(&format!(
                "crew__change action auth with job_id {}",
                job.job_id
            )),
            "{step}"
        );
        holds_no_value(&answer);
        end_job(&job.job_id).await;
        let _ = crew().remove(&connection.id).await;
    });
}

/// Stop takes the card down and leaves the job waiting for its sign-in.
#[test]
#[serial]
fn stopping_the_chat_takes_the_card_down_and_leaves_the_job() {
    run(async {
        const HOST: &str = "signin-stop.example.test";
        let login = format!("wanjun@{HOST}");
        flag(HOST, "need-auth");
        let session = chat("stop").await;
        let mut call = Running::start(
            &session,
            PersonTool::Read,
            "setup_check",
            json!({"server": login}),
        )
        .await;
        let approval = call.next_card().await;
        call.allow(&approval).await;
        let card = call.next_card().await;
        let id = card["id"].as_str().unwrap().to_owned();
        call.cancel.cancel();
        let (answer, _) = call.answer().await;
        let answer = answer.unwrap();
        assert!(!PendingUserActions::global().is_pending(&id));
        let job = job_for(&login);
        assert_eq!(job.state, SetupState::NeedsSignIn, "Stop left the job");
        assert_eq!(answer["state"], "needs_sign_in");
        holds_no_value(&answer);
        end_job(&job.job_id).await;
    });
}

/// `biorouter serve`: no person's proof, so no card. The job's answer names the step in words,
/// as before.
#[test]
#[serial]
fn without_a_person_s_proof_no_sign_in_card_is_parked() {
    run(async {
        const HOST: &str = "signin-serve.example.test";
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0004";
        let connection = saved_workspace(WORKSPACE, HOST).await;
        flag(HOST, "need-auth");
        let session = chat("serve").await;
        // Put back however the test ends, a failed assertion included, so no later test in this
        // binary runs without a person's proof for an unrelated reason.
        let _restore = ProofRestored;
        set_user_proof_available(false);
        let call = Running::start(
            &session,
            PersonTool::Read,
            "setup_check",
            json!({"connection": connection.name}),
        )
        .await;
        let (answer, cards) = call.answer().await;
        let answer = answer.unwrap();
        assert!(cards.is_empty(), "{cards:?}");
        assert!(!PendingUserActions::global().has_pending_in_session(&session));
        assert_eq!(answer["state"], "needs_sign_in");
        let job = answer["job_id"].as_str().unwrap().to_owned();
        assert_eq!(
            answer["next_step"],
            format!(
                "The person signs in: open Crew, where this job waits with a Sign in button, or run biorouter crew auth --setup-job {job}. Then follow it with crew__read action setup_status."
            )
        );
        // Asked for by name, the sign-in still opens no card: `auth` says where it can be done.
        let asked =
            Running::start(&session, PersonTool::Change, "auth", json!({"job_id": job})).await;
        let (asked, cards) = asked.answer().await;
        assert!(cards.is_empty(), "{cards:?}");
        assert_eq!(
            asked.unwrap_err(),
            format!(
                "No Sign in window can open for this chat. The person signs in from the Crew view of the desktop app, or runs biorouter crew auth --setup-job {job} in a terminal."
            )
        );
        end_job(&job).await;
        let _ = crew().remove(&connection.id).await;
    });
}

/// A connect whose server wants a password parks its one approval, then the Sign in card for
/// the saved connection. The call's wait does not run out while the card is up; once the
/// person's sign-in is adopted, the card comes down and the call answers connected, with no
/// second approval.
#[test]
#[serial]
fn a_connect_that_needs_a_sign_in_connects_once_the_person_signs_in() {
    run(async {
        const HOST: &str = "signin-connect.example.test";
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0005";
        let connection = saved_workspace(WORKSPACE, HOST).await;
        flag(HOST, "need-auth");
        let session = chat("connect").await;
        let mut call = Running::start(
            &session,
            PersonTool::Change,
            "connect",
            json!({"connection": connection.name}),
        )
        .await;
        let approval = call.next_card().await;
        call.allow(&approval).await;
        let card = call.next_card().await;
        assert_eq!(card["actionType"], "crewStep", "{card}");
        assert_eq!(
            card["step"],
            json!({
                "kind": "signIn",
                "target": {"kind": "connection", "connectionId": connection.id},
                "server": biorouter::crew::server_label(&connection.ssh_target, connection.port).await,
                "login": connection.ssh_target,
            })
        );
        let id = card["id"].as_str().unwrap().to_owned();
        tokio::time::sleep(LONGER_THAN_THE_CALL_WAITS).await;
        assert!(
            !call.task.is_finished(),
            "the call answered while the card was up"
        );
        assert!(PendingUserActions::global().is_pending(&id));

        let _terminal = sign_in_to_connection(&connection.id).await;
        let (answer, cards) = call.answer().await;
        let answer = answer.expect("connected");
        assert!(!PendingUserActions::global().is_pending(&id));
        assert_eq!(answer["status"], "connected", "{answer}");
        let approvals = cards
            .iter()
            .filter(|card| card["actionType"] == "toolConfirmation")
            .count();
        assert_eq!(approvals, 1, "no second approval: {cards:?}");
        assert_eq!(cards.len(), 2, "{cards:?}");
        assert_eq!(
            crew().connection(&connection.id).await.unwrap().status,
            "connected"
        );
        let _ = crew().disconnect(&connection.id).await;
        let _ = crew().remove(&connection.id).await;
    });
}

/// An invitation join has no named connection parameter. Its saved route still supplies the
/// login for the unknown-host-key card, without asking the model to name a route.
#[test]
#[serial]
fn an_invitation_join_uses_its_saved_route_for_the_host_key_card() {
    run(async {
        const HOST: &str = "signin-invitation-key.example.test";
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0099";
        flag(HOST, "unknown-host-key");
        let session = chat("invitation host key").await;
        let mut call = Running::start(
            &session,
            PersonTool::Change,
            "join",
            json!({
                "invitation": invitation(WORKSPACE, HOST),
            }),
        )
        .await;
        let approval = call.next_card().await;
        call.allow(&approval).await;
        let host_key = call.next_card().await;
        let action_type = host_key["actionType"].clone();
        let step = host_key["step"].clone();
        let offered = step["offered"].clone();

        let (status, body) = http(
            "POST",
            "/action-required/crew-step",
            Some(json!({"id": host_key["id"], "session_id": session, "outcome": "dismiss"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (answer, cards) = call.answer().await;
        let saved = crew()
            .list()
            .await
            .into_iter()
            .find(|connection| connection.workspace_id == WORKSPACE)
            .expect("the invitation saved its exact workspace");
        crew().remove(&saved.id).await.unwrap();
        assert_eq!(action_type, "crewStep");
        assert_eq!(step["kind"], "hostKey");
        assert_eq!(step["login"], format!("alice@{HOST}"));
        assert_eq!(step["host"], HOST);
        assert_eq!(step["port"], 22);
        assert_eq!(
            offered,
            "SHA256:XA01Ou0eChdPDyZiadGxCZ/g+7RnODImHcZEtuRmV8g"
        );
        assert!(answer.is_err());
        assert_eq!(cards.len(), 2, "approval and host-key: {cards:?}");
    });
}

/// A refused key may be one with a passphrase: the refusal offers the Sign in window, and
/// `auth` on that connection then shows it. A dismissed card signs nothing in.
#[test]
#[serial]
fn a_refused_key_offers_the_sign_in_window() {
    run(async {
        const HOST: &str = "signin-key.example.test";
        const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0006";
        let connection = saved_workspace(WORKSPACE, HOST).await;
        flag(HOST, "refuse-key");
        let session = chat("key").await;
        let mut call = Running::start(
            &session,
            PersonTool::Change,
            "connect",
            json!({"connection": connection.name}),
        )
        .await;
        let approval = call.next_card().await;
        call.allow(&approval).await;
        let (answer, cards) = call.answer().await;
        let refusal = answer.unwrap_err();
        assert_eq!(
            cards.len(),
            1,
            "no Sign in card for a refused key: {cards:?}"
        );
        assert!(
            refusal.contains("(code: crew_ssh_key_refused)"),
            "{refusal}"
        );
        assert!(
            refusal.contains(&format!(
                "crew__change action auth with connection {}",
                connection.name
            )),
            "{refusal}"
        );
        assert!(refusal.contains("the card that opens"), "{refusal}");

        let mut asked = Running::start(
            &session,
            PersonTool::Change,
            "auth",
            json!({"connection": connection.name}),
        )
        .await;
        let card = asked.next_card().await;
        assert_eq!(card["actionType"], "crewStep", "{card}");
        assert_eq!(card["step"]["target"]["connectionId"], connection.id);
        let (status, _) = http(
            "POST",
            "/action-required/crew-step",
            Some(json!({"id": card["id"], "session_id": session, "outcome": "dismiss"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (answer, cards) = asked.answer().await;
        assert_eq!(cards.len(), 1, "auth asks for no approval: {cards:?}");
        assert_eq!(
            answer.unwrap_err(),
            "The person closed the Sign in card, so nothing was signed in."
        );
        let _ = crew().remove(&connection.id).await;
    });
}
