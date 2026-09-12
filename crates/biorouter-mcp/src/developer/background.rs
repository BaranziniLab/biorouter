//! Background shell jobs for the developer extension.
//!
//! Supervises long-running commands started by `shell` with `background=true`
//! so the agent can launch something (a dev server, build, test suite, training
//! run), keep watching it, and continue once it finishes, without killing the
//! job or guessing whether it is done. The shape mirrors Claude Code / Codex:
//! start in a background process group, get a durable `job_id`, read *only new
//! output since the last check*, and decide done-vs-running from the **OS exit
//! status** (never from log text). `wait` adds a bounded in-turn watch that
//! returns the instant the job exits, or at a timeout with `status: running`
//! so the agent can keep watching while the job stays alive.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{watch, Mutex};

use super::shell::{configure_shell_command, ShellConfig};
use crate::active_work::{active_work, ActiveWorkKind};

/// Cap a single job's captured output so a runaway process cannot exhaust
/// memory. Matches the foreground shell's 400 KB ceiling.
const MAX_OUTPUT_BYTES: usize = 400_000;
/// Ceiling for `shell_status`'s bounded wait.
///
/// There is no default: the retired `shell_wait` had one because waiting was
/// the whole tool, whereas `shell_status` waits only when the caller passes
/// `timeout_secs`, and an absent value means "peek", not "wait 120 seconds".
pub const MAX_WAIT_SECS: u64 = 600;

/// Terminal-or-running state of a job. `Exited` carries the real OS exit code,
/// the only source of truth for done-vs-running.
#[derive(Debug, Clone, PartialEq, Eq)]
enum JobStatus {
    Running,
    Exited(i32),
    /// Process ended without a normal exit code (e.g. killed by a signal) and
    /// was not a `shell_kill`.
    Ended(String),
    Killed,
}

impl JobStatus {
    fn is_terminal(&self) -> bool {
        !matches!(self, JobStatus::Running)
    }

    fn describe(&self) -> String {
        match self {
            JobStatus::Running => "running".to_string(),
            JobStatus::Exited(0) => "exited(0), success".to_string(),
            JobStatus::Exited(code) => format!("exited({code}), non-zero"),
            JobStatus::Ended(why) => format!("ended ({why})"),
            JobStatus::Killed => "killed".to_string(),
        }
    }
}

/// Captured output plus a per-job read cursor so reads return only what is new
/// since the previous read.
#[derive(Default)]
struct Output {
    buf: String,
    cursor: usize,
    truncated: bool,
}

struct Job {
    label: String,
    command: String,
    started: Instant,
    /// Process-group leader pid (== child pid because we spawn it in its own
    /// group), used to signal the whole group on kill.
    pid: Option<u32>,
    /// Proof that `pid` still names *this* job's child and not a pid the OS
    /// recycled in the meantime — checked before any force-kill (see
    /// "orphan reaping" / `same_process`).
    identity: Arc<JobIdentity>,
    status: Arc<Mutex<JobStatus>>,
    output: Arc<Mutex<Output>>,
    /// Set before signalling so the supervisor records `Killed` rather than
    /// `Ended` when `wait()` returns.
    killed: Arc<AtomicBool>,
    /// Flips to `true` once the job reaches a terminal state; `wait` parks on
    /// this race-free instead of polling.
    done_rx: watch::Receiver<bool>,
}

/// Registry of background shell jobs, shared (via `Arc`) by the developer
/// server's `shell`, `shell_status` and `shell_kill` tools.
#[derive(Default)]
pub struct BackgroundJobs {
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    next_id: AtomicU64,
}

impl BackgroundJobs {
    pub fn new() -> Self {
        // Reap background jobs orphaned by a previously-crashed Biorouter
        // process before we start tracking new ones (see "orphan reaping").
        reap_orphans();
        Self {
            jobs: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Spawn `command` as a background job in its own process group, wire up
    /// output capture and a supervisor that records the terminal status, and
    /// register the job. Returns the new job id.
    ///
    /// `session_id` is the chat that started the job, carried onto its
    /// active-work row: the listing shows a row only to a caller that could open
    /// its chat, and answers a row that names no chat as a private chat's
    /// (issue #56).
    pub async fn spawn(
        &self,
        command: &str,
        label: Option<String>,
        working_dir: Option<PathBuf>,
        session_id: Option<String>,
    ) -> Result<String, String> {
        let id = format!("job-{}", self.next_id.fetch_add(1, Ordering::SeqCst));
        let label = label.unwrap_or_else(|| command.chars().take(40).collect());

        // Reuse the foreground shell's hardened command builder (same shell,
        // sanitized git/editor env, own process group) but override
        // kill_on_drop: a background job must survive the tool call returning.
        let shell_config = ShellConfig::default();
        // Returns Err only under `BIOROUTER_SHELL_SANDBOX=strict` on a host that
        // cannot provide a full sandbox — refuse to start the job (BR-69).
        let mut cmd = configure_shell_command(&shell_config, command, working_dir.as_deref())?;
        cmd.kill_on_drop(false);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to start command: {e}"))?;
        let pid = child.id();

        // Record this job's process-group-leader pid to the run dir so a future
        // Biorouter process can reap it if we crash before it finishes; the
        // supervisor removes the record once the job exits (see "orphan
        // reaping"). kill_on_drop(false) means nothing else would clean it up.
        // The pid alone is not a safe kill target — pids get recycled — so the
        // record also carries the job's identity (when we started it, what we
        // ran), which every kill path re-verifies against the live process.
        let identity = Arc::new(JobIdentity {
            started_epoch: now_epoch(),
            command: sanitize_command(command),
        });
        if let Some(pid) = pid {
            record_job_pidfile(pid, &identity);
        }

        let status = Arc::new(Mutex::new(JobStatus::Running));
        let output = Arc::new(Mutex::new(Output::default()));
        let killed = Arc::new(AtomicBool::new(false));
        let (done_tx, done_rx) = watch::channel(false);

        if let Some(out) = child.stdout.take() {
            spawn_reader(out, output.clone());
        }
        if let Some(err) = child.stderr.take() {
            spawn_reader(err, output.clone());
        }

        // Surface this job in the process-wide "active work" view (BR-42) with a
        // cancel action that kills its process group, and deregister it when the
        // supervisor sees it reach a terminal state below.
        let reg_id = {
            let killed_for_cancel = killed.clone();
            let pid_for_cancel = pid;
            let identity_for_cancel = identity.clone();
            active_work().register(
                ActiveWorkKind::BackgroundJob,
                format!("{id}: {label}"),
                Some(command.to_string()),
                session_id,
                Some(Arc::new(move || {
                    killed_for_cancel.store(true, Ordering::SeqCst);
                    kill_process_group(pid_for_cancel, identity_for_cancel.clone());
                })),
            )
        };

        // Supervisor: own the child so it is not dropped (and thus not killed)
        // when the tool returns; record the terminal status from the OS.
        let status_for_sup = status.clone();
        let killed_for_sup = killed.clone();
        let pid_for_sup = pid;
        let reg_id_for_sup = reg_id.clone();
        tokio::spawn(async move {
            let wait_result = child.wait().await;
            let terminal = if killed_for_sup.load(Ordering::SeqCst) {
                JobStatus::Killed
            } else {
                match wait_result {
                    Ok(st) => match st.code() {
                        Some(code) => JobStatus::Exited(code),
                        None => JobStatus::Ended("terminated by signal".to_string()),
                    },
                    Err(e) => JobStatus::Ended(format!("wait failed: {e}")),
                }
            };
            *status_for_sup.lock().await = terminal;
            // The job reached a terminal state under our supervision, so it is
            // no longer an orphan candidate — drop its run-dir record — and it is
            // no longer "active work".
            if let Some(pid) = pid_for_sup {
                remove_job_pidfile(pid);
            }
            active_work().deregister(&reg_id_for_sup);
            let _ = done_tx.send(true);
        });

        let job = Arc::new(Job {
            label,
            command: command.to_string(),
            started: Instant::now(),
            pid,
            identity,
            status,
            output,
            killed,
            done_rx,
        });
        self.jobs.lock().await.insert(id.clone(), job);
        Ok(id)
    }

    async fn job(&self, id: &str) -> Result<Arc<Job>, String> {
        self.jobs
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| format!("no such background job: {id}"))
    }

    /// Return the new output since the last read (advancing the cursor) plus a
    /// truncation note if the buffer cap was hit.
    async fn drain_new_output(job: &Job) -> String {
        let mut out = job.output.lock().await;
        let new: String = out.buf.get(out.cursor..).unwrap_or("").to_string();
        out.cursor = out.buf.len();
        let mut s = new;
        if out.truncated {
            s.push_str("\n[output truncated at 400 KB]");
        }
        s
    }

    /// Status + only-new-output snapshot for a job.
    pub async fn snapshot(&self, id: &str) -> Result<String, String> {
        let job = self.job(id).await?;
        let status = job.status.lock().await.describe();
        let new_output = Self::drain_new_output(&job).await;
        let elapsed = job.started.elapsed().as_secs();
        let body = if new_output.trim().is_empty() {
            "(no new output)".to_string()
        } else {
            new_output
        };
        Ok(format!(
            "job {id} [{}], status: {status} ({elapsed}s elapsed)\nnew output since last check:\n{body}",
            job.label
        ))
    }

    /// Watch a job for up to `dur_secs`, returning early when it exits.
    pub async fn wait(&self, id: &str, dur_secs: u64) -> Result<String, String> {
        let job = self.job(id).await?;
        if !job.status.lock().await.is_terminal() {
            let mut rx = job.done_rx.clone();
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(dur_secs),
                rx.wait_for(|done| *done),
            )
            .await;
        }
        let terminal = job.status.lock().await.is_terminal();
        let snap = self.snapshot(id).await?;
        if terminal {
            Ok(format!("{snap}\n\nThe job has finished."))
        } else {
            Ok(format!(
                "{snap}\n\nStill running after {dur_secs}s. The job was NOT killed. Call shell_status again to keep watching, or do other work and check back."
            ))
        }
    }

    /// Kill a job's whole process group (graceful first, then force).
    pub async fn kill(&self, id: &str) -> Result<String, String> {
        let job = self.job(id).await?;
        if job.status.lock().await.is_terminal() {
            return Ok(format!("job {id} has already finished; nothing to kill"));
        }
        job.killed.store(true, Ordering::SeqCst);
        kill_process_group(job.pid, job.identity.clone());
        let mut rx = job.done_rx.clone();
        let confirmed = matches!(
            tokio::time::timeout(
                std::time::Duration::from_secs(KILL_CONFIRM_SECS),
                rx.wait_for(|done| *done),
            )
            .await,
            Ok(Ok(_))
        ) || job.status.lock().await.is_terminal();
        if !confirmed {
            job.killed.store(false, Ordering::SeqCst);
            return Err(format!(
                "sent kill signal to job {id}, but its exit was not confirmed within {KILL_CONFIRM_SECS} seconds"
            ));
        }
        Ok(format!("sent kill signal to job {id}"))
    }

    /// One-line summary of every background job — id, label, status, runtime,
    /// whether unread output is waiting, and the command — so the agent can
    /// rediscover jobs whose `job_id` it has lost. Backs `shell_status` with no job_id.
    pub async fn list(&self) -> String {
        let jobs = self.jobs.lock().await;
        if jobs.is_empty() {
            return "No background jobs.".to_string();
        }
        let mut ids: Vec<_> = jobs.keys().cloned().collect();
        ids.sort();
        let mut lines = Vec::new();
        for id in ids {
            let job = &jobs[&id];
            let status = job.status.lock().await.describe();
            // Peek at the read cursor without draining it, so listing a job
            // doesn't consume the output a later `shell_status` should return.
            let new_output = {
                let out = job.output.lock().await;
                out.cursor < out.buf.len()
            };
            let pending = if new_output {
                " · new output available"
            } else {
                ""
            };
            lines.push(format!(
                "- {id} [{}]: {status} ({}s elapsed){pending}, `{}`",
                job.label,
                job.started.elapsed().as_secs(),
                job.command
            ));
        }
        lines.join("\n")
    }
}

/// Stream a child pipe into the shared buffer, line by line, honoring the cap.
fn spawn_reader<R>(reader: R, output: Arc<Mutex<Output>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut out = output.lock().await;
            if out.buf.len() >= MAX_OUTPUT_BYTES {
                out.truncated = true;
                continue;
            }
            out.buf.push_str(&line);
            out.buf.push('\n');
        }
    });
}

/// How long a job gets to shut down cleanly before we force-kill it. Same on
/// both platforms: Unix sends SIGTERM then SIGKILL, Windows asks `taskkill` to
/// close the tree then escalates to `/F`.
const GRACE_MS: u64 = 1500;
const KILL_CONFIRM_SECS: u64 = 12;
#[cfg(windows)]
const CONTROL_COMMAND_MS: u64 = 3000;

/// Kill the whole process group led by `pid`: ask it to stop, give it
/// `GRACE_MS` to flush, then force. Mirrors the foreground shell's kill idiom.
///
/// The forced second phase re-verifies `identity` first. By then the job may
/// already have exited and the OS may have handed its pid to someone else
/// (Windows recycles pids aggressively), and a force-kill is tree-wide — so we
/// only escalate if the pid still looks like the process we started.
fn kill_process_group(pid: Option<u32>, identity: Arc<JobIdentity>) {
    let Some(pid) = pid else { return };
    #[cfg(unix)]
    {
        unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(GRACE_MS)).await;
            if still_ours(pid, &identity).await {
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            }
        });
    }
    #[cfg(windows)]
    {
        // Graceful phase: `taskkill` *without* `/F` posts WM_CLOSE / a console
        // close event to the tree, which is the closest Windows equivalent of
        // SIGTERM — the job gets to flush its output and remove its own files.
        // Going straight to `/F` (as this used to) killed dev servers and test
        // runners mid-write.
        tokio::spawn(async move {
            if let Err(error) = run_taskkill(pid, false).await {
                tracing::warn!("Graceful taskkill failed for background job {pid}: {error}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(GRACE_MS)).await;
            if still_ours(pid, &identity).await {
                if let Err(error) = run_taskkill(pid, true).await {
                    tracing::warn!("Forced taskkill failed for background job {pid}: {error}");
                }
            }
        });
    }
}

#[cfg(windows)]
async fn run_taskkill(pid: u32, force: bool) -> Result<(), String> {
    let program = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or_else(|| "SystemRoot is not set; cannot locate taskkill.exe".to_string())?
        .join("System32")
        .join("taskkill.exe");
    let mut command = tokio::process::Command::new(program);
    if force {
        command.arg("/F");
    }
    command
        .args(["/T", "/PID", &pid.to_string()])
        .kill_on_drop(true);
    crate::developer::shell::strip_daemon_private_env(&mut command);
    let output = tokio::time::timeout(
        std::time::Duration::from_millis(CONTROL_COMMAND_MS),
        command.output(),
    )
    .await
    .map_err(|_| format!("taskkill timed out for pid {pid}"))?
    .map_err(|e| format!("failed to run taskkill for pid {pid}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(format!(
        "taskkill failed for pid {pid} with {}; stdout: {}; stderr: {}",
        output.status,
        stdout.trim(),
        stderr.trim()
    ))
}

/// Whether `pid` still names the process we recorded, for a kill we are already
/// committed to (a `shell_kill` / cancel whose graceful phase just elapsed).
///
/// Unlike reaping, this pid was recorded seconds ago by *this* process and the
/// job is still tracked as running, so "the OS won't tell us" (no `ps`, no
/// PowerShell) is not evidence of pid reuse — we proceed. We back off only when
/// the OS positively describes a *different* process.
async fn still_ours(pid: u32, identity: &JobIdentity) -> bool {
    let identity = identity.clone();
    tokio::task::spawn_blocking(move || match process_facts(pid) {
        Some(facts) => same_process(Some(&identity), &facts),
        None => true,
    })
    .await
    .unwrap_or(true)
}

// ── orphan reaping ──────────────────────────────────────────────────────────
// Background jobs are spawned with `kill_on_drop(false)` so they outlive the
// tool call that started them — but that also means a daemon crash orphans the
// whole process group with no supervisor left to reap it. Mirror the llama.cpp
// sidecar's pid-file scheme (`llamacpp_sidecar.rs`): record each job's
// process-group-leader pid under a run dir keyed by *this* process's pid, and
// on the next `BackgroundJobs::new()` in any Biorouter process, kill the process
// groups recorded by processes that no longer exist. Records of still-living
// parents (e.g. a CLI and the desktop app running side by side) are left alone,
// so a live daemon's jobs are never reaped out from under it.
//
// Unlike the sidecar (one child), a Biorouter process can own many background
// jobs, so there is one file per job: `<parent-pid>-<child-pid>.pid`.
//
// A pid is NOT a safe kill target on its own. Pid files outlive crashes, the OS
// recycles pids (Windows especially aggressively), and what we do to a matched
// record is force-kill it *and its whole child tree*. So every record also
// carries the job's identity — when we started it and what we ran — and no kill
// path fires until the live process still matches it (`same_process`). A record
// we cannot positively tie back to our own child is dropped, not killed.

/// Directory holding the per-job pid files. Honors `BIOROUTER_DEVELOPER_RUN_DIR`
/// (tests point it at a scratch dir); otherwise `<data>/developer/run`.
fn run_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("BIOROUTER_DEVELOPER_RUN_DIR") {
        return PathBuf::from(dir);
    }
    use etcetera::AppStrategy;
    etcetera::choose_app_strategy(crate::APP_STRATEGY.clone())
        .map(|s| s.data_dir().join("developer/run"))
        .unwrap_or_else(|_| std::env::temp_dir().join("biorouter/developer/run"))
}

fn pidfile_name(parent: u32, child: u32) -> String {
    format!("{parent}-{child}.pid")
}

/// What we knew about a background job's child at spawn time. Recorded next to
/// the pid so a later kill can prove the pid still names *our* child.
#[derive(Debug, Clone, PartialEq, Eq)]
struct JobIdentity {
    /// Wall-clock seconds since the epoch at which we spawned the child.
    started_epoch: u64,
    /// The command we handed to the shell (whitespace-collapsed, truncated).
    command: String,
}

/// The recorded command is only a fingerprint, not a re-runnable script — keep
/// the pid file small and single-line.
const CMD_RECORD_MAX_CHARS: usize = 200;

/// How far the OS-reported creation time of a live pid may sit from the moment
/// we recorded it and still be the same process. We write the record within
/// milliseconds of spawning, so this only absorbs second-granularity rounding —
/// it is deliberately tight, because it is what rules out a recycled pid.
const START_SKEW_SECS: u64 = 5;

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sanitize_command(command: &str) -> String {
    collapse_ws(command)
        .chars()
        .take(CMD_RECORD_MAX_CHARS)
        .collect()
}

fn record_job_pidfile(child: u32, identity: &JobIdentity) {
    record_pidfile_in(&run_dir(), std::process::id(), child, identity);
}

fn remove_job_pidfile(child: u32) {
    remove_pidfile_in(&run_dir(), std::process::id(), child);
}

/// Body of a pid file: the child pid on line 1 (so the format stays readable
/// and back-compatible), then the identity as `key=value` lines.
fn pidfile_body(child: u32, identity: &JobIdentity) -> String {
    format!(
        "{child}\nstarted={}\ncmd={}\n",
        identity.started_epoch, identity.command
    )
}

/// Child pid + identity from a pid-file body. A body written before identities
/// existed (a bare pid) parses to `(Some(pid), None)` — see `same_process` for
/// what an unidentified record is allowed to do.
fn parse_pidfile(body: &str) -> (Option<u32>, Option<JobIdentity>) {
    let mut lines = body.lines();
    let child = lines.next().and_then(|l| l.trim().parse::<u32>().ok());
    let (mut started, mut command) = (None, None);
    for line in lines {
        if let Some(v) = line.strip_prefix("started=") {
            started = v.trim().parse::<u64>().ok();
        } else if let Some(v) = line.strip_prefix("cmd=") {
            command = Some(v.trim().to_string());
        }
    }
    let identity = match (started, command) {
        (Some(started_epoch), Some(command)) if !command.is_empty() => Some(JobIdentity {
            started_epoch,
            command,
        }),
        _ => None,
    };
    (child, identity)
}

fn record_pidfile_in(dir: &Path, parent: u32, child: u32, identity: &JobIdentity) {
    if std::fs::create_dir_all(dir).is_ok() {
        let _ = std::fs::write(
            dir.join(pidfile_name(parent, child)),
            pidfile_body(child, identity),
        );
    }
}

fn remove_pidfile_in(dir: &Path, parent: u32, child: u32) {
    let _ = std::fs::remove_file(dir.join(pidfile_name(parent, child)));
}

/// Kill background-job process groups recorded by Biorouter processes that are
/// no longer alive, then delete their pid files. Best-effort and idempotent.
fn reap_orphans() {
    reap_orphans_in(&run_dir());
}

/// Returns the child pids whose process groups were killed (for tests).
fn reap_orphans_in(dir: &Path) -> Vec<u32> {
    let mut reaped = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return reaped;
    };
    let me = std::process::id();
    // Liveness is the same answer for every record left by the same parent, and
    // on Windows each probe is a `tasklist` spawn — ask once per distinct pid.
    let mut liveness: HashMap<u32, bool> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pid") {
            continue;
        }
        // Filename is `<parent>-<child>.pid`; the child pid is also stored in
        // the file body (source of truth, matching the sidecar), with the
        // filename as a fallback.
        let stem = path.file_stem().and_then(|s| s.to_str());
        let parent: Option<u32> = stem
            .and_then(|s| s.split('-').next())
            .and_then(|s| s.parse().ok());
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        let (child, identity) = parse_pidfile(&body);
        let child = child.or_else(|| {
            stem.and_then(|s| s.split('-').nth(1))
                .and_then(|s| s.parse().ok())
        });
        let (Some(parent), Some(child)) = (parent, child) else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        // Never touch a record owned by this process or any still-living one.
        if parent == me || *liveness.entry(parent).or_insert_with(|| pid_alive(parent)) {
            continue;
        }
        if *liveness.entry(child).or_insert_with(|| pid_alive(child)) {
            // The pid is live — but is it still *our* child? A stale record plus
            // a recycled pid would otherwise aim a tree-wide force-kill at an
            // innocent process. Only a positive match earns the kill; anything
            // else (mismatch, or an OS that won't tell us) drops the record and
            // leaves the process alone.
            match process_facts(child) {
                Some(facts) if same_process(identity.as_ref(), &facts) => {
                    tracing::info!(
                        "Reaping orphaned background job process group {child} left by exited Biorouter process {parent}"
                    );
                    kill_orphan_group(child);
                    reaped.push(child);
                }
                _ => {
                    tracing::debug!(
                        "Not reaping pid {child} recorded by exited Biorouter process {parent}: it no longer looks like the job we started (pid reuse); dropping the record only"
                    );
                }
            }
        }
        let _ = std::fs::remove_file(&path);
    }
    reaped
}

/// Whether `pid` currently names a live process.
fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0): 0 => exists; EPERM => exists but not ours; ESRCH => gone.
        if unsafe { libc::kill(pid as i32, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("tasklist");
        command.args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"]);
        crate::developer::shell::strip_daemon_private_env_std(&mut command);
        command
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
            .unwrap_or(false)
    }
}

/// What the OS will tell us about a live pid. The two platforms expose
/// different evidence, so each fills in what it has and leaves the rest `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessFacts {
    /// Unix: `pgid == pid`. Every background job is its own group leader
    /// (`process_group(0)`), so a reused pid that is *not* a leader is provably
    /// not ours. `None` on Windows, which has no process-group concept — there
    /// the creation time below is the guard instead.
    group_leader: Option<bool>,
    /// Windows: the process's creation time, in seconds since the epoch. `None`
    /// on Unix, where `ps` has no portable start-time-as-epoch field (macOS has
    /// no `etimes`, and `lstart` is locale-formatted).
    start_epoch: Option<u64>,
    /// The process's command line as the OS reports it.
    cmdline: String,
}

/// Does the live process described by `facts` still look like the job we
/// recorded? This is the PID-reuse guard for every force-kill in this module.
///
/// Both platforms get a real check:
///   * Unix — the group-leader test (a reused pid is very unlikely to also lead
///     its own group) plus the command fingerprint.
///   * Windows — the creation time must sit within `START_SKEW_SECS` of the
///     moment we wrote the record, plus the command fingerprint. A recycled pid
///     necessarily belongs to a process created *after* ours died, so a stale
///     record cannot pass.
///
/// A record with no identity (written before identities existed) can only fall
/// back to the group-leader test; where even that is unavailable, it earns no
/// kill at all. Refusing to reap is a leaked orphan; reaping the wrong pid takes
/// out somebody else's process tree.
fn same_process(recorded: Option<&JobIdentity>, facts: &ProcessFacts) -> bool {
    let Some(identity) = recorded else {
        return facts.group_leader == Some(true);
    };
    if facts.group_leader == Some(false) {
        return false;
    }
    command_matches(&identity.command, &facts.cmdline)
        && start_matches(identity.started_epoch, facts.start_epoch)
}

/// The OS may report the recorded command wrapped in its shell (`/bin/sh -c
/// <command>`, `cmd /C <command>`) or — when the shell exec'd a simple command
/// instead of forking — already unwrapped, with the shell quoting stripped. So
/// accept the whole fingerprint as a substring, and otherwise fall back to the
/// program name, which survives both transformations.
fn command_matches(recorded: &str, cmdline: &str) -> bool {
    let want = collapse_ws(recorded);
    let have = collapse_ws(cmdline);
    if want.is_empty() || have.is_empty() {
        return false;
    }
    if have.contains(&want) {
        return true;
    }
    let Some(program) = want.split_whitespace().next() else {
        return false;
    };
    have.split_whitespace().any(|tok| {
        tok == program
            || tok.ends_with(&format!("/{program}"))
            || tok.ends_with(&format!("\\{program}"))
    })
}

/// A creation time the OS won't give us cannot veto a kill (Unix never reports
/// one); one it does give us must line up with when we wrote the record.
fn start_matches(recorded: u64, actual: Option<u64>) -> bool {
    let Some(actual) = actual else { return true };
    recorded.abs_diff(actual) <= START_SKEW_SECS
}

#[cfg(unix)]
fn process_facts(pid: u32) -> Option<ProcessFacts> {
    let mut command = std::process::Command::new("ps");
    command.args(["-o", "pgid=,args=", "-p", &pid.to_string()]);
    crate::developer::shell::strip_daemon_private_env_std(&mut command);
    let out = command.output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_ps_facts(&String::from_utf8_lossy(&out.stdout), pid)
}

/// `ps -o pgid=,args=` prints `"<pgid> <argv joined by spaces>"`. A command can
/// itself contain newlines, so the args run to the end of the output rather than
/// the end of the first line.
#[cfg_attr(not(unix), allow(dead_code))]
fn parse_ps_facts(stdout: &str, pid: u32) -> Option<ProcessFacts> {
    let text = stdout.trim_start();
    let (pgid, args) = text.split_once(char::is_whitespace)?;
    let pgid: u32 = pgid.trim().parse().ok()?;
    Some(ProcessFacts {
        group_leader: Some(pgid == pid),
        start_epoch: None,
        cmdline: args.trim().to_string(),
    })
}

#[cfg(windows)]
fn process_facts(pid: u32) -> Option<ProcessFacts> {
    // One CIM query for the two facts Windows *does* expose: when the process
    // was created and its full command line. Only ever run on a kill path (a
    // stale pid record at startup, or an escalation after the grace period), so
    // the cost of a PowerShell spawn is not on any hot path.
    let script = format!(
        "$p = Get-CimInstance Win32_Process -Filter 'ProcessId={pid}' -ErrorAction SilentlyContinue; \
         if ($p) {{ [int64]([datetimeoffset]$p.CreationDate).ToUnixTimeSeconds(); $p.CommandLine }}"
    );
    for shell in ["powershell", "pwsh"] {
        let mut command = std::process::Command::new(shell);
        command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        crate::developer::shell::strip_daemon_private_env_std(&mut command);
        let Ok(out) = command.output() else {
            continue; // shell not installed — try the other one
        };
        if !out.status.success() {
            continue;
        }
        if let Some(facts) = parse_win_facts(&String::from_utf8_lossy(&out.stdout)) {
            return Some(facts);
        }
        // The query ran and found nothing: the pid is gone (or is a protected
        // process we may not inspect). Either way it is not a kill target.
        return None;
    }
    None
}

/// The CIM script prints the creation time (epoch seconds) on the first line and
/// the command line on the rest. Empty output means "no such process".
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_win_facts(stdout: &str) -> Option<ProcessFacts> {
    let mut lines = stdout.lines();
    let start_epoch: u64 = lines.next()?.trim().parse().ok()?;
    let cmdline = collapse_ws(&lines.collect::<Vec<_>>().join(" "));
    Some(ProcessFacts {
        group_leader: None,
        start_epoch: Some(start_epoch),
        cmdline,
    })
}

/// Force-kill the whole process group led by `pid`. Callers must have already
/// established that `pid` is ours (`same_process`); these are known orphans of a
/// dead parent, with nobody left to ask for a graceful shutdown, so they go
/// straight to the hard kill rather than the graceful-then-force path a live
/// `shell_kill` uses.
fn kill_orphan_group(pid: u32) {
    #[cfg(unix)]
    {
        unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
    }
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("taskkill");
        command.args(["/F", "/T", "/PID", &pid.to_string()]);
        crate::developer::shell::strip_daemon_private_env_std(&mut command);
        let _ = command.output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point the pid-file run dir at a per-binary scratch dir (set once) so the
    /// test suite never reaps or records against the real `<data>/developer/run`
    /// — that could kill a developer's actual background jobs during `cargo
    /// test`. Every test that builds a `BackgroundJobs` must go through this.
    fn ensure_test_run_dir() -> &'static Path {
        static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        DIR.get_or_init(|| {
            let d = std::env::temp_dir().join(format!("br-dev-run-tests-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&d);
            std::env::set_var("BIOROUTER_DEVELOPER_RUN_DIR", &d);
            d
        })
    }

    fn new_jobs() -> BackgroundJobs {
        ensure_test_run_dir();
        BackgroundJobs::new()
    }

    /// A fresh, isolated scratch dir for hermetic reaping tests that pass the
    /// dir explicitly and must not share state with other tests.
    fn scratch_run_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d =
            std::env::temp_dir().join(format!("br-dev-run-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// How long a test will wait for a spawned job to reach a terminal state,
    /// or for its output to arrive.
    ///
    /// ⚠ Windows gets six times as long, and the reason is the **shell**, not
    /// the machine. `spawn` runs the command through PowerShell there and
    /// through `sh` on Unix, and a PowerShell cold start is tens of times more
    /// expensive: measured on windows-latest, three of these tests
    /// (`list_reports_command_status_and_unread_output`,
    /// `nonzero_exit_code_is_surfaced`, `output_is_incremental`) went red at
    /// 5 s inside a binary that runs a thousand tests across every core, and
    /// they failed in the shape a too-short deadline produces rather than the
    /// shape a bug produces: the status read `Running` (not a wrong exit code)
    /// and the first output read was EMPTY (not the wrong text). They had all
    /// passed on the same runner forty minutes earlier with no change to this
    /// file, which is what makes it contention rather than behaviour.
    ///
    /// A longer bound costs nothing on a passing run because every wait here
    /// polls and returns as soon as the condition holds. It is a hang detector,
    /// so it should be set where only a hang can reach it — a bound tight
    /// enough to be tripped by a busy runner is measuring the runner.
    const JOB_WAIT_MS: u64 = if cfg!(windows) { 30_000 } else { 5_000 };

    async fn wait_terminal(jobs: &BackgroundJobs, id: &str, max_ms: u64) -> JobStatus {
        let job = jobs.job(id).await.unwrap();
        let deadline = Instant::now() + std::time::Duration::from_millis(max_ms);
        loop {
            let st = job.status.lock().await.clone();
            if st.is_terminal() || Instant::now() > deadline {
                return st;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    }

    async fn collect_output_until(
        jobs: &BackgroundJobs,
        id: &str,
        needle: &str,
        max_ms: u64,
    ) -> String {
        let job = jobs.job(id).await.unwrap();
        let deadline = Instant::now() + std::time::Duration::from_millis(max_ms);
        let mut collected = String::new();
        while Instant::now() <= deadline {
            collected.push_str(&BackgroundJobs::drain_new_output(&job).await);
            if collected.contains(needle) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        collected
    }

    #[tokio::test]
    async fn start_lists_and_completes_with_output() {
        let jobs = new_jobs();
        let id = jobs.spawn("echo hello-bg", None, None, None).await.unwrap();
        assert!(jobs.list().await.contains(&id));
        assert_eq!(
            wait_terminal(&jobs, &id, JOB_WAIT_MS).await,
            JobStatus::Exited(0)
        );
        let snap = jobs.snapshot(&id).await.unwrap();
        assert!(snap.contains("hello-bg"), "snapshot: {snap}");
    }

    #[tokio::test]
    async fn list_reports_command_status_and_unread_output() {
        let jobs = new_jobs();
        let id = jobs.spawn("echo listme", None, None, None).await.unwrap();
        assert_eq!(
            wait_terminal(&jobs, &id, JOB_WAIT_MS).await,
            JobStatus::Exited(0)
        );

        let listing = jobs.list().await;
        assert!(listing.contains(&id), "listing: {listing}");
        assert!(
            listing.contains("echo listme"),
            "command missing: {listing}"
        );
        assert!(
            listing.contains("exited(0)"),
            "terminal status missing: {listing}"
        );
        assert!(listing.contains("elapsed"), "runtime missing: {listing}");
        assert!(
            listing.contains("new output available"),
            "unread output not flagged: {listing}"
        );

        // Listing must not drain the cursor; a real read still sees the output,
        // after which the listing stops flagging it.
        let drained = BackgroundJobs::drain_new_output(&jobs.job(&id).await.unwrap()).await;
        assert!(drained.contains("listme"), "drained: {drained}");
        let listing2 = jobs.list().await;
        assert!(
            !listing2.contains("new output available"),
            "flag should clear after read: {listing2}"
        );
    }

    #[tokio::test]
    async fn list_is_empty_message_with_no_jobs() {
        let jobs = new_jobs();
        assert_eq!(jobs.list().await, "No background jobs.");
    }

    #[tokio::test]
    async fn nonzero_exit_code_is_surfaced() {
        let jobs = new_jobs();
        let id = jobs.spawn("exit 3", None, None, None).await.unwrap();
        assert_eq!(
            wait_terminal(&jobs, &id, JOB_WAIT_MS).await,
            JobStatus::Exited(3)
        );
    }

    #[tokio::test]
    async fn output_is_incremental() {
        let jobs = new_jobs();
        let command = if cfg!(windows) {
            "Write-Output first; Start-Sleep -Milliseconds 2000; Write-Output second"
        } else {
            "echo first; sleep 2; echo second"
        };
        let id = jobs.spawn(command, None, None, None).await.unwrap();
        let first = collect_output_until(&jobs, &id, "first", JOB_WAIT_MS).await;
        assert!(first.contains("first"), "first read: {first}");
        assert!(!first.contains("second"), "second leaked early: {first}");
        assert_eq!(
            wait_terminal(&jobs, &id, JOB_WAIT_MS).await,
            JobStatus::Exited(0)
        );
        let second = collect_output_until(&jobs, &id, "second", JOB_WAIT_MS).await;
        assert!(second.contains("second"), "second read: {second}");
        assert!(!second.contains("first"), "first duplicated: {second}");
    }

    #[tokio::test]
    async fn wait_returns_early_on_completion() {
        let jobs = new_jobs();
        let id = jobs.spawn("echo done", None, None, None).await.unwrap();
        let started = Instant::now();
        let out = jobs.wait(&id, 30).await.unwrap();
        assert!(out.contains("finished"), "wait result: {out}");
        assert!(started.elapsed().as_secs() < 5);
    }

    #[tokio::test]
    async fn wait_times_out_without_killing_then_kill_works() {
        let jobs = new_jobs();
        let id = jobs.spawn("sleep 30", None, None, None).await.unwrap();
        let out = jobs.wait(&id, 1).await.unwrap();
        assert!(out.contains("Still running"), "wait result: {out}");
        assert_eq!(
            *jobs.job(&id).await.unwrap().status.lock().await,
            JobStatus::Running,
            "bounded wait must NOT kill the job"
        );
        jobs.kill(&id).await.unwrap();
        assert_eq!(
            *jobs.job(&id).await.unwrap().status.lock().await,
            JobStatus::Killed,
            "a successful kill must already have an OS-confirmed terminal state"
        );
    }

    #[tokio::test]
    async fn unknown_job_is_clean_error() {
        let jobs = new_jobs();
        assert!(jobs.snapshot("job-nope").await.is_err());
        assert!(jobs.wait("job-nope", 1).await.is_err());
        assert!(jobs.kill("job-nope").await.is_err());
    }

    // ── orphan reaping ──────────────────────────────────────────────────────

    #[cfg(unix)]
    fn spawn_group_leader_sleep() -> std::process::Child {
        use std::os::unix::process::CommandExt;
        // Own process group so pgid == pid, matching how real jobs are spawned.
        // Return the Child so the test can `wait()` it (clearing the zombie a
        // SIGKILL leaves behind, since this process — unlike a dead daemon,
        // whose orphans reparent to init — stays alive to parent it).
        std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .expect("spawn sleep")
    }

    #[cfg(unix)]
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let pid = child.id();
        let _ = child.wait(); // reap it, so kill(pid, 0) => ESRCH
        pid
    }

    /// The identity a real spawn would have recorded for `sleep 30`.
    #[cfg(unix)]
    fn sleep_identity() -> JobIdentity {
        JobIdentity {
            started_epoch: now_epoch(),
            command: "sleep 30".to_string(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn reap_kills_orphan_of_dead_parent_and_spares_live_parent() {
        let dir = scratch_run_dir("reap");

        // Orphan whose recorded parent is gone → must be reaped.
        let mut orphan_child = spawn_group_leader_sleep();
        let orphan = orphan_child.id();
        let dead_parent = dead_pid();
        record_pidfile_in(&dir, dead_parent, orphan, &sleep_identity());

        // Job whose recorded parent (this process) is alive → must be spared.
        let mut live = spawn_group_leader_sleep();
        let live_child = live.id();
        record_pidfile_in(&dir, std::process::id(), live_child, &sleep_identity());

        let reaped = reap_orphans_in(&dir);

        assert!(
            reaped.contains(&orphan),
            "orphan of a dead parent should be reaped, got {reaped:?}"
        );
        // The reaper SIGKILLed the group; reap the zombie so the pid is freed.
        let _ = orphan_child.wait();
        assert!(!pid_alive(orphan), "orphan should be dead after reaping");
        assert!(
            !dir.join(pidfile_name(dead_parent, orphan)).exists(),
            "orphan pid file should be removed after reaping"
        );

        assert!(
            !reaped.contains(&live_child) && pid_alive(live_child),
            "job of a still-living parent must not be reaped"
        );
        assert!(
            matches!(live.try_wait(), Ok(None)),
            "live-parent job must still be running"
        );
        assert!(
            dir.join(pidfile_name(std::process::id(), live_child))
                .exists(),
            "live-parent pid file must be kept"
        );

        let _ = live.kill();
        let _ = live.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn reap_removes_malformed_pidfiles() {
        let dir = scratch_run_dir("malformed");
        std::fs::write(dir.join("not-a-pid.pid"), "garbage").unwrap();
        std::fs::write(dir.join("keep.txt"), "123").unwrap(); // non-.pid, ignored
        reap_orphans_in(&dir);
        assert!(
            !dir.join("not-a-pid.pid").exists(),
            "unparseable pid file should be swept"
        );
        assert!(
            dir.join("keep.txt").exists(),
            "non-.pid files must be left alone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A stale pid file whose pid the OS has since handed to somebody else must
    /// NOT be reaped — the kill is tree-wide and would take out an innocent
    /// process. Stands in for a recycled pid by recording a *different* command
    /// against a live group leader.
    #[cfg(unix)]
    #[test]
    fn reap_spares_a_pid_that_no_longer_matches_the_recorded_job() {
        let dir = scratch_run_dir("recycled");

        let mut innocent = spawn_group_leader_sleep();
        let recycled_pid = innocent.id();
        let dead_parent = dead_pid();
        record_pidfile_in(
            &dir,
            dead_parent,
            recycled_pid,
            &JobIdentity {
                started_epoch: now_epoch(),
                command: "npm run some-long-dev-server".to_string(),
            },
        );

        let reaped = reap_orphans_in(&dir);

        assert!(
            !reaped.contains(&recycled_pid),
            "a pid whose live process does not match the record must not be killed, got {reaped:?}"
        );
        assert!(
            matches!(innocent.try_wait(), Ok(None)),
            "the innocent process must still be running"
        );
        assert!(
            !dir.join(pidfile_name(dead_parent, recycled_pid)).exists(),
            "the unusable record should still be swept"
        );

        let _ = innocent.kill();
        let _ = innocent.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End-to-end: the identity a real spawn records must actually match what
    /// the OS reports for that child — through the shell wrapper
    /// `configure_shell_command` puts in front of it. The unit tests below can
    /// only simulate that wrapper; this one runs it.
    #[cfg(unix)]
    #[tokio::test]
    async fn recorded_identity_matches_the_live_child_of_a_real_spawn() {
        let jobs = new_jobs();
        let id = jobs.spawn("sleep 30", None, None, None).await.unwrap();
        let job = jobs.job(&id).await.unwrap();
        let pid = job.pid.unwrap();

        let facts = process_facts(pid).expect("ps should describe our own live child");
        assert!(
            same_process(Some(&job.identity), &facts),
            "a job we just spawned must pass its own pid-reuse guard; recorded {:?}, live {facts:?}",
            job.identity
        );

        jobs.kill(&id).await.unwrap();
        assert_eq!(
            wait_terminal(&jobs, &id, JOB_WAIT_MS).await,
            JobStatus::Killed
        );
    }

    // ── pid-reuse guard (GAP-2) ─────────────────────────────────────────────
    // `same_process` is the one gate every force-kill goes through. It is pure,
    // so both platforms' evidence shapes can be exercised anywhere: Unix fills
    // in `group_leader`, Windows fills in `start_epoch`.

    fn unix_facts(group_leader: bool, cmdline: &str) -> ProcessFacts {
        ProcessFacts {
            group_leader: Some(group_leader),
            start_epoch: None,
            cmdline: cmdline.to_string(),
        }
    }

    fn windows_facts(start_epoch: u64, cmdline: &str) -> ProcessFacts {
        ProcessFacts {
            group_leader: None,
            start_epoch: Some(start_epoch),
            cmdline: cmdline.to_string(),
        }
    }

    fn identity(started_epoch: u64, command: &str) -> JobIdentity {
        JobIdentity {
            started_epoch,
            command: command.to_string(),
        }
    }

    #[test]
    fn same_process_accepts_our_own_job_on_either_platform() {
        let id = identity(1_700_000_000, "npm run dev");
        assert!(same_process(
            Some(&id),
            &unix_facts(true, "/bin/zsh -c npm run dev")
        ));
        assert!(same_process(
            Some(&id),
            &windows_facts(1_700_000_000, "cmd /C npm run dev")
        ));
    }

    #[test]
    fn same_process_rejects_a_recycled_pid_on_windows() {
        // The exact GAP-2 scenario: a stale record, the pid handed to some other
        // process. Windows has no group-leader check, so without the creation
        // time + command guard this collapsed to "the pid is alive" — which is
        // precisely what a recycled pid satisfies.
        let id = identity(1_700_000_000, "npm run dev");

        // Same pid, unrelated process started hours later.
        assert!(
            !same_process(
                Some(&id),
                &windows_facts(1_700_030_000, "C:\\Windows\\explorer.exe")
            ),
            "an unrelated process holding a recycled pid must not be reapable"
        );
        // Even the same command line, if it was started long after we recorded.
        assert!(
            !same_process(
                Some(&id),
                &windows_facts(1_700_030_000, "cmd /C npm run dev")
            ),
            "creation time far from the record means a different process"
        );
        // And a process created at the right moment but running something else.
        assert!(
            !same_process(
                Some(&id),
                &windows_facts(1_700_000_001, "C:\\Windows\\explorer.exe")
            ),
            "a different command line means a different process"
        );
    }

    #[test]
    fn same_process_rejects_a_reused_pid_that_does_not_lead_its_group_on_unix() {
        let id = identity(1_700_000_000, "npm run dev");
        assert!(!same_process(
            Some(&id),
            &unix_facts(false, "/bin/zsh -c npm run dev")
        ));
    }

    #[test]
    fn same_process_never_kills_on_an_unidentified_record() {
        // Pid files written before identities existed. Unix still has its
        // group-leader guard; Windows (group_leader: None) has nothing left, and
        // must therefore refuse — an un-provable pid is not worth an innocent
        // process tree.
        assert!(same_process(None, &unix_facts(true, "sleep 30")));
        assert!(!same_process(None, &unix_facts(false, "sleep 30")));
        assert!(!same_process(
            None,
            &windows_facts(1_700_000_000, "cmd /C npm run dev")
        ));
    }

    #[test]
    fn command_matches_survives_the_shell_wrapper_and_exec() {
        // Wrapped by the shell we spawned it with.
        assert!(command_matches("npm run dev", "/bin/zsh -c npm run dev"));
        assert!(command_matches("npm run dev", "cmd /C npm run dev"));
        // The shell exec'd a simple command, so the wrapper is gone and the
        // quoting has been stripped — fall back to the program name.
        assert!(command_matches(
            "echo \"hi there\" > out.txt",
            "echo hi there"
        ));
        assert!(command_matches("npm run dev", "node /opt/bin/npm run dev"));
        // Whitespace is not signal.
        assert!(command_matches("npm   run\n dev", "/bin/sh -c npm run dev"));
        // Different program entirely.
        assert!(!command_matches("npm run dev", "/usr/bin/sleep 30"));
        assert!(!command_matches("npm run dev", ""));
        assert!(!command_matches("", "npm run dev"));
    }

    #[test]
    fn start_matches_tolerates_rounding_but_not_a_later_process() {
        assert!(start_matches(1_700_000_000, Some(1_700_000_002)));
        assert!(start_matches(1_700_000_000, Some(1_699_999_998)));
        assert!(!start_matches(1_700_000_000, Some(1_700_000_060)));
        // Unix never reports one; absence cannot veto.
        assert!(start_matches(1_700_000_000, None));
    }

    #[test]
    fn pidfile_round_trips_the_identity() {
        let id = identity(1_700_000_000, "npm run dev");
        let (child, parsed) = parse_pidfile(&pidfile_body(4242, &id));
        assert_eq!(child, Some(4242));
        assert_eq!(parsed.as_ref(), Some(&id));

        // A record from before identities existed: pid only, no identity.
        let (child, parsed) = parse_pidfile("4242");
        assert_eq!(child, Some(4242));
        assert_eq!(parsed, None);

        // Garbage stays garbage (the reaper sweeps it).
        assert_eq!(parse_pidfile("nonsense"), (None, None));
    }

    #[test]
    fn recorded_command_is_single_line_and_bounded() {
        let cmd = sanitize_command(&format!("echo start\n{}\necho end", "x".repeat(500)));
        assert!(!cmd.contains('\n'), "pid file is line-based: {cmd}");
        assert_eq!(cmd.chars().count(), CMD_RECORD_MAX_CHARS);
        // …and it still round-trips through the pid file.
        let id = identity(1, &cmd);
        assert_eq!(parse_pidfile(&pidfile_body(7, &id)).1, Some(id));
    }

    #[test]
    fn parses_ps_output() {
        let facts = parse_ps_facts(" 4242 /bin/zsh -c npm run dev\n", 4242).unwrap();
        assert_eq!(facts.group_leader, Some(true));
        assert_eq!(facts.cmdline, "/bin/zsh -c npm run dev");
        assert_eq!(facts.start_epoch, None);
        // Same pid, but it does not lead its group → not one of our jobs.
        assert_eq!(
            parse_ps_facts(" 999 /bin/zsh -c npm run dev\n", 4242)
                .unwrap()
                .group_leader,
            Some(false)
        );
        // No such process: `ps` prints nothing.
        assert!(parse_ps_facts("", 4242).is_none());
    }

    #[test]
    fn parses_windows_cim_output() {
        let facts = parse_win_facts("1700000000\ncmd /C npm run dev\n").unwrap();
        assert_eq!(facts.start_epoch, Some(1_700_000_000));
        assert_eq!(facts.cmdline, "cmd /C npm run dev");
        assert_eq!(facts.group_leader, None);
        // No such process → empty output → no facts, so no kill.
        assert!(parse_win_facts("").is_none());
        assert!(parse_win_facts("\r\n").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_records_pidfile_and_terminal_removes_it() {
        let dir = ensure_test_run_dir().to_path_buf();
        let jobs = new_jobs();
        let id = jobs.spawn("sleep 30", None, None, None).await.unwrap();
        let pid = jobs.job(&id).await.unwrap().pid.unwrap();

        let pidfile = dir.join(pidfile_name(std::process::id(), pid));
        assert!(pidfile.exists(), "spawn should record a pid file");

        // The record must carry enough to prove the pid is ours later on.
        let (recorded_pid, recorded_id) =
            parse_pidfile(&std::fs::read_to_string(&pidfile).unwrap());
        assert_eq!(recorded_pid, Some(pid));
        let recorded_id = recorded_id.expect("pid file must carry the job identity");
        assert_eq!(recorded_id.command, "sleep 30");
        assert!(recorded_id.started_epoch.abs_diff(now_epoch()) <= START_SKEW_SECS);

        jobs.kill(&id).await.unwrap();
        assert_eq!(
            wait_terminal(&jobs, &id, JOB_WAIT_MS).await,
            JobStatus::Killed
        );

        // The supervisor removes the record once the job reaches a terminal state.
        let mut gone = false;
        for _ in 0..120 {
            if !pidfile.exists() {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(gone, "terminal job should remove its pid file");
    }
}
