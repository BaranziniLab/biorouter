//! `biorouter schedule …`.
//!
//! ## Which process makes the change
//!
//! A running daemon holds the schedule in memory and runs it; the file
//! (`<data>/schedule.json`) is what every process shares. So a change goes to
//! the DAEMON whenever this terminal can reach one — the same way `session send`
//! finds it: `BIOROUTER_SERVER__SECRET_KEY` and `BIOROUTER_PORT` from this
//! shell, and a daemon answering `/status` there. The daemon then registers,
//! lists and fires the job at once.
//!
//! Only when no daemon can be reached does the command write the file with a
//! `Scheduler` of its own — and then it says so, and says when the change will
//! take effect, instead of the "added" it used to print for a job the running
//! app would not see until a restart (QA 2026-09-10, F2). A running daemon now
//! notices such a write itself (`Scheduler::spawn_file_watcher`). An agent's
//! shell is always on this path: the daemon's secret is stripped from every tool
//! child's environment (issue #57), and that is deliberate.
//!
//! ## Who says yes to a change made in the file
//!
//! PR #251 (QA 2026-09-10, F1) made `platform__manage_schedule` park an approval
//! card before it changes what the scheduler will do, because a standing
//! unattended agent run is the most consequential thing an agent can arrange and
//! it was arranging them with nobody asked. This command was the way round that
//! card: a chat with `developer__shell` and no schedule tool ran `biorouter
//! schedule add`, which wrote the file directly, and nobody was asked here
//! either.
//!
//! So a change on the FILE path now needs a person at this terminal
//! ([`Consent`]) — `needs_terminal::require` first, then a confirmation that says
//! in words when the job will run, what it will run, and that every run is an
//! unattended agent. The daemon path is not gated here and does not need to be:
//! reaching it takes `BIOROUTER_SERVER__SECRET_KEY`, which is the operator's own
//! key and is stripped from every tool child — so an agent's shell has neither a
//! daemon to ask nor a terminal to be asked at, which is exactly the case that
//! must fail. A scripted deployment keeps working: give it the key and a running
//! daemon.
//!
//! ⚠ **There is deliberately no `--yes`.** A flag that skips the question is a
//! flag the agent writes into the same command line, which would leave the gate
//! costing an honest operator a keystroke and an agent nothing.

use anyhow::{anyhow, bail, Context, Result};
use biorouter::scheduler::{
    get_default_scheduled_workflows_dir, get_default_scheduler_storage_path, ScheduledJob,
    Scheduler, SchedulerError, EXTERNAL_CHANGE_PICKUP, RUN_CANCELLED_MARKER,
};
use biorouter::session::SessionManager;
use std::path::Path;
use std::sync::Arc;

use super::apps::{configured_port, daemon_ok, DAEMON_HOST};
use super::needs_terminal;
use super::session_watch::{daemon_auth, daemon_json_request, DaemonAuth};

fn validate_cron_expression(cron: &str) -> Result<()> {
    // Basic validation and helpful suggestions
    if cron.trim().is_empty() {
        bail!("Cron expression cannot be empty");
    }

    // Check for common mistakes and provide helpful suggestions
    let parts: Vec<&str> = cron.split_whitespace().collect();

    match parts.len() {
        5 => {
            // Standard 5-field cron (minute hour day month weekday)
            println!("Using standard 5-field cron format: {}", cron);
        }
        6 => {
            // 6-field cron with seconds (second minute hour day month weekday)
            println!("Using 6-field cron format with seconds: {}", cron);
        }
        1 if cron.starts_with('@') => {
            // Shorthand expressions like @hourly, @daily, etc.
            let valid_shorthands = [
                "@yearly",
                "@annually",
                "@monthly",
                "@weekly",
                "@daily",
                "@midnight",
                "@hourly",
            ];
            if valid_shorthands.contains(&cron) {
                println!("Using cron shorthand: {}", cron);
            } else {
                println!(
                    "Unknown cron shorthand '{}'. Valid options: {}",
                    cron,
                    valid_shorthands.join(", ")
                );
            }
        }
        _ => {
            println!("Unusual cron format detected: '{}'", cron);
            println!("   Common formats:");
            println!("   - 5 fields: '0 * * * *' (minute hour day month weekday)");
            println!("   - 6 fields: '0 0 * * * *' (second minute hour day month weekday)");
            println!("   - Shorthand: '@hourly', '@daily', '@weekly', '@monthly'");
        }
    }

    // Provide examples for common scheduling needs
    if cron == "* * * * *" {
        println!("This will run every minute. Did you mean:");
        println!("   - '0 * * * *' for every hour?");
        println!("   - '0 0 * * *' for every day?");
    }

    Ok(())
}

/// Where a `biorouter schedule` change goes.
pub(crate) enum Reach {
    /// A running daemon answered on this terminal's port with this terminal's
    /// key.
    Daemon { auth: DaemonAuth, port: u16 },
    /// No daemon could be reached from here; `why` says why.
    File { why: String },
}

impl Reach {
    /// Decided once per command, from this shell's environment.
    async fn from_environment() -> Self {
        Self::probe(daemon_auth().await.ok(), configured_port()).await
    }

    /// The decision, with its inputs passed in rather than read from the
    /// process environment — so a test can aim it at a fake daemon without
    /// setting `BIOROUTER_PORT`, which other tests in this binary read.
    pub(crate) async fn probe(auth: Option<DaemonAuth>, port: u16) -> Self {
        let Some(auth) = auth else {
            return Reach::File {
                why: "BIOROUTER_SERVER__SECRET_KEY is not set".to_string(),
            };
        };
        if daemon_ok(DAEMON_HOST, port).await {
            Reach::Daemon { auth, port }
        } else {
            Reach::File {
                why: format!("no daemon answered on {DAEMON_HOST}:{port}"),
            }
        }
    }
}

/// Who says yes to a change this terminal makes in the schedule file itself.
///
/// The gate `platform__manage_schedule` gets from `agents/platform_approval.rs`,
/// in the one form a separate process can offer it: that card is parked in the
/// daemon's `PendingUserActions` and shown in the interface, and nothing this
/// command can reach.
pub(crate) enum Consent {
    /// Ask the person at this terminal, and refuse when there is none. What an
    /// agent's tool child gets, because it has no terminal.
    AskThisTerminal { terminal: bool },
    /// A test's stand-in for a person who said yes, so the paths that follow the
    /// gate stay testable. `#[cfg(test)]` for the same reason
    /// [`LocalStore::at_data_dir`] is: it must not exist in a shipped binary.
    #[cfg(test)]
    AlreadyGiven,
}

/// What a refused change says. One sentence per surface it names, because the
/// reader is either a person who needs the alternative or an agent that needs to
/// be told to ask the person.
fn needs_a_person(action: &str) -> String {
    format!(
        "`{action}` would change what Biorouter runs on its own, so it needs a person: run it at an \
         interactive terminal, or do it in Biorouter itself (the Scheduler page, or ask the \
         assistant — `manage_schedule` shows an approval card). Nothing was changed.\nA script \
         can do it without a terminal by asking a running daemon instead: set \
         BIOROUTER_SERVER__SECRET_KEY to that daemon's key (it is deliberately not in a tool's \
         environment) and BIOROUTER_PORT to its port."
    )
}

impl Consent {
    /// `Ok` once the change may go ahead.
    ///
    /// `action` names it for the refusal (`schedule add`); `summary` is what the
    /// person is shown, and must say what will happen in their terms rather than
    /// restate the arguments — the rule `platform_approval`'s card follows.
    fn require(&self, action: &str, summary: &str) -> Result<()> {
        let terminal = match self {
            Consent::AskThisTerminal { terminal } => *terminal,
            #[cfg(test)]
            Consent::AlreadyGiven => return Ok(()),
        };
        // First, and before anything is printed or written: a refusal must not
        // arrive after a wall of output, and `cliclack` under a pipe dies with a
        // bare `Error: not connected` (see `needs_terminal`).
        needs_terminal::require(terminal, &needs_a_person(action))?;
        println!("{summary}");
        if cliclack::confirm("Go ahead?")
            .initial_value(false)
            .interact()?
        {
            Ok(())
        } else {
            bail!("Declined. Nothing was changed.")
        }
    }
}

/// The schedule file this terminal writes when no daemon can be reached, and
/// the session store a `Scheduler` over it needs.
pub(crate) struct LocalStore {
    storage_path: std::path::PathBuf,
    sessions: Arc<SessionManager>,
}

impl LocalStore {
    fn for_this_user() -> Result<Self> {
        Ok(Self {
            storage_path: get_default_scheduler_storage_path()
                .context("Failed to get scheduler storage path")?,
            sessions: Arc::new(SessionManager::instance()),
        })
    }

    async fn scheduler(&self) -> Result<Arc<Scheduler>> {
        Scheduler::new(self.storage_path.clone(), Arc::clone(&self.sessions))
            .await
            .context("Failed to initialize scheduler")
    }

    /// The store under `BIOROUTER_PATH_ROOT`, without the process-wide
    /// `SessionManager::instance()` — which resolves its path once per process,
    /// so a test that initialised it under its own temp root would hand that
    /// (soon deleted) root to every later test in the binary.
    #[cfg(test)]
    fn at_data_dir(data_dir: &Path) -> Self {
        Self {
            storage_path: data_dir.join("schedule.json"),
            sessions: Arc::new(SessionManager::new(data_dir.to_path_buf())),
        }
    }
}

/// How long a schedule request to the daemon may take. Every one of them is a
/// small read or write; `run_now` is the exception and passes no deadline.
const REQUEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// What a change made straight in the schedule file owes the user: why the
/// daemon was not used, and when the change actually takes effect.
///
/// The pickup time is the daemon's own promise
/// ([`EXTERNAL_CHANGE_PICKUP`]), not a number written here, so the sentence
/// cannot drift from what the daemon does.
fn written_to_the_file(why: &str, done: &str, pickup: &str, tail: &str) -> String {
    format!(
        "No running Biorouter could be reached from this terminal ({why}), so {done}. A \
         Biorouter that is already running — the desktop app included — {pickup} {} \
         seconds{tail}",
        EXTERNAL_CHANGE_PICKUP.as_secs()
    )
}

/// A daemon that refused this terminal's key. Reported, never worked around:
/// the user pointed this terminal at that daemon, and writing its file behind
/// its back would be a second, silent answer to a question it already refused.
fn key_refused(port: u16, consequence: &str) -> anyhow::Error {
    anyhow!(
        "The Biorouter on {DAEMON_HOST}:{port} refused this terminal's key (HTTP 401). \
         {consequence}. BIOROUTER_SERVER__SECRET_KEY must be the key that daemon was started \
         with."
    )
}

/// The reason in a daemon's error body: `{"message": …}` when it sent one, the
/// body itself otherwise.
fn daemon_message(body: &str) -> String {
    let body = body.trim();
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            if body.is_empty() {
                "no reason given".to_string()
            } else {
                body.to_string()
            }
        })
}

fn stopped_message(schedule_id: &str) -> String {
    format!(
        "Schedule '{}' was stopped before it finished, so no work was recorded and its last-run \
         cursor was not advanced.",
        schedule_id
    )
}

/// `schedule add` through the running daemon, which copies the workflow, parses
/// the cron and registers the job itself — so its cron engine, its list and the
/// file agree the moment it answers.
async fn add_through_daemon(
    auth: &DaemonAuth,
    port: u16,
    schedule_id: &str,
    cron: &str,
    workflow_source: &str,
) -> Result<String> {
    // The daemon resolves a relative path against ITS working directory.
    let source = std::path::absolute(workflow_source)
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| workflow_source.to_string());
    let body = serde_json::json!({
        "id": schedule_id,
        "workflow_source": source,
        "cron": cron,
    })
    .to_string();
    let (status, answer) = daemon_json_request(
        "POST",
        "/schedule/create",
        Some(&body),
        auth,
        port,
        Some(REQUEST_DEADLINE),
    )
    .await?;
    match status {
        200 => Ok(format!(
            "Scheduled job '{schedule_id}' added to the running Biorouter on {DAEMON_HOST}:{port}; \
             it is live now.\n  cron: {cron}\n  workflow: {source} (the daemon keeps its own copy)"
        )),
        401 => Err(key_refused(port, "Nothing was scheduled")),
        409 => bail!("Error: Job with ID '{}' already exists.", schedule_id),
        _ => bail!(
            "The running Biorouter on {DAEMON_HOST}:{port} did not add '{schedule_id}' (HTTP \
             {status}): {}. Nothing was scheduled.",
            daemon_message(&answer)
        ),
    }
}

async fn remove_through_daemon(auth: &DaemonAuth, port: u16, schedule_id: &str) -> Result<String> {
    let path = format!("/schedule/delete/{}", urlencoding::encode(schedule_id));
    let (status, answer) =
        daemon_json_request("DELETE", &path, None, auth, port, Some(REQUEST_DEADLINE)).await?;
    match status {
        200 | 204 => Ok(format!(
            "Scheduled job '{schedule_id}' removed from the running Biorouter on \
             {DAEMON_HOST}:{port}; it will not run again."
        )),
        401 => Err(key_refused(port, "Nothing was removed")),
        404 => bail!("Error: Job with ID '{}' not found.", schedule_id),
        _ => bail!(
            "The running Biorouter on {DAEMON_HOST}:{port} did not remove '{schedule_id}' (HTTP \
             {status}): {}. Nothing was removed.",
            daemon_message(&answer)
        ),
    }
}

async fn list_through_daemon(auth: &DaemonAuth, port: u16) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct Listing {
        jobs: Vec<ScheduledJob>,
    }
    let (status, answer) = daemon_json_request(
        "GET",
        "/schedule/list",
        None,
        auth,
        port,
        Some(REQUEST_DEADLINE),
    )
    .await?;
    match status {
        200 => {
            let listing: Listing = serde_json::from_str(answer.trim())
                .context("the daemon's schedule list could not be read")?;
            Ok(render_schedule_list(
                &format!("Scheduled Jobs (from the running Biorouter on {DAEMON_HOST}:{port}):"),
                listing.jobs,
            ))
        }
        401 => Err(key_refused(port, "The schedules could not be listed")),
        _ => bail!(
            "The running Biorouter on {DAEMON_HOST}:{port} did not list its schedules (HTTP \
             {status}): {}",
            daemon_message(&answer)
        ),
    }
}

/// `run_now` in the daemon, where the desktop's Stop button can reach the run.
/// No deadline: the route answers when the run ends.
async fn run_now_through_daemon(auth: &DaemonAuth, port: u16, schedule_id: &str) -> Result<String> {
    let path = format!("/schedule/{}/run_now", urlencoding::encode(schedule_id));
    eprintln!(
        "Running '{schedule_id}' in the Biorouter on {DAEMON_HOST}:{port}. Stopping this command \
         does not stop the run; stop it from the app."
    );
    let (status, answer) = daemon_json_request("POST", &path, None, auth, port, None).await?;
    match status {
        200 => {
            let session_id = serde_json::from_str::<serde_json::Value>(answer.trim())
                .ok()
                .and_then(|value| {
                    value
                        .get("session_id")
                        .and_then(|id| id.as_str())
                        .map(str::to_string)
                })
                .ok_or_else(|| {
                    anyhow!("the daemon answered run_now with a body this client could not read")
                })?;
            // The sentinel `ScheduleDetailView.tsx` branches on too.
            if session_id == "CANCELLED" {
                Ok(stopped_message(schedule_id))
            } else {
                Ok(format!(
                    "Successfully triggered schedule '{schedule_id}' on the running Biorouter on \
                     {DAEMON_HOST}:{port}. New session ID: {session_id}"
                ))
            }
        }
        401 => Err(key_refused(port, "Nothing was run")),
        404 => bail!("Error: Job with ID '{}' not found.", schedule_id),
        _ => bail!(
            "Failed to run schedule '{}' now: {}",
            schedule_id,
            daemon_message(&answer)
        ),
    }
}

pub async fn handle_schedule_add(
    schedule_id: String,
    cron: String,
    workflow_source_arg: String, // This is expected to be a file path by the Scheduler
) -> Result<()> {
    validate_cron_expression(&cron)?;
    let report = add_schedule(
        Reach::from_environment().await,
        LocalStore::for_this_user,
        Consent::AskThisTerminal {
            terminal: needs_terminal::prompt_can_run(),
        },
        &schedule_id,
        &cron,
        &workflow_source_arg,
    )
    .await?;
    println!("{report}");
    Ok(())
}

/// `schedule add`, minus the printing: what it did, in the words the terminal
/// prints, or why it could not.
pub(crate) async fn add_schedule(
    reach: Reach,
    local: impl FnOnce() -> Result<LocalStore>,
    consent: Consent,
    schedule_id: &str,
    cron: &str,
    workflow_source_arg: &str,
) -> Result<String> {
    let why = match reach {
        Reach::Daemon { auth, port } => {
            return add_through_daemon(&auth, port, schedule_id, cron, workflow_source_arg).await
        }
        Reach::File { why } => why,
    };

    // ⚠ Before the `Scheduler` is built, and so before the workflow is copied
    // into its store: a refusal must leave nothing behind.
    consent.require(
        "biorouter schedule add",
        &format!(
            "Schedule '{schedule_id}' will run the workflow {workflow_source_arg} automatically \
             {}, in background mode: every run is a new session that nobody watches, under the \
             permission mode Biorouter is configured with.\n  cron: {cron}",
            biorouter::agents::describe_cron(cron)
        ),
    )?;

    // The Scheduler's add_scheduled_job will handle copying the workflow from workflow_source_arg
    // to its internal storage and validating the path.
    let job = ScheduledJob {
        id: schedule_id.to_string(),
        source: workflow_source_arg.to_string(), // Pass the original user-provided path
        cron: cron.to_string(),
        last_run: None,
        currently_running: false,
        paused: false,
        current_session_id: None,
        process_start_time: None,
        run_count: 0,
        max_runs: None,
        // `biorouter schedule add` schedules a workflow file, not a chat.
        creator_session_id: None,
        last_error: None,
        owns_source: None,
    };

    let store = local()?;
    let scheduler = store.scheduler().await?;

    match scheduler.add_scheduled_job(job, true).await {
        Ok(_) => {
            // The scheduler has copied the workflow to its internal directory.
            // We can reconstruct the likely path for display if needed, or adjust success message.
            let scheduled_workflows_dir = get_default_scheduled_workflows_dir()
                .unwrap_or_else(|_| Path::new("./.biorouter_scheduled_workflows").to_path_buf()); // Fallback for display
            let extension = Path::new(workflow_source_arg)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("yaml");
            let final_workflow_path =
                scheduled_workflows_dir.join(format!("{}.{}", schedule_id, extension));

            // ⚠ Not "added". The shipped command printed exactly that for a job
            // a running app would not see until it restarted (QA 2026-09-10,
            // F2). What the user is owed is when it will actually run.
            Ok(format!(
                "Scheduled job '{}' written to {}.\n  cron: {}\n  workflow: {}\n{}",
                schedule_id,
                store.storage_path.display(),
                cron,
                final_workflow_path.display(),
                written_to_the_file(
                    &why,
                    "the job was written to the schedule file directly",
                    "picks it up from that file within",
                    "; if none is running, it first runs the next time Biorouter starts."
                )
            ))
        }
        Err(e) => {
            // No local file to clean up by the CLI in this revised flow.
            match e {
                SchedulerError::JobIdExists(job_id) => {
                    bail!("Error: Job with ID '{}' already exists.", job_id);
                }
                SchedulerError::WorkflowLoadError(msg) => {
                    bail!(
                        "Error with workflow source: {}. Path: {}",
                        msg,
                        workflow_source_arg
                    );
                }
                _ => Err(anyhow::Error::new(e))
                    .context(format!("Failed to add job '{}' to scheduler", schedule_id)),
            }
        }
    }
}

pub async fn handle_schedule_list() -> Result<()> {
    let report = list_schedules(Reach::from_environment().await, LocalStore::for_this_user).await?;
    println!("{report}");
    Ok(())
}

/// `schedule list`, minus the printing. From the daemon when one is reachable —
/// what it lists is what will fire — and from the file otherwise.
pub(crate) async fn list_schedules(
    reach: Reach,
    local: impl FnOnce() -> Result<LocalStore>,
) -> Result<String> {
    if let Reach::Daemon { auth, port } = reach {
        return list_through_daemon(&auth, port).await;
    }
    let scheduler = local()?.scheduler().await?;
    Ok(render_schedule_list(
        "Scheduled Jobs:",
        scheduler.list_scheduled_jobs().await,
    ))
}

fn render_schedule_list(heading: &str, jobs: Vec<ScheduledJob>) -> String {
    if jobs.is_empty() {
        return "No scheduled jobs found.".to_string();
    }
    let mut lines = vec![heading.to_string()];
    lines.extend(jobs.iter().map(render_schedule_entry));
    lines.join("\n")
}

/// One schedule's block in `biorouter schedule list`.
///
/// Split out from the `println!` it used to be so that the `last error` line has
/// somewhere to be asserted. Rendering is the whole behaviour here, and the loop
/// around it needs a populated `~/.config/biorouter` to run at all.
fn render_schedule_entry(job: &ScheduledJob) -> String {
    let status = if job.currently_running {
        "running"
    } else if job.paused {
        "paused"
    } else {
        "idle"
    };

    let mut entry = format!(
        "- {}\n  status: {}\n  cron: {}\n  workflow: {}\n  last run: {}",
        job.id,
        status,
        job.cron,
        job.source, // This source is now the path within scheduled_workflows_dir
        job.last_run
            .map_or_else(|| "Never".to_string(), |dt| dt.to_rfc3339())
    );
    // A schedule mints a fresh session per run, so `last_error` is the only
    // durable home a failure has (issue #56 §9.3 C2) — and since issue #148B a
    // *stopped* or privacy-refused run lands here too, rather than being
    // recorded as a success. The desktop schedule view already renders it;
    // without this line the terminal was the one surface where a job that has
    // been failing since the day it was created still reads as healthy.
    if let Some(error) = job.last_error.as_deref() {
        entry.push_str(&format!("\n  last error: {}", error));
    }
    entry
}

pub async fn handle_schedule_remove(schedule_id: String) -> Result<()> {
    let report = remove_schedule(
        Reach::from_environment().await,
        LocalStore::for_this_user,
        Consent::AskThisTerminal {
            terminal: needs_terminal::prompt_can_run(),
        },
        &schedule_id,
    )
    .await?;
    println!("{report}");
    Ok(())
}

/// `schedule remove`, minus the printing.
pub(crate) async fn remove_schedule(
    reach: Reach,
    local: impl FnOnce() -> Result<LocalStore>,
    consent: Consent,
    schedule_id: &str,
) -> Result<String> {
    let why = match reach {
        Reach::Daemon { auth, port } => {
            return remove_through_daemon(&auth, port, schedule_id).await
        }
        Reach::File { why } => why,
    };
    // Gated for the reason `manage_schedule`'s delete is: a standing run the user
    // set up is theirs, and removing it takes its workflow copy with it.
    consent.require(
        "biorouter schedule remove",
        &format!(
            "Schedule '{schedule_id}' will be deleted, along with the copy of its workflow \
             Biorouter keeps. It will never run again."
        ),
    )?;
    let store = local()?;
    let scheduler = store.scheduler().await?;

    match scheduler.remove_scheduled_job(schedule_id, true).await {
        Ok(_) => Ok(format!(
            "Scheduled job '{}' and its associated workflow removed from {}.\n{}",
            schedule_id,
            store.storage_path.display(),
            written_to_the_file(
                &why,
                "the change was made in the schedule file directly",
                "stops scheduling it within",
                "; a run it has already started finishes first."
            )
        )),
        Err(e) => match e {
            SchedulerError::JobNotFound(job_id) => {
                bail!("Error: Job with ID '{}' not found.", job_id);
            }
            _ => Err(anyhow::Error::new(e)).context(format!(
                "Failed to remove job '{}' from scheduler",
                schedule_id
            )),
        },
    }
}

pub async fn handle_schedule_sessions(schedule_id: String, limit: Option<usize>) -> Result<()> {
    let scheduler_storage_path =
        get_default_scheduler_storage_path().context("Failed to get scheduler storage path")?;
    let session_manager = Arc::new(SessionManager::instance());
    let scheduler = Scheduler::new(scheduler_storage_path, session_manager)
        .await
        .context("Failed to initialize scheduler")?;

    match scheduler.sessions(&schedule_id, limit.unwrap_or(50)).await {
        Ok(sessions) => {
            if sessions.is_empty() {
                println!("No sessions found for schedule ID '{}'.", schedule_id);
            } else {
                println!("Sessions for schedule ID '{}':", schedule_id);
                // sessions is now Vec<(String, SessionMetadata)>
                for (session_name, metadata) in sessions {
                    println!(
                        "  - Session ID: {}, Working Dir: {}, Description: \"{}\", Schedule ID: {:?}",
                        session_name, // Display the session_name as Session ID
                        metadata.working_dir.display(),
                        metadata.name,
                        metadata.schedule_id.as_deref().unwrap_or("N/A")
                    );
                }
            }
        }
        Err(e) => {
            bail!(
                "Failed to get sessions for schedule '{}': {:?}",
                schedule_id,
                e
            );
        }
    }
    Ok(())
}

pub async fn handle_schedule_run_now(schedule_id: String) -> Result<()> {
    let report = run_schedule_now(
        Reach::from_environment().await,
        LocalStore::for_this_user,
        Consent::AskThisTerminal {
            terminal: needs_terminal::prompt_can_run(),
        },
        &schedule_id,
    )
    .await?;
    println!("{report}");
    Ok(())
}

/// `schedule run-now`, minus the printing. In the daemon when one is reachable;
/// otherwise in this terminal's own process, as it always ran.
pub(crate) async fn run_schedule_now(
    reach: Reach,
    local: impl FnOnce() -> Result<LocalStore>,
    consent: Consent,
    schedule_id: &str,
) -> Result<String> {
    let why = match reach {
        Reach::Daemon { auth, port } => {
            return run_now_through_daemon(&auth, port, schedule_id).await
        }
        Reach::File { why } => why,
    };
    // The most immediate of the three: this does not arrange an agent run, it
    // starts one. `manage_schedule`'s `run_now` parks a card for the same reason.
    consent.require(
        "biorouter schedule run-now",
        &format!(
            "The workflow of schedule '{schedule_id}' will run once, now, in this terminal's own \
             process: an agent session that nobody watches, under the permission mode Biorouter \
             is configured with. Its regular schedule is unchanged."
        ),
    )?;
    eprintln!(
        "No running Biorouter could be reached from this terminal ({why}), so '{schedule_id}' \
         runs here, in this terminal."
    );
    let scheduler = local()?.scheduler().await?;
    run_now_message(schedule_id, scheduler.run_now(schedule_id).await)
}

/// What the terminal prints for a finished `schedule run-now`, or the error it
/// exits with.
///
/// Split from the handler for the same reason as [`render_schedule_entry`]: the
/// handler builds a real `Scheduler` over the user's own data directory, and the
/// decision worth testing is this one.
fn run_now_message(schedule_id: &str, result: Result<String, SchedulerError>) -> Result<String> {
    match result {
        Ok(session_id) => Ok(format!(
            "Successfully triggered schedule '{}'. New session ID: {}",
            schedule_id, session_id
        )),
        Err(SchedulerError::JobNotFound(job_id)) => {
            bail!("Error: Job with ID '{}' not found.", job_id)
        }
        // A stopped run is not a failed one (issue #148B). The desktop app got
        // this outcome as its own `CANCELLED` sentinel; the terminal got the
        // catch-all below, which reported the stop as a failure — and did it by
        // Debug-formatting the error, so the user read `AnyhowError(the run was
        // stopped, so it was successfully cancelled …)` rather than the sentence
        // inside it.
        Err(SchedulerError::AnyhowError(ref err))
            if err.to_string().contains(RUN_CANCELLED_MARKER) =>
        {
            Ok(stopped_message(schedule_id))
        }
        // `{}` and not `{:?}`: `SchedulerError`'s `Display` is the whole point of
        // the carefully-worded messages behind it — the privacy barrier's
        // refusal, in particular, explains what the user has to change. `Debug`
        // wraps them in `AnyhowError(…)` and helps nobody.
        Err(e) => bail!("Failed to run schedule '{}' now: {}", schedule_id, e),
    }
}

pub async fn handle_schedule_services_status() -> Result<()> {
    println!("Service management has been removed as Temporal scheduler is no longer supported.");
    println!(
        "The built-in scheduler runs within the biorouter process and requires no external services."
    );
    Ok(())
}

pub async fn handle_schedule_services_stop() -> Result<()> {
    println!("Service management has been removed as Temporal scheduler is no longer supported.");
    println!(
        "The built-in scheduler runs within the biorouter process and requires no external services."
    );
    Ok(())
}

pub async fn handle_schedule_cron_help() -> Result<()> {
    println!("Cron Expression Guide for biorouter Scheduler");
    println!("===========================================");
    println!();

    println!("HOURLY SCHEDULES (Most Common Request):");
    println!("  0 * * * *       - Every hour at minute 0 (e.g., 1:00, 2:00, 3:00...)");
    println!("  30 * * * *      - Every hour at minute 30 (e.g., 1:30, 2:30, 3:30...)");
    println!("  0 */2 * * *     - Every 2 hours at minute 0 (e.g., 2:00, 4:00, 6:00...)");
    println!("  0 */3 * * *     - Every 3 hours at minute 0 (e.g., 3:00, 6:00, 9:00...)");
    println!("  @hourly         - Every hour (same as \"0 * * * *\")");
    println!();

    println!("DAILY SCHEDULES:");
    println!("  0 9 * * *       - Every day at 9:00 AM");
    println!("  30 14 * * *     - Every day at 2:30 PM");
    println!("  0 0 * * *       - Every day at midnight");
    println!("  @daily          - Every day at midnight");
    println!();

    println!("WEEKLY SCHEDULES:");
    println!("  0 9 * * 1       - Every Monday at 9:00 AM");
    println!("  0 17 * * 5      - Every Friday at 5:00 PM");
    println!("  0 0 * * 0       - Every Sunday at midnight");
    println!("  @weekly         - Every Sunday at midnight");
    println!();

    println!("MONTHLY SCHEDULES:");
    println!("  0 9 1 * *       - First day of every month at 9:00 AM");
    println!("  0 0 15 * *      - 15th of every month at midnight");
    println!("  @monthly        - First day of every month at midnight");
    println!();

    println!("CRON FORMAT:");
    println!("  Standard 5-field: minute hour day month weekday");
    println!("  ┌───────────── minute (0 - 59)");
    println!("  │ ┌─────────── hour (0 - 23)");
    println!("  │ │ ┌───────── day of month (1 - 31)");
    println!("  │ │ │ ┌─────── month (1 - 12)");
    println!("  │ │ │ │ ┌───── day of week (0 - 7, Sunday = 0 or 7)");
    println!("  │ │ │ │ │");
    println!("  * * * * *");
    println!();

    println!("SPECIAL CHARACTERS:");
    println!("  *     - Any value (every minute, hour, day, etc.)");
    println!("  */n   - Every nth interval (*/5 = every 5 minutes)");
    println!("  n-m   - Range (1-5 = 1,2,3,4,5)");
    println!("  n,m   - List (1,3,5 = 1 or 3 or 5)");
    println!();

    println!("SHORTHAND EXPRESSIONS:");
    println!("  @yearly   - Once a year (0 0 1 1 *)");
    println!("  @monthly  - Once a month (0 0 1 * *)");
    println!("  @weekly   - Once a week (0 0 * * 0)");
    println!("  @daily    - Once a day (0 0 * * *)");
    println!("  @hourly   - Once an hour (0 * * * *)");
    println!();

    println!("EXAMPLES:");
    println!(
        "  biorouter schedule add --schedule-id hourly-report --cron \"0 * * * *\" --workflow-source report.yaml"
    );
    println!(
        "  biorouter schedule add --schedule-id daily-backup --cron \"@daily\" --workflow-source backup.yaml"
    );
    println!("  biorouter schedule add --schedule-id weekly-summary --cron \"0 9 * * 1\" --workflow-source summary.yaml");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    fn job(id: &str) -> ScheduledJob {
        ScheduledJob {
            id: id.to_string(),
            source: "/tmp/wf.yaml".to_string(),
            cron: "0 0 9 * * *".to_string(),
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

    /// A scheduled run mints a fresh session each time, so `last_error` is the
    /// only durable home a failure has. The desktop schedules view renders it;
    /// the terminal did not, which made `schedule list` the one surface where a
    /// job that had been failing since the day it was created still read as
    /// healthy — status `idle`, and nothing else.
    ///
    /// Fails the implementation that printed only id/status/cron/workflow/last
    /// run.
    #[test]
    fn a_failing_schedule_does_not_read_as_healthy_in_the_terminal() {
        let mut failing = job("nightly");
        failing.last_error = Some(
            "the privacy barrier refused this run's turn; switch it to a private model."
                .to_string(),
        );
        let rendered = render_schedule_entry(&failing);
        assert!(
            rendered.contains("last error:"),
            "the failure has to be visible: {rendered}"
        );
        assert!(
            rendered.contains("switch it to a private model"),
            "and it has to be the actual sentence, not a flag: {rendered}"
        );

        // A healthy job gains no noise.
        assert!(!render_schedule_entry(&job("nightly")).contains("last error"));
    }

    /// Issue #148B, the half the terminal never got. A stopped run reaches the
    /// caller as a `SchedulerError`; the GUI turns it into its own `CANCELLED`
    /// sentinel, while the CLI had no arm for it at all and fell through to the
    /// catch-all — which reported the user's own Stop as a failure, and
    /// Debug-formatted it, so what printed was `AnyhowError(...)`.
    ///
    /// Fails an implementation with no cancellation arm.
    #[test]
    fn stopping_a_run_is_reported_as_a_stop_not_as_a_failure() {
        let stopped = Err(SchedulerError::AnyhowError(anyhow!(
            "the run was stopped, so it {} rather than finishing; the schedule's last-run \
             cursor was not advanced",
            RUN_CANCELLED_MARKER
        )));
        let message = run_now_message("nightly", stopped).expect("a stop is not an error exit");
        assert!(
            message.contains("was stopped before it finished"),
            "{message}"
        );
        assert!(
            !message.contains("AnyhowError"),
            "the Debug wrapper must not reach the user: {message}"
        );
    }

    /// Every other failure keeps its own words. `{:?}` on a `SchedulerError`
    /// prints `AnyhowError(...)` and buries the sentence the privacy barrier
    /// wrote for the person at the keyboard.
    ///
    /// Fails the `{:?}` implementation this replaced.
    #[test]
    fn a_failed_run_reports_its_reason_in_words() {
        let refused = Err(SchedulerError::AnyhowError(anyhow!(
            "the privacy barrier refused this run's turn, so no work was done. This chat is \
             private and the model it is bound to is public; switch it to a private model."
        )));
        let error = run_now_message("nightly", refused).expect_err("a refusal is an error exit");
        let text = format!("{error}");
        assert!(text.contains("switch it to a private model"), "{text}");
        assert!(
            !text.contains("AnyhowError("),
            "Debug formatting buries the message: {text}"
        );
    }

    /// A missing schedule keeps its own, more specific message rather than
    /// being folded into the generic failure above.
    #[test]
    fn a_missing_schedule_says_so() {
        let error = run_now_message(
            "nightly",
            Err(SchedulerError::JobNotFound("nightly".to_string())),
        )
        .expect_err("a missing schedule is an error exit");
        assert!(format!("{error}").contains("not found"), "{error}");
    }

    // -- F2: reaching the daemon ------------------------------------------------

    /// A daemon that answers `/status` the way `biorouterd` does and replies to
    /// every other request with `reply`, recording each one it was sent.
    struct FakeDaemon {
        port: u16,
        requests: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl FakeDaemon {
        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    async fn fake_daemon(reply: fn(&str) -> (u16, String)) -> FakeDaemon {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind((DAEMON_HOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let seen = Arc::clone(&seen);
                tokio::spawn(async move {
                    let request = read_request(&mut socket).await;
                    let (status, body) = if request.starts_with("GET /status ") {
                        (200, "ok".to_string())
                    } else {
                        seen.lock().unwrap().push(request.clone());
                        reply(&request)
                    };
                    let response = format!(
                        "HTTP/1.1 {status} Fake\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        FakeDaemon { port, requests }
    }

    /// One request off the socket: its head, then as many body bytes as its
    /// `Content-Length` promises.
    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;
        let mut raw = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let read = socket.read(&mut chunk).await.unwrap_or(0);
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..read]);
            if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if raw.len() >= end + 4 + length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&raw).into_owned()
    }

    /// A loopback port nothing is listening on.
    async fn unused_port() -> u16 {
        let listener = tokio::net::TcpListener::bind((DAEMON_HOST, 0))
            .await
            .unwrap();
        listener.local_addr().unwrap().port()
    }

    fn body_of(request: &str) -> serde_json::Value {
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap_or_default())
            .unwrap_or_else(|e| panic!("the request body is not JSON ({e}): {request}"))
    }

    fn local_store_is_off_limits() -> Result<LocalStore> {
        panic!(
            "a daemon was reachable, so this terminal must not write the schedule file behind \
             its back — the daemon owns it"
        )
    }

    const CREATED: &str =
        r#"{"id":"qaf-probe","source":"/tmp/probe.yaml","cron":"0 2 * * *","last_run":null}"#;

    /// QA 2026-09-10, F2, the half the CLI owns: `schedule add` built its own
    /// `Scheduler` over the file even when a daemon was running and reachable,
    /// so the daemon never registered the job. When a daemon answers on this
    /// terminal's port with this terminal's key, it makes the change itself —
    /// the one way its cron engine, its list and the file agree at once — and
    /// this terminal never touches the file.
    ///
    /// Fails the shipped command, which wrote the file and sent nothing.
    #[tokio::test]
    async fn a_schedule_added_while_a_daemon_runs_is_registered_with_that_daemon() {
        let dir = tempfile::tempdir().unwrap();
        let workflow = dir.path().join("probe.yaml");
        std::fs::write(
            &workflow,
            "title: Probe\ndescription: d\nprompt: echo probe\n",
        )
        .unwrap();
        let daemon = fake_daemon(|request| {
            if request.starts_with("POST /schedule/create ") {
                (200, CREATED.to_string())
            } else {
                (404, "{}".to_string())
            }
        })
        .await;

        let reach = Reach::probe(Some(DaemonAuth::for_test("s3cret", "")), daemon.port).await;
        assert!(
            matches!(reach, Reach::Daemon { .. }),
            "a daemon that answers /status is reachable"
        );
        let report = add_schedule(
            reach,
            local_store_is_off_limits,
            // ⚠ Not `AlreadyGiven`. The daemon path must not reach the gate at
            // all, and a consent that can never be granted is what proves it:
            // this test fails if the gate is ever moved above the branch.
            Consent::AskThisTerminal { terminal: false },
            "qaf-probe",
            "0 2 * * *",
            &workflow.to_string_lossy(),
        )
        .await
        .expect("the daemon accepted it");

        let requests = daemon.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        let request = &requests[0];
        assert!(
            request.starts_with("POST /schedule/create HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(request.contains("X-Secret-Key: s3cret\r\n"), "{request}");
        let body = body_of(request);
        assert_eq!(body["id"], "qaf-probe");
        assert_eq!(body["cron"], "0 2 * * *");
        assert_eq!(
            body["workflow_source"],
            serde_json::Value::String(workflow.to_string_lossy().into_owned()),
            "the daemon runs in another directory, so the path it is given must be absolute"
        );
        assert!(report.contains("running Biorouter"), "{report}");
        assert!(report.contains(&daemon.port.to_string()), "{report}");
    }

    /// The daemon's refusal is the answer. A key the daemon rejects is a
    /// configuration mistake to report — not a reason to go round the daemon and
    /// write its file anyway.
    #[tokio::test]
    async fn a_daemon_that_refuses_this_terminals_key_is_reported_not_worked_around() {
        let dir = tempfile::tempdir().unwrap();
        let workflow = dir.path().join("probe.yaml");
        std::fs::write(
            &workflow,
            "title: Probe\ndescription: d\nprompt: echo probe\n",
        )
        .unwrap();
        let daemon = fake_daemon(|_| (401, String::new())).await;

        let reach = Reach::probe(Some(DaemonAuth::for_test("wrong", "")), daemon.port).await;
        let error = add_schedule(
            reach,
            local_store_is_off_limits,
            Consent::AskThisTerminal { terminal: false },
            "qaf-probe",
            "0 2 * * *",
            &workflow.to_string_lossy(),
        )
        .await
        .expect_err("a refused key is not a scheduled job");
        let text = format!("{error:#}");
        assert!(text.contains("401"), "{text}");
        assert!(text.contains("BIOROUTER_SERVER__SECRET_KEY"), "{text}");
        assert!(text.contains("Nothing was scheduled"), "{text}");
    }

    /// With no daemon to reach, the job goes into the file — and the terminal
    /// says what that means rather than "added". The shipped command printed
    /// `Scheduled job '…' added.` and nothing else, for a job the running app
    /// would not see until it restarted.
    ///
    /// Fails the shipped command: its report says nothing about when the job
    /// will run.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_schedule_added_with_no_daemon_says_exactly_when_it_will_run() {
        let root = tempfile::tempdir().unwrap();
        let _env = env_lock::lock_env([(
            "BIOROUTER_PATH_ROOT",
            Some(root.path().to_string_lossy().into_owned()),
        )]);
        let data_dir = root.path().join("data");
        let workflow = root.path().join("probe.yaml");
        std::fs::write(
            &workflow,
            "title: Probe\ndescription: d\nprompt: echo probe\n",
        )
        .unwrap();
        let port = unused_port().await;

        let reach = Reach::probe(Some(DaemonAuth::for_test("s3cret", "")), port).await;
        let report = add_schedule(
            reach,
            || Ok(LocalStore::at_data_dir(&data_dir)),
            Consent::AlreadyGiven,
            "qaf-probe",
            "0 2 * * *",
            &workflow.to_string_lossy(),
        )
        .await
        .expect("with no daemon, and a person who said yes, the job goes into the file");

        let on_disk: Vec<ScheduledJob> =
            serde_json::from_str(&std::fs::read_to_string(data_dir.join("schedule.json")).unwrap())
                .unwrap();
        assert_eq!(on_disk.len(), 1);
        assert_eq!(on_disk[0].id, "qaf-probe");

        assert!(
            report.contains(&format!("no daemon answered on {DAEMON_HOST}:{port}")),
            "the report must say why the daemon was not used: {report}"
        );
        assert!(
            report.contains("picks it up from that file within"),
            "the report must say when a running Biorouter will see it: {report}"
        );
        assert!(
            report.contains("next time Biorouter starts"),
            "the report must say what happens when none is running: {report}"
        );
    }

    /// With no key in this shell there is nothing to probe with, and the report
    /// names that — the case an agent's shell is always in, since the daemon's
    /// secret is stripped from every tool's environment (issue #57).
    #[tokio::test]
    async fn without_a_key_there_is_no_daemon_to_reach_and_the_report_says_so() {
        let daemon = fake_daemon(|_| (500, String::new())).await;
        match Reach::probe(None, daemon.port).await {
            Reach::File { why } => assert!(why.contains("BIOROUTER_SERVER__SECRET_KEY"), "{why}"),
            Reach::Daemon { .. } => panic!("without a key the daemon cannot be asked anything"),
        }
        assert!(
            daemon.requests().is_empty(),
            "nothing may be sent without a key"
        );
    }

    /// `schedule remove` reaches the daemon the same way, so the job stops
    /// being listed and stops firing at once.
    ///
    /// Fails the shipped command, which removed it from the file only.
    #[tokio::test]
    async fn a_schedule_removed_while_a_daemon_runs_is_removed_by_that_daemon() {
        let daemon = fake_daemon(|request| {
            if request.starts_with("DELETE /schedule/delete/qaf-probe ") {
                (204, String::new())
            } else {
                (404, "{}".to_string())
            }
        })
        .await;
        let reach = Reach::probe(Some(DaemonAuth::for_test("s3cret", "")), daemon.port).await;
        let report = remove_schedule(
            reach,
            local_store_is_off_limits,
            Consent::AskThisTerminal { terminal: false },
            "qaf-probe",
        )
        .await
        .expect("the daemon removed it");
        assert_eq!(daemon.requests().len(), 1, "{:?}", daemon.requests());
        assert!(report.contains("running Biorouter"), "{report}");
    }

    /// A delete the daemon reports as 404 is "not found", in the same words the
    /// file path has always used.
    #[tokio::test]
    async fn a_schedule_the_daemon_does_not_have_is_not_found() {
        let daemon = fake_daemon(|_| (404, String::new())).await;
        let reach = Reach::probe(Some(DaemonAuth::for_test("s3cret", "")), daemon.port).await;
        let error = remove_schedule(
            reach,
            local_store_is_off_limits,
            Consent::AskThisTerminal { terminal: false },
            "nightly",
        )
        .await
        .expect_err("404 is not a removal");
        assert!(format!("{error}").contains("not found"), "{error}");
    }

    /// `schedule list` asks the daemon too: what it lists is what will fire.
    ///
    /// Fails the shipped command, which read the file with a scheduler of its
    /// own.
    #[tokio::test]
    async fn schedules_are_listed_by_the_daemon_that_runs_them() {
        let daemon = fake_daemon(|request| {
            if request.starts_with("GET /schedule/list ") {
                (200, format!(r#"{{"jobs":[{CREATED}]}}"#))
            } else {
                (404, "{}".to_string())
            }
        })
        .await;
        let reach = Reach::probe(Some(DaemonAuth::for_test("s3cret", "")), daemon.port).await;
        let listing = list_schedules(reach, local_store_is_off_limits)
            .await
            .expect("the daemon listed them");
        assert!(listing.contains("qaf-probe"), "{listing}");
        assert!(listing.contains("running Biorouter"), "{listing}");
    }

    // ──────────────────────────────────────────────────────────────────────
    // The approval gate on the file path (QA 2026-09-12, the CLI half of F1).
    //
    // PR #251 made `platform__manage_schedule` park a card before it changes
    // what the scheduler will do. `biorouter schedule add` from a chat's shell
    // was the way round it: the file path wrote a standing unattended agent run
    // and nobody was asked. An agent's tool child has neither the daemon's key
    // (#57) nor a terminal, so requiring a person at one closes it and leaves a
    // keyed script working.
    // ──────────────────────────────────────────────────────────────────────

    /// The measured defect: with no daemon to reach and nobody at a terminal,
    /// `schedule add` wrote the job and printed a success.
    ///
    /// Fails the shipped command twice: it returns `Ok`, and `schedule.json`
    /// exists.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_schedule_added_with_nobody_to_ask_is_refused_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let _env = env_lock::lock_env([(
            "BIOROUTER_PATH_ROOT",
            Some(root.path().to_string_lossy().into_owned()),
        )]);
        let data_dir = root.path().join("data");
        let workflow = root.path().join("probe.yaml");
        std::fs::write(
            &workflow,
            "title: Probe\ndescription: d\nprompt: echo probe\n",
        )
        .unwrap();

        let error = add_schedule(
            // The agent's shell exactly: no key, so no daemon to ask.
            Reach::probe(None, unused_port().await).await,
            || Ok(LocalStore::at_data_dir(&data_dir)),
            Consent::AskThisTerminal { terminal: false },
            "agent-added",
            "0 2 * * *",
            &workflow.to_string_lossy(),
        )
        .await
        .expect_err("a standing agent run may not be created with nobody asked");

        assert!(
            !data_dir.join("schedule.json").exists(),
            "the refusal must land before anything is written"
        );
        let text = format!("{error:#}");
        assert!(text.contains("needs a person"), "{text}");
        assert!(
            text.contains("manage_schedule"),
            "the refusal must name the surface that does ask: {text}"
        );
        assert!(
            text.contains("BIOROUTER_SERVER__SECRET_KEY"),
            "and the one a script can use: {text}"
        );
        assert!(text.contains("Nothing was changed"), "{text}");
    }

    /// `main` gives the refusal exit 2 and prints the sentence alone, and it can
    /// only do that by downcasting — so the type has to survive the `?`.
    #[tokio::test]
    #[serial_test::serial]
    async fn the_refusal_is_the_needs_a_terminal_one_so_main_exits_2() {
        let root = tempfile::tempdir().unwrap();
        let _env = env_lock::lock_env([(
            "BIOROUTER_PATH_ROOT",
            Some(root.path().to_string_lossy().into_owned()),
        )]);
        let workflow = root.path().join("probe.yaml");
        std::fs::write(
            &workflow,
            "title: Probe\ndescription: d\nprompt: echo probe\n",
        )
        .unwrap();
        let error = add_schedule(
            Reach::probe(None, unused_port().await).await,
            || Ok(LocalStore::at_data_dir(&root.path().join("data"))),
            Consent::AskThisTerminal { terminal: false },
            "agent-added",
            "0 2 * * *",
            &workflow.to_string_lossy(),
        )
        .await
        .unwrap_err();
        assert!(
            error
                .downcast_ref::<needs_terminal::NeedsTerminal>()
                .is_some(),
            "{error:#}"
        );
    }

    /// The other two file-path mutations are gated as well: `manage_schedule`
    /// parks a card for delete and run_now, and run_now is the one that does not
    /// arrange an unattended agent run but starts one.
    ///
    /// Fails the shipped command, which ran and deleted with nobody asked.
    #[tokio::test]
    #[serial_test::serial]
    async fn removing_and_running_a_schedule_need_a_person_too() {
        let root = tempfile::tempdir().unwrap();
        let _env = env_lock::lock_env([(
            "BIOROUTER_PATH_ROOT",
            Some(root.path().to_string_lossy().into_owned()),
        )]);
        let data_dir = root.path().join("data");
        let port = unused_port().await;

        for error in [
            remove_schedule(
                Reach::probe(None, port).await,
                || panic!("the gate must refuse before a Scheduler is built"),
                Consent::AskThisTerminal { terminal: false },
                "nightly",
            )
            .await
            .expect_err("a delete with nobody asked is refused"),
            run_schedule_now(
                Reach::probe(None, port).await,
                || panic!("the gate must refuse before a Scheduler is built"),
                Consent::AskThisTerminal { terminal: false },
                "nightly",
            )
            .await
            .expect_err("starting an unattended agent run with nobody asked is refused"),
        ] {
            let text = format!("{error:#}");
            assert!(text.contains("needs a person"), "{text}");
        }
        assert!(
            !data_dir.join("schedule.json").exists(),
            "nothing may be written on either path"
        );
    }

    /// The sentence the person reads has to say when the job runs, and it says it
    /// in the same words the `manage_schedule` card does — one describer, not
    /// two, so the terminal and the card cannot describe one cron differently.
    #[test]
    fn the_question_says_when_the_job_will_run_in_the_cards_own_words() {
        assert_eq!(
            biorouter::agents::describe_cron("0 2 * * *"),
            "every day at 02:00, this computer's local time"
        );
    }

    /// `schedule run-now` runs the job IN the daemon when there is one — where
    /// the desktop's Stop button can reach it — rather than in this terminal.
    ///
    /// Fails the shipped command, which ran it here.
    #[tokio::test]
    async fn run_now_runs_in_the_daemon_that_holds_the_schedule() {
        let daemon = fake_daemon(|request| {
            if request.starts_with("POST /schedule/qaf-probe/run_now ") {
                (200, r#"{"session_id":"20260911_42"}"#.to_string())
            } else {
                (404, "{}".to_string())
            }
        })
        .await;
        let reach = Reach::probe(Some(DaemonAuth::for_test("s3cret", "")), daemon.port).await;
        let report = run_schedule_now(
            reach,
            local_store_is_off_limits,
            Consent::AskThisTerminal { terminal: false },
            "qaf-probe",
        )
        .await
        .expect("the daemon ran it");
        assert!(report.contains("20260911_42"), "{report}");
    }
}
