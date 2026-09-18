//! Container-isolated sandbox backend (via the `docker` CLI).

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;

use crate::{
    resolve_in_workspace, ExecOutput, NetworkPolicy, Result, SandboxClient, SandboxError,
    SandboxSpec,
};

/// Runs each command in a fresh `--rm` container with the workspace
/// bind-mounted at `/work`. Hardening defaults: `--network none`,
/// `--cap-drop ALL`, `--security-opt no-new-privileges`, a non-root user
/// (`65534:65534` = nobody:nogroup), and pid/memory/cpu caps.
///
/// Because the workspace is a host bind mount, `read`/`write`/`list` operate on
/// the host side of the mount (identical to the local backend), so file state
/// persists across the ephemeral containers.
pub struct DockerSandbox {
    spec: SandboxSpec,
}

impl DockerSandbox {
    /// Small, widely-available default image with a POSIX shell + Python.
    pub const DEFAULT_IMAGE: &'static str = "python:3.12-slim";

    pub fn new(spec: SandboxSpec) -> Self {
        Self { spec }
    }

    pub fn spec(&self) -> &SandboxSpec {
        &self.spec
    }

    /// Build the argv passed to `docker` (everything after the program name).
    ///
    /// Exposed so the isolation flags can be unit-tested without invoking docker.
    pub fn build_run_args(&self, argv: &[String]) -> Vec<String> {
        let mut a: Vec<String> = vec!["run".into(), "--rm".into(), "-i".into()];

        match self.spec.network {
            NetworkPolicy::None => {
                a.push("--network".into());
                a.push("none".into());
            }
            NetworkPolicy::Host => {
                a.push("--network".into());
                a.push("host".into());
            }
        }

        // Privilege + resource hardening.
        a.push("--cap-drop".into());
        a.push("ALL".into());
        a.push("--security-opt".into());
        a.push("no-new-privileges".into());
        a.push("--user".into());
        a.push("65534:65534".into());
        a.push("--pids-limit".into());
        a.push("256".into());
        if let Some(mem) = &self.spec.max_mem {
            a.push("--memory".into());
            a.push(mem.clone());
        }
        if let Some(cpus) = self.spec.cpus {
            a.push("--cpus".into());
            a.push(format!("{cpus}"));
        }

        // Workspace mount + working dir.
        a.push("-v".into());
        a.push(format!("{}:/work:rw", self.spec.workspace.display()));
        a.push("-w".into());
        a.push("/work".into());

        // Extra mounts.
        for m in &self.spec.mounts {
            let mode = if m.writable { "rw" } else { "ro" };
            a.push("-v".into());
            a.push(format!(
                "{}:{}:{}",
                m.host.display(),
                m.guest.display(),
                mode
            ));
        }

        // Env (vault-resolved secrets land here).
        for (k, v) in &self.spec.env {
            if crate::environment::is_daemon_private_env_key(k) {
                continue;
            }
            a.push("-e".into());
            a.push(format!("{k}={v}"));
        }

        // Image, then the command (always last).
        let image = self
            .spec
            .image
            .clone()
            .unwrap_or_else(|| Self::DEFAULT_IMAGE.to_string());
        a.push(image);
        a.extend(argv.iter().cloned());
        a
    }
}

#[async_trait]
impl SandboxClient for DockerSandbox {
    fn label(&self) -> &'static str {
        "docker"
    }

    async fn available(&self) -> bool {
        which_docker().await
    }

    async fn exec(&self, argv: &[String], stdin: Option<&str>) -> Result<ExecOutput> {
        if argv.is_empty() {
            return Err(SandboxError::Exec("empty argv".into()));
        }
        if !self.available().await {
            return Err(SandboxError::Unavailable(
                "`docker` CLI not found on PATH".into(),
            ));
        }

        let run_args = self.build_run_args(argv);
        let mut cmd = tokio::process::Command::new("docker");
        crate::console::no_console_window(&mut cmd);
        cmd.args(&run_args)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        crate::environment::strip_daemon_private_env(&mut cmd);

        let mut child = cmd.spawn()?;
        if let Some(input) = stdin {
            if let Some(mut si) = child.stdin.take() {
                si.write_all(input.as_bytes()).await?;
                si.shutdown().await.ok();
            }
        } else {
            drop(child.stdin.take());
        }

        match tokio::time::timeout(self.spec.timeout, child.wait_with_output()).await {
            Ok(result) => {
                let out = result?;
                Ok(ExecOutput {
                    stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                    code: out.status.code().unwrap_or(-1),
                    timed_out: false,
                })
            }
            Err(_elapsed) => Ok(ExecOutput {
                stdout: String::new(),
                stderr: format!("container timed out after {:?}", self.spec.timeout),
                code: -1,
                timed_out: true,
            }),
        }
    }

    async fn read(&self, guest_path: &str) -> Result<Vec<u8>> {
        // Workspace is a host bind mount → read the host side directly.
        let p = resolve_in_workspace(&self.spec.workspace, guest_path, false)?;
        Ok(tokio::fs::read(p).await?)
    }

    async fn write(&self, guest_path: &str, bytes: &[u8]) -> Result<()> {
        let p = resolve_in_workspace(&self.spec.workspace, guest_path, true)?;
        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(p, bytes).await?;
        Ok(())
    }

    async fn list(&self, guest_dir: &str) -> Result<Vec<String>> {
        let p = resolve_in_workspace(&self.spec.workspace, guest_dir, false)?;
        let mut rd = tokio::fs::read_dir(p).await?;
        let mut names = Vec::new();
        while let Some(entry) = rd.next_entry().await? {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        Ok(names)
    }

    async fn snapshot(&self) -> Result<String> {
        // The bind-mounted workspace is the persistence boundary.
        Ok(self.spec.workspace.display().to_string())
    }
}

/// Returns true if the `docker` CLI is invokable.
async fn which_docker() -> bool {
    let mut command = tokio::process::Command::new("docker");
    command
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::environment::strip_daemon_private_env(&mut command);
    crate::console::no_console_window(&mut command);
    command.status().await.map(|s| s.success()).unwrap_or(false)
}
