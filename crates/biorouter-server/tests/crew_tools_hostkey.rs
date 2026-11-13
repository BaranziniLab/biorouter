//! A server's host key from a chat (WI-2, the owner's D1 and D6 rulings on AO-12): the card the
//! daemon's host (`routes::crew_tools::hostkey`) parks when a step it ran was refused on a host
//! key, driven the way the `crew` extension drives it, one `PersonToolCall` at a time.
//!
//! Pinned here, against the real host, the real setup service, the real approval registry and
//! routes, and the real `ssh-keygen`:
//!
//! - an unknown key parks exactly one host-key card, whose fingerprint is the daemon's own (here
//!   the fake `ssh-keyscan`'s), never anything from the call's parameters;
//! - the card cannot be approved: a forged allow, with the person's proof, leaves it waiting;
//! - once the key is in the known-hosts file (the development profile's, the transport's
//!   `UserKnownHostsFile`, which is this binary's HOME override) the card is withdrawn and the
//!   step is tried again, once, on the approval the person already gave for it;
//! - a dismissed card is not tried again, and the model is told the step and the fingerprint;
//! - a changed key parks no card that waits, and answers with the IT wording.
//!
//! **Offline.** `ssh` and `ssh-keyscan` are this binary, put first on `PATH` by a `#[ctor]`: the
//! fake `ssh` refuses `unknown.hostkey.test` on its host key until the known-hosts file holds it
//! (then the connection is refused, so the retry ends at once), and always refuses
//! `changed.hostkey.test` on a changed key. **Its own binary**: the host and the user-action
//! digest are process-global `OnceLock`s.
#![cfg(unix)]

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root before `main`,
// so nothing here can open the developer's real `sessions.db` or Crew profile.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use biorouter::action_required_manager::ActionRequiredManager;
use biorouter::conversation::message::{ActionRequiredData, CrewStepRequest, MessageContent};
use biorouter::crew::tool_host::{self, PersonTool, PersonToolCall};
use biorouter::pending_user_action::PendingUserActions;
use biorouter::privacy::CallCapability;
use biorouter::session::session_manager::SessionType;
use biorouter_server::state::AppState;
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

/// Set only in a fake tool's process: the fixture directory.
const FAKE_ROOT: &str = "BIOROUTER_CREW_TOOLS_HOSTKEY_FAKE_ROOT";
/// Set only in a fake tool's process: which tool it plays, `ssh` or `ssh-keyscan`.
const FAKE_TOOL: &str = "BIOROUTER_CREW_TOOLS_HOSTKEY_FAKE_TOOL";
/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-tools-hostkey-user-action-key";
/// The daemon secret the full router checks.
const SECRET: &str = "crew-tools-hostkey-secret";

/// A server whose key this computer does not know yet.
const UNKNOWN_HOST: &str = "unknown.hostkey.test";
/// A server that answered with another key than the one this computer trusts.
const CHANGED_HOST: &str = "changed.hostkey.test";
/// The key the server offers, as `ssh-keyscan` prints it, and its fingerprint.
const BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIDVMyBQ3psSwstgc8+Mce5VHIZN5GFuu3xE6WV8pfFkD";
const FINGERPRINT: &str = "SHA256:XA01Ou0eChdPDyZiadGxCZ/g+7RnODImHcZEtuRmV8g";
/// The changed server's new key, as OpenSSH's banner prints it.
const CHANGED_FINGERPRINT: &str = "SHA256:TmV3S2V5TmV3S2V5TmV3S2V5TmV3S2V5TmV3S2V5TmU";
const JOIN_WORKSPACE: &str = "3f2a9c1e-77b0-4d4e-8a11-0000000d0096";

// ---------------------------------------------------------------------------------------------
// The fake tools, run by this same binary before `main`.
// ---------------------------------------------------------------------------------------------

mod fake {
    use super::{BLOB, CHANGED_FINGERPRINT, CHANGED_HOST, UNKNOWN_HOST};
    use std::io::Write;
    use std::path::Path;

    /// OpenSSH options that take a value, so the target is found the way `ssh` finds it.
    const WITH_VALUE: &[&str] = &[
        "-B", "-b", "-c", "-D", "-E", "-e", "-F", "-I", "-i", "-J", "-L", "-l", "-m", "-O", "-o",
        "-p", "-Q", "-R", "-S", "-W", "-w",
    ];

    fn target(args: &[String]) -> Option<String> {
        let mut rest = args.iter();
        while let Some(arg) = rest.next() {
            if WITH_VALUE.contains(&arg.as_str()) {
                rest.next();
            } else if !arg.starts_with('-') {
                return Some(
                    arg.rsplit_once('@')
                        .map_or(arg.as_str(), |(_, host)| host)
                        .to_owned(),
                );
            }
        }
        None
    }

    fn log(root: &Path, file: &str, line: &str) {
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join(file))
        {
            let _ = writeln!(log, "{line}");
        }
    }

    pub fn run(root: &Path, tool: &str) -> i32 {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if tool == "ssh-keyscan" {
            log(root, "keyscan.log", &args.join(" "));
            let host = args.last().cloned().unwrap_or_default();
            if host == UNKNOWN_HOST {
                println!("{host} ssh-ed25519 {BLOB}");
            }
            return 0;
        }
        // `-O exit` / `-O check` against a control master this fake never creates.
        if args.iter().any(|arg| arg == "-O") {
            return 0;
        }
        let Some(host) = target(&args) else {
            eprintln!("fake ssh: no destination in {args:?}");
            return 255;
        };
        if args.first().map(String::as_str) == Some("-G") {
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "hostname {host}\nport 22");
            // The known hosts files, as `ssh -G` lists them: the one Crew's ssh passes, then a
            // second one of the person's configuration, which a key may be added to instead.
            let user_file = args
                .windows(2)
                .find_map(|pair| {
                    (pair[0] == "-o")
                        .then(|| pair[1].strip_prefix("UserKnownHostsFile="))
                        .flatten()
                })
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "userknownhostsfile {user_file} {}\nglobalknownhostsfile {}\nhashknownhosts no",
                root.join("second_known_hosts").display(),
                root.join("global_known_hosts").display()
            );
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
            return 0;
        }
        log(root, "attempts.log", &host);
        let known = ["profile/home/.ssh/known_hosts", "second_known_hosts"]
            .iter()
            .map(|file| std::fs::read_to_string(root.join(file)).unwrap_or_default())
            .collect::<String>();
        let stderr = if host == CHANGED_HOST {
            [
                "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@",
                "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @",
                "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@",
                "The fingerprint for the ED25519 key sent by the remote host is",
                &format!("{CHANGED_FINGERPRINT}."),
                "Offending ED25519 key in /home/you/.ssh/known_hosts:3",
                &format!("Host key for {host} has changed and you have requested strict checking."),
                "Host key verification failed.",
            ]
            .join("\r\n")
        } else if host == UNKNOWN_HOST && !known.contains(UNKNOWN_HOST) {
            format!("No ED25519 host key is known for {host} and you have requested strict checking.\r\nHost key verification failed.")
        } else {
            format!("ssh: connect to host {host} port 22: Connection refused")
        };
        eprintln!("{stderr}");
        255
    }
}

/// This test process's fixture directory: the fake tools, their logs, and the development
/// profile, whose `home/.ssh/known_hosts` is the file the daemon's ssh trusts keys from.
fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!(
        "biorouter-crew-tools-hostkey-{}",
        std::process::id()
    ))
}

fn known_hosts() -> PathBuf {
    fixture_root().join("profile/home/.ssh/known_hosts")
}

#[ctor::ctor]
fn fake_tool_or_fixture() {
    if let (Some(root), Some(tool)) = (std::env::var_os(FAKE_ROOT), std::env::var(FAKE_TOOL).ok()) {
        let code = fake::run(Path::new(&root), &tool);
        std::process::exit(code);
    }
    let root = fixture_root();
    let prepared = (|| -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        for directory in ["bin", "profile/home/.ssh"] {
            fs::create_dir_all(root.join(directory))?;
        }
        let executable = std::env::current_exe()?;
        let (root_text, executable_text) = (root.display(), executable.display());
        if format!("{root_text}{executable_text}").contains('\'') {
            return Err(std::io::Error::other("a fixture path contains a quote"));
        }
        for tool in ["ssh", "ssh-keyscan"] {
            let script = root.join("bin").join(tool);
            fs::write(
                &script,
                format!(
                    "#!/bin/sh\n{FAKE_ROOT}='{root_text}' {FAKE_TOOL}='{tool}' exec '{executable_text}' \"$@\"\n"
                ),
            )?;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    })();
    if let Err(error) = prepared {
        eprintln!(
            "crew_tools_hostkey: could not prepare the fixture at {}: {error}. Refusing to run: \
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
    if std::env::var_os(FAKE_ROOT).is_none() {
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

/// A request to the full router with the daemon secret and the desktop's proof, as a person's
/// click sends it.
async fn http(uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("X-Secret-Key", SECRET)
        .header("X-User-Action", USER_KEY)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = biorouter_server::routes::configure(state().await, SECRET.into())
        .oneshot(request)
        .await
        .unwrap();
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

async fn chat(name: &str) -> String {
    state()
        .await
        .session_manager()
        .create_session(
            std::env::temp_dir().join("crew-tools-hostkey"),
            name.to_owned(),
            SessionType::User,
        )
        .await
        .expect("a chat")
        .id
}

/// Every ssh connection the fake was asked to make to `host` (not counting `ssh -G`).
fn attempts(host: &str) -> usize {
    fs::read_to_string(fixture_root().join("attempts.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == host)
        .count()
}

/// The host-key cards the daemon published to `session` so far, in order.
fn published_host_keys(session: &str) -> Vec<(String, CrewStepRequest)> {
    ActionRequiredManager::global()
        .drain_requests(session)
        .into_iter()
        .flat_map(|message| message.content)
        .filter_map(|content| match content {
            MessageContent::ActionRequired(action) => match action.data {
                ActionRequiredData::CrewStep { id, step } => Some((id, step)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// A call, and the person at its cards
// ---------------------------------------------------------------------------------------------

/// What the person does at a host-key card when it first appears.
#[derive(Clone, Copy)]
enum AtTheCard {
    /// Try a forged allow first, then add the key to the known-hosts file.
    ForgeThenAddTheKey,
    /// Add the key to the second known hosts file `ssh -G` lists, not the profile's.
    AddTheKeyToTheSecondFile,
    /// Choose Trust this key: the card's own values, posted with the person's proof.
    TrustTheKey,
    ChangeJoinBeforeTrust(&'static str),
    /// Choose Not now.
    Dismiss,
    /// Do nothing: a card that waited would hold the call until its time ran out.
    Watch,
}

/// What a call answered, and what it showed the person.
struct Called {
    answer: Result<Value, String>,
    /// Each approval card's prompt, in order. Every one is allowed.
    approvals: Vec<String>,
    /// Each host-key card that waited, as it was parked.
    host_keys: Vec<Value>,
}

/// Call `crew__read setup_check` for `server` from `session`, allowing every approval card and
/// answering a host-key card as `at_the_card` says.
async fn check(session: &str, server: &str, at_the_card: AtTheCard) -> Called {
    check_with(session, json!({"server": server}), at_the_card).await
}

/// [`check`], with the call's whole `params`.
async fn check_with(session: &str, params: Value, at_the_card: AtTheCard) -> Called {
    call_with(
        session,
        PersonTool::Read,
        "setup_check",
        params,
        at_the_card,
    )
    .await
}

async fn call_with(
    session: &str,
    tool: PersonTool,
    action: &str,
    params: Value,
    at_the_card: AtTheCard,
) -> Called {
    let host = tool_host::get().expect("the host is installed");
    let call = PersonToolCall {
        tool,
        action: action.into(),
        params,
        session_id: session.to_owned(),
        tool_call_id: Some(format!("call-{}", uuid::Uuid::new_v4())),
        capability: CallCapability::public_enforced(),
        cancel: CancellationToken::new(),
        approvals: Arc::clone(PendingUserActions::global()),
    };
    let task =
        tokio::spawn(async move { host.call(call).await.map_err(|error| error.to_string()) });
    let mut called = Called {
        answer: Err(String::new()),
        approvals: Vec::new(),
        host_keys: Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while !task.is_finished() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the call never ended"
        );
        for message in PendingUserActions::global().pending_cards_for_session(session) {
            let card = serde_json::to_value(&message).unwrap();
            let data = card["content"][0]["data"].clone();
            let id = data["id"].as_str().unwrap_or_default().to_owned();
            if id.is_empty() || !seen.insert(id.clone()) {
                continue;
            }
            match data["actionType"].as_str() {
                Some("crewStep") => {
                    called.host_keys.push(data["step"].clone());
                    answer_host_key(session, &id, &data["step"], at_the_card).await;
                }
                _ => {
                    called
                        .approvals
                        .push(data["prompt"].as_str().unwrap_or_default().to_owned());
                    let (status, body) = http(
                        "/action-required/tool-confirmation",
                        json!({"id": id, "principalType": "Tool", "action": "allow_once", "sessionId": session}),
                    )
                    .await;
                    assert_eq!(status, StatusCode::OK, "{body}");
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    called.answer = task.await.expect("the call ran");
    called
}

async fn answer_host_key(session: &str, id: &str, step: &Value, at_the_card: AtTheCard) {
    if let AtTheCard::ChangeJoinBeforeTrust(field) = at_the_card {
        let crew = biorouter::crew::manager().unwrap();
        let saved = crew
            .list()
            .await
            .into_iter()
            .find(|connection| connection.workspace_id == JOIN_WORKSPACE)
            .expect("the initial join saved its record before asking about the key");
        let mut input = serde_json::to_value(&saved).unwrap();
        input.as_object_mut().unwrap().retain(|key, _| {
            ![
                "id",
                "node_id",
                "policy_epoch",
                "status",
                "last_error",
                "device_id",
                "public_key",
            ]
            .contains(&key.as_str())
        });
        match field {
            "route" => input["ssh_target"] = json!(format!("other@{UNKNOWN_HOST}")),
            "identity" => input["identity_file"] = json!("/private/test/changed-identity"),
            "access" => {
                input["remote_root"] = json!("/private/test/changed-root");
                input["remote_execution"] = json!(true);
                input["remote_access"] = json!("full");
            }
            "policy" => {
                input["mode"] = json!("public");
                input["institution_id"] = Value::Null;
                crew.update(&saved.id, serde_json::from_value(input.clone()).unwrap())
                    .await
                    .unwrap();
                input["mode"] = json!("private");
                input["institution_id"] = json!("ucsf");
            }
            _ => panic!("unknown fixture mutation"),
        }
        crew.update(&saved.id, serde_json::from_value(input).unwrap())
            .await
            .unwrap();
    }
    match at_the_card {
        AtTheCard::AddTheKeyToTheSecondFile => {
            fs::write(
                fixture_root().join("second_known_hosts"),
                format!("{UNKNOWN_HOST} ssh-ed25519 {BLOB}\n"),
            )
            .unwrap();
        }
        AtTheCard::TrustTheKey | AtTheCard::ChangeJoinBeforeTrust(_) => {
            // What the desktop card's Trust this key posts: the card's own values, nothing else.
            let (status, body) = http(
                "/crew/host-keys/trust",
                json!({
                    "host": step["host"],
                    "port": step["port"],
                    "login": step["login"],
                    "fingerprint": step["offered"],
                }),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["already_trusted"], false, "{body}");
        }
        AtTheCard::ForgeThenAddTheKey => {
            // An allow, with the person's proof, cannot answer a host-key card.
            let (status, body) = http(
                "/action-required/tool-confirmation",
                json!({"id": id, "principalType": "Tool", "action": "allow_once", "sessionId": session}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_ne!(body["status"], "delivered", "{body}");
            tokio::time::sleep(Duration::from_millis(1500)).await;
            assert!(
                PendingUserActions::global().is_pending(id),
                "a forged allow took the card down"
            );
            // The person checks the key with IT and adds it in a terminal of their own.
            fs::write(
                known_hosts(),
                format!("{UNKNOWN_HOST} ssh-ed25519 {BLOB}\n"),
            )
            .unwrap();
        }
        AtTheCard::Dismiss => {
            let (status, body) = http(
                "/action-required/crew-step",
                json!({"id": id, "session_id": session, "outcome": "dismiss"}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        AtTheCard::Watch => {}
    }
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

fn join_params() -> Value {
    let invitation =
        biorouter_crew::invitation::message(&biorouter_crew::invitation::WorkspaceInvitation {
            workspace_id: JOIN_WORKSPACE.into(),
            workspace_public_key: "11".repeat(32),
            socket_path: "/tmp/crew-1000-0123456789abcdef0123456789abcdef/broker.sock".into(),
            owner_uid: 1000,
            workspace_name: Some("join-retry-lab".into()),
            host_username: Some("alice".into()),
            host_display_name: None,
            mode: Some(biorouter_crew::Mode::Private),
            institution_id: Some("ucsf".into()),
            ssh_host: Some(UNKNOWN_HOST.into()),
            ssh_port: None,
            proxy_jump: None,
            invitee_username: Some("you".into()),
        })
        .unwrap();
    json!({"invitation": invitation, "ssh_target": format!("you@{UNKNOWN_HOST}"), "agent_access": "off"})
}

#[test]
#[serial]
fn an_initial_invitation_join_retries_its_own_saved_route_after_trust() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let session = chat("join retry").await;
        let before = attempts(UNKNOWN_HOST);
        let called = call_with(
            &session,
            PersonTool::Change,
            "join",
            join_params(),
            AtTheCard::TrustTheKey,
        )
        .await;
        assert_eq!(called.approvals.len(), 1, "{:?}", called.approvals);
        assert_eq!(called.host_keys.len(), 1, "{:?}", called.host_keys);
        assert_eq!(
            attempts(UNKNOWN_HOST) - before,
            2,
            "the transport retries once after Trust"
        );
        let error = called
            .answer
            .expect_err("the offline server refuses the retried transport");
        assert!(!error.contains("already has a connection"), "{error}");
        assert!(!PendingUserActions::global().has_pending_in_session(&session));

        let before = attempts(UNKNOWN_HOST);
        let again = call_with(
            &session,
            PersonTool::Change,
            "join",
            join_params(),
            AtTheCard::Watch,
        )
        .await;
        assert!(again.approvals.is_empty());
        assert!(again.host_keys.is_empty());
        assert!(again
            .answer
            .unwrap_err()
            .contains("already has a connection"));
        assert_eq!(
            attempts(UNKNOWN_HOST),
            before,
            "another call cannot reuse the saved-route token"
        );
        let crew = biorouter::crew::manager().unwrap();
        let saved = crew
            .list()
            .await
            .into_iter()
            .find(|connection| connection.workspace_id == JOIN_WORKSPACE)
            .unwrap();
        crew.remove(&saved.id).await.unwrap();
    });
}

#[test]
#[serial]
fn a_join_retry_refuses_saved_route_identity_access_or_policy_changes_while_trust_waits() {
    run(async {
        for field in ["route", "identity", "access", "policy"] {
            let _ = fs::remove_file(known_hosts());
            let session = chat(&format!("join retry changed {field}")).await;
            let before = attempts(UNKNOWN_HOST);
            let called = call_with(
                &session,
                PersonTool::Change,
                "join",
                join_params(),
                AtTheCard::ChangeJoinBeforeTrust(field),
            )
            .await;
            assert_eq!(called.approvals.len(), 1, "{field}: {:?}", called.approvals);
            assert_eq!(called.host_keys.len(), 1, "{field}: {:?}", called.host_keys);
            assert!(
                called
                    .answer
                    .unwrap_err()
                    .contains("connection changed while its host key was being checked"),
                "{field}"
            );
            assert_eq!(
                attempts(UNKNOWN_HOST) - before,
                1,
                "{field}: no transport retry after a changed record"
            );
            let crew = biorouter::crew::manager().unwrap();
            let saved = crew
                .list()
                .await
                .into_iter()
                .find(|connection| connection.workspace_id == JOIN_WORKSPACE)
                .unwrap();
            crew.remove(&saved.id).await.unwrap();
        }
    });
}

/// An unknown key parks one card with the daemon's fingerprint; a forged allow leaves it
/// waiting; the key in the known-hosts file takes it down, and the check runs again, once.
#[test]
#[serial]
fn an_unknown_key_waits_for_the_person_and_the_step_is_tried_again_once() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let session = chat("unknown key").await;
        let server = format!("you@{UNKNOWN_HOST}");
        let before = attempts(UNKNOWN_HOST);
        let called = check(&session, &server, AtTheCard::ForgeThenAddTheKey).await;

        assert_eq!(called.host_keys.len(), 1, "{:?}", called.host_keys);
        assert_eq!(
            called.host_keys[0],
            json!({
                "kind": "hostKey",
                "host": UNKNOWN_HOST,
                "port": 22,
                "login": server,
                "offered": FINGERPRINT,
                "changed": false,
            })
        );
        let scans = fs::read_to_string(fixture_root().join("keyscan.log")).unwrap_or_default();
        assert!(
            scans
                .lines()
                .any(|line| line == format!("-T 5 -t ed25519 -p 22 {UNKNOWN_HOST}")),
            "the daemon reads the key of the type OpenSSH named, from the host it named: {scans}"
        );
        // The check, and the check tried again on the person's one Allow: one approval, and one
        // more connection.
        assert_eq!(called.approvals.len(), 1, "{:?}", called.approvals);
        assert!(called.approvals[0].starts_with(&format!("Check {server} as you")));
        assert_eq!(attempts(UNKNOWN_HOST) - before, 2);
        let answer = called.answer.expect("the job answers");
        assert_eq!(answer["state"], "failed", "{answer}");
        assert_eq!(answer["error"]["code"], "crew_ssh_unreachable", "{answer}");
        // Nothing is left waiting, and nothing else was carded.
        assert!(!PendingUserActions::global().has_pending_in_session(&session));
        assert_eq!(published_host_keys(&session).len(), 1);
    });
}

/// Trust this key on the card (D2): the person posts the card's own values with their proof,
/// the daemon reads the key again and writes the one line, and the call waiting on the card sees
/// it, takes the card down and tries the step again, once, on the approval already given.
#[test]
#[serial]
fn trusting_the_key_on_the_card_lets_the_step_go_on_once() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let _ = fs::remove_file(fixture_root().join("second_known_hosts"));
        let session = chat("trusted on the card").await;
        let server = format!("you@{UNKNOWN_HOST}");
        let before = attempts(UNKNOWN_HOST);
        let called = check(&session, &server, AtTheCard::TrustTheKey).await;
        assert_eq!(called.host_keys.len(), 1, "{:?}", called.host_keys);
        assert_eq!(called.host_keys[0]["offered"], FINGERPRINT);
        assert_eq!(
            fs::read_to_string(known_hosts()).unwrap(),
            format!("{UNKNOWN_HOST} ssh-ed25519 {BLOB}\n"),
            "exactly the key the daemon read again, in the file ssh -G names first"
        );
        assert_eq!(called.approvals.len(), 1, "{:?}", called.approvals);
        assert_eq!(attempts(UNKNOWN_HOST) - before, 2, "tried again once");
        let answer = called.answer.expect("the job answers");
        assert_eq!(answer["error"]["code"], "crew_ssh_unreachable", "{answer}");
        assert!(!PendingUserActions::global().has_pending_in_session(&session));
    });
}

/// The card waits on every known hosts file `ssh -G` lists for the step's login, not only the
/// profile's: a key the person adds to another file of their configuration takes it down too.
#[test]
#[serial]
fn a_key_added_to_any_file_ssh_reads_lets_the_step_go_on() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let second = fixture_root().join("second_known_hosts");
        let _ = fs::remove_file(&second);
        let session = chat("second file").await;
        let before = attempts(UNKNOWN_HOST);
        let called = check(
            &session,
            &format!("you@{UNKNOWN_HOST}"),
            AtTheCard::AddTheKeyToTheSecondFile,
        )
        .await;
        let _ = fs::remove_file(&second);
        assert_eq!(called.host_keys.len(), 1, "{:?}", called.host_keys);
        assert_eq!(attempts(UNKNOWN_HOST) - before, 2, "tried again once");
        assert!(!PendingUserActions::global().has_pending_in_session(&session));
    });
}

/// A dismissed card is not tried again, and the model is told the step and the fingerprint.
#[test]
#[serial]
fn a_dismissed_card_tells_the_model_the_step_and_the_fingerprint() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let session = chat("dismissed").await;
        let before = attempts(UNKNOWN_HOST);
        let called = check(&session, &format!("you@{UNKNOWN_HOST}"), AtTheCard::Dismiss).await;
        assert_eq!(called.host_keys.len(), 1);
        assert_eq!(called.approvals.len(), 1, "not tried again");
        assert_eq!(attempts(UNKNOWN_HOST) - before, 1);
        let answer = called.answer.expect("the job answers");
        assert_eq!(
            answer["error"]["code"], "crew_ssh_host_key_unknown",
            "{answer}"
        );
        let step = answer["next_step"].as_str().unwrap_or_default();
        assert!(
            step.starts_with("Do not add, scan or accept the key yourself (no ssh-keyscan"),
            "{answer}"
        );
        // The daemon read the key itself: it says so, and never calls it the server's offer.
        assert!(
            step.contains(&format!(
                "This computer read {UNKNOWN_HOST}'s host key fingerprint as {FINGERPRINT}."
            )),
            "{answer}"
        );
        let detail = answer["error"]["detail"].as_str().unwrap_or_default();
        assert!(
            detail.starts_with(&format!(
                "Host key fingerprint read by this computer: {FINGERPRINT}"
            )),
            "{answer}"
        );
        assert!(!detail.contains("Offered host key fingerprint"), "{answer}");
    });
}

/// Every `ssh-keyscan` the daemon ran against `host`.
fn scans_of(host: &str) -> usize {
    fs::read_to_string(fixture_root().join("keyscan.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.ends_with(&format!(" {host}")))
        .count()
}

/// Behind a jump host the daemon reads no key itself: an `ssh-keyscan` from this computer would
/// not take the connection's route, and could reach another machine than the one that refused.
/// The card shows no fingerprint, any key the person adds for the host takes it down, and the
/// check is tried again, once.
#[test]
#[serial]
fn behind_a_jump_host_no_key_is_read_and_any_added_key_counts() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let session = chat("jump host").await;
        let server = format!("you@{UNKNOWN_HOST}");
        let (scans, before) = (scans_of(UNKNOWN_HOST), attempts(UNKNOWN_HOST));
        let called = check_with(
            &session,
            json!({"server": server, "proxy_jump": "gate.hostkey.test"}),
            AtTheCard::ForgeThenAddTheKey,
        )
        .await;
        assert_eq!(called.host_keys.len(), 1, "{:?}", called.host_keys);
        assert_eq!(
            called.host_keys[0],
            json!({
                "kind": "hostKey",
                "host": UNKNOWN_HOST,
                "port": 22,
                "login": server,
                "changed": false,
            }),
            "no fingerprint the daemon could not stand behind"
        );
        assert_eq!(
            scans_of(UNKNOWN_HOST),
            scans,
            "no key read from this computer"
        );
        assert_eq!(called.approvals.len(), 1, "{:?}", called.approvals);
        assert_eq!(attempts(UNKNOWN_HOST) - before, 2, "tried again once");
        let answer = called.answer.expect("the job answers");
        assert_eq!(answer["error"]["code"], "crew_ssh_unreachable", "{answer}");
        assert!(!PendingUserActions::global().has_pending_in_session(&session));
    });
}

/// A changed key parks no card that waits and offers no fix: the call answers at once with the
/// IT wording and both what OpenSSH said and the new fingerprint, and the card it showed is
/// already released.
#[test]
#[serial]
fn a_changed_key_answers_with_the_it_wording_and_waits_for_nothing() {
    run(async {
        let session = chat("changed key").await;
        let server = format!("you@{CHANGED_HOST}");
        // Nobody answers its card: the call must end on its own, at once.
        let started = std::time::Instant::now();
        let called = check(&session, &server, AtTheCard::Watch).await;
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "a changed key's card held the call"
        );
        assert_eq!(called.approvals.len(), 1);
        let answer = called.answer.expect("the job answers");
        assert_eq!(
            answer["error"]["code"], "crew_ssh_host_key_changed",
            "{answer}"
        );
        let step = answer["next_step"].as_str().unwrap_or_default();
        assert!(step.contains("IT team confirms the change"), "{answer}");
        assert!(step.contains(CHANGED_FINGERPRINT), "{answer}");
        assert!(!step.contains("then try again"), "{answer}");
        let shown = published_host_keys(&session);
        assert_eq!(shown.len(), 1, "{shown:?}");
        let (id, card) = &shown[0];
        assert_eq!(
            card,
            &CrewStepRequest::HostKey {
                host: CHANGED_HOST.into(),
                port: 22,
                login: server,
                offered: Some(CHANGED_FINGERPRINT.into()),
                known: None,
                changed: true,
            }
        );
        assert!(!PendingUserActions::global().is_pending(id));
    });
}

/// The call cannot hand the card a fingerprint: a parameter that tries is refused before
/// anything runs, and nothing is parked.
#[test]
#[serial]
fn a_fingerprint_in_the_call_s_parameters_reaches_no_card() {
    run(async {
        let session = chat("parameters").await;
        let host = tool_host::get().expect("the host is installed");
        let refused = host
            .call(PersonToolCall {
                tool: PersonTool::Read,
                action: "setup_check".into(),
                params: json!({"server": format!("you@{UNKNOWN_HOST}"), "offered": FINGERPRINT}),
                session_id: session.clone(),
                tool_call_id: None,
                capability: CallCapability::public_enforced(),
                cancel: CancellationToken::new(),
                approvals: Arc::clone(PendingUserActions::global()),
            })
            .await
            .unwrap_err()
            .to_string();
        assert!(refused.contains("offered"), "{refused}");
        assert!(!PendingUserActions::global().has_pending_in_session(&session));
        assert!(published_host_keys(&session).is_empty());
    });
}
