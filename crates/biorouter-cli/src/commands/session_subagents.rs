//! `biorouter session subagents <SESSION_ID>`: the subagents one chat has
//! started, with their status, model and setup, and the chat's phase plan.
//!
//! The rows are the ones every other surface draws, from ONE function,
//! `biorouter::agents::subagent_roster::session_subagents`:
//!
//! * **With a reachable daemon** this asks `GET /sessions/{id}/subagents`, which
//!   runs that function where the children actually run, so a child's live state
//!   (queued, running, stopping) is known. The daemon applies its reach gate and
//!   filters the children the caller may not see.
//! * **With none** it runs the same function in this process, against the local
//!   session store, and says once (on stderr) that live state needs a running
//!   daemon. A child that was running reads as interrupted or unknown then,
//!   because nothing in this process is executing it.
//!
//! ⚠ **The local path applies the same tier rule as the route.** Otherwise
//! unsetting `BIOROUTER_SERVER__SECRET_KEY` would be a way around the daemon's
//! gate: a terminal on a public model is refused a private parent, and is not
//! listed a private child. The capability is the one every other CLI command
//! states to the daemon, the configured provider's tier.

use std::collections::HashSet;
use std::io::IsTerminal;

use anyhow::{anyhow, bail, Result};
// `QUIET_AFTER_SECONDS`: a child active but silent for at least this long is
// shown as quiet, a heuristic, so it is labelled "quiet", never "stalled". It is
// the plan's own constant, which also decides each phase's quiet count, so a row
// and the phase line printed above it can never disagree; the TUI pane and the
// desktop panel use the same value.
use biorouter::agents::orchestration_plan::{
    OrchestrationSnapshot, PhaseStatus, QUIET_AFTER_SECONDS,
};
use biorouter::agents::subagent_roster::{SessionSubagents, SubagentRow, SubagentRunStatus};
use biorouter::privacy::visibility::{appears_in_list, may_read};
use biorouter::privacy::ProviderTier;
use biorouter::session::SessionManager;

use super::apps::{configured_port, daemon_ok};
use super::session_watch::{
    daemon_auth, daemon_get_json, no_daemon_at, refusal_sentence, with_supplied_key, DaemonAuth,
};

/// The prefix a subagent's session name carries, which every surface drops.
pub const SUBAGENT_NAME_PREFIX: &str = "Subagent: ";

/// How often `--watch` redraws.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// How long one request to the daemon may take.
const REQUEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// What a terminal on a public model is told about a private chat on the local
/// path. The daemon's own refusal is printed as the daemon wrote it.
pub const LOCAL_OUT_OF_REACH: &str =
    "This chat is private, and this terminal is configured for a model hosted outside your \
     institution, so its subagents are not shown here. Run this from a terminal configured for a \
     private model, or open the chat in the desktop app.";

/// `--format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SubagentsFormat {
    Text,
    Json,
}

/// The glyph a child's status is drawn with, on every terminal surface.
pub fn status_glyph(status: SubagentRunStatus) -> &'static str {
    match status {
        SubagentRunStatus::Queued => "○",
        SubagentRunStatus::Running | SubagentRunStatus::Cancelling => "◐",
        SubagentRunStatus::Completed => "●",
        SubagentRunStatus::Blocked => "⏸",
        SubagentRunStatus::Incomplete | SubagentRunStatus::Failed => "✗",
        SubagentRunStatus::Interrupted => "⊘",
        SubagentRunStatus::Unknown => "·",
    }
}

/// The word a child's status is shown with beside its glyph.
pub fn status_label(status: SubagentRunStatus) -> &'static str {
    match status {
        SubagentRunStatus::Queued => "queued",
        SubagentRunStatus::Running => "running",
        SubagentRunStatus::Cancelling => "stopping",
        SubagentRunStatus::Completed => "completed",
        SubagentRunStatus::Blocked => "blocked",
        SubagentRunStatus::Incomplete => "incomplete",
        SubagentRunStatus::Failed => "failed",
        SubagentRunStatus::Interrupted => "interrupted",
        SubagentRunStatus::Unknown => "unknown",
    }
}

/// The glyph a phase is drawn with.
pub fn phase_glyph(status: PhaseStatus) -> &'static str {
    match status {
        PhaseStatus::Pending => "○",
        PhaseStatus::InProgress => "◐",
        PhaseStatus::Completed => "●",
        PhaseStatus::Blocked => "⏸",
        PhaseStatus::Skipped => "⊘",
    }
}

/// `42s`, `1m42s`, `2h05m`.
pub fn format_elapsed(seconds: u64) -> String {
    match seconds {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m{:02}s", s / 60, s % 60),
        s => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// `3 ext · 1 skill · 0 kb`.
pub fn setup_summary(row: &SubagentRow) -> String {
    format!(
        "{} ext · {} · {} kb",
        row.extensions.len(),
        plural(row.skills.len(), "skill", "skills"),
        row.knowledge_bases.len()
    )
}

/// Model-written text made safe to print: every control character (an escape
/// sequence, a carriage return, a newline) becomes a space, so a name or a
/// result line cannot move the cursor, recolour the terminal or forge a line.
pub fn printable(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

fn display_name(row: &SubagentRow) -> String {
    let name = printable(
        row.name
            .strip_prefix(SUBAGENT_NAME_PREFIX)
            .unwrap_or(&row.name),
    );
    let role = row
        .role
        .as_deref()
        .map(printable)
        .filter(|role| !role.is_empty());
    match (name.is_empty(), role) {
        (true, Some(role)) => role,
        (true, None) => row.session_id.clone(),
        (false, Some(role)) if role != name => format!("{name} ({role})"),
        (false, _) => name,
    }
}

fn dim(text: &str, color: bool) -> String {
    if color {
        console::style(text).dim().force_styling(true).to_string()
    } else {
        text.to_string()
    }
}

/// One line per phase: glyph, title, `done/total done`, and the counts that are
/// not zero.
fn render_plan(plan: &OrchestrationSnapshot, out: &mut Vec<String>) {
    out.push(format!("Plan: {}", printable(&plan.title)));
    if let Some(goal) = plan
        .goal
        .as_deref()
        .map(printable)
        .filter(|goal| !goal.is_empty())
    {
        out.push(format!("  Goal: {goal}"));
    }
    let width = plan
        .phases
        .iter()
        .map(|phase| printable(&phase.title).chars().count())
        .max()
        .unwrap_or(0);
    for phase in &plan.phases {
        let counts = &phase.counts;
        let mut line = format!(
            "  {} {:<width$}  {}/{} done",
            phase_glyph(phase.status),
            printable(&phase.title),
            counts.done,
            counts.total,
        );
        for (count, word) in [
            (counts.running, "running"),
            (counts.blocked, "blocked"),
            (counts.failed, "failed"),
            (counts.quiet, "quiet"),
        ] {
            if count > 0 {
                line.push_str(&format!(" · {count} {word}"));
            }
        }
        if phase.status == PhaseStatus::Skipped {
            line.push_str(" · skipped");
        }
        out.push(line);
    }
}

/// The text `biorouter session subagents` prints: the plan first when there is
/// one, then one numbered entry per child, then how to open one.
///
/// Pure, so it is tested with hand-built rosters; `color` only dims the result
/// lines.
pub fn render_text(roster: &SessionSubagents, color: bool) -> String {
    let mut out = Vec::new();
    if let Some(plan) = &roster.plan {
        render_plan(plan, &mut out);
        out.push(String::new());
    }
    if roster.subagents.is_empty() {
        out.push(format!(
            "No subagents for session {}.",
            roster.parent_session_id
        ));
        return out.join("\n");
    }
    out.push(format!(
        "Subagents of session {} ({}):",
        roster.parent_session_id,
        roster.subagents.len()
    ));
    let phase_titles: std::collections::HashMap<&str, &str> = roster
        .plan
        .iter()
        .flat_map(|plan| plan.phases.iter())
        .map(|phase| (phase.id.as_str(), phase.title.as_str()))
        .collect();
    for (index, row) in roster.subagents.iter().enumerate() {
        let mut head = format!(
            "  {}. {} {}  {}",
            index + 1,
            status_glyph(row.status),
            display_name(row),
            status_label(row.status)
        );
        if row.status.is_active() {
            if let Some(quiet) = row.quiet_seconds.filter(|s| *s >= QUIET_AFTER_SECONDS) {
                head.push_str(&format!(" · quiet {}", format_elapsed(quiet)));
            }
        }
        if let Some(phase) = row.phase_id.as_deref() {
            let title = phase_titles.get(phase).copied().unwrap_or(phase);
            head.push_str(&format!(" · phase {}", printable(title)));
        }
        out.push(head);

        let model = row
            .model_name
            .as_deref()
            .or(row.provider_name.as_deref())
            .map(printable)
            .unwrap_or_else(|| "no model recorded".to_string());
        let mut detail = format!(
            "       {}  {}  {}  {}",
            row.session_id,
            model,
            setup_summary(row),
            plural(
                row.tool_call_count.max(0) as usize,
                "tool call",
                "tool calls"
            ),
        );
        if let Some(elapsed) = row.elapsed_seconds {
            detail.push_str(&format!("  {}", format_elapsed(elapsed)));
        }
        out.push(detail);
        if let Some(result) = row
            .result_line
            .as_deref()
            .map(printable)
            .filter(|line| !line.is_empty())
        {
            out.push(format!("       {}", dim(&result, color)));
        }
    }
    out.push(String::new());
    out.push(
        "Follow a running one: biorouter session attach <session id>. Read a finished one: \
         biorouter session export --session-id <session id>."
            .to_string(),
    );
    out.join("\n")
}

/// The plan's phases may only name children the roster still lists.
fn confine_plan(roster: &mut SessionSubagents) {
    let visible: HashSet<String> = roster
        .subagents
        .iter()
        .map(|row| row.session_id.clone())
        .collect();
    if let Some(plan) = roster.plan.as_mut() {
        for phase in &mut plan.phases {
            phase.agent_ids.retain(|id| visible.contains(id));
        }
    }
}

/// Where the answer came from.
#[derive(Clone)]
enum Source {
    /// A daemon answered; later reads ask it again with the same credentials,
    /// including a user-action key read from stdin once.
    Daemon(DaemonAuth),
    /// The local store; the string says why no daemon answered.
    Local(String),
}

/// What the daemon said: the roster, or `Ok(Err(why))` when nothing is
/// listening, in which case the caller reads the local store. A daemon that
/// answers and refuses is an error, never a reason to fall back: the local path
/// must not be a way around its gate.
async fn from_daemon(
    session_id: &str,
    auth: &DaemonAuth,
) -> Result<std::result::Result<SessionSubagents, String>> {
    let port = configured_port();
    if !daemon_ok(super::apps::DAEMON_HOST, port).await {
        return Ok(Err(no_daemon_at(port)));
    }
    let path = format!("/sessions/{session_id}/subagents");
    let (code, body) = daemon_get_json(&path, auth, port, REQUEST_DEADLINE).await?;
    match code {
        200 => Ok(Ok(parse_roster(&body)?)),
        404 => Err(anyhow!("Session '{session_id}' not found.")),
        403 => Err(anyhow!(
            "{}",
            refusal_sentence(&body).unwrap_or_else(|| "the daemon refused the request".into())
        )),
        401 => Err(anyhow!(
            "the daemon refused the secret key (401): BIOROUTER_SERVER__SECRET_KEY does not \
             match the daemon's"
        )),
        other => Err(anyhow!(
            "the daemon answered GET {path} with {other}: {}",
            refusal_sentence(&body).unwrap_or_default()
        )),
    }
}

/// The route's JSON body, read from the first `{` to the last `}` so stray
/// framing around it is ignored.
fn parse_roster(body: &str) -> Result<SessionSubagents> {
    let slice = body
        .find('{')
        .zip(body.rfind('}'))
        .and_then(|(start, end)| body.get(start..=end))
        .ok_or_else(|| anyhow!("the daemon's answer held no JSON object"))?;
    serde_json::from_str(slice)
        .map_err(|error| anyhow!("the daemon's answer was not a subagent roster: {error}"))
}

/// The roster from this process's own session store, under the same tier rule
/// the route applies.
async fn from_local_store(session_id: &str) -> Result<SessionSubagents> {
    let manager = SessionManager::instance();
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    let capability = crate::session::privacy::caller_capability().await;
    from_store(&manager, session_id, capability, enforced).await
}

/// [`from_local_store`] against a given store, at a given capability: the
/// parent must be readable at `capability`, and each child is listed only when
/// it may appear in a listing at `capability`. `enforced` is the master switch.
async fn from_store(
    manager: &SessionManager,
    session_id: &str,
    capability: ProviderTier,
    enforced: bool,
) -> Result<SessionSubagents> {
    let parent = manager
        .get_session(session_id, false)
        .await
        .map_err(|_| anyhow!("Session '{session_id}' not found."))?;
    if enforced && !may_read(capability, parent.privacy_tier) {
        bail!("{LOCAL_OUT_OF_REACH}");
    }
    let mut roster =
        biorouter::agents::subagent_roster::session_subagents(manager, session_id).await?;
    if enforced {
        let before = roster.subagents.len();
        roster
            .subagents
            .retain(|row| appears_in_list(capability, row.privacy_tier));
        if roster.subagents.len() != before {
            // Rebuilt from what is left, so no phase counts an omitted child.
            roster.plan = biorouter::agents::orchestration_plan::snapshot(
                manager,
                session_id,
                &roster.subagents,
            )
            .await;
        }
    }
    confine_plan(&mut roster);
    Ok(roster)
}

/// The first read: decide where the roster comes from, and read it.
///
/// The user-action key is read from stdin only once a daemon is known to be
/// listening, because it is only ever sent to one.
async fn first_fetch(
    session_id: &str,
    user_action_key_stdin: bool,
) -> Result<(SessionSubagents, Source)> {
    let auth = match daemon_auth().await {
        Ok(auth) => auth,
        Err(_) => {
            let why = "BIOROUTER_SERVER__SECRET_KEY is not set, so no daemon was asked";
            return Ok((
                from_local_store(session_id).await?,
                Source::Local(why.to_string()),
            ));
        }
    };
    if !daemon_ok(super::apps::DAEMON_HOST, configured_port()).await {
        let why = no_daemon_at(configured_port());
        return Ok((from_local_store(session_id).await?, Source::Local(why)));
    }
    let auth = with_supplied_key(auth, user_action_key_stdin).await?;
    match from_daemon(session_id, &auth).await? {
        Ok(roster) => Ok((roster, Source::Daemon(auth))),
        Err(why) => Ok((from_local_store(session_id).await?, Source::Local(why))),
    }
}

fn local_note(why: &str) -> String {
    format!(
        "note: read from the local session store ({why}). Live state needs a running biorouter \
         daemon, so a subagent that was running reads as interrupted or unknown."
    )
}

fn render(roster: &SessionSubagents, format: SubagentsFormat, color: bool) -> Result<String> {
    Ok(match format {
        SubagentsFormat::Json => serde_json::to_string(roster)?,
        SubagentsFormat::Text => render_text(roster, color),
    })
}

/// `biorouter session subagents <SESSION_ID> [--format text|json] [--watch]`.
pub async fn handle_session_subagents(
    session_id: &str,
    format: SubagentsFormat,
    watch: bool,
    user_action_key_stdin: bool,
) -> Result<()> {
    let terminal = std::io::stdout().is_terminal();
    let color = format == SubagentsFormat::Text && terminal;
    let (roster, source) = first_fetch(session_id, user_action_key_stdin).await?;
    if let Source::Local(why) = &source {
        eprintln!("{}", local_note(why));
    }
    if !watch {
        println!("{}", render(&roster, format, color)?);
        return Ok(());
    }

    // `--watch`. Text on a terminal redraws in place; anything else (a pipe,
    // `--format json`) prints one frame per tick, so a script reads a stream.
    let redraw = format == SubagentsFormat::Text && terminal;
    watch_until(
        tokio::signal::ctrl_c(),
        WATCH_INTERVAL,
        (roster, source),
        |(roster, _)| {
            let frame = render(roster, format, color)?;
            if redraw {
                print!("\x1b[2J\x1b[H");
                println!("{frame}");
                println!();
                println!(
                    "{}",
                    dim("Refreshing every 2 s. Press Ctrl-C to stop.", color)
                );
            } else {
                println!("{frame}");
            }
            Ok(())
        },
        |(_, source)| refresh(session_id, source),
    )
    .await
}

/// The `--watch` loop: draw `state`, wait `interval`, refresh it, until `stop`
/// resolves.
///
/// `stop` is ONE future for the whole watch (Ctrl-C), raced against the refresh
/// as well as the wait. Its first poll replaces the default SIGINT handler for
/// the rest of the process, and a `ctrl_c()` made afresh per tick only sees a
/// signal delivered while it exists, so a Ctrl-C pressed during a refresh (which
/// can wait on the daemon for several seconds) used to be dropped.
async fn watch_until<T, Stop, Draw, Refresh, Next>(
    stop: Stop,
    interval: std::time::Duration,
    mut state: T,
    mut draw: Draw,
    mut refresh: Refresh,
) -> Result<()>
where
    Stop: std::future::Future,
    Draw: FnMut(&T) -> Result<()>,
    Refresh: FnMut(T) -> Next,
    Next: std::future::Future<Output = Result<T>>,
{
    tokio::pin!(stop);
    loop {
        draw(&state)?;
        tokio::select! {
            _ = &mut stop => return Ok(()),
            _ = tokio::time::sleep(interval) => {}
        }
        tokio::select! {
            _ = &mut stop => return Ok(()),
            next = refresh(state) => {
                state = next?;
            }
        }
    }
}

/// One `--watch` tick's read, from wherever the last one came from.
async fn refresh(session_id: &str, source: Source) -> Result<(SessionSubagents, Source)> {
    match source {
        Source::Daemon(auth) => match from_daemon(session_id, &auth).await? {
            Ok(roster) => Ok((roster, Source::Daemon(auth))),
            // The daemon went away mid-watch: say so once and carry on from the
            // local store rather than ending the watch.
            Err(why) => {
                eprintln!("{}", local_note(&why));
                Ok((from_local_store(session_id).await?, Source::Local(why)))
            }
        },
        Source::Local(why) => Ok((from_local_store(session_id).await?, Source::Local(why))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use biorouter::agents::orchestration_plan::{PhaseCounts, PhaseKind, PhaseSnapshot};
    use biorouter::privacy::SessionClassification;

    fn row(id: &str, status: SubagentRunStatus) -> SubagentRow {
        let now = chrono::Utc::now();
        SubagentRow {
            session_id: id.to_string(),
            name: format!("{SUBAGENT_NAME_PREFIX}audit the {id} module"),
            role: None,
            phase_id: None,
            status,
            loop_phase: None,
            quiet_seconds: None,
            created_at: now,
            updated_at: now,
            ended_at: None,
            elapsed_seconds: Some(102),
            provider_name: Some("versa_azure".to_string()),
            model_name: Some("gpt-5.5".to_string()),
            extensions: vec!["developer".into(), "memory".into(), "knowledge".into()],
            skills: vec!["code-review".into()],
            knowledge_bases: Vec::new(),
            privacy_tier: SessionClassification::Public,
            total_tokens: Some(1200),
            message_count: 4,
            tool_call_count: 12,
            result_line: None,
        }
    }

    fn phase(id: &str, title: &str, status: PhaseStatus, counts: PhaseCounts) -> PhaseSnapshot {
        PhaseSnapshot {
            id: id.to_string(),
            title: title.to_string(),
            goal: None,
            kind: PhaseKind::Research,
            status,
            depends_on: Vec::new(),
            agent_ids: Vec::new(),
            counts,
            handoff: None,
            started_at: None,
            finished_at: None,
        }
    }

    fn roster(
        subagents: Vec<SubagentRow>,
        plan: Option<OrchestrationSnapshot>,
    ) -> SessionSubagents {
        SessionSubagents {
            parent_session_id: "20261104_parent".to_string(),
            subagents,
            plan,
        }
    }

    #[test]
    fn every_status_has_its_own_glyph_and_word() {
        use SubagentRunStatus as S;
        let all = [
            S::Queued,
            S::Running,
            S::Cancelling,
            S::Completed,
            S::Blocked,
            S::Incomplete,
            S::Failed,
            S::Interrupted,
            S::Unknown,
        ];
        let expected = ["○", "◐", "◐", "●", "⏸", "✗", "✗", "⊘", "·"];
        for (status, glyph) in all.iter().zip(expected) {
            assert_eq!(status_glyph(*status), glyph, "{status:?}");
        }
        let words: HashSet<&str> = all.iter().map(|s| status_label(*s)).collect();
        assert_eq!(words.len(), all.len(), "two statuses share a word");
    }

    #[test]
    fn a_child_line_carries_its_identity_model_setup_and_progress() {
        let mut child = row("c1", SubagentRunStatus::Running);
        child.role = Some("reviewer".to_string());
        child.quiet_seconds = Some(240);
        let text = render_text(&roster(vec![child], None), false);
        assert!(
            text.contains("1. ◐ audit the c1 module (reviewer)  running"),
            "{text}"
        );
        assert!(text.contains("quiet 4m00s"), "{text}");
        assert!(
            !text.contains(SUBAGENT_NAME_PREFIX),
            "the prefix is dropped: {text}"
        );
        assert!(
            text.contains("c1  gpt-5.5  3 ext · 1 skill · 0 kb  12 tool calls  1m42s"),
            "{text}"
        );
        assert!(
            text.contains("biorouter session attach <session id>"),
            "{text}"
        );
    }

    #[test]
    fn the_result_line_is_dimmed_only_when_color_is_on() {
        let mut child = row("c1", SubagentRunStatus::Completed);
        child.result_line = Some("Found three call sites.".to_string());
        let plain = render_text(&roster(vec![child.clone()], None), false);
        assert!(plain.contains("       Found three call sites."), "{plain}");
        assert!(
            !plain.contains('\u{1b}'),
            "no escape codes without color: {plain:?}"
        );
        let colored = render_text(&roster(vec![child], None), true);
        assert!(
            colored.contains("\u{1b}[2m"),
            "the result line is dimmed: {colored:?}"
        );
    }

    #[test]
    fn a_quiet_reading_is_shown_only_for_an_active_child() {
        let mut done = row("c1", SubagentRunStatus::Completed);
        done.quiet_seconds = Some(600);
        let mut fresh = row("c2", SubagentRunStatus::Running);
        fresh.quiet_seconds = Some(5);
        let text = render_text(&roster(vec![done, fresh], None), false);
        assert!(!text.contains("quiet"), "{text}");
    }

    /// A Ctrl-C pressed while a refresh is waiting on the daemon ends the watch
    /// then, not after the refresh: the stop is raced against the refresh too.
    #[tokio::test]
    async fn a_stop_during_a_slow_refresh_ends_the_watch_at_once() {
        use std::time::Duration;

        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let mut stop_tx = Some(stop_tx);
        let mut draws = 0u32;
        let started = std::time::Instant::now();
        let watch = watch_until(
            async {
                let _ = stop_rx.await;
            },
            Duration::from_millis(10),
            0u32,
            |_| {
                draws += 1;
                Ok(())
            },
            |ticks| {
                // The first refresh hangs on a slow daemon; the stop lands in it.
                let stop = stop_tx.take();
                async move {
                    if let Some(stop) = stop {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        let _ = stop.send(());
                    }
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    Ok(ticks + 1)
                }
            },
        );
        tokio::time::timeout(Duration::from_secs(5), watch)
            .await
            .expect("a stop during a refresh must end the watch without waiting for it")
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(draws, 1, "the hung refresh never drew a second frame");
    }

    /// The row's quiet label uses the plan's threshold, the one the phase line
    /// above it counts with: two minutes is not quiet on either, three is.
    #[test]
    fn a_row_is_quiet_on_the_same_threshold_as_its_phase() {
        assert_eq!(QUIET_AFTER_SECONDS, 180);
        let mut two_minutes = row("c1", SubagentRunStatus::Running);
        two_minutes.quiet_seconds = Some(120);
        let text = render_text(&roster(vec![two_minutes], None), false);
        assert!(!text.contains("quiet"), "{text}");

        let mut three_minutes = row("c1", SubagentRunStatus::Running);
        three_minutes.quiet_seconds = Some(QUIET_AFTER_SECONDS);
        let text = render_text(&roster(vec![three_minutes], None), false);
        assert!(text.contains("quiet 3m00s"), "{text}");
    }

    #[test]
    fn the_plan_comes_first_with_per_phase_progress() {
        let plan = OrchestrationSnapshot {
            title: "Migrate the auth layer".to_string(),
            goal: Some("One login path".to_string()),
            revision: 2,
            phases: vec![
                phase(
                    "research",
                    "Research",
                    PhaseStatus::Completed,
                    PhaseCounts {
                        total: 3,
                        done: 3,
                        ..PhaseCounts::default()
                    },
                ),
                phase(
                    "impl",
                    "Implementation",
                    PhaseStatus::InProgress,
                    PhaseCounts {
                        total: 4,
                        done: 1,
                        running: 2,
                        failed: 1,
                        quiet: 1,
                        blocked: 0,
                    },
                ),
                phase(
                    "verify",
                    "Verification",
                    PhaseStatus::Pending,
                    PhaseCounts::default(),
                ),
            ],
        };
        let mut child = row("c1", SubagentRunStatus::Running);
        child.phase_id = Some("impl".to_string());
        let text = render_text(&roster(vec![child], Some(plan)), false);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Plan: Migrate the auth layer");
        assert_eq!(lines[1], "  Goal: One login path");
        assert_eq!(lines[2], "  ● Research        3/3 done");
        assert_eq!(
            lines[3],
            "  ◐ Implementation  1/4 done · 2 running · 1 failed · 1 quiet"
        );
        assert_eq!(lines[4], "  ○ Verification    0/0 done");
        let subagents_at = lines
            .iter()
            .position(|line| line.starts_with("Subagents of session"))
            .expect("the children follow the plan");
        assert!(subagents_at > 4);
        assert!(text.contains("· phase Implementation"), "{text}");
    }

    #[test]
    fn an_empty_roster_says_so_and_still_shows_a_plan() {
        let text = render_text(&roster(Vec::new(), None), false);
        assert_eq!(text, "No subagents for session 20261104_parent.");
        let plan = OrchestrationSnapshot {
            title: "Plan".to_string(),
            goal: None,
            revision: 1,
            phases: vec![phase(
                "p",
                "Plan",
                PhaseStatus::Pending,
                PhaseCounts::default(),
            )],
        };
        let text = render_text(&roster(Vec::new(), Some(plan)), false);
        assert!(text.starts_with("Plan: Plan\n"), "{text}");
        assert!(
            text.ends_with("No subagents for session 20261104_parent."),
            "{text}"
        );
    }

    #[test]
    fn a_child_with_no_name_or_model_still_reads() {
        let mut child = row("c9", SubagentRunStatus::Unknown);
        child.name = SUBAGENT_NAME_PREFIX.to_string();
        child.model_name = None;
        child.provider_name = None;
        child.elapsed_seconds = None;
        child.tool_call_count = 1;
        let text = render_text(&roster(vec![child.clone()], None), false);
        assert!(text.contains("1. · c9  unknown"), "{text}");
        assert!(text.contains("no model recorded"), "{text}");
        assert!(text.contains("1 tool call"), "{text}");
        child.role = Some("scout".to_string());
        let text = render_text(&roster(vec![child], None), false);
        assert!(text.contains("1. · scout  unknown"), "{text}");
    }

    #[test]
    fn model_written_text_cannot_drive_the_terminal() {
        let mut child = row("c1", SubagentRunStatus::Completed);
        child.name = format!("{SUBAGENT_NAME_PREFIX}evil\u{1b}[2J\rname");
        child.result_line = Some("line one\nforged line\u{1b}]0;title\u{7}".to_string());
        let text = render_text(&roster(vec![child], None), false);
        assert!(!text.contains('\u{1b}'), "{text:?}");
        assert!(!text.contains('\r'), "{text:?}");
        assert!(text.contains("evil [2J name"), "{text:?}");
        assert!(text.contains("line one forged line ]0;title"), "{text:?}");
        assert_eq!(
            text.lines().filter(|l| l.contains("forged")).count(),
            1,
            "a newline in a result cannot start a line of its own: {text:?}"
        );
    }

    #[test]
    fn elapsed_reads_in_the_largest_two_units() {
        assert_eq!(format_elapsed(0), "0s");
        assert_eq!(format_elapsed(59), "59s");
        assert_eq!(format_elapsed(60), "1m00s");
        assert_eq!(format_elapsed(102), "1m42s");
        assert_eq!(format_elapsed(3600), "1h00m");
        assert_eq!(format_elapsed(7500), "2h05m");
    }

    #[test]
    fn the_route_json_round_trips_and_is_what_json_prints() {
        let original = roster(vec![row("c1", SubagentRunStatus::Blocked)], None);
        let json = render(&original, SubagentsFormat::Json, false).unwrap();
        // Chunked framing around the object, as a raw HTTP body can carry.
        let framed = format!("1a2\r\n{json}\r\n0\r\n\r\n");
        assert_eq!(parse_roster(&framed).unwrap(), original);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["parentSessionId"], "20261104_parent");
        assert_eq!(value["subagents"][0]["status"], "blocked");
        assert!(parse_roster("not json").is_err());
    }

    #[test]
    fn a_phase_never_names_a_child_the_roster_omitted() {
        let mut p = phase("p", "P", PhaseStatus::InProgress, PhaseCounts::default());
        p.agent_ids = vec!["c1".to_string(), "hidden".to_string()];
        let mut r = roster(
            vec![row("c1", SubagentRunStatus::Running)],
            Some(OrchestrationSnapshot {
                title: "T".to_string(),
                goal: None,
                revision: 1,
                phases: vec![p],
            }),
        );
        confine_plan(&mut r);
        assert_eq!(r.plan.unwrap().phases[0].agent_ids, vec!["c1".to_string()]);
    }

    async fn seed(
        manager: &SessionManager,
        dir: &std::path::Path,
        name: &str,
        kind: biorouter::session::SessionType,
        parent: Option<&str>,
        tier: SessionClassification,
    ) -> String {
        let session = manager
            .create_session(dir.to_path_buf(), name.to_string(), kind)
            .await
            .unwrap();
        let mut update = manager
            .update(&session.id)
            .parent_session_id(parent.map(str::to_string));
        if tier == SessionClassification::Private {
            update = update
                .provider_name("versa_azure")
                .model_config(biorouter::model::ModelConfig::new("gpt-4o").unwrap())
                .raise_privacy(SessionClassification::Private, "turn:versa_azure");
        }
        update.apply().await.unwrap();
        session.id
    }

    /// The local path answers as the route does: a public terminal is refused a
    /// private parent, and is not listed a private child of a public one.
    #[tokio::test]
    async fn the_local_store_applies_the_routes_tier_rule() {
        use biorouter::session::SessionType;
        let dir = tempfile::TempDir::new().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf());
        let public_parent = seed(
            &manager,
            dir.path(),
            "Public parent",
            SessionType::User,
            None,
            SessionClassification::Public,
        )
        .await;
        let public_child = seed(
            &manager,
            dir.path(),
            "Subagent: public child",
            SessionType::SubAgent,
            Some(&public_parent),
            SessionClassification::Public,
        )
        .await;
        let private_child = seed(
            &manager,
            dir.path(),
            "Subagent: private child",
            SessionType::SubAgent,
            Some(&public_parent),
            SessionClassification::Private,
        )
        .await;
        let private_parent = seed(
            &manager,
            dir.path(),
            "Private parent",
            SessionType::User,
            None,
            SessionClassification::Private,
        )
        .await;

        let ids = |roster: &SessionSubagents| -> Vec<String> {
            roster
                .subagents
                .iter()
                .map(|row| row.session_id.clone())
                .collect()
        };

        // A private terminal sees both, which proves the omission below is the
        // filter and not an empty roster.
        let all = from_store(&manager, &public_parent, ProviderTier::Private, true)
            .await
            .unwrap();
        assert!(ids(&all).contains(&public_child) && ids(&all).contains(&private_child));

        let public = from_store(&manager, &public_parent, ProviderTier::Public, true)
            .await
            .unwrap();
        assert_eq!(ids(&public), vec![public_child.clone()]);

        let refused = from_store(&manager, &private_parent, ProviderTier::Public, true)
            .await
            .unwrap_err();
        assert_eq!(refused.to_string(), LOCAL_OUT_OF_REACH);
        assert!(
            from_store(&manager, &private_parent, ProviderTier::Private, true)
                .await
                .is_ok()
        );

        // With the master switch off nothing is filtered, as the route's gate is
        // inert then too.
        let off = from_store(&manager, &private_parent, ProviderTier::Public, false).await;
        assert!(off.is_ok());

        let missing = from_store(&manager, "20990101_1", ProviderTier::Private, true)
            .await
            .unwrap_err();
        assert!(missing.to_string().contains("not found"), "{missing}");
    }

    #[test]
    fn the_local_note_says_live_state_needs_a_daemon() {
        let note = local_note("no daemon");
        assert!(note.starts_with("note: "));
        assert!(
            note.contains("Live state needs a running biorouter"),
            "{note}"
        );
        assert!(note.contains("interrupted or unknown"), "{note}");
    }
}
