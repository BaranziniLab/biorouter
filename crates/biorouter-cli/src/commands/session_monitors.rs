//! `biorouter session monitors <SESSION_ID> list|add|remove`: the decision-model
//! monitors that watch one chat.
//!
//! A monitor asks a decision model one question about a rolling window of the
//! chat after each agent iteration or at the end of a turn, and when the
//! probability of the wanted answer reaches its threshold it stops the turn,
//! shows a notice, or tells the model. Monitors are stored in the chat's
//! `decision_settings.v1`, the same record the desktop's Monitors dialog edits.
//!
//! * **With a reachable daemon** the record is read with
//!   `GET /sessions/{id}/decision-settings` and written with `PUT` on the same
//!   path. A write names where the chat's content goes, so a daemon that holds
//!   a user-action key wants it; see
//!   `session_watch::protected_write_with_key_if_wanted`.
//! * **With none** the same record is read and written in this process, through
//!   `biorouter::decision::settings`, under the tier rule the route applies.

use anyhow::{anyhow, bail, Result};
use biorouter::decision::settings::{
    DecisionSettings, MonitorAction, MonitorCriterion, MonitorErrorPolicy, MonitorTiming,
    MonitorWindow, MAX_MONITORS,
};
use biorouter::decision::{DecisionKind, DecisionModelRef, Question, Threshold};
use biorouter::session::SessionManager;

use super::apps::configured_port;
use super::decide::{clip, format_p, model_label, parse_choice, parse_probability};
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
    Notify,
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
    /// Fire when the probability of the wanted answer is at least this (0 to 1).
    #[arg(long, value_parser = parse_probability)]
    pub threshold: f64,
    /// What to do when it fires: stop the turn, show a notice, or tell the model.
    #[arg(long, value_enum, default_value_t = ActionArg::Notify)]
    pub action: ActionArg,
    /// When to ask: after every iteration of the agent loop, or once as a turn ends.
    #[arg(long, value_enum, default_value_t = TimingArg::PostIteration)]
    pub timing: TimingArg,
    /// The decision model, `provider/model`. Without it the chat's default
    /// decision model is used.
    #[arg(long, value_name = "PROVIDER/MODEL")]
    pub model: Option<String>,
    /// For a choice question: one option, `name` or `name=description`. Repeat it.
    #[arg(long = "choice", value_name = "NAME[=DESCRIPTION]", value_parser = parse_choice)]
    pub choices: Vec<(String, Option<String>)>,
    /// For a choice question: the option whose probability is compared.
    #[arg(long, value_name = "OPTION")]
    pub option: Option<String>,
    /// Evaluations in a row at or above the threshold before it fires (1 to 5).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=5))]
    pub consecutive: u32,
    /// After firing, re-arm only once the probability drops below this.
    #[arg(long = "rearm-below", value_name = "P", value_parser = parse_probability)]
    pub rearm_below: Option<f64>,
    /// What to do when a check fails: show a notice, or pause the turn.
    #[arg(long = "on-error", value_enum, default_value_t = OnErrorArg::Notify)]
    pub on_error: OnErrorArg,
    /// At most this many checks per turn (1 to 200).
    #[arg(long = "max-checks", default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=200))]
    pub max_checks: u32,
    /// An id for the monitor; made from the name when not given.
    #[arg(long)]
    pub id: Option<String>,
    /// Add it switched off.
    #[arg(long)]
    pub disabled: bool,
}

/// What `session monitors` does.
#[derive(clap::Subcommand, Debug, Clone)]
pub enum MonitorsCommand {
    /// List the chat's monitors and its default decision model.
    List {
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
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
        if add.option.is_some() {
            bail!("--option names one of the --choice options; this question is yes or no");
        }
        DecisionKind::Binary {}
    } else {
        DecisionKind::Choice {
            options: add.choices.iter().cloned().collect(),
        }
    };
    if let DecisionKind::Choice { options } = &kind {
        match &add.option {
            None => bail!(
                "say which option the threshold is on with --option (one of: {})",
                options.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
            Some(option) if !options.contains_key(option) => {
                bail!("--option `{option}` is not one of the --choice options")
            }
            Some(_) => {}
        }
    }
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
    if let Some(rearm) = add.rearm_below {
        if rearm >= add.threshold {
            bail!("--rearm-below must be under --threshold, or the monitor never re-arms");
        }
    }
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
        option: add.option.clone(),
        consecutive: add.consecutive,
        rearm_below: add.rearm_below,
        action: add.action.into(),
        on_error: add.on_error.into(),
        max_evals_per_turn: add.max_checks,
        enabled: !add.disabled,
    })
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

fn timing_text(timing: MonitorTiming) -> &'static str {
    match timing {
        MonitorTiming::PostIteration => "after each iteration",
        MonitorTiming::TurnEnd => "as the turn ends",
    }
}

fn action_text(action: MonitorAction) -> &'static str {
    match action {
        MonitorAction::Stop => "stops the turn",
        MonitorAction::Notify => "shows a notice",
        MonitorAction::Steer => "tells the model",
    }
}

/// What a monitor compares, readable: `P(yes) ≥ 0.85`, `P(rrms) ≥ 0.70`, and
/// for a score `P(at least mild) ≥ 0.85`, the level named by its label whether
/// `option` gives the label or its index, as `decide` names the same threshold.
pub fn condition_text(monitor: &MonitorCriterion) -> String {
    let answer = match (monitor.pass_threshold(), &monitor.question.kind) {
        (Ok(Threshold::AtLeastLevel { level, .. }), DecisionKind::Score { levels }) => {
            let label = levels
                .get(level)
                .map(|label| label.trim())
                .filter(|label| !label.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("level {level}"));
            format!("at least {label}")
        }
        _ => monitor.option.as_deref().unwrap_or("yes").to_string(),
    };
    format!("P({answer}) ≥ {}", format_p(monitor.threshold))
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
            out.push(format!(
                "  {}. {}  {}  {state}",
                i + 1,
                monitor.id,
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
            if let Some(rearm) = monitor.rearm_below {
                details.push(format!("re-arms below {}", format_p(rearm)));
            }
            details.push(match monitor.on_error {
                MonitorErrorPolicy::Notify => "an error shows a notice".to_string(),
                MonitorErrorPolicy::Pause => "an error pauses the turn".to_string(),
            });
            details.push(format!(
                "at most {} checks a turn",
                monitor.max_evals_per_turn
            ));
            if let Some(model) = &monitor.model {
                details.push(model_label(model));
            }
            out.push(format!("     {}", details.join(" · ")));
        }
    }
    match &settings.default_model {
        Some(model) => out.push(format!("Default decision model: {}", model_label(model))),
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
    match code {
        200 | 201 => parse_settings(&answer).or_else(|_| Ok(settings.clone())),
        204 => Ok(settings.clone()),
        403 => Err(anyhow!(
            "the daemon refused the change: {}. Nothing was changed. A daemon started with a \
             user-action key wants it: pass --user-action-key-stdin and pipe the key as the \
             first line of stdin, or run this from a terminal.",
            refusal_sentence(&answer).unwrap_or_else(|| "no reason given".to_string())
        )),
        other => Err(daemon_error(
            session_id,
            &format!("PUT {path}"),
            other,
            &answer,
        )),
    }
}

async fn local_manager(session_id: &str) -> Result<SessionManager> {
    let manager = SessionManager::instance();
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
    Ok(manager)
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
            Some(local_manager(session_id).await?)
        }
    };
    let mut settings = match (&source, &local) {
        (Reach::Daemon(auth), _) => daemon_load(session_id, auth).await?,
        (_, Some(manager)) => biorouter::decision::settings::load(manager, session_id).await?,
        _ => unreachable!("a local read always has its store"),
    };

    let message = match command {
        MonitorsCommand::List { json } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&settings)?);
            } else {
                for line in render_monitors(session_id, &settings) {
                    println!("{line}");
                }
            }
            return Ok(());
        }
        MonitorsCommand::Add(add) => {
            let monitor = build_monitor(&add, &settings.monitors)?;
            let message = format!(
                "Added monitor `{}` to session {session_id}: {} {}.",
                monitor.id,
                condition_text(&monitor),
                action_text(monitor.action)
            );
            settings.monitors.push(monitor);
            message
        }
        MonitorsCommand::Remove { monitor } => {
            let removed = remove_monitor(&mut settings, &monitor)?;
            format!(
                "Removed monitor `{}` from session {session_id}.",
                removed.id
            )
        }
    };
    biorouter::decision::settings::validate(&settings).map_err(|e| anyhow!("{e}"))?;
    match (source, local) {
        (Reach::Daemon(auth), _) => {
            daemon_save(session_id, &settings, auth).await?;
        }
        (_, Some(manager)) => {
            biorouter::decision::settings::save(&manager, session_id, settings).await?;
        }
        _ => unreachable!("a local write always has its store"),
    }
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
            MonitorsCommand::List { json: true }
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
        assert_eq!(
            lines[3],
            "     P(yes) ≥ 0.85 · 2 in a row · stops the turn · after each iteration · re-arms \
             below 0.70 · an error shows a notice · at most 50 checks a turn · \
             ollama_decision/tev1:0.8b"
        );
        assert_eq!(
            lines[4],
            "Default decision model: ollama_decision/tev1:0.8b"
        );

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
        assert!(
            lines[3].ends_with(" · baseten_decision/mammoth-1"),
            "{}",
            lines[3]
        );
        assert_eq!(
            lines[4],
            "Default decision model: baseten_decision/mammoth-1"
        );

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
}
