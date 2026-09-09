//! `/loop` and `/schedule` slash commands: recurring prompts on the existing
//! cron scheduler.
//!
//! Both commands wrap the prompt in a minimal workflow file under the
//! scheduled-workflows directory and register a [`ScheduledJob`] with the
//! shared [`SchedulerTrait`] service — the same mechanism behind
//! `biorouter schedule` and the GUI scheduler. Each firing runs as its own
//! `SessionType::Scheduled` session. `/loop` jobs use the `loop-` id prefix
//! and an interval shorthand (`30s`, `5m`, `2h`, `1d`); `/schedule` jobs use
//! the `task-` prefix and additionally accept `@daily`-style shorthands and
//! quoted cron expressions.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};

use crate::conversation::message::Message;
use crate::scheduler::{get_default_scheduled_workflows_dir, ScheduledJob};
use crate::scheduler_trait::SchedulerTrait;

use super::goal::ellipsize;
use super::Agent;

pub(crate) const LOOP_ID_PREFIX: &str = "loop-";
pub(crate) const SCHEDULE_ID_PREFIX: &str = "task-";

/// Default firing cap for `/loop` jobs — the backstop that keeps a loop from
/// running forever. Overridable with `BIOROUTER_LOOP_MAX_RUNS`. Durable
/// `/schedule` jobs are unbounded by design. (`/schedule` is the tool for
/// long-lived recurrence; `/loop` is bounded quick polling.)
const LOOP_DEFAULT_MAX_RUNS: u32 = 100;

/// Resolve the `/loop` firing cap, honoring `BIOROUTER_LOOP_MAX_RUNS`.
fn loop_max_runs() -> u32 {
    std::env::var("BIOROUTER_LOOP_MAX_RUNS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(LOOP_DEFAULT_MAX_RUNS)
}

const SCHEDULE_USAGE: &str = "Usage: `/schedule <spec> <prompt>`, where spec is an interval \
                              (`5m`, `2h`, `1d`), a shorthand (`@hourly`, `@daily`, \
                              `@weekly`, `@monthly`), or a quoted cron expression \
                              (`\"0 9 * * 1\"`).\nManage with `/schedule list`, \
                              `/schedule remove <id>`, `/schedule run <id>`, \
                              `/schedule pause|unpause <id>`, `/schedule sessions <id>`.";

/// Convert an interval shorthand (`30s`, `5m`, `2h`, `1d`; bare numbers are
/// minutes) into a 6-field cron expression plus a human-readable label.
fn interval_to_cron(token: &str) -> Option<(String, String)> {
    let t = token.trim().to_ascii_lowercase();
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let unit = t.get(digits.len()..).unwrap_or("");
    let n: u64 = digits.parse().ok()?;
    if n == 0 {
        return None;
    }
    match unit {
        "s" | "sec" | "secs" => {
            (n <= 59).then(|| (format!("*/{n} * * * * *"), format!("every {n} second(s)")))
        }
        "" | "m" | "min" | "mins" => {
            (n <= 59).then(|| (format!("0 */{n} * * * *"), format!("every {n} minute(s)")))
        }
        "h" | "hr" | "hrs" => {
            (n <= 23).then(|| (format!("0 0 */{n} * * *"), format!("every {n} hour(s)")))
        }
        "d" | "day" | "days" => {
            (n <= 31).then(|| (format!("0 0 0 */{n} * *"), format!("every {n} day(s)")))
        }
        _ => None,
    }
}

/// Convert `@daily`-style shorthands to a 6-field cron expression.
fn shorthand_to_cron(token: &str) -> Option<(String, String)> {
    let cron = match token.to_ascii_lowercase().as_str() {
        "@hourly" => "0 0 * * * *",
        "@daily" | "@midnight" => "0 0 0 * * *",
        "@weekly" => "0 0 0 * * 0",
        "@monthly" => "0 0 0 1 * *",
        "@yearly" | "@annually" => "0 0 0 1 1 *",
        _ => return None,
    };
    Some((cron.to_string(), token.to_ascii_lowercase()))
}

/// Parse `/schedule` creation arguments: a schedule spec (interval shorthand,
/// `@shorthand`, or a quoted 5/6-field cron expression) followed by the prompt.
fn parse_schedule_spec(params_str: &str) -> Result<(String, String, String)> {
    let trimmed = params_str.trim();
    if let Some(rest) = trimmed.strip_prefix('"') {
        let (cron, prompt) = rest
            .split_once('"')
            .ok_or_else(|| anyhow!("Unclosed quote in cron expression"))?;
        let cron = cron.trim().to_string();
        let fields = cron.split_whitespace().count();
        if !(5..=6).contains(&fields) {
            return Err(anyhow!(
                "Cron expression must have 5 or 6 fields, got {fields}: \"{cron}\""
            ));
        }
        let prompt = prompt.trim().to_string();
        return Ok((cron.clone(), format!("cron \"{cron}\""), prompt));
    }

    let (token, prompt) = trimmed
        .split_once(char::is_whitespace)
        .map(|(t, p)| (t, p.trim()))
        .unwrap_or((trimmed, ""));
    let (cron, human) = shorthand_to_cron(token)
        .or_else(|| interval_to_cron(token))
        .ok_or_else(|| {
            anyhow!(
                "Unrecognized schedule '{token}'. Use an interval (30s, 5m, 2h, 1d), a \
                 shorthand (@hourly, @daily, @weekly, @monthly), or a quoted cron \
                 expression like \"0 9 * * *\"."
            )
        })?;
    Ok((cron, human, prompt.to_string()))
}

/// Whether a job's workflow file lives in the managed scheduled-workflows
/// directory (and is therefore safe to delete with the job).
fn owns_workflow_file(source: &str) -> bool {
    get_default_scheduled_workflows_dir()
        .map(|dir| Path::new(source).starts_with(&dir))
        .unwrap_or(false)
}

fn format_job_line(job: &ScheduledJob) -> String {
    let state = if job.currently_running {
        " · running"
    } else if job.paused {
        " · paused"
    } else {
        ""
    };
    let last = job
        .last_run
        .map(|t| format!(" · last run {}", t.format("%Y-%m-%d %H:%M UTC")))
        .unwrap_or_default();
    let runs = match job.max_runs {
        Some(max) => format!(" · {}/{} runs", job.run_count, max),
        None if job.run_count > 0 => format!(" · {} runs", job.run_count),
        None => String::new(),
    };

    format!("- `{}`: cron `{}`{state}{runs}{last}", job.id, job.cron)
}

impl Agent {
    /// The scheduler service: the injected one when available (server/GUI),
    /// otherwise a lazily-created in-process [`crate::scheduler::Scheduler`]
    /// over the default storage, so `/loop` and `/schedule` also work in
    /// plain CLI/TUI sessions.
    pub(crate) async fn scheduler(&self) -> Result<Arc<dyn SchedulerTrait>> {
        if let Some(service) = self.config.scheduler_service.clone() {
            return Ok(service);
        }
        self.fallback_scheduler
            .get_or_try_init(|| async {
                let storage = crate::scheduler::get_default_scheduler_storage_path()?;
                let scheduler =
                    crate::scheduler::Scheduler::new(storage, self.config.session_manager.clone())
                        .await?;
                let service: Arc<dyn SchedulerTrait> = scheduler;
                Ok::<_, anyhow::Error>(service)
            })
            .await
            .cloned()
    }

    /// Write a one-prompt workflow file and register it as a cron job.
    /// `max_runs` bounds total firings (`Some` for `/loop`, `None` for durable
    /// `/schedule`).
    async fn create_recurring_job(
        &self,
        id_prefix: &str,
        cron: &str,
        prompt: &str,
        session_id: &str,
        max_runs: Option<u32>,
    ) -> Result<String> {
        let scheduler = self.scheduler().await?;
        let suffix: String = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(8)
            .collect();
        let id = format!("{id_prefix}{suffix}");
        let dir = get_default_scheduled_workflows_dir()?;
        let path = dir.join(format!("{id}.yaml"));

        let kind = if id_prefix == LOOP_ID_PREFIX {
            "/loop"
        } else {
            "/schedule"
        };
        let workflow = serde_json::json!({
            "version": "1.0.0",
            "title": format!("{kind}: {}", ellipsize(prompt, 60)),
            "description": format!("Recurring {kind} task created in session {session_id}"),
            "prompt": prompt,
        });
        tokio::fs::write(&path, serde_yaml::to_string(&workflow)?).await?;

        let job = ScheduledJob {
            id: id.clone(),
            source: path.to_string_lossy().into_owned(),
            cron: cron.to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            run_count: 0,
            max_runs,
            // Issue #56 (§9.3 C2 / R5). `/loop` and `/schedule` are the two
            // surfaces that DO have a creating chat, and each run resolves its
            // provider from this session before falling back to the global
            // default — so a recurring task started from a private chat keeps
            // running on that chat's model instead of the user's commercial one.
            creator_session_id: Some(session_id.to_string()),
            last_error: None,
            owns_source: None,
        };
        if let Err(e) = scheduler.add_scheduled_job(job, false).await {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(anyhow!("Failed to schedule job: {e}"));
        }
        Ok(id)
    }

    /// `/loop` slash command: `<interval> <prompt>` creates a recurring task;
    /// no args lists active loops; `stop <id|all>` removes them.
    pub(crate) async fn handle_loop_command(
        &self,
        params_str: &str,
        session_id: &str,
    ) -> Result<Option<Message>> {
        let arg = params_str.trim();
        const USAGE: &str = "Usage: `/loop <interval> <prompt>`, with an interval like `30s`, \
                             `5m`, `2h`, `1d`.\nManage with `/loop` (list) and \
                             `/loop stop <id|all>`.";

        if arg.is_empty() || arg == "list" {
            let loops: Vec<ScheduledJob> = self
                .scheduler()
                .await?
                .list_scheduled_jobs()
                .await
                .into_iter()
                .filter(|j| j.id.starts_with(LOOP_ID_PREFIX))
                .collect();
            let text = if loops.is_empty() {
                format!("No active loops.\n\n{USAGE}")
            } else {
                let lines: Vec<String> = loops.iter().map(format_job_line).collect();
                format!(
                    "🔁 Active loops:\n{}\n\nStop with `/loop stop <id|all>`; view runs \
                     with `/schedule sessions <id>`.",
                    lines.join("\n")
                )
            };
            return Ok(Some(Message::assistant().with_text(text)));
        }

        if let Some(target) = arg
            .strip_prefix("stop")
            .or_else(|| arg.strip_prefix("cancel"))
            .or_else(|| arg.strip_prefix("remove"))
            .map(str::trim)
        {
            if target.is_empty() {
                return Ok(Some(Message::assistant().with_text(
                    "Specify which loop to stop: `/loop stop <id|all>` (see `/loop` for ids).",
                )));
            }
            let scheduler = self.scheduler().await?;
            let ids: Vec<String> = if target == "all" {
                scheduler
                    .list_scheduled_jobs()
                    .await
                    .into_iter()
                    .filter(|j| j.id.starts_with(LOOP_ID_PREFIX))
                    .map(|j| j.id)
                    .collect()
            } else {
                vec![target.to_string()]
            };
            if ids.is_empty() {
                return Ok(Some(Message::assistant().with_text("No active loops.")));
            }
            let mut stopped = Vec::new();
            for id in ids {
                match scheduler.remove_scheduled_job(&id, true).await {
                    Ok(()) => stopped.push(id),
                    Err(e) => {
                        return Ok(Some(
                            Message::assistant()
                                .with_text(format!("Could not stop loop '{id}': {e}")),
                        ))
                    }
                }
            }
            return Ok(Some(
                Message::assistant().with_text(format!("Stopped loop(s): {}", stopped.join(", "))),
            ));
        }

        let (token, prompt) = arg
            .split_once(char::is_whitespace)
            .map(|(t, p)| (t, p.trim()))
            .unwrap_or((arg, ""));
        let Some((cron, human)) = interval_to_cron(token) else {
            return Ok(Some(
                Message::assistant()
                    .with_text(format!("Unrecognized interval '{token}'.\n{USAGE}")),
            ));
        };
        if prompt.is_empty() {
            return Ok(Some(
                Message::assistant().with_text(format!("Missing prompt.\n{USAGE}")),
            ));
        }

        let max_runs = loop_max_runs();
        let id = self
            .create_recurring_job(LOOP_ID_PREFIX, &cron, prompt, session_id, Some(max_runs))
            .await?;
        Ok(Some(Message::assistant().with_text(format!(
            "🔁 Loop `{id}` created; runs {human} (cron `{cron}`).\n\
             Prompt: {}\n\n\
             Each iteration runs as its own scheduled session: view them with \
             `/schedule sessions {id}`, stop with `/loop stop {id}`. Iterations fire \
             while a Biorouter process (the GUI's server or an open CLI session) is \
             running. Overlapping runs are skipped (a slow iteration won't stack), and \
             the loop auto-stops after {max_runs} runs. For durable, unbounded \
             recurrence use `/schedule` instead.",
            ellipsize(prompt, 200)
        ))))
    }

    /// `/schedule` slash command: durable recurring tasks on the cron
    /// scheduler. `<spec> <prompt>` creates; `list`, `remove`, `run`,
    /// `pause`, `unpause`, and `sessions` manage existing jobs.
    pub(crate) async fn handle_schedule_command(
        &self,
        params_str: &str,
        session_id: &str,
    ) -> Result<Option<Message>> {
        let arg = params_str.trim();

        if arg.is_empty() || arg == "list" {
            let jobs = self.scheduler().await?.list_scheduled_jobs().await;
            let text = if jobs.is_empty() {
                format!("No scheduled tasks.\n\n{SCHEDULE_USAGE}")
            } else {
                let lines: Vec<String> = jobs.iter().map(format_job_line).collect();
                format!(
                    "📅 Scheduled tasks:\n{}\n\n{SCHEDULE_USAGE}",
                    lines.join("\n")
                )
            };
            return Ok(Some(Message::assistant().with_text(text)));
        }

        let (verb, rest) = arg
            .split_once(char::is_whitespace)
            .map(|(v, r)| (v, r.trim()))
            .unwrap_or((arg, ""));
        if let Some(message) = self.schedule_management(verb, rest).await? {
            return Ok(Some(message));
        }

        // Creation: `<spec> <prompt>`.
        let (cron, human, prompt) = match parse_schedule_spec(arg) {
            Ok(parsed) => parsed,
            Err(e) => {
                return Ok(Some(
                    Message::assistant().with_text(format!("{e}\n\n{SCHEDULE_USAGE}")),
                ))
            }
        };
        if prompt.is_empty() {
            return Ok(Some(
                Message::assistant().with_text(format!("Missing prompt.\n\n{SCHEDULE_USAGE}")),
            ));
        }
        let id = self
            .create_recurring_job(SCHEDULE_ID_PREFIX, &cron, &prompt, session_id, None)
            .await?;
        Ok(Some(Message::assistant().with_text(format!(
            "📅 Schedule `{id}` created; runs {human} (cron `{cron}`).\n\
             Prompt: {}\n\n\
             Each run is its own scheduled session: `/schedule sessions {id}` lists \
             them, `/schedule remove {id}` deletes the task. Runs fire while a \
             Biorouter process (the GUI's server or an open CLI session) is running.",
            ellipsize(&prompt, 200)
        ))))
    }

    /// Management verbs of `/schedule`. Returns `Ok(None)` when `verb` is not
    /// a management verb (the caller then treats the input as a creation spec).
    async fn schedule_management(&self, verb: &str, rest: &str) -> Result<Option<Message>> {
        const VERBS: &[&str] = &[
            "remove", "delete", "run", "pause", "unpause", "resume", "sessions",
        ];
        if !VERBS.contains(&verb) {
            return Ok(None);
        }
        if rest.is_empty() {
            return Ok(Some(Message::assistant().with_text(SCHEDULE_USAGE)));
        }

        match verb {
            "remove" | "delete" => {
                let scheduler = self.scheduler().await?;
                let owns = scheduler
                    .list_scheduled_jobs()
                    .await
                    .iter()
                    .find(|j| j.id == rest)
                    .map(|j| owns_workflow_file(&j.source))
                    .unwrap_or(false);
                return Ok(Some(
                    match scheduler.remove_scheduled_job(rest, owns).await {
                        Ok(()) => {
                            Message::assistant().with_text(format!("Removed schedule `{rest}`."))
                        }
                        Err(e) => Message::assistant()
                            .with_text(format!("Could not remove schedule '{rest}': {e}")),
                    },
                ));
            }
            "run" => {
                let scheduler = self.scheduler().await?;
                let id = rest.to_string();
                tokio::spawn(async move {
                    if let Err(e) = scheduler.run_now(&id).await {
                        tracing::error!("/schedule run '{}' failed: {}", id, e);
                    }
                });
                Ok(Some(Message::assistant().with_text(format!(
                    "▶️ Started `{rest}` in the background; results appear under \
                     `/schedule sessions {rest}`."
                ))))
            }
            "pause" => {
                return Ok(Some(
                    match self.scheduler().await?.pause_schedule(rest).await {
                        Ok(()) => {
                            Message::assistant().with_text(format!("Paused schedule `{rest}`."))
                        }
                        Err(e) => {
                            Message::assistant().with_text(format!("Could not pause '{rest}': {e}"))
                        }
                    },
                ));
            }
            "unpause" | "resume" => {
                return Ok(Some(
                    match self.scheduler().await?.unpause_schedule(rest).await {
                        Ok(()) => {
                            Message::assistant().with_text(format!("Resumed schedule `{rest}`."))
                        }
                        Err(e) => Message::assistant()
                            .with_text(format!("Could not resume '{rest}': {e}")),
                    },
                ));
            }
            "sessions" => {
                let sessions = self
                    .scheduler()
                    .await?
                    .sessions(rest, 5)
                    .await
                    .map_err(|e| anyhow!("Could not list sessions for '{rest}': {e}"))?;
                let text = if sessions.is_empty() {
                    format!("No runs recorded yet for `{rest}`.")
                } else {
                    let lines: Vec<String> = sessions
                        .iter()
                        .map(|(id, s)| format!("- `{}`: {}", id, s.created_at))
                        .collect();
                    format!("Recent runs of `{rest}`:\n{}", lines.join("\n"))
                };
                Ok(Some(Message::assistant().with_text(text)))
            }
            _ => unreachable!("verb membership checked above"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_shorthands_convert_to_six_field_cron() {
        assert_eq!(
            interval_to_cron("30s").unwrap().0,
            "*/30 * * * * *".to_string()
        );
        assert_eq!(
            interval_to_cron("5m").unwrap().0,
            "0 */5 * * * *".to_string()
        );
        assert_eq!(
            interval_to_cron("2h").unwrap().0,
            "0 0 */2 * * *".to_string()
        );
        assert_eq!(
            interval_to_cron("1d").unwrap().0,
            "0 0 0 */1 * *".to_string()
        );
        // Bare numbers are minutes.
        assert_eq!(
            interval_to_cron("15").unwrap().0,
            "0 */15 * * * *".to_string()
        );
    }

    #[test]
    fn invalid_intervals_rejected() {
        for bad in ["0m", "60m", "24h", "weekly", "5x", "", "m"] {
            assert!(interval_to_cron(bad).is_none(), "should reject {bad}");
        }
    }

    #[test]
    fn loop_jobs_are_bounded_and_format_shows_progress() {
        // A /loop default cap exists and is positive (the never-finishes guard).
        assert!(loop_max_runs() > 0);

        let mut job = ScheduledJob {
            id: "loop-abc".to_string(),
            source: "/tmp/loop-abc.yaml".to_string(),
            cron: "0 */5 * * * *".to_string(),
            last_run: None,
            currently_running: false,
            paused: false,
            current_session_id: None,
            process_start_time: None,
            run_count: 3,
            max_runs: Some(100),
            creator_session_id: None,
            last_error: None,
            owns_source: None,
        };
        assert!(format_job_line(&job).contains("3/100 runs"));

        // Durable /schedule jobs are unbounded; no "x/y runs" until they run.
        job.max_runs = None;
        job.run_count = 0;
        assert!(!format_job_line(&job).contains("runs"));
    }

    #[test]
    fn shorthands_convert() {
        assert_eq!(shorthand_to_cron("@daily").unwrap().0, "0 0 0 * * *");
        assert_eq!(shorthand_to_cron("@weekly").unwrap().0, "0 0 0 * * 0");
        assert!(shorthand_to_cron("@fortnightly").is_none());
    }

    #[test]
    fn schedule_spec_parses_quoted_cron() {
        let (cron, _, prompt) = parse_schedule_spec("\"0 9 * * 1\" summarize my inbox").unwrap();
        assert_eq!(cron, "0 9 * * 1");
        assert_eq!(prompt, "summarize my inbox");
    }

    #[test]
    fn schedule_spec_parses_interval_and_shorthand() {
        let (cron, _, prompt) = parse_schedule_spec("@daily check the lab queue").unwrap();
        assert_eq!(cron, "0 0 0 * * *");
        assert_eq!(prompt, "check the lab queue");

        let (cron, _, prompt) = parse_schedule_spec("2h poll the sequencer").unwrap();
        assert_eq!(cron, "0 0 */2 * * *");
        assert_eq!(prompt, "poll the sequencer");
    }

    #[test]
    fn schedule_spec_rejects_bad_cron_field_count() {
        assert!(parse_schedule_spec("\"0 9 *\" do things").is_err());
        assert!(parse_schedule_spec("\"0 9 * * 1").is_err()); // unclosed quote
        assert!(parse_schedule_spec("tomorrow do things").is_err());
    }
}
