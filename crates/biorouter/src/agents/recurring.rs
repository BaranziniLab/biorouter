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

    /// Issue #56. What a slash command run in this chat records as the standing of
    /// whoever armed a schedule (`ScheduledJob::armed_with_private_reach`):
    /// `Some(true)` when this chat runs a private model — whoever sent the
    /// message reached a private chat, which the turn gate does not admit on a
    /// public caller's word — and `None` otherwise.
    ///
    /// ⚠ **Never `Some(false)`.** A public chat's schedule runs on that chat's
    /// public model, and moving the chat onto a private model later is the
    /// person's act (`TierRaiseNeedsUser`); a `false` here would refuse the
    /// schedule's runs from then on. `None` refuses only the one move nobody chose
    /// — the private default standing in for a creator chat that is gone
    /// (`scheduler::RunModelSource::DefaultInPlaceOfCreator`).
    ///
    /// Not a tool call, so there is no admitted `CallCapability` to inherit: the
    /// master switch and the bound model are read here, once each.
    pub(crate) async fn private_reach_of_this_chat(&self) -> Option<bool> {
        if !crate::privacy::privacy_tiers_enabled() {
            return None;
        }
        match self.provider().await {
            Ok(provider) if provider.tier().is_private() => Some(true),
            _ => None,
        }
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
            // Issue #56. The standing of the chat the schedule acts for. Its runs
            // take this chat's model while the chat gives one; once it does not,
            // they fall back to the configured default, and a PRIVATE default is
            // refused unless this says someone with private reach made it. A
            // public chat's `/loop` records nothing, so deleting that chat — which
            // takes only the daemon secret — cannot turn its runs private.
            armed_with_private_reach: self.private_reach_of_this_chat().await,
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
                // This one run is held to this chat's standing; nothing is recorded.
                let standing = self.private_reach_of_this_chat().await;
                tokio::spawn(async move {
                    if let Err(e) = scheduler.run_now_armed(&id, standing).await {
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
                // A resume is an arming: it records this chat's standing, and
                // records nothing from a public chat.
                let standing = self.private_reach_of_this_chat().await;
                return Ok(Some(
                    match self
                        .scheduler()
                        .await?
                        .unpause_schedule_armed(rest, standing)
                        .await
                    {
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
            armed_with_private_reach: None,
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

/// Issue #56: what `/loop`, `/schedule` and their resume record as the standing
/// of whoever armed the schedule (`ScheduledJob::armed_with_private_reach`).
///
/// Independent QA, 2026-09-14: a `/loop` made in a public chat recorded nothing,
/// and one secret-only `DELETE` of that chat turned its runs private. The run is
/// now refused that fallback (`scheduler::scheduled_run_refusal`); what these
/// pin is the other half — that a PRIVATE chat's schedule records the standing
/// that keeps it running once the person deletes the chat, and that a public
/// chat's never records a `false` that would refuse it after the person moves
/// the chat onto a private model.
#[cfg(test)]
mod standing_tests {
    use super::*;
    use crate::agents::AgentConfig;
    use crate::config::permission::PermissionManager;
    use crate::config::BioRouterMode;
    use crate::conversation::message::Message as ConversationMessage;
    use crate::model::ModelConfig;
    use crate::privacy::ProviderTier;
    use crate::providers::base::{Provider, ProviderMetadata, ProviderUsage};
    use crate::providers::errors::ProviderError;
    use crate::scheduler::Scheduler;
    use crate::session::session_manager::SessionType;
    use crate::session::SessionManager;
    use rmcp::model::Tool;
    use std::path::PathBuf;

    /// A provider whose only interesting property is its tier; no turn runs.
    struct TieredProvider(ProviderTier);

    #[async_trait::async_trait]
    impl Provider for TieredProvider {
        fn metadata() -> ProviderMetadata {
            ProviderMetadata::new("tiered", "Tiered", "", "tiered-model", vec![], "", vec![])
        }
        fn get_name(&self) -> &str {
            match self.0 {
                ProviderTier::Private => "versa_azure",
                ProviderTier::Public => "openai",
            }
        }
        fn tier(&self) -> ProviderTier {
            self.0
        }
        async fn complete_with_model(
            &self,
            _model_config: &ModelConfig,
            _system: &str,
            _messages: &[ConversationMessage],
            _tools: &[Tool],
        ) -> Result<(ConversationMessage, ProviderUsage), ProviderError> {
            unreachable!("no test here runs a turn")
        }
        fn get_model_config(&self) -> ModelConfig {
            ModelConfig::new_or_fail("tiered-model")
        }
    }

    struct Chat {
        _dir: tempfile::TempDir,
        agent: Agent,
        scheduler: Arc<Scheduler>,
        session_id: String,
    }

    /// A chat bound to a model of `tier` (or to none), over a real scheduler.
    async fn chat(tier: Option<ProviderTier>) -> Chat {
        let dir = tempfile::tempdir().unwrap();
        let session_manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let scheduler = Scheduler::new(dir.path().join("schedule.json"), session_manager.clone())
            .await
            .unwrap();
        let agent = Agent::with_config(AgentConfig::new(
            session_manager.clone(),
            Arc::new(PermissionManager::new(dir.path().to_path_buf())),
            Some(scheduler.clone()),
            BioRouterMode::Auto,
        ));
        let session = session_manager
            .create_session(
                PathBuf::from("."),
                "standing probe".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        if let Some(tier) = tier {
            agent
                .update_provider(Arc::new(TieredProvider(tier)), &session.id)
                .await
                .unwrap();
        }
        Chat {
            _dir: dir,
            agent,
            scheduler,
            session_id: session.id,
        }
    }

    async fn only_job(scheduler: &Scheduler) -> ScheduledJob {
        let jobs = scheduler.list_scheduled_jobs().await;
        assert_eq!(jobs.len(), 1, "{jobs:?}");
        jobs.into_iter().next().unwrap()
    }

    #[tokio::test]
    async fn a_loop_or_schedule_records_its_chats_standing_and_never_a_false() {
        for (tier, want) in [
            (Some(ProviderTier::Private), Some(true)),
            (Some(ProviderTier::Public), None),
            (None, None),
        ] {
            for command in [
                "/loop 1d probe the queue",
                "/schedule @daily probe the queue",
            ] {
                let chat = chat(tier).await;
                let (name, params) = command.trim_start_matches('/').split_once(' ').unwrap();
                let reply = if name == "loop" {
                    chat.agent
                        .handle_loop_command(params, &chat.session_id)
                        .await
                } else {
                    chat.agent
                        .handle_schedule_command(params, &chat.session_id)
                        .await
                };
                let reply = reply.unwrap().expect("the command answers");
                assert!(
                    reply.as_concat_text().contains("created"),
                    "{command}: {}",
                    reply.as_concat_text()
                );
                let job = only_job(&chat.scheduler).await;
                assert_eq!(
                    job.creator_session_id.as_deref(),
                    Some(chat.session_id.as_str())
                );
                assert_eq!(
                    job.armed_with_private_reach, want,
                    "{command} in a chat on {tier:?} recorded the wrong standing"
                );
            }
        }
    }

    /// `/schedule resume` is an arming: a private chat's records `Some(true)`,
    /// and a public chat's leaves whatever the schedule held.
    #[tokio::test]
    async fn a_schedule_resume_records_a_private_chats_standing_only() {
        for (tier, want) in [
            (ProviderTier::Private, Some(true)),
            (ProviderTier::Public, Some(false)),
        ] {
            let chat = chat(Some(tier)).await;
            chat.agent
                .handle_schedule_command("@daily probe the queue", &chat.session_id)
                .await
                .unwrap();
            let id = only_job(&chat.scheduler).await.id;
            chat.scheduler.pause_schedule(&id).await.unwrap();
            // As a public-only HTTP caller's re-time would have left it.
            chat.scheduler
                .update_schedule_armed(&id, "0 0 0 * * *".to_string(), Some(false))
                .await
                .unwrap();
            let reply = chat
                .agent
                .handle_schedule_command(&format!("resume {id}"), &chat.session_id)
                .await
                .unwrap()
                .unwrap();
            assert!(
                reply.as_concat_text().contains("Resumed"),
                "{}",
                reply.as_concat_text()
            );
            let job = only_job(&chat.scheduler).await;
            assert!(!job.paused);
            assert_eq!(
                job.armed_with_private_reach, want,
                "a resume from a chat on {tier:?}"
            );
        }
    }
}
