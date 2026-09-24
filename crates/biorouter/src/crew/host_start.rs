//! D-HOST: "Start it for me", one-click hosting.
//!
//! The Host dialog's second step shows the commands that start Crew on the server with this
//! computer's hosting key. Until now the person ran them in a terminal and pasted what they
//! printed back (alice T-27/F5). On an explicit click carrying proof of a person, the daemon
//! now runs **exactly those commands** itself, over non-interactive `ssh` to the login the
//! person typed, shows their output as it arrives, and reads the result the way it reads a
//! paste. The manual path stays.
//!
//! What keeps this from being a remote shell:
//!
//! - **The command is fixed text.** [`host_start_command`] builds it from two values only, both
//!   under a grammar that admits no shell syntax: the workspace name (`[a-z0-9-]`, as the
//!   broker's own rule) and the hosting key this daemon prepared (`[0-9a-f]{64}`, read from its
//!   own registry by preparation ID, never from the request). Nothing a renderer or an agent
//!   writes becomes command text. It is character for character what the dialog shows
//!   (`hostStartCommands` in `ui/desktop/src/components/crew/onboarding/joinText.ts`; a test
//!   reads that file).
//! - **Only the account the person authenticates as.** It is the login they typed for this
//!   workspace, over the same hardened `ssh` the bridge uses (`BatchMode=yes`, strict host-key
//!   checking, no forwarding, the same preflight), so it gains no account or privilege beyond
//!   theirs, and it never prompts: a server that asks for a password or a code refuses with
//!   that classified reason and the person runs the commands themselves.
//!
//!   The plan's wording (§16 D-HOST) is "over the already-authenticated SSH transport". At this
//!   step there is none: the Host dialog saves the connection, and so first signs in, only at
//!   Create, after the start. So the run is a fresh `ssh` to the login the request names, and
//!   nothing ties that login to the host setup. That is the same trust the Create step already
//!   gives the same field: `POST /crew/connections` takes `ssh_target` from the renderer under
//!   the same proof of a person, and its Connect runs `biorouter-crew bridge` over this same
//!   hardened, non-interactive `ssh` to it. A start adds a fixed command to that login and no
//!   account, route or privilege the renderer could not already name.
//! - **Only this computer's own host setup, once at a time.** The preparation must be this
//!   daemon's pending hosting identity, not one already used by a saved connection; a second
//!   click while a run is under way answers that run.

use super::{safe_atom, transport, CrewManager};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;

/// The most output kept (and shown) per run.
const MAX_OUTPUT: usize = 64 * 1024;
/// How long one run may take before it is stopped.
const RUN_LIMIT: Duration = Duration::from_secs(180);
/// How long a finished run's output stays readable.
const KEEP_FINISHED: Duration = Duration::from_secs(15 * 60);
/// At most this many runs at once, across every setup on this daemon.
const MAX_RUNNING: usize = 2;

/// What the person asked to host: the name, and the SSH login and route they typed.
#[derive(Clone, Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HostStartRequest {
    /// This computer's pending hosting identity, from `POST /crew/devices/prepare`.
    pub preparation_id: String,
    /// The workspace name, as the broker's rule allows it (`lab`, `chen-lab`).
    pub workspace_name: String,
    /// The SSH login to start Crew as: an alias or `user@host`.
    pub ssh_target: String,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub identity_file: Option<String>,
    #[serde(default)]
    pub proxy_jump: Option<String>,
}

/// Where a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HostStartState {
    Running,
    /// `ssh` finished and the output was read: see `found` or `problem`.
    Finished,
    /// `ssh` could not run the commands (sign-in, host key, network), or the run was stopped.
    Failed,
}

/// What the output says, read as the dialog reads a paste.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StartOutput {
    /// The text to preview and pin: the `brcrew1:` line `start` printed, else the status
    /// JSON on one line.
    Found { text: String },
    /// Why the output can't pin a workspace: `starting` (Crew was still starting),
    /// `not_installed` (no `biorouter-crew` on the server), `server_error` (it printed an
    /// error, in `detail`), or `unreadable` (nothing Crew printed).
    Problem {
        problem: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

/// A run, as `GET /crew/host/start/{id}` answers it.
#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct HostStartStatus {
    pub job_id: String,
    /// The exact command text that runs, as the dialog shows it.
    pub command: String,
    pub state: HostStartState,
    /// Everything the commands printed so far (stdout and stderr, in arrival order), without
    /// control characters, at most 64 KiB.
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Set once `finished`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<StartOutput>,
    /// Set once `failed`: a typed code (`crew_ssh_auth_required`, `crew_ssh_host_key_unknown`,
    /// `crew_ssh_host_key_changed`, `crew_ssh_unreachable`, `crew_ssh_failed`,
    /// `crew_host_start_timed_out`, `crew_host_start_cancelled`) and a sentence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<HostStartError>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct HostStartError {
    pub code: String,
    pub message: String,
}

/// A refusal to start a run, with the HTTP shape the route answers.
#[derive(Debug)]
pub struct HostStartRefused {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for HostStartRefused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HostStartRefused {}

fn refused(status: u16, code: &'static str, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(HostStartRefused {
        status,
        code,
        message: message.into(),
    })
}

/// The commands that start Crew on the server as `slug` with `bootstrap_key`, character for
/// character what the Host dialog shows (`hostStartCommands`). Refuses any value outside its
/// grammar, so no shell syntax can reach the command.
pub fn host_start_command(slug: &str, bootstrap_key: &str) -> Result<String> {
    ensure!(
        biorouter_crew::workspace_name_valid(slug)
            && slug
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "Workspace names use lowercase letters, numbers and hyphens."
    );
    ensure!(
        bootstrap_key.len() == 64
            && bootstrap_key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "This computer's hosting key is not in the expected form."
    );
    let state_dir = format!("\"$HOME/.local/share/biorouter-crew/{slug}\"");
    Ok([
        "umask 077".to_owned(),
        "mkdir -p \"$HOME/.local/share/biorouter-crew\"".to_owned(),
        format!(
            "\"$HOME/.local/bin/biorouter-crew\" start --state-dir {state_dir} --name {slug} --bootstrap-key {bootstrap_key}"
        ),
        format!("\"$HOME/.local/bin/biorouter-crew\" status --state-dir {state_dir}"),
    ]
    .join("\n"))
}

/// The value inside `"invitation": "…"` when it is a `brcrew1:` line.
fn quoted_invitation(output: &str) -> Option<String> {
    let mut rest = output;
    while let Some((_, after)) = rest.split_once("\"invitation\"") {
        rest = after;
        let Some(value) = after.trim_start().strip_prefix(':') else {
            continue;
        };
        let Some((inside, _)) = value
            .trim_start()
            .strip_prefix('"')
            .and_then(|quoted| quoted.split_once('"'))
        else {
            continue;
        };
        let token: String = inside.chars().filter(|c| !c.is_whitespace()).collect();
        if token.starts_with("brcrew1:") {
            return Some(token);
        }
    }
    None
}

/// Read what the commands printed, as the dialog reads a paste (`readStartOutput`): the
/// invitation line, else the status JSON, else the problem the output shows.
pub fn read_start_output(output: &str, exit_code: Option<i32>) -> StartOutput {
    if let Some(token) = quoted_invitation(output) {
        return StartOutput::Found { text: token };
    }
    if let Some(line) = output.lines().find(|line| line.contains("brcrew1:")) {
        return StartOutput::Found {
            text: line.trim().to_owned(),
        };
    }
    let mut starting = false;
    let mut invitation_error = None;
    for line in output.lines() {
        let Ok(Value::Object(object)) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if object.get("workspace_id").is_some_and(Value::is_string)
            && object.get("socket").is_some_and(Value::is_string)
        {
            return StartOutput::Found {
                text: line.trim().to_owned(),
            };
        }
        if object.get("state").and_then(Value::as_str) == Some("starting")
            && object.contains_key("started_pid")
        {
            starting = true;
        }
        if object.get("invitation").is_some_and(Value::is_null) {
            if let Some(error) = object.get("invitation_error").and_then(Value::as_str) {
                invitation_error.get_or_insert_with(|| error.to_owned());
            }
        }
    }
    let lower = output.to_ascii_lowercase();
    if exit_code == Some(127)
        || (lower.contains("biorouter-crew")
            && (lower.contains("not found") || lower.contains("no such file or directory")))
    {
        return problem("not_installed", None);
    }
    if starting {
        return problem("starting", None);
    }
    let error = invitation_error.or_else(|| {
        output.lines().find_map(|line| {
            line.trim()
                .strip_prefix("Error:")
                .map(|detail| detail.trim().to_owned())
        })
    });
    match error.filter(|detail| !detail.is_empty()) {
        Some(detail) => problem(
            "server_error",
            Some(
                detail
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(200)
                    .collect(),
            ),
        ),
        None => problem("unreadable", None),
    }
}

fn problem(problem: &str, detail: Option<String>) -> StartOutput {
    StartOutput::Problem {
        problem: problem.to_owned(),
        detail,
    }
}

/// Output as it may be shown: printable text, newlines and tabs, nothing a terminal or a
/// screen would act on.
fn shown(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .filter(|c| {
            *c == '\n'
                || *c == '\t'
                || !(c.is_control() || crate::utils::is_invisible_formatting(*c))
        })
        .collect()
}

/// A run's state, its exit code, and what it came to (one of `result` and `error`, once done).
type RunState = (
    HostStartState,
    Option<i32>,
    Option<StartOutput>,
    Option<HostStartError>,
);

struct Job {
    preparation_id: String,
    command: String,
    output: Mutex<Vec<u8>>,
    state: Mutex<RunState>,
    finished_at: Mutex<Option<Instant>>,
    cancel: tokio_util::sync::CancellationToken,
}

impl Job {
    fn status(&self, job_id: &str) -> HostStartStatus {
        let (state, exit_code, result, error) = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        HostStartStatus {
            job_id: job_id.to_owned(),
            command: self.command.clone(),
            state,
            output: shown(
                &self
                    .output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
            exit_code,
            result,
            error,
        }
    }
    fn finish(
        &self,
        state: HostStartState,
        exit_code: Option<i32>,
        result: Option<StartOutput>,
        error: Option<HostStartError>,
    ) {
        *self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = (state, exit_code, result, error);
        *self
            .finished_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
    }
    fn running(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0
            == HostStartState::Running
    }
}

static JOBS: LazyLock<Mutex<HashMap<String, Arc<Job>>>> = LazyLock::new(Default::default);

fn jobs() -> std::sync::MutexGuard<'static, HashMap<String, Arc<Job>>> {
    let mut jobs = JOBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    jobs.retain(|_, job| {
        job.finished_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none_or(|at| at.elapsed() < KEEP_FINISHED)
    });
    jobs
}

/// A run's status, or `None` for an unknown (or long finished) run.
pub fn host_start_status(job_id: &str) -> Option<HostStartStatus> {
    jobs().get(job_id).map(|job| job.status(job_id))
}

/// Stop a run. `false` for an unknown run; stopping a finished run changes nothing.
pub fn cancel_host_start(job_id: &str) -> bool {
    match jobs().get(job_id) {
        Some(job) => {
            job.cancel.cancel();
            true
        }
        None => false,
    }
}

/// Starts run one at a time, so two clicks can never both start a run for one host setup.
static STARTING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

/// The login and route, refused unless each is in the form a saved connection allows.
fn check_route(request: &HostStartRequest) -> Result<()> {
    if !safe_atom(&request.ssh_target) {
        return Err(refused(
            400,
            "crew_request_invalid",
            "Type the server login as an SSH alias or user@host.",
        ));
    }
    if let Some(jump) = &request.proxy_jump {
        if !jump.split(',').all(safe_atom) {
            return Err(refused(
                400,
                "crew_request_invalid",
                "The jump host isn't in a form SSH accepts.",
            ));
        }
    }
    if let Some(identity) = &request.identity_file {
        if !std::path::Path::new(identity).is_absolute() || identity.contains('\n') {
            return Err(refused(
                400,
                "crew_request_invalid",
                "The key file must be an absolute path.",
            ));
        }
    }
    Ok(())
}

/// Why a finished `ssh` counts as failed, if it does: stopped, too slow, or `ssh` itself
/// (exit 255) could not run the commands.
fn failure(
    cancelled: bool,
    timed_out: bool,
    exit_code: Option<i32>,
    stderr: &str,
) -> Option<HostStartError> {
    if cancelled {
        return Some(HostStartError {
            code: "crew_host_start_cancelled".into(),
            message:
                "Stopped. Crew may have started on the server; run the commands yourself to check."
                    .into(),
        });
    }
    if timed_out {
        return Some(HostStartError {
            code: "crew_host_start_timed_out".into(),
            message: "Starting Crew took too long, so it was stopped. Run the commands yourself to see what the server says.".into(),
        });
    }
    // ssh's own failure: it never ran the commands, or lost the server.
    (exit_code == Some(255)).then(|| {
        let kind = transport::classify_exit(exit_code, stderr);
        HostStartError {
            code: kind.api_code().into(),
            message: ssh_sentence(kind),
        }
    })
}

/// Copy `reader` into the run's output and, when given, a second buffer of its own.
async fn capture(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    job: Arc<Job>,
    also: Option<Arc<Mutex<Vec<u8>>>>,
) {
    let mut buffer = [0u8; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                for sink in std::iter::once(&job.output).chain(also.as_deref()) {
                    let mut sink = sink
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let room = MAX_OUTPUT.saturating_sub(sink.len());
                    sink.extend_from_slice(&buffer[..read.min(room)]);
                }
            }
        }
    }
}

/// Watch a started `ssh` to its end (or the limit, or a cancel), then record what it came to.
fn watch(job: Arc<Job>, mut child: tokio::process::Child) {
    let stdout = child
        .stdout
        .take()
        .map(|out| tokio::spawn(capture(out, Arc::clone(&job), None)));
    let stderr_buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr = child.stderr.take().map(|err| {
        tokio::spawn(capture(
            err,
            Arc::clone(&job),
            Some(Arc::clone(&stderr_buffer)),
        ))
    });
    tokio::spawn(async move {
        let waited = tokio::select! {
            status = tokio::time::timeout(RUN_LIMIT, child.wait()) => status.ok(),
            () = job.cancel.cancelled() => None,
        };
        let cancelled = job.cancel.is_cancelled();
        let timed_out = waited.is_none();
        if timed_out {
            let _ = child.kill().await;
        }
        for task in [stdout, stderr].into_iter().flatten() {
            let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        }
        let exit_code = waited
            .and_then(|status| status.ok())
            .and_then(|status| status.code());
        let stderr = shown(
            &stderr_buffer
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        match failure(cancelled, timed_out, exit_code, &stderr) {
            Some(error) => job.finish(HostStartState::Failed, exit_code, None, Some(error)),
            None => {
                let output = shown(
                    &job.output
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
                let result = read_start_output(&output, exit_code);
                job.finish(HostStartState::Finished, exit_code, Some(result), None);
            }
        }
    });
}

impl CrewManager {
    /// The hosting key of `preparation_id`, when it is this computer's pending host setup and
    /// no saved connection has used it yet.
    async fn host_setup_key(&self, preparation_id: &str) -> Result<String> {
        let registry = self.registry.lock().await;
        if registry.completed_preparations.contains_key(preparation_id) {
            return Err(refused(
                409,
                "crew_host_setup_used",
                "This host setup already has a saved connection. Open it from Crew.",
            ));
        }
        registry
            .pending_device
            .as_ref()
            .filter(|prepared| prepared.preparation_id == preparation_id)
            .map(|prepared| prepared.public_key.clone())
            .ok_or_else(|| {
                refused(
                    409,
                    "crew_host_setup_unknown",
                    "This computer has no host setup with that ID. Start hosting again.",
                )
            })
    }

    /// Start the host setup's commands on the server (see the module documentation). Answers
    /// the run already under way for the same setup rather than starting a second.
    pub async fn start_host(&self, request: HostStartRequest) -> Result<HostStartStatus> {
        check_route(&request)?;
        let bootstrap_key = self.host_setup_key(&request.preparation_id).await?;
        let command = host_start_command(&request.workspace_name, &bootstrap_key)
            .map_err(|error| refused(400, "crew_request_invalid", error.to_string()))?;
        let _starting = STARTING.lock().await;
        {
            let jobs = jobs();
            if let Some((id, job)) = jobs
                .iter()
                .find(|(_, job)| job.running() && job.preparation_id == request.preparation_id)
            {
                return Ok(job.status(id));
            }
            if jobs.values().filter(|job| job.running()).count() >= MAX_RUNNING {
                return Err(refused(
                    409,
                    "crew_host_start_busy",
                    "Crew is already being started on a server from this computer. Wait for it to finish.",
                ));
            }
        }
        let mut args = transport::login_args(
            request.port,
            request.identity_file.as_deref(),
            request.proxy_jump.as_deref(),
            None,
        );
        args.extend([
            "-o".into(),
            "BatchMode=yes".into(),
            request.ssh_target.clone(),
            command.clone(),
        ]);
        // The preflight reads the invocation that will run, login included, as
        // `Transport::connect` does: `ssh -G` with options and no host prints its usage and
        // exits 255, which would refuse every start.
        super::ssh_policy::preflight(&args, &request.ssh_target).await?;
        let mut ssh = tokio::process::Command::new("ssh");
        ssh.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        crate::subprocess::prepare_agent_child_command(&mut ssh);
        let child = ssh.spawn().context("Couldn't run ssh")?;
        let job_id = uuid::Uuid::new_v4().to_string();
        let job = Arc::new(Job {
            preparation_id: request.preparation_id,
            command,
            output: Mutex::new(Vec::new()),
            state: Mutex::new((HostStartState::Running, None, None, None)),
            finished_at: Mutex::new(None),
            cancel: tokio_util::sync::CancellationToken::new(),
        });
        jobs().insert(job_id.clone(), Arc::clone(&job));
        watch(Arc::clone(&job), child);
        Ok(job.status(&job_id))
    }
}

/// What to tell the person when `ssh` itself failed. The manual path is always the way on.
fn ssh_sentence(kind: super::SshFailureKind) -> String {
    use super::SshFailureKind as Kind;
    match kind {
        Kind::AuthRequired => "The server asks for a password or a code, so Biorouter can't sign in for you. Run the commands yourself in a terminal.",
        Kind::HostKeyUnknown => "This computer hasn't connected to the server before. Sign in once in a terminal to check its fingerprint, then try again.",
        Kind::HostKeyChanged => "The server's identity changed since this computer last connected. Check with the server's administrator before you continue.",
        Kind::Unreachable => "Couldn't reach the server. Check the login and your network, then try again.",
        Kind::BridgeMissing | Kind::Other => "SSH couldn't run the commands. Run them yourself in a terminal to see why.",
    }
    .to_owned()
}
