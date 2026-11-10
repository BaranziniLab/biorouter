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

use std::collections::{HashMap, HashSet};
use std::io::IsTerminal;

use anyhow::{anyhow, bail, Result};
// `QUIET_AFTER_SECONDS`: a child active but silent for at least this long is
// shown as quiet, a heuristic, so it is labelled "quiet", never "stalled". It is
// the plan's own constant, which also decides each phase's quiet count, so a row
// and the phase line printed above it can never disagree; the TUI pane and the
// desktop panel use the same value.
use biorouter::agents::orchestration_plan::{
    OrchestrationSnapshot, PhaseSnapshot, PhaseStatus, QUIET_AFTER_SECONDS,
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

/// Longest model label a row prints, in characters (the desktop's
/// `MODEL_LABEL_MAX`).
const MODEL_LABEL_MAX: usize = 24;

/// A Bedrock-style id (`us.anthropic.claude-sonnet-4-5-20250929-v1:0`): an
/// optional region, the vendor, then the model.
static BEDROCK_ID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^(?:[a-z]{2}\.)?(?:anthropic|meta|amazon|cohere|mistral|ai21)\.(.+)$")
        .expect("a valid pattern")
});

/// Suffixes a model id carries that say nothing a row needs, removed in this
/// order: `:latest`, a Bedrock version (`-v1:0`), a release date
/// (`-20251101`, `@20250514`) and a dashed one (`-2026-04-24`).
static MODEL_SUFFIXES: std::sync::LazyLock<[regex::Regex; 4]> = std::sync::LazyLock::new(|| {
    [
        r":latest$",
        r"-v\d+:\d+$",
        r"[-@]20\d{6}$",
        r"-\d{4}-\d{2}-\d{2}$",
    ]
    .map(|pattern| regex::Regex::new(pattern).expect("a valid pattern"))
});

/// A model name short enough for a row: no provider path
/// (`accounts/fireworks/models/`, `us.anthropic.`), no release date and no
/// `:latest`, at most [`MODEL_LABEL_MAX`] characters. The provider's name when
/// the row has no model, and `None` when it has neither.
///
/// ⚠ The desktop's `shortModelLabel` (`ui/desktop/src/utils/sessionSubagents.ts`)
/// is the same rule, step for step, so a child reads as the same model on every
/// surface: `gpt-5.5-2026-04-24` is `gpt-5.5` in the side panel, the TUI pane and
/// this command alike.
pub fn short_model_label(model: Option<&str>, provider: Option<&str>) -> Option<String> {
    let raw = model.map(str::trim).unwrap_or_default();
    if raw.is_empty() {
        return provider
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string);
    }
    let mut label = raw.rsplit('/').next().unwrap_or(raw).to_string();
    if let Some(rest) = BEDROCK_ID.captures(&label).and_then(|c| c.get(1)) {
        label = rest.as_str().to_string();
    }
    for suffix in MODEL_SUFFIXES.iter() {
        label = suffix.replace(&label, "").into_owned();
    }
    if label.is_empty() {
        label = raw.to_string();
    }
    if label.chars().count() > MODEL_LABEL_MAX {
        let mut cut: String = label.chars().take(MODEL_LABEL_MAX - 1).collect();
        cut.push('…');
        return Some(cut);
    }
    Some(label)
}

/// The counts of a child's setup that are not zero: `3 ext · 1 skill ·
/// 2 knowledge bases`, in that order (the desktop's). Knowledge bases are
/// written out, because "2 KB" reads as two kilobytes; `compact` writes them
/// `2 kb bases`, for a row too narrow for the words.
pub fn setup_counts(
    extensions: usize,
    skills: usize,
    knowledge_bases: usize,
    compact: bool,
) -> Vec<String> {
    let mut parts = Vec::new();
    if extensions > 0 {
        parts.push(format!("{extensions} ext"));
    }
    if skills > 0 {
        parts.push(plural(skills, "skill", "skills"));
    }
    if knowledge_bases > 0 {
        parts.push(if compact {
            plural(knowledge_bases, "kb base", "kb bases")
        } else {
            plural(knowledge_bases, "knowledge base", "knowledge bases")
        });
    }
    parts
}

/// `3 ext · 1 skill · 1 knowledge base`, or nothing for a child with none.
pub fn setup_summary(row: &SubagentRow) -> String {
    setup_counts(
        row.extensions.len(),
        row.skills.len(),
        row.knowledge_bases.len(),
        false,
    )
    .join(" · ")
}

/// Widest a subagent's session name is printed under its role, in columns.
const NAME_LINE_MAX: usize = 72;

/// `text` cut to `max` display columns, ending in `…` when it was cut.
pub fn clip_columns(text: &str, max: usize) -> String {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if UnicodeWidthStr::width(text) <= max {
        return text.to_string();
    }
    let budget = max.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + width > budget {
            break;
        }
        out.push(ch);
        used += width;
    }
    out.push('…');
    out
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

/// The session name without its prefix, safe to print.
fn session_name(row: &SubagentRow) -> String {
    printable(
        row.name
            .strip_prefix(SUBAGENT_NAME_PREFIX)
            .unwrap_or(&row.name),
    )
}

fn role(row: &SubagentRow) -> Option<String> {
    row.role
        .as_deref()
        .map(printable)
        .filter(|role| !role.is_empty())
}

/// What a row leads with: the role the parent gave the child (`celsius`), else
/// its session name, which is derived from the instructions and so reads like a
/// sentence. The desktop's `subagentTitle`.
fn row_label(row: &SubagentRow) -> String {
    role(row).unwrap_or_else(|| {
        let name = session_name(row);
        if name.is_empty() {
            row.session_id.clone()
        } else {
            name
        }
    })
}

/// The session name, printed second, when the row leads with a role that says
/// something else.
fn secondary_name(row: &SubagentRow) -> Option<String> {
    let role = role(row)?;
    let name = session_name(row);
    (!name.is_empty() && !name.eq_ignore_ascii_case(&role)).then_some(name)
}

fn dim(text: &str, color: bool) -> String {
    if color {
        console::style(text).dim().force_styling(true).to_string()
    } else {
        text.to_string()
    }
}

/// What a phase row says about its gate and the gate that holds it, said to
/// the person reading: ` · gate set by the agent, passed by you · needs you`,
/// ` · waits for Research's gate`. Empty for an ungated phase nothing holds.
/// A held phase names its holder by title, as the row names phases.
pub fn gate_suffix(phase: &PhaseSnapshot, titles: &HashMap<&str, &str>) -> String {
    use super::session_gates::{decision_words, held_words, owner_words};
    let mut suffix = String::new();
    if let Some(gate) = &phase.gate {
        suffix.push_str(" · gate ");
        suffix.push_str(owner_words(gate.owner));
        if let Some(decision) = gate.person_decision {
            suffix.push_str(", ");
            suffix.push_str(decision_words(decision));
        }
        if gate.needs_person && !phase.status.is_closed() {
            suffix.push_str(" · needs you");
        }
    }
    if let Some(holder) = phase.held_by.as_deref() {
        let title = titles.get(holder).copied().unwrap_or(holder);
        suffix.push_str(" · ");
        suffix.push_str(&held_words(title));
    }
    suffix
}

/// One line per phase: glyph, title, `done/total done`, the counts that are
/// not zero, then who set its gate and what holds it.
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
    let titles: HashMap<&str, &str> = plan
        .phases
        .iter()
        .map(|phase| (phase.id.as_str(), phase.title.as_str()))
        .collect();
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
        line.push_str(&gate_suffix(phase, &titles));
        out.push(line);
    }
}

/// The text `biorouter session subagents` prints: the plan first when there is
/// one, then one numbered entry per child, then how to open one.
///
/// Pure, so it is tested with hand-built rosters; `color` only dims the session
/// name printed under a role and the result lines.
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
            row_label(row),
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
        if let Some(name) = secondary_name(row) {
            out.push(format!(
                "       {}",
                dim(&clip_columns(&name, NAME_LINE_MAX), color)
            ));
        }

        let model = short_model_label(row.model_name.as_deref(), row.provider_name.as_deref())
            .map(|label| printable(&label))
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| "no model recorded".to_string());
        let mut detail = vec![row.session_id.clone(), model];
        let setup = setup_summary(row);
        if !setup.is_empty() {
            detail.push(setup);
        }
        if row.tool_call_count > 0 {
            detail.push(plural(
                row.tool_call_count as usize,
                "tool call",
                "tool calls",
            ));
        }
        if let Some(elapsed) = row.elapsed_seconds {
            detail.push(format_elapsed(elapsed));
        }
        out.push(format!("       {}", detail.join("  ")));
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
pub(crate) fn parse_roster(body: &str) -> Result<SessionSubagents> {
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
pub(crate) async fn from_store(
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
        super::print_line(render(&roster, format, color)?);
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
                super::print_line(&frame);
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
            gate: None,
            held_by: None,
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
        child.knowledge_bases = vec!["soul".into()];
        let text = render_text(&roster(vec![child], None), false);
        let lines: Vec<&str> = text.lines().collect();
        let head = lines
            .iter()
            .position(|line| line.contains("1. ◐ reviewer  running"))
            .unwrap_or_else(|| panic!("the role leads the row: {text}"));
        assert_eq!(
            lines[head + 1],
            "       audit the c1 module",
            "the session name follows it: {text}"
        );
        assert!(lines[head].contains("quiet 4m00s"), "{text}");
        assert!(
            !text.contains(SUBAGENT_NAME_PREFIX),
            "the prefix is dropped: {text}"
        );
        assert!(
            text.contains("c1  gpt-5.5  3 ext · 1 skill · 1 knowledge base  12 tool calls  1m42s"),
            "{text}"
        );
        assert!(
            text.contains("biorouter session attach <session id>"),
            "{text}"
        );
    }

    /// The instruction-derived name reads like a sentence; the role the parent
    /// gave is what tells siblings apart, so it leads, and the name is printed
    /// under it (dimmed, and cut short) rather than lost.
    #[test]
    fn a_role_leads_and_the_long_name_follows_dimmed_and_clipped() {
        let mut child = row("c1", SubagentRunStatus::Completed);
        child.name = format!(
            "{SUBAGENT_NAME_PREFIX}Convert 37 degrees Celsius to Fahrenheit and report only the \
             number, rounded to one decimal place, with no explanation at all"
        );
        child.role = Some("celsius".to_string());
        let plain = render_text(&roster(vec![child.clone()], None), false);
        let lines: Vec<&str> = plain.lines().collect();
        let head = lines
            .iter()
            .position(|line| line.starts_with("  1. "))
            .unwrap();
        assert_eq!(lines[head], "  1. ● celsius  completed", "{plain}");
        let name = lines[head + 1].trim_start();
        assert!(name.starts_with("Convert 37 degrees Celsius"), "{plain}");
        assert!(name.ends_with('…'), "a long name is cut: {plain}");
        assert!(
            unicode_width::UnicodeWidthStr::width(name) <= NAME_LINE_MAX,
            "{plain}"
        );
        let colored = render_text(&roster(vec![child.clone()], None), true);
        assert!(
            colored.contains("\u{1b}[2mConvert 37"),
            "the name is dimmed: {colored:?}"
        );

        // A role equal to the name, or none at all, prints no second line.
        child.role = Some(session_name(&child));
        let same = render_text(&roster(vec![child.clone()], None), false);
        assert_eq!(same.lines().filter(|l| l.contains("Convert 37")).count(), 1);
        child.role = None;
        let unnamed = render_text(&roster(vec![child], None), false);
        assert!(unnamed.contains("1. ● Convert 37 degrees"), "{unnamed}");
        assert_eq!(
            unnamed.lines().filter(|l| l.contains("Convert 37")).count(),
            1
        );
    }

    /// The row prints the desktop's short model label, and only the counts that
    /// are not zero: "0 ext · 0 skills · 0 kb" said nothing, and "kb" alone was
    /// ambiguous.
    #[test]
    fn the_detail_line_has_a_short_model_and_no_zero_counts() {
        let mut child = row("c1", SubagentRunStatus::Completed);
        child.model_name = Some("gpt-5.5-2026-04-24".to_string());
        child.extensions.clear();
        child.skills.clear();
        child.tool_call_count = 0;
        let text = render_text(&roster(vec![child.clone()], None), false);
        assert!(text.contains("       c1  gpt-5.5  1m42s"), "{text}");
        for zero in ["0 ext", "0 skills", "0 kb", "0 tool calls", "2026-04-24"] {
            assert!(!text.contains(zero), "{zero:?} in {text}");
        }
        child.knowledge_bases = vec!["soul".into(), "lab".into()];
        child.skills = vec!["ggplot".into()];
        let text = render_text(&roster(vec![child], None), false);
        assert!(
            text.contains("c1  gpt-5.5  1 skill · 2 knowledge bases  1m42s"),
            "{text}"
        );
    }

    #[test]
    fn short_model_label_matches_the_desktop_rule() {
        let label = |model: &str| short_model_label(Some(model), None).unwrap();
        // The cases `shortModelLabel` is tested with in `sessionSubagents.test.ts`.
        assert_eq!(label("claude-sonnet-4-5-20250929"), "claude-sonnet-4-5");
        assert_eq!(label("accounts/fireworks/models/qwen3-235b"), "qwen3-235b");
        assert_eq!(
            label("us.anthropic.claude-opus-4-1-20250805-v1:0"),
            "claude-opus-4-1"
        );
        assert_eq!(label("qwen3.8:latest"), "qwen3.8");
        assert_eq!(label("gpt-5.5-2026-01-15"), "gpt-5.5");
        assert_eq!(
            label("a-really-long-model-name-that-goes-on-and-on"),
            "a-really-long-model-nam…"
        );
        assert_eq!(
            short_model_label(None, Some("ollama")).as_deref(),
            Some("ollama")
        );
        assert_eq!(
            short_model_label(Some("  "), Some("ollama")).as_deref(),
            Some("ollama")
        );
        assert_eq!(short_model_label(None, None), None);
        // The live finding.
        assert_eq!(label("gpt-5.5-2026-04-24"), "gpt-5.5");
        assert_eq!(label("claude-opus-4-5@20250514"), "claude-opus-4-5");
        // Nothing left after the path: the raw id rather than an empty label.
        assert_eq!(label("models/"), "models/");
    }

    #[test]
    fn setup_counts_skip_zeros_and_spell_out_knowledge_bases() {
        assert!(setup_counts(0, 0, 0, false).is_empty());
        assert_eq!(
            setup_counts(3, 1, 1, false),
            ["3 ext", "1 skill", "1 knowledge base"]
        );
        assert_eq!(
            setup_counts(0, 2, 2, false),
            ["2 skills", "2 knowledge bases"]
        );
        assert_eq!(setup_counts(0, 0, 1, true), ["1 kb base"]);
        assert_eq!(setup_counts(0, 0, 3, true), ["3 kb bases"]);
        for compact in [false, true] {
            let text = setup_counts(1, 1, 2, compact).join(" · ");
            assert!(!text.contains("KB") && !text.ends_with(" kb"), "{text}");
        }
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

    /// A phase row says who set its gate, the person's own decision on it,
    /// that it needs the person, and which gate holds a later phase, by
    /// title.
    #[test]
    fn phase_rows_say_who_set_a_gate_and_what_waits_for_it() {
        use biorouter::agents::orchestration_plan::{
            GateOwner, PersonGateAction, PhaseGateSnapshot,
        };
        let gate = |owner: GateOwner| PhaseGateSnapshot {
            threshold: 0.8,
            question_count: 1,
            attempts: 0,
            max_attempts: 3,
            last_passed: None,
            last_min_p: None,
            blocked: false,
            owner,
            questions: Default::default(),
            model: None,
            model_adapter: None,
            allow_uncalibrated: false,
            last_error: None,
            person_decision: None,
            retries_granted: 0,
            needs_person: false,
            holds: Vec::new(),
        };
        let mut research = phase(
            "research",
            "Research",
            PhaseStatus::Blocked,
            PhaseCounts::default(),
        );
        let mut blocked = gate(GateOwner::Agent);
        blocked.needs_person = true;
        research.gate = Some(blocked);
        let mut write = phase(
            "write",
            "Write",
            PhaseStatus::Pending,
            PhaseCounts::default(),
        );
        write.held_by = Some("research".to_string());
        let mut plan_phase = phase(
            "plan",
            "Plan",
            PhaseStatus::Completed,
            PhaseCounts::default(),
        );
        let mut passed = gate(GateOwner::Workflow);
        passed.person_decision = Some(PersonGateAction::Pass);
        plan_phase.gate = Some(passed);
        let mut mine = phase(
            "verify",
            "Verify",
            PhaseStatus::Pending,
            PhaseCounts::default(),
        );
        mine.gate = Some(gate(GateOwner::Person));
        mine.held_by = Some("gone".to_string());
        let plan = OrchestrationSnapshot {
            title: "Moon".to_string(),
            goal: None,
            revision: 3,
            phases: vec![plan_phase, research, write, mine],
        };
        let text = render_text(&roster(Vec::new(), Some(plan)), false);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[1],
            "  ● Plan      0/0 done · gate set by the workflow, passed by you"
        );
        assert_eq!(
            lines[2],
            "  ⏸ Research  0/0 done · gate set by the agent · needs you"
        );
        assert_eq!(
            lines[3],
            "  ○ Write     0/0 done · waits for Research's gate"
        );
        assert_eq!(
            lines[4], "  ○ Verify    0/0 done · gate set by you · waits for gone's gate",
            "a holder the plan does not list is named by its id"
        );
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
