//! The decision log: every decision call a chat made, stored as one row in the
//! `decisions` table, and the aggregates every surface draws from it.
//!
//! Decision models can be asked thousands of times in one chat (a monitor every
//! iteration, a `decide` batch over hundreds of chunks). The log is therefore the
//! ONLY place individual calls are listed. The transcript, the side panel, History,
//! the TUI and the CLI read [`summary`] (per source and criterion, and per turn) and
//! never draw one row per call; the decision history view pages through [`list`].
//!
//! Contract module for wave 2: the types and signatures here are fixed. Fill the
//! bodies; do not change shapes without telling the coordinator.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

use super::service::{DecisionLogEntry, DecisionOutcome, DecisionSink};
use super::types::{Decision, DecisionContext, DecisionModelRef, DecisionOrigin};
use crate::session::SessionManager;

/// Stored context is capped at this many bytes (the rest is cut and
/// `context_truncated` set).
pub const MAX_STORED_CONTEXT_BYTES: usize = 64 * 1024;
/// `contextPreview` length on list rows.
pub const CONTEXT_PREVIEW_CHARS: usize = 500;
/// Largest page [`list`] returns.
pub const MAX_PAGE: u32 = 200;
/// How many recent turns [`summary`] reports.
pub const RECENT_TURNS: usize = 20;

/// One decision call, as a list row (without the full stored context).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub origin: DecisionOrigin,
    /// The monitor id, tool call id or phase id that asked.
    pub origin_id: Option<String>,
    /// A human label for the origin (the monitor's name, "decide", the phase title).
    pub label: Option<String>,
    pub turn_id: Option<String>,
    pub batch_id: Option<String>,
    pub chunk_index: Option<u32>,
    pub chunk_id: Option<String>,
    pub model: Option<DecisionModelRef>,
    pub served_model: Option<String>,
    /// `public` or `private`, the decision model's tier.
    pub tier: Option<String>,
    pub calibrated: Option<bool>,
    /// The questions asked (name -> Question), as JSON.
    #[schema(value_type = Object)]
    pub questions: Value,
    pub decisions: Vec<Decision>,
    pub outcome: DecisionOutcome,
    pub context_preview: String,
    pub context_bytes: u64,
    pub context_truncated: bool,
    pub error: Option<String>,
    pub latency_ms: Option<u64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub privacy_enforced: bool,
}

/// One decision call with the exact context it was sent (capped).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionDetail {
    pub record: DecisionRecord,
    pub context: DecisionContext,
}

/// Filters for [`list`]. Every field is optional; `limit` is capped at [`MAX_PAGE`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionQuery {
    pub origin: Option<DecisionOrigin>,
    pub origin_id: Option<String>,
    pub turn_id: Option<String>,
    pub batch_id: Option<String>,
    pub outcome: Option<DecisionOutcome>,
    /// Keep rows whose lowest pass probability is at least this.
    pub min_p: Option<f64>,
    /// Keep rows whose lowest pass probability is at most this.
    pub max_p: Option<f64>,
    #[serde(default)]
    pub errors_only: bool,
    /// Opaque cursor from a previous page's `nextCursor` (newest first).
    pub before: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionPage {
    /// Newest first.
    pub records: Vec<DecisionRecord>,
    pub next_cursor: Option<String>,
    /// Rows matching the filter, across all pages.
    pub total: u64,
}

/// Every call of one origin (one monitor, the decide tool, one phase gate, the
/// decision chat) in one chat, aggregated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionAggregate {
    pub origin: DecisionOrigin,
    pub origin_id: Option<String>,
    pub label: String,
    pub count: u64,
    pub passed: u64,
    pub failed: u64,
    pub errors: u64,
    pub last_at: DateTime<Utc>,
    /// The last call's first answer, readable ("yes", "rrms", "level 2").
    pub last_answer: Option<String>,
    pub last_pass_probability: Option<f64>,
    pub model: Option<DecisionModelRef>,
}

/// One origin's calls within one turn, aggregated (the transcript marker's data).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionTurnSummary {
    pub turn_id: String,
    pub origin: DecisionOrigin,
    pub origin_id: Option<String>,
    pub label: String,
    pub count: u64,
    pub passed: u64,
    pub failed: u64,
    pub errors: u64,
    pub first_at: DateTime<Utc>,
    pub last_at: DateTime<Utc>,
    pub last_pass_probability: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionDecisionSummary {
    pub total: u64,
    pub aggregates: Vec<DecisionAggregate>,
    /// Newest turn first, at most [`RECENT_TURNS`] turns.
    pub recent_turns: Vec<DecisionTurnSummary>,
}

/// One origin's line in a per-turn transcript marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionTurnMarkerItem {
    pub origin: DecisionOrigin,
    pub origin_id: Option<String>,
    pub label: String,
    pub count: u64,
    pub passed: u64,
    pub failed: u64,
    pub errors: u64,
    pub last_pass_probability: Option<f64>,
    /// A monitor that fired this turn.
    #[serde(default)]
    pub fired: bool,
}

/// What a per-turn transcript marker carries (in `MessageMetadata::decision_summary`):
/// every decision call of one turn, aggregated per origin, never one row per call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DecisionTurnMarker {
    pub turn_id: String,
    pub items: Vec<DecisionTurnMarkerItem>,
}

/// Writes [`DecisionLogEntry`]s into the `decisions` table.
pub struct SqliteDecisionSink {
    pub session_manager: Arc<SessionManager>,
}

#[async_trait]
impl DecisionSink for SqliteDecisionSink {
    async fn record(&self, entry: DecisionLogEntry) -> anyhow::Result<()> {
        let _ = (&self.session_manager, entry);
        Ok(())
    }
}

/// One page of a chat's decisions, newest first.
pub async fn list(
    session_manager: &SessionManager,
    session_id: &str,
    query: &DecisionQuery,
) -> anyhow::Result<DecisionPage> {
    let _ = (session_manager, session_id, query);
    Ok(DecisionPage {
        records: Vec::new(),
        next_cursor: None,
        total: 0,
    })
}

/// One decision with its stored context, or `None` when the chat has no such row.
pub async fn get(
    session_manager: &SessionManager,
    session_id: &str,
    decision_id: &str,
) -> anyhow::Result<Option<DecisionDetail>> {
    let _ = (session_manager, session_id, decision_id);
    Ok(None)
}

/// A chat's decisions aggregated by origin and by recent turn.
pub async fn summary(
    session_manager: &SessionManager,
    session_id: &str,
) -> anyhow::Result<SessionDecisionSummary> {
    let _ = (session_manager, session_id);
    Ok(SessionDecisionSummary {
        total: 0,
        aggregates: Vec::new(),
        recent_turns: Vec::new(),
    })
}

/// Decision counts for several chats at once (History's per-chat count).
pub async fn count_for_sessions(
    session_manager: &SessionManager,
    session_ids: &[String],
) -> anyhow::Result<HashMap<String, u64>> {
    let _ = (session_manager, session_ids);
    Ok(HashMap::new())
}
