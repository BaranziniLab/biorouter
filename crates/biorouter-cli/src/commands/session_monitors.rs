//! `biorouter session monitors <SESSION_ID> list|add|remove`: the decision-model
//! monitors that watch one chat.
//!
//! A monitor asks a decision model one question about a rolling window of the
//! chat after each agent iteration or at the end of a turn, and when the
//! probability of the wanted answer reaches its threshold (or, with `--below`,
//! falls below it) it stops the turn, shows a notice, or tells the model.
//! Monitors are stored in the chat's `decision_settings.v1`, the same record
//! the desktop's Monitors dialog edits.
//!
//! * **With a reachable daemon** the record is read with
//!   `GET /sessions/{id}/decision-settings` and written with `PUT` on the same
//!   path. A write names where the chat's content goes, so a daemon that holds
//!   a user-action key wants it; see
//!   `session_watch::protected_write_with_key_if_wanted`. The daemon checks
//!   that what the save turns on can run for the chat (400 for a malformed
//!   record, 422 for one that cannot run now), and its sentence is printed
//!   as it is: neither refusal is about the key.
//! * **With none** the same record is read and written in this process, through
//!   `biorouter::decision::settings`, under the tier rule the route applies,
//!   and stored with the same check the route makes
//!   (`biorouter::decision::save_checked`), with the chat's bound model read
//!   off its session row rather than its agent. It does not copy a monitor set
//!   to also watch subagents onto running ones: without a daemon none is
//!   running.
//!
//! Every edit reads the whole record, changes one entry and writes the whole
//! record back, so the other entries keep who set them (`setBy`,
//! `setBySession`, `inheritedFrom`) byte for byte. The person may remove any
//! monitor, one an agent set included.

use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use biorouter::decision::settings::{
    DecisionSettings, MonitorAction, MonitorCriterion, MonitorDirection, MonitorErrorPolicy,
    MonitorOwner, MonitorTiming, MonitorWindow, MAX_MONITORS,
};
use biorouter::decision::{DecisionKind, DecisionModelRef, DecisionService, Question, Threshold};
use biorouter::session::{SessionManager, SessionType};

use super::apps::configured_port;
use super::decide::{clip, format_p, parse_choice, parse_probability, terminal_text};
use super::session_decisions::{check_local_reach, daemon_error, parse_json, reach, Reach};
use super::session_watch::{
    daemon_get_json, protected_write_with_key_if_wanted, refusal_sentence, DaemonAuth,
};

const REQUEST_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
/// Longest a monitor's name may be.
pub const MAX_NAME_CHARS: usize = 60;
/// Longest a generated id is.
const MAX_ID_CHARS: usize = 40;

/// What a terminal on a public model is told about a private chat on the local
/// path.
pub const LOCAL_OUT_OF_REACH: &str =
    "This chat is private, and this terminal is configured for a model hosted outside your \
     institution, so its monitors are not shown or changed here. Run this from a terminal \
     configured for a private model, or open the chat in the desktop app.";

/// What `add --subagents` is refused with on a subagent's own chat, which
/// starts no subagents of its own.
pub const SUBAGENTS_ON_A_SUBAGENT: &str =
    "Only a chat that starts subagents can share its monitors with them.";

/// `--timing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TimingArg {
    #[value(alias = "post_iteration")]
    PostIteration,
    #[value(alias = "turn_end")]
    TurnEnd,
}

impl From<TimingArg> for MonitorTiming {
    fn from(timing: TimingArg) -> Self {
        match timing {
            TimingArg::PostIteration => MonitorTiming::PostIteration,
            TimingArg::TurnEnd => MonitorTiming::TurnEnd,
        }
    }
}

/// `--action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ActionArg {
    Stop,
    Notify,
    Steer,
}

impl From<ActionArg> for MonitorAction {
    fn from(action: ActionArg) -> Self {
        match action {
            ActionArg::Stop => MonitorAction::Stop,
            ActionArg::Notify => MonitorAction::Notify,
            ActionArg::Steer => MonitorAction::Steer,
        }
    }
}

/// `--on-error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OnErrorArg {
    /// Show a notice and skip the check; after three failures in a row the
    /// monitor is off for the rest of the turn.
    Notify,
    /// Stop the turn whenever a check fails, so the rule never goes unchecked.
    Pause,
}

impl From<OnErrorArg> for MonitorErrorPolicy {
    fn from(policy: OnErrorArg) -> Self {
        match policy {
            OnErrorArg::Notify => MonitorErrorPolicy::Notify,
            OnErrorArg::Pause => MonitorErrorPolicy::Pause,
        }
    }
}

/// `session monitors <id> add`.
#[derive(clap::Args, Debug, Clone)]
pub struct AddMonitor {
    /// A short name for the monitor (at most 60 characters).
    #[arg(long)]
    pub name: String,
    /// The question the decision model answers about the chat's recent window.
    #[arg(long)]
    pub ask: String,
    /// Fire when the probability of the wanted answer is at least this (above
    /// 0, at most 1), or falls below it with --below.
    #[arg(long, value_name = "P", value_parser = super::decide::parse_positive_probability)]
    pub threshold: f64,
    /// Fire when the probability falls below --threshold instead of reaching it.
    #[arg(long)]
    pub below: bool,
    /// What to do when it fires: stop the turn, show a notice, or tell the model.
    #[arg(long, value_enum, default_value_t = ActionArg::Notify)]
    pub action: ActionArg,
    /// When to ask: after every iteration of the agent loop, or once as a turn ends.
    #[arg(long, value_enum, default_value_t = TimingArg::PostIteration)]
    pub timing: TimingArg,
    /// The decision model, by its name (tev1:0.8b, joey-1) or as
    /// provider/model. Without it the chat's default decision model is used.
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,
    /// For a choice question: one option, `name` or `name=description`. Repeat it.
    #[arg(long = "choice", value_name = "NAME[=DESCRIPTION]", value_parser = parse_choice)]
    pub choices: Vec<(String, Option<String>)>,
    /// The answer whose probability is compared: for a yes/no question `yes`
    /// (the default) or `no`; for a choice question one of the --choice
    /// options.
    #[arg(long, value_name = "OPTION")]
    pub option: Option<String>,
    /// Checks in a row on the firing side of the threshold before it fires (1
    /// to 5).
    #[arg(
        long,
        value_name = "N",
        default_value_t = 1,
        value_parser = clap::value_parser!(u32).range(1..=5)
    )]
    pub consecutive: u32,
    /// After firing, re-arm only once the probability drops below this. For a
    /// stop monitor it also keeps the monitor from stopping later turns while
    /// the answer holds; without it a stop monitor stops every turn in which
    /// its answer holds.
    #[arg(long = "rearm-below", value_name = "P", value_parser = parse_probability)]
    pub rearm_below: Option<f64>,
    /// For a --below monitor: after firing, re-arm only once the probability
    /// rises to this or above (over --threshold). For a stop monitor it also
    /// keeps the monitor from stopping later turns while the answer holds.
    #[arg(long = "rearm-above", value_name = "P", value_parser = parse_probability)]
    pub rearm_above: Option<f64>,
    /// What to do when a check fails: notify shows a notice, pause stops the
    /// turn whenever a check fails.
    #[arg(long = "on-error", value_enum, default_value_t = OnErrorArg::Notify)]
    pub on_error: OnErrorArg,
    /// At most this many checks per turn (1 to 200).
    #[arg(
        long = "max-checks",
        value_name = "N",
        default_value_t = 50,
        value_parser = clap::value_parser!(u32).range(1..=200)
    )]
    pub max_checks: u32,
    /// An id for the monitor; made from the name when not given.
    #[arg(long)]
    pub id: Option<String>,
    /// Add it switched off.
    #[arg(long)]
    pub disabled: bool,
    /// Also watch the subagents this chat starts (the monitor is copied onto
    /// each of them).
    #[arg(long)]
    pub subagents: bool,
}

/// What `session monitors` does.
#[derive(clap::Subcommand, Debug, Clone)]
pub enum MonitorsCommand {
    /// List the chat's monitors and its default decision model.
    List {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
        /// Output format (text, json); --format json is the same as --json.
        #[arg(short, long, value_enum, default_value_t = super::TextOrJson::Text)]
        format: super::TextOrJson,
    },
    /// Add a monitor.
    Add(AddMonitor),
    /// Remove a monitor by its id or name.
    Remove {
        /// The monitor's id or name.
        monitor: String,
    },
}

/// `form-reached` from `Form reached!`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(MAX_ID_CHARS).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "monitor".to_string()
    } else {
        out
    }
}

/// The monitor `add` describes, with an id no other monitor of the chat has.
pub fn build_monitor(add: &AddMonitor, existing: &[MonitorCriterion]) -> Result<MonitorCriterion> {
    let name = add.name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        bail!("a monitor's name must be 1 to {MAX_NAME_CHARS} characters");
    }
    if existing.len() >= MAX_MONITORS {
        bail!("a chat takes at most {MAX_MONITORS} monitors; remove one first");
    }
    let kind = if add.choices.is_empty() {
        DecisionKind::Binary {}
    } else {
        DecisionKind::Choice {
            options: add.choices.iter().cloned().collect(),
        }
    };
    if add.ask.trim().is_empty() {
        bail!("{}", super::decide::EMPTY_ASK);
    }
    let option = watched_option(&kind, add.option.as_deref())?;
    let question = Question {
        instructions: add.ask.trim().to_string(),
        kind,
        threshold: None,
    };
    let questions = std::iter::once(("monitor".to_string(), question.clone())).collect();
    biorouter::decision::types::validate_questions(&questions).map_err(|e| anyhow!("{e}"))?;
    let model = match add.model.as_deref() {
        Some(raw) => Some(DecisionModelRef::parse(raw).ok_or_else(|| {
            anyhow!("--model takes `provider/model`, for example `ollama_decision/tev1:0.8b`")
        })?),
        None => None,
    };
    let direction = direction_and_rearm(add)?;
    let id = match &add.id {
        Some(id) => {
            let id = id.trim().to_string();
            if slug(&id) != id {
                bail!(
                    "a monitor id is lowercase letters, digits and `-`, such as `{}`",
                    slug(&id)
                );
            }
            if existing.iter().any(|m| m.id == id) {
                bail!("this chat already has a monitor `{id}`");
            }
            id
        }
        None => unique_id(&slug(name), existing),
    };
    Ok(MonitorCriterion {
        id,
        name: name.to_string(),
        question,
        model,
        timing: add.timing.into(),
        window: MonitorWindow::default(),
        threshold: add.threshold,
        option,
        consecutive: add.consecutive,
        rearm_below: add.rearm_below,
        action: add.action.into(),
        on_error: add.on_error.into(),
        max_evals_per_turn: add.max_checks,
        enabled: !add.disabled,
        direction,
        rearm_above: add.rearm_above,
        // The person sets every monitor this command adds.
        set_by: MonitorOwner::Person,
        set_by_session: None,
        inherited_from: None,
        apply_to_subagents: add.subagents,
    })
}

/// The answer `--option` names for a question of `kind`. On a yes/no
/// question it picks the answer watched: `yes`, the default, is stored as
/// none (as the desktop stores it), and `no` as `no`. A choice question needs
/// one of its options.
fn watched_option(kind: &DecisionKind, option: Option<&str>) -> Result<Option<String>> {
    Ok(match (kind, option) {
        (DecisionKind::Binary {}, None) => None,
        (DecisionKind::Binary {}, Some(answer)) if answer.trim().eq_ignore_ascii_case("yes") => {
            None
        }
        (DecisionKind::Binary {}, Some(answer)) if answer.trim().eq_ignore_ascii_case("no") => {
            Some("no".to_string())
        }
        (DecisionKind::Binary {}, Some(_)) => {
            bail!("--option on a yes/no question is `yes` or `no`")
        }
        (DecisionKind::Choice { options }, None) => bail!(
            "say which option the threshold is on with --option (one of: {})",
            options.keys().cloned().collect::<Vec<_>>().join(", ")
        ),
        (DecisionKind::Choice { options }, Some(option)) if !options.contains_key(option) => {
            bail!("--option `{option}` is not one of the --choice options")
        }
        (_, option) => option.map(str::to_string),
    })
}

/// The side of its threshold `add` fires on (`--below`), with its re-arm
/// level checked: `--rearm-below` belongs to a monitor that fires at or
/// above, `--rearm-above` to a `--below` one, and each sits on the far side
/// of the threshold from where the monitor fires.
fn direction_and_rearm(add: &AddMonitor) -> Result<MonitorDirection> {
    if !add.below {
        if add.rearm_above.is_some() {
            bail!(
                "--rearm-above is for a --below monitor; a monitor that fires at or above its \
                 threshold re-arms with --rearm-below"
            );
        }
        if add.rearm_below.is_some_and(|rearm| rearm >= add.threshold) {
            bail!("--rearm-below must be under --threshold, or the monitor never re-arms");
        }
        return Ok(MonitorDirection::AtOrAbove);
    }
    if add.rearm_below.is_some() {
        bail!(
            "--rearm-below is for a monitor that fires at or above its threshold; a --below \
             monitor re-arms with --rearm-above"
        );
    }
    if add.rearm_above.is_some_and(|rearm| rearm <= add.threshold) {
        bail!(
            "--rearm-above must be over --threshold: it is how high the probability climbs back \
             before the monitor may fire again"
        );
    }
    Ok(MonitorDirection::Below)
}

/// `add --subagents` on a chat of this kind: refused on a subagent's own
/// chat, which starts no subagents.
pub fn refuse_subagents_on(session_type: SessionType) -> Result<()> {
    if session_type == SessionType::SubAgent {
        bail!("{SUBAGENTS_ON_A_SUBAGENT}");
    }
    Ok(())
}

fn unique_id(base: &str, existing: &[MonitorCriterion]) -> String {
    if !existing.iter().any(|m| m.id == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !existing.iter().any(|m| m.id == *candidate))
        .unwrap_or_else(|| base.to_string())
}

/// Remove the monitor named by id, else by name (case-blind). The removed one.
pub fn remove_monitor(settings: &mut DecisionSettings, wanted: &str) -> Result<MonitorCriterion> {
    let wanted = wanted.trim();
    let index = settings
        .monitors
        .iter()
        .position(|m| m.id == wanted)
        .or_else(|| {
            let matches: Vec<usize> = settings
                .monitors
                .iter()
                .enumerate()
                .filter(|(_, m)| m.name.eq_ignore_ascii_case(wanted))
                .map(|(i, _)| i)
                .collect();
            (matches.len() == 1).then(|| matches[0])
        })
        .ok_or_else(|| {
            let known = settings
                .monitors
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            if known.is_empty() {
                anyhow!("this chat has no monitors")
            } else {
                anyhow!("no monitor `{wanted}`; this chat has: {known}")
            }
        })?;
    Ok(settings.monitors.remove(index))
}

/// When a monitor is checked, as `list` and `run --explain` say it.
pub(crate) fn timing_text(timing: MonitorTiming) -> &'static str {
    match timing {
        MonitorTiming::PostIteration => "after each iteration",
        MonitorTiming::TurnEnd => "as the turn ends",
    }
}

/// What a monitor does when it fires, as `list` and `run --explain` say it.
pub(crate) fn action_text(action: MonitorAction) -> &'static str {
    match action {
        MonitorAction::Stop => "stops the turn",
        MonitorAction::Notify => "shows a notice",
        MonitorAction::Steer => "tells the model",
    }
}

/// What a monitor does when its check errors, as `list` and `run --explain`
/// say it.
pub(crate) fn error_text(on_error: MonitorErrorPolicy) -> &'static str {
    match on_error {
        MonitorErrorPolicy::Notify => "an error shows a notice",
        MonitorErrorPolicy::Pause => "an error pauses the turn",
    }
}

/// What a monitor compares, readable: `P(yes) ≥ 0.85`, `P(rrms) ≥ 0.70`, and
/// for a score `P(at least mild) ≥ 0.85`, the level named by its label whether
/// `option` gives the label or its index, as `decide` names the same threshold.
/// A monitor that fires below its threshold reads `P(yes) < 0.30`.
pub fn condition_text(monitor: &MonitorCriterion) -> String {
    let answer = match (monitor.pass_threshold(), &monitor.question.kind) {
        (Ok(Threshold::AtLeastLevel { level, .. }), DecisionKind::Score { levels }) => {
            let label = levels
                .get(level)
                .map(|label| label.trim())
                .filter(|label| !label.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("level {level}"));
            format!("at least {}", terminal_text(&label))
        }
        _ => terminal_text(monitor.option.as_deref().unwrap_or("yes")),
    };
    let sign = match monitor.direction {
        MonitorDirection::AtOrAbove => "≥",
        MonitorDirection::Below => "<",
    };
    format!("P({answer}) {sign} {}", format_p(monitor.threshold))
}

/// When a monitor that has fired may fire again: `re-arms below 0.70` /
/// `re-arms above 0.50` when it names the level, and for a stop monitor that
/// names none, that it stops again while its answer holds (it never latches,
/// by the engine's own rule, `MonitorCriterion::latches`). A notify or steer
/// monitor without a level re-arms at its threshold, which goes without
/// saying.
pub fn rearm_text(monitor: &MonitorCriterion) -> Option<String> {
    let rearm = match monitor.direction {
        MonitorDirection::AtOrAbove => monitor.rearm_below.map(|p| ("below", p)),
        MonitorDirection::Below => monitor.rearm_above.map(|p| ("above", p)),
    };
    match rearm {
        Some((side, p)) => Some(format!("re-arms {side} {}", format_p(p))),
        None if !monitor.latches() => Some("stops again while it holds".to_string()),
        None => None,
    }
}

/// Who set a monitor, when it was not the person on this chat: an agent (the
/// agent of the chat that started this subagent), or a copy of a monitor of
/// that chat set to also watch its subagents.
pub fn owner_text(monitor: &MonitorCriterion) -> Option<String> {
    if monitor.set_by == MonitorOwner::Agent {
        return Some(match monitor.set_by_session.as_deref() {
            Some(chat) => format!("set by the agent of chat {}", clip(chat, 80)),
            None => "set by an agent".to_string(),
        });
    }
    monitor.inherited_from.as_deref().map(|chat| {
        format!(
            "from the chat that started this subagent ({})",
            clip(chat, 80)
        )
    })
}

/// The monitors as text.
pub fn render_monitors(session_id: &str, settings: &DecisionSettings) -> Vec<String> {
    let mut out = Vec::new();
    if settings.monitors.is_empty() {
        out.push(format!("Session {session_id} has no monitors."));
        out.push(format!(
            "Add one: biorouter session monitors {session_id} add --name \"Form reached\" --ask \
             \"Has the agent reached the consent form?\" --threshold 0.85 --action stop"
        ));
    } else {
        out.push(format!(
            "Monitors of session {session_id} ({}):",
            settings.monitors.len()
        ));
        for (i, monitor) in settings.monitors.iter().enumerate() {
            let state = if monitor.enabled { "on" } else { "off" };
            let owner = owner_text(monitor)
                .map(|owner| format!(" · {owner}"))
                .unwrap_or_default();
            out.push(format!(
                "  {}. {}  {}  {state}{owner}",
                i + 1,
                clip(&monitor.id, 80),
                clip(&monitor.name, MAX_NAME_CHARS)
            ));
            out.push(format!(
                "     {}",
                clip(&monitor.question.instructions, 100)
            ));
            let mut details = vec![
                condition_text(monitor),
                if monitor.consecutive > 1 {
                    format!("{} in a row", monitor.consecutive)
                } else {
                    "once".to_string()
                },
                action_text(monitor.action).to_string(),
                timing_text(monitor.timing).to_string(),
            ];
            if let Some(rearm) = rearm_text(monitor) {
                details.push(rearm);
            }
            details.push(error_text(monitor.on_error).to_string());
            details.push(format!(
                "at most {} checks a turn",
                monitor.max_evals_per_turn
            ));
            if let Some(model) = &monitor.model {
                details.push(super::decide::model_for_person(model));
            }
            if monitor.apply_to_subagents {
                details.push("also watches the subagents this chat starts".to_string());
            }
            out.push(format!("     {}", details.join(" · ")));
        }
    }
    match &settings.default_model {
        Some(model) => out.push(format!(
            "Default decision model: {}",
            super::decide::model_for_person(model)
        )),
        None => out.push(
            "Default decision model: chosen automatically (local first, within the chat's tier)"
                .to_string(),
        ),
    }
    out
}

// ── reading and writing ─────────────────────────────────────────────────────

fn settings_path(session_id: &str) -> String {
    format!("/sessions/{session_id}/decision-settings")
}

/// The record a route answered with: the settings themselves, or an object
/// holding them under `settings`.
pub fn parse_settings(body: &str) -> Result<DecisionSettings> {
    let value: serde_json::Value = parse_json(body, "decision settings")?;
    let inner = match value.get("settings") {
        Some(settings) if settings.is_object() => settings.clone(),
        _ => value,
    };
    serde_json::from_value(inner)
        .map_err(|error| anyhow!("the daemon's answer was not decision settings: {error}"))
}

async fn daemon_load(session_id: &str, auth: &DaemonAuth) -> Result<DecisionSettings> {
    let path = settings_path(session_id);
    let (code, body) = daemon_get_json(&path, auth, configured_port(), REQUEST_DEADLINE).await?;
    if code != 200 {
        return Err(daemon_error(
            session_id,
            &format!("GET {path}"),
            code,
            &body,
        ));
    }
    parse_settings(&body)
}

async fn daemon_save(
    session_id: &str,
    settings: &DecisionSettings,
    auth: DaemonAuth,
) -> Result<DecisionSettings> {
    let path = settings_path(session_id);
    let body = serde_json::to_string(settings)?;
    let ((code, answer), _) = protected_write_with_key_if_wanted(
        "PUT",
        &path,
        &body,
        auth,
        configured_port(),
        REQUEST_DEADLINE,
    )
    .await?;
    save_answer(session_id, &path, code, &answer, settings)
}

/// What the daemon's answer to the settings `PUT` means: the record it
/// stored, or the refusal as the sentence a person reads.
///
/// A malformed record (400) and one whose monitor cannot run for this chat
/// now (422: the model is not there, cannot take the question, or the chat's
/// tier, an institution or a Crew grant refuses it) are answered with the
/// daemon's sentence alone. Neither is about the user-action key, so neither
/// carries the key advice a 403 does: a key would not change either answer.
fn save_answer(
    session_id: &str,
    path: &str,
    code: u16,
    answer: &str,
    sent: &DecisionSettings,
) -> Result<DecisionSettings> {
    match code {
        200 | 201 => parse_settings(answer).or_else(|_| Ok(sent.clone())),
        204 => Ok(sent.clone()),
        400 | 422 => Err(anyhow!(
            "{}",
            refusal_sentence(answer).unwrap_or_else(|| format!(
                "the daemon refused the change ({code}) without saying why. Nothing was saved."
            ))
        )),
        403 => Err(anyhow!(
            "the daemon refused the change: {}. Nothing was changed. A daemon started with a \
             user-action key wants it: pass --user-action-key-stdin and pipe the key as the \
             first line of stdin, or run this from a terminal.",
            refusal_sentence(answer).unwrap_or_else(|| "no reason given".to_string())
        )),
        other => Err(daemon_error(
            session_id,
            &format!("PUT {path}"),
            other,
            answer,
        )),
    }
}

/// The kind of chat `session_id` is, as the daemon reads it (its metadata
/// only, no transcript).
async fn daemon_session_type(session_id: &str, auth: &DaemonAuth) -> Result<SessionType> {
    let path = format!("/sessions/{session_id}?metadata_only=true");
    let (code, body) = daemon_get_json(&path, auth, configured_port(), REQUEST_DEADLINE).await?;
    if code != 200 {
        return Err(daemon_error(
            session_id,
            &format!("GET {path}"),
            code,
            &body,
        ));
    }
    session_type_of(&body)
}

/// A session's `session_type` from the daemon's `GET /sessions/{id}`; absent,
/// it reads as a person's chat, as the store's own default does.
fn session_type_of(body: &str) -> Result<SessionType> {
    let value: serde_json::Value = parse_json(body, "a session")?;
    match value.get("session_type") {
        None | Some(serde_json::Value::Null) => Ok(SessionType::default()),
        Some(kind) => serde_json::from_value(kind.clone())
            .map_err(|error| anyhow!("the daemon's answer named no session type: {error}")),
    }
}

/// The local session store, once the chat is within this terminal's reach,
/// with the privacy switch as this command sampled it (once, for the reach
/// rule and the save check alike).
struct LocalStore {
    manager: Arc<SessionManager>,
    enforced: bool,
}

async fn local_store(session_id: &str) -> Result<LocalStore> {
    let manager = Arc::new(SessionManager::instance());
    let enforced = biorouter::privacy::privacy_tiers_enabled();
    let capability = crate::session::privacy::caller_capability().await;
    check_local_reach(
        &manager,
        session_id,
        capability,
        enforced,
        LOCAL_OUT_OF_REACH,
    )
    .await?;
    Ok(LocalStore { manager, enforced })
}

/// The kind of chat `session_id` is in the local store.
async fn local_session_type(manager: &SessionManager, session_id: &str) -> Result<SessionType> {
    Ok(manager
        .get_session(session_id, false)
        .await
        .map_err(|_| anyhow!("Session '{session_id}' not found."))?
        .session_type)
}

/// Store `settings` in the local store with the check the daemon's route
/// makes before it stores them: what the save turns on must be able to run
/// for this chat now (its model there, able to take the question, and
/// allowed by the chat's tier, the private model it is bound to, its
/// institutions and any Crew grant). The check sends nothing of the chat.
/// `Err` is the sentence a person reads, starting "Nothing was saved", and
/// then nothing was stored.
///
/// The model the chat is bound to is read off its session row
/// (`biorouter::decision::save::load_for_save`). The route asks the chat's
/// agent first and reads the same row when it holds none; with no daemon
/// there is no agent to ask, and a chat bound to a private model before its
/// first turn is still classified Public.
///
/// Unlike the route it copies nothing onto running subagents: without a
/// daemon, none is running.
pub async fn save_locally(
    service: &DecisionService,
    manager: Arc<SessionManager>,
    session_id: &str,
    enforced: bool,
    settings: DecisionSettings,
) -> Result<DecisionSettings> {
    let session =
        biorouter::decision::save::load_for_save(Arc::clone(&manager), session_id, enforced)
            .await?;
    biorouter::decision::save_checked(service, &manager, &session, settings).await
}

/// What `add` prints once the monitor is stored.
fn added_message(session_id: &str, monitor: &MonitorCriterion) -> String {
    let mut message = format!(
        "Added monitor `{}` to session {session_id}: {} {}.",
        monitor.id,
        condition_text(monitor),
        action_text(monitor.action)
    );
    if monitor.apply_to_subagents {
        message.push_str(" It also watches the subagents this chat starts.");
    }
    message
}

/// What `remove` prints once the record without it is stored. A copy from
/// the chat that started this subagent comes back when that chat's monitors
/// are saved while the subagent runs (`settings::sync_inherited`), which the
/// person is told.
fn removed_message(session_id: &str, removed: &MonitorCriterion) -> String {
    let mut message = format!(
        "Removed monitor `{}` from session {session_id}.",
        clip(&removed.id, 80)
    );
    if removed.set_by == MonitorOwner::Agent {
        if let Some(owner) = owner_text(removed) {
            message.push_str(&format!(" It was {owner}."));
        }
    } else if let Some(chat) = removed.inherited_from.as_deref() {
        message.push_str(&format!(
            " It came from the chat that started this subagent ({}); saving that chat's \
             monitors again while this subagent runs copies it back.",
            clip(chat, 80)
        ));
    }
    message
}

/// What `add` and `remove` change.
#[derive(Debug, Clone)]
pub enum MonitorEdit {
    Add(AddMonitor),
    /// A monitor's id or name.
    Remove(String),
}

/// Apply `edit` to `settings`, the chat's whole record as stored, and say
/// what to print once the result is stored. Every other entry is left as it
/// was, who set it included. `session_type` is asked only when an added
/// monitor also watches subagents, which a subagent's own chat may not set.
async fn apply_edit<F, Fut>(
    session_id: &str,
    settings: &mut DecisionSettings,
    edit: MonitorEdit,
    session_type: F,
) -> Result<String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<SessionType>>,
{
    let message = match edit {
        MonitorEdit::Add(add) => {
            let monitor = build_monitor(&add, &settings.monitors)?;
            if monitor.apply_to_subagents {
                refuse_subagents_on(session_type().await?)?;
            }
            let message = added_message(session_id, &monitor);
            settings.monitors.push(monitor);
            message
        }
        MonitorEdit::Remove(monitor) => {
            // Any monitor, one an agent set or one copied from the chat that
            // started this subagent included: the person's word wins.
            let removed = remove_monitor(settings, &monitor)?;
            removed_message(session_id, &removed)
        }
    };
    biorouter::decision::settings::validate(settings).map_err(|e| anyhow!("{e}"))?;
    Ok(message)
}

/// `add` or `remove` through the daemon: read the record, change it, and
/// `PUT` it back, where the daemon checks it and copies a monitor set to
/// also watch subagents onto the running ones.
async fn edit_through_daemon(
    auth: DaemonAuth,
    session_id: &str,
    edit: MonitorEdit,
) -> Result<String> {
    let mut settings = daemon_load(session_id, &auth).await?;
    let message = apply_edit(session_id, &mut settings, edit, || {
        daemon_session_type(session_id, &auth)
    })
    .await?;
    daemon_save(session_id, &settings, auth).await?;
    Ok(message)
}

/// `add` or `remove` in the local store: read the record, change it, and
/// store it through the save check ([`save_locally`]). `Err` is the sentence
/// a person reads, and then nothing was stored.
async fn edit_locally(
    service: &DecisionService,
    store: &LocalStore,
    session_id: &str,
    edit: MonitorEdit,
) -> Result<String> {
    let mut settings = biorouter::decision::settings::load(&store.manager, session_id).await?;
    let message = apply_edit(session_id, &mut settings, edit, || {
        local_session_type(&store.manager, session_id)
    })
    .await?;
    save_locally(
        service,
        Arc::clone(&store.manager),
        session_id,
        store.enforced,
        settings,
    )
    .await?;
    Ok(message)
}

/// `biorouter session monitors <SESSION_ID> ...`.
pub async fn handle_session_monitors(
    session_id: &str,
    command: MonitorsCommand,
    user_action_key_stdin: bool,
) -> Result<()> {
    let source = reach(user_action_key_stdin).await?;
    let local = match &source {
        Reach::Daemon(_) => None,
        Reach::Local(why) => {
            eprintln!("note: read from the local session store ({why}).");
            Some(local_store(session_id).await?)
        }
    };
    let edit = match command {
        MonitorsCommand::List { json, format } => {
            let json = json || format == super::TextOrJson::Json;
            let settings = match (&source, &local) {
                (Reach::Daemon(auth), _) => daemon_load(session_id, auth).await?,
                (_, Some(store)) => {
                    biorouter::decision::settings::load(&store.manager, session_id).await?
                }
                _ => unreachable!("a local read always has its store"),
            };
            if json {
                super::print_line(serde_json::to_string_pretty(&settings)?);
            } else {
                for line in render_monitors(session_id, &settings) {
                    super::print_line(line);
                }
            }
            return Ok(());
        }
        MonitorsCommand::Add(mut add) => {
            // A bare model name is spelled out as `decide --model` takes it.
            add.model = super::decide::spell_model_option(add.model).await?;
            MonitorEdit::Add(add)
        }
        MonitorsCommand::Remove { monitor } => MonitorEdit::Remove(monitor),
    };
    let message = match (source, local) {
        (Reach::Daemon(auth), _) => edit_through_daemon(auth, session_id, edit).await,
        (_, Some(store)) => {
            edit_locally(biorouter::decision::global(), &store, session_id, edit).await
        }
        _ => unreachable!("a local write always has its store"),
    }
    .map_err(super::decide::advise)?;
    println!("{message}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Harness {
        #[command(subcommand)]
        command: MonitorsCommand,
    }

    fn parse(args: &[&str]) -> std::result::Result<MonitorsCommand, clap::Error> {
        let mut argv = vec!["monitors"];
        argv.extend_from_slice(args);
        Harness::try_parse_from(argv).map(|h| h.command)
    }

    fn add(args: &[&str]) -> AddMonitor {
        let mut argv = vec!["add"];
        argv.extend_from_slice(args);
        match parse(&argv).unwrap() {
            MonitorsCommand::Add(add) => add,
            other => panic!("not an add: {other:?}"),
        }
    }

    #[test]
    fn add_parses_with_the_contracts_defaults() {
        let a = add(&[
            "--name",
            "Form reached!",
            "--ask",
            "Has the agent reached the consent form page?",
            "--threshold",
            "0.85",
            "--action",
            "stop",
            "--timing",
            "turn-end",
            "--model",
            "ollama_decision/tev1:0.8b",
        ]);
        let monitor = build_monitor(&a, &[]).unwrap();
        assert_eq!(monitor.id, "form-reached");
        assert_eq!(monitor.name, "Form reached!");
        assert_eq!(monitor.question.kind, DecisionKind::Binary {});
        assert_eq!(monitor.threshold, 0.85);
        assert_eq!(monitor.action, MonitorAction::Stop);
        assert_eq!(monitor.timing, MonitorTiming::TurnEnd);
        assert_eq!(
            monitor.model,
            Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b"))
        );
        assert_eq!(monitor.consecutive, 1);
        assert_eq!(monitor.on_error, MonitorErrorPolicy::Notify);
        assert_eq!(monitor.max_evals_per_turn, 50);
        assert_eq!(monitor.window, MonitorWindow::default());
        assert!(monitor.enabled);

        let underscore = add(&[
            "--name",
            "x",
            "--ask",
            "q",
            "--threshold",
            "0.5",
            "--timing",
            "post_iteration",
        ]);
        assert_eq!(underscore.timing, TimingArg::PostIteration);

        assert!(
            parse(&["add", "--name", "x", "--ask", "q"]).is_err(),
            "--threshold is required"
        );
        assert!(parse(&["add", "--name", "x", "--ask", "q", "--threshold", "2"]).is_err());
        assert!(parse(&[
            "add",
            "--name",
            "x",
            "--ask",
            "q",
            "--threshold",
            "0.5",
            "--consecutive",
            "6"
        ])
        .is_err());
        assert!(parse(&[
            "add",
            "--name",
            "x",
            "--ask",
            "q",
            "--threshold",
            "0.5",
            "--action",
            "explode"
        ])
        .is_err());
        assert!(
            matches!(parse(&["remove", "form-reached"]).unwrap(), MonitorsCommand::Remove { monitor } if monitor == "form-reached")
        );
        assert!(matches!(
            parse(&["list", "--json"]).unwrap(),
            MonitorsCommand::List {
                json: true,
                format: super::super::TextOrJson::Text
            }
        ));
    }

    #[test]
    fn ids_are_unique_and_choices_name_their_option() {
        let first = build_monitor(
            &add(&["--name", "Form reached", "--ask", "q", "--threshold", "0.8"]),
            &[],
        )
        .unwrap();
        let second = build_monitor(
            &add(&[
                "--name",
                "Form  reached",
                "--ask",
                "q",
                "--threshold",
                "0.8",
            ]),
            std::slice::from_ref(&first),
        )
        .unwrap();
        assert_eq!(second.id, "form-reached-2");
        assert!(build_monitor(
            &add(&[
                "--name",
                "y",
                "--ask",
                "q",
                "--threshold",
                "0.8",
                "--id",
                "form-reached"
            ]),
            std::slice::from_ref(&first)
        )
        .unwrap_err()
        .to_string()
        .contains("already has"));
        assert!(build_monitor(
            &add(&[
                "--name",
                "y",
                "--ask",
                "q",
                "--threshold",
                "0.8",
                "--id",
                "Bad Id"
            ]),
            &[]
        )
        .is_err());

        let choice = add(&[
            "--name",
            "Course",
            "--ask",
            "Which course?",
            "--threshold",
            "0.7",
            "--choice",
            "rrms",
            "--choice",
            "spms",
            "--option",
            "rrms",
        ]);
        let monitor = build_monitor(&choice, &[]).unwrap();
        assert_eq!(condition_text(&monitor), "P(rrms) ≥ 0.70");
        let no_option = add(&[
            "--name",
            "Course",
            "--ask",
            "q",
            "--threshold",
            "0.7",
            "--choice",
            "a",
            "--choice",
            "b",
        ]);
        assert!(build_monitor(&no_option, &[])
            .unwrap_err()
            .to_string()
            .contains("--option"));
        let stray_option = add(&[
            "--name",
            "x",
            "--ask",
            "q",
            "--threshold",
            "0.7",
            "--option",
            "a",
        ]);
        assert!(build_monitor(&stray_option, &[]).is_err());
        let rearm = add(&[
            "--name",
            "x",
            "--ask",
            "q",
            "--threshold",
            "0.7",
            "--rearm-below",
            "0.8",
        ]);
        assert!(build_monitor(&rearm, &[])
            .unwrap_err()
            .to_string()
            .contains("re-arms"));

        let eight: Vec<MonitorCriterion> = (0..MAX_MONITORS)
            .map(|i| {
                let mut m = first.clone();
                m.id = format!("m{i}");
                m
            })
            .collect();
        assert!(build_monitor(
            &add(&["--name", "x", "--ask", "q", "--threshold", "0.5"]),
            &eight
        )
        .unwrap_err()
        .to_string()
        .contains("at most 8"));
        assert_eq!(slug("  !!  "), "monitor");
        assert_eq!(slug("EDSS: stated?"), "edss-stated");
    }

    /// A score monitor (set over HTTP) names the lowest level that counts by its
    /// label, whether its `option` holds the label or the level's index; it used
    /// to print the raw option, so `"1"` read as `P(1)`.
    #[test]
    fn condition_text_names_a_score_level_by_its_label() {
        let mut monitor = build_monitor(
            &add(&["--name", "Severity", "--ask", "q", "--threshold", "0.85"]),
            &[],
        )
        .unwrap();
        assert_eq!(condition_text(&monitor), "P(yes) ≥ 0.85");
        monitor.option = Some("no".to_string());
        assert_eq!(condition_text(&monitor), "P(no) ≥ 0.85");

        monitor.question.kind = DecisionKind::Score {
            levels: vec!["none".into(), "mild".into(), "severe".into()],
        };
        monitor.option = Some("mild".to_string());
        assert_eq!(condition_text(&monitor), "P(at least mild) ≥ 0.85");
        monitor.option = Some("2".to_string());
        assert_eq!(condition_text(&monitor), "P(at least severe) ≥ 0.85");
        monitor.option = Some("0".to_string());
        assert_eq!(condition_text(&monitor), "P(at least none) ≥ 0.85");
    }

    #[test]
    fn remove_takes_an_id_or_a_unique_name() {
        let monitor = build_monitor(
            &add(&["--name", "Form reached", "--ask", "q", "--threshold", "0.8"]),
            &[],
        )
        .unwrap();
        let mut settings = DecisionSettings {
            default_model: None,
            monitors: vec![monitor.clone()],
        };
        assert!(remove_monitor(&mut settings, "nope")
            .unwrap_err()
            .to_string()
            .contains("this chat has: form-reached"));
        assert_eq!(
            remove_monitor(&mut settings, "FORM REACHED").unwrap().id,
            "form-reached"
        );
        assert!(settings.monitors.is_empty());
        assert!(remove_monitor(&mut settings, "x")
            .unwrap_err()
            .to_string()
            .contains("no monitors"));
    }

    #[test]
    fn the_list_reads_as_a_sentence_per_monitor() {
        let mut monitor = build_monitor(
            &add(&[
                "--name",
                "Form reached",
                "--ask",
                "Has the agent reached the consent form page?",
                "--threshold",
                "0.85",
                "--action",
                "stop",
                "--consecutive",
                "2",
                "--rearm-below",
                "0.7",
                "--model",
                "ollama_decision/tev1:0.8b",
            ]),
            &[],
        )
        .unwrap();
        monitor.enabled = false;
        let settings = DecisionSettings {
            default_model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            monitors: vec![monitor],
        };
        let lines = render_monitors("s1", &settings);
        assert_eq!(lines[0], "Monitors of session s1 (1):");
        assert_eq!(lines[1], "  1. form-reached  Form reached  off");
        assert_eq!(
            lines[2],
            "     Has the agent reached the consent form page?"
        );
        // R3: a model a person reads is its name and where it runs.
        let local = format!(
            "tev1:0.8b · {}",
            super::super::decide::model_place("ollama_decision", "tev1:0.8b").unwrap()
        );
        assert_eq!(
            lines[3],
            format!(
                "     P(yes) ≥ 0.85 · 2 in a row · stops the turn · after each iteration · \
                 re-arms below 0.70 · an error shows a notice · at most 50 checks a turn · \
                 {local}"
            )
        );
        assert_eq!(lines[4], format!("Default decision model: {local}"));

        // A Baseten model configured by its slug is listed by its alias, as
        // every other terminal surface names it; the record keeps the slug.
        let mut hosted = settings.clone();
        let slug = DecisionModelRef::new(
            "baseten_decision",
            "decision-models/baseten-mammoth-27b-latest",
        );
        hosted.monitors[0].model = Some(slug.clone());
        hosted.default_model = Some(slug);
        let lines = render_monitors("s1", &hosted);
        assert!(lines[3].ends_with(" · mammoth-1 · Baseten"), "{}", lines[3]);
        assert_eq!(lines[4], "Default decision model: mammoth-1 · Baseten");

        let empty = render_monitors("s2", &DecisionSettings::default());
        assert_eq!(empty[0], "Session s2 has no monitors.");
        assert!(empty[1].starts_with("Add one: biorouter session monitors s2 add"));
        assert!(empty[2].contains("chosen automatically"));
    }

    /// What `add` builds is what the store keeps, and the local path's reach
    /// rule refuses a private chat to a public terminal.
    #[tokio::test]
    async fn a_built_monitor_round_trips_through_the_local_store() {
        use biorouter::privacy::ProviderTier;
        let dir = tempfile::tempdir().unwrap();
        let manager = SessionManager::new(dir.path().to_path_buf());
        let chat = manager
            .create_session(
                dir.path().to_path_buf(),
                "notes".to_string(),
                biorouter::session::SessionType::User,
            )
            .await
            .unwrap();
        let monitor = build_monitor(
            &add(&[
                "--name",
                "Form reached",
                "--ask",
                "Has the agent reached the consent form?",
                "--threshold",
                "0.85",
                "--action",
                "stop",
            ]),
            &[],
        )
        .unwrap();
        let settings = DecisionSettings {
            default_model: None,
            monitors: vec![monitor],
        };
        biorouter::decision::settings::validate(&settings).unwrap();
        biorouter::decision::settings::save(&manager, &chat.id, settings.clone())
            .await
            .unwrap();
        assert_eq!(
            biorouter::decision::settings::load(&manager, &chat.id)
                .await
                .unwrap(),
            settings
        );

        let private =
            crate::commands::session_decisions::tests::private_chat(&manager, dir.path()).await;
        let refused = check_local_reach(
            &manager,
            &private.id,
            ProviderTier::Public,
            true,
            LOCAL_OUT_OF_REACH,
        )
        .await
        .unwrap_err();
        assert_eq!(refused.to_string(), LOCAL_OUT_OF_REACH);
    }

    #[test]
    fn settings_parse_bare_or_wrapped() {
        let bare = r#"{"defaultModel":null,"monitors":[]}"#;
        assert_eq!(parse_settings(bare).unwrap(), DecisionSettings::default());
        let wrapped = r#"{"settings":{"defaultModel":{"provider":"ollama_decision","model":"tev1:0.8b"},"monitors":[]}}"#;
        assert_eq!(
            parse_settings(wrapped).unwrap().default_model,
            Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b"))
        );
        assert!(parse_settings("not json").is_err());
    }

    /// `--below`, `--rearm-above` and `--option yes|no` on a yes/no question
    /// reach the stored monitor, and what `add` builds passes the store's own
    /// validation.
    #[test]
    fn add_parses_below_rearm_above_and_a_yes_no_option() {
        let below = build_monitor(
            &add(&[
                "--name",
                "Banana",
                "--ask",
                "Has the assistant written the word banana?",
                "--threshold",
                "0.3",
                "--below",
                "--rearm-above",
                "0.5",
                "--action",
                "stop",
                "--on-error",
                "pause",
            ]),
            &[],
        )
        .unwrap();
        assert_eq!(below.direction, MonitorDirection::Below);
        assert_eq!(below.threshold, 0.3);
        assert_eq!(below.rearm_above, Some(0.5));
        assert_eq!(below.rearm_below, None);
        assert_eq!(below.on_error, MonitorErrorPolicy::Pause);
        assert_eq!(below.option, None);
        let settings = DecisionSettings {
            default_model: None,
            monitors: vec![below.clone()],
        };
        biorouter::decision::settings::validate(&settings).unwrap();
        let wire = serde_json::to_value(&below).unwrap();
        assert_eq!(wire["direction"], "below");
        assert_eq!(wire["rearmAbove"], 0.5);

        // Without --below nothing new is written.
        let plain = build_monitor(
            &add(&["--name", "x", "--ask", "q", "--threshold", "0.8"]),
            &[],
        )
        .unwrap();
        assert_eq!(plain.direction, MonitorDirection::AtOrAbove);
        let wire = serde_json::to_value(&plain).unwrap();
        assert!(wire.get("direction").is_none() && wire.get("rearmAbove").is_none());

        // `--option no` watches P(no); `yes` (any case) is the default and is
        // stored as none, as the desktop stores it.
        let no = build_monitor(
            &add(&[
                "--name",
                "x",
                "--ask",
                "q",
                "--threshold",
                "0.8",
                "--option",
                "NO",
            ]),
            &[],
        )
        .unwrap();
        assert_eq!(no.option.as_deref(), Some("no"));
        assert_eq!(condition_text(&no), "P(no) ≥ 0.80");
        let yes = build_monitor(
            &add(&[
                "--name",
                "x",
                "--ask",
                "q",
                "--threshold",
                "0.8",
                "--option",
                "Yes",
            ]),
            &[],
        )
        .unwrap();
        assert_eq!(yes.option, None);
        let other = build_monitor(
            &add(&[
                "--name",
                "x",
                "--ask",
                "q",
                "--threshold",
                "0.8",
                "--option",
                "maybe",
            ]),
            &[],
        )
        .unwrap_err();
        assert_eq!(
            other.to_string(),
            "--option on a yes/no question is `yes` or `no`"
        );
    }

    /// Each re-arm flag belongs to one direction, and `--rearm-above` must
    /// sit over the threshold.
    #[test]
    fn a_below_monitor_refuses_rearm_below() {
        let refused = |args: &[&str]| {
            let mut argv = vec!["--name", "x", "--ask", "q", "--threshold", "0.3"];
            argv.extend_from_slice(args);
            build_monitor(&add(&argv), &[]).unwrap_err().to_string()
        };
        assert_eq!(
            refused(&["--below", "--rearm-below", "0.2"]),
            "--rearm-below is for a monitor that fires at or above its threshold; a --below \
             monitor re-arms with --rearm-above"
        );
        assert!(refused(&["--rearm-above", "0.5"]).starts_with("--rearm-above is for a --below"));
        assert!(refused(&["--below", "--rearm-above", "0.3"])
            .starts_with("--rearm-above must be over --threshold"));
        assert!(refused(&["--below", "--rearm-above", "0.2"])
            .starts_with("--rearm-above must be over --threshold"));
        // The at-or-above rule is unchanged.
        assert!(refused(&["--rearm-below", "0.4"]).contains("re-arms"));
    }

    #[test]
    fn condition_text_and_render_lines() {
        let below = build_monitor(
            &add(&[
                "--name",
                "Banana",
                "--ask",
                "Has the assistant written the word banana?",
                "--threshold",
                "0.3",
                "--below",
                "--rearm-above",
                "0.5",
                "--action",
                "stop",
                "--on-error",
                "pause",
                "--model",
                "ollama_decision/tev1:0.8b",
            ]),
            &[],
        )
        .unwrap();
        assert_eq!(condition_text(&below), "P(yes) < 0.30");
        let mut held = below.clone();
        held.id = "held".into();
        held.rearm_above = None;
        let mut notice = build_monitor(
            &add(&["--name", "Note", "--ask", "q", "--threshold", "0.6"]),
            &[],
        )
        .unwrap();
        notice.option = Some("no".into());
        let settings = DecisionSettings {
            default_model: None,
            monitors: vec![below, held, notice],
        };
        let lines = render_monitors("s1", &settings);
        let local = format!(
            "tev1:0.8b · {}",
            super::super::decide::model_place("ollama_decision", "tev1:0.8b").unwrap()
        );
        assert_eq!(
            lines[3],
            format!(
                "     P(yes) < 0.30 · once · stops the turn · after each iteration · re-arms \
                 above 0.50 · an error pauses the turn · at most 50 checks a turn · {local}"
            )
        );
        assert_eq!(
            lines[6],
            format!(
                "     P(yes) < 0.30 · once · stops the turn · after each iteration · stops \
                 again while it holds · an error pauses the turn · at most 50 checks a turn · \
                 {local}"
            )
        );
        // A notify monitor with no re-arm level re-arms at its threshold,
        // which goes without saying.
        assert_eq!(
            lines[9],
            "     P(no) ≥ 0.60 · once · shows a notice · after each iteration · an error shows \
             a notice · at most 50 checks a turn"
        );
        assert_eq!(
            added_message("s1", &settings.monitors[0]),
            "Added monitor `banana` to session s1: P(yes) < 0.30 stops the turn."
        );
    }

    /// `--subagents` sets the monitor to also watch the subagents this chat
    /// starts, and is refused on a subagent's own chat, which starts none.
    #[test]
    fn session_monitors_add_subagents_sets_apply_to_subagents() {
        let shared = build_monitor(
            &add(&[
                "--name",
                "rm",
                "--ask",
                "Has the agent run a shell command that deletes a file?",
                "--threshold",
                "0.6",
                "--action",
                "stop",
                "--subagents",
            ]),
            &[],
        )
        .unwrap();
        assert!(shared.apply_to_subagents);
        assert_eq!(shared.set_by, MonitorOwner::Person);
        let wire = serde_json::to_value(&shared).unwrap();
        assert_eq!(wire["applyToSubagents"], true);
        biorouter::decision::settings::validate(&DecisionSettings {
            default_model: None,
            monitors: vec![shared.clone()],
        })
        .unwrap();
        assert!(added_message("p1", &shared)
            .ends_with("It also watches the subagents this chat starts."));
        let lines = render_monitors(
            "p1",
            &DecisionSettings {
                default_model: None,
                monitors: vec![shared.clone()],
            },
        );
        assert!(
            lines[3].ends_with(" · also watches the subagents this chat starts"),
            "{}",
            lines[3]
        );
        let plain = build_monitor(
            &add(&["--name", "x", "--ask", "q", "--threshold", "0.6"]),
            &[],
        )
        .unwrap();
        assert!(!plain.apply_to_subagents);

        assert_eq!(
            refuse_subagents_on(SessionType::SubAgent)
                .unwrap_err()
                .to_string(),
            SUBAGENTS_ON_A_SUBAGENT
        );
        refuse_subagents_on(SessionType::User).unwrap();
        refuse_subagents_on(SessionType::Scheduled).unwrap();

        // The daemon's `GET /sessions/{id}` names the kind.
        assert_eq!(
            session_type_of(r#"{"id":"c1","session_type":"sub_agent"}"#).unwrap(),
            SessionType::SubAgent
        );
        assert_eq!(
            session_type_of(r#"{"id":"c1"}"#).unwrap(),
            SessionType::User
        );
        assert!(session_type_of(r#"{"session_type":"robot"}"#).is_err());
    }

    /// A subagent's chat lists who set each monitor that the person did not
    /// set there, an edit leaves the other entries exactly as they were, and
    /// the person may remove a monitor an agent set.
    #[test]
    fn the_list_names_the_owner_and_an_edit_keeps_it() {
        let mut agent = build_monitor(
            &add(&[
                "--name",
                "Off task",
                "--ask",
                "q",
                "--threshold",
                "0.8",
                "--action",
                "steer",
            ]),
            &[],
        )
        .unwrap();
        agent.set_by = MonitorOwner::Agent;
        agent.set_by_session = Some("20261106_1".into());
        let mut copied = build_monitor(
            &add(&["--name", "rm", "--ask", "q", "--threshold", "0.6"]),
            std::slice::from_ref(&agent),
        )
        .unwrap();
        copied.inherited_from = Some("20261106_1".into());
        let mut settings = DecisionSettings {
            default_model: None,
            monitors: vec![agent.clone(), copied.clone()],
        };
        biorouter::decision::settings::validate(&settings).unwrap();
        let lines = render_monitors("20261106_2", &settings);
        assert_eq!(
            lines[1],
            "  1. off-task  Off task  on · set by the agent of chat 20261106_1"
        );
        assert_eq!(
            lines[4],
            "  2. rm  rm  on · from the chat that started this subagent (20261106_1)"
        );

        let before = serde_json::to_value(&settings.monitors).unwrap();
        let mine = build_monitor(
            &add(&["--name", "Mine", "--ask", "q", "--threshold", "0.9"]),
            &settings.monitors,
        )
        .unwrap();
        settings.monitors.push(mine);
        let after = serde_json::to_value(&settings.monitors[..2]).unwrap();
        assert_eq!(before, after, "the other entries are untouched");
        biorouter::decision::settings::validate(&settings).unwrap();

        let removed = remove_monitor(&mut settings, "off-task").unwrap();
        assert_eq!(removed, agent);
        assert_eq!(
            removed_message("20261106_2", &removed),
            "Removed monitor `off-task` from session 20261106_2. It was set by the agent of \
             chat 20261106_1."
        );
        biorouter::decision::settings::validate(&settings).unwrap();
        assert_eq!(settings.monitors[0], copied);

        // A copy may be removed too, and the person is told it comes back.
        let removed = remove_monitor(&mut settings, "rm").unwrap();
        assert_eq!(
            removed_message("20261106_2", &removed),
            "Removed monitor `rm` from session 20261106_2. It came from the chat that started \
             this subagent (20261106_1); saving that chat's monitors again while this subagent \
             runs copies it back."
        );
        assert_eq!(
            removed_message("20261106_2", &mine_only()),
            "Removed monitor `mine` from session 20261106_2."
        );
    }

    fn mine_only() -> MonitorCriterion {
        build_monitor(
            &add(&["--name", "Mine", "--ask", "q", "--threshold", "0.9"]),
            &[],
        )
        .unwrap()
    }

    /// The help says what a stop monitor does without a re-arm level, and
    /// what `pause` does.
    #[test]
    fn the_help_says_what_rearm_and_on_error_do() {
        use clap::CommandFactory;
        let command = Harness::command();
        let add = command.find_subcommand("add").unwrap();
        let help = |name: &str| {
            add.get_arguments()
                .find(|arg| arg.get_id() == name)
                .and_then(|arg| arg.get_help())
                .map(|help| help.to_string())
                .unwrap_or_default()
        };
        assert!(
            help("rearm_below")
                .contains("without it a stop monitor stops every turn in which its answer holds"),
            "{}",
            help("rearm_below")
        );
        assert!(help("on_error").contains("pause stops the turn whenever a check fails"));
        assert!(help("below").starts_with(
            "Fire when the probability falls below --threshold instead of reaching it"
        ));
        assert!(help("subagents").starts_with("Also watch the subagents this chat starts"));
    }

    /// The daemon's 422 (and 400) is the whole answer: its sentence, with no
    /// advice about a user-action key, which would not change it. A 403 keeps
    /// the advice.
    #[test]
    fn a_422_from_the_daemon_is_its_sentence_without_key_advice() {
        let sent = DecisionSettings::default();
        let sentence = "Nothing was saved: monitor \"X\" could not run. This chat is private, so \
                        this decision cannot run on `joey-1`.";
        let path = settings_path("s1");
        for code in [422, 400] {
            let error = save_answer("s1", &path, code, sentence, &sent).unwrap_err();
            assert_eq!(error.to_string(), sentence, "{code}");
            assert!(!error.to_string().contains("user-action key"));
        }
        let json = format!(r#"{{"message":{}}}"#, serde_json::json!(sentence));
        assert_eq!(
            save_answer("s1", &path, 422, &json, &sent)
                .unwrap_err()
                .to_string(),
            sentence
        );
        assert!(save_answer("s1", &path, 422, "", &sent)
            .unwrap_err()
            .to_string()
            .contains("Nothing was saved"));

        let refused = save_answer("s1", &path, 403, "proof needed", &sent).unwrap_err();
        assert!(refused.to_string().contains("user-action key"));
        assert_eq!(save_answer("s1", &path, 204, "", &sent).unwrap(), sent);
    }

    /// A decision service over no models at all: every named model is
    /// missing, and nothing is ever sent.
    struct NoModels;

    #[async_trait::async_trait]
    impl biorouter::decision::DecisionModelSource for NoModels {
        async fn list(&self) -> Vec<biorouter::decision::DecisionModelEntry> {
            Vec::new()
        }
        async fn open(
            &self,
            wanted: &DecisionModelRef,
        ) -> biorouter::decision::DecisionResult<Arc<dyn biorouter::decision::DecisionModel>>
        {
            Err(biorouter::decision::DecisionError::NoModel(format!(
                "No decision model `{}` is listed.",
                wanted.label()
            )))
        }
    }

    struct Allow;

    #[async_trait::async_trait]
    impl biorouter::decision::DecisionUseGate for Allow {
        async fn check(
            &self,
            _: &biorouter::decision::DecisionSession,
            _: &dyn biorouter::decision::DecisionModel,
        ) -> biorouter::decision::DecisionResult<()> {
            Ok(())
        }
    }

    fn no_models_service() -> DecisionService {
        DecisionService::new(Arc::new(NoModels))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None)
    }

    /// The local path stores through the save check the daemon's route makes:
    /// a monitor whose model is not there is refused with the sentence and
    /// nothing is stored, while switching it off is never checked.
    #[tokio::test]
    async fn a_local_save_runs_the_save_check() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let chat = manager
            .create_session(
                dir.path().to_path_buf(),
                "notes".to_string(),
                SessionType::User,
            )
            .await
            .unwrap();
        let service = no_models_service();
        let gone = [
            "--name",
            "Gone",
            "--ask",
            "Is the task done?",
            "--threshold",
            "0.8",
            "--model",
            "test_decision/gone",
        ];

        // `add` on the local path goes through the check.
        let store = LocalStore {
            manager: manager.clone(),
            enforced: true,
        };
        let error = edit_locally(&service, &store, &chat.id, MonitorEdit::Add(add(&gone)))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("Nothing was saved: monitor \"Gone\" could not run."),
            "{error}"
        );
        assert_eq!(
            biorouter::decision::settings::load(&manager, &chat.id)
                .await
                .unwrap(),
            DecisionSettings::default(),
            "nothing was stored"
        );

        let mut monitor = build_monitor(&add(&gone), &[]).unwrap();
        let settings = DecisionSettings {
            default_model: None,
            monitors: vec![monitor.clone()],
        };

        let error = save_locally(&service, manager.clone(), &chat.id, true, settings)
            .await
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<biorouter::decision::UnusableDecisionSettings>()
                .is_some(),
            "{error}"
        );
        assert!(
            error
                .to_string()
                .starts_with("Nothing was saved: monitor \"Gone\" could not run."),
            "{error}"
        );
        assert_eq!(
            biorouter::decision::settings::load(&manager, &chat.id)
                .await
                .unwrap(),
            DecisionSettings::default(),
            "nothing was stored"
        );

        // Switched off, it asks nothing of the model and is stored.
        monitor.enabled = false;
        let off = DecisionSettings {
            default_model: None,
            monitors: vec![monitor.clone()],
        };
        save_locally(&service, manager.clone(), &chat.id, true, off.clone())
            .await
            .unwrap();
        assert_eq!(
            biorouter::decision::settings::load(&manager, &chat.id)
                .await
                .unwrap(),
            off
        );
        // Switching it back on is checked again, and leaves it off.
        monitor.enabled = true;
        let on = DecisionSettings {
            default_model: None,
            monitors: vec![monitor],
        };
        assert!(save_locally(&service, manager.clone(), &chat.id, true, on)
            .await
            .is_err());
        assert_eq!(
            biorouter::decision::settings::load(&manager, &chat.id)
                .await
                .unwrap(),
            off
        );
    }

    /// Baseten's hosted `joey-1`, as the catalog lists it: a public model
    /// that takes yes/no questions. Nothing is ever sent to it here.
    struct Joey;

    #[async_trait::async_trait]
    impl biorouter::decision::DecisionModel for Joey {
        fn model_ref(&self) -> DecisionModelRef {
            DecisionModelRef::new("baseten_decision", "joey-1")
                .with_adapter(biorouter::decision::types::DecisionAdapter::SystemOne)
        }
        fn display_name(&self) -> String {
            "joey-1".into()
        }
        fn tier(&self) -> biorouter::privacy::ProviderTier {
            biorouter::privacy::ProviderTier::Public
        }
        fn affiliation(&self) -> Option<biorouter::privacy::affiliation::ModelAffiliation> {
            None
        }
        fn caps(&self) -> biorouter::decision::DecisionCaps {
            biorouter::decision::DecisionCaps {
                kinds: vec![biorouter::decision::types::DecisionKindTag::Binary],
                max_questions: 64,
                context_tokens: 4_096,
                max_concurrency: 1,
                max_request_bytes: None,
                local: false,
            }
        }
        async fn decide(
            &self,
            _: &biorouter::decision::types::DecisionRequest,
        ) -> biorouter::decision::DecisionResult<biorouter::decision::types::ModelVerdicts>
        {
            panic!("a save check sends nothing");
        }
    }

    struct OnlyJoey;

    #[async_trait::async_trait]
    impl biorouter::decision::DecisionModelSource for OnlyJoey {
        async fn list(&self) -> Vec<biorouter::decision::DecisionModelEntry> {
            vec![biorouter::decision::DecisionModelEntry {
                provider: "baseten_decision".into(),
                model: "joey-1".into(),
                display_name: "joey-1".into(),
                context_tokens: 4_096,
                kinds: vec![biorouter::decision::types::DecisionKindTag::Binary],
                tier: biorouter::privacy::ProviderTier::Public,
                affiliation: None,
                adapter: biorouter::decision::types::DecisionAdapter::SystemOne,
                local: false,
                load_refusal: None,
            }]
        }
        async fn open(
            &self,
            wanted: &DecisionModelRef,
        ) -> biorouter::decision::DecisionResult<Arc<dyn biorouter::decision::DecisionModel>>
        {
            if wanted.model == "joey-1" {
                Ok(Arc::new(Joey))
            } else {
                Err(biorouter::decision::DecisionError::NoModel(format!(
                    "No decision model `{}` is listed.",
                    wanted.label()
                )))
            }
        }
    }

    /// Item 6, as a live run found it: with no daemon, a chat bound to a
    /// private model that has not had its first turn (still classified
    /// Public, its row naming `versa_azure`) is refused a public decision
    /// model with the sentence the daemon's route answers 422 with, and
    /// nothing is stored. The same monitor on a chat bound to nothing is
    /// stored, so what refuses it is the bound model.
    #[tokio::test]
    async fn a_local_save_refuses_a_public_monitor_on_a_chat_bound_to_a_private_model() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let create = |name: &'static str| {
            let manager = manager.clone();
            let dir = dir.path().to_path_buf();
            async move {
                manager
                    .create_session(dir, name.to_string(), SessionType::User)
                    .await
                    .unwrap()
            }
        };
        let bound = create("bound").await;
        manager
            .update(&bound.id)
            .provider_name("versa_azure")
            .model_config(biorouter::model::ModelConfig::new_or_fail("gpt-5.5"))
            .apply()
            .await
            .unwrap();
        let unbound = create("unbound").await;
        let service = DecisionService::new(Arc::new(OnlyJoey))
            .with_use_gate(Arc::new(Allow))
            .with_default_model(None);
        let store = LocalStore {
            manager: manager.clone(),
            enforced: true,
        };
        let joey = [
            "--name",
            "Done",
            "--ask",
            "Is the task done?",
            "--threshold",
            "0.8",
            "--model",
            "baseten_decision/joey-1",
        ];

        let error = edit_locally(&service, &store, &bound.id, MonitorEdit::Add(add(&joey)))
            .await
            .unwrap_err();
        assert!(
            error.to_string().starts_with(
                "Nothing was saved: monitor \"Done\" could not run. This chat runs on a private \
                 model, so it becomes private at its first turn, and `joey-1` is a public model \
                 a private chat cannot use."
            ),
            "{error}"
        );
        assert!(error
            .downcast_ref::<biorouter::decision::UnusableDecisionSettings>()
            .is_some());
        assert_eq!(
            biorouter::decision::settings::load(&manager, &bound.id)
                .await
                .unwrap(),
            DecisionSettings::default(),
            "nothing was stored"
        );

        edit_locally(&service, &store, &unbound.id, MonitorEdit::Add(add(&joey)))
            .await
            .unwrap();
        assert_eq!(
            biorouter::decision::settings::load(&manager, &unbound.id)
                .await
                .unwrap()
                .monitors
                .len(),
            1
        );
    }

    /// On the local path `add --subagents` is refused on a subagent's own
    /// chat and stored on the person's, and `remove` takes a monitor an
    /// agent set while the others keep who set them.
    #[tokio::test]
    async fn the_local_path_shares_with_subagents_only_from_a_chat_that_starts_them() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let create = |kind: SessionType| {
            let manager = manager.clone();
            let dir = dir.path().to_path_buf();
            async move {
                manager
                    .create_session(dir, "chat".to_string(), kind)
                    .await
                    .unwrap()
            }
        };
        let parent = create(SessionType::User).await;
        let child = create(SessionType::SubAgent).await;
        let service = no_models_service();
        let store = LocalStore {
            manager: manager.clone(),
            enforced: true,
        };
        // Off, so the save check asks no model: what is measured here is
        // the subagent rule alone.
        let shared = [
            "--name",
            "rm",
            "--ask",
            "Has the agent deleted a file?",
            "--threshold",
            "0.6",
            "--action",
            "stop",
            "--subagents",
            "--disabled",
        ];

        let refused = edit_locally(&service, &store, &child.id, MonitorEdit::Add(add(&shared)))
            .await
            .unwrap_err();
        assert_eq!(refused.to_string(), SUBAGENTS_ON_A_SUBAGENT);
        assert_eq!(
            biorouter::decision::settings::load(&manager, &child.id)
                .await
                .unwrap(),
            DecisionSettings::default()
        );

        let said = edit_locally(&service, &store, &parent.id, MonitorEdit::Add(add(&shared)))
            .await
            .unwrap();
        assert!(said.ends_with("It also watches the subagents this chat starts."));
        let stored = biorouter::decision::settings::load(&manager, &parent.id)
            .await
            .unwrap();
        assert!(stored.monitors[0].apply_to_subagents);

        // An agent's monitor beside it: the person removes it, and the
        // person's monitor is stored back exactly as it was.
        let mut agent = build_monitor(
            &add(&[
                "--name",
                "Off task",
                "--ask",
                "q",
                "--threshold",
                "0.8",
                "--action",
                "steer",
                "--disabled",
            ]),
            &stored.monitors,
        )
        .unwrap();
        agent.set_by = MonitorOwner::Agent;
        agent.set_by_session = Some(parent.id.clone());
        let mut with_agent = stored.clone();
        with_agent.monitors.push(agent);
        biorouter::decision::settings::save(&manager, &parent.id, with_agent)
            .await
            .unwrap();
        let said = edit_locally(
            &service,
            &store,
            &parent.id,
            MonitorEdit::Remove("off-task".into()),
        )
        .await
        .unwrap();
        assert!(said.contains("It was set by the agent of chat"), "{said}");
        assert_eq!(
            biorouter::decision::settings::load(&manager, &parent.id)
                .await
                .unwrap(),
            stored
        );
    }
}
