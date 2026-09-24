//! Shared wire contract for human-authorized daemon room observation.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Default, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Initial {
    #[default]
    Latest,
    All,
}
#[derive(Deserialize, Serialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ObserveRequest {
    pub channel_id: Option<String>,
    pub after: Option<String>,
    #[serde(default)]
    pub initial: Initial,
}
#[derive(Deserialize, Serialize, utoipa::ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ObserveEvent {
    State {
        connection_id: String,
        #[schema(inline)]
        connection_mode: super::ClusterMode,
        connection_policy_epoch: u64,
        connection_institution_id: Option<String>,
        snapshot: Value,
        runs: Vec<Value>,
        /// How each person the snapshot names is shown, keyed by principal ID. Absent from a
        /// daemon that predates it; clients then compute their own.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        #[schema(inline)]
        labels: BTreeMap<String, PersonLabel>,
    },
    Messages {
        channel_id: String,
        messages: Vec<Value>,
        cursor: Option<String>,
        reset: bool,
    },
    Reconnect {
        cursor: Option<String>,
    },
    Error {
        code: String,
        error: String,
        clear: bool,
    },
}

/// How one person is shown, from the naming design's display rule. Display only: the daemon
/// computes it from the same snapshot the frame carries, so every client applies one collision
/// rule, including the confusable skeleton a renderer cannot reproduce.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
pub struct PersonLabel {
    /// `Display name (@username)`, or `@username` when the two are equal case-insensitively;
    /// always both when the name collides with someone else's.
    pub full: String,
    /// The display name alone, for chips and avatars; `@username` when the two are equal, and
    /// the same as `full` when the name collides.
    pub short: String,
    /// Another person in this workspace has a display name that is, or looks like, this one.
    /// Both are then shown with their `@username` everywhere.
    pub collides: bool,
}
