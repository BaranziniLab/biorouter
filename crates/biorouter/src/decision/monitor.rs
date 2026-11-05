//! Monitors: decision models watching a chat while its agent works, and the
//! per-turn tally the transcript marker is drawn from.
//!
//! A monitor ([`MonitorCriterion`], stored in the chat's `decision_settings.v1`)
//! asks one question about a rolling window of the conversation
//! ([`build_window`]) at one point of the reply loop, and fires when the
//! probability of the answer it watches for reaches its threshold. Firing stops
//! the turn, shows a notice, or tells the model (design §4).
//!
//! The parts, each kept apart so the loop pays only for what it uses:
//!
//! - [`CriterionState`]: the pure state machine for one criterion (threshold,
//!   consecutive evaluations, the latch and its re-arm level, the error streak,
//!   the per-turn evaluation cap). No I/O; unit tested on its own.
//! - [`MonitorRegistry`]: every chat's criterion states and its settings, cached
//!   once per turn, held on the `Agent` (never as a local of the reply loop's
//!   generator, whose `poll` frame is close to the stack cliff).
//!   [`MonitorRegistry::any_due`] is the synchronous check the loop asks first,
//!   so a chat with no monitors makes no request and awaits nothing.
//! - [`MonitorRegistry::plan`] / [`ask_planned`] / [`MonitorRegistry::apply`]:
//!   one evaluation point, split so the agent can race the only await (the
//!   decision calls) against a steer and a cancel. All due criteria that share a
//!   model and a window go in ONE multi-question request over one state, and a
//!   criterion whose window has not changed since its last evaluation is not
//!   asked again.
//! - The turn tally ([`note_decision`], [`take_turn_tally`]): every decision the
//!   service records with a turn id is counted per origin, in process, so the
//!   one transcript marker a turn gets can say what decision models did without
//!   reading the decision log. Owner rule A1: decision calls can number in the
//!   thousands, so nothing here is ever drawn one row per call.
//!
//! This builds on Albert Lee's prototype in PR #469 (Baseten System One exit
//! criteria), which first evaluated a decision model on the reply loop's
//! progress tail; the window, the threshold rule at equality and the
//! fail-closed option carry over from it.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError, RwLock};

use indexmap::IndexMap;

use super::error::{DecisionError, DecisionResult};
use super::log::{DecisionTurnMarker, DecisionTurnMarkerItem};
use super::service::{DecisionLogEntry, DecisionOutcome, DecisionService, DecisionSession};
use super::settings::{
    DecisionSettings, MonitorAction, MonitorCriterion, MonitorErrorPolicy, MonitorTiming,
    MonitorWindow,
};
use super::types::{Decision, DecisionModelRef, DecisionOrigin, DecisionRequest, Question};
use super::window::{build_window, WindowSpec};
use crate::conversation::Conversation;

/// Errors in a row after which a criterion is off for the rest of the turn.
pub const ERRORS_BEFORE_OFF: u32 = 3;
/// The evaluation cap a criterion can ask for, whatever it stores.
pub const MAX_EVALS_PER_TURN: u32 = 200;
/// A probability this close below a threshold still counts as at it, the same
/// tolerance `Threshold::passes` uses.
const THRESHOLD_EPSILON: f64 = 1e-9;

/// What one evaluation of one criterion came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Observation {
    /// The probability of the answer the criterion watches for.
    Probability(f64),
    /// The decision call failed. `refused` is a privacy or workspace refusal,
    /// which will not change within the turn.
    Error { message: String, refused: bool },
}

/// What the state machine asks the loop to do after one observation.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Quiet,
    /// The criterion fired: P reached the threshold for `consecutive`
    /// evaluations in a row.
    Fired {
        action: MonitorAction,
        p: f64,
    },
    /// It would have stopped the turn, but a message the person typed is
    /// waiting; the person's message wins and the criterion stays armed.
    Suppressed {
        p: Option<f64>,
    },
    /// The call failed and the criterion fails closed (`onError: pause`).
    FailedClosed {
        error: String,
    },
    /// The call failed; show this once. `off`: the criterion is off for the
    /// rest of the turn (a refusal, or the third error in a row).
    ErrorNotice {
        error: String,
        off: bool,
    },
}

/// One criterion's counters within the current turn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CriterionTurn {
    pub evaluations: u32,
    pub at_or_above: u32,
    pub below: u32,
    pub errors: u32,
    pub fired: bool,
    pub disabled: bool,
    error_notice_shown: bool,
}

/// One criterion's state. The latch and the consecutive count survive turns
/// (a monitor that stopped the turn on "form reached" must not stop the next
/// turn on the same page); [`CriterionTurn`] and the error streak do not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CriterionState {
    pub consecutive: u32,
    pub latched: bool,
    pub error_streak: u32,
    pub last_p: Option<f64>,
    /// The window hash the criterion was last asked about.
    pub last_window: Option<String>,
    pub turn: CriterionTurn,
}

impl CriterionState {
    pub fn begin_turn(&mut self) {
        self.turn = CriterionTurn::default();
        self.error_streak = 0;
    }

    /// Whether the criterion is evaluated at `timing` now.
    pub fn is_due(&self, criterion: &MonitorCriterion, timing: MonitorTiming) -> bool {
        criterion.enabled
            && criterion.timing == timing
            && !self.turn.disabled
            && self.turn.evaluations < criterion.max_evals_per_turn.min(MAX_EVALS_PER_TURN)
    }

    /// Fold one evaluation in. `steer_waiting`: a message the person typed is
    /// queued, so a stop (including a fail-closed one) is held back.
    pub fn observe(
        &mut self,
        criterion: &MonitorCriterion,
        observation: Observation,
        steer_waiting: bool,
    ) -> Step {
        match observation {
            Observation::Probability(p) => self.observe_probability(criterion, p, steer_waiting),
            Observation::Error { message, refused } => {
                self.turn.evaluations += 1;
                self.turn.errors += 1;
                self.error_streak += 1;
                if criterion.on_error == MonitorErrorPolicy::Pause {
                    if steer_waiting {
                        return Step::Suppressed { p: None };
                    }
                    self.turn.fired = true;
                    return Step::FailedClosed { error: message };
                }
                let off = refused || self.error_streak >= ERRORS_BEFORE_OFF;
                if off {
                    self.turn.disabled = true;
                }
                if self.turn.error_notice_shown {
                    return Step::Quiet;
                }
                self.turn.error_notice_shown = true;
                Step::ErrorNotice {
                    error: message,
                    off,
                }
            }
        }
    }

    fn observe_probability(
        &mut self,
        criterion: &MonitorCriterion,
        p: f64,
        steer_waiting: bool,
    ) -> Step {
        self.turn.evaluations += 1;
        self.error_streak = 0;
        self.last_p = Some(p);
        let at = p + THRESHOLD_EPSILON >= criterion.threshold;
        if at {
            self.turn.at_or_above += 1;
        } else {
            self.turn.below += 1;
        }
        if self.latched {
            // Hysteresis: a fired criterion re-arms only once P falls below
            // `rearmBelow` (or below its threshold, when none is set).
            let rearm = criterion.rearm_below.unwrap_or(criterion.threshold);
            if p < rearm {
                self.latched = false;
                self.consecutive = 0;
            } else {
                return Step::Quiet;
            }
        }
        if !at {
            self.consecutive = 0;
            return Step::Quiet;
        }
        let needed = criterion.consecutive.clamp(1, 5);
        self.consecutive = (self.consecutive + 1).min(needed);
        if self.consecutive < needed {
            return Step::Quiet;
        }
        if steer_waiting && criterion.action == MonitorAction::Stop {
            // Stays at the threshold count, so the next evaluation fires if
            // P is still there once the person's message has been answered.
            return Step::Suppressed { p: Some(p) };
        }
        self.latched = true;
        self.consecutive = 0;
        self.turn.fired = true;
        Step::Fired {
            action: criterion.action,
            p,
        }
    }
}

/// The question a criterion sends: its own, carrying the monitor's firing
/// rule as a threshold ([`MonitorCriterion::pass_threshold`]), so the log
/// records pass and fail and the service computes the probability watched.
fn monitor_question(criterion: &MonitorCriterion) -> Result<Question, String> {
    criterion.question_with_threshold()
}

/// The model a criterion asks: its own, else the chat's default, else `None`
/// (the service chooses, local first, within the chat's tier).
fn model_for(
    criterion: &MonitorCriterion,
    settings: &DecisionSettings,
) -> Option<DecisionModelRef> {
    criterion
        .model
        .clone()
        .or_else(|| settings.default_model.clone())
}

/// A window's grouping key: criteria with equal windows share one state.
type WindowKey = (u32, u32, bool);

fn window_key(window: MonitorWindow) -> WindowKey {
    (window.entries, window.max_chars, window.tool_calls)
}

/// One request an evaluation point will make: every due criterion that shares
/// a model and a window, over one state.
#[derive(Debug, Clone)]
pub struct PlannedRequest {
    pub model: Option<DecisionModelRef>,
    /// The criteria asked, in settings order: (id, name).
    pub criteria: Vec<(String, String)>,
    pub request: DecisionRequest,
    pub window_sha: String,
}

impl PlannedRequest {
    /// The log's origin id: the criterion's id, or the ids joined with `+`
    /// when one request carries several criteria.
    pub fn origin_id(&self) -> String {
        self.criteria
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>()
            .join("+")
    }

    pub fn label(&self) -> String {
        self.criteria
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

/// What one evaluation point decided, per criterion that has something to say.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorEvent {
    pub criterion_id: String,
    pub name: String,
    pub question: String,
    pub threshold: f64,
    pub step: Step,
}

/// The outcome of one planned request: the decisions, or why there were none.
pub type AskOutcome = DecisionResult<Vec<Decision>>;

#[derive(Debug, Default)]
struct SessionMonitors {
    turn_id: Option<String>,
    /// Loaded once per turn; `None` until then.
    settings: Option<Arc<DecisionSettings>>,
    criteria: HashMap<String, CriterionState>,
    /// Criteria a plan could not build a question for this turn, with why.
    invalid: Vec<(String, String)>,
}

/// Every chat's monitor state on one agent. Held on the `Agent`.
#[derive(Default)]
pub struct MonitorRegistry {
    sessions: Mutex<HashMap<String, SessionMonitors>>,
}

impl MonitorRegistry {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, SessionMonitors>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A reply started: remember its turn id, forget the settings cached for
    /// the last turn, and reset every criterion's per-turn counters.
    pub fn begin_turn(&self, session_id: &str, turn_id: &str) {
        let mut sessions = self.lock();
        let entry = sessions.entry(session_id.to_string()).or_default();
        entry.turn_id = Some(turn_id.to_string());
        entry.settings = None;
        entry.invalid.clear();
        for state in entry.criteria.values_mut() {
            state.begin_turn();
        }
    }

    /// The reply ended. Keeps the criteria (their latches outlive the turn).
    pub fn end_turn(&self, session_id: &str, turn_id: &str) {
        let mut sessions = self.lock();
        if let Some(entry) = sessions.get_mut(session_id) {
            if entry.turn_id.as_deref() == Some(turn_id) {
                entry.turn_id = None;
                entry.settings = None;
            }
        }
    }

    pub fn turn_id(&self, session_id: &str) -> Option<String> {
        self.lock()
            .get(session_id)
            .and_then(|entry| entry.turn_id.clone())
    }

    /// Cache the chat's settings for this turn, dropping the state of any
    /// criterion that no longer exists.
    pub fn cache_settings(&self, session_id: &str, settings: DecisionSettings) {
        let mut sessions = self.lock();
        let entry = sessions.entry(session_id.to_string()).or_default();
        entry
            .criteria
            .retain(|id, _| settings.monitors.iter().any(|m| &m.id == id));
        entry.settings = Some(Arc::new(settings));
    }

    pub fn settings(&self, session_id: &str) -> Option<Arc<DecisionSettings>> {
        self.lock()
            .get(session_id)
            .and_then(|entry| entry.settings.clone())
    }

    /// Whether any criterion is due at `timing`. Synchronous and cheap: the
    /// loop asks this before it awaits anything.
    pub fn any_due(&self, session_id: &str, timing: MonitorTiming) -> bool {
        let sessions = self.lock();
        let Some(entry) = sessions.get(session_id) else {
            return false;
        };
        let Some(settings) = entry.settings.as_ref() else {
            return false;
        };
        settings.monitors.iter().any(|criterion| {
            entry
                .criteria
                .get(&criterion.id)
                .map(|state| state.is_due(criterion, timing))
                .unwrap_or_else(|| CriterionState::default().is_due(criterion, timing))
        })
    }

    /// The requests one evaluation point makes, grouped by (model, window).
    /// A criterion whose window is unchanged since it was last asked is left
    /// out; a criterion whose question cannot be built is reported by
    /// [`Self::take_invalid`] and turned off for the turn.
    pub fn plan(
        &self,
        session_id: &str,
        timing: MonitorTiming,
        conversation: &Conversation,
        iteration: u32,
    ) -> Vec<PlannedRequest> {
        let mut sessions = self.lock();
        let Some(entry) = sessions.get_mut(session_id) else {
            return Vec::new();
        };
        let Some(settings) = entry.settings.clone() else {
            return Vec::new();
        };
        let mut groups: IndexMap<(Option<DecisionModelRef>, WindowKey), Vec<&MonitorCriterion>> =
            IndexMap::new();
        for criterion in &settings.monitors {
            let state = entry.criteria.entry(criterion.id.clone()).or_default();
            if !state.is_due(criterion, timing) {
                continue;
            }
            groups
                .entry((
                    model_for(criterion, &settings),
                    window_key(criterion.window),
                ))
                .or_default()
                .push(criterion);
        }
        let mut planned = Vec::new();
        for ((model, (entries, max_chars, tool_calls)), criteria) in groups {
            let spec = WindowSpec {
                messages: entries as usize,
                max_chars: max_chars as usize,
                tool_calls,
            };
            let Some(state) = build_window(conversation, spec, iteration) else {
                continue;
            };
            let mut questions = IndexMap::new();
            let mut asked = Vec::new();
            for criterion in criteria {
                let criterion_state = entry.criteria.entry(criterion.id.clone()).or_default();
                if criterion_state.last_window.as_deref() == Some(state.sha256.as_str()) {
                    continue;
                }
                match monitor_question(criterion) {
                    Ok(question) => {
                        questions.insert(criterion.id.clone(), question);
                        asked.push((criterion.id.clone(), criterion.name.clone()));
                    }
                    Err(why) => {
                        criterion_state.turn.disabled = true;
                        entry.invalid.push((criterion.name.clone(), why));
                    }
                }
            }
            if questions.is_empty() {
                continue;
            }
            planned.push(PlannedRequest {
                model,
                criteria: asked,
                request: DecisionRequest {
                    questions,
                    context: state.context,
                },
                window_sha: state.sha256,
            });
        }
        planned
    }

    /// Criteria whose question could not be built since the last call, as
    /// (name, why). Each is reported once per turn.
    pub fn take_invalid(&self, session_id: &str) -> Vec<(String, String)> {
        self.lock()
            .get_mut(session_id)
            .map(|entry| std::mem::take(&mut entry.invalid))
            .unwrap_or_default()
    }

    /// Fold the answers to `planned` in, in the same order, and return what
    /// each criterion with something to say decided.
    pub fn apply(
        &self,
        session_id: &str,
        planned: &[PlannedRequest],
        outcomes: Vec<AskOutcome>,
        steer_waiting: bool,
    ) -> Vec<MonitorEvent> {
        let mut sessions = self.lock();
        let Some(entry) = sessions.get_mut(session_id) else {
            return Vec::new();
        };
        let Some(settings) = entry.settings.clone() else {
            return Vec::new();
        };
        let mut events = Vec::new();
        for (plan, outcome) in planned.iter().zip(outcomes) {
            for (id, _) in &plan.criteria {
                let Some(criterion) = settings.monitors.iter().find(|m| &m.id == id) else {
                    continue;
                };
                let observation = match &outcome {
                    Ok(decisions) => match decisions.iter().find(|d| &d.question == id) {
                        Some(decision) => {
                            Observation::Probability(decision.pass_probability.unwrap_or(0.0))
                        }
                        None => Observation::Error {
                            message: format!("the decision model gave no answer for `{id}`"),
                            refused: false,
                        },
                    },
                    Err(error) => Observation::Error {
                        message: error.to_string(),
                        refused: matches!(error, DecisionError::Refused(_)),
                    },
                };
                let state = entry.criteria.entry(id.clone()).or_default();
                // An answered window is not asked again until it changes; a
                // failed one is retried at the next evaluation point.
                if outcome.is_ok() {
                    state.last_window = Some(plan.window_sha.clone());
                }
                let step = state.observe(criterion, observation, steer_waiting);
                if step != Step::Quiet {
                    events.push(MonitorEvent {
                        criterion_id: id.clone(),
                        name: criterion.name.clone(),
                        question: criterion.question.instructions.clone(),
                        threshold: criterion.threshold,
                        step,
                    });
                }
            }
        }
        events
    }

    /// One marker item per criterion evaluated this turn.
    pub fn marker_items(&self, session_id: &str) -> Vec<DecisionTurnMarkerItem> {
        let sessions = self.lock();
        let Some(entry) = sessions.get(session_id) else {
            return Vec::new();
        };
        let Some(settings) = entry.settings.as_ref() else {
            return Vec::new();
        };
        settings
            .monitors
            .iter()
            .filter_map(|criterion| {
                let state = entry.criteria.get(&criterion.id)?;
                (state.turn.evaluations > 0).then(|| DecisionTurnMarkerItem {
                    origin: DecisionOrigin::Monitor,
                    origin_id: Some(criterion.id.clone()),
                    label: criterion.name.clone(),
                    count: u64::from(state.turn.evaluations),
                    passed: u64::from(state.turn.at_or_above),
                    failed: u64::from(state.turn.below),
                    errors: u64::from(state.turn.errors),
                    last_pass_probability: state.last_p,
                    fired: state.turn.fired,
                })
            })
            .collect()
    }

    /// Whether a criterion was turned off for the rest of the turn.
    pub fn is_off_this_turn(&self, session_id: &str, criterion_id: &str) -> bool {
        self.lock()
            .get(session_id)
            .and_then(|entry| entry.criteria.get(criterion_id))
            .is_some_and(|state| state.turn.disabled)
    }
}

/// Ask every planned request, in parallel, through `service`. `base` is the
/// chat as sampled for this point; each request is logged with origin
/// `monitor`, the criterion id (ids joined with `+` for a shared request), the
/// monitor's name and the turn.
pub async fn ask_planned(
    service: &DecisionService,
    base: &DecisionSession,
    planned: &[PlannedRequest],
) -> Vec<AskOutcome> {
    let asks = planned.iter().map(|plan| {
        let session = base
            .clone()
            .with_origin(DecisionOrigin::Monitor, Some(plan.origin_id()))
            .with_origin_label(Some(plan.label()));
        async move {
            service
                .decide(Some(&session), plan.model.as_ref(), &plan.request)
                .await
        }
    });
    futures::future::join_all(asks).await
}

/// How a probability reads in a notice: two decimals.
pub fn format_p(p: f64) -> String {
    format!("{p:.2}")
}

/// The notice for one monitor event, and whether the loop should stop or
/// tell the model. `None` for a step that says nothing.
pub fn event_notice(event: &MonitorEvent) -> Option<String> {
    let name = &event.name;
    let threshold = format_p(event.threshold);
    match &event.step {
        Step::Quiet | Step::Suppressed { .. } => None,
        Step::Fired { action, p } => {
            let p = format_p(*p);
            Some(match action {
                MonitorAction::Stop => format!(
                    "Monitor '{name}' fired (P {p}, threshold {threshold}), so the turn stops \
                     here. Send a message to carry on."
                ),
                MonitorAction::Notify => {
                    format!("Monitor '{name}' fired (P {p}, threshold {threshold}).")
                }
                MonitorAction::Steer => format!(
                    "Monitor '{name}' fired (P {p}, threshold {threshold}) and told the agent."
                ),
            })
        }
        Step::FailedClosed { error } => Some(format!(
            "Monitor '{name}' could not be checked ({error}). It is set to pause when that \
             happens, so the turn stops here."
        )),
        Step::ErrorNotice { error, off: true } => Some(format!(
            "Monitor '{name}' could not be checked ({error}), and is off for the rest of this \
             turn."
        )),
        Step::ErrorNotice { error, off: false } => Some(format!(
            "Monitor '{name}' could not be checked ({error}). It keeps trying; after \
             {ERRORS_BEFORE_OFF} failures in a row it is off for the rest of this turn."
        )),
    }
}

/// What the model reads when a `steer` monitor fires. Model-only.
pub fn steer_note(event: &MonitorEvent) -> Option<String> {
    let Step::Fired {
        action: MonitorAction::Steer,
        p,
    } = &event.step
    else {
        return None;
    };
    Some(format!(
        "A decision model watching this conversation reports that the monitor '{}' fired. Its \
         question: \"{}\" The probability of the answer it watches for is {} (threshold {}). \
         Take this into account before you continue. The monitor only reads the recent \
         conversation; check the evidence yourself before you rely on it.",
        event.name,
        event.question,
        format_p(*p),
        format_p(event.threshold)
    ))
}

/// Whether an event ends the turn.
pub fn event_stops(event: &MonitorEvent) -> bool {
    matches!(
        event.step,
        Step::Fired {
            action: MonitorAction::Stop,
            ..
        } | Step::FailedClosed { .. }
    )
}

// ---------------------------------------------------------------------------
// The turn tally.
// ---------------------------------------------------------------------------

/// Turns tallied at once. A turn whose reply never ended cleanly is dropped
/// oldest first once there are more.
const MAX_TALLIED_TURNS: usize = 512;

static TURN_TALLIES: LazyLock<Mutex<IndexMap<String, Vec<DecisionTurnMarkerItem>>>> =
    LazyLock::new(|| Mutex::new(IndexMap::new()));

fn default_label(origin: DecisionOrigin) -> &'static str {
    match origin {
        DecisionOrigin::Chat => "decision chat",
        DecisionOrigin::Monitor => "monitor",
        DecisionOrigin::Tool => "decide",
        DecisionOrigin::Gate => "gate",
        DecisionOrigin::Adhoc => "decision",
    }
}

/// Count one recorded decision call against its turn. The decision service
/// calls this for every entry it hands its sink; an entry without a turn id
/// (an ad hoc decision) is not counted.
pub fn note_decision(entry: &DecisionLogEntry) {
    let Some(turn_id) = entry.turn_id.as_deref() else {
        return;
    };
    let mut tallies = TURN_TALLIES.lock().unwrap_or_else(PoisonError::into_inner);
    if !tallies.contains_key(turn_id) && tallies.len() >= MAX_TALLIED_TURNS {
        tallies.shift_remove_index(0);
    }
    let items = tallies.entry(turn_id.to_string()).or_default();
    let index = match items
        .iter()
        .position(|item| item.origin == entry.origin && item.origin_id == entry.origin_id)
    {
        Some(index) => index,
        None => {
            items.push(DecisionTurnMarkerItem {
                origin: entry.origin,
                origin_id: entry.origin_id.clone(),
                label: entry
                    .origin_label
                    .clone()
                    .unwrap_or_else(|| default_label(entry.origin).to_string()),
                count: 0,
                passed: 0,
                failed: 0,
                errors: 0,
                last_pass_probability: None,
                fired: false,
            });
            items.len() - 1
        }
    };
    let item = &mut items[index];
    item.count += 1;
    match entry.outcome {
        DecisionOutcome::Passed => item.passed += 1,
        DecisionOutcome::Failed => item.failed += 1,
        DecisionOutcome::Error | DecisionOutcome::Refused => item.errors += 1,
        DecisionOutcome::Decided => {}
    }
    if let Some(p) = super::log::lowest_pass_probability(&entry.decisions) {
        item.last_pass_probability = Some(p);
    }
}

/// Take the tally of `turn_id`, leaving none behind.
pub fn take_turn_tally(turn_id: &str) -> Vec<DecisionTurnMarkerItem> {
    TURN_TALLIES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .shift_remove(turn_id)
        .unwrap_or_default()
}

/// Whether any decision has been counted against `turn_id`.
pub fn turn_has_tally(turn_id: &str) -> bool {
    TURN_TALLIES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(turn_id)
}

/// The turn's marker: the monitors' own per-criterion items (which a shared
/// multi-question request cannot give the log), then every other origin's
/// tally. A decision chat's own decision is its reply card, so it is left out.
/// `None` when nothing ran.
pub fn turn_marker(
    turn_id: &str,
    monitors: Vec<DecisionTurnMarkerItem>,
    tally: Vec<DecisionTurnMarkerItem>,
) -> Option<DecisionTurnMarker> {
    let mut items = monitors;
    items.extend(
        tally
            .into_iter()
            .filter(|item| !matches!(item.origin, DecisionOrigin::Monitor | DecisionOrigin::Chat)),
    );
    (!items.is_empty()).then(|| DecisionTurnMarker {
        turn_id: turn_id.to_string(),
        items,
    })
}

fn plural(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The marker's text, readable on every surface (the desktop draws the
/// structured `decisionSummary` instead): one clause per origin.
pub fn marker_text(marker: &DecisionTurnMarker) -> String {
    let clauses: Vec<String> = marker
        .items
        .iter()
        .map(|item| {
            let mut clause = match item.origin {
                DecisionOrigin::Monitor => {
                    let mut text = format!(
                        "monitor '{}' checked {}",
                        item.label,
                        plural(item.count, "time", "times")
                    );
                    if let Some(p) = item.last_pass_probability {
                        text.push_str(&format!(" (last P {})", format_p(p)));
                    }
                    if item.fired {
                        text.push_str(", fired");
                    }
                    text
                }
                DecisionOrigin::Tool => {
                    format!("{}: {}", item.label, plural(item.count, "chunk", "chunks"))
                }
                DecisionOrigin::Gate => format!(
                    "gate '{}': {}, {} passed",
                    item.label,
                    plural(item.count, "check", "checks"),
                    item.passed
                ),
                DecisionOrigin::Chat | DecisionOrigin::Adhoc => {
                    format!("{}: {}", item.label, plural(item.count, "call", "calls"))
                }
            };
            if item.errors > 0 {
                clause.push_str(&format!(", {}", plural(item.errors, "error", "errors")));
            }
            clause
        })
        .collect();
    format!("Decision models: {}", clauses.join("; "))
}

/// The decision service an agent asks: the process's, unless a test or an
/// embedder installed its own.
#[derive(Default)]
pub struct ServiceSlot {
    custom: RwLock<Option<Arc<DecisionService>>>,
}

impl ServiceSlot {
    pub fn set(&self, service: Arc<DecisionService>) {
        *self.custom.write().unwrap_or_else(PoisonError::into_inner) = Some(service);
    }

    /// The installed service, or `None` for the process's [`super::global`].
    pub fn custom(&self) -> Option<Arc<DecisionService>> {
        self.custom
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Everything the agent keeps about decision models, as one field.
#[derive(Default)]
pub struct AgentDecisions {
    pub monitors: Arc<MonitorRegistry>,
    pub service: ServiceSlot,
}

/// One reply, registered as a decision turn for as long as it lives: a fresh
/// turn id in [`super::turns`] (which the `decide` tool and phase gates read)
/// and in the agent's [`MonitorRegistry`]. Dropping it, however the reply
/// ended, ends the turn there and forgets its tally, so a Stop that drops the
/// reply stream leaves nothing behind.
pub struct DecisionTurnGuard {
    session_id: String,
    turn_id: String,
    monitors: Arc<MonitorRegistry>,
}

impl DecisionTurnGuard {
    pub fn begin(monitors: Arc<MonitorRegistry>, session_id: &str) -> Self {
        let turn_id = uuid::Uuid::now_v7().to_string();
        super::turns::begin_turn(session_id, &turn_id);
        monitors.begin_turn(session_id, &turn_id);
        Self {
            session_id: session_id.to_string(),
            turn_id,
            monitors,
        }
    }

    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }
}

impl Drop for DecisionTurnGuard {
    fn drop(&mut self) {
        super::turns::end_turn(&self.session_id, &self.turn_id);
        self.monitors.end_turn(&self.session_id, &self.turn_id);
        let _ = take_turn_tally(&self.turn_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::message::Message;
    use crate::decision::types::{DecisionKind, Threshold};

    fn criterion(threshold: f64) -> MonitorCriterion {
        MonitorCriterion {
            id: "m1".into(),
            name: "Form reached".into(),
            question: Question {
                instructions: "Has the agent reached the consent form page?".into(),
                kind: DecisionKind::Binary {},
                threshold: None,
            },
            model: None,
            timing: MonitorTiming::PostIteration,
            window: MonitorWindow::default(),
            threshold,
            option: None,
            consecutive: 1,
            rearm_below: None,
            action: MonitorAction::Stop,
            on_error: MonitorErrorPolicy::Notify,
            max_evals_per_turn: 50,
            enabled: true,
        }
    }

    fn p(value: f64) -> Observation {
        Observation::Probability(value)
    }

    fn error() -> Observation {
        Observation::Error {
            message: "connection refused".into(),
            refused: false,
        }
    }

    #[test]
    fn a_criterion_fires_at_its_threshold_including_equality() {
        let c = criterion(0.85);
        let mut state = CriterionState::default();
        assert_eq!(state.observe(&c, p(0.5), false), Step::Quiet);
        assert_eq!(
            state.observe(&c, p(0.85), false),
            Step::Fired {
                action: MonitorAction::Stop,
                p: 0.85
            }
        );
        assert!(state.latched);
        assert!(state.turn.fired);
        assert_eq!(state.turn.evaluations, 2);
        assert_eq!((state.turn.at_or_above, state.turn.below), (1, 1));
    }

    #[test]
    fn consecutive_evaluations_must_all_reach_the_threshold() {
        let mut c = criterion(0.8);
        c.consecutive = 3;
        let mut state = CriterionState::default();
        assert_eq!(state.observe(&c, p(0.9), false), Step::Quiet);
        assert_eq!(state.observe(&c, p(0.9), false), Step::Quiet);
        // A dip resets the run.
        assert_eq!(state.observe(&c, p(0.7), false), Step::Quiet);
        assert_eq!(state.observe(&c, p(0.9), false), Step::Quiet);
        assert_eq!(state.observe(&c, p(0.95), false), Step::Quiet);
        assert!(matches!(
            state.observe(&c, p(0.91), false),
            Step::Fired { .. }
        ));
    }

    #[test]
    fn a_fired_criterion_latches_until_p_falls_below_its_rearm_level() {
        let mut c = criterion(0.85);
        c.rearm_below = Some(0.7);
        c.action = MonitorAction::Notify;
        let mut state = CriterionState::default();
        assert!(matches!(
            state.observe(&c, p(0.9), false),
            Step::Fired { .. }
        ));
        // Still high: latched, quiet.
        assert_eq!(state.observe(&c, p(0.95), false), Step::Quiet);
        // Between the re-arm level and the threshold: still latched.
        assert_eq!(state.observe(&c, p(0.75), false), Step::Quiet);
        assert!(state.latched);
        // Below the re-arm level: re-armed, and it fires again on the next rise.
        assert_eq!(state.observe(&c, p(0.6), false), Step::Quiet);
        assert!(!state.latched);
        assert!(matches!(
            state.observe(&c, p(0.9), false),
            Step::Fired { .. }
        ));
    }

    #[test]
    fn without_a_rearm_level_the_threshold_itself_rearms() {
        let mut c = criterion(0.85);
        c.action = MonitorAction::Notify;
        let mut state = CriterionState::default();
        assert!(matches!(
            state.observe(&c, p(0.9), false),
            Step::Fired { .. }
        ));
        assert_eq!(state.observe(&c, p(0.84), false), Step::Quiet);
        assert!(!state.latched);
    }

    #[test]
    fn the_latch_survives_a_new_turn_but_the_turn_counters_do_not() {
        let c = criterion(0.85);
        let mut state = CriterionState::default();
        assert!(matches!(
            state.observe(&c, p(0.9), false),
            Step::Fired { .. }
        ));
        state.begin_turn();
        assert!(state.latched);
        assert_eq!(state.turn, CriterionTurn::default());
        assert_eq!(state.observe(&c, p(0.9), false), Step::Quiet);
    }

    #[test]
    fn three_errors_in_a_row_turn_a_notify_criterion_off_with_one_notice() {
        let c = criterion(0.85);
        let mut state = CriterionState::default();
        assert_eq!(
            state.observe(&c, error(), false),
            Step::ErrorNotice {
                error: "connection refused".into(),
                off: false
            }
        );
        assert!(state.is_due(&c, MonitorTiming::PostIteration));
        assert_eq!(state.observe(&c, error(), false), Step::Quiet);
        assert_eq!(state.observe(&c, error(), false), Step::Quiet);
        assert!(!state.is_due(&c, MonitorTiming::PostIteration));
        assert_eq!(state.turn.errors, 3);
        // A success in between resets the streak.
        let mut fresh = CriterionState::default();
        fresh.observe(&c, error(), false);
        fresh.observe(&c, error(), false);
        fresh.observe(&c, p(0.1), false);
        fresh.observe(&c, error(), false);
        assert!(fresh.is_due(&c, MonitorTiming::PostIteration));
    }

    #[test]
    fn a_refusal_turns_the_criterion_off_at_once() {
        let c = criterion(0.85);
        let mut state = CriterionState::default();
        let step = state.observe(
            &c,
            Observation::Error {
                message: "private chat".into(),
                refused: true,
            },
            false,
        );
        assert_eq!(
            step,
            Step::ErrorNotice {
                error: "private chat".into(),
                off: true
            }
        );
        assert!(!state.is_due(&c, MonitorTiming::PostIteration));
    }

    #[test]
    fn a_pause_criterion_fails_closed_on_an_error() {
        let mut c = criterion(0.85);
        c.on_error = MonitorErrorPolicy::Pause;
        let mut state = CriterionState::default();
        let step = state.observe(&c, error(), false);
        assert!(matches!(step, Step::FailedClosed { .. }));
        assert!(event_stops(&MonitorEvent {
            criterion_id: "m1".into(),
            name: "Form reached".into(),
            question: String::new(),
            threshold: 0.85,
            step,
        }));
    }

    #[test]
    fn a_waiting_steer_holds_back_a_stop_and_keeps_the_criterion_armed() {
        let c = criterion(0.85);
        let mut state = CriterionState::default();
        assert_eq!(
            state.observe(&c, p(0.9), true),
            Step::Suppressed { p: Some(0.9) }
        );
        assert!(!state.latched);
        assert!(!state.turn.fired);
        // Once the person's message is answered, it fires.
        assert!(matches!(
            state.observe(&c, p(0.9), false),
            Step::Fired { .. }
        ));
        // A notify criterion is not held back: it does not end anything.
        let mut notify = criterion(0.85);
        notify.action = MonitorAction::Notify;
        let mut other = CriterionState::default();
        assert!(matches!(
            other.observe(&notify, p(0.9), true),
            Step::Fired { .. }
        ));
    }

    #[test]
    fn the_evaluation_cap_and_the_timing_and_the_switch_decide_what_is_due() {
        let mut c = criterion(0.85);
        c.max_evals_per_turn = 2;
        let mut state = CriterionState::default();
        assert!(state.is_due(&c, MonitorTiming::PostIteration));
        assert!(!state.is_due(&c, MonitorTiming::TurnEnd));
        state.observe(&c, p(0.1), false);
        state.observe(&c, p(0.1), false);
        assert!(!state.is_due(&c, MonitorTiming::PostIteration));
        state.begin_turn();
        assert!(state.is_due(&c, MonitorTiming::PostIteration));
        c.enabled = false;
        assert!(!state.is_due(&c, MonitorTiming::PostIteration));
        // Whatever is stored, no more than the hard cap.
        c.enabled = true;
        c.max_evals_per_turn = 10_000;
        state.turn.evaluations = MAX_EVALS_PER_TURN;
        assert!(!state.is_due(&c, MonitorTiming::PostIteration));
    }

    fn conversation(texts: &[&str]) -> Conversation {
        Conversation::new_unvalidated(
            texts
                .iter()
                .enumerate()
                .map(|(i, text)| {
                    if i % 2 == 0 {
                        Message::user().with_text(*text)
                    } else {
                        Message::assistant().with_text(*text)
                    }
                })
                .collect::<Vec<_>>(),
        )
    }

    fn settings(monitors: Vec<MonitorCriterion>) -> DecisionSettings {
        DecisionSettings {
            default_model: None,
            monitors,
        }
    }

    #[test]
    fn criteria_sharing_a_model_and_window_share_one_request() {
        let registry = MonitorRegistry::default();
        registry.begin_turn("s", "t1");
        let mut second = criterion(0.5);
        second.id = "m2".into();
        second.name = "Signed".into();
        let mut elsewhere = criterion(0.5);
        elsewhere.id = "m3".into();
        elsewhere.name = "Other model".into();
        elsewhere.model = Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b"));
        let mut later = criterion(0.5);
        later.id = "m4".into();
        later.timing = MonitorTiming::TurnEnd;
        registry.cache_settings(
            "s",
            settings(vec![criterion(0.8), second, elsewhere, later]),
        );
        assert!(registry.any_due("s", MonitorTiming::PostIteration));
        assert!(registry.any_due("s", MonitorTiming::TurnEnd));

        let conv = conversation(&["Open the consent form.", "It is open."]);
        let planned = registry.plan("s", MonitorTiming::PostIteration, &conv, 1);
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].origin_id(), "m1+m2");
        assert_eq!(planned[0].label(), "Form reached + Signed");
        assert_eq!(
            planned[0].request.questions.keys().collect::<Vec<_>>(),
            vec!["m1", "m2"]
        );
        assert_eq!(
            planned[0].request.questions["m1"].threshold,
            Some(Threshold::Yes(0.8))
        );
        assert_eq!(planned[1].origin_id(), "m3");
        assert!(planned[0].request.validate().is_ok());
    }

    fn decision_for(question: &str, pass_probability: f64) -> Decision {
        use crate::decision::types::{Answer, DecisionUsage, ProbabilitySource};
        Decision {
            question: question.into(),
            answer: Answer::Binary {
                value: pass_probability >= 0.5,
            },
            probabilities: IndexMap::from([
                ("yes".to_string(), pass_probability),
                ("no".to_string(), 1.0 - pass_probability),
            ]),
            confidence: (2.0 * pass_probability - 1.0).abs(),
            passed: None,
            pass_probability: Some(pass_probability),
            source: ProbabilitySource::Native,
            calibrated: true,
            model: DecisionModelRef::new("ollama_decision", "tev1:0.8b"),
            tier: crate::privacy::ProviderTier::Private,
            request_id: "r1".into(),
            latency_ms: 22,
            usage: DecisionUsage::default(),
            raw: serde_json::Value::Null,
        }
    }

    #[test]
    fn an_unchanged_window_is_not_asked_again_and_answers_reach_each_criterion() {
        let registry = MonitorRegistry::default();
        registry.begin_turn("s", "t1");
        let mut second = criterion(0.5);
        second.id = "m2".into();
        second.name = "Signed".into();
        second.action = MonitorAction::Notify;
        registry.cache_settings("s", settings(vec![criterion(0.8), second]));
        let conv = conversation(&["Open the consent form.", "It is open."]);
        let planned = registry.plan("s", MonitorTiming::PostIteration, &conv, 1);
        let events = registry.apply(
            "s",
            &planned,
            vec![Ok(vec![decision_for("m1", 0.3), decision_for("m2", 0.9)])],
            false,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].criterion_id, "m2");
        assert_eq!(
            event_notice(&events[0]).as_deref(),
            Some("Monitor 'Signed' fired (P 0.90, threshold 0.50).")
        );
        // Same window: nothing to ask.
        assert!(registry
            .plan("s", MonitorTiming::PostIteration, &conv, 2)
            .is_empty());
        // A new message changes the window.
        let longer = conversation(&["Open the consent form.", "It is open.", "Sign it."]);
        assert_eq!(
            registry
                .plan("s", MonitorTiming::PostIteration, &longer, 3)
                .len(),
            1
        );
        let items = registry.marker_items("s");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "Form reached");
        assert_eq!(items[0].last_pass_probability, Some(0.3));
        assert!(!items[0].fired);
        assert!(items[1].fired);
    }

    #[test]
    fn a_failed_request_is_an_error_for_every_criterion_it_carried() {
        let registry = MonitorRegistry::default();
        registry.begin_turn("s", "t1");
        registry.cache_settings("s", settings(vec![criterion(0.8)]));
        let conv = conversation(&["Open the consent form."]);
        let planned = registry.plan("s", MonitorTiming::PostIteration, &conv, 1);
        let events = registry.apply(
            "s",
            &planned,
            vec![Err(DecisionError::Refused("this chat is private".into()))],
            false,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].step,
            Step::ErrorNotice { off: true, .. }
        ));
        assert!(registry.is_off_this_turn("s", "m1"));
        assert!(!registry.any_due("s", MonitorTiming::PostIteration));
        // The failed window is retried next turn.
        registry.begin_turn("s", "t2");
        registry.cache_settings("s", settings(vec![criterion(0.8)]));
        assert_eq!(
            registry
                .plan("s", MonitorTiming::PostIteration, &conv, 1)
                .len(),
            1
        );
    }

    #[test]
    fn a_criterion_that_cannot_be_asked_is_reported_once_and_left_off() {
        let registry = MonitorRegistry::default();
        registry.begin_turn("s", "t1");
        let mut bad = criterion(0.8);
        bad.option = Some("maybe".into());
        registry.cache_settings("s", settings(vec![bad]));
        let conv = conversation(&["Hello."]);
        assert!(registry
            .plan("s", MonitorTiming::PostIteration, &conv, 1)
            .is_empty());
        assert_eq!(registry.take_invalid("s").len(), 1);
        assert!(registry.take_invalid("s").is_empty());
        assert!(!registry.any_due("s", MonitorTiming::PostIteration));
    }

    #[test]
    fn a_chat_without_settings_has_nothing_due() {
        let registry = MonitorRegistry::default();
        assert!(!registry.any_due("nobody", MonitorTiming::PostIteration));
        registry.begin_turn("s", "t1");
        assert!(!registry.any_due("s", MonitorTiming::PostIteration));
        registry.cache_settings("s", DecisionSettings::default());
        assert!(!registry.any_due("s", MonitorTiming::PostIteration));
        assert!(registry
            .plan("s", MonitorTiming::PostIteration, &conversation(&["x"]), 1)
            .is_empty());
    }

    fn entry(
        turn: &str,
        origin: DecisionOrigin,
        id: &str,
        outcome: DecisionOutcome,
    ) -> DecisionLogEntry {
        DecisionLogEntry {
            session_id: Some("s".into()),
            origin,
            origin_id: Some(id.into()),
            origin_label: Some(
                if origin == DecisionOrigin::Tool {
                    "decide"
                } else {
                    "Build"
                }
                .into(),
            ),
            turn_id: Some(turn.into()),
            batch_id: None,
            chunk_index: None,
            chunk_id: None,
            model: None,
            served_model: None,
            tier: None,
            questions: IndexMap::new(),
            context: crate::decision::types::DecisionContext::Text(String::new()),
            context_bytes: 0,
            outcome,
            decisions: Vec::new(),
            error: None,
            latency_ms: None,
            usage: Default::default(),
            privacy_enforced: true,
        }
    }

    #[test]
    fn a_turn_guard_registers_the_turn_until_it_is_dropped() {
        let monitors = Arc::new(MonitorRegistry::default());
        let guard = DecisionTurnGuard::begin(monitors.clone(), "guard-test-session");
        let turn = guard.turn_id().to_string();
        assert_eq!(
            super::super::turns::current_turn("guard-test-session").as_deref(),
            Some(turn.as_str())
        );
        assert_eq!(monitors.turn_id("guard-test-session"), Some(turn.clone()));
        note_decision(&entry(
            &turn,
            DecisionOrigin::Tool,
            "c",
            DecisionOutcome::Decided,
        ));
        drop(guard);
        assert!(super::super::turns::current_turn("guard-test-session").is_none());
        assert!(monitors.turn_id("guard-test-session").is_none());
        assert!(!turn_has_tally(&turn), "a dropped turn forgets its tally");
    }

    #[test]
    fn the_tally_aggregates_per_origin_and_the_marker_reads_as_one_line() {
        let turn = "tally-test-turn";
        for _ in 0..598 {
            note_decision(&entry(
                turn,
                DecisionOrigin::Tool,
                "call-1",
                DecisionOutcome::Decided,
            ));
        }
        note_decision(&entry(
            turn,
            DecisionOrigin::Tool,
            "call-1",
            DecisionOutcome::Error,
        ));
        note_decision(&entry(
            turn,
            DecisionOrigin::Tool,
            "call-1",
            DecisionOutcome::Refused,
        ));
        note_decision(&entry(
            turn,
            DecisionOrigin::Gate,
            "phase-1",
            DecisionOutcome::Failed,
        ));
        note_decision(&entry(
            turn,
            DecisionOrigin::Gate,
            "phase-1",
            DecisionOutcome::Passed,
        ));
        note_decision(&entry(
            turn,
            DecisionOrigin::Monitor,
            "m1",
            DecisionOutcome::Decided,
        ));
        note_decision(&entry(
            turn,
            DecisionOrigin::Chat,
            "x",
            DecisionOutcome::Decided,
        ));
        assert!(turn_has_tally(turn));
        let tally = take_turn_tally(turn);
        assert!(!turn_has_tally(turn));
        assert_eq!(tally.len(), 4);
        let monitors = vec![DecisionTurnMarkerItem {
            origin: DecisionOrigin::Monitor,
            origin_id: Some("m1".into()),
            label: "Form reached".into(),
            count: 18,
            passed: 0,
            failed: 18,
            errors: 0,
            last_pass_probability: Some(0.31),
            fired: false,
        }];
        let marker = turn_marker(turn, monitors, tally).unwrap();
        assert_eq!(
            marker.items.len(),
            3,
            "monitor and chat tallies are left out"
        );
        assert_eq!(
            marker_text(&marker),
            "Decision models: monitor 'Form reached' checked 18 times (last P 0.31); \
             decide: 600 chunks, 2 errors; gate 'Build': 2 checks, 1 passed"
        );
        assert!(turn_marker("empty", Vec::new(), Vec::new()).is_none());
    }
}
