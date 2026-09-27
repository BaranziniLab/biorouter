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
        /// What the connected broker's last verified `hello` said it supports (for example
        /// `unique_names_v1`). They decide only which requests a client offers; the broker still
        /// refuses what it does not support. Absent before the first verified hello, and from a
        /// daemon that predates it.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capabilities: Vec<String>,
    },
    Messages {
        channel_id: String,
        messages: Vec<Value>,
        cursor: Option<String>,
        reset: bool,
        /// How many messages of the page this frame was taken from are still to come. `0` ends
        /// the page, so on the opening frames it marks the end of the channel's backlog. Absent
        /// from a daemon that predates it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remaining: Option<u32>,
        /// The largest page the observer asks the broker for right now. It shrinks when the
        /// broker answers `response_too_large`, so a client must not assume a fixed size.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        page_size: Option<u32>,
        /// How the broker names each author of this frame's messages, keyed by principal ID,
        /// including a person who has since left the workspace. Display only.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        #[schema(inline)]
        people: BTreeMap<String, MessagePerson>,
        /// The names of the channels this frame's messages name, limited by the broker to the
        /// channels the viewer can read. Display only.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        channel_names: BTreeMap<String, String>,
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

/// One entry of the `people` map the broker returns beside messages (`messages.history`).
/// Display only: it names a message's author, and is the only name a client has for someone who
/// has left the workspace since they posted.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
pub struct MessagePerson {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// False for a person who was removed from the workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
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
