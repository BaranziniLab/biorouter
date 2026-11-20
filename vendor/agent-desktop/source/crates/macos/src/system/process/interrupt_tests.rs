use crate::system::process::run_with_timeout;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PID_FILE_ENV: &str = "AD_INTERRUPT_TEST_PID_FILE";
const SCRIPT_ENV: &str = "AD_INTERRUPT_TEST_SCRIPT";
const OBEYS_TERM: &str = "echo $$ > \"$0\"; sleep 30; :";
const IGNORES_TERM: &str = "trap '' TERM; echo $$ > \"$0\"; sleep 30; :";

fn helper_name() -> String {
    let module = module_path!();
    let local = module.split_once("::").map_or(module, |(_, rest)| rest);
    format!("{local}::helper_runs_a_slow_subprocess")
}

fn group_is_alive(process_group: i32) -> bool {
    unsafe { libc::kill(-process_group, 0) == 0 }
}

#[test]
#[ignore = "re-executed as a child by a_terminated_host_takes_its_subprocess_group_with_it"]
fn helper_runs_a_slow_subprocess() {
    let Some(pid_file) = std::env::var_os(PID_FILE_ENV) else {
        return;
    };
    let script = std::env::var(SCRIPT_ENV).unwrap_or_default();
    let mut command = Command::new("/bin/sh");
    command.args(["-c", &script]).arg(pid_file);
    super::forward_to_active_group();
    let _ = run_with_timeout(&mut command, "helper", Duration::from_secs(30));
}

#[test]
fn a_terminated_host_takes_its_subprocess_group_with_it() {
    assert_group_dies_with_host("obeys", OBEYS_TERM);
}

#[test]
fn a_terminated_host_kills_a_subprocess_group_that_ignores_sigterm() {
    assert_group_dies_with_host("ignores", IGNORES_TERM);
}

fn assert_group_dies_with_host(label: &str, script: &str) {
    let pid_file =
        std::env::temp_dir().join(format!("ad-interrupt-{label}-{}", std::process::id()));
    let _ = std::fs::remove_file(&pid_file);
    let mut host = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", &helper_name()])
        .env(PID_FILE_ENV, &pid_file)
        .env(SCRIPT_ENV, script)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let process_group = loop {
        let recorded = std::fs::read_to_string(&pid_file).unwrap_or_default();
        if let Ok(pid) = recorded.trim().parse::<i32>() {
            break pid;
        }
        assert!(Instant::now() < deadline, "the helper never started");
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = std::fs::remove_file(&pid_file);
    assert!(group_is_alive(process_group), "the subprocess is running");

    unsafe { libc::kill(host.id() as i32, libc::SIGTERM) };
    host.wait().unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while group_is_alive(process_group) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let survived = group_is_alive(process_group);
    if survived {
        unsafe { libc::kill(-process_group, libc::SIGKILL) };
    }
    assert!(
        !survived,
        "the subprocess group outlived its terminated host"
    );
}
