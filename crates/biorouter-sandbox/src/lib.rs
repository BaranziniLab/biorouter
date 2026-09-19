//! `biorouter-sandbox` — capability-scoped sandboxed execution for Biorouter.
//!
//! This is a **leaf crate**: it depends only on small runtime utilities, never
//! on `biorouter` or `biorouter-mcp`. Both of those depend on *this* crate, so
//! the `SandboxClient` trait can be shared without a dependency cycle (the
//! engine crate `biorouter` already depends on `biorouter-mcp`, so the trait
//! cannot live in either of them).
//!
//! Two backends ship here:
//! - [`LocalProcessSandbox`] — runs commands directly on the host as the
//!   current user. It provides a **workspace path-jail** for file ops and a
//!   wall-clock timeout, but it does **not** isolate the process (no
//!   namespaces, no resource caps). Use only for trusted single-user desktop.
//! - [`DockerSandbox`] — container isolation via the `docker` CLI:
//!   `--network none` by default, `--cap-drop ALL`, `--security-opt
//!   no-new-privileges`, a non-root user, pid/memory/cpu caps, and a
//!   bind-mounted workspace.
//!
//! The trait is intentionally small and `async` so callers (the Developer and
//! Computer-Controller MCP servers) can route their shell/script execution
//! through whichever backend the app's manifest grants.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub mod console;
pub mod docker;
pub mod environment;
pub mod local;
pub mod seatbelt;
pub mod shell_sandbox;

pub use docker::DockerSandbox;
pub use local::LocalProcessSandbox;

/// A single host→guest bind mount.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mount {
    pub host: PathBuf,
    pub guest: PathBuf,
    pub writable: bool,
}

/// Network policy for a sandbox. Defaults to fully isolated (`None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicy {
    /// No network access at all (`docker --network none`).
    #[default]
    None,
    /// Host networking (`docker --network host`). Use sparingly.
    Host,
}

/// Declarative description of a sandbox workspace and its resource limits.
///
/// Serializable so it can be persisted (e.g. inside a paused run's state) and
/// reconstructed deterministically on resume.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SandboxSpec {
    /// Working directory mounted as the workspace root (the guest cwd `/work`).
    pub workspace: PathBuf,
    /// Extra mounts beyond the workspace root.
    #[serde(default)]
    pub mounts: Vec<Mount>,
    /// Network access policy (default: none).
    #[serde(default)]
    pub network: NetworkPolicy,
    /// Wall-clock timeout for a single `exec`.
    pub timeout: Duration,
    /// Memory cap in docker form (e.g. `"512m"`); `None` = backend default.
    #[serde(default)]
    pub max_mem: Option<String>,
    /// CPU cap (`docker --cpus`); `None` = unlimited.
    #[serde(default)]
    pub cpus: Option<f64>,
    /// Container image for the docker backend (default: [`DockerSandbox::DEFAULT_IMAGE`]).
    #[serde(default)]
    pub image: Option<String>,
    /// Environment variables injected into the exec. Vault-resolved secrets
    /// land here so plaintext never crosses a wire or the model context.
    #[serde(default)]
    pub env: Vec<(String, String)>,
}

impl SandboxSpec {
    /// A spec rooted at `workspace` with a 60s default timeout and no network.
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            mounts: Vec::new(),
            network: NetworkPolicy::None,
            timeout: Duration::from_secs(60),
            max_mem: None,
            cpus: None,
            image: None,
            env: Vec::new(),
        }
    }

    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }
    pub fn with_network(mut self, n: NetworkPolicy) -> Self {
        self.network = n;
        self
    }
    pub fn with_env(mut self, env: Vec<(String, String)>) -> Self {
        self.env = env;
        self
    }
    pub fn with_image(mut self, image: impl Into<String>) -> Self {
        self.image = Some(image.into());
        self
    }
    pub fn with_max_mem(mut self, m: impl Into<String>) -> Self {
        self.max_mem = Some(m.into());
        self
    }
    pub fn with_cpus(mut self, c: f64) -> Self {
        self.cpus = Some(c);
        self
    }
    pub fn with_mount(mut self, m: Mount) -> Self {
        self.mounts.push(m);
        self
    }
}

/// Captured result of a single `exec`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
    pub timed_out: bool,
}

impl ExecOutput {
    /// True only when the command exited 0 and did not time out.
    pub fn success(&self) -> bool {
        self.code == 0 && !self.timed_out
    }
}

/// Errors a sandbox backend can return.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("path '{0}' escapes the sandbox workspace")]
    PathEscape(String),
    #[error("sandbox backend unavailable: {0}")]
    Unavailable(String),
    #[error("exec failed: {0}")]
    Exec(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SandboxError>;

/// A backend that can run commands and manipulate files inside a bounded
/// workspace. Implementations must keep all file operations within the
/// workspace jail (see [`resolve_in_workspace`]).
#[async_trait]
pub trait SandboxClient: Send + Sync {
    /// Short label for diagnostics, e.g. `"local(unsafe)"` or `"docker"`.
    fn label(&self) -> &'static str;

    /// Whether this backend is actually usable in the current environment
    /// (e.g. the docker backend checks the `docker` CLI is present).
    async fn available(&self) -> bool;

    /// Run a command. `argv[0]` is the program; `argv[1..]` the arguments.
    /// `stdin` is fed to the process if `Some`.
    async fn exec(&self, argv: &[String], stdin: Option<&str>) -> Result<ExecOutput>;

    /// Read a workspace-relative file (jail-enforced).
    async fn read(&self, guest_path: &str) -> Result<Vec<u8>>;

    /// Write a workspace-relative file, creating parent dirs (jail-enforced).
    async fn write(&self, guest_path: &str, bytes: &[u8]) -> Result<()>;

    /// List the entries of a workspace-relative directory (jail-enforced).
    async fn list(&self, guest_dir: &str) -> Result<Vec<String>>;

    /// Snapshot the workspace; returns an opaque handle (a path for the local /
    /// bind-mounted backends).
    async fn snapshot(&self) -> Result<String>;
}

/// Construct a backend by kind: `"local"` or `"docker"`.
pub fn create(spec: SandboxSpec, kind: &str) -> Result<std::sync::Arc<dyn SandboxClient>> {
    match kind {
        "local" => Ok(std::sync::Arc::new(LocalProcessSandbox::new(spec))),
        "docker" => Ok(std::sync::Arc::new(DockerSandbox::new(spec))),
        other => Err(SandboxError::Other(format!(
            "unknown sandbox kind '{other}'"
        ))),
    }
}

/// Resolve a workspace-relative (or absolute) path to a real path **inside**
/// `workspace`, rejecting `..` traversal and symlink escape.
///
/// The deepest *existing* ancestor is canonicalized (so a not-yet-created file
/// is still checked against where it would land), and the resolved real path
/// must remain under the canonicalized workspace. This defeats both `../`
/// traversal and a symlink inside the workspace that points outside it.
pub fn resolve_in_workspace(
    workspace: &Path,
    guest_path: &str,
    _need_write: bool,
) -> Result<PathBuf> {
    let rel = Path::new(guest_path);
    if rel.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(SandboxError::PathEscape(guest_path.to_string()));
    }
    let ws = workspace.canonicalize().map_err(|e| {
        SandboxError::Other(format!(
            "workspace '{}' not found: {e}",
            workspace.display()
        ))
    })?;
    let candidate = if rel.is_absolute() {
        rel.to_path_buf()
    } else {
        ws.join(rel)
    };
    let real = canonicalize_existing_ancestor(&candidate)?;
    if !real.starts_with(&ws) {
        return Err(SandboxError::PathEscape(guest_path.to_string()));
    }
    Ok(candidate)
}

/// Canonicalize the deepest existing ancestor of `p`, re-appending the
/// not-yet-existing tail. Used to validate paths for files that don't exist yet.
fn canonicalize_existing_ancestor(p: &Path) -> Result<PathBuf> {
    let mut cur: &Path = p;
    loop {
        if cur.exists() {
            let real = cur.canonicalize()?;
            if cur == p {
                return Ok(real);
            }
            let tail = p
                .strip_prefix(cur)
                .map_err(|_| SandboxError::PathEscape(p.display().to_string()))?;
            return Ok(real.join(tail));
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return Err(SandboxError::PathEscape(p.display().to_string())),
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn spec_serde_roundtrip_preserves_policy_and_limits() {
        let spec = SandboxSpec::new("/tmp/ws")
            .with_network(NetworkPolicy::Host)
            .with_max_mem("1g")
            .with_cpus(2.0)
            .with_timeout(Duration::from_secs(30));
        let json = serde_json::to_string(&spec).unwrap();
        let back: SandboxSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.network, NetworkPolicy::Host);
        assert_eq!(back.max_mem.as_deref(), Some("1g"));
        assert_eq!(back.cpus, Some(2.0));
        assert_eq!(back.timeout, Duration::from_secs(30));
    }

    #[test]
    fn create_rejects_unknown_kind() {
        // Note: the Ok variant (Arc<dyn SandboxClient>) isn't Debug, so match
        // rather than unwrap_err().
        let res = create(SandboxSpec::new("/tmp"), "qemu");
        assert!(matches!(res, Err(SandboxError::Other(_))));
    }
}
