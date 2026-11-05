//! Per-chat decision settings: the chat's default decision model, its monitors,
//! and, for a chat whose main model is a decision model, the question it asks.
//!
//! Stored in the chat's `extension_data` under [`DECISION_SETTINGS_STATE`] and
//! [`DECISION_CHAT_STATE`], written only through `update_extension_state` (one key
//! per transaction).
//!
//! Contract module for wave 2: the types and signatures here are fixed. Fill the
//! bodies; do not change shapes without telling the coordinator.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::types::{DecisionModelRef, Question};
use crate::session::SessionManager;

pub const DECISION_SETTINGS_STATE: &str = "decision_settings";
pub const DECISION_SETTINGS_VERSION: &str = "v1";
pub const DECISION_CHAT_STATE: &str = "decision_chat";
pub const DECISION_CHAT_VERSION: &str = "v1";

/// At most this many monitors per chat.
pub const MAX_MONITORS: usize = 8;

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
    /// Newest entries kept (messages and tool lines), at most 48.
    pub entries: u32,
    /// Character budget, at most 12000.
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

/// Check a settings value before it is stored. `Err` is the sentence a caller shows.
pub fn validate(settings: &DecisionSettings) -> Result<(), String> {
    let _ = settings;
    Ok(())
}

pub async fn load(
    session_manager: &SessionManager,
    session_id: &str,
) -> anyhow::Result<DecisionSettings> {
    let _ = (session_manager, session_id);
    Ok(DecisionSettings::default())
}

/// Validate and store. Returns the stored value.
pub async fn save(
    session_manager: &SessionManager,
    session_id: &str,
    settings: DecisionSettings,
) -> anyhow::Result<DecisionSettings> {
    let _ = (session_manager, session_id);
    Ok(settings)
}

pub async fn load_chat(
    session_manager: &SessionManager,
    session_id: &str,
) -> anyhow::Result<Option<DecisionChatSettings>> {
    let _ = (session_manager, session_id);
    Ok(None)
}

pub async fn save_chat(
    session_manager: &SessionManager,
    session_id: &str,
    settings: DecisionChatSettings,
) -> anyhow::Result<DecisionChatSettings> {
    let _ = (session_manager, session_id);
    Ok(settings)
}
