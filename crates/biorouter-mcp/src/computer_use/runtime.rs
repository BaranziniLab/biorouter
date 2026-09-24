use super::{contract, manifest};
use anyhow::{bail, Context, Result};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::process::Stdio;
#[cfg(windows)]
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};

const MAX_FRAME_BYTES: u64 = 32 * 1024 * 1024;

pub struct Runtime {
    child: Child,
    #[cfg(windows)]
    process_job: super::windows_job::ProcessJob,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    next_id: u64,
    _temporary_files: tempfile::TempDir,
}

impl Drop for Runtime {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            crate::developer::shell::kill_process_group_now(pid);
        }
        #[cfg(windows)]
        self.process_job.terminate();
    }
}

/// Pass through the desktop/session prerequisites the helper needs, and nothing
/// else: model credentials and daemon secrets never reach it.
fn inherit_session_env(command: &mut Command) -> Result<()> {
    const SESSION: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_ALL",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SystemRoot",
        "WINDIR",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "XDG_RUNTIME_DIR",
        "XDG_SESSION_TYPE",
        "DBUS_SESSION_BUS_ADDRESS",
    ];
    // Windows process startup is not self-contained: the loader, the CRT and
    // `powershell.exe` -- which the helper shells out to for every UIA call --
    // read these. The operating system writes them at logon, so none of them
    // can carry a model credential or a daemon secret, and inheriting the OS's
    // own PATHEXT or ComSpec is strictly safer than clearing it: a cleared
    // variable falls back to a default the agent equally cannot influence.
    //
    // On the same Windows runner, the pinned helper took 26.8 s to answer
    // `doctor --json` without PSModulePath. System32 modules alone were also
    // slow; System32 plus Program Files modules answered in 2.6 s. The inherited
    // path was fast but may contain user-writable directories.
    #[cfg(windows)]
    const PLATFORM: &[&str] = &[
        "ComSpec",
        "PATHEXT",
        "SystemDrive",
        "ProgramData",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramW6432",
        "CommonProgramFiles",
        "CommonProgramFiles(x86)",
        "CommonProgramW6432",
        "ALLUSERSPROFILE",
        "PUBLIC",
        "HOMEDRIVE",
        "HOMEPATH",
        "USERNAME",
        "USERDOMAIN",
        "PROCESSOR_ARCHITECTURE",
        "PROCESSOR_ARCHITEW6432",
        "NUMBER_OF_PROCESSORS",
        "OS",
    ];
    #[cfg(not(windows))]
    const PLATFORM: &[&str] = &[];
    for name in SESSION.iter().chain(PLATFORM.iter()) {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    #[cfg(windows)]
    command.env("PSModulePath", trusted_powershell_module_path()?);
    Ok(())
}

#[cfg(windows)]
fn trusted_powershell_module_path() -> Result<OsString> {
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    use windows_sys::Win32::UI::Shell::{SHGetFolderPathW, CSIDL_PROGRAM_FILES};

    let mut system = [0u16; 260];
    let length = unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
    if length == 0 || length >= system.len() {
        bail!("computer_use_missing_dependency: Windows system directory unavailable");
    }
    let system_modules = PathBuf::from(OsString::from_wide(&system[..length]))
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("Modules");

    let mut program_files = [0u16; 260];
    let status = unsafe {
        SHGetFolderPathW(
            std::ptr::null_mut(),
            CSIDL_PROGRAM_FILES as i32,
            std::ptr::null_mut(),
            0,
            program_files.as_mut_ptr(),
        )
    };
    if status != 0 {
        bail!("computer_use_missing_dependency: Windows Program Files directory unavailable");
    }
    let length = program_files
        .iter()
        .position(|&unit| unit == 0)
        .filter(|&length| length > 0)
        .context("computer_use_missing_dependency: Windows Program Files directory invalid")?;
    let all_users_modules = PathBuf::from(OsString::from_wide(&program_files[..length]))
        .join("WindowsPowerShell")
        .join("Modules");
    std::env::join_paths([system_modules, all_users_modules])
        .context("computer_use_missing_dependency: Windows PowerShell module path invalid")
}

#[cfg(all(test, windows))]
mod windows_environment_tests {
    use super::*;

    #[test]
    fn helper_receives_only_machine_powershell_module_directories() {
        let trusted = trusted_powershell_module_path().unwrap();
        let directories: Vec<_> = std::env::split_paths(&trusted).collect();
        assert_eq!(directories.len(), 2);
        let system_modules = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("Modules");
        let all_users_modules = PathBuf::from(std::env::var_os("ProgramFiles").unwrap())
            .join("WindowsPowerShell")
            .join("Modules");
        assert!(directories[0]
            .to_string_lossy()
            .eq_ignore_ascii_case(&system_modules.to_string_lossy()));
        assert!(directories[1]
            .to_string_lossy()
            .eq_ignore_ascii_case(&all_users_modules.to_string_lossy()));
        let profile = PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
        assert!(directories.iter().all(|path| !path.starts_with(&profile)));

        let mut command = Command::new("powershell.exe");
        command.env_clear();
        inherit_session_env(&mut command).unwrap();
        let env = command.as_std().get_envs().collect::<Vec<_>>();
        let module_path = env
            .iter()
            .find(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("PSModulePath"))
            .and_then(|(_, value)| *value)
            .unwrap();
        assert_eq!(module_path, trusted.as_os_str());
        let path = env
            .iter()
            .find(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("PATH"))
            .and_then(|(_, value)| *value)
            .unwrap();
        let expected_path = std::env::var_os("PATH").unwrap();
        assert_eq!(path, expected_path.as_os_str());
    }
}

impl Runtime {
    pub async fn start() -> Result<Self> {
        Self::start_payload(tokio::task::spawn_blocking(manifest::locate).await??).await
    }

    pub(super) async fn start_payload(payload: manifest::RuntimePayload) -> Result<Self> {
        let mut runtime = Self::spawn_payload(payload, &["mcp"])?;
        runtime.handshake().await?;
        Ok(runtime)
    }

    fn spawn_payload(payload: manifest::RuntimePayload, arguments: &[&str]) -> Result<Self> {
        let temporary_files = tempfile::Builder::new()
            .prefix("biorouter-computer-use-")
            .tempdir()?;
        let mut command = Command::new(&payload.executable);
        command
            .args(arguments)
            .current_dir(&payload.root)
            .env_clear();
        inherit_session_env(&mut command)?;
        for name in ["TMPDIR", "TMP", "TEMP"] {
            command.env(name, temporary_files.path());
        }
        let installation = format!(
            "{:x}",
            Sha256::digest(
                format!("{}:{}", payload.executable.display(), payload.payload_id).as_bytes()
            )
        );
        command.env(
            "OPEN_COMPUTER_USE_AGENT_SOCKET_NAMESPACE",
            format!("biorouter-{}-{installation}", manifest::UPSTREAM_VERSION),
        );
        command.env("OPEN_COMPUTER_USE_ALLOW_GLOBAL_POINTER_FALLBACKS", "1");
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(
            windows_sys::Win32::System::Threading::CREATE_NO_WINDOW
                | windows_sys::Win32::System::Threading::CREATE_SUSPENDED,
        );
        let mut child = command
            .spawn()
            .context("computer_use_missing_runtime: helper could not start")?;
        #[cfg(windows)]
        let process_job = super::windows_job::ProcessJob::attach(&child)?;
        let input = child.stdin.take().context("helper stdin unavailable")?;
        let output = BufReader::new(child.stdout.take().context("helper stdout unavailable")?);
        Ok(Self {
            child,
            #[cfg(windows)]
            process_job,
            input,
            output,
            next_id: 1,
            _temporary_files: temporary_files,
        })
    }

    pub async fn doctor() -> Result<Value> {
        let payload = tokio::task::spawn_blocking(manifest::locate).await??;
        let mut runtime = Self::spawn_payload(payload, &["doctor", "--json"])?;
        let operation = async {
            // ONE newline-terminated line, not read-to-EOF. `read_to_end` returns
            // only when every writer closes the pipe -- and on Windows the helper's
            // own children inherit that handle, so a descendant that outlives the
            // probe stalls it for the full 30 s timeout even though the readiness
            // JSON was already written. All three helpers emit exactly one line
            // (Windows main.go, Linux main.go, macOS OpenComputerUseMain.swift).
            let mut bytes = Vec::new();
            let read = (&mut runtime.output)
                .take(65537)
                .read_until(b'\n', &mut bytes)
                .await?;
            if read > 65536 {
                bail!("computer_use_protocol_error: oversized readiness response");
            }
            let status = runtime.child.wait().await?;
            if !status.success() {
                bail!("computer_use_probe_failed: native readiness probe failed");
            }
            let result: Value = serde_json::from_slice(&bytes)
                .context("computer_use_protocol_error: invalid readiness response")?;
            if !matches!(
                result.get("state").and_then(Value::as_str),
                Some(
                    "ready"
                        | "os_permission_required"
                        | "desktop_unavailable"
                        | "unsupported_environment"
                        | "missing_dependency"
                )
            ) || [
                "desktop_available",
                "accessibility",
                "screen_recording",
                "capture_available",
            ]
            .iter()
            .any(|key| !result.get(*key).is_some_and(Value::is_boolean))
                || !result.get("message").is_some_and(Value::is_string)
            {
                bail!("computer_use_protocol_error: malformed readiness state");
            }
            Ok(result)
        };
        // ⚠ This bound SUPERVISES a bound of the helper's own, and must exceed it
        // or the inner one is dead code.
        //
        // The Windows helper runs its diagnostics through PowerShell under
        // `context.WithTimeout(30*time.Second)`, and on expiry it answers
        // honestly: `missing_dependency`, "Windows runtime timed out after 30s".
        // That answer is worth having — it names PowerShell, which is what a
        // user has to act on. But this timeout was ALSO 30 s, and it starts
        // strictly earlier (spawn, process start and the helper's own setup all
        // happen before its clock begins), so it always won. The helper's
        // diagnosis could never be delivered, and the user was told only
        // "could not check the native runtime".
        //
        // Measured on a Windows runner before this change: 34.91 s to fail,
        // which is this 30 s plus the 5 s shutdown below — the outer bound
        // firing, never the inner one.
        //
        // 45 s leaves room for the helper to answer at 30 s plus process start.
        // The extra latency is only ever paid on a machine where the helper
        // cannot talk to PowerShell at all, and there a real diagnosis is worth
        // more than a fast non-answer.
        const PROBE_BOUND: std::time::Duration = std::time::Duration::from_secs(45);
        let result = tokio::time::timeout(PROBE_BOUND, operation).await;
        runtime.shutdown().await;
        result.context("computer_use_probe_timeout: native readiness probe did not complete")?
    }

    pub async fn shutdown(&mut self) {
        if let Some(_pid) = self.child.id() {
            #[cfg(unix)]
            crate::developer::shell::kill_process_group_now(_pid);
            #[cfg(windows)]
            {
                self.process_job.terminate();
                self.process_job.wait_empty().await;
            }
            let _ = self.child.start_kill();
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait()).await;
        }
    }

    async fn write(&mut self, request: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(request)?;
        if bytes.len() as u64 > MAX_FRAME_BYTES {
            bail!("computer_use_payload_limit: request exceeds 32 MiB");
        }
        bytes.push(b'\n');
        self.input.write_all(&bytes).await?;
        self.input.flush().await?;
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        for _ in 0..128 {
            let mut bytes = Vec::new();
            let size = (&mut self.output)
                .take(MAX_FRAME_BYTES + 1)
                .read_until(b'\n', &mut bytes)
                .await?;
            if size == 0 {
                bail!("computer_use_runtime_disconnected: refresh app state after reconnecting");
            }
            if size as u64 > MAX_FRAME_BYTES || bytes.last() != Some(&b'\n') {
                bail!("computer_use_payload_limit: invalid or oversized runtime frame");
            }
            let response: Value = serde_json::from_slice(&bytes)
                .context("computer_use_protocol_error: malformed runtime frame")?;
            if response.get("method").is_some() && response.get("id").is_none() {
                continue;
            }
            if response.get("id") != Some(&json!(id)) {
                bail!("computer_use_protocol_error: unexpected response identity");
            }
            if let Some(error) = response.get("error") {
                bail!(
                    "computer_use_runtime_error: {}",
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("native request failed")
                );
            }
            return response
                .get("result")
                .cloned()
                .context("computer_use_protocol_error: missing result");
        }
        bail!("computer_use_protocol_error: excessive runtime notifications")
    }

    async fn handshake(&mut self) -> Result<()> {
        let reply = self.request("initialize", json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"biorouter-computer-use","version":env!("CARGO_PKG_VERSION")}})).await?;
        if reply.pointer("/serverInfo/version").and_then(Value::as_str)
            != Some(manifest::UPSTREAM_VERSION)
            || reply.get("protocolVersion").and_then(Value::as_str) != Some("2025-03-26")
        {
            bail!("computer_use_incompatible_runtime: unexpected protocol or runtime version");
        }
        self.write(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
            .await?;
        let listing = self.request("tools/list", json!({})).await?;
        validate_tools(&listing)?;
        Ok(())
    }

    pub async fn call(&mut self, name: &str, arguments: Value) -> Result<CallToolResult> {
        let result = self
            .request("tools/call", json!({"name":name,"arguments":arguments}))
            .await?;
        serde_json::from_value(result).context("computer_use_protocol_error: malformed tool result")
    }
}

pub fn validate_tools(listing: &Value) -> Result<()> {
    let tools = listing
        .get("tools")
        .and_then(Value::as_array)
        .context("computer_use_incompatible_runtime: missing tools")?;
    for expected in contract::tools() {
        let found = tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some(expected.name.as_ref()))
            .context("computer_use_incompatible_runtime: missing required tool")?;
        let actual = found
            .get("inputSchema")
            .context("computer_use_incompatible_runtime: missing schema")?;
        let expected = serde_json::to_value(&expected.input_schema)?;
        if contract::semantic_schema(actual) != contract::semantic_schema(&expected) {
            bail!(
                "computer_use_incompatible_runtime: schema mismatch for {}",
                found["name"]
            );
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
pub(super) mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    pub async fn fixture(mode: &str) -> (tempfile::TempDir, Runtime) {
        let root = tempfile::tempdir().unwrap();
        let tools = json!({"tools":contract::tools()});
        std::fs::write(
            root.path().join("tools.json"),
            serde_json::to_vec(&tools).unwrap(),
        )
        .unwrap();
        std::fs::write(root.path().join("mode"), mode).unwrap();
        let script = r##"#!/usr/bin/python3
import json, os, subprocess, sys, time
with open('tools.json') as f: tools = json.load(f)
with open('mode') as f: mode = f.read()
count = 0
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request: continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion':'2025-03-26','serverInfo':{'version':'0.3.5'}}
    elif method == 'tools/list': result = tools
    else:
        count += 1
        if mode == 'block':
            child = subprocess.Popen(['/bin/sleep','60'])
            with open('descendant.pid','w') as f: f.write(str(child.pid))
            time.sleep(60)
        result = {'content':[{'type':'text','text':str(count)},{'type':'image','mimeType':'image/png','data':'aW1hZ2U='}], 'isError':mode == 'error'}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"##;
        let executable = root.path().join("ocu");
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let runtime = Runtime::start_payload(manifest::RuntimePayload {
            executable,
            root: root.path().to_owned(),
            development_override: true,
            payload_id: "fixture".into(),
        })
        .await
        .unwrap();
        (root, runtime)
    }

    #[tokio::test]
    async fn native_connection_preserves_images_error_flag_and_per_connection_state() {
        let (_a, mut a) = fixture("success").await;
        let (_b, mut b) = fixture("error").await;
        let first = a
            .call("get_app_state", json!({"app":"fixture"}))
            .await
            .unwrap();
        assert_eq!(first.content[0].as_text().unwrap().text, "1");
        assert_eq!(first.content[1].as_image().unwrap().data, "aW1hZ2U=");
        let second = a
            .call("click", json!({"app":"fixture","element_index":"0"}))
            .await
            .unwrap();
        assert_eq!(second.content[0].as_text().unwrap().text, "2");
        let other = b
            .call("get_app_state", json!({"app":"fixture"}))
            .await
            .unwrap();
        assert_eq!(other.content[0].as_text().unwrap().text, "1");
        assert_eq!(other.is_error, Some(true));
    }

    #[tokio::test]
    #[ignore = "requires an explicitly selected packaged native payload; performs no desktop observation"]
    async fn packaged_native_runtime_matches_the_reviewed_contract() {
        assert!(
            std::env::var_os("BIOROUTER_COMPUTER_USE_DIR").is_some(),
            "select the payload explicitly"
        );
        let mut runtime =
            tokio::time::timeout(std::time::Duration::from_secs(30), Runtime::start())
                .await
                .expect("native initialize and tools/list timed out")
                .expect("packaged runtime handshake failed");
        runtime.shutdown().await;
    }

    #[test]
    fn handshake_rejects_missing_or_changed_native_schemas() {
        let mut listing = json!({"tools":contract::tools()});
        assert!(validate_tools(&listing).is_ok());
        listing["tools"][0]["inputSchema"]["additionalProperties"] = json!(true);
        assert!(validate_tools(&listing).is_err());
        listing["tools"] = json!([]);
        assert!(validate_tools(&listing).is_err());
    }
}

#[cfg(test)]
mod schema_contract_tests {
    use super::*;
    #[test]
    fn native_ci_schema_snapshot_matches_the_runtime_handshake_contract() {
        let listing: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/computer-use-tools.json"))
                .unwrap();
        validate_tools(&listing).unwrap();
        assert_eq!(
            listing["tools"].as_array().unwrap().len(),
            contract::TOOL_NAMES.len()
        );
        for (name, argument, bound) in [
            ("click", "click_count", "minimum"),
            ("click", "click_count", "maximum"),
            ("scroll", "pages", "exclusiveMinimum"),
            ("scroll", "pages", "maximum"),
        ] {
            let mut drifted = listing.clone();
            let tool = drifted["tools"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .find(|tool| tool["name"] == name)
                .unwrap();
            tool["inputSchema"]["properties"][argument]
                .as_object_mut()
                .unwrap()
                .remove(bound);
            assert!(validate_tools(&drifted).is_err());
        }
    }
}
