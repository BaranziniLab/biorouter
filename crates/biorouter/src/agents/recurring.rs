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
use crate::privacy::CallCapability;
use crate::scheduler::{
    get_default_scheduled_workflows_dir, schedule_work, scheduled_run_preflight, ScheduledJob,
};
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

/// What a `/schedule` or `/loop` verb typed into a chat that is NOT running a
/// private model is told when the schedule it names is one that chat may not
/// touch — issue #56.
///
/// Fixed text around the two things the person typed (`command`, `id`), and
/// nothing else: no model, no chat, no reason specific to the schedule. It says
/// the same thing for a schedule whose work is private and for an id that names
/// no schedule, for the reason `routes::session_reach::SCHEDULE_OUT_OF_REACH`
/// does — which chat a schedule acts for is exactly what the listing redacts.
fn slash_verb_out_of_reach(command: &str, id: &str) -> String {
    format!(
        "`/{command} {id}` changed nothing. This chat is not running a private model, so its \
         `/schedule` and `/loop` commands may manage only schedules whose work is public, and \
         `{id}` is not one of them, or there is no schedule with that id; the two are answered \
         the same way. A schedule's work is private when its runs use a private model or act \
         for a private chat. Nothing was run, paused, resumed or removed. To manage it, use the \
         Scheduler in the desktop app, or type the command in a chat running a private model."
    )
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
    /// Not a tool call, so there is no admitted `CallCapability` to inherit: each
    /// slash command samples ONE (`CallCapability::sample`) and every decision it
    /// makes — whether a verb reaches a schedule, and what it records — reads
    /// that one, so a model swapped mid-command cannot be gated on one tier and
    /// recorded on another.
    pub(crate) fn private_reach_of_this_chat(cap: CallCapability) -> Option<bool> {
        (cap.enforced() && cap.tier().is_private()).then_some(true)
    }

    /// Issue #56 — may a `/schedule` or `/loop` verb typed into this chat touch
    /// `job` (`None`: the id names no schedule)?
    ///
    /// A slash command's standing is the chat it runs in. `Agent::reply` runs
    /// `execute_command` for every user message before any model call, and a
    /// public chat accepts `POST /reply` from any holder of the daemon secret — so
    /// without this, a caller refused `POST /schedule/<id>/pause` (403) typed
    /// `/schedule pause <id>` into a public chat and got `paused: true`
    /// (independent QA, 2026-09-14).
    ///
    /// * A chat on a private model may manage any schedule — the counterpart of
    ///   an HTTP caller that states a private capability.
    /// * A chat on a public model, or with none bound, may manage only a schedule
    ///   whose work is public, by `scheduler::schedule_work` — THE definition the
    ///   daemon's schedule routes answer with. An id that names no schedule is
    ///   answered as private work, as those routes answer it.
    async fn slash_verb_reaches(&self, cap: CallCapability, job: Option<&ScheduledJob>) -> bool {
        if !cap.enforced() || cap.tier().is_private() {
            return true;
        }
        match job {
            Some(job) => schedule_work(job, &self.config.session_manager)
                .await
                .is_public(),
            None => false,
        }
    }

    /// The schedule `id` names, as the scheduler holds it now.
    async fn named_schedule(scheduler: &Arc<dyn SchedulerTrait>, id: &str) -> Option<ScheduledJob> {
        scheduler
            .list_scheduled_jobs()
            .await
            .into_iter()
            .find(|job| job.id == id)
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
        cap: CallCapability,
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
            armed_with_private_reach: Self::private_reach_of_this_chat(cap),
        };
        if let Err(e) = scheduler.add_scheduled_job(job, false).await {
            let _ = tokio::fs::remove_file(&path).await;
            return Err(anyhow!("Failed to schedule job: {e}"));
        }
        Ok(id)
    }

    /// `/loop stop <id|all>`, on the command's one sampled capability.
    ///
    /// Issue #56: `all` stops every loop this chat may stop and COUNTS the ones
    /// it may not, which are left running and never named; a single id this chat
    /// may not touch is refused and nothing is stopped.
    async fn stop_loops(&self, target: &str, cap: CallCapability) -> Result<Message> {
        let scheduler = self.scheduler().await?;
        let (ids, left) = if target == "all" {
            let mut ids = Vec::new();
            let mut left = 0usize;
            for job in scheduler
                .list_scheduled_jobs()
                .await
                .into_iter()
                .filter(|j| j.id.starts_with(LOOP_ID_PREFIX))
            {
                if self.slash_verb_reaches(cap, Some(&job)).await {
                    ids.push(job.id);
                } else {
                    left += 1;
                }
            }
            (ids, left)
        } else {
            let job = Self::named_schedule(&scheduler, target).await;
            if !self.slash_verb_reaches(cap, job.as_ref()).await {
                return Ok(
                    Message::assistant().with_text(slash_verb_out_of_reach("loop stop", target))
                );
            }
            (vec![target.to_string()], 0)
        };
        let left_note = match left {
            0 => String::new(),
            1 => " One other loop was left running: its work is private, so it can be stopped \
                   only from the Scheduler in the desktop app or from a chat running a private \
                   model."
                .to_string(),
            n => format!(
                " {n} other loops were left running: their work is private, so they can be \
                 stopped only from the Scheduler in the desktop app or from a chat running a \
                 private model."
            ),
        };
        if ids.is_empty() {
            let text = if left == 0 {
                "No active loops.".to_string()
            } else {
                format!("No loop was stopped.{left_note}")
            };
            return Ok(Message::assistant().with_text(text));
        }
        let mut stopped = Vec::new();
        for id in ids {
            if let Err(e) = scheduler.remove_scheduled_job(&id, true).await {
                return Ok(
                    Message::assistant().with_text(format!("Could not stop loop '{id}': {e}"))
                );
            }
            stopped.push(id);
        }
        Ok(Message::assistant().with_text(format!(
            "Stopped loop(s): {}.{left_note}",
            stopped.join(", ")
        )))
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

        // Issue #56: ONE sample of this chat's capability for the whole command —
        // what it may stop and what a new loop records are read off the same one.
        let cap = CallCapability::sample(&self.provider).await;

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
            return self.stop_loops(target, cap).await.map(Some);
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
            .create_recurring_job(
                LOOP_ID_PREFIX,
                &cron,
                prompt,
                session_id,
                Some(max_runs),
                cap,
            )
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

        // Issue #56: ONE sample of this chat's capability for the whole command.
        let cap = CallCapability::sample(&self.provider).await;

        let (verb, rest) = arg
            .split_once(char::is_whitespace)
            .map(|(v, r)| (v, r.trim()))
            .unwrap_or((arg, ""));
        if let Some(message) = self.schedule_management(verb, rest, cap).await? {
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
            .create_recurring_job(SCHEDULE_ID_PREFIX, &cron, &prompt, session_id, None, cap)
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
    ///
    /// Issue #56: every verb that changes or runs a schedule first asks
    /// [`Self::slash_verb_reaches`] on the chat's ONE sampled capability, and a
    /// refusal changes nothing. `sessions` filters instead, as `GET
    /// /schedule/{id}/sessions` does: a run's chat is listed exactly when this
    /// chat would be shown it in a listing.
    async fn schedule_management(
        &self,
        verb: &str,
        rest: &str,
        cap: CallCapability,
    ) -> Result<Option<Message>> {
        const VERBS: &[&str] = &[
            "remove", "delete", "run", "pause", "unpause", "resume", "sessions",
        ];
        if !VERBS.contains(&verb) {
            return Ok(None);
        }
        if rest.is_empty() {
            return Ok(Some(Message::assistant().with_text(SCHEDULE_USAGE)));
        }

        let scheduler = self.scheduler().await?;
        if verb == "sessions" {
            return self
                .schedule_sessions(&scheduler, rest, cap)
                .await
                .map(Some);
        }
        let job = Self::named_schedule(&scheduler, rest).await;
        if !self.slash_verb_reaches(cap, job.as_ref()).await {
            return Ok(Some(Message::assistant().with_text(
                slash_verb_out_of_reach(&format!("schedule {verb}"), rest),
            )));
        }

        match verb {
            "remove" | "delete" => {
                let owns = job
                    .as_ref()
                    .map(|j| owns_workflow_file(&j.source))
                    .unwrap_or(false);
                Ok(Some(
                    match scheduler.remove_scheduled_job(rest, owns).await {
                        Ok(()) => {
                            Message::assistant().with_text(format!("Removed schedule `{rest}`."))
                        }
                        Err(e) => Message::assistant()
                            .with_text(format!("Could not remove schedule '{rest}': {e}")),
                    },
                ))
            }
            "run" => {
                let Some(job) = job else {
                    return Ok(Some(
                        Message::assistant()
                            .with_text(format!("There is no schedule with the id `{rest}`.")),
                    ));
                };
                if job.currently_running {
                    return Ok(Some(Message::assistant().with_text(format!(
                        "`{rest}` is already running, so another run was not started."
                    ))));
                }
                // This run keeps the command's reach even if the target changes
                // before execution. Its transient false never re-arms the job.
                let standing = cap.enforced().then_some(cap.tier().is_private());
                // Issue #56: this reply used to say "Started" before the run
                // existed, and the run was then refused (independent QA,
                // 2026-09-14). Ask the run's own decision first and say so. The run
                // asks again when it starts.
                if let Some(refusal) = scheduled_run_preflight(
                    &job,
                    &self.config.session_manager,
                    cap.enforced(),
                    standing.or(job.armed_with_private_reach),
                )
                .await
                {
                    return Ok(Some(
                        Message::assistant()
                            .with_text(format!("`{rest}` was not started. {refusal}")),
                    ));
                }
                let id = rest.to_string();
                tokio::spawn(async move {
                    if let Err(e) = scheduler.run_now_armed(&id, standing).await {
                        tracing::error!("/schedule run '{}' failed: {}", id, e);
                    }
                });
                Ok(Some(Message::assistant().with_text(format!(
                    "▶️ Requested a background run of `{rest}`; check its status and results under \
                     `/schedule sessions {rest}`."
                ))))
            }
            "pause" => Ok(Some(match scheduler.pause_schedule(rest).await {
                Ok(()) => Message::assistant().with_text(format!("Paused schedule `{rest}`.")),
                Err(e) => Message::assistant().with_text(format!("Could not pause '{rest}': {e}")),
            })),
            "unpause" | "resume" => {
                // A resume is an arming: it records this chat's standing, and
                // records nothing from a public chat.
                let standing = Self::private_reach_of_this_chat(cap);
                Ok(Some(
                    match scheduler.unpause_schedule_armed(rest, standing).await {
                        Ok(()) => {
                            Message::assistant().with_text(format!("Resumed schedule `{rest}`."))
                        }
                        Err(e) => Message::assistant()
                            .with_text(format!("Could not resume '{rest}': {e}")),
                    },
                ))
            }
            _ => unreachable!("verb membership checked above"),
        }
    }

    /// `/schedule sessions <id>`: the schedule's recent runs this chat may be
    /// shown.
    ///
    /// Filtered, not refused, as `GET /schedule/{id}/sessions` filters through
    /// `lists_session`: a run of a private schedule is a private chat, and a chat
    /// that is not running a private model is listed only public ones
    /// (`privacy::visibility::appears_in_list`). The empty answer does not say
    /// whether anything was withheld — it varies with this chat's capability,
    /// never with the schedule.
    async fn schedule_sessions(
        &self,
        scheduler: &Arc<dyn SchedulerTrait>,
        id: &str,
        cap: CallCapability,
    ) -> Result<Message> {
        const SHOWN: usize = 5;
        // Read past the ones withheld, so a filtered list is not merely shorter.
        let read = if cap.enforced() { SHOWN * 10 } else { SHOWN };
        let sessions = scheduler
            .sessions(id, read)
            .await
            .map_err(|e| anyhow!("Could not list sessions for '{id}': {e}"))?;
        let lines: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| {
                !cap.enforced()
                    || crate::privacy::visibility::appears_in_list(cap.tier(), s.privacy_tier)
            })
            .take(SHOWN)
            .map(|(run, s)| format!("- `{}`: {}", run, s.created_at))
            .collect();
        let text = if !lines.is_empty() {
            format!("Recent runs of `{id}`:\n{}", lines.join("\n"))
        } else if cap.restricts_private_data() {
            format!(
                "No runs of `{id}` that this chat can see. A run of a schedule whose work is \
                 private is a private chat, and this chat is not running a private model."
            )
        } else {
            format!("No runs recorded yet for `{id}`.")
        };
        Ok(Message::assistant().with_text(text))
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

    // ── Issue #56: a slash command's standing is the chat it runs in ──────────

    /// A chat row in `chat`'s session store: `provider` recorded on it, and
    /// classified private when `private`.
    async fn row(chat: &Chat, provider: Option<&str>, private: bool) -> String {
        let sessions = &chat.agent.config.session_manager;
        let session = sessions
            .create_session(
                PathBuf::from("."),
                "slash reach fixture".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        let mut update = sessions.update(&session.id);
        if let Some(provider) = provider {
            update = update.provider_name(provider);
        }
        if private {
            update = update.raise_privacy(
                crate::privacy::SessionClassification::Private,
                "turn:versa_azure",
            );
        }
        update.apply().await.unwrap();
        session.id
    }

    /// Add a dormant schedule made from `creator`.
    async fn seed(chat: &Chat, id: &str, creator: &str, paused: bool) {
        let workflow = chat._dir.path().join(format!("{id}.yaml"));
        std::fs::write(&workflow, "prompt: probe\n").unwrap();
        let job = ScheduledJob {
            id: id.to_string(),
            source: workflow.to_string_lossy().into_owned(),
            cron: "0 0 0 1 1 *".to_string(),
            last_run: None,
            currently_running: false,
            paused,
            current_session_id: None,
            process_start_time: None,
            run_count: 0,
            max_runs: None,
            creator_session_id: Some(creator.to_string()),
            last_error: None,
            owns_source: None,
            armed_with_private_reach: None,
        };
        chat.scheduler.add_scheduled_job(job, false).await.unwrap();
    }

    async fn job(chat: &Chat, id: &str) -> Option<ScheduledJob> {
        chat.scheduler
            .list_scheduled_jobs()
            .await
            .into_iter()
            .find(|job| job.id == id)
    }

    async fn say(chat: &Chat, command: &str) -> String {
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
        reply
            .unwrap()
            .expect("the command answers")
            .as_concat_text()
    }

    /// Independent QA, 2026-09-14: `POST /schedule/<private>/pause` was refused
    /// 403 to a caller holding only the daemon secret, and the same caller typing
    /// `/schedule pause <private>` into a public chat got `paused: true`.
    ///
    /// Every verb that changes or runs a schedule, from a chat on a public model
    /// and from one with no model, on each way a schedule's work is private — a
    /// private creator, a creator bound to a private model (what a secret-only
    /// restart left behind), and an id naming nothing. Each is refused in the
    /// fixed words and changes nothing.
    #[tokio::test]
    async fn a_public_chats_slash_verbs_do_not_reach_private_work() {
        for tier in [Some(ProviderTier::Public), None] {
            let chat = chat(tier).await;
            let private_creator = row(&chat, Some("br-public-probe"), true).await;
            let bound_private = row(&chat, Some("versa_azure"), false).await;
            seed(&chat, "task-private", &private_creator, false).await;
            seed(&chat, "task-private-paused", &private_creator, true).await;
            seed(&chat, "task-private-model", &bound_private, false).await;
            seed(&chat, "loop-private", &private_creator, false).await;

            for command in [
                "/schedule pause task-private",
                "/schedule pause task-private-model",
                "/schedule run task-private",
                "/schedule run task-private-model",
                "/schedule remove task-private",
                "/schedule delete task-private-model",
                "/schedule resume task-private-paused",
                "/schedule unpause task-private-paused",
                "/loop stop loop-private",
                "/loop stop task-private",
                "/schedule pause task-that-does-not-exist",
            ] {
                let reply = say(&chat, command).await;
                assert!(
                    reply.contains("changed nothing") && !reply.contains("Started"),
                    "{command} from a chat on {tier:?} was not refused: {reply}"
                );
                assert!(
                    !reply.contains(&private_creator) && !reply.contains("versa_azure"),
                    "the refusal named something the person did not type: {reply}"
                );
            }
            for id in ["task-private", "task-private-model", "loop-private"] {
                let job = job(&chat, id)
                    .await
                    .expect("a refused verb removed a schedule");
                assert!(!job.paused, "{id} was paused by a refused verb");
                assert_eq!(job.run_count, 0, "{id}");
                assert!(
                    !job.currently_running && job.last_error.is_none(),
                    "{id} ran"
                );
            }
            let paused = job(&chat, "task-private-paused").await.unwrap();
            assert!(paused.paused, "a refused resume resumed it");
            assert_eq!(paused.armed_with_private_reach, None);

            // `/loop stop all` stops the loops in reach and leaves the rest.
            let public_creator = row(&chat, Some("br-public-probe"), false).await;
            seed(&chat, "loop-public", &public_creator, false).await;
            let reply = say(&chat, "/loop stop all").await;
            assert!(
                reply.contains("loop-public") && reply.contains("One other loop was left running"),
                "{reply}"
            );
            assert!(!reply.contains("loop-private"), "{reply}");
            assert!(job(&chat, "loop-public").await.is_none());
            assert!(
                job(&chat, "loop-private").await.is_some(),
                "`/loop stop all` in a public chat stopped a private chat's loop"
            );
        }
    }

    /// The other half, without which the refusals above would pass on a build
    /// that refused every verb: a chat on a private model manages private work,
    /// and a public chat still manages PUBLIC work — the whole Schedules surface
    /// of an install configured with a public model.
    #[tokio::test]
    async fn a_private_chat_manages_any_schedule_and_a_public_chat_manages_public_work() {
        for (tier, creator_is_private) in
            [(ProviderTier::Private, true), (ProviderTier::Public, false)]
        {
            let chat = chat(Some(tier)).await;
            let creator = row(&chat, Some("br-public-probe"), creator_is_private).await;
            seed(&chat, "task-work", &creator, false).await;
            seed(&chat, "loop-work", &creator, false).await;

            assert!(say(&chat, "/schedule pause task-work")
                .await
                .contains("Paused"));
            assert!(job(&chat, "task-work").await.unwrap().paused);
            assert!(say(&chat, "/schedule resume task-work")
                .await
                .contains("Resumed"));
            let resumed = job(&chat, "task-work").await.unwrap();
            assert!(!resumed.paused);
            assert_eq!(
                resumed.armed_with_private_reach,
                (tier == ProviderTier::Private).then_some(true),
                "a resume records the chat's standing, and nothing from a public chat"
            );
            // The run's model cannot be built here, so it fails in the
            // background; what matters is that the verb reached it.
            assert!(say(&chat, "/schedule run task-work")
                .await
                .contains("Requested a background run"));
            for _ in 0..100 {
                if job(&chat, "task-work")
                    .await
                    .is_some_and(|job| !job.currently_running && job.last_error.is_some())
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            assert!(say(&chat, "/schedule remove task-work")
                .await
                .contains("Removed"));
            assert!(job(&chat, "task-work").await.is_none());
            assert!(say(&chat, "/loop stop all").await.contains("loop-work"));
            assert!(job(&chat, "loop-work").await.is_none());
        }
    }

    /// Independent QA, 2026-09-14: `/schedule run <id>` answered "▶️ Started …"
    /// and the run was then refused. The measured chain: a `/schedule` made in a
    /// chat that records no model, on an install whose default is private, run
    /// from that same chat. It must not claim a start.
    #[tokio::test]
    async fn a_schedule_run_that_cannot_start_does_not_say_it_started() {
        let chat = chat(None).await;
        let versa_default = std::collections::HashMap::from([(
            "BIOROUTER_PROVIDER".to_string(),
            "versa_azure".to_string(),
        )]);
        let reply = crate::config::with_config_overrides(
            versa_default.clone(),
            say(&chat, "/schedule @yearly probe the queue"),
        )
        .await;
        assert!(reply.contains("created"), "{reply}");
        let id = only_job(&chat.scheduler).await.id;
        let reply = crate::config::with_config_overrides(
            versa_default,
            say(&chat, &format!("/schedule run {id}")),
        )
        .await;
        assert!(
            !reply.contains("Started"),
            "a run that could not start was reported as started: {reply}"
        );
        assert!(
            reply.contains("changed nothing") || reply.contains("was not started"),
            "{reply}"
        );
        let job = only_job(&chat.scheduler).await;
        assert!(!job.currently_running && job.last_error.is_none() && job.run_count == 0);
    }

    /// `/schedule sessions <id>` filters rather than refuses, as `GET
    /// /schedule/{id}/sessions` does: a public chat is shown a schedule's public
    /// runs and never its private ones.
    #[tokio::test]
    async fn a_public_chat_is_shown_only_the_runs_it_could_list() {
        let chat = chat(Some(ProviderTier::Public)).await;
        let creator = row(&chat, Some("br-public-probe"), false).await;
        seed(&chat, "task-runs", &creator, false).await;
        let sessions = &chat.agent.config.session_manager;
        let mut runs = Vec::new();
        for private in [false, true] {
            let run = sessions
                .create_session(
                    PathBuf::from("."),
                    "Scheduled job: task-runs".to_string(),
                    SessionType::Scheduled,
                )
                .await
                .unwrap();
            let mut update = sessions
                .update(&run.id)
                .schedule_id(Some("task-runs".into()));
            if private {
                update = update.raise_privacy(
                    crate::privacy::SessionClassification::Private,
                    "turn:versa_azure",
                );
            }
            update.apply().await.unwrap();
            // The listing skips a chat with no messages.
            sessions
                .add_message(&run.id, &ConversationMessage::user().with_text("probe"))
                .await
                .unwrap();
            runs.push(run.id);
        }
        let reply = say(&chat, "/schedule sessions task-runs").await;
        assert!(
            reply.contains(&runs[0]),
            "the public run is listed: {reply}"
        );
        assert!(
            !reply.contains(&runs[1]),
            "a public chat was shown a private run: {reply}"
        );
    }
}
