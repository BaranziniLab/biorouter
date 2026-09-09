use anyhow::{bail, Context, Result};
use biorouter::scheduler::{
    get_default_scheduled_workflows_dir, get_default_scheduler_storage_path, ScheduledJob,
    Scheduler, SchedulerError, RUN_CANCELLED_MARKER,
};
use biorouter::session::SessionManager;
use std::path::Path;
use std::sync::Arc;

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

pub async fn handle_schedule_add(
    schedule_id: String,
    cron: String,
    workflow_source_arg: String, // This is expected to be a file path by the Scheduler
) -> Result<()> {
    validate_cron_expression(&cron)?;

    // The Scheduler's add_scheduled_job will handle copying the workflow from workflow_source_arg
    // to its internal storage and validating the path.
    let job = ScheduledJob {
        id: schedule_id.clone(),
        source: workflow_source_arg.clone(), // Pass the original user-provided path
        cron: cron.clone(),
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

    let scheduler_storage_path =
        get_default_scheduler_storage_path().context("Failed to get scheduler storage path")?;
    let session_manager = Arc::new(SessionManager::instance());
    let scheduler = Scheduler::new(scheduler_storage_path, session_manager)
        .await
        .context("Failed to initialize scheduler")?;

    match scheduler.add_scheduled_job(job, true).await {
        Ok(_) => {
            // The scheduler has copied the workflow to its internal directory.
            // We can reconstruct the likely path for display if needed, or adjust success message.
            let scheduled_workflows_dir = get_default_scheduled_workflows_dir()
                .unwrap_or_else(|_| Path::new("./.biorouter_scheduled_workflows").to_path_buf()); // Fallback for display
            let extension = Path::new(&workflow_source_arg)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("yaml");
            let final_workflow_path =
                scheduled_workflows_dir.join(format!("{}.{}", schedule_id, extension));

            println!("Scheduled job '{}' added.", schedule_id);
            println!("  cron: {}", cron);
            println!("  workflow: {}", final_workflow_path.display());
            Ok(())
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
    let scheduler_storage_path =
        get_default_scheduler_storage_path().context("Failed to get scheduler storage path")?;
    let session_manager = Arc::new(SessionManager::instance());
    let scheduler = Scheduler::new(scheduler_storage_path, session_manager)
        .await
        .context("Failed to initialize scheduler")?;

    let jobs = scheduler.list_scheduled_jobs().await;
    if jobs.is_empty() {
        println!("No scheduled jobs found.");
    } else {
        println!("Scheduled Jobs:");
        for job in jobs {
            println!("{}", render_schedule_entry(&job));
        }
    }
    Ok(())
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
    let scheduler_storage_path =
        get_default_scheduler_storage_path().context("Failed to get scheduler storage path")?;
    let session_manager = Arc::new(SessionManager::instance());
    let scheduler = Scheduler::new(scheduler_storage_path, session_manager)
        .await
        .context("Failed to initialize scheduler")?;

    match scheduler.remove_scheduled_job(&schedule_id, true).await {
        Ok(_) => {
            println!(
                "Scheduled job '{}' and its associated workflow removed.",
                schedule_id
            );
            Ok(())
        }
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
    let scheduler_storage_path =
        get_default_scheduler_storage_path().context("Failed to get scheduler storage path")?;
    let session_manager = Arc::new(SessionManager::instance());
    let scheduler = Scheduler::new(scheduler_storage_path, session_manager)
        .await
        .context("Failed to initialize scheduler")?;

    println!(
        "{}",
        run_now_message(&schedule_id, scheduler.run_now(&schedule_id).await)?
    );
    Ok(())
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
            Ok(format!(
                "Schedule '{}' was stopped before it finished, so no work was recorded and its \
                 last-run cursor was not advanced.",
                schedule_id
            ))
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
}
