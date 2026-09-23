//! Shared wire contract for human-authorized daemon room observation.
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
