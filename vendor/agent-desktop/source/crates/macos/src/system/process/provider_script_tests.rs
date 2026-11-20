use super::run_provider_script;
use agent_desktop_core::{ErrorCode, ProviderScriptRequest};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static TEMP_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let counter = TEMP_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "agent-desktop-provider-script-{}-{timestamp}-{counter}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn write_script(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct EnvironmentGuard {
    name: &'static str,
    previous: Option<OsString>,
}

impl EnvironmentGuard {
    fn set(name: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(name);
        unsafe { std::env::set_var(name, value) };
        Self { name, previous }
    }
}

impl Drop for EnvironmentGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            unsafe { std::env::set_var(self.name, value) };
        } else {
            unsafe { std::env::remove_var(self.name) };
        }
    }
}

struct ProcessCleanup(i32);

impl Drop for ProcessCleanup {
    fn drop(&mut self) {
        unsafe { kill(self.0, 9) };
    }
}

unsafe extern "C" {
    fn kill(process: i32, signal: i32) -> i32;
}

fn process_exists(process: i32) -> bool {
    unsafe { kill(process, 0) == 0 }
}

fn request(script: PathBuf, working_dir: PathBuf, timeout: Duration) -> ProviderScriptRequest {
    ProviderScriptRequest {
        script,
        args: Vec::new(),
        env: Vec::new(),
        working_dir,
        timeout,
    }
}

#[test]
fn script_timeout_kills_the_running_child() {
    let directory = TempDirectory::new();
    let pid_file = directory.0.join("pid");
    let script = directory.write_script(
        "timeout.sh",
        "printf '%s\\n' \"$$\" > \"$1\"\nexec /bin/sleep 30\n",
    );
    let mut request = request(script, directory.0.clone(), Duration::from_secs(1));
    request.args.push(pid_file.to_string_lossy().into_owned());

    let error = run_provider_script(&request).expect_err("the script must time out");

    assert_eq!(error.code, ErrorCode::Timeout);
    let process = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    assert!(!process_exists(process));
}

#[test]
fn script_detached_process_group_survives() {
    let directory = TempDirectory::new();
    let pid_file = directory.0.join("pid");
    let script = directory.write_script(
        "detached.sh",
        "set -m\n/bin/sleep 30 >/dev/null 2>&1 &\nprintf '%s\\n' \"$!\" > \"$1\"\n",
    );
    let mut request = request(script, directory.0.clone(), Duration::from_secs(2));
    request.args.push(pid_file.to_string_lossy().into_owned());

    let started = Instant::now();
    let output = run_provider_script(&request).unwrap();
    let process = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse::<i32>()
        .unwrap();
    let _cleanup = ProcessCleanup(process);

    assert_eq!(output.status, Some(0));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(process_exists(process));
}

#[test]
fn script_environment_is_limited_to_allowed_and_requested_values() {
    const SENTINEL: &str = "AGENT_DESKTOP_PROVIDER_SCRIPT_SENTINEL";
    const REQUEST_VALUE: &str = "AGENT_DESKTOP_PROVIDER_SCRIPT_REQUEST";
    let _environment = EnvironmentGuard::set(SENTINEL, "caller-only");
    let caller_environment = std::env::vars_os()
        .map(|(name, value)| format!("{}={}", name.to_string_lossy(), value.to_string_lossy()))
        .collect::<HashSet<_>>();
    let directory = TempDirectory::new();
    let script = directory.write_script("environment.sh", "env\n");
    let mut request = request(script, directory.0.clone(), Duration::from_secs(2));
    request
        .env
        .push((REQUEST_VALUE.to_owned(), "request-only".to_owned()));

    let output = run_provider_script(&request).unwrap();
    let child_environment = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    let allowed_names = HashSet::from(["PATH", "HOME", "TMPDIR", REQUEST_VALUE]);
    let shell_names = HashSet::from(["PWD", "SHLVL", "_"]);
    let leaked_caller_value = caller_environment.iter().find(|entry| {
        let name = entry.split_once('=').map_or("", |(name, _)| name);
        !allowed_names.contains(name)
            && !shell_names.contains(name)
            && child_environment.contains(*entry)
    });

    assert_eq!(output.status, Some(0));
    assert!(child_environment.contains(&format!("{REQUEST_VALUE}=request-only")));
    assert!(
        leaked_caller_value.is_none(),
        "provider script inherited caller-only environment value {leaked_caller_value:?}"
    );
    for name in ["PATH", "HOME", "TMPDIR"] {
        if let Some(value) = std::env::var_os(name) {
            assert!(child_environment.contains(&format!("{name}={}", value.to_string_lossy())));
        }
    }
}
