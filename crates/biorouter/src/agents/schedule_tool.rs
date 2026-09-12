//! Schedule tool handlers for the biorouter agent
//!
//! This module contains all the handlers for the schedule management platform tool,
//! including job creation, execution, monitoring, and session management.
//!
//! ## Approval posture
//!
//! A schedule is a standing agent run: once it exists, a new session starts on
//! its cron with nobody watching and does whatever its workflow says. That is at
//! least as consequential as writing the workflow file it runs — yet `create`,
//! `delete` and `kill` went straight to the scheduler while
//! `platform__manage_workflow` asked before saving that file (QA 2026-09-10,
//! finding F1: in one Autonomous turn the model asked permission to write a YAML
//! file and then, without a card, set up a daily 02:00 run with `developer`).
//!
//! So every action that changes what the scheduler will do —
//! [`MUTATING_ACTIONS`] — now parks the workflow tool's own proof-backed card
//! ([`super::platform_approval`]), in every permission mode, and the card says in
//! words when the job runs, what it runs and how. The reads —
//! [`READ_ONLY_ACTIONS`] — park nothing. On a daemon that can never obtain a
//! person's proof (`biorouter serve`), the mutating actions are absent from the
//! schema and refused here (SD-8), exactly as the workflow tool's are.

use std::path::Path;
use std::sync::Arc;

use crate::mcp_utils::ToolResult;
use chrono::Utc;
use rmcp::model::{Content, ErrorCode, ErrorData};
use tokio_util::sync::CancellationToken;

use super::platform_approval::{require_platform_approval, PlatformApproval};
use super::platform_tools::PLATFORM_MANAGE_SCHEDULE_TOOL_NAME;
use super::Agent;
use crate::permission::tool_risk::ToolRisk;
use crate::scheduler::ScheduledJob;
use crate::scheduler_trait::SchedulerTrait;
use crate::workflow::Workflow;

/// The actions that change what the scheduler will do, and therefore need a
/// person: each one starts, stops, silences, re-arms or removes a standing run.
pub const MUTATING_ACTIONS: &[&str] = &["create", "run_now", "pause", "unpause", "delete", "kill"];

/// The actions that only read.
pub const READ_ONLY_ACTIONS: &[&str] = &["list", "inspect", "sessions", "session_content"];

/// Every action the model may be offered, given whether a person is reachable.
///
/// The schema's `enum` is derived from this rather than written out beside it,
/// for the reason `workflow_tool::available_actions` gives: a schema listing a
/// verb the handler refuses advertises a call that always fails.
pub fn available_actions(can_ask_a_person: bool) -> Vec<&'static str> {
    let mut actions: Vec<&'static str> = READ_ONLY_ACTIONS.to_vec();
    if can_ask_a_person {
        actions.extend_from_slice(MUTATING_ACTIONS);
    }
    actions
}

/// What a mutating action meets on a daemon that cannot ask anyone.
fn refusal_without_a_person(action: &str) -> ErrorData {
    invalid(format!(
        "`{action}` changes the user's scheduled jobs, so it needs their approval — and this \
         Biorouter is running in a mode that cannot ask anyone (a browser session started by \
         `biorouter serve` has no way to prove a person acted). Read-only actions still work \
         here: {}. To make this change, the user has to do it in the Biorouter desktop app or \
         with the `biorouter` command line.",
        READ_ONLY_ACTIONS.join(", ")
    ))
}

fn invalid(message: impl Into<String>) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, message.into(), None)
}

fn job_id_argument(arguments: &serde_json::Value) -> Result<String, ErrorData> {
    arguments
        .get("job_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| invalid("Missing 'job_id' parameter"))
}

/// Read a workflow file the way a scheduled run will: JSON by extension,
/// otherwise YAML.
fn read_workflow(path: &Path) -> Result<Workflow, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Cannot read workflow file: {e}"))?;
    if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
        serde_json::from_str::<Workflow>(&content)
            .map_err(|e| format!("Invalid JSON workflow: {e}"))
    } else {
        serde_yaml::from_str::<Workflow>(&content)
            .map_err(|e| format!("Invalid YAML workflow: {e}"))
    }
}

/// The name a card gives the workflow a schedule runs: its title, or — for a
/// file that no longer reads — its path, which is still true.
fn workflow_label(source: &str) -> String {
    match read_workflow(Path::new(source)) {
        Ok(workflow) if !workflow.title.trim().is_empty() => workflow.title.trim().to_string(),
        _ => source.to_string(),
    }
}

/// The permission mode a scheduled run inherits, in the words Settings uses.
///
/// A scheduled run builds its agent with `Agent::new()`, which reads the global
/// mode at the moment it runs — so this is "currently", not a promise.
fn permission_mode_label() -> &'static str {
    use crate::config::BioRouterMode;
    match crate::config::Config::global()
        .get_biorouter_mode()
        .unwrap_or(BioRouterMode::Auto)
    {
        BioRouterMode::Auto => "Autonomous",
        BioRouterMode::Approve => "Manual",
        BioRouterMode::SmartApprove => "Smart",
        BioRouterMode::Chat => "Chat only",
    }
}

/// A cron expression in words, for a person deciding whether to allow it.
///
/// Deliberately partial. It names the shapes people actually write — a time
/// every day, on certain weekdays or on one day of the month, and "every N
/// seconds/minutes/hours" — and QUOTES anything else rather than guessing: a
/// wrong sentence on an approval card is worse than none, and the card carries
/// the raw expression beside it either way.
///
/// Clock times are this computer's local time because that is how the engine
/// reads them (`Job::new_async_tz(.., Local)` in `scheduler.rs`), and day-of-week
/// numbers follow the engine's parser (`croner`): 0 and 7 are Sunday.
pub(crate) fn describe_cron(expression: &str) -> String {
    let quoted = || format!("on the cron schedule `{}`", expression.trim());
    let fields: Vec<&str> = expression.split_whitespace().collect();
    let (second, minute, hour, day, month, weekday) = match fields.as_slice() {
        [minute, hour, day, month, weekday] => ("0", *minute, *hour, *day, *month, *weekday),
        [second, minute, hour, day, month, weekday] => {
            (*second, *minute, *hour, *day, *month, *weekday)
        }
        _ => return quoted(),
    };
    let any = |field: &str| field == "*" || field == "?";
    let number = |field: &str, max: u32| field.parse::<u32>().ok().filter(|n| *n <= max);
    let step = |field: &str| {
        field
            .strip_prefix("*/")
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|n| *n > 0)
    };
    let every = |n: u32, unit: &str| {
        if n == 1 {
            format!("every {unit}")
        } else {
            format!("every {n} {unit}s")
        }
    };
    if !any(month) {
        return quoted();
    }

    // A clock time: second, minute and hour all fixed.
    if let (Some(s), Some(m), Some(h)) = (number(second, 59), number(minute, 59), number(hour, 23))
    {
        let time = if s == 0 {
            format!("{h:02}:{m:02}")
        } else {
            format!("{h:02}:{m:02}:{s:02}")
        };
        let clock = format!("at {time}, this computer's local time");
        return match (any(day), any(weekday)) {
            (true, true) => format!("every day {clock}"),
            (true, false) => match weekdays(weekday) {
                Some(days) => format!("every {days} {clock}"),
                None => quoted(),
            },
            (false, true) => match number(day, 31).filter(|d| *d >= 1) {
                Some(d) => format!("on the {} of every month {clock}", ordinal(d)),
                None => quoted(),
            },
            (false, false) => quoted(),
        };
    }

    // An interval: nothing coarser than the field that steps is restricted.
    if !(any(day) && any(weekday)) {
        return quoted();
    }
    match (second, minute, hour) {
        ("*", "*", "*") => "every second".to_string(),
        (s, "*", "*") if step(s).is_some() => every(step(s).unwrap_or(1), "second"),
        ("0", "*", "*") => "every minute".to_string(),
        ("0", m, "*") if step(m).is_some() => every(step(m).unwrap_or(1), "minute"),
        ("0", m, h) if number(m, 59).is_some() && (any(h) || step(h).is_some()) => {
            let m = number(m, 59).unwrap_or(0);
            let hours = if any(h) {
                "every hour".to_string()
            } else {
                every(step(h).unwrap_or(1), "hour")
            };
            if m == 0 {
                format!("{hours}, on the hour")
            } else {
                format!("{hours}, at {m} minutes past")
            }
        }
        _ => quoted(),
    }
}

/// `1-5` → "Monday to Friday", `1,3,5` → "Monday, Wednesday and Friday". `None`
/// for anything this does not recognise (`L`, `#`, steps, a wrapping range) — the
/// caller then quotes the expression rather than naming the wrong days.
fn weekdays(field: &str) -> Option<String> {
    const NAMES: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    // 0..=7 as the engine numbers them — 0 and 7 are both Sunday — or a
    // three-letter name.
    let day = |token: &str| -> Option<usize> {
        const ABBREVIATIONS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];
        let upper = token.to_ascii_uppercase();
        ABBREVIATIONS
            .iter()
            .position(|abbreviation| *abbreviation == upper)
            .or_else(|| token.parse::<usize>().ok().filter(|n| *n <= 7))
    };
    let mut parts = Vec::new();
    for item in field.split(',') {
        match item.split_once('-') {
            Some((from, to)) => {
                let (from, to) = (day(from)?, day(to)?);
                // A range that wraps past Saturday (`5-1`), or runs Sunday to
                // Sunday (`0-7`), is not named; the caller quotes it instead.
                if from >= to || from % 7 == to % 7 {
                    return None;
                }
                parts.push(format!("{} to {}", NAMES[from % 7], NAMES[to % 7]));
            }
            None => parts.push(NAMES[day(item)? % 7].to_string()),
        }
    }
    Some(match parts.as_slice() {
        [] => return None,
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    })
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// The sentence on a by-id action's card, and how risky the action is.
///
/// Each one says what will happen to the user's standing run in their terms,
/// not the tool's. `delete` is graded like the workflow tool's delete; the rest
/// are ordinary changes.
fn job_change_summary(
    action: &str,
    job: &ScheduledJob,
    workflow: &str,
    when: &str,
    mode: &str,
) -> (String, ToolRisk) {
    let id = &job.id;
    match action {
        "run_now" => (
            format!(
                "Run the schedule '{id}' once, right now: the workflow '{workflow}' starts in a \
                 new background session that nobody watches, under your permission mode \
                 (currently {mode}). Its regular schedule ({when}) is unchanged."
            ),
            ToolRisk::Medium,
        ),
        "pause" => (
            format!(
                "Pause the schedule '{id}', which runs the workflow '{workflow}' {when}. It will \
                 not run again until it is resumed."
            ),
            ToolRisk::Medium,
        ),
        "unpause" => (
            format!(
                "Resume the schedule '{id}': the workflow '{workflow}' will run automatically \
                 again {when}, each time in a new background session that nobody watches, under \
                 your permission mode (currently {mode})."
            ),
            ToolRisk::Medium,
        ),
        "delete" => (
            format!(
                "Delete the schedule '{id}', which runs the workflow '{workflow}' {when}. It will \
                 never run again."
            ),
            ToolRisk::High,
        ),
        _ => (
            format!(
                "Stop the run of the schedule '{id}' (the workflow '{workflow}') that is in \
                 progress now. What it has already done stays done; the run is recorded as \
                 stopped, not finished."
            ),
            ToolRisk::Medium,
        ),
    }
}

impl Agent {
    /// Handle schedule management tool calls.
    ///
    /// `creator_session_id` is the chat this tool call is running inside, taken
    /// from `dispatch_tool_call`'s own `session` argument. `create` records it on
    /// the job so a scheduled run resolves the *creating chat's* model rather
    /// than the global default (issue #56, R5) — see
    /// `scheduler::resolve_scheduled_provider`. It is also the chat every
    /// approval card below is shown in.
    /// `cap` is the caller's admitted capability, sampled by `dispatch_tool_call`
    /// in the schedule branch. Two of the actions below read another chat's
    /// content — `session_content` returns a whole transcript, `sessions` returns
    /// LLM-generated titles and working directories — so this tool needs the same
    /// capability its `workspace_*` siblings take. It shipped with none at all,
    /// which is why the parameter looks bolted on: it is.
    /// `cancellation_token` is the turn's, so a Stop releases a card nobody has
    /// answered instead of leaving the turn parked for the card's whole lifetime.
    pub async fn handle_schedule_management(
        &self,
        arguments: serde_json::Value,
        _request_id: String,
        creator_session_id: &str,
        cap: crate::privacy::CallCapability,
        cancellation_token: Option<CancellationToken>,
    ) -> ToolResult<Vec<Content>> {
        let scheduler = self.config.scheduler_service.clone().ok_or_else(|| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                "Scheduler not available".to_string(),
                None,
            )
        })?;

        let action = arguments
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing 'action' parameter".to_string(),
                    None,
                )
            })?
            .to_string();

        // Sampled ONCE per call, as the workflow tool does: two reads of a
        // daemon-level fact can disagree, and the second is the one a change
        // would run behind.
        let can_ask_a_person = crate::pending_user_action::user_proof_available();
        if MUTATING_ACTIONS.contains(&action.as_str()) && !can_ask_a_person {
            return Err(refusal_without_a_person(&action));
        }

        let cancel = cancellation_token.as_ref();
        match action.as_str() {
            "list" => self.handle_list_jobs(scheduler).await,
            "create" => {
                self.handle_create_job(scheduler, arguments, creator_session_id, cancel)
                    .await
            }
            "run_now" => {
                self.handle_run_now(scheduler, arguments, creator_session_id, cancel)
                    .await
            }
            "pause" => {
                self.handle_pause_job(scheduler, arguments, creator_session_id, cancel)
                    .await
            }
            "unpause" => {
                self.handle_unpause_job(scheduler, arguments, creator_session_id, cancel)
                    .await
            }
            "delete" => {
                self.handle_delete_job(scheduler, arguments, creator_session_id, cancel)
                    .await
            }
            "kill" => {
                self.handle_kill_job(scheduler, arguments, creator_session_id, cancel)
                    .await
            }
            "inspect" => self.handle_inspect_job(scheduler, arguments).await,
            "sessions" => self.handle_list_sessions(scheduler, arguments, cap).await,
            "session_content" => self.handle_session_content(arguments, cap).await,
            other => Err(ErrorData::new(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "Unknown action '{other}'. Available actions: {}",
                    available_actions(can_ask_a_person).join(", ")
                ),
                None,
            )),
        }
    }

    /// Find the schedule a by-id action names, say in words what the action will
    /// do to it, and park the card. Returns once the user has allowed it.
    ///
    /// ⚠ The lookup comes BEFORE the card, and so do the refusals the scheduler
    /// would make anyway: a card for a schedule that does not exist, or a Stop for
    /// a run that is not running, spends the user's attention on something that
    /// cannot happen.
    async fn ask_about_job(
        &self,
        scheduler: &Arc<dyn SchedulerTrait>,
        action: &str,
        job_id: &str,
        session_id: &str,
        cancellation_token: Option<&CancellationToken>,
    ) -> Result<(), ErrorData> {
        let job = scheduler
            .list_scheduled_jobs()
            .await
            .into_iter()
            .find(|job| job.id == job_id)
            .ok_or_else(|| {
                invalid(format!(
                    "There is no schedule with the id '{job_id}'. Use action \"list\" to see the \
                     schedules that exist."
                ))
            })?;
        if action == "kill" && !job.currently_running {
            return Err(invalid(format!(
                "The schedule '{job_id}' is not running right now, so there is nothing to stop. \
                 Nothing was changed."
            )));
        }
        if action == "pause" && job.currently_running {
            return Err(invalid(format!(
                "The schedule '{job_id}' is running right now and cannot be paused mid-run; stop \
                 it with action \"kill\" or wait for the run to finish. Nothing was changed."
            )));
        }

        let workflow = workflow_label(&job.source);
        let when = describe_cron(&job.cron);
        let (summary, risk) =
            job_change_summary(action, &job, &workflow, &when, permission_mode_label());
        let card = serde_json::json!({
            "action": action,
            "job_id": job.id,
            "schedule": when,
            "cron_expression": job.cron,
            "paused": job.paused,
            "running_now": job.currently_running,
            "workflow": workflow,
            "workflow_path": job.source,
        });
        require_platform_approval(
            PlatformApproval {
                tool_name: PLATFORM_MANAGE_SCHEDULE_TOOL_NAME,
                action,
                session_id,
                summary: &summary,
                arguments: &card,
                risk,
            },
            cancellation_token,
        )
        .await
    }

    async fn handle_list_jobs(
        &self,
        scheduler: Arc<dyn SchedulerTrait>,
    ) -> ToolResult<Vec<Content>> {
        let jobs = scheduler.list_scheduled_jobs().await;
        let jobs_json = serde_json::to_string_pretty(&jobs).map_err(|e| {
            ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to serialize jobs: {}", e),
                None,
            )
        })?;
        Ok(vec![Content::text(format!(
            "Scheduled Jobs:\n{}",
            jobs_json
        ))])
    }

    async fn handle_create_job(
        &self,
        scheduler: Arc<dyn SchedulerTrait>,
        arguments: serde_json::Value,
        creator_session_id: &str,
        cancellation_token: Option<&CancellationToken>,
    ) -> ToolResult<Vec<Content>> {
        let workflow_path = arguments
            .get("workflow_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing 'workflow_path' parameter".to_string(),
                    None,
                )
            })?;

        let cron_expression = arguments
            .get("cron_expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing 'cron_expression' parameter".to_string(),
                    None,
                )
            })?;

        // Everything below is checked BEFORE the card: an approval for a schedule
        // that cannot be created spends the user's attention on nothing.
        if !Path::new(workflow_path).exists() {
            return Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Workflow file not found: {}", workflow_path),
                None,
            ));
        }
        let workflow = read_workflow(Path::new(workflow_path))
            .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e, None))?;

        // The card is the user's one look at what will run unattended on this
        // schedule, and hidden characters are exactly what a card cannot show —
        // the same sweep that stops `platform__manage_workflow` saving one.
        if workflow.check_for_security_warnings() {
            return Err(invalid(format!(
                "The workflow at {workflow_path} contains hidden Unicode characters that could \
                 carry instructions the user cannot see. It has not been scheduled. Tell the user \
                 the file is suspicious."
            )));
        }

        crate::scheduler::normalize_cron(cron_expression)
            .map_err(|e| invalid(format!("That schedule cannot be created: {e}")))?;

        // ⚠ Always "background", whatever `execution_mode` the call carried. The
        // scheduler has no other mode, and the success message used to echo the
        // argument back — so a card or a result naming a mode the run will not
        // use would be the one untrue sentence in the exchange.
        let when = describe_cron(cron_expression);
        let title = workflow.title.trim();
        let summary = format!(
            "Run the workflow '{title}' automatically {when}, in background mode: every run is a \
             new session that nobody watches, on this chat's model, under your permission mode \
             (currently {}).",
            permission_mode_label()
        );
        // The card shows the WHOLE workflow, not only its path: the path is a
        // pointer at a file anything with a shell could have written a moment
        // ago, and what the user is agreeing to is what that file tells an
        // unattended agent to do.
        let card = serde_json::json!({
            "action": "create",
            "schedule": when,
            "cron_expression": cron_expression,
            "execution_mode": "background",
            "workflow_path": workflow_path,
            "workflow": workflow,
        });
        require_platform_approval(
            PlatformApproval {
                tool_name: PLATFORM_MANAGE_SCHEDULE_TOOL_NAME,
                action: "create",
                session_id: creator_session_id,
                summary: &summary,
                arguments: &card,
                risk: ToolRisk::Medium,
            },
            cancellation_token,
        )
        .await?;

        // Generate unique job ID
        let job_id = format!("agent_created_{}", Utc::now().timestamp());

        let job = crate::scheduler::ScheduledJob {
            id: job_id.clone(),
            source: workflow_path.to_string(),
            cron: cron_expression.to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            run_count: 0,
            max_runs: None,
            // Issue #56 (R5), the third creation surface after `/loop` and
            // `/schedule`. A schedule the agent makes on the user's behalf from
            // a private chat must run on that chat's model, not the user's
            // commercial default — `resolve_scheduled_provider` needs the id to
            // do it.
            //
            // ⚠ Not `session_context::current_session_id()`. That task-local is
            // scoped around a scheduled run and a subagent run and nowhere
            // else — in particular not around `Agent::reply` on the ordinary
            // chat path — so it reads `None` in exactly the case this closes.
            // `dispatch_tool_call` holds the real `Session`; it is passed down.
            creator_session_id: Some(creator_session_id.to_string()),
            last_error: None,
            owns_source: None,
        };

        match scheduler.add_scheduled_job(job, true).await {
            Ok(()) => Ok(vec![Content::text(format!(
                "Successfully created scheduled job '{}' for workflow '{}' with cron expression '{}' in background mode",
                job_id, workflow_path, cron_expression
            ))]),
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to create job: {}", e),
                None,
            )),
        }
    }

    /// Run a scheduled job immediately
    async fn handle_run_now(
        &self,
        scheduler: Arc<dyn SchedulerTrait>,
        arguments: serde_json::Value,
        session_id: &str,
        cancellation_token: Option<&CancellationToken>,
    ) -> ToolResult<Vec<Content>> {
        let job_id = job_id_argument(&arguments)?;
        self.ask_about_job(
            &scheduler,
            "run_now",
            &job_id,
            session_id,
            cancellation_token,
        )
        .await?;

        match scheduler.run_now(&job_id).await {
            Ok(session_id) => Ok(vec![Content::text(format!(
                "Successfully started job '{}'. Session ID: {}",
                job_id, session_id
            ))]),
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to run job: {}", e),
                None,
            )),
        }
    }

    /// Pause a scheduled job
    async fn handle_pause_job(
        &self,
        scheduler: Arc<dyn SchedulerTrait>,
        arguments: serde_json::Value,
        session_id: &str,
        cancellation_token: Option<&CancellationToken>,
    ) -> ToolResult<Vec<Content>> {
        let job_id = job_id_argument(&arguments)?;
        self.ask_about_job(&scheduler, "pause", &job_id, session_id, cancellation_token)
            .await?;

        match scheduler.pause_schedule(&job_id).await {
            Ok(()) => Ok(vec![Content::text(format!(
                "Successfully paused job '{}'",
                job_id
            ))]),
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to pause job: {}", e),
                None,
            )),
        }
    }

    /// Resume a paused scheduled job
    async fn handle_unpause_job(
        &self,
        scheduler: Arc<dyn SchedulerTrait>,
        arguments: serde_json::Value,
        session_id: &str,
        cancellation_token: Option<&CancellationToken>,
    ) -> ToolResult<Vec<Content>> {
        let job_id = job_id_argument(&arguments)?;
        self.ask_about_job(
            &scheduler,
            "unpause",
            &job_id,
            session_id,
            cancellation_token,
        )
        .await?;

        match scheduler.unpause_schedule(&job_id).await {
            Ok(()) => Ok(vec![Content::text(format!(
                "Successfully unpaused job '{}'",
                job_id
            ))]),
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to unpause job: {}", e),
                None,
            )),
        }
    }

    /// Delete a scheduled job
    async fn handle_delete_job(
        &self,
        scheduler: Arc<dyn SchedulerTrait>,
        arguments: serde_json::Value,
        session_id: &str,
        cancellation_token: Option<&CancellationToken>,
    ) -> ToolResult<Vec<Content>> {
        let job_id = job_id_argument(&arguments)?;
        self.ask_about_job(
            &scheduler,
            "delete",
            &job_id,
            session_id,
            cancellation_token,
        )
        .await?;

        match scheduler.remove_scheduled_job(&job_id, true).await {
            Ok(()) => Ok(vec![Content::text(format!(
                "Successfully deleted job '{}'",
                job_id
            ))]),
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to delete job: {}", e),
                None,
            )),
        }
    }

    /// Terminate a currently running job
    async fn handle_kill_job(
        &self,
        scheduler: Arc<dyn SchedulerTrait>,
        arguments: serde_json::Value,
        session_id: &str,
        cancellation_token: Option<&CancellationToken>,
    ) -> ToolResult<Vec<Content>> {
        let job_id = job_id_argument(&arguments)?;
        self.ask_about_job(&scheduler, "kill", &job_id, session_id, cancellation_token)
            .await?;

        match scheduler.kill_running_job(&job_id).await {
            Ok(()) => Ok(vec![Content::text(format!(
                "Successfully killed running job '{}'",
                job_id
            ))]),
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to kill job: {}", e),
                None,
            )),
        }
    }

    /// Get information about a running job
    async fn handle_inspect_job(
        &self,
        scheduler: Arc<dyn SchedulerTrait>,
        arguments: serde_json::Value,
    ) -> ToolResult<Vec<Content>> {
        let job_id = job_id_argument(&arguments)?;

        match scheduler.get_running_job_info(&job_id).await {
            Ok(Some((session_id, start_time))) => {
                let duration = Utc::now().signed_duration_since(start_time);
                Ok(vec![Content::text(format!(
                    "Job '{}' is currently running:\n- Session ID: {}\n- Started: {}\n- Duration: {} seconds",
                    job_id, session_id, start_time.to_rfc3339(), duration.num_seconds()
                ))])
            }
            Ok(None) => Ok(vec![Content::text(format!(
                "Job '{}' is not currently running",
                job_id
            ))]),
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to inspect job: {}", e),
                None,
            )),
        }
    }

    /// List execution sessions for a job.
    ///
    /// Rows the caller may not see are **omitted**, not redacted, for the reason
    /// [`appears_in_list`](crate::privacy::visibility::appears_in_list) states:
    /// a row here carries the session's name — LLM-generated from the
    /// conversation — and its working directory, both content under §11.4. The
    /// filter runs before the rows are rendered, so a private run is absent from
    /// the list rather than present-but-blank.
    async fn handle_list_sessions(
        &self,
        scheduler: Arc<dyn SchedulerTrait>,
        arguments: serde_json::Value,
        cap: crate::privacy::CallCapability,
    ) -> ToolResult<Vec<Content>> {
        let job_id = arguments
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    "Missing 'job_id' parameter".to_string(),
                    None,
                )
            })?;

        let limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        match scheduler.sessions(job_id, limit).await {
            Ok(sessions) => {
                let sessions: Vec<_> = sessions
                    .into_iter()
                    .filter(|(_, session)| {
                        !cap.enforced()
                            || crate::privacy::visibility::appears_in_list(
                                cap.tier(),
                                session.privacy_tier,
                            )
                    })
                    .collect();
                if sessions.is_empty() {
                    Ok(vec![Content::text(format!(
                        "No sessions found for job '{}'",
                        job_id
                    ))])
                } else {
                    let sessions_info: Vec<String> = sessions
                        .into_iter()
                        .map(|(session_name, session)| {
                            format!(
                                "- Session: {} (Messages: {}, Working Dir: {})",
                                session_name,
                                session.conversation.unwrap_or_default().len(),
                                session.working_dir.display()
                            )
                        })
                        .collect();

                    Ok(vec![Content::text(format!(
                        "Sessions for job '{}':\n{}",
                        job_id,
                        sessions_info.join("\n")
                    ))])
                }
            }
            Err(e) => Err(ErrorData::new(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to list sessions: {}", e),
                None,
            )),
        }
    }

    /// Get the full content (metadata and messages) of a specific session.
    ///
    /// ⚠ **This is `workspace_read_conversation` under another name**, and it
    /// shipped without the gate that one has: an arbitrary caller-supplied
    /// `session_id`, `get_session(id, true)`, and the whole session — every
    /// message, tool call and tool response — serialised back to the model. The
    /// §7 READ predicate is asked here through the one adapter, before the
    /// transcript is loaded.
    async fn handle_session_content(
        &self,
        arguments: serde_json::Value,
        cap: crate::privacy::CallCapability,
    ) -> ToolResult<Vec<Content>> {
        let session_id = arguments
            .get("session_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    "Missing 'session_id' parameter".to_string(),
                    None,
                )
            })?;

        // Ahead of the read, and phrased identically for private / unreadable /
        // absent, so the refusal is not an existence oracle for private ids.
        if let Err(refusal) = crate::privacy::visibility::refuse_unless_readable(
            cap,
            &self.config.session_manager,
            session_id,
        )
        .await
        {
            return Err(ErrorData::new(ErrorCode::INVALID_REQUEST, refusal, None));
        }

        let session = match self
            .config
            .session_manager
            .get_session(session_id, true)
            .await
        {
            Ok(metadata) => metadata,
            Err(e) => {
                return Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to read session for '{}': {}", session_id, e),
                    None,
                ));
            }
        };

        // Format the response with metadata and messages
        let metadata_json = match serde_json::to_string_pretty(&session) {
            Ok(json) => json,
            Err(e) => {
                return Err(ErrorData::new(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to serialize metadata: {}", e),
                    None,
                ));
            }
        };

        Ok(vec![Content::text(format!(
            "Session '{}' Content:\n\nSession:\n{}",
            session_id, metadata_json
        ))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::platform_tools::PLATFORM_MANAGE_SCHEDULE_TOOL_NAME;
    use crate::agents::AgentConfig;
    use crate::config::permission::PermissionManager;
    use crate::config::BioRouterMode;
    use crate::conversation::message::{ActionRequiredData, MessageContent};
    use crate::pending_user_action::{
        DecisionAuthority, PendingUserActions, ResolveOutcome, UserActionOutcome,
    };
    use crate::permission::Permission;
    use crate::scheduler::{ScheduledJob, SchedulerError};
    use crate::session::session_manager::{Session, SessionType};
    use crate::session::SessionManager;
    use rmcp::model::{CallToolRequestParams, JsonObject};
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    /// A scheduler that does nothing but remember what it was asked to do.
    #[derive(Default)]
    struct RecordingScheduler {
        jobs: tokio::sync::Mutex<Vec<ScheduledJob>>,
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl RecordingScheduler {
        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }

        /// Every call that would have CHANGED something. Reads are left out:
        /// looking a job up before asking about it is the point, not a leak.
        fn mutations(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| {
                    !["list", "inspect", "sessions"]
                        .iter()
                        .any(|read| call.starts_with(read))
                })
                .cloned()
                .collect()
        }
    }

    #[async_trait::async_trait]
    impl SchedulerTrait for RecordingScheduler {
        async fn add_scheduled_job(
            &self,
            job: ScheduledJob,
            _copy_workflow: bool,
        ) -> Result<(), SchedulerError> {
            self.record(format!("add {}", job.id));
            self.jobs.lock().await.push(job);
            Ok(())
        }

        async fn schedule_workflow(
            &self,
            _workflow_path: PathBuf,
            _cron_schedule: Option<String>,
        ) -> Result<(), SchedulerError> {
            self.record("schedule_workflow".to_string());
            Ok(())
        }

        async fn list_scheduled_jobs(&self) -> Vec<ScheduledJob> {
            self.record("list".to_string());
            self.jobs.lock().await.clone()
        }

        async fn remove_scheduled_job(
            &self,
            id: &str,
            _remove_owned_workflow: bool,
        ) -> Result<(), SchedulerError> {
            self.record(format!("remove {id}"));
            self.jobs.lock().await.retain(|job| job.id != id);
            Ok(())
        }

        async fn pause_schedule(&self, id: &str) -> Result<(), SchedulerError> {
            self.record(format!("pause {id}"));
            Ok(())
        }

        async fn unpause_schedule(&self, id: &str) -> Result<(), SchedulerError> {
            self.record(format!("unpause {id}"));
            Ok(())
        }

        async fn run_now(&self, id: &str) -> Result<String, SchedulerError> {
            self.record(format!("run_now {id}"));
            Ok("scheduled-run-session".to_string())
        }

        async fn sessions(
            &self,
            sched_id: &str,
            _limit: usize,
        ) -> Result<Vec<(String, Session)>, SchedulerError> {
            self.record(format!("sessions {sched_id}"));
            Ok(Vec::new())
        }

        async fn update_schedule(
            &self,
            sched_id: &str,
            _new_cron: String,
        ) -> Result<(), SchedulerError> {
            self.record(format!("update {sched_id}"));
            Ok(())
        }

        async fn kill_running_job(&self, sched_id: &str) -> Result<(), SchedulerError> {
            self.record(format!("kill {sched_id}"));
            Ok(())
        }

        async fn kill_running_job_in_session(
            &self,
            sched_id: &str,
            expected_session_id: Option<&str>,
        ) -> Result<(), SchedulerError> {
            self.record(format!("kill {sched_id} in {expected_session_id:?}"));
            Ok(())
        }

        async fn get_running_job_info(
            &self,
            sched_id: &str,
        ) -> Result<Option<(String, chrono::DateTime<Utc>)>, SchedulerError> {
            self.record(format!("inspect {sched_id}"));
            Ok(None)
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        agent: Arc<Agent>,
        scheduler: Arc<RecordingScheduler>,
        session: Session,
        workflow: PathBuf,
    }

    fn job(id: &str, source: &Path) -> ScheduledJob {
        ScheduledJob {
            id: id.to_string(),
            source: source.to_string_lossy().into_owned(),
            cron: "0 0 2 * * *".to_string(),
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

    /// An agent over a recording scheduler, a chat to call from, and a workflow
    /// file on disk. `jobs` lets a test start with schedules already in place.
    ///
    /// ⚠ The session id is minted here, not by `create_session`. Test databases
    /// each number their sessions from `YYYYMMDD_1`, so every test's first chat
    /// has the SAME id — and the approval registry these tests read is
    /// process-global and keyed by it. Two tests would answer each other's cards.
    async fn fixture(jobs: impl FnOnce(&Path) -> Vec<ScheduledJob>) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let workflow = dir.path().join("nightly-probe.yaml");
        std::fs::write(
            &workflow,
            "title: Nightly probe\ndescription: Echoes a marker\nprompt: echo the probe marker\n",
        )
        .unwrap();
        let scheduler = Arc::new(RecordingScheduler::default());
        *scheduler.jobs.lock().await = jobs(&workflow);
        let session_manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let agent = Arc::new(Agent::with_config(AgentConfig::new(
            session_manager,
            Arc::new(PermissionManager::new(dir.path().to_path_buf())),
            Some(scheduler.clone()),
            BioRouterMode::Auto,
        )));
        let session = Session {
            id: format!("schedule-tool-{}", uuid::Uuid::new_v4()),
            session_type: SessionType::User,
            working_dir: dir.path().to_path_buf(),
            ..Default::default()
        };
        Fixture {
            _dir: dir,
            agent,
            scheduler,
            session,
            workflow,
        }
    }

    /// Call the tool exactly as a chat does — through `dispatch_tool_call` — and
    /// hand back its text or its error, so a test can print either.
    fn spawn_call(
        fx: &Fixture,
        arguments: serde_json::Value,
        cancel: Option<CancellationToken>,
    ) -> tokio::task::JoinHandle<Result<String, String>> {
        let agent = Arc::clone(&fx.agent);
        let session = fx.session.clone();
        tokio::spawn(async move {
            let call = CallToolRequestParams {
                task: None,
                meta: None,
                name: PLATFORM_MANAGE_SCHEDULE_TOOL_NAME.into(),
                arguments: arguments.as_object().cloned(),
            };
            let (_, dispatched) = agent
                .dispatch_tool_call(call, "req-schedule".to_string(), cancel, &session)
                .await;
            let result = dispatched
                .map_err(|e| e.message.to_string())?
                .result
                .await
                .map_err(|e| e.message.to_string())?;
            Ok(result
                .content
                .iter()
                .filter_map(|content| content.as_text().map(|text| text.text.clone()))
                .collect::<Vec<_>>()
                .join("\n"))
        })
    }

    struct Card {
        id: String,
        tool_name: String,
        prompt: String,
        arguments: JsonObject,
    }

    /// The approval card parked for `session_id`, or a panic that says what the
    /// handler did INSTEAD of asking. From the registry alone an early return and
    /// a slow handler look identical, so on a timeout the handler is asked.
    async fn approval_card(
        session_id: &str,
        running: tokio::task::JoinHandle<Result<String, String>>,
    ) -> (Card, tokio::task::JoinHandle<Result<String, String>>) {
        let found = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                for message in PendingUserActions::global().pending_cards_for_session(session_id) {
                    for content in message.content {
                        if let MessageContent::ActionRequired(action) = content {
                            if let ActionRequiredData::ToolConfirmation {
                                id,
                                tool_name,
                                arguments,
                                prompt,
                                ..
                            } = action.data
                            {
                                return Card {
                                    id,
                                    tool_name,
                                    prompt: prompt.unwrap_or_default(),
                                    arguments,
                                };
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        match found {
            Ok(card) => (card, running),
            Err(_) => {
                let outcome = tokio::time::timeout(Duration::from_secs(5), running).await;
                panic!(
                    "no approval card was raised within 10s. The call's own outcome was: \
                     {outcome:#?}\n(an Ok(Ok(..)) here means it went straight through without \
                     asking anyone)"
                );
            }
        }
    }

    fn answer(session_id: &str, card: &Card, outcome: UserActionOutcome) {
        assert_eq!(
            PendingUserActions::global().resolve_in_session(
                session_id,
                &card.id,
                outcome,
                DecisionAuthority::for_test_proven(),
            ),
            ResolveOutcome::Delivered
        );
    }

    fn deny(session_id: &str, card: &Card) {
        answer(
            session_id,
            card,
            UserActionOutcome::Denied {
                permission: Permission::DenyOnce,
            },
        );
    }

    fn approve(session_id: &str, card: &Card) {
        answer(
            session_id,
            card,
            UserActionOutcome::Approved {
                permission: Permission::AllowOnce,
            },
        );
    }

    /// F1 (QA 2026-09-10): `create` put a daily agent run on the user's machine
    /// with no card at all, while saving a workflow FILE through the sibling tool
    /// asked first. The card has to exist, has to be proof-backed like the
    /// workflow tool's, has to say in words when the job runs, what it runs and
    /// how — and nothing may be scheduled while it is unanswered.
    ///
    /// Fails the shipped handler, which went straight to `add_scheduled_job`.
    #[tokio::test]
    async fn create_asks_first_and_the_card_says_when_what_and_how() {
        let fx = fixture(|_| Vec::new()).await;
        let running = spawn_call(
            &fx,
            serde_json::json!({
                "action": "create",
                "workflow_path": fx.workflow.to_string_lossy(),
                "cron_expression": "0 2 * * *",
            }),
            None,
        );

        let (card, running) = approval_card(&fx.session.id, running).await;
        assert_eq!(card.tool_name, PLATFORM_MANAGE_SCHEDULE_TOOL_NAME);
        assert!(
            PendingUserActions::global().requires_user_proof_in_session(&fx.session.id, &card.id),
            "like the workflow tool's, this card must not be answerable by a model"
        );
        for needle in ["every day at 02:00", "Nightly probe", "background"] {
            assert!(
                card.prompt.contains(needle),
                "the card must say `{needle}`: {}",
                card.prompt
            );
        }
        assert_eq!(
            card.arguments
                .get("cron_expression")
                .and_then(|v| v.as_str()),
            Some("0 2 * * *"),
            "the card must still show the exact expression it is asking about"
        );
        assert!(
            fx.scheduler.mutations().is_empty(),
            "nothing may be scheduled while the card is unanswered: {:?}",
            fx.scheduler.mutations()
        );

        deny(&fx.session.id, &card);
        let refused = running
            .await
            .unwrap()
            .expect_err("a declined create is not a success");
        assert!(refused.contains("declined"), "{refused}");
        assert!(
            fx.scheduler.mutations().is_empty(),
            "a declined create scheduled something: {:?}",
            fx.scheduler.mutations()
        );
    }

    /// Issue #56 (R5), moved here from `tests/agent.rs` when the approval
    /// arrived: an integration test cannot grant a proof-backed card, and this
    /// one's whole assertion sits behind that card. A schedule the agent makes
    /// from a chat must remember that chat, so its runs resolve the chat's
    /// model rather than the global default.
    ///
    /// The id comes from `dispatch_tool_call`'s own `session` argument, NOT from
    /// `session_context::current_session_id()`, which is scoped around scheduled
    /// and subagent runs only and reads `None` on the ordinary chat path.
    #[tokio::test]
    async fn an_approved_create_schedules_it_for_the_chat_that_asked() {
        let fx = fixture(|_| Vec::new()).await;
        let running = spawn_call(
            &fx,
            serde_json::json!({
                "action": "create",
                "workflow_path": fx.workflow.to_string_lossy(),
                "cron_expression": "0 0 1 * * *",
            }),
            None,
        );
        let (card, running) = approval_card(&fx.session.id, running).await;
        approve(&fx.session.id, &card);

        let text = running.await.unwrap().expect("an approved create succeeds");
        assert!(
            text.contains("Successfully created scheduled job"),
            "{text}"
        );
        let jobs = fx.scheduler.jobs.lock().await.clone();
        assert_eq!(jobs.len(), 1, "{jobs:?}");
        assert_eq!(
            jobs[0].creator_session_id.as_deref(),
            Some(fx.session.id.as_str()),
            "the schedule must remember the chat it was created from, or its runs fall back \
             to the global default and leave a private chat's work on a public model"
        );
    }

    /// The by-id half of F1. Each of these changes what the user's standing
    /// automation does — runs it outside its schedule, silences it, re-arms it,
    /// removes it, stops it mid-run — and each went straight to the scheduler.
    ///
    /// Fails the shipped handler on the first action: no card is ever raised.
    #[tokio::test]
    async fn every_action_that_changes_a_schedule_asks_first() {
        let fx = fixture(|workflow| {
            let mut running = job("nightly-running", workflow);
            running.currently_running = true;
            let mut paused = job("nightly-paused", workflow);
            paused.paused = true;
            vec![job("nightly", workflow), running, paused]
        })
        .await;

        for (action, id) in [
            ("run_now", "nightly"),
            ("pause", "nightly"),
            ("unpause", "nightly-paused"),
            ("delete", "nightly"),
            ("kill", "nightly-running"),
        ] {
            let running = spawn_call(
                &fx,
                serde_json::json!({ "action": action, "job_id": id }),
                None,
            );
            let (card, running) = approval_card(&fx.session.id, running).await;
            assert_eq!(card.tool_name, PLATFORM_MANAGE_SCHEDULE_TOOL_NAME);
            assert!(
                card.prompt.contains(id),
                "`{action}`'s card must name the schedule it acts on: {}",
                card.prompt
            );
            assert!(
                card.prompt.contains("Nightly probe"),
                "`{action}`'s card must name the workflow the schedule runs: {}",
                card.prompt
            );
            deny(&fx.session.id, &card);
            let refused = running.await.unwrap();
            assert!(
                refused
                    .as_ref()
                    .is_err_and(|text| text.contains("declined")),
                "`{action}` declined must fail and say so: {refused:?}"
            );
        }
        assert!(
            fx.scheduler.mutations().is_empty(),
            "a declined action still reached the scheduler: {:?}",
            fx.scheduler.mutations()
        );
    }

    /// Approval is a gate, not a wall: the same call, allowed, does the thing.
    #[tokio::test]
    async fn an_approved_delete_removes_the_schedule() {
        let fx = fixture(|workflow| vec![job("nightly", workflow)]).await;
        let running = spawn_call(
            &fx,
            serde_json::json!({ "action": "delete", "job_id": "nightly" }),
            None,
        );
        let (card, running) = approval_card(&fx.session.id, running).await;
        approve(&fx.session.id, &card);
        let text = running.await.unwrap().expect("an approved delete succeeds");
        assert!(text.contains("nightly"), "{text}");
        assert_eq!(fx.scheduler.mutations(), vec!["remove nightly".to_string()]);
    }

    /// The guard against over-gating: reading the schedule changes nothing, so
    /// it asks nobody. A card on `list` would teach users to click through.
    #[tokio::test]
    async fn reading_the_schedule_asks_nobody() {
        let fx = fixture(|workflow| vec![job("nightly", workflow)]).await;
        for arguments in [
            serde_json::json!({ "action": "list" }),
            serde_json::json!({ "action": "inspect", "job_id": "nightly" }),
            serde_json::json!({ "action": "sessions", "job_id": "nightly" }),
        ] {
            let outcome = tokio::time::timeout(
                Duration::from_secs(10),
                spawn_call(&fx, arguments.clone(), None),
            )
            .await
            .unwrap_or_else(|_| panic!("{arguments} waited on a person"))
            .unwrap();
            assert!(outcome.is_ok(), "{arguments}: {outcome:?}");
            assert!(
                PendingUserActions::global()
                    .pending_cards_for_session(&fx.session.id)
                    .is_empty(),
                "{arguments} raised a card"
            );
        }
        assert!(fx.scheduler.mutations().is_empty());
    }

    /// A card for something that cannot happen spends the user's attention on
    /// nothing, and teaches them the cards do not matter. Each of these is
    /// refused at once, with no card: a schedule that does not exist, a Stop for
    /// a run that is not running, an expression the engine cannot parse, and a
    /// workflow carrying characters the card could not show.
    #[tokio::test]
    async fn what_cannot_happen_is_refused_without_a_card() {
        let fx = fixture(|workflow| vec![job("nightly", workflow)]).await;
        let hidden = fx._dir.path().join("hidden.yaml");
        std::fs::write(
            &hidden,
            "title: Innocent\ndescription: looks fine\nprompt: \"summarise\u{E0041}\u{E0042}\"\n",
        )
        .unwrap();

        for (arguments, needle) in [
            (
                serde_json::json!({ "action": "delete", "job_id": "no-such-job" }),
                "no schedule with the id 'no-such-job'",
            ),
            (
                serde_json::json!({ "action": "kill", "job_id": "nightly" }),
                "not running right now",
            ),
            (
                serde_json::json!({
                    "action": "create",
                    "workflow_path": fx.workflow.to_string_lossy(),
                    "cron_expression": "every night please",
                }),
                "cannot be created",
            ),
            (
                serde_json::json!({
                    "action": "create",
                    "workflow_path": hidden.to_string_lossy(),
                    "cron_expression": "0 2 * * *",
                }),
                "hidden Unicode",
            ),
        ] {
            let outcome = tokio::time::timeout(
                Duration::from_secs(10),
                spawn_call(&fx, arguments.clone(), None),
            )
            .await
            .unwrap_or_else(|_| panic!("{arguments} waited on a person"))
            .unwrap();
            let refused = outcome.expect_err(&format!("{arguments} must be refused"));
            assert!(refused.contains(needle), "{arguments}: {refused}");
            assert!(
                PendingUserActions::global()
                    .pending_cards_for_session(&fx.session.id)
                    .is_empty(),
                "{arguments} raised a card for something that cannot happen"
            );
        }
        assert!(fx.scheduler.mutations().is_empty());
    }

    /// A Stop in the chat releases a card nobody answered, and nothing changes.
    /// Without the turn's token the turn would sit on the card for its whole
    /// lifetime.
    #[tokio::test]
    async fn a_stop_releases_an_unanswered_card_and_changes_nothing() {
        let fx = fixture(|_| Vec::new()).await;
        let stop = CancellationToken::new();
        let running = spawn_call(
            &fx,
            serde_json::json!({
                "action": "create",
                "workflow_path": fx.workflow.to_string_lossy(),
                "cron_expression": "0 2 * * *",
            }),
            Some(stop.clone()),
        );
        let (_card, running) = approval_card(&fx.session.id, running).await;
        stop.cancel();
        let refused = tokio::time::timeout(Duration::from_secs(10), running)
            .await
            .expect("a Stop must release the parked call")
            .unwrap()
            .expect_err("a stopped create is not a success");
        assert!(refused.contains("Nothing was changed"), "{refused}");
        assert!(fx.scheduler.mutations().is_empty());
    }

    /// The sentence is what the user actually decides on, so the shapes people
    /// write are named — and anything else is quoted rather than guessed, since
    /// a wrong sentence on an approval card is worse than none.
    #[test]
    fn the_cron_sentence_names_the_common_shapes_and_quotes_the_rest() {
        let local = ", this computer's local time";
        for (cron, expected) in [
            ("0 2 * * *", format!("every day at 02:00{local}")),
            ("0 0 2 * * *", format!("every day at 02:00{local}")),
            ("30 0 2 * * *", format!("every day at 02:00:30{local}")),
            (
                "0 9 * * 1-5",
                format!("every Monday to Friday at 09:00{local}"),
            ),
            (
                "30 14 * * 1,3,5",
                format!("every Monday, Wednesday and Friday at 14:30{local}"),
            ),
            ("0 8 * * SUN", format!("every Sunday at 08:00{local}")),
            ("0 8 * * 7", format!("every Sunday at 08:00{local}")),
            (
                "0 8 * * 1-7",
                format!("every Monday to Sunday at 08:00{local}"),
            ),
            (
                "0 0 1 * *",
                format!("on the 1st of every month at 00:00{local}"),
            ),
            (
                "0 6 22 * *",
                format!("on the 22nd of every month at 06:00{local}"),
            ),
            (
                "0 6 13 * *",
                format!("on the 13th of every month at 06:00{local}"),
            ),
            ("*/15 * * * *", "every 15 minutes".to_string()),
            ("* * * * *", "every minute".to_string()),
            ("* * * * * *", "every second".to_string()),
            ("*/5 * * * * *", "every 5 seconds".to_string()),
            ("0 * * * *", "every hour, on the hour".to_string()),
            ("45 * * * *", "every hour, at 45 minutes past".to_string()),
            ("0 */6 * * *", "every 6 hours, on the hour".to_string()),
        ] {
            assert_eq!(describe_cron(cron), expected, "{cron}");
        }

        // Not guessed at: a restricted month, both day fields, the engine's own
        // extensions, a wrapping range, and nonsense.
        for cron in [
            "0 9 1 1 *",
            "0 9 1 * 1",
            "0 9 * * 1#2",
            "0 9 * * 5L",
            "0 9 * * 5-1",
            "0 9 * * 0-7",
            "0 25 * * *",
            "every night",
            "@daily",
        ] {
            assert_eq!(
                describe_cron(cron),
                format!("on the cron schedule `{cron}`"),
                "{cron} must be quoted, not described"
            );
        }
    }

    /// The two sets partition the tool: an action in both would be offered on a
    /// proofless daemon AND refused there, and one in neither has no posture.
    #[test]
    fn the_two_action_sets_partition_the_surface() {
        for action in MUTATING_ACTIONS {
            assert!(
                !READ_ONLY_ACTIONS.contains(action),
                "`{action}` cannot be both"
            );
        }
        assert_eq!(
            available_actions(true).len(),
            MUTATING_ACTIONS.len() + READ_ONLY_ACTIONS.len()
        );
    }

    /// Every action the schema offers has an arm in the handler's `match`, and
    /// the two halves are a schema literal and a `match` that never mention
    /// each other: an action advertised with no arm answers "Unknown action",
    /// which reads to the model as its own mistake.
    #[test]
    fn every_offered_action_has_a_dispatch_arm() {
        let source = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agents/schedule_tool.rs"),
        )
        .expect("the audit must not pass vacuously: this file must be readable");
        let body = source
            .split("match action.as_str() {")
            .nth(1)
            .and_then(|rest| rest.split("other =>").next())
            .expect("the dispatch match must be findable");
        for action in available_actions(true) {
            assert!(
                body.contains(&format!("\"{action}\" =>")),
                "`{action}` is offered but has no arm in `handle_schedule_management`"
            );
        }
    }

    /// SD-8, the schema half: on a daemon that can never ask a person, the six
    /// actions whose card can never be answered are not offered, the reads are,
    /// and the description says why. The handler half is the refusal below.
    #[test]
    fn a_proofless_daemon_is_offered_the_reads_and_told_why_not_the_rest() {
        let offered = |can_ask_a_person: bool| -> Vec<String> {
            let tool = crate::agents::platform_tools::manage_schedule_tool(can_ask_a_person);
            tool.input_schema["properties"]["action"]["enum"]
                .as_array()
                .expect("the action enum")
                .iter()
                .map(|value| value.as_str().unwrap().to_string())
                .collect()
        };
        let without = offered(false);
        let with = offered(true);
        for action in READ_ONLY_ACTIONS {
            assert!(without.iter().any(|a| a == action), "`{action}` only reads");
        }
        for action in MUTATING_ACTIONS {
            assert!(
                !without.iter().any(|a| a == action),
                "`{action}` needs a person and must not be offered without one"
            );
            assert!(with.iter().any(|a| a == action));
        }
        let description = crate::agents::platform_tools::manage_schedule_tool(false)
            .description
            .unwrap_or_default()
            .to_string();
        assert!(
            description.contains("cannot ask the user"),
            "the schema must say why the changes are missing: {description}"
        );

        let refusal = refusal_without_a_person("create").message.to_string();
        assert!(refusal.contains("cannot ask anyone"), "{refusal}");
        for action in READ_ONLY_ACTIONS {
            assert!(
                refusal.contains(action),
                "the refusal should name what still works: {refusal}"
            );
        }
    }
}
