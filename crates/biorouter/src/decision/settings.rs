//! Per-chat decision settings: the chat's default decision model, its monitors,
//! and, for a chat whose main model is a decision model, the question it asks.
//!
//! Stored in the chat's `extension_data` under [`DECISION_SETTINGS_STATE`] and
//! [`DECISION_CHAT_STATE`], written only through `update_extension_state` (one key
//! per transaction), so a save here can never erase a to-do list, a goal or an
//! orchestration plan written at the same moment, which a read-modify-write of
//! the whole `extension_data` blob would.
//!
//! Both records name where a chat's content goes (a monitor's model reads a
//! window of the conversation), so a route that writes them needs the user's
//! proof; that check belongs to the route, not here. [`save`] and
//! [`save_chat`] validate before they write and refuse with the sentence a
//! person reads.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::types::{
    validate_questions, DecisionKind, DecisionModelRef, Question, Threshold, NO, YES,
};
use crate::session::SessionManager;

pub const DECISION_SETTINGS_STATE: &str = "decision_settings";
pub const DECISION_SETTINGS_VERSION: &str = "v1";
pub const DECISION_CHAT_STATE: &str = "decision_chat";
pub const DECISION_CHAT_VERSION: &str = "v1";

/// At most this many monitors per chat.
pub const MAX_MONITORS: usize = 8;
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

/// The rolling window a monitor sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
    /// Fires when P(wanted answer) is at least this (0..1).
    pub threshold: f64,
    /// For a choice question: the option whose probability is compared. Binary
    /// questions compare P(yes).
    pub option: Option<String>,
    /// Evaluations in a row at or above the threshold before it fires (1..5).
    #[serde(default = "one")]
    pub consecutive: u32,
    /// After firing, re-arm only once P drops below this.
    pub rearm_below: Option<f64>,
    #[serde(default)]
    pub action: MonitorAction,
    #[serde(default)]
    pub on_error: MonitorErrorPolicy,
    /// Evaluation cap per turn (at most 200).
    #[serde(default = "default_max_evals")]
    pub max_evals_per_turn: u32,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn one() -> u32 {
    1
}
fn default_max_evals() -> u32 {
    50
}
fn yes() -> bool {
    true
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
#[serde(rename_all = "camelCase")]
pub struct DecisionSettings {
    pub default_model: Option<DecisionModelRef>,
    #[serde(default)]
    pub monitors: Vec<MonitorCriterion>,
}

/// The question a decision chat asks of each message the person sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
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
    if !probability(monitor.threshold) {
        return Err(format!(
            "Monitor \"{name}\": the threshold must be a probability between 0 and 1."
        ));
    }
    let threshold = monitor.pass_threshold()?;
    if let Some(own) = &monitor.question.threshold {
        if *own != threshold {
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
    if let Some(rearm) = monitor.rearm_below {
        if !probability(rearm) || rearm > monitor.threshold {
            return Err(format!(
                "Monitor \"{name}\": it re-arms below a probability between 0 and its threshold \
                 ({}), not {rearm}.",
                monitor.threshold
            ));
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

/// Validate and store. Returns the stored value.
pub async fn save(
    session_manager: &SessionManager,
    session_id: &str,
    settings: DecisionSettings,
) -> anyhow::Result<DecisionSettings> {
    validate(&settings).map_err(InvalidDecisionSettings)?;
    let value = serde_json::to_value(&settings)?;
    let stored = session_manager
        .update_extension_state(
            session_id,
            DECISION_SETTINGS_STATE,
            DECISION_SETTINGS_VERSION,
            move |_| Ok(value),
        )
        .await?;
    anyhow::ensure!(stored.is_some(), "No chat {session_id} exists.");
    Ok(settings)
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

#[cfg(test)]
mod tests {
    use super::*;
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
            option: None,
            consecutive: 1,
            rearm_below: Some(0.7),
            action: MonitorAction::Stop,
            on_error: MonitorErrorPolicy::Notify,
            max_evals_per_turn: 50,
            enabled: true,
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
