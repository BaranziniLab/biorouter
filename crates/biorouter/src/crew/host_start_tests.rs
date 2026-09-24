//! D-HOST: "Start it for me" runs exactly the dialog's fixed commands, over non-interactive
//! `ssh` as the login the person typed, for this computer's own host setup only, and reads the
//! result as a paste is read.

use super::host_start::{
    cancel_host_start, host_start_command, host_start_status, read_start_output, HostStartRefused,
    HostStartRequest, HostStartState, StartOutput,
};
use super::*;
use std::fs;
use std::path::Path;
use std::time::Duration;

const KEY: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

#[test]
fn the_command_is_the_dialogs_fixed_text() {
    assert_eq!(
        host_start_command("chen-lab", KEY).unwrap(),
        format!(
            "umask 077\n\
             mkdir -p \"$HOME/.local/share/biorouter-crew\"\n\
             \"$HOME/.local/bin/biorouter-crew\" start --state-dir \"$HOME/.local/share/biorouter-crew/chen-lab\" --name chen-lab --bootstrap-key {KEY}\n\
             \"$HOME/.local/bin/biorouter-crew\" status --state-dir \"$HOME/.local/share/biorouter-crew/chen-lab\""
        )
    );
}

/// The dialog shows `hostStartCommands` from `joinText.ts`; the daemon runs
/// [`host_start_command`]. Filling the dialog's own template lines with the same values must
/// give the same text, so what runs is character for character what the person saw.
#[test]
fn the_command_matches_the_host_dialogs_template() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ui/desktop/src/components/crew/onboarding/joinText.ts");
    let source = fs::read_to_string(&path).expect("the Host dialog's command template");
    let start = source
        .find("export function hostStartCommands(")
        .expect("hostStartCommands");
    let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
    let template = |line: &'static str| -> &'static str {
        assert!(body.contains(line), "{line} is not in hostStartCommands");
        line
    };
    let state_dir_line =
        template("const stateDir = `\"$HOME/.local/share/biorouter-crew/${slug}\"`;");
    let lines = [
        template("'umask 077',"),
        template("'mkdir -p \"$HOME/.local/share/biorouter-crew\"',"),
        template("`\"$HOME/.local/bin/biorouter-crew\" start --state-dir ${stateDir} --name ${slug} --bootstrap-key ${bootstrapKey}`,"),
        template("`\"$HOME/.local/bin/biorouter-crew\" status --state-dir ${stateDir}`,"),
    ];
    assert!(body.contains("].join('\\n');"), "joined with newlines");
    let slug = "chen-lab";
    let state_dir = state_dir_line
        .trim_start_matches("const stateDir = `")
        .trim_end_matches("`;")
        .replace("${slug}", slug);
    let filled: Vec<String> = lines
        .iter()
        .map(|line| {
            line.trim_end_matches(',')
                .trim_matches(|c| c == '\'' || c == '`')
                .replace("${stateDir}", &state_dir)
                .replace("${slug}", slug)
                .replace("${bootstrapKey}", KEY)
        })
        .collect();
    assert_eq!(host_start_command(slug, KEY).unwrap(), filled.join("\n"));
}

#[test]
fn nothing_outside_the_grammar_reaches_the_command() {
    for slug in [
        "",
        "Lab",
        "lab; rm -rf ~",
        "lab$(id)",
        "lab`id`",
        "lab\nid",
        "-lab",
        "lab-",
        "a b",
        "lab\"",
        "4b4b4b4b-4b4b-44b4-84b4-4b4b4b4b4b4b",
    ] {
        assert!(host_start_command(slug, KEY).is_err(), "{slug:?}");
    }
    for key in [
        "",
        &KEY[..63],
        &KEY.to_ascii_uppercase(),
        &format!("{}; id", &KEY[..60]),
        &format!("{KEY}0"),
    ] {
        assert!(host_start_command("lab", key).is_err(), "{key:?}");
    }
}

#[test]
fn the_output_is_read_as_a_paste_is() {
    let start = format!(
        "{{\"workspace_id\":\"w\",\"started_pid\":42,\"invitation\":\"brcrew1:AbC-12_x\"}}\n{{\"workspace_id\":\"w\",\"socket\":\"/tmp/s\"}}\n"
    );
    assert_eq!(
        read_start_output(&start, Some(0)),
        StartOutput::Found {
            text: "brcrew1:AbC-12_x".into()
        }
    );
    // An older biorouter-crew prints no invitation line: the status JSON pins the workspace.
    let status = "{\"workspace_id\":\"w\",\"started_pid\":42}\n{\"workspace_id\":\"w\",\"socket\":\"/tmp/s\",\"host_uid\":1000}\n";
    assert_eq!(
        read_start_output(status, Some(0)),
        StartOutput::Found {
            text: "{\"workspace_id\":\"w\",\"socket\":\"/tmp/s\",\"host_uid\":1000}".into()
        }
    );
    let problem = |output: &str, code: Option<i32>| match read_start_output(output, code) {
        StartOutput::Problem { problem, detail } => (problem, detail),
        found => panic!("{found:?}"),
    };
    assert_eq!(
        problem(
            "sh: 1: /home/a/.local/bin/biorouter-crew: not found\n",
            Some(127)
        )
        .0,
        "not_installed"
    );
    assert_eq!(
        problem("{\"state\":\"starting\",\"started_pid\":42}\n", Some(0)).0,
        "starting"
    );
    assert_eq!(
        problem("Error: storage_corrupt:   workspace identity\n", Some(1)),
        (
            "server_error".into(),
            Some("storage_corrupt: workspace identity".into())
        )
    );
    assert_eq!(
        problem(
            "{\"workspace_id\":\"w\",\"invitation\":null,\"invitation_error\":\"name taken\"}\n",
            Some(0)
        ),
        ("server_error".into(), Some("name taken".into()))
    );
    assert_eq!(problem("", Some(0)).0, "unreadable");
}

fn request(preparation_id: &str) -> HostStartRequest {
    HostStartRequest {
        preparation_id: preparation_id.into(),
        workspace_name: "chen-lab".into(),
        ssh_target: "crew_alice@lab-server".into(),
        port: None,
        identity_file: None,
        proxy_jump: None,
    }
}

fn refusal(error: anyhow::Error) -> (u16, &'static str) {
    let refused = error
        .downcast_ref::<HostStartRefused>()
        .unwrap_or_else(|| panic!("not a typed refusal: {error}"));
    (refused.status, refused.code)
}

#[tokio::test]
async fn only_this_computers_own_unused_host_setup_can_start() {
    let root = tempfile::tempdir().unwrap();
    let manager = CrewManager::new(root.path().to_path_buf()).unwrap();
    // No host setup at all.
    assert_eq!(
        refusal(manager.start_host(request("nope")).await.unwrap_err()),
        (409, "crew_host_setup_unknown")
    );
    {
        let mut registry = manager.registry.lock().await;
        registry.pending_device = Some(PreparedDevice {
            preparation_id: "mine".into(),
            public_key: KEY.into(),
            device_id: "d".into(),
        });
        registry
            .completed_preparations
            .insert("used".into(), "connection".into());
    }
    // Someone else's (or a made-up) preparation, and one already saved as a connection.
    assert_eq!(
        refusal(manager.start_host(request("theirs")).await.unwrap_err()),
        (409, "crew_host_setup_unknown")
    );
    assert_eq!(
        refusal(manager.start_host(request("used")).await.unwrap_err()),
        (409, "crew_host_setup_used")
    );
    // A name or login outside the grammar never reaches ssh.
    for bad in [
        HostStartRequest {
            workspace_name: "lab; id".into(),
            ..request("mine")
        },
        HostStartRequest {
            ssh_target: "-oProxyCommand=id".into(),
            ..request("mine")
        },
        HostStartRequest {
            ssh_target: "a b".into(),
            ..request("mine")
        },
        HostStartRequest {
            proxy_jump: Some("jump;id".into()),
            ..request("mine")
        },
        HostStartRequest {
            identity_file: Some("relative/key".into()),
            ..request("mine")
        },
    ] {
        assert_eq!(
            refusal(manager.start_host(bad).await.unwrap_err()),
            (400, "crew_request_invalid")
        );
    }
    // The request carries no key and no command; an unknown field is refused outright.
    let smuggled = serde_json::from_value::<HostStartRequest>(json!({
        "preparation_id": "mine", "workspace_name": "lab", "ssh_target": "a@b",
        "command": "id"
    }));
    assert!(smuggled.is_err());
}

/// A scripted `ssh`: `-G` answers settings the preflight accepts; a run logs its arguments
/// (one per line) and then behaves as `mode` says.
#[cfg(unix)]
fn write_fake_ssh(root: &Path, mode: &str) {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let behaviour = match mode {
        "start" => format!(
            "printf '%s\\n' '{{\"workspace_id\":\"w\",\"started_pid\":42,\"invitation\":\"brcrew1:TOKEN\"}}'\nprintf '%s\\n' 'note on stderr' >&2\nprintf '%s\\n' '{{\"workspace_id\":\"w\",\"socket\":\"/tmp/s\"}}'\nexit 0"
        ),
        "auth" => "printf '%s\\n' 'crew_alice@lab-server: Permission denied (publickey,keyboard-interactive).' >&2\nexit 255".to_owned(),
        "hang" => "sleep 30".to_owned(),
        other => panic!("{other}"),
    };
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "-G" ]; then
  printf '%s\n' 'hostname 127.0.0.1' 'port 22' 'stricthostkeychecking yes' \
    'forwardagent no' 'forwardx11 no' 'permitlocalcommand no' \
    'clearallforwardings yes' 'nohostauthenticationforlocalhost no' \
    'tunnel no' 'forkafterauthentication no' \
    'gssapidelegatecredentials no' 'proxycommand none' \
    'controlmaster no' 'controlpersist no' 'controlpath none'
  exit 0
fi
for arg; do printf '%s\0' "$arg" >> '{log}'; done
{behaviour}
"#,
        log = root.join("args.log").display()
    );
    fs::write(bin.join("ssh"), script).unwrap();
    fs::set_permissions(bin.join("ssh"), fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
async fn started(
    root: &Path,
    mode: &str,
) -> (Arc<CrewManager>, String, env_lock::EnvGuard<'static>) {
    write_fake_ssh(root, mode);
    let profile = root.join("profile");
    fs::create_dir_all(&profile).unwrap();
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let profile = profile.to_string_lossy().into_owned();
    let env = crate::test_sandbox::relocate_path_root_and(
        profile.as_str(),
        [
            ("BIOROUTER_DEV_PROFILE_ROOT", Some(profile.as_str())),
            ("BIOROUTER_DISABLE_KEYRING", Some("true")),
            ("PATH", Some(path.as_str())),
        ],
    );
    let manager = CrewManager::shared(root.join("manager")).unwrap();
    let prepared = manager.prepare_device().await.unwrap();
    (manager, prepared.preparation_id, env)
}

#[cfg(unix)]
async fn settled(job_id: &str) -> super::HostStartStatus {
    for _ in 0..250 {
        let status = host_start_status(job_id).expect("the run is known");
        if status.state != HostStartState::Running {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the run never settled");
}

#[cfg(unix)]
#[tokio::test]
async fn a_start_runs_exactly_the_fixed_command_and_reads_its_result() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let (manager, preparation, _env) = started(root.path(), "start").await;
    let key = manager
        .registry
        .lock()
        .await
        .pending_device
        .clone()
        .unwrap()
        .public_key;
    let first = manager.start_host(request(&preparation)).await.unwrap();
    let expected = host_start_command("chen-lab", &key).unwrap();
    assert_eq!(first.command, expected);
    let done = settled(&first.job_id).await;
    assert_eq!(done.state, HostStartState::Finished);
    assert_eq!(
        done.result,
        Some(StartOutput::Found {
            text: "brcrew1:TOKEN".into()
        })
    );
    assert!(done.output.contains("brcrew1:TOKEN") && done.output.contains("note on stderr"));
    // ssh got the hardened, non-interactive options, the login, and the command as ONE
    // final argument: exactly the text shown.
    let args: Vec<String> = fs::read(root.path().join("args.log"))
        .unwrap()
        .split(|b| *b == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8(arg.to_vec()).unwrap())
        .collect();
    assert_eq!(args.last(), Some(&expected));
    assert_eq!(args[args.len() - 2], "crew_alice@lab-server");
    for option in [
        "BatchMode=yes",
        "StrictHostKeyChecking=yes",
        "ForwardAgent=no",
        "ControlPath=none",
        "PermitLocalCommand=no",
    ] {
        assert!(args.iter().any(|arg| arg == option), "{option}: {args:?}");
    }
    assert!(!args.iter().any(|arg| arg == "-S"), "no multiplexed master");
}

#[cfg(unix)]
#[tokio::test]
async fn a_server_that_wants_a_password_is_refused_with_why_and_never_prompts() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let (manager, preparation, _env) = started(root.path(), "auth").await;
    let first = manager.start_host(request(&preparation)).await.unwrap();
    let done = settled(&first.job_id).await;
    assert_eq!(done.state, HostStartState::Failed);
    let error = done.error.unwrap();
    assert_eq!(error.code, "crew_ssh_auth_required");
    assert!(
        error.message.contains("Run the commands yourself"),
        "{}",
        error.message
    );
    assert_eq!(done.result, None);
}

#[cfg(unix)]
#[tokio::test]
async fn a_second_click_answers_the_run_under_way_and_cancel_stops_it() {
    if !crate::test_sandbox::in_a_process_of_its_own() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let (manager, preparation, _env) = started(root.path(), "hang").await;
    let first = manager.start_host(request(&preparation)).await.unwrap();
    let again = manager.start_host(request(&preparation)).await.unwrap();
    assert_eq!(again.job_id, first.job_id, "one run per host setup");
    assert!(cancel_host_start(&first.job_id));
    let done = settled(&first.job_id).await;
    assert_eq!(done.state, HostStartState::Failed);
    assert_eq!(done.error.unwrap().code, "crew_host_start_cancelled");
    assert!(!cancel_host_start("unknown-run"));
    assert!(host_start_status("unknown-run").is_none());
}
