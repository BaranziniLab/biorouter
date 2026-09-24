//! Workspace admission over HTTP (S3a): `POST /crew/connections/from-invitation`,
//! `GET /crew/connections/{id}/invitation`, and `GET`/`POST /crew/connections/{id}/join`, as
//! `docs/research/biorouter-crew/naming-design.md` ("Daemon routes, OpenAPI and TypeScript
//! client") specifies them.
//!
//! **Offline.** Nothing here reaches a network or a real broker. The daemon's `ssh` is this
//! binary: a `#[ctor]` puts a two-line `ssh` script first on `PATH`, and that script runs this
//! same executable with [`FAKE_SSH_ROOT`] set, which the ctor recognizes before `main` and turns
//! into a fake OpenSSH (`-G` prints settings the Crew SSH policy accepts) and a fake bridge that
//! speaks the broker protocol. The bridge signs `hello` with a workspace key the tests know, so
//! the daemon's real identity verification runs, and it answers everything else from a
//! per-host scenario file each test writes, logging every frame it receives. That lets a test
//! play a hostile bridge: an answer that carries its own device code, a capability it does not
//! have, a refusal whose words must not reach the person.
//!
//! **Sandboxed.** `test_sandbox` points the Biorouter config root at a throwaway directory, and
//! the ctor selects the development file credential backend under this binary's own fixture
//! directory, so no test touches the operator's keychain, `~/.config/biorouter` or `~/.ssh`.
//! The user-action digest is a process-global `OnceLock`, so the "no key installed" answer is
//! pinned where it cannot race: the routes use the same three-way proof decision as every other
//! person-gated Crew route.
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
use futures::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serial_test::serial;
use std::{
    fs,
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use tower::ServiceExt;

/// Set only in the fake `ssh` process: the fixture directory it reads scenarios from.
const FAKE_SSH_ROOT: &str = "BIOROUTER_CREW_JOIN_ROUTES_FAKE_SSH";
/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-join-routes-user-action-key";
/// The host account's UID the invitations pin.
const HOST_UID: u32 = 1000;
/// The joiner's UID as the fake broker reports it in a challenge.
const JOINER_UID: u32 = 1001;
const SOCKET: &str = "/tmp/crew-1000-0123456789abcdef0123456789abcdef/broker.sock";
/// A device code a hostile bridge slips into its join status. Valid Crockford base32, so a
/// client that trusted it would show it.
const HOSTILE_CODE: &str = "ZZZZ-ZZZZ-ZZZZ-ZZZZ";
/// Words a hostile bridge puts in a refusal; they are for the daemon's log only.
const HOSTILE_WORDS: &str = "call 555-0100 and read me your password";

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
    std::env::temp_dir().join(format!("biorouter-crew-join-routes-{}", std::process::id()))
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
            "crew_join_routes: could not prepare the fixture at {}: {error}. Refusing to run: \
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
// HTTP helpers
// ---------------------------------------------------------------------------------------------

fn install_the_desktop_key() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
        biorouter_server::auth::install_user_action_digest(Some(digest));
    });
}

#[derive(Clone, Copy, Debug)]
enum Proof {
    /// No `X-User-Action`: what a model holding only the daemon secret sends.
    Absent,
    /// A key that is not the one whose digest this daemon holds.
    Wrong,
    /// The desktop app's proof that a person acted.
    Person,
}

fn app() -> Router {
    biorouter_server::routes::crew_authentication::routes()
}

async fn call_app(
    app: Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
    proof: Proof,
) -> (StatusCode, Value) {
    install_the_desktop_key();
    let mut request = Request::builder().method(method).uri(uri);
    match proof {
        Proof::Absent => {}
        Proof::Wrong => request = request.header("X-User-Action", "not-the-desktop-key"),
        Proof::Person => request = request.header("X-User-Action", USER_KEY),
    }
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, value)
}

async fn call(method: &str, uri: &str, body: Option<Value>, proof: Proof) -> (StatusCode, Value) {
    call_app(app(), method, uri, body, proof).await
}

fn crew() -> std::sync::Arc<biorouter::crew::CrewManager> {
    biorouter::crew::manager().expect("the sandboxed Crew manager")
}

/// Every saved connection to `workspace_id`.
async fn saved_for(workspace_id: &str) -> Vec<biorouter::crew::Connection> {
    crew()
        .list()
        .await
        .into_iter()
        .filter(|connection| connection.workspace_id == workspace_id)
        .collect()
}

/// Run `body`, then remove every connection it recorded in `created`, even when it panicked:
/// a connected bridge belongs to this test's runtime and must not outlive it.
async fn with_cleanup<F>(created: &Mutex<Vec<String>>, body: F)
where
    F: std::future::Future<Output = ()>,
{
    let outcome = AssertUnwindSafe(body).catch_unwind().await;
    let ids: Vec<String> = created.lock().unwrap().drain(..).collect();
    for id in ids {
        let _ = crew().remove(&id).await;
    }
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

// ---------------------------------------------------------------------------------------------
// Invitations
// ---------------------------------------------------------------------------------------------

fn workspace_invitation(
    workspace_id: &str,
    ssh_host: &str,
) -> biorouter_crew::invitation::WorkspaceInvitation {
    biorouter_crew::invitation::WorkspaceInvitation {
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
        invitee_username: Some("bob".into()),
    }
}

/// The host's whole message, as a joiner pastes it.
fn invitation_message(workspace_id: &str, ssh_host: &str) -> String {
    biorouter_crew::invitation::message(&workspace_invitation(workspace_id, ssh_host)).unwrap()
}

/// This computer's device code for `connection`, computed here from the pins and the saved
/// public key: what the daemon must show, whatever the workspace says.
fn local_code(connection: &Value) -> String {
    biorouter_crew::format_device_code(
        &biorouter_crew::device_code_from_hex(
            connection["workspace_id"].as_str().unwrap(),
            connection["workspace_public_key"].as_str().unwrap(),
            connection["public_key"].as_str().unwrap(),
        )
        .unwrap(),
    )
}

/// Save a joiner's connection from the invitation for `workspace_id` on `host`, and record it
/// for cleanup.
async fn save_joiner(created: &Mutex<Vec<String>>, workspace_id: &str, host: &str) -> Value {
    let (status, body) = call(
        "POST",
        "/crew/connections/from-invitation",
        Some(json!({"invitation": invitation_message(workspace_id, host)})),
        Proof::Person,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let connection = body["connection"].clone();
    created
        .lock()
        .unwrap()
        .push(connection["id"].as_str().unwrap().to_owned());
    connection
}

fn assert_refused(status: StatusCode, body: &Value, expected: StatusCode, code: &str) {
    assert_eq!(status, expected, "{body}");
    assert_eq!(body["code"], code, "{body}");
    assert!(
        body["error"].as_str().is_some_and(|text| !text.is_empty()),
        "every refusal says why in words: {body}"
    );
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

/// All four routes refuse a caller who cannot prove a person asked, before they read the body,
/// look up the connection or touch the workspace: a model holding the daemon secret gets
/// nothing, not even a malformed-body or unknown-connection answer.
#[tokio::test]
#[serial]
async fn every_admission_route_refuses_without_proof_of_a_person() {
    const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a1";
    let invitation = invitation_message(WORKSPACE, "hpc.example.org");
    let unknown = "00000000-0000-4000-8000-000000000000";
    let routes = [
        (
            "POST",
            "/crew/connections/from-invitation".to_owned(),
            Some(json!({"invitation": invitation, "preview": true})),
        ),
        (
            "POST",
            "/crew/connections/from-invitation".to_owned(),
            Some(json!({"invitation": invitation})),
        ),
        (
            "POST",
            "/crew/connections/from-invitation".to_owned(),
            Some(json!({"not_a_field": true})),
        ),
        (
            "GET",
            format!("/crew/connections/{unknown}/invitation?invitee=@bob"),
            None,
        ),
        ("GET", format!("/crew/connections/{unknown}/join"), None),
        ("POST", format!("/crew/connections/{unknown}/join"), None),
    ];
    for (method, uri, body) in routes {
        for proof in [Proof::Absent, Proof::Wrong] {
            let (status, answer) = call(method, &uri, body.clone(), proof).await;
            assert_refused(
                status,
                &answer,
                StatusCode::FORBIDDEN,
                "crew_user_action_required",
            );
            assert_eq!(
                answer.as_object().unwrap().len(),
                2,
                "{method} {uri} with {proof:?} answers only its code and words: {answer}"
            );
        }
    }
    assert!(
        saved_for(WORKSPACE).await.is_empty(),
        "an unproven save wrote a connection"
    );
}

/// A preview describes the invitation and the save it would make, in the shape both the desktop
/// and the CLI read, and writes nothing: no connection, no device key, no registry file.
#[tokio::test]
#[serial]
async fn a_preview_describes_the_invitation_and_saves_nothing() {
    const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a2";
    let crew_root = biorouter::config::paths::Paths::config_dir().join("crew");
    let before = (
        crew().list().await.len(),
        fs::read(crew_root.join("connections.json")).ok(),
        fs::read_dir(crew_root.join("credentials"))
            .map(|entries| entries.count())
            .unwrap_or(0),
        fs::read_dir(fixture_root().join("profile"))
            .map(|entries| entries.count())
            .unwrap_or(0),
    );
    let message = invitation_message(WORKSPACE, "hpc.example.org");
    let key = workspace_key::workspace().public_hex();
    let fingerprint = biorouter_crew::invitation::workspace_key_fingerprint(&key).unwrap();

    let (status, body) = call(
        "POST",
        "/crew/connections/from-invitation",
        Some(json!({"invitation": format!("Hi Bob!\n\n{message}\n\nSee you Monday."), "preview": true})),
        Proof::Person,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("connection").is_none(), "{body}");
    let preview = &body["preview"];
    for (field, expected) in [
        ("source", json!("invitation")),
        ("workspace_id", json!(WORKSPACE)),
        ("workspace_public_key", json!(key)),
        ("workspace_key_fingerprint", json!(fingerprint)),
        (
            "fingerprint",
            json!(biorouter_crew::invitation::grouped_fingerprint(
                &fingerprint
            )),
        ),
        ("socket_path", json!(SOCKET)),
        ("owner_uid", json!(HOST_UID)),
        ("workspace_name", json!("lab")),
        ("host_username", json!("alice")),
        ("host_display_name", json!("Alice Chen")),
        ("invitee_username", json!("bob")),
        ("ssh_host", json!("hpc.example.org")),
        ("ssh_port", Value::Null),
        ("server", json!("hpc.example.org")),
        ("username", json!("bob")),
        ("ssh_target", json!("bob@hpc.example.org")),
        ("workspace_mode", json!("private")),
        ("mode", json!("private")),
        ("institution_id", json!("ucsf")),
        ("mode_differs", json!(false)),
        ("name", json!("lab")),
        ("missing", json!([])),
    ] {
        assert_eq!(preview[field], expected, "preview.{field}: {preview}");
    }

    // The person's choices change the plan, never the invitation's own hints.
    let (status, body) = call(
        "POST",
        "/crew/connections/from-invitation",
        Some(json!({
            "invitation": message,
            "preview": true,
            "username": "@robert",
            "mode": "public",
            "institution_id": null,
            "advanced": {"port": 2222},
        })),
        Proof::Person,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let preview = &body["preview"];
    assert_eq!(preview["username"], "robert", "{preview}");
    assert_eq!(preview["ssh_target"], "robert@hpc.example.org", "{preview}");
    assert_eq!(preview["mode"], "public", "{preview}");
    assert_eq!(preview["mode_differs"], true, "{preview}");
    assert_eq!(preview["port"], 2222, "{preview}");
    assert_eq!(preview["ssh_port"], Value::Null, "{preview}");
    assert_eq!(preview["workspace_mode"], "private", "{preview}");

    let after = (
        crew().list().await.len(),
        fs::read(crew_root.join("connections.json")).ok(),
        fs::read_dir(crew_root.join("credentials"))
            .map(|entries| entries.count())
            .unwrap_or(0),
        fs::read_dir(fixture_root().join("profile"))
            .map(|entries| entries.count())
            .unwrap_or(0),
    );
    assert_eq!(before, after, "a preview wrote something");
    assert!(saved_for(WORKSPACE).await.is_empty());

    // Refusals are typed, and never quote the paste back.
    for (text, reason) in [
        (
            "Hi Bob, here is the SECRET-PASTE thing".to_owned(),
            "invitation_not_found",
        ),
        (
            "brcrew1:eyJ2IjoyfQ SECRET-PASTE".to_owned(),
            "invitation_unsupported_version",
        ),
    ] {
        for preview in [true, false] {
            let (status, body) = call(
                "POST",
                "/crew/connections/from-invitation",
                Some(json!({"invitation": text, "preview": preview})),
                Proof::Person,
            )
            .await;
            assert_refused(
                status,
                &body,
                StatusCode::BAD_REQUEST,
                "crew_invitation_invalid",
            );
            assert_eq!(body["reason"], reason, "{body}");
            assert!(!body.to_string().contains("SECRET-PASTE"), "{body}");
        }
    }
    // A body in the wrong shape is a coded refusal too, not a bare extractor error.
    for body in [
        json!({"invitation": message, "ssh_target": "bob@elsewhere"}),
        json!({"preview": true}),
        json!({"invitation": message, "mode": "secret"}),
    ] {
        let (status, answer) = call(
            "POST",
            "/crew/connections/from-invitation",
            Some(body),
            Proof::Person,
        )
        .await;
        assert!(status.is_client_error(), "{status}: {answer}");
        assert_eq!(answer["code"], "crew_request_invalid", "{answer}");
        assert!(answer["detail"].is_string(), "{answer}");
    }
}

/// A save pins the workspace exactly as the invitation says, with the invitation's institution
/// for a Private workspace, and is idempotent; a different identity or different settings for
/// the same workspace are refused with the connection concerned.
#[tokio::test]
#[serial]
async fn a_saved_connection_is_pinned_exactly_as_the_invitation_says() {
    const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a3";
    const PUBLIC_WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a4";
    let created = Mutex::new(Vec::new());
    with_cleanup(&created, async {
        let connection = save_joiner(&created, WORKSPACE, "hpc.example.org").await;
        let id = connection["id"].as_str().unwrap().to_owned();
        for (field, expected) in [
            ("workspace_id", json!(WORKSPACE)),
            (
                "workspace_public_key",
                json!(workspace_key::workspace().public_hex()),
            ),
            ("socket_path", json!(SOCKET)),
            ("owner_uid", json!(HOST_UID)),
            ("ssh_target", json!("bob@hpc.example.org")),
            ("port", Value::Null),
            ("proxy_jump", Value::Null),
            ("mode", json!("private")),
            ("institution_id", json!("ucsf")),
            ("name", json!("lab")),
            ("node_id", Value::Null),
            ("status", json!("disconnected")),
        ] {
            assert_eq!(connection[field], expected, "connection.{field}: {connection}");
        }
        let saved = saved_for(WORKSPACE).await;
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, id);
        assert_eq!(saved[0].public_key, connection["public_key"]);
        // The device key was saved with the connection, and its code is the one the broker
        // computes from the same pins and key.
        assert_eq!(
            crew().device_code(&id).await.unwrap(),
            local_code(&connection)
        );

        // Pasting it again keeps the one device key, and so the one code.
        let again = save_joiner(&created, WORKSPACE, "hpc.example.org").await;
        assert_eq!(
            (&again["id"], &again["public_key"]),
            (&connection["id"], &connection["public_key"])
        );
        created.lock().unwrap().dedup();

        // The same workspace with other settings: refused, naming the saved connection.
        let (status, body) = call(
            "POST",
            "/crew/connections/from-invitation",
            Some(json!({"invitation": invitation_message(WORKSPACE, "hpc.example.org"), "username": "robert"})),
            Proof::Person,
        )
        .await;
        assert_refused(status, &body, StatusCode::CONFLICT, "crew_connection_exists");
        assert_eq!(body["connection_id"], id.as_str(), "{body}");

        // The same workspace ID pinned to another key is never re-pinned from a paste.
        let mut tampered = workspace_invitation(WORKSPACE, "hpc.example.org");
        tampered.workspace_public_key = workspace_key::Key::from_seed([4; 32]).public_hex();
        for preview in [true, false] {
            let (status, body) = call(
                "POST",
                "/crew/connections/from-invitation",
                Some(json!({
                    "invitation": biorouter_crew::invitation::message(&tampered).unwrap(),
                    "preview": preview,
                })),
                Proof::Person,
            )
            .await;
            assert_refused(status, &body, StatusCode::CONFLICT, "crew_invitation_conflict");
            assert_eq!(body["connection_id"], id.as_str(), "{body}");
        }
        assert_eq!(
            saved_for(WORKSPACE).await[0].workspace_public_key,
            workspace_key::workspace().public_hex()
        );

        // Private needs an institution: a Public invitation joined as Private without one is
        // refused, and nothing is saved.
        let mut public = workspace_invitation(PUBLIC_WORKSPACE, "hpc.example.org");
        public.mode = Some(biorouter_crew::Mode::Public);
        public.institution_id = None;
        let (status, body) = call(
            "POST",
            "/crew/connections/from-invitation",
            Some(json!({
                "invitation": biorouter_crew::invitation::message(&public).unwrap(),
                "mode": "private",
            })),
            Proof::Person,
        )
        .await;
        assert_refused(status, &body, StatusCode::BAD_REQUEST, "crew_invitation_invalid");
        assert_eq!(body["reason"], "missing", "{body}");
        assert_eq!(body["missing"], "institution", "{body}");
        assert!(saved_for(PUBLIC_WORKSPACE).await.is_empty());
    })
    .await;
}

/// The join status shows the device code this daemon computed from its own key, whatever the
/// workspace's answer carries; a claim is sent only when the host approved, and a refusal's
/// unauthenticated words never reach the person.
#[tokio::test]
#[serial]
async fn join_status_shows_only_the_code_this_computer_computed() {
    const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a5";
    const HOST: &str = "join.example.test";
    let created = Mutex::new(Vec::new());
    with_cleanup(&created, async {
        let pending = |join_id: &str, approved: bool, refused: bool| {
            let mut answer = json!({
                "invited": true,
                "join_id": join_id,
                "workspace_name": "lab",
                "inviter": {"username": "alice", "display_name": "Alice Chen"},
                "add_device": false,
                "approved": approved,
                "expires_at": 4_000_000_000u64,
                // What a hostile bridge adds, under every name a client might read.
                "code": HOSTILE_CODE,
                "device_code": HOSTILE_CODE,
                "approved_code": HOSTILE_CODE.replace('-', ""),
            });
            if refused {
                answer["last_refusal"] = json!("code_mismatch");
            }
            json!({"result": answer})
        };
        let scenario = Scenario::new(
            WORKSPACE,
            &["human_names_v1", "unique_names_v1", "join_by_name_v1"],
        );
        stage(
            HOST,
            &scenario
                .clone()
                .answer("enrollment.pending", pending("join-1", false, false)),
        );
        let connection = save_joiner(&created, WORKSPACE, HOST).await;
        let id = connection["id"].as_str().unwrap().to_owned();
        let code = local_code(&connection);
        assert_ne!(code, HOSTILE_CODE);
        crew().connect(&id).await.expect("the fake workspace verifies");
        let join = format!("/crew/connections/{id}/join");

        // Unproven: refused before the workspace is asked anything.
        for method in ["GET", "POST"] {
            let (status, body) = call(method, &join, None, Proof::Absent).await;
            assert_refused(status, &body, StatusCode::FORBIDDEN, "crew_user_action_required");
        }
        assert!(received(HOST, "enrollment.pending").is_empty());
        assert!(received(HOST, "auth.join").is_empty());

        let status_is = |body: &Value, state: &str| {
            assert_eq!(body["status"], state, "{body}");
            assert_eq!(body["code"], code.as_str(), "the code is computed here: {body}");
            let text = body.to_string();
            assert!(!text.contains(HOSTILE_CODE), "{text}");
            assert!(!text.contains(&HOSTILE_CODE.replace('-', "")), "{text}");
            assert!(body.get("device_code").is_none() && body.get("approved_code").is_none());
        };

        let (status, body) = call("GET", &join, None, Proof::Person).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        status_is(&body, "invited");
        assert_eq!(
            body["inviter"],
            json!({"username": "alice", "display_name": "Alice Chen"})
        );
        assert_eq!(body["workspace_name"], "lab");
        assert_eq!(body["expires_at"], 4_000_000_000u64);
        assert_eq!(body["add_device"], false);
        let asked = received(HOST, "enrollment.pending");
        assert_eq!(asked.len(), 1);
        assert!(
            asked[0].get("auth").is_none(),
            "the join status is asked unsigned, like hello: {}",
            asked[0]
        );

        // Not approved yet: nothing is claimed.
        let (status, body) = call("POST", &join, None, Proof::Person).await;
        assert_refused(status, &body, StatusCode::CONFLICT, "crew_join_not_approved");
        assert!(received(HOST, "auth.join").is_empty());

        // Approved, and the workspace refuses the claim: typed, and its words stay in the log.
        stage(
            HOST,
            &scenario
                .clone()
                .answer("enrollment.pending", pending("join-1", true, false))
                .answer(
                    "auth.join",
                    json!({"error": {"code": "code_mismatch", "message": format!("code_mismatch: {HOSTILE_WORDS}")}}),
                ),
        );
        let (status, body) = call("GET", &join, None, Proof::Person).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        status_is(&body, "approved");
        let (status, body) = call("POST", &join, None, Proof::Person).await;
        assert_refused(status, &body, StatusCode::CONFLICT, "crew_join_code_mismatch");
        assert!(!body.to_string().contains(HOSTILE_WORDS), "{body}");
        let claims = received(HOST, "auth.join");
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0]["params"]["join_id"], "join-1");
        assert_eq!(claims[0]["params"]["public_key"], connection["public_key"]);
        assert_eq!(
            claims[0]["auth"]["device_id"], connection["device_id"],
            "the claim is signed with the saved device key: {}",
            claims[0]
        );

        // The workspace now reports the refusal: this computer shows the mismatch with its own
        // code, and does not claim again under the same approval.
        stage(
            HOST,
            &scenario
                .clone()
                .answer("enrollment.pending", pending("join-1", true, true)),
        );
        let (status, body) = call("GET", &join, None, Proof::Person).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        status_is(&body, "code_mismatch");
        let (status, body) = call("POST", &join, None, Proof::Person).await;
        assert_refused(status, &body, StatusCode::CONFLICT, "crew_join_code_mismatch");
        assert_eq!(received(HOST, "auth.join").len(), 1);

        // Re-invited and approved: the claim goes through and the answer says joined.
        stage(
            HOST,
            &scenario
                .clone()
                .answer("enrollment.pending", pending("join-2", true, false))
                .answer(
                    "auth.join",
                    json!({"result": {
                        "principal": {"username": "bob", "display_name": "bob"},
                        "device_id": connection["device_id"],
                        "workspace": {"id": WORKSPACE},
                    }}),
                ),
        );
        let (status, body) = call("POST", &join, None, Proof::Person).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["joined"], true, "{body}");
        assert_eq!(body["status"], "joined", "{body}");
        assert_eq!(body["workspace_name"], "lab", "{body}");
        assert!(body.get("code").is_none(), "{body}");
        let claims = received(HOST, "auth.join");
        assert_eq!(claims.len(), 2);
        assert_eq!(claims[1]["params"]["join_id"], "join-2");
    })
    .await;
}

/// A workspace whose verified `hello` does not announce `join_by_name_v1` is `unsupported`,
/// and nothing about joining is ever sent to it.
#[tokio::test]
#[serial]
async fn a_workspace_without_join_by_name_is_unsupported() {
    const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a6";
    const HOST: &str = "old-broker.example.test";
    let created = Mutex::new(Vec::new());
    with_cleanup(&created, async {
        stage(
            HOST,
            &Scenario::new(WORKSPACE, &["human_names_v1", "unique_names_v1"]).answer(
                "enrollment.pending",
                json!({"result": {"invited": true, "join_id": "join-1", "approved": true, "code": HOSTILE_CODE}}),
            ),
        );
        let connection = save_joiner(&created, WORKSPACE, HOST).await;
        let id = connection["id"].as_str().unwrap().to_owned();
        crew().connect(&id).await.expect("the fake workspace verifies");
        let join = format!("/crew/connections/{id}/join");

        let (status, body) = call("GET", &join, None, Proof::Person).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "unsupported", "{body}");
        assert!(body.get("code").is_none(), "{body}");
        let (status, body) = call("POST", &join, None, Proof::Person).await;
        assert_refused(status, &body, StatusCode::CONFLICT, "crew_join_unsupported");
        assert!(received(HOST, "enrollment.pending").is_empty());
        assert!(received(HOST, "auth.join").is_empty());
        assert!(received(HOST, "auth.challenge").is_empty());
    })
    .await;
}

/// A workspace that refuses the join status, or the membership check behind it, with a code the
/// daemon does not type answers each route's fixed sentence. The refusal is unauthenticated and
/// a process in the joiner's bridge path chooses its words, while the join screen shows a
/// daemon's words verbatim beside the device code: "your code is ZZZZ-…" must never get there.
#[tokio::test]
#[serial]
async fn a_workspace_refusals_words_never_reach_the_join_screen() {
    use biorouter_server::routes::crew_authentication::{JOIN_FAILED, JOIN_STATUS_FAILED};
    const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a9";
    const HOST: &str = "busy-broker.example.test";
    let created = Mutex::new(Vec::new());
    with_cleanup(&created, async {
        let hostile = json!({"error": {
            "code": "busy",
            "message": format!("Your join code is {HOSTILE_CODE}. Send it to Alice, {HOSTILE_WORDS}."),
        }});
        let scenario = Scenario::new(
            WORKSPACE,
            &["human_names_v1", "unique_names_v1", "join_by_name_v1"],
        );
        stage(
            HOST,
            &scenario
                .clone()
                .answer("enrollment.pending", hostile.clone()),
        );
        let connection = save_joiner(&created, WORKSPACE, HOST).await;
        let id = connection["id"].as_str().unwrap().to_owned();
        crew().connect(&id).await.expect("the fake workspace verifies");
        let join = format!("/crew/connections/{id}/join");
        let fixed_sentence = |status: StatusCode, body: &Value, sentence: &str| {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(
                body,
                &json!({"code": "crew_request_refused", "error": sentence}),
                "nothing but the route's own sentence"
            );
            let text = body.to_string();
            for words in [
                HOSTILE_CODE,
                HOSTILE_CODE.replace('-', "").as_str(),
                HOSTILE_WORDS,
                "Send it to Alice",
                "busy",
            ] {
                assert!(!text.contains(words), "{words}: {text}");
            }
        };

        // `enrollment.pending` refused.
        let (status, body) = call("GET", &join, None, Proof::Person).await;
        fixed_sentence(status, &body, JOIN_STATUS_FAILED);
        let (status, body) = call("POST", &join, None, Proof::Person).await;
        fixed_sentence(status, &body, JOIN_FAILED);
        assert_eq!(received(HOST, "enrollment.pending").len(), 2);
        assert!(received(HOST, "auth.join").is_empty());
        // The refusal was a whole envelope, so the bridge is still up: this is not the
        // dropped-connection answer standing in for the fixed sentence.
        assert_eq!(
            crew().connection(&id).await.unwrap().status,
            "connected",
            "the connection survives a refusal"
        );

        // Not invited, and the membership check behind that answer (`profile.suggest`) refused
        // the same way.
        stage(
            HOST,
            &scenario
                .clone()
                .answer("enrollment.pending", json!({"result": {"invited": false}}))
                .answer("profile.suggest", hostile.clone()),
        );
        let (status, body) = call("GET", &join, None, Proof::Person).await;
        fixed_sentence(status, &body, JOIN_STATUS_FAILED);
        let (status, body) = call("POST", &join, None, Proof::Person).await;
        fixed_sentence(status, &body, JOIN_FAILED);
        assert_eq!(
            received(HOST, "profile.suggest").len(),
            2,
            "both answers came from the membership check"
        );
        assert!(received(HOST, "auth.join").is_empty());
    })
    .await;
}

/// An unknown connection is a coded 404 and an offline one a coded 409, so a client can tell
/// them from a daemon that predates the routes (an uncoded 404).
#[tokio::test]
#[serial]
async fn unknown_and_offline_connections_are_typed_refusals() {
    const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a7";
    let created = Mutex::new(Vec::new());
    with_cleanup(&created, async {
        let unknown = "00000000-0000-4000-8000-000000000000";
        for (method, uri) in [
            ("GET", format!("/crew/connections/{unknown}/invitation")),
            ("GET", format!("/crew/connections/{unknown}/join")),
            ("POST", format!("/crew/connections/{unknown}/join")),
        ] {
            let (status, body) = call(method, &uri, None, Proof::Person).await;
            assert_refused(
                status,
                &body,
                StatusCode::NOT_FOUND,
                "crew_connection_not_found",
            );
        }

        let connection = save_joiner(&created, WORKSPACE, "offline.example.test").await;
        let id = connection["id"].as_str().unwrap();
        for (method, uri) in [
            (
                "GET",
                format!("/crew/connections/{id}/invitation?invitee=%40bob"),
            ),
            ("GET", format!("/crew/connections/{id}/join")),
            ("POST", format!("/crew/connections/{id}/join")),
        ] {
            let (status, body) = call(method, &uri, None, Proof::Person).await;
            assert_refused(status, &body, StatusCode::CONFLICT, "crew_not_connected");
        }
        assert!(!log_path(&fixture_root(), "offline.example.test").exists());

        let (status, body) = call(
            "GET",
            &format!("/crew/connections/{id}/invitation?invitee=bob%20lee"),
            None,
            Proof::Person,
        )
        .await;
        assert_refused(
            status,
            &body,
            StatusCode::BAD_REQUEST,
            "crew_invalid_selector",
        );
        let (status, body) = call(
            "GET",
            &format!("/crew/connections/{id}/invitation?for=bob"),
            None,
            Proof::Person,
        )
        .await;
        assert_eq!(body["code"], "crew_request_invalid", "{status}: {body}");
    })
    .await;
}

/// The host's invitation is built from the verified connection, the workspace's own signed
/// word and `ssh -G`: it names the real server, never the host's local alias or the
/// connection's local name, and it pins exactly what the host's connection pins.
#[tokio::test]
#[serial]
async fn a_host_invitation_names_the_server_and_never_a_local_alias() {
    const WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000000a8";
    const ALIAS: &str = "hpc-alias-a8";
    let created = Mutex::new(Vec::new());
    with_cleanup(&created, async {
        let key = workspace_key::workspace().public_hex();
        let mut scenario = Scenario::new(
            WORKSPACE,
            &["human_names_v1", "unique_names_v1", "join_by_name_v1"],
        )
        .answer(
            "workspace.snapshot",
            json!({"result": {
                "workspace": {"id": WORKSPACE, "name": "lab", "mode": "private", "institution_id": "ucsf"},
                "principals": [
                    {"id": "p-carol", "uid": 1002, "username": "carol", "display_name": "Carol", "active": true},
                    {"id": "p-alice", "uid": HOST_UID, "username": "alice", "display_name": "Alice Chen", "active": true},
                ],
            }}),
        );
        scenario.resolves_to = Some("hpc.example.org".into());
        stage(ALIAS, &scenario);

        // What `biorouter-crew status` prints for the host, saved with the host's own login.
        let status_json = json!({
            "protocol": 1,
            "workspace_id": WORKSPACE,
            "host_uid": HOST_UID,
            "workspace_public_key": key,
            "socket": SOCKET,
            "workspace_key_fingerprint": biorouter_crew::invitation::workspace_key_fingerprint(&key).unwrap(),
        })
        .to_string();
        let (status, body) = call(
            "POST",
            "/crew/connections/from-invitation",
            Some(json!({
                "invitation": status_json,
                "institution_id": "ucsf",
                "advanced": {"ssh_target": format!("alice@{ALIAS}"), "name": "my-laptop-copy"},
            })),
            Proof::Person,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let host = body["connection"].clone();
        let id = host["id"].as_str().unwrap().to_owned();
        created.lock().unwrap().push(id.clone());
        assert_eq!(host["ssh_target"], format!("alice@{ALIAS}"));
        crew().connect(&id).await.expect("the fake workspace verifies");

        let (status, body) = call(
            "GET",
            &format!("/crew/connections/{id}/invitation?invitee=%40bob"),
            None,
            Proof::Person,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (message, line) = (
            body["message"].as_str().unwrap(),
            body["line"].as_str().unwrap(),
        );
        assert!(line.starts_with("brcrew1:"), "{line}");
        assert!(message.contains(line), "{message}");
        for text in [message, line] {
            assert!(!text.contains(ALIAS), "{text}");
            assert!(!text.contains("my-laptop-copy"), "{text}");
        }
        let invitation = biorouter_crew::invitation::parse(message).unwrap().invitation;
        assert_eq!(
            invitation,
            biorouter_crew::invitation::WorkspaceInvitation {
                workspace_id: WORKSPACE.into(),
                workspace_public_key: key.clone(),
                socket_path: SOCKET.into(),
                owner_uid: HOST_UID,
                workspace_name: Some("lab".into()),
                host_username: Some("alice".into()),
                host_display_name: Some("Alice Chen".into()),
                mode: Some(biorouter_crew::Mode::Private),
                institution_id: Some("ucsf".into()),
                ssh_host: Some("hpc.example.org".into()),
                ssh_port: None,
                proxy_jump: None,
                invitee_username: Some("bob".into()),
            }
        );
        let snapshots = received(ALIAS, "workspace.snapshot");
        assert_eq!(snapshots.len(), 1);
        assert!(
            snapshots[0].get("auth").is_some(),
            "the host's snapshot is a signed read: {}",
            snapshots[0]
        );

        // No invitee: the message names nobody.
        let (status, body) = call(
            "GET",
            &format!("/crew/connections/{id}/invitation"),
            None,
            Proof::Person,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let invitation = biorouter_crew::invitation::parse(body["line"].as_str().unwrap())
            .unwrap()
            .invitation;
        assert_eq!(invitation.invitee_username, None);
    })
    .await;
}

/// The routes are registered on the daemon the way `routes::configure` composes it (a dropped
/// merge line would pass every test above), next to the generic request route, which can never
/// carry a join: `auth.join` and `enrollment.pending` are sent only by the daemon's own join.
#[tokio::test]
#[serial]
async fn the_admission_routes_are_registered_and_the_generic_request_route_cannot_join() {
    let state = biorouter_server::state::AppState::new()
        .await
        .expect("app state");
    let daemon = biorouter_server::routes::configure(state, "crew-join-routes-secret".into());
    let unknown = "00000000-0000-4000-8000-000000000000";
    for (method, uri) in [
        ("POST", "/crew/connections/from-invitation".to_owned()),
        ("GET", format!("/crew/connections/{unknown}/invitation")),
        ("GET", format!("/crew/connections/{unknown}/join")),
        ("POST", format!("/crew/connections/{unknown}/join")),
    ] {
        let body = (uri.ends_with("from-invitation")).then(|| json!({"invitation": "x"}));
        let (status, answer) = call_app(daemon.clone(), method, &uri, body, Proof::Absent).await;
        assert_refused(
            status,
            &answer,
            StatusCode::FORBIDDEN,
            "crew_user_action_required",
        );
        let (status, answer) = call_app(daemon.clone(), method, &uri, None, Proof::Person).await;
        assert!(
            answer["code"].is_string(),
            "{method} {uri} is served by a current route: {status} {answer}"
        );
    }
    for method in ["auth.join", "enrollment.pending"] {
        let (status, answer) = call_app(
            daemon.clone(),
            "POST",
            &format!("/crew/connections/{unknown}/request"),
            Some(json!({"method": method, "params": {"public_key": "00", "join_id": "join-1"}})),
            Proof::Person,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{method}: {answer}");
        assert_eq!(answer["code"], "crew_request_refused", "{method}: {answer}");
    }
}
