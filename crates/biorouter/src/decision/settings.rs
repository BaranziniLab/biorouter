//! Per-chat decision settings: the chat's default decision model, its monitors,
//! and, for a chat whose main model is a decision model, the question it asks.
//!
//! Stored in the chat's `extension_data` under [`DECISION_SETTINGS_STATE`] and
//! [`DECISION_CHAT_STATE`], written only through `update_extension_state` (one key
//! per transaction), so a save here can never erase a to-do list, a goal or an
//! orchestration plan written at the same moment, which a read-modify-write of
//! the whole `extension_data` blob would. Every writer of the monitors
//! ([`save`], and [`update`] with the writers built on it) reads, changes,
//! validates and stores them in that one transaction, so two writers (a
//! person's save and an agent's edit, say) never lose each other's change.
//!
//! Both records name where a chat's content goes (a monitor's model reads a
//! window of the conversation), so a route that writes them needs the user's
//! proof; that check belongs to the route, not here. [`save`] and
//! [`save_chat`] validate before they write and refuse with the sentence a
//! person reads.
//!
//! Every record here refuses a key it does not have, as the question, kind,
//! threshold and model reference inside it already do. A misspelled key
//! (`on_error` for `onError`) would otherwise be dropped and its field left at
//! the default, and the defaults (`notify` on a firing, `notify` on an error)
//! are the ones that let a turn carry on. The keys added since the first
//! version (`direction`, `rearmAbove`, `setBy`, `setBySession`,
//! `inheritedFrom`, `applyToSubagents`) are written only when they differ from
//! their default, so a monitor that uses none of them is stored byte for byte
//! as it was, and an older build still reads it. An older build reports a
//! record that uses one of them as unreadable rather than guessing.
//!
//! Who set a monitor matters on a subagent's chat. The person's monitors
//! (including copies of a parent chat's monitors set to also watch its
//! subagents) are never changed by an agent; the agent of the chat that
//! started the subagent may add, change and remove its own, at most
//! [`MAX_AGENT_MONITORS`] of them ([`apply_agent_edit`]).

use std::collections::BTreeSet;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::types::{
    validate_questions, DecisionKind, DecisionModelRef, Question, Threshold, NO, YES,
};
use crate::conversation::message::{Message, SystemNotificationType};
use crate::session::SessionManager;

pub const DECISION_SETTINGS_STATE: &str = "decision_settings";
pub const DECISION_SETTINGS_VERSION: &str = "v1";
pub const DECISION_CHAT_STATE: &str = "decision_chat";
pub const DECISION_CHAT_VERSION: &str = "v1";
/// The monitors' runtime state that outlives a turn (a fired monitor's latch,
/// a run of consecutive checks). Internal: written only inside a turn by the
/// monitor engine, never by a route, and never shown by one.
pub const DECISION_MONITOR_STATE: &str = "decision_monitor_state";
pub const DECISION_MONITOR_STATE_VERSION: &str = "v1";

/// At most this many monitors per chat.
pub const MAX_MONITORS: usize = 8;
/// At most this many of a chat's monitors set by an agent (the agent of the
/// chat that started this subagent), within [`MAX_MONITORS`].
pub const MAX_AGENT_MONITORS: usize = 4;
/// A monitor's name, in characters.
pub const MAX_MONITOR_NAME_CHARS: usize = 60;
/// A monitor id, in characters. The id doubles as the question's name when
/// monitors share one multi-question request, so it is never longer than a
/// question name may be.
pub const MAX_MONITOR_ID_CHARS: usize = 64;
/// Window bounds: newest messages kept, and the character budget. The
/// ceilings are what [`super::window::build_window`] can deliver, so a stored
/// window is never accepted and shown at a size it is then cut from.
pub const MAX_WINDOW_ENTRIES: u32 = super::window::MAX_WINDOW_MESSAGES as u32;
pub const MIN_WINDOW_CHARS: u32 = 200;
pub const MAX_WINDOW_CHARS: u32 = super::window::MAX_WINDOW_CHARS as u32;
/// Evaluations in a row before a monitor fires.
pub const MAX_CONSECUTIVE: u32 = 5;
/// Evaluation cap per turn.
pub const MAX_EVALS_PER_TURN: u32 = 200;
/// Evaluation cap per turn when a monitor names none.
pub const DEFAULT_EVALS_PER_TURN: u32 = 50;
/// A [`MonitorStopReport`]'s copy of the question, in characters.
pub const MAX_STOP_REPORT_QUESTION_CHARS: usize = 300;
/// The name a decision chat's question is validated under.
const CHAT_QUESTION_NAME: &str = "question";

/// When a monitor is evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum MonitorTiming {
    /// After every agent-loop iteration (each model reply and its tool results).
    #[default]
    PostIteration,
    /// Once, when the turn is about to end.
    TurnEnd,
}

/// What a monitor does when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum MonitorAction {
    /// End the turn and hand control back to the person, with a notice.
    Stop,
    /// Show one notice and keep going.
    #[default]
    Notify,
    /// Tell the model (a stored steer-like note) and keep going.
    Steer,
}

/// What a monitor does when its decision call fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum MonitorErrorPolicy {
    /// Show one notice; after three errors in a row the monitor is off for the turn.
    #[default]
    Notify,
    /// Treat an error as firing with `Stop` (fail closed, for safety criteria).
    Pause,
}

/// Which side of its threshold a monitor fires on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum MonitorDirection {
    /// Fires when the probability of the answer it watches is at or above
    /// the threshold.
    #[default]
    AtOrAbove,
    /// Fires when the probability of the answer it watches falls below the
    /// threshold.
    Below,
}

/// Who set a monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum MonitorOwner {
    /// The person: in the desktop, the CLI, over HTTP or in a workflow, or a
    /// copy of a parent chat's monitor set to also watch its subagents. An
    /// agent never changes or removes one.
    #[default]
    Person,
    /// The agent of the chat that started this subagent (`setBySession`).
    Agent,
}

/// The rolling window a monitor sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MonitorWindow {
    /// Newest messages kept (a message's tool calls and results count with
    /// it), at most 24.
    pub entries: u32,
    /// Character budget, at most 6000.
    pub max_chars: u32,
    /// Include tool calls and results.
    pub tool_calls: bool,
}

impl Default for MonitorWindow {
    fn default() -> Self {
        Self {
            entries: 24,
            max_chars: 6000,
            tool_calls: true,
        }
    }
}

/// One monitor: a decision model watching the chat for a criterion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MonitorCriterion {
    /// Slug, unique within the chat.
    pub id: String,
    /// Short human name (at most 60 chars).
    pub name: String,
    pub question: Question,
    pub model: Option<DecisionModelRef>,
    #[serde(default)]
    pub timing: MonitorTiming,
    #[serde(default)]
    pub window: MonitorWindow,
    /// The probability the monitor compares P(wanted answer) with: above 0
    /// and at most 1. It fires at or above it, or below it when `direction`
    /// is `below`.
    pub threshold: f64,
    /// `at_or_above` (the default, and then not written) or `below`.
    #[serde(default, skip_serializing_if = "is_at_or_above")]
    pub direction: MonitorDirection,
    /// The answer whose probability is compared: `yes` (or none) or `no` for a
    /// yes/no question, an option's name for a choice, and the lowest level that
    /// counts for a score, by its label or its index (`"0"` is the lowest).
    pub option: Option<String>,
    /// Evaluations in a row on its firing side before it fires (1..5).
    #[serde(default = "one")]
    pub consecutive: u32,
    /// For a monitor that fires at or above its threshold: after firing,
    /// re-arm only once P drops below this (0..threshold). A `below` monitor
    /// sets `rearmAbove` instead.
    ///
    /// For a `stop` monitor it also decides what later turns see. Without it
    /// a stop monitor never latches: it stops every turn in which its answer
    /// holds, so the person who carries on is stopped again at the first
    /// check that still holds. With it the monitor stops once and stays
    /// quiet until P falls below it. A `notify` or `steer` monitor re-arms
    /// here, or at its threshold when it is not set.
    pub rearm_below: Option<f64>,
    /// For a `below` monitor, the counterpart of `rearmBelow`: after firing,
    /// re-arm only once P rises to this or above (threshold..1). A `stop`
    /// monitor without it stops every turn in which its answer holds; with it
    /// the monitor stops once and stays quiet until P reaches it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rearm_above: Option<f64>,
    #[serde(default)]
    pub action: MonitorAction,
    #[serde(default)]
    pub on_error: MonitorErrorPolicy,
    /// Evaluation cap per turn (at most 200).
    #[serde(default = "default_max_evals")]
    pub max_evals_per_turn: u32,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Who set it: `person` (the default, and then not written) or `agent`.
    #[serde(default, skip_serializing_if = "is_person")]
    pub set_by: MonitorOwner,
    /// For a monitor an agent set: the chat whose agent set it, which is the
    /// chat that started this subagent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_by_session: Option<String>,
    /// For a copy of a person's monitor: the chat it was copied from, whose
    /// monitor is set to also watch the subagents it starts. The copy follows
    /// that monitor while this subagent runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_from: Option<String>,
    /// A person's monitor that is copied onto every subagent this chat
    /// starts, and onto the ones running when it is saved.
    #[serde(default, skip_serializing_if = "is_false")]
    pub apply_to_subagents: bool,
}

fn one() -> u32 {
    1
}
fn default_max_evals() -> u32 {
    DEFAULT_EVALS_PER_TURN
}
fn yes() -> bool {
    true
}
fn is_at_or_above(direction: &MonitorDirection) -> bool {
    *direction == MonitorDirection::AtOrAbove
}
fn is_person(owner: &MonitorOwner) -> bool {
    *owner == MonitorOwner::Person
}
fn is_false(value: &bool) -> bool {
    !*value
}

impl MonitorCriterion {
    /// The monitor's firing rule as a [`Threshold`] the decision service
    /// applies, so a monitor fires exactly when its decision `passed`:
    ///
    /// - a yes/no question compares P(yes), or P(no) when `option` is `no`;
    /// - a choice question compares P(`option`);
    /// - a score question compares P(level at or above `option`), where
    ///   `option` is a level's label or its index (`"0"` is the lowest).
    ///
    /// `Err` is the sentence [`validate`] gives for a criterion whose `option`
    /// does not fit its question.
    pub fn pass_threshold(&self) -> Result<Threshold, String> {
        let p = self.threshold;
        let name = &self.name;
        match (&self.question.kind, self.option.as_deref()) {
            (DecisionKind::Binary {}, None) => Ok(Threshold::Yes(p)),
            (DecisionKind::Binary {}, Some(option)) if option.eq_ignore_ascii_case(YES) => {
                Ok(Threshold::Yes(p))
            }
            (DecisionKind::Binary {}, Some(option)) if option.eq_ignore_ascii_case(NO) => {
                Ok(Threshold::No(p))
            }
            (DecisionKind::Binary {}, Some(option)) => Err(format!(
                "Monitor \"{name}\" asks a yes/no question, so it can watch `yes` or `no`, not `{option}`."
            )),
            (DecisionKind::Choice { options }, Some(option)) if options.contains_key(option) => {
                Ok(Threshold::Option {
                    name: option.to_string(),
                    p,
                })
            }
            (DecisionKind::Choice { .. }, Some(option)) => Err(format!(
                "Monitor \"{name}\" watches `{option}`, which is not one of its question's options."
            )),
            (DecisionKind::Choice { .. }, None) => Err(format!(
                "Monitor \"{name}\" asks a choice question; name the option whose probability it watches."
            )),
            (DecisionKind::Score { levels }, Some(option)) => levels
                .iter()
                .position(|level| level.trim() == option.trim())
                .or_else(|| option.trim().parse::<usize>().ok().filter(|i| *i < levels.len()))
                .map(|level| Threshold::AtLeastLevel { level, p })
                .ok_or_else(|| {
                    format!(
                        "Monitor \"{name}\" watches level `{option}`, which is not one of its \
                         question's levels."
                    )
                }),
            (DecisionKind::Score { .. }, None) => Err(format!(
                "Monitor \"{name}\" asks a score question; name the lowest level that counts."
            )),
        }
    }

    /// The criterion's question carrying [`Self::pass_threshold`], ready to
    /// send. `Err` as for [`Self::pass_threshold`].
    pub fn question_with_threshold(&self) -> Result<Question, String> {
        Ok(Question {
            threshold: Some(self.pass_threshold()?),
            ..self.question.clone()
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionSettings {
    pub default_model: Option<DecisionModelRef>,
    #[serde(default)]
    pub monitors: Vec<MonitorCriterion>,
}

/// The question a decision chat asks of each message the person sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecisionChatSettings {
    pub question: Question,
}

/// A settings value [`save`] or [`save_chat`] refused. The message is the
/// sentence a person reads; a route answers it with 400 rather than 500, which
/// it can tell by `downcast_ref::<InvalidDecisionSettings>()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidDecisionSettings(pub String);

impl std::fmt::Display for InvalidDecisionSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidDecisionSettings {}

fn valid_monitor_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.chars().count() <= MAX_MONITOR_ID_CHARS
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

fn validate_model(model: &DecisionModelRef, whose: &str) -> Result<(), String> {
    if model.provider.trim().is_empty() || model.model.trim().is_empty() {
        return Err(format!(
            "{whose} names a decision model without a provider or a model name."
        ));
    }
    Ok(())
}

fn probability(p: f64) -> bool {
    p.is_finite() && (0.0..=1.0).contains(&p)
}

/// A chat id a monitor names as its setter or its source: present and not blank.
fn names_a_chat(id: Option<&str>) -> bool {
    id.is_some_and(|id| !id.trim().is_empty())
}

fn validate_monitor(monitor: &MonitorCriterion) -> Result<(), String> {
    let id = &monitor.id;
    if !valid_monitor_id(id) {
        return Err(format!(
            "Monitor id `{id}` must be 1 to {MAX_MONITOR_ID_CHARS} lowercase letters, digits, `-` \
             or `_`, starting with a letter or digit."
        ));
    }
    let name = monitor.name.trim();
    if name.is_empty() || monitor.name.chars().count() > MAX_MONITOR_NAME_CHARS {
        return Err(format!(
            "Monitor `{id}` needs a name of 1 to {MAX_MONITOR_NAME_CHARS} characters."
        ));
    }
    let mut question = IndexMap::new();
    question.insert(id.clone(), monitor.question.clone());
    validate_questions(&question).map_err(|why| format!("Monitor \"{name}\": {why}."))?;
    let threshold = monitor.threshold;
    // 0 is refused: at or above it fires on every check, and below it never.
    // 1 stays: a model can answer with exactly 1.00, and "only when certain"
    // is a real rule.
    if !(threshold.is_finite() && threshold > 0.0 && threshold <= 1.0) {
        return Err(format!(
            "Monitor \"{name}\": the threshold must be a probability above 0 and at most 1, not \
             {threshold}."
        ));
    }
    let rule = monitor.pass_threshold()?;
    if let Some(own) = &monitor.question.threshold {
        if *own != rule {
            return Err(format!(
                "Monitor \"{name}\": its question carries a threshold of its own that differs from \
                 the monitor's. Set the monitor's threshold and option, and leave the question's \
                 empty."
            ));
        }
    }
    if !(1..=MAX_CONSECUTIVE).contains(&monitor.consecutive) {
        return Err(format!(
            "Monitor \"{name}\": it can wait for 1 to {MAX_CONSECUTIVE} evaluations in a row, not {}.",
            monitor.consecutive
        ));
    }
    match monitor.direction {
        MonitorDirection::AtOrAbove => {
            if monitor.rearm_above.is_some() {
                return Err(format!(
                    "Monitor \"{name}\": it fires at or above its threshold, so it re-arms below a \
                     probability; set `rearmBelow`, not `rearmAbove`."
                ));
            }
            if let Some(rearm) = monitor.rearm_below {
                if !probability(rearm) || rearm > threshold {
                    return Err(format!(
                        "Monitor \"{name}\": it re-arms below a probability between 0 and its \
                         threshold ({threshold}), not {rearm}."
                    ));
                }
            }
        }
        MonitorDirection::Below => {
            if monitor.rearm_below.is_some() {
                return Err(format!(
                    "Monitor \"{name}\": it fires below its threshold, so it re-arms above a \
                     probability; set `rearmAbove`, not `rearmBelow`."
                ));
            }
            if let Some(rearm) = monitor.rearm_above {
                if !probability(rearm) || rearm < threshold {
                    return Err(format!(
                        "Monitor \"{name}\": it re-arms at or above a probability between its \
                         threshold ({threshold}) and 1, not {rearm}."
                    ));
                }
            }
        }
    }
    let window = monitor.window;
    if !(1..=MAX_WINDOW_ENTRIES).contains(&window.entries) {
        return Err(format!(
            "Monitor \"{name}\": its window keeps 1 to {MAX_WINDOW_ENTRIES} entries, not {}.",
            window.entries
        ));
    }
    if !(MIN_WINDOW_CHARS..=MAX_WINDOW_CHARS).contains(&window.max_chars) {
        return Err(format!(
            "Monitor \"{name}\": its window holds {MIN_WINDOW_CHARS} to {MAX_WINDOW_CHARS} \
             characters, not {}.",
            window.max_chars
        ));
    }
    if !(1..=MAX_EVALS_PER_TURN).contains(&monitor.max_evals_per_turn) {
        return Err(format!(
            "Monitor \"{name}\": it can be checked 1 to {MAX_EVALS_PER_TURN} times a turn, not {}.",
            monitor.max_evals_per_turn
        ));
    }
    if let Some(model) = &monitor.model {
        validate_model(model, &format!("Monitor \"{name}\""))?;
    }
    validate_owner(monitor, name)
}

/// Who set the monitor, and what that allows.
fn validate_owner(monitor: &MonitorCriterion, name: &str) -> Result<(), String> {
    match monitor.set_by {
        MonitorOwner::Agent => {
            if !names_a_chat(monitor.set_by_session.as_deref()) {
                return Err(format!(
                    "Monitor \"{name}\" is marked as set by an agent but does not name the chat \
                     whose agent set it."
                ));
            }
            if monitor.inherited_from.is_some() {
                return Err(format!(
                    "Monitor \"{name}\" is marked as set by an agent, so it cannot also be a copy \
                     of a person's monitor from another chat."
                ));
            }
            if monitor.apply_to_subagents {
                return Err(format!(
                    "Monitor \"{name}\" is marked as set by an agent, so it cannot be set to also \
                     watch subagents; only the person sets that."
                ));
            }
            if monitor.action == MonitorAction::Notify {
                return Err(
                    "A monitor an agent sets on a subagent can stop it or steer it; nobody \
                            reads a notice in a subagent's chat."
                        .to_string(),
                );
            }
        }
        MonitorOwner::Person => {
            if monitor.set_by_session.is_some() {
                return Err(format!(
                    "Monitor \"{name}\" names the chat whose agent set it, but it is marked as set \
                     by the person."
                ));
            }
            if let Some(source) = monitor.inherited_from.as_deref() {
                if !names_a_chat(Some(source)) {
                    return Err(format!(
                        "Monitor \"{name}\" is marked as a copy from another chat but does not \
                         name that chat."
                    ));
                }
                if monitor.apply_to_subagents {
                    return Err(format!(
                        "Monitor \"{name}\" is a copy of a monitor from another chat, so it \
                         cannot itself be set to also watch subagents."
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Check a settings value before it is stored. `Err` is the sentence a caller shows.
pub fn validate(settings: &DecisionSettings) -> Result<(), String> {
    if settings.monitors.len() > MAX_MONITORS {
        return Err(format!(
            "A chat can have at most {MAX_MONITORS} monitors, not {}.",
            settings.monitors.len()
        ));
    }
    let agents = agent_monitor_count(&settings.monitors);
    if agents > MAX_AGENT_MONITORS {
        return Err(format!(
            "A chat can have at most {MAX_AGENT_MONITORS} monitors set by an agent, not {agents}."
        ));
    }
    if let Some(model) = &settings.default_model {
        validate_model(model, "The chat's default decision model")?;
    }
    let mut ids = std::collections::HashSet::new();
    for monitor in &settings.monitors {
        validate_monitor(monitor)?;
        if !ids.insert(monitor.id.as_str()) {
            return Err(format!(
                "Two monitors share the id `{}`; each needs its own.",
                monitor.id
            ));
        }
    }
    Ok(())
}

fn agent_monitor_count(monitors: &[MonitorCriterion]) -> usize {
    monitors
        .iter()
        .filter(|m| m.set_by == MonitorOwner::Agent)
        .count()
}

/// Check a decision chat's question before it is stored. `Err` is the sentence
/// a caller shows.
pub fn validate_chat(settings: &DecisionChatSettings) -> Result<(), String> {
    let mut question = IndexMap::new();
    question.insert(CHAT_QUESTION_NAME.to_string(), settings.question.clone());
    validate_questions(&question).map_err(|why| format!("{why}."))
}

/// The chat's decision settings, or the default (no default model, no
/// monitors) when it has none or no such chat exists. A stored value this
/// build cannot read is an error rather than the default, so a later [`save`]
/// over a reader's empty form cannot silently erase it.
pub async fn load(
    session_manager: &SessionManager,
    session_id: &str,
) -> anyhow::Result<DecisionSettings> {
    let stored = session_manager
        .get_extension_state(
            session_id,
            DECISION_SETTINGS_STATE,
            DECISION_SETTINGS_VERSION,
        )
        .await?;
    match stored {
        None => Ok(DecisionSettings::default()),
        Some(value) => serde_json::from_value(value).map_err(|error| {
            anyhow::anyhow!("This chat's decision settings could not be read: {error}")
        }),
    }
}

/// The one transaction every monitors writer goes through. `mutate` sees the
/// stored record (`Err` with the reason when this build cannot read it) and
/// returns the record to store, which is validated before it is written. A
/// refusal from `mutate` or from validation rolls the transaction back, so
/// nothing is written, not even the chat's activity time.
async fn write_settings<F>(
    session_manager: &SessionManager,
    session_id: &str,
    mutate: F,
) -> anyhow::Result<DecisionSettings>
where
    F: FnOnce(Result<Option<DecisionSettings>, String>) -> anyhow::Result<DecisionSettings> + Send,
{
    let mut written: Option<DecisionSettings> = None;
    let stored = session_manager
        .update_extension_state(
            session_id,
            DECISION_SETTINGS_STATE,
            DECISION_SETTINGS_VERSION,
            |current| {
                let current = current
                    .map(|value| {
                        serde_json::from_value::<DecisionSettings>(value.clone())
                            .map_err(|error| error.to_string())
                    })
                    .transpose();
                let next = mutate(current)?;
                validate(&next).map_err(InvalidDecisionSettings)?;
                let value = serde_json::to_value(&next)?;
                written = Some(next);
                Ok(value)
            },
        )
        .await?;
    match (stored, written) {
        (Some(_), Some(settings)) => Ok(settings),
        _ => anyhow::bail!("No chat {session_id} exists."),
    }
}

/// Read, change, validate and store a chat's decision settings in ONE
/// transaction, so a writer never stores a change made over a copy another
/// writer has already replaced. `mutate` sees the stored settings (`None`
/// when the chat has none) and returns the settings to store; its `Err` is a
/// sentence, returned as [`InvalidDecisionSettings`], and so is a value
/// [`validate`] refuses. Either way nothing is written.
///
/// A stored record this build cannot read is never handed to `mutate` as
/// "none": the update is refused with the reason, so an edit cannot erase
/// monitors it could not see. [`save`], which replaces the record whatever
/// it holds, is how a person replaces an unreadable one.
pub async fn update<F>(
    session_manager: &SessionManager,
    session_id: &str,
    mutate: F,
) -> anyhow::Result<DecisionSettings>
where
    F: FnOnce(Option<DecisionSettings>) -> Result<DecisionSettings, String> + Send,
{
    write_settings(session_manager, session_id, move |current| {
        let current = current.map_err(|error| {
            anyhow::anyhow!(
                "This chat's decision settings could not be read ({error}), so they were not \
                 changed."
            )
        })?;
        mutate(current).map_err(|why| InvalidDecisionSettings(why).into())
    })
    .await
}

/// Validate and store, replacing whatever the chat held. Returns the stored
/// value. Goes through the same transaction as [`update`].
pub async fn save(
    session_manager: &SessionManager,
    session_id: &str,
    settings: DecisionSettings,
) -> anyhow::Result<DecisionSettings> {
    validate(&settings).map_err(InvalidDecisionSettings)?;
    write_settings(session_manager, session_id, move |_| Ok(settings)).await
}

/// The decision chat's question, or `None` when it has none or no such chat
/// exists. An unreadable stored value is an error, as in [`load`].
pub async fn load_chat(
    session_manager: &SessionManager,
    session_id: &str,
) -> anyhow::Result<Option<DecisionChatSettings>> {
    let stored = session_manager
        .get_extension_state(session_id, DECISION_CHAT_STATE, DECISION_CHAT_VERSION)
        .await?;
    stored
        .map(|value| {
            serde_json::from_value(value).map_err(|error| {
                anyhow::anyhow!("This decision chat's question could not be read: {error}")
            })
        })
        .transpose()
}

/// Validate and store. Returns the stored value.
pub async fn save_chat(
    session_manager: &SessionManager,
    session_id: &str,
    settings: DecisionChatSettings,
) -> anyhow::Result<DecisionChatSettings> {
    validate_chat(&settings).map_err(InvalidDecisionSettings)?;
    let value = serde_json::to_value(&settings)?;
    let stored = session_manager
        .update_extension_state(
            session_id,
            DECISION_CHAT_STATE,
            DECISION_CHAT_VERSION,
            move |_| Ok(value),
        )
        .await?;
    anyhow::ensure!(stored.is_some(), "No chat {session_id} exists.");
    Ok(settings)
}

// ---------------------------------------------------------------------------
// Monitors an agent sets on the subagents it starts.
// ---------------------------------------------------------------------------

// The four enums below are `#[schemars(inline)]` and their variants carry
// plain `//` comments, not `///` docs, on purpose. [`MonitorSpec`]'s schema is
// in three agent tools (the subagent spawn, `workspace_orchestrate` and
// `workspace_monitor`, the last in every chat's tool list), so every byte here
// is paid on every turn, three times over. A documented variant makes schemars
// emit a `oneOf` of described constants and a referenced enum carries its own
// description beside the field's; the field docs on `MonitorSpec` already say
// what each value means, so either would only repeat them.

/// The answer a [`MonitorSpec`] watches.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum MonitorSpecWatch {
    // Fire when P(yes) reaches the threshold.
    #[default]
    Yes,
    // Fire when P(no) reaches the threshold.
    No,
}

/// What a [`MonitorSpec`] does when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum MonitorSpecAction {
    // End the subagent's turn.
    Stop,
    // Tell the subagent, which keeps going.
    Steer,
}

/// When a [`MonitorSpec`] is checked.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum MonitorSpecTiming {
    // After every step the subagent takes.
    #[default]
    #[serde(alias = "post_iteration")]
    EachStep,
    // Once, when the subagent's turn is about to end.
    TurnEnd,
}

/// What a [`MonitorSpec`] does when a check fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(inline)]
pub enum MonitorSpecOnError {
    // The check is skipped; after three failures in a row the monitor is off
    // for the rest of the turn.
    Notify,
    // The subagent's turn stops, so the rule never goes unchecked.
    Pause,
}

// A monitor as an agent declares it on a subagent: a yes/no question a
// decision model answers from the subagent's newest messages, and what happens
// when the answer reaches the threshold. Taken by the subagent spawn, an
// orchestration phase and `workspace_monitor`; stored through
// [`MonitorSpec::to_criterion`] as a monitor set by the agent.
//
// No `///` on the type and terse `///` on the fields, because those lines are
// the schema the three tools send the model (see the note above the enums):
// they hold what a call needs and nothing else. The finer limits (the id's
// characters, the name's minimum) are checked, and explained, by
// `to_criterion`'s refusals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonitorSpec {
    /// Short name, at most 60 characters.
    pub name: String,
    /// Optional; made from the name. Reuse one you set to replace it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// A yes/no question about the subagent's newest messages.
    pub question: String,
    /// The answer compared with the threshold.
    #[serde(default)]
    pub watch: MonitorSpecWatch,
    /// Fires when P(watched answer) reaches this; above 0, at most 1.
    pub threshold: f64,
    /// "stop" ends the subagent's turn; "steer" redirects it.
    pub action: MonitorSpecAction,
    /// "each_step": after every step; "turn_end": once, as its turn ends.
    #[serde(default)]
    pub timing: MonitorSpecTiming,
    /// provider/model from decision_models. Default: the subagent's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Checks in a row over the threshold before it fires: 1 to 5, default 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consecutive: Option<u32>,
    /// On a failed check: "pause" stops the turn, "notify" skips the check.
    /// Default: pause for stop, notify for steer.
    #[serde(default, alias = "onError", skip_serializing_if = "Option::is_none")]
    pub on_error: Option<MonitorSpecOnError>,
    /// Checks per turn at most: 1 to 200, default 50.
    #[serde(default, alias = "maxChecks", skip_serializing_if = "Option::is_none")]
    pub max_checks: Option<u32>,
}

impl MonitorSpec {
    /// The stored monitor: set by the agent of `owner_session` (the chat that
    /// started the subagent). Without an `id`, a slug of the name not in
    /// `taken_ids` (`-2`, `-3` … appended when it is). `Err` is the sentence
    /// the agent reads.
    pub fn to_criterion(
        &self,
        owner_session: &str,
        taken_ids: &BTreeSet<String>,
    ) -> Result<MonitorCriterion, String> {
        let name = self.name.trim();
        let id = match self
            .id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
        {
            Some(id) => id.to_string(),
            None => unique_monitor_id(&monitor_slug(name), taken_ids),
        };
        let model = match self.model.as_deref().map(str::trim) {
            Some(text) if !text.is_empty() => {
                Some(DecisionModelRef::parse(text).ok_or_else(|| {
                    format!(
                        "Monitor \"{name}\": its model is provider/model, for example \
                     ollama_decision/tev1:0.8b, not `{text}`."
                    )
                })?)
            }
            _ => None,
        };
        let action = match self.action {
            MonitorSpecAction::Stop => MonitorAction::Stop,
            MonitorSpecAction::Steer => MonitorAction::Steer,
        };
        let on_error = match self.on_error {
            Some(MonitorSpecOnError::Pause) => MonitorErrorPolicy::Pause,
            Some(MonitorSpecOnError::Notify) => MonitorErrorPolicy::Notify,
            None if action == MonitorAction::Stop => MonitorErrorPolicy::Pause,
            None => MonitorErrorPolicy::Notify,
        };
        let criterion = MonitorCriterion {
            id,
            name: name.to_string(),
            question: Question {
                instructions: self.question.trim().to_string(),
                kind: DecisionKind::Binary {},
                threshold: None,
            },
            model,
            timing: match self.timing {
                MonitorSpecTiming::EachStep => MonitorTiming::PostIteration,
                MonitorSpecTiming::TurnEnd => MonitorTiming::TurnEnd,
            },
            window: MonitorWindow::default(),
            threshold: self.threshold,
            direction: MonitorDirection::AtOrAbove,
            option: match self.watch {
                MonitorSpecWatch::Yes => None,
                MonitorSpecWatch::No => Some(NO.to_string()),
            },
            consecutive: self.consecutive.unwrap_or(1),
            rearm_below: None,
            rearm_above: None,
            action,
            on_error,
            max_evals_per_turn: self.max_checks.unwrap_or(DEFAULT_EVALS_PER_TURN),
            enabled: true,
            set_by: MonitorOwner::Agent,
            set_by_session: Some(owner_session.to_string()),
            inherited_from: None,
            apply_to_subagents: false,
        };
        validate_monitor(&criterion)?;
        Ok(criterion)
    }
}

/// A monitor id made from its name: lowercase letters and digits, every other
/// run of characters one `-`.
fn monitor_slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out
        .trim_matches('-')
        .chars()
        .take(MAX_MONITOR_ID_CHARS)
        .collect();
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "monitor".to_string()
    } else {
        out.to_string()
    }
}

/// `base`, or `base-2`, `base-3` … whichever is not taken, cut so the
/// suffix still fits an id.
fn unique_monitor_id(base: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    (2usize..)
        .map(|n| {
            let suffix = format!("-{n}");
            let stem: String = base
                .chars()
                .take(MAX_MONITOR_ID_CHARS.saturating_sub(suffix.len()))
                .collect();
            format!("{}{suffix}", stem.trim_end_matches('-'))
        })
        .find(|candidate| !taken.contains(candidate))
        .expect("a finite set of ids leaves one free")
}

/// A change an agent makes to the monitors of a subagent it started.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEdit {
    /// Add these monitors, or replace the caller's own monitors with the same
    /// ids. Each is a monitor set by the caller ([`MonitorSpec::to_criterion`]).
    Upsert(Vec<MonitorCriterion>),
    /// Remove the caller's own monitors with these ids.
    Remove(Vec<String>),
}

/// Apply an agent's edit to the monitors of subagent `child_id`, all or
/// nothing, in one transaction. The agent of `caller_id` may change only the
/// monitors it set: never a person's (a copy of a parent chat's included),
/// never one another chat's agent set, and never the chat's default decision
/// model. `Err` is the sentence the agent reads, ending in "Nothing was
/// changed."
///
/// The caller checks that the target is its direct subagent, that it may
/// write it, and that the monitors' models may read it, before calling.
pub async fn apply_agent_edit(
    session_manager: &SessionManager,
    child_id: &str,
    caller_id: &str,
    edit: AgentEdit,
) -> Result<DecisionSettings, String> {
    update(session_manager, child_id, |current| {
        agent_edited(current.unwrap_or_default(), caller_id, edit)
    })
    .await
    .map_err(
        |error| match error.downcast_ref::<InvalidDecisionSettings>() {
            Some(InvalidDecisionSettings(why)) => nothing_changed(why),
            None => nothing_changed(&error.to_string()),
        },
    )
}

fn nothing_changed(why: &str) -> String {
    let why = why.trim_end();
    if why.ends_with("Nothing was changed.") {
        return why.to_string();
    }
    format!("{}. Nothing was changed.", why.trim_end_matches('.'))
}

/// Whether the agent of `caller` may change or remove `monitor`.
fn agent_may_change(monitor: &MonitorCriterion, caller: &str) -> Result<(), String> {
    let id = &monitor.id;
    match monitor.set_by {
        MonitorOwner::Person => Err(format!(
            "Monitor `{id}` on this subagent was set by the person, so you cannot change or \
             remove it. Nothing was changed."
        )),
        MonitorOwner::Agent if monitor.set_by_session.as_deref() != Some(caller) => Err(format!(
            "Monitor `{id}` on this subagent was set by another chat's agent, so you cannot \
             change or remove it. Nothing was changed."
        )),
        MonitorOwner::Agent => Ok(()),
    }
}

/// [`apply_agent_edit`]'s rule, without the store.
fn agent_edited(
    mut settings: DecisionSettings,
    caller: &str,
    edit: AgentEdit,
) -> Result<DecisionSettings, String> {
    let before = settings.monitors.len();
    match edit {
        AgentEdit::Upsert(entries) => {
            let mut seen = BTreeSet::new();
            for entry in entries {
                if entry.set_by != MonitorOwner::Agent
                    || entry.set_by_session.as_deref() != Some(caller)
                {
                    return Err(format!(
                        "Monitor `{}` is not marked as set by your chat's agent, so it cannot be \
                         put on this subagent. Nothing was changed.",
                        entry.id
                    ));
                }
                if !seen.insert(entry.id.clone()) {
                    return Err(format!(
                        "Monitor `{}` is named twice in one change. Nothing was changed.",
                        entry.id
                    ));
                }
                match settings.monitors.iter().position(|m| m.id == entry.id) {
                    Some(at) => {
                        agent_may_change(&settings.monitors[at], caller)?;
                        settings.monitors[at] = entry;
                    }
                    None => settings.monitors.push(entry),
                }
            }
        }
        AgentEdit::Remove(ids) => {
            let mut removed = BTreeSet::new();
            for id in ids {
                if !removed.insert(id.clone()) {
                    continue;
                }
                let at = settings
                    .monitors
                    .iter()
                    .position(|m| m.id == id)
                    .ok_or_else(|| {
                        format!("This subagent has no monitor `{id}`. Nothing was changed.")
                    })?;
                agent_may_change(&settings.monitors[at], caller)?;
                settings.monitors.remove(at);
            }
        }
    }
    if settings.monitors.len() > MAX_MONITORS
        || agent_monitor_count(&settings.monitors) > MAX_AGENT_MONITORS
    {
        return Err(format!(
            "This subagent has {before} monitors. A chat holds at most {MAX_MONITORS}, and at \
             most {MAX_AGENT_MONITORS} of them set by an agent. Nothing was changed."
        ));
    }
    Ok(settings)
}

/// The copies a subagent of `parent_id` takes of the parent's monitors: every
/// enabled monitor set to also watch subagents, as the person's monitor, with
/// the same id, marked as copied from the parent.
fn inherited_copies(parent: &DecisionSettings, parent_id: &str) -> Vec<MonitorCriterion> {
    parent
        .monitors
        .iter()
        .filter(|m| m.enabled && m.apply_to_subagents)
        .map(|m| MonitorCriterion {
            set_by: MonitorOwner::Person,
            set_by_session: None,
            inherited_from: Some(parent_id.to_string()),
            apply_to_subagents: false,
            ..m.clone()
        })
        .collect()
}

/// Where one of a new subagent's monitors came from, as a refusal names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildSource {
    Inherited,
    Workflow,
    Phase,
    Spawn,
}

impl ChildSource {
    fn describe(self) -> &'static str {
        match self {
            ChildSource::Inherited => "a monitor of this chat set to also watch its subagents",
            ChildSource::Workflow => "one of the sub-workflow's monitors",
            ChildSource::Phase => "one of the phase's monitors",
            ChildSource::Spawn => "one of this spawn's monitors",
        }
    }
}

/// The decision settings a new subagent of `parent_id` starts with, or `None`
/// when there is nothing to write (no default model and no monitors).
///
/// - The default decision model is the parent's, else the sub-workflow's.
/// - The monitors are, in order: copies of the parent's enabled monitors set
///   to also watch subagents (the person's, marked as copied from the
///   parent); the sub-workflow's monitors (the person's); the phase's
///   monitors; the spawn's monitors. The phase and spawn entries are taken
///   as given, so they are the agent's ([`MonitorSpec::to_criterion`]).
/// - A spawn monitor replaces a phase monitor with the same id. Any other
///   shared id is refused with a sentence naming it.
///
/// The result is validated, so a subagent never starts with settings it
/// could not have saved.
pub fn child_settings(
    parent: Option<&DecisionSettings>,
    parent_id: &str,
    workflow: Option<&DecisionSettings>,
    phase: &[MonitorCriterion],
    spawn: &[MonitorCriterion],
) -> Result<Option<DecisionSettings>, String> {
    let default_model = parent
        .and_then(|p| p.default_model.clone())
        .or_else(|| workflow.and_then(|w| w.default_model.clone()));

    let mut placed: Vec<(MonitorCriterion, ChildSource)> = Vec::new();
    let mut place = |monitor: MonitorCriterion, source: ChildSource| -> Result<(), String> {
        match placed.iter().position(|(m, _)| m.id == monitor.id) {
            Some(at) if source == ChildSource::Spawn && placed[at].1 == ChildSource::Phase => {
                placed[at] = (monitor, source);
                Ok(())
            }
            Some(at) => Err(format!(
                "Monitor id `{}` is taken twice on this subagent, by {} and by {}; give one of \
                 them another id.",
                monitor.id,
                placed[at].1.describe(),
                source.describe()
            )),
            None => {
                placed.push((monitor, source));
                Ok(())
            }
        }
    };
    if let Some(parent) = parent {
        for monitor in inherited_copies(parent, parent_id) {
            place(monitor, ChildSource::Inherited)?;
        }
    }
    if let Some(workflow) = workflow {
        for monitor in &workflow.monitors {
            // A sub-workflow's monitors are the person's: the workflow's
            // author set them, and no agent did.
            let monitor = MonitorCriterion {
                set_by: MonitorOwner::Person,
                set_by_session: None,
                inherited_from: None,
                apply_to_subagents: false,
                ..monitor.clone()
            };
            place(monitor, ChildSource::Workflow)?;
        }
    }
    for monitor in phase {
        place(monitor.clone(), ChildSource::Phase)?;
    }
    for monitor in spawn {
        place(monitor.clone(), ChildSource::Spawn)?;
    }

    let monitors: Vec<MonitorCriterion> = placed.into_iter().map(|(m, _)| m).collect();
    if default_model.is_none() && monitors.is_empty() {
        return Ok(None);
    }
    let settings = DecisionSettings {
        default_model,
        monitors,
    };
    validate(&settings)?;
    Ok(Some(settings))
}

/// What [`sync_inherited`] did to one subagent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub child_id: String,
    /// Ids of the agent's monitors removed to make room for the person's.
    pub dropped: Vec<String>,
    /// Why the subagent's monitors were not changed, when they were not.
    pub error: Option<String>,
}

/// The notice a subagent's chat keeps when a monitor its agent set is removed
/// to make room for the person's.
pub fn dropped_for_room_notice(name: &str) -> String {
    format!(
        "Monitor '{name}', set by the agent, was removed to make room for the person's monitors."
    )
}

/// A subagent's settings with its copies of `parent_id`'s monitors replaced
/// by `fresh`, and the agent's monitors (id, name) removed to make room. The
/// copies come first, as a new subagent has them. The person wins: an agent's
/// monitor with a copy's id, or one past [`MAX_MONITORS`], is removed, newest
/// first. A person's own monitor with a copy's id is refused.
fn resynced(
    current: DecisionSettings,
    parent_id: &str,
    fresh: &[MonitorCriterion],
) -> Result<(DecisionSettings, Vec<(String, String)>), String> {
    let DecisionSettings {
        default_model,
        monitors,
    } = current;
    let mut dropped = Vec::new();
    let mut next: Vec<MonitorCriterion> = fresh.to_vec();
    for monitor in monitors {
        if monitor.inherited_from.as_deref() == Some(parent_id) {
            continue;
        }
        if fresh.iter().any(|copy| copy.id == monitor.id) {
            if monitor.set_by == MonitorOwner::Agent {
                dropped.push((monitor.id, monitor.name));
                continue;
            }
            return Err(format!(
                "Monitor id `{}` is used by one of this subagent's own monitors, so the copy of \
                 the person's monitor from chat {parent_id} could not be put on it.",
                monitor.id
            ));
        }
        next.push(monitor);
    }
    while next.len() > MAX_MONITORS {
        match next.iter().rposition(|m| m.set_by == MonitorOwner::Agent) {
            Some(at) => {
                let monitor = next.remove(at);
                dropped.push((monitor.id, monitor.name));
            }
            None => {
                return Err(format!(
                    "This subagent would hold {} of the person's monitors, more than the \
                     {MAX_MONITORS} a chat holds.",
                    next.len()
                ))
            }
        }
    }
    Ok((
        DecisionSettings {
            default_model,
            monitors: next,
        },
        dropped,
    ))
}

/// After a person saves `parent_id`'s monitors (`saved`), bring the copies
/// its running subagents hold up to date: each child in `live_children` gets
/// fresh copies of the parent's monitors set to also watch subagents, in
/// place of the ones it holds from this parent (copies from any other chat,
/// the person's own monitors on the child and the agent's are kept).
///
/// When a child would then hold more than [`MAX_MONITORS`], the monitors its
/// agent set are removed, newest first, and its chat keeps a notice for each
/// ([`dropped_for_room_notice`]). A child whose copies are already current is
/// not written, so its activity time does not move (R-30).
///
/// One report per child, in order. A failure leaves that child's monitors as
/// they were; it is in the report and logged at WARN.
pub async fn sync_inherited(
    session_manager: &SessionManager,
    parent_id: &str,
    saved: &DecisionSettings,
    live_children: &[String],
) -> Vec<SyncReport> {
    let fresh = inherited_copies(saved, parent_id);
    let mut reports = Vec::with_capacity(live_children.len());
    for child in live_children {
        let report = sync_child(session_manager, parent_id, &fresh, child).await;
        if let Some(error) = &report.error {
            tracing::warn!(
                parent = %parent_id,
                child = %child,
                "a subagent's copies of its parent's monitors were not updated: {error}"
            );
        }
        reports.push(report);
    }
    reports
}

async fn sync_child(
    session_manager: &SessionManager,
    parent_id: &str,
    fresh: &[MonitorCriterion],
    child: &str,
) -> SyncReport {
    let report = |dropped: Vec<String>, error: Option<String>| SyncReport {
        child_id: child.to_string(),
        dropped,
        error,
    };
    // A read first: a child whose copies are already current is left alone,
    // since any write stamps its activity time.
    let current = match load(session_manager, child).await {
        Ok(current) => current,
        Err(error) => return report(Vec::new(), Some(error.to_string())),
    };
    match resynced(current.clone(), parent_id, fresh) {
        Err(why) => return report(Vec::new(), Some(why)),
        Ok((next, _)) if next == current => return report(Vec::new(), None),
        Ok(_) => {}
    }

    let mut dropped: Vec<(String, String)> = Vec::new();
    let written = update(session_manager, child, |current| {
        let (next, removed) = resynced(current.unwrap_or_default(), parent_id, fresh)?;
        dropped = removed;
        Ok(next)
    })
    .await;
    if let Err(error) = written {
        return report(Vec::new(), Some(error.to_string()));
    }
    for (_, name) in &dropped {
        let notice = Message::assistant()
            .with_system_notification(
                SystemNotificationType::InlineMessage,
                dropped_for_room_notice(name),
            )
            .user_only();
        if let Err(error) = session_manager.add_message(child, &notice).await {
            tracing::warn!(
                child = %child,
                "the notice that an agent's monitor was removed could not be stored: {error}"
            );
        }
    }
    report(dropped.into_iter().map(|(id, _)| id).collect(), None)
}

// ---------------------------------------------------------------------------
// What a monitor's stop tells the chat that started the subagent.
// ---------------------------------------------------------------------------

/// Why a monitor stopped a turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MonitorStopReason {
    /// Its answer crossed the threshold.
    Fired,
    /// It is set to pause when it cannot check, and a check failed.
    CouldNotCheck { error: String },
    /// It is set to pause when it cannot check, and it reached its checks
    /// for the turn.
    CheckLimit { limit: u32 },
    /// A copy of it on a subagent of this chat stopped that subagent.
    ChildStopped {
        child_session_id: String,
        child_title: String,
    },
    /// The chat's monitors could not be read, and one of them is set to pause
    /// when it cannot check.
    Unreadable { error: String },
}

/// One monitor's stop of a turn, as the monitor engine records it and the
/// chat that started the subagent reads it in the subagent's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorStopReport {
    /// The chat whose turn stopped.
    pub session_id: String,
    pub monitor_id: String,
    pub name: String,
    pub set_by: MonitorOwner,
    pub set_by_session: Option<String>,
    pub inherited_from: Option<String>,
    /// The monitor's question, at most [`MAX_STOP_REPORT_QUESTION_CHARS`]
    /// characters.
    pub question: String,
    pub threshold: f64,
    /// It fires below its threshold.
    pub below: bool,
    /// The probability that decided, when a check answered.
    pub p: Option<f64>,
    pub reason: MonitorStopReason,
}

impl MonitorStopReport {
    /// The report for `criterion` stopping `session_id`'s turn.
    pub fn new(
        session_id: &str,
        criterion: &MonitorCriterion,
        p: Option<f64>,
        reason: MonitorStopReason,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            monitor_id: criterion.id.clone(),
            name: criterion.name.clone(),
            set_by: criterion.set_by,
            set_by_session: criterion.set_by_session.clone(),
            inherited_from: criterion.inherited_from.clone(),
            question: clipped_question(&criterion.question.instructions),
            threshold: criterion.threshold,
            below: criterion.direction == MonitorDirection::Below,
            p,
            reason,
        }
    }
}

/// `text` cut to [`MAX_STOP_REPORT_QUESTION_CHARS`], ending in `…` when cut.
pub fn clipped_question(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_STOP_REPORT_QUESTION_CHARS {
        return text.to_string();
    }
    let mut out: String = text
        .chars()
        .take(MAX_STOP_REPORT_QUESTION_CHARS - 1)
        .collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// The monitor engine's read, at every point of a turn.
// ---------------------------------------------------------------------------

/// What [`read_for_monitoring`] found.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsRead {
    /// The stored text is the one read last time; nothing was parsed.
    Unchanged,
    /// The chat has no decision settings (or no such chat exists).
    Absent,
    /// New settings, with the stored text to compare next time.
    Read {
        settings: DecisionSettings,
        text: String,
    },
    /// The settings could not be read. `pause_named` is true when the stored
    /// record visibly holds an enabled monitor set to pause when it cannot
    /// check, so the engine can fail closed for it.
    Unreadable { error: String, pause_named: bool },
}

/// Read the chat's decision settings for the monitor engine, cheaply: one
/// key of the chat's row as stored text, parsed only when it differs from
/// `last_text` (the text of the engine's last good read).
pub async fn read_for_monitoring(
    session_manager: &SessionManager,
    session_id: &str,
    last_text: Option<&str>,
) -> SettingsRead {
    let text = match session_manager
        .get_extension_state_json(
            session_id,
            DECISION_SETTINGS_STATE,
            DECISION_SETTINGS_VERSION,
        )
        .await
    {
        Ok(Some(text)) => text,
        Ok(None) => return SettingsRead::Absent,
        Err(error) => {
            return SettingsRead::Unreadable {
                error: error.to_string(),
                pause_named: false,
            }
        }
    };
    if last_text == Some(text.as_str()) {
        return SettingsRead::Unchanged;
    }
    match serde_json::from_str::<DecisionSettings>(&text) {
        Ok(settings) => SettingsRead::Read { settings, text },
        Err(error) => SettingsRead::Unreadable {
            error: error.to_string(),
            pause_named: pause_named_in(&text),
        },
    }
}

/// A lenient look at a record this build cannot read: does it hold a monitor,
/// not switched off, set to pause when it cannot check (`onError` `pause`)?
fn pause_named_in(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    value
        .get("monitors")
        .and_then(|monitors| monitors.as_array())
        .is_some_and(|monitors| {
            monitors.iter().any(|monitor| {
                monitor.get("onError").and_then(|v| v.as_str()) == Some("pause")
                    && monitor.get("enabled") != Some(&serde_json::Value::Bool(false))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::MessageContent;
    use crate::session::session_manager::SessionType;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn binary() -> Question {
        Question {
            instructions: "Has the agent reached the consent form page?".to_string(),
            kind: DecisionKind::Binary {},
            threshold: None,
        }
    }

    fn monitor(id: &str) -> MonitorCriterion {
        MonitorCriterion {
            id: id.to_string(),
            name: "Form reached".to_string(),
            question: binary(),
            model: None,
            timing: MonitorTiming::PostIteration,
            window: MonitorWindow::default(),
            threshold: 0.85,
            direction: MonitorDirection::AtOrAbove,
            option: None,
            consecutive: 1,
            rearm_below: Some(0.7),
            rearm_above: None,
            action: MonitorAction::Stop,
            on_error: MonitorErrorPolicy::Notify,
            max_evals_per_turn: 50,
            enabled: true,
            set_by: MonitorOwner::Person,
            set_by_session: None,
            inherited_from: None,
            apply_to_subagents: false,
        }
    }

    /// A monitor the agent of `caller` set.
    fn agent_monitor(id: &str, caller: &str) -> MonitorCriterion {
        MonitorCriterion {
            name: format!("Agent {id}"),
            rearm_below: None,
            set_by: MonitorOwner::Agent,
            set_by_session: Some(caller.to_string()),
            ..monitor(id)
        }
    }

    /// A person's monitor set to also watch the chat's subagents.
    fn for_subagents(id: &str) -> MonitorCriterion {
        MonitorCriterion {
            name: format!("Person {id}"),
            apply_to_subagents: true,
            ..monitor(id)
        }
    }

    fn refused(settings: &DecisionSettings, needle: &str) {
        let why = validate(settings).expect_err("validation should refuse this");
        assert!(why.contains(needle), "{why:?} does not mention {needle:?}");
        assert!(why.ends_with('.'), "{why:?} is not a sentence");
    }

    fn with(edit: impl FnOnce(&mut MonitorCriterion)) -> DecisionSettings {
        let mut m = monitor("form-reached");
        edit(&mut m);
        DecisionSettings {
            default_model: None,
            monitors: vec![m],
        }
    }

    fn of(monitors: Vec<MonitorCriterion>) -> DecisionSettings {
        DecisionSettings {
            default_model: None,
            monitors,
        }
    }

    fn ids(settings: &DecisionSettings) -> Vec<&str> {
        settings.monitors.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn a_well_formed_value_passes_and_the_wire_shape_is_camel_case() {
        let settings = DecisionSettings {
            default_model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            monitors: vec![monitor("form-reached"), monitor("task_done")],
        };
        validate(&settings).unwrap();
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["defaultModel"]["provider"], "ollama_decision");
        assert_eq!(json["monitors"][0]["rearmBelow"], 0.7);
        assert_eq!(json["monitors"][0]["window"]["maxChars"], 6000);
        assert_eq!(json["monitors"][0]["timing"], "post_iteration");

        // The serde defaults fill what a minimal criterion leaves out.
        let minimal: MonitorCriterion = serde_json::from_value(serde_json::json!({
            "id": "m1", "name": "Form reached", "threshold": 0.9, "option": null,
            "model": null, "rearmBelow": null,
            "question": {"instructions": "Is the form reached?", "kind": {"binary": {}}}
        }))
        .unwrap();
        assert_eq!(minimal.consecutive, 1);
        assert_eq!(minimal.max_evals_per_turn, 50);
        assert!(minimal.enabled);
        assert_eq!(minimal.action, MonitorAction::Notify);
        assert_eq!(minimal.window, MonitorWindow::default());
    }

    /// A record as the build before the ownership and direction keys wrote
    /// it reads as the person's monitor firing at or above its threshold, and
    /// is written back byte for byte.
    #[test]
    fn old_records_read_as_person_monitors() {
        let stored = concat!(
            r#"{"defaultModel":null,"monitors":[{"id":"form-reached","name":"Form reached","#,
            r#""question":{"instructions":"Has the agent reached the consent form page?","#,
            r#""kind":{"binary":{}}},"model":null,"timing":"post_iteration","#,
            r#""window":{"entries":24,"maxChars":6000,"toolCalls":true},"threshold":0.85,"#,
            r#""option":null,"consecutive":1,"rearmBelow":0.7,"action":"stop","#,
            r#""onError":"notify","maxEvalsPerTurn":50,"enabled":true}]}"#
        );
        let read: DecisionSettings = serde_json::from_str(stored).unwrap();
        let m = &read.monitors[0];
        assert_eq!(m.set_by, MonitorOwner::Person);
        assert_eq!(m.set_by_session, None);
        assert_eq!(m.inherited_from, None);
        assert!(!m.apply_to_subagents);
        assert_eq!(m.direction, MonitorDirection::AtOrAbove);
        assert_eq!(m.rearm_above, None);
        assert_eq!(read, of(vec![monitor("form-reached")]));
        validate(&read).unwrap();
        assert_eq!(serde_json::to_string(&read).unwrap(), stored);
    }

    #[test]
    fn an_at_or_above_monitor_writes_no_new_keys() {
        let json = serde_json::to_value(monitor("m")).unwrap();
        for key in [
            "direction",
            "rearmAbove",
            "setBy",
            "setBySession",
            "inheritedFrom",
            "applyToSubagents",
        ] {
            assert!(json.get(key).is_none(), "{key} written: {json}");
        }

        let below = MonitorCriterion {
            threshold: 0.3,
            direction: MonitorDirection::Below,
            rearm_below: None,
            rearm_above: Some(0.6),
            ..monitor("below")
        };
        let agent = MonitorCriterion {
            action: MonitorAction::Steer,
            ..agent_monitor("agent", "parent-1")
        };
        let copy = MonitorCriterion {
            inherited_from: Some("parent-1".to_string()),
            ..monitor("copy")
        };
        let settings = of(vec![below, agent, copy, for_subagents("swarm")]);
        validate(&settings).unwrap();
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["monitors"][0]["direction"], "below");
        assert_eq!(json["monitors"][0]["rearmAbove"], 0.6);
        assert_eq!(json["monitors"][1]["setBy"], "agent");
        assert_eq!(json["monitors"][1]["setBySession"], "parent-1");
        assert!(json["monitors"][1].get("direction").is_none());
        assert_eq!(json["monitors"][2]["inheritedFrom"], "parent-1");
        assert!(json["monitors"][2].get("setBy").is_none());
        assert_eq!(json["monitors"][3]["applyToSubagents"], true);
        let read: DecisionSettings = serde_json::from_value(json).unwrap();
        assert_eq!(read, settings);
    }

    /// A key the record does not have is refused at every level, rather than
    /// dropped with its field left at a default that fails open.
    #[test]
    fn a_key_the_record_does_not_have_is_refused_at_every_level() {
        let full = serde_json::to_value(DecisionSettings {
            default_model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            monitors: vec![monitor("form-reached")],
        })
        .unwrap();
        // Every field written, read back unchanged: a stored record (always
        // written from these structs) has no key they refuse.
        let read: DecisionSettings = serde_json::from_value(full.clone()).unwrap();
        assert_eq!(serde_json::to_value(&read).unwrap(), full);

        let refused = |edit: &dyn Fn(&mut serde_json::Value), needle: &str| {
            let mut value = full.clone();
            edit(&mut value);
            let error = serde_json::from_value::<DecisionSettings>(value.clone())
                .expect_err(&format!("{value} was read"));
            assert!(
                error.to_string().contains(needle),
                "{error} does not name {needle}"
            );
        };
        refused(
            &|v| v["Monitors"] = serde_json::json!([]),
            "unknown field `Monitors`",
        );
        refused(
            &|v| v["monitors"][0]["on_error"] = serde_json::json!("pause"),
            "unknown field `on_error`",
        );
        refused(
            &|v| v["monitors"][0]["Action"] = serde_json::json!("stop"),
            "unknown field `Action`",
        );
        refused(
            &|v| v["monitors"][0]["window"]["max_chars"] = serde_json::json!(800),
            "unknown field `max_chars`",
        );
        refused(
            &|v| v["monitors"][0]["Direction"] = serde_json::json!("below"),
            "unknown field `Direction`",
        );
        refused(
            &|v| v["monitors"][0]["rearm_above"] = serde_json::json!(0.9),
            "unknown field `rearm_above`",
        );
        refused(
            &|v| v["monitors"][0]["set_by"] = serde_json::json!("agent"),
            "unknown field `set_by`",
        );
        refused(
            &|v| v["monitors"][0]["direction"] = serde_json::json!("above"),
            "unknown variant `above`",
        );

        let chat = serde_json::to_value(DecisionChatSettings { question: binary() }).unwrap();
        let read: DecisionChatSettings = serde_json::from_value(chat.clone()).unwrap();
        assert_eq!(serde_json::to_value(&read).unwrap(), chat);
        let mut misspelled = chat;
        misspelled["questions"] = misspelled["question"].clone();
        let error = serde_json::from_value::<DecisionChatSettings>(misspelled).unwrap_err();
        assert!(
            error.to_string().contains("unknown field `questions`"),
            "{error}"
        );
    }

    #[test]
    fn each_bound_is_refused_with_a_sentence() {
        refused(
            &DecisionSettings {
                default_model: None,
                monitors: (0..=MAX_MONITORS)
                    .map(|i| monitor(&format!("m{i}")))
                    .collect(),
            },
            "at most 8 monitors",
        );
        refused(
            &DecisionSettings {
                default_model: None,
                monitors: vec![monitor("same"), monitor("same")],
            },
            "share the id",
        );
        for id in ["", "Form", "-lead", "has space", &"x".repeat(65)] {
            refused(&with(|m| m.id = id.to_string()), "Monitor id");
        }
        refused(&with(|m| m.name = "  ".to_string()), "needs a name");
        refused(&with(|m| m.name = "n".repeat(61)), "needs a name");
        refused(
            &with(|m| m.question.instructions = String::new()),
            "instructions",
        );
        refused(
            &with(|m| m.threshold = 1.2),
            "threshold must be a probability",
        );
        refused(
            &with(|m| m.threshold = f64::NAN),
            "threshold must be a probability",
        );
        refused(&with(|m| m.consecutive = 0), "1 to 5 evaluations");
        refused(&with(|m| m.consecutive = 6), "1 to 5 evaluations");
        refused(&with(|m| m.rearm_below = Some(0.9)), "re-arms below");
        refused(&with(|m| m.window.entries = 0), "1 to 24 entries");
        refused(&with(|m| m.window.entries = 25), "1 to 24 entries");
        refused(&with(|m| m.window.entries = 48), "1 to 24 entries");
        refused(
            &with(|m| m.window.max_chars = 6_001),
            "200 to 6000 characters",
        );
        refused(
            &with(|m| m.window.max_chars = 12_000),
            "200 to 6000 characters",
        );
        refused(&with(|m| m.window.max_chars = 10), "200 to 6000 characters");
        refused(&with(|m| m.max_evals_per_turn = 201), "1 to 200 times");
        refused(&with(|m| m.max_evals_per_turn = 0), "1 to 200 times");
        refused(
            &with(|m| m.model = Some(DecisionModelRef::new("", "tev1"))),
            "without a provider",
        );
        refused(
            &DecisionSettings {
                default_model: Some(DecisionModelRef::new("ollama_decision", " ")),
                monitors: Vec::new(),
            },
            "default decision model",
        );
        refused(
            &with(|m| m.option = Some("maybe".to_string())),
            "`yes` or `no`",
        );
        refused(
            &with(|m| m.question.threshold = Some(Threshold::Yes(0.5))),
            "threshold of its own",
        );
    }

    #[test]
    fn the_direction_and_its_rearm_field_are_checked() {
        // 0 fires on every check (or, below, never); 1 is a real rule.
        refused(
            &with(|m| m.threshold = 0.0),
            "the threshold must be a probability above 0 and at most 1, not 0.",
        );
        refused(
            &with(|m| m.threshold = -0.1),
            "threshold must be a probability",
        );
        validate(&with(|m| m.threshold = 1.0)).unwrap();
        validate(&with(|m| {
            m.threshold = 1.0;
            m.direction = MonitorDirection::Below;
            m.rearm_below = None;
            m.rearm_above = Some(1.0);
        }))
        .unwrap();

        refused(
            &with(|m| {
                m.rearm_below = None;
                m.rearm_above = Some(0.9);
            }),
            "it fires at or above its threshold, so it re-arms below a probability; set \
             `rearmBelow`, not `rearmAbove`.",
        );
        let below = |edit: &dyn Fn(&mut MonitorCriterion)| {
            with(|m| {
                m.threshold = 0.3;
                m.direction = MonitorDirection::Below;
                m.rearm_below = None;
                edit(m);
            })
        };
        validate(&below(&|_| {})).unwrap();
        validate(&below(&|m| m.rearm_above = Some(0.3))).unwrap();
        validate(&below(&|m| m.rearm_above = Some(0.6))).unwrap();
        refused(
            &below(&|m| m.rearm_below = Some(0.1)),
            "it fires below its threshold, so it re-arms above a probability; set `rearmAbove`, \
             not `rearmBelow`.",
        );
        refused(
            &below(&|m| m.rearm_above = Some(0.2)),
            "it re-arms at or above a probability between its threshold (0.3) and 1, not 0.2.",
        );
        refused(
            &below(&|m| m.rearm_above = Some(1.5)),
            "between its threshold (0.3) and 1, not 1.5.",
        );
        refused(
            &below(&|m| m.rearm_above = Some(f64::NAN)),
            "between its threshold (0.3) and 1",
        );
    }

    #[test]
    fn an_agent_monitor_may_not_notify_or_apply_to_subagents() {
        validate(&of(vec![agent_monitor("stop", "parent-1")])).unwrap();
        validate(&of(vec![MonitorCriterion {
            action: MonitorAction::Steer,
            ..agent_monitor("steer", "parent-1")
        }]))
        .unwrap();

        let agent = |edit: &dyn Fn(&mut MonitorCriterion)| {
            let mut m = agent_monitor("a", "parent-1");
            edit(&mut m);
            of(vec![m])
        };
        refused(
            &agent(&|m| m.action = MonitorAction::Notify),
            "A monitor an agent sets on a subagent can stop it or steer it; nobody reads a \
             notice in a subagent's chat.",
        );
        refused(
            &agent(&|m| m.apply_to_subagents = true),
            "cannot be set to also watch subagents",
        );
        refused(
            &agent(&|m| m.inherited_from = Some("parent-1".to_string())),
            "cannot also be a copy",
        );
        refused(
            &agent(&|m| m.set_by_session = None),
            "does not name the chat whose agent set it",
        );
        refused(
            &agent(&|m| m.set_by_session = Some("  ".to_string())),
            "does not name the chat whose agent set it",
        );

        refused(
            &with(|m| m.set_by_session = Some("parent-1".to_string())),
            "it is marked as set by the person",
        );
        refused(
            &with(|m| {
                m.inherited_from = Some("parent-1".to_string());
                m.apply_to_subagents = true;
            }),
            "cannot itself be set to also watch subagents",
        );
        refused(
            &with(|m| m.inherited_from = Some(String::new())),
            "does not name that chat",
        );
    }

    #[test]
    fn agent_monitors_are_capped_at_four_within_eight() {
        let agents = |n: usize| -> Vec<MonitorCriterion> {
            (0..n)
                .map(|i| agent_monitor(&format!("a{i}"), "parent-1"))
                .collect()
        };
        let people = |n: usize| -> Vec<MonitorCriterion> {
            (0..n).map(|i| monitor(&format!("p{i}"))).collect()
        };
        validate(&of(agents(4))).unwrap();
        refused(&of(agents(5)), "at most 4 monitors set by an agent, not 5.");
        validate(&of([people(4), agents(4)].concat())).unwrap();
        refused(&of([people(5), agents(4)].concat()), "at most 8 monitors");

        // The same caps, through the agent's edit.
        let seven = agent_edited(of(people(3)), "parent-1", AgentEdit::Upsert(agents(4))).unwrap();
        assert_eq!(seven.monitors.len(), 7);
        let fifth = agent_monitor("a4", "parent-1");
        assert_eq!(
            agent_edited(seven.clone(), "parent-1", AgentEdit::Upsert(vec![fifth])).unwrap_err(),
            "This subagent has 7 monitors. A chat holds at most 8, and at most 4 of them set by \
             an agent. Nothing was changed."
        );
        let full = of([people(6), agents(2)].concat());
        assert!(agent_edited(
            full,
            "parent-1",
            AgentEdit::Upsert(vec![agent_monitor("a9", "parent-1")])
        )
        .unwrap_err()
        .starts_with("This subagent has 8 monitors."));
        // Replacing one of its own is not adding one.
        let replaced = agent_edited(
            seven,
            "parent-1",
            AgentEdit::Upsert(vec![MonitorCriterion {
                threshold: 0.5,
                ..agent_monitor("a3", "parent-1")
            }]),
        )
        .unwrap();
        assert_eq!(replaced.monitors.len(), 7);
        assert_eq!(replaced.monitors[6].threshold, 0.5);
    }

    #[tokio::test]
    async fn an_agent_cannot_change_remove_or_reuse_a_persons_monitor() {
        let (_dir, sm, child) = store().await;
        let copy = MonitorCriterion {
            name: "Safety".to_string(),
            inherited_from: Some("parent-1".to_string()),
            ..monitor("safety")
        };
        let original = DecisionSettings {
            default_model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            monitors: vec![monitor("form-reached"), copy],
        };
        save(&sm, &child, original.clone()).await.unwrap();
        let persons = |settings: &DecisionSettings| {
            serde_json::to_string(
                &settings
                    .monitors
                    .iter()
                    .filter(|m| m.set_by == MonitorOwner::Person)
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        };
        let before = persons(&original);

        for id in ["form-reached", "safety"] {
            for edit in [
                AgentEdit::Upsert(vec![agent_monitor(id, "parent-1")]),
                AgentEdit::Remove(vec![id.to_string()]),
            ] {
                assert_eq!(
                    apply_agent_edit(&sm, &child, "parent-1", edit)
                        .await
                        .unwrap_err(),
                    format!(
                        "Monitor `{id}` on this subagent was set by the person, so you cannot \
                         change or remove it. Nothing was changed."
                    )
                );
            }
        }
        // A batch is all or nothing: the good entry before the bad one is
        // not kept.
        let error = apply_agent_edit(
            &sm,
            &child,
            "parent-1",
            AgentEdit::Upsert(vec![
                agent_monitor("tests", "parent-1"),
                agent_monitor("safety", "parent-1"),
            ]),
        )
        .await
        .unwrap_err();
        assert!(error.contains("set by the person"), "{error}");
        assert_eq!(load(&sm, &child).await.unwrap(), original);

        // An entry not stamped as this caller's is refused.
        let error = apply_agent_edit(
            &sm,
            &child,
            "parent-1",
            AgentEdit::Upsert(vec![monitor("mine")]),
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("not marked as set by your chat's agent"),
            "{error}"
        );

        // Its own monitors it adds, replaces in place and removes.
        let added = apply_agent_edit(
            &sm,
            &child,
            "parent-1",
            AgentEdit::Upsert(vec![agent_monitor("tests", "parent-1")]),
        )
        .await
        .unwrap();
        assert_eq!(ids(&added), ["form-reached", "safety", "tests"]);
        let changed = apply_agent_edit(
            &sm,
            &child,
            "parent-1",
            AgentEdit::Upsert(vec![MonitorCriterion {
                threshold: 0.6,
                ..agent_monitor("tests", "parent-1")
            }]),
        )
        .await
        .unwrap();
        assert_eq!(changed.monitors[2].threshold, 0.6);
        // Another chat's agent cannot touch it.
        let error = apply_agent_edit(
            &sm,
            &child,
            "parent-2",
            AgentEdit::Remove(vec!["tests".to_string()]),
        )
        .await
        .unwrap_err();
        assert!(error.contains("set by another chat's agent"), "{error}");
        let error = apply_agent_edit(
            &sm,
            &child,
            "parent-1",
            AgentEdit::Remove(vec!["nope".to_string()]),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            "This subagent has no monitor `nope`. Nothing was changed."
        );
        let removed = apply_agent_edit(
            &sm,
            &child,
            "parent-1",
            AgentEdit::Remove(vec!["tests".to_string(), "tests".to_string()]),
        )
        .await
        .unwrap();
        assert_eq!(ids(&removed), ["form-reached", "safety"]);

        let after = load(&sm, &child).await.unwrap();
        assert_eq!(persons(&after), before, "a person's monitor changed");
        assert_eq!(after.default_model, original.default_model);
        assert_eq!(after, original);

        // A refusal in validation reads as a sentence that says so.
        let error = apply_agent_edit(
            &sm,
            &child,
            "parent-1",
            AgentEdit::Upsert(vec![MonitorCriterion {
                threshold: 0.0,
                ..agent_monitor("zero", "parent-1")
            }]),
        )
        .await
        .unwrap_err();
        assert!(error.ends_with(". Nothing was changed."), "{error}");
        assert!(apply_agent_edit(
            &sm,
            "no-such-chat",
            "parent-1",
            AgentEdit::Upsert(vec![agent_monitor("x", "parent-1")])
        )
        .await
        .unwrap_err()
        .ends_with("Nothing was changed."));
    }

    #[test]
    fn monitor_spec_watch_no_becomes_option_no_and_timing_names_map() {
        let spec: MonitorSpec = serde_json::from_value(serde_json::json!({
            "name": "Off task",
            "question": "Has the agent wandered off its task?",
            "watch": "no",
            "threshold": 0.8,
            "action": "stop",
            "timing": "turn_end"
        }))
        .unwrap();
        let none = BTreeSet::new();
        let c = spec.to_criterion("parent-1", &none).unwrap();
        assert_eq!(c.id, "off-task");
        assert_eq!(c.option.as_deref(), Some("no"));
        assert_eq!(c.pass_threshold().unwrap(), Threshold::No(0.8));
        assert_eq!(c.timing, MonitorTiming::TurnEnd);
        assert_eq!(c.action, MonitorAction::Stop);
        assert_eq!(
            c.on_error,
            MonitorErrorPolicy::Pause,
            "a stop pauses on errors"
        );
        assert_eq!(c.set_by, MonitorOwner::Agent);
        assert_eq!(c.set_by_session.as_deref(), Some("parent-1"));
        assert_eq!(c.consecutive, 1);
        assert_eq!(c.max_evals_per_turn, 50);
        assert_eq!(c.direction, MonitorDirection::AtOrAbove);
        assert!(c.enabled && c.model.is_none() && c.rearm_below.is_none());
        assert_eq!(c.question.kind, DecisionKind::Binary {});

        let parse = |value: serde_json::Value| -> MonitorSpec {
            let mut base = serde_json::json!({
                "name": "Tests pass", "question": "Do the tests pass?",
                "threshold": 0.9, "action": "steer"
            });
            for (key, v) in value.as_object().unwrap() {
                base[key] = v.clone();
            }
            serde_json::from_value(base).unwrap()
        };
        let steer = parse(serde_json::json!({}))
            .to_criterion("p", &none)
            .unwrap();
        assert_eq!(steer.option, None, "yes is the default");
        assert_eq!(steer.timing, MonitorTiming::PostIteration);
        assert_eq!(steer.on_error, MonitorErrorPolicy::Notify);
        for (timing, expected) in [
            ("each_step", MonitorTiming::PostIteration),
            ("post_iteration", MonitorTiming::PostIteration),
            ("turn_end", MonitorTiming::TurnEnd),
        ] {
            let c = parse(serde_json::json!({ "timing": timing }))
                .to_criterion("p", &none)
                .unwrap();
            assert_eq!(c.timing, expected, "{timing}");
        }
        let c = parse(serde_json::json!({
            "onError": "pause", "maxChecks": 10, "consecutive": 2,
            "model": "ollama_decision/tev1:0.8b", "id": "tests_pass"
        }))
        .to_criterion("p", &none)
        .unwrap();
        assert_eq!(c.on_error, MonitorErrorPolicy::Pause);
        assert_eq!(c.max_evals_per_turn, 10);
        assert_eq!(c.consecutive, 2);
        assert_eq!(c.id, "tests_pass");
        assert_eq!(
            c.model,
            Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b"))
        );
        let c = parse(serde_json::json!({"on_error": "notify", "max_checks": 7}))
            .to_criterion("p", &none)
            .unwrap();
        assert_eq!(c.on_error, MonitorErrorPolicy::Notify);
        assert_eq!(c.max_evals_per_turn, 7);

        // Ids are made unique against the ones taken.
        let taken: BTreeSet<String> = ["tests-pass".to_string(), "tests-pass-2".to_string()]
            .into_iter()
            .collect();
        assert_eq!(
            parse(serde_json::json!({}))
                .to_criterion("p", &taken)
                .unwrap()
                .id,
            "tests-pass-3"
        );
        let long_slug = monitor_slug(&"x".repeat(80));
        let long: BTreeSet<String> = [long_slug.clone()].into_iter().collect();
        let id = unique_monitor_id(&long_slug, &long);
        assert!(
            id.ends_with("-2") && id.chars().count() <= MAX_MONITOR_ID_CHARS,
            "{id}"
        );

        // What it refuses, with a sentence.
        let refusal = |value: serde_json::Value| parse(value).to_criterion("p", &none).unwrap_err();
        assert!(refusal(serde_json::json!({"model": "tev1"})).contains("provider/model"));
        assert!(refusal(serde_json::json!({"threshold": 0.0})).contains("above 0"));
        assert!(refusal(serde_json::json!({"consecutive": 9})).contains("1 to 5"));
        assert!(refusal(serde_json::json!({"maxChecks": 500})).contains("1 to 200"));
        assert!(refusal(serde_json::json!({"name": " "})).contains("needs a name"));
        assert!(refusal(serde_json::json!({"id": "Bad Id"})).contains("Monitor id"));
        for bad in [
            serde_json::json!({"name": "x", "question": "q?", "threshold": 0.5, "action": "notify"}),
            serde_json::json!({"name": "x", "question": "q?", "threshold": 0.5, "action": "stop", "treshold": 1}),
            serde_json::json!({"name": "x", "question": "q?", "threshold": 0.5, "action": "stop", "watch": "maybe"}),
            serde_json::json!({"name": "x", "question": "q?", "threshold": 0.5}),
        ] {
            assert!(
                serde_json::from_value::<MonitorSpec>(bad.clone()).is_err(),
                "{bad}"
            );
        }

        // The schema the agent reads names every field.
        let schema = serde_json::to_value(schemars::schema_for!(MonitorSpec)).unwrap();
        for field in [
            "name",
            "question",
            "watch",
            "threshold",
            "action",
            "timing",
            "on_error",
        ] {
            assert!(
                schema["properties"].get(field).is_some(),
                "{field} missing from {schema}"
            );
        }
    }

    #[test]
    fn child_settings_orders_inherited_workflow_phase_and_spawn_monitors_and_refuses_collisions() {
        let model_a = DecisionModelRef::new("ollama_decision", "tev1:0.8b");
        let model_b = DecisionModelRef::new("baseten_decision", "clef");
        let parent = DecisionSettings {
            default_model: Some(model_a.clone()),
            monitors: vec![
                for_subagents("p1"),
                monitor("p2"),
                MonitorCriterion {
                    enabled: false,
                    ..for_subagents("p3")
                },
            ],
        };
        let workflow = DecisionSettings {
            default_model: Some(model_b.clone()),
            monitors: vec![for_subagents("w1")],
        };
        let phase = vec![
            agent_monitor("ph1", "parent-1"),
            agent_monitor("shared", "parent-1"),
        ];
        let spawn = vec![
            MonitorCriterion {
                threshold: 0.55,
                ..agent_monitor("shared", "parent-1")
            },
            agent_monitor("sp1", "parent-1"),
        ];

        let child = child_settings(Some(&parent), "parent-1", Some(&workflow), &phase, &spawn)
            .unwrap()
            .unwrap();
        assert_eq!(child.default_model, Some(model_a));
        assert_eq!(ids(&child), ["p1", "w1", "ph1", "shared", "sp1"]);
        let p1 = &child.monitors[0];
        assert_eq!(p1.set_by, MonitorOwner::Person);
        assert_eq!(p1.inherited_from.as_deref(), Some("parent-1"));
        assert!(!p1.apply_to_subagents);
        assert_eq!(p1.question, parent.monitors[0].question);
        let w1 = &child.monitors[1];
        assert_eq!(
            (w1.set_by, w1.inherited_from.as_deref()),
            (MonitorOwner::Person, None)
        );
        assert!(!w1.apply_to_subagents);
        assert_eq!(child.monitors[3].threshold, 0.55, "the spawn's entry won");

        // Without a parent model, the sub-workflow's; with nothing, nothing.
        let child = child_settings(None, "parent-1", Some(&workflow), &[], &[])
            .unwrap()
            .unwrap();
        assert_eq!(child.default_model, Some(model_b));
        assert_eq!(
            child_settings(Some(&of(vec![monitor("p2")])), "parent-1", None, &[], &[]).unwrap(),
            None
        );
        assert_eq!(
            child_settings(None, "parent-1", None, &[], &[]).unwrap(),
            None
        );

        // Any other shared id is refused, naming it.
        let clash = |workflow: Option<&DecisionSettings>,
                     phase: &[MonitorCriterion],
                     spawn: &[MonitorCriterion]| {
            child_settings(Some(&parent), "parent-1", workflow, phase, spawn).unwrap_err()
        };
        let error = clash(Some(&of(vec![monitor("p1")])), &[], &[]);
        assert!(
            error.contains("`p1`")
                && error.contains("set to also watch its subagents")
                && error.contains("sub-workflow's"),
            "{error}"
        );
        let error = clash(None, &[], &[agent_monitor("p1", "parent-1")]);
        assert!(
            error.contains("`p1`") && error.contains("this spawn's"),
            "{error}"
        );
        let error = clash(
            None,
            &[
                agent_monitor("dup", "parent-1"),
                agent_monitor("dup", "parent-1"),
            ],
            &[],
        );
        assert!(
            error.contains("`dup`") && error.contains("phase's"),
            "{error}"
        );
        let error = clash(
            None,
            &[],
            &[
                agent_monitor("dup", "parent-1"),
                agent_monitor("dup", "parent-1"),
            ],
        );
        assert!(error.contains("`dup`"), "{error}");

        // The result is validated: five of the agent's are too many.
        let five: Vec<MonitorCriterion> = (0..5)
            .map(|i| agent_monitor(&format!("a{i}"), "parent-1"))
            .collect();
        assert!(clash(None, &five, &[]).contains("at most 4 monitors set by an agent"));
    }

    #[tokio::test]
    async fn sync_inherited_replaces_only_this_parents_copies_and_drops_agent_monitors_for_room() {
        let (dir, sm, parent) = store().await;
        let mut children = Vec::new();
        for _ in 0..4 {
            children.push(
                sm.create_session(dir.path().into(), "child".into(), SessionType::User)
                    .await
                    .unwrap()
                    .id,
            );
        }
        let (crowded, clashing, own_clash, unreadable) = (
            children[0].clone(),
            children[1].clone(),
            children[2].clone(),
            children[3].clone(),
        );

        let old_copy = |id: &str| MonitorCriterion {
            name: format!("Person {id}"),
            inherited_from: Some(parent.clone()),
            ..monitor(id)
        };
        let named_agent = |id: &str| MonitorCriterion {
            name: format!("Agent {}", id.to_uppercase()),
            ..agent_monitor(id, &parent)
        };
        let from_other = MonitorCriterion {
            inherited_from: Some("another-parent".to_string()),
            ..monitor("q1")
        };
        let crowded_before = DecisionSettings {
            default_model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            monitors: vec![
                old_copy("s1"),
                from_other.clone(),
                monitor("own1"),
                monitor("own2"),
                named_agent("a1"),
                named_agent("a2"),
                named_agent("a3"),
                named_agent("a4"),
            ],
        };
        save(&sm, &crowded, crowded_before.clone()).await.unwrap();
        save(
            &sm,
            &clashing,
            of(vec![named_agent("s2"), named_agent("a1")]),
        )
        .await
        .unwrap();
        save(&sm, &own_clash, of(vec![monitor("s3")]))
            .await
            .unwrap();
        sm.update_extension_state(
            &unreadable,
            DECISION_SETTINGS_STATE,
            DECISION_SETTINGS_VERSION,
            |_| Ok(serde_json::json!({"monitors": "not a list"})),
        )
        .await
        .unwrap();

        let saved = DecisionSettings {
            default_model: None,
            monitors: vec![
                MonitorCriterion {
                    threshold: 0.95,
                    ..for_subagents("s1")
                },
                for_subagents("s2"),
                for_subagents("s3"),
                monitor("parent-only"),
            ],
        };
        let reports = sync_inherited(&sm, &parent, &saved, &children).await;
        assert_eq!(reports.len(), 4);

        // The crowded child: fresh copies first, the other chat's copy, the
        // person's own and the agent's kept, the newest agent monitors dropped.
        assert_eq!(
            reports[0],
            SyncReport {
                child_id: crowded.clone(),
                dropped: vec!["a4".to_string(), "a3".to_string()],
                error: None,
            }
        );
        let after = load(&sm, &crowded).await.unwrap();
        assert_eq!(
            ids(&after),
            ["s1", "s2", "s3", "q1", "own1", "own2", "a1", "a2"]
        );
        assert_eq!(after.monitors[0].threshold, 0.95, "s1 was refreshed");
        assert_eq!(
            after.monitors[0].inherited_from.as_deref(),
            Some(parent.as_str())
        );
        assert!(!after.monitors[0].apply_to_subagents);
        assert_eq!(after.monitors[3], from_other, "another chat's copy changed");
        assert_eq!(after.default_model, crowded_before.default_model);
        let notices: Vec<(String, bool)> = sm
            .get_session(&crowded, true)
            .await
            .unwrap()
            .conversation
            .unwrap()
            .messages()
            .iter()
            .flat_map(|message| {
                message
                    .content
                    .iter()
                    .filter_map(|content| match content {
                        MessageContent::SystemNotification(n) => {
                            Some((n.msg.clone(), message.is_agent_visible()))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(
            notices,
            [
                (
                    "Monitor 'Agent A4', set by the agent, was removed to make room for the \
                     person's monitors."
                        .to_string(),
                    false
                ),
                (
                    "Monitor 'Agent A3', set by the agent, was removed to make room for the \
                     person's monitors."
                        .to_string(),
                    false
                ),
            ]
        );

        // An agent monitor with a copy's id gives way to the person's.
        assert_eq!(reports[1].dropped, ["s2"]);
        assert_eq!(
            ids(&load(&sm, &clashing).await.unwrap()),
            ["s1", "s2", "s3", "a1"]
        );
        // A person's own monitor with a copy's id is not overwritten.
        assert!(reports[2]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("`s3`")));
        assert_eq!(ids(&load(&sm, &own_clash).await.unwrap()), ["s3"]);
        // An unreadable record is left as it is.
        assert!(reports[3].error.is_some());
        assert!(load(&sm, &unreadable).await.is_err());

        // Turning the parent's monitors off for subagents removes the copies.
        let reports = sync_inherited(
            &sm,
            &parent,
            &of(Vec::new()),
            std::slice::from_ref(&crowded),
        )
        .await;
        assert_eq!(reports[0].error, None);
        assert_eq!(
            ids(&load(&sm, &crowded).await.unwrap()),
            ["q1", "own1", "own2", "a1", "a2"]
        );
    }

    /// R-30: a sync that changes nothing in a child does not write it, so the
    /// child's activity time stays where it was.
    #[tokio::test]
    async fn a_sync_that_changes_nothing_writes_nothing() {
        let (dir, sm, parent) = store().await;
        let child = sm
            .create_session(dir.path().into(), "child".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let saved = of(vec![for_subagents("s1")]);
        sync_inherited(&sm, &parent, &saved, std::slice::from_ref(&child)).await;
        let first = sm.get_session(&child, false).await.unwrap().updated_at;
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        let reports = sync_inherited(&sm, &parent, &saved, std::slice::from_ref(&child)).await;
        assert_eq!(reports[0].error, None);
        assert_eq!(
            sm.get_session(&child, false).await.unwrap().updated_at,
            first,
            "an unchanged child was written"
        );
        // A child with nothing from this parent and nothing to get is not
        // written either.
        let other = sm
            .create_session(dir.path().into(), "other".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        let before = sm.get_session(&other, false).await.unwrap().updated_at;
        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        sync_inherited(&sm, &parent, &of(Vec::new()), std::slice::from_ref(&other)).await;
        assert_eq!(
            sm.get_session(&other, false).await.unwrap().updated_at,
            before
        );
        assert_eq!(
            sm.get_extension_state(&other, DECISION_SETTINGS_STATE, DECISION_SETTINGS_VERSION)
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn update_is_one_transaction() {
        let (_dir, sm, id) = store().await;
        let mut writers = Vec::new();
        for i in 0..MAX_MONITORS {
            let sm = Arc::clone(&sm);
            let id = id.clone();
            writers.push(tokio::spawn(async move {
                update(&sm, &id, move |current| {
                    let mut settings = current.unwrap_or_default();
                    settings.monitors.push(monitor(&format!("m{i}")));
                    Ok(settings)
                })
                .await
            }));
        }
        for writer in writers {
            writer.await.unwrap().unwrap();
        }
        let stored = load(&sm, &id).await.unwrap();
        let mut seen: Vec<&str> = ids(&stored);
        seen.sort_unstable();
        assert_eq!(
            seen,
            ["m0", "m1", "m2", "m3", "m4", "m5", "m6", "m7"],
            "an update was lost"
        );

        // A refusal, from `mutate` or from validation, writes nothing.
        let error = update(&sm, &id, |_| Err("Not today.".to_string()))
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<InvalidDecisionSettings>(),
            Some(&InvalidDecisionSettings("Not today.".to_string()))
        );
        let error = update(&sm, &id, |current| {
            let mut settings = current.unwrap();
            settings.monitors[0].threshold = 0.0;
            Ok(settings)
        })
        .await
        .unwrap_err();
        assert!(error.downcast_ref::<InvalidDecisionSettings>().is_some());
        assert_eq!(load(&sm, &id).await.unwrap(), stored);
        assert!(
            update(&sm, "no-such-chat", |_| Ok(DecisionSettings::default()))
                .await
                .is_err()
        );

        // A record this build cannot read is never handed over as "none";
        // a person's save still replaces it.
        sm.update_extension_state(
            &id,
            DECISION_SETTINGS_STATE,
            DECISION_SETTINGS_VERSION,
            |_| Ok(serde_json::json!({"monitors": [{"id": "m", "future": true}]})),
        )
        .await
        .unwrap();
        let error = update(&sm, &id, |_| -> Result<DecisionSettings, String> {
            panic!("an unreadable record reached mutate")
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("could not be read"), "{error}");
        let replacement = of(vec![monitor("fresh")]);
        save(&sm, &id, replacement.clone()).await.unwrap();
        assert_eq!(load(&sm, &id).await.unwrap(), replacement);
    }

    #[tokio::test]
    async fn the_same_stored_text_is_not_parsed_again() {
        let (_dir, sm, id) = store().await;
        assert_eq!(
            read_for_monitoring(&sm, &id, None).await,
            SettingsRead::Absent
        );
        assert_eq!(
            read_for_monitoring(&sm, "no-such-chat", Some("{}")).await,
            SettingsRead::Absent
        );

        let settings = of(vec![monitor("form-reached")]);
        save(&sm, &id, settings.clone()).await.unwrap();
        let SettingsRead::Read {
            settings: read,
            text,
        } = read_for_monitoring(&sm, &id, None).await
        else {
            panic!("the saved settings were not read");
        };
        assert_eq!(read, settings);
        assert_eq!(
            read_for_monitoring(&sm, &id, Some(&text)).await,
            SettingsRead::Unchanged
        );

        // Text it has seen is not parsed again, even text it could not read:
        // the engine keeps what it decided about it.
        sm.update_extension_state(
            &id,
            DECISION_SETTINGS_STATE,
            DECISION_SETTINGS_VERSION,
            |_| Ok(serde_json::json!({"monitors": "not a list"})),
        )
        .await
        .unwrap();
        let raw = sm
            .get_extension_state_json(&id, DECISION_SETTINGS_STATE, DECISION_SETTINGS_VERSION)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            read_for_monitoring(&sm, &id, Some(&text)).await,
            SettingsRead::Unreadable { .. }
        ));
        assert_eq!(
            read_for_monitoring(&sm, &id, Some(&raw)).await,
            SettingsRead::Unchanged
        );

        // A new save is new text.
        let changed = of(vec![monitor("other")]);
        save(&sm, &id, changed.clone()).await.unwrap();
        match read_for_monitoring(&sm, &id, Some(&text)).await {
            SettingsRead::Read {
                settings,
                text: new,
            } => {
                assert_eq!(settings, changed);
                assert_ne!(new, text);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn the_pause_scan_finds_pause_in_a_record_this_build_cannot_read() {
        let (_dir, sm, id) = store().await;
        let read_raw = |value: serde_json::Value| {
            let sm = Arc::clone(&sm);
            let id = id.clone();
            async move {
                sm.update_extension_state(
                    &id,
                    DECISION_SETTINGS_STATE,
                    DECISION_SETTINGS_VERSION,
                    move |_| Ok(value),
                )
                .await
                .unwrap();
                read_for_monitoring(&sm, &id, None).await
            }
        };
        let pause_named = |read: SettingsRead| match read {
            SettingsRead::Unreadable { pause_named, error } => {
                assert!(!error.is_empty());
                pause_named
            }
            other => panic!("read as {other:?}"),
        };

        // A key this build does not know, beside a monitor set to pause.
        let future = |extra: serde_json::Value| {
            let mut m = serde_json::to_value(MonitorCriterion {
                on_error: MonitorErrorPolicy::Pause,
                ..monitor("m")
            })
            .unwrap();
            m["futureKey"] = serde_json::json!(true);
            for (key, value) in extra.as_object().unwrap() {
                m[key] = value.clone();
            }
            serde_json::json!({"defaultModel": null, "monitors": [monitor("quiet"), m]})
        };
        assert!(pause_named(read_raw(future(serde_json::json!({}))).await));
        assert!(pause_named(
            read_raw(future(serde_json::json!({"enabled": true}))).await
        ));
        assert!(!pause_named(
            read_raw(future(serde_json::json!({"enabled": false}))).await
        ));
        assert!(!pause_named(
            read_raw(future(serde_json::json!({"onError": "notify"}))).await
        ));
        assert!(!pause_named(
            read_raw(serde_json::json!({"monitors": "not a list"})).await
        ));
        assert!(!pause_named(read_raw(serde_json::json!(["pause"])).await));
        assert!(!pause_named_in("not json"));
    }

    #[test]
    fn a_stop_report_carries_the_owner_and_a_clipped_question() {
        let criterion = MonitorCriterion {
            question: Question {
                instructions: "q".repeat(400),
                ..binary()
            },
            direction: MonitorDirection::Below,
            threshold: 0.3,
            rearm_below: None,
            ..agent_monitor("off-task", "parent-1")
        };
        let report = MonitorStopReport::new(
            "child-1",
            &criterion,
            Some(0.1),
            MonitorStopReason::CouldNotCheck {
                error: "timed out".to_string(),
            },
        );
        assert_eq!(
            report.question.chars().count(),
            MAX_STOP_REPORT_QUESTION_CHARS
        );
        assert!(report.question.ends_with('…'));
        assert!(report.below);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["set_by"], "agent");
        assert_eq!(json["set_by_session"], "parent-1");
        assert_eq!(json["monitor_id"], "off-task");
        assert_eq!(
            json["reason"],
            serde_json::json!({"kind": "could_not_check", "error": "timed out"})
        );
        let back: MonitorStopReport = serde_json::from_value(json).unwrap();
        assert_eq!(back, report);
        for (reason, kind) in [
            (MonitorStopReason::Fired, "fired"),
            (MonitorStopReason::CheckLimit { limit: 50 }, "check_limit"),
            (
                MonitorStopReason::ChildStopped {
                    child_session_id: "c".into(),
                    child_title: "t".into(),
                },
                "child_stopped",
            ),
            (
                MonitorStopReason::Unreadable { error: "e".into() },
                "unreadable",
            ),
        ] {
            assert_eq!(serde_json::to_value(reason).unwrap()["kind"], kind);
        }
        assert_eq!(clipped_question("  short  "), "short");
    }

    #[test]
    fn the_firing_rule_follows_the_question_kind() {
        let mut m = monitor("m");
        assert_eq!(m.pass_threshold().unwrap(), Threshold::Yes(0.85));
        m.option = Some("no".to_string());
        assert_eq!(m.pass_threshold().unwrap(), Threshold::No(0.85));

        let mut options = IndexMap::new();
        options.insert("rrms".to_string(), Some("Relapsing-remitting".to_string()));
        options.insert("spms".to_string(), None);
        m.question.kind = DecisionKind::Choice { options };
        m.option = Some("spms".to_string());
        assert_eq!(
            m.pass_threshold().unwrap(),
            Threshold::Option {
                name: "spms".to_string(),
                p: 0.85
            }
        );
        m.option = None;
        assert!(m.pass_threshold().unwrap_err().contains("name the option"));
        m.option = Some("ppms".to_string());
        assert!(validate(&DecisionSettings {
            default_model: None,
            monitors: vec![m.clone()]
        })
        .unwrap_err()
        .contains("not one of its question's options"));

        m.question.kind = DecisionKind::Score {
            levels: vec!["none".into(), "mild".into(), "severe".into()],
        };
        m.option = Some("mild".to_string());
        assert_eq!(
            m.pass_threshold().unwrap(),
            Threshold::AtLeastLevel { level: 1, p: 0.85 }
        );
        m.option = Some("2".to_string());
        assert_eq!(
            m.pass_threshold().unwrap(),
            Threshold::AtLeastLevel { level: 2, p: 0.85 }
        );
        m.option = Some("7".to_string());
        assert!(m.pass_threshold().is_err());

        // The question it sends carries that rule, and a matching own
        // threshold is accepted.
        m.option = Some("mild".to_string());
        let sent = m.question_with_threshold().unwrap();
        assert_eq!(
            sent.threshold,
            Some(Threshold::AtLeastLevel { level: 1, p: 0.85 })
        );
        m.question = sent;
        validate(&DecisionSettings {
            default_model: None,
            monitors: vec![m],
        })
        .unwrap();
    }

    async fn store() -> (TempDir, Arc<SessionManager>, String) {
        let dir = TempDir::new().unwrap();
        let sm = Arc::new(SessionManager::new(dir.path().to_path_buf()));
        let id = sm
            .create_session(dir.path().into(), "chat".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        (dir, sm, id)
    }

    #[tokio::test]
    async fn settings_round_trip_and_an_invalid_save_writes_nothing() {
        let (_dir, sm, id) = store().await;
        assert_eq!(load(&sm, &id).await.unwrap(), DecisionSettings::default());
        assert_eq!(
            load(&sm, "no-such-chat").await.unwrap(),
            DecisionSettings::default()
        );

        let settings = DecisionSettings {
            default_model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            monitors: vec![monitor("form-reached")],
        };
        assert_eq!(save(&sm, &id, settings.clone()).await.unwrap(), settings);
        assert_eq!(load(&sm, &id).await.unwrap(), settings);

        let error = save(&sm, &id, with(|m| m.threshold = 2.0))
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<InvalidDecisionSettings>().is_some());
        assert_eq!(
            load(&sm, &id).await.unwrap(),
            settings,
            "a refused save wrote"
        );

        assert!(save(&sm, "no-such-chat", settings.clone()).await.is_err());

        let chat = DecisionChatSettings { question: binary() };
        assert_eq!(load_chat(&sm, &id).await.unwrap(), None);
        assert_eq!(save_chat(&sm, &id, chat.clone()).await.unwrap(), chat);
        assert_eq!(load_chat(&sm, &id).await.unwrap(), Some(chat));
        let bad = DecisionChatSettings {
            question: Question {
                kind: DecisionKind::Score { levels: Vec::new() },
                ..binary()
            },
        };
        let error = save_chat(&sm, &id, bad).await.unwrap_err();
        assert!(error.downcast_ref::<InvalidDecisionSettings>().is_some());
        // The two records are separate keys: neither save touched the other.
        assert_eq!(load(&sm, &id).await.unwrap(), settings);
    }

    #[tokio::test]
    async fn an_unreadable_stored_value_is_an_error_not_an_empty_form() {
        let (_dir, sm, id) = store().await;
        sm.update_extension_state(
            &id,
            DECISION_SETTINGS_STATE,
            DECISION_SETTINGS_VERSION,
            |_| Ok(serde_json::json!({"monitors": "not a list"})),
        )
        .await
        .unwrap();
        assert!(load(&sm, &id).await.is_err());
    }

    /// The per-key write: saving decision settings while another key of the
    /// same `extension_data` is written at the same moment loses neither.
    #[tokio::test]
    async fn a_save_never_clobbers_another_key() {
        let (_dir, sm, id) = store().await;
        let settings = DecisionSettings {
            default_model: None,
            monitors: vec![monitor("form-reached")],
        };
        let mut writers = Vec::new();
        for round in 0..20 {
            let sm = Arc::clone(&sm);
            let id = id.clone();
            let settings = settings.clone();
            writers.push(tokio::spawn(async move {
                if round % 2 == 0 {
                    save(&sm, &id, settings).await.map(|_| ())
                } else {
                    sm.update_extension_state(&id, "todo", "v0", move |current| {
                        let n = current.and_then(|v| v.as_u64()).unwrap_or(0);
                        Ok(serde_json::json!(n + 1))
                    })
                    .await
                    .map(|_| ())
                }
            }));
        }
        for writer in writers {
            writer.await.unwrap().unwrap();
        }
        assert_eq!(load(&sm, &id).await.unwrap(), settings);
        assert_eq!(
            sm.get_extension_state(&id, "todo", "v0").await.unwrap(),
            Some(serde_json::json!(10)),
            "a decision-settings save erased a concurrent write to another key"
        );
    }
}
