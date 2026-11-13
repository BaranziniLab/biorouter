//! "Trust this key" (the owner's D2 ruling on AO-12): `POST /crew/host-keys/trust` and
//! `POST /crew/host-keys/read`, against the real routes, the real person gate and the real
//! `ssh-keygen`, with a fake `ssh -G` and a fake `ssh-keyscan`.
//!
//! Pinned here:
//!
//! - no proof that a person acted is a 403, and nothing is read or written;
//! - a fingerprint other than the one the server gives is refused, and nothing is written;
//! - a server the known hosts files already hold a key for (a changed key) is refused, whatever
//!   fingerprint is sent, and the file is left exactly as it was;
//! - success writes exactly one line, under the name OpenSSH looks the key up by, to the first
//!   `UserKnownHostsFile` `ssh -G` names, hashed when `HashKnownHosts` is set; a second call
//!   changes nothing;
//! - a server the person's SSH settings reach through a jump host is never read from here;
//! - a known hosts file of `/dev/null` or `none` holds no key, and a first `UserKnownHostsFile`
//!   like that is never written to.
//!
//! **Offline.** `ssh` and `ssh-keyscan` are this binary, put first on `PATH` by a `#[ctor]`.
//! **Its own binary**: the user-action digest is a process-global `OnceLock`.
#![cfg(unix)]

// Redirects this binary's Biorouter data/config/state dirs at a throwaway root before `main`.
#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use serial_test::serial;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Once},
};
use tower::ServiceExt;

/// Set only in a fake tool's process: the fixture directory.
const FAKE_ROOT: &str = "BIOROUTER_CREW_HOST_KEYS_FAKE_ROOT";
/// Set only in a fake tool's process: which tool it plays, `ssh` or `ssh-keyscan`.
const FAKE_TOOL: &str = "BIOROUTER_CREW_HOST_KEYS_FAKE_TOOL";
/// The user-action key the desktop app would hold; the daemon keeps only its digest.
const USER_KEY: &str = "crew-host-keys-user-action-key";

/// A server whose key this computer does not know yet.
const HOST: &str = "unknown.hostkey.test";
/// A server whose key is hashed into the known hosts file (`HashKnownHosts yes`).
const HASHED_HOST: &str = "hashed.hostkey.test";
/// A server the person's SSH settings reach through a jump host.
const JUMPED_HOST: &str = "jumped.hostkey.test";
/// A server that gives no key.
const SILENT_HOST: &str = "silent.hostkey.test";
/// A server whose known hosts lookup fails: the fake `ssh-keygen` exits 2 for it.
const BROKEN_HOST: &str = "broken.hostkey.test";
/// Servers whose `GlobalKnownHostsFile` is `/dev/null`, and `none`.
const NULL_GLOBAL_HOST: &str = "nullglobal.hostkey.test";
const NONE_GLOBAL_HOST: &str = "noneglobal.hostkey.test";
/// Servers whose `UserKnownHostsFile` is `/dev/null`, and `none`.
const NULL_USER_HOST: &str = "nulluser.hostkey.test";
const NONE_USER_HOST: &str = "noneuser.hostkey.test";
/// What Trust says when the first `UserKnownHostsFile` names no file.
const NO_FILE_TO_SAVE_IN: &str = "keep no known hosts file, so Crew can't save its key. Name one with UserKnownHostsFile, or add the key in a terminal of your own.";
/// The key every fake server gives, as `ssh-keyscan` prints it, and its fingerprint.
const BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIDVMyBQ3psSwstgc8+Mce5VHIZN5GFuu3xE6WV8pfFkD";
const FINGERPRINT: &str = "SHA256:XA01Ou0eChdPDyZiadGxCZ/g+7RnODImHcZEtuRmV8g";
/// Another key, and its fingerprint.
const OTHER_BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

mod fake {
    use super::{BLOB, JUMPED_HOST, SILENT_HOST};
    use std::io::Write;
    use std::path::Path;

    fn log(root: &Path, line: &str) {
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("keyscan.log"))
        {
            let _ = writeln!(log, "{line}");
        }
    }

    /// The value after `flag`, or of `-o NAME=value`.
    fn option<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| pair[1].as_str())
    }

    fn config(args: &[String], name: &str) -> Option<String> {
        args.windows(2).find_map(|pair| {
            (pair[0] == "-o")
                .then(|| pair[1].strip_prefix(&format!("{name}=")).map(str::to_owned))
                .flatten()
        })
    }

    pub fn run(root: &Path, tool: &str) -> i32 {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let host = args.last().cloned().unwrap_or_default();
        let host = host
            .rsplit_once('@')
            .map_or(host.as_str(), |(_, host)| host)
            .to_owned();
        if tool == "ssh-keyscan" {
            log(root, &args.join(" "));
            if host != SILENT_HOST {
                println!("# {host}:22 SSH-2.0-OpenSSH_9.6");
                println!("{host} ssh-ed25519 {BLOB}");
            }
            return 0;
        }
        if args.first().map(String::as_str) != Some("-G") {
            eprintln!("fake ssh only resolves");
            return 255;
        }
        let port = option(&args, "-p").unwrap_or("22");
        let user_file = match host.as_str() {
            super::NULL_USER_HOST => "/dev/null".to_owned(),
            super::NONE_USER_HOST => "none".to_owned(),
            _ => config(&args, "UserKnownHostsFile").unwrap_or_default(),
        };
        let global_file = match host.as_str() {
            super::NULL_GLOBAL_HOST => "/dev/null".to_owned(),
            super::NONE_GLOBAL_HOST => "none".to_owned(),
            _ => root.join("global_known_hosts").display().to_string(),
        };
        let mut out = std::io::stdout().lock();
        let _ = writeln!(
            out,
            "host {host}\nhostname {host}\nport {port}\nproxycommand none"
        );
        if host == JUMPED_HOST {
            let _ = writeln!(out, "proxyjump gate.hostkey.test");
        }
        let hash = if host == super::HASHED_HOST {
            "yes"
        } else {
            "no"
        };
        let _ = writeln!(out, "hashknownhosts {hash}");
        let _ = writeln!(out, "userknownhostsfile {user_file}");
        let _ = writeln!(out, "globalknownhostsfile {global_file}");
        0
    }
}

fn fixture_root() -> PathBuf {
    std::env::temp_dir().join(format!("biorouter-crew-host-keys-{}", std::process::id()))
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
        fs::write(root.join("profile/home/.ssh/config"), "")?;
        let executable = std::env::current_exe()?;
        let (root_text, executable_text) = (root.display(), executable.display());
        if format!("{root_text}{executable_text}").contains('\'') {
            return Err(std::io::Error::other("a fixture path contains a quote"));
        }
        // `ssh-keygen` is the real one, except that a lookup of BROKEN_HOST fails, as a lookup
        // in a file it can't read does.
        let real_keygen = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .map(|directory| directory.join("ssh-keygen"))
            .find(|candidate| candidate.is_file());
        let keygen_tail = match &real_keygen {
            Some(real) if !real.display().to_string().contains('\'') => {
                format!("exec '{}' \"$@\"\n", real.display())
            }
            _ => "exit 127\n".to_owned(),
        };
        let keygen = root.join("bin/ssh-keygen");
        fs::write(
            &keygen,
            format!(
                "#!/bin/sh\nfor word in \"$@\"; do [ \"$word\" = '{BROKEN_HOST}' ] && exit 2; done\n{keygen_tail}"
            ),
        )?;
        fs::set_permissions(&keygen, fs::Permissions::from_mode(0o700))?;
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
            "crew_host_keys_trust: could not prepare the fixture at {}: {error}. Refusing to run: \
             the daemon would reach the real ssh and the person's known hosts file.",
            root.display()
        );
        std::process::abort();
    }
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::set_var("PATH", format!("{}:{path}", root.join("bin").display()));
    std::env::set_var("BIOROUTER_DEV_PROFILE_ROOT", root.join("profile"));
}

#[ctor::dtor]
fn remove_fixture() {
    if std::env::var_os(FAKE_ROOT).is_none() {
        let _ = fs::remove_dir_all(fixture_root());
    }
}

static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("a runtime")
});

fn run<F: std::future::Future<Output = ()>>(test: F) {
    static DIGEST: Once = Once::new();
    DIGEST.call_once(|| {
        let digest: [u8; 32] = <sha2::Sha256 as sha2::Digest>::digest(USER_KEY.as_bytes()).into();
        biorouter_server::auth::install_user_action_digest(Some(digest));
    });
    RUNTIME.block_on(test);
}

/// A request to the host-key routes, with the person's proof when `proven`.
async fn post(path: &str, body: Value, proven: bool) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if proven {
        request = request.header("X-User-Action", USER_KEY);
    }
    let response = biorouter_server::routes::crew_host_keys::routes()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
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

async fn trust(host: &str, port: u16, fingerprint: &str, proven: bool) -> (StatusCode, Value) {
    post(
        "/crew/host-keys/trust",
        json!({"host": host, "port": port, "login": format!("you@{host}"), "fingerprint": fingerprint}),
        proven,
    )
    .await
}

fn known_hosts_text() -> String {
    fs::read_to_string(known_hosts()).unwrap_or_default()
}

fn scans() -> usize {
    fs::read_to_string(fixture_root().join("keyscan.log"))
        .unwrap_or_default()
        .lines()
        .count()
}

/// The fingerprints `ssh-keygen -F` finds for `lookup` in the known hosts file.
fn found(lookup: &str) -> Vec<String> {
    let output = std::process::Command::new("ssh-keygen")
        .args(["-l", "-F", lookup, "-f"])
        .arg(known_hosts())
        .output()
        .expect("ssh-keygen runs");
    biorouter::crew::host_keys::listed_fingerprints(&String::from_utf8_lossy(&output.stdout))
}

#[test]
#[serial]
fn without_a_person_s_proof_nothing_is_read_or_written() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let before = scans();
        let (status, body) = trust(HOST, 22, FINGERPRINT, false).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["code"], "crew_user_action_required", "{body}");
        let (status, body) = post(
            "/crew/host-keys/read",
            json!({"host": HOST, "port": 22}),
            false,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(scans(), before, "no key was read");
        assert!(known_hosts_text().is_empty(), "nothing was written");
    });
}

#[test]
#[serial]
fn a_fingerprint_the_server_does_not_give_is_refused() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let other = biorouter::crew::host_keys::fingerprint_of(OTHER_BLOB).unwrap();
        let (status, body) = trust(HOST, 22, &other, true).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "crew_ssh_host_key_changed", "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("doesn't match"),
            "{body}"
        );
        assert!(known_hosts_text().is_empty(), "nothing was written");
    });
}

/// A server the file already holds another key for is a changed key: never replaced, whatever
/// fingerprint is sent, and the file is left exactly as it was.
#[test]
#[serial]
fn a_changed_key_is_always_refused() {
    run(async {
        let before = format!("{HOST} ssh-ed25519 {OTHER_BLOB}\n");
        fs::write(known_hosts(), &before).unwrap();
        let scanned = scans();
        for fingerprint in [
            FINGERPRINT.to_owned(),
            biorouter::crew::host_keys::fingerprint_of(OTHER_BLOB).unwrap(),
        ] {
            let (status, body) = trust(HOST, 22, &fingerprint, true).await;
            if fingerprint == FINGERPRINT {
                assert_eq!(status, StatusCode::CONFLICT, "{body}");
                assert_eq!(body["code"], "crew_ssh_host_key_changed", "{body}");
                assert!(
                    body["error"].as_str().unwrap().contains("already knows"),
                    "{body}"
                );
            } else {
                // Exactly the key already there: nothing to add, and nothing changed.
                assert_eq!(status, StatusCode::OK, "{body}");
                assert_eq!(body["already_trusted"], true, "{body}");
            }
        }
        assert_eq!(known_hosts_text(), before, "the file is untouched");
        assert_eq!(scans(), scanned, "a known server is never read again");
        // A revoked key for the host is known too.
        let revoked = format!("@revoked {HOST} ssh-ed25519 {BLOB}\n");
        fs::write(known_hosts(), &revoked).unwrap();
        let (status, body) = trust(HOST, 22, FINGERPRINT, true).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(known_hosts_text(), revoked);
    });
}

/// When `ssh-keygen` can't say what a known hosts file holds, the file could hold another key
/// for the server, so nothing is read from the server and nothing is written.
#[test]
#[serial]
fn a_known_hosts_file_that_cannot_be_read_is_never_taken_for_none() {
    run(async {
        let before = "other.example.edu ssh-ed25519 AAAA\n";
        fs::write(known_hosts(), before).unwrap();
        let scanned = scans();
        let (status, body) = trust(BROKEN_HOST, 22, FINGERPRINT, true).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "crew_request_refused", "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("can't tell"),
            "{body}"
        );
        assert_eq!(known_hosts_text(), before, "the file is untouched");
        assert_eq!(scans(), scanned, "nothing was read from the server");
    });
}

/// A key filed under the lower case name, hashed, is the server's key whatever case the request
/// spells the name in: another key is refused, not added beside it.
#[test]
#[serial]
fn a_hashed_key_is_found_whatever_case_the_name_is_spelled_in() {
    run(async {
        fs::write(known_hosts(), format!("{HOST} ssh-ed25519 {OTHER_BLOB}\n")).unwrap();
        let hashed = std::process::Command::new("ssh-keygen")
            .arg("-H")
            .arg("-f")
            .arg(known_hosts())
            .output()
            .expect("ssh-keygen runs");
        assert!(hashed.status.success(), "{hashed:?}");
        let _ = fs::remove_file(known_hosts().with_extension("old"));
        let before = known_hosts_text();
        assert!(before.starts_with("|1|"), "{before}");
        let (status, body) = trust(&HOST.to_ascii_uppercase(), 22, FINGERPRINT, true).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "crew_ssh_host_key_changed", "{body}");
        assert_eq!(known_hosts_text(), before, "the file is untouched");
    });
}

/// Success writes one line, the line ssh finds the key by; a second call changes nothing.
#[test]
#[serial]
fn trusting_writes_one_line_once() {
    run(async {
        fs::write(known_hosts(), "other.example.edu ssh-ed25519 AAAA").unwrap();
        let (status, body) = trust(HOST, 22, FINGERPRINT, true).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["already_trusted"], false, "{body}");
        assert_eq!(body["fingerprint"], FINGERPRINT);
        assert_eq!(
            body["known_hosts_file"],
            known_hosts().display().to_string(),
            "the first UserKnownHostsFile ssh -G names"
        );
        assert_eq!(
            known_hosts_text(),
            format!("other.example.edu ssh-ed25519 AAAA\n{HOST} ssh-ed25519 {BLOB}\n")
        );
        assert_eq!(found(HOST), vec![FINGERPRINT.to_owned()]);
        let (status, body) = trust(HOST, 22, FINGERPRINT, true).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["already_trusted"], true, "{body}");
        assert_eq!(known_hosts_text().matches(BLOB).count(), 1, "one line");
    });
}

/// `GlobalKnownHostsFile /dev/null`, or `none`, names no file, so it holds no key: Trust reads
/// the user file alone and writes the key there. A live configuration with an empty user file
/// beside `/dev/null` was refused as "Crew couldn't read /dev/null" (2026-11-12).
#[test]
#[serial]
fn a_null_global_known_hosts_file_holds_no_key() {
    run(async {
        for host in [NULL_GLOBAL_HOST, NONE_GLOBAL_HOST] {
            fs::write(known_hosts(), "").unwrap();
            let (status, body) = trust(host, 22, FINGERPRINT, true).await;
            assert_eq!(status, StatusCode::OK, "{host}: {body}");
            assert_eq!(body["already_trusted"], false, "{body}");
            assert_eq!(
                body["known_hosts_file"],
                known_hosts().display().to_string(),
                "{body}"
            );
            assert_eq!(known_hosts_text(), format!("{host} ssh-ed25519 {BLOB}\n"));
            assert_eq!(found(host), vec![FINGERPRINT.to_owned()]);
        }
    });
}

/// When the first `UserKnownHostsFile` is `/dev/null`, or `none`, a key written there would be
/// lost: Trust says so, reads nothing from the server and writes nothing.
#[test]
#[serial]
fn a_null_user_known_hosts_file_is_never_written_to() {
    run(async {
        for host in [NULL_USER_HOST, NONE_USER_HOST] {
            let _ = fs::remove_file(known_hosts());
            let scanned = scans();
            let (status, body) = trust(host, 22, FINGERPRINT, true).await;
            assert_eq!(status, StatusCode::CONFLICT, "{host}: {body}");
            assert_eq!(body["code"], "crew_request_refused", "{body}");
            assert_eq!(
                body["error"],
                format!("Your SSH settings for {host} {NO_FILE_TO_SAVE_IN}"),
                "{body}"
            );
            assert_eq!(scans(), scanned, "nothing was read from the server");
            assert!(known_hosts_text().is_empty(), "nothing was written");
        }
    });
}

#[test]
#[serial]
fn a_key_off_port_22_is_filed_under_host_and_port() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let (status, body) = trust(HOST, 2222, FINGERPRINT, true).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            known_hosts_text(),
            format!("[{HOST}]:2222 ssh-ed25519 {BLOB}\n")
        );
        assert!(found(HOST).is_empty(), "port 22's name holds nothing");
        assert_eq!(
            found(&format!("[{HOST}]:2222")),
            vec![FINGERPRINT.to_owned()]
        );
    });
}

#[test]
#[serial]
fn a_hashed_known_hosts_file_gets_a_hashed_line() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let (status, body) = trust(HASHED_HOST, 22, FINGERPRINT, true).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let text = known_hosts_text();
        assert!(text.starts_with("|1|"), "{text}");
        assert!(!text.contains(HASHED_HOST), "{text}");
        assert_eq!(found(HASHED_HOST), vec![FINGERPRINT.to_owned()]);
    });
}

/// Through a jump host a key read from here could be another machine's: nothing is read or
/// written.
#[test]
#[serial]
fn a_server_behind_a_jump_host_is_never_read_from_here() {
    run(async {
        let _ = fs::remove_file(known_hosts());
        let before = scans();
        let (status, body) = trust(JUMPED_HOST, 22, FINGERPRINT, true).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "crew_request_refused", "{body}");
        let (status, body) = post(
            "/crew/host-keys/read",
            json!({"host": JUMPED_HOST, "port": 22}),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(scans(), before);
        assert!(known_hosts_text().is_empty());
    });
}

#[test]
#[serial]
fn the_read_route_shows_what_this_computer_reads() {
    run(async {
        let (status, body) = post(
            "/crew/host-keys/read",
            json!({"host": HOST, "port": 2222, "login": format!("you@{HOST}")}),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            json!({"host": HOST, "port": 2222, "fingerprint": FINGERPRINT, "key_type": "ssh-ed25519"})
        );
        let (status, body) = post(
            "/crew/host-keys/read",
            json!({"host": SILENT_HOST, "port": 22}),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
        assert_eq!(body["code"], "crew_ssh_unreachable", "{body}");
    });
}

/// A body the route does not take, or a name that could be an option, is refused before
/// anything runs.
#[test]
#[serial]
fn only_plain_requests_are_taken() {
    run(async {
        let before = scans();
        for body in [
            json!({"host": HOST, "port": 22, "fingerprint": FINGERPRINT, "key": format!("ssh-ed25519 {BLOB}")}),
            json!({"host": "-oProxyCommand=x", "port": 22, "fingerprint": FINGERPRINT}),
            json!({"host": HOST, "port": 22, "login": "-F/tmp/x", "fingerprint": FINGERPRINT}),
            json!({"host": HOST, "port": 22, "fingerprint": "SHA256:short"}),
        ] {
            let (status, answer) = post("/crew/host-keys/trust", body.clone(), true).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body} {answer}");
            assert_eq!(answer["code"], "crew_request_invalid", "{answer}");
        }
        assert_eq!(scans(), before);
    });
}
