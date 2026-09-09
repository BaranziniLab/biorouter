use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, Local, Utc};
use fs2::FileExt as _;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_cron_scheduler::{job::JobId, Job, JobScheduler as TokioJobScheduler};
use tokio_util::sync::CancellationToken;

use crate::agents::AgentEvent;
use crate::agents::{Agent, SessionConfig};
use crate::config::paths::Paths;
use crate::config::{resolve_extensions_for_new_session, Config};
use crate::conversation::message::Message;
use crate::conversation::Conversation;
use crate::scheduler_trait::SchedulerTrait;
use crate::session::session_manager::SessionType;
use crate::session::{Session, SessionManager};
use crate::workflow::Workflow;

type RunningTasksMap = HashMap<String, CancellationToken>;
type JobsMap = HashMap<String, (JobId, ScheduledJob)>;

/// The substring that marks a scheduler error as *"the run was stopped, it did
/// not fail"*.
///
/// It exists because two independent readers key on this text: `run_now_handler`
/// in `biorouter-server`, which turns a cancelled run into the `CANCELLED`
/// sentinel the desktop schedule view (`ScheduleDetailView.tsx`) branches on,
/// and a human reading `last_error`. Before issue #148B a cancelled run was
/// reported as a *success*, so this string was never produced by anything and
/// the route's branch was dead code.
pub const RUN_CANCELLED_MARKER: &str = "was successfully cancelled";

/// Count of in-progress *interactive* (user-facing) agent turns. While > 0 the
/// scheduler defers scheduled jobs so background work doesn't compete with the
/// user for the provider / rate-limit budget (jcode's "pause when active").
static ACTIVE_INTERACTIVE_TURNS: AtomicUsize = AtomicUsize::new(0);

/// RAII guard marking an interactive turn in progress. Hold it for the lifetime
/// of an interactive reply (e.g. the HTTP `/reply` SSE stream).
pub struct InteractiveTurnGuard;

/// Begin an interactive turn; the returned guard decrements the counter on drop.
pub fn interactive_turn_guard() -> InteractiveTurnGuard {
    ACTIVE_INTERACTIVE_TURNS.fetch_add(1, Ordering::SeqCst);
    InteractiveTurnGuard
}

impl Drop for InteractiveTurnGuard {
    fn drop(&mut self) {
        ACTIVE_INTERACTIVE_TURNS.fetch_sub(1, Ordering::SeqCst);
    }
}

fn interactive_active() -> bool {
    ACTIVE_INTERACTIVE_TURNS.load(Ordering::SeqCst) > 0
}

/// Whether to pause scheduled work while a user is interacting. Default on; set
/// `BIOROUTER_SCHEDULER_PAUSE_ON_ACTIVE=0` to disable.
fn pause_on_active() -> bool {
    !matches!(
        std::env::var("BIOROUTER_SCHEDULER_PAUSE_ON_ACTIVE")
            .ok()
            .as_deref(),
        Some("0") | Some("false")
    )
}

pub fn get_default_scheduler_storage_path() -> Result<PathBuf, io::Error> {
    let data_dir = Paths::data_dir();
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("schedule.json"))
}

pub fn get_default_scheduled_workflows_dir() -> Result<PathBuf, SchedulerError> {
    let data_dir = Paths::data_dir();
    let workflows_dir = data_dir.join("scheduled_workflows");
    fs::create_dir_all(&workflows_dir).map_err(SchedulerError::StorageError)?;
    Ok(workflows_dir)
}

/// The largest a schedule id may be. Not a security property on its own — the
/// character set below is — but a `format!("{id}.{ext}")` filename still has to
/// fit a filesystem's name limit, and an id nobody can type is an id nobody can
/// manage.
const MAX_SCHEDULE_ID_LEN: usize = 64;

/// A schedule id is a **plain slug**, because it names a file.
///
/// ⚠ **This is a security boundary, not a tidiness rule.** The id is
/// interpolated into a filename under [`get_default_scheduled_workflows_dir`],
/// and `Path::join` *discards its base* when the argument is absolute, while
/// `..` is resolved by the kernel at `fs::copy`. An unvalidated id is therefore
/// an arbitrary-file-write primitive reachable from `POST /schedule/create`:
/// `{"id": "/tmp/pwned"}` wrote `/tmp/pwned.yaml`.
///
/// Shaped after `biorouter_mcp::knowledge::paths::validate_kb_id`, which exists
/// for exactly the same reason, and deliberately a little wider than it: `_` and
/// upper case are allowed because a schedule id is typed by the user in the GUI
/// and derived from a workflow's file stem by
/// `Scheduler::generate_unique_job_id`, so both already occur in the field.
/// What is refused is every character that can leave the directory or change
/// what the name means — `/`, `\`, `.`, `:`, NUL and the rest.
pub fn validate_schedule_id(id: &str) -> Result<(), SchedulerError> {
    if id.is_empty() {
        return Err(SchedulerError::InvalidJobId(
            "schedule id must not be empty".to_string(),
        ));
    }
    if id.len() > MAX_SCHEDULE_ID_LEN {
        return Err(SchedulerError::InvalidJobId(format!(
            "schedule id is longer than {MAX_SCHEDULE_ID_LEN} characters"
        )));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(SchedulerError::InvalidJobId(format!(
            "schedule id '{id}' may only contain letters, digits, '-' and '_'"
        )));
    }
    Ok(())
}

/// The file extension a copied workflow keeps.
///
/// Read off the *source* path, which is caller-supplied too, so it is checked
/// rather than trusted: it is the second value interpolated into the same
/// filename. A file name cannot contain a path separator on any platform we
/// build for, so this cannot traverse — but `:` on Windows names an alternate
/// data stream, and an absurdly long extension is a name-limit failure with a
/// confusing message. Anything unexpected falls back to `yaml`.
fn workflow_copy_extension(original: &Path) -> String {
    original
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| {
            !ext.is_empty() && ext.len() <= 16 && ext.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .unwrap_or("yaml")
        .to_string()
}

/// Where `make_copy` will put the job's own copy of its workflow, and the check
/// that there is something to copy.
///
/// Deliberately does **no** copying: [`Scheduler::add_scheduled_job`] plans the
/// path here and performs the copy only after every refusal has had its say.
fn planned_workflow_copy(job: &ScheduledJob) -> Result<PathBuf, SchedulerError> {
    let original = Path::new(&job.source);
    if !original.is_file() {
        return Err(SchedulerError::WorkflowLoadError(format!(
            "Workflow file not found: {}",
            job.source
        )));
    }
    let scheduled_workflows_dir = get_default_scheduled_workflows_dir()?;
    Ok(scheduled_workflows_dir.join(format!("{}.{}", job.id, workflow_copy_extension(original))))
}

#[derive(Debug)]
pub enum SchedulerError {
    JobIdExists(String),
    /// The id is not a plain slug. See [`validate_schedule_id`] — this is the
    /// arbitrary-file-write refusal, and it maps to a 400, not a 500.
    InvalidJobId(String),
    JobNotFound(String),
    StorageError(io::Error),
    WorkflowLoadError(String),
    AgentSetupError(String),
    PersistError(String),
    CronParseError(String),
    SchedulerInternalError(String),
    AnyhowError(anyhow::Error),
}

impl std::fmt::Display for SchedulerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchedulerError::JobIdExists(id) => write!(f, "Job ID '{}' already exists.", id),
            SchedulerError::InvalidJobId(why) => write!(f, "Invalid job ID: {}", why),
            SchedulerError::JobNotFound(id) => write!(f, "Job ID '{}' not found.", id),
            SchedulerError::StorageError(e) => write!(f, "Storage error: {}", e),
            SchedulerError::WorkflowLoadError(e) => write!(f, "Workflow load error: {}", e),
            SchedulerError::AgentSetupError(e) => write!(f, "Agent setup error: {}", e),
            SchedulerError::PersistError(e) => write!(f, "Failed to persist schedules: {}", e),
            SchedulerError::CronParseError(e) => write!(f, "Invalid cron string: {}", e),
            SchedulerError::SchedulerInternalError(e) => {
                write!(f, "Scheduler internal error: {}", e)
            }
            SchedulerError::AnyhowError(e) => write!(f, "Scheduler operation failed: {}", e),
        }
    }
}

impl std::error::Error for SchedulerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SchedulerError::StorageError(e) => Some(e),
            SchedulerError::AnyhowError(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<io::Error> for SchedulerError {
    fn from(err: io::Error) -> Self {
        SchedulerError::StorageError(err)
    }
}

impl From<serde_json::Error> for SchedulerError {
    fn from(err: serde_json::Error) -> Self {
        SchedulerError::PersistError(err.to_string())
    }
}

impl From<anyhow::Error> for SchedulerError {
    fn from(err: anyhow::Error) -> Self {
        SchedulerError::AnyhowError(err)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, utoipa::ToSchema)]
pub struct ScheduledJob {
    pub id: String,
    pub source: String,
    pub cron: String,
    pub last_run: Option<DateTime<Utc>>,
    #[serde(default)]
    pub currently_running: bool,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub current_session_id: Option<String>,
    #[serde(default)]
    pub process_start_time: Option<DateTime<Utc>>,
    /// Number of times this job has fired. Used to enforce `max_runs`.
    #[serde(default)]
    pub run_count: u32,
    /// Optional cap on total firings. When `Some(n)`, the job auto-pauses once
    /// `run_count` reaches `n` — the bound that keeps `/loop` from running
    /// forever. `None` = unbounded (durable `/schedule` jobs).
    #[serde(default)]
    pub max_runs: Option<u32>,
    /// Issue #56 (§9.3 C2). The chat this schedule was created from, when there
    /// was one — `/loop` and `/schedule` always have one; a workflow scheduled
    /// from the CLI or the schedules route does not.
    ///
    /// A scheduled run resolves its provider from THIS session's recorded
    /// `provider_name` before falling back to the global default (see
    /// [`resolve_scheduled_provider`]). Without it a job created from a chat on
    /// a private model silently runs on the user's commercial default — R5 —
    /// and, once the design's §6.3 rule that a `Scheduled` session inherits its
    /// creator's classification lands, Gate A would refuse the bind on every
    /// tick forever.
    #[serde(default)]
    pub creator_session_id: Option<String>,
    /// The last run's failure, kept ON THE JOB so the schedules UI can show it.
    ///
    /// A cron tick that returns `Err` used to leave nothing behind but a log
    /// line, and a scheduled run mints a fresh session each time, so there was
    /// no surface anywhere that a repeating job had been failing since the day
    /// it was created. Cleared by the next successful run.
    #[serde(default)]
    pub last_error: Option<String>,
    /// Did the scheduler write [`Self::source`] itself, and may it therefore
    /// delete that file when the job goes?
    ///
    /// ⚠ Written by [`Scheduler::add_scheduled_job`] from its `make_copy`
    /// argument and by nothing else — a value supplied at a construction site is
    /// overwritten, so callers pass `None` and let the scheduler answer.
    ///
    /// ⚠ `Option`, not `bool`, and the third state is the whole point. A job
    /// persisted before this field existed deserialises as `None`, and the two
    /// creation paths that produced those rows are indistinguishable from the
    /// row alone: `POST /schedule/create` and `biorouter schedule add` copied the
    /// workflow and own the copy, while a schedule added from a workflow row
    /// points straight at the user's own file. `false` for both would strand
    /// every legacy copy; `true` for both would delete a user's workflow. `None`
    /// says "unrecorded", and [`scheduler_owns_source`] answers it from where the
    /// file actually lives.
    #[serde(default)]
    pub owns_source: Option<bool>,
}

/// May the scheduler delete `job.source` when the job is removed?
///
/// Deleting a schedule used to delete the workflow it was created from. The
/// "Add schedule" button on a workflow row schedules the user's file **in
/// place** ([`Scheduler::schedule_workflow`] passes `make_copy: false`), while
/// `DELETE /schedule/delete/{id}` asked for the workflow to be removed
/// unconditionally — so one click on Delete took the workflow with it, and the
/// row vanished from the workflows page too.
///
/// Two signals, asked in this order:
///
/// 1. **Containment is a floor, not a heuristic.** `make_copy` writes into
///    [`get_default_scheduled_workflows_dir`] and nowhere else, and the
///    destination is *derived* from that directory ([`planned_workflow_copy`]),
///    so a file the scheduler owns is always inside it. Asking first means no
///    answer this function can give ever deletes a file outside the scheduler's
///    own store — the property that has to hold when the record below is absent.
///    A path that fails this test costs at most an orphaned copy; the other
///    direction costs the user their workflow.
/// 2. **Inside the store, the record decides.** `None` — a row written before
///    the record existed — reads as owned, because `make_copy` is the only
///    writer of that directory. An explicit `Some(false)` is still honoured, so
///    a user who schedules a file that happens to live there keeps it.
fn scheduler_owns_source(job: &ScheduledJob) -> bool {
    let Ok(store) = get_default_scheduled_workflows_dir() else {
        return false;
    };
    if !Path::new(&job.source).starts_with(&store) {
        return false;
    }
    job.owns_source.unwrap_or(true)
}

/// Decide, under one lock, whether a fired cron job should actually run, and
/// stamp its start state if so. Returns the run snapshot when the caller should
/// execute. Its `last_run` is the last successful run, suitable as a cursor.
///
/// Skips when the job is gone, paused, still running from a previous firing
/// (overlap guard — a slow run never stacks), or has hit its `max_runs` cap
/// (which also auto-pauses it). On a real run it marks the job running and bumps
/// `run_count`; completion advances `last_run` only after success.
///
/// ⚠ `running_tasks` is taken here, **inside the `jobs` lock**, and that is the
/// whole reason it is a parameter rather than the caller's business (issue
/// #148A). `currently_running` and the cancel token are two halves of one fact
/// published under two independent mutexes: a caller that registered the token
/// after releasing `jobs` left a window in which another task could see
/// `currently_running == true`, reach for the token, and find nothing — a Stop
/// that silently cancelled nothing. Registering costs no `.await`
/// ([`register_running_task`] takes a `std::sync::Mutex`), so there is no reason
/// for the window to exist.
async fn claim_run_slot(
    jobs: &Arc<Mutex<JobsMap>>,
    running_tasks: &Arc<StdMutex<RunningTasksMap>>,
    job_id: &str,
    token: &CancellationToken,
    now: DateTime<Utc>,
) -> RunSlot {
    let mut jobs_guard = jobs.lock().await;
    match jobs_guard.get_mut(job_id) {
        None => RunSlot::Skipped,
        Some((_, job)) if job.paused => RunSlot::Skipped,
        Some((_, job)) if job.currently_running => {
            tracing::info!("Skipping job '{}': previous run still in progress", job_id);
            RunSlot::Skipped
        }
        Some((_, job)) if job.max_runs.is_some_and(|max| job.run_count >= max) => {
            tracing::info!(
                "Job '{}' reached its run cap ({:?}); auto-pausing",
                job_id,
                job.max_runs
            );
            job.paused = true;
            RunSlot::Capped
        }
        // Resource-aware deferral (jcode "ambient" idea): skip this firing (the
        // cron fires again next interval) when the provider is rate-limited or a
        // user is mid-conversation, so background work never competes with the
        // user. Does NOT bump run_count, so the job isn't consumed.
        Some(_) if crate::providers::retry::is_rate_limited() => {
            tracing::info!(
                "Deferring scheduled job '{}': provider rate-limited, backing off",
                job_id
            );
            RunSlot::Skipped
        }
        Some(_) if pause_on_active() && interactive_active() => {
            tracing::info!("Deferring scheduled job '{}': user session active", job_id);
            RunSlot::Skipped
        }
        Some((_, job)) => {
            job.currently_running = true;
            job.process_start_time = Some(now);
            job.run_count = job.run_count.saturating_add(1);
            let claimed = job.clone();
            register_running_task(running_tasks, job_id, token.clone());
            RunSlot::Claimed(Box::new(claimed))
        }
    }
}

/// What [`claim_run_slot`] decided, and — crucially — what the caller therefore
/// owes the schedule file.
///
/// It used to be an `Option`, and the `None` arm cost a whole-file rewrite on
/// every deferred tick for no change at all. Naming the auto-pause separately is
/// what lets a skip write nothing (issue #140: every needless write is another
/// chance to clobber another process's job).
enum RunSlot {
    /// Run it. Carries the snapshot to execute.
    Claimed(Box<ScheduledJob>),
    /// Do not run: the job hit `max_runs` and was auto-paused. That pause must
    /// reach disk or it does not survive a restart.
    Capped,
    /// Do not run, and nothing changed.
    Skipped,
}

// ---------------------------------------------------------------------------
// Issue #140 — the schedule file is shared, so nobody may rewrite all of it.
//
// `biorouter schedule …` builds its OWN `Scheduler` over the same
// `schedule.json` the daemon already has open, and the daemon's map is loaded
// once, at construction. The old `persist_jobs` serialised that whole map over
// the file, so the last writer silently deleted every job the other one had
// added since — measured: a CLI-created job vanished the moment the GUI's Pause
// button touched an unrelated schedule.
//
// The fix is that a mutation now READS the current file, applies only the change
// it is responsible for, and writes the result back — all under an exclusive
// cross-process lock, and published with a rename so a reader never sees a torn
// file. Two consequences worth stating, because they are the reason this is not
// simply "merge the map in":
//
//  * a mutation publishes FIELDS, not a job. A stale in-memory copy of a job can
//    no longer overwrite another process's `last_run` just because someone
//    pressed Pause.
//  * every field edit is *modify-if-present* ([`edit_job`]). A job another
//    process deleted stays deleted; only [`Scheduler::add_scheduled_job`] ever
//    inserts.
// ---------------------------------------------------------------------------

/// An exclusive, cross-process lock on the schedule file, held for one whole
/// read-modify-write. Same shape as `knowledge::soul`'s reconcile lock.
struct ScheduleFileLock(fs::File);

impl ScheduleFileLock {
    fn acquire(storage_path: &Path) -> Result<Self, io::Error> {
        if let Some(parent) = storage_path.parent() {
            fs::create_dir_all(parent)?;
        }
        // A sidecar rather than the schedule file itself: the write below
        // replaces `schedule.json` by rename, which would drop a lock held on
        // the old inode while another process still believed it held one.
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(schedule_lock_path(storage_path))?;
        file.lock_exclusive()?;
        Ok(Self(file))
    }
}

impl Drop for ScheduleFileLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn schedule_lock_path(storage_path: &Path) -> PathBuf {
    let mut name = storage_path
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_else(|| std::ffi::OsString::from("schedule.json"));
    name.push(".lock");
    storage_path.with_file_name(name)
}

/// The schedule file's current contents.
///
/// A missing or empty file is "no jobs yet". **Everything else that cannot be
/// turned into a job list is an error** — an unreadable file and an unparseable
/// one alike — because of what every caller does next: it applies its own change
/// to whatever comes back and writes the result. Handing back an empty list on a
/// parse failure therefore does not "recover", it *publishes* the emptiness:
/// nothing but [`Scheduler::add_scheduled_job`] ever inserts, so the jobs that
/// failed to parse are gone from the file the moment anything else pauses,
/// finishes or starts a run.
///
/// A copy of the unparseable file is kept aside for the operator to repair from,
/// but that copy is a courtesy — the refusal to continue is what protects the
/// data, and the refusal happens whether or not the copy succeeded.
fn read_jobs_file(storage_path: &Path) -> Result<Vec<ScheduledJob>, io::Error> {
    let data = match fs::read_to_string(storage_path) {
        Ok(data) => data,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    if data.trim().is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str(&data) {
        Ok(list) => Ok(list),
        Err(e) => {
            // ⚠ A FIXED name, created only if absent. Every reader comes through
            // here — including the reconcile at the top of every cron tick — so
            // a timestamped copy would leave one `schedule.corrupt-<ms>` file per
            // tick beside a file nobody has repaired yet. Not overwriting also
            // means the FIRST corruption is the one kept, which is the one that
            // still has the jobs in it.
            let backup = storage_path.with_extension("corrupt");
            let kept = if backup.exists() {
                format!(
                    "An earlier copy is already at {} and was left alone.",
                    backup.display()
                )
            } else {
                match fs::copy(storage_path, &backup) {
                    Ok(_) => format!("A copy was kept at {}.", backup.display()),
                    Err(copy_error) => format!(
                        "A copy could NOT be kept ({copy_error}), so the file itself is the only \
                         copy — it has been left exactly as it is."
                    ),
                }
            };
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{} is not a valid schedule list ({e}). Refusing to continue: the next write \
                     would replace it with an empty one. {kept} Repair or delete the file to \
                     resume scheduling.",
                    storage_path.display(),
                ),
            ))
        }
    }
}

/// Publish `list` by rename, so a concurrent reader sees either the old file or
/// the new one and never a half-written one.
fn write_jobs_file(storage_path: &Path, list: &[ScheduledJob]) -> Result<(), SchedulerError> {
    let data = serde_json::to_string_pretty(list)?;
    let mut tmp_name = storage_path.as_os_str().to_os_string();
    tmp_name.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp_name);
    fs::write(&tmp, data)?;
    fs::rename(&tmp, storage_path)?;
    Ok(())
}

/// Read the schedule file under the same exclusive lock a write takes, on the
/// blocking pool.
///
/// The lock is not decoration: without it a reader can land between
/// [`write_jobs_file`]'s `fs::write` to the temp file and its `fs::rename`, and
/// — more to the point — a reader that skipped the lock would be a second reader
/// of a shared file with its own policy, which is how the two halves of this
/// module drifted apart in the first place ([`Scheduler::load_jobs_from_storage`]
/// used to open the file itself and treat an unparseable one as "no jobs").
async fn read_jobs(storage_path: &Path) -> Result<Vec<ScheduledJob>, SchedulerError> {
    let storage_path = storage_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let _lock = ScheduleFileLock::acquire(&storage_path)?;
        read_jobs_file(&storage_path).map_err(SchedulerError::from)
    })
    .await
    .map_err(|e| SchedulerError::PersistError(e.to_string()))?
}

/// Apply `change` to the on-disk schedule list under the exclusive lock, and
/// hand back whatever `change` returns.
///
/// The whole read-modify-write runs on the blocking pool: `fs2`'s lock blocks,
/// and holding it across an `.await` on the scheduler's runtime is how a cron
/// callback would deadlock with a route handler.
///
/// `change` returns a `Result`, and an `Err` **aborts the write** — the file is
/// left exactly as it was found. That is what lets a caller decide *under the
/// lock* that its change must not happen at all (see
/// [`Scheduler::add_scheduled_job`], whose duplicate-id check has to see the same
/// list the insert would go into, not this process's memory).
async fn persist_change<F, T>(storage_path: &Path, change: F) -> Result<T, SchedulerError>
where
    F: FnOnce(&mut Vec<ScheduledJob>) -> Result<T, SchedulerError> + Send + 'static,
    T: Send + 'static,
{
    let storage_path = storage_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let _lock = ScheduleFileLock::acquire(&storage_path)?;
        let mut list = read_jobs_file(&storage_path)?;
        let value = change(&mut list)?;
        write_jobs_file(&storage_path, &list)?;
        Ok(value)
    })
    .await
    .map_err(|e| SchedulerError::PersistError(e.to_string()))?
}

/// Edit the on-disk row for `id` **if it is still there**, reporting whether it
/// was.
///
/// The no-op arm is the point: this process's copy of a job is not evidence that
/// the job still exists. Re-inserting it would turn "the CLI deleted a job" into
/// "the daemon undid the delete" — the same silent divergence as #140, mirrored.
///
/// ⚠ The returned `bool` is the other half of that fix, and it is load-bearing.
/// Not resurrecting the row is only half a convergence: a daemon that quietly
/// writes nothing still lists the job, still holds its cron entry, and still
/// fires it. `false` means *the file no longer has this job*, and every caller
/// that can act on that must — see [`Scheduler::forget_job`].
#[must_use]
fn edit_job(list: &mut [ScheduledJob], id: &str, edit: impl FnOnce(&mut ScheduledJob)) -> bool {
    match list.iter_mut().find(|job| job.id == id) {
        Some(job) => {
            edit(job);
            true
        }
        None => false,
    }
}

/// Mark a job as started. Only the run-state fields are published.
///
/// Returns the job's authoritative run count *after* this run was counted, or
/// `None` when the file no longer has the job at all.
async fn persist_run_start(
    storage_path: &Path,
    job: &ScheduledJob,
    counts_toward_cap: bool,
) -> Result<Option<u32>, SchedulerError> {
    let id = job.id.clone();
    let started = job.process_start_time;
    persist_change(storage_path, move |list| {
        let mut count = None;
        let found = edit_job(list, &id, |stored| {
            stored.currently_running = true;
            stored.process_start_time = started;
            // Read-modify-write against the FILE, not `max` against this
            // process's copy. `run_count` is a monotonic counter of firings and
            // this process's copy is loaded once, so two processes running the
            // same `/loop` both wrote `max(disk, stale)` and lost every
            // increment the other made — a bounded loop that never reaches its
            // cap. `run_now` passes `counts_toward_cap = false`, because
            // pressing "Run now" has never consumed a `/loop`'s budget.
            if counts_toward_cap {
                stored.run_count = stored.run_count.saturating_add(1);
            }
            count = Some(stored.run_count);
        });
        Ok(if found { count } else { None })
    })
    .await
}

/// Persist the auto-pause `claim_run_slot` applied when a job hit `max_runs`.
///
/// Returns `false` when the file no longer has the job.
async fn persist_auto_pause(storage_path: &Path, job_id: &str) -> Result<bool, SchedulerError> {
    let id = job_id.to_string();
    persist_change(storage_path, move |list| {
        Ok(edit_job(list, &id, |stored| stored.paused = true))
    })
    .await
}

/// What a finished run writes back onto its job.
///
/// Extracted from the two completion sites (the cron callback and `run_now`) so
/// that the rule issue #148B is about — **`last_run` advances only on a real
/// success** — is one testable statement instead of two copies of an `if`.
/// `last_run` is a data cursor for the Meditation job, so advancing it past a
/// run that was killed or refused silently skips an unprocessed window.
#[derive(Clone, Debug)]
struct RunCompletion {
    finished_at: DateTime<Utc>,
    error: Option<String>,
}

impl RunCompletion {
    fn from_result(result: &Result<String>, finished_at: DateTime<Utc>) -> Self {
        Self {
            finished_at,
            error: result.as_ref().err().map(|e| format!("{e:#}")),
        }
    }

    fn apply(&self, job: &mut ScheduledJob) {
        job.currently_running = false;
        job.current_session_id = None;
        job.process_start_time = None;
        // Issue #56 (§9.3 C2). A failing tick leaves a job-level error the
        // schedules UI can show, instead of only a log line nobody reads — a
        // scheduled run mints a new session each time, so the failure has no
        // other durable home. Cleared by the next run that succeeds.
        job.last_error.clone_from(&self.error);
        if self.error.is_none() {
            job.last_run = Some(self.finished_at);
        }
    }
}

/// Record a finished run, in memory and on disk. Never fails the caller: the run
/// is already over, and a persist error must not also lose the in-memory clear.
///
/// Returns `false` when the file no longer has this job — it was deleted while
/// the run was in flight — so the caller can drop it from this process too.
async fn record_run_completion(
    storage_path: &Path,
    jobs: &Arc<Mutex<JobsMap>>,
    job_id: &str,
    result: &Result<String>,
) -> bool {
    let completion = RunCompletion::from_result(result, Utc::now());
    {
        let mut jobs_guard = jobs.lock().await;
        if let Some((_, job)) = jobs_guard.get_mut(job_id) {
            completion.apply(job);
        }
    }
    let id = job_id.to_string();
    let for_disk = completion.clone();
    match persist_change(storage_path, move |list| {
        Ok(edit_job(list, &id, |stored| for_disk.apply(stored)))
    })
    .await
    {
        Ok(found) => found,
        Err(e) => {
            tracing::error!("Failed to persist job completion for '{}': {}", job_id, e);
            // A failed write is not evidence the job is gone; keep it.
            true
        }
    }
}

/// The cancel-token registry is a plain `std::sync::Mutex` on purpose.
///
/// Issue #148A: `run_now` must get from "this job is now marked running" to
/// "a task owns clearing that mark" without a single `.await` in between, or a
/// dropped handler future strands the flag. A `tokio::sync::Mutex` lock is an
/// await point; these critical sections are a `HashMap` insert and a remove, so
/// a blocking lock costs nothing and closes the window.
fn register_running_task(
    tasks: &Arc<StdMutex<RunningTasksMap>>,
    job_id: &str,
    token: CancellationToken,
) {
    tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(job_id.to_string(), token);
}

fn unregister_running_task(tasks: &Arc<StdMutex<RunningTasksMap>>, job_id: &str) {
    tasks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(job_id);
}

// ---------------------------------------------------------------------------
// Issue #140, the direction the modify-if-present fix does NOT cover on its own.
//
// `edit_job` refusing to resurrect a deleted row stops the daemon corrupting the
// file. It does nothing about the daemon's own copy. The CLI is entirely offline
// — every `biorouter schedule …` subcommand builds its own `Scheduler` over
// `<data>/schedule.json` and never contacts `biorouterd` — while the daemon's map
// is filled exactly once, at construction. So after `biorouter schedule remove`:
// the row is off disk, and the running daemon still lists the job from
// `/schedule/list`, still holds its cron entry, and still FIRES it, burning
// tokens on a schedule the user deleted, until the daemon is restarted.
//
// Before the modify-if-present fix that divergence was at least self-limiting —
// the daemon's whole-map write put the row back, wrongly but visibly. Now it is
// permanent and silent, which is worse. The file is the shared source of truth,
// so the fix is the other half of the same rule: **the daemon converges on the
// file.** A job the file no longer has is dropped from this process's map and
// from the cron scheduler, at the two moments it matters — before a tick decides
// to run something, and whenever the job list is served.
// ---------------------------------------------------------------------------

/// Drop `job_id` from this process's map and cancel its cron entry.
///
/// The map removal is what actually stops the job: [`claim_run_slot`]'s `None`
/// arm skips a job it cannot find. Removing the `tokio_cron_scheduler` entry as
/// well is not tidiness — a stale entry keyed on the same job id would fire
/// alongside a later job re-created under that id, and the two would double-run.
async fn forget_job(jobs: &Arc<Mutex<JobsMap>>, tokio_scheduler: &TokioJobScheduler, job_id: &str) {
    let uuid = jobs.lock().await.remove(job_id).map(|(uuid, _)| uuid);
    if let Some(uuid) = uuid {
        if let Err(e) = tokio_scheduler.remove(&uuid).await {
            tracing::warn!(
                "Dropped job '{}' from memory but could not remove its cron entry: {}",
                job_id,
                e
            );
        }
    }
}

/// One firing of a cron job: reconcile against the file, claim a run slot, run
/// it, record what happened.
///
/// A free function rather than the body of the closure in
/// [`Scheduler::create_cron_task`] because it is the whole lifecycle of a
/// scheduled run and reads as one sequence; the closure keeps only the argument
/// clones that a `move` closure has to make per firing.
async fn run_cron_tick(
    task_job_id: String,
    current_jobs_arc: Arc<Mutex<JobsMap>>,
    running_tasks: Arc<StdMutex<RunningTasksMap>>,
    local_storage_path: PathBuf,
    cron_handle: TokioJobScheduler,
) {
    // The file is the shared source of truth, and the CLI writes it from another
    // process. Converge BEFORE claiming a slot: a job the user deleted must not
    // spend a single token, and the tick that discovers the deletion is the one
    // that retires the cron entry. A read failure leaves everything alone.
    if let Err(e) = converge_removals(&local_storage_path, &current_jobs_arc, &cron_handle).await {
        tracing::warn!(
            "Could not reconcile {} before running '{}': {}",
            local_storage_path.display(),
            task_job_id,
            e
        );
    }

    let cancel_token = CancellationToken::new();
    let job_to_execute = match claim_run_slot(
        &current_jobs_arc,
        &running_tasks,
        &task_job_id,
        &cancel_token,
        Utc::now(),
    )
    .await
    {
        // A deferred tick changed nothing, so it writes nothing.
        RunSlot::Skipped => return,
        RunSlot::Capped => {
            match persist_auto_pause(&local_storage_path, &task_job_id).await {
                Ok(true) => {}
                Ok(false) => forget_job(&current_jobs_arc, &cron_handle, &task_job_id).await,
                Err(e) => {
                    tracing::error!("Failed to persist auto-pause for '{}': {}", task_job_id, e);
                }
            }
            return;
        }
        RunSlot::Claimed(job) => *job,
    };

    match persist_run_start(&local_storage_path, &job_to_execute, true).await {
        // The file's count is authoritative — another process may have fired
        // this same `/loop` since our map was loaded — so adopt it, or
        // `claim_run_slot`'s cap check keeps testing a number that is too low to
        // ever reach `max_runs`.
        Ok(Some(count)) => {
            if let Some((_, job)) = current_jobs_arc.lock().await.get_mut(&task_job_id) {
                job.run_count = job.run_count.max(count);
            }
        }
        // Deleted between the reconcile above and now. The run has not started
        // yet, so abandon it rather than burn a turn.
        Ok(None) => {
            unregister_running_task(&running_tasks, &task_job_id);
            forget_job(&current_jobs_arc, &cron_handle, &task_job_id).await;
            return;
        }
        Err(e) => tracing::error!("Failed to persist job status: {}", e),
    }

    let result = execute_job(
        job_to_execute,
        current_jobs_arc.clone(),
        task_job_id.clone(),
        cancel_token.clone(),
    )
    .await;

    // Completion BEFORE unregistering: the two together are what "this job is
    // running" means, and clearing the flag first is what keeps
    // `kill_running_job` from seeing a running job with no token to cancel.
    let still_on_disk = record_run_completion(
        &local_storage_path,
        &current_jobs_arc,
        &task_job_id,
        &result,
    )
    .await;
    unregister_running_task(&running_tasks, &task_job_id);
    if !still_on_disk {
        forget_job(&current_jobs_arc, &cron_handle, &task_job_id).await;
    }

    match result {
        Ok(_) => tracing::info!("Job '{}' completed", task_job_id),
        Err(ref e) => tracing::error!("Job '{}' failed: {}", task_job_id, e),
    }
}

/// Drop every in-memory job the schedule file no longer has.
///
/// An error reading the file propagates and **nothing is dropped**: a read that
/// failed is not evidence that a job was deleted, and the cost of the two
/// mistakes is not symmetric — forgetting a job the user still has means their
/// schedule silently stops.
async fn converge_removals(
    storage_path: &Path,
    jobs: &Arc<Mutex<JobsMap>>,
    tokio_scheduler: &TokioJobScheduler,
) -> Result<(), SchedulerError> {
    let on_disk: std::collections::HashSet<String> = read_jobs(storage_path)
        .await?
        .into_iter()
        .map(|job| job.id)
        .collect();

    let vanished: Vec<String> = {
        let jobs_guard = jobs.lock().await;
        jobs_guard
            .keys()
            .filter(|id| !on_disk.contains(*id))
            .cloned()
            .collect()
    };

    for id in vanished {
        tracing::info!(
            "Schedule '{}' is no longer in {}; dropping it from this process",
            id,
            storage_path.display()
        );
        forget_job(jobs, tokio_scheduler, &id).await;
    }
    Ok(())
}

pub struct Scheduler {
    tokio_scheduler: TokioJobScheduler,
    jobs: Arc<Mutex<JobsMap>>,
    storage_path: PathBuf,
    running_tasks: Arc<StdMutex<RunningTasksMap>>,
    session_manager: Arc<SessionManager>,
}

impl Scheduler {
    pub async fn new(
        storage_path: PathBuf,
        session_manager: Arc<SessionManager>,
    ) -> Result<Arc<Self>, SchedulerError> {
        let internal_scheduler = TokioJobScheduler::new()
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        let jobs = Arc::new(Mutex::new(HashMap::new()));
        let running_tasks = Arc::new(StdMutex::new(HashMap::new()));

        let arc_self = Arc::new(Self {
            tokio_scheduler: internal_scheduler,
            jobs,
            storage_path,
            running_tasks,
            session_manager,
        });

        arc_self.load_jobs_from_storage().await;
        arc_self
            .tokio_scheduler
            .start()
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        Ok(arc_self)
    }

    fn create_cron_task(&self, job: ScheduledJob) -> Result<Job, SchedulerError> {
        let job_for_task = job.clone();
        let jobs_arc = self.jobs.clone();
        let storage_path = self.storage_path.clone();
        let running_tasks_arc = self.running_tasks.clone();
        // `JobsSchedulerLocked` is a handle, and cloning it clones the handle —
        // the tick needs one so it can retire its own cron entry when the file
        // says the job is gone (see `converge_removals`).
        let cron_handle = self.tokio_scheduler.clone();

        let cron_parts: Vec<&str> = job.cron.split_whitespace().collect();
        let cron = match cron_parts.len() {
            5 => {
                tracing::warn!(
                    "Job '{}' has legacy 5-field cron '{}', converting to 6-field",
                    job.id,
                    job.cron
                );
                format!("0 {}", job.cron)
            }
            6 => job.cron.clone(),
            _ => {
                return Err(SchedulerError::CronParseError(format!(
                    "Invalid cron expression '{}': expected 5 or 6 fields, got {}",
                    job.cron,
                    cron_parts.len()
                )))
            }
        };

        let local_tz = Local::now().timezone();

        Job::new_async_tz(&cron, local_tz, move |_uuid, _l| {
            tracing::info!("Cron task triggered for job '{}'", job_for_task.id);
            Box::pin(run_cron_tick(
                job_for_task.id.clone(),
                jobs_arc.clone(),
                running_tasks_arc.clone(),
                storage_path.clone(),
                cron_handle.clone(),
            ))
        })
        .map_err(|e| SchedulerError::CronParseError(e.to_string()))
    }

    pub async fn add_scheduled_job(
        &self,
        original_job_spec: ScheduledJob,
        make_copy: bool,
    ) -> Result<(), SchedulerError> {
        // ⚠ FIRST, ahead of every other check and long before anything derives a
        // path from it. The id names a file (see [`validate_schedule_id`]), and
        // the copy below used to run before the cron was parsed and before the
        // duplicate guard, so `{"id": "/tmp/pwned"}` landed `/tmp/pwned.yaml`
        // even on the request paths that answered 400 or 409.
        validate_schedule_id(&original_job_spec.id)?;
        {
            let jobs_guard = self.jobs.lock().await;
            if jobs_guard.contains_key(&original_job_spec.id) {
                return Err(SchedulerError::JobIdExists(original_job_spec.id.clone()));
            }
        }

        let mut stored_job = original_job_spec;
        let job_id = stored_job.id.clone();
        let original_source = stored_job.source.clone();

        // The copy is PLANNED here and PERFORMED at the bottom, once every
        // refusal has had its say.
        //
        // ⚠ The order is the fix, not a tidy-up. `fs::copy` truncates its
        // destination, and the destination is `<id>.<ext>` — so copying before
        // the duplicate guard meant a rejected `add` that merely reused an
        // existing id had already overwritten *that job's* workflow file, and
        // then answered 409 as though nothing had happened.
        let planned_copy = if make_copy {
            Some(planned_workflow_copy(&stored_job)?)
        } else {
            None
        };
        // The scheduler is the authority on ownership, not the caller: whether
        // `source` is a file we wrote is decided right here, by `make_copy`, and
        // whatever the construction site put in the field is overwritten. See
        // [`scheduler_owns_source`] for what the answer is then used for.
        stored_job.owns_source = Some(planned_copy.is_some());
        if let Some(destination) = planned_copy.as_ref() {
            stored_job.source = destination.to_string_lossy().into_owned();
            stored_job.current_session_id = None;
            stored_job.process_start_time = None;
        }

        // Built before the write so an invalid cron is refused without touching
        // the file, and so the write below is the last fallible step that can
        // leave the two out of step.
        //
        // ⚠ It is built from the job as it will be STORED, which is why the
        // source rewrite above happens first — but note that the task itself
        // only ever reads the id and the cron string (`run_cron_tick` re-reads
        // the row), so this cannot bind a stale path.
        let cron_task = self.create_cron_task(stored_job.clone())?;

        // The ONE mutation that inserts. Everything else edits in place, so that
        // a job another process removed is never resurrected (issue #140).
        //
        // ⚠ The duplicate-id check is HERE, inside the lock, and not only in the
        // memory check at the top of this function. The memory check asks "does
        // *this process* know that id", which a daemon that has never seen a
        // CLI-created job answers `false` to — and the old `retain(…); push(…)`
        // then force-replaced the stranger's row, taking its cron, its cursor
        // and (via `make_copy`) its workflow file with it. An `Err` from the
        // closure leaves the file exactly as it was.
        //
        // ⚠ The write also precedes the map insert on purpose: a tick that fires
        // in the gap reconciles against a file that already has the job, whereas
        // the other order would have the reconcile delete a job that was mid-add.
        let spec_for_disk = stored_job.clone();
        let id_for_disk = job_id.clone();
        persist_change(&self.storage_path, move |list| {
            if list.iter().any(|job| job.id == id_for_disk) {
                return Err(SchedulerError::JobIdExists(id_for_disk));
            }
            list.push(spec_for_disk);
            Ok(())
        })
        .await?;

        // The row is ours now — the id was a slug, the cron parsed, and no other
        // process holds this id — so the copy can finally happen. If it fails
        // the row is taken back out, because a job whose workflow file does not
        // exist would fire forever and fail every time.
        if let Some(destination) = planned_copy {
            if let Err(error) = fs::copy(Path::new(&original_source), &destination) {
                let id_to_undo = job_id.clone();
                let _ = persist_change(&self.storage_path, move |list| {
                    list.retain(|job| job.id != id_to_undo);
                    Ok(())
                })
                .await;
                return Err(SchedulerError::StorageError(error));
            }
        }

        let job_uuid = self
            .tokio_scheduler
            .add(cron_task)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        {
            let mut jobs_guard = self.jobs.lock().await;
            jobs_guard.insert(job_id, (job_uuid, stored_job));
        }
        Ok(())
    }

    pub async fn schedule_workflow(
        &self,
        workflow_path: PathBuf,
        cron_schedule: Option<String>,
    ) -> Result<(), SchedulerError> {
        let workflow_path_str = workflow_path.to_string_lossy().to_string();

        let existing_job_id = {
            let jobs_guard = self.jobs.lock().await;
            jobs_guard
                .iter()
                .find(|(_, (_, job))| job.source == workflow_path_str)
                .map(|(id, _)| id.clone())
        };

        match cron_schedule {
            Some(cron) => {
                if let Some(job_id) = existing_job_id {
                    self.update_schedule(&job_id, cron).await
                } else {
                    let job_id = self.generate_unique_job_id(&workflow_path).await;
                    let job = ScheduledJob {
                        id: job_id,
                        source: workflow_path_str,
                        cron,
                        last_run: None,
                        currently_running: false,
                        paused: false,
                        current_session_id: None,
                        process_start_time: None,
                        run_count: 0,
                        max_runs: None,
                        // A workflow scheduled by path has no creating chat.
                        creator_session_id: None,
                        last_error: None,
                        owns_source: None,
                    };
                    self.add_scheduled_job(job, false).await
                }
            }
            None => {
                if let Some(job_id) = existing_job_id {
                    self.remove_scheduled_job(&job_id, false).await
                } else {
                    Ok(())
                }
            }
        }
    }

    async fn generate_unique_job_id(&self, path: &Path) -> String {
        let base_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed")
            .to_string();

        let jobs_guard = self.jobs.lock().await;
        let mut id = base_id.clone();
        let mut counter = 1;

        while jobs_guard.contains_key(&id) {
            id = format!("{}_{}", base_id, counter);
            counter += 1;
        }

        id
    }

    /// Fill the in-memory map from the schedule file. The ONLY call site is
    /// [`Scheduler::new`] — there is no re-read, no mtime check and no watcher,
    /// which is why [`converge_removals`] exists.
    ///
    /// ⚠ It goes through [`read_jobs`] rather than opening the file itself. It
    /// used to be a second reader with its own policy — no [`ScheduleFileLock`],
    /// and an unparseable file silently became "no jobs" without so much as a
    /// copy aside — and it is the reader most likely to *meet* a corrupt file,
    /// since it runs before anything else in the process has touched it.
    async fn load_jobs_from_storage(self: &Arc<Self>) {
        let list = match read_jobs(&self.storage_path).await {
            Ok(list) => list,
            Err(e) => {
                tracing::error!(
                    "Failed to read {}: {}. Starting with an empty schedule list; the file has \
                     NOT been modified.",
                    self.storage_path.display(),
                    e
                );
                return;
            }
        };

        for mut job_to_load in list {
            // BR-38: a scheduled run lives only in memory — no process or turn
            // survives a daemon restart. A job persisted as `currently_running`
            // was mid-run when we crashed, so its run is already gone. Reconcile
            // the stale flag on load; otherwise the overlap guard in
            // `claim_run_slot` would treat the ghost run as still in progress and
            // skip this job forever.
            if job_to_load.currently_running
                || job_to_load.current_session_id.is_some()
                || job_to_load.process_start_time.is_some()
            {
                tracing::warn!(
                    "Resetting stale running state for scheduled job '{}' on load (session {:?} did not survive restart)",
                    job_to_load.id,
                    job_to_load.current_session_id
                );
                job_to_load.currently_running = false;
                job_to_load.current_session_id = None;
                job_to_load.process_start_time = None;
            }

            if !Path::new(&job_to_load.source).exists() {
                tracing::warn!(
                    "Workflow file {} not found, skipping job '{}'",
                    job_to_load.source,
                    job_to_load.id
                );
                continue;
            }

            let cron_task = match self.create_cron_task(job_to_load.clone()) {
                Ok(task) => task,
                Err(e) => {
                    tracing::error!(
                        "Failed to create cron task for job '{}': {}. Skipping.",
                        job_to_load.id,
                        e
                    );
                    continue;
                }
            };

            let job_uuid = match self.tokio_scheduler.add(cron_task).await {
                Ok(uuid) => uuid,
                Err(e) => {
                    tracing::error!(
                        "Failed to add job '{}' to scheduler: {}. Skipping.",
                        job_to_load.id,
                        e
                    );
                    continue;
                }
            };

            let mut jobs_guard = self.jobs.lock().await;
            jobs_guard.insert(job_to_load.id.clone(), (job_uuid, job_to_load));
        }
    }

    /// Every schedule this process knows about, after reconciling against the
    /// file.
    ///
    /// The reconcile is why this is not a bare map read. `/schedule/list` and
    /// `biorouter schedule list` are the surfaces a user checks after deleting a
    /// schedule somewhere else, and a daemon whose map was loaded once at
    /// construction would keep listing it — see [`converge_removals`]. A read
    /// failure is logged and the map served as-is, because failing to read the
    /// file is not evidence that anything was deleted.
    pub async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob> {
        if let Err(e) =
            converge_removals(&self.storage_path, &self.jobs, &self.tokio_scheduler).await
        {
            tracing::warn!(
                "Could not reconcile {} while listing schedules: {}",
                self.storage_path.display(),
                e
            );
        }
        self.jobs
            .lock()
            .await
            .values()
            .map(|(_, j)| j.clone())
            .collect()
    }

    /// Remove a job, and its workflow file **only if the scheduler wrote that
    /// file**.
    ///
    /// ⚠ `remove_owned_workflow` is a request, not a permission.
    /// [`scheduler_owns_source`] has the last word, so a `true` from a caller
    /// that cannot tell a copy from a pointer — which is every HTTP and CLI
    /// delete, none of which reads the job first — can no longer take the user's
    /// own workflow with it.
    pub async fn remove_scheduled_job(
        &self,
        id: &str,
        remove_owned_workflow: bool,
    ) -> Result<(), SchedulerError> {
        let (job_uuid, workflow_to_delete) = {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.remove(id) {
                // Decided from the row while we still hold it, so the two halves
                // of the answer — the path and the right to delete it — can
                // never come from different jobs.
                Some((uuid, job)) => {
                    let path = (remove_owned_workflow && scheduler_owns_source(&job))
                        .then(|| job.source.clone());
                    (uuid, path)
                }
                None => return Err(SchedulerError::JobNotFound(id.to_string())),
            }
        };

        self.tokio_scheduler
            .remove(&job_uuid)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        if let Some(workflow_path) = workflow_to_delete {
            let path = Path::new(&workflow_path);
            if path.exists() {
                fs::remove_file(path)?;
            }
        }

        let removed_id = id.to_string();
        persist_change(&self.storage_path, move |list| {
            list.retain(|job| job.id != removed_id);
            Ok(())
        })
        .await?;
        Ok(())
    }

    pub async fn sessions(
        &self,
        sched_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, Session)>, SchedulerError> {
        let all_sessions = self
            .session_manager
            .list_sessions()
            .await
            .map_err(|e| SchedulerError::StorageError(io::Error::other(e)))?;

        let mut schedule_sessions: Vec<(String, Session)> = all_sessions
            .into_iter()
            .filter(|s| s.schedule_id.as_deref() == Some(sched_id))
            .map(|s| (s.id.clone(), s))
            .collect();

        schedule_sessions.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at));
        schedule_sessions.truncate(limit);

        Ok(schedule_sessions)
    }

    /// Run a schedule immediately.
    ///
    /// ## Issue #148A — the run must outlive its caller
    ///
    /// This is reached from an axum handler, and **axum drops a handler's future
    /// when the client disconnects.** The old body marked the job
    /// `currently_running`, persisted that, and then awaited `execute_job`
    /// *inline*; every line that cleared the flag came after that await. A
    /// browser tab closed mid-run — or any dropped request — therefore left the
    /// flag set forever, and a job with `currently_running == true` is skipped
    /// by [`claim_run_slot`], refused by `pause_schedule` and refused by
    /// `update_schedule`. One dropped request bricked the schedule until the
    /// daemon restarted (BR-38's load-time reconcile was the only escape).
    ///
    /// So the run is handed to a **detached task** that owns the whole
    /// lifecycle, and this function only awaits its handle. Dropping a
    /// `JoinHandle` does not cancel the task, so the completion path — clearing
    /// the flag, recording `last_run`/`last_error`, persisting — always runs.
    ///
    /// ⚠ Between the claim below and `tokio::spawn` there is deliberately **no
    /// `.await`**. Re-introducing one (a `tokio::sync::Mutex` on the cancel-token
    /// registry, an eager persist) re-opens exactly the window this fixes, and
    /// nothing in the type system will say so.
    pub async fn run_now(&self, sched_id: &str) -> Result<String, SchedulerError> {
        let cancel_token = CancellationToken::new();
        let job_to_run = {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((_, job)) => {
                    if job.currently_running {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Job '{}' is already running",
                            sched_id
                        )));
                    }
                    job.currently_running = true;
                    job.process_start_time = Some(Utc::now());
                    // Under the SAME lock that published `currently_running`.
                    // See `claim_run_slot`: the flag and the token are two halves
                    // of one fact, and registering after releasing `jobs` left a
                    // window in which `kill_running_job` could see a running job
                    // and find no token to cancel.
                    register_running_task(&self.running_tasks, sched_id, cancel_token.clone());
                    job.clone()
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        };

        let jobs = self.jobs.clone();
        let running_tasks = self.running_tasks.clone();
        let storage_path = self.storage_path.clone();
        let cron_handle = self.tokio_scheduler.clone();
        let job_id = sched_id.to_string();

        let handle = tokio::spawn(async move {
            // `false`: pressing "Run now" has never consumed a `/loop`'s
            // `max_runs` budget, and making it do so would let the schedules UI
            // silently retire a bounded loop.
            match persist_run_start(&storage_path, &job_to_run, false).await {
                Ok(Some(_)) => {}
                Ok(None) => {
                    // Deleted from the file since this process loaded it. Give
                    // up the claim rather than run a schedule that is gone.
                    unregister_running_task(&running_tasks, &job_id);
                    forget_job(&jobs, &cron_handle, &job_id).await;
                    return Err(anyhow!(
                        "schedule '{job_id}' no longer exists; it was deleted while this run was \
                         being started"
                    ));
                }
                Err(e) => {
                    tracing::error!("Failed to persist run-now start for '{}': {}", job_id, e);
                }
            }

            let result = execute_job(job_to_run, jobs.clone(), job_id.clone(), cancel_token).await;

            // Completion BEFORE unregistering — see the cron path for why.
            let still_on_disk = record_run_completion(&storage_path, &jobs, &job_id, &result).await;
            unregister_running_task(&running_tasks, &job_id);
            if !still_on_disk {
                forget_job(&jobs, &cron_handle, &job_id).await;
            }
            result
        });

        match handle.await {
            Ok(Ok(session_id)) => Ok(session_id),
            Ok(Err(e)) => Err(SchedulerError::AnyhowError(anyhow!(
                "Job '{}' failed: {}",
                sched_id,
                e
            ))),
            // A panicked run is the one case the completion path above cannot
            // reach; BR-38's load-time reconcile clears the flag on restart.
            Err(join_error) => Err(SchedulerError::SchedulerInternalError(format!(
                "scheduled run '{sched_id}' panicked: {join_error}"
            ))),
        }
    }

    pub async fn pause_schedule(&self, sched_id: &str) -> Result<(), SchedulerError> {
        {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((_, job)) => {
                    if job.currently_running {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Cannot pause running schedule '{}'",
                            sched_id
                        )));
                    }
                    job.paused = true;
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        }

        self.publish_paused(sched_id, true).await
    }

    pub async fn unpause_schedule(&self, sched_id: &str) -> Result<(), SchedulerError> {
        {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((_, job)) => job.paused = false,
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        }

        self.publish_paused(sched_id, false).await
    }

    /// Publish the one field pause/unpause owns.
    ///
    /// Issue #140: this used to serialise the *whole* in-memory map, so pressing
    /// Pause in the desktop app deleted every schedule the CLI had added since
    /// the daemon started.
    ///
    /// A row the file no longer has is not re-inserted — and, since the fix's
    /// other half, is not left in this process either: pausing a job somebody
    /// deleted tells us the deletion happened, so the job is dropped here and
    /// the caller is told it is gone.
    async fn publish_paused(&self, sched_id: &str, paused: bool) -> Result<(), SchedulerError> {
        let id = sched_id.to_string();
        let found = persist_change(&self.storage_path, move |list| {
            Ok(edit_job(list, &id, |stored| stored.paused = paused))
        })
        .await?;
        if !found {
            forget_job(&self.jobs, &self.tokio_scheduler, sched_id).await;
            return Err(SchedulerError::JobNotFound(sched_id.to_string()));
        }
        Ok(())
    }

    pub async fn update_schedule(
        &self,
        sched_id: &str,
        new_cron: String,
    ) -> Result<(), SchedulerError> {
        let (old_uuid, updated_job) = {
            let mut jobs_guard = self.jobs.lock().await;
            match jobs_guard.get_mut(sched_id) {
                Some((uuid, job)) => {
                    if job.currently_running {
                        return Err(SchedulerError::AnyhowError(anyhow!(
                            "Cannot update running schedule '{}'",
                            sched_id
                        )));
                    }
                    if new_cron == job.cron {
                        return Ok(());
                    }
                    job.cron = new_cron.clone();
                    (*uuid, job.clone())
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
            }
        };

        self.tokio_scheduler
            .remove(&old_uuid)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        let cron_task = self.create_cron_task(updated_job)?;
        let new_uuid = self
            .tokio_scheduler
            .add(cron_task)
            .await
            .map_err(|e| SchedulerError::SchedulerInternalError(e.to_string()))?;

        {
            let mut jobs_guard = self.jobs.lock().await;
            if let Some((uuid, _)) = jobs_guard.get_mut(sched_id) {
                *uuid = new_uuid;
            }
        }

        let id = sched_id.to_string();
        let found = persist_change(&self.storage_path, move |list| {
            Ok(edit_job(list, &id, |stored| stored.cron = new_cron))
        })
        .await?;
        if !found {
            forget_job(&self.jobs, &self.tokio_scheduler, sched_id).await;
            return Err(SchedulerError::JobNotFound(sched_id.to_string()));
        }
        Ok(())
    }

    /// Stop a run that is in progress.
    ///
    /// ⚠ **A cancel that cancelled nothing is an error, not a success.** This
    /// used to return `Ok(())` whenever `running_tasks` held no token — the
    /// route then reported *"Successfully killed running job"*, the desktop
    /// Stop button went quiet, and the run carried on. That is the shape of the
    /// #148 cancel complaint. After the fix in [`claim_run_slot`] and
    /// [`Scheduler::run_now`] the token is registered under the same lock that
    /// publishes `currently_running`, and cleared only after the completion has
    /// been recorded, so "running with no token" no longer has a legitimate
    /// window: reaching it means the run already finished (or that this process
    /// is not the one running it), and the caller must be told.
    pub async fn kill_running_job(&self, sched_id: &str) -> Result<(), SchedulerError> {
        {
            let jobs_guard = self.jobs.lock().await;
            match jobs_guard.get(sched_id) {
                Some((_, job)) if !job.currently_running => {
                    return Err(SchedulerError::AnyhowError(anyhow!(
                        "Schedule '{}' is not running",
                        sched_id
                    )));
                }
                None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),
                _ => {}
            }
        }

        let cancelled = {
            let tasks = self
                .running_tasks
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match tasks.get(sched_id) {
                Some(token) => {
                    token.cancel();
                    true
                }
                None => false,
            }
        };

        if !cancelled {
            return Err(SchedulerError::AnyhowError(anyhow!(
                "Schedule '{}' has no run this process can stop: it is marked running but the \
                 run has already finished, or it was started by another Biorouter process. \
                 Nothing was cancelled.",
                sched_id
            )));
        }
        Ok(())
    }

    pub async fn get_running_job_info(
        &self,
        sched_id: &str,
    ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError> {
        let jobs_guard = self.jobs.lock().await;
        match jobs_guard.get(sched_id) {
            Some((_, job)) if job.currently_running => {
                match (&job.current_session_id, &job.process_start_time) {
                    (Some(sid), Some(start)) => Ok(Some((sid.clone(), *start))),
                    _ => Ok(None),
                }
            }
            Some(_) => Ok(None),
            None => Err(SchedulerError::JobNotFound(sched_id.to_string())),
        }
    }
}

/// The `(provider_name, model_config)` a scheduled run binds.
///
/// Issue #56 (§9.3 C2). This used to read `Config::global()` and nothing else,
/// which is wrong twice over:
///
/// * **R5.** A `/loop` or `/schedule` created from a chat running on a private
///   model produced runs on the user's *commercial* default — the schedule
///   quietly does its work somewhere the chat that asked for it would not.
/// * **C2.** Under §6.3 a `Scheduled` session inherits its creator's
///   classification, and Gate A refuses a public provider on a private row. A
///   job created from a private chat would therefore fail on *every* tick,
///   forever, with no repair affordance — a fresh session is minted per run, so
///   there is nothing for the user to switch.
///
/// The creating session is consulted first and the global default is the
/// fallback, so a job with no creator (a workflow scheduled from the CLI or the
/// schedules route) behaves exactly as it did before. A creator row that is
/// gone, or that records no provider, also falls back rather than failing: the
/// chat may legitimately have been deleted long after the schedule was made.
async fn resolve_scheduled_provider(
    job: &ScheduledJob,
    session_manager: &SessionManager,
) -> Result<(String, crate::model::ModelConfig)> {
    if let Some(creator_id) = job.creator_session_id.as_deref() {
        match session_manager.get_session(creator_id, false).await {
            Ok(creator) => match creator.provider_name.clone() {
                Some(provider_name) => {
                    // A row with a provider but no model config is a legacy row;
                    // `Agent::update_provider` has always written both together.
                    // Fall back for the model alone, exactly as
                    // `Agent::rebind_from_row` does, rather than discarding a
                    // perfectly good provider.
                    let model_config = match creator.model_config.clone() {
                        Some(model_config) => model_config,
                        None => {
                            let model_name = Config::global().get_biorouter_model()?;
                            crate::model::ModelConfig::new(&model_name)?
                        }
                    };
                    return Ok((provider_name, model_config));
                }
                None => tracing::warn!(
                    job = %job.id,
                    session = %creator_id,
                    "the chat this schedule was created from records no provider; \
                     using the global default"
                ),
            },
            Err(e) => tracing::warn!(
                job = %job.id,
                session = %creator_id,
                "the chat this schedule was created from could not be read ({e}); \
                 using the global default"
            ),
        }
    }

    let config = Config::global();
    let provider_name = config.get_biorouter_provider()?;
    let model_name = config.get_biorouter_model()?;
    Ok((provider_name, crate::model::ModelConfig::new(&model_name)?))
}

fn scheduled_prompt(job: &ScheduledJob, workflow: &Workflow) -> String {
    let base = workflow
        .prompt
        .as_ref()
        .or(workflow.instructions.as_ref())
        .cloned()
        .unwrap_or_default();
    if job.id != crate::knowledge::soul::MEDITATION_SCHEDULE_ID {
        return base;
    }

    let after = job
        .last_run
        .unwrap_or_else(|| Utc::now() - chrono::Duration::days(7));
    format!(
        "{base}\n\nMeditation discovery window: when searching Chat Recall, pass after_date exactly as `{}`. This cursor is the last successful Meditation run; on the first run it covers seven days. Exclude this scheduled session and every result whose name starts with `Scheduled job:`. If no real user session remains, make no knowledge write.",
        after.to_rfc3339()
    )
}

#[allow(clippy::too_many_lines)]
async fn execute_job(
    job: ScheduledJob,
    jobs: Arc<Mutex<JobsMap>>,
    job_id: String,
    cancel_token: CancellationToken,
) -> Result<String> {
    if job.source.is_empty() {
        return Ok(job.id.to_string());
    }

    let workflow_path = Path::new(&job.source);
    let workflow_content = fs::read_to_string(workflow_path)?;

    let workflow: Workflow = {
        let extension = workflow_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("yaml")
            .to_lowercase();

        match extension.as_str() {
            "json" | "jsonl" => serde_json::from_str(&workflow_content)?,
            _ => serde_yaml::from_str(&workflow_content)?,
        }
    };

    let agent = Agent::new();

    // Issue #56 (§9.3 C2 / R5). The creating chat's model first, the global
    // default only as a fallback — see [`resolve_scheduled_provider`].
    let (provider_name, model_config) =
        resolve_scheduled_provider(&job, agent.config.session_manager.as_ref()).await?;

    // ⚠ DELIBERATE BEHAVIOUR CHANGE, and the one place this task makes a job
    // fail that used to run. `resolve_scheduled_provider` falls back carefully —
    // a creator row that is gone, or that records no provider, yields the global
    // default — but a creator that DID name a provider is taken at its word, and
    // if that provider can no longer be constructed (its credential was revoked,
    // its endpoint retired) this `?` ends the run.
    //
    // Falling back to the global default here instead would be the R5 defect
    // wearing a repair's clothing: the job silently moves the private chat's
    // work onto the user's commercial default, which is exactly what the chat
    // chose a private model to avoid. And under C2 it would not even work — a
    // `Scheduled` row inherits its creator's classification, so Gate A refuses
    // the public bind a few lines below regardless.
    //
    // Failing loudly is therefore the correct outcome, and it is no longer
    // invisible: `run_workflow_job` records the error on the job as
    // `last_error`, and both schedule views render it.
    let agent_provider =
        crate::providers::create_from_persisted(&provider_name, model_config).await?;

    let mut extensions = resolve_extensions_for_new_session(workflow.extensions.as_deref(), None);
    crate::workflow::runtime::ensure_required_extensions(&workflow, &mut extensions);
    for ext in extensions {
        agent.add_extension(ext.clone()).await?;
    }

    let session = agent
        .config
        .session_manager
        .create_session(
            std::env::current_dir()?,
            format!("Scheduled job: {}", job.id),
            SessionType::Scheduled,
        )
        .await?;

    agent.update_provider(agent_provider, &session.id).await?;

    let prepared_workflow_prompt = crate::workflow::runtime::prepare_prompt(
        agent.config.session_manager.as_ref(),
        &session.id,
        &workflow,
    )
    .await?;

    // Persist and apply the workflow before the first model call. Scheduled
    // runs used to load only `extensions`; their declared knowledge base,
    // required skills, instructions, and structured-output components were
    // silently ignored until after the turn had already finished.
    agent
        .config
        .session_manager
        .update(&session.id)
        .schedule_id(Some(job.id.clone()))
        .workflow(Some(workflow.clone()))
        .apply()
        .await?;

    crate::workflow::runtime::install_prepared(
        &agent,
        &session.id,
        &workflow,
        true,
        prepared_workflow_prompt,
    )
    .await?;

    let mut jobs_guard = jobs.lock().await;
    if let Some((_, job_def)) = jobs_guard.get_mut(job_id.as_str()) {
        job_def.current_session_id = Some(session.id.clone());
    }
    drop(jobs_guard);

    let prompt_text = scheduled_prompt(&job, &workflow);

    let user_message = Message::user().with_text(prompt_text);
    let mut conversation = Conversation::new_unvalidated(vec![user_message.clone()]);

    let session_config = SessionConfig {
        id: session.id.clone(),
        schedule_id: Some(job.id.clone()),
        max_turns: None,
        max_tool_calls: None,
        budget: None,
        retry_config: None,
        reasoning_effort: None,
    };

    let session_id = session_config.id.clone();
    // Kept, because the outcome of this run is decided partly by whether the
    // token fired — see `finish_scheduled_run` at the bottom of this function.
    let run_token = cancel_token.clone();

    // ⚠ **A scheduled run is unattended by definition, and the scope must cover
    // the whole RUN — not the reply call, and certainly not a single dispatch.**
    //
    // `Agent::reply` returns a *stream*: almost nothing happens inside the call
    // that builds it, and every tool the turn dispatches runs while the loop
    // below polls it. `without_human_surface` is a `tokio::task_local`, so it
    // follows the future it is wrapped around and NOTHING else — wrapping only
    // the `reply(..)` call would set the flag for the construction and clear it
    // before the first tool ever ran, which is the exact shape of the mistake
    // this comment exists to stop being made again.
    //
    // Without it, a decision raised inside a `SessionType::Scheduled` session
    // parks a card in a queue no interface drains: nobody can answer it, the run
    // blocks until the TTL, and the outcome used to be recorded as a success —
    // which for `daily-meditation` silently consumed a day of the user's chats.
    //
    // ⚠ This covers everything that asks through `PendingUserActions::park` or
    // `ActionRequiredManager::request_and_wait` (any MCP server's
    // `create_elicitation` included). The tool-permission prompt in
    // `agents/tool_execution.rs` does NOT consult `no_human_surface()` yet, so it
    // still parks for its TTL; `classify_scheduled_run` below is what stops that
    // being reported as a success.
    let transcript = &mut conversation;
    let stream_error = crate::user_surface::without_human_surface(async {
        let stream = crate::session_context::with_session_id(Some(session_id.clone()), async {
            agent
                .reply(user_message, session_config, Some(cancel_token))
                .await
        })
        .await?;

        use futures::StreamExt;
        let mut stream = std::pin::pin!(stream);

        let mut stream_error = None;
        while let Some(message_result) = stream.next().await {
            tokio::task::yield_now().await;

            if let Err(error) = apply_scheduled_stream_item(transcript, message_result) {
                tracing::error!("Error in agent stream: {}", error);
                stream_error = Some(error);
                break;
            }
        }
        Ok::<_, anyhow::Error>(stream_error)
    })
    .await?;

    // Scheduled run finished: SessionEnd hooks (awaited; failure-open).
    {
        let hooks = agent.hooks_manager();
        // BR-28: shutdown boundary — join any observe-only hook still running
        // from the run's last turn, so it finishes here instead of being killed
        // mid-flight when the runtime tears the job's tasks down.
        hooks
            .join_fired(crate::hooks::FIRE_JOIN_BUDGET_SHUTDOWN)
            .await;
        let mut payload = crate::hooks::HookPayload::new(
            crate::hooks::HookEvent::SessionEnd,
            &session.id,
            session.working_dir.to_string_lossy(),
        );
        payload.source = Some("scheduled_run_complete".to_string());
        hooks
            .dispatch(
                crate::hooks::HookEvent::SessionEnd,
                Some("scheduled_run_complete"),
                &payload,
                &session.working_dir,
            )
            .await;
    }

    if let Some(error) = stream_error {
        return Err(error);
    }
    finish_scheduled_run(session.id, run_token.is_cancelled(), &conversation)
}

/// Why a scheduled run stopped (issue #148B).
///
/// Two of these three used to be indistinguishable from success at the
/// `execute_job` boundary, because neither ends the reply stream with an `Err`:
///
/// * **Cancelled.** `Agent::reply`'s loop `break`s on a cancelled token, so the
///   stream simply ends. `execute_job` returned `Ok`, both completion paths
///   cleared `last_error` and advanced `last_run` — and for `daily-meditation`
///   `last_run` *is* the Chat Recall discovery cursor, so pressing Stop silently
///   skipped an unprocessed window of the user's chats.
/// * **Refused.** Gate B returns its refusal as a normal one-message stream (an
///   `Err` there would surface as a 500 from `/reply`). The scheduled run drained
///   it without looking at it and reported success.
/// * **Approval expired.** A tool the turn wanted needed a person to approve it,
///   and a `SessionType::Scheduled` session has no interface anybody is looking
///   at. The prompt parked for its whole TTL (an hour by default) and then
///   expired; the turn carried on and ended normally, so the stream reported no
///   error and the run was recorded as a success — clearing `last_error` and
///   advancing `last_run`. That cursor is `daily-meditation`'s Chat Recall
///   discovery position, so a run in which the one thing it wanted to do was
///   refused silently consumed a day of the user's chats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScheduledRunEnd {
    Completed,
    Cancelled,
    Refused,
    ApprovalExpired,
}

/// Classify a finished run from the two signals available at the boundary.
///
/// The refusal arm keys on [`crate::privacy::refusal::TURN_REFUSAL_MARKER`],
/// which exists for exactly this: a substring two independent readers agree on.
///
/// ⚠ **The shape of the transcript is as load-bearing as the marker**, and this
/// is the second time that has been learned here. Only *assistant* text counts,
/// because a workflow prompt quoting the marker would otherwise fail every run
/// of that schedule. But scanning the whole transcript for assistant text is the
/// same mistake wearing the other hat, and it bites the job this feature exists
/// to protect: `daily-meditation` reads the user's chats, and any run that
/// *summarises a chat in which a turn was once refused* would be classified
/// `Refused`, never advance `last_run`, and re-scan the same window forever.
///
/// So the test is structural, not textual. Gate B refuses a turn by returning
/// the refusal as the *whole* reply — one assistant message and nothing else —
/// which no run that did work can look like, because a run that did work talks
/// about what it did. Exactly one assistant message, and it carries the marker.
fn classify_scheduled_run(cancelled: bool, conversation: &Conversation) -> ScheduledRunEnd {
    if cancelled {
        return ScheduledRunEnd::Cancelled;
    }
    let assistant: Vec<&Message> = conversation
        .iter()
        .filter(|message| message.role == rmcp::model::Role::Assistant)
        .collect();
    let refused = match assistant.as_slice() {
        [only] => only
            .as_concat_text()
            .contains(crate::privacy::refusal::TURN_REFUSAL_MARKER),
        _ => false,
    };
    if refused {
        return ScheduledRunEnd::Refused;
    }
    if an_approval_expired(conversation) {
        return ScheduledRunEnd::ApprovalExpired;
    }
    ScheduledRunEnd::Completed
}

/// Did a tool in this run stop because its permission prompt expired unanswered?
///
/// ⚠ **A TOOL RESPONSE, not text.** The refusal arm above had to reason
/// carefully about a workflow prompt that quotes its marker; this one cannot
/// have that problem, because a tool response is written by the tool machinery
/// and neither the model nor the workflow author can author one. That is the
/// whole reason the structural half is checked rather than the assistant's
/// `user_only` "the prompt expired" notification, whose wording lives inline in
/// `tool_execution.rs` and would drift silently.
///
/// The needle is the same `EXPIRED_RESPONSE` constant the expiry arm writes, so
/// the two halves cannot disagree about what an expiry looks like.
fn an_approval_expired(conversation: &Conversation) -> bool {
    conversation.iter().any(|message| {
        message.content.iter().any(|content| {
            let crate::conversation::message::MessageContent::ToolResponse(response) = content
            else {
                return false;
            };
            let Ok(result) = &response.tool_result else {
                return false;
            };
            result.content.iter().any(|item| {
                item.as_text()
                    .is_some_and(|text| text.text.contains(crate::agents::agent::EXPIRED_RESPONSE))
            })
        })
    })
}

/// The one place a scheduled run is allowed to call itself a success.
///
/// §14.4: a refusal reaching this string may name the tier and nothing else —
/// no session title, no working directory — so the refused arm restates the
/// policy rather than echoing the transcript.
fn finish_scheduled_run(
    session_id: String,
    cancelled: bool,
    conversation: &Conversation,
) -> Result<String> {
    match classify_scheduled_run(cancelled, conversation) {
        ScheduledRunEnd::Completed => Ok(session_id),
        ScheduledRunEnd::Cancelled => Err(anyhow!(
            "the run was stopped, so it {RUN_CANCELLED_MARKER} rather than finishing; \
             the schedule's last-run cursor was not advanced"
        )),
        ScheduledRunEnd::Refused => Err(anyhow!(
            "the privacy barrier refused this run's turn, so no work was done and the \
             schedule's last-run cursor was not advanced. This chat is private and the \
             model it is bound to is public; switch it to a private model."
        )),
        ScheduledRunEnd::ApprovalExpired => Err(anyhow!(
            "this run asked for a tool permission nobody could answer — a scheduled run \
             has no interface to show the card on — so the prompt expired, the tool did \
             not run, and the schedule's last-run cursor was not advanced. Run this \
             schedule from a chat, or give the workflow a permission mode that does not \
             need approval."
        )),
    }
}

fn apply_scheduled_stream_item(
    conversation: &mut Conversation,
    item: Result<AgentEvent>,
) -> Result<()> {
    match item? {
        AgentEvent::Message(message) => conversation.push(message),
        AgentEvent::HistoryReplaced(updated) => *conversation = updated,
        _ => {}
    }
    Ok(())
}

#[async_trait]
impl SchedulerTrait for Scheduler {
    async fn add_scheduled_job(
        &self,
        job: ScheduledJob,
        make_copy: bool,
    ) -> Result<(), SchedulerError> {
        self.add_scheduled_job(job, make_copy).await
    }

    async fn schedule_workflow(
        &self,
        workflow_path: PathBuf,
        cron_schedule: Option<String>,
    ) -> Result<(), SchedulerError> {
        self.schedule_workflow(workflow_path, cron_schedule).await
    }

    async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob> {
        self.list_scheduled_jobs().await
    }

    async fn remove_scheduled_job(
        &self,
        id: &str,
        remove_owned_workflow: bool,
    ) -> Result<(), SchedulerError> {
        self.remove_scheduled_job(id, remove_owned_workflow).await
    }

    async fn pause_schedule(&self, id: &str) -> Result<(), SchedulerError> {
        self.pause_schedule(id).await
    }

    async fn unpause_schedule(&self, id: &str) -> Result<(), SchedulerError> {
        self.unpause_schedule(id).await
    }

    async fn run_now(&self, id: &str) -> Result<String, SchedulerError> {
        self.run_now(id).await
    }

    async fn sessions(
        &self,
        sched_id: &str,
        limit: usize,
    ) -> Result<Vec<(String, Session)>, SchedulerError> {
        self.sessions(sched_id, limit).await
    }

    async fn update_schedule(
        &self,
        sched_id: &str,
        new_cron: String,
    ) -> Result<(), SchedulerError> {
        self.update_schedule(sched_id, new_cron).await
    }

    async fn kill_running_job(&self, sched_id: &str) -> Result<(), SchedulerError> {
        self.kill_running_job(sched_id).await
    }

    async fn get_running_job_info(
        &self,
        sched_id: &str,
    ) -> Result<Option<(String, DateTime<Utc>)>, SchedulerError> {
        self.get_running_job_info(sched_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::time::{sleep, Duration};

    fn create_test_workflow(dir: &Path, name: &str) -> PathBuf {
        let workflow_path = dir.join(format!("{}.yaml", name));
        fs::write(&workflow_path, "prompt: test\n").unwrap();
        workflow_path
    }

    /// A job with a cron that will not fire during a test, so the only runs are
    /// the explicit ones.
    fn dormant_job(id: &str, source: &Path) -> ScheduledJob {
        ScheduledJob {
            id: id.to_string(),
            source: source.to_string_lossy().into_owned(),
            cron: "0 0 0 1 1 *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            run_count: 0,
            max_runs: None,
            creator_session_id: None,
            last_error: None,
            owns_source: None,
        }
    }

    /// The pure half of the arbitrary-write refusal.
    ///
    /// Every string below reaches `format!("{id}.{ext}")` and then
    /// `Path::join`, where an absolute argument silently replaces the base and
    /// `..` is resolved by the kernel at `fs::copy`.
    #[test]
    fn a_schedule_id_that_is_not_a_plain_slug_is_refused() {
        for good in [
            "daily-meditation",
            "scheduled_job",
            "loop-1a2b3c4d",
            "a",
            "A1",
        ] {
            assert!(
                validate_schedule_id(good).is_ok(),
                "{good} is a plain slug and must stay creatable"
            );
        }
        for bad in [
            "",
            "/tmp/pwned",
            "../../pwned",
            "..",
            ".",
            "a/b",
            "a\\b",
            "with space",
            "dot.ted",
            "c:stream",
            "nul\0byte",
            "\u{2044}slash",
        ] {
            assert!(
                matches!(
                    validate_schedule_id(bad),
                    Err(SchedulerError::InvalidJobId(_))
                ),
                "{bad:?} must be refused: it names a file"
            );
        }
        assert!(validate_schedule_id(&"a".repeat(MAX_SCHEDULE_ID_LEN)).is_ok());
        assert!(validate_schedule_id(&"a".repeat(MAX_SCHEDULE_ID_LEN + 1)).is_err());
    }

    /// The whole defect, driven through the real entry point: an absolute id
    /// escapes `scheduled_workflows` entirely, and the copy used to happen
    /// before *any* validation — so the file landed even on the requests that
    /// were then refused.
    ///
    /// Asserting the refusal alone would not catch it. The write is the finding;
    /// the error code is only how it is reported.
    #[tokio::test]
    async fn an_absolute_schedule_id_is_refused_and_writes_nothing() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let workflow_path = create_test_workflow(temp_dir.path(), "real-workflow");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path.clone(), session_manager)
            .await
            .unwrap();

        // `Path::join` discards `scheduled_workflows` for an absolute argument,
        // so this id chooses its own directory. Pointed at the test's own temp
        // dir rather than /tmp so a regression cannot litter the machine.
        let outside = temp_dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let escaped = outside.join("pwned");
        let mut job = dormant_job(escaped.to_str().unwrap(), &workflow_path);
        job.cron = "0 0 3 * * *".to_string();

        let result = scheduler.add_scheduled_job(job, true).await;

        // ⚠ The WRITE is asserted first, and deliberately. The finding is the
        // file, not the status code: the old order copied before it parsed the
        // cron and before the duplicate guard, so the file landed even on the
        // requests that were then refused. A test that checks the error first
        // reports "wrong error" for a run that also wrote outside the managed
        // directory.
        assert!(
            !escaped.with_extension("yaml").exists(),
            "the id escaped the managed directory and wrote {}",
            escaped.with_extension("yaml").display()
        );
        assert!(
            fs::read_dir(&outside).unwrap().next().is_none(),
            "a refused add must leave nothing behind"
        );
        assert!(
            matches!(result, Err(SchedulerError::InvalidJobId(_))),
            "an absolute id must be refused, got {result:?}"
        );
        assert!(
            !storage_path.exists() || ids_on_disk(&storage_path).is_empty(),
            "a refused add must not persist a row either"
        );
    }

    /// The second half of the ordering fix. `fs::copy` TRUNCATES its
    /// destination, so an `add` that merely reused an existing id had already
    /// replaced that job's workflow file by the time it answered 409 — a
    /// rejected request silently rewriting what a live schedule runs.
    ///
    /// Two `Scheduler`s over one file, the shipped topology of issue #140: the
    /// second one's in-memory map has never heard of the first one's job, so the
    /// cheap check at the top of `add_scheduled_job` passes and the only guard
    /// left is the one inside `persist_change` — which the copy used to run
    /// ahead of.
    #[tokio::test]
    async fn a_duplicate_id_does_not_overwrite_the_workflow_the_id_already_owns() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let daemon = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        let cli = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();

        // Unique per run: `make_copy` writes into the real managed directory,
        // which is shared with whatever else is on this machine.
        let id = format!("dup-guard-{}", uuid::Uuid::new_v4().simple());
        let mine = create_test_workflow(temp_dir.path(), "mine");
        let mut first = dormant_job(&id, &mine);
        first.cron = "0 0 3 * * *".to_string();
        cli.add_scheduled_job(first, true).await.unwrap();

        let copied = get_default_scheduled_workflows_dir()
            .unwrap()
            .join(format!("{id}.yaml"));
        let before = fs::read_to_string(&copied).expect("the first add copied its workflow");

        let theirs = temp_dir.path().join("theirs.yaml");
        fs::write(&theirs, "prompt: replaced\n").unwrap();
        let mut second = dormant_job(&id, &theirs);
        second.cron = "0 0 3 * * *".to_string();
        let result = daemon.add_scheduled_job(second, true).await;

        let after = fs::read_to_string(&copied).unwrap();
        let _ = fs::remove_file(&copied);
        assert!(
            matches!(result, Err(SchedulerError::JobIdExists(_))),
            "the duplicate id must be refused, got {result:?}"
        );
        assert_eq!(
            after, before,
            "a refused duplicate must not have rewritten the existing job's workflow"
        );
    }

    /// The reported defect, driven at the scheduler. The "Add schedule" button
    /// on a workflow row schedules the user's file **in place**, and
    /// `DELETE /schedule/delete/{id}` asked for the workflow to be removed —
    /// so deleting the schedule deleted the workflow, and its row vanished from
    /// the workflows page too.
    ///
    /// ⚠ The assertion is the FILE, not the flag. An implementation that records
    /// ownership faithfully and then ignores it at the delete passes every
    /// assertion about `owns_source` and still destroys the user's workflow.
    #[tokio::test]
    async fn deleting_a_schedule_made_from_a_workflow_row_leaves_the_workflow_on_disk() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let workflow = create_test_workflow(temp_dir.path(), "probe-del-test");
        let scheduler = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();

        // `make_copy: false` — exactly what `schedule_workflow` passes, so
        // `source` IS the user's file.
        scheduler
            .add_scheduled_job(dormant_job("probe-del-test", &workflow), false)
            .await
            .unwrap();
        assert_eq!(
            scheduler.list_scheduled_jobs().await[0].owns_source,
            Some(false),
            "a workflow scheduled in place is not the scheduler's to delete"
        );

        // `true` is what the delete route asks for, and asking is all it may do.
        scheduler
            .remove_scheduled_job("probe-del-test", true)
            .await
            .unwrap();

        assert!(
            workflow.exists(),
            "deleting the schedule deleted the workflow it was created from: {}",
            workflow.display()
        );
        assert_eq!(
            fs::read_to_string(&workflow).unwrap(),
            "prompt: test\n",
            "the workflow survived as a file but not as its contents"
        );
        assert!(
            ids_on_disk(&storage_path).is_empty(),
            "the schedule must go"
        );
    }

    /// The other half, and what stops the fix degenerating into "never delete
    /// anything": a schedule created through `POST /schedule/create` copies the
    /// workflow into the scheduler's own store, and *that* copy is the
    /// scheduler's to remove.
    ///
    /// Both files are asserted. Deleting neither is a leak; deleting both is the
    /// bug above wearing this test's clothes.
    #[tokio::test]
    async fn deleting_a_schedule_that_copied_its_workflow_removes_the_copy_and_only_the_copy() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let original = create_test_workflow(temp_dir.path(), "original");
        let scheduler = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();

        // Unique per run: `make_copy` writes into the real managed directory,
        // which is shared with whatever else is on this machine.
        let id = format!("owned-copy-{}", uuid::Uuid::new_v4().simple());
        scheduler
            .add_scheduled_job(dormant_job(&id, &original), true)
            .await
            .unwrap();

        let copy = get_default_scheduled_workflows_dir()
            .unwrap()
            .join(format!("{id}.yaml"));
        assert!(copy.exists(), "make_copy did not copy");
        assert_eq!(
            scheduler.list_scheduled_jobs().await[0].owns_source,
            Some(true),
            "the scheduler wrote this file, so it must record that it owns it"
        );

        scheduler.remove_scheduled_job(&id, true).await.unwrap();

        let copy_survived = copy.exists();
        let _ = fs::remove_file(&copy);
        assert!(
            !copy_survived,
            "the scheduler's own copy was left behind at {}",
            copy.display()
        );
        assert!(
            original.exists(),
            "the workflow the copy was made FROM must never be touched"
        );
    }

    /// A row persisted before `owns_source` existed carries no answer, and the
    /// two creation paths that produced such rows are indistinguishable from the
    /// row alone. Where the file lives is the only signal left, and it is a
    /// sound one: `make_copy` is the only writer of that directory.
    ///
    /// Also pins the floor. An explicit `Some(true)` on a path OUTSIDE the store
    /// must still answer `false` — nothing this function says may ever delete a
    /// file the scheduler could not have written.
    #[test]
    fn ownership_of_an_unrecorded_job_is_decided_by_where_its_source_lives() {
        let store = get_default_scheduled_workflows_dir().unwrap();
        let inside = store.join("legacy-copy.yaml");
        let outside = Path::new("/Users/someone/.config/biorouter/workflows/mine.yaml");

        let job = |source: &Path, owns: Option<bool>| ScheduledJob {
            owns_source: owns,
            ..dormant_job("probe", source)
        };

        assert!(
            scheduler_owns_source(&job(&inside, None)),
            "a legacy row inside the scheduler's own store is a copy it made"
        );
        assert!(
            !scheduler_owns_source(&job(outside, None)),
            "a legacy row pointing outside the store is the user's own workflow"
        );
        assert!(
            !scheduler_owns_source(&job(&inside, Some(false))),
            "a recorded `false` must survive being inside the store"
        );
        assert!(
            !scheduler_owns_source(&job(outside, Some(true))),
            "no record may authorise deleting a file outside the scheduler's store"
        );
    }

    /// The `#[serde(default)]` that makes the paragraph above true: a schedule
    /// file written by an older build has no `owns_source` key at all, and must
    /// load as *unrecorded* rather than failing the whole document — which would
    /// take every other job on the machine with it.
    #[test]
    fn a_row_written_before_the_ownership_record_loads_as_unrecorded() {
        let legacy = r#"[{
            "id": "daily-meditation",
            "source": "/Users/wgu/.local/share/biorouter/scheduled_workflows/daily-meditation.yaml",
            "cron": "0 0 3 * * *",
            "last_run": null,
            "currently_running": false,
            "paused": true,
            "current_session_id": null,
            "process_start_time": null,
            "run_count": 3,
            "max_runs": null,
            "creator_session_id": null,
            "last_error": null
        }]"#;
        let jobs: Vec<ScheduledJob> = serde_json::from_str(legacy).unwrap();
        assert_eq!(jobs[0].owns_source, None);
    }

    /// The delete path may not grow a second route to `fs::remove_file`.
    ///
    /// Structural because the failure it guards is an *omission*: the route
    /// asks for the workflow to be removed and always has, so a rewrite that
    /// drops the ownership question restores the original bug while every
    /// caller still reads correctly.
    #[test]
    fn the_delete_path_asks_who_owns_the_source_before_unlinking_it() {
        let src = include_str!("scheduler.rs");
        let (production, _) = src
            .split_once("\n#[cfg(test)]")
            .expect("scheduler.rs has no test module");
        let (_, after) = production
            .split_once("pub async fn remove_scheduled_job(")
            .expect("remove_scheduled_job is gone");
        let (body, _) = after
            .split_once("\n    pub async fn ")
            .expect("could not find the end of remove_scheduled_job");
        assert!(
            body.contains("scheduler_owns_source(&job)"),
            "remove_scheduled_job unlinks a file without asking whether the scheduler wrote it"
        );
        assert_eq!(
            production.matches("fs::remove_file(").count(),
            1,
            "a second unlink appeared in the scheduler; it needs the same ownership question"
        );
    }

    fn ids_on_disk(storage_path: &Path) -> Vec<String> {
        jobs_on_disk(storage_path)
            .into_iter()
            .map(|job| job.id)
            .collect()
    }

    fn jobs_on_disk(storage_path: &Path) -> Vec<ScheduledJob> {
        serde_json::from_str(&fs::read_to_string(storage_path).unwrap()).unwrap()
    }

    /// Issue #140, the measured defect: `biorouter schedule add` created a job,
    /// the desktop app never saw it, and pressing Pause on an *unrelated*
    /// schedule deleted it.
    ///
    /// Two `Scheduler`s over one file is exactly the shipped topology — the CLI
    /// builds its own for every subcommand while the daemon holds one open — and
    /// the daemon's map is loaded once, at construction. Fails against the old
    /// `persist_jobs`, which serialised that stale whole map over the file:
    /// `cli-added` is gone from disk the moment the daemon pauses `shared`.
    #[tokio::test]
    async fn one_schedulers_write_does_not_delete_the_others_job() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let shared_wf = create_test_workflow(temp_dir.path(), "shared");
        let cli_wf = create_test_workflow(temp_dir.path(), "cli_added");

        // The daemon, holding one job.
        let daemon = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        daemon
            .add_scheduled_job(dormant_job("shared", &shared_wf), false)
            .await
            .unwrap();

        // The CLI: a second Scheduler over the same file, which adds a job the
        // daemon's map will never hear about.
        let cli = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        cli.add_scheduled_job(dormant_job("cli-added", &cli_wf), false)
            .await
            .unwrap();
        assert!(
            ids_on_disk(&storage_path).contains(&"cli-added".to_string()),
            "precondition: the CLI's job reached disk"
        );

        // The GUI's Pause button, on the UNRELATED job.
        daemon.pause_schedule("shared").await.unwrap();

        let on_disk = jobs_on_disk(&storage_path);
        let ids: Vec<&str> = on_disk.iter().map(|job| job.id.as_str()).collect();
        assert!(
            ids.contains(&"cli-added"),
            "pausing an unrelated job deleted the other writer's schedule: {ids:?}"
        );
        assert!(
            on_disk
                .iter()
                .find(|job| job.id == "shared")
                .expect("the paused job is still on disk")
                .paused,
            "the pause itself must still be published"
        );
    }

    /// The mirror of the test above, and the reason the fix publishes FIELDS
    /// through [`edit_job`] rather than upserting whole jobs from memory.
    ///
    /// Fails against the obvious wrong fix — "read the file, then write every
    /// job I hold back over it" — which passes the test above while quietly
    /// resurrecting a job the other writer deleted.
    #[tokio::test]
    async fn a_write_does_not_resurrect_a_job_another_writer_removed() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let keep_wf = create_test_workflow(temp_dir.path(), "keep");
        let doomed_wf = create_test_workflow(temp_dir.path(), "doomed");

        let daemon = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        daemon
            .add_scheduled_job(dormant_job("keep", &keep_wf), false)
            .await
            .unwrap();
        daemon
            .add_scheduled_job(dormant_job("doomed", &doomed_wf), false)
            .await
            .unwrap();

        // A second writer that loaded both, and removes one.
        let cli = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        cli.remove_scheduled_job("doomed", false).await.unwrap();
        assert_eq!(ids_on_disk(&storage_path), vec!["keep".to_string()]);

        // The daemon still holds `doomed` in memory. Neither touching an
        // unrelated job nor touching `doomed` itself may bring it back.
        daemon.pause_schedule("keep").await.unwrap();
        // ⚠ This used to be `.unwrap()`, i.e. it asserted that pausing a deleted
        // job SUCCEEDS silently. It does not any more, and the change is the
        // point of `a_job_deleted_from_the_file_stops_being_served_and_stops_firing`:
        // a no-op that reports success leaves the caller believing the job is
        // paused and this process still listing and firing it. The disk
        // assertion below — the thing this test was written for — is unchanged.
        let refused = daemon
            .pause_schedule("doomed")
            .await
            .expect_err("pausing a job the file no longer has is not a success");
        assert!(
            matches!(refused, SchedulerError::JobNotFound(_)),
            "{refused}"
        );

        assert_eq!(
            ids_on_disk(&storage_path),
            vec!["keep".to_string()],
            "a removed job must stay removed"
        );
    }

    #[tokio::test]
    async fn test_job_fires_and_records_its_outcome_on_schedule() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let workflow_path = create_test_workflow(temp_dir.path(), "scheduled_job");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "scheduled_job".to_string(),
            source: workflow_path.to_string_lossy().to_string(),
            cron: "* * * * * *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            run_count: 0,
            max_runs: None,
            creator_session_id: None,
            last_error: None,
            owns_source: None,
        };

        scheduler.add_scheduled_job(job, true).await.unwrap();
        let mut observed = None;
        for _ in 0..40 {
            let job = scheduler.list_scheduled_jobs().await.remove(0);
            if job.run_count > 0 && !job.currently_running {
                observed = Some(job);
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }

        let job = observed.expect("cron should fire and finish within four seconds");
        assert_eq!(job.run_count, 1, "one cron firing should be claimed");
        assert!(
            job.last_run.is_some() || job.last_error.is_some(),
            "a completed firing must expose either success or failure"
        );
    }

    #[tokio::test]
    async fn test_paused_job_does_not_run() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let workflow_path = create_test_workflow(temp_dir.path(), "paused_job");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let job = ScheduledJob {
            id: "paused_job".to_string(),
            source: workflow_path.to_string_lossy().to_string(),
            cron: "* * * * * *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            run_count: 0,
            max_runs: None,
            creator_session_id: None,
            last_error: None,
            owns_source: None,
        };

        scheduler.add_scheduled_job(job, true).await.unwrap();
        scheduler.pause_schedule("paused_job").await.unwrap();
        sleep(Duration::from_millis(1500)).await;

        let jobs = scheduler.list_scheduled_jobs().await;
        assert!(jobs[0].last_run.is_none(), "Paused job should not run");
    }

    // BR-38: a job persisted mid-run (crash while running) must reload with its
    // running state cleared, or `claim_run_slot`'s overlap guard skips it forever.
    #[tokio::test]
    async fn test_stale_running_flag_reconciled_on_load() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let workflow_path = create_test_workflow(temp_dir.path(), "stale_job");

        // Simulate a job that was mid-run at crash time. Use a cron that will not
        // fire during the test so the reset can only come from load-time reconcile.
        let stored = vec![ScheduledJob {
            id: "stale_job".to_string(),
            source: workflow_path.to_string_lossy().to_string(),
            cron: "0 0 0 1 1 *".to_string(),
            last_run: None,
            currently_running: true,
            paused: false,
            current_session_id: Some("ghost-session".to_string()),
            process_start_time: Some(Utc::now()),
            run_count: 0,
            max_runs: None,
            creator_session_id: None,
            last_error: None,
            owns_source: None,
        }];
        fs::write(&storage_path, serde_json::to_string(&stored).unwrap()).unwrap();

        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        let jobs = scheduler.list_scheduled_jobs().await;
        assert_eq!(jobs.len(), 1);
        assert!(
            !jobs[0].currently_running,
            "stale currently_running flag should be reset on load"
        );
        assert!(
            jobs[0].current_session_id.is_none(),
            "stale session id should be cleared on load"
        );
        assert!(
            jobs[0].process_start_time.is_none(),
            "stale process_start_time should be cleared on load"
        );
    }

    #[test]
    fn scheduled_workflow_state_is_applied_before_the_first_model_call() {
        let source = include_str!("scheduler.rs");
        let execute = source
            .split("async fn execute_job(")
            .nth(1)
            .and_then(|rest| rest.split("impl SchedulerTrait for Scheduler").next())
            .expect("execute_job production body");
        let persisted = execute
            .find(".workflow(Some(workflow.clone()))")
            .expect("scheduled workflow is persisted");
        let prepared = execute
            .find("runtime::prepare_prompt")
            .expect("workflow skills and prompt are preflighted");
        // Knowledge selection and prompt assembly are ONE call now
        // (`runtime::install_prepared`), shared with `biorouter run --workflow`,
        // which used to do neither. Their relative order stopped being a
        // property of this function the moment they stopped being two calls
        // here — so it is asserted where it now lives, in `install_prepared`
        // itself, rather than dropped.
        let installed = execute
            .find("runtime::install_prepared")
            .expect("scheduled knowledge selection, skills and components are applied");
        let reply = execute.find(".reply(").expect("scheduled model call");

        assert!(
            prepared < persisted,
            "fallible workflow inputs are preflighted first"
        );
        assert!(
            persisted < installed,
            "workflow must be stored before its state is installed"
        );
        assert!(installed < reply, "all workflow state must precede reply");

        let runtime = include_str!("workflow/runtime.rs");
        let install = runtime
            .split("pub async fn install_prepared(")
            .nth(1)
            .and_then(|rest| rest.split("\n}").next())
            .expect("install_prepared production body");
        let knowledge = install
            .find("apply_knowledge_selection")
            .expect("the shared install applies the knowledge selection");
        let skills_and_components = install
            .find("apply_prepared_to_agent")
            .expect("the shared install applies skills and components");
        assert!(
            knowledge < skills_and_components,
            "knowledge precedes prompt assembly"
        );
    }

    /// Issue #148A: `run_now` is awaited inside an axum handler, and axum drops
    /// a handler's future when the client disconnects. The old body marked the
    /// job running, then awaited `execute_job` inline — every line that cleared
    /// the flag was after that await — so a closed tab left the job
    /// `currently_running` forever, which makes `claim_run_slot` skip it, and
    /// `pause_schedule` / `update_schedule` refuse it.
    ///
    /// The drop is real, not simulated: the future is polled until it parks and
    /// then dropped, which is what axum does to it. Against the old
    /// implementation the first poll parks inside the start-of-run persist, so
    /// the flag is set and no task exists to clear it, and this test hangs on
    /// the assertion below until it fails.
    /// ⚠ `current_thread`, and the precondition below is read WITHOUT awaiting.
    /// Both are load-bearing, and this test was flaky (~1 run in 8) until they
    /// were: the fix under test detaches the run, and a nonexistent source makes
    /// that detached task finish almost immediately, so an `.await` between the
    /// poll and the precondition hands it exactly the opening it needs to clear
    /// the flag before the assertion reads it. The test then fails reporting
    /// that the job was never marked running — the opposite of what happened.
    #[tokio::test(flavor = "current_thread")]
    async fn a_dropped_run_now_future_still_clears_the_running_flag() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let scheduler = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();

        // A source that does not exist, so the run fails fast instead of
        // reaching a provider or the developer's real session store. What is
        // under test is that the flag is cleared at all, not what cleared it.
        let job = dormant_job("dropped-run", &temp_dir.path().join("gone.yaml"));
        scheduler.add_scheduled_job(job, false).await.unwrap();

        {
            let mut run = Box::pin(scheduler.run_now("dropped-run"));
            assert!(
                futures::poll!(run.as_mut()).is_pending(),
                "the run must still be in flight when the caller goes away"
            );
            // `try_lock`, not `list_scheduled_jobs().await`: on a current-thread
            // runtime a task is only preempted at an await point, so reading the
            // flag without one is what makes this precondition deterministic
            // rather than a race against the detached run.
            let running = scheduler
                .jobs
                .try_lock()
                .expect("nothing else holds the jobs lock at this point")
                .get("dropped-run")
                .map(|(_, job)| job.currently_running)
                .expect("the job is registered");
            assert!(
                running,
                "precondition: the job was marked running before the drop"
            );
            drop(run);
        }

        // Poll the FILE, not memory: `record_run_completion` writes memory first,
        // so a cleared on-disk copy proves both halves ran.
        let mut cleared = false;
        for _ in 0..100 {
            sleep(Duration::from_millis(50)).await;
            if !jobs_on_disk(&storage_path)[0].currently_running {
                cleared = true;
                break;
            }
        }
        assert!(
            cleared,
            "a dropped run-now left the job marked running forever, bricking the schedule"
        );
        assert!(
            !scheduler.list_scheduled_jobs().await[0].currently_running,
            "the in-memory copy must be cleared too, or this process keeps skipping the job"
        );
    }

    /// Issue #148B, the half that loses data. Cancellation `break`s the reply
    /// loop rather than erroring, so `execute_job` returned `Ok` and both
    /// completion paths advanced `last_run` — which, for `daily-meditation`, IS
    /// the Chat Recall discovery cursor. Stopping a run therefore skipped an
    /// unprocessed window of the user's chats, silently.
    ///
    /// Fails a wrong implementation that returns `Ok` on a cancelled token, and
    /// separately one that advances `last_run` on any completion rather than on
    /// a successful one.
    #[test]
    fn a_cancelled_run_is_not_a_success_and_does_not_move_the_cursor() {
        let empty = Conversation::default();
        assert_eq!(
            classify_scheduled_run(true, &empty),
            ScheduledRunEnd::Cancelled
        );

        let error = finish_scheduled_run("session-1".to_string(), true, &empty)
            .expect_err("a cancelled run must not report success");
        assert!(
            error.to_string().contains(RUN_CANCELLED_MARKER),
            "the server route keys on this marker to return its CANCELLED sentinel: {error}"
        );

        let cursor = chrono::DateTime::parse_from_rfc3339("2026-08-20T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut job = dormant_job("daily-meditation", Path::new("/does/not/matter.yaml"));
        job.last_run = Some(cursor);
        job.currently_running = true;
        job.current_session_id = Some("run-session".to_string());

        RunCompletion::from_result(&Err(error), Utc::now()).apply(&mut job);

        assert_eq!(
            job.last_run,
            Some(cursor),
            "a cancelled run must leave the discovery cursor where it was"
        );
        assert!(!job.currently_running, "the run is over either way");
        assert!(job.current_session_id.is_none());
        assert!(
            job.last_error
                .is_some_and(|e| e.contains(RUN_CANCELLED_MARKER)),
            "the stop must be visible on the schedule, not only in a log line"
        );
    }

    /// A successful run is still a success — the guard against "fix cancellation
    /// by never advancing the cursor", which would break the Meditation job in
    /// the other direction (it would re-scan the same window forever).
    #[test]
    fn a_successful_run_still_advances_the_cursor_and_clears_the_error() {
        let finished_at = Utc::now();
        let mut job = dormant_job("daily-meditation", Path::new("/does/not/matter.yaml"));
        job.last_error = Some("a failure from the previous run".to_string());

        RunCompletion::from_result(&Ok("session-1".to_string()), finished_at).apply(&mut job);

        assert_eq!(job.last_run, Some(finished_at));
        assert!(job.last_error.is_none());
    }

    /// Issue #148B's other shape: Gate B returns its refusal as a normal
    /// one-message stream (an `Err` there would surface as a 500 from `/reply`),
    /// which the scheduled run drained without inspecting and reported as
    /// success.
    ///
    /// Fails an implementation that only checks the cancel token. The third case
    /// is the one a naive "does the transcript contain the marker" check gets
    /// wrong: the run's own prompt is in the same `Conversation`, so a workflow
    /// that quotes the marker would otherwise fail every single run.
    #[test]
    fn a_privacy_refusal_is_not_a_success_but_an_echo_of_it_is_harmless() {
        let mut refused = Conversation::default();
        refused.push(Message::assistant().with_text(format!(
            "This chat is private, so {}. Switch this chat to a private model.",
            crate::privacy::refusal::TURN_REFUSAL_MARKER
        )));
        assert_eq!(
            classify_scheduled_run(false, &refused),
            ScheduledRunEnd::Refused
        );
        assert!(finish_scheduled_run("session-1".to_string(), false, &refused).is_err());

        let mut ordinary = Conversation::default();
        ordinary.push(Message::assistant().with_text("Meditation complete."));
        assert_eq!(
            classify_scheduled_run(false, &ordinary),
            ScheduledRunEnd::Completed
        );

        let mut quoted_by_the_prompt = Conversation::default();
        quoted_by_the_prompt.push(
            Message::user().with_text(crate::privacy::refusal::TURN_REFUSAL_MARKER.to_string()),
        );
        quoted_by_the_prompt.push(Message::assistant().with_text("Meditation complete."));
        assert_eq!(
            classify_scheduled_run(false, &quoted_by_the_prompt),
            ScheduledRunEnd::Completed,
            "only the assistant's own words are evidence of a refusal"
        );
    }

    /// The mirror image of the bug above, and the one that bites the job this
    /// whole classification exists for.
    ///
    /// `daily-meditation` reads the user's chat history. Sooner or later one of
    /// those chats contains a turn the privacy barrier refused, and the run
    /// quotes it back while summarising. A classifier that scans the whole
    /// transcript for an assistant message containing the marker calls that run
    /// `Refused`, so `last_run` — which IS the Chat Recall discovery cursor —
    /// never advances, and every subsequent run re-scans the same window and
    /// quotes the same refusal again. Permanently stuck, and reported as a
    /// failure every night.
    ///
    /// Fails the "any assistant message" implementation this replaced.
    #[test]
    fn a_run_that_merely_quotes_an_old_refusal_still_counts_as_work_done() {
        let mut summarised = Conversation::default();
        summarised.push(Message::user().with_text("Meditate over the last week of chats."));
        summarised.push(Message::assistant().with_text("Reading the week's sessions."));
        summarised.push(Message::assistant().with_text(format!(
            "One session ended with a privacy refusal: \"{}\". Recorded that the user works on \
             private data.",
            crate::privacy::refusal::TURN_REFUSAL_MARKER
        )));
        assert_eq!(
            classify_scheduled_run(false, &summarised),
            ScheduledRunEnd::Completed,
            "a run that did work and happened to quote a past refusal is not itself a refusal"
        );

        // And the consequence the classifier exists to protect: the cursor moves.
        let finished_at = Utc::now();
        let mut job = dormant_job("daily-meditation", Path::new("/does/not/matter.yaml"));
        let outcome = finish_scheduled_run("session-1".to_string(), false, &summarised);
        RunCompletion::from_result(&outcome, finished_at).apply(&mut job);
        assert_eq!(
            job.last_run,
            Some(finished_at),
            "the discovery cursor must advance, or the same window is re-scanned forever"
        );

        // A real refusal is still a refusal: Gate B returns it as the WHOLE
        // reply, so one assistant message and nothing else.
        let mut only_the_refusal = Conversation::default();
        only_the_refusal.push(Message::user().with_text("Meditate over the last week of chats."));
        only_the_refusal.push(Message::assistant().with_text(format!(
            "This chat is private, so {}. Switch this chat to a private model.",
            crate::privacy::refusal::TURN_REFUSAL_MARKER
        )));
        assert_eq!(
            classify_scheduled_run(false, &only_the_refusal),
            ScheduledRunEnd::Refused
        );
    }

    /// The transcript an expired permission prompt leaves behind: the tool
    /// machinery writes an error tool response carrying `EXPIRED_RESPONSE`, and
    /// the turn then carries on and ends normally.
    fn expired_approval_transcript() -> Conversation {
        let mut conversation = Conversation::default();
        conversation.push(Message::user().with_text("Meditate over the last week of chats."));
        conversation.push(Message::assistant().with_text("Reading the week's sessions."));
        conversation.push(Message::user().with_tool_response(
            "req-1",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text(
                    crate::agents::agent::EXPIRED_RESPONSE,
                )],
                structured_content: None,
                is_error: Some(true),
                meta: None,
            }),
        ));
        conversation
            .push(Message::assistant().with_text("I was going to write the note but could not."));
        conversation
    }

    /// A scheduled run that stopped because an approval nobody could give
    /// expired is NOT a success.
    ///
    /// `SessionType::Scheduled` has no interface, so the card parks in a queue
    /// no one drains for the whole hour-long TTL. The turn then continues and
    /// ends without error, which the classifier's `_ => false` arm read as
    /// success — clearing `last_error` and advancing `last_run`. For
    /// `daily-meditation` that cursor IS the Chat Recall discovery position, so
    /// a run in which the write never happened silently consumed a day of the
    /// user's chats.
    #[test]
    fn an_expired_approval_is_not_a_success_and_does_not_move_the_cursor() {
        let expired = expired_approval_transcript();
        assert_eq!(
            classify_scheduled_run(false, &expired),
            ScheduledRunEnd::ApprovalExpired,
            "an approval that expired unanswered is not work done"
        );
        assert!(finish_scheduled_run("session-1".to_string(), false, &expired).is_err());

        // The consequence, which is the finding: the discovery cursor must stay
        // where it is, and the failure must be visible on the schedule.
        let cursor = chrono::DateTime::parse_from_rfc3339("2026-08-20T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut job = dormant_job("daily-meditation", Path::new("/does/not/matter.yaml"));
        job.last_run = Some(cursor);
        let outcome = finish_scheduled_run("session-1".to_string(), false, &expired);
        RunCompletion::from_result(&outcome, Utc::now()).apply(&mut job);
        assert_eq!(
            job.last_run,
            Some(cursor),
            "a run whose only tool was never approved must not advance the cursor"
        );
        assert!(
            job.last_error.is_some(),
            "the expiry must be visible on the schedule, not only in a log line"
        );
    }

    /// The over-match this classifier must not have. A run that merely *talks
    /// about* an expired prompt — `daily-meditation` summarising a chat in which
    /// one happened — did work, and its cursor must advance. Only a real tool
    /// response counts, and neither the model nor a workflow author can write
    /// one of those.
    #[test]
    fn a_run_that_merely_quotes_an_expiry_still_counts_as_work_done() {
        let mut summarised = Conversation::default();
        summarised.push(Message::user().with_text(format!(
            "Note that a past chat said: {}",
            crate::agents::agent::EXPIRED_RESPONSE
        )));
        summarised.push(Message::assistant().with_text(format!(
            "One session ended with \"{}\" — recorded.",
            crate::agents::agent::EXPIRED_RESPONSE
        )));
        assert_eq!(
            classify_scheduled_run(false, &summarised),
            ScheduledRunEnd::Completed,
            "prose about an expiry is not an expiry; only a tool response is"
        );

        // And a tool response that succeeded is not one either.
        let mut worked = Conversation::default();
        worked.push(Message::user().with_tool_response(
            "req-1",
            Ok(rmcp::model::CallToolResult {
                content: vec![rmcp::model::Content::text("wrote the note")],
                structured_content: None,
                is_error: None,
                meta: None,
            }),
        ));
        worked.push(Message::assistant().with_text("Done."));
        assert_eq!(
            classify_scheduled_run(false, &worked),
            ScheduledRunEnd::Completed
        );
    }

    /// **The scope has to cover the RUN, not the call that builds it.**
    ///
    /// `Agent::reply` returns a stream; every tool the turn dispatches runs while
    /// the drain loop polls it. `without_human_surface` is a `tokio::task_local`,
    /// so it follows the future it wraps and nothing else — wrapping only the
    /// `reply(..)` call would set the flag for the construction and clear it
    /// before the first tool ran, and no counter anywhere would report that.
    ///
    /// A source-shape pin, for the same reason as
    /// `a_scheduled_run_never_reports_success_without_classifying_its_end`: a
    /// behavioural test needs a real provider and would write into the
    /// developer's own session store. Modelled on
    /// `biorouter-server/tests/call_tool_no_human_surface.rs`, which pins the
    /// same property at the other surface that has no person to ask.
    #[test]
    fn a_scheduled_run_drains_its_stream_inside_the_unattended_scope() {
        let source = include_str!("scheduler.rs");
        let execute = source
            .split("async fn execute_job(")
            .nth(1)
            .and_then(|rest| rest.split("\n/// Why a scheduled run stopped").next())
            .expect("execute_job production body");

        let scope = execute
            .find("without_human_surface(async")
            .expect("a scheduled run must be scoped as unattended: there is nobody to ask");
        let reply = execute
            .find(".reply(user_message, session_config")
            .expect("execute_job starts the run's turn");
        let drain = execute
            .find("while let Some(message_result) = stream.next().await")
            .expect("execute_job drains the reply stream");
        // `match_indices` rather than a slice-then-find: `clippy::string_slice`
        // is denied here, and rightly — a byte offset into a `&str` is only safe
        // on a character boundary, which a search result happens to be and the
        // next edit to these markers might not.
        let scope_end = execute
            .match_indices("\n    })\n    .await?;")
            .map(|(index, _)| index)
            .find(|index| *index > scope)
            .expect("the unattended scope is closed");

        assert!(
            scope < reply && reply < scope_end,
            "the turn must START inside the unattended scope"
        );
        assert!(
            scope < drain && drain < scope_end,
            "the stream must be DRAINED inside the unattended scope — a task_local does not \
             survive a future awaited outside its own scope(), so scoping the reply call \
             alone would clear the flag before the first tool ran"
        );
    }

    /// Issue #140's other direction, and the one the modify-if-present fix made
    /// *worse* rather than better.
    ///
    /// The CLI is entirely offline: every `biorouter schedule …` subcommand
    /// builds its own `Scheduler` over the same `schedule.json` and never
    /// contacts `biorouterd`. The daemon's map is filled once, at construction.
    /// So after `biorouter schedule remove`, the daemon that never saw the delete
    /// used to keep serving the job from `/schedule/list` AND keep firing its
    /// cron entry — burning tokens on a schedule the user deleted, until the
    /// daemon restarted. Before the modify-if-present fix that at least
    /// resurrected the row on disk, wrongly but visibly; after it, the
    /// divergence was permanent and silent.
    ///
    /// Fails an implementation whose only response to a deleted row is to write
    /// nothing: the job is still listed, and `run_count` still climbs.
    ///
    /// ⚠ `#[serial]`, and every assertion below is CONVERGENT rather than
    /// instantaneous. This job fires every second, so the property under test is
    /// eventual by construction: a tick already in flight when the delete lands
    /// still records its own completion, and the daemon drops the row on the
    /// NEXT reconcile. Run in parallel with the other 25 scheduler tests, several
    /// of which also drive one-second crons, this one's ticks were starved often
    /// enough to fail roughly 1 run in 5 — while passing 6/6 in isolation, which
    /// is exactly the shape that reads as a code regression and is not one.
    #[serial_test::serial]
    #[tokio::test]
    async fn a_job_deleted_from_the_file_stops_being_served_and_stops_firing() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let workflow = create_test_workflow(temp_dir.path(), "deleted_elsewhere");

        let daemon = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();

        // Fires every second, so "still firing" is observable in a test.
        let mut job = dormant_job("deleted-elsewhere", &workflow);
        job.cron = "* * * * * *".to_string();
        daemon.add_scheduled_job(job, false).await.unwrap();

        // It really is live before the delete.
        let mut fired = false;
        for _ in 0..40 {
            sleep(Duration::from_millis(100)).await;
            if daemon.list_scheduled_jobs().await[0].run_count > 0 {
                fired = true;
                break;
            }
        }
        assert!(fired, "precondition: the job fires while it is on disk");

        // The CLI, in another process, deletes it. Modelled as a direct write to
        // the file because that is exactly what the offline CLI's own
        // `Scheduler` does, and because writing it here proves the daemon reads
        // the file rather than being told by an API call.
        fs::write(&storage_path, "[]").unwrap();

        // ⚠ CONVERGES, not instantaneous — and the difference is a real race, not
        // test hygiene. This job fires every second, and a tick already in flight
        // when the write lands will finish by persisting its own run state, which
        // puts the row back in the file the test just emptied. The daemon then
        // drops it on the NEXT reconcile. Asserting emptiness on the first call
        // therefore fails whenever a tick happens to straddle the write — which
        // it did once in a full-suite run while passing 3/3 in isolation.
        //
        // Production's contract is convergence, so that is what this asserts.
        let mut converged = false;
        for _ in 0..40 {
            if daemon.list_scheduled_jobs().await.is_empty() {
                converged = true;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert!(
            converged,
            "a job the file no longer has must stop being served from /schedule/list"
        );

        // And it must stop firing — which is also a CONVERGENT property, for the
        // same reason. A tick already running when the delete landed may still
        // record its own completion, so both the run count and the on-disk row
        // can move once more before the next reconcile drops the job for good.
        //
        // Settle first, then assert the count is frozen. Asserting immediately
        // measures whether a tick happened to be in flight, which is a property
        // of the machine rather than of the code under test.
        let mut settled = false;
        for _ in 0..40 {
            let before = jobs_after_delete_run_count(&daemon).await;
            sleep(Duration::from_millis(1200)).await;
            if jobs_after_delete_run_count(&daemon).await == before
                && daemon.list_scheduled_jobs().await.is_empty()
                && ids_on_disk(&storage_path).is_empty()
            {
                settled = true;
                break;
            }
        }
        assert!(
            settled,
            "a deleted schedule kept firing: the daemon is still burning turns on it"
        );

        // Now that it has settled, it must STAY gone across several more cron
        // intervals — the part that would catch a job resurrecting itself.
        let frozen = jobs_after_delete_run_count(&daemon).await;
        sleep(Duration::from_millis(2500)).await;
        assert_eq!(
            jobs_after_delete_run_count(&daemon).await,
            frozen,
            "a settled deletion started firing again"
        );
        assert!(
            daemon.list_scheduled_jobs().await.is_empty(),
            "and it must stay gone"
        );
        assert_eq!(ids_on_disk(&storage_path), Vec::<String>::new());
    }

    /// How many runs the (possibly already forgotten) job has recorded. `0` once
    /// it is gone, which is stable — the point is that it does not climb.
    async fn jobs_after_delete_run_count(scheduler: &Arc<Scheduler>) -> u32 {
        scheduler
            .list_scheduled_jobs()
            .await
            .first()
            .map_or(0, |job| job.run_count)
    }

    /// The recovery path must not be the destruction path.
    ///
    /// A parse failure used to be answered with `Ok(Vec::new())`: the caller then
    /// applied its own change to that empty list and published it, so the FIRST
    /// pause, completion or run-start after a corrupt file turned the corruption
    /// into an empty schedule. Nothing but `add_scheduled_job` ever re-inserts,
    /// so every other job was gone. Worse, the arm that logged "NOT copied aside
    /// (the copy failed)" returned the same empty list — destroying outright.
    ///
    /// Fails an implementation whose parse arm returns an empty list, with or
    /// without the copy.
    #[tokio::test]
    async fn a_corrupt_schedule_file_is_refused_not_emptied() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let corrupt = "[{\"id\": \"half-written\", \"cron\": ";
        fs::write(&storage_path, corrupt).unwrap();

        // Every door into the file refuses, and none of them writes.
        let read = read_jobs(&storage_path)
            .await
            .expect_err("an unparseable schedule file is an error, not an empty list");
        assert!(
            read.to_string().contains("not a valid schedule list"),
            "{read}"
        );

        let written = persist_change(&storage_path, |list| {
            list.retain(|job| job.id != "anything");
            Ok(())
        })
        .await;
        assert!(
            written.is_err(),
            "a mutation must abort rather than publish its change over a file it could not read"
        );

        assert_eq!(
            fs::read_to_string(&storage_path).unwrap(),
            corrupt,
            "the corrupt file must be left byte-for-byte intact"
        );

        // A copy is kept aside for the operator, under ONE fixed name however
        // many readers hit the file — every cron tick reconciles, so a
        // timestamped copy would litter the data directory.
        let backup = temp_dir.path().join("schedule.corrupt");
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            corrupt,
            "the unparseable file is copied aside"
        );
        for _ in 0..5 {
            let _ = read_jobs(&storage_path).await;
        }
        let backups: Vec<_> = fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("corrupt"))
            .collect();
        assert_eq!(
            backups.len(),
            1,
            "repeated reads must not leave one backup each: {backups:?}"
        );

        // And a Scheduler over it starts empty WITHOUT touching the file, rather
        // than loading empty and then publishing that emptiness.
        let scheduler = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        assert!(scheduler.list_scheduled_jobs().await.is_empty());
        assert_eq!(
            fs::read_to_string(&storage_path).unwrap(),
            corrupt,
            "loading must not rewrite the file it could not parse"
        );
    }

    /// `add_scheduled_job` checked for a duplicate id in THIS process's map and
    /// then force-replaced the row on disk (`retain(…); push(…)`). A daemon that
    /// has never seen a CLI-created job answers the memory check `false`, so
    /// re-using that id silently overwrote the stranger's cron, its cursor — and,
    /// under `make_copy`, its workflow file.
    ///
    /// Fails an implementation that checks duplicates only in memory.
    #[tokio::test]
    async fn adding_a_job_never_overwrites_one_this_process_has_not_seen() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let theirs = create_test_workflow(temp_dir.path(), "theirs");
        let mine = create_test_workflow(temp_dir.path(), "mine");

        // The daemon starts FIRST, over a file that does not exist yet, so its
        // map is empty and stays empty — it is loaded exactly once. That is the
        // shipped topology, not a contrivance.
        let daemon = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();

        // Then the CLI, in its own process, creates `report`.
        let cli = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        let mut theirs_job = dormant_job("report", &theirs);
        theirs_job.cron = "0 0 9 * * *".to_string();
        cli.add_scheduled_job(theirs_job, false).await.unwrap();

        // The daemon's memory check cannot see it; the file's must.
        let refused = daemon
            .add_scheduled_job(dormant_job("report", &mine), false)
            .await
            .expect_err("an id the FILE already has must be refused, not force-replaced");
        assert!(
            matches!(refused, SchedulerError::JobIdExists(_)),
            "{refused}"
        );

        let on_disk = jobs_on_disk(&storage_path);
        assert_eq!(on_disk.len(), 1);
        assert_eq!(
            on_disk[0].cron, "0 0 9 * * *",
            "the other process's schedule must be untouched"
        );
        assert_eq!(
            on_disk[0].source,
            theirs.to_string_lossy(),
            "and so must its workflow"
        );
    }

    /// `run_count` is a monotonic counter of firings shared by every process
    /// that runs the job. Publishing `max(disk, this process's copy)` loses every
    /// increment the other process made — this process's copy was loaded once —
    /// so a bounded `/loop` under-counts and never reaches `max_runs`.
    ///
    /// Fails the `stored.run_count.max(run_count)` implementation this replaced.
    #[tokio::test]
    async fn each_firing_increments_the_shared_run_count() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let workflow = create_test_workflow(temp_dir.path(), "counted");

        let scheduler = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        scheduler
            .add_scheduled_job(dormant_job("counted", &workflow), false)
            .await
            .unwrap();

        // A stale in-memory copy, exactly as a daemon holds after another
        // process has fired the job twice.
        let stale = dormant_job("counted", &workflow);
        assert_eq!(stale.run_count, 0);

        for expected in 1..=3 {
            let count = persist_run_start(&storage_path, &stale, true)
                .await
                .unwrap()
                .expect("the job is on disk");
            assert_eq!(
                count, expected,
                "every firing must advance the file's counter, stale caller or not"
            );
            assert_eq!(jobs_on_disk(&storage_path)[0].run_count, expected);
        }

        // "Run now" is not a firing and must not consume a `/loop`'s budget.
        let unchanged = persist_run_start(&storage_path, &stale, false)
            .await
            .unwrap()
            .expect("the job is on disk");
        assert_eq!(unchanged, 3);
        assert_eq!(jobs_on_disk(&storage_path)[0].run_count, 3);

        // And a job the file no longer has reports that, rather than silently
        // writing nothing.
        fs::write(&storage_path, "[]").unwrap();
        assert_eq!(
            persist_run_start(&storage_path, &stale, true)
                .await
                .unwrap(),
            None
        );
    }

    /// The `RunSlot::Capped` arm: hitting `max_runs` auto-pauses the job, and
    /// that pause has to REACH DISK or it does not survive a restart — the loop
    /// would resume the moment the daemon came back.
    ///
    /// Fails an implementation whose `Capped` arm returns without persisting
    /// (and, as a bonus, one that has `claim_run_slot` run the job anyway).
    #[tokio::test]
    async fn hitting_the_run_cap_pauses_the_job_on_disk_and_survives_a_restart() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let workflow = create_test_workflow(temp_dir.path(), "capped");

        let scheduler = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        let mut job = dormant_job("capped", &workflow);
        job.cron = "* * * * * *".to_string();
        job.max_runs = Some(1);
        job.run_count = 1; // already at the cap, so the next tick is the capped one
        scheduler.add_scheduled_job(job, false).await.unwrap();

        let mut paused_on_disk = false;
        for _ in 0..40 {
            sleep(Duration::from_millis(100)).await;
            if jobs_on_disk(&storage_path)[0].paused {
                paused_on_disk = true;
                break;
            }
        }
        assert!(
            paused_on_disk,
            "the auto-pause never reached disk, so the loop resumes on the next daemon start"
        );
        assert_eq!(
            jobs_on_disk(&storage_path)[0].run_count,
            1,
            "a capped tick must not consume another run"
        );

        // The restart it has to survive.
        drop(scheduler);
        let restarted = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        assert!(restarted.list_scheduled_jobs().await[0].paused);
    }

    /// The cross-process lock. Two `Scheduler`s over one file is the shipped
    /// topology, and a read-modify-write that is not serialised loses the write
    /// that lands between another writer's read and its own rename.
    ///
    /// Concurrency is genuine — the writers run on a multi-thread runtime and
    /// each holds the lock across a real file read and write. Fails an
    /// implementation whose `persist_change` does not take `ScheduleFileLock`:
    /// with 16 racing edits, interleaved read-modify-writes drop rows.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_writers_do_not_lose_each_others_rows() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        fs::write(&storage_path, "[]").unwrap();

        let mut writers = Vec::new();
        for n in 0..16 {
            let path = storage_path.clone();
            let source = temp_dir.path().join(format!("wf-{n}.yaml"));
            writers.push(tokio::spawn(async move {
                let job = dormant_job(&format!("job-{n}"), &source);
                persist_change(&path, move |list| {
                    // A real read-modify-write: read the list, think, then push.
                    let existing = list.len();
                    list.push(job);
                    assert_eq!(list.len(), existing + 1);
                    Ok(())
                })
                .await
                .unwrap();
            }));
        }
        for writer in writers {
            writer.await.unwrap();
        }

        let mut ids = ids_on_disk(&storage_path);
        ids.sort();
        let mut expected: Vec<String> = (0..16).map(|n| format!("job-{n}")).collect();
        expected.sort();
        assert_eq!(
            ids, expected,
            "a concurrent writer's row was lost: the read-modify-write is not serialised"
        );
    }

    /// The publish is atomic: a reader sees the old file or the new one, never a
    /// half-written one. `write_jobs_file` builds a complete temp file and
    /// renames it over the target.
    ///
    /// ⚠ Asserting only "the content is right afterwards" tests nothing — an
    /// in-place `fs::write` passes that, and so does a torn write once it
    /// finishes. What separates the two is the *mechanism*, so the assertion is
    /// on the mechanism: a rename replaces the directory entry, so the target's
    /// inode changes, while writing in place keeps it. That is Unix-only, hence
    /// the `cfg` — and the leftover check below runs everywhere, because a
    /// "publish" that copies and leaves the temp behind is its own bug.
    ///
    /// Fails an implementation that writes the target in place.
    #[test]
    fn the_publish_is_a_rename_over_a_complete_file() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let workflow = temp_dir.path().join("wf.yaml");
        let previous = vec![dormant_job("before", &workflow)];
        write_jobs_file(&storage_path, &previous).unwrap();

        #[cfg(unix)]
        let inode_before = {
            use std::os::unix::fs::MetadataExt as _;
            fs::metadata(&storage_path).unwrap().ino()
        };

        // Big enough that an in-place write of it is genuinely non-atomic.
        let next: Vec<ScheduledJob> = (0..200)
            .map(|n| dormant_job(&format!("after-{n}"), &workflow))
            .collect();
        write_jobs_file(&storage_path, &next).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let inode_after = fs::metadata(&storage_path).unwrap().ino();
            assert_ne!(
                inode_before, inode_after,
                "the schedule file was written in place: a concurrent reader can see it torn"
            );
        }

        assert_eq!(jobs_on_disk(&storage_path).len(), 200);
        let leftovers: Vec<_> = fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "the temp file must be renamed, not copied and left: {leftovers:?}"
        );
    }

    /// Issue #148's cancel complaint. `kill_running_job` returned `Ok(())`
    /// whenever the cancel-token registry held nothing, and the route turned that
    /// into "Successfully killed running job" — a Stop button that reported
    /// success and cancelled nothing.
    ///
    /// Fails the implementation that ignored a missing token.
    #[tokio::test]
    async fn stopping_a_run_that_cannot_be_stopped_is_an_error() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let workflow = create_test_workflow(temp_dir.path(), "killable");
        let scheduler = Scheduler::new(
            storage_path.clone(),
            Arc::new(SessionManager::new(temp_dir.path().to_path_buf())),
        )
        .await
        .unwrap();
        scheduler
            .add_scheduled_job(dormant_job("killable", &workflow), false)
            .await
            .unwrap();

        // Not running at all: already an error, and still is.
        assert!(scheduler.kill_running_job("killable").await.is_err());

        // Marked running with no token — the state a crashed or foreign run
        // leaves behind, and the one that used to report success.
        {
            let mut jobs = scheduler.jobs.lock().await;
            jobs.get_mut("killable").unwrap().1.currently_running = true;
        }
        let error = scheduler
            .kill_running_job("killable")
            .await
            .expect_err("a cancel that cancelled nothing must not report success");
        assert!(
            error.to_string().contains("Nothing was cancelled"),
            "the caller has to be able to tell: {error}"
        );

        // With a token registered it really does cancel.
        let token = CancellationToken::new();
        register_running_task(&scheduler.running_tasks, "killable", token.clone());
        scheduler.kill_running_job("killable").await.unwrap();
        assert!(token.is_cancelled());
    }

    /// The cancel token and `currently_running` are published under two
    /// independent mutexes, so they have to be written under the SAME `jobs`
    /// lock or there is a window in which a run is visibly running and has no
    /// token to cancel — `kill_running_job`'s old silent `Ok(())`.
    ///
    /// A source-shape pin, because the window is a scheduling race a test cannot
    /// reliably hit. Fails the implementation that registered the token after
    /// the `jobs` guard was dropped.
    #[test]
    fn the_cancel_token_is_registered_under_the_lock_that_publishes_the_flag() {
        let source = include_str!("scheduler.rs");

        let claim = source
            .split("async fn claim_run_slot(")
            .nth(1)
            .and_then(|rest| rest.split("\n/// What [`claim_run_slot`]").next())
            .expect("claim_run_slot production body");
        assert!(
            claim.contains("register_running_task(running_tasks, job_id, token.clone())"),
            "claim_run_slot must register the token inside its own `jobs` lock"
        );

        let run_now = source
            .split("pub async fn run_now(&self, sched_id: &str)")
            .nth(1)
            .and_then(|rest| rest.split("pub async fn pause_schedule").next())
            .expect("run_now production body");
        let registered = run_now
            .find("register_running_task")
            .expect("run_now registers its cancel token");
        let guard_dropped = run_now
            .find("None => return Err(SchedulerError::JobNotFound(sched_id.to_string())),")
            .expect("the end of run_now's jobs-lock block");
        assert!(
            registered < guard_dropped,
            "run_now must register the token inside the `jobs` lock, not after it"
        );
    }

    /// The classifier is only worth anything if `execute_job` actually goes
    /// through it. A source-shape pin, because the alternative — ticking a real
    /// job — needs a real provider and writes into the developer's own session
    /// store (see the note above `privacy_c2_tests`).
    ///
    /// Fails the exact regression it guards: someone restoring the old
    /// `Ok(session.id)` tail.
    #[test]
    fn a_scheduled_run_never_reports_success_without_classifying_its_end() {
        let source = include_str!("scheduler.rs");
        let execute = source
            .split("async fn execute_job(")
            .nth(1)
            .and_then(|rest| rest.split("\n/// Why a scheduled run stopped").next())
            .expect("execute_job production body");

        assert!(
            execute.contains("finish_scheduled_run(session.id, run_token.is_cancelled()"),
            "the run's outcome must be classified from the cancel token and the transcript"
        );
        assert!(
            !execute.contains("Ok(session.id)"),
            "a scheduled run must not return success straight from the session id"
        );
    }

    #[test]
    fn scheduled_stream_errors_remain_failures_after_event_collection() {
        let mut conversation = Conversation::default();
        let error = apply_scheduled_stream_item(
            &mut conversation,
            Err(anyhow::anyhow!("fixture scheduled stream failed")),
        )
        .expect_err("a failed agent stream must fail the scheduled run");
        assert!(error
            .to_string()
            .contains("fixture scheduled stream failed"));
    }

    #[test]
    fn meditation_prompt_uses_the_last_successful_run_as_its_recall_cursor() {
        let cursor = chrono::DateTime::parse_from_rfc3339("2026-08-28T10:11:12Z")
            .unwrap()
            .with_timezone(&Utc);
        let workflow = Workflow::builder()
            .title("Meditation")
            .description("test")
            .instructions("Update Soul")
            .build()
            .unwrap();
        let job = ScheduledJob {
            id: crate::knowledge::soul::MEDITATION_SCHEDULE_ID.to_string(),
            source: String::new(),
            cron: crate::knowledge::soul::MEDITATION_CRON.to_string(),
            last_run: Some(cursor),
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            run_count: 1,
            max_runs: None,
            creator_session_id: None,
            last_error: None,
            owns_source: None,
        };
        let prompt = scheduled_prompt(&job, &workflow);
        assert!(prompt.contains("2026-08-28T10:11:12+00:00"), "{prompt}");
        assert!(
            prompt.contains("last successful Meditation run"),
            "{prompt}"
        );
        assert!(prompt.contains("Scheduled job:"), "{prompt}");
    }
}

/// Issue #56, Task 24 (§9.3 C2 / R5): a scheduled job created from a private
/// chat.
///
/// ⚠ SCOPE, because the plan's sketch of the first test reads
/// `tick(&job).await` and this one does not. `execute_job` builds its agent with
/// `Agent::new()`, whose session manager is the process-wide
/// `SessionManager::instance()` — the developer's REAL `~/.config/biorouter`
/// store — and then drives a real `Agent::reply` against a real provider built
/// from the registry (`versa_azure` needs a UCSF credential). A unit test cannot
/// tick a job without writing sessions into the user's own history and calling a
/// paid endpoint. What it CAN do, and what C2 is actually about, is the two
/// steps that used to be wrong: which provider a run resolves, and whether
/// binding it to the fresh `Scheduled` row is admitted by Gate A.
#[cfg(test)]
mod privacy_c2_tests {
    use super::*;
    use crate::agents::AgentConfig as BioRouterAgentConfig;
    use crate::config::permission::PermissionManager;
    use crate::config::BioRouterMode;
    use crate::conversation::message::Message as ConversationMessage;
    use crate::model::ModelConfig;
    use crate::privacy::{ProviderTier, SessionClassification};
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage, Usage};
    use crate::providers::errors::ProviderError;
    use crate::session::session_manager::SessionType;
    use async_trait::async_trait;
    use rmcp::model::Tool;
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// A real `Provider` whose only interesting property is its tier — the same
    /// fixture shape `agents::agent::gate_a_bind_tests` uses, and for the same
    /// reason: the gates read `tier()`, `get_name()` and `get_model_config()`
    /// and nothing here ever completes a turn.
    struct TieredProvider {
        name: &'static str,
        model: &'static str,
        tier: ProviderTier,
    }

    #[async_trait]
    impl Provider for TieredProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::new("tiered", "Tiered", "", "tiered-model", vec![], "", vec![])
        }

        fn get_name(&self) -> &str {
            self.name
        }

        fn tier(&self) -> ProviderTier {
            self.tier
        }

        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[ConversationMessage],
            _tools: &[Tool],
        ) -> Result<(ConversationMessage, ProviderUsage), ProviderError> {
            Ok((
                ConversationMessage::assistant().with_text("ok"),
                ProviderUsage::new(self.model.to_string(), Usage::default()),
            ))
        }

        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail(self.model)
        }
    }

    fn versa_azure() -> Arc<dyn Provider> {
        Arc::new(TieredProvider {
            name: "versa_azure",
            model: "gpt-5.5",
            tier: ProviderTier::Private,
        })
    }

    /// A job with only the fields these tests care about set.
    fn job_named(id: &str, source: &str, cron: &str) -> ScheduledJob {
        ScheduledJob {
            id: id.to_string(),
            source: source.to_string(),
            cron: cron.to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            run_count: 0,
            max_runs: None,
            creator_session_id: None,
            last_error: None,
            owns_source: None,
        }
    }

    /// An agent over `session_manager`, so a test can bind a provider to a row
    /// the way production does — through Gate A — instead of hand-writing the
    /// columns and asserting against its own fixture.
    fn agent_over(session_manager: Arc<SessionManager>, dir: &Path) -> Agent {
        Agent::with_config(BioRouterAgentConfig::new(
            session_manager,
            Arc::new(PermissionManager::new(dir.to_path_buf())),
            None,
            BioRouterMode::Auto,
        ))
    }

    #[tokio::test]
    async fn a_scheduled_job_from_a_private_session_still_runs() {
        let temp_dir = tempdir().unwrap();
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let agent = agent_over(session_manager.clone(), temp_dir.path());

        // The creating chat: on a private model, and ratcheted private by the
        // turn that ran there.
        let creator = session_manager
            .create_session(PathBuf::from("."), "creator".to_string(), SessionType::User)
            .await
            .unwrap();
        agent
            .update_provider(versa_azure(), &creator.id)
            .await
            .unwrap();
        session_manager
            .update(&creator.id)
            .raise_privacy(SessionClassification::Private, "turn:versa_azure")
            .apply()
            .await
            .unwrap();

        let mut job = job_named("loop-abc", "/does/not/matter.yaml", "0 0 0 1 1 *");
        job.creator_session_id = Some(creator.id.clone());

        // 1. The run resolves the creating chat's model, not the global default.
        let (provider_name, model_config) =
            resolve_scheduled_provider(&job, session_manager.as_ref())
                .await
                .unwrap();
        assert_eq!(provider_name, "versa_azure");
        assert_eq!(model_config.model_name, "gpt-5.5");

        // 2. ...and that name really is a private provider, so the bind below is
        //    not passing for the wrong reason. This is the registry's own
        //    metadata — the same value every UI surface reads.
        let registry_tier = crate::providers::providers()
            .await
            .into_iter()
            .find(|(metadata, _)| metadata.name == provider_name)
            .map(|(metadata, _)| metadata.tier)
            .unwrap_or_default();
        assert_eq!(
            registry_tier,
            ProviderTier::Private,
            "{provider_name} must be a private provider for this test to mean anything"
        );

        // 3. Binding it to the run's fresh `Scheduled` session is admitted. This
        //    is the step that used to fail forever, once the row a scheduled run
        //    is born with carries its creator's classification.
        let run_session = session_manager
            .create_session(
                PathBuf::from("."),
                format!("Scheduled job: {}", job.id),
                SessionType::Scheduled,
            )
            .await
            .unwrap();
        session_manager
            .update(&run_session.id)
            .raise_privacy(SessionClassification::Private, "scheduled:creator")
            .apply()
            .await
            .unwrap();
        let bind = agent.update_provider(versa_azure(), &run_session.id).await;
        assert!(bind.is_ok(), "{bind:?}");

        let row = session_manager
            .get_session(&run_session.id, false)
            .await
            .unwrap();
        assert_eq!(row.provider_name.as_deref(), Some("versa_azure"));
    }

    #[tokio::test]
    async fn a_job_with_no_creating_chat_is_unchanged() {
        // The fallback is the whole of the old behaviour, so a workflow
        // scheduled from the CLI or the schedules route must not start
        // depending on a session that does not exist. Asserted through a
        // creator id that names NO row — the shape a deleted chat leaves — so
        // the assertion does not depend on what this machine's config.yaml says
        // the global provider is.
        let temp_dir = tempdir().unwrap();
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));

        let mut job = job_named("orphan", "/does/not/matter.yaml", "0 0 0 1 1 *");
        job.creator_session_id = Some("no-such-session".to_string());
        let from_missing = resolve_scheduled_provider(&job, session_manager.as_ref()).await;

        job.creator_session_id = None;
        let from_global = resolve_scheduled_provider(&job, session_manager.as_ref()).await;

        match (from_missing, from_global) {
            (Ok((a, _)), Ok((b, _))) => assert_eq!(a, b),
            (Err(_), Err(_)) => {}
            (a, b) => {
                panic!("a missing creator must fall back to the global default: {a:?} vs {b:?}")
            }
        }
    }

    #[tokio::test]
    async fn a_failed_run_leaves_a_job_level_error_on_the_schedule() {
        // The other half of C2: a repeating job that fails every tick used to
        // leave nothing but a log line, and a fresh session per run means there
        // is no chat to carry the error either.
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("schedule.json");
        let session_manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let scheduler = Scheduler::new(storage_path, session_manager).await.unwrap();

        // A cron that will not fire during the test, so the only run is the
        // explicit one below, and a source that does not exist so the run fails
        // before it can reach a provider or the real session store.
        let job = job_named(
            "broken",
            &temp_dir.path().join("gone.yaml").to_string_lossy(),
            "0 0 0 1 1 *",
        );
        scheduler.add_scheduled_job(job, false).await.unwrap();

        assert!(scheduler.run_now("broken").await.is_err());

        let jobs = scheduler.list_scheduled_jobs().await;
        assert!(
            jobs[0].last_error.is_some(),
            "a failed run must leave a job-level error the schedules UI can show"
        );
    }
}
