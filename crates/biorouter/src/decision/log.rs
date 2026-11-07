//! The decision log: every decision call a chat made, stored as one row in the
//! `decisions` table, and the aggregates every surface draws from it.
//!
//! Decision models can be asked thousands of times in one chat (a monitor every
//! iteration, a `decide` batch over hundreds of chunks). The log is therefore the
//! ONLY place individual calls are listed. The transcript, the side panel, History,
//! the TUI and the CLI read [`summary`] (per source and criterion, and per turn) and
//! never draw one row per call; the decision history view pages through [`list`].
//!
//! # What a row is
//!
//! One row is one model request: a monitor evaluation, one chunk of a `decide`
//! batch, one phase-gate check, one decision-chat reply. Every question the
//! request carried is in `decisions_json`, so a 3-question batch over 600 chunks
//! is 600 rows, not 1,800. A refusal or an error is a row too (outcome `refused`
//! or `error`), because the history view is where the person goes to see why a
//! monitor stopped answering. A batch stopped before any chunk was sent is ONE
//! such row, never one per chunk.
//!
//! A row's id is the request id every [`Decision`] of that request carries
//! ([`row_id`]), so anything holding a decision (a decision chat's reply, a
//! tool result) names its row without asking the log. A row with no decisions
//! (a failure) gets an id of its own, unless its caller chose the request id
//! before the call, as a decision chat does, so its failed reply names its row.
//!
//! # Privacy
//!
//! A row holds exactly what the model was sent (capped at
//! [`MAX_STORED_CONTEXT_BYTES`]), kept at the chat's tier. For a monitor, the
//! `decide` tool, a phase gate or a decision chat that is the chat's own
//! content; for an ad hoc decision about a chat (`POST /decisions/decide` with
//! a session, `biorouter decide --session`) it is whatever the caller sent,
//! which the chat's reach then covers.
//! The sink stores a row only beside a chat that exists, in the same statement
//! that checks it, and `SessionStorage::delete_chat_side_rows` deletes a chat's
//! rows with the chat, because session ids are reissued. A decision with no chat
//! (an ad hoc `biorouter decide` with no session) is not stored at all: no
//! surface lists it and no delete would ever remove it. Readers here take a
//! session id and never decide who may see it; every route that calls them asks
//! `session_reach` first.
//!
//! # Usage
//!
//! A stored row with token usage is also added to the usage ledger
//! (`token_events` and the chat's `accumulated_*` counters) through
//! `SessionManager::apply_side_usage_event`, keyed `decision:<row id>`, so the
//! Usage panel, `biorouter usage` and the chat's cost count it, as a decision
//! request rather than a turn. That write never moves the context gauge, the
//! chat's `schedule_id` or its `updated_at` (R-30), and a deleted chat's
//! decision spend stays counted with its other usage. A decision with no chat
//! is billed by its provider and counted nowhere here; `biorouter decide`
//! prints its usage instead.
//!
//! A request the provider answered is counted even when nobody was waiting
//! for the answer any more: its deadline had passed (the row says it timed
//! out), or its caller went away (a stopped turn, a dropped HTTP request).
//! The service hands such an answer to [`DecisionSink::record_late`] under
//! the request's id. That fills the tokens and the latency into the timeout
//! row already written, or writes the row when none was, and then bills it
//! under the same `decision:<row id>` key, so a request is billed once
//! however its answer arrived. A row that already holds tokens is left alone,
//! and a chat deleted meanwhile stores and bills nothing. An answer that
//! never arrives (a provider that bills a request it never answers, or a
//! process that exits first) is not counted.
//!
//! The table schema lives with the rest of the session store
//! (`SessionStorage::create_decisions_table`, migration 21).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite};
use utoipa::ToSchema;

use super::service::{DecisionLogEntry, DecisionOutcome, DecisionSink};
use super::types::{
    Answer, ContextMessage, Decision, DecisionAdapter, DecisionContext, DecisionModelRef,
    DecisionOrigin, ProbabilitySource, YES,
};
use crate::session::{SessionManager, SideUsageEntry};

/// Stored context is capped at this many bytes (the rest is cut and
/// `context_truncated` set).
pub const MAX_STORED_CONTEXT_BYTES: usize = 64 * 1024;
/// `contextPreview` length on list rows.
pub const CONTEXT_PREVIEW_CHARS: usize = 500;
/// Largest page [`list`] returns.
pub const MAX_PAGE: u32 = 200;
/// How many recent turns [`summary`] reports.
pub const RECENT_TURNS: usize = 20;
/// The page size [`list`] uses when the query names none.
pub const DEFAULT_PAGE: u32 = 50;

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
    /// Only the row with this id (a request id, as a decision chat's reply or
    /// a decision card names it): a page of at most one row.
    pub decision_id: Option<String>,
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

/// A [`DecisionQuery`] (or its cursor) this module cannot run. The message is
/// a sentence for the caller; a route answers it with 400 rather than 500,
/// which it can tell by `downcast_ref::<InvalidDecisionQuery>()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidDecisionQuery(pub String);

impl std::fmt::Display for InvalidDecisionQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InvalidDecisionQuery {}

/// Writes [`DecisionLogEntry`]s into the `decisions` table.
pub struct SqliteDecisionSink {
    pub session_manager: Arc<SessionManager>,
}

/// Install a [`SqliteDecisionSink`] over `session_manager` as the process's
/// decision log ([`super::global`]). The daemon calls it once at start and the
/// CLI once per session it builds. Calling it again replaces the sink, so it is
/// idempotent for the same store and follows the newest store otherwise (a
/// `--no-session` run's private store, whose rows then go with it).
pub fn install_global_sink(session_manager: Arc<SessionManager>) {
    super::global().set_sink(Arc::new(SqliteDecisionSink { session_manager }));
}

/// The 30 columns one row is written with, in bind order, after `INSERT` or
/// `INSERT OR IGNORE`. `?2` is the session id, bound once and read twice:
/// once as the column, once by the existence check, so a row is never stored
/// for a chat that is gone (or not yet made).
macro_rules! insert_decision_into {
    () => {
        "INTO decisions (\
            id, session_id, created_at, origin, origin_id, label, turn_id, batch_id, \
            chunk_index, chunk_id, provider, model, served_model, tier, adapter, calibrated, \
            questions_json, decisions_json, outcome, answer, pass_probability, \
            context_json, context_preview, context_bytes, context_truncated, error, \
            latency_ms, input_tokens, output_tokens, privacy_enforced) \
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30 \
          WHERE EXISTS (SELECT 1 FROM sessions WHERE id = ?2)"
    };
}

/// One row, refused by the primary key when its id is taken.
const INSERT_DECISION: &str = concat!("INSERT ", insert_decision_into!());

/// One row for a late answer, when its request wrote none (its caller went
/// away before the timeout row): a row that exists is left for
/// [`FILL_LATE_USAGE`].
const INSERT_DECISION_IF_ABSENT: &str = concat!("INSERT OR IGNORE ", insert_decision_into!());

/// A late answer's tokens, latency and sentence, onto the row its request
/// wrote without usage (the timeout row). A row that already holds tokens is
/// left alone, so a second delivery changes nothing.
const FILL_LATE_USAGE: &str = "UPDATE decisions \
        SET input_tokens = ?1, output_tokens = ?2, latency_ms = ?3, error = ?4 \
      WHERE id = ?5 AND session_id = ?6 \
        AND input_tokens IS NULL AND output_tokens IS NULL";

/// Every column a [`DecisionRecord`] is read from: all but `context_json`,
/// which only [`get`] reads.
const RECORD_COLUMNS: &str = "id, session_id, created_at, origin, origin_id, label, turn_id, \
     batch_id, chunk_index, chunk_id, provider, model, served_model, tier, adapter, calibrated, \
     questions_json, decisions_json, outcome, context_preview, context_bytes, context_truncated, \
     error, latency_ms, input_tokens, output_tokens, privacy_enforced";

#[async_trait]
impl DecisionSink for SqliteDecisionSink {
    async fn record(&self, entry: DecisionLogEntry) -> anyhow::Result<()> {
        let id = row_id(&entry);
        self.store(id, entry).await
    }

    async fn record_as(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
        let id = if id.trim().is_empty() {
            row_id(&entry)
        } else {
            id.to_string()
        };
        self.store(id, entry).await
    }

    /// A late or abandoned answer (see the module's Usage section): its row,
    /// or its tokens on the row its request already wrote, then its usage,
    /// billed once under the row's id. Nothing for a chat that is gone.
    async fn record_late(&self, id: &str, entry: DecisionLogEntry) -> anyhow::Result<()> {
        let Some(session_id) = entry.session_id.clone() else {
            return Ok(());
        };
        let id = if id.trim().is_empty() {
            row_id(&entry)
        } else {
            id.to_string()
        };
        self.insert(INSERT_DECISION_IF_ABSENT, &id, &session_id, &entry)
            .await?;
        let pool = self.session_manager.storage().pool().await?;
        sqlx::query(FILL_LATE_USAGE)
            .bind(entry.usage.input_tokens)
            .bind(entry.usage.output_tokens)
            .bind(entry.latency_ms.and_then(|ms| i64::try_from(ms).ok()))
            .bind(entry.error.as_deref())
            .bind(&id)
            .bind(&session_id)
            .execute(pool)
            .await?;
        // The ledger's own insert is gated on the chat existing and keyed by
        // the row, so a chat deleted meanwhile bills nothing and a request
        // billed already (or delivered twice) is not billed again.
        let Some(usage) = side_usage(&id, &session_id, &entry) else {
            return Ok(());
        };
        self.session_manager
            .apply_side_usage_event(usage)
            .await
            .map(|_| ())
            .with_context(|| {
                format!(
                    "decision {id}'s late answer was logged, but its usage could not be added \
                     to the usage ledger"
                )
            })
    }
}

impl SqliteDecisionSink {
    /// Store `entry` as the row `id`, beside its chat only.
    async fn store(&self, id: String, entry: DecisionLogEntry) -> anyhow::Result<()> {
        let Some(session_id) = entry.session_id.clone() else {
            tracing::debug!("an ad hoc decision with no chat is not stored in the decision log");
            return Ok(());
        };
        let inserted = self
            .insert(INSERT_DECISION, &id, &session_id, &entry)
            .await?;
        if inserted == 0 {
            tracing::debug!(
                session_id,
                "a decision for a chat that no longer exists was not stored"
            );
            return Ok(());
        }
        // A separate statement after the row, so a billing failure can never
        // lose the record of what was sent. Its error is returned, and the
        // service only warns: the decision itself still succeeds.
        let Some(usage) = side_usage(&id, &session_id, &entry) else {
            return Ok(());
        };
        self.session_manager
            .apply_side_usage_event(usage)
            .await
            .map(|_| ())
            .with_context(|| {
                format!(
                    "decision {id} was logged, but its usage could not be added to the usage \
                     ledger"
                )
            })
    }

    /// Run `sql` (an insert of `insert_decision_into!`'s shape) for `entry`
    /// as the row `id` of `session_id`; the rows it inserted.
    async fn insert(
        &self,
        sql: &'static str,
        id: &str,
        session_id: &str,
        entry: &DecisionLogEntry,
    ) -> anyhow::Result<u64> {
        let row = StoredRow::from_entry(entry);
        let pool = self.session_manager.storage().pool().await?;
        let inserted = sqlx::query(sql)
            .bind(id)
            .bind(session_id)
            .bind(Utc::now().timestamp_millis())
            .bind(sql_enum(&entry.origin))
            .bind(entry.origin_id.as_deref())
            .bind(entry.origin_label.as_deref())
            .bind(entry.turn_id.as_deref())
            .bind(entry.batch_id.as_deref())
            .bind(entry.chunk_index.and_then(|i| i64::try_from(i).ok()))
            .bind(entry.chunk_id.as_deref())
            .bind(entry.model.as_ref().map(|m| m.provider.as_str()))
            .bind(entry.model.as_ref().map(|m| m.model.as_str()))
            .bind(entry.served_model.as_deref())
            .bind(entry.tier.map(|tier| sql_enum(&tier)))
            .bind(row.adapter)
            .bind(row.calibrated)
            .bind(row.questions_json)
            .bind(row.decisions_json)
            .bind(sql_enum(&entry.outcome))
            .bind(row.answer)
            .bind(row.pass_probability)
            .bind(row.context_json)
            .bind(row.context_preview)
            .bind(i64::try_from(entry.context_bytes).unwrap_or(i64::MAX))
            .bind(row.context_truncated)
            .bind(entry.error.as_deref())
            .bind(entry.latency_ms.and_then(|ms| i64::try_from(ms).ok()))
            .bind(entry.usage.input_tokens)
            .bind(entry.usage.output_tokens)
            .bind(entry.privacy_enforced)
            .execute(pool)
            .await?
            .rows_affected();
        Ok(inserted)
    }
}

/// What the usage ledger takes for one stored decision row: its tokens, keyed
/// `decision:<row id>` so a retried write bills once, under the provider and
/// the model as requested (the price table is keyed by the requested Baseten
/// slug, not the served model's name). `None` when there is nothing to bill: no
/// model was reached (a refusal, no model), or the model reported no usage.
///
/// Only the decision providers are ledgered. The usage report tells decision
/// requests from model turns by the provider's name
/// (`is_decision_chat_provider`), so a decision recorded under a chat
/// provider's name would be counted as a turn. No adapter does that today
/// (every decision model opens under `baseten_decision` or `ollama_decision`);
/// one that ever does needs its own discriminator before it is ledgered.
fn side_usage(id: &str, session_id: &str, entry: &DecisionLogEntry) -> Option<SideUsageEntry> {
    let model = entry.model.as_ref()?;
    entry.usage.billed_total()?;
    if !crate::providers::decision::chat::is_decision_chat_provider(&model.provider) {
        tracing::debug!(
            provider = model.provider.as_str(),
            "a decision under a chat provider's name is not added to the usage ledger"
        );
        return None;
    }
    Some(SideUsageEntry {
        event_key: format!("decision:{id}"),
        session_id: session_id.to_string(),
        provider: model.provider.clone(),
        model_id: model.model.clone(),
        input_tokens: entry.usage.input_tokens,
        output_tokens: entry.usage.output_tokens,
    })
}

/// The id `entry` is stored under when its caller chose none: the request id
/// its decisions share (the service mints one per request, uuid v7, so ids
/// sort by time), or for a row with no decisions, which only a failure has, a
/// fresh uuid v7. A caller that chose the id first
/// ([`super::DecisionService::decide_as`]) has its row stored under that id
/// ([`DecisionSink::record_as`]), failure or not.
pub fn row_id(entry: &DecisionLogEntry) -> String {
    entry
        .decisions
        .first()
        .map(|decision| decision.request_id.clone())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string())
}

/// The derived and serialized parts of one row.
struct StoredRow {
    adapter: Option<String>,
    calibrated: Option<bool>,
    questions_json: String,
    decisions_json: String,
    answer: Option<String>,
    pass_probability: Option<f64>,
    context_json: String,
    context_preview: String,
    context_truncated: bool,
}

impl StoredRow {
    fn from_entry(entry: &DecisionLogEntry) -> Self {
        let (context_json, stored_context, context_truncated) = cap_context(&entry.context);
        Self {
            adapter: adapter_used(entry).map(|adapter| sql_enum(&adapter)),
            calibrated: (!entry.decisions.is_empty())
                .then(|| entry.decisions.iter().all(|d| d.calibrated)),
            questions_json: serde_json::to_string(&entry.questions)
                .unwrap_or_else(|_| "{}".to_string()),
            decisions_json: serde_json::to_string(&entry.decisions)
                .unwrap_or_else(|_| "[]".to_string()),
            answer: entry.decisions.first().map(readable_answer),
            pass_probability: lowest_pass_probability(&entry.decisions),
            context_preview: context_preview(&stored_context),
            context_json,
            context_truncated,
        }
    }
}

/// The adapter that produced the decisions, read off their probability source;
/// for a row with none (an error), the adapter the caller asked for, if any.
fn adapter_used(entry: &DecisionLogEntry) -> Option<DecisionAdapter> {
    entry
        .decisions
        .first()
        .map(|decision| match decision.source {
            ProbabilitySource::Native => DecisionAdapter::SystemOne,
            ProbabilitySource::Logprob { .. } => DecisionAdapter::Logprob,
            ProbabilitySource::Verbalized { .. } => DecisionAdapter::Verbalized,
        })
        .or_else(|| entry.model.as_ref().and_then(|model| model.adapter))
}

/// One decision's answer as a person reads it: `yes`, an option name, or a
/// score level's label (`level N` when the label is blank).
pub fn readable_answer(decision: &Decision) -> String {
    match &decision.answer {
        Answer::Binary { value } => if *value { "yes" } else { "no" }.to_string(),
        Answer::Choice { option } => option.clone(),
        Answer::Score { level, label, .. } => {
            if label.trim().is_empty() {
                format!("level {level}")
            } else {
                label.clone()
            }
        }
    }
}

/// The probability a decision is judged by: the one its threshold was compared
/// against, or for a binary question with no threshold, P(yes). `None` for a
/// choice or score question asked without a threshold, which has no single
/// probability of "passing".
pub fn decision_pass_probability(decision: &Decision) -> Option<f64> {
    decision.pass_probability.or_else(|| match decision.answer {
        Answer::Binary { .. } => decision.probabilities.get(YES).copied(),
        _ => None,
    })
}

/// The lowest [`decision_pass_probability`] in one request: the row's figure
/// for the probability filter, the side panel's bar and the gate chip, because
/// a request passes only when every thresholded question does.
pub fn lowest_pass_probability(decisions: &[Decision]) -> Option<f64> {
    decisions
        .iter()
        .filter_map(decision_pass_probability)
        .filter(|p| p.is_finite())
        .reduce(f64::min)
}

/// The context as stored: its JSON, the context that JSON holds, and whether it
/// had to be cut to fit [`MAX_STORED_CONTEXT_BYTES`].
fn cap_context(context: &DecisionContext) -> (String, DecisionContext, bool) {
    let full = to_json(context);
    if full.len() <= MAX_STORED_CONTEXT_BYTES {
        return (full, context.clone(), false);
    }
    let capped = match context {
        DecisionContext::Text(text) => fit_text(text, false, DecisionContext::Text),
        // Structured data cut in the middle is no longer data; it is kept as
        // the head of its text, and `context_truncated` says so.
        DecisionContext::Json(value) => {
            let text = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            fit_text(&text, false, DecisionContext::Text)
        }
        DecisionContext::Messages(messages) => fit_messages(messages),
    };
    (to_json(&capped), capped, true)
}

fn to_json(context: &DecisionContext) -> String {
    serde_json::to_string(context).unwrap_or_default()
}

/// The longest cut of `text` whose context fits, keeping its head (or, with
/// `keep_tail`, its end). JSON escaping can grow a string (a quote or a newline
/// doubles), so the cut is measured serialized, by a binary search over its
/// length in bytes: about 17 serializations of at most the cap, and only for a
/// context already over it.
fn fit_text(
    text: &str,
    keep_tail: bool,
    wrap: impl Fn(String) -> DecisionContext,
) -> DecisionContext {
    let cut = |bytes: usize| {
        wrap(if keep_tail {
            tail_within(text, bytes).to_string()
        } else {
            head_within(text, bytes).to_string()
        })
    };
    let fits = |context: &DecisionContext| to_json(context).len() <= MAX_STORED_CONTEXT_BYTES;
    let mut high = MAX_STORED_CONTEXT_BYTES.min(text.len());
    let widest = cut(high);
    if fits(&widest) {
        return widest;
    }
    // `low` always fits (an empty cut is a few bytes of JSON); `high` never does.
    let mut low = 0;
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        if fits(&cut(middle)) {
            low = middle;
        } else {
            high = middle;
        }
    }
    cut(low)
}

/// The newest messages that fit; when even the newest alone does not, its
/// content's end. A monitor's window is newest-last, so the end is what it was
/// asked about.
fn fit_messages(messages: &[ContextMessage]) -> DecisionContext {
    let Some(last) = messages.last() else {
        return DecisionContext::Messages(Vec::new());
    };
    let wrapper = to_json(&DecisionContext::Messages(Vec::new())).len();
    let sizes: Vec<usize> = messages
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
        .collect();
    let mut total = wrapper + sizes.iter().sum::<usize>() + sizes.len().saturating_sub(1);
    let mut start = 0;
    while total > MAX_STORED_CONTEXT_BYTES && start + 1 < messages.len() {
        total -= sizes[start] + 1;
        start += 1;
    }
    if total <= MAX_STORED_CONTEXT_BYTES {
        return DecisionContext::Messages(messages[start..].to_vec());
    }
    let role = last.role.clone();
    fit_text(&last.content, true, move |content| {
        DecisionContext::Messages(vec![ContextMessage {
            role: role.clone(),
            content,
        }])
    })
}

/// The longest prefix of `text` of at most `bytes` bytes, on a char boundary.
fn head_within(text: &str, bytes: usize) -> &str {
    if text.len() <= bytes {
        return text;
    }
    let mut end = bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

/// The longest suffix of `text` of at most `bytes` bytes, on a char boundary.
fn tail_within(text: &str, bytes: usize) -> &str {
    if text.len() <= bytes {
        return text;
    }
    let mut start = text.len() - bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text.get(start..).unwrap_or_default()
}

/// At most [`CONTEXT_PREVIEW_CHARS`] characters of the stored context: the
/// start of a text or JSON state, the end of a conversation window (the newest
/// part is what a monitor was asked about). A cut is marked with `…`.
fn context_preview(context: &DecisionContext) -> String {
    let text = context.render_text();
    let chars = text.chars().count();
    if chars <= CONTEXT_PREVIEW_CHARS {
        return text;
    }
    let keep = CONTEXT_PREVIEW_CHARS - 1;
    match context {
        DecisionContext::Messages(_) => {
            let tail: String = text.chars().skip(chars - keep).collect();
            format!("…{tail}")
        }
        _ => {
            let head: String = text.chars().take(keep).collect();
            format!("{head}…")
        }
    }
}

/// A unit enum's serde spelling (`monitor`, `passed`, `system_one`, `private`),
/// which is what the table stores and the HTTP layer speaks.
fn sql_enum<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(text)) => text,
        _ => String::new(),
    }
}

fn parse_enum<T: DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_value(Value::String(text.to_string())).ok()
}

fn millis_to_time(ms: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default()
}

fn count(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

/// What a surface calls an origin whose rows carry no label.
fn default_label(origin: DecisionOrigin, origin_id: Option<&str>) -> String {
    match (origin, origin_id) {
        (DecisionOrigin::Chat, _) => "Decision chat".to_string(),
        (DecisionOrigin::Monitor, Some(id)) => format!("Monitor {id}"),
        (DecisionOrigin::Monitor, None) => "Monitor".to_string(),
        (DecisionOrigin::Tool, _) => "decide".to_string(),
        (DecisionOrigin::Gate, Some(id)) => format!("Gate on phase {id}"),
        (DecisionOrigin::Gate, None) => "Phase gate".to_string(),
        (DecisionOrigin::Adhoc, _) => "Ad hoc".to_string(),
    }
}

fn model_ref(
    provider: Option<String>,
    model: Option<String>,
    adapter: Option<String>,
) -> Option<DecisionModelRef> {
    let (provider, model) = provider.zip(model)?;
    Some(DecisionModelRef {
        provider,
        model,
        adapter: adapter.as_deref().and_then(parse_enum),
    })
}

fn record_from_row(row: &SqliteRow) -> anyhow::Result<DecisionRecord> {
    let origin_text: String = row.try_get("origin")?;
    let outcome_text: String = row.try_get("outcome")?;
    let questions_json: String = row.try_get("questions_json")?;
    let decisions_json: Option<String> = row.try_get("decisions_json")?;
    let id: String = row.try_get("id")?;
    let decisions = match decisions_json.as_deref() {
        None | Some("") => Vec::new(),
        Some(json) => serde_json::from_str(json).unwrap_or_else(|error| {
            tracing::warn!(%error, decision_id = %id, "a stored decision could not be read");
            Vec::new()
        }),
    };
    Ok(DecisionRecord {
        session_id: row.try_get("session_id")?,
        created_at: millis_to_time(row.try_get("created_at")?),
        origin: parse_enum(&origin_text).unwrap_or_default(),
        origin_id: row.try_get("origin_id")?,
        label: row.try_get("label")?,
        turn_id: row.try_get("turn_id")?,
        batch_id: row.try_get("batch_id")?,
        chunk_index: row
            .try_get::<Option<i64>, _>("chunk_index")?
            .and_then(|i| u32::try_from(i).ok()),
        chunk_id: row.try_get("chunk_id")?,
        model: model_ref(
            row.try_get("provider")?,
            row.try_get("model")?,
            row.try_get("adapter")?,
        ),
        served_model: row.try_get("served_model")?,
        tier: row.try_get("tier")?,
        calibrated: row.try_get("calibrated")?,
        questions: serde_json::from_str(&questions_json).unwrap_or(Value::Null),
        decisions,
        outcome: parse_enum(&outcome_text).unwrap_or(DecisionOutcome::Error),
        context_preview: row
            .try_get::<Option<String>, _>("context_preview")?
            .unwrap_or_default(),
        context_bytes: row
            .try_get::<Option<i64>, _>("context_bytes")?
            .map(count)
            .unwrap_or(0),
        context_truncated: row.try_get("context_truncated")?,
        error: row.try_get("error")?,
        latency_ms: row
            .try_get::<Option<i64>, _>("latency_ms")?
            .and_then(|ms| u64::try_from(ms).ok()),
        input_tokens: row.try_get("input_tokens")?,
        output_tokens: row.try_get("output_tokens")?,
        privacy_enforced: row.try_get("privacy_enforced")?,
        id,
    })
}

/// A page cursor: the newest-first sort key of the last row a page returned.
fn encode_cursor(record: &DecisionRecord) -> String {
    format!("{}:{}", record.created_at.timestamp_millis(), record.id)
}

fn decode_cursor(cursor: &str) -> Result<(i64, String), InvalidDecisionQuery> {
    cursor
        .split_once(':')
        .and_then(|(at, id)| Some((at.parse::<i64>().ok()?, id)))
        .filter(|(_, id)| !id.is_empty())
        .map(|(at, id)| (at, id.to_string()))
        .ok_or_else(|| {
            InvalidDecisionQuery(
                "That decision page cursor was not issued by this server; start again from the \
                 first page."
                    .to_string(),
            )
        })
}

fn check_probability(name: &str, value: Option<f64>) -> Result<(), InvalidDecisionQuery> {
    match value {
        Some(p) if !p.is_finite() || !(0.0..=1.0).contains(&p) => Err(InvalidDecisionQuery(
            format!("`{name}` must be a probability between 0 and 1."),
        )),
        _ => Ok(()),
    }
}

/// The `WHERE` clause every listing of one chat's decisions shares.
fn push_filters(builder: &mut QueryBuilder<'_, Sqlite>, session_id: &str, query: &DecisionQuery) {
    builder
        .push(" WHERE session_id = ")
        .push_bind(session_id.to_string());
    if let Some(decision_id) = &query.decision_id {
        builder.push(" AND id = ").push_bind(decision_id.clone());
    }
    if let Some(origin) = query.origin {
        builder.push(" AND origin = ").push_bind(sql_enum(&origin));
    }
    if let Some(origin_id) = &query.origin_id {
        builder
            .push(" AND origin_id = ")
            .push_bind(origin_id.clone());
    }
    if let Some(turn_id) = &query.turn_id {
        builder.push(" AND turn_id = ").push_bind(turn_id.clone());
    }
    if let Some(batch_id) = &query.batch_id {
        builder.push(" AND batch_id = ").push_bind(batch_id.clone());
    }
    if let Some(outcome) = query.outcome {
        builder
            .push(" AND outcome = ")
            .push_bind(sql_enum(&outcome));
    }
    if query.errors_only {
        builder.push(" AND outcome IN ('error', 'refused')");
    }
    if let Some(min_p) = query.min_p {
        builder.push(" AND pass_probability >= ").push_bind(min_p);
    }
    if let Some(max_p) = query.max_p {
        builder.push(" AND pass_probability <= ").push_bind(max_p);
    }
}

/// One page of a chat's decisions, newest first.
///
/// A row's pass probability is [`lowest_pass_probability`] of its decisions, so
/// `minP`/`maxP` drop rows that have none (an error, or a choice asked with no
/// threshold). `errorsOnly` keeps `error` and `refused` rows. A malformed query
/// or cursor is an [`InvalidDecisionQuery`].
pub async fn list(
    session_manager: &SessionManager,
    session_id: &str,
    query: &DecisionQuery,
) -> anyhow::Result<DecisionPage> {
    check_probability("minP", query.min_p)?;
    check_probability("maxP", query.max_p)?;
    let cursor = query.before.as_deref().map(decode_cursor).transpose()?;
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let pool = session_manager.storage().pool().await?;

    let mut counting = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM decisions");
    push_filters(&mut counting, session_id, query);
    let total: i64 = counting.build_query_scalar().fetch_one(pool).await?;

    let mut paging = QueryBuilder::<Sqlite>::new(format!("SELECT {RECORD_COLUMNS} FROM decisions"));
    push_filters(&mut paging, session_id, query);
    if let Some((at, id)) = cursor {
        paging
            .push(" AND (created_at < ")
            .push_bind(at)
            .push(" OR (created_at = ")
            .push_bind(at)
            .push(" AND id < ")
            .push_bind(id)
            .push("))");
    }
    paging
        .push(" ORDER BY created_at DESC, id DESC LIMIT ")
        .push_bind(i64::from(limit) + 1);
    let rows = paging.build().fetch_all(pool).await?;

    let mut records = rows
        .iter()
        .map(record_from_row)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let next_cursor = if records.len() > limit as usize {
        records.truncate(limit as usize);
        records.last().map(encode_cursor)
    } else {
        None
    };
    Ok(DecisionPage {
        records,
        next_cursor,
        total: count(total),
    })
}

/// One decision with its stored context, or `None` when the chat has no such row.
pub async fn get(
    session_manager: &SessionManager,
    session_id: &str,
    decision_id: &str,
) -> anyhow::Result<Option<DecisionDetail>> {
    let pool = session_manager.storage().pool().await?;
    let row = sqlx::query(&format!(
        "SELECT {RECORD_COLUMNS}, context_json FROM decisions WHERE session_id = ?1 AND id = ?2"
    ))
    .bind(session_id)
    .bind(decision_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let record = record_from_row(&row)?;
    let stored: Option<String> = row.try_get("context_json")?;
    let context = match stored {
        None => DecisionContext::Text(String::new()),
        Some(json) => serde_json::from_str(&json).unwrap_or(DecisionContext::Text(json)),
    };
    Ok(Some(DecisionDetail { record, context }))
}

// ⚠ What one aggregate is. A monitor is one source per monitor id and a gate
// one per phase id, but a `decide` call's origin id is its own batch id, so
// grouping tool rows by origin id would list one source per batch: a chat that
// called `decide` forty times would show forty rows. Tool rows are therefore
// grouped by label (one "decide" source per chat) and report no origin id; the
// history still filters one batch by `batchId`. `SOURCE_KEY` and
// `SOURCE_ORIGIN_ID` are spliced into every aggregate below, so the summary,
// the recent turns and the turn marker group the same way.
//
// `SOURCE_KEY` is what rows are partitioned by beside `origin`.
macro_rules! source_key {
    ($t:literal) => {
        concat!(
            "CASE WHEN ",
            $t,
            "origin = 'tool' THEN ",
            $t,
            "label ELSE ",
            $t,
            "origin_id END"
        )
    };
}
// The origin id an aggregate reports: none for the tool's, which spans batches.
macro_rules! source_origin_id {
    ($t:literal) => {
        concat!(
            "CASE WHEN ",
            $t,
            "origin = 'tool' THEN NULL ELSE ",
            $t,
            "origin_id END"
        )
    };
}

/// Each source's calls, with its newest row's label, answer, probability and
/// model. Window functions, so the newest row is chosen by `(created_at, id)`
/// and never by which row SQLite happened to visit last.
const AGGREGATES: &str = concat!(
    "SELECT origin, origin_id, label, answer, pass_probability, provider, model, adapter, \
            created_at, n, passed, failed, errors \
       FROM (SELECT origin, ",
    source_origin_id!(""),
    " AS origin_id, label, answer, pass_probability, provider, model, adapter, \
                    created_at, \
                    COUNT(*) OVER g AS n, \
                    SUM(outcome = 'passed') OVER g AS passed, \
                    SUM(outcome = 'failed') OVER g AS failed, \
                    SUM(outcome IN ('error', 'refused')) OVER g AS errors, \
                    ROW_NUMBER() OVER (PARTITION BY origin, ",
    source_key!(""),
    " ORDER BY created_at DESC, id DESC) AS rn \
               FROM decisions \
              WHERE session_id = ?1 \
             WINDOW g AS (PARTITION BY origin, ",
    source_key!(""),
    ")) \
      WHERE rn = 1 \
      ORDER BY created_at DESC, origin, origin_id"
);

/// The newest `?2` turns, each source's calls within each, newest turn first.
const RECENT_TURN_SUMMARIES: &str = concat!(
    "WITH recent AS (SELECT turn_id, MAX(created_at) AS turn_last \
                       FROM decisions \
                      WHERE session_id = ?1 AND turn_id IS NOT NULL \
                      GROUP BY turn_id \
                      ORDER BY turn_last DESC, turn_id DESC \
                      LIMIT ?2) \
     SELECT turn_id, origin, origin_id, label, pass_probability, turn_last, \
            n, passed, failed, errors, first_at, last_at \
       FROM (SELECT d.turn_id, d.origin, ",
    source_origin_id!("d."),
    " AS origin_id, d.label, d.pass_probability, r.turn_last, \
                    COUNT(*) OVER g AS n, \
                    SUM(d.outcome = 'passed') OVER g AS passed, \
                    SUM(d.outcome = 'failed') OVER g AS failed, \
                    SUM(d.outcome IN ('error', 'refused')) OVER g AS errors, \
                    MIN(d.created_at) OVER g AS first_at, \
                    MAX(d.created_at) OVER g AS last_at, \
                    ROW_NUMBER() OVER (PARTITION BY d.turn_id, d.origin, ",
    source_key!("d."),
    " ORDER BY d.created_at DESC, d.id DESC) AS rn \
               FROM decisions d JOIN recent r ON r.turn_id = d.turn_id \
              WHERE d.session_id = ?1 \
             WINDOW g AS (PARTITION BY d.turn_id, d.origin, ",
    source_key!("d."),
    ")) \
      WHERE rn = 1 \
      ORDER BY turn_last DESC, turn_id DESC, first_at ASC, origin, origin_id"
);

/// One turn's calls, each source's aggregated.
const TURN_MARKER: &str = concat!(
    "SELECT origin, origin_id, label, pass_probability, n, passed, failed, errors \
       FROM (SELECT origin, ",
    source_origin_id!(""),
    " AS origin_id, label, pass_probability, created_at, \
                    COUNT(*) OVER g AS n, \
                    SUM(outcome = 'passed') OVER g AS passed, \
                    SUM(outcome = 'failed') OVER g AS failed, \
                    SUM(outcome IN ('error', 'refused')) OVER g AS errors, \
                    MIN(created_at) OVER g AS first_at, \
                    ROW_NUMBER() OVER (PARTITION BY origin, ",
    source_key!(""),
    " ORDER BY created_at DESC, id DESC) AS rn \
               FROM decisions \
              WHERE session_id = ?1 AND turn_id = ?2 \
             WINDOW g AS (PARTITION BY origin, ",
    source_key!(""),
    ")) \
      WHERE rn = 1 \
      ORDER BY first_at ASC, origin, origin_id"
);

/// A chat's decisions aggregated by origin and by recent turn.
///
/// `errors` counts `error` and `refused` calls; `passed` and `failed` count
/// calls whose thresholds all passed, or one failed. A call with no threshold
/// (`decided`) is in `count` only. The last answer and probability are the
/// newest call's, so a source whose newest call failed shows no answer rather
/// than an older one.
pub async fn summary(
    session_manager: &SessionManager,
    session_id: &str,
) -> anyhow::Result<SessionDecisionSummary> {
    let pool = session_manager.storage().pool().await?;
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions WHERE session_id = ?1")
        .bind(session_id)
        .fetch_one(pool)
        .await?;

    let aggregates = sqlx::query(AGGREGATES)
        .bind(session_id)
        .fetch_all(pool)
        .await?
        .iter()
        .map(|row| -> anyhow::Result<DecisionAggregate> {
            let origin_text: String = row.try_get("origin")?;
            let origin: DecisionOrigin = parse_enum(&origin_text).unwrap_or_default();
            let origin_id: Option<String> = row.try_get("origin_id")?;
            let label: Option<String> = row.try_get("label")?;
            Ok(DecisionAggregate {
                label: label.unwrap_or_else(|| default_label(origin, origin_id.as_deref())),
                count: count(row.try_get("n")?),
                passed: count(row.try_get("passed")?),
                failed: count(row.try_get("failed")?),
                errors: count(row.try_get("errors")?),
                last_at: millis_to_time(row.try_get("created_at")?),
                last_answer: row.try_get("answer")?,
                last_pass_probability: row.try_get("pass_probability")?,
                model: model_ref(
                    row.try_get("provider")?,
                    row.try_get("model")?,
                    row.try_get("adapter")?,
                ),
                origin,
                origin_id,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let recent_turns = sqlx::query(RECENT_TURN_SUMMARIES)
        .bind(session_id)
        .bind(i64::try_from(RECENT_TURNS).unwrap_or(i64::MAX))
        .fetch_all(pool)
        .await?
        .iter()
        .map(|row| -> anyhow::Result<DecisionTurnSummary> {
            let origin_text: String = row.try_get("origin")?;
            let origin: DecisionOrigin = parse_enum(&origin_text).unwrap_or_default();
            let origin_id: Option<String> = row.try_get("origin_id")?;
            let label: Option<String> = row.try_get("label")?;
            Ok(DecisionTurnSummary {
                turn_id: row.try_get("turn_id")?,
                label: label.unwrap_or_else(|| default_label(origin, origin_id.as_deref())),
                count: count(row.try_get("n")?),
                passed: count(row.try_get("passed")?),
                failed: count(row.try_get("failed")?),
                errors: count(row.try_get("errors")?),
                first_at: millis_to_time(row.try_get("first_at")?),
                last_at: millis_to_time(row.try_get("last_at")?),
                last_pass_probability: row.try_get("pass_probability")?,
                origin,
                origin_id,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(SessionDecisionSummary {
        total: count(total),
        aggregates,
        recent_turns,
    })
}

/// One turn's calls as the transcript marker carries them, aggregated per
/// origin. `fired` is left `false`; the monitor runner, which knows which of
/// its monitors fired, sets it. Empty when the turn made no decision.
pub async fn turn_marker(
    session_manager: &SessionManager,
    session_id: &str,
    turn_id: &str,
) -> anyhow::Result<DecisionTurnMarker> {
    let pool = session_manager.storage().pool().await?;
    let rows = sqlx::query(TURN_MARKER)
        .bind(session_id)
        .bind(turn_id)
        .fetch_all(pool)
        .await?;
    let items = rows
        .iter()
        .map(|row| -> anyhow::Result<DecisionTurnMarkerItem> {
            let origin_text: String = row.try_get("origin")?;
            let origin: DecisionOrigin = parse_enum(&origin_text).unwrap_or_default();
            let origin_id: Option<String> = row.try_get("origin_id")?;
            let label: Option<String> = row.try_get("label")?;
            Ok(DecisionTurnMarkerItem {
                label: label.unwrap_or_else(|| default_label(origin, origin_id.as_deref())),
                count: count(row.try_get("n")?),
                passed: count(row.try_get("passed")?),
                failed: count(row.try_get("failed")?),
                errors: count(row.try_get("errors")?),
                last_pass_probability: row.try_get("pass_probability")?,
                fired: false,
                origin,
                origin_id,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(DecisionTurnMarker {
        turn_id: turn_id.to_string(),
        items,
    })
}

/// Decision counts for several chats at once (History's per-chat count). One
/// grouped query, however many ids. Every id asked for is in the map, with 0
/// for a chat that made no decision.
pub async fn count_for_sessions(
    session_manager: &SessionManager,
    session_ids: &[String],
) -> anyhow::Result<HashMap<String, u64>> {
    let mut counts: HashMap<String, u64> = session_ids.iter().map(|id| (id.clone(), 0)).collect();
    if session_ids.is_empty() {
        return Ok(counts);
    }
    let pool = session_manager.storage().pool().await?;
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT session_id, COUNT(*) FROM decisions \
          WHERE session_id IN (SELECT value FROM json_each(?1)) \
          GROUP BY session_id",
    )
    .bind(serde_json::to_string(session_ids)?)
    .fetch_all(pool)
    .await?;
    for (session_id, n) in rows {
        counts.insert(session_id, count(n));
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::types::{DecisionKind, DecisionUsage, Question, Threshold};
    use crate::privacy::ProviderTier;
    use crate::session::session_manager::SessionType;
    use indexmap::IndexMap;
    use tempfile::TempDir;

    fn binary_question(threshold: Option<f64>) -> Question {
        Question {
            instructions: "Does the note state multiple sclerosis?".to_string(),
            kind: DecisionKind::Binary {},
            threshold: threshold.map(Threshold::Yes),
        }
    }

    fn decision(question: &str, p_yes: f64, threshold: Option<f64>) -> Decision {
        let mut probabilities = IndexMap::new();
        probabilities.insert("yes".to_string(), p_yes);
        probabilities.insert("no".to_string(), 1.0 - p_yes);
        Decision {
            question: question.to_string(),
            answer: Answer::Binary {
                value: p_yes >= 0.5,
            },
            probabilities,
            confidence: (2.0 * p_yes - 1.0).abs(),
            passed: threshold.map(|t| p_yes >= t),
            pass_probability: threshold.map(|_| p_yes),
            source: ProbabilitySource::Native,
            calibrated: true,
            model: DecisionModelRef::new("ollama_decision", "tev1:0.8b"),
            tier: ProviderTier::Private,
            // One request per decision, minted as the service mints them: the
            // row is stored under this id.
            request_id: uuid::Uuid::now_v7().to_string(),
            latency_ms: 22,
            usage: DecisionUsage {
                input_tokens: Some(120),
                output_tokens: Some(2),
            },
            raw: Value::Null,
        }
    }

    struct Entry {
        origin: DecisionOrigin,
        origin_id: Option<&'static str>,
        label: Option<&'static str>,
        turn: Option<&'static str>,
        batch: Option<&'static str>,
        outcome: DecisionOutcome,
        p_yes: Option<f64>,
        threshold: Option<f64>,
        context: DecisionContext,
    }

    impl Default for Entry {
        fn default() -> Self {
            Self {
                origin: DecisionOrigin::Monitor,
                origin_id: Some("form-reached"),
                label: Some("Form reached"),
                turn: Some("t1"),
                batch: None,
                outcome: DecisionOutcome::Failed,
                p_yes: Some(0.31),
                threshold: Some(0.85),
                context: DecisionContext::Text("the agent is on the login page".to_string()),
            }
        }
    }

    fn log_entry(session_id: Option<&str>, e: Entry) -> DecisionLogEntry {
        let mut questions = IndexMap::new();
        questions.insert("has_ms".to_string(), binary_question(e.threshold));
        let decisions = e
            .p_yes
            .map(|p| vec![decision("has_ms", p, e.threshold)])
            .unwrap_or_default();
        let context_bytes = e.context.byte_len();
        DecisionLogEntry {
            session_id: session_id.map(str::to_string),
            origin: e.origin,
            origin_id: e.origin_id.map(str::to_string),
            origin_label: e.label.map(str::to_string),
            turn_id: e.turn.map(str::to_string),
            batch_id: e.batch.map(str::to_string),
            chunk_index: e.batch.map(|_| 3),
            chunk_id: e.batch.map(|_| "note-0004".to_string()),
            model: Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b")),
            served_model: Some("tev1:0.8b".to_string()),
            tier: Some(ProviderTier::Private),
            questions,
            context: e.context,
            context_bytes,
            outcome: e.outcome,
            error: matches!(e.outcome, DecisionOutcome::Error | DecisionOutcome::Refused)
                .then(|| "the model is not loaded".to_string()),
            decisions,
            latency_ms: Some(22),
            usage: DecisionUsage {
                input_tokens: Some(120),
                output_tokens: Some(2),
            },
            privacy_enforced: true,
        }
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

    async fn write(sm: &Arc<SessionManager>, entry: DecisionLogEntry) {
        SqliteDecisionSink {
            session_manager: Arc::clone(sm),
        }
        .record(entry)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_row_round_trips_through_list_and_get() {
        let (_dir, sm, id) = store().await;
        write(&sm, log_entry(Some(&id), Entry::default())).await;

        let page = list(&sm, &id, &DecisionQuery::default()).await.unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.next_cursor, None);
        let record = &page.records[0];
        assert_eq!(record.session_id.as_deref(), Some(id.as_str()));
        assert_eq!(record.origin, DecisionOrigin::Monitor);
        assert_eq!(record.origin_id.as_deref(), Some("form-reached"));
        assert_eq!(record.label.as_deref(), Some("Form reached"));
        assert_eq!(record.turn_id.as_deref(), Some("t1"));
        assert_eq!(record.outcome, DecisionOutcome::Failed);
        assert_eq!(record.tier.as_deref(), Some("private"));
        assert_eq!(record.calibrated, Some(true));
        assert_eq!(record.decisions.len(), 1);
        assert_eq!(record.decisions[0].pass_probability, Some(0.31));
        assert_eq!(
            record.model,
            Some(
                DecisionModelRef::new("ollama_decision", "tev1:0.8b")
                    .with_adapter(DecisionAdapter::SystemOne)
            )
        );
        assert_eq!(
            record.questions["has_ms"]["kind"],
            serde_json::json!({"binary": {}})
        );
        assert_eq!(record.context_preview, "the agent is on the login page");
        assert!(!record.context_truncated);
        assert_eq!(record.latency_ms, Some(22));
        assert_eq!(record.input_tokens, Some(120));
        assert!(record.privacy_enforced);

        let detail = get(&sm, &id, &record.id).await.unwrap().unwrap();
        assert_eq!(&detail.record, record);
        assert_eq!(
            detail.context,
            DecisionContext::Text("the agent is on the login page".to_string())
        );
        assert_eq!(get(&sm, "someone-else", &record.id).await.unwrap(), None);
        assert_eq!(get(&sm, &id, "no-such-row").await.unwrap(), None);
    }

    #[tokio::test]
    async fn no_row_is_stored_without_a_live_chat() {
        let (_dir, sm, id) = store().await;
        write(&sm, log_entry(None, Entry::default())).await;
        write(&sm, log_entry(Some("19990101_1"), Entry::default())).await;
        let pool = sm.storage().pool().await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(rows, 0, "an ad hoc or orphaned decision was stored");
        assert_eq!(
            list(&sm, &id, &DecisionQuery::default())
                .await
                .unwrap()
                .total,
            0
        );
    }

    #[tokio::test]
    async fn pages_walk_every_row_newest_first_without_repeats() {
        let (_dir, sm, id) = store().await;
        for _ in 0..7 {
            write(&sm, log_entry(Some(&id), Entry::default())).await;
        }
        let mut seen = Vec::new();
        let mut query = DecisionQuery {
            limit: Some(3),
            ..DecisionQuery::default()
        };
        loop {
            let page = list(&sm, &id, &query).await.unwrap();
            assert_eq!(page.total, 7, "total counts every page");
            assert!(page.records.len() <= 3);
            seen.extend(page.records.iter().map(|r| (r.created_at, r.id.clone())));
            match page.next_cursor {
                Some(cursor) => query.before = Some(cursor),
                None => break,
            }
        }
        assert_eq!(seen.len(), 7);
        let mut sorted = seen.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(seen, sorted, "pages are newest first by (created_at, id)");
        sorted.dedup();
        assert_eq!(sorted.len(), 7, "no row appears on two pages");
    }

    #[tokio::test]
    async fn a_bad_cursor_or_probability_is_a_query_error() {
        let (_dir, sm, id) = store().await;
        for query in [
            DecisionQuery {
                before: Some("not-a-cursor".to_string()),
                ..DecisionQuery::default()
            },
            DecisionQuery {
                min_p: Some(1.5),
                ..DecisionQuery::default()
            },
            DecisionQuery {
                max_p: Some(f64::NAN),
                ..DecisionQuery::default()
            },
        ] {
            let error = list(&sm, &id, &query).await.unwrap_err();
            assert!(
                error.downcast_ref::<InvalidDecisionQuery>().is_some(),
                "{error:#}"
            );
        }
    }

    #[tokio::test]
    async fn filters_narrow_the_page_and_the_total() {
        let (_dir, sm, id) = store().await;
        write(&sm, log_entry(Some(&id), Entry::default())).await;
        write(
            &sm,
            log_entry(
                Some(&id),
                Entry {
                    outcome: DecisionOutcome::Passed,
                    p_yes: Some(0.93),
                    turn: Some("t2"),
                    ..Entry::default()
                },
            ),
        )
        .await;
        write(
            &sm,
            log_entry(
                Some(&id),
                Entry {
                    origin: DecisionOrigin::Tool,
                    origin_id: Some("call_9"),
                    label: Some("decide"),
                    batch: Some("b1"),
                    outcome: DecisionOutcome::Decided,
                    p_yes: Some(0.6),
                    threshold: None,
                    ..Entry::default()
                },
            ),
        )
        .await;
        write(
            &sm,
            log_entry(
                Some(&id),
                Entry {
                    outcome: DecisionOutcome::Error,
                    p_yes: None,
                    ..Entry::default()
                },
            ),
        )
        .await;
        write(
            &sm,
            log_entry(
                Some(&id),
                Entry {
                    origin: DecisionOrigin::Gate,
                    origin_id: Some("phase-2"),
                    outcome: DecisionOutcome::Refused,
                    p_yes: None,
                    ..Entry::default()
                },
            ),
        )
        .await;

        let total = |query: DecisionQuery| {
            let sm = Arc::clone(&sm);
            let id = id.clone();
            async move {
                let page = list(&sm, &id, &query).await.unwrap();
                assert_eq!(page.total as usize, page.records.len());
                page.total
            }
        };
        assert_eq!(total(DecisionQuery::default()).await, 5);
        assert_eq!(
            total(DecisionQuery {
                origin: Some(DecisionOrigin::Monitor),
                ..DecisionQuery::default()
            })
            .await,
            3
        );
        assert_eq!(
            total(DecisionQuery {
                origin_id: Some("call_9".into()),
                ..DecisionQuery::default()
            })
            .await,
            1
        );
        assert_eq!(
            total(DecisionQuery {
                turn_id: Some("t2".into()),
                ..DecisionQuery::default()
            })
            .await,
            1
        );
        assert_eq!(
            total(DecisionQuery {
                batch_id: Some("b1".into()),
                ..DecisionQuery::default()
            })
            .await,
            1
        );
        assert_eq!(
            total(DecisionQuery {
                outcome: Some(DecisionOutcome::Passed),
                ..DecisionQuery::default()
            })
            .await,
            1
        );
        assert_eq!(
            total(DecisionQuery {
                errors_only: true,
                ..DecisionQuery::default()
            })
            .await,
            2,
            "errors and refusals"
        );
        // 0.31, 0.93, and 0.6 (P(yes) of an unthresholded yes/no question).
        assert_eq!(
            total(DecisionQuery {
                min_p: Some(0.5),
                ..DecisionQuery::default()
            })
            .await,
            2
        );
        assert_eq!(
            total(DecisionQuery {
                min_p: Some(0.4),
                max_p: Some(0.7),
                ..DecisionQuery::default()
            })
            .await,
            1
        );
    }

    #[tokio::test]
    async fn the_summary_aggregates_per_origin_and_per_turn_never_per_call() {
        let (_dir, sm, id) = store().await;
        // Turn t1: three monitor checks and a two-chunk decide batch.
        for p in [0.2, 0.4, 0.31] {
            write(
                &sm,
                log_entry(
                    Some(&id),
                    Entry {
                        p_yes: Some(p),
                        ..Entry::default()
                    },
                ),
            )
            .await;
        }
        for _ in 0..2 {
            write(
                &sm,
                log_entry(
                    Some(&id),
                    Entry {
                        origin: DecisionOrigin::Tool,
                        origin_id: Some("call_1"),
                        label: None,
                        batch: Some("b1"),
                        outcome: DecisionOutcome::Passed,
                        p_yes: Some(0.95),
                        ..Entry::default()
                    },
                ),
            )
            .await;
        }
        // Turn t2: the monitor fires, then errors.
        write(
            &sm,
            log_entry(
                Some(&id),
                Entry {
                    turn: Some("t2"),
                    outcome: DecisionOutcome::Passed,
                    p_yes: Some(0.91),
                    ..Entry::default()
                },
            ),
        )
        .await;
        write(
            &sm,
            log_entry(
                Some(&id),
                Entry {
                    turn: Some("t2"),
                    outcome: DecisionOutcome::Error,
                    p_yes: None,
                    ..Entry::default()
                },
            ),
        )
        .await;

        let summary = summary(&sm, &id).await.unwrap();
        assert_eq!(summary.total, 7);
        assert_eq!(
            summary.aggregates.len(),
            2,
            "one row per source, not per call"
        );
        let monitor = summary
            .aggregates
            .iter()
            .find(|a| a.origin == DecisionOrigin::Monitor)
            .unwrap();
        assert_eq!(monitor.label, "Form reached");
        assert_eq!(
            (
                monitor.count,
                monitor.passed,
                monitor.failed,
                monitor.errors
            ),
            (5, 1, 3, 1)
        );
        assert_eq!(monitor.last_answer, None, "the newest call errored");
        assert_eq!(monitor.last_pass_probability, None);
        assert_eq!(
            summary.aggregates[0].origin,
            DecisionOrigin::Monitor,
            "newest source first"
        );
        let tool = summary
            .aggregates
            .iter()
            .find(|a| a.origin == DecisionOrigin::Tool)
            .unwrap();
        assert_eq!(
            tool.label, "decide",
            "a row with no label takes the origin's name"
        );
        assert_eq!((tool.count, tool.passed), (2, 2));
        assert_eq!(tool.last_answer.as_deref(), Some("yes"));
        assert_eq!(tool.last_pass_probability, Some(0.95));
        assert_eq!(tool.model.as_ref().unwrap().model, "tev1:0.8b");

        let turns: Vec<(&str, DecisionOrigin, u64)> = summary
            .recent_turns
            .iter()
            .map(|t| (t.turn_id.as_str(), t.origin, t.count))
            .collect();
        assert_eq!(
            turns,
            vec![
                ("t2", DecisionOrigin::Monitor, 2),
                ("t1", DecisionOrigin::Monitor, 3),
                ("t1", DecisionOrigin::Tool, 2),
            ]
        );
        let t1_monitor = &summary.recent_turns[1];
        assert_eq!(t1_monitor.last_pass_probability, Some(0.31));
        assert!(t1_monitor.first_at <= t1_monitor.last_at);

        let marker = turn_marker(&sm, &id, "t1").await.unwrap();
        assert_eq!(marker.items.len(), 2);
        assert_eq!(marker.items[0].origin, DecisionOrigin::Monitor);
        assert_eq!(marker.items[0].count, 3);
        assert_eq!(marker.items[1].label, "decide");
        assert!(turn_marker(&sm, &id, "t9").await.unwrap().items.is_empty());
    }

    #[tokio::test]
    async fn recent_turns_keep_only_the_newest_turns() {
        let (_dir, sm, id) = store().await;
        let turns: Vec<String> = (0..RECENT_TURNS + 3)
            .map(|i| format!("turn-{i:02}"))
            .collect();
        for turn in &turns {
            let mut entry = log_entry(Some(&id), Entry::default());
            entry.turn_id = Some(turn.clone());
            write(&sm, entry).await;
            // Distinct milliseconds, so "newest" is not a tie.
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        let summary = summary(&sm, &id).await.unwrap();
        assert_eq!(summary.recent_turns.len(), RECENT_TURNS);
        assert_eq!(summary.recent_turns[0].turn_id, turns[turns.len() - 1]);
        assert!(
            !summary.recent_turns.iter().any(|t| t.turn_id == turns[0]),
            "the oldest turns fall off"
        );
    }

    #[tokio::test]
    async fn counts_cover_every_chat_asked_in_one_query() {
        let (dir, sm, a) = store().await;
        let b = sm
            .create_session(dir.path().into(), "b".into(), SessionType::User)
            .await
            .unwrap()
            .id;
        for _ in 0..3 {
            write(&sm, log_entry(Some(&a), Entry::default())).await;
        }
        let counts = count_for_sessions(&sm, &[a.clone(), b.clone(), "missing".into()])
            .await
            .unwrap();
        assert_eq!(counts.get(&a), Some(&3));
        assert_eq!(counts.get(&b), Some(&0));
        assert_eq!(counts.get("missing"), Some(&0));
        assert!(count_for_sessions(&sm, &[]).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_oversized_context_is_cut_to_the_cap_and_marked() {
        let (_dir, sm, id) = store().await;
        // Quotes and newlines double in JSON, so the cut must be measured
        // serialized, not by the text's own length.
        let text = "\"line\"\n".repeat(20_000);
        write(
            &sm,
            log_entry(
                Some(&id),
                Entry {
                    context: DecisionContext::Text(text.clone()),
                    ..Entry::default()
                },
            ),
        )
        .await;
        let window: Vec<ContextMessage> = (0..40)
            .map(|i| ContextMessage {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: format!("message {i} ") + &"é".repeat(2_000),
            })
            .collect();
        write(
            &sm,
            log_entry(
                Some(&id),
                Entry {
                    context: DecisionContext::Messages(window.clone()),
                    ..Entry::default()
                },
            ),
        )
        .await;

        let page = list(&sm, &id, &DecisionQuery::default()).await.unwrap();
        for record in &page.records {
            assert!(record.context_truncated);
            assert!(record.context_preview.chars().count() <= CONTEXT_PREVIEW_CHARS);
            let detail = get(&sm, &id, &record.id).await.unwrap().unwrap();
            let stored = serde_json::to_string(&detail.context).unwrap();
            assert!(stored.len() <= MAX_STORED_CONTEXT_BYTES, "{}", stored.len());
            match &detail.context {
                DecisionContext::Text(kept) => {
                    assert!(text.starts_with(kept.as_str()));
                    assert!(
                        stored.len() > MAX_STORED_CONTEXT_BYTES - 16,
                        "cut far more than it had to: {}",
                        stored.len()
                    );
                    assert_eq!(record.context_bytes, text.len() as u64);
                    assert!(record.context_preview.ends_with('…'));
                }
                DecisionContext::Messages(kept) => {
                    assert_eq!(kept.last(), window.last(), "the newest message is kept");
                    assert!(kept.len() < window.len());
                    // Whole messages are dropped, so the gap is under one message.
                    assert!(stored.len() > MAX_STORED_CONTEXT_BYTES - 4_100);
                    assert!(record.context_preview.starts_with('…'));
                }
                DecisionContext::Json(_) => panic!("unexpected context kind"),
            }
        }

        // One message larger than the cap keeps its end.
        let huge = DecisionContext::Messages(vec![ContextMessage {
            role: "assistant".to_string(),
            content: format!("{}THE END", "x".repeat(100_000)),
        }]);
        let (json, stored, truncated) = cap_context(&huge);
        assert!(truncated && json.len() <= MAX_STORED_CONTEXT_BYTES);
        let DecisionContext::Messages(kept) = stored else {
            panic!("a window stays a window");
        };
        assert!(kept[0].content.ends_with("THE END"));
    }

    /// A row is stored under the request id its decisions carry, so whatever
    /// holds a decision (a decision chat's reply) names its row with no
    /// lookup. A failure row, which has no decisions, gets an id of its own.
    #[tokio::test]
    async fn a_row_is_stored_under_its_requests_id() {
        let (_dir, sm, id) = store().await;
        let mut entry = log_entry(Some(&id), Entry::default());
        // Two questions of one request share its id; the row takes it.
        let request_id = entry.decisions[0].request_id.clone();
        let mut second = decision("edss_stated", 0.7, Some(0.5));
        second.request_id = request_id.clone();
        entry.decisions.push(second);
        assert_eq!(row_id(&entry), request_id);
        write(&sm, entry).await;

        let detail = get(&sm, &id, &request_id)
            .await
            .unwrap()
            .expect("the row is found by its request id");
        assert_eq!(detail.record.id, request_id);
        assert!(detail
            .record
            .decisions
            .iter()
            .all(|d| d.request_id == request_id));

        let failure = log_entry(
            Some(&id),
            Entry {
                outcome: DecisionOutcome::Error,
                p_yes: None,
                ..Entry::default()
            },
        );
        let (first, again) = (row_id(&failure), row_id(&failure));
        assert_ne!(first, again, "a failure row's id is minted, not shared");
        write(&sm, failure).await;
        let page = list(&sm, &id, &DecisionQuery::default()).await.unwrap();
        assert_eq!(page.total, 2);
        assert!(page.records.iter().any(|r| r.id != request_id
            && r.outcome == DecisionOutcome::Error
            && uuid::Uuid::parse_str(&r.id).is_ok()));
    }

    /// `decisionId` selects exactly one row, so a decision card's link can
    /// open the history on that call; the other filters still apply beside it.
    #[tokio::test]
    async fn a_decision_id_filter_selects_one_row() {
        let (_dir, sm, id) = store().await;
        let wanted = log_entry(Some(&id), Entry::default());
        let wanted_id = row_id(&wanted);
        write(&sm, wanted).await;
        for _ in 0..3 {
            write(&sm, log_entry(Some(&id), Entry::default())).await;
        }
        let one = list(
            &sm,
            &id,
            &DecisionQuery {
                decision_id: Some(wanted_id.clone()),
                ..DecisionQuery::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(one.total, 1);
        assert_eq!(one.records.len(), 1);
        assert_eq!(one.records[0].id, wanted_id);
        assert_eq!(one.next_cursor, None);

        let none = list(
            &sm,
            &id,
            &DecisionQuery {
                decision_id: Some(wanted_id.clone()),
                origin: Some(DecisionOrigin::Tool),
                ..DecisionQuery::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(none.total, 0, "the id is one filter among the others");
        let elsewhere = list(
            &sm,
            "another-chat",
            &DecisionQuery {
                decision_id: Some(wanted_id),
                ..DecisionQuery::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(elsewhere.total, 0, "an id never reaches another chat's row");
    }

    /// Every `decide` call has its own batch id as its origin id, so grouping
    /// tool rows by origin id would list one source per call. They are one
    /// "decide" source per chat (and per turn); monitors stay one per monitor
    /// id and gates one per phase id, and the history still filters a batch.
    #[tokio::test]
    async fn tool_calls_are_one_source_while_monitors_and_gates_stay_per_id() {
        let (_dir, sm, id) = store().await;
        for (turn, batch) in [("t1", "b1"), ("t1", "b2"), ("t2", "b3")] {
            for _ in 0..2 {
                write(
                    &sm,
                    log_entry(
                        Some(&id),
                        Entry {
                            origin: DecisionOrigin::Tool,
                            origin_id: Some(batch),
                            label: Some("decide"),
                            turn: Some(turn),
                            batch: Some(batch),
                            outcome: DecisionOutcome::Passed,
                            p_yes: Some(0.9),
                            ..Entry::default()
                        },
                    ),
                )
                .await;
            }
        }
        for (origin, origin_id, label) in [
            (DecisionOrigin::Monitor, "form-reached", "Form reached"),
            (DecisionOrigin::Monitor, "login-page", "Login page"),
            (DecisionOrigin::Gate, "phase-1", "Collect"),
            (DecisionOrigin::Gate, "phase-2", "Collect"),
        ] {
            write(
                &sm,
                log_entry(
                    Some(&id),
                    Entry {
                        origin,
                        origin_id: Some(origin_id),
                        label: Some(label),
                        turn: Some("t0"),
                        ..Entry::default()
                    },
                ),
            )
            .await;
        }

        let summary = summary(&sm, &id).await.unwrap();
        assert_eq!(summary.total, 10);
        let tools: Vec<_> = summary
            .aggregates
            .iter()
            .filter(|a| a.origin == DecisionOrigin::Tool)
            .collect();
        assert_eq!(tools.len(), 1, "one decide source, not one per batch");
        assert_eq!(tools[0].label, "decide");
        assert_eq!((tools[0].count, tools[0].passed), (6, 6));
        assert_eq!(tools[0].origin_id, None, "it spans every batch");
        let ids = |origin: DecisionOrigin| {
            let mut ids: Vec<String> = summary
                .aggregates
                .iter()
                .filter(|a| a.origin == origin)
                .filter_map(|a| a.origin_id.clone())
                .collect();
            ids.sort();
            ids
        };
        assert_eq!(ids(DecisionOrigin::Monitor), ["form-reached", "login-page"]);
        assert_eq!(
            ids(DecisionOrigin::Gate),
            ["phase-1", "phase-2"],
            "two gates with one title are still two gates"
        );

        let tool_turns: Vec<(&str, u64)> = summary
            .recent_turns
            .iter()
            .filter(|t| t.origin == DecisionOrigin::Tool)
            .map(|t| (t.turn_id.as_str(), t.count))
            .collect();
        assert_eq!(tool_turns, vec![("t2", 2), ("t1", 4)], "one line per turn");
        let marker = turn_marker(&sm, &id, "t1").await.unwrap();
        let tool_items: Vec<_> = marker
            .items
            .iter()
            .filter(|i| i.origin == DecisionOrigin::Tool)
            .collect();
        assert_eq!(tool_items.len(), 1);
        assert_eq!(
            (tool_items[0].count, tool_items[0].origin_id.clone()),
            (4, None)
        );

        let batch = list(
            &sm,
            &id,
            &DecisionQuery {
                batch_id: Some("b2".into()),
                ..DecisionQuery::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(batch.total, 2, "the history still filters one batch");
    }

    /// One `token_events` row: key, provider, model, input, output, billed,
    /// session type.
    type LedgerEvent = (
        String,
        String,
        String,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        String,
    );
    type Counts = (Option<i64>, Option<i64>, Option<i64>);
    type SessionColumns = (Counts, Counts, Option<String>, String);

    /// The ledger and the columns the side write must leave alone, for one chat.
    #[derive(Debug, PartialEq)]
    struct Ledger {
        events: Vec<LedgerEvent>,
        accumulated: Counts,
        gauge: Counts,
        schedule_id: Option<String>,
        updated_at: String,
    }

    async fn ledger(sm: &SessionManager, id: &str) -> Ledger {
        let pool = sm.storage().pool().await.unwrap();
        let events = sqlx::query_as(
            "SELECT event_key, provider, model_id, input_tokens, output_tokens, \
                    billed_total_tokens, session_type \
               FROM token_events WHERE session_id = ? ORDER BY id",
        )
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap();
        let row = sqlx::query(
            "SELECT accumulated_total_tokens, accumulated_input_tokens, \
                    accumulated_output_tokens, total_tokens, input_tokens, output_tokens, \
                    schedule_id, CAST(updated_at AS TEXT) AS updated_at \
               FROM sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
        let counts = |a: &str, b: &str, c: &str| -> Counts { (row.get(a), row.get(b), row.get(c)) };
        let (accumulated, gauge, schedule_id, updated_at): SessionColumns = (
            counts(
                "accumulated_total_tokens",
                "accumulated_input_tokens",
                "accumulated_output_tokens",
            ),
            counts("total_tokens", "input_tokens", "output_tokens"),
            row.get("schedule_id"),
            row.get("updated_at"),
        );
        Ledger {
            events,
            accumulated,
            gauge,
            schedule_id,
            updated_at,
        }
    }

    /// Give the chat a gauge, a schedule link and an old activity stamp, so a
    /// write that touched any of them would show.
    async fn mark_chat(sm: &SessionManager, id: &str) {
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query(
            "UPDATE sessions SET total_tokens = 5000, input_tokens = 4000, output_tokens = 1000, \
                    schedule_id = 'sched-1', updated_at = '2026-01-01 00:00:00' WHERE id = ?",
        )
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }

    fn baseten_entry(session_id: &str, outcome: DecisionOutcome) -> DecisionLogEntry {
        let mut entry = log_entry(
            Some(session_id),
            Entry {
                outcome,
                p_yes: matches!(outcome, DecisionOutcome::Passed | DecisionOutcome::Failed)
                    .then_some(0.31),
                ..Entry::default()
            },
        );
        let slug = "decision-models/baseten-joey-2b-latest";
        entry.model = Some(DecisionModelRef::new("baseten_decision", slug));
        entry.served_model = Some("joey-2b-0.3".to_string());
        for decision in &mut entry.decisions {
            decision.model = DecisionModelRef::new("baseten_decision", slug);
        }
        entry
    }

    #[tokio::test]
    async fn a_decision_with_usage_lands_in_the_usage_ledger_once() {
        let (_dir, sm, id) = store().await;
        mark_chat(&sm, &id).await;
        let entry = baseten_entry(&id, DecisionOutcome::Failed);
        let row = row_id(&entry);
        let sink = SqliteDecisionSink {
            session_manager: Arc::clone(&sm),
        };
        sink.record(entry.clone()).await.unwrap();

        let after = ledger(&sm, &id).await;
        assert_eq!(
            after.events,
            vec![(
                format!("decision:{row}"),
                "baseten_decision".to_string(),
                // The requested slug, which the price table is keyed by, not
                // the served model.
                "decision-models/baseten-joey-2b-latest".to_string(),
                Some(120),
                Some(2),
                Some(122),
                "user".to_string(),
            )],
            "one ledger row under the decision's own row id"
        );
        assert_eq!(after.accumulated, (Some(122), Some(120), Some(2)));
        assert_eq!(
            after.gauge,
            (Some(5000), Some(4000), Some(1000)),
            "a decision is not part of the chat's context window"
        );
        assert_eq!(after.schedule_id.as_deref(), Some("sched-1"));
        assert_eq!(
            after.updated_at, "2026-01-01 00:00:00",
            "a decision's usage is not activity (R-30)"
        );

        // The same row again is refused by the log's primary key before the
        // ledger is reached, so nothing is billed twice.
        assert!(sink.record_as(&row, entry).await.is_err());
        assert_eq!(ledger(&sm, &id).await, after);

        // The usage report counts it as a decision request, not a turn.
        let usage = sm.get_session_model_usage(&id).await.unwrap();
        assert!(
            usage
                .iter()
                .any(|row| row.provider.as_deref() == Some("baseten_decision")),
            "{usage:?}"
        );
    }

    #[tokio::test]
    async fn a_refusal_or_a_gone_chat_bills_nothing() {
        let (_dir, sm, id) = store().await;
        let mut refused = baseten_entry(&id, DecisionOutcome::Refused);
        refused.usage = DecisionUsage::default();
        write(&sm, refused).await;
        // No model was reached at all.
        let mut no_model = baseten_entry(&id, DecisionOutcome::Error);
        no_model.model = None;
        write(&sm, no_model).await;
        write(&sm, baseten_entry("19990101_1", DecisionOutcome::Passed)).await;
        let after = ledger(&sm, &id).await;
        assert!(after.events.is_empty(), "{:?}", after.events);
        assert_eq!(after.accumulated, (None, None, None));
        let pool = sm.storage().pool().await.unwrap();
        let orphans: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM token_events")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(
            orphans, 0,
            "a decision for a gone chat writes no ledger row"
        );
    }

    #[tokio::test]
    async fn a_billed_answer_that_failed_is_still_ledgered() {
        let (_dir, sm, id) = store().await;
        let failed = baseten_entry(&id, DecisionOutcome::Error);
        assert!(failed.decisions.is_empty() && failed.usage.billed_total() == Some(122));
        let sink = SqliteDecisionSink {
            session_manager: Arc::clone(&sm),
        };
        sink.record_as("caller-chosen", failed).await.unwrap();
        let events = ledger(&sm, &id).await.events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "decision:caller-chosen");
        assert_eq!(events[0].5, Some(122));
    }

    /// What the service hands `record_late`: an error row with the tokens a
    /// late answer was billed for.
    fn late_entry(session_id: &str, text: &str) -> DecisionLogEntry {
        let mut entry = baseten_entry(session_id, DecisionOutcome::Error);
        entry.usage = DecisionUsage {
            input_tokens: Some(30),
            output_tokens: Some(1),
        };
        entry.latency_ms = Some(5_000);
        entry.error = Some(text.to_string());
        entry
    }

    /// The decision row as stored: tokens, latency and error.
    async fn stored_usage(
        sm: &SessionManager,
        id: &str,
    ) -> Vec<(Option<i64>, Option<i64>, Option<i64>, Option<String>)> {
        let pool = sm.storage().pool().await.unwrap();
        sqlx::query_as(
            "SELECT input_tokens, output_tokens, latency_ms, error FROM decisions WHERE id = ?",
        )
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    /// A late answer fills its tokens into the timeout row its request wrote
    /// and bills it once under that row's key, however often it is delivered,
    /// without touching the chat's activity, gauge or schedule.
    #[tokio::test]
    async fn a_late_usage_fills_a_timeout_row_and_bills_once() {
        let (_dir, sm, id) = store().await;
        mark_chat(&sm, &id).await;
        let sink = SqliteDecisionSink {
            session_manager: Arc::clone(&sm),
        };
        let mut timeout = baseten_entry(&id, DecisionOutcome::Error);
        timeout.usage = DecisionUsage::default();
        timeout.latency_ms = None;
        timeout.error = Some("joey-1 did not answer within 2 s. Nothing was decided.".into());
        sink.record_as("req-late", timeout).await.unwrap();
        let before = ledger(&sm, &id).await;
        assert!(before.events.is_empty(), "a timeout row bills nothing");

        let text = "joey-1 did not answer within 2 s. Nothing was decided. It answered after \
                    5.0 s, too late to use; its tokens are counted in Usage.";
        sink.record_late("req-late", late_entry(&id, text))
            .await
            .unwrap();
        sink.record_late("req-late", late_entry(&id, "a second delivery"))
            .await
            .unwrap();

        assert_eq!(
            stored_usage(&sm, "req-late").await,
            vec![(Some(30), Some(1), Some(5_000), Some(text.to_string()))],
            "one row, filled once"
        );
        let after = ledger(&sm, &id).await;
        assert_eq!(
            after.events,
            vec![(
                "decision:req-late".to_string(),
                "baseten_decision".to_string(),
                "decision-models/baseten-joey-2b-latest".to_string(),
                Some(30),
                Some(1),
                Some(31),
                "user".to_string(),
            )],
            "billed once, under the row's own key"
        );
        assert_eq!(after.accumulated, (Some(31), Some(30), Some(1)));
        assert_eq!(after.gauge, before.gauge);
        assert_eq!(after.schedule_id, before.schedule_id);
        assert_eq!(
            after.updated_at, "2026-01-01 00:00:00",
            "a late answer is not activity (R-30)"
        );
    }

    /// An abandoned request wrote no row; its late answer is stored as one.
    #[tokio::test]
    async fn a_late_row_is_stored_when_none_was_written() {
        let (_dir, sm, id) = store().await;
        let sink = SqliteDecisionSink {
            session_manager: Arc::clone(&sm),
        };
        let text = "The request was abandoned before joey-1 answered (the turn was stopped or \
                    the caller went away), so its answer was not used. It answered after 4.0 \
                    s; its tokens are counted in Usage.";
        sink.record_late("req-abandoned", late_entry(&id, text))
            .await
            .unwrap();
        let page = list(&sm, &id, &DecisionQuery::default()).await.unwrap();
        assert_eq!(page.records.len(), 1);
        let record = &page.records[0];
        assert_eq!(record.id, "req-abandoned");
        assert_eq!(record.outcome, DecisionOutcome::Error);
        assert_eq!(record.error.as_deref(), Some(text));
        assert_eq!(
            (record.input_tokens, record.output_tokens),
            (Some(30), Some(1))
        );
        let events = ledger(&sm, &id).await.events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "decision:req-abandoned");
    }

    /// A chat deleted before its late answer came stores and bills nothing.
    #[tokio::test]
    async fn a_late_write_for_a_deleted_chat_stores_and_bills_nothing() {
        let (_dir, sm, _id) = store().await;
        let sink = SqliteDecisionSink {
            session_manager: Arc::clone(&sm),
        };
        sink.record_late("req-gone", late_entry("19990101_1", "late"))
            .await
            .unwrap();
        let pool = sm.storage().pool().await.unwrap();
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM decisions")
            .fetch_one(pool)
            .await
            .unwrap();
        let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM token_events")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!((rows, events), (0, 0));
    }

    #[test]
    fn only_a_decision_provider_is_ledgered() {
        let mut entry = log_entry(Some("s"), Entry::default());
        assert_eq!(
            side_usage("r", "s", &entry).map(|usage| usage.event_key),
            Some("decision:r".to_string())
        );
        // A decision recorded under a chat provider's name would read as a
        // model turn in the usage report.
        entry.model = Some(DecisionModelRef::new("openai", "gpt-4o-mini"));
        assert!(side_usage("r", "s", &entry).is_none());
        entry.model = Some(DecisionModelRef::new("ollama_decision", "tev1:0.8b"));
        entry.usage = DecisionUsage::default();
        assert!(
            side_usage("r", "s", &entry).is_none(),
            "no usage, nothing to bill"
        );
        entry.usage.output_tokens = Some(1);
        assert_eq!(
            side_usage("r", "s", &entry).map(|usage| (usage.input_tokens, usage.output_tokens)),
            Some((None, Some(1)))
        );
    }

    /// The usage report and the ledger tell a decision request from a turn by
    /// the provider's name, and the desktop names the same providers
    /// (`isDecisionProvider`, which labels a decision row in the chat's cost
    /// popover and hides a decision chat's context ring). The two lists never
    /// import each other, so they are read against each other here.
    #[test]
    fn the_desktop_names_the_same_decision_providers() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../ui/desktop/src/components/decisions/decisionWire.ts"
        );
        let source = std::fs::read_to_string(path).expect("the desktop's decision wire module");
        let (_, after) = source
            .split_once("export const DECISION_PROVIDER_IDS")
            .expect("DECISION_PROVIDER_IDS is declared");
        let (list, _) = after.split_once("]);").expect("the list ends");
        let (_, list) = list.split_once("new Set([").expect("the list is a Set");
        let mut desktop: Vec<&str> = list
            .split(',')
            .map(|item| item.trim().trim_matches('\''))
            .filter(|item| !item.is_empty())
            .collect();
        desktop.sort_unstable();
        let mut core = crate::providers::decision::chat::DECISION_CHAT_PROVIDERS.to_vec();
        core.sort_unstable();
        assert_eq!(desktop, core);
    }

    #[test]
    fn readable_answers_and_pass_probabilities() {
        let yes = decision("q", 0.8, None);
        assert_eq!(readable_answer(&yes), "yes");
        assert_eq!(decision_pass_probability(&yes), Some(0.8));
        let mut score = decision("q", 0.8, None);
        score.answer = Answer::Score {
            level: 2,
            label: String::new(),
            expected: 1.7,
        };
        assert_eq!(readable_answer(&score), "level 2");
        assert_eq!(decision_pass_probability(&score), None);
        assert_eq!(
            lowest_pass_probability(&[
                decision("a", 0.9, Some(0.5)),
                decision("b", 0.4, Some(0.5)),
                score
            ]),
            Some(0.4)
        );
        assert_eq!(lowest_pass_probability(&[]), None);
    }
}
