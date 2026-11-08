#![cfg(test)]
#![cfg(all(feature = "dictation", unix))]

#[path = "../src/test_sandbox.rs"]
mod test_sandbox;

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct TerminalRun {
    output: String,
    root: tempfile::TempDir,
    provider_requests: usize,
}

fn terminal(
    classic: bool,
    extra_env: &[(&str, &str)],
    accepted_turn: bool,
    steps: &[(&str, &[u8])],
) -> TerminalRun {
    terminal_with_session(classic, extra_env, accepted_turn, steps, |_| {
        vec![
            "session".into(),
            "--provider".into(),
            "anthropic".into(),
            "--model".into(),
            "claude-sonnet-4-5".into(),
        ]
    })
}

fn terminal_with_session(
    classic: bool,
    extra_env: &[(&str, &str)],
    accepted_turn: bool,
    steps: &[(&str, &[u8])],
    session_args: impl FnOnce(&std::path::Path) -> Vec<String>,
) -> TerminalRun {
    let root = tempfile::tempdir().expect("sandbox");
    let home = root.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    let model = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/whisper.cpp/source/models/for-tests-ggml-base.en.bin");
    let provider = accepted_turn.then(fake_anthropic);
    let args = session_args(root.path());
    let (mut master, mut slave) = (-1, -1);
    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0, "openpty: {}", std::io::Error::last_os_error());
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    let size = libc::winsize {
        ws_row: 40,
        ws_col: 120,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe { libc::ioctl(slave.as_raw_fd(), libc::TIOCSWINSZ, &size) },
        0
    );
    let mut command = Command::new(env!("CARGO_BIN_EXE_biorouter"));
    command
        .args(&args)
        .current_dir(&home)
        .env("BIOROUTER_PATH_ROOT", root.path())
        .env("HOME", &home)
        .env("BIOROUTER_DISABLE_KEYRING", "true")
        .env("ANTHROPIC_API_KEY", "placeholder-never-sent")
        .env("BIOROUTER_DICTATION_CAPTURE_TEST_SOURCE", "tone")
        .env("BIOROUTER_DICTATION_TEST_MODEL", model)
        .env("BIOROUTER_DICTATION_TEST_TEXT", "EPAS1 levels rose")
        .env("TERM", "xterm")
        .env("NO_COLOR", "1")
        .stdin(Stdio::from(slave.try_clone().expect("stdin")))
        .stdout(Stdio::from(slave.try_clone().expect("stdout")))
        .stderr(Stdio::from(slave));
    if classic {
        command.env("BIOROUTER_CLI_CLASSIC", "1");
    }
    if let Some((host, _, _)) = &provider {
        command.env("ANTHROPIC_HOST", host).env("OLLAMA_HOST", host);
    }
    for (key, value) in extra_env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("session process");
    drop(command);
    let mut reader = std::fs::File::from(master);
    let mut writer = reader.try_clone().expect("writer");
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let seen = Arc::clone(&output);
    let reader_done = Arc::new(AtomicBool::new(false));
    let stop_reader = Arc::clone(&reader_done);
    let reader_thread = std::thread::spawn(move || {
        let mut bytes = [0u8; 4096];
        let mut stop_deadline = None;
        loop {
            if stop_reader.load(Ordering::SeqCst) && stop_deadline.is_none() {
                stop_deadline = Some(Instant::now() + Duration::from_millis(250));
            }
            if stop_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
            let wait_ms = stop_deadline
                .map(|deadline| {
                    deadline
                        .saturating_duration_since(Instant::now())
                        .as_millis()
                        .min(100) as i32
                })
                .unwrap_or(100);
            let mut fd = libc::pollfd {
                fd: reader.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut fd, 1, wait_ms) };
            if ready < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
            if ready > 0 && fd.revents & libc::POLLIN != 0 {
                match reader.read(&mut bytes) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => seen.lock().unwrap().extend_from_slice(&bytes[..count]),
                }
            } else if ready > 0 && fd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                break;
            }
            if stop_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
        }
    });
    let mut offset = 0;
    for &(prompt, key) in steps {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if String::from_utf8_lossy(&output.lock().unwrap()[offset..]).contains(prompt) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {prompt}: {}",
                String::from_utf8_lossy(&output.lock().unwrap())
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(if prompt == "REC" {
            700
        } else {
            300
        }));
        offset = output.lock().unwrap().len();
        if key == b"\x1b~" {
            writer.write_all(b"\x1b").expect("cancel dictation");
            std::thread::sleep(Duration::from_millis(200));
            writer.write_all(b"~").expect("probe idle composer");
        } else {
            writer.write_all(key).expect("type key");
        }
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    while child.try_wait().expect("poll child").is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().expect("kill stalled session");
    }
    let status = child.wait().expect("session exit");
    drop(writer);
    reader_done.store(true, Ordering::SeqCst);
    let _ = reader_thread.join();
    let text = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
    assert!(status.success(), "session failed: {text}");
    let provider_requests = if let Some((_, done, server)) = provider {
        done.store(true, std::sync::atomic::Ordering::SeqCst);
        server.join().expect("provider responder")
    } else {
        0
    };
    TerminalRun {
        output: text,
        root,
        provider_requests,
    }
}

fn fake_anthropic() -> (
    String,
    Arc<std::sync::atomic::AtomicBool>,
    std::thread::JoinHandle<usize>,
) {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback responder");
    listener.set_nonblocking(true).unwrap();
    let host = format!("http://{}", listener.local_addr().unwrap());
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = Arc::clone(&done);
    let server = std::thread::spawn(move || {
        let body = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_terminal\",\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut requests = 0;
        while Instant::now() < deadline && !stopped.load(std::sync::atomic::Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut request = [0u8; 8192];
                    let _ = stream.read(&mut request);
                    let _ = stream.write_all(response.as_bytes());
                    requests += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(error) => panic!("provider responder: {error}"),
            }
        }
        requests
    });
    (host, done, server)
}

fn observed_correction(run: &TerminalRun, term: &str, from: &str) {
    let store = biorouter::security::dictionary_store::store::DictionaryStore::open(
        run.root.path().join("config/dictionary"),
    );
    let view = store.view().expect("dictionary after accepted turn");
    let rows: Vec<_> = view
        .entries()
        .iter()
        .filter(|entry| entry.written == term)
        .collect();
    assert_eq!(rows.len(), 1, "one corrected term");
    assert_eq!(rows[0].misrecognitions.len(), 1, "one correction pair");
    assert_eq!(rows[0].misrecognitions[0].text, from);
    assert_eq!(rows[0].misrecognitions[0].count, 1);
    assert_eq!(view.file().sightings.len(), 1, "one accepted observation");
}

#[test]
fn classic_ctrl_t_uses_fake_capture_and_requires_another_enter() {
    if !cfg!(debug_assertions) {
        return;
    }
    let run = terminal(
        true,
        &[],
        false,
        &[
            ("❯", b"Please check \x14"),
            ("Starting microphone", b""),
            ("REC", b"\x14"),
            ("EPAS1 levels rose", b"\x03"),
            ("❯", b"/exit\r"),
        ],
    );
    assert!(
        run.output.contains("Please check EPAS1 levels rose"),
        "{}",
        run.output
    );
    assert!(run.output.contains("Closing session"), "{}", run.output);
}

#[test]
fn tui_ctrl_t_escape_then_ctrl_t_stop_keeps_text_in_composer() {
    if !cfg!(debug_assertions) {
        return;
    }
    let run = terminal(
        false,
        &[],
        false,
        &[
            ("Send a message", b"\x14"),
            ("REC", b"\x1b~"),
            ("~", b"\x7f\x14"),
            ("REC", b"\x14"),
            ("EPAS1 levels rose", b"\x03\x04"),
        ],
    );
    assert!(run.output.contains("EPAS1 levels rose"), "{}", run.output);
}

#[test]
fn a_newline_shortcut_collision_names_the_disabled_dictation_key() {
    let run = terminal(
        true,
        &[("BIOROUTER_CLI_NEWLINE_KEY", "t")],
        false,
        &[("Ctrl+T dictation is off", b"/exit\r")],
    );
    assert!(
        run.output.contains("BIOROUTER_CLI_DICTATION_KEY"),
        "{}",
        run.output
    );
}

#[test]
fn tui_ctrl_c_cancels_a_recording_without_inserting_text() {
    if !cfg!(debug_assertions) {
        return;
    }
    let run = terminal(
        false,
        &[],
        false,
        &[
            ("Send a message", b"\x14"),
            ("REC", b"\x03~"),
            ("~", b"\x7f/exit\r\r"),
        ],
    );
    assert!(
        !run.output.contains("EPAS1 levels rose"),
        "cancelled capture inserted text: {}",
        run.output
    );
}

#[test]
fn classic_pty_insert_edit_enter_teaches_exactly_one_correction_pair() {
    if !cfg!(debug_assertions) {
        return;
    }
    let run = terminal(
        true,
        &[("BIOROUTER_DICTATION_TEST_TEXT", "Epas one")],
        true,
        &[
            ("❯", b"\x14"),
            ("REC", b"\x14"),
            ("Epas one", b"\x01\x0bEPAS1\r"),
            ("ok", b"/exit\r"),
        ],
    );
    assert!(
        run.provider_requests >= 1,
        "the edited message was not accepted by the provider"
    );
    observed_correction(&run, "EPAS1", "Epas one");
}

#[test]
fn tui_pty_insert_edit_enter_teaches_exactly_one_correction_pair() {
    if !cfg!(debug_assertions) {
        return;
    }
    let run = terminal(
        false,
        &[("BIOROUTER_DICTATION_TEST_TEXT", "Epas one")],
        true,
        &[
            ("Send a message", b"\x14"),
            ("REC", b"\x14"),
            ("Epas one", b"\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7fEPAS1\r"),
            ("ok", b"/exit\r\r"),
        ],
    );
    assert!(
        run.provider_requests >= 1,
        "the edited message was not accepted by the provider"
    );
    observed_correction(&run, "EPAS1", "Epas one");
}

fn private_add_from_session(touched: bool) -> TerminalRun {
    use biorouter::privacy::affiliation::InstitutionId;
    use biorouter::privacy::SessionClassification;
    use biorouter::session::session_manager::SessionType;
    use biorouter::session::SessionManager;

    let run = terminal_with_session(
        true,
        &[],
        true,
        &[
            ("❯", b"/dictionary add EPAS1\r"),
            ("Added EPAS1 to the private dictionary.", b"/exit\r"),
        ],
        |root| {
            let runtime = tokio::runtime::Runtime::new().expect("session seed runtime");
            let working_dir = std::fs::canonicalize(root.join("home")).expect("sandbox home");
            let session_id = runtime.block_on(async {
                let manager = SessionManager::new(root.join("data"));
                let session = manager
                    .create_session(working_dir, "Private dictation".into(), SessionType::User)
                    .await
                    .unwrap();
                manager
                    .update(&session.id)
                    .raise_privacy(SessionClassification::Private, "fixture")
                    .apply()
                    .await
                    .unwrap();
                if touched {
                    manager
                        .record_session_affiliation(&session.id, InstitutionId::new("ucsf"))
                        .await
                        .unwrap();
                }
                manager.close().await;
                session.id
            });
            vec![
                "session".into(),
                "--resume".into(),
                "--session-id".into(),
                session_id,
                "--provider".into(),
                "ollama".into(),
                "--model".into(),
                "biorouter-fixture".into(),
            ]
        },
    );
    assert_eq!(
        run.provider_requests, 0,
        "the dictionary command must not start a provider turn"
    );
    run
}

#[test]
fn private_session_dictionary_add_command_stamps_touched_ucsf() {
    let run = private_add_from_session(true);
    let store = biorouter::security::dictionary_store::store::DictionaryStore::open(
        run.root.path().join("config/dictionary"),
    );
    let view = store.view().expect("dictionary from session command");
    let row = view
        .entries()
        .iter()
        .find(|entry| entry.written == "EPAS1")
        .expect("added term");
    assert_eq!(row.affiliation.as_ref().unwrap().institutions, vec!["ucsf"]);
}

#[test]
fn private_session_dictionary_add_command_preserves_empty_affiliation() {
    let run = private_add_from_session(false);
    let store = biorouter::security::dictionary_store::store::DictionaryStore::open(
        run.root.path().join("config/dictionary"),
    );
    let view = store.view().expect("dictionary from session command");
    let row = view
        .entries()
        .iter()
        .find(|entry| entry.written == "EPAS1")
        .expect("added term");
    assert!(row.affiliation.is_none());
}

#[tokio::test]
async fn private_session_add_stamps_touched_institution_and_empty_record_stays_unclaimed() {
    if std::env::var_os("BIOROUTER_DICTATION_AUTH_FIXTURE_CHILD").is_none() {
        let (mut master, mut slave) = (-1, -1);
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0);
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "private_session_add_stamps_touched_institution_and_empty_record_stays_unclaimed",
                "--nocapture",
            ])
            .env("BIOROUTER_DICTATION_AUTH_FIXTURE_CHILD", "1")
            .env("TERM", "xterm")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let reader = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = std::fs::File::from(master).read_to_string(&mut text);
            text
        });
        let result = child.wait_with_output().unwrap();
        let diagnostics = reader.join().unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            diagnostics
        );
        return;
    }
    use biorouter::privacy::affiliation::{InstitutionId, ModelAffiliation};
    use biorouter::privacy::{ProviderTier, SessionClassification};
    use biorouter::session::session_manager::SessionType;
    use biorouter::session::SessionManager;

    let manager = SessionManager::instance();
    let root = tempfile::tempdir().unwrap();
    let touched = manager
        .create_session(
            root.path().to_path_buf(),
            "Touched".into(),
            SessionType::User,
        )
        .await
        .unwrap();
    let empty = manager
        .create_session(root.path().to_path_buf(), "Empty".into(), SessionType::User)
        .await
        .unwrap();
    for id in [&touched.id, &empty.id] {
        manager
            .update(id)
            .raise_privacy(SessionClassification::Private, "test")
            .apply()
            .await
            .unwrap();
    }
    manager
        .record_session_affiliation(&touched.id, InstitutionId::new("ucsf"))
        .await
        .unwrap();
    let model = Some((ProviderTier::Private, Some(ModelAffiliation::Local)));
    biorouter_cli::dictation::frontend::add_term(&touched.id, model, "EPAS1")
        .await
        .unwrap();
    biorouter_cli::dictation::frontend::add_term(&empty.id, model, "ABCA1")
        .await
        .unwrap();
    let dictated = biorouter_cli::dictation::frontend::DictatedSpan::new(
        String::new(),
        "Epas one".into(),
        String::new(),
    );
    biorouter_cli::dictation::frontend::observe_correction(&touched.id, model, &dictated, "EPAS1")
        .await;
    let store = biorouter::security::dictionary_store::open_store();
    let view = store.view().unwrap();
    let epas = view
        .entries()
        .iter()
        .find(|entry| entry.written == "EPAS1")
        .unwrap();
    let abca = view
        .entries()
        .iter()
        .find(|entry| entry.written == "ABCA1")
        .unwrap();
    assert_eq!(
        epas.affiliation.as_ref().unwrap().institutions,
        vec!["ucsf"]
    );
    assert_eq!(
        epas.misrecognitions.len(),
        1,
        "one accepted correction is attached to the private row"
    );
    assert!(abca.affiliation.is_none());
    let (mut target, standing) =
        biorouter::dictation::cli_bias(biorouter::dictation::CliTerminal {
            session: Some(SessionClassification::Private),
            interactive: true,
            caller_private: true,
            agent_shell: false,
        });
    target.affiliation = Some(ModelAffiliation::institution(InstitutionId::new(
        "stanford",
    )));
    let other = biorouter::dictation::bias_set(&view, &target, standing, true);
    assert!(other.entries().iter().all(|entry| entry.written != "EPAS1"));
    target.affiliation = Some(ModelAffiliation::Local);
    let local = biorouter::dictation::bias_set(&view, &target, standing, true);
    assert!(local.entries().iter().any(|entry| entry.written == "EPAS1"));

    use std::os::unix::fs::PermissionsExt;
    let policy_path = biorouter::config::paths::Paths::managed_policy_path().unwrap();
    std::fs::create_dir_all(policy_path.parent().unwrap()).unwrap();
    std::fs::set_permissions(
        policy_path.parent().unwrap(),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(&policy_path, "dictation:\n  dictionary: false\n").unwrap();
    std::fs::set_permissions(&policy_path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(biorouter::managed::ManagedPolicy::load()
        .dictation()
        .dictionary
        .reason()
        .is_some());
    let before = std::fs::read(store.store_path()).unwrap();
    let second = biorouter_cli::dictation::frontend::DictatedSpan::new(
        String::new(),
        "Epas one".into(),
        String::new(),
    );
    biorouter_cli::dictation::frontend::observe_correction(&touched.id, model, &second, "EPAS1")
        .await;
    assert_eq!(
        std::fs::read(store.store_path()).unwrap(),
        before,
        "managed deny leaves the observation and counters unchanged"
    );
    assert!(
        biorouter_cli::dictation::frontend::add_term(&touched.id, model, "FOXP2")
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(store.store_path()).unwrap(),
        before,
        "managed deny leaves additions unchanged"
    );
    std::fs::remove_file(&policy_path).unwrap();
    std::env::set_var("BIOROUTER_TERMINAL", "1");
    let agent_shell_pair = biorouter_cli::dictation::frontend::DictatedSpan::new(
        String::new(),
        "Epas one".into(),
        String::new(),
    );
    biorouter_cli::dictation::frontend::observe_correction(
        &touched.id,
        model,
        &agent_shell_pair,
        "EPAS1",
    )
    .await;
    assert!(
        biorouter_cli::dictation::frontend::add_term(&touched.id, model, "FOXP2")
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(store.store_path()).unwrap(),
        before,
        "agent shell leaves the dictionary byte identical"
    );
}
