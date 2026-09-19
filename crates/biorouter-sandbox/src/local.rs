//! In-process sandbox backend.

use std::path::Path;

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;

use crate::{resolve_in_workspace, ExecOutput, Result, SandboxClient, SandboxError, SandboxSpec};

/// Runs commands directly on the host as the current user.
///
/// **Isolation is limited**: this backend enforces a workspace path-jail for
/// `read`/`write`/`list` and a wall-clock timeout (killing the child on
/// expiry via `kill_on_drop`), but it does **not** sandbox the process — no
/// namespaces, no cgroups, no capability dropping, and the host environment is
/// inherited except for daemon-private credentials (the spec's `env` is *added*,
/// not a clean slate). It exists as the today-behavior baseline and the migration
/// target for [`crate::DockerSandbox`].
pub struct LocalProcessSandbox {
    spec: SandboxSpec,
}

impl LocalProcessSandbox {
    pub fn new(spec: SandboxSpec) -> Self {
        tracing::warn!(
            workspace = %spec.workspace.display(),
            "LocalProcessSandbox active: tool commands run UNSANDBOXED on the host as the current user"
        );
        Self { spec }
    }

    pub fn workspace(&self) -> &Path {
        &self.spec.workspace
    }

    pub fn spec(&self) -> &SandboxSpec {
        &self.spec
    }
}

#[async_trait]
impl SandboxClient for LocalProcessSandbox {
    fn label(&self) -> &'static str {
        "local(unsafe)"
    }

    async fn available(&self) -> bool {
        true
    }

    async fn exec(&self, argv: &[String], stdin: Option<&str>) -> Result<ExecOutput> {
        if argv.is_empty() {
            return Err(SandboxError::Exec("empty argv".into()));
        }
        let mut cmd = tokio::process::Command::new(&argv[0]);
        crate::console::no_console_window(&mut cmd);
        cmd.args(&argv[1..])
            .current_dir(&self.spec.workspace)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        for (k, v) in &self.spec.env {
            cmd.env(k, v);
        }
        crate::environment::strip_daemon_private_env(&mut cmd);

        let mut child = cmd.spawn()?;
        if let Some(input) = stdin {
            if let Some(mut si) = child.stdin.take() {
                si.write_all(input.as_bytes()).await?;
                si.shutdown().await.ok();
            }
        } else {
            // Close stdin so programs that read it (e.g. `cat`) don't block forever.
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
            // On timeout the `wait_with_output` future is dropped, which drops the
            // `Child`; `kill_on_drop(true)` then reaps the runaway process.
            Err(_elapsed) => Ok(ExecOutput {
                stdout: String::new(),
                stderr: format!("command timed out after {:?}", self.spec.timeout),
                code: -1,
                timed_out: true,
            }),
        }
    }

    async fn read(&self, guest_path: &str) -> Result<Vec<u8>> {
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
        // The workspace dir is the persistence boundary; an identity snapshot
        // returns its path. A copy-on-write tar snapshot is a Phase-4 follow-up.
        Ok(self.spec.workspace.display().to_string())
    }
}
